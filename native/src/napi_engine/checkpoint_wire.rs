//! Checkpoint wire for the coarse N-API boundary.

use super::*;

/// Validate one controlled path before moving publication to a worker thread.
pub(crate) fn parse_managed_path(value: String) -> Result<PathBuf> {
    if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
        return Err(Error::new(
            Status::InvalidArg,
            "managedDirectory must be a nonempty NUL-free path of at most 32768 UTF-8 bytes",
        ));
    }
    Ok(PathBuf::from(value))
}

/// Validate one exact operation token before any file work.
pub(crate) fn parse_checkpoint_operation_id(value: String) -> Result<CheckpointOperationId> {
    CheckpointOperationId::parse(value)
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))
}

/// Read controlled publication options without allocating unbounded JS strings.
pub(crate) fn parse_managed_checkpoint_publication_options(
    options: &Object<'_>,
) -> Result<(PathBuf, CheckpointOperationId)> {
    let managed_directory = options
        .get::<JsString<'_>>("managedDirectory")?
        .ok_or_else(|| {
            Error::new(
                Status::InvalidArg,
                "checkpoint publication options omit managedDirectory",
            )
        })?;
    let operation_id = options.get::<JsString<'_>>("operationId")?.ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            "checkpoint publication options omit operationId",
        )
    })?;
    let managed_directory =
        bounded_js_string(managed_directory, "managedDirectory", 32_768, false)?;
    let operation_id = bounded_js_string(operation_id, "operationId", 32, false)?;
    Ok((
        parse_managed_path(managed_directory)?,
        parse_checkpoint_operation_id(operation_id)?,
    ))
}

/// Copy one JavaScript string only after bounded well-formed UTF-16 validation.
pub(crate) fn bounded_js_string(
    value: JsString<'_>,
    field: &str,
    max_utf8_bytes: usize,
    allow_empty: bool,
) -> Result<String> {
    let utf16_len = value.utf16_len()?;
    if utf16_len > max_utf8_bytes {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} exceeds its {max_utf8_bytes}-byte limit"),
        ));
    }
    let utf8_len = value.utf8_len()?;
    if utf8_len > max_utf8_bytes {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} exceeds its {max_utf8_bytes}-byte limit"),
        ));
    }
    let decoded = value.into_utf16()?.as_str().map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{field} must be a well-formed UTF-16 string"),
        )
    })?;
    if (!allow_empty && decoded.is_empty()) || decoded.len() > max_utf8_bytes {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be nonempty and at most {max_utf8_bytes} UTF-8 bytes"),
        ));
    }
    Ok(decoded)
}

/// Parse one canonical Uint32 wire value without JavaScript Number narrowing.
pub(super) fn parse_u32_hex(value: &str, field: &str) -> Result<u32> {
    if value.len() != 8
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be 8 lowercase hexadecimal digits"),
        ));
    }
    u32::from_str_radix(value, 16).map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{field} is not a canonical unsigned 32-bit value"),
        )
    })
}

/// Parse one canonical u64 wire value without JavaScript Number narrowing.
pub(crate) fn parse_u64_hex(value: &str, field: &str, allow_zero: bool) -> Result<u64> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be 16 lowercase hexadecimal digits"),
        ));
    }
    let parsed = u64::from_str_radix(value, 16).map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{field} is not a canonical unsigned 64-bit value"),
        )
    })?;
    if !allow_zero && parsed == 0 {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be positive"),
        ));
    }
    Ok(parsed)
}

/// Convert the Rust descriptor without exposing any authoritative payload bytes.
pub(crate) fn checkpoint_descriptor_to_napi(
    descriptor: CheckpointDescriptor,
) -> ManagedCheckpointDescriptor {
    ManagedCheckpointDescriptor {
        protocol_version: descriptor.protocol_version,
        operation_id: descriptor.operation_id.as_str().to_owned(),
        transition_epoch: descriptor.transition_epoch_hex,
        run_id: descriptor.run_id,
        generation: descriptor.generation_hex,
        completed_step: descriptor.completed_step_hex,
        boundary_kind: descriptor.boundary_kind.as_str().to_owned(),
        checkpoint_format_version: descriptor.checkpoint_format_version_hex,
        state_version: descriptor.state_version_hex,
        graph_layout_version: descriptor.graph_layout_version_hex,
        managed_root: descriptor.managed_root,
        relative_filename: descriptor.relative_filename,
        logical_root_sha256: descriptor.logical_root_sha256,
        stored_byte_count: descriptor.stored_byte_count_hex,
        decoded_byte_count: descriptor.decoded_byte_count_hex,
        role_count: descriptor.role_count_hex,
        population_count: descriptor.population_count_hex,
        weight_count: descriptor.weight_count_hex,
        recurrent_state_count: descriptor.recurrent_state_count_hex,
        weights_encoding: descriptor.weights_encoding.as_str().to_owned(),
        recurrent_state_encoding: descriptor.recurrent_state_encoding.as_str().to_owned(),
        graph_layout_sha256: descriptor.graph_layout_sha256,
        write_validation_policy: descriptor.write_validation_policy.as_str().to_owned(),
    }
}

/// Convert one elite-object descriptor without exposing its numeric body.
pub(crate) fn hall_of_fame_weights_descriptor_to_napi(
    descriptor: HallOfFameWeightsDescriptor,
) -> ManagedHallOfFameWeightsDescriptor {
    ManagedHallOfFameWeightsDescriptor {
        version: descriptor.version,
        logical_sha256: descriptor.logical_sha256,
        relative_filename: descriptor.relative_filename,
        encoding: descriptor.encoding.as_str().to_owned(),
        stored_byte_count: descriptor.stored_byte_count_hex,
        decoded_byte_count: descriptor.decoded_byte_count_hex,
        weight_count: descriptor.weight_count_hex,
    }
}

/// Parse a persistence acknowledgement before napi-rs can allocate its strings.
pub(crate) fn checkpoint_descriptor_from_napi_object(
    descriptor: &Object<'_>,
) -> Result<CheckpointDescriptor> {
    require_exact_checkpoint_descriptor_keys(descriptor)?;
    let protocol_version = descriptor.get::<f64>("protocolVersion")?.ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            "checkpoint descriptor omits protocolVersion",
        )
    })?;
    if !protocol_version.is_finite()
        || protocol_version.fract() != 0.0
        || protocol_version != f64::from(CHECKPOINT_DESCRIPTOR_VERSION)
    {
        return Err(Error::new(
            Status::InvalidArg,
            "protocolVersion must be the exact supported integer",
        ));
    }
    checkpoint_descriptor_from_napi(ManagedCheckpointDescriptor {
        protocol_version: CHECKPOINT_DESCRIPTOR_VERSION,
        operation_id: bounded_object_string(descriptor, "operationId", 32)?,
        transition_epoch: bounded_object_string(descriptor, "transitionEpoch", 16)?,
        run_id: bounded_object_string(descriptor, "runId", MAX_EXPERIMENTAL_RUN_ID_BYTES)?,
        generation: bounded_object_string(descriptor, "generation", 16)?,
        completed_step: bounded_object_string(descriptor, "completedStep", 16)?,
        boundary_kind: bounded_object_string(descriptor, "boundaryKind", 10)?,
        checkpoint_format_version: bounded_object_string(
            descriptor,
            "checkpointFormatVersion",
            16,
        )?,
        state_version: bounded_object_string(descriptor, "stateVersion", 16)?,
        graph_layout_version: bounded_object_string(descriptor, "graphLayoutVersion", 16)?,
        managed_root: bounded_object_string(descriptor, "managedRoot", 13)?,
        relative_filename: bounded_object_string(descriptor, "relativeFilename", 78)?,
        logical_root_sha256: bounded_object_string(descriptor, "logicalRootSha256", 64)?,
        stored_byte_count: bounded_object_string(descriptor, "storedByteCount", 16)?,
        decoded_byte_count: bounded_object_string(descriptor, "decodedByteCount", 16)?,
        role_count: bounded_object_string(descriptor, "roleCount", 16)?,
        population_count: bounded_object_string(descriptor, "populationCount", 16)?,
        weight_count: bounded_object_string(descriptor, "weightCount", 16)?,
        recurrent_state_count: bounded_object_string(descriptor, "recurrentStateCount", 16)?,
        weights_encoding: bounded_object_string(descriptor, "weightsEncoding", 32)?,
        recurrent_state_encoding: bounded_object_string(descriptor, "recurrentStateEncoding", 32)?,
        graph_layout_sha256: bounded_object_string(descriptor, "graphLayoutSha256", 64)?,
        write_validation_policy: bounded_object_string(descriptor, "writeValidationPolicy", 64)?,
    })
}

/// Require the complete enumerable own-key set before reading any field value.
pub(super) fn require_exact_checkpoint_descriptor_keys(descriptor: &Object<'_>) -> Result<()> {
    let names = descriptor.get_property_names()?;
    let length = names.get_array_length()?;
    if usize::try_from(length).ok() != Some(CHECKPOINT_DESCRIPTOR_INPUT_KEYS.len()) {
        return Err(Error::new(
            Status::InvalidArg,
            "checkpoint descriptor has unknown or missing fields",
        ));
    }
    let mut seen = [false; CHECKPOINT_DESCRIPTOR_INPUT_KEYS.len()];
    for index in 0..length {
        let key = names.get_element::<JsString<'_>>(index)?;
        let key = bounded_js_string(key, "checkpoint descriptor key", 64, false)?;
        let position = CHECKPOINT_DESCRIPTOR_INPUT_KEYS
            .iter()
            .position(|expected| *expected == key)
            .ok_or_else(|| {
                Error::new(
                    Status::InvalidArg,
                    format!("checkpoint descriptor contains unknown field {key}"),
                )
            })?;
        if seen[position]
            || !descriptor.has_own_property(CHECKPOINT_DESCRIPTOR_INPUT_KEYS[position])?
        {
            return Err(Error::new(
                Status::InvalidArg,
                "checkpoint descriptor has duplicate or inherited fields",
            ));
        }
        seen[position] = true;
    }
    if seen.iter().any(|present| !present) {
        return Err(Error::new(
            Status::InvalidArg,
            "checkpoint descriptor has unknown or missing fields",
        ));
    }
    Ok(())
}

/// Read one required bounded string property through a raw JavaScript handle.
pub(crate) fn bounded_object_string(
    object: &Object<'_>,
    field: &str,
    max_utf8_bytes: usize,
) -> Result<String> {
    let value = object.get::<JsString<'_>>(field)?.ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            format!("checkpoint descriptor omits {field}"),
        )
    })?;
    bounded_js_string(value, field, max_utf8_bytes, false)
}

/// Reconstruct one scalar descriptor supplied by the persistence worker.
pub(crate) fn checkpoint_descriptor_from_napi(
    descriptor: ManagedCheckpointDescriptor,
) -> Result<CheckpointDescriptor> {
    if descriptor.protocol_version != CHECKPOINT_DESCRIPTOR_VERSION {
        return Err(Error::new(
            Status::InvalidArg,
            "protocolVersion is not the supported checkpoint descriptor version",
        ));
    }
    if descriptor.managed_root != "checkpoint-v3" {
        return Err(Error::new(
            Status::InvalidArg,
            "managedRoot is not the controlled checkpoint-v3 root",
        ));
    }
    for (field, value) in [
        ("transitionEpoch", descriptor.transition_epoch.as_str()),
        ("generation", descriptor.generation.as_str()),
        ("completedStep", descriptor.completed_step.as_str()),
        (
            "checkpointFormatVersion",
            descriptor.checkpoint_format_version.as_str(),
        ),
        ("stateVersion", descriptor.state_version.as_str()),
        (
            "graphLayoutVersion",
            descriptor.graph_layout_version.as_str(),
        ),
        ("storedByteCount", descriptor.stored_byte_count.as_str()),
        ("decodedByteCount", descriptor.decoded_byte_count.as_str()),
        ("roleCount", descriptor.role_count.as_str()),
        ("populationCount", descriptor.population_count.as_str()),
        ("weightCount", descriptor.weight_count.as_str()),
        (
            "recurrentStateCount",
            descriptor.recurrent_state_count.as_str(),
        ),
    ] {
        let allow_zero = field != "transitionEpoch";
        parse_u64_hex(value, field, allow_zero)?;
    }
    let operation_id = parse_checkpoint_operation_id(descriptor.operation_id)?;
    let boundary_kind = match descriptor.boundary_kind.as_str() {
        "run-start" => CheckpointBoundaryKind::RunStart,
        "generation" => CheckpointBoundaryKind::Generation,
        _ => {
            return Err(Error::new(
                Status::InvalidArg,
                "boundaryKind is not a supported checkpoint boundary",
            ));
        }
    };
    let generation = parse_u64_hex(&descriptor.generation, "generation", true)?;
    let completed_step = parse_u64_hex(&descriptor.completed_step, "completedStep", true)?;
    match boundary_kind {
        CheckpointBoundaryKind::RunStart if generation != 1 || completed_step != 0 => {
            return Err(Error::new(
                Status::InvalidArg,
                "run-start descriptor must be generation one at completed step zero",
            ));
        }
        CheckpointBoundaryKind::Generation if completed_step == 0 => {
            return Err(Error::new(
                Status::InvalidArg,
                "generation descriptor must have a positive completed-step count",
            ));
        }
        _ => {}
    }
    let weights_encoding = parse_numeric_encoding(&descriptor.weights_encoding)?;
    let recurrent_state_encoding = parse_numeric_encoding(&descriptor.recurrent_state_encoding)?;
    if descriptor.write_validation_policy
        != CheckpointWriteValidationPolicy::SinglePassLogicalHashesFsyncRenameV1.as_str()
    {
        return Err(Error::new(
            Status::InvalidArg,
            "writeValidationPolicy is not the approved checkpoint policy",
        ));
    }
    for (field, value, maximum) in [
        (
            "runId",
            descriptor.run_id.as_str(),
            MAX_EXPERIMENTAL_RUN_ID_BYTES,
        ),
        (
            "relativeFilename",
            descriptor.relative_filename.as_str(),
            78usize,
        ),
        (
            "logicalRootSha256",
            descriptor.logical_root_sha256.as_str(),
            64usize,
        ),
        (
            "graphLayoutSha256",
            descriptor.graph_layout_sha256.as_str(),
            64usize,
        ),
    ] {
        if value.is_empty() || value.len() > maximum || value.contains('\0') {
            return Err(Error::new(
                Status::InvalidArg,
                format!("{field} exceeds the bounded descriptor fixture field"),
            ));
        }
    }
    for (field, value) in [
        ("logicalRootSha256", descriptor.logical_root_sha256.as_str()),
        ("graphLayoutSha256", descriptor.graph_layout_sha256.as_str()),
    ] {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::new(
                Status::InvalidArg,
                format!("{field} must be a lowercase SHA-256 hex string"),
            ));
        }
    }
    if descriptor.relative_filename != format!("{}.checkpoint-v3", descriptor.logical_root_sha256) {
        return Err(Error::new(
            Status::InvalidArg,
            "relativeFilename is not derived from logicalRootSha256",
        ));
    }
    Ok(CheckpointDescriptor {
        protocol_version: descriptor.protocol_version,
        managed_root: descriptor.managed_root,
        operation_id,
        transition_epoch_hex: descriptor.transition_epoch,
        run_id: descriptor.run_id,
        generation_hex: descriptor.generation,
        completed_step_hex: descriptor.completed_step,
        boundary_kind,
        checkpoint_format_version_hex: descriptor.checkpoint_format_version,
        state_version_hex: descriptor.state_version,
        graph_layout_version_hex: descriptor.graph_layout_version,
        logical_root_sha256: descriptor.logical_root_sha256,
        relative_filename: descriptor.relative_filename,
        stored_byte_count_hex: descriptor.stored_byte_count,
        decoded_byte_count_hex: descriptor.decoded_byte_count,
        population_count_hex: descriptor.population_count,
        role_count_hex: descriptor.role_count,
        weight_count_hex: descriptor.weight_count,
        recurrent_state_count_hex: descriptor.recurrent_state_count,
        weights_encoding,
        recurrent_state_encoding,
        graph_layout_sha256: descriptor.graph_layout_sha256,
        write_validation_policy:
            CheckpointWriteValidationPolicy::SinglePassLogicalHashesFsyncRenameV1,
    })
}

/// Parse one stable numeric encoding used by the checkpoint descriptor.
pub(crate) fn parse_numeric_encoding(value: &str) -> Result<NumericEncoding> {
    match value {
        "raw-f32le-v1" => Ok(NumericEncoding::RawF32LeV1),
        "f32le-shuffle4-zstd-v1" => Ok(NumericEncoding::F32LeShuffle4ZstdV1),
        _ => Err(Error::new(
            Status::InvalidArg,
            "checkpoint descriptor contains an unsupported numeric encoding",
        )),
    }
}

/// Validate one scalar elite-object descriptor before the background queue owns it.
pub(crate) fn hall_of_fame_weights_descriptor_from_napi(
    descriptor: ManagedHallOfFameWeightsDescriptor,
) -> Result<HallOfFameWeightsDescriptor> {
    if descriptor.version != 1
        || descriptor.logical_sha256.len() != 64
        || !descriptor
            .logical_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || descriptor.relative_filename != format!("{}.hof-weights-v1", descriptor.logical_sha256)
    {
        return Err(Error::new(
            Status::InvalidArg,
            "invalid Hall-of-Fame weight identity",
        ));
    }
    let encoding = parse_numeric_encoding(&descriptor.encoding)?;
    for (field, value) in [
        ("storedByteCount", descriptor.stored_byte_count.as_str()),
        ("decodedByteCount", descriptor.decoded_byte_count.as_str()),
        ("weightCount", descriptor.weight_count.as_str()),
    ] {
        parse_u64_hex(value, field, true)?;
    }
    Ok(HallOfFameWeightsDescriptor {
        version: descriptor.version,
        logical_sha256: descriptor.logical_sha256,
        relative_filename: descriptor.relative_filename,
        encoding,
        stored_byte_count_hex: descriptor.stored_byte_count,
        decoded_byte_count_hex: descriptor.decoded_byte_count,
        weight_count_hex: descriptor.weight_count,
    })
}

/// Convert one exact u64 or Float64-bit word to canonical wire hexadecimal.
pub(crate) fn u64_hex(value: u64) -> String {
    format!("{value:016x}")
}

/// Parse one canonical positive command sequence for the background fixture.
pub(crate) fn parse_background_sequence(value: JsString<'_>) -> Result<u64> {
    let value = bounded_js_string(value, "sequenceHex", 16, false)?;
    parse_u64_hex(&value, "sequenceHex", false)
}
