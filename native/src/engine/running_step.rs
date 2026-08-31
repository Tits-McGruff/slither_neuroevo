//! Private orchestration of one authoritative fixed-step attempt.
//!
//! This joins the already-reviewed prefix, shared controller, physics, and
//! publication transactions without exposing their intermediate mutable state.
//! External observations are exposed only after a complete physical step has
//! staged successfully, then the final swap waits for matching local Node send
//! results. Connected controlled-snake deaths stage fresh collision-safe
//! replacements and wait for the same reliable result boundary. A terminal
//! physical result instead stages one owned next-generation checkpoint boundary
//! beside the unchanged source authority; it never performs old-generation
//! controlled-death replacement first.

use super::checkpoint::{CheckpointDescriptor, CheckpointLimits, CheckpointOperationId};
use super::control::{NeuralControlError, NeuralControlPipeline};
use super::control_phase::{
    ControlCommitWorkspace, ControlPhaseError, ControlPhaseInputs, ControlPhaseWorkspace,
    PreparedExternalObservation,
};
use super::external_replacement::{AssignmentResolution, UnavailableControllerReservation};
use super::fixed_step::{FixedStepPrefixError, FixedStepPrefixInputs, FixedStepPrefixWorkspace};
use super::generation::{
    admit_prepared_generation_boundary, prepare_generation_boundary, AdmittedGenerationBoundary,
    GenerationCommitRecord, GenerationTransitionError, PreparedGenerationMetadata,
};
use super::generation_start::{
    GenerationStartConfig, GenerationStartError, GenerationStartWorkspace, PreparedGenerationStart,
};
use super::graph::GraphLimits;
use super::inference::{GraphExecutionPlan, InferenceError, InferenceMathBackend};
#[cfg(feature = "engine-test-hooks")]
use super::physics::PhysicsPhaseAllocations;
use super::physics::PhysicsStepKey;
use super::sensors::{SensorError, SensorEvaluator};
use super::state::{
    AuthoritativeState, AuthorityPhase, ControllerKind, ControllerLeaseStatus,
    FixedStepContinuationState, GenerationStartPreflight, GenerationStartPublication,
    GenerationStartReplacement, RunningStepPreflight, RunningStepPublication,
    RunningStepReplacement, SnakeKind, StateCandidate, StateError, WorldPoint, WorldState,
};
use super::step_config::{
    GenerationGuardConfig, RunningStepConfigProjection, RunningStepWorkLimits, StepConfigError,
};
use super::world_step::{
    ExternalDeliveryStatus, WorldStepDiagnostics, WorldStepError, WorldStepWorkspace,
};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
#[cfg(feature = "engine-test-hooks")]
use std::time::Instant;

/// First complete nonterminal phase-chain coordinator contract.
pub const RUNNING_STEP_COORDINATOR_VERSION: u32 = 1;

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

/// Scheduler and wall-clock values supplied for exactly one fixed-step attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningStepInputs {
    /// Monotonic wall-clock boundary used only by controller leases.
    pub wall_now_ms: u64,
    /// Scheduler debt remaining after this fixed step commits.
    pub wall_accumulator_seconds: f64,
}

/// Opaque final authority handoff after every reliable external event resolved.
///
/// Only this module can construct the wrapper, and it does so immediately after
/// the prevalidated observation-marker, assignment-token, or disconnect writes.
/// Sibling engine modules may build scratch [`RunningStepReplacement`] values,
/// but cannot use a retained preflight token to publish one after mutating it.
pub(crate) struct ResolvedRunningStepReplacement<'buffers> {
    replacement: RunningStepReplacement<'buffers>,
    external_replacements: usize,
    removed_dead_external_leases: usize,
}

/// Opaque final generation handoff after every reliable assignment resolved.
///
/// Only the coordinator can construct this wrapper from the same buffers that
/// passed the pre-delivery successor admission.
pub(crate) struct ResolvedGenerationStartReplacement<'buffers> {
    replacement: GenerationStartReplacement<'buffers>,
    external_replacements: usize,
    removed_source_leases: usize,
}

impl<'buffers> ResolvedGenerationStartReplacement<'buffers> {
    fn new(replacement: GenerationStartReplacement<'buffers>) -> Self {
        let external_replacements = replacement.proof.replacements();
        let removed_source_leases = replacement.proof.removed_dead_leases();
        Self {
            replacement,
            external_replacements,
            removed_source_leases,
        }
    }

    pub(crate) fn into_parts(self) -> (GenerationStartReplacement<'buffers>, usize, usize) {
        (
            self.replacement,
            self.external_replacements,
            self.removed_source_leases,
        )
    }
}

impl<'buffers> ResolvedRunningStepReplacement<'buffers> {
    fn new(replacement: RunningStepReplacement<'buffers>) -> Self {
        let external_replacements = replacement.mutation.external_replacements();
        let removed_dead_external_leases = replacement.mutation.removed_dead_external_leases();
        Self {
            replacement,
            external_replacements,
            removed_dead_external_leases,
        }
    }

    pub(crate) fn into_parts(self) -> (RunningStepReplacement<'buffers>, usize, usize) {
        (
            self.replacement,
            self.external_replacements,
            self.removed_dead_external_leases,
        )
    }
}

/// A successful complete nonterminal authority publication.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningStepOutcome<'workspace> {
    /// Exact key, completed-step identity, and admitted memory result.
    pub publication: RunningStepPublication,
    /// Retained work/capacity diagnostics captured before the swap.
    pub diagnostics: &'workspace WorldStepDiagnostics,
}

/// One pre-movement external observation whose step has otherwise staged fully.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalDeliveryEventKind {
    /// One pre-movement Protocol 2 sensor observation.
    Observation,
    /// One fresh controlled-death assignment carrying a new browser-visible ID.
    ReplacementAssignment { frame_v1_id: u32 },
}

/// One reliable external event whose step has otherwise staged fully.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalObservationEvent {
    /// Exact full step identity that produced this observation.
    pub step_key: PhysicsStepKey,
    /// Monotonic event identity within this coordinator instance.
    pub event_sequence: u64,
    /// Exact live socket epoch that may accept the event.
    pub connection_id: u64,
    /// Exact controller assignment epoch that owns the event.
    pub lease_id: u64,
    /// Browser player or separate Protocol 2 RL controller.
    pub controller_kind: ControllerKind,
    /// Observation or controlled-death replacement assignment.
    pub delivery_kind: ExternalDeliveryEventKind,
    /// Stable internal snake identity sampled.
    pub snake_id: u64,
    /// Stable pre-movement head position accompanying the observation.
    pub position: WorldPoint,
    /// Stable pre-movement heading accompanying the observation.
    pub direction: f64,
    observation_start: usize,
    observation_len: usize,
    token_index: Option<usize>,
}

/// Borrowed bridge batch retained until every exact local send result resolves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalObservationBatch<'workspace> {
    events: &'workspace [ExternalObservationEvent],
    observations: &'workspace [f32],
    tokens: &'workspace [String],
    statuses: &'workspace [ExternalDeliveryStatus],
}

impl<'workspace> ExternalObservationBatch<'workspace> {
    /// Canonically ordered event metadata.
    #[must_use]
    pub const fn events(self) -> &'workspace [ExternalObservationEvent] {
        self.events
    }

    /// Packed observation for one event without an additional copy.
    #[must_use]
    pub fn observation(self, event_index: usize) -> Option<&'workspace [f32]> {
        let event = self.events.get(event_index)?;
        if event.delivery_kind != ExternalDeliveryEventKind::Observation {
            return None;
        }
        let end = event.observation_start.checked_add(event.observation_len)?;
        self.observations.get(event.observation_start..end)
    }

    /// Opaque new resume token for one replacement assignment.
    #[must_use]
    pub fn resume_token(self, event_index: usize) -> Option<&'workspace str> {
        let event = self.events.get(event_index)?;
        let token_index = event.token_index?;
        self.tokens.get(token_index).map(String::as_str)
    }

    /// Whether a matching accepted result has already arrived for one event.
    #[must_use]
    pub fn is_accepted(self, event_index: usize) -> Option<bool> {
        self.statuses
            .get(event_index)
            .map(|status| *status == ExternalDeliveryStatus::Accepted)
    }

    /// Local send status already recorded for one event.
    #[must_use]
    pub fn status(self, event_index: usize) -> Option<ExternalDeliveryStatus> {
        self.statuses.get(event_index).copied()
    }

    /// Number of exact send results still required before publication.
    #[must_use]
    pub fn remaining(self) -> usize {
        self.statuses
            .iter()
            .filter(|status| **status == ExternalDeliveryStatus::Pending)
            .count()
    }
}

/// Local Node result for one attempted reliable external-observation send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalDeliveryResult {
    /// Exact step identity copied from the emitted event.
    pub step_key: PhysicsStepKey,
    /// Exact emitted event sequence.
    pub event_sequence: u64,
    /// Socket epoch to which Node attempted the send.
    pub connection_id: u64,
    /// Controller assignment epoch to which Node attempted the send.
    pub lease_id: u64,
    /// `true` only when Node accepted the event into the socket send path.
    pub accepted: bool,
}

/// One terminal fixed step retained for the persistence handoff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenerationTransitionBatch<'workspace> {
    source_key: PhysicsStepKey,
    reason: GenerationTransitionReason,
    elapsed_seconds: f64,
    alive_evolved: usize,
    candidate: &'workspace StateCandidate,
    metadata: PreparedGenerationMetadata,
    commit_record: &'workspace GenerationCommitRecord,
    combined_state_bytes: usize,
    memory_ceiling_bytes: usize,
    checkpoint_descriptor: Option<&'workspace CheckpointDescriptor>,
    persistence_acknowledged: bool,
}

impl<'workspace> GenerationTransitionBatch<'workspace> {
    /// Exact source operation that produced this terminal result.
    #[must_use]
    pub const fn source_key(self) -> PhysicsStepKey {
        self.source_key
    }

    /// Duration or early-alive-count rule that ended the generation.
    #[must_use]
    pub const fn reason(self) -> GenerationTransitionReason {
        self.reason
    }

    /// Simulated generation time after the terminal fixed delta.
    #[must_use]
    pub const fn elapsed_seconds(self) -> f64 {
        self.elapsed_seconds
    }

    /// Evolved snakes still alive in the immutable terminal physics result.
    #[must_use]
    pub const fn alive_evolved(self) -> usize {
        self.alive_evolved
    }

    /// Fully admitted zero-world, zero-recurrent-state checkpoint candidate.
    #[must_use]
    pub const fn candidate(self) -> &'workspace StateCandidate {
        self.candidate
    }

    /// Compact history and exact staged Hall-of-Fame reference.
    #[must_use]
    pub const fn metadata(self) -> PreparedGenerationMetadata {
        self.metadata
    }

    /// Exact summary and elite reference constructed by Rust admission.
    #[must_use]
    pub const fn commit_record(self) -> &'workspace GenerationCommitRecord {
        self.commit_record
    }

    /// Conservative current-plus-successor admission charge.
    #[must_use]
    pub const fn combined_state_bytes(self) -> usize {
        self.combined_state_bytes
    }

    /// Full process authority ceiling used by the dual-state admission.
    #[must_use]
    pub const fn memory_ceiling_bytes(self) -> usize {
        self.memory_ceiling_bytes
    }

    /// Immutable managed-file descriptor already published for this transition.
    #[must_use]
    pub const fn checkpoint_descriptor(self) -> Option<&'workspace CheckpointDescriptor> {
        self.checkpoint_descriptor
    }

    /// Whether the exact descriptor's SQLite metadata/current-pointer commit
    /// has been acknowledged.
    #[must_use]
    pub const fn persistence_acknowledged(self) -> bool {
        self.persistence_acknowledged
    }
}

/// Result of initially driving one complete fixed-step attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RunningStepProgress<'workspace> {
    /// The step required no external delivery and published immediately.
    Published(RunningStepOutcome<'workspace>),
    /// A complete staged step is waiting for matching local Node results.
    ExternalDeliveryPending(ExternalObservationBatch<'workspace>),
    /// A terminal result is waiting for its managed checkpoint/metadata barrier.
    GenerationTransitionPending(GenerationTransitionBatch<'workspace>),
}

/// State after applying zero or more Node delivery results.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExternalDeliveryState<'workspace> {
    /// No matching staged delivery remains; supplied results were stale.
    Idle,
    /// At least one exact event still awaits a result.
    Pending(ExternalObservationBatch<'workspace>),
    /// Every exact event resolved and the complete step published once.
    Published(RunningStepOutcome<'workspace>),
    /// Every reliable generation-start assignment resolved; the unchanged old
    /// authority and durable successor boundary are ready for final admission.
    GenerationAssignmentsReady(GenerationTransitionBatch<'workspace>),
}

/// Initial or retained progress while connected controllers receive their
/// fresh next-generation snakes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GenerationReassignmentProgress<'workspace> {
    /// At least one exact local assignment send still needs a result.
    DeliveryPending(ExternalObservationBatch<'workspace>),
    /// No assignments were required, or every exact result already resolved.
    Ready(GenerationTransitionBatch<'workspace>),
}

/// Accounting for accepted and ignored Node results.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalDeliveryResolution<'workspace> {
    /// Previously-unaccepted exact events accepted by this call.
    pub matched_acceptances: usize,
    /// Previously-unresolved exact events whose matching local send failed.
    pub matched_failures: usize,
    /// Stale, replaced, unknown, or duplicate results ignored.
    pub ignored_results: usize,
    /// Pending, idle, or newly published state after processing the results.
    pub state: ExternalDeliveryState<'workspace>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingExternalSource {
    Observation { retained_index: usize },
    ReplacementAssignment { assignment_index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingDeliveryContext {
    RunningStep,
    GenerationStart,
}

/// Prevalidated in-place coordinator rebind after one generation publication.
///
/// Construction is fallible while the published successor can still be
/// compared with every retained coordinator identity. Applying the token only
/// changes the process-local world epoch, preserving all reusable workspaces
/// and the monotonic external-event sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedCoordinatorGenerationRebind {
    source_world_epoch: u64,
    successor_world_epoch: u64,
}

/// Reason a completed physical step must enter the later generation transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationTransitionReason {
    /// Configured generation duration was reached.
    Duration,
    /// The evolved alive count reached the configured early-end rule.
    EarlyAliveCount,
}

#[derive(Debug)]
struct PendingGenerationTransition {
    reason: GenerationTransitionReason,
    elapsed_seconds: f64,
    alive_evolved: usize,
    boundary: AdmittedGenerationBoundary,
    checkpoint_descriptor: Option<CheckpointDescriptor>,
    persistence_acknowledged: bool,
    inputs: RunningStepInputs,
    running_fixed_step: Option<FixedStepContinuationState>,
    preflight: Option<GenerationStartPreflight>,
}

impl PendingGenerationTransition {
    fn batch(&self) -> GenerationTransitionBatch<'_> {
        GenerationTransitionBatch {
            source_key: self.boundary.source_key(),
            reason: self.reason,
            elapsed_seconds: self.elapsed_seconds,
            alive_evolved: self.alive_evolved,
            candidate: self.boundary.candidate(),
            metadata: self.boundary.metadata(),
            commit_record: self.boundary.commit_record(),
            combined_state_bytes: self.boundary.combined_state_bytes(),
            memory_ceiling_bytes: self.boundary.full_memory_ceiling_bytes(),
            checkpoint_descriptor: self.checkpoint_descriptor.as_ref(),
            persistence_acknowledged: self.persistence_acknowledged,
        }
    }
}

enum StagedStep {
    Nonterminal { surviving_observations: usize },
    Generation(Box<PendingGenerationTransition>),
}

/// Reusable single-owner coordinator for one admitted authority/config/graph.
#[derive(Debug)]
pub struct RunningStepCoordinator {
    projection: RunningStepConfigProjection,
    math_backend: InferenceMathBackend,
    world_epoch: u64,
    config_revision: u64,
    config_hash: String,
    graph_layout_digest: [u8; 32],
    work_limits: RunningStepWorkLimits,
    last_wall_now_ms: Option<u64>,
    prefix: FixedStepPrefixWorkspace,
    control: ControlPhaseWorkspace,
    control_commit: ControlCommitWorkspace,
    world_step: WorldStepWorkspace,
    generation_start: GenerationStartWorkspace,
    pending_generation: Option<PendingGenerationTransition>,
    pending_key: Option<PhysicsStepKey>,
    pending_inputs: Option<RunningStepInputs>,
    pending_preflight: Option<RunningStepPreflight>,
    pending_delivery_context: Option<PendingDeliveryContext>,
    last_published_diagnostics: WorldStepDiagnostics,
    pending_events: Vec<ExternalObservationEvent>,
    pending_statuses: Vec<ExternalDeliveryStatus>,
    pending_sources: Vec<PendingExternalSource>,
    pending_observations: Vec<PreparedExternalObservation>,
    pending_observation_statuses: Vec<ExternalDeliveryStatus>,
    pending_tokens: Vec<String>,
    pending_token_count: usize,
    next_external_event_sequence: u64,
    #[cfg(feature = "engine-test-hooks")]
    last_phase_timings: RunningStepPhaseTimings,
    #[cfg(feature = "engine-test-hooks")]
    allocation_snapshot: Option<fn() -> u64>,
    #[cfg(feature = "engine-test-hooks")]
    allocation_cursor: Option<u64>,
    #[cfg(feature = "engine-test-hooks")]
    last_phase_allocations: RunningStepPhaseAllocations,
}

impl RunningStepCoordinator {
    /// Build every persistent phase workspace from one admitted authority.
    ///
    /// A later config revision, graph replacement, Reset, New Run, or import
    /// requires a new coordinator. Hot steps never accept a caller-supplied
    /// graph, sensor layout, or gameplay configuration.
    pub fn try_new(
        authority: &AuthoritativeState,
        limits: RunningStepWorkLimits,
    ) -> Result<Self, RunningStepError> {
        let projection = authority.running_step_config(limits)?;
        let sensor = SensorEvaluator::new(projection.sensor.clone())?;
        let state = authority.state();
        let math_backend = InferenceMathBackend::from_label(&state.identity.math_backend)
            .ok_or_else(|| InferenceError::UnknownMathBackend {
                backend: state.identity.math_backend.clone(),
            })?;
        let inference =
            GraphExecutionPlan::build_with_math_backend(authority.graph(), math_backend)?;
        let neural = NeuralControlPipeline::try_new(
            state.config.max_world_snakes,
            sensor,
            inference,
            state.config.worker_scratch_bytes,
        )?;
        Ok(Self {
            projection,
            math_backend,
            world_epoch: authority.world_epoch(),
            config_revision: state.identity.config_revision,
            config_hash: state.identity.config_hash.clone(),
            graph_layout_digest: authority.graph().layout_digest_sha256,
            work_limits: limits,
            last_wall_now_ms: None,
            prefix: FixedStepPrefixWorkspace::new(),
            control: ControlPhaseWorkspace::new(neural),
            control_commit: ControlCommitWorkspace::new(),
            world_step: WorldStepWorkspace::new(),
            generation_start: GenerationStartWorkspace::new(),
            pending_generation: None,
            pending_key: None,
            pending_inputs: None,
            pending_preflight: None,
            pending_delivery_context: None,
            last_published_diagnostics: WorldStepDiagnostics::default(),
            pending_events: Vec::new(),
            pending_statuses: Vec::new(),
            pending_sources: Vec::new(),
            pending_observations: Vec::new(),
            pending_observation_statuses: Vec::new(),
            pending_tokens: Vec::new(),
            pending_token_count: 0,
            next_external_event_sequence: 1,
            #[cfg(feature = "engine-test-hooks")]
            last_phase_timings: RunningStepPhaseTimings::default(),
            #[cfg(feature = "engine-test-hooks")]
            allocation_snapshot: None,
            #[cfg(feature = "engine-test-hooks")]
            allocation_cursor: None,
            #[cfg(feature = "engine-test-hooks")]
            last_phase_allocations: RunningStepPhaseAllocations::default(),
        })
    }

    /// Exact numeric implementation bound by the admitted run identity.
    #[must_use]
    pub const fn math_backend(&self) -> InferenceMathBackend {
        self.math_backend
    }

    /// Stage one complete step and publish it immediately or await Node delivery.
    ///
    /// Any failure changes reusable scratch and the process-local attempt epoch,
    /// but not the authoritative [`StateCandidate`](super::state::StateCandidate).
    /// External observations become visible only after every physics substep and
    /// the nonterminal guard succeed. Their score markers and the complete world
    /// then publish together after matching local Node send acceptance.
    pub fn advance_nonterminal<'workspace>(
        &'workspace mut self,
        authority: &mut AuthoritativeState,
        inputs: RunningStepInputs,
    ) -> Result<RunningStepProgress<'workspace>, RunningStepError> {
        #[cfg(feature = "engine-test-hooks")]
        {
            self.last_phase_timings = RunningStepPhaseTimings::default();
            self.last_phase_allocations = RunningStepPhaseAllocations::default();
            self.allocation_cursor = self.allocation_snapshot.map(|snapshot| snapshot());
        }
        #[cfg(feature = "engine-test-hooks")]
        let authority_started = Instant::now();
        self.validate_authority(authority)?;
        if self.pending_key.is_some() {
            return Err(RunningStepError::ExternalDeliveryPending {
                count: self
                    .pending_statuses
                    .iter()
                    .filter(|status| **status == ExternalDeliveryStatus::Pending)
                    .count(),
            });
        }
        if self.pending_generation.is_some() {
            return Err(RunningStepError::GenerationTransitionPending);
        }
        if !inputs.wall_accumulator_seconds.is_finite() || inputs.wall_accumulator_seconds < 0.0 {
            return Err(RunningStepError::InvalidSchedulerAccumulator(
                inputs.wall_accumulator_seconds,
            ));
        }
        if self
            .last_wall_now_ms
            .is_some_and(|previous| inputs.wall_now_ms < previous)
        {
            return Err(RunningStepError::RegressingWallClock {
                previous_ms: self.last_wall_now_ms.unwrap_or_default(),
                actual_ms: inputs.wall_now_ms,
            });
        }

        let key = authority.begin_running_step()?;
        self.last_wall_now_ms = Some(inputs.wall_now_ms);
        #[cfg(feature = "engine-test-hooks")]
        {
            self.last_phase_timings.authority_begin_ms = elapsed_ms(authority_started);
            self.last_phase_allocations.authority_begin =
                allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
        }
        let staged = (|| -> Result<StagedStep, RunningStepError> {
            let state = authority.state();
            #[cfg(feature = "engine-test-hooks")]
            let prefix_started = Instant::now();
            let prefix = self.prefix.prepare(FixedStepPrefixInputs {
                key,
                world: &state.world,
                rng: &state.rng,
                allocators: &state.allocators,
                generation_elapsed_seconds: state.generation.elapsed_seconds,
                ambient_accumulator: state.fixed_step.ambient_pellet_accumulator,
                baseline_lifecycle: &state.fixed_step.baseline_lifecycle,
                config: self.projection.world_step.prefix,
            })?;
            #[cfg(feature = "engine-test-hooks")]
            {
                self.last_phase_timings.prefix_ms = elapsed_ms(prefix_started);
                self.last_phase_allocations.prefix =
                    allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
            }
            #[cfg(feature = "engine-test-hooks")]
            let control_started = Instant::now();
            let selected = self.control.prepare(ControlPhaseInputs {
                prefix,
                generation: &state.fixed_step.sensor_generation,
                population: &state.population,
                brains: &state.brains,
                wall_now_ms: inputs.wall_now_ms,
                config: self.projection.world_step.control,
            })?;
            #[cfg(feature = "engine-test-hooks")]
            {
                self.last_phase_timings.control_selection_ms = elapsed_ms(control_started);
                self.last_phase_allocations.control_selection =
                    allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
            }
            #[cfg(feature = "engine-test-hooks")]
            let commit_started = Instant::now();
            let committed = self.control_commit.prepare(selected)?;
            #[cfg(feature = "engine-test-hooks")]
            {
                self.last_phase_timings.control_commit_ms = elapsed_ms(commit_started);
                self.last_phase_allocations.control_commit =
                    allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
            }
            #[cfg(feature = "engine-test-hooks")]
            let world_step_started = Instant::now();
            let prepared = self
                .world_step
                .prepare_deferred_external_replacement(committed, self.projection.world_step)?;
            #[cfg(feature = "engine-test-hooks")]
            {
                self.last_phase_timings.world_step_ms = elapsed_ms(world_step_started);
                self.last_phase_allocations.world_step =
                    allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
            }
            #[cfg(feature = "engine-test-hooks")]
            let guard_started = Instant::now();
            if let Some((reason, alive_evolved)) = generation_transition_required(
                prepared.world(),
                prepared.generation_elapsed_seconds(),
                self.projection.generation_guard,
            ) {
                #[cfg(feature = "engine-test-hooks")]
                {
                    self.last_phase_timings.generation_guard_ms = elapsed_ms(guard_started);
                    self.last_phase_allocations.generation_guard =
                        allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
                }
                let elapsed_seconds = prepared.generation_elapsed_seconds();
                let next = prepare_generation_boundary(
                    state,
                    prepared.world(),
                    prepared.rng(),
                    prepared.allocators(),
                    elapsed_seconds,
                    authority.graph(),
                )?;
                let boundary = admit_prepared_generation_boundary(authority, key, next)?;
                return Ok(StagedStep::Generation(Box::new(
                    PendingGenerationTransition {
                        reason,
                        elapsed_seconds,
                        alive_evolved,
                        boundary,
                        checkpoint_descriptor: None,
                        persistence_acknowledged: false,
                        inputs,
                        running_fixed_step: None,
                        preflight: None,
                    },
                )));
            }
            #[cfg(feature = "engine-test-hooks")]
            {
                self.last_phase_timings.generation_guard_ms = elapsed_ms(guard_started);
                self.last_phase_allocations.generation_guard =
                    allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
            }
            #[cfg(feature = "engine-test-hooks")]
            let replacement_started = Instant::now();
            let prepared = self.world_step.complete_deferred_external_replacement(
                committed,
                self.projection.world_step,
                authority.graph(),
                inputs.wall_now_ms,
            )?;
            #[cfg(feature = "engine-test-hooks")]
            {
                self.last_phase_timings.world_step_ms += elapsed_ms(replacement_started);
                self.last_phase_allocations.world_step = self
                    .last_phase_allocations
                    .world_step
                    .saturating_add(allocation_delta(
                        self.allocation_snapshot,
                        &mut self.allocation_cursor,
                    ));
            }
            let surviving_observations = prepared
                .external_events()
                .iter()
                .filter(|event| {
                    prepared
                        .world()
                        .snakes
                        .iter()
                        .any(|snake| snake.alive && snake.id == event.snake_id())
                })
                .count();
            Ok(StagedStep::Nonterminal {
                surviving_observations,
            })
        })();
        let surviving_observations = match staged {
            Ok(StagedStep::Nonterminal {
                surviving_observations,
            }) => surviving_observations,
            Ok(StagedStep::Generation(pending)) => {
                self.pending_generation = Some(*pending);
                self.world_step.invalidate_publication();
                self.control_commit.invalidate_publication();
                self.clear_pending_metadata();
                let batch = self
                    .pending_generation
                    .as_ref()
                    .expect("just-staged generation transition must remain present")
                    .batch();
                return Ok(RunningStepProgress::GenerationTransitionPending(batch));
            }
            Err(error) => {
                self.discard_staged();
                return Err(error);
            }
        };
        let diagnostics = self.world_step.diagnostics();
        let Some(external_count) =
            surviving_observations.checked_add(diagnostics.external_replacement.replacements)
        else {
            self.discard_staged();
            return Err(RunningStepError::ArithmeticOverflow {
                context: "combined external delivery count",
            });
        };

        if external_count != 0 {
            if let Err(error) = self.retain_external_delivery(key, inputs) {
                self.discard_staged();
                return Err(error);
            }
            if let Err(error) = self.world_step.preflight_external_deliveries(
                key,
                &self.pending_observations,
                inputs.wall_now_ms,
                self.projection.world_step.control.controller_timing,
            ) {
                self.discard_staged();
                return Err(error.into());
            }
            let preflight = match self.preflight_staged(authority, key, inputs) {
                Ok(preflight) => preflight,
                Err(error) => {
                    self.discard_staged();
                    return Err(error);
                }
            };
            self.pending_preflight = Some(preflight);
            if let Err(error) = self.validate_pending_publication(authority, key) {
                self.discard_staged();
                return Err(error);
            }
            return Ok(RunningStepProgress::ExternalDeliveryPending(
                self.pending_batch_prevalidated(),
            ));
        }

        Ok(RunningStepProgress::Published(self.publish_staged(
            authority,
            key,
            inputs,
            diagnostics,
        )?))
    }

    /// Apply local Node send results to the one retained external event batch.
    ///
    /// Wrong-world, wrong-step, wrong-connection, wrong-assignment, unknown,
    /// and duplicate results are ignored. An exact accepted result advances
    /// only its prevalidated score marker. An exact failed result leaves that
    /// marker accumulated and applies the prevalidated disconnect transition.
    /// Once every event is resolved, those choices and the complete physical
    /// step publish together through an infallible private swap.
    pub fn submit_external_delivery_results<'workspace>(
        &'workspace mut self,
        authority: &mut AuthoritativeState,
        results: &[ExternalDeliveryResult],
    ) -> Result<ExternalDeliveryResolution<'workspace>, RunningStepError> {
        self.validate_authority(authority)?;
        let Some(key) = self.pending_key else {
            return Ok(ExternalDeliveryResolution {
                matched_acceptances: 0,
                matched_failures: 0,
                ignored_results: results.len(),
                state: ExternalDeliveryState::Idle,
            });
        };
        let context = self.pending_delivery_context.ok_or(
            RunningStepError::PendingDeliveryStateMismatch {
                field: "delivery context",
            },
        )?;

        if authority.validate_running_step_key(key).is_err() {
            self.discard_staged();
            return Ok(ExternalDeliveryResolution {
                matched_acceptances: 0,
                matched_failures: 0,
                ignored_results: results.len(),
                state: ExternalDeliveryState::Idle,
            });
        }

        let mut matched_acceptances = 0usize;
        let mut matched_failures = 0usize;
        let mut ignored_results = 0usize;
        for result in results {
            let Ok(index) = self
                .pending_events
                .binary_search_by_key(&result.event_sequence, |event| event.event_sequence)
            else {
                ignored_results = ignored_results.saturating_add(1);
                continue;
            };
            let event = self.pending_events[index];
            if result.step_key != key
                || event.step_key != key
                || result.connection_id != event.connection_id
                || result.lease_id != event.lease_id
            {
                ignored_results = ignored_results.saturating_add(1);
                continue;
            }
            if self.pending_statuses[index] != ExternalDeliveryStatus::Pending {
                ignored_results = ignored_results.saturating_add(1);
            } else {
                let next_status = if result.accepted {
                    ExternalDeliveryStatus::Accepted
                } else {
                    ExternalDeliveryStatus::Failed
                };
                match self.pending_sources[index] {
                    PendingExternalSource::Observation { retained_index } => {
                        if context != PendingDeliveryContext::RunningStep {
                            return Err(RunningStepError::PendingDeliveryStateMismatch {
                                field: "generation observation result",
                            });
                        }
                        let status = self
                            .pending_observation_statuses
                            .get_mut(retained_index)
                            .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                                field: "observation result index",
                            })?;
                        if *status != ExternalDeliveryStatus::Pending {
                            return Err(RunningStepError::PendingDeliveryStateMismatch {
                                field: "observation result state",
                            });
                        }
                        *status = next_status;
                    }
                    PendingExternalSource::ReplacementAssignment { .. } => {
                        let resolution = self.world_step.resolve_replacement_assignment(
                            key,
                            event.lease_id,
                            event.connection_id,
                            result.accepted,
                        )?;
                        let expected = if result.accepted {
                            AssignmentResolution::Accepted
                        } else {
                            AssignmentResolution::Failed
                        };
                        if resolution != expected {
                            return Err(RunningStepError::PendingDeliveryStateMismatch {
                                field: "replacement result state",
                            });
                        }
                    }
                }
                self.pending_statuses[index] = next_status;
                if result.accepted {
                    matched_acceptances = matched_acceptances.saturating_add(1);
                } else {
                    matched_failures = matched_failures.saturating_add(1);
                }
            }
        }

        if self
            .pending_statuses
            .contains(&ExternalDeliveryStatus::Pending)
        {
            return Ok(ExternalDeliveryResolution {
                matched_acceptances,
                matched_failures,
                ignored_results,
                state: ExternalDeliveryState::Pending(self.pending_batch_prevalidated()),
            });
        }

        if context == PendingDeliveryContext::GenerationStart {
            self.validate_pending_batch(key)?;
            {
                let _buffers = self.world_step.generation_reassignment_buffers(key)?;
            }
            let transition = self
                .pending_generation
                .as_ref()
                .ok_or(RunningStepError::GenerationTransitionNotPending)?
                .batch();
            return Ok(ExternalDeliveryResolution {
                matched_acceptances,
                matched_failures,
                ignored_results,
                state: ExternalDeliveryState::GenerationAssignmentsReady(transition),
            });
        }

        let inputs = self
            .pending_inputs
            .expect("prevalidated pending delivery must retain step inputs");
        let preflight = self
            .pending_preflight
            .expect("prevalidated pending delivery must retain state proof");
        self.world_step.commit_prevalidated_external_deliveries(
            key,
            &self.pending_observations,
            &self.pending_observation_statuses,
        );
        let diagnostics = self.world_step.diagnostics();
        let outcome =
            self.publish_prevalidated_staged(authority, key, inputs, preflight, diagnostics)?;
        Ok(ExternalDeliveryResolution {
            matched_acceptances,
            matched_failures,
            ignored_results,
            state: ExternalDeliveryState::Published(outcome),
        })
    }

    /// Last accepted wall-clock boundary, including a failed staged attempt.
    #[must_use]
    pub const fn last_wall_now_ms(&self) -> Option<u64> {
        self.last_wall_now_ms
    }

    /// Reborrow the exact unresolved reliable-delivery batch without mutation.
    #[must_use]
    pub fn pending_external_delivery(&self) -> Option<ExternalObservationBatch<'_>> {
        self.pending_key?;
        Some(self.pending_batch_prevalidated())
    }

    /// Inspect the retained terminal boundary without rerunning evolution.
    #[must_use]
    pub fn pending_generation_transition(&self) -> Option<GenerationTransitionBatch<'_>> {
        self.pending_generation
            .as_ref()
            .map(PendingGenerationTransition::batch)
    }

    /// Token-scoped old-controller outcomes that the lifecycle bridge must
    /// retain before publishing the replacement world.
    pub fn pending_unavailable_controller_reservations(
        &self,
    ) -> Result<&[UnavailableControllerReservation], RunningStepError> {
        let Some(key) = self.pending_key else {
            return Ok(&[]);
        };
        Ok(self.world_step.unavailable_controller_reservations(key)?)
    }

    /// Publish the pending immutable managed checkpoint without changing authority.
    ///
    /// The eventual Rust coordinator/persistence thread calls this off the Node
    /// event loop, then commits small SQLite metadata before acknowledging the
    /// exact source key. A failed call leaves the same admitted transition ready
    /// for an explicit retry.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_pending_generation_checkpoint(
        &mut self,
        authority: &AuthoritativeState,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
        limits: &CheckpointLimits,
        graph_limits: &GraphLimits,
    ) -> Result<CheckpointDescriptor, RunningStepError> {
        self.validate_authority(authority)?;
        let pending = self
            .pending_generation
            .as_mut()
            .ok_or(RunningStepError::GenerationTransitionNotPending)?;
        if let Some(descriptor) = &pending.checkpoint_descriptor {
            if descriptor.operation_id == operation_id {
                return Ok(descriptor.clone());
            }
            return Err(RunningStepError::GenerationCheckpointAlreadyPublished {
                operation_id: descriptor.operation_id.as_str().to_owned(),
            });
        }
        let descriptor = pending.boundary.publish_managed_checkpoint(
            authority,
            managed_directory,
            operation_id,
            limits,
            graph_limits,
        )?;
        pending.checkpoint_descriptor = Some(descriptor.clone());
        Ok(descriptor)
    }

    /// Accept one exact successful SQLite metadata/current-pointer commit and
    /// only then construct the collision-safe next world.
    ///
    /// The persistence worker must echo the complete immutable descriptor it
    /// committed. A mismatched or premature acknowledgement changes nothing.
    /// Successful acknowledgement is retained even if generation construction
    /// fails, because SQLite has already made the checkpoint boundary current;
    /// an explicit retry then uses the unchanged boundary and deterministic
    /// streams, never a newly evolved population.
    pub fn acknowledge_pending_generation_persistence<'workspace>(
        &'workspace mut self,
        authority: &AuthoritativeState,
        committed: &CheckpointDescriptor,
    ) -> Result<PreparedGenerationStart<'workspace, 'workspace>, RunningStepError> {
        self.validate_authority(authority)?;
        let config = GenerationStartConfig::from_work_limits(self.work_limits);
        let (pending_generation, generation_start) =
            (&mut self.pending_generation, &mut self.generation_start);
        let pending = pending_generation
            .as_mut()
            .ok_or(RunningStepError::GenerationTransitionNotPending)?;
        authority.validate_running_step_key(pending.boundary.source_key())?;
        let expected = pending
            .checkpoint_descriptor
            .as_ref()
            .ok_or(RunningStepError::GenerationCheckpointNotPublished)?;
        if let Some(field) = expected.first_mismatch(committed) {
            return Err(RunningStepError::GenerationPersistenceAcknowledgementMismatch { field });
        }
        pending.persistence_acknowledged = true;
        let source = pending.boundary.candidate();
        if generation_start.retains(source, config) {
            return Ok(generation_start.retained(source, config)?);
        }
        Ok(generation_start.prepare(source, config)?)
    }

    /// Retry or reborrow next-world construction after an already accepted
    /// persistence acknowledgement without repeating SQLite work.
    pub fn prepare_acknowledged_generation_start<'workspace>(
        &'workspace mut self,
        authority: &AuthoritativeState,
    ) -> Result<PreparedGenerationStart<'workspace, 'workspace>, RunningStepError> {
        self.validate_authority(authority)?;
        let config = GenerationStartConfig::from_work_limits(self.work_limits);
        let (pending_generation, generation_start) =
            (&mut self.pending_generation, &mut self.generation_start);
        let pending = pending_generation
            .as_ref()
            .ok_or(RunningStepError::GenerationTransitionNotPending)?;
        authority.validate_running_step_key(pending.boundary.source_key())?;
        if !pending.persistence_acknowledged {
            return Err(RunningStepError::GenerationPersistenceNotAcknowledged);
        }
        let source = pending.boundary.candidate();
        if generation_start.retains(source, config) {
            return Ok(generation_start.retained(source, config)?);
        }
        Ok(generation_start.prepare(source, config)?)
    }

    /// Stage or reborrow the reliable fresh-snake assignments for controllers
    /// that are still connected after the durable boundary was acknowledged.
    ///
    /// The current authority remains the preceding world throughout this
    /// operation. The collision-safe generation base is deterministic and
    /// retained; fresh resume tokens are generated only once and every
    /// connected browser/RL owner must report an exact local-send result before
    /// the successor can be admitted.
    pub fn prepare_acknowledged_generation_reassignments<'workspace>(
        &'workspace mut self,
        authority: &AuthoritativeState,
    ) -> Result<GenerationReassignmentProgress<'workspace>, RunningStepError> {
        self.validate_authority(authority)?;
        let pending = self
            .pending_generation
            .as_ref()
            .ok_or(RunningStepError::GenerationTransitionNotPending)?;
        let key = pending.boundary.source_key();
        authority.validate_running_step_key(key)?;
        if !pending.persistence_acknowledged {
            return Err(RunningStepError::GenerationPersistenceNotAcknowledged);
        }
        if let Some(context) = self.pending_delivery_context {
            if context != PendingDeliveryContext::GenerationStart {
                return Err(RunningStepError::PendingDeliveryStateMismatch {
                    field: "generation delivery context",
                });
            }
            self.validate_pending_batch(key)?;
            if self
                .pending_statuses
                .contains(&ExternalDeliveryStatus::Pending)
            {
                return Ok(GenerationReassignmentProgress::DeliveryPending(
                    self.pending_batch_prevalidated(),
                ));
            }
            return Ok(GenerationReassignmentProgress::Ready(
                self.pending_generation
                    .as_ref()
                    .expect("validated generation transition must remain present")
                    .batch(),
            ));
        }

        let connected_count = authority
            .state()
            .world
            .controller_leases
            .iter()
            .filter(|lease| {
                lease.status == ControllerLeaseStatus::Connected && lease.connection_id.is_some()
            })
            .count();
        let generation_config = GenerationStartConfig::from_work_limits(self.work_limits);
        let full_replacement_config = self.projection.world_step.external_replacement;
        let inputs = pending.inputs;
        let (start_diagnostics, initial_fixed_step) = {
            let (pending_generation, generation_start, world_step) = (
                &self.pending_generation,
                &mut self.generation_start,
                &mut self.world_step,
            );
            let pending = pending_generation
                .as_ref()
                .ok_or(RunningStepError::GenerationTransitionNotPending)?;
            let source = pending.boundary.candidate();
            let prepared = if generation_start.retains(source, generation_config) {
                generation_start.retained(source, generation_config)?
            } else {
                generation_start.prepare(source, generation_config)?
            };
            let start_diagnostics = prepared.diagnostics();
            let initial_fixed_step = prepared.fixed_step().clone();
            let mut replacement_config = full_replacement_config;
            let remaining_candidates = full_replacement_config
                .spawn
                .maximum_candidates_per_batch
                .checked_sub(start_diagnostics.candidates_examined)
                .ok_or(RunningStepError::GenerationWorkBudgetExceeded {
                    work: "spawn candidates",
                    used: start_diagnostics.candidates_examined,
                    maximum: full_replacement_config.spawn.maximum_candidates_per_batch,
                })?;
            let remaining_geometry = full_replacement_config
                .spawn
                .maximum_geometry_checks_per_batch
                .checked_sub(start_diagnostics.geometry_checks)
                .ok_or(RunningStepError::GenerationWorkBudgetExceeded {
                    work: "spawn geometry checks",
                    used: start_diagnostics.geometry_checks,
                    maximum: full_replacement_config
                        .spawn
                        .maximum_geometry_checks_per_batch,
                })?;
            if connected_count != 0 && (remaining_candidates == 0 || remaining_geometry == 0) {
                return Err(RunningStepError::GenerationWorkBudgetExceeded {
                    work: if remaining_candidates == 0 {
                        "spawn candidates"
                    } else {
                        "spawn geometry checks"
                    },
                    used: if remaining_candidates == 0 {
                        start_diagnostics.candidates_examined
                    } else {
                        start_diagnostics.geometry_checks
                    },
                    maximum: if remaining_candidates == 0 {
                        full_replacement_config.spawn.maximum_candidates_per_batch
                    } else {
                        full_replacement_config
                            .spawn
                            .maximum_geometry_checks_per_batch
                    },
                });
            }
            replacement_config.spawn.maximum_candidates_per_batch = remaining_candidates.max(1);
            replacement_config.spawn.maximum_geometry_checks_per_batch = remaining_geometry.max(1);
            world_step.invalidate_publication();
            world_step.prepare_generation_reassignments(
                key,
                prepared.world(),
                prepared.rng(),
                prepared.allocators(),
                &source.brains,
                &authority.state().world,
                authority.graph(),
                source.generation.population_epoch,
                inputs.wall_now_ms,
                replacement_config,
            )?;
            (start_diagnostics, initial_fixed_step)
        };
        let replacement_diagnostics = self.world_step.diagnostics().external_replacement;
        let total_candidates = start_diagnostics
            .candidates_examined
            .checked_add(replacement_diagnostics.candidates_examined)
            .ok_or(RunningStepError::ArithmeticOverflow {
                context: "complete generation spawn candidates",
            })?;
        let total_geometry = start_diagnostics
            .geometry_checks
            .checked_add(replacement_diagnostics.geometry_checks)
            .ok_or(RunningStepError::ArithmeticOverflow {
                context: "complete generation spawn geometry checks",
            })?;
        if total_candidates > full_replacement_config.spawn.maximum_candidates_per_batch
            || total_geometry
                > full_replacement_config
                    .spawn
                    .maximum_geometry_checks_per_batch
        {
            self.world_step.invalidate_publication();
            return Err(RunningStepError::GenerationWorkBudgetExceeded {
                work: if total_candidates
                    > full_replacement_config.spawn.maximum_candidates_per_batch
                {
                    "spawn candidates"
                } else {
                    "spawn geometry checks"
                },
                used: if total_candidates
                    > full_replacement_config.spawn.maximum_candidates_per_batch
                {
                    total_candidates
                } else {
                    total_geometry
                },
                maximum: if total_candidates
                    > full_replacement_config.spawn.maximum_candidates_per_batch
                {
                    full_replacement_config.spawn.maximum_candidates_per_batch
                } else {
                    full_replacement_config
                        .spawn
                        .maximum_geometry_checks_per_batch
                },
            });
        }

        let preflight_result = {
            let (pending_generation, world_step) =
                (&mut self.pending_generation, &mut self.world_step);
            let pending = pending_generation
                .as_mut()
                .ok_or(RunningStepError::GenerationTransitionNotPending)?;
            match &pending.running_fixed_step {
                Some(retained) if retained != &initial_fixed_step => {
                    return Err(RunningStepError::PendingDeliveryStateMismatch {
                        field: "retained generation fixed-step state",
                    });
                }
                Some(_) => {}
                None => pending.running_fixed_step = Some(initial_fixed_step),
            }
            let unavailable_controller_reservations = world_step
                .unavailable_controller_reservations(key)?
                .to_vec();
            let buffers = world_step.generation_reassignment_validation_buffers(key)?;
            let mut replacement = GenerationStartReplacement {
                key,
                world: buffers.world,
                rng: buffers.rng,
                allocators: buffers.allocators,
                brains: buffers.brains,
                fixed_step: pending.running_fixed_step.as_mut().ok_or(
                    RunningStepError::PendingDeliveryStateMismatch {
                        field: "generation fixed-step state",
                    },
                )?,
                wall_accumulator_seconds: inputs.wall_accumulator_seconds,
                proof: buffers.proof,
            };
            pending.boundary.preflight_running_start(
                authority,
                &mut replacement,
                &unavailable_controller_reservations,
            )
        };
        let preflight = match preflight_result {
            Ok(preflight) => preflight,
            Err(error) => {
                self.world_step.invalidate_publication();
                return Err(error.into());
            }
        };
        self.pending_generation
            .as_mut()
            .ok_or(RunningStepError::GenerationTransitionNotPending)?
            .preflight = Some(preflight);

        if let Err(error) = self.retain_generation_assignment_delivery(key, inputs) {
            self.world_step.invalidate_publication();
            self.clear_pending_metadata();
            if let Some(pending) = self.pending_generation.as_mut() {
                pending.preflight = None;
            }
            return Err(error);
        }
        if let Err(error) = self.validate_pending_batch(key) {
            self.world_step.invalidate_publication();
            self.clear_pending_metadata();
            if let Some(pending) = self.pending_generation.as_mut() {
                pending.preflight = None;
            }
            return Err(error);
        }
        if self.pending_events.is_empty() {
            self.world_step.generation_reassignment_buffers(key)?;
            return Ok(GenerationReassignmentProgress::Ready(
                self.pending_generation
                    .as_ref()
                    .expect("generation transition must remain present")
                    .batch(),
            ));
        }
        Ok(GenerationReassignmentProgress::DeliveryPending(
            self.pending_batch_prevalidated(),
        ))
    }

    /// Publish the exact preflighted running successor after persistence and
    /// every connected-controller assignment have resolved.
    ///
    /// Success consumes the retained terminal boundary and leaves this
    /// coordinator ready for an in-place world-epoch rebind. The caller must
    /// validate and commit that rebind before attempting another fixed step.
    pub fn publish_acknowledged_generation_start(
        &mut self,
        authority: &mut AuthoritativeState,
    ) -> Result<GenerationStartPublication, RunningStepError> {
        self.validate_authority(authority)?;
        let pending = self
            .pending_generation
            .as_ref()
            .ok_or(RunningStepError::GenerationTransitionNotPending)?;
        let key = pending.boundary.source_key();
        authority.validate_running_step_key(key)?;
        if !pending.persistence_acknowledged {
            return Err(RunningStepError::GenerationPersistenceNotAcknowledged);
        }
        if self.pending_delivery_context != Some(PendingDeliveryContext::GenerationStart) {
            return Err(RunningStepError::PendingDeliveryStateMismatch {
                field: "generation delivery context",
            });
        }
        self.validate_pending_batch(key)?;
        let unresolved = self
            .pending_statuses
            .iter()
            .filter(|status| **status == ExternalDeliveryStatus::Pending)
            .count();
        if unresolved != 0 {
            return Err(RunningStepError::ExternalDeliveryPending { count: unresolved });
        }
        let unavailable_controller_reservations = self
            .world_step
            .unavailable_controller_reservations(key)?
            .to_vec();

        let publication = {
            let (pending_generation, world_step) =
                (&mut self.pending_generation, &mut self.world_step);
            let pending = pending_generation
                .as_mut()
                .ok_or(RunningStepError::GenerationTransitionNotPending)?;
            let preflight =
                pending
                    .preflight
                    .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                        field: "generation state preflight",
                    })?;
            let buffers = world_step.generation_reassignment_buffers(key)?;
            let replacement = GenerationStartReplacement {
                key,
                world: buffers.world,
                rng: buffers.rng,
                allocators: buffers.allocators,
                brains: buffers.brains,
                fixed_step: pending.running_fixed_step.as_mut().ok_or(
                    RunningStepError::PendingDeliveryStateMismatch {
                        field: "generation fixed-step state",
                    },
                )?,
                wall_accumulator_seconds: pending.inputs.wall_accumulator_seconds,
                proof: buffers.proof,
            };
            let resolved = ResolvedGenerationStartReplacement::new(replacement);
            pending.boundary.publish_running_start(
                authority,
                preflight,
                resolved,
                unavailable_controller_reservations,
            )?
        };

        self.pending_generation = None;
        self.world_step.invalidate_publication();
        self.control_commit.invalidate_publication();
        self.clear_pending_metadata();
        Ok(publication)
    }

    /// Validate that one completed generation publication can reuse this
    /// coordinator without reallocating its large persistent workspaces.
    ///
    /// This runs only after the authority swap, while the caller still retains
    /// the exact publication record. It does not mutate the coordinator; the
    /// resulting private token is committed only after the scheduler retires
    /// the same terminal ticket.
    pub(crate) fn prepare_published_generation_rebind(
        &self,
        authority: &AuthoritativeState,
        publication: &GenerationStartPublication,
    ) -> Result<PreparedCoordinatorGenerationRebind, RunningStepError> {
        if self.pending_generation.is_some()
            || self.pending_key.is_some()
            || self.pending_inputs.is_some()
            || self.pending_preflight.is_some()
            || self.pending_delivery_context.is_some()
            || !self.pending_events.is_empty()
            || !self.pending_statuses.is_empty()
            || !self.pending_sources.is_empty()
            || !self.pending_observations.is_empty()
            || !self.pending_observation_statuses.is_empty()
            || self.pending_token_count != 0
        {
            return Err(RunningStepError::PendingDeliveryStateMismatch {
                field: "generation coordinator rebind state",
            });
        }

        let state = authority.state();
        if publication.source_key.world_epoch() != self.world_epoch
            || publication.source_key.config_revision() != self.config_revision
        {
            return Err(RunningStepError::AuthorityMismatch {
                field: "generation rebind source identity",
            });
        }
        if authority.world_epoch() != publication.world_epoch
            || state.phase != AuthorityPhase::Running
            || state.generation.generation != publication.generation
            || state.generation.completed_step != publication.completed_step
            || state.generation.population_epoch != publication.population_epoch
            || authority.memory_estimate() != publication.memory
        {
            return Err(RunningStepError::AuthorityMismatch {
                field: "generation rebind successor identity",
            });
        }
        if state.identity.config_revision != self.config_revision
            || state.identity.config_hash != self.config_hash
            || authority.graph().layout_digest_sha256 != self.graph_layout_digest
            || state.identity.math_backend != self.math_backend.label()
        {
            return Err(RunningStepError::AuthorityMismatch {
                field: "generation rebind immutable contract",
            });
        }

        Ok(PreparedCoordinatorGenerationRebind {
            source_world_epoch: self.world_epoch,
            successor_world_epoch: publication.world_epoch,
        })
    }

    /// Apply one already-validated coordinator rebind without allocation or a
    /// second fallible authority operation.
    pub(crate) fn commit_published_generation_rebind(
        &mut self,
        prepared: PreparedCoordinatorGenerationRebind,
    ) {
        debug_assert_eq!(self.world_epoch, prepared.source_world_epoch);
        self.world_epoch = prepared.successor_world_epoch;
    }

    /// Explicitly discard one exact pending transition without changing authority.
    ///
    /// A persistence retry should keep the transition instead. This escape hatch
    /// exists for a reviewed failure path that also rejects the scheduler ticket;
    /// a wrong or stale key cannot discard a different transition.
    pub fn discard_pending_generation_transition(
        &mut self,
        authority: &AuthoritativeState,
        source_key: PhysicsStepKey,
    ) -> Result<bool, RunningStepError> {
        self.validate_authority(authority)?;
        if self
            .pending_generation
            .as_ref()
            .is_none_or(|pending| pending.boundary.source_key() != source_key)
        {
            return Ok(false);
        }
        if self
            .pending_generation
            .as_ref()
            .is_some_and(|pending| pending.persistence_acknowledged)
        {
            return Err(RunningStepError::GenerationPersistenceAlreadyCommitted);
        }
        self.pending_generation = None;
        Ok(true)
    }

    /// Current bridge counts and retained capacities without changing state.
    #[must_use]
    pub fn external_delivery_diagnostics(&self) -> ExternalDeliveryDiagnostics {
        ExternalDeliveryDiagnostics {
            pending_events: self.pending_events.len(),
            remaining_events: self
                .pending_statuses
                .iter()
                .filter(|status| **status == ExternalDeliveryStatus::Pending)
                .count(),
            event_capacity: self.pending_events.capacity(),
            acceptance_capacity: self.pending_statuses.capacity(),
            disconnect_capacity: self.world_step.diagnostics().external_disconnect_capacity,
            observation_capacity: self
                .control_commit
                .diagnostics()
                .external_observation_capacity,
            observation_event_capacity: self.pending_observations.capacity(),
            source_capacity: self.pending_sources.capacity(),
            token_capacity: self.pending_tokens.capacity(),
        }
    }

    fn retain_external_delivery(
        &mut self,
        key: PhysicsStepKey,
        inputs: RunningStepInputs,
    ) -> Result<(), RunningStepError> {
        let (source_events, observations) = self.control_commit.external_delivery_buffers(key)?;
        let final_world = self.world_step.staged_world(key)?;
        let surviving_observations = source_events
            .iter()
            .filter(|event| {
                final_world
                    .snakes
                    .iter()
                    .any(|snake| snake.alive && snake.id == event.snake_id())
            })
            .count();
        let replacement_count = self.world_step.replacement_assignments(key)?.len();
        let total_count = surviving_observations
            .checked_add(replacement_count)
            .ok_or(RunningStepError::ArithmeticOverflow {
                context: "retained external delivery count",
            })?;
        let count_u64 =
            u64::try_from(total_count).map_err(|_| RunningStepError::ArithmeticOverflow {
                context: "external event count",
            })?;
        let next_sequence = self
            .next_external_event_sequence
            .checked_add(count_u64)
            .ok_or(RunningStepError::ArithmeticOverflow {
                context: "external event sequence",
            })?;
        reserve_for(
            &mut self.pending_events,
            total_count,
            "external event metadata",
        )?;
        reserve_for(
            &mut self.pending_statuses,
            total_count,
            "external delivery statuses",
        )?;
        reserve_for(
            &mut self.pending_sources,
            total_count,
            "external event sources",
        )?;
        reserve_for(
            &mut self.pending_observations,
            surviving_observations,
            "surviving external observations",
        )?;
        reserve_for(
            &mut self.pending_observation_statuses,
            surviving_observations,
            "external observation statuses",
        )?;
        reserve_for(
            &mut self.pending_tokens,
            replacement_count,
            "external replacement token records",
        )?;
        while self.pending_tokens.len() < replacement_count {
            self.pending_tokens.push(String::new());
        }
        self.pending_token_count = replacement_count;
        self.pending_events.clear();
        self.pending_statuses.clear();
        self.pending_sources.clear();
        self.pending_observations.clear();
        self.pending_observation_statuses.clear();

        let first_sequence = self.next_external_event_sequence;
        for assignment_index in 0..replacement_count {
            let assignment = self.world_step.replacement_assignments(key)?[assignment_index];
            let replacement = final_world
                .snakes
                .iter()
                .find(|snake| snake.alive && snake.id == assignment.snake_id)
                .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                    field: "replacement snake identity",
                })?;
            let resume_token = self
                .world_step
                .replacement_resume_token(key, assignment_index)?;
            copy_string_reusing(
                &mut self.pending_tokens[assignment_index],
                resume_token,
                "external replacement resume token",
            )?;
            let event_index = self.pending_events.len();
            let ordinal =
                u64::try_from(event_index).map_err(|_| RunningStepError::ArithmeticOverflow {
                    context: "replacement event ordinal",
                })?;
            let event_sequence = first_sequence.checked_add(ordinal).ok_or(
                RunningStepError::ArithmeticOverflow {
                    context: "replacement event sequence",
                },
            )?;
            self.pending_events.push(ExternalObservationEvent {
                step_key: key,
                event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id,
                controller_kind: assignment.controller_kind,
                delivery_kind: ExternalDeliveryEventKind::ReplacementAssignment {
                    frame_v1_id: assignment.frame_v1_id,
                },
                snake_id: assignment.snake_id,
                position: replacement.position,
                direction: replacement.direction,
                observation_start: 0,
                observation_len: 0,
                token_index: Some(assignment_index),
            });
            self.pending_statuses.push(ExternalDeliveryStatus::Pending);
            self.pending_sources
                .push(PendingExternalSource::ReplacementAssignment { assignment_index });
        }

        for source in source_events.iter().copied() {
            if !final_world
                .snakes
                .iter()
                .any(|snake| snake.alive && snake.id == source.snake_id())
            {
                continue;
            }
            let (observation_start, observation_len) = source.observation_range();
            let observation_end = observation_start.checked_add(observation_len).ok_or(
                RunningStepError::PendingDeliveryStateMismatch {
                    field: "observation range overflow",
                },
            )?;
            if observation_end > observations.len() {
                return Err(RunningStepError::PendingDeliveryStateMismatch {
                    field: "packed observation ranges",
                });
            }
            let event_index = self.pending_events.len();
            let ordinal =
                u64::try_from(event_index).map_err(|_| RunningStepError::ArithmeticOverflow {
                    context: "external event ordinal",
                })?;
            let event_sequence = first_sequence.checked_add(ordinal).ok_or(
                RunningStepError::ArithmeticOverflow {
                    context: "external event sequence",
                },
            )?;
            let retained_index = self.pending_observations.len();
            self.pending_observations.push(source);
            self.pending_observation_statuses
                .push(ExternalDeliveryStatus::Pending);
            self.pending_events.push(ExternalObservationEvent {
                step_key: key,
                event_sequence,
                connection_id: source.connection_id(),
                lease_id: source.lease_id(),
                controller_kind: source.kind(),
                delivery_kind: ExternalDeliveryEventKind::Observation,
                snake_id: source.snake_id(),
                position: source.position(),
                direction: source.direction(),
                observation_start,
                observation_len,
                token_index: None,
            });
            self.pending_statuses.push(ExternalDeliveryStatus::Pending);
            self.pending_sources
                .push(PendingExternalSource::Observation { retained_index });
        }
        if self.pending_events.len() != total_count
            || self.pending_sources.len() != total_count
            || self.pending_observations.len() != surviving_observations
        {
            return Err(RunningStepError::PendingDeliveryStateMismatch {
                field: "combined retained delivery counts",
            });
        }
        self.next_external_event_sequence = next_sequence;
        self.pending_key = Some(key);
        self.pending_inputs = Some(inputs);
        self.pending_delivery_context = Some(PendingDeliveryContext::RunningStep);
        Ok(())
    }

    fn retain_generation_assignment_delivery(
        &mut self,
        key: PhysicsStepKey,
        inputs: RunningStepInputs,
    ) -> Result<(), RunningStepError> {
        let replacement_count = self.world_step.replacement_assignments(key)?.len();
        let count_u64 =
            u64::try_from(replacement_count).map_err(|_| RunningStepError::ArithmeticOverflow {
                context: "generation assignment event count",
            })?;
        let next_sequence = self
            .next_external_event_sequence
            .checked_add(count_u64)
            .ok_or(RunningStepError::ArithmeticOverflow {
                context: "generation assignment event sequence",
            })?;
        reserve_for(
            &mut self.pending_events,
            replacement_count,
            "generation assignment metadata",
        )?;
        reserve_for(
            &mut self.pending_statuses,
            replacement_count,
            "generation assignment statuses",
        )?;
        reserve_for(
            &mut self.pending_sources,
            replacement_count,
            "generation assignment sources",
        )?;
        reserve_for(
            &mut self.pending_tokens,
            replacement_count,
            "generation assignment token records",
        )?;
        while self.pending_tokens.len() < replacement_count {
            self.pending_tokens.push(String::new());
        }
        self.pending_events.clear();
        self.pending_statuses.clear();
        self.pending_sources.clear();
        self.pending_observations.clear();
        self.pending_observation_statuses.clear();
        self.pending_token_count = replacement_count;
        self.pending_preflight = None;

        let final_world = self.world_step.staged_world(key)?;
        let first_sequence = self.next_external_event_sequence;
        for assignment_index in 0..replacement_count {
            let assignment = self.world_step.replacement_assignments(key)?[assignment_index];
            let replacement = final_world
                .snakes
                .iter()
                .find(|snake| snake.alive && snake.id == assignment.snake_id)
                .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                    field: "generation replacement snake identity",
                })?;
            let resume_token = self
                .world_step
                .replacement_resume_token(key, assignment_index)?;
            copy_string_reusing(
                &mut self.pending_tokens[assignment_index],
                resume_token,
                "generation replacement resume token",
            )?;
            let ordinal = u64::try_from(assignment_index).map_err(|_| {
                RunningStepError::ArithmeticOverflow {
                    context: "generation assignment event ordinal",
                }
            })?;
            let event_sequence = first_sequence.checked_add(ordinal).ok_or(
                RunningStepError::ArithmeticOverflow {
                    context: "generation assignment event sequence",
                },
            )?;
            self.pending_events.push(ExternalObservationEvent {
                step_key: key,
                event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id,
                controller_kind: assignment.controller_kind,
                delivery_kind: ExternalDeliveryEventKind::ReplacementAssignment {
                    frame_v1_id: assignment.frame_v1_id,
                },
                snake_id: assignment.snake_id,
                position: replacement.position,
                direction: replacement.direction,
                observation_start: 0,
                observation_len: 0,
                token_index: Some(assignment_index),
            });
            self.pending_statuses.push(ExternalDeliveryStatus::Pending);
            self.pending_sources
                .push(PendingExternalSource::ReplacementAssignment { assignment_index });
        }
        if self.pending_events.len() != replacement_count
            || self.pending_statuses.len() != replacement_count
            || self.pending_sources.len() != replacement_count
        {
            return Err(RunningStepError::PendingDeliveryStateMismatch {
                field: "generation assignment retained counts",
            });
        }
        self.next_external_event_sequence = next_sequence;
        self.pending_key = Some(key);
        self.pending_inputs = Some(inputs);
        self.pending_delivery_context = Some(PendingDeliveryContext::GenerationStart);
        Ok(())
    }

    fn validate_pending_batch(&self, key: PhysicsStepKey) -> Result<(), RunningStepError> {
        let pending_key = self
            .pending_key
            .ok_or(RunningStepError::PendingDeliveryStateMismatch { field: "step key" })?;
        if self.pending_events.len() != self.pending_statuses.len()
            || self.pending_events.len() != self.pending_sources.len()
            || self.pending_observations.len() != self.pending_observation_statuses.len()
        {
            return Err(RunningStepError::PendingDeliveryStateMismatch {
                field: "event and status counts",
            });
        }
        if key != pending_key {
            return Err(RunningStepError::PendingDeliveryStateMismatch { field: "step key" });
        }
        let context = self.pending_delivery_context.ok_or(
            RunningStepError::PendingDeliveryStateMismatch {
                field: "delivery context",
            },
        )?;
        let empty_observations: &[f32] = &[];
        let observations = match context {
            PendingDeliveryContext::RunningStep => {
                self.control_commit
                    .external_delivery_buffers(pending_key)?
                    .1
            }
            PendingDeliveryContext::GenerationStart => empty_observations,
        };
        for (index, event) in self.pending_events.iter().enumerate() {
            if event.step_key != key
                || (index != 0
                    && self.pending_events[index - 1].event_sequence >= event.event_sequence)
            {
                return Err(RunningStepError::PendingDeliveryStateMismatch {
                    field: "event identity order",
                });
            }
            match (event.delivery_kind, self.pending_sources[index]) {
                (
                    ExternalDeliveryEventKind::Observation,
                    PendingExternalSource::Observation { retained_index },
                ) if context == PendingDeliveryContext::RunningStep => {
                    let source = self.pending_observations.get(retained_index).ok_or(
                        RunningStepError::PendingDeliveryStateMismatch {
                            field: "retained observation index",
                        },
                    )?;
                    let end = event
                        .observation_start
                        .checked_add(event.observation_len)
                        .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                            field: "observation range overflow",
                        })?;
                    if end > observations.len()
                        || source.snake_id() != event.snake_id
                        || source.lease_id() != event.lease_id
                    {
                        return Err(RunningStepError::PendingDeliveryStateMismatch {
                            field: "observation event mapping",
                        });
                    }
                }
                (
                    ExternalDeliveryEventKind::ReplacementAssignment { .. },
                    PendingExternalSource::ReplacementAssignment { assignment_index },
                ) if event.token_index == Some(assignment_index)
                    && assignment_index < self.pending_token_count => {}
                _ => {
                    return Err(RunningStepError::PendingDeliveryStateMismatch {
                        field: "external event source kind",
                    });
                }
            }
        }
        Ok(())
    }

    fn pending_batch_prevalidated(&self) -> ExternalObservationBatch<'_> {
        let key = self
            .pending_key
            .expect("prevalidated pending delivery must retain its step key");
        let empty_observations: &[f32] = &[];
        let observations = match self
            .pending_delivery_context
            .expect("prevalidated pending delivery must retain its context")
        {
            PendingDeliveryContext::RunningStep => {
                self.control_commit
                    .external_delivery_buffers_prevalidated(key)
                    .1
            }
            PendingDeliveryContext::GenerationStart => empty_observations,
        };
        debug_assert_eq!(self.pending_events.len(), self.pending_statuses.len());
        ExternalObservationBatch {
            events: &self.pending_events,
            observations,
            tokens: &self.pending_tokens[..self.pending_token_count],
            statuses: &self.pending_statuses,
        }
    }

    fn validate_pending_publication(
        &mut self,
        authority: &AuthoritativeState,
        key: PhysicsStepKey,
    ) -> Result<(), RunningStepError> {
        self.validate_pending_batch(key)?;
        let inputs = self
            .pending_inputs
            .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                field: "step inputs",
            })?;
        let preflight =
            self.pending_preflight
                .ok_or(RunningStepError::PendingDeliveryStateMismatch {
                    field: "state preflight",
                })?;
        self.world_step
            .validate_external_delivery_preflight(key, &self.pending_observations)?;
        let mut buffers = self.world_step.validation_buffers(key)?;
        let brains = if let Some(brains) = buffers.replacement_brains.take() {
            brains
        } else {
            self.control_commit.publication_brains(key)?
        };
        authority.validate_preflighted_running_step(
            preflight,
            &RunningStepReplacement {
                key,
                world: buffers.world,
                rng: buffers.rng,
                allocators: buffers.allocators,
                brains,
                baseline_lifecycle: buffers.baseline_lifecycle,
                ambient_pellet_accumulator: buffers.ambient_pellet_accumulator,
                sensor_generation: buffers.sensor_generation,
                generation_elapsed_seconds: buffers.generation_elapsed_seconds,
                wall_accumulator_seconds: inputs.wall_accumulator_seconds,
                mutation: buffers.mutation,
            },
        )?;
        Ok(())
    }

    fn publish_staged<'workspace>(
        &'workspace mut self,
        authority: &mut AuthoritativeState,
        key: PhysicsStepKey,
        inputs: RunningStepInputs,
        diagnostics: WorldStepDiagnostics,
    ) -> Result<RunningStepOutcome<'workspace>, RunningStepError> {
        #[cfg(feature = "engine-test-hooks")]
        let publication_started = Instant::now();
        let publication = (|| -> Result<RunningStepPublication, RunningStepError> {
            let mut buffers = self.world_step.publication_buffers(key)?;
            let brains = if let Some(brains) = buffers.replacement_brains.take() {
                brains
            } else {
                self.control_commit.publication_brains(key)?
            };
            Ok(authority.publish_running_step(RunningStepReplacement {
                key,
                world: buffers.world,
                rng: buffers.rng,
                allocators: buffers.allocators,
                brains,
                baseline_lifecycle: buffers.baseline_lifecycle,
                ambient_pellet_accumulator: buffers.ambient_pellet_accumulator,
                sensor_generation: buffers.sensor_generation,
                generation_elapsed_seconds: buffers.generation_elapsed_seconds,
                wall_accumulator_seconds: inputs.wall_accumulator_seconds,
                mutation: buffers.mutation,
            })?)
        })();
        self.world_step.invalidate_publication();
        self.control_commit.invalidate_publication();
        self.clear_pending_metadata();
        self.last_published_diagnostics = diagnostics;
        #[cfg(feature = "engine-test-hooks")]
        {
            self.last_phase_timings.publication_ms = elapsed_ms(publication_started);
            self.last_phase_allocations.publication =
                allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
        }
        Ok(RunningStepOutcome {
            publication: publication?,
            diagnostics: &self.last_published_diagnostics,
        })
    }

    fn preflight_staged(
        &mut self,
        authority: &mut AuthoritativeState,
        key: PhysicsStepKey,
        inputs: RunningStepInputs,
    ) -> Result<RunningStepPreflight, RunningStepError> {
        let mut buffers = self.world_step.validation_buffers(key)?;
        let brains = if let Some(brains) = buffers.replacement_brains.take() {
            brains
        } else {
            self.control_commit.publication_brains(key)?
        };
        let mut replacement = RunningStepReplacement {
            key,
            world: buffers.world,
            rng: buffers.rng,
            allocators: buffers.allocators,
            brains,
            baseline_lifecycle: buffers.baseline_lifecycle,
            ambient_pellet_accumulator: buffers.ambient_pellet_accumulator,
            sensor_generation: buffers.sensor_generation,
            generation_elapsed_seconds: buffers.generation_elapsed_seconds,
            wall_accumulator_seconds: inputs.wall_accumulator_seconds,
            mutation: buffers.mutation,
        };
        Ok(authority.preflight_running_step(&mut replacement)?)
    }

    fn publish_prevalidated_staged<'workspace>(
        &'workspace mut self,
        authority: &mut AuthoritativeState,
        key: PhysicsStepKey,
        inputs: RunningStepInputs,
        preflight: RunningStepPreflight,
        diagnostics: WorldStepDiagnostics,
    ) -> Result<RunningStepOutcome<'workspace>, RunningStepError> {
        #[cfg(feature = "engine-test-hooks")]
        {
            self.allocation_cursor = self.allocation_snapshot.map(|snapshot| snapshot());
        }
        let mut buffers = self.world_step.publication_buffers_prevalidated(key);
        let brains = buffers
            .replacement_brains
            .take()
            .unwrap_or_else(|| self.control_commit.publication_brains_prevalidated(key));
        let resolved = ResolvedRunningStepReplacement::new(RunningStepReplacement {
            key,
            world: buffers.world,
            rng: buffers.rng,
            allocators: buffers.allocators,
            brains,
            baseline_lifecycle: buffers.baseline_lifecycle,
            ambient_pellet_accumulator: buffers.ambient_pellet_accumulator,
            sensor_generation: buffers.sensor_generation,
            generation_elapsed_seconds: buffers.generation_elapsed_seconds,
            wall_accumulator_seconds: inputs.wall_accumulator_seconds,
            mutation: buffers.mutation,
        });
        let publication = authority.publish_prevalidated_running_step(preflight, resolved);
        self.world_step.invalidate_publication();
        self.control_commit.invalidate_publication();
        self.clear_pending_metadata();
        self.last_published_diagnostics = diagnostics;
        Ok(RunningStepOutcome {
            publication: publication?,
            diagnostics: &self.last_published_diagnostics,
        })
    }

    fn discard_staged(&mut self) {
        self.world_step.invalidate_publication();
        self.control_commit.invalidate_publication();
        self.clear_pending_metadata();
    }

    fn clear_pending_metadata(&mut self) {
        self.pending_key = None;
        self.pending_inputs = None;
        self.pending_preflight = None;
        self.pending_delivery_context = None;
        self.pending_events.clear();
        self.pending_statuses.clear();
        self.pending_sources.clear();
        self.pending_observations.clear();
        self.pending_observation_statuses.clear();
        self.pending_token_count = 0;
    }

    fn validate_authority(&self, authority: &AuthoritativeState) -> Result<(), RunningStepError> {
        let state = authority.state();
        if authority.world_epoch() != self.world_epoch {
            return Err(RunningStepError::AuthorityMismatch {
                field: "world epoch",
            });
        }
        if state.identity.config_revision != self.config_revision {
            return Err(RunningStepError::AuthorityMismatch {
                field: "config revision",
            });
        }
        if state.identity.config_hash != self.config_hash {
            return Err(RunningStepError::AuthorityMismatch {
                field: "config hash",
            });
        }
        if authority.graph().layout_digest_sha256 != self.graph_layout_digest {
            return Err(RunningStepError::AuthorityMismatch {
                field: "graph layout",
            });
        }
        Ok(())
    }

    /// Return the most recent coarse phase timings from a test-hook build.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) const fn last_phase_timings(&self) -> RunningStepPhaseTimings {
        self.last_phase_timings
    }

    /// Install the benchmark allocator counter used only by test-hook builds.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn set_allocation_snapshot(&mut self, snapshot: fn() -> u64) {
        self.allocation_snapshot = Some(snapshot);
        self.allocation_cursor = None;
        self.last_phase_allocations = RunningStepPhaseAllocations::default();
        self.world_step.set_allocation_snapshot(snapshot);
    }

    /// Return the most recent coarse phase allocation counts.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) const fn last_phase_allocations(&self) -> RunningStepPhaseAllocations {
        self.last_phase_allocations
    }

    /// Return the most recent fine-grained allocation counts inside physics.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) const fn last_physics_phase_allocations(&self) -> PhysicsPhaseAllocations {
        self.world_step.physics_phase_allocations()
    }
}

#[cfg(feature = "engine-test-hooks")]
fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

#[cfg(feature = "engine-test-hooks")]
fn allocation_delta(snapshot: Option<fn() -> u64>, cursor: &mut Option<u64>) -> u64 {
    let Some(snapshot) = snapshot else {
        return 0;
    };
    let current = snapshot();
    let previous = cursor.replace(current).unwrap_or(current);
    current.saturating_sub(previous)
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), RunningStepError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| RunningStepError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

fn copy_string_reusing(
    target: &mut String,
    source: &str,
    buffer: &'static str,
) -> Result<(), RunningStepError> {
    if target.capacity() < source.len() {
        target
            .try_reserve_exact(source.len().saturating_sub(target.len()))
            .map_err(|_| RunningStepError::AllocationFailed {
                buffer,
                required: source.len(),
            })?;
    }
    target.clear();
    target.push_str(source);
    Ok(())
}

fn generation_transition_required(
    world: &WorldState,
    elapsed_seconds: f64,
    config: GenerationGuardConfig,
) -> Option<(GenerationTransitionReason, usize)> {
    let alive_evolved = world
        .snakes
        .iter()
        .filter(|snake| {
            snake.alive && snake.kind == SnakeKind::Evolved && snake.population_slot.is_some()
        })
        .count();
    if elapsed_seconds >= config.generation_seconds {
        return Some((GenerationTransitionReason::Duration, alive_evolved));
    }
    if elapsed_seconds >= config.early_end_minimum_seconds
        && alive_evolved <= config.early_end_alive_threshold
    {
        return Some((GenerationTransitionReason::EarlyAliveCount, alive_evolved));
    }
    None
}

/// Failure before one complete nonterminal fixed step becomes authoritative.
#[derive(Debug)]
pub enum RunningStepError {
    /// Admitted normalized configuration could not project.
    Config(Box<StepConfigError>),
    /// Corrected sensor evaluator construction failed.
    Sensor(Box<SensorError>),
    /// Compiled graph execution planning failed.
    Inference(Box<InferenceError>),
    /// Heterogeneous neural pipeline allocation or shape failed.
    Neural(Box<NeuralControlError>),
    /// Fixed-step prefix staging failed.
    Prefix(Box<FixedStepPrefixError>),
    /// Shared control selection or internal commit failed.
    Control(Box<ControlPhaseError>),
    /// Complete physics/world staging failed.
    WorldStep(Box<WorldStepError>),
    /// Evolution or dual-state generation-boundary admission failed.
    Generation(Box<GenerationTransitionError>),
    /// Collision-safe next-world construction failed after persistence.
    GenerationStart(Box<GenerationStartError>),
    /// Authority key/admission/publication failed.
    State(Box<StateError>),
    /// The coordinator was built for a different immutable authority contract.
    AuthorityMismatch { field: &'static str },
    /// Scheduler debt was non-finite or negative.
    InvalidSchedulerAccumulator(f64),
    /// A wall-clock value regressed behind an earlier accepted boundary.
    RegressingWallClock { previous_ms: u64, actual_ms: u64 },
    /// Another complete step cannot begin while local send results are pending.
    ExternalDeliveryPending { count: usize },
    /// Retained bridge metadata no longer forms one complete keyed batch.
    PendingDeliveryStateMismatch { field: &'static str },
    /// Reusable bridge storage could not reserve its checked bounded size.
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    /// Checked event sequence or range arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Initial generation construction plus connected-controller placement
    /// exceeded one admitted aggregate work ceiling.
    GenerationWorkBudgetExceeded {
        work: &'static str,
        used: usize,
        maximum: usize,
    },
    /// A staged generation transition already blocks another fixed step.
    GenerationTransitionPending,
    /// No staged generation transition exists for a requested persistence action.
    GenerationTransitionNotPending,
    /// One immutable file is already correlated with a different operation.
    GenerationCheckpointAlreadyPublished { operation_id: String },
    /// SQLite acknowledgement arrived before any immutable file descriptor.
    GenerationCheckpointNotPublished,
    /// SQLite acknowledgement did not exactly echo the published descriptor.
    GenerationPersistenceAcknowledgementMismatch { field: &'static str },
    /// Next-world construction was requested before SQLite commit success.
    GenerationPersistenceNotAcknowledged,
    /// A durable current-pointer commit cannot be discarded as an old attempt.
    GenerationPersistenceAlreadyCommitted,
}

macro_rules! boxed_from {
    ($source:ty, $variant:ident) => {
        impl From<$source> for RunningStepError {
            fn from(error: $source) -> Self {
                Self::$variant(Box::new(error))
            }
        }
    };
}

boxed_from!(StepConfigError, Config);
boxed_from!(SensorError, Sensor);
boxed_from!(InferenceError, Inference);
boxed_from!(NeuralControlError, Neural);
boxed_from!(FixedStepPrefixError, Prefix);
boxed_from!(ControlPhaseError, Control);
boxed_from!(WorldStepError, WorldStep);
boxed_from!(GenerationTransitionError, Generation);
boxed_from!(GenerationStartError, GenerationStart);
boxed_from!(StateError, State);

impl Display for RunningStepError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "{error}"),
            Self::Sensor(error) => write!(formatter, "{error}"),
            Self::Inference(error) => write!(formatter, "{error}"),
            Self::Neural(error) => write!(formatter, "{error}"),
            Self::Prefix(error) => write!(formatter, "{error}"),
            Self::Control(error) => write!(formatter, "{error}"),
            Self::WorldStep(error) => write!(formatter, "{error}"),
            Self::Generation(error) => write!(formatter, "{error}"),
            Self::GenerationStart(error) => write!(formatter, "{error}"),
            Self::State(error) => write!(formatter, "{error}"),
            Self::AuthorityMismatch { field } => {
                write!(
                    formatter,
                    "running-step coordinator authority changed: {field}"
                )
            }
            Self::InvalidSchedulerAccumulator(value) => {
                write!(formatter, "invalid scheduler accumulator {value}")
            }
            Self::RegressingWallClock {
                previous_ms,
                actual_ms,
            } => write!(
                formatter,
                "controller wall clock regressed from {previous_ms} ms to {actual_ms} ms"
            ),
            Self::ExternalDeliveryPending { count } => write!(
                formatter,
                "{count} external observations still require matching Node delivery results"
            ),
            Self::PendingDeliveryStateMismatch { field } => {
                write!(
                    formatter,
                    "pending external delivery state mismatch: {field}"
                )
            }
            Self::AllocationFailed { buffer, required } => write!(
                formatter,
                "failed to reserve {required} entries for running-step {buffer}"
            ),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "running-step arithmetic overflow: {context}")
            }
            Self::GenerationWorkBudgetExceeded {
                work,
                used,
                maximum,
            } => write!(
                formatter,
                "complete generation used {used} {work}; maximum is {maximum}"
            ),
            Self::GenerationTransitionPending => {
                write!(
                    formatter,
                    "a generation transition is waiting for persistence"
                )
            }
            Self::GenerationTransitionNotPending => {
                write!(
                    formatter,
                    "no generation transition is waiting for persistence"
                )
            }
            Self::GenerationCheckpointAlreadyPublished { operation_id } => write!(
                formatter,
                "generation checkpoint is already bound to operation {operation_id}"
            ),
            Self::GenerationCheckpointNotPublished => write!(
                formatter,
                "generation persistence cannot commit before checkpoint publication"
            ),
            Self::GenerationPersistenceAcknowledgementMismatch { field } => write!(
                formatter,
                "generation persistence acknowledgement changed {field}"
            ),
            Self::GenerationPersistenceNotAcknowledged => write!(
                formatter,
                "generation construction requires a successful persistence acknowledgement"
            ),
            Self::GenerationPersistenceAlreadyCommitted => write!(
                formatter,
                "a committed generation checkpoint cannot be discarded as an uncommitted attempt"
            ),
        }
    }
}

impl Error for RunningStepError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(error) => Some(error.as_ref()),
            Self::Sensor(error) => Some(error.as_ref()),
            Self::Inference(error) => Some(error.as_ref()),
            Self::Neural(error) => Some(error.as_ref()),
            Self::Prefix(error) => Some(error.as_ref()),
            Self::Control(error) => Some(error.as_ref()),
            Self::WorldStep(error) => Some(error.as_ref()),
            Self::Generation(error) => Some(error.as_ref()),
            Self::GenerationStart(error) => Some(error.as_ref()),
            Self::State(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}
