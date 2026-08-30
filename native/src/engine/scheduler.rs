//! Rust-owned fixed-step scheduling and bounded wall-debt admission.
//!
//! This module deliberately produces at most one fixed-step ticket after one
//! explicit command-service boundary. It never batches authoritative steps,
//! changes the fixed delta, or mutates world state. The complete running-step
//! coordinator consumes the ticket, and the scheduler retires it only after
//! the matching authority publication succeeds.

use super::running_step::RunningStepInputs;
use super::state::{
    AuthoritativeState, AuthorityPhase, GenerationStartPublication, RunningStepPublication,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First bounded one-step-at-a-time scheduler contract.
pub const FIXED_STEP_SCHEDULER_VERSION: u32 = 1;
/// Hard safety ceiling for a configured catch-up horizon.
const MAXIMUM_CATCH_UP_HORIZON_SECONDS: f64 = 60.0;

/// Operational scheduler policy kept outside experiment/gameplay identity.
///
/// The default is provisional until the approved P0-P3 target measurements.
/// Changing this value changes only which old scheduling requests are dropped;
/// it never changes an admitted fixed step or its physics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedStepSchedulerPolicy {
    /// Versioned scheduling/debt algorithm.
    pub algorithm_version: u32,
    /// Maximum real-wall catch-up backlog retained after the next eligible step.
    pub catch_up_horizon_seconds: f64,
}

impl FixedStepSchedulerPolicy {
    /// Approved-plan provisional catch-up horizon.
    #[must_use]
    pub const fn provisional_defaults() -> Self {
        Self {
            algorithm_version: FIXED_STEP_SCHEDULER_VERSION,
            catch_up_horizon_seconds: 0.250,
        }
    }

    fn validate(self) -> Result<(), SchedulerError> {
        if self.algorithm_version != FIXED_STEP_SCHEDULER_VERSION {
            return Err(SchedulerError::InvalidPolicy {
                field: "algorithm version",
            });
        }
        if !self.catch_up_horizon_seconds.is_finite()
            || self.catch_up_horizon_seconds < 0.0
            || self.catch_up_horizon_seconds > MAXIMUM_CATCH_UP_HORIZON_SECONDS
        {
            return Err(SchedulerError::InvalidPolicy {
                field: "catch-up horizon",
            });
        }
        Ok(())
    }
}

/// Whether the just-serviced boundary has an interactive browser player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerServiceMode {
    /// No browser player currently owns a snake.
    Background,
    /// A browser player currently owns a snake and must not face a catch-up burst.
    Interactive,
}

/// Result after the inbound command/action queue was serviced once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SchedulerReadiness {
    /// No complete fixed step is requested yet.
    Idle {
        /// Remaining simulated seconds before one complete step is eligible.
        simulation_seconds_until_step: f64,
        /// Equivalent real-wall duration at the admitted simulation multiplier.
        wall_seconds_until_step: f64,
    },
    /// Exactly one complete fixed-step ticket may now be prepared.
    StepDue {
        /// Complete steps represented by current retained debt.
        due_steps: usize,
        /// Real-wall backlog remaining after the next eligible step.
        catch_up_wall_debt_seconds: f64,
    },
}

/// One exact fixed-step scheduling ticket.
///
/// A ticket is single-use. Its remaining debt becomes authoritative only when
/// the matching complete running-step publication succeeds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledStep {
    sequence: u64,
    source_completed_step: u64,
    wall_now_ms: u64,
    wall_accumulator_after_step: f64,
    service_mode: SchedulerServiceMode,
}

impl ScheduledStep {
    /// Monotonic process-local scheduler ticket sequence.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Complete authoritative step from which this ticket starts.
    #[must_use]
    pub const fn source_completed_step(self) -> u64 {
        self.source_completed_step
    }

    /// Wall-clock boundary retained for controller lease evaluation.
    #[must_use]
    pub const fn wall_now_ms(self) -> u64 {
        self.wall_now_ms
    }

    /// Remaining simulated scheduling debt if this step publishes.
    #[must_use]
    pub const fn wall_accumulator_after_step(self) -> f64 {
        self.wall_accumulator_after_step
    }

    /// Controller/service mode observed immediately before this step.
    #[must_use]
    pub const fn service_mode(self) -> SchedulerServiceMode {
        self.service_mode
    }

    /// Exact inputs consumed by the complete running-step coordinator.
    #[must_use]
    pub const fn running_step_inputs(self) -> RunningStepInputs {
        RunningStepInputs {
            wall_now_ms: self.wall_now_ms,
            wall_accumulator_seconds: self.wall_accumulator_after_step,
        }
    }
}

/// Current operational scheduler measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FixedStepSchedulerDiagnostics {
    /// Fixed authoritative delta; it never scales with `simSpeed`.
    pub fixed_step_seconds: f64,
    /// Admitted requested simulation multiplier.
    pub requested_multiplier: f64,
    /// Simulated seconds currently requested but not yet committed.
    pub pending_simulation_seconds: f64,
    /// Real-wall equivalent of all retained scheduling debt.
    pub pending_wall_seconds: f64,
    /// Real-wall catch-up debt after reserving the next eligible step.
    pub catch_up_wall_debt_seconds: f64,
    /// Maximum real-wall scheduling debt observed by this scheduler instance.
    pub maximum_wall_debt_seconds: f64,
    /// Wall seconds observed since the scheduler clock was initialized.
    pub observed_wall_seconds: f64,
    /// Simulated seconds committed through this scheduler instance.
    pub completed_simulation_seconds: f64,
    /// Completed steps committed through this scheduler instance.
    pub completed_steps: u64,
    /// Lifetime achieved simulated-seconds per observed wall-second.
    pub achieved_multiplier: f64,
    /// Scheduling debt dropped in simulated seconds over this instance.
    pub dropped_simulation_seconds: f64,
    /// Scheduling debt dropped in real-wall seconds over this instance.
    pub dropped_wall_seconds: f64,
    /// Simulated scheduling debt dropped at the latest wall observation.
    pub dropped_simulation_seconds_latest: f64,
    /// Real-wall scheduling debt dropped at the latest wall observation.
    pub dropped_wall_seconds_latest: f64,
    /// Explicit command/action service boundaries observed.
    pub command_service_boundaries: u64,
    /// Service boundaries at which an interactive player was present.
    pub interactive_service_boundaries: u64,
    /// Whether one exact step ticket is awaiting success or rejection.
    pub step_pending: bool,
    /// Whether discarded debt has left a retained catch-up backlog to drain.
    pub overloaded: bool,
}

/// Rust-owned one-step-at-a-time scheduler bound to one authority incarnation.
#[derive(Debug)]
pub struct FixedStepScheduler {
    policy: FixedStepSchedulerPolicy,
    world_epoch: u64,
    config_revision: u64,
    config_hash: String,
    fixed_step_seconds: f64,
    requested_multiplier: f64,
    expected_completed_step: u64,
    accumulator_seconds: f64,
    last_wall_now_ms: Option<u64>,
    next_ticket_sequence: u64,
    serviced_mode: Option<SchedulerServiceMode>,
    pending_step: Option<ScheduledStep>,
    observed_wall_seconds: f64,
    completed_simulation_seconds: f64,
    completed_steps: u64,
    dropped_simulation_seconds: f64,
    dropped_wall_seconds: f64,
    dropped_simulation_seconds_latest: f64,
    dropped_wall_seconds_latest: f64,
    overload_active: bool,
    maximum_wall_debt_seconds: f64,
    command_service_boundaries: u64,
    interactive_service_boundaries: u64,
}

impl FixedStepScheduler {
    /// Bind a scheduler to one admitted running authority.
    ///
    /// The first wall-clock observation initializes the clock without treating
    /// asynchronous process startup as debt. A Reset, New Run, import, config
    /// replacement, or separately admitted authority requires a new scheduler.
    pub fn try_new(
        authority: &AuthoritativeState,
        policy: FixedStepSchedulerPolicy,
    ) -> Result<Self, SchedulerError> {
        policy.validate()?;
        let state = authority.state();
        if state.phase != AuthorityPhase::Running {
            return Err(SchedulerError::AuthorityMismatch { field: "phase" });
        }
        let fixed_step_seconds = state.config.fixed_step_seconds;
        let requested_multiplier = state.config.requested_sim_speed;
        let accumulator_seconds = state.generation.wall_accumulator_seconds;
        if !fixed_step_seconds.is_finite() || fixed_step_seconds <= 0.0 {
            return Err(SchedulerError::InvalidAuthorityScalar {
                field: "fixed step",
            });
        }
        if !requested_multiplier.is_finite() || requested_multiplier <= 0.0 {
            return Err(SchedulerError::InvalidAuthorityScalar {
                field: "simulation multiplier",
            });
        }
        if !accumulator_seconds.is_finite() || accumulator_seconds < 0.0 {
            return Err(SchedulerError::InvalidAuthorityScalar {
                field: "wall accumulator",
            });
        }
        Ok(Self {
            policy,
            world_epoch: authority.world_epoch(),
            config_revision: state.identity.config_revision,
            config_hash: state.identity.config_hash.clone(),
            fixed_step_seconds,
            requested_multiplier,
            expected_completed_step: state.generation.completed_step,
            accumulator_seconds,
            last_wall_now_ms: None,
            next_ticket_sequence: 1,
            serviced_mode: None,
            pending_step: None,
            observed_wall_seconds: 0.0,
            completed_simulation_seconds: 0.0,
            completed_steps: 0,
            dropped_simulation_seconds: 0.0,
            dropped_wall_seconds: 0.0,
            dropped_simulation_seconds_latest: 0.0,
            dropped_wall_seconds_latest: 0.0,
            overload_active: false,
            maximum_wall_debt_seconds: accumulator_seconds / requested_multiplier,
            command_service_boundaries: 0,
            interactive_service_boundaries: 0,
        })
    }

    /// Set the wall-clock origin once after asynchronous initialization.
    ///
    /// Existing fractional debt is retained and no elapsed startup duration is
    /// added. Reusing this operation would hide or manufacture wall debt, so a
    /// lifecycle replacement must construct a new scheduler instead.
    pub fn reset_wall_clock(
        &mut self,
        authority: &AuthoritativeState,
        wall_now_ms: u64,
    ) -> Result<(), SchedulerError> {
        self.validate_authority(authority)?;
        if self.pending_step.is_some() {
            return Err(SchedulerError::StepPending);
        }
        if self.last_wall_now_ms.is_some() {
            return Err(SchedulerError::ClockAlreadyInitialized);
        }
        self.last_wall_now_ms = Some(wall_now_ms);
        self.serviced_mode = None;
        self.dropped_simulation_seconds_latest = 0.0;
        self.dropped_wall_seconds_latest = 0.0;
        Ok(())
    }

    /// Record one explicit inbound command/action service boundary.
    ///
    /// The caller invokes this only after draining commands and newest actions.
    /// At most one step can become available from this service call. A second
    /// overdue step requires another call, which gives the bridge/Node path a
    /// service opportunity rather than executing an uninterrupted catch-up
    /// burst.
    pub fn service_after_command_drain(
        &mut self,
        authority: &AuthoritativeState,
        wall_now_ms: u64,
        mode: SchedulerServiceMode,
    ) -> Result<SchedulerReadiness, SchedulerError> {
        self.validate_authority(authority)?;
        if self.pending_step.is_some() {
            return Err(SchedulerError::StepPending);
        }
        let command_service_boundaries = self.command_service_boundaries.checked_add(1).ok_or(
            SchedulerError::ArithmeticOverflow {
                context: "command service count",
            },
        )?;
        let interactive_service_boundaries = if mode == SchedulerServiceMode::Interactive {
            self.interactive_service_boundaries.checked_add(1).ok_or(
                SchedulerError::ArithmeticOverflow {
                    context: "interactive service count",
                },
            )?
        } else {
            self.interactive_service_boundaries
        };
        self.observe_wall_clock(wall_now_ms)?;
        self.command_service_boundaries = command_service_boundaries;
        self.interactive_service_boundaries = interactive_service_boundaries;

        let due_steps = self.due_steps();
        if due_steps == 0 {
            self.serviced_mode = None;
            let simulation_seconds_until_step =
                (self.fixed_step_seconds - self.accumulator_seconds).max(0.0);
            return Ok(SchedulerReadiness::Idle {
                simulation_seconds_until_step,
                wall_seconds_until_step: simulation_seconds_until_step / self.requested_multiplier,
            });
        }
        self.serviced_mode = Some(mode);
        Ok(SchedulerReadiness::StepDue {
            due_steps,
            catch_up_wall_debt_seconds: self.catch_up_wall_debt_seconds(),
        })
    }

    /// Prepare the sole step authorized by the latest command-service boundary.
    pub fn prepare_due_step(
        &mut self,
        authority: &AuthoritativeState,
    ) -> Result<ScheduledStep, SchedulerError> {
        self.validate_authority(authority)?;
        if self.pending_step.is_some() {
            return Err(SchedulerError::StepPending);
        }
        let service_mode = self
            .serviced_mode
            .ok_or(SchedulerError::CommandServiceRequired)?;
        if self.due_steps() == 0 {
            return Err(SchedulerError::StepNotDue);
        }
        let sequence = self.next_ticket_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(SchedulerError::ArithmeticOverflow {
                context: "scheduler ticket sequence",
            })?;
        let allowance = self.fixed_step_seconds * 1.0e-9;
        let remaining = if self.accumulator_seconds <= self.fixed_step_seconds + allowance {
            0.0
        } else {
            self.accumulator_seconds - self.fixed_step_seconds
        };
        if !remaining.is_finite() || remaining < 0.0 {
            return Err(SchedulerError::ArithmeticOverflow {
                context: "remaining scheduling debt",
            });
        }
        let step = ScheduledStep {
            sequence,
            source_completed_step: self.expected_completed_step,
            wall_now_ms: self
                .last_wall_now_ms
                .ok_or(SchedulerError::ClockNotInitialized)?,
            wall_accumulator_after_step: remaining,
            service_mode,
        };
        self.next_ticket_sequence = next_sequence;
        self.serviced_mode = None;
        self.pending_step = Some(step);
        Ok(step)
    }

    /// Retire one rejected running-step attempt without consuming its debt.
    ///
    /// A retry must cross a fresh command/action service boundary, so input that
    /// arrived while the failed attempt ran can affect the next eligible step.
    pub fn reject_step(
        &mut self,
        authority: &AuthoritativeState,
        step: ScheduledStep,
    ) -> Result<(), SchedulerError> {
        self.validate_pending(step)?;
        self.validate_authority(authority)?;
        if authority.state().generation.completed_step != step.source_completed_step {
            return Err(SchedulerError::PublicationMismatch {
                field: "completed step changed after rejection",
            });
        }
        self.pending_step = None;
        self.serviced_mode = None;
        Ok(())
    }

    /// Commit scheduler continuation after the exact authority step publishes.
    pub fn commit_step(
        &mut self,
        authority: &AuthoritativeState,
        step: ScheduledStep,
        publication: RunningStepPublication,
    ) -> Result<(), SchedulerError> {
        self.validate_pending(step)?;
        self.validate_authority_identity(authority)?;
        if authority
            .validate_running_step_publication(publication)
            .is_err()
        {
            return Err(SchedulerError::PublicationMismatch {
                field: "complete authority publication identity",
            });
        }
        let expected_completed_step = step.source_completed_step.checked_add(1).ok_or(
            SchedulerError::ArithmeticOverflow {
                context: "completed step",
            },
        )?;
        if publication.key.source_completed_step() != step.source_completed_step {
            return Err(SchedulerError::PublicationMismatch {
                field: "source completed step",
            });
        }
        if publication.completed_step != expected_completed_step
            || authority.state().generation.completed_step != expected_completed_step
        {
            return Err(SchedulerError::PublicationMismatch {
                field: "published completed step",
            });
        }
        if authority
            .state()
            .generation
            .wall_accumulator_seconds
            .to_bits()
            != step.wall_accumulator_after_step.to_bits()
        {
            return Err(SchedulerError::PublicationMismatch {
                field: "published wall accumulator",
            });
        }
        let completed_simulation_seconds =
            self.completed_simulation_seconds + self.fixed_step_seconds;
        if !completed_simulation_seconds.is_finite() {
            return Err(SchedulerError::ArithmeticOverflow {
                context: "completed simulation seconds",
            });
        }
        let completed_steps =
            self.completed_steps
                .checked_add(1)
                .ok_or(SchedulerError::ArithmeticOverflow {
                    context: "completed scheduler steps",
                })?;

        self.accumulator_seconds = step.wall_accumulator_after_step;
        self.expected_completed_step = expected_completed_step;
        self.completed_simulation_seconds = completed_simulation_seconds;
        self.completed_steps = completed_steps;
        self.pending_step = None;
        self.serviced_mode = None;
        if self.due_steps() == 0 {
            self.overload_active = false;
        }
        Ok(())
    }

    /// Retire the terminal ticket and rebind this scheduler to the published
    /// next-generation authority without counting persistence/assignment wait
    /// time as simulation debt.
    ///
    /// `resume_wall_now_ms` is sampled after the complete authority swap. It
    /// becomes the next wall-clock origin, so the server can remain responsive
    /// during checkpoint I/O without triggering a catch-up burst afterward.
    pub fn commit_generation_transition(
        &mut self,
        authority: &AuthoritativeState,
        step: ScheduledStep,
        publication: &GenerationStartPublication,
        resume_wall_now_ms: u64,
    ) -> Result<(), SchedulerError> {
        self.validate_pending(step)?;
        if resume_wall_now_ms < step.wall_now_ms {
            return Err(SchedulerError::RegressingWallClock {
                previous_ms: step.wall_now_ms,
                actual_ms: resume_wall_now_ms,
            });
        }
        let state = authority.state();
        let expected_completed_step = step.source_completed_step.checked_add(1).ok_or(
            SchedulerError::ArithmeticOverflow {
                context: "generation completed step",
            },
        )?;
        let expected_generation = publication.source_key.generation().checked_add(1).ok_or(
            SchedulerError::ArithmeticOverflow {
                context: "generation identity",
            },
        )?;
        let expected_population_epoch = publication
            .source_key
            .population_epoch()
            .checked_add(1)
            .ok_or(SchedulerError::ArithmeticOverflow {
                context: "population epoch",
            })?;
        if publication.source_key.world_epoch() != self.world_epoch
            || publication.source_key.source_completed_step() != step.source_completed_step
            || publication.source_key.config_revision() != self.config_revision
        {
            return Err(SchedulerError::PublicationMismatch {
                field: "terminal source identity",
            });
        }
        if state.phase != AuthorityPhase::Running
            || authority.world_epoch() != publication.world_epoch
            || state.generation.generation != publication.generation
            || state.generation.completed_step != publication.completed_step
            || state.generation.population_epoch != publication.population_epoch
            || state.generation.generation != expected_generation
            || state.generation.completed_step != expected_completed_step
            || state.generation.population_epoch != expected_population_epoch
            || authority.memory_estimate() != publication.memory
        {
            return Err(SchedulerError::PublicationMismatch {
                field: "running successor identity",
            });
        }
        if state.identity.config_revision != self.config_revision
            || state.identity.config_hash != self.config_hash
            || state.config.fixed_step_seconds.to_bits() != self.fixed_step_seconds.to_bits()
            || state.config.requested_sim_speed.to_bits() != self.requested_multiplier.to_bits()
        {
            return Err(SchedulerError::AuthorityMismatch {
                field: "generation successor configuration",
            });
        }
        if state.generation.wall_accumulator_seconds.to_bits()
            != step.wall_accumulator_after_step.to_bits()
        {
            return Err(SchedulerError::PublicationMismatch {
                field: "generation wall accumulator",
            });
        }
        let completed_simulation_seconds =
            self.completed_simulation_seconds + self.fixed_step_seconds;
        if !completed_simulation_seconds.is_finite() {
            return Err(SchedulerError::ArithmeticOverflow {
                context: "generation completed simulation seconds",
            });
        }
        let completed_steps =
            self.completed_steps
                .checked_add(1)
                .ok_or(SchedulerError::ArithmeticOverflow {
                    context: "generation scheduler steps",
                })?;

        self.world_epoch = publication.world_epoch;
        self.expected_completed_step = expected_completed_step;
        self.accumulator_seconds = step.wall_accumulator_after_step;
        self.last_wall_now_ms = Some(resume_wall_now_ms);
        self.completed_simulation_seconds = completed_simulation_seconds;
        self.completed_steps = completed_steps;
        self.pending_step = None;
        self.serviced_mode = None;
        if self.due_steps() == 0 {
            self.overload_active = false;
        }
        Ok(())
    }

    /// Read the current policy.
    #[must_use]
    pub const fn policy(&self) -> FixedStepSchedulerPolicy {
        self.policy
    }

    /// Read current operational diagnostics without changing scheduling state.
    #[must_use]
    pub fn diagnostics(&self) -> FixedStepSchedulerDiagnostics {
        FixedStepSchedulerDiagnostics {
            fixed_step_seconds: self.fixed_step_seconds,
            requested_multiplier: self.requested_multiplier,
            pending_simulation_seconds: self.accumulator_seconds,
            pending_wall_seconds: self.accumulator_seconds / self.requested_multiplier,
            catch_up_wall_debt_seconds: self.catch_up_wall_debt_seconds(),
            maximum_wall_debt_seconds: self.maximum_wall_debt_seconds,
            observed_wall_seconds: self.observed_wall_seconds,
            completed_simulation_seconds: self.completed_simulation_seconds,
            completed_steps: self.completed_steps,
            achieved_multiplier: if self.observed_wall_seconds > 0.0 {
                self.completed_simulation_seconds / self.observed_wall_seconds
            } else {
                0.0
            },
            dropped_simulation_seconds: self.dropped_simulation_seconds,
            dropped_wall_seconds: self.dropped_wall_seconds,
            dropped_simulation_seconds_latest: self.dropped_simulation_seconds_latest,
            dropped_wall_seconds_latest: self.dropped_wall_seconds_latest,
            command_service_boundaries: self.command_service_boundaries,
            interactive_service_boundaries: self.interactive_service_boundaries,
            step_pending: self.pending_step.is_some(),
            overloaded: self.overload_active,
        }
    }

    fn observe_wall_clock(&mut self, wall_now_ms: u64) -> Result<(), SchedulerError> {
        let elapsed_wall_seconds = match self.last_wall_now_ms {
            Some(previous) if wall_now_ms < previous => {
                return Err(SchedulerError::RegressingWallClock {
                    previous_ms: previous,
                    actual_ms: wall_now_ms,
                });
            }
            Some(previous) => (wall_now_ms - previous) as f64 / 1_000.0,
            None => 0.0,
        };
        let observed_wall_seconds = self.observed_wall_seconds + elapsed_wall_seconds;
        let requested_simulation_seconds = elapsed_wall_seconds * self.requested_multiplier;
        let raw_accumulator = self.accumulator_seconds + requested_simulation_seconds;
        let maximum_accumulator = self.maximum_accumulator_seconds()?;
        if !observed_wall_seconds.is_finite()
            || !requested_simulation_seconds.is_finite()
            || !raw_accumulator.is_finite()
        {
            return Err(SchedulerError::ArithmeticOverflow {
                context: "wall-clock accumulation",
            });
        }
        let dropped_simulation_seconds_latest = (raw_accumulator - maximum_accumulator).max(0.0);
        let accumulator_seconds = raw_accumulator.min(maximum_accumulator);
        let dropped_wall_seconds_latest =
            dropped_simulation_seconds_latest / self.requested_multiplier;
        let dropped_simulation_seconds =
            self.dropped_simulation_seconds + dropped_simulation_seconds_latest;
        let dropped_wall_seconds = self.dropped_wall_seconds + dropped_wall_seconds_latest;
        let current_wall_debt = accumulator_seconds / self.requested_multiplier;
        if !dropped_simulation_seconds.is_finite()
            || !dropped_wall_seconds.is_finite()
            || !current_wall_debt.is_finite()
        {
            return Err(SchedulerError::ArithmeticOverflow {
                context: "scheduler diagnostics",
            });
        }

        self.last_wall_now_ms = Some(wall_now_ms);
        self.observed_wall_seconds = observed_wall_seconds;
        self.accumulator_seconds = accumulator_seconds;
        self.dropped_simulation_seconds_latest = dropped_simulation_seconds_latest;
        self.dropped_wall_seconds_latest = dropped_wall_seconds_latest;
        if dropped_wall_seconds_latest > 0.0 {
            self.overload_active = true;
        }
        self.dropped_simulation_seconds = dropped_simulation_seconds;
        self.dropped_wall_seconds = dropped_wall_seconds;
        self.maximum_wall_debt_seconds = self.maximum_wall_debt_seconds.max(current_wall_debt);
        Ok(())
    }

    fn maximum_accumulator_seconds(&self) -> Result<f64, SchedulerError> {
        let catch_up = self.policy.catch_up_horizon_seconds * self.requested_multiplier;
        let maximum = self.fixed_step_seconds + catch_up;
        if !catch_up.is_finite() || !maximum.is_finite() {
            return Err(SchedulerError::ArithmeticOverflow {
                context: "catch-up horizon",
            });
        }
        Ok(maximum)
    }

    fn due_steps(&self) -> usize {
        let allowance = self.fixed_step_seconds * 1.0e-9;
        if self.accumulator_seconds + allowance < self.fixed_step_seconds {
            return 0;
        }
        ((self.accumulator_seconds + allowance) / self.fixed_step_seconds).floor() as usize
    }

    fn catch_up_wall_debt_seconds(&self) -> f64 {
        (self.accumulator_seconds - self.fixed_step_seconds).max(0.0) / self.requested_multiplier
    }

    fn validate_pending(&self, step: ScheduledStep) -> Result<(), SchedulerError> {
        match self.pending_step {
            Some(pending) if pending == step => Ok(()),
            Some(_) => Err(SchedulerError::StaleStepTicket),
            None => Err(SchedulerError::NoStepPending),
        }
    }

    fn validate_authority(&self, authority: &AuthoritativeState) -> Result<(), SchedulerError> {
        self.validate_authority_identity(authority)?;
        if authority.state().phase != AuthorityPhase::Running {
            return Err(SchedulerError::AuthorityMismatch { field: "phase" });
        }
        if authority.state().generation.completed_step != self.expected_completed_step {
            return Err(SchedulerError::AuthorityMismatch {
                field: "completed step",
            });
        }
        Ok(())
    }

    fn validate_authority_identity(
        &self,
        authority: &AuthoritativeState,
    ) -> Result<(), SchedulerError> {
        let state = authority.state();
        if authority.world_epoch() != self.world_epoch {
            return Err(SchedulerError::AuthorityMismatch {
                field: "world epoch",
            });
        }
        if state.identity.config_revision != self.config_revision {
            return Err(SchedulerError::AuthorityMismatch {
                field: "config revision",
            });
        }
        if state.identity.config_hash != self.config_hash {
            return Err(SchedulerError::AuthorityMismatch {
                field: "config hash",
            });
        }
        if state.config.fixed_step_seconds.to_bits() != self.fixed_step_seconds.to_bits() {
            return Err(SchedulerError::AuthorityMismatch {
                field: "fixed step",
            });
        }
        if state.config.requested_sim_speed.to_bits() != self.requested_multiplier.to_bits() {
            return Err(SchedulerError::AuthorityMismatch {
                field: "simulation multiplier",
            });
        }
        Ok(())
    }
}

/// Checked scheduler configuration, clock, ticket, or publication failure.
#[derive(Clone, Debug, PartialEq)]
pub enum SchedulerError {
    /// Operational scheduler policy is invalid.
    InvalidPolicy { field: &'static str },
    /// A scalar in the admitted authority cannot drive the scheduler.
    InvalidAuthorityScalar { field: &'static str },
    /// The scheduler was constructed for a different authority or config.
    AuthorityMismatch { field: &'static str },
    /// Wall time moved backward.
    RegressingWallClock { previous_ms: u64, actual_ms: u64 },
    /// The startup wall-clock origin was already established.
    ClockAlreadyInitialized,
    /// One ticket is already waiting for success or rejection.
    StepPending,
    /// The caller did not service commands/actions before requesting a step.
    CommandServiceRequired,
    /// Current debt does not yet represent one complete fixed step.
    StepNotDue,
    /// No scheduler ticket is waiting.
    NoStepPending,
    /// The supplied ticket is not the exact pending ticket.
    StaleStepTicket,
    /// The first wall-clock boundary has not been initialized.
    ClockNotInitialized,
    /// A publication does not match the pending ticket or resulting authority.
    PublicationMismatch { field: &'static str },
    /// Checked scheduler arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
}

impl Display for SchedulerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolicy { field } => write!(formatter, "invalid scheduler policy {field}"),
            Self::InvalidAuthorityScalar { field } => {
                write!(formatter, "invalid scheduler authority scalar {field}")
            }
            Self::AuthorityMismatch { field } => {
                write!(formatter, "scheduler authority changed: {field}")
            }
            Self::RegressingWallClock {
                previous_ms,
                actual_ms,
            } => write!(
                formatter,
                "scheduler wall clock regressed from {previous_ms} ms to {actual_ms} ms"
            ),
            Self::ClockAlreadyInitialized => {
                write!(formatter, "scheduler wall clock is already initialized")
            }
            Self::StepPending => write!(formatter, "one scheduler step is already pending"),
            Self::CommandServiceRequired => {
                write!(
                    formatter,
                    "commands/actions must be serviced before the next step"
                )
            }
            Self::StepNotDue => write!(formatter, "no complete fixed step is due"),
            Self::NoStepPending => write!(formatter, "no scheduler step is pending"),
            Self::StaleStepTicket => write!(formatter, "scheduler step ticket is stale"),
            Self::ClockNotInitialized => {
                write!(formatter, "scheduler wall clock is not initialized")
            }
            Self::PublicationMismatch { field } => {
                write!(formatter, "scheduler publication mismatch: {field}")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "scheduler arithmetic overflow: {context}")
            }
        }
    }
}

impl Error for SchedulerError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler_for_clock(
        fixed_step_seconds: f64,
        requested_multiplier: f64,
        accumulator_seconds: f64,
    ) -> FixedStepScheduler {
        FixedStepScheduler {
            policy: FixedStepSchedulerPolicy::provisional_defaults(),
            world_epoch: 1,
            config_revision: 1,
            config_hash: "test".to_owned(),
            fixed_step_seconds,
            requested_multiplier,
            expected_completed_step: 0,
            accumulator_seconds,
            last_wall_now_ms: None,
            next_ticket_sequence: 1,
            serviced_mode: None,
            pending_step: None,
            observed_wall_seconds: 0.0,
            completed_simulation_seconds: 0.0,
            completed_steps: 0,
            dropped_simulation_seconds: 0.0,
            dropped_wall_seconds: 0.0,
            dropped_simulation_seconds_latest: 0.0,
            dropped_wall_seconds_latest: 0.0,
            overload_active: false,
            maximum_wall_debt_seconds: accumulator_seconds / requested_multiplier,
            command_service_boundaries: 0,
            interactive_service_boundaries: 0,
        }
    }

    #[test]
    fn first_wall_observation_does_not_turn_startup_time_into_debt() {
        let mut scheduler = scheduler_for_clock(1.0 / 60.0, 1.0, 0.0);
        scheduler.observe_wall_clock(900_000).unwrap();
        let diagnostics = scheduler.diagnostics();
        assert_eq!(
            diagnostics.pending_simulation_seconds.to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            diagnostics.observed_wall_seconds.to_bits(),
            0.0_f64.to_bits()
        );

        scheduler.observe_wall_clock(900_017).unwrap();
        let diagnostics = scheduler.diagnostics();
        assert_eq!(
            diagnostics.observed_wall_seconds.to_bits(),
            0.017_f64.to_bits()
        );
        assert_eq!(
            diagnostics.pending_simulation_seconds.to_bits(),
            0.017_f64.to_bits()
        );
        assert_eq!(scheduler.due_steps(), 1);
    }

    #[test]
    fn multiplier_changes_requested_step_count_but_never_fixed_delta() {
        let mut scheduler = scheduler_for_clock(1.0 / 60.0, 12.0, 0.0);
        scheduler.observe_wall_clock(10_000).unwrap();
        scheduler.observe_wall_clock(10_017).unwrap();
        assert_eq!(scheduler.due_steps(), 12);
        assert_eq!(
            scheduler.diagnostics().fixed_step_seconds.to_bits(),
            (1.0_f64 / 60.0).to_bits()
        );
        assert!((scheduler.diagnostics().pending_simulation_seconds - 0.204).abs() < 1.0e-15);
    }

    #[test]
    fn catch_up_horizon_drops_only_excess_scheduling_debt() {
        let mut scheduler = scheduler_for_clock(1.0 / 60.0, 1.0, 0.0);
        scheduler.observe_wall_clock(1_000).unwrap();
        scheduler.observe_wall_clock(11_000).unwrap();
        let diagnostics = scheduler.diagnostics();
        let maximum = (1.0_f64 / 60.0) + 0.250;
        assert!((diagnostics.pending_simulation_seconds - maximum).abs() < 1.0e-12);
        assert!((diagnostics.catch_up_wall_debt_seconds - 0.250).abs() < 1.0e-12);
        assert!((diagnostics.dropped_wall_seconds_latest - (10.0 - maximum)).abs() < 1.0e-12);
        assert!(diagnostics.overloaded);
        assert_eq!(scheduler.due_steps(), 16);
    }

    #[test]
    fn a_low_rate_step_can_mature_even_when_it_exceeds_the_catch_up_horizon() {
        let mut scheduler = scheduler_for_clock(1.0, 0.1, 0.0);
        scheduler.observe_wall_clock(1_000).unwrap();
        scheduler.observe_wall_clock(11_000).unwrap();
        assert_eq!(scheduler.due_steps(), 1);
        assert_eq!(
            scheduler.diagnostics().pending_simulation_seconds.to_bits(),
            1.0_f64.to_bits()
        );
        assert_eq!(
            scheduler.diagnostics().dropped_simulation_seconds.to_bits(),
            0.0_f64.to_bits()
        );
    }

    #[test]
    fn regressing_wall_time_and_invalid_policy_are_rejected_atomically() {
        for value in [f64::NAN, -f64::EPSILON, 61.0] {
            let policy = FixedStepSchedulerPolicy {
                catch_up_horizon_seconds: value,
                ..FixedStepSchedulerPolicy::provisional_defaults()
            };
            assert!(matches!(
                policy.validate(),
                Err(SchedulerError::InvalidPolicy {
                    field: "catch-up horizon"
                })
            ));
        }

        let mut scheduler = scheduler_for_clock(1.0 / 60.0, 1.0, 0.0);
        scheduler.observe_wall_clock(100).unwrap();
        let before = scheduler.diagnostics();
        assert_eq!(
            scheduler.observe_wall_clock(99),
            Err(SchedulerError::RegressingWallClock {
                previous_ms: 100,
                actual_ms: 99,
            })
        );
        assert_eq!(scheduler.diagnostics(), before);
    }
}
