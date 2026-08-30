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
use super::generation_start::{
    GenerationStartConfig, GenerationStartError, GenerationStartWorkspace,
};
use super::frame_v1::{
    pack_authoritative_frame_v1_into, FrameV1Error, FrameV1Metadata, FrameV1ViewDescriptor,
};
use super::graph::{GraphBundle, GraphLimits};
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
                write!(formatter, "run-start frame-v1 requires published running authority")
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
