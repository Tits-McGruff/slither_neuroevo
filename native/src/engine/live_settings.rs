//! Atomic validation and preparation of live normalized-setting replacements.

use super::state::{
    normalized_config_hash, NormalizedEngineConfig, NormalizedSettingValue, StateError,
};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Maximum settings carried by one existing Protocol 2 request.
pub const MAXIMUM_LIVE_SETTING_UPDATES: usize = 64;

/// One numeric Protocol 2 setting update after the thin bridge has bounded its path.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveSettingUpdate {
    /// Canonical normalized-setting path.
    pub path: String,
    /// Finite numeric wire value; booleans use exactly zero or one.
    pub value: f64,
}

/// Complete small replacement prepared without changing authoritative state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreparedLiveSettings {
    pub(crate) config: NormalizedEngineConfig,
    pub(crate) config_revision: u64,
    pub(crate) config_hash: String,
}

/// Prepare an all-or-nothing live-settings update against the current config.
pub(crate) fn prepare_live_settings(
    current: &NormalizedEngineConfig,
    current_revision: u64,
    updates: &[LiveSettingUpdate],
) -> Result<PreparedLiveSettings, LiveSettingsError> {
    if updates.is_empty() || updates.len() > MAXIMUM_LIVE_SETTING_UPDATES {
        return Err(LiveSettingsError::InvalidCount);
    }
    let mut seen = BTreeSet::new();
    let mut config = current.clone();
    for update in updates {
        if !seen.insert(update.path.as_str()) {
            return Err(LiveSettingsError::DuplicatePath(update.path.clone()));
        }
        let rule = live_rule(&update.path)
            .ok_or_else(|| LiveSettingsError::ResetOnlyOrUnknown(update.path.clone()))?;
        if !update.value.is_finite() || !(rule.minimum..=rule.maximum).contains(&update.value) {
            return Err(LiveSettingsError::OutOfRange(update.path.clone()));
        }
        let setting = config
            .settings
            .binary_search_by(|setting| setting.path.as_str().cmp(update.path.as_str()))
            .ok()
            .and_then(|index| config.settings.get_mut(index))
            .ok_or_else(|| LiveSettingsError::MissingPath(update.path.clone()))?;
        setting.value = match (&setting.value, rule.kind) {
            (NormalizedSettingValue::Bool(_), LiveValueKind::Boolean)
                if update.value == 0.0 || update.value == 1.0 =>
            {
                NormalizedSettingValue::Bool(update.value == 1.0)
            }
            (NormalizedSettingValue::Integer(_), LiveValueKind::Integer)
                if update.value.fract() == 0.0
                    && update.value >= i64::MIN as f64
                    && update.value <= i64::MAX as f64 =>
            {
                NormalizedSettingValue::Integer(update.value as i64)
            }
            (NormalizedSettingValue::Float(_), LiveValueKind::Float) => {
                NormalizedSettingValue::Float(update.value)
            }
            _ => return Err(LiveSettingsError::WrongType(update.path.clone())),
        };
        if update.path == "simSpeed" {
            config.requested_sim_speed = update.value;
        }
    }
    let config_revision = current_revision
        .checked_add(1)
        .ok_or(LiveSettingsError::RevisionExhausted)?;
    let config_hash = normalized_config_hash(&config).map_err(LiveSettingsError::State)?;
    Ok(PreparedLiveSettings {
        config,
        config_revision,
        config_hash,
    })
}

#[derive(Clone, Copy)]
enum LiveValueKind {
    Boolean,
    Integer,
    Float,
}

#[derive(Clone, Copy)]
struct LiveRule {
    kind: LiveValueKind,
    minimum: f64,
    maximum: f64,
}

const fn float(minimum: f64, maximum: f64) -> LiveRule {
    LiveRule {
        kind: LiveValueKind::Float,
        minimum,
        maximum,
    }
}

const fn integer(minimum: f64, maximum: f64) -> LiveRule {
    LiveRule {
        kind: LiveValueKind::Integer,
        minimum,
        maximum,
    }
}

const fn boolean() -> LiveRule {
    LiveRule {
        kind: LiveValueKind::Boolean,
        minimum: 0.0,
        maximum: 1.0,
    }
}

/// Keep the Rust authority's accepted live subset explicit and independently ranged.
fn live_rule(path: &str) -> Option<LiveRule> {
    Some(match path {
        "simSpeed" => float(0.1, 12.0),
        "foodSpawn.edgeFalloffEnabled" | "sense.debug" => boolean(),
        "foodSpawn.edgeFadeStart" => float(0.05, 0.85),
        "foodSpawn.edgeFadePower" => float(1.0, 6.0),
        "foodSpawn.filamentPower" => float(1.5, 8.0),
        "foodSpawn.warpScale" => float(0.0, 0.2),
        "foodSpawn.warpFreq" => float(0.0003, 0.003),
        "foodSpawn.freqLarge" => float(0.001, 0.006),
        "foodSpawn.freqMedium" => float(0.0015, 0.01),
        "foodSpawn.freqSmall" => float(0.0025, 0.02),
        "foodSpawn.dustStrength" => float(0.0, 1.0),
        "sense.rNearBase" => float(200.0, 900.0),
        "sense.rNearScale" => float(0.0, 600.0),
        "sense.rNearMin" => float(150.0, 900.0),
        "sense.rNearMax" => float(200.0, 1_200.0),
        "sense.rFarBase" => float(400.0, 2_000.0),
        "sense.rFarScale" => float(0.0, 1_200.0),
        "sense.rFarMin" => float(400.0, 2_200.0),
        "sense.rFarMax" => float(600.0, 3_000.0),
        "sense.foodKBase" => float(0.5, 12.0),
        "sense.maxPelletChecks" => integer(100.0, 3_000.0),
        "sense.maxSegmentChecks" => integer(200.0, 4_000.0),
        "baselineBots.respawnDelay" => float(0.5, 60.0),
        "snakeSizeSpeedPenalty" => float(0.0, 0.7),
        "snakeBoostSizePenalty" => float(0.0, 0.95),
        "boost.minPointsToBoost" => float(0.0, 60.0),
        "boost.pointsCostPerSecond" => float(0.0, 80.0),
        "boost.pointsCostSizeFactor" => float(0.0, 4.0),
        "boost.lenLossPerPoint" => float(0.0, 2.0),
        "boost.pelletValueFactor" => float(0.0, 1.5),
        "boost.pelletJitter" => float(0.0, 80.0),
        "collision.substepMaxDt" => float(0.006, 0.05),
        "collision.skipSegments" => integer(0.0, 30.0),
        "collision.hitScale" => float(0.45, 1.2),
        "collision.neighborRange" => integer(1.0, 3.0),
        "observer.focusRecheckSeconds" => float(0.1, 6.0),
        "observer.focusSwitchMargin" => float(1.0, 1.6),
        "observer.earlyEndMinSeconds" => float(0.0, 50.0),
        "observer.earlyEndAliveThreshold" => integer(1.0, 25.0),
        "observer.overviewPadding" => float(1.0, 1.8),
        "observer.zoomLerpFollow" | "observer.zoomLerpOverview" => float(0.0, 0.4),
        "observer.overviewExtraWorldMargin" => float(0.0, 1_200.0),
        "reward.pointsPerFood" => float(0.0, 20.0),
        "reward.pointsPerKill" | "reward.fitnessKill" => float(0.0, 400.0),
        "reward.pointsPerSecondAlive" | "reward.fitnessSurvivalPerSecond" => float(0.0, 10.0),
        "reward.fitnessFood" => float(0.0, 80.0),
        "reward.fitnessLengthPerSegment" => float(0.0, 100.0),
        "reward.fitnessPointsNorm" => float(0.0, 300.0),
        "reward.fitnessTopPointsBonus" => float(0.0, 600.0),
        "brain.controlDt" => float(0.008, 0.06),
        _ => return None,
    })
}

/// Recoverable rejection of one complete live-settings batch.
#[derive(Debug)]
pub enum LiveSettingsError {
    InvalidCount,
    DuplicatePath(String),
    ResetOnlyOrUnknown(String),
    MissingPath(String),
    OutOfRange(String),
    WrongType(String),
    RevisionExhausted,
    State(StateError),
}

impl Display for LiveSettingsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCount => write!(formatter, "live settings require 1 to 64 updates"),
            Self::DuplicatePath(path) => write!(formatter, "duplicate live setting {path}"),
            Self::ResetOnlyOrUnknown(path) => {
                write!(formatter, "setting requires reset or is unknown: {path}")
            }
            Self::MissingPath(path) => {
                write!(formatter, "normalized config omits live setting {path}")
            }
            Self::OutOfRange(path) => write!(formatter, "live setting is out of range: {path}"),
            Self::WrongType(path) => write!(
                formatter,
                "live setting has the wrong normalized type: {path}"
            ),
            Self::RevisionExhausted => write!(formatter, "config revision is exhausted"),
            Self::State(error) => write!(formatter, "live config hash failed: {error}"),
        }
    }
}

impl Error for LiveSettingsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::step_config::fixture_default_settings;

    fn config() -> NormalizedEngineConfig {
        let settings = fixture_default_settings(4, 1);
        NormalizedEngineConfig {
            version: 1,
            settings_schema_sha256: crate::engine::state::normalized_settings_schema_hash(
                &settings,
            )
            .expect("fixture schema"),
            settings,
            graph_architecture_key: "fixture".into(),
            fixed_step_seconds: 1.0 / 60.0,
            requested_sim_speed: 1.0,
            world_radius: 3_500.0,
            population_count: 4,
            baseline_count: 1,
            max_world_snakes: 8,
            max_non_population_brains: 3,
            max_body_points: 1_000,
            max_pellets: 1_000,
            spatial_index_bytes: 1,
            worker_scratch_bytes: 1,
            checkpoint_scratch_bytes: 1,
            controller_input_hold_ms: 500,
            controller_disconnect_grace_ms: 30_000,
        }
    }

    #[test]
    fn prepares_atomic_typed_values_and_new_identity() {
        let prepared = prepare_live_settings(
            &config(),
            7,
            &[
                LiveSettingUpdate {
                    path: "simSpeed".into(),
                    value: 2.5,
                },
                LiveSettingUpdate {
                    path: "sense.maxPelletChecks".into(),
                    value: 900.0,
                },
                LiveSettingUpdate {
                    path: "foodSpawn.edgeFalloffEnabled".into(),
                    value: 0.0,
                },
            ],
        )
        .expect("valid live batch");
        assert_eq!(prepared.config_revision, 8);
        assert_eq!(prepared.config.requested_sim_speed, 2.5);
        assert_eq!(
            prepared.config_hash,
            normalized_config_hash(&prepared.config).unwrap()
        );
    }

    #[test]
    fn rejects_the_complete_batch_before_changing_the_source() {
        let source = config();
        let snapshot = source.clone();
        let error = prepare_live_settings(
            &source,
            1,
            &[
                LiveSettingUpdate {
                    path: "simSpeed".into(),
                    value: 2.0,
                },
                LiveSettingUpdate {
                    path: "snakeCount".into(),
                    value: 10.0,
                },
            ],
        )
        .expect_err("reset-only setting must reject");
        assert!(error.to_string().contains("requires reset"));
        assert_eq!(source, snapshot);
    }
}
