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
use super::controllers::{
    commit_disconnect_prevalidated, prepare_disconnect, validate_disconnect_proposal,
    ControllerError, ControllerTiming, DisconnectProposal,
};
use super::external_replacement::{
    AssignmentResolution, ExternalReplacementBuffers, ExternalReplacementConfig,
    ExternalReplacementDiagnostics, ExternalReplacementError, ExternalReplacementWorkspace,
    ReplacementAssignment, UnavailableControllerReservation,
};
use super::fixed_step::{copy_lifecycle_reusing, FixedStepPrefixConfig, FixedStepPrefixError};
use super::graph::CompiledGraph;
#[cfg(feature = "engine-test-hooks")]
use super::physics::PhysicsPhaseAllocations;
use super::physics::{
    PhysicsConfig, PhysicsError, PhysicsPipelineWorkspace, PhysicsStepDiagnostics, PhysicsStepKey,
    PhysicsStepWorkspace, PhysicsSubstepDiagnostics,
};
use super::sensors::{SensorError, SensorGenerationState};
use super::spatial::PelletIndexDiagnostics;
use super::state::{
    AllocatorState, BrainRuntimeState, ControllerLeaseStatus, RngStateBundle,
    RunningStepMutationContract, SnakeKind, WorldState,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First complete post-control world-step join identity.
pub const WORLD_STEP_VERSION: u32 = 1;
/// Hard safety ceiling for collision-only subdivisions of one fixed step.
pub const MAXIMUM_PHYSICS_SUBSTEPS: usize = 256;

/// Local Node disposition for one exact external observation event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalDeliveryStatus {
    /// No matching local send result has resolved this event yet.
    Pending,
    /// Node accepted the event into the matching socket send path.
    Accepted,
    /// The matching socket send failed and the controller must disconnect.
    Failed,
}

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
    /// Collision-safe externally controlled death replacement settings.
    pub external_replacement: ExternalReplacementConfig,
    /// Number of collision-only substeps whose deltas sum to one fixed step.
    pub physics_substeps: usize,
}

impl WorldStepConfig {
    /// Current TypeScript formula defaults with internally consistent capacities.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        let prefix = FixedStepPrefixConfig::typescript_defaults();
        let control = ControlPhaseConfig::typescript_defaults();
        let mut physics = PhysicsConfig::typescript_defaults();
        physics.maximum_body_points = prefix.maximum_body_points;
        physics.maximum_pellets = prefix.maximum_pellets;
        let external_replacement = ExternalReplacementConfig {
            spawn: prefix.baseline_spawn,
            snake_base_speed: prefix.baseline_snake_base_speed,
            controller_timing: control.controller_timing,
            maximum_snakes: prefix.maximum_snakes,
            maximum_body_points: prefix.maximum_body_points,
            maximum_brains: control.maximum_brains,
            ..ExternalReplacementConfig::typescript_defaults()
        };
        Self {
            algorithm_version: WORLD_STEP_VERSION,
            prefix,
            control,
            physics,
            baseline: prefix.baseline,
            external_replacement,
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
        self.external_replacement.validate()?;
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
        if self.external_replacement.spawn != self.prefix.baseline_spawn
            || self.external_replacement.snake_base_speed.to_bits()
                != self.prefix.baseline_snake_base_speed.to_bits()
            || self.external_replacement.controller_timing != self.control.controller_timing
            || self.external_replacement.maximum_snakes != self.prefix.maximum_snakes
            || self.external_replacement.maximum_body_points != self.prefix.maximum_body_points
            || self.external_replacement.maximum_brains != self.control.maximum_brains
        {
            return Err(WorldStepError::InvalidConfig {
                field: "external replacement",
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
    /// Controlled-death replacement work and retained capacities.
    pub external_replacement: ExternalReplacementDiagnostics,
    /// External observation markers still blocking authoritative publication.
    pub external_deliveries_pending: usize,
    /// Retained prevalidated send-failure disconnect proposals.
    pub external_disconnect_capacity: usize,
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
    replacement_brains: Option<&'workspace [BrainRuntimeState]>,
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
    /// Replacement-owned brain records when a controlled death occurred.
    pub replacement_brains: Option<&'workspace mut Vec<BrainRuntimeState>>,
    /// Private proof describing identity-changing controlled-death work.
    pub mutation: RunningStepMutationContract<'workspace>,
    /// Complete post-step baseline lifecycle continuation.
    pub baseline_lifecycle: &'workspace mut BaselineLifecycleState,
    /// Fractional ambient-pellet credit after the prefix.
    pub ambient_pellet_accumulator: f64,
    /// Generation-best sensor continuation after physics.
    pub sensor_generation: SensorGenerationState,
    /// Simulated generation time after exactly one fixed delta.
    pub generation_elapsed_seconds: f64,
}

#[derive(Clone, Copy, Debug)]
struct ExternalObservationResolution {
    event: PreparedExternalObservation,
    snake_index: usize,
    lease_index: usize,
    disconnect: DisconnectProposal,
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
    pub fn brains(&self) -> &[BrainRuntimeState] {
        self.replacement_brains
            .unwrap_or_else(|| self.control.brains())
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
    external_replacement: ExternalReplacementWorkspace,
    lifecycle: Option<BaselineLifecycleState>,
    sensor_generation: SensorGenerationState,
    key: Option<PhysicsStepKey>,
    prepared_config: Option<WorldStepConfig>,
    generation_elapsed_seconds: f64,
    ambient_pellet_accumulator: f64,
    pending_external_deliveries: usize,
    pending_replacement_deliveries: usize,
    replacement_active: bool,
    generation_reassignment_active: bool,
    replacement_deferred: bool,
    external_disconnects: Vec<ExternalObservationResolution>,
    ready: bool,
    diagnostics: WorldStepDiagnostics,
    #[cfg(feature = "engine-test-hooks")]
    allocation_snapshot: Option<fn() -> u64>,
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
        self.prepare_internal(control, config, false)
    }

    /// Advance physics while deliberately deferring controlled-death replacement.
    ///
    /// The running-step coordinator uses this private seam to evaluate the
    /// generation guard from the complete physical result before spending old-
    /// generation external RNG/identities or attempting a replacement spawn.
    pub(crate) fn prepare_deferred_external_replacement<'workspace, 'control>(
        &'workspace mut self,
        control: PreparedControlCommit<'control>,
        config: WorldStepConfig,
    ) -> Result<PreparedWorldStep<'workspace, 'control>, WorldStepError> {
        self.prepare_internal(control, config, true)
    }

    fn prepare_internal<'workspace, 'control>(
        &'workspace mut self,
        control: PreparedControlCommit<'control>,
        config: WorldStepConfig,
        defer_external_replacement: bool,
    ) -> Result<PreparedWorldStep<'workspace, 'control>, WorldStepError> {
        self.key = None;
        self.prepared_config = None;
        self.ready = false;
        self.pending_external_deliveries = 0;
        self.pending_replacement_deliveries = 0;
        self.replacement_active = false;
        self.generation_reassignment_active = false;
        self.replacement_deferred = false;
        self.external_replacement.discard();
        self.external_disconnects.clear();
        self.diagnostics = WorldStepDiagnostics::default();
        #[cfg(feature = "engine-test-hooks")]
        self.phases
            .reset_allocation_tracking(self.allocation_snapshot);
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
        #[cfg(feature = "engine-test-hooks")]
        self.phases.record_begin_allocations();
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

            if has_dead_controller(physics.world()) && !defer_external_replacement {
                return Err(WorldStepError::ExternalReplacementContextRequired);
            }
        }
        #[cfg(feature = "engine-test-hooks")]
        self.phases.record_finalize_allocations();

        self.key = Some(key);
        self.prepared_config = Some(config);
        self.replacement_deferred = defer_external_replacement;
        self.generation_elapsed_seconds = control.generation_elapsed_seconds();
        self.ambient_pellet_accumulator = control.ambient_accumulator();
        let final_world = if self.replacement_active {
            self.external_replacement.staged_world(key)?
        } else {
            self.physics.finish(key)?.world()
        };
        self.pending_external_deliveries = control
            .external_events()
            .iter()
            .filter(|event| external_observation_survives(final_world, **event))
            .count();
        self.ready = true;
        self.diagnostics = WorldStepDiagnostics {
            control: control.diagnostics(),
            physics: self.physics.diagnostics(),
            last_substep: self.phases.substep_diagnostics(),
            pellet_index: self.phases.pellet_index_diagnostics(),
            baseline: self.baseline.diagnostics(),
            external_replacement: self.external_replacement.diagnostics(),
            external_deliveries_pending: self
                .pending_external_deliveries
                .saturating_add(self.pending_replacement_deliveries),
            external_disconnect_capacity: self.external_disconnects.capacity(),
        };
        self.prepared(control, config)
    }

    /// Finish the deferred controlled-death phase after the nonterminal guard.
    ///
    /// This never reruns physics. A replacement failure invalidates the staged
    /// result so callers cannot accidentally publish the unreplaced base world.
    pub(crate) fn complete_deferred_external_replacement<'workspace, 'control>(
        &'workspace mut self,
        control: PreparedControlCommit<'control>,
        config: WorldStepConfig,
        graph: &CompiledGraph,
        wall_now_ms: u64,
    ) -> Result<PreparedWorldStep<'workspace, 'control>, WorldStepError> {
        let key = control.key();
        if !self.ready || self.key != Some(key) || !self.replacement_deferred {
            return Err(WorldStepError::ResultNotReady);
        }
        if self.prepared_config != Some(config) {
            return Err(WorldStepError::DeferredReplacementMismatch { field: "config" });
        }
        config.validate_against(control)?;
        self.ready = false;
        self.replacement_deferred = false;
        let physics = self.physics.finish(key)?;
        if has_dead_controller(physics.world()) {
            let replacements = self.external_replacement.prepare(
                key,
                physics.world(),
                physics.rng(),
                physics.allocators(),
                control.brains(),
                graph,
                wall_now_ms,
                config.external_replacement,
            )?;
            self.pending_replacement_deliveries = replacements.assignments().len();
            self.replacement_active = true;
        }
        let final_world = if self.replacement_active {
            self.external_replacement.staged_world(key)?
        } else {
            physics.world()
        };
        self.pending_external_deliveries = control
            .external_events()
            .iter()
            .filter(|event| external_observation_survives(final_world, **event))
            .count();
        self.ready = true;
        self.diagnostics.external_replacement = self.external_replacement.diagnostics();
        self.diagnostics.external_deliveries_pending = self
            .pending_external_deliveries
            .saturating_add(self.pending_replacement_deliveries);
        self.prepared(control, config)
    }

    /// Reuse the controlled-death transaction to assign fresh snakes to the
    /// controllers still connected after a durable generation boundary.
    ///
    /// This path owns no physics result or running-step continuation. It only
    /// stages the collision-safe generation base plus reliable assignments;
    /// the generation coordinator performs the later full-state admission and
    /// authority swap.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_generation_reassignments(
        &mut self,
        key: PhysicsStepKey,
        base_world: &WorldState,
        base_rng: &RngStateBundle,
        base_allocators: &AllocatorState,
        base_brains: &[BrainRuntimeState],
        controller_source_world: &WorldState,
        graph: &CompiledGraph,
        successor_population_epoch: u64,
        wall_now_ms: u64,
        config: ExternalReplacementConfig,
    ) -> Result<(), WorldStepError> {
        if self.ready || self.key.is_some() {
            return Err(WorldStepError::ResultNotReady);
        }
        self.pending_external_deliveries = 0;
        self.pending_replacement_deliveries = 0;
        self.replacement_active = false;
        self.generation_reassignment_active = false;
        self.replacement_deferred = false;
        self.external_disconnects.clear();
        self.diagnostics = WorldStepDiagnostics::default();
        let prepared = self.external_replacement.prepare_generation_reassignments(
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
        )?;
        self.pending_replacement_deliveries = prepared.assignments().len();
        self.replacement_active = true;
        self.generation_reassignment_active = true;
        self.key = Some(key);
        self.ready = true;
        self.diagnostics.external_replacement = prepared.diagnostics();
        self.diagnostics.external_deliveries_pending = self.pending_replacement_deliveries;
        Ok(())
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
                external_replacement: self.external_replacement.diagnostics(),
                external_disconnect_capacity: self.external_disconnects.capacity(),
                ..WorldStepDiagnostics::default()
            }
        }
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn set_allocation_snapshot(&mut self, snapshot: fn() -> u64) {
        self.allocation_snapshot = Some(snapshot);
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) const fn physics_phase_allocations(&self) -> PhysicsPhaseAllocations {
        self.phases.phase_allocations()
    }

    /// Canonically ordered controlled-death assignments awaiting local sends.
    pub(crate) fn replacement_assignments(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[ReplacementAssignment], WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        if !self.replacement_active {
            return Ok(&[]);
        }
        Ok(self.external_replacement.assignments(key)?)
    }

    /// Disconnected old-token outcomes retained across a replacement boundary.
    pub(crate) fn unavailable_controller_reservations(
        &self,
        key: PhysicsStepKey,
    ) -> Result<&[UnavailableControllerReservation], WorldStepError> {
        if !self.ready || self.key != Some(key) || !self.replacement_active {
            return Ok(&[]);
        }
        Ok(self.external_replacement.unavailable_reservations(key)?)
    }

    /// Complete provisional world used to filter superseded observations.
    pub(crate) fn staged_world(&self, key: PhysicsStepKey) -> Result<&WorldState, WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        if self.replacement_active {
            Ok(self.external_replacement.staged_world(key)?)
        } else {
            Ok(self.physics.finish(key)?.world())
        }
    }

    /// Opaque new resume token carried by one replacement assignment.
    pub(crate) fn replacement_resume_token(
        &self,
        key: PhysicsStepKey,
        assignment_index: usize,
    ) -> Result<&str, WorldStepError> {
        if !self.replacement_active {
            return Err(WorldStepError::ResultNotReady);
        }
        Ok(self
            .external_replacement
            .resume_token(key, assignment_index)?)
    }

    /// Apply one exact first-result-wins replacement-assignment disposition.
    pub(crate) fn resolve_replacement_assignment(
        &mut self,
        key: PhysicsStepKey,
        lease_id: u64,
        connection_id: u64,
        accepted: bool,
    ) -> Result<AssignmentResolution, WorldStepError> {
        if !self.ready || self.key != Some(key) || !self.replacement_active {
            return Ok(AssignmentResolution::Ignored);
        }
        let resolution =
            self.external_replacement
                .resolve_assignment(key, lease_id, connection_id, accepted)?;
        if matches!(
            resolution,
            AssignmentResolution::Accepted | AssignmentResolution::Failed
        ) {
            self.pending_replacement_deliveries = self
                .pending_replacement_deliveries
                .checked_sub(1)
                .ok_or(WorldStepError::ExternalDeliveryMismatch {
                    field: "replacement pending count",
                })?;
            self.diagnostics.external_deliveries_pending = self
                .pending_external_deliveries
                .saturating_add(self.pending_replacement_deliveries);
        }
        Ok(resolution)
    }

    /// Prevalidate both possible local-send outcomes for every external event.
    ///
    /// Every snake, lease, connection, assignment, score boundary, disconnect
    /// deadline, and retained proposal is checked before the event batch becomes
    /// visible to Node. A later accepted result may therefore advance the exact
    /// marker, while a failed result may disconnect the exact controller, with
    /// no fallible work remaining between resolution and authority publication.
    pub(crate) fn preflight_external_deliveries(
        &mut self,
        key: PhysicsStepKey,
        events: &[PreparedExternalObservation],
        disconnected_at_ms: u64,
        timing: ControllerTiming,
    ) -> Result<(), WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        if events.len() != self.pending_external_deliveries {
            return Err(WorldStepError::ExternalDeliveryMismatch {
                field: "event count",
            });
        }

        reserve_for(
            &mut self.external_disconnects,
            events.len(),
            "external disconnect proposals",
        )?;
        self.external_disconnects.clear();

        let world = if self.replacement_active {
            self.external_replacement.validation_buffers(key)?.world
        } else {
            self.physics.publication_buffers(key)?.world
        };
        let mut previous_snake_id = None;
        for event in events {
            if previous_snake_id.is_some_and(|previous| previous >= event.snake_id()) {
                return Err(WorldStepError::ExternalDeliveryMismatch {
                    field: "canonical snake order",
                });
            }
            previous_snake_id = Some(event.snake_id());
            let snake_index = world
                .snakes
                .iter()
                .position(|snake| snake.id == event.snake_id())
                .ok_or(WorldStepError::ExternalDeliveryMismatch {
                    field: "snake identity",
                })?;
            let lease_index = world
                .controller_leases
                .iter()
                .position(|lease| lease.id == event.lease_id())
                .ok_or(WorldStepError::ExternalDeliveryMismatch {
                    field: "lease identity",
                })?;
            let snake = &world.snakes[snake_index];
            let lease = &world.controller_leases[lease_index];
            if !snake.alive
                || snake.id != event.snake_id()
                || snake.kind != SnakeKind::External
                || lease.id != event.lease_id()
                || lease.snake_id != event.snake_id()
                || lease.kind != event.kind()
                || lease.connection_id != Some(event.connection_id())
                || lease.status != ControllerLeaseStatus::Connected
            {
                return Err(WorldStepError::ExternalDeliveryMismatch {
                    field: "snake or assignment identity",
                });
            }
            event.delivery().validate(snake)?;
            let disconnect = prepare_disconnect(
                lease,
                snake,
                event.connection_id(),
                disconnected_at_ms,
                timing,
            )?;
            validate_disconnect_proposal(lease, snake, disconnect)?;
            self.external_disconnects
                .push(ExternalObservationResolution {
                    event: *event,
                    snake_index,
                    lease_index,
                    disconnect,
                });
        }
        self.diagnostics.external_disconnect_capacity = self.external_disconnects.capacity();
        Ok(())
    }

    /// Recheck the retained event/proposal join before the batch becomes visible.
    pub(crate) fn validate_external_delivery_preflight(
        &mut self,
        key: PhysicsStepKey,
        events: &[PreparedExternalObservation],
    ) -> Result<(), WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        if events.len() != self.pending_external_deliveries
            || events.len() != self.external_disconnects.len()
        {
            return Err(WorldStepError::ExternalDeliveryMismatch {
                field: "preflight event count",
            });
        }
        let world = if self.replacement_active {
            self.external_replacement.validation_buffers(key)?.world
        } else {
            self.physics.publication_buffers(key)?.world
        };
        for (event, record) in events.iter().zip(self.external_disconnects.iter().copied()) {
            let snake = world.snakes.get(record.snake_index).ok_or(
                WorldStepError::ExternalDeliveryMismatch {
                    field: "preflight snake index",
                },
            )?;
            let lease = world.controller_leases.get(record.lease_index).ok_or(
                WorldStepError::ExternalDeliveryMismatch {
                    field: "preflight lease index",
                },
            )?;
            if record.event != *event
                || snake.id != event.snake_id()
                || lease.id != event.lease_id()
            {
                return Err(WorldStepError::ExternalDeliveryMismatch {
                    field: "preflight retained event",
                });
            }
            event.delivery().validate(snake)?;
            validate_disconnect_proposal(lease, snake, record.disconnect)?;
        }
        Ok(())
    }

    /// Apply one fully resolved external-delivery batch with no remaining failure path.
    ///
    /// [`Self::preflight_external_deliveries`] checked the exact marker and
    /// disconnect alternative for every event against this unchanged staged
    /// world. The coordinator guarantees every status is resolved before this
    /// method is called.
    pub(crate) fn commit_prevalidated_external_deliveries(
        &mut self,
        key: PhysicsStepKey,
        events: &[PreparedExternalObservation],
        statuses: &[ExternalDeliveryStatus],
    ) {
        debug_assert!(self.ready && self.key == Some(key));
        debug_assert_eq!(events.len(), self.pending_external_deliveries);
        debug_assert_eq!(events.len(), statuses.len());
        debug_assert_eq!(events.len(), self.external_disconnects.len());
        let world = if self.replacement_active {
            self.external_replacement
                .validation_buffers(key)
                .expect("prevalidated replacement world must remain ready")
                .world
        } else {
            self.physics
                .publication_buffers(key)
                .expect("prevalidated physics world must remain ready")
                .world
        };
        for ((event, status), record) in events
            .iter()
            .zip(statuses.iter())
            .zip(self.external_disconnects.iter().copied())
        {
            debug_assert_eq!(record.event, *event);
            let snake = &mut world.snakes[record.snake_index];
            match status {
                ExternalDeliveryStatus::Accepted => {
                    event.delivery().commit_prevalidated(snake);
                }
                ExternalDeliveryStatus::Failed => {
                    let lease = &mut world.controller_leases[record.lease_index];
                    commit_disconnect_prevalidated(lease, snake, record.disconnect);
                }
                ExternalDeliveryStatus::Pending => {
                    unreachable!("prevalidated external delivery cannot publish while pending");
                }
            }
        }
        self.pending_external_deliveries = 0;
        self.diagnostics.external_deliveries_pending = self.pending_replacement_deliveries;
    }

    /// Borrow one complete mutable result only for the authoritative coordinator.
    pub(crate) fn publication_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<WorldStepPublicationBuffers<'_>, WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        let pending = self
            .pending_external_deliveries
            .saturating_add(self.pending_replacement_deliveries);
        if pending != 0 {
            return Err(WorldStepError::ExternalDeliveryPending { count: pending });
        }
        self.resolved_buffers(key)
    }

    /// Resolved generation-base/controller buffers for successor admission.
    pub(crate) fn generation_reassignment_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<ExternalReplacementBuffers<'_>, WorldStepError> {
        if !self.ready
            || self.key != Some(key)
            || !self.replacement_active
            || !self.generation_reassignment_active
        {
            return Err(WorldStepError::ResultNotReady);
        }
        Ok(self.external_replacement.publication_buffers(key)?)
    }

    /// Unresolved generation/controller buffers used for full successor
    /// admission before any assignment is exposed to Node.
    pub(crate) fn generation_reassignment_validation_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<ExternalReplacementBuffers<'_>, WorldStepError> {
        if !self.ready
            || self.key != Some(key)
            || !self.replacement_active
            || !self.generation_reassignment_active
        {
            return Err(WorldStepError::ResultNotReady);
        }
        Ok(self.external_replacement.validation_buffers(key)?)
    }

    /// Borrow publication buffers after all checks were completed before Node delivery.
    pub(crate) fn publication_buffers_prevalidated(
        &mut self,
        key: PhysicsStepKey,
    ) -> WorldStepPublicationBuffers<'_> {
        debug_assert_eq!(self.pending_external_deliveries, 0);
        debug_assert_eq!(self.pending_replacement_deliveries, 0);
        self.resolved_buffers(key)
            .expect("prevalidated complete world-step buffers must remain ready")
    }

    /// Borrow a complete result for reversible state admission before external
    /// delivery. Unlike publication, this deliberately permits unresolved
    /// markers because the preflight restores both authority and scratch.
    pub(crate) fn validation_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<WorldStepPublicationBuffers<'_>, WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        let Self {
            physics,
            external_replacement,
            replacement_active,
            lifecycle,
            sensor_generation,
            generation_elapsed_seconds,
            ambient_pellet_accumulator,
            ..
        } = self;
        let (world, rng, allocators, replacement_brains, mutation) = if *replacement_active {
            let replacement = external_replacement.validation_buffers(key)?;
            (
                replacement.world,
                replacement.rng,
                replacement.allocators,
                Some(replacement.brains),
                RunningStepMutationContract::external(replacement.proof),
            )
        } else {
            let physics = physics.publication_buffers(key)?;
            (
                physics.world,
                physics.rng,
                physics.allocators,
                None,
                RunningStepMutationContract::default(),
            )
        };
        Ok(WorldStepPublicationBuffers {
            world,
            rng,
            allocators,
            replacement_brains,
            mutation,
            baseline_lifecycle: lifecycle.as_mut().ok_or(WorldStepError::ResultNotReady)?,
            ambient_pellet_accumulator: *ambient_pellet_accumulator,
            sensor_generation: *sensor_generation,
            generation_elapsed_seconds: *generation_elapsed_seconds,
        })
    }

    fn resolved_buffers(
        &mut self,
        key: PhysicsStepKey,
    ) -> Result<WorldStepPublicationBuffers<'_>, WorldStepError> {
        if !self.ready || self.key != Some(key) {
            return Err(WorldStepError::ResultNotReady);
        }
        let Self {
            physics,
            external_replacement,
            replacement_active,
            lifecycle,
            sensor_generation,
            generation_elapsed_seconds,
            ambient_pellet_accumulator,
            ..
        } = self;
        let (world, rng, allocators, replacement_brains, mutation) = if *replacement_active {
            let replacement = external_replacement.publication_buffers(key)?;
            (
                replacement.world,
                replacement.rng,
                replacement.allocators,
                Some(replacement.brains),
                RunningStepMutationContract::external(replacement.proof),
            )
        } else {
            let physics = physics.publication_buffers(key)?;
            (
                physics.world,
                physics.rng,
                physics.allocators,
                None,
                RunningStepMutationContract::default(),
            )
        };
        Ok(WorldStepPublicationBuffers {
            world,
            rng,
            allocators,
            replacement_brains,
            mutation,
            baseline_lifecycle: lifecycle.as_mut().ok_or(WorldStepError::ResultNotReady)?,
            ambient_pellet_accumulator: *ambient_pellet_accumulator,
            sensor_generation: *sensor_generation,
            generation_elapsed_seconds: *generation_elapsed_seconds,
        })
    }

    /// Invalidate the last view after any authority-publication attempt.
    pub(crate) fn invalidate_publication(&mut self) {
        self.key = None;
        self.prepared_config = None;
        self.pending_external_deliveries = 0;
        self.pending_replacement_deliveries = 0;
        self.replacement_active = false;
        self.generation_reassignment_active = false;
        self.replacement_deferred = false;
        self.external_replacement.discard();
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
        let (world, rng, allocators, replacement_brains) = if self.replacement_active {
            (
                self.external_replacement.staged_world(control.key())?,
                self.external_replacement.staged_rng(control.key())?,
                self.external_replacement.staged_allocators(control.key())?,
                Some(self.external_replacement.staged_brains(control.key())?),
            )
        } else {
            (physics.world(), physics.rng(), physics.allocators(), None)
        };
        Ok(PreparedWorldStep {
            key: control.key(),
            config,
            control,
            world,
            rng,
            allocators,
            replacement_brains,
            lifecycle: self
                .lifecycle
                .as_ref()
                .ok_or(WorldStepError::ResultNotReady)?,
            sensor_generation: self.sensor_generation,
            diagnostics: self.diagnostics,
        })
    }
}

fn has_dead_controller(world: &WorldState) -> bool {
    world.controller_leases.iter().any(|lease| {
        !world
            .snakes
            .iter()
            .any(|snake| snake.id == lease.snake_id && snake.alive)
    })
}

fn external_observation_survives(world: &WorldState, event: PreparedExternalObservation) -> bool {
    world
        .snakes
        .iter()
        .any(|snake| snake.id == event.snake_id() && snake.alive)
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
    /// A prevalidated send-failure disconnect could not be constructed.
    Controller(Box<ControllerError>),
    /// Controlled-death replacement staging failed.
    ExternalReplacement(Box<ExternalReplacementError>),
    /// A low-level caller omitted the graph/wall replacement context.
    ExternalReplacementContextRequired,
    /// Joined step settings are internally inconsistent.
    InvalidConfig { field: &'static str },
    /// The supplied complete control boundary used different settings.
    ControlConfigMismatch { field: &'static str },
    /// Deferred completion did not match the exact prepared physics result.
    DeferredReplacementMismatch { field: &'static str },
    /// Externally delivered marker metadata no longer matches staged authority.
    ExternalDeliveryMismatch { field: &'static str },
    /// Reusable external-delivery staging could not reserve its checked size.
    ExternalDeliveryAllocation {
        buffer: &'static str,
        required: usize,
    },
    /// One or more external marker results still block publication.
    ExternalDeliveryPending { count: usize },
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

impl From<ControllerError> for WorldStepError {
    fn from(error: ControllerError) -> Self {
        Self::Controller(Box::new(error))
    }
}

impl From<ExternalReplacementError> for WorldStepError {
    fn from(error: ExternalReplacementError) -> Self {
        Self::ExternalReplacement(Box::new(error))
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
            Self::Controller(error) => write!(formatter, "{error}"),
            Self::ExternalReplacement(error) => write!(formatter, "{error}"),
            Self::ExternalReplacementContextRequired => write!(
                formatter,
                "controlled death requires the complete external-replacement context"
            ),
            Self::InvalidConfig { field } => {
                write!(formatter, "invalid complete world-step config {field}")
            }
            Self::ControlConfigMismatch { field } => {
                write!(
                    formatter,
                    "complete world-step {field} differs from control staging"
                )
            }
            Self::DeferredReplacementMismatch { field } => write!(
                formatter,
                "deferred external replacement {field} differs from prepared physics"
            ),
            Self::ExternalDeliveryMismatch { field } => {
                write!(formatter, "external delivery mismatch: {field}")
            }
            Self::ExternalDeliveryAllocation { buffer, required } => write!(
                formatter,
                "failed to reserve {required} entries for world-step {buffer}"
            ),
            Self::ExternalDeliveryPending { count } => write!(
                formatter,
                "{count} external observation deliveries still block publication"
            ),
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
            Self::Controller(error) => Some(error.as_ref()),
            Self::ExternalReplacement(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), WorldStepError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| WorldStepError::ExternalDeliveryAllocation { buffer, required })?;
    }
    Ok(())
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
