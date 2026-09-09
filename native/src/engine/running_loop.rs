//! Retained Rust ownership across repeated authoritative scheduler boundaries.
//!
//! This layer joins the one-step scheduler, complete running-step coordinator,
//! and direct frame-v1 packer without introducing a per-step N-API contract.
//! The future background runtime owns one instance and supplies only its
//! monotonic clock, already-drained command boundary, presentation-only view,
//! and reusable frame storage.

use super::checkpoint::{CheckpointDescriptor, CheckpointLimits, CheckpointOperationId};
use super::display::RunningDisplayStatus;
use super::frame_v1::{
    pack_authoritative_frame_v1_into, FrameV1Error, FrameV1Metadata, FrameV1ViewDescriptor,
};
use super::generation::GenerationCommitRecord;
use super::graph::GraphLimits;
use super::physics::PhysicsStepKey;
use super::running_step::{
    ExternalDeliveryResult, ExternalDeliveryState, ExternalObservationBatch,
    GenerationReassignmentProgress, GenerationTransitionBatch, GenerationTransitionReason,
    RunningStepCoordinator, RunningStepError, RunningStepProgress,
};
use super::scheduler::{
    FixedStepScheduler, FixedStepSchedulerDiagnostics, FixedStepSchedulerPolicy, ScheduledStep,
    SchedulerError, SchedulerReadiness, SchedulerServiceMode,
};
use super::state::{AuthoritativeState, GenerationStartPublication, RunningStepPublication};
use super::step_config::RunningStepWorkLimits;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

/// Version of the retained running-authority loop contract.
pub const RUNNING_AUTHORITY_LOOP_VERSION: u32 = 1;

/// Current ability of one retained running-authority loop to accept a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunningAuthorityLoopState {
    /// The next drained command boundary may service the scheduler.
    Ready,
    /// A fresh assignment awaits delivery before its snake can become current.
    ControllerJoinPending,
    /// A same-snake reconnect assignment awaits its exact local-send result.
    ControllerReclaimPending,
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
    /// No scheduler ticket exists while a fresh assignment is unresolved.
    ControllerJoinPending,
    /// No scheduler ticket exists while a reconnect assignment is unresolved.
    ControllerReclaimPending,
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

/// Immutable managed descriptor and exact Rust-constructed generation record.
///
/// Both values originate from the retained admitted transition. The thin
/// persistence bridge may encode and commit them, but cannot supply or alter
/// generation statistics, identities, slots, or Hall-of-Fame values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGenerationCheckpointPublication {
    /// Descriptor for the immutable managed checkpoint file.
    pub descriptor: CheckpointDescriptor,
    /// Exact compact history and Hall-of-Fame reference admitted by Rust.
    pub commit_record: GenerationCommitRecord,
}

/// State reached after applying exact local-send results to a retained batch.
#[derive(Clone, Debug, PartialEq)]
pub enum RunningAuthorityDeliveryState {
    /// No retained delivery batch exists; every supplied result was stale.
    Idle,
    /// One ordinary fixed step still awaits reliable delivery results.
    RunningStepPending {
        /// Retained scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact unresolved event count.
        remaining: usize,
    },
    /// The ordinary fixed step published and its scheduler ticket retired.
    RunningStepPublished {
        /// Retained scheduler ticket identity.
        ticket_sequence: u64,
        /// Complete steps represented by debt at the original service boundary.
        due_steps: usize,
        /// Exact authority publication correlated with the ticket.
        publication: RunningStepPublication,
        /// Frame metadata when post-delivery frame packing was requested.
        frame: Option<FrameV1Metadata>,
    },
    /// Next-generation assignments still await one or more exact results.
    GenerationAssignmentsPending {
        /// Retained terminal scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact unresolved assignment count.
        remaining: usize,
    },
    /// All required generation-start assignments resolved while old authority
    /// remains current until the explicit final publication call.
    GenerationAssignmentsReady {
        /// Retained terminal scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact terminal source identity.
        source_key: PhysicsStepKey,
        /// Fully admitted successor generation identity.
        successor_generation: u64,
        /// Successor completed-step chronology.
        successor_completed_step: u64,
    },
}

/// Exact accounting and owned scalar state after delivery-result submission.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningAuthorityDeliveryResolution {
    /// Previously-unaccepted exact events accepted by this call.
    pub matched_acceptances: usize,
    /// Previously-unresolved exact events whose local send failed.
    pub matched_failures: usize,
    /// Stale, unknown, replaced, or duplicate results ignored.
    pub ignored_results: usize,
    /// Retained or published state after applying the results.
    pub state: RunningAuthorityDeliveryState,
}

/// One complete durable-boundary-to-running publication and scheduler rebind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGenerationStartResolution {
    /// Retired terminal scheduler ticket identity.
    pub ticket_sequence: u64,
    /// Complete steps represented by debt at the terminal service boundary.
    pub due_steps: usize,
    /// Exact final authority publication.
    pub publication: GenerationStartPublication,
    /// Frame metadata when post-swap frame packing was requested.
    pub frame: Option<FrameV1Metadata>,
}

/// One retained scheduler and complete-step coordinator around running authority.
#[derive(Debug)]
pub struct RunningAuthorityLoop {
    authority: AuthoritativeState,
    scheduler: FixedStepScheduler,
    coordinator: RunningStepCoordinator,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
    wall_origin_ms: u64,
    pending_step: Option<ScheduledStep>,
    pending_due_steps: Option<usize>,
    state: RunningAuthorityLoopState,
    background_clock: Option<std::time::Instant>,
    pending_reclaim: Option<(u64, super::state::PreparedControllerReclaim)>,
    pending_join: Option<(u64, super::state::PreparedControllerJoin)>,
    work_limits: RunningStepWorkLimits,
}

/// Fallible scheduler/coordinator construction completed before authority moves.
pub(crate) struct PreparedRunningAuthorityLoop {
    work_limits: RunningStepWorkLimits,
    scheduler: FixedStepScheduler,
    coordinator: RunningStepCoordinator,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
    wall_origin_ms: u64,
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
        checkpoint_limits: &CheckpointLimits,
        graph_limits: &GraphLimits,
    ) -> Result<PreparedRunningAuthorityLoop, RunningAuthorityLoopError> {
        let coordinator = RunningStepCoordinator::try_new(authority, work_limits)?;
        let mut scheduler = FixedStepScheduler::try_new(authority, policy)?;
        scheduler.reset_wall_clock(authority, wall_origin_ms)?;
        Ok(PreparedRunningAuthorityLoop {
            work_limits,
            scheduler,
            coordinator,
            checkpoint_limits: checkpoint_limits.clone(),
            graph_limits: graph_limits.clone(),
            wall_origin_ms,
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
            checkpoint_limits: prepared.checkpoint_limits,
            graph_limits: prepared.graph_limits,
            wall_origin_ms: prepared.wall_origin_ms,
            pending_step: None,
            pending_due_steps: None,
            state: RunningAuthorityLoopState::Ready,
            background_clock: None,
            pending_reclaim: None,
            pending_join: None,
            work_limits: prepared.work_limits,
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
            RunningAuthorityLoopState::ControllerJoinPending => {
                return Ok(RunningAuthorityLoopProgress::ControllerJoinPending);
            }
            RunningAuthorityLoopState::ControllerReclaimPending => {
                return Ok(RunningAuthorityLoopProgress::ControllerReclaimPending);
            }
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

    /// Bind command receipt timestamps to the same clock as background scheduling.
    pub(crate) fn set_background_clock(&mut self, origin: std::time::Instant) {
        self.background_clock = Some(origin);
    }

    /// Output reservation and the fresh-boundary gate precede this lease write.
    pub(crate) fn apply_controller_action(
        &mut self,
        sequence: u64,
        action: &super::contract::ControllerActionRequest,
        wall_now_ms: u64,
    ) -> Result<(), String> {
        if self.state != RunningAuthorityLoopState::Ready {
            return Err("controller action requires an unprepared step boundary".to_owned());
        }
        let accepted_at_ms = self.controller_receipt_ms(action.received_at)?;
        self.authority.apply_controller_action(
            super::controllers::LatestActionInput {
                lease_id: action.lease_id,
                connection_id: action.connection_id,
                turn: action.turn,
                boost: action.boost,
                client_tick: action.client_tick,
                arrival_sequence: sequence,
                accepted_at_ms,
            },
            wall_now_ms,
        )
    }

    /// Prepare one fresh assignment only after the full reliable output fits.
    pub(crate) fn prepare_controller_join(
        &mut self,
        request_sequence: u64,
        request: &super::contract::ControllerJoinRequest,
        wall_now_ms: u64,
    ) -> Result<super::contract::RunningAuthorityEvent, String> {
        if self.state != RunningAuthorityLoopState::Ready {
            return Err("join requires an unprepared source boundary".into());
        }
        let token = super::external_replacement::fresh_resume_token(
            &self.authority.state().world.controller_leases,
        )
        .map_err(|error| error.to_string())?;
        let input = super::state::ControllerJoinInput {
            kind: request.kind,
            identity_key: request.identity_key.clone(),
            resume_token: token,
            connection_id: request.connection_id,
            arrival_sequence: request_sequence,
            received_at_ms: self.controller_receipt_ms(request.received_at)?,
            boundary_at_ms: wall_now_ms,
        };
        let prepared = self
            .authority
            .prepare_controller_join(input, self.work_limits)?;
        let event = super::contract::RunningAuthorityEvent::ControllerJoinAssignment {
            request_sequence,
            controller_kind: request.kind,
            connection_id: prepared.connection_id(),
            lease_id: prepared.lease_id(),
            frame_v1_id: prepared.frame_v1_id(),
            completed_step: self.completed_step(),
            resume_token: prepared.resume_token().into(),
        };
        self.pending_join = Some((request_sequence, prepared));
        self.state = RunningAuthorityLoopState::ControllerJoinPending;
        Ok(event)
    }

    /// Failed sends discard only detached buffers; stale receipts touch no barrier.
    pub(crate) fn resolve_controller_join(
        &mut self,
        receipt: &super::contract::ControllerReclaimReceipt,
    ) -> Result<bool, String> {
        let Some((sequence, prepared)) = self.pending_join.as_ref() else {
            return Ok(false);
        };
        if self.state != RunningAuthorityLoopState::ControllerJoinPending
            || *sequence != receipt.request_sequence
            || prepared.connection_id() != receipt.connection_id
            || prepared.lease_id() != receipt.lease_id
        {
            return Ok(false);
        }
        if receipt.accepted {
            self.authority.validate_controller_join(prepared)?;
        }
        let (_, prepared) = self.pending_join.take().expect("matched join candidate");
        if receipt.accepted {
            self.authority.commit_controller_join(prepared)?;
        }
        self.state = RunningAuthorityLoopState::Ready;
        Ok(true)
    }

    /// Stage one same-snake assignment only after the complete output reservation.
    pub(crate) fn prepare_controller_reclaim(
        &mut self,
        request_sequence: u64,
        request: &super::contract::ControllerReclaimRequest,
        wall_now_ms: u64,
    ) -> Result<super::contract::RunningAuthorityEvent, String> {
        if self.state != RunningAuthorityLoopState::Ready {
            return Err("reclaim requires an unprepared source boundary".into());
        }
        let source = self.authority.state();
        let token =
            super::external_replacement::fresh_resume_token(&source.world.controller_leases)
                .map_err(|error| error.to_string())?;
        let input = super::controllers::ReclaimInput {
            kind: request.kind,
            scope: &source.identity.run_id,
            resume_token: &request.resume_token,
            next_resume_token: token,
            connection_id: request.connection_id,
            arrival_sequence: request_sequence,
            received_at_ms: self.controller_receipt_ms(request.received_at)?,
            boundary_at_ms: wall_now_ms,
        };
        let prepared = if request.resume_token.is_empty() {
            self.authority
                .prepare_legacy_controller_reclaim(&request.identity_key, input)?
                .ok_or_else(|| "no reserved legacy identity match".to_owned())?
        } else {
            self.authority.prepare_controller_reclaim(input)?
        };
        let event = super::contract::RunningAuthorityEvent::ControllerReclaimAssignment {
            request_sequence,
            controller_kind: request.kind,
            connection_id: prepared.connection_id(),
            lease_id: prepared.lease_id(),
            frame_v1_id: prepared.frame_v1_id(),
            completed_step: self.completed_step(),
            resume_token: prepared.resume_token().into(),
        };
        self.pending_reclaim = Some((request_sequence, prepared));
        self.state = RunningAuthorityLoopState::ControllerReclaimPending;
        Ok(event)
    }

    /// Ignore stale/cross-phase receipts; a failed local send preserves the old
    /// token and lease. The caller reserves its result before any commit.
    pub(crate) fn resolve_controller_reclaim(
        &mut self,
        receipt: &super::contract::ControllerReclaimReceipt,
    ) -> Result<bool, String> {
        let Some((sequence, prepared)) = self.pending_reclaim.as_ref() else {
            return Ok(false);
        };
        if self.state != RunningAuthorityLoopState::ControllerReclaimPending
            || *sequence != receipt.request_sequence
            || prepared.connection_id() != receipt.connection_id
            || prepared.lease_id() != receipt.lease_id
        {
            return Ok(false);
        }
        if receipt.accepted {
            self.authority.commit_controller_reclaim(prepared.clone())?;
        }
        self.pending_reclaim = None;
        self.state = RunningAuthorityLoopState::Ready;
        Ok(true)
    }

    /// Convert a transport receipt to the scheduler's one elapsed-time domain.
    fn controller_receipt_ms(&self, received_at: std::time::Instant) -> Result<u64, String> {
        let origin = self
            .background_clock
            .ok_or_else(|| "background clock is missing".to_owned())?;
        u64::try_from(
            received_at
                .checked_duration_since(origin)
                .unwrap_or_default()
                .as_millis(),
        )
        .map_err(|_| "controller clock overflow".to_owned())
    }

    /// Preserve the staged step and apply a correlated close only after it retires.
    pub(crate) fn disconnect_controller(
        &mut self,
        close: &super::contract::ControllerDisconnectRequest,
        wall_now_ms: u64,
    ) -> Result<bool, String> {
        if self.state != RunningAuthorityLoopState::Ready {
            return Err("controller disconnect requires an unprepared step boundary".to_owned());
        }
        let disconnected_at_ms = self.controller_receipt_ms(close.received_at)?;
        self.authority.disconnect_controller(
            close.lease_id,
            close.connection_id,
            disconnected_at_ms,
            wall_now_ms,
        )
    }

    /// Current authoritative generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.authority.state().generation.generation
    }

    /// Current process-local authoritative world incarnation.
    #[must_use]
    pub fn world_epoch(&self) -> u64 {
        self.authority.world_epoch()
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

    /// Read only the published authority and never grow the admitted display buffer.
    pub(crate) fn pack_display_into(
        &self,
        sequence: u64,
        output: &mut Vec<u8>,
    ) -> Result<RunningDisplayStatus, FrameV1Error> {
        let frame = super::frame_v1::pack_authoritative_frame_v1_bounded_into(
            &self.authority,
            FrameV1ViewDescriptor::default(),
            output,
            output.capacity(),
        )?;
        let state = self.authority.state();
        let mut alive_population = 0;
        let mut baseline_bots_alive = 0;
        for snake in &state.world.snakes {
            if snake.alive {
                alive_population += usize::from(snake.population_slot.is_some());
                baseline_bots_alive += usize::from(snake.baseline_slot.is_some());
            }
        }
        Ok(RunningDisplayStatus {
            sequence,
            world_epoch: self.world_epoch(),
            completed_step: self.completed_step(),
            generation_time: state.generation.elapsed_seconds,
            alive_population,
            baseline_bots_alive,
            baseline_bots_total: state.config.baseline_count,
            frame,
        })
    }

    /// Total admitted authoritative-state bytes as bounded runtime metadata.
    #[must_use]
    pub fn authoritative_memory_bytes(&self) -> usize {
        self.authority.memory_estimate().total_bytes
    }

    /// Current operational scheduler diagnostics.
    #[must_use]
    pub fn scheduler_diagnostics(&self) -> FixedStepSchedulerDiagnostics {
        self.scheduler.diagnostics()
    }

    /// Borrow the exact retained external batch for future reliable routing.
    #[must_use]
    pub fn pending_external_delivery(&self) -> Option<ExternalObservationBatch<'_>> {
        if !matches!(
            self.state,
            RunningAuthorityLoopState::ExternalDeliveryPending
                | RunningAuthorityLoopState::GenerationTransitionPending
        ) {
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

    /// Borrow only the retained source controller records for queue admission.
    /// These remain private Rust data and cannot be supplied by a bridge caller.
    pub(crate) fn generation_source_controller_leases(&self) -> &[super::state::ControllerLease] {
        &self.authority.state().world.controller_leases
    }

    /// Resolve an observed internal ID using the still-current source authority.
    pub(crate) fn controller_frame_v1_id(&self, snake_id: u64) -> Option<u32> {
        self.authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == snake_id)
            .map(|snake| snake.frame_v1_id)
    }

    /// Publish or exactly retry the immutable checkpoint and return only its
    /// bounded descriptor plus the Rust-admitted scalar commit record.
    ///
    /// File or validation failure leaves the old authority, scheduler ticket,
    /// admitted successor, and any earlier exact publication unchanged.
    pub fn publish_pending_generation_checkpoint(
        &mut self,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
    ) -> Result<RunningGenerationCheckpointPublication, RunningAuthorityLoopError> {
        self.require_action_state(
            "publish a generation checkpoint",
            RunningAuthorityLoopState::GenerationTransitionPending,
        )?;
        self.validate_retained_step()?;
        let descriptor = self.coordinator.publish_pending_generation_checkpoint(
            &self.authority,
            managed_directory,
            operation_id,
            &self.checkpoint_limits,
            &self.graph_limits,
        )?;
        let commit_record = self
            .coordinator
            .pending_generation_transition()
            .ok_or(RunningAuthorityLoopError::RetainedStateMismatch {
                field: "generation transition after checkpoint publication",
            })?
            .commit_record()
            .to_owned();
        Ok(RunningGenerationCheckpointPublication {
            descriptor,
            commit_record,
        })
    }

    /// Retain only the exact complete descriptor committed by the SQLite
    /// worker, then prepare or reborrow the deterministic successor world.
    ///
    /// A premature or mismatched acknowledgement is recoverable and changes no
    /// authority. Once an exact acknowledgement is accepted it remains retained
    /// even when bounded successor construction returns an error for retry.
    pub fn acknowledge_pending_generation_persistence(
        &mut self,
        committed: &CheckpointDescriptor,
    ) -> Result<(), RunningAuthorityLoopError> {
        self.require_action_state(
            "acknowledge generation persistence",
            RunningAuthorityLoopState::GenerationTransitionPending,
        )?;
        self.validate_retained_step()?;
        let _prepared = self
            .coordinator
            .acknowledge_pending_generation_persistence(&self.authority, committed)?;
        Ok(())
    }

    /// Stage or reborrow every reliable connected-controller reassignment after
    /// exact persistence acknowledgement while keeping old authority current.
    pub fn prepare_acknowledged_generation_reassignments(
        &mut self,
    ) -> Result<GenerationReassignmentProgress<'_>, RunningAuthorityLoopError> {
        self.require_action_state(
            "prepare generation controller reassignments",
            RunningAuthorityLoopState::GenerationTransitionPending,
        )?;
        self.validate_retained_step()?;
        Ok(self
            .coordinator
            .prepare_acknowledged_generation_reassignments(&self.authority)?)
    }

    /// Apply exact local-send results to the one retained ordinary-step or
    /// generation-assignment batch.
    ///
    /// Stale and mismatched results are counted and ignored by the coordinator.
    /// An ordinary step retires its scheduler ticket only after every event
    /// resolves. Generation assignments never swap authority here.
    pub fn submit_external_delivery_results(
        &mut self,
        results: &[ExternalDeliveryResult],
        frame: Option<RunningFramePublication<'_>>,
    ) -> Result<RunningAuthorityDeliveryResolution, RunningAuthorityLoopError> {
        let blocked_state = self.state;
        if blocked_state == RunningAuthorityLoopState::Faulted {
            return Err(RunningAuthorityLoopError::AlreadyFaulted);
        }
        if blocked_state == RunningAuthorityLoopState::Ready
            || blocked_state == RunningAuthorityLoopState::ControllerReclaimPending
            || blocked_state == RunningAuthorityLoopState::ControllerJoinPending
            || (blocked_state == RunningAuthorityLoopState::GenerationTransitionPending
                && self.coordinator.pending_external_delivery().is_none())
        {
            return Ok(RunningAuthorityDeliveryResolution {
                matched_acceptances: 0,
                matched_failures: 0,
                ignored_results: results.len(),
                state: RunningAuthorityDeliveryState::Idle,
            });
        }
        let step = match self.pending_step {
            Some(step) => step,
            None => {
                self.state = RunningAuthorityLoopState::Faulted;
                return Err(RunningAuthorityLoopError::RetainedStateMismatch {
                    field: "pending scheduler ticket",
                });
            }
        };
        let due_steps = match self.pending_due_steps {
            Some(due_steps) => due_steps,
            None => {
                self.state = RunningAuthorityLoopState::Faulted;
                return Err(RunningAuthorityLoopError::RetainedStateMismatch {
                    field: "pending due-step count",
                });
            }
        };
        let resolution = match self
            .coordinator
            .submit_external_delivery_results(&mut self.authority, results)
        {
            Ok(resolution) => resolution,
            Err(error) => {
                self.state = RunningAuthorityLoopState::Faulted;
                return Err(error.into());
            }
        };
        let matched_acceptances = resolution.matched_acceptances;
        let matched_failures = resolution.matched_failures;
        let ignored_results = resolution.ignored_results;
        let state = match resolution.state {
            ExternalDeliveryState::Pending(batch) => match blocked_state {
                RunningAuthorityLoopState::ExternalDeliveryPending => {
                    RunningAuthorityDeliveryState::RunningStepPending {
                        ticket_sequence: step.sequence(),
                        remaining: batch.remaining(),
                    }
                }
                RunningAuthorityLoopState::GenerationTransitionPending => {
                    RunningAuthorityDeliveryState::GenerationAssignmentsPending {
                        ticket_sequence: step.sequence(),
                        remaining: batch.remaining(),
                    }
                }
                RunningAuthorityLoopState::Ready
                | RunningAuthorityLoopState::ControllerReclaimPending
                | RunningAuthorityLoopState::ControllerJoinPending
                | RunningAuthorityLoopState::Faulted => {
                    unreachable!("blocked state was checked before delivery submission")
                }
            },
            ExternalDeliveryState::Published(outcome) => {
                if blocked_state != RunningAuthorityLoopState::ExternalDeliveryPending {
                    self.state = RunningAuthorityLoopState::Faulted;
                    return Err(RunningAuthorityLoopError::RetainedStateMismatch {
                        field: "ordinary publication during generation assignment",
                    });
                }
                let publication = outcome.publication;
                if let Err(error) = self
                    .scheduler
                    .commit_step(&self.authority, step, publication)
                {
                    self.state = RunningAuthorityLoopState::Faulted;
                    return Err(error.into());
                }
                self.pending_step = None;
                self.pending_due_steps = None;
                let frame = match frame {
                    Some(publication_request) => {
                        match pack_authoritative_frame_v1_into(
                            &self.authority,
                            publication_request.view,
                            publication_request.output,
                        ) {
                            Ok(metadata) => Some(metadata),
                            Err(error) => {
                                self.state = RunningAuthorityLoopState::Faulted;
                                return Err(error.into());
                            }
                        }
                    }
                    None => None,
                };
                self.state = RunningAuthorityLoopState::Ready;
                RunningAuthorityDeliveryState::RunningStepPublished {
                    ticket_sequence: step.sequence(),
                    due_steps,
                    publication,
                    frame,
                }
            }
            ExternalDeliveryState::GenerationAssignmentsReady(batch) => {
                if blocked_state != RunningAuthorityLoopState::GenerationTransitionPending {
                    self.state = RunningAuthorityLoopState::Faulted;
                    return Err(RunningAuthorityLoopError::RetainedStateMismatch {
                        field: "generation assignment completion during ordinary step",
                    });
                }
                let successor = batch.candidate();
                RunningAuthorityDeliveryState::GenerationAssignmentsReady {
                    ticket_sequence: step.sequence(),
                    source_key: batch.source_key(),
                    successor_generation: successor.generation.generation,
                    successor_completed_step: successor.generation.completed_step,
                }
            }
            ExternalDeliveryState::Idle => {
                if blocked_state == RunningAuthorityLoopState::GenerationTransitionPending {
                    RunningAuthorityDeliveryState::Idle
                } else {
                    self.state = RunningAuthorityLoopState::Faulted;
                    return Err(RunningAuthorityLoopError::RetainedStateMismatch {
                        field: "delivery blocker disappeared",
                    });
                }
            }
        };
        Ok(RunningAuthorityDeliveryResolution {
            matched_acceptances,
            matched_failures,
            ignored_results,
            state,
        })
    }

    /// Perform the one final successor authority swap, retire the retained
    /// terminal scheduler ticket, and rebind the same coordinator in place.
    ///
    /// `resume_wall_now_ms` must be sampled after persistence and assignment
    /// work. Scheduler debt begins again at that boundary, excluding the wait.
    /// Premature barrier calls return without changing authority. Any failure
    /// after the already-preflighted swap faults the loop rather than risking a
    /// second publication.
    pub fn publish_acknowledged_generation_start(
        &mut self,
        resume_wall_now_ms: u64,
        frame: Option<RunningFramePublication<'_>>,
    ) -> Result<RunningGenerationStartResolution, RunningAuthorityLoopError> {
        self.require_action_state(
            "publish an acknowledged generation start",
            RunningAuthorityLoopState::GenerationTransitionPending,
        )?;
        let step = self.validate_retained_step()?;
        let due_steps = self
            .pending_due_steps
            .expect("validated retained step must include due-step count");
        if resume_wall_now_ms < step.wall_now_ms() {
            return Err(SchedulerError::RegressingWallClock {
                previous_ms: step.wall_now_ms(),
                actual_ms: resume_wall_now_ms,
            }
            .into());
        }

        let publication = self
            .coordinator
            .publish_acknowledged_generation_start(&mut self.authority)?;
        let prepared_rebind = match self
            .coordinator
            .prepare_published_generation_rebind(&self.authority, &publication)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                self.state = RunningAuthorityLoopState::Faulted;
                return Err(error.into());
            }
        };
        if let Err(error) = self.scheduler.commit_generation_transition(
            &self.authority,
            step,
            &publication,
            resume_wall_now_ms,
        ) {
            self.state = RunningAuthorityLoopState::Faulted;
            return Err(error.into());
        }
        self.coordinator
            .commit_published_generation_rebind(prepared_rebind);
        self.pending_step = None;
        self.pending_due_steps = None;
        let frame = match frame {
            Some(publication_request) => match pack_authoritative_frame_v1_into(
                &self.authority,
                publication_request.view,
                publication_request.output,
            ) {
                Ok(metadata) => Some(metadata),
                Err(error) => {
                    self.state = RunningAuthorityLoopState::Faulted;
                    return Err(error.into());
                }
            },
            None => None,
        };
        self.state = RunningAuthorityLoopState::Ready;
        Ok(RunningGenerationStartResolution {
            ticket_sequence: step.sequence(),
            due_steps,
            publication,
            frame,
        })
    }

    /// Verify this unserviced loop can adopt a fresh thread-local clock.
    ///
    /// The background coordinator measures elapsed monotonic time from zero at
    /// its actual thread root. Requiring the matching prepared origin excludes
    /// asynchronous construction and thread-spawn delay from scheduler debt.
    pub(crate) fn validate_background_start(&self) -> Result<(), RunningAuthorityLoopError> {
        let diagnostics = self.scheduler.diagnostics();
        if self.wall_origin_ms != 0 {
            return Err(RunningAuthorityLoopError::InvalidBackgroundStart {
                field: "wall-clock origin",
            });
        }
        if self.state != RunningAuthorityLoopState::Ready {
            return Err(RunningAuthorityLoopError::InvalidBackgroundStart {
                field: "loop state",
            });
        }
        if self.pending_step.is_some()
            || self.pending_due_steps.is_some()
            || diagnostics.step_pending
        {
            return Err(RunningAuthorityLoopError::InvalidBackgroundStart {
                field: "pending scheduler work",
            });
        }
        if diagnostics.completed_steps != 0 || diagnostics.command_service_boundaries != 0 {
            return Err(RunningAuthorityLoopError::InvalidBackgroundStart {
                field: "prior scheduler service",
            });
        }
        Ok(())
    }

    fn require_action_state(
        &self,
        action: &'static str,
        required: RunningAuthorityLoopState,
    ) -> Result<(), RunningAuthorityLoopError> {
        if self.state == required {
            return Ok(());
        }
        Err(RunningAuthorityLoopError::InvalidActionState {
            action,
            required: match required {
                RunningAuthorityLoopState::Ready => "ready state",
                RunningAuthorityLoopState::ControllerReclaimPending
                | RunningAuthorityLoopState::ControllerJoinPending => {
                    "controller-reclaim-pending state"
                }
                RunningAuthorityLoopState::ExternalDeliveryPending => {
                    "external-delivery-pending state"
                }
                RunningAuthorityLoopState::GenerationTransitionPending => {
                    "generation-transition-pending state"
                }
                RunningAuthorityLoopState::Faulted => "faulted state",
            },
            actual: self.state,
        })
    }

    fn validate_retained_step(&self) -> Result<ScheduledStep, RunningAuthorityLoopError> {
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
        Ok(step)
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
            RunningAuthorityLoopState::Ready
            | RunningAuthorityLoopState::ControllerReclaimPending
            | RunningAuthorityLoopState::ControllerJoinPending
            | RunningAuthorityLoopState::Faulted => {
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
    /// An action was requested outside its retained barrier phase.
    InvalidActionState {
        action: &'static str,
        required: &'static str,
        actual: RunningAuthorityLoopState,
    },
    /// Retained blocker and scheduler ownership became inconsistent.
    RetainedStateMismatch { field: &'static str },
    /// The loop was already serviced or prepared against another clock origin.
    InvalidBackgroundStart { field: &'static str },
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
            Self::InvalidActionState {
                action,
                required,
                actual,
            } => write!(
                formatter,
                "cannot {action}: requires {required}, current state is {actual:?}"
            ),
            Self::RetainedStateMismatch { field } => {
                write!(
                    formatter,
                    "running authority loop retained-state mismatch: {field}"
                )
            }
            Self::InvalidBackgroundStart { field } => {
                write!(
                    formatter,
                    "running authority loop cannot enter background runtime: {field}"
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
            Self::AlreadyFaulted
            | Self::InvalidActionState { .. }
            | Self::RetainedStateMismatch { .. }
            | Self::InvalidBackgroundStart { .. } => None,
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
