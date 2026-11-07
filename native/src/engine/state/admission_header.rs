//! Admission header for authoritative state.

use super::*;

pub(super) fn validate_admission_header(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    validate_versions(&candidate.versions, graph)?;
    if candidate.versions.rng_bundle != candidate.rng.version {
        return invalid(
            "versions.rng_bundle",
            "recorded RNG version differs from its payload",
        );
    }
    validate_identity(&candidate.identity, policy)?;
    validate_config(
        &candidate.config,
        &candidate.versions,
        &candidate.identity,
        policy,
    )?;
    if candidate.versions.sensor == 4
        || candidate
            .config
            .settings
            .iter()
            .any(|entry| entry.path == "sense.contractVersion")
    {
        let contract = crate::engine::sensor_contract::SensorContractIdentity::from_settings(
            &candidate.config.settings,
        )
        .map_err(|error| StateError::InvalidField {
            field: "config.sensor_contract",
            reason: error.to_string(),
        })?;
        let width = graph
            .nodes
            .iter()
            .find(|node| node.node_type == crate::engine::graph::CompiledNodeType::Input)
            .map(|node| node.output_size);
        if contract.layout_version() != candidate.versions.sensor
            || width != Some(contract.input_size())
        {
            return invalid(
                "config.sensor_contract",
                "version or graph Input does not match structural sensor contract",
            );
        }
    }
    if candidate.config.graph_architecture_key != graph.architecture_key {
        return invalid(
            "config.graph_architecture_key",
            "does not match the compiled graph",
        );
    }
    Ok(())
}

pub(super) fn validate_policy(policy: &StateAdmissionPolicy) -> Result<(), StateError> {
    if policy.memory_ceiling_bytes == 0 {
        return invalid("admission.memory_ceiling_bytes", "ceiling must be positive");
    }
    validate_text(
        "admission.expected_source_revision",
        &policy.expected_source_revision,
    )?;
    validate_text(
        "admission.expected_engine_build_id",
        &policy.expected_engine_build_id,
    )?;
    validate_sha256_hex(
        "admission.expected_source_sha256",
        &policy.expected_source_sha256,
    )?;
    validate_text(
        "admission.expected_target_triple",
        &policy.expected_target_triple,
    )?;
    validate_text(
        "admission.expected_build_profile",
        &policy.expected_build_profile,
    )?;
    validate_build_class(
        "admission.expected_build_class",
        &policy.expected_build_class,
    )?;
    validate_text(
        "admission.expected_rustc_version",
        &policy.expected_rustc_version,
    )?;
    validate_sha256(
        "admission.expected_build_contract_sha256",
        &policy.expected_build_contract_sha256,
    )?;
    validate_text(
        "admission.expected_math_backend",
        &policy.expected_math_backend,
    )?;
    if policy.allowed_math_backends.len() > 3
        || policy
            .allowed_math_backends
            .iter()
            .enumerate()
            .any(|(index, backend)| {
                policy.allowed_math_backends[..index].contains(backend)
                    || backend == &policy.expected_math_backend
                    || crate::engine::inference::InferenceMathBackend::from_label(backend).is_none()
            })
    {
        return invalid(
            "admission.allowed_math_backends",
            "at most three distinct supported arithmetic alternatives",
        );
    }
    validate_sha256(
        "admission.expected_settings_schema_sha256",
        &policy.expected_settings_schema_sha256,
    )?;
    // Frozen/life and v4/v2 families, including explicitly recorded ray budgets.
    // One primary hash plus nine alternatives keeps admission finite.
    if policy.allowed_settings_schema_sha256.len() > 9 {
        return invalid(
            "admission.allowed_settings_schema_sha256",
            "at most nine explicitly admitted alternative schemas",
        );
    }
    for schema in &policy.allowed_settings_schema_sha256 {
        validate_sha256("admission.allowed_settings_schema_sha256", schema)?;
    }
    Ok(())
}

pub(super) fn validate_versions(
    versions: &ContractVersions,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    if versions.state != ENGINE_STATE_VERSION {
        return invalid("versions.state", "unsupported state-contract version");
    }
    for (field, actual, expected) in [
        ("versions.engine", versions.engine, ENGINE_CONTRACT_VERSION),
        ("versions.protocol", versions.protocol, PROTOCOL_VERSION),
        (
            "versions.serializer",
            versions.serializer,
            SERIALIZER_VERSION,
        ),
        (
            "versions.checkpoint",
            versions.checkpoint,
            CHECKPOINT_VERSION,
        ),
    ] {
        if actual != expected {
            return invalid(field, "unsupported contract version");
        }
    }
    if !matches!(versions.sensor, 3 | 4) {
        return invalid("versions.sensor", "unsupported sensor version");
    }
    if !matches!(
        versions.rng_bundle,
        RNG_BUNDLE_VERSION | RNG_BUNDLE_WITH_EVOLVED_VERSION | RNG_BUNDLE_WITH_BASELINE_V2_VERSION
    ) {
        return invalid("versions.rng_bundle", "unsupported RNG-bundle version");
    }
    if graph.layout_version == 0 || graph.architecture_key.is_empty() {
        return invalid("graph", "compiled graph identity is incomplete");
    }
    if versions.graph_layout != graph.layout_version {
        return invalid(
            "versions.graph_layout",
            "does not match the compiled graph layout",
        );
    }
    Ok(())
}

pub(super) fn validate_identity(
    identity: &RunIdentity,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    if let Some(origin) = &identity.legacy_conversion {
        origin
            .validate()
            .map_err(|reason| StateError::InvalidField {
                field: "identity.legacy_conversion",
                reason: reason.into(),
            })?;
    }
    validate_text("identity.run_id", &identity.run_id)?;
    validate_sha256("identity.config_hash", &identity.config_hash)?;
    validate_text("identity.source_revision", &identity.source_revision)?;
    validate_text("identity.engine_build_id", &identity.engine_build_id)?;
    validate_sha256_hex("identity.source_sha256", &identity.source_sha256)?;
    validate_text("identity.target_triple", &identity.target_triple)?;
    validate_text("identity.build_profile", &identity.build_profile)?;
    validate_build_class("identity.build_class", &identity.build_class)?;
    validate_text("identity.rustc_version", &identity.rustc_version)?;
    validate_sha256(
        "identity.build_contract_sha256",
        &identity.build_contract_sha256,
    )?;
    validate_text("identity.math_backend", &identity.math_backend)?;
    let incompatible = identity.target_triple != policy.expected_target_triple
        || identity.build_profile != policy.expected_build_profile
        || identity.build_class != policy.expected_build_class
        || (identity.math_backend != policy.expected_math_backend
            && !policy
                .allowed_math_backends
                .contains(&identity.math_backend));
    let not_exact = identity.source_revision != policy.expected_source_revision
        || identity.engine_build_id != policy.expected_engine_build_id
        || identity.source_sha256 != policy.expected_source_sha256
        || identity.rustc_version != policy.expected_rustc_version
        || identity.build_contract_sha256 != policy.expected_build_contract_sha256;
    if incompatible || (policy.require_exact_build_identity && not_exact) {
        return invalid(
            "identity",
            "checkpoint build identity is not admitted by the loaded engine",
        );
    }
    Ok(())
}

/// Decode one already-versioned SHA-256 identity without allocating.
pub(crate) fn decode_sha256_identity(
    field: &'static str,
    identity: &str,
) -> Result<[u8; 32], StateError> {
    let Some(hex) = identity.strip_prefix("sha256:") else {
        return invalid(field, "must use the sha256:<64 lowercase hex> form");
    };
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return invalid(field, "must contain exactly 64 lowercase hex digits");
    }
    let mut output = [0_u8; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let high = lowercase_hex_nibble(pair[0])
            .ok_or_else(|| invalid_error(field, "must contain lowercase hexadecimal digits"))?;
        let low = lowercase_hex_nibble(pair[1])
            .ok_or_else(|| invalid_error(field, "must contain lowercase hexadecimal digits"))?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

/// Convert one lowercase hexadecimal byte to its numeric nibble.
pub(super) const fn lowercase_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}
