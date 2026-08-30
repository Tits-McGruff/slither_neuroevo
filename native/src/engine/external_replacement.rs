//! Atomic staging for externally controlled snakes that die during a fixed step.
//!
//! The temporary TypeScript runtime creates a new genome and body before it
//! attempts to send the replacement assignment.  The Rust migration keeps that
//! ordering, but isolates every draw to the external-controller RNG, allocates
//! fresh exact identities, checks the complete body against live geometry, and
//! exposes no result until the whole replacement set is ready.  A failed local
//! assignment send disconnects the already-staged replacement under the old
//! known resume token; it never leaves a player and brain simultaneously
//! steering the snake.

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

/// First external death-replacement staging contract.
pub const EXTERNAL_REPLACEMENT_VERSION: u32 = 1;
/// Current browser skin for an ordinary externally controlled snake.
const EXTERNAL_SNAKE_SKIN: u32 = 0;
/// Bytes of operating-system entropy used by one replacement resume token.
const RESUME_TOKEN_BYTES: usize = 24;
/// Unpadded base64url characters produced from exactly 24 bytes.
const RESUME_TOKEN_LENGTH: usize = 32;
/// Bounded attempts to avoid the vanishingly unlikely live-token collision.
const TOKEN_ATTEMPTS: usize = 8;
/// Base64url alphabet shared with Node's current `randomBytes(...).toString('base64url')`.
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Complete admitted settings and ceilings for one replacement batch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalReplacementConfig {
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
        if self.version != EXTERNAL_REPLACEMENT_VERSION
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
    assignment: ReplacementAssignment,
    snake_index: usize,
    disconnect: DisconnectProposal,
}

/// Result of one exact assignment result without exposing internal buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignmentResolution {
    /// The exact pending assignment accepted and rotated its token.
    Accepted,
    /// The exact pending assignment failed and entered disconnect grace.
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
                .reserve_frame_v1_ids(count_u32)
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
        self.validate_ready_shape(source_rng, source_allocators, config, graph)?;
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
        if accepted {
            let token = self
                .tokens
                .get(assignment.token_index)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let lease = self
                .world
                .controller_leases
                .iter_mut()
                .find(|lease| lease.id == assignment.lease_id)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            debug_assert!(lease.resume_token.capacity() >= token.len());
            lease.resume_token.clear();
            lease.resume_token.push_str(token);
            self.statuses[index] = AssignmentDeliveryStatus::Accepted;
            Ok(AssignmentResolution::Accepted)
        } else {
            let record = self.records[index];
            let lease_index = self
                .world
                .controller_leases
                .iter()
                .position(|lease| lease.id == assignment.lease_id)
                .ok_or(ExternalReplacementError::InternalShapeMismatch)?;
            let lease = &mut self.world.controller_leases[lease_index];
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
        let new_snake = SnakeState {
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
            body: BodyRange {
                start: body_start,
                len: self.replacement_body.len(),
            },
            skin: EXTERNAL_SNAKE_SKIN,
        };
        self.world.snakes[old_snake_index] = new_snake;

        let new_brain = BrainRuntimeState {
            handle: brain_handle,
            owner: BrainOwner::Entity(snake_id),
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
        replacement_lease.latest_action = LatestControllerAction {
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
            .reserve_frame_v1_ids(frame_count)
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
                    if lease.connection_id.is_some()
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

fn authority_world_digest(world: &WorldState) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"slither-external-replacement-world-v1\0");
    hash_usize(&mut hash, world.snakes.len());
    for snake in &world.snakes {
        hash.update(snake.id.to_le_bytes());
        hash.update(snake.frame_v1_id.to_le_bytes());
        hash.update([snake_kind_tag(snake.kind), u8::from(snake.alive)]);
        hash_option_u32(&mut hash, snake.population_slot);
        hash_option_brain(&mut hash, snake.brain);
        hash_option_u32(&mut hash, snake.baseline_slot);
        hash.update([snake
            .baseline_strategy
            .map_or(u8::MAX, baseline_strategy_tag)]);
        hash_point(&mut hash, snake.position);
        hash_point(&mut hash, snake.previous_position);
        hash.update(snake.direction.to_bits().to_le_bytes());
        hash.update(snake.radius.to_bits().to_le_bytes());
        hash.update(snake.speed.to_bits().to_le_bytes());
        hash.update([u8::from(snake.boost)]);
        hash.update(snake.age_seconds.to_bits().to_le_bytes());
        hash.update(snake.food.to_bits().to_le_bytes());
        hash.update(snake.points.to_bits().to_le_bytes());
        hash.update(snake.kills.to_le_bytes());
        hash.update(snake.target_length.to_bits().to_le_bytes());
        hash.update(snake.fitness.to_bits().to_le_bytes());
        hash.update(snake.turn.to_bits().to_le_bytes());
        hash.update(snake.previous_turn.to_bits().to_le_bytes());
        hash.update([
            u8::from(snake.input_boost),
            u8::from(snake.previous_input_boost),
        ]);
        hash.update(snake.control_accumulator_seconds.to_bits().to_le_bytes());
        hash.update(snake.delivered_observation_points.to_bits().to_le_bytes());
        hash_usize(&mut hash, snake.body.start);
        hash_usize(&mut hash, snake.body.len);
        hash.update(snake.skin.to_le_bytes());
    }
    hash_usize(&mut hash, world.body_points.len());
    for point in &world.body_points {
        hash_point(&mut hash, *point);
    }
    hash_usize(&mut hash, world.pellets.len());
    for pellet in &world.pellets {
        hash.update(pellet.id.to_le_bytes());
        hash_point(&mut hash, pellet.position);
        hash.update(pellet.value.to_bits().to_le_bytes());
        hash.update(pellet.kind.to_le_bytes());
        hash.update(pellet.color.to_le_bytes());
        hash_option_u64(&mut hash, pellet.owner);
    }
    hash_usize(&mut hash, world.controller_leases.len());
    for lease in &world.controller_leases {
        hash.update(lease.id.to_le_bytes());
        hash.update(lease.snake_id.to_le_bytes());
        hash.update([controller_kind_tag(lease.kind)]);
        hash_option_u64(&mut hash, lease.connection_id);
        hash_string(&mut hash, &lease.scope);
        hash_string(&mut hash, &lease.resume_token);
        hash.update([controller_status_tag(lease.status)]);
        hash.update(lease.latest_action.turn.to_bits().to_le_bytes());
        hash.update([u8::from(lease.latest_action.boost)]);
        hash.update(lease.latest_action.client_tick.to_le_bytes());
        hash.update(lease.latest_action.arrival_sequence.to_le_bytes());
        hash.update(lease.latest_action.accepted_at_ms.to_le_bytes());
        hash.update(lease.last_observed_at_ms.to_le_bytes());
        hash_option_u64(&mut hash, lease.disconnected_at_ms);
        hash_option_u64(&mut hash, lease.input_hold_expires_at_ms);
        hash_option_u64(&mut hash, lease.grace_expires_at_ms);
        hash_option_u64(&mut hash, lease.takeover_committed_at_ms);
    }
    hash.finalize().into()
}

fn authority_rng_digest(rng: &RngStateBundle) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"slither-external-replacement-rng-v1\0");
    hash.update(rng.version.to_le_bytes());
    hash_serialized_rng(&mut hash, &rng.world);
    hash_serialized_rng(&mut hash, &rng.evolution);
    hash_serialized_rng(&mut hash, &rng.external_controller);
    hash_usize(&mut hash, rng.baselines.len());
    for baseline in &rng.baselines {
        hash.update(baseline.slot.to_le_bytes());
        hash_serialized_rng(&mut hash, &baseline.state);
    }
    hash.finalize().into()
}

fn authority_brain_digest(brains: &[BrainRuntimeState]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"slither-external-replacement-brains-v1\0");
    hash_usize(&mut hash, brains.len());
    for brain in brains {
        hash.update(brain.handle.id.to_le_bytes());
        hash.update(brain.handle.epoch.to_le_bytes());
        match brain.owner {
            BrainOwner::PopulationSlot(slot) => {
                hash.update([0]);
                hash.update(slot.to_le_bytes());
            }
            BrainOwner::Entity(id) => {
                hash.update([1]);
                hash.update(id.to_le_bytes());
            }
        }
        match brain.non_population_weights.as_deref() {
            Some(weights) => {
                hash.update([1]);
                hash_f32_slice(&mut hash, weights);
            }
            None => hash.update([0]),
        }
        hash_f32_slice(&mut hash, &brain.recurrent);
    }
    hash.finalize().into()
}

fn hash_serialized_rng(hash: &mut Sha256, rng: &super::rng::SerializedRngState) {
    hash_string(hash, &rng.algorithm);
    hash.update(rng.version.to_le_bytes());
    hash_string(hash, &rng.state_hex);
    hash_string(hash, &rng.gaussian_algorithm);
    hash.update(rng.gaussian_version.to_le_bytes());
    hash.update([u8::from(rng.gaussian_spare_valid)]);
    match rng.gaussian_spare_hex.as_deref() {
        Some(value) => {
            hash.update([1]);
            hash_string(hash, value);
        }
        None => hash.update([0]),
    }
}

fn hash_f32_slice(hash: &mut Sha256, values: &[f32]) {
    hash_usize(hash, values.len());
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
}

fn hash_string(hash: &mut Sha256, value: &str) {
    hash_usize(hash, value.len());
    hash.update(value.as_bytes());
}

fn hash_usize(hash: &mut Sha256, value: usize) {
    hash.update((value as u64).to_le_bytes());
}

fn hash_point(hash: &mut Sha256, point: WorldPoint) {
    hash.update(point.x.to_bits().to_le_bytes());
    hash.update(point.y.to_bits().to_le_bytes());
}

fn hash_option_u32(hash: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.to_le_bytes());
        }
        None => hash.update([0]),
    }
}

fn hash_option_u64(hash: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.to_le_bytes());
        }
        None => hash.update([0]),
    }
}

fn hash_option_brain(hash: &mut Sha256, value: Option<BrainHandle>) {
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.id.to_le_bytes());
            hash.update(value.epoch.to_le_bytes());
        }
        None => hash.update([0]),
    }
}

const fn snake_kind_tag(kind: SnakeKind) -> u8 {
    match kind {
        SnakeKind::Evolved => 0,
        SnakeKind::External => 1,
        SnakeKind::Baseline => 2,
        SnakeKind::Resurrected => 3,
    }
}

const fn baseline_strategy_tag(strategy: super::state::BaselineStrategyState) -> u8 {
    match strategy {
        super::state::BaselineStrategyState::Roam => 0,
        super::state::BaselineStrategyState::Seek => 1,
        super::state::BaselineStrategyState::Avoid => 2,
        super::state::BaselineStrategyState::Boost => 3,
    }
}

const fn controller_kind_tag(kind: ControllerKind) -> u8 {
    match kind {
        ControllerKind::Player => 0,
        ControllerKind::ReinforcementLearning => 1,
    }
}

const fn controller_status_tag(status: ControllerLeaseStatus) -> u8 {
    match status {
        ControllerLeaseStatus::Connected => 0,
        ControllerLeaseStatus::HoldingLastInput => 1,
        ControllerLeaseStatus::ReservedNeutral => 2,
        ControllerLeaseStatus::NeuralTakeover => 3,
    }
}

fn find_unique_snake(
    world: &WorldState,
    snake_id: u64,
) -> Result<&SnakeState, ExternalReplacementError> {
    let index = find_unique_snake_index(world, snake_id)?;
    Ok(&world.snakes[index])
}

fn validate_generation_controller_source(
    world: &WorldState,
) -> Result<(usize, usize), ExternalReplacementError> {
    let mut connected = 0usize;
    for (index, lease) in world.controller_leases.iter().enumerate() {
        if world.controller_leases[..index]
            .iter()
            .any(|prior| prior.id == lease.id)
        {
            return Err(ExternalReplacementError::DuplicateLeaseId(lease.id));
        }
        if world.controller_leases[..index]
            .iter()
            .any(|prior| prior.snake_id == lease.snake_id)
        {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: lease.id,
                snake_id: lease.snake_id,
            });
        }
        let snake = find_unique_snake(world, lease.snake_id)?;
        if !snake.alive || snake.kind != SnakeKind::External {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: lease.id,
                snake_id: lease.snake_id,
            });
        }
        match (lease.status, lease.connection_id) {
            (ControllerLeaseStatus::Connected, Some(connection_id)) if connection_id != 0 => {
                if world.controller_leases[..index]
                    .iter()
                    .any(|prior| prior.connection_id == Some(connection_id))
                {
                    return Err(ExternalReplacementError::InvalidLeaseTarget {
                        lease_id: lease.id,
                        snake_id: lease.snake_id,
                    });
                }
                connected = connected.checked_add(1).ok_or(
                    ExternalReplacementError::ArithmeticOverflow {
                        context: "connected generation controller count",
                    },
                )?;
            }
            (
                ControllerLeaseStatus::HoldingLastInput
                | ControllerLeaseStatus::ReservedNeutral
                | ControllerLeaseStatus::NeuralTakeover,
                None,
            ) => {}
            _ => {
                return Err(ExternalReplacementError::InvalidLeaseTarget {
                    lease_id: lease.id,
                    snake_id: lease.snake_id,
                });
            }
        }
    }
    let omitted = world.controller_leases.len().checked_sub(connected).ok_or(
        ExternalReplacementError::ArithmeticOverflow {
            context: "omitted generation controller count",
        },
    )?;
    Ok((connected, omitted))
}

fn unavailable_reservation(
    lease: &super::state::ControllerLease,
) -> Result<UnavailableControllerReservation, ExternalReplacementError> {
    let reason = match (lease.status, lease.connection_id) {
        (
            ControllerLeaseStatus::HoldingLastInput | ControllerLeaseStatus::ReservedNeutral,
            None,
        ) => UnavailableControllerReason::SnakeUnavailable,
        (ControllerLeaseStatus::NeuralTakeover, None) => UnavailableControllerReason::GraceExpired,
        _ => {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: lease.id,
                snake_id: lease.snake_id,
            });
        }
    };
    Ok(UnavailableControllerReservation {
        source_lease_id: lease.id,
        source_snake_id: lease.snake_id,
        controller_kind: lease.kind,
        scope: lease.scope.clone(),
        resume_token: lease.resume_token.clone(),
        disconnected_at_ms: lease.disconnected_at_ms,
        grace_expires_at_ms: lease.grace_expires_at_ms,
        reason,
    })
}

fn find_unique_snake_index(
    world: &WorldState,
    snake_id: u64,
) -> Result<usize, ExternalReplacementError> {
    let mut found = None;
    for (index, snake) in world.snakes.iter().enumerate() {
        if snake.id != snake_id {
            continue;
        }
        if found.replace(index).is_some() {
            return Err(ExternalReplacementError::DuplicateSnakeId(snake_id));
        }
    }
    found.ok_or(ExternalReplacementError::UnknownSnake(snake_id))
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

fn encode_base64url_24(
    bytes: &[u8; RESUME_TOKEN_BYTES],
    output: &mut String,
) -> Result<(), ExternalReplacementError> {
    reserve_string(output, RESUME_TOKEN_LENGTH, "replacement resume token")?;
    output.clear();
    for chunk in bytes.chunks_exact(3) {
        let bits = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        output.push(BASE64URL[((bits >> 18) & 0x3f) as usize] as char);
        output.push(BASE64URL[((bits >> 12) & 0x3f) as usize] as char);
        output.push(BASE64URL[((bits >> 6) & 0x3f) as usize] as char);
        output.push(BASE64URL[(bits & 0x3f) as usize] as char);
    }
    if output.len() != RESUME_TOKEN_LENGTH {
        return Err(ExternalReplacementError::InternalShapeMismatch);
    }
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
mod tests {
    use super::*;
    use crate::engine::graph::{
        GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::rng::StatefulRng;
    use crate::engine::state::{
        PelletState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START, EXTERNAL_ENTITY_ID_START,
        RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION,
    };

    const OLD_SNAKE_ID: u64 = EXTERNAL_ENTITY_ID_START + 1;
    const OLD_BRAIN_ID: u64 = 20;
    const OLD_LEASE_ID: u64 = 40;
    const CONNECTION_ID: u64 = 77;
    const WALL_NOW_MS: u64 = 1_000;

    fn graph() -> GraphBundle {
        GraphBundle::compile(
            GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".to_owned(),
                        kind: GraphNodeKind::Input { output_size: 3 },
                    },
                    GraphNodeSpec {
                        id: "memory".to_owned(),
                        kind: GraphNodeKind::Gru {
                            input_size: 3,
                            hidden_size: 2,
                        },
                    },
                    GraphNodeSpec {
                        id: "head".to_owned(),
                        kind: GraphNodeKind::Dense {
                            input_size: 2,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![edge("input", "memory"), edge("memory", "head")],
                outputs: vec![GraphOutputRef {
                    node_id: "head".to_owned(),
                    port: None,
                }],
                output_size: 2,
            },
            &GraphLimits {
                max_nodes: 8,
                max_edges: 8,
                max_graph_outputs: 2,
                max_identifier_bytes: 32,
                max_total_referenced_identifier_bytes: 512,
                max_tensor_width: 16,
                max_mlp_hidden_layers: 2,
                max_split_output_ports: 2,
                max_parameter_floats: 1_000,
                max_recurrent_state_floats: 100,
                max_canonical_layout_bytes: 10_000,
                max_architecture_key_bytes: 10_000,
            },
        )
        .expect("replacement fixture graph must compile")
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            from_port: None,
            to_port: None,
        }
    }

    fn key() -> PhysicsStepKey {
        PhysicsStepKey::new(7, 3, 10, 4, 9, [0x5a; 32], 12)
    }

    fn rng() -> RngStateBundle {
        RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: StatefulRng::new(11.0).export_state(),
            evolution: StatefulRng::new(12.0).export_state(),
            external_controller: StatefulRng::new(13.0).export_state(),
            baselines: Vec::new(),
        }
    }

    fn allocators() -> AllocatorState {
        AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 100,
            next_brain_id: 500,
            next_genome_id: 600,
            next_controller_lease_id: 700,
            next_frame_v1_id: 1_000,
            next_external_id: EXTERNAL_ENTITY_ID_START + 100,
            next_baseline_id: BASELINE_ENTITY_ID_START + 100,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START + 100,
        }
    }

    fn snake(
        id: u64,
        frame_v1_id: u32,
        kind: SnakeKind,
        alive: bool,
        position: WorldPoint,
        body: BodyRange,
        brain: Option<BrainHandle>,
    ) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id,
            kind,
            alive,
            population_slot: None,
            brain,
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: 0.25,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 3.0,
            food: 2.0,
            points: 4.0,
            kills: 1,
            target_length: body.len as f64,
            fitness: 5.0,
            turn: 0.75,
            previous_turn: 0.5,
            input_boost: true,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 1.0,
            body,
            skin: 2,
        }
    }

    fn connected_lease(
        id: u64,
        snake_id: u64,
        connection_id: u64,
    ) -> super::super::state::ControllerLease {
        super::super::state::ControllerLease {
            id,
            snake_id,
            kind: ControllerKind::Player,
            connection_id: Some(connection_id),
            scope: "run-a".to_owned(),
            resume_token: format!("old-token-{id}"),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                turn: 0.75,
                boost: true,
                client_tick: 9,
                arrival_sequence: 10,
                accepted_at_ms: 900,
            },
            last_observed_at_ms: 900,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        }
    }

    fn fixture(graph: &CompiledGraph) -> (WorldState, Vec<BrainRuntimeState>) {
        let obstacle_body = [
            WorldPoint { x: -20.0, y: 0.0 },
            WorldPoint { x: -27.5, y: 0.0 },
        ];
        let dead_body = [
            WorldPoint { x: 100.0, y: 100.0 },
            WorldPoint { x: 92.5, y: 100.0 },
        ];
        let world = WorldState {
            snakes: vec![
                snake(
                    1,
                    1,
                    SnakeKind::Evolved,
                    true,
                    obstacle_body[0],
                    BodyRange { start: 0, len: 2 },
                    None,
                ),
                snake(
                    OLD_SNAKE_ID,
                    10,
                    SnakeKind::External,
                    false,
                    dead_body[0],
                    BodyRange { start: 2, len: 2 },
                    Some(BrainHandle {
                        id: OLD_BRAIN_ID,
                        epoch: key().population_epoch(),
                    }),
                ),
            ],
            body_points: obstacle_body.into_iter().chain(dead_body).collect(),
            pellets: vec![PelletState {
                id: 8,
                position: WorldPoint { x: 10.0, y: 10.0 },
                value: 1.0,
                kind: 2,
                color: 3,
                owner: Some(OLD_SNAKE_ID),
            }],
            controller_leases: vec![connected_lease(OLD_LEASE_ID, OLD_SNAKE_ID, CONNECTION_ID)],
        };
        let brains = vec![BrainRuntimeState {
            handle: BrainHandle {
                id: OLD_BRAIN_ID,
                epoch: key().population_epoch(),
            },
            owner: BrainOwner::Entity(OLD_SNAKE_ID),
            non_population_weights: Some(vec![0.25; graph.total_parameters].into_boxed_slice()),
            recurrent: vec![0.5; graph.total_state_size].into_boxed_slice(),
        }];
        (world, brains)
    }

    fn generation_base(graph: &CompiledGraph) -> (WorldState, Vec<BrainRuntimeState>) {
        let epoch = key().population_epoch() + 1;
        let evolved_head = WorldPoint { x: -80.0, y: -40.0 };
        let baseline_head = WorldPoint { x: 80.0, y: 40.0 };
        let mut evolved = snake(
            1,
            20,
            SnakeKind::Evolved,
            true,
            evolved_head,
            BodyRange { start: 0, len: 2 },
            Some(BrainHandle { id: 21, epoch }),
        );
        evolved.population_slot = Some(0);
        let mut baseline = snake(
            BASELINE_ENTITY_ID_START + 1,
            21,
            SnakeKind::Baseline,
            true,
            baseline_head,
            BodyRange { start: 2, len: 2 },
            None,
        );
        baseline.baseline_slot = Some(0);
        baseline.baseline_strategy = Some(super::super::state::BaselineStrategyState::Roam);
        let world = WorldState {
            snakes: vec![evolved, baseline],
            body_points: vec![
                evolved_head,
                WorldPoint { x: -87.5, y: -40.0 },
                baseline_head,
                WorldPoint { x: 72.5, y: 40.0 },
            ],
            pellets: vec![PelletState {
                id: 9,
                position: WorldPoint { x: 0.0, y: 180.0 },
                value: 1.0,
                kind: 1,
                color: 2,
                owner: None,
            }],
            controller_leases: Vec::new(),
        };
        let brains = vec![BrainRuntimeState {
            handle: BrainHandle { id: 21, epoch },
            owner: BrainOwner::PopulationSlot(0),
            non_population_weights: None,
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        }];
        (world, brains)
    }

    fn generation_controller_source() -> WorldState {
        let graph = graph();
        let (mut world, _) = fixture(&graph);
        let connected = world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == OLD_SNAKE_ID)
            .expect("connected external source snake");
        connected.alive = true;

        let disconnected_id = OLD_SNAKE_ID + 1;
        let disconnected_head = WorldPoint { x: 300.0, y: 300.0 };
        let body_start = world.body_points.len();
        world
            .body_points
            .extend([disconnected_head, WorldPoint { x: 292.5, y: 300.0 }]);
        world.snakes.push(snake(
            disconnected_id,
            11,
            SnakeKind::External,
            true,
            disconnected_head,
            BodyRange {
                start: body_start,
                len: 2,
            },
            Some(BrainHandle {
                id: OLD_BRAIN_ID + 1,
                epoch: key().population_epoch(),
            }),
        ));
        let mut disconnected =
            connected_lease(OLD_LEASE_ID + 1, disconnected_id, CONNECTION_ID + 1);
        disconnected.connection_id = None;
        disconnected.status = ControllerLeaseStatus::ReservedNeutral;
        disconnected.disconnected_at_ms = Some(800);
        disconnected.input_hold_expires_at_ms = Some(1_300);
        disconnected.grace_expires_at_ms = Some(30_800);
        world.controller_leases.push(disconnected);
        world
    }

    fn deterministic_entropy(seed: u8) -> impl FnMut(&mut [u8]) -> Result<(), ()> {
        let mut call = 0u8;
        move |bytes| {
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = seed
                    .wrapping_add(call)
                    .wrapping_add(u8::try_from(index).unwrap_or(0));
            }
            call = call.wrapping_add(1);
            Ok(())
        }
    }

    #[test]
    fn generation_reassignment_preserves_the_base_and_replaces_only_connected_owners() {
        let graph = graph();
        let (base_world, base_brains) = generation_base(&graph);
        let controller_source = generation_controller_source();
        let base_world_before = base_world.clone();
        let base_brains_before = base_brains.clone();
        let controller_source_before = controller_source.clone();
        let base_rng = rng();
        let base_rng_before = base_rng.clone();
        let base_allocators = allocators();
        let base_allocators_before = base_allocators.clone();
        let successor_epoch = key().population_epoch() + 1;
        let mut workspace = ExternalReplacementWorkspace::new();
        let (assignment, new_token) = {
            let prepared = workspace
                .prepare_generation_reassignments_with_entropy(
                    key(),
                    &base_world,
                    &base_rng,
                    &base_allocators,
                    &base_brains,
                    &controller_source,
                    &graph,
                    successor_epoch,
                    WALL_NOW_MS,
                    ExternalReplacementConfig::typescript_defaults(),
                    deterministic_entropy(31),
                )
                .expect("one connected generation owner must receive a replacement");
            assert_eq!(prepared.assignments().len(), 1);
            assert_eq!(prepared.statuses(), &[AssignmentDeliveryStatus::Pending]);
            assert_eq!(prepared.diagnostics().replacements, 1);
            assert_eq!(prepared.diagnostics().removed_dead_leases, 1);
            assert_eq!(prepared.unavailable_reservations().len(), 1);
            let unavailable = &prepared.unavailable_reservations()[0];
            assert_eq!(unavailable.source_lease_id, OLD_LEASE_ID + 1);
            assert_eq!(unavailable.source_snake_id, OLD_SNAKE_ID + 1);
            assert_eq!(
                unavailable.reason,
                UnavailableControllerReason::SnakeUnavailable
            );
            assert_eq!(
                unavailable.resume_token,
                format!("old-token-{}", OLD_LEASE_ID + 1)
            );
            assert_eq!(unavailable.grace_expires_at_ms, Some(30_800));
            (
                prepared.assignments()[0],
                prepared.resume_token(0).unwrap().to_owned(),
            )
        };

        assert_eq!(base_world, base_world_before);
        assert_eq!(base_brains, base_brains_before);
        assert_eq!(controller_source, controller_source_before);
        assert_eq!(base_rng, base_rng_before);
        assert_eq!(base_allocators, base_allocators_before);
        assert_eq!(assignment.source_lease_id, OLD_LEASE_ID);
        assert!(controller_source
            .controller_leases
            .iter()
            .any(|lease| lease.id == OLD_LEASE_ID + 1));
        assert!(!workspace
            .assignments(key())
            .unwrap()
            .iter()
            .any(|event| event.source_lease_id == OLD_LEASE_ID + 1));

        assert_eq!(
            workspace
                .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, true,)
                .unwrap(),
            AssignmentResolution::Accepted
        );
        let staged_world = workspace.world(key()).unwrap();
        assert_eq!(
            &staged_world.snakes[..base_world.snakes.len()],
            &base_world.snakes
        );
        assert_eq!(staged_world.pellets, base_world.pellets);
        assert_eq!(staged_world.controller_leases.len(), 1);
        let lease = &staged_world.controller_leases[0];
        assert_eq!(lease.id, assignment.lease_id);
        assert_eq!(lease.resume_token, new_token);
        assert_eq!(lease.connection_id, Some(CONNECTION_ID));
        assert_eq!(lease.status, ControllerLeaseStatus::Connected);
        let replacement = staged_world
            .snakes
            .iter()
            .find(|snake| snake.id == assignment.snake_id)
            .expect("connected owner replacement snake");
        assert!(replacement.alive);
        assert_eq!(replacement.kind, SnakeKind::External);

        let staged_brains = workspace.brains(key()).unwrap();
        assert_eq!(&staged_brains[..base_brains.len()], &base_brains);
        let external_brain = staged_brains
            .iter()
            .find(|brain| brain.owner == BrainOwner::Entity(assignment.snake_id))
            .expect("replacement external brain");
        assert_eq!(external_brain.handle.epoch, successor_epoch);
        let staged_rng = workspace.rng(key()).unwrap();
        assert_eq!(staged_rng.world, base_rng.world);
        assert_eq!(staged_rng.evolution, base_rng.evolution);
        assert_eq!(staged_rng.baselines, base_rng.baselines);
        assert_ne!(staged_rng.external_controller, base_rng.external_controller);
        let staged_allocators = workspace.allocators(key()).unwrap();
        assert_eq!(
            staged_allocators.next_entity_id,
            base_allocators.next_entity_id
        );
        assert_eq!(
            staged_allocators.next_genome_id,
            base_allocators.next_genome_id
        );
        assert_eq!(
            staged_allocators.next_baseline_id,
            base_allocators.next_baseline_id
        );
        assert_eq!(
            staged_allocators.next_external_id,
            base_allocators.next_external_id + 1
        );
        assert_eq!(
            staged_allocators.next_controller_lease_id,
            base_allocators.next_controller_lease_id + 1
        );
    }

    #[test]
    fn generation_reassignment_without_connected_owners_consumes_no_rng_or_ids() {
        let graph = graph();
        let (base_world, base_brains) = generation_base(&graph);
        let mut controller_source = generation_controller_source();
        controller_source.controller_leases.remove(0);
        controller_source
            .snakes
            .retain(|snake| snake.id != OLD_SNAKE_ID);
        let base_rng = rng();
        let base_allocators = allocators();
        let mut workspace = ExternalReplacementWorkspace::new();
        {
            let prepared = workspace
                .prepare_generation_reassignments_with_entropy(
                    key(),
                    &base_world,
                    &base_rng,
                    &base_allocators,
                    &base_brains,
                    &controller_source,
                    &graph,
                    key().population_epoch() + 1,
                    WALL_NOW_MS,
                    ExternalReplacementConfig::typescript_defaults(),
                    |_| panic!("no entropy is allowed without a connected owner"),
                )
                .expect("a disconnected reservation is omitted without replacement");
            assert!(prepared.assignments().is_empty());
            assert_eq!(prepared.diagnostics().removed_dead_leases, 1);
            assert_eq!(prepared.unavailable_reservations().len(), 1);
        }
        assert_eq!(workspace.world(key()).unwrap(), &base_world);
        assert_eq!(workspace.brains(key()).unwrap(), &base_brains);
        assert_eq!(workspace.rng(key()).unwrap(), &base_rng);
        assert_eq!(workspace.allocators(key()).unwrap(), &base_allocators);
    }

    #[test]
    fn invalid_generation_controller_source_is_atomic() {
        let graph = graph();
        let (base_world, base_brains) = generation_base(&graph);
        let mut controller_source = generation_controller_source();
        controller_source.controller_leases[0].connection_id = None;
        let base_rng = rng();
        let base_allocators = allocators();
        let mut workspace = ExternalReplacementWorkspace::new();
        let error = workspace
            .prepare_generation_reassignments_with_entropy(
                key(),
                &base_world,
                &base_rng,
                &base_allocators,
                &base_brains,
                &controller_source,
                &graph,
                key().population_epoch() + 1,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                deterministic_entropy(41),
            )
            .expect_err("a connected lease without a live connection must fail");
        assert!(matches!(
            error,
            ExternalReplacementError::InvalidLeaseTarget {
                lease_id: OLD_LEASE_ID,
                snake_id: OLD_SNAKE_ID
            }
        ));
        assert!(!workspace.is_ready());
        assert_eq!(base_world, generation_base(&graph).0);
        assert_eq!(base_rng, rng());
        assert_eq!(base_allocators, allocators());
    }

    #[test]
    fn connected_death_stages_fresh_external_only_state_and_waits_for_delivery() {
        let graph = graph();
        let (world, brains) = fixture(&graph);
        let source_world = world.clone();
        let source_rng = rng();
        let source_allocators = allocators();
        let config = ExternalReplacementConfig::typescript_defaults();
        let mut workspace = ExternalReplacementWorkspace::new();
        let (assignment, token) = {
            let prepared = workspace
                .prepare_with_entropy(
                    key(),
                    &world,
                    &source_rng,
                    &source_allocators,
                    &brains,
                    &graph,
                    WALL_NOW_MS,
                    config,
                    deterministic_entropy(1),
                )
                .expect("connected death must stage one replacement");
            assert_eq!(prepared.assignments().len(), 1);
            assert_eq!(prepared.statuses(), &[AssignmentDeliveryStatus::Pending]);
            let assignment = prepared.assignments()[0];
            let token = prepared.resume_token(0).unwrap().to_owned();
            assert_eq!(token.len(), RESUME_TOKEN_LENGTH);
            assert_eq!(assignment.source_lease_id, OLD_LEASE_ID);
            assert_eq!(assignment.snake_id, source_allocators.next_external_id);
            assert_eq!(assignment.frame_v1_id, source_allocators.next_frame_v1_id);
            assert_eq!(
                assignment.lease_id,
                source_allocators.next_controller_lease_id
            );
            assert_eq!(world, source_world);
            assert_eq!(source_rng, rng());
            assert_eq!(source_allocators, allocators());
            (assignment, token)
        };

        assert!(matches!(
            workspace.world(key()),
            Err(ExternalReplacementError::AssignmentsPending)
        ));
        assert_eq!(
            workspace
                .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, true,)
                .unwrap(),
            AssignmentResolution::Accepted
        );

        let staged_world = workspace.world(key()).unwrap();
        let replacement = staged_world
            .snakes
            .iter()
            .find(|snake| snake.id == assignment.snake_id)
            .expect("replacement snake");
        assert!(replacement.alive);
        assert_eq!(replacement.kind, SnakeKind::External);
        assert_eq!(replacement.body.len, config.spawn.snake_start_len);
        assert_eq!(
            staged_world.body_points[replacement.body.start],
            replacement.position
        );
        assert!(staged_world
            .pellets
            .iter()
            .all(|pellet| pellet.owner.is_none()));
        let lease = staged_world
            .controller_leases
            .iter()
            .find(|lease| lease.id == assignment.lease_id)
            .expect("replacement lease");
        assert_eq!(lease.resume_token, token);
        assert_eq!(lease.connection_id, Some(CONNECTION_ID));
        assert_eq!(lease.status, ControllerLeaseStatus::Connected);
        assert_eq!(lease.latest_action.turn, 0.0);
        assert!(!lease.latest_action.boost);

        let brain = workspace
            .brains(key())
            .unwrap()
            .iter()
            .find(|brain| brain.owner == BrainOwner::Entity(assignment.snake_id))
            .expect("replacement brain");
        assert_eq!(brain.handle.id, source_allocators.next_brain_id);
        assert_eq!(
            brain.non_population_weights.as_ref().unwrap().len(),
            graph.total_parameters
        );
        assert!(brain
            .non_population_weights
            .as_ref()
            .unwrap()
            .iter()
            .any(|value| value.to_bits() != 0));
        assert_eq!(brain.recurrent.len(), graph.total_state_size);
        assert!(brain.recurrent.iter().all(|value| value.to_bits() == 0));

        let staged_rng = workspace.rng(key()).unwrap();
        assert_eq!(staged_rng.world, source_rng.world);
        assert_eq!(staged_rng.evolution, source_rng.evolution);
        assert_eq!(staged_rng.baselines, source_rng.baselines);
        assert_ne!(
            staged_rng.external_controller,
            source_rng.external_controller
        );
        let staged_allocators = workspace.allocators(key()).unwrap();
        assert_eq!(
            staged_allocators.next_external_id,
            source_allocators.next_external_id + 1
        );
        assert_eq!(
            staged_allocators.next_frame_v1_id,
            source_allocators.next_frame_v1_id + 1
        );
        assert_eq!(
            staged_allocators.next_brain_id,
            source_allocators.next_brain_id + 1
        );
        assert_eq!(
            staged_allocators.next_controller_lease_id,
            source_allocators.next_controller_lease_id + 1
        );
        assert_eq!(
            staged_allocators.next_entity_id,
            source_allocators.next_entity_id
        );
        assert_eq!(
            staged_allocators.next_genome_id,
            source_allocators.next_genome_id
        );
    }

    #[test]
    fn failed_assignment_keeps_the_known_token_and_enters_exact_grace() {
        let graph = graph();
        let (world, brains) = fixture(&graph);
        let source_rng = rng();
        let source_allocators = allocators();
        let mut workspace = ExternalReplacementWorkspace::new();
        let (assignment, new_token) = {
            let prepared = workspace
                .prepare_with_entropy(
                    key(),
                    &world,
                    &source_rng,
                    &source_allocators,
                    &brains,
                    &graph,
                    WALL_NOW_MS,
                    ExternalReplacementConfig::typescript_defaults(),
                    deterministic_entropy(2),
                )
                .unwrap();
            (
                prepared.assignments()[0],
                prepared.resume_token(0).unwrap().to_owned(),
            )
        };
        assert_eq!(
            workspace
                .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, false,)
                .unwrap(),
            AssignmentResolution::Failed
        );
        assert_eq!(
            workspace
                .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, true,)
                .unwrap(),
            AssignmentResolution::Ignored
        );
        let staged = workspace.world(key()).unwrap();
        let lease = staged
            .controller_leases
            .iter()
            .find(|lease| lease.id == assignment.lease_id)
            .unwrap();
        let snake = staged
            .snakes
            .iter()
            .find(|snake| snake.id == assignment.snake_id)
            .unwrap();
        assert_eq!(lease.resume_token, format!("old-token-{OLD_LEASE_ID}"));
        assert_ne!(lease.resume_token, new_token);
        assert_eq!(lease.connection_id, None);
        assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
        assert_eq!(lease.disconnected_at_ms, Some(WALL_NOW_MS));
        assert_eq!(lease.input_hold_expires_at_ms, Some(WALL_NOW_MS + 500));
        assert_eq!(lease.grace_expires_at_ms, Some(WALL_NOW_MS + 30_000));
        assert_eq!(snake.turn, 0.0);
        assert!(!snake.input_boost);
        assert!(snake.alive);
    }

    #[test]
    fn opaque_authority_proof_rejects_any_post_validation_buffer_substitution() {
        let graph = graph();
        let (world, brains) = fixture(&graph);
        let source_rng = rng();
        let source_allocators = allocators();
        let mut workspace = ExternalReplacementWorkspace::new();
        {
            let prepared = workspace
                .prepare_with_entropy(
                    key(),
                    &world,
                    &source_rng,
                    &source_allocators,
                    &brains,
                    &graph,
                    WALL_NOW_MS,
                    ExternalReplacementConfig::typescript_defaults(),
                    deterministic_entropy(9),
                )
                .expect("valid replacement must issue one opaque authority proof");
            assert_eq!(prepared.assignments().len(), 1);
        }

        let buffers = workspace.validation_buffers(key()).unwrap();
        assert!(buffers.proof.matches(
            key(),
            buffers.world,
            buffers.rng,
            buffers.allocators,
            buffers.brains,
        ));

        let brain_index = buffers
            .brains
            .iter()
            .position(|brain| brain.owner == BrainOwner::Entity(allocators().next_external_id))
            .unwrap();
        let original_weight = buffers.brains[brain_index]
            .non_population_weights
            .as_ref()
            .unwrap()[0];
        buffers.brains[brain_index]
            .non_population_weights
            .as_mut()
            .unwrap()[0] = f32::from_bits(original_weight.to_bits() ^ 1);
        assert!(!buffers.proof.matches(
            key(),
            buffers.world,
            buffers.rng,
            buffers.allocators,
            buffers.brains,
        ));
        buffers.brains[brain_index]
            .non_population_weights
            .as_mut()
            .unwrap()[0] = original_weight;

        let original_rng_state = buffers.rng.external_controller.state_hex.clone();
        buffers.rng.external_controller.state_hex = "12345678".into();
        assert!(!buffers.proof.matches(
            key(),
            buffers.world,
            buffers.rng,
            buffers.allocators,
            buffers.brains,
        ));
        buffers.rng.external_controller.state_hex = original_rng_state;

        let original_external_id = buffers.allocators.next_external_id;
        buffers.allocators.next_external_id += 1;
        assert!(!buffers.proof.matches(
            key(),
            buffers.world,
            buffers.rng,
            buffers.allocators,
            buffers.brains,
        ));
        buffers.allocators.next_external_id = original_external_id;

        let removed_lease = buffers.world.controller_leases.pop().unwrap();
        assert!(!buffers.proof.matches(
            key(),
            buffers.world,
            buffers.rng,
            buffers.allocators,
            buffers.brains,
        ));
        buffers.world.controller_leases.push(removed_lease);
        assert!(buffers.proof.matches(
            key(),
            buffers.world,
            buffers.rng,
            buffers.allocators,
            buffers.brains,
        ));
    }

    #[test]
    fn disconnected_dead_lease_is_removed_without_rng_or_allocator_use() {
        let graph = graph();
        let (mut world, brains) = fixture(&graph);
        let lease = &mut world.controller_leases[0];
        lease.status = ControllerLeaseStatus::ReservedNeutral;
        lease.connection_id = None;
        lease.disconnected_at_ms = Some(900);
        lease.input_hold_expires_at_ms = Some(1_400);
        lease.grace_expires_at_ms = Some(30_900);
        lease.last_observed_at_ms = 900;
        let source_rng = rng();
        let source_allocators = allocators();
        let mut workspace = ExternalReplacementWorkspace::new();
        {
            let prepared = workspace
                .prepare_with_entropy(
                    key(),
                    &world,
                    &source_rng,
                    &source_allocators,
                    &brains,
                    &graph,
                    WALL_NOW_MS,
                    ExternalReplacementConfig::typescript_defaults(),
                    |_| panic!("no token entropy is allowed without a live connection"),
                )
                .unwrap();
            assert!(prepared.assignments().is_empty());
            assert_eq!(prepared.diagnostics().removed_dead_leases, 1);
            assert_eq!(prepared.unavailable_reservations().len(), 1);
            assert_eq!(
                prepared.unavailable_reservations()[0].resume_token,
                format!("old-token-{OLD_LEASE_ID}")
            );
        }
        assert!(workspace.world(key()).unwrap().controller_leases.is_empty());
        assert_eq!(workspace.rng(key()).unwrap(), &source_rng);
        assert_eq!(workspace.allocators(key()).unwrap(), &source_allocators);
    }

    #[test]
    fn entropy_and_aggregate_work_failures_expose_no_result_and_retry_cleanly() {
        let graph = graph();
        let (world, brains) = fixture(&graph);
        let source_rng = rng();
        let source_allocators = allocators();
        let config = ExternalReplacementConfig::typescript_defaults();
        let mut workspace = ExternalReplacementWorkspace::new();
        let error = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                config,
                |_| Err(()),
            )
            .expect_err("OS entropy failure must reject staging");
        assert!(matches!(
            error,
            ExternalReplacementError::EntropyUnavailable
        ));
        assert!(!workspace.is_ready());
        assert!(matches!(
            workspace.world(key()),
            Err(ExternalReplacementError::ResultNotReady)
        ));

        {
            let retry = workspace
                .prepare_with_entropy(
                    key(),
                    &world,
                    &source_rng,
                    &source_allocators,
                    &brains,
                    &graph,
                    WALL_NOW_MS,
                    config,
                    deterministic_entropy(9),
                )
                .expect("a clean retry must succeed");
            assert_eq!(retry.assignments().len(), 1);
        }

        let mut two_world = world.clone();
        let second_id = OLD_SNAKE_ID + 1;
        let second_start = two_world.body_points.len();
        let second_head = WorldPoint { x: 300.0, y: 300.0 };
        two_world
            .body_points
            .extend([second_head, WorldPoint { x: 292.5, y: 300.0 }]);
        two_world.snakes.push(snake(
            second_id,
            11,
            SnakeKind::External,
            false,
            second_head,
            BodyRange {
                start: second_start,
                len: 2,
            },
            None,
        ));
        two_world.controller_leases.push(connected_lease(
            OLD_LEASE_ID + 1,
            second_id,
            CONNECTION_ID + 1,
        ));
        let mut tight = config;
        tight.spawn.maximum_candidates_per_batch = 1;
        let error = workspace
            .prepare_with_entropy(
                key(),
                &two_world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                tight,
                deterministic_entropy(11),
            )
            .expect_err("the second replacement must not exceed the shared work budget");
        assert!(matches!(
            error,
            ExternalReplacementError::Spawn(inner)
                if matches!(*inner, SpawnError::WorkBudgetExceeded { .. })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(world, fixture(&graph).0);
        assert_eq!(source_rng, rng());
        assert_eq!(source_allocators, allocators());
    }

    #[test]
    fn stale_or_duplicate_assignment_results_cannot_resolve_the_batch() {
        let graph = graph();
        let (world, brains) = fixture(&graph);
        let source_rng = rng();
        let source_allocators = allocators();
        let mut workspace = ExternalReplacementWorkspace::new();
        let assignment = {
            let prepared = workspace
                .prepare_with_entropy(
                    key(),
                    &world,
                    &source_rng,
                    &source_allocators,
                    &brains,
                    &graph,
                    WALL_NOW_MS,
                    ExternalReplacementConfig::typescript_defaults(),
                    deterministic_entropy(13),
                )
                .unwrap();
            prepared.assignments()[0]
        };
        assert_eq!(
            workspace
                .resolve_assignment(
                    key(),
                    assignment.lease_id,
                    assignment.connection_id + 1,
                    true,
                )
                .unwrap(),
            AssignmentResolution::Ignored
        );
        assert_eq!(
            workspace.statuses(key()).unwrap(),
            &[AssignmentDeliveryStatus::Pending]
        );
        assert!(matches!(
            workspace.world(key()),
            Err(ExternalReplacementError::AssignmentsPending)
        ));
        assert!(matches!(
            workspace.resolve_assignment(
                PhysicsStepKey::new(8, 3, 10, 4, 9, [0x5a; 32], 12),
                assignment.lease_id,
                assignment.connection_id,
                true,
            ),
            Err(ExternalReplacementError::ResultNotReady)
        ));
    }
}
