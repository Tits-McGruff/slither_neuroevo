//! Strict ustar for managed checkpoints.

use super::*;

/// Append one fixed-path regular USTAR entry.
pub(super) fn append_ustar<W: Write, R: Read>(
    archive: &mut TarBuilder<W>,
    path: &'static str,
    size: u64,
    reader: R,
) -> Result<(), CheckpointError> {
    let mut header = Header::new_ustar();
    header.set_entry_type(EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(size);
    header.set_cksum();
    archive.append_data(&mut header, path, reader)?;
    Ok(())
}

/// Append one selected numeric role without materializing a second flat buffer.
pub(super) fn append_numeric_ustar<W: Write>(
    archive: &mut TarBuilder<W>,
    path: &'static str,
    candidate: &NumericCandidate,
    source: &FloatSource<'_>,
) -> Result<(), CheckpointError> {
    if candidate.scratch_capacity_growths != 0
        || (candidate.float_count != 0 && candidate.encoded_blocks == 0)
    {
        return Err(CheckpointError::format(
            "NUMERIC_CANDIDATE",
            "numeric candidate did not preserve reusable bounded scratch invariants",
        ));
    }
    match candidate.encoding {
        NumericEncoding::RawF32LeV1 => append_ustar(
            archive,
            path,
            candidate.stored_bytes,
            FloatByteReader::new(source),
        ),
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let compressed_path = candidate.compressed_path.as_ref().ok_or_else(|| {
                CheckpointError::format("NUMERIC_CANDIDATE", "compressed candidate path is missing")
            })?;
            append_ustar(
                archive,
                path,
                candidate.stored_bytes,
                BufReader::new(File::open(compressed_path)?),
            )
        }
    }
}

/// Compute the exact completed USTAR length for fixed entry sizes.
pub(super) fn expected_archive_length<const N: usize>(
    sizes: [u64; N],
) -> Result<u64, CheckpointError> {
    let mut total = USTAR_TRAILER_BYTES;
    for size in sizes {
        let padding = (USTAR_BLOCK_BYTES - (size % USTAR_BLOCK_BYTES)) % USTAR_BLOCK_BYTES;
        total = total
            .checked_add(USTAR_BLOCK_BYTES)
            .and_then(|value| value.checked_add(size))
            .and_then(|value| value.checked_add(padding))
            .ok_or_else(|| {
                CheckpointError::format("ARCHIVE_OVERFLOW", "USTAR length overflows u64")
            })?;
    }
    Ok(total)
}

/// One strictly scanned ordinary regular-file entry.
#[derive(Clone, Debug)]
pub(in crate::engine) struct ScannedEntry {
    pub(in crate::engine) name: String,
    #[cfg(test)]
    pub(super) header_offset: u64,
    pub(in crate::engine) data_offset: u64,
    pub(in crate::engine) size: u64,
}

/// Scan the complete archive structure before allocating or decoding any role.
pub(super) fn scan_strict_ustar(
    file: &mut File,
    archive_length: u64,
    limits: &CheckpointLimits,
) -> Result<Vec<ScannedEntry>, CheckpointError> {
    scan_strict_ustar_roles(file, archive_length, USTAR_ENTRY_COUNT, |name, size| {
        validate_scanned_entry_size(name, size, limits)
    })
}

/// Check the shared strict container envelope, then apply each caller's fixed role-size policy.
pub(in crate::engine) fn scan_strict_ustar_roles(
    file: &mut File,
    archive_length: u64,
    entry_count: usize,
    validate_role: impl FnMut(&str, u64) -> Result<(), CheckpointError>,
) -> Result<Vec<ScannedEntry>, CheckpointError> {
    scan_strict_ustar_role_range(
        file,
        archive_length,
        entry_count,
        entry_count,
        validate_role,
    )
}

/// Scan a bounded version-dispatched role count; each caller validates its exact manifest set.
pub(in crate::engine) fn scan_strict_ustar_role_range(
    file: &mut File,
    archive_length: u64,
    minimum_entry_count: usize,
    entry_count: usize,
    mut validate_role: impl FnMut(&str, u64) -> Result<(), CheckpointError>,
) -> Result<Vec<ScannedEntry>, CheckpointError> {
    file.seek(SeekFrom::Start(0))?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(entry_count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve fixed USTAR entry table")
    })?;
    let mut offset = 0u64;
    loop {
        let header_end = offset.checked_add(USTAR_BLOCK_BYTES).ok_or_else(|| {
            CheckpointError::format("USTAR_OVERFLOW", "USTAR header offset overflows")
        })?;
        if header_end > archive_length {
            return Err(CheckpointError::format(
                "USTAR_TRUNCATED_HEADER",
                "archive ends inside USTAR header",
            ));
        }
        let mut header = [0u8; USTAR_BLOCK_BYTES as usize];
        file.read_exact(&mut header)?;
        if header.iter().all(|byte| *byte == 0) {
            let trailer_end = offset.checked_add(USTAR_TRAILER_BYTES).ok_or_else(|| {
                CheckpointError::format("USTAR_OVERFLOW", "USTAR trailer offset overflows")
            })?;
            if trailer_end != archive_length {
                return Err(CheckpointError::format(
                    "USTAR_TRAILER",
                    "archive must end with exactly two zero blocks",
                ));
            }
            let mut second = [0u8; USTAR_BLOCK_BYTES as usize];
            file.read_exact(&mut second)?;
            if second.iter().any(|byte| *byte != 0) {
                return Err(CheckpointError::format(
                    "USTAR_TRAILER",
                    "second USTAR trailer block is nonzero",
                ));
            }
            break;
        }
        if entries.len() >= entry_count {
            return Err(CheckpointError::format(
                "USTAR_ENTRY_COUNT",
                "archive contains too many entries",
            ));
        }
        let (name, size) = parse_ustar_header(&header)?;
        if entries
            .iter()
            .any(|entry: &ScannedEntry| entry.name == name)
        {
            return Err(CheckpointError::format(
                "USTAR_DUPLICATE",
                format!("duplicate USTAR entry {name}"),
            ));
        }
        validate_role(&name, size)?;
        let data_offset = header_end;
        let padding = (USTAR_BLOCK_BYTES - (size % USTAR_BLOCK_BYTES)) % USTAR_BLOCK_BYTES;
        let next = data_offset
            .checked_add(size)
            .and_then(|value| value.checked_add(padding))
            .ok_or_else(|| {
                CheckpointError::format("USTAR_OVERFLOW", "USTAR entry offset overflows")
            })?;
        if next > archive_length {
            return Err(CheckpointError::format(
                "USTAR_TRUNCATED_ENTRY",
                format!("entry {name} exceeds archive"),
            ));
        }
        file.seek(SeekFrom::Start(data_offset + size))?;
        if padding > 0 {
            let mut pad = [0u8; USTAR_BLOCK_BYTES as usize];
            file.read_exact(&mut pad[..padding as usize])?;
            if pad[..padding as usize].iter().any(|byte| *byte != 0) {
                return Err(CheckpointError::format(
                    "USTAR_PADDING",
                    format!("entry {name} has nonzero padding"),
                ));
            }
        }
        entries.push(ScannedEntry {
            name,
            #[cfg(test)]
            header_offset: offset,
            data_offset,
            size,
        });
        offset = next;
        file.seek(SeekFrom::Start(offset))?;
    }
    if entries.len() < minimum_entry_count
        || entries.len() > entry_count
        || entries
            .last()
            .is_none_or(|entry| entry.name != MANIFEST_PATH)
    {
        return Err(CheckpointError::format(
            "USTAR_ENTRY_SET",
            "archive must contain its fixed role set followed by manifest.json",
        ));
    }
    Ok(entries)
}

/// Parse the retained strict USTAR v1 header subset and verify its standard checksum.
pub(super) fn parse_ustar_header(header: &[u8; 512]) -> Result<(String, u64), CheckpointError> {
    let expected_checksum = parse_ustar_octal(&header[148..156], "checksum")?;
    let actual_checksum = header.iter().enumerate().fold(0u64, |sum, (index, byte)| {
        sum + if (148..156).contains(&index) {
            0x20
        } else {
            u64::from(*byte)
        }
    });
    if expected_checksum != actual_checksum {
        return Err(CheckpointError::format(
            "USTAR_CHECKSUM",
            "USTAR header checksum mismatch",
        ));
    }
    if &header[257..263] != b"ustar\0" || &header[263..265] != b"00" {
        return Err(CheckpointError::format(
            "USTAR_VERSION",
            "missing USTAR magic/version",
        ));
    }
    if header[156] != b'0' {
        return Err(CheckpointError::format(
            "USTAR_ENTRY_TYPE",
            "only explicit regular-file entries are supported",
        ));
    }
    if header[157..257].iter().any(|byte| *byte != 0)
        || header[329..345].iter().any(|byte| *byte != 0)
        || header[345..500].iter().any(|byte| *byte != 0)
        || header[500..512].iter().any(|byte| *byte != 0)
    {
        return Err(CheckpointError::format(
            "USTAR_UNSUPPORTED",
            "links, devices, prefixes, and extension/reserved fields are unsupported",
        ));
    }
    let name_end = header[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    if header[name_end..100].iter().any(|byte| *byte != 0) {
        return Err(CheckpointError::format(
            "USTAR_PATH",
            "USTAR name has hidden bytes after NUL",
        ));
    }
    let name_bytes = &header[..name_end];
    if name_bytes.is_empty() || name_bytes.iter().any(|byte| !(0x20..=0x7e).contains(byte)) {
        return Err(CheckpointError::format(
            "USTAR_PATH",
            "USTAR entry path is not printable ASCII",
        ));
    }
    let name = std::str::from_utf8(name_bytes)
        .map_err(|_| CheckpointError::format("USTAR_PATH", "USTAR entry path is not UTF-8"))?
        .to_owned();
    let size = parse_ustar_octal(&header[124..136], "size")?;
    Ok((name, size))
}

/// Parse one strict trailing-NUL/space USTAR octal field.
pub(super) fn parse_ustar_octal(field: &[u8], label: &'static str) -> Result<u64, CheckpointError> {
    let end = field
        .iter()
        .rposition(|byte| !matches!(byte, 0 | b' '))
        .map_or(0, |index| index + 1);
    let digits = &field[..end];
    if digits.is_empty() || digits.iter().any(|byte| !(b'0'..=b'7').contains(byte)) {
        return Err(CheckpointError::format(
            "USTAR_OCTAL",
            format!("invalid {label} octal field"),
        ));
    }
    let mut value = 0u64;
    for digit in digits {
        value = value
            .checked_mul(8)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or_else(|| {
                CheckpointError::format("USTAR_OCTAL", format!("{label} octal field overflows"))
            })?;
    }
    Ok(value)
}

/// Accept only the fixed role paths and reject all generic path semantics.
pub(super) fn validate_fixed_safe_path(path: &str) -> Result<(), CheckpointError> {
    let allowed = [
        STATE_PATH,
        GRAPH_PATH,
        POPULATION_INDEX_PATH,
        WEIGHTS_RAW_PATH,
        WEIGHTS_ZSTD_PATH,
        RECURRENT_RAW_PATH,
        RECURRENT_ZSTD_PATH,
        MANIFEST_PATH,
    ];
    if !allowed.contains(&path)
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(CheckpointError::format(
            "USTAR_PATH",
            format!("unsupported or unsafe fixed role path {path:?}"),
        ));
    }
    Ok(())
}

/// Apply path-specific stored-byte limits during the structural scan.
pub(in crate::engine) fn validate_scanned_entry_size(
    path: &str,
    size: u64,
    limits: &CheckpointLimits,
) -> Result<(), CheckpointError> {
    validate_fixed_safe_path(path)?;
    let limit = match path {
        STATE_PATH => limits.max_state_bytes as u64,
        GRAPH_PATH => limits.max_graph_bytes as u64,
        POPULATION_INDEX_PATH => limits.max_population_index_bytes as u64,
        WEIGHTS_RAW_PATH | WEIGHTS_ZSTD_PATH | RECURRENT_RAW_PATH | RECURRENT_ZSTD_PATH => {
            limits.max_numeric_stored_bytes
        }
        MANIFEST_PATH => limits.max_manifest_bytes as u64,
        _ => {
            return Err(CheckpointError::format(
                "USTAR_PATH",
                "unknown fixed role path",
            ))
        }
    };
    if size > limit {
        return Err(CheckpointError::format(
            "ENTRY_LIMIT",
            format!("entry {path} bytes {size} exceed limit {limit}"),
        ));
    }
    Ok(())
}

/// Read one already-size-bounded entry into memory.
pub(super) fn read_entry_bytes(
    file: &mut File,
    entry: &ScannedEntry,
    limit: usize,
) -> Result<Vec<u8>, CheckpointError> {
    let size = u64_to_usize(entry.size, "entry byte length")?;
    if size > limit {
        return Err(CheckpointError::format(
            "ENTRY_LIMIT",
            "entry exceeds role byte limit",
        ));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve bounded entry bytes")
    })?;
    bytes.resize(size, 0);
    file.seek(SeekFrom::Start(entry.data_offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
