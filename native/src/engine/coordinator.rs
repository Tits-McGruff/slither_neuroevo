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
    RunningStepCostMicros,
};
use super::scheduler::SchedulerServiceMode;
use super::work_progress::ProgressScope;
use super::world_step::ExternalDeliveryStatus;

/// Inclusive microsecond ceilings for the allocation-free production step histogram.
///
/// The final bucket is open-ended. Published p95/p99 values are therefore
/// conservative bucket upper bounds, while maximum and mean use exact sampled
/// microseconds. The denser sub-frame buckets preserve useful P0/P1 resolution
/// without retaining one record per authoritative step.
pub(crate) const STEP_TIMING_BUCKET_UPPER_MICROS: [u64; 25] = [
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
    16_667,
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
    /// Real-wall scheduling debt discarded, rounded up to whole microseconds.
    pub scheduler_dropped_wall_micros: u64,
    /// Whether discarded debt has left a catch-up backlog.
    pub scheduler_overloaded: bool,
    /// Command-drain boundaries serviced by the scheduler.
    pub command_service_boundaries: u64,
    /// Completed evolution/codec work bytes while the coordinator owns a long barrier.
    pub coordinator_work_bytes: u64,
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
    /// Fixed inclusive bucket ceilings; the final u64::MAX bucket is open-ended.
    pub step_timing_bucket_upper_micros: [u64; STEP_TIMING_BUCKET_UPPER_MICROS.len()],
    /// Exact lifetime counts, suitable for subtraction only when the snapshot is consistent.
    pub step_timing_bucket_counts: [u64; STEP_TIMING_BUCKET_UPPER_MICROS.len()],
    /// Counts represent the published sample prefix without an overlapping bucket update.
    pub step_timing_histogram_consistent: bool,
    /// Ordinary steps above the 16.667 ms gate with phase attribution.
    pub slow_step_samples: u64,
    /// Sum of control-selection time within attributed slow steps.
    pub slow_step_control_micros: u64,
    /// Spatial-index rebuild time within attributed slow control steps.
    pub slow_step_control_index_micros: u64,
    /// Neural batch time within attributed slow control steps.
    pub slow_step_control_neural_micros: u64,
    /// Sensor sampling time within attributed slow neural batches.
    pub slow_step_control_sensing_micros: u64,
    /// Graph inference time within attributed slow neural batches.
    pub slow_step_control_inference_micros: u64,
    /// Sum of world-step time within attributed slow steps.
    pub slow_step_world_micros: u64,
    /// Sum of remaining service time within attributed slow steps.
    pub slow_step_other_micros: u64,
    /// Successful terminal steps that staged a complete generation boundary.
    pub terminal_step_samples: u64,
    /// Terminal computation time before the Node persistence barrier begins.
    pub terminal_step_total_micros: u64,
    /// Largest successful terminal computation, independent of ordinary-step percentiles.
    pub terminal_step_max_micros: u64,
    /// Control selection within terminal computations.
    pub terminal_step_control_micros: u64,
    /// Physics within terminal computations.
    pub terminal_step_world_micros: u64,
    /// Successor preparation, including serial evolution and candidate construction.
    pub terminal_step_preparation_micros: u64,
    /// Serial evolution within successor preparation; do not add it to preparation again.
    pub terminal_step_evolution_micros: u64,
    /// Statistics/species scan within evolution, disjoint from reproduction.
    pub terminal_step_evolution_summary_micros: u64,
    /// Elite copying and breeding/mutation within evolution.
    pub terminal_step_evolution_reproduction_micros: u64,
    /// Full successor admission and validation before announcing the transition.
    pub terminal_step_admission_micros: u64,
    /// Unattributed terminal computation after bounding all disjoint coarse costs.
    pub terminal_step_other_micros: u64,
}

/// Atomics updated only by the authority thread and read by health callers.
#[derive(Debug)]
pub(crate) struct RunningAuthorityMetrics {
    /// One coherent fixed-size quality prefix, isolated from production execution.
    #[cfg(feature = "engine-test-hooks")]
    observation_quality: Mutex<super::observation_quality::ObservationQuality>,
    loop_state: AtomicU8,
    world_epoch: AtomicU64,
    generation: AtomicU64,
    completed_step: AtomicU64,
    generation_checkpoint_published: AtomicBool,
    generation_persistence_acknowledged: AtomicBool,
    pending_external_deliveries: AtomicUsize,
    scheduler_completed_steps: AtomicU64,
    scheduler_dropped_wall_micros: AtomicU64,
    scheduler_overloaded: AtomicBool,
    command_service_boundaries: AtomicU64,
    coordinator_work_bytes: Arc<AtomicU64>,
    wait_calls: AtomicU64,
    blocked_wait_calls: AtomicU64,
    timeout_wakes: AtomicU64,
    command_wakes: AtomicU64,
    step_timing_samples: AtomicU64,
    step_timing_total_micros: AtomicU64,
    step_timing_max_micros: AtomicU64,
    step_timing_buckets: [AtomicU64; STEP_TIMING_BUCKET_UPPER_MICROS.len()],
    slow_step_samples: AtomicU64,
    slow_step_control_micros: AtomicU64,
    slow_step_control_index_micros: AtomicU64,
    slow_step_control_neural_micros: AtomicU64,
    slow_step_control_sensing_micros: AtomicU64,
    slow_step_control_inference_micros: AtomicU64,
    slow_step_world_micros: AtomicU64,
    slow_step_other_micros: AtomicU64,
    terminal_step_samples: AtomicU64,
    terminal_step_total_micros: AtomicU64,
    terminal_step_max_micros: AtomicU64,
    terminal_step_control_micros: AtomicU64,
    terminal_step_world_micros: AtomicU64,
    terminal_step_preparation_micros: AtomicU64,
    terminal_step_evolution_micros: AtomicU64,
    terminal_step_evolution_summary_micros: AtomicU64,
    terminal_step_evolution_reproduction_micros: AtomicU64,
    terminal_step_admission_micros: AtomicU64,
    terminal_step_other_micros: AtomicU64,
}

impl RunningAuthorityMetrics {
    /// Seed metrics from an unserviced retained loop before thread start.
    pub(crate) fn new(running: &RunningAuthorityLoop) -> Self {
        let diagnostics = running.scheduler_diagnostics();
        Self {
            #[cfg(feature = "engine-test-hooks")]
            observation_quality: Mutex::new(
                super::observation_quality::ObservationQuality::default(),
            ),
            loop_state: AtomicU8::new(loop_state_code(running.state())),
            world_epoch: AtomicU64::new(running.world_epoch()),
            generation: AtomicU64::new(running.generation()),
            completed_step: AtomicU64::new(running.completed_step()),
            generation_checkpoint_published: AtomicBool::new(false),
            generation_persistence_acknowledged: AtomicBool::new(false),
            pending_external_deliveries: AtomicUsize::new(0),
            scheduler_completed_steps: AtomicU64::new(diagnostics.completed_steps),
            scheduler_dropped_wall_micros: AtomicU64::new(
                (diagnostics.dropped_wall_seconds * 1_000_000.0).ceil() as u64,
            ),
            scheduler_overloaded: AtomicBool::new(diagnostics.overloaded),
            command_service_boundaries: AtomicU64::new(diagnostics.command_service_boundaries),
            coordinator_work_bytes: Arc::new(AtomicU64::new(0)),
            wait_calls: AtomicU64::new(0),
            blocked_wait_calls: AtomicU64::new(0),
            timeout_wakes: AtomicU64::new(0),
            command_wakes: AtomicU64::new(0),
            step_timing_samples: AtomicU64::new(0),
            step_timing_total_micros: AtomicU64::new(0),
            step_timing_max_micros: AtomicU64::new(0),
            step_timing_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            slow_step_samples: AtomicU64::new(0),
            slow_step_control_micros: AtomicU64::new(0),
            slow_step_control_index_micros: AtomicU64::new(0),
            slow_step_control_neural_micros: AtomicU64::new(0),
            slow_step_control_sensing_micros: AtomicU64::new(0),
            slow_step_control_inference_micros: AtomicU64::new(0),
            slow_step_world_micros: AtomicU64::new(0),
            slow_step_other_micros: AtomicU64::new(0),
            terminal_step_samples: AtomicU64::new(0),
            terminal_step_total_micros: AtomicU64::new(0),
            terminal_step_max_micros: AtomicU64::new(0),
            terminal_step_control_micros: AtomicU64::new(0),
            terminal_step_world_micros: AtomicU64::new(0),
            terminal_step_preparation_micros: AtomicU64::new(0),
            terminal_step_evolution_micros: AtomicU64::new(0),
            terminal_step_evolution_summary_micros: AtomicU64::new(0),
            terminal_step_evolution_reproduction_micros: AtomicU64::new(0),
            terminal_step_admission_micros: AtomicU64::new(0),
            terminal_step_other_micros: AtomicU64::new(0),
        }
    }

    /// Refresh the published bounded authority/scheduler snapshot.
    pub(crate) fn observe(&self, running: &RunningAuthorityLoop) {
        #[cfg(feature = "engine-test-hooks")]
        if let Some((run_id, key)) = running.qualification_observation_scope() {
            let mut quality = lock_recover(&self.observation_quality);
            if quality.needs_record(run_id, key) {
                if let Some(counts) = running.qualification_observation_counts() {
                    quality.record(
                        run_id,
                        key,
                        running.world_epoch(),
                        running.generation(),
                        running.completed_step(),
                        counts,
                    );
                }
            }
        }
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
        self.scheduler_dropped_wall_micros.store(
            (diagnostics.dropped_wall_seconds * 1_000_000.0).ceil() as u64,
            Ordering::Release,
        );
        self.scheduler_overloaded
            .store(diagnostics.overloaded, Ordering::Release);
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
    fn record_step_duration(&self, duration: Duration, cost: Option<RunningStepCostMicros>) {
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        saturating_increment(&self.step_timing_total_micros, micros);
        // Keep the pre-rename API for the supported Rust 1.92 toolchain.
        #[allow(deprecated)]
        let _ = self.step_timing_max_micros.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |previous| Some(previous.max(micros)),
        );
        let bucket = STEP_TIMING_BUCKET_UPPER_MICROS
            .partition_point(|upper| *upper < micros)
            .min(STEP_TIMING_BUCKET_UPPER_MICROS.len() - 1);
        saturating_increment(&self.step_timing_buckets[bucket], 1);
        if let Some(cost) = cost.filter(|_| micros > 16_667) {
            let control = cost.control_selection.min(micros);
            let index = cost.control_index.min(control);
            let neural = cost.control_neural.min(control - index);
            let sensing = cost.control_sensing.min(neural);
            let inference = cost.control_inference.min(neural - sensing);
            let world = cost.world_step.min(micros - control);
            saturating_increment(&self.slow_step_samples, 1);
            saturating_increment(&self.slow_step_control_micros, control);
            saturating_increment(&self.slow_step_control_index_micros, index);
            saturating_increment(&self.slow_step_control_neural_micros, neural);
            saturating_increment(&self.slow_step_control_sensing_micros, sensing);
            saturating_increment(&self.slow_step_control_inference_micros, inference);
            saturating_increment(&self.slow_step_world_micros, world);
            saturating_increment(&self.slow_step_other_micros, micros - control - world);
        }
        // Publish the sample prefix after its bucket. An acquire read observes
        // every bucket in that prefix; equal before/after counts plus an exact
        // bucket sum then exclude any partially observed successor sample.
        #[allow(deprecated)]
        let _ =
            self.step_timing_samples
                .fetch_update(Ordering::Release, Ordering::Relaxed, |value| {
                    Some(value.saturating_add(1))
                });
    }

    /// Attribute successful terminal computation separately from the later durability barrier.
    fn record_terminal_step_duration(&self, duration: Duration, cost: RunningStepCostMicros) {
        let micros = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        let control = cost.control_selection.min(micros);
        let world = cost.world_step.min(micros - control);
        let preparation = cost.generation_preparation.min(micros - control - world);
        let evolution = cost.generation_evolution.min(preparation);
        let summary = cost.generation_evolution_summary.min(evolution);
        let reproduction = cost
            .generation_evolution_reproduction
            .min(evolution - summary);
        let admission = cost
            .generation_admission
            .min(micros - control - world - preparation);
        saturating_increment(&self.terminal_step_total_micros, micros);
        saturating_increment(&self.terminal_step_control_micros, control);
        saturating_increment(&self.terminal_step_world_micros, world);
        saturating_increment(&self.terminal_step_preparation_micros, preparation);
        saturating_increment(&self.terminal_step_evolution_micros, evolution);
        saturating_increment(&self.terminal_step_evolution_summary_micros, summary);
        saturating_increment(
            &self.terminal_step_evolution_reproduction_micros,
            reproduction,
        );
        saturating_increment(&self.terminal_step_admission_micros, admission);
        saturating_increment(
            &self.terminal_step_other_micros,
            micros - control - world - preparation - admission,
        );
        #[allow(deprecated)]
        let _ = self.terminal_step_max_micros.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |previous| Some(previous.max(micros)),
        );
        saturating_increment(&self.terminal_step_samples, 1);
    }

    /// Return the conservative inclusive bucket ceiling for one percentile.
    fn step_percentile_micros(
        &self,
        samples: u64,
        buckets: &[u64; STEP_TIMING_BUCKET_UPPER_MICROS.len()],
        percentile: u64,
    ) -> u64 {
        if samples == 0 {
            return 0;
        }
        let rank = u64::try_from((u128::from(samples) * u128::from(percentile)).div_ceil(100))
            .unwrap_or(u64::MAX);
        let mut cumulative = 0u64;
        for (index, bucket) in buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(*bucket);
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

    /// Serialize a coherent ordinary quality prefix without touching the live world.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn observation_quality_for_test(&self) -> Result<String, EngineError> {
        lock_recover(&self.observation_quality)
            .to_json()
            .map_err(|error| EngineError::new(EngineErrorCode::Faulted, error.to_string()))
    }

    /// Read a bounded, allocation-free operational snapshot.
    pub(crate) fn snapshot(&self) -> RunningAuthorityHealth {
        let step_timing_samples = self.step_timing_samples.load(Ordering::Acquire);
        let step_timing_bucket_counts =
            std::array::from_fn(|index| self.step_timing_buckets[index].load(Ordering::Relaxed));
        let samples_after = self.step_timing_samples.load(Ordering::Acquire);
        let bucket_sum = step_timing_bucket_counts
            .iter()
            .try_fold(0u64, |sum, count| sum.checked_add(*count));
        let step_timing_histogram_consistent = step_timing_samples != u64::MAX
            && samples_after == step_timing_samples
            && bucket_sum == Some(step_timing_samples);
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
            scheduler_dropped_wall_micros: self
                .scheduler_dropped_wall_micros
                .load(Ordering::Acquire),
            scheduler_overloaded: self.scheduler_overloaded.load(Ordering::Acquire),
            command_service_boundaries: self.command_service_boundaries.load(Ordering::Acquire),
            coordinator_work_bytes: self.coordinator_work_bytes.load(Ordering::Acquire),
            wait_calls: self.wait_calls.load(Ordering::Relaxed),
            blocked_wait_calls: self.blocked_wait_calls.load(Ordering::Relaxed),
            timeout_wakes: self.timeout_wakes.load(Ordering::Relaxed),
            command_wakes: self.command_wakes.load(Ordering::Relaxed),
            step_timing_samples,
            step_timing_total_micros: self.step_timing_total_micros.load(Ordering::Relaxed),
            step_timing_max_micros: self.step_timing_max_micros.load(Ordering::Relaxed),
            step_timing_p95_micros: self.step_percentile_micros(
                step_timing_samples,
                &step_timing_bucket_counts,
                95,
            ),
            step_timing_p99_micros: self.step_percentile_micros(
                step_timing_samples,
                &step_timing_bucket_counts,
                99,
            ),
            step_timing_bucket_upper_micros: STEP_TIMING_BUCKET_UPPER_MICROS,
            step_timing_bucket_counts,
            step_timing_histogram_consistent,
            slow_step_samples: self.slow_step_samples.load(Ordering::Relaxed),
            slow_step_control_micros: self.slow_step_control_micros.load(Ordering::Relaxed),
            slow_step_control_index_micros: self
                .slow_step_control_index_micros
                .load(Ordering::Relaxed),
            slow_step_control_neural_micros: self
                .slow_step_control_neural_micros
                .load(Ordering::Relaxed),
            slow_step_control_sensing_micros: self
                .slow_step_control_sensing_micros
                .load(Ordering::Relaxed),
            slow_step_control_inference_micros: self
                .slow_step_control_inference_micros
                .load(Ordering::Relaxed),
            slow_step_world_micros: self.slow_step_world_micros.load(Ordering::Relaxed),
            slow_step_other_micros: self.slow_step_other_micros.load(Ordering::Relaxed),
            terminal_step_samples: self.terminal_step_samples.load(Ordering::Relaxed),
            terminal_step_total_micros: self.terminal_step_total_micros.load(Ordering::Relaxed),
            terminal_step_max_micros: self.terminal_step_max_micros.load(Ordering::Relaxed),
            terminal_step_control_micros: self.terminal_step_control_micros.load(Ordering::Relaxed),
            terminal_step_world_micros: self.terminal_step_world_micros.load(Ordering::Relaxed),
            terminal_step_preparation_micros: self
                .terminal_step_preparation_micros
                .load(Ordering::Relaxed),
            terminal_step_evolution_micros: self
                .terminal_step_evolution_micros
                .load(Ordering::Relaxed),
            terminal_step_evolution_summary_micros: self
                .terminal_step_evolution_summary_micros
                .load(Ordering::Relaxed),
            terminal_step_evolution_reproduction_micros: self
                .terminal_step_evolution_reproduction_micros
                .load(Ordering::Relaxed),
            terminal_step_admission_micros: self
                .terminal_step_admission_micros
                .load(Ordering::Relaxed),
            terminal_step_other_micros: self.terminal_step_other_micros.load(Ordering::Relaxed),
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
    teachers: &super::teacher_mailbox::TeacherMailbox,
) -> Result<(), EngineError> {
    running.validate_background_start().map_err(|error| {
        EngineError::new(
            EngineErrorCode::InvalidLifecycle,
            format!("invalid background authority handoff: {error}"),
        )
    })?;
    // Evolution advances this counter per complete genome and the checkpoint
    // codec advances it block by block. Health can sample both long phases
    // without waiting for the coordinator to finish the retained barrier.
    let _progress_scope = ProgressScope::enter(Arc::clone(&metrics.coordinator_work_bytes));
    output.push_reliable(ReliableEvent::Started)?;

    let wall_origin = Instant::now();
    running.set_background_clock(wall_origin);
    let mut wait = RunningWait::Immediate;
    let mut batches = Vec::new();
    let mut announced_generation_source = None;
    let mut announced_delivery_ticket = None;
    loop {
        teachers.service(running);
        if let Some(display) = display {
            // The preceding command/service reservation has left scope. This
            // sample cannot precede its reliable result or describe staged work.
            if running.visualization_enabled() {
                running.publish_visualization(display)?;
            } else {
                display.clear_visualization()?;
            }
            let now_ms = monotonic_elapsed_ms(wall_origin)?;
            display.publish_inspector(running, now_ms)?;
            display.publish_if_due(running, now_ms, output)?;
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
        let mut service_reservation = if running.state() == RunningAuthorityLoopState::Ready {
            Some(output.reserve_authority_reply(service_reply_bytes)?)
        } else {
            None
        };
        let service_started = Instant::now();
        let mut progress = match running.service_after_command_drain(
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
        let mut controller_output = None;
        if let RunningAuthorityLoopProgress::ExternalDeliveryPending {
            ticket_sequence, ..
        } = progress
        {
            if announced_delivery_ticket != Some(ticket_sequence) {
                let reservation = service_reservation.take().ok_or_else(|| {
                    EngineError::new(
                        EngineErrorCode::Faulted,
                        "controller output lost its admitted reply",
                    )
                })?;
                let (admitted, event) = prepare_controller_output(
                    running,
                    service_reply_bytes,
                    progress,
                    ticket_sequence,
                )?;
                progress = admitted;
                controller_output = Some((reservation, event));
                announced_delivery_ticket = Some(ticket_sequence);
            }
        }
        if was_ready
            && matches!(
                progress,
                RunningAuthorityLoopProgress::Published { .. }
                    | RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
                    | RunningAuthorityLoopProgress::GenerationTransitionPending { .. }
            )
        {
            let cost = match progress {
                RunningAuthorityLoopProgress::GenerationTransitionPending { .. } => None,
                _ => Some(running.last_step_cost()),
            };
            let duration = service_started.elapsed();
            metrics.record_step_duration(duration, cost);
            if cost.is_none() {
                metrics.record_terminal_step_duration(duration, running.last_step_cost());
            }
        }
        metrics.observe(running);
        if let Some((reservation, event)) = controller_output {
            reservation.publish(event)?;
        }
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
            RunningAuthorityLoopProgress::ExternalDeliveryPending { .. } => RunningWait::Blocked,
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

/// Own one reserved queue event before committing any observation markers.
fn prepare_controller_output(
    running: &mut RunningAuthorityLoop,
    reserved_bytes: usize,
    mut progress: RunningAuthorityLoopProgress,
    ticket_sequence: u64,
) -> Result<(RunningAuthorityLoopProgress, ReliableEvent), EngineError> {
    let messages = super::controller_output::own_pending_messages(running)?;
    let has_observations = messages
        .iter()
        .any(|message| message.event.delivery_kind == ExternalDeliveryEventKind::Observation);
    let has_assignments = messages
        .iter()
        .any(|message| message.event.delivery_kind != ExternalDeliveryEventKind::Observation);
    let event = ReliableEvent::RunningAuthority(Box::new(if has_assignments {
        RunningAuthorityEvent::ControllerMessages {
            ticket_sequence,
            messages,
        }
    } else {
        RunningAuthorityEvent::ControllerObservations {
            ticket_sequence,
            messages,
        }
    }));
    if event.owned_bytes() > reserved_bytes {
        return Err(EngineError::new(
            EngineErrorCode::QueueByteLimit,
            "controller output exceeded its pre-step reservation",
        ));
    }
    if has_observations {
        progress = running
            .admit_pending_observations(None)
            .map_err(|error| EngineError::new(EngineErrorCode::Faulted, error.to_string()))?;
    }
    Ok((progress, event))
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
        RunningAuthorityCommand::FinishGeneration { request_id, expected_run_id, expected_generation } => running
            .request_finish_generation(request_id, expected_run_id, expected_generation)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|effective_step| RunningAuthorityEvent::FinishGenerationAccepted { command_sequence, effective_step }),
        RunningAuthorityCommand::SelectEntity { target, expected_run_id, expected_generation } => running
            .select_entity(target, &expected_run_id, expected_generation)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|selection| RunningAuthorityEvent::EntitySelected { command_sequence, selection }),
        RunningAuthorityCommand::ConfigureInspector { expected_run_id, selection, pure_probe } => {
            let enabled = selection.is_some();
            running.configure_inspector_subscription(&expected_run_id, selection.map(|value| *value),
                super::inspector::MAX_INSPECTOR_BYTES, command_sequence, pure_probe)
                .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
                .map(|()| RunningAuthorityEvent::InspectorChanged { command_sequence, enabled })
        }
        RunningAuthorityCommand::GodModeMove { token, frame_v1_id, x, y } => running
            .apply_god_mode_move(*token, frame_v1_id, x, y)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(publication, effective_step)| RunningAuthorityEvent::GodModeMoved {
                command_sequence,
                frame_v1_id: publication.frame_v1_id,
                x: publication.x,
                y: publication.y,
                effective_step,
            }),
        RunningAuthorityCommand::GodModeKill { token, frame_v1_id } => running
            .apply_god_mode_kill(*token, frame_v1_id)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(publication, effective_step)| RunningAuthorityEvent::GodModeKilled {
                command_sequence,
                frame_v1_id: publication.frame_v1_id,
                pellets_dropped: publication.pellets_dropped,
                effective_step,
            }),
        RunningAuthorityCommand::SetVisualization { enabled } => {
            running.set_visualization_enabled(enabled);
            Ok(RunningAuthorityEvent::VisualizationChanged {
                command_sequence,
                enabled,
            })
        }
        RunningAuthorityCommand::ActivateShadow(request) => {
            let identity = request.candidate.artifact_sha256;
            let slot = request.shadow_slot;
            running.activate_shadow(&request, command_sequence)
                .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
                .map(|()| RunningAuthorityEvent::ShadowChanged { command_sequence, shadow_slot: slot, artifact_sha256: identity,
                    enabled: true, effective_step: running.completed_step() })
        }
        RunningAuthorityCommand::StopShadow { shadow_slot, capture_epoch, activation_sequence } => {
            let identity = running.shadow_snapshots().get(shadow_slot as usize).and_then(|snapshot| *snapshot).map_or([0;32], |snapshot| snapshot.artifact_sha256);
            running.stop_shadow(shadow_slot, capture_epoch, activation_sequence).map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
                .map(|()| RunningAuthorityEvent::ShadowChanged { command_sequence, shadow_slot, artifact_sha256: identity,
                    enabled: false, effective_step: running.completed_step() })
        }
        RunningAuthorityCommand::StartTeacherCapture(request) => running.start_teacher_capture(&request)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|contract| RunningAuthorityEvent::TeacherCaptureChanged {
                command_sequence, capture_epoch: contract.capture_epoch, enabled: true,
            }),
        RunningAuthorityCommand::StopTeacherCapture { capture_epoch } => {
            if running.teacher_capture_status().is_none_or(|status| status.0.capture_epoch != capture_epoch) {
                Err(EngineError::new(EngineErrorCode::InvalidCommand, "teacher capture epoch mismatch"))
            } else {
                running.stop_teacher_capture().map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
                    .map(|()| RunningAuthorityEvent::TeacherCaptureChanged { command_sequence, capture_epoch, enabled: false })
            }
        }
        RunningAuthorityCommand::SpawnSavedBrain(request) => running.spawn_saved_brain(&request)
            .map_err(|detail| EngineError::new(EngineErrorCode::InvalidCommand, detail))
            .map(|(publication, effective_step)| RunningAuthorityEvent::SavedBrainSpawned {
                command_sequence, frame_v1_id: publication.frame_v1_id, effective_step,
            }),
        RunningAuthorityCommand::ResurrectHallOfFame {
            entry_id,
            managed_directory,
            weights,
        } => running
            .resurrect_hall_of_fame(std::path::Path::new(&managed_directory), entry_id, &weights)
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
                    workload: publication.workload.map(Box::new),
                    command_sequence,
                    descriptor: Box::new(publication.descriptor),
                    hall_of_fame_weights: Box::new(publication.hall_of_fame_weights),
                    commit_record: Box::new(publication.commit_record),
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
        RunningAuthorityCommand::PrepareGenerationReassignments => {
            let run_id = running.controller_run_id().to_owned();
            running
            .prepare_acknowledged_generation_reassignments()
            .map_err(running_control_error)
            .and_then(|progress| {
                let (ready, assignments) = match progress {
                    GenerationReassignmentProgress::DeliveryPending(batch) => {
                        (false, own_pending_generation_assignments(batch, &run_id)?)
                    }
                    GenerationReassignmentProgress::Ready(_) => (true, Box::default()),
                };
                Ok(RunningAuthorityEvent::GenerationReassignmentsPrepared {
                    command_sequence,
                    ready,
                    assignments,
                })
            })
        },
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
                    // The published successor has replaced the old population and
                    // worker bindings. Return its freed pages at this cold boundary.
                    super::allocator_maintenance::release_unused_pages();
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
                        .checked_add(size_of::<super::generation::GenerationCommitRecord>())
                        .and_then(|bytes| {
                            let diagnostics = running.pending_generation_workload().map_or(0, |_| {
                                size_of::<super::generation_work::GenerationWork>()
                            });
                            bytes.checked_add(diagnostics)
                        })
                        .and_then(|bytes| bytes.checked_add(descriptor_bytes))
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
            let reward_count = running
                .generation_source_controller_leases()
                .iter()
                .filter(|lease| {
                    lease.status == super::state::ControllerLeaseStatus::Connected
                        && lease.connection_id.is_some()
                        && lease.reward.is_some()
                })
                .count();
            let reward_bytes = running
                .sensor_input_size()
                .checked_mul(size_of::<f32>())
                .and_then(|bytes| {
                    bytes.checked_add(crate::contracts::EXTERNAL_REWARD_MAX_ENVELOPE_BYTES)
                })
                .and_then(|bytes| bytes.checked_mul(reward_count))
                .ok_or_else(overflow)?;
            count
                .checked_mul(
                    size_of::<RunningGenerationAssignment>()
                        + super::external_replacement::RESUME_TOKEN_LENGTH,
                )
                .and_then(|bytes| bytes.checked_add(reward_bytes))
                .ok_or_else(overflow)?
        }
        RunningAuthorityCommand::SubmitGenerationAssignmentReceipts { .. }
        | RunningAuthorityCommand::SubmitControllerDeliveryReceipts { .. }
        | RunningAuthorityCommand::SubmitControllerAction(_)
        | RunningAuthorityCommand::DisconnectController(_) => 0,
        // A canonical `sha256:` identity is 71 bytes; allow allocator rounding too.
        RunningAuthorityCommand::ApplyLiveSettings { .. } => 128,
        RunningAuthorityCommand::FinishGeneration { .. } => 0,
        RunningAuthorityCommand::SelectEntity { .. } => 0,
        RunningAuthorityCommand::ConfigureInspector { .. } => 0,
        RunningAuthorityCommand::GodModeMove { .. } => 0,
        RunningAuthorityCommand::GodModeKill { .. } => 0,
        RunningAuthorityCommand::SetVisualization { .. } => 0,
        RunningAuthorityCommand::ActivateShadow(_) | RunningAuthorityCommand::StopShadow { .. } => {
            0
        }
        RunningAuthorityCommand::StartTeacherCapture(_)
        | RunningAuthorityCommand::StopTeacherCapture { .. } => 0,
        RunningAuthorityCommand::ResurrectHallOfFame { .. }
        | RunningAuthorityCommand::SpawnSavedBrain(_) => 0,
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

pub(crate) fn own_pending_generation_assignments(
    batch: ExternalObservationBatch<'_>,
    run_id: &str,
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
        let final_samples = batch.final_observation(index).unwrap_or(&[]);
        let mut final_sensors = Vec::new();
        final_sensors
            .try_reserve_exact(final_samples.len())
            .map_err(|_| {
                EngineError::new(
                    EngineErrorCode::QueueByteLimit,
                    "failed to reserve cutoff sensor reply",
                )
            })?;
        final_sensors.extend_from_slice(final_samples);
        assignments.push(RunningGenerationAssignment {
            reward_json: super::controller_output::own_reward_json(event.reward, run_id)?,
            final_sensors: final_sensors.into_boxed_slice(),
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
    // `try_update` is the newer spelling and is unavailable on Rust 1.92.
    #[allow(deprecated)]
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

#[cfg(test)]
#[path = "archive_continuation_tests.rs"]
mod archive_continuation_tests;

#[cfg(all(test, feature = "engine-test-hooks"))]
mod tests;
