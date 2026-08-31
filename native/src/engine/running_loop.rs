//! Retained Rust ownership across repeated authoritative scheduler boundaries.
//!
//! This layer joins the one-step scheduler, complete running-step coordinator,
//! and direct frame-v1 packer without introducing a per-step N-API contract.
//! The future background runtime owns one instance and supplies only its
//! monotonic clock, already-drained command boundary, presentation-only view,
//! and reusable frame storage.

use super::frame_v1::{
    pack_authoritative_frame_v1_into, FrameV1Error, FrameV1Metadata, FrameV1ViewDescriptor,
};
use super::physics::PhysicsStepKey;
use super::running_step::{
    ExternalObservationBatch, GenerationTransitionBatch, GenerationTransitionReason,
    RunningStepCoordinator, RunningStepError, RunningStepProgress,
};
use super::scheduler::{
    FixedStepScheduler, FixedStepSchedulerDiagnostics, FixedStepSchedulerPolicy, ScheduledStep,
    SchedulerError, SchedulerReadiness, SchedulerServiceMode,
};
use super::state::{AuthoritativeState, RunningStepPublication};
use super::step_config::RunningStepWorkLimits;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Version of the retained running-authority loop contract.
pub const RUNNING_AUTHORITY_LOOP_VERSION: u32 = 1;

/// Current ability of one retained running-authority loop to accept a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunningAuthorityLoopState {
    /// The next drained command boundary may service the scheduler.
    Ready,
    /// One complete staged step awaits exact reliable-delivery results.
    ExternalDeliveryPending,
    /// One terminal step awaits checkpoint metadata and successor admission.
    GenerationTransitionPending,
    /// A first unrecoverable loop error permanently ended this instance.
    Faulted,
}

/// Caller-owned optional output for one successful authoritative publication.
pub struct RunningFramePublication<'buffer> {
    /// Presentation-only values echoed into frame-v1 header fields.
    pub view: FrameV1ViewDescriptor,
    /// Reusable storage retained by the future background frame publisher.
    pub output: &'buffer mut Vec<u8>,
}

impl<'buffer> RunningFramePublication<'buffer> {
    /// Bind one presentation descriptor to caller-owned reusable storage.
    #[must_use]
    pub fn new(view: FrameV1ViewDescriptor, output: &'buffer mut Vec<u8>) -> Self {
        Self { view, output }
    }
}

/// Owned scalar result of one post-command-drain service opportunity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RunningAuthorityLoopProgress {
    /// No complete fixed delta is due yet.
    Idle {
        /// Simulated seconds still needed for the next complete step.
        simulation_seconds_until_step: f64,
        /// Equivalent real-wall seconds at the admitted multiplier.
        wall_seconds_until_step: f64,
    },
    /// One exact scheduler ticket and authoritative step published.
    Published {
        /// Monotonic scheduler ticket identity.
        ticket_sequence: u64,
        /// Complete steps represented by retained debt before this publication.
        due_steps: usize,
        /// Exact authority publication correlated with the ticket.
        publication: RunningStepPublication,
        /// Frame metadata when this service boundary requested frame packing.
        frame: Option<FrameV1Metadata>,
    },
    /// A complete staged step is blocked on reliable external delivery.
    ExternalDeliveryPending {
        /// Scheduler ticket retained until matching delivery resolution.
        ticket_sequence: u64,
        /// Exact unresolved event count.
        remaining: usize,
    },
    /// A terminal step is blocked on its durable generation handoff.
    GenerationTransitionPending {
        /// Scheduler ticket retained through persistence and successor admission.
        ticket_sequence: u64,
        /// Exact terminal source identity.
        source_key: PhysicsStepKey,
        /// Rule that ended the generation.
        reason: GenerationTransitionReason,
        /// Fully admitted successor generation identity.
        successor_generation: u64,
        /// Successor completed-step chronology.
        successor_completed_step: u64,
    },
}

/// One retained scheduler and complete-step coordinator around running authority.
#[derive(Debug)]
pub struct RunningAuthorityLoop {
    authority: AuthoritativeState,
    scheduler: FixedStepScheduler,
    coordinator: RunningStepCoordinator,
    pending_step: Option<ScheduledStep>,
    pending_due_steps: Option<usize>,
    state: RunningAuthorityLoopState,
}

/// Fallible scheduler/coordinator construction completed before authority moves.
pub(crate) struct PreparedRunningAuthorityLoop {
    scheduler: FixedStepScheduler,
    coordinator: RunningStepCoordinator,
}

impl RunningAuthorityLoop {
    /// Complete every fallible handoff check while the prior owner remains intact.
    ///
    /// `wall_origin_ms` excludes asynchronous durability and startup time from
    /// scheduling debt. Authority moves only through the infallible
    /// [`Self::from_prepared`] call after this succeeds.
    pub(crate) fn prepare(
        authority: &AuthoritativeState,
        work_limits: RunningStepWorkLimits,
        policy: FixedStepSchedulerPolicy,
        wall_origin_ms: u64,
    ) -> Result<PreparedRunningAuthorityLoop, RunningAuthorityLoopError> {
        let coordinator = RunningStepCoordinator::try_new(authority, work_limits)?;
        let mut scheduler = FixedStepScheduler::try_new(authority, policy)?;
        scheduler.reset_wall_clock(authority, wall_origin_ms)?;
        Ok(PreparedRunningAuthorityLoop {
            scheduler,
            coordinator,
        })
    }

    /// Move authority only after [`Self::prepare`] has made construction infallible.
    pub(crate) fn from_prepared(
        authority: AuthoritativeState,
        prepared: PreparedRunningAuthorityLoop,
    ) -> Self {
        Self {
            authority,
            scheduler: prepared.scheduler,
            coordinator: prepared.coordinator,
            pending_step: None,
            pending_due_steps: None,
            state: RunningAuthorityLoopState::Ready,
        }
    }

    /// Service one already-drained command/action boundary and at most one step.
    ///
    /// The monotonic clock is supplied by the future Rust background runtime,
    /// never by JavaScript. A caller may omit frame packing at non-display
    /// cadence boundaries. Any error permanently faults this loop so a retry
    /// cannot conceal a partially published authoritative step.
    pub fn service_after_command_drain(
        &mut self,
        wall_now_ms: u64,
        mode: SchedulerServiceMode,
        frame: Option<RunningFramePublication<'_>>,
    ) -> Result<RunningAuthorityLoopProgress, RunningAuthorityLoopError> {
        match self.state {
            RunningAuthorityLoopState::Faulted => {
                return Err(RunningAuthorityLoopError::AlreadyFaulted)
            }
            RunningAuthorityLoopState::ExternalDeliveryPending
            | RunningAuthorityLoopState::GenerationTransitionPending => {
                let retained = self.retained_blocked_progress();
                if retained.is_err() {
                    self.state = RunningAuthorityLoopState::Faulted;
                }
                return retained;
            }
            RunningAuthorityLoopState::Ready => {}
        }

        let result = self.service_ready_boundary(wall_now_ms, mode, frame);
        if result.is_err() {
            self.state = RunningAuthorityLoopState::Faulted;
        }
        result
    }

    /// Current loop state without exposing world or population storage.
    #[must_use]
    pub const fn state(&self) -> RunningAuthorityLoopState {
        self.state
    }

    /// Current authoritative generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.authority.state().generation.generation
    }

    /// Current published authoritative completed-step count.
    #[must_use]
    pub fn completed_step(&self) -> u64 {
        self.authority.state().generation.completed_step
    }

    /// Current authoritative pellet count as bounded runtime metadata.
    #[must_use]
    pub fn pellet_count(&self) -> usize {
        self.authority.state().world.pellets.len()
    }

    /// Frame storage already charged by state admission.
    #[must_use]
    pub fn admitted_frame_bytes(&self) -> usize {
        self.authority.memory_estimate().frame_bytes
    }

    /// Current operational scheduler diagnostics.
    #[must_use]
    pub fn scheduler_diagnostics(&self) -> FixedStepSchedulerDiagnostics {
        self.scheduler.diagnostics()
    }

    /// Borrow the exact retained external batch for future reliable routing.
    #[must_use]
    pub fn pending_external_delivery(&self) -> Option<ExternalObservationBatch<'_>> {
        if self.state != RunningAuthorityLoopState::ExternalDeliveryPending {
            return None;
        }
        self.coordinator.pending_external_delivery()
    }

    /// Borrow the exact retained terminal transition for persistence handoff.
    #[must_use]
    pub fn pending_generation_transition(&self) -> Option<GenerationTransitionBatch<'_>> {
        if self.state != RunningAuthorityLoopState::GenerationTransitionPending {
            return None;
        }
        self.coordinator.pending_generation_transition()
    }

    fn service_ready_boundary(
        &mut self,
        wall_now_ms: u64,
        mode: SchedulerServiceMode,
        frame: Option<RunningFramePublication<'_>>,
    ) -> Result<RunningAuthorityLoopProgress, RunningAuthorityLoopError> {
        let readiness =
            self.scheduler
                .service_after_command_drain(&self.authority, wall_now_ms, mode)?;
        let SchedulerReadiness::StepDue { due_steps, .. } = readiness else {
            let SchedulerReadiness::Idle {
                simulation_seconds_until_step,
                wall_seconds_until_step,
            } = readiness
            else {
                unreachable!("scheduler readiness variants are exhaustive")
            };
            return Ok(RunningAuthorityLoopProgress::Idle {
                simulation_seconds_until_step,
                wall_seconds_until_step,
            });
        };

        let step = self.scheduler.prepare_due_step(&self.authority)?;
        self.pending_step = Some(step);
        self.pending_due_steps = Some(due_steps);
        match self
            .coordinator
            .advance_nonterminal(&mut self.authority, step.running_step_inputs())?
        {
            RunningStepProgress::Published(outcome) => {
                let publication = outcome.publication;
                self.scheduler
                    .commit_step(&self.authority, step, publication)?;
                self.pending_step = None;
                self.pending_due_steps = None;
                let frame = match frame {
                    Some(publication_request) => Some(pack_authoritative_frame_v1_into(
                        &self.authority,
                        publication_request.view,
                        publication_request.output,
                    )?),
                    None => None,
                };
                self.state = RunningAuthorityLoopState::Ready;
                Ok(RunningAuthorityLoopProgress::Published {
                    ticket_sequence: step.sequence(),
                    due_steps,
                    publication,
                    frame,
                })
            }
            RunningStepProgress::ExternalDeliveryPending(batch) => {
                self.state = RunningAuthorityLoopState::ExternalDeliveryPending;
                Ok(RunningAuthorityLoopProgress::ExternalDeliveryPending {
                    ticket_sequence: step.sequence(),
                    remaining: batch.remaining(),
                })
            }
            RunningStepProgress::GenerationTransitionPending(batch) => {
                self.state = RunningAuthorityLoopState::GenerationTransitionPending;
                Ok(generation_pending_progress(step, batch))
            }
        }
    }

    fn retained_blocked_progress(
        &self,
    ) -> Result<RunningAuthorityLoopProgress, RunningAuthorityLoopError> {
        let step = self
            .pending_step
            .ok_or(RunningAuthorityLoopError::RetainedStateMismatch {
                field: "pending scheduler ticket",
            })?;
        if self.pending_due_steps.is_none() {
            return Err(RunningAuthorityLoopError::RetainedStateMismatch {
                field: "pending due-step count",
            });
        }
        match self.state {
            RunningAuthorityLoopState::ExternalDeliveryPending => {
                let batch = self.coordinator.pending_external_delivery().ok_or(
                    RunningAuthorityLoopError::RetainedStateMismatch {
                        field: "external delivery batch",
                    },
                )?;
                Ok(RunningAuthorityLoopProgress::ExternalDeliveryPending {
                    ticket_sequence: step.sequence(),
                    remaining: batch.remaining(),
                })
            }
            RunningAuthorityLoopState::GenerationTransitionPending => {
                let batch = self.coordinator.pending_generation_transition().ok_or(
                    RunningAuthorityLoopError::RetainedStateMismatch {
                        field: "generation transition",
                    },
                )?;
                Ok(generation_pending_progress(step, batch))
            }
            RunningAuthorityLoopState::Ready | RunningAuthorityLoopState::Faulted => {
                Err(RunningAuthorityLoopError::RetainedStateMismatch {
                    field: "blocked loop state",
                })
            }
        }
    }
}

fn generation_pending_progress(
    step: ScheduledStep,
    batch: GenerationTransitionBatch<'_>,
) -> RunningAuthorityLoopProgress {
    let successor = batch.candidate();
    RunningAuthorityLoopProgress::GenerationTransitionPending {
        ticket_sequence: step.sequence(),
        source_key: batch.source_key(),
        reason: batch.reason(),
        successor_generation: successor.generation.generation,
        successor_completed_step: successor.generation.completed_step,
    }
}

/// Terminal failure of one retained running-authority loop.
#[derive(Debug)]
pub enum RunningAuthorityLoopError {
    /// A prior error permanently ended the instance.
    AlreadyFaulted,
    /// Retained blocker and scheduler ownership became inconsistent.
    RetainedStateMismatch { field: &'static str },
    /// Rust-owned fixed-step scheduling failed.
    Scheduler(Box<SchedulerError>),
    /// Complete authoritative step staging or publication failed.
    RunningStep(Box<RunningStepError>),
    /// Optional post-publication frame packing failed.
    Frame(Box<FrameV1Error>),
}

impl Display for RunningAuthorityLoopError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyFaulted => write!(formatter, "running authority loop is faulted"),
            Self::RetainedStateMismatch { field } => {
                write!(
                    formatter,
                    "running authority loop retained-state mismatch: {field}"
                )
            }
            Self::Scheduler(error) => {
                write!(formatter, "running authority scheduler failed: {error}")
            }
            Self::RunningStep(error) => write!(formatter, "running authority step failed: {error}"),
            Self::Frame(error) => write!(formatter, "running authority frame failed: {error}"),
        }
    }
}

impl Error for RunningAuthorityLoopError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scheduler(error) => Some(error),
            Self::RunningStep(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::AlreadyFaulted | Self::RetainedStateMismatch { .. } => None,
        }
    }
}

impl From<SchedulerError> for RunningAuthorityLoopError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(Box::new(error))
    }
}

impl From<RunningStepError> for RunningAuthorityLoopError {
    fn from(error: RunningStepError) -> Self {
        Self::RunningStep(Box::new(error))
    }
}

impl From<FrameV1Error> for RunningAuthorityLoopError {
    fn from(error: FrameV1Error) -> Self {
        Self::Frame(Box::new(error))
    }
}
