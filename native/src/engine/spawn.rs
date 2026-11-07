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
    pub(crate) fn validate(self) -> Result<(), SpawnError> {
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
    /// Cached complete-body bounds used only to reject provably distant pairs.
    bounds: BodyBounds,
}

/// Complete finite polyline bounds, cached once per immutable obstacle body.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BodyBounds {
    minimum: WorldPoint,
    maximum: WorldPoint,
}

impl BodyBounds {
    /// Read a nonempty body whose complete coordinate range was already validated.
    fn for_body(body: &[WorldPoint]) -> Self {
        let mut bounds = Self {
            minimum: body[0],
            maximum: body[0],
        };
        for point in &body[1..] {
            bounds.minimum.x = bounds.minimum.x.min(point.x);
            bounds.minimum.y = bounds.minimum.y.min(point.y);
            bounds.maximum.x = bounds.maximum.x.max(point.x);
            bounds.maximum.y = bounds.maximum.y.max(point.y);
        }
        bounds
    }

    /// Reject only strict separation, padding for threshold and floating-point rounding.
    fn separated_from(self, other: Self, threshold: f64) -> bool {
        let scale = self
            .minimum
            .x
            .abs()
            .max(self.minimum.y.abs())
            .max(self.maximum.x.abs())
            .max(self.maximum.y.abs())
            .max(other.minimum.x.abs())
            .max(other.minimum.y.abs())
            .max(other.maximum.x.abs())
            .max(other.maximum.y.abs())
            .max(threshold.abs())
            .max(1.0);
        // Near-touching boxes stay on the exact path. Overflow also leaves the
        // comparison on that path rather than admitting a falsely clear body.
        let padding = threshold + 16.0 * f64::EPSILON * scale;
        self.maximum.x + padding < other.minimum.x
            || other.maximum.x + padding < self.minimum.x
            || self.maximum.y + padding < other.minimum.y
            || other.maximum.y + padding < self.minimum.y
    }
}

/// Canonical live-source identity together with its validated, complete body bounds.
#[derive(Clone, Copy, Debug)]
struct SourceObstacle {
    index: usize,
    bounds: BodyBounds,
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
#[derive(Debug, Default)]
pub struct SpawnWorkspace {
    order: Vec<usize>,
    source_order: Vec<SourceObstacle>,
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
    source_order: &'a [SourceObstacle],
    placements: &'a [SpawnPlacement],
    staged_body_points: &'a [WorldPoint],
}

/// Bounded evolved-only placement outcome; legacy spawn errors retain their meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvolvedPlacementStatus {
    Placed,
    /// No clear body in this slot's complete candidate allowance.
    PendingNoPlacement,
    /// The remaining batch candidate or geometry allowance prevented completion.
    PendingWorkLimit,
}

/// A complete evolved attempt, including failure draws and bounded fallback progress.
pub struct PreparedEvolvedPlacement<'scratch, 'world> {
    status: EvolvedPlacementStatus,
    spawns: PreparedSpawns<'scratch, 'world>,
    fallback_cursor: u64,
}

impl<'scratch, 'world> PreparedEvolvedPlacement<'scratch, 'world> {
    #[must_use]
    pub const fn status(&self) -> EvolvedPlacementStatus {
        self.status
    }
    #[must_use]
    pub const fn spawns(&self) -> &PreparedSpawns<'scratch, 'world> {
        &self.spawns
    }
    #[must_use]
    pub const fn fallback_cursor(&self) -> u64 {
        self.fallback_cursor
    }
}

impl SpawnWorkspace {
    /// Actual retained placement allocations charged to cold transaction scratch.
    pub(crate) fn owned_bytes(&self) -> Result<usize, String> {
        let bytes = [
            (self.order.capacity(), std::mem::size_of::<usize>()),
            (
                self.source_order.capacity(),
                std::mem::size_of::<SourceObstacle>(),
            ),
            (
                self.placements.capacity(),
                std::mem::size_of::<SpawnPlacement>(),
            ),
            (
                self.body_points.capacity(),
                std::mem::size_of::<WorldPoint>(),
            ),
            (
                self.candidate_body.capacity(),
                std::mem::size_of::<WorldPoint>(),
            ),
        ]
        .into_iter()
        .try_fold(std::mem::size_of::<Self>(), |total, (count, width)| {
            count
                .checked_mul(width)
                .and_then(|owned| total.checked_add(owned))
        });
        let rng_bytes = self.next_rng.as_ref().map_or(0, |rng| {
            rng.algorithm.capacity()
                + rng.state_hex.capacity()
                + rng.gaussian_algorithm.capacity()
                + rng.gaussian_spare_hex.as_ref().map_or(0, String::capacity)
        });
        bytes
            .and_then(|owned| owned.checked_add(rng_bytes))
            .ok_or_else(|| "spawn allocation accounting overflowed".into())
    }

    /// Attempt one evolved replacement with fresh complete geometry checks. Only
    /// this API turns congestion/work exhaustion into a pending continuation.
    /// It allocates no physical IDs and never changes source world/RNG state.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_evolved_replacement<'scratch, 'world>(
        &'scratch mut self,
        source_world: &'world WorldState,
        slot: u32,
        source_rng: &SerializedRngState,
        config: SpawnConfig,
        fallback_cursor: u64,
        remaining_batch_candidates: usize,
        remaining_batch_geometry: usize,
        maximum_new_body_points: usize,
    ) -> Result<PreparedEvolvedPlacement<'scratch, 'world>, SpawnError> {
        self.clear_staging();
        config.validate()?;
        self.prepare_source_order(source_world)?;
        if config.snake_start_len > maximum_new_body_points {
            return Err(SpawnError::BodyCapacityExceeded {
                required: config.snake_start_len,
                maximum: maximum_new_body_points,
            });
        }
        let fallback_count = config
            .fallback_position_count
            .checked_mul(config.fallback_heading_count)
            .ok_or(SpawnError::ArithmeticOverflow {
                context: "evolved fallback count",
            })?;
        let mut cursor = usize::try_from(fallback_cursor)
            .ok()
            .filter(|cursor| *cursor < fallback_count)
            .ok_or(SpawnError::InvalidConfig {
                field: "evolved fallback cursor",
            })?;
        reserve_for(
            &mut self.candidate_body,
            config.snake_start_len,
            "evolved candidate body",
        )?;
        reserve_for(
            &mut self.body_points,
            config.snake_start_len,
            "evolved placement body",
        )?;
        reserve_for(&mut self.placements, 1, "evolved placement")?;
        let key = SpawnKey {
            domain: SpawnDomain::Evolved,
            slot: u64::from(slot),
        };
        let limit = if remaining_batch_geometry == 0 {
            0
        } else {
            remaining_batch_candidates.min(256)
        };
        let mut geometry_config = config;
        geometry_config.maximum_geometry_checks_per_batch = remaining_batch_geometry.min(250_000);
        let mut rng = StatefulRng::from_state(source_rng)?;
        let mut status = if limit < 256 {
            EvolvedPlacementStatus::PendingWorkLimit
        } else {
            EvolvedPlacementStatus::PendingNoPlacement
        };
        for candidate_index in 0..limit {
            let fallback = candidate_index >= 32.min(config.random_attempts_per_request);
            let (head, direction) = if fallback {
                (
                    fallback_position(cursor / config.fallback_heading_count, config),
                    (cursor % config.fallback_heading_count) as f64 * TAU
                        / config.fallback_heading_count as f64,
                )
            } else {
                let angle = rng.next_f64() * TAU;
                let radius =
                    rng.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
                let head = WorldPoint {
                    x: angle.cos() * radius,
                    y: angle.sin() * radius,
                };
                (head, rng.next_f64() * TAU)
            };
            self.candidates_examined += 1;
            build_body(&mut self.candidate_body, head, direction, config)?;
            let accepted = match candidate_is_valid(
                &self.candidate_body,
                SpawnObstacles {
                    source_world,
                    source_order: &self.source_order,
                    placements: &[],
                    staged_body_points: &[],
                },
                geometry_config,
                key,
                &mut self.geometry_checks,
            ) {
                Ok(accepted) => accepted,
                Err(SpawnError::WorkBudgetExceeded {
                    work: "geometry checks",
                    ..
                }) => {
                    status = EvolvedPlacementStatus::PendingWorkLimit;
                    break;
                }
                Err(error) => return Err(error),
            };
            if fallback {
                cursor = (cursor + 1) % fallback_count;
            }
            if accepted {
                let placement = self.accept_candidate(key, direction, fallback, 0)?;
                self.placements.push(placement);
                self.fallback_placements = usize::from(fallback);
                status = EvolvedPlacementStatus::Placed;
                break;
            }
        }
        self.next_rng = Some(rng.export_state());
        self.ready = status == EvolvedPlacementStatus::Placed;
        Ok(PreparedEvolvedPlacement {
            status,
            spawns: self.prepared(source_world),
            fallback_cursor: cursor as u64,
        })
    }

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

    /// Latest work and retained capacity, including after a rejected batch.
    #[must_use]
    pub const fn diagnostics(&self) -> SpawnCapacityDiagnostics {
        SpawnCapacityDiagnostics {
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
        }
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
            let body = checked_source_body(source_world, snake.id, snake.body)?;
            self.source_order.push(SourceObstacle {
                index,
                bounds: BodyBounds::for_body(body),
            });
        }
        self.source_order
            .sort_unstable_by_key(|obstacle| source_world.snakes[obstacle.index].id);
        for pair in self.source_order.windows(2) {
            let left = source_world.snakes[pair[0].index].id;
            let right = source_world.snakes[pair[1].index].id;
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
            bounds: BodyBounds::for_body(&self.candidate_body),
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
            diagnostics: self.diagnostics(),
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

    let candidate_bounds = BodyBounds::for_body(candidate);
    for source in obstacles.source_order {
        let snake = &obstacles.source_world.snakes[source.index];
        let minimum_distance = config.snake_radius + snake.radius + config.body_clearance;
        consume_geometry_check(geometry_checks, config, key)?;
        if candidate_bounds.separated_from(source.bounds, minimum_distance) {
            continue;
        }
        // prepare_source_order validated this range and every coordinate once;
        // the same source world stays immutably borrowed throughout the batch.
        let body = &obstacles.source_world.body_points
            [snake.body.start..snake.body.start + snake.body.len];
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
        consume_geometry_check(geometry_checks, config, key)?;
        if candidate_bounds.separated_from(placement.bounds, staged_minimum) {
            continue;
        }
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
mod tests;
