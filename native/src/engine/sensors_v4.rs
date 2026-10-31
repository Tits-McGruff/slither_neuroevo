//! Complete native v4 qualification observations from a stable source boundary.
//! Profiles are still candidates. Runtime routing/defaults and measured limits
//! must qualify before this evaluator becomes the production observation path.

use super::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState, EvolvedLifeStatus};
use super::movement::{effective_boost_cost, MovementConfig};
use super::ray_work::{RayWorkDiagnostics, RayWorkMeter};
use super::sensor_contract::{SensorContractIdentity, SensorProfile};
use super::sensor_geometry::{HeadSeparation, RayIntersection, SensorRay, MOTION_HORIZON_SECONDS};
use super::sensor_layout::MAX_SENSOR_BINS;
use super::sensor_rays::{BodyRayIndex, RayIndexError, RayQueryScratch};
use super::sensors::{
    angle_to_centered_bin, closest_point_on_segment, validate_snake_for_sensing,
    CenteredAngularBins, ObservationDeliveryMarker, SensorConfig, SensorEvaluator, SensorRadii,
};
use super::spatial::{
    BodySensorQueryScratch, IndexedSensorWorld, PelletQueryScratch, SpatialQueryDiagnostics,
};
use super::state::{SnakeKind, SnakeState, WorldPoint, WorldState};
use std::f64::consts::{PI, TAU};
use std::fmt::{Display, Formatter};

/// Immutable actual economic/life context. No population-best neural input.
#[derive(Clone, Copy)]
pub struct V4ObservationContext<'source> {
    pub generation_elapsed_seconds: f64,
    pub evolved: &'source EvolvedGenerationState,
    pub experiment: EvolvedExperimentConfig,
}

/// Work and incomplete-query facts from this exact observation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V4SampleDiagnostics {
    pub pellets: SpatialQueryDiagnostics,
    pub body: SpatialQueryDiagnostics,
    pub ray: RayWorkDiagnostics,
    pub head_checks: usize,
    pub incomplete_rays: usize,
}

/// Pure sample receipt plus bounded representative identities for inspection.
/// These IDs convey no undeclared target measurements to the v2 teacher.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V4SensorSample {
    pub delivery: ObservationDeliveryMarker,
    pub radii: SensorRadii,
    pub diagnostics: V4SampleDiagnostics,
    pub representative_head_ids: [u64; MAX_SENSOR_BINS],
}

/// Retained worker-local query storage; fixed bin arrays avoid warm allocation.
#[derive(Clone, Debug, Default)]
pub struct V4SensorScratch {
    pub(crate) body_query_ns: u64,
    pub(crate) body_ray_query_ns: u64,
    pellets: PelletQueryScratch,
    body: BodySensorQueryScratch,
    rays: RayQueryScratch,
    near_food: [f64; MAX_SENSOR_BINS],
    far_food: [f64; MAX_SENSOR_BINS],
    body_clearance: [f64; MAX_SENSOR_BINS],
    head_clearance: [f64; MAX_SENSOR_BINS],
    nearest_head: [usize; MAX_SENSOR_BINS],
    head_count: [usize; MAX_SENSOR_BINS],
    urgency: [f64; MAX_SENSOR_BINS],
}

/// Worker-local retained query capacities, with fixed arrays included explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V4ScratchCapacity {
    pub fixed_bytes: usize,
    pub body_duplicate_marker_capacity: usize,
    pub body_candidate_capacity: usize,
    pub pellet_candidate_capacity: usize,
    pub ray_packet_capacity: usize,
    pub ray_packet_bytes: usize,
}

impl V4SensorScratch {
    /// Inspect storage without allocating, querying, or consuming a delivery.
    pub fn diagnostics(&self) -> V4ScratchCapacity {
        V4ScratchCapacity {
            fixed_bytes: std::mem::size_of::<Self>(),
            body_duplicate_marker_capacity: self.body.duplicate_marker_capacity(),
            body_candidate_capacity: self.body.candidate_capacity(),
            pellet_candidate_capacity: self.pellets.candidate_capacity(),
            ray_packet_capacity: self.rays.packet_capacity(),
            ray_packet_bytes: self.rays.packet_owned_bytes(),
        }
    }
}

/// Cold-admitted shared head facts; physical array indices are never population slots.
#[derive(Debug, Default)]
pub struct V4HeadWorkspace {
    velocities: Vec<WorldPoint>,
}

/// Read-only head facts bound to the same immutable pre-movement world.
#[derive(Clone, Copy)]
pub struct PreparedV4Heads<'source> {
    source: &'source WorldState,
    velocities: &'source [WorldPoint],
    profile: SensorProfile,
}

impl V4HeadWorkspace {
    /// Allocate only within the caller's checked shared staging allowance.
    pub fn try_new(maximum: usize) -> Result<Self, V4SensorError> {
        let mut velocities = Vec::new();
        velocities
            .try_reserve_exact(maximum)
            .map_err(|_| V4SensorError::Allocation)?;
        Ok(Self { velocities })
    }

    /// Actual owned slots, included in neural staging admission/capacity diagnostics.
    pub fn capacity(&self) -> usize {
        self.velocities.capacity()
    }

    /// Validate alive heads once and retain exact separate sin/cos products for Motion.
    /// A standalone caller whose world exceeds its neural staging width keeps the
    /// original uncached path; this optimization never imposes a new world-size cap.
    pub fn prepare<'source>(
        &'source mut self,
        world: &'source WorldState,
        profile: SensorProfile,
    ) -> Result<Option<PreparedV4Heads<'source>>, V4SensorError> {
        if profile == SensorProfile::V3 {
            return Err(V4SensorError::Config);
        }
        self.velocities.clear();
        if world.snakes.len() > self.velocities.capacity() {
            return Ok(None);
        }
        for snake in &world.snakes {
            let mut velocity = WorldPoint { x: 0.0, y: 0.0 };
            if snake.alive {
                validate_snake_for_sensing(snake).map_err(|_| V4SensorError::Shape)?;
                if profile == SensorProfile::Motion {
                    // Preserve the reference formula's operations and rounding.
                    velocity.x = snake.direction.cos() * snake.speed;
                    velocity.y = snake.direction.sin() * snake.speed;
                }
            }
            self.velocities.push(velocity);
        }
        Ok(Some(PreparedV4Heads {
            source: world,
            velocities: &self.velocities,
            profile,
        }))
    }
}

impl PreparedV4Heads<'_> {
    fn is_from(self, world: &WorldState, profile: SensorProfile) -> bool {
        std::ptr::eq(self.source, world)
            && self.velocities.len() == world.snakes.len()
            && self.profile == profile
    }
}

/// One coordinator-bound v4 observation context, including its fixed grants.
/// Ray hierarchy and life ledger borrow the same stable pre-movement boundary.
#[derive(Clone, Copy)]
pub struct PreparedV4Sensing<'source> {
    pub body_rays: BodyRayIndex<'source, 'source>,
    pub context: V4ObservationContext<'source>,
    /// Optional shared validation/velocity facts; None retains the pure reference path.
    pub heads: Option<PreparedV4Heads<'source>>,
    pub grants_by_snake: &'source [u32],
}

/// Validated candidate evaluator. Physical movement curves remain shared with
/// movement; only tactical radius scaling uses the candidate size transform.
#[derive(Clone, Debug)]
pub struct SensorV4Evaluator {
    layout: SensorContractIdentity,
    common: SensorEvaluator,
    movement: MovementConfig,
    fixed_dt: f64,
}

impl SensorV4Evaluator {
    pub fn new(
        profile: SensorProfile,
        common: SensorConfig,
        movement: MovementConfig,
        fixed_dt: f64,
    ) -> Result<Self, V4SensorError> {
        if profile == SensorProfile::V3
            || !fixed_dt.is_finite()
            || fixed_dt <= 0.0
            || fixed_dt > 1.0
        {
            return Err(V4SensorError::Config);
        }
        common.validate().map_err(|_| V4SensorError::Config)?;
        movement.validate().map_err(|_| V4SensorError::Config)?;
        if common.world_radius != movement.world_radius
            || common.snake_boost_speed != movement.snake_boost_speed
            || common.snake_start_length != movement.snake_start_len
            || common.snake_max_length != movement.snake_max_len
            || common.minimum_boost_points != movement.boost_min_points
            || common.boost_points_cost_per_second != movement.boost_points_cost_per_second
            || common.boost_points_cost_size_factor != movement.boost_points_cost_size_factor
        {
            return Err(V4SensorError::Config);
        }
        let layout =
            SensorContractIdentity::new(profile, common.bins).map_err(|_| V4SensorError::Config)?;
        let common = SensorEvaluator::new(common).map_err(|_| V4SensorError::Config)?;
        Ok(Self {
            layout,
            common,
            movement,
            fixed_dt,
        })
    }

    pub fn layout(&self) -> &SensorContractIdentity {
        &self.layout
    }

    /// Frozen v3 evaluator retained exclusively for version-one baselines.
    pub(crate) fn legacy_evaluator(&self) -> &SensorEvaluator {
        &self.common
    }

    /// Construct the complete vector without consuming delivery or touching
    /// world/RNG/recurrent/lifecycle continuation. Caller owns the work grant.
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &self,
        indexed: &IndexedSensorWorld<'_>,
        body_rays: BodyRayIndex<'_, '_>,
        context: V4ObservationContext<'_>,
        snake_index: usize,
        output: &mut [f32],
        ray_work: &mut RayWorkMeter,
        scratch: &mut V4SensorScratch,
    ) -> Result<V4SensorSample, V4SensorError> {
        self.sample_with_trace(
            indexed,
            body_rays,
            context,
            snake_index,
            output,
            ray_work,
            scratch,
            None,
            None,
        )
    }

    /// Copy optional raw diagnostics during the same pure observation pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sample_with_trace(
        &self,
        indexed: &IndexedSensorWorld<'_>,
        body_rays: BodyRayIndex<'_, '_>,
        context: V4ObservationContext<'_>,
        snake_index: usize,
        output: &mut [f32],
        ray_work: &mut RayWorkMeter,
        scratch: &mut V4SensorScratch,
        mut trace: Option<&mut super::sensor_trace::SensorTrace>,
        heads: Option<PreparedV4Heads<'_>>,
    ) -> Result<V4SensorSample, V4SensorError> {
        if let Some(trace) = trace.as_deref_mut() {
            trace.valid = false;
        }
        if !body_rays.is_from(indexed.body_index())
            || output.len() != self.layout.input_size()
            || !context.generation_elapsed_seconds.is_finite()
            || context.generation_elapsed_seconds < 0.0
        {
            return Err(V4SensorError::Shape);
        }
        context
            .experiment
            .validate()
            .map_err(|_| V4SensorError::Life)?;
        let world = indexed.world();
        if heads.is_some_and(|heads| !heads.is_from(world, self.layout.profile())) {
            return Err(V4SensorError::Shape);
        }
        let snake = world.snakes.get(snake_index).ok_or(V4SensorError::Shape)?;
        validate_snake_for_sensing(snake).map_err(|_| V4SensorError::Shape)?;
        if snake.id == 0
            || snake.body.len == 0
            || !snake.completed_step_heading_delta.is_finite()
            || !snake.completed_step_speed_delta.is_finite()
            || !snake.completed_step_applied_turn.is_finite()
            || !snake.turn.is_finite()
        {
            return Err(V4SensorError::Shape);
        }
        let (life_points, assessment, life_budget) = life_scalars(snake, context)?;
        let cfg = self.common.config();
        let bins = self.layout.bins();
        let difference = snake.body.len.saturating_sub(cfg.snake_start_length) as f64;
        let size = difference / (difference + 40.0);
        let radii = self.common.radii(size);
        let cost = effective_boost_cost(snake.body.len, self.movement);
        let headroom = (snake.points - cfg.minimum_boost_points).max(0.0);
        let eligible = snake.body.len > self.movement.snake_min_len + 1
            && snake.points >= cfg.minimum_boost_points
            && snake.points > 0.0
            && cost > 0.0;
        let delta = snake.points - snake.delivered_observation_points;
        if let Some(trace) = trace.as_deref_mut() {
            trace.begin(snake, self.layout.profile(), bins, radii);
            trace.size_factor = size;
            trace.boost_cost = cost;
            trace.boost_headroom = headroom;
            trace.boost_eligible = Some(eligible);
            trace.life_points = Some(life_points);
            trace.assessment_points = Some(assessment);
            trace.generation_elapsed_seconds = Some(context.generation_elapsed_seconds);
            trace.generation_seconds = cfg.generation_seconds;
            trace.food_saturation = cfg.food_saturation;
            trace.boost_speed = cfg.snake_boost_speed;
        }
        output.fill(0.0);
        output[..24].copy_from_slice(&[
            snake.direction.sin() as f32,
            snake.direction.cos() as f32,
            bipolar(size),
            bipolar(snake.radius / self.movement.snake_radius_max),
            bipolar(snake.speed / cfg.snake_boost_speed),
            saturate(snake.points, 140.0),
            if cost == 0.0 {
                1.0
            } else {
                bipolar(headroom / (headroom + 2.0 * cost))
            },
            saturate(cost, 7.0),
            flag(eligible),
            flag(snake.boost),
            flag(snake.input_boost),
            signed(f64::from(snake.turn)),
            signed(f64::from(snake.completed_step_applied_turn)),
            signed(
                snake.completed_step_heading_delta / self.fixed_dt / self.movement.snake_turn_rate,
            ),
            signed(snake.completed_step_speed_delta / cfg.snake_boost_speed),
            signed(delta / (delta.abs() + 10.0)),
            saturate(life_points, 200.0),
            saturate(assessment, 1000.0),
            saturate(snake.age_seconds, 60.0),
            (1.0 - context.generation_elapsed_seconds / cfg.generation_seconds).clamp(0.0, 1.0)
                as f32,
            life_budget,
            flag(!snake.observation_delivered),
            -1.0,
            -1.0,
        ]);
        scratch.near_food.fill(0.0);
        scratch.far_food.fill(0.0);
        scratch.body_clearance.fill(radii.near);
        scratch.head_clearance.fill(radii.near);
        scratch.nearest_head.fill(usize::MAX);
        scratch.head_count.fill(0);
        scratch.urgency.fill(0.0);
        let pellets = indexed
            .pellet_index()
            .collect_sensor_candidates(
                snake.position,
                radii.far,
                self.common.effective_pellet_limit(),
                &mut scratch.pellets,
            )
            .map_err(|_| V4SensorError::Spatial)?;
        // Share one stack-only set across dense pellet, body and head queries.
        // Sparse queries keep their original formula instead of preparing boundaries.
        let mut sector_bins = (snake.direction.abs() <= TAU
            && (pellets.candidates >= bins || world.snakes.len() >= bins))
            .then(|| CenteredAngularBins::new(snake.direction, bins));
        for pellet in indexed.pellet_index().candidates(&scratch.pellets) {
            let dx = pellet.position.x - snake.position.x;
            let dy = pellet.position.y - snake.position.y;
            let distance = dx.hypot(dy);
            let bin = sector_bin(
                sector_bins.as_ref(),
                dx,
                dy,
                distance,
                snake.direction,
                bins,
            );
            let strength = (1.0 - distance / radii.far).clamp(0.0, 1.0)
                * (pellet.value / cfg.food_value).clamp(0.0, 6.0);
            if distance <= radii.near {
                scratch.near_food[bin] += strength;
            } else {
                scratch.far_food[bin] += strength;
            }
        }
        output[22] = flag(pellets.candidate_limit_reached);
        let body_started = std::time::Instant::now();
        let broad = radii.near
            + (snake.radius + indexed.body_index().maximum_owner_radius())
                * cfg.collision_hit_scale;
        let body = indexed
            .body_index()
            .collect_sensor_candidates(
                snake.position,
                broad,
                snake.id,
                self.common.effective_segment_limit(),
                &mut scratch.body,
            )
            .map_err(|_| V4SensorError::Spatial)?;
        if body.candidate_limit_reached {
            scratch.body_clearance.fill(0.0);
        } else {
            if sector_bins.is_none() && body.candidates >= bins && snake.direction.abs() <= TAU {
                sector_bins = Some(CenteredAngularBins::new(snake.direction, bins));
            }
            for segment in indexed.body_index().sensor_candidates(&scratch.body) {
                let closest = closest_point_on_segment(snake.position, segment.start, segment.end);
                let distance = closest.distance_squared.sqrt();
                let clearance = (distance
                    - (snake.radius + segment.owner_radius) * cfg.collision_hit_scale)
                    .max(0.0);
                if clearance <= radii.near {
                    let bin = sector_bin(
                        sector_bins.as_ref(),
                        closest.point.x - snake.position.x,
                        closest.point.y - snake.position.y,
                        distance,
                        snake.direction,
                        bins,
                    );
                    scratch.body_clearance[bin] = scratch.body_clearance[bin].min(clearance);
                }
            }
        }
        scratch.body_query_ns = super::generation_work::nanos(body_started);
        let ray_started = std::time::Instant::now();
        let first_ray = SensorRay::new(snake.position, snake.direction - PI, radii.near)
            .map_err(|_| V4SensorError::Geometry)?;
        let mut packet = [first_ray; MAX_SENSOR_BINS];
        for (bin, ray) in packet.iter_mut().enumerate().take(bins) {
            *ray = SensorRay::new(
                snake.position,
                snake.direction - PI + bin as f64 * TAU / bins as f64,
                radii.near,
            )
            .map_err(|_| V4SensorError::Geometry)?;
        }
        let results = body_rays.trace_packet(
            &packet[..bins],
            snake.id,
            snake.radius,
            cfg.collision_hit_scale,
            ray_work,
            &mut scratch.rays,
        )?;
        scratch.body_ray_query_ns = super::generation_work::nanos(ray_started);
        let mut incomplete_rays = 0;
        for bin in 0..bins {
            let angle = snake.direction - PI + bin as f64 * TAU / bins as f64;
            let ray = SensorRay::new(snake.position, angle, radii.near)
                .map_err(|_| V4SensorError::Geometry)?;
            output[24 + bin] = saturate(scratch.near_food[bin], cfg.food_saturation);
            output[24 + bins + bin] = saturate(scratch.far_food[bin], cfg.food_saturation);
            output[24 + 2 * bins + bin] = bipolar(scratch.body_clearance[bin] / radii.near);
            output[24 + 3 * bins + bin] = match results[bin] {
                RayIntersection::Hit(distance) => bipolar(distance / radii.near),
                RayIntersection::Clear => 1.0,
                RayIntersection::Incomplete => {
                    incomplete_rays += 1;
                    -1.0
                }
            };
            let wall_clearance = ray
                .wall_clearance(cfg.world_radius, snake.radius)
                .map_err(|_| V4SensorError::Geometry)?;
            output[24 + 4 * bins + bin] = bipolar(wall_clearance / radii.near);
            if let Some(trace) = trace.as_deref_mut() {
                trace.ray_headings[bin] = angle;
                trace.wall_clearance[bin] = wall_clearance;
                trace.body_rays[bin] = match results[bin] {
                    RayIntersection::Hit(distance) => {
                        super::sensor_trace::SensorTraceRay::Hit(distance)
                    }
                    RayIntersection::Clear => super::sensor_trace::SensorTraceRay::Clear,
                    RayIntersection::Incomplete => super::sensor_trace::SensorTraceRay::Incomplete,
                };
            }
        }
        output[23] = flag(body.candidate_limit_reached || incomplete_rays != 0);
        let motion = self.layout.profile() == SensorProfile::Motion;
        let (forward_x, forward_y) = (snake.direction.cos(), snake.direction.sin());
        let own_vx = forward_x * snake.speed;
        let own_vy = forward_y * snake.speed;
        let mut head_checks = 0;
        for (index, other) in world.snakes.iter().enumerate() {
            if !other.alive || other.id == snake.id {
                continue;
            }
            if heads.is_none() {
                validate_snake_for_sensing(other).map_err(|_| V4SensorError::Shape)?;
            }
            head_checks += 1;
            let dx = other.position.x - snake.position.x;
            let dy = other.position.y - snake.position.y;
            let radius = (snake.radius + other.radius) * cfg.collision_hit_scale;
            let separation = HeadSeparation::new(WorldPoint { x: dx, y: dy });
            let distance = separation.distance();
            let clearance = (distance - radius).max(0.0);
            if clearance <= radii.near {
                let bin = sector_bin(
                    sector_bins.as_ref(),
                    dx,
                    dy,
                    distance,
                    snake.direction,
                    bins,
                );
                scratch.head_count[bin] += 1;
                let previous = scratch.nearest_head[bin];
                if previous == usize::MAX
                    || clearance < scratch.head_clearance[bin]
                    || (clearance == scratch.head_clearance[bin]
                        && other.id < world.snakes[previous].id)
                {
                    scratch.head_clearance[bin] = clearance;
                    scratch.nearest_head[bin] = index;
                }
            }
            if motion {
                let velocity = heads.map_or_else(
                    || WorldPoint {
                        x: other.direction.cos() * other.speed,
                        y: other.direction.sin() * other.speed,
                    },
                    |heads| heads.velocities[index],
                );
                let encounter = separation
                    .projected_encounter(
                        super::state::WorldPoint {
                            x: velocity.x - own_vx,
                            y: velocity.y - own_vy,
                        },
                        radius,
                    )
                    .map_err(|_| V4SensorError::Geometry)?;
                if let Some(encounter) = encounter {
                    let contact = encounter.relative_at_contact;
                    let distance = if sector_bins.is_some() {
                        contact.x.hypot(contact.y)
                    } else {
                        0.0
                    };
                    let bin = sector_bin(
                        sector_bins.as_ref(),
                        contact.x,
                        contact.y,
                        distance,
                        snake.direction,
                        bins,
                    );
                    scratch.urgency[bin] =
                        scratch.urgency[bin].max(1.0 - encounter.seconds / MOTION_HORIZON_SECONDS);
                    if let Some(trace) = trace.as_deref_mut() {
                        trace.encounter_seconds[bin] = Some(
                            trace.encounter_seconds[bin].map_or(encounter.seconds, |previous| {
                                previous.min(encounter.seconds)
                            }),
                        );
                    }
                }
            }
        }
        let mut representatives = [0; MAX_SENSOR_BINS];
        for bin in 0..bins {
            output[24 + 5 * bins + bin] = bipolar(scratch.head_clearance[bin] / radii.near);
            if let Some(trace) = trace.as_deref_mut() {
                trace.food_strength[bin] = scratch.near_food[bin];
                trace.far_food_strength[bin] = scratch.far_food[bin];
                trace.body_clearance[bin] = scratch.body_clearance[bin];
                trace.head_clearance[bin] = scratch.head_clearance[bin];
                trace.head_count[bin] = scratch.head_count[bin];
                trace.urgency[bin] = scratch.urgency[bin];
                if scratch.nearest_head[bin] != usize::MAX {
                    trace.head_ids[bin] = world.snakes[scratch.nearest_head[bin]].id;
                }
            }
            if !motion {
                continue;
            }
            let index = scratch.nearest_head[bin];
            output[24 + 6 * bins + bin] = flag(index != usize::MAX);
            output[24 + 13 * bins + bin] = scratch.urgency[bin] as f32;
            output[24 + 14 * bins + bin] =
                (scratch.head_count[bin] as f64 / (scratch.head_count[bin] as f64 + 4.0)) as f32;
            if index == usize::MAX {
                continue;
            }
            let other = &world.snakes[index];
            representatives[bin] = other.id;
            let bearing = wrap(
                (other.position.y - snake.position.y).atan2(other.position.x - snake.position.x)
                    - snake.direction,
            );
            output[24 + 7 * bins + bin] =
                signed(wrap(bearing - (-PI + bin as f64 * TAU / bins as f64)) / (PI / bins as f64));
            output[24 + 8 * bins + bin] =
                signed((other.body.len as f64 / snake.body.len as f64).log2() / 4.0);
            let heading = other.direction - snake.direction;
            output[24 + 9 * bins + bin] = heading.sin() as f32;
            output[24 + 10 * bins + bin] = heading.cos() as f32;
            let velocity = heads.map_or_else(
                || WorldPoint {
                    x: other.direction.cos() * other.speed,
                    y: other.direction.sin() * other.speed,
                },
                |heads| heads.velocities[index],
            );
            let vx = velocity.x - own_vx;
            let vy = velocity.y - own_vy;
            output[24 + 11 * bins + bin] =
                signed((vx * forward_x + vy * forward_y) / (2.0 * cfg.snake_boost_speed));
            output[24 + 12 * bins + bin] =
                signed((-vx * forward_y + vy * forward_x) / (2.0 * cfg.snake_boost_speed));
            if let Some(trace) = trace.as_deref_mut() {
                trace.head_bearing_offset[bin] =
                    wrap(bearing - (-PI + bin as f64 * TAU / bins as f64));
                trace.head_length_ratio[bin] = other.body.len as f64 / snake.body.len as f64;
                trace.relative_heading[bin] = heading;
                trace.relative_forward_speed[bin] = vx * forward_x + vy * forward_y;
                trace.relative_side_speed[bin] = -vx * forward_y + vy * forward_x;
            }
        }
        if output
            .iter()
            .any(|value| !value.is_finite() || !(-1.0..=1.0).contains(value))
        {
            return Err(V4SensorError::Geometry);
        }
        if let Some(trace) = trace {
            trace.pellet_saturated = pellets.candidate_limit_reached;
            trace.body_saturated = body.candidate_limit_reached;
            trace.valid = true;
        }
        Ok(V4SensorSample {
            delivery: ObservationDeliveryMarker {
                snake_id: snake.id,
                previous_delivered_points: snake.delivered_observation_points,
                previous_observation_delivered: snake.observation_delivered,
                sampled_points: snake.points,
            },
            radii,
            diagnostics: V4SampleDiagnostics {
                pellets,
                body,
                ray: ray_work.diagnostics(),
                head_checks,
                incomplete_rays,
            },
            representative_head_ids: representatives,
        })
    }
}

fn life_scalars(
    snake: &SnakeState,
    context: V4ObservationContext<'_>,
) -> Result<(f64, f64, f32), V4SensorError> {
    if snake.kind != SnakeKind::Evolved {
        return Ok((snake.points, snake.points, 0.0));
    }
    let policy = context.experiment;
    if context.evolved.slots.is_empty() && policy.version == 0 {
        return Ok((snake.points, snake.points, -1.0));
    }
    let slot = snake.population_slot.ok_or(V4SensorError::Life)? as usize;
    let owner = context.evolved.slots.get(slot).ok_or(V4SensorError::Life)?;
    if owner.population_slot as usize != slot
        || owner.physical_id != snake.id
        || owner.status != EvolvedLifeStatus::Alive
        || owner.lives_started == 0
    {
        return Err(V4SensorError::Life);
    }
    owner.ledger.validate().map_err(|_| V4SensorError::Life)?;
    owner
        .check_wallet(snake.points)
        .map_err(|_| V4SensorError::Life)?;
    let budget = if !policy.enabled {
        -1.0
    } else if policy.max_lives == 0 {
        1.0
    } else {
        if owner.lives_started > policy.max_lives {
            return Err(V4SensorError::Life);
        }
        -(owner.lives_started as f64 / policy.max_lives as f64) as f32
    };
    let assessment = if policy.version == 0 {
        snake.points
    } else {
        owner.assessment(policy.points, policy.nonpoints).points
    };
    let life_points = owner.ledger.life_net_points();
    if !assessment.is_finite() || assessment < 0.0 {
        return Err(V4SensorError::Life);
    }
    Ok((life_points, assessment, budget))
}
/// Cheap shared geometry only decides interior bins. Keep v4's exact sine/atan2
/// normalization for coincident/boundary/extreme vectors and large restored headings.
#[inline]
fn sector_bin(
    classifier: Option<&CenteredAngularBins>,
    x: f64,
    y: f64,
    distance: f64,
    direction: f64,
    bins: usize,
) -> usize {
    // Squared-distance norms can underflow for a nonzero vector. Subnormal norms
    // also lack enough guard precision; both retain the original angle formula.
    let interior = if distance.is_finite() && distance >= f64::MIN_POSITIVE {
        classifier.and_then(|classifier| classifier.classify_interior(x, y, distance))
    } else {
        None
    };
    interior.unwrap_or_else(|| angle_to_centered_bin(wrap(y.atan2(x) - direction), bins))
}

fn wrap(angle: f64) -> f64 {
    angle.sin().atan2(angle.cos())
}
fn signed(value: f64) -> f32 {
    value.clamp(-1.0, 1.0) as f32
}
fn bipolar(value: f64) -> f32 {
    (2.0 * value.clamp(0.0, 1.0) - 1.0) as f32
}
fn saturate(value: f64, scale: f64) -> f32 {
    bipolar(value / (value + scale))
}
fn flag(value: bool) -> f32 {
    if value {
        1.0
    } else {
        -1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V4SensorError {
    Allocation,
    Config,
    Shape,
    Life,
    Spatial,
    Geometry,
    Rays(RayIndexError),
}
impl From<RayIndexError> for V4SensorError {
    fn from(value: RayIndexError) -> Self {
        Self::Rays(value)
    }
}
impl Display for V4SensorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid sensor-v4 sample: {self:?}")
    }
}
impl std::error::Error for V4SensorError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::sensor_geometry::projected_head_encounter;
    use crate::engine::sensor_rays::BodyRayIndexWorkspace;
    use crate::engine::spatial::SensorIndexConfig;
    use crate::engine::state::{BodyRange, PelletState, WorldPoint, WorldState};

    fn point(x: f64, y: f64) -> WorldPoint {
        WorldPoint { x, y }
    }
    fn add_snake(
        world: &mut WorldState,
        id: u64,
        kind: SnakeKind,
        position: WorldPoint,
        heading: f64,
        length: usize,
    ) {
        let start = world.body_points.len();
        world.body_points.extend((0..length).map(|i| {
            point(
                position.x - heading.cos() * i as f64 * 7.5,
                position.y - heading.sin() * i as f64 * 7.5,
            )
        }));
        world.snakes.push(SnakeState {
            id,
            frame_v1_id: id as u32,
            kind,
            alive: true,
            population_slot: (kind == SnakeKind::Evolved).then_some(0),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: heading,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: length as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.06,
            delivered_observation_points: 0.0,
            observation_delivered: false,
            completed_step_heading_delta: 0.0,
            completed_step_speed_delta: 0.0,
            completed_step_applied_turn: 0.0,
            body: BodyRange { start, len: length },
            skin: 0,
        });
    }
    fn evaluator(profile: SensorProfile, bins: usize) -> SensorV4Evaluator {
        SensorV4Evaluator::new(
            profile,
            SensorConfig {
                bins,
                ..SensorConfig::default()
            },
            MovementConfig::typescript_defaults(),
            1.0 / 60.0,
        )
        .unwrap()
    }
    fn sample(
        evaluator: &SensorV4Evaluator,
        world: &WorldState,
        lives: &EvolvedGenerationState,
        policy: EvolvedExperimentConfig,
        grant: u32,
    ) -> (Vec<f32>, V4SensorSample) {
        let indexed = IndexedSensorWorld::build(
            world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 100_000,
            },
        )
        .unwrap();
        let mut rays = BodyRayIndexWorkspace::default();
        let view = rays.prepare(indexed.body_index()).unwrap();
        let mut output = vec![0.0; evaluator.layout().input_size()];
        let result = evaluator
            .sample(
                &indexed,
                view,
                V4ObservationContext {
                    generation_elapsed_seconds: 120.0,
                    evolved: lives,
                    experiment: policy,
                },
                0,
                &mut output,
                &mut RayWorkMeter::new(grant),
                &mut V4SensorScratch::default(),
            )
            .unwrap();
        (output, result)
    }
    #[test]
    fn sector_bin_matches_original_v4_formula_for_dense_boundary_and_extreme_vectors() {
        let mut interior = 0;
        let mut ordinary = 0;
        for bins in 8..=MAX_SENSOR_BINS {
            for direction in [
                -1.0e300, -1.0e20, -TAU, -PI, -2.1, -0.37, -0.0, 0.0, 0.37, 2.1, PI, TAU, 1.0e20,
                1.0e300,
            ] {
                let classifier = CenteredAngularBins::new(direction, bins);
                for scale in [
                    1.0e-320,
                    f64::MIN_POSITIVE / 2.0,
                    f64::MIN_POSITIVE,
                    1.0e-300,
                    1.0e-12,
                    1.0,
                    1.0e12,
                    1.0e300,
                ] {
                    for sample in 0..2000_u64 {
                        let angle = (sample.wrapping_mul(65_537) % 2000) as f64 / 2000.0 * TAU;
                        let (y, x) = angle.sin_cos();
                        let (x, y) = (x * scale, y * scale);
                        let distance = x.hypot(y);
                        let expected = angle_to_centered_bin(wrap(y.atan2(x) - direction), bins);
                        assert_eq!(
                            sector_bin(Some(&classifier), x, y, distance, direction, bins),
                            expected,
                            "B={bins} heading={direction} scale={scale} angle={angle}"
                        );
                        // Body proximity supplies sqrt(dx*dx+dy*dy), which can underflow.
                        let squared_distance = (x * x + y * y).sqrt();
                        assert_eq!(sector_bin(Some(&classifier),x,y,squared_distance,direction,bins),
                            expected, "squared norm: B={bins} heading={direction} scale={scale} angle={angle}");
                        if scale == 1.0 && direction.abs() <= TAU {
                            ordinary += 1;
                            interior +=
                                usize::from(classifier.classify_interior(x, y, distance).is_some());
                        }
                        if sample % 101 == 0 {
                            assert_eq!(sector_bin(None, x, y, distance, direction, bins), expected);
                        }
                    }
                    for boundary in 0..bins {
                        let angle = direction - PI + (boundary as f64 + 0.5) * TAU / bins as f64;
                        for delta in [
                            -1.0e-9,
                            -1.0e-10,
                            -1.0e-12,
                            -f64::EPSILON,
                            0.0,
                            f64::EPSILON,
                            1.0e-12,
                            1.0e-10,
                            1.0e-9,
                        ] {
                            let (y, x) = (angle + delta).sin_cos();
                            let (x, y) = (x * scale, y * scale);
                            let expected =
                                angle_to_centered_bin(wrap(y.atan2(x) - direction), bins);
                            assert_eq!(sector_bin(Some(&classifier), x,y,x.hypot(y),direction,bins),
                                       expected, "B={bins} heading={direction} scale={scale} boundary={boundary} delta={delta}");
                        }
                    }
                }
                for (x, y) in [
                    (0.0, 0.0),
                    (-0.0, 0.0),
                    (0.0, -0.0),
                    (-0.0, -0.0),
                    (f64::INFINITY, 1.0),
                    (1.0, f64::INFINITY),
                    (f64::NAN, 1.0),
                ] {
                    let expected = angle_to_centered_bin(wrap(y.atan2(x) - direction), bins);
                    assert_eq!(
                        sector_bin(Some(&classifier), x, y, x.hypot(y), direction, bins),
                        expected
                    );
                }
            }
        }
        // Ensure dense ordinary inputs exercise the optimization rather than always falling back.
        assert!(interior > ordinary * 9 / 10);
    }

    #[test]
    fn food_channels_and_raw_traces_match_reference_accumulation_for_sparse_and_dense_queries() {
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for bins in 8..=MAX_SENSOR_BINS {
                let evaluator = evaluator(profile, bins);
                for direction in [-TAU, -PI, -0.37, 0.0, PI, TAU, 1.0e20] {
                    let mut source = WorldState::default();
                    add_snake(
                        &mut source,
                        1,
                        SnakeKind::External,
                        point(0.0, 0.0),
                        direction,
                        5,
                    );
                    let mut positions = Vec::new();
                    for index in 0..256 {
                        let angle = index as f64 * 2.399963229728653;
                        let distance = [
                            0.0, 50.0, 519.999999, 520.0, 520.000001, 1199.0, 1200.0, 1200.1,
                        ][index % 8];
                        positions.push(point(angle.cos() * distance, angle.sin() * distance));
                    }
                    for boundary in 0..bins {
                        let angle = direction - PI + (boundary as f64 + 0.5) * TAU / bins as f64;
                        for delta in [-1.0e-9, -1.0e-12, 0.0, 1.0e-12, 1.0e-9] {
                            for distance in [520.0, 521.0] {
                                positions.push(point(
                                    (angle + delta).cos() * distance,
                                    (angle + delta).sin() * distance,
                                ));
                            }
                        }
                    }
                    positions.extend([
                        point(0.0, 0.0),
                        point(-0.0, 0.0),
                        point(0.0, -0.0),
                        point(-0.0, -0.0),
                    ]);
                    for (index, position) in positions.into_iter().enumerate() {
                        source.pellets.push(PelletState {
                            id: index as u64 + 1000,
                            position,
                            value: [0.1, 1.0, 6.0, 8.0][index % 4],
                            kind: 0,
                            color: 0,
                            owner: None,
                        });
                    }
                    for sparse in [true, false] {
                        let mut world = source.clone();
                        if sparse {
                            world.pellets.truncate(bins - 1);
                        }
                        let before = world.clone();
                        let indexed = IndexedSensorWorld::build(
                            &world,
                            SensorIndexConfig {
                                body_cell_size: 70.0,
                                pellet_cell_size: 120.0,
                                maximum_body_entries: 1_000_000,
                                maximum_pellet_entries: 100_000,
                            },
                        )
                        .unwrap();
                        let mut rays = BodyRayIndexWorkspace::default();
                        let view = rays.prepare(indexed.body_index()).unwrap();
                        let lives = EvolvedGenerationState::default();
                        let context = V4ObservationContext {
                            generation_elapsed_seconds: 120.0,
                            evolved: &lives,
                            experiment: EvolvedExperimentConfig::legacy(240.0),
                        };
                        let mut output = vec![0.0; evaluator.layout().input_size()];
                        let mut scratch = V4SensorScratch::default();
                        let mut trace = super::super::sensor_trace::SensorTrace::default();
                        let result = evaluator
                            .sample_with_trace(
                                &indexed,
                                view,
                                context,
                                0,
                                &mut output,
                                &mut RayWorkMeter::new(4096),
                                &mut scratch,
                                Some(&mut trace),
                                None,
                            )
                            .unwrap();
                        let mut near = [0.0; MAX_SENSOR_BINS];
                        let mut far = [0.0; MAX_SENSOR_BINS];
                        let cfg = evaluator.common.config();
                        for pellet in indexed.pellet_index().candidates(&scratch.pellets) {
                            let (dx, dy) = (pellet.position.x, pellet.position.y);
                            let distance = dx.hypot(dy);
                            // Independent old v4 bin formula and original ordered accumulation.
                            let bin = angle_to_centered_bin(wrap(dy.atan2(dx) - direction), bins);
                            let strength = (1.0 - distance / result.radii.far).clamp(0.0, 1.0)
                                * (pellet.value / cfg.food_value).clamp(0.0, 6.0);
                            if distance <= result.radii.near {
                                near[bin] += strength;
                            } else {
                                far[bin] += strength;
                            }
                        }
                        for bin in 0..bins {
                            assert_eq!(trace.food_strength[bin].to_bits(), near[bin].to_bits());
                            assert_eq!(trace.far_food_strength[bin].to_bits(), far[bin].to_bits());
                            assert_eq!(
                                output[24 + bin].to_bits(),
                                saturate(near[bin], cfg.food_saturation).to_bits()
                            );
                            assert_eq!(
                                output[24 + bins + bin].to_bits(),
                                saturate(far[bin], cfg.food_saturation).to_bits()
                            );
                        }
                        assert_eq!(world, before);
                    }
                }
            }
        }
    }

    #[test]
    fn body_head_and_projected_sectors_match_original_scan_across_profiles_and_bins() {
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for bins in 8..=MAX_SENSOR_BINS {
                let evaluator = evaluator(profile, bins);
                for direction in [-TAU, -PI, -0.37, 0.0, PI, TAU, 1.0e20] {
                    let mut world = WorldState::default();
                    add_snake(
                        &mut world,
                        1,
                        SnakeKind::External,
                        point(0.0, 0.0),
                        direction,
                        5,
                    );
                    for index in 0..bins * 2 {
                        let angle = direction - PI
                            + (index as f64 + 0.5) * TAU / bins as f64
                            + [-1.0e-12, 0.0, 1.0e-12][index % 3];
                        let distance = [0.0, 20.0, 100.0, 519.0, 530.0, 600.0, 1200.0][index % 7];
                        let heading = angle + [0.0, PI, PI / 2.0][index % 3];
                        add_snake(
                            &mut world,
                            900 - index as u64 * 13,
                            SnakeKind::External,
                            point(angle.cos() * distance, angle.sin() * distance),
                            heading,
                            10 + index % 3,
                        );
                        let other = world.snakes.last_mut().unwrap();
                        other.speed = [0.0, 165.0, 247.5, 1.0e5][index % 4];
                        other.alive = index % 11 != 0;
                    }
                    let before = world.clone();
                    let indexed = IndexedSensorWorld::build(
                        &world,
                        SensorIndexConfig {
                            body_cell_size: 70.0,
                            pellet_cell_size: 120.0,
                            maximum_body_entries: 1_000_000,
                            maximum_pellet_entries: 100_000,
                        },
                    )
                    .unwrap();
                    let mut rays = BodyRayIndexWorkspace::default();
                    let view = rays.prepare(indexed.body_index()).unwrap();
                    let lives = EvolvedGenerationState::default();
                    let context = V4ObservationContext {
                        generation_elapsed_seconds: 120.0,
                        evolved: &lives,
                        experiment: EvolvedExperimentConfig::legacy(240.0),
                    };
                    let mut output = vec![0.0; evaluator.layout().input_size()];
                    let mut scratch = V4SensorScratch::default();
                    let mut trace = super::super::sensor_trace::SensorTrace::default();
                    let result = evaluator
                        .sample_with_trace(
                            &indexed,
                            view,
                            context,
                            0,
                            &mut output,
                            &mut RayWorkMeter::new(4096),
                            &mut scratch,
                            Some(&mut trace),
                            None,
                        )
                        .unwrap();
                    let cfg = evaluator.common.config();
                    let snake = &world.snakes[0];
                    let mut body = [result.radii.near; MAX_SENSOR_BINS];
                    if result.diagnostics.body.candidate_limit_reached {
                        body.fill(0.0);
                    } else {
                        for segment in indexed.body_index().sensor_candidates(&scratch.body) {
                            let closest = closest_point_on_segment(
                                snake.position,
                                segment.start,
                                segment.end,
                            );
                            let clearance = (closest.distance_squared.sqrt()
                                - (snake.radius + segment.owner_radius) * cfg.collision_hit_scale)
                                .max(0.0);
                            if clearance <= result.radii.near {
                                let bin = angle_to_centered_bin(
                                    wrap(
                                        (closest.point.y - snake.position.y)
                                            .atan2(closest.point.x - snake.position.x)
                                            - direction,
                                    ),
                                    bins,
                                );
                                body[bin] = body[bin].min(clearance);
                            }
                        }
                    }
                    let mut counts = [0_usize; MAX_SENSOR_BINS];
                    let mut clearance = [result.radii.near; MAX_SENSOR_BINS];
                    let mut nearest = [usize::MAX; MAX_SENSOR_BINS];
                    let mut urgency = [0.0_f64; MAX_SENSOR_BINS];
                    let mut seconds = [None::<f64>; MAX_SENSOR_BINS];
                    for (index, other) in world.snakes.iter().enumerate().skip(1) {
                        if !other.alive {
                            continue;
                        }
                        let (dx, dy) = (
                            other.position.x - snake.position.x,
                            other.position.y - snake.position.y,
                        );
                        let radius = (snake.radius + other.radius) * cfg.collision_hit_scale;
                        let gap = (dx.hypot(dy) - radius).max(0.0);
                        if gap <= result.radii.near {
                            let bin = angle_to_centered_bin(wrap(dy.atan2(dx) - direction), bins);
                            counts[bin] += 1;
                            if nearest[bin] == usize::MAX
                                || gap < clearance[bin]
                                || (gap == clearance[bin]
                                    && other.id < world.snakes[nearest[bin]].id)
                            {
                                nearest[bin] = index;
                                clearance[bin] = gap;
                            }
                        }
                        if profile == SensorProfile::Motion {
                            let encounter = projected_head_encounter(
                                point(dx, dy),
                                point(
                                    other.direction.cos() * other.speed
                                        - direction.cos() * snake.speed,
                                    other.direction.sin() * other.speed
                                        - direction.sin() * snake.speed,
                                ),
                                radius,
                            )
                            .unwrap();
                            if let Some(encounter) = encounter {
                                let bin = angle_to_centered_bin(
                                    wrap(
                                        encounter
                                            .relative_at_contact
                                            .y
                                            .atan2(encounter.relative_at_contact.x)
                                            - direction,
                                    ),
                                    bins,
                                );
                                urgency[bin] = urgency[bin]
                                    .max(1.0 - encounter.seconds / MOTION_HORIZON_SECONDS);
                                seconds[bin] =
                                    Some(seconds[bin].map_or(encounter.seconds, |previous| {
                                        previous.min(encounter.seconds)
                                    }));
                            }
                        }
                    }
                    for bin in 0..bins {
                        assert_eq!(trace.body_clearance[bin].to_bits(), body[bin].to_bits());
                        assert_eq!(
                            trace.head_clearance[bin].to_bits(),
                            clearance[bin].to_bits()
                        );
                        assert_eq!(trace.head_count[bin], counts[bin]);
                        let id = if nearest[bin] == usize::MAX {
                            0
                        } else {
                            world.snakes[nearest[bin]].id
                        };
                        assert_eq!(trace.head_ids[bin], id);
                        assert_eq!(trace.urgency[bin].to_bits(), urgency[bin].to_bits());
                        assert_eq!(trace.encounter_seconds[bin], seconds[bin]);
                        assert_eq!(
                            output[24 + 2 * bins + bin].to_bits(),
                            bipolar(body[bin] / result.radii.near).to_bits()
                        );
                        assert_eq!(
                            output[24 + 5 * bins + bin].to_bits(),
                            bipolar(clearance[bin] / result.radii.near).to_bits()
                        );
                        if profile == SensorProfile::Motion {
                            assert_eq!(result.representative_head_ids[bin], id);
                            assert_eq!(
                                output[24 + 13 * bins + bin].to_bits(),
                                (urgency[bin] as f32).to_bits()
                            );
                            assert_eq!(
                                output[24 + 14 * bins + bin].to_bits(),
                                ((counts[bin] as f64 / (counts[bin] as f64 + 4.0)) as f32)
                                    .to_bits()
                            );
                        }
                    }
                    assert_eq!(world, before);
                }
            }
        }
    }

    #[test]
    fn shared_head_facts_preserve_every_vector_receipt_and_trace_across_profiles_bins_and_grants() {
        let mut world = WorldState::default();
        for (index, heading) in [0.0, PI, PI / 2.0, -PI / 2.0, TAU * 1.0e8, -TAU * 1.0e8]
            .into_iter()
            .enumerate()
        {
            add_snake(
                &mut world,
                900 - index as u64 * 13,
                SnakeKind::External,
                point(index as f64 * 37.0 - 100.0, (index % 3) as f64 * 41.0),
                heading,
                10 + index,
            );
            world.snakes[index].speed = [0.0, 165.0, 247.5, 42.0, 1.0e5, 5.0][index];
        }
        let original = world.clone();
        let indexed = IndexedSensorWorld::build(
            &world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 100_000,
            },
        )
        .unwrap();
        let mut rays = BodyRayIndexWorkspace::default();
        let body_rays = rays.prepare(indexed.body_index()).unwrap();
        let lives = EvolvedGenerationState::default();
        let context = V4ObservationContext {
            generation_elapsed_seconds: 120.0,
            evolved: &lives,
            experiment: EvolvedExperimentConfig::legacy(240.0),
        };
        let mut heads = V4HeadWorkspace::try_new(world.snakes.len()).unwrap();
        let capacity = heads.capacity();
        let mut reference_scratch = V4SensorScratch::default();
        let mut cached_scratch = V4SensorScratch::default();
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            let prepared = heads.prepare(&world, profile).unwrap().unwrap();
            for bins in 8..=32 {
                let evaluator = evaluator(profile, bins);
                for grant in [0, 7, 128, 4096] {
                    for observer in 0..world.snakes.len() {
                        let mut reference = vec![0.0; evaluator.layout().input_size()];
                        let mut cached = reference.clone();
                        let mut reference_trace =
                            super::super::sensor_trace::SensorTrace::default();
                        let mut cached_trace = super::super::sensor_trace::SensorTrace::default();
                        let mut reference_meter = RayWorkMeter::new(grant);
                        let mut cached_meter = RayWorkMeter::new(grant);
                        let expected = evaluator
                            .sample_with_trace(
                                &indexed,
                                body_rays,
                                context,
                                observer,
                                &mut reference,
                                &mut reference_meter,
                                &mut reference_scratch,
                                Some(&mut reference_trace),
                                None,
                            )
                            .unwrap();
                        let actual = evaluator
                            .sample_with_trace(
                                &indexed,
                                body_rays,
                                context,
                                observer,
                                &mut cached,
                                &mut cached_meter,
                                &mut cached_scratch,
                                Some(&mut cached_trace),
                                Some(prepared),
                            )
                            .unwrap();
                        assert_eq!(
                            reference
                                .iter()
                                .map(|value| value.to_bits())
                                .collect::<Vec<_>>(),
                            cached
                                .iter()
                                .map(|value| value.to_bits())
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(actual, expected);
                        assert_eq!(cached_meter, reference_meter);
                        assert_eq!(cached_trace, reference_trace);
                    }
                }
            }
            assert_eq!(heads.capacity(), capacity);
        }
        assert_eq!(world, original);
    }

    #[test]
    fn shared_head_facts_reject_wrong_world_profile_and_invalid_alive_heads_without_growth() {
        let mut world = WorldState::default();
        add_snake(&mut world, 7, SnakeKind::External, point(0.0, 0.0), 0.0, 10);
        add_snake(
            &mut world,
            99,
            SnakeKind::Baseline,
            point(80.0, 0.0),
            PI,
            20,
        );
        let other_world = world.clone();
        let mut heads = V4HeadWorkspace::try_new(2).unwrap();
        let prepared = heads
            .prepare(&world, SensorProfile::Motion)
            .unwrap()
            .unwrap();
        assert!(prepared.is_from(&world, SensorProfile::Motion));
        assert!(!prepared.is_from(&other_world, SensorProfile::Motion));
        assert!(!prepared.is_from(&world, SensorProfile::Basic));
        let indexed = IndexedSensorWorld::build(
            &other_world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 100_000,
            },
        )
        .unwrap();
        let mut rays = BodyRayIndexWorkspace::default();
        let view = rays.prepare(indexed.body_index()).unwrap();
        let lives = EvolvedGenerationState::default();
        let evaluator = evaluator(SensorProfile::Motion, 8);
        let mut output = vec![0.25; evaluator.layout().input_size()];
        assert_eq!(
            evaluator.sample_with_trace(
                &indexed,
                view,
                V4ObservationContext {
                    generation_elapsed_seconds: 0.0,
                    evolved: &lives,
                    experiment: EvolvedExperimentConfig::legacy(240.0),
                },
                0,
                &mut output,
                &mut RayWorkMeter::new(128),
                &mut V4SensorScratch::default(),
                None,
                Some(prepared)
            ),
            Err(V4SensorError::Shape)
        );
        assert!(output.iter().all(|value| *value == 0.25));
        let capacity = heads.capacity();
        world.snakes[1].speed = f64::NAN;
        assert!(matches!(
            heads.prepare(&world, SensorProfile::Motion),
            Err(V4SensorError::Shape)
        ));
        world.snakes[1].alive = false;
        assert!(heads
            .prepare(&world, SensorProfile::Motion)
            .unwrap()
            .is_some());
        assert_eq!(heads.capacity(), capacity);
        let mut short = V4HeadWorkspace::try_new(1).unwrap();
        assert!(short
            .prepare(&world, SensorProfile::Motion)
            .unwrap()
            .is_none());
        assert_eq!(short.capacity(), 1);
        assert!(matches!(
            short.prepare(&world, SensorProfile::V3),
            Err(V4SensorError::Config)
        ));
        assert!(V4HeadWorkspace::try_new(usize::MAX).is_err());
    }

    #[test]
    fn raw_trace_preserves_quota_results_and_distinguishes_incomplete_rays() {
        use crate::engine::sensor_trace::{SensorTrace, SensorTraceRay};
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 10);
        add_snake(&mut world, 2, SnakeKind::Baseline, point(80.0, 0.0), PI, 20);
        let indexed = IndexedSensorWorld::build(
            &world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 100_000,
            },
        )
        .unwrap();
        let mut rays = BodyRayIndexWorkspace::default();
        let view = rays.prepare(indexed.body_index()).unwrap();
        let lives = EvolvedGenerationState::default();
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for grant in [0, 4096] {
                let evaluator = evaluator(profile, 16);
                let context = V4ObservationContext {
                    generation_elapsed_seconds: 120.0,
                    evolved: &lives,
                    experiment: EvolvedExperimentConfig::legacy(240.0),
                };
                let mut expected = vec![0.0; evaluator.layout().input_size()];
                let mut actual = expected.clone();
                let mut plain_meter = RayWorkMeter::new(grant);
                let plain = evaluator
                    .sample(
                        &indexed,
                        view,
                        context,
                        0,
                        &mut expected,
                        &mut plain_meter,
                        &mut V4SensorScratch::default(),
                    )
                    .unwrap();
                let mut trace = SensorTrace::default();
                let mut traced_meter = RayWorkMeter::new(grant);
                let traced = evaluator
                    .sample_with_trace(
                        &indexed,
                        view,
                        context,
                        0,
                        &mut actual,
                        &mut traced_meter,
                        &mut V4SensorScratch::default(),
                        Some(&mut trace),
                        None,
                    )
                    .unwrap();
                assert_eq!(actual, expected);
                assert_eq!(traced.diagnostics, plain.diagnostics);
                assert_eq!(traced.delivery, plain.delivery);
                assert_eq!(traced_meter, plain_meter);
                assert!(trace.valid);
                assert_eq!(trace.physical_id, 1);
                assert_eq!(trace.head_ids[8], 2);
                assert_eq!(trace.head_count[8], 1);
                assert_eq!(
                    trace.head_clearance[8],
                    80.0 - 18.0 * evaluator.common.config().collision_hit_scale
                );
                if grant == 0 {
                    assert!(trace.body_rays[..16].contains(&SensorTraceRay::Incomplete));
                } else {
                    assert!(trace.body_rays[..16]
                        .iter()
                        .all(|ray| matches!(ray, SensorTraceRay::Hit(_) | SensorTraceRay::Clear)));
                    assert!(trace.body_rays[..16]
                        .iter()
                        .any(|ray| matches!(ray, SensorTraceRay::Hit(_))));
                }
                assert!(trace.body_rays[16..]
                    .iter()
                    .all(|ray| *ray == SensorTraceRay::NotSampled));
                if profile == SensorProfile::Motion {
                    assert_eq!(trace.relative_forward_speed[8], -330.0);
                    assert_eq!(trace.head_length_ratio[8], 2.0);
                    assert!(trace.encounter_seconds.iter().any(Option::is_some));
                }
                let mut invalid = vec![];
                assert!(evaluator
                    .sample_with_trace(
                        &indexed,
                        view,
                        context,
                        0,
                        &mut invalid,
                        &mut traced_meter,
                        &mut V4SensorScratch::default(),
                        Some(&mut trace),
                        None,
                    )
                    .is_err());
                assert!(!trace.valid);
            }
        }
    }
    fn simple(profile: SensorProfile, world: &WorldState) -> (Vec<f32>, V4SensorSample) {
        sample(
            &evaluator(profile, 16),
            world,
            &EvolvedGenerationState::default(),
            EvolvedExperimentConfig::legacy(240.0),
            1024,
        )
    }
    fn close(actual: f32, expected: f64) {
        assert!(
            (f64::from(actual) - expected).abs() < 1e-6,
            "{actual} != {expected}"
        );
    }

    fn baseline_v2_action(
        profile: SensorProfile,
        world: &WorldState,
        grant: u32,
    ) -> (Vec<f32>, super::super::baseline_v2::BaselineV2Action) {
        use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Policy, BaselineV2State};
        let sensors = evaluator(profile, 16);
        let (values, _) = sample(
            &sensors,
            world,
            &EvolvedGenerationState::default(),
            EvolvedExperimentConfig::legacy(240.0),
            grant,
        );
        let layout = super::super::sensor_layout::SensorLayout::for_profile(profile, 16).unwrap();
        let policy = BaselineV2Policy::new(
            &layout,
            SensorConfig::default(),
            BaselineV2Mode::Deterministic,
        )
        .unwrap();
        let action = policy
            .evaluate(&values, BaselineV2State::default(), 12345, 1.0 / 60.0)
            .unwrap();
        (values, action)
    }
    #[test]
    fn baseline_v2_actual_crossing_and_wall_samples_avoid_known_routes_without_probes() {
        let mut crossing = WorldState::default();
        add_snake(
            &mut crossing,
            1,
            SnakeKind::External,
            point(0.0, 0.0),
            0.0,
            5,
        );
        crossing.snakes[0].speed = 100.0;
        add_snake(
            &mut crossing,
            2,
            SnakeKind::Baseline,
            point(40.0, 40.0),
            -PI / 2.0,
            5,
        );
        crossing.snakes[1].speed = 100.0;
        let before = crossing.clone();
        let (values, action) = baseline_v2_action(SensorProfile::Motion, &crossing, 1024);
        assert!(action.urgent_sectors != 0);
        assert_eq!(action.urgent_sectors & (1 << action.sector), 0);
        assert!(!action.unsafe_route);
        assert_eq!(crossing, before);
        assert_eq!(values[21], 1.0);
        let mut wall = WorldState::default();
        add_snake(
            &mut wall,
            1,
            SnakeKind::External,
            point(3460.0, 0.0),
            0.0,
            50,
        );
        let (_, escape) = baseline_v2_action(SensorProfile::Motion, &wall, 1024);
        assert_ne!(escape.sector, 8);
        assert!(escape.turn.abs() > 0.5 && !escape.boost);
        assert_eq!(escape.uniform_draws, 0);
    }
    #[test]
    fn baseline_v2_actual_head_on_differs_from_receding_without_privileged_target_identity() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 50);
        world.snakes[0].speed = 100.0;
        world.snakes[0].points = 100.0;
        add_snake(&mut world, 2, SnakeKind::Baseline, point(70.0, 0.0), PI, 10);
        world.snakes[1].speed = 100.0;
        let (approaching, action) = baseline_v2_action(SensorProfile::Motion, &world, 1024);
        assert!(action.urgent_sectors != 0);
        assert_ne!(action.sector, 8);
        world.snakes[1].direction = 0.0;
        let (receding, other) = baseline_v2_action(SensorProfile::Motion, &world, 1024);
        assert_eq!(other.urgent_sectors, 0);
        assert!(approaching[24 + 13 * 16 + 8] > receding[24 + 13 * 16 + 8]);
        world.snakes[1].id = 123456;
        world.snakes[1].frame_v1_id = 123456;
        let (renamed, renamed_action) = baseline_v2_action(SensorProfile::Motion, &world, 1024);
        assert_eq!(receding, renamed);
        assert_eq!(other, renamed_action);
    }
    #[test]
    fn baseline_v2_actual_smaller_pursuit_and_larger_threat_use_visible_size_and_economy() {
        let mut pursuit = WorldState::default();
        add_snake(
            &mut pursuit,
            1,
            SnakeKind::External,
            point(0.0, 0.0),
            0.0,
            50,
        );
        pursuit.snakes[0].points = 100.0;
        add_snake(
            &mut pursuit,
            2,
            SnakeKind::Baseline,
            point(550.0, 0.0),
            0.0,
            5,
        );
        pursuit.snakes[1].speed = 100.0;
        let (_, attack) = baseline_v2_action(SensorProfile::Motion, &pursuit, 1024);
        assert_eq!(attack.sector, 8);
        assert!(attack.boost);
        let mut threat = WorldState::default();
        add_snake(
            &mut threat,
            1,
            SnakeKind::External,
            point(0.0, 0.0),
            0.0,
            50,
        );
        threat.snakes[0].points = 100.0;
        add_snake(
            &mut threat,
            2,
            SnakeKind::Baseline,
            point(550.0, 0.0),
            PI,
            100,
        );
        threat.snakes[1].speed = 100.0;
        let (_, escape) = baseline_v2_action(SensorProfile::Motion, &threat, 1024);
        assert_ne!(escape.sector, 8);
        assert!(!escape.boost && !escape.unsafe_route);
    }

    #[test]
    fn baseline_v2_actual_saturated_capsules_disable_boost_and_remain_finite_for_both_profiles() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 50);
        world.snakes[0].points = 100.0;
        add_snake(
            &mut world,
            2,
            SnakeKind::Baseline,
            point(400.0, 0.0),
            0.0,
            5,
        );
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            let (values, action) = baseline_v2_action(profile, &world, 0);
            assert_eq!(values[23], 1.0);
            assert!(!action.boost);
            assert!(action.turn.is_finite() && action.normalized_score.is_finite());
        }
    }

    #[test]
    fn exact_scalar_prefix_uses_local_economics_whole_step_motion_and_actual_cost() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 25);
        let snake = &mut world.snakes[0];
        snake.points = 140.0;
        snake.delivered_observation_points = 130.0;
        snake.age_seconds = 60.0;
        snake.input_boost = true;
        snake.turn = 0.75;
        snake.completed_step_applied_turn = -0.25;
        snake.completed_step_heading_delta = 3.2 / 60.0 * 0.5;
        snake.completed_step_speed_delta = 50.0;
        let before = world.clone();
        let (values, result) = simple(SensorProfile::Motion, &world);
        close(values[0], 0.0);
        close(values[1], 1.0);
        close(values[2], -1.0 / 3.0);
        close(values[3], 0.0);
        close(values[4], -0.34);
        close(values[5], 0.0);
        let cost = 7.0 * (1.0 + 1.1 * 20.0 / 9995.0);
        close(values[6], 2.0 * 138.8 / (138.8 + 2.0 * cost) - 1.0);
        close(values[7], 2.0 * cost / (cost + 7.0) - 1.0);
        assert_eq!(&values[8..11], &[1.0, -1.0, 1.0]);
        close(values[11], 0.75);
        close(values[12], -0.25);
        close(values[13], 0.5);
        close(values[14], 0.1);
        close(values[15], 0.5);
        close(values[16], 2.0 * 140.0 / 340.0 - 1.0);
        close(values[17], 2.0 * 140.0 / 1140.0 - 1.0);
        close(values[18], 0.0);
        close(values[19], 0.5);
        close(values[20], 0.0);
        assert_eq!(&values[21..24], &[1.0, -1.0, -1.0]);
        assert!((result.radii.near - (520.0 + 260.0 / 3.0)).abs() < 1e-9);
        assert_eq!(world, before);
        assert!(!world.snakes[0].observation_delivered);
        result.delivery.commit(&mut world.snakes[0]).unwrap();
        let (second, _) = simple(SensorProfile::Motion, &world);
        assert_eq!(second[21], -1.0);
        assert_eq!(second[15], 0.0);
    }

    #[test]
    fn food_rings_have_exact_near_boundary_and_coincident_food_has_finite_declared_sector() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
        for (id, distance) in [
            (10, 0.0),
            (11, 520.0),
            (12, 521.0),
            (13, 1200.0),
            (14, 1200.1),
        ] {
            world.pellets.push(PelletState {
                id,
                position: point(distance, 0.0),
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            });
        }
        let (values, result) = simple(SensorProfile::Basic, &world);
        let near = 1.0 + 1.0 - 520.0 / 1200.0;
        let far = 1.0 - 521.0 / 1200.0;
        close(values[24 + 8], 2.0 * near / (near + 4.0) - 1.0);
        close(values[24 + 16 + 8], 2.0 * far / (far + 4.0) - 1.0);
        assert_eq!(result.diagnostics.pellets.candidates, 4);
        for bin in 0..16 {
            if bin != 8 {
                assert_eq!(values[24 + bin], -1.0);
                assert_eq!(values[40 + bin], -1.0);
            }
        }
    }

    #[test]
    fn all_profiles_bins_and_kind_sentinels_have_finite_exact_widths_and_absence() {
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for bins in 8..=32 {
                for (kind, sentinel) in [
                    (SnakeKind::Evolved, -1.0),
                    (SnakeKind::External, 0.0),
                    (SnakeKind::Baseline, 0.0),
                    (SnakeKind::Resurrected, 0.0),
                ] {
                    let mut world = WorldState::default();
                    add_snake(&mut world, 1, kind, point(0.0, 0.0), 0.0, 5);
                    let (values, result) = sample(
                        &evaluator(profile, bins),
                        &world,
                        &EvolvedGenerationState::default(),
                        EvolvedExperimentConfig::legacy(240.0),
                        1024,
                    );
                    assert_eq!(
                        values.len(),
                        24 + if profile == SensorProfile::Basic {
                            6
                        } else {
                            15
                        } * bins
                    );
                    assert_eq!(values[20], sentinel);
                    assert!(values
                        .iter()
                        .all(|value| value.is_finite() && (-1.0..=1.0).contains(value)));
                    assert_eq!(result.representative_head_ids, [0; 32]);
                    if profile == SensorProfile::Motion {
                        assert!(values[24 + 6 * bins..24 + 7 * bins]
                            .iter()
                            .all(|v| *v == -1.0));
                        assert!(values[24 + 7 * bins..].iter().all(|v| *v == 0.0));
                    }
                }
            }
        }
    }

    #[test]
    fn approaching_receding_and_tied_nearest_heads_share_exact_ordered_motion_channels() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
        add_snake(
            &mut world,
            30,
            SnakeKind::Baseline,
            point(100.0, 0.0),
            PI,
            20,
        );
        add_snake(
            &mut world,
            20,
            SnakeKind::Baseline,
            point(100.0, 0.0),
            PI,
            20,
        );
        let (approach, result) = simple(SensorProfile::Motion, &world);
        let bin = 8;
        assert_eq!(result.representative_head_ids[bin], 20);
        assert_eq!(approach[24 + 6 * 16 + bin], 1.0);
        close(approach[24 + 7 * 16 + bin], 0.0);
        close(approach[24 + 8 * 16 + bin], 0.5);
        close(approach[24 + 9 * 16 + bin], 0.0);
        close(approach[24 + 10 * 16 + bin], -1.0);
        close(approach[24 + 11 * 16 + bin], -0.33);
        close(approach[24 + 12 * 16 + bin], 0.0);
        close(
            approach[24 + 13 * 16 + bin],
            1.0 - (100.0 - 18.0 * 0.82) / 330.0 / 2.0,
        );
        close(approach[24 + 14 * 16 + bin], 2.0 / 6.0);
        world.snakes.swap(1, 2);
        let (reordered, ids) = simple(SensorProfile::Motion, &world);
        assert_eq!(approach, reordered);
        assert_eq!(result.representative_head_ids, ids.representative_head_ids);
        for other in &mut world.snakes[1..] {
            other.direction = 0.0;
            other.speed = 500.0;
        }
        let (recede, _) = simple(SensorProfile::Motion, &world);
        close(recede[24 + 11 * 16 + bin], 0.335);
        assert_eq!(recede[24 + 13 * 16 + bin], 0.0);
    }

    #[test]
    fn crossing_urgency_includes_nonrepresentative_heads_and_predicted_contact_sector() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
        world.snakes[0].speed = 100.0;
        add_snake(&mut world, 2, SnakeKind::Baseline, point(25.0, 0.0), 0.0, 5);
        world.snakes[1].speed = 100.0;
        add_snake(
            &mut world,
            3,
            SnakeKind::Baseline,
            point(100.0, 100.0),
            -PI / 2.0,
            5,
        );
        world.snakes[2].speed = 100.0;
        let (values, ids) = simple(SensorProfile::Motion, &world);
        assert_eq!(ids.representative_head_ids[8], 2);
        let crossing_bin = 10;
        assert!(values[24 + 13 * 16 + crossing_bin] > 0.5);
        assert_eq!(values[24 + 13 * 16 + 8], 0.0);
    }

    #[test]
    fn failed_ray_and_detail_queries_are_visible_and_do_not_consume_life_delivery() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
        add_snake(
            &mut world,
            2,
            SnakeKind::Baseline,
            point(150.0, 0.0),
            PI / 2.0,
            2202,
        );
        // Keep all other segments in the near detail range, proving the cap.
        for p in &mut world.body_points[5..] {
            *p = point(150.0, 0.0);
        }
        for id in 10..1010 {
            world.pellets.push(PelletState {
                id,
                position: point(20.0, 0.0),
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            });
        }
        let before = world.clone();
        let (values, result) = sample(
            &evaluator(SensorProfile::Basic, 16),
            &world,
            &EvolvedGenerationState::default(),
            EvolvedExperimentConfig::legacy(240.0),
            0,
        );
        assert_eq!(&values[21..24], &[1.0, 1.0, 1.0]);
        assert!(result.diagnostics.body.candidate_limit_reached);
        assert_eq!(result.diagnostics.incomplete_rays, 16);
        assert!(values[24 + 2 * 16..24 + 4 * 16].iter().all(|v| *v == -1.0));
        assert_eq!(world, before);
    }

    #[test]
    fn zero_cost_has_unbounded_budget_and_retains_actual_no_positive_spend_eligibility() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 25);
        world.snakes[0].points = 100.0;
        world.snakes[0].input_boost = true;
        let evaluator = SensorV4Evaluator::new(
            SensorProfile::Basic,
            SensorConfig {
                boost_points_cost_per_second: 0.0,
                ..SensorConfig::default()
            },
            MovementConfig {
                boost_points_cost_per_second: 0.0,
                ..MovementConfig::typescript_defaults()
            },
            1.0 / 60.0,
        )
        .unwrap();
        let (values, _) = sample(
            &evaluator,
            &world,
            &EvolvedGenerationState::default(),
            EvolvedExperimentConfig::legacy(240.0),
            128,
        );
        assert_eq!(values[6], 1.0);
        assert_eq!(values[7], -1.0);
        assert_eq!(values[8], -1.0);
        assert_eq!(values[9], -1.0);
        assert_eq!(values[10], 1.0);
    }

    #[test]
    fn evolved_carried_wallet_and_finite_unlimited_budgets_use_actual_owner_ledger() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::Evolved, point(0.0, 0.0), 0.0, 5);
        world.snakes[0].points = 80.0;
        let mut lives = EvolvedGenerationState::initialize(&world, 1, 5).unwrap();
        let owner = &mut lives.slots[0];
        owner
            .finalize(super::super::evolved_lives::EvolvedDeathCause::GodMode)
            .unwrap();
        assert_eq!(owner.begin_life(2, 5, true).unwrap(), 80.0);
        owner.alive_award(1.0, 20.0).unwrap();
        world.snakes[0].id = 2;
        world.snakes[0].frame_v1_id = 2;
        world.snakes[0].points = 100.0;
        world.snakes[0].age_seconds = 1.0;
        let policy = EvolvedExperimentConfig {
            version: 1,
            enabled: true,
            max_lives: 4,
            points: super::super::evolved_lives::PointAssessmentMode::CumulativeNet,
            ..EvolvedExperimentConfig::legacy(240.0)
        };
        let (values, _) = sample(
            &evaluator(SensorProfile::Motion, 16),
            &world,
            &lives,
            policy,
            512,
        );
        close(values[5], 2.0 * 100.0 / 240.0 - 1.0);
        close(values[16], 2.0 * 20.0 / 220.0 - 1.0);
        close(values[17], 2.0 * 100.0 / 1100.0 - 1.0);
        assert_eq!(values[20], -0.5);
        let (unlimited, _) = sample(
            &evaluator(SensorProfile::Motion, 16),
            &world,
            &lives,
            EvolvedExperimentConfig {
                max_lives: 0,
                ..policy
            },
            512,
        );
        assert_eq!(unlimited[20], 1.0);
        let (off, _) = sample(
            &evaluator(SensorProfile::Motion, 16),
            &world,
            &lives,
            EvolvedExperimentConfig {
                enabled: false,
                ..policy
            },
            512,
        );
        assert_eq!(off[20], -1.0);
    }

    #[test]
    fn malformed_shapes_and_mixed_source_indexes_reject_before_output_or_delivery_changes() {
        let mut world = WorldState::default();
        add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
        let config = SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 1000,
            maximum_pellet_entries: 1000,
        };
        let indexed = IndexedSensorWorld::build(&world, config).unwrap();
        let other_indexed = IndexedSensorWorld::build(&world, config).unwrap();
        let mut workspace = BodyRayIndexWorkspace::default();
        let mixed_view = workspace.prepare(other_indexed.body_index()).unwrap();
        let lives = EvolvedGenerationState::default();
        let context = V4ObservationContext {
            generation_elapsed_seconds: 0.0,
            evolved: &lives,
            experiment: EvolvedExperimentConfig::legacy(240.0),
        };
        let evaluator = evaluator(SensorProfile::Motion, 16);
        let mut output = vec![99.0; 264];
        let mut meter = RayWorkMeter::new(512);
        assert_eq!(
            evaluator.sample(
                &indexed,
                mixed_view,
                context,
                0,
                &mut output,
                &mut meter,
                &mut V4SensorScratch::default()
            ),
            Err(V4SensorError::Shape)
        );
        assert!(output.iter().all(|value| *value == 99.0));
        assert_eq!(meter.remaining(), 512);
        let view = workspace.prepare(indexed.body_index()).unwrap();
        assert_eq!(
            evaluator.sample(
                &indexed,
                view,
                context,
                0,
                &mut output[..263],
                &mut meter,
                &mut V4SensorScratch::default()
            ),
            Err(V4SensorError::Shape)
        );
        assert!(output.iter().all(|value| *value == 99.0));
        assert!(!world.snakes[0].observation_delivered);
        assert!(SensorV4Evaluator::new(
            SensorProfile::V3,
            SensorConfig::default(),
            MovementConfig::typescript_defaults(),
            1.0 / 60.0
        )
        .is_err());
        assert!(SensorV4Evaluator::new(
            SensorProfile::Basic,
            SensorConfig::default(),
            MovementConfig {
                snake_boost_speed: 400.0,
                ..MovementConfig::typescript_defaults()
            },
            1.0 / 60.0
        )
        .is_err());
    }

    #[test]
    fn serial_and_parallel_workers_receive_identical_complete_candidate_observations() {
        use rayon::prelude::*;
        let mut world = WorldState::default();
        for id in 1..=24 {
            add_snake(
                &mut world,
                id,
                SnakeKind::External,
                point((id % 6) as f64 * 150.0, (id / 6) as f64 * 175.0),
                id as f64 * 0.25,
                5 + id as usize % 20,
            );
        }
        let before = world.clone();
        let indexed = IndexedSensorWorld::build(
            &world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 1000,
            },
        )
        .unwrap();
        let mut rays = BodyRayIndexWorkspace::default();
        let view = rays.prepare(indexed.body_index()).unwrap();
        let lives = EvolvedGenerationState::default();
        let context = V4ObservationContext {
            generation_elapsed_seconds: 0.0,
            evolved: &lives,
            experiment: EvolvedExperimentConfig::legacy(240.0),
        };
        let evaluator = evaluator(SensorProfile::Motion, 32);
        let mut reference = None;
        let available = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(4);
        for workers in [1, available] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            let result: Vec<_> = pool.install(|| {
                (0..world.snakes.len())
                    .into_par_iter()
                    .map_init(V4SensorScratch::default, |scratch, index| {
                        let mut output = vec![0.0; evaluator.layout().input_size()];
                        let sample = evaluator
                            .sample(
                                &indexed,
                                view,
                                context,
                                index,
                                &mut output,
                                &mut RayWorkMeter::new(1024),
                                scratch,
                            )
                            .unwrap();
                        (output, sample)
                    })
                    .collect()
            });
            if let Some(expected) = &reference {
                assert_eq!(&result, expected);
            } else {
                reference = Some(result);
            }
        }
        assert_eq!(world, before);
    }

    #[cfg(feature = "engine-test-hooks")]
    #[test]
    fn prototype_budget_quality_reports_all_candidates_on_existing_native_source_shapes() {
        use crate::engine::calculation::CalculationBatchKey;
        use crate::engine::ray_work::{
            prepare_ray_quotas, RayObservationOwner, RayQuotaCursor, RayWorkLimits,
        };
        use crate::engine::sensing_fixture::{build_world, Stage4SensingScenarioName};
        for scenario in [Stage4SensingScenarioName::P0, Stage4SensingScenarioName::P1] {
            let world = build_world(scenario.spec()).unwrap();
            let indexed = IndexedSensorWorld::build(
                &world,
                SensorIndexConfig {
                    body_cell_size: 70.0,
                    pellet_cell_size: 120.0,
                    maximum_body_entries: 1_000_000,
                    maximum_pellet_entries: 100_000,
                },
            )
            .unwrap();
            let mut rays = BodyRayIndexWorkspace::default();
            let view = rays.prepare(indexed.body_index()).unwrap();
            let lives = EvolvedGenerationState::default();
            let context = V4ObservationContext {
                generation_elapsed_seconds: 120.0,
                evolved: &lives,
                experiment: EvolvedExperimentConfig::legacy(240.0),
            };
            let mut owners: Vec<_> = world
                .snakes
                .iter()
                .enumerate()
                .map(|(index, snake)| (RayObservationOwner::of(snake).unwrap(), index))
                .collect();
            owners.sort_unstable_by_key(|entry| entry.0);
            let keys: Vec<_> = owners.iter().map(|entry| entry.0).collect();
            let mut grants = vec![0; keys.len()];
            for bins in [16, 32] {
                let evaluator = evaluator(SensorProfile::Motion, bins);
                let mut output = vec![0.0; evaluator.layout().input_size()];
                let mut scratch = V4SensorScratch::default();
                for batch in [16_384, 32_768, 65_536] {
                    for cap in [128, 256, 512, 1024] {
                        // Exploration floor only. Production minimum is not selected here.
                        let limits = RayWorkLimits::new(1, cap, batch, 512).unwrap();
                        let proposal = prepare_ray_quotas(
                            CalculationBatchKey::new(1, 1, 1),
                            limits,
                            RayQuotaCursor::default(),
                            &keys,
                            &mut grants,
                        )
                        .unwrap();
                        let mut saturated = 0;
                        let mut ray_count = 0;
                        let mut charged = 0;
                        for ((_, index), grant) in owners.iter().zip(&grants) {
                            let mut meter = RayWorkMeter::new(*grant);
                            let sample = evaluator
                                .sample(
                                    &indexed,
                                    view,
                                    context,
                                    *index,
                                    &mut output,
                                    &mut meter,
                                    &mut scratch,
                                )
                                .unwrap();
                            saturated += usize::from(sample.diagnostics.incomplete_rays > 0);
                            ray_count += sample.diagnostics.incomplete_rays;
                            charged += grant - meter.remaining();
                        }
                        assert!(charged <= proposal.granted());
                        println!("prototype-v4 {scenario:?} bins={bins} batch={batch} cap={cap} samples={} saturated={saturated} incomplete_rays={ray_count} charged={charged}", owners.len());
                    }
                }
            }
        }
    }
}
