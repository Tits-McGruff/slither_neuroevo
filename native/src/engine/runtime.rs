//! Pure-Rust runtime owning one background coordinator thread.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{Builder, JoinHandle};
use std::{error::Error, fmt::Display};

#[cfg(test)]
use super::contract::CompletedEvent;
use super::contract::{CommandBatch, EngineFault, EngineInit};
pub use super::coordinator::RunningAuthorityHealth;
use super::coordinator::{
    fault_and_stop, panic_error, run_coordinator, run_running_coordinator, CoordinatorState,
    LifecycleState, RunningAuthorityMetrics,
};
use super::error::{EngineError, EngineErrorCode};
use super::queues::{
    DrainResult, InboundMetrics, InboundQueue, OutputMetrics, OutputQueue, WakeMetrics, WakeSink,
};
use super::running_loop::RunningAuthorityLoop;
use super::state::AuthoritativeState;

/// Small health snapshot that never copies authoritative world state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineHealth {
    /// Current one-shot lifecycle.
    pub lifecycle: LifecycleState,
    /// Inbound queue counters.
    pub inbound: InboundMetrics,
    /// Output queue counters.
    pub output: OutputMetrics,
    /// Coalesced wake counters.
    pub wake: WakeMetrics,
    /// Fully processed command batches.
    pub processed_batches: u64,
    /// Fully processed commands.
    pub processed_commands: u64,
    /// Bounded autonomous-authority state when the real loop mode is selected.
    pub running_authority: Option<RunningAuthorityHealth>,
    /// First retained fault, if any.
    pub fault: Option<EngineFault>,
}

/// Non-throwaway, N-API-independent owner of the Stage 3 coordinator spine.
pub struct EngineRuntime {
    init: EngineInit,
    mode: RuntimeMode,
    inbound: Arc<InboundQueue>,
    output: Arc<OutputQueue>,
    coordinator: Arc<CoordinatorState>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

/// Recoverable failure before a retained loop enters the background runtime.
#[derive(Debug)]
pub struct RunningAuthorityRuntimeCreationError {
    running: Box<RunningAuthorityLoop>,
    error: EngineError,
}

impl RunningAuthorityRuntimeCreationError {
    fn new(running: RunningAuthorityLoop, error: EngineError) -> Self {
        Self {
            running: Box::new(running),
            error,
        }
    }

    /// Inspect the bounded runtime configuration or handoff failure.
    #[must_use]
    pub const fn error(&self) -> &EngineError {
        &self.error
    }

    /// Recover the exact retained loop without reconstructing authority.
    #[must_use]
    pub fn into_running_loop(self) -> RunningAuthorityLoop {
        *self.running
    }

    /// Recover both the exact retained loop and the bounded failure.
    #[must_use]
    pub fn into_parts(self) -> (RunningAuthorityLoop, EngineError) {
        (*self.running, self.error)
    }
}

impl Display for RunningAuthorityRuntimeCreationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "running-authority runtime creation failed: {}",
            self.error
        )
    }
}

impl Error for RunningAuthorityRuntimeCreationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// Explicit separation between the temporary probe bridge and real Rust authority.
enum RuntimeMode {
    /// Own one validated state skeleton before the coordinator can start.
    Authoritative(Arc<Mutex<AuthoritativeState>>),
    /// Transfer one unserviced retained loop into the coordinator thread.
    RunningAuthority {
        loop_slot: Arc<Mutex<Option<RunningAuthorityLoop>>>,
        metrics: Arc<RunningAuthorityMetrics>,
        memory_bytes: usize,
    },
    /// Exercise only the coarse bridge; no game state exists in this mode.
    ExperimentalProbe,
}

impl RuntimeMode {
    /// Return a stable diagnostic name without exposing authoritative state.
    fn name(&self) -> &'static str {
        match self {
            Self::Authoritative(_) => "authoritative",
            Self::RunningAuthority { .. } => "running-authoritative",
            Self::ExperimentalProbe => "experimental-probe",
        }
    }
}

impl std::fmt::Debug for EngineRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EngineRuntime")
            .field("init", &self.init)
            .field("mode", &self.mode.name())
            .field("health", &self.health())
            .finish_non_exhaustive()
    }
}

impl EngineRuntime {
    /// Validate configuration and create an unstarted runtime that already owns authority.
    pub fn new_authoritative(
        init: EngineInit,
        state: AuthoritativeState,
        wake_sink: Arc<dyn WakeSink>,
    ) -> Result<Self, EngineError> {
        init.validate()?;
        Ok(Self::new_with_validated_mode(
            init,
            RuntimeMode::Authoritative(Arc::new(Mutex::new(state))),
            wake_sink,
        ))
    }

    /// Validate and retain one unserviced loop for background thread ownership.
    ///
    /// Every failure returns the exact loop to its prior owner. The loop must
    /// have been prepared at wall origin zero and never serviced; the actual
    /// coordinator thread starts its Rust monotonic elapsed clock from zero.
    pub fn new_running_authority(
        init: EngineInit,
        running: RunningAuthorityLoop,
        wake_sink: Arc<dyn WakeSink>,
    ) -> Result<Self, RunningAuthorityRuntimeCreationError> {
        if let Err(error) = init.validate() {
            return Err(RunningAuthorityRuntimeCreationError::new(running, error));
        }
        if let Err(error) = running.validate_background_start() {
            return Err(RunningAuthorityRuntimeCreationError::new(
                running,
                EngineError::new(
                    EngineErrorCode::InvalidLifecycle,
                    format!("invalid retained-loop handoff: {error}"),
                ),
            ));
        }
        let memory_bytes = running.authoritative_memory_bytes();
        let metrics = Arc::new(RunningAuthorityMetrics::new(&running));
        Ok(Self::new_with_validated_mode(
            init,
            RuntimeMode::RunningAuthority {
                loop_slot: Arc::new(Mutex::new(Some(running))),
                metrics,
                memory_bytes,
            },
            wake_sink,
        ))
    }

    /// Create the explicitly non-authoritative runtime used only by the probe bridge.
    pub(crate) fn new_experimental_probe(
        init: EngineInit,
        wake_sink: Arc<dyn WakeSink>,
    ) -> Result<Self, EngineError> {
        init.validate()?;
        Ok(Self::new_with_validated_mode(
            init,
            RuntimeMode::ExperimentalProbe,
            wake_sink,
        ))
    }

    /// Construct one explicit runtime mode after its shared configuration passed validation.
    fn new_with_validated_mode(
        init: EngineInit,
        mode: RuntimeMode,
        wake_sink: Arc<dyn WakeSink>,
    ) -> Self {
        Self {
            init,
            mode,
            inbound: Arc::new(InboundQueue::new(init.inbound)),
            output: Arc::new(OutputQueue::new(init.output, wake_sink)),
            coordinator: Arc::new(CoordinatorState::new()),
            thread: Mutex::new(None),
        }
    }

    /// Report whether this runtime owns validated or running authority.
    #[must_use]
    pub fn owns_authoritative_state(&self) -> bool {
        matches!(
            &self.mode,
            RuntimeMode::Authoritative(_) | RuntimeMode::RunningAuthority { .. }
        )
    }

    /// Report admitted authoritative-state bytes without exposing or copying game state.
    #[must_use]
    pub fn authoritative_state_memory_bytes(&self) -> Option<usize> {
        match &self.mode {
            RuntimeMode::Authoritative(state) => {
                Some(lock_recover(state).memory_estimate().total_bytes)
            }
            RuntimeMode::RunningAuthority { memory_bytes, .. } => Some(*memory_bytes),
            RuntimeMode::ExperimentalProbe => None,
        }
    }

    /// Borrow the retained state only for internal Rust tests; it is never an N-API surface.
    #[cfg(test)]
    pub(crate) fn authoritative_state_for_test(
        &self,
    ) -> Option<MutexGuard<'_, AuthoritativeState>> {
        match &self.mode {
            RuntimeMode::Authoritative(state) => Some(lock_recover(state)),
            RuntimeMode::RunningAuthority { .. } | RuntimeMode::ExperimentalProbe => None,
        }
    }

    /// Report whether the background thread restored its exact loop after exit.
    #[cfg(test)]
    pub(crate) fn running_authority_retained_for_test(&self) -> bool {
        match &self.mode {
            RuntimeMode::RunningAuthority { loop_slot, .. } => lock_recover(loop_slot).is_some(),
            RuntimeMode::Authoritative(_) | RuntimeMode::ExperimentalProbe => false,
        }
    }

    /// Start the coordinator exactly once.
    pub fn start(&self) -> Result<(), EngineError> {
        // The handle lock serializes the Created -> Running -> published-handle
        // interval against every joiner.
        let mut slot = lock_recover(&self.thread);
        self.coordinator.transition(|lifecycle| match lifecycle {
            LifecycleState::Created => Ok(LifecycleState::Running),
            other => Err(EngineError::new(
                EngineErrorCode::InvalidLifecycle,
                format!("engine cannot start from {other:?}"),
            )),
        })?;

        let inbound = self.inbound.clone();
        let output = self.output.clone();
        let coordinator = self.coordinator.clone();
        let running_work = match &self.mode {
            RuntimeMode::RunningAuthority {
                loop_slot, metrics, ..
            } => Some((Arc::clone(loop_slot), Arc::clone(metrics))),
            RuntimeMode::Authoritative(_) | RuntimeMode::ExperimentalProbe => None,
        };
        let spawn = Builder::new()
            .name("slither-engine-coordinator".to_owned())
            .spawn(move || {
                if let Some((loop_slot, metrics)) = running_work {
                    run_running_thread_root(&inbound, &output, &coordinator, &loop_slot, &metrics);
                } else {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        run_coordinator(&inbound, &output, &coordinator);
                    }));
                    match result {
                        Ok(()) => coordinator.mark_normal_stopped(),
                        Err(payload) => {
                            let error = panic_error(payload.as_ref());
                            fault_and_stop(&inbound, &output, &coordinator, error);
                        }
                    }
                }
            });

        match spawn {
            Ok(handle) => {
                *slot = Some(handle);
                Ok(())
            }
            Err(error) => {
                let failure = EngineError::new(
                    EngineErrorCode::ThreadSpawn,
                    format!("failed to spawn engine coordinator: {error}"),
                );
                fault_and_stop(
                    &self.inbound,
                    &self.output,
                    &self.coordinator,
                    failure.clone(),
                );
                Err(failure)
            }
        }
    }

    /// Enqueue one validated batch atomically and without waiting for capacity.
    pub fn try_submit(&self, batch: CommandBatch) -> Result<(), EngineError> {
        batch.validate_output_shape(&self.init.output)?;
        match self.coordinator.lifecycle() {
            LifecycleState::Running => self.inbound.try_push(batch),
            LifecycleState::Faulted => Err(EngineError::new(
                EngineErrorCode::Faulted,
                "engine is faulted",
            )),
            lifecycle => Err(EngineError::new(
                EngineErrorCode::InvalidLifecycle,
                format!("engine cannot accept commands while {lifecycle:?}"),
            )),
        }
    }

    /// Fault the experimental engine because its external bridge can no longer
    /// deliver or safely translate events. The first fault remains authoritative.
    pub fn report_bridge_fault(&self, error: EngineError) {
        fault_and_stop(&self.inbound, &self.output, &self.coordinator, error);
    }

    /// Request an orderly stop. Repeated calls are harmless and nonblocking.
    pub fn request_stop(&self) {
        let transition = self.coordinator.transition(|lifecycle| match lifecycle {
            LifecycleState::Created => Ok(LifecycleState::Stopped),
            LifecycleState::Running => Ok(LifecycleState::StopRequested),
            LifecycleState::StopRequested | LifecycleState::Faulted | LifecycleState::Stopped => {
                Ok(lifecycle)
            }
        });
        if transition.is_ok() {
            self.inbound.request_stop();
        }
    }

    /// Request stop and join the coordinator. A future N-API adapter must call
    /// this from `AsyncTask`, never from Node's main event loop.
    pub fn join(&self) -> Result<(), EngineError> {
        self.request_stop();
        // Hold the handle lock through the OS join. A second joiner therefore
        // cannot observe an empty slot and report completion prematurely.
        let mut slot = lock_recover(&self.thread);
        let handle = slot.take();
        let Some(handle) = handle else {
            return Ok(());
        };
        match handle.join() {
            Ok(()) => {
                self.coordinator.mark_joined();
                Ok(())
            }
            Err(payload) => {
                let failure = EngineError::new(
                    EngineErrorCode::ThreadJoin,
                    panic_error(payload.as_ref()).detail,
                );
                fault_and_stop(
                    &self.inbound,
                    &self.output,
                    &self.coordinator,
                    failure.clone(),
                );
                self.coordinator.mark_joined();
                Err(failure)
            }
        }
    }

    /// Drain prepared output in priority order using caller-supplied positive bounds.
    pub fn drain_outputs(
        &self,
        max_events: usize,
        max_owned_bytes: usize,
    ) -> Result<DrainResult, EngineError> {
        if max_events == 0 || max_owned_bytes == 0 {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "output drain limits must be positive",
            ));
        }
        let minimum_bytes = self
            .init
            .output
            .max_event_owned_bytes
            .max(super::error::MAX_ERROR_DETAIL_BYTES);
        if max_owned_bytes < minimum_bytes {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                format!("output drain byte limit must be at least {minimum_bytes}"),
            ));
        }
        Ok(self.output.drain(max_events, max_owned_bytes))
    }

    /// Return a small consistent-enough operational snapshot.
    pub fn health(&self) -> EngineHealth {
        let (processed_batches, processed_commands) = self.coordinator.processed();
        let running_authority = match &self.mode {
            RuntimeMode::RunningAuthority { metrics, .. } => Some(metrics.snapshot()),
            RuntimeMode::Authoritative(_) | RuntimeMode::ExperimentalProbe => None,
        };
        EngineHealth {
            lifecycle: self.coordinator.lifecycle(),
            inbound: self.inbound.metrics(),
            output: self.output.metrics(),
            wake: self.output.wake_metrics(),
            processed_batches,
            processed_commands,
            running_authority,
            fault: self.coordinator.fault(),
        }
    }

    /// Drain all currently available events for internal tests and adapters.
    #[cfg(test)]
    fn drain_all_for_test(&self) -> Vec<CompletedEvent> {
        let mut events = Vec::new();
        loop {
            let drained = match self.drain_outputs(usize::MAX, usize::MAX) {
                Ok(result) => result,
                Err(_) => return events,
            };
            events.extend(drained.events);
            if !drained.more_work {
                return events;
            }
        }
    }
}

fn run_running_thread_root(
    inbound: &Arc<InboundQueue>,
    output: &Arc<OutputQueue>,
    coordinator: &Arc<CoordinatorState>,
    loop_slot: &Arc<Mutex<Option<RunningAuthorityLoop>>>,
    metrics: &Arc<RunningAuthorityMetrics>,
) {
    let Some(mut running) = lock_recover(loop_slot).take() else {
        fault_and_stop(
            inbound,
            output,
            coordinator,
            EngineError::new(
                EngineErrorCode::InvalidLifecycle,
                "running-authority thread started without its retained loop",
            ),
        );
        return;
    };

    let result = catch_unwind(AssertUnwindSafe(|| {
        run_running_coordinator(inbound, output, coordinator, &mut running, metrics)
    }));
    metrics.observe(&running);
    let prior = lock_recover(loop_slot).replace(running);
    if prior.is_some() {
        fault_and_stop(
            inbound,
            output,
            coordinator,
            EngineError::new(
                EngineErrorCode::Faulted,
                "running-authority loop slot was unexpectedly occupied at thread exit",
            ),
        );
        return;
    }

    match result {
        Ok(Ok(())) => coordinator.mark_normal_stopped(),
        Ok(Err(error)) => fault_and_stop(inbound, output, coordinator, error),
        Err(payload) => {
            let error = panic_error(payload.as_ref());
            fault_and_stop(inbound, output, coordinator, error);
        }
    }
}

impl Drop for EngineRuntime {
    fn drop(&mut self) {
        // This signals through the queue's constant-time condition-variable handshake;
        // it never waits for coordinator completion or joins the thread. Dropping the
        // JoinHandle detaches, and its Arc-owned state remains valid until it exits.
        self.inbound.request_stop();
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::contract::{
        EngineCommand, InboundLimits, OutputLimits, SequencedCommand, ENGINE_CONTRACT_VERSION,
    };
    use super::super::queues::NoopWakeSink;
    use super::*;
    use std::sync::mpsc;
    use std::sync::Barrier;
    use std::time::{Duration, Instant};

    fn init() -> EngineInit {
        EngineInit {
            contract_version: ENGINE_CONTRACT_VERSION,
            inbound: InboundLimits {
                max_batches: 4,
                max_commands: 8,
                max_owned_bytes: 64,
                max_batch_commands: 4,
                max_batch_owned_bytes: 32,
            },
            output: OutputLimits {
                max_reliable: 8,
                max_reliable_owned_bytes: 64,
                max_discrete: 4,
                max_discrete_owned_bytes: 64,
                max_total_owned_bytes: 128,
                max_event_owned_bytes: 64,
                max_frame_connections: 4,
            },
        }
    }

    fn probe(sequence: u64, value: u8) -> CommandBatch {
        CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence,
                command: EngineCommand::Probe {
                    correlation_id: sequence + 100,
                    payload: vec![value],
                },
            }]
            .into_boxed_slice(),
        }
    }

    fn wait_until(runtime: &EngineRuntime, predicate: impl Fn(&EngineHealth) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if predicate(&runtime.health()) {
                return;
            }
            std::thread::yield_now();
        }
        panic!("condition not reached: {:?}", runtime.health());
    }

    #[test]
    fn lifecycle_is_one_shot_and_probe_is_correlated() {
        let runtime =
            EngineRuntime::new_experimental_probe(init(), Arc::new(NoopWakeSink)).assert_ok();
        assert!(!runtime.owns_authoritative_state());
        assert_eq!(runtime.authoritative_state_memory_bytes(), None);
        assert!(runtime.authoritative_state_for_test().is_none());
        assert_eq!(runtime.health().lifecycle, LifecycleState::Created);
        assert!(runtime.try_submit(probe(1, 7)).is_err());
        runtime.start().assert_ok();
        assert!(runtime.start().is_err());
        runtime.try_submit(probe(1, 7)).assert_ok();
        wait_until(&runtime, |health| health.processed_commands == 1);
        runtime.request_stop();
        runtime.request_stop();
        runtime.join().assert_ok();
        assert_eq!(runtime.health().lifecycle, LifecycleState::Stopped);
        let events = runtime.drain_all_for_test();
        assert!(events.iter().any(|event| matches!(event, CompletedEvent::Reliable(super::super::contract::ReliableEvent::ProbeResult { sequence: 1, correlation_id: 101, payload }) if payload == &[7])));
        assert!(runtime.try_submit(probe(2, 8)).is_err());
    }

    #[test]
    fn panic_becomes_fault_and_joins_cleanly() {
        let runtime =
            EngineRuntime::new_experimental_probe(init(), Arc::new(NoopWakeSink)).assert_ok();
        runtime.start().assert_ok();
        runtime
            .try_submit(CommandBatch {
                contract_version: ENGINE_CONTRACT_VERSION,
                commands: vec![SequencedCommand {
                    sequence: 1,
                    command: EngineCommand::PanicForTest,
                }]
                .into_boxed_slice(),
            })
            .assert_ok();
        wait_until(&runtime, |health| {
            health.lifecycle == LifecycleState::Faulted
        });
        assert!(runtime.health().fault.is_some());
        runtime.join().assert_ok();
        assert_eq!(runtime.health().lifecycle, LifecycleState::Stopped);
        assert!(runtime
            .drain_all_for_test()
            .iter()
            .any(|event| matches!(event, CompletedEvent::Fault(_))));
    }

    #[test]
    fn drop_only_signals_and_does_not_wait_for_join() {
        let runtime =
            EngineRuntime::new_experimental_probe(init(), Arc::new(NoopWakeSink)).assert_ok();
        runtime.start().assert_ok();
        let start = Instant::now();
        drop(runtime);
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn invalid_limits_fail_before_runtime_creation() {
        let mut invalid = init();
        invalid.inbound.max_batches = 0;
        assert_eq!(
            EngineRuntime::new_experimental_probe(invalid, Arc::new(NoopWakeSink))
                .err()
                .map(|error| error.code),
            Some(EngineErrorCode::InvalidConfiguration)
        );
    }

    struct BlockingWake {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }

    impl WakeSink for BlockingWake {
        fn notify(&self) -> Result<(), EngineError> {
            self.entered.wait();
            self.release.wait();
            Ok(())
        }
    }

    #[test]
    fn concurrent_joiners_wait_for_the_same_coordinator_completion() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let runtime = Arc::new(
            EngineRuntime::new_experimental_probe(
                init(),
                Arc::new(BlockingWake {
                    entered: Arc::clone(&entered),
                    release: Arc::clone(&release),
                }),
            )
            .assert_ok(),
        );
        runtime.start().assert_ok();
        entered.wait();

        let start_join = Arc::new(Barrier::new(3));
        let (finished_tx, finished_rx) = mpsc::channel();
        let joiners: Vec<_> = (0..2)
            .map(|_| {
                let runtime = Arc::clone(&runtime);
                let start_join = Arc::clone(&start_join);
                let finished_tx = finished_tx.clone();
                std::thread::spawn(move || {
                    start_join.wait();
                    let result = runtime.join();
                    let _ = finished_tx.send(result);
                })
            })
            .collect();
        drop(finished_tx);
        start_join.wait();
        assert!(finished_rx
            .recv_timeout(Duration::from_millis(200))
            .is_err());
        release.wait();
        for _ in 0..2 {
            finished_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("both joiners finish after coordinator release")
                .assert_ok();
        }
        for joiner in joiners {
            assert!(joiner.join().is_ok());
        }
        assert_eq!(runtime.health().lifecycle, LifecycleState::Stopped);
    }

    #[test]
    fn bridge_fault_racing_started_wake_suppresses_orderly_stop() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let runtime = EngineRuntime::new_experimental_probe(
            init(),
            Arc::new(BlockingWake {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
        )
        .assert_ok();
        runtime.start().assert_ok();
        entered.wait();
        runtime.report_bridge_fault(EngineError::new(
            EngineErrorCode::WakeDelivery,
            "fault racing blocked wake",
        ));
        release.wait();
        wait_until(&runtime, |health| {
            health.lifecycle == LifecycleState::Faulted
        });
        runtime.join().assert_ok();

        let events = runtime.drain_all_for_test();
        assert!(events
            .iter()
            .any(|event| matches!(event, CompletedEvent::Fault(_))));
        assert!(!events.iter().any(|event| matches!(
            event,
            CompletedEvent::Reliable(super::super::contract::ReliableEvent::Stopped)
        )));
    }

    #[test]
    fn first_bridge_fault_remains_consistent_after_drain() {
        let runtime =
            EngineRuntime::new_experimental_probe(init(), Arc::new(NoopWakeSink)).assert_ok();
        runtime.report_bridge_fault(EngineError::new(EngineErrorCode::Faulted, "first"));
        let first = runtime.drain_outputs(8, 1024).assert_ok();
        assert!(matches!(
            first.events.first(),
            Some(CompletedEvent::Fault(fault)) if fault.detail() == "first"
        ));
        runtime.report_bridge_fault(EngineError::new(EngineErrorCode::WakeDelivery, "second"));
        let health = runtime.health();
        assert_eq!(
            health.fault.as_ref().map(EngineFault::detail),
            Some("first")
        );
        assert!(runtime.drain_outputs(8, 1024).assert_ok().events.is_empty());
    }

    trait AssertOk<T> {
        fn assert_ok(self) -> T;
    }
    impl<T, E: std::fmt::Debug> AssertOk<T> for Result<T, E> {
        fn assert_ok(self) -> T {
            match self {
                Ok(value) => value,
                Err(error) => panic!("unexpected error: {error:?}"),
            }
        }
    }
}
