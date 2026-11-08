//! Archive byte/file primitives; the caller retains scratch and publication lifetimes.

use super::*;

pub(super) fn direct_file(
    directory: &Path,
    relative_filename: &str,
    expected_bytes: u64,
    label: &str,
) -> Result<PathBuf, CheckpointError> {
    if relative_filename.is_empty()
        || relative_filename.contains(['/', '\\', '\0'])
        || relative_filename == "."
        || relative_filename == ".."
    {
        return Err(CheckpointError::format(
            "EXPORT_PATH",
            format!("{label} filename is not a controlled direct child"),
        ));
    }
    let path = directory.join(relative_filename);
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() != expected_bytes
    {
        return Err(CheckpointError::format(
            "EXPORT_FILE",
            format!("{label} is not the expected regular file"),
        ));
    }
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn save_role(
    role: &str,
    path: &str,
    encoding: &str,
    stored_bytes: u64,
    decoded_bytes: u64,
    decoded_count: u64,
    record_size: u32,
    logical_sha256: [u8; 32],
) -> SaveRole {
    SaveRole {
        role: role.to_owned(),
        path: path.to_owned(),
        encoding: encoding.to_owned(),
        stored_bytes_hex: hex_u64(stored_bytes),
        decoded_bytes_hex: hex_u64(decoded_bytes),
        decoded_count_hex: hex_u64(decoded_count),
        record_size,
        logical_sha256: hex_digest(logical_sha256),
    }
}

/// Version-four roots bind catalog labels/timestamps and base inventory version as well as role bytes.
pub(super) fn save_logical_root(
    roles: &[SaveRole],
    entries: &[SavedBrainArchiveEntryV1],
    records_version: u32,
) -> Result<[u8; 32], CheckpointError> {
    let base = logical_root(roles)?;
    if entries.is_empty() {
        return Ok(base);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"slither-neuroevo-save-root\0v4\0");
    hasher.update(base);
    hasher.update(records_version.to_le_bytes());
    hasher.update(
        serde_json::to_vec(entries)
            .map_err(|error| saved_brain_archive::invalid(error.to_string()))?,
    );
    Ok(hasher.finalize().into())
}

pub(super) fn logical_root(roles: &[SaveRole]) -> Result<[u8; 32], CheckpointError> {
    let mut hasher = Sha256::new();
    hasher.update(SAVE_ROOT_DOMAIN);
    hasher.update(
        u32::try_from(roles.len())
            .map_err(|_| CheckpointError::format("COUNT_OVERFLOW", "save role count overflowed"))?
            .to_le_bytes(),
    );
    for role in roles {
        let role_bytes = role.role.as_bytes();
        hasher.update(
            u16::try_from(role_bytes.len())
                .map_err(|_| CheckpointError::format("EXPORT_ROLE", "save role name is too long"))?
                .to_le_bytes(),
        );
        hasher.update(role_bytes);
        hasher.update(parse_hex_u64(&role.decoded_bytes_hex, "role decoded bytes")?.to_le_bytes());
        hasher.update(parse_digest(&role.logical_sha256, "role SHA-256")?);
    }
    Ok(hasher.finalize().into())
}

pub(super) fn append_file<W: Write>(
    archive: &mut TarBuilder<W>,
    path: &str,
    source: &Path,
    offset: u64,
    size: u64,
) -> Result<(), CheckpointError> {
    let mut file = BufReader::new(File::open(source)?);
    file.seek(SeekFrom::Start(offset))?;
    append_reader(archive, path, size, file.take(size))
}

pub(super) fn append_bytes<W: Write>(
    archive: &mut TarBuilder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), CheckpointError> {
    append_reader(archive, path, bytes.len() as u64, io::Cursor::new(bytes))
}

pub(super) fn append_reader<W: Write, R: Read>(
    archive: &mut TarBuilder<W>,
    path: &str,
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
    archive.append_data(&mut header, path, ProgressReader(reader))?;
    Ok(())
}

pub(super) fn expected_archive_length(sizes: &[u64]) -> Result<u64, CheckpointError> {
    sizes.iter().try_fold(USTAR_TRAILER_BYTES, |total, size| {
        let padding = (USTAR_BLOCK_BYTES - (size % USTAR_BLOCK_BYTES)) % USTAR_BLOCK_BYTES;
        total
            .checked_add(USTAR_BLOCK_BYTES)
            .and_then(|value| value.checked_add(*size))
            .and_then(|value| value.checked_add(padding))
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "save archive length overflowed")
            })
    })
}

pub(super) struct HashWriter<'a>(pub(super) &'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        advance_work_progress(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn hash_file_range(
    path: &Path,
    offset: u64,
    length: u64,
) -> Result<[u8; 32], CheckpointError> {
    let mut file = BufReader::new(File::open(path)?);
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = file.take(length);
    let mut hasher = Sha256::new();
    let copied = io::copy(&mut reader, &mut HashWriter(&mut hasher))?;
    if copied != length {
        return Err(CheckpointError::format(
            "EXPORT_READ",
            "managed source ended before its declared length",
        ));
    }
    Ok(hasher.finalize().into())
}

pub(super) fn validate_operation_id(value: &str) -> Result<(), CheckpointError> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "EXPORT_OPERATION",
            "export operation ID must be 32 lowercase hexadecimal digits",
        ));
    }
    Ok(())
}

pub(super) fn parse_hex_u64(value: &str, label: &str) -> Result<u64, CheckpointError> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "EXPORT_U64",
            format!("{label} is not canonical u64 hexadecimal"),
        ));
    }
    u64::from_str_radix(value, 16)
        .map_err(|_| CheckpointError::format("EXPORT_U64", format!("{label} is out of range")))
}

pub(super) fn parse_digest(value: &str, label: &str) -> Result<[u8; 32], CheckpointError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "EXPORT_SHA256",
            format!("{label} is not canonical SHA-256 hexadecimal"),
        ));
    }
    let mut digest = [0u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| CheckpointError::format("EXPORT_SHA256", format!("{label} is invalid")))?;
    }
    Ok(digest)
}

pub(super) fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

pub(super) fn hex_u64(value: u64) -> String {
    format!("{value:016x}")
}

pub(super) fn hex_digest(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
