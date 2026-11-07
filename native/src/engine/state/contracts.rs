//! Persistent state shapes and allocator continuations.

use super::*;

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
    /// Explicit saved-brain source for this evolving lineage, independent of live membership.
    pub founder: Option<crate::engine::founder::FounderOrigin>,
    /// Partial source provenance retained independently of this new run's exact continuation.
    pub legacy_conversion: Option<crate::engine::legacy_origin::LegacyPopulationOrigin>,
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
    pub baseline_behaviors: Vec<crate::engine::rng::BaselineBehaviorRngState>,
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
        let serialized = crate::engine::rng::derive_evolved_respawn_rngs(seed, generation, count)
            .map_err(|error| StateError::InvalidRng {
            stream: "evolved-respawn".into(),
            reason: error.to_string(),
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
    pub reward: Option<crate::engine::external_reward::ExternalRewardLedger>,
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
    pub ray_cursor: crate::engine::ray_work::RayQuotaCursor,
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
    pub evolved_generation: crate::engine::evolved_lives::EvolvedGenerationState,
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
            evolved_generation: crate::engine::evolved_lives::EvolvedGenerationState::default(),
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
