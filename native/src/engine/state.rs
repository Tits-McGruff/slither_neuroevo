//! Rust-owned persistent state contracts for the authoritative engine.
//!
//! This module deliberately contains data ownership, admission estimates, and
//! validation only. Gameplay behavior, N-API conversion, serialization,
//! database access, and managed-checkpoint I/O belong to later layers. A
//! candidate becomes authoritative only after the complete candidate has been
//! validated against its compiled graph and caller-supplied memory ceiling.

use super::baseline::{BaselineLifecycleState, BaselineSlotRuntime};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::graph::{
    CompiledGraph, CompiledNode, GraphBundle, GraphEdge, GraphNodeKind, GraphNodeSpec,
    GraphOutputRef, GraphSpec,
};
use super::rng::{RngError, SerializedRngState, StatefulRng};
use super::sensors::SensorGenerationState;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::mem::size_of;
use std::sync::{Arc, Mutex};

/// State-contract version implemented by this module.
pub const ENGINE_STATE_VERSION: u32 = 1;
/// Protocol 2 is the compatibility target for the first Rust cutover.
pub const PROTOCOL_VERSION: u32 = 2;
/// Binary display-frame v1 remains the first browser target.
pub const SERIALIZER_VERSION: u32 = 1;
/// Sensor v3 is the only supported sensor layout.
pub const SENSOR_VERSION: u32 = 3;
/// Version of the independent RNG-stream bundle.
pub const RNG_BUNDLE_VERSION: u32 = 1;
/// Version of normalized configuration encoded by this state contract.
pub const NORMALIZED_CONFIG_VERSION: u32 = 1;
/// Version of the monotonic allocator continuation bundle.
pub const ALLOCATOR_VERSION: u32 = 1;
/// Managed checkpoint version selected by the approved plan.
pub const CHECKPOINT_VERSION: u32 = 3;
/// Exact generation-boundary contract implemented by this module.
pub const GENERATION_BOUNDARY_VERSION: u32 = 1;
/// Largest integer that binary frame v1 can represent exactly as `f32`.
pub const FRAME_V1_MAX_EXACT_ID: u32 = 16_777_216;
/// One-past-the-end public-ID value used to represent exhausted allocation.
pub const FRAME_V1_EXHAUSTED_ID: u32 = FRAME_V1_MAX_EXACT_ID + 1;
/// Conservative strong/weak counter words stored beside each `Arc` allocation.
const ARC_COUNTER_BYTES: usize = 2 * size_of::<usize>();

/// First internal ID in the external-controller entity domain.
pub const EXTERNAL_ENTITY_ID_START: u64 = 1_u64 << 62;
/// First internal ID in the baseline-bot entity domain.
pub const BASELINE_ENTITY_ID_START: u64 = 1_u64 << 63;
/// First internal ID in the Hall-of-Fame resurrection entity domain.
pub const RESURRECTED_ENTITY_ID_START: u64 = (1_u64 << 63) | (1_u64 << 62);
/// Exhaustion sentinel for the resurrection entity domain.
pub const RESURRECTED_ENTITY_ID_EXHAUSTED: u64 = u64::MAX;

/// Exact running-build identity and admission ceiling supplied by the wrapper.
#[derive(Debug, PartialEq, Eq)]
pub struct StateAdmissionPolicy {
    /// Remaining hard peak ceiling after the caller accounts for the current
    /// engine, Node, transfer spools, and any simultaneously staged state.
    pub memory_ceiling_bytes: usize,
    /// Source revision of the currently loaded native addon.
    pub expected_source_revision: String,
    /// Source-derived engine ABI/build identity of the loaded addon.
    pub expected_engine_build_id: String,
    /// Raw 64-digit lowercase SHA-256 exported by the loaded native addon.
    pub expected_source_sha256: String,
    /// Target triple whose floating-point/runtime contract is active.
    pub expected_target_triple: String,
    /// Cargo build profile of the loaded addon.
    pub expected_build_profile: String,
    /// Production or test-hooks build-class identity.
    pub expected_build_class: String,
    /// Exact compiler version captured by the native build.
    pub expected_rustc_version: String,
    /// Versioned digest of effective correctness-relevant build attributes.
    pub expected_build_contract_sha256: String,
    /// Stable math backend identity included in exact-continuation scope.
    pub expected_math_backend: String,
    /// Versioned path-and-value-kind layout expected for normalized settings.
    pub expected_settings_schema_sha256: String,
}

/// Version identities required to interpret one engine state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractVersions {
    /// Rust-owned state layout version.
    pub state: u32,
    /// Coarse engine contract/ABI version.
    pub engine: u32,
    /// Browser and external-client protocol version.
    pub protocol: u32,
    /// Binary display-frame version.
    pub serializer: u32,
    /// Sensor layout version.
    pub sensor: u32,
    /// RNG bundle version.
    pub rng_bundle: u32,
    /// Managed checkpoint payload version.
    pub checkpoint: u32,
    /// Canonical compiled-graph layout version.
    pub graph_layout: u32,
}

/// Stable experiment identity and source/config provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunIdentity {
    /// Evolutionary lineage identity.
    pub run_id: String,
    /// Normalized root seed.
    pub seed: u32,
    /// Monotonic normalized-config revision.
    pub config_revision: u64,
    /// Digest of the complete normalized configuration.
    pub config_hash: String,
    /// Exact source revision used for version-scoped continuation.
    pub source_revision: String,
    /// Source-derived engine build identity.
    pub engine_build_id: String,
    /// Raw 64-digit lowercase SHA-256 of the selected native source inputs.
    pub source_sha256: String,
    /// Exact compilation target used by this continuation.
    pub target_triple: String,
    /// Cargo build profile used by this continuation.
    pub build_profile: String,
    /// Production or test-hooks build class used by this continuation.
    pub build_class: String,
    /// Exact compiler version used by this continuation.
    pub rustc_version: String,
    /// Versioned digest of effective correctness-relevant build attributes.
    pub build_contract_sha256: String,
    /// Stable scalar/neural math implementation identity.
    pub math_backend: String,
}

/// Typed value for one complete normalized configuration entry.
#[derive(Clone, Debug, PartialEq)]
pub enum NormalizedSettingValue {
    /// Boolean setting.
    Bool(bool),
    /// Integral setting retained without floating-point conversion.
    Integer(i64),
    /// Finite floating-point setting.
    Float(f64),
    /// Bounded textual setting or stable enum label.
    Text(String),
}

/// One path/value pair from the complete normalized configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedSetting {
    /// Stable configuration path.
    pub path: String,
    /// Normalized value for that path.
    pub value: NormalizedSettingValue,
}

/// Configuration admitted against one explicit versioned settings schema.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedEngineConfig {
    /// Configuration schema version.
    pub version: u32,
    /// Path-sorted normalized configuration complete for the admitted schema.
    pub settings: Vec<NormalizedSetting>,
    /// SHA-256 of the sorted setting paths and their value kinds.
    pub settings_schema_sha256: String,
    /// Collision-safe compiled-graph architecture identity.
    pub graph_architecture_key: String,
    /// Fixed authoritative physics step in seconds.
    pub fixed_step_seconds: f64,
    /// Requested simulated-seconds multiplier.
    pub requested_sim_speed: f64,
    /// Configured world radius.
    pub world_radius: f64,
    /// Exact dense evolved-population count.
    pub population_count: usize,
    /// Exact configured durable baseline-bot slot count.
    pub baseline_count: usize,
    /// Maximum admitted world snake records across all controller kinds.
    pub max_world_snakes: usize,
    /// Maximum admitted non-population neural brains.
    pub max_non_population_brains: usize,
    /// Maximum requested body-point storage.
    pub max_body_points: usize,
    /// Maximum requested pellet storage.
    pub max_pellets: usize,
    /// Declared peak bytes for derived collision/sensor spatial indexes.
    pub spatial_index_bytes: usize,
    /// Declared peak bytes for activation and calculation-worker scratch.
    pub worker_scratch_bytes: usize,
    /// Declared peak bytes for checkpoint construction scratch.
    pub checkpoint_scratch_bytes: usize,
    /// Latest input hold duration in wall-clock milliseconds.
    pub controller_input_hold_ms: u64,
    /// Exclusive disconnect grace in wall-clock milliseconds.
    pub controller_disconnect_grace_ms: u64,
}

/// Stable identity for a neural brain allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrainHandle {
    /// Monotonic allocation identity; zero is invalid.
    pub id: u64,
    /// Population/brain epoch guarding against stale state inheritance.
    pub epoch: u64,
}

/// Lineage metadata retained independently from per-frame snake data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenomeLineage {
    /// Stable genome identity; zero is invalid.
    pub genome_id: u64,
    /// Generation in which this genome was created.
    pub birth_generation: u64,
    /// Optional stable first-parent genome identity.
    pub parent_a: Option<u64>,
    /// Optional stable second-parent genome identity.
    pub parent_b: Option<u64>,
}

/// One dense evolved-population genome with packed neural parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct PopulationGenome {
    /// Dense stable slot, equal to this record's population-array index.
    pub slot: u32,
    /// Brain allocation owning recurrent state for this slot.
    pub brain: BrainHandle,
    /// Versioned lineage metadata.
    pub lineage: GenomeLineage,
    /// Finite retained fitness.
    pub fitness: f64,
    /// Packed graph parameters in compiled-graph order.
    pub weights: Box<[f32]>,
}

/// The entity that owns one brain allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainOwner {
    /// Dense evolved-population slot.
    PopulationSlot(u32),
    /// Stable non-population snake identity.
    Entity(u64),
}

/// Recurrent state keyed by stable brain handle rather than snake-array index.
#[derive(Clone, Debug, PartialEq)]
pub struct BrainRuntimeState {
    /// Stable allocation and epoch.
    pub handle: BrainHandle,
    /// Population slot or non-population entity owning this brain.
    pub owner: BrainOwner,
    /// Packed parameters for a non-population brain. Population-brain weights
    /// remain owned once by the matching [`PopulationGenome`].
    pub non_population_weights: Option<Box<[f32]>>,
    /// Packed recurrent state in compiled-graph order.
    pub recurrent: Box<[f32]>,
}

/// Independent authoritative RNG continuations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RngStateBundle {
    /// Bundle format version.
    pub version: u32,
    /// World construction, ambient pellet, and gameplay stream.
    pub world: SerializedRngState,
    /// Genome initialization, selection, and mutation stream.
    pub evolution: SerializedRngState,
    /// External-controller stream, isolated from world/evolution draws.
    /// Connection bookkeeping itself must not advance this stream.
    pub external_controller: SerializedRngState,
    /// Stable per-baseline-slot streams.
    pub baselines: Vec<BaselineRngState>,
}

/// One stable baseline-bot RNG continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaselineRngState {
    /// Dense baseline slot.
    pub slot: u32,
    /// Independent continuation for this slot.
    pub state: SerializedRngState,
}

/// Durable baseline-strategy state. Strategy evaluation exists in Stage 5;
/// the complete shared-observation and fixed-step authority join remains pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineStrategyState {
    /// General exploration.
    Roam,
    /// Food-seeking behavior.
    Seek,
    /// Hazard-avoidance behavior.
    Avoid,
    /// Explicit boost behavior.
    Boost,
}

/// Monotonic deterministic allocation continuations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllocatorState {
    /// Bundle format version.
    pub version: u32,
    /// Next stable internal entity identity.
    pub next_entity_id: u64,
    /// Next stable brain-allocation identity.
    pub next_brain_id: u64,
    /// Next stable genome-lineage identity.
    pub next_genome_id: u64,
    /// Next controller-lease identity.
    pub next_controller_lease_id: u64,
    /// Next exact frame-v1 public identity, or the exhaustion sentinel.
    pub next_frame_v1_id: u32,
    /// Next external-controller identity candidate.
    pub next_external_id: u64,
    /// Next baseline-bot identity candidate.
    pub next_baseline_id: u64,
    /// Next Hall-of-Fame resurrection identity candidate.
    pub next_resurrected_id: u64,
}

/// One atomically reserved contiguous range of exact frame-v1 IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameV1IdReservation {
    /// First reserved public ID.
    pub first: u32,
    /// Last reserved public ID, inclusive.
    pub last: u32,
}

/// One atomically reserved contiguous range of internal `u64` IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InternalIdReservation {
    /// First reserved identity.
    pub first: u64,
    /// Last reserved identity, inclusive.
    pub last: u64,
}

impl AllocatorState {
    /// Reserve `count` public IDs without partial mutation on exhaustion.
    pub fn reserve_frame_v1_ids(
        &mut self,
        count: u32,
    ) -> Result<Option<FrameV1IdReservation>, StateError> {
        if count == 0 {
            return Ok(None);
        }
        let first = self.next_frame_v1_id;
        if first == 0 || first >= FRAME_V1_EXHAUSTED_ID {
            return Err(StateError::IdExhausted {
                kind: "frame-v1",
                requested: u64::from(count),
            });
        }
        let last = first
            .checked_add(count - 1)
            .filter(|last| *last <= FRAME_V1_MAX_EXACT_ID)
            .ok_or(StateError::IdExhausted {
                kind: "frame-v1",
                requested: u64::from(count),
            })?;
        let next = last.checked_add(1).ok_or(StateError::ArithmeticOverflow {
            context: "frame-v1 ID reservation",
        })?;
        self.next_frame_v1_id = next;
        Ok(Some(FrameV1IdReservation { first, last }))
    }

    /// Reserve general evolved-snake/pellet entity IDs atomically.
    pub fn reserve_entity_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(
            &mut self.next_entity_id,
            count,
            1,
            EXTERNAL_ENTITY_ID_START,
            "entity",
        )
    }

    /// Reserve external-controller entity IDs atomically.
    pub fn reserve_external_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(
            &mut self.next_external_id,
            count,
            EXTERNAL_ENTITY_ID_START,
            BASELINE_ENTITY_ID_START,
            "external entity",
        )
    }

    /// Reserve baseline-bot entity IDs atomically.
    pub fn reserve_baseline_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(
            &mut self.next_baseline_id,
            count,
            BASELINE_ENTITY_ID_START,
            RESURRECTED_ENTITY_ID_START,
            "baseline entity",
        )
    }

    /// Reserve Hall-of-Fame resurrection entity IDs atomically.
    pub fn reserve_resurrected_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(
            &mut self.next_resurrected_id,
            count,
            RESURRECTED_ENTITY_ID_START,
            RESURRECTED_ENTITY_ID_EXHAUSTED,
            "resurrected entity",
        )
    }

    /// Reserve neural brain-handle IDs atomically.
    pub fn reserve_brain_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(&mut self.next_brain_id, count, 1, u64::MAX, "brain")
    }

    /// Reserve genome-lineage IDs atomically.
    pub fn reserve_genome_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(&mut self.next_genome_id, count, 1, u64::MAX, "genome")
    }

    /// Reserve controller-lease IDs atomically.
    pub fn reserve_controller_lease_ids(
        &mut self,
        count: u64,
    ) -> Result<Option<InternalIdReservation>, StateError> {
        reserve_internal_ids(
            &mut self.next_controller_lease_id,
            count,
            1,
            u64::MAX,
            "controller lease",
        )
    }
}

fn reserve_internal_ids(
    next: &mut u64,
    count: u64,
    start: u64,
    exhausted: u64,
    kind: &'static str,
) -> Result<Option<InternalIdReservation>, StateError> {
    if count == 0 {
        return Ok(None);
    }
    let first = *next;
    if first < start || first >= exhausted {
        return Err(StateError::IdExhausted {
            kind,
            requested: count,
        });
    }
    let last = first
        .checked_add(count - 1)
        .filter(|last| *last < exhausted)
        .ok_or(StateError::IdExhausted {
            kind,
            requested: count,
        })?;
    let following = last.checked_add(1).ok_or(StateError::ArithmeticOverflow {
        context: "internal ID reservation",
    })?;
    *next = following;
    Ok(Some(InternalIdReservation { first, last }))
}

/// Exact checkpoint boundary kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationBoundaryKind {
    /// Initial population assigned for a new run.
    RunStart,
    /// Evolved population assigned after a completed generation.
    Generation,
}

/// Whether state is at an exact save boundary or in a running round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityPhase {
    /// Exact pre-spawn, pre-sensor, zero-recurrent-state boundary.
    GenerationBoundary(GenerationBoundaryKind),
    /// Live non-checkpoint state. Mid-round save/restore is not implied.
    Running,
}

/// Minimal generation and scheduler continuation.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationState {
    /// Boundary schema version.
    pub boundary_version: u32,
    /// Current generation, starting at one.
    pub generation: u64,
    /// Number of fully committed authoritative fixed steps.
    pub completed_step: u64,
    /// Current population/brain epoch, starting at one.
    pub population_epoch: u64,
    /// Elapsed simulated seconds in the current round.
    pub elapsed_seconds: f64,
    /// Wall-clock scheduler debt in simulated seconds.
    pub wall_accumulator_seconds: f64,
    /// Best fitness observed before this state.
    pub best_fitness_ever: f64,
}

/// Stable class of world snake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnakeKind {
    /// Evolved population member.
    Evolved,
    /// Deterministic built-in baseline bot.
    Baseline,
    /// Browser player or external RL-controlled snake.
    External,
    /// Hall-of-Fame resurrection.
    Resurrected,
}

/// Double-precision world-space point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldPoint {
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
}

/// Range into pooled body-point storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyRange {
    /// First point in pooled storage.
    pub start: usize,
    /// Number of points, in head-to-tail order.
    pub len: usize,
}

/// Persistent scalar skeleton for one snake.
#[derive(Clone, Debug, PartialEq)]
pub struct SnakeState {
    /// Stable internal identity.
    pub id: u64,
    /// Exact public identity used by frame v1 and client commands.
    pub frame_v1_id: u32,
    /// Entity class.
    pub kind: SnakeKind,
    /// Whether the snake participates in live simulation.
    pub alive: bool,
    /// Dense evolved-population slot, when applicable.
    pub population_slot: Option<u32>,
    /// Stable neural brain, when applicable.
    pub brain: Option<BrainHandle>,
    /// Stable baseline slot, only for a built-in baseline snake.
    pub baseline_slot: Option<u32>,
    /// Durable strategy state, only for a built-in baseline snake.
    pub baseline_strategy: Option<BaselineStrategyState>,
    /// Current and previous head positions.
    pub position: WorldPoint,
    /// Previous committed head position.
    pub previous_position: WorldPoint,
    /// Heading in radians.
    pub direction: f64,
    /// Collision/render radius.
    pub radius: f64,
    /// Current speed.
    pub speed: f64,
    /// Current boost state.
    pub boost: bool,
    /// Age in simulated seconds.
    pub age_seconds: f64,
    /// Accumulated food value.
    pub food: f64,
    /// Accumulated points.
    pub points: f64,
    /// Stable integer kill count.
    pub kills: u64,
    /// Desired body length.
    pub target_length: f64,
    /// Current finite fitness.
    pub fitness: f64,
    /// Latest selected turn input.
    pub turn: f32,
    /// Previous selected turn input.
    pub previous_turn: f32,
    /// Latest selected boost input.
    pub input_boost: bool,
    /// Previous selected boost input.
    pub previous_input_boost: bool,
    /// Controller cadence accumulator; control-phase version 1 reserves its
    /// maximum admitted interval as the pending-first-neural-action sentinel.
    pub control_accumulator_seconds: f64,
    /// Points at the prior delivered-observation boundary.
    pub delivered_observation_points: f64,
    /// Pooled head-to-tail body range.
    pub body: BodyRange,
    /// Stable display skin/color identifier.
    pub skin: u32,
}

/// Persistent scalar skeleton for one pellet.
#[derive(Clone, Debug, PartialEq)]
pub struct PelletState {
    /// Stable internal identity.
    pub id: u64,
    /// World position.
    pub position: WorldPoint,
    /// Finite positive food value.
    pub value: f64,
    /// Stable pellet-kind identifier.
    pub kind: u32,
    /// Stable display color identifier.
    pub color: u32,
    /// Optional owning snake/entity identity.
    pub owner: Option<u64>,
}

/// External controller class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerKind {
    /// Interactive browser player.
    Player,
    /// Separate reinforcement-learning client.
    ReinforcementLearning,
}

/// Current wall-time lease status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerLeaseStatus {
    /// Live socket owns the snake. Connected leases carry no stored deadline;
    /// Stage 5 action selection neutralizes input by latest-action wall age.
    Connected,
    /// Disconnected owner retains exclusive control while its last action is
    /// held for the configured short wall-time interval.
    HoldingLastInput,
    /// The input-hold interval expired, so exclusive grace remains with
    /// neutral steering and boost disabled.
    ReservedNeutral,
    /// Grace expired and one explicit neural takeover has occurred.
    NeuralTakeover,
}

/// Latest-value external action retained independently from frame/sensor flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatestControllerAction {
    /// Finite turn value in `[-1, 1]`.
    pub turn: f32,
    /// Boost request.
    pub boost: bool,
    /// Client-reported tick retained only for diagnostics.
    pub client_tick: u64,
    /// Monotonic Node-assigned command arrival sequence.
    pub arrival_sequence: u64,
    /// Monotonic wall milliseconds when accepted.
    pub accepted_at_ms: u64,
}

/// Rust-owned wall-time controller lease.
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerLease {
    /// Stable monotonic lease identity.
    pub id: u64,
    /// Stable internal snake identity.
    pub snake_id: u64,
    /// Player or RL owner.
    pub kind: ControllerKind,
    /// Current connection identity; absent while reserved/taken over.
    pub connection_id: Option<u64>,
    /// Run/session scope preventing cross-run reclaim.
    pub scope: String,
    /// Opaque OS-entropy reclaim token. It is not generated from engine RNG.
    pub resume_token: String,
    /// Current ownership status.
    pub status: ControllerLeaseStatus,
    /// Latest accepted action.
    pub latest_action: LatestControllerAction,
    /// Latest monotonic wall time accepted by this lease state machine.
    pub last_observed_at_ms: u64,
    /// Monotonic wall milliseconds when disconnect occurred.
    pub disconnected_at_ms: Option<u64>,
    /// Checked latest-action-plus-input-hold deadline, absent while connected.
    pub input_hold_expires_at_ms: Option<u64>,
    /// Checked disconnect-plus-grace deadline, absent while connected.
    pub grace_expires_at_ms: Option<u64>,
    /// Wall milliseconds when the one explicit takeover committed.
    pub takeover_committed_at_ms: Option<u64>,
}

/// Complete world-storage skeleton. Spatial indexes are derived, not authority.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldState {
    /// Snake records.
    pub snakes: Vec<SnakeState>,
    /// Pooled head-to-tail body coordinates.
    pub body_points: Vec<WorldPoint>,
    /// Pellet records.
    pub pellets: Vec<PelletState>,
    /// External controller leases.
    pub controller_leases: Vec<ControllerLease>,
}

/// Generation-scoped continuation that must move with the authoritative world.
///
/// Ordinary checkpoints do not encode these values because their admitted
/// generation boundary is explicitly pre-spawn: the ambient credit and sensor
/// best are zero and no baseline slot has been initialized. A running state,
/// however, retains all three values through the same atomic publication as the
/// world, RNG, allocators, brains, and scheduler continuation.
#[derive(Clone, Debug, PartialEq)]
pub struct FixedStepContinuationState {
    /// Fractional ambient-pellet credit carried between fixed steps.
    pub ambient_pellet_accumulator: f64,
    /// Durable generation-scoped baseline slot timers and actions.
    pub baseline_lifecycle: BaselineLifecycleState,
    /// Monotonic generation-best score used by sensor-v3 normalization.
    pub sensor_generation: SensorGenerationState,
}

impl FixedStepContinuationState {
    /// Construct the only continuation admitted at an exact generation boundary.
    #[must_use]
    pub fn generation_boundary() -> Self {
        Self {
            ambient_pellet_accumulator: 0.0,
            baseline_lifecycle: BaselineLifecycleState::generation_boundary(),
            sensor_generation: SensorGenerationState::new(),
        }
    }
}

/// Fully assembled but not-yet-authoritative state candidate.
///
/// The N-API/checkpoint decoder must separately preflight declared record and
/// byte counts before constructing this already-owned value. The checked
/// admission in [`AuthoritativeState::validate_and_own`] prevents publication
/// of an over-budget candidate, but cannot undo allocations the decoder has
/// already performed.
#[derive(Clone, Debug, PartialEq)]
pub struct StateCandidate {
    /// Version identities.
    pub versions: ContractVersions,
    /// Run and build identity.
    pub identity: RunIdentity,
    /// Complete normalized configuration.
    pub config: NormalizedEngineConfig,
    /// Exact checkpoint/live phase.
    pub phase: AuthorityPhase,
    /// Generation/scheduler continuation.
    pub generation: GenerationState,
    /// Generation-scoped fixed-step continuation.
    pub fixed_step: FixedStepContinuationState,
    /// Independent RNG continuation.
    pub rng: RngStateBundle,
    /// Monotonic allocation continuation.
    pub allocators: AllocatorState,
    /// Dense evolved population.
    pub population: Vec<PopulationGenome>,
    /// All population and admitted non-population neural states.
    pub brains: Vec<BrainRuntimeState>,
    /// Current world storage.
    pub world: WorldState,
}

/// Reusable checked memory estimate for an engine candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateMemoryEstimate {
    /// Rust structs and vector element buffers.
    pub structural_bytes: usize,
    /// Packed population parameters.
    pub weight_bytes: usize,
    /// Packed recurrent state.
    pub recurrent_bytes: usize,
    /// Dynamic graph metadata owned by the shared compiled graph.
    pub graph_bytes: usize,
    /// Dynamic UTF-8 metadata/settings/token storage.
    pub text_bytes: usize,
    /// Maximum binary display-frame allocation implied by the configuration.
    pub frame_bytes: usize,
    /// Declared derived spatial-index allocation.
    pub spatial_bytes: usize,
    /// Declared worker and checkpoint scratch allocations.
    pub scratch_bytes: usize,
    /// Conservative temporary validation-set/range allocation.
    pub validation_bytes: usize,
    /// Total estimated owned bytes.
    pub total_bytes: usize,
}

/// Validated Rust-owned state. Fields stay private so an invalid candidate
/// cannot be assembled by struct literal and accidentally published.
#[derive(Debug)]
pub struct AuthoritativeState {
    candidate: StateCandidate,
    graph: Arc<GraphBundle>,
    memory: StateMemoryEstimate,
}

impl AuthoritativeState {
    /// Validate the complete candidate and only then return publishable state.
    pub fn validate_and_own(
        candidate: StateCandidate,
        graph: Arc<GraphBundle>,
        policy: &StateAdmissionPolicy,
    ) -> Result<Self, StateError> {
        validate_policy(policy)?;
        validate_admission_header(&candidate, graph.compiled(), policy)?;
        let memory = estimate_state_memory(&candidate, &graph)?;
        if memory.total_bytes > policy.memory_ceiling_bytes {
            return Err(StateError::MemoryCeilingExceeded {
                estimated_bytes: memory.total_bytes,
                ceiling_bytes: policy.memory_ceiling_bytes,
            });
        }
        validate_candidate(&candidate, graph.compiled(), policy)?;
        Ok(Self {
            candidate,
            graph,
            memory,
        })
    }

    /// Read the immutable state contract.
    #[must_use]
    pub fn state(&self) -> &StateCandidate {
        &self.candidate
    }

    /// Read the immutable compiled graph shared by compatible genomes.
    #[must_use]
    pub fn graph(&self) -> &CompiledGraph {
        self.graph.compiled()
    }

    /// Read the original source graph retained with the compiled layout.
    #[must_use]
    pub fn graph_spec(&self) -> &GraphSpec {
        self.graph.spec()
    }

    /// Read the inseparable source graph and compiled-layout bundle.
    #[must_use]
    pub fn graph_bundle(&self) -> &GraphBundle {
        &self.graph
    }

    /// Read the admission estimate accepted at construction.
    #[must_use]
    pub fn memory_estimate(&self) -> StateMemoryEstimate {
        self.memory
    }

    /// Borrow the only state shape accepted by ordinary checkpoint encoding.
    pub fn checkpoint_boundary(&self) -> Result<GenerationBoundaryView<'_>, StateError> {
        let AuthorityPhase::GenerationBoundary(kind) = self.candidate.phase else {
            return invalid(
                "phase",
                "ordinary checkpoints require an exact generation boundary",
            );
        };
        Ok(GenerationBoundaryView { state: self, kind })
    }
}

/// Opaque proof that an authoritative state is checkpoint-eligible.
///
/// Checkpoint writers accept this view rather than a general running state.
#[derive(Debug)]
pub struct GenerationBoundaryView<'a> {
    state: &'a AuthoritativeState,
    kind: GenerationBoundaryKind,
}

impl GenerationBoundaryView<'_> {
    /// Read the validated boundary payload.
    #[must_use]
    pub fn state(&self) -> &StateCandidate {
        &self.state.candidate
    }

    /// Read the exact kind of generation boundary.
    #[must_use]
    pub fn kind(&self) -> GenerationBoundaryKind {
        self.kind
    }

    /// Read the original graph definition required by a checkpoint-v3 writer.
    #[must_use]
    pub fn graph_spec(&self) -> &GraphSpec {
        self.state.graph_spec()
    }

    /// Read the compiled graph identity required by a checkpoint-v3 writer.
    #[must_use]
    pub fn graph(&self) -> &CompiledGraph {
        self.state.graph()
    }

    /// Read the inseparable graph source/layout bundle.
    #[must_use]
    pub fn graph_bundle(&self) -> &GraphBundle {
        self.state.graph_bundle()
    }

    /// Read the already-admitted owned-state memory estimate so a checkpoint
    /// adapter can replace the configured scratch reservation with its actual
    /// bounded working-set requirement.
    #[must_use]
    pub fn memory_estimate(&self) -> StateMemoryEstimate {
        self.state.memory_estimate()
    }
}

/// Validation or checked-memory failure.
#[derive(Clone, Debug, PartialEq)]
pub enum StateError {
    /// Required version/identity/text field is missing or invalid.
    InvalidField { field: &'static str, reason: String },
    /// A finite scalar contract was violated.
    NonFinite { field: &'static str, index: usize },
    /// Checked byte/count arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Estimated state exceeds the caller-approved ceiling.
    MemoryCeilingExceeded {
        /// Checked state estimate.
        estimated_bytes: usize,
        /// Caller-approved ceiling.
        ceiling_bytes: usize,
    },
    /// A monotonic allocator cannot satisfy the requested range.
    IdExhausted {
        /// Allocation namespace.
        kind: &'static str,
        /// Number of identities requested atomically.
        requested: u64,
    },
    /// Population slots must be exactly dense and ordered.
    NonDensePopulationSlot { index: usize, slot: u32 },
    /// A stable identity was repeated.
    DuplicateId { kind: &'static str, id: u64 },
    /// A brain handle was repeated or mismatched.
    DuplicateBrainHandle(BrainHandle),
    /// Genome packed weight length disagrees with the compiled graph.
    WeightLength {
        /// Population slot.
        slot: u32,
        /// Required packed-float count.
        expected: usize,
        /// Supplied packed-float count.
        actual: usize,
    },
    /// A non-population brain's packed weight length is invalid.
    BrainWeightLength {
        /// Stable brain handle.
        handle: BrainHandle,
        /// Required packed-float count.
        expected: usize,
        /// Supplied packed-float count.
        actual: usize,
    },
    /// Brain recurrent-state length disagrees with the compiled graph.
    RecurrentLength {
        /// Brain handle.
        handle: BrainHandle,
        /// Required packed-float count.
        expected: usize,
        /// Supplied packed-float count.
        actual: usize,
    },
    /// Brain owner/epoch does not match its population or entity.
    InvalidBrainOwner(BrainHandle),
    /// Body storage range is invalid or overlaps another snake.
    InvalidBodyRange { snake_id: u64 },
    /// Exact generation boundary contains live/transient state.
    DirtyGenerationBoundary { reason: &'static str },
    /// RNG continuation failed strict algorithm/state validation.
    InvalidRng { stream: String, reason: String },
    /// A lease references no live snake.
    UnknownLeaseSnake(u64),
    /// A controller lease identity is repeated.
    DuplicateLeaseId(u64),
    /// Multiple leases target the same snake.
    DuplicateLeaseSnake(u64),
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => write!(formatter, "invalid {field}: {reason}"),
            Self::NonFinite { field, index } => {
                write!(formatter, "{field}[{index}] must be finite")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "checked arithmetic overflow while calculating {context}")
            }
            Self::MemoryCeilingExceeded {
                estimated_bytes,
                ceiling_bytes,
            } => write!(
                formatter,
                "state requires an estimated {estimated_bytes} bytes, exceeding the {ceiling_bytes}-byte ceiling"
            ),
            Self::IdExhausted { kind, requested } => {
                write!(formatter, "{kind} ID space cannot reserve {requested} identities")
            }
            Self::NonDensePopulationSlot { index, slot } => {
                write!(formatter, "population index {index} contains slot {slot}")
            }
            Self::DuplicateId { kind, id } => write!(formatter, "duplicate {kind} id {id}"),
            Self::DuplicateBrainHandle(handle) => write!(
                formatter,
                "duplicate brain handle {} at epoch {}",
                handle.id, handle.epoch
            ),
            Self::WeightLength {
                slot,
                expected,
                actual,
            } => write!(
                formatter,
                "population slot {slot} has {actual} weights; expected {expected}"
            ),
            Self::BrainWeightLength {
                handle,
                expected,
                actual,
            } => write!(
                formatter,
                "non-population brain {}:{} has {actual} weights; expected {expected}",
                handle.id, handle.epoch
            ),
            Self::RecurrentLength {
                handle,
                expected,
                actual,
            } => write!(
                formatter,
                "brain {}:{} has {actual} recurrent values; expected {expected}",
                handle.id, handle.epoch
            ),
            Self::InvalidBrainOwner(handle) => write!(
                formatter,
                "brain {}:{} has an invalid owner/epoch mapping",
                handle.id, handle.epoch
            ),
            Self::InvalidBodyRange { snake_id } => {
                write!(formatter, "snake {snake_id} has an invalid or overlapping body range")
            }
            Self::DirtyGenerationBoundary { reason } => {
                write!(formatter, "generation boundary is not clean: {reason}")
            }
            Self::InvalidRng { stream, reason } => {
                write!(formatter, "invalid RNG stream {stream}: {reason}")
            }
            Self::UnknownLeaseSnake(id) => write!(formatter, "lease references unknown snake {id}"),
            Self::DuplicateLeaseId(id) => write!(formatter, "duplicate controller lease id {id}"),
            Self::DuplicateLeaseSnake(id) => {
                write!(formatter, "multiple controller leases target snake {id}")
            }
        }
    }
}

impl Error for StateError {}

/// Checked `count * item_size` helper used before allocation/admission.
pub fn checked_allocation_bytes(
    count: usize,
    item_size: usize,
    context: &'static str,
) -> Result<usize, StateError> {
    count
        .checked_mul(item_size)
        .ok_or(StateError::ArithmeticOverflow { context })
}

/// Estimate complete candidate and compiled-graph owned memory without
/// publishing authority.
pub fn estimate_state_memory(
    candidate: &StateCandidate,
    graph: &GraphBundle,
) -> Result<StateMemoryEstimate, StateError> {
    let compiled = graph.compiled();
    let mut estimate = StateMemoryEstimate {
        structural_bytes: checked_add(
            size_of::<Mutex<AuthoritativeState>>(),
            ARC_COUNTER_BYTES,
            "authoritative runtime state wrapper",
        )?,
        ..StateMemoryEstimate::default()
    };

    // Charge the larger of already-owned capacity and declared peak capacity.
    // This prevents an empty generation-boundary world from passing admission
    // while declaring an unsafe later body/pellet/world allocation.
    add_structural::<NormalizedSetting>(&mut estimate, candidate.config.settings.capacity())?;
    add_structural::<PopulationGenome>(&mut estimate, candidate.population.capacity())?;
    let maximum_brains = candidate
        .config
        .population_count
        .checked_add(candidate.config.max_non_population_brains)
        .ok_or(StateError::ArithmeticOverflow {
            context: "maximum brain records",
        })?;
    add_structural::<BrainRuntimeState>(
        &mut estimate,
        candidate.brains.capacity().max(maximum_brains),
    )?;
    add_structural::<BaselineRngState>(&mut estimate, candidate.rng.baselines.capacity())?;
    add_structural::<BaselineSlotRuntime>(
        &mut estimate,
        candidate
            .fixed_step
            .baseline_lifecycle
            .slots
            .capacity()
            .max(candidate.config.baseline_count),
    )?;
    add_structural::<SnakeState>(
        &mut estimate,
        candidate
            .world
            .snakes
            .capacity()
            .max(candidate.config.max_world_snakes),
    )?;
    add_structural::<WorldPoint>(
        &mut estimate,
        candidate
            .world
            .body_points
            .capacity()
            .max(candidate.config.max_body_points),
    )?;
    add_structural::<PelletState>(
        &mut estimate,
        candidate
            .world
            .pellets
            .capacity()
            .max(candidate.config.max_pellets),
    )?;
    add_structural::<ControllerLease>(
        &mut estimate,
        candidate
            .world
            .controller_leases
            .capacity()
            .max(candidate.config.max_world_snakes),
    )?;

    for genome in &candidate.population {
        estimate.weight_bytes = checked_add(
            estimate.weight_bytes,
            checked_allocation_bytes(genome.weights.len(), size_of::<f32>(), "genome weights")?,
            "weight memory",
        )?;
    }
    let mut non_population_weight_bytes = 0usize;
    let mut current_recurrent_bytes = 0usize;
    for brain in &candidate.brains {
        if let Some(weights) = &brain.non_population_weights {
            non_population_weight_bytes = checked_add(
                non_population_weight_bytes,
                checked_allocation_bytes(
                    weights.len(),
                    size_of::<f32>(),
                    "non-population brain weights",
                )?,
                "weight memory",
            )?;
        }
        current_recurrent_bytes = checked_add(
            current_recurrent_bytes,
            checked_allocation_bytes(brain.recurrent.len(), size_of::<f32>(), "recurrent state")?,
            "recurrent memory",
        )?;
    }
    let requested_non_population_weights = checked_allocation_bytes(
        candidate.config.max_non_population_brains,
        checked_allocation_bytes(
            compiled.total_parameters,
            size_of::<f32>(),
            "one non-population weight block",
        )?,
        "maximum non-population weights",
    )?;
    estimate.weight_bytes = checked_add(
        estimate.weight_bytes,
        non_population_weight_bytes.max(requested_non_population_weights),
        "weight memory",
    )?;
    let requested_recurrent_bytes = checked_allocation_bytes(
        maximum_brains,
        checked_allocation_bytes(
            compiled.total_state_size,
            size_of::<f32>(),
            "one recurrent-state block",
        )?,
        "maximum recurrent state",
    )?;
    estimate.recurrent_bytes = current_recurrent_bytes.max(requested_recurrent_bytes);

    add_candidate_text(&mut estimate, candidate)?;
    estimate.graph_bytes = estimate_graph_memory(graph)?;
    estimate.frame_bytes = estimate_frame_v1_bytes(&candidate.config)?;
    estimate.spatial_bytes = candidate.config.spatial_index_bytes;
    estimate.scratch_bytes = checked_add(
        candidate.config.worker_scratch_bytes,
        candidate.config.checkpoint_scratch_bytes,
        "declared engine scratch",
    )?;
    estimate.validation_bytes = estimate_validation_memory(candidate)?;
    estimate.total_bytes = checked_sum(&[
        estimate.structural_bytes,
        estimate.weight_bytes,
        estimate.recurrent_bytes,
        estimate.graph_bytes,
        estimate.text_bytes,
        estimate.frame_bytes,
        estimate.spatial_bytes,
        estimate.scratch_bytes,
        estimate.validation_bytes,
    ])?;
    Ok(estimate)
}

/// Preflight the final generation-boundary allocation from decoded metadata and
/// declared numeric counts before a checkpoint decoder allocates packed Float32 buffers.
///
/// `candidate_shell` must contain the decoded identity/configuration/RNG/allocator,
/// population metadata, and population brain records, but its weight and recurrent
/// boxes may remain empty. The returned estimate charges the exact population
/// weight count plus the existing conservative configured recurrent/non-population
/// peaks used by ordinary state admission.
pub fn preflight_generation_boundary_allocation(
    candidate_shell: &StateCandidate,
    graph: &GraphBundle,
    declared_weight_floats: usize,
    declared_recurrent_floats: usize,
    policy: &StateAdmissionPolicy,
) -> Result<StateMemoryEstimate, StateError> {
    validate_policy(policy)?;
    validate_admission_header(candidate_shell, graph.compiled(), policy)?;
    validate_generation(&candidate_shell.generation)?;
    validate_rng_bundle(&candidate_shell.rng, &candidate_shell.config)?;
    validate_allocators(&candidate_shell.allocators)?;
    validate_generation_boundary(candidate_shell, graph.compiled())?;
    validate_fixed_step_continuation(candidate_shell)?;
    if candidate_shell.population.len() != candidate_shell.config.population_count
        || candidate_shell.brains.len() != candidate_shell.population.len()
    {
        return invalid(
            "population",
            "checkpoint allocation shell must contain one population brain per configured slot",
        );
    }
    if candidate_shell
        .population
        .iter()
        .any(|genome| !genome.weights.is_empty())
        || candidate_shell
            .brains
            .iter()
            .any(|brain| brain.non_population_weights.is_some() || !brain.recurrent.is_empty())
    {
        return invalid(
            "population",
            "checkpoint allocation shell must not allocate numeric payloads before preflight",
        );
    }
    let expected_weights = candidate_shell
        .population
        .len()
        .checked_mul(graph.compiled().total_parameters)
        .ok_or(StateError::ArithmeticOverflow {
            context: "declared checkpoint population weights",
        })?;
    let expected_recurrent = candidate_shell
        .brains
        .len()
        .checked_mul(graph.compiled().total_state_size)
        .ok_or(StateError::ArithmeticOverflow {
            context: "declared checkpoint recurrent state",
        })?;
    if declared_weight_floats != expected_weights || declared_recurrent_floats != expected_recurrent
    {
        return invalid(
            "population",
            "declared checkpoint numeric counts disagree with graph/population shape",
        );
    }
    let mut estimate = estimate_state_memory(candidate_shell, graph)?;
    let population_weight_bytes = checked_allocation_bytes(
        declared_weight_floats,
        size_of::<f32>(),
        "declared checkpoint population weight bytes",
    )?;
    estimate.weight_bytes = checked_add(
        estimate.weight_bytes,
        population_weight_bytes,
        "checkpoint preflight weight memory",
    )?;
    estimate.total_bytes = checked_add(
        estimate.total_bytes,
        population_weight_bytes,
        "checkpoint preflight total memory",
    )?;
    if estimate.total_bytes > policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: estimate.total_bytes,
            ceiling_bytes: policy.memory_ceiling_bytes,
        });
    }
    Ok(estimate)
}

/// Calculate the versioned schema identity for sorted normalized settings.
///
/// The encoding hashes the setting count followed by length-prefixed UTF-8
/// paths and one value-kind byte: bool `0`, integer `1`, float `2`, text `3`.
/// Values are deliberately excluded so TypeScript/N-API can reproduce the
/// admitted layout independently from a particular configuration instance.
pub fn normalized_settings_schema_hash(
    settings: &[NormalizedSetting],
) -> Result<String, StateError> {
    let mut hasher = Sha256::new();
    hasher.update(b"slither-normalized-settings-schema\0v1");
    hash_u64(
        &mut hasher,
        u64::try_from(settings.len()).map_err(|_| StateError::ArithmeticOverflow {
            context: "normalized settings schema count",
        })?,
    );
    let mut previous: Option<&[u8]> = None;
    for setting in settings {
        validate_text("config.settings.path", &setting.path)?;
        let path = setting.path.as_bytes();
        if previous.is_some_and(|prior| prior >= path) {
            return invalid(
                "config.settings",
                "paths must be unique and sorted by raw UTF-8 bytes",
            );
        }
        previous = Some(path);
        hash_text(&mut hasher, &setting.path)?;
        hasher.update([match setting.value {
            NormalizedSettingValue::Bool(_) => 0,
            NormalizedSettingValue::Integer(_) => 1,
            NormalizedSettingValue::Float(_) => 2,
            NormalizedSettingValue::Text(_) => 3,
        }]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Calculate the canonical SHA-256 identity of one normalized configuration.
///
/// The encoding is domain-separated and length-prefixed; it is an internal
/// state-identity contract rather than human-readable JSON.
pub fn normalized_config_hash(config: &NormalizedEngineConfig) -> Result<String, StateError> {
    let mut hasher = Sha256::new();
    hasher.update(b"slither-normalized-config\0v1");
    hash_u64(&mut hasher, u64::from(config.version));
    hash_text(&mut hasher, &config.settings_schema_sha256)?;
    hash_text(&mut hasher, &config.graph_architecture_key)?;
    hash_u64(&mut hasher, config.fixed_step_seconds.to_bits());
    hash_u64(&mut hasher, config.requested_sim_speed.to_bits());
    hash_u64(&mut hasher, config.world_radius.to_bits());
    for value in [
        config.population_count,
        config.baseline_count,
        config.max_world_snakes,
        config.max_non_population_brains,
        config.max_body_points,
        config.max_pellets,
        config.spatial_index_bytes,
        config.worker_scratch_bytes,
        config.checkpoint_scratch_bytes,
    ] {
        hash_u64(
            &mut hasher,
            u64::try_from(value).map_err(|_| StateError::ArithmeticOverflow {
                context: "canonical config integer",
            })?,
        );
    }
    hash_u64(&mut hasher, config.controller_input_hold_ms);
    hash_u64(&mut hasher, config.controller_disconnect_grace_ms);
    hash_u64(
        &mut hasher,
        u64::try_from(config.settings.len()).map_err(|_| StateError::ArithmeticOverflow {
            context: "canonical setting count",
        })?,
    );
    for setting in &config.settings {
        hash_text(&mut hasher, &setting.path)?;
        match &setting.value {
            NormalizedSettingValue::Bool(value) => {
                hasher.update([0, u8::from(*value)]);
            }
            NormalizedSettingValue::Integer(value) => {
                hasher.update([1]);
                hasher.update(value.to_le_bytes());
            }
            NormalizedSettingValue::Float(value) => {
                hasher.update([2]);
                hasher.update(value.to_bits().to_le_bytes());
            }
            NormalizedSettingValue::Text(value) => {
                hasher.update([3]);
                hash_text(&mut hasher, value)?;
            }
        }
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

fn hash_text(hasher: &mut Sha256, value: &str) -> Result<(), StateError> {
    hash_u64(
        hasher,
        u64::try_from(value.len()).map_err(|_| StateError::ArithmeticOverflow {
            context: "canonical config text length",
        })?,
    );
    hasher.update(value.as_bytes());
    Ok(())
}

fn require_integer_projection(
    config: &NormalizedEngineConfig,
    path: &'static str,
    expected: i64,
) -> Result<(), StateError> {
    let value = config
        .settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value);
    if value != Some(&NormalizedSettingValue::Integer(expected)) {
        return invalid("config.settings", "required integer projection differs");
    }
    Ok(())
}

fn require_float_projection(
    config: &NormalizedEngineConfig,
    path: &'static str,
    expected: f64,
) -> Result<(), StateError> {
    let value = config
        .settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value);
    match value {
        Some(NormalizedSettingValue::Float(actual)) if actual.to_bits() == expected.to_bits() => {
            Ok(())
        }
        _ => invalid("config.settings", "required floating projection differs"),
    }
}

/// Validate one complete candidate without mutating it.
fn validate_candidate(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    validate_admission_header(candidate, graph, policy)?;
    validate_generation(&candidate.generation)?;
    validate_rng_bundle(&candidate.rng, &candidate.config)?;
    validate_allocators(&candidate.allocators)?;
    validate_population(candidate, graph)?;
    validate_world(candidate)?;
    if matches!(candidate.phase, AuthorityPhase::GenerationBoundary(_)) {
        validate_generation_boundary(candidate, graph)?;
    }
    validate_fixed_step_continuation(candidate)?;
    Ok(())
}

fn validate_admission_header(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    validate_versions(&candidate.versions, graph)?;
    validate_identity(&candidate.identity, policy)?;
    validate_config(
        &candidate.config,
        &candidate.versions,
        &candidate.identity,
        policy,
    )?;
    if candidate.config.graph_architecture_key != graph.architecture_key {
        return invalid(
            "config.graph_architecture_key",
            "does not match the compiled graph",
        );
    }
    Ok(())
}

fn validate_policy(policy: &StateAdmissionPolicy) -> Result<(), StateError> {
    if policy.memory_ceiling_bytes == 0 {
        return invalid("admission.memory_ceiling_bytes", "ceiling must be positive");
    }
    validate_text(
        "admission.expected_source_revision",
        &policy.expected_source_revision,
    )?;
    validate_text(
        "admission.expected_engine_build_id",
        &policy.expected_engine_build_id,
    )?;
    validate_sha256_hex(
        "admission.expected_source_sha256",
        &policy.expected_source_sha256,
    )?;
    validate_text(
        "admission.expected_target_triple",
        &policy.expected_target_triple,
    )?;
    validate_text(
        "admission.expected_build_profile",
        &policy.expected_build_profile,
    )?;
    validate_build_class(
        "admission.expected_build_class",
        &policy.expected_build_class,
    )?;
    validate_text(
        "admission.expected_rustc_version",
        &policy.expected_rustc_version,
    )?;
    validate_sha256(
        "admission.expected_build_contract_sha256",
        &policy.expected_build_contract_sha256,
    )?;
    validate_text(
        "admission.expected_math_backend",
        &policy.expected_math_backend,
    )?;
    validate_sha256(
        "admission.expected_settings_schema_sha256",
        &policy.expected_settings_schema_sha256,
    )?;
    Ok(())
}

fn validate_versions(versions: &ContractVersions, graph: &CompiledGraph) -> Result<(), StateError> {
    if versions.state != ENGINE_STATE_VERSION {
        return invalid("versions.state", "unsupported state-contract version");
    }
    for (field, actual, expected) in [
        ("versions.engine", versions.engine, ENGINE_CONTRACT_VERSION),
        ("versions.protocol", versions.protocol, PROTOCOL_VERSION),
        (
            "versions.serializer",
            versions.serializer,
            SERIALIZER_VERSION,
        ),
        ("versions.sensor", versions.sensor, SENSOR_VERSION),
        (
            "versions.rng_bundle",
            versions.rng_bundle,
            RNG_BUNDLE_VERSION,
        ),
        (
            "versions.checkpoint",
            versions.checkpoint,
            CHECKPOINT_VERSION,
        ),
    ] {
        if actual != expected {
            return invalid(field, "unsupported contract version");
        }
    }
    if graph.layout_version == 0 || graph.architecture_key.is_empty() {
        return invalid("graph", "compiled graph identity is incomplete");
    }
    if versions.graph_layout != graph.layout_version {
        return invalid(
            "versions.graph_layout",
            "does not match the compiled graph layout",
        );
    }
    Ok(())
}

fn validate_identity(
    identity: &RunIdentity,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    validate_text("identity.run_id", &identity.run_id)?;
    validate_sha256("identity.config_hash", &identity.config_hash)?;
    validate_text("identity.source_revision", &identity.source_revision)?;
    validate_text("identity.engine_build_id", &identity.engine_build_id)?;
    validate_sha256_hex("identity.source_sha256", &identity.source_sha256)?;
    validate_text("identity.target_triple", &identity.target_triple)?;
    validate_text("identity.build_profile", &identity.build_profile)?;
    validate_build_class("identity.build_class", &identity.build_class)?;
    validate_text("identity.rustc_version", &identity.rustc_version)?;
    validate_sha256(
        "identity.build_contract_sha256",
        &identity.build_contract_sha256,
    )?;
    validate_text("identity.math_backend", &identity.math_backend)?;
    if identity.source_revision != policy.expected_source_revision
        || identity.engine_build_id != policy.expected_engine_build_id
        || identity.source_sha256 != policy.expected_source_sha256
        || identity.target_triple != policy.expected_target_triple
        || identity.build_profile != policy.expected_build_profile
        || identity.build_class != policy.expected_build_class
        || identity.rustc_version != policy.expected_rustc_version
        || identity.build_contract_sha256 != policy.expected_build_contract_sha256
        || identity.math_backend != policy.expected_math_backend
    {
        return invalid(
            "identity",
            "source/build/compiler/target/math identity does not match the loaded engine",
        );
    }
    Ok(())
}

fn validate_config(
    config: &NormalizedEngineConfig,
    versions: &ContractVersions,
    identity: &RunIdentity,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    if config.version != NORMALIZED_CONFIG_VERSION {
        return invalid("config.version", "unsupported normalized-config version");
    }
    if identity.config_revision == 0 {
        return invalid("identity.config_revision", "revision must be positive");
    }
    validate_positive_f64("config.fixed_step_seconds", config.fixed_step_seconds)?;
    validate_positive_f64("config.requested_sim_speed", config.requested_sim_speed)?;
    validate_positive_f64("config.world_radius", config.world_radius)?;
    if config.population_count == 0 {
        return invalid("config.population_count", "count must be positive");
    }
    let minimum_world_snakes = config
        .population_count
        .checked_add(config.baseline_count)
        .and_then(|count| count.checked_add(config.max_non_population_brains))
        .ok_or(StateError::ArithmeticOverflow {
            context: "maximum world snakes",
        })?;
    if config.max_world_snakes < minimum_world_snakes {
        return invalid(
            "config.max_world_snakes",
            "must cover population, baseline slots, and non-population brains",
        );
    }
    if config.max_body_points == 0 || config.max_pellets == 0 {
        return invalid(
            "config.capacity",
            "body and pellet capacities must be positive",
        );
    }
    if config.controller_input_hold_ms == 0
        || config.controller_disconnect_grace_ms == 0
        || config.controller_input_hold_ms >= config.controller_disconnect_grace_ms
    {
        return invalid(
            "config.controller_wall_time",
            "input hold must be positive and shorter than disconnect grace",
        );
    }
    if config.spatial_index_bytes == 0
        || config.worker_scratch_bytes == 0
        || config.checkpoint_scratch_bytes == 0
    {
        return invalid(
            "config.memory_reservations",
            "spatial, worker, and checkpoint scratch declarations must be positive",
        );
    }
    validate_text(
        "config.graph_architecture_key",
        &config.graph_architecture_key,
    )?;
    if config.settings.is_empty() {
        return invalid(
            "config.settings",
            "settings complete for the admitted schema are required",
        );
    }
    validate_sha256(
        "config.settings_schema_sha256",
        &config.settings_schema_sha256,
    )?;
    let actual_schema = normalized_settings_schema_hash(&config.settings)?;
    if actual_schema != config.settings_schema_sha256
        || actual_schema != policy.expected_settings_schema_sha256
    {
        return invalid(
            "config.settings_schema_sha256",
            "setting paths or value kinds do not match the admitted schema",
        );
    }
    for setting in &config.settings {
        match &setting.value {
            NormalizedSettingValue::Float(value) if !value.is_finite() => {
                return invalid("config.settings.value", "floating value must be finite");
            }
            NormalizedSettingValue::Text(value) => {
                validate_text("config.settings.value", value)?;
            }
            _ => {}
        }
    }
    require_integer_projection(config, "brain.sensorVersion", i64::from(versions.sensor))?;
    require_integer_projection(
        config,
        "baselineBots.count",
        i64::try_from(config.baseline_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "baseline count projection",
        })?,
    )?;
    require_float_projection(config, "simSpeed", config.requested_sim_speed)?;
    require_integer_projection(
        config,
        "snakeCount",
        i64::try_from(config.population_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "population count projection",
        })?,
    )?;
    if config.world_radius.fract() != 0.0
        || config.world_radius > i64::MAX as f64
        || config.world_radius < i64::MIN as f64
    {
        return invalid(
            "config.world_radius",
            "worldRadius integer projection is not representable",
        );
    }
    require_integer_projection(config, "worldRadius", config.world_radius as i64)?;
    let computed_hash = normalized_config_hash(config)?;
    if identity.config_hash != computed_hash {
        return invalid(
            "identity.config_hash",
            "does not match canonical normalized configuration",
        );
    }
    Ok(())
}

fn validate_generation(generation: &GenerationState) -> Result<(), StateError> {
    if generation.boundary_version != GENERATION_BOUNDARY_VERSION {
        return invalid(
            "generation.boundary_version",
            "unsupported boundary version",
        );
    }
    if generation.generation == 0 || generation.population_epoch == 0 {
        return invalid(
            "generation",
            "generation and population epoch must be positive",
        );
    }
    validate_nonnegative_f64("generation.elapsed_seconds", generation.elapsed_seconds)?;
    validate_nonnegative_f64(
        "generation.wall_accumulator_seconds",
        generation.wall_accumulator_seconds,
    )?;
    if !generation.best_fitness_ever.is_finite() {
        return invalid("generation.best_fitness_ever", "value must be finite");
    }
    Ok(())
}

fn validate_fixed_step_continuation(candidate: &StateCandidate) -> Result<(), StateError> {
    let continuation = &candidate.fixed_step;
    validate_nonnegative_f64(
        "fixed_step.ambient_pellet_accumulator",
        continuation.ambient_pellet_accumulator,
    )?;
    let generation_best = continuation.sensor_generation.best_points_this_generation();
    if !generation_best.is_finite() || generation_best < 0.0 {
        return invalid(
            "fixed_step.sensor_generation",
            "generation best points must be finite and nonnegative",
        );
    }
    let maximum_alive_evolved_points = candidate
        .world
        .snakes
        .iter()
        .filter(|snake| snake.alive && snake.kind == SnakeKind::Evolved)
        .fold(0.0_f64, |maximum, snake| maximum.max(snake.points));
    if generation_best < maximum_alive_evolved_points {
        return invalid(
            "fixed_step.sensor_generation",
            "generation best points cannot trail a live evolved snake",
        );
    }
    continuation
        .baseline_lifecycle
        .validate_authoritative(
            &candidate.world,
            candidate.config.baseline_count,
            matches!(candidate.phase, AuthorityPhase::GenerationBoundary(_)),
        )
        .map_err(|error| StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            reason: error.to_string(),
        })?;
    Ok(())
}

fn validate_rng_bundle(
    bundle: &RngStateBundle,
    config: &NormalizedEngineConfig,
) -> Result<(), StateError> {
    if bundle.version != RNG_BUNDLE_VERSION {
        return invalid("rng.version", "unsupported RNG-bundle version");
    }
    if bundle.baselines.len() != config.baseline_count {
        return invalid(
            "rng.baselines",
            "count does not match normalized baseline configuration",
        );
    }
    validate_rng("world", &bundle.world)?;
    validate_rng("evolution", &bundle.evolution)?;
    validate_rng("external-controller", &bundle.external_controller)?;
    let mut expected_slot = 0u32;
    for baseline in &bundle.baselines {
        if baseline.slot != expected_slot {
            return invalid("rng.baselines", "baseline slots must be dense and ordered");
        }
        validate_rng(&format!("baseline:{}", baseline.slot), &baseline.state)?;
        expected_slot = expected_slot
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "baseline RNG slot",
            })?;
    }
    Ok(())
}

fn validate_rng(stream: &str, state: &SerializedRngState) -> Result<(), StateError> {
    StatefulRng::from_state(state)
        .map(|_| ())
        .map_err(|error: RngError| StateError::InvalidRng {
            stream: stream.to_owned(),
            reason: error.to_string(),
        })
}

fn validate_allocators(allocators: &AllocatorState) -> Result<(), StateError> {
    if allocators.version != ALLOCATOR_VERSION
        || allocators.next_entity_id == 0
        || allocators.next_brain_id == 0
        || allocators.next_genome_id == 0
        || allocators.next_controller_lease_id == 0
        || allocators.next_external_id == 0
        || allocators.next_baseline_id == 0
        || allocators.next_resurrected_id == 0
    {
        return invalid(
            "allocators",
            "version is unsupported or a next identity is zero",
        );
    }
    validate_next_domain(
        "allocators.next_entity_id",
        allocators.next_entity_id,
        1,
        EXTERNAL_ENTITY_ID_START,
    )?;
    validate_next_domain(
        "allocators.next_external_id",
        allocators.next_external_id,
        EXTERNAL_ENTITY_ID_START,
        BASELINE_ENTITY_ID_START,
    )?;
    validate_next_domain(
        "allocators.next_baseline_id",
        allocators.next_baseline_id,
        BASELINE_ENTITY_ID_START,
        RESURRECTED_ENTITY_ID_START,
    )?;
    validate_next_domain(
        "allocators.next_resurrected_id",
        allocators.next_resurrected_id,
        RESURRECTED_ENTITY_ID_START,
        RESURRECTED_ENTITY_ID_EXHAUSTED,
    )?;
    if allocators.next_frame_v1_id == 0 || allocators.next_frame_v1_id > FRAME_V1_EXHAUSTED_ID {
        return invalid(
            "allocators.next_frame_v1_id",
            "outside frame-v1 exact-ID range",
        );
    }
    Ok(())
}

fn validate_population(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    if candidate.population.len() != candidate.config.population_count {
        return invalid(
            "population",
            "count does not match normalized configuration",
        );
    }
    let max_population = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
    if candidate.population.len() > max_population {
        return invalid("population", "count exceeds stable u32 slot space");
    }

    let mut genome_ids = BTreeSet::new();
    let mut population_handles = BTreeSet::new();
    let mut max_genome_id = 0u64;
    for (index, genome) in candidate.population.iter().enumerate() {
        let expected_slot = u32::try_from(index).map_err(|_| StateError::ArithmeticOverflow {
            context: "population slot",
        })?;
        if genome.slot != expected_slot {
            return Err(StateError::NonDensePopulationSlot {
                index,
                slot: genome.slot,
            });
        }
        validate_handle(genome.brain, candidate.generation.population_epoch)?;
        if !population_handles.insert(genome.brain) {
            return Err(StateError::DuplicateBrainHandle(genome.brain));
        }
        if genome.lineage.genome_id == 0
            || genome.lineage.genome_id == u64::MAX
            || !genome_ids.insert(genome.lineage.genome_id)
        {
            return Err(StateError::DuplicateId {
                kind: "genome",
                id: genome.lineage.genome_id,
            });
        }
        max_genome_id = max_genome_id.max(genome.lineage.genome_id);
        for parent in [genome.lineage.parent_a, genome.lineage.parent_b]
            .into_iter()
            .flatten()
        {
            if parent == 0 || parent >= candidate.allocators.next_genome_id {
                return invalid(
                    "population.lineage.parent",
                    "parent identity must be nonzero and precede the next genome identity",
                );
            }
        }
        if genome.lineage.birth_generation == 0
            || genome.lineage.birth_generation > candidate.generation.generation
        {
            return invalid("population.lineage", "invalid birth generation");
        }
        if !genome.fitness.is_finite() {
            return Err(StateError::NonFinite {
                field: "population.fitness",
                index,
            });
        }
        if genome.weights.len() != graph.total_parameters {
            return Err(StateError::WeightLength {
                slot: genome.slot,
                expected: graph.total_parameters,
                actual: genome.weights.len(),
            });
        }
        validate_f32_slice("population.weights", index, &genome.weights)?;
    }

    let max_brains = candidate
        .population
        .len()
        .checked_add(candidate.config.max_non_population_brains)
        .ok_or(StateError::ArithmeticOverflow {
            context: "brain capacity",
        })?;
    if candidate.brains.len() > max_brains {
        return invalid("brains", "count exceeds configured capacity");
    }
    let mut all_handles = BTreeSet::new();
    for (index, brain) in candidate.brains.iter().enumerate() {
        validate_handle(brain.handle, candidate.generation.population_epoch)?;
        if !all_handles.insert(brain.handle) {
            return Err(StateError::DuplicateBrainHandle(brain.handle));
        }
        if brain.recurrent.len() != graph.total_state_size {
            return Err(StateError::RecurrentLength {
                handle: brain.handle,
                expected: graph.total_state_size,
                actual: brain.recurrent.len(),
            });
        }
        validate_f32_slice("brains.recurrent", index, &brain.recurrent)?;
        match brain.owner {
            BrainOwner::PopulationSlot(slot) => {
                if brain.non_population_weights.is_some() {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                }
                let genome = candidate.population.get(slot as usize);
                if genome.is_none_or(|genome| genome.brain != brain.handle) {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                }
            }
            BrainOwner::Entity(entity_id) => {
                let Some(weights) = &brain.non_population_weights else {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                };
                if entity_id == 0 {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                }
                if weights.len() != graph.total_parameters {
                    return Err(StateError::BrainWeightLength {
                        handle: brain.handle,
                        expected: graph.total_parameters,
                        actual: weights.len(),
                    });
                }
                validate_f32_slice("brains.non_population_weights", index, weights)?;
            }
        }
    }
    for genome in &candidate.population {
        if !all_handles.contains(&genome.brain) {
            return Err(StateError::InvalidBrainOwner(genome.brain));
        }
    }
    if candidate.allocators.next_genome_id != u64::MAX
        && candidate.allocators.next_genome_id <= max_genome_id
    {
        return invalid(
            "allocators.next_genome_id",
            "does not follow existing genome identities",
        );
    }
    Ok(())
}

fn validate_handle(handle: BrainHandle, expected_epoch: u64) -> Result<(), StateError> {
    if handle.id == 0
        || handle.id == u64::MAX
        || handle.epoch == 0
        || handle.epoch != expected_epoch
    {
        return Err(StateError::InvalidBrainOwner(handle));
    }
    Ok(())
}

fn validate_world(candidate: &StateCandidate) -> Result<(), StateError> {
    if candidate.world.body_points.len() > candidate.config.max_body_points
        || candidate.world.pellets.len() > candidate.config.max_pellets
        || candidate.world.snakes.len() > candidate.config.max_world_snakes
        || candidate.world.controller_leases.len() > candidate.config.max_world_snakes
    {
        return invalid("world.capacity", "configured world capacity exceeded");
    }
    for (index, point) in candidate.world.body_points.iter().enumerate() {
        validate_point("world.body_points", index, *point)?;
    }

    let mut entity_ids = BTreeSet::new();
    let mut snake_ids = BTreeSet::new();
    let mut public_ids = BTreeSet::new();
    let mut evolved_slots = BTreeSet::new();
    let mut world_brains = BTreeSet::new();
    let mut baseline_slots = BTreeSet::new();
    let mut ranges = Vec::with_capacity(candidate.world.snakes.len());
    let mut max_general_id = 0u64;
    let mut max_external_id = 0u64;
    let mut max_baseline_id = 0u64;
    let mut max_resurrected_id = 0u64;
    let mut max_public_id = 0u32;
    for (index, snake) in candidate.world.snakes.iter().enumerate() {
        if snake.id == 0 || snake.id == u64::MAX || !entity_ids.insert(snake.id) {
            return Err(StateError::DuplicateId {
                kind: "snake",
                id: snake.id,
            });
        }
        snake_ids.insert(snake.id);
        if snake.frame_v1_id == 0
            || snake.frame_v1_id > FRAME_V1_MAX_EXACT_ID
            || !public_ids.insert(snake.frame_v1_id)
        {
            return Err(StateError::DuplicateId {
                kind: "frame-v1 snake",
                id: u64::from(snake.frame_v1_id),
            });
        }
        validate_snake(index, snake, candidate)?;
        match snake.kind {
            SnakeKind::Evolved => {
                validate_entity_id_domain("evolved snake", snake.id, 1, EXTERNAL_ENTITY_ID_START)?;
                max_general_id = max_general_id.max(snake.id);
                let Some(slot) = snake.population_slot else {
                    return invalid(
                        "world.snakes.population_slot",
                        "evolved snake requires a population slot",
                    );
                };
                if !evolved_slots.insert(slot) {
                    return Err(StateError::DuplicateId {
                        kind: "world population slot",
                        id: u64::from(slot),
                    });
                }
            }
            SnakeKind::External => {
                validate_entity_id_domain(
                    "external snake",
                    snake.id,
                    EXTERNAL_ENTITY_ID_START,
                    BASELINE_ENTITY_ID_START,
                )?;
                max_external_id = max_external_id.max(snake.id);
            }
            SnakeKind::Baseline => {
                validate_entity_id_domain(
                    "baseline snake",
                    snake.id,
                    BASELINE_ENTITY_ID_START,
                    RESURRECTED_ENTITY_ID_START,
                )?;
                max_baseline_id = max_baseline_id.max(snake.id);
            }
            SnakeKind::Resurrected => {
                validate_entity_id_domain(
                    "resurrected snake",
                    snake.id,
                    RESURRECTED_ENTITY_ID_START,
                    RESURRECTED_ENTITY_ID_EXHAUSTED,
                )?;
                max_resurrected_id = max_resurrected_id.max(snake.id);
            }
        }
        max_public_id = max_public_id.max(snake.frame_v1_id);
        if let Some(handle) = snake.brain {
            if !world_brains.insert(handle) {
                return Err(StateError::DuplicateBrainHandle(handle));
            }
        }
        if let Some(slot) = snake.baseline_slot {
            if !baseline_slots.insert(slot)
                || candidate
                    .rng
                    .baselines
                    .get(slot as usize)
                    .is_none_or(|baseline| baseline.slot != slot)
            {
                return invalid(
                    "world.snakes.baseline_slot",
                    "baseline slot is duplicate or has no matching RNG stream",
                );
            }
        }
        let end = snake
            .body
            .start
            .checked_add(snake.body.len)
            .ok_or(StateError::InvalidBodyRange { snake_id: snake.id })?;
        if end > candidate.world.body_points.len() || (snake.alive && snake.body.len == 0) {
            return Err(StateError::InvalidBodyRange { snake_id: snake.id });
        }
        if snake.body.len != 0 {
            if candidate.world.body_points[snake.body.start] != snake.position {
                return Err(StateError::InvalidBodyRange { snake_id: snake.id });
            }
            ranges.push((snake.body.start, end, snake.id));
        }
    }

    for brain in &candidate.brains {
        if let BrainOwner::Entity(entity_id) = brain.owner {
            let matched = candidate.world.snakes.iter().any(|snake| {
                snake.id == entity_id
                    && snake.brain == Some(brain.handle)
                    && snake.population_slot.is_none()
            });
            if !matched {
                return Err(StateError::InvalidBrainOwner(brain.handle));
            }
        }
    }
    ranges.sort_unstable_by_key(|range| range.0);
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(StateError::InvalidBodyRange {
                snake_id: pair[1].2,
            });
        }
    }

    for (index, pellet) in candidate.world.pellets.iter().enumerate() {
        if pellet.id == 0 || pellet.id >= EXTERNAL_ENTITY_ID_START || !entity_ids.insert(pellet.id)
        {
            return Err(StateError::DuplicateId {
                kind: "world entity",
                id: pellet.id,
            });
        }
        max_general_id = max_general_id.max(pellet.id);
        validate_point("world.pellets.position", index, pellet.position)?;
        if !pellet.value.is_finite() || pellet.value <= 0.0 {
            return Err(StateError::NonFinite {
                field: "world.pellets.value",
                index,
            });
        }
        if pellet
            .owner
            .is_some_and(|owner| !snake_ids.contains(&owner))
        {
            return invalid("world.pellets.owner", "owner does not identify a snake");
        }
    }

    validate_next_after(
        "allocators.next_entity_id",
        candidate.allocators.next_entity_id,
        max_general_id,
        EXTERNAL_ENTITY_ID_START,
    )?;
    validate_next_after(
        "allocators.next_external_id",
        candidate.allocators.next_external_id,
        max_external_id,
        BASELINE_ENTITY_ID_START,
    )?;
    validate_next_after(
        "allocators.next_baseline_id",
        candidate.allocators.next_baseline_id,
        max_baseline_id,
        RESURRECTED_ENTITY_ID_START,
    )?;
    validate_next_after(
        "allocators.next_resurrected_id",
        candidate.allocators.next_resurrected_id,
        max_resurrected_id,
        RESURRECTED_ENTITY_ID_EXHAUSTED,
    )?;
    if candidate.allocators.next_frame_v1_id != FRAME_V1_EXHAUSTED_ID
        && candidate.allocators.next_frame_v1_id <= max_public_id
    {
        return invalid(
            "allocators.next_frame_v1_id",
            "does not follow existing public IDs",
        );
    }

    let mut lease_ids = BTreeSet::new();
    let mut lease_snakes = BTreeSet::new();
    let mut resume_tokens = BTreeSet::new();
    let mut connection_ids = BTreeSet::new();
    for lease in &candidate.world.controller_leases {
        let snake = candidate
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == lease.snake_id && snake.alive)
            .ok_or(StateError::UnknownLeaseSnake(lease.snake_id))?;
        validate_lease(lease, snake, candidate)?;
        if !lease_ids.insert(lease.id) {
            return Err(StateError::DuplicateLeaseId(lease.id));
        }
        if !lease_snakes.insert(lease.snake_id) {
            return Err(StateError::DuplicateLeaseSnake(lease.snake_id));
        }
        if !resume_tokens.insert(lease.resume_token.as_str()) {
            return invalid("controller_lease.resume_token", "token must be unique");
        }
        if let Some(connection_id) = lease.connection_id {
            if !connection_ids.insert(connection_id) {
                return invalid(
                    "controller_lease.connection_id",
                    "connection must own at most one lease",
                );
            }
        }
    }
    validate_next_after(
        "allocators.next_controller_lease_id",
        candidate.allocators.next_controller_lease_id,
        candidate
            .world
            .controller_leases
            .iter()
            .map(|lease| lease.id)
            .max()
            .unwrap_or(0),
        u64::MAX,
    )?;
    validate_next_after(
        "allocators.next_brain_id",
        candidate.allocators.next_brain_id,
        candidate
            .brains
            .iter()
            .map(|brain| brain.handle.id)
            .max()
            .unwrap_or(0),
        u64::MAX,
    )?;
    Ok(())
}

fn validate_snake(
    index: usize,
    snake: &SnakeState,
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    validate_point("world.snakes.position", index, snake.position)?;
    validate_point(
        "world.snakes.previous_position",
        index,
        snake.previous_position,
    )?;
    for (field, value) in [
        ("world.snakes.direction", snake.direction),
        ("world.snakes.radius", snake.radius),
        ("world.snakes.speed", snake.speed),
        ("world.snakes.age_seconds", snake.age_seconds),
        ("world.snakes.food", snake.food),
        ("world.snakes.points", snake.points),
        ("world.snakes.target_length", snake.target_length),
        ("world.snakes.fitness", snake.fitness),
        (
            "world.snakes.control_accumulator_seconds",
            snake.control_accumulator_seconds,
        ),
        (
            "world.snakes.delivered_observation_points",
            snake.delivered_observation_points,
        ),
    ] {
        if !value.is_finite() {
            return Err(StateError::NonFinite { field, index });
        }
    }
    if snake.radius <= 0.0
        || snake.speed < 0.0
        || snake.age_seconds < 0.0
        || snake.target_length < 0.0
        || snake.control_accumulator_seconds < 0.0
        || !snake.turn.is_finite()
        || !snake.previous_turn.is_finite()
        || !(-1.0..=1.0).contains(&snake.turn)
        || !(-1.0..=1.0).contains(&snake.previous_turn)
    {
        return invalid("world.snakes", "invalid scalar range");
    }
    match snake.kind {
        SnakeKind::Evolved => {
            let slot = snake
                .population_slot
                .ok_or_else(|| StateError::InvalidField {
                    field: "world.snakes.population_slot",
                    reason: "evolved snake requires a population slot".to_owned(),
                })?;
            let genome = candidate.population.get(slot as usize);
            if genome.is_none_or(|genome| Some(genome.brain) != snake.brain) {
                return invalid(
                    "world.snakes.brain",
                    "evolved snake mapping is inconsistent",
                );
            }
            if snake.baseline_slot.is_some() || snake.baseline_strategy.is_some() {
                return invalid(
                    "world.snakes.baseline",
                    "evolved snake cannot own baseline state",
                );
            }
        }
        SnakeKind::Baseline => {
            if snake.population_slot.is_some()
                || snake.baseline_slot.is_none()
                || snake.baseline_strategy.is_none()
            {
                return invalid(
                    "world.snakes.baseline",
                    "baseline snake requires only a stable slot and strategy",
                );
            }
        }
        _ if snake.population_slot.is_some()
            || snake.baseline_slot.is_some()
            || snake.baseline_strategy.is_some() =>
        {
            return invalid(
                "world.snakes.owner_slot",
                "population and baseline slots must match the snake kind",
            );
        }
        _ => {}
    }
    if let Some(handle) = snake.brain {
        let found = candidate
            .brains
            .iter()
            .any(|brain| brain.handle == handle && brain.owner == BrainOwner::Entity(snake.id));
        let evolved_mapping = snake.kind == SnakeKind::Evolved
            && candidate.brains.iter().any(|brain| {
                brain.handle == handle && matches!(brain.owner, BrainOwner::PopulationSlot(_))
            });
        if !found && !evolved_mapping {
            return Err(StateError::InvalidBrainOwner(handle));
        }
    }
    Ok(())
}

fn validate_lease(
    lease: &ControllerLease,
    snake: &SnakeState,
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    if lease.id == 0 || lease.id == u64::MAX || lease.snake_id == 0 {
        return invalid(
            "controller_lease",
            "lease and snake identities must be positive",
        );
    }
    if snake.kind != SnakeKind::External {
        return invalid(
            "controller_lease.snake_id",
            "controller leases may target only external snakes",
        );
    }
    validate_text("controller_lease.scope", &lease.scope)?;
    validate_text("controller_lease.resume_token", &lease.resume_token)?;
    if lease.scope != candidate.identity.run_id {
        return invalid(
            "controller_lease.scope",
            "lease is not scoped to the current run",
        );
    }
    if lease.connection_id == Some(0) {
        return invalid(
            "controller_lease.connection_id",
            "connection identity must be nonzero",
        );
    }
    if !lease.latest_action.turn.is_finite() || !(-1.0..=1.0).contains(&lease.latest_action.turn) {
        return invalid(
            "controller_lease.latest_action.turn",
            "turn must be in [-1, 1]",
        );
    }
    if lease.latest_action.arrival_sequence == 0 {
        return invalid(
            "controller_lease.latest_action.arrival_sequence",
            "arrival sequence must be positive",
        );
    }
    if lease.last_observed_at_ms < lease.latest_action.accepted_at_ms {
        return invalid(
            "controller_lease.last_observed_at_ms",
            "last observed wall time precedes the latest action",
        );
    }
    match lease.status {
        ControllerLeaseStatus::Connected => {
            if lease.connection_id.is_none()
                || lease.disconnected_at_ms.is_some()
                || lease.input_hold_expires_at_ms.is_some()
                || lease.grace_expires_at_ms.is_some()
                || lease.takeover_committed_at_ms.is_some()
            {
                return invalid(
                    "controller_lease",
                    "connected lease requires only a live connection",
                );
            }
        }
        ControllerLeaseStatus::HoldingLastInput => {
            let (Some(disconnected), Some(hold_deadline), Some(grace_deadline)) = (
                lease.disconnected_at_ms,
                lease.input_hold_expires_at_ms,
                lease.grace_expires_at_ms,
            ) else {
                return invalid(
                    "controller_lease",
                    "held-input lease requires disconnect, hold, and grace deadlines",
                );
            };
            let expected_hold_deadline = lease
                .latest_action
                .accepted_at_ms
                .checked_add(candidate.config.controller_input_hold_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller input-hold deadline",
                })?;
            let expected_grace_deadline = disconnected
                .checked_add(candidate.config.controller_disconnect_grace_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller grace deadline",
                })?;
            if lease.connection_id.is_some()
                || lease.takeover_committed_at_ms.is_some()
                || hold_deadline != expected_hold_deadline
                || hold_deadline <= disconnected
                || grace_deadline != expected_grace_deadline
                || lease.latest_action.accepted_at_ms > disconnected
                || lease.last_observed_at_ms < disconnected
                || snake.turn.to_bits() != lease.latest_action.turn.to_bits()
                || snake.input_boost != lease.latest_action.boost
            {
                return invalid(
                    "controller_lease",
                    "held-input ownership must be exclusive and preserve the last accepted action",
                );
            }
        }
        ControllerLeaseStatus::ReservedNeutral => {
            let (Some(disconnected), Some(hold_deadline), Some(grace_deadline)) = (
                lease.disconnected_at_ms,
                lease.input_hold_expires_at_ms,
                lease.grace_expires_at_ms,
            ) else {
                return invalid(
                    "controller_lease",
                    "neutral grace requires disconnect, hold, and grace deadlines",
                );
            };
            let expected_hold_deadline = lease
                .latest_action
                .accepted_at_ms
                .checked_add(candidate.config.controller_input_hold_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller input-hold deadline",
                })?;
            let expected_grace_deadline = disconnected
                .checked_add(candidate.config.controller_disconnect_grace_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller grace deadline",
                })?;
            if lease.connection_id.is_some()
                || lease.takeover_committed_at_ms.is_some()
                || hold_deadline != expected_hold_deadline
                || grace_deadline != expected_grace_deadline
                || lease.latest_action.accepted_at_ms > disconnected
                || lease.last_observed_at_ms < disconnected
                || snake.turn != 0.0
                || snake.input_boost
            {
                return invalid(
                    "controller_lease",
                    "neutral grace must be exclusive with neutral steering and boost input",
                );
            }
        }
        ControllerLeaseStatus::NeuralTakeover => {
            let (Some(disconnected), Some(hold_deadline), Some(grace_deadline), Some(takeover)) = (
                lease.disconnected_at_ms,
                lease.input_hold_expires_at_ms,
                lease.grace_expires_at_ms,
                lease.takeover_committed_at_ms,
            ) else {
                return invalid(
                    "controller_lease",
                    "neural takeover requires disconnect, hold, grace, and commit evidence",
                );
            };
            let expected_hold_deadline = lease
                .latest_action
                .accepted_at_ms
                .checked_add(candidate.config.controller_input_hold_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller input-hold deadline",
                })?;
            let expected_grace_deadline = disconnected
                .checked_add(candidate.config.controller_disconnect_grace_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller grace deadline",
                })?;
            if lease.connection_id.is_some()
                || hold_deadline != expected_hold_deadline
                || grace_deadline != expected_grace_deadline
                || takeover < grace_deadline
                || lease.latest_action.accepted_at_ms > disconnected
                || lease.last_observed_at_ms < takeover
                || snake.brain.is_none()
            {
                return invalid(
                    "controller_lease",
                    "neural takeover must occur once after grace and target a neural snake",
                );
            }
        }
    }
    Ok(())
}

fn validate_generation_boundary(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    if matches!(
        candidate.phase,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart)
    ) && (candidate.generation.generation != 1 || candidate.generation.completed_step != 0)
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "run-start boundary must describe generation one before any committed step",
        });
    }
    if matches!(
        candidate.phase,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation)
    ) && (candidate.generation.generation == 1 || candidate.generation.completed_step == 0)
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason:
                "post-generation boundary must follow a completed step in generation two or later",
        });
    }
    if candidate.generation.elapsed_seconds != 0.0
        || candidate.generation.wall_accumulator_seconds != 0.0
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "elapsed time and wall accumulator must be zero",
        });
    }
    if candidate.fixed_step.ambient_pellet_accumulator != 0.0
        || candidate
            .fixed_step
            .sensor_generation
            .best_points_this_generation()
            != 0.0
        || !candidate.fixed_step.baseline_lifecycle.slots.is_empty()
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "fixed-step continuation must be reset before spawn or sensing",
        });
    }
    if !candidate.world.snakes.is_empty()
        || !candidate.world.body_points.is_empty()
        || !candidate.world.pellets.is_empty()
        || !candidate.world.controller_leases.is_empty()
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "pre-spawn boundary cannot contain world entities or controller leases",
        });
    }
    if candidate.brains.len() != candidate.population.len() {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "boundary contains non-population brains or misses population brains",
        });
    }
    if graph.total_state_size != 0
        && candidate
            .brains
            .iter()
            .any(|brain| brain.recurrent.iter().any(|value| value.to_bits() != 0))
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "recurrent state must be zero before spawn, sensing, or inference",
        });
    }
    Ok(())
}

fn validate_f32_slice(
    field: &'static str,
    outer_index: usize,
    values: &[f32],
) -> Result<(), StateError> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(StateError::NonFinite {
            field,
            index: outer_index,
        });
    }
    Ok(())
}

fn validate_point(field: &'static str, index: usize, point: WorldPoint) -> Result<(), StateError> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(StateError::NonFinite { field, index });
    }
    Ok(())
}

fn validate_positive_f64(field: &'static str, value: f64) -> Result<(), StateError> {
    if !value.is_finite() || value <= 0.0 {
        return invalid(field, "value must be finite and positive");
    }
    Ok(())
}

fn validate_nonnegative_f64(field: &'static str, value: f64) -> Result<(), StateError> {
    if !value.is_finite() || value < 0.0 {
        return invalid(field, "value must be finite and non-negative");
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str) -> Result<(), StateError> {
    if value.is_empty() || value.as_bytes().contains(&0) {
        return invalid(field, "text must be non-empty and contain no NUL bytes");
    }
    Ok(())
}

fn validate_sha256(field: &'static str, value: &str) -> Result<(), StateError> {
    let Some(hexadecimal) = value.strip_prefix("sha256:") else {
        return invalid(field, "must use sha256:<64 lowercase hexadecimal digits>");
    };
    if hexadecimal.len() != 64
        || !hexadecimal
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid(field, "must use sha256:<64 lowercase hexadecimal digits>");
    }
    Ok(())
}

fn validate_sha256_hex(field: &'static str, value: &str) -> Result<(), StateError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid(
            field,
            "must contain 64 lowercase SHA-256 hexadecimal digits",
        );
    }
    Ok(())
}

fn validate_build_class(field: &'static str, value: &str) -> Result<(), StateError> {
    if !matches!(value, "production" | "test-hooks") {
        return invalid(field, "must be production or test-hooks");
    }
    Ok(())
}

fn invalid<T>(field: &'static str, reason: &str) -> Result<T, StateError> {
    Err(StateError::InvalidField {
        field,
        reason: reason.to_owned(),
    })
}

fn checked_add(left: usize, right: usize, context: &'static str) -> Result<usize, StateError> {
    left.checked_add(right)
        .ok_or(StateError::ArithmeticOverflow { context })
}

fn checked_sum(values: &[usize]) -> Result<usize, StateError> {
    values.iter().try_fold(0usize, |total, value| {
        checked_add(total, *value, "state memory total")
    })
}

fn estimate_frame_v1_bytes(config: &NormalizedEngineConfig) -> Result<usize, StateError> {
    let snake_headers =
        checked_allocation_bytes(config.max_world_snakes, 8, "frame snake headers")?;
    let body_coordinates =
        checked_allocation_bytes(config.max_body_points, 2, "frame body coordinates")?;
    let pellet_fields = checked_allocation_bytes(config.max_pellets, 5, "frame pellet fields")?;
    let float_count = checked_sum(&[7, snake_headers, body_coordinates, 1, pellet_fields])?;
    checked_allocation_bytes(float_count, size_of::<f32>(), "frame-v1 bytes")
}

fn estimate_validation_memory(candidate: &StateCandidate) -> Result<usize, StateError> {
    // A conservative BTree allocation allowance. Exact allocator overhead is
    // implementation-specific, so admission uses a deliberately padded node.
    const TREE_NODE_ALLOWANCE: usize = 96;
    let population_nodes =
        candidate
            .config
            .population_count
            .checked_mul(2)
            .ok_or(StateError::ArithmeticOverflow {
                context: "population validation nodes",
            })?;
    let brain_nodes = candidate
        .config
        .population_count
        .checked_add(candidate.config.max_non_population_brains)
        .ok_or(StateError::ArithmeticOverflow {
            context: "brain validation nodes",
        })?;
    let snake_nodes = candidate.config.max_world_snakes.checked_mul(10).ok_or(
        StateError::ArithmeticOverflow {
            context: "world validation nodes",
        },
    )?;
    let nodes = checked_sum(&[
        population_nodes,
        brain_nodes,
        snake_nodes,
        candidate.config.max_pellets,
        candidate.config.baseline_count,
    ])?;
    let trees = checked_allocation_bytes(nodes, TREE_NODE_ALLOWANCE, "validation trees")?;
    let ranges = checked_allocation_bytes(
        candidate.config.max_world_snakes,
        size_of::<(usize, usize, u64)>(),
        "validation body ranges",
    )?;
    checked_add(trees, ranges, "validation scratch")
}

fn validate_next_domain(
    field: &'static str,
    next: u64,
    start: u64,
    exhausted: u64,
) -> Result<(), StateError> {
    if next < start || next > exhausted {
        return invalid(field, "outside its checked allocator domain");
    }
    Ok(())
}

fn validate_entity_id_domain(
    kind: &'static str,
    id: u64,
    start: u64,
    exhausted: u64,
) -> Result<(), StateError> {
    if id < start || id >= exhausted {
        return invalid(kind, "identity is outside its checked domain");
    }
    Ok(())
}

fn validate_next_after(
    field: &'static str,
    next: u64,
    maximum_existing: u64,
    exhausted: u64,
) -> Result<(), StateError> {
    if next != exhausted && next <= maximum_existing {
        return invalid(field, "does not follow existing identities");
    }
    Ok(())
}

fn add_structural<T>(estimate: &mut StateMemoryEstimate, count: usize) -> Result<(), StateError> {
    estimate.structural_bytes = checked_add(
        estimate.structural_bytes,
        checked_allocation_bytes(count, size_of::<T>(), "state vector storage")?,
        "structural memory",
    )?;
    Ok(())
}

fn add_text(estimate: &mut StateMemoryEstimate, value: &String) -> Result<(), StateError> {
    estimate.text_bytes = checked_add(estimate.text_bytes, value.capacity(), "text memory")?;
    Ok(())
}

fn add_rng_text(
    estimate: &mut StateMemoryEstimate,
    state: &SerializedRngState,
) -> Result<(), StateError> {
    add_text(estimate, &state.algorithm)?;
    add_text(estimate, &state.state_hex)?;
    add_text(estimate, &state.gaussian_algorithm)?;
    if let Some(spare) = &state.gaussian_spare_hex {
        add_text(estimate, spare)?;
    }
    Ok(())
}

fn add_candidate_text(
    estimate: &mut StateMemoryEstimate,
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    add_text(estimate, &candidate.identity.run_id)?;
    add_text(estimate, &candidate.identity.config_hash)?;
    add_text(estimate, &candidate.identity.source_revision)?;
    add_text(estimate, &candidate.identity.engine_build_id)?;
    add_text(estimate, &candidate.identity.source_sha256)?;
    add_text(estimate, &candidate.identity.target_triple)?;
    add_text(estimate, &candidate.identity.build_profile)?;
    add_text(estimate, &candidate.identity.build_class)?;
    add_text(estimate, &candidate.identity.rustc_version)?;
    add_text(estimate, &candidate.identity.build_contract_sha256)?;
    add_text(estimate, &candidate.identity.math_backend)?;
    for setting in &candidate.config.settings {
        add_text(estimate, &setting.path)?;
        if let NormalizedSettingValue::Text(value) = &setting.value {
            add_text(estimate, value)?;
        }
    }
    add_text(estimate, &candidate.config.settings_schema_sha256)?;
    add_text(estimate, &candidate.config.graph_architecture_key)?;
    add_rng_text(estimate, &candidate.rng.world)?;
    add_rng_text(estimate, &candidate.rng.evolution)?;
    add_rng_text(estimate, &candidate.rng.external_controller)?;
    for baseline in &candidate.rng.baselines {
        add_rng_text(estimate, &baseline.state)?;
    }
    for lease in &candidate.world.controller_leases {
        add_text(estimate, &lease.scope)?;
        add_text(estimate, &lease.resume_token)?;
    }
    Ok(())
}

fn estimate_graph_memory(bundle: &GraphBundle) -> Result<usize, StateError> {
    let spec = bundle.spec();
    let graph = bundle.compiled();
    // The bundle stores both structs inline. Charge each source-spec dynamic
    // allocation as well as the compiled metadata previously retained alone.
    let mut bytes = checked_add(
        size_of::<GraphBundle>(),
        ARC_COUNTER_BYTES,
        "graph Arc control words",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            spec.nodes.capacity(),
            size_of::<GraphNodeSpec>(),
            "source graph nodes",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            spec.edges.capacity(),
            size_of::<GraphEdge>(),
            "source graph edges",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            spec.outputs.capacity(),
            size_of::<GraphOutputRef>(),
            "source graph outputs",
        )?,
        "graph memory",
    )?;
    for node in &spec.nodes {
        bytes = checked_add(bytes, node.id.capacity(), "graph memory")?;
        match &node.kind {
            GraphNodeKind::Mlp { hidden_sizes, .. } => {
                bytes = checked_add(
                    bytes,
                    checked_allocation_bytes(
                        hidden_sizes.capacity(),
                        size_of::<usize>(),
                        "source graph MLP hidden sizes",
                    )?,
                    "graph memory",
                )?;
            }
            GraphNodeKind::Split { output_sizes } => {
                bytes = checked_add(
                    bytes,
                    checked_allocation_bytes(
                        output_sizes.capacity(),
                        size_of::<usize>(),
                        "source graph Split output sizes",
                    )?,
                    "graph memory",
                )?;
            }
            GraphNodeKind::Input { .. }
            | GraphNodeKind::Dense { .. }
            | GraphNodeKind::Gru { .. }
            | GraphNodeKind::Lstm { .. }
            | GraphNodeKind::Rru { .. }
            | GraphNodeKind::Concat => {}
        }
    }
    for edge in &spec.edges {
        bytes = checked_add(bytes, edge.from.capacity(), "graph memory")?;
        bytes = checked_add(bytes, edge.to.capacity(), "graph memory")?;
    }
    for output in &spec.outputs {
        bytes = checked_add(bytes, output.node_id.capacity(), "graph memory")?;
    }
    bytes = checked_add(bytes, graph.architecture_key.capacity(), "graph memory")?;
    bytes = checked_add(
        bytes,
        graph.canonical_layout_bytes.capacity(),
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            graph.nodes.capacity(),
            size_of::<CompiledNode>(),
            "graph nodes",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(graph.order.capacity(), size_of::<String>(), "graph order")?,
        "graph memory",
    )?;
    for node in &graph.nodes {
        bytes = checked_add(bytes, node.id.capacity(), "graph memory")?;
        bytes = checked_add(
            bytes,
            checked_allocation_bytes(
                node.output_sizes.capacity(),
                size_of::<usize>(),
                "graph ports",
            )?,
            "graph memory",
        )?;
        bytes = checked_add(
            bytes,
            checked_allocation_bytes(
                node.hidden_sizes.capacity(),
                size_of::<usize>(),
                "graph hidden sizes",
            )?,
            "graph memory",
        )?;
        bytes = checked_add(
            bytes,
            checked_allocation_bytes(
                node.inputs.capacity(),
                size_of::<super::graph::CompiledInputRef>(),
                "graph inputs",
            )?,
            "graph memory",
        )?;
        for input in &node.inputs {
            bytes = checked_add(bytes, input.from_id.capacity(), "graph memory")?;
        }
    }
    for id in &graph.order {
        bytes = checked_add(bytes, id.capacity(), "graph memory")?;
    }
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            graph.recurrent_nodes.capacity(),
            size_of::<super::graph::RecurrentNodeInfo>(),
            "graph recurrent metadata",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            graph.outputs.capacity(),
            size_of::<super::graph::GraphOutputRef>(),
            "graph outputs",
        )?,
        "graph memory",
    )?;
    for output in &graph.outputs {
        bytes = checked_add(bytes, output.node_id.capacity(), "graph memory")?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::contract::{EngineInit, InboundLimits, OutputLimits};
    use crate::engine::graph::{
        GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::movement::{MovementConfig, MovementWorkspace};
    use crate::engine::queues::NoopWakeSink;
    use crate::engine::rng::{derive_seed, StatefulRng};
    use crate::engine::runtime::EngineRuntime;

    fn default_graph_spec() -> GraphSpec {
        GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "mlp".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 83,
                        hidden_sizes: vec![64],
                        output_size: 64,
                    },
                },
                GraphNodeSpec {
                    id: "gru".into(),
                    kind: GraphNodeKind::Gru {
                        input_size: 64,
                        hidden_size: 16,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 16,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                GraphEdge {
                    from: "input".into(),
                    to: "mlp".into(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "mlp".into(),
                    to: "gru".into(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "gru".into(),
                    to: "head".into(),
                    from_port: None,
                    to_port: None,
                },
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".into(),
                port: None,
            }],
            output_size: 2,
        }
    }

    fn default_graph_limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 16,
            max_edges: 32,
            max_graph_outputs: 4,
            max_identifier_bytes: 64,
            max_total_referenced_identifier_bytes: 1_024,
            max_tensor_width: 1_024,
            max_mlp_hidden_layers: 8,
            max_split_output_ports: 16,
            max_parameter_floats: 1_000_000,
            max_recurrent_state_floats: 16_384,
            max_canonical_layout_bytes: 65_536,
            max_architecture_key_bytes: 200_000,
        }
    }

    fn default_graph() -> Arc<GraphBundle> {
        Arc::new(
            GraphBundle::compile(default_graph_spec(), &default_graph_limits())
                .expect("default graph must compile"),
        )
    }

    fn runtime_init() -> EngineInit {
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

    fn rng(seed: u32, label: &str) -> SerializedRngState {
        StatefulRng::new(f64::from(derive_seed(f64::from(seed), label))).export_state()
    }

    fn test_settings(count: usize) -> Vec<NormalizedSetting> {
        vec![
            NormalizedSetting {
                path: "baselineBots.count".into(),
                value: NormalizedSettingValue::Integer(0),
            },
            NormalizedSetting {
                path: "brain.sensorVersion".into(),
                value: NormalizedSettingValue::Integer(i64::from(SENSOR_VERSION)),
            },
            NormalizedSetting {
                path: "simSpeed".into(),
                value: NormalizedSettingValue::Float(1.0),
            },
            NormalizedSetting {
                path: "snakeCount".into(),
                value: NormalizedSettingValue::Integer(count as i64),
            },
            NormalizedSetting {
                path: "worldRadius".into(),
                value: NormalizedSettingValue::Integer(2_500),
            },
        ]
    }

    fn policy(memory_ceiling_bytes: usize) -> StateAdmissionPolicy {
        StateAdmissionPolicy {
            memory_ceiling_bytes,
            expected_source_revision: "test-source".into(),
            expected_engine_build_id: "test-engine".into(),
            expected_source_sha256:
                "1111111111111111111111111111111111111111111111111111111111111111".into(),
            expected_target_triple: "x86_64-pc-windows-msvc".into(),
            expected_build_profile: "release".into(),
            expected_build_class: "production".into(),
            expected_rustc_version: "rustc 1.92.0 (test)".into(),
            expected_build_contract_sha256:
                "sha256:3333333333333333333333333333333333333333333333333333333333333333".into(),
            expected_math_backend: "rust-scalar-v1".into(),
            expected_settings_schema_sha256: normalized_settings_schema_hash(&test_settings(1))
                .expect("test settings schema must hash"),
        }
    }

    fn own(
        candidate: StateCandidate,
        graph: Arc<GraphBundle>,
        memory_ceiling_bytes: usize,
    ) -> Result<AuthoritativeState, StateError> {
        AuthoritativeState::validate_and_own(candidate, graph, &policy(memory_ceiling_bytes))
    }

    fn candidate(graph: &CompiledGraph, count: usize) -> StateCandidate {
        let epoch = 1;
        let population = (0..count)
            .map(|slot| PopulationGenome {
                slot: slot as u32,
                brain: BrainHandle {
                    id: slot as u64 + 1,
                    epoch,
                },
                lineage: GenomeLineage {
                    genome_id: 100 + slot as u64,
                    birth_generation: 1,
                    parent_a: None,
                    parent_b: None,
                },
                fitness: slot as f64,
                weights: vec![slot as f32 + 0.25; graph.total_parameters].into_boxed_slice(),
            })
            .collect::<Vec<_>>();
        let brains = population
            .iter()
            .map(|genome| BrainRuntimeState {
                handle: genome.brain,
                owner: BrainOwner::PopulationSlot(genome.slot),
                non_population_weights: None,
                recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
            })
            .collect();
        let settings = test_settings(count);
        let settings_schema_sha256 =
            normalized_settings_schema_hash(&settings).expect("test settings schema must hash");
        let config = NormalizedEngineConfig {
            version: NORMALIZED_CONFIG_VERSION,
            settings,
            settings_schema_sha256,
            graph_architecture_key: graph.architecture_key.clone(),
            fixed_step_seconds: 1.0 / 60.0,
            requested_sim_speed: 1.0,
            world_radius: 2_500.0,
            population_count: count,
            baseline_count: 0,
            max_world_snakes: count + 17,
            max_non_population_brains: 16,
            max_body_points: 100_000,
            max_pellets: 10_000,
            spatial_index_bytes: 2 * 1024 * 1024,
            worker_scratch_bytes: 1024 * 1024,
            checkpoint_scratch_bytes: 1024 * 1024,
            controller_input_hold_ms: 500,
            controller_disconnect_grace_ms: 30_000,
        };
        let config_hash = normalized_config_hash(&config).expect("test config must hash");
        StateCandidate {
            versions: ContractVersions {
                state: ENGINE_STATE_VERSION,
                engine: ENGINE_CONTRACT_VERSION,
                protocol: PROTOCOL_VERSION,
                serializer: SERIALIZER_VERSION,
                sensor: SENSOR_VERSION,
                rng_bundle: RNG_BUNDLE_VERSION,
                checkpoint: CHECKPOINT_VERSION,
                graph_layout: graph.layout_version,
            },
            identity: RunIdentity {
                run_id: "run-test".into(),
                seed: 42,
                config_revision: 1,
                config_hash,
                source_revision: "test-source".into(),
                engine_build_id: "test-engine".into(),
                source_sha256: "1111111111111111111111111111111111111111111111111111111111111111"
                    .into(),
                target_triple: "x86_64-pc-windows-msvc".into(),
                build_profile: "release".into(),
                build_class: "production".into(),
                rustc_version: "rustc 1.92.0 (test)".into(),
                build_contract_sha256:
                    "sha256:3333333333333333333333333333333333333333333333333333333333333333".into(),
                math_backend: "rust-scalar-v1".into(),
            },
            config,
            phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart),
            generation: GenerationState {
                boundary_version: GENERATION_BOUNDARY_VERSION,
                generation: 1,
                completed_step: 0,
                population_epoch: epoch,
                elapsed_seconds: 0.0,
                wall_accumulator_seconds: 0.0,
                best_fitness_ever: 0.0,
            },
            fixed_step: FixedStepContinuationState::generation_boundary(),
            rng: RngStateBundle {
                version: RNG_BUNDLE_VERSION,
                world: rng(42, "world"),
                evolution: rng(42, "evolution"),
                external_controller: rng(42, "external-controller"),
                baselines: Vec::new(),
            },
            allocators: AllocatorState {
                version: ALLOCATOR_VERSION,
                next_entity_id: 1,
                next_brain_id: count as u64 + 1,
                next_genome_id: 100 + count as u64,
                next_controller_lease_id: 1,
                next_frame_v1_id: 1,
                next_external_id: EXTERNAL_ENTITY_ID_START,
                next_baseline_id: BASELINE_ENTITY_ID_START,
                next_resurrected_id: RESURRECTED_ENTITY_ID_START,
            },
            population,
            brains,
            world: WorldState::default(),
        }
    }

    fn refresh_config_hash(candidate: &mut StateCandidate) {
        candidate.identity.config_hash =
            normalized_config_hash(&candidate.config).expect("test config must hash");
    }

    fn enable_one_live_baseline(
        candidate: &mut StateCandidate,
        snake_id: u64,
        respawn_remaining_seconds: Option<f64>,
    ) {
        candidate.config.baseline_count = 1;
        candidate
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == "baselineBots.count")
            .expect("test baseline-count setting")
            .value = NormalizedSettingValue::Integer(1);
        candidate.rng.baselines = vec![BaselineRngState {
            slot: 0,
            state: rng(42, "baseline:0"),
        }];
        candidate.fixed_step.baseline_lifecycle = BaselineLifecycleState {
            version: crate::engine::baseline::BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds,
            }],
        };
        refresh_config_hash(candidate);
    }

    fn push_evolved_snake(
        candidate: &mut StateCandidate,
        slot: u32,
        id: u64,
        frame_v1_id: u32,
        position: WorldPoint,
    ) {
        let body_start = candidate.world.body_points.len();
        candidate.world.body_points.push(position);
        candidate.world.snakes.push(SnakeState {
            id,
            frame_v1_id,
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(slot),
            brain: Some(candidate.population[slot as usize].brain),
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: 0.0,
            radius: 8.0,
            speed: 100.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: body_start,
                len: 1,
            },
            skin: 0,
        });
        candidate.allocators.next_entity_id = candidate.allocators.next_entity_id.max(id + 1);
        candidate.allocators.next_frame_v1_id =
            candidate.allocators.next_frame_v1_id.max(frame_v1_id + 1);
    }

    fn push_external_snake(
        candidate: &mut StateCandidate,
        graph: &CompiledGraph,
        id: u64,
        frame_v1_id: u32,
        position: WorldPoint,
    ) {
        let handle = BrainHandle {
            id: candidate.allocators.next_brain_id,
            epoch: candidate.generation.population_epoch,
        };
        candidate.allocators.next_brain_id += 1;
        candidate.brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::Entity(id),
            non_population_weights: Some(vec![0.0; graph.total_parameters].into_boxed_slice()),
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        });
        let body_start = candidate.world.body_points.len();
        candidate.world.body_points.push(position);
        candidate.world.snakes.push(SnakeState {
            id,
            frame_v1_id,
            kind: SnakeKind::External,
            alive: true,
            population_slot: None,
            brain: Some(handle),
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: 0.0,
            radius: 8.0,
            speed: 100.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: body_start,
                len: 1,
            },
            skin: 0,
        });
        candidate.allocators.next_external_id = candidate.allocators.next_external_id.max(id + 1);
        candidate.allocators.next_frame_v1_id =
            candidate.allocators.next_frame_v1_id.max(frame_v1_id + 1);
    }

    fn connected_lease(id: u64, snake_id: u64, connection_id: u64, token: &str) -> ControllerLease {
        ControllerLease {
            id,
            snake_id,
            kind: ControllerKind::Player,
            connection_id: Some(connection_id),
            scope: "run-test".into(),
            resume_token: token.into(),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                turn: 0.25,
                boost: true,
                client_tick: 0,
                arrival_sequence: 1,
                accepted_at_ms: 100,
            },
            last_observed_at_ms: 100,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        }
    }

    #[test]
    fn default_population_owns_distinct_genomes_and_stable_brain_state() {
        let graph = default_graph();
        let candidate = candidate(&graph, 55);
        let estimate = estimate_state_memory(&candidate, &graph).unwrap();
        let state = own(candidate, Arc::clone(&graph), estimate.total_bytes).unwrap();

        assert_eq!(graph.total_parameters, 13_458);
        assert_eq!(graph.total_state_size, 16);
        assert_eq!(state.state().population.len(), 55);
        assert_ne!(
            state.state().population[0].weights[0],
            state.state().population[1].weights[0]
        );
        assert_eq!(
            state.state().brains[54].handle,
            state.state().population[54].brain
        );
        assert_eq!(state.graph().architecture_key, graph.architecture_key);
        assert_eq!(state.graph_spec(), graph.spec());
        assert_eq!(state.graph_bundle().compiled(), graph.compiled());
        let boundary = state.checkpoint_boundary().unwrap();
        assert_eq!(boundary.graph_spec(), graph.spec());
        assert_eq!(boundary.graph(), graph.compiled());
        assert_eq!(boundary.graph_bundle().compiled(), graph.compiled());
        assert!(
            estimate.structural_bytes >= size_of::<Mutex<AuthoritativeState>>() + ARC_COUNTER_BYTES
        );
        assert!(estimate.graph_bytes >= size_of::<GraphBundle>() + ARC_COUNTER_BYTES);
        assert_eq!(state.memory_estimate(), estimate);
    }

    #[test]
    fn authoritative_runtime_owns_the_validated_state_before_start() {
        let graph = default_graph();
        let state = own(candidate(&graph, 2), graph, 32 * 1024 * 1024).unwrap();
        let expected_run_id = state.state().identity.run_id.clone();
        let expected_generation = state.state().generation.generation;
        let expected_memory_bytes = state.memory_estimate().total_bytes;
        let runtime =
            EngineRuntime::new_authoritative(runtime_init(), state, Arc::new(NoopWakeSink))
                .unwrap();

        assert!(runtime.owns_authoritative_state());
        assert_eq!(
            runtime.authoritative_state_memory_bytes(),
            Some(expected_memory_bytes)
        );
        let retained = runtime
            .authoritative_state_for_test()
            .expect("authoritative runtime retains exactly one validated state");
        assert_eq!(retained.state().identity.run_id, expected_run_id);
        assert_eq!(retained.state().generation.generation, expected_generation);
    }

    #[test]
    fn retained_source_graph_capacity_is_charged_before_state_publication() {
        let compact_graph = default_graph();
        let compact_candidate = candidate(&compact_graph, 1);
        let compact_estimate = estimate_state_memory(&compact_candidate, &compact_graph).unwrap();

        let mut expanded_source = default_graph_spec();
        expanded_source.nodes.reserve(256);
        expanded_source.edges.reserve(512);
        expanded_source.outputs.reserve(128);
        expanded_source.nodes[1].id.reserve(16_384);
        expanded_source.edges[0].from.reserve(8_192);
        if let GraphNodeKind::Mlp { hidden_sizes, .. } = &mut expanded_source.nodes[1].kind {
            hidden_sizes.reserve(2_048);
        } else {
            panic!("default fixture's second node must be an MLP");
        }
        let expanded_graph = Arc::new(
            GraphBundle::compile(expanded_source, &default_graph_limits())
                .expect("expanded source graph must remain valid"),
        );
        assert_eq!(expanded_graph.compiled(), compact_graph.compiled());
        let expanded_candidate = candidate(&expanded_graph, 1);
        let expanded_estimate =
            estimate_state_memory(&expanded_candidate, &expanded_graph).unwrap();

        assert!(expanded_estimate.graph_bytes > compact_estimate.graph_bytes);
        assert!(matches!(
            own(
                expanded_candidate,
                expanded_graph,
                compact_estimate.total_bytes
            ),
            Err(StateError::MemoryCeilingExceeded { .. })
        ));
    }

    #[test]
    fn exact_contract_identity_config_projection_and_hash_are_required() {
        let graph = default_graph();

        let mut wrong_version = candidate(&graph, 2);
        wrong_version.versions.protocol += 1;
        assert!(matches!(
            own(wrong_version, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "versions.protocol",
                ..
            })
        ));

        let mismatches: [fn(&mut StateAdmissionPolicy); 9] = [
            |policy| policy.expected_source_revision = "stale-source".into(),
            |policy| policy.expected_engine_build_id = "stale-engine".into(),
            |policy| {
                policy.expected_source_sha256 =
                    "2222222222222222222222222222222222222222222222222222222222222222".into();
            },
            |policy| policy.expected_target_triple = "x86_64-unknown-linux-gnu".into(),
            |policy| policy.expected_build_profile = "debug".into(),
            |policy| policy.expected_build_class = "test-hooks".into(),
            |policy| policy.expected_rustc_version = "rustc other".into(),
            |policy| {
                policy.expected_build_contract_sha256 =
                    "sha256:4444444444444444444444444444444444444444444444444444444444444444"
                        .into();
            },
            |policy| policy.expected_math_backend = "other-math".into(),
        ];
        for mismatch in mismatches {
            let mut wrong_policy = policy(usize::MAX);
            mismatch(&mut wrong_policy);
            assert!(matches!(
                AuthoritativeState::validate_and_own(
                    candidate(&graph, 2),
                    Arc::clone(&graph),
                    &wrong_policy
                ),
                Err(StateError::InvalidField {
                    field: "identity",
                    ..
                })
            ));
        }

        let mut malformed_source_sha = candidate(&graph, 2);
        malformed_source_sha.identity.source_sha256 = "ABC".into();
        assert!(matches!(
            own(malformed_source_sha, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "identity.source_sha256",
                ..
            })
        ));

        let mut contradictory = candidate(&graph, 2);
        let snake_count = contradictory
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == "snakeCount")
            .unwrap();
        snake_count.value = NormalizedSettingValue::Integer(99);
        refresh_config_hash(&mut contradictory);
        assert!(matches!(
            own(contradictory, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "config.settings",
                ..
            })
        ));

        let mut wrong_hash = candidate(&graph, 2);
        wrong_hash.identity.config_hash = "sha256:wrong".into();
        assert!(matches!(
            own(wrong_hash, graph, usize::MAX),
            Err(StateError::InvalidField {
                field: "identity.config_hash",
                ..
            })
        ));
    }

    #[test]
    fn settings_schema_rejects_omissions_unknown_paths_and_type_changes() {
        let graph = default_graph();
        assert_eq!(
            normalized_settings_schema_hash(&test_settings(1)).unwrap(),
            normalized_settings_schema_hash(&test_settings(99)).unwrap()
        );

        let mut omitted = candidate(&graph, 2);
        omitted.config.settings.remove(0);
        refresh_config_hash(&mut omitted);
        assert!(matches!(
            own(omitted, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "config.settings_schema_sha256",
                ..
            })
        ));

        let mut unknown = candidate(&graph, 2);
        unknown.config.settings.insert(
            0,
            NormalizedSetting {
                path: "aaa.unknown".into(),
                value: NormalizedSettingValue::Bool(false),
            },
        );
        refresh_config_hash(&mut unknown);
        assert!(matches!(
            own(unknown, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "config.settings_schema_sha256",
                ..
            })
        ));

        let mut wrong_kind = candidate(&graph, 2);
        wrong_kind
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == "simSpeed")
            .unwrap()
            .value = NormalizedSettingValue::Integer(1);
        refresh_config_hash(&mut wrong_kind);
        assert!(matches!(
            own(wrong_kind, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "config.settings_schema_sha256",
                ..
            })
        ));

        let exact = candidate(&graph, 2);
        let mut wrong_policy = policy(usize::MAX);
        wrong_policy.expected_settings_schema_sha256 =
            "sha256:2222222222222222222222222222222222222222222222222222222222222222".into();
        assert!(matches!(
            AuthoritativeState::validate_and_own(exact, graph, &wrong_policy),
            Err(StateError::InvalidField {
                field: "config.settings_schema_sha256",
                ..
            })
        ));
    }

    #[test]
    fn requested_capacity_and_scratch_are_charged_before_authority() {
        let graph = default_graph();
        let compact = candidate(&graph, 1);
        let compact_estimate = estimate_state_memory(&compact, &graph).unwrap();

        let mut expanded = candidate(&graph, 1);
        expanded.config.max_body_points += 500_000;
        expanded.config.max_pellets += 50_000;
        expanded.config.spatial_index_bytes += 8 * 1024 * 1024;
        expanded.config.worker_scratch_bytes += 4 * 1024 * 1024;
        refresh_config_hash(&mut expanded);
        let expanded_estimate = estimate_state_memory(&expanded, &graph).unwrap();
        assert!(expanded_estimate.structural_bytes > compact_estimate.structural_bytes);
        assert!(expanded_estimate.frame_bytes > compact_estimate.frame_bytes);
        assert!(expanded_estimate.spatial_bytes > compact_estimate.spatial_bytes);
        assert!(expanded_estimate.scratch_bytes > compact_estimate.scratch_bytes);
        assert!(matches!(
            own(expanded, graph, compact_estimate.total_bytes),
            Err(StateError::MemoryCeilingExceeded { .. })
        ));
    }

    #[test]
    fn checkpoint_view_rejects_running_state_and_frame_reservation_is_atomic() {
        let graph = default_graph();
        let boundary = own(candidate(&graph, 1), Arc::clone(&graph), usize::MAX).unwrap();
        assert_eq!(
            boundary.checkpoint_boundary().unwrap().kind(),
            GenerationBoundaryKind::RunStart
        );

        let mut running = candidate(&graph, 1);
        running.phase = AuthorityPhase::Running;
        let running = own(running, graph, usize::MAX).unwrap();
        assert!(running.checkpoint_boundary().is_err());

        let mut allocators = running.state().allocators.clone();
        allocators.next_frame_v1_id = FRAME_V1_MAX_EXACT_ID - 1;
        assert_eq!(
            allocators.reserve_frame_v1_ids(2).unwrap(),
            Some(FrameV1IdReservation {
                first: FRAME_V1_MAX_EXACT_ID - 1,
                last: FRAME_V1_MAX_EXACT_ID,
            })
        );
        assert_eq!(allocators.next_frame_v1_id, FRAME_V1_EXHAUSTED_ID);
        let before = allocators.clone();
        assert!(matches!(
            allocators.reserve_frame_v1_ids(1),
            Err(StateError::IdExhausted {
                kind: "frame-v1",
                requested: 1
            })
        ));
        assert_eq!(allocators, before);
    }

    #[test]
    fn rejects_invalid_lengths_nonfinite_values_and_duplicate_slots_or_ids() {
        let graph = default_graph();

        let mut bad_slot = candidate(&graph, 2);
        bad_slot.population[1].slot = 0;
        assert!(matches!(
            own(bad_slot, Arc::clone(&graph), usize::MAX),
            Err(StateError::NonDensePopulationSlot { .. })
        ));

        let mut bad_weights = candidate(&graph, 2);
        bad_weights.population[0].weights =
            vec![0.0; graph.total_parameters - 1].into_boxed_slice();
        assert!(matches!(
            own(bad_weights, Arc::clone(&graph), usize::MAX),
            Err(StateError::WeightLength { .. })
        ));

        let mut bad_recurrent = candidate(&graph, 2);
        bad_recurrent.brains[0].recurrent =
            vec![0.0; graph.total_state_size + 1].into_boxed_slice();
        assert!(matches!(
            own(bad_recurrent, Arc::clone(&graph), usize::MAX),
            Err(StateError::RecurrentLength { .. })
        ));

        let mut nonfinite = candidate(&graph, 2);
        nonfinite.population[0].weights[7] = f32::NAN;
        assert!(matches!(
            own(nonfinite, Arc::clone(&graph), usize::MAX),
            Err(StateError::NonFinite {
                field: "population.weights",
                ..
            })
        ));

        let mut duplicate_id = candidate(&graph, 2);
        duplicate_id.population[1].lineage.genome_id = duplicate_id.population[0].lineage.genome_id;
        assert!(matches!(
            own(duplicate_id, Arc::clone(&graph), usize::MAX),
            Err(StateError::DuplicateId { kind: "genome", .. })
        ));

        let mut duplicate_handle = candidate(&graph, 2);
        duplicate_handle.population[1].brain = duplicate_handle.population[0].brain;
        assert!(matches!(
            own(duplicate_handle, graph, usize::MAX),
            Err(StateError::DuplicateBrainHandle(_))
        ));
    }

    #[test]
    fn checked_memory_overflow_and_ceiling_fail_before_authority() {
        assert!(matches!(
            checked_allocation_bytes(usize::MAX, 2, "test overflow"),
            Err(StateError::ArithmeticOverflow {
                context: "test overflow"
            })
        ));

        let graph = default_graph();
        let state_candidate = candidate(&graph, 2);
        let estimate = estimate_state_memory(&state_candidate, &graph).unwrap();
        assert!(matches!(
            own(
                state_candidate,
                Arc::clone(&graph),
                estimate.total_bytes - 1
            ),
            Err(StateError::MemoryCeilingExceeded { .. })
        ));

        let compact = candidate(&graph, 1);
        let compact_estimate = estimate_state_memory(&compact, &graph).unwrap();
        let mut over_reserved = candidate(&graph, 1);
        over_reserved.identity.run_id.reserve(4_096);
        let reserved_estimate = estimate_state_memory(&over_reserved, &graph).unwrap();
        assert!(reserved_estimate.text_bytes >= compact_estimate.text_bytes + 4_096);
    }

    #[test]
    fn validating_connected_controller_state_is_rng_pure() {
        let graph = default_graph();
        let mut live = candidate(&graph, 1);
        live.phase = AuthorityPhase::Running;
        push_external_snake(
            &mut live,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            1,
            WorldPoint { x: 1.0, y: 2.0 },
        );
        live.allocators.next_controller_lease_id = 2;
        live.world.controller_leases.push(connected_lease(
            1,
            EXTERNAL_ENTITY_ID_START,
            7,
            "os-entropy-token",
        ));
        let before = live.rng.clone();
        let estimate = estimate_state_memory(&live, &graph).unwrap();
        let state = own(live, graph, estimate.total_bytes).unwrap();
        assert_eq!(state.state().rng, before);
    }

    #[test]
    fn exact_boundary_requires_zero_state_and_no_spawned_world() {
        let graph = default_graph();
        let clean = candidate(&graph, 2);
        assert!(own(clean, Arc::clone(&graph), usize::MAX).is_ok());

        let mut advanced_run_start = candidate(&graph, 2);
        advanced_run_start.generation.completed_step = 1;
        assert!(matches!(
            own(advanced_run_start, Arc::clone(&graph), usize::MAX),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));

        let mut unadvanced_generation = candidate(&graph, 2);
        unadvanced_generation.phase =
            AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation);
        unadvanced_generation.generation.generation = 2;
        assert!(matches!(
            own(
                unadvanced_generation.clone(),
                Arc::clone(&graph),
                usize::MAX
            ),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));
        unadvanced_generation.generation.completed_step = 1;
        assert!(own(unadvanced_generation, Arc::clone(&graph), usize::MAX).is_ok());

        let mut dirty_recurrent = candidate(&graph, 2);
        dirty_recurrent.brains[0].recurrent[0] = 0.5;
        assert!(matches!(
            own(dirty_recurrent, Arc::clone(&graph), usize::MAX),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));

        let mut dirty_time = candidate(&graph, 2);
        dirty_time.generation.elapsed_seconds = 1.0 / 60.0;
        assert!(matches!(
            own(dirty_time, graph, usize::MAX),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));
    }

    #[test]
    fn fixed_step_continuation_is_owned_live_and_reset_at_checkpoint_boundaries() {
        let graph = default_graph();

        let mut dirty_ambient = candidate(&graph, 1);
        dirty_ambient.fixed_step.ambient_pellet_accumulator = 0.25;
        assert!(matches!(
            own(dirty_ambient, Arc::clone(&graph), usize::MAX),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));

        let mut donor = candidate(&graph, 1);
        donor.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut donor, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
        donor.world.snakes[0].points = 12.5;
        let mut sensor_generation = SensorGenerationState::new();
        sensor_generation
            .update_after_step(&donor.world)
            .expect("finite evolved score should update generation best");
        let mut dirty_sensor = candidate(&graph, 1);
        dirty_sensor.fixed_step.sensor_generation = sensor_generation;
        assert!(matches!(
            own(dirty_sensor, Arc::clone(&graph), usize::MAX),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));

        let mut dirty_baseline = candidate(&graph, 1);
        dirty_baseline
            .fixed_step
            .baseline_lifecycle
            .slots
            .push(BaselineSlotRuntime {
                slot: 0,
                snake_id: BASELINE_ENTITY_ID_START,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: None,
            });
        assert!(matches!(
            own(dirty_baseline, Arc::clone(&graph), usize::MAX),
            Err(StateError::DirtyGenerationBoundary { .. })
        ));

        let mut missing_live_baseline = candidate(&graph, 1);
        missing_live_baseline.phase = AuthorityPhase::Running;
        enable_one_live_baseline(&mut missing_live_baseline, BASELINE_ENTITY_ID_START, None);
        assert!(matches!(
            own(missing_live_baseline, graph, usize::MAX),
            Err(StateError::InvalidField {
                field: "fixed_step.baseline_lifecycle",
                ..
            })
        ));
    }

    #[test]
    fn live_fixed_step_continuation_survives_ownership_and_charges_retained_capacity() {
        let graph = default_graph();
        let mut live = candidate(&graph, 1);
        live.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut live, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
        live.world.snakes[0].points = 12.5;
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "fixed_step.sensor_generation",
                ..
            })
        ));

        let baseline_position = WorldPoint { x: 8.0, y: 9.0 };
        let body_start = live.world.body_points.len();
        live.world.body_points.push(baseline_position);
        live.world.snakes.push(SnakeState {
            id: BASELINE_ENTITY_ID_START,
            frame_v1_id: 2,
            kind: SnakeKind::Baseline,
            alive: true,
            population_slot: None,
            brain: None,
            baseline_slot: Some(0),
            baseline_strategy: Some(BaselineStrategyState::Roam),
            position: baseline_position,
            previous_position: baseline_position,
            direction: 0.0,
            radius: 8.0,
            speed: 100.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: body_start,
                len: 1,
            },
            skin: 0,
        });
        live.allocators.next_baseline_id = BASELINE_ENTITY_ID_START + 1;
        live.allocators.next_frame_v1_id = 3;
        enable_one_live_baseline(&mut live, BASELINE_ENTITY_ID_START, None);
        live.fixed_step.ambient_pellet_accumulator = 0.75;
        live.fixed_step
            .sensor_generation
            .update_after_step(&live.world)
            .expect("finite evolved score should update generation best");

        let expected_continuation = live.fixed_step.clone();
        let compact_estimate = estimate_state_memory(&live, &graph).unwrap();
        let owned = own(
            live.clone(),
            Arc::clone(&graph),
            compact_estimate.total_bytes,
        )
        .expect("complete live continuation should be admitted");
        assert_eq!(owned.state().fixed_step, expected_continuation);
        assert_eq!(
            owned
                .state()
                .fixed_step
                .sensor_generation
                .best_points_this_generation(),
            12.5
        );

        let mut invalid_strategy_timer = live.clone();
        invalid_strategy_timer.fixed_step.baseline_lifecycle.slots[0].strategy_timer_seconds = 0.1;
        assert!(matches!(
            own(invalid_strategy_timer, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "fixed_step.baseline_lifecycle",
                ..
            })
        ));

        let mut invalid_wander = live.clone();
        invalid_wander.fixed_step.baseline_lifecycle.slots[0].wander_angle = 0.31;
        assert!(matches!(
            own(invalid_wander, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "fixed_step.baseline_lifecycle",
                ..
            })
        ));

        let mut mismatched_turn = live.clone();
        mismatched_turn.fixed_step.baseline_lifecycle.slots[0].turn = 0.25;
        assert!(matches!(
            own(mismatched_turn, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "fixed_step.baseline_lifecycle",
                ..
            })
        ));

        let mut mismatched_boost = live.clone();
        mismatched_boost.fixed_step.baseline_lifecycle.slots[0].boost = true;
        assert!(matches!(
            own(mismatched_boost, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "fixed_step.baseline_lifecycle",
                ..
            })
        ));

        let mut dead_action_divergence = live.clone();
        let baseline_index = dead_action_divergence
            .world
            .snakes
            .iter()
            .position(|snake| snake.id == BASELINE_ENTITY_ID_START)
            .expect("test baseline must exist");
        dead_action_divergence.world.snakes[baseline_index].alive = false;
        dead_action_divergence.world.snakes[baseline_index].turn = 0.5;
        dead_action_divergence.world.snakes[baseline_index].input_boost = true;
        dead_action_divergence.fixed_step.baseline_lifecycle.slots[0].respawn_remaining_seconds =
            Some(3.0);
        assert!(own(dead_action_divergence, Arc::clone(&graph), usize::MAX,).is_ok());

        let mut over_reserved = live;
        over_reserved
            .fixed_step
            .baseline_lifecycle
            .slots
            .reserve_exact(32);
        let reserved_estimate = estimate_state_memory(&over_reserved, &graph).unwrap();
        assert!(reserved_estimate.structural_bytes > compact_estimate.structural_bytes);
        assert!(matches!(
            own(over_reserved, graph, compact_estimate.total_bytes),
            Err(StateError::MemoryCeilingExceeded { .. })
        ));
    }

    #[test]
    fn running_world_rejects_duplicate_slots_brains_and_incoherent_bodies() {
        let graph = default_graph();
        let mut duplicate = candidate(&graph, 2);
        duplicate.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut duplicate, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
        push_evolved_snake(&mut duplicate, 0, 2, 2, WorldPoint { x: 3.0, y: 4.0 });
        assert!(matches!(
            own(duplicate, Arc::clone(&graph), usize::MAX),
            Err(StateError::DuplicateId {
                kind: "world population slot",
                ..
            })
        ));

        let mut wrong_head = candidate(&graph, 1);
        wrong_head.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut wrong_head, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
        wrong_head.world.body_points[0].x = 9.0;
        assert!(matches!(
            own(wrong_head, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidBodyRange { .. })
        ));

        let mut empty_alive_body = candidate(&graph, 1);
        empty_alive_body.phase = AuthorityPhase::Running;
        push_evolved_snake(
            &mut empty_alive_body,
            0,
            1,
            1,
            WorldPoint { x: 1.0, y: 2.0 },
        );
        empty_alive_body.world.snakes[0].body.len = 0;
        assert!(matches!(
            own(empty_alive_body, graph, usize::MAX),
            Err(StateError::InvalidBodyRange { .. })
        ));
    }

    #[test]
    fn normalized_wall_death_movement_is_admitted_as_authoritative_state() {
        let graph = default_graph();
        let mut running = candidate(&graph, 1);
        running.phase = AuthorityPhase::Running;
        let start = WorldPoint {
            x: running.config.world_radius - 8.4,
            y: 0.0,
        };
        push_evolved_snake(&mut running, 0, 1, 1, start);

        let mut movement_config = MovementConfig::typescript_defaults();
        movement_config.world_radius = running.config.world_radius;
        let mut workspace = MovementWorkspace::new();
        let prepared = workspace
            .prepare(
                &running.world,
                movement_config,
                1.0 / 180.0,
                running.config.max_body_points,
                running.config.max_pellets,
            )
            .expect("wall movement should prepare");
        assert!(prepared.proposals()[0].wall_death);
        let staged_snakes = prepared.snakes().to_vec();
        let staged_body_points = prepared.body_points().to_vec();
        assert!(!staged_snakes[0].alive);
        assert_eq!(staged_body_points[0], staged_snakes[0].position);

        running.world.snakes = staged_snakes;
        running.world.body_points = staged_body_points;
        assert!(own(running, graph, usize::MAX).is_ok());
    }

    #[test]
    fn allocator_lineage_domains_and_exhaustion_are_checked() {
        let graph = default_graph();
        let mut zero_parent = candidate(&graph, 1);
        zero_parent.population[0].lineage.parent_a = Some(0);
        assert!(matches!(
            own(zero_parent, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "population.lineage.parent",
                ..
            })
        ));

        let mut stale_genome_allocator = candidate(&graph, 1);
        stale_genome_allocator.allocators.next_genome_id =
            stale_genome_allocator.population[0].lineage.genome_id;
        assert!(matches!(
            own(stale_genome_allocator, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "allocators.next_genome_id",
                ..
            })
        ));

        let mut bad_domain = candidate(&graph, 1);
        bad_domain.allocators.next_external_id = 100_000;
        assert!(matches!(
            own(bad_domain, graph, usize::MAX),
            Err(StateError::InvalidField {
                field: "allocators.next_external_id",
                ..
            })
        ));

        let mut allocators = candidate(&default_graph(), 1).allocators;
        allocators.next_genome_id = u64::MAX - 1;
        assert_eq!(
            allocators.reserve_genome_ids(1).unwrap(),
            Some(InternalIdReservation {
                first: u64::MAX - 1,
                last: u64::MAX - 1,
            })
        );
        let exhausted = allocators.clone();
        assert!(matches!(
            allocators.reserve_genome_ids(1),
            Err(StateError::IdExhausted {
                kind: "genome",
                requested: 1
            })
        ));
        assert_eq!(allocators, exhausted);
    }

    #[test]
    fn baseline_slots_require_matching_unique_rng_state() {
        let graph = default_graph();
        let mut live = candidate(&graph, 1);
        live.phase = AuthorityPhase::Running;
        let position = WorldPoint { x: 4.0, y: 5.0 };
        live.world.body_points.push(position);
        live.world.snakes.push(SnakeState {
            id: BASELINE_ENTITY_ID_START,
            frame_v1_id: 1,
            kind: SnakeKind::Baseline,
            alive: true,
            population_slot: None,
            brain: None,
            baseline_slot: Some(0),
            baseline_strategy: Some(BaselineStrategyState::Roam),
            position,
            previous_position: position,
            direction: 0.0,
            radius: 8.0,
            speed: 100.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange { start: 0, len: 1 },
            skin: 0,
        });
        live.allocators.next_baseline_id = BASELINE_ENTITY_ID_START + 1;
        live.allocators.next_frame_v1_id = 2;
        enable_one_live_baseline(&mut live, BASELINE_ENTITY_ID_START, None);
        assert!(own(live.clone(), Arc::clone(&graph), usize::MAX).is_ok());

        live.world.snakes[0].baseline_slot = Some(1);
        assert!(matches!(
            own(live, graph, usize::MAX),
            Err(StateError::InvalidField {
                field: "world.snakes.baseline_slot",
                ..
            })
        ));
    }

    #[test]
    fn controller_lease_status_tokens_connections_and_grace_are_consistent() {
        let graph = default_graph();
        let mut live = candidate(&graph, 2);
        live.phase = AuthorityPhase::Running;
        let first_id = EXTERNAL_ENTITY_ID_START;
        let second_id = first_id + 1;
        push_external_snake(
            &mut live,
            &graph,
            first_id,
            1,
            WorldPoint { x: 1.0, y: 2.0 },
        );
        push_external_snake(
            &mut live,
            &graph,
            second_id,
            2,
            WorldPoint { x: 3.0, y: 4.0 },
        );
        live.world.controller_leases = vec![
            connected_lease(1, first_id, 7, "same-token"),
            connected_lease(2, second_id, 8, "same-token"),
        ];
        live.allocators.next_controller_lease_id = 3;
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease.resume_token",
                ..
            })
        ));

        live.world.controller_leases[1].resume_token = "other-token".into();
        live.world.controller_leases[1].connection_id = Some(7);
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease.connection_id",
                ..
            })
        ));

        live.world.controller_leases.truncate(1);
        live.world.controller_leases[0].scope = "other-run".into();
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease.scope",
                ..
            })
        ));
        live.world.controller_leases[0].scope = "run-test".into();
        let lease = &mut live.world.controller_leases[0];
        lease.connection_id = None;
        lease.status = ControllerLeaseStatus::HoldingLastInput;
        lease.latest_action.accepted_at_ms = 800;
        lease.last_observed_at_ms = 1_000;
        lease.disconnected_at_ms = Some(1_000);
        lease.input_hold_expires_at_ms = Some(1_300);
        lease.grace_expires_at_ms = Some(31_000);
        live.world.snakes[0].turn = lease.latest_action.turn;
        live.world.snakes[0].input_boost = lease.latest_action.boost;
        assert!(own(live.clone(), Arc::clone(&graph), usize::MAX).is_ok());

        live.world.snakes[0].turn = 0.5;
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease",
                ..
            })
        ));
        live.world.snakes[0].turn = 0.0;
        live.world.snakes[0].input_boost = false;

        live.world.controller_leases[0].latest_action.accepted_at_ms = 100;
        live.world.controller_leases[0].input_hold_expires_at_ms = Some(600);
        live.world.snakes[0].turn = live.world.controller_leases[0].latest_action.turn;
        live.world.snakes[0].input_boost = live.world.controller_leases[0].latest_action.boost;
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease",
                ..
            })
        ));
        live.world.snakes[0].turn = 0.0;
        live.world.snakes[0].input_boost = false;
        live.world.controller_leases[0].status = ControllerLeaseStatus::ReservedNeutral;
        assert!(own(live.clone(), Arc::clone(&graph), usize::MAX).is_ok());

        live.world.controller_leases[0].status = ControllerLeaseStatus::NeuralTakeover;
        live.world.controller_leases[0].takeover_committed_at_ms = Some(30_999);
        live.world.controller_leases[0].last_observed_at_ms = 30_999;
        assert!(matches!(
            own(live.clone(), Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease",
                ..
            })
        ));
        live.world.controller_leases[0].takeover_committed_at_ms = Some(31_000);
        live.world.controller_leases[0].last_observed_at_ms = 31_000;
        assert!(own(live, graph, usize::MAX).is_ok());
    }

    #[test]
    fn controller_leases_reject_non_external_snake_kinds() {
        let graph = default_graph();
        for (kind, id) in [
            (SnakeKind::Evolved, 1),
            (SnakeKind::Baseline, BASELINE_ENTITY_ID_START),
            (SnakeKind::Resurrected, RESURRECTED_ENTITY_ID_START),
        ] {
            let mut live = candidate(&graph, 1);
            live.phase = AuthorityPhase::Running;
            push_evolved_snake(&mut live, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
            if kind != SnakeKind::Evolved {
                let snake = &mut live.world.snakes[0];
                snake.id = id;
                snake.kind = kind;
                snake.population_slot = None;
                snake.brain = None;
                if kind == SnakeKind::Baseline {
                    snake.baseline_slot = Some(0);
                    snake.baseline_strategy = Some(BaselineStrategyState::Roam);
                    live.allocators.next_baseline_id = id + 1;
                    enable_one_live_baseline(&mut live, id, None);
                } else {
                    live.allocators.next_resurrected_id = id + 1;
                }
                live.allocators.next_entity_id = 1;
            }
            live.world
                .controller_leases
                .push(connected_lease(1, id, 7, "non-external-token"));
            live.allocators.next_controller_lease_id = 2;
            assert!(matches!(
                own(live, Arc::clone(&graph), usize::MAX),
                Err(StateError::InvalidField {
                    field: "controller_lease.snake_id",
                    ..
                })
            ));
        }
    }

    #[test]
    fn neural_takeover_rejects_a_brainless_external_snake() {
        let graph = default_graph();
        let mut live = candidate(&graph, 1);
        live.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut live, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
        let snake = &mut live.world.snakes[0];
        snake.id = EXTERNAL_ENTITY_ID_START;
        snake.kind = SnakeKind::External;
        snake.population_slot = None;
        snake.brain = None;
        live.allocators.next_entity_id = 1;
        live.allocators.next_external_id = EXTERNAL_ENTITY_ID_START + 1;
        live.world.controller_leases.push(ControllerLease {
            id: 1,
            snake_id: EXTERNAL_ENTITY_ID_START,
            kind: ControllerKind::ReinforcementLearning,
            connection_id: None,
            scope: "run-test".into(),
            resume_token: "rl-token".into(),
            status: ControllerLeaseStatus::NeuralTakeover,
            latest_action: LatestControllerAction {
                turn: 0.0,
                boost: false,
                client_tick: 0,
                arrival_sequence: 1,
                accepted_at_ms: 100,
            },
            last_observed_at_ms: 31_000,
            disconnected_at_ms: Some(1_000),
            input_hold_expires_at_ms: Some(600),
            grace_expires_at_ms: Some(31_000),
            takeover_committed_at_ms: Some(31_000),
        });
        live.allocators.next_controller_lease_id = 2;
        assert!(matches!(
            own(live, graph, usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease",
                ..
            })
        ));
    }

    #[test]
    fn malformed_rng_is_rejected_without_partial_state() {
        let graph = default_graph();
        let mut invalid = candidate(&graph, 1);
        invalid.rng.world.state_hex = "0x00000000".into();
        assert!(matches!(
            own(invalid, graph, usize::MAX),
            Err(StateError::InvalidRng { stream, .. }) if stream == "world"
        ));
    }
}
