//! Existing build/schema/limit policies without checkpoint or restore ownership.

use super::*;

/// Bind admission to the currently loaded addon and exact settings schema.
pub(super) fn current_build_policy(
    memory_ceiling_bytes: usize,
    settings_schema_sha256: String,
    require_exact_build_identity: bool,
) -> StateAdmissionPolicy {
    let build_identifier = crate::native_addon_build_identifier();
    StateAdmissionPolicy {
        memory_ceiling_bytes,
        require_exact_build_identity,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: crate::native_addon_source_sha256(),
        expected_target_triple: crate::native_addon_build_target(),
        expected_build_profile: crate::native_addon_build_profile(),
        expected_build_class: crate::native_addon_build_class(),
        expected_rustc_version: crate::native_addon_rustc_version(),
        expected_build_contract_sha256: crate::native_addon_build_contract_sha256(),
        expected_math_backend: InferenceMathBackend::Scalar.label().to_owned(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
        expected_settings_schema_sha256: settings_schema_sha256,
    }
}

/// Production restore recognizes only complete frozen or complete life-contract schemas.
pub(super) fn production_restore_policy(
    memory_ceiling_bytes: usize,
    require_exact_build_identity: bool,
) -> Result<StateAdmissionPolicy, FreshRunError> {
    let legacy = normalized_settings_schema_hash(&typescript_default_settings(
        STAGE6A_P0_POPULATION_COUNT,
        STAGE6A_P0_BASELINE_COUNT,
    ))?;
    let modern = normalized_settings_schema_hash(&production_default_settings(
        STAGE6A_P0_POPULATION_COUNT,
        STAGE6A_P0_BASELINE_COUNT,
    ))?;
    let mut policy =
        current_build_policy(memory_ceiling_bytes, legacy, require_exact_build_identity);
    policy
        .allowed_math_backends
        .push(InferenceMathBackend::Sse2.label().into());
    policy
        .allowed_math_backends
        .push(InferenceMathBackend::I16MatrixFma128.label().into());
    policy
        .allowed_math_backends
        .push(InferenceMathBackend::I16AffineFma128.label().into());
    policy.allowed_settings_schema_sha256.push(modern);
    for mut settings in [
        typescript_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT),
        production_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT),
    ] {
        crate::engine::sensor_contract::SensorContractIdentity::new(
            crate::engine::sensor_contract::SensorProfile::Motion,
            16,
        )
        .and_then(|contract| contract.record_v4_settings(&mut settings))
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&settings)?);
        let mut recorded_rays = settings.clone();
        // Only field types enter this hash; these values select no defaults.
        crate::engine::ray_work::RayWorkLimits::new(1, 1, 512, 512)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?
            .record_settings(&mut recorded_rays);
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&recorded_rays)?);
        crate::engine::baseline_v2::BaselineV2Settings {
            mode: crate::engine::baseline_v2::BaselineV2Mode::Stochastic,
            control_interval_seconds: 0.06,
        }
        .record_settings(&mut settings)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&settings)?);
        crate::engine::ray_work::RayWorkLimits::new(1, 1, 512, 512)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?
            .record_settings(&mut settings);
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&settings)?);
    }
    Ok(policy)
}

/// Rebuild the fixed P0 archive validation contract without a second population.
pub(crate) fn stage6a_p0_archive_validation_contract(
    memory_ceiling_bytes: usize,
    require_exact_build_identity: bool,
) -> Result<(CheckpointLimits, GraphLimits, StateAdmissionPolicy), FreshRunError> {
    Ok((
        production_checkpoint_limits(),
        stage6a_p0_graph_limits(),
        production_restore_policy(memory_ceiling_bytes, require_exact_build_identity)?,
    ))
}

/// Production graph ceilings covering current editor graphs without unbounded allocation.
pub(crate) fn stage6a_p0_graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 64,
        max_edges: 128,
        max_graph_outputs: 8,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 16 * 1024,
        max_tensor_width: crate::contracts::MAX_BRAIN_TENSOR_WIDTH,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 16,
        max_parameter_floats: crate::contracts::MAX_BRAIN_PARAMETERS,
        max_recurrent_state_floats: crate::contracts::MAX_BRAIN_STATE_FLOATS,
        max_canonical_layout_bytes: 128 * 1024,
        max_architecture_key_bytes: 256 * 1024,
    }
}

/// Managed-checkpoint ceilings for the current population and admitted custom graphs.
/// The 512 MiB envelope admits P3's roughly 461 MiB raw weights and bounds decoded
/// allocation and codec work. Raising it requires remeasuring memory and aligning
/// Node's candidate/final-file disk reserves; the retention budget is independent.
pub(crate) fn production_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 512 * MIB_U64,
        max_manifest_bytes: MIB,
        max_state_bytes: 4 * MIB,
        max_graph_bytes: MIB,
        max_population_index_bytes: MIB,
        max_population_count: MAXIMUM_FRESH_RUN_POPULATION_COUNT,
        max_setting_count: 128,
        max_baseline_rng_count: MAXIMUM_FRESH_RUN_BASELINE_COUNT,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 4 * MIB,
        max_weight_floats: MAXIMUM_FRESH_RUN_POPULATION_COUNT
            * crate::contracts::MAX_BRAIN_PARAMETERS,
        max_recurrent_floats: MAXIMUM_FRESH_RUN_POPULATION_COUNT
            * crate::contracts::MAX_BRAIN_STATE_FLOATS,
        max_numeric_stored_bytes: 512 * MIB_U64,
        max_numeric_candidate_bytes: 512 * MIB_U64,
        max_total_decoded_bytes: 512 * MIB_U64,
    }
}
