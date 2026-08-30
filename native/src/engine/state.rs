//! Rust-owned persistent state contracts for the authoritative engine.
//!
//! This module deliberately contains data ownership, admission estimates, and
//! validation only. Gameplay behavior, N-API conversion, serialization,
//! database access, and managed-checkpoint I/O belong to later layers. A
//! candidate becomes authoritative only after the complete candidate has been
//! validated against its compiled graph and caller-supplied memory ceiling.

use super::baseline::{BaselineLifecycleState, BaselineSlotRuntime};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::external_replacement::{
    ExternalReplacementAuthorityProof, UnavailableControllerReason,
    UnavailableControllerReservation,
};
use super::graph::{
    CompiledGraph, CompiledNode, GraphBundle, GraphEdge, GraphNodeKind, GraphNodeSpec,
    GraphOutputRef, GraphSpec,
};
use super::physics::{PhysicsStepKey, PhysicsStepKeyField};
use super::rng::{RngError, SerializedRngState, StatefulRng};
use super::run_start::RunStartPersistenceProof;
use super::running_step::{ResolvedGenerationStartReplacement, ResolvedRunningStepReplacement};
use super::sensors::SensorGenerationState;
use super::step_config::{
    project_evolution_config, project_running_step_config, RunningStepConfigProjection,
    RunningStepWorkLimits, StepConfigError,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::mem::size_of;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
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
/// Next process-local identity assigned to a newly admitted authoritative world.
static NEXT_WORLD_EPOCH: AtomicU64 = AtomicU64::new(1);

fn allocate_world_epoch() -> Result<u64, StateError> {
    NEXT_WORLD_EPOCH
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| StateError::ArithmeticOverflow {
            context: "process-local world epoch",
        })
}
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

/// Mutable buffers produced by one complete running fixed-step transaction.
///
/// The publication boundary swaps these buffers with the current authority so
/// the caller retains the old allocations for reuse. Immutable run identity,
/// normalized configuration, graph and population weights are never supplied
/// by the caller and therefore cannot change through this operation. This is a
/// low-level transaction value, not a complete scheduler: the fixed-step
/// coordinator must still derive every buffer through the keyed phase chain and
/// resolve terminal-generation, controller-delivery and replacement policy
/// before calling the publication method.
pub(crate) struct RunningStepReplacement<'buffers> {
    /// Complete keyed attempt derived from the current authority.
    pub key: PhysicsStepKey,
    /// Post-physics world, including controller leases.
    pub world: &'buffers mut WorldState,
    /// Post-step gameplay RNG continuations.
    pub rng: &'buffers mut RngStateBundle,
    /// Post-step deterministic allocator continuations.
    pub allocators: &'buffers mut AllocatorState,
    /// Post-control recurrent state for every existing brain.
    pub brains: &'buffers mut Vec<BrainRuntimeState>,
    /// Post-step durable baseline lifecycle.
    pub baseline_lifecycle: &'buffers mut BaselineLifecycleState,
    /// Post-step fractional ambient-pellet credit.
    pub ambient_pellet_accumulator: f64,
    /// Post-step generation-best sensor continuation.
    pub sensor_generation: SensorGenerationState,
    /// Simulated generation time after exactly one fixed delta.
    pub generation_elapsed_seconds: f64,
    /// Scheduler debt retained after this committed step.
    pub wall_accumulator_seconds: f64,
    /// Private coordinator proof for the only permitted entity-identity changes.
    pub mutation: RunningStepMutationContract<'buffers>,
}

/// Complete next-generation running buffers before the old authority is
/// replaced.
pub(crate) struct GenerationStartReplacement<'buffers> {
    /// Terminal fixed-step attempt that produced the durable boundary.
    pub key: PhysicsStepKey,
    /// Collision-safe evolved, baseline, and external world.
    pub world: &'buffers mut WorldState,
    /// RNG continuation after generation construction and external genomes.
    pub rng: &'buffers mut RngStateBundle,
    /// Exact allocator continuation after every new entity and lease.
    pub allocators: &'buffers mut AllocatorState,
    /// Successor population brains plus admitted external brains.
    pub brains: &'buffers mut Vec<BrainRuntimeState>,
    /// Initialized generation-scoped baseline, ambient, and sensor state.
    pub fixed_step: &'buffers mut FixedStepContinuationState,
    /// Scheduler debt remaining after the terminal fixed step, excluding the
    /// persistence wait itself.
    pub wall_accumulator_seconds: f64,
    /// Opaque proof that the controller transaction produced these buffers.
    pub proof: &'buffers ExternalReplacementAuthorityProof,
}

/// Complete collision-safe buffers used to activate a durable fresh run-start boundary.
///
/// This carries no population or graph input: those remain inside the admitted
/// boundary. The source address is retained by the generation-start workspace
/// so a proposal prepared from another boundary cannot be published here.
pub(crate) struct InitialRunStartReplacement<'buffers> {
    /// Exact admitted boundary object borrowed during preparation.
    pub source_address: usize,
    /// Collision-safe evolved and baseline world with no external leases.
    pub world: &'buffers mut WorldState,
    /// RNG continuation after initial snakes and pellets.
    pub rng: &'buffers mut RngStateBundle,
    /// Allocator continuation after initial entities, frames, and pellets.
    pub allocators: &'buffers mut AllocatorState,
    /// Initialized generation-scoped continuation.
    pub fixed_step: &'buffers mut FixedStepContinuationState,
    /// Opaque proof available only after the exact durable descriptor acknowledgement.
    pub persistence_proof: &'buffers RunStartPersistenceProof,
}

/// Exact identity-changing work performed by the private complete-step coordinator.
///
/// A normal fixed step carries zeroes and therefore retains the strict historic
/// RNG, allocator, brain-identity, and weight contract. Controlled-death
/// replacement is the only current path that may advance the isolated external
/// RNG and its dedicated identity domains during a nonterminal step.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RunningStepMutationContract<'proof> {
    proof: Option<&'proof ExternalReplacementAuthorityProof>,
}

impl<'proof> RunningStepMutationContract<'proof> {
    pub(crate) const fn external(proof: &'proof ExternalReplacementAuthorityProof) -> Self {
        Self { proof: Some(proof) }
    }

    pub(crate) const fn external_replacements(self) -> usize {
        match self.proof {
            Some(proof) => proof.replacements(),
            None => 0,
        }
    }

    pub(crate) const fn removed_dead_external_leases(self) -> usize {
        match self.proof {
            Some(proof) => proof.removed_dead_leases(),
            None => 0,
        }
    }

    fn proof_identity(self) -> usize {
        self.proof
            .map_or(0, |proof| std::ptr::from_ref(proof).addr())
    }

    fn matches(self, replacement: &RunningStepReplacement<'_>) -> bool {
        self.proof.is_none_or(|proof| {
            proof.matches(
                replacement.key,
                replacement.world,
                replacement.rng,
                replacement.allocators,
                replacement.brains,
            )
        })
    }
}

/// Result of one successful atomic running-step publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningStepPublication {
    /// Key whose staged values became authoritative.
    pub key: PhysicsStepKey,
    /// Completed-step identity after publication.
    pub completed_step: u64,
    /// Recomputed admitted memory estimate for the new authority.
    pub memory: StateMemoryEstimate,
}

/// Result of activating one durable generation-one run-start boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunStartPublication {
    /// Process-local world incarnation retained from the staged boundary.
    pub world_epoch: u64,
    /// First running generation.
    pub generation: u64,
    /// Run-start completed-step identity, always zero.
    pub completed_step: u64,
    /// First population/brain epoch.
    pub population_epoch: u64,
    /// Recomputed admitted memory estimate for the running authority.
    pub memory: StateMemoryEstimate,
}

/// Result of one successful durable-boundary-to-running authority swap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationStartPublication {
    /// Terminal fixed-step key belonging to the replaced authority.
    pub source_key: PhysicsStepKey,
    /// New process-local world incarnation.
    pub world_epoch: u64,
    /// First running generation after the completed round.
    pub generation: u64,
    /// Completed-step identity including the terminal step.
    pub completed_step: u64,
    /// New population/brain epoch.
    pub population_epoch: u64,
    /// Recomputed admitted memory estimate for the running successor.
    pub memory: StateMemoryEstimate,
    /// Connected controllers that received a fresh assignment.
    pub external_assignments: usize,
    /// Disconnected or already-taken-over old-token outcomes that the thin
    /// lifecycle bridge must retain after the old world is gone.
    pub unavailable_controller_reservations: Vec<UnavailableControllerReservation>,
}

/// Process-local proof that one exact set of running-step buffers passed full
/// state validation while authority was exclusively borrowed, then was restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunningStepPreflight {
    key: PhysicsStepKey,
    completed_step: u64,
    memory: StateMemoryEstimate,
    buffers: RunningStepBufferIdentity,
    external_replacements: usize,
    removed_dead_external_leases: usize,
}

/// Process-local proof that one exact successor buffer set passed complete
/// running-state validation and was restored before assignments were exposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GenerationStartPreflight {
    source_key: PhysicsStepKey,
    memory: StateMemoryEstimate,
    buffers: GenerationStartBufferIdentity,
    external_replacements: usize,
    removed_source_leases: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RunningStepBufferIdentity {
    world: usize,
    rng: usize,
    allocators: usize,
    brains: usize,
    baseline_lifecycle: usize,
    ambient_pellet_accumulator: u64,
    sensor_generation_best: u64,
    generation_elapsed_seconds: u64,
    wall_accumulator_seconds: u64,
    mutation_proof: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GenerationStartBufferIdentity {
    world: usize,
    rng: usize,
    allocators: usize,
    brains: usize,
    fixed_step: usize,
    wall_accumulator_seconds: u64,
    proof: usize,
}

/// Reusable identity/range storage for complete world validation.
#[derive(Debug, Default)]
struct WorldValidationScratch {
    entity_ids: Vec<u64>,
    snake_ids: Vec<u64>,
    public_ids: Vec<u32>,
    evolved_slots: Vec<u32>,
    world_brains: Vec<BrainHandle>,
    baseline_slots: Vec<u32>,
    ranges: Vec<(usize, usize, u64)>,
    lease_ids: Vec<u64>,
    lease_snakes: Vec<u64>,
    resume_token_order: Vec<usize>,
    connection_ids: Vec<u64>,
}

impl WorldValidationScratch {
    fn clear(&mut self) {
        self.entity_ids.clear();
        self.snake_ids.clear();
        self.public_ids.clear();
        self.evolved_slots.clear();
        self.world_brains.clear();
        self.baseline_slots.clear();
        self.ranges.clear();
        self.lease_ids.clear();
        self.lease_snakes.clear();
        self.resume_token_order.clear();
        self.connection_ids.clear();
    }
}

/// Validated Rust-owned state. Fields stay private so an invalid candidate
/// cannot be assembled by struct literal and accidentally published.
#[derive(Debug)]
pub struct AuthoritativeState {
    candidate: StateCandidate,
    graph: Arc<GraphBundle>,
    memory: StateMemoryEstimate,
    memory_ceiling_bytes: usize,
    world_epoch: u64,
    latest_operation_epoch: u64,
    world_validation: WorldValidationScratch,
}

/// Fully admitted next-generation boundary retained beside the still-current world.
///
/// Fields stay private so only the generation transaction can use the reduced
/// checkpoint budget or eventually consume the staged authority. The budget is
/// the process ceiling remaining after charging the complete current authority.
#[derive(Debug)]
pub(crate) struct AdmittedGenerationSuccessor {
    source_key: PhysicsStepKey,
    authority: AuthoritativeState,
    checkpoint_policy: StateAdmissionPolicy,
    full_memory_ceiling_bytes: usize,
    combined_state_bytes: usize,
}

impl AdmittedGenerationSuccessor {
    /// Exact terminal fixed-step attempt from which this successor was derived.
    pub(crate) const fn source_key(&self) -> PhysicsStepKey {
        self.source_key
    }

    /// Read the fully validated pre-spawn boundary without making it current.
    pub(crate) const fn authority(&self) -> &AuthoritativeState {
        &self.authority
    }

    /// Remaining-budget policy used for managed-checkpoint workspace admission.
    pub(crate) const fn checkpoint_policy(&self) -> &StateAdmissionPolicy {
        &self.checkpoint_policy
    }

    /// Conservative current-plus-successor state bytes retained simultaneously.
    pub(crate) const fn combined_state_bytes(&self) -> usize {
        self.combined_state_bytes
    }

    /// Full process ceiling restored only when a later transaction consumes the old world.
    pub(crate) const fn full_memory_ceiling_bytes(&self) -> usize {
        self.full_memory_ceiling_bytes
    }

    /// Validate a complete running successor, then restore its exact boundary
    /// before any reliable assignment becomes visible outside Rust.
    pub(crate) fn preflight_running_start(
        &mut self,
        current: &AuthoritativeState,
        replacement: &mut GenerationStartReplacement<'_>,
        unavailable_controller_reservations: &[UnavailableControllerReservation],
    ) -> Result<GenerationStartPreflight, StateError> {
        current.validate_running_step_key(replacement.key)?;
        if replacement.key != self.source_key {
            return invalid(
                "generation.start.key",
                "running successor does not match its terminal source",
            );
        }
        if !replacement.proof.matches(
            replacement.key,
            replacement.world,
            replacement.rng,
            replacement.allocators,
            replacement.brains,
        ) {
            return invalid(
                "generation.start.external_replacement",
                "opaque generation controller proof does not match staged buffers",
            );
        }
        let external_replacements = replacement.proof.replacements();
        let removed_source_leases = replacement.proof.removed_dead_leases();
        validate_generation_start_replacement_contract(
            &current.candidate,
            &self.authority.candidate,
            replacement,
            external_replacements,
            removed_source_leases,
            unavailable_controller_reservations,
        )?;
        let buffers = generation_start_buffer_identity(replacement);
        let prior_phase = self.authority.candidate.phase;
        let prior_wall_accumulator = self.authority.candidate.generation.wall_accumulator_seconds;
        swap_generation_start_buffers(&mut self.authority.candidate, replacement);
        self.authority.candidate.phase = AuthorityPhase::Running;
        self.authority.candidate.generation.wall_accumulator_seconds =
            replacement.wall_accumulator_seconds;

        let validation = catch_unwind(AssertUnwindSafe(|| {
            let memory = estimate_state_memory(&self.authority.candidate, &self.authority.graph)?;
            if memory.total_bytes > self.full_memory_ceiling_bytes {
                return Err(StateError::MemoryCeilingExceeded {
                    estimated_bytes: memory.total_bytes,
                    ceiling_bytes: self.full_memory_ceiling_bytes,
                });
            }
            validate_population(&self.authority.candidate, self.authority.graph())?;
            validate_running_mutable_state(
                &self.authority.candidate,
                self.authority.graph.compiled(),
                &mut self.authority.world_validation,
            )?;
            Ok(memory)
        }));

        self.authority.candidate.phase = prior_phase;
        self.authority.candidate.generation.wall_accumulator_seconds = prior_wall_accumulator;
        swap_generation_start_buffers(&mut self.authority.candidate, replacement);
        match validation {
            Ok(Ok(memory)) => Ok(GenerationStartPreflight {
                source_key: replacement.key,
                memory,
                buffers,
                external_replacements,
                removed_source_leases,
            }),
            Ok(Err(error)) => Err(error),
            Err(payload) => resume_unwind(payload),
        }
    }

    /// Atomically replace the old authority with the exact preflighted running
    /// successor after every reliable assignment has resolved.
    pub(crate) fn publish_running_start(
        &mut self,
        current: &mut AuthoritativeState,
        preflight: GenerationStartPreflight,
        resolved: ResolvedGenerationStartReplacement<'_>,
        unavailable_controller_reservations: Vec<UnavailableControllerReservation>,
    ) -> Result<GenerationStartPublication, StateError> {
        let (mut replacement, external_replacements, removed_source_leases) = resolved.into_parts();
        current.validate_running_step_key(replacement.key)?;
        if replacement.key != self.source_key
            || preflight.source_key != replacement.key
            || preflight.buffers != generation_start_buffer_identity(&replacement)
            || preflight.external_replacements != external_replacements
            || preflight.removed_source_leases != removed_source_leases
        {
            return invalid(
                "generation.start.preflight",
                "resolved buffers, source, or controller counts changed",
            );
        }
        validate_generation_start_replacement_contract(
            &current.candidate,
            &self.authority.candidate,
            &replacement,
            external_replacements,
            removed_source_leases,
            &unavailable_controller_reservations,
        )?;
        let prior_phase = self.authority.candidate.phase;
        let prior_wall_accumulator = self.authority.candidate.generation.wall_accumulator_seconds;
        swap_generation_start_buffers(&mut self.authority.candidate, &mut replacement);
        self.authority.candidate.phase = AuthorityPhase::Running;
        self.authority.candidate.generation.wall_accumulator_seconds =
            replacement.wall_accumulator_seconds;

        let memory = estimate_state_memory(&self.authority.candidate, &self.authority.graph);
        let memory_error = match memory {
            Ok(memory) if memory == preflight.memory => None,
            Ok(_) => Some(invalid_error(
                "generation.start.preflight.memory",
                "resolved assignment changed admitted allocation",
            )),
            Err(error) => Some(error),
        };
        if let Some(error) = memory_error {
            self.authority.candidate.phase = prior_phase;
            self.authority.candidate.generation.wall_accumulator_seconds = prior_wall_accumulator;
            swap_generation_start_buffers(&mut self.authority.candidate, &mut replacement);
            return Err(error);
        }

        self.authority.memory = preflight.memory;
        self.authority.memory_ceiling_bytes = self.full_memory_ceiling_bytes;
        let publication = GenerationStartPublication {
            source_key: replacement.key,
            world_epoch: self.authority.world_epoch,
            generation: self.authority.candidate.generation.generation,
            completed_step: self.authority.candidate.generation.completed_step,
            population_epoch: self.authority.candidate.generation.population_epoch,
            memory: preflight.memory,
            external_assignments: external_replacements,
            unavailable_controller_reservations,
        };
        std::mem::swap(current, &mut self.authority);
        Ok(publication)
    }
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
        let mut world_validation = WorldValidationScratch::default();
        validate_candidate(&candidate, graph.compiled(), policy, &mut world_validation)?;
        let world_epoch = allocate_world_epoch()?;
        Ok(Self {
            candidate,
            graph,
            memory,
            memory_ceiling_bytes: policy.memory_ceiling_bytes,
            world_epoch,
            latest_operation_epoch: 0,
            world_validation,
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

    /// Unique process-local incarnation used to reject cross-authority work.
    #[must_use]
    pub(crate) const fn world_epoch(&self) -> u64 {
        self.world_epoch
    }

    /// Project the complete fixed-step and sensor formulas from this admitted authority.
    ///
    /// Callers cannot substitute a different normalized settings object. Work
    /// ceilings remain explicit process policy and fail the step if exceeded;
    /// they do not silently truncate sensing, collision, or spawn truth.
    pub fn running_step_config(
        &self,
        limits: RunningStepWorkLimits,
    ) -> Result<RunningStepConfigProjection, StepConfigError> {
        project_running_step_config(&self.candidate.config, limits)
    }

    /// Project the complete selection-pressure contract from admitted run state.
    pub fn evolution_config(&self) -> Result<super::evolution::EvolutionConfig, StepConfigError> {
        project_evolution_config(&self.candidate.config, self.graph().total_parameters)
    }

    /// Admit one exact next-generation boundary while retaining this authority.
    ///
    /// The successor is checked against the active terminal-step key and may use
    /// only the memory budget left after this complete authority. This deliberately
    /// charges two full admitted states during the handoff. Later measurements may
    /// justify a narrower shared-allocation estimate, but no shared graph or scratch
    /// is silently subtracted here.
    pub(crate) fn admit_generation_successor(
        &self,
        source_key: PhysicsStepKey,
        candidate: StateCandidate,
    ) -> Result<AdmittedGenerationSuccessor, StateError> {
        self.validate_running_step_key(source_key)?;
        validate_generation_successor_relation(&self.candidate, &candidate)?;
        let remaining_bytes = self
            .memory_ceiling_bytes
            .checked_sub(self.memory.total_bytes)
            .ok_or(StateError::MemoryCeilingExceeded {
                estimated_bytes: self.memory.total_bytes,
                ceiling_bytes: self.memory_ceiling_bytes,
            })?;
        let checkpoint_policy = successor_admission_policy(&self.candidate, remaining_bytes);
        let successor =
            Self::validate_and_own(candidate, Arc::clone(&self.graph), &checkpoint_policy)?;
        let combined_state_bytes = self
            .memory
            .total_bytes
            .checked_add(successor.memory.total_bytes)
            .ok_or(StateError::ArithmeticOverflow {
                context: "current plus staged generation authority",
            })?;
        if combined_state_bytes > self.memory_ceiling_bytes {
            return Err(StateError::MemoryCeilingExceeded {
                estimated_bytes: combined_state_bytes,
                ceiling_bytes: self.memory_ceiling_bytes,
            });
        }
        Ok(AdmittedGenerationSuccessor {
            source_key,
            authority: successor,
            checkpoint_policy,
            full_memory_ceiling_bytes: self.memory_ceiling_bytes,
            combined_state_bytes,
        })
    }

    /// Atomically activate one exact durable fresh run-start boundary.
    ///
    /// The caller can supply only the mutable world/RNG/allocator/continuation
    /// buffers prepared from this exact boundary. Immutable run identity,
    /// configuration, graph, population weights, brain ownership, and recurrent
    /// state never leave this authority. Any validation error or panic restores
    /// the generation-one boundary and all staged buffers before returning.
    pub(crate) fn publish_initial_run_start(
        &mut self,
        replacement: &mut InitialRunStartReplacement<'_>,
    ) -> Result<RunStartPublication, StateError> {
        if self.candidate.phase
            != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart)
        {
            return invalid(
                "run_start.phase",
                "initial activation requires the durable run-start boundary",
            );
        }
        if self.candidate.generation.generation != 1
            || self.candidate.generation.completed_step != 0
            || self.candidate.generation.population_epoch != 1
            || self.candidate.generation.elapsed_seconds.to_bits() != 0.0_f64.to_bits()
            || self.candidate.generation.wall_accumulator_seconds.to_bits() != 0.0_f64.to_bits()
        {
            return invalid(
                "run_start.generation",
                "initial activation requires generation one at completed step zero",
            );
        }
        if replacement.source_address != std::ptr::from_ref(&self.candidate).addr() {
            return invalid(
                "run_start.source",
                "initial running buffers were prepared from another boundary",
            );
        }
        if !replacement
            .persistence_proof
            .matches(replacement.source_address, self.world_epoch)
        {
            return invalid(
                "run_start.persistence",
                "initial running buffers lack the exact persistence proof",
            );
        }
        if !replacement.world.controller_leases.is_empty() {
            return invalid(
                "run_start.controllers",
                "fresh run activation cannot carry controller leases",
            );
        }

        let prior_phase = self.candidate.phase;
        swap_initial_run_start_buffers(&mut self.candidate, replacement);
        self.candidate.phase = AuthorityPhase::Running;
        let validation = catch_unwind(AssertUnwindSafe(|| {
            let memory = estimate_state_memory(&self.candidate, &self.graph)?;
            if memory.total_bytes > self.memory_ceiling_bytes {
                return Err(StateError::MemoryCeilingExceeded {
                    estimated_bytes: memory.total_bytes,
                    ceiling_bytes: self.memory_ceiling_bytes,
                });
            }
            validate_population(&self.candidate, self.graph.compiled())?;
            validate_running_mutable_state(
                &self.candidate,
                self.graph.compiled(),
                &mut self.world_validation,
            )?;
            Ok(memory)
        }));

        match validation {
            Ok(Ok(memory)) => {
                self.memory = memory;
                Ok(RunStartPublication {
                    world_epoch: self.world_epoch,
                    generation: self.candidate.generation.generation,
                    completed_step: self.candidate.generation.completed_step,
                    population_epoch: self.candidate.generation.population_epoch,
                    memory,
                })
            }
            Ok(Err(error)) => {
                self.candidate.phase = prior_phase;
                swap_initial_run_start_buffers(&mut self.candidate, replacement);
                Err(error)
            }
            Err(payload) => {
                self.candidate.phase = prior_phase;
                swap_initial_run_start_buffers(&mut self.candidate, replacement);
                resume_unwind(payload)
            }
        }
    }

    /// Begin one fresh running-step attempt from the current authority.
    ///
    /// Every call advances the process-local operation epoch, including after
    /// a prior attempt failed. A result prepared under an older attempt is
    /// therefore stale before any buffer swap can occur. Receiving a key does
    /// not authorize bypassing the fixed-step phase/configuration coordinator.
    pub fn begin_running_step(&mut self) -> Result<PhysicsStepKey, StateError> {
        if self.candidate.phase != AuthorityPhase::Running {
            return invalid("phase", "a fixed step requires running authority");
        }
        let operation_epoch =
            self.latest_operation_epoch
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "fixed-step operation epoch",
                })?;
        let key = self.running_step_key(operation_epoch)?;
        self.latest_operation_epoch = operation_epoch;
        Ok(key)
    }

    /// Publish one fully staged running fixed step with one reversible swap.
    ///
    /// All fallible key checks happen before the swap. Complete mutable-state
    /// validation and memory admission run while `&mut self` excludes readers;
    /// rejection or unwinding restores the prior authoritative buffers and
    /// scalar continuation before the error or panic leaves this method. The
    /// caller remains responsible for supplying the one complete keyed phase
    /// result; this method validates state shape and provenance identity, not
    /// gameplay formulas independently of the phase workspaces.
    pub(crate) fn publish_running_step(
        &mut self,
        mut replacement: RunningStepReplacement<'_>,
    ) -> Result<RunningStepPublication, StateError> {
        self.validate_running_step_key(replacement.key)?;
        let completed_step = replacement
            .key
            .source_completed_step()
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "committed fixed-step identity",
            })?;
        validate_running_replacement_contract(&self.candidate, &replacement)?;
        let prior_completed_step = self.candidate.generation.completed_step;
        let prior_elapsed_seconds = self.candidate.generation.elapsed_seconds;
        let prior_wall_accumulator_seconds = self.candidate.generation.wall_accumulator_seconds;
        let prior_ambient_pellet_accumulator = self.candidate.fixed_step.ambient_pellet_accumulator;
        let prior_sensor_generation = self.candidate.fixed_step.sensor_generation;

        swap_running_step_buffers(&mut self.candidate, &mut replacement);
        self.candidate.generation.completed_step = completed_step;
        self.candidate.generation.elapsed_seconds = replacement.generation_elapsed_seconds;
        self.candidate.generation.wall_accumulator_seconds = replacement.wall_accumulator_seconds;
        self.candidate.fixed_step.ambient_pellet_accumulator =
            replacement.ambient_pellet_accumulator;
        self.candidate.fixed_step.sensor_generation = replacement.sensor_generation;

        let validation = catch_unwind(AssertUnwindSafe(|| {
            let memory = estimate_state_memory(&self.candidate, &self.graph)?;
            if memory.total_bytes > self.memory_ceiling_bytes {
                return Err(StateError::MemoryCeilingExceeded {
                    estimated_bytes: memory.total_bytes,
                    ceiling_bytes: self.memory_ceiling_bytes,
                });
            }
            validate_running_mutable_state(
                &self.candidate,
                self.graph.compiled(),
                &mut self.world_validation,
            )?;
            Ok(memory)
        }));

        match validation {
            Ok(Ok(memory)) => {
                self.memory = memory;
                Ok(RunningStepPublication {
                    key: replacement.key,
                    completed_step,
                    memory,
                })
            }
            Ok(Err(error)) => {
                self.candidate.generation.completed_step = prior_completed_step;
                self.candidate.generation.elapsed_seconds = prior_elapsed_seconds;
                self.candidate.generation.wall_accumulator_seconds = prior_wall_accumulator_seconds;
                self.candidate.fixed_step.ambient_pellet_accumulator =
                    prior_ambient_pellet_accumulator;
                self.candidate.fixed_step.sensor_generation = prior_sensor_generation;
                swap_running_step_buffers(&mut self.candidate, &mut replacement);
                Err(error)
            }
            Err(payload) => {
                self.candidate.generation.completed_step = prior_completed_step;
                self.candidate.generation.elapsed_seconds = prior_elapsed_seconds;
                self.candidate.generation.wall_accumulator_seconds = prior_wall_accumulator_seconds;
                self.candidate.fixed_step.ambient_pellet_accumulator =
                    prior_ambient_pellet_accumulator;
                self.candidate.fixed_step.sensor_generation = prior_sensor_generation;
                swap_running_step_buffers(&mut self.candidate, &mut replacement);
                resume_unwind(payload)
            }
        }
    }

    /// Fully validate one exact replacement and restore both authority and
    /// scratch before returning a process-local publication proof.
    ///
    /// This is used only when a complete physical step must wait for a local
    /// Node send result. It prevents an observation from being accepted by
    /// Node before discovering that the staged authority itself is invalid or
    /// exceeds its memory ceiling.
    pub(crate) fn preflight_running_step(
        &mut self,
        replacement: &mut RunningStepReplacement<'_>,
    ) -> Result<RunningStepPreflight, StateError> {
        self.validate_running_step_key(replacement.key)?;
        let completed_step = replacement
            .key
            .source_completed_step()
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "preflight fixed-step identity",
            })?;
        validate_running_replacement_contract(&self.candidate, replacement)?;
        let buffers = running_step_buffer_identity(replacement);
        let prior_completed_step = self.candidate.generation.completed_step;
        let prior_elapsed_seconds = self.candidate.generation.elapsed_seconds;
        let prior_wall_accumulator_seconds = self.candidate.generation.wall_accumulator_seconds;
        let prior_ambient_pellet_accumulator = self.candidate.fixed_step.ambient_pellet_accumulator;
        let prior_sensor_generation = self.candidate.fixed_step.sensor_generation;

        swap_running_step_buffers(&mut self.candidate, replacement);
        self.candidate.generation.completed_step = completed_step;
        self.candidate.generation.elapsed_seconds = replacement.generation_elapsed_seconds;
        self.candidate.generation.wall_accumulator_seconds = replacement.wall_accumulator_seconds;
        self.candidate.fixed_step.ambient_pellet_accumulator =
            replacement.ambient_pellet_accumulator;
        self.candidate.fixed_step.sensor_generation = replacement.sensor_generation;

        let validation = catch_unwind(AssertUnwindSafe(|| {
            let memory = estimate_state_memory(&self.candidate, &self.graph)?;
            if memory.total_bytes > self.memory_ceiling_bytes {
                return Err(StateError::MemoryCeilingExceeded {
                    estimated_bytes: memory.total_bytes,
                    ceiling_bytes: self.memory_ceiling_bytes,
                });
            }
            validate_running_mutable_state(
                &self.candidate,
                self.graph.compiled(),
                &mut self.world_validation,
            )?;
            Ok(memory)
        }));

        self.candidate.generation.completed_step = prior_completed_step;
        self.candidate.generation.elapsed_seconds = prior_elapsed_seconds;
        self.candidate.generation.wall_accumulator_seconds = prior_wall_accumulator_seconds;
        self.candidate.fixed_step.ambient_pellet_accumulator = prior_ambient_pellet_accumulator;
        self.candidate.fixed_step.sensor_generation = prior_sensor_generation;
        swap_running_step_buffers(&mut self.candidate, replacement);

        match validation {
            Ok(Ok(memory)) => Ok(RunningStepPreflight {
                key: replacement.key,
                completed_step,
                memory,
                buffers,
                external_replacements: replacement.mutation.external_replacements(),
                removed_dead_external_leases: replacement.mutation.removed_dead_external_leases(),
            }),
            Ok(Err(error)) => Err(error),
            Err(payload) => resume_unwind(payload),
        }
    }

    /// Revalidate an exact preflight token and replacement without publishing.
    ///
    /// The coordinator calls this before it accepts any local Node result. The
    /// retained buffers then remain exclusively owned until the only permitted
    /// content changes—prevalidated delivery markers or controller
    /// disconnects—are applied and published.
    pub(crate) fn validate_preflighted_running_step(
        &self,
        preflight: RunningStepPreflight,
        replacement: &RunningStepReplacement<'_>,
    ) -> Result<(), StateError> {
        self.validate_running_step_key(replacement.key)?;
        if preflight.key != replacement.key
            || preflight.buffers != running_step_buffer_identity(replacement)
        {
            return invalid(
                "fixed_step.preflight",
                "replacement buffers or scalar continuation changed",
            );
        }
        let completed_step = replacement
            .key
            .source_completed_step()
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "preflighted fixed-step identity",
            })?;
        if completed_step != preflight.completed_step {
            return invalid("fixed_step.preflight", "completed-step identity changed");
        }
        validate_running_replacement_contract(&self.candidate, replacement)?;
        Ok(())
    }

    /// Publish a replacement sealed by the private running-step coordinator.
    ///
    /// Sibling modules cannot construct the consumed wrapper, so retaining a
    /// preflight token and arbitrary mutable scratch is insufficient to reach
    /// this swap. The final memory estimate must still equal the preflighted
    /// estimate; a mismatch restores every authoritative buffer and scalar.
    pub(crate) fn publish_prevalidated_running_step(
        &mut self,
        preflight: RunningStepPreflight,
        resolved: ResolvedRunningStepReplacement<'_>,
    ) -> Result<RunningStepPublication, StateError> {
        let (mut replacement, external_replacements, removed_dead_external_leases) =
            resolved.into_parts();
        self.validate_running_step_key(replacement.key)?;
        if preflight.key != replacement.key
            || preflight.buffers != running_step_buffer_identity(&replacement)
            || preflight.external_replacements != external_replacements
            || preflight.removed_dead_external_leases != removed_dead_external_leases
        {
            return invalid(
                "fixed_step.preflight",
                "resolved replacement buffers, scalars, or mutation counts changed",
            );
        }
        let completed_step = replacement
            .key
            .source_completed_step()
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "prevalidated completed-step identity",
            })?;
        if completed_step != preflight.completed_step {
            return invalid("fixed_step.preflight", "completed-step identity changed");
        }
        validate_resolved_running_replacement_contract(
            &self.candidate,
            &replacement,
            external_replacements,
            removed_dead_external_leases,
        )?;
        let prior_completed_step = self.candidate.generation.completed_step;
        let prior_elapsed_seconds = self.candidate.generation.elapsed_seconds;
        let prior_wall_accumulator_seconds = self.candidate.generation.wall_accumulator_seconds;
        let prior_ambient_pellet_accumulator = self.candidate.fixed_step.ambient_pellet_accumulator;
        let prior_sensor_generation = self.candidate.fixed_step.sensor_generation;

        swap_running_step_buffers(&mut self.candidate, &mut replacement);
        self.candidate.generation.completed_step = preflight.completed_step;
        self.candidate.generation.elapsed_seconds = replacement.generation_elapsed_seconds;
        self.candidate.generation.wall_accumulator_seconds = replacement.wall_accumulator_seconds;
        self.candidate.fixed_step.ambient_pellet_accumulator =
            replacement.ambient_pellet_accumulator;
        self.candidate.fixed_step.sensor_generation = replacement.sensor_generation;

        let memory = estimate_state_memory(&self.candidate, &self.graph);
        let memory_error = match memory {
            Ok(memory) if memory == preflight.memory => None,
            Ok(_) => Some(invalid_error(
                "fixed_step.preflight.memory",
                "resolved replacement allocation changed after preflight",
            )),
            Err(error) => Some(error),
        };
        if let Some(error) = memory_error {
            self.candidate.generation.completed_step = prior_completed_step;
            self.candidate.generation.elapsed_seconds = prior_elapsed_seconds;
            self.candidate.generation.wall_accumulator_seconds = prior_wall_accumulator_seconds;
            self.candidate.fixed_step.ambient_pellet_accumulator = prior_ambient_pellet_accumulator;
            self.candidate.fixed_step.sensor_generation = prior_sensor_generation;
            swap_running_step_buffers(&mut self.candidate, &mut replacement);
            return Err(error);
        }
        self.memory = preflight.memory;
        Ok(RunningStepPublication {
            key: replacement.key,
            completed_step: preflight.completed_step,
            memory: preflight.memory,
        })
    }

    /// Revalidate one staged operation against the still-current authority
    /// without publishing or advancing its operation epoch.
    pub(crate) fn validate_running_step_key(&self, key: PhysicsStepKey) -> Result<(), StateError> {
        if self.latest_operation_epoch == 0 {
            return invalid("fixed_step.key", "no running-step attempt is active");
        }
        let expected_key = self.running_step_key(self.latest_operation_epoch)?;
        if let Some(field) = key.first_mismatch(expected_key) {
            return Err(StateError::StaleFixedStep { field });
        }
        Ok(())
    }

    /// Revalidate the exact publication most recently committed by this authority.
    ///
    /// The source step in a publication is one behind the now-authoritative
    /// completed step. This check deliberately binds every process/run/config
    /// identity component plus the admitted memory result, so a scheduler
    /// cannot retire a ticket with a fabricated or foreign publication.
    pub(crate) fn validate_running_step_publication(
        &self,
        publication: RunningStepPublication,
    ) -> Result<(), StateError> {
        if self.candidate.phase != AuthorityPhase::Running {
            return invalid(
                "fixed_step.publication.phase",
                "a running-step publication requires running authority",
            );
        }
        if self.latest_operation_epoch == 0 {
            return invalid(
                "fixed_step.publication.key",
                "no running-step operation has published",
            );
        }
        if publication.completed_step != self.candidate.generation.completed_step {
            return invalid(
                "fixed_step.publication.completed_step",
                "publication does not match the authoritative completed step",
            );
        }
        if publication.memory != self.memory {
            return invalid(
                "fixed_step.publication.memory",
                "publication does not match the admitted authority memory",
            );
        }
        let source_completed_step =
            publication
                .completed_step
                .checked_sub(1)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "running-step publication source completed step",
                })?;
        let expected_key = PhysicsStepKey::new(
            self.world_epoch,
            self.candidate.generation.generation,
            source_completed_step,
            self.candidate.generation.population_epoch,
            self.candidate.identity.config_revision,
            decode_sha256_identity("identity.config_hash", &self.candidate.identity.config_hash)?,
            self.latest_operation_epoch,
        );
        if let Some(field) = publication.key.first_mismatch(expected_key) {
            return Err(StateError::StaleFixedStep { field });
        }
        Ok(())
    }

    /// Reconstruct the complete current key for one process-local operation.
    fn running_step_key(&self, operation_epoch: u64) -> Result<PhysicsStepKey, StateError> {
        Ok(PhysicsStepKey::new(
            self.world_epoch,
            self.candidate.generation.generation,
            self.candidate.generation.completed_step,
            self.candidate.generation.population_epoch,
            self.candidate.identity.config_revision,
            decode_sha256_identity("identity.config_hash", &self.candidate.identity.config_hash)?,
            operation_epoch,
        ))
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

fn validate_generation_successor_relation(
    source: &StateCandidate,
    successor: &StateCandidate,
) -> Result<(), StateError> {
    if source.phase != AuthorityPhase::Running {
        return invalid(
            "generation.source.phase",
            "a generation successor requires running source authority",
        );
    }
    if successor.phase != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation) {
        return invalid(
            "generation.successor.phase",
            "a terminal fixed step must stage a generation boundary",
        );
    }
    if successor.versions != source.versions
        || successor.identity != source.identity
        || successor.config != source.config
    {
        return invalid(
            "generation.successor.identity",
            "version, run, graph, or normalized configuration identity changed",
        );
    }
    let expected_generation =
        source
            .generation
            .generation
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "staged successor generation",
            })?;
    let expected_completed_step =
        source
            .generation
            .completed_step
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "staged successor completed step",
            })?;
    let expected_population_epoch = source.generation.population_epoch.checked_add(1).ok_or(
        StateError::ArithmeticOverflow {
            context: "staged successor population epoch",
        },
    )?;
    if successor.generation.generation != expected_generation
        || successor.generation.completed_step != expected_completed_step
        || successor.generation.population_epoch != expected_population_epoch
    {
        return invalid(
            "generation.successor.sequence",
            "generation, completed-step, or population epoch is not the exact successor",
        );
    }
    if successor.generation.best_fitness_ever < source.generation.best_fitness_ever {
        return invalid(
            "generation.successor.best_fitness_ever",
            "all-generation best fitness regressed",
        );
    }
    Ok(())
}

fn successor_admission_policy(
    source: &StateCandidate,
    remaining_memory_ceiling_bytes: usize,
) -> StateAdmissionPolicy {
    StateAdmissionPolicy {
        memory_ceiling_bytes: remaining_memory_ceiling_bytes,
        expected_source_revision: source.identity.source_revision.clone(),
        expected_engine_build_id: source.identity.engine_build_id.clone(),
        expected_source_sha256: source.identity.source_sha256.clone(),
        expected_target_triple: source.identity.target_triple.clone(),
        expected_build_profile: source.identity.build_profile.clone(),
        expected_build_class: source.identity.build_class.clone(),
        expected_rustc_version: source.identity.rustc_version.clone(),
        expected_build_contract_sha256: source.identity.build_contract_sha256.clone(),
        expected_math_backend: source.identity.math_backend.clone(),
        expected_settings_schema_sha256: source.config.settings_schema_sha256.clone(),
    }
}

fn running_step_buffer_identity(
    replacement: &RunningStepReplacement<'_>,
) -> RunningStepBufferIdentity {
    RunningStepBufferIdentity {
        world: std::ptr::from_ref(&*replacement.world).addr(),
        rng: std::ptr::from_ref(&*replacement.rng).addr(),
        allocators: std::ptr::from_ref(&*replacement.allocators).addr(),
        brains: std::ptr::from_ref(&*replacement.brains).addr(),
        baseline_lifecycle: std::ptr::from_ref(&*replacement.baseline_lifecycle).addr(),
        ambient_pellet_accumulator: replacement.ambient_pellet_accumulator.to_bits(),
        sensor_generation_best: replacement
            .sensor_generation
            .best_points_this_generation()
            .to_bits(),
        generation_elapsed_seconds: replacement.generation_elapsed_seconds.to_bits(),
        wall_accumulator_seconds: replacement.wall_accumulator_seconds.to_bits(),
        mutation_proof: replacement.mutation.proof_identity(),
    }
}

fn generation_start_buffer_identity(
    replacement: &GenerationStartReplacement<'_>,
) -> GenerationStartBufferIdentity {
    GenerationStartBufferIdentity {
        world: std::ptr::from_ref(&*replacement.world).addr(),
        rng: std::ptr::from_ref(&*replacement.rng).addr(),
        allocators: std::ptr::from_ref(&*replacement.allocators).addr(),
        brains: std::ptr::from_ref(&*replacement.brains).addr(),
        fixed_step: std::ptr::from_ref(&*replacement.fixed_step).addr(),
        wall_accumulator_seconds: replacement.wall_accumulator_seconds.to_bits(),
        proof: std::ptr::from_ref(replacement.proof).addr(),
    }
}

fn swap_generation_start_buffers(
    candidate: &mut StateCandidate,
    replacement: &mut GenerationStartReplacement<'_>,
) {
    std::mem::swap(&mut candidate.world, replacement.world);
    std::mem::swap(&mut candidate.rng, replacement.rng);
    std::mem::swap(&mut candidate.allocators, replacement.allocators);
    std::mem::swap(&mut candidate.brains, replacement.brains);
    std::mem::swap(&mut candidate.fixed_step, replacement.fixed_step);
}

fn swap_initial_run_start_buffers(
    candidate: &mut StateCandidate,
    replacement: &mut InitialRunStartReplacement<'_>,
) {
    std::mem::swap(&mut candidate.world, replacement.world);
    std::mem::swap(&mut candidate.rng, replacement.rng);
    std::mem::swap(&mut candidate.allocators, replacement.allocators);
    std::mem::swap(&mut candidate.fixed_step, replacement.fixed_step);
}

fn validate_generation_start_replacement_contract(
    current: &StateCandidate,
    boundary: &StateCandidate,
    replacement: &GenerationStartReplacement<'_>,
    external_replacements: usize,
    removed_source_leases: usize,
    unavailable_controller_reservations: &[UnavailableControllerReservation],
) -> Result<(), StateError> {
    if boundary.phase != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation) {
        return invalid(
            "generation.start.phase",
            "running construction requires the exact durable generation boundary",
        );
    }
    if !replacement.wall_accumulator_seconds.is_finite()
        || replacement.wall_accumulator_seconds < 0.0
    {
        return invalid(
            "generation.start.wall_accumulator_seconds",
            "scheduler continuation must be finite and non-negative",
        );
    }
    let accounted_source_leases = external_replacements
        .checked_add(removed_source_leases)
        .ok_or(StateError::ArithmeticOverflow {
            context: "generation controller source accounting",
        })?;
    if accounted_source_leases != current.world.controller_leases.len()
        || replacement.world.controller_leases.len() != external_replacements
        || unavailable_controller_reservations.len() != removed_source_leases
    {
        return invalid(
            "generation.start.controller_leases",
            "connected replacements and unavailable reservations do not account for the old leases",
        );
    }
    let mut unavailable_index = 0usize;
    for lease in &current.world.controller_leases {
        if lease.status == ControllerLeaseStatus::Connected {
            continue;
        }
        let record = unavailable_controller_reservations
            .get(unavailable_index)
            .ok_or_else(|| {
                invalid_error(
                    "generation.start.unavailable_controllers",
                    "an old disconnected lease has no retained reclaim outcome",
                )
            })?;
        let expected_reason = match lease.status {
            ControllerLeaseStatus::HoldingLastInput | ControllerLeaseStatus::ReservedNeutral => {
                UnavailableControllerReason::SnakeUnavailable
            }
            ControllerLeaseStatus::NeuralTakeover => UnavailableControllerReason::GraceExpired,
            ControllerLeaseStatus::Connected => unreachable!("connected leases are skipped"),
        };
        if record.source_lease_id != lease.id
            || record.source_snake_id != lease.snake_id
            || record.controller_kind != lease.kind
            || record.scope != lease.scope
            || record.resume_token != lease.resume_token
            || record.disconnected_at_ms != lease.disconnected_at_ms
            || record.grace_expires_at_ms != lease.grace_expires_at_ms
            || record.reason != expected_reason
        {
            return invalid(
                "generation.start.unavailable_controllers",
                "retained reclaim outcome does not match its old controller lease",
            );
        }
        unavailable_index =
            unavailable_index
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "unavailable controller index",
                })?;
    }
    if unavailable_index != unavailable_controller_reservations.len() {
        return invalid(
            "generation.start.unavailable_controllers",
            "retained reclaim outcomes contain an extra or reordered lease",
        );
    }
    if replacement.rng.version != boundary.rng.version
        || replacement.rng.evolution != boundary.rng.evolution
        || (external_replacements == 0
            && replacement.rng.external_controller != boundary.rng.external_controller)
    {
        return invalid(
            "generation.start.rng",
            "generation construction changed evolution RNG or unexplained external RNG",
        );
    }
    validate_running_allocator_continuation(
        &boundary.allocators,
        replacement.allocators,
        external_replacements,
    )?;
    Ok(())
}

/// Exchange only fields a running fixed step is allowed to replace.
fn swap_running_step_buffers(
    candidate: &mut StateCandidate,
    replacement: &mut RunningStepReplacement<'_>,
) {
    std::mem::swap(&mut candidate.world, replacement.world);
    std::mem::swap(&mut candidate.rng, replacement.rng);
    std::mem::swap(&mut candidate.allocators, replacement.allocators);
    std::mem::swap(&mut candidate.brains, replacement.brains);
    std::mem::swap(
        &mut candidate.fixed_step.baseline_lifecycle,
        replacement.baseline_lifecycle,
    );
}

/// Reject immutable or monotonic-continuation changes before any swap.
fn validate_running_replacement_contract(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
) -> Result<(), StateError> {
    if !replacement.mutation.matches(replacement) {
        return invalid(
            "fixed_step.external_replacement",
            "opaque replacement proof does not match the exact staged buffers",
        );
    }
    validate_running_replacement_contract_with_counts(
        candidate,
        replacement,
        replacement.mutation.external_replacements(),
        replacement.mutation.removed_dead_external_leases(),
    )
}

/// Recheck the final sealed delivery result without trusting the now-stale
/// pre-delivery world digest. Only `engine::running_step` can construct the
/// consumed wrapper that reaches this path.
fn validate_resolved_running_replacement_contract(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
    external_replacements: usize,
    removed_dead_external_leases: usize,
) -> Result<(), StateError> {
    validate_running_replacement_contract_with_counts(
        candidate,
        replacement,
        external_replacements,
        removed_dead_external_leases,
    )
}

fn validate_running_replacement_contract_with_counts(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
    external_replacements: usize,
    removed_dead_external_leases: usize,
) -> Result<(), StateError> {
    let expected_elapsed =
        candidate.generation.elapsed_seconds + candidate.config.fixed_step_seconds;
    if !expected_elapsed.is_finite() {
        return invalid(
            "fixed_step.generation_elapsed_seconds",
            "one fixed-delta advance must remain finite",
        );
    }
    if replacement.generation_elapsed_seconds.to_bits() != expected_elapsed.to_bits() {
        return invalid(
            "fixed_step.generation_elapsed_seconds",
            "must advance by exactly one admitted fixed delta",
        );
    }
    if replacement.sensor_generation.best_points_this_generation()
        < candidate
            .fixed_step
            .sensor_generation
            .best_points_this_generation()
    {
        return invalid(
            "fixed_step.sensor_generation",
            "generation best cannot regress",
        );
    }
    if replacement.rng.version != candidate.rng.version
        || replacement.rng.evolution != candidate.rng.evolution
        || (external_replacements == 0
            && replacement.rng.external_controller != candidate.rng.external_controller)
    {
        return invalid(
            "fixed_step.rng",
            "nonterminal steps cannot replace RNG identity or evolution, and only a controlled replacement may advance the external stream",
        );
    }
    validate_running_allocator_continuation(
        &candidate.allocators,
        replacement.allocators,
        external_replacements,
    )?;
    validate_running_brain_continuation(
        candidate,
        replacement,
        external_replacements != 0 || removed_dead_external_leases != 0,
    )?;
    let expected_leases = candidate
        .world
        .controller_leases
        .len()
        .checked_sub(removed_dead_external_leases)
        .ok_or_else(|| {
            invalid_error(
                "fixed_step.controller_leases",
                "removed external lease count exceeds the source lease count",
            )
        })?;
    if replacement.world.controller_leases.len() != expected_leases {
        return invalid(
            "fixed_step.controller_leases",
            "lease count does not match the private controlled-death mutation proof",
        );
    }
    Ok(())
}

/// Enforce the allocator domains that may advance during one nonterminal step.
fn validate_running_allocator_continuation(
    source: &AllocatorState,
    staged: &AllocatorState,
    replacement_count: usize,
) -> Result<(), StateError> {
    let replacements =
        u64::try_from(replacement_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "controlled replacement count",
        })?;
    let expected_brain =
        source
            .next_brain_id
            .checked_add(replacements)
            .ok_or(StateError::ArithmeticOverflow {
                context: "controlled replacement brain continuation",
            })?;
    let expected_lease = source
        .next_controller_lease_id
        .checked_add(replacements)
        .ok_or(StateError::ArithmeticOverflow {
            context: "controlled replacement lease continuation",
        })?;
    let expected_external = source.next_external_id.checked_add(replacements).ok_or(
        StateError::ArithmeticOverflow {
            context: "controlled replacement external continuation",
        },
    )?;
    let frame_replacements =
        u32::try_from(replacement_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "controlled replacement frame count",
        })?;
    let minimum_frame = source
        .next_frame_v1_id
        .checked_add(frame_replacements)
        .ok_or(StateError::ArithmeticOverflow {
            context: "controlled replacement frame continuation",
        })?;
    if staged.version != source.version
        || staged.next_brain_id != expected_brain
        || staged.next_genome_id != source.next_genome_id
        || staged.next_controller_lease_id != expected_lease
        || staged.next_external_id != expected_external
        || staged.next_resurrected_id != source.next_resurrected_id
    {
        return invalid(
            "fixed_step.allocators",
            "nonterminal identity domains do not match the private controlled-death mutation proof",
        );
    }
    if staged.next_entity_id < source.next_entity_id
        || staged.next_frame_v1_id < minimum_frame
        || staged.next_baseline_id < source.next_baseline_id
    {
        return invalid(
            "fixed_step.allocators",
            "monotonic gameplay allocator regressed",
        );
    }
    Ok(())
}

/// Keep all existing brain identities and weights immutable unless an opaque
/// replacement-workspace proof binds the complete staged brain payload.
fn validate_running_brain_continuation(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
    permits_identity_change: bool,
) -> Result<(), StateError> {
    if permits_identity_change {
        return Ok(());
    }
    if replacement.brains.len() != candidate.brains.len() {
        return invalid(
            "fixed_step.brains",
            "a stable-identity running step cannot change brain records",
        );
    }
    for source in &candidate.brains {
        let retained = replacement.brains.iter().any(|staged| {
            source.handle == staged.handle
                && source.owner == staged.owner
                && optional_f32_bits_equal(
                    source.non_population_weights.as_deref(),
                    staged.non_population_weights.as_deref(),
                )
        });
        if retained {
            continue;
        }
        return invalid(
            "fixed_step.brains",
            "a stable-identity running step may change recurrent state but not brain identity or weights",
        );
    }
    Ok(())
}

/// Compare optional packed Float32 values by exact stored bits.
fn optional_f32_bits_equal(left: Option<&[f32]>, right: Option<&[f32]>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| left.to_bits() == right.to_bits())
        }
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// Revalidate every mutable authority component after a staged running step.
///
/// Immutable admission identity, versions, normalized configuration, graph and
/// population weights cannot be supplied by [`RunningStepReplacement`] and
/// remain the values admitted by [`AuthoritativeState::validate_and_own`].
fn validate_running_mutable_state(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
    world_scratch: &mut WorldValidationScratch,
) -> Result<(), StateError> {
    if candidate.phase != AuthorityPhase::Running {
        return invalid("phase", "running-step publication changed authority phase");
    }
    validate_generation(&candidate.generation)?;
    validate_rng_bundle(&candidate.rng, &candidate.config)?;
    validate_allocators(&candidate.allocators)?;
    validate_running_recurrent_state(candidate, graph)?;
    validate_world_with_scratch(candidate, world_scratch)?;
    validate_fixed_step_continuation(candidate)?;
    Ok(())
}

/// Validate the only mutable neural payload after a running-step swap.
///
/// `validate_running_replacement_contract` has already proved either that all
/// static brain fields match the admitted source or that the opaque controlled-
/// death proof binds the exact replacement brain bytes. Population metadata and
/// genome weights are not part of the replacement. Only recurrent length and
/// finiteness still require the general post-swap scan.
fn validate_running_recurrent_state(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    for (index, brain) in candidate.brains.iter().enumerate() {
        if brain.recurrent.len() != graph.total_state_size {
            return Err(StateError::RecurrentLength {
                handle: brain.handle,
                expected: graph.total_state_size,
                actual: brain.recurrent.len(),
            });
        }
        validate_f32_slice("brains.recurrent", index, &brain.recurrent)?;
    }
    Ok(())
}

/// Decode one already-versioned SHA-256 identity without allocating.
fn decode_sha256_identity(field: &'static str, identity: &str) -> Result<[u8; 32], StateError> {
    let Some(hex) = identity.strip_prefix("sha256:") else {
        return invalid(field, "must use the sha256:<64 lowercase hex> form");
    };
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return invalid(field, "must contain exactly 64 lowercase hex digits");
    }
    let mut output = [0_u8; 32];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let high = lowercase_hex_nibble(pair[0])
            .ok_or_else(|| invalid_error(field, "must contain lowercase hexadecimal digits"))?;
        let low = lowercase_hex_nibble(pair[1])
            .ok_or_else(|| invalid_error(field, "must contain lowercase hexadecimal digits"))?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

/// Convert one lowercase hexadecimal byte to its numeric nibble.
const fn lowercase_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
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
    /// Reusable validation storage could not reserve its checked size.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// A prepared fixed step no longer names the current authority/operation.
    StaleFixedStep { field: PhysicsStepKeyField },
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
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "could not reserve {required} entries for {context}"
            ),
            Self::StaleFixedStep { field } => {
                write!(formatter, "stale fixed-step proposal: {field:?} changed")
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
    world_scratch: &mut WorldValidationScratch,
) -> Result<(), StateError> {
    validate_admission_header(candidate, graph, policy)?;
    validate_generation(&candidate.generation)?;
    validate_rng_bundle(&candidate.rng, &candidate.config)?;
    validate_allocators(&candidate.allocators)?;
    validate_population(candidate, graph)?;
    validate_world_with_scratch(candidate, world_scratch)?;
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
        validate_baseline_rng(baseline.slot, &baseline.state)?;
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

/// Validate one dense baseline stream without formatting its success-path label.
fn validate_baseline_rng(slot: u32, state: &SerializedRngState) -> Result<(), StateError> {
    StatefulRng::from_state(state)
        .map(|_| ())
        .map_err(|error: RngError| StateError::InvalidRng {
            stream: format!("baseline:{slot}"),
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

fn validate_world_with_scratch(
    candidate: &StateCandidate,
    scratch: &mut WorldValidationScratch,
) -> Result<(), StateError> {
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

    let entity_capacity = candidate
        .world
        .snakes
        .len()
        .checked_add(candidate.world.pellets.len())
        .ok_or(StateError::ArithmeticOverflow {
            context: "world validation entity count",
        })?;
    scratch.clear();
    reserve_validation(&mut scratch.entity_ids, entity_capacity, "world entity IDs")?;
    reserve_validation(
        &mut scratch.snake_ids,
        candidate.world.snakes.len(),
        "world snake IDs",
    )?;
    reserve_validation(
        &mut scratch.public_ids,
        candidate.world.snakes.len(),
        "frame-v1 snake IDs",
    )?;
    reserve_validation(
        &mut scratch.evolved_slots,
        candidate.config.population_count,
        "world population slots",
    )?;
    reserve_validation(
        &mut scratch.world_brains,
        candidate.world.snakes.len(),
        "world brain handles",
    )?;
    reserve_validation(
        &mut scratch.baseline_slots,
        candidate.config.baseline_count,
        "world baseline slots",
    )?;
    reserve_validation(
        &mut scratch.ranges,
        candidate.world.snakes.len(),
        "world body ranges",
    )?;
    let entity_ids = &mut scratch.entity_ids;
    let snake_ids = &mut scratch.snake_ids;
    let public_ids = &mut scratch.public_ids;
    let evolved_slots = &mut scratch.evolved_slots;
    let world_brains = &mut scratch.world_brains;
    let baseline_slots = &mut scratch.baseline_slots;
    let ranges = &mut scratch.ranges;
    let mut max_general_id = 0u64;
    let mut max_external_id = 0u64;
    let mut max_baseline_id = 0u64;
    let mut max_resurrected_id = 0u64;
    let mut max_public_id = 0u32;
    for (index, snake) in candidate.world.snakes.iter().enumerate() {
        if snake.id == 0 || snake.id == u64::MAX {
            return Err(StateError::DuplicateId {
                kind: "snake",
                id: snake.id,
            });
        }
        entity_ids.push(snake.id);
        snake_ids.push(snake.id);
        if snake.frame_v1_id == 0 || snake.frame_v1_id > FRAME_V1_MAX_EXACT_ID {
            return Err(StateError::DuplicateId {
                kind: "frame-v1 snake",
                id: u64::from(snake.frame_v1_id),
            });
        }
        public_ids.push(snake.frame_v1_id);
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
                evolved_slots.push(slot);
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
            world_brains.push(handle);
        }
        if let Some(slot) = snake.baseline_slot {
            if candidate
                .rng
                .baselines
                .get(slot as usize)
                .is_none_or(|baseline| baseline.slot != slot)
            {
                return invalid(
                    "world.snakes.baseline_slot",
                    "baseline slot has no matching RNG stream",
                );
            }
            baseline_slots.push(slot);
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

    if let Some(id) = first_duplicate_after_sort(snake_ids) {
        return Err(StateError::DuplicateId { kind: "snake", id });
    }
    if let Some(id) = first_duplicate_after_sort(public_ids) {
        return Err(StateError::DuplicateId {
            kind: "frame-v1 snake",
            id: u64::from(id),
        });
    }
    if let Some(slot) = first_duplicate_after_sort(evolved_slots) {
        return Err(StateError::DuplicateId {
            kind: "world population slot",
            id: u64::from(slot),
        });
    }
    if let Some(handle) = first_duplicate_after_sort(world_brains) {
        return Err(StateError::DuplicateBrainHandle(handle));
    }
    if first_duplicate_after_sort(baseline_slots).is_some() {
        return invalid("world.snakes.baseline_slot", "baseline slot must be unique");
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
        if pellet.id == 0 || pellet.id >= EXTERNAL_ENTITY_ID_START {
            return Err(StateError::DuplicateId {
                kind: "world entity",
                id: pellet.id,
            });
        }
        entity_ids.push(pellet.id);
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
            .is_some_and(|owner| snake_ids.binary_search(&owner).is_err())
        {
            return invalid("world.pellets.owner", "owner does not identify a snake");
        }
    }
    if let Some(id) = first_duplicate_after_sort(entity_ids) {
        return Err(StateError::DuplicateId {
            kind: "world entity",
            id,
        });
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

    let lease_count = candidate.world.controller_leases.len();
    reserve_validation(&mut scratch.lease_ids, lease_count, "controller lease IDs")?;
    reserve_validation(
        &mut scratch.lease_snakes,
        lease_count,
        "controller lease snake IDs",
    )?;
    reserve_validation(
        &mut scratch.resume_token_order,
        lease_count,
        "controller resume-token order",
    )?;
    reserve_validation(
        &mut scratch.connection_ids,
        lease_count,
        "controller connection IDs",
    )?;
    for (lease_index, lease) in candidate.world.controller_leases.iter().enumerate() {
        let snake = candidate
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == lease.snake_id && snake.alive)
            .ok_or(StateError::UnknownLeaseSnake(lease.snake_id))?;
        validate_lease(lease, snake, candidate)?;
        scratch.lease_ids.push(lease.id);
        scratch.lease_snakes.push(lease.snake_id);
        scratch.resume_token_order.push(lease_index);
        if let Some(connection_id) = lease.connection_id {
            scratch.connection_ids.push(connection_id);
        }
    }
    if let Some(id) = first_duplicate_after_sort(&mut scratch.lease_ids) {
        return Err(StateError::DuplicateLeaseId(id));
    }
    if let Some(id) = first_duplicate_after_sort(&mut scratch.lease_snakes) {
        return Err(StateError::DuplicateLeaseSnake(id));
    }
    scratch.resume_token_order.sort_unstable_by(|left, right| {
        candidate.world.controller_leases[*left]
            .resume_token
            .cmp(&candidate.world.controller_leases[*right].resume_token)
    });
    if scratch.resume_token_order.windows(2).any(|pair| {
        candidate.world.controller_leases[pair[0]].resume_token
            == candidate.world.controller_leases[pair[1]].resume_token
    }) {
        return invalid("controller_lease.resume_token", "token must be unique");
    }
    if first_duplicate_after_sort(&mut scratch.connection_ids).is_some() {
        return invalid(
            "controller_lease.connection_id",
            "connection must own at most one lease",
        );
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

/// Sort one temporary identity list and return its first repeated value.
fn first_duplicate_after_sort<T: Copy + Ord>(values: &mut [T]) -> Option<T> {
    values.sort_unstable();
    values
        .windows(2)
        .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
}

fn reserve_validation<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), StateError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| StateError::AllocationFailed { context, required })?;
    }
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
    Err(invalid_error(field, reason))
}

fn invalid_error(field: &'static str, reason: &str) -> StateError {
    StateError::InvalidField {
        field,
        reason: reason.to_owned(),
    }
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
    use crate::engine::checkpoint::{CheckpointLimits, CheckpointOperationId};
    use crate::engine::contract::{EngineInit, InboundLimits, OutputLimits};
    use crate::engine::graph::{
        GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::movement::{MovementConfig, MovementWorkspace};
    use crate::engine::queues::NoopWakeSink;
    use crate::engine::rng::{derive_seed, StatefulRng};
    use crate::engine::runtime::EngineRuntime;
    use crate::engine::step_config::RunningStepWorkLimits;
    use crate::engine::{
        ExternalDeliveryEventKind, ExternalDeliveryResult, ExternalDeliveryState,
        ExternalDeliveryStatus, FixedStepScheduler, FixedStepSchedulerPolicy,
        GenerationReassignmentProgress, GenerationTransitionReason, RunningStepCoordinator,
        RunningStepError, RunningStepInputs, RunningStepProgress, SchedulerError,
        SchedulerReadiness, SchedulerServiceMode,
    };
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64 as TestAtomicU64, Ordering as TestOrdering};

    static NEXT_GENERATION_HANDOFF_DIRECTORY: TestAtomicU64 = TestAtomicU64::new(1);

    struct GenerationHandoffDirectory {
        path: PathBuf,
    }

    impl GenerationHandoffDirectory {
        fn new() -> Self {
            let sequence = NEXT_GENERATION_HANDOFF_DIRECTORY.fetch_add(1, TestOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "slither-generation-handoff-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("generation handoff directory must be unique");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for GenerationHandoffDirectory {
        fn drop(&mut self) {
            let expected_prefix = format!("slither-generation-handoff-{}-", std::process::id());
            let is_owned = self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&expected_prefix));
            if is_owned {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

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

    fn generation_handoff_checkpoint_limits() -> CheckpointLimits {
        CheckpointLimits {
            max_archive_bytes: 64 * 1024 * 1024,
            max_manifest_bytes: 1024 * 1024,
            max_state_bytes: 4 * 1024 * 1024,
            max_graph_bytes: 4 * 1024 * 1024,
            max_population_index_bytes: 4 * 1024 * 1024,
            max_population_count: 300,
            max_setting_count: 512,
            max_baseline_rng_count: 128,
            max_string_bytes: 256 * 1024,
            max_total_string_bytes: 4 * 1024 * 1024,
            max_weight_floats: 8_000_000,
            max_recurrent_floats: 1_000_000,
            max_numeric_stored_bytes: 64 * 1024 * 1024,
            max_numeric_candidate_bytes: 64 * 1024 * 1024,
            max_total_decoded_bytes: 128 * 1024 * 1024,
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

    fn running_candidate(graph: &CompiledGraph) -> StateCandidate {
        let mut running = candidate(graph, 1);
        running.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut running, 0, 1, 1, WorldPoint { x: 10.0, y: 20.0 });
        running
    }

    fn complete_running_candidate(graph: &CompiledGraph) -> StateCandidate {
        let mut running = running_candidate(graph);
        running.config.settings = crate::engine::step_config::tests::default_settings(1, 0);
        running.config.settings_schema_sha256 =
            normalized_settings_schema_hash(&running.config.settings)
                .expect("complete running settings schema must hash");
        running.config.world_radius = 3_500.0;
        running.config.max_world_snakes = 32;
        running.config.max_non_population_brains = 31;
        running.config.max_pellets = 10_000;
        running.config.spatial_index_bytes = 8 * 1024 * 1024;
        running.config.worker_scratch_bytes = 8 * 1024 * 1024;

        let head = running.world.snakes[0].position;
        running.world.body_points = (0..5)
            .map(|offset| WorldPoint {
                x: head.x - (offset as f64 * 7.5),
                y: head.y,
            })
            .collect();
        let snake = &mut running.world.snakes[0];
        snake.body = BodyRange { start: 0, len: 5 };
        snake.target_length = 5.0;
        snake.radius = 9.0;
        snake.speed = 165.0;
        refresh_config_hash(&mut running);
        running
    }

    fn complete_running_policy(
        candidate: &StateCandidate,
        memory_ceiling_bytes: usize,
    ) -> StateAdmissionPolicy {
        let mut admission = policy(memory_ceiling_bytes);
        admission.expected_settings_schema_sha256 = candidate.config.settings_schema_sha256.clone();
        admission
    }

    fn set_complete_setting(
        candidate: &mut StateCandidate,
        path: &str,
        value: NormalizedSettingValue,
    ) {
        candidate
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == path)
            .expect("complete running setting must exist")
            .value = value;
    }

    fn own_complete_running(
        candidate: StateCandidate,
        graph: Arc<GraphBundle>,
    ) -> AuthoritativeState {
        let estimated = estimate_state_memory(&candidate, &graph)
            .expect("complete running fixture memory must estimate");
        let memory_ceiling = estimated
            .total_bytes
            .checked_add(64 * 1024 * 1024)
            .expect("test memory ceiling must fit");
        let admission = complete_running_policy(&candidate, memory_ceiling);
        AuthoritativeState::validate_and_own(candidate, graph, &admission)
            .expect("complete running fixture must admit")
    }

    struct RunningStepBuffers {
        world: WorldState,
        rng: RngStateBundle,
        allocators: AllocatorState,
        brains: Vec<BrainRuntimeState>,
        baseline_lifecycle: BaselineLifecycleState,
        ambient_pellet_accumulator: f64,
        sensor_generation: SensorGenerationState,
        generation_elapsed_seconds: f64,
        wall_accumulator_seconds: f64,
    }

    impl RunningStepBuffers {
        fn from_state(state: &StateCandidate) -> Self {
            Self {
                world: state.world.clone(),
                rng: state.rng.clone(),
                allocators: state.allocators.clone(),
                brains: state.brains.clone(),
                baseline_lifecycle: state.fixed_step.baseline_lifecycle.clone(),
                ambient_pellet_accumulator: state.fixed_step.ambient_pellet_accumulator,
                sensor_generation: state.fixed_step.sensor_generation,
                generation_elapsed_seconds: state.generation.elapsed_seconds + 1.0 / 60.0,
                wall_accumulator_seconds: state.generation.wall_accumulator_seconds,
            }
        }

        fn replacement(&mut self, key: PhysicsStepKey) -> RunningStepReplacement<'_> {
            RunningStepReplacement {
                key,
                world: &mut self.world,
                rng: &mut self.rng,
                allocators: &mut self.allocators,
                brains: &mut self.brains,
                baseline_lifecycle: &mut self.baseline_lifecycle,
                ambient_pellet_accumulator: self.ambient_pellet_accumulator,
                sensor_generation: self.sensor_generation,
                generation_elapsed_seconds: self.generation_elapsed_seconds,
                wall_accumulator_seconds: self.wall_accumulator_seconds,
                mutation: RunningStepMutationContract::default(),
            }
        }
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

    fn push_connected_external_fixture(
        candidate: &mut StateCandidate,
        graph: &CompiledGraph,
        snake_id: u64,
        frame_v1_id: u32,
        lease_id: u64,
        connection_id: u64,
        position: WorldPoint,
    ) {
        push_external_snake(candidate, graph, snake_id, frame_v1_id, position);
        let snake_index = candidate.world.snakes.len() - 1;
        let body_start = candidate.world.snakes[snake_index].body.start;
        candidate
            .world
            .body_points
            .extend((1..5).map(|offset| WorldPoint {
                x: position.x - (offset as f64 * 7.5),
                y: position.y,
            }));
        let snake = &mut candidate.world.snakes[snake_index];
        snake.body = BodyRange {
            start: body_start,
            len: 5,
        };
        snake.target_length = 5.0;
        snake.radius = 9.0;
        snake.speed = 165.0;
        snake.points = 10.0;
        snake.delivered_observation_points = 2.0;
        candidate.world.controller_leases.push(connected_lease(
            lease_id,
            snake_id,
            connection_id,
            &format!("external-delivery-{lease_id}"),
        ));
        candidate.allocators.next_controller_lease_id = candidate
            .allocators
            .next_controller_lease_id
            .max(lease_id + 1);
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
    fn running_step_publication_swaps_every_mutable_continuation_atomically() {
        let graph = default_graph();
        let source = running_candidate(&graph);
        let source_snapshot = source.clone();
        let mut authority = own(source, graph, usize::MAX).expect("running state must admit");
        let key = authority
            .begin_running_step()
            .expect("running step must begin");

        let mut world = authority.state().world.clone();
        world.snakes[0].position = WorldPoint { x: 11.0, y: 20.0 };
        world.body_points[0] = world.snakes[0].position;
        let mut rng = authority.state().rng.clone();
        let mut world_stream = StatefulRng::from_state(&rng.world).expect("source RNG must decode");
        let _ = world_stream.next_f64();
        rng.world = world_stream.export_state();
        let mut allocators = authority.state().allocators.clone();
        allocators.next_entity_id += 1;
        let mut brains = authority.state().brains.clone();
        brains[0].recurrent[0] = 0.25;
        let mut baseline_lifecycle = authority.state().fixed_step.baseline_lifecycle.clone();
        let sensor_generation = SensorGenerationState::new();

        let publication = authority
            .publish_running_step(RunningStepReplacement {
                key,
                world: &mut world,
                rng: &mut rng,
                allocators: &mut allocators,
                brains: &mut brains,
                baseline_lifecycle: &mut baseline_lifecycle,
                ambient_pellet_accumulator: 0.75,
                sensor_generation,
                generation_elapsed_seconds: 1.0 / 60.0,
                wall_accumulator_seconds: 0.125,
                mutation: RunningStepMutationContract::default(),
            })
            .expect("complete valid step must publish");

        assert_eq!(publication.key, key);
        assert_eq!(publication.completed_step, 1);
        assert_eq!(publication.memory, authority.memory_estimate());
        assert_eq!(authority.state().generation.completed_step, 1);
        assert_eq!(authority.state().generation.elapsed_seconds, 1.0 / 60.0);
        assert_eq!(authority.state().generation.wall_accumulator_seconds, 0.125);
        assert_eq!(authority.state().world.snakes[0].position.x, 11.0);
        assert_eq!(authority.state().allocators.next_entity_id, 3);
        assert_eq!(authority.state().brains[0].recurrent[0], 0.25);
        assert_eq!(
            authority.state().fixed_step.ambient_pellet_accumulator,
            0.75
        );
        assert_eq!(
            authority.state().fixed_step.sensor_generation,
            sensor_generation
        );

        assert_eq!(world, source_snapshot.world);
        assert_eq!(rng, source_snapshot.rng);
        assert_eq!(allocators, source_snapshot.allocators);
        assert_eq!(brains, source_snapshot.brains);
        assert_eq!(
            baseline_lifecycle,
            source_snapshot.fixed_step.baseline_lifecycle
        );
    }

    #[test]
    fn running_step_rejects_every_stale_identity_before_any_swap() {
        let graph = default_graph();
        let mut authority =
            own(running_candidate(&graph), graph, usize::MAX).expect("running state must admit");
        let key = authority
            .begin_running_step()
            .expect("running step must begin");
        let source = authority.state().clone();
        let mut buffers = RunningStepBuffers::from_state(authority.state());
        let stale = [
            (
                PhysicsStepKey::new(
                    key.world_epoch() + 1,
                    key.generation(),
                    key.source_completed_step(),
                    key.population_epoch(),
                    key.config_revision(),
                    key.config_hash(),
                    key.operation_epoch(),
                ),
                PhysicsStepKeyField::WorldEpoch,
            ),
            (
                PhysicsStepKey::new(
                    key.world_epoch(),
                    key.generation() + 1,
                    key.source_completed_step(),
                    key.population_epoch(),
                    key.config_revision(),
                    key.config_hash(),
                    key.operation_epoch(),
                ),
                PhysicsStepKeyField::Generation,
            ),
            (
                PhysicsStepKey::new(
                    key.world_epoch(),
                    key.generation(),
                    key.source_completed_step() + 1,
                    key.population_epoch(),
                    key.config_revision(),
                    key.config_hash(),
                    key.operation_epoch(),
                ),
                PhysicsStepKeyField::SourceCompletedStep,
            ),
            (
                PhysicsStepKey::new(
                    key.world_epoch(),
                    key.generation(),
                    key.source_completed_step(),
                    key.population_epoch() + 1,
                    key.config_revision(),
                    key.config_hash(),
                    key.operation_epoch(),
                ),
                PhysicsStepKeyField::PopulationEpoch,
            ),
            (
                PhysicsStepKey::new(
                    key.world_epoch(),
                    key.generation(),
                    key.source_completed_step(),
                    key.population_epoch(),
                    key.config_revision() + 1,
                    key.config_hash(),
                    key.operation_epoch(),
                ),
                PhysicsStepKeyField::ConfigRevision,
            ),
            (
                PhysicsStepKey::new(
                    key.world_epoch(),
                    key.generation(),
                    key.source_completed_step(),
                    key.population_epoch(),
                    key.config_revision(),
                    [0x55; 32],
                    key.operation_epoch(),
                ),
                PhysicsStepKeyField::ConfigHash,
            ),
            (
                PhysicsStepKey::new(
                    key.world_epoch(),
                    key.generation(),
                    key.source_completed_step(),
                    key.population_epoch(),
                    key.config_revision(),
                    key.config_hash(),
                    key.operation_epoch() + 1,
                ),
                PhysicsStepKeyField::OperationEpoch,
            ),
        ];

        for (stale_key, expected_field) in stale {
            assert_eq!(
                authority.publish_running_step(buffers.replacement(stale_key)),
                Err(StateError::StaleFixedStep {
                    field: expected_field
                })
            );
            assert_eq!(authority.state(), &source);
            assert_eq!(buffers.world, source.world);
            assert_eq!(buffers.brains, source.brains);
        }

        let newer_key = authority
            .begin_running_step()
            .expect("newer attempt must begin");
        assert_eq!(
            authority.publish_running_step(buffers.replacement(key)),
            Err(StateError::StaleFixedStep {
                field: PhysicsStepKeyField::OperationEpoch
            })
        );
        authority
            .publish_running_step(buffers.replacement(newer_key))
            .expect("current exact key must publish");
    }

    #[test]
    fn running_step_rejects_immutable_or_regressing_continuations_before_swap() {
        let graph = default_graph();
        let mut source = running_candidate(&graph);
        push_external_snake(
            &mut source,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            WorldPoint { x: 30.0, y: 40.0 },
        );
        source.world.snakes[0].points = 5.0;
        source
            .fixed_step
            .sensor_generation
            .update_after_step(&source.world)
            .expect("source generation best must update");
        let mut authority = own(source.clone(), graph, usize::MAX)
            .expect("running source with external brain must admit");
        let key = authority
            .begin_running_step()
            .expect("running step must begin");

        let mut wrong_elapsed = RunningStepBuffers::from_state(authority.state());
        wrong_elapsed.generation_elapsed_seconds += 0.001;
        assert!(matches!(
            authority.publish_running_step(wrong_elapsed.replacement(key)),
            Err(StateError::InvalidField {
                field: "fixed_step.generation_elapsed_seconds",
                ..
            })
        ));

        let mut regressed_sensor = RunningStepBuffers::from_state(authority.state());
        regressed_sensor.sensor_generation.reset();
        assert!(matches!(
            authority.publish_running_step(regressed_sensor.replacement(key)),
            Err(StateError::InvalidField {
                field: "fixed_step.sensor_generation",
                ..
            })
        ));

        let mut wrong_rng = RunningStepBuffers::from_state(authority.state());
        let mut evolution = StatefulRng::from_state(&wrong_rng.rng.evolution)
            .expect("evolution stream must decode");
        let _ = evolution.next_f64();
        wrong_rng.rng.evolution = evolution.export_state();
        assert!(matches!(
            authority.publish_running_step(wrong_rng.replacement(key)),
            Err(StateError::InvalidField {
                field: "fixed_step.rng",
                ..
            })
        ));

        let mut wrong_allocator = RunningStepBuffers::from_state(authority.state());
        wrong_allocator.allocators.next_genome_id += 1;
        assert!(matches!(
            authority.publish_running_step(wrong_allocator.replacement(key)),
            Err(StateError::InvalidField {
                field: "fixed_step.allocators",
                ..
            })
        ));

        let mut wrong_weights = RunningStepBuffers::from_state(authority.state());
        let external_weights = wrong_weights.brains[1]
            .non_population_weights
            .as_mut()
            .expect("external brain must own weights");
        external_weights[0] = 0.5;
        assert!(matches!(
            authority.publish_running_step(wrong_weights.replacement(key)),
            Err(StateError::InvalidField {
                field: "fixed_step.brains",
                ..
            })
        ));

        assert_eq!(authority.state(), &source);
        assert_eq!(authority.state().generation.completed_step, 0);
    }

    #[test]
    fn running_step_validation_and_memory_failure_restore_authority_and_scratch() {
        let graph = default_graph();
        let source = running_candidate(&graph);
        let exact_ceiling = estimate_state_memory(&source, &graph)
            .expect("source memory must estimate")
            .total_bytes;
        let mut authority =
            own(source.clone(), graph, exact_ceiling).expect("source must fit its exact estimate");
        let source_memory = authority.memory_estimate();

        let invalid_key = authority
            .begin_running_step()
            .expect("invalid attempt must begin");
        let mut invalid = RunningStepBuffers::from_state(authority.state());
        invalid.world.snakes[0].position.x = f64::NAN;
        assert!(matches!(
            authority.publish_running_step(invalid.replacement(invalid_key)),
            Err(StateError::NonFinite {
                field: "world.snakes.position",
                ..
            })
        ));
        assert_eq!(authority.state(), &source);
        assert_eq!(authority.memory_estimate(), source_memory);
        assert!(invalid.world.snakes[0].position.x.is_nan());

        let invalid_recurrent_key = authority
            .begin_running_step()
            .expect("invalid recurrent attempt must begin");
        let mut invalid_recurrent = RunningStepBuffers::from_state(authority.state());
        invalid_recurrent.brains[0].recurrent[0] = f32::NAN;
        assert!(matches!(
            authority.publish_running_step(invalid_recurrent.replacement(invalid_recurrent_key)),
            Err(StateError::NonFinite {
                field: "brains.recurrent",
                ..
            })
        ));
        assert_eq!(authority.state(), &source);
        assert_eq!(authority.memory_estimate(), source_memory);
        assert!(invalid_recurrent.brains[0].recurrent[0].is_nan());

        let oversized_key = authority
            .begin_running_step()
            .expect("oversized attempt must begin");
        let mut oversized = RunningStepBuffers::from_state(authority.state());
        oversized
            .world
            .pellets
            .try_reserve_exact(100_000)
            .expect("test reserve must succeed");
        let oversized_capacity = oversized.world.pellets.capacity();
        assert!(matches!(
            authority.publish_running_step(oversized.replacement(oversized_key)),
            Err(StateError::MemoryCeilingExceeded { .. })
        ));
        assert_eq!(authority.state(), &source);
        assert_eq!(authority.memory_estimate(), source_memory);
        assert_eq!(oversized.world.pellets.capacity(), oversized_capacity);

        let valid_key = authority
            .begin_running_step()
            .expect("valid retry must begin");
        let mut valid = RunningStepBuffers::from_state(authority.state());
        authority
            .publish_running_step(valid.replacement(valid_key))
            .expect("later valid step must publish");
        assert_eq!(authority.state().generation.completed_step, 1);
    }

    #[test]
    fn running_step_cannot_begin_from_a_generation_boundary() {
        let graph = default_graph();
        let mut boundary =
            own(candidate(&graph, 1), graph, usize::MAX).expect("generation boundary must admit");
        assert!(matches!(
            boundary.begin_running_step(),
            Err(StateError::InvalidField { field: "phase", .. })
        ));
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
    fn complete_nonterminal_coordinator_publishes_every_step_continuation_once() {
        let graph = default_graph();
        let candidate = complete_running_candidate(&graph);
        let source = candidate.clone();
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("complete admitted config must construct the coordinator");

        let outcome = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.125,
                },
            )
            .expect("ordinary nonterminal step must publish")
        {
            RunningStepProgress::Published(outcome) => outcome,
            RunningStepProgress::ExternalDeliveryPending(_) => {
                panic!("ordinary neural step must not require external delivery")
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("ordinary neural step must not end the generation")
            }
        };

        assert_eq!(outcome.publication.completed_step, 1);
        assert_eq!(outcome.diagnostics.physics.expected_substeps, 3);
        assert_eq!(outcome.diagnostics.physics.completed_substeps, 3);
        assert_eq!(coordinator.last_wall_now_ms(), Some(100));
        assert_eq!(authority.state().generation.completed_step, 1);
        assert_eq!(
            authority.state().generation.elapsed_seconds.to_bits(),
            (1.0_f64 / 60.0).to_bits()
        );
        assert_eq!(
            authority
                .state()
                .generation
                .wall_accumulator_seconds
                .to_bits(),
            0.125_f64.to_bits()
        );
        assert_ne!(authority.state().world, source.world);
        assert_ne!(authority.state().rng, source.rng);
        assert_ne!(authority.state().brains, source.brains);
        assert_eq!(authority.state().identity, source.identity);
        assert_eq!(authority.state().population, source.population);
        assert_eq!(authority.state().phase, AuthorityPhase::Running);
    }

    #[test]
    fn scheduler_requires_a_fresh_command_service_boundary_before_each_overdue_step() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        set_complete_setting(
            &mut candidate,
            "simSpeed",
            NormalizedSettingValue::Float(12.0),
        );
        candidate.config.requested_sim_speed = 12.0;
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut running = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("12x fixture must construct the running coordinator");
        let mut scheduler = FixedStepScheduler::try_new(
            &authority,
            FixedStepSchedulerPolicy::provisional_defaults(),
        )
        .expect("running authority must construct the scheduler");

        scheduler
            .reset_wall_clock(&authority, 1_000)
            .expect("async initialization must reset the scheduler clock");
        let initialized = scheduler.diagnostics();
        for repeated_wall_ms in [2_000, 999] {
            assert_eq!(
                scheduler.reset_wall_clock(&authority, repeated_wall_ms),
                Err(SchedulerError::ClockAlreadyInitialized),
                "neither forward nor backward rebasing may hide or manufacture debt"
            );
            assert_eq!(
                scheduler.diagnostics(),
                initialized,
                "a rejected rebase must be atomic"
            );
        }
        let readiness = scheduler
            .service_after_command_drain(&authority, 1_017, SchedulerServiceMode::Interactive)
            .expect("one explicit service boundary must observe current wall debt");
        assert!(matches!(
            readiness,
            SchedulerReadiness::StepDue { due_steps: 12, .. }
        ));
        let step = scheduler
            .prepare_due_step(&authority)
            .expect("one serviced step must become available");
        assert_eq!(step.wall_now_ms(), 1_017);
        assert_eq!(step.service_mode(), SchedulerServiceMode::Interactive);
        let progress = running
            .advance_nonterminal(&mut authority, step.running_step_inputs())
            .expect("the scheduled nonterminal step must publish");
        let publication = match progress {
            RunningStepProgress::Published(outcome) => outcome.publication,
            RunningStepProgress::ExternalDeliveryPending(_) => {
                panic!("the neural-only fixture must not wait for Node delivery")
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("the scheduled fixture must remain nonterminal")
            }
        };

        let key = publication.key;
        let mut different_hash = key.config_hash();
        different_hash[0] ^= 1;
        let forged_keys = [
            PhysicsStepKey::new(
                key.world_epoch().checked_add(1).unwrap(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation().checked_add(1).unwrap(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step().checked_add(1).unwrap(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch().checked_add(1).unwrap(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision().checked_add(1).unwrap(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                different_hash,
                key.operation_epoch(),
            ),
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch().checked_add(1).unwrap(),
            ),
        ];
        let pending = scheduler.diagnostics();
        for forged_key in forged_keys {
            assert_eq!(
                scheduler.commit_step(
                    &authority,
                    step,
                    RunningStepPublication {
                        key: forged_key,
                        ..publication
                    },
                ),
                Err(SchedulerError::PublicationMismatch {
                    field: "complete authority publication identity",
                })
            );
            assert_eq!(
                scheduler.diagnostics(),
                pending,
                "a forged key must not retire or mutate the pending ticket"
            );
        }
        let mut wrong_memory = publication.memory;
        wrong_memory.total_bytes = wrong_memory.total_bytes.checked_add(1).unwrap();
        assert_eq!(
            scheduler.commit_step(
                &authority,
                step,
                RunningStepPublication {
                    memory: wrong_memory,
                    ..publication
                },
            ),
            Err(SchedulerError::PublicationMismatch {
                field: "complete authority publication identity",
            })
        );
        assert_eq!(scheduler.diagnostics(), pending);

        scheduler
            .commit_step(&authority, step, publication)
            .expect("the exact publication must retire the scheduler ticket");

        let diagnostics = scheduler.diagnostics();
        assert_eq!(diagnostics.completed_steps, 1);
        assert_eq!(diagnostics.command_service_boundaries, 1);
        assert_eq!(diagnostics.interactive_service_boundaries, 1);
        assert!(diagnostics.pending_simulation_seconds > candidate_fixed_dt(&authority));
        assert_eq!(
            authority
                .state()
                .generation
                .wall_accumulator_seconds
                .to_bits(),
            step.wall_accumulator_after_step().to_bits()
        );
        assert_eq!(
            scheduler.prepare_due_step(&authority),
            Err(SchedulerError::CommandServiceRequired)
        );

        let second_readiness = scheduler
            .service_after_command_drain(&authority, 1_017, SchedulerServiceMode::Interactive)
            .expect("a second overdue step requires a second service boundary");
        assert!(matches!(
            second_readiness,
            SchedulerReadiness::StepDue { due_steps: 11, .. }
        ));
        let rejected = scheduler
            .prepare_due_step(&authority)
            .expect("the newly serviced second step must be available");
        let debt_before_rejection = scheduler.diagnostics().pending_simulation_seconds;
        scheduler
            .reject_step(&authority, rejected)
            .expect("a failed attempt must retain its scheduling debt");
        assert_eq!(
            scheduler.diagnostics().pending_simulation_seconds.to_bits(),
            debt_before_rejection.to_bits()
        );
        assert_eq!(authority.state().generation.completed_step, 1);
        assert_eq!(
            scheduler.prepare_due_step(&authority),
            Err(SchedulerError::CommandServiceRequired)
        );
    }

    #[test]
    fn scheduler_holds_debt_while_external_delivery_blocks_publication() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut running = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("external fixture must construct the running coordinator");
        let mut scheduler = FixedStepScheduler::try_new(
            &authority,
            FixedStepSchedulerPolicy::provisional_defaults(),
        )
        .expect("external fixture must construct the scheduler");
        scheduler.reset_wall_clock(&authority, 100).unwrap();
        assert!(matches!(
            scheduler
                .service_after_command_drain(&authority, 117, SchedulerServiceMode::Interactive,)
                .unwrap(),
            SchedulerReadiness::StepDue { due_steps: 1, .. }
        ));
        let step = scheduler.prepare_due_step(&authority).unwrap();
        let debt_before_publication = scheduler.diagnostics().pending_simulation_seconds;
        let event = match running
            .advance_nonterminal(&mut authority, step.running_step_inputs())
            .expect("complete external step must await a local send result")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
            RunningStepProgress::Published(_) => panic!("external delivery must block the swap"),
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("external delivery fixture must remain nonterminal")
            }
        };
        assert_eq!(authority.state(), &source);
        assert!(scheduler.diagnostics().step_pending);
        assert_eq!(
            scheduler.diagnostics().pending_simulation_seconds.to_bits(),
            debt_before_publication.to_bits()
        );
        assert_eq!(
            scheduler.service_after_command_drain(
                &authority,
                118,
                SchedulerServiceMode::Interactive,
            ),
            Err(SchedulerError::StepPending)
        );

        let accepted = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: true,
        };
        let publication = match running
            .submit_external_delivery_results(&mut authority, &[accepted])
            .expect("matching local result must publish")
            .state
        {
            ExternalDeliveryState::Published(outcome) => outcome.publication,
            _ => panic!("the sole resolved event must publish the step"),
        };
        scheduler
            .commit_step(&authority, step, publication)
            .expect("the delayed exact publication must consume one step of debt");
        assert!(!scheduler.diagnostics().step_pending);
        assert_eq!(scheduler.diagnostics().completed_steps, 1);
        assert_eq!(authority.state().generation.completed_step, 1);
        assert_eq!(
            authority
                .state()
                .generation
                .wall_accumulator_seconds
                .to_bits(),
            step.wall_accumulator_after_step().to_bits()
        );
    }

    #[test]
    fn scheduler_holds_terminal_ticket_and_debt_until_explicit_transition_discard() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut running = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal fixture must construct the running coordinator");
        let mut scheduler = FixedStepScheduler::try_new(
            &authority,
            FixedStepSchedulerPolicy::provisional_defaults(),
        )
        .expect("terminal fixture must construct the scheduler");
        scheduler.reset_wall_clock(&authority, 100).unwrap();
        scheduler
            .service_after_command_drain(&authority, 117, SchedulerServiceMode::Background)
            .unwrap();
        let step = scheduler.prepare_due_step(&authority).unwrap();
        let retained_debt = scheduler.diagnostics().pending_simulation_seconds;
        let transition = match running
            .advance_nonterminal(&mut authority, step.running_step_inputs())
            .expect("terminal result must stage its generation boundary")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
        assert_eq!(transition.candidate().generation.generation, 2);
        let transition_source_key = transition.source_key();
        assert_eq!(authority.state(), &source);
        assert!(scheduler.diagnostics().step_pending);
        assert_eq!(
            scheduler.diagnostics().pending_simulation_seconds.to_bits(),
            retained_debt.to_bits()
        );
        assert_eq!(scheduler.diagnostics().completed_steps, 0);
        assert_eq!(
            scheduler.prepare_due_step(&authority),
            Err(SchedulerError::StepPending)
        );
        assert!(running
            .discard_pending_generation_transition(&authority, transition_source_key)
            .expect("matching explicit discard must succeed"));
        assert!(running.pending_generation_transition().is_none());
        scheduler
            .reject_step(&authority, step)
            .expect("discarded persistence handoff must reject its scheduler ticket");
        assert_eq!(
            scheduler.diagnostics().pending_simulation_seconds.to_bits(),
            retained_debt.to_bits()
        );
        assert_eq!(scheduler.diagnostics().completed_steps, 0);
        assert_eq!(
            scheduler.prepare_due_step(&authority),
            Err(SchedulerError::CommandServiceRequired)
        );
    }

    #[test]
    fn scheduler_rebind_excludes_generation_persistence_wait_from_wall_debt() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal fixture must construct the coordinator");
        let mut scheduler = FixedStepScheduler::try_new(
            &authority,
            FixedStepSchedulerPolicy::provisional_defaults(),
        )
        .expect("terminal fixture must construct the scheduler");
        scheduler.reset_wall_clock(&authority, 100).unwrap();
        scheduler
            .service_after_command_drain(&authority, 117, SchedulerServiceMode::Background)
            .unwrap();
        let step = scheduler.prepare_due_step(&authority).unwrap();
        match coordinator
            .advance_nonterminal(&mut authority, step.running_step_inputs())
            .expect("terminal scheduled step must stage its generation boundary")
        {
            RunningStepProgress::GenerationTransitionPending(_) => {}
            other => panic!("expected pending generation transition, got {other:?}"),
        }
        let directory = GenerationHandoffDirectory::new();
        let descriptor = coordinator
            .publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                CheckpointOperationId::parse("00000000000000000000000000000083").unwrap(),
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            )
            .expect("terminal checkpoint must publish");
        coordinator
            .acknowledge_pending_generation_persistence(&authority, &descriptor)
            .expect("exact metadata commit must construct the base world");
        assert!(matches!(
            coordinator
                .prepare_acknowledged_generation_reassignments(&authority)
                .expect("no-controller generation needs no delivery"),
            GenerationReassignmentProgress::Ready(_)
        ));
        let publication = coordinator
            .publish_acknowledged_generation_start(&mut authority)
            .expect("fully durable no-controller generation must publish");
        assert_eq!(publication.external_assignments, 0);
        assert!(publication.unavailable_controller_reservations.is_empty());
        assert!(scheduler.diagnostics().step_pending);
        scheduler
            .commit_generation_transition(&authority, step, &publication, 10_000)
            .expect("scheduler must rebind to the exact running successor");
        let after_rebind = scheduler.diagnostics();
        assert!(!after_rebind.step_pending);
        assert_eq!(after_rebind.completed_steps, 1);
        assert_eq!(
            after_rebind.pending_simulation_seconds.to_bits(),
            step.wall_accumulator_after_step().to_bits()
        );
        assert!((after_rebind.observed_wall_seconds - 0.017).abs() < 1.0e-12);

        scheduler
            .service_after_command_drain(&authority, 10_017, SchedulerServiceMode::Background)
            .expect("new generation must accept the next command-service boundary");
        let after_next_wall = scheduler.diagnostics();
        assert!((after_next_wall.observed_wall_seconds - 0.034).abs() < 1.0e-12);
        assert!(after_next_wall.pending_simulation_seconds < 0.1);
    }

    #[test]
    fn exact_persistence_acknowledgement_precedes_next_world_construction() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let limits = RunningStepWorkLimits::provisional_defaults();
        let expected_projection = authority
            .running_step_config(limits)
            .expect("terminal settings must project");
        let mut coordinator = RunningStepCoordinator::try_new(&authority, limits)
            .expect("terminal fixture must construct the coordinator");
        let transition = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("terminal result must stage its exact boundary")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        let source_key = transition.source_key();
        assert!(transition.checkpoint_descriptor().is_none());
        assert!(!transition.persistence_acknowledged());
        assert!(matches!(
            coordinator.prepare_acknowledged_generation_start(&authority),
            Err(RunningStepError::GenerationPersistenceNotAcknowledged)
        ));

        let directory = GenerationHandoffDirectory::new();
        let operation = CheckpointOperationId::parse("00000000000000000000000000000081").unwrap();
        let descriptor = coordinator
            .publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                operation.clone(),
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            )
            .expect("managed checkpoint publication must succeed");
        assert!(directory
            .path()
            .join(&descriptor.relative_filename)
            .is_file());
        assert_eq!(
            coordinator
                .publish_pending_generation_checkpoint(
                    &authority,
                    directory.path(),
                    operation,
                    &generation_handoff_checkpoint_limits(),
                    &default_graph_limits(),
                )
                .expect("an exact retry must reuse the retained descriptor"),
            descriptor
        );
        assert!(matches!(
            coordinator.publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                CheckpointOperationId::parse("00000000000000000000000000000082").unwrap(),
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            ),
            Err(RunningStepError::GenerationCheckpointAlreadyPublished { .. })
        ));

        let mut wrong = descriptor.clone();
        let replacement = if wrong.logical_root_sha256.ends_with('0') {
            "1"
        } else {
            "0"
        };
        wrong.logical_root_sha256.replace_range(63..64, replacement);
        assert!(matches!(
            coordinator.acknowledge_pending_generation_persistence(&authority, &wrong),
            Err(
                RunningStepError::GenerationPersistenceAcknowledgementMismatch {
                    field: "logical root"
                }
            )
        ));
        assert_eq!(authority.state(), &source);
        assert!(!coordinator
            .pending_generation_transition()
            .expect("wrong acknowledgement must retain the transition")
            .persistence_acknowledged());

        let (first_world, first_rng, first_allocators) = {
            let prepared = coordinator
                .acknowledge_pending_generation_persistence(&authority, &descriptor)
                .expect("the exact committed descriptor may construct the next world");
            let expected_snakes = prepared
                .source()
                .config
                .population_count
                .checked_add(prepared.source().config.baseline_count)
                .unwrap();
            assert_eq!(prepared.world().snakes.len(), expected_snakes);
            assert_eq!(
                prepared.world().pellets.len(),
                expected_projection.world_step.prefix.ambient.target_count
            );
            (
                prepared.world().clone(),
                prepared.rng().clone(),
                prepared.allocators().clone(),
            )
        };
        assert_eq!(authority.state(), &source);
        let retained = coordinator
            .pending_generation_transition()
            .expect("acknowledged transition must remain pending until final publication");
        assert_eq!(retained.source_key(), source_key);
        assert_eq!(retained.checkpoint_descriptor(), Some(&descriptor));
        assert!(retained.persistence_acknowledged());
        assert!(matches!(
            coordinator.discard_pending_generation_transition(&authority, source_key),
            Err(RunningStepError::GenerationPersistenceAlreadyCommitted)
        ));

        let retried = coordinator
            .prepare_acknowledged_generation_start(&authority)
            .expect("a successful construction must be reborrowed without new draws");
        assert_eq!(retried.world(), &first_world);
        assert_eq!(retried.rng(), &first_rng);
        assert_eq!(retried.allocators(), &first_allocators);
        assert_eq!(authority.state(), &source);
    }

    #[test]
    fn durable_generation_boundary_reassigns_connected_controller_before_authority_swap() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_200.0,
                y: -1_200.0,
            },
        );
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal external fixture must construct the coordinator");
        let transition = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 500,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("terminal external fixture must stage a generation boundary")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        let source_key = transition.source_key();
        let directory = GenerationHandoffDirectory::new();
        let operation = CheckpointOperationId::parse("00000000000000000000000000000091")
            .expect("test operation ID");
        let descriptor = coordinator
            .publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                operation,
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            )
            .expect("generation checkpoint file must publish");
        coordinator
            .acknowledge_pending_generation_persistence(&authority, &descriptor)
            .expect("exact metadata acknowledgement must construct the generation base");

        let (event, token) = match coordinator
            .prepare_acknowledged_generation_reassignments(&authority)
            .expect("connected owner must stage one reliable generation assignment")
        {
            GenerationReassignmentProgress::DeliveryPending(batch) => {
                assert_eq!(batch.events().len(), 1);
                assert_eq!(batch.remaining(), 1);
                let event = batch.events()[0];
                assert_eq!(event.step_key, source_key);
                assert_eq!(event.connection_id, 7);
                assert_eq!(event.controller_kind, ControllerKind::Player);
                assert!(matches!(
                    event.delivery_kind,
                    ExternalDeliveryEventKind::ReplacementAssignment { .. }
                ));
                assert!(event.snake_id >= EXTERNAL_ENTITY_ID_START);
                let token = batch
                    .resume_token(0)
                    .expect("assignment must carry a fresh token")
                    .to_owned();
                assert!(!token.is_empty());
                assert_ne!(token, "external-delivery-1");
                (event, token)
            }
            GenerationReassignmentProgress::Ready(_) => {
                panic!("connected owner must wait for its exact local send result")
            }
        };
        assert_eq!(authority.state(), &source);

        let stale = ExternalDeliveryResult {
            step_key: source_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id + 1,
            lease_id: event.lease_id,
            accepted: true,
        };
        let pending = coordinator
            .submit_external_delivery_results(&mut authority, &[stale])
            .expect("stale connection result must be ignored");
        assert_eq!(pending.ignored_results, 1);
        assert!(matches!(pending.state, ExternalDeliveryState::Pending(_)));
        assert_eq!(authority.state(), &source);

        let accepted = ExternalDeliveryResult {
            step_key: source_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: true,
        };
        let resolved = coordinator
            .submit_external_delivery_results(&mut authority, &[accepted])
            .expect("exact assignment result must resolve the generation handoff");
        assert_eq!(resolved.matched_acceptances, 1);
        assert!(matches!(
            resolved.state,
            ExternalDeliveryState::GenerationAssignmentsReady(transition)
                if transition.source_key() == source_key
                    && transition.persistence_acknowledged()
        ));
        assert_eq!(authority.state(), &source);
        assert!(matches!(
            coordinator
                .prepare_acknowledged_generation_reassignments(&authority)
                .expect("resolved assignment must be reborrowed without new entropy"),
            GenerationReassignmentProgress::Ready(transition)
                if transition.source_key() == source_key
        ));
        let publication = coordinator
            .publish_acknowledged_generation_start(&mut authority)
            .expect("durable and assignment-resolved generation must publish atomically");
        assert_eq!(publication.source_key, source_key);
        assert_eq!(publication.world_epoch, authority.world_epoch());
        assert_eq!(publication.generation, source.generation.generation + 1);
        assert_eq!(
            publication.completed_step,
            source.generation.completed_step + 1
        );
        assert_eq!(
            publication.population_epoch,
            source.generation.population_epoch + 1
        );
        assert_eq!(publication.memory, authority.memory_estimate());
        assert_eq!(publication.external_assignments, 1);
        assert!(publication.unavailable_controller_reservations.is_empty());
        assert_eq!(authority.state().phase, AuthorityPhase::Running);
        assert_eq!(
            authority
                .state()
                .generation
                .wall_accumulator_seconds
                .to_bits(),
            0.0_f64.to_bits()
        );
        let replacement = authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == event.snake_id)
            .expect("published generation must contain the assigned snake");
        assert!(replacement.alive);
        assert_eq!(replacement.kind, SnakeKind::External);
        let lease = authority
            .state()
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.id == event.lease_id)
            .expect("published generation must contain the accepted lease");
        assert_eq!(lease.connection_id, Some(event.connection_id));
        assert_eq!(lease.resume_token, token);
        assert!(coordinator.pending_generation_transition().is_none());
        assert!(matches!(
            coordinator.advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 501,
                    wall_accumulator_seconds: 0.0,
                }
            ),
            Err(RunningStepError::AuthorityMismatch {
                field: "world epoch"
            })
        ));
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("the published successor must admit a fresh running coordinator");
    }

    #[test]
    fn durable_generation_boundary_retains_disconnected_token_without_hidden_reassignment() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_200.0,
                y: -1_200.0,
            },
        );
        let source_snake_id = candidate.world.controller_leases[0].snake_id;
        let source_token = candidate.world.controller_leases[0].resume_token.clone();
        let source_scope = candidate.world.controller_leases[0].scope.clone();
        let external_snake = candidate
            .world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == source_snake_id)
            .expect("external fixture snake must exist");
        external_snake.turn = 0.0;
        external_snake.input_boost = false;
        let lease = &mut candidate.world.controller_leases[0];
        lease.connection_id = None;
        lease.status = ControllerLeaseStatus::ReservedNeutral;
        lease.last_observed_at_ms = 200;
        lease.disconnected_at_ms = Some(200);
        lease.input_hold_expires_at_ms = Some(600);
        lease.grace_expires_at_ms = Some(30_200);
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal disconnected fixture must construct the coordinator");
        let transition = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 1_000,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("terminal disconnected fixture must stage a generation boundary")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        let source_key = transition.source_key();
        let directory = GenerationHandoffDirectory::new();
        let descriptor = coordinator
            .publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                CheckpointOperationId::parse("00000000000000000000000000000092").unwrap(),
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            )
            .expect("generation checkpoint file must publish");
        coordinator
            .acknowledge_pending_generation_persistence(&authority, &descriptor)
            .expect("exact metadata acknowledgement must construct the generation base");

        assert!(matches!(
            coordinator
                .prepare_acknowledged_generation_reassignments(&authority)
                .expect("disconnected owner needs no assignment delivery"),
            GenerationReassignmentProgress::Ready(ready)
                if ready.source_key() == source_key
        ));
        let unavailable = coordinator
            .pending_unavailable_controller_reservations()
            .expect("disconnected token outcome must remain inspectable before publication");
        assert_eq!(unavailable.len(), 1);
        assert_eq!(unavailable[0].source_lease_id, 1);
        assert_eq!(unavailable[0].source_snake_id, source_snake_id);
        assert_eq!(unavailable[0].controller_kind, ControllerKind::Player);
        assert_eq!(unavailable[0].scope, source_scope);
        assert_eq!(unavailable[0].resume_token, source_token);
        assert_eq!(unavailable[0].disconnected_at_ms, Some(200));
        assert_eq!(unavailable[0].grace_expires_at_ms, Some(30_200));
        assert_eq!(
            unavailable[0].reason,
            crate::engine::external_replacement::UnavailableControllerReason::SnakeUnavailable
        );
        let expected_unavailable = unavailable[0].clone();
        assert_eq!(authority.state(), &source);

        let publication = coordinator
            .publish_acknowledged_generation_start(&mut authority)
            .expect("durable disconnected generation must publish without a hidden snake");
        assert_eq!(publication.source_key, source_key);
        assert_eq!(publication.external_assignments, 0);
        assert_eq!(
            publication.unavailable_controller_reservations,
            vec![expected_unavailable]
        );
        assert!(authority.state().world.controller_leases.is_empty());
        assert!(authority
            .state()
            .world
            .snakes
            .iter()
            .all(|snake| snake.kind != SnakeKind::External));
        assert!(coordinator.pending_generation_transition().is_none());
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("the published successor must admit a fresh running coordinator");
    }

    fn candidate_fixed_dt(authority: &AuthoritativeState) -> f64 {
        authority.state().config.fixed_step_seconds
    }

    #[test]
    fn warmed_coordinator_rejects_a_different_authority_with_matching_epochs_and_graph() {
        let graph = default_graph();
        let mut first = complete_running_candidate(&graph);
        let resurrected_id = RESURRECTED_ENTITY_ID_START;
        push_external_snake(
            &mut first,
            &graph,
            resurrected_id,
            2,
            WorldPoint {
                x: 1_000.0,
                y: -1_000.0,
            },
        );
        first.allocators.next_external_id = EXTERNAL_ENTITY_ID_START;
        first.allocators.next_resurrected_id = resurrected_id + 1;
        let resurrected_index = first.world.snakes.len() - 1;
        let body_start = first.world.snakes[resurrected_index].body.start;
        let head = first.world.snakes[resurrected_index].position;
        first
            .world
            .body_points
            .extend((1..5).map(|offset| WorldPoint {
                x: head.x - (offset as f64 * 7.5),
                y: head.y,
            }));
        let resurrected = &mut first.world.snakes[resurrected_index];
        resurrected.kind = SnakeKind::Resurrected;
        resurrected.body = BodyRange {
            start: body_start,
            len: 5,
        };
        resurrected.target_length = 5.0;
        resurrected.radius = 9.0;
        resurrected.speed = 165.0;

        let mut second = first.clone();
        second.brains[1]
            .non_population_weights
            .as_mut()
            .expect("resurrected brain must own weights")[0] = 0.875;
        assert_eq!(first.identity, second.identity);
        assert_eq!(first.config, second.config);
        assert_ne!(first.brains[1], second.brains[1]);

        let mut first_authority = own_complete_running(first, Arc::clone(&graph));
        let mut second_authority = own_complete_running(second, Arc::clone(&graph));
        assert_ne!(
            first_authority.world_epoch(),
            second_authority.world_epoch()
        );
        let second_source = second_authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &first_authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("first authority must construct the coordinator");
        coordinator
            .advance_nonterminal(
                &mut first_authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("first authority must warm every control cache");

        assert!(matches!(
            coordinator.advance_nonterminal(
                &mut second_authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            ),
            Err(RunningStepError::AuthorityMismatch {
                field: "world epoch",
            })
        ));
        assert_eq!(second_authority.state(), &second_source);
    }

    #[test]
    fn external_delivery_acceptance_publishes_once_and_stale_results_are_ignored() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let external_id = EXTERNAL_ENTITY_ID_START;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            external_id,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("external fixture must construct the coordinator");

        let event = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("complete external step must await local Node acceptance")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                assert_eq!(batch.events().len(), 1);
                assert_eq!(batch.observation(0).unwrap().len(), 83);
                assert_eq!(batch.remaining(), 1);
                assert_eq!(batch.is_accepted(0), Some(false));
                batch.events()[0]
            }
            RunningStepProgress::Published(_) => {
                panic!("external score marker must not publish before Node acceptance")
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("external observation fixture must remain nonterminal")
            }
        };
        assert_eq!(authority.state(), &source);
        assert_eq!(coordinator.last_wall_now_ms(), Some(100));
        assert!(matches!(
            coordinator.advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 101,
                    wall_accumulator_seconds: 0.0,
                },
            ),
            Err(RunningStepError::ExternalDeliveryPending { count: 1 })
        ));
        assert_eq!(authority.state(), &source);

        let stale = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id + 1,
            accepted: true,
        };
        let replaced_connection = ExternalDeliveryResult {
            lease_id: event.lease_id,
            connection_id: event.connection_id + 1,
            ..stale
        };
        let stale_resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[stale, replaced_connection])
            .expect("stale assignment result must be ignored");
        assert_eq!(stale_resolution.matched_acceptances, 0);
        assert_eq!(stale_resolution.ignored_results, 2);
        assert!(matches!(
            stale_resolution.state,
            ExternalDeliveryState::Pending(batch) if batch.remaining() == 1
        ));
        assert_eq!(authority.state(), &source);

        let accepted = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: true,
        };
        let resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[accepted])
            .expect("matching accepted result must publish the complete step");
        let outcome = match resolution.state {
            ExternalDeliveryState::Published(outcome) => outcome,
            _ => panic!("the only matching event must complete publication"),
        };
        assert_eq!(resolution.matched_acceptances, 1);
        assert_eq!(resolution.ignored_results, 0);
        assert_eq!(outcome.publication.completed_step, 1);
        let published_external = authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == external_id)
            .unwrap();
        assert_eq!(published_external.delivered_observation_points, 10.01);

        let published = authority.state().clone();
        let duplicate = coordinator
            .submit_external_delivery_results(&mut authority, &[accepted])
            .expect("duplicate accepted result after publication must be ignored");
        assert_eq!(duplicate.matched_acceptances, 0);
        assert_eq!(duplicate.ignored_results, 1);
        assert!(matches!(duplicate.state, ExternalDeliveryState::Idle));
        assert_eq!(authority.state(), &published);
    }

    #[test]
    fn failed_external_delivery_publishes_the_step_with_a_disconnected_controller() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let external_id = EXTERNAL_ENTITY_ID_START;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            external_id,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("external fixture must construct the coordinator");
        let inputs = RunningStepInputs {
            wall_now_ms: 100,
            wall_accumulator_seconds: 0.0,
        };

        let first_event = match coordinator
            .advance_nonterminal(&mut authority, inputs)
            .expect("first attempt must stage")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
            RunningStepProgress::Published(_) => panic!("external event must defer publication"),
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("external send-failure fixture must remain nonterminal")
            }
        };
        let rejected = ExternalDeliveryResult {
            step_key: first_event.step_key,
            event_sequence: first_event.event_sequence,
            connection_id: first_event.connection_id,
            lease_id: first_event.lease_id,
            accepted: false,
        };
        let resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[rejected])
            .expect("matching failed send must publish the prevalidated disconnect");
        let outcome = match resolution.state {
            ExternalDeliveryState::Published(outcome) => outcome,
            _ => panic!("the failed send must resolve the only pending event"),
        };
        assert_eq!(resolution.matched_acceptances, 0);
        assert_eq!(resolution.matched_failures, 1);
        assert_eq!(resolution.ignored_results, 0);
        assert_eq!(outcome.publication.completed_step, 1);
        assert_eq!(outcome.diagnostics.external_deliveries_pending, 0);
        assert_eq!(authority.state().generation.completed_step, 1);
        assert_eq!(
            authority
                .state()
                .world
                .snakes
                .iter()
                .find(|snake| snake.id == external_id)
                .unwrap()
                .delivered_observation_points,
            2.0
        );
        let lease = authority
            .state()
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.id == first_event.lease_id)
            .unwrap();
        assert_eq!(lease.connection_id, None);
        assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
        assert_eq!(lease.disconnected_at_ms, Some(inputs.wall_now_ms));
        assert_eq!(lease.input_hold_expires_at_ms, Some(600));
        assert_eq!(lease.grace_expires_at_ms, Some(30_100));

        let stale_acceptance = ExternalDeliveryResult {
            accepted: true,
            ..rejected
        };
        let stale = coordinator
            .submit_external_delivery_results(&mut authority, &[stale_acceptance])
            .expect("result after publication must be ignored");
        assert_eq!(stale.matched_acceptances, 0);
        assert_eq!(stale.matched_failures, 0);
        assert_eq!(stale.ignored_results, 1);
        assert!(matches!(stale.state, ExternalDeliveryState::Idle));

        assert!(matches!(
            coordinator
                .advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: 101,
                        wall_accumulator_seconds: 0.0,
                    },
                )
                .expect("disconnected grace must no longer emit to the failed socket"),
            RunningStepProgress::Published(_)
        ));
        assert_eq!(authority.state().generation.completed_step, 2);
    }

    #[test]
    fn multiple_external_observations_publish_only_after_every_matching_acceptance() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let first_id = EXTERNAL_ENTITY_ID_START;
        let second_id = EXTERNAL_ENTITY_ID_START + 1;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            first_id,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            second_id,
            3,
            2,
            8,
            WorldPoint {
                x: -1_000.0,
                y: 1_000.0,
            },
        );
        candidate.world.controller_leases.last_mut().unwrap().kind =
            ControllerKind::ReinforcementLearning;
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("two-controller fixture must construct the coordinator");

        let events = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("complete step must stage both observations")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                assert_eq!(batch.events().len(), 2);
                assert_eq!(batch.remaining(), 2);
                [batch.events()[0], batch.events()[1]]
            }
            RunningStepProgress::Published(_) => panic!("both events must require acceptance"),
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("two-controller fixture must remain nonterminal")
            }
        };
        assert!(events[0].snake_id < events[1].snake_id);
        assert_eq!(events[0].controller_kind, ControllerKind::Player);
        assert_eq!(
            events[1].controller_kind,
            ControllerKind::ReinforcementLearning
        );

        let first_result = ExternalDeliveryResult {
            step_key: events[0].step_key,
            event_sequence: events[0].event_sequence,
            connection_id: events[0].connection_id,
            lease_id: events[0].lease_id,
            accepted: true,
        };
        let first = coordinator
            .submit_external_delivery_results(&mut authority, &[first_result])
            .expect("first exact result must be retained without publication");
        assert_eq!(first.matched_acceptances, 1);
        assert!(matches!(
            first.state,
            ExternalDeliveryState::Pending(batch)
                if batch.remaining() == 1 && batch.is_accepted(0) == Some(true)
        ));
        assert_eq!(authority.state(), &source);

        let duplicate = coordinator
            .submit_external_delivery_results(&mut authority, &[first_result])
            .expect("duplicate acceptance must be ignored");
        assert_eq!(duplicate.matched_acceptances, 0);
        assert_eq!(duplicate.ignored_results, 1);
        assert!(matches!(
            duplicate.state,
            ExternalDeliveryState::Pending(batch) if batch.remaining() == 1
        ));
        assert_eq!(authority.state(), &source);

        let duplicate_failure = ExternalDeliveryResult {
            accepted: false,
            ..first_result
        };
        let duplicate_failure_resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[duplicate_failure])
            .expect("a negative duplicate cannot override prior acceptance");
        assert_eq!(duplicate_failure_resolution.matched_acceptances, 0);
        assert_eq!(duplicate_failure_resolution.matched_failures, 0);
        assert_eq!(duplicate_failure_resolution.ignored_results, 1);
        assert!(matches!(
            duplicate_failure_resolution.state,
            ExternalDeliveryState::Pending(batch)
                if batch.remaining() == 1
                    && batch.status(0) == Some(ExternalDeliveryStatus::Accepted)
        ));
        assert_eq!(authority.state(), &source);

        let second_result = ExternalDeliveryResult {
            step_key: events[1].step_key,
            event_sequence: events[1].event_sequence,
            connection_id: events[1].connection_id,
            lease_id: events[1].lease_id,
            accepted: true,
        };
        let completed = coordinator
            .submit_external_delivery_results(&mut authority, &[second_result])
            .expect("last exact result must publish once");
        let completed_outcome = match completed.state {
            ExternalDeliveryState::Published(outcome) => outcome,
            _ => panic!("last exact acceptance must publish"),
        };
        assert_eq!(completed.matched_failures, 0);
        assert_eq!(completed_outcome.diagnostics.external_deliveries_pending, 0);
        for snake_id in [first_id, second_id] {
            let snake = authority
                .state()
                .world
                .snakes
                .iter()
                .find(|snake| snake.id == snake_id)
                .unwrap();
            assert_eq!(snake.delivered_observation_points, 10.01);
        }
        assert_eq!(authority.state().generation.completed_step, 1);
    }

    #[test]
    fn mixed_external_results_advance_only_accepted_markers_and_disconnect_failures() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let accepted_id = EXTERNAL_ENTITY_ID_START;
        let failed_id = EXTERNAL_ENTITY_ID_START + 1;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            accepted_id,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            failed_id,
            3,
            2,
            8,
            WorldPoint {
                x: -1_000.0,
                y: 1_000.0,
            },
        );
        candidate.world.controller_leases.last_mut().unwrap().kind =
            ControllerKind::ReinforcementLearning;
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("mixed-result fixture must construct the coordinator");
        let events = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("both external observations must stage")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                [batch.events()[0], batch.events()[1]]
            }
            RunningStepProgress::Published(_) => panic!("both events require local results"),
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("mixed-result fixture must remain nonterminal")
            }
        };
        let accepted = ExternalDeliveryResult {
            step_key: events[0].step_key,
            event_sequence: events[0].event_sequence,
            connection_id: events[0].connection_id,
            lease_id: events[0].lease_id,
            accepted: true,
        };
        let failed = ExternalDeliveryResult {
            step_key: events[1].step_key,
            event_sequence: events[1].event_sequence,
            connection_id: events[1].connection_id,
            lease_id: events[1].lease_id,
            accepted: false,
        };
        let resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[failed, accepted])
            .expect("prevalidated mixed results must publish atomically");
        assert_eq!(resolution.matched_acceptances, 1);
        assert_eq!(resolution.matched_failures, 1);
        assert_eq!(resolution.ignored_results, 0);
        let outcome = match resolution.state {
            ExternalDeliveryState::Published(outcome) => outcome,
            _ => panic!("all resolved events must publish exactly once"),
        };
        assert_eq!(outcome.diagnostics.external_deliveries_pending, 0);

        let accepted_snake = authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == accepted_id)
            .unwrap();
        let failed_snake = authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == failed_id)
            .unwrap();
        assert_eq!(accepted_snake.delivered_observation_points, 10.01);
        assert_eq!(failed_snake.delivered_observation_points, 2.0);
        let accepted_lease = authority
            .state()
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.snake_id == accepted_id)
            .unwrap();
        let failed_lease = authority
            .state()
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.snake_id == failed_id)
            .unwrap();
        assert_eq!(accepted_lease.status, ControllerLeaseStatus::Connected);
        assert_eq!(accepted_lease.connection_id, Some(7));
        assert_eq!(failed_lease.status, ControllerLeaseStatus::HoldingLastInput);
        assert_eq!(failed_lease.connection_id, None);
        assert_eq!(authority.state().generation.completed_step, 1);
    }

    #[test]
    fn repeated_external_delivery_reuses_every_reported_bridge_capacity() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("external fixture must construct the coordinator");
        let mut warmed_capacities = None;

        for boundary in 0..24u64 {
            let event = match coordinator
                .advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: 100 + boundary,
                        wall_accumulator_seconds: 0.0,
                    },
                )
                .expect("external boundary must stage")
            {
                RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
                RunningStepProgress::Published(_) => {
                    panic!("external boundary must await local acceptance")
                }
                RunningStepProgress::GenerationTransitionPending(_) => {
                    panic!("warmed external boundary must remain nonterminal")
                }
            };
            let pending = coordinator.external_delivery_diagnostics();
            assert_eq!(pending.pending_events, 1);
            assert_eq!(pending.remaining_events, 1);
            let capacities = (
                pending.event_capacity,
                pending.acceptance_capacity,
                pending.disconnect_capacity,
                pending.observation_capacity,
            );
            if let Some(expected) = warmed_capacities {
                assert_eq!(capacities, expected);
            } else {
                warmed_capacities = Some(capacities);
            }

            let accepted = ExternalDeliveryResult {
                step_key: event.step_key,
                event_sequence: event.event_sequence,
                connection_id: event.connection_id,
                lease_id: event.lease_id,
                accepted: true,
            };
            assert!(matches!(
                coordinator
                    .submit_external_delivery_results(&mut authority, &[accepted])
                    .expect("matching result must publish")
                    .state,
                ExternalDeliveryState::Published(_)
            ));
            let idle = coordinator.external_delivery_diagnostics();
            assert_eq!(idle.pending_events, 0);
            assert_eq!(idle.remaining_events, 0);
            assert_eq!(
                (
                    idle.event_capacity,
                    idle.acceptance_capacity,
                    idle.disconnect_capacity,
                    idle.observation_capacity,
                ),
                warmed_capacities.unwrap()
            );
        }
        assert_eq!(authority.state().generation.completed_step, 24);
    }

    #[test]
    fn superseded_operation_ignores_its_old_external_result_without_publication() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("external fixture must construct the coordinator");
        let event = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("external step must stage")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
            RunningStepProgress::Published(_) => panic!("external event must defer publication"),
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("stale-result fixture must remain nonterminal")
            }
        };
        authority
            .begin_running_step()
            .expect("a newer operation must supersede the retained proposal");
        let result = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: true,
        };
        let resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[result])
            .expect("superseded result must be ignored");
        assert_eq!(resolution.matched_acceptances, 0);
        assert_eq!(resolution.ignored_results, 1);
        assert!(matches!(resolution.state, ExternalDeliveryState::Idle));
        assert_eq!(authority.state(), &source);
    }

    #[test]
    fn terminal_or_failed_admission_never_exposes_an_external_delivery_batch() {
        let graph = default_graph();

        let mut terminal = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut terminal,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        set_complete_setting(
            &mut terminal,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut terminal,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        terminal.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        refresh_config_hash(&mut terminal);
        let mut terminal_authority = own_complete_running(terminal, Arc::clone(&graph));
        let terminal_source = terminal_authority.state().clone();
        let mut terminal_coordinator = RunningStepCoordinator::try_new(
            &terminal_authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal fixture must construct the coordinator");
        let transition = match terminal_coordinator
            .advance_nonterminal(
                &mut terminal_authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("terminal attempt must stage one generation transition")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
        assert_eq!(terminal_authority.state(), &terminal_source);
        assert!(matches!(
            terminal_coordinator
                .submit_external_delivery_results(&mut terminal_authority, &[])
                .expect("terminal attempt must retain no bridge batch")
                .state,
            ExternalDeliveryState::Idle
        ));

        let mut inadmissible = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut inadmissible,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint {
                x: 1_000.0,
                y: 1_000.0,
            },
        );
        let mut inadmissible_authority = own_complete_running(inadmissible, Arc::clone(&graph));
        let inadmissible_source = inadmissible_authority.state().clone();
        let mut inadmissible_coordinator = RunningStepCoordinator::try_new(
            &inadmissible_authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("memory fixture must construct the coordinator");
        inadmissible_authority.memory_ceiling_bytes = 1;
        assert!(matches!(
            inadmissible_coordinator.advance_nonterminal(
                &mut inadmissible_authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            ),
            Err(RunningStepError::State(error))
                if matches!(*error, StateError::MemoryCeilingExceeded { .. })
        ));
        assert_eq!(inadmissible_authority.state(), &inadmissible_source);
        assert!(matches!(
            inadmissible_coordinator
                .submit_external_delivery_results(&mut inadmissible_authority, &[])
                .expect("failed admission must expose no bridge batch")
                .state,
            ExternalDeliveryState::Idle
        ));
    }

    #[test]
    fn controlled_wall_death_waits_for_reliable_replacement_assignment_before_publication() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let old_snake_id = EXTERNAL_ENTITY_ID_START;
        let old_lease_id = 1;
        let connection_id = 7;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            old_snake_id,
            2,
            old_lease_id,
            connection_id,
            WorldPoint { x: 3_490.5, y: 0.0 },
        );
        let source = candidate.clone();
        let old_token = source
            .world
            .controller_leases
            .last()
            .unwrap()
            .resume_token
            .clone();
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("controlled wall-death fixture must construct the coordinator");

        let (event, new_token) = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("controlled death must stage one reliable replacement assignment")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                assert_eq!(batch.events().len(), 1);
                assert_eq!(batch.remaining(), 1);
                assert_eq!(batch.observation(0), None);
                let event = batch.events()[0];
                assert!(matches!(
                    event.delivery_kind,
                    ExternalDeliveryEventKind::ReplacementAssignment { frame_v1_id }
                        if frame_v1_id == source.allocators.next_frame_v1_id
                ));
                assert_eq!(event.snake_id, source.allocators.next_external_id);
                assert_eq!(event.lease_id, source.allocators.next_controller_lease_id);
                assert_eq!(event.connection_id, connection_id);
                assert!(event.position.x.is_finite() && event.position.y.is_finite());
                assert!(event.direction.is_finite());
                let token = batch
                    .resume_token(0)
                    .expect("replacement assignment must carry one opaque token")
                    .to_owned();
                assert_eq!(token.len(), 32);
                assert_ne!(token, old_token);
                (event, token)
            }
            RunningStepProgress::Published(_) => {
                panic!("replacement assignment must resolve before authority publication")
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("controlled wall-death fixture must remain nonterminal")
            }
        };
        assert_eq!(authority.state(), &source);

        let stale = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: old_lease_id,
            accepted: true,
        };
        let stale_resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[stale])
            .expect("superseded lease result must be ignored");
        assert_eq!(stale_resolution.matched_acceptances, 0);
        assert_eq!(stale_resolution.ignored_results, 1);
        assert!(matches!(
            stale_resolution.state,
            ExternalDeliveryState::Pending(batch) if batch.remaining() == 1
        ));
        assert_eq!(authority.state(), &source);

        let accepted = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: true,
        };
        let resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[accepted])
            .expect("accepted replacement assignment must publish the complete fixed step");
        assert_eq!(resolution.matched_acceptances, 1);
        assert_eq!(resolution.matched_failures, 0);
        let outcome = match resolution.state {
            ExternalDeliveryState::Published(outcome) => outcome,
            _ => panic!("the resolved replacement must publish exactly once"),
        };
        assert_eq!(outcome.publication.completed_step, 1);
        assert_eq!(outcome.diagnostics.external_deliveries_pending, 0);

        let published = authority.state();
        assert!(published
            .world
            .snakes
            .iter()
            .all(|snake| snake.id != old_snake_id));
        let replacement = published
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == event.snake_id)
            .expect("fresh replacement snake must become authoritative");
        assert!(replacement.alive);
        assert_eq!(replacement.kind, SnakeKind::External);
        assert_eq!(replacement.position, event.position);
        assert_eq!(replacement.direction, event.direction);
        assert_eq!(replacement.body.len, 5);
        assert_eq!(
            published.world.body_points[replacement.body.start],
            replacement.position
        );
        let lease = published
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.id == event.lease_id)
            .expect("fresh replacement lease must become authoritative");
        assert_eq!(lease.snake_id, event.snake_id);
        assert_eq!(lease.connection_id, Some(connection_id));
        assert_eq!(lease.status, ControllerLeaseStatus::Connected);
        assert_eq!(lease.resume_token, new_token);
        assert_eq!(lease.latest_action.turn, 0.0);
        assert!(!lease.latest_action.boost);

        let brain = published
            .brains
            .iter()
            .find(|brain| brain.owner == BrainOwner::Entity(event.snake_id))
            .expect("fresh replacement brain must become authoritative");
        assert_eq!(brain.handle.id, source.allocators.next_brain_id);
        assert_eq!(
            brain.non_population_weights.as_ref().unwrap().len(),
            graph.total_parameters
        );
        assert!(brain
            .non_population_weights
            .as_ref()
            .unwrap()
            .iter()
            .any(|weight| weight.to_bits() != 0));
        assert_eq!(brain.recurrent.len(), graph.total_state_size);
        assert!(brain.recurrent.iter().all(|value| value.to_bits() == 0));
        assert_ne!(
            published.rng.external_controller,
            source.rng.external_controller
        );
        assert_eq!(published.rng.evolution, source.rng.evolution);
        assert_eq!(
            published.allocators.next_external_id,
            source.allocators.next_external_id + 1
        );
        assert_eq!(
            published.allocators.next_frame_v1_id,
            source.allocators.next_frame_v1_id + 1
        );
        assert_eq!(
            published.allocators.next_brain_id,
            source.allocators.next_brain_id + 1
        );
        assert_eq!(
            published.allocators.next_controller_lease_id,
            source.allocators.next_controller_lease_id + 1
        );

        let published = published.clone();
        let duplicate = coordinator
            .submit_external_delivery_results(&mut authority, &[accepted])
            .expect("duplicate replacement result after publication must be ignored");
        assert_eq!(duplicate.matched_acceptances, 0);
        assert_eq!(duplicate.ignored_results, 1);
        assert!(matches!(duplicate.state, ExternalDeliveryState::Idle));
        assert_eq!(authority.state(), &published);
    }

    #[test]
    fn failed_replacement_assignment_keeps_the_known_token_and_disconnect_grace() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let old_snake_id = EXTERNAL_ENTITY_ID_START;
        let old_lease_id = 1;
        let connection_id = 7;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            old_snake_id,
            2,
            old_lease_id,
            connection_id,
            WorldPoint { x: 3_490.5, y: 0.0 },
        );
        let old_token = candidate
            .world
            .controller_leases
            .last()
            .unwrap()
            .resume_token
            .clone();
        let source = candidate.clone();
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("failed-assignment fixture must construct the coordinator");
        let (event, rejected_token) = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("controlled death must stage its assignment")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                (batch.events()[0], batch.resume_token(0).unwrap().to_owned())
            }
            RunningStepProgress::Published(_) => panic!("assignment must resolve first"),
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("failed-assignment fixture must remain nonterminal")
            }
        };
        assert_ne!(rejected_token, old_token);
        assert_eq!(authority.state(), &source);

        let rejected = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: false,
        };
        let resolution = coordinator
            .submit_external_delivery_results(&mut authority, &[rejected])
            .expect("failed local assignment send must publish the disconnected replacement");
        assert_eq!(resolution.matched_failures, 1);
        assert!(matches!(
            resolution.state,
            ExternalDeliveryState::Published(_)
        ));

        let published = authority.state();
        assert!(published
            .world
            .snakes
            .iter()
            .all(|snake| snake.id != old_snake_id));
        let replacement = published
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == event.snake_id)
            .expect("failed delivery must not discard the fresh snake");
        assert!(replacement.alive);
        assert_eq!(replacement.turn, 0.0);
        assert!(!replacement.input_boost);
        let lease = published
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.id == event.lease_id)
            .expect("fresh lease must enter disconnect grace");
        assert_eq!(lease.resume_token, old_token);
        assert_ne!(lease.resume_token, rejected_token);
        assert_eq!(lease.connection_id, None);
        assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
        assert_eq!(lease.disconnected_at_ms, Some(100));
        assert_eq!(lease.input_hold_expires_at_ms, Some(600));
        assert_eq!(lease.grace_expires_at_ms, Some(30_100));
        assert_eq!(lease.takeover_committed_at_ms, None);

        let duplicate = coordinator
            .submit_external_delivery_results(
                &mut authority,
                &[ExternalDeliveryResult {
                    accepted: true,
                    ..rejected
                }],
            )
            .expect("late acceptance after a failed assignment must be ignored");
        assert_eq!(duplicate.matched_acceptances, 0);
        assert_eq!(duplicate.ignored_results, 1);
        assert!(matches!(duplicate.state, ExternalDeliveryState::Idle));
    }

    #[test]
    fn already_disconnected_controlled_death_removes_only_the_dead_lease() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let old_snake_id = EXTERNAL_ENTITY_ID_START;
        let old_lease_id = 1;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            old_snake_id,
            2,
            old_lease_id,
            7,
            WorldPoint { x: 3_490.5, y: 0.0 },
        );
        let lease = candidate.world.controller_leases.last_mut().unwrap();
        lease.connection_id = None;
        lease.status = ControllerLeaseStatus::HoldingLastInput;
        lease.disconnected_at_ms = Some(100);
        lease.input_hold_expires_at_ms = Some(600);
        lease.grace_expires_at_ms = Some(30_100);
        let snake = candidate.world.snakes.last_mut().unwrap();
        snake.turn = lease.latest_action.turn;
        snake.input_boost = lease.latest_action.boost;
        let source_rng = candidate.rng.clone();
        let source_allocators = candidate.allocators.clone();
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("disconnected wall-death fixture must construct the coordinator");

        let outcome = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("a disconnected controlled death needs no Node assignment result")
        {
            RunningStepProgress::Published(outcome) => outcome,
            RunningStepProgress::ExternalDeliveryPending(_) => {
                panic!("a disconnected dead owner has no socket to receive an assignment")
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("disconnected controlled-death fixture must remain nonterminal")
            }
        };
        assert_eq!(outcome.diagnostics.external_replacement.replacements, 0);
        assert_eq!(
            outcome.diagnostics.external_replacement.removed_dead_leases,
            1
        );
        let published = authority.state();
        let dead = published
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == old_snake_id)
            .expect("the dead body remains part of the running world boundary");
        assert!(!dead.alive);
        assert!(published
            .world
            .controller_leases
            .iter()
            .all(|lease| lease.id != old_lease_id));
        assert_eq!(
            published.rng.external_controller,
            source_rng.external_controller
        );
        assert_eq!(
            published.allocators.next_external_id,
            source_allocators.next_external_id
        );
        assert_eq!(
            published.allocators.next_brain_id,
            source_allocators.next_brain_id
        );
        assert_eq!(
            published.allocators.next_controller_lease_id,
            source_allocators.next_controller_lease_id
        );
    }

    #[test]
    fn regressing_controller_wall_clock_never_starts_a_new_attempt() {
        let graph = default_graph();
        let candidate = complete_running_candidate(&graph);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("complete admitted config must construct the coordinator");
        assert!(matches!(
            coordinator
                .advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: 100,
                        wall_accumulator_seconds: 0.0,
                    },
                )
                .expect("first boundary must publish"),
            RunningStepProgress::Published(_)
        ));
        let source = authority.state().clone();
        assert!(matches!(
            coordinator.advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 99,
                    wall_accumulator_seconds: 0.0,
                },
            ),
            Err(RunningStepError::RegressingWallClock {
                previous_ms: 100,
                actual_ms: 99,
            })
        ));
        assert_eq!(authority.state(), &source);
    }

    #[test]
    fn generation_guards_refuse_nonterminal_publication_without_mutation() {
        let graph = default_graph();
        let cases = [
            (
                GenerationTransitionReason::EarlyAliveCount,
                240.0,
                8.0,
                8.0 - (1.0 / 120.0),
            ),
            (
                GenerationTransitionReason::Duration,
                8.0,
                50.0,
                8.0 - (1.0 / 120.0),
            ),
        ];

        for (expected_reason, generation_seconds, early_seconds, elapsed_seconds) in cases {
            let mut candidate = complete_running_candidate(&graph);
            set_complete_setting(
                &mut candidate,
                "generationSeconds",
                NormalizedSettingValue::Float(generation_seconds),
            );
            set_complete_setting(
                &mut candidate,
                "observer.earlyEndMinSeconds",
                NormalizedSettingValue::Float(early_seconds),
            );
            candidate.generation.elapsed_seconds = elapsed_seconds;
            refresh_config_hash(&mut candidate);
            let mut authority = own_complete_running(candidate, Arc::clone(&graph));
            let source = authority.state().clone();
            let mut coordinator = RunningStepCoordinator::try_new(
                &authority,
                RunningStepWorkLimits::provisional_defaults(),
            )
            .expect("terminal fixture must construct the coordinator");

            let transition = match coordinator
                .advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: 200,
                        wall_accumulator_seconds: 0.0,
                    },
                )
                .expect("terminal guard must stage a generation transition")
            {
                RunningStepProgress::GenerationTransitionPending(transition) => transition,
                other => panic!("expected pending generation transition, got {other:?}"),
            };
            assert_eq!(transition.reason(), expected_reason);
            assert_eq!(transition.alive_evolved(), 1);
            assert_eq!(authority.state(), &source);
        }
    }

    #[test]
    fn terminal_guard_runs_before_old_generation_external_replacement() {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            EXTERNAL_ENTITY_ID_START,
            2,
            1,
            7,
            WorldPoint { x: 3_490.5, y: 0.0 },
        );
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(8.0),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(50.0),
        );
        candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
        candidate.allocators.next_external_id = BASELINE_ENTITY_ID_START;
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal controlled-death fixture must construct the coordinator");

        let transition = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("terminal wall death must stage a generation transition")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
        assert_eq!(authority.state(), &source);
        assert!(matches!(
            coordinator
                .submit_external_delivery_results(&mut authority, &[])
                .expect("terminal attempt must expose no replacement assignment")
                .state,
            ExternalDeliveryState::Idle
        ));
    }

    #[test]
    fn invalid_scheduler_accumulator_never_starts_or_changes_a_step() {
        let graph = default_graph();
        let candidate = complete_running_candidate(&graph);
        let mut authority = own_complete_running(candidate, graph);
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("complete admitted config must construct the coordinator");

        for value in [f64::NAN, f64::INFINITY, -f64::EPSILON] {
            assert!(matches!(
                coordinator.advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: 100,
                        wall_accumulator_seconds: value,
                    },
                ),
                Err(RunningStepError::InvalidSchedulerAccumulator(actual))
                    if actual.to_bits() == value.to_bits()
            ));
            assert_eq!(authority.state(), &source);
            assert_eq!(coordinator.last_wall_now_ms(), None);
        }
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
