//! Deterministic collision-safe placement for complete initial snake bodies.
//!
//! The current TypeScript constructor draws an angle, an area-uniform radius,
//! and a heading, then accepts the resulting body without checking overlap.
//! This module preserves that three-draw candidate formula while correcting
//! admission: requests are visited in stable-key order, every complete body is
//! checked against the circular wall and all live/staged bodies, and exhausted
//! random attempts use a bounded deterministic fallback. Preparation mutates
//! only reusable scratch; the source world and serialized RNG remain borrowed.

use super::rng::{RngError, SerializedRngState, StatefulRng};
use super::state::{BodyRange, WorldPoint, WorldState};
use std::error::Error;
use std::f64::consts::TAU;
use std::fmt::{Display, Formatter};

/// Golden angle used by the deterministic low-discrepancy fallback.
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
/// First collision-safe spawn algorithm encoded by checkpoints/config identity.
pub const SPAWN_ALGORITHM_VERSION: u32 = 1;
/// Hard admission ceiling for one request's random plus fallback candidates.
const MAXIMUM_CANDIDATES_PER_REQUEST: usize = 1_000_000;
/// Hard admission ceiling for candidates examined by one complete batch.
const MAXIMUM_CANDIDATES_PER_BATCH: usize = 10_000_000;
/// Hard admission ceiling for wall/body comparisons in one complete batch.
const MAXIMUM_GEOMETRY_CHECKS_PER_BATCH: usize = 100_000_000;

/// Stable namespace for one spawn-placement key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SpawnDomain {
    /// Dense evolved-population slot.
    Evolved = 0,
    /// Durable built-in baseline slot.
    Baseline = 1,
    /// External browser-player or RL entity.
    External = 2,
    /// Hall-of-Fame resurrection entity.
    Resurrected = 3,
}

/// Stable identity controlling candidate draw and placement order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpawnKey {
    /// Stable entity namespace.
    pub domain: SpawnDomain,
    /// Population/baseline slot or exact external/resurrected identity.
    pub slot: u64,
}

/// One snake whose complete initial body requires a placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnRequest {
    /// Stable key used instead of caller container order.
    pub key: SpawnKey,
}

/// Versioned values projected from the admitted gameplay configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnConfig {
    /// Versioned candidate ordering and fallback contract.
    pub algorithm_version: u32,
    /// Circular arena radius.
    pub world_radius: f64,
    /// Fraction of arena radius used by random and fallback head candidates.
    pub spawn_radius_fraction: f64,
    /// Collision radius of a newly spawned snake.
    pub snake_radius: f64,
    /// Initial target distance between adjacent body points.
    pub snake_spacing: f64,
    /// Exact initial body-point count.
    pub snake_start_len: usize,
    /// Extra distance required between a body and the arena wall.
    pub wall_clearance: f64,
    /// Extra distance required beyond the two bodies' collision radii.
    pub body_clearance: f64,
    /// Random three-draw candidates attempted for each request.
    pub random_attempts_per_request: usize,
    /// Number of deterministic low-discrepancy fallback head positions.
    pub fallback_position_count: usize,
    /// Number of evenly spaced headings tried at each fallback position.
    pub fallback_heading_count: usize,
    /// Admitted work ceiling across random and fallback candidates per request.
    pub maximum_candidates_per_request: usize,
    /// Admitted candidate ceiling across the complete prepared batch.
    pub maximum_candidates_per_batch: usize,
    /// Admitted wall/body comparison ceiling across the complete batch.
    pub maximum_geometry_checks_per_batch: usize,
}

impl SpawnConfig {
    /// Current TypeScript geometry plus bounded correction defaults.
    #[must_use]
    pub const fn typescript_geometry_defaults() -> Self {
        Self {
            algorithm_version: SPAWN_ALGORITHM_VERSION,
            world_radius: 3_500.0,
            spawn_radius_fraction: 0.60,
            snake_radius: 9.0,
            snake_spacing: 7.5,
            snake_start_len: 5,
            wall_clearance: 0.0,
            body_clearance: 0.0,
            random_attempts_per_request: 32,
            fallback_position_count: 2_048,
            fallback_heading_count: 16,
            maximum_candidates_per_request: 40_000,
            maximum_candidates_per_batch: 500_000,
            maximum_geometry_checks_per_batch: 10_000_000,
        }
    }

    /// Validate all derived-geometry and bounded-work inputs before staging.
    fn validate(self) -> Result<(), SpawnError> {
        for (field, value) in [
            ("world_radius", self.world_radius),
            ("spawn_radius_fraction", self.spawn_radius_fraction),
            ("snake_radius", self.snake_radius),
            ("snake_spacing", self.snake_spacing),
            ("wall_clearance", self.wall_clearance),
            ("body_clearance", self.body_clearance),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(SpawnError::InvalidConfig { field });
            }
        }
        if self.algorithm_version != SPAWN_ALGORITHM_VERSION
            || self.world_radius <= 0.0
            || self.spawn_radius_fraction <= 0.0
            || self.spawn_radius_fraction > 1.0
            || self.snake_radius <= 0.0
            || self.snake_spacing <= 0.0
            || self.snake_start_len == 0
            || self.random_attempts_per_request == 0
            || self.fallback_position_count == 0
            || self.fallback_heading_count == 0
            || self.maximum_candidates_per_request == 0
            || self.maximum_candidates_per_request > MAXIMUM_CANDIDATES_PER_REQUEST
            || self.maximum_candidates_per_batch == 0
            || self.maximum_candidates_per_batch > MAXIMUM_CANDIDATES_PER_BATCH
            || self.maximum_geometry_checks_per_batch == 0
            || self.maximum_geometry_checks_per_batch > MAXIMUM_GEOMETRY_CHECKS_PER_BATCH
            || self.wall_clearance >= self.world_radius
            || self.body_clearance >= self.world_radius
        {
            return Err(SpawnError::InvalidConfig {
                field: "spawn ranges",
            });
        }
        let fallback_candidates = self
            .fallback_position_count
            .checked_mul(self.fallback_heading_count)
            .ok_or(SpawnError::ArithmeticOverflow {
                context: "fallback candidate count",
            })?;
        let total_candidates = self
            .random_attempts_per_request
            .checked_add(fallback_candidates)
            .ok_or(SpawnError::ArithmeticOverflow {
                context: "total candidate count",
            })?;
        if total_candidates > self.maximum_candidates_per_request {
            return Err(SpawnError::InvalidConfig {
                field: "candidate work budget",
            });
        }
        let tail_distance = (self.snake_start_len - 1) as f64 * self.snake_spacing;
        if !tail_distance.is_finite() {
            return Err(SpawnError::InvalidConfig {
                field: "initial body extent",
            });
        }
        Ok(())
    }
}

/// One collision-safe placement retained in stable-key order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnPlacement {
    /// Stable request identity.
    pub key: SpawnKey,
    /// Head position, equal to the first packed body point.
    pub head: WorldPoint,
    /// Initial heading in radians.
    pub direction: f64,
    /// Range into the prepared packed body-point storage.
    pub body: BodyRange,
    /// Whether the deterministic fallback supplied this placement.
    pub used_fallback: bool,
    /// One-based candidate count examined for this request.
    pub candidates_examined: usize,
}

/// Current sizes and retained capacities for one prepared placement batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpawnCapacityDiagnostics {
    /// Prepared request count.
    pub placements: usize,
    /// Prepared packed body-point count.
    pub body_points: usize,
    /// Total candidates examined in the latest successful batch.
    pub candidates_examined: usize,
    /// Requests placed by deterministic fallback.
    pub fallback_placements: usize,
    /// Wall/body geometric comparisons performed by the successful batch.
    pub geometry_checks: usize,
    /// Retained stable-order capacity.
    pub order_capacity: usize,
    /// Retained canonical live-source obstacle order capacity.
    pub source_order_capacity: usize,
    /// Retained placement capacity.
    pub placement_capacity: usize,
    /// Retained packed body-point capacity.
    pub body_point_capacity: usize,
    /// Retained single-candidate body capacity.
    pub candidate_body_capacity: usize,
}

/// Immutable view of one completely prepared placement batch.
#[derive(Debug)]
pub struct PreparedSpawns<'scratch, 'world> {
    source_world: &'world WorldState,
    placements: &'scratch [SpawnPlacement],
    body_points: &'scratch [WorldPoint],
    next_rng: &'scratch SerializedRngState,
    diagnostics: SpawnCapacityDiagnostics,
}

impl<'scratch, 'world> PreparedSpawns<'scratch, 'world> {
    /// Immutable source boundary against which placements were checked.
    #[must_use]
    pub const fn source_world(&self) -> &'world WorldState {
        self.source_world
    }

    /// Placements in stable-key order.
    #[must_use]
    pub const fn placements(&self) -> &'scratch [SpawnPlacement] {
        self.placements
    }

    /// Packed body points referenced by every placement.
    #[must_use]
    pub const fn body_points(&self) -> &'scratch [WorldPoint] {
        self.body_points
    }

    /// Read one placement's complete head-to-tail body.
    #[must_use]
    pub fn body_for(&self, placement: &SpawnPlacement) -> Option<&'scratch [WorldPoint]> {
        placement
            .body
            .start
            .checked_add(placement.body.len)
            .and_then(|end| self.body_points.get(placement.body.start..end))
    }

    /// Gameplay RNG continuation after all attempted random candidates.
    #[must_use]
    pub const fn next_rng(&self) -> &'scratch SerializedRngState {
        self.next_rng
    }

    /// Bounded staging and retained-capacity diagnostics.
    #[must_use]
    pub const fn diagnostics(&self) -> SpawnCapacityDiagnostics {
        self.diagnostics
    }
}

/// Reusable scratch for collision-safe spawn preparation.
#[derive(Default)]
pub struct SpawnWorkspace {
    order: Vec<usize>,
    source_order: Vec<usize>,
    placements: Vec<SpawnPlacement>,
    body_points: Vec<WorldPoint>,
    candidate_body: Vec<WorldPoint>,
    next_rng: Option<SerializedRngState>,
    candidates_examined: usize,
    fallback_placements: usize,
    geometry_checks: usize,
    ready: bool,
}

#[derive(Clone, Copy)]
struct SpawnObstacles<'a> {
    source_world: &'a WorldState,
    source_order: &'a [usize],
    placements: &'a [SpawnPlacement],
    staged_body_points: &'a [WorldPoint],
}

impl SpawnWorkspace {
    /// Prepare a batch sharing one versioned gameplay RNG continuation.
    ///
    /// Callers use separate batches for independently owned streams (for
    /// example one population world stream versus per-baseline streams). The
    /// requests within this batch are always drawn and emitted by stable key,
    /// never by caller container order.
    pub fn prepare<'scratch, 'world>(
        &'scratch mut self,
        source_world: &'world WorldState,
        requests: &[SpawnRequest],
        source_rng: &SerializedRngState,
        config: SpawnConfig,
        maximum_new_body_points: usize,
    ) -> Result<PreparedSpawns<'scratch, 'world>, SpawnError> {
        self.clear_staging();
        config.validate()?;
        self.prepare_source_order(source_world)?;

        let required_body_points = requests.len().checked_mul(config.snake_start_len).ok_or(
            SpawnError::ArithmeticOverflow {
                context: "requested initial body points",
            },
        )?;
        if required_body_points > maximum_new_body_points {
            return Err(SpawnError::BodyCapacityExceeded {
                required: required_body_points,
                maximum: maximum_new_body_points,
            });
        }
        reserve_for(&mut self.order, requests.len(), "spawn order")?;
        reserve_for(&mut self.placements, requests.len(), "spawn placements")?;
        reserve_for(
            &mut self.body_points,
            required_body_points,
            "spawn body points",
        )?;
        reserve_for(
            &mut self.candidate_body,
            config.snake_start_len,
            "candidate body points",
        )?;

        self.order.extend(0..requests.len());
        self.order
            .sort_unstable_by_key(|index| requests[*index].key);
        for pair in self.order.windows(2) {
            let left = requests[pair[0]].key;
            let right = requests[pair[1]].key;
            if left == right {
                return Err(SpawnError::DuplicateRequestKey(left));
            }
        }

        let mut rng = StatefulRng::from_state(source_rng)?;
        for order_index in 0..self.order.len() {
            let request = requests[self.order[order_index]];
            let placement = self.place_request(source_world, request, &mut rng, config)?;
            self.placements.push(placement);
        }
        self.next_rng = Some(rng.export_state());
        self.ready = true;
        Ok(self.prepared(source_world))
    }

    /// Whether the most recent call produced a complete placement batch.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    fn place_request(
        &mut self,
        source_world: &WorldState,
        request: SpawnRequest,
        rng: &mut StatefulRng,
        config: SpawnConfig,
    ) -> Result<SpawnPlacement, SpawnError> {
        let start_examined = self.candidates_examined;
        for _ in 0..config.random_attempts_per_request {
            let angle = rng.next_f64() * TAU;
            let radius =
                rng.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
            let head = WorldPoint {
                x: angle.cos() * radius,
                y: angle.sin() * radius,
            };
            let direction = rng.next_f64() * TAU;
            self.record_candidate(request.key, config)?;
            build_body(&mut self.candidate_body, head, direction, config)?;
            if candidate_is_valid(
                &self.candidate_body,
                SpawnObstacles {
                    source_world,
                    source_order: &self.source_order,
                    placements: &self.placements,
                    staged_body_points: &self.body_points,
                },
                config,
                request.key,
                &mut self.geometry_checks,
            )? {
                return self.accept_candidate(request.key, direction, false, start_examined);
            }
        }

        for position_index in 0..config.fallback_position_count {
            let head = fallback_position(position_index, config);
            for heading_index in 0..config.fallback_heading_count {
                let direction = heading_index as f64 * TAU / config.fallback_heading_count as f64;
                self.record_candidate(request.key, config)?;
                build_body(&mut self.candidate_body, head, direction, config)?;
                if candidate_is_valid(
                    &self.candidate_body,
                    SpawnObstacles {
                        source_world,
                        source_order: &self.source_order,
                        placements: &self.placements,
                        staged_body_points: &self.body_points,
                    },
                    config,
                    request.key,
                    &mut self.geometry_checks,
                )? {
                    self.fallback_placements = self.fallback_placements.checked_add(1).ok_or(
                        SpawnError::ArithmeticOverflow {
                            context: "fallback placement count",
                        },
                    )?;
                    return self.accept_candidate(request.key, direction, true, start_examined);
                }
            }
        }

        Err(SpawnError::NoCollisionSafePlacement {
            key: request.key,
            random_attempts: config.random_attempts_per_request,
            fallback_candidates: config
                .fallback_position_count
                .checked_mul(config.fallback_heading_count)
                .ok_or(SpawnError::ArithmeticOverflow {
                    context: "fallback candidate count",
                })?,
        })
    }

    fn prepare_source_order(&mut self, source_world: &WorldState) -> Result<(), SpawnError> {
        reserve_for(
            &mut self.source_order,
            source_world.snakes.len(),
            "spawn source order",
        )?;
        for (index, snake) in source_world.snakes.iter().enumerate() {
            if !snake.alive {
                continue;
            }
            if !snake.radius.is_finite() || snake.radius <= 0.0 {
                return Err(SpawnError::InvalidSourceSnake { snake_id: snake.id });
            }
            checked_source_body(source_world, snake.id, snake.body)?;
            self.source_order.push(index);
        }
        self.source_order
            .sort_unstable_by_key(|index| source_world.snakes[*index].id);
        for pair in self.source_order.windows(2) {
            let left = source_world.snakes[pair[0]].id;
            let right = source_world.snakes[pair[1]].id;
            if left == right {
                return Err(SpawnError::DuplicateSourceSnakeId(left));
            }
        }
        Ok(())
    }

    fn record_candidate(&mut self, key: SpawnKey, config: SpawnConfig) -> Result<(), SpawnError> {
        let required =
            self.candidates_examined
                .checked_add(1)
                .ok_or(SpawnError::ArithmeticOverflow {
                    context: "examined spawn candidates",
                })?;
        if required > config.maximum_candidates_per_batch {
            return Err(SpawnError::WorkBudgetExceeded {
                key,
                work: "spawn candidates",
                required,
                maximum: config.maximum_candidates_per_batch,
            });
        }
        self.candidates_examined = required;
        Ok(())
    }

    fn accept_candidate(
        &mut self,
        key: SpawnKey,
        direction: f64,
        used_fallback: bool,
        start_examined: usize,
    ) -> Result<SpawnPlacement, SpawnError> {
        let start = self.body_points.len();
        let end =
            start
                .checked_add(self.candidate_body.len())
                .ok_or(SpawnError::ArithmeticOverflow {
                    context: "accepted spawn body range",
                })?;
        self.body_points.extend_from_slice(&self.candidate_body);
        let head = self.candidate_body[0];
        Ok(SpawnPlacement {
            key,
            head,
            direction,
            body: BodyRange {
                start,
                len: end - start,
            },
            used_fallback,
            candidates_examined: self.candidates_examined - start_examined,
        })
    }

    fn prepared<'scratch, 'world>(
        &'scratch self,
        source_world: &'world WorldState,
    ) -> PreparedSpawns<'scratch, 'world> {
        PreparedSpawns {
            source_world,
            placements: &self.placements,
            body_points: &self.body_points,
            next_rng: self
                .next_rng
                .as_ref()
                .expect("ready spawn workspace must retain an RNG continuation"),
            diagnostics: SpawnCapacityDiagnostics {
                placements: self.placements.len(),
                body_points: self.body_points.len(),
                candidates_examined: self.candidates_examined,
                fallback_placements: self.fallback_placements,
                geometry_checks: self.geometry_checks,
                order_capacity: self.order.capacity(),
                source_order_capacity: self.source_order.capacity(),
                placement_capacity: self.placements.capacity(),
                body_point_capacity: self.body_points.capacity(),
                candidate_body_capacity: self.candidate_body.capacity(),
            },
        }
    }

    fn clear_staging(&mut self) {
        self.order.clear();
        self.source_order.clear();
        self.placements.clear();
        self.body_points.clear();
        self.candidate_body.clear();
        self.next_rng = None;
        self.candidates_examined = 0;
        self.fallback_placements = 0;
        self.geometry_checks = 0;
        self.ready = false;
    }
}

/// Checked placement failure that never partially mutates authority.
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnError {
    /// One projected setting was non-finite or outside the supported range.
    InvalidConfig { field: &'static str },
    /// Checked count/range arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// The caller's admitted body-point ceiling cannot fit the requested batch.
    BodyCapacityExceeded { required: usize, maximum: usize },
    /// A reusable scratch buffer could not reserve its checked requirement.
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    /// Two requests claimed the same stable placement key.
    DuplicateRequestKey(SpawnKey),
    /// A live source snake had unusable collision geometry.
    InvalidSourceSnake { snake_id: u64 },
    /// Two live source obstacles claimed the same stable identity.
    DuplicateSourceSnakeId(u64),
    /// The serialized gameplay stream failed strict restoration.
    InvalidRng(RngError),
    /// Bounded random and deterministic candidates were all unsafe.
    NoCollisionSafePlacement {
        key: SpawnKey,
        random_attempts: usize,
        fallback_candidates: usize,
    },
    /// A declared complete-batch work limit was reached before admission.
    WorkBudgetExceeded {
        key: SpawnKey,
        work: &'static str,
        required: usize,
        maximum: usize,
    },
}

impl Display for SpawnError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => write!(formatter, "invalid spawn config: {field}"),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "checked arithmetic overflow while calculating {context}")
            }
            Self::BodyCapacityExceeded { required, maximum } => write!(
                formatter,
                "spawn requires {required} new body points but the admitted maximum is {maximum}"
            ),
            Self::AllocationFailed { buffer, required } => {
                write!(formatter, "failed to reserve {required} entries for {buffer}")
            }
            Self::DuplicateRequestKey(key) => write!(
                formatter,
                "duplicate {:?} spawn key {}",
                key.domain, key.slot
            ),
            Self::InvalidSourceSnake { snake_id } => {
                write!(formatter, "source snake {snake_id} has invalid spawn-obstacle geometry")
            }
            Self::DuplicateSourceSnakeId(snake_id) => {
                write!(formatter, "duplicate live source snake ID {snake_id}")
            }
            Self::InvalidRng(error) => write!(formatter, "invalid spawn RNG: {error}"),
            Self::NoCollisionSafePlacement {
                key,
                random_attempts,
                fallback_candidates,
            } => write!(
                formatter,
                "no collision-safe placement for {:?} {} after {random_attempts} random and {fallback_candidates} deterministic candidates",
                key.domain, key.slot
            ),
            Self::WorkBudgetExceeded {
                key,
                work,
                required,
                maximum,
            } => write!(
                formatter,
                "spawn {:?} {} requires at least {required} {work}, exceeding the configured batch maximum {maximum}",
                key.domain, key.slot
            ),
        }
    }
}

impl Error for SpawnError {}

impl From<RngError> for SpawnError {
    fn from(value: RngError) -> Self {
        Self::InvalidRng(value)
    }
}

fn build_body(
    destination: &mut Vec<WorldPoint>,
    head: WorldPoint,
    direction: f64,
    config: SpawnConfig,
) -> Result<(), SpawnError> {
    destination.clear();
    if !head.x.is_finite() || !head.y.is_finite() || !direction.is_finite() {
        return Err(SpawnError::InvalidConfig {
            field: "derived spawn candidate",
        });
    }
    let step_x = direction.cos() * config.snake_spacing;
    let step_y = direction.sin() * config.snake_spacing;
    let mut point = head;
    for _ in 0..config.snake_start_len {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(SpawnError::InvalidConfig {
                field: "derived initial body",
            });
        }
        destination.push(point);
        point.x -= step_x;
        point.y -= step_y;
    }
    Ok(())
}

fn fallback_position(index: usize, config: SpawnConfig) -> WorldPoint {
    if index == 0 {
        return WorldPoint { x: 0.0, y: 0.0 };
    }
    let normalized = (index as f64 - 0.5) / config.fallback_position_count as f64;
    let radius = normalized.sqrt() * (config.world_radius * config.spawn_radius_fraction);
    let angle = index as f64 * GOLDEN_ANGLE;
    WorldPoint {
        x: angle.cos() * radius,
        y: angle.sin() * radius,
    }
}

fn candidate_is_valid(
    candidate: &[WorldPoint],
    obstacles: SpawnObstacles<'_>,
    config: SpawnConfig,
    key: SpawnKey,
    geometry_checks: &mut usize,
) -> Result<bool, SpawnError> {
    let wall_limit = config.world_radius - config.snake_radius - config.wall_clearance;
    if wall_limit <= 0.0 {
        return Ok(false);
    }
    let wall_limit_squared = wall_limit * wall_limit;
    for point in candidate {
        consume_geometry_check(geometry_checks, config, key)?;
        if point.x * point.x + point.y * point.y >= wall_limit_squared {
            return Ok(false);
        }
    }

    for source_index in obstacles.source_order {
        let snake = &obstacles.source_world.snakes[*source_index];
        let body = checked_source_body(obstacles.source_world, snake.id, snake.body)?;
        let minimum_distance = config.snake_radius + snake.radius + config.body_clearance;
        if polylines_within(
            candidate,
            body,
            minimum_distance,
            geometry_checks,
            config,
            key,
        )? {
            return Ok(false);
        }
    }
    let staged_minimum = config.snake_radius * 2.0 + config.body_clearance;
    for placement in obstacles.placements {
        let end = placement.body.start.checked_add(placement.body.len).ok_or(
            SpawnError::ArithmeticOverflow {
                context: "staged spawn body range",
            },
        )?;
        let body = obstacles
            .staged_body_points
            .get(placement.body.start..end)
            .ok_or(SpawnError::ArithmeticOverflow {
                context: "staged spawn body lookup",
            })?;
        if polylines_within(
            candidate,
            body,
            staged_minimum,
            geometry_checks,
            config,
            key,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn checked_source_body(
    world: &WorldState,
    snake_id: u64,
    range: BodyRange,
) -> Result<&[WorldPoint], SpawnError> {
    let end = range
        .start
        .checked_add(range.len)
        .ok_or(SpawnError::InvalidSourceSnake { snake_id })?;
    let body = world
        .body_points
        .get(range.start..end)
        .filter(|body| !body.is_empty())
        .ok_or(SpawnError::InvalidSourceSnake { snake_id })?;
    if body
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err(SpawnError::InvalidSourceSnake { snake_id });
    }
    Ok(body)
}

fn polylines_within(
    first: &[WorldPoint],
    second: &[WorldPoint],
    threshold: f64,
    geometry_checks: &mut usize,
    config: SpawnConfig,
    key: SpawnKey,
) -> Result<bool, SpawnError> {
    let threshold_squared = threshold * threshold;
    if first.len() == 1 && second.len() == 1 {
        consume_geometry_check(geometry_checks, config, key)?;
        return Ok(point_distance_squared(first[0], second[0]) <= threshold_squared);
    }
    if first.len() == 1 {
        for segment in second.windows(2) {
            consume_geometry_check(geometry_checks, config, key)?;
            if point_segment_distance_squared(first[0], segment[0], segment[1]) <= threshold_squared
            {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if second.len() == 1 {
        for segment in first.windows(2) {
            consume_geometry_check(geometry_checks, config, key)?;
            if point_segment_distance_squared(second[0], segment[0], segment[1])
                <= threshold_squared
            {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    for first_segment in first.windows(2) {
        for second_segment in second.windows(2) {
            consume_geometry_check(geometry_checks, config, key)?;
            if segment_segment_distance_squared(
                first_segment[0],
                first_segment[1],
                second_segment[0],
                second_segment[1],
            ) <= threshold_squared
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn consume_geometry_check(
    geometry_checks: &mut usize,
    config: SpawnConfig,
    key: SpawnKey,
) -> Result<(), SpawnError> {
    let required = geometry_checks
        .checked_add(1)
        .ok_or(SpawnError::ArithmeticOverflow {
            context: "spawn geometry comparisons",
        })?;
    if required > config.maximum_geometry_checks_per_batch {
        return Err(SpawnError::WorkBudgetExceeded {
            key,
            work: "geometry checks",
            required,
            maximum: config.maximum_geometry_checks_per_batch,
        });
    }
    *geometry_checks = required;
    Ok(())
}

fn segment_segment_distance_squared(
    first_start: WorldPoint,
    first_end: WorldPoint,
    second_start: WorldPoint,
    second_end: WorldPoint,
) -> f64 {
    if segments_intersect(first_start, first_end, second_start, second_end) {
        return 0.0;
    }
    point_segment_distance_squared(first_start, second_start, second_end)
        .min(point_segment_distance_squared(
            first_end,
            second_start,
            second_end,
        ))
        .min(point_segment_distance_squared(
            second_start,
            first_start,
            first_end,
        ))
        .min(point_segment_distance_squared(
            second_end,
            first_start,
            first_end,
        ))
}

fn segments_intersect(
    first_start: WorldPoint,
    first_end: WorldPoint,
    second_start: WorldPoint,
    second_end: WorldPoint,
) -> bool {
    let first_side = cross(first_start, first_end, second_start);
    let second_side = cross(first_start, first_end, second_end);
    let third_side = cross(second_start, second_end, first_start);
    let fourth_side = cross(second_start, second_end, first_end);
    if ((first_side > 0.0 && second_side < 0.0) || (first_side < 0.0 && second_side > 0.0))
        && ((third_side > 0.0 && fourth_side < 0.0) || (third_side < 0.0 && fourth_side > 0.0))
    {
        return true;
    }
    (first_side == 0.0 && point_on_segment(second_start, first_start, first_end))
        || (second_side == 0.0 && point_on_segment(second_end, first_start, first_end))
        || (third_side == 0.0 && point_on_segment(first_start, second_start, second_end))
        || (fourth_side == 0.0 && point_on_segment(first_end, second_start, second_end))
}

fn cross(start: WorldPoint, end: WorldPoint, point: WorldPoint) -> f64 {
    (end.x - start.x) * (point.y - start.y) - (end.y - start.y) * (point.x - start.x)
}

fn point_on_segment(point: WorldPoint, start: WorldPoint, end: WorldPoint) -> bool {
    point.x >= start.x.min(end.x)
        && point.x <= start.x.max(end.x)
        && point.y >= start.y.min(end.y)
        && point.y <= start.y.max(end.y)
}

fn point_segment_distance_squared(point: WorldPoint, start: WorldPoint, end: WorldPoint) -> f64 {
    let segment_x = end.x - start.x;
    let segment_y = end.y - start.y;
    let offset_x = point.x - start.x;
    let offset_y = point.y - start.y;
    let squared_length = segment_x * segment_x + segment_y * segment_y;
    let along = if squared_length > 0.0 {
        ((offset_x * segment_x + offset_y * segment_y) / squared_length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest_x = start.x + segment_x * along;
    let closest_y = start.y + segment_y * along;
    let dx = point.x - closest_x;
    let dy = point.y - closest_y;
    dx * dx + dy * dy
}

fn point_distance_squared(first: WorldPoint, second: WorldPoint) -> f64 {
    let dx = first.x - second.x;
    let dy = first.y - second.y;
    dx * dx + dy * dy
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), SpawnError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| SpawnError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{BrainHandle, SnakeKind, SnakeState};

    fn key(slot: u64) -> SpawnKey {
        SpawnKey {
            domain: SpawnDomain::Evolved,
            slot,
        }
    }

    fn request(slot: u64) -> SpawnRequest {
        SpawnRequest { key: key(slot) }
    }

    fn snake(id: u64, start: usize, body: &[WorldPoint], radius: f64) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("test ID should fit"),
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(u32::try_from(id - 1).expect("test slot should fit")),
            brain: Some(BrainHandle { id, epoch: 1 }),
            baseline_slot: None,
            baseline_strategy: None,
            position: body[0],
            previous_position: body[0],
            direction: 0.0,
            radius,
            speed: 0.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: body.len() as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start,
                len: body.len(),
            },
            skin: 0,
        }
    }

    fn world_with_body(body: &[WorldPoint], radius: f64) -> WorldState {
        WorldState {
            snakes: vec![snake(1, 0, body, radius)],
            body_points: body.to_vec(),
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        }
    }

    fn assert_batch_clear(prepared: &PreparedSpawns<'_, '_>, config: SpawnConfig) {
        for (index, placement) in prepared.placements().iter().enumerate() {
            let body = prepared.body_for(placement).expect("placement body");
            let wall_limit = config.world_radius - config.snake_radius - config.wall_clearance;
            for point in body {
                assert!(point.x * point.x + point.y * point.y < wall_limit * wall_limit);
            }
            for other in prepared.placements().iter().skip(index + 1) {
                let other_body = prepared.body_for(other).expect("other placement body");
                let mut geometry_checks = 0;
                assert!(!polylines_within(
                    body,
                    other_body,
                    config.snake_radius * 2.0 + config.body_clearance,
                    &mut geometry_checks,
                    config,
                    placement.key,
                )
                .expect("prepared bodies should fit the test work budget"));
            }
        }
    }

    #[test]
    fn complete_body_crossing_is_rejected_even_when_heads_are_clear() {
        let existing = [
            WorldPoint { x: 0.0, y: -30.0 },
            WorldPoint { x: 0.0, y: 30.0 },
        ];
        let world = world_with_body(&existing, 2.0);
        let candidate = [
            WorldPoint { x: 40.0, y: 0.0 },
            WorldPoint { x: -40.0, y: 0.0 },
        ];
        let config = SpawnConfig {
            snake_radius: 2.0,
            ..SpawnConfig::typescript_geometry_defaults()
        };
        let mut geometry_checks = 0;
        assert!(!candidate_is_valid(
            &candidate,
            SpawnObstacles {
                source_world: &world,
                source_order: &[0],
                placements: &[],
                staged_body_points: &[],
            },
            config,
            key(0),
            &mut geometry_checks,
        )
        .expect("finite source should validate"));
    }

    #[test]
    fn stable_request_order_owns_the_same_locations_and_rng_continuation() {
        fn run(
            requests: &[SpawnRequest],
        ) -> (Vec<SpawnPlacement>, Vec<WorldPoint>, SerializedRngState) {
            let mut workspace = SpawnWorkspace::default();
            let world = WorldState::default();
            let source = StatefulRng::new(0x5afe as f64).export_state();
            let prepared = workspace
                .prepare(
                    &world,
                    requests,
                    &source,
                    SpawnConfig::typescript_geometry_defaults(),
                    requests.len() * 5,
                )
                .expect("default world should fit");
            (
                prepared.placements().to_vec(),
                prepared.body_points().to_vec(),
                prepared.next_rng().clone(),
            )
        }

        let forward = [request(0), request(1), request(2), request(3)];
        let reversed = [request(3), request(2), request(1), request(0)];
        assert_eq!(run(&forward), run(&reversed));
    }

    #[test]
    fn first_random_candidate_matches_the_typescript_three_draw_formula() {
        let source = StatefulRng::new(0x5afe as f64).export_state();
        let config = SpawnConfig::typescript_geometry_defaults();
        // Captured by scripts/stage5/generate-spawn-fixtures.ts and retained in
        // docs/todo/evidence/stage5/typescript-spawn-fixtures.json. Keep these
        // literal values independent of Rust's candidate/body helpers.
        let expected_direction = 3.799_256_610_584_831_8;
        let expected_body = [
            WorldPoint {
                x: -641.684_079_186_302_6,
                y: 620.807_787_646_342_3,
            },
            WorldPoint {
                x: -635.748_411_621_326_1,
                y: 625.392_310_583_146_7,
            },
            WorldPoint {
                x: -629.812_744_056_349_6,
                y: 629.976_833_519_951_1,
            },
            WorldPoint {
                x: -623.877_076_491_373_1,
                y: 634.561_356_456_755_6,
            },
            WorldPoint {
                x: -617.941_408_926_396_7,
                y: 639.145_879_393_56,
            },
        ];

        let world = WorldState::default();
        let mut workspace = SpawnWorkspace::default();
        let prepared = workspace
            .prepare(&world, &[request(4)], &source, config, 5)
            .expect("first random candidate should fit an empty default world");
        let placement = prepared.placements()[0];
        assert!(!placement.used_fallback);
        assert_eq!(placement.candidates_examined, 1);
        assert!((placement.direction - expected_direction).abs() <= 1.0e-12);
        let actual_body = prepared.body_for(&placement).expect("prepared body");
        for (actual, expected) in actual_body.iter().zip(expected_body) {
            assert!((actual.x - expected.x).abs() <= 1.0e-9);
            assert!((actual.y - expected.y).abs() <= 1.0e-9);
        }
        assert_eq!(placement.head, actual_body[0]);
        assert_eq!(prepared.next_rng().state_hex, "0x9acbaf14");
        assert!(!prepared.next_rng().gaussian_spare_valid);
    }

    #[test]
    fn source_snake_container_order_cannot_change_placement() {
        fn source_world(reverse: bool) -> WorldState {
            let first = [
                WorldPoint {
                    x: -300.0,
                    y: -30.0,
                },
                WorldPoint { x: -300.0, y: 30.0 },
            ];
            let second = [
                WorldPoint { x: 300.0, y: -30.0 },
                WorldPoint { x: 300.0, y: 30.0 },
            ];
            let mut world = WorldState {
                snakes: vec![snake(1, 0, &first, 9.0), snake(2, 2, &second, 9.0)],
                body_points: first.into_iter().chain(second).collect(),
                pellets: Vec::new(),
                controller_leases: Vec::new(),
            };
            if reverse {
                world.snakes.reverse();
            }
            world
        }

        fn run(world: &WorldState) -> (Vec<SpawnPlacement>, Vec<WorldPoint>, SerializedRngState) {
            let source = StatefulRng::new(604.0).export_state();
            let mut workspace = SpawnWorkspace::default();
            let prepared = workspace
                .prepare(
                    world,
                    &[request(8), request(9)],
                    &source,
                    SpawnConfig::typescript_geometry_defaults(),
                    10,
                )
                .expect("fixture placements should fit");
            (
                prepared.placements().to_vec(),
                prepared.body_points().to_vec(),
                prepared.next_rng().clone(),
            )
        }

        assert_eq!(run(&source_world(false)), run(&source_world(true)));
    }

    #[test]
    fn blocked_candidate_and_tight_budget_remain_source_order_invariant() {
        fn run(reverse: bool) -> (Vec<SpawnPlacement>, Vec<WorldPoint>, SerializedRngState) {
            let source = StatefulRng::new(91.0).export_state();
            let mut probe = StatefulRng::from_state(&source).expect("fixture RNG");
            let config = SpawnConfig {
                world_radius: 500.0,
                snake_radius: 2.0,
                snake_spacing: 3.0,
                snake_start_len: 5,
                random_attempts_per_request: 1,
                fallback_position_count: 1,
                fallback_heading_count: 1,
                maximum_geometry_checks_per_batch: 600,
                ..SpawnConfig::typescript_geometry_defaults()
            };
            let angle = probe.next_f64() * TAU;
            let radius =
                probe.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
            let blocked_head = WorldPoint {
                x: angle.cos() * radius,
                y: angle.sin() * radius,
            };
            let _direction = probe.next_f64();
            let long_clear_body = (0..100)
                .map(|index| WorldPoint {
                    x: 10_000.0,
                    y: index as f64 * 3.0,
                })
                .collect::<Vec<_>>();
            let mut body_points = vec![blocked_head];
            body_points.extend_from_slice(&long_clear_body);
            let mut world = WorldState {
                snakes: vec![
                    snake(1, 0, &[blocked_head], 2.0),
                    snake(2, 1, &long_clear_body, 2.0),
                ],
                body_points,
                pellets: Vec::new(),
                controller_leases: Vec::new(),
            };
            if reverse {
                world.snakes.reverse();
            }
            let mut workspace = SpawnWorkspace::default();
            let prepared = workspace
                .prepare(&world, &[request(7)], &source, config, 5)
                .expect("canonical obstacle order should fit the tight budget");
            assert!(prepared.placements()[0].used_fallback);
            (
                prepared.placements().to_vec(),
                prepared.body_points().to_vec(),
                prepared.next_rng().clone(),
            )
        }

        assert_eq!(run(false), run(true));
    }

    #[test]
    fn exact_wall_tangency_is_rejected_before_the_first_movement_boundary() {
        let config = SpawnConfig::typescript_geometry_defaults();
        let candidate = [WorldPoint {
            x: config.world_radius - config.snake_radius - config.wall_clearance,
            y: 0.0,
        }];
        let mut geometry_checks = 0;
        let empty_world = WorldState::default();
        assert!(!candidate_is_valid(
            &candidate,
            SpawnObstacles {
                source_world: &empty_world,
                source_order: &[],
                placements: &[],
                staged_body_points: &[],
            },
            config,
            key(0),
            &mut geometry_checks,
        )
        .expect("tangent fixture is finite"));
    }

    #[test]
    fn sixty_four_snakes_are_complete_body_safe_or_the_batch_fails() {
        let requests = (0..64).map(request).collect::<Vec<_>>();
        let source = StatefulRng::new(0x5afe as f64).export_state();
        let world = WorldState::default();
        let config = SpawnConfig::typescript_geometry_defaults();
        let mut workspace = SpawnWorkspace::default();
        let prepared = workspace
            .prepare(&world, &requests, &source, config, requests.len() * 5)
            .expect("default arena should fit the correction fixture");
        assert_eq!(prepared.placements().len(), requests.len());
        assert_batch_clear(&prepared, config);
    }

    #[test]
    fn deterministic_fallback_is_used_after_a_blocked_random_candidate() {
        let source = StatefulRng::new(91.0).export_state();
        let mut probe = StatefulRng::from_state(&source).expect("valid fixture RNG");
        let config = SpawnConfig {
            world_radius: 500.0,
            snake_radius: 2.0,
            snake_spacing: 3.0,
            snake_start_len: 5,
            random_attempts_per_request: 1,
            fallback_position_count: 128,
            fallback_heading_count: 8,
            ..SpawnConfig::typescript_geometry_defaults()
        };
        let angle = probe.next_f64() * TAU;
        let radius = probe.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
        let blocked_head = WorldPoint {
            x: angle.cos() * radius,
            y: angle.sin() * radius,
        };
        let _heading = probe.next_f64();
        let world = world_with_body(&[blocked_head], 20.0);
        let mut workspace = SpawnWorkspace::default();
        let prepared = workspace
            .prepare(&world, &[request(7)], &source, config, 5)
            .expect("fallback should find clear space");
        assert!(prepared.placements()[0].used_fallback);
        assert!(prepared.placements()[0].candidates_examined > 1);
        assert_eq!(prepared.next_rng(), &probe.export_state());
    }

    #[test]
    fn aggregate_geometry_budget_fails_promptly_and_atomically() {
        let source = StatefulRng::new(71.0).export_state();
        let world = WorldState::default();
        let config = SpawnConfig {
            maximum_geometry_checks_per_batch: 1,
            ..SpawnConfig::typescript_geometry_defaults()
        };
        let mut workspace = SpawnWorkspace::default();
        let error = workspace
            .prepare(&world, &[request(3)], &source, config, 5)
            .expect_err("five-point wall validation must exceed one geometry check");
        assert_eq!(
            error,
            SpawnError::WorkBudgetExceeded {
                key: key(3),
                work: "geometry checks",
                required: 2,
                maximum: 1,
            }
        );
        assert!(!workspace.is_ready());
        assert_eq!(source, StatefulRng::new(71.0).export_state());
        assert_eq!(world, WorldState::default());
    }

    #[test]
    fn aggregate_candidate_budget_fails_promptly_and_atomically() {
        let source = StatefulRng::new(72.0).export_state();
        let world = WorldState::default();
        let config = SpawnConfig {
            world_radius: 20.0,
            snake_radius: 9.0,
            snake_spacing: 7.5,
            snake_start_len: 5,
            random_attempts_per_request: 1,
            fallback_position_count: 1,
            fallback_heading_count: 1,
            maximum_candidates_per_batch: 1,
            ..SpawnConfig::typescript_geometry_defaults()
        };
        let mut workspace = SpawnWorkspace::default();
        let error = workspace
            .prepare(&world, &[request(4)], &source, config, 5)
            .expect_err("the second candidate must exceed the aggregate budget");
        assert_eq!(
            error,
            SpawnError::WorkBudgetExceeded {
                key: key(4),
                work: "spawn candidates",
                required: 2,
                maximum: 1,
            }
        );
        assert!(!workspace.is_ready());
        assert_eq!(source, StatefulRng::new(72.0).export_state());
        assert_eq!(world, WorldState::default());
    }

    #[test]
    fn impossible_request_fails_without_world_or_source_rng_mutation() {
        let source = StatefulRng::new(17.0).export_state();
        let world = WorldState::default();
        let world_before = world.clone();
        let source_before = source.clone();
        let config = SpawnConfig {
            world_radius: 20.0,
            spawn_radius_fraction: 0.60,
            snake_radius: 9.0,
            snake_spacing: 7.5,
            snake_start_len: 5,
            random_attempts_per_request: 2,
            fallback_position_count: 8,
            fallback_heading_count: 4,
            ..SpawnConfig::typescript_geometry_defaults()
        };
        let mut workspace = SpawnWorkspace::default();
        let error = workspace
            .prepare(&world, &[request(0)], &source, config, 5)
            .expect_err("body cannot fit inside the tiny wall");
        assert!(matches!(
            error,
            SpawnError::NoCollisionSafePlacement { key: failed, .. } if failed == key(0)
        ));
        assert!(!workspace.is_ready());
        assert_eq!(world, world_before);
        assert_eq!(source, source_before);
    }

    #[test]
    fn duplicate_and_capacity_failures_happen_before_any_rng_result() {
        let source = StatefulRng::new(31.0).export_state();
        let world = WorldState::default();
        let mut workspace = SpawnWorkspace::default();
        let duplicate = workspace
            .prepare(
                &world,
                &[request(2), request(2)],
                &source,
                SpawnConfig::typescript_geometry_defaults(),
                10,
            )
            .expect_err("duplicate stable keys must fail");
        assert_eq!(duplicate, SpawnError::DuplicateRequestKey(key(2)));
        assert!(!workspace.is_ready());

        let capacity = workspace
            .prepare(
                &world,
                &[request(0), request(1)],
                &source,
                SpawnConfig::typescript_geometry_defaults(),
                9,
            )
            .expect_err("declared body ceiling is too small");
        assert_eq!(
            capacity,
            SpawnError::BodyCapacityExceeded {
                required: 10,
                maximum: 9,
            }
        );
        assert!(!workspace.is_ready());
    }

    #[test]
    fn repeated_batches_reuse_every_reported_capacity() {
        let source = StatefulRng::new(501.0).export_state();
        let world = WorldState::default();
        let requests = (0..24).map(request).collect::<Vec<_>>();
        let config = SpawnConfig::typescript_geometry_defaults();
        let mut workspace = SpawnWorkspace::default();
        let first = workspace
            .prepare(&world, &requests, &source, config, requests.len() * 5)
            .expect("warm batch")
            .diagnostics();
        assert!(first.order_capacity >= requests.len());
        assert!(first.placement_capacity >= requests.len());
        assert!(first.body_point_capacity >= requests.len() * 5);
        assert!(first.candidate_body_capacity >= 5);

        for _ in 0..24 {
            let next = workspace
                .prepare(&world, &requests, &source, config, requests.len() * 5)
                .expect("reused batch")
                .diagnostics();
            assert_eq!(next.order_capacity, first.order_capacity);
            assert_eq!(next.placement_capacity, first.placement_capacity);
            assert_eq!(next.body_point_capacity, first.body_point_capacity);
            assert_eq!(next.candidate_body_capacity, first.candidate_body_capacity);
        }
    }

    #[test]
    fn invalid_source_geometry_and_rng_fail_closed() {
        let body = [WorldPoint { x: 0.0, y: 0.0 }];
        let mut world = world_with_body(&body, 9.0);
        world.snakes[0].radius = f64::NAN;
        let source = StatefulRng::new(9.0).export_state();
        let mut workspace = SpawnWorkspace::default();
        assert!(matches!(
            workspace.prepare(
                &world,
                &[request(0)],
                &source,
                SpawnConfig::typescript_geometry_defaults(),
                5,
            ),
            Err(SpawnError::InvalidSourceSnake { snake_id: 1 })
        ));
        assert!(!workspace.is_ready());

        let mut invalid_rng = source;
        invalid_rng.state_hex = "0x00000000".to_owned();
        assert!(matches!(
            workspace.prepare(
                &WorldState::default(),
                &[request(0)],
                &invalid_rng,
                SpawnConfig::typescript_geometry_defaults(),
                5,
            ),
            Err(SpawnError::InvalidRng(RngError::ZeroXorshiftState))
        ));
        assert!(!workspace.is_ready());
    }
}
