//! Background coordinator loop for the minimum Rust engine spine.

use std::any::Any;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::contract::{CommandBatch, EngineCommand, EngineFault, ReliableEvent};
use super::error::{EngineError, EngineErrorCode};
use super::queues::{InboundQueue, InboundWaitResult, OutputQueue};
use super::running_loop::{
    RunningAuthorityLoop, RunningAuthorityLoopProgress, RunningAuthorityLoopState,
};
use super::scheduler::SchedulerServiceMode;

/// Observable lifecycle of the one-shot engine coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleState {
    /// Constructed but not started.
    Created,
    /// Background coordinator is accepting work.
    Running,
    /// An orderly stop has been requested.
    StopRequested,
    /// A caught panic or unrecoverable bounded-queue failure stopped authority.
    Faulted,
    /// Coordinator is no longer running and cannot restart.
    Stopped,
}

/// State shared between runtime control methods and the coordinator root.
#[derive(Debug)]
pub(crate) struct CoordinatorState {
    lifecycle: Mutex<LifecycleState>,
    fault: Mutex<Option<EngineFault>>,
    processed_batches: AtomicU64,
    processed_commands: AtomicU64,
}

/// Small atomic snapshot of one loop owned by the background coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunningAuthorityHealth {
    /// Current retained-loop state.
    pub loop_state: RunningAuthorityLoopState,
    /// Current published authoritative generation.
    pub generation: u64,
    /// Current published authoritative completed-step count.
    pub completed_step: u64,
    /// Scheduler tickets committed by this retained loop.
    pub scheduler_completed_steps: u64,
    /// Command-drain boundaries serviced by the scheduler.
    pub command_service_boundaries: u64,
    /// Condition-variable waits entered by the coordinator.
    pub wait_calls: u64,
    /// Waits entered while authoritative work was externally blocked.
    pub blocked_wait_calls: u64,
    /// Timed scheduler waits that reached their deadline.
    pub timeout_wakes: u64,
    /// Waits woken by one or more inbound command batches.
    pub command_wakes: u64,
}

/// Atomics updated only by the authority thread and read by health callers.
#[derive(Debug)]
pub(crate) struct RunningAuthorityMetrics {
    loop_state: AtomicU8,
    generation: AtomicU64,
    completed_step: AtomicU64,
    scheduler_completed_steps: AtomicU64,
    command_service_boundaries: AtomicU64,
    wait_calls: AtomicU64,
    blocked_wait_calls: AtomicU64,
    timeout_wakes: AtomicU64,
    command_wakes: AtomicU64,
}

impl RunningAuthorityMetrics {
    /// Seed metrics from an unserviced retained loop before thread start.
    pub(crate) fn new(running: &RunningAuthorityLoop) -> Self {
        let diagnostics = running.scheduler_diagnostics();
        Self {
            loop_state: AtomicU8::new(loop_state_code(running.state())),
            generation: AtomicU64::new(running.generation()),
            completed_step: AtomicU64::new(running.completed_step()),
            scheduler_completed_steps: AtomicU64::new(diagnostics.completed_steps),
            command_service_boundaries: AtomicU64::new(diagnostics.command_service_boundaries),
            wait_calls: AtomicU64::new(0),
            blocked_wait_calls: AtomicU64::new(0),
            timeout_wakes: AtomicU64::new(0),
            command_wakes: AtomicU64::new(0),
        }
    }

    /// Refresh the published bounded authority/scheduler snapshot.
    pub(crate) fn observe(&self, running: &RunningAuthorityLoop) {
        let diagnostics = running.scheduler_diagnostics();
        self.generation
            .store(running.generation(), Ordering::Release);
        self.completed_step
            .store(running.completed_step(), Ordering::Release);
        self.scheduler_completed_steps
            .store(diagnostics.completed_steps, Ordering::Release);
        self.command_service_boundaries
            .store(diagnostics.command_service_boundaries, Ordering::Release);
        self.loop_state
            .store(loop_state_code(running.state()), Ordering::Release);
    }

    /// Record one actual condition-variable wait.
    fn record_wait(&self, blocked: bool) {
        saturating_increment(&self.wait_calls, 1);
        if blocked {
            saturating_increment(&self.blocked_wait_calls, 1);
        }
    }

    /// Record why one condition-variable wait completed.
    fn record_wake(&self, result: InboundWaitResult) {
        match result {
            InboundWaitResult::CommandsReady => saturating_increment(&self.command_wakes, 1),
            InboundWaitResult::TimedOut => saturating_increment(&self.timeout_wakes, 1),
            InboundWaitResult::Stopped => {}
        }
    }

    /// Read a bounded, allocation-free operational snapshot.
    pub(crate) fn snapshot(&self) -> RunningAuthorityHealth {
        RunningAuthorityHealth {
            loop_state: loop_state_from_code(self.loop_state.load(Ordering::Acquire)),
            generation: self.generation.load(Ordering::Acquire),
            completed_step: self.completed_step.load(Ordering::Acquire),
            scheduler_completed_steps: self.scheduler_completed_steps.load(Ordering::Acquire),
            command_service_boundaries: self.command_service_boundaries.load(Ordering::Acquire),
            wait_calls: self.wait_calls.load(Ordering::Relaxed),
            blocked_wait_calls: self.blocked_wait_calls.load(Ordering::Relaxed),
            timeout_wakes: self.timeout_wakes.load(Ordering::Relaxed),
            command_wakes: self.command_wakes.load(Ordering::Relaxed),
        }
    }
}

impl CoordinatorState {
    /// Create state for an unstarted runtime.
    pub(crate) fn new() -> Self {
        Self {
            lifecycle: Mutex::new(LifecycleState::Created),
            fault: Mutex::new(None),
            processed_batches: AtomicU64::new(0),
            processed_commands: AtomicU64::new(0),
        }
    }

    /// Read the current lifecycle.
    pub(crate) fn lifecycle(&self) -> LifecycleState {
        *lock_recover(&self.lifecycle)
    }

    /// Execute a small lifecycle transition under its dedicated lock.
    pub(crate) fn transition(
        &self,
        transition: impl FnOnce(LifecycleState) -> Result<LifecycleState, EngineError>,
    ) -> Result<LifecycleState, EngineError> {
        let mut state = lock_recover(&self.lifecycle);
        let next = transition(*state)?;
        *state = next;
        Ok(next)
    }

    /// Mark normal coordinator completion without overwriting a fault.
    pub(crate) fn mark_normal_stopped(&self) {
        let mut lifecycle = lock_recover(&self.lifecycle);
        if *lifecycle != LifecycleState::Faulted {
            *lifecycle = LifecycleState::Stopped;
        }
    }

    /// Convert a faulted coordinator to terminal stopped state after joining.
    pub(crate) fn mark_joined(&self) {
        let mut lifecycle = lock_recover(&self.lifecycle);
        *lifecycle = LifecycleState::Stopped;
    }

    /// Return the retained first fault.
    pub(crate) fn fault(&self) -> Option<EngineFault> {
        lock_recover(&self.fault).clone()
    }

    /// Publish the first fault without using normal output capacity.
    pub(crate) fn publish_fault(&self, output: &OutputQueue, error: EngineError) {
        let fault = EngineFault::from(error);
        let mut lifecycle = lock_recover(&self.lifecycle);
        if *lifecycle == LifecycleState::Stopped {
            return;
        }
        let mut retained = lock_recover(&self.fault);
        if retained.is_some() {
            return;
        }
        let previous_lifecycle = *lifecycle;
        *retained = Some(fault.clone());
        *lifecycle = LifecycleState::Faulted;
        if !output.retain_reserved_fault(fault) {
            *retained = None;
            *lifecycle = previous_lifecycle;
            return;
        }
        drop(retained);
        drop(lifecycle);
        output.signal_retained_fault();
    }

    /// Snapshot processed work counters.
    pub(crate) fn processed(&self) -> (u64, u64) {
        (
            self.processed_batches.load(Ordering::Relaxed),
            self.processed_commands.load(Ordering::Relaxed),
        )
    }

    /// Record one fully processed batch after its complete output publishes.
    fn record_processed(&self, command_count: usize) {
        saturating_increment(
            &self.processed_commands,
            u64::try_from(command_count).unwrap_or(u64::MAX),
        );
        saturating_increment(&self.processed_batches, 1);
    }
}

/// Atomically close future admission, discard unapplied work, and retain the
/// first fault. A racing successful submit was linearized before this closure
/// and is reported by the discarded-work counters rather than as applied.
pub(crate) fn fault_and_stop(
    inbound: &InboundQueue,
    output: &OutputQueue,
    state: &CoordinatorState,
    error: EngineError,
) {
    inbound.request_fault_stop();
    state.publish_fault(output, error);
}

/// Run until out-of-band stop, a caught outer panic, or an output fault.
pub(crate) fn run_coordinator(
    inbound: &Arc<InboundQueue>,
    output: &Arc<OutputQueue>,
    state: &Arc<CoordinatorState>,
) {
    if let Err(error) = output.push_reliable(ReliableEvent::Started) {
        fault_and_stop(inbound, output, state, error);
        return;
    }

    while let Some(batch) = inbound.wait_pop() {
        if let Err(error) = process_command_batch(batch, output, state) {
            fault_and_stop(inbound, output, state, error);
            return;
        }
    }

    if let Err(error) = output.publish_orderly_stopped() {
        fault_and_stop(inbound, output, state, error);
    }
}

/// Run one real authority from a Rust monotonic clock until stop or a blocker.
pub(crate) fn run_running_coordinator(
    inbound: &Arc<InboundQueue>,
    output: &Arc<OutputQueue>,
    state: &Arc<CoordinatorState>,
    running: &mut RunningAuthorityLoop,
    metrics: &RunningAuthorityMetrics,
) -> Result<(), EngineError> {
    running.validate_background_start().map_err(|error| {
        EngineError::new(
            EngineErrorCode::InvalidLifecycle,
            format!("invalid background authority handoff: {error}"),
        )
    })?;
    output.push_reliable(ReliableEvent::Started)?;

    let wall_origin = Instant::now();
    let mut wait = RunningWait::Immediate;
    let mut batches = Vec::new();
    loop {
        match wait {
            RunningWait::Immediate => {}
            RunningWait::Timed(timeout) => {
                metrics.record_wait(false);
                let result = inbound.wait_until_ready(Some(timeout));
                metrics.record_wake(result);
                if result == InboundWaitResult::Stopped {
                    publish_orderly_stopped(output)?;
                    return Ok(());
                }
            }
            RunningWait::Blocked => {
                metrics.record_wait(true);
                let result = inbound.wait_until_ready(None);
                metrics.record_wake(result);
                if result == InboundWaitResult::Stopped {
                    publish_orderly_stopped(output)?;
                    return Ok(());
                }
            }
        }

        let stop_requested = inbound.drain_step_boundary(&mut batches);
        for batch in batches.drain(..) {
            process_command_batch(batch, output, state)?;
        }
        if stop_requested {
            publish_orderly_stopped(output)?;
            return Ok(());
        }

        let wall_now_ms = u64::try_from(wall_origin.elapsed().as_millis()).map_err(|_| {
            EngineError::new(
                EngineErrorCode::Faulted,
                "background monotonic clock exceeded the supported millisecond range",
            )
        })?;
        let progress = match running.service_after_command_drain(
            wall_now_ms,
            SchedulerServiceMode::Background,
            None,
        ) {
            Ok(progress) => progress,
            Err(error) => {
                metrics.observe(running);
                return Err(EngineError::new(
                    EngineErrorCode::Faulted,
                    format!("background authority service failed: {error}"),
                ));
            }
        };
        metrics.observe(running);
        wait = match progress {
            RunningAuthorityLoopProgress::Idle {
                wall_seconds_until_step,
                ..
            } => RunningWait::Timed(positive_wait_duration(wall_seconds_until_step)?),
            RunningAuthorityLoopProgress::Published { .. } => RunningWait::Immediate,
            RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
            | RunningAuthorityLoopProgress::GenerationTransitionPending { .. } => {
                RunningWait::Blocked
            }
        };
    }
}

fn publish_orderly_stopped(output: &OutputQueue) -> Result<(), EngineError> {
    output.publish_orderly_stopped().map(|_| ())
}

fn process_command_batch(
    batch: CommandBatch,
    output: &OutputQueue,
    state: &CoordinatorState,
) -> Result<(), EngineError> {
    let command_count = batch.commands.len();
    let mut results = Vec::with_capacity(command_count);
    for sequenced in batch.commands {
        match sequenced.command {
            EngineCommand::Probe {
                correlation_id,
                payload,
            } => {
                results.push(ReliableEvent::ProbeResult {
                    sequence: sequenced.sequence,
                    correlation_id,
                    payload,
                });
            }
            EngineCommand::Unsupported { kind, .. } => {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidCommand,
                    format!("unsupported command kind {kind} reached coordinator"),
                ));
            }
            #[cfg(any(test, feature = "engine-test-hooks"))]
            EngineCommand::PanicForTest => {
                panic!("test-only coordinator panic injection");
            }
        }
    }
    output.push_reliable_batch(results)?;
    state.record_processed(command_count);
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum RunningWait {
    Immediate,
    Timed(Duration),
    Blocked,
}

fn positive_wait_duration(seconds: f64) -> Result<Duration, EngineError> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(EngineError::new(
            EngineErrorCode::Faulted,
            "scheduler returned a non-positive background wait",
        ));
    }
    let duration = Duration::try_from_secs_f64(seconds).map_err(|_| {
        EngineError::new(
            EngineErrorCode::Faulted,
            "scheduler background wait exceeded the supported duration",
        )
    })?;
    Ok(duration.max(Duration::from_nanos(1)))
}

fn loop_state_code(state: RunningAuthorityLoopState) -> u8 {
    match state {
        RunningAuthorityLoopState::Ready => 0,
        RunningAuthorityLoopState::ExternalDeliveryPending => 1,
        RunningAuthorityLoopState::GenerationTransitionPending => 2,
        RunningAuthorityLoopState::Faulted => 3,
    }
}

fn loop_state_from_code(code: u8) -> RunningAuthorityLoopState {
    match code {
        0 => RunningAuthorityLoopState::Ready,
        1 => RunningAuthorityLoopState::ExternalDeliveryPending,
        2 => RunningAuthorityLoopState::GenerationTransitionPending,
        _ => RunningAuthorityLoopState::Faulted,
    }
}

fn saturating_increment(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}

/// Convert a caught panic payload into bounded generic diagnostic detail.
pub(crate) fn panic_error(payload: &(dyn Any + Send)) -> EngineError {
    let detail = if let Some(message) = payload.downcast_ref::<&str>() {
        format!("coordinator panic: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("coordinator panic: {message}")
    } else {
        "coordinator panic with non-string payload".to_owned()
    };
    EngineError::new(EngineErrorCode::Faulted, detail)
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
