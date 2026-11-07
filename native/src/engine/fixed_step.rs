//! Reusable prefix of one complete authoritative fixed-step transaction.
//!
//! This module joins the already-verified once-per-step accounting, ambient
//! pellet, baseline-timer, and collision-safe baseline-respawn phases into one
//! corrected pre-control boundary. Successful due respawns are complete before
//! any controller samples the world. An impossible placement makes no result
//! available but deliberately does not choose the later scheduler's
//! owner-visible retry-versus-fault policy. Nothing here publishes authority.

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
use super::spawn::{
    SpawnCapacityDiagnostics, SpawnConfig, SpawnDomain, SpawnError, SpawnKey, SpawnRequest,
    SpawnWorkspace,
};
use super::state::{
    AllocatorState, BaselineRngState, BaselineStrategyState, BodyRange, ControllerLease,
    RngStateBundle, SnakeKind, SnakeState, StateError, WorldPoint, WorldState,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Joined fixed-step-prefix identity with collision-safe due baseline respawns.
pub const FIXED_STEP_PREFIX_VERSION: u32 = 2;
/// Current browser frame-v1 skin used by built-in baseline snakes.
const BASELINE_SNAKE_SKIN: u32 = 2;

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
    /// Collision-safe geometry and bounded work for baseline replacement.
    pub baseline_spawn: SpawnConfig,
    /// Initial speed of one newly replaced baseline snake.
    pub baseline_snake_base_speed: f64,
    /// Maximum admitted world snake records.
    pub maximum_snakes: usize,
    /// Maximum admitted packed body points after replacement compaction.
    pub maximum_body_points: usize,
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
            baseline_spawn: SpawnConfig::typescript_geometry_defaults(),
            baseline_snake_base_speed: 165.0,
            maximum_snakes: 512,
            maximum_body_points: 100_000,
            maximum_pellets: 200_000,
        }
    }

    pub(crate) fn validate_shape(self) -> Result<(), FixedStepPrefixError> {
        if self.algorithm_version != FIXED_STEP_PREFIX_VERSION {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        if !self.fixed_dt.is_finite() || self.fixed_dt <= 0.0 || self.fixed_dt > 1.0 {
            return Err(FixedStepPrefixError::InvalidConfig { field: "fixed_dt" });
        }
        self.baseline_spawn
            .validate()
            .map_err(|error| FixedStepPrefixError::Spawn(Box::new(error)))?;
        if !self.baseline_snake_base_speed.is_finite() || self.baseline_snake_base_speed <= 0.0 {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "baseline_snake_base_speed",
            });
        }
        if self.maximum_snakes == 0 || self.baseline.slot_count > self.maximum_snakes {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "maximum_snakes",
            });
        }
        if self.maximum_body_points == 0
            || self.baseline_spawn.snake_start_len > self.maximum_body_points
        {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "maximum_body_points",
            });
        }
        if self.maximum_pellets == 0 {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "maximum_pellets",
            });
        }
        self.accounting.validate()?;
        self.ambient.validate(self.maximum_pellets)?;
        self.baseline.validate()?;
        if self.ambient.world_radius.to_bits() != self.baseline_spawn.world_radius.to_bits() {
            return Err(FixedStepPrefixError::InvalidConfig {
                field: "baseline spawn world radius",
            });
        }
        Ok(())
    }
}

/// Immutable inputs from one admitted authority and operation epoch.
#[derive(Clone, Copy)]
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

/// Collision-safe baseline replacement work and retained storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaselineRespawnDiagnostics {
    /// Stable due slots named by timer staging.
    pub due_slots: usize,
    /// Slots completely replaced in the latest successful prefix.
    pub completed_slots: usize,
    /// Total random/fallback candidates examined across independent streams.
    pub candidates_examined: usize,
    /// Placements supplied by deterministic fallback.
    pub fallback_placements: usize,
    /// Total wall/body comparisons across all due slots.
    pub geometry_checks: usize,
    /// Old pellet-owner references cleared while preserving pellet color.
    pub cleared_pellet_owners: usize,
    /// Latest retained spawn-workspace size and capacities.
    pub spawn: SpawnCapacityDiagnostics,
    /// Retained stable due-slot capacity.
    pub due_slot_capacity: usize,
    /// Retained replaced-identity capacity.
    pub replaced_id_capacity: usize,
    /// Retained one-placement body-copy capacity.
    pub respawn_body_capacity: usize,
    /// Retained alternate packed-body buffer capacity.
    pub body_compaction_capacity: usize,
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
    /// Collision-safe due baseline replacement work.
    pub baseline_respawn: BaselineRespawnDiagnostics,
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
#[derive(Clone, Debug, Default)]
pub(crate) struct RngCopyScratch {
    pub(crate) world_gaussian_spare: String,
    pub(crate) evolution_gaussian_spare: String,
    pub(crate) external_gaussian_spare: String,
    pub(crate) baseline_gaussian_spares: Vec<String>,
}

/// Complete, still non-authoritative pre-control boundary.
#[derive(Clone, Copy, Debug)]
pub struct PreparedFixedStepPrefix<'workspace, 'source> {
    recurrent_resets: &'workspace [super::state::BrainHandle],
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
    /// Zero-source first inference markers created by successful evolved births.
    pub(crate) const fn evolved_recurrent_resets(self) -> &'workspace [super::state::BrainHandle] {
        self.recurrent_resets
    }

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
    evolved: super::evolved_replacement::EvolvedReplacementWorkspace,
    accounting: StepAccountingWorkspace,
    ambient: AmbientWorkspace,
    baseline: BaselineLifecycleWorkspace,
    spawn: SpawnWorkspace,
    world: WorldState,
    rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    allocators: Option<AllocatorState>,
    lifecycle: Option<BaselineLifecycleState>,
    due_respawn_slots: Vec<u32>,
    replaced_baseline_ids: Vec<u64>,
    respawn_body: Vec<WorldPoint>,
    compacted_body_points: Vec<WorldPoint>,
    baseline_respawn_diagnostics: BaselineRespawnDiagnostics,
    generation_elapsed_seconds: f64,
    ambient_accumulator: f64,
    ready: bool,
}

impl FixedStepPrefixWorkspace {
    /// Immutable pre-control source retained after publication; never resamples sensors.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn qualification_world(&self) -> &WorldState {
        &self.world
    }

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
        self.evolved.clear();
        self.baseline_respawn_diagnostics = BaselineRespawnDiagnostics::default();
        self.due_respawn_slots.clear();
        self.replaced_baseline_ids.clear();
        self.respawn_body.clear();
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
        reserve_for(
            &mut self.due_respawn_slots,
            baseline.due_slots().len(),
            "due baseline respawn slots",
        )?;
        self.due_respawn_slots
            .extend_from_slice(baseline.due_slots());

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
        baseline.apply_before_respawn_resolution(
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
        let next_ambient_accumulator = ambient.next_accumulator();
        self.resolve_due_baseline_respawns(inputs.config)?;
        self.generation_elapsed_seconds = next_elapsed;
        self.ambient_accumulator = next_ambient_accumulator;
        self.ready = true;

        self.prepared(inputs)
    }

    /// Join evolved births after the existing prefix phases, before shared control.
    pub(crate) fn prepare_with_evolved<'workspace, 'source>(
        &'workspace mut self,
        inputs: FixedStepPrefixInputs<'source>,
        lives: &mut super::evolved_lives::EvolvedGenerationState,
        policy: super::evolved_lives::EvolvedExperimentConfig,
        initial_accumulator: f64,
    ) -> Result<PreparedFixedStepPrefix<'workspace, 'source>, FixedStepPrefixError> {
        let _prefix = self.prepare(inputs)?;
        self.ready = false;
        self.evolved.prepare(
            inputs.key,
            &mut self.world,
            self.rng
                .as_mut()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
            self.allocators
                .as_mut()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
            lives,
            policy,
            inputs.config,
            inputs.generation_elapsed_seconds,
            initial_accumulator,
        )?;
        self.ready = true;
        self.prepared(inputs)
    }

    /// Seal identity/RNG work only after the whole physics result exists.
    pub(crate) fn seal_evolved(
        &mut self,
        key: PhysicsStepKey,
        rng: &RngStateBundle,
    ) -> Result<(), super::evolved_lives::EvolvedLifeError> {
        self.evolved.seal(key, rng)
    }

    pub(crate) fn evolved_proof(
        &self,
    ) -> Option<&super::evolved_replacement::EvolvedReplacementAuthorityProof> {
        self.evolved.proof()
    }

    /// Resolve every due baseline slot on the private prefix working copy.
    ///
    /// Slots are visited in the canonical timer order, but each placement uses
    /// its own durable RNG stream. A completed replacement immediately becomes
    /// an obstacle for later slots. Any failure leaves `ready == false`; the
    /// caller-owned source world, RNG, allocator, and lifecycle are untouched.
    fn resolve_due_baseline_respawns(
        &mut self,
        config: FixedStepPrefixConfig,
    ) -> Result<(), FixedStepPrefixError> {
        let due_count = self.due_respawn_slots.len();
        self.baseline_respawn_diagnostics.due_slots = due_count;
        self.baseline_respawn_diagnostics.due_slot_capacity = self.due_respawn_slots.capacity();
        if due_count == 0 {
            return Ok(());
        }
        if self
            .due_respawn_slots
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(FixedStepPrefixError::InternalShapeMismatch);
        }

        let mut final_body_points = self.world.snakes.iter().try_fold(0usize, |total, snake| {
            total
                .checked_add(snake.body.len)
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "referenced body-point count",
                })
        })?;
        for &slot in &self.due_respawn_slots {
            let snake_index = find_baseline_slot_index(&self.world, slot)?;
            let snake = &self.world.snakes[snake_index];
            let slot_index =
                usize::try_from(slot).map_err(|_| FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "slot index",
                })?;
            let runtime = self
                .lifecycle
                .as_ref()
                .and_then(|state| state.slots.get(slot_index))
                .ok_or(FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "lifecycle slot",
                })?;
            let rng = self
                .rng
                .as_ref()
                .and_then(|bundle| bundle.baselines.get(slot_index))
                .ok_or(FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "RNG slot",
                })?;
            if snake.alive
                || snake.kind != SnakeKind::Baseline
                || snake.baseline_slot != Some(slot)
                || runtime.slot != slot
                || runtime.snake_id != snake.id
                || runtime.respawn_remaining_seconds.map(f64::to_bits) != Some(0.0f64.to_bits())
                || rng.slot != slot
            {
                return Err(FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "due slot source",
                });
            }
            final_body_points = final_body_points
                .checked_sub(snake.body.len)
                .and_then(|value| value.checked_add(config.baseline_spawn.snake_start_len))
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "compacted baseline respawn bodies",
                })?;
        }
        if final_body_points > config.maximum_body_points {
            return Err(FixedStepPrefixError::BodyCapacityExceeded {
                required: final_body_points,
                maximum: config.maximum_body_points,
            });
        }

        let appended_body_points = due_count
            .checked_mul(config.baseline_spawn.snake_start_len)
            .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                context: "temporary baseline respawn bodies",
            })?;
        let temporary_body_points = self
            .world
            .body_points
            .len()
            .checked_add(appended_body_points)
            .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                context: "temporary packed body storage",
            })?;
        reserve_for(
            &mut self.world.body_points,
            temporary_body_points,
            "temporary respawn body points",
        )?;
        self.compacted_body_points.clear();
        reserve_for(
            &mut self.compacted_body_points,
            final_body_points,
            "compacted respawn body points",
        )?;
        reserve_for(
            &mut self.respawn_body,
            config.baseline_spawn.snake_start_len,
            "one baseline respawn body",
        )?;
        reserve_for(
            &mut self.replaced_baseline_ids,
            due_count,
            "replaced baseline identities",
        )?;

        let baseline_count =
            u64::try_from(due_count).map_err(|_| FixedStepPrefixError::ArithmeticOverflow {
                context: "baseline ID count",
            })?;
        let frame_count =
            u32::try_from(due_count).map_err(|_| FixedStepPrefixError::ArithmeticOverflow {
                context: "frame-v1 ID count",
            })?;
        let (first_baseline_id, first_frame_id) = {
            let allocators = self
                .allocators
                .as_mut()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            let baseline = allocators
                .reserve_baseline_ids(baseline_count)
                .map_err(|error| FixedStepPrefixError::Allocator(Box::new(error)))?
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            let frame = allocators
                .reserve_frame_v1_ids(
                    frame_count,
                    self.world.snakes.iter().map(|snake| snake.frame_v1_id),
                )
                .map_err(|error| FixedStepPrefixError::Allocator(Box::new(error)))?
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            (baseline.first, frame.first)
        };

        for ordinal in 0..due_count {
            let slot = self.due_respawn_slots[ordinal];
            let slot_index =
                usize::try_from(slot).map_err(|_| FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "slot index",
                })?;
            let request = [SpawnRequest {
                key: SpawnKey {
                    domain: SpawnDomain::Baseline,
                    slot: u64::from(slot),
                },
            }];
            let remaining_candidates = config
                .baseline_spawn
                .maximum_candidates_per_batch
                .checked_sub(self.baseline_respawn_diagnostics.candidates_examined)
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            if remaining_candidates == 0 {
                return Err(FixedStepPrefixError::BaselineRespawn {
                    slot,
                    error: Box::new(SpawnError::WorkBudgetExceeded {
                        key: request[0].key,
                        work: "baseline respawn candidates",
                        required: config
                            .baseline_spawn
                            .maximum_candidates_per_batch
                            .checked_add(1)
                            .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                                context: "baseline respawn candidate limit",
                            })?,
                        maximum: config.baseline_spawn.maximum_candidates_per_batch,
                    }),
                });
            }
            let remaining_geometry = config
                .baseline_spawn
                .maximum_geometry_checks_per_batch
                .checked_sub(self.baseline_respawn_diagnostics.geometry_checks)
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            if remaining_geometry == 0 {
                return Err(FixedStepPrefixError::BaselineRespawn {
                    slot,
                    error: Box::new(SpawnError::WorkBudgetExceeded {
                        key: request[0].key,
                        work: "baseline respawn geometry checks",
                        required: config
                            .baseline_spawn
                            .maximum_geometry_checks_per_batch
                            .checked_add(1)
                            .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                                context: "baseline respawn geometry limit",
                            })?,
                        maximum: config.baseline_spawn.maximum_geometry_checks_per_batch,
                    }),
                });
            }
            let mut slot_spawn_config = config.baseline_spawn;
            slot_spawn_config.maximum_candidates_per_batch = remaining_candidates;
            slot_spawn_config.maximum_geometry_checks_per_batch = remaining_geometry;
            let prepared = {
                let source_rng = self
                    .rng
                    .as_ref()
                    .and_then(|bundle| bundle.baselines.get(slot_index))
                    .ok_or(FixedStepPrefixError::BaselineRespawnShape {
                        slot,
                        field: "RNG slot",
                    })?;
                self.spawn
                    .prepare(
                        &self.world,
                        &request,
                        &source_rng.state,
                        slot_spawn_config,
                        config.baseline_spawn.snake_start_len,
                    )
                    .map_err(|error| FixedStepPrefixError::BaselineRespawn {
                        slot,
                        error: Box::new(error),
                    })?
            };
            let spawn_diagnostics = prepared.diagnostics();
            let candidates_examined = self
                .baseline_respawn_diagnostics
                .candidates_examined
                .checked_add(spawn_diagnostics.candidates_examined)
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "baseline respawn candidate count",
                })?;
            if candidates_examined > config.baseline_spawn.maximum_candidates_per_batch {
                return Err(FixedStepPrefixError::BaselineRespawn {
                    slot,
                    error: Box::new(SpawnError::WorkBudgetExceeded {
                        key: request[0].key,
                        work: "baseline respawn candidates",
                        required: candidates_examined,
                        maximum: config.baseline_spawn.maximum_candidates_per_batch,
                    }),
                });
            }
            let geometry_checks = self
                .baseline_respawn_diagnostics
                .geometry_checks
                .checked_add(spawn_diagnostics.geometry_checks)
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "baseline respawn geometry checks",
                })?;
            if geometry_checks > config.baseline_spawn.maximum_geometry_checks_per_batch {
                return Err(FixedStepPrefixError::BaselineRespawn {
                    slot,
                    error: Box::new(SpawnError::WorkBudgetExceeded {
                        key: request[0].key,
                        work: "baseline respawn geometry checks",
                        required: geometry_checks,
                        maximum: config.baseline_spawn.maximum_geometry_checks_per_batch,
                    }),
                });
            }
            let placement = *prepared
                .placements()
                .first()
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            if prepared.placements().len() != 1 || placement.key != request[0].key {
                return Err(FixedStepPrefixError::InternalShapeMismatch);
            }
            let body = prepared
                .body_for(&placement)
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            if body.len() != config.baseline_spawn.snake_start_len {
                return Err(FixedStepPrefixError::InternalShapeMismatch);
            }
            self.respawn_body.clear();
            self.respawn_body.extend_from_slice(body);
            let baseline_rng = self
                .rng
                .as_mut()
                .and_then(|bundle| bundle.baselines.get_mut(slot_index))
                .ok_or(FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "RNG slot",
                })?;
            let spare = self
                .rng_copy_scratch
                .baseline_gaussian_spares
                .get_mut(slot_index)
                .ok_or(FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "RNG copy scratch",
                })?;
            copy_serialized_rng_reusing(&mut baseline_rng.state, prepared.next_rng(), spare)?;
            self.baseline_respawn_diagnostics.candidates_examined = candidates_examined;
            self.baseline_respawn_diagnostics.geometry_checks = geometry_checks;
            self.baseline_respawn_diagnostics.fallback_placements = self
                .baseline_respawn_diagnostics
                .fallback_placements
                .checked_add(spawn_diagnostics.fallback_placements)
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "baseline fallback placement count",
                })?;
            self.baseline_respawn_diagnostics.spawn = spawn_diagnostics;

            let world_index = find_baseline_slot_index(&self.world, slot)?;
            let old_snake_id = self.world.snakes[world_index].id;
            let ordinal_u64 =
                u64::try_from(ordinal).map_err(|_| FixedStepPrefixError::ArithmeticOverflow {
                    context: "baseline ID offset",
                })?;
            let ordinal_u32 =
                u32::try_from(ordinal).map_err(|_| FixedStepPrefixError::ArithmeticOverflow {
                    context: "frame-v1 ID offset",
                })?;
            let snake_id = first_baseline_id.checked_add(ordinal_u64).ok_or(
                FixedStepPrefixError::ArithmeticOverflow {
                    context: "baseline ID assignment",
                },
            )?;
            let frame_v1_id = first_frame_id.checked_add(ordinal_u32).ok_or(
                FixedStepPrefixError::ArithmeticOverflow {
                    context: "frame-v1 ID assignment",
                },
            )?;
            let body_start = self.world.body_points.len();
            self.world.body_points.extend_from_slice(&self.respawn_body);
            let mut snake = SnakeState {
                id: snake_id,
                frame_v1_id,
                kind: SnakeKind::Baseline,
                alive: true,
                population_slot: None,
                brain: None,
                baseline_slot: Some(slot),
                baseline_strategy: Some(BaselineStrategyState::Roam),
                position: placement.head,
                previous_position: placement.head,
                direction: placement.direction,
                radius: config.baseline_spawn.snake_radius,
                speed: config.baseline_snake_base_speed,
                boost: false,
                age_seconds: 0.0,
                food: 0.0,
                points: 0.0,
                kills: 0,
                target_length: config.baseline_spawn.snake_start_len as f64,
                fitness: 0.0,
                turn: 0.0,
                previous_turn: 0.0,
                input_boost: false,
                previous_input_boost: false,
                control_accumulator_seconds: 0.0,
                delivered_observation_points: 0.0,
                observation_delivered: false,
                completed_step_heading_delta: 0.0,
                completed_step_speed_delta: 0.0,
                completed_step_applied_turn: 0.0,
                body: BodyRange {
                    start: body_start,
                    len: self.respawn_body.len(),
                },
                skin: BASELINE_SNAKE_SKIN,
            };
            let (age_seconds, points) = config
                .accounting
                .advance_live_snake(&snake, config.fixed_dt)?;
            snake.age_seconds = age_seconds;
            snake.points = points;
            self.world.snakes[world_index] = snake;
            self.lifecycle
                .as_mut()
                .and_then(|state| state.slots.get_mut(slot_index))
                .ok_or(FixedStepPrefixError::BaselineRespawnShape {
                    slot,
                    field: "lifecycle slot",
                })?
                .reset_after_respawn(snake_id);
            self.replaced_baseline_ids.push(old_snake_id);
        }

        self.replaced_baseline_ids.sort_unstable();
        if self
            .replaced_baseline_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(FixedStepPrefixError::InternalShapeMismatch);
        }
        for pellet in &mut self.world.pellets {
            if pellet
                .owner
                .is_some_and(|owner| self.replaced_baseline_ids.binary_search(&owner).is_ok())
            {
                pellet.owner = None;
                self.baseline_respawn_diagnostics.cleared_pellet_owners += 1;
            }
        }
        compact_world_bodies(
            &mut self.world,
            &mut self.compacted_body_points,
            config.maximum_body_points,
        )?;
        self.lifecycle
            .as_ref()
            .ok_or(FixedStepPrefixError::InternalShapeMismatch)?
            .validate_authoritative(&self.world, config.baseline.slot_count, false)?;
        for pellet in &self.world.pellets {
            if pellet
                .owner
                .is_some_and(|owner| !self.world.snakes.iter().any(|snake| snake.id == owner))
            {
                return Err(FixedStepPrefixError::InternalShapeMismatch);
            }
        }
        self.baseline_respawn_diagnostics.completed_slots = due_count;
        self.baseline_respawn_diagnostics.due_slot_capacity = self.due_respawn_slots.capacity();
        self.baseline_respawn_diagnostics.replaced_id_capacity =
            self.replaced_baseline_ids.capacity();
        self.baseline_respawn_diagnostics.respawn_body_capacity = self.respawn_body.capacity();
        self.baseline_respawn_diagnostics.body_compaction_capacity =
            self.compacted_body_points.capacity();
        Ok(())
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
            baseline_respawn: self.baseline_respawn_diagnostics,
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
            recurrent_resets: self.evolved.recurrent_resets(),
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

fn find_baseline_slot_index(world: &WorldState, slot: u32) -> Result<usize, FixedStepPrefixError> {
    let mut found = None;
    for (index, snake) in world.snakes.iter().enumerate() {
        if snake.kind != SnakeKind::Baseline || snake.baseline_slot != Some(slot) {
            continue;
        }
        if found.replace(index).is_some() {
            return Err(FixedStepPrefixError::BaselineRespawnShape {
                slot,
                field: "duplicate world slot",
            });
        }
    }
    found.ok_or(FixedStepPrefixError::BaselineRespawnShape {
        slot,
        field: "missing world slot",
    })
}

pub(crate) fn compact_world_bodies(
    world: &mut WorldState,
    scratch: &mut Vec<WorldPoint>,
    maximum_body_points: usize,
) -> Result<(), FixedStepPrefixError> {
    let required = world.snakes.iter().try_fold(0usize, |total, snake| {
        let end = snake.body.start.checked_add(snake.body.len).ok_or(
            FixedStepPrefixError::ArithmeticOverflow {
                context: "body-range end during compaction",
            },
        )?;
        if end > world.body_points.len()
            || (snake.alive && snake.body.len == 0)
            || (snake.body.len != 0 && world.body_points[snake.body.start] != snake.position)
        {
            return Err(FixedStepPrefixError::InternalShapeMismatch);
        }
        total
            .checked_add(snake.body.len)
            .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                context: "body-point compaction count",
            })
    })?;
    if required > maximum_body_points {
        return Err(FixedStepPrefixError::BodyCapacityExceeded {
            required,
            maximum: maximum_body_points,
        });
    }
    scratch.clear();
    reserve_for(scratch, required, "compacted body points")?;
    for index in 0..world.snakes.len() {
        let range = world.snakes[index].body;
        let end = range
            .start
            .checked_add(range.len)
            .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
        let start = scratch.len();
        scratch.extend_from_slice(
            world
                .body_points
                .get(range.start..end)
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
        );
        world.snakes[index].body = BodyRange {
            start,
            len: range.len,
        };
    }
    if scratch.len() != required {
        return Err(FixedStepPrefixError::InternalShapeMismatch);
    }
    std::mem::swap(&mut world.body_points, scratch);
    Ok(())
}

pub(crate) fn copy_world_reusing(
    target: &mut WorldState,
    source: &WorldState,
    required_pellets: usize,
) -> Result<(), FixedStepPrefixError> {
    target.ray_cursor = source.ray_cursor;
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

pub(crate) fn copy_controller_leases_reusing(
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
        let mut identity_key = String::new();
        reserve_string(
            &mut identity_key,
            lease.identity_key.len(),
            "controller legacy identity",
        )?;
        identity_key.push_str(&lease.identity_key);
        let mut resume_token = String::new();
        reserve_string(
            &mut resume_token,
            lease.resume_token.len(),
            "controller resume token",
        )?;
        resume_token.push_str(&lease.resume_token);
        target.push(ControllerLease {
            reward: lease.reward,
            identity_key,
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
        &mut target.identity_key,
        source.identity_key.len(),
        "controller legacy identity",
    )?;
    reserve_string(
        &mut target.resume_token,
        source.resume_token.len(),
        "controller resume token",
    )?;
    target.id = source.id;
    target.reward = source.reward;
    target.snake_id = source.snake_id;
    target.kind = source.kind;
    target.connection_id = source.connection_id;
    target.scope.clear();
    target.scope.push_str(&source.scope);
    target.identity_key.clear();
    target.identity_key.push_str(&source.identity_key);
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

pub(crate) fn copy_rng_bundle_reusing(
    target: &mut Option<RngStateBundle>,
    scratch: &mut RngCopyScratch,
    source: &RngStateBundle,
) -> Result<(), FixedStepPrefixError> {
    if target.is_none() {
        *target = Some(RngStateBundle {
            baseline_behaviors: Vec::new(),
            evolved_respawns: Vec::new(),
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
    reserve_for(
        &mut target.baseline_behaviors,
        source.baseline_behaviors.len(),
        "baseline behavior RNG states",
    )?;
    target.baseline_behaviors.clear();
    target
        .baseline_behaviors
        .extend_from_slice(&source.baseline_behaviors);
    reserve_for(
        &mut target.evolved_respawns,
        source.evolved_respawns.len(),
        "evolved RNG states",
    )?;
    target.evolved_respawns.clear();
    target
        .evolved_respawns
        .extend_from_slice(&source.evolved_respawns);
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

pub(crate) fn copy_serialized_rng_reusing(
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

pub(crate) fn controller_text_capacity(leases: &[ControllerLease]) -> usize {
    leases.iter().fold(0usize, |total, lease| {
        total
            .saturating_add(lease.scope.capacity())
            .saturating_add(lease.identity_key.capacity())
            .saturating_add(lease.resume_token.capacity())
    })
}

pub(crate) fn rng_text_capacity(rng: Option<&RngStateBundle>, scratch: &RngCopyScratch) -> usize {
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

pub(crate) fn copy_lifecycle_reusing(
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
    /// Evolved accounting or lifecycle staging failed.
    Evolved(super::evolved_lives::EvolvedLifeError),
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
    /// Final packed body storage cannot admit every replacement.
    BodyCapacityExceeded { required: usize, maximum: usize },
    /// A due stable slot disagreed with world/lifecycle/RNG staging.
    BaselineRespawnShape { slot: u32, field: &'static str },
    /// Collision-safe placement could not complete for one due slot.
    ///
    /// The later authority coordinator must map this non-authoritative failure
    /// to the reviewed retry-versus-fault behavior before production cutover.
    BaselineRespawn { slot: u32, error: Box<SpawnError> },
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
    /// Spawn configuration rejected the boundary before due-slot work.
    Spawn(Box<SpawnError>),
    /// Deterministic identity reservation failed on the private working copy.
    Allocator(Box<StateError>),
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
            Self::Evolved(error) => write!(formatter, "{error}"),
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
            Self::BodyCapacityExceeded { required, maximum } => write!(
                formatter,
                "baseline respawn requires {required} body points but the admitted maximum is {maximum}"
            ),
            Self::BaselineRespawnShape { slot, field } => {
                write!(formatter, "baseline respawn slot {slot} has invalid {field}")
            }
            Self::BaselineRespawn { slot, error } => {
                write!(formatter, "baseline respawn slot {slot} is unresolved: {error}")
            }
            Self::ResultNotReady => write!(formatter, "fixed-step prefix result is not ready"),
            Self::InternalShapeMismatch => {
                write!(formatter, "fixed-step prefix internal shape mismatch")
            }
            Self::Accounting(error) => Display::fmt(error, formatter),
            Self::Ambient(error) => Display::fmt(error, formatter),
            Self::Baseline(error) => Display::fmt(error, formatter),
            Self::Spawn(error) => Display::fmt(error, formatter),
            Self::Allocator(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for FixedStepPrefixError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Evolved(error) => Some(error),
            Self::Accounting(error) => Some(error.as_ref()),
            Self::Ambient(error) => Some(error.as_ref()),
            Self::Baseline(error) => Some(error.as_ref()),
            Self::BaselineRespawn { error, .. } | Self::Spawn(error) => Some(error.as_ref()),
            Self::Allocator(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
