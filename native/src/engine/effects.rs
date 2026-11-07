//! Deterministic gameplay side-effect realization after immutable collisions.
//!
//! Movement and collision deliberately stage boost-tail requests and deaths
//! without touching authoritative RNG, allocators, or pellets. This phase
//! preflights attempted work and retained storage, reserves one contiguous
//! retained pellet-ID range, then realizes every attempt in stable snake order
//! against the owning gameplay RNG stream. Its output is still scratch: only
//! the later complete physics transaction may publish it.

use super::collision::PreparedCollision;
use super::evolved_lives::{
    EvolvedCorpseMode, EvolvedExperimentConfig, EvolvedGenerationState, PelletOverflowMode,
};

use super::fixed_step::{copy_rng_bundle_reusing, FixedStepPrefixError, RngCopyScratch};
use super::food::FoodConfig;
use super::movement::MovementConfig;
use super::rng::{RngError, StatefulRng};
use super::state::{
    AllocatorState, PelletState, RngStateBundle, SnakeKind, SnakeState, StateError, WorldPoint,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Stable frame-v1 pellet-kind value for large corpse remains.
pub const CORPSE_BIG_PELLET_KIND: u32 = 1;
/// Stable frame-v1 pellet-kind value for small corpse remains.
pub const CORPSE_SMALL_PELLET_KIND: u32 = 2;
/// Stable frame-v1 pellet-kind value for boost trails.
pub const BOOST_PELLET_KIND: u32 = 3;

const MINIMUM_BIG_VALUE: f64 = 0.05;
const MINIMUM_SMALL_VALUE: f64 = 0.02;
const MINIMUM_GROWTH_PER_FOOD: f64 = 1.0e-6;
const MINIMUM_DEATH_PELLET_LIMIT: usize = 20;

/// Borrowed life policy for physical effects; contains no historical-life storage.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EffectLifecyclePolicy<'a> {
    pub experiment: EvolvedExperimentConfig,
    pub lives: &'a EvolvedGenerationState,
}

impl EffectLifecyclePolicy<'_> {
    pub(crate) fn corpse_mode(self, snake: &SnakeState) -> Result<EvolvedCorpseMode, EffectError> {
        if snake.kind != SnakeKind::Evolved || !self.experiment.enabled {
            return Ok(EvolvedCorpseMode::Normal);
        }
        let owner = snake
            .population_slot
            .and_then(|slot| self.lives.slots.get(slot as usize))
            .filter(|owner| owner.physical_id == snake.id)
            .ok_or(EffectError::ShapeMismatch)?;
        Ok(if self.experiment.replaceable(owner.lives_started) {
            self.experiment.corpse_mode
        } else {
            EvolvedCorpseMode::Normal
        })
    }
}

/// Death-to-pellet settings projected from one admitted configuration revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathDropConfig {
    /// Recycled body fraction for a start-length snake.
    pub drop_fraction_small: f64,
    /// Recycled body fraction for a maximum-length snake.
    pub drop_fraction_large: f64,
    /// Size-curve exponent between the small and large fractions.
    pub drop_fraction_power: f64,
    /// Large corpse-pellet value relative to normal food.
    pub big_pellet_value_factor: f64,
    /// Small corpse-pellet value relative to normal food.
    pub small_pellet_value_factor: f64,
    /// Fraction of recycled value assigned to large pellets.
    pub big_share: f64,
    /// Symmetric per-axis large-pellet jitter.
    pub jitter: f64,
    /// Symmetric per-axis small-pellet cluster jitter.
    pub cluster_jitter: f64,
    /// Configured per-corpse pellet ceiling before the TypeScript minimum.
    pub maximum_pellets: usize,
}

impl DeathDropConfig {
    /// Current TypeScript defaults retained as an executable comparison fixture.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            drop_fraction_small: 0.95,
            drop_fraction_large: 0.33,
            drop_fraction_power: 1.6,
            big_pellet_value_factor: 3.0,
            small_pellet_value_factor: 1.0,
            big_share: 0.78,
            jitter: 8.0,
            cluster_jitter: 14.0,
            maximum_pellets: 420,
        }
    }

    pub(crate) fn validate(self) -> Result<(), EffectError> {
        for (field, value) in [
            ("drop_fraction_small", self.drop_fraction_small),
            ("drop_fraction_large", self.drop_fraction_large),
            ("drop_fraction_power", self.drop_fraction_power),
            ("big_pellet_value_factor", self.big_pellet_value_factor),
            ("small_pellet_value_factor", self.small_pellet_value_factor),
            ("big_share", self.big_share),
            ("jitter", self.jitter),
            ("cluster_jitter", self.cluster_jitter),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(EffectError::InvalidConfig { field });
            }
        }
        if self.maximum_pellets == 0 {
            return Err(EffectError::InvalidConfig {
                field: "maximum_pellets",
            });
        }
        Ok(())
    }
}

/// One durable baseline-death notification for the later respawn coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaselineDeathEvent {
    /// Stable baseline slot whose gameplay stream remains reserved.
    pub slot: u32,
    /// Stable dead snake identity.
    pub snake_id: u64,
}

/// Retained sizes and capacities for one prepared effect phase.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectDiagnostics {
    /// All attempted new emissions, including storage discards.
    pub attempted_pellets: usize,
    pub attempted_value: f64,
    pub emitted_value: f64,
    pub discarded_pellets: usize,
    pub discarded_value: f64,
    /// Pellets surviving food claims.
    pub retained_pellets: usize,
    /// Realized boost-trail pellets.
    pub boost_pellets: usize,
    /// Realized large corpse pellets.
    pub corpse_big_pellets: usize,
    /// Realized small corpse pellets.
    pub corpse_small_pellets: usize,
    /// Complete post-effect pellet count.
    pub total_pellets: usize,
    /// Baseline deaths awaiting deterministic respawn handling.
    pub baseline_deaths: usize,
    /// Retained stable snake-order capacity.
    pub snake_order_capacity: usize,
    /// Retained death-plan capacity.
    pub death_plan_capacity: usize,
    /// Retained pellet capacity.
    pub pellet_capacity: usize,
    /// Retained baseline-event capacity.
    pub baseline_event_capacity: usize,
    /// Retained runtime baseline-stream capacity.
    pub baseline_rng_capacity: usize,
}

/// Constant-sized attempted/emitted/discarded totals across a complete fixed step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectEmissionTotals {
    pub attempted_count: usize,
    pub attempted_value: f64,
    pub emitted_count: usize,
    pub emitted_value: f64,
    pub discarded_count: usize,
    pub discarded_value: f64,
}

impl EffectEmissionTotals {
    pub(crate) fn accumulate(&mut self, phase: EffectDiagnostics) -> Result<(), EffectError> {
        self.attempted_count = self
            .attempted_count
            .checked_add(phase.attempted_pellets)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "step attempted emissions",
            })?;
        let emitted = phase
            .attempted_pellets
            .checked_sub(phase.discarded_pellets)
            .ok_or(EffectError::ShapeMismatch)?;
        self.emitted_count =
            self.emitted_count
                .checked_add(emitted)
                .ok_or(EffectError::ArithmeticOverflow {
                    context: "step retained emissions",
                })?;
        self.discarded_count = self
            .discarded_count
            .checked_add(phase.discarded_pellets)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "step discarded emissions",
            })?;
        self.attempted_value += phase.attempted_value;
        self.emitted_value += phase.emitted_value;
        self.discarded_value += phase.discarded_value;
        for (value, field) in [
            (self.attempted_value, "step attempted emission value"),
            (self.emitted_value, "step retained emission value"),
            (self.discarded_value, "step discarded emission value"),
        ] {
            validate_generated(value, field)?;
        }
        Ok(())
    }
}

/// Actual corpse emission attributed to one physical owner before metadata disappears.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorpseEmission {
    pub snake_index: usize,
    pub snake_id: u64,
    pub count: usize,
    pub value: f64,
    pub discarded_count: usize,
    pub discarded_value: f64,
}

/// Immutable view of one completely realized, not-yet-authoritative effect phase.
#[derive(Clone, Copy, Debug)]
pub struct PreparedEffects<'effects, 'collision, 'food, 'world> {
    collision: PreparedCollision<'collision, 'food, 'world>,
    pellets: &'effects [PelletState],
    rng: &'effects RngStateBundle,
    allocators: &'effects AllocatorState,
    baseline_deaths: &'effects [BaselineDeathEvent],
    corpse_emissions: &'effects [CorpseEmission],
    diagnostics: EffectDiagnostics,
}

impl<'effects, 'collision, 'food, 'world> PreparedEffects<'effects, 'collision, 'food, 'world> {
    /// Exact collision snapshot whose requests were realized.
    #[must_use]
    pub const fn collision(self) -> PreparedCollision<'collision, 'food, 'world> {
        self.collision
    }

    /// Complete retained and newly generated pellet state.
    #[must_use]
    pub const fn pellets(self) -> &'effects [PelletState] {
        self.pellets
    }

    /// Staged gameplay RNG continuation; evolution RNG is byte-for-byte unchanged.
    #[must_use]
    pub const fn rng(self) -> &'effects RngStateBundle {
        self.rng
    }

    /// Staged allocator continuation after one atomic pellet-ID reservation.
    #[must_use]
    pub const fn allocators(self) -> &'effects AllocatorState {
        self.allocators
    }

    /// Stable baseline-death notifications for the later coordinator.
    #[must_use]
    pub const fn baseline_deaths(self) -> &'effects [BaselineDeathEvent] {
        self.baseline_deaths
    }

    /// Exact emitted corpse totals, independent of expected recycling budgets.
    #[must_use]
    pub const fn corpse_emissions(self) -> &'effects [CorpseEmission] {
        self.corpse_emissions
    }

    /// Current work sizes and retained capacities.
    #[must_use]
    pub const fn diagnostics(self) -> EffectDiagnostics {
        self.diagnostics
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DeathPlan {
    snake_index: usize,
    big_count: usize,
    small_count: usize,
    value_budget: Option<f64>,
}

/// Actual retained/discarded realization, including every attempted random draw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CorpseRealization {
    big: usize,
    small: usize,
    emitted_value: f64,
    discarded_count: usize,
    discarded_value: f64,
}

#[derive(Debug)]
struct RuntimeRngBundle {
    world: StatefulRng,
    external_controller: StatefulRng,
    baselines: Vec<StatefulRng>,
}

impl RuntimeRngBundle {
    fn from_serialized(
        source: &RngStateBundle,
        baselines: &mut Vec<StatefulRng>,
    ) -> Result<Self, EffectError> {
        let world = StatefulRng::from_state(&source.world).map_err(|source| EffectError::Rng {
            stream: "world".to_owned(),
            source,
        })?;
        let external_controller =
            StatefulRng::from_state(&source.external_controller).map_err(|source| {
                EffectError::Rng {
                    stream: "external-controller".to_owned(),
                    source,
                }
            })?;
        baselines.clear();
        reserve_for(baselines, source.baselines.len(), "baseline RNG streams")?;
        for (index, baseline) in source.baselines.iter().enumerate() {
            let expected = u32::try_from(index).map_err(|_| EffectError::ArithmeticOverflow {
                context: "baseline RNG slot",
            })?;
            if baseline.slot != expected {
                return Err(EffectError::NonDenseBaselineRng {
                    index,
                    slot: baseline.slot,
                });
            }
            baselines.push(StatefulRng::from_state(&baseline.state).map_err(|source| {
                EffectError::Rng {
                    stream: format!("baseline:{}", baseline.slot),
                    source,
                }
            })?);
        }
        Ok(Self {
            world,
            external_controller,
            baselines: std::mem::take(baselines),
        })
    }

    fn stream_for(
        &mut self,
        snake: &SnakeState,
    ) -> Result<(&mut StatefulRng, &'static str), EffectError> {
        match snake.kind {
            SnakeKind::Evolved | SnakeKind::Resurrected => Ok((&mut self.world, "world")),
            SnakeKind::External => Ok((&mut self.external_controller, "external-controller")),
            SnakeKind::Baseline => {
                let slot = snake
                    .baseline_slot
                    .ok_or(EffectError::MissingBaselineSlot(snake.id))?;
                let index = usize::try_from(slot).map_err(|_| EffectError::ArithmeticOverflow {
                    context: "baseline slot index",
                })?;
                let stream = self
                    .baselines
                    .get_mut(index)
                    .ok_or(EffectError::MissingBaselineRng(slot))?;
                Ok((stream, "baseline"))
            }
        }
    }

    fn export_into(
        &self,
        source: &RngStateBundle,
        target: &mut Option<RngStateBundle>,
        scratch: &mut RngCopyScratch,
    ) -> Result<(), EffectError> {
        copy_rng_bundle_reusing(target, scratch, source).map_err(map_rng_copy_error)?;
        let target = target.as_mut().ok_or(EffectError::ShapeMismatch)?;
        if target.baselines.len() != self.baselines.len()
            || scratch.baseline_gaussian_spares.len() < self.baselines.len()
        {
            return Err(EffectError::ShapeMismatch);
        }
        self.world
            .export_state_into_reusing(&mut target.world, &mut scratch.world_gaussian_spare);
        self.external_controller.export_state_into_reusing(
            &mut target.external_controller,
            &mut scratch.external_gaussian_spare,
        );
        for ((destination, runtime), retained_spare) in target
            .baselines
            .iter_mut()
            .zip(&self.baselines)
            .zip(&mut scratch.baseline_gaussian_spares)
        {
            runtime.export_state_into_reusing(&mut destination.state, retained_spare);
        }
        Ok(())
    }

    /// Export a cold-path owned continuation for an ordered operator mutation.
    fn export_owned(&self, source: &RngStateBundle) -> Result<RngStateBundle, EffectError> {
        if source.baselines.len() != self.baselines.len() {
            return Err(EffectError::ShapeMismatch);
        }
        let mut target = source.clone();
        self.world.export_state_into(&mut target.world);
        self.external_controller
            .export_state_into(&mut target.external_controller);
        for (destination, runtime) in target.baselines.iter_mut().zip(&self.baselines) {
            runtime.export_state_into(&mut destination.state);
        }
        Ok(target)
    }
}

/// Complete normal corpse-pellet side effects prepared for one God Mode death.
#[derive(Debug)]
pub(crate) struct PreparedSingleCorpse {
    pub(crate) pellets: Vec<PelletState>,
    pub(crate) rng: RngStateBundle,
    pub(crate) allocators: AllocatorState,
    pub(crate) discarded_count: usize,
    pub(crate) discarded_value: f64,
}

/// Reuse the normal death formula, identity allocator, and owning RNG stream for one corpse.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_single_corpse(
    source_rng: &RngStateBundle,
    source_allocators: &AllocatorState,
    snake: &SnakeState,
    body: &[WorldPoint],
    existing_pellets: usize,
    movement_config: MovementConfig,
    food_config: FoodConfig,
    death_config: DeathDropConfig,
    maximum_pellets: usize,
) -> Result<PreparedSingleCorpse, EffectError> {
    prepare_single_corpse_with_policy(
        source_rng,
        source_allocators,
        snake,
        body,
        existing_pellets,
        movement_config,
        food_config,
        death_config,
        maximum_pellets,
        EvolvedCorpseMode::Normal,
        PelletOverflowMode::Fail,
    )
}

/// Cold-path counterpart of the ordinary retained-only emission transaction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_single_corpse_with_policy(
    source_rng: &RngStateBundle,
    source_allocators: &AllocatorState,
    snake: &SnakeState,
    body: &[WorldPoint],
    existing_pellets: usize,
    movement_config: MovementConfig,
    food_config: FoodConfig,
    death_config: DeathDropConfig,
    maximum_pellets: usize,
    corpse_mode: EvolvedCorpseMode,
    overflow: PelletOverflowMode,
) -> Result<PreparedSingleCorpse, EffectError> {
    movement_config
        .validate()
        .map_err(|_| EffectError::InvalidConfig { field: "movement" })?;
    death_config.validate()?;
    if !food_config.growth_per_food.is_finite() || food_config.growth_per_food < 0.0 {
        return Err(EffectError::InvalidConfig {
            field: "growth_per_food",
        });
    }
    if existing_pellets > maximum_pellets {
        return Err(EffectError::PelletCapacityExceeded {
            required: existing_pellets,
            maximum: maximum_pellets,
        });
    }
    if body.is_empty() {
        return Ok(PreparedSingleCorpse {
            pellets: Vec::new(),
            rng: source_rng.clone(),
            allocators: source_allocators.clone(),
            discarded_count: 0,
            discarded_value: 0.0,
        });
    }

    let plan = death_plan(
        0,
        body.len(),
        movement_config,
        food_config,
        death_config,
        corpse_mode,
    )?;
    let (big_count, small_count) = (plan.big_count, plan.small_count);
    let generated_count =
        big_count
            .checked_add(small_count)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "single corpse pellet count",
            })?;
    let total_pellets =
        existing_pellets
            .checked_add(generated_count)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "single corpse world pellet count",
            })?;
    if total_pellets > maximum_pellets && overflow == PelletOverflowMode::Fail {
        return Err(EffectError::PelletCapacityExceeded {
            required: total_pellets,
            maximum: maximum_pellets,
        });
    }

    let mut pellets = Vec::new();
    let retained_count = generated_count.min(maximum_pellets - existing_pellets);
    reserve_for(&mut pellets, retained_count, "single corpse pellets")?;
    let mut allocators = source_allocators.clone();
    let generated_u64 =
        u64::try_from(retained_count).map_err(|_| EffectError::ArithmeticOverflow {
            context: "single corpse pellet ID count",
        })?;
    let reservation = allocators
        .reserve_entity_ids(generated_u64)
        .map_err(EffectError::Allocator)?;
    let mut baseline_rngs = Vec::new();
    let mut runtime = RuntimeRngBundle::from_serialized(source_rng, &mut baseline_rngs)?;
    let (stream, _) = runtime.stream_for(snake)?;
    let mut next_id = reservation.map_or(source_allocators.next_entity_id, |range| range.first);
    let realized = realize_corpse_pellets(
        &mut pellets,
        &mut next_id,
        snake,
        body,
        plan,
        death_config,
        (movement_config.food_value * death_config.big_pellet_value_factor).max(MINIMUM_BIG_VALUE),
        (movement_config.food_value * death_config.small_pellet_value_factor)
            .max(MINIMUM_SMALL_VALUE),
        stream,
        retained_count,
    )?;
    if realized.big + realized.small != retained_count
        || next_id
            != source_allocators
                .next_entity_id
                .checked_add(generated_u64)
                .ok_or(EffectError::ArithmeticOverflow {
                    context: "single corpse reserved pellet range",
                })?
    {
        return Err(EffectError::ShapeMismatch);
    }
    Ok(PreparedSingleCorpse {
        pellets,
        rng: runtime.export_owned(source_rng)?,
        allocators,
        discarded_count: realized.discarded_count,
        discarded_value: realized.discarded_value,
    })
}

/// Reusable staging storage for deterministic pellet and RNG realization.
#[derive(Debug, Default)]
pub struct EffectWorkspace {
    snake_order: Vec<usize>,
    death_plans: Vec<DeathPlan>,
    corpse_emissions: Vec<CorpseEmission>,
    pellets: Vec<PelletState>,
    baseline_deaths: Vec<BaselineDeathEvent>,
    baseline_rngs: Vec<StatefulRng>,
    staged_rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    staged_allocators: Option<AllocatorState>,
    attempted_pellets: usize,
    attempted_value: f64,
    emitted_value: f64,
    discarded_pellets: usize,
    discarded_value: f64,
    boost_pellets: usize,
    corpse_big_pellets: usize,
    corpse_small_pellets: usize,
    ready: bool,
}

impl EffectWorkspace {
    /// Construct empty reusable side-effect scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Preflight and realize all boost/death requests without mutating authority.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'effects, 'collision, 'food, 'world>(
        &'effects mut self,
        collision: PreparedCollision<'collision, 'food, 'world>,
        rng: &RngStateBundle,
        allocators: &AllocatorState,
        movement_config: MovementConfig,
        food_config: FoodConfig,
        death_config: DeathDropConfig,
        maximum_pellets: usize,
    ) -> Result<PreparedEffects<'effects, 'collision, 'food, 'world>, EffectError> {
        self.prepare_with_lifecycle(
            collision,
            rng,
            allocators,
            movement_config,
            food_config,
            death_config,
            maximum_pellets,
            None,
        )
    }

    /// Realize all attempts in canonical order, allocating identities only for retained pellets.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_with_lifecycle<'effects, 'collision, 'food, 'world>(
        &'effects mut self,
        collision: PreparedCollision<'collision, 'food, 'world>,
        rng: &RngStateBundle,
        allocators: &AllocatorState,
        movement_config: MovementConfig,
        food_config: FoodConfig,
        death_config: DeathDropConfig,
        maximum_pellets: usize,
        lifecycle: Option<EffectLifecyclePolicy<'_>>,
    ) -> Result<PreparedEffects<'effects, 'collision, 'food, 'world>, EffectError> {
        self.clear();
        if let Some(policy) = lifecycle {
            policy
                .experiment
                .validate()
                .map_err(|_| EffectError::InvalidConfig {
                    field: "life policy",
                })?;
        }
        let overflow = lifecycle.map_or(PelletOverflowMode::Fail, |policy| {
            policy.experiment.pellet_overflow
        });
        movement_config
            .validate()
            .map_err(|_| EffectError::InvalidConfig { field: "movement" })?;
        death_config.validate()?;
        if !food_config.growth_per_food.is_finite() || food_config.growth_per_food < 0.0 {
            return Err(EffectError::InvalidConfig {
                field: "growth_per_food",
            });
        }

        let food = collision.food();
        let source = food.source_world();
        if source.snakes.len() != food.snakes().len() {
            return Err(EffectError::ShapeMismatch);
        }
        reserve_for(
            &mut self.snake_order,
            food.snakes().len(),
            "effect snake order",
        )?;
        self.snake_order.extend(0..food.snakes().len());
        self.snake_order
            .sort_unstable_by_key(|index| food.snakes()[*index].id);
        for pair in self.snake_order.windows(2) {
            if food.snakes()[pair[0]].id == food.snakes()[pair[1]].id {
                return Err(EffectError::DuplicateSnakeId(food.snakes()[pair[0]].id));
            }
        }

        reserve_for(
            &mut self.death_plans,
            collision.deaths().len(),
            "death plans",
        )?;
        reserve_for(
            &mut self.baseline_deaths,
            collision.deaths().len(),
            "baseline death events",
        )?;
        let mut generated_count = food.boost_drops().len();
        for death in collision.deaths() {
            let snake = food
                .snakes()
                .get(death.victim_index)
                .ok_or(EffectError::ShapeMismatch)?;
            if snake.id != death.victim_id {
                return Err(EffectError::ShapeMismatch);
            }
            if snake.kind == SnakeKind::Baseline {
                let slot = snake
                    .baseline_slot
                    .ok_or(EffectError::MissingBaselineSlot(snake.id))?;
                self.baseline_deaths.push(BaselineDeathEvent {
                    slot,
                    snake_id: snake.id,
                });
            }
            if !death.drop_corpse_pellets {
                continue;
            }
            let body = food
                .body_for(snake)
                .ok_or(EffectError::InvalidBodyRange { snake_id: snake.id })?;
            if body.is_empty() {
                continue;
            }
            let mode = match lifecycle {
                Some(policy) => policy.corpse_mode(snake)?,
                None => EvolvedCorpseMode::Normal,
            };
            let plan = death_plan(
                death.victim_index,
                body.len(),
                movement_config,
                food_config,
                death_config,
                mode,
            )?;
            let (big_count, small_count) = (plan.big_count, plan.small_count);
            generated_count = generated_count
                .checked_add(big_count)
                .and_then(|value| value.checked_add(small_count))
                .ok_or(EffectError::ArithmeticOverflow {
                    context: "generated pellet count",
                })?;
            self.death_plans.push(plan);
        }
        self.baseline_deaths
            .sort_unstable_by_key(|event| event.snake_id);
        self.death_plans
            .sort_unstable_by_key(|plan| food.snakes()[plan.snake_index].id);

        let total_pellets = food
            .remaining_pellets()
            .len()
            .checked_add(generated_count)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "post-effect pellet count",
            })?;
        if food.remaining_pellets().len() > maximum_pellets
            || (total_pellets > maximum_pellets && overflow == PelletOverflowMode::Fail)
        {
            return Err(EffectError::PelletCapacityExceeded {
                required: total_pellets,
                maximum: maximum_pellets,
            });
        }
        let retained_count = generated_count.min(maximum_pellets - food.remaining_pellets().len());
        let retained_total = food.remaining_pellets().len() + retained_count;
        reserve_for(&mut self.pellets, retained_total, "effect pellets")?;

        let mut staged_allocators = allocators.clone();
        let generated_u64 =
            u64::try_from(retained_count).map_err(|_| EffectError::ArithmeticOverflow {
                context: "generated pellet ID count",
            })?;
        let reservation = staged_allocators
            .reserve_entity_ids(generated_u64)
            .map_err(EffectError::Allocator)?;
        let first_generated_id = reservation.map(|value| value.first);
        let last_generated_id = reservation.map(|value| value.last);

        self.pellets.extend_from_slice(food.remaining_pellets());
        if generated_count == 0 {
            copy_rng_bundle_reusing(&mut self.staged_rng, &mut self.rng_copy_scratch, rng)
                .map_err(map_rng_copy_error)?;
            self.staged_allocators = Some(staged_allocators);
            self.ready = true;
            return self.view(collision);
        }

        let mut runtime = RuntimeRngBundle::from_serialized(rng, &mut self.baseline_rngs)?;
        let mut next_id = first_generated_id.unwrap_or(allocators.next_entity_id);

        for request in food.boost_drops() {
            let snake = find_snake(food.snakes(), &self.snake_order, request.owner_id)?;
            let (stream, _) = runtime.stream_for(snake)?;
            let x = jittered(stream, request.base_position.x, request.jitter);
            let y = jittered(stream, request.base_position.y, request.jitter);
            validate_generated(x, "boost pellet x")?;
            validate_generated(y, "boost pellet y")?;
            validate_positive(request.value, "boost pellet value")?;
            if self.pellets.len() < maximum_pellets {
                self.pellets.push(PelletState {
                    id: next_id,
                    position: WorldPoint { x, y },
                    value: request.value,
                    kind: BOOST_PELLET_KIND,
                    color: snake.frame_v1_id,
                    owner: Some(snake.id),
                });
                next_id = increment_reserved_id(next_id)?;
                self.boost_pellets += 1;
                self.emitted_value += request.value;
            } else {
                self.discarded_pellets += 1;
                self.discarded_value += request.value;
            }
            self.attempted_pellets += 1;
            self.attempted_value += request.value;
        }

        let big_base = (movement_config.food_value * death_config.big_pellet_value_factor)
            .max(MINIMUM_BIG_VALUE);
        let small_base = (movement_config.food_value * death_config.small_pellet_value_factor)
            .max(MINIMUM_SMALL_VALUE);
        validate_positive(big_base, "large corpse pellet base value")?;
        validate_positive(small_base, "small corpse pellet base value")?;
        reserve_for(
            &mut self.corpse_emissions,
            self.death_plans.len(),
            "corpse emission totals",
        )?;
        for plan in &self.death_plans {
            let pellet_start = self.pellets.len();
            let snake = &food.snakes()[plan.snake_index];
            let body = food
                .body_for(snake)
                .ok_or(EffectError::InvalidBodyRange { snake_id: snake.id })?;
            let (stream, _) = runtime.stream_for(snake)?;
            let realized = realize_corpse_pellets(
                &mut self.pellets,
                &mut next_id,
                snake,
                body,
                *plan,
                death_config,
                big_base,
                small_base,
                stream,
                maximum_pellets,
            )?;
            self.corpse_emissions.push(CorpseEmission {
                snake_index: plan.snake_index,
                snake_id: snake.id,
                count: self.pellets.len() - pellet_start,
                value: realized.emitted_value,
                discarded_count: realized.discarded_count,
                discarded_value: realized.discarded_value,
            });
            self.emitted_value += realized.emitted_value;
            self.corpse_big_pellets += realized.big;
            self.corpse_small_pellets += realized.small;
            self.discarded_pellets += realized.discarded_count;
            self.discarded_value += realized.discarded_value;
            self.attempted_pellets += plan.big_count + plan.small_count;
            self.attempted_value += realized.emitted_value + realized.discarded_value;
        }
        debug_assert_eq!(self.pellets.len(), retained_total);
        let expected_next_id = match last_generated_id {
            Some(last) => last.checked_add(1).ok_or(EffectError::ArithmeticOverflow {
                context: "reserved pellet range end",
            })?,
            None => allocators.next_entity_id,
        };
        if next_id != expected_next_id {
            return Err(EffectError::ShapeMismatch);
        }
        validate_generated(self.emitted_value, "emitted effect value")?;
        validate_generated(self.attempted_value, "attempted emission value")?;
        validate_generated(self.discarded_value, "discarded emission value")?;
        runtime.export_into(rng, &mut self.staged_rng, &mut self.rng_copy_scratch)?;
        self.baseline_rngs = runtime.baselines;
        self.staged_allocators = Some(staged_allocators);
        self.ready = true;
        self.view(collision)
    }

    fn view<'effects, 'collision, 'food, 'world>(
        &'effects self,
        collision: PreparedCollision<'collision, 'food, 'world>,
    ) -> Result<PreparedEffects<'effects, 'collision, 'food, 'world>, EffectError> {
        let rng = self.staged_rng.as_ref().ok_or(EffectError::ShapeMismatch)?;
        let allocators = self
            .staged_allocators
            .as_ref()
            .ok_or(EffectError::ShapeMismatch)?;
        Ok(PreparedEffects {
            collision,
            pellets: &self.pellets,
            rng,
            allocators,
            baseline_deaths: &self.baseline_deaths,
            corpse_emissions: &self.corpse_emissions,
            diagnostics: self.diagnostics(),
        })
    }

    /// Current sizes and retained capacities, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> EffectDiagnostics {
        let retained_pellets = self
            .pellets
            .len()
            .saturating_sub(self.boost_pellets)
            .saturating_sub(self.corpse_big_pellets)
            .saturating_sub(self.corpse_small_pellets);
        EffectDiagnostics {
            attempted_pellets: self.attempted_pellets,
            attempted_value: self.attempted_value,
            emitted_value: self.emitted_value,
            discarded_pellets: self.discarded_pellets,
            discarded_value: self.discarded_value,
            retained_pellets,
            boost_pellets: self.boost_pellets,
            corpse_big_pellets: self.corpse_big_pellets,
            corpse_small_pellets: self.corpse_small_pellets,
            total_pellets: self.pellets.len(),
            baseline_deaths: self.baseline_deaths.len(),
            snake_order_capacity: self.snake_order.capacity(),
            death_plan_capacity: self.death_plans.capacity(),
            pellet_capacity: self.pellets.capacity(),
            baseline_event_capacity: self.baseline_deaths.capacity(),
            baseline_rng_capacity: self.baseline_rngs.capacity(),
        }
    }

    /// Whether the latest preparation completed every staged effect.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    fn clear(&mut self) {
        self.ready = false;
        self.snake_order.clear();
        self.death_plans.clear();
        self.corpse_emissions.clear();
        self.pellets.clear();
        self.baseline_deaths.clear();
        self.baseline_rngs.clear();
        self.attempted_pellets = 0;
        self.attempted_value = 0.0;
        self.emitted_value = 0.0;
        self.discarded_pellets = 0;
        self.discarded_value = 0.0;
        self.boost_pellets = 0;
        self.corpse_big_pellets = 0;
        self.corpse_small_pellets = 0;
    }
}

/// Choose the frozen formula or replacement-mass compensation before drawing.
fn death_plan(
    snake_index: usize,
    body_length: usize,
    movement: MovementConfig,
    food: FoodConfig,
    death: DeathDropConfig,
    mode: EvolvedCorpseMode,
) -> Result<DeathPlan, EffectError> {
    let (big_count, small_count, value_budget) = match mode {
        EvolvedCorpseMode::Normal => {
            let (big, small) = death_pellet_counts(body_length, movement, food, death)?;
            (big, small, None)
        }
        EvolvedCorpseMode::None => (0, 0, None),
        EvolvedCorpseMode::SubtractReplacementMass => {
            let denominator = movement
                .snake_max_len
                .saturating_sub(movement.snake_start_len)
                .max(1) as f64;
            let normalized = ((body_length as f64 - movement.snake_start_len as f64) / denominator)
                .clamp(0.0, 1.0);
            let fraction = (death.drop_fraction_small
                - (death.drop_fraction_small - death.drop_fraction_large)
                    * normalized.powf(death.drop_fraction_power))
            .clamp(0.0, 1.0);
            let budget = (body_length as f64 * fraction - movement.snake_start_len as f64).max(0.0)
                / food.growth_per_food.max(MINIMUM_GROWTH_PER_FOOD);
            validate_generated(budget, "compensated corpse budget")?;
            let big_base =
                (movement.food_value * death.big_pellet_value_factor).max(MINIMUM_BIG_VALUE);
            let small_base =
                (movement.food_value * death.small_pellet_value_factor).max(MINIMUM_SMALL_VALUE);
            let big_budget = budget * death.big_share.clamp(0.0, 1.0);
            let mut big = floor_to_usize(big_budget / big_base, "compensated big count")?;
            let mut small = floor_to_usize(
                (budget - big_budget).max(0.0) / small_base,
                "compensated small count",
            )?;
            let total = big
                .checked_add(small)
                .ok_or(EffectError::ArithmeticOverflow {
                    context: "compensated corpse count",
                })?;
            let limit = death.maximum_pellets.max(MINIMUM_DEATH_PELLET_LIMIT);
            if total > limit {
                let scale = limit as f64 / total as f64;
                big = floor_to_usize(big as f64 * scale, "compensated scaled big count")?;
                small = floor_to_usize(small as f64 * scale, "compensated scaled small count")?;
            }
            (big, small, Some(budget))
        }
    };
    Ok(DeathPlan {
        snake_index,
        big_count,
        small_count,
        value_budget,
    })
}

/// Apply the compensation ceiling without changing frozen normal arithmetic.
fn budgeted_value(raw: f64, scale: f64, remaining: &mut Option<f64>) -> f64 {
    match remaining {
        Some(budget) => {
            let value = (raw * scale).min(*budget);
            *budget = (*budget - value).max(0.0);
            value
        }
        None => raw,
    }
}

fn death_pellet_counts(
    body_length: usize,
    movement: MovementConfig,
    food: FoodConfig,
    death: DeathDropConfig,
) -> Result<(usize, usize), EffectError> {
    let denominator = movement
        .snake_max_len
        .saturating_sub(movement.snake_start_len)
        .max(1) as f64;
    let normalized =
        ((body_length as f64 - movement.snake_start_len as f64) / denominator).clamp(0.0, 1.0);
    let fraction = (death.drop_fraction_small
        - (death.drop_fraction_small - death.drop_fraction_large)
            * normalized.powf(death.drop_fraction_power))
    .clamp(0.0, 1.0);
    let drop_length = (body_length as f64 * fraction).max(0.0);
    let total_value = drop_length / food.growth_per_food.max(MINIMUM_GROWTH_PER_FOOD);
    let big_base = (movement.food_value * death.big_pellet_value_factor).max(MINIMUM_BIG_VALUE);
    let small_base =
        (movement.food_value * death.small_pellet_value_factor).max(MINIMUM_SMALL_VALUE);
    for (field, value) in [
        ("death fraction", fraction),
        ("death total value", total_value),
        ("large corpse value", big_base),
        ("small corpse value", small_base),
    ] {
        validate_generated(value, field)?;
    }
    let big_share = death.big_share.clamp(0.0, 1.0);
    let big_budget = total_value * big_share;
    let small_budget = (total_value - big_budget).max(0.0);
    let mut big_count = floor_to_usize(big_budget / big_base, "large corpse pellet count")?.max(1);
    let mut small_count = floor_to_usize(small_budget / small_base, "small corpse pellet count")?;
    let configured_limit = death.maximum_pellets.max(MINIMUM_DEATH_PELLET_LIMIT);
    let total_count =
        big_count
            .checked_add(small_count)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "per-corpse pellet count",
            })?;
    if total_count > configured_limit {
        let scale = configured_limit as f64 / total_count as f64;
        big_count = floor_to_usize(big_count as f64 * scale, "scaled large pellet count")?.max(1);
        small_count = floor_to_usize(small_count as f64 * scale, "scaled small pellet count")?;
    }
    Ok((big_count, small_count))
}

fn find_snake<'a>(
    snakes: &'a [SnakeState],
    order: &[usize],
    id: u64,
) -> Result<&'a SnakeState, EffectError> {
    let position = order
        .binary_search_by_key(&id, |index| snakes[*index].id)
        .map_err(|_| EffectError::UnknownSnake(id))?;
    snakes
        .get(order[position])
        .ok_or(EffectError::ShapeMismatch)
}

fn jittered(rng: &mut StatefulRng, center: f64, radius: f64) -> f64 {
    center - radius + rng.next_f64() * (radius * 2.0)
}

fn push_corpse_pellet(
    pellets: &mut Vec<PelletState>,
    id: u64,
    snake: &SnakeState,
    position: WorldPoint,
    value: f64,
    kind: u32,
) -> Result<(), EffectError> {
    validate_generated(position.x, "corpse pellet x")?;
    validate_generated(position.y, "corpse pellet y")?;
    validate_positive(value, "corpse pellet value")?;
    pellets.push(PelletState {
        id,
        position,
        value,
        kind,
        color: snake.frame_v1_id,
        owner: Some(snake.id),
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn realize_corpse_pellets(
    pellets: &mut Vec<PelletState>,
    next_id: &mut u64,
    snake: &SnakeState,
    body: &[WorldPoint],
    plan: DeathPlan,
    config: DeathDropConfig,
    big_base: f64,
    small_base: f64,
    rng: &mut StatefulRng,
    maximum_pellets: usize,
) -> Result<CorpseRealization, EffectError> {
    let mut realized = CorpseRealization::default();
    let upper_value =
        plan.big_count as f64 * big_base * 1.15 + plan.small_count as f64 * small_base * 1.20;
    let scale = plan.value_budget.map_or(1.0, |budget| {
        if upper_value > budget {
            budget / upper_value
        } else {
            1.0
        }
    });
    let mut remaining_budget = plan.value_budget;
    for index in 0..plan.big_count {
        let body_index = if plan.big_count <= 1 {
            0
        } else {
            index
                .checked_mul(body.len() - 1)
                .and_then(|value| value.checked_div(plan.big_count - 1))
                .ok_or(EffectError::ArithmeticOverflow {
                    context: "large corpse body index",
                })?
        };
        let point = body[body_index];
        let raw_value = big_base * (0.85 + rng.next_f64() * 0.30);
        let value = budgeted_value(raw_value, scale, &mut remaining_budget);
        let x = jittered(rng, point.x, config.jitter);
        let y = jittered(rng, point.y, config.jitter);
        validate_generated(x, "corpse pellet x")?;
        validate_generated(y, "corpse pellet y")?;
        validate_positive(value, "corpse pellet value")?;
        if pellets.len() < maximum_pellets {
            push_corpse_pellet(
                pellets,
                *next_id,
                snake,
                WorldPoint { x, y },
                value,
                CORPSE_BIG_PELLET_KIND,
            )?;
            *next_id = increment_reserved_id(*next_id)?;
            realized.big += 1;
            realized.emitted_value += value;
        } else {
            realized.discarded_count += 1;
            realized.discarded_value += value;
        }
    }
    for index in 0..plan.small_count {
        let body_index = index
            .checked_mul(body.len() - 1)
            .and_then(|value| value.checked_div(plan.small_count.max(1)))
            .ok_or(EffectError::ArithmeticOverflow {
                context: "small corpse body index",
            })?;
        let point = body[body_index];
        let raw_value = small_base * (0.80 + rng.next_f64() * 0.40);
        let value = budgeted_value(raw_value, scale, &mut remaining_budget);
        let x = jittered(rng, point.x, config.cluster_jitter);
        let y = jittered(rng, point.y, config.cluster_jitter);
        validate_generated(x, "corpse pellet x")?;
        validate_generated(y, "corpse pellet y")?;
        validate_positive(value, "corpse pellet value")?;
        if pellets.len() < maximum_pellets {
            push_corpse_pellet(
                pellets,
                *next_id,
                snake,
                WorldPoint { x, y },
                value,
                CORPSE_SMALL_PELLET_KIND,
            )?;
            *next_id = increment_reserved_id(*next_id)?;
            realized.small += 1;
            realized.emitted_value += value;
        } else {
            realized.discarded_count += 1;
            realized.discarded_value += value;
        }
    }
    validate_generated(realized.emitted_value, "emitted corpse value")?;
    validate_generated(realized.discarded_value, "discarded corpse value")?;
    Ok(realized)
}

fn increment_reserved_id(current: u64) -> Result<u64, EffectError> {
    current
        .checked_add(1)
        .ok_or(EffectError::ArithmeticOverflow {
            context: "reserved pellet ID",
        })
}

fn floor_to_usize(value: f64, context: &'static str) -> Result<usize, EffectError> {
    if !value.is_finite() || value < 0.0 || value.floor() > usize::MAX as f64 {
        return Err(EffectError::ArithmeticOverflow { context });
    }
    Ok(value.floor() as usize)
}

fn validate_generated(value: f64, field: &'static str) -> Result<(), EffectError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(EffectError::NonFiniteGenerated { field })
    }
}

fn validate_positive(value: f64, field: &'static str) -> Result<(), EffectError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(EffectError::InvalidGenerated { field })
    }
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), EffectError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| EffectError::AllocationFailed { context, required })?;
    }
    Ok(())
}

fn map_rng_copy_error(error: FixedStepPrefixError) -> EffectError {
    match error {
        FixedStepPrefixError::AllocationFailed { context, required } => {
            EffectError::AllocationFailed { context, required }
        }
        FixedStepPrefixError::ArithmeticOverflow { context } => {
            EffectError::ArithmeticOverflow { context }
        }
        _ => EffectError::ShapeMismatch,
    }
}

/// Checked effect-staging failure. No variant publishes partial authority.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectError {
    /// A projected configuration value is unsupported.
    InvalidConfig { field: &'static str },
    /// Input phase shapes or identities disagree.
    ShapeMismatch,
    /// A stable snake identity appeared more than once.
    DuplicateSnakeId(u64),
    /// A staged request references no snake in the bound snapshot.
    UnknownSnake(u64),
    /// A body range cannot be resolved in the bound packed storage.
    InvalidBodyRange { snake_id: u64 },
    /// A baseline snake has no stable baseline slot.
    MissingBaselineSlot(u64),
    /// A baseline slot has no matching retained RNG stream.
    MissingBaselineRng(u32),
    /// Baseline RNG records are not dense and ordered.
    NonDenseBaselineRng { index: usize, slot: u32 },
    /// A serialized gameplay RNG continuation is invalid.
    Rng { stream: String, source: RngError },
    /// Monotonic pellet-ID reservation failed atomically.
    Allocator(StateError),
    /// The complete resulting pellet set exceeds the admitted capacity.
    PelletCapacityExceeded { required: usize, maximum: usize },
    /// Count or identity arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// A generated scalar was NaN or infinite.
    NonFiniteGenerated { field: &'static str },
    /// A generated positive scalar was zero or negative.
    InvalidGenerated { field: &'static str },
    /// Scratch reservation failed before authority publication.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
}

impl Display for EffectError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => write!(formatter, "invalid effect config: {field}"),
            Self::ShapeMismatch => write!(formatter, "effect input phases disagree on shape"),
            Self::DuplicateSnakeId(id) => write!(formatter, "duplicate effect snake ID {id}"),
            Self::UnknownSnake(id) => {
                write!(formatter, "effect request references unknown snake {id}")
            }
            Self::InvalidBodyRange { snake_id } => {
                write!(
                    formatter,
                    "snake {snake_id} has an invalid effect body range"
                )
            }
            Self::MissingBaselineSlot(id) => {
                write!(formatter, "baseline snake {id} has no stable baseline slot")
            }
            Self::MissingBaselineRng(slot) => {
                write!(formatter, "baseline slot {slot} has no gameplay RNG stream")
            }
            Self::NonDenseBaselineRng { index, slot } => write!(
                formatter,
                "baseline RNG index {index} contains non-dense slot {slot}"
            ),
            Self::Rng { stream, source } => {
                write!(formatter, "invalid {stream} gameplay RNG: {source}")
            }
            Self::Allocator(source) => write!(formatter, "pellet allocator failed: {source}"),
            Self::PelletCapacityExceeded { required, maximum } => write!(
                formatter,
                "effects need {required} pellets, exceeding maximum {maximum}"
            ),
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "checked arithmetic overflow while calculating {context}"
                )
            }
            Self::NonFiniteGenerated { field } => {
                write!(formatter, "generated {field} must be finite")
            }
            Self::InvalidGenerated { field } => {
                write!(formatter, "generated {field} must be finite and positive")
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "effects could not reserve {required} entries for {context}"
            ),
        }
    }
}

impl Error for EffectError {}

#[cfg(test)]
mod tests;
