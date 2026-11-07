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

mod admission_header;
pub(crate) use admission_header::decode_sha256_identity;
use admission_header::{validate_admission_header, validate_policy};
mod config_validation;
use config_validation::{
    validate_allocators, validate_candidate, validate_config, validate_fixed_step_continuation,
    validate_generation, validate_next_after, validate_rng_bundle,
};
mod world_validation;
use world_validation::{
    validate_generation_boundary, validate_population, validate_world_with_scratch,
};
mod value_checks;
pub(crate) use value_checks::validate_f32_slice;
use value_checks::{
    checked_add, checked_sum, invalid, invalid_error, validate_build_class,
    validate_nonnegative_f64, validate_point, validate_positive_f64, validate_sha256,
    validate_sha256_hex, validate_text,
};
mod config_hash;

pub use config_hash::{normalized_config_hash, normalized_settings_schema_hash};
mod memory_budget;

pub use memory_budget::{
    checked_allocation_bytes, estimate_state_memory, preflight_generation_boundary_allocation,
};
// Keep the existing crate facade in production builds that omit its test-hook caller.
#[allow(unused_imports)]
pub(crate) use memory_budget::{estimate_graph_memory, generation_numeric_peak_bytes};
mod publication_buffers;
use publication_buffers::{
    generation_start_buffer_identity, running_step_buffer_identity, swap_generation_start_buffers,
    swap_initial_run_start_buffers, swap_running_step_buffers,
};
mod publication_checks;
use publication_checks::{
    successor_admission_policy, validate_generation_start_replacement_contract,
    validate_generation_successor_relation, validate_resolved_running_replacement_contract,
    validate_running_mutable_state, validate_running_replacement_contract,
};
mod contracts;
pub use contracts::{
    AllocatorState, AuthorityPhase, BaselineRngState, BaselineStrategyState, BodyRange,
    BrainHandle, BrainOwner, BrainRuntimeState, ContractVersions, ControllerKind, ControllerLease,
    ControllerLeaseStatus, EvolvedRespawnRngState, FixedStepContinuationState,
    FrameV1IdReservation, GenerationBoundaryKind, GenerationState, GenomeLineage,
    InternalIdReservation, LatestControllerAction, NormalizedEngineConfig, NormalizedSetting,
    NormalizedSettingValue, PelletState, PopulationGenome, RngStateBundle, RunIdentity, SnakeKind,
    SnakeState, StateAdmissionPolicy, StateCandidate, StateMemoryEstimate, WorldPoint, WorldState,
};
mod errors;
pub use errors::StateError;

#[cfg(test)]
mod tests;
