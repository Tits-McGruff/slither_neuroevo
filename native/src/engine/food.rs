//! Deterministic food claims and post-food body-length finalization.
//!
//! TypeScript removes pellets while visiting snakes in container order. This
//! stage instead reads one immutable pellet boundary, chooses one winner for
//! every contested pellet by distance then stable snake ID, and applies all
//! gains in stable pellet-ID order. It consumes staged movement rather than
//! authority and publishes only reusable scratch for a later complete physics
//! transaction.

use super::movement::{
    desired_body_length, radius_for_length, BoostDropRequest, MovementConfig, MovementProposal,
    PreparedMovement,
};
use super::spatial::{
    IndexedPelletWorld, PelletQueryScratch, SpatialIndexError, SpatialQueryDiagnostics,
};
use super::state::{BodyRange, PelletState, SnakeState, WorldPoint, WorldState};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt::{Display, Formatter};

const MIN_DISTANCE: f64 = 1.0e-6;

/// Food/scoring values projected from one admitted settings revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoodConfig {
    /// Extra radius beyond the current snake radius that can consume food.
    pub eat_radius_padding: f64,
    /// Score added per unit pellet value.
    pub points_per_food: f64,
    /// Target body length added per unit pellet value.
    pub growth_per_food: f64,
}

impl FoodConfig {
    /// Current TypeScript defaults retained as a named comparison fixture.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            eat_radius_padding: 6.0,
            points_per_food: 20.0,
            growth_per_food: 1.0,
        }
    }

    fn validate(self) -> Result<(), FoodError> {
        for (field, value) in [
            ("eat_radius_padding", self.eat_radius_padding),
            ("points_per_food", self.points_per_food),
            ("growth_per_food", self.growth_per_food),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(FoodError::InvalidConfig { field });
            }
        }
        Ok(())
    }
}

/// One immutable contested-pellet outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoodClaim {
    /// Stable pellet identity.
    pub pellet_id: u64,
    /// Stable winning snake identity.
    pub snake_id: u64,
    /// Source snake-array index; never a public identity.
    pub snake_index: usize,
    /// Source pellet-array index; valid only for the bound source world.
    pub pellet_source_index: usize,
    /// Exact squared head-to-pellet distance used for winner selection.
    pub distance_squared: f64,
    /// Positive food value applied by this claim.
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FoodWinner {
    snake_id: u64,
    snake_index: usize,
    distance_squared: f64,
}

/// Retained scratch capacities and current post-food sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoodCapacityDiagnostics {
    /// Number of staged snake records.
    pub snakes: usize,
    /// Number of final packed body points.
    pub body_points: usize,
    /// Number of post-movement body points retained for swept collisions.
    pub movement_body_points: usize,
    /// Number of consumed pellets.
    pub claims: usize,
    /// Number of source pellets not consumed.
    pub remaining_pellets: usize,
    /// Number of staged pre-jitter boost drops carried forward.
    pub boost_drops: usize,
    /// Retained stable snake-order capacity.
    pub snake_order_capacity: usize,
    /// Retained stable pellet-order capacity.
    pub pellet_order_capacity: usize,
    /// Retained winner-slot capacity.
    pub winner_capacity: usize,
    /// Retained snake-record capacity.
    pub snake_capacity: usize,
    /// Retained final-length capacity.
    pub final_length_capacity: usize,
    /// Retained packed-body capacity.
    pub body_point_capacity: usize,
    /// Retained post-movement body-range capacity.
    pub movement_body_range_capacity: usize,
    /// Retained post-movement packed-body capacity.
    pub movement_body_point_capacity: usize,
    /// Retained post-movement radius capacity.
    pub movement_radius_capacity: usize,
    /// Retained claim capacity.
    pub claim_capacity: usize,
    /// Retained remaining-pellet capacity.
    pub remaining_pellet_capacity: usize,
    /// Retained boost-drop capacity.
    pub boost_drop_capacity: usize,
    /// Retained movement-proposal capacity carried into later physics phases.
    pub movement_proposal_capacity: usize,
    /// Retained complete pellet-query candidate capacity.
    pub query_candidate_capacity: usize,
}

/// Immutable view of one completely prepared food/final-body phase.
#[derive(Clone, Copy, Debug)]
pub struct PreparedFood<'scratch, 'world> {
    source_world: &'world WorldState,
    snakes: &'scratch [SnakeState],
    body_points: &'scratch [WorldPoint],
    movement_body_ranges: &'scratch [BodyRange],
    movement_body_points: &'scratch [WorldPoint],
    movement_radii: &'scratch [f64],
    remaining_pellets: &'scratch [PelletState],
    claims: &'scratch [FoodClaim],
    boost_drops: &'scratch [BoostDropRequest],
    movement_proposals: &'scratch [MovementProposal],
    query_diagnostics: SpatialQueryDiagnostics,
    diagnostics: FoodCapacityDiagnostics,
}

impl<'scratch, 'world> PreparedFood<'scratch, 'world> {
    /// Immutable authoritative boundary from which this food phase was prepared.
    #[must_use]
    pub const fn source_world(self) -> &'world WorldState {
        self.source_world
    }

    /// Staged snake records in the source container's shape.
    #[must_use]
    pub const fn snakes(self) -> &'scratch [SnakeState] {
        self.snakes
    }

    /// Complete final packed body storage in stable snake-ID order.
    #[must_use]
    pub const fn body_points(self) -> &'scratch [WorldPoint] {
        self.body_points
    }

    /// Post-movement, pre-food body storage retained for continuous collisions.
    #[must_use]
    pub const fn movement_body_points(self) -> &'scratch [WorldPoint] {
        self.movement_body_points
    }

    /// Source pellets not consumed, ordered by stable pellet ID.
    #[must_use]
    pub const fn remaining_pellets(self) -> &'scratch [PelletState] {
        self.remaining_pellets
    }

    /// Winning food claims ordered by stable pellet ID.
    #[must_use]
    pub const fn claims(self) -> &'scratch [FoodClaim] {
        self.claims
    }

    /// Deterministic pre-jitter boost requests carried from movement.
    #[must_use]
    pub const fn boost_drops(self) -> &'scratch [BoostDropRequest] {
        self.boost_drops
    }

    /// Stable-ID movement metadata retained for later collision and commit phases.
    #[must_use]
    pub const fn movement_proposals(self) -> &'scratch [MovementProposal] {
        self.movement_proposals
    }

    /// Aggregate complete-query work across every eligible head.
    #[must_use]
    pub const fn query_diagnostics(self) -> SpatialQueryDiagnostics {
        self.query_diagnostics
    }

    /// Current sizes and retained capacities.
    #[must_use]
    pub const fn diagnostics(self) -> FoodCapacityDiagnostics {
        self.diagnostics
    }

    /// Resolve one final staged body without assuming snake-array order.
    #[must_use]
    pub fn body_for(self, snake: &SnakeState) -> Option<&'scratch [WorldPoint]> {
        let end = snake.body.start.checked_add(snake.body.len)?;
        self.body_points.get(snake.body.start..end)
    }

    /// Resolve one post-movement body by source-array index.
    ///
    /// Boost-removed tail points are already absent here. Ordinary target
    /// shrink happens later during food finalization, so collision detection
    /// uses this retained geometry to sweep those disappearing segments up to
    /// (but not including) the final boundary.
    #[must_use]
    pub fn movement_body_for_index(self, snake_index: usize) -> Option<&'scratch [WorldPoint]> {
        let range = *self.movement_body_ranges.get(snake_index)?;
        let end = range.start.checked_add(range.len)?;
        self.movement_body_points.get(range.start..end)
    }

    /// Post-movement, pre-food radius for one source-array index.
    #[must_use]
    pub fn movement_radius_for_index(self, snake_index: usize) -> Option<f64> {
        self.movement_radii.get(snake_index).copied()
    }
}

/// Reusable two-phase contested-food and final-body scratch.
#[derive(Clone, Debug, Default)]
pub struct FoodWorkspace {
    snake_order: Vec<usize>,
    pellet_order: Vec<usize>,
    winners: Vec<Option<FoodWinner>>,
    next_snakes: Vec<SnakeState>,
    final_lengths: Vec<usize>,
    next_body_points: Vec<WorldPoint>,
    movement_body_ranges: Vec<BodyRange>,
    movement_body_points: Vec<WorldPoint>,
    movement_radii: Vec<f64>,
    remaining_pellets: Vec<PelletState>,
    claims: Vec<FoodClaim>,
    boost_drops: Vec<BoostDropRequest>,
    movement_proposals: Vec<MovementProposal>,
    query_scratch: PelletQueryScratch,
    query_diagnostics: SpatialQueryDiagnostics,
    ready: bool,
}

impl FoodWorkspace {
    /// Construct empty reusable food/body-finalization scratch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            snake_order: Vec::new(),
            pellet_order: Vec::new(),
            winners: Vec::new(),
            next_snakes: Vec::new(),
            final_lengths: Vec::new(),
            next_body_points: Vec::new(),
            movement_body_ranges: Vec::new(),
            movement_body_points: Vec::new(),
            movement_radii: Vec::new(),
            remaining_pellets: Vec::new(),
            claims: Vec::new(),
            boost_drops: Vec::new(),
            movement_proposals: Vec::new(),
            query_scratch: PelletQueryScratch::default(),
            query_diagnostics: SpatialQueryDiagnostics {
                cells_visited: 0,
                entries_visited: 0,
                candidates: 0,
                candidate_limit_reached: false,
            },
            ready: false,
        }
    }

    /// Resolve every food claim and final body length without mutating authority.
    pub fn prepare<'scratch, 'world>(
        &'scratch mut self,
        indexed: &IndexedPelletWorld<'world>,
        movement: PreparedMovement<'_, 'world>,
        movement_config: MovementConfig,
        food_config: FoodConfig,
        maximum_body_points: usize,
        maximum_pellets: usize,
    ) -> Result<PreparedFood<'scratch, 'world>, FoodError> {
        self.clear();
        movement_config.validate().map_err(FoodError::Movement)?;
        food_config.validate()?;
        if maximum_body_points == 0 || maximum_pellets == 0 {
            return Err(FoodError::InvalidCapacity);
        }
        let source = movement.source_world();
        if !std::ptr::eq(indexed.world(), source) {
            return Err(FoodError::SourceWorldMismatch);
        }
        if source.pellets.len() > maximum_pellets {
            return Err(FoodError::PelletCapacityExceeded {
                required: source.pellets.len(),
                maximum: maximum_pellets,
            });
        }
        if source.snakes.len() != movement.snakes().len() {
            return Err(FoodError::MovementShapeMismatch);
        }

        reserve_for(
            &mut self.snake_order,
            source.snakes.len(),
            "stable snake order",
        )?;
        reserve_for(&mut self.next_snakes, source.snakes.len(), "staged snakes")?;
        reserve_for(
            &mut self.final_lengths,
            source.snakes.len(),
            "final body lengths",
        )?;
        reserve_for(
            &mut self.pellet_order,
            source.pellets.len(),
            "stable pellet order",
        )?;
        reserve_for(&mut self.winners, source.pellets.len(), "food winners")?;
        reserve_for(&mut self.claims, source.pellets.len(), "food claims")?;
        reserve_for(
            &mut self.movement_proposals,
            movement.proposals().len(),
            "movement proposals",
        )?;
        self.movement_proposals
            .extend_from_slice(movement.proposals());
        reserve_for(
            &mut self.movement_body_ranges,
            movement.snakes().len(),
            "movement body ranges",
        )?;
        reserve_for(
            &mut self.movement_body_points,
            movement.body_points().len(),
            "movement body points",
        )?;
        self.movement_body_ranges
            .extend(movement.snakes().iter().map(|snake| snake.body));
        self.movement_body_points
            .extend_from_slice(movement.body_points());
        reserve_for(
            &mut self.movement_radii,
            movement.snakes().len(),
            "movement radii",
        )?;
        self.movement_radii
            .extend(movement.snakes().iter().map(|snake| snake.radius));
        reserve_for(
            &mut self.remaining_pellets,
            source.pellets.len(),
            "remaining pellets",
        )?;
        reserve_for(
            &mut self.boost_drops,
            movement.boost_drops().len(),
            "boost drops",
        )?;
        self.next_snakes.extend(movement.snakes().iter().cloned());
        self.final_lengths.resize(source.snakes.len(), 0);
        self.winners.resize(source.pellets.len(), None);
        self.boost_drops.extend_from_slice(movement.boost_drops());

        self.snake_order.extend(0..source.snakes.len());
        self.snake_order
            .sort_unstable_by_key(|index| movement.snakes()[*index].id);
        for pair in self.snake_order.windows(2) {
            if movement.snakes()[pair[0]].id == movement.snakes()[pair[1]].id {
                return Err(FoodError::DuplicateSnakeId(movement.snakes()[pair[0]].id));
            }
        }
        for index in 0..source.snakes.len() {
            if source.snakes[index].id != movement.snakes()[index].id {
                return Err(FoodError::MovementShapeMismatch);
            }
        }

        self.pellet_order.extend(0..source.pellets.len());
        self.pellet_order.sort_unstable_by(|left, right| {
            source.pellets[*left]
                .id
                .cmp(&source.pellets[*right].id)
                .then_with(|| left.cmp(right))
        });
        for &pellet_index in &self.pellet_order {
            validate_pellet(&source.pellets[pellet_index])?;
        }
        for pair in self.pellet_order.windows(2) {
            if source.pellets[pair[0]].id == source.pellets[pair[1]].id {
                return Err(FoodError::DuplicatePelletId(source.pellets[pair[0]].id));
            }
        }

        for &snake_index in &self.snake_order {
            let snake = &movement.snakes()[snake_index];
            if !snake.alive {
                continue;
            }
            let radius = snake.radius + food_config.eat_radius_padding;
            if !radius.is_finite() || radius < 0.0 {
                return Err(FoodError::NonFiniteDerived {
                    snake_id: snake.id,
                    field: "eat radius",
                });
            }
            let query = indexed.pellet_index().collect_candidates(
                snake.position,
                radius,
                &mut self.query_scratch,
            )?;
            add_query_diagnostics(&mut self.query_diagnostics, query);
            for pellet in indexed.pellet_index().candidates(&self.query_scratch) {
                let source_pellet = source
                    .pellets
                    .get(pellet.source_index)
                    .ok_or(FoodError::IndexedPelletMismatch)?;
                if pellet.id != source_pellet.id
                    || pellet.position != source_pellet.position
                    || pellet.value.to_bits() != source_pellet.value.to_bits()
                {
                    return Err(FoodError::IndexedPelletMismatch);
                }
                let dx = pellet.position.x - snake.position.x;
                let dy = pellet.position.y - snake.position.y;
                let distance_squared = dx * dx + dy * dy;
                if !distance_squared.is_finite() {
                    return Err(FoodError::NonFiniteDerived {
                        snake_id: snake.id,
                        field: "food distance",
                    });
                }
                let candidate = FoodWinner {
                    snake_id: snake.id,
                    snake_index,
                    distance_squared,
                };
                let winner = &mut self.winners[pellet.source_index];
                if winner.is_none_or(|current| winner_precedes(candidate, current)) {
                    *winner = Some(candidate);
                }
            }
        }

        for &pellet_index in &self.pellet_order {
            let pellet = &source.pellets[pellet_index];
            let Some(winner) = self.winners[pellet_index] else {
                self.remaining_pellets.push(pellet.clone());
                continue;
            };
            let snake = self
                .next_snakes
                .get_mut(winner.snake_index)
                .ok_or(FoodError::MovementShapeMismatch)?;
            if snake.id != winner.snake_id || !snake.alive {
                return Err(FoodError::MovementShapeMismatch);
            }
            let food = snake.food + pellet.value;
            let points = snake.points + pellet.value * food_config.points_per_food;
            let target_length = (snake.target_length + pellet.value * food_config.growth_per_food)
                .max(movement_config.snake_min_len as f64)
                .min(movement_config.snake_max_len as f64);
            for (field, value) in [
                ("food", food),
                ("points", points),
                ("target length", target_length),
            ] {
                if !value.is_finite() {
                    return Err(FoodError::NonFiniteDerived {
                        snake_id: snake.id,
                        field,
                    });
                }
            }
            snake.food = food;
            snake.points = points;
            snake.target_length = target_length;
            self.claims.push(FoodClaim {
                pellet_id: pellet.id,
                snake_id: winner.snake_id,
                snake_index: winner.snake_index,
                pellet_source_index: pellet_index,
                distance_squared: winner.distance_squared,
                value: pellet.value,
            });
        }

        let staged_pellet_total = self
            .remaining_pellets
            .len()
            .checked_add(self.boost_drops.len())
            .ok_or(FoodError::ArithmeticOverflow {
                context: "post-food pellet total",
            })?;
        if staged_pellet_total > maximum_pellets {
            return Err(FoodError::PelletCapacityExceeded {
                required: staged_pellet_total,
                maximum: maximum_pellets,
            });
        }

        let mut total_body_points = 0usize;
        for &snake_index in &self.snake_order {
            let snake = &self.next_snakes[snake_index];
            let body = movement
                .body_for(&movement.snakes()[snake_index])
                .ok_or(FoodError::InvalidBodyRange { snake_id: snake.id })?;
            if snake.alive && body.is_empty() {
                return Err(FoodError::InvalidBodyRange { snake_id: snake.id });
            }
            let final_length = if snake.alive {
                desired_body_length(snake.target_length, movement_config)
            } else {
                body.len()
            };
            total_body_points = total_body_points.checked_add(final_length).ok_or(
                FoodError::ArithmeticOverflow {
                    context: "final body-point total",
                },
            )?;
            if total_body_points > maximum_body_points {
                return Err(FoodError::BodyCapacityExceeded {
                    required: total_body_points,
                    maximum: maximum_body_points,
                });
            }
            self.final_lengths[snake_index] = final_length;
        }

        reserve_for(
            &mut self.next_body_points,
            total_body_points,
            "final packed body points",
        )?;
        for &snake_index in &self.snake_order {
            let source_snake = &movement.snakes()[snake_index];
            let source_body =
                movement
                    .body_for(source_snake)
                    .ok_or(FoodError::InvalidBodyRange {
                        snake_id: source_snake.id,
                    })?;
            let desired = self.final_lengths[snake_index];
            let body_start = self.next_body_points.len();
            self.next_body_points
                .extend_from_slice(&source_body[..source_body.len().min(desired)]);
            if self.next_snakes[snake_index].alive {
                grow_body(
                    &mut self.next_body_points,
                    body_start,
                    desired,
                    movement_config.snake_spacing,
                    source_snake.id,
                )?;
                self.next_snakes[snake_index].radius = radius_for_length(desired, movement_config);
            }
            self.next_snakes[snake_index].body = BodyRange {
                start: body_start,
                len: desired,
            };
            validate_final_snake(
                &self.next_snakes[snake_index],
                &self.next_body_points[body_start..body_start + desired],
            )?;
        }
        debug_assert_eq!(self.next_body_points.len(), total_body_points);
        self.ready = true;
        let diagnostics = self.diagnostics();
        Ok(PreparedFood {
            source_world: source,
            snakes: &self.next_snakes,
            body_points: &self.next_body_points,
            movement_body_ranges: &self.movement_body_ranges,
            movement_body_points: &self.movement_body_points,
            movement_radii: &self.movement_radii,
            remaining_pellets: &self.remaining_pellets,
            claims: &self.claims,
            boost_drops: &self.boost_drops,
            movement_proposals: &self.movement_proposals,
            query_diagnostics: self.query_diagnostics,
            diagnostics,
        })
    }

    /// Current sizes and retained capacities, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> FoodCapacityDiagnostics {
        FoodCapacityDiagnostics {
            snakes: self.next_snakes.len(),
            body_points: self.next_body_points.len(),
            movement_body_points: self.movement_body_points.len(),
            claims: self.claims.len(),
            remaining_pellets: self.remaining_pellets.len(),
            boost_drops: self.boost_drops.len(),
            snake_order_capacity: self.snake_order.capacity(),
            pellet_order_capacity: self.pellet_order.capacity(),
            winner_capacity: self.winners.capacity(),
            snake_capacity: self.next_snakes.capacity(),
            final_length_capacity: self.final_lengths.capacity(),
            body_point_capacity: self.next_body_points.capacity(),
            movement_body_range_capacity: self.movement_body_ranges.capacity(),
            movement_body_point_capacity: self.movement_body_points.capacity(),
            movement_radius_capacity: self.movement_radii.capacity(),
            claim_capacity: self.claims.capacity(),
            remaining_pellet_capacity: self.remaining_pellets.capacity(),
            boost_drop_capacity: self.boost_drops.capacity(),
            movement_proposal_capacity: self.movement_proposals.capacity(),
            query_candidate_capacity: self.query_scratch.candidate_capacity(),
        }
    }

    /// Whether the most recent preparation reached a complete staged result.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    fn clear(&mut self) {
        self.ready = false;
        self.snake_order.clear();
        self.pellet_order.clear();
        self.winners.clear();
        self.next_snakes.clear();
        self.final_lengths.clear();
        self.next_body_points.clear();
        self.movement_body_ranges.clear();
        self.movement_body_points.clear();
        self.movement_radii.clear();
        self.remaining_pellets.clear();
        self.claims.clear();
        self.boost_drops.clear();
        self.movement_proposals.clear();
        self.query_diagnostics = SpatialQueryDiagnostics::default();
    }
}

fn winner_precedes(candidate: FoodWinner, current: FoodWinner) -> bool {
    match candidate
        .distance_squared
        .total_cmp(&current.distance_squared)
    {
        Ordering::Less => true,
        Ordering::Equal => candidate.snake_id < current.snake_id,
        Ordering::Greater => false,
    }
}

fn validate_pellet(pellet: &PelletState) -> Result<(), FoodError> {
    if pellet.id == 0
        || !pellet.position.x.is_finite()
        || !pellet.position.y.is_finite()
        || !pellet.value.is_finite()
        || pellet.value <= 0.0
    {
        return Err(FoodError::InvalidPellet {
            pellet_id: pellet.id,
        });
    }
    Ok(())
}

fn grow_body(
    output: &mut Vec<WorldPoint>,
    start: usize,
    desired: usize,
    spacing: f64,
    snake_id: u64,
) -> Result<(), FoodError> {
    while output.len().saturating_sub(start) < desired {
        let tail = *output
            .last()
            .filter(|_| output.len() > start)
            .ok_or(FoodError::InvalidBodyRange { snake_id })?;
        let before = if output.len().saturating_sub(start) >= 2 {
            output[output.len() - 2]
        } else {
            tail
        };
        let dx = tail.x - before.x;
        let dy = tail.y - before.y;
        let distance = dx.hypot(dy);
        let divisor = if distance == 0.0 {
            MIN_DISTANCE
        } else {
            distance
        };
        let point = WorldPoint {
            x: tail.x + dx / divisor * spacing,
            y: tail.y + dy / divisor * spacing,
        };
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(FoodError::NonFiniteDerived {
                snake_id,
                field: "grown body point",
            });
        }
        output.push(point);
    }
    Ok(())
}

fn validate_final_snake(snake: &SnakeState, body: &[WorldPoint]) -> Result<(), FoodError> {
    if snake.alive && body.is_empty() {
        return Err(FoodError::InvalidBodyRange { snake_id: snake.id });
    }
    if !body.is_empty() && body[0] != snake.position {
        return Err(FoodError::InvalidBodyRange { snake_id: snake.id });
    }
    for (field, value) in [
        ("position.x", snake.position.x),
        ("position.y", snake.position.y),
        ("radius", snake.radius),
        ("food", snake.food),
        ("points", snake.points),
        ("target length", snake.target_length),
    ] {
        if !value.is_finite() {
            return Err(FoodError::NonFiniteDerived {
                snake_id: snake.id,
                field,
            });
        }
    }
    Ok(())
}

fn add_query_diagnostics(total: &mut SpatialQueryDiagnostics, next: SpatialQueryDiagnostics) {
    total.cells_visited = total.cells_visited.saturating_add(next.cells_visited);
    total.entries_visited = total.entries_visited.saturating_add(next.entries_visited);
    total.candidates = total.candidates.saturating_add(next.candidates);
    total.candidate_limit_reached |= next.candidate_limit_reached;
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), FoodError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| FoodError::AllocationFailed { context, required })?;
    }
    Ok(())
}

/// Rejected source, capacity, query, arithmetic, or food preparation.
#[derive(Clone, Debug, PartialEq)]
pub enum FoodError {
    /// One food configuration field is invalid.
    InvalidConfig { field: &'static str },
    /// Movement configuration failed validation.
    Movement(super::movement::MovementError),
    /// Admitted body or pellet capacity is zero.
    InvalidCapacity,
    /// The movement and indexed-pellet views do not borrow the same world.
    SourceWorldMismatch,
    /// Movement records do not match the bound source container.
    MovementShapeMismatch,
    /// Stable snake identity is duplicated.
    DuplicateSnakeId(u64),
    /// Stable pellet identity is duplicated.
    DuplicatePelletId(u64),
    /// One source pellet is malformed.
    InvalidPellet { pellet_id: u64 },
    /// An indexed pellet no longer matches its bound source record.
    IndexedPelletMismatch,
    /// One staged body range is empty, incoherent, or out of bounds.
    InvalidBodyRange { snake_id: u64 },
    /// Finite source values overflowed while deriving post-food state.
    NonFiniteDerived { snake_id: u64, field: &'static str },
    /// Checked arithmetic failed.
    ArithmeticOverflow { context: &'static str },
    /// Complete final body storage exceeds its admitted capacity.
    BodyCapacityExceeded { required: usize, maximum: usize },
    /// Remaining plus staged boost pellets exceed admitted storage.
    PelletCapacityExceeded { required: usize, maximum: usize },
    /// Complete pellet query failed.
    Spatial(SpatialIndexError),
    /// Reusable scratch could not be reserved.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
}

impl From<SpatialIndexError> for FoodError {
    fn from(value: SpatialIndexError) -> Self {
        Self::Spatial(value)
    }
}

impl Display for FoodError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => write!(formatter, "invalid food config: {field}"),
            Self::Movement(error) => write!(formatter, "movement config failed: {error}"),
            Self::InvalidCapacity => write!(formatter, "food capacities must be positive"),
            Self::SourceWorldMismatch => {
                write!(
                    formatter,
                    "movement and pellet index use different source worlds"
                )
            }
            Self::MovementShapeMismatch => {
                write!(formatter, "movement records do not match the source world")
            }
            Self::DuplicateSnakeId(id) => write!(formatter, "duplicate food snake ID {id}"),
            Self::DuplicatePelletId(id) => write!(formatter, "duplicate food pellet ID {id}"),
            Self::InvalidPellet { pellet_id } => {
                write!(formatter, "invalid source pellet {pellet_id}")
            }
            Self::IndexedPelletMismatch => {
                write!(
                    formatter,
                    "pellet index does not match its bound source world"
                )
            }
            Self::InvalidBodyRange { snake_id } => {
                write!(formatter, "invalid final body range for snake {snake_id}")
            }
            Self::NonFiniteDerived { snake_id, field } => {
                write!(
                    formatter,
                    "food derived non-finite {field} for snake {snake_id}"
                )
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "food arithmetic overflow while calculating {context}"
                )
            }
            Self::BodyCapacityExceeded { required, maximum } => write!(
                formatter,
                "food finalization needs {required} body points, exceeding maximum {maximum}"
            ),
            Self::PelletCapacityExceeded { required, maximum } => write!(
                formatter,
                "food finalization needs {required} pellets, exceeding maximum {maximum}"
            ),
            Self::Spatial(error) => write!(formatter, "food pellet query failed: {error}"),
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "food could not reserve {required} entries for {context}"
            ),
        }
    }
}

impl Error for FoodError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::movement::MovementWorkspace;
    use crate::engine::state::{SnakeKind, WorldState};

    const DT: f64 = 1.0 / 180.0;
    type NormalizedSnake = (u64, SnakeState, Vec<WorldPoint>);
    type NormalizedClaim = (u64, u64, u64);
    type NormalizedFoodResult = (Vec<NormalizedSnake>, Vec<NormalizedClaim>, Vec<u64>);

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 2.0e-12,
            "actual {actual:.17} differs from expected {expected:.17}"
        );
    }

    fn snake(
        id: u64,
        body_start: usize,
        length: usize,
        position: WorldPoint,
        direction: f64,
    ) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(u32::try_from(id - 1).expect("fixture slot should fit")),
            brain: None,
            baseline_slot: None,
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
                start: body_start,
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

    fn world_with_one(length: usize) -> WorldState {
        let position = WorldPoint { x: 0.0, y: 0.0 };
        WorldState {
            snakes: vec![snake(1, 0, length, position, 0.0)],
            body_points: line_body(position, 0.0, length),
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        }
    }

    fn pellet(id: u64, x: f64, y: f64, value: f64) -> PelletState {
        PelletState {
            id,
            position: WorldPoint { x, y },
            value,
            kind: 0,
            color: 0,
            owner: None,
        }
    }

    fn index(world: &WorldState) -> IndexedPelletWorld<'_> {
        IndexedPelletWorld::build(world, 120.0, 10_000).expect("fixture indexes should build")
    }

    #[test]
    fn no_food_finishes_the_retained_typescript_boost_fixture() {
        let mut world = world_with_one(8);
        world.snakes[0].turn = -0.5;
        world.snakes[0].input_boost = true;
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("food finalization should prepare");
        let staged = &prepared.snakes()[0];
        let body = prepared.body_for(staged).expect("final body should exist");

        assert!(std::ptr::eq(prepared.source_world(), &world));
        assert_eq!(prepared.movement_proposals(), movement.proposals());
        close(staged.position.x, 0.982_628_430_314_376_3);
        close(staged.position.y, -0.008_732_258_602_803_35);
        close(staged.direction, -0.008_886_399_452_323_957);
        close(staged.speed, 176.880_101_352_972_45);
        close(staged.points, 9.961_098_271_357_901);
        close(staged.target_length, 7.993_775_723_417_264);
        close(staged.radius, 9.187_161_711_298_957);
        assert_eq!(body.len(), 7);
        close(body[6].x, -44.017_367_541_676_194);
        close(body[6].y, -0.000_000_021_100_342_633_887_367);
        assert!(prepared.claims().is_empty());
        assert!(prepared.remaining_pellets().is_empty());
        assert_eq!(prepared.boost_drops().len(), 1);
    }

    #[test]
    fn food_updates_score_target_radius_and_body_after_movement() {
        let mut world = world_with_one(5);
        world.pellets.push(pellet(20, 0.0, 0.0, 1.0));
        let authority_before = world.clone();
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("food should prepare");
        let staged = &prepared.snakes()[0];

        close(staged.food, 1.0);
        close(staged.points, 30.0);
        close(staged.target_length, 6.0);
        close(staged.radius, 9.095_090_486_186_674);
        assert_eq!(staged.body.len, 6);
        let body = prepared.body_for(staged).expect("grown body should exist");
        for (actual, expected) in body.iter().zip([
            WorldPoint {
                x: 0.916_666_666_666_666_7,
                y: 0.0,
            },
            WorldPoint {
                x: -6.583_333_333_333_334,
                y: 0.0,
            },
            WorldPoint {
                x: -14.083_333_333_333_334,
                y: 0.0,
            },
            WorldPoint {
                x: -21.583_333_333_333_332,
                y: 0.0,
            },
            WorldPoint {
                x: -29.083_333_333_333_332,
                y: 0.0,
            },
            WorldPoint {
                x: -36.583_333_333_333_33,
                y: 0.0,
            },
        ]) {
            close(actual.x, expected.x);
            close(actual.y, expected.y);
        }
        assert_eq!(prepared.claims().len(), 1);
        assert_eq!(prepared.claims()[0].pellet_id, 20);
        assert_eq!(prepared.claims()[0].snake_id, 1);
        assert!(prepared.remaining_pellets().is_empty());
        assert_eq!(world, authority_before);
    }

    #[test]
    fn same_substep_food_prevents_premature_ordinary_shrink() {
        let mut world = world_with_one(5);
        world.snakes[0].target_length = 4.0;
        world.pellets.push(pellet(20, 0.0, 0.0, 1.0));
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should defer ordinary shrink");
        assert_eq!(movement.snakes()[0].body.len, 5);
        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("food should cancel the pending shrink");

        assert_eq!(prepared.snakes()[0].body.len, 5);
        close(prepared.snakes()[0].target_length, 5.0);
    }

    fn contested_world(reverse: bool) -> WorldState {
        let first_position = WorldPoint { x: 0.0, y: 0.0 };
        let second_position = WorldPoint { x: 2.0, y: 0.0 };
        let first = snake(1, 0, 5, first_position, 0.0);
        let mut second = snake(2, 5, 5, second_position, 0.0);
        let first_body = line_body(first_position, 0.0, 5);
        let second_body = line_body(second_position, 0.0, 5);
        if !reverse {
            WorldState {
                snakes: vec![first, second],
                body_points: [first_body, second_body].concat(),
                pellets: vec![pellet(20, 1.0, 0.0, 1.0), pellet(10, 2.0, 0.0, 0.5)],
                controller_leases: Vec::new(),
            }
        } else {
            second.body.start = 0;
            let mut reversed_first = first;
            reversed_first.body.start = 5;
            WorldState {
                snakes: vec![second, reversed_first],
                body_points: [second_body, first_body].concat(),
                pellets: vec![pellet(10, 2.0, 0.0, 0.5), pellet(20, 1.0, 0.0, 1.0)],
                controller_leases: Vec::new(),
            }
        }
    }

    fn normalized_food_result(prepared: PreparedFood<'_, '_>) -> NormalizedFoodResult {
        let mut snakes = prepared
            .snakes()
            .iter()
            .map(|snake| {
                let mut normalized = snake.clone();
                let body = prepared
                    .body_for(snake)
                    .expect("body should resolve")
                    .to_vec();
                normalized.body.start = 0;
                (snake.id, normalized, body)
            })
            .collect::<Vec<_>>();
        snakes.sort_by_key(|entry| entry.0);
        let claims = prepared
            .claims()
            .iter()
            .map(|claim| {
                (
                    claim.pellet_id,
                    claim.snake_id,
                    claim.distance_squared.to_bits(),
                )
            })
            .collect();
        let remaining = prepared
            .remaining_pellets()
            .iter()
            .map(|pellet| pellet.id)
            .collect();
        (snakes, claims, remaining)
    }

    #[test]
    fn contested_food_uses_nearest_then_stable_id_independent_of_container_order() {
        fn execute(world: &WorldState) -> NormalizedFoodResult {
            let indexed = index(world);
            let mut movement_workspace = MovementWorkspace::new();
            let movement = movement_workspace
                .prepare(world, MovementConfig::typescript_defaults(), DT, 100, 100)
                .expect("movement should prepare");
            let mut food_workspace = FoodWorkspace::new();
            let prepared = food_workspace
                .prepare(
                    &indexed,
                    movement,
                    MovementConfig::typescript_defaults(),
                    FoodConfig::typescript_defaults(),
                    100,
                    100,
                )
                .expect("contested food should prepare");
            normalized_food_result(prepared)
        }

        let forward = execute(&contested_world(false));
        let reversed = execute(&contested_world(true));
        assert_eq!(forward, reversed);
        assert_eq!(
            forward
                .1
                .iter()
                .map(|claim| (claim.0, claim.1))
                .collect::<Vec<_>>(),
            vec![(10, 2), (20, 1)]
        );

        let tied_position = WorldPoint { x: 0.0, y: 0.0 };
        let mut tied = WorldState {
            snakes: vec![
                snake(2, 0, 5, tied_position, 0.0),
                snake(1, 5, 5, tied_position, 0.0),
            ],
            body_points: [
                line_body(tied_position, 0.0, 5),
                line_body(tied_position, 0.0, 5),
            ]
            .concat(),
            pellets: vec![pellet(30, 1.0, 0.0, 1.0)],
            controller_leases: Vec::new(),
        };
        tied.snakes[1].population_slot = Some(0);
        tied.snakes[0].population_slot = Some(1);
        let tied_result = execute(&tied);
        assert_eq!(tied_result.1.len(), 1);
        assert_eq!((tied_result.1[0].0, tied_result.1[0].1), (30, 1));
    }

    #[test]
    fn boost_shrink_food_eligibility_uses_the_pre_finalization_radius() {
        let movement_config = MovementConfig::typescript_defaults();
        let mut world = world_with_one(8);
        world.snakes[0].input_boost = true;
        world.snakes[0].radius = radius_for_length(8, movement_config);
        let retained_eat_radius = world.snakes[0].radius + 6.0;
        let finalized_eat_radius = radius_for_length(7, movement_config) + 6.0;
        let pellet_distance = (retained_eat_radius + finalized_eat_radius) * 0.5;
        world.pellets.push(pellet(
            20,
            0.982_628_430_314_376_3 + pellet_distance,
            -0.008_732_258_602_803_35,
            1.0,
        ));
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, movement_config, DT, 100, 100)
            .expect("boost movement should prepare");
        let staged_head = movement.snakes()[0].position;
        let pellet_position = world.pellets[0].position;
        let staged_distance =
            (pellet_position.x - staged_head.x).hypot(pellet_position.y - staged_head.y);
        assert!(staged_distance > finalized_eat_radius);
        assert!(staged_distance < retained_eat_radius);

        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("pre-finalization radius should admit the pellet");

        assert_eq!(prepared.claims().len(), 1);
        assert_eq!(prepared.claims()[0].snake_id, 1);
        assert!(prepared.remaining_pellets().is_empty());
    }

    #[test]
    fn body_capacity_failure_leaves_authority_untouched_and_staging_unready() {
        let mut world = world_with_one(5);
        world.snakes[0].target_length = 6.0;
        let authority_before = world.clone();
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 5, 100)
            .expect("movement should defer final growth");
        let mut food_workspace = FoodWorkspace::new();

        assert!(matches!(
            food_workspace.prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                5,
                100,
            ),
            Err(FoodError::BodyCapacityExceeded {
                required: 6,
                maximum: 5
            })
        ));
        assert!(!food_workspace.is_ready());
        assert_eq!(world, authority_before);
    }

    #[test]
    fn consumed_food_frees_capacity_for_a_staged_boost_drop_atomically() {
        fn execute(pellet_x: f64) -> Result<(usize, usize), FoodError> {
            let mut world = world_with_one(8);
            world.snakes[0].input_boost = true;
            world.pellets.push(pellet(20, pellet_x, 0.0, 1.0));
            let indexed = index(&world);
            let mut movement_workspace = MovementWorkspace::new();
            let movement = movement_workspace
                .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 1)
                .expect("one bounded boost request should stage");
            assert_eq!(movement.boost_drops().len(), 1);
            let mut food_workspace = FoodWorkspace::new();
            let prepared = food_workspace.prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                1,
            )?;
            Ok((
                prepared.remaining_pellets().len(),
                prepared.boost_drops().len(),
            ))
        }

        assert_eq!(execute(0.0), Ok((0, 1)));
        assert!(matches!(
            execute(1_000.0),
            Err(FoodError::PelletCapacityExceeded {
                required: 2,
                maximum: 1
            })
        ));
    }

    #[test]
    fn wall_dead_snake_cannot_claim_food() {
        let mut world = world_with_one(5);
        world.snakes[0].position = WorldPoint { x: 3_490.2, y: 0.0 };
        world.snakes[0].previous_position = world.snakes[0].position;
        for (index, point) in world.body_points.iter_mut().enumerate() {
            point.x = 3_490.2 - index as f64 * 7.5;
        }
        world.pellets.push(pellet(20, 3_491.0, 0.0, 1.0));
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("wall movement should prepare");
        assert!(!movement.snakes()[0].alive);
        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("dead-snake food phase should prepare");

        assert!(prepared.claims().is_empty());
        assert_eq!(prepared.remaining_pellets().len(), 1);
        assert_eq!(prepared.snakes()[0].food, 0.0);
    }

    #[test]
    fn source_world_identity_is_required() {
        let first = world_with_one(5);
        let second = first.clone();
        let indexed = index(&second);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&first, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();

        assert!(matches!(
            food_workspace.prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            ),
            Err(FoodError::SourceWorldMismatch)
        ));
    }

    #[test]
    fn finite_extreme_food_values_cannot_publish_non_finite_staging() {
        let mut world = world_with_one(5);
        world.pellets.push(pellet(20, 0.0, 0.0, f64::MAX));
        let authority_before = world.clone();
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();

        assert!(matches!(
            food_workspace.prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            ),
            Err(FoodError::NonFiniteDerived { snake_id: 1, .. })
        ));
        assert!(!food_workspace.is_ready());
        assert_eq!(world, authority_before);
    }

    #[test]
    fn warmed_preparation_reuses_all_food_and_body_capacities() {
        let mut world = world_with_one(5);
        world.pellets.push(pellet(20, 0.0, 0.0, 1.0));
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let first = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("warm food pass should prepare")
            .diagnostics();

        for _ in 0..24 {
            let next = food_workspace
                .prepare(
                    &indexed,
                    movement,
                    MovementConfig::typescript_defaults(),
                    FoodConfig::typescript_defaults(),
                    100,
                    100,
                )
                .expect("reused food pass should prepare")
                .diagnostics();
            assert_eq!(next.snake_order_capacity, first.snake_order_capacity);
            assert_eq!(next.pellet_order_capacity, first.pellet_order_capacity);
            assert_eq!(next.winner_capacity, first.winner_capacity);
            assert_eq!(next.snake_capacity, first.snake_capacity);
            assert_eq!(next.final_length_capacity, first.final_length_capacity);
            assert_eq!(next.body_point_capacity, first.body_point_capacity);
            assert_eq!(
                next.movement_body_range_capacity,
                first.movement_body_range_capacity
            );
            assert_eq!(
                next.movement_body_point_capacity,
                first.movement_body_point_capacity
            );
            assert_eq!(
                next.movement_radius_capacity,
                first.movement_radius_capacity
            );
            assert_eq!(next.claim_capacity, first.claim_capacity);
            assert_eq!(
                next.remaining_pellet_capacity,
                first.remaining_pellet_capacity
            );
            assert_eq!(next.boost_drop_capacity, first.boost_drop_capacity);
            assert_eq!(
                next.movement_proposal_capacity,
                first.movement_proposal_capacity
            );
            assert_eq!(
                next.query_candidate_capacity,
                first.query_candidate_capacity
            );
        }
    }
}
