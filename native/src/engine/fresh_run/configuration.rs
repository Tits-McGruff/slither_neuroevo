//! Fresh configuration projections; construction and publication stay in the parent.

use super::*;

/// Qualified defaults apply only to genuinely new configurations. Recorded
/// v3/v1 settings continue through their existing explicit constructors.
pub(super) fn selected_fresh_default_settings(
) -> Result<Vec<crate::engine::state::NormalizedSetting>, FreshRunError> {
    let mut settings =
        production_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT);
    crate::engine::sensor_contract::SensorContractIdentity::new(
        crate::engine::sensor_contract::SensorProfile::Motion,
        16,
    )
    .and_then(|contract| contract.record_v4_settings(&mut settings))
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?
        .record_settings(&mut settings);
    crate::engine::baseline_v2::BaselineV2Settings {
        mode: crate::engine::baseline_v2::BaselineV2Mode::Stochastic,
        control_interval_seconds: 1.0 / 60.0,
    }
    .record_settings(&mut settings)
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    settings
        .iter_mut()
        .find(|setting| setting.path == "sense.maxPelletChecks")
        .ok_or_else(|| FreshRunError::Settings("default pellet work setting is missing".into()))?
        .value = NormalizedSettingValue::Integer(3000);
    Ok(settings)
}

/// Only scalar settings cross the fresh-construction interface; Rust derives the digest.
pub(super) fn numeric_fresh_settings(
    settings: Vec<crate::engine::state::NormalizedSetting>,
) -> Vec<FreshRunSettingUpdate> {
    settings
        .into_iter()
        .filter_map(|setting| {
            Some(FreshRunSettingUpdate {
                path: setting.path,
                value: match setting.value {
                    NormalizedSettingValue::Bool(value) => f64::from(value),
                    NormalizedSettingValue::Integer(value) => value as f64,
                    NormalizedSettingValue::Float(value) => value,
                    NormalizedSettingValue::Text(_) => return None,
                },
            })
        })
        .collect()
}

/// Apply exact typed numeric replacements before projection and state admission.
pub(super) fn apply_fresh_run_settings(
    settings: &mut [crate::engine::state::NormalizedSetting],
    updates: &[FreshRunSettingUpdate],
) -> Result<(), FreshRunError> {
    if updates.len() > 128 {
        return Err(FreshRunError::Settings(
            "fresh-run settings exceed 128 entries".to_owned(),
        ));
    }
    for (index, update) in updates.iter().enumerate() {
        if !update.value.is_finite() || update.path.is_empty() || update.path.len() > 128 {
            return Err(FreshRunError::Settings(
                "fresh-run setting is not bounded and finite".to_owned(),
            ));
        }
        if updates[..index]
            .iter()
            .any(|prior| prior.path == update.path)
        {
            return Err(FreshRunError::Settings(format!(
                "duplicate fresh-run setting {}",
                update.path
            )));
        }
        let setting = settings
            .binary_search_by(|setting| setting.path.as_str().cmp(update.path.as_str()))
            .ok()
            .and_then(|position| settings.get_mut(position))
            .ok_or_else(|| {
                FreshRunError::Settings(format!("unknown fresh-run setting {}", update.path))
            })?;
        let replacement = match &setting.value {
            NormalizedSettingValue::Bool(_) if update.value == 0.0 || update.value == 1.0 => {
                NormalizedSettingValue::Bool(update.value == 1.0)
            }
            NormalizedSettingValue::Integer(_)
                if update.value.fract() == 0.0
                    && update.value >= i64::MIN as f64
                    && update.value <= i64::MAX as f64 =>
            {
                NormalizedSettingValue::Integer(update.value as i64)
            }
            NormalizedSettingValue::Float(_) => NormalizedSettingValue::Float(update.value),
            _ => {
                return Err(FreshRunError::Settings(format!(
                    "fresh-run setting {} has the wrong numeric type",
                    update.path
                )))
            }
        };
        setting.value = replacement;
    }
    Ok(())
}

/// Read one required Float64 setting after typed replacement.
pub(super) fn fresh_float(
    config: &NormalizedEngineConfig,
    path: &str,
) -> Result<f64, FreshRunError> {
    match config
        .settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value)
    {
        Some(NormalizedSettingValue::Float(value)) => Ok(*value),
        _ => Err(FreshRunError::Settings(format!(
            "missing floating fresh-run setting {path}"
        ))),
    }
}

/// Read one required non-negative integer setting after typed replacement.
pub(super) fn fresh_integer(
    config: &NormalizedEngineConfig,
    path: &str,
) -> Result<usize, FreshRunError> {
    fresh_setting_integer(&config.settings, path)
}

/// Read one required non-negative integer directly from a normalized setting set.
pub(super) fn fresh_setting_integer(
    settings: &[crate::engine::state::NormalizedSetting],
    path: &str,
) -> Result<usize, FreshRunError> {
    match settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value)
    {
        Some(NormalizedSettingValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            FreshRunError::Settings(format!("invalid integer fresh-run setting {path}"))
        }),
        _ => Err(FreshRunError::Settings(format!(
            "missing integer fresh-run setting {path}"
        ))),
    }
}

/// Validate the opaque lineage label before any graph or population allocation.
pub(super) fn validated_run_id(value: &str) -> Result<String, FreshRunError> {
    if value.is_empty() || value.contains('\0') || value.len() > 256 {
        return Err(FreshRunError::InvalidRunId);
    }
    Ok(value.to_owned())
}

/// Construct the exact supported P0 normalized configuration.
pub(super) fn stage6a_p0_config(
    graph: &GraphBundle,
    settings: Vec<crate::engine::state::NormalizedSetting>,
    settings_schema_sha256: String,
    population_count: usize,
    baseline_count: usize,
) -> NormalizedEngineConfig {
    NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256,
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 3_500.0,
        population_count,
        baseline_count,
        max_world_snakes: population_count + baseline_count + 64,
        max_non_population_brains: 64,
        max_body_points: MAXIMUM_RUNNING_BODY_POINTS,
        max_pellets: MAXIMUM_RUNNING_PELLETS,
        spatial_index_bytes: 256 * MIB,
        worker_scratch_bytes: 512 * MIB,
        checkpoint_scratch_bytes: 512 * MIB,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    }
}
