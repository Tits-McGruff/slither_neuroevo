//! Manifest validation for managed checkpoints.

use super::*;

/// Private bounded manifest for the fixed checkpoint role set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::engine) struct CheckpointManifest {
    pub(in crate::engine) magic: String,
    pub(in crate::engine) container_version: u32,
    pub(in crate::engine) archive_kind: String,
    pub(in crate::engine) checkpoint_format_version: u32,
    pub(in crate::engine) state_version: u32,
    pub(in crate::engine) graph_layout_version: u32,
    pub(in crate::engine) run_id: String,
    pub(in crate::engine) generation_hex: String,
    pub(in crate::engine) completed_step_hex: String,
    pub(in crate::engine) boundary_kind: String,
    pub(in crate::engine) logical_root_sha256: String,
    pub(in crate::engine) roles: Vec<ManifestRole>,
    pub(in crate::engine) role_stored_bytes_hex: String,
    pub(in crate::engine) role_decoded_bytes_hex: String,
    pub(in crate::engine) population_count_hex: String,
    pub(in crate::engine) weight_float_count_hex: String,
    pub(in crate::engine) recurrent_float_count_hex: String,
    pub(in crate::engine) weights_encoding: String,
    pub(in crate::engine) recurrent_encoding: String,
    pub(in crate::engine) graph_architecture_key: String,
    pub(in crate::engine) graph_layout_sha256: String,
    pub(in crate::engine) write_validation_policy: String,
}

/// One encoding-independent role declaration in manifest order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::engine) struct ManifestRole {
    pub(in crate::engine) role: String,
    pub(in crate::engine) path: String,
    pub(in crate::engine) encoding: String,
    pub(in crate::engine) stored_bytes_hex: String,
    pub(in crate::engine) decoded_bytes_hex: String,
    pub(in crate::engine) decoded_count_hex: String,
    pub(in crate::engine) record_size: u32,
    pub(in crate::engine) logical_sha256: String,
}

/// Internal manifest admission result with decoded exact counts and digests.
pub(super) struct ValidatedManifest {
    pub(super) roles: Vec<ValidatedRole>,
    pub(super) population_count: usize,
    pub(super) weight_float_count: usize,
    pub(super) recurrent_float_count: usize,
    pub(super) weights_encoding: NumericEncoding,
    pub(super) recurrent_encoding: NumericEncoding,
}

/// One fully checked manifest role tuple.
pub(super) struct ValidatedRole {
    pub(super) logical_length: u64,
    pub(super) logical_sha256: [u8; 32],
}

/// Select the fixed presentation path for one numeric logical role.
pub(super) fn numeric_path(weights: bool, encoding: NumericEncoding) -> &'static str {
    match (weights, encoding) {
        (true, NumericEncoding::RawF32LeV1) => WEIGHTS_RAW_PATH,
        (true, NumericEncoding::F32LeShuffle4ZstdV1) => WEIGHTS_ZSTD_PATH,
        (false, NumericEncoding::RawF32LeV1) => RECURRENT_RAW_PATH,
        (false, NumericEncoding::F32LeShuffle4ZstdV1) => RECURRENT_ZSTD_PATH,
    }
}

/// Construct a manifest declaration for one raw binary role.
pub(super) fn manifest_role(
    role: &SmallRole,
    digest: &LogicalRoleDigest,
    decoded_count: u64,
) -> ManifestRole {
    ManifestRole {
        role: role.role.to_owned(),
        path: role.path.to_owned(),
        encoding: "raw-binary-v1".to_owned(),
        stored_bytes_hex: encode_u64_hex(role.bytes.len() as u64),
        decoded_bytes_hex: encode_u64_hex(digest.logical_length),
        decoded_count_hex: encode_u64_hex(decoded_count),
        record_size: role.record_size,
        logical_sha256: encode_digest(digest.logical_sha256),
    }
}

/// Construct a manifest declaration for one adaptive numeric role.
pub(super) fn manifest_numeric_role(
    role: &'static str,
    path: &'static str,
    candidate: &NumericCandidate,
    digest: &LogicalRoleDigest,
) -> ManifestRole {
    ManifestRole {
        role: role.to_owned(),
        path: path.to_owned(),
        encoding: candidate.encoding.as_str().to_owned(),
        stored_bytes_hex: encode_u64_hex(candidate.stored_bytes),
        decoded_bytes_hex: encode_u64_hex(candidate.raw_bytes),
        decoded_count_hex: encode_u64_hex(candidate.float_count as u64),
        record_size: 4,
        logical_sha256: encode_digest(digest.logical_sha256),
    }
}

/// Validate every manifest field and role tuple before reading role bodies.
pub(super) fn validate_manifest(
    manifest: &CheckpointManifest,
    entries: &[ScannedEntry],
    limits: &CheckpointLimits,
) -> Result<ValidatedManifest, CheckpointError> {
    if manifest.magic != "slither-neuroevo-checkpoint"
        || manifest.container_version != CHECKPOINT_CONTAINER_VERSION
        || manifest.archive_kind != "managed-checkpoint-v3"
        || manifest.checkpoint_format_version != CHECKPOINT_VERSION
        || manifest.graph_layout_version != CANONICAL_GRAPH_LAYOUT_VERSION
    {
        return Err(CheckpointError::format(
            "MANIFEST_VERSION",
            "unsupported checkpoint manifest identity/version",
        ));
    }
    parse_u64_hex(&manifest.generation_hex, "generation")?;
    parse_u64_hex(&manifest.completed_step_hex, "completed step")?;
    CheckpointBoundaryKind::parse(&manifest.boundary_kind)?;
    CheckpointWriteValidationPolicy::parse(&manifest.write_validation_policy)?;
    if manifest.run_id.is_empty()
        || manifest.run_id.contains('\0')
        || manifest.run_id.len() > 256
        || manifest.run_id.len() > limits.max_string_bytes
    {
        return Err(CheckpointError::format(
            "MANIFEST_RUN_ID",
            "run ID is empty or exceeds string limit",
        ));
    }
    if manifest.graph_architecture_key.is_empty()
        || manifest.graph_architecture_key.len() > limits.max_total_string_bytes
    {
        return Err(CheckpointError::format(
            "MANIFEST_GRAPH_KEY",
            "graph architecture key is empty or exceeds text limit",
        ));
    }
    parse_digest(&manifest.graph_layout_sha256, "graph layout SHA-256")?;
    let weights_encoding = NumericEncoding::parse(&manifest.weights_encoding)?;
    let recurrent_encoding = NumericEncoding::parse(&manifest.recurrent_encoding)?;
    let population_count = u64_to_usize(
        parse_u64_hex(&manifest.population_count_hex, "population count")?,
        "population count",
    )?;
    let weight_float_count = u64_to_usize(
        parse_u64_hex(&manifest.weight_float_count_hex, "weight count")?,
        "weight count",
    )?;
    let recurrent_float_count = u64_to_usize(
        parse_u64_hex(&manifest.recurrent_float_count_hex, "recurrent count")?,
        "recurrent count",
    )?;
    if population_count > limits.max_population_count
        || weight_float_count > limits.max_weight_floats
        || recurrent_float_count > limits.max_recurrent_floats
    {
        return Err(CheckpointError::format(
            "MANIFEST_LIMIT",
            "manifest counts exceed checkpoint limits",
        ));
    }
    if manifest.roles.len() != LOGICAL_ROLE_COUNT || entries.len() != USTAR_ENTRY_COUNT {
        return Err(CheckpointError::format(
            "MANIFEST_ROLES",
            "manifest has an invalid role count",
        ));
    }
    let expected_roles = [
        ("checkpoint", STATE_PATH, "raw-binary-v1", 0u32, 1u64),
        ("graph", GRAPH_PATH, "raw-binary-v1", 0u32, 1u64),
        (
            "population-index",
            POPULATION_INDEX_PATH,
            "raw-binary-v1",
            POPULATION_INDEX_RECORD_BYTES,
            population_count as u64,
        ),
        (
            "population-weights",
            numeric_path(true, weights_encoding),
            weights_encoding.as_str(),
            4u32,
            weight_float_count as u64,
        ),
        (
            "population-recurrent",
            numeric_path(false, recurrent_encoding),
            recurrent_encoding.as_str(),
            4u32,
            recurrent_float_count as u64,
        ),
    ];
    let mut validated_roles = Vec::new();
    validated_roles
        .try_reserve_exact(LOGICAL_ROLE_COUNT)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve validated role tuples")
        })?;
    let mut stored_total = 0u64;
    let mut decoded_total = 0u64;
    for (index, role) in manifest.roles.iter().enumerate() {
        let expected = expected_roles[index];
        if role.role != expected.0
            || role.path != expected.1
            || role.encoding != expected.2
            || role.record_size != expected.3
            || entries[index].name != role.path
        {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                format!("role {index} identity/path/encoding/record size mismatch"),
            ));
        }
        let stored = parse_u64_hex(&role.stored_bytes_hex, "role stored bytes")?;
        let decoded = parse_u64_hex(&role.decoded_bytes_hex, "role decoded bytes")?;
        let decoded_count = parse_u64_hex(&role.decoded_count_hex, "role decoded count")?;
        if stored != entries[index].size || decoded_count != expected.4 {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "role stored length or logical count mismatch",
            ));
        }
        if index < 3 && stored != decoded {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "raw binary role stored/decoded lengths differ",
            ));
        }
        if index == 3
            && decoded
                != (weight_float_count as u64).checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "weight decoded bytes overflow")
                })?
        {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "weight decoded length disagrees with count",
            ));
        }
        if index == 4
            && decoded
                != (recurrent_float_count as u64)
                    .checked_mul(4)
                    .ok_or_else(|| {
                        CheckpointError::format(
                            "COUNT_OVERFLOW",
                            "recurrent decoded bytes overflow",
                        )
                    })?
        {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "recurrent decoded length disagrees with count",
            ));
        }
        let logical_sha256 = parse_digest(&role.logical_sha256, "role logical SHA-256")?;
        stored_total = stored_total.checked_add(stored).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "role stored total overflows")
        })?;
        decoded_total = decoded_total.checked_add(decoded).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "role decoded total overflows")
        })?;
        validated_roles.push(ValidatedRole {
            logical_length: decoded,
            logical_sha256,
        });
    }
    if stored_total != parse_u64_hex(&manifest.role_stored_bytes_hex, "role stored total")?
        || decoded_total != parse_u64_hex(&manifest.role_decoded_bytes_hex, "role decoded total")?
        || decoded_total > limits.max_total_decoded_bytes
    {
        return Err(CheckpointError::format(
            "MANIFEST_TOTAL",
            "manifest role byte totals disagree or exceed limit",
        ));
    }
    let root_roles: Vec<LogicalRoleDigest> = expected_roles
        .iter()
        .zip(&validated_roles)
        .map(|(expected, role)| LogicalRoleDigest {
            role: expected.0,
            logical_length: role.logical_length,
            logical_sha256: role.logical_sha256,
        })
        .collect();
    let computed_root = compute_logical_root(&root_roles)?;
    if parse_digest(&manifest.logical_root_sha256, "logical root SHA-256")? != computed_root {
        return Err(CheckpointError::format(
            "LOGICAL_ROOT",
            "manifest logical root does not match ordered role tuples",
        ));
    }
    Ok(ValidatedManifest {
        roles: validated_roles,
        population_count,
        weight_float_count,
        recurrent_float_count,
        weights_encoding,
        recurrent_encoding,
    })
}

/// Cross-check every manifest fact mirrored from authoritative decoded content.
pub(super) fn crosscheck_manifest_state(
    manifest: &CheckpointManifest,
    validated: &ValidatedManifest,
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), CheckpointError> {
    let boundary = match candidate.phase {
        AuthorityPhase::GenerationBoundary(kind) => CheckpointBoundaryKind::from_state(kind),
        AuthorityPhase::Running => {
            return Err(CheckpointError::format(
                "MANIFEST_STATE_MISMATCH",
                "restored checkpoint unexpectedly contains running state",
            ));
        }
    };
    let actual_weight_count = candidate
        .population
        .iter()
        .try_fold(0usize, |total, genome| {
            total.checked_add(genome.weights.len())
        })
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "admitted weight count overflows")
        })?;
    let actual_recurrent_count = candidate
        .brains
        .iter()
        .try_fold(0usize, |total, brain| {
            total.checked_add(brain.recurrent.len())
        })
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "admitted recurrent count overflows")
        })?;
    if manifest.checkpoint_format_version != candidate.versions.checkpoint
        || manifest.state_version != candidate.versions.state
        || manifest.graph_layout_version != candidate.versions.graph_layout
        || manifest.run_id != candidate.identity.run_id
        || parse_u64_hex(&manifest.generation_hex, "generation")? != candidate.generation.generation
        || parse_u64_hex(&manifest.completed_step_hex, "completed step")?
            != candidate.generation.completed_step
        || CheckpointBoundaryKind::parse(&manifest.boundary_kind)? != boundary
        || validated.population_count != candidate.population.len()
        || validated.population_count != candidate.config.population_count
        || validated.weight_float_count != actual_weight_count
        || validated.recurrent_float_count != actual_recurrent_count
        || manifest.graph_architecture_key != candidate.config.graph_architecture_key
        || manifest.graph_architecture_key != graph.architecture_key
        || manifest.graph_layout_sha256 != graph.layout_digest_hex()
    {
        return Err(CheckpointError::format(
            "MANIFEST_STATE_MISMATCH",
            "manifest mirrors do not match admitted authoritative state/graph/counts",
        ));
    }
    Ok(())
}

/// Read and verify a raw small binary role using its one logical digest.
pub(super) fn read_and_verify_small_role(
    file: &mut File,
    entry: &ScannedEntry,
    role: &ValidatedRole,
    limit: usize,
) -> Result<Vec<u8>, CheckpointError> {
    let bytes = read_entry_bytes(file, entry, limit)?;
    if role.logical_length != bytes.len() as u64 || sha256(&bytes) != role.logical_sha256 {
        return Err(CheckpointError::format(
            "LOGICAL_ROLE_SHA256",
            format!("logical role {} failed length/SHA-256", entry.name),
        ));
    }
    Ok(bytes)
}

/// Map a validated private manifest to the bounded Node-facing descriptor contract.
pub(super) fn content_descriptor_from_manifest(
    manifest: &CheckpointManifest,
    archive_length: u64,
    actual_filename: String,
) -> Result<CheckpointContentDescriptor, CheckpointError> {
    let expected_filename = format!("{}.checkpoint-v3", manifest.logical_root_sha256);
    if actual_filename != expected_filename {
        return Err(CheckpointError::format(
            "MANAGED_FILENAME",
            format!("managed filename {actual_filename:?} does not match digest-derived {expected_filename:?}"),
        ));
    }
    Ok(CheckpointContentDescriptor {
        run_id: manifest.run_id.clone(),
        generation_hex: canonical_u64_hex(&manifest.generation_hex, "generation")?,
        completed_step_hex: canonical_u64_hex(&manifest.completed_step_hex, "completed step")?,
        boundary_kind: CheckpointBoundaryKind::parse(&manifest.boundary_kind)?,
        checkpoint_format_version_hex: encode_u64_hex(u64::from(
            manifest.checkpoint_format_version,
        )),
        state_version_hex: encode_u64_hex(u64::from(manifest.state_version)),
        graph_layout_version_hex: encode_u64_hex(u64::from(manifest.graph_layout_version)),
        managed_root: "checkpoint-v3".to_owned(),
        logical_root_sha256: encode_digest(parse_digest(
            &manifest.logical_root_sha256,
            "logical root",
        )?),
        relative_filename: expected_filename,
        stored_byte_count_hex: encode_u64_hex(archive_length),
        decoded_byte_count_hex: canonical_u64_hex(
            &manifest.role_decoded_bytes_hex,
            "decoded role bytes",
        )?,
        population_count_hex: canonical_u64_hex(
            &manifest.population_count_hex,
            "population count",
        )?,
        role_count_hex: encode_u64_hex(LOGICAL_ROLE_COUNT as u64),
        weight_count_hex: canonical_u64_hex(&manifest.weight_float_count_hex, "weight count")?,
        recurrent_state_count_hex: canonical_u64_hex(
            &manifest.recurrent_float_count_hex,
            "recurrent count",
        )?,
        weights_encoding: NumericEncoding::parse(&manifest.weights_encoding)?,
        recurrent_state_encoding: NumericEncoding::parse(&manifest.recurrent_encoding)?,
        graph_layout_sha256: encode_digest(parse_digest(
            &manifest.graph_layout_sha256,
            "graph layout",
        )?),
        write_validation_policy: CheckpointWriteValidationPolicy::parse(
            &manifest.write_validation_policy,
        )?,
    })
}

/// Combine immutable content facts with the current publication correlation values.
pub(super) fn publication_descriptor(
    content: CheckpointContentDescriptor,
    operation_id: CheckpointOperationId,
    transition_epoch: u64,
) -> CheckpointDescriptor {
    CheckpointDescriptor {
        protocol_version: CHECKPOINT_DESCRIPTOR_VERSION,
        managed_root: content.managed_root,
        operation_id,
        transition_epoch_hex: encode_u64_hex(transition_epoch),
        run_id: content.run_id,
        generation_hex: content.generation_hex,
        completed_step_hex: content.completed_step_hex,
        boundary_kind: content.boundary_kind,
        checkpoint_format_version_hex: content.checkpoint_format_version_hex,
        state_version_hex: content.state_version_hex,
        graph_layout_version_hex: content.graph_layout_version_hex,
        logical_root_sha256: content.logical_root_sha256,
        relative_filename: content.relative_filename,
        stored_byte_count_hex: content.stored_byte_count_hex,
        decoded_byte_count_hex: content.decoded_byte_count_hex,
        population_count_hex: content.population_count_hex,
        role_count_hex: content.role_count_hex,
        weight_count_hex: content.weight_count_hex,
        recurrent_state_count_hex: content.recurrent_state_count_hex,
        weights_encoding: content.weights_encoding,
        recurrent_state_encoding: content.recurrent_state_encoding,
        graph_layout_sha256: content.graph_layout_sha256,
        write_validation_policy: content.write_validation_policy,
    }
    .compact_storage()
}

/// Bind one fully restored checkpoint's immutable content to a new local
/// publication operation. Import uses this after the flattened save has been
/// reconstructed and admitted, so it does not need to decode the population a
/// second time merely to build the small SQLite descriptor.
pub(crate) fn publication_descriptor_for_restored(
    restored: &RestoredCheckpoint,
    operation_id: CheckpointOperationId,
) -> CheckpointDescriptor {
    publication_descriptor(
        restored.content.clone(),
        operation_id,
        restored.state.world_epoch(),
    )
}

/// Compute the Stage-2-compatible ordered logical root.
pub(super) fn compute_logical_root(
    roles: &[LogicalRoleDigest],
) -> Result<[u8; 32], CheckpointError> {
    let mut hasher = Sha256::new();
    hasher.update(LOGICAL_ROOT_DOMAIN);
    hasher.update(usize_to_u32(roles.len(), "logical role count")?.to_le_bytes());
    let mut seen = Vec::new();
    seen.try_reserve_exact(roles.len()).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve logical role-name set")
    })?;
    for role in roles {
        if role.role.is_empty() || role.role.len() > u16::MAX as usize || seen.contains(&role.role)
        {
            return Err(CheckpointError::format(
                "LOGICAL_ROLE",
                "logical role name is empty, duplicate, or too long",
            ));
        }
        seen.push(role.role);
        hasher.update((role.role.len() as u16).to_le_bytes());
        hasher.update(role.role.as_bytes());
        hasher.update(role.logical_length.to_le_bytes());
        hasher.update(role.logical_sha256);
    }
    Ok(hasher.finalize().into())
}

/// Hash one already-bounded logical role.
pub(super) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Encode exactly 32 bytes as lowercase hexadecimal.
pub(super) fn encode_digest(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(64);
    for byte in digest {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

/// Parse an exact lowercase SHA-256 string.
pub(super) fn parse_digest(value: &str, label: &'static str) -> Result<[u8; 32], CheckpointError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "SHA256",
            format!("{label} must be 64 lowercase hexadecimal digits"),
        ));
    }
    let mut digest = [0u8; 32];
    for (index, target) in digest.iter_mut().enumerate() {
        let high = hex_nibble(value.as_bytes()[index * 2])?;
        let low = hex_nibble(value.as_bytes()[index * 2 + 1])?;
        *target = (high << 4) | low;
    }
    Ok(digest)
}

/// Decode one lowercase hexadecimal nibble after shape validation.
pub(super) fn hex_nibble(byte: u8) -> Result<u8, CheckpointError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(CheckpointError::format(
            "HEX",
            "invalid lowercase hexadecimal digit",
        )),
    }
}

/// Encode a true `u64` fact for JavaScript-safe handoff.
pub(super) fn encode_u64_hex(value: u64) -> String {
    format!("{value:016x}")
}

/// Parse one canonical fixed-width lowercase hexadecimal `u64`.
pub(super) fn parse_u64_hex(value: &str, label: &'static str) -> Result<u64, CheckpointError> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "U64_HEX",
            format!("{label} must be 16 lowercase hexadecimal digits"),
        ));
    }
    u64::from_str_radix(value, 16)
        .map_err(|_| CheckpointError::format("U64_HEX", format!("{label} is not a canonical u64")))
}

/// Validate and retain a canonical fixed-width lowercase hexadecimal `u64`.
pub(super) fn canonical_u64_hex(
    value: &str,
    label: &'static str,
) -> Result<String, CheckpointError> {
    Ok(encode_u64_hex(parse_u64_hex(value, label)?))
}

/// Checked `usize` to `u64` conversion.
pub(super) fn usize_to_u64(value: usize, label: &'static str) -> Result<u64, CheckpointError> {
    u64::try_from(value)
        .map_err(|_| CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit u64")))
}

/// Checked `usize` to `u32` conversion.
pub(super) fn usize_to_u32(value: usize, label: &'static str) -> Result<u32, CheckpointError> {
    u32::try_from(value)
        .map_err(|_| CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit u32")))
}

/// Checked `u64` to platform-size conversion.
pub(super) fn u64_to_usize(value: u64, label: &'static str) -> Result<usize, CheckpointError> {
    usize::try_from(value).map_err(|_| {
        CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit usize"))
    })
}

/// Infallible-on-supported-target `u32` to `usize` conversion expressed as a contract check.
pub(super) fn u32_to_usize(value: u32, label: &'static str) -> Result<usize, CheckpointError> {
    usize::try_from(value).map_err(|_| {
        CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit usize"))
    })
}
