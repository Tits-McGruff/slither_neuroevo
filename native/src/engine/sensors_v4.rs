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
mod tests;
