//! Rust-owned persistent state contracts for the authoritative engine.
//!
//! This module deliberately contains data ownership, admission estimates, and
//! validation only. Gameplay behavior, N-API conversion, serialization,
//! database access, and managed-checkpoint I/O belong to later layers. A
//! candidate becomes authoritative only after the complete candidate has been
//! validated against its compiled graph and caller-supplied memory ceiling.

use super::baseline::{BaselineLifecycleState, BaselineSlotRuntime};
#[path = "controller_join.rs"]
mod controller_join;
#[path = "entity_selection.rs"]
mod entity_selection;
#[path = "resurrection.rs"]
mod resurrection;
use super::contract::ENGINE_CONTRACT_VERSION;
use super::external_replacement::{
    ExternalReplacementAuthorityProof, UnavailableControllerReason,
    UnavailableControllerReservation,
};
use super::graph::{
    CompiledGraph, CompiledNode, GraphBundle, GraphEdge, GraphNodeKind, GraphNodeSpec,
    GraphOutputRef, GraphSpec,
};
use super::live_settings::{
    prepare_live_settings, LiveSettingUpdate, LiveSettingsError, PreparedLiveSettings,
};
use super::physics::{PhysicsStepKey, PhysicsStepKeyField};
pub use super::retained_origin::RetainedNeuralOrigin;
use super::rng::{RngError, SerializedRngState, StatefulRng};
use super::run_start::RunStartPersistenceProof;
use super::running_step::{ResolvedGenerationStartReplacement, ResolvedRunningStepReplacement};
use super::sensors::SensorGenerationState;
use super::step_config::{
    project_evolution_config, project_running_step_config, RunningStepConfigProjection,
    RunningStepWorkLimits, StepConfigError,
};
pub use controller_join::{ControllerJoinInput, PreparedControllerJoin};
pub use entity_selection::{EntitySelection, EntitySelectionTarget, EntitySelectionToken};
pub use resurrection::ResurrectionPublication;
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
/// Frozen historical sensor version and ordinary production default.
pub const SENSOR_VERSION: u32 = 3;
/// Version of the independent RNG-stream bundle.
pub const RNG_BUNDLE_VERSION: u32 = 1;
/// Additive bundle encoding with dense uniform-only evolved respawn streams.
pub const RNG_BUNDLE_WITH_EVOLVED_VERSION: u32 = 2;
/// Version three adds dense isolated baseline-v2 behavior continuations.
pub const RNG_BUNDLE_WITH_BASELINE_V2_VERSION: u32 = 3;
/// Version of normalized configuration encoded by this state contract.
pub const NORMALIZED_CONFIG_VERSION: u32 = 1;
/// Version of the monotonic allocator continuation bundle.
pub const ALLOCATOR_VERSION: u32 = 1;
/// Managed checkpoint version selected by the approved plan.
pub const CHECKPOINT_VERSION: u32 = 3;
/// Next process-local identity assigned to a newly admitted authoritative world.
static NEXT_WORLD_EPOCH: AtomicU64 = AtomicU64::new(1);

// `try_update` is the newer spelling and is unavailable on Rust 1.92.
#[allow(deprecated)]
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
/// Serialized one-past-the-end public-ID cursor, wrapped at the next reservation.
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateAdmissionPolicy {
    /// Remaining hard peak ceiling after the caller accounts for the current
    /// engine, Node, transfer spools, and any simultaneously staged state.
    pub memory_ceiling_bytes: usize,
    /// Whether producer source/compiler identity must match rather than only
    /// the versioned state, target, build class/profile, and math contracts.
    pub require_exact_build_identity: bool,
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
    /// A bounded restore-only alternative. Running admission normally stays exact.
    pub allowed_math_backends: Vec<String>,
    /// Versioned path-and-value-kind layout expected for normalized settings.
    pub expected_settings_schema_sha256: String,
    /// Explicit additional schema fingerprints; empty for frozen and running admission.
    pub allowed_settings_schema_sha256: Vec<String>,
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
    /// Partial source provenance retained independently of this new run's exact continuation.
    pub legacy_conversion: Option<super::legacy_origin::LegacyPopulationOrigin>,
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
    /// Immutable validated retained provenance; population/external owners have none.
    pub retained_origin: Option<Arc<RetainedNeuralOrigin>>,
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
    /// Per-population-slot respawn streams; absent in the frozen v1 contract.
    pub evolved_respawns: Vec<EvolvedRespawnRngState>,
    /// Dense uniform-only baseline-v2 behavior; absent in frozen v1/v2 bundles.
    pub baseline_behaviors: Vec<super::rng::BaselineBehaviorRngState>,
}

/// Dense uniform-only respawn continuation. Delay and placement never use
/// Gaussian sampling, so no string or spare storage is needed in hot copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvolvedRespawnRngState {
    pub slot: u32,
    pub uniform_state: u32,
}

impl EvolvedRespawnRngState {
    /// Derive one generation's streams through the canonical full-width labels.
    pub fn derive_generation(
        seed: u32,
        generation: u64,
        count: usize,
    ) -> Result<Vec<Self>, StateError> {
        let serialized =
            super::rng::derive_evolved_respawn_rngs(seed, generation, count).map_err(|error| {
                StateError::InvalidRng {
                    stream: "evolved-respawn".into(),
                    reason: error.to_string(),
                }
            })?;
        let mut states = Vec::new();
        states
            .try_reserve_exact(count)
            .map_err(|_| StateError::InvalidField {
                field: "rng.evolved_respawns",
                reason: "could not allocate evolved RNG states".into(),
            })?;
        for (slot, stream) in serialized.iter().enumerate() {
            let runtime =
                StatefulRng::from_state(stream).map_err(|error| StateError::InvalidRng {
                    stream: "evolved-respawn".into(),
                    reason: error.to_string(),
                })?;
            states.push(Self {
                slot: slot as u32,
                uniform_state: runtime.uniform_state(),
            });
        }
        Ok(states)
    }

    /// Restore one temporary runtime without altering its authoritative record.
    pub(crate) fn runtime(self) -> Result<StatefulRng, RngError> {
        StatefulRng::from_uniform_state(self.uniform_state)
    }
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

/// Deterministic allocation continuations; durable identities are monotonic.
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
    /// Next frame-v1 candidate; the one-past-limit cursor wraps on reservation.
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
    /// Reserve a contiguous public range without reusing any retained snake ID.
    /// Search forward, then wrap; durable entity and lease IDs are never recycled.
    pub fn reserve_frame_v1_ids<I>(
        &mut self,
        count: u32,
        occupied: I,
    ) -> Result<Option<FrameV1IdReservation>, StateError>
    where
        I: Iterator<Item = u32> + Clone,
    {
        if count == 0 {
            return Ok(None);
        }
        if self.next_frame_v1_id == 0
            || self.next_frame_v1_id > FRAME_V1_EXHAUSTED_ID
            || count > FRAME_V1_MAX_EXACT_ID
        {
            return Err(StateError::IdExhausted {
                kind: "frame-v1",
                requested: u64::from(count),
            });
        }
        let cursor = if self.next_frame_v1_id == FRAME_V1_EXHAUSTED_ID {
            1
        } else {
            self.next_frame_v1_id
        };
        // Every collision moves past a retained ID. No namespace-sized bitmap or allocation is needed.
        for (start, stop) in [(cursor, FRAME_V1_EXHAUSTED_ID), (1, cursor)] {
            let mut first = start;
            while first < stop {
                let Some(last) = first
                    .checked_add(count - 1)
                    .filter(|last| *last <= FRAME_V1_MAX_EXACT_ID)
                else {
                    break;
                };
                if let Some(collision) = occupied
                    .clone()
                    .filter(|id| *id >= first && *id <= last)
                    .max()
                {
                    first = collision + 1;
                    continue;
                }
                self.next_frame_v1_id = last + 1;
                return Ok(Some(FrameV1IdReservation { first, last }));
            }
        }
        Err(StateError::IdExhausted {
            kind: "frame-v1",
            requested: u64::from(count),
        })
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
    /// Retained non-population neural owner: Hall-of-Fame or saved brain.
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
    /// Whether an observation was consumed or accepted in this physical life.
    pub observation_delivered: bool,
    /// Signed wrapped heading change over the last complete physical fixed step.
    pub completed_step_heading_delta: f64,
    /// Speed change over the last complete physical fixed step.
    pub completed_step_speed_delta: f64,
    /// Turn actually applied during that complete step, independent of delivery cadence.
    pub completed_step_applied_turn: f32,
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
    /// False for neutral assignment/reclaim input; true only after an explicit command.
    pub is_explicit: bool,
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
    /// Runtime-only accounting for this assigned physical life. Generation
    /// checkpoints are pre-spawn and establish fresh ledgers on assignment.
    pub reward: Option<super::external_reward::ExternalRewardLedger>,
    /// Optional bounded trusted-LAN legacy identity, scoped by run and kind.
    /// Empty means this lease cannot be reclaimed without its token.
    pub identity_key: String,
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
    /// Generation-scoped sensor ray fairness, published only with complete steps.
    pub ray_cursor: super::ray_work::RayQuotaCursor,
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
    /// Dense population-owned life and economic continuation.
    pub evolved_generation: super::evolved_lives::EvolvedGenerationState,
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
            evolved_generation: super::evolved_lives::EvolvedGenerationState::default(),
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
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
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
    /// Ledger derived from exactly the same prefix and physics transaction.
    pub evolved_generation: &'buffers mut super::evolved_lives::EvolvedGenerationState,
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
/// A normal fixed step carries no entity-mutation proof and therefore retains
/// the strict allocator, brain-identity, and weight contract. Gameplay effects
/// may advance their isolated owner RNG streams in ordinary physics. Controlled-
/// death replacement is the only current path that may additionally advance
/// external identity domains during a nonterminal step.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RunningStepMutationContract<'proof> {
    proof: Option<&'proof ExternalReplacementAuthorityProof>,
    baseline_v2: Option<super::control_phase::BaselineV2ControlProof>,
    evolved: Option<&'proof super::evolved_replacement::EvolvedReplacementAuthorityProof>,
}

impl<'proof> RunningStepMutationContract<'proof> {
    pub(crate) const fn with_baseline_v2(
        mut self,
        proof: Option<super::control_phase::BaselineV2ControlProof>,
    ) -> Self {
        self.baseline_v2 = proof;
        self
    }
    pub(crate) const fn external(proof: &'proof ExternalReplacementAuthorityProof) -> Self {
        Self {
            proof: Some(proof),
            baseline_v2: None,
            evolved: None,
        }
    }

    pub(crate) const fn with_evolved(
        mut self,
        proof: Option<&'proof super::evolved_replacement::EvolvedReplacementAuthorityProof>,
    ) -> Self {
        self.evolved = proof;
        self
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
        self.baseline_v2.is_none_or(|proof| {
            proof.matches(
                replacement.key,
                replacement.rng,
                &*replacement.baseline_lifecycle,
            )
        }) && self.proof.is_none_or(|proof| {
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
    evolved_generation: usize,
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
    evolved_mutation_proof: usize,
    baseline_v2_proof: Option<(Option<PhysicsStepKey>, [u8; 32])>,
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
    /// Ephemeral nonce protects selections across process restarts and replacement worlds.
    selection_nonce: [u64; 2],
    world_epoch: u64,
    latest_operation_epoch: u64,
    world_validation: WorldValidationScratch,
}

/// One bounded reconnect transaction pinned to the unprepared source step.
/// The runtime retains this until exact local assignment delivery resolves.
#[derive(Clone, Debug)]
pub struct PreparedControllerReclaim {
    world_epoch: u64,
    operation_epoch: u64,
    completed_step: u64,
    lease_index: usize,
    snake_index: usize,
    frame_v1_id: u32,
    expected_memory: StateMemoryEstimate,
    next_memory: StateMemoryEstimate,
    proposal: super::controllers::ReclaimProposal,
}

impl PreparedControllerReclaim {
    /// Exact existing lease returned in the pending assignment.
    #[must_use]
    pub fn lease_id(&self) -> u64 {
        self.proposal.lease_id()
    }

    /// Exact destination connection, independent of the old socket.
    #[must_use]
    pub fn connection_id(&self) -> u64 {
        self.proposal.connection_id()
    }

    /// Existing browser-visible snake identity; reclaim never creates a snake.
    #[must_use]
    pub const fn frame_v1_id(&self) -> u32 {
        self.frame_v1_id
    }

    /// Newly staged token; a failed send leaves the previous token current.
    #[must_use]
    pub fn resume_token(&self) -> &str {
        self.proposal.resume_token()
    }
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
            selection_nonce: entity_selection::new_selection_nonce()?,
            world_epoch,
            latest_operation_epoch: 0,
            world_validation,
        })
    }

    /// Consume a private restored boundary into a distinct recovery lineage.
    /// Re-admission moves the existing buffers and charges the new identity;
    /// it neither copies population storage nor changes RNG or chronology.
    pub(crate) fn into_recovery_branch(
        mut self,
        run_id: String,
        policy: &StateAdmissionPolicy,
    ) -> Result<Self, StateError> {
        if !matches!(self.candidate.phase, AuthorityPhase::GenerationBoundary(_))
            || run_id == self.candidate.identity.run_id
        {
            return invalid(
                "recovery.run_id",
                "recovery requires a distinct run at a retained boundary",
            );
        }
        self.candidate.identity.run_id = run_id;
        self.candidate.identity.source_revision = policy.expected_source_revision.clone();
        self.candidate.identity.engine_build_id = policy.expected_engine_build_id.clone();
        self.candidate.identity.source_sha256 = policy.expected_source_sha256.clone();
        self.candidate.identity.target_triple = policy.expected_target_triple.clone();
        self.candidate.identity.build_profile = policy.expected_build_profile.clone();
        self.candidate.identity.build_class = policy.expected_build_class.clone();
        self.candidate.identity.rustc_version = policy.expected_rustc_version.clone();
        self.candidate.identity.build_contract_sha256 =
            policy.expected_build_contract_sha256.clone();
        // Recovery changes build identity, not the recorded scalar/SSE2 arithmetic.
        Self::validate_and_own(self.candidate, self.graph, policy)
    }

    /// Consume an offline source after its checkpoint has passed ordinary admission.
    /// Only canonical population weights are used by the new-run constructor;
    /// world, recurrent state and chronology are deliberately retired here.
    pub(crate) fn into_population_for_new_run(self) -> Vec<PopulationGenome> {
        self.candidate.population
    }

    /// Read the immutable state contract.
    #[must_use]
    pub fn state(&self) -> &StateCandidate {
        &self.candidate
    }

    /// Prepare a small config-only replacement without copying world or population storage.
    pub(crate) fn prepare_live_settings(
        &self,
        updates: &[LiveSettingUpdate],
    ) -> Result<PreparedLiveSettings, LiveSettingsError> {
        prepare_live_settings(
            &self.candidate.config,
            self.candidate.identity.config_revision,
            updates,
        )
    }

    /// Exchange one already prepared config identity with the current authority.
    /// Calling this a second time with the same value restores the prior config.
    pub(crate) fn swap_prepared_live_settings(&mut self, replacement: &mut PreparedLiveSettings) {
        std::mem::swap(&mut self.candidate.config, &mut replacement.config);
        std::mem::swap(
            &mut self.candidate.identity.config_revision,
            &mut replacement.config_revision,
        );
        std::mem::swap(
            &mut self.candidate.identity.config_hash,
            &mut replacement.config_hash,
        );
    }

    /// Apply one prevalidated-address God Mode move without exposing mutable world storage.
    pub(crate) fn apply_god_mode_move(
        &mut self,
        frame_v1_id: u32,
        x: f64,
        y: f64,
    ) -> Result<super::god_mode::GodModeMovePublication, super::god_mode::GodModeError> {
        super::god_mode::apply_god_mode_move(
            &mut self.candidate.world,
            self.candidate.config.world_radius,
            frame_v1_id,
            WorldPoint { x, y },
        )
    }

    /// Prepare and commit one ordinary side-effect-bearing God Mode death.
    pub(crate) fn apply_god_mode_kill(
        &mut self,
        frame_v1_id: u32,
        physics: super::physics::PhysicsConfig,
    ) -> Result<super::god_mode::GodModeKillPublication, super::god_mode::GodModeError> {
        use super::evolved_lives::{EvolvedGenerationState, EvolvedLifeError};
        use super::god_mode::GodModeError;
        let policy = super::step_config::project_evolved_experiment_config(&self.candidate.config)
            .map_err(|_| GodModeError::LifeAccounting(EvolvedLifeError::InvalidPolicy))?;
        let mut staged_lives = if self
            .candidate
            .fixed_step
            .evolved_generation
            .slots
            .is_empty()
        {
            EvolvedGenerationState::initialize(
                &self.candidate.world,
                self.candidate.population.len(),
                physics.movement.snake_start_len,
            )
            .map_err(GodModeError::LifeAccounting)?
        } else {
            self.candidate.fixed_step.evolved_generation.clone()
        };
        let mut prepared = super::god_mode::prepare_god_mode_kill(
            &self.candidate.world,
            &self.candidate.rng,
            &self.candidate.allocators,
            physics,
            frame_v1_id,
            Some(super::effects::EffectLifecyclePolicy {
                experiment: policy,
                lives: &staged_lives,
            }),
        )?;
        let (count, value, discarded_count, discarded_value) = prepared.corpse_summary();
        if let Some(slot) = self
            .candidate
            .world
            .snakes
            .iter()
            .find(|snake| {
                snake.frame_v1_id == frame_v1_id && snake.alive && snake.kind == SnakeKind::Evolved
            })
            .and_then(|snake| snake.population_slot)
        {
            staged_lives
                .finalize_god_mode(
                    slot as usize,
                    (count, value, discarded_count, discarded_value),
                )
                .map_err(GodModeError::LifeAccounting)?;
        }
        // Delay draws and exhaustion belong to this ordered command's commitment,
        // before the next prefix can advance ambient/world RNG.
        staged_lives
            .schedule_deaths(
                policy,
                prepared.rng_mut(),
                self.candidate.generation.completed_step,
                self.candidate.generation.elapsed_seconds,
                self.candidate.config.fixed_step_seconds,
            )
            .map_err(GodModeError::LifeAccounting)?;
        let publication = super::god_mode::commit_god_mode_kill(
            &mut self.candidate.world,
            &mut self.candidate.rng,
            &mut self.candidate.allocators,
            prepared,
        )?;
        self.candidate.fixed_step.evolved_generation = staged_lives;
        Ok(publication)
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

    /// Budget currently available while this state may overlap another owner.
    pub(crate) const fn memory_ceiling_bytes(&self) -> usize {
        self.memory_ceiling_bytes
    }

    /// Restrict a private boundary before it can allocate running-world storage.
    pub(crate) fn restrict_overlap_memory_budget(
        &mut self,
        ceiling: usize,
    ) -> Result<(), StateError> {
        if !matches!(self.candidate.phase, AuthorityPhase::GenerationBoundary(_)) {
            return invalid(
                "memory.overlap",
                "only a private boundary can receive overlap admission",
            );
        }
        let selected = self.memory_ceiling_bytes.min(ceiling);
        if self.memory.total_bytes > selected {
            return Err(StateError::MemoryCeilingExceeded {
                estimated_bytes: self.memory.total_bytes,
                ceiling_bytes: selected,
            });
        }
        self.memory_ceiling_bytes = selected;
        Ok(())
    }

    /// Restore a caller-selected running budget only after predecessor retirement.
    /// This changes no state allocation, RNG, graph, or persisted configuration.
    pub(crate) fn reclaim_retired_memory_budget(
        &mut self,
        ceiling: usize,
    ) -> Result<(), StateError> {
        if self.candidate.phase != AuthorityPhase::Running || ceiling < self.memory_ceiling_bytes {
            return invalid(
                "memory.retirement",
                "budget reclamation requires running authority and a nondecreasing ceiling",
            );
        }
        if self.memory.total_bytes > ceiling {
            return Err(StateError::MemoryCeilingExceeded {
                estimated_bytes: self.memory.total_bytes,
                ceiling_bytes: ceiling,
            });
        }
        self.memory_ceiling_bytes = ceiling;
        Ok(())
    }

    /// Reject an impossible successor numeric allocation before evolution constructs it.
    /// Full successor/state/codec admission remains a separate, stronger gate.
    pub(crate) fn preflight_generation_numeric_allocation(
        &self,
        retained_bytes: usize,
    ) -> Result<(), StateError> {
        let peak = generation_numeric_peak_bytes(
            self.memory.total_bytes,
            self.candidate.population.len(),
            self.graph.compiled(),
        )?
        .checked_add(retained_bytes)
        .ok_or(StateError::ArithmeticOverflow {
            context: "generation numeric peak and retained cutoff observations",
        })?;
        if peak > self.memory_ceiling_bytes {
            return Err(StateError::MemoryCeilingExceeded {
                estimated_bytes: peak,
                ceiling_bytes: self.memory_ceiling_bytes,
            });
        }
        Ok(())
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
        retained_bytes: usize,
    ) -> Result<AdmittedGenerationSuccessor, StateError> {
        self.validate_running_step_key(source_key)?;
        validate_generation_successor_relation(&self.candidate, &candidate)?;
        let remaining_bytes = self
            .memory_ceiling_bytes
            .checked_sub(self.memory.total_bytes)
            .and_then(|bytes| bytes.checked_sub(retained_bytes))
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
            .checked_add(retained_bytes)
            .and_then(|bytes| bytes.checked_add(successor.memory.total_bytes))
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
        let restoring = replacement.persistence_proof.restores_checkpoint();
        if !matches!(self.candidate.phase, AuthorityPhase::GenerationBoundary(_))
            || (!restoring
                && self.candidate.phase
                    != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart))
        {
            return invalid(
                "run_start.phase",
                "initial activation requires the durable run-start boundary",
            );
        }
        if (!restoring
            && (self.candidate.generation.generation != 1
                || self.candidate.generation.completed_step != 0
                || self.candidate.generation.population_epoch != 1))
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

    /// Called only by the retained loop at an unprepared pre-step boundary.
    /// The checked proposal changes one lease without copying population state.
    pub(crate) fn apply_controller_action(
        &mut self,
        input: super::controllers::LatestActionInput,
        boundary_at_ms: u64,
        completed_step: u64,
    ) -> Result<(), String> {
        if self.candidate.phase != AuthorityPhase::Running {
            return Err("controller action requires running authority".to_owned());
        }
        let lease = self
            .candidate
            .world
            .controller_leases
            .iter_mut()
            .find(|lease| lease.id == input.lease_id)
            .ok_or_else(|| "controller action names an unavailable lease".to_owned())?;
        let proposal =
            super::controllers::prepare_queued_latest_action(lease, input, boundary_at_ms)
                .map_err(|error| error.to_string())?;
        let mut reward = lease.reward;
        if let Some(ledger) = reward.as_mut() {
            ledger
                .record_command_received(
                    input.arrival_sequence,
                    input.client_action_id,
                    input.client_tick,
                    completed_step,
                )
                .map_err(|error| error.to_string())?;
        }
        super::controllers::commit_latest_action(lease, proposal)
            .map_err(|error| error.to_string())?;
        lease.reward = reward;
        Ok(())
    }

    /// Resolve one exact close only while the loop owns an unprepared source.
    pub(crate) fn disconnect_controller(
        &mut self,
        lease_id: u64,
        connection_id: u64,
        disconnected_at_ms: u64,
        boundary_at_ms: u64,
    ) -> Result<bool, String> {
        if self.candidate.phase != AuthorityPhase::Running {
            return Err("disconnect requires running authority".to_owned());
        }
        let timing = super::controllers::ControllerTiming::from_config(&self.candidate.config)
            .map_err(|error| error.to_string())?;
        let world = &mut self.candidate.world;
        let Some(lease) = world
            .controller_leases
            .iter_mut()
            .find(|lease| lease.id == lease_id)
        else {
            return Ok(false);
        };
        if lease.connection_id != Some(connection_id)
            || lease.status != ControllerLeaseStatus::Connected
        {
            return Ok(false);
        }
        let snake = world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == lease.snake_id)
            .ok_or_else(|| "controller disconnect lost its assigned snake".to_owned())?;
        let proposal = super::controllers::prepare_queued_disconnect(
            lease,
            snake,
            connection_id,
            disconnected_at_ms,
            boundary_at_ms,
            timing,
        )
        .map_err(|error| error.to_string())?;
        super::controllers::commit_disconnect(lease, snake, proposal)
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    /// Resolve the bounded legacy name before using the same token transaction.
    /// No match permits the enclosing join operation to stage a fresh snake;
    /// ambiguity is an explicit error and never creates another reservation.
    pub fn prepare_legacy_controller_reclaim(
        &self,
        identity_key: &str,
        input: super::controllers::ReclaimInput<'_>,
    ) -> Result<Option<PreparedControllerReclaim>, String> {
        if !input.resume_token.is_empty() {
            return Err("an explicit token must not fall back to legacy identity".into());
        }
        let timing = super::controllers::ControllerTiming::from_config(&self.candidate.config)
            .map_err(|error| error.to_string())?;
        let lease = super::controllers::select_legacy_reclaim(
            &self.candidate.world,
            input.kind,
            input.scope,
            identity_key,
            input.boundary_at_ms,
            timing,
        )
        .map_err(|error| error.to_string())?;
        let Some(lease) = lease else {
            return Ok(None);
        };
        self.prepare_controller_reclaim(super::controllers::ReclaimInput {
            resume_token: &lease.resume_token,
            kind: input.kind,
            scope: input.scope,
            next_resume_token: input.next_resume_token,
            connection_id: input.connection_id,
            arrival_sequence: input.arrival_sequence,
            received_at_ms: input.received_at_ms,
            boundary_at_ms: input.boundary_at_ms,
        })
        .map(Some)
    }

    /// Stage a token reclaim against the complete authority, including global
    /// connection/token uniqueness and the admitted memory ceiling. The caller
    /// must reserve the complete reliable reply before invoking this method.
    pub fn prepare_controller_reclaim(
        &self,
        mut input: super::controllers::ReclaimInput<'_>,
    ) -> Result<PreparedControllerReclaim, String> {
        if self.candidate.phase != AuthorityPhase::Running {
            return Err("reclaim requires running authority".into());
        }
        let leases = &self.candidate.world.controller_leases;
        let lease_index = leases
            .iter()
            .position(|lease| {
                lease.resume_token == input.resume_token
                    && lease.scope == input.scope
                    && lease.kind == input.kind
            })
            .ok_or_else(|| "invalid".to_owned())?;
        let lease = &leases[lease_index];
        if leases.iter().enumerate().any(|(index, other)| {
            other.resume_token == input.next_resume_token
                || (index != lease_index && other.connection_id == Some(input.connection_id))
        }) {
            return Err("reclaim token or connection is already assigned".into());
        }
        let snake_index = self
            .candidate
            .world
            .snakes
            .iter()
            .position(|snake| snake.id == lease.snake_id)
            .ok_or_else(|| "snake-unavailable".to_owned())?;
        let timing = super::controllers::ControllerTiming::from_config(&self.candidate.config)
            .map_err(|error| error.to_string())?;
        // Retained retry copies have this same exact token allocation, so the
        // preflight memory estimate remains exact after receipt correlation.
        input.next_resume_token = input.next_resume_token.into_boxed_str().into_string();
        let next_text = self
            .memory
            .text_bytes
            .checked_sub(lease.resume_token.capacity())
            .and_then(|bytes| bytes.checked_add(input.next_resume_token.capacity()))
            .ok_or_else(|| "reclaim token memory overflow".to_owned())?;
        let mut next_memory = self.memory;
        next_memory.text_bytes = next_text;
        next_memory.total_bytes = self
            .memory
            .total_bytes
            .checked_sub(self.memory.text_bytes)
            .and_then(|bytes| bytes.checked_add(next_text))
            .filter(|bytes| *bytes <= self.memory_ceiling_bytes)
            .ok_or_else(|| "reclaim exceeds admitted authority memory".to_owned())?;
        let snake = &self.candidate.world.snakes[snake_index];
        let proposal = super::controllers::prepare_reclaim(lease, snake, input, timing)
            .map_err(|error| error.to_string())?;
        Ok(PreparedControllerReclaim {
            world_epoch: self.world_epoch,
            operation_epoch: self.latest_operation_epoch,
            completed_step: self.candidate.generation.completed_step,
            lease_index,
            snake_index,
            frame_v1_id: snake.frame_v1_id,
            expected_memory: self.memory,
            next_memory,
            proposal,
        })
    }

    /// Commit only after an exact successful delivery receipt. Rejection leaves
    /// every authoritative field intact; failed sends simply drop preparation.
    pub fn commit_controller_reclaim(
        &mut self,
        prepared: PreparedControllerReclaim,
    ) -> Result<(), String> {
        if self.candidate.phase != AuthorityPhase::Running
            || self.world_epoch != prepared.world_epoch
            || self.latest_operation_epoch != prepared.operation_epoch
            || self.candidate.generation.completed_step != prepared.completed_step
            || self.memory != prepared.expected_memory
        {
            return Err("reclaim source boundary changed".into());
        }
        let world = &mut self.candidate.world;
        if world
            .controller_leases
            .iter()
            .enumerate()
            .any(|(index, lease)| {
                lease.resume_token == prepared.resume_token()
                    || (index != prepared.lease_index
                        && lease.connection_id == Some(prepared.connection_id()))
            })
        {
            return Err("reclaim token or connection changed before delivery".into());
        }
        let lease = world
            .controller_leases
            .get_mut(prepared.lease_index)
            .ok_or_else(|| "reclaim lease disappeared".to_owned())?;
        let snake = world
            .snakes
            .get_mut(prepared.snake_index)
            .ok_or_else(|| "reclaim snake disappeared".to_owned())?;
        super::controllers::commit_reclaim(lease, snake, prepared.proposal)
            .map_err(|error| error.to_string())?;
        self.memory = prepared.next_memory;
        Ok(())
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
        require_exact_build_identity: true,
        expected_source_revision: source.identity.source_revision.clone(),
        expected_engine_build_id: source.identity.engine_build_id.clone(),
        expected_source_sha256: source.identity.source_sha256.clone(),
        expected_target_triple: source.identity.target_triple.clone(),
        expected_build_profile: source.identity.build_profile.clone(),
        expected_build_class: source.identity.build_class.clone(),
        expected_rustc_version: source.identity.rustc_version.clone(),
        expected_build_contract_sha256: source.identity.build_contract_sha256.clone(),
        expected_math_backend: source.identity.math_backend.clone(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
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
        evolved_generation: std::ptr::from_ref(&*replacement.evolved_generation).addr(),
        baseline_lifecycle: std::ptr::from_ref(&*replacement.baseline_lifecycle).addr(),
        ambient_pellet_accumulator: replacement.ambient_pellet_accumulator.to_bits(),
        sensor_generation_best: replacement
            .sensor_generation
            .best_points_this_generation()
            .to_bits(),
        generation_elapsed_seconds: replacement.generation_elapsed_seconds.to_bits(),
        wall_accumulator_seconds: replacement.wall_accumulator_seconds.to_bits(),
        mutation_proof: replacement.mutation.proof_identity(),
        baseline_v2_proof: replacement
            .mutation
            .baseline_v2
            .map(|proof| proof.identity()),
        evolved_mutation_proof: replacement
            .mutation
            .evolved
            .map_or(0, |proof| std::ptr::from_ref(proof).addr()),
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
        || replacement.rng.evolved_respawns != boundary.rng.evolved_respawns
        || replacement.rng.baseline_behaviors != boundary.rng.baseline_behaviors
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
        &mut candidate.fixed_step.evolved_generation,
        replacement.evolved_generation,
    );
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
    if candidate.rng.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION
        && replacement.mutation.baseline_v2.is_none()
    {
        return invalid(
            "fixed_step.baseline_v2",
            "complete v2 control seal is required",
        );
    }
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
    if candidate.rng.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION
        && replacement.mutation.baseline_v2.is_none()
    {
        return invalid(
            "fixed_step.baseline_v2",
            "resolved v2 control seal is required",
        );
    }
    if replacement.mutation.baseline_v2.is_some_and(|proof| {
        !proof.matches(
            replacement.key,
            replacement.rng,
            &*replacement.baseline_lifecycle,
        )
    }) {
        return invalid(
            "fixed_step.baseline_v2",
            "resolved v2 continuation changed after control",
        );
    }

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
    if replacement.mutation.evolved.is_some_and(|proof| {
        !proof.matches(
            replacement.key,
            candidate,
            replacement.world,
            replacement.rng,
            replacement.allocators,
        )
    }) {
        return invalid(
            "fixed_step.evolved_replacement",
            "private evolved witness does not match source or staged identity/RNG",
        );
    }
    // Once initialized, a live continuation cannot silently drop its ledger or
    // reassign a population owner. Each evolved replacement carries its own
    // narrow witness independently of external replacement permissions.
    let source_lives = &candidate.fixed_step.evolved_generation;
    if !source_lives.slots.is_empty() {
        replacement
            .evolved_generation
            .validate_event_continuation(source_lives)
            .map_err(|error| StateError::InvalidField {
                field: "fixed_step.evolved_generation.life_ends",
                reason: error.to_string(),
            })?;
        if replacement.evolved_generation.version != source_lives.version
            || replacement.evolved_generation.slots.len() != source_lives.slots.len()
        {
            return invalid(
                "fixed_step.evolved_generation",
                "running life continuation was removed or changed version",
            );
        }
        for (source, staged) in source_lives
            .slots
            .iter()
            .zip(&replacement.evolved_generation.slots)
        {
            let record = replacement
                .mutation
                .evolved
                .and_then(|proof| proof.record(source.population_slot));
            let identity_valid = if let Some(record) = record {
                source.physical_id == record.old_id
                    && staged.physical_id == record.new_id
                    && source.lives_started.checked_add(1) == Some(staged.lives_started)
                    && source.successful_respawns.checked_add(1) == Some(staged.successful_respawns)
            } else {
                source.physical_id == staged.physical_id
                    && source.lives_started == staged.lives_started
                    && source.successful_respawns == staged.successful_respawns
            };
            if source.population_slot != staged.population_slot
                || !identity_valid
                || staged.deaths < source.deaths
                || staged.ledger.operations < source.ledger.operations
                || staged.ledger.generation_gross_earned < source.ledger.generation_gross_earned
                || staged.ledger.generation_boost_spent < source.ledger.generation_boost_spent
            {
                return invalid(
                    "fixed_step.evolved_generation",
                    "stable population life or monotonic accounting changed",
                );
            }
        }
    }
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
        || (replacement.mutation.baseline_v2.is_none()
            && replacement.rng.baseline_behaviors != candidate.rng.baseline_behaviors)
        || replacement.rng.evolution != candidate.rng.evolution
        || (replacement.mutation.evolved.is_none()
            && replacement.rng.evolved_respawns != candidate.rng.evolved_respawns)
    {
        return invalid(
            "fixed_step.rng",
            "nonterminal steps cannot replace RNG identity or advance evolution",
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
        || staged.next_baseline_id < source.next_baseline_id
    {
        return invalid(
            "fixed_step.allocators",
            "monotonic gameplay allocator regressed",
        );
    }
    // Public IDs are circular. Reservation proofs and world validation enforce live uniqueness;
    // unlike durable counters, their next candidate need not exceed every existing ID.
    validate_allocators(staged)?;
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
                && source.retained_origin == staged.retained_origin
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
pub(crate) fn decode_sha256_identity(
    field: &'static str,
    identity: &str,
) -> Result<[u8; 32], StateError> {
    let Some(hex) = identity.strip_prefix("sha256:") else {
        return invalid(field, "must use the sha256:<64 lowercase hex> form");
    };
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return invalid(field, "must contain exactly 64 lowercase hex digits");
    }
    let mut output = [0_u8; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
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

/// Minimum current-plus-successor numeric peak, checked before any output weights exist.
pub(crate) fn generation_numeric_peak_bytes(
    current_bytes: usize,
    population: usize,
    graph: &CompiledGraph,
) -> Result<usize, StateError> {
    let scalars = graph
        .total_parameters
        .checked_add(graph.total_state_size)
        .and_then(|per_genome| per_genome.checked_mul(population))
        .ok_or(StateError::ArithmeticOverflow {
            context: "next generation numeric shape",
        })?;
    checked_add(
        current_bytes,
        checked_allocation_bytes(scalars, size_of::<f32>(), "next generation numeric bytes")?,
        "current plus next generation numeric memory",
    )
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
    // Conservatively reserve one Arc allocation per possible retained owner,
    // including its reference counters; shared copies may only reduce this bound.
    estimate.structural_bytes = checked_add(
        estimate.structural_bytes,
        checked_allocation_bytes(
            candidate.config.max_non_population_brains,
            size_of::<RetainedNeuralOrigin>() + ARC_COUNTER_BYTES,
            "retained brain origins",
        )?,
        "retained origin memory",
    )?;
    add_structural::<BaselineRngState>(&mut estimate, candidate.rng.baselines.capacity())?;
    add_structural::<super::rng::BaselineBehaviorRngState>(
        &mut estimate,
        candidate.rng.baseline_behaviors.capacity().max(
            if candidate.rng.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION {
                candidate.config.baseline_count
            } else {
                0
            },
        ),
    )?;
    add_structural::<EvolvedRespawnRngState>(
        &mut estimate,
        candidate.rng.evolved_respawns.capacity().max(
            if candidate.rng.version >= RNG_BUNDLE_WITH_EVOLVED_VERSION {
                candidate.config.population_count
            } else {
                0
            },
        ),
    )?;
    add_structural::<super::evolved_lives::EvolvedSlotRuntime>(
        &mut estimate,
        candidate
            .fixed_step
            .evolved_generation
            .slots
            .capacity()
            .max(candidate.config.population_count),
    )?;
    add_structural::<super::evolved_lives::EvolvedLifeEndRecord>(
        &mut estimate,
        candidate
            .fixed_step
            .evolved_generation
            .transaction_life_ends
            .capacity()
            .max(candidate.config.population_count),
    )?;
    add_structural::<super::evolved_lives::EvolvedLifeEndRecord>(
        &mut estimate,
        candidate
            .fixed_step
            .evolved_generation
            .recent_life_ends
            .capacity()
            .max(super::evolved_lives::MAX_RECENT_LIFE_ENDS),
    )?;
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
    if let Some(backend) =
        super::inference::InferenceMathBackend::from_label(&candidate.identity.math_backend)
            .filter(|backend| backend.uses_i16_cache())
    {
        let cache_owners = candidate
            .config
            .population_count
            .checked_add(candidate.config.max_non_population_brains)
            .ok_or(StateError::ArithmeticOverflow {
                context: "i16 cache owners",
            })?;
        let cache_bytes = if backend == super::inference::InferenceMathBackend::I16MatrixFma128 {
            super::inference::matrix_i16_cache_admission_bytes(compiled, cache_owners)
        } else {
            super::inference::i16_cache_admission_bytes(compiled, cache_owners, backend)
        }
        .ok_or(StateError::ArithmeticOverflow {
            context: "i16 inference cache",
        })?;
        estimate.scratch_bytes =
            checked_add(estimate.scratch_bytes, cache_bytes, "i16 inference cache")?;
    }
    // The species norm cache extends the previous slot/count summary scratch.
    estimate.scratch_bytes = checked_add(
        estimate.scratch_bytes,
        checked_allocation_bytes(
            candidate.config.population_count,
            super::evolution::SPECIES_NORM_BYTES_PER_GENOME,
            "species norm scratch",
        )?,
        "species norm scratch",
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
        || candidate_shell.brains.iter().any(|brain| {
            brain.non_population_weights.is_some()
                || brain.retained_origin.is_some()
                || !brain.recurrent.is_empty()
        })
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
    if candidate.versions.rng_bundle != candidate.rng.version {
        return invalid(
            "versions.rng_bundle",
            "recorded RNG version differs from its payload",
        );
    }
    validate_identity(&candidate.identity, policy)?;
    validate_config(
        &candidate.config,
        &candidate.versions,
        &candidate.identity,
        policy,
    )?;
    if candidate.versions.sensor == 4
        || candidate
            .config
            .settings
            .iter()
            .any(|entry| entry.path == "sense.contractVersion")
    {
        let contract = super::sensor_contract::SensorContractIdentity::from_settings(
            &candidate.config.settings,
        )
        .map_err(|error| StateError::InvalidField {
            field: "config.sensor_contract",
            reason: error.to_string(),
        })?;
        let width = graph
            .nodes
            .iter()
            .find(|node| node.node_type == super::graph::CompiledNodeType::Input)
            .map(|node| node.output_size);
        if contract.layout_version() != candidate.versions.sensor
            || width != Some(contract.input_size())
        {
            return invalid(
                "config.sensor_contract",
                "version or graph Input does not match structural sensor contract",
            );
        }
    }
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
    if policy.allowed_math_backends.len() > 3
        || policy
            .allowed_math_backends
            .iter()
            .enumerate()
            .any(|(index, backend)| {
                policy.allowed_math_backends[..index].contains(backend)
                    || backend == &policy.expected_math_backend
                    || super::inference::InferenceMathBackend::from_label(backend).is_none()
            })
    {
        return invalid(
            "admission.allowed_math_backends",
            "at most three distinct supported arithmetic alternatives",
        );
    }
    validate_sha256(
        "admission.expected_settings_schema_sha256",
        &policy.expected_settings_schema_sha256,
    )?;
    // Frozen/life and v4/v2 families, including explicitly recorded ray budgets.
    // One primary hash plus nine alternatives keeps admission finite.
    if policy.allowed_settings_schema_sha256.len() > 9 {
        return invalid(
            "admission.allowed_settings_schema_sha256",
            "at most nine explicitly admitted alternative schemas",
        );
    }
    for schema in &policy.allowed_settings_schema_sha256 {
        validate_sha256("admission.allowed_settings_schema_sha256", schema)?;
    }
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
    if !matches!(versions.sensor, 3 | 4) {
        return invalid("versions.sensor", "unsupported sensor version");
    }
    if !matches!(
        versions.rng_bundle,
        RNG_BUNDLE_VERSION | RNG_BUNDLE_WITH_EVOLVED_VERSION | RNG_BUNDLE_WITH_BASELINE_V2_VERSION
    ) {
        return invalid("versions.rng_bundle", "unsupported RNG-bundle version");
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
    if let Some(origin) = &identity.legacy_conversion {
        origin
            .validate()
            .map_err(|reason| StateError::InvalidField {
                field: "identity.legacy_conversion",
                reason: reason.into(),
            })?;
    }
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
    let incompatible = identity.target_triple != policy.expected_target_triple
        || identity.build_profile != policy.expected_build_profile
        || identity.build_class != policy.expected_build_class
        || (identity.math_backend != policy.expected_math_backend
            && !policy
                .allowed_math_backends
                .contains(&identity.math_backend));
    let not_exact = identity.source_revision != policy.expected_source_revision
        || identity.engine_build_id != policy.expected_engine_build_id
        || identity.source_sha256 != policy.expected_source_sha256
        || identity.rustc_version != policy.expected_rustc_version
        || identity.build_contract_sha256 != policy.expected_build_contract_sha256;
    if incompatible || (policy.require_exact_build_identity && not_exact) {
        return invalid(
            "identity",
            "checkpoint build identity is not admitted by the loaded engine",
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
        || (actual_schema != policy.expected_settings_schema_sha256
            && !policy
                .allowed_settings_schema_sha256
                .contains(&actual_schema))
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
    if continuation.evolved_generation.version != super::evolved_lives::EVOLVED_LIVES_VERSION {
        return invalid(
            "fixed_step.evolved_generation",
            "unsupported life accounting version",
        );
    }
    continuation
        .evolved_generation
        .validate_events()
        .map_err(|error| StateError::InvalidField {
            field: "fixed_step.evolved_generation.life_ends",
            reason: error.to_string(),
        })?;
    let placement = continuation.evolved_generation.placement_work;
    placement
        .validate()
        .map_err(|error| StateError::InvalidField {
            field: "fixed_step.evolved_generation.placement_work",
            reason: error.to_string(),
        })?;
    if (continuation.evolved_generation.slots.is_empty()
        && placement != super::evolved_lives::EvolvedPlacementWork::default())
        || (placement.step != 0 && placement.step != candidate.generation.completed_step)
    {
        return invalid("fixed_step.evolved_generation.placement_work", "placement report must belong to the committed step and be empty before life initialization");
    }
    if !continuation.evolved_generation.slots.is_empty() {
        if matches!(candidate.phase, AuthorityPhase::GenerationBoundary(_)) {
            return invalid(
                "fixed_step.evolved_generation",
                "pre-spawn boundary cannot contain physical lives",
            );
        }
        continuation
            .evolved_generation
            .validate(&candidate.world, candidate.config.population_count)
            .map_err(|error| StateError::InvalidField {
                field: "fixed_step.evolved_generation",
                reason: error.to_string(),
            })?;
    }
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
    let v2 = super::baseline_v2::BaselineV2Settings::from_settings(&candidate.config.settings)
        .map_err(|_| StateError::InvalidField {
            field: "baselineBots.controlVersion",
            reason: "invalid baseline-v2 settings".into(),
        })?
        .is_some();
    for slot in &continuation.baseline_lifecycle.slots {
        if slot.v2_policy.is_some() != v2
            || slot
                .v2_last_control_step
                .is_some_and(|step| step == 0 || step > candidate.generation.completed_step)
        {
            return invalid(
                "fixed_step.baseline_lifecycle",
                "policy/cadence continuation differs from baseline contract",
            );
        }
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
    if !matches!(
        bundle.version,
        RNG_BUNDLE_VERSION | RNG_BUNDLE_WITH_EVOLVED_VERSION | RNG_BUNDLE_WITH_BASELINE_V2_VERSION
    ) {
        return invalid("rng.version", "unsupported RNG-bundle version");
    }
    if bundle.version == RNG_BUNDLE_VERSION && !bundle.evolved_respawns.is_empty() {
        return invalid(
            "rng.evolved_respawns",
            "frozen RNG bundle cannot carry evolved streams",
        );
    }
    if bundle.version >= RNG_BUNDLE_WITH_EVOLVED_VERSION {
        if bundle.evolved_respawns.len() != config.population_count {
            return invalid("rng.evolved_respawns", "count must match the population");
        }
        for (slot, stream) in bundle.evolved_respawns.iter().enumerate() {
            if stream.slot as usize != slot || stream.runtime().is_err() {
                return invalid(
                    "rng.evolved_respawns",
                    "evolved streams must be dense and nonzero",
                );
            }
        }
    }
    let v2 =
        super::baseline_v2::BaselineV2Settings::from_settings(&config.settings).map_err(|_| {
            StateError::InvalidField {
                field: "baselineBots.controlVersion",
                reason: "incomplete or incompatible baseline-v2 settings".into(),
            }
        })?;
    if v2.is_some() != (bundle.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION) {
        return invalid(
            "rng.version",
            "baseline-v2 settings require exactly RNG bundle v3",
        );
    }
    if bundle.version < RNG_BUNDLE_WITH_BASELINE_V2_VERSION && !bundle.baseline_behaviors.is_empty()
    {
        return invalid(
            "rng.baseline_behaviors",
            "frozen RNG bundles cannot carry behavior streams",
        );
    }
    if v2.is_some() {
        if bundle.baseline_behaviors.len() != config.baseline_count {
            return invalid("rng.baseline_behaviors", "count must match baseline slots");
        }
        for (slot, stream) in bundle.baseline_behaviors.iter().enumerate() {
            if stream.slot as usize != slot || stream.runtime().is_err() {
                return invalid(
                    "rng.baseline_behaviors",
                    "behavior slots must be dense and nonzero",
                );
            }
        }
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
                if brain.non_population_weights.is_some() || brain.retained_origin.is_some() {
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
                if let Some(origin) = &brain.retained_origin {
                    origin.validate_weights(weights).map_err(|reason| {
                        StateError::InvalidField {
                            field: "brains.retained_origin",
                            reason,
                        }
                    })?;
                }
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
                    && (brain.retained_origin.is_some() == (snake.kind == SnakeKind::Resurrected))
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
        (
            "world.snakes.completed_step_heading_delta",
            snake.completed_step_heading_delta,
        ),
        (
            "world.snakes.completed_step_speed_delta",
            snake.completed_step_speed_delta,
        ),
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
        || !snake.completed_step_applied_turn.is_finite()
        || !(-1.0..=1.0).contains(&snake.completed_step_applied_turn)
        || snake.completed_step_heading_delta.abs() > std::f64::consts::PI
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
    if snake.kind == SnakeKind::Resurrected && snake.brain.is_none() {
        return invalid(
            "world.snakes.brain",
            "retained neural snake requires an owned brain and origin",
        );
    }
    if let Some(handle) = snake.brain {
        let found = candidate.brains.iter().any(|brain| {
            brain.handle == handle
                && brain.owner == BrainOwner::Entity(snake.id)
                && (brain.retained_origin.is_some() == (snake.kind == SnakeKind::Resurrected))
        });
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
    if lease.identity_key.len() > 128 || lease.identity_key.contains('\0') {
        return invalid(
            "controller_lease.identity_key",
            "legacy identity exceeds 128 UTF-8 bytes or contains NUL",
        );
    }
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
    if candidate.world.ray_cursor != super::ray_work::RayQuotaCursor::default()
        || !candidate.world.snakes.is_empty()
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

pub(crate) fn validate_f32_slice(
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
    if let Some(origin) = &candidate.identity.legacy_conversion {
        for text in [
            &origin.source_run_id,
            &origin.source_generation,
            &origin.source_sha256,
        ]
        .into_iter()
        .flatten()
        {
            add_text(estimate, text)?;
        }
    }
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
        add_text(estimate, &lease.identity_key)?;
        add_text(estimate, &lease.resume_token)?;
    }
    Ok(())
}

pub(crate) fn estimate_graph_memory(bundle: &GraphBundle) -> Result<usize, StateError> {
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
mod tests;
