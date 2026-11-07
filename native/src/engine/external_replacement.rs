//! Atomic staging for externally controlled snakes that die during a fixed step.
//!
//! The temporary TypeScript runtime creates a new genome and body before it
//! attempts to send the replacement assignment.  The Rust migration keeps that
//! ordering, but isolates every draw to the external-controller RNG, allocates
//! fresh exact identities, checks the complete body against live geometry, and
//! exposes no result until the whole replacement set is ready.  A failed local
//! assignment send disconnects the already-staged replacement under its fresh
//! token. The old token cannot silently reclaim a different snake; recovery
//! requires an explicit fresh join. A player and brain never steer together.

use super::control_phase::{copy_brains_reusing, ControlPhaseError};
use super::controllers::{
    commit_disconnect_prevalidated, prepare_disconnect, validate_disconnect_proposal,
    ControllerError, ControllerTiming, DisconnectProposal,
};
use super::fixed_step::{
    copy_rng_bundle_reusing, copy_serialized_rng_reusing, copy_world_reusing, FixedStepPrefixError,
    RngCopyScratch,
};
use super::genome::{
    initialize_random_genome, GenomeInitializationConfig, GenomeInitializationError,
};
use super::graph::CompiledGraph;
use super::physics::PhysicsStepKey;
use super::spawn::{
    SpawnCapacityDiagnostics, SpawnConfig, SpawnDomain, SpawnError, SpawnKey, SpawnRequest,
    SpawnWorkspace,
};
use super::state::{
    AllocatorState, BodyRange, BrainHandle, BrainOwner, BrainRuntimeState, ControllerKind,
    ControllerLeaseStatus, LatestControllerAction, RngStateBundle, SnakeKind, SnakeState,
    StateError, WorldPoint, WorldState,
};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};

mod authority_digest;
#[allow(unused_imports)]
pub(crate) use authority_digest::authority_world_digest;
use authority_digest::*;
mod lease_validation;
use lease_validation::*;
#[allow(unused_imports)]
pub(crate) use lease_validation::{fresh_resume_token, RESUME_TOKEN_LENGTH};

/// First external death-replacement staging contract.
pub const EXTERNAL_REPLACEMENT_VERSION: u32 = 1;
/// Current browser skin for an ordinary externally controlled snake.
const EXTERNAL_SNAKE_SKIN: u32 = 0;

/// Borrowed admitted boundary for one new external snake. No source field is
/// changed while a join assignment is being prepared or delivered.
pub struct FreshExternalSource<'a> {
    pub world: &'a WorldState,
    pub brains: &'a [BrainRuntimeState],
    pub rng: &'a RngStateBundle,
    pub allocators: &'a AllocatorState,
    pub graph: &'a CompiledGraph,
    pub brain_epoch: u64,
}

/// Detached gameplay portion of a fresh join. The enclosing authority must
/// correlate the source boundary and successful delivery before appending it.
#[derive(Debug)]
pub struct PreparedFreshExternal {
    pub snake: SnakeState,
    pub body: Vec<WorldPoint>,
    pub brain: BrainRuntimeState,
    pub lease_id: u64,
    pub next_allocators: AllocatorState,
    pub next_external_rng: super::rng::SerializedRngState,
}

/// Prepare only one bounded new snake, using the same genome, placement, and
/// initial controls as death replacement. Failure and retry consume no source
/// RNG or IDs and never copy the existing population or world.
pub fn prepare_fresh_external(
    source: FreshExternalSource<'_>,
    config: ExternalReplacementConfig,
) -> Result<PreparedFreshExternal, ExternalReplacementError> {
    config.validate()?;
    let required_snakes = source.world.snakes.len().saturating_add(1);
    let required_brains = source.brains.len().saturating_add(1);
    let required_body = source
        .world
        .body_points
        .len()
        .saturating_add(config.spawn.snake_start_len);
    if required_snakes > config.maximum_snakes {
        return Err(ExternalReplacementError::SnakeCapacityExceeded {
            required: required_snakes,
            maximum: config.maximum_snakes,
        });
    }
    if required_brains > config.maximum_brains {
        return Err(ExternalReplacementError::BrainCapacityExceeded {
            required: required_brains,
            maximum: config.maximum_brains,
        });
    }
    if required_body > config.maximum_body_points {
        return Err(ExternalReplacementError::BodyCapacityExceeded {
            required: required_body,
            maximum: config.maximum_body_points,
        });
    }
    let mut next_allocators = source.allocators.clone();
    let allocate = |error| ExternalReplacementError::Allocator(Box::new(error));
    let snake_id = next_allocators
        .reserve_external_ids(1)
        .map_err(allocate)?
        .ok_or(ExternalReplacementError::InternalShapeMismatch)?
        .first;
    let frame_v1_id = next_allocators
        .reserve_frame_v1_ids(1, source.world.snakes.iter().map(|snake| snake.frame_v1_id))
        .map_err(allocate)?
        .ok_or(ExternalReplacementError::InternalShapeMismatch)?
        .first;
    let brain_handle = BrainHandle {
        id: next_allocators
            .reserve_brain_ids(1)
            .map_err(allocate)?
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?
            .first,
        epoch: source.brain_epoch,
    };
    let lease_id = next_allocators
        .reserve_controller_lease_ids(1)
        .map_err(allocate)?
        .ok_or(ExternalReplacementError::InternalShapeMismatch)?
        .first;
    let initialized =
        initialize_random_genome(source.graph, &source.rng.external_controller, config.genome)
            .map_err(|error| ExternalReplacementError::Genome(Box::new(error)))?;
    let (weights, post_genome_rng) = initialized.into_parts();
    let requests = [SpawnRequest {
        key: SpawnKey {
            domain: SpawnDomain::External,
            slot: snake_id,
        },
    }];
    let mut workspace = SpawnWorkspace::default();
    let prepared = workspace
        .prepare(
            source.world,
            &requests,
            &post_genome_rng,
            config.spawn,
            config.spawn.snake_start_len,
        )
        .map_err(|error| ExternalReplacementError::Spawn(Box::new(error)))?;
    let placement = prepared
        .placements()
        .first()
        .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
    let points = prepared
        .body_for(placement)
        .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
    let mut body = Vec::new();
    body.try_reserve_exact(points.len()).map_err(|_| {
        ExternalReplacementError::AllocationFailed {
            buffer: "fresh external body",
            required: points.len(),
        }
    })?;
    body.extend_from_slice(points);
    let snake = initial_external_snake(
        snake_id,
        frame_v1_id,
        brain_handle,
        placement,
        BodyRange {
            start: source.world.body_points.len(),
            len: body.len(),
        },
        config,
    );
    let brain = BrainRuntimeState {
        handle: brain_handle,
        owner: BrainOwner::Entity(snake_id),
        retained_origin: None,
        non_population_weights: Some(weights.into_boxed_slice()),
        recurrent: try_zero_f32_box(
            source.graph.total_state_size,
            "fresh external recurrent state",
        )?,
    };
    Ok(PreparedFreshExternal {
        snake,
        body,
        brain,
        lease_id,
        next_allocators,
        next_external_rng: prepared.next_rng().clone(),
    })
}

/// Shared neutral initial state for fresh joins and death replacements.
fn initial_external_snake(
    snake_id: u64,
    frame_v1_id: u32,
    brain_handle: BrainHandle,
    placement: &super::spawn::SpawnPlacement,
    body: BodyRange,
    config: ExternalReplacementConfig,
) -> SnakeState {
    SnakeState {
        id: snake_id,
        frame_v1_id,
        kind: SnakeKind::External,
        alive: true,
        population_slot: None,
        brain: Some(brain_handle),
        baseline_slot: None,
        baseline_strategy: None,
        position: placement.head,
        previous_position: placement.head,
        direction: placement.direction,
        radius: config.spawn.snake_radius,
        speed: config.snake_base_speed,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: config.spawn.snake_start_len as f64,
        fitness: 0.0,
        turn: 0.0,
        previous_turn: 0.0,
        input_boost: false,
        previous_input_boost: false,
        control_accumulator_seconds: 0.0,
        delivered_observation_points: 0.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body,
        skin: EXTERNAL_SNAKE_SKIN,
    }
}

/// Complete admitted settings and ceilings for one replacement batch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalReplacementConfig {
    /// Authoritative fixed-step duration for the replacement's birth boundary.
    pub fixed_step_seconds: f64,
    /// Versioned transaction shape.
    pub version: u32,
    /// Collision-safe complete-body placement contract.
    pub spawn: SpawnConfig,
    /// Current TypeScript-compatible random genome formulas.
    pub genome: GenomeInitializationConfig,
    /// Initial movement speed of a replacement.
    pub snake_base_speed: f64,
    /// Owner-selected wall-time controller rules.
    pub controller_timing: ControllerTiming,
    /// Maximum admitted snake records after replacement.
    pub maximum_snakes: usize,
    /// Maximum admitted packed body points after compaction.
    pub maximum_body_points: usize,
    /// Maximum admitted brain records after replacement.
    pub maximum_brains: usize,
}

impl ExternalReplacementConfig {
    /// Current gameplay values with provisional bounded spawn work.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            fixed_step_seconds: 1.0 / 60.0,
            version: EXTERNAL_REPLACEMENT_VERSION,
            spawn: SpawnConfig::typescript_geometry_defaults(),
            genome: GenomeInitializationConfig::typescript_defaults(),
            snake_base_speed: 165.0,
            controller_timing: ControllerTiming::approved_defaults(),
            maximum_snakes: 1_000,
            maximum_body_points: 1_000_000,
            maximum_brains: 1_000,
        }
    }

    pub(crate) fn validate(self) -> Result<(), ExternalReplacementError> {
        if !self.fixed_step_seconds.is_finite()
            || self.fixed_step_seconds <= 0.0
            || self.fixed_step_seconds > 1.0
            || self.version != EXTERNAL_REPLACEMENT_VERSION
            || !self.snake_base_speed.is_finite()
            || self.snake_base_speed < 0.0
            || self.maximum_snakes == 0
            || self.maximum_body_points == 0
            || self.maximum_brains == 0
        {
            return Err(ExternalReplacementError::InvalidConfig);
        }
        self.spawn
            .validate()
            .map_err(|error| ExternalReplacementError::Spawn(Box::new(error)))?;
        self.genome
            .validate()
            .map_err(|error| ExternalReplacementError::Genome(Box::new(error)))?;
        ControllerTiming::new(
            self.controller_timing.input_hold_ms(),
            self.controller_timing.disconnect_grace_ms(),
        )
        .map_err(|error| ExternalReplacementError::Controller(Box::new(error)))?;
        Ok(())
    }
}

impl Default for ExternalReplacementConfig {
    fn default() -> Self {
        Self::typescript_defaults()
    }
}

/// Local send state for one reliable replacement assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignmentDeliveryStatus {
    /// No matching Node result has resolved this assignment.
    Pending,
    /// Node accepted the assignment into the exact live socket send path.
    Accepted,
    /// The local send failed and the replacement entered disconnect grace.
    Failed,
}

/// One reliable assignment produced by a complete replacement batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplacementAssignment {
    /// Lease epoch that owned the snake that died.
    pub source_lease_id: u64,
    /// Fresh assignment/lease epoch for the replacement.
    pub lease_id: u64,
    /// Exact still-live socket epoch.
    pub connection_id: u64,
    /// Browser player or separate Protocol 2 RL controller.
    pub controller_kind: ControllerKind,
    /// Fresh exact internal snake identity.
    pub snake_id: u64,
    /// Fresh exact frame-v1/client identity.
    pub frame_v1_id: u32,
    token_index: usize,
}

/// Why an old disconnected controller receives no snake in the replacement
/// world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableControllerReason {
    /// Its prior snake ceased to exist before grace-time reclaim.
    SnakeUnavailable,
    /// Grace had already expired and neural takeover was committed.
    GraceExpired,
}

/// Small token-scoped handoff record for a disconnected old controller whose
/// snake is absent from the replacement world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableControllerReservation {
    /// Old assignment/lease epoch.
    pub source_lease_id: u64,
    /// Old snake identity that is no longer available.
    pub source_snake_id: u64,
    /// Browser player or Protocol 2 RL client.
    pub controller_kind: ControllerKind,
    /// Run/session scope preventing cross-run reclaim.
    pub scope: String,
    /// Old reclaim token whose failure must be explicit.
    pub resume_token: String,
    /// Wall-clock disconnect boundary, when retained.
    pub disconnected_at_ms: Option<u64>,
    /// End of exclusive reclaim grace, when retained.
    pub grace_expires_at_ms: Option<u64>,
    /// Exact unavailable/expired result category.
    pub reason: UnavailableControllerReason,
}

/// Opaque process-local proof of the exact buffers produced by this workspace.
///
/// The fields are deliberately private to this module. The final authority
/// boundary can verify the exact staged world, RNG, allocator, and brain bytes,
/// but no sibling module can manufacture a proof for arbitrary buffers.
#[derive(Debug)]
pub(crate) struct ExternalReplacementAuthorityProof {
    key: PhysicsStepKey,
    replacements: usize,
    removed_dead_leases: usize,
    world_sha256: [u8; 32],
    rng_sha256: [u8; 32],
    allocators: AllocatorState,
    brains_sha256: [u8; 32],
}

impl ExternalReplacementAuthorityProof {
    #[must_use]
    pub(crate) const fn replacements(&self) -> usize {
        self.replacements
    }

    #[must_use]
    pub(crate) const fn removed_dead_leases(&self) -> usize {
        self.removed_dead_leases
    }

    #[must_use]
    pub(crate) fn matches(
        &self,
        key: PhysicsStepKey,
        world: &WorldState,
        rng: &RngStateBundle,
        allocators: &AllocatorState,
        brains: &[BrainRuntimeState],
    ) -> bool {
        self.key == key
            && self.world_sha256 == authority_world_digest(world)
            && self.rng_sha256 == authority_rng_digest(rng)
            && self.allocators == *allocators
            && self.brains_sha256 == authority_brain_digest(brains)
    }
}

/// Work and retained-capacity evidence for one replacement preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExternalReplacementDiagnostics {
    /// Connected dead leases replaced and awaiting/reusing assignments.
    pub replacements: usize,
    /// Dead disconnected/taken-over leases removed without replacement.
    pub removed_dead_leases: usize,
    /// Old pellet-owner references cleared after identity replacement.
    pub cleared_pellet_owners: usize,
    /// Total random/fallback candidates examined across replacements.
    pub candidates_examined: usize,
    /// Total placements supplied by deterministic fallback.
    pub fallback_placements: usize,
    /// Total wall/body geometry comparisons across replacements.
    pub geometry_checks: usize,
    /// Latest single-replacement spawn capacities and counts.
    pub spawn: SpawnCapacityDiagnostics,
    /// Retained stable lease-order capacity.
    pub lease_order_capacity: usize,
    /// Retained replacement-record capacity.
    pub assignment_capacity: usize,
    /// Retained delivery-status capacity.
    pub status_capacity: usize,
    /// Retained token-record capacity.
    pub token_capacity: usize,
    /// Retained unavailable-controller handoff capacity.
    pub unavailable_reservation_capacity: usize,
    /// Retained temporary replacement-body capacity.
    pub replacement_body_capacity: usize,
    /// Retained alternate packed-body capacity.
    pub compacted_body_capacity: usize,
    /// Retained working snake capacity.
    pub snake_capacity: usize,
    /// Retained working brain capacity.
    pub brain_capacity: usize,
}

/// Complete read-only replacement result before any authority swap.
#[derive(Debug)]
pub struct PreparedExternalReplacements<'workspace, 'source> {
    key: PhysicsStepKey,
    source_world: &'source WorldState,
    source_rng: &'source RngStateBundle,
    source_allocators: &'source AllocatorState,
    source_brains: &'source [BrainRuntimeState],
    assignments: &'workspace [ReplacementAssignment],
    statuses: &'workspace [AssignmentDeliveryStatus],
    tokens: &'workspace [String],
    unavailable_reservations: &'workspace [UnavailableControllerReservation],
    diagnostics: ExternalReplacementDiagnostics,
}

/// Mutable replacement buffers borrowed only by the complete world-step owner.
pub(crate) struct ExternalReplacementBuffers<'workspace> {
    /// Complete post-replacement world.
    pub world: &'workspace mut WorldState,
    /// RNG bundle whose external-controller stream alone advanced.
    pub rng: &'workspace mut RngStateBundle,
    /// Exact deterministic allocator continuation.
    pub allocators: &'workspace mut AllocatorState,
    /// Brain records after fresh external brains replaced dead owners.
    pub brains: &'workspace mut Vec<BrainRuntimeState>,
    /// Opaque proof bound to these exact provisional buffers.
    pub proof: &'workspace ExternalReplacementAuthorityProof,
}

impl PreparedExternalReplacements<'_, '_> {
    /// Exact fixed-step operation whose deaths were resolved.
    #[must_use]
    pub const fn key(&self) -> PhysicsStepKey {
        self.key
    }

    /// Immutable post-physics source boundary.
    #[must_use]
    pub const fn source_world(&self) -> &WorldState {
        self.source_world
    }

    /// Immutable post-physics RNG boundary.
    #[must_use]
    pub const fn source_rng(&self) -> &RngStateBundle {
        self.source_rng
    }

    /// Immutable post-physics allocator boundary.
    #[must_use]
    pub const fn source_allocators(&self) -> &AllocatorState {
        self.source_allocators
    }

    /// Immutable post-control brain boundary.
    #[must_use]
    pub const fn source_brains(&self) -> &[BrainRuntimeState] {
        self.source_brains
    }

    /// Canonically ordered reliable assignment metadata.
    #[must_use]
    pub const fn assignments(&self) -> &[ReplacementAssignment] {
        self.assignments
    }

    /// Current first-result-wins assignment states.
    #[must_use]
    pub const fn statuses(&self) -> &[AssignmentDeliveryStatus] {
        self.statuses
    }

    /// New token carried by one assignment without copying it into the lease early.
    #[must_use]
    pub fn resume_token(&self, assignment_index: usize) -> Option<&str> {
        let assignment = self.assignments.get(assignment_index)?;
        self.tokens.get(assignment.token_index).map(String::as_str)
    }

    /// Old disconnected tokens requiring an explicit unavailable or expired
    /// reclaim result after this world replaces their former snake.
    #[must_use]
    pub const fn unavailable_reservations(&self) -> &[UnavailableControllerReservation] {
        self.unavailable_reservations
    }

    /// Latest work and retained-capacity evidence.
    #[must_use]
    pub const fn diagnostics(&self) -> ExternalReplacementDiagnostics {
        self.diagnostics
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeadLeaseTarget {
    lease_id: u64,
    snake_id: u64,
    connected: bool,
}

#[derive(Clone, Copy, Debug)]
struct ReplacementRecord {
    /// Final old-life accounting survives physical slot reuse until output admission.
    old_reward: Option<super::external_reward::ExternalRewardLedger>,
    assignment: ReplacementAssignment,
    snake_index: usize,
    disconnect: DisconnectProposal,
}

/// Result of one exact assignment result without exposing internal buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignmentResolution {
    /// The exact pending assignment accepted and rotated its token.
    Accepted,
    /// The exact pending assignment failed, invalidated the old token, and entered grace.
    Failed,
    /// The identity was unknown, stale, or had already resolved.
    Ignored,
}

/// Reusable owner of one complete replacement batch.
#[derive(Debug, Default)]
pub struct ExternalReplacementWorkspace {
    spawn: SpawnWorkspace,
    world: WorldState,
    rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    allocators: Option<AllocatorState>,
    brains: Vec<BrainRuntimeState>,
    lease_order: Vec<DeadLeaseTarget>,
    records: Vec<ReplacementRecord>,
    assignments: Vec<ReplacementAssignment>,
    statuses: Vec<AssignmentDeliveryStatus>,
    tokens: Vec<String>,
    unavailable_reservations: Vec<UnavailableControllerReservation>,
    token_count: usize,
    replacement_body: Vec<WorldPoint>,
    compacted_body: Vec<WorldPoint>,
    proof: Option<ExternalReplacementAuthorityProof>,
    key: Option<PhysicsStepKey>,
    diagnostics: ExternalReplacementDiagnostics,
    ready: bool,
}

impl ExternalReplacementWorkspace {
    /// Construct empty reusable replacement scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Invalidate the current staged batch while retaining reusable storage.
    pub(crate) fn discard(&mut self) {
        self.clear_staging();
    }

    /// Stage every dead controller lease from one complete post-physics boundary.
    ///
    /// A connected owner receives a fresh collision-safe snake, brain, lease
    /// epoch and not-yet-committed token. A dead disconnected or taken-over
    /// lease is removed because there is no valid connection to receive a
    /// replacement assignment. Source buffers never mutate.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'workspace, 'source>(
        &'workspace mut self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_rng: &'source RngStateBundle,
        source_allocators: &'source AllocatorState,
        source_brains: &'source [BrainRuntimeState],
        graph: &CompiledGraph,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
    ) -> Result<PreparedExternalReplacements<'workspace, 'source>, ExternalReplacementError> {
        self.prepare_with_entropy(
            key,
            source_world,
            source_rng,
            source_allocators,
            source_brains,
            graph,
            wall_now_ms,
            config,
            |bytes| getrandom::fill(bytes).map_err(|_| ()),
        )
    }

    /// Stage fresh snakes for every controller that remains connected across a
    /// durable generation boundary.
    ///
    /// `base_*` is the already-constructed next-generation world: evolved and
    /// baseline snakes, initial pellets, continued RNG streams, exact
    /// allocators, and successor population brains. `controller_source_world`
    /// is the still-authoritative preceding world whose connected leases must
    /// receive reliable fresh assignments. Disconnected reservations are not
    /// assigned a snake here; the bridge retains their old-token outcome as an
    /// explicit compatibility/tombstone concern.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_generation_reassignments<'workspace, 'source>(
        &'workspace mut self,
        key: PhysicsStepKey,
        base_world: &'source WorldState,
        base_rng: &'source RngStateBundle,
        base_allocators: &'source AllocatorState,
        base_brains: &'source [BrainRuntimeState],
        controller_source_world: &WorldState,
        graph: &CompiledGraph,
        successor_population_epoch: u64,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
    ) -> Result<PreparedExternalReplacements<'workspace, 'source>, ExternalReplacementError> {
        self.prepare_generation_reassignments_with_entropy(
            key,
            base_world,
            base_rng,
            base_allocators,
            base_brains,
            controller_source_world,
            graph,
            successor_population_epoch,
            wall_now_ms,
            config,
            |bytes| getrandom::fill(bytes).map_err(|_| ()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_generation_reassignments_with_entropy<'workspace, 'source, F>(
        &'workspace mut self,
        key: PhysicsStepKey,
        base_world: &'source WorldState,
        base_rng: &'source RngStateBundle,
        base_allocators: &'source AllocatorState,
        base_brains: &'source [BrainRuntimeState],
        controller_source_world: &WorldState,
        graph: &CompiledGraph,
        successor_population_epoch: u64,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
        mut fill_entropy: F,
    ) -> Result<PreparedExternalReplacements<'workspace, 'source>, ExternalReplacementError>
    where
        F: FnMut(&mut [u8]) -> Result<(), ()>,
    {
        self.clear_staging();
        config.validate()?;
        let expected_epoch = key.population_epoch().checked_add(1).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "successor population epoch",
            },
        )?;
        if successor_population_epoch != expected_epoch {
            return Err(ExternalReplacementError::SuccessorPopulationEpochMismatch {
                expected: expected_epoch,
                actual: successor_population_epoch,
            });
        }
        if !base_world.controller_leases.is_empty()
            || base_world
                .snakes
                .iter()
                .any(|snake| !matches!(snake.kind, SnakeKind::Evolved | SnakeKind::Baseline))
            || base_brains.iter().any(|brain| {
                !matches!(brain.owner, BrainOwner::PopulationSlot(_))
                    || brain.handle.epoch != successor_population_epoch
            })
        {
            return Err(ExternalReplacementError::InvalidGenerationBase {
                reason:
                    "base world contains controllers, non-generation snakes, or wrong-epoch brains",
            });
        }

        let (connected_count, omitted_count) =
            validate_generation_controller_source(controller_source_world)?;
        let required_snakes = base_world.snakes.len().checked_add(connected_count).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "generation controller placeholder count",
            },
        )?;
        if required_snakes > config.maximum_snakes {
            return Err(ExternalReplacementError::SnakeCapacityExceeded {
                required: required_snakes,
                maximum: config.maximum_snakes,
            });
        }
        let required_body_points = base_world
            .body_points
            .len()
            .checked_add(connected_count)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "generation controller placeholder bodies",
            })?;
        if required_body_points > config.maximum_body_points {
            return Err(ExternalReplacementError::BodyCapacityExceeded {
                required: required_body_points,
                maximum: config.maximum_body_points,
            });
        }

        copy_world_reusing(&mut self.world, base_world, base_world.pellets.len())
            .map_err(|error| ExternalReplacementError::WorldCopy(Box::new(error)))?;
        copy_rng_bundle_reusing(&mut self.rng, &mut self.rng_copy_scratch, base_rng)
            .map_err(|error| ExternalReplacementError::WorldCopy(Box::new(error)))?;
        self.allocators = Some(base_allocators.clone());
        copy_brains_reusing(&mut self.brains, base_brains, false)
            .map_err(|error| ExternalReplacementError::BrainCopy(Box::new(error)))?;
        reserve_for(
            &mut self.world.snakes,
            required_snakes,
            "generation controller placeholders",
        )?;
        reserve_for(
            &mut self.world.body_points,
            required_body_points,
            "generation controller placeholder bodies",
        )?;
        reserve_for(
            &mut self.world.controller_leases,
            connected_count,
            "generation controller leases",
        )?;
        reserve_for(
            &mut self.unavailable_reservations,
            omitted_count,
            "unavailable generation controller reservations",
        )?;

        for lease in &controller_source_world.controller_leases {
            if lease.status != ControllerLeaseStatus::Connected {
                self.unavailable_reservations
                    .push(unavailable_reservation(lease)?);
                continue;
            }
            let source_snake = find_unique_snake(controller_source_world, lease.snake_id)?;
            let body_start = self.world.body_points.len();
            self.world.body_points.push(source_snake.position);
            let mut placeholder = source_snake.clone();
            placeholder.alive = false;
            placeholder.brain = None;
            placeholder.body = BodyRange {
                start: body_start,
                len: 1,
            };
            self.world.snakes.push(placeholder);
            self.world.controller_leases.push(lease.clone());
        }
        self.diagnostics.removed_dead_leases = omitted_count;

        self.finish_preparation(
            key,
            base_world,
            base_rng,
            base_allocators,
            base_brains,
            graph,
            successor_population_epoch,
            wall_now_ms,
            config,
            &mut fill_entropy,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_with_entropy<'workspace, 'source, F>(
        &'workspace mut self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_rng: &'source RngStateBundle,
        source_allocators: &'source AllocatorState,
        source_brains: &'source [BrainRuntimeState],
        graph: &CompiledGraph,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
        mut fill_entropy: F,
    ) -> Result<PreparedExternalReplacements<'workspace, 'source>, ExternalReplacementError>
    where
        F: FnMut(&mut [u8]) -> Result<(), ()>,
    {
        self.clear_staging();
        config.validate()?;
        if source_world.snakes.len() > config.maximum_snakes {
            return Err(ExternalReplacementError::SnakeCapacityExceeded {
                required: source_world.snakes.len(),
                maximum: config.maximum_snakes,
            });
        }
        copy_world_reusing(&mut self.world, source_world, source_world.pellets.len())
            .map_err(|error| ExternalReplacementError::WorldCopy(Box::new(error)))?;
        copy_rng_bundle_reusing(&mut self.rng, &mut self.rng_copy_scratch, source_rng)
            .map_err(|error| ExternalReplacementError::WorldCopy(Box::new(error)))?;
        self.allocators = Some(source_allocators.clone());
        copy_brains_reusing(&mut self.brains, source_brains, false)
            .map_err(|error| ExternalReplacementError::BrainCopy(Box::new(error)))?;
        self.finish_preparation(
            key,
            source_world,
            source_rng,
            source_allocators,
            source_brains,
            graph,
            key.population_epoch(),
            wall_now_ms,
            config,
            &mut fill_entropy,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_preparation<'workspace, 'source, F>(
        &'workspace mut self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_rng: &'source RngStateBundle,
        source_allocators: &'source AllocatorState,
        source_brains: &'source [BrainRuntimeState],
        graph: &CompiledGraph,
        brain_epoch: u64,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
        fill_entropy: &mut F,
    ) -> Result<PreparedExternalReplacements<'workspace, 'source>, ExternalReplacementError>
    where
        F: FnMut(&mut [u8]) -> Result<(), ()>,
    {
        self.prepare_dead_lease_order()?;

        let replacement_count = self
            .lease_order
            .iter()
            .filter(|target| target.connected)
            .count();
        self.preflight_storage(replacement_count, config)?;
        self.remove_unconnected_dead_leases()?;

        let count_u64 = u64::try_from(replacement_count).map_err(|_| {
            ExternalReplacementError::ArithmeticOverflow {
                context: "external replacement count",
            }
        })?;
        let count_u32 = u32::try_from(replacement_count).map_err(|_| {
            ExternalReplacementError::ArithmeticOverflow {
                context: "replacement frame ID count",
            }
        })?;
        let reservations = if replacement_count == 0 {
            None
        } else {
            let allocators = self
                .allocators
                .as_mut()
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let external = allocators
                .reserve_external_ids(count_u64)
                .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let frames = allocators
                .reserve_frame_v1_ids(
                    count_u32,
                    source_world.snakes.iter().map(|snake| snake.frame_v1_id),
                )
                .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let brains = allocators
                .reserve_brain_ids(count_u64)
                .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let leases = allocators
                .reserve_controller_lease_ids(count_u64)
                .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            Some((external.first, frames.first, brains.first, leases.first))
        };

        let mut replacement_ordinal = 0usize;
        for target_index in 0..self.lease_order.len() {
            let target = self.lease_order[target_index];
            if !target.connected {
                continue;
            }
            let (first_external, first_frame, first_brain, first_lease) =
                reservations.ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            self.stage_one_replacement(
                key,
                target,
                replacement_ordinal,
                first_external,
                first_frame,
                first_brain,
                first_lease,
                graph,
                brain_epoch,
                wall_now_ms,
                config,
                fill_entropy,
            )?;
            replacement_ordinal += 1;
        }
        if replacement_ordinal != replacement_count {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }

        self.clear_replaced_pellet_owners();
        compact_world_bodies(
            &mut self.world,
            &mut self.compacted_body,
            config.maximum_body_points,
        )?;
        self.validate_ready_shape(source_world, source_rng, source_allocators, config, graph)?;
        self.key = Some(key);
        self.ready = true;
        self.diagnostics.replacements = self.assignments.len();
        self.diagnostics.lease_order_capacity = self.lease_order.capacity();
        self.diagnostics.assignment_capacity = self.assignments.capacity();
        self.diagnostics.status_capacity = self.statuses.capacity();
        self.diagnostics.token_capacity = self.tokens.capacity();
        self.diagnostics.unavailable_reservation_capacity =
            self.unavailable_reservations.capacity();
        self.diagnostics.replacement_body_capacity = self.replacement_body.capacity();
        self.diagnostics.compacted_body_capacity = self.compacted_body.capacity();
        self.diagnostics.snake_capacity = self.world.snakes.capacity();
        self.diagnostics.brain_capacity = self.brains.capacity();
        let rng = self
            .rng
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let allocators = self
            .allocators
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        self.proof = Some(ExternalReplacementAuthorityProof {
            key,
            replacements: self.diagnostics.replacements,
            removed_dead_leases: self.diagnostics.removed_dead_leases,
            world_sha256: authority_world_digest(&self.world),
            rng_sha256: authority_rng_digest(rng),
            allocators: allocators.clone(),
            brains_sha256: authority_brain_digest(&self.brains),
        });
        Ok(self.prepared(
            key,
            source_world,
            source_rng,
            source_allocators,
            source_brains,
        ))
    }

    /// Whether the latest attempt produced a complete replacement batch.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Resolve one exact assignment with first-result-wins semantics.
    pub fn resolve_assignment(
        &mut self,
        key: PhysicsStepKey,
        lease_id: u64,
        connection_id: u64,
        accepted: bool,
    ) -> Result<AssignmentResolution, ExternalReplacementError> {
        if !self.ready || self.key != Some(key) {
            return Err(ExternalReplacementError::ResultNotReady);
        }
        let Ok(index) = self
            .assignments
            .binary_search_by_key(&lease_id, |assignment| assignment.lease_id)
        else {
            return Ok(AssignmentResolution::Ignored);
        };
        let assignment = self.assignments[index];
        if assignment.connection_id != connection_id
            || self.statuses[index] != AssignmentDeliveryStatus::Pending
        {
            return Ok(AssignmentResolution::Ignored);
        }
        self.validate_record(index)?;
        let token = self
            .tokens
            .get(assignment.token_index)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let record = self.records[index];
        let lease_index = self
            .world
            .controller_leases
            .iter()
            .position(|lease| lease.id == assignment.lease_id)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let lease = &mut self.world.controller_leases[lease_index];
        // This lease owns a new snake even when delivery fails. Its predecessor's
        // token must never report a successful same-snake reclaim for this identity.
        // Capacity and token uniqueness were checked before exposing the batch.
        debug_assert!(lease.resume_token.capacity() >= token.len());
        lease.resume_token.clear();
        lease.resume_token.push_str(token);
        if accepted {
            self.statuses[index] = AssignmentDeliveryStatus::Accepted;
            Ok(AssignmentResolution::Accepted)
        } else {
            let snake = &mut self.world.snakes[record.snake_index];
            commit_disconnect_prevalidated(lease, snake, record.disconnect);
            self.statuses[index] = AssignmentDeliveryStatus::Failed;
            Ok(AssignmentResolution::Failed)
        }
    }

    /// Current retained diagnostics, including after a rejected attempt.
    #[must_use]
    pub fn diagnostics(&self) -> ExternalReplacementDiagnostics {
        if self.ready {
            self.diagnostics
        } else {
            ExternalReplacementDiagnostics {
                lease_order_capacity: self.lease_order.capacity(),
                assignment_capacity: self.assignments.capacity(),
                status_capacity: self.statuses.capacity(),
                token_capacity: self.tokens.capacity(),
                unavailable_reservation_capacity: self.unavailable_reservations.capacity(),
                replacement_body_capacity: self.replacement_body.capacity(),
                compacted_body_capacity: self.compacted_body.capacity(),
                snake_capacity: self.world.snakes.capacity(),
                brain_capacity: self.brains.capacity(),
                ..ExternalReplacementDiagnostics::default()
            }
        }
    }

    /// Read the completely staged world after every delivery has resolved.
    pub fn world(&self, key: PhysicsStepKey) -> Result<&WorldState, ExternalReplacementError> {
        self.ensure_resolved(key)?;
        Ok(&self.world)
    }

    /// Read the completely staged RNG bundle after every delivery has resolved.
    pub fn rng(&self, key: PhysicsStepKey) -> Result<&RngStateBundle, ExternalReplacementError> {
        self.ensure_resolved(key)?;
        self.rng
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)
    }

    /// Read the allocator continuation after every delivery has resolved.
    pub fn allocators(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&AllocatorState, ExternalReplacementError> {
        self.ensure_resolved(key)?;
        self.allocators
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)
    }

    /// Read the completely staged brain records after every delivery has resolved.
    pub fn brains(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[BrainRuntimeState], ExternalReplacementError> {
        self.ensure_resolved(key)?;
        Ok(&self.brains)
    }

    /// Read the canonical assignment batch after preparation or partial resolution.
    pub fn assignments(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[ReplacementAssignment], ExternalReplacementError> {
        self.ensure_ready(key)?;
        Ok(&self.assignments)
    }

    /// Final old-life ledger for the same ordered replacement assignment.
    pub fn old_reward(
        &self,
        key: PhysicsStepKey,
        index: usize,
    ) -> Result<Option<super::external_reward::ExternalRewardLedger>, ExternalReplacementError>
    {
        self.ensure_ready(key)?;
        self.records
            .get(index)
            .map(|record| record.old_reward)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)
    }

    /// Read old disconnected tokens whose prior snakes do not exist in the
    /// replacement world.
    pub fn unavailable_reservations(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[UnavailableControllerReservation], ExternalReplacementError> {
        self.ensure_ready(key)?;
        Ok(&self.unavailable_reservations)
    }

    /// Read one retained replacement token after preparation.
    pub fn resume_token(
        &self,
        key: PhysicsStepKey,
        assignment_index: usize,
    ) -> Result<&str, ExternalReplacementError> {
        self.ensure_ready(key)?;
        let assignment = self
            .assignments
            .get(assignment_index)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        self.tokens
            .get(assignment.token_index)
            .map(String::as_str)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)
    }

    /// Read current first-result-wins assignment states.
    pub fn statuses(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[AssignmentDeliveryStatus], ExternalReplacementError> {
        self.ensure_ready(key)?;
        Ok(&self.statuses)
    }

    pub(crate) fn staged_world(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&WorldState, ExternalReplacementError> {
        self.ensure_ready(key)?;
        Ok(&self.world)
    }

    pub(crate) fn staged_rng(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&RngStateBundle, ExternalReplacementError> {
        self.ensure_ready(key)?;
        self.rng
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)
    }

    pub(crate) fn staged_allocators(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&AllocatorState, ExternalReplacementError> {
        self.ensure_ready(key)?;
        self.allocators
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)
    }

    pub(crate) fn staged_brains(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[BrainRuntimeState], ExternalReplacementError> {
        self.ensure_ready(key)?;
        Ok(&self.brains)
    }

    /// Mutable provisional buffers used only for reversible full-state admission.
    ///
    /// This does not make the result publishable. The complete world-step owner
    /// must still resolve every assignment and request [`Self::publication_buffers`].
    pub(crate) fn validation_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<ExternalReplacementBuffers<'_>, ExternalReplacementError> {
        self.ensure_ready(key)?;
        self.buffers()
    }

    /// Mutable buffers after every exact local assignment result resolved.
    pub(crate) fn publication_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<ExternalReplacementBuffers<'_>, ExternalReplacementError> {
        self.ensure_resolved(key)?;
        self.validate_resolved_records()?;
        self.buffers()
    }

    fn prepare_dead_lease_order(&mut self) -> Result<(), ExternalReplacementError> {
        reserve_for(
            &mut self.lease_order,
            self.world.controller_leases.len(),
            "dead controller lease order",
        )?;
        for lease in &self.world.controller_leases {
            let snake = find_unique_snake(&self.world, lease.snake_id)?;
            if snake.kind != SnakeKind::External {
                return Err(ExternalReplacementError::InvalidLeaseTarget {
                    lease_id: lease.id,
                    snake_id: lease.snake_id,
                });
            }
            if !snake.alive {
                let connected = match (lease.status, lease.connection_id) {
                    (ControllerLeaseStatus::Connected, Some(connection_id))
                        if connection_id != 0 =>
                    {
                        true
                    }
                    (ControllerLeaseStatus::HoldingLastInput, None)
                    | (ControllerLeaseStatus::ReservedNeutral, None)
                    | (ControllerLeaseStatus::NeuralTakeover, None) => false,
                    _ => {
                        return Err(ExternalReplacementError::InvalidLeaseTarget {
                            lease_id: lease.id,
                            snake_id: lease.snake_id,
                        });
                    }
                };
                self.lease_order.push(DeadLeaseTarget {
                    lease_id: lease.id,
                    snake_id: lease.snake_id,
                    connected,
                });
            }
        }
        self.lease_order
            .sort_unstable_by_key(|target| target.lease_id);
        if let Some(pair) = self
            .lease_order
            .windows(2)
            .find(|pair| pair[0].lease_id == pair[1].lease_id)
        {
            return Err(ExternalReplacementError::DuplicateLeaseId(pair[0].lease_id));
        }
        Ok(())
    }

    fn preflight_storage(
        &mut self,
        replacement_count: usize,
        config: ExternalReplacementConfig,
    ) -> Result<(), ExternalReplacementError> {
        let referenced_body_points =
            self.world.snakes.iter().try_fold(0usize, |total, snake| {
                let end = snake.body.start.checked_add(snake.body.len).ok_or(
                    ExternalReplacementError::ArithmeticOverflow {
                        context: "source body range",
                    },
                )?;
                if end > self.world.body_points.len()
                    || (snake.body.len != 0
                        && self.world.body_points[snake.body.start] != snake.position)
                {
                    return Err(ExternalReplacementError::InternalShapeMismatch);
                }
                total.checked_add(snake.body.len).ok_or(
                    ExternalReplacementError::ArithmeticOverflow {
                        context: "referenced source body points",
                    },
                )
            })?;
        let removed_body_points = self
            .lease_order
            .iter()
            .filter(|target| target.connected)
            .try_fold(0usize, |total, target| {
                let snake = find_unique_snake(&self.world, target.snake_id)?;
                total.checked_add(snake.body.len).ok_or(
                    ExternalReplacementError::ArithmeticOverflow {
                        context: "replaced source body points",
                    },
                )
            })?;
        let added_body_points = replacement_count
            .checked_mul(config.spawn.snake_start_len)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "replacement body points",
            })?;
        let final_body_points = referenced_body_points
            .checked_sub(removed_body_points)
            .and_then(|count| count.checked_add(added_body_points))
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "final replacement body points",
            })?;
        if final_body_points > config.maximum_body_points {
            return Err(ExternalReplacementError::BodyCapacityExceeded {
                required: final_body_points,
                maximum: config.maximum_body_points,
            });
        }
        reserve_for(
            &mut self.compacted_body,
            final_body_points,
            "replacement compacted body",
        )?;
        let temporary_body_points = self
            .world
            .body_points
            .len()
            .checked_add(added_body_points)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "temporary replacement body points",
            })?;
        reserve_for(
            &mut self.world.body_points,
            temporary_body_points,
            "temporary replacement body",
        )?;
        reserve_for(
            &mut self.replacement_body,
            config.spawn.snake_start_len,
            "one replacement body",
        )?;
        reserve_for(&mut self.records, replacement_count, "replacement records")?;
        reserve_for(
            &mut self.assignments,
            replacement_count,
            "replacement assignments",
        )?;
        reserve_for(
            &mut self.statuses,
            replacement_count,
            "replacement statuses",
        )?;
        reserve_for(
            &mut self.tokens,
            replacement_count,
            "replacement token records",
        )?;
        while self.tokens.len() < replacement_count {
            self.tokens.push(String::new());
        }
        self.token_count = replacement_count;
        for token in &mut self.tokens[..replacement_count] {
            reserve_string(token, RESUME_TOKEN_LENGTH, "replacement resume token")?;
            token.clear();
        }

        let missing_old_brains = self
            .lease_order
            .iter()
            .filter(|target| target.connected)
            .try_fold(0usize, |count, target| {
                let snake = find_unique_snake(&self.world, target.snake_id)?;
                if let Some(handle) = snake.brain {
                    let found = self.brains.iter().any(|brain| {
                        brain.handle == handle && brain.owner == BrainOwner::Entity(snake.id)
                    });
                    if !found {
                        return Err(ExternalReplacementError::InvalidBrainOwner {
                            snake_id: snake.id,
                        });
                    }
                    Ok(count)
                } else {
                    count
                        .checked_add(1)
                        .ok_or(ExternalReplacementError::ArithmeticOverflow {
                            context: "replacement brain count",
                        })
                }
            })?;
        let final_brains = self.brains.len().checked_add(missing_old_brains).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "final brain count",
            },
        )?;
        if final_brains > config.maximum_brains {
            return Err(ExternalReplacementError::BrainCapacityExceeded {
                required: final_brains,
                maximum: config.maximum_brains,
            });
        }
        reserve_for(&mut self.brains, final_brains, "replacement brain records")?;
        Ok(())
    }

    fn remove_unconnected_dead_leases(&mut self) -> Result<(), ExternalReplacementError> {
        let unavailable_count = self
            .lease_order
            .iter()
            .filter(|target| !target.connected)
            .count();
        let final_unavailable_count = self
            .unavailable_reservations
            .len()
            .checked_add(unavailable_count)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "unavailable controller reservation count",
            })?;
        reserve_for(
            &mut self.unavailable_reservations,
            final_unavailable_count,
            "unavailable controller reservations",
        )?;
        for target in self
            .lease_order
            .iter()
            .copied()
            .filter(|target| !target.connected)
        {
            let lease = self
                .world
                .controller_leases
                .iter()
                .find(|lease| lease.id == target.lease_id)
                .ok_or(ExternalReplacementError::InvalidLeaseTarget {
                    lease_id: target.lease_id,
                    snake_id: target.snake_id,
                })?;
            self.unavailable_reservations
                .push(unavailable_reservation(lease)?);
        }
        let before = self.world.controller_leases.len();
        self.world.controller_leases.retain(|lease| {
            self.lease_order
                .binary_search_by_key(&lease.id, |target| target.lease_id)
                .ok()
                .is_none_or(|index| self.lease_order[index].connected)
        });
        self.diagnostics.removed_dead_leases = self
            .diagnostics
            .removed_dead_leases
            .saturating_add(before - self.world.controller_leases.len());
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_one_replacement<F>(
        &mut self,
        key: PhysicsStepKey,
        target: DeadLeaseTarget,
        ordinal: usize,
        first_external_id: u64,
        first_frame_id: u32,
        first_brain_id: u64,
        first_lease_id: u64,
        graph: &CompiledGraph,
        brain_epoch: u64,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
        fill_entropy: &mut F,
    ) -> Result<(), ExternalReplacementError>
    where
        F: FnMut(&mut [u8]) -> Result<(), ()>,
    {
        let ordinal_u64 =
            u64::try_from(ordinal).map_err(|_| ExternalReplacementError::ArithmeticOverflow {
                context: "replacement identity offset",
            })?;
        let ordinal_u32 =
            u32::try_from(ordinal).map_err(|_| ExternalReplacementError::ArithmeticOverflow {
                context: "replacement frame identity offset",
            })?;
        let snake_id = first_external_id.checked_add(ordinal_u64).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "replacement snake identity",
            },
        )?;
        let frame_v1_id = first_frame_id.checked_add(ordinal_u32).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "replacement frame identity",
            },
        )?;
        let brain_handle = BrainHandle {
            id: first_brain_id.checked_add(ordinal_u64).ok_or(
                ExternalReplacementError::ArithmeticOverflow {
                    context: "replacement brain identity",
                },
            )?,
            epoch: brain_epoch,
        };
        let lease_id = first_lease_id.checked_add(ordinal_u64).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "replacement lease identity",
            },
        )?;

        self.generate_unique_token(ordinal, fill_entropy)?;
        let old_snake_index = find_unique_snake_index(&self.world, target.snake_id)?;
        let old_snake = self.world.snakes[old_snake_index].clone();
        if old_snake.alive || old_snake.kind != SnakeKind::External {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: target.lease_id,
                snake_id: target.snake_id,
            });
        }
        let lease_index = self
            .world
            .controller_leases
            .iter()
            .position(|lease| lease.id == target.lease_id)
            .ok_or(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: target.lease_id,
                snake_id: target.snake_id,
            })?;
        let old_lease = self.world.controller_leases[lease_index].clone();
        let connection_id =
            old_lease
                .connection_id
                .ok_or(ExternalReplacementError::InvalidLeaseTarget {
                    lease_id: target.lease_id,
                    snake_id: target.snake_id,
                })?;
        if old_lease.status != ControllerLeaseStatus::Connected
            || old_lease.snake_id != old_snake.id
        {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: target.lease_id,
                snake_id: target.snake_id,
            });
        }

        let source_external_rng = &self
            .rng
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?
            .external_controller;
        let initialized = initialize_random_genome(graph, source_external_rng, config.genome)
            .map_err(|error| ExternalReplacementError::Genome(Box::new(error)))?;
        let (weights, post_genome_rng) = initialized.into_parts();
        let request = [SpawnRequest {
            key: SpawnKey {
                domain: SpawnDomain::External,
                slot: snake_id,
            },
        }];
        let remaining_candidates = config
            .spawn
            .maximum_candidates_per_batch
            .checked_sub(self.diagnostics.candidates_examined)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let remaining_geometry = config
            .spawn
            .maximum_geometry_checks_per_batch
            .checked_sub(self.diagnostics.geometry_checks)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        if remaining_candidates == 0 || remaining_geometry == 0 {
            return Err(ExternalReplacementError::Spawn(Box::new(
                SpawnError::WorkBudgetExceeded {
                    key: request[0].key,
                    work: if remaining_candidates == 0 {
                        "external replacement candidates"
                    } else {
                        "external replacement geometry checks"
                    },
                    required: 1,
                    maximum: 0,
                },
            )));
        }
        let mut spawn_config = config.spawn;
        spawn_config.maximum_candidates_per_batch = remaining_candidates;
        spawn_config.maximum_geometry_checks_per_batch = remaining_geometry;
        let prepared = self
            .spawn
            .prepare(
                &self.world,
                &request,
                &post_genome_rng,
                spawn_config,
                config.spawn.snake_start_len,
            )
            .map_err(|error| ExternalReplacementError::Spawn(Box::new(error)))?;
        let placement = *prepared
            .placements()
            .first()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        if prepared.placements().len() != 1 || placement.key != request[0].key {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        let body = prepared
            .body_for(&placement)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        if body.len() != config.spawn.snake_start_len {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        self.replacement_body.clear();
        self.replacement_body.extend_from_slice(body);
        let spawn_diagnostics = prepared.diagnostics();
        self.diagnostics.candidates_examined = self
            .diagnostics
            .candidates_examined
            .checked_add(spawn_diagnostics.candidates_examined)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "replacement spawn candidate count",
            })?;
        self.diagnostics.fallback_placements = self
            .diagnostics
            .fallback_placements
            .checked_add(spawn_diagnostics.fallback_placements)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "replacement fallback placement count",
            })?;
        self.diagnostics.geometry_checks = self
            .diagnostics
            .geometry_checks
            .checked_add(spawn_diagnostics.geometry_checks)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "replacement geometry-check count",
            })?;
        self.diagnostics.spawn = spawn_diagnostics;
        if self.diagnostics.candidates_examined > config.spawn.maximum_candidates_per_batch
            || self.diagnostics.geometry_checks > config.spawn.maximum_geometry_checks_per_batch
        {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        {
            let rng = self
                .rng
                .as_mut()
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            copy_serialized_rng_reusing(
                &mut rng.external_controller,
                prepared.next_rng(),
                &mut self.rng_copy_scratch.external_gaussian_spare,
            )
            .map_err(|error| ExternalReplacementError::WorldCopy(Box::new(error)))?;
        }
        let body_start = self.world.body_points.len();
        self.world
            .body_points
            .extend_from_slice(&self.replacement_body);
        let new_snake = initial_external_snake(
            snake_id,
            frame_v1_id,
            brain_handle,
            &placement,
            BodyRange {
                start: body_start,
                len: self.replacement_body.len(),
            },
            config,
        );
        self.world.snakes[old_snake_index] = new_snake;

        let new_brain = BrainRuntimeState {
            handle: brain_handle,
            owner: BrainOwner::Entity(snake_id),
            retained_origin: None,
            non_population_weights: Some(weights.into_boxed_slice()),
            recurrent: try_zero_f32_box(graph.total_state_size, "replacement recurrent state")?,
        };
        if let Some(old_handle) = old_snake.brain {
            let brain_index = self
                .brains
                .iter()
                .position(|brain| {
                    brain.handle == old_handle && brain.owner == BrainOwner::Entity(old_snake.id)
                })
                .ok_or(ExternalReplacementError::InvalidBrainOwner {
                    snake_id: old_snake.id,
                })?;
            self.brains[brain_index] = new_brain;
        } else {
            self.brains.push(new_brain);
        }

        let completed_step = key.source_completed_step().checked_add(1).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "replacement client tick",
            },
        )?;
        let replacement_lease = &mut self.world.controller_leases[lease_index];
        replacement_lease.id = lease_id;
        replacement_lease.snake_id = snake_id;
        replacement_lease.reward = old_lease
            .reward
            .map(|_| {
                super::external_reward::ExternalRewardLedger::new(
                    super::external_reward::RewardLifeIdentity {
                        world_epoch: key.world_epoch(),
                        lease_id,
                        physical_snake_id: snake_id,
                        frame_snake_id: frame_v1_id,
                    },
                    super::external_reward::RewardBoundary {
                        completed_step,
                        elapsed_seconds: completed_step as f64 * config.fixed_step_seconds,
                    },
                    self.world.snakes[old_snake_index].body.len,
                    self.world.snakes[old_snake_index].points,
                )
            })
            .transpose()
            .map_err(|_| ExternalReplacementError::InternalShapeMismatch)?;
        replacement_lease.latest_action = LatestControllerAction {
            is_explicit: false,
            turn: 0.0,
            boost: false,
            client_tick: completed_step,
            arrival_sequence: old_lease.latest_action.arrival_sequence,
            accepted_at_ms: wall_now_ms,
        };
        replacement_lease.last_observed_at_ms = wall_now_ms;
        replacement_lease.disconnected_at_ms = None;
        replacement_lease.input_hold_expires_at_ms = None;
        replacement_lease.grace_expires_at_ms = None;
        replacement_lease.takeover_committed_at_ms = None;
        replacement_lease.status = ControllerLeaseStatus::Connected;
        reserve_string(
            &mut replacement_lease.resume_token,
            RESUME_TOKEN_LENGTH,
            "retained replacement token",
        )?;

        let disconnect = prepare_disconnect(
            replacement_lease,
            &self.world.snakes[old_snake_index],
            connection_id,
            wall_now_ms,
            config.controller_timing,
        )
        .map_err(|error| ExternalReplacementError::Controller(Box::new(error)))?;
        validate_disconnect_proposal(
            replacement_lease,
            &self.world.snakes[old_snake_index],
            disconnect,
        )
        .map_err(|error| ExternalReplacementError::Controller(Box::new(error)))?;
        let assignment = ReplacementAssignment {
            source_lease_id: target.lease_id,
            lease_id,
            connection_id,
            controller_kind: old_lease.kind,
            snake_id,
            frame_v1_id,
            token_index: ordinal,
        };
        self.records.push(ReplacementRecord {
            old_reward: old_lease.reward,
            assignment,
            snake_index: old_snake_index,
            disconnect,
        });
        self.assignments.push(assignment);
        self.statuses.push(AssignmentDeliveryStatus::Pending);
        Ok(())
    }

    fn generate_unique_token<F>(
        &mut self,
        token_index: usize,
        fill_entropy: &mut F,
    ) -> Result<(), ExternalReplacementError>
    where
        F: FnMut(&mut [u8]) -> Result<(), ()>,
    {
        let mut bytes = [0u8; RESUME_TOKEN_BYTES];
        for _ in 0..TOKEN_ATTEMPTS {
            fill_entropy(&mut bytes).map_err(|_| ExternalReplacementError::EntropyUnavailable)?;
            let (prior_tokens, current_and_later) = self.tokens.split_at_mut(token_index);
            let token = current_and_later
                .first_mut()
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            encode_base64url_24(&bytes, token)?;
            let collides_live = self
                .world
                .controller_leases
                .iter()
                .any(|lease| lease.resume_token == *token);
            let collides_unavailable = self
                .unavailable_reservations
                .iter()
                .any(|reservation| reservation.resume_token == *token);
            let collides_batch = prior_tokens.iter().any(|prior| prior == token);
            if !collides_live && !collides_unavailable && !collides_batch {
                return Ok(());
            }
        }
        Err(ExternalReplacementError::TokenCollision)
    }

    fn clear_replaced_pellet_owners(&mut self) {
        for pellet in &mut self.world.pellets {
            if pellet.owner.is_some_and(|owner| {
                self.lease_order
                    .iter()
                    .any(|target| target.connected && target.snake_id == owner)
            }) {
                pellet.owner = None;
                self.diagnostics.cleared_pellet_owners += 1;
            }
        }
    }

    fn validate_ready_shape(
        &self,
        source_world: &WorldState,
        source_rng: &RngStateBundle,
        source_allocators: &AllocatorState,
        config: ExternalReplacementConfig,
        graph: &CompiledGraph,
    ) -> Result<(), ExternalReplacementError> {
        if self.records.len() != self.assignments.len()
            || self.records.len() != self.statuses.len()
            || self.records.len() != self.token_count
            || self.world.snakes.len() > config.maximum_snakes
            || self.world.body_points.len() > config.maximum_body_points
            || self.brains.len() > config.maximum_brains
        {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        let rng = self
            .rng
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        if rng.version != source_rng.version
            || rng.world != source_rng.world
            || rng.evolution != source_rng.evolution
            || rng.baselines != source_rng.baselines
            || rng.evolved_respawns != source_rng.evolved_respawns
            || rng.baseline_behaviors != source_rng.baseline_behaviors
        {
            return Err(ExternalReplacementError::RngIsolationViolation);
        }
        let allocators = self
            .allocators
            .as_ref()
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let replacement_count = u64::try_from(self.assignments.len()).map_err(|_| {
            ExternalReplacementError::ArithmeticOverflow {
                context: "validated replacement count",
            }
        })?;
        let frame_count = u32::try_from(self.assignments.len()).map_err(|_| {
            ExternalReplacementError::ArithmeticOverflow {
                context: "validated frame replacement count",
            }
        })?;
        let mut expected_allocators = source_allocators.clone();
        expected_allocators
            .reserve_external_ids(replacement_count)
            .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?;
        expected_allocators
            .reserve_frame_v1_ids(
                frame_count,
                source_world.snakes.iter().map(|snake| snake.frame_v1_id),
            )
            .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?;
        expected_allocators
            .reserve_brain_ids(replacement_count)
            .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?;
        expected_allocators
            .reserve_controller_lease_ids(replacement_count)
            .map_err(|error| ExternalReplacementError::Allocator(Box::new(error)))?;
        if *allocators != expected_allocators {
            return Err(ExternalReplacementError::AllocatorIsolationViolation);
        }
        if self.assignments.is_empty()
            && (rng.external_controller != source_rng.external_controller
                || allocators != source_allocators)
        {
            return Err(ExternalReplacementError::RngIsolationViolation);
        }
        if self.assignments.windows(2).any(|pair| {
            pair[0].source_lease_id >= pair[1].source_lease_id
                || pair[0].lease_id >= pair[1].lease_id
        }) {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        for index in 0..self.records.len() {
            self.validate_record(index)?;
            let assignment = self.assignments[index];
            let brain = self
                .brains
                .iter()
                .find(|brain| brain.owner == BrainOwner::Entity(assignment.snake_id))
                .ok_or(ExternalReplacementError::InvalidBrainOwner {
                    snake_id: assignment.snake_id,
                })?;
            if brain
                .non_population_weights
                .as_ref()
                .map(|weights| weights.len())
                != Some(graph.total_parameters)
                || brain.recurrent.len() != graph.total_state_size
                || brain.recurrent.iter().any(|value| value.to_bits() != 0)
            {
                return Err(ExternalReplacementError::InvalidBrainOwner {
                    snake_id: assignment.snake_id,
                });
            }
        }
        Ok(())
    }

    fn ensure_ready(&self, key: PhysicsStepKey) -> Result<(), ExternalReplacementError> {
        if !self.ready || self.key != Some(key) {
            return Err(ExternalReplacementError::ResultNotReady);
        }
        Ok(())
    }

    fn ensure_resolved(&self, key: PhysicsStepKey) -> Result<(), ExternalReplacementError> {
        self.ensure_ready(key)?;
        if self.statuses.contains(&AssignmentDeliveryStatus::Pending) {
            return Err(ExternalReplacementError::AssignmentsPending);
        }
        Ok(())
    }

    fn validate_resolved_records(&self) -> Result<(), ExternalReplacementError> {
        for (index, status) in self.statuses.iter().copied().enumerate() {
            let record = *self
                .records
                .get(index)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let assignment = *self
                .assignments
                .get(index)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let snake = self
                .world
                .snakes
                .get(record.snake_index)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let lease = self
                .world
                .controller_leases
                .iter()
                .find(|lease| lease.id == assignment.lease_id)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            if record.assignment != assignment
                || !snake.alive
                || snake.id != assignment.snake_id
                || snake.frame_v1_id != assignment.frame_v1_id
                || snake.kind != SnakeKind::External
                || lease.snake_id != snake.id
                || lease.kind != assignment.controller_kind
            {
                return Err(ExternalReplacementError::InternalShapeMismatch);
            }
            match status {
                AssignmentDeliveryStatus::Pending => {
                    return Err(ExternalReplacementError::AssignmentsPending);
                }
                AssignmentDeliveryStatus::Accepted => {
                    let token = self
                        .tokens
                        .get(assignment.token_index)
                        .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
                    if lease.connection_id != Some(assignment.connection_id)
                        || lease.status != ControllerLeaseStatus::Connected
                        || lease.resume_token.as_str() != token.as_str()
                    {
                        return Err(ExternalReplacementError::InternalShapeMismatch);
                    }
                }
                AssignmentDeliveryStatus::Failed => {
                    let token = self
                        .tokens
                        .get(assignment.token_index)
                        .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
                    if lease.connection_id.is_some()
                        || lease.resume_token.as_str() != token.as_str()
                        || !matches!(
                            lease.status,
                            ControllerLeaseStatus::HoldingLastInput
                                | ControllerLeaseStatus::ReservedNeutral
                        )
                        || lease.disconnected_at_ms.is_none()
                        || lease.input_hold_expires_at_ms.is_none()
                        || lease.grace_expires_at_ms.is_none()
                        || lease.takeover_committed_at_ms.is_some()
                    {
                        return Err(ExternalReplacementError::InternalShapeMismatch);
                    }
                }
            }
        }
        Ok(())
    }

    fn buffers(&mut self) -> Result<ExternalReplacementBuffers<'_>, ExternalReplacementError> {
        let Self {
            world,
            rng,
            allocators,
            brains,
            proof,
            ..
        } = self;
        Ok(ExternalReplacementBuffers {
            world,
            rng: rng
                .as_mut()
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?,
            allocators: allocators
                .as_mut()
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?,
            brains,
            proof: proof
                .as_ref()
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?,
        })
    }

    fn validate_record(&self, index: usize) -> Result<(), ExternalReplacementError> {
        let record = *self
            .records
            .get(index)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let assignment = *self
            .assignments
            .get(index)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        if record.assignment != assignment {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        let snake = self
            .world
            .snakes
            .get(record.snake_index)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let lease = self
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.id == assignment.lease_id)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        if !snake.alive
            || snake.id != assignment.snake_id
            || snake.frame_v1_id != assignment.frame_v1_id
            || snake.kind != SnakeKind::External
            || lease.snake_id != snake.id
            || lease.kind != assignment.controller_kind
            || lease.connection_id != Some(assignment.connection_id)
            || lease.status != ControllerLeaseStatus::Connected
        {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        validate_disconnect_proposal(lease, snake, record.disconnect)
            .map_err(|error| ExternalReplacementError::Controller(Box::new(error)))
    }

    fn prepared<'workspace, 'source>(
        &'workspace self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_rng: &'source RngStateBundle,
        source_allocators: &'source AllocatorState,
        source_brains: &'source [BrainRuntimeState],
    ) -> PreparedExternalReplacements<'workspace, 'source> {
        PreparedExternalReplacements {
            key,
            source_world,
            source_rng,
            source_allocators,
            source_brains,
            assignments: &self.assignments,
            statuses: &self.statuses,
            tokens: &self.tokens[..self.token_count],
            unavailable_reservations: &self.unavailable_reservations,
            diagnostics: self.diagnostics,
        }
    }

    fn clear_staging(&mut self) {
        self.lease_order.clear();
        self.records.clear();
        self.assignments.clear();
        self.statuses.clear();
        self.token_count = 0;
        self.unavailable_reservations.clear();
        self.replacement_body.clear();
        self.proof = None;
        self.key = None;
        self.diagnostics = ExternalReplacementDiagnostics::default();
        self.ready = false;
    }
}

fn compact_world_bodies(
    world: &mut WorldState,
    scratch: &mut Vec<WorldPoint>,
    maximum_body_points: usize,
) -> Result<(), ExternalReplacementError> {
    let required = world.snakes.iter().try_fold(0usize, |total, snake| {
        let end = snake.body.start.checked_add(snake.body.len).ok_or(
            ExternalReplacementError::ArithmeticOverflow {
                context: "body range during replacement compaction",
            },
        )?;
        if end > world.body_points.len()
            || (snake.alive && snake.body.len == 0)
            || (snake.body.len != 0 && world.body_points[snake.body.start] != snake.position)
        {
            return Err(ExternalReplacementError::InternalShapeMismatch);
        }
        total
            .checked_add(snake.body.len)
            .ok_or(ExternalReplacementError::ArithmeticOverflow {
                context: "replacement body compaction count",
            })
    })?;
    if required > maximum_body_points {
        return Err(ExternalReplacementError::BodyCapacityExceeded {
            required,
            maximum: maximum_body_points,
        });
    }
    scratch.clear();
    reserve_for(scratch, required, "replacement body compaction")?;
    for index in 0..world.snakes.len() {
        let range = world.snakes[index].body;
        let end = range
            .start
            .checked_add(range.len)
            .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
        let start = scratch.len();
        scratch.extend_from_slice(
            world
                .body_points
                .get(range.start..end)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?,
        );
        world.snakes[index].body = BodyRange {
            start,
            len: range.len,
        };
    }
    std::mem::swap(&mut world.body_points, scratch);
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), ExternalReplacementError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| ExternalReplacementError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

fn reserve_string(
    value: &mut String,
    required: usize,
    buffer: &'static str,
) -> Result<(), ExternalReplacementError> {
    if value.capacity() < required {
        value
            .try_reserve_exact(required.saturating_sub(value.len()))
            .map_err(|_| ExternalReplacementError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

fn try_zero_f32_box(
    required: usize,
    buffer: &'static str,
) -> Result<Box<[f32]>, ExternalReplacementError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(required)
        .map_err(|_| ExternalReplacementError::AllocationFailed { buffer, required })?;
    values.resize(required, 0.0);
    Ok(values.into_boxed_slice())
}

/// Rejected replacement attempt; no prepared result may be observed.
#[derive(Debug)]
pub enum ExternalReplacementError {
    /// Projected settings or ceilings are invalid.
    InvalidConfig,
    /// The collision-safe generation world is not the empty-controller base
    /// expected by this reassignment path.
    InvalidGenerationBase { reason: &'static str },
    /// The requested brain epoch is not the exact next population epoch.
    SuccessorPopulationEpochMismatch { expected: u64, actual: u64 },
    /// The compiled graph cannot own a useful external brain.
    InvalidGraph,
    /// Checked arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// A checked scratch reservation failed.
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    /// Source world/RNG copy failed.
    WorldCopy(Box<FixedStepPrefixError>),
    /// Source brain copy failed.
    BrainCopy(Box<ControlPhaseError>),
    /// Genome initialization failed.
    Genome(Box<GenomeInitializationError>),
    /// Collision-safe placement failed.
    Spawn(Box<SpawnError>),
    /// Deterministic allocator reservation failed.
    Allocator(Box<StateError>),
    /// Controller deadline or snapshot validation failed.
    Controller(Box<ControllerError>),
    /// Operating-system entropy was unavailable.
    EntropyUnavailable,
    /// Bounded token generation repeatedly collided with live tokens.
    TokenCollision,
    /// The source contains no named snake.
    UnknownSnake(u64),
    /// The source repeats a snake identity.
    DuplicateSnakeId(u64),
    /// The source repeats a lease identity.
    DuplicateLeaseId(u64),
    /// A dead lease does not identify a compatible external snake/socket state.
    InvalidLeaseTarget { lease_id: u64, snake_id: u64 },
    /// A source external brain does not match its snake.
    InvalidBrainOwner { snake_id: u64 },
    /// The admitted snake ceiling is too small.
    SnakeCapacityExceeded { required: usize, maximum: usize },
    /// The admitted body ceiling is too small.
    BodyCapacityExceeded { required: usize, maximum: usize },
    /// The admitted brain ceiling is too small.
    BrainCapacityExceeded { required: usize, maximum: usize },
    /// A non-external RNG stream changed.
    RngIsolationViolation,
    /// An unrelated allocator domain changed.
    AllocatorIsolationViolation,
    /// The latest attempt has no complete result.
    ResultNotReady,
    /// One or more reliable assignment sends have not reported a result.
    AssignmentsPending,
    /// Internal prepared ranges/identities disagree.
    InternalShapeMismatch,
}

impl Display for ExternalReplacementError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig => write!(formatter, "invalid external replacement configuration"),
            Self::InvalidGenerationBase { reason } => {
                write!(formatter, "invalid generation reassignment base: {reason}")
            }
            Self::SuccessorPopulationEpochMismatch { expected, actual } => write!(
                formatter,
                "successor population epoch is {actual}; expected {expected}"
            ),
            Self::InvalidGraph => write!(formatter, "external replacement graph has no parameters"),
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "external replacement arithmetic overflow in {context}"
                )
            }
            Self::AllocationFailed { buffer, required } => write!(
                formatter,
                "unable to reserve {required} values for {buffer}"
            ),
            Self::WorldCopy(error) => write!(formatter, "replacement world copy failed: {error}"),
            Self::BrainCopy(error) => write!(formatter, "replacement brain copy failed: {error}"),
            Self::Genome(error) => write!(formatter, "replacement genome failed: {error}"),
            Self::Spawn(error) => write!(formatter, "replacement spawn failed: {error}"),
            Self::Allocator(error) => write!(formatter, "replacement allocator failed: {error}"),
            Self::Controller(error) => write!(formatter, "replacement controller failed: {error}"),
            Self::EntropyUnavailable => write!(formatter, "resume-token entropy was unavailable"),
            Self::TokenCollision => write!(formatter, "unable to generate a unique resume token"),
            Self::UnknownSnake(id) => write!(formatter, "replacement source lacks snake {id}"),
            Self::DuplicateSnakeId(id) => {
                write!(formatter, "replacement source repeats snake {id}")
            }
            Self::DuplicateLeaseId(id) => {
                write!(formatter, "replacement source repeats lease {id}")
            }
            Self::InvalidLeaseTarget { lease_id, snake_id } => write!(
                formatter,
                "lease {lease_id} cannot replace external snake {snake_id}"
            ),
            Self::InvalidBrainOwner { snake_id } => {
                write!(
                    formatter,
                    "external snake {snake_id} has an invalid brain owner"
                )
            }
            Self::SnakeCapacityExceeded { required, maximum } => write!(
                formatter,
                "replacement requires {required} snakes; maximum is {maximum}"
            ),
            Self::BodyCapacityExceeded { required, maximum } => write!(
                formatter,
                "replacement requires {required} body points; maximum is {maximum}"
            ),
            Self::BrainCapacityExceeded { required, maximum } => write!(
                formatter,
                "replacement requires {required} brains; maximum is {maximum}"
            ),
            Self::RngIsolationViolation => write!(
                formatter,
                "external replacement changed a non-external RNG stream"
            ),
            Self::AllocatorIsolationViolation => write!(
                formatter,
                "external replacement changed an unrelated allocator domain"
            ),
            Self::ResultNotReady => write!(formatter, "external replacement result is not ready"),
            Self::AssignmentsPending => write!(
                formatter,
                "external replacement assignments are still awaiting delivery results"
            ),
            Self::InternalShapeMismatch => {
                write!(formatter, "external replacement staging shape mismatch")
            }
        }
    }
}

impl Error for ExternalReplacementError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorldCopy(error) => Some(error),
            Self::BrainCopy(error) => Some(error),
            Self::Genome(error) => Some(error),
            Self::Spawn(error) => Some(error),
            Self::Allocator(error) => Some(error),
            Self::Controller(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
