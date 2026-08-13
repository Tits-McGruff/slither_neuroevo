//! Corrected sensor-v3 construction over one stable pre-movement world view.
//!
//! Formulas and label order are mapped from `src/sensors.ts` and
//! `src/protocol/sensors.ts`. The Rust path intentionally corrects the current
//! midpoint-only body lookup by consuming
//! [`BodySpatialIndex`](crate::engine::spatial::BodySpatialIndex).
//! Pure probes return a delivery marker without mutating snake state; only a
//! caller that actually consumes or accepts the observation commits it.

use super::sensor_layout::{SensorLayout, SensorLayoutError};
use super::spatial::{
    BodySensorQueryScratch, IndexedSensorWorld, PelletQueryScratch, SpatialIndexError,
    SpatialQueryDiagnostics,
};
use super::state::{SnakeKind, SnakeState, WorldPoint, WorldState};
use std::error::Error;
use std::f64::consts::{PI, TAU};
use std::fmt;

/// Saturating divisor for score changes since the prior delivered sample.
const POINTS_DELTA_SCALE: f64 = 10.0;
/// Current TypeScript minimum work floor for pellet queries.
const MIN_EFFECTIVE_PELLET_CHECKS: usize = 120;
/// Current TypeScript minimum work floor for body-segment queries.
const MIN_EFFECTIVE_SEGMENT_CHECKS: usize = 200;

/// Complete validated inputs used by the current v3 sensor formulas.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorConfig {
    /// Number of angular bins in each of four channels.
    pub bins: usize,
    /// Arena radius.
    pub world_radius: f64,
    /// Default food value used to normalize pellet strength.
    pub food_value: f64,
    /// Boost-speed denominator for the speed scalar.
    pub snake_boost_speed: f64,
    /// Initial body-point count used by size normalization.
    pub snake_start_length: usize,
    /// Maximum body-point count used by size/length normalization.
    pub snake_max_length: usize,
    /// Points required before boost is allowed.
    pub minimum_boost_points: f64,
    /// Base boost cost per simulated second.
    pub boost_points_cost_per_second: f64,
    /// Size multiplier applied to boost cost.
    pub boost_points_cost_size_factor: f64,
    /// Configured generation length for the age scalar.
    pub generation_seconds: f64,
    /// Near-radius base.
    pub near_radius_base: f64,
    /// Near-radius size multiplier.
    pub near_radius_scale: f64,
    /// Near-radius lower clamp.
    pub near_radius_minimum: f64,
    /// Near-radius upper clamp.
    pub near_radius_maximum: f64,
    /// Far-radius base.
    pub far_radius_base: f64,
    /// Far-radius size multiplier.
    pub far_radius_scale: f64,
    /// Far-radius lower clamp.
    pub far_radius_minimum: f64,
    /// Far-radius upper clamp.
    pub far_radius_maximum: f64,
    /// Food-density saturation constant.
    pub food_saturation: f64,
    /// Collision radius multiplier used by hazard/head bins.
    pub collision_hit_scale: f64,
    /// Configured sensor-only pellet detail limit.
    pub maximum_pellet_checks: usize,
    /// Configured sensor-only body detail limit.
    pub maximum_segment_checks: usize,
}

impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            bins: 16,
            world_radius: 3_500.0,
            food_value: 1.0,
            snake_boost_speed: 500.0,
            snake_start_length: 5,
            snake_max_length: 10_000,
            minimum_boost_points: 1.2,
            boost_points_cost_per_second: 7.0,
            boost_points_cost_size_factor: 1.1,
            generation_seconds: 240.0,
            near_radius_base: 520.0,
            near_radius_scale: 260.0,
            near_radius_minimum: 420.0,
            near_radius_maximum: 1_100.0,
            far_radius_base: 1_200.0,
            far_radius_scale: 520.0,
            far_radius_minimum: 900.0,
            far_radius_maximum: 2_400.0,
            food_saturation: 4.0,
            collision_hit_scale: 0.82,
            maximum_pellet_checks: 900,
            maximum_segment_checks: 2_200,
        }
    }
}

impl SensorConfig {
    /// Validate the full sensor configuration before it reaches a hot loop.
    pub fn validate(&self) -> Result<(), SensorError> {
        SensorLayout::new(self.bins)?;
        for (field, value, minimum, maximum) in [
            ("world_radius", self.world_radius, 800.0, 10_000.0),
            ("food_value", self.food_value, 0.1, 8.0),
            ("snake_boost_speed", self.snake_boost_speed, 40.0, 1_200.0),
            ("minimum_boost_points", self.minimum_boost_points, 0.0, 60.0),
            (
                "boost_points_cost_per_second",
                self.boost_points_cost_per_second,
                0.0,
                80.0,
            ),
            (
                "boost_points_cost_size_factor",
                self.boost_points_cost_size_factor,
                0.0,
                4.0,
            ),
            ("generation_seconds", self.generation_seconds, 8.0, 480.0),
            ("near_radius_base", self.near_radius_base, 200.0, 900.0),
            ("near_radius_scale", self.near_radius_scale, 0.0, 600.0),
            (
                "near_radius_minimum",
                self.near_radius_minimum,
                150.0,
                900.0,
            ),
            (
                "near_radius_maximum",
                self.near_radius_maximum,
                200.0,
                1_200.0,
            ),
            ("far_radius_base", self.far_radius_base, 400.0, 2_000.0),
            ("far_radius_scale", self.far_radius_scale, 0.0, 1_200.0),
            (
                "far_radius_minimum",
                self.far_radius_minimum,
                400.0,
                2_200.0,
            ),
            (
                "far_radius_maximum",
                self.far_radius_maximum,
                600.0,
                3_000.0,
            ),
            ("food_saturation", self.food_saturation, 0.5, 12.0),
            ("collision_hit_scale", self.collision_hit_scale, 0.45, 1.2),
        ] {
            require_config_range(field, value, minimum, maximum)?;
        }
        if self.snake_start_length < 5
            || self.snake_start_length > 140
            || self.snake_max_length < 60
            || self.snake_max_length > 100_000
            || self.snake_max_length < self.snake_start_length
            || self.near_radius_maximum < self.near_radius_minimum
            || self.far_radius_maximum < self.far_radius_minimum
            || self.maximum_pellet_checks < 100
            || self.maximum_pellet_checks > 3_000
            || self.maximum_segment_checks < 200
            || self.maximum_segment_checks > 4_000
        {
            return Err(SensorError::InvalidConfig {
                field: "sensor ranges",
            });
        }
        Ok(())
    }
}

/// Near/far radii after size scaling and monotonic clamps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorRadii {
    /// Body, wall, and other-head query radius.
    pub near: f64,
    /// Pellet query radius.
    pub far: f64,
}

/// Generation-scoped sensor state initialized before the first sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SensorGenerationState {
    best_points_this_generation: f64,
}

impl SensorGenerationState {
    /// Construct an explicitly initialized generation sensor state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the generation best before construction/reset/import sampling.
    pub fn reset(&mut self) {
        self.best_points_this_generation = 0.0;
    }

    /// Preserve the maximum score among alive evolved population members.
    pub fn update_after_step(&mut self, world: &WorldState) -> Result<(), SensorError> {
        let mut current = 0.0_f64;
        for snake in &world.snakes {
            if snake.alive && snake.kind == SnakeKind::Evolved {
                if !snake.points.is_finite() || snake.points < 0.0 {
                    return Err(SensorError::InvalidWorldValue {
                        field: "snake.points",
                        snake_id: snake.id,
                    });
                }
                current = current.max(snake.points);
            }
        }
        self.best_points_this_generation = self.best_points_this_generation.max(current);
        Ok(())
    }

    /// Current monotonic generation best supplied to observation formulas.
    #[must_use]
    pub fn best_points_this_generation(&self) -> f64 {
        self.best_points_this_generation
    }
}

/// One pure sample's exact score-boundary commit token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObservationDeliveryMarker {
    /// Stable snake identity sampled.
    pub snake_id: u64,
    /// Boundary value that must still be current when this marker commits.
    pub previous_delivered_points: f64,
    /// Snake points captured at observation construction.
    pub sampled_points: f64,
}

impl ObservationDeliveryMarker {
    /// Commit the sampled score boundary after real internal consumption or
    /// successful Node acceptance for the matching external assignment.
    pub fn commit(self, snake: &mut SnakeState) -> Result<(), SensorError> {
        if snake.id != self.snake_id {
            return Err(SensorError::DeliverySnakeMismatch {
                expected: self.snake_id,
                actual: snake.id,
            });
        }
        if snake.delivered_observation_points.to_bits() != self.previous_delivered_points.to_bits()
        {
            return Err(SensorError::StaleDeliveryMarker { snake_id: snake.id });
        }
        snake.delivered_observation_points = self.sampled_points;
        Ok(())
    }
}

/// Per-sample work and saturation counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SensorSampleDiagnostics {
    /// Pellet spatial query work.
    pub pellet_query: SpatialQueryDiagnostics,
    /// Body spatial query work.
    pub body_query: SpatialQueryDiagnostics,
    /// Detailed pellet candidates evaluated after nearest-first ordering.
    pub pellet_checks: usize,
    /// Detailed non-self segment candidates evaluated.
    pub segment_checks: usize,
    /// Other live heads evaluated.
    pub head_checks: usize,
    /// Number of times this sample hit its pellet detail limit.
    pub pellet_cap_hits: u64,
    /// Number of times this sample hit its segment detail limit.
    pub segment_cap_hits: u64,
    /// Body cap forced conservative hazard values instead of false-clear bins.
    pub conservative_body_saturation: bool,
    /// Target head was outside the configured arena radius.
    pub target_outside_world: bool,
}

/// Reusable per-worker scratch for one observation at a time.
#[derive(Clone, Debug, Default)]
pub struct SensorScratch {
    food_bins: Vec<f32>,
    hazard_bins: Vec<f32>,
    head_bins: Vec<f32>,
    /// Reusable body-query duplicate and ordering storage.
    pub body_query: BodySensorQueryScratch,
    /// Reusable pellet-query ordering storage.
    pub pellet_query: PelletQueryScratch,
}

/// Reusable sensor-scratch capacities used to prove warm-path stability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SensorScratchDiagnostics {
    /// Food-bin Float32 slots.
    pub food_bin_capacity: usize,
    /// Body-hazard-bin Float32 slots.
    pub hazard_bin_capacity: usize,
    /// Other-head-bin Float32 slots.
    pub head_bin_capacity: usize,
    /// Body duplicate-marker slots.
    pub body_duplicate_marker_capacity: usize,
    /// Bounded body candidate slots.
    pub body_candidate_capacity: usize,
    /// Bounded pellet candidate slots.
    pub pellet_candidate_capacity: usize,
}

impl SensorScratch {
    /// Report owned capacities without allocating or changing query state.
    #[must_use]
    pub fn diagnostics(&self) -> SensorScratchDiagnostics {
        SensorScratchDiagnostics {
            food_bin_capacity: self.food_bins.capacity(),
            hazard_bin_capacity: self.hazard_bins.capacity(),
            head_bin_capacity: self.head_bins.capacity(),
            body_duplicate_marker_capacity: self.body_query.duplicate_marker_capacity(),
            body_candidate_capacity: self.body_query.candidate_capacity(),
            pellet_candidate_capacity: self.pellet_query.candidate_capacity(),
        }
    }

    fn prepare_bins(&mut self, bins: usize) -> Result<(), SensorError> {
        resize_scratch(&mut self.food_bins, bins, "food bins")?;
        resize_scratch(&mut self.hazard_bins, bins, "hazard bins")?;
        resize_scratch(&mut self.head_bins, bins, "head bins")?;
        self.food_bins.fill(0.0);
        self.hazard_bins.fill(0.0);
        self.head_bins.fill(0.0);
        Ok(())
    }
}

/// Successful pure observation construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorSample {
    /// Token committed only when this observation is actually delivered.
    pub delivery: ObservationDeliveryMarker,
    /// Work/saturation measurements for this sample.
    pub diagnostics: SensorSampleDiagnostics,
    /// Size-scaled query radii used for this sample.
    pub radii: SensorRadii,
}

/// Validated reusable sensor-v3 evaluator.
#[derive(Clone, Debug)]
pub struct SensorEvaluator {
    config: SensorConfig,
    layout: SensorLayout,
}

impl SensorEvaluator {
    /// Validate a configuration and prepare its immutable layout once.
    pub fn new(config: SensorConfig) -> Result<Self, SensorError> {
        config.validate()?;
        let layout = SensorLayout::new(config.bins)?;
        Ok(Self { config, layout })
    }

    /// Read the exact layout required by this evaluator.
    #[must_use]
    pub fn layout(&self) -> &SensorLayout {
        &self.layout
    }

    /// Read the validated formula configuration.
    #[must_use]
    pub fn config(&self) -> &SensorConfig {
        &self.config
    }

    /// Effective per-sample pellet-detail ceiling after the safety floor.
    #[must_use]
    pub fn effective_pellet_limit(&self) -> usize {
        self.config
            .maximum_pellet_checks
            .max(MIN_EFFECTIVE_PELLET_CHECKS)
    }

    /// Effective per-sample body-detail ceiling after the safety floor.
    #[must_use]
    pub fn effective_segment_limit(&self) -> usize {
        self.config
            .maximum_segment_checks
            .max(MIN_EFFECTIVE_SEGMENT_CHECKS)
    }

    /// Compute size-dependent near/far radii with the current clamps.
    #[must_use]
    pub fn radii(&self, size_normalized: f64) -> SensorRadii {
        let size = clamp(
            if size_normalized.is_finite() {
                size_normalized
            } else {
                0.0
            },
            0.0,
            1.0,
        );
        let near = clamp(
            self.config.near_radius_base + self.config.near_radius_scale * size,
            self.config.near_radius_minimum,
            self.config.near_radius_maximum,
        );
        let mut far = clamp(
            self.config.far_radius_base + self.config.far_radius_scale * size,
            self.config.far_radius_minimum,
            self.config.far_radius_maximum,
        );
        if far < near + 1.0 {
            far = near + 1.0;
        }
        SensorRadii { near, far }
    }

    /// Build one complete pure observation from a stable indexed world view.
    ///
    /// `output` must have exactly [`SensorLayout::input_size`] values. No
    /// authoritative state changes until the returned delivery marker commits.
    pub fn sample(
        &self,
        indexed_world: &IndexedSensorWorld<'_>,
        generation: &SensorGenerationState,
        snake_index: usize,
        output: &mut [f32],
        scratch: &mut SensorScratch,
    ) -> Result<SensorSample, SensorError> {
        if output.len() != self.layout.input_size {
            return Err(SensorError::OutputLength {
                expected: self.layout.input_size,
                actual: output.len(),
            });
        }
        let world = indexed_world.world();
        let snake = world
            .snakes
            .get(snake_index)
            .ok_or(SensorError::UnknownSnakeIndex { snake_index })?;
        validate_snake_for_sensing(snake)?;
        if !generation.best_points_this_generation.is_finite()
            || generation.best_points_this_generation < 0.0
        {
            return Err(SensorError::InvalidGenerationBest);
        }
        scratch.prepare_bins(self.layout.bins)?;
        output.fill(0.0);

        let length = snake.body.len;
        let size_denominator = self
            .config
            .snake_max_length
            .saturating_sub(self.config.snake_start_length)
            .max(1) as f64;
        let size_normalized = clamp(
            (length as f64 - self.config.snake_start_length as f64) / size_denominator,
            0.0,
            1.0,
        );
        let radii = self.radii(size_normalized);
        let best_points = generation.best_points_this_generation.max(0.001);

        output[0] = snake.direction.sin() as f32;
        output[1] = snake.direction.cos() as f32;
        output[2] = clamp(size_normalized * 2.0 - 1.0, -1.0, 1.0) as f32;
        let boost_margin = snake.points - self.config.minimum_boost_points;
        output[3] = clamp(
            boost_margin / self.config.minimum_boost_points.max(1.0e-6),
            -1.0,
            1.0,
        ) as f32;
        let logarithmic_fraction = (1.0 + snake.points).ln() / (1.0 + best_points).ln();
        output[4] = clamp(logarithmic_fraction * 2.0 - 1.0, -1.0, 1.0) as f32;
        output[5] =
            ratio_to_bipolar(snake.speed / self.config.snake_boost_speed.max(1.0e-6)) as f32;
        output[6] = if snake.boost { 1.0 } else { -1.0 };
        output[7] = clamp(2.0 * (snake.points / best_points.max(1.0)) - 1.0, -1.0, 1.0) as f32;
        output[8] = clamp(
            (snake.points - snake.delivered_observation_points) / POINTS_DELTA_SCALE,
            -1.0,
            1.0,
        ) as f32;
        output[9] = clamp(
            2.0 * (length as f64 / self.config.snake_max_length.max(1) as f64) - 1.0,
            -1.0,
            1.0,
        ) as f32;
        output[10] = clamp(
            (snake.points - self.config.minimum_boost_points)
                / self.config.minimum_boost_points.max(1.0),
            -1.0,
            1.0,
        ) as f32;
        let effective_cost = self.config.boost_points_cost_per_second
            * (1.0 + self.config.boost_points_cost_size_factor * size_normalized);
        let cost_scale = self.config.boost_points_cost_per_second * 3.0;
        output[11] = clamp(
            2.0 * (effective_cost / cost_scale.max(1.0)) - 1.0,
            -1.0,
            1.0,
        ) as f32;
        let distance_to_center = snake.position.x.hypot(snake.position.y);
        let wall_distance = self.config.world_radius - distance_to_center;
        output[12] = clamp(
            2.0 * (wall_distance / self.config.world_radius) - 1.0,
            -1.0,
            1.0,
        ) as f32;

        let effective_pellet_limit = self.effective_pellet_limit();
        let pellet_query = indexed_world.pellet_index().collect_sensor_candidates(
            snake.position,
            radii.far,
            effective_pellet_limit,
            &mut scratch.pellet_query,
        )?;
        let pellet_checks = pellet_query.candidates;
        let pellet_capped = pellet_query.candidate_limit_reached;
        let mut nearest_food_distance = radii.far;
        let mut nearest_food_delta = WorldPoint { x: 0.0, y: 0.0 };
        let mut found_food = false;
        for pellet in indexed_world
            .pellet_index()
            .candidates(&scratch.pellet_query)
        {
            let dx = pellet.position.x - snake.position.x;
            let dy = pellet.position.y - snake.position.y;
            let distance_squared = dx * dx + dy * dy;
            let distance = distance_squared.sqrt();
            if !found_food || distance < nearest_food_distance {
                nearest_food_distance = distance;
                nearest_food_delta = WorldPoint { x: dx, y: dy };
                found_food = true;
            }
            if distance_squared <= 1.0e-6 || distance_squared > radii.far * radii.far {
                continue;
            }
            let relative = normalize_angle(dy.atan2(dx) - snake.direction);
            let bin = angle_to_centered_bin(relative, self.layout.bins);
            let distance_weight = 1.0 - distance / radii.far;
            let value_weight = clamp(pellet.value / self.config.food_value.max(1.0e-6), 0.0, 6.0);
            scratch.food_bins[bin] =
                (f64::from(scratch.food_bins[bin]) + distance_weight * value_weight) as f32;
        }
        output[13] = clamp(
            2.0 * (1.0 - nearest_food_distance / radii.far) - 1.0,
            -1.0,
            1.0,
        ) as f32;
        if found_food && nearest_food_distance > 1.0e-6 {
            let relative =
                normalize_angle(nearest_food_delta.y.atan2(nearest_food_delta.x) - snake.direction);
            output[14] = relative.sin() as f32;
            output[15] = relative.cos() as f32;
        }
        for (index, strength) in scratch.food_bins.iter().copied().enumerate() {
            let strength = f64::from(strength);
            let fraction = strength / (strength + self.config.food_saturation.max(0.1));
            output[self.layout.offsets.food + index] = ratio_to_bipolar(fraction) as f32;
        }

        scratch.hazard_bins.fill(radii.near as f32);
        let broad_body_radius = radii.near
            + (snake.radius + indexed_world.body_index().maximum_owner_radius())
                * self.config.collision_hit_scale.max(1.0);
        let effective_segment_limit = self.effective_segment_limit();
        let body_query = indexed_world.body_index().collect_sensor_candidates(
            snake.position,
            broad_body_radius,
            snake.id,
            effective_segment_limit,
            &mut scratch.body_query,
        )?;
        let segment_capped = body_query.candidate_limit_reached;
        let segment_checks = if segment_capped {
            0
        } else {
            body_query.candidates
        };
        let mut nearest_body_distance = radii.near;
        if segment_capped {
            // The retained prefix cannot change the conservative capped result,
            // so avoid exact calculations whose values would be overwritten.
            nearest_body_distance = 0.0;
            scratch.hazard_bins.fill(0.0);
        } else {
            for segment in indexed_world
                .body_index()
                .sensor_candidates(&scratch.body_query)
            {
                let closest = closest_point_on_segment(snake.position, segment.start, segment.end);
                let distance = closest.distance_squared.sqrt();
                let raw_clearance = (distance - (snake.radius + segment.owner_radius)).max(0.0);
                nearest_body_distance = nearest_body_distance.min(raw_clearance);

                let hit_threshold =
                    (snake.radius + segment.owner_radius) * self.config.collision_hit_scale;
                let maximum_distance = radii.near + hit_threshold;
                if closest.distance_squared > maximum_distance * maximum_distance {
                    continue;
                }
                let clearance = (distance - hit_threshold).max(0.0);
                if clearance > radii.near {
                    continue;
                }
                let relative = normalize_angle(
                    (closest.point.y - snake.position.y).atan2(closest.point.x - snake.position.x)
                        - snake.direction,
                );
                let bin = angle_to_centered_bin(relative, self.layout.bins);
                if clearance < f64::from(scratch.hazard_bins[bin]) {
                    scratch.hazard_bins[bin] = clearance as f32;
                }
            }
        }
        output[16] = clamp(
            2.0 * (1.0 - nearest_body_distance / radii.near) - 1.0,
            -1.0,
            1.0,
        ) as f32;
        for (index, clearance) in scratch.hazard_bins.iter().copied().enumerate() {
            output[self.layout.offsets.hazard + index] =
                ratio_to_bipolar(f64::from(clearance) / radii.near) as f32;
        }

        let mut nearest_head_distance = radii.near;
        scratch.head_bins.fill(radii.near as f32);
        let mut head_checks = 0usize;
        for other in &world.snakes {
            if !other.alive || other.id == snake.id {
                continue;
            }
            head_checks = head_checks.saturating_add(1);
            let dx = other.position.x - snake.position.x;
            let dy = other.position.y - snake.position.y;
            let distance_squared = dx * dx + dy * dy;
            let distance = distance_squared.sqrt();
            let raw_clearance = (distance - (snake.radius + other.radius)).max(0.0);
            nearest_head_distance = nearest_head_distance.min(raw_clearance);
            if distance_squared > radii.near * radii.near {
                continue;
            }
            let hit_threshold = (snake.radius + other.radius) * self.config.collision_hit_scale;
            let clearance = (distance - hit_threshold).max(0.0);
            let relative = normalize_angle(dy.atan2(dx) - snake.direction);
            let bin = angle_to_centered_bin(relative, self.layout.bins);
            if clearance < f64::from(scratch.head_bins[bin]) {
                scratch.head_bins[bin] = clearance as f32;
            }
        }
        output[17] = clamp(
            2.0 * (1.0 - nearest_head_distance / radii.near) - 1.0,
            -1.0,
            1.0,
        ) as f32;
        for (index, clearance) in scratch.head_bins.iter().copied().enumerate() {
            output[self.layout.offsets.head + index] =
                ratio_to_bipolar(f64::from(clearance) / radii.near) as f32;
        }

        let target_outside_world = distance_to_center > self.config.world_radius;
        for index in 0..self.layout.bins {
            let theta = snake.direction + centered_bin_to_angle(index, self.layout.bins);
            let mut distance =
                distance_to_wall_along_ray(snake.position, theta, self.config.world_radius);
            if !distance.is_finite() || distance <= 0.0 {
                distance = 0.0;
            }
            let clearance = clamp(distance - snake.radius, 0.0, radii.near);
            output[self.layout.offsets.wall + index] =
                ratio_to_bipolar(clearance / radii.near) as f32;
        }

        let age_ratio = (snake.age_seconds / self.config.generation_seconds.max(1.0)).min(1.0);
        output[18] = clamp(2.0 * age_ratio - 1.0, -1.0, 1.0) as f32;
        if output.iter().any(|value| !value.is_finite()) {
            return Err(SensorError::NonFiniteOutput { snake_id: snake.id });
        }
        Ok(SensorSample {
            delivery: ObservationDeliveryMarker {
                snake_id: snake.id,
                previous_delivered_points: snake.delivered_observation_points,
                sampled_points: snake.points,
            },
            diagnostics: SensorSampleDiagnostics {
                pellet_query,
                body_query,
                pellet_checks,
                segment_checks,
                head_checks,
                pellet_cap_hits: u64::from(pellet_capped),
                segment_cap_hits: u64::from(segment_capped),
                conservative_body_saturation: segment_capped,
                target_outside_world,
            },
            radii,
        })
    }
}

/// Sensor configuration, world-view, or delivery-boundary failure.
#[derive(Clone, Debug, PartialEq)]
pub enum SensorError {
    /// Sensor-layout validation failed.
    Layout(SensorLayoutError),
    /// Spatial query/index validation failed.
    Spatial(SpatialIndexError),
    /// A formula setting was invalid.
    InvalidConfig { field: &'static str },
    /// Caller output length disagreed with the layout.
    OutputLength { expected: usize, actual: usize },
    /// Dense snake index was absent from the stable world view.
    UnknownSnakeIndex { snake_index: usize },
    /// Required snake state was invalid.
    InvalidWorldValue { field: &'static str, snake_id: u64 },
    /// Generation best was not initialized to a finite non-negative value.
    InvalidGenerationBest,
    /// A computed Float32 observation contained a non-finite value.
    NonFiniteOutput { snake_id: u64 },
    /// A marker was offered to a different snake.
    DeliverySnakeMismatch { expected: u64, actual: u64 },
    /// A later observation already advanced this snake's delivery boundary.
    StaleDeliveryMarker { snake_id: u64 },
    /// Fallible reusable scratch growth failed.
    ScratchAllocation {
        context: &'static str,
        requested: usize,
    },
}

impl From<SensorLayoutError> for SensorError {
    fn from(error: SensorLayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<SpatialIndexError> for SensorError {
    fn from(error: SpatialIndexError) -> Self {
        Self::Spatial(error)
    }
}

impl fmt::Display for SensorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(formatter, "{error}"),
            Self::Spatial(error) => write!(formatter, "{error}"),
            Self::InvalidConfig { field } => {
                write!(formatter, "invalid sensor configuration {field}")
            }
            Self::OutputLength { expected, actual } => write!(
                formatter,
                "sensor output length must be {expected}; received {actual}"
            ),
            Self::UnknownSnakeIndex { snake_index } => {
                write!(formatter, "sensor snake index {snake_index} is absent")
            }
            Self::InvalidWorldValue { field, snake_id } => {
                write!(
                    formatter,
                    "snake {snake_id} has invalid sensor field {field}"
                )
            }
            Self::InvalidGenerationBest => write!(formatter, "generation best points are invalid"),
            Self::NonFiniteOutput { snake_id } => {
                write!(
                    formatter,
                    "snake {snake_id} produced a non-finite observation"
                )
            }
            Self::DeliverySnakeMismatch { expected, actual } => write!(
                formatter,
                "observation for snake {expected} cannot advance snake {actual}"
            ),
            Self::StaleDeliveryMarker { snake_id } => {
                write!(
                    formatter,
                    "snake {snake_id} observation delivery marker is stale"
                )
            }
            Self::ScratchAllocation { context, requested } => {
                write!(
                    formatter,
                    "could not reserve {requested} values for {context}"
                )
            }
        }
    }
}

impl Error for SensorError {}

/// Map one relative angle to the current centered angular bin.
#[must_use]
fn angle_to_centered_bin(relative_angle: f64, bins: usize) -> usize {
    let mut unit = (relative_angle + PI) / TAU;
    unit = (unit + 0.5 / bins as f64) % 1.0;
    clamp((unit * bins as f64).floor(), 0.0, (bins - 1) as f64) as usize
}

fn centered_bin_to_angle(index: usize, bins: usize) -> f64 {
    -PI + index as f64 / bins as f64 * TAU
}

fn ratio_to_bipolar(ratio: f64) -> f64 {
    clamp(ratio, 0.0, 1.0) * 2.0 - 1.0
}

fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.max(minimum).min(maximum)
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClosestPoint {
    point: WorldPoint,
    distance_squared: f64,
}

fn closest_point_on_segment(point: WorldPoint, start: WorldPoint, end: WorldPoint) -> ClosestPoint {
    let abx = end.x - start.x;
    let aby = end.y - start.y;
    let apx = point.x - start.x;
    let apy = point.y - start.y;
    let squared_length = abx * abx + aby * aby;
    let mut along = 0.0;
    if squared_length > 1.0e-9 {
        along = (apx * abx + apy * aby) / squared_length;
    }
    along = clamp(along, 0.0, 1.0);
    let closest = WorldPoint {
        x: start.x + abx * along,
        y: start.y + aby * along,
    };
    let dx = point.x - closest.x;
    let dy = point.y - closest.y;
    ClosestPoint {
        point: closest,
        distance_squared: dx * dx + dy * dy,
    }
}

fn distance_to_wall_along_ray(point: WorldPoint, direction: f64, world_radius: f64) -> f64 {
    let unit_x = direction.cos();
    let unit_y = direction.sin();
    let projection = point.x * unit_x + point.y * unit_y;
    let circle = point.x * point.x + point.y * point.y - world_radius * world_radius;
    let discriminant = projection * projection - circle;
    if discriminant <= 0.0 {
        0.0
    } else {
        -projection + discriminant.sqrt()
    }
}

fn resize_scratch(
    values: &mut Vec<f32>,
    length: usize,
    context: &'static str,
) -> Result<(), SensorError> {
    if values.len() < length {
        values
            .try_reserve_exact(length - values.len())
            .map_err(|_| SensorError::ScratchAllocation {
                context,
                requested: length,
            })?;
        values.resize(length, 0.0);
    } else if values.len() > length {
        values.truncate(length);
    }
    Ok(())
}

fn validate_snake_for_sensing(snake: &SnakeState) -> Result<(), SensorError> {
    for (field, value) in [
        ("position.x", snake.position.x),
        ("position.y", snake.position.y),
        ("direction", snake.direction),
        ("radius", snake.radius),
        ("speed", snake.speed),
        ("age_seconds", snake.age_seconds),
        ("points", snake.points),
        (
            "delivered_observation_points",
            snake.delivered_observation_points,
        ),
    ] {
        if !value.is_finite() {
            return Err(SensorError::InvalidWorldValue {
                field,
                snake_id: snake.id,
            });
        }
    }
    if !snake.alive
        || snake.radius <= 0.0
        || snake.speed < 0.0
        || snake.age_seconds < 0.0
        || snake.points < 0.0
        || snake.delivered_observation_points < 0.0
    {
        return Err(SensorError::InvalidWorldValue {
            field: "alive/radius/speed/points",
            snake_id: snake.id,
        });
    }
    Ok(())
}

fn require_config_range(
    field: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), SensorError> {
    if !value.is_finite() || value < minimum || value > maximum {
        return Err(SensorError::InvalidConfig { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::spatial::SensorIndexConfig;
    use crate::engine::state::{BodyRange, PelletState, SnakeKind};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReferenceDocument {
        fixture_version: u32,
        sensor_layout_version: String,
        cases: Vec<ReferenceCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReferenceCase {
        name: String,
        config: ReferenceConfig,
        best_points_this_generation: f64,
        target_id: u64,
        snakes: Vec<ReferenceSnake>,
        pellets: Vec<ReferencePellet>,
        expected: Vec<f32>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReferenceConfig {
        bins: usize,
        world_radius: f64,
        food_value: f64,
        snake_boost_speed: f64,
        snake_start_length: usize,
        snake_max_length: usize,
        minimum_boost_points: f64,
        boost_points_cost_per_second: f64,
        boost_points_cost_size_factor: f64,
        generation_seconds: f64,
        near_radius_base: f64,
        near_radius_scale: f64,
        near_radius_minimum: f64,
        near_radius_maximum: f64,
        far_radius_base: f64,
        far_radius_scale: f64,
        far_radius_minimum: f64,
        far_radius_maximum: f64,
        food_saturation: f64,
        collision_hit_scale: f64,
        maximum_pellet_checks: usize,
        maximum_segment_checks: usize,
    }

    impl From<ReferenceConfig> for SensorConfig {
        fn from(source: ReferenceConfig) -> Self {
            Self {
                bins: source.bins,
                world_radius: source.world_radius,
                food_value: source.food_value,
                snake_boost_speed: source.snake_boost_speed,
                snake_start_length: source.snake_start_length,
                snake_max_length: source.snake_max_length,
                minimum_boost_points: source.minimum_boost_points,
                boost_points_cost_per_second: source.boost_points_cost_per_second,
                boost_points_cost_size_factor: source.boost_points_cost_size_factor,
                generation_seconds: source.generation_seconds,
                near_radius_base: source.near_radius_base,
                near_radius_scale: source.near_radius_scale,
                near_radius_minimum: source.near_radius_minimum,
                near_radius_maximum: source.near_radius_maximum,
                far_radius_base: source.far_radius_base,
                far_radius_scale: source.far_radius_scale,
                far_radius_minimum: source.far_radius_minimum,
                far_radius_maximum: source.far_radius_maximum,
                food_saturation: source.food_saturation,
                collision_hit_scale: source.collision_hit_scale,
                maximum_pellet_checks: source.maximum_pellet_checks,
                maximum_segment_checks: source.maximum_segment_checks,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Deserialize)]
    struct ReferencePoint {
        x: f64,
        y: f64,
    }

    impl From<ReferencePoint> for WorldPoint {
        fn from(source: ReferencePoint) -> Self {
            Self {
                x: source.x,
                y: source.y,
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReferenceSnake {
        id: u64,
        kind: String,
        alive: bool,
        position: ReferencePoint,
        direction: f64,
        radius: f64,
        speed: f64,
        boost: bool,
        age_seconds: f64,
        points: f64,
        delivered_observation_points: f64,
        body: Vec<ReferencePoint>,
    }

    #[derive(Debug, Deserialize)]
    struct ReferencePellet {
        id: u64,
        position: ReferencePoint,
        value: f64,
        kind: u32,
        color: u32,
        owner: Option<u64>,
    }

    fn snake(id: u64, kind: SnakeKind, body: BodyRange, position: WorldPoint) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("test id should fit"),
            kind,
            alive: true,
            population_slot: None,
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: 0.0,
            radius: 10.0,
            speed: 165.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: body.len as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body,
            skin: 0,
        }
    }

    fn indexed_world(world: &WorldState) -> IndexedSensorWorld<'_> {
        IndexedSensorWorld::build(
            world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 100_000,
            },
        )
        .expect("sensor indexes should build")
    }

    fn reference_world(case: &ReferenceCase) -> WorldState {
        let mut world = WorldState::default();
        for source in &case.snakes {
            let body_start = world.body_points.len();
            world
                .body_points
                .extend(source.body.iter().copied().map(WorldPoint::from));
            let kind = match source.kind.as_str() {
                "evolved" => SnakeKind::Evolved,
                "baseline" => SnakeKind::Baseline,
                "external" => SnakeKind::External,
                "resurrected" => SnakeKind::Resurrected,
                other => panic!("unsupported reference snake kind {other}"),
            };
            let mut converted = snake(
                source.id,
                kind,
                BodyRange {
                    start: body_start,
                    len: source.body.len(),
                },
                source.position.into(),
            );
            converted.alive = source.alive;
            converted.direction = source.direction;
            converted.radius = source.radius;
            converted.speed = source.speed;
            converted.boost = source.boost;
            converted.age_seconds = source.age_seconds;
            converted.points = source.points;
            converted.delivered_observation_points = source.delivered_observation_points;
            converted.target_length = source.body.len() as f64;
            world.snakes.push(converted);
        }
        world
            .pellets
            .extend(case.pellets.iter().map(|source| PelletState {
                id: source.id,
                position: source.position.into(),
                value: source.value,
                kind: source.kind,
                color: source.color,
                owner: source.owner,
            }));
        world
    }

    fn sample(
        evaluator: &SensorEvaluator,
        world: &WorldState,
        snake_index: usize,
        generation: &SensorGenerationState,
    ) -> (Vec<f32>, SensorSample) {
        let indexed = indexed_world(world);
        let mut output = vec![0.0; evaluator.layout().input_size];
        let mut scratch = SensorScratch::default();
        let sample = evaluator
            .sample(&indexed, generation, snake_index, &mut output, &mut scratch)
            .expect("sensor sample should succeed");
        (output, sample)
    }

    #[test]
    fn default_layout_produces_all_scalars_and_clear_channels() {
        let evaluator =
            SensorEvaluator::new(SensorConfig::default()).expect("config should validate");
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: -7.5, y: 0.0 },
                WorldPoint { x: -15.0, y: 0.0 },
                WorldPoint { x: -22.5, y: 0.0 },
                WorldPoint { x: -30.0, y: 0.0 },
            ],
            ..WorldState::default()
        };
        world.snakes.push(snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 5 },
            WorldPoint { x: 0.0, y: 0.0 },
        ));
        let (output, result) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        assert_eq!(output.len(), 83);
        assert_eq!(output[0], 0.0);
        assert_eq!(output[1], 1.0);
        assert_eq!(output[2], -1.0);
        assert_eq!(output[5], -0.34);
        assert_eq!(output[6], -1.0);
        assert_eq!(output[8], 0.0);
        assert_eq!(output[9], -0.999);
        assert_eq!(output[12], 1.0);
        assert_eq!(output[13], -1.0);
        assert_eq!(output[16], -1.0);
        assert_eq!(output[17], -1.0);
        assert_eq!(output[18], -1.0);
        for index in 0..evaluator.layout().bins {
            assert_eq!(output[evaluator.layout().offsets.food + index], -1.0);
            assert_eq!(output[evaluator.layout().offsets.hazard + index], 1.0);
            assert_eq!(output[evaluator.layout().offsets.head + index], 1.0);
        }
        assert_eq!(result.delivery.sampled_points, 0.0);
    }

    #[test]
    fn long_body_segment_is_visible_near_an_endpoint_not_its_midpoint() {
        let config = SensorConfig {
            bins: 8,
            near_radius_base: 200.0,
            near_radius_scale: 0.0,
            near_radius_minimum: 200.0,
            near_radius_maximum: 200.0,
            ..SensorConfig::default()
        };
        let evaluator = SensorEvaluator::new(config).expect("config should validate");
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: -7.5, y: 0.0 },
                WorldPoint { x: 80.0, y: -20.0 },
                WorldPoint {
                    x: 1_000.0,
                    y: 20.0,
                },
            ],
            ..WorldState::default()
        };
        world.snakes.push(snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 2 },
            WorldPoint { x: 0.0, y: 0.0 },
        ));
        let mut other = snake(
            2,
            SnakeKind::Evolved,
            BodyRange { start: 2, len: 2 },
            WorldPoint { x: 80.0, y: -20.0 },
        );
        other.radius = 10.0;
        world.snakes.push(other);
        let (output, _) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        let forward = angle_to_centered_bin(0.0, evaluator.layout().bins);
        assert!(output[16] > -1.0);
        assert!(output[evaluator.layout().offsets.hazard + forward] < 1.0);
    }

    #[test]
    fn segment_cap_reports_and_saturates_conservatively() {
        let config = SensorConfig {
            bins: 8,
            maximum_segment_checks: 200,
            near_radius_base: 500.0,
            near_radius_scale: 0.0,
            near_radius_minimum: 500.0,
            near_radius_maximum: 500.0,
            ..SensorConfig::default()
        };
        let evaluator = SensorEvaluator::new(config).expect("config should validate");
        let mut world = WorldState::default();
        world.body_points.extend([
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -7.5, y: 0.0 },
        ]);
        world.snakes.push(snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 2 },
            WorldPoint { x: 0.0, y: 0.0 },
        ));
        for offset in 0..201usize {
            let angle = offset as f64 / 201.0 * TAU;
            let start = world.body_points.len();
            let position = WorldPoint {
                x: angle.cos() * 200.0,
                y: angle.sin() * 200.0,
            };
            world.body_points.push(position);
            world.body_points.push(WorldPoint {
                x: position.x + 1.0,
                y: position.y + 1.0,
            });
            world.snakes.push(snake(
                offset as u64 + 2,
                SnakeKind::Evolved,
                BodyRange { start, len: 2 },
                position,
            ));
        }
        let (output, result) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        assert_eq!(result.diagnostics.segment_cap_hits, 1);
        assert_eq!(result.diagnostics.segment_checks, 0);
        assert_eq!(result.diagnostics.body_query.candidates, 200);
        assert!(result.diagnostics.conservative_body_saturation);
        assert_eq!(output[16], 1.0);
        for index in 0..evaluator.layout().bins {
            assert_eq!(output[evaluator.layout().offsets.hazard + index], -1.0);
        }
    }

    #[test]
    fn pellet_cap_is_measured_after_nearest_first_ordering() {
        let config = SensorConfig {
            bins: 8,
            maximum_pellet_checks: 100,
            ..SensorConfig::default()
        };
        let evaluator = SensorEvaluator::new(config).expect("config should validate");
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: -7.5, y: 0.0 },
            ],
            ..WorldState::default()
        };
        world.snakes.push(snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 2 },
            WorldPoint { x: 0.0, y: 0.0 },
        ));
        for index in 0..121usize {
            world.pellets.push(PelletState {
                id: index as u64 + 10,
                position: WorldPoint {
                    x: index as f64 + 10.0,
                    y: 0.0,
                },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            });
        }
        let (output, result) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        assert_eq!(result.diagnostics.pellet_cap_hits, 1);
        assert_eq!(result.diagnostics.pellet_checks, 120);
        assert!(output[13] > -1.0);
    }

    #[test]
    fn pure_probes_accumulate_until_a_matching_delivery_commits() {
        let evaluator =
            SensorEvaluator::new(SensorConfig::default()).expect("config should validate");
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: -7.5, y: 0.0 },
            ],
            ..WorldState::default()
        };
        let mut observer = snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 2 },
            WorldPoint { x: 0.0, y: 0.0 },
        );
        observer.points = 4.0;
        world.snakes.push(observer);

        let (_, first) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        let (second_output, second) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        assert_eq!(second_output[8], 0.4);
        assert_eq!(world.snakes[0].delivered_observation_points, 0.0);
        second
            .delivery
            .commit(&mut world.snakes[0])
            .expect("accepted observation should commit");
        assert_eq!(world.snakes[0].delivered_observation_points, 4.0);
        assert!(matches!(
            first.delivery.commit(&mut world.snakes[0]),
            Err(SensorError::StaleDeliveryMarker { snake_id: 1 })
        ));
        let (third_output, _) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        assert_eq!(third_output[8], 0.0);
    }

    #[test]
    fn generation_best_is_initialized_and_excludes_non_population_snakes() {
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: 10.0, y: 0.0 },
            ],
            ..WorldState::default()
        };
        let mut evolved = snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 1 },
            WorldPoint { x: 0.0, y: 0.0 },
        );
        evolved.points = 5.0;
        let mut baseline = snake(
            2,
            SnakeKind::Baseline,
            BodyRange { start: 1, len: 1 },
            WorldPoint { x: 10.0, y: 0.0 },
        );
        baseline.points = 50.0;
        world.snakes.extend([evolved, baseline]);
        let mut generation = SensorGenerationState::new();
        assert_eq!(generation.best_points_this_generation(), 0.0);
        generation
            .update_after_step(&world)
            .expect("valid evolved scores should update");
        assert_eq!(generation.best_points_this_generation(), 5.0);
        generation.reset();
        assert_eq!(generation.best_points_this_generation(), 0.0);
    }

    #[test]
    fn all_supported_bin_counts_produce_finite_exact_length_observations() {
        for bins in 8..=32 {
            let config = SensorConfig {
                bins,
                ..SensorConfig::default()
            };
            let evaluator = SensorEvaluator::new(config).expect("config should validate");
            let mut world = WorldState {
                body_points: vec![
                    WorldPoint { x: 0.0, y: 0.0 },
                    WorldPoint { x: -7.5, y: 0.0 },
                ],
                ..WorldState::default()
            };
            world.snakes.push(snake(
                1,
                SnakeKind::Evolved,
                BodyRange { start: 0, len: 2 },
                WorldPoint { x: 0.0, y: 0.0 },
            ));
            let (output, _) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
            assert_eq!(output.len(), 19 + 4 * bins);
            assert!(output.iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn corrected_typescript_and_rust_vectors_match_shared_reference_cases() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("sensor-v3-reference.json");
        let fixture = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", fixture_path.display()));
        let document: ReferenceDocument =
            serde_json::from_str(&fixture).expect("shared sensor fixture should parse");
        assert_eq!(document.fixture_version, 1);
        assert_eq!(document.sensor_layout_version, "v3");
        assert_eq!(document.cases.len(), 3);
        for case in document.cases {
            let world = reference_world(&case);
            let evaluator = SensorEvaluator::new(case.config.into())
                .unwrap_or_else(|error| panic!("{} config failed: {error}", case.name));
            let target_index = world
                .snakes
                .iter()
                .position(|snake| snake.id == case.target_id)
                .unwrap_or_else(|| panic!("{} target is absent", case.name));
            let indexed = indexed_world(&world);
            let generation = SensorGenerationState {
                best_points_this_generation: case.best_points_this_generation,
            };
            let mut actual = vec![0.0; evaluator.layout().input_size];
            let mut scratch = SensorScratch::default();
            evaluator
                .sample(
                    &indexed,
                    &generation,
                    target_index,
                    &mut actual,
                    &mut scratch,
                )
                .unwrap_or_else(|error| panic!("{} sample failed: {error}", case.name));
            assert_eq!(actual.len(), case.expected.len(), "{} length", case.name);
            for (index, (actual, expected)) in actual.iter().zip(&case.expected).enumerate() {
                let difference = f64::from((*actual - *expected).abs());
                assert!(
                    difference <= 2.0e-5,
                    "{} value {index}: actual {actual}, expected {expected}, difference {difference}",
                    case.name
                );
            }
        }
    }
}
