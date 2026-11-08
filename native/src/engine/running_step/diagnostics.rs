//! Timing/allocation values and projections; the coordinator retains measurement cursors.

#[cfg(feature = "engine-test-hooks")]
use super::*;

/// Test-hook-only elapsed time for each coarse scalar fixed-step phase.
///
/// These observations are deliberately unavailable in production builds and
/// do not claim finer subsystem attribution inside the joined phase owners.
#[cfg(feature = "engine-test-hooks")]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RunningStepPhaseTimings {
    pub authority_begin_ms: f64,
    pub prefix_ms: f64,
    pub control_selection_ms: f64,
    pub control_commit_ms: f64,
    pub world_step_ms: f64,
    pub generation_guard_ms: f64,
    pub publication_ms: f64,
}

/// Allocation-free coarse costs used to explain ordinary steps above the timing gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunningStepCostMicros {
    pub control_selection: u64,
    pub control_index: u64,
    pub control_neural: u64,
    pub control_sensing: u64,
    pub control_inference: u64,
    pub world_step: u64,
    pub generation_preparation: u64,
    pub generation_evolution: u64,
    /// Weight statistics and species classification inside evolution.
    pub generation_evolution_summary: u64,
    /// Elite copying and breeding/mutation inside evolution.
    pub generation_evolution_reproduction: u64,
    pub generation_admission: u64,
}

/// Test-hook-only allocation-operation counts for coarse fixed-step phases.
#[cfg(feature = "engine-test-hooks")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunningStepPhaseAllocations {
    pub authority_begin: u64,
    pub prefix: u64,
    pub control_selection: u64,
    pub control_commit: u64,
    pub world_step: u64,
    pub generation_guard: u64,
    pub publication: u64,
}

/// Retained bridge-buffer counts and capacities for allocation evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExternalDeliveryDiagnostics {
    /// Events in the one currently pending batch.
    pub pending_events: usize,
    /// Exact accepted results still required.
    pub remaining_events: usize,
    /// Retained public event-envelope capacity.
    pub event_capacity: usize,
    /// Retained per-event local-send status capacity.
    pub acceptance_capacity: usize,
    /// Retained prevalidated send-failure disconnect capacity.
    pub disconnect_capacity: usize,
    /// Retained packed Float32 observation capacity in the control workspace.
    pub observation_capacity: usize,
    /// Retained raw pre-control observation-event capacity.
    pub observation_event_capacity: usize,
    /// Retained event-source mapping capacity.
    pub source_capacity: usize,
    /// Retained small replacement-token record capacity.
    pub token_capacity: usize,
}

#[cfg(feature = "engine-test-hooks")]
pub(super) fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

#[cfg(feature = "engine-test-hooks")]
pub(super) fn allocation_delta(snapshot: Option<fn() -> u64>, cursor: &mut Option<u64>) -> u64 {
    let Some(snapshot) = snapshot else {
        return 0;
    };
    let current = snapshot();
    let previous = cursor.replace(current).unwrap_or(current);
    current.saturating_sub(previous)
}
