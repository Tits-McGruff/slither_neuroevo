//! Bounded inventory and cross-record validation with independent read cursors.

use super::*;

pub(super) struct InventoryFacts {
    pub(super) version: u32,
    pub(super) completion_bytes: u64,
    pub(super) experiment_bytes: u64,
    pub(super) path: PathBuf,
    pub(super) history_count: u64,
    pub(super) hall_of_fame_count: u64,
    pub(super) history_bytes: u64,
    pub(super) hall_of_fame_bytes: u64,
}

pub(super) fn validate_inventory(
    directory: &Path,
    operation_id: &str,
    descriptor: &ExportInventoryDescriptor,
    maximum_step: u64,
) -> Result<InventoryFacts, CheckpointError> {
    if !matches!(descriptor.version, 1..=3)
        || descriptor.relative_filename
            != format!(".{operation_id}.export-inventory-v{}", descriptor.version)
    {
        return Err(CheckpointError::format(
            "EXPORT_INVENTORY",
            "inventory identity does not match the export operation",
        ));
    }
    let history_count = parse_hex_u64(&descriptor.history_count_hex, "history count")?;
    let hall_of_fame_count =
        parse_hex_u64(&descriptor.hall_of_fame_count_hex, "Hall-of-Fame count")?;
    if history_count > MAX_EXPORT_GENERATIONS || hall_of_fame_count > history_count {
        return Err(CheckpointError::format(
            "EXPORT_INVENTORY",
            "inventory counts are inconsistent or exceed the generation limit",
        ));
    }
    let history_bytes = history_count
        .checked_mul(HISTORY_RECORD_BYTES)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "history inventory length overflowed")
        })?;
    let hall_of_fame_bytes = hall_of_fame_count
        .checked_mul(HOF_RECORD_BYTES)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame inventory length overflowed")
        })?;
    let completion_bytes = if descriptor.version >= 2 {
        history_count
            .checked_mul(COMPLETION_RECORD_BYTES)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "completion inventory overflow")
            })?
    } else {
        0
    };
    let experiment_bytes = if descriptor.version == 3 {
        history_count
            .checked_mul(EXPERIMENT_RECORD_BYTES)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "experiment inventory overflow")
            })?
    } else {
        0
    };
    let expected_bytes = INVENTORY_HEADER_BYTES
        .checked_add(experiment_bytes)
        .and_then(|value| value.checked_add(completion_bytes))
        .and_then(|value| value.checked_add(history_bytes))
        .and_then(|value| value.checked_add(hall_of_fame_bytes))
        .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "inventory length overflowed"))?;
    if parse_hex_u64(&descriptor.stored_byte_count_hex, "inventory stored bytes")? != expected_bytes
    {
        return Err(CheckpointError::format(
            "EXPORT_INVENTORY",
            "inventory declared length does not match its counts",
        ));
    }
    let path = direct_file(
        directory,
        &descriptor.relative_filename,
        expected_bytes,
        "export inventory",
    )?;
    let actual_sha256 = hash_file_range(&path, 0, expected_bytes)?;
    if hex_digest(actual_sha256) != descriptor.sha256 {
        return Err(CheckpointError::format(
            "EXPORT_INVENTORY_SHA256",
            "inventory SHA-256 does not match its descriptor",
        ));
    }
    let mut file = File::open(&path)?;
    let mut header = [0u8; INVENTORY_HEADER_BYTES as usize];
    file.read_exact(&mut header)?;
    if &header[..INVENTORY_MAGIC.len()] != inventory_magic(descriptor.version)
        || header[INVENTORY_MAGIC.len()..16]
            .iter()
            .any(|byte| *byte != 0)
        || u64::from_le_bytes(header[16..24].try_into().unwrap()) != history_count
        || u64::from_le_bytes(header[24..32].try_into().unwrap()) != hall_of_fame_count
    {
        return Err(CheckpointError::format(
            "EXPORT_INVENTORY_HEADER",
            "inventory header is malformed or disagrees with its descriptor",
        ));
    }
    validate_history_records(&mut file, history_count)?;
    if descriptor.version >= 2 {
        file.seek(SeekFrom::Start(
            INVENTORY_HEADER_BYTES + history_bytes + hall_of_fame_bytes,
        ))?;
        validate_completion_records(&mut file, history_count, maximum_step)?;
    }
    if descriptor.version == 3 {
        let completion_offset = INVENTORY_HEADER_BYTES + history_bytes + hall_of_fame_bytes;
        file.seek(SeekFrom::Start(completion_offset + completion_bytes))?;
        let mut validator = ExperimentArchiveValidator::new(
            &path,
            completion_offset,
            INVENTORY_HEADER_BYTES + history_bytes,
            hall_of_fame_count,
            maximum_step,
        )?;
        let mut record = [0u8; EXPERIMENT_RECORD_BYTES as usize];
        for generation in 1..=history_count {
            file.read_exact(&mut record)?;
            validator.validate(&record, generation)?;
        }
    }
    Ok(InventoryFacts {
        version: descriptor.version,
        completion_bytes,
        experiment_bytes,
        path,
        history_count,
        hall_of_fame_count,
        history_bytes,
        hall_of_fame_bytes,
    })
}

pub(super) fn validate_history_records(file: &mut File, count: u64) -> Result<(), CheckpointError> {
    let mut record = [0u8; HISTORY_RECORD_BYTES as usize];
    for generation in 1..=count {
        file.read_exact(&mut record)?;
        if read_u64(&record, 0) != generation
            || [8usize, 16, 24, 40, 48]
                .into_iter()
                .any(|offset| !f64::from_bits(read_u64(&record, offset)).is_finite())
        {
            return Err(CheckpointError::format(
                "EXPORT_HISTORY",
                "history records are non-contiguous or contain non-finite values",
            ));
        }
    }
    Ok(())
}

/// Validate the compact completion sidecar, including the explicit absent legacy sentinel.
pub(super) fn validate_completion_record(
    record: &[u8; COMPLETION_RECORD_BYTES as usize],
    generation: u64,
    maximum_step: u64,
) -> Result<u64, CheckpointError> {
    if read_u64(record, 0) != generation {
        return Err(CheckpointError::format(
            "COMPLETION_GENERATION",
            "completion generation is not contiguous",
        ));
    }
    if record[8..].iter().all(|byte| *byte == 0) {
        return Ok(0);
    }
    let version = u32::from_le_bytes(record[8..12].try_into().unwrap());
    let reason = u32::from_le_bytes(record[12..16].try_into().unwrap());
    let actual = f64::from_bits(read_u64(record, 16));
    let duration = f64::from_bits(read_u64(record, 24));
    let step = read_u64(record, 32);
    if version != 1
        || reason > 2
        || !actual.is_finite()
        || actual <= 0.0
        || !duration.is_finite()
        || !(8.0..=604_800.0).contains(&duration)
        || actual > duration + 1.0 / 60.0 + 0.0002
        || step == 0
        || step > maximum_step
    {
        return Err(CheckpointError::format(
            "COMPLETION_RECORD",
            "invalid completion chronology",
        ));
    }
    Ok(step)
}

pub(super) fn validate_completion_records(
    file: &mut File,
    count: u64,
    maximum_step: u64,
) -> Result<(), CheckpointError> {
    let mut record = [0u8; COMPLETION_RECORD_BYTES as usize];
    let mut previous_step = 0;
    for generation in 1..=count {
        file.read_exact(&mut record)?;
        let step = validate_completion_record(&record, generation, maximum_step)?;
        if step != 0 {
            if step <= previous_step {
                return Err(CheckpointError::format(
                    "COMPLETION_STEP",
                    "completion steps do not advance",
                ));
            }
            previous_step = step;
        }
    }
    Ok(())
}

/// Inventory versions extend metadata only; checkpoint-v3 retains its own version.
pub(super) fn inventory_magic(version: u32) -> &'static [u8; 13] {
    match version {
        3 => INVENTORY_MAGIC_V3,
        2 => INVENTORY_MAGIC_V2,
        _ => INVENTORY_MAGIC,
    }
}

/// Independent bounded readers bind dense experiment rows to completion and sparse HoF rows.
/// Separate opens avoid shared seek positions across platform-specific cloned file handles.
pub(super) struct ExperimentArchiveValidator {
    pub(super) completion: BufReader<File>,
    pub(super) hall: BufReader<File>,
    pub(super) hall_remaining: u64,
    pub(super) hall_record: [u8; HOF_RECORD_BYTES as usize],
    pub(super) hall_loaded: bool,
    pub(super) maximum_step: u64,
    pub(super) previous_step: u64,
}

impl ExperimentArchiveValidator {
    pub(super) fn new(
        path: &Path,
        completion_offset: u64,
        hall_offset: u64,
        hall_count: u64,
        maximum_step: u64,
    ) -> Result<Self, CheckpointError> {
        let mut completion = File::open(path)?;
        completion.seek(SeekFrom::Start(completion_offset))?;
        let mut hall = File::open(path)?;
        hall.seek(SeekFrom::Start(hall_offset))?;
        Ok(Self {
            completion: BufReader::new(completion),
            hall: BufReader::new(hall),
            hall_remaining: hall_count,
            hall_record: [0; HOF_RECORD_BYTES as usize],
            hall_loaded: false,
            maximum_step,
            previous_step: 0,
        })
    }
    pub(super) fn validate(
        &mut self,
        record: &[u8; EXPERIMENT_RECORD_BYTES as usize],
        generation: u64,
    ) -> Result<(), CheckpointError> {
        let mut completion = [0u8; COMPLETION_RECORD_BYTES as usize];
        self.completion.read_exact(&mut completion)?;
        validate_completion_record(&completion, generation, self.maximum_step)?;
        if !self.hall_loaded && self.hall_remaining > 0 {
            self.hall.read_exact(&mut self.hall_record)?;
            self.hall_remaining -= 1;
            self.hall_loaded = true;
        }
        let matching_hall = self.hall_loaded && read_u64(&self.hall_record, 0) == generation;
        if self.hall_loaded && read_u64(&self.hall_record, 0) < generation {
            return Err(CheckpointError::format(
                "EXPERIMENT_HOF",
                "HoF generation ordering mismatch",
            ));
        }
        if read_u64(record, 0) != generation {
            return Err(CheckpointError::format(
                "EXPERIMENT_GENERATION",
                "experiment generation is not contiguous",
            ));
        }
        if record[8..].iter().any(|byte| *byte != 0) {
            let decoded = crate::engine::experiment_records::GenerationExperimentRecords::decode(
                &record[8..],
            )
            .map_err(|error| CheckpointError::format("EXPERIMENT_RECORD", error))?;
            let h = decoded.history;
            let f = decoded.hall_of_fame;
            if h[2] != generation
                || h[3] <= self.previous_step
                || h[3] > self.maximum_step
                || completion[8..].iter().all(|byte| *byte == 0)
                || h[3] != read_u64(&completion, 32)
                || h[4] != u32::from_le_bytes(completion[12..16].try_into().unwrap()) as u64
                || h[5] != read_u64(&completion, 16)
                || h[6] != read_u64(&completion, 24)
            {
                return Err(CheckpointError::format(
                    "EXPERIMENT_COMPLETION",
                    "experiment does not match completion chronology",
                ));
            }
            if matching_hall
                && (f[2] != u32::from_le_bytes(self.hall_record[8..12].try_into().unwrap()) as u64
                    || f[3] != read_u64(&self.hall_record, 16)
                    || f[20] != read_u64(&self.hall_record, 40)
                    || f[24] != read_u64(&self.hall_record, 48))
            {
                return Err(CheckpointError::format(
                    "EXPERIMENT_HOF",
                    "experiment does not match physical HoF source",
                ));
            }
            self.previous_step = h[3];
        }
        if matching_hall {
            self.hall_loaded = false;
        }
        Ok(())
    }
}
