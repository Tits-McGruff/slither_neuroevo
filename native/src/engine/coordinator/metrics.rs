//! Atomic health counters; the coordinator retains update and wake ordering.

use super::*;

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
    pub(super) observation_quality: Mutex<crate::engine::observation_quality::ObservationQuality>,
    pub(super) loop_state: AtomicU8,
    pub(super) world_epoch: AtomicU64,
    pub(super) generation: AtomicU64,
    pub(super) completed_step: AtomicU64,
    pub(super) generation_checkpoint_published: AtomicBool,
    pub(super) generation_persistence_acknowledged: AtomicBool,
    pub(super) pending_external_deliveries: AtomicUsize,
    pub(super) scheduler_completed_steps: AtomicU64,
    pub(super) scheduler_dropped_wall_micros: AtomicU64,
    pub(super) scheduler_overloaded: AtomicBool,
    pub(super) command_service_boundaries: AtomicU64,
    pub(super) coordinator_work_bytes: Arc<AtomicU64>,
    pub(super) wait_calls: AtomicU64,
    pub(super) blocked_wait_calls: AtomicU64,
    pub(super) timeout_wakes: AtomicU64,
    pub(super) command_wakes: AtomicU64,
    pub(super) step_timing_samples: AtomicU64,
    pub(super) step_timing_total_micros: AtomicU64,
    pub(super) step_timing_max_micros: AtomicU64,
    pub(super) step_timing_buckets: [AtomicU64; STEP_TIMING_BUCKET_UPPER_MICROS.len()],
    pub(super) slow_step_samples: AtomicU64,
    pub(super) slow_step_control_micros: AtomicU64,
    pub(super) slow_step_control_index_micros: AtomicU64,
    pub(super) slow_step_control_neural_micros: AtomicU64,
    pub(super) slow_step_control_sensing_micros: AtomicU64,
    pub(super) slow_step_control_inference_micros: AtomicU64,
    pub(super) slow_step_world_micros: AtomicU64,
    pub(super) slow_step_other_micros: AtomicU64,
    pub(super) terminal_step_samples: AtomicU64,
    pub(super) terminal_step_total_micros: AtomicU64,
    pub(super) terminal_step_max_micros: AtomicU64,
    pub(super) terminal_step_control_micros: AtomicU64,
    pub(super) terminal_step_world_micros: AtomicU64,
    pub(super) terminal_step_preparation_micros: AtomicU64,
    pub(super) terminal_step_evolution_micros: AtomicU64,
    pub(super) terminal_step_evolution_summary_micros: AtomicU64,
    pub(super) terminal_step_evolution_reproduction_micros: AtomicU64,
    pub(super) terminal_step_admission_micros: AtomicU64,
    pub(super) terminal_step_other_micros: AtomicU64,
}

impl RunningAuthorityMetrics {
    /// Seed metrics from an unserviced retained loop before thread start.
    pub(crate) fn new(running: &RunningAuthorityLoop) -> Self {
        let diagnostics = running.scheduler_diagnostics();
        Self {
            #[cfg(feature = "engine-test-hooks")]
            observation_quality: Mutex::new(
                crate::engine::observation_quality::ObservationQuality::default(),
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
    pub(super) fn record_wait(&self, blocked: bool) {
        saturating_increment(&self.wait_calls, 1);
        if blocked {
            saturating_increment(&self.blocked_wait_calls, 1);
        }
    }

    /// Record why one condition-variable wait completed.
    pub(super) fn record_wake(&self, result: InboundWaitResult) {
        match result {
            InboundWaitResult::CommandsReady => saturating_increment(&self.command_wakes, 1),
            InboundWaitResult::TimedOut => saturating_increment(&self.timeout_wakes, 1),
            InboundWaitResult::Stopped => {}
        }
    }

    /// Retain bounded production timing without allocating or crossing authority ownership.
    pub(super) fn record_step_duration(
        &self,
        duration: Duration,
        cost: Option<RunningStepCostMicros>,
    ) {
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
    pub(super) fn record_terminal_step_duration(
        &self,
        duration: Duration,
        cost: RunningStepCostMicros,
    ) {
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
    pub(super) fn step_percentile_micros(
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

pub(super) fn loop_state_code(state: RunningAuthorityLoopState) -> u8 {
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

pub(super) fn loop_state_from_code(code: u8) -> RunningAuthorityLoopState {
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

pub(super) fn saturating_increment(counter: &AtomicU64, amount: u64) {
    // `try_update` is the newer spelling and is unavailable on Rust 1.92.
    #[allow(deprecated)]
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}
