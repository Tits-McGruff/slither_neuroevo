//! Baseline v2 action selection from declared v4 observations only.
//! No world, snake, target ID, sensor probe or lifecycle RNG enters this API.
//! The caller stages the returned policy/behavior continuation with its delivery.

use super::baseline_control::BaselineLifeStage;
use super::rng::StatefulRng;
use super::sensor_contract::SensorProfile;
use super::sensor_layout::{SensorLayout, MAX_SENSOR_BINS};
use super::sensors::SensorConfig;
use super::state::BaselineStrategyState;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt::{Display, Formatter};

/// Versioned teacher formula and bounded policy state identity.
pub const BASELINE_V2_VERSION: u32 = 2;
const COMMITMENT_SECONDS: f64 = 0.2;
const SWITCH_IMPROVEMENT: f64 = 0.15;
const URGENT_URGENCY: f64 = 0.75; // TTC < 0.5 s in the declared two-second horizon.
const SAFE_CLEARANCE: f64 = 0.3; // V1's -0.4 bipolar veto, decoded to [0,1].

/// Strategy RNG policy; lifecycle/placement randomness is a separate stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineV2Mode {
    Deterministic,
    Stochastic,
}

/// Complete reset-only baseline-v2 setting extension; absence means frozen v1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineV2Settings {
    pub mode: BaselineV2Mode,
    pub control_interval_seconds: f64,
}
impl BaselineV2Settings {
    pub fn validate(self) -> Result<(), BaselineV2Error> {
        if !self.control_interval_seconds.is_finite()
            || !(0.008..=0.06).contains(&self.control_interval_seconds)
        {
            return Err(BaselineV2Error::Contract);
        }
        Ok(())
    }
    pub fn from_settings(
        settings: &[super::state::NormalizedSetting],
    ) -> Result<Option<Self>, BaselineV2Error> {
        use super::state::NormalizedSettingValue as Value;
        if settings.windows(2).any(|pair| pair[0].path >= pair[1].path) {
            return Err(BaselineV2Error::Contract);
        }
        let get = |path: &str| {
            settings
                .binary_search_by(|setting| setting.path.as_str().cmp(path))
                .ok()
                .map(|index| &settings[index].value)
        };
        let values = [
            get("baselineBots.controlVersion"),
            get("baselineBots.mode"),
            get("baselineBots.controlDt"),
        ];
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [Some(Value::Integer(2)), Some(Value::Integer(mode)), Some(Value::Float(interval))] =
            values
        else {
            return Err(BaselineV2Error::Contract);
        };
        if super::sensor_contract::SensorContractIdentity::from_settings(settings)
            .map_err(|_| BaselineV2Error::Contract)?
            .profile()
            == SensorProfile::V3
        {
            return Err(BaselineV2Error::Contract);
        }
        let mode = match mode {
            0 => BaselineV2Mode::Deterministic,
            1 => BaselineV2Mode::Stochastic,
            _ => return Err(BaselineV2Error::Contract),
        };
        let result = Self {
            mode,
            control_interval_seconds: *interval,
        };
        result.validate()?;
        Ok(Some(result))
    }
    /// Explicit cold configuration selection; never called to reinterpret imports.
    pub fn record_settings(
        self,
        settings: &mut Vec<super::state::NormalizedSetting>,
    ) -> Result<(), BaselineV2Error> {
        use super::state::{NormalizedSetting, NormalizedSettingValue as Value};
        self.validate()?;
        if super::sensor_contract::SensorContractIdentity::from_settings(settings)
            .map_err(|_| BaselineV2Error::Contract)?
            .profile()
            == SensorProfile::V3
        {
            return Err(BaselineV2Error::Contract);
        }
        for (path, value) in [
            ("baselineBots.controlVersion", Value::Integer(2)),
            (
                "baselineBots.mode",
                Value::Integer(if self.mode == BaselineV2Mode::Deterministic {
                    0
                } else {
                    1
                }),
            ),
            (
                "baselineBots.controlDt",
                Value::Float(self.control_interval_seconds),
            ),
        ] {
            if let Some(setting) = settings.iter_mut().find(|setting| setting.path == path) {
                setting.value = value;
            } else {
                settings.push(NormalizedSetting {
                    path: path.into(),
                    value,
                });
            }
        }
        settings.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        Ok(())
    }
}

/// Physical-life policy scratch, bounded independently of life length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineV2State {
    pub version: u32,
    /// World heading, not a stale relative sector or privileged target identity.
    pub committed_heading: Option<f64>,
    pub commitment_remaining_seconds: f64,
    pub wander_angle: f64,
    pub wander_remaining_seconds: f64,
    pub strategy: BaselineStrategyState,
}
impl Default for BaselineV2State {
    fn default() -> Self {
        Self {
            version: BASELINE_V2_VERSION,
            committed_heading: None,
            commitment_remaining_seconds: 0.0,
            wander_angle: 0.0,
            wander_remaining_seconds: 0.0,
            strategy: BaselineStrategyState::Roam,
        }
    }
}
impl BaselineV2State {
    pub(crate) fn validate(self) -> Result<(), BaselineV2Error> {
        if self.version != BASELINE_V2_VERSION
            || self
                .committed_heading
                .is_some_and(|v| !v.is_finite() || !(-PI..=PI).contains(&v))
            || !self.commitment_remaining_seconds.is_finite()
            || !(0.0..=COMMITMENT_SECONDS).contains(&self.commitment_remaining_seconds)
            || (self.committed_heading.is_none() && self.commitment_remaining_seconds != 0.0)
            || !self.wander_angle.is_finite()
            || !(-0.3..=0.3).contains(&self.wander_angle)
            || !self.wander_remaining_seconds.is_finite()
            || !(0.0..=2.0).contains(&self.wander_remaining_seconds)
        {
            return Err(BaselineV2Error::State);
        }
        Ok(())
    }
}

/// One staged action plus explicit behavior RNG continuation; nothing is published.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineV2Action {
    pub turn: f32,
    pub boost: bool,
    pub sector: usize,
    pub life_stage: BaselineLifeStage,
    pub normalized_score: f64,
    pub unsafe_route: bool,
    /// Exact tactical classification used by boost eligibility and capture strata.
    pub escape_or_pursuit: bool,
    pub urgent_sectors: u32,
    pub next_state: BaselineV2State,
    pub next_behavior_rng_word: u32,
    pub uniform_draws: u8,
}

/// Cold-validated structural/environment contract. Hot evaluation allocates nothing.
#[derive(Clone, Debug)]
pub struct BaselineV2Policy {
    profile: SensorProfile,
    bins: usize,
    environment: SensorConfig,
    mode: BaselineV2Mode,
}
impl BaselineV2Policy {
    pub fn new(
        layout: &SensorLayout,
        environment: SensorConfig,
        mode: BaselineV2Mode,
    ) -> Result<Self, BaselineV2Error> {
        layout
            .contract_identity()
            .map_err(|_| BaselineV2Error::Contract)?;
        if layout.profile == SensorProfile::V3 || environment.bins != layout.bins {
            return Err(BaselineV2Error::Contract);
        }
        environment
            .validate()
            .map_err(|_| BaselineV2Error::Environment)?;
        Ok(Self {
            profile: layout.profile,
            bins: layout.bins,
            environment,
            mode,
        })
    }

    /// Elapsed time is the actual interval between due boundaries. Capture/probes
    /// cannot create an action boundary or modify either supplied continuation.
    pub fn evaluate(
        &self,
        observation: &[f32],
        source: BaselineV2State,
        behavior_rng_word: u32,
        elapsed_seconds: f64,
    ) -> Result<BaselineV2Action, BaselineV2Error> {
        source.validate()?;
        if !elapsed_seconds.is_finite() || !(0.0..=1.0).contains(&elapsed_seconds) {
            return Err(BaselineV2Error::Elapsed);
        }
        let mut rng = StatefulRng::from_uniform_state(behavior_rng_word)
            .map_err(|_| BaselineV2Error::BehaviorRng)?;
        let sample = Observation::new(observation, self.profile, self.bins)?;
        let mut state = if observation[21] == 1.0 {
            BaselineV2State::default()
        } else {
            source
        };
        state.commitment_remaining_seconds =
            (state.commitment_remaining_seconds - elapsed_seconds).max(0.0);
        state.wander_remaining_seconds =
            (state.wander_remaining_seconds - elapsed_seconds).max(0.0);
        let stage = self.life_stage(observation[2]);
        let heading = f64::from(observation[0]).atan2(f64::from(observation[1]));
        let size = unit(observation[2]);
        let near = (self.environment.near_radius_base + self.environment.near_radius_scale * size)
            .clamp(
                self.environment.near_radius_minimum,
                self.environment.near_radius_maximum,
            );
        let mut biases = [0.0f64; MAX_SENSOR_BINS];
        let mut pursuit = 0u32;
        let mut urgent = 0u32;
        let mut best_food = 0.0f64;
        // One bounded sector pass. Every target quantity below is decoded from
        // the observation; even intercept distance is an approximation from
        // visible clearance, not an undeclared target center/radius.
        for bin in 0..self.bins {
            let s = sample.sector(bin);
            best_food = best_food.max(s.near_food.max(0.25 * s.far_food));
            if s.urgency > URGENT_URGENCY {
                urgent |= 1 << bin;
            }
            if self.profile == SensorProfile::Motion && s.present {
                let bearing = angle(bin, self.bins) + s.bearing_residual * PI / self.bins as f64;
                if s.relative_length > 0.0 {
                    add_bias(
                        &mut biases,
                        self.bins,
                        wrap(bearing + PI),
                        0.6 * (1.0 - s.head_clearance) * (1.0 + s.relative_length),
                    );
                } else if s.relative_length < 0.0 && stage != BaselineLifeStage::Small {
                    let lead_scale = self.environment.snake_boost_speed / near;
                    let lead = (s.head_clearance * bearing.sin() + s.velocity_lateral * lead_scale)
                        .atan2(s.head_clearance * bearing.cos() + s.velocity_forward * lead_scale);
                    let target = sector(lead, self.bins);
                    pursuit |= 1 << target;
                    add_bias(
                        &mut biases,
                        self.bins,
                        lead,
                        if stage == BaselineLifeStage::Medium {
                            0.8
                        } else {
                            0.4
                        },
                    );
                }
            }
        }
        let mut draws = 0;
        if self.mode == BaselineV2Mode::Deterministic {
            // A stochastic continuation cannot introduce wandering when mode changes.
            state.wander_angle = 0.0;
            if state.wander_remaining_seconds == 0.0 {
                state.wander_remaining_seconds = 1.3;
            }
        } else if best_food <= 0.05 && urgent == 0 && state.wander_remaining_seconds == 0.0 {
            state.wander_angle = (rng.next_f64() - 0.5) * 0.6;
            state.wander_remaining_seconds = 0.6 + rng.next_f64() * 1.4;
            draws = 2;
            state.committed_heading = None;
            state.commitment_remaining_seconds = 0.0;
        }
        let (food_weight, clearance_weight, food_cap) = match stage {
            BaselineLifeStage::Small => (0.5, 1.8, 0.4),
            BaselineLifeStage::Medium => (0.8, 1.2, 0.6),
            BaselineLifeStage::Large => (0.4, 1.5, 0.4),
        };
        let mut scores = [-1.0f64; MAX_SENSOR_BINS];
        let mut safe = [false; MAX_SENSOR_BINS];
        let mut clearances = [0.0f64; MAX_SENSOR_BINS];
        let mut best = 0;
        let mut fallback = 0;
        let mut fallback_score = f64::NEG_INFINITY;
        let mut any_safe = false;
        for bin in 0..self.bins {
            let s = sample.sector(bin);
            let clearance = s
                .body_sector
                .min(s.body_ray)
                .min(s.wall_ray)
                .min(s.head_clearance);
            clearances[bin] = clearance;
            safe[bin] = clearance >= SAFE_CLEARANCE && s.urgency <= URGENT_URGENCY;
            any_safe |= safe[bin];
            // Food behind a capsule/wall or urgent crossing is discounted. Motion
            // distinguishes closing/head-on and crossing pressure; Basic has no
            // representative/velocity/urgency/density data and omits those tactics.
            let bearing = angle(bin, self.bins) + s.bearing_residual * PI / self.bins as f64;
            let closing = (-(s.velocity_forward * bearing.cos()
                + s.velocity_lateral * bearing.sin()))
            .max(0.0);
            let head_on = if s.present && s.heading_cos < -0.5 {
                closing
            } else {
                0.0
            };
            let crossing = if s.present {
                closing * s.heading_sin.abs()
            } else {
                0.0
            };
            let food =
                (s.near_food + 0.25 * s.far_food).min(food_cap) * clearance * (1.0 - s.urgency);
            scores[bin] =
                ((food_weight * food + clearance_weight * (2.0 * clearance - 1.0) + biases[bin]
                    - 2.5 * s.urgency
                    - 0.5 * head_on
                    - 0.35 * crossing
                    - if stage == BaselineLifeStage::Large {
                        0.2 * s.density
                    } else {
                        0.4 * s.density
                    })
                    / 4.0)
                    .clamp(-1.0, 1.0);
            let escape_clearance = clearance - s.urgency;
            if preferred(escape_clearance, bin, fallback_score, fallback, self.bins) {
                fallback = bin;
                fallback_score = escape_clearance;
            }
            if (safe[bin] && !safe[best])
                || (safe[bin] == safe[best]
                    && preferred(scores[bin], bin, scores[best], best, self.bins))
            {
                best = bin;
            }
        }
        if !any_safe {
            best = fallback;
        }
        let mut held_heading = None;
        if let Some(committed) = state.committed_heading {
            let old = sector(wrap(committed - heading), self.bins);
            if safe[old]
                && (state.commitment_remaining_seconds > 0.0
                    || scores[best] - scores[old] < SWITCH_IMPROVEMENT)
            {
                best = old;
                held_heading = Some(committed);
            }
        }
        let selected = sample.sector(best);
        let roam = best_food <= 0.05 && urgent == 0 && pursuit & (1 << best) == 0;
        let wander = state
            .wander_angle
            .clamp(-0.8 * PI / self.bins as f64, 0.8 * PI / self.bins as f64);
        let target_angle = held_heading.map_or_else(
            || wrap(angle(best, self.bins) + if roam { wander } else { 0.0 }),
            |committed| wrap(committed - heading),
        );
        if held_heading.is_none() {
            state.committed_heading = Some(wrap(heading + target_angle));
            state.commitment_remaining_seconds = if safe[best] { COMMITMENT_SECONDS } else { 0.0 };
        }
        // Half a second of point budget and declared boost-speed route clearance.
        // Saturation prevents boosting into unknown geometry. Zero cost's +1
        // budget sentinel is valid, but actual eligibility is still authoritative.
        // Compare in the transmitted Float32 domain, including the exact half-second boundary.
        let enough_budget = observation[6] >= -0.6f32; // bipolar(0.5/(0.5+2)).
        let escape_or_pursuit = urgent != 0 || pursuit & (1 << best) != 0;
        let boost = escape_or_pursuit
            && observation[8] == 1.0
            && enough_budget
            && observation[23] == -1.0
            && safe[best]
            && selected.urgency == 0.0
            && clearances[best] * near > self.environment.snake_boost_speed * 0.5
            && target_angle.abs() < PI / 4.0;
        state.strategy = if urgent != 0 || !safe[best] {
            BaselineStrategyState::Avoid
        } else if boost {
            BaselineStrategyState::Boost
        } else if !roam {
            BaselineStrategyState::Seek
        } else {
            BaselineStrategyState::Roam
        };
        Ok(BaselineV2Action {
            turn: (target_angle / FRAC_PI_2).clamp(-1.0, 1.0) as f32,
            boost,
            sector: best,
            life_stage: stage,
            normalized_score: scores[best],
            unsafe_route: !safe[best],
            escape_or_pursuit,
            urgent_sectors: urgent,
            next_state: state,
            next_behavior_rng_word: rng.uniform_state(),
            uniform_draws: draws,
        })
    }

    fn life_stage(&self, size: f32) -> BaselineLifeStage {
        let threshold = |length: usize| {
            let d = length.saturating_sub(self.environment.snake_start_length) as f64;
            (2.0 * d / (d + 40.0) - 1.0) as f32
        };
        if self.environment.snake_start_length >= 80 || size >= threshold(80) {
            BaselineLifeStage::Large
        } else if self.environment.snake_start_length >= 25 || size >= threshold(25) {
            BaselineLifeStage::Medium
        } else {
            BaselineLifeStage::Small
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Sector {
    near_food: f64,
    far_food: f64,
    body_sector: f64,
    body_ray: f64,
    wall_ray: f64,
    head_clearance: f64,
    present: bool,
    bearing_residual: f64,
    relative_length: f64,
    heading_sin: f64,
    heading_cos: f64,
    velocity_forward: f64,
    velocity_lateral: f64,
    urgency: f64,
    density: f64,
}
struct Observation<'a> {
    values: &'a [f32],
    profile: SensorProfile,
    bins: usize,
}
impl<'a> Observation<'a> {
    fn new(
        values: &'a [f32],
        profile: SensorProfile,
        bins: usize,
    ) -> Result<Self, BaselineV2Error> {
        let channels = if profile == SensorProfile::Motion {
            15
        } else {
            6
        };
        if values.len() != 24 + channels * bins {
            return Err(BaselineV2Error::Observation);
        }
        if values
            .iter()
            .any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v))
            || values[19] < 0.0
            || [8, 9, 10, 21, 22, 23]
                .into_iter()
                .any(|i| !matches!(values[i], -1.0 | 1.0))
            || (f64::from(values[0]).hypot(f64::from(values[1])) - 1.0).abs() > 1e-5
        {
            return Err(BaselineV2Error::Observation);
        }
        let result = Self {
            values,
            profile,
            bins,
        };
        if profile == SensorProfile::Motion {
            for bin in 0..bins {
                if !matches!(values[24 + 6 * bins + bin], -1.0 | 1.0)
                    || (13..15).any(|channel| values[24 + channel * bins + bin] < 0.0)
                {
                    return Err(BaselineV2Error::Observation);
                }
                let s = result.sector(bin);
                if (!s.present && (7..13).any(|c| values[24 + c * bins + bin] != 0.0))
                    || (s.present && (s.heading_sin.hypot(s.heading_cos) - 1.0).abs() > 1e-5)
                {
                    return Err(BaselineV2Error::Observation);
                }
            }
        }
        Ok(result)
    }
    fn sector(&self, bin: usize) -> Sector {
        let get = |c| self.values[24 + c * self.bins + bin];
        let mut s = Sector {
            near_food: unit(get(0)),
            far_food: unit(get(1)),
            body_sector: unit(get(2)),
            body_ray: unit(get(3)),
            wall_ray: unit(get(4)),
            head_clearance: unit(get(5)),
            ..Sector::default()
        };
        if self.profile == SensorProfile::Motion {
            s.present = get(6) == 1.0;
            s.bearing_residual = f64::from(get(7));
            s.relative_length = f64::from(get(8));
            s.heading_sin = f64::from(get(9));
            s.heading_cos = f64::from(get(10));
            s.velocity_forward = f64::from(get(11));
            s.velocity_lateral = f64::from(get(12));
            s.urgency = f64::from(get(13));
            s.density = f64::from(get(14));
        }
        s
    }
}
fn unit(value: f32) -> f64 {
    (f64::from(value) + 1.0) * 0.5
}
fn wrap(value: f64) -> f64 {
    (value + PI).rem_euclid(TAU) - PI
}
fn angle(bin: usize, bins: usize) -> f64 {
    -PI + bin as f64 * TAU / bins as f64
}
fn sector(bearing: f64, bins: usize) -> usize {
    (((wrap(bearing) + PI) * bins as f64 / TAU + 0.5).floor() as usize) % bins
}
fn preferred(score: f64, bin: usize, previous: f64, old: usize, bins: usize) -> bool {
    // Integer half-sector distances make symmetric angular ties exact even for
    // odd bin counts, without depending on rounded +/- pi arithmetic.
    let distance = |index: usize| (2 * index).abs_diff(bins);
    score > previous
        || (score == previous
            && (distance(bin) < distance(old) || (distance(bin) == distance(old) && bin < old)))
}

fn add_bias(biases: &mut [f64; MAX_SENSOR_BINS], bins: usize, bearing: f64, strength: f64) {
    let center = sector(bearing, bins);
    for bin in [(center + bins - 1) % bins, center, (center + 1) % bins] {
        biases[bin] += strength
            * (1.0 - wrap(angle(bin, bins) - bearing).abs() / (TAU / bins as f64)).max(0.0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineV2Error {
    Contract,
    Environment,
    State,
    Elapsed,
    BehaviorRng,
    Observation,
}
impl Display for BaselineV2Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid baseline v2: {self:?}")
    }
}
impl std::error::Error for BaselineV2Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rng::baseline_v2_behavior_word;
    fn policy(profile: SensorProfile, bins: usize, mode: BaselineV2Mode) -> BaselineV2Policy {
        BaselineV2Policy::new(
            &SensorLayout::for_profile(profile, bins).unwrap(),
            SensorConfig {
                bins,
                ..SensorConfig::default()
            },
            mode,
        )
        .unwrap()
    }
    fn sample(profile: SensorProfile, bins: usize) -> Vec<f32> {
        let mut row = vec![
            0.0;
            24 + if profile == SensorProfile::Motion {
                15 * bins
            } else {
                6 * bins
            }
        ];
        row[1] = 1.0;
        row[2] = -1.0;
        row[6] = -1.0;
        for i in [8, 9, 10, 21, 22, 23] {
            row[i] = -1.0;
        }
        for c in 0..6 {
            row[24 + c * bins..24 + (c + 1) * bins].fill(if c < 2 { -1.0 } else { 1.0 });
        }
        if profile == SensorProfile::Motion {
            row[24 + 6 * bins..24 + 7 * bins].fill(-1.0);
        }
        row
    }
    fn medium(row: &mut [f32]) {
        row[2] = (2.0 * 45.0 / 85.0 - 1.0) as f32;
    }
    fn put(row: &mut [f32], bins: usize, channel: usize, bin: usize, value: f32) {
        row[24 + channel * bins + bin] = value;
    }
    fn evaluate(p: &BaselineV2Policy, row: &[f32], state: BaselineV2State) -> BaselineV2Action {
        p.evaluate(row, state, 12345, 0.1).unwrap()
    }
    #[test]
    fn complete_v2_setting_extension_requires_v4_and_rejects_partial_or_revised_identity() {
        use crate::engine::sensor_contract::SensorContractIdentity;
        use crate::engine::state::NormalizedSettingValue;
        let mut settings = crate::engine::step_config::production_default_settings(12, 2);
        assert_eq!(BaselineV2Settings::from_settings(&settings).unwrap(), None);
        let spec = BaselineV2Settings {
            mode: BaselineV2Mode::Stochastic,
            control_interval_seconds: 0.06,
        };
        let before = settings.clone();
        assert!(spec.record_settings(&mut settings).is_err());
        assert_eq!(settings, before);
        SensorContractIdentity::new(SensorProfile::Motion, 16)
            .unwrap()
            .record_v4_settings(&mut settings)
            .unwrap();
        spec.record_settings(&mut settings).unwrap();
        assert_eq!(
            BaselineV2Settings::from_settings(&settings).unwrap(),
            Some(spec)
        );
        for path in [
            "baselineBots.controlVersion",
            "baselineBots.mode",
            "baselineBots.controlDt",
        ] {
            let mut bad = settings.clone();
            bad.retain(|v| v.path != path);
            assert!(BaselineV2Settings::from_settings(&bad).is_err());
        }
        for (path, value) in [
            (
                "baselineBots.controlVersion",
                NormalizedSettingValue::Integer(3),
            ),
            ("baselineBots.mode", NormalizedSettingValue::Integer(2)),
            (
                "baselineBots.controlDt",
                NormalizedSettingValue::Float(0.061),
            ),
        ] {
            let mut bad = settings.clone();
            bad.iter_mut().find(|v| v.path == path).unwrap().value = value;
            assert!(BaselineV2Settings::from_settings(&bad).is_err());
        }
    }

    #[test]
    fn deterministic_ties_use_exact_smallest_turn_then_index_and_no_draws() {
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for bins in 8..=32 {
                let p = policy(profile, bins, BaselineV2Mode::Deterministic);
                let result = evaluate(&p, &sample(profile, bins), BaselineV2State::default());
                assert_eq!(result.sector, bins / 2);
                assert_eq!(result.next_behavior_rng_word, 12345);
                assert_eq!(result.uniform_draws, 0);
                assert!(!result.boost && !result.unsafe_route);
            }
        }
    }
    #[test]
    fn life_stage_thresholds_use_declared_float32_tactical_size() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        for (length, expected) in [
            (5, BaselineLifeStage::Small),
            (24, BaselineLifeStage::Small),
            (25, BaselineLifeStage::Medium),
            (79, BaselineLifeStage::Medium),
            (80, BaselineLifeStage::Large),
        ] {
            let mut row = sample(SensorProfile::Motion, 16);
            let d = (length - 5) as f64;
            row[2] = (2.0 * d / (d + 40.0) - 1.0) as f32;
            assert_eq!(
                evaluate(&p, &row, BaselineV2State::default()).life_stage,
                expected
            );
        }
    }
    #[test]
    fn capsule_food_trap_is_vetoed_and_a_safe_food_opening_is_selected() {
        let p = policy(SensorProfile::Basic, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Basic, 16);
        put(&mut row, 16, 0, 8, 1.0);
        put(&mut row, 16, 3, 8, -1.0);
        let a = evaluate(&p, &row, BaselineV2State::default());
        assert_eq!(a.sector, 7);
        assert!(!a.boost);
        put(&mut row, 16, 0, 9, 1.0);
        assert_eq!(evaluate(&p, &row, BaselineV2State::default()).sector, 9);
    }
    #[test]
    fn closing_head_on_risk_differs_from_receding_and_basic_omits_motion() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        row[24 + 3 * 16..24 + 4 * 16].fill(0.2);
        put(&mut row, 16, 5, 8, 0.2);
        put(&mut row, 16, 6, 8, 1.0);
        put(&mut row, 16, 10, 8, -1.0);
        put(&mut row, 16, 11, 8, -0.8);
        assert_eq!(evaluate(&p, &row, BaselineV2State::default()).sector, 7);
        put(&mut row, 16, 11, 8, 0.8);
        assert_eq!(evaluate(&p, &row, BaselineV2State::default()).sector, 8);
        let basic = policy(SensorProfile::Basic, 16, BaselineV2Mode::Deterministic);
        assert_eq!(
            evaluate(&basic, &row[..120], BaselineV2State::default()).sector,
            8
        );
    }
    #[test]
    fn commitment_holds_world_heading_and_unsafe_or_new_life_breaks_it() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        row[0] = 0.2f64.sin() as f32;
        row[1] = 0.2f64.cos() as f32;
        let state = BaselineV2State {
            committed_heading: Some(0.0),
            commitment_remaining_seconds: 0.2,
            ..BaselineV2State::default()
        };
        let held = evaluate(&p, &row, state);
        assert!((held.turn as f64 + 0.2 / FRAC_PI_2).abs() < 1e-6);
        assert_eq!(held.next_state.committed_heading, Some(0.0));
        put(&mut row, 16, 13, held.sector, 0.9);
        assert_ne!(evaluate(&p, &row, state).sector, held.sector);
        put(&mut row, 16, 13, held.sector, 0.0);
        row[21] = 1.0;
        assert_eq!(evaluate(&p, &row, state).sector, 8);
    }
    #[test]
    fn safe_commitment_then_normalized_improvement_controls_switching() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        medium(&mut row);
        put(&mut row, 16, 6, 9, 1.0);
        put(&mut row, 16, 8, 9, -0.5);
        put(&mut row, 16, 10, 9, 1.0);
        let state = BaselineV2State {
            committed_heading: Some(angle(7, 16)),
            commitment_remaining_seconds: 0.2,
            ..BaselineV2State::default()
        };
        let held = evaluate(&p, &row, state);
        assert_eq!(held.sector, 7);
        let changed = evaluate(&p, &row, held.next_state);
        assert_eq!(changed.sector, 9);
        let weak = evaluate(
            &p,
            &sample(SensorProfile::Motion, 16),
            BaselineV2State {
                commitment_remaining_seconds: 0.0,
                ..state
            },
        );
        assert_eq!(weak.sector, 7);
    }
    #[test]
    fn boost_requires_economy_eligibility_safe_route_and_complete_geometry() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        medium(&mut row);
        put(&mut row, 16, 6, 8, 1.0);
        put(&mut row, 16, 8, 8, -0.5);
        put(&mut row, 16, 10, 8, 1.0);
        row[8] = 1.0;
        row[6] = 1.0;
        assert!(evaluate(&p, &row, BaselineV2State::default()).boost);
        for (index, value) in [(8, -1.0), (6, -1.0), (23, 1.0)] {
            let mut invalid = row.clone();
            invalid[index] = value;
            assert!(!evaluate(&p, &invalid, BaselineV2State::default()).boost);
        }
        put(&mut row, 16, 3, 8, -1.0);
        assert!(!evaluate(&p, &row, BaselineV2State::default()).boost);
    }
    #[test]
    fn half_second_budget_boundary_uses_the_actual_float32_encoding() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        medium(&mut row);
        put(&mut row, 16, 6, 8, 1.0);
        put(&mut row, 16, 8, 8, -0.5);
        put(&mut row, 16, 10, 8, 1.0);
        row[8] = 1.0;
        row[6] = -0.6;
        assert!(evaluate(&p, &row, BaselineV2State::default()).boost);
        row[6] = -0.600001;
        assert!(!evaluate(&p, &row, BaselineV2State::default()).boost);
        row[19] = -0.1;
        assert_eq!(
            p.evaluate(&row, BaselineV2State::default(), 1, 0.1),
            Err(BaselineV2Error::Observation)
        );
    }
    #[test]
    fn stochastic_stream_is_isolated_reproducible_and_validation_is_atomic() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Stochastic);
        let row = sample(SensorProfile::Motion, 16);
        let source = BaselineV2State::default();
        let seed = baseline_v2_behavior_word(42, 1, 9, 3, true);
        let a = p.evaluate(&row, source, seed, 0.1).unwrap();
        assert_eq!(a, p.evaluate(&row, source, seed, 0.1).unwrap());
        assert_eq!(a.uniform_draws, 2);
        assert_ne!(a.next_behavior_rng_word, seed);
        assert_ne!(
            seed,
            baseline_v2_behavior_word(42, 1, (1u64 << 32) + 9, 3, true)
        );
        assert_ne!(seed, baseline_v2_behavior_word(42, 1, 9, 4, true));
        assert_eq!(
            baseline_v2_behavior_word(42, 1, 9, 3, false),
            baseline_v2_behavior_word(42, 1, 123, 3, false)
        );
        let mut malformed = row.clone();
        malformed[30] = f32::NAN;
        assert_eq!(
            p.evaluate(&malformed, source, seed, 0.1),
            Err(BaselineV2Error::Observation)
        );
        assert_eq!(source, BaselineV2State::default());
        assert_eq!(p.evaluate(&row, source, seed, 0.1).unwrap(), a);
        let deterministic = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let d = deterministic
            .evaluate(&row, a.next_state, a.next_behavior_rng_word, 0.1)
            .unwrap();
        assert_eq!(d.next_behavior_rng_word, a.next_behavior_rng_word);
        assert_eq!(d.next_state.wander_angle, 0.0);
    }
    #[test]
    fn unknown_routes_do_not_override_known_urgent_crossing_in_emergency_fallback() {
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        row[24 + 2 * 16..24 + 4 * 16].fill(-1.0);
        row[23] = 1.0;
        put(&mut row, 16, 13, 8, 0.99);
        let a = evaluate(&p, &row, BaselineV2State::default());
        assert_eq!(a.sector, 7);
        assert!(a.unsafe_route && !a.boost);
        assert_eq!(a.urgent_sectors & (1 << a.sector), 0);
    }
    #[test]
    fn structural_and_absence_malformed_samples_are_rejected() {
        assert!(BaselineV2Policy::new(
            &SensorLayout::new(16).unwrap(),
            SensorConfig::default(),
            BaselineV2Mode::Deterministic
        )
        .is_err());
        let p = policy(SensorProfile::Motion, 16, BaselineV2Mode::Deterministic);
        let mut row = sample(SensorProfile::Motion, 16);
        put(&mut row, 16, 11, 8, 0.5);
        assert_eq!(
            p.evaluate(&row, BaselineV2State::default(), 1, 0.1),
            Err(BaselineV2Error::Observation)
        );
    }
}
