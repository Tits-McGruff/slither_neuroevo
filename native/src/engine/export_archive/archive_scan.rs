//! Bounded read-only archive scanning and manifest validation before extraction/publication.

use super::*;

/// Inspect at most 111 USTAR headers and one small final manifest before any
/// decoded import file is staged. Full integrity validation still follows.
pub fn estimate_import_disk_bytes(path: &Path) -> Result<ImportDiskEstimate, CheckpointError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(CheckpointError::format(
            "IMPORT_DISK_ESTIMATE",
            "upload is not a nonempty regular file",
        ));
    }
    if upload_looks_like_legacy_json(path)? {
        if metadata.len() > MAX_LEGACY_JSON_BYTES {
            return Err(CheckpointError::format(
                "IMPORT_ARCHIVE_LIMIT",
                "legacy JSON exceeds 512 MiB",
            ));
        }
        // Legacy JSON builds one managed checkpoint; count both its private
        // partial and final publication even when rename reuses the blocks.
        let allowance = 528 * 1024 * 1024;
        return Ok(ImportDiskEstimate {
            candidate_spool_bytes: allowance,
            final_managed_bytes: allowance,
        });
    }
    if metadata.len() > MAX_EXPORT_ARCHIVE_BYTES {
        return Err(CheckpointError::format(
            "IMPORT_ARCHIVE_LIMIT",
            "save exceeds four GiB",
        ));
    }
    let mut file = File::open(path)?;
    let mut cursor = 0u64;
    let mut manifest_bytes = None;
    for _ordinal in 0..(11 + MAX_SAVED_BRAINS) {
        let header_end = cursor.checked_add(USTAR_BLOCK_BYTES).ok_or_else(|| {
            CheckpointError::format("IMPORT_DISK_ESTIMATE", "archive header offset overflowed")
        })?;
        if header_end > metadata.len() {
            return Err(CheckpointError::format(
                "IMPORT_DISK_ESTIMATE",
                "archive header is truncated",
            ));
        }
        file.seek(SeekFrom::Start(cursor))?;
        let mut header = [0u8; USTAR_BLOCK_BYTES as usize];
        file.read_exact(&mut header)?;
        if &header[257..263] != b"ustar\0" {
            return Err(CheckpointError::format(
                "IMPORT_DISK_ESTIMATE",
                "save requires USTAR headers",
            ));
        }
        let size = parse_ustar_entry_size(&header[124..136])?;
        let padded = size
            .checked_add(USTAR_BLOCK_BYTES - 1)
            .and_then(|value| value.checked_div(USTAR_BLOCK_BYTES))
            .and_then(|blocks| blocks.checked_mul(USTAR_BLOCK_BYTES))
            .ok_or_else(|| {
                CheckpointError::format("IMPORT_DISK_ESTIMATE", "archive entry length overflowed")
            })?;
        let next = header_end.checked_add(padded).ok_or_else(|| {
            CheckpointError::format("IMPORT_DISK_ESTIMATE", "archive entry offset overflowed")
        })?;
        if next > metadata.len() {
            return Err(CheckpointError::format(
                "IMPORT_DISK_ESTIMATE",
                "archive entry is truncated",
            ));
        }
        let name = header[..100]
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        if name == MANIFEST_PATH.as_bytes() {
            let name = header[..100]
                .split(|byte| *byte == 0)
                .next()
                .unwrap_or_default();
            if name != MANIFEST_PATH.as_bytes() || size == 0 || size > 1024 * 1024 {
                return Err(CheckpointError::format(
                    "IMPORT_DISK_ESTIMATE",
                    "final save manifest is missing or too large",
                ));
            }
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size as usize).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve bounded save manifest")
            })?;
            bytes.resize(size as usize, 0);
            file.read_exact(&mut bytes)?;
            manifest_bytes = Some(bytes);
            break;
        }
        cursor = next;
    }
    let manifest: SaveManifest = serde_json::from_slice(&manifest_bytes.ok_or_else(|| {
        CheckpointError::format("IMPORT_DISK_ESTIMATE", "save manifest is missing")
    })?)
    .map_err(|error| CheckpointError::format("IMPORT_MANIFEST_JSON", error.to_string()))?;
    if manifest.magic != "slither-neuroevo-save"
        || manifest.roles.len() != manifest.base_role_count()? + manifest.saved_brains.len()
        || manifest.roles[7].role != "hall-of-fame-weights"
    {
        return Err(CheckpointError::format(
            "IMPORT_DISK_ESTIMATE",
            "save manifest has unsupported roles",
        ));
    }
    let hof_decoded = parse_hex_u64(
        &manifest.roles[7].decoded_bytes_hex,
        "Hall-of-Fame decoded bytes",
    )?;
    let hof_count = parse_hex_u64(&manifest.hall_of_fame_count_hex, "Hall-of-Fame count")?;
    if hof_decoded > MAX_EXPORT_ARCHIVE_BYTES || hof_count > MAX_EXPORT_GENERATIONS {
        return Err(CheckpointError::format(
            "IMPORT_DISK_ESTIMATE",
            "save exceeds Hall-of-Fame limits",
        ));
    }
    // Checkpoint reassembly and the fixed inventory fit within the archive's
    // stored roles plus one MiB. The raw HoF work file adds decoded bytes;
    // independently published objects add at most their raw bytes plus one
    // filesystem block each. Count one extra object partial at peak.
    let checkpoint_and_inventory = metadata.len().checked_add(1024 * 1024).ok_or_else(|| {
        CheckpointError::format("IMPORT_DISK_ESTIMATE", "checkpoint allowance overflowed")
    })?;
    let saved_numeric_bytes = (manifest.saved_brains.len() as u64)
        .checked_mul(crate::contracts::MAX_BRAIN_WEIGHT_BYTES + 4096)
        .ok_or_else(|| saved_brain_archive::invalid("saved numeric allowance overflow"))?;
    let objects = hof_decoded
        .checked_add(saved_numeric_bytes)
        .ok_or_else(|| saved_brain_archive::invalid("saved object allowance overflow"))?
        .checked_add((hof_count + 1).checked_mul(4096).ok_or_else(|| {
            CheckpointError::format("IMPORT_DISK_ESTIMATE", "object allowance overflowed")
        })?)
        .ok_or_else(|| CheckpointError::format("IMPORT_DISK_ESTIMATE", "object size overflowed"))?;
    let final_managed_bytes = checkpoint_and_inventory
        .checked_add(objects)
        .ok_or_else(|| {
            CheckpointError::format("IMPORT_DISK_ESTIMATE", "final managed allowance overflowed")
        })?;
    let candidate_spool_bytes = final_managed_bytes
        .checked_add(hof_decoded)
        .and_then(|bytes| {
            bytes.checked_add(if manifest.saved_brains.is_empty() {
                0
            } else {
                crate::contracts::MAX_BRAIN_NUMERIC_CANDIDATE_BYTES
            })
        })
        .ok_or_else(|| {
            CheckpointError::format("IMPORT_DISK_ESTIMATE", "candidate allowance overflowed")
        })?;
    Ok(ImportDiskEstimate {
        candidate_spool_bytes,
        final_managed_bytes,
    })
}

pub(super) fn parse_ustar_entry_size(field: &[u8]) -> Result<u64, CheckpointError> {
    let digits = field
        .iter()
        .copied()
        .skip_while(|byte| *byte == b' ')
        .take_while(|byte| *byte != 0 && *byte != b' ')
        .collect::<Vec<_>>();
    if digits.is_empty() || !digits.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
        return Err(CheckpointError::format(
            "IMPORT_DISK_ESTIMATE",
            "USTAR entry size is invalid",
        ));
    }
    let value = std::str::from_utf8(&digits).map_err(|_| {
        CheckpointError::format("IMPORT_DISK_ESTIMATE", "USTAR entry size is not ASCII")
    })?;
    u64::from_str_radix(value, 8)
        .map_err(|_| CheckpointError::format("IMPORT_DISK_ESTIMATE", "USTAR entry size overflowed"))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SaveManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) legacy_conversion: Option<LegacyPopulationOrigin>,
    pub(super) magic: String,
    pub(super) archive_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) records_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) saved_brains: Vec<SavedBrainArchiveEntryV1>,
    pub(super) archive_kind: String,
    pub(super) run_id: String,
    pub(super) generation_hex: String,
    pub(super) completed_step_hex: String,
    pub(super) checkpoint_logical_root_sha256: String,
    pub(super) logical_root_sha256: String,
    pub(super) history_count_hex: String,
    pub(super) hall_of_fame_count_hex: String,
    pub(super) hall_of_fame_weight_count_hex: String,
    pub(super) checkpoint_manifest: CheckpointManifest,
    pub(super) roles: Vec<SaveRole>,
}

impl SaveManifest {
    /// Outer v4 adds explicit originals while preserving fixed inventory versions 1..3.
    pub(super) fn records_version(&self) -> Result<u32, CheckpointError> {
        let version = match (self.archive_version, self.records_version) {
            (1..=3, None) if self.saved_brains.is_empty() => self.archive_version,
            (4, Some(version @ 1..=3)) if !self.saved_brains.is_empty() => version,
            _ => {
                return Err(saved_brain_archive::invalid(
                    "unsupported save/record version pairing",
                ))
            }
        };
        saved_brain_archive::validate_selection(&self.saved_brains)?;
        Ok(version)
    }

    pub(super) fn base_role_count(&self) -> Result<usize, CheckpointError> {
        let version = self.records_version()?;
        Ok(8 + usize::from(version >= 2) + usize::from(version == 3))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SaveRole {
    pub(super) role: String,
    pub(super) path: String,
    pub(super) encoding: String,
    pub(super) stored_bytes_hex: String,
    pub(super) decoded_bytes_hex: String,
    pub(super) decoded_count_hex: String,
    pub(super) record_size: u32,
    pub(super) logical_sha256: String,
}

pub(super) struct ScannedImportArchive {
    pub(super) manifest: SaveManifest,
    pub(super) entry_paths: Vec<String>,
    pub(super) entry_sizes: Vec<u64>,
    pub(super) entry_hashes: Vec<[u8; 32]>,
}

pub(super) fn scan_import_archive(
    path: &Path,
    checkpoint_limits: &CheckpointLimits,
) -> Result<ScannedImportArchive, CheckpointError> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let entries = crate::engine::checkpoint::scan_strict_ustar_role_range(
        &mut file,
        length,
        9,
        11 + MAX_SAVED_BRAINS,
        |name, size| {
            let limit = match name {
                HISTORY_PATH => MAX_EXPORT_GENERATIONS * HISTORY_RECORD_BYTES,
                COMPLETION_PATH => MAX_EXPORT_GENERATIONS * COMPLETION_RECORD_BYTES,
                EXPERIMENT_PATH => MAX_EXPORT_GENERATIONS * EXPERIMENT_RECORD_BYTES,
                HOF_INDEX_PATH => MAX_EXPORT_GENERATIONS * HOF_RECORD_BYTES,
                HOF_WEIGHTS_PATH | HOF_WEIGHTS_ZSTD_PATH => MAX_EXPORT_ARCHIVE_BYTES,
                MANIFEST_PATH => 1024 * 1024,
                _ if name.starts_with("brains/") => {
                    crate::engine::brain_artifact::MAX_BRAIN_ARTIFACT_BYTES as u64
                }
                _ => return validate_scanned_entry_size(name, size, checkpoint_limits),
            };
            if size > limit {
                return Err(CheckpointError::format(
                    "IMPORT_ENTRY_LIMIT",
                    format!("save entry {name} exceeds its stored limit"),
                ));
            }
            Ok(())
        },
    )?;
    let mut sizes = Vec::new();
    sizes
        .try_reserve_exact(11 + MAX_SAVED_BRAINS)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve save entry-size table")
        })?;
    let mut hashes = Vec::new();
    hashes
        .try_reserve_exact(10 + MAX_SAVED_BRAINS)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve save entry-hash table")
        })?;
    let mut paths = Vec::new();
    paths
        .try_reserve_exact(11 + MAX_SAVED_BRAINS)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve save entry-path table")
        })?;
    let mut manifest = None;
    for (seen, extent) in entries.iter().enumerate() {
        file.seek(SeekFrom::Start(extent.data_offset))?;
        let mut entry = (&mut file).take(extent.size);
        let size = extent.size;
        paths.push(extent.name.clone());
        sizes.push(size);
        if seen + 1 < entries.len() {
            let mut hasher = Sha256::new();
            let copied = io::copy(&mut entry, &mut HashWriter(&mut hasher))?;
            if copied != size {
                return Err(CheckpointError::format(
                    "IMPORT_USTAR",
                    "save entry ended before its declared length",
                ));
            }
            hashes.push(hasher.finalize().into());
        } else {
            if extent.name != MANIFEST_PATH {
                return Err(CheckpointError::format(
                    "IMPORT_USTAR",
                    "save manifest must be the final entry",
                ));
            }
            if size == 0 || size > 1024 * 1024 {
                return Err(CheckpointError::format(
                    "IMPORT_MANIFEST_LIMIT",
                    "save manifest is empty or exceeds one MiB",
                ));
            }
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size as usize).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve bounded save manifest")
            })?;
            entry.read_to_end(&mut bytes)?;
            manifest = Some(serde_json::from_slice(&bytes).map_err(|error| {
                CheckpointError::format(
                    "IMPORT_MANIFEST_JSON",
                    format!("save manifest is invalid: {error}"),
                )
            })?);
        }
    }
    Ok(ScannedImportArchive {
        manifest: manifest.ok_or_else(|| {
            CheckpointError::format("IMPORT_MANIFEST", "save manifest is missing")
        })?,
        entry_paths: paths,
        entry_sizes: sizes,
        entry_hashes: hashes,
    })
}

pub(super) fn validate_import_manifest(
    manifest: &SaveManifest,
    entry_paths: &[String],
    entry_sizes: &[u64],
    entry_hashes: &[[u8; 32]],
    archive_bytes: u64,
) -> Result<(), CheckpointError> {
    if let Some(origin) = &manifest.legacy_conversion {
        origin
            .validate()
            .map_err(|error| CheckpointError::format("LEGACY_ORIGIN", error))?;
    }
    let records_version = manifest.records_version()?;
    let base_roles = manifest.base_role_count()?;
    if manifest.magic != "slither-neuroevo-save"
        || manifest.archive_kind
            != if manifest.legacy_conversion.is_some() {
                "legacy-population-import"
            } else {
                "exact-generation-boundary-v1"
            }
        || manifest.run_id.is_empty()
        || manifest.run_id.contains('\0')
        || manifest.run_id.len() > 256
        || manifest.roles.len() != base_roles + manifest.saved_brains.len()
        || manifest.checkpoint_manifest.roles.len() != 5
        || entry_paths.len() != manifest.roles.len() + 1
        || entry_sizes.len() != manifest.roles.len() + 1
        || entry_hashes.len() != manifest.roles.len()
        || entry_paths.last().is_none_or(|path| path != MANIFEST_PATH)
        || entry_paths
            .iter()
            .enumerate()
            .any(|(index, path)| entry_paths[..index].iter().any(|previous| previous == path))
    {
        return Err(CheckpointError::format(
            "IMPORT_MANIFEST",
            "save manifest identity, run ID, or role count is invalid",
        ));
    }
    let generation = parse_hex_u64(&manifest.generation_hex, "save generation")?;
    parse_hex_u64(&manifest.completed_step_hex, "save completed step")?;
    parse_digest(
        &manifest.checkpoint_logical_root_sha256,
        "save checkpoint root",
    )?;
    parse_digest(&manifest.logical_root_sha256, "save logical root")?;
    if manifest.checkpoint_manifest.magic != "slither-neuroevo-checkpoint"
        || manifest.checkpoint_manifest.archive_kind != "managed-checkpoint-v3"
        || manifest.checkpoint_manifest.run_id != manifest.run_id
        || manifest.checkpoint_manifest.generation_hex != manifest.generation_hex
        || manifest.checkpoint_manifest.completed_step_hex != manifest.completed_step_hex
        || manifest.checkpoint_manifest.logical_root_sha256
            != manifest.checkpoint_logical_root_sha256
    {
        return Err(CheckpointError::format(
            "IMPORT_CHECKPOINT_MANIFEST",
            "flattened checkpoint manifest identity disagrees with the save",
        ));
    }
    let history_count = parse_hex_u64(&manifest.history_count_hex, "save history count")?;
    let hall_of_fame_count =
        parse_hex_u64(&manifest.hall_of_fame_count_hex, "save Hall-of-Fame count")?;
    let hall_of_fame_weight_count = parse_hex_u64(
        &manifest.hall_of_fame_weight_count_hex,
        "save Hall-of-Fame weight count",
    )?;
    if generation == 0
        || history_count != generation - 1
        || hall_of_fame_count > history_count
        || history_count > MAX_EXPORT_GENERATIONS
    {
        return Err(CheckpointError::format(
            "IMPORT_COVERAGE",
            "save history and Hall-of-Fame coverage is inconsistent",
        ));
    }
    for (index, checkpoint_role) in manifest.checkpoint_manifest.roles.iter().enumerate() {
        let role = &manifest.roles[index];
        let stored = parse_hex_u64(&role.stored_bytes_hex, "checkpoint role stored bytes")?;
        if role.role != checkpoint_role.role
            || role.path != checkpoint_role.path
            || role.encoding != checkpoint_role.encoding
            || role.stored_bytes_hex != checkpoint_role.stored_bytes_hex
            || role.decoded_bytes_hex != checkpoint_role.decoded_bytes_hex
            || role.decoded_count_hex != checkpoint_role.decoded_count_hex
            || role.record_size != checkpoint_role.record_size
            || role.logical_sha256 != checkpoint_role.logical_sha256
            || entry_paths[index] != role.path
            || entry_sizes[index] != stored
        {
            return Err(CheckpointError::format(
                "IMPORT_CHECKPOINT_ROLE",
                "flattened checkpoint role disagrees with its validated manifest",
            ));
        }
        let logical_sha256 = parse_digest(&role.logical_sha256, "checkpoint role logical SHA-256")?;
        if matches!(role.encoding.as_str(), "raw-binary-v1" | "raw-f32le-v1")
            && logical_sha256 != entry_hashes[index]
        {
            return Err(CheckpointError::format(
                "IMPORT_CHECKPOINT_ROLE",
                "raw flattened checkpoint role failed its logical SHA-256",
            ));
        }
    }
    let expected = [
        (
            "history",
            HISTORY_PATH,
            "raw-history-v1",
            history_count,
            HISTORY_RECORD_BYTES as u32,
        ),
        (
            "hall-of-fame-index",
            HOF_INDEX_PATH,
            "raw-hof-index-v1",
            hall_of_fame_count,
            HOF_RECORD_BYTES as u32,
        ),
    ];
    for (relative_index, role) in manifest.roles[5..7].iter().enumerate() {
        let index = relative_index + 5;
        let (name, path, encoding, count, record_size) = expected[relative_index];
        let stored = parse_hex_u64(&role.stored_bytes_hex, "role stored bytes")?;
        let decoded = parse_hex_u64(&role.decoded_bytes_hex, "role decoded bytes")?;
        let declared_count = parse_hex_u64(&role.decoded_count_hex, "role decoded count")?;
        if role.role != name
            || role.path != path
            || role.encoding != encoding
            || role.record_size != record_size
            || entry_paths[index] != path
            || stored != entry_sizes[index]
            || decoded != stored
            || declared_count != count
            || parse_digest(&role.logical_sha256, "role logical SHA-256")? != entry_hashes[index]
        {
            return Err(CheckpointError::format(
                "IMPORT_ROLE",
                format!("save role {name} disagrees with its entry or manifest contract"),
            ));
        }
    }
    if records_version >= 2 {
        let role = &manifest.roles[8];
        let size = history_count
            .checked_mul(COMPLETION_RECORD_BYTES)
            .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "completion role overflow"))?;
        if role.role != "generation-completion"
            || role.path != COMPLETION_PATH
            || role.encoding != "raw-generation-completion-v1"
            || role.record_size != COMPLETION_RECORD_BYTES as u32
            || entry_paths[8] != COMPLETION_PATH
            || entry_sizes[8] != size
            || parse_hex_u64(&role.stored_bytes_hex, "completion stored bytes")? != size
            || parse_hex_u64(&role.decoded_bytes_hex, "completion decoded bytes")? != size
            || parse_hex_u64(&role.decoded_count_hex, "completion count")? != history_count
            || parse_digest(&role.logical_sha256, "completion digest")? != entry_hashes[8]
        {
            return Err(CheckpointError::format(
                "IMPORT_COMPLETION",
                "completion role disagrees with manifest",
            ));
        }
    }
    if records_version == 3 {
        let role = &manifest.roles[9];
        let size = history_count
            .checked_mul(EXPERIMENT_RECORD_BYTES)
            .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "experiment role overflow"))?;
        if role.role != "generation-experiments"
            || role.path != EXPERIMENT_PATH
            || role.encoding != "raw-generation-experiment-v1"
            || role.record_size != EXPERIMENT_RECORD_BYTES as u32
            || entry_paths[9] != EXPERIMENT_PATH
            || entry_sizes[9] != size
            || parse_hex_u64(&role.stored_bytes_hex, "experiment stored bytes")? != size
            || parse_hex_u64(&role.decoded_bytes_hex, "experiment decoded bytes")? != size
            || parse_hex_u64(&role.decoded_count_hex, "experiment count")? != history_count
            || parse_digest(&role.logical_sha256, "experiment digest")? != entry_hashes[9]
        {
            return Err(CheckpointError::format(
                "IMPORT_EXPERIMENT",
                "experiment role disagrees with manifest",
            ));
        }
    }
    let hof_role = &manifest.roles[7];
    let hof_encoding = NumericEncoding::parse(&hof_role.encoding)?;
    let hof_path = match hof_encoding {
        NumericEncoding::RawF32LeV1 => HOF_WEIGHTS_PATH,
        NumericEncoding::F32LeShuffle4ZstdV1 => HOF_WEIGHTS_ZSTD_PATH,
    };
    let hof_stored = parse_hex_u64(&hof_role.stored_bytes_hex, "Hall-of-Fame stored bytes")?;
    let hof_decoded = parse_hex_u64(&hof_role.decoded_bytes_hex, "Hall-of-Fame decoded bytes")?;
    if hof_role.role != "hall-of-fame-weights"
        || hof_role.path != hof_path
        || entry_paths[7] != hof_path
        || hof_stored != entry_sizes[7]
        || hof_decoded != hall_of_fame_weight_count.saturating_mul(4)
        || parse_hex_u64(&hof_role.decoded_count_hex, "Hall-of-Fame decoded count")?
            != hall_of_fame_weight_count
        || hof_role.record_size != 4
        || (hof_encoding == NumericEncoding::RawF32LeV1
            && (hof_stored != hof_decoded
                || parse_digest(&hof_role.logical_sha256, "Hall-of-Fame logical SHA-256")?
                    != entry_hashes[7]))
    {
        return Err(CheckpointError::format(
            "IMPORT_ROLE",
            "save Hall-of-Fame weights disagree with the adaptive role contract",
        ));
    }
    for (relative, entry) in manifest.saved_brains.iter().enumerate() {
        let index = base_roles + relative;
        let role = &manifest.roles[index];
        let bytes = entry.bytes()?;
        if role.role != "saved-brain"
            || role.path != entry.path()
            || role.encoding != "portable-slither-brain-v1"
            || role.record_size != 0
            || entry_paths[index] != entry.path()
            || entry_sizes[index] != bytes
            || parse_hex_u64(&role.stored_bytes_hex, "saved brain stored bytes")? != bytes
            || parse_hex_u64(&role.decoded_bytes_hex, "saved brain decoded bytes")? != bytes
            || parse_hex_u64(&role.decoded_count_hex, "saved brain count")? != 1
            || role.logical_sha256 != entry.brain_id
            || parse_digest(&entry.brain_id, "saved brain identity")? != entry_hashes[index]
        {
            return Err(saved_brain_archive::invalid(
                "saved brain role disagrees with its original",
            ));
        }
    }
    if entry_sizes[5] != history_count.saturating_mul(HISTORY_RECORD_BYTES)
        || entry_sizes[6] != hall_of_fame_count.saturating_mul(HOF_RECORD_BYTES)
        || hex_digest(save_logical_root(
            &manifest.roles,
            &manifest.saved_brains,
            records_version,
        )?) != manifest.logical_root_sha256
        || expected_archive_length(entry_sizes)? != archive_bytes
    {
        return Err(CheckpointError::format(
            "IMPORT_MANIFEST",
            "save aggregate lengths or logical root are invalid",
        ));
    }
    Ok(())
}

/// Locate one role in the previously validated, closed-order raw USTAR stream.
pub(super) fn save_role_offset(
    manifest: &SaveManifest,
    ordinal: usize,
) -> Result<u64, CheckpointError> {
    let mut offset = USTAR_BLOCK_BYTES;
    for role in manifest.roles.iter().take(ordinal) {
        let size = parse_hex_u64(&role.stored_bytes_hex, "role extent")?;
        let padded = size
            .checked_add(USTAR_BLOCK_BYTES - 1)
            .map(|n| n / USTAR_BLOCK_BYTES * USTAR_BLOCK_BYTES)
            .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "role extent overflow"))?;
        offset = offset
            .checked_add(USTAR_BLOCK_BYTES)
            .and_then(|n| n.checked_add(padded))
            .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "role offset overflow"))?;
    }
    Ok(offset)
}
