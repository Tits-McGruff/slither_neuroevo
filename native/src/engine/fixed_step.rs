//! Reusable prefix of one complete authoritative fixed-step transaction.
//!
//! This module joins the already-verified once-per-step accounting, ambient
//! pellet, and baseline-timer phases into one corrected pre-control boundary.
//! It deliberately stops before any due baseline respawn because the approved
//! plan does not yet select the owner-visible outcome for an impossible
//! mid-generation placement. Nothing in this module publishes authority.

use super::accounting::{
    StepAccountingConfig, StepAccountingDiagnostics, StepAccountingError, StepAccountingWorkspace,
};
use super::ambient::{AmbientDiagnostics, AmbientError, AmbientPelletConfig, AmbientWorkspace};
use super::baseline::{
    BaselineLifecycleConfig, BaselineLifecycleDiagnostics, BaselineLifecycleError,
    BaselineLifecycleState, BaselineLifecycleWorkspace,
};
use super::physics::{PhysicsStepKey, PhysicsStepKeyField};
use super::rng::SerializedRngState;
use super::state::{AllocatorState, BaselineRngState, ControllerLease, RngStateBundle, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First joined fixed-step-prefix contract identity.
pub const FIXED_STEP_PREFIX_VERSION: u32 = 1;

/// Complete settings and capacities consumed by the joined prefix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedStepPrefixConfig {
    /// Versioned join ordering and validation identity.
    pub algorithm_version: u32,
    /// Fixed authoritative step in simulated seconds.
    pub fixed_dt: f64,
    /// Survival-score and age settings.
    pub accounting: StepAccountingConfig,
    /// Ambient-pellet generation settings.
    pub ambient: AmbientPelletConfig,
    /// Durable baseline-slot timer settings.
    pub baseline: BaselineLifecycleConfig,
    /// Maximum admitted world snake records.
    pub maximum_snakes: usize,
    /// Maximum admitted pellet records after ambient generation.
    pub maximum_pellets: usize,
}

impl FixedStepPrefixConfig {
    /// Current TypeScript formula defaults with explicit Rust capacities.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            algorithm_version: FIXED_STEP_PREFIX_VERSION,
            fixed_dt: 1.0 / 60.0,
            accounting: StepAccountingConfig::typescript_defaults(),
            ambient: AmbientPelletConfig::typescript_defaults(),
            baseline: BaselineLifecycleConfig::typescript_defaults(),
            maximum_snakes: 512,
            maximum_pellets: 200_000,
        }
    }

    fn validate_shape(self) -> Result<(), FixedStepPrefixError> {
        if self.algorithm_version != FIXED_STEP_PREFIX_VERSION {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        if !self.fixed_dt.is_finite() || self.fixed_dt <= 0.0 || self.fixed_dt > 1.0 {
            return Err(FixedStepPrefixError::InvalidConfig { field: "fixed_dt" });
        }
        if self.maximum_snakes == 0 {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "maximum_snakes",
            });
        }
        if self.maximum_pellets == 0 {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "maximum_pellets",
            });
        }
        Ok(())
    }
}

/// Immutable inputs from one admitted authority and operation epoch.
pub struct FixedStepPrefixInputs<'source> {
    /// Complete world/generation/config/operation identity.
    pub key: PhysicsStepKey,
    /// Immutable world before this fixed step.
    pub world: &'source WorldState,
    /// Immutable gameplay RNG continuation.
    pub rng: &'source RngStateBundle,
    /// Immutable deterministic allocator continuation.
    pub allocators: &'source AllocatorState,
    /// Generation elapsed seconds before this fixed step.
    pub generation_elapsed_seconds: f64,
    /// Fractional ambient-pellet credit before this fixed step.
    pub ambient_accumulator: f64,
    /// Generation-scoped baseline timer/action state.
    pub baseline_lifecycle: &'source BaselineLifecycleState,
    /// Exact projected prefix settings and capacities.
    pub config: FixedStepPrefixConfig,
}

/// Retained storage and phase-work diagnostics for the latest prefix attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedStepPrefixDiagnostics {
    /// Once-per-step accounting work.
    pub accounting: StepAccountingDiagnostics,
    /// Ambient generation work.
    pub ambient: AmbientDiagnostics,
    /// Baseline timer work.
    pub baseline: BaselineLifecycleDiagnostics,
    /// Retained working snake capacity.
    pub snake_capacity: usize,
    /// Retained working body-point capacity.
    pub body_point_capacity: usize,
    /// Retained working pellet capacity.
    pub pellet_capacity: usize,
    /// Retained working controller-lease capacity.
    pub controller_lease_capacity: usize,
    /// Retained working baseline-slot capacity.
    pub baseline_slot_capacity: usize,
    /// Retained working per-baseline RNG capacity.
    pub baseline_rng_capacity: usize,
    /// Retained controller scope/token string capacity.
    pub controller_text_capacity: usize,
    /// Retained RNG string capacity, including logically absent Gaussian spares.
    pub rng_text_capacity: usize,
}

/// String storage retained while a serialized RNG has no logical Gaussian spare.
#[derive(Debug, Default)]
struct RngCopyScratch {
    world_gaussian_spare: String,
    evolution_gaussian_spare: String,
    external_gaussian_spare: String,
    baseline_gaussian_spares: Vec<String>,
}

/// Complete, still non-authoritative pre-control boundary.
#[derive(Clone, Copy, Debug)]
pub struct PreparedFixedStepPrefix<'workspace, 'source> {
    key: PhysicsStepKey,
    source_world: &'source WorldState,
    source_rng: &'source RngStateBundle,
    source_allocators: &'source AllocatorState,
    source_lifecycle: &'source BaselineLifecycleState,
    source_elapsed_seconds: f64,
    source_ambient_accumulator: f64,
    config: FixedStepPrefixConfig,
    world: &'workspace WorldState,
    rng: &'workspace RngStateBundle,
    allocators: &'workspace AllocatorState,
    lifecycle: &'workspace BaselineLifecycleState,
    generation_elapsed_seconds: f64,
    ambient_accumulator: f64,
    diagnostics: FixedStepPrefixDiagnostics,
}

impl<'workspace, 'source> PreparedFixedStepPrefix<'workspace, 'source> {
    /// Exact authority/config/operation identity prepared.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Exact prefix settings bound to this prepared boundary.
    #[must_use]
    pub const fn config(self) -> FixedStepPrefixConfig {
        self.config
    }

    /// Combined accounting-plus-ambient world used by every controller class.
    #[must_use]
    pub const fn world(self) -> &'workspace WorldState {
        self.world
    }

    /// RNG continuation after ambient generation.
    #[must_use]
    pub const fn rng(self) -> &'workspace RngStateBundle {
        self.rng
    }

    /// Allocator continuation after ambient entity-ID reservation.
    #[must_use]
    pub const fn allocators(self) -> &'workspace AllocatorState {
        self.allocators
    }

    /// Baseline timers after this fixed-step boundary.
    #[must_use]
    pub const fn baseline_lifecycle(self) -> &'workspace BaselineLifecycleState {
        self.lifecycle
    }

    /// Generation elapsed seconds after exactly one fixed increment.
    #[must_use]
    pub const fn generation_elapsed_seconds(self) -> f64 {
        self.generation_elapsed_seconds
    }

    /// Fractional ambient-pellet credit after realized spawns.
    #[must_use]
    pub const fn ambient_accumulator(self) -> f64 {
        self.ambient_accumulator
    }

    /// Phase work and retained allocation diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> FixedStepPrefixDiagnostics {
        self.diagnostics
    }

    /// Revalidate all source provenance before a later control join accepts it.
    #[allow(clippy::too_many_arguments)]
    pub fn validate_current(
        self,
        current_key: PhysicsStepKey,
        current_world: &WorldState,
        current_rng: &RngStateBundle,
        current_allocators: &AllocatorState,
        current_elapsed_seconds: f64,
        current_ambient_accumulator: f64,
        current_lifecycle: &BaselineLifecycleState,
        current_config: FixedStepPrefixConfig,
    ) -> Result<(), FixedStepPrefixError> {
        if let Some(field) = self.key.first_mismatch(current_key) {
            return Err(FixedStepPrefixError::StepKeyMismatch { field });
        }
        if !std::ptr::eq(self.source_world, current_world) {
            return Err(FixedStepPrefixError::SourceChanged { field: "world" });
        }
        if !std::ptr::eq(self.source_rng, current_rng) {
            return Err(FixedStepPrefixError::SourceChanged {
                field: "RNG bundle",
            });
        }
        if !std::ptr::eq(self.source_allocators, current_allocators) {
            return Err(FixedStepPrefixError::SourceChanged {
                field: "allocators",
            });
        }
        if !std::ptr::eq(self.source_lifecycle, current_lifecycle) {
            return Err(FixedStepPrefixError::SourceChanged {
                field: "baseline lifecycle",
            });
        }
        if self.source_elapsed_seconds.to_bits() != current_elapsed_seconds.to_bits() {
            return Err(FixedStepPrefixError::SourceChanged {
                field: "generation elapsed time",
            });
        }
        if self.source_ambient_accumulator.to_bits() != current_ambient_accumulator.to_bits() {
            return Err(FixedStepPrefixError::SourceChanged {
                field: "ambient accumulator",
            });
        }
        if self.config != current_config {
            return Err(FixedStepPrefixError::SourceChanged { field: "config" });
        }
        Ok(())
    }
}

/// Reusable owner of the corrected fixed-step prefix.
#[derive(Debug, Default)]
pub struct FixedStepPrefixWorkspace {
    accounting: StepAccountingWorkspace,
    ambient: AmbientWorkspace,
    baseline: BaselineLifecycleWorkspace,
    world: WorldState,
    rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    allocators: Option<AllocatorState>,
    lifecycle: Option<BaselineLifecycleState>,
    generation_elapsed_seconds: f64,
    ambient_accumulator: f64,
    ready: bool,
}

impl FixedStepPrefixWorkspace {
    /// Construct empty reusable prefix scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare the corrected pre-control boundary without publishing authority.
    ///
    /// Accounting, ambient generation and timer staging all read the same
    /// immutable source. This is valid because accounting changes only snake
    /// age/points, while ambient generation reads only pellets/RNG/allocator and
    /// timers read only slot liveness. Their outputs are joined only after all
    /// validations and storage reservations succeed.
    pub fn prepare<'workspace, 'source>(
        &'workspace mut self,
        inputs: FixedStepPrefixInputs<'source>,
    ) -> Result<PreparedFixedStepPrefix<'workspace, 'source>, FixedStepPrefixError> {
        self.ready = false;
        inputs.config.validate_shape()?;

        let accounting = self.accounting.prepare(
            inputs.key,
            inputs.world,
            inputs.generation_elapsed_seconds,
            inputs.config.fixed_dt,
            inputs.config.accounting,
            inputs.config.maximum_snakes,
        )?;
        let ambient = self.ambient.prepare(
            inputs.key,
            inputs.world,
            &inputs.rng.world,
            inputs.allocators,
            inputs.ambient_accumulator,
            accounting.next_elapsed_seconds(),
            inputs.config.fixed_dt,
            inputs.config.ambient,
            inputs.config.maximum_pellets,
        )?;
        let baseline = self.baseline.prepare_timers(
            inputs.key,
            inputs.world,
            inputs.baseline_lifecycle,
            inputs.config.fixed_dt,
            inputs.config.baseline,
        )?;
        if baseline.requires_respawn_resolution() {
            return Err(FixedStepPrefixError::BaselineRespawnsRequireDecision {
                count: baseline.due_slots().len(),
            });
        }

        accounting.validate_current(
            inputs.key,
            inputs.world,
            inputs.generation_elapsed_seconds,
            inputs.config.fixed_dt,
            inputs.config.accounting,
            inputs.config.maximum_snakes,
        )?;
        ambient.validate_current(
            inputs.key,
            inputs.world,
            &inputs.rng.world,
            inputs.allocators,
            inputs.ambient_accumulator,
            accounting.next_elapsed_seconds(),
            inputs.config.fixed_dt,
            inputs.config.ambient,
            inputs.config.maximum_pellets,
        )?;
        baseline.validate_current(
            inputs.key,
            inputs.world,
            inputs.baseline_lifecycle,
            inputs.config.fixed_dt,
            inputs.config.baseline,
        )?;

        let required_pellets = inputs
            .world
            .pellets
            .len()
            .checked_add(ambient.generated().len())
            .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                context: "joined ambient pellet count",
            })?;
        copy_world_reusing(&mut self.world, inputs.world, required_pellets)?;
        copy_rng_bundle_reusing(&mut self.rng, &mut self.rng_copy_scratch, inputs.rng)?;
        copy_lifecycle_reusing(&mut self.lifecycle, inputs.baseline_lifecycle)?;
        match &mut self.allocators {
            Some(current) => current.clone_from(inputs.allocators),
            None => self.allocators = Some(inputs.allocators.clone()),
        }

        let next_elapsed = accounting.apply_to_working_copy(
            inputs.key,
            inputs.world,
            inputs.generation_elapsed_seconds,
            inputs.config.fixed_dt,
            inputs.config.accounting,
            inputs.config.maximum_snakes,
            &mut self.world,
        )?;
        baseline.apply_without_due_respawns(
            inputs.key,
            inputs.world,
            inputs.baseline_lifecycle,
            inputs.config.fixed_dt,
            inputs.config.baseline,
            self.lifecycle
                .as_mut()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
        )?;

        if self.world.pellets != inputs.world.pellets {
            return Err(FixedStepPrefixError::InternalShapeMismatch);
        }
        self.world.pellets.extend_from_slice(ambient.generated());
        if self.world.pellets.len() != required_pellets {
            return Err(FixedStepPrefixError::InternalShapeMismatch);
        }
        let rng = self
            .rng
            .as_mut()
            .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
        copy_serialized_rng_reusing(
            &mut rng.world,
            ambient.next_rng(),
            &mut self.rng_copy_scratch.world_gaussian_spare,
        )?;
        self.allocators
            .as_mut()
            .ok_or(FixedStepPrefixError::InternalShapeMismatch)?
            .clone_from(ambient.next_allocators());
        self.generation_elapsed_seconds = next_elapsed;
        self.ambient_accumulator = ambient.next_accumulator();
        self.ready = true;

        self.prepared(inputs)
    }

    /// Whether the latest attempt produced a complete prefix.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Latest phase and retained-capacity diagnostics, including after failure.
    #[must_use]
    pub fn diagnostics(&self) -> FixedStepPrefixDiagnostics {
        FixedStepPrefixDiagnostics {
            accounting: self.accounting.diagnostics(),
            ambient: self.ambient.diagnostics(),
            baseline: self.baseline.diagnostics(),
            snake_capacity: self.world.snakes.capacity(),
            body_point_capacity: self.world.body_points.capacity(),
            pellet_capacity: self.world.pellets.capacity(),
            controller_lease_capacity: self.world.controller_leases.capacity(),
            baseline_slot_capacity: self
                .lifecycle
                .as_ref()
                .map_or(0, |state| state.slots.capacity()),
            baseline_rng_capacity: self.rng.as_ref().map_or(0, |rng| rng.baselines.capacity()),
            controller_text_capacity: controller_text_capacity(&self.world.controller_leases),
            rng_text_capacity: rng_text_capacity(self.rng.as_ref(), &self.rng_copy_scratch),
        }
    }

    fn prepared<'workspace, 'source>(
        &'workspace self,
        inputs: FixedStepPrefixInputs<'source>,
    ) -> Result<PreparedFixedStepPrefix<'workspace, 'source>, FixedStepPrefixError> {
        if !self.ready {
            return Err(FixedStepPrefixError::ResultNotReady);
        }
        Ok(PreparedFixedStepPrefix {
            key: inputs.key,
            source_world: inputs.world,
            source_rng: inputs.rng,
            source_allocators: inputs.allocators,
            source_lifecycle: inputs.baseline_lifecycle,
            source_elapsed_seconds: inputs.generation_elapsed_seconds,
            source_ambient_accumulator: inputs.ambient_accumulator,
            config: inputs.config,
            world: &self.world,
            rng: self
                .rng
                .as_ref()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
            allocators: self
                .allocators
                .as_ref()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
            lifecycle: self
                .lifecycle
                .as_ref()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
            generation_elapsed_seconds: self.generation_elapsed_seconds,
            ambient_accumulator: self.ambient_accumulator,
            diagnostics: self.diagnostics(),
        })
    }
}

fn copy_world_reusing(
    target: &mut WorldState,
    source: &WorldState,
    required_pellets: usize,
) -> Result<(), FixedStepPrefixError> {
    reserve_for(&mut target.snakes, source.snakes.len(), "working snakes")?;
    reserve_for(
        &mut target.body_points,
        source.body_points.len(),
        "working body points",
    )?;
    reserve_for(&mut target.pellets, required_pellets, "working pellets")?;
    reserve_for(
        &mut target.controller_leases,
        source.controller_leases.len(),
        "working controller leases",
    )?;

    target.snakes.clear();
    target.snakes.extend_from_slice(&source.snakes);
    target.body_points.clear();
    target.body_points.extend_from_slice(&source.body_points);
    target.pellets.clear();
    target.pellets.extend_from_slice(&source.pellets);
    copy_controller_leases_reusing(&mut target.controller_leases, &source.controller_leases)
}

fn copy_controller_leases_reusing(
    target: &mut Vec<ControllerLease>,
    source: &[ControllerLease],
) -> Result<(), FixedStepPrefixError> {
    let common = target.len().min(source.len());
    for index in 0..common {
        copy_lease_reusing(&mut target[index], &source[index])?;
    }
    target.truncate(source.len());
    for lease in &source[common..] {
        let mut scope = String::new();
        reserve_string(&mut scope, lease.scope.len(), "controller scope")?;
        scope.push_str(&lease.scope);
        let mut resume_token = String::new();
        reserve_string(
            &mut resume_token,
            lease.resume_token.len(),
            "controller resume token",
        )?;
        resume_token.push_str(&lease.resume_token);
        target.push(ControllerLease {
            id: lease.id,
            snake_id: lease.snake_id,
            kind: lease.kind,
            connection_id: lease.connection_id,
            scope,
            resume_token,
            status: lease.status,
            latest_action: lease.latest_action,
            last_observed_at_ms: lease.last_observed_at_ms,
            disconnected_at_ms: lease.disconnected_at_ms,
            input_hold_expires_at_ms: lease.input_hold_expires_at_ms,
            grace_expires_at_ms: lease.grace_expires_at_ms,
            takeover_committed_at_ms: lease.takeover_committed_at_ms,
        });
    }
    Ok(())
}

fn copy_lease_reusing(
    target: &mut ControllerLease,
    source: &ControllerLease,
) -> Result<(), FixedStepPrefixError> {
    reserve_string(&mut target.scope, source.scope.len(), "controller scope")?;
    reserve_string(
        &mut target.resume_token,
        source.resume_token.len(),
        "controller resume token",
    )?;
    target.id = source.id;
    target.snake_id = source.snake_id;
    target.kind = source.kind;
    target.connection_id = source.connection_id;
    target.scope.clear();
    target.scope.push_str(&source.scope);
    target.resume_token.clear();
    target.resume_token.push_str(&source.resume_token);
    target.status = source.status;
    target.latest_action = source.latest_action;
    target.last_observed_at_ms = source.last_observed_at_ms;
    target.disconnected_at_ms = source.disconnected_at_ms;
    target.input_hold_expires_at_ms = source.input_hold_expires_at_ms;
    target.grace_expires_at_ms = source.grace_expires_at_ms;
    target.takeover_committed_at_ms = source.takeover_committed_at_ms;
    Ok(())
}

fn copy_rng_bundle_reusing(
    target: &mut Option<RngStateBundle>,
    scratch: &mut RngCopyScratch,
    source: &RngStateBundle,
) -> Result<(), FixedStepPrefixError> {
    if target.is_none() {
        *target = Some(RngStateBundle {
            version: source.version,
            world: empty_serialized_rng(),
            evolution: empty_serialized_rng(),
            external_controller: empty_serialized_rng(),
            baselines: Vec::new(),
        });
    }
    let target = target
        .as_mut()
        .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
    target.version = source.version;
    copy_serialized_rng_reusing(
        &mut target.world,
        &source.world,
        &mut scratch.world_gaussian_spare,
    )?;
    copy_serialized_rng_reusing(
        &mut target.evolution,
        &source.evolution,
        &mut scratch.evolution_gaussian_spare,
    )?;
    copy_serialized_rng_reusing(
        &mut target.external_controller,
        &source.external_controller,
        &mut scratch.external_gaussian_spare,
    )?;
    reserve_for(
        &mut target.baselines,
        source.baselines.len(),
        "baseline RNG states",
    )?;
    reserve_for(
        &mut scratch.baseline_gaussian_spares,
        source.baselines.len(),
        "baseline Gaussian spare buffers",
    )?;
    while scratch.baseline_gaussian_spares.len() < source.baselines.len() {
        scratch.baseline_gaussian_spares.push(String::new());
    }
    let common = target.baselines.len().min(source.baselines.len());
    for index in 0..common {
        target.baselines[index].slot = source.baselines[index].slot;
        copy_serialized_rng_reusing(
            &mut target.baselines[index].state,
            &source.baselines[index].state,
            &mut scratch.baseline_gaussian_spares[index],
        )?;
    }
    target.baselines.truncate(source.baselines.len());
    for (index, baseline) in source.baselines[common..].iter().enumerate() {
        let mut state = empty_serialized_rng();
        copy_serialized_rng_reusing(
            &mut state,
            &baseline.state,
            &mut scratch.baseline_gaussian_spares[common + index],
        )?;
        target.baselines.push(BaselineRngState {
            slot: baseline.slot,
            state,
        });
    }
    Ok(())
}

fn copy_serialized_rng_reusing(
    target: &mut SerializedRngState,
    source: &SerializedRngState,
    retained_gaussian_spare: &mut String,
) -> Result<(), FixedStepPrefixError> {
    reserve_string(
        &mut target.algorithm,
        source.algorithm.len(),
        "RNG algorithm",
    )?;
    reserve_string(&mut target.state_hex, source.state_hex.len(), "RNG state")?;
    reserve_string(
        &mut target.gaussian_algorithm,
        source.gaussian_algorithm.len(),
        "Gaussian algorithm",
    )?;
    match &source.gaussian_spare_hex {
        Some(source_spare) => {
            if target.gaussian_spare_hex.is_none() {
                reserve_string(
                    retained_gaussian_spare,
                    source_spare.len(),
                    "Gaussian spare",
                )?;
                target.gaussian_spare_hex = Some(std::mem::take(retained_gaussian_spare));
            }
            let target_spare = target
                .gaussian_spare_hex
                .as_mut()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            reserve_string(target_spare, source_spare.len(), "Gaussian spare")?;
            target_spare.clear();
            target_spare.push_str(source_spare);
        }
        None => {
            if let Some(mut spare) = target.gaussian_spare_hex.take() {
                spare.clear();
                if spare.capacity() > retained_gaussian_spare.capacity() {
                    *retained_gaussian_spare = spare;
                }
            }
        }
    }
    target.algorithm.clear();
    target.algorithm.push_str(&source.algorithm);
    target.version = source.version;
    target.state_hex.clear();
    target.state_hex.push_str(&source.state_hex);
    target.gaussian_algorithm.clear();
    target
        .gaussian_algorithm
        .push_str(&source.gaussian_algorithm);
    target.gaussian_version = source.gaussian_version;
    target.gaussian_spare_valid = source.gaussian_spare_valid;
    Ok(())
}

fn controller_text_capacity(leases: &[ControllerLease]) -> usize {
    leases.iter().fold(0usize, |total, lease| {
        total
            .saturating_add(lease.scope.capacity())
            .saturating_add(lease.resume_token.capacity())
    })
}

fn rng_text_capacity(rng: Option<&RngStateBundle>, scratch: &RngCopyScratch) -> usize {
    let scratch_capacity = scratch
        .world_gaussian_spare
        .capacity()
        .saturating_add(scratch.evolution_gaussian_spare.capacity())
        .saturating_add(scratch.external_gaussian_spare.capacity())
        .saturating_add(
            scratch
                .baseline_gaussian_spares
                .iter()
                .fold(0usize, |total, value| {
                    total.saturating_add(value.capacity())
                }),
        );
    rng.map_or(scratch_capacity, |bundle| {
        let fixed = serialized_rng_text_capacity(&bundle.world)
            .saturating_add(serialized_rng_text_capacity(&bundle.evolution))
            .saturating_add(serialized_rng_text_capacity(&bundle.external_controller));
        bundle
            .baselines
            .iter()
            .fold(scratch_capacity.saturating_add(fixed), |total, baseline| {
                total.saturating_add(serialized_rng_text_capacity(&baseline.state))
            })
    })
}

fn serialized_rng_text_capacity(state: &SerializedRngState) -> usize {
    state
        .algorithm
        .capacity()
        .saturating_add(state.state_hex.capacity())
        .saturating_add(state.gaussian_algorithm.capacity())
        .saturating_add(
            state
                .gaussian_spare_hex
                .as_ref()
                .map_or(0, String::capacity),
        )
}

fn empty_serialized_rng() -> SerializedRngState {
    SerializedRngState {
        algorithm: String::new(),
        version: 0,
        state_hex: String::new(),
        gaussian_algorithm: String::new(),
        gaussian_version: 0,
        gaussian_spare_valid: false,
        gaussian_spare_hex: None,
    }
}

fn copy_lifecycle_reusing(
    target: &mut Option<BaselineLifecycleState>,
    source: &BaselineLifecycleState,
) -> Result<(), FixedStepPrefixError> {
    if target.is_none() {
        *target = Some(BaselineLifecycleState {
            version: source.version,
            slots: Vec::new(),
        });
    }
    let target = target
        .as_mut()
        .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
    reserve_for(
        &mut target.slots,
        source.slots.len(),
        "working baseline slots",
    )?;
    target.version = source.version;
    target.slots.clear();
    target.slots.extend_from_slice(&source.slots);
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), FixedStepPrefixError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| FixedStepPrefixError::AllocationFailed { context, required })?;
    }
    Ok(())
}

fn reserve_string(
    value: &mut String,
    required: usize,
    context: &'static str,
) -> Result<(), FixedStepPrefixError> {
    if value.capacity() < required {
        value
            .try_reserve_exact(required.saturating_sub(value.len()))
            .map_err(|_| FixedStepPrefixError::AllocationFailed { context, required })?;
    }
    Ok(())
}

/// Rejected fixed-step-prefix preparation. No variant publishes authority.
#[derive(Debug)]
pub enum FixedStepPrefixError {
    /// Joined prefix settings or capacity are invalid.
    InvalidConfig { field: &'static str },
    /// One exact authority/config/operation key component changed.
    StepKeyMismatch { field: PhysicsStepKeyField },
    /// A non-key source input changed after preparation.
    SourceChanged { field: &'static str },
    /// Checked size arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Reusable storage could not be reserved before writes.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// One or more due baseline slots require the reviewed placement policy.
    BaselineRespawnsRequireDecision { count: usize },
    /// A complete prefix is not available.
    ResultNotReady,
    /// Internal joined-buffer shape disagreed after successful phase validation.
    InternalShapeMismatch,
    /// Once-per-step accounting rejected the boundary.
    Accounting(Box<StepAccountingError>),
    /// Ambient generation rejected the boundary.
    Ambient(Box<AmbientError>),
    /// Baseline lifecycle rejected the boundary.
    Baseline(Box<BaselineLifecycleError>),
}

impl From<StepAccountingError> for FixedStepPrefixError {
    fn from(error: StepAccountingError) -> Self {
        Self::Accounting(Box::new(error))
    }
}

impl From<AmbientError> for FixedStepPrefixError {
    fn from(error: AmbientError) -> Self {
        Self::Ambient(Box::new(error))
    }
}

impl From<BaselineLifecycleError> for FixedStepPrefixError {
    fn from(error: BaselineLifecycleError) -> Self {
        Self::Baseline(Box::new(error))
    }
}

impl Display for FixedStepPrefixError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => write!(formatter, "invalid fixed-step prefix {field}"),
            Self::StepKeyMismatch { field } => {
                write!(formatter, "fixed-step prefix key changed at {field:?}")
            }
            Self::SourceChanged { field } => {
                write!(formatter, "fixed-step prefix source {field} changed")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "fixed-step prefix overflow in {context}")
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "failed to reserve {required} entries for {context}"
            ),
            Self::BaselineRespawnsRequireDecision { count } => write!(
                formatter,
                "{count} due baseline respawns require the reviewed placement-failure rule"
            ),
            Self::ResultNotReady => write!(formatter, "fixed-step prefix result is not ready"),
            Self::InternalShapeMismatch => {
                write!(formatter, "fixed-step prefix internal shape mismatch")
            }
            Self::Accounting(error) => Display::fmt(error, formatter),
            Self::Ambient(error) => Display::fmt(error, formatter),
            Self::Baseline(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for FixedStepPrefixError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Accounting(error) => Some(error.as_ref()),
            Self::Ambient(error) => Some(error.as_ref()),
            Self::Baseline(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ambient::AMBIENT_PELLET_ALGORITHM_VERSION;
    use crate::engine::baseline::{BaselineSlotRuntime, BASELINE_LIFECYCLE_VERSION};
    use crate::engine::rng::StatefulRng;
    use crate::engine::state::{
        BaselineStrategyState, BodyRange, ControllerKind, ControllerLeaseStatus,
        LatestControllerAction, SnakeKind, SnakeState, WorldPoint, ALLOCATOR_VERSION,
        RNG_BUNDLE_VERSION,
    };

    const DT: f64 = 1.0 / 60.0;

    fn key(operation_epoch: u64) -> PhysicsStepKey {
        PhysicsStepKey::new(7, 3, 40, 2, 9, [0x5a; 32], operation_epoch)
    }

    fn snake(id: u64, kind: SnakeKind, body_start: usize) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).unwrap_or(7),
            kind,
            alive: true,
            population_slot: (kind == SnakeKind::Evolved).then_some(0),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position: WorldPoint {
                x: (id % 10_000) as f64,
                y: 0.0,
            },
            previous_position: WorldPoint {
                x: (id % 10_000) as f64,
                y: 0.0,
            },
            direction: 0.0,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 1.0,
            food: 0.0,
            points: 5.0,
            kills: 0,
            target_length: 2.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: body_start,
                len: 2,
            },
            skin: 0,
        }
    }

    fn world() -> WorldState {
        let snake = snake(10, SnakeKind::Evolved, 0);
        WorldState {
            body_points: vec![
                snake.position,
                WorldPoint {
                    x: snake.position.x - 7.5,
                    y: 0.0,
                },
            ],
            snakes: vec![snake],
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        }
    }

    fn rng_bundle(baseline_count: usize) -> RngStateBundle {
        RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: StatefulRng::new(101.0).export_state(),
            evolution: StatefulRng::new(202.0).export_state(),
            external_controller: StatefulRng::new(303.0).export_state(),
            baselines: (0..baseline_count)
                .map(|slot| BaselineRngState {
                    slot: u32::try_from(slot).expect("test slot must fit"),
                    state: StatefulRng::new(404.0 + slot as f64).export_state(),
                })
                .collect(),
        }
    }

    fn allocators() -> AllocatorState {
        AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 100,
            next_brain_id: 1,
            next_genome_id: 1,
            next_controller_lease_id: 1,
            next_frame_v1_id: 100,
            next_external_id: 1_000_000_000_000,
            next_baseline_id: 2_000_000_000_000,
            next_resurrected_id: 3_000_000_000_000,
        }
    }

    fn lifecycle() -> BaselineLifecycleState {
        BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: Vec::new(),
        }
    }

    fn world_with_waiting_baseline() -> (WorldState, BaselineLifecycleState) {
        let mut source_world = world();
        let mut baseline = snake(2_000_000_000_010, SnakeKind::Baseline, 2);
        baseline.alive = false;
        baseline.population_slot = None;
        baseline.baseline_slot = Some(0);
        baseline.baseline_strategy = Some(BaselineStrategyState::Roam);
        source_world.body_points.extend([
            baseline.position,
            WorldPoint {
                x: baseline.position.x - 7.5,
                y: 0.0,
            },
        ]);
        source_world.snakes.push(baseline.clone());
        let lifecycle = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: baseline.id,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(10.0),
            }],
        };
        (source_world, lifecycle)
    }

    fn config(slot_count: usize) -> FixedStepPrefixConfig {
        let mut ambient = AmbientPelletConfig::typescript_defaults();
        ambient.algorithm_version = AMBIENT_PELLET_ALGORITHM_VERSION;
        ambient.target_count = 1;
        ambient.spawn_per_second = 60.0;
        FixedStepPrefixConfig {
            fixed_dt: DT,
            ambient,
            baseline: BaselineLifecycleConfig {
                slot_count,
                ..BaselineLifecycleConfig::typescript_defaults()
            },
            maximum_snakes: 16,
            maximum_pellets: 16,
            ..FixedStepPrefixConfig::typescript_defaults()
        }
    }

    #[test]
    fn prefix_joins_accounting_ambient_and_timers_without_authority_write() {
        let (source_world, source_lifecycle) = world_with_waiting_baseline();
        let source_rng = rng_bundle(1);
        let source_allocators = allocators();
        let original_world = source_world.clone();
        let original_rng = source_rng.clone();
        let original_allocators = source_allocators.clone();
        let original_lifecycle = source_lifecycle.clone();
        let mut workspace = FixedStepPrefixWorkspace::new();

        let prepared = workspace
            .prepare(FixedStepPrefixInputs {
                key: key(1),
                world: &source_world,
                rng: &source_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: config(1),
            })
            .expect("complete prefix must prepare");

        assert_eq!(source_world, original_world);
        assert_eq!(source_rng, original_rng);
        assert_eq!(source_allocators, original_allocators);
        assert_eq!(source_lifecycle, original_lifecycle);
        assert_eq!(prepared.key(), key(1));
        assert_eq!(prepared.world().snakes[0].age_seconds, 1.0 + DT);
        assert_eq!(
            prepared.world().snakes[0].points,
            5.0 + DT * StepAccountingConfig::typescript_defaults().points_per_second_alive
        );
        assert_eq!(prepared.world().pellets.len(), 1);
        assert_eq!(prepared.world().pellets[0].id, 100);
        assert_ne!(prepared.rng().world, source_rng.world);
        assert_eq!(prepared.rng().evolution, source_rng.evolution);
        assert_eq!(
            prepared.rng().external_controller,
            source_rng.external_controller
        );
        assert_eq!(prepared.rng().baselines, source_rng.baselines);
        assert_eq!(prepared.allocators().next_entity_id, 101);
        assert_eq!(prepared.generation_elapsed_seconds(), 4.0 + DT);
        assert_eq!(prepared.ambient_accumulator(), 0.0);
        assert_eq!(
            prepared.baseline_lifecycle().slots[0].respawn_remaining_seconds,
            Some(10.0 - DT)
        );
        assert!(workspace.is_ready());
    }

    #[test]
    fn due_baseline_respawn_stays_explicit_and_no_prefix_becomes_ready() {
        let mut source_world = world();
        source_world.snakes[0] = snake(2_000_000_000_010, SnakeKind::Baseline, 0);
        source_world.snakes[0].alive = false;
        source_world.snakes[0].population_slot = None;
        source_world.snakes[0].baseline_slot = Some(0);
        source_world.snakes[0].baseline_strategy = Some(BaselineStrategyState::Roam);
        let source_rng = rng_bundle(1);
        let source_allocators = allocators();
        let source_lifecycle = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: source_world.snakes[0].id,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(DT * 0.5),
            }],
        };
        let original = source_world.clone();
        let mut workspace = FixedStepPrefixWorkspace::new();

        let error = workspace
            .prepare(FixedStepPrefixInputs {
                key: key(2),
                world: &source_world,
                rng: &source_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: config(1),
            })
            .expect_err("due respawn must await the reviewed placement rule");
        assert!(matches!(
            error,
            FixedStepPrefixError::BaselineRespawnsRequireDecision { count: 1 }
        ));
        assert!(!workspace.is_ready());
        assert_eq!(source_world, original);
    }

    #[test]
    fn complete_source_provenance_rejects_every_stale_join_input() {
        let source_world = world();
        let source_rng = rng_bundle(0);
        let source_allocators = allocators();
        let source_lifecycle = lifecycle();
        let prefix_config = config(0);
        let mut workspace = FixedStepPrefixWorkspace::new();
        let prepared = workspace
            .prepare(FixedStepPrefixInputs {
                key: key(3),
                world: &source_world,
                rng: &source_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: prefix_config,
            })
            .unwrap();

        let other_world = source_world.clone();
        let other_rng = source_rng.clone();
        let other_allocators = source_allocators.clone();
        let other_lifecycle = source_lifecycle.clone();
        let mut changed_config = prefix_config;
        changed_config.maximum_pellets += 1;
        let cases = [
            prepared.validate_current(
                key(4),
                &source_world,
                &source_rng,
                &source_allocators,
                4.0,
                0.0,
                &source_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &other_world,
                &source_rng,
                &source_allocators,
                4.0,
                0.0,
                &source_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &source_world,
                &other_rng,
                &source_allocators,
                4.0,
                0.0,
                &source_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &source_world,
                &source_rng,
                &other_allocators,
                4.0,
                0.0,
                &source_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &source_world,
                &source_rng,
                &source_allocators,
                5.0,
                0.0,
                &source_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &source_world,
                &source_rng,
                &source_allocators,
                4.0,
                1.0,
                &source_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &source_world,
                &source_rng,
                &source_allocators,
                4.0,
                0.0,
                &other_lifecycle,
                prefix_config,
            ),
            prepared.validate_current(
                key(3),
                &source_world,
                &source_rng,
                &source_allocators,
                4.0,
                0.0,
                &source_lifecycle,
                changed_config,
            ),
        ];
        assert!(cases.iter().all(Result::is_err));
    }

    #[test]
    fn warmed_prefix_reuses_every_reported_capacity_with_controller_text() {
        let mut source_world = world();
        let mut external = snake(1_000_000_000_010, SnakeKind::External, 2);
        external.population_slot = None;
        source_world.body_points.extend([
            external.position,
            WorldPoint {
                x: external.position.x - 7.5,
                y: 0.0,
            },
        ]);
        source_world.snakes.push(external.clone());
        source_world.controller_leases.push(ControllerLease {
            id: 1,
            snake_id: external.id,
            kind: ControllerKind::Player,
            connection_id: Some(8),
            scope: "run-with-retained-controller-text".to_owned(),
            resume_token: "0123456789abcdef0123456789abcdef".to_owned(),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                turn: 0.25,
                boost: false,
                client_tick: 1,
                arrival_sequence: 1,
                accepted_at_ms: 100,
            },
            last_observed_at_ms: 100,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        });
        let source_rng = rng_bundle(0);
        let source_allocators = allocators();
        let source_lifecycle = lifecycle();
        let mut prefix_config = config(0);
        prefix_config.ambient.target_count = 0;
        prefix_config.ambient.spawn_per_second = 0.0;
        let mut workspace = FixedStepPrefixWorkspace::new();

        workspace
            .prepare(FixedStepPrefixInputs {
                key: key(10),
                world: &source_world,
                rng: &source_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: prefix_config,
            })
            .unwrap();
        let warmed = workspace.diagnostics();
        for operation_epoch in 11..35 {
            let prepared = workspace
                .prepare(FixedStepPrefixInputs {
                    key: key(operation_epoch),
                    world: &source_world,
                    rng: &source_rng,
                    allocators: &source_allocators,
                    generation_elapsed_seconds: 4.0,
                    ambient_accumulator: 0.0,
                    baseline_lifecycle: &source_lifecycle,
                    config: prefix_config,
                })
                .unwrap();
            assert_eq!(
                prepared.world().controller_leases,
                source_world.controller_leases
            );
            assert_eq!(workspace.diagnostics(), warmed);
        }
    }

    #[test]
    fn gaussian_spare_toggles_reuse_retained_string_storage() {
        let (source_world, source_lifecycle) = world_with_waiting_baseline();
        let source_allocators = allocators();
        let mut with_spares = rng_bundle(1);
        let mut world_rng = StatefulRng::new(101.0);
        let mut evolution_rng = StatefulRng::new(202.0);
        let mut external_rng = StatefulRng::new(303.0);
        let mut baseline_rng = StatefulRng::new(404.0);
        let _ = world_rng.gaussian();
        let _ = evolution_rng.gaussian();
        let _ = external_rng.gaussian();
        let _ = baseline_rng.gaussian();
        with_spares.world = world_rng.export_state();
        with_spares.evolution = evolution_rng.export_state();
        with_spares.external_controller = external_rng.export_state();
        with_spares.baselines[0].state = baseline_rng.export_state();
        assert!(with_spares.world.gaussian_spare_hex.is_some());

        let mut without_spares = with_spares.clone();
        for state in [
            &mut without_spares.world,
            &mut without_spares.evolution,
            &mut without_spares.external_controller,
            &mut without_spares.baselines[0].state,
        ] {
            state.gaussian_spare_valid = false;
            state.gaussian_spare_hex = None;
        }

        let mut prefix_config = config(1);
        prefix_config.ambient.target_count = 0;
        prefix_config.ambient.spawn_per_second = 0.0;
        let mut workspace = FixedStepPrefixWorkspace::new();
        workspace
            .prepare(FixedStepPrefixInputs {
                key: key(40),
                world: &source_world,
                rng: &with_spares,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: prefix_config,
            })
            .unwrap();
        let warmed = workspace.diagnostics();
        assert!(warmed.rng_text_capacity > 0);

        for operation_epoch in 41..65 {
            let expected_rng = if operation_epoch % 2 == 0 {
                &with_spares
            } else {
                &without_spares
            };
            let prepared = workspace
                .prepare(FixedStepPrefixInputs {
                    key: key(operation_epoch),
                    world: &source_world,
                    rng: expected_rng,
                    allocators: &source_allocators,
                    generation_elapsed_seconds: 4.0,
                    ambient_accumulator: 0.0,
                    baseline_lifecycle: &source_lifecycle,
                    config: prefix_config,
                })
                .unwrap();
            assert_eq!(prepared.rng(), expected_rng);
            assert_eq!(workspace.diagnostics(), warmed);
        }
    }
}
