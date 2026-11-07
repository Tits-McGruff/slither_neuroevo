//! Config hash for authoritative state.

use super::*;

/// Calculate the versioned schema identity for sorted normalized settings.
///
/// The encoding hashes the setting count followed by length-prefixed UTF-8
/// paths and one value-kind byte: bool `0`, integer `1`, float `2`, text `3`.
/// Values are deliberately excluded so TypeScript/N-API can reproduce the
/// admitted layout independently from a particular configuration instance.
pub fn normalized_settings_schema_hash(
    settings: &[NormalizedSetting],
) -> Result<String, StateError> {
    let mut hasher = Sha256::new();
    hasher.update(b"slither-normalized-settings-schema\0v1");
    hash_u64(
        &mut hasher,
        u64::try_from(settings.len()).map_err(|_| StateError::ArithmeticOverflow {
            context: "normalized settings schema count",
        })?,
    );
    let mut previous: Option<&[u8]> = None;
    for setting in settings {
        validate_text("config.settings.path", &setting.path)?;
        let path = setting.path.as_bytes();
        if previous.is_some_and(|prior| prior >= path) {
            return invalid(
                "config.settings",
                "paths must be unique and sorted by raw UTF-8 bytes",
            );
        }
        previous = Some(path);
        hash_text(&mut hasher, &setting.path)?;
        hasher.update([match setting.value {
            NormalizedSettingValue::Bool(_) => 0,
            NormalizedSettingValue::Integer(_) => 1,
            NormalizedSettingValue::Float(_) => 2,
            NormalizedSettingValue::Text(_) => 3,
        }]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Calculate the canonical SHA-256 identity of one normalized configuration.
///
/// The encoding is domain-separated and length-prefixed; it is an internal
/// state-identity contract rather than human-readable JSON.
pub fn normalized_config_hash(config: &NormalizedEngineConfig) -> Result<String, StateError> {
    let mut hasher = Sha256::new();
    hasher.update(b"slither-normalized-config\0v1");
    hash_u64(&mut hasher, u64::from(config.version));
    hash_text(&mut hasher, &config.settings_schema_sha256)?;
    hash_text(&mut hasher, &config.graph_architecture_key)?;
    hash_u64(&mut hasher, config.fixed_step_seconds.to_bits());
    hash_u64(&mut hasher, config.requested_sim_speed.to_bits());
    hash_u64(&mut hasher, config.world_radius.to_bits());
    for value in [
        config.population_count,
        config.baseline_count,
        config.max_world_snakes,
        config.max_non_population_brains,
        config.max_body_points,
        config.max_pellets,
        config.spatial_index_bytes,
        config.worker_scratch_bytes,
        config.checkpoint_scratch_bytes,
    ] {
        hash_u64(
            &mut hasher,
            u64::try_from(value).map_err(|_| StateError::ArithmeticOverflow {
                context: "canonical config integer",
            })?,
        );
    }
    hash_u64(&mut hasher, config.controller_input_hold_ms);
    hash_u64(&mut hasher, config.controller_disconnect_grace_ms);
    hash_u64(
        &mut hasher,
        u64::try_from(config.settings.len()).map_err(|_| StateError::ArithmeticOverflow {
            context: "canonical setting count",
        })?,
    );
    for setting in &config.settings {
        hash_text(&mut hasher, &setting.path)?;
        match &setting.value {
            NormalizedSettingValue::Bool(value) => {
                hasher.update([0, u8::from(*value)]);
            }
            NormalizedSettingValue::Integer(value) => {
                hasher.update([1]);
                hasher.update(value.to_le_bytes());
            }
            NormalizedSettingValue::Float(value) => {
                hasher.update([2]);
                hasher.update(value.to_bits().to_le_bytes());
            }
            NormalizedSettingValue::Text(value) => {
                hasher.update([3]);
                hash_text(&mut hasher, value)?;
            }
        }
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub(super) fn hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

pub(super) fn hash_text(hasher: &mut Sha256, value: &str) -> Result<(), StateError> {
    hash_u64(
        hasher,
        u64::try_from(value.len()).map_err(|_| StateError::ArithmeticOverflow {
            context: "canonical config text length",
        })?,
    );
    hasher.update(value.as_bytes());
    Ok(())
}
