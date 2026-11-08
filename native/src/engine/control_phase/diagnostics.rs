//! Control capacity/work projections without workspace or publication ownership.

use super::*;

/// Coarse control-selection costs retained without per-step allocations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlPhaseCostMicros {
    pub work: crate::engine::generation_work::StepWork,
    pub spatial_index: u64,
    pub neural_batch: u64,
    pub neural_sensing: u64,
    pub neural_inference: u64,
}

/// Work counts and retained capacities for one shared control boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlPhaseDiagnostics {
    /// Shared complete body-ray hierarchy; workers only borrow this storage.
    pub ray_index: crate::engine::sensor_rays::RayIndexCapacity,
    /// Stable observation-owner/physical-index pairs retained by the coordinator.
    pub ray_owner_capacity: usize,
    /// Canonical stable keys used for fair grant assignment.
    pub ray_key_capacity: usize,
    /// Owner-order grants retained independently of worker scheduling.
    pub ray_owner_grant_capacity: usize,
    /// Physical-array grant map for the immutable source boundary.
    pub ray_snake_grant_capacity: usize,
    /// Alive snakes assigned exactly one source.
    pub controls: usize,
    /// Internally consumed baseline observations.
    pub baseline_observations: usize,
    /// Externally delivered observations awaiting Node acceptance.
    pub external_observations: usize,
    /// Complete due neural graph evaluations.
    pub neural_evaluations: usize,
    /// Neural outputs held because cadence was not due.
    pub neural_held: usize,
    /// Explicit external-to-neural transitions in this batch.
    pub neural_takeovers: usize,
    /// Retained complete body-index diagnostics.
    pub body_index: BodyIndexDiagnostics,
    /// Retained complete pellet-index diagnostics.
    pub pellet_index: PelletIndexDiagnostics,
    /// Shared per-sample sensor scratch capacities.
    pub sensor_scratch: SensorScratchDiagnostics,
    /// Coarse neural pipeline capacities.
    pub neural: NeuralControlCapacityDiagnostics,
    /// Stable snake-order capacity.
    pub snake_order_capacity: usize,
    /// Stable brain-order capacity.
    pub brain_order_capacity: usize,
    /// Stable lease-order capacity.
    pub lease_order_capacity: usize,
    /// Chosen-control capacity.
    pub control_capacity: usize,
    /// Lease-proposal capacity.
    pub controller_transition_capacity: usize,
    /// Retained baseline-result record count.
    pub baseline_result_records: usize,
    /// Retained absent-Gaussian-spare buffers for baseline results.
    pub baseline_rng_spare_capacity: usize,
    /// External-event metadata capacity.
    pub external_event_capacity: usize,
    /// Packed external-observation Float32 capacity.
    pub external_observation_capacity: usize,
    /// Neural-candidate capacity.
    pub neural_candidate_capacity: usize,
    /// Takeover reset-handle capacity.
    pub reset_brain_capacity: usize,
}

/// Retained buffers and source work represented by one applied control boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlCommitDiagnostics {
    /// Selection/sensing/inference work that produced the applied boundary.
    pub selection: ControlPhaseDiagnostics,
    /// Retained working snake capacity.
    pub snake_capacity: usize,
    /// Retained working body-point capacity.
    pub body_point_capacity: usize,
    /// Retained working pellet capacity.
    pub pellet_capacity: usize,
    /// Retained controller-lease capacity.
    pub controller_lease_capacity: usize,
    /// Retained brain-record capacity.
    pub brain_capacity: usize,
    /// Immutable non-population weight values copied on this attempt.
    pub brain_weight_values_copied: usize,
    /// Recurrent values copied from the source boundary before publication.
    pub brain_recurrent_values_copied: usize,
    /// Retained baseline-slot capacity.
    pub baseline_slot_capacity: usize,
    /// Retained baseline-RNG capacity.
    pub baseline_rng_capacity: usize,
    /// Retained external-event capacity.
    pub external_event_capacity: usize,
    /// Retained packed external-observation capacity.
    pub external_observation_capacity: usize,
    /// Retained serialized RNG text capacity, including absent Gaussian spares.
    pub rng_text_capacity: usize,
}

/// Borrowed single-brain visualization captured from one control boundary.
#[derive(Clone, Copy, Debug)]
pub struct FocusedVisualization<'a> {
    /// Full physical identity captured before movement, distinct from frame IDs.
    pub physical_id: u64,
    /// Browser/frame identity of the selected neural snake.
    pub frame_v1_id: u32,
    /// Completed step that becomes authoritative with this capture.
    pub completed_step: u64,
    /// Static graph-layer structure.
    pub layers: &'a [VisualizationLayerPlan],
    /// Packed finite activation values for layers that carry them.
    pub values: &'a [f32],
}
