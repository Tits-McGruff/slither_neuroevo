//! Complete multi-substep physics working transaction.
//!
//! Movement, food, collision, and effect phases deliberately produce scratch
//! views. This module copies one fully prepared substep into an independent
//! working world, applies all deaths and kill awards there, and only advances
//! that working transaction after every identity and capacity check succeeds.
//! Multiple collision substeps therefore cannot partly mutate authority. A
//! later fixed-step coordinator will combine the finished physics result with
//! staged control/recurrent and before/after-step state before one authority
//! publication; this module does not expose a partial-authority fallback.

use super::collision::{CollisionConfig, CollisionError, CollisionWorkspace};
use super::effects::{
    BaselineDeathEvent, DeathDropConfig, EffectError, EffectWorkspace, PreparedEffects,
};
use super::fixed_step::{
    controller_text_capacity, copy_controller_leases_reusing, copy_rng_bundle_reusing,
    rng_text_capacity, FixedStepPrefixError, RngCopyScratch,
};
use super::food::{FoodConfig, FoodError, FoodWorkspace};
use super::movement::{MovementConfig, MovementError, MovementWorkspace};
use super::spatial::{
    IndexedPelletWorld, PelletIndexDiagnostics, PelletSpatialIndex, SpatialIndexError,
};
use super::state::{
    AllocatorState, PelletState, RngStateBundle, SnakeState, WorldPoint, WorldState,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Full identity of one in-process fixed-step proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicsStepKey {
    world_epoch: u64,
    generation: u64,
    source_completed_step: u64,
    population_epoch: u64,
    config_revision: u64,
    config_hash: [u8; 32],
    operation_epoch: u64,
}

impl PhysicsStepKey {
    /// Construct one key from admitted authority and the coordinator's current epoch.
    #[must_use]
    pub const fn new(
        world_epoch: u64,
        generation: u64,
        source_completed_step: u64,
        population_epoch: u64,
        config_revision: u64,
        config_hash: [u8; 32],
        operation_epoch: u64,
    ) -> Self {
        Self {
            world_epoch,
            generation,
            source_completed_step,
            population_epoch,
            config_revision,
            config_hash,
            operation_epoch,
        }
    }

    /// Process-local authority identity changed by Reset, New Run, or import.
    #[must_use]
    pub const fn world_epoch(self) -> u64 {
        self.world_epoch
    }

    /// Current generation identity.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Fully committed step before this proposal.
    #[must_use]
    pub const fn source_completed_step(self) -> u64 {
        self.source_completed_step
    }

    /// Stable population/brain epoch.
    #[must_use]
    pub const fn population_epoch(self) -> u64 {
        self.population_epoch
    }

    /// Normalized configuration revision.
    #[must_use]
    pub const fn config_revision(self) -> u64 {
        self.config_revision
    }

    /// Exact normalized configuration SHA-256 bytes.
    #[must_use]
    pub const fn config_hash(self) -> [u8; 32] {
        self.config_hash
    }

    /// In-process authority replacement/command epoch.
    #[must_use]
    pub const fn operation_epoch(self) -> u64 {
        self.operation_epoch
    }

    /// Name the first identity component that differs from another boundary.
    #[must_use]
    pub fn first_mismatch(self, current: Self) -> Option<PhysicsStepKeyField> {
        if self.world_epoch != current.world_epoch {
            Some(PhysicsStepKeyField::WorldEpoch)
        } else if self.generation != current.generation {
            Some(PhysicsStepKeyField::Generation)
        } else if self.source_completed_step != current.source_completed_step {
            Some(PhysicsStepKeyField::SourceCompletedStep)
        } else if self.population_epoch != current.population_epoch {
            Some(PhysicsStepKeyField::PopulationEpoch)
        } else if self.config_revision != current.config_revision {
            Some(PhysicsStepKeyField::ConfigRevision)
        } else if self.config_hash != current.config_hash {
            Some(PhysicsStepKeyField::ConfigHash)
        } else if self.operation_epoch != current.operation_epoch {
            Some(PhysicsStepKeyField::OperationEpoch)
        } else {
            None
        }
    }

    fn validate(self) -> Result<(), PhysicsError> {
        if self.world_epoch == 0
            || self.generation == 0
            || self.population_epoch == 0
            || self.config_revision == 0
            || self.operation_epoch == 0
        {
            return Err(PhysicsError::InvalidStepKey);
        }
        Ok(())
    }
}

/// Compact diagnostic naming the first stale fixed-step identity component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsStepKeyField {
    /// Reset, New Run, or import replaced the complete world.
    WorldEpoch,
    /// Generation identity changed.
    Generation,
    /// The source committed-step boundary changed.
    SourceCompletedStep,
    /// Population/brain ownership changed.
    PopulationEpoch,
    /// Normalized settings revision changed.
    ConfigRevision,
    /// Normalized settings digest changed.
    ConfigHash,
    /// A newer in-process operation superseded this proposal.
    OperationEpoch,
}

/// Identity of the next expected collision substep in one working transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicsSubstepKey {
    step: PhysicsStepKey,
    ordinal: usize,
}

impl PhysicsSubstepKey {
    /// Parent fixed-step identity.
    #[must_use]
    pub const fn step(self) -> PhysicsStepKey {
        self.step
    }

    /// Zero-based collision substep ordinal.
    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.ordinal
    }
}

/// Complete phase settings projected from one admitted configuration revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsConfig {
    /// Movement, boost, body, and wall settings.
    pub movement: MovementConfig,
    /// Food claim and growth settings.
    pub food: FoodConfig,
    /// Swept collision settings and checked work limits.
    pub collision: CollisionConfig,
    /// Corpse-pellet formula and checked count settings.
    pub death: DeathDropConfig,
    /// Complete pellet-index cell width used by food queries.
    pub pellet_index_cell_size: f64,
    /// Complete pellet-index entry ceiling used by food queries.
    pub maximum_pellet_index_entries: usize,
    /// Fixed collision-substep delta in simulated seconds.
    pub substep_dt: f64,
    /// Admitted packed body-point ceiling.
    pub maximum_body_points: usize,
    /// Admitted authoritative pellet ceiling.
    pub maximum_pellets: usize,
    /// Points awarded once to the selected body owner.
    pub points_per_kill: f64,
}

impl PhysicsConfig {
    /// Current TypeScript default retained as a comparison fixture.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            movement: MovementConfig::typescript_defaults(),
            food: FoodConfig::typescript_defaults(),
            collision: CollisionConfig::typescript_defaults(),
            death: DeathDropConfig::typescript_defaults(),
            pellet_index_cell_size: 120.0,
            maximum_pellet_index_entries: 1_000_000,
            substep_dt: 1.0 / 180.0,
            maximum_body_points: 100_000,
            maximum_pellets: 100_000,
            points_per_kill: 400.0,
        }
    }

    pub(crate) fn validate(self) -> Result<(), PhysicsError> {
        self.movement.validate()?;
        self.food.validate()?;
        self.collision.validate()?;
        self.death.validate()?;
        if !self.substep_dt.is_finite() || self.substep_dt <= 0.0 {
            return Err(PhysicsError::InvalidConfig {
                field: "substep_dt",
            });
        }
        if !self.pellet_index_cell_size.is_finite() || self.pellet_index_cell_size <= 0.0 {
            return Err(PhysicsError::InvalidConfig {
                field: "pellet_index_cell_size",
            });
        }
        if self.maximum_body_points == 0
            || self.maximum_pellets == 0
            || self.maximum_pellet_index_entries == 0
        {
            return Err(PhysicsError::InvalidConfig {
                field: "state capacity",
            });
        }
        if !self.points_per_kill.is_finite() || self.points_per_kill < 0.0 {
            return Err(PhysicsError::InvalidConfig {
                field: "points_per_kill",
            });
        }
        Ok(())
    }
}

/// Current sizes and retained capacities for one staged substep outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicsSubstepDiagnostics {
    /// Staged snake count.
    pub snakes: usize,
    /// Staged packed body-point count.
    pub body_points: usize,
    /// Staged pellet count.
    pub pellets: usize,
    /// Applied deaths.
    pub deaths: usize,
    /// Applied kill awards.
    pub awards: usize,
    /// Baseline deaths carried to the complete step result.
    pub baseline_deaths: usize,
    /// Retained snake capacity.
    pub snake_capacity: usize,
    /// Retained body-point capacity.
    pub body_point_capacity: usize,
    /// Retained pellet capacity.
    pub pellet_capacity: usize,
    /// Retained death-marker capacity.
    pub death_marker_capacity: usize,
    /// Retained baseline-event capacity.
    pub baseline_event_capacity: usize,
    /// Retained validation-order capacity.
    pub validation_order_capacity: usize,
    /// Retained validation-range capacity.
    pub validation_range_capacity: usize,
    /// Retained pellet-order capacity.
    pub pellet_order_capacity: usize,
}

/// Current sizes and retained capacities across one working fixed step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicsStepDiagnostics {
    /// Required collision substeps.
    pub expected_substeps: usize,
    /// Completely accepted collision substeps.
    pub completed_substeps: usize,
    /// Current snake count.
    pub snakes: usize,
    /// Current body-point count.
    pub body_points: usize,
    /// Current pellet count.
    pub pellets: usize,
    /// Baseline-death events accumulated across substeps.
    pub baseline_deaths: usize,
    /// Stable controller-target count retained for death protection.
    pub controlled_snakes: usize,
    /// Retained working snake capacity.
    pub snake_capacity: usize,
    /// Retained working body-point capacity.
    pub body_point_capacity: usize,
    /// Retained working pellet capacity.
    pub pellet_capacity: usize,
    /// Retained controller-lease capacity carried unchanged through physics.
    pub controller_lease_capacity: usize,
    /// Retained controller scope/token text capacity carried through physics.
    pub controller_text_capacity: usize,
    /// Retained per-baseline serialized RNG capacity.
    pub baseline_rng_capacity: usize,
    /// Retained absent-Gaussian-spare vector capacity.
    pub baseline_rng_spare_capacity: usize,
    /// Retained serialized RNG text capacity, including absent spare storage.
    pub rng_text_capacity: usize,
    /// Retained controlled-ID capacity.
    pub controlled_snake_capacity: usize,
    /// Retained baseline-event capacity.
    pub baseline_event_capacity: usize,
    /// Retained validation-order capacity.
    pub validation_order_capacity: usize,
    /// Retained validation-range capacity.
    pub validation_range_capacity: usize,
    /// Retained pellet-order capacity.
    pub pellet_order_capacity: usize,
}

/// Test-hook-only allocation counts inside the complete physics transaction.
#[cfg(feature = "engine-test-hooks")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PhysicsPhaseAllocations {
    pub begin: u64,
    pub pellet_index: u64,
    pub movement: u64,
    pub food: u64,
    pub collision: u64,
    pub effects: u64,
    pub result_application: u64,
    pub accept: u64,
    pub finalize: u64,
}

/// Read-only complete physics result for later full-step publication.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPhysicsStep<'step> {
    key: PhysicsStepKey,
    world: &'step WorldState,
    rng: &'step RngStateBundle,
    allocators: &'step AllocatorState,
    baseline_deaths: &'step [BaselineDeathEvent],
    diagnostics: PhysicsStepDiagnostics,
}

/// Mutable complete-step buffers admitted for the one authority swap.
pub(crate) struct PhysicsPublicationBuffers<'step> {
    /// Fully advanced physical world.
    pub world: &'step mut WorldState,
    /// Fully advanced gameplay RNG bundle.
    pub rng: &'step mut RngStateBundle,
    /// Fully advanced deterministic allocator bundle.
    pub allocators: &'step mut AllocatorState,
}

/// Unforgeable view of baseline deaths emitted by one complete physics step.
///
/// The baseline lifecycle accepts this keyed view instead of a raw event slice,
/// so a same-ID event retained across Reset, New Run, or import cannot be
/// relabelled as belonging to a newer authority epoch.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPhysicsBaselineDeaths<'step> {
    key: PhysicsStepKey,
    world: &'step WorldState,
    events: &'step [BaselineDeathEvent],
}

impl<'step> PreparedPhysicsBaselineDeaths<'step> {
    /// Exact fixed-step identity that produced the events.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Complete post-physics world against which every event was validated.
    #[must_use]
    pub const fn world(self) -> &'step WorldState {
        self.world
    }

    /// Canonical stable-ID death events from the complete physics result.
    #[must_use]
    pub const fn events(self) -> &'step [BaselineDeathEvent] {
        self.events
    }

    #[cfg(test)]
    pub(crate) const fn test_fixture(
        key: PhysicsStepKey,
        world: &'step WorldState,
        events: &'step [BaselineDeathEvent],
    ) -> Self {
        Self { key, world, events }
    }
}

impl<'step> PreparedPhysicsStep<'step> {
    /// Exact generation/config/operation identity prepared.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Complete physical world after every declared collision substep.
    ///
    /// Controller leases carry the already-committed control boundary through
    /// physics unchanged. A controlled death remains only an intermediate
    /// result: the complete world-step transaction must stage its replacement
    /// assignment before this world can become publishable authority.
    #[must_use]
    pub const fn world(self) -> &'step WorldState {
        self.world
    }

    /// Complete gameplay RNG continuation after every declared substep.
    #[must_use]
    pub const fn rng(self) -> &'step RngStateBundle {
        self.rng
    }

    /// Complete monotonic allocator continuation after every declared substep.
    #[must_use]
    pub const fn allocators(self) -> &'step AllocatorState {
        self.allocators
    }

    /// Baseline deaths requiring deterministic respawn handling before publication.
    #[must_use]
    pub const fn baseline_deaths(self) -> &'step [BaselineDeathEvent] {
        self.baseline_deaths
    }

    /// Keyed baseline-death proof for the lifecycle transaction.
    #[must_use]
    pub const fn prepared_baseline_deaths(self) -> PreparedPhysicsBaselineDeaths<'step> {
        PreparedPhysicsBaselineDeaths {
            key: self.key,
            world: self.world,
            events: self.baseline_deaths,
        }
    }

    /// Current work and retained-capacity diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> PhysicsStepDiagnostics {
        self.diagnostics
    }
}

/// Reusable phase workspaces owned by the single physics coordinator.
///
/// Callers cannot submit independently prepared phase output. Only
/// [`PhysicsStepWorkspace::advance_substep`] can join these buffers to a
/// working step, and that method supplies the step's exact world, RNG,
/// allocator continuation, delta, capacities, and phase configuration.
#[derive(Debug)]
pub struct PhysicsPipelineWorkspace {
    pellet_index: PelletSpatialIndex,
    movement: MovementWorkspace,
    food: FoodWorkspace,
    collision: CollisionWorkspace,
    effects: EffectWorkspace,
    substep: PhysicsSubstepWorkspace,
    #[cfg(feature = "engine-test-hooks")]
    allocation_snapshot: Option<fn() -> u64>,
    #[cfg(feature = "engine-test-hooks")]
    allocation_cursor: Option<u64>,
    #[cfg(feature = "engine-test-hooks")]
    phase_allocations: PhysicsPhaseAllocations,
}

impl Default for PhysicsPipelineWorkspace {
    fn default() -> Self {
        Self {
            pellet_index: PelletSpatialIndex::empty(),
            movement: MovementWorkspace::default(),
            food: FoodWorkspace::default(),
            collision: CollisionWorkspace::default(),
            effects: EffectWorkspace::default(),
            substep: PhysicsSubstepWorkspace::default(),
            #[cfg(feature = "engine-test-hooks")]
            allocation_snapshot: None,
            #[cfg(feature = "engine-test-hooks")]
            allocation_cursor: None,
            #[cfg(feature = "engine-test-hooks")]
            phase_allocations: PhysicsPhaseAllocations::default(),
        }
    }
}

impl PhysicsPipelineWorkspace {
    /// Construct empty reusable physics-phase scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Diagnostics for the last complete or rejected result-application phase.
    #[must_use]
    pub fn substep_diagnostics(&self) -> PhysicsSubstepDiagnostics {
        self.substep.diagnostics()
    }

    /// Retained complete pellet-index storage from the last attempted substep.
    #[must_use]
    pub fn pellet_index_diagnostics(&self) -> PelletIndexDiagnostics {
        self.pellet_index.diagnostics()
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn reset_allocation_tracking(&mut self, snapshot: Option<fn() -> u64>) {
        self.allocation_snapshot = snapshot;
        self.allocation_cursor = snapshot.map(|snapshot| snapshot());
        self.phase_allocations = PhysicsPhaseAllocations::default();
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn record_begin_allocations(&mut self) {
        self.phase_allocations.begin =
            allocation_delta(self.allocation_snapshot, &mut self.allocation_cursor);
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn record_finalize_allocations(&mut self) {
        self.phase_allocations.finalize =
            self.phase_allocations
                .finalize
                .saturating_add(allocation_delta(
                    self.allocation_snapshot,
                    &mut self.allocation_cursor,
                ));
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) const fn phase_allocations(&self) -> PhysicsPhaseAllocations {
        self.phase_allocations
    }
}

/// Reusable storage for one checked substep outcome before it joins a step.
#[derive(Clone, Debug, Default)]
struct PhysicsSubstepWorkspace {
    next_world: WorldState,
    next_rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    next_allocators: Option<AllocatorState>,
    death_markers: Vec<bool>,
    baseline_deaths: Vec<BaselineDeathEvent>,
    ready: Option<PhysicsSubstepKey>,
    staged_config: Option<PhysicsConfig>,
    validation: PhysicalValidationScratch,
    deaths: usize,
    awards: usize,
}

impl PhysicsSubstepWorkspace {
    /// Apply one complete effect/collision snapshot into independent scratch.
    fn prepare<'collision, 'food, 'world>(
        &mut self,
        effects: PreparedEffects<'_, 'collision, 'food, 'world>,
        expected_source: &WorldState,
        key: PhysicsSubstepKey,
        config: PhysicsConfig,
    ) -> Result<(), PhysicsError> {
        self.clear();
        key.step.validate()?;
        config.validate()?;
        let collision = effects.collision();
        let food = collision.food();
        if !std::ptr::eq(food.source_world(), expected_source)
            || food.snakes().len() != expected_source.snakes.len()
        {
            return Err(PhysicsError::SourceWorldMismatch);
        }

        reserve_for(
            &mut self.next_world.snakes,
            food.snakes().len(),
            "physics snakes",
        )?;
        reserve_for(
            &mut self.next_world.body_points,
            food.body_points().len(),
            "physics body points",
        )?;
        reserve_for(
            &mut self.next_world.pellets,
            effects.pellets().len(),
            "physics pellets",
        )?;
        reserve_for(
            &mut self.death_markers,
            food.snakes().len(),
            "physics death markers",
        )?;
        reserve_for(
            &mut self.baseline_deaths,
            effects.baseline_deaths().len(),
            "physics baseline events",
        )?;
        self.next_world.snakes.extend_from_slice(food.snakes());
        self.next_world
            .body_points
            .extend_from_slice(food.body_points());
        self.next_world.pellets.extend_from_slice(effects.pellets());
        self.next_world.controller_leases.clear();
        self.death_markers.resize(food.snakes().len(), false);
        self.baseline_deaths
            .extend_from_slice(effects.baseline_deaths());
        prepare_snake_order(&self.next_world.snakes, &mut self.validation.snake_order)?;

        let mut prior_death_id = None;
        for death in collision.deaths() {
            if prior_death_id.is_some_and(|prior| death.victim_id <= prior) {
                return Err(PhysicsError::NonCanonicalDeathOrder);
            }
            prior_death_id = Some(death.victim_id);
            let snake = self
                .next_world
                .snakes
                .get_mut(death.victim_index)
                .ok_or(PhysicsError::ShapeMismatch)?;
            if snake.id != death.victim_id || self.death_markers[death.victim_index] {
                return Err(PhysicsError::ShapeMismatch);
            }
            self.death_markers[death.victim_index] = true;
            snake.alive = false;
            self.deaths += 1;
        }

        let mut prior_victim_id = None;
        for award in collision.awards() {
            if prior_victim_id.is_some_and(|prior| award.victim_id <= prior) {
                return Err(PhysicsError::NonCanonicalAwardOrder);
            }
            prior_victim_id = Some(award.victim_id);
            let victim_index = find_snake_index(
                &self.next_world.snakes,
                &self.validation.snake_order,
                award.victim_id,
            )?;
            if !self.death_markers[victim_index] {
                return Err(PhysicsError::AwardWithoutDeath {
                    victim_id: award.victim_id,
                });
            }
            let killer_index = find_snake_index(
                &self.next_world.snakes,
                &self.validation.snake_order,
                award.killer_id,
            )?;
            let killer = &mut self.next_world.snakes[killer_index];
            let next_kills =
                killer
                    .kills
                    .checked_add(1)
                    .ok_or(PhysicsError::KillCountOverflow {
                        killer_id: killer.id,
                    })?;
            let next_points = killer.points + config.points_per_kill;
            if !next_points.is_finite() {
                return Err(PhysicsError::NonFiniteKillPoints {
                    killer_id: killer.id,
                });
            }
            killer.kills = next_kills;
            killer.points = next_points;
            self.awards += 1;
        }

        validate_physical_world(&self.next_world, &mut self.validation)?;
        validate_baseline_events(&self.baseline_deaths, &self.next_world.snakes)?;
        copy_rng_bundle_reusing(
            &mut self.next_rng,
            &mut self.rng_copy_scratch,
            effects.rng(),
        )
        .map_err(map_reuse_error)?;
        self.next_allocators = Some(effects.allocators().clone());
        self.ready = Some(key);
        self.staged_config = Some(config);
        Ok(())
    }

    /// Current sizes and retained capacities, including after rejection.
    #[must_use]
    fn diagnostics(&self) -> PhysicsSubstepDiagnostics {
        PhysicsSubstepDiagnostics {
            snakes: self.next_world.snakes.len(),
            body_points: self.next_world.body_points.len(),
            pellets: self.next_world.pellets.len(),
            deaths: self.deaths,
            awards: self.awards,
            baseline_deaths: self.baseline_deaths.len(),
            snake_capacity: self.next_world.snakes.capacity(),
            body_point_capacity: self.next_world.body_points.capacity(),
            pellet_capacity: self.next_world.pellets.capacity(),
            death_marker_capacity: self.death_markers.capacity(),
            baseline_event_capacity: self.baseline_deaths.capacity(),
            validation_order_capacity: self.validation.snake_order.capacity(),
            validation_range_capacity: self.validation.body_ranges.capacity(),
            pellet_order_capacity: self.validation.pellet_order.capacity(),
        }
    }

    fn clear(&mut self) {
        self.ready = None;
        self.next_world.snakes.clear();
        self.next_world.body_points.clear();
        self.next_world.pellets.clear();
        self.next_world.controller_leases.clear();
        self.death_markers.clear();
        self.baseline_deaths.clear();
        self.staged_config = None;
        self.deaths = 0;
        self.awards = 0;
    }
}

/// Reusable owner of one complete, still non-authoritative physics step.
#[derive(Clone, Debug, Default)]
pub struct PhysicsStepWorkspace {
    key: Option<PhysicsStepKey>,
    world: WorldState,
    rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    allocators: Option<AllocatorState>,
    controlled_snake_ids: Vec<u64>,
    baseline_deaths: Vec<BaselineDeathEvent>,
    config: Option<PhysicsConfig>,
    validation: PhysicalValidationScratch,
    expected_substeps: usize,
    completed_substeps: usize,
}

impl PhysicsStepWorkspace {
    /// Construct empty reusable fixed-step scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start one physics transaction from an immutable authoritative boundary.
    pub fn begin(
        &mut self,
        key: PhysicsStepKey,
        source_world: &WorldState,
        source_rng: &RngStateBundle,
        source_allocators: &AllocatorState,
        config: PhysicsConfig,
        expected_substeps: usize,
    ) -> Result<(), PhysicsError> {
        self.discard();
        key.validate()?;
        config.validate()?;
        if expected_substeps == 0 {
            return Err(PhysicsError::InvalidSubstepCount);
        }
        reserve_for(
            &mut self.world.snakes,
            source_world.snakes.len(),
            "working snakes",
        )?;
        reserve_for(
            &mut self.world.body_points,
            source_world.body_points.len(),
            "working body points",
        )?;
        reserve_for(
            &mut self.world.pellets,
            source_world.pellets.len(),
            "working pellets",
        )?;
        reserve_for(
            &mut self.world.controller_leases,
            source_world.controller_leases.len(),
            "working controller leases",
        )?;
        reserve_for(
            &mut self.controlled_snake_ids,
            source_world.controller_leases.len(),
            "controlled snake IDs",
        )?;
        self.world.snakes.extend_from_slice(&source_world.snakes);
        self.world
            .body_points
            .extend_from_slice(&source_world.body_points);
        self.world.pellets.extend_from_slice(&source_world.pellets);
        copy_controller_leases_reusing(
            &mut self.world.controller_leases,
            &source_world.controller_leases,
        )
        .map_err(map_reuse_error)?;
        self.controlled_snake_ids.extend(
            source_world
                .controller_leases
                .iter()
                .map(|lease| lease.snake_id),
        );
        self.controlled_snake_ids.sort_unstable();
        if self
            .controlled_snake_ids
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(PhysicsError::DuplicateControlledSnake);
        }
        validate_physical_world(&self.world, &mut self.validation)?;
        copy_rng_bundle_reusing(&mut self.rng, &mut self.rng_copy_scratch, source_rng)
            .map_err(map_reuse_error)?;
        self.allocators = Some(source_allocators.clone());
        self.config = Some(config);
        self.key = Some(key);
        self.expected_substeps = expected_substeps;
        Ok(())
    }

    /// Current immutable physical boundary for movement/index construction.
    pub fn world(&self) -> Result<&WorldState, PhysicsError> {
        self.key
            .map(|_| &self.world)
            .ok_or(PhysicsError::StepNotStarted)
    }

    /// Current gameplay RNG continuation for effect realization.
    pub fn rng(&self) -> Result<&RngStateBundle, PhysicsError> {
        self.rng.as_ref().ok_or(PhysicsError::StepNotStarted)
    }

    /// Current allocator continuation for effect realization.
    pub fn allocators(&self) -> Result<&AllocatorState, PhysicsError> {
        self.allocators.as_ref().ok_or(PhysicsError::StepNotStarted)
    }

    /// Kill-credit configuration bound to this step's config identity.
    pub fn config(&self) -> Result<PhysicsConfig, PhysicsError> {
        self.config.ok_or(PhysicsError::StepNotStarted)
    }

    /// Stable sorted controller targets whose death requires replacement staging.
    pub fn controlled_snake_ids(&self) -> Result<&[u64], PhysicsError> {
        self.key
            .map(|_| self.controlled_snake_ids.as_slice())
            .ok_or(PhysicsError::StepNotStarted)
    }

    /// Exact identity expected for the next substep.
    pub fn next_substep_key(&self) -> Result<PhysicsSubstepKey, PhysicsError> {
        let step = self.key.ok_or(PhysicsError::StepNotStarted)?;
        if self.completed_substeps >= self.expected_substeps {
            return Err(PhysicsError::TooManySubsteps {
                expected: self.expected_substeps,
            });
        }
        Ok(PhysicsSubstepKey {
            step,
            ordinal: self.completed_substeps,
        })
    }

    /// Prepare and accept exactly the next substep from this transaction's state.
    ///
    /// This is the only phase-chain entry point. It deliberately does not
    /// accept a caller-created `PreparedEffects`: every upstream phase receives
    /// the stored working world, current RNG/allocator continuation, exact
    /// substep delta, admitted capacities, and the full bound configuration.
    pub fn advance_substep(
        &mut self,
        pipeline: &mut PhysicsPipelineWorkspace,
        current_key: PhysicsStepKey,
    ) -> Result<(), PhysicsError> {
        self.ensure_current_key(current_key)?;
        let prepared_key = self.next_substep_key()?;
        let config = self.config.ok_or(PhysicsError::StepNotStarted)?;
        {
            let world = &self.world;
            let rng = self.rng.as_ref().ok_or(PhysicsError::StepNotStarted)?;
            let allocators = self
                .allocators
                .as_ref()
                .ok_or(PhysicsError::StepNotStarted)?;
            let PhysicsPipelineWorkspace {
                pellet_index,
                movement,
                food,
                collision,
                effects,
                substep,
                #[cfg(feature = "engine-test-hooks")]
                allocation_snapshot,
                #[cfg(feature = "engine-test-hooks")]
                allocation_cursor,
                #[cfg(feature = "engine-test-hooks")]
                phase_allocations,
            } = pipeline;
            pellet_index.rebuild(
                world,
                config.pellet_index_cell_size,
                config.maximum_pellet_index_entries,
            )?;
            #[cfg(feature = "engine-test-hooks")]
            {
                phase_allocations.pellet_index = phase_allocations
                    .pellet_index
                    .saturating_add(allocation_delta(*allocation_snapshot, allocation_cursor));
            }
            let movement = movement.prepare(
                world,
                config.movement,
                config.substep_dt,
                config.maximum_body_points,
                config.maximum_pellets,
            )?;
            #[cfg(feature = "engine-test-hooks")]
            {
                phase_allocations.movement = phase_allocations
                    .movement
                    .saturating_add(allocation_delta(*allocation_snapshot, allocation_cursor));
            }
            let indexed = IndexedPelletWorld::from_index(
                world,
                std::mem::replace(pellet_index, PelletSpatialIndex::empty()),
            );
            let food_result = food.prepare(
                &indexed,
                movement,
                config.movement,
                config.food,
                config.maximum_body_points,
                config.maximum_pellets,
            );
            *pellet_index = indexed.into_index();
            let food = food_result?;
            #[cfg(feature = "engine-test-hooks")]
            {
                phase_allocations.food = phase_allocations
                    .food
                    .saturating_add(allocation_delta(*allocation_snapshot, allocation_cursor));
            }
            let collision = collision.prepare(food, config.collision)?;
            #[cfg(feature = "engine-test-hooks")]
            {
                phase_allocations.collision = phase_allocations
                    .collision
                    .saturating_add(allocation_delta(*allocation_snapshot, allocation_cursor));
            }
            let effects = effects.prepare(
                collision,
                rng,
                allocators,
                config.movement,
                config.food,
                config.death,
                config.maximum_pellets,
            )?;
            #[cfg(feature = "engine-test-hooks")]
            {
                phase_allocations.effects = phase_allocations
                    .effects
                    .saturating_add(allocation_delta(*allocation_snapshot, allocation_cursor));
            }
            substep.prepare(effects, world, prepared_key, config)?;
            #[cfg(feature = "engine-test-hooks")]
            {
                phase_allocations.result_application = phase_allocations
                    .result_application
                    .saturating_add(allocation_delta(*allocation_snapshot, allocation_cursor));
            }
        }
        self.accept_prepared_substep(&mut pipeline.substep, prepared_key)?;
        #[cfg(feature = "engine-test-hooks")]
        {
            pipeline.phase_allocations.accept =
                pipeline
                    .phase_allocations
                    .accept
                    .saturating_add(allocation_delta(
                        pipeline.allocation_snapshot,
                        &mut pipeline.allocation_cursor,
                    ));
        }
        Ok(())
    }

    /// Atomically replace the working boundary with one completely prepared substep.
    fn accept_prepared_substep(
        &mut self,
        substep: &mut PhysicsSubstepWorkspace,
        prepared_key: PhysicsSubstepKey,
    ) -> Result<(), PhysicsError> {
        let expected = self.next_substep_key()?;
        if prepared_key != expected || substep.ready != Some(expected) {
            return Err(PhysicsError::SubstepIdentityMismatch);
        }
        if substep.staged_config != self.config {
            return Err(PhysicsError::SubstepConfigMismatch);
        }
        let next_rng = substep
            .next_rng
            .as_mut()
            .ok_or(PhysicsError::SubstepNotReady)?;
        let next_allocators = substep
            .next_allocators
            .as_mut()
            .ok_or(PhysicsError::SubstepNotReady)?;
        let current_rng = self.rng.as_mut().ok_or(PhysicsError::StepNotStarted)?;
        let current_allocators = self
            .allocators
            .as_mut()
            .ok_or(PhysicsError::StepNotStarted)?;

        let required_baseline_events = self
            .baseline_deaths
            .len()
            .checked_add(substep.baseline_deaths.len())
            .ok_or(PhysicsError::ArithmeticOverflow {
                context: "step baseline events",
            })?;
        reserve_for(
            &mut self.baseline_deaths,
            required_baseline_events,
            "step baseline events",
        )?;
        for event in &substep.baseline_deaths {
            if self
                .baseline_deaths
                .binary_search_by_key(&event.snake_id, |current| current.snake_id)
                .is_ok()
            {
                return Err(PhysicsError::DuplicateBaselineDeath(event.snake_id));
            }
        }

        std::mem::swap(&mut self.world.snakes, &mut substep.next_world.snakes);
        std::mem::swap(
            &mut self.world.body_points,
            &mut substep.next_world.body_points,
        );
        std::mem::swap(&mut self.world.pellets, &mut substep.next_world.pellets);
        std::mem::swap(&mut self.validation, &mut substep.validation);
        std::mem::swap(current_rng, next_rng);
        std::mem::swap(current_allocators, next_allocators);
        self.baseline_deaths
            .extend_from_slice(&substep.baseline_deaths);
        self.baseline_deaths
            .sort_unstable_by_key(|event| event.snake_id);
        self.completed_substeps += 1;
        substep.ready = None;
        Ok(())
    }

    fn ensure_current_key(&self, current_key: PhysicsStepKey) -> Result<(), PhysicsError> {
        let staged = self.key.ok_or(PhysicsError::StepNotStarted)?;
        if let Some(field) = staged.first_mismatch(current_key) {
            return Err(PhysicsError::StepKeyMismatch { field });
        }
        Ok(())
    }

    /// Return a complete result only after every declared substep was accepted.
    pub fn finish(
        &self,
        current_key: PhysicsStepKey,
    ) -> Result<PreparedPhysicsStep<'_>, PhysicsError> {
        self.ensure_current_key(current_key)?;
        let staged = self.key.ok_or(PhysicsError::StepNotStarted)?;
        if self.completed_substeps != self.expected_substeps {
            return Err(PhysicsError::IncompleteSubsteps {
                completed: self.completed_substeps,
                expected: self.expected_substeps,
            });
        }
        Ok(PreparedPhysicsStep {
            key: staged,
            world: &self.world,
            rng: self.rng.as_ref().ok_or(PhysicsError::StepNotStarted)?,
            allocators: self
                .allocators
                .as_ref()
                .ok_or(PhysicsError::StepNotStarted)?,
            baseline_deaths: &self.baseline_deaths,
            diagnostics: self.diagnostics(),
        })
    }

    /// Borrow the complete mutable buffers only after every declared substep.
    pub(crate) fn publication_buffers(
        &mut self,
        current_key: PhysicsStepKey,
    ) -> Result<PhysicsPublicationBuffers<'_>, PhysicsError> {
        self.ensure_current_key(current_key)?;
        if self.completed_substeps != self.expected_substeps {
            return Err(PhysicsError::IncompleteSubsteps {
                completed: self.completed_substeps,
                expected: self.expected_substeps,
            });
        }
        let Self {
            world,
            rng,
            allocators,
            ..
        } = self;
        Ok(PhysicsPublicationBuffers {
            world,
            rng: rng.as_mut().ok_or(PhysicsError::StepNotStarted)?,
            allocators: allocators.as_mut().ok_or(PhysicsError::StepNotStarted)?,
        })
    }

    /// Current work and retained capacities, including incomplete/rejected state.
    #[must_use]
    pub fn diagnostics(&self) -> PhysicsStepDiagnostics {
        PhysicsStepDiagnostics {
            expected_substeps: self.expected_substeps,
            completed_substeps: self.completed_substeps,
            snakes: self.world.snakes.len(),
            body_points: self.world.body_points.len(),
            pellets: self.world.pellets.len(),
            baseline_deaths: self.baseline_deaths.len(),
            controlled_snakes: self.controlled_snake_ids.len(),
            snake_capacity: self.world.snakes.capacity(),
            body_point_capacity: self.world.body_points.capacity(),
            pellet_capacity: self.world.pellets.capacity(),
            controller_lease_capacity: self.world.controller_leases.capacity(),
            controller_text_capacity: controller_text_capacity(&self.world.controller_leases),
            baseline_rng_capacity: self.rng.as_ref().map_or(0, |rng| rng.baselines.capacity()),
            baseline_rng_spare_capacity: self.rng_copy_scratch.baseline_gaussian_spares.capacity(),
            rng_text_capacity: rng_text_capacity(self.rng.as_ref(), &self.rng_copy_scratch),
            controlled_snake_capacity: self.controlled_snake_ids.capacity(),
            baseline_event_capacity: self.baseline_deaths.capacity(),
            validation_order_capacity: self.validation.snake_order.capacity(),
            validation_range_capacity: self.validation.body_ranges.capacity(),
            pellet_order_capacity: self.validation.pellet_order.capacity(),
        }
    }

    /// Discard the working result while retaining reusable allocations.
    pub fn discard(&mut self) {
        self.key = None;
        self.world.snakes.clear();
        self.world.body_points.clear();
        self.world.pellets.clear();
        self.controlled_snake_ids.clear();
        self.baseline_deaths.clear();
        self.config = None;
        self.expected_substeps = 0;
        self.completed_substeps = 0;
    }
}

fn map_reuse_error(error: FixedStepPrefixError) -> PhysicsError {
    match error {
        FixedStepPrefixError::AllocationFailed { context, required } => {
            PhysicsError::AllocationFailed { context, required }
        }
        FixedStepPrefixError::ArithmeticOverflow { context } => {
            PhysicsError::ArithmeticOverflow { context }
        }
        _ => PhysicsError::ShapeMismatch,
    }
}

fn find_snake_index(
    snakes: &[SnakeState],
    order: &[usize],
    id: u64,
) -> Result<usize, PhysicsError> {
    order
        .binary_search_by_key(&id, |index| snakes[*index].id)
        .map(|position| order[position])
        .map_err(|_| PhysicsError::UnknownSnake(id))
}

fn prepare_snake_order(snakes: &[SnakeState], order: &mut Vec<usize>) -> Result<(), PhysicsError> {
    order.clear();
    reserve_for(order, snakes.len(), "physical snake order")?;
    order.extend(0..snakes.len());
    order.sort_unstable_by_key(|index| snakes[*index].id);
    for pair in order.windows(2) {
        if snakes[pair[0]].id == snakes[pair[1]].id {
            return Err(PhysicsError::DuplicateSnakeId(snakes[pair[0]].id));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct PhysicalValidationScratch {
    snake_order: Vec<usize>,
    body_ranges: Vec<(usize, usize, u64)>,
    pellet_order: Vec<usize>,
}

fn validate_physical_world(
    world: &WorldState,
    scratch: &mut PhysicalValidationScratch,
) -> Result<(), PhysicsError> {
    scratch.snake_order.clear();
    scratch.body_ranges.clear();
    scratch.pellet_order.clear();
    prepare_snake_order(&world.snakes, &mut scratch.snake_order)?;
    reserve_for(
        &mut scratch.body_ranges,
        world.snakes.len(),
        "physical body ranges",
    )?;
    reserve_for(
        &mut scratch.pellet_order,
        world.pellets.len(),
        "physical pellet order",
    )?;
    for &snake_index in &scratch.snake_order {
        let snake = &world.snakes[snake_index];
        validate_point(snake.position, "snake position")?;
        validate_point(snake.previous_position, "snake previous position")?;
        for (field, value) in [
            ("snake direction", snake.direction),
            ("snake radius", snake.radius),
            ("snake speed", snake.speed),
            ("snake points", snake.points),
            ("snake food", snake.food),
            ("snake target length", snake.target_length),
        ] {
            if !value.is_finite() {
                return Err(PhysicsError::NonFiniteState { field });
            }
        }
        let end = snake
            .body
            .start
            .checked_add(snake.body.len)
            .ok_or(PhysicsError::InvalidBodyRange { snake_id: snake.id })?;
        if end > world.body_points.len() || (snake.alive && snake.body.len == 0) {
            return Err(PhysicsError::InvalidBodyRange { snake_id: snake.id });
        }
        if snake.body.len > 0 && world.body_points[snake.body.start] != snake.position {
            return Err(PhysicsError::InvalidBodyRange { snake_id: snake.id });
        }
        scratch.body_ranges.push((snake.body.start, end, snake.id));
    }
    scratch.body_ranges.sort_unstable_by_key(|range| range.0);
    for pair in scratch.body_ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(PhysicsError::InvalidBodyRange {
                snake_id: pair[1].2,
            });
        }
    }
    for point in &world.body_points {
        validate_point(*point, "body point")?;
    }
    scratch.pellet_order.extend(0..world.pellets.len());
    scratch
        .pellet_order
        .sort_unstable_by_key(|index| world.pellets[*index].id);
    for pair in scratch.pellet_order.windows(2) {
        if world.pellets[pair[0]].id == world.pellets[pair[1]].id {
            return Err(PhysicsError::InvalidPellet(world.pellets[pair[0]].id));
        }
    }
    for &pellet_index in &scratch.pellet_order {
        let pellet = &world.pellets[pellet_index];
        validate_pellet(pellet)?;
        if scratch
            .snake_order
            .binary_search_by_key(&pellet.id, |index| world.snakes[*index].id)
            .is_ok()
            || pellet.owner.is_some_and(|owner| {
                scratch
                    .snake_order
                    .binary_search_by_key(&owner, |index| world.snakes[*index].id)
                    .is_err()
            })
        {
            return Err(PhysicsError::InvalidPellet(pellet.id));
        }
    }
    Ok(())
}

fn validate_pellet(pellet: &PelletState) -> Result<(), PhysicsError> {
    if pellet.id == 0 || !pellet.value.is_finite() || pellet.value <= 0.0 {
        return Err(PhysicsError::InvalidPellet(pellet.id));
    }
    validate_point(pellet.position, "pellet position")
}

fn validate_point(point: WorldPoint, field: &'static str) -> Result<(), PhysicsError> {
    if point.x.is_finite() && point.y.is_finite() {
        Ok(())
    } else {
        Err(PhysicsError::NonFiniteState { field })
    }
}

fn validate_baseline_events(
    events: &[BaselineDeathEvent],
    snakes: &[SnakeState],
) -> Result<(), PhysicsError> {
    let mut prior = None;
    for event in events {
        if prior.is_some_and(|id| event.snake_id <= id) {
            return Err(PhysicsError::NonCanonicalBaselineEvents);
        }
        prior = Some(event.snake_id);
        let snake = snakes
            .iter()
            .find(|snake| snake.id == event.snake_id)
            .ok_or(PhysicsError::UnknownSnake(event.snake_id))?;
        if snake.baseline_slot != Some(event.slot) || snake.alive {
            return Err(PhysicsError::InvalidBaselineDeath(event.snake_id));
        }
    }
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), PhysicsError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| PhysicsError::AllocationFailed { context, required })?;
    }
    Ok(())
}

#[cfg(feature = "engine-test-hooks")]
fn allocation_delta(snapshot: Option<fn() -> u64>, cursor: &mut Option<u64>) -> u64 {
    let Some(snapshot) = snapshot else {
        return 0;
    };
    let current = snapshot();
    let previous = cursor.replace(current).unwrap_or(current);
    current.saturating_sub(previous)
}

/// Checked physics staging failure. No variant publishes partial authority.
#[derive(Clone, Debug, PartialEq)]
pub enum PhysicsError {
    /// Generation/config/operation identity is incomplete.
    InvalidStepKey,
    /// A fixed step must contain at least one collision substep.
    InvalidSubstepCount,
    /// The working transaction has not started.
    StepNotStarted,
    /// A substep has not completed preparation.
    SubstepNotReady,
    /// Movement preparation rejected the complete substep.
    Movement(Box<MovementError>),
    /// Complete spatial-index construction rejected the substep.
    Spatial(Box<SpatialIndexError>),
    /// Food claim/finalization rejected the complete substep.
    Food(Box<FoodError>),
    /// Collision detection rejected the complete substep.
    Collision(Box<CollisionError>),
    /// RNG/allocator effect realization rejected the complete substep.
    Effects(Box<EffectError>),
    /// The projected kill-credit config changed inside one step identity.
    SubstepConfigMismatch,
    /// A substep was prepared from a different physical boundary.
    SourceWorldMismatch,
    /// Input phase shapes or stable identities disagree.
    ShapeMismatch,
    /// A stable snake identity appears more than once.
    DuplicateSnakeId(u64),
    /// Two controller leases target the same snake.
    DuplicateControlledSnake,
    /// A staged request references no snake in this boundary.
    UnknownSnake(u64),
    /// Death proposals are not strictly stable-ID ordered.
    NonCanonicalDeathOrder,
    /// Kill awards are not strictly victim-ID ordered.
    NonCanonicalAwardOrder,
    /// Baseline events are not strictly stable-ID ordered.
    NonCanonicalBaselineEvents,
    /// A kill award references no death in the same snapshot.
    AwardWithoutDeath { victim_id: u64 },
    /// A baseline death event disagrees with staged snake state.
    InvalidBaselineDeath(u64),
    /// Stable kill count overflowed.
    KillCountOverflow { killer_id: u64 },
    /// Kill points produced NaN or infinity.
    NonFiniteKillPoints { killer_id: u64 },
    /// A body range is invalid or overlaps another body.
    InvalidBodyRange { snake_id: u64 },
    /// A pellet identity/value is invalid.
    InvalidPellet(u64),
    /// A staged physical scalar is NaN or infinity.
    NonFiniteState { field: &'static str },
    /// A projected configuration value is invalid.
    InvalidConfig { field: &'static str },
    /// More substeps were requested after the declared count completed.
    TooManySubsteps { expected: usize },
    /// The parent fixed-step identity changed.
    StepKeyMismatch { field: PhysicsStepKeyField },
    /// Internal staged/expected substep identity disagrees.
    SubstepIdentityMismatch,
    /// Finish was requested before every declared substep completed.
    IncompleteSubsteps { completed: usize, expected: usize },
    /// One baseline cannot die twice in a fixed step.
    DuplicateBaselineDeath(u64),
    /// Checked count arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Scratch reservation failed before authority publication.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
}

impl Display for PhysicsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStepKey => write!(formatter, "invalid physics step identity"),
            Self::InvalidSubstepCount => {
                write!(formatter, "physics step needs at least one substep")
            }
            Self::StepNotStarted => write!(formatter, "physics step has not started"),
            Self::SubstepNotReady => write!(formatter, "physics substep is not ready"),
            Self::Movement(error) => write!(formatter, "movement phase failed: {error}"),
            Self::Spatial(error) => write!(formatter, "spatial-index phase failed: {error}"),
            Self::Food(error) => write!(formatter, "food phase failed: {error}"),
            Self::Collision(error) => write!(formatter, "collision phase failed: {error}"),
            Self::Effects(error) => write!(formatter, "effect phase failed: {error}"),
            Self::SubstepConfigMismatch => {
                write!(formatter, "physics substep configuration changed")
            }
            Self::SourceWorldMismatch => write!(formatter, "physics substep source world changed"),
            Self::ShapeMismatch => write!(formatter, "physics phase shapes disagree"),
            Self::DuplicateSnakeId(id) => write!(formatter, "duplicate physics snake ID {id}"),
            Self::DuplicateControlledSnake => {
                write!(formatter, "controller snake IDs are not unique")
            }
            Self::UnknownSnake(id) => write!(formatter, "physics references unknown snake {id}"),
            Self::NonCanonicalDeathOrder => {
                write!(formatter, "death proposals are not in stable order")
            }
            Self::NonCanonicalAwardOrder => {
                write!(formatter, "kill awards are not in stable victim order")
            }
            Self::NonCanonicalBaselineEvents => {
                write!(formatter, "baseline deaths are not in stable order")
            }
            Self::AwardWithoutDeath { victim_id } => {
                write!(formatter, "kill award victim {victim_id} did not die")
            }
            Self::InvalidBaselineDeath(id) => {
                write!(formatter, "invalid baseline death event for snake {id}")
            }
            Self::KillCountOverflow { killer_id } => {
                write!(formatter, "killer {killer_id} kill count overflowed")
            }
            Self::NonFiniteKillPoints { killer_id } => {
                write!(formatter, "killer {killer_id} points became non-finite")
            }
            Self::InvalidBodyRange { snake_id } => write!(
                formatter,
                "snake {snake_id} has an invalid physics body range"
            ),
            Self::InvalidPellet(id) => {
                write!(formatter, "pellet {id} has an invalid physics value")
            }
            Self::NonFiniteState { field } => write!(formatter, "physics {field} must be finite"),
            Self::InvalidConfig { field } => write!(formatter, "invalid physics config: {field}"),
            Self::TooManySubsteps { expected } => write!(
                formatter,
                "physics step already completed its {expected} substeps"
            ),
            Self::StepKeyMismatch { field } => {
                write!(formatter, "physics step identity changed at {field:?}")
            }
            Self::SubstepIdentityMismatch => write!(
                formatter,
                "physics substep identity changed before acceptance"
            ),
            Self::IncompleteSubsteps {
                completed,
                expected,
            } => write!(
                formatter,
                "physics step completed {completed} of {expected} substeps"
            ),
            Self::DuplicateBaselineDeath(id) => write!(
                formatter,
                "baseline snake {id} died twice in one physics step"
            ),
            Self::ArithmeticOverflow { context } => write!(
                formatter,
                "checked arithmetic overflow while calculating {context}"
            ),
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "physics could not reserve {required} entries for {context}"
            ),
        }
    }
}

impl Error for PhysicsError {}

impl From<MovementError> for PhysicsError {
    fn from(error: MovementError) -> Self {
        Self::Movement(Box::new(error))
    }
}

impl From<SpatialIndexError> for PhysicsError {
    fn from(error: SpatialIndexError) -> Self {
        Self::Spatial(Box::new(error))
    }
}

impl From<FoodError> for PhysicsError {
    fn from(error: FoodError) -> Self {
        Self::Food(Box::new(error))
    }
}

impl From<CollisionError> for PhysicsError {
    fn from(error: CollisionError) -> Self {
        Self::Collision(Box::new(error))
    }
}

impl From<EffectError> for PhysicsError {
    fn from(error: EffectError) -> Self {
        Self::Effects(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rng::StatefulRng;
    use crate::engine::state::{
        BaselineRngState, BaselineStrategyState, BodyRange, ControllerKind, ControllerLease,
        ControllerLeaseStatus, LatestControllerAction, SnakeKind, ALLOCATOR_VERSION,
        BASELINE_ENTITY_ID_START, EXTERNAL_ENTITY_ID_START, RESURRECTED_ENTITY_ID_START,
        RNG_BUNDLE_VERSION,
    };

    type PhaseWorkspaces = PhysicsPipelineWorkspace;

    fn key(operation_epoch: u64) -> PhysicsStepKey {
        PhysicsStepKey::new(2, 3, 40, 7, 11, [0x5a; 32], operation_epoch)
    }

    fn rng_bundle(baseline_count: usize) -> RngStateBundle {
        RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: StatefulRng::new(11.0).export_state(),
            evolution: StatefulRng::new(22.0).export_state(),
            external_controller: StatefulRng::new(33.0).export_state(),
            baselines: (0..baseline_count)
                .map(|slot| BaselineRngState {
                    slot: u32::try_from(slot).expect("fixture slot should fit"),
                    state: StatefulRng::new(44.0 + slot as f64).export_state(),
                })
                .collect(),
        }
    }

    fn allocators() -> AllocatorState {
        AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1_000,
            next_brain_id: 1,
            next_genome_id: 1,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        }
    }

    fn snake(
        id: u64,
        kind: SnakeKind,
        position: WorldPoint,
        direction: f64,
        length: usize,
    ) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("fixture ID should fit"),
            kind,
            alive: true,
            population_slot: (kind == SnakeKind::Evolved).then_some(0),
            brain: None,
            baseline_slot: (kind == SnakeKind::Baseline).then_some(0),
            baseline_strategy: (kind == SnakeKind::Baseline).then_some(BaselineStrategyState::Roam),
            position,
            previous_position: position,
            direction,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 10.0,
            kills: 0,
            target_length: length as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: 0,
                len: length,
            },
            skin: 0,
        }
    }

    fn line_body(position: WorldPoint, direction: f64, length: usize) -> Vec<WorldPoint> {
        (0..length)
            .map(|index| WorldPoint {
                x: position.x - direction.cos() * index as f64 * 7.5,
                y: position.y - direction.sin() * index as f64 * 7.5,
            })
            .collect()
    }

    fn pack_world(entries: Vec<(SnakeState, Vec<WorldPoint>)>) -> WorldState {
        let mut world = WorldState::default();
        for (mut snake, body) in entries {
            snake.body = BodyRange {
                start: world.body_points.len(),
                len: body.len(),
            };
            snake.position = body[0];
            snake.previous_position = body[0];
            world.body_points.extend(body);
            world.snakes.push(snake);
        }
        world
    }

    fn body_collision_world(victim_kind: SnakeKind, reverse: bool) -> WorldState {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let owner_position = WorldPoint { x: 100.0, y: 0.0 };
        let victim = (
            snake(7, victim_kind, victim_position, 0.0, 12),
            line_body(victim_position, 0.0, 12),
        );
        let owner = (
            snake(20, SnakeKind::Evolved, owner_position, 0.0, 15),
            line_body(owner_position, 0.0, 15),
        );
        if reverse {
            pack_world(vec![owner, victim])
        } else {
            pack_world(vec![victim, owner])
        }
    }

    fn head_head_world() -> WorldState {
        let position = WorldPoint { x: 0.0, y: 0.0 };
        pack_world(vec![
            (
                snake(1, SnakeKind::Evolved, position, 0.0, 8),
                line_body(position, 0.0, 8),
            ),
            (
                snake(2, SnakeKind::Evolved, position, std::f64::consts::PI, 8),
                line_body(position, std::f64::consts::PI, 8),
            ),
        ])
    }

    fn killed_body_owner_world() -> WorldState {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let owner_position = WorldPoint { x: 100.0, y: 0.0 };
        pack_world(vec![
            (
                snake(1, SnakeKind::Evolved, victim_position, 0.0, 5),
                line_body(victim_position, 0.0, 5),
            ),
            (
                snake(2, SnakeKind::Evolved, owner_position, 0.0, 15),
                line_body(owner_position, 0.0, 15),
            ),
            (
                snake(
                    3,
                    SnakeKind::Evolved,
                    owner_position,
                    std::f64::consts::PI,
                    5,
                ),
                line_body(owner_position, std::f64::consts::PI, 5),
            ),
        ])
    }

    fn lease(snake_id: u64) -> ControllerLease {
        ControllerLease {
            id: 1,
            snake_id,
            kind: ControllerKind::Player,
            connection_id: Some(1),
            scope: "run".to_owned(),
            resume_token: "token".to_owned(),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                turn: 0.0,
                boost: false,
                client_tick: 0,
                arrival_sequence: 1,
                accepted_at_ms: 1,
            },
            last_observed_at_ms: 1,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        }
    }

    fn stage_and_accept(
        step: &mut PhysicsStepWorkspace,
        phases: &mut PhaseWorkspaces,
    ) -> Result<(), PhysicsError> {
        let current_key = step.key.ok_or(PhysicsError::StepNotStarted)?;
        step.advance_substep(phases, current_key)
    }

    fn snake_by_id(world: &WorldState, id: u64) -> &SnakeState {
        world
            .snakes
            .iter()
            .find(|snake| snake.id == id)
            .expect("fixture snake should exist")
    }

    fn body_by_id(world: &WorldState, id: u64) -> &[WorldPoint] {
        let snake = snake_by_id(world, id);
        &world.body_points[snake.body.start..snake.body.start + snake.body.len]
    }

    #[test]
    fn body_death_and_owner_credit_commit_only_to_the_working_transaction() {
        let source = body_collision_world(SnakeKind::Evolved, false);
        let source_copy = source.clone();
        let rng = rng_bundle(0);
        let allocators = allocators();
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng,
            &allocators,
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .expect("step should begin");
        let mut phases = PhaseWorkspaces::default();
        stage_and_accept(&mut step, &mut phases).expect("substep should join");

        assert_eq!(source, source_copy);
        assert!(snake_by_id(&source, 7).alive);
        assert_eq!(snake_by_id(&source, 20).kills, 0);
        assert_eq!(snake_by_id(step.world().unwrap(), 20).points, 410.0);

        let prepared = step.finish(key(1)).expect("step should finish");
        assert!(!snake_by_id(prepared.world(), 7).alive);
        assert_eq!(snake_by_id(prepared.world(), 20).kills, 1);
        assert_eq!(snake_by_id(prepared.world(), 20).points, 410.0);
        assert_eq!(prepared.world().pellets.len(), 4);
        assert_eq!(prepared.allocators().next_entity_id, 1_004);
        assert_ne!(prepared.rng().world, rng.world);
        assert_eq!(prepared.rng().evolution, rng.evolution);
        assert_eq!(prepared.diagnostics().completed_substeps, 1);
    }

    #[test]
    fn simultaneous_heads_die_without_any_kill_credit() {
        let source = head_head_world();
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        let mut phases = PhaseWorkspaces::default();
        stage_and_accept(&mut step, &mut phases).unwrap();
        let prepared = step.finish(key(1)).unwrap();
        assert!(prepared.world().snakes.iter().all(|snake| !snake.alive));
        assert!(prepared.world().snakes.iter().all(|snake| snake.kills == 0));
        assert!(prepared
            .world()
            .snakes
            .iter()
            .all(|snake| snake.points == 10.0));
        assert_eq!(prepared.world().pellets.len(), 4);
    }

    #[test]
    fn body_owner_receives_one_credit_even_when_the_same_snapshot_kills_it() {
        let source = killed_body_owner_world();
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        step.advance_substep(&mut PhaseWorkspaces::default(), key(1))
            .unwrap();
        let prepared = step.finish(key(1)).unwrap();
        assert!(prepared.world().snakes.iter().all(|snake| !snake.alive));
        assert_eq!(snake_by_id(prepared.world(), 2).kills, 1);
        assert_eq!(snake_by_id(prepared.world(), 2).points, 410.0);
        assert_eq!(snake_by_id(prepared.world(), 1).kills, 0);
        assert_eq!(snake_by_id(prepared.world(), 3).kills, 0);
    }

    #[test]
    fn reversing_container_order_preserves_per_id_state_bodies_pellets_rng_and_ids() {
        fn execute(source: &WorldState) -> (WorldState, RngStateBundle, AllocatorState) {
            let mut step = PhysicsStepWorkspace::new();
            step.begin(
                key(1),
                source,
                &rng_bundle(0),
                &allocators(),
                PhysicsConfig::typescript_defaults(),
                1,
            )
            .unwrap();
            stage_and_accept(&mut step, &mut PhaseWorkspaces::default()).unwrap();
            let prepared = step.finish(key(1)).unwrap();
            (
                prepared.world().clone(),
                prepared.rng().clone(),
                prepared.allocators().clone(),
            )
        }

        let forward = execute(&body_collision_world(SnakeKind::Evolved, false));
        let reversed = execute(&body_collision_world(SnakeKind::Evolved, true));
        for id in [7, 20] {
            assert_eq!(snake_by_id(&forward.0, id), snake_by_id(&reversed.0, id));
            assert_eq!(body_by_id(&forward.0, id), body_by_id(&reversed.0, id));
        }
        assert_eq!(forward.0.pellets, reversed.0.pellets);
        assert_eq!(forward.1, reversed.1);
        assert_eq!(forward.2, reversed.2);
    }

    #[test]
    fn controlled_death_remains_a_complete_intermediate_physics_result() {
        let mut source = body_collision_world(SnakeKind::External, false);
        source.controller_leases.push(lease(7));
        let source_copy = source.clone();
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        let mut phases = PhaseWorkspaces::default();
        step.advance_substep(&mut phases, key(1))
            .expect("physics must retain the controlled death for the outer replacement join");
        assert!(phases.substep.ready.is_none());
        let prepared = step.finish(key(1)).expect("complete intermediate physics");
        assert!(!snake_by_id(prepared.world(), 7).alive);
        assert_eq!(prepared.world().controller_leases, source.controller_leases);
        assert_eq!(source, source_copy);
    }

    #[test]
    fn stale_key_components_cannot_advance_or_finish_a_working_step() {
        let source = body_collision_world(SnakeKind::Evolved, false);
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        let mut phases = PhaseWorkspaces::default();
        assert_eq!(
            step.advance_substep(&mut phases, key(2)),
            Err(PhysicsError::StepKeyMismatch {
                field: PhysicsStepKeyField::OperationEpoch,
            })
        );
        assert_eq!(step.diagnostics().completed_substeps, 0);
        let wrong_world = PhysicsStepKey::new(3, 3, 40, 7, 11, [0x5a; 32], 1);
        assert_eq!(
            step.advance_substep(&mut phases, wrong_world),
            Err(PhysicsError::StepKeyMismatch {
                field: PhysicsStepKeyField::WorldEpoch,
            })
        );
        let wrong_revision = PhysicsStepKey::new(2, 3, 40, 7, 12, [0x5a; 32], 1);
        assert_eq!(
            step.advance_substep(&mut phases, wrong_revision),
            Err(PhysicsError::StepKeyMismatch {
                field: PhysicsStepKeyField::ConfigRevision,
            })
        );
        let wrong_hash = PhysicsStepKey::new(2, 3, 40, 7, 11, [0x5b; 32], 1);
        assert_eq!(
            step.advance_substep(&mut phases, wrong_hash),
            Err(PhysicsError::StepKeyMismatch {
                field: PhysicsStepKeyField::ConfigHash,
            })
        );
        assert_eq!(step.diagnostics().completed_substeps, 0);
        step.advance_substep(&mut phases, key(1)).unwrap();
        assert!(matches!(
            step.finish(key(2)),
            Err(PhysicsError::StepKeyMismatch {
                field: PhysicsStepKeyField::OperationEpoch,
            })
        ));
    }

    #[test]
    fn multi_substep_step_carries_rng_allocator_and_awards_only_once() {
        let source = body_collision_world(SnakeKind::Evolved, false);
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            2,
        )
        .unwrap();
        let mut phases = PhaseWorkspaces::default();
        stage_and_accept(&mut step, &mut phases).unwrap();
        assert!(matches!(
            step.finish(key(1)),
            Err(PhysicsError::IncompleteSubsteps {
                completed: 1,
                expected: 2
            })
        ));
        let after_first_rng = step.rng().unwrap().clone();
        let after_first_allocator = step.allocators().unwrap().clone();
        stage_and_accept(&mut step, &mut phases).unwrap();
        let prepared = step.finish(key(1)).unwrap();
        assert_eq!(snake_by_id(prepared.world(), 20).kills, 1);
        assert_eq!(snake_by_id(prepared.world(), 20).points, 410.0);
        assert_eq!(prepared.rng(), &after_first_rng);
        assert_eq!(prepared.allocators(), &after_first_allocator);
        assert_eq!(prepared.world().pellets.len(), 4);
        assert!(matches!(
            step.next_substep_key(),
            Err(PhysicsError::TooManySubsteps { expected: 2 })
        ));
    }

    #[test]
    fn failure_after_one_substep_cannot_expose_a_partial_fixed_step() {
        let source = body_collision_world(SnakeKind::Evolved, false);
        let source_copy = source.clone();
        let rng = rng_bundle(0);
        let rng_copy = rng.clone();
        let allocators = allocators();
        let allocator_copy = allocators.clone();
        let mut step = PhysicsStepWorkspace::new();
        let mut config = PhysicsConfig::typescript_defaults();
        config.maximum_pellet_index_entries = 1;
        step.begin(key(1), &source, &rng, &allocators, config, 2)
            .unwrap();
        let mut phases = PhaseWorkspaces::default();
        step.advance_substep(&mut phases, key(1)).unwrap();
        assert_eq!(step.diagnostics().completed_substeps, 1);

        let error = step
            .advance_substep(&mut phases, key(1))
            .expect_err("second substep should reject its larger pellet index");
        assert!(matches!(
            error,
            PhysicsError::Spatial(error)
                if matches!(
                    *error,
                    SpatialIndexError::EntryLimitExceeded {
                        kind: "pellet",
                        required: 4,
                        maximum: 1,
                    }
                )
        ));
        assert!(matches!(
            step.finish(key(1)),
            Err(PhysicsError::IncompleteSubsteps {
                completed: 1,
                expected: 2,
            })
        ));
        assert_eq!(source, source_copy);
        assert_eq!(rng, rng_copy);
        assert_eq!(allocators, allocator_copy);
    }

    #[test]
    fn kill_overflow_failure_keeps_step_unmodified_and_valid_retry_clears_scratch() {
        let mut source = body_collision_world(SnakeKind::Evolved, false);
        snake_by_id_mut(&mut source, 20).kills = u64::MAX;
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        let before = step.world().unwrap().clone();
        let mut phases = PhaseWorkspaces::default();
        let error = step
            .advance_substep(&mut phases, key(1))
            .expect_err("kill overflow should reject scratch");
        assert_eq!(error, PhysicsError::KillCountOverflow { killer_id: 20 });
        assert!(phases.substep.ready.is_none());
        assert_eq!(step.world().unwrap(), &before);

        snake_by_id_mut(&mut source, 20).kills = 0;
        step.begin(
            key(2),
            &source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        stage_and_accept(&mut step, &mut phases).unwrap();
        assert_eq!(
            snake_by_id(step.finish(key(2)).unwrap().world(), 20).kills,
            1
        );
    }

    fn snake_by_id_mut(world: &mut WorldState, id: u64) -> &mut SnakeState {
        world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == id)
            .expect("fixture snake should exist")
    }

    #[test]
    fn baseline_wall_death_accumulates_one_respawn_event_without_rng_draw() {
        let position = WorldPoint { x: 3_495.0, y: 0.0 };
        let source = pack_world(vec![(
            snake(9, SnakeKind::Baseline, position, 0.0, 8),
            line_body(position, 0.0, 8),
        )]);
        let rng = rng_bundle(1);
        let allocators = allocators();
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng,
            &allocators,
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        stage_and_accept(&mut step, &mut PhaseWorkspaces::default()).unwrap();
        let prepared = step.finish(key(1)).unwrap();
        assert_eq!(
            prepared.baseline_deaths(),
            &[BaselineDeathEvent {
                slot: 0,
                snake_id: 9,
            }]
        );
        assert!(prepared.world().pellets.is_empty());
        assert_eq!(prepared.rng(), &rng);
        assert_eq!(prepared.allocators(), &allocators);
        let baseline_deaths = prepared.prepared_baseline_deaths();
        assert_eq!(baseline_deaths.key(), key(1));
        assert!(std::ptr::eq(baseline_deaths.world(), prepared.world()));
        assert_eq!(baseline_deaths.events(), prepared.baseline_deaths());
    }

    #[test]
    fn normal_baseline_death_uses_its_stream_and_emits_one_respawn_event() {
        let source = body_collision_world(SnakeKind::Baseline, false);
        let rng = rng_bundle(1);
        let allocators = allocators();
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            &source,
            &rng,
            &allocators,
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        step.advance_substep(&mut PhaseWorkspaces::default(), key(1))
            .unwrap();
        let prepared = step.finish(key(1)).unwrap();
        assert_eq!(
            prepared.baseline_deaths(),
            &[BaselineDeathEvent {
                slot: 0,
                snake_id: 7,
            }]
        );
        assert_eq!(prepared.world().pellets.len(), 4);
        assert_eq!(prepared.allocators().next_entity_id, 1_004);
        assert_ne!(prepared.rng().baselines, rng.baselines);
        assert_eq!(prepared.rng().world, rng.world);
        assert_eq!(prepared.rng().evolution, rng.evolution);
        assert_eq!(prepared.rng().external_controller, rng.external_controller);
    }

    #[test]
    fn repeated_steps_reuse_all_reported_physics_capacities() {
        let mut source = body_collision_world(SnakeKind::Baseline, false);
        source.pellets.extend((0..32).map(|offset| PelletState {
            id: 100 + offset,
            position: WorldPoint {
                x: 2_000.0 + offset as f64,
                y: 2_000.0,
            },
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        }));
        let rng = rng_bundle(1);
        let allocators = allocators();
        let mut step = PhysicsStepWorkspace::new();
        let mut phases = PhaseWorkspaces::default();
        step.begin(
            key(1),
            &source,
            &rng,
            &allocators,
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        stage_and_accept(&mut step, &mut phases).unwrap();
        step.finish(key(1)).unwrap();
        let mut first_step = None;
        let mut first_substep = None;
        let mut first_pellet_index = None;
        for operation_epoch in 2..=25 {
            step.begin(
                key(operation_epoch),
                &source,
                &rng,
                &allocators,
                PhysicsConfig::typescript_defaults(),
                1,
            )
            .unwrap();
            stage_and_accept(&mut step, &mut phases).unwrap();
            let prepared = step.finish(key(operation_epoch)).unwrap();
            let step_diagnostics = prepared.diagnostics();
            let substep_diagnostics = phases.substep_diagnostics();
            let pellet_index_diagnostics = phases.pellet_index_diagnostics();
            assert_eq!(pellet_index_diagnostics.pellets, 32);
            assert!(pellet_index_diagnostics.estimated_bytes > 0);
            for (capacity, required) in [
                (step_diagnostics.snake_capacity, step_diagnostics.snakes),
                (
                    step_diagnostics.body_point_capacity,
                    step_diagnostics.body_points,
                ),
                (step_diagnostics.pellet_capacity, step_diagnostics.pellets),
                (
                    step_diagnostics.baseline_event_capacity,
                    step_diagnostics.baseline_deaths,
                ),
                (
                    step_diagnostics.controlled_snake_capacity,
                    step_diagnostics.controlled_snakes,
                ),
                (
                    step_diagnostics.validation_order_capacity,
                    step_diagnostics.snakes,
                ),
                (
                    step_diagnostics.validation_range_capacity,
                    step_diagnostics.snakes,
                ),
                (
                    step_diagnostics.pellet_order_capacity,
                    step_diagnostics.pellets,
                ),
            ] {
                assert!(capacity >= required);
            }
            if let Some(expected) = first_step {
                assert_eq!(step_diagnostics, expected);
            } else {
                first_step = Some(step_diagnostics);
            }
            if let Some(expected) = first_substep {
                assert_eq!(substep_diagnostics, expected);
            } else {
                first_substep = Some(substep_diagnostics);
            }
            if let Some(expected) = first_pellet_index {
                assert_eq!(pellet_index_diagnostics, expected);
            } else {
                first_pellet_index = Some(pellet_index_diagnostics);
            }
        }
    }
}
