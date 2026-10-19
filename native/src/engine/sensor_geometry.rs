//! Sensor-v4 geometry prototype. These pure held-motion estimates do not
//! advance physics, strategies, cadence, recurrent state or gameplay RNG.

use super::ray_work::{RayWorkMeter, RayWorkUnit};
use super::state::WorldPoint;
use std::fmt::{Display, Formatter};

/// Declared head-motion prediction horizon, in simulated seconds.
pub const MOTION_HORIZON_SECONDS: f64 = 2.0;
/// Far beyond supported world/speed bounds; rejects arithmetic-hostile inputs
/// at this public boundary while keeping all derived products finite.
const GEOMETRY_BOUND: f64 = 1.0e12;

/// Validated finite ray, with physical-distance parameterization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorRay {
    origin: WorldPoint,
    direction: WorldPoint,
    maximum: f64,
}

impl SensorRay {
    /// Heading is in world coordinates; maximum is the near sensing radius.
    pub fn new(
        origin: WorldPoint,
        heading: f64,
        maximum: f64,
    ) -> Result<Self, SensorGeometryError> {
        if !valid_point(origin) || !heading.is_finite() || !valid_nonnegative(maximum) {
            return Err(SensorGeometryError);
        }
        Ok(Self {
            origin,
            direction: WorldPoint {
                x: heading.cos(),
                y: heading.sin(),
            },
            maximum,
        })
    }

    pub const fn origin(self) -> WorldPoint {
        self.origin
    }
    pub const fn direction(self) -> WorldPoint {
        self.direction
    }
    pub const fn maximum(self) -> f64 {
        self.maximum
    }

    /// Conservative entry distance into an inflated world-axis bounding box.
    /// This is a broad-phase lower bound, never a capsule collision result.
    pub fn aabb_clearance(
        self,
        minimum: WorldPoint,
        maximum: WorldPoint,
        inflation: f64,
    ) -> Result<Option<f64>, SensorGeometryError> {
        if !valid_point(minimum)
            || !valid_point(maximum)
            || minimum.x > maximum.x
            || minimum.y > maximum.y
            || !valid_nonnegative(inflation)
        {
            return Err(SensorGeometryError);
        }
        let mut enter: f64 = 0.0;
        let mut exit = self.maximum;
        for (origin, delta, low, high) in [
            (
                self.origin.x,
                self.direction.x,
                minimum.x - inflation,
                maximum.x + inflation,
            ),
            (
                self.origin.y,
                self.direction.y,
                minimum.y - inflation,
                maximum.y + inflation,
            ),
        ] {
            if delta == 0.0 {
                if origin < low || origin > high {
                    return Ok(None);
                }
            } else {
                let first = (low - origin) / delta;
                let last = (high - origin) / delta;
                enter = enter.max(first.min(last));
                exit = exit.min(first.max(last));
                if enter > exit {
                    return Ok(None);
                }
            }
        }
        Ok(Some(enter))
    }

    /// Exact first intersection with the closed segment capsule, including
    /// tangency and initial overlap. Radius includes both collision radii.
    pub fn capsule_clearance(
        self,
        start: WorldPoint,
        end: WorldPoint,
        radius: f64,
    ) -> Result<Option<f64>, SensorGeometryError> {
        if !valid_point(start) || !valid_point(end) || !valid_nonnegative(radius) {
            return Err(SensorGeometryError);
        }
        let mut nearest = ray_disc(self, start, radius);
        nearest = minimum(nearest, ray_disc(self, end, radius));
        let sx = end.x - start.x;
        let sy = end.y - start.y;
        let length = sx.hypot(sy);
        if length > 0.0 {
            let ux = sx / length;
            let uy = sy / length;
            let px = self.origin.x - start.x;
            let py = self.origin.y - start.y;
            let local_origin = WorldPoint {
                x: px * ux + py * uy,
                y: -px * uy + py * ux,
            };
            let local_direction = WorldPoint {
                x: self.direction.x * ux + self.direction.y * uy,
                y: -self.direction.x * uy + self.direction.y * ux,
            };
            nearest = minimum(
                nearest,
                ray_rectangle(local_origin, local_direction, length, radius, self.maximum),
            );
        }
        Ok(nearest)
    }

    /// Meter one bounded capsule test before doing geometry. Incomplete work
    /// is a distinct result, so callers cannot accidentally publish a clear ray.
    pub fn charged_capsule_clearance(
        self,
        start: WorldPoint,
        end: WorldPoint,
        radius: f64,
        meter: &mut RayWorkMeter,
    ) -> Result<RayIntersection, SensorGeometryError> {
        if !valid_point(start) || !valid_point(end) || !valid_nonnegative(radius) {
            return Err(SensorGeometryError);
        }
        if !meter.charge(RayWorkUnit::Intersection) {
            return Ok(RayIntersection::Incomplete);
        }
        Ok(match self.capsule_clearance(start, end, radius)? {
            Some(distance) => RayIntersection::Hit(distance),
            None => RayIntersection::Clear,
        })
    }

    /// Circular arena clearance for the observer's collision disc. An observer
    /// already outside the admissible center radius receives zero clearance.
    pub fn wall_clearance(
        self,
        arena_radius: f64,
        observer_radius: f64,
    ) -> Result<f64, SensorGeometryError> {
        if !valid_nonnegative(arena_radius)
            || !valid_nonnegative(observer_radius)
            || arena_radius <= observer_radius
        {
            return Err(SensorGeometryError);
        }
        let limit = arena_radius - observer_radius;
        let distance = self.origin.x.hypot(self.origin.y);
        if distance >= limit {
            return Ok(0.0);
        }
        let projection = self.origin.x * self.direction.x + self.origin.y * self.direction.y;
        let a = self.direction.x * self.direction.x + self.direction.y * self.direction.y;
        let c = (limit - distance) * (limit + distance);
        let root = (projection * projection + a * c).sqrt();
        let clearance = if projection > 0.0 {
            c / (root + projection)
        } else {
            (root - projection) / a
        };
        Ok(clearance.min(self.maximum))
    }
}

/// A known ray result and an explicitly conservative work-limit result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayIntersection {
    Hit(f64),
    Clear,
    Incomplete,
}

/// Earliest projected collision and relative encounter bearing source. The
/// caller bins this contact vector, rather than only the current nearest head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectedHeadEncounter {
    pub seconds: f64,
    pub relative_at_contact: WorldPoint,
}

/// One immutable relative position and its exact hypot, shared by sector and motion sensing.
/// Private fields prevent a caller from supplying a norm for a different position.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeadSeparation {
    position: WorldPoint,
    distance: f64,
}

impl HeadSeparation {
    /// Measure exactly once. Motion admission validates the retained position;
    /// Basic sector sensing keeps its existing distance-only numeric behavior.
    pub(crate) fn new(position: WorldPoint) -> Self {
        Self {
            distance: position.x.hypot(position.y),
            position,
        }
    }

    pub(crate) const fn distance(self) -> f64 {
        self.distance
    }

    /// Reuse this position's norm without weakening the ordinary geometry boundary.
    pub(crate) fn projected_encounter(
        self,
        relative_velocity: WorldPoint,
        combined_radius: f64,
    ) -> Result<Option<ProjectedHeadEncounter>, SensorGeometryError> {
        validate_head_motion(self.position, relative_velocity, combined_radius)?;
        Ok(projected_head_encounter_from_separation(
            self,
            relative_velocity,
            combined_radius,
        ))
    }
}

/// Keep every motion entry point's finite/range admission identical.
fn validate_head_motion(
    position: WorldPoint,
    velocity: WorldPoint,
    radius: f64,
) -> Result<(), SensorGeometryError> {
    if !valid_point(position) || !valid_point(velocity) || !valid_nonnegative(radius) {
        return Err(SensorGeometryError);
    }
    Ok(())
}

/// Relative-motion disc intersection over the declared two-second horizon.
/// Position and velocity are other minus observer in world coordinates.
pub fn projected_head_encounter(
    relative_position: WorldPoint,
    relative_velocity: WorldPoint,
    combined_radius: f64,
) -> Result<Option<ProjectedHeadEncounter>, SensorGeometryError> {
    validate_head_motion(relative_position, relative_velocity, combined_radius)?;
    Ok(projected_head_encounter_from_separation(
        HeadSeparation::new(relative_position),
        relative_velocity,
        combined_radius,
    ))
}

/// The frozen solver, receiving the same already measured hypot and operand order.
fn projected_head_encounter_from_separation(
    separation: HeadSeparation,
    relative_velocity: WorldPoint,
    combined_radius: f64,
) -> Option<ProjectedHeadEncounter> {
    let p = separation.position;
    let v = relative_velocity;
    let distance = separation.distance;
    if distance <= combined_radius {
        return Some(ProjectedHeadEncounter {
            seconds: 0.0,
            relative_at_contact: p,
        });
    }
    let a = v.x * v.x + v.y * v.y;
    let projection = p.x * v.x + p.y * v.y;
    if a == 0.0 || projection >= 0.0 {
        return None;
    }
    let cross = p.x * v.y - p.y * v.x;
    let perpendicular_squared = cross * cross / a;
    let radius_squared = combined_radius * combined_radius;
    if perpendicular_squared > radius_squared {
        return None;
    }
    let root = (a * (radius_squared - perpendicular_squared)).sqrt();
    // Product form avoids subtracting nearly equal roots near initial contact.
    let c = (distance - combined_radius) * (distance + combined_radius);
    let seconds = c / (-projection + root);
    if seconds > MOTION_HORIZON_SECONDS || !seconds.is_finite() {
        return None;
    }
    Some(ProjectedHeadEncounter {
        seconds,
        relative_at_contact: WorldPoint {
            x: p.x + v.x * seconds,
            y: p.y + v.y * seconds,
        },
    })
}

fn ray_disc(ray: SensorRay, center: WorldPoint, radius: f64) -> Option<f64> {
    let x = center.x - ray.origin.x;
    let y = center.y - ray.origin.y;
    if x.hypot(y) <= radius {
        return Some(0.0);
    }
    let a = ray.direction.x * ray.direction.x + ray.direction.y * ray.direction.y;
    let along = x * ray.direction.x + y * ray.direction.y;
    if along < 0.0 {
        return None;
    }
    let cross = x * ray.direction.y - y * ray.direction.x;
    let perpendicular_squared = cross * cross / a;
    if perpendicular_squared > radius * radius {
        return None;
    }
    let distance = (along / a - ((radius * radius - perpendicular_squared) / a).sqrt()).max(0.0);
    (distance <= ray.maximum).then_some(distance)
}

fn ray_rectangle(
    origin: WorldPoint,
    direction: WorldPoint,
    length: f64,
    radius: f64,
    maximum: f64,
) -> Option<f64> {
    let mut enter: f64 = 0.0;
    let mut exit = maximum;
    for (position, delta, low, high) in [
        (origin.x, direction.x, 0.0, length),
        (origin.y, direction.y, -radius, radius),
    ] {
        if delta == 0.0 {
            if position < low || position > high {
                return None;
            }
        } else {
            let first = (low - position) / delta;
            let last = (high - position) / delta;
            enter = enter.max(first.min(last));
            exit = exit.min(first.max(last));
            if enter > exit {
                return None;
            }
        }
    }
    Some(enter)
}

fn minimum(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}
fn valid_nonnegative(value: f64) -> bool {
    value.is_finite() && (0.0..=GEOMETRY_BOUND).contains(&value)
}
fn valid_point(point: WorldPoint) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x.abs() <= GEOMETRY_BOUND
        && point.y.abs() <= GEOMETRY_BOUND
}

/// Reject malformed/unsupported numeric geometry at a public native boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorGeometryError;
impl Display for SensorGeometryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid sensor-v4 geometry")
    }
}
impl std::error::Error for SensorGeometryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};
    fn point(x: f64, y: f64) -> WorldPoint {
        WorldPoint { x, y }
    }
    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn shared_head_distance_preserves_numeric_edges_and_geometry_rejections() {
        let positions = [
            -1.0e12,
            -100.0,
            -10.0,
            -0.0,
            0.0,
            1.0e-300,
            10.0,
            100.0,
            1.0e12,
            f64::NAN,
            f64::INFINITY,
        ];
        let velocities = [
            point(-50.0, 0.0),
            point(0.0, 50.0),
            point(0.0, 0.0),
            point(50.0, -50.0),
            point(1.0e12, -1.0e12),
            point(f64::NAN, 0.0),
        ];
        let radii = [0.0, 10.0, 1.0e12, -1.0, f64::NAN];
        let bits = |result: Result<Option<ProjectedHeadEncounter>, SensorGeometryError>| {
            result.map(|encounter| {
                encounter.map(|value| {
                    [
                        value.seconds.to_bits(),
                        value.relative_at_contact.x.to_bits(),
                        value.relative_at_contact.y.to_bits(),
                    ]
                })
            })
        };
        for x in positions {
            for y in positions {
                let position = point(x, y);
                let separation = HeadSeparation::new(position);
                assert_eq!(separation.distance().to_bits(), x.hypot(y).to_bits());
                for velocity in velocities {
                    for radius in radii {
                        assert_eq!(
                            bits(separation.projected_encounter(velocity, radius)),
                            bits(projected_head_encounter(position, velocity, radius))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn capsule_side_endpoint_degenerate_and_initial_overlap_are_exact() {
        let ray = SensorRay::new(point(0.0, 0.0), 0.0, 100.0).unwrap();
        close(
            ray.capsule_clearance(point(20.0, -30.0), point(20.0, 30.0), 3.0)
                .unwrap()
                .unwrap(),
            17.0,
        );
        close(
            ray.capsule_clearance(point(20.0, 0.0), point(30.0, 0.0), 3.0)
                .unwrap()
                .unwrap(),
            17.0,
        );
        close(
            ray.capsule_clearance(point(20.0, 0.0), point(20.0, 0.0), 3.0)
                .unwrap()
                .unwrap(),
            17.0,
        );
        assert_eq!(
            ray.capsule_clearance(point(-10.0, 0.0), point(10.0, 0.0), 1.0)
                .unwrap(),
            Some(0.0)
        );
        assert_eq!(
            ray.capsule_clearance(point(20.0, 8.0), point(30.0, 8.0), 3.0)
                .unwrap(),
            None
        );
    }

    #[test]
    fn closed_capsules_include_tangency_and_range_endpoint_without_false_clearance() {
        let ray = SensorRay::new(point(0.0, 0.0), 0.0, 10.0).unwrap();
        assert_eq!(
            ray.capsule_clearance(point(10.0, 3.0), point(10.0, 3.0), 3.0)
                .unwrap(),
            Some(10.0)
        );
        assert_eq!(
            ray.capsule_clearance(point(10.0, 3.0001), point(10.0, 3.0001), 3.0)
                .unwrap(),
            None
        );
        assert_eq!(
            ray.capsule_clearance(point(12.0, 0.0), point(12.0, 0.0), 2.0)
                .unwrap(),
            Some(10.0)
        );
        assert_eq!(
            ray.capsule_clearance(point(12.0001, 0.0), point(12.0001, 0.0), 2.0)
                .unwrap(),
            None
        );
        assert_eq!(
            ray.capsule_clearance(point(-20.0, 0.0), point(-10.0, 0.0), 3.0)
                .unwrap(),
            None
        );
    }

    #[test]
    fn rotated_long_body_route_uses_capsule_geometry_not_midpoint_distance() {
        let ray = SensorRay::new(point(0.0, 0.0), FRAC_PI_2, 100.0).unwrap();
        close(
            ray.capsule_clearance(point(-1000.0, 25.0), point(5.0, 25.0), 4.0)
                .unwrap()
                .unwrap(),
            21.0,
        );
        let backwards = SensorRay::new(point(0.0, 0.0), PI, 100.0).unwrap();
        close(
            backwards
                .capsule_clearance(point(-25.0, -1000.0), point(-25.0, 5.0), 4.0)
                .unwrap()
                .unwrap(),
            21.0,
        );
    }

    #[test]
    fn wall_uses_observer_disc_and_reports_outside_and_bounded_routes() {
        close(
            SensorRay::new(point(80.0, 0.0), 0.0, 500.0)
                .unwrap()
                .wall_clearance(100.0, 10.0)
                .unwrap(),
            10.0,
        );
        close(
            SensorRay::new(point(80.0, 0.0), PI, 500.0)
                .unwrap()
                .wall_clearance(100.0, 10.0)
                .unwrap(),
            170.0,
        );
        close(
            SensorRay::new(point(0.0, 0.0), 0.0, 50.0)
                .unwrap()
                .wall_clearance(100.0, 10.0)
                .unwrap(),
            50.0,
        );
        close(
            SensorRay::new(point(95.0, 0.0), 0.0, 500.0)
                .unwrap()
                .wall_clearance(100.0, 10.0)
                .unwrap(),
            0.0,
        );
        close(
            SensorRay::new(point(89.999999, 0.0), 0.0, 500.0)
                .unwrap()
                .wall_clearance(100.0, 10.0)
                .unwrap(),
            0.000001,
        );
    }

    #[test]
    fn approaching_receding_stationary_and_overlap_heads_have_distinct_encounters() {
        let approach = projected_head_encounter(point(100.0, 0.0), point(-50.0, 0.0), 10.0)
            .unwrap()
            .unwrap();
        close(approach.seconds, 1.8);
        close(approach.relative_at_contact.x, 10.0);
        assert_eq!(
            projected_head_encounter(point(100.0, 0.0), point(50.0, 0.0), 10.0).unwrap(),
            None
        );
        assert_eq!(
            projected_head_encounter(point(100.0, 0.0), point(0.0, 0.0), 10.0).unwrap(),
            None
        );
        assert_eq!(
            projected_head_encounter(point(5.0, 0.0), point(50.0, 0.0), 10.0)
                .unwrap()
                .unwrap()
                .seconds,
            0.0
        );
    }

    #[test]
    fn crossing_threat_is_projected_at_contact_and_not_hidden_by_current_bearing() {
        let encounter = projected_head_encounter(point(100.0, 100.0), point(-50.0, -50.0), 10.0)
            .unwrap()
            .unwrap();
        close(encounter.seconds, 2.0 - 0.2 / 2.0f64.sqrt());
        close(
            encounter
                .relative_at_contact
                .x
                .hypot(encounter.relative_at_contact.y),
            10.0,
        );
        assert_eq!(
            projected_head_encounter(point(100.0, 10.0001), point(-50.0, 0.0), 10.0).unwrap(),
            None
        );
        let tangent = projected_head_encounter(point(100.0, 10.0), point(-50.0, 0.0), 10.0)
            .unwrap()
            .unwrap();
        close(tangent.seconds, 2.0);
        assert_eq!(
            projected_head_encounter(point(100.0001, 10.0), point(-50.0, 0.0), 10.0).unwrap(),
            None
        );
    }

    #[test]
    fn exhausted_work_remains_unknown_instead_of_a_false_clear_route() {
        let ray = SensorRay::new(point(0.0, 0.0), 0.0, 100.0).unwrap();
        let mut meter = RayWorkMeter::new(1);
        assert_eq!(
            ray.charged_capsule_clearance(point(20.0, 8.0), point(30.0, 8.0), 3.0, &mut meter)
                .unwrap(),
            RayIntersection::Clear
        );
        assert_eq!(
            ray.charged_capsule_clearance(point(20.0, -30.0), point(20.0, 30.0), 3.0, &mut meter)
                .unwrap(),
            RayIntersection::Incomplete
        );
        assert_eq!(meter.diagnostics().intersections, 1);
        assert!(meter.diagnostics().saturated);
        let before = meter;
        assert!(ray
            .charged_capsule_clearance(point(f64::NAN, 0.0), point(0.0, 0.0), 3.0, &mut meter)
            .is_err());
        assert_eq!(meter, before);
    }

    #[test]
    fn malformed_and_arithmetic_hostile_inputs_reject_before_geometry() {
        assert!(SensorRay::new(point(f64::NAN, 0.0), 0.0, 1.0).is_err());
        assert!(SensorRay::new(point(0.0, 0.0), f64::INFINITY, 1.0).is_err());
        assert!(SensorRay::new(point(0.0, 0.0), 0.0, -1.0).is_err());
        let ray = SensorRay::new(point(0.0, 0.0), 0.0, 1.0).unwrap();
        assert!(ray
            .capsule_clearance(point(1e300, 0.0), point(0.0, 0.0), 1.0)
            .is_err());
        assert!(ray.wall_clearance(1.0, 1.0).is_err());
        assert!(projected_head_encounter(point(1.0, 0.0), point(1.0, f64::NAN), 1.0).is_err());
    }
}
