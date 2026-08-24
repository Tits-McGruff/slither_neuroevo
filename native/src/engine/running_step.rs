//! Private orchestration of one definitely nonterminal authoritative fixed step.
//!
//! This joins the already-reviewed prefix, shared controller, physics, and
//! publication transactions without exposing their intermediate mutable state.
//! External observation delivery, controlled-snake replacement, and generation
//! transition remain explicit fail-closed boundaries until their dependent
//! Stage 5/6 contracts are implemented.

use super::control::{NeuralControlError, NeuralControlPipeline};
use super::control_phase::{
    ControlCommitWorkspace, ControlPhaseError, ControlPhaseInputs, ControlPhaseWorkspace,
};
use super::fixed_step::{FixedStepPrefixError, FixedStepPrefixInputs, FixedStepPrefixWorkspace};
use super::inference::{GraphExecutionPlan, InferenceError};
use super::sensors::{SensorError, SensorEvaluator};
use super::state::{
    AuthoritativeState, RunningStepPublication, RunningStepReplacement, SnakeKind, StateError,
    WorldState,
};
use super::step_config::{
    GenerationGuardConfig, RunningStepConfigProjection, RunningStepWorkLimits, StepConfigError,
};
use super::world_step::{WorldStepDiagnostics, WorldStepError, WorldStepWorkspace};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First complete nonterminal phase-chain coordinator contract.
pub const RUNNING_STEP_COORDINATOR_VERSION: u32 = 1;

/// Scheduler and wall-clock values supplied for exactly one fixed-step attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningStepInputs {
    /// Monotonic wall-clock boundary used only by controller leases.
    pub wall_now_ms: u64,
    /// Scheduler debt remaining after this fixed step commits.
    pub wall_accumulator_seconds: f64,
}

/// A successful complete nonterminal authority publication.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningStepOutcome {
    /// Exact key, completed-step identity, and admitted memory result.
    pub publication: RunningStepPublication,
    /// Work/capacity diagnostics captured before the swap.
    pub diagnostics: WorldStepDiagnostics,
}

/// Reason a completed physical step must enter the later generation transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationTransitionReason {
    /// Configured generation duration was reached.
    Duration,
    /// The evolved alive count reached the configured early-end rule.
    EarlyAliveCount,
}

/// Reusable single-owner coordinator for one admitted authority/config/graph.
#[derive(Debug)]
pub struct RunningStepCoordinator {
    projection: RunningStepConfigProjection,
    world_epoch: u64,
    config_revision: u64,
    config_hash: String,
    graph_layout_digest: [u8; 32],
    last_wall_now_ms: Option<u64>,
    prefix: FixedStepPrefixWorkspace,
    control: ControlPhaseWorkspace,
    control_commit: ControlCommitWorkspace,
    world_step: WorldStepWorkspace,
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
        let inference = GraphExecutionPlan::build(authority.graph())?;
        let state = authority.state();
        let neural = NeuralControlPipeline::try_new(
            state.config.max_world_snakes,
            sensor,
            inference,
            state.config.worker_scratch_bytes,
        )?;
        Ok(Self {
            projection,
            world_epoch: authority.world_epoch(),
            config_revision: state.identity.config_revision,
            config_hash: state.identity.config_hash.clone(),
            graph_layout_digest: authority.graph().layout_digest_sha256,
            last_wall_now_ms: None,
            prefix: FixedStepPrefixWorkspace::new(),
            control: ControlPhaseWorkspace::new(neural),
            control_commit: ControlCommitWorkspace::new(),
            world_step: WorldStepWorkspace::new(),
        })
    }

    /// Advance and publish one complete step only when no deferred boundary is required.
    ///
    /// Any failure changes reusable scratch and the process-local attempt epoch,
    /// but not the authoritative [`StateCandidate`](super::state::StateCandidate).
    /// External observations stop before physics because their score-delivery
    /// markers require a matching Node send result. A terminal physical result
    /// stops before publication because Stage 6 owns evolution and replacement.
    pub fn advance_nonterminal(
        &mut self,
        authority: &mut AuthoritativeState,
        inputs: RunningStepInputs,
    ) -> Result<RunningStepOutcome, RunningStepError> {
        self.validate_authority(authority)?;
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
        let diagnostics = {
            let state = authority.state();
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
            let selected = self.control.prepare(ControlPhaseInputs {
                prefix,
                generation: &state.fixed_step.sensor_generation,
                population: &state.population,
                brains: &state.brains,
                wall_now_ms: inputs.wall_now_ms,
                config: self.projection.world_step.control,
            })?;
            if !selected.external_events().is_empty() {
                return Err(RunningStepError::ExternalDeliveryRequired {
                    count: selected.external_events().len(),
                });
            }
            let committed = self.control_commit.prepare(selected)?;
            let prepared = self
                .world_step
                .prepare(committed, self.projection.world_step)?;
            if let Some((reason, alive_evolved)) = generation_transition_required(
                prepared.world(),
                prepared.generation_elapsed_seconds(),
                self.projection.generation_guard,
            ) {
                return Err(RunningStepError::GenerationTransitionRequired {
                    reason,
                    elapsed_seconds: prepared.generation_elapsed_seconds(),
                    alive_evolved,
                });
            }
            prepared.diagnostics()
        };

        let publication = (|| -> Result<RunningStepPublication, RunningStepError> {
            let buffers = self.world_step.publication_buffers(key)?;
            let brains = self.control_commit.publication_brains(key)?;
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
            })?)
        })();
        self.world_step.invalidate_publication();
        self.control_commit.invalidate_publication();

        Ok(RunningStepOutcome {
            publication: publication?,
            diagnostics,
        })
    }

    /// Last accepted wall-clock boundary, including a failed staged attempt.
    #[must_use]
    pub const fn last_wall_now_ms(&self) -> Option<u64> {
        self.last_wall_now_ms
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
    /// Authority key/admission/publication failed.
    State(Box<StateError>),
    /// The coordinator was built for a different immutable authority contract.
    AuthorityMismatch { field: &'static str },
    /// Scheduler debt was non-finite or negative.
    InvalidSchedulerAccumulator(f64),
    /// A wall-clock value regressed behind an earlier accepted boundary.
    RegressingWallClock { previous_ms: u64, actual_ms: u64 },
    /// Node must accept or reject external observations before this step can continue.
    ExternalDeliveryRequired { count: usize },
    /// This physical result requires the not-yet-implemented generation transition.
    GenerationTransitionRequired {
        reason: GenerationTransitionReason,
        elapsed_seconds: f64,
        alive_evolved: usize,
    },
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
            Self::State(error) => write!(formatter, "{error}"),
            Self::AuthorityMismatch { field } => {
                write!(formatter, "running-step coordinator authority changed: {field}")
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
            Self::ExternalDeliveryRequired { count } => write!(
                formatter,
                "{count} external observations require matching Node delivery results"
            ),
            Self::GenerationTransitionRequired {
                reason,
                elapsed_seconds,
                alive_evolved,
            } => write!(
                formatter,
                "generation transition required after {elapsed_seconds} seconds with {alive_evolved} evolved snakes alive ({reason:?})"
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
            Self::State(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}
