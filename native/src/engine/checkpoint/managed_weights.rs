//! Managed weights for managed checkpoints.

use super::*;

/// Publish one elite genome independently of its population checkpoint.
///
/// The file is content-addressed by decoded Float32 bits, so repeated winners
/// reuse one immutable object even when their source generation differs.
pub fn publish_hall_of_fame_weights(
    managed_directory: &Path,
    operation_id: &CheckpointOperationId,
    weights: &[f32],
    limits: &CheckpointLimits,
) -> Result<HallOfFameWeightsDescriptor, CheckpointError> {
    publish_managed_neural_weights(managed_directory, operation_id, weights, limits, u64::MAX)
}

/// Shared immutable weight publication for retained winners and library brains.
/// The legacy suffix is a storage compatibility contract, independent of origin.
/// Reject encoded quota overflow before publishing the final content-addressed file.
pub fn publish_managed_neural_weights(
    managed_directory: &Path,
    operation_id: &CheckpointOperationId,
    weights: &[f32],
    limits: &CheckpointLimits,
    stored_byte_limit: u64,
) -> Result<ManagedNeuralWeightsDescriptor, CheckpointError> {
    validate_limits(limits)?;
    fs::create_dir_all(managed_directory)?;
    let managed_directory = managed_directory.canonicalize()?;
    let source = FloatSource::new(
        vec![weights],
        limits.max_weight_floats,
        "Hall-of-Fame weight",
    )?;
    let mut artifacts = TemporaryArtifacts::new();
    let candidate = select_numeric_candidate(
        &managed_directory,
        operation_id.as_str(),
        "hof-weights",
        &source,
        limits,
        &mut artifacts,
    )?;
    let logical_sha256 = encode_digest(candidate.logical_sha256);
    let relative_filename = format!("{logical_sha256}{HALL_OF_FAME_WEIGHTS_SUFFIX}");
    let final_path = managed_directory.join(&relative_filename);
    let descriptor = HallOfFameWeightsDescriptor {
        version: 1,
        logical_sha256,
        relative_filename,
        encoding: candidate.encoding,
        stored_byte_count_hex: encode_u64_hex(candidate.stored_bytes),
        decoded_byte_count_hex: encode_u64_hex(candidate.raw_bytes),
        weight_count_hex: encode_u64_hex(usize_to_u64(
            candidate.float_count,
            "Hall-of-Fame weight count",
        )?),
    }
    .compact_storage();
    if final_path.exists() {
        return read_existing_neural_weight_descriptor(
            &final_path,
            &descriptor,
            stored_byte_limit.min(limits.max_numeric_stored_bytes),
        );
    }
    if candidate.stored_bytes > stored_byte_limit {
        return Err(CheckpointError::format(
            "NEURAL_WEIGHTS_QUOTA",
            "managed neural weights exceed the available stored-byte allowance",
        ));
    }

    let partial_path = match candidate.encoding {
        NumericEncoding::RawF32LeV1 => {
            let path =
                managed_directory.join(format!(".{}.hof-weights.partial", operation_id.as_str()));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            artifacts.track(path.clone());
            let mut writer = BufWriter::new(file);
            io::copy(&mut FloatByteReader::new(&source), &mut writer)?;
            writer.flush()?;
            let file = writer.into_inner().map_err(|error| error.into_error())?;
            file.sync_all()?;
            if file.metadata()?.len() != candidate.stored_bytes {
                return Err(CheckpointError::format(
                    "HOF_WEIGHTS_LENGTH",
                    "raw Hall-of-Fame file length changed during publication",
                ));
            }
            drop(file);
            path
        }
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let path = candidate.compressed_path.clone().ok_or_else(|| {
                CheckpointError::format(
                    "HOF_WEIGHTS_CODEC",
                    "compressed Hall-of-Fame candidate is missing",
                )
            })?;
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            file.sync_all()?;
            drop(file);
            path
        }
    };
    match rename_noreplace(&partial_path, &final_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists || final_path.exists() => {
            sync_parent_directory(&managed_directory)?;
            return read_existing_neural_weight_descriptor(
                &final_path,
                &descriptor,
                stored_byte_limit,
            );
        }
        Err(error) => return Err(error.into()),
    }
    sync_parent_directory(&managed_directory)?;
    validate_existing_hall_of_fame_weights(&final_path, &descriptor)?;
    Ok(descriptor)
}

/// Preserve a valid older object's actual encoding and bytes even if today's
/// adaptive codec would choose differently. Exact decoded digest/finite checks
/// disambiguate the two admitted encodings without trusting file suffix/content hints.
pub(super) fn read_existing_neural_weight_descriptor(
    path: &Path,
    expected: &ManagedNeuralWeightsDescriptor,
    stored_byte_limit: u64,
) -> Result<ManagedNeuralWeightsDescriptor, CheckpointError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CheckpointError::format(
            "NEURAL_WEIGHTS_FILE",
            "shared weight object must be a regular file",
        ));
    }
    if metadata.len() > stored_byte_limit {
        return Err(CheckpointError::format(
            "NEURAL_WEIGHTS_QUOTA",
            "existing shared weights exceed stored-byte allowance",
        ));
    }
    let mut actual = expected.clone();
    actual.stored_byte_count_hex = encode_u64_hex(metadata.len());
    for encoding in [
        expected.encoding,
        match expected.encoding {
            NumericEncoding::RawF32LeV1 => NumericEncoding::F32LeShuffle4ZstdV1,
            NumericEncoding::F32LeShuffle4ZstdV1 => NumericEncoding::RawF32LeV1,
        },
    ] {
        actual.encoding = encoding;
        if validate_existing_hall_of_fame_weights(path, &actual).is_ok() {
            return Ok(actual);
        }
    }
    Err(CheckpointError::format(
        "NEURAL_WEIGHTS_IDENTITY",
        "existing shared object has invalid packed weight identity",
    ))
}

/// Fully verify an existing content-addressed elite object before reusing it.
pub(crate) fn validate_existing_hall_of_fame_weights(
    path: &Path,
    descriptor: &HallOfFameWeightsDescriptor,
) -> Result<(), CheckpointError> {
    read_validated_hall_of_fame_weights(path, descriptor).map(drop)
}

/// Fully decode one independently retained elite object for save composition.
pub(crate) fn read_validated_hall_of_fame_weights(
    path: &Path,
    descriptor: &HallOfFameWeightsDescriptor,
) -> Result<Box<[f32]>, CheckpointError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(CheckpointError::format(
            "HOF_WEIGHTS_TYPE",
            "Hall-of-Fame object must be a regular file",
        ));
    }
    let stored_bytes = parse_u64_hex(
        &descriptor.stored_byte_count_hex,
        "Hall-of-Fame stored bytes",
    )?;
    let weight_count = u64_to_usize(
        parse_u64_hex(&descriptor.weight_count_hex, "Hall-of-Fame weight count")?,
        "Hall-of-Fame weight count",
    )?;
    let logical_sha256 = parse_digest(&descriptor.logical_sha256, "Hall-of-Fame logical SHA-256")?;
    let decoded_bytes = usize_to_u64(weight_count, "Hall-of-Fame weight count")?
        .checked_mul(4)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame decoded bytes overflow")
        })?;
    if metadata.len() != stored_bytes
        || parse_u64_hex(
            &descriptor.decoded_byte_count_hex,
            "Hall-of-Fame decoded bytes",
        )? != decoded_bytes
    {
        return Err(CheckpointError::format(
            "HOF_WEIGHTS_LENGTH",
            "Hall-of-Fame object length disagrees with its descriptor",
        ));
    }
    let mut file = File::open(path)?;
    let entry = ScannedEntry {
        name: descriptor.relative_filename.clone(),
        #[cfg(test)]
        header_offset: 0,
        data_offset: 0,
        size: stored_bytes,
    };
    let decoded = decode_numeric_role(
        &mut file,
        &entry,
        descriptor.encoding,
        weight_count,
        1,
        weight_count,
        logical_sha256,
    )?;
    if decoded.len() != 1 || decoded[0].len() != weight_count {
        return Err(CheckpointError::format(
            "HOF_WEIGHTS_COUNT",
            "Hall-of-Fame object decoded to the wrong shape",
        ));
    }
    decoded.into_iter().next().ok_or_else(|| {
        CheckpointError::format(
            "HOF_WEIGHTS_COUNT",
            "Hall-of-Fame object decoded without one genome",
        )
    })
}
