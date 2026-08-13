//! Deterministic gameplay side-effect realization after immutable collisions.
//!
//! Movement and collision deliberately stage boost-tail requests and deaths
//! without touching authoritative RNG, allocators, or pellets. This phase
//! preflights their complete size and identity requirements, reserves one
//! contiguous pellet-ID range, then realizes requests in stable snake order
//! against the owning gameplay RNG stream. Its output is still scratch: only
//! the later complete physics transaction may publish it.

use super::collision::PreparedCollision;
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

    fn validate(self) -> Result<(), EffectError> {
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectDiagnostics {
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

/// Immutable view of one completely realized, not-yet-authoritative effect phase.
#[derive(Clone, Copy, Debug)]
pub struct PreparedEffects<'effects, 'collision, 'food, 'world> {
    collision: PreparedCollision<'collision, 'food, 'world>,
    pellets: &'effects [PelletState],
    rng: &'effects RngStateBundle,
    allocators: &'effects AllocatorState,
    baseline_deaths: &'effects [BaselineDeathEvent],
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

    /// Current work sizes and retained capacities.
    #[must_use]
    pub const fn diagnostics(self) -> EffectDiagnostics {
        self.diagnostics
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeathPlan {
    snake_index: usize,
    big_count: usize,
    small_count: usize,
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

    fn export_into(&self, source: &RngStateBundle) -> RngStateBundle {
        let mut next = source.clone();
        next.world = self.world.export_state();
        next.external_controller = self.external_controller.export_state();
        for (destination, runtime) in next.baselines.iter_mut().zip(&self.baselines) {
            destination.state = runtime.export_state();
        }
        next
    }
}

/// Reusable staging storage for deterministic pellet and RNG realization.
#[derive(Debug, Default)]
pub struct EffectWorkspace {
    snake_order: Vec<usize>,
    death_plans: Vec<DeathPlan>,
    pellets: Vec<PelletState>,
    baseline_deaths: Vec<BaselineDeathEvent>,
    baseline_rngs: Vec<StatefulRng>,
    staged_rng: Option<RngStateBundle>,
    staged_allocators: Option<AllocatorState>,
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
        self.clear();
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
            let (big_count, small_count) =
                death_pellet_counts(body.len(), movement_config, food_config, death_config)?;
            generated_count = generated_count
                .checked_add(big_count)
                .and_then(|value| value.checked_add(small_count))
                .ok_or(EffectError::ArithmeticOverflow {
                    context: "generated pellet count",
                })?;
            self.death_plans.push(DeathPlan {
                snake_index: death.victim_index,
                big_count,
                small_count,
            });
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
        if total_pellets > maximum_pellets {
            return Err(EffectError::PelletCapacityExceeded {
                required: total_pellets,
                maximum: maximum_pellets,
            });
        }
        reserve_for(&mut self.pellets, total_pellets, "effect pellets")?;

        let mut staged_allocators = allocators.clone();
        let generated_u64 =
            u64::try_from(generated_count).map_err(|_| EffectError::ArithmeticOverflow {
                context: "generated pellet ID count",
            })?;
        let reservation = staged_allocators
            .reserve_entity_ids(generated_u64)
            .map_err(EffectError::Allocator)?;
        let first_generated_id = reservation.map(|value| value.first);
        let last_generated_id = reservation.map(|value| value.last);

        self.pellets.extend_from_slice(food.remaining_pellets());
        if generated_count == 0 {
            self.staged_rng = Some(rng.clone());
            self.staged_allocators = Some(staged_allocators);
            self.ready = true;
            return self.view(collision);
        }

        let mut runtime = RuntimeRngBundle::from_serialized(rng, &mut self.baseline_rngs)?;
        let mut next_id = first_generated_id.ok_or(EffectError::ShapeMismatch)?;

        for request in food.boost_drops() {
            let snake = find_snake(food.snakes(), &self.snake_order, request.owner_id)?;
            let (stream, _) = runtime.stream_for(snake)?;
            let x = jittered(stream, request.base_position.x, request.jitter);
            let y = jittered(stream, request.base_position.y, request.jitter);
            validate_generated(x, "boost pellet x")?;
            validate_generated(y, "boost pellet y")?;
            validate_positive(request.value, "boost pellet value")?;
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
        }

        let big_base = (movement_config.food_value * death_config.big_pellet_value_factor)
            .max(MINIMUM_BIG_VALUE);
        let small_base = (movement_config.food_value * death_config.small_pellet_value_factor)
            .max(MINIMUM_SMALL_VALUE);
        validate_positive(big_base, "large corpse pellet base value")?;
        validate_positive(small_base, "small corpse pellet base value")?;
        for plan in &self.death_plans {
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
            )?;
            self.corpse_big_pellets += realized.0;
            self.corpse_small_pellets += realized.1;
        }
        debug_assert_eq!(self.pellets.len(), total_pellets);
        let expected_next_id = last_generated_id
            .ok_or(EffectError::ShapeMismatch)?
            .checked_add(1)
            .ok_or(EffectError::ArithmeticOverflow {
                context: "reserved pellet range end",
            })?;
        if next_id != expected_next_id {
            return Err(EffectError::ShapeMismatch);
        }
        self.staged_rng = Some(runtime.export_into(rng));
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
        self.pellets.clear();
        self.baseline_deaths.clear();
        self.baseline_rngs.clear();
        self.staged_rng = None;
        self.staged_allocators = None;
        self.boost_pellets = 0;
        self.corpse_big_pellets = 0;
        self.corpse_small_pellets = 0;
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
) -> Result<(usize, usize), EffectError> {
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
        let value = big_base * (0.85 + rng.next_f64() * 0.30);
        let x = jittered(rng, point.x, config.jitter);
        let y = jittered(rng, point.y, config.jitter);
        push_corpse_pellet(
            pellets,
            *next_id,
            snake,
            WorldPoint { x, y },
            value,
            CORPSE_BIG_PELLET_KIND,
        )?;
        *next_id = increment_reserved_id(*next_id)?;
    }
    for index in 0..plan.small_count {
        let body_index = index
            .checked_mul(body.len() - 1)
            .and_then(|value| value.checked_div(plan.small_count.max(1)))
            .ok_or(EffectError::ArithmeticOverflow {
                context: "small corpse body index",
            })?;
        let point = body[body_index];
        let value = small_base * (0.80 + rng.next_f64() * 0.40);
        let x = jittered(rng, point.x, config.cluster_jitter);
        let y = jittered(rng, point.y, config.cluster_jitter);
        push_corpse_pellet(
            pellets,
            *next_id,
            snake,
            WorldPoint { x, y },
            value,
            CORPSE_SMALL_PELLET_KIND,
        )?;
        *next_id = increment_reserved_id(*next_id)?;
    }
    Ok((plan.big_count, plan.small_count))
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
mod tests {
    use super::*;
    use crate::engine::collision::{CollisionConfig, CollisionWorkspace};
    use crate::engine::food::FoodWorkspace;
    use crate::engine::movement::MovementWorkspace;
    use crate::engine::rng::{
        SerializedRngState, LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM,
        LEGACY_TYPESCRIPT_GAUSSIAN_VERSION, RNG_ALGORITHM, RNG_VERSION,
    };
    use crate::engine::spatial::{IndexedSensorWorld, SensorIndexConfig};
    use crate::engine::state::{
        BaselineRngState, BodyRange, WorldState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START,
        EXTERNAL_ENTITY_ID_START, RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION,
    };

    const DT: f64 = 1.0 / 180.0;

    #[derive(Clone, Debug, PartialEq)]
    struct EffectResult {
        pellets: Vec<PelletState>,
        rng: RngStateBundle,
        allocators: AllocatorState,
        baseline_deaths: Vec<BaselineDeathEvent>,
        diagnostics: EffectDiagnostics,
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
            frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
            kind,
            alive: true,
            population_slot: (kind == SnakeKind::Evolved).then_some(0),
            brain: None,
            baseline_slot: (kind == SnakeKind::Baseline).then_some(0),
            baseline_strategy: None,
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

    fn rng_bundle() -> RngStateBundle {
        RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: StatefulRng::new(11.0).export_state(),
            evolution: StatefulRng::new(22.0).export_state(),
            external_controller: StatefulRng::new(33.0).export_state(),
            baselines: vec![BaselineRngState {
                slot: 0,
                state: StatefulRng::new(44.0).export_state(),
            }],
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

    fn indexed(world: &WorldState) -> IndexedSensorWorld<'_> {
        IndexedSensorWorld::build(
            world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 1_000_000,
            },
        )
        .expect("fixture world should index")
    }

    fn execute(
        world: &WorldState,
        rng: &RngStateBundle,
        allocators: &AllocatorState,
        maximum_pellets: usize,
    ) -> Result<EffectResult, EffectError> {
        let movement_config = MovementConfig::typescript_defaults();
        let indexed = indexed(world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(world, movement_config, DT, 100_000, 100_000)
            .expect("fixture movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("fixture food should prepare");
        let mut collision_workspace = CollisionWorkspace::new();
        let collision = collision_workspace
            .prepare(food, CollisionConfig::typescript_defaults())
            .expect("fixture collision should prepare");
        let mut effects_workspace = EffectWorkspace::new();
        let prepared = effects_workspace.prepare(
            collision,
            rng,
            allocators,
            movement_config,
            FoodConfig::typescript_defaults(),
            DeathDropConfig::typescript_defaults(),
            maximum_pellets,
        )?;
        Ok(EffectResult {
            pellets: prepared.pellets().to_vec(),
            rng: prepared.rng().clone(),
            allocators: prepared.allocators().clone(),
            baseline_deaths: prepared.baseline_deaths().to_vec(),
            diagnostics: prepared.diagnostics(),
        })
    }

    fn close(actual: f64, expected: f64) {
        let tolerance = 1.0e-12_f64.max(expected.abs() * 1.0e-12);
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn matches_retained_typescript_death_formula_rng_and_ordering() {
        let legacy = SerializedRngState {
            algorithm: RNG_ALGORITHM.to_owned(),
            version: RNG_VERSION,
            state_hex: "0x25e2bbed".to_owned(),
            gaussian_algorithm: LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM.to_owned(),
            gaussian_version: LEGACY_TYPESCRIPT_GAUSSIAN_VERSION,
            gaussian_spare_valid: false,
            gaussian_spare_hex: None,
        };
        let mut rng = StatefulRng::from_legacy_typescript_state(&legacy)
            .expect("retained legacy uniform continuation should decode");
        let mut fixture_snake = snake(
            7,
            SnakeKind::Evolved,
            WorldPoint { x: 0.0, y: 0.0 },
            0.0,
            12,
        );
        fixture_snake.frame_v1_id = 7;
        let body: Vec<WorldPoint> = (0..12)
            .map(|index| WorldPoint {
                x: -(index as f64) * 7.5,
                y: if index % 2 == 0 { 0.0 } else { 1.25 },
            })
            .collect();
        let movement = MovementConfig::typescript_defaults();
        let food = FoodConfig::typescript_defaults();
        let death = DeathDropConfig::typescript_defaults();
        let (big_count, small_count) =
            death_pellet_counts(body.len(), movement, food, death).expect("valid fixture counts");
        assert_eq!((big_count, small_count), (2, 2));
        let mut pellets = Vec::new();
        let mut next_id = 1_000;
        realize_corpse_pellets(
            &mut pellets,
            &mut next_id,
            &fixture_snake,
            &body,
            DeathPlan {
                snake_index: 0,
                big_count,
                small_count,
            },
            death,
            3.0,
            1.0,
            &mut rng,
        )
        .expect("fixture death should realize");

        let expected = [
            (3.7043982185423374, 2.57913351804018, 2.6677111503202466, 1),
            (-75.60867307335138, 8.878263261169195, 3.0139878390356896, 1),
            (6.539477038197219, 12.556274109520018, 0.9609815296716988, 2),
            (
                -42.59541824180633,
                0.4477313784882426,
                0.9449886957183481,
                2,
            ),
        ];
        assert_eq!(pellets.len(), expected.len());
        for (pellet, expected) in pellets.iter().zip(expected) {
            close(pellet.position.x, expected.0);
            close(pellet.position.y, expected.1);
            close(pellet.value, expected.2);
            assert_eq!(pellet.kind, expected.3);
            assert_eq!(pellet.color, 7);
            assert_eq!(pellet.owner, Some(7));
        }
        assert_eq!(rng.export_state().state_hex, "0x78aa3ba5");
    }

    #[test]
    fn gameplay_rng_streams_are_isolated_and_evolution_never_advances() {
        for kind in [
            SnakeKind::Evolved,
            SnakeKind::Resurrected,
            SnakeKind::External,
            SnakeKind::Baseline,
        ] {
            let world = body_collision_world(kind, false);
            let before_rng = rng_bundle();
            let before_world = world.clone();
            let before_allocators = allocators();
            let result = execute(&world, &before_rng, &before_allocators, 100_000)
                .expect("body death should realize effects");
            assert_eq!(
                world, before_world,
                "effect staging mutated source authority"
            );
            assert_eq!(before_allocators.next_entity_id, 1_000);
            assert_eq!(result.rng.evolution, before_rng.evolution);
            assert_eq!(
                result.rng.world != before_rng.world,
                matches!(kind, SnakeKind::Evolved | SnakeKind::Resurrected)
            );
            assert_eq!(
                result.rng.external_controller != before_rng.external_controller,
                kind == SnakeKind::External
            );
            assert_eq!(
                result.rng.baselines[0].state != before_rng.baselines[0].state,
                kind == SnakeKind::Baseline
            );
            assert_eq!(
                result.baseline_deaths,
                if kind == SnakeKind::Baseline {
                    vec![BaselineDeathEvent {
                        slot: 0,
                        snake_id: 7,
                    }]
                } else {
                    Vec::new()
                }
            );
            assert!(result.diagnostics.corpse_big_pellets > 0);
            assert_eq!(
                result.allocators.next_entity_id,
                1_000 + result.pellets.len() as u64
            );
        }
    }

    #[test]
    fn boost_request_uses_two_owner_stream_draws_and_preserves_other_streams() {
        let position = WorldPoint { x: 0.0, y: 0.0 };
        let mut external = snake(7, SnakeKind::External, position, 0.0, 8);
        external.input_boost = true;
        let world = pack_world(vec![(external, line_body(position, 0.0, 8))]);
        let before = rng_bundle();
        let allocators = allocators();
        let result = execute(&world, &before, &allocators, 100_000)
            .expect("one boost-tail request should realize");

        assert_eq!(result.pellets.len(), 1);
        let pellet = &result.pellets[0];
        let mut expected_rng = StatefulRng::from_state(&before.external_controller)
            .expect("fixture external stream should restore");
        let expected_x = -60.5 - 10.0 + expected_rng.next_f64() * 20.0;
        let expected_y = -10.0 + expected_rng.next_f64() * 20.0;
        close(pellet.position.x, expected_x);
        close(pellet.position.y, expected_y);
        close(pellet.value, 0.65);
        assert_eq!(pellet.kind, BOOST_PELLET_KIND);
        assert_eq!(pellet.color, 7);
        assert_eq!(pellet.owner, Some(7));
        assert_eq!(result.rng.external_controller, expected_rng.export_state());
        assert_eq!(result.rng.world, before.world);
        assert_eq!(result.rng.evolution, before.evolution);
        assert_eq!(result.rng.baselines, before.baselines);
        assert_eq!(result.allocators.next_entity_id, 1_001);
        assert_eq!(result.diagnostics.boost_pellets, 1);
    }

    #[test]
    fn stable_snake_order_makes_container_order_irrelevant() {
        let rng = rng_bundle();
        let allocators = allocators();
        let forward = execute(
            &body_collision_world(SnakeKind::External, false),
            &rng,
            &allocators,
            100_000,
        )
        .expect("forward effects");
        let reversed = execute(
            &body_collision_world(SnakeKind::External, true),
            &rng,
            &allocators,
            100_000,
        )
        .expect("reversed effects");
        assert_eq!(forward, reversed);
    }

    #[test]
    fn two_same_stream_deaths_use_stable_id_draw_and_pellet_order() {
        fn world(reverse: bool) -> WorldState {
            let entries = vec![
                (
                    snake(
                        7,
                        SnakeKind::Evolved,
                        WorldPoint { x: 0.0, y: 0.0 },
                        0.0,
                        12,
                    ),
                    line_body(WorldPoint { x: 0.0, y: 0.0 }, 0.0, 12),
                ),
                (
                    snake(
                        20,
                        SnakeKind::Evolved,
                        WorldPoint { x: 100.0, y: 0.0 },
                        0.0,
                        15,
                    ),
                    line_body(WorldPoint { x: 100.0, y: 0.0 }, 0.0, 15),
                ),
                (
                    snake(
                        8,
                        SnakeKind::Evolved,
                        WorldPoint { x: 0.0, y: 40.0 },
                        0.0,
                        12,
                    ),
                    line_body(WorldPoint { x: 0.0, y: 40.0 }, 0.0, 12),
                ),
                (
                    snake(
                        21,
                        SnakeKind::Evolved,
                        WorldPoint { x: 100.0, y: 40.0 },
                        0.0,
                        15,
                    ),
                    line_body(WorldPoint { x: 100.0, y: 40.0 }, 0.0, 15),
                ),
            ];
            if reverse {
                pack_world(entries.into_iter().rev().collect())
            } else {
                pack_world(entries)
            }
        }

        let rng = rng_bundle();
        let allocators = allocators();
        let forward = execute(&world(false), &rng, &allocators, 100_000)
            .expect("two forward deaths should realize");
        let reversed = execute(&world(true), &rng, &allocators, 100_000)
            .expect("two reversed deaths should realize");
        assert_eq!(forward, reversed);
        assert_eq!(forward.diagnostics.corpse_big_pellets, 4);
        assert_eq!(forward.diagnostics.corpse_small_pellets, 4);
        assert_eq!(
            forward
                .pellets
                .iter()
                .map(|pellet| pellet.owner)
                .collect::<Vec<_>>(),
            vec![
                Some(7),
                Some(7),
                Some(7),
                Some(7),
                Some(8),
                Some(8),
                Some(8),
                Some(8),
            ]
        );
        assert_eq!(
            forward
                .pellets
                .iter()
                .map(|pellet| pellet.id)
                .collect::<Vec<_>>(),
            (1_000..1_008).collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_effect_boundary_preserves_rng_and_allocator_exactly() {
        let first = snake(
            1,
            SnakeKind::Evolved,
            WorldPoint { x: -500.0, y: 0.0 },
            0.0,
            5,
        );
        let second = snake(
            2,
            SnakeKind::Evolved,
            WorldPoint { x: 500.0, y: 0.0 },
            0.0,
            5,
        );
        let world = pack_world(vec![
            (first, line_body(WorldPoint { x: -500.0, y: 0.0 }, 0.0, 5)),
            (second, line_body(WorldPoint { x: 500.0, y: 0.0 }, 0.0, 5)),
        ]);
        let rng = rng_bundle();
        let allocators = allocators();
        let result = execute(&world, &rng, &allocators, 100_000).expect("empty effects");
        assert!(result.pellets.is_empty());
        assert_eq!(result.rng, rng);
        assert_eq!(result.allocators, allocators);
        assert_eq!(result.diagnostics.total_pellets, 0);
    }

    #[test]
    fn wall_killed_baseline_notifies_without_drops_or_rng_draws() {
        let position = WorldPoint { x: 3_495.0, y: 0.0 };
        let baseline = snake(9, SnakeKind::Baseline, position, 0.0, 8);
        let world = pack_world(vec![(baseline, line_body(position, 0.0, 8))]);
        let rng = rng_bundle();
        let allocators = allocators();
        let result = execute(&world, &rng, &allocators, 100_000).expect("wall effects");
        assert!(result.pellets.is_empty());
        assert_eq!(result.rng, rng);
        assert_eq!(result.allocators, allocators);
        assert_eq!(
            result.baseline_deaths,
            vec![BaselineDeathEvent {
                slot: 0,
                snake_id: 9,
            }]
        );
    }

    #[test]
    fn pellet_capacity_and_allocator_exhaustion_reject_before_publication() {
        let world = body_collision_world(SnakeKind::Evolved, false);
        let rng = rng_bundle();
        let allocators = allocators();
        let error = execute(&world, &rng, &allocators, 1)
            .expect_err("complete death effects should exceed one pellet");
        assert!(matches!(
            error,
            EffectError::PelletCapacityExceeded {
                required,
                maximum: 1
            } if required > 1
        ));
        assert_eq!(rng, rng_bundle());
        assert_eq!(allocators, self::allocators());

        let mut exhausted = allocators.clone();
        exhausted.next_entity_id = EXTERNAL_ENTITY_ID_START - 1;
        let error = execute(&world, &rng, &exhausted, 100_000)
            .expect_err("entity domain should reject the whole reservation");
        assert!(matches!(
            error,
            EffectError::Allocator(StateError::IdExhausted { kind: "entity", .. })
        ));
        assert_eq!(exhausted.next_entity_id, EXTERNAL_ENTITY_ID_START - 1);
    }

    #[test]
    fn malformed_baseline_stream_is_rejected_without_ready_output() {
        let world = body_collision_world(SnakeKind::Baseline, false);
        let indexed = indexed(&world);
        let movement_config = MovementConfig::typescript_defaults();
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, movement_config, DT, 100_000, 100_000)
            .expect("movement");
        let mut food_workspace = FoodWorkspace::new();
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("food");
        let mut collision_workspace = CollisionWorkspace::new();
        let collision = collision_workspace
            .prepare(food, CollisionConfig::typescript_defaults())
            .expect("collision");
        let mut rng = rng_bundle();
        rng.baselines[0].slot = 1;
        let mut workspace = EffectWorkspace::new();
        let error = workspace
            .prepare(
                collision,
                &rng,
                &allocators(),
                movement_config,
                FoodConfig::typescript_defaults(),
                DeathDropConfig::typescript_defaults(),
                100_000,
            )
            .expect_err("non-dense baseline RNG should reject");
        assert_eq!(
            error,
            EffectError::NonDenseBaselineRng { index: 0, slot: 1 }
        );
        assert!(!workspace.is_ready());
    }

    #[test]
    fn post_draw_failure_leaves_inputs_unchanged_and_next_prepare_clears_scratch() {
        let world = body_collision_world(SnakeKind::Evolved, false);
        let source_world = world.clone();
        let indexed = indexed(&world);
        let movement_config = MovementConfig::typescript_defaults();
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, movement_config, DT, 100_000, 100_000)
            .expect("movement");
        let mut food_workspace = FoodWorkspace::new();
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("food");
        let mut collision_workspace = CollisionWorkspace::new();
        let collision = collision_workspace
            .prepare(food, CollisionConfig::typescript_defaults())
            .expect("collision");
        let rng = rng_bundle();
        let source_rng = rng.clone();
        let allocators = allocators();
        let source_allocators = allocators.clone();
        let mut workspace = EffectWorkspace::new();
        let mut extreme = DeathDropConfig::typescript_defaults();
        extreme.jitter = f64::MAX;
        extreme.cluster_jitter = f64::MAX;
        let error = workspace
            .prepare(
                collision,
                &rng,
                &allocators,
                movement_config,
                FoodConfig::typescript_defaults(),
                extreme,
                100_000,
            )
            .expect_err("finite extreme jitter should fail after the first draw");
        assert!(matches!(error, EffectError::NonFiniteGenerated { .. }));
        assert!(!workspace.is_ready());
        assert_eq!(world, source_world);
        assert_eq!(rng, source_rng);
        assert_eq!(allocators, source_allocators);

        let prepared = workspace
            .prepare(
                collision,
                &rng,
                &allocators,
                movement_config,
                FoodConfig::typescript_defaults(),
                DeathDropConfig::typescript_defaults(),
                100_000,
            )
            .expect("valid retry should clear rejected partial scratch");
        assert_eq!(prepared.pellets().len(), 4);
        assert_eq!(prepared.pellets()[0].id, 1_000);
        assert!(workspace.is_ready());
    }

    #[test]
    fn checked_reservation_and_effect_buffers_reuse_warmed_capacity() {
        let mut values = Vec::with_capacity(5);
        reserve_for(&mut values, 10, "reservation regression")
            .expect("checked reserve should cover the required final length");
        assert!(values.capacity() >= 10);
        let reserved_capacity = values.capacity();
        values.extend(0..10);
        assert_eq!(values.capacity(), reserved_capacity);

        let world = body_collision_world(SnakeKind::Baseline, false);
        let indexed = indexed(&world);
        let movement_config = MovementConfig::typescript_defaults();
        let rng = rng_bundle();
        let allocators = allocators();
        let mut movement_workspace = MovementWorkspace::new();
        let mut food_workspace = FoodWorkspace::new();
        let mut collision_workspace = CollisionWorkspace::new();
        let mut effects_workspace = EffectWorkspace::new();
        let mut warmed: Option<EffectDiagnostics> = None;
        for _ in 0..24 {
            let movement = movement_workspace
                .prepare(&world, movement_config, DT, 100_000, 100_000)
                .expect("movement");
            let food = food_workspace
                .prepare(
                    &indexed,
                    movement,
                    movement_config,
                    FoodConfig::typescript_defaults(),
                    100_000,
                    100_000,
                )
                .expect("food");
            let collision = collision_workspace
                .prepare(food, CollisionConfig::typescript_defaults())
                .expect("collision");
            let prepared = effects_workspace
                .prepare(
                    collision,
                    &rng,
                    &allocators,
                    movement_config,
                    FoodConfig::typescript_defaults(),
                    DeathDropConfig::typescript_defaults(),
                    100_000,
                )
                .expect("effects");
            let diagnostics = prepared.diagnostics();
            assert!(diagnostics.snake_order_capacity >= world.snakes.len());
            assert!(diagnostics.death_plan_capacity >= 1);
            assert!(diagnostics.pellet_capacity >= diagnostics.total_pellets);
            assert!(diagnostics.baseline_event_capacity >= 1);
            assert!(diagnostics.baseline_rng_capacity >= rng.baselines.len());
            if let Some(expected) = warmed {
                assert_eq!(diagnostics, expected);
            } else {
                warmed = Some(diagnostics);
            }
        }
    }
}
