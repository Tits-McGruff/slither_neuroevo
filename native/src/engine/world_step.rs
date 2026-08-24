//! Complete post-control fixed-step staging.
//!
//! This module accepts only one already-complete internal control boundary,
//! drives every configured physics substep from it, applies keyed baseline
//! death notifications, and updates generation-scoped sensor state. The
//! returned boundary is still non-authoritative. A later coordinator must
//! revalidate current authority and perform the one final swap.

use super::baseline::{
    BaselineLifecycleConfig, BaselineLifecycleDiagnostics, BaselineLifecycleError,
    BaselineLifecycleState, BaselineLifecycleWorkspace,
};
use super::calculation::CalculationBatchKey;
use super::control_phase::{
    ControlCommitDiagnostics, ControlPhaseConfig, ControlPhaseError, PreparedControlCommit,
    PreparedExternalObservation,
};
use super::fixed_step::{copy_lifecycle_reusing, FixedStepPrefixConfig, FixedStepPrefixError};
use super::physics::{
    PhysicsConfig, PhysicsError, PhysicsPipelineWorkspace, PhysicsStepDiagnostics, PhysicsStepKey,
    PhysicsStepWorkspace, PhysicsSubstepDiagnostics,
};
use super::sensors::{SensorError, SensorGenerationState};
use super::spatial::PelletIndexDiagnostics;
use super::state::{AllocatorState, BrainRuntimeState, RngStateBundle, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First complete post-control world-step join identity.
pub const WORLD_STEP_VERSION: u32 = 1;
/// Hard safety ceiling for collision-only subdivisions of one fixed step.
pub const MAXIMUM_PHYSICS_SUBSTEPS: usize = 256;

/// Complete settings and work counts bound across control and physics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldStepConfig {
    /// Versioned join and phase-order identity.
    pub algorithm_version: u32,
    /// Exact settings used by the pre-control prefix.
    pub prefix: FixedStepPrefixConfig,
    /// Exact settings used by controller selection and publication.
    pub control: ControlPhaseConfig,
    /// Exact movement/food/collision/effect settings for every substep.
    pub physics: PhysicsConfig,
    /// Exact post-physics baseline death-delay settings.
    pub baseline: BaselineLifecycleConfig,
    /// Number of collision-only substeps whose deltas sum to one fixed step.
    pub physics_substeps: usize,
}

impl WorldStepConfig {
    /// Current TypeScript formula defaults with internally consistent capacities.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        let prefix = FixedStepPrefixConfig::typescript_defaults();
        let mut physics = PhysicsConfig::typescript_defaults();
        physics.maximum_body_points = prefix.maximum_body_points;
        physics.maximum_pellets = prefix.maximum_pellets;
        Self {
            algorithm_version: WORLD_STEP_VERSION,
            prefix,
            control: ControlPhaseConfig::typescript_defaults(),
            physics,
            baseline: prefix.baseline,
            physics_substeps: 3,
        }
    }

    pub(crate) fn validate_shape(self) -> Result<(), WorldStepError> {
        if self.algorithm_version != WORLD_STEP_VERSION {
            return Err(WorldStepError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        self.prefix.validate_shape()?;
        self.control.validate()?;
        self.physics.validate()?;
        self.baseline.validate()?;
        if self.baseline != self.prefix.baseline {
            return Err(WorldStepError::InvalidConfig {
                field: "baseline lifecycle",
            });
        }
        if self.control.maximum_snakes != self.prefix.maximum_snakes {
            return Err(WorldStepError::InvalidConfig {
                field: "maximum snakes",
            });
        }
        if self.physics.maximum_pellets != self.prefix.maximum_pellets {
            return Err(WorldStepError::InvalidConfig {
                field: "maximum pellets",
            });
        }
        if self.physics.maximum_body_points != self.prefix.maximum_body_points {
            return Err(WorldStepError::InvalidConfig {
                field: "maximum body points",
            });
        }
        if self.physics.movement.world_radius.to_bits()
            != self.prefix.ambient.world_radius.to_bits()
        {
            return Err(WorldStepError::InvalidConfig {
                field: "world radius",
            });
        }
        if self.physics.movement.world_radius.to_bits()
            != self.prefix.baseline_spawn.world_radius.to_bits()
            || self.physics.movement.snake_radius.to_bits()
                != self.prefix.baseline_spawn.snake_radius.to_bits()
            || self.physics.movement.snake_spacing.to_bits()
                != self.prefix.baseline_spawn.snake_spacing.to_bits()
            || self.physics.movement.snake_start_len != self.prefix.baseline_spawn.snake_start_len
            || self.physics.movement.snake_base_speed.to_bits()
                != self.prefix.baseline_snake_base_speed.to_bits()
        {
            return Err(WorldStepError::InvalidConfig {
                field: "baseline spawn movement settings",
            });
        }
        if self.physics_substeps == 0 || self.physics_substeps > MAXIMUM_PHYSICS_SUBSTEPS {
            return Err(WorldStepError::InvalidConfig {
                field: "physics substeps",
            });
        }
        let substep_count = self.physics_substeps as f64;
        let combined_dt = self.physics.substep_dt * substep_count;
        let scale = self.prefix.fixed_dt.abs().max(combined_dt.abs()).max(1.0);
        let tolerance = 8.0 * f64::EPSILON * scale;
        if !combined_dt.is_finite() || (combined_dt - self.prefix.fixed_dt).abs() > tolerance {
            return Err(WorldStepError::InvalidConfig {
                field: "physics substep sum",
            });
        }
        Ok(())
    }

    fn validate_against(self, control: PreparedControlCommit<'_>) -> Result<(), WorldStepError> {
        if self.prefix != control.prefix_config() {
            return Err(WorldStepError::ControlConfigMismatch { field: "prefix" });
        }
        if self.control != control.control_config() {
            return Err(WorldStepError::ControlConfigMismatch { field: "control" });
        }
        self.validate_shape()?;
        Ok(())
    }
}

/// Retained work and phase diagnostics for one complete staged world step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStepDiagnostics {
    /// Shared controller-selection and internal-publication work.
    pub control: ControlCommitDiagnostics,
    /// Complete accepted physics transaction.
    pub physics: PhysicsStepDiagnostics,
    /// Last accepted or rejected substep application buffer.
    pub last_substep: PhysicsSubstepDiagnostics,
    /// Retained pellet index used by the physics pipeline.
    pub pellet_index: PelletIndexDiagnostics,
    /// Post-physics baseline death-notification staging.
    pub baseline: BaselineLifecycleDiagnostics,
}

/// One complete post-control fixed step that remains non-authoritative.
#[derive(Clone, Copy, Debug)]
pub struct PreparedWorldStep<'workspace, 'control> {
    key: PhysicsStepKey,
    config: WorldStepConfig,
    control: PreparedControlCommit<'control>,
    world: &'workspace WorldState,
    rng: &'workspace RngStateBundle,
    allocators: &'workspace AllocatorState,
    lifecycle: &'workspace BaselineLifecycleState,
    sensor_generation: SensorGenerationState,
    diagnostics: WorldStepDiagnostics,
}

/// Mutable complete-step buffers admitted for the one authoritative swap.
pub(crate) struct WorldStepPublicationBuffers<'workspace> {
    /// Complete post-physics world.
    pub world: &'workspace mut WorldState,
    /// Complete post-step gameplay RNG continuations.
    pub rng: &'workspace mut RngStateBundle,
    /// Complete post-step deterministic allocator continuations.
    pub allocators: &'workspace mut AllocatorState,
    /// Complete post-step baseline lifecycle continuation.
    pub baseline_lifecycle: &'workspace mut BaselineLifecycleState,
    /// Fractional ambient-pellet credit after the prefix.
    pub ambient_pellet_accumulator: f64,
    /// Generation-best sensor continuation after physics.
    pub sensor_generation: SensorGenerationState,
    /// Simulated generation time after exactly one fixed delta.
    pub generation_elapsed_seconds: f64,
}

impl<'workspace, 'control> PreparedWorldStep<'workspace, 'control> {
    /// Exact authority/config/operation identity staged.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Exact prefix, control, physics, lifecycle, and subdivision settings staged.
    #[must_use]
    pub const fn config(self) -> WorldStepConfig {
        self.config
    }

    /// Exact heterogeneous calculation identity applied before physics.
    #[must_use]
    pub const fn calculation_key(self) -> CalculationBatchKey {
        self.control.calculation_key()
    }

    /// Complete physical and controller world after every substep.
    #[must_use]
    pub const fn world(self) -> &'workspace WorldState {
        self.world
    }

    /// Gameplay RNG continuation after prefix, baseline control, and effects.
    #[must_use]
    pub const fn rng(self) -> &'workspace RngStateBundle {
        self.rng
    }

    /// Deterministic allocator continuation after prefix and effects.
    #[must_use]
    pub const fn allocators(self) -> &'workspace AllocatorState {
        self.allocators
    }

    /// Neural runtime state after the complete pre-movement control boundary.
    #[must_use]
    pub const fn brains(self) -> &'control [BrainRuntimeState] {
        self.control.brains()
    }

    /// Baseline continuation after control and post-physics death notifications.
    #[must_use]
    pub const fn baseline_lifecycle(self) -> &'workspace BaselineLifecycleState {
        self.lifecycle
    }

    /// Monotonic generation-best sensor continuation after physics.
    #[must_use]
    pub const fn sensor_generation(self) -> SensorGenerationState {
        self.sensor_generation
    }

    /// Generation elapsed time after the once-per-step prefix.
    #[must_use]
    pub const fn generation_elapsed_seconds(self) -> f64 {
        self.control.generation_elapsed_seconds()
    }

    /// Ambient fractional pellet credit after this step's prefix.
    #[must_use]
    pub const fn ambient_accumulator(self) -> f64 {
        self.control.ambient_accumulator()
    }

    /// Packed external observations still awaiting matching Node acceptance.
    #[must_use]
    pub const fn external_events(self) -> &'control [PreparedExternalObservation] {
        self.control.external_events()
    }

    /// Resolve one external event and its packed Float32 observation without copying.
    #[must_use]
    pub fn external_observation(
        self,
        event_index: usize,
    ) -> Option<(&'control PreparedExternalObservation, &'control [f32])> {
        self.control.external_observation(event_index)
    }

    /// Work and retained capacities across every joined phase.
    #[must_use]
    pub const fn diagnostics(self) -> WorldStepDiagnostics {
        self.diagnostics
    }
}

/// Reusable owner of post-control physics and continuation staging.
#[derive(Debug, Default)]
pub struct WorldStepWorkspace {
    physics: PhysicsStepWorkspace,
    phases: PhysicsPipelineWorkspace,
    baseline: BaselineLifecycleWorkspace,
    lifecycle: Option<BaselineLifecycleState>,
    sensor_generation: SensorGenerationState,
    key: Option<PhysicsStepKey>,
    generation_elapsed_seconds: f64,
    ambient_pellet_accumulator: f64,
    ready: bool,
    diagnostics: WorldStepDiagnostics,
}

impl WorldStepWorkspace {
    /// Construct empty reusable complete-step scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance one already-committed control boundary through every physics substep.
    ///
    /// Failure may change reusable scratch but leaves the control source and all
    /// authoritative state untouched. The result remains non-authoritative.
    pub fn prepare<'workspace, 'control>(
        &'workspace mut self,
        control: PreparedControlCommit<'control>,
        config: WorldStepConfig,
    ) -> Result<PreparedWorldStep<'workspace, 'control>, WorldStepError> {
        self.key = None;
        self.ready = false;
        self.diagnostics = WorldStepDiagnostics::default();
        config.validate_against(control)?;
        let key = control.key();
        self.physics.begin(
            key,
            control.world(),
            control.rng(),
            control.allocators(),
            config.physics,
            config.physics_substeps,
        )?;
        for _ in 0..config.physics_substeps {
            self.physics.advance_substep(&mut self.phases, key)?;
        }

        copy_lifecycle_reusing(&mut self.lifecycle, control.baseline_lifecycle())?;
        {
            let physics = self.physics.finish(key)?;
            let deaths = self.baseline.prepare_committed_deaths(
                physics.prepared_baseline_deaths(),
                key,
                control.baseline_lifecycle(),
                config.baseline,
            )?;
            deaths.apply_to_working_copy(
                key,
                control.baseline_lifecycle(),
                config.baseline,
                self.lifecycle
                    .as_mut()
                    .ok_or(WorldStepError::ResultNotReady)?,
            )?;
            self.sensor_generation = control.sensor_generation();
            self.sensor_generation.update_after_step(physics.world())?;
        }

        self.key = Some(key);
        self.generation_elapsed_seconds = control.generation_elapsed_seconds();
        self.ambient_pellet_accumulator = control.ambient_accumulator();
        self.ready = true;
        self.diagnostics = WorldStepDiagnostics {
            control: control.diagnostics(),
            physics: self.physics.diagnostics(),
            last_substep: self.phases.substep_diagnostics(),
            pellet_index: self.phases.pellet_index_diagnostics(),
            baseline: self.baseline.diagnostics(),
        };
        self.prepared(control, config)
    }

    /// Whether the latest attempt produced one complete staged world step.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Current retained capacities, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> WorldStepDiagnostics {
        if self.ready {
            self.diagnostics
        } else {
            WorldStepDiagnostics {
                physics: self.physics.diagnostics(),
                last_substep: self.phases.substep_diagnostics(),
                pellet_index: self.phases.pellet_index_diagnostics(),
                baseline: self.baseline.diagnostics(),
                ..WorldStepDiagnostics::default()
            }
        }
    }

    /// Borrow one complete mutable result only for the authoritative coordinator.
    pub(crate) fn publication_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<WorldStepPublicationBuffers<'_>, WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        let Self {
            physics,
            lifecycle,
            sensor_generation,
            generation_elapsed_seconds,
            ambient_pellet_accumulator,
            ..
        } = self;
        let physics = physics.publication_buffers(key)?;
        Ok(WorldStepPublicationBuffers {
            world: physics.world,
            rng: physics.rng,
            allocators: physics.allocators,
            baseline_lifecycle: lifecycle.as_mut().ok_or(WorldStepError::ResultNotReady)?,
            ambient_pellet_accumulator: *ambient_pellet_accumulator,
            sensor_generation: *sensor_generation,
            generation_elapsed_seconds: *generation_elapsed_seconds,
        })
    }

    /// Invalidate the last view after any authority-publication attempt.
    pub(crate) fn invalidate_publication(&mut self) {
        self.key = None;
        self.ready = false;
    }

    fn prepared<'workspace, 'control>(
        &'workspace self,
        control: PreparedControlCommit<'control>,
        config: WorldStepConfig,
    ) -> Result<PreparedWorldStep<'workspace, 'control>, WorldStepError> {
        if !self.ready {
            return Err(WorldStepError::ResultNotReady);
        }
        let physics = self.physics.finish(control.key())?;
        Ok(PreparedWorldStep {
            key: control.key(),
            config,
            control,
            world: physics.world(),
            rng: physics.rng(),
            allocators: physics.allocators(),
            lifecycle: self
                .lifecycle
                .as_ref()
                .ok_or(WorldStepError::ResultNotReady)?,
            sensor_generation: self.sensor_generation,
            diagnostics: self.diagnostics,
        })
    }
}

/// Complete-step staging, configuration, or phase failure.
#[derive(Debug)]
pub enum WorldStepError {
    /// Prefix-owned reusable continuation copy failed.
    Prefix(Box<FixedStepPrefixError>),
    /// Shared control-selection configuration is invalid.
    Control(Box<ControlPhaseError>),
    /// One physics phase or transaction failed.
    Physics(Box<PhysicsError>),
    /// Baseline death-delay staging failed.
    Baseline(Box<BaselineLifecycleError>),
    /// Generation sensor continuation rejected the post-physics world.
    Sensor(Box<SensorError>),
    /// Joined step settings are internally inconsistent.
    InvalidConfig { field: &'static str },
    /// The supplied complete control boundary used different settings.
    ControlConfigMismatch { field: &'static str },
    /// No complete result is available.
    ResultNotReady,
}

impl From<FixedStepPrefixError> for WorldStepError {
    fn from(error: FixedStepPrefixError) -> Self {
        Self::Prefix(Box::new(error))
    }
}

impl From<ControlPhaseError> for WorldStepError {
    fn from(error: ControlPhaseError) -> Self {
        Self::Control(Box::new(error))
    }
}

impl From<PhysicsError> for WorldStepError {
    fn from(error: PhysicsError) -> Self {
        Self::Physics(Box::new(error))
    }
}

impl From<BaselineLifecycleError> for WorldStepError {
    fn from(error: BaselineLifecycleError) -> Self {
        Self::Baseline(Box::new(error))
    }
}

impl From<SensorError> for WorldStepError {
    fn from(error: SensorError) -> Self {
        Self::Sensor(Box::new(error))
    }
}

impl Display for WorldStepError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prefix(error) => write!(formatter, "{error}"),
            Self::Control(error) => write!(formatter, "{error}"),
            Self::Physics(error) => write!(formatter, "{error}"),
            Self::Baseline(error) => write!(formatter, "{error}"),
            Self::Sensor(error) => write!(formatter, "{error}"),
            Self::InvalidConfig { field } => {
                write!(formatter, "invalid complete world-step config {field}")
            }
            Self::ControlConfigMismatch { field } => {
                write!(
                    formatter,
                    "complete world-step {field} differs from control staging"
                )
            }
            Self::ResultNotReady => write!(formatter, "no complete world step is ready"),
        }
    }
}

impl Error for WorldStepError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Prefix(error) => Some(error.as_ref()),
            Self::Control(error) => Some(error.as_ref()),
            Self::Physics(error) => Some(error.as_ref()),
            Self::Baseline(error) => Some(error.as_ref()),
            Self::Sensor(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_substeps_cover_exactly_one_fixed_step() {
        let config = WorldStepConfig::typescript_defaults();
        let combined = config.physics.substep_dt * config.physics_substeps as f64;
        assert!((combined - config.prefix.fixed_dt).abs() <= 8.0 * f64::EPSILON);
        assert_eq!(config.baseline, config.prefix.baseline);
        assert_eq!(
            config.physics.maximum_body_points,
            config.prefix.maximum_body_points
        );
        assert_eq!(
            config.physics.maximum_pellets,
            config.prefix.maximum_pellets
        );
    }
}
