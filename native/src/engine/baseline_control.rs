//! Pure baseline-bot strategy evaluation from one already-built sensor observation.
//!
//! This slice deliberately does not sample sensors or publish authoritative
//! state. The later fixed-step coordinator must obtain the corrected shared
//! observation, retain its delivery marker, evaluate every live baseline slot,
//! and join the staged result to the same keyed transaction before committing.

use super::baseline::{validate_strategy_runtime, BaselineSlotRuntime};
use super::rng::StatefulRng;
use super::sensor_layout::{
    SensorLayout, MAX_SENSOR_BINS, MIN_SENSOR_BINS, SENSOR_CHANNEL_COUNT, SENSOR_SCALAR_COUNT,
};
use super::state::{
    BaselineRngState, BaselineStrategyState, SnakeKind, SnakeState, WorldPoint, WorldState,
};
use std::error::Error;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use std::fmt::{Display, Formatter};

/// First Rust baseline-controller algorithm identity.
pub const BASELINE_CONTROL_VERSION: u32 = 1;
/// Small snakes use the survival-and-growth policy below this body length.
const MEDIUM_LENGTH_THRESHOLD: usize = 25;
/// Large snakes use the crowd-pressure policy at or above this body length.
const LARGE_LENGTH_THRESHOLD: usize = 80;
/// Hard admission ceiling above the current configurable world population.
const ABSOLUTE_MAXIMUM_WORLD_SNAKES: usize = 10_000;
const VETO_PENALTY: f64 = 1_000.0;
const VETO_THRESHOLD: f64 = -0.4;
const WANDER_ANGLE_SCALE: f64 = 0.6;
const FOOD_TRIGGER_THRESHOLD: f64 = 0.1;
const ENVIRONMENT_SAFE_THRESHOLD: f64 = -0.3;
const AVOID_DURATION_BASE: f64 = 0.35;
const BOOST_CHANCE_PER_BOUNDARY: f64 = 0.02;
const SCORE_TIE_TOLERANCE: f64 = 1.0e-6;

/// Versioned live settings needed by the baseline action formula.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineControlConfig {
    /// Formula identity.
    pub algorithm_version: u32,
    /// Current live boost threshold from normalized settings.
    pub minimum_points_to_boost: f64,
    /// Admitted total world-snake ceiling for bounded canonical scans.
    pub maximum_world_snakes: usize,
}

impl BaselineControlConfig {
    /// Current TypeScript defaults and the maximum configured 300-snake case
    /// plus admitted non-population controllers.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            algorithm_version: BASELINE_CONTROL_VERSION,
            minimum_points_to_boost: 1.2,
            maximum_world_snakes: 512,
        }
    }

    pub(crate) fn validate(self) -> Result<(), BaselineControlError> {
        if self.algorithm_version != BASELINE_CONTROL_VERSION {
            return Err(BaselineControlError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        if !self.minimum_points_to_boost.is_finite()
            || !(0.0..=60.0).contains(&self.minimum_points_to_boost)
        {
            return Err(BaselineControlError::InvalidConfig {
                field: "minimum_points_to_boost",
            });
        }
        if self.maximum_world_snakes == 0
            || self.maximum_world_snakes > ABSOLUTE_MAXIMUM_WORLD_SNAKES
        {
            return Err(BaselineControlError::InvalidConfig {
                field: "maximum_world_snakes",
            });
        }
        Ok(())
    }
}

/// Body-length policy selected for one action boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineLifeStage {
    /// Survival, growth, and escape-only boost.
    Small,
    /// Nearby-head hunting and conditional attack boost.
    Medium,
    /// Nearby crowd-centroid pressure without attack boost.
    Large,
}

/// Small diagnostics retained with one pure evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaselineControlDiagnostics {
    /// Total world records inspected through stable-ID ordering.
    pub world_snakes: usize,
    /// Retained canonical index capacity.
    pub canonical_order_capacity: usize,
    /// Exact number of uniform samples consumed from this slot's stream.
    pub uniform_draws: usize,
    /// Stable target identity selected by the medium policy, when present.
    pub target_snake_id: Option<u64>,
}

/// Read-only output of one pure controller evaluation.
///
/// This is not an authority-commit token. It intentionally exposes no apply
/// method: the later coordinator must revalidate and join it with the shared
/// observation delivery and complete fixed-step key.
#[derive(Clone, Copy, Debug)]
pub struct BaselineControlEvaluation<'workspace, 'source> {
    source_world: &'source WorldState,
    source_slot: &'source BaselineSlotRuntime,
    source_rng: &'source BaselineRngState,
    source_observation: &'source [f32],
    config: BaselineControlConfig,
    fixed_dt: f64,
    snake_index: usize,
    next_slot: BaselineSlotRuntime,
    next_strategy: BaselineStrategyState,
    effective_strategy: BaselineStrategyState,
    life_stage: BaselineLifeStage,
    target_bin: usize,
    next_rng: &'workspace BaselineRngState,
    diagnostics: BaselineControlDiagnostics,
}

impl<'workspace, 'source> BaselineControlEvaluation<'workspace, 'source> {
    /// Immutable world boundary used by this calculation.
    #[must_use]
    pub const fn source_world(self) -> &'source WorldState {
        self.source_world
    }

    /// Immutable lifecycle slot used by this calculation.
    #[must_use]
    pub const fn source_slot(self) -> &'source BaselineSlotRuntime {
        self.source_slot
    }

    /// Immutable per-slot RNG continuation used by this calculation.
    #[must_use]
    pub const fn source_rng(self) -> &'source BaselineRngState {
        self.source_rng
    }

    /// Exact shared corrected observation consumed by this calculation.
    #[must_use]
    pub const fn source_observation(self) -> &'source [f32] {
        self.source_observation
    }

    /// Validated formula configuration used.
    #[must_use]
    pub const fn config(self) -> BaselineControlConfig {
        self.config
    }

    /// Fixed delta used for behavior timers.
    #[must_use]
    pub const fn fixed_dt(self) -> f64 {
        self.fixed_dt
    }

    /// Source world index for the controlled baseline snake.
    #[must_use]
    pub const fn snake_index(self) -> usize {
        self.snake_index
    }

    /// Staged lifecycle action/timer continuation.
    #[must_use]
    pub const fn next_slot(self) -> BaselineSlotRuntime {
        self.next_slot
    }

    /// Durable strategy state written to the baseline snake after a joined commit.
    #[must_use]
    pub const fn next_strategy(self) -> BaselineStrategyState {
        self.next_strategy
    }

    /// Action-only state after the medium hunter override.
    #[must_use]
    pub const fn effective_strategy(self) -> BaselineStrategyState {
        self.effective_strategy
    }

    /// Body-length policy used.
    #[must_use]
    pub const fn life_stage(self) -> BaselineLifeStage {
        self.life_stage
    }

    /// Selected sensor bin.
    #[must_use]
    pub const fn target_bin(self) -> usize {
        self.target_bin
    }

    /// Staged per-slot RNG continuation.
    #[must_use]
    pub const fn next_rng(self) -> &'workspace BaselineRngState {
        self.next_rng
    }

    /// Bounded-work and draw diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> BaselineControlDiagnostics {
        self.diagnostics
    }
}

/// Reusable scratch for stable target ordering and serialized RNG output.
#[derive(Debug)]
pub struct BaselineControlWorkspace {
    canonical_snake_order: Vec<usize>,
    next_rng: BaselineRngState,
    ready: bool,
}

impl Default for BaselineControlWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl BaselineControlWorkspace {
    /// Construct reusable non-authoritative controller scratch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            canonical_snake_order: Vec::new(),
            next_rng: BaselineRngState {
                slot: 0,
                state: StatefulRng::new(1.0).export_state(),
            },
            ready: false,
        }
    }

    /// Evaluate one live baseline from an already-built corrected observation.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'workspace, 'source>(
        &'workspace mut self,
        source_world: &'source WorldState,
        snake_index: usize,
        source_slot: &'source BaselineSlotRuntime,
        source_rng: &'source BaselineRngState,
        observation: &'source [f32],
        layout: &SensorLayout,
        config: BaselineControlConfig,
        fixed_dt: f64,
    ) -> Result<BaselineControlEvaluation<'workspace, 'source>, BaselineControlError> {
        self.ready = false;
        config.validate()?;
        validate_layout(layout)?;
        if !fixed_dt.is_finite() || fixed_dt <= 0.0 || fixed_dt > 1.0 {
            return Err(BaselineControlError::InvalidFixedDelta);
        }
        if source_world.snakes.len() > config.maximum_world_snakes {
            return Err(BaselineControlError::WorldCapacityExceeded {
                actual: source_world.snakes.len(),
                maximum: config.maximum_world_snakes,
            });
        }
        if observation.len() != layout.input_size {
            return Err(BaselineControlError::ObservationLength {
                expected: layout.input_size,
                actual: observation.len(),
            });
        }
        if let Some(index) = observation.iter().position(|value| !value.is_finite()) {
            return Err(BaselineControlError::NonFiniteObservation { index });
        }
        validate_slot_shape(source_slot)?;
        if source_rng.slot != source_slot.slot {
            return Err(BaselineControlError::RngSlotMismatch {
                expected: source_slot.slot,
                actual: source_rng.slot,
            });
        }

        self.canonical_snake_order.clear();
        reserve_for(
            &mut self.canonical_snake_order,
            source_world.snakes.len(),
            "canonical snake order",
        )?;
        self.canonical_snake_order
            .extend(0..source_world.snakes.len());
        self.canonical_snake_order
            .sort_unstable_by_key(|index| source_world.snakes[*index].id);
        for pair in self.canonical_snake_order.windows(2) {
            let first = source_world.snakes[pair[0]].id;
            let second = source_world.snakes[pair[1]].id;
            if first == 0 || first == second {
                return Err(BaselineControlError::InvalidWorldSnake {
                    snake_id: second,
                    field: "id",
                });
            }
        }
        if self
            .canonical_snake_order
            .first()
            .is_some_and(|index| source_world.snakes[*index].id == 0)
        {
            return Err(BaselineControlError::InvalidWorldSnake {
                snake_id: 0,
                field: "id",
            });
        }
        for index in self.canonical_snake_order.iter().copied() {
            validate_relevant_snake(source_world, index)?;
        }

        let snake = source_world
            .snakes
            .get(snake_index)
            .ok_or(BaselineControlError::InvalidSnakeIndex(snake_index))?;
        if snake.kind != SnakeKind::Baseline
            || !snake.alive
            || snake.baseline_slot != Some(source_slot.slot)
            || snake.id != source_slot.snake_id
            || source_slot.respawn_remaining_seconds.is_some()
        {
            return Err(BaselineControlError::InvalidControlledSnake {
                slot: source_slot.slot,
                snake_id: snake.id,
            });
        }
        let source_strategy = snake
            .baseline_strategy
            .ok_or(BaselineControlError::MissingStrategy { snake_id: snake.id })?;
        validate_strategy_runtime(*source_slot, source_strategy)
            .map_err(|_| BaselineControlError::InvalidSlotState(source_slot.slot))?;
        let mut rng = StatefulRng::from_state(&source_rng.state)
            .map_err(|_| BaselineControlError::InvalidRngState(source_rng.slot))?;
        let mut uniform_draws = 0usize;
        let outcome = evaluate(
            source_world,
            snake_index,
            source_slot,
            source_strategy,
            observation,
            layout,
            config,
            fixed_dt,
            &self.canonical_snake_order,
            &mut rng,
            &mut uniform_draws,
        )?;
        self.next_rng.slot = source_rng.slot;
        rng.export_state_into(&mut self.next_rng.state);
        self.ready = true;
        Ok(BaselineControlEvaluation {
            source_world,
            source_slot,
            source_rng,
            source_observation: observation,
            config,
            fixed_dt,
            snake_index,
            next_slot: outcome.next_slot,
            next_strategy: outcome.next_strategy,
            effective_strategy: outcome.effective_strategy,
            life_stage: outcome.life_stage,
            target_bin: outcome.target_bin,
            next_rng: &self.next_rng,
            diagnostics: BaselineControlDiagnostics {
                world_snakes: source_world.snakes.len(),
                canonical_order_capacity: self.canonical_snake_order.capacity(),
                uniform_draws,
                target_snake_id: outcome.target_snake_id,
            },
        })
    }

    /// Whether the latest call completed every calculation and validation.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }
}

#[derive(Clone, Copy, Debug)]
struct EvaluationOutcome {
    next_slot: BaselineSlotRuntime,
    next_strategy: BaselineStrategyState,
    effective_strategy: BaselineStrategyState,
    life_stage: BaselineLifeStage,
    target_bin: usize,
    target_snake_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct DirectionBias {
    angle: f64,
    strength: f64,
    falloff: f64,
}

#[allow(clippy::too_many_arguments)]
fn evaluate(
    world: &WorldState,
    snake_index: usize,
    source_slot: &BaselineSlotRuntime,
    source_strategy: BaselineStrategyState,
    observation: &[f32],
    layout: &SensorLayout,
    config: BaselineControlConfig,
    fixed_dt: f64,
    canonical_snake_order: &[usize],
    rng: &mut StatefulRng,
    uniform_draws: &mut usize,
) -> Result<EvaluationOutcome, BaselineControlError> {
    let snake = &world.snakes[snake_index];
    let life_stage = if snake.body.len < MEDIUM_LENGTH_THRESHOLD {
        BaselineLifeStage::Small
    } else if snake.body.len < LARGE_LENGTH_THRESHOLD {
        BaselineLifeStage::Medium
    } else {
        BaselineLifeStage::Large
    };
    let (next_strategy, strategy_timer_seconds) = update_strategy(
        source_strategy,
        source_slot.strategy_timer_seconds,
        observation,
        layout,
        snake,
        config,
        fixed_dt,
        rng,
        uniform_draws,
    );
    let mut effective_strategy = next_strategy;
    let mut food_weight;
    let mut clearance_weight;
    let food_clamp;
    let mut direction_bias = DirectionBias::default();
    let mut target_snake_id = None;
    let mut strict_boost = false;
    let mut attack_boost = false;

    match life_stage {
        BaselineLifeStage::Small => {
            food_weight = 0.5;
            clearance_weight = 1.8;
            if next_strategy == BaselineStrategyState::Seek {
                clearance_weight = 1.6;
            } else if next_strategy == BaselineStrategyState::Avoid {
                food_weight = 0.0;
                clearance_weight = 2.5;
            }
            food_clamp = 0.4;
            strict_boost = true;
        }
        BaselineLifeStage::Medium => {
            food_weight = 0.8;
            clearance_weight = 1.2;
            if next_strategy == BaselineStrategyState::Avoid {
                food_weight = 0.0;
                clearance_weight = 2.0;
            } else {
                let sense_radius = snake.radius * 25.0;
                if !sense_radius.is_finite() {
                    return Err(BaselineControlError::NonFiniteDerived);
                }
                if let Some(target_index) =
                    closest_medium_target(world, snake_index, canonical_snake_order, sense_radius)?
                {
                    let target = &world.snakes[target_index];
                    let my_head = head(world, snake_index)?;
                    let target_head = head(world, target_index)?;
                    let angle_to = (target_head.y - my_head.y).atan2(target_head.x - my_head.x);
                    if snake.body.len as f64 > target.body.len as f64 * 2.5 && snake.body.len > 50 {
                        direction_bias = DirectionBias {
                            angle: normalize_angle(angle_to + FRAC_PI_2 - 0.4 - snake.direction),
                            strength: 1.0,
                            falloff: 1.2,
                        };
                    } else {
                        let lead_x = target_head.x + target.direction.cos() * target.speed * 0.5;
                        let lead_y = target_head.y + target.direction.sin() * target.speed * 0.5;
                        if !lead_x.is_finite() || !lead_y.is_finite() {
                            return Err(BaselineControlError::NonFiniteDerived);
                        }
                        let lead_angle = (lead_y - my_head.y).atan2(lead_x - my_head.x);
                        direction_bias = DirectionBias {
                            angle: normalize_angle(lead_angle - snake.direction),
                            strength: 0.8,
                            falloff: 1.2,
                        };
                    }
                    effective_strategy = BaselineStrategyState::Seek;
                    target_snake_id = Some(target.id);
                    attack_boost = true;
                }
            }
            food_clamp = 0.6;
        }
        BaselineLifeStage::Large => {
            food_weight = 0.4;
            clearance_weight = 1.5;
            if next_strategy == BaselineStrategyState::Avoid {
                food_weight = 0.0;
                clearance_weight = 2.5;
            } else {
                let sense_radius = snake.radius * 30.0;
                if !sense_radius.is_finite() {
                    return Err(BaselineControlError::NonFiniteDerived);
                }
                if let Some(angle) =
                    crowd_angle(world, snake_index, canonical_snake_order, sense_radius)?
                {
                    direction_bias = DirectionBias {
                        angle: normalize_angle(angle - snake.direction),
                        strength: 0.6,
                        falloff: 1.0,
                    };
                }
            }
            food_clamp = 0.4;
        }
    }

    let target_bin = evaluate_bins(
        observation,
        layout,
        food_weight,
        clearance_weight,
        food_clamp,
        direction_bias,
    );
    let mut wander_angle = source_slot.wander_angle;
    let mut wander_timer_seconds = source_slot.wander_timer_seconds;
    if effective_strategy == BaselineStrategyState::Roam {
        wander_timer_seconds -= fixed_dt;
        if wander_timer_seconds <= 0.0 {
            wander_angle = (next_uniform(rng, uniform_draws) - 0.5) * WANDER_ANGLE_SCALE;
            wander_timer_seconds = 0.6 + next_uniform(rng, uniform_draws) * 1.4;
        }
    }
    let target_angle = centered_bin_to_angle(target_bin, layout.bins)
        + if effective_strategy == BaselineStrategyState::Roam {
            wander_angle
        } else {
            0.0
        };
    let turn = (target_angle / FRAC_PI_2).clamp(-1.0, 1.0);
    let mut boost = effective_strategy == BaselineStrategyState::Boost;
    if effective_strategy == BaselineStrategyState::Avoid {
        let hazard = f64::from(observation[layout.offsets.hazard + target_bin]);
        let wall = f64::from(observation[layout.offsets.wall + target_bin]);
        boost = (hazard + wall) * 0.5 > 0.2;
    }
    if attack_boost && effective_strategy != BaselineStrategyState::Avoid {
        let hazard = f64::from(observation[layout.offsets.hazard + target_bin]);
        let wall = f64::from(observation[layout.offsets.wall + target_bin]);
        if (hazard + wall) * 0.5 > -0.1 {
            boost = true;
        }
    }
    if strict_boost && effective_strategy != BaselineStrategyState::Avoid {
        boost = false;
    }
    if !turn.is_finite()
        || !strategy_timer_seconds.is_finite()
        || !wander_angle.is_finite()
        || !wander_timer_seconds.is_finite()
    {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    let turn = turn as f32;
    if !turn.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    let mut next_slot = *source_slot;
    next_slot.strategy_timer_seconds = strategy_timer_seconds;
    next_slot.wander_angle = wander_angle;
    next_slot.wander_timer_seconds = wander_timer_seconds;
    next_slot.turn = turn;
    next_slot.boost = boost;
    Ok(EvaluationOutcome {
        next_slot,
        next_strategy,
        effective_strategy,
        life_stage,
        target_bin,
        target_snake_id,
    })
}

#[allow(clippy::too_many_arguments)]
fn update_strategy(
    source: BaselineStrategyState,
    source_timer: f64,
    observation: &[f32],
    layout: &SensorLayout,
    snake: &SnakeState,
    config: BaselineControlConfig,
    fixed_dt: f64,
    rng: &mut StatefulRng,
    uniform_draws: &mut usize,
) -> (BaselineStrategyState, f64) {
    let mut strategy = source;
    let mut timer = source_timer;
    if matches!(
        strategy,
        BaselineStrategyState::Avoid | BaselineStrategyState::Boost
    ) {
        timer -= fixed_dt;
        if timer <= 0.0 {
            strategy = BaselineStrategyState::Roam;
            timer = 0.0;
        }
    }
    let mut worst_clearance = f64::INFINITY;
    let mut best_food = f64::NEG_INFINITY;
    for bin in 0..layout.bins {
        let hazard = f64::from(observation[layout.offsets.hazard + bin]);
        let wall = f64::from(observation[layout.offsets.wall + bin]);
        let head = f64::from(observation[layout.offsets.head + bin]);
        worst_clearance = worst_clearance.min(hazard.min(wall).min(head));
        best_food = best_food.max(f64::from(observation[layout.offsets.food + bin]));
    }
    if strategy != BaselineStrategyState::Avoid && worst_clearance < -0.15 {
        strategy = BaselineStrategyState::Avoid;
        timer = AVOID_DURATION_BASE + next_uniform(rng, uniform_draws) * AVOID_DURATION_BASE;
    } else if !matches!(
        strategy,
        BaselineStrategyState::Avoid | BaselineStrategyState::Boost
    ) {
        strategy = if best_food > FOOD_TRIGGER_THRESHOLD {
            BaselineStrategyState::Seek
        } else {
            BaselineStrategyState::Roam
        };
        let boost_allowed = snake.points > config.minimum_points_to_boost * 1.1;
        let environment_safe = worst_clearance > ENVIRONMENT_SAFE_THRESHOLD;
        if boost_allowed
            && environment_safe
            && next_uniform(rng, uniform_draws) < BOOST_CHANCE_PER_BOUNDARY
        {
            strategy = BaselineStrategyState::Boost;
            timer = 0.2 + next_uniform(rng, uniform_draws) * 0.2;
        }
    }
    (strategy, timer)
}

fn evaluate_bins(
    observation: &[f32],
    layout: &SensorLayout,
    food_weight: f64,
    clearance_weight: f64,
    food_clamp: f64,
    bias: DirectionBias,
) -> usize {
    let mut best_score = f64::NEG_INFINITY;
    let mut best_angle = f64::INFINITY;
    let mut target_bin = 0usize;
    let mut best_clearance = f64::NEG_INFINITY;
    let mut best_clearance_angle = f64::INFINITY;
    let mut best_clearance_bin = 0usize;
    let mut any_non_veto = false;
    for bin in 0..layout.bins {
        let angle = centered_bin_to_angle(bin, layout.bins);
        let food = f64::from(observation[layout.offsets.food + bin]).min(food_clamp);
        let hazard = f64::from(observation[layout.offsets.hazard + bin]);
        let wall = f64::from(observation[layout.offsets.wall + bin]);
        let head = f64::from(observation[layout.offsets.head + bin]);
        let clearance = hazard.min(wall).min(head);
        if clearance > best_clearance + SCORE_TIE_TOLERANCE
            || ((clearance - best_clearance).abs() <= SCORE_TIE_TOLERANCE
                && angle.abs() < best_clearance_angle.abs())
        {
            best_clearance = clearance;
            best_clearance_bin = bin;
            best_clearance_angle = angle;
        }
        let mut score = food * food_weight + clearance * clearance_weight;
        if bias.strength > 0.0 {
            let difference = normalize_angle(angle - bias.angle).abs();
            if difference < bias.falloff {
                score += bias.strength * (1.0 - difference / bias.falloff);
            }
        }
        if angle.abs() > FRAC_PI_4 && clearance < 0.2 {
            let side_risk = (angle.abs() - FRAC_PI_4) / FRAC_PI_2;
            let proximity_risk = (1.0 - clearance).clamp(0.0, 1.0);
            score -= side_risk * proximity_risk * 50.0;
        }
        if clearance < VETO_THRESHOLD {
            score -= VETO_PENALTY;
        } else {
            any_non_veto = true;
        }
        if score > best_score + SCORE_TIE_TOLERANCE
            || ((score - best_score).abs() <= SCORE_TIE_TOLERANCE && angle.abs() < best_angle.abs())
        {
            best_score = score;
            target_bin = bin;
            best_angle = angle;
        }
    }
    if any_non_veto {
        target_bin
    } else {
        best_clearance_bin
    }
}

fn closest_medium_target(
    world: &WorldState,
    snake_index: usize,
    canonical_order: &[usize],
    sense_radius: f64,
) -> Result<Option<usize>, BaselineControlError> {
    let controlled_head = head(world, snake_index)?;
    let limit_squared = sense_radius * sense_radius;
    if !limit_squared.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    let mut closest = None;
    let mut closest_squared = f64::INFINITY;
    for other_index in canonical_order.iter().copied() {
        let other = &world.snakes[other_index];
        if other_index == snake_index || !other.alive {
            continue;
        }
        let other_head = head(world, other_index)?;
        let dx = other_head.x - controlled_head.x;
        let dy = other_head.y - controlled_head.y;
        let squared = squared_distance(dx, dy)?;
        if squared < limit_squared && squared < closest_squared {
            closest = Some(other_index);
            closest_squared = squared;
        }
    }
    Ok(closest)
}

fn crowd_angle(
    world: &WorldState,
    snake_index: usize,
    canonical_order: &[usize],
    sense_radius: f64,
) -> Result<Option<f64>, BaselineControlError> {
    let controlled_head = head(world, snake_index)?;
    let limit_squared = sense_radius * sense_radius;
    if !limit_squared.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut count = 0usize;
    for other_index in canonical_order.iter().copied() {
        let other = &world.snakes[other_index];
        if other_index == snake_index || !other.alive {
            continue;
        }
        let other_head = head(world, other_index)?;
        let dx = other_head.x - controlled_head.x;
        let dy = other_head.y - controlled_head.y;
        if squared_distance(dx, dy)? < limit_squared {
            sum_x += other_head.x;
            sum_y += other_head.y;
            if !sum_x.is_finite() || !sum_y.is_finite() {
                return Err(BaselineControlError::NonFiniteDerived);
            }
            count += 1;
        }
    }
    if count == 0 {
        return Ok(None);
    }
    let center_x = sum_x / count as f64;
    let center_y = sum_y / count as f64;
    let delta_x = center_x - controlled_head.x;
    let delta_y = center_y - controlled_head.y;
    if !delta_x.is_finite() || !delta_y.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    let angle = delta_y.atan2(delta_x);
    if !angle.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    Ok(Some(angle))
}

fn squared_distance(dx: f64, dy: f64) -> Result<f64, BaselineControlError> {
    if !dx.is_finite() || !dy.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    let x_squared = dx * dx;
    let y_squared = dy * dy;
    let squared = x_squared + y_squared;
    if !x_squared.is_finite() || !y_squared.is_finite() || !squared.is_finite() {
        return Err(BaselineControlError::NonFiniteDerived);
    }
    Ok(squared)
}

fn head(world: &WorldState, snake_index: usize) -> Result<WorldPoint, BaselineControlError> {
    let snake = &world.snakes[snake_index];
    world.body_points.get(snake.body.start).copied().ok_or(
        BaselineControlError::InvalidWorldSnake {
            snake_id: snake.id,
            field: "body",
        },
    )
}

fn next_uniform(rng: &mut StatefulRng, draws: &mut usize) -> f64 {
    *draws += 1;
    rng.next_f64()
}

fn centered_bin_to_angle(index: usize, bins: usize) -> f64 {
    -PI + index as f64 / bins as f64 * TAU
}

fn normalize_angle(mut angle: f64) -> f64 {
    angle %= TAU;
    while angle > PI {
        angle -= TAU;
    }
    while angle < -PI {
        angle += TAU;
    }
    angle
}

fn validate_layout(layout: &SensorLayout) -> Result<(), BaselineControlError> {
    let expected_input = SENSOR_SCALAR_COUNT
        .checked_add(layout.bins.saturating_mul(SENSOR_CHANNEL_COUNT))
        .ok_or(BaselineControlError::InvalidSensorLayout)?;
    if !(MIN_SENSOR_BINS..=MAX_SENSOR_BINS).contains(&layout.bins)
        || layout.scalar_count != SENSOR_SCALAR_COUNT
        || layout.channel_count != SENSOR_CHANNEL_COUNT
        || layout.input_size != expected_input
        || layout.offsets.food != SENSOR_SCALAR_COUNT
        || layout.offsets.hazard != SENSOR_SCALAR_COUNT + layout.bins
        || layout.offsets.wall != SENSOR_SCALAR_COUNT + layout.bins * 2
        || layout.offsets.head != SENSOR_SCALAR_COUNT + layout.bins * 3
    {
        return Err(BaselineControlError::InvalidSensorLayout);
    }
    Ok(())
}

fn validate_slot_shape(slot: &BaselineSlotRuntime) -> Result<(), BaselineControlError> {
    if slot.snake_id == 0
        || !slot.strategy_timer_seconds.is_finite()
        || slot.strategy_timer_seconds < 0.0
        || !slot.wander_angle.is_finite()
        || !slot.wander_timer_seconds.is_finite()
        || slot.wander_timer_seconds < 0.0
        || !slot.turn.is_finite()
        || !(-1.0..=1.0).contains(&slot.turn)
        || slot.respawn_remaining_seconds.is_some()
    {
        return Err(BaselineControlError::InvalidSlotState(slot.slot));
    }
    Ok(())
}

fn validate_relevant_snake(world: &WorldState, index: usize) -> Result<(), BaselineControlError> {
    let snake = &world.snakes[index];
    if snake.id == 0 {
        return Err(BaselineControlError::InvalidWorldSnake {
            snake_id: snake.id,
            field: "id",
        });
    }
    if !snake.alive {
        return Ok(());
    }
    let end = snake.body.start.checked_add(snake.body.len).ok_or(
        BaselineControlError::InvalidWorldSnake {
            snake_id: snake.id,
            field: "body",
        },
    )?;
    if snake.body.len == 0 || end > world.body_points.len() {
        return Err(BaselineControlError::InvalidWorldSnake {
            snake_id: snake.id,
            field: "body",
        });
    }
    let head = world.body_points[snake.body.start];
    if !head.x.is_finite()
        || !head.y.is_finite()
        || head != snake.position
        || !snake.direction.is_finite()
        || snake.direction.abs() > TAU
        || !snake.radius.is_finite()
        || snake.radius <= 0.0
        || !snake.speed.is_finite()
        || snake.speed < 0.0
        || !snake.points.is_finite()
    {
        return Err(BaselineControlError::InvalidWorldSnake {
            snake_id: snake.id,
            field: "scalar",
        });
    }
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), BaselineControlError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| BaselineControlError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

/// Checked pure baseline-controller failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineControlError {
    /// Formula settings were unsupported or outside admitted ranges.
    InvalidConfig { field: &'static str },
    /// Fixed delta was non-finite, non-positive, or implausibly large.
    InvalidFixedDelta,
    /// Sensor layout did not match the exact v3 offset contract.
    InvalidSensorLayout,
    /// World storage exceeded its declared bounded scan size.
    WorldCapacityExceeded { actual: usize, maximum: usize },
    /// Requested world index did not exist.
    InvalidSnakeIndex(usize),
    /// Stable IDs, live geometry, or formula scalars were invalid.
    InvalidWorldSnake { snake_id: u64, field: &'static str },
    /// Requested snake did not match one live baseline slot.
    InvalidControlledSnake { slot: u32, snake_id: u64 },
    /// Live baseline snake had no canonical strategy.
    MissingStrategy { snake_id: u64 },
    /// Lifecycle scratch was invalid or represented a dead/waiting slot.
    InvalidSlotState(u32),
    /// RNG state belonged to a different durable baseline slot.
    RngSlotMismatch { expected: u32, actual: u32 },
    /// Per-slot RNG continuation failed strict decoding.
    InvalidRngState(u32),
    /// Shared observation length did not match sensor v3.
    ObservationLength { expected: usize, actual: usize },
    /// Shared observation contained a non-finite Float32.
    NonFiniteObservation { index: usize },
    /// Checked reusable scratch allocation failed.
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    /// A finite admitted input produced a non-finite action intermediate.
    NonFiniteDerived,
}

impl Display for BaselineControlError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => {
                write!(formatter, "invalid baseline-control config: {field}")
            }
            Self::InvalidFixedDelta => write!(formatter, "invalid baseline-control fixed delta"),
            Self::InvalidSensorLayout => {
                write!(formatter, "invalid baseline-control sensor-v3 layout")
            }
            Self::WorldCapacityExceeded { actual, maximum } => write!(
                formatter,
                "baseline-control world has {actual} snakes but limit is {maximum}"
            ),
            Self::InvalidSnakeIndex(index) => write!(
                formatter,
                "baseline-control snake index {index} does not exist"
            ),
            Self::InvalidWorldSnake { snake_id, field } => write!(
                formatter,
                "baseline-control snake {snake_id} has invalid {field}"
            ),
            Self::InvalidControlledSnake { slot, snake_id } => write!(
                formatter,
                "baseline slot {slot} does not own live snake {snake_id}"
            ),
            Self::MissingStrategy { snake_id } => {
                write!(formatter, "baseline snake {snake_id} has no strategy state")
            }
            Self::InvalidSlotState(slot) => {
                write!(formatter, "baseline slot {slot} is not control-ready")
            }
            Self::RngSlotMismatch { expected, actual } => write!(
                formatter,
                "baseline RNG slot {actual} does not match {expected}"
            ),
            Self::InvalidRngState(slot) => {
                write!(formatter, "baseline RNG slot {slot} has invalid state")
            }
            Self::ObservationLength { expected, actual } => write!(
                formatter,
                "baseline observation length {actual} does not match {expected}"
            ),
            Self::NonFiniteObservation { index } => write!(
                formatter,
                "baseline observation value {index} is not finite"
            ),
            Self::AllocationFailed { buffer, required } => write!(
                formatter,
                "baseline-control allocation for {buffer} ({required} entries) failed"
            ),
            Self::NonFiniteDerived => write!(
                formatter,
                "baseline control produced a non-finite derived value"
            ),
        }
    }
}

impl Error for BaselineControlError {}

#[cfg(test)]
mod tests;
