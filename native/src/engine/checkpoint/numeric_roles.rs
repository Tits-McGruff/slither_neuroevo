//! Numeric roles for managed checkpoints.

use super::*;

/// Measured candidate for one numeric role.
pub(super) struct NumericCandidate {
    pub(super) encoding: NumericEncoding,
    pub(super) stored_bytes: u64,
    pub(super) raw_bytes: u64,
    pub(super) float_count: usize,
    pub(super) logical_sha256: [u8; 32],
    pub(super) compressed_path: Option<PathBuf>,
    /// Encoded source block count retained for codec regression evidence.
    pub(super) encoded_blocks: usize,
    /// Unexpected reusable-scratch capacity changes; valid encoding keeps this zero.
    pub(super) scratch_capacity_growths: usize,
}

/// Adaptive encoding selected for one already materialized raw Float32 file.
pub(in crate::engine) struct AdaptiveNumericFile {
    pub(in crate::engine) encoding: NumericEncoding,
    pub(in crate::engine) stored_bytes: u64,
    pub(in crate::engine) decoded_bytes: u64,
    pub(in crate::engine) float_count: u64,
    pub(in crate::engine) logical_sha256: [u8; 32],
    pub(in crate::engine) compressed_path: Option<PathBuf>,
}

/// Stream one shuffled-Zstandard candidate to an operation-local bounded file.
pub(super) fn select_numeric_candidate(
    directory: &Path,
    operation_id: &str,
    label: &'static str,
    source: &FloatSource<'_>,
    limits: &CheckpointLimits,
    artifacts: &mut TemporaryArtifacts,
) -> Result<NumericCandidate, CheckpointError> {
    select_numeric_reader_candidate(
        directory,
        operation_id,
        label,
        FloatByteReader::new(source),
        source.total_floats,
        limits,
        artifacts,
    )
}

/// Select the smaller approved encoding for one exact finite raw Float32 file.
pub(in crate::engine) fn select_adaptive_numeric_file(
    raw_path: &Path,
    directory: &Path,
    operation_id: &str,
    label: &'static str,
    float_count: u64,
    limits: &CheckpointLimits,
) -> Result<AdaptiveNumericFile, CheckpointError> {
    let expected_bytes = float_count.checked_mul(4).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "numeric file byte count overflowed")
    })?;
    let metadata = fs::symlink_metadata(raw_path)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() != expected_bytes
    {
        return Err(CheckpointError::format(
            "NUMERIC_SOURCE",
            "adaptive numeric source is not the expected regular raw file",
        ));
    }
    let float_count_usize = u64_to_usize(float_count, "adaptive numeric float count")?;
    let mut artifacts = TemporaryArtifacts::new();
    let candidate = select_numeric_reader_candidate(
        directory,
        operation_id,
        label,
        BufReader::new(File::open(raw_path)?),
        float_count_usize,
        limits,
        &mut artifacts,
    )?;
    if let Some(path) = &candidate.compressed_path {
        artifacts
            .paths
            .retain(|candidate_path| candidate_path != path);
    }
    Ok(AdaptiveNumericFile {
        encoding: candidate.encoding,
        stored_bytes: candidate.stored_bytes,
        decoded_bytes: candidate.raw_bytes,
        float_count,
        logical_sha256: candidate.logical_sha256,
        compressed_path: candidate.compressed_path,
    })
}

/// Stream raw Float32 bytes through the shared adaptive candidate codec.
pub(super) fn select_numeric_reader_candidate<R: Read>(
    directory: &Path,
    operation_id: &str,
    label: &'static str,
    mut reader: R,
    total_floats: usize,
    limits: &CheckpointLimits,
    artifacts: &mut TemporaryArtifacts,
) -> Result<NumericCandidate, CheckpointError> {
    let _phase = ArchivePhaseScope::enter(ArchivePhase::NumericEncode);
    let raw_bytes = usize_to_u64(total_floats, "numeric count")?
        .checked_mul(4)
        .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "numeric byte count overflows"))?;
    if raw_bytes > limits.max_total_decoded_bytes {
        return Err(CheckpointError::format(
            "DECODED_LIMIT",
            format!("{label} raw bytes exceed aggregate decoded limit"),
        ));
    }
    if total_floats == 0 {
        return Ok(NumericCandidate {
            encoding: NumericEncoding::RawF32LeV1,
            stored_bytes: 0,
            raw_bytes: 0,
            float_count: 0,
            logical_sha256: sha256(&[]),
            compressed_path: None,
            encoded_blocks: 0,
            scratch_capacity_growths: 0,
        });
    }
    let candidate_path = directory.join(format!(".{operation_id}.{label}.codec.partial"));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&candidate_path)?;
    artifacts.track(candidate_path.clone());
    let mut writer = BufWriter::new(file);
    let mut compressor = ZstdCompressor::new(ZSTD_LEVEL)?;
    compressor.include_checksum(false)?;
    compressor.include_contentsize(true)?;
    compressor.window_log(ZSTD_WINDOW_LOG_MAX)?;
    let mut hasher = Sha256::new();
    let block_float_limit = SHUFFLED_BLOCK_BYTES / 4;
    let mut encoded_floats = 0usize;
    let mut candidate_bytes = 0u64;
    let mut candidate_abandoned = false;
    let mut encoded_blocks = 0usize;
    let mut scratch_capacity_growths = 0usize;
    let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
    let mut raw = Vec::new();
    raw.try_reserve_exact(SHUFFLED_BLOCK_BYTES).map_err(|_| {
        CheckpointError::format(
            "ALLOCATION",
            "unable to reserve reusable numeric input scratch",
        )
    })?;
    let mut shuffled = Vec::new();
    shuffled
        .try_reserve_exact(SHUFFLED_BLOCK_BYTES)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve reusable shuffled scratch")
        })?;
    let mut frame = Vec::new();
    frame.try_reserve_exact(max_frame_bytes).map_err(|_| {
        CheckpointError::format(
            "ALLOCATION",
            "unable to reserve reusable Zstandard frame scratch",
        )
    })?;
    let initial_capacities = (raw.capacity(), shuffled.capacity(), frame.capacity());
    loop {
        let remaining = total_floats.saturating_sub(encoded_floats);
        if remaining == 0 {
            break;
        }
        let count = remaining.min(block_float_limit);
        let block_bytes = count.checked_mul(4).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "numeric block byte count overflows")
        })?;
        raw.clear();
        raw.resize(block_bytes, 0);
        reader.read_exact(&mut raw)?;
        hasher.update(&raw);
        if !candidate_abandoned {
            shuffle_f32_bytes_into(&raw, &mut shuffled)?;
            frame.clear();
            let written = compressor.compress_to_buffer(&shuffled, &mut frame)?;
            if written != frame.len() || frame.len() > max_frame_bytes {
                return Err(CheckpointError::format(
                    "ZSTD_FRAME",
                    "writer produced an invalid bounded frame length",
                ));
            }
            validate_zstd_frame_header(&frame, block_bytes)?;
            let next_candidate_bytes = candidate_bytes
                .checked_add(SHUFFLED_BLOCK_HEADER_BYTES as u64)
                .and_then(|value| value.checked_add(frame.len() as u64))
                .ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "codec candidate bytes overflow")
                })?;
            if next_candidate_bytes > limits.max_numeric_candidate_bytes {
                candidate_abandoned = true;
            } else {
                writer.write_all(SHUFFLED_BLOCK_MAGIC)?;
                writer
                    .write_all(&usize_to_u32(count, "shuffled block float count")?.to_le_bytes())?;
                writer.write_all(
                    &usize_to_u32(frame.len(), "Zstandard frame bytes")?.to_le_bytes(),
                )?;
                writer.write_all(&frame)?;
                candidate_bytes = next_candidate_bytes;
            }
        }
        encoded_floats = encoded_floats.checked_add(count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "encoded numeric count overflows")
        })?;
        encoded_blocks += 1;
        if (raw.capacity(), shuffled.capacity(), frame.capacity()) != initial_capacities {
            scratch_capacity_growths += 1;
        }
        advance_work_progress(block_bytes);
    }
    let mut trailing = [0u8; 1];
    if reader.read(&mut trailing)? != 0 || encoded_floats != total_floats {
        return Err(CheckpointError::format(
            "NUMERIC_SOURCE",
            "numeric source count changed during encoding",
        ));
    }
    writer.flush()?;
    let file = writer.into_inner().map_err(|error| error.into_error())?;
    let actual_candidate_bytes = file.metadata()?.len();
    drop(file);
    if actual_candidate_bytes != candidate_bytes {
        return Err(CheckpointError::format(
            "NUMERIC_CANDIDATE",
            "codec candidate file length disagrees with streamed count",
        ));
    }
    let logical_sha256: [u8; 32] = hasher.finalize().into();
    if !candidate_abandoned
        && candidate_bytes < raw_bytes
        && candidate_bytes <= limits.max_numeric_stored_bytes
    {
        Ok(NumericCandidate {
            encoding: NumericEncoding::F32LeShuffle4ZstdV1,
            stored_bytes: candidate_bytes,
            raw_bytes,
            float_count: total_floats,
            logical_sha256,
            compressed_path: Some(candidate_path),
            encoded_blocks,
            scratch_capacity_growths,
        })
    } else {
        fs::remove_file(&candidate_path)?;
        if raw_bytes > limits.max_numeric_stored_bytes {
            return Err(CheckpointError::format(
                "NUMERIC_STORED_LIMIT",
                format!("{label} raw bytes exceed numeric stored limit"),
            ));
        }
        Ok(NumericCandidate {
            encoding: NumericEncoding::RawF32LeV1,
            stored_bytes: raw_bytes,
            raw_bytes,
            float_count: total_floats,
            logical_sha256,
            compressed_path: None,
            encoded_blocks,
            scratch_capacity_growths,
        })
    }
}

/// Shuffle packed Float32 bytes into four contiguous byte planes using reusable scratch.
pub(super) fn shuffle_f32_bytes_into(
    raw: &[u8],
    shuffled: &mut Vec<u8>,
) -> Result<(), CheckpointError> {
    if !raw.len().is_multiple_of(4) || raw.len() > SHUFFLED_BLOCK_BYTES {
        return Err(CheckpointError::format(
            "SHUFFLED_BLOCK",
            "invalid raw Float32 block length",
        ));
    }
    let count = raw.len() / 4;
    shuffled.clear();
    if shuffled.capacity() < raw.len() {
        return Err(CheckpointError::format(
            "SHUFFLED_SCRATCH",
            "reusable shuffled scratch capacity is too small",
        ));
    }
    shuffled.resize(raw.len(), 0);
    for byte_index in 0..4 {
        for value_index in 0..count {
            shuffled[byte_index * count + value_index] = raw[value_index * 4 + byte_index];
        }
    }
    Ok(())
}

/// Reconstruct one bounded packed block in reusable scratch before bulk hashing.
pub(super) fn unshuffle_f32_bytes_into(
    shuffled: &[u8],
    raw: &mut Vec<u8>,
) -> Result<(), CheckpointError> {
    if !shuffled.len().is_multiple_of(4) || shuffled.len() > SHUFFLED_BLOCK_BYTES {
        return Err(CheckpointError::format(
            "SHUFFLED_BLOCK",
            "invalid shuffled Float32 block length",
        ));
    }
    if raw.capacity() < shuffled.len() {
        return Err(CheckpointError::format(
            "SHUFFLED_SCRATCH",
            "reusable raw scratch capacity is too small",
        ));
    }
    raw.clear();
    raw.resize(shuffled.len(), 0);
    let count = shuffled.len() / 4;
    for (value_index, bytes) in raw.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *bytes = [
            shuffled[value_index],
            shuffled[count + value_index],
            shuffled[2 * count + value_index],
            shuffled[3 * count + value_index],
        ];
    }
    Ok(())
}

/// Final packed buffers allocated in their authoritative per-brain ownership shape.
pub(super) struct SegmentedF32Builder {
    pub(super) segments: Vec<Vec<f32>>,
    pub(super) segment_length: usize,
    pub(super) total_expected: usize,
    pub(super) total_written: usize,
}

impl SegmentedF32Builder {
    /// Preflight and allocate each final segment before decompression starts.
    pub(super) fn new(
        segment_count: usize,
        segment_length: usize,
        expected: usize,
    ) -> Result<Self, CheckpointError> {
        if segment_count.checked_mul(segment_length) != Some(expected) {
            return Err(CheckpointError::format(
                "NUMERIC_SEGMENTS",
                "numeric segment shape disagrees with expected float count",
            ));
        }
        let mut segments = Vec::new();
        segments.try_reserve_exact(segment_count).map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve numeric segments")
        })?;
        for _ in 0..segment_count {
            let mut segment = Vec::new();
            segment.try_reserve_exact(segment_length).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve numeric segment floats")
            })?;
            segments.push(segment);
        }
        Ok(Self {
            segments,
            segment_length,
            total_expected: expected,
            total_written: 0,
        })
    }

    /// Append packed little-endian values, checking once per block and final owner.
    pub(super) fn append_packed(&mut self, bytes: &[u8]) -> Result<(), CheckpointError> {
        let (values, remainder) = bytes.as_chunks::<4>();
        if !remainder.is_empty() {
            return Err(CheckpointError::format(
                "NUMERIC_LENGTH",
                "decoded numeric chunk is not Float32 aligned",
            ));
        }
        if values.len() > self.total_expected - self.total_written {
            return Err(CheckpointError::format(
                "NUMERIC_COUNT",
                "decoded numeric role exceeds declared count",
            ));
        }
        if values.is_empty() {
            return Ok(());
        }
        if self.segment_length == 0 {
            return Err(CheckpointError::format(
                "NUMERIC_SEGMENTS",
                "nonempty payload has zero-length segments",
            ));
        }
        let mut remaining = values;
        while !remaining.is_empty() {
            let segment_index = self.total_written / self.segment_length;
            let segment = &mut self.segments[segment_index];
            let take = remaining.len().min(self.segment_length - segment.len());
            let (chunk, rest) = remaining.split_at(take);
            // Every segment reserved its full declared length before decoding;
            // this exact-size iterator cannot extend beyond that allocation.
            segment.extend(
                chunk
                    .iter()
                    .map(|bytes| f32::from_bits(u32::from_le_bytes(*bytes))),
            );
            self.total_written += take;
            remaining = rest;
        }
        Ok(())
    }

    /// Finish only after every segment received its exact count.
    pub(super) fn finish(self) -> Result<Vec<Box<[f32]>>, CheckpointError> {
        if self.total_written != self.total_expected
            || self
                .segments
                .iter()
                .any(|segment| segment.len() != self.segment_length)
        {
            return Err(CheckpointError::format(
                "NUMERIC_COUNT",
                "decoded numeric count is incomplete",
            ));
        }
        Ok(self
            .segments
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect())
    }
}

/// Inspect and admit one complete ordinary Zstandard frame before output allocation.
pub(super) fn validate_zstd_frame_header(
    frame: &[u8],
    expected_decoded_bytes: usize,
) -> Result<(), CheckpointError> {
    let mut header = MaybeUninit::<zstd_safe::zstd_sys::ZSTD_FrameHeader>::uninit();
    // SAFETY: `header` points to writable uninitialized storage of exactly the
    // required C type, while `frame` remains alive and immutable for the call.
    // Zstandard retains neither pointer. We call `assume_init` only on the
    // documented zero-success result below.
    let result = unsafe {
        zstd_safe::zstd_sys::ZSTD_getFrameHeader(
            header.as_mut_ptr(),
            frame.as_ptr().cast(),
            frame.len(),
        )
    };
    // SAFETY: `ZSTD_isError` only classifies the returned integer.
    if unsafe { zstd_safe::zstd_sys::ZSTD_isError(result) } != 0 {
        return Err(CheckpointError::format(
            "ZSTD_FRAME",
            "invalid Zstandard frame header",
        ));
    }
    if result != 0 {
        return Err(CheckpointError::format(
            "ZSTD_FRAME",
            "incomplete Zstandard frame header",
        ));
    }
    // SAFETY: a zero `ZSTD_getFrameHeader` result guarantees every field was initialized.
    let header = unsafe { header.assume_init() };
    if header.frameType != zstd_safe::zstd_sys::ZSTD_FrameType_e::ZSTD_frame || header.dictID != 0 {
        return Err(CheckpointError::format(
            "ZSTD_FRAME",
            "SFZ1 requires one ordinary dictionary-free Zstandard frame",
        ));
    }
    if header.frameContentSize != expected_decoded_bytes as u64 {
        return Err(CheckpointError::format(
            "ZSTD_CONTENT_SIZE",
            "Zstandard frame must declare its exact decoded block size",
        ));
    }
    let maximum_window = 1u64.checked_shl(ZSTD_WINDOW_LOG_MAX).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "Zstandard window bound overflows")
    })?;
    if header.windowSize == 0 || header.windowSize > maximum_window {
        return Err(CheckpointError::format(
            "ZSTD_WINDOW",
            format!(
                "Zstandard decoder window {} exceeds bound {maximum_window}",
                header.windowSize
            ),
        ));
    }
    Ok(())
}

/// Validate one numeric role's complete stored structure before final allocations.
pub(super) fn preflight_numeric_role(
    file: &mut File,
    entry: &ScannedEntry,
    encoding: NumericEncoding,
    expected_floats: usize,
) -> Result<(), CheckpointError> {
    let expected_bytes = usize_to_u64(expected_floats, "numeric float count")?
        .checked_mul(4)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "numeric decoded bytes overflow")
        })?;
    match encoding {
        NumericEncoding::RawF32LeV1 => {
            if entry.size != expected_bytes {
                return Err(CheckpointError::format(
                    "NUMERIC_LENGTH",
                    "raw numeric stored length disagrees with count",
                ));
            }
        }
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
            let mut remaining = entry.size;
            let mut decoded_total = 0usize;
            let mut frame = Vec::new();
            frame.try_reserve_exact(max_frame_bytes).map_err(|_| {
                CheckpointError::format(
                    "ALLOCATION",
                    "unable to reserve reusable bounded Zstandard preflight frame",
                )
            })?;
            file.seek(SeekFrom::Start(entry.data_offset))?;
            while remaining > 0 {
                if remaining < SHUFFLED_BLOCK_HEADER_BYTES as u64 {
                    return Err(CheckpointError::format(
                        "SHUFFLED_HEADER",
                        "truncated SFZ1 block header",
                    ));
                }
                let mut header = [0u8; SHUFFLED_BLOCK_HEADER_BYTES];
                file.read_exact(&mut header)?;
                remaining -= SHUFFLED_BLOCK_HEADER_BYTES as u64;
                if &header[..4] != SHUFFLED_BLOCK_MAGIC {
                    return Err(CheckpointError::format(
                        "SHUFFLED_MAGIC",
                        "unsupported SFZ1 block marker",
                    ));
                }
                let float_count = u32_to_usize(
                    u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
                    "SFZ1 float count",
                )?;
                let frame_bytes = u32_to_usize(
                    u32::from_le_bytes([header[8], header[9], header[10], header[11]]),
                    "SFZ1 frame bytes",
                )?;
                let decoded_bytes = float_count.checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 decoded block bytes overflow")
                })?;
                if float_count == 0
                    || decoded_bytes > SHUFFLED_BLOCK_BYTES
                    || frame_bytes == 0
                    || frame_bytes > max_frame_bytes
                    || frame_bytes as u64 > remaining
                {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 block declaration exceeds bounds",
                    ));
                }
                decoded_total = decoded_total.checked_add(float_count).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 aggregate count overflows")
                })?;
                if decoded_total > expected_floats {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 decoded count exceeds manifest",
                    ));
                }
                frame.clear();
                frame.resize(frame_bytes, 0);
                file.read_exact(&mut frame)?;
                remaining -= frame_bytes as u64;
                validate_zstd_frame_header(&frame, decoded_bytes)?;
                let compressed_size =
                    zstd_safe::find_frame_compressed_size(&frame).map_err(|_| {
                        CheckpointError::format(
                            "ZSTD_FRAME",
                            "unable to determine Zstandard frame boundary",
                        )
                    })?;
                if compressed_size != frame.len() {
                    return Err(CheckpointError::format(
                        "ZSTD_FRAME",
                        "SFZ1 envelope contains trailing frame bytes",
                    ));
                }
                advance_work_progress(frame_bytes);
            }
            if decoded_total != expected_floats || expected_bytes != (decoded_total as u64) * 4 {
                return Err(CheckpointError::format(
                    "NUMERIC_COUNT",
                    "SFZ1 decoded count is incomplete",
                ));
            }
        }
    }
    Ok(())
}

/// Fully validate one bounded adaptive stream while writing raw Float32 bytes.
pub(in crate::engine) fn decode_adaptive_numeric_reader<R: Read, W: Write>(
    input: &mut R,
    encoding: NumericEncoding,
    stored_bytes: u64,
    expected_floats: u64,
    expected_sha256: [u8; 32],
    output: &mut W,
) -> Result<(), CheckpointError> {
    let _phase = ArchivePhaseScope::enter(ArchivePhase::NumericDecode);
    let expected_floats = u64_to_usize(expected_floats, "adaptive numeric float count")?;
    if encoding == NumericEncoding::RawF32LeV1
        && stored_bytes
            != expected_floats
                .checked_mul(4)
                .and_then(|bytes| u64::try_from(bytes).ok())
                .ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "raw numeric length overflowed")
                })?
    {
        return Err(CheckpointError::format(
            "NUMERIC_LENGTH",
            "raw numeric stored length disagrees with count",
        ));
    }
    decode_adaptive_numeric_reader_inner(
        input,
        encoding,
        stored_bytes,
        expected_floats,
        expected_sha256,
        output,
    )
}

/// Shared bounded decoder after file-specific or stream-specific preflight.
pub(super) fn decode_adaptive_numeric_reader_inner<R: Read, W: Write>(
    file: &mut R,
    encoding: NumericEncoding,
    stored_bytes: u64,
    expected_floats: usize,
    expected_sha256: [u8; 32],
    output: &mut W,
) -> Result<(), CheckpointError> {
    let mut hasher = Sha256::new();
    match encoding {
        NumericEncoding::RawF32LeV1 => {
            let mut remaining = stored_bytes;
            let initial_bytes = u64_to_usize(
                remaining.min(SHUFFLED_BLOCK_BYTES as u64),
                "raw numeric buffer bytes",
            )?;
            let mut buffer = vec![0u8; initial_bytes];
            while remaining > 0 {
                let take = buffer
                    .len()
                    .min(u64_to_usize(remaining, "raw numeric bytes")?);
                file.read_exact(&mut buffer[..take])?;
                hasher.update(&buffer[..take]);
                output.write_all(&buffer[..take])?;
                advance_work_progress(take);
                remaining -= take as u64;
            }
        }
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
            let mut remaining = stored_bytes;
            let mut decompressor = ZstdDecompressor::new()?;
            decompressor.window_log_max(ZSTD_WINDOW_LOG_MAX)?;
            let mut frame = Vec::with_capacity(max_frame_bytes);
            let mut shuffled = Vec::with_capacity(SHUFFLED_BLOCK_BYTES);
            let mut raw = Vec::with_capacity(SHUFFLED_BLOCK_BYTES);
            let mut decoded_total = 0usize;
            while remaining > 0 {
                if remaining < SHUFFLED_BLOCK_HEADER_BYTES as u64 {
                    return Err(CheckpointError::format(
                        "SHUFFLED_HEADER",
                        "adaptive numeric file changed after preflight",
                    ));
                }
                let mut header = [0u8; SHUFFLED_BLOCK_HEADER_BYTES];
                file.read_exact(&mut header)?;
                remaining -= SHUFFLED_BLOCK_HEADER_BYTES as u64;
                if &header[..4] != SHUFFLED_BLOCK_MAGIC {
                    return Err(CheckpointError::format(
                        "SHUFFLED_MAGIC",
                        "adaptive numeric file changed after preflight",
                    ));
                }
                let float_count = u32_to_usize(
                    u32::from_le_bytes(header[4..8].try_into().unwrap()),
                    "SFZ1 float count",
                )?;
                let frame_bytes = u32_to_usize(
                    u32::from_le_bytes(header[8..12].try_into().unwrap()),
                    "SFZ1 frame bytes",
                )?;
                let decoded_bytes = float_count.checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "decoded numeric block overflowed")
                })?;
                decoded_total = decoded_total.checked_add(float_count).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "decoded numeric count overflowed")
                })?;
                if float_count == 0
                    || decoded_bytes > SHUFFLED_BLOCK_BYTES
                    || decoded_total > expected_floats
                    || frame_bytes == 0
                    || frame_bytes > max_frame_bytes
                    || frame_bytes as u64 > remaining
                {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "adaptive numeric file changed after preflight",
                    ));
                }
                frame.clear();
                frame.resize(frame_bytes, 0);
                file.read_exact(&mut frame)?;
                remaining -= frame_bytes as u64;
                validate_zstd_frame_header(&frame, decoded_bytes)?;
                let compressed_size =
                    zstd_safe::find_frame_compressed_size(&frame).map_err(|_| {
                        CheckpointError::format(
                            "ZSTD_FRAME",
                            "unable to determine adaptive Zstandard frame boundary",
                        )
                    })?;
                if compressed_size != frame.len() {
                    return Err(CheckpointError::format(
                        "ZSTD_FRAME",
                        "adaptive SFZ1 envelope contains trailing frame bytes",
                    ));
                }
                shuffled.clear();
                let written = decompressor.decompress_to_buffer(&frame, &mut shuffled)?;
                if written != decoded_bytes || shuffled.len() != decoded_bytes {
                    return Err(CheckpointError::format(
                        "ZSTD_DECODED_LENGTH",
                        "decoded numeric block length changed after preflight",
                    ));
                }
                unshuffle_f32_bytes_into(&shuffled, &mut raw)?;
                hasher.update(&raw);
                output.write_all(&raw)?;
                advance_work_progress(decoded_bytes);
            }
            if decoded_total != expected_floats {
                return Err(CheckpointError::format(
                    "NUMERIC_COUNT",
                    "adaptive numeric file decoded the wrong Float32 count",
                ));
            }
        }
    }
    if <[u8; 32]>::from(hasher.finalize()) != expected_sha256 {
        return Err(CheckpointError::format(
            "NUMERIC_SHA256",
            "adaptive numeric file failed its logical SHA-256",
        ));
    }
    Ok(())
}

/// Decode one preflighted raw or shuffled numeric role into final per-owner allocations.
pub(super) fn decode_numeric_role(
    file: &mut File,
    entry: &ScannedEntry,
    encoding: NumericEncoding,
    expected_floats: usize,
    segment_count: usize,
    segment_length: usize,
    expected_sha256: [u8; 32],
) -> Result<Vec<Box<[f32]>>, CheckpointError> {
    let _phase = ArchivePhaseScope::enter(ArchivePhase::NumericDecode);
    let expected_bytes = usize_to_u64(expected_floats, "numeric float count")?
        .checked_mul(4)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "numeric decoded bytes overflow")
        })?;
    preflight_numeric_role(file, entry, encoding, expected_floats)?;
    let mut output = SegmentedF32Builder::new(segment_count, segment_length, expected_floats)?;
    let mut hasher = Sha256::new();
    file.seek(SeekFrom::Start(entry.data_offset))?;
    match encoding {
        NumericEncoding::RawF32LeV1 => {
            if entry.size != expected_bytes {
                return Err(CheckpointError::format(
                    "NUMERIC_LENGTH",
                    "raw numeric stored length disagrees with count",
                ));
            }
            let mut remaining = entry.size;
            let mut buffer = vec![
                0u8;
                SHUFFLED_BLOCK_BYTES
                    .min(usize::try_from(remaining).unwrap_or(usize::MAX))
            ];
            while remaining > 0 {
                let take = buffer
                    .len()
                    .min(u64_to_usize(remaining, "raw numeric remaining bytes")?);
                file.read_exact(&mut buffer[..take])?;
                if take % 4 != 0 {
                    return Err(CheckpointError::format(
                        "NUMERIC_LENGTH",
                        "raw numeric chunk is not Float32 aligned",
                    ));
                }
                hasher.update(&buffer[..take]);
                output.append_packed(&buffer[..take])?;
                advance_work_progress(take);
                remaining -= take as u64;
            }
        }
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
            let mut remaining = entry.size;
            let mut decoded_total = 0usize;
            let mut decompressor = ZstdDecompressor::new()?;
            decompressor.window_log_max(ZSTD_WINDOW_LOG_MAX)?;
            let mut frame = Vec::new();
            frame.try_reserve_exact(max_frame_bytes).map_err(|_| {
                CheckpointError::format(
                    "ALLOCATION",
                    "unable to reserve reusable bounded Zstandard frame",
                )
            })?;
            let mut shuffled = Vec::new();
            shuffled
                .try_reserve_exact(SHUFFLED_BLOCK_BYTES)
                .map_err(|_| {
                    CheckpointError::format(
                        "ALLOCATION",
                        "unable to reserve reusable decoded Zstandard scratch",
                    )
                })?;
            let mut raw = Vec::new();
            raw.try_reserve_exact(SHUFFLED_BLOCK_BYTES).map_err(|_| {
                CheckpointError::format(
                    "ALLOCATION",
                    "unable to reserve reusable packed numeric scratch",
                )
            })?;
            while remaining > 0 {
                if remaining < SHUFFLED_BLOCK_HEADER_BYTES as u64 {
                    return Err(CheckpointError::format(
                        "SHUFFLED_HEADER",
                        "truncated SFZ1 block header",
                    ));
                }
                let mut header = [0u8; SHUFFLED_BLOCK_HEADER_BYTES];
                file.read_exact(&mut header)?;
                remaining -= SHUFFLED_BLOCK_HEADER_BYTES as u64;
                if &header[..4] != SHUFFLED_BLOCK_MAGIC {
                    return Err(CheckpointError::format(
                        "SHUFFLED_MAGIC",
                        "unsupported SFZ1 block marker",
                    ));
                }
                let float_count = u32_to_usize(
                    u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
                    "SFZ1 float count",
                )?;
                let frame_bytes = u32_to_usize(
                    u32::from_le_bytes([header[8], header[9], header[10], header[11]]),
                    "SFZ1 frame bytes",
                )?;
                let decoded_bytes = float_count.checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 decoded block bytes overflow")
                })?;
                if float_count == 0
                    || decoded_bytes > SHUFFLED_BLOCK_BYTES
                    || frame_bytes == 0
                    || frame_bytes > max_frame_bytes
                    || frame_bytes as u64 > remaining
                {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 block declaration exceeds bounds",
                    ));
                }
                decoded_total = decoded_total.checked_add(float_count).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 aggregate count overflows")
                })?;
                if decoded_total > expected_floats {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 decoded count exceeds manifest",
                    ));
                }
                frame.clear();
                frame.resize(frame_bytes, 0);
                file.read_exact(&mut frame)?;
                remaining -= frame_bytes as u64;
                validate_zstd_frame_header(&frame, decoded_bytes)?;
                let compressed_size =
                    zstd_safe::find_frame_compressed_size(&frame).map_err(|_| {
                        CheckpointError::format(
                            "ZSTD_FRAME",
                            "unable to determine Zstandard frame boundary",
                        )
                    })?;
                if compressed_size != frame.len() {
                    return Err(CheckpointError::format(
                        "ZSTD_FRAME",
                        "SFZ1 envelope contains trailing frame bytes",
                    ));
                }
                shuffled.clear();
                let written = decompressor.decompress_to_buffer(&frame, &mut shuffled)?;
                if written != decoded_bytes || shuffled.len() != decoded_bytes {
                    return Err(CheckpointError::format(
                        "ZSTD_DECODED_LENGTH",
                        "decoded Zstandard block length mismatch",
                    ));
                }
                unshuffle_f32_bytes_into(&shuffled, &mut raw)?;
                hasher.update(&raw);
                output.append_packed(&raw)?;
                advance_work_progress(decoded_bytes);
            }
            if decoded_total != expected_floats || expected_bytes != (decoded_total as u64) * 4 {
                return Err(CheckpointError::format(
                    "NUMERIC_COUNT",
                    "SFZ1 decoded count is incomplete",
                ));
            }
        }
    }
    let actual_sha256: [u8; 32] = hasher.finalize().into();
    if actual_sha256 != expected_sha256 {
        return Err(CheckpointError::format(
            "LOGICAL_ROLE_SHA256",
            "numeric logical SHA-256 mismatch",
        ));
    }
    output.finish()
}
