//! Opt-in fixed-size diagnostics copied from an actual sensor evaluation.
//! No inverse normalization, spatial query, RNG draw or delivery commit occurs here.

use super::sensor_contract::SensorProfile;
use super::sensor_layout::MAX_SENSOR_BINS;
use super::sensors::SensorRadii;
use super::state::SnakeState;

/// A body ray's actual result; an exhausted query must never look clear.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SensorTraceRay {
    #[default]
    NotSampled,
    Hit(f64),
    Clear,
    Incomplete,
}

/// One physical sample's unnormalized measurements, independent of model execution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorTrace {
    pub valid: bool,
    pub physical_id: u64,
    pub profile: SensorProfile,
    pub bins: usize,
    pub near_radius: f64,
    pub far_radius: f64,
    pub body_length: usize,
    pub age_seconds: f64,
    pub food: f64,
    pub kills: u64,
    pub fitness: f64,
    pub size_factor: f64,
    pub boost_cost: f64,
    pub boost_headroom: f64,
    pub boost_eligible: Option<bool>,
    pub assessment_points: Option<f64>,
    pub life_points: Option<f64>,
    pub generation_best_points: Option<f64>,
    pub generation_elapsed_seconds: Option<f64>,
    pub generation_seconds: f64,
    pub food_saturation: f64,
    pub boost_speed: f64,
    pub nearest_food_distance: Option<f64>,
    pub nearest_food_bearing: Option<f64>,
    pub nearest_body_clearance: Option<f64>,
    pub nearest_head_clearance: Option<f64>,
    pub pellet_saturated: bool,
    pub body_saturated: bool,
    /// Legacy v3 aggregates its full food reach here; v4 uses the inner ring.
    pub food_strength: [f64; MAX_SENSOR_BINS],
    /// Only v4 has a separate outer ring.
    pub far_food_strength: [f64; MAX_SENSOR_BINS],
    /// Exact pre-normalization sector minima, including conservative zero when saturated.
    pub body_clearance: [f64; MAX_SENSOR_BINS],
    pub body_rays: [SensorTraceRay; MAX_SENSOR_BINS],
    pub ray_headings: [f64; MAX_SENSOR_BINS],
    /// Exact sampler value; v3 caps at near reach, while v4 retains actual ray clearance.
    pub wall_clearance: [f64; MAX_SENSOR_BINS],
    pub head_clearance: [f64; MAX_SENSOR_BINS],
    pub head_count: [usize; MAX_SENSOR_BINS],
    pub head_ids: [u64; MAX_SENSOR_BINS],
    pub head_bearing_offset: [f64; MAX_SENSOR_BINS],
    pub head_length_ratio: [f64; MAX_SENSOR_BINS],
    pub relative_heading: [f64; MAX_SENSOR_BINS],
    pub relative_forward_speed: [f64; MAX_SENSOR_BINS],
    pub relative_side_speed: [f64; MAX_SENSOR_BINS],
    pub urgency: [f64; MAX_SENSOR_BINS],
    pub encounter_seconds: [Option<f64>; MAX_SENSOR_BINS],
}
impl Default for SensorTrace {
    fn default() -> Self {
        Self {
            valid: false,
            physical_id: 0,
            profile: SensorProfile::V3,
            bins: 0,
            near_radius: 0.0,
            far_radius: 0.0,
            body_length: 0,
            age_seconds: 0.0,
            food: 0.0,
            kills: 0,
            fitness: 0.0,
            size_factor: 0.0,
            boost_cost: 0.0,
            boost_headroom: 0.0,
            boost_eligible: None,
            assessment_points: None,
            life_points: None,
            generation_best_points: None,
            generation_elapsed_seconds: None,
            generation_seconds: 0.0,
            food_saturation: 0.0,
            boost_speed: 0.0,
            nearest_food_distance: None,
            nearest_food_bearing: None,
            nearest_body_clearance: None,
            nearest_head_clearance: None,
            pellet_saturated: false,
            body_saturated: false,
            food_strength: [0.0; MAX_SENSOR_BINS],
            far_food_strength: [0.0; MAX_SENSOR_BINS],
            body_clearance: [0.0; MAX_SENSOR_BINS],
            body_rays: [SensorTraceRay::NotSampled; MAX_SENSOR_BINS],
            ray_headings: [0.0; MAX_SENSOR_BINS],
            wall_clearance: [0.0; MAX_SENSOR_BINS],
            head_clearance: [0.0; MAX_SENSOR_BINS],
            head_count: [0; MAX_SENSOR_BINS],
            head_ids: [0; MAX_SENSOR_BINS],
            head_bearing_offset: [0.0; MAX_SENSOR_BINS],
            head_length_ratio: [0.0; MAX_SENSOR_BINS],
            relative_heading: [0.0; MAX_SENSOR_BINS],
            relative_forward_speed: [0.0; MAX_SENSOR_BINS],
            relative_side_speed: [0.0; MAX_SENSOR_BINS],
            urgency: [0.0; MAX_SENSOR_BINS],
            encounter_seconds: [None; MAX_SENSOR_BINS],
        }
    }
}
impl SensorTrace {
    /// Called only for the selected sample; all unused capacity has explicit absent values.
    pub fn begin(
        &mut self,
        snake: &SnakeState,
        profile: SensorProfile,
        bins: usize,
        radii: SensorRadii,
    ) {
        *self = Self {
            physical_id: snake.id,
            profile,
            bins,
            near_radius: radii.near,
            far_radius: radii.far,
            body_length: snake.body.len,
            age_seconds: snake.age_seconds,
            food: snake.food,
            kills: snake.kills,
            fitness: snake.fitness,
            ..Self::default()
        };
    }
}
