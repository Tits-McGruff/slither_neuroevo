//! Staged snake steering, boost, movement, and packed body-point updates.
//!
//! This module maps the current TypeScript `Snake::advance` formulas without
//! copying its per-snake allocation or container-order mutation. Preparation
//! reads one immutable world, writes only reusable scratch, and orders every
//! side-effect request by stable snake ID. Food, collision, RNG realization,
//! and authoritative commit are deliberately later transaction phases.

use super::state::{BodyRange, SnakeState, WorldPoint, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

const SPEED_RESPONSE: f64 = 6.5;
const BOOST_TRAIL_OFFSET: f64 = 8.0;
const MIN_DISTANCE: f64 = 1.0e-6;

/// Versioned movement values projected from the normalized engine settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementConfig {
    /// Circular arena radius.
    pub world_radius: f64,
    /// Non-boost speed before size penalty.
    pub snake_base_speed: f64,
    /// Boost speed before size penalty.
    pub snake_boost_speed: f64,
    /// Base turn rate in radians per second.
    pub snake_turn_rate: f64,
    /// Radius at the starting body length.
    pub snake_radius: f64,
    /// Maximum collision/render radius.
    pub snake_radius_max: f64,
    /// Logarithmic thickness multiplier.
    pub snake_thickness_scale: f64,
    /// Logarithmic thickness divisor.
    pub snake_thickness_log_div: f64,
    /// Target distance between adjacent body points.
    pub snake_spacing: f64,
    /// Starting body-point count.
    pub snake_start_len: usize,
    /// Maximum body-point count.
    pub snake_max_len: usize,
    /// Minimum body-point count.
    pub snake_min_len: usize,
    /// Fractional base-speed loss at maximum size.
    pub snake_size_speed_penalty: f64,
    /// Fractional boost-addition loss at maximum size.
    pub snake_boost_size_penalty: f64,
    /// Turn-rate denominator penalty at maximum size.
    pub snake_turn_penalty: f64,
    /// Minimum points required to begin a boost burn.
    pub boost_min_points: f64,
    /// Base points burned per simulated second.
    pub boost_points_cost_per_second: f64,
    /// Size multiplier applied to boost point cost.
    pub boost_points_cost_size_factor: f64,
    /// Target body length lost per point burned.
    pub boost_len_loss_per_point: f64,
    /// Authoritative food value used for one boost trail pellet.
    pub food_value: f64,
    /// Boost trail pellet value relative to normal food.
    pub boost_pellet_value_factor: f64,
    /// Symmetric jitter bound applied when RNG requests are realized.
    pub boost_pellet_jitter: f64,
}

impl MovementConfig {
    /// Current TypeScript defaults retained as a named comparison fixture.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            world_radius: 3_500.0,
            snake_base_speed: 165.0,
            snake_boost_speed: 500.0,
            snake_turn_rate: 3.2,
            snake_radius: 9.0,
            snake_radius_max: 18.0,
            snake_thickness_scale: 2.9,
            snake_thickness_log_div: 30.0,
            snake_spacing: 7.5,
            snake_start_len: 5,
            snake_max_len: 10_000,
            snake_min_len: 4,
            snake_size_speed_penalty: 0.18,
            snake_boost_size_penalty: 0.28,
            snake_turn_penalty: 1.4,
            boost_min_points: 1.2,
            boost_points_cost_per_second: 7.0,
            boost_points_cost_size_factor: 1.1,
            boost_len_loss_per_point: 0.16,
            food_value: 1.0,
            boost_pellet_value_factor: 0.65,
            boost_pellet_jitter: 10.0,
        }
    }

    /// Validate gameplay ranges before any proposal storage changes.
    pub fn validate(self) -> Result<(), MovementError> {
        for (field, value) in [
            ("world_radius", self.world_radius),
            ("snake_base_speed", self.snake_base_speed),
            ("snake_boost_speed", self.snake_boost_speed),
            ("snake_turn_rate", self.snake_turn_rate),
            ("snake_radius", self.snake_radius),
            ("snake_radius_max", self.snake_radius_max),
            ("snake_thickness_scale", self.snake_thickness_scale),
            ("snake_thickness_log_div", self.snake_thickness_log_div),
            ("snake_spacing", self.snake_spacing),
            ("snake_size_speed_penalty", self.snake_size_speed_penalty),
            ("snake_boost_size_penalty", self.snake_boost_size_penalty),
            ("snake_turn_penalty", self.snake_turn_penalty),
            ("boost_min_points", self.boost_min_points),
            (
                "boost_points_cost_per_second",
                self.boost_points_cost_per_second,
            ),
            (
                "boost_points_cost_size_factor",
                self.boost_points_cost_size_factor,
            ),
            ("boost_len_loss_per_point", self.boost_len_loss_per_point),
            ("food_value", self.food_value),
            ("boost_pellet_value_factor", self.boost_pellet_value_factor),
            ("boost_pellet_jitter", self.boost_pellet_jitter),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(MovementError::InvalidConfig { field });
            }
        }
        if self.world_radius <= 0.0
            || self.snake_base_speed <= 0.0
            || self.snake_boost_speed <= 0.0
            || self.snake_turn_rate <= 0.0
            || self.snake_radius <= 0.0
            || self.snake_radius_max < self.snake_radius
            || self.snake_thickness_log_div <= 0.0
            || self.snake_spacing <= 0.0
            || self.snake_size_speed_penalty > 1.0
            || self.snake_boost_size_penalty > 1.0
            || self.snake_start_len == 0
            || self.snake_min_len == 0
            || self.snake_min_len > self.snake_start_len
            || self.snake_start_len > self.snake_max_len
        {
            return Err(MovementError::InvalidConfig {
                field: "movement ranges",
            });
        }
        Ok(())
    }
}

/// One deterministic boost-trail pellet request before RNG jitter is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoostDropRequest {
    /// Stable snake identity whose gameplay RNG stream owns the later draw.
    pub owner_id: u64,
    /// Tail-derived pellet center before symmetric jitter.
    pub base_position: WorldPoint,
    /// Positive pellet food value.
    pub value: f64,
    /// Symmetric per-axis jitter bound.
    pub jitter: f64,
}

/// Diagnostic metadata for one staged snake movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementProposal {
    /// Stable source-array index; never used as public identity.
    pub snake_index: usize,
    /// Stable internal snake identity.
    pub snake_id: u64,
    /// Points consumed by boost during this substep.
    pub boost_points_spent: f64,
    /// Whether the proposed head touched or crossed the arena boundary.
    pub wall_death: bool,
    /// Number of tail points converted into boost requests.
    pub boost_drop_count: usize,
}

/// Retained scratch capacities and current staged sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MovementCapacityDiagnostics {
    /// Current snake proposal count.
    pub snakes: usize,
    /// Current packed body-point count.
    pub body_points: usize,
    /// Current boost-drop request count.
    pub boost_drops: usize,
    /// Retained stable-order capacity.
    pub order_capacity: usize,
    /// Retained staged snake capacity.
    pub snake_capacity: usize,
    /// Retained desired-length capacity.
    pub desired_length_capacity: usize,
    /// Retained proposal capacity.
    pub proposal_capacity: usize,
    /// Retained packed body-point capacity.
    pub body_point_capacity: usize,
    /// Retained boost-drop capacity.
    pub boost_drop_capacity: usize,
}

/// Immutable view of one successfully prepared movement phase.
#[derive(Clone, Copy, Debug)]
pub struct PreparedMovement<'scratch, 'world> {
    source_world: &'world WorldState,
    snakes: &'scratch [SnakeState],
    body_points: &'scratch [WorldPoint],
    proposals: &'scratch [MovementProposal],
    boost_drops: &'scratch [BoostDropRequest],
    diagnostics: MovementCapacityDiagnostics,
}

impl<'scratch, 'world> PreparedMovement<'scratch, 'world> {
    /// Immutable authoritative boundary from which movement was prepared.
    #[must_use]
    pub const fn source_world(self) -> &'world WorldState {
        self.source_world
    }

    /// Staged snake records in the source container's shape.
    #[must_use]
    pub const fn snakes(self) -> &'scratch [SnakeState] {
        self.snakes
    }

    /// Complete staged packed body storage, ordered by stable snake ID.
    #[must_use]
    pub const fn body_points(self) -> &'scratch [WorldPoint] {
        self.body_points
    }

    /// Movement metadata ordered by stable snake ID.
    #[must_use]
    pub const fn proposals(self) -> &'scratch [MovementProposal] {
        self.proposals
    }

    /// Boost-tail requests ordered by stable snake ID then tail-to-head removal.
    #[must_use]
    pub const fn boost_drops(self) -> &'scratch [BoostDropRequest] {
        self.boost_drops
    }

    /// Current sizes and retained capacities.
    #[must_use]
    pub const fn diagnostics(self) -> MovementCapacityDiagnostics {
        self.diagnostics
    }

    /// Resolve one staged body without assuming snake-array order.
    #[must_use]
    pub fn body_for(self, snake: &SnakeState) -> Option<&'scratch [WorldPoint]> {
        let end = snake.body.start.checked_add(snake.body.len)?;
        self.body_points.get(snake.body.start..end)
    }
}

/// Reusable staging storage for movement and packed body construction.
#[derive(Clone, Debug, Default)]
pub struct MovementWorkspace {
    order: Vec<usize>,
    next_snakes: Vec<SnakeState>,
    desired_lengths: Vec<usize>,
    proposals: Vec<MovementProposal>,
    next_body_points: Vec<WorldPoint>,
    boost_drops: Vec<BoostDropRequest>,
    ready: bool,
}

impl MovementWorkspace {
    /// Construct empty reusable movement scratch.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            order: Vec::new(),
            next_snakes: Vec::new(),
            desired_lengths: Vec::new(),
            proposals: Vec::new(),
            next_body_points: Vec::new(),
            boost_drops: Vec::new(),
            ready: false,
        }
    }

    /// Prepare every snake's scalar and body result without mutating authority.
    ///
    /// `maximum_body_points` and `maximum_pellets` are the admitted engine
    /// capacities. Boost requests are bounded here, while the post-food phase
    /// checks their combined count with the pellets that survived claims. Food
    /// and collision phases may inspect this view, but only a later complete
    /// transaction may swap it into authority.
    pub fn prepare<'scratch, 'world>(
        &'scratch mut self,
        world: &'world WorldState,
        config: MovementConfig,
        dt: f64,
        maximum_body_points: usize,
        maximum_pellets: usize,
    ) -> Result<PreparedMovement<'scratch, 'world>, MovementError> {
        self.ready = false;
        self.order.clear();
        self.next_snakes.clear();
        self.desired_lengths.clear();
        self.proposals.clear();
        self.next_body_points.clear();
        self.boost_drops.clear();
        config.validate()?;
        if !dt.is_finite() || dt <= 0.0 {
            return Err(MovementError::InvalidDelta(dt));
        }
        if maximum_body_points == 0 || maximum_pellets == 0 {
            return Err(MovementError::InvalidCapacity);
        }
        if world.body_points.len() > maximum_body_points {
            return Err(MovementError::BodyCapacityExceeded {
                required: world.body_points.len(),
                maximum: maximum_body_points,
            });
        }
        if world.pellets.len() > maximum_pellets {
            return Err(MovementError::PelletCapacityExceeded {
                required: world.pellets.len(),
                maximum: maximum_pellets,
            });
        }

        reserve_for(&mut self.order, world.snakes.len(), "stable snake order")?;
        reserve_for(
            &mut self.next_snakes,
            world.snakes.len(),
            "staged snake records",
        )?;
        reserve_for(
            &mut self.desired_lengths,
            world.snakes.len(),
            "staged body lengths",
        )?;
        reserve_for(
            &mut self.proposals,
            world.snakes.len(),
            "movement proposals",
        )?;
        self.order.extend(0..world.snakes.len());
        self.order
            .sort_unstable_by_key(|index| world.snakes[*index].id);
        for pair in self.order.windows(2) {
            if world.snakes[pair[0]].id == world.snakes[pair[1]].id {
                return Err(MovementError::DuplicateSnakeId(world.snakes[pair[0]].id));
            }
        }
        self.next_snakes.extend(world.snakes.iter().cloned());
        self.desired_lengths.resize(world.snakes.len(), 0);

        let mut total_body_points = 0usize;
        for &snake_index in &self.order {
            let source = &world.snakes[snake_index];
            let body = body_slice(world, source)?;
            validate_snake(source, body)?;
            let mut staged = source.clone();
            let mut desired_length = body.len();
            let mut boost_points_spent = 0.0;
            let mut wall_death = false;
            let boost_drop_start = self.boost_drops.len();

            if source.alive {
                let boost_size = size_normalized(body.len(), config);
                if source.input_boost
                    && body.len()
                        > config.snake_min_len.checked_add(1).ok_or(
                            MovementError::ArithmeticOverflow {
                                context: "minimum boost body length",
                            },
                        )?
                    && source.points >= config.boost_min_points
                {
                    let cost_rate = config.boost_points_cost_per_second
                        * (1.0 + config.boost_points_cost_size_factor * boost_size);
                    boost_points_spent = source.points.min(cost_rate * dt);
                    if boost_points_spent > 0.0 {
                        staged.points -= boost_points_spent;
                        staged.target_length = clamp(
                            staged.target_length
                                - boost_points_spent * config.boost_len_loss_per_point,
                            config.snake_min_len as f64,
                            config.snake_max_len as f64,
                        );
                        staged.boost = true;
                    } else {
                        staged.boost = false;
                    }
                } else {
                    staged.boost = false;
                }
                let boost_desired_length = desired_body_length(staged.target_length, config);
                if staged.boost {
                    stage_boost_drops(
                        &mut self.boost_drops,
                        source.id,
                        body,
                        boost_desired_length,
                        config,
                        maximum_pellets,
                    )?;
                    desired_length = body.len().min(boost_desired_length);
                }

                // Boost burn removes tail points before TypeScript calculates
                // this substep's size-dependent speed and turn. Food claims
                // must resolve before ordinary target shrink/growth, so that
                // final body-length work belongs to the next transaction phase.
                let movement_size = size_normalized(desired_length, config);
                let base_speed = config.snake_base_speed
                    * (1.0 - config.snake_size_speed_penalty * movement_size);
                let boost_ratio =
                    config.snake_boost_speed / config.snake_base_speed.max(MIN_DISTANCE);
                let boost_addition = (boost_ratio - 1.0).max(0.0)
                    * (1.0 - config.snake_boost_size_penalty * movement_size);
                let boost_speed = base_speed * (1.0 + boost_addition.max(0.0));
                let target_speed = if staged.boost {
                    boost_speed
                } else {
                    base_speed
                };
                staged.speed = source.speed
                    + (target_speed - source.speed) * (1.0 - (-dt * SPEED_RESPONSE).exp());
                let turn_rate =
                    config.snake_turn_rate / (1.0 + config.snake_turn_penalty * movement_size);
                staged.previous_position = source.position;
                staged.direction =
                    normalize_angle(source.direction + f64::from(source.turn) * turn_rate * dt);
                staged.position = WorldPoint {
                    x: source.position.x + staged.direction.cos() * staged.speed * dt,
                    y: source.position.y + staged.direction.sin() * staged.speed * dt,
                };
                wall_death = staged.position.x.hypot(staged.position.y) + source.radius
                    >= config.world_radius;
                if wall_death {
                    staged.alive = false;
                    // Boost shrink already happened, but TypeScript returns at
                    // the wall before food or ordinary target growth/shrink.
                }
            }

            total_body_points = total_body_points.checked_add(desired_length).ok_or(
                MovementError::ArithmeticOverflow {
                    context: "staged body-point total",
                },
            )?;
            if total_body_points > maximum_body_points {
                return Err(MovementError::BodyCapacityExceeded {
                    required: total_body_points,
                    maximum: maximum_body_points,
                });
            }
            self.desired_lengths[snake_index] = desired_length;
            self.next_snakes[snake_index] = staged;
            self.proposals.push(MovementProposal {
                snake_index,
                snake_id: source.id,
                boost_points_spent,
                wall_death,
                boost_drop_count: self.boost_drops.len() - boost_drop_start,
            });
        }

        reserve_for(
            &mut self.next_body_points,
            total_body_points,
            "staged packed body points",
        )?;
        for &snake_index in &self.order {
            let source = &world.snakes[snake_index];
            let old_body = body_slice(world, source)?;
            let desired = self.desired_lengths[snake_index];
            let body_start = self.next_body_points.len();
            let retained = desired.min(old_body.len());
            self.next_body_points
                .extend_from_slice(&old_body[..retained]);
            let staged = &mut self.next_snakes[snake_index];
            if source.alive {
                let Some(head) = self.next_body_points.get_mut(body_start) else {
                    return Err(MovementError::InvalidBodyRange {
                        snake_id: source.id,
                    });
                };
                // TypeScript returns from wall death after advancing scalar
                // x/y but before rewriting points[0]. Rust normalizes that
                // stale internal body head so the staged world retains its
                // admitted `body[0] == position` invariant. Dead snakes do not
                // follow, grow, change radius, render, or collide.
                *head = staged.position;
                if staged.alive {
                    follow_body(
                        &mut self.next_body_points[body_start..body_start + retained],
                        config.snake_spacing,
                    );
                }
            }
            staged.body = BodyRange {
                start: body_start,
                len: desired,
            };
            validate_derived_snake(staged)?;
            for point in &self.next_body_points[body_start..body_start + desired] {
                if !point.x.is_finite() || !point.y.is_finite() {
                    return Err(MovementError::NonFiniteDerived {
                        snake_id: source.id,
                        field: "body point",
                    });
                }
            }
        }
        debug_assert_eq!(self.next_body_points.len(), total_body_points);
        self.ready = true;
        let diagnostics = self.diagnostics();
        Ok(PreparedMovement {
            source_world: world,
            snakes: &self.next_snakes,
            body_points: &self.next_body_points,
            proposals: &self.proposals,
            boost_drops: &self.boost_drops,
            diagnostics,
        })
    }

    /// Current sizes and retained capacities, even after a rejected prepare.
    #[must_use]
    pub fn diagnostics(&self) -> MovementCapacityDiagnostics {
        MovementCapacityDiagnostics {
            snakes: self.next_snakes.len(),
            body_points: self.next_body_points.len(),
            boost_drops: self.boost_drops.len(),
            order_capacity: self.order.capacity(),
            snake_capacity: self.next_snakes.capacity(),
            desired_length_capacity: self.desired_lengths.capacity(),
            proposal_capacity: self.proposals.capacity(),
            body_point_capacity: self.next_body_points.capacity(),
            boost_drop_capacity: self.boost_drops.capacity(),
        }
    }

    /// Whether the most recent preparation reached a complete staged result.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }
}

fn body_slice<'a>(
    world: &'a WorldState,
    snake: &SnakeState,
) -> Result<&'a [WorldPoint], MovementError> {
    let end =
        snake
            .body
            .start
            .checked_add(snake.body.len)
            .ok_or(MovementError::ArithmeticOverflow {
                context: "source body range",
            })?;
    world
        .body_points
        .get(snake.body.start..end)
        .ok_or(MovementError::InvalidBodyRange { snake_id: snake.id })
}

fn validate_snake(snake: &SnakeState, body: &[WorldPoint]) -> Result<(), MovementError> {
    if snake.id == 0 || (snake.alive && body.is_empty()) {
        return Err(MovementError::InvalidBodyRange { snake_id: snake.id });
    }
    if !body.is_empty() && body[0] != snake.position {
        return Err(MovementError::InvalidBodyRange { snake_id: snake.id });
    }
    for value in [
        snake.position.x,
        snake.position.y,
        snake.previous_position.x,
        snake.previous_position.y,
        snake.direction,
        snake.radius,
        snake.speed,
        snake.points,
        snake.target_length,
        f64::from(snake.turn),
    ] {
        if !value.is_finite() {
            return Err(MovementError::InvalidSnakeScalar { snake_id: snake.id });
        }
    }
    if snake.radius <= 0.0
        || snake.speed < 0.0
        || snake.points < 0.0
        || snake.target_length < 0.0
        || !(-1.0..=1.0).contains(&snake.turn)
    {
        return Err(MovementError::InvalidSnakeScalar { snake_id: snake.id });
    }
    if body
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err(MovementError::InvalidBodyPoint { snake_id: snake.id });
    }
    Ok(())
}

fn size_normalized(length: usize, config: MovementConfig) -> f64 {
    let denominator = config
        .snake_max_len
        .saturating_sub(config.snake_start_len)
        .max(1) as f64;
    clamp(
        length.saturating_sub(config.snake_start_len) as f64 / denominator,
        0.0,
        1.0,
    )
}

pub(crate) fn desired_body_length(target: f64, config: MovementConfig) -> usize {
    clamp(
        target,
        config.snake_min_len as f64,
        config.snake_max_len as f64,
    )
    .floor() as usize
}

pub(crate) fn radius_for_length(length: usize, config: MovementConfig) -> f64 {
    let growth = length.saturating_sub(config.snake_start_len) as f64;
    let radius = config.snake_radius
        + config.snake_thickness_scale
            * (growth / config.snake_thickness_log_div.max(MIN_DISTANCE)).ln_1p();
    clamp(radius, config.snake_radius, config.snake_radius_max)
}

fn stage_boost_drops(
    output: &mut Vec<BoostDropRequest>,
    owner_id: u64,
    body: &[WorldPoint],
    desired: usize,
    config: MovementConfig,
    maximum_pellets: usize,
) -> Result<(), MovementError> {
    let drop_count = body.len().saturating_sub(desired);
    let required =
        output
            .len()
            .checked_add(drop_count)
            .ok_or(MovementError::ArithmeticOverflow {
                context: "boost drop request total",
            })?;
    if required > maximum_pellets {
        return Err(MovementError::PelletCapacityExceeded {
            required,
            maximum: maximum_pellets,
        });
    }
    reserve_additional(output, drop_count, "boost drop requests")?;
    let value = (config.food_value * config.boost_pellet_value_factor).max(0.02);
    if desired >= body.len() {
        return Ok(());
    }
    for current_len in (desired + 1..=body.len()).rev() {
        let tail = body[current_len - 1];
        let back = if current_len >= 2 {
            body[current_len - 2]
        } else {
            tail
        };
        let dx = tail.x - back.x;
        let dy = tail.y - back.y;
        let distance = distance_or_epsilon(dx, dy);
        let request = BoostDropRequest {
            owner_id,
            base_position: WorldPoint {
                x: tail.x + dx / distance * BOOST_TRAIL_OFFSET,
                y: tail.y + dy / distance * BOOST_TRAIL_OFFSET,
            },
            value,
            jitter: config.boost_pellet_jitter,
        };
        if !request.base_position.x.is_finite()
            || !request.base_position.y.is_finite()
            || !request.value.is_finite()
            || !request.jitter.is_finite()
        {
            return Err(MovementError::NonFiniteDerived {
                snake_id: owner_id,
                field: "boost drop",
            });
        }
        output.push(request);
    }
    Ok(())
}

fn follow_body(body: &mut [WorldPoint], spacing: f64) {
    for index in 1..body.len() {
        let previous = body[index - 1];
        let current = &mut body[index];
        let dx = current.x - previous.x;
        let dy = current.y - previous.y;
        let distance = distance_or_epsilon(dx, dy);
        let adjustment = (distance - spacing) / distance;
        current.x -= dx * adjustment;
        current.y -= dy * adjustment;
    }
}

fn normalize_angle(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut normalized = angle % tau;
    if normalized > std::f64::consts::PI {
        normalized -= tau;
    }
    if normalized < -std::f64::consts::PI {
        normalized += tau;
    }
    normalized
}

fn distance_or_epsilon(dx: f64, dy: f64) -> f64 {
    let distance = dx.hypot(dy);
    if distance == 0.0 {
        MIN_DISTANCE
    } else {
        distance
    }
}

fn validate_derived_snake(snake: &SnakeState) -> Result<(), MovementError> {
    for (field, value) in [
        ("position.x", snake.position.x),
        ("position.y", snake.position.y),
        ("previous_position.x", snake.previous_position.x),
        ("previous_position.y", snake.previous_position.y),
        ("direction", snake.direction),
        ("radius", snake.radius),
        ("speed", snake.speed),
        ("points", snake.points),
        ("target_length", snake.target_length),
    ] {
        if !value.is_finite() {
            return Err(MovementError::NonFiniteDerived {
                snake_id: snake.id,
                field,
            });
        }
    }
    Ok(())
}

fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.max(minimum).min(maximum)
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), MovementError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| MovementError::AllocationFailed { context, required })?;
    }
    Ok(())
}

fn reserve_additional<T>(
    values: &mut Vec<T>,
    additional: usize,
    context: &'static str,
) -> Result<(), MovementError> {
    let required = values
        .len()
        .checked_add(additional)
        .ok_or(MovementError::ArithmeticOverflow { context })?;
    reserve_for(values, required, context)
}

/// Rejected configuration, source state, capacity, or movement preparation.
#[derive(Clone, Debug, PartialEq)]
pub enum MovementError {
    /// One named movement configuration field is invalid.
    InvalidConfig { field: &'static str },
    /// The collision substep delta is non-positive or non-finite.
    InvalidDelta(f64),
    /// Admitted body or pellet capacity is zero.
    InvalidCapacity,
    /// Stable snake identity is duplicated.
    DuplicateSnakeId(u64),
    /// A pooled body range is empty/incoherent/out of bounds.
    InvalidBodyRange { snake_id: u64 },
    /// One authoritative body coordinate is non-finite.
    InvalidBodyPoint { snake_id: u64 },
    /// One authoritative snake scalar is non-finite or outside its range.
    InvalidSnakeScalar { snake_id: u64 },
    /// Finite inputs overflowed while deriving a staged scalar or request.
    NonFiniteDerived { snake_id: u64, field: &'static str },
    /// Checked arithmetic failed.
    ArithmeticOverflow { context: &'static str },
    /// Complete staged body storage exceeds its admitted capacity.
    BodyCapacityExceeded { required: usize, maximum: usize },
    /// Boost requests plus current pellets exceed admitted storage.
    PelletCapacityExceeded { required: usize, maximum: usize },
    /// Reusable scratch could not be reserved.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
}

impl Display for MovementError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => write!(formatter, "invalid movement config: {field}"),
            Self::InvalidDelta(value) => write!(formatter, "invalid movement delta {value}"),
            Self::InvalidCapacity => write!(formatter, "movement capacities must be positive"),
            Self::DuplicateSnakeId(id) => write!(formatter, "duplicate movement snake ID {id}"),
            Self::InvalidBodyRange { snake_id } => {
                write!(
                    formatter,
                    "invalid movement body range for snake {snake_id}"
                )
            }
            Self::InvalidBodyPoint { snake_id } => {
                write!(
                    formatter,
                    "invalid movement body point for snake {snake_id}"
                )
            }
            Self::InvalidSnakeScalar { snake_id } => {
                write!(formatter, "invalid movement scalar for snake {snake_id}")
            }
            Self::NonFiniteDerived { snake_id, field } => write!(
                formatter,
                "movement derived non-finite {field} for snake {snake_id}"
            ),
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "movement arithmetic overflow while calculating {context}"
                )
            }
            Self::BodyCapacityExceeded { required, maximum } => write!(
                formatter,
                "movement needs {required} body points, exceeding maximum {maximum}"
            ),
            Self::PelletCapacityExceeded { required, maximum } => write!(
                formatter,
                "movement needs {required} pellets, exceeding maximum {maximum}"
            ),
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "movement could not reserve {required} entries for {context}"
            ),
        }
    }
}

impl Error for MovementError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{SnakeKind, WorldState};

    const DT: f64 = 1.0 / 180.0;

    fn snake(id: u64, body_start: usize, length: usize) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(u32::try_from(id - 1).expect("fixture slot should fit")),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position: WorldPoint { x: 0.0, y: 0.0 },
            previous_position: WorldPoint { x: 0.0, y: 0.0 },
            direction: 0.0,
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
                start: body_start,
                len: length,
            },
            skin: 0,
        }
    }

    fn line_body(length: usize, offset_y: f64) -> Vec<WorldPoint> {
        (0..length)
            .map(|index| WorldPoint {
                x: -(index as f64) * 7.5,
                y: offset_y,
            })
            .collect()
    }

    fn world_with_one(length: usize) -> WorldState {
        WorldState {
            snakes: vec![snake(1, 0, length)],
            body_points: line_body(length, 0.0),
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        }
    }

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 2.0e-12,
            "actual {actual:.17} differs from expected {expected:.17}"
        );
    }

    #[test]
    fn ordinary_turn_and_body_match_retained_typescript_substep() {
        let mut world = world_with_one(5);
        world.snakes[0].turn = 1.0;
        let authority_before = world.clone();
        let mut workspace = MovementWorkspace::new();
        let prepared = workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("ordinary movement should prepare");
        let staged = &prepared.snakes()[0];
        let body = prepared
            .body_for(staged)
            .expect("staged body should be addressable");

        close(staged.position.x, 0.916_521_814_514_685);
        close(staged.position.y, 0.016_295_437_904_130_29);
        close(staged.direction, 0.017_777_777_777_777_778);
        close(staged.speed, 165.0);
        close(body[1].x, -6.583_464_128_350_458_5);
        close(body[1].y, 0.001_774_527_971_592_294_4);
        close(body[4].x, -29.083_463_959_652_295);
        close(body[4].y, 0.000_002_291_560_545_631_536);
        assert!(!staged.boost);
        assert_eq!(prepared.boost_drops(), &[]);
        assert_eq!(world, authority_before);
    }

    #[test]
    fn boost_burn_shrink_and_body_match_pre_food_typescript_order() {
        let mut world = world_with_one(8);
        world.snakes[0].turn = -0.5;
        world.snakes[0].input_boost = true;
        let mut workspace = MovementWorkspace::new();
        let prepared = workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("boost movement should prepare");
        let staged = &prepared.snakes()[0];
        let body = prepared.body_for(staged).expect("boost body should exist");

        close(staged.position.x, 0.982_628_430_314_376_3);
        close(staged.position.y, -0.008_732_258_602_803_35);
        close(staged.direction, -0.008_886_399_452_323_957);
        close(staged.speed, 176.880_101_352_972_45);
        close(staged.points, 9.961_098_271_357_901);
        close(staged.target_length, 7.993_775_723_417_264);
        // Radius is updated only after food and final target-length resolution.
        close(staged.radius, 9.0);
        assert!(staged.boost);
        assert_eq!(body.len(), 7);
        close(body[6].x, -44.017_367_541_676_194);
        close(body[6].y, -0.000_000_021_100_342_633_887_367);
        assert_eq!(prepared.proposals()[0].boost_drop_count, 1);
        assert_eq!(prepared.boost_drops().len(), 1);
        let drop = prepared.boost_drops()[0];
        assert_eq!(drop.owner_id, 1);
        close(drop.base_position.x, -60.5);
        close(drop.base_position.y, 0.0);
        close(drop.value, 0.65);
        close(drop.jitter, 10.0);
    }

    #[test]
    fn ordinary_target_shrink_waits_for_food_without_creating_boost_pellets() {
        let mut world = world_with_one(5);
        world.snakes[0].target_length = 4.0;
        let mut workspace = MovementWorkspace::new();
        let prepared = workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("ordinary shrink should prepare");

        assert!(!prepared.snakes()[0].boost);
        assert_eq!(prepared.snakes()[0].body.len, 5);
        assert!(prepared.boost_drops().is_empty());
    }

    #[test]
    fn boost_eligibility_uses_exact_length_and_points_boundaries() {
        let mut short = world_with_one(5);
        short.snakes[0].input_boost = true;
        short.snakes[0].points = 100.0;
        let mut short_workspace = MovementWorkspace::new();
        let short_result = short_workspace
            .prepare(&short, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("short snake should still move");
        assert!(!short_result.snakes()[0].boost);
        assert!(short_result.boost_drops().is_empty());

        let mut below_points = world_with_one(6);
        below_points.snakes[0].input_boost = true;
        below_points.snakes[0].points = 1.2_f64.next_down();
        let mut below_workspace = MovementWorkspace::new();
        let below_result = below_workspace
            .prepare(
                &below_points,
                MovementConfig::typescript_defaults(),
                DT,
                100,
                100,
            )
            .expect("below-threshold snake should move");
        assert!(!below_result.snakes()[0].boost);

        let mut exact = world_with_one(6);
        exact.snakes[0].input_boost = true;
        exact.snakes[0].points = 1.2;
        let mut exact_workspace = MovementWorkspace::new();
        let exact_result = exact_workspace
            .prepare(&exact, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("exact-threshold snake should boost");
        assert!(exact_result.snakes()[0].boost);
        assert_eq!(exact_result.boost_drops().len(), 1);
    }

    #[test]
    fn multiple_boost_drops_follow_typescript_tail_pop_order() {
        let mut world = world_with_one(10);
        world.snakes[0].input_boost = true;
        world.snakes[0].points = 100.0;
        let mut workspace = MovementWorkspace::new();
        let prepared = workspace
            .prepare(&world, MovementConfig::typescript_defaults(), 1.0, 100, 100)
            .expect("multi-drop boost should prepare");

        assert_eq!(prepared.snakes()[0].body.len, 8);
        assert_eq!(prepared.boost_drops().len(), 2);
        close(prepared.boost_drops()[0].base_position.x, -75.5);
        close(prepared.boost_drops()[1].base_position.x, -68.0);
    }

    #[test]
    fn body_following_uses_epsilon_only_for_exact_zero_distance() {
        let mut sub_micro = [
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: 1.0e-9, y: 0.0 },
        ];
        follow_body(&mut sub_micro, 7.5);
        close(sub_micro[1].x, 7.5);

        let mut exact_zero = [WorldPoint { x: 0.0, y: 0.0 }, WorldPoint { x: 0.0, y: 0.0 }];
        follow_body(&mut exact_zero, 7.5);
        assert_eq!(exact_zero[1], WorldPoint { x: 0.0, y: 0.0 });
    }

    #[test]
    fn stable_id_order_makes_reversed_source_containers_equivalent() {
        fn fixture(reverse: bool) -> WorldState {
            let mut first_body = line_body(5, 0.0);
            let mut second_body = line_body(5, 100.0);
            for point in &mut second_body {
                point.x += 20.0;
            }
            let mut first = snake(1, 0, 5);
            first.turn = 0.5;
            let mut second = snake(2, 5, 5);
            second.position = second_body[0];
            second.previous_position = second.position;
            second.turn = -0.75;
            if !reverse {
                first_body.extend(second_body);
                WorldState {
                    snakes: vec![first, second],
                    body_points: first_body,
                    pellets: Vec::new(),
                    controller_leases: Vec::new(),
                }
            } else {
                second.body.start = 0;
                first.body.start = 5;
                second_body.extend(first_body);
                WorldState {
                    snakes: vec![second, first],
                    body_points: second_body,
                    pellets: Vec::new(),
                    controller_leases: Vec::new(),
                }
            }
        }

        fn normalized(
            prepared: PreparedMovement<'_, '_>,
        ) -> Vec<(u64, SnakeState, Vec<WorldPoint>)> {
            let mut output = prepared
                .snakes()
                .iter()
                .map(|snake| {
                    let mut record = snake.clone();
                    let body = prepared
                        .body_for(snake)
                        .expect("body should resolve")
                        .to_vec();
                    record.body.start = 0;
                    (snake.id, record, body)
                })
                .collect::<Vec<_>>();
            output.sort_by_key(|entry| entry.0);
            output
        }

        let forward = fixture(false);
        let reversed = fixture(true);
        let mut forward_workspace = MovementWorkspace::new();
        let mut reverse_workspace = MovementWorkspace::new();
        let forward_prepared = forward_workspace
            .prepare(
                &forward,
                MovementConfig::typescript_defaults(),
                DT,
                100,
                100,
            )
            .expect("forward fixture should prepare");
        let reverse_prepared = reverse_workspace
            .prepare(
                &reversed,
                MovementConfig::typescript_defaults(),
                DT,
                100,
                100,
            )
            .expect("reversed fixture should prepare");
        assert_eq!(normalized(forward_prepared), normalized(reverse_prepared));
        assert_eq!(
            forward_prepared
                .proposals()
                .iter()
                .map(|proposal| proposal.snake_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            reverse_prepared
                .proposals()
                .iter()
                .map(|proposal| proposal.snake_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn capacity_and_malformed_state_fail_without_authoritative_writes() {
        let mut world = world_with_one(8);
        let authority_before = world.clone();
        let mut workspace = MovementWorkspace::new();
        assert!(matches!(
            workspace.prepare(&world, MovementConfig::typescript_defaults(), DT, 7, 100),
            Err(MovementError::BodyCapacityExceeded {
                required: 8,
                maximum: 7
            })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(world, authority_before);

        world.body_points[0].x = f64::NAN;
        assert!(matches!(
            workspace.prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100),
            Err(MovementError::InvalidBodyRange { snake_id: 1 })
                | Err(MovementError::InvalidBodyPoint { snake_id: 1 })
        ));
        assert!(!workspace.is_ready());
    }

    #[test]
    fn finite_extreme_inputs_cannot_publish_non_finite_staging() {
        let world = world_with_one(5);
        let authority_before = world.clone();
        let mut config = MovementConfig::typescript_defaults();
        config.world_radius = f64::MAX;
        config.snake_base_speed = f64::MAX;
        config.snake_boost_speed = f64::MAX;
        let mut workspace = MovementWorkspace::new();

        assert!(matches!(
            workspace.prepare(&world, config, f64::MAX, 100, 100),
            Err(MovementError::NonFiniteDerived { snake_id: 1, .. })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(world, authority_before);
    }

    #[test]
    fn warmed_preparation_reuses_every_capacity() {
        let mut world = world_with_one(8);
        world.snakes[0].input_boost = true;
        let mut workspace = MovementWorkspace::new();
        let first = workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("warm preparation should succeed")
            .diagnostics();
        for _ in 0..24 {
            let next = workspace
                .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
                .expect("reused preparation should succeed")
                .diagnostics();
            assert_eq!(next.order_capacity, first.order_capacity);
            assert_eq!(next.snake_capacity, first.snake_capacity);
            assert_eq!(next.desired_length_capacity, first.desired_length_capacity);
            assert_eq!(next.proposal_capacity, first.proposal_capacity);
            assert_eq!(next.body_point_capacity, first.body_point_capacity);
            assert_eq!(next.boost_drop_capacity, first.boost_drop_capacity);
        }
    }

    #[test]
    fn wall_contact_is_staged_as_death_without_mutating_authority() {
        let mut world = world_with_one(5);
        world.snakes[0].position = WorldPoint { x: 3_490.2, y: 0.0 };
        world.snakes[0].previous_position = world.snakes[0].position;
        world.snakes[0].target_length = 10.0;
        for (index, point) in world.body_points.iter_mut().enumerate() {
            point.x = 3_490.2 - index as f64 * 7.5;
        }
        let authority_before = world.clone();
        let mut workspace = MovementWorkspace::new();
        let prepared = workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("wall movement should stage");
        assert!(prepared.proposals()[0].wall_death);
        assert!(!prepared.snakes()[0].alive);
        assert_eq!(prepared.snakes()[0].body.len, 5);
        let staged_body = prepared
            .body_for(&prepared.snakes()[0])
            .expect("wall body should remain addressable");
        assert_eq!(staged_body[0], prepared.snakes()[0].position);
        assert_eq!(&staged_body[1..], &authority_before.body_points[1..]);
        assert_eq!(world, authority_before);
    }
}
