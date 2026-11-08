//! Wire shapes for the coarse N-API boundary.

use super::*;

/// Test-hook-only request for one real managed checkpoint publication.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage3CheckpointFixtureOptions {
    /// Disposable server-controlled directory receiving the immutable file.
    pub managed_directory: String,
    /// Exact 32-digit lowercase hexadecimal operation identifier.
    pub operation_id: String,
    /// Exact positive transition epoch as 16 lowercase hexadecimal digits.
    pub transition_epoch: String,
}

/// Scalar-only descriptor returned by a real Rust checkpoint publisher.
#[napi(object)]
#[cfg_attr(
    feature = "engine-test-hooks",
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(
    feature = "engine-test-hooks",
    serde(rename_all = "camelCase", deny_unknown_fields)
)]
pub struct ManagedCheckpointDescriptor {
    /// Descriptor protocol version.
    pub protocol_version: u32,
    /// Exact operation identifier.
    pub operation_id: String,
    /// Exact transition epoch.
    pub transition_epoch: String,
    /// Exact run identity.
    pub run_id: String,
    /// Exact generation.
    pub generation: String,
    /// Exact completed fixed-step count.
    pub completed_step: String,
    /// Generation-boundary kind.
    pub boundary_kind: String,
    /// Managed checkpoint format version.
    pub checkpoint_format_version: String,
    /// Authoritative state contract version.
    pub state_version: String,
    /// Compiled graph-layout version.
    pub graph_layout_version: String,
    /// Fixed controlled managed-root label.
    pub managed_root: String,
    /// Digest-derived immutable filename.
    pub relative_filename: String,
    /// Encoding-independent logical root.
    pub logical_root_sha256: String,
    /// Complete stored archive bytes.
    pub stored_byte_count: String,
    /// Aggregate decoded logical bytes.
    pub decoded_byte_count: String,
    /// Logical role count.
    pub role_count: String,
    /// Dense population count.
    pub population_count: String,
    /// Packed population weight count.
    pub weight_count: String,
    /// Packed recurrent-state count.
    pub recurrent_state_count: String,
    /// Selected population-weight encoding.
    pub weights_encoding: String,
    /// Selected recurrent-state encoding.
    pub recurrent_state_encoding: String,
    /// Ordered graph-layout digest.
    pub graph_layout_sha256: String,
    /// Completed write-validation policy.
    pub write_validation_policy: String,
}

/// Scalar-only descriptor for one deduplicated Hall-of-Fame weight object.
#[napi(object)]
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
pub struct ManagedHallOfFameWeightsDescriptor {
    pub version: u32,
    pub logical_sha256: String,
    pub relative_filename: String,
    pub encoding: String,
    pub stored_byte_count: String,
    pub decoded_byte_count: String,
    pub weight_count: String,
}

/// Controlled immutable-checkpoint publication request.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct ManagedCheckpointPublicationOptions {
    /// Disposable server-controlled directory receiving the immutable file.
    pub managed_directory: String,
    /// Exact 32-digit lowercase hexadecimal operation identifier.
    pub operation_id: String,
}

/// Exact Rust-owned eight-field generation summary wire record.
#[napi(object)]
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
pub struct Stage6GenerationSummaryRecord {
    pub completed_generation: String,
    pub best_f64_hex: String,
    pub average_f64_hex: String,
    pub minimum_f64_hex: String,
    pub species_count: String,
    pub top_species_size: String,
    pub average_weight_f64_hex: String,
    pub weight_variance_f64_hex: String,
}

/// Exact Rust-owned Hall-of-Fame successor reference wire record.
#[napi(object)]
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
pub struct Stage6HallOfFameRecord {
    pub completed_generation: String,
    pub source_population_slot: String,
    pub source_snake_id: String,
    pub fitness_f64_hex: String,
    pub points_f64_hex: String,
    pub length: String,
    pub successor_population_slot: String,
    pub successor_genome_id: String,
}

/// Exact versioned terminal chronology persisted beside compact generation history.
#[napi(object)]
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
pub struct Stage6GenerationCompletionRecord {
    pub version: u32,
    pub reason: String,
    pub actual_elapsed_f64_hex: String,
    pub configured_duration_f64_hex: String,
    pub completed_step: String,
}

/// Fixed metadata extension; byte order and all numeric bits are Rust-authored.
#[napi(object)]
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
pub struct Stage6GenerationExperimentRecord {
    pub version: u32,
    pub history_hex: String,
    pub hall_of_fame_hex: String,
}

/// Complete scalar-only generation commit assembled by Rust admission.
#[napi(object)]
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
pub struct Stage6GenerationCommitRecord {
    pub experiment: Option<Stage6GenerationExperimentRecord>,
    pub completion: Option<Stage6GenerationCompletionRecord>,
    pub summary: Stage6GenerationSummaryRecord,
    pub hall_of_fame: Stage6HallOfFameRecord,
}

/// Real retained generation publication returned to the persistence client.
#[napi(object)]
pub struct Stage6GenerationCheckpointPublication {
    pub workload_json: Option<String>,
    pub descriptor: ManagedCheckpointDescriptor,
    pub hall_of_fame_weights: ManagedHallOfFameWeightsDescriptor,
    pub generation_commit: Stage6GenerationCommitRecord,
}

/// Scalar proof of the fixture's current authority and retained barrier.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationHandoffSnapshot {
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub transition_pending: bool,
    pub checkpoint_published: bool,
    pub persistence_acknowledged: bool,
    pub generation_checkpoint_publications: u32,
    pub authority_publications: u32,
}

/// One reliable fresh-snake assignment generated by the retained Rust session.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationAssignment {
    pub operation_epoch: String,
    pub event_sequence: String,
    pub connection_id: String,
    pub lease_id: String,
    pub snake_id: String,
    pub resume_token: String,
}

/// Exact local-send result for the retained generation assignment.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationAssignmentResult {
    pub operation_epoch: String,
    pub event_sequence: String,
    pub connection_id: String,
    pub lease_id: String,
    pub accepted: bool,
}

/// Scalar result of the one final old-to-new authority swap.
#[napi(object)]
pub struct Stage6GenerationStartPublication {
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub population_epoch: String,
    pub external_assignments: u32,
}

/// Exact retained fixed-step identity emitted by the background authority path.
#[napi(object)]
pub struct Stage6BackgroundStepKey {
    pub world_epoch: String,
    pub generation: String,
    pub source_completed_step: String,
    pub population_epoch: String,
    pub config_revision: String,
    pub config_sha256: String,
    pub operation_epoch: String,
}

/// One background generation-transition announcement.
#[napi(object)]
pub struct Stage6BackgroundGenerationTransition {
    pub ticket_sequence: String,
    pub source_key: Stage6BackgroundStepKey,
    pub reason: String,
    pub successor_generation: String,
    pub successor_completed_step: String,
}

/// One exact Rust-owned reassignment emitted from the retained successor.
#[napi(object)]
pub struct Stage6BackgroundGenerationAssignment {
    pub reward_json: Option<String>,
    pub final_sensors: Option<Vec<f64>>,
    pub operation_epoch: String,
    pub event_sequence: String,
    pub connection_id: String,
    pub lease_id: String,
    pub controller_kind: String,
    pub snake_id: String,
    pub frame_v1_id: String,
    pub resume_token: String,
}

/// Prepared successor reassignment batch.
#[napi(object)]
pub struct Stage6BackgroundGenerationReassignments {
    pub ready: bool,
    pub assignments: Vec<Stage6BackgroundGenerationAssignment>,
}

/// Exact accounting after applying one local assignment receipt command.
#[napi(object)]
pub struct Stage6BackgroundGenerationReceiptResolution {
    pub matched_acceptances: String,
    pub matched_failures: String,
    pub ignored_receipts: String,
    pub state: String,
    pub remaining: Option<String>,
    pub source_key: Option<Stage6BackgroundStepKey>,
    pub successor_generation: Option<String>,
    pub successor_completed_step: Option<String>,
}

/// Token-scoped unavailable controller result preserved after the old world retires.
#[napi(object)]
pub struct Stage6BackgroundUnavailableController {
    pub source_lease_id: String,
    pub source_snake_id: String,
    pub controller_kind: String,
    pub scope: String,
    pub resume_token: String,
    pub disconnected_at_ms: Option<String>,
    pub grace_expires_at_ms: Option<String>,
    pub reason: String,
}

/// Final background authority publication with no population-sized data.
#[napi(object)]
pub struct Stage6BackgroundGenerationStart {
    pub ticket_sequence: String,
    pub due_steps: String,
    pub source_key: Stage6BackgroundStepKey,
    pub publication: Stage6GenerationStartPublication,
    pub unavailable_controllers: Vec<Stage6BackgroundUnavailableController>,
}

/// Result of applying one correlated close at a fresh source boundary.
#[napi(object)]
pub struct BackgroundControllerDisconnect {
    pub lease_id: String,
    pub completed_step: String,
    pub applied: bool,
}

/// Retained token-reclaim assignment using only exact control identities.
#[napi(object)]
pub struct BackgroundControllerReclaimAssignment {
    pub request_sequence: String,
    pub controller_kind: String,
    pub connection_id: String,
    pub lease_id: String,
    pub snake_id: u32,
    pub completed_step: String,
    pub resume_token: String,
}

/// Result of one exact local-send completion for reclaim.
#[napi(object)]
pub struct BackgroundControllerReclaimResolution {
    pub request_sequence: String,
    pub matched: bool,
    pub accepted: bool,
}

/// One typed output drained from the real background runtime.
#[napi(object)]
pub struct BackgroundControllerMessage {
    pub reward_json: Option<String>,
    pub operation_epoch: String,
    pub event_sequence: String,
    pub connection_id: String,
    pub lease_id: String,
    pub controller_kind: String,
    pub internal_snake_id: String,
    pub snake_id: u32,
    pub source_completed_step: String,
    pub kind: String,
    pub sensors: Option<Vec<f64>>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub direction: Option<f64>,
    pub resume_token: Option<String>,
}

/// Exact ordinary-step transport completion, separate from generation receipts.
#[napi(object)]
pub struct BackgroundControllerReceiptResolution {
    pub matched_acceptances: String,
    pub matched_failures: String,
    pub ignored_receipts: String,
    pub remaining: String,
    pub published_completed_step: Option<String>,
}

/// Small tagged provenance copied only for one explicitly selected live entity.
#[napi(object)]
pub struct BackgroundRetainedOrigin {
    pub kind: String,
    pub weights_sha256: String,
    pub entry_id: Option<String>,
    pub brain_id: Option<String>,
    pub source_kind: Option<String>,
}

impl From<crate::engine::retained_origin::RetainedNeuralOrigin> for BackgroundRetainedOrigin {
    fn from(origin: crate::engine::retained_origin::RetainedNeuralOrigin) -> Self {
        use crate::engine::retained_origin::RetainedNeuralOrigin;
        let hex = |bytes: &[u8; 32]| bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        match origin {
            RetainedNeuralOrigin::HallOfFame {
                entry_id,
                weight_digest,
            } => Self {
                kind: "hall-of-fame".into(),
                weights_sha256: hex(&weight_digest),
                entry_id: Some(u64_hex(entry_id)),
                brain_id: None,
                source_kind: None,
            },
            RetainedNeuralOrigin::SavedBrain {
                brain_id,
                weight_digest,
                source_kind,
            } => Self {
                kind: "saved-brain".into(),
                weights_sha256: hex(&weight_digest),
                entry_id: None,
                brain_id: Some(hex(&brain_id)),
                source_kind: Some(
                    match source_kind {
                        crate::engine::brain_artifact::BrainSourceKind::Distilled => "distilled",
                        crate::engine::brain_artifact::BrainSourceKind::Imported => "imported",
                        crate::engine::brain_artifact::BrainSourceKind::Manual => "manual",
                    }
                    .into(),
                ),
            },
        }
    }
}

/// On-demand native identity and presentation scalars for one live entity.
#[napi(object)]
pub struct BackgroundEntitySelection {
    pub token: String,
    pub snake_id: u32,
    pub population_slot: Option<u32>,
    pub life_ordinal: Option<String>,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub skin: u32,
    pub retained_origin: Option<BackgroundRetainedOrigin>,
}

/// Small authoritative result of one in-bounds God Mode translation.
#[napi(object)]
pub struct BackgroundGodModeMove {
    pub snake_id: u32,
    pub x: f64,
    pub y: f64,
    pub effective_step: String,
}

/// Small authoritative result of one normal side-effect-bearing God Mode death.
#[napi(object)]
pub struct BackgroundGodModeKill {
    pub snake_id: u32,
    pub pellets_dropped: String,
    pub effective_step: String,
}

/// Small authoritative result of one retained-winner resurrection.
#[napi(object)]
pub struct BackgroundHallOfFameResurrection {
    pub snake_id: u32,
    pub effective_step: String,
}

/// Immutable candidate activation receipt at an ordered authoritative boundary.
#[napi(object)]
pub struct BackgroundShadowChange {
    pub shadow_slot: u32,
    pub artifact_sha256: String,
    pub enabled: bool,
    pub effective_step: String,
}

/// One typed output drained from the real background runtime.
#[napi(object)]
pub struct Stage6BackgroundGenerationEvent {
    pub kind: String,
    pub command_sequence: Option<String>,
    pub transition: Option<Stage6BackgroundGenerationTransition>,
    pub checkpoint: Option<Stage6GenerationCheckpointPublication>,
    pub acknowledged_operation_id: Option<String>,
    pub reassignments: Option<Stage6BackgroundGenerationReassignments>,
    pub receipt_resolution: Option<Stage6BackgroundGenerationReceiptResolution>,
    pub generation_start: Option<Stage6BackgroundGenerationStart>,
    pub import_publication: Option<Stage6RunStartPublication>,
    pub display: Option<crate::napi_running_engine::BackgroundDisplayStatus>,
    pub controller_observations: Option<Vec<BackgroundControllerMessage>>,
    pub controller_messages: Option<Vec<BackgroundControllerMessage>>,
    pub controller_receipt_resolution: Option<BackgroundControllerReceiptResolution>,
    pub controller_action_lease_id: Option<String>,
    pub controller_action_completed_step: Option<String>,
    pub settings_config_revision: Option<String>,
    pub settings_config_hash: Option<String>,
    pub settings_effective_step: Option<String>,
    pub finish_generation_effective_step: Option<String>,
    pub entity_selection: Option<BackgroundEntitySelection>,
    pub god_mode_move: Option<BackgroundGodModeMove>,
    pub god_mode_kill: Option<BackgroundGodModeKill>,
    pub hall_of_fame_resurrection: Option<BackgroundHallOfFameResurrection>,
    pub saved_brain_spawn: Option<BackgroundHallOfFameResurrection>,
    pub visualization_enabled: Option<bool>,
    pub inspector_enabled: Option<bool>,
    pub teacher_capture_epoch: Option<String>,
    pub teacher_capture_enabled: Option<bool>,
    pub shadow_change: Option<BackgroundShadowChange>,
    pub controller_disconnect: Option<BackgroundControllerDisconnect>,
    pub controller_join_assignment: Option<BackgroundControllerReclaimAssignment>,
    pub controller_join_resolution: Option<BackgroundControllerReclaimResolution>,
    pub controller_reclaim_assignment: Option<BackgroundControllerReclaimAssignment>,
    pub controller_reclaim_resolution: Option<BackgroundControllerReclaimResolution>,
    pub rejection_code: Option<String>,
    pub rejection_detail: Option<String>,
    pub fault_code: Option<String>,
    pub fault_detail: Option<String>,
}

/// Bounded output drain from the background generation fixture.
#[napi(object)]
pub struct Stage6BackgroundGenerationDrain {
    pub events: Vec<Stage6BackgroundGenerationEvent>,
    pub more_work: bool,
    pub generation: String,
}

/// Small scalar health snapshot from the background Rust authority thread.
#[napi(object)]
pub struct Stage6BackgroundGenerationHealth {
    pub calculation_workers: u32,
    pub lifecycle: String,
    pub loop_state: String,
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub generation_checkpoint_published: bool,
    pub generation_persistence_acknowledged: bool,
    pub pending_external_deliveries: String,
    pub scheduler_completed_steps: String,
    pub scheduler_dropped_wall_micros: String,
    pub scheduler_overloaded: bool,
    pub command_service_boundaries: String,
    pub coordinator_work_bytes: String,
    pub processed_commands: String,
    pub step_timing_samples: String,
    pub step_timing_total_micros: String,
    pub step_timing_max_micros: String,
    pub step_timing_p95_micros: String,
    pub step_timing_p99_micros: String,
    /// Exact fixed histogram layout, ending in the open-ended u64::MAX bucket.
    pub step_timing_bucket_upper_micros: Option<Vec<String>>,
    /// Lifetime counts for interval subtraction after checking consistency.
    pub step_timing_bucket_counts: Option<Vec<String>>,
    /// False if this bounded read overlapped an unfinished timing publication.
    pub step_timing_histogram_consistent: bool,
    pub slow_step_samples: String,
    pub slow_step_control_micros: String,
    pub slow_step_control_index_micros: String,
    pub slow_step_control_neural_micros: String,
    pub slow_step_control_sensing_micros: String,
    pub slow_step_control_inference_micros: String,
    pub slow_step_world_micros: String,
    pub slow_step_other_micros: String,
    pub terminal_step_samples: String,
    pub terminal_step_total_micros: String,
    pub terminal_step_max_micros: String,
    pub terminal_step_control_micros: String,
    pub terminal_step_world_micros: String,
    pub terminal_step_preparation_micros: String,
    pub terminal_step_evolution_micros: String,
    pub terminal_step_evolution_summary_micros: String,
    pub terminal_step_evolution_reproduction_micros: String,
    pub terminal_step_admission_micros: String,
    pub terminal_step_other_micros: String,
    pub fault_code: Option<String>,
    pub fault_detail: Option<String>,
}

/// Scalar proof of the fixture's staged or activated fresh run.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6RunStartHandoffSnapshot {
    pub transition_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub checkpoint_published: bool,
    pub persistence_acknowledged: bool,
    pub authority_published: bool,
    pub snake_count: String,
    pub pellet_count: String,
    pub checkpoint_publications: u32,
    pub authority_publications: u32,
}

/// Scalar result of one durable-boundary-to-running activation.
#[napi(object)]
pub struct Stage6RunStartPublication {
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub population_epoch: String,
}

/// One replaceable browser frame packed directly from retained Rust authority.
#[napi(object)]
pub struct ExperimentalFreshRunFrameV1 {
    /// Complete little-endian frame-v1 bytes; no population/archive bytes appear here.
    pub bytes: Uint8Array,
    /// Exact authoritative generation written into the frame.
    pub generation: String,
    /// Exact completed fixed-step count for the packed authority.
    pub completed_step: String,
    /// Exact number of authoritative snake records.
    pub total_snakes: String,
    /// Exact number of alive snake records present in the frame.
    pub alive_snakes: String,
    /// Exact number of pellet records present in the frame.
    pub pellets: String,
    /// Exact number of Float32 entries in the frame.
    pub float_length: String,
    /// Exact number of bytes in the frame.
    pub byte_length: String,
}

/// Rust-only frame payload and checked scalar metadata produced by a worker.
pub struct FreshRunFrameV1Output {
    pub(super) bytes: Vec<u8>,
    pub(super) generation: u64,
    pub(super) completed_step: u64,
    pub(super) total_snakes: u64,
    pub(super) alive_snakes: u64,
    pub(super) pellets: u64,
    pub(super) float_length: u64,
    pub(super) byte_length: u64,
}

/// Bounded scalar view of the experimental fixed-P0 fresh-run session.
#[napi(object)]
pub struct ExperimentalFreshRunSnapshot {
    /// Stable lifecycle phase; an active worker operation takes precedence.
    pub phase: String,
    /// Exact process-local transition token once construction has completed.
    pub transition_epoch: Option<String>,
    /// Exact current generation once construction has completed.
    pub generation: Option<String>,
    /// Exact completed-step count once construction has completed.
    pub completed_step: Option<String>,
    /// Whether the immutable managed file has published.
    pub checkpoint_published: Option<bool>,
    /// Whether Rust retained the worker's exact committed descriptor.
    pub persistence_acknowledged: Option<bool>,
    /// Whether the collision-safe running authority has published.
    pub authority_published: Option<bool>,
    /// Whether the one initial neutral-view frame has been packed.
    pub initial_frame_published: Option<bool>,
    /// Whether one complete Rust-scheduled step and its frame have published.
    pub first_scheduled_frame_published: Option<bool>,
    /// Exact current authoritative snake count once construction has completed.
    pub snake_count: Option<String>,
    /// Exact current authoritative pellet count once construction has completed.
    pub pellet_count: Option<String>,
    /// First bounded terminal panic detail, when the session is faulted.
    pub fault_detail: Option<String>,
}

/// Rust-only scalar snapshot used between a libuv worker and N-API resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshRunScalarSnapshot {
    pub(super) phase: &'static str,
    pub(super) transition_epoch: Option<u64>,
    pub(super) generation: Option<u64>,
    pub(super) completed_step: Option<u64>,
    pub(super) checkpoint_published: Option<bool>,
    pub(super) persistence_acknowledged: Option<bool>,
    pub(super) authority_published: Option<bool>,
    pub(super) initial_frame_published: Option<bool>,
    pub(super) first_scheduled_frame_published: Option<bool>,
    pub(super) snake_count: Option<usize>,
    pub(super) pellet_count: Option<usize>,
    pub(super) fault_detail: Option<String>,
}

/// Test-only constructor receipt, separate from the production snapshot contract.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct QualifiedFreshRunInitialization {
    pub snapshot: ExperimentalFreshRunSnapshot,
    pub trained_cohort_json: Option<String>,
    pub capacity_audit_json: Option<String>,
}

/// JavaScript initialization limits for the experimental engine.
///
/// The adapter additionally caps queue-entry metadata, individual payloads,
/// batch payloads, and total queued bytes. Those caps prevent a JavaScript
/// caller from turning a sparse array or nominal limit into an attempted
/// multi-gigabyte allocation; they are not simulation workload promises.
#[napi(object)]
pub struct ExperimentalEngineOptions {
    /// Must equal the exported Stage 3 engine contract version.
    pub contract_version: f64,
    /// Maximum queued command batches.
    pub max_inbound_batches: f64,
    /// Maximum queued commands across all batches.
    pub max_inbound_commands: f64,
    /// Maximum queued payload bytes across all batches.
    pub max_inbound_owned_bytes: f64,
    /// Maximum commands in one atomic batch.
    pub max_batch_commands: f64,
    /// Maximum payload bytes in one atomic batch.
    pub max_batch_owned_bytes: f64,
    /// Maximum queued reliable output events.
    pub max_output_reliable: f64,
    /// Maximum queued reliable output payload bytes.
    pub max_output_reliable_owned_bytes: f64,
    /// Maximum queued discrete output events.
    pub max_output_discrete: f64,
    /// Maximum queued discrete output payload bytes.
    pub max_output_discrete_owned_bytes: f64,
    /// Maximum bytes across all normal output classes.
    pub max_output_total_owned_bytes: f64,
    /// Maximum bytes owned by one output event.
    pub max_output_event_owned_bytes: f64,
    /// Maximum connections retaining replaceable frames.
    pub max_output_frame_connections: f64,
}

/// One prepared event returned by a bounded output drain.
#[napi(object)]
pub struct ExperimentalEngineEvent {
    /// Stable event kind.
    pub kind: String,
    /// Exact command or publication sequence when the event has one.
    pub sequence: Option<BigInt>,
    /// Exact probe correlation identifier when applicable.
    pub correlation_id: Option<BigInt>,
    /// Exact connection identifier for a frame event.
    pub connection_id: Option<BigInt>,
    /// Already-prepared event bytes when applicable.
    pub payload: Option<Uint8Array>,
    /// Stable fault category when this is a fault event.
    pub fault_code: Option<String>,
    /// Bounded fault diagnostic when this is a fault event.
    pub fault_detail: Option<String>,
}

/// Result of one bounded output drain.
#[napi(object)]
pub struct ExperimentalEngineDrain {
    /// Events removed by this call in engine priority order.
    pub events: Vec<ExperimentalEngineEvent>,
    /// Whether the same consumer should drain again without waiting for a wake.
    pub more_work: bool,
    /// Exact output-generation counter observed after the drain.
    pub generation: BigInt,
}

/// Small operational state that never copies authoritative world data.
#[napi(object)]
pub struct ExperimentalEngineHealth {
    /// One-shot lifecycle name.
    pub lifecycle: String,
    /// Current inbound batch depth.
    pub inbound_batches: BigInt,
    /// Current inbound command depth.
    pub inbound_commands: BigInt,
    /// Current inbound owned bytes.
    pub inbound_owned_bytes: BigInt,
    /// Highest observed inbound batch depth.
    pub inbound_high_water_batches: BigInt,
    /// Highest observed inbound command depth.
    pub inbound_high_water_commands: BigInt,
    /// Highest observed inbound payload bytes.
    pub inbound_high_water_owned_bytes: BigInt,
    /// Exact rejected-submission count.
    pub inbound_rejections: BigInt,
    /// Exact accepted batches discarded by the first terminal fault.
    pub inbound_fault_discarded_batches: BigInt,
    /// Exact accepted commands discarded by the first terminal fault.
    pub inbound_fault_discarded_commands: BigInt,
    /// Exact accepted payload bytes discarded by the first terminal fault.
    pub inbound_fault_discarded_owned_bytes: BigInt,
    /// Exact last admitted command sequence.
    pub inbound_last_accepted_sequence: Option<BigInt>,
    /// Whether the out-of-band stop flag is set.
    pub inbound_stop_requested: bool,
    /// Current reliable output count.
    pub output_reliable: BigInt,
    /// Current reliable output-owned bytes.
    pub output_reliable_owned_bytes: BigInt,
    /// Current discrete output count.
    pub output_discrete: BigInt,
    /// Current discrete output-owned bytes.
    pub output_discrete_owned_bytes: BigInt,
    /// Whether one replaceable stats item is retained.
    pub output_has_stats: bool,
    /// Current replaceable frame count.
    pub output_frames: BigInt,
    /// Current total normal output-owned bytes.
    pub output_owned_bytes: BigInt,
    /// Highest observed normal output event count.
    pub output_high_water_count: BigInt,
    /// Highest observed normal output-owned bytes.
    pub output_high_water_owned_bytes: BigInt,
    /// Exact reliable/discrete output overflow attempts.
    pub output_priority_overflows: BigInt,
    /// Exact stats replacement count.
    pub output_stats_replacements: BigInt,
    /// Exact frame replacement count.
    pub output_frame_replacements: BigInt,
    /// Exact stale stats publication count.
    pub output_stale_stats: BigInt,
    /// Exact stale frame publication count.
    pub output_stale_frames: BigInt,
    /// Exact stats rejection count.
    pub output_stats_rejections: BigInt,
    /// Exact frame rejection count.
    pub output_frame_rejections: BigInt,
    /// Exact stats eviction count.
    pub output_stats_evictions: BigInt,
    /// Exact frame eviction count.
    pub output_frame_evictions: BigInt,
    /// Whether the out-of-capacity first-fault slot is occupied.
    pub output_has_reserved_fault: bool,
    /// Exact processed-batch count.
    pub processed_batches: BigInt,
    /// Exact processed-command count.
    pub processed_commands: BigInt,
    /// Exact output generation.
    pub wake_generation: BigInt,
    /// Exact attempted-wake count.
    pub wake_attempts: BigInt,
    /// Exact accepted-wake count.
    pub wake_notifications: BigInt,
    /// Exact failed-wake count.
    pub wake_failures: BigInt,
    /// Exact wake re-arm race count.
    pub wake_rearm_races: BigInt,
    /// Whether one coalesced wake is currently outstanding.
    pub wake_pending: bool,
    /// First terminal fault category, when present.
    pub fault_code: Option<String>,
    /// First bounded terminal fault detail, when present.
    pub fault_detail: Option<String>,
}
