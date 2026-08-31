//! Durable fresh-run checkpoint and authority-activation barrier.
//!
//! A generation-one/step-zero candidate is admitted and kept private here. It
//! can publish one immutable managed checkpoint, but it cannot construct or
//! expose running authority until the SQLite metadata/current-pointer worker
//! echoes that exact complete descriptor. Retry reuses the same admitted
//! boundary and descriptor; JavaScript never supplies transition chronology or
//! authoritative population data.

use super::checkpoint::{
    publish_checkpoint, CheckpointDescriptor, CheckpointError, CheckpointLimits,
    CheckpointOperationId,
};
use super::frame_v1::{
    pack_authoritative_frame_v1_into, FrameV1Error, FrameV1Metadata, FrameV1ViewDescriptor,
};
use super::generation_start::{
    GenerationStartConfig, GenerationStartError, GenerationStartWorkspace,
};
use super::graph::{GraphBundle, GraphLimits};
use super::running_loop::{RunningAuthorityLoop, RunningAuthorityLoopError};
use super::running_step::{RunningStepCoordinator, RunningStepError, RunningStepProgress};
use super::scheduler::{
    FixedStepScheduler, FixedStepSchedulerPolicy, SchedulerError, SchedulerReadiness,
    SchedulerServiceMode,
};
use super::state::{
    AuthoritativeState, AuthorityPhase, GenerationBoundaryKind, RunStartPublication,
    StateAdmissionPolicy, StateCandidate, StateError,
};
use super::step_config::RunningStepWorkLimits;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::Arc;

/// First fresh-run persistence and activation contract.
pub const RUN_START_TRANSITION_VERSION: u32 = 1;

/// Opaque process-local proof created only after exact persistence acknowledgement.
pub(crate) struct RunStartPersistenceProof {
    source_address: usize,
    world_epoch: u64,
}

impl RunStartPersistenceProof {
    fn new(authority: &AuthoritativeState) -> Self {
        Self {
            source_address: std::ptr::from_ref(authority.state()).addr(),
            world_epoch: authority.world_epoch(),
        }
    }

    pub(crate) fn matches(&self, source_address: usize, world_epoch: u64) -> bool {
        self.source_address == source_address && self.world_epoch == world_epoch
    }
}

/// One admitted fresh run retained until durability authorizes activation.
#[derive(Debug)]
pub struct PendingRunStartTransition {
    authority: AuthoritativeState,
    admission_policy: StateAdmissionPolicy,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
    work_limits: RunningStepWorkLimits,
    generation_start: GenerationStartWorkspace,
    checkpoint_descriptor: Option<CheckpointDescriptor>,
    persistence_acknowledged: bool,
    authority_published: bool,
    first_scheduled_step_attempted: bool,
    first_scheduled_frame_published: bool,
}

impl PendingRunStartTransition {
    /// Admit one complete generation-one boundary without making it runnable.
    #[allow(clippy::too_many_arguments)]
    pub fn admit(
        candidate: StateCandidate,
        graph: Arc<GraphBundle>,
        admission_policy: StateAdmissionPolicy,
        checkpoint_limits: CheckpointLimits,
        graph_limits: GraphLimits,
        work_limits: RunningStepWorkLimits,
    ) -> Result<Self, RunStartTransitionError> {
        if candidate.phase != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart)
            || candidate.generation.generation != 1
            || candidate.generation.completed_step != 0
            || candidate.generation.population_epoch != 1
        {
            return Err(RunStartTransitionError::InvalidBoundary);
        }
        let authority = AuthoritativeState::validate_and_own(candidate, graph, &admission_policy)?;
        Ok(Self {
            authority,
            admission_policy,
            checkpoint_limits,
            graph_limits,
            work_limits,
            generation_start: GenerationStartWorkspace::new(),
            checkpoint_descriptor: None,
            persistence_acknowledged: false,
            authority_published: false,
            first_scheduled_step_attempted: false,
            first_scheduled_frame_published: false,
        })
    }

    /// Publish or exactly retry the immutable run-start file.
    ///
    /// The transition epoch is the Rust-allocated process-local world
    /// incarnation. It is used only as a nonzero handoff correlation token;
    /// generation and completed-step values remain the persistent chronology.
    pub fn publish_checkpoint(
        &mut self,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
    ) -> Result<CheckpointDescriptor, RunStartTransitionError> {
        if self.authority_published {
            return Err(RunStartTransitionError::AuthorityAlreadyPublished);
        }
        if let Some(descriptor) = &self.checkpoint_descriptor {
            if descriptor.operation_id == operation_id {
                return Ok(descriptor.clone());
            }
            return Err(RunStartTransitionError::CheckpointAlreadyPublished {
                operation_id: descriptor.operation_id.as_str().to_owned(),
            });
        }
        let descriptor = publish_checkpoint(
            managed_directory,
            operation_id,
            self.authority.world_epoch(),
            self.authority.checkpoint_boundary()?,
            &self.checkpoint_limits,
            &self.graph_limits,
            &self.admission_policy,
        )?;
        self.checkpoint_descriptor = Some(descriptor.clone());
        Ok(descriptor)
    }

    /// Retain only the exact descriptor committed by the SQLite worker.
    pub fn acknowledge_persistence(
        &mut self,
        committed: &CheckpointDescriptor,
    ) -> Result<(), RunStartTransitionError> {
        if self.authority_published {
            return Err(RunStartTransitionError::AuthorityAlreadyPublished);
        }
        let expected = self
            .checkpoint_descriptor
            .as_ref()
            .ok_or(RunStartTransitionError::CheckpointNotPublished)?;
        if let Some(field) = expected.first_mismatch(committed) {
            return Err(RunStartTransitionError::PersistenceAcknowledgementMismatch { field });
        }
        self.persistence_acknowledged = true;
        Ok(())
    }

    /// Construct and atomically activate the running world after exact durability.
    ///
    /// A construction or state-admission failure leaves the durable boundary and
    /// acknowledgement retained for an explicit deterministic retry.
    pub fn publish_running_authority(
        &mut self,
    ) -> Result<RunStartPublication, RunStartTransitionError> {
        if self.authority_published {
            return Err(RunStartTransitionError::AuthorityAlreadyPublished);
        }
        if !self.persistence_acknowledged {
            return Err(RunStartTransitionError::PersistenceNotAcknowledged);
        }
        let config = GenerationStartConfig::from_work_limits(self.work_limits);
        if !self
            .generation_start
            .retains(self.authority.state(), config)
        {
            let _prepared = self
                .generation_start
                .prepare(self.authority.state(), config)?;
        }
        let persistence_proof = RunStartPersistenceProof::new(&self.authority);
        let publication = self.generation_start.publish_initial_run_start(
            &mut self.authority,
            config,
            &persistence_proof,
        )?;
        self.authority_published = true;
        Ok(publication)
    }

    /// Pack the first neutral-view browser frame only after running publication.
    ///
    /// This keeps the authoritative state private while allowing the coarse
    /// experimental adapter to transfer one replaceable display payload.
    pub fn pack_initial_frame_v1(
        &self,
        output: &mut Vec<u8>,
    ) -> Result<FrameV1Metadata, RunStartTransitionError> {
        if !self.authority_published {
            return Err(RunStartTransitionError::AuthorityNotPublished);
        }
        Ok(pack_authoritative_frame_v1_into(
            &self.authority,
            FrameV1ViewDescriptor::default(),
            output,
        )?)
    }

    /// Execute exactly one Rust-scheduled fixed step and pack its resulting frame.
    ///
    /// This is the bounded forward bridge used by the experimental fixed-P0
    /// session before the continuous background runtime is connected. The
    /// caller supplies no clock, scheduler debt, controls, world data, IDs, or
    /// statistics. Rust derives the smallest whole-millisecond service boundary
    /// that makes one admitted fixed delta due, executes the complete running
    /// coordinator, commits the exact scheduler ticket, and then packs frame v1.
    ///
    /// Once execution begins the operation is permanently single-use. That
    /// prevents an unexpected delivery/generation branch or a post-publication
    /// frame failure from being retried as a second hidden authoritative step.
    pub fn publish_first_scheduled_frame_v1(
        &mut self,
        output: &mut Vec<u8>,
    ) -> Result<FrameV1Metadata, RunStartTransitionError> {
        if !self.authority_published {
            return Err(RunStartTransitionError::AuthorityNotPublished);
        }
        if self.first_scheduled_step_attempted {
            return Err(RunStartTransitionError::FirstScheduledStepAlreadyAttempted);
        }
        self.first_scheduled_step_attempted = true;

        let mut coordinator = RunningStepCoordinator::try_new(&self.authority, self.work_limits)?;
        let mut scheduler = FixedStepScheduler::try_new(
            &self.authority,
            FixedStepSchedulerPolicy::provisional_defaults(),
        )?;
        scheduler.reset_wall_clock(&self.authority, 0)?;
        let service_wall_ms = first_step_service_wall_ms(&self.authority)?;
        let readiness = scheduler.service_after_command_drain(
            &self.authority,
            service_wall_ms,
            SchedulerServiceMode::Background,
        )?;
        if !matches!(readiness, SchedulerReadiness::StepDue { .. }) {
            return Err(RunStartTransitionError::FirstScheduledStepNotDue);
        }
        let step = scheduler.prepare_due_step(&self.authority)?;
        let publication = match coordinator
            .advance_nonterminal(&mut self.authority, step.running_step_inputs())?
        {
            RunningStepProgress::Published(outcome) => outcome.publication,
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                return Err(
                    RunStartTransitionError::UnexpectedFirstStepExternalDelivery {
                        remaining: batch.remaining(),
                    },
                );
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                return Err(RunStartTransitionError::UnexpectedFirstStepGenerationTransition);
            }
        };
        scheduler.commit_step(&self.authority, step, publication)?;
        let metadata = pack_authoritative_frame_v1_into(
            &self.authority,
            FrameV1ViewDescriptor::default(),
            output,
        )?;
        self.first_scheduled_frame_published = true;
        Ok(metadata)
    }

    /// Consume an activated step-zero authority into its retained Rust loop.
    ///
    /// This is the future background-thread handoff. The experimental one-shot
    /// step and the retained loop are deliberately exclusive so two scheduler
    /// owners can never advance the same authority incarnation.
    pub fn into_running_loop(
        self,
        policy: FixedStepSchedulerPolicy,
        wall_origin_ms: u64,
    ) -> Result<RunningAuthorityLoop, RunStartLoopHandoffError> {
        if !self.authority_published {
            return Err(RunStartLoopHandoffError::new(
                self,
                RunStartTransitionError::AuthorityNotPublished,
            ));
        }
        if self.first_scheduled_step_attempted {
            return Err(RunStartLoopHandoffError::new(
                self,
                RunStartTransitionError::ExperimentalStepAlreadyOwnsAuthority,
            ));
        }
        let prepared = match RunningAuthorityLoop::prepare(
            &self.authority,
            self.work_limits,
            policy,
            wall_origin_ms,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return Err(RunStartLoopHandoffError::new(
                    self,
                    RunStartTransitionError::from(error),
                ))
            }
        };
        Ok(RunningAuthorityLoop::from_prepared(
            self.authority,
            prepared,
        ))
    }

    /// Exact Rust-owned transition correlation token.
    #[must_use]
    pub const fn transition_epoch(&self) -> u64 {
        self.authority.world_epoch()
    }

    /// Current generation, exposed only as bounded scalar proof.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.authority.state().generation.generation
    }

    /// Current completed-step count, exposed only as bounded scalar proof.
    #[must_use]
    pub fn completed_step(&self) -> u64 {
        self.authority.state().generation.completed_step
    }

    /// Whether the immutable descriptor has published.
    #[must_use]
    pub const fn checkpoint_published(&self) -> bool {
        self.checkpoint_descriptor.is_some()
    }

    /// Whether the exact SQLite commit acknowledgement is retained.
    #[must_use]
    pub const fn persistence_acknowledged(&self) -> bool {
        self.persistence_acknowledged
    }

    /// Whether the collision-safe running authority is now active.
    #[must_use]
    pub const fn authority_published(&self) -> bool {
        self.authority_published
    }

    /// Whether the one experimental scheduled-step attempt has started.
    #[must_use]
    pub const fn first_scheduled_step_attempted(&self) -> bool {
        self.first_scheduled_step_attempted
    }

    /// Whether that exact step also produced its post-publication frame.
    #[must_use]
    pub const fn first_scheduled_frame_published(&self) -> bool {
        self.first_scheduled_frame_published
    }

    /// Current authoritative snake count as bounded activation proof.
    #[must_use]
    pub fn snake_count(&self) -> usize {
        self.authority.state().world.snakes.len()
    }

    /// Current authoritative pellet count as bounded activation proof.
    #[must_use]
    pub fn pellet_count(&self) -> usize {
        self.authority.state().world.pellets.len()
    }
}

/// Recoverable failure before run-start authority moves into the retained loop.
#[derive(Debug)]
pub struct RunStartLoopHandoffError {
    transition: Box<PendingRunStartTransition>,
    error: RunStartTransitionError,
}

impl RunStartLoopHandoffError {
    fn new(transition: PendingRunStartTransition, error: RunStartTransitionError) -> Self {
        Self {
            transition: Box::new(transition),
            error,
        }
    }

    /// Inspect the failed precondition or construction error.
    #[must_use]
    pub const fn error(&self) -> &RunStartTransitionError {
        &self.error
    }

    /// Recover the exact unchanged run-start transition for retry or shutdown.
    #[must_use]
    pub fn into_transition(self) -> PendingRunStartTransition {
        *self.transition
    }

    /// Recover both the exact transition and its bounded failure.
    #[must_use]
    pub fn into_parts(self) -> (PendingRunStartTransition, RunStartTransitionError) {
        (*self.transition, self.error)
    }
}

impl Display for RunStartLoopHandoffError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "run-start loop handoff failed: {}", self.error)
    }
}

impl Error for RunStartLoopHandoffError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// Failure before a fresh run becomes running authority.
#[derive(Debug)]
pub enum RunStartTransitionError {
    /// The candidate is not generation one at the exact run-start boundary.
    InvalidBoundary,
    /// Complete state admission or publication failed.
    State(Box<StateError>),
    /// Managed checkpoint publication failed.
    Checkpoint(Box<CheckpointError>),
    /// Collision-safe initial-world construction failed.
    GenerationStart(Box<GenerationStartError>),
    /// Direct frame-v1 packing failed after activation.
    Frame(Box<FrameV1Error>),
    /// One immutable file is already correlated with another operation.
    CheckpointAlreadyPublished { operation_id: String },
    /// SQLite acknowledgement arrived before immutable publication.
    CheckpointNotPublished,
    /// The worker did not echo the complete published descriptor.
    PersistenceAcknowledgementMismatch { field: &'static str },
    /// Running construction was requested before SQLite commit success.
    PersistenceNotAcknowledged,
    /// A second activation or persistence mutation was attempted.
    AuthorityAlreadyPublished,
    /// Display publication was attempted before running authority existed.
    AuthorityNotPublished,
    /// The bounded first scheduled-step bridge was already consumed.
    FirstScheduledStepAlreadyAttempted,
    /// The one-shot experiment already created a different scheduler owner.
    ExperimentalStepAlreadyOwnsAuthority,
    /// Rust's internally derived service boundary did not make one step due.
    FirstScheduledStepNotDue,
    /// Fixed-P0 unexpectedly required a Node delivery on its first step.
    UnexpectedFirstStepExternalDelivery { remaining: usize },
    /// Fixed-P0 unexpectedly reached a generation boundary on its first step.
    UnexpectedFirstStepGenerationTransition,
    /// Complete running-step construction or publication failed.
    RunningStep(Box<RunningStepError>),
    /// Rust-owned fixed-step scheduling failed.
    Scheduler(Box<SchedulerError>),
    /// Retained running-authority loop construction failed.
    RunningLoop(Box<RunningAuthorityLoopError>),
}

impl Display for RunStartTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBoundary => write!(
                formatter,
                "run-start transition requires generation one at completed step zero"
            ),
            Self::State(error) => write!(formatter, "run-start state failed: {error}"),
            Self::Checkpoint(error) => write!(formatter, "run-start checkpoint failed: {error}"),
            Self::GenerationStart(error) => {
                write!(formatter, "run-start world construction failed: {error}")
            }
            Self::Frame(error) => write!(formatter, "run-start frame-v1 packing failed: {error}"),
            Self::CheckpointAlreadyPublished { operation_id } => write!(
                formatter,
                "run-start checkpoint is already bound to operation {operation_id}"
            ),
            Self::CheckpointNotPublished => write!(
                formatter,
                "run-start persistence cannot be acknowledged before checkpoint publication"
            ),
            Self::PersistenceAcknowledgementMismatch { field } => write!(
                formatter,
                "run-start persistence acknowledgement mismatched {field}"
            ),
            Self::PersistenceNotAcknowledged => write!(
                formatter,
                "run-start activation requires a successful persistence acknowledgement"
            ),
            Self::AuthorityAlreadyPublished => {
                write!(formatter, "run-start authority has already been published")
            }
            Self::AuthorityNotPublished => {
                write!(
                    formatter,
                    "run-start frame-v1 requires published running authority"
                )
            }
            Self::FirstScheduledStepAlreadyAttempted => write!(
                formatter,
                "run-start first scheduled frame-v1 step has already been attempted"
            ),
            Self::ExperimentalStepAlreadyOwnsAuthority => write!(
                formatter,
                "run-start one-shot scheduled step already owns this authority handoff"
            ),
            Self::FirstScheduledStepNotDue => write!(
                formatter,
                "run-start internally derived scheduler boundary did not make one step due"
            ),
            Self::UnexpectedFirstStepExternalDelivery { remaining } => write!(
                formatter,
                "run-start fixed-P0 first step unexpectedly requires {remaining} external deliveries"
            ),
            Self::UnexpectedFirstStepGenerationTransition => write!(
                formatter,
                "run-start fixed-P0 first step unexpectedly reached a generation transition"
            ),
            Self::RunningStep(error) => {
                write!(formatter, "run-start first scheduled step failed: {error}")
            }
            Self::Scheduler(error) => {
                write!(formatter, "run-start first scheduled step scheduler failed: {error}")
            }
            Self::RunningLoop(error) => {
                write!(formatter, "run-start retained running loop failed: {error}")
            }
        }
    }
}

impl Error for RunStartTransitionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Checkpoint(error) => Some(error),
            Self::GenerationStart(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::RunningStep(error) => Some(error),
            Self::Scheduler(error) => Some(error),
            Self::RunningLoop(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StateError> for RunStartTransitionError {
    fn from(error: StateError) -> Self {
        Self::State(Box::new(error))
    }
}

impl From<CheckpointError> for RunStartTransitionError {
    fn from(error: CheckpointError) -> Self {
        Self::Checkpoint(Box::new(error))
    }
}

impl From<GenerationStartError> for RunStartTransitionError {
    fn from(error: GenerationStartError) -> Self {
        Self::GenerationStart(Box::new(error))
    }
}

impl From<FrameV1Error> for RunStartTransitionError {
    fn from(error: FrameV1Error) -> Self {
        Self::Frame(Box::new(error))
    }
}

impl From<RunningStepError> for RunStartTransitionError {
    fn from(error: RunningStepError) -> Self {
        Self::RunningStep(Box::new(error))
    }
}

impl From<SchedulerError> for RunStartTransitionError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(Box::new(error))
    }
}

impl From<RunningAuthorityLoopError> for RunStartTransitionError {
    fn from(error: RunningAuthorityLoopError) -> Self {
        Self::RunningLoop(Box::new(error))
    }
}

/// Derive the smallest positive whole-millisecond boundary that requests one step.
fn first_step_service_wall_ms(
    authority: &AuthoritativeState,
) -> Result<u64, RunStartTransitionError> {
    let state = authority.state();
    let required_wall_seconds = state.config.fixed_step_seconds / state.config.requested_sim_speed;
    let required_wall_ms = (required_wall_seconds * 1_000.0).ceil();
    if !required_wall_ms.is_finite() || required_wall_ms < 1.0 || required_wall_ms > u64::MAX as f64
    {
        return Err(RunStartTransitionError::FirstScheduledStepNotDue);
    }
    Ok(required_wall_ms as u64)
}
