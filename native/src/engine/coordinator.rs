//! Background coordinator loop for the minimum Rust engine spine.

use std::any::Any;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::contract::{
    CommandBatch, EngineCommand, EngineFault, ExternalDeliveryReceipt,
    GenerationAssignmentReceiptState, ReliableEvent, RunningAuthorityCommand,
    RunningAuthorityEvent, RunningGenerationAssignment,
};
use super::error::{EngineError, EngineErrorCode};
use super::queues::{InboundQueue, InboundWaitResult, OutputQueue};
use super::running_loop::{
    RunningAuthorityDeliveryState, RunningAuthorityLoop, RunningAuthorityLoopProgress,
    RunningAuthorityLoopState,
};
use super::running_step::{
    ExternalDeliveryEventKind, ExternalObservationBatch, GenerationReassignmentProgress,
};
use super::scheduler::SchedulerServiceMode;
use super::world_step::ExternalDeliveryStatus;

/// Inclusive microsecond ceilings for the allocation-free production step histogram.
///
/// The final bucket is open-ended. Published p95/p99 values are therefore
/// conservative bucket upper bounds, while maximum and mean use exact sampled
/// microseconds. The denser sub-frame buckets preserve useful P0/P1 resolution
/// without retaining one record per authoritative step.
const STEP_TIMING_BUCKET_UPPER_MICROS: [u64; 24] = [
    100,
    200,
    300,
    400,
    500,
    750,
    1_000,
    1_500,
    2_000,
    3_000,
    4_000,
    6_000,
    8_000,
    12_000,
    16_000,
    24_000,
    32_000,
    48_000,
    64_000,
    96_000,
    128_000,
    192_000,
    256_000,
    u64::MAX,
];

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
    /// Current process-local authoritative world incarnation.
    pub world_epoch: u64,
    /// Current published authoritative generation.
    pub generation: u64,
    /// Current published authoritative completed-step count.
    pub completed_step: u64,
    /// Whether the retained transition already owns an immutable descriptor.
    pub generation_checkpoint_published: bool,
    /// Whether Rust retained the worker's complete matching descriptor.
    pub generation_persistence_acknowledged: bool,
    /// Exact unresolved reliable deliveries retained by the coordinator.
    pub pending_external_deliveries: usize,
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
    /// Successful authoritative step computations represented by timing data.
    pub step_timing_samples: u64,
    /// Saturating sum of sampled step-computation time in microseconds.
    pub step_timing_total_micros: u64,
    /// Largest sampled step-computation time in microseconds.
    pub step_timing_max_micros: u64,
    /// Conservative inclusive histogram ceiling containing the 95th percentile.
    pub step_timing_p95_micros: u64,
    /// Conservative inclusive histogram ceiling containing the 99th percentile.
    pub step_timing_p99_micros: u64,
}

/// Atomics updated only by the authority thread and read by health callers.
#[derive(Debug)]
pub(crate) struct RunningAuthorityMetrics {
    loop_state: AtomicU8,
    world_epoch: AtomicU64,
    generation: AtomicU64,
    completed_step: AtomicU64,
    generation_checkpoint_published: AtomicBool,
    generation_persistence_acknowledged: AtomicBool,
    pending_external_deliveries: AtomicUsize,
    scheduler_completed_steps: AtomicU64,
    command_service_boundaries: AtomicU64,
    wait_calls: AtomicU64,
    blocked_wait_calls: AtomicU64,
    timeout_wakes: AtomicU64,
    command_wakes: AtomicU64,
    step_timing_samples: AtomicU64,
    step_timing_total_micros: AtomicU64,
    step_timing_max_micros: AtomicU64,
    step_timing_buckets: [AtomicU64; STEP_TIMING_BUCKET_UPPER_MICROS.len()],
}

impl RunningAuthorityMetrics {
    /// Seed metrics from an unserviced retained loop before thread start.
    pub(crate) fn new(running: &RunningAuthorityLoop) -> Self {
        let diagnostics = running.scheduler_diagnostics();
        Self {
            loop_state: AtomicU8::new(loop_state_code(running.state())),
            world_epoch: AtomicU64::new(running.world_epoch()),
            generation: AtomicU64::new(running.generation()),
            completed_step: AtomicU64::new(running.completed_step()),
            generation_checkpoint_published: AtomicBool::new(false),
            generation_persistence_acknowledged: AtomicBool::new(false),
            pending_external_deliveries: AtomicUsize::new(0),
            scheduler_completed_steps: AtomicU64::new(diagnostics.completed_steps),
            command_service_boundaries: AtomicU64::new(diagnostics.command_service_boundaries),
            wait_calls: AtomicU64::new(0),
            blocked_wait_calls: AtomicU64::new(0),
            timeout_wakes: AtomicU64::new(0),
            command_wakes: AtomicU64::new(0),
            step_timing_samples: AtomicU64::new(0),
            step_timing_total_micros: AtomicU64::new(0),
            step_timing_max_micros: AtomicU64::new(0),
            step_timing_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    /// Refresh the published bounded authority/scheduler snapshot.
    pub(crate) fn observe(&self, running: &RunningAuthorityLoop) {
        let diagnostics = running.scheduler_diagnostics();
        let transition = running.pending_generation_transition();
        let generation_checkpoint_published = transition
            .and_then(|pending| pending.checkpoint_descriptor())
            .is_some();
        let generation_persistence_acknowledged =
            transition.is_some_and(|pending| pending.persistence_acknowledged());
        let pending_external_deliveries = running.pending_external_delivery().map_or(
            usize::from(matches!(
                running.state(),
                RunningAuthorityLoopState::ControllerReclaimPending
                    | RunningAuthorityLoopState::ControllerJoinPending
            )),
            |batch| batch.remaining(),
        );
        self.world_epoch
            .store(running.world_epoch(), Ordering::Release);
        self.generation
            .store(running.generation(), Ordering::Release);
        self.completed_step
            .store(running.completed_step(), Ordering::Release);
        self.generation_checkpoint_published
            .store(generation_checkpoint_published, Ordering::Release);
        self.generation_persistence_acknowledged
            .store(generation_persistence_acknowledged, Ordering::Release);
        self.pending_external_deliveries
            .store(pending_external_deliveries, Ordering::Release);
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

    /// Retain bounded production timing without allocating or crossing authority ownership.
    fn record_step_duration(&self, duration: Duration) {
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        saturating_increment(&self.step_timing_samples, 1);
        saturating_increment(&self.step_timing_total_micros, micros);
        let _ = self.step_timing_max_micros.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |previous| Some(previous.max(micros)),
        );
        let bucket = STEP_TIMING_BUCKET_UPPER_MICROS
            .partition_point(|upper| *upper < micros)
            .min(STEP_TIMING_BUCKET_UPPER_MICROS.len() - 1);
        saturating_increment(&self.step_timing_buckets[bucket], 1);
    }

    /// Return the conservative inclusive bucket ceiling for one percentile.
    fn step_percentile_micros(&self, samples: u64, percentile: u64) -> u64 {
        if samples == 0 {
            return 0;
        }
        let rank = u64::try_from((u128::from(samples) * u128::from(percentile)).div_ceil(100))
            .unwrap_or(u64::MAX);
        let mut cumulative = 0u64;
        for (index, bucket) in self.step_timing_buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(bucket.load(Ordering::Relaxed));
            if cumulative >= rank {
                let upper = STEP_TIMING_BUCKET_UPPER_MICROS[index];
                return if upper == u64::MAX {
                    self.step_timing_max_micros.load(Ordering::Relaxed)
                } else {
                    upper
                };
            }
        }
        self.step_timing_max_micros.load(Ordering::Relaxed)
    }

    /// Read a bounded, allocation-free operational snapshot.
    pub(crate) fn snapshot(&self) -> RunningAuthorityHealth {
        let step_timing_samples = self.step_timing_samples.load(Ordering::Relaxed);
        RunningAuthorityHealth {
            loop_state: loop_state_from_code(self.loop_state.load(Ordering::Acquire)),
            world_epoch: self.world_epoch.load(Ordering::Acquire),
            generation: self.generation.load(Ordering::Acquire),
            completed_step: self.completed_step.load(Ordering::Acquire),
            generation_checkpoint_published: self
                .generation_checkpoint_published
                .load(Ordering::Acquire),
            generation_persistence_acknowledged: self
                .generation_persistence_acknowledged
                .load(Ordering::Acquire),
            pending_external_deliveries: self.pending_external_deliveries.load(Ordering::Acquire),
            scheduler_completed_steps: self.scheduler_completed_steps.load(Ordering::Acquire),
            command_service_boundaries: self.command_service_boundaries.load(Ordering::Acquire),
            wait_calls: self.wait_calls.load(Ordering::Relaxed),
            blocked_wait_calls: self.blocked_wait_calls.load(Ordering::Relaxed),
            timeout_wakes: self.timeout_wakes.load(Ordering::Relaxed),
            command_wakes: self.command_wakes.load(Ordering::Relaxed),
            step_timing_samples,
            step_timing_total_micros: self.step_timing_total_micros.load(Ordering::Relaxed),
            step_timing_max_micros: self.step_timing_max_micros.load(Ordering::Relaxed),
            step_timing_p95_micros: self.step_percentile_micros(step_timing_samples, 95),
            step_timing_p99_micros: self.step_percentile_micros(step_timing_samples, 99),
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
    pub(crate) fn publish_fault(&self, output: &OutputQueue, error: EngineError) -> bool {
        let fault = EngineFault::from(error);
        let mut lifecycle = lock_recover(&self.lifecycle);
        if *lifecycle == LifecycleState::Stopped {
            return false;
        }
        let mut retained = lock_recover(&self.fault);
        if retained.is_some() {
            return true;
        }
        let previous_lifecycle = *lifecycle;
        *retained = Some(fault.clone());
        *lifecycle = LifecycleState::Faulted;
        if !output.retain_reserved_fault(fault) {
            *retained = None;
            *lifecycle = previous_lifecycle;
            return false;
        }
        drop(retained);
        drop(lifecycle);
        output.signal_retained_fault();
        true
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
    // Mark fault-stop before publishing so neither coordinator path can
    // dequeue raced work or mistake the shutdown for an orderly stop. The
    // queue accounts and discards every still-accepted batch before waking.
    inbound.request_fault_stop(|| state.publish_fault(output, error));
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

    if let Err(error) = publish_orderly_stopped(inbound, output) {
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
    display: Option<&super::display::RunningDisplayCache>,
) -> Result<(), EngineError> {
    running.validate_background_start().map_err(|error| {
        EngineError::new(
            EngineErrorCode::InvalidLifecycle,
            format!("invalid background authority handoff: {error}"),
        )
    })?;
    output.push_reliable(ReliableEvent::Started)?;

    let wall_origin = Instant::now();
    running.set_background_clock(wall_origin);
    let mut wait = RunningWait::Immediate;
    let mut batches = Vec::new();
    let mut announced_generation_source = None;
    let mut announced_delivery_ticket = None;
    loop {
        if let Some(display) = display {
            // The preceding command/service reservation has left scope. This
            // sample cannot precede its reliable result or describe staged work.
            display.publish_if_due(running, monotonic_elapsed_ms(wall_origin)?, output)?;
        }
        match wait {
            RunningWait::Immediate => {}
            RunningWait::Timed(timeout) => {
                metrics.record_wait(false);
                let result = inbound.wait_until_ready(Some(timeout));
                metrics.record_wake(result);
                if result == InboundWaitResult::Stopped {
                    publish_orderly_stopped(inbound, output)?;
                    return Ok(());
                }
            }
            RunningWait::Blocked => {
                metrics.record_wait(true);
                let result = inbound.wait_until_eligible(None, false);
                metrics.record_wake(result);
                if result == InboundWaitResult::Stopped {
                    publish_orderly_stopped(inbound, output)?;
                    return Ok(());
                }
            }
        }

        let was_ready = running.state() == RunningAuthorityLoopState::Ready;
        let stop_requested = if was_ready {
            inbound.drain_step_boundary(&mut batches)
        } else {
            inbound.drain_eligible_boundary(&mut batches, false)
        };
        let reclaim_cutoff = batches.iter().any(|batch| {
            batch.commands.iter().any(|item| {
                matches!(
                    item.command,
                    EngineCommand::RunningAuthority(
                        RunningAuthorityCommand::ReclaimController(_)
                            | RunningAuthorityCommand::JoinController(_)
                    )
                )
            })
        });
        for batch in batches.drain(..) {
            if let [super::contract::SequencedCommand {
                command: EngineCommand::RunningAuthority(command),
                ..
            }] = batch.commands.as_ref()
            {
                output.wait_reliable_capacity(running_command_reply_reservation(
                    command, running, output,
                ))?;
            }
            process_running_command_batch(
                batch,
                output,
                state,
                running,
                metrics,
                monotonic_elapsed_ms(wall_origin)?,
            )?;
        }
        if stop_requested {
            publish_orderly_stopped(inbound, output)?;
            return Ok(());
        }

        if (!was_ready || reclaim_cutoff) && running.state() == RunningAuthorityLoopState::Ready {
            // A receipt just retired the blocker. Deferred actions belong to
            // the next step, so take their cutoff before servicing it.
            wait = RunningWait::Immediate;
            continue;
        }

        let service_reply_bytes = super::controller_output::service_reply_bound(running)?;
        if running.state() == RunningAuthorityLoopState::Ready
            && output.wait_reliable_capacity(service_reply_bytes)?
        {
            // Input accepted while display/control output blocked belongs to
            // the next eligible step, so take a fresh cutoff before servicing it.
            wait = RunningWait::Immediate;
            continue;
        }
        let service_reservation = if running.state() == RunningAuthorityLoopState::Ready {
            Some(output.reserve_authority_reply(service_reply_bytes)?)
        } else {
            None
        };
        let service_started = Instant::now();
        let progress = match running.service_after_command_drain(
            monotonic_elapsed_ms(wall_origin)?,
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
        if was_ready
            && matches!(
                progress,
                RunningAuthorityLoopProgress::Published { .. }
                    | RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
                    | RunningAuthorityLoopProgress::GenerationTransitionPending { .. }
            )
        {
            metrics.record_step_duration(service_started.elapsed());
        }
        metrics.observe(running);
        wait = match progress {
            RunningAuthorityLoopProgress::ControllerReclaimPending
            | RunningAuthorityLoopProgress::ControllerJoinPending
            | RunningAuthorityLoopProgress::ImportPending => RunningWait::Blocked,
            RunningAuthorityLoopProgress::Idle {
                wall_seconds_until_step,
                ..
            } => {
                announced_generation_source = None;
                RunningWait::Timed(positive_wait_duration(wall_seconds_until_step)?)
            }
            RunningAuthorityLoopProgress::Published { .. } => {
                announced_generation_source = None;
                announced_delivery_ticket = None;
                RunningWait::Immediate
            }
            RunningAuthorityLoopProgress::ExternalDeliveryPending {
                ticket_sequence, ..
            } => {
                if announced_delivery_ticket != Some(ticket_sequence) {
                    let reservation = service_reservation.ok_or_else(|| {
                        EngineError::new(
                            EngineErrorCode::Faulted,
                            "ordinary controller messages lost their admitted reply",
                        )
                    })?;
                    let messages = super::controller_output::own_pending_messages(running)?;
                    reservation.publish(ReliableEvent::RunningAuthority(Box::new(
                        RunningAuthorityEvent::ControllerMessages {
                            ticket_sequence,
                            messages,
                        },
                    )))?;
                    announced_delivery_ticket = Some(ticket_sequence);
                }
                RunningWait::Blocked
            }
            RunningAuthorityLoopProgress::GenerationTransitionPending {
                ticket_sequence,
                source_key,
                reason,
                successor_generation,
                successor_completed_step,
            } => {
                if announced_generation_source != Some(source_key) {
                    let reservation = service_reservation.ok_or_else(|| {
                        EngineError::new(
                            EngineErrorCode::Faulted,
                            "generation announcement lost its admitted reply",
                        )
                    })?;
                    reservation.publish(ReliableEvent::RunningAuthority(Box::new(
                        RunningAuthorityEvent::GenerationTransitionPending {
                            ticket_sequence,
                            source_key,
                            reason,
                            successor_generation,
                            successor_completed_step,
                        },
                    )))?;
                    announced_generation_source = Some(source_key);
                }
                RunningWait::Blocked
            }
        };
    }
}

/// Reserve the possible transition announcement before servicing any new step.
/// The authority thread is the only normal producer, and draining only frees
/// space. A full queue therefore stops before scheduler/evolution mutation.
#[cfg(all(test, feature = "engine-test-hooks"))]
fn service_running_with_output_capacity(
    running: &mut RunningAuthorityLoop,
    output: &OutputQueue,
    wall_now_ms: u64,
) -> Result<RunningAuthorityLoopProgress, EngineError> {
    if running.state() == RunningAuthorityLoopState::Ready {
        output.preflight_reliable_reservation(&[super::controller_output::service_reply_bound(
            running,
        )?])?;
    }
    running
        .service_after_command_drain(wall_now_ms, SchedulerServiceMode::Background, None)
        .map_err(|error| EngineError::new(EngineErrorCode::Faulted, error.to_string()))
}

fn publish_orderly_stopped(
    inbound: &InboundQueue,
    output: &OutputQueue,
) -> Result<(), EngineError> {
    inbound.publish_orderly_stopped(output)
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
            EngineCommand::RunningAuthority(_) => {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidCommand,
                    "running-authority control reached a runtime without retained authority",
                ));
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

fn process_running_command_batch(
    batch: CommandBatch,
    output: &OutputQueue,
    state: &CoordinatorState,
    running: &mut RunningAuthorityLoop,
    metrics: &RunningAuthorityMetrics,
    wall_now_ms: u64,
) -> Result<(), EngineError> {
    if !batch
        .commands
        .iter()
        .any(|command| command.command.is_running_authority_control())
    {
        return process_command_batch(batch, output, state);
    }
    if batch.commands.len() != 1 {
        return Err(EngineError::new(
            EngineErrorCode::InvalidCommand,
            "running-authority control must occupy one command batch",
        ));
    }
    let EngineCommand::RunningAuthority(command) = &batch.commands[0].command else {
        unreachable!("validated single authority command");
    };
    let reservation = output
        .reserve_authority_reply(running_command_reply_reservation(command, running, output))?;
    let sequenced = batch
        .commands
        .into_vec()
        .pop()
        .expect("validated authority batch must contain one command");
    let EngineCommand::RunningAuthority(command) = sequenced.command else {
        return Err(EngineError::new(
            EngineErrorCode::InvalidCommand,
            "running-authority batch lost its typed control command",
        ));
    };
    let event = execute_running_authority_command(
        sequenced.sequence,
        command,
        running,
        wall_now_ms,
        output.max_event_owned_bytes(),
    )?;
    metrics.observe(running);
    reservation.publish(ReliableEvent::RunningAuthority(Box::new(event)))?;
    state.record_processed(1);
    Ok(())
}

/// Reserve the full successful reply or a bounded rejection before mutation.
/// An oversized success becomes a rejection and never needs its oversized bytes.
fn running_command_reply_reservation(
    command: &RunningAuthorityCommand,
    running: &RunningAuthorityLoop,
    output: &OutputQueue,
) -> usize {
    let rejection = (size_of::<RunningAuthorityEvent>() + super::error::MAX_ERROR_DETAIL_BYTES)
        .min(output.max_event_owned_bytes());
    running_response_owned_byte_bound(command, running)
        .ok()
        .filter(|bytes| *bytes <= output.max_event_owned_bytes())
        .map_or(rejection, |bytes| bytes.max(rejection))
}

fn execute_running_authority_command(
    command_sequence: u64,
    command: RunningAuthorityCommand,
    running: &mut RunningAuthorityLoop,
    wall_now_ms: u64,
    event_byte_limit: usize,
) -> Result<RunningAuthorityEvent, EngineError> {
    let result = running_response_owned_byte_bound(&command, running).and_then(|required| {
        if required > event_byte_limit {
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                format!("generation response needs {required} bytes, output event limit is {event_byte_limit}"),
            ));
        }
        match command {
        RunningAuthorityCommand::JoinController(request) => running
            .prepare_controller_join(command_sequence, &request, wall_now_ms)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail)),
        RunningAuthorityCommand::SubmitControllerJoinReceipt(receipt) => running
            .resolve_controller_join(&receipt)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|matched| RunningAuthorityEvent::ControllerJoinResolved {
                command_sequence, request_sequence: receipt.request_sequence, matched,
                accepted: matched && receipt.accepted,
            }),
        RunningAuthorityCommand::ReclaimController(request) => running
            .prepare_controller_reclaim(command_sequence, &request, wall_now_ms)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail)),
        RunningAuthorityCommand::SubmitControllerReclaimReceipt(receipt) => running
            .resolve_controller_reclaim(&receipt)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|matched| RunningAuthorityEvent::ControllerReclaimResolved {
                command_sequence, request_sequence: receipt.request_sequence, matched,
                accepted: matched && receipt.accepted,
            }),
        RunningAuthorityCommand::DisconnectController(close) => running
            .disconnect_controller(&close, wall_now_ms)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|applied| RunningAuthorityEvent::ControllerDisconnected {
                command_sequence, lease_id: close.lease_id, completed_step: running.completed_step(), applied,
            }),
        RunningAuthorityCommand::SubmitControllerAction(action) => running
            .apply_controller_action(command_sequence, &action, wall_now_ms)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|()| RunningAuthorityEvent::ControllerActionApplied {
                command_sequence, lease_id: action.lease_id, completed_step: running.completed_step(),
            }),
        RunningAuthorityCommand::ApplyLiveSettings { updates } => running
            .apply_live_settings(&updates)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(config_revision, config_hash, effective_step)| {
                RunningAuthorityEvent::LiveSettingsApplied {
                    command_sequence,
                    config_revision,
                    config_hash,
                    effective_step,
                }
            }),
        RunningAuthorityCommand::GodModeMove { frame_v1_id, x, y } => running
            .apply_god_mode_move(frame_v1_id, x, y)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(publication, effective_step)| RunningAuthorityEvent::GodModeMoved {
                command_sequence,
                frame_v1_id: publication.frame_v1_id,
                x: publication.x,
                y: publication.y,
                effective_step,
            }),
        RunningAuthorityCommand::GodModeKill { frame_v1_id } => running
            .apply_god_mode_kill(frame_v1_id)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(publication, effective_step)| RunningAuthorityEvent::GodModeKilled {
                command_sequence,
                frame_v1_id: publication.frame_v1_id,
                pellets_dropped: publication.pellets_dropped,
                effective_step,
            }),
        RunningAuthorityCommand::ResurrectHallOfFame {
            managed_directory,
            weights,
        } => running
            .resurrect_hall_of_fame(std::path::Path::new(&managed_directory), &weights)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(publication, effective_step)| RunningAuthorityEvent::HallOfFameResurrected {
                command_sequence,
                frame_v1_id: publication.frame_v1_id,
                effective_step,
            }),
        RunningAuthorityCommand::PublishGenerationCheckpoint {
            managed_directory,
            operation_id,
        } => running
            .publish_pending_generation_checkpoint(
                std::path::Path::new(&managed_directory),
                operation_id,
            )
            .map_err(running_control_error)
            .map(
                |publication| RunningAuthorityEvent::GenerationCheckpointPublished {
                    command_sequence,
                    descriptor: Box::new(publication.descriptor),
                    hall_of_fame_weights: Box::new(publication.hall_of_fame_weights),
                    commit_record: publication.commit_record,
                },
            ),
        RunningAuthorityCommand::AcknowledgeGenerationPersistence { descriptor } => {
            let operation_id = descriptor.operation_id.clone();
            running
                .acknowledge_pending_generation_persistence(&descriptor)
                .map_err(running_control_error)
                .map(
                    |()| RunningAuthorityEvent::GenerationPersistenceAcknowledged {
                        command_sequence,
                        operation_id,
                    },
                )
        }
        RunningAuthorityCommand::PrepareGenerationReassignments => running
            .prepare_acknowledged_generation_reassignments()
            .map_err(running_control_error)
            .and_then(|progress| {
                let (ready, assignments) = match progress {
                    GenerationReassignmentProgress::DeliveryPending(batch) => {
                        (false, own_pending_generation_assignments(batch)?)
                    }
                    GenerationReassignmentProgress::Ready(_) => (true, Box::default()),
                };
                Ok(RunningAuthorityEvent::GenerationReassignmentsPrepared {
                    command_sequence,
                    ready,
                    assignments,
                })
            }),
        RunningAuthorityCommand::SubmitGenerationAssignmentReceipts { receipts } => {
            submit_generation_assignment_receipts(command_sequence, &receipts, running)
        }
        RunningAuthorityCommand::SubmitControllerDeliveryReceipts { receipts } => {
            super::controller_output::submit_receipts(command_sequence, &receipts, running)
        }
        RunningAuthorityCommand::PublishAcknowledgedGenerationStart => running
            .publish_acknowledged_generation_start(wall_now_ms, None)
            .map_err(running_control_error)
            .map(
                |mut resolution| {
                    // The bound uses exactly one record per source controller.
                    // Strip spare capacity from the cold generation-only output.
                    let reservations = &mut resolution.publication.unavailable_controller_reservations;
                    *reservations = std::mem::take(reservations).into_boxed_slice().into_vec();
                    for reservation in reservations {
                        reservation.scope = std::mem::take(&mut reservation.scope).into_boxed_str().into_string();
                        reservation.resume_token = std::mem::take(&mut reservation.resume_token).into_boxed_str().into_string();
                    }
                    RunningAuthorityEvent::GenerationStartPublished {
                        command_sequence,
                        resolution,
                    }
                },
            ),
        RunningAuthorityCommand::StagePreparedImport { slot } => running
            .stage_prepared_import(&slot)
            .map_err(running_control_error)
            .map(|()| RunningAuthorityEvent::ImportStaged { command_sequence }),
        RunningAuthorityCommand::PublishPreparedImport {
            slot,
            descriptor,
            branch_run_id,
        } => running
            .publish_prepared_import(&slot, &descriptor, branch_run_id.as_deref(), wall_now_ms)
            .map_err(running_control_error)
            .map(|publication| RunningAuthorityEvent::ImportPublished {
                command_sequence,
                world_epoch: publication.world_epoch,
                generation: publication.generation,
                completed_step: publication.completed_step,
                population_epoch: publication.population_epoch,
            }),
        RunningAuthorityCommand::CancelPreparedImport { slot } => running
            .cancel_prepared_import(&slot, wall_now_ms)
            .map_err(running_control_error)
            .map(|()| RunningAuthorityEvent::ImportCancelled { command_sequence }),
        }
    });

    match result {
        Ok(event) => Ok(event),
        Err(error)
            if running.state() != RunningAuthorityLoopState::Faulted
                && error.code != EngineErrorCode::Faulted =>
        {
            Ok(RunningAuthorityEvent::CommandRejected {
                command_sequence,
                code: error.code,
                detail: super::error::truncate_utf8(
                    &error.to_string(),
                    event_byte_limit
                        .saturating_sub(size_of::<RunningAuthorityEvent>())
                        .min(super::error::MAX_ERROR_DETAIL_BYTES),
                ),
            })
        }
        Err(error) => Err(EngineError::new(
            EngineErrorCode::Faulted,
            format!("running-authority control failed terminally: {error}"),
        )),
    }
}

/// Derive the largest successful reply from immutable retained Rust data.
/// This executes before file publication, acknowledgement, assignment staging
/// or authority replacement; it never consumes RNG or mutates a barrier.
fn running_response_owned_byte_bound(
    command: &RunningAuthorityCommand,
    running: &RunningAuthorityLoop,
) -> Result<usize, EngineError> {
    let overflow = || {
        EngineError::new(
            EngineErrorCode::QueueByteLimit,
            "generation response byte bound overflowed",
        )
    };
    let dynamic = match command {
        RunningAuthorityCommand::ReclaimController(_)
        | RunningAuthorityCommand::JoinController(_) => {
            super::external_replacement::RESUME_TOKEN_LENGTH
        }
        RunningAuthorityCommand::SubmitControllerReclaimReceipt(_)
        | RunningAuthorityCommand::SubmitControllerJoinReceipt(_) => 0,
        RunningAuthorityCommand::PublishGenerationCheckpoint { .. } => {
            match running.pending_generation_transition() {
                Some(pending) => {
                    let descriptor_bytes = match pending.checkpoint_descriptor() {
                        Some(descriptor) => descriptor.owned_bytes(),
                        None => {
                            super::checkpoint::CheckpointDescriptor::publication_owned_byte_bound(
                                &pending.candidate().identity.run_id,
                            )
                            .ok_or_else(overflow)?
                        }
                    };
                    size_of::<super::checkpoint::CheckpointDescriptor>()
                        .checked_add(descriptor_bytes)
                        .and_then(|bytes| {
                            bytes.checked_add(size_of::<
                                super::checkpoint::HallOfFameWeightsDescriptor,
                            >())
                        })
                        .and_then(|bytes| {
                            bytes.checked_add(running.pending_hall_of_fame_weights_descriptor().map_or(
                                super::checkpoint::HallOfFameWeightsDescriptor::PUBLICATION_OWNED_BYTES,
                                |descriptor| descriptor.owned_bytes(),
                            ))
                        })
                        .ok_or_else(overflow)?
                }
                None => 0, // Invalid-phase rejection cannot publish a descriptor.
            }
        }
        RunningAuthorityCommand::AcknowledgeGenerationPersistence { descriptor } => {
            descriptor.operation_id.owned_bytes()
        }
        RunningAuthorityCommand::PrepareGenerationReassignments => {
            let count = running
                .generation_source_controller_leases()
                .iter()
                .filter(|lease| {
                    lease.status == super::state::ControllerLeaseStatus::Connected
                        && lease.connection_id.is_some()
                })
                .count();
            count
                .checked_mul(
                    size_of::<RunningGenerationAssignment>()
                        + super::external_replacement::RESUME_TOKEN_LENGTH,
                )
                .ok_or_else(overflow)?
        }
        RunningAuthorityCommand::SubmitGenerationAssignmentReceipts { .. }
        | RunningAuthorityCommand::SubmitControllerDeliveryReceipts { .. }
        | RunningAuthorityCommand::SubmitControllerAction(_)
        | RunningAuthorityCommand::DisconnectController(_) => 0,
        // A canonical `sha256:` identity is 71 bytes; allow allocator rounding too.
        RunningAuthorityCommand::ApplyLiveSettings { .. } => 128,
        RunningAuthorityCommand::GodModeMove { .. } => 0,
        RunningAuthorityCommand::GodModeKill { .. } => 0,
        RunningAuthorityCommand::ResurrectHallOfFame { .. } => 0,
        RunningAuthorityCommand::PublishAcknowledgedGenerationStart => {
            // Every unavailable record is a unique old-controller outcome and
            // retains exactly that source controller's scope and known token.
            running
                .generation_source_controller_leases()
                .iter()
                .try_fold(0usize, |bytes, lease| {
                    bytes
                        .checked_add(size_of::<
                            super::external_replacement::UnavailableControllerReservation,
                        >())
                        .and_then(|bytes| bytes.checked_add(lease.scope.len()))
                        .and_then(|bytes| bytes.checked_add(lease.resume_token.len()))
                        .ok_or_else(overflow)
                })?
        }
        RunningAuthorityCommand::StagePreparedImport { .. }
        | RunningAuthorityCommand::PublishPreparedImport { .. }
        | RunningAuthorityCommand::CancelPreparedImport { .. } => 0,
    };
    size_of::<RunningAuthorityEvent>()
        .checked_add(dynamic)
        .ok_or_else(overflow)
}

fn own_pending_generation_assignments(
    batch: ExternalObservationBatch<'_>,
) -> Result<Box<[RunningGenerationAssignment]>, EngineError> {
    let pending_count = batch.remaining();
    let mut assignments = Vec::new();
    assignments
        .try_reserve_exact(pending_count)
        .map_err(|error| {
            EngineError::new(
                EngineErrorCode::QueueCountLimit,
                format!("failed to reserve generation assignment metadata: {error}"),
            )
        })?;
    for (index, event) in batch.events().iter().copied().enumerate() {
        if batch.status(index) != Some(ExternalDeliveryStatus::Pending) {
            continue;
        }
        let ExternalDeliveryEventKind::ReplacementAssignment { frame_v1_id } = event.delivery_kind
        else {
            return Err(EngineError::new(
                EngineErrorCode::Faulted,
                "generation reassignment batch contained a non-assignment event",
            ));
        };
        let token = batch.resume_token(index).ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::Faulted,
                "generation reassignment omitted its Rust-owned resume token",
            )
        })?;
        let mut resume_token = String::new();
        resume_token
            .try_reserve_exact(token.len())
            .map_err(|error| {
                EngineError::new(
                    EngineErrorCode::QueueByteLimit,
                    format!("failed to reserve generation assignment token: {error}"),
                )
            })?;
        resume_token.push_str(token);
        assignments.push(RunningGenerationAssignment {
            operation_epoch: event.step_key.operation_epoch(),
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            controller_kind: event.controller_kind,
            snake_id: event.snake_id,
            frame_v1_id,
            resume_token: resume_token.into_boxed_str(),
        });
    }
    if assignments.len() != pending_count {
        return Err(EngineError::new(
            EngineErrorCode::Faulted,
            "generation assignment bridge count did not match retained pending status",
        ));
    }
    Ok(assignments.into_boxed_slice())
}

fn submit_generation_assignment_receipts(
    command_sequence: u64,
    receipts: &[ExternalDeliveryReceipt],
    running: &mut RunningAuthorityLoop,
) -> Result<RunningAuthorityEvent, EngineError> {
    if running.state() != RunningAuthorityLoopState::GenerationTransitionPending {
        return Err(EngineError::new(
            EngineErrorCode::InvalidCommand,
            "generation assignment receipts require a retained generation transition",
        ));
    }
    let Some(batch) = running.pending_external_delivery() else {
        return Err(EngineError::new(
            EngineErrorCode::InvalidCommand,
            "generation assignment receipts require a prepared retained assignment batch",
        ));
    };
    let (exact_results, bridge_ignored) =
        super::controller_output::correlate_receipts(batch, receipts)?;
    let resolution = running
        .submit_external_delivery_results(&exact_results, None)
        .map_err(running_control_error)?;
    let state =
        match resolution.state {
            RunningAuthorityDeliveryState::GenerationAssignmentsPending { remaining, .. } => {
                GenerationAssignmentReceiptState::Pending { remaining }
            }
            RunningAuthorityDeliveryState::GenerationAssignmentsReady {
                source_key,
                successor_generation,
                successor_completed_step,
                ..
            } => GenerationAssignmentReceiptState::Ready {
                source_key,
                successor_generation,
                successor_completed_step,
            },
            _ => return Err(EngineError::new(
                EngineErrorCode::Faulted,
                "generation assignment receipt resolved outside the retained generation barrier",
            )),
        };
    Ok(RunningAuthorityEvent::GenerationAssignmentReceiptsApplied {
        command_sequence,
        matched_acceptances: resolution.matched_acceptances,
        matched_failures: resolution.matched_failures,
        ignored_receipts: resolution.ignored_results.saturating_add(bridge_ignored),
        state,
    })
}

fn running_control_error(error: super::running_loop::RunningAuthorityLoopError) -> EngineError {
    EngineError::new(EngineErrorCode::InvalidCommand, error.to_string())
}

fn monotonic_elapsed_ms(origin: Instant) -> Result<u64, EngineError> {
    u64::try_from(origin.elapsed().as_millis()).map_err(|_| {
        EngineError::new(
            EngineErrorCode::Faulted,
            "background monotonic clock exceeded the supported millisecond range",
        )
    })
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
        RunningAuthorityLoopState::ControllerReclaimPending => 4,
        RunningAuthorityLoopState::ControllerJoinPending => 5,
        RunningAuthorityLoopState::ExternalDeliveryPending => 1,
        RunningAuthorityLoopState::GenerationTransitionPending => 2,
        RunningAuthorityLoopState::ImportPending => 6,
        RunningAuthorityLoopState::Faulted => 3,
    }
}

fn loop_state_from_code(code: u8) -> RunningAuthorityLoopState {
    match code {
        0 => RunningAuthorityLoopState::Ready,
        4 => RunningAuthorityLoopState::ControllerReclaimPending,
        5 => RunningAuthorityLoopState::ControllerJoinPending,
        1 => RunningAuthorityLoopState::ExternalDeliveryPending,
        2 => RunningAuthorityLoopState::GenerationTransitionPending,
        6 => RunningAuthorityLoopState::ImportPending,
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

#[cfg(all(test, feature = "engine-test-hooks"))]
mod tests {
    use super::*;
    use crate::engine::checkpoint::{CheckpointDescriptor, CheckpointOperationId};
    use crate::engine::contract::{CompletedEvent, SequencedCommand, ENGINE_CONTRACT_VERSION};
    use crate::engine::generation_handoff_fixture::{
        background_generation_handoff_disconnected_fixture, background_generation_handoff_fixture,
        background_generation_handoff_runtime_init,
    };
    use crate::engine::queues::NoopWakeSink;
    use crate::engine::runtime::EngineRuntime;
    use std::path::PathBuf;

    /// A process-unique test-owned directory; no owner save path is used.
    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create() -> Self {
            let path = std::env::temp_dir().join(format!(
                "slither-generation-output-capacity-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn publish_command(&self) -> RunningAuthorityCommand {
            RunningAuthorityCommand::PublishGenerationCheckpoint {
                managed_directory: self.0.to_str().unwrap().to_owned(),
                operation_id: CheckpointOperationId::parse("51515151515151515151515151515151")
                    .unwrap(),
            }
        }

        fn file_count(&self) -> usize {
            std::fs::read_dir(&self.0).unwrap().count()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ignored = std::fs::remove_dir_all(&self.0);
        }
    }

    fn output_with_cap(cap: usize) -> OutputQueue {
        let mut limits = background_generation_handoff_runtime_init().output;
        limits.max_event_owned_bytes = cap;
        OutputQueue::new(limits, Arc::new(NoopWakeSink))
    }

    fn advance_to_transition(running: &mut RunningAuthorityLoop) {
        let output = output_with_cap(1024 * 1024);
        assert!(matches!(
            service_running_with_output_capacity(running, &output, 0).unwrap(),
            RunningAuthorityLoopProgress::Idle { .. }
        ));
        assert!(matches!(
            service_running_with_output_capacity(running, &output, 500).unwrap(),
            RunningAuthorityLoopProgress::GenerationTransitionPending { .. }
        ));
    }

    /// Exercise the real command processor and queue byte accounting together.
    fn command_reply(
        running: &mut RunningAuthorityLoop,
        command: RunningAuthorityCommand,
        cap: usize,
    ) -> RunningAuthorityEvent {
        let output = output_with_cap(cap);
        let state = CoordinatorState::new();
        let metrics = RunningAuthorityMetrics::new(running);
        process_running_command_batch(
            CommandBatch {
                contract_version: ENGINE_CONTRACT_VERSION,
                commands: vec![SequencedCommand {
                    sequence: 1,
                    command: EngineCommand::RunningAuthority(command),
                }]
                .into_boxed_slice(),
            },
            &output,
            &state,
            running,
            &metrics,
            1000,
        )
        .unwrap();
        assert_eq!(state.processed(), (1, 1));
        let mut events = output.drain(usize::MAX, usize::MAX).events;
        assert_eq!(events.len(), 1);
        let event = events.pop().unwrap();
        assert!(event.owned_bytes() <= cap);
        match event {
            CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) => *event,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn assert_byte_rejection(event: RunningAuthorityEvent) {
        assert!(matches!(
            event,
            RunningAuthorityEvent::CommandRejected {
                code: EngineErrorCode::QueueByteLimit,
                ..
            }
        ));
    }

    #[test]
    fn production_step_histogram_reports_bounded_conservative_percentiles() {
        let running = background_generation_handoff_fixture().unwrap().running;
        let metrics = RunningAuthorityMetrics::new(&running);
        for _ in 0..95 {
            metrics.record_step_duration(Duration::from_micros(150));
        }
        for _ in 0..4 {
            metrics.record_step_duration(Duration::from_micros(900));
        }
        metrics.record_step_duration(Duration::from_micros(300_000));

        let health = metrics.snapshot();
        assert_eq!(health.step_timing_samples, 100);
        assert_eq!(health.step_timing_total_micros, 317_850);
        assert_eq!(health.step_timing_max_micros, 300_000);
        assert_eq!(health.step_timing_p95_micros, 200);
        assert_eq!(health.step_timing_p99_micros, 1_000);
    }

    /// Resume the existing connected-controller fixture into a normal generation.
    fn ordinary_controller_loop() -> RunningAuthorityLoop {
        let managed = TestDirectory::create();
        let mut running = background_generation_handoff_fixture().unwrap().running;
        advance_to_transition(&mut running);
        publish_and_acknowledge(&mut running, &managed);
        let assignments = match command_reply(
            &mut running,
            RunningAuthorityCommand::PrepareGenerationReassignments,
            1024 * 1024,
        ) {
            RunningAuthorityEvent::GenerationReassignmentsPrepared { assignments, .. } => {
                assignments
            }
            other => panic!("missing assignments: {other:?}"),
        };
        let receipts = assignments
            .iter()
            .map(|assignment| ExternalDeliveryReceipt {
                operation_epoch: assignment.operation_epoch,
                event_sequence: assignment.event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id,
                accepted: true,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        assert!(matches!(
            command_reply(
                &mut running,
                RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                    receipts: receipts.clone()
                },
                1024 * 1024
            ),
            RunningAuthorityEvent::CommandRejected { .. }
        ));
        command_reply(
            &mut running,
            RunningAuthorityCommand::SubmitGenerationAssignmentReceipts { receipts },
            1024 * 1024,
        );
        command_reply(
            &mut running,
            RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
            1024 * 1024,
        );
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        running
    }

    #[test]
    fn fresh_join_capacity_and_receipts_preserve_source_and_retry_once() {
        use crate::engine::contract::{ControllerJoinRequest, ControllerReclaimReceipt};
        let mut running = ordinary_controller_loop();
        let origin = Instant::now();
        running.set_background_clock(origin);
        let before = running.generation_source_controller_leases().to_vec();
        let request = RunningAuthorityCommand::JoinController(Box::new(ControllerJoinRequest {
            connection_id: 999,
            kind: before[0].kind,
            identity_key: "player:new-owner".into(),
            received_at: origin + Duration::from_millis(1250),
        }));
        let bound = running_response_owned_byte_bound(&request, &running).unwrap();
        assert!(matches!(
            execute_running_authority_command(100, request.clone(), &mut running, 1500, bound - 1)
                .unwrap(),
            RunningAuthorityEvent::CommandRejected {
                code: EngineErrorCode::QueueByteLimit,
                ..
            }
        ));
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        assert_eq!(running.generation_source_controller_leases(), before);
        let assignment =
            execute_running_authority_command(100, request.clone(), &mut running, 1500, bound)
                .unwrap();
        let RunningAuthorityEvent::ControllerJoinAssignment {
            lease_id,
            frame_v1_id,
            ..
        } = assignment
        else {
            panic!("fresh assignment expected: {assignment:?}");
        };
        assert_eq!(running.generation_source_controller_leases(), before);
        let mut receipt = ControllerReclaimReceipt {
            request_sequence: 100,
            connection_id: 999,
            lease_id,
            accepted: false,
        };
        assert!(!running.resolve_controller_reclaim(&receipt).unwrap());
        assert!(running.resolve_controller_join(&receipt).unwrap());
        assert_eq!(running.generation_source_controller_leases(), before);
        let retry =
            execute_running_authority_command(101, request, &mut running, 1600, bound).unwrap();
        assert!(
            matches!(retry, RunningAuthorityEvent::ControllerJoinAssignment { lease_id: next, frame_v1_id: frame, .. }
            if next == lease_id && frame == frame_v1_id)
        );
        receipt.request_sequence = 101;
        receipt.accepted = true;
        let completion = RunningAuthorityCommand::SubmitControllerJoinReceipt(receipt.clone());
        let completion_bound = running_response_owned_byte_bound(&completion, &running).unwrap();
        assert!(matches!(
            execute_running_authority_command(
                102,
                completion.clone(),
                &mut running,
                1700,
                completion_bound - 1
            )
            .unwrap(),
            RunningAuthorityEvent::CommandRejected {
                code: EngineErrorCode::QueueByteLimit,
                ..
            }
        ));
        assert_eq!(running.generation_source_controller_leases(), before);
        assert_eq!(
            running.state(),
            RunningAuthorityLoopState::ControllerJoinPending
        );
        assert!(matches!(
            running
                .service_after_command_drain(1800, SchedulerServiceMode::Background, None)
                .unwrap(),
            RunningAuthorityLoopProgress::ControllerJoinPending
        ));
        execute_running_authority_command(102, completion, &mut running, 1800, completion_bound)
            .unwrap();
        let after = running.generation_source_controller_leases().to_vec();
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(&after[..before.len()], before);
        assert_eq!(after.last().unwrap().latest_action.accepted_at_ms, 1250);
        assert!(!running.resolve_controller_join(&receipt).unwrap());
        assert_eq!(running.generation_source_controller_leases(), after);
    }

    #[test]
    fn reclaim_capacity_and_receipts_preserve_source_and_retry_once() {
        use crate::engine::contract::{ControllerReclaimReceipt, ControllerReclaimRequest};
        let mut running = ordinary_controller_loop();
        let origin = Instant::now();
        running.set_background_clock(origin);
        let before = running.generation_source_controller_leases()[0].clone();
        let request =
            RunningAuthorityCommand::ReclaimController(Box::new(ControllerReclaimRequest {
                connection_id: 999,
                kind: before.kind,
                resume_token: before.resume_token.clone(),
                identity_key: String::new(),
                received_at: origin + Duration::from_millis(1250),
            }));
        let bound = running_response_owned_byte_bound(&request, &running).unwrap();
        assert!(matches!(
            execute_running_authority_command(100, request.clone(), &mut running, 1500, bound - 1)
                .unwrap(),
            RunningAuthorityEvent::CommandRejected {
                code: EngineErrorCode::QueueByteLimit,
                ..
            }
        ));
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        assert_eq!(running.generation_source_controller_leases()[0], before);
        let output = output_with_cap(1024 * 1024);
        let fill = || {
            for sequence in 0..background_generation_handoff_runtime_init()
                .output
                .max_reliable
            {
                output
                    .push_reliable(ReliableEvent::ProbeResult {
                        sequence: sequence as u64,
                        correlation_id: 0,
                        payload: Vec::new(),
                    })
                    .unwrap();
            }
        };
        let batch = |sequence, command| CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence,
                command: EngineCommand::RunningAuthority(command),
            }]
            .into_boxed_slice(),
        };
        let metrics = RunningAuthorityMetrics::new(&running);
        let state = CoordinatorState::new();
        fill();
        assert!(process_running_command_batch(
            batch(100, request.clone()),
            &output,
            &state,
            &mut running,
            &metrics,
            1500
        )
        .is_err());
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        output.drain(usize::MAX, usize::MAX);
        process_running_command_batch(
            batch(100, request.clone()),
            &output,
            &state,
            &mut running,
            &metrics,
            1500,
        )
        .unwrap();
        let event = output.drain(usize::MAX, usize::MAX).events.pop().unwrap();
        assert!(
            matches!(event, CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) if matches!(*event,
            RunningAuthorityEvent::ControllerReclaimAssignment { request_sequence: 100, connection_id: 999, .. }))
        );
        assert_eq!(running.generation_source_controller_leases()[0], before);
        assert_eq!(
            running.state(),
            RunningAuthorityLoopState::ControllerReclaimPending
        );
        assert_eq!(metrics.snapshot().pending_external_deliveries, 1);
        let mut receipt = ControllerReclaimReceipt {
            request_sequence: 100,
            connection_id: 998,
            lease_id: before.id,
            accepted: true,
        };
        assert!(!running.resolve_controller_reclaim(&receipt).unwrap());
        assert!(matches!(
            running
                .service_after_command_drain(1800, SchedulerServiceMode::Background, None)
                .unwrap(),
            RunningAuthorityLoopProgress::ControllerReclaimPending
        ));
        receipt.connection_id = 999;
        receipt.accepted = false;
        assert!(running.resolve_controller_reclaim(&receipt).unwrap());
        assert_eq!(running.generation_source_controller_leases()[0], before);
        process_running_command_batch(
            batch(101, request),
            &output,
            &state,
            &mut running,
            &metrics,
            1800,
        )
        .unwrap();
        output.drain(usize::MAX, usize::MAX);
        receipt.request_sequence = 101;
        receipt.accepted = true;
        let completion = RunningAuthorityCommand::SubmitControllerReclaimReceipt(receipt.clone());
        fill();
        assert!(process_running_command_batch(
            batch(102, completion.clone()),
            &output,
            &state,
            &mut running,
            &metrics,
            1900
        )
        .is_err());
        assert_eq!(running.generation_source_controller_leases()[0], before);
        assert_eq!(
            running.state(),
            RunningAuthorityLoopState::ControllerReclaimPending
        );
        output.drain(usize::MAX, usize::MAX);
        process_running_command_batch(
            batch(102, completion),
            &output,
            &state,
            &mut running,
            &metrics,
            1900,
        )
        .unwrap();
        let after = running.generation_source_controller_leases()[0].clone();
        assert_eq!(after.connection_id, Some(999));
        assert_eq!(after.snake_id, before.snake_id);
        assert_ne!(after.resume_token, before.resume_token);
        assert!(!running.resolve_controller_reclaim(&receipt).unwrap());
        assert_eq!(running.generation_source_controller_leases()[0], after);
    }

    #[test]
    fn disconnect_output_capacity_preserves_lease_and_original_close_time() {
        let mut running = ordinary_controller_loop();
        let origin = Instant::now();
        running.set_background_clock(origin);
        let before = running.generation_source_controller_leases()[0].clone();
        let close = super::super::contract::ControllerDisconnectRequest {
            lease_id: before.id,
            connection_id: before.connection_id.unwrap(),
            received_at: origin + Duration::from_millis(1250),
        };
        let batch = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 100,
                command: EngineCommand::RunningAuthority(
                    RunningAuthorityCommand::DisconnectController(close),
                ),
            }]
            .into_boxed_slice(),
        };
        let output = output_with_cap(1024 * 1024);
        for sequence in 0..background_generation_handoff_runtime_init()
            .output
            .max_reliable
        {
            output
                .push_reliable(ReliableEvent::ProbeResult {
                    sequence: sequence as u64,
                    correlation_id: 0,
                    payload: Vec::new(),
                })
                .unwrap();
        }
        let metrics = RunningAuthorityMetrics::new(&running);
        let state = CoordinatorState::new();
        assert!(process_running_command_batch(
            batch.clone(),
            &output,
            &state,
            &mut running,
            &metrics,
            1500
        )
        .is_err());
        assert_eq!(running.generation_source_controller_leases()[0], before);
        output.drain(usize::MAX, usize::MAX);
        process_running_command_batch(batch.clone(), &output, &state, &mut running, &metrics, 1500)
            .unwrap();
        let after = running.generation_source_controller_leases()[0].clone();
        assert_eq!(after.connection_id, None);
        assert_eq!(after.disconnected_at_ms, Some(1250));
        assert_eq!(after.grace_expires_at_ms, Some(31_250));
        assert_eq!(after.last_observed_at_ms, 1500);
        process_running_command_batch(batch, &output, &state, &mut running, &metrics, 1700)
            .unwrap();
        assert_eq!(running.generation_source_controller_leases()[0], after);
        assert_eq!(running.completed_step(), 1);
    }

    #[test]
    fn action_output_capacity_precedes_lease_mutation_and_exact_retry() {
        let mut running = ordinary_controller_loop();
        let origin = Instant::now();
        running.set_background_clock(origin);
        let before = running.generation_source_controller_leases()[0].clone();
        let action = super::super::contract::ControllerActionRequest {
            lease_id: before.id,
            connection_id: before.connection_id.unwrap(),
            turn: 0.75,
            boost: false,
            client_tick: 42,
            received_at: origin + Duration::from_millis(1200),
        };
        let batch = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 100,
                command: EngineCommand::RunningAuthority(
                    RunningAuthorityCommand::SubmitControllerAction(action),
                ),
            }]
            .into_boxed_slice(),
        };
        let output = output_with_cap(1024 * 1024);
        for sequence in 0..background_generation_handoff_runtime_init()
            .output
            .max_reliable
        {
            output
                .push_reliable(ReliableEvent::ProbeResult {
                    sequence: sequence as u64,
                    correlation_id: 0,
                    payload: Vec::new(),
                })
                .unwrap();
        }
        let metrics = RunningAuthorityMetrics::new(&running);
        let state = CoordinatorState::new();
        assert!(process_running_command_batch(
            batch.clone(),
            &output,
            &state,
            &mut running,
            &metrics,
            1200
        )
        .is_err());
        assert_eq!(running.generation_source_controller_leases()[0], before);
        output.drain(usize::MAX, usize::MAX);
        process_running_command_batch(batch, &output, &state, &mut running, &metrics, 1200)
            .unwrap();
        let after = &running.generation_source_controller_leases()[0];
        assert_eq!(after.latest_action.turn, 0.75);
        assert_eq!(after.latest_action.arrival_sequence, 100);
        assert_eq!(after.latest_action.accepted_at_ms, 1200);
        assert_eq!(running.completed_step(), 1);
    }

    #[test]
    fn ordinary_controller_capacity_rejects_before_preparation_and_retries_once() {
        let mut running = ordinary_controller_loop();
        let before = RunningAuthorityMetrics::new(&running).snapshot();
        let bound = super::super::controller_output::service_reply_bound(&running).unwrap();
        let small = output_with_cap(bound - 1);
        assert_eq!(
            service_running_with_output_capacity(&mut running, &small, 1200)
                .unwrap_err()
                .code,
            EngineErrorCode::QueueByteLimit
        );
        assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
        assert!(running.pending_external_delivery().is_none());
        let output = output_with_cap(bound);
        assert!(matches!(
            service_running_with_output_capacity(&mut running, &output, 1200).unwrap(),
            RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
        ));
        let messages = super::super::controller_output::own_pending_messages(&running).unwrap();
        assert!(!messages.is_empty());
        let event = RunningAuthorityEvent::ControllerMessages {
            ticket_sequence: 2,
            messages: messages.clone(),
        };
        assert!(size_of::<RunningAuthorityEvent>() + event.owned_bytes() <= bound);
        let receipts: Vec<_> = messages
            .iter()
            .map(|message| ExternalDeliveryReceipt {
                operation_epoch: message.event.step_key.operation_epoch(),
                event_sequence: message.event.event_sequence,
                connection_id: message.event.connection_id,
                lease_id: message.event.lease_id,
                accepted: true,
            })
            .collect();
        let stale = ExternalDeliveryReceipt {
            connection_id: u64::MAX,
            ..receipts[0]
        };
        assert!(matches!(
            command_reply(
                &mut running,
                RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                    receipts: vec![stale].into_boxed_slice()
                },
                1024 * 1024
            ),
            RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
                matched_acceptances: 0,
                ignored_receipts: 1,
                published_completed_step: None,
                ..
            }
        ));
        assert_eq!(
            super::super::controller_output::own_pending_messages(&running).unwrap(),
            messages
        );
        let mut duplicate_receipts = receipts;
        duplicate_receipts.push(duplicate_receipts[0]);
        let command = RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
            receipts: duplicate_receipts.into_boxed_slice(),
        };
        let batch = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::RunningAuthority(command.clone()),
            }]
            .into_boxed_slice(),
        };
        let state = CoordinatorState::new();
        let metrics = RunningAuthorityMetrics::new(&running);
        let mut byte_limits = background_generation_handoff_runtime_init().output;
        byte_limits.max_event_owned_bytes = bound;
        byte_limits.max_reliable_owned_bytes = bound;
        let byte_full = OutputQueue::new(byte_limits, Arc::new(NoopWakeSink));
        byte_full
            .push_reliable(ReliableEvent::ProbeResult {
                sequence: 0,
                correlation_id: 0,
                payload: vec![0; bound],
            })
            .unwrap();
        assert_eq!(
            process_running_command_batch(
                batch.clone(),
                &byte_full,
                &state,
                &mut running,
                &metrics,
                1200
            )
            .unwrap_err()
            .code,
            EngineErrorCode::QueueByteLimit
        );
        assert_eq!(running.completed_step(), before.completed_step);
        assert_eq!(
            super::super::controller_output::own_pending_messages(&running).unwrap(),
            messages
        );

        // A full queue retains the exact completion for retry before publication.
        let output = output_with_cap(1024 * 1024);
        for sequence in 0..background_generation_handoff_runtime_init()
            .output
            .max_reliable
        {
            output
                .push_reliable(ReliableEvent::ProbeResult {
                    sequence: sequence as u64,
                    correlation_id: 0,
                    payload: Vec::new(),
                })
                .unwrap();
        }
        assert_eq!(
            process_running_command_batch(
                batch.clone(),
                &output,
                &state,
                &mut running,
                &metrics,
                1200
            )
            .unwrap_err()
            .code,
            EngineErrorCode::QueueCountLimit
        );
        assert_eq!(running.completed_step(), before.completed_step);
        assert_eq!(
            super::super::controller_output::own_pending_messages(&running).unwrap(),
            messages
        );
        output.drain(usize::MAX, usize::MAX);
        process_running_command_batch(batch, &output, &state, &mut running, &metrics, 1200)
            .unwrap();
        let result = output.drain(usize::MAX, usize::MAX).events.pop().unwrap();
        assert!(
            matches!(result, CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) if matches!(*event, RunningAuthorityEvent::ControllerDeliveryReceiptsApplied { matched_acceptances, ignored_receipts: 1, remaining: 0, published_completed_step: Some(_), .. } if matched_acceptances == messages.len()))
        );
        assert_eq!(running.completed_step(), before.completed_step + 1);
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        assert!(matches!(
            command_reply(&mut running, command, 1024 * 1024),
            RunningAuthorityEvent::CommandRejected { .. }
        ));
        assert_eq!(running.completed_step(), before.completed_step + 1);
    }

    fn publish_and_acknowledge(
        running: &mut RunningAuthorityLoop,
        managed: &TestDirectory,
    ) -> CheckpointDescriptor {
        let command = managed.publish_command();
        let bound = running_response_owned_byte_bound(&command, running).unwrap();
        let descriptor = match command_reply(running, command, bound) {
            RunningAuthorityEvent::GenerationCheckpointPublished { descriptor, .. } => *descriptor,
            other => panic!("checkpoint did not publish: {other:?}"),
        };
        assert!(matches!(
            command_reply(
                running,
                RunningAuthorityCommand::AcknowledgeGenerationPersistence {
                    descriptor: Box::new(descriptor.clone()),
                },
                1024 * 1024,
            ),
            RunningAuthorityEvent::GenerationPersistenceAcknowledged { .. }
        ));
        descriptor
    }

    #[test]
    fn generation_checkpoint_and_ack_byte_limits_reject_before_mutation_and_allow_exact_retry() {
        let managed = TestDirectory::create();
        let mut running = background_generation_handoff_fixture().unwrap().running;
        let old_epoch = running.world_epoch();
        advance_to_transition(&mut running);
        let command = managed.publish_command();
        let bound = running_response_owned_byte_bound(&command, &running).unwrap();
        assert_byte_rejection(command_reply(&mut running, command, bound - 1));
        assert_eq!(managed.file_count(), 0);
        assert!(running
            .pending_generation_transition()
            .unwrap()
            .checkpoint_descriptor()
            .is_none());
        assert_eq!(running.world_epoch(), old_epoch);
        let publication = command_reply(&mut running, managed.publish_command(), bound);
        assert_eq!(
            size_of::<RunningAuthorityEvent>() + publication.owned_bytes(),
            bound
        );
        let (descriptor, record) = match publication {
            RunningAuthorityEvent::GenerationCheckpointPublished {
                descriptor,
                commit_record,
                ..
            } => (*descriptor, commit_record),
            other => panic!("checkpoint did not publish: {other:?}"),
        };
        assert_eq!(
            descriptor.owned_bytes(),
            CheckpointDescriptor::publication_owned_byte_bound(&descriptor.run_id).unwrap()
        );
        assert!(matches!(
            command_reply(&mut running, managed.publish_command(), bound),
            RunningAuthorityEvent::GenerationCheckpointPublished { descriptor: retry, commit_record, .. }
                if *retry == descriptor && commit_record == record
        ));
        assert_eq!(managed.file_count(), 2);
        let ack = RunningAuthorityCommand::AcknowledgeGenerationPersistence {
            descriptor: Box::new(descriptor),
        };
        let ack_bound = running_response_owned_byte_bound(&ack, &running).unwrap();
        assert_byte_rejection(command_reply(&mut running, ack.clone(), ack_bound - 1));
        assert!(!running
            .pending_generation_transition()
            .unwrap()
            .persistence_acknowledged());
        assert!(running.pending_external_delivery().is_none());
        assert_eq!(running.world_epoch(), old_epoch);
        assert!(matches!(
            command_reply(&mut running, ack, ack_bound),
            RunningAuthorityEvent::GenerationPersistenceAcknowledged { .. }
        ));
        assert!(running
            .pending_generation_transition()
            .unwrap()
            .persistence_acknowledged());
        assert_eq!(running.world_epoch(), old_epoch);
    }

    #[test]
    fn generation_assignment_limit_rejects_before_staging_and_accepts_exact_sized_output() {
        let managed = TestDirectory::create();
        let mut running = background_generation_handoff_fixture().unwrap().running;
        let old_epoch = running.world_epoch();
        advance_to_transition(&mut running);
        publish_and_acknowledge(&mut running, &managed);
        let command = RunningAuthorityCommand::PrepareGenerationReassignments;
        let bound = running_response_owned_byte_bound(&command, &running).unwrap();
        assert_byte_rejection(command_reply(&mut running, command.clone(), bound - 1));
        assert!(running.pending_external_delivery().is_none());
        assert_eq!(running.world_epoch(), old_epoch);
        let reply = command_reply(&mut running, command, bound);
        assert_eq!(
            size_of::<RunningAuthorityEvent>() + reply.owned_bytes(),
            bound
        );
        let assignment = match reply {
            RunningAuthorityEvent::GenerationReassignmentsPrepared {
                ready: false,
                assignments,
                ..
            } => {
                assert_eq!(assignments.len(), 1);
                assignments[0].clone()
            }
            other => panic!("assignment did not stage: {other:?}"),
        };
        assert_eq!(running.pending_external_delivery().unwrap().remaining(), 1);
        let receipts = RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
            receipts: vec![ExternalDeliveryReceipt {
                operation_epoch: assignment.operation_epoch,
                event_sequence: assignment.event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id,
                accepted: true,
            }]
            .into_boxed_slice(),
        };
        command_reply(&mut running, receipts.clone(), 1024 * 1024);
        assert!(matches!(
            command_reply(
                &mut running,
                RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
                1024 * 1024
            ),
            RunningAuthorityEvent::GenerationStartPublished { .. }
        ));
        assert_ne!(running.world_epoch(), old_epoch);
        assert_eq!(running.generation(), 2);
        let output = output_with_cap(1024 * 1024);
        assert!(matches!(
            service_running_with_output_capacity(&mut running, &output, 1200).unwrap(),
            RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
        ));
        let before = RunningAuthorityMetrics::new(&running).snapshot();
        let pending = running.pending_external_delivery().unwrap();
        let before_events = pending.events().to_vec();
        let before_observations: Vec<_> = (0..before_events.len())
            .map(|index| pending.observation(index).map(<[f32]>::to_vec))
            .collect();
        let before_statuses: Vec<_> = (0..before_events.len())
            .map(|index| pending.status(index))
            .collect();
        assert!(matches!(
            command_reply(&mut running, receipts, 1024 * 1024),
            RunningAuthorityEvent::CommandRejected {
                code: EngineErrorCode::InvalidCommand,
                ..
            }
        ));
        assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
        let pending = running.pending_external_delivery().unwrap();
        assert_eq!(pending.events(), before_events);
        for index in 0..before_events.len() {
            assert_eq!(
                pending.observation(index),
                before_observations[index].as_deref()
            );
            assert_eq!(pending.status(index), before_statuses[index]);
        }
    }

    #[test]
    fn generation_final_reply_limit_preserves_old_authority_and_retained_unavailable_token() {
        let managed = TestDirectory::create();
        let mut running = background_generation_handoff_disconnected_fixture()
            .unwrap()
            .running;
        let old_epoch = running.world_epoch();
        let old_token = running.generation_source_controller_leases()[0]
            .resume_token
            .clone();
        advance_to_transition(&mut running);
        publish_and_acknowledge(&mut running, &managed);
        assert!(matches!(
            command_reply(
                &mut running,
                RunningAuthorityCommand::PrepareGenerationReassignments,
                1024 * 1024
            ),
            RunningAuthorityEvent::GenerationReassignmentsPrepared { ready: true, .. }
        ));
        let command = RunningAuthorityCommand::PublishAcknowledgedGenerationStart;
        let bound = running_response_owned_byte_bound(&command, &running).unwrap();
        assert_byte_rejection(command_reply(&mut running, command.clone(), bound - 1));
        assert_eq!(running.world_epoch(), old_epoch);
        assert_eq!(running.generation(), 1);
        assert_eq!(running.completed_step(), 0);
        assert!(running
            .pending_generation_transition()
            .unwrap()
            .persistence_acknowledged());
        let reply = command_reply(&mut running, command.clone(), bound);
        assert_eq!(
            size_of::<RunningAuthorityEvent>() + reply.owned_bytes(),
            bound
        );
        match reply {
            RunningAuthorityEvent::GenerationStartPublished { resolution, .. } => {
                let unavailable = resolution.publication.unavailable_controller_reservations;
                assert_eq!(unavailable.len(), 1);
                assert_eq!(unavailable[0].resume_token, old_token);
            }
            other => panic!("generation did not publish: {other:?}"),
        }
        assert_eq!(running.generation(), 2);
        assert_eq!(running.completed_step(), 1);
        let next_epoch = running.world_epoch();
        assert_ne!(next_epoch, old_epoch);
        assert!(matches!(
            command_reply(&mut running, command, bound),
            RunningAuthorityEvent::CommandRejected { .. }
        ));
        assert_eq!(running.world_epoch(), next_epoch);
        assert_eq!(managed.file_count(), 2);
    }

    #[test]
    fn generation_announcement_reserves_count_before_scheduler_or_evolution_mutates() {
        let mut running = background_generation_handoff_fixture().unwrap().running;
        let mut limits = background_generation_handoff_runtime_init().output;
        limits.max_reliable = 2;
        let output = OutputQueue::new(limits, Arc::new(NoopWakeSink));
        service_running_with_output_capacity(&mut running, &output, 0).unwrap();
        output.push_reliable(ReliableEvent::Started).unwrap();
        output
            .push_reliable(ReliableEvent::ProbeResult {
                sequence: 1,
                correlation_id: 1,
                payload: Vec::new(),
            })
            .unwrap();
        let before = RunningAuthorityMetrics::new(&running).snapshot();
        assert_eq!(
            service_running_with_output_capacity(&mut running, &output, 500)
                .unwrap_err()
                .code,
            EngineErrorCode::QueueCountLimit
        );
        assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
        assert!(running.pending_generation_transition().is_none());
        assert_eq!(output.drain(usize::MAX, usize::MAX).events.len(), 2);
        assert!(matches!(
            service_running_with_output_capacity(&mut running, &output, 500).unwrap(),
            RunningAuthorityLoopProgress::GenerationTransitionPending {
                successor_generation: 2,
                ..
            }
        ));
    }

    #[test]
    fn running_runtime_rejects_undersized_scalar_event_limit_without_consuming_authority() {
        let running = background_generation_handoff_fixture().unwrap().running;
        let before = RunningAuthorityMetrics::new(&running).snapshot();
        let mut init = background_generation_handoff_runtime_init();
        init.output.max_event_owned_bytes = size_of::<RunningAuthorityEvent>() - 1;
        let failure = EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink))
            .unwrap_err();
        assert_eq!(failure.error().code, EngineErrorCode::InvalidConfiguration);
        let running = failure.into_running_loop();
        assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
    }

    /// Observe an actual background wait or result without assuming host speed.
    fn wait_for_runtime(runtime: &EngineRuntime, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            std::thread::yield_now();
        }
        panic!(
            "background runtime did not progress: {:?}",
            runtime.health()
        );
    }

    /// Submit a single bounded command through normal runtime admission.
    fn submit_runtime_command(runtime: &EngineRuntime, sequence: u64, command: EngineCommand) {
        runtime
            .try_submit(CommandBatch {
                contract_version: ENGINE_CONTRACT_VERSION,
                commands: vec![SequencedCommand { sequence, command }].into_boxed_slice(),
            })
            .unwrap();
    }

    #[test]
    fn background_queue_drain_retries_retained_publication_after_count_or_byte_pressure() {
        for byte_pressure in [false, true] {
            let managed = TestDirectory::create();
            let running = background_generation_handoff_fixture().unwrap().running;
            let mut init = background_generation_handoff_runtime_init();
            init.output.max_reliable = if byte_pressure { 8 } else { 2 };
            init.output.max_event_owned_bytes = 2048;
            init.output.max_reliable_owned_bytes = 2048;
            init.inbound.max_batch_owned_bytes = 2048;
            let runtime =
                EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink))
                    .unwrap();
            runtime.start().unwrap();
            wait_for_runtime(&runtime, || runtime.health().output.reliable == 2);
            if byte_pressure {
                submit_runtime_command(
                    &runtime,
                    1,
                    EngineCommand::Probe {
                        correlation_id: 1,
                        payload: vec![0; 2048 - size_of::<RunningAuthorityEvent>()],
                    },
                );
                wait_for_runtime(&runtime, || runtime.health().processed_commands == 1);
            }
            let before = runtime.health().running_authority.unwrap();
            submit_runtime_command(
                &runtime,
                2,
                EngineCommand::RunningAuthority(managed.publish_command()),
            );
            wait_for_runtime(&runtime, || runtime.health().output.capacity_waits > 0);
            let blocked = runtime.health();
            assert!(blocked.fault.is_none());
            let authority = blocked.running_authority.unwrap();
            assert_eq!(authority.world_epoch, before.world_epoch);
            assert_eq!(authority.generation, 1);
            assert!(!authority.generation_checkpoint_published);
            assert_eq!(managed.file_count(), 0);
            runtime.drain_outputs(usize::MAX, usize::MAX).unwrap();
            wait_for_runtime(&runtime, || {
                runtime
                    .health()
                    .running_authority
                    .unwrap()
                    .generation_checkpoint_published
            });
            wait_for_runtime(&runtime, || {
                runtime.health().processed_commands == if byte_pressure { 2 } else { 1 }
            });
            let events = runtime
                .drain_outputs(usize::MAX, usize::MAX)
                .unwrap()
                .events;
            assert_eq!(events.len(), 1);
            assert!(
                matches!(&events[0], CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event))
                if matches!(**event, RunningAuthorityEvent::GenerationCheckpointPublished { command_sequence: 2, .. }))
            );
            assert_eq!(managed.file_count(), 2);
            assert_eq!(
                runtime.health().running_authority.unwrap().world_epoch,
                before.world_epoch
            );
            runtime.join().unwrap();
            assert!(runtime.health().fault.is_none());
        }
    }

    #[test]
    fn stopping_a_full_background_output_queue_unblocks_join_without_publication() {
        let managed = TestDirectory::create();
        let running = background_generation_handoff_fixture().unwrap().running;
        let mut init = background_generation_handoff_runtime_init();
        init.output.max_reliable = 2;
        let runtime =
            EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink)).unwrap();
        runtime.start().unwrap();
        wait_for_runtime(&runtime, || runtime.health().output.reliable == 2);
        submit_runtime_command(
            &runtime,
            1,
            EngineCommand::RunningAuthority(managed.publish_command()),
        );
        wait_for_runtime(&runtime, || runtime.health().output.capacity_waits > 0);
        runtime.join().unwrap();
        assert!(runtime.health().fault.is_some());
        assert_eq!(managed.file_count(), 0);
        assert!(
            !runtime
                .health()
                .running_authority
                .unwrap()
                .generation_checkpoint_published
        );
        assert!(runtime.running_authority_retained_for_test());
    }
}
