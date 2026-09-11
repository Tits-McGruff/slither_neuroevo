//! Bounded self-contained save assembly from one exact leased checkpoint.
//!
//! SQLite stays outside Rust's archive codec. Its worker publishes one fixed-width
//! inventory beside immutable checkpoint and Hall-of-Fame files; this module
//! validates those files, fully decodes every referenced numeric object, and
//! writes one ordinary USTAR download without copying population data through Node.

use super::checkpoint::{
    read_validated_hall_of_fame_weights, rename_noreplace, restore_committed_checkpoint,
    sync_parent_directory, CheckpointDescriptor, CheckpointError, CheckpointLimits,
    HallOfFameWeightsDescriptor, NumericEncoding,
};
use super::graph::GraphLimits;
use super::state::StateAdmissionPolicy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tar::{Archive as TarArchive, Builder as TarBuilder, EntryType, Header};

const INVENTORY_MAGIC: &[u8; 13] = b"SLITHER-EXPV1";
const INVENTORY_HEADER_BYTES: u64 = 32;
const HISTORY_RECORD_BYTES: u64 = 56;
const HOF_RECORD_BYTES: u64 = 120;
const MAX_EXPORT_GENERATIONS: u64 = 1_000_000;
const MAX_EXPORT_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const USTAR_BLOCK_BYTES: u64 = 512;
const USTAR_TRAILER_BYTES: u64 = 1024;
const CHECKPOINT_ARCHIVE_PATH: &str = "checkpoint/checkpoint-v3.ustar";
const HISTORY_PATH: &str = "history.bin";
const HOF_INDEX_PATH: &str = "hof/index.bin";
const HOF_WEIGHTS_PATH: &str = "hof/weights.f32le";
const MANIFEST_PATH: &str = "manifest.json";
const SAVE_ROOT_DOMAIN: &[u8] = b"slither-neuroevo-save-root\0v1\0";

/// Exact fixed-width inventory facts returned by the metadata worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportInventoryDescriptor {
    pub version: u32,
    pub relative_filename: String,
    pub sha256: String,
    pub stored_byte_count_hex: String,
    pub history_count_hex: String,
    pub hall_of_fame_count_hex: String,
}

/// Small result returned to Node after an atomically published download is ready.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportArchiveDescriptor {
    pub operation_id: String,
    pub checkpoint_id: String,
    pub relative_filename: String,
    pub download_filename: String,
    pub stored_byte_count_hex: String,
    pub logical_root_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveManifest {
    magic: String,
    archive_version: u32,
    archive_kind: String,
    run_id: String,
    generation_hex: String,
    completed_step_hex: String,
    checkpoint_logical_root_sha256: String,
    logical_root_sha256: String,
    history_count_hex: String,
    hall_of_fame_count_hex: String,
    hall_of_fame_weight_count_hex: String,
    roles: Vec<SaveRole>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveRole {
    role: String,
    path: String,
    encoding: String,
    stored_bytes_hex: String,
    decoded_bytes_hex: String,
    decoded_count_hex: String,
    record_size: u32,
    logical_sha256: String,
}

struct ScratchFiles {
    paths: Vec<PathBuf>,
}

impl ScratchFiles {
    fn new() -> Self {
        Self { paths: Vec::new() }
    }

    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for ScratchFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// Compose, fully re-read, and atomically publish one self-contained save archive.
#[allow(clippy::too_many_arguments)]
pub fn compose_export_archive(
    managed_directory: &Path,
    operation_id: &str,
    checkpoint: &CheckpointDescriptor,
    inventory: &ExportInventoryDescriptor,
    checkpoint_limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<ExportArchiveDescriptor, CheckpointError> {
    validate_operation_id(operation_id)?;
    if checkpoint.operation_id.as_str() == operation_id {
        return Err(CheckpointError::format(
            "EXPORT_OPERATION",
            "export operation must be distinct from checkpoint publication",
        ));
    }
    let managed_directory = managed_directory.canonicalize()?;
    let checkpoint_path = direct_file(
        &managed_directory,
        &checkpoint.relative_filename,
        parse_hex_u64(&checkpoint.stored_byte_count_hex, "checkpoint stored bytes")?,
        "checkpoint",
    )?;
    let restored = restore_committed_checkpoint(
        &managed_directory,
        checkpoint,
        checkpoint_limits,
        graph_limits,
        admission_policy,
    )?;
    if restored.content.logical_root_sha256 != checkpoint.logical_root_sha256 {
        return Err(CheckpointError::format(
            "EXPORT_CHECKPOINT",
            "restored checkpoint identity changed before export",
        ));
    }

    let inventory_facts = validate_inventory(&managed_directory, operation_id, inventory)?;
    let expected_history = parse_hex_u64(&checkpoint.generation_hex, "checkpoint generation")?
        .checked_sub(1)
        .ok_or_else(|| {
            CheckpointError::format("EXPORT_GENERATION", "checkpoint generation is zero")
        })?;
    if inventory_facts.history_count != expected_history
        || inventory_facts.hall_of_fame_count != expected_history
    {
        return Err(CheckpointError::format(
            "EXPORT_COVERAGE",
            "inventory does not cover every completed checkpoint generation",
        ));
    }

    let mut scratch = ScratchFiles::new();
    let hof_weights_partial =
        managed_directory.join(format!(".{operation_id}.export-hof-weights.partial"));
    scratch.track(hof_weights_partial.clone());
    let (hof_weights_bytes, hof_weight_count, hof_weights_sha256) = build_hall_of_fame_weights(
        &managed_directory,
        &inventory_facts,
        &hof_weights_partial,
        checkpoint,
    )?;

    let checkpoint_bytes = fs::metadata(&checkpoint_path)?.len();
    let checkpoint_sha256 = hash_file_range(&checkpoint_path, 0, checkpoint_bytes)?;
    let history_sha256 = hash_file_range(
        &inventory_facts.path,
        INVENTORY_HEADER_BYTES,
        inventory_facts.history_bytes,
    )?;
    let hof_index_offset = INVENTORY_HEADER_BYTES
        .checked_add(inventory_facts.history_bytes)
        .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "inventory offset overflowed"))?;
    let hof_index_sha256 = hash_file_range(
        &inventory_facts.path,
        hof_index_offset,
        inventory_facts.hall_of_fame_bytes,
    )?;
    let roles = vec![
        save_role(
            "checkpoint-v3",
            CHECKPOINT_ARCHIVE_PATH,
            "raw-ustar-v3",
            checkpoint_bytes,
            checkpoint_bytes,
            1,
            0,
            checkpoint_sha256,
        ),
        save_role(
            "history",
            HISTORY_PATH,
            "raw-history-v1",
            inventory_facts.history_bytes,
            inventory_facts.history_bytes,
            inventory_facts.history_count,
            HISTORY_RECORD_BYTES as u32,
            history_sha256,
        ),
        save_role(
            "hall-of-fame-index",
            HOF_INDEX_PATH,
            "raw-hof-index-v1",
            inventory_facts.hall_of_fame_bytes,
            inventory_facts.hall_of_fame_bytes,
            inventory_facts.hall_of_fame_count,
            HOF_RECORD_BYTES as u32,
            hof_index_sha256,
        ),
        save_role(
            "hall-of-fame-weights",
            HOF_WEIGHTS_PATH,
            NumericEncoding::RawF32LeV1.as_str(),
            hof_weights_bytes,
            hof_weights_bytes,
            hof_weight_count,
            4,
            hof_weights_sha256,
        ),
    ];
    let save_root = logical_root(&roles)?;
    let manifest = SaveManifest {
        magic: "slither-neuroevo-save".to_owned(),
        archive_version: 1,
        archive_kind: "exact-generation-boundary-v1".to_owned(),
        run_id: checkpoint.run_id.clone(),
        generation_hex: checkpoint.generation_hex.clone(),
        completed_step_hex: checkpoint.completed_step_hex.clone(),
        checkpoint_logical_root_sha256: checkpoint.logical_root_sha256.clone(),
        logical_root_sha256: hex_digest(save_root),
        history_count_hex: hex_u64(inventory_facts.history_count),
        hall_of_fame_count_hex: hex_u64(inventory_facts.hall_of_fame_count),
        hall_of_fame_weight_count_hex: hex_u64(hof_weight_count),
        roles,
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| {
        CheckpointError::format(
            "EXPORT_MANIFEST",
            format!("manifest encoding failed: {error}"),
        )
    })?;
    if manifest_bytes.len() > checkpoint_limits.max_manifest_bytes {
        return Err(CheckpointError::format(
            "EXPORT_MANIFEST",
            "save manifest exceeds the bounded checkpoint manifest limit",
        ));
    }

    let relative_filename = format!(".{operation_id}.slither-save.ready");
    let final_path = managed_directory.join(&relative_filename);
    let partial_path = managed_directory.join(format!(".{operation_id}.slither-save.partial"));
    scratch.track(partial_path.clone());
    let expected_archive_bytes = expected_archive_length(&[
        checkpoint_bytes,
        inventory_facts.history_bytes,
        inventory_facts.hall_of_fame_bytes,
        hof_weights_bytes,
        manifest_bytes.len() as u64,
    ])?;
    if expected_archive_bytes > MAX_EXPORT_ARCHIVE_BYTES {
        return Err(CheckpointError::format(
            "EXPORT_ARCHIVE_LIMIT",
            "save archive exceeds the four-GiB transfer limit",
        ));
    }

    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial_path)?;
    let mut archive = TarBuilder::new(BufWriter::new(output));
    append_file(
        &mut archive,
        CHECKPOINT_ARCHIVE_PATH,
        &checkpoint_path,
        0,
        checkpoint_bytes,
    )?;
    append_file(
        &mut archive,
        HISTORY_PATH,
        &inventory_facts.path,
        INVENTORY_HEADER_BYTES,
        inventory_facts.history_bytes,
    )?;
    append_file(
        &mut archive,
        HOF_INDEX_PATH,
        &inventory_facts.path,
        hof_index_offset,
        inventory_facts.hall_of_fame_bytes,
    )?;
    append_file(
        &mut archive,
        HOF_WEIGHTS_PATH,
        &hof_weights_partial,
        0,
        hof_weights_bytes,
    )?;
    append_bytes(&mut archive, MANIFEST_PATH, &manifest_bytes)?;
    archive.finish()?;
    let mut writer = archive.into_inner()?;
    writer.flush()?;
    let output = writer.into_inner().map_err(|error| error.into_error())?;
    output.sync_all()?;
    if output.metadata()?.len() != expected_archive_bytes {
        return Err(CheckpointError::format(
            "EXPORT_ARCHIVE_LENGTH",
            "completed save archive has an unexpected length",
        ));
    }
    drop(output);
    validate_completed_archive(&partial_path, &manifest, expected_archive_bytes)?;
    rename_noreplace(&partial_path, &final_path)?;
    scratch.track(final_path.clone());
    sync_parent_directory(&managed_directory)?;
    scratch
        .paths
        .retain(|path| path != &partial_path && path != &final_path);

    let generation = parse_hex_u64(&checkpoint.generation_hex, "checkpoint generation")?;
    Ok(ExportArchiveDescriptor {
        operation_id: operation_id.to_owned(),
        checkpoint_id: checkpoint.logical_root_sha256.clone(),
        relative_filename,
        download_filename: format!(
            "slither-neuroevo-{}-gen-{generation}-v1.slither-save",
            &checkpoint.logical_root_sha256[..12]
        ),
        stored_byte_count_hex: hex_u64(expected_archive_bytes),
        logical_root_sha256: manifest.logical_root_sha256,
    })
}

struct InventoryFacts {
    path: PathBuf,
    history_count: u64,
    hall_of_fame_count: u64,
    history_bytes: u64,
    hall_of_fame_bytes: u64,
}

fn validate_inventory(
    directory: &Path,
    operation_id: &str,
    descriptor: &ExportInventoryDescriptor,
) -> Result<InventoryFacts, CheckpointError> {
    if descriptor.version != 1
        || descriptor.relative_filename != format!(".{operation_id}.export-inventory-v1")
    {
        return Err(CheckpointError::format(
            "EXPORT_INVENTORY",
            "inventory identity does not match the export operation",
        ));
    }
    let history_count = parse_hex_u64(&descriptor.history_count_hex, "history count")?;
    let hall_of_fame_count =
        parse_hex_u64(&descriptor.hall_of_fame_count_hex, "Hall-of-Fame count")?;
    if history_count != hall_of_fame_count || history_count > MAX_EXPORT_GENERATIONS {
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
    let expected_bytes = INVENTORY_HEADER_BYTES
        .checked_add(history_bytes)
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
    if &header[..INVENTORY_MAGIC.len()] != INVENTORY_MAGIC
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
    Ok(InventoryFacts {
        path,
        history_count,
        hall_of_fame_count,
        history_bytes,
        hall_of_fame_bytes,
    })
}

fn validate_history_records(file: &mut File, count: u64) -> Result<(), CheckpointError> {
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

fn build_hall_of_fame_weights(
    directory: &Path,
    inventory: &InventoryFacts,
    output_path: &Path,
    checkpoint: &CheckpointDescriptor,
) -> Result<(u64, u64, [u8; 32]), CheckpointError> {
    let population_count = parse_hex_u64(&checkpoint.population_count_hex, "population count")?;
    let checkpoint_weights = parse_hex_u64(&checkpoint.weight_count_hex, "weight count")?;
    let expected_per_genome = if population_count == 0 {
        return Err(CheckpointError::format(
            "EXPORT_POPULATION",
            "checkpoint population count is zero",
        ));
    } else if checkpoint_weights % population_count != 0 {
        return Err(CheckpointError::format(
            "EXPORT_POPULATION",
            "checkpoint weight count does not divide by population",
        ));
    } else {
        checkpoint_weights / population_count
    };
    let expected_total_weights = inventory
        .hall_of_fame_count
        .checked_mul(expected_per_genome)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame weight count overflowed")
        })?;
    let expected_total_bytes = expected_total_weights.checked_mul(4).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame byte count overflowed")
    })?;
    if expected_total_bytes > MAX_EXPORT_ARCHIVE_BYTES {
        return Err(CheckpointError::format(
            "EXPORT_ARCHIVE_LIMIT",
            "Hall-of-Fame weights alone exceed the archive limit",
        ));
    }
    let mut input = File::open(&inventory.path)?;
    input.seek(SeekFrom::Start(
        INVENTORY_HEADER_BYTES + inventory.history_bytes,
    ))?;
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_path)?;
    let mut output = BufWriter::new(output);
    let mut hasher = Sha256::new();
    let mut total_weights = 0u64;
    let mut record = [0u8; HOF_RECORD_BYTES as usize];
    for generation in 1..=inventory.hall_of_fame_count {
        input.read_exact(&mut record)?;
        if read_u64(&record, 0) != generation
            || !f64::from_bits(read_u64(&record, 32)).is_finite()
            || !f64::from_bits(read_u64(&record, 40)).is_finite()
            || record[89..96].iter().any(|byte| *byte != 0)
        {
            return Err(CheckpointError::format(
                "EXPORT_HOF_INDEX",
                "Hall-of-Fame records are malformed or non-contiguous",
            ));
        }
        let logical_sha256 = hex_digest(record[56..88].try_into().unwrap());
        let encoding = match record[88] {
            0 => NumericEncoding::RawF32LeV1,
            1 => NumericEncoding::F32LeShuffle4ZstdV1,
            _ => {
                return Err(CheckpointError::format(
                    "EXPORT_HOF_INDEX",
                    "Hall-of-Fame record has an unsupported encoding",
                ))
            }
        };
        let stored_bytes = read_u64(&record, 96);
        let decoded_bytes = read_u64(&record, 104);
        let weight_count = read_u64(&record, 112);
        if weight_count != expected_per_genome || decoded_bytes != weight_count.saturating_mul(4) {
            return Err(CheckpointError::format(
                "EXPORT_HOF_INDEX",
                "Hall-of-Fame weight shape disagrees with the checkpoint graph",
            ));
        }
        let descriptor = HallOfFameWeightsDescriptor {
            version: 1,
            logical_sha256: logical_sha256.clone(),
            relative_filename: format!("{logical_sha256}.hof-weights-v1"),
            encoding,
            stored_byte_count_hex: hex_u64(stored_bytes),
            decoded_byte_count_hex: hex_u64(decoded_bytes),
            weight_count_hex: hex_u64(weight_count),
        };
        let path = direct_file(
            directory,
            &descriptor.relative_filename,
            stored_bytes,
            "Hall-of-Fame weights",
        )?;
        let weights = read_validated_hall_of_fame_weights(&path, &descriptor)?;
        for value in weights.iter().copied() {
            let bytes = value.to_bits().to_le_bytes();
            hasher.update(bytes);
            output.write_all(&bytes)?;
        }
        total_weights = total_weights.checked_add(weight_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame weight count overflowed")
        })?;
    }
    output.flush()?;
    let output = output.into_inner().map_err(|error| error.into_error())?;
    output.sync_all()?;
    let total_bytes = total_weights.checked_mul(4).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame byte count overflowed")
    })?;
    if output.metadata()?.len() != total_bytes {
        return Err(CheckpointError::format(
            "EXPORT_HOF_WEIGHTS",
            "Hall-of-Fame aggregate length changed while writing",
        ));
    }
    if total_weights != expected_total_weights {
        return Err(CheckpointError::format(
            "EXPORT_HOF_WEIGHTS",
            "Hall-of-Fame aggregate count changed while writing",
        ));
    }
    Ok((total_bytes, total_weights, hasher.finalize().into()))
}

fn direct_file(
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
fn save_role(
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

fn logical_root(roles: &[SaveRole]) -> Result<[u8; 32], CheckpointError> {
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

fn append_file<W: Write>(
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

fn append_bytes<W: Write>(
    archive: &mut TarBuilder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), CheckpointError> {
    append_reader(archive, path, bytes.len() as u64, io::Cursor::new(bytes))
}

fn append_reader<W: Write, R: Read>(
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
    archive.append_data(&mut header, path, reader)?;
    Ok(())
}

fn expected_archive_length(sizes: &[u64]) -> Result<u64, CheckpointError> {
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

fn validate_completed_archive(
    path: &Path,
    manifest: &SaveManifest,
    expected_bytes: u64,
) -> Result<(), CheckpointError> {
    if fs::metadata(path)?.len() != expected_bytes {
        return Err(CheckpointError::format(
            "EXPORT_POSTWRITE",
            "save archive length changed before validation",
        ));
    }
    let file = File::open(path)?;
    let mut archive = TarArchive::new(BufReader::new(file));
    let expected_paths = [
        CHECKPOINT_ARCHIVE_PATH,
        HISTORY_PATH,
        HOF_INDEX_PATH,
        HOF_WEIGHTS_PATH,
        MANIFEST_PATH,
    ];
    let mut seen = 0usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if seen >= expected_paths.len()
            || !entry.header().entry_type().is_file()
            || entry.path()?.as_ref() != Path::new(expected_paths[seen])
        {
            return Err(CheckpointError::format(
                "EXPORT_POSTWRITE",
                "save archive entry order or type is invalid",
            ));
        }
        if seen < manifest.roles.len() {
            let role = &manifest.roles[seen];
            let mut hasher = Sha256::new();
            io::copy(&mut entry, &mut HashWriter(&mut hasher))?;
            if hex_digest(hasher.finalize().into()) != role.logical_sha256 {
                return Err(CheckpointError::format(
                    "EXPORT_POSTWRITE",
                    format!("save role {} failed post-write SHA-256", role.role),
                ));
            }
        } else {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            let decoded: SaveManifest = serde_json::from_slice(&bytes).map_err(|error| {
                CheckpointError::format(
                    "EXPORT_POSTWRITE",
                    format!("save manifest failed post-write decode: {error}"),
                )
            })?;
            if &decoded != manifest {
                return Err(CheckpointError::format(
                    "EXPORT_POSTWRITE",
                    "save manifest changed during archive publication",
                ));
            }
        }
        seen += 1;
    }
    if seen != expected_paths.len() {
        return Err(CheckpointError::format(
            "EXPORT_POSTWRITE",
            "save archive is missing required entries",
        ));
    }
    Ok(())
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn hash_file_range(path: &Path, offset: u64, length: u64) -> Result<[u8; 32], CheckpointError> {
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

fn validate_operation_id(value: &str) -> Result<(), CheckpointError> {
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

fn parse_hex_u64(value: &str, label: &str) -> Result<u64, CheckpointError> {
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

fn parse_digest(value: &str, label: &str) -> Result<[u8; 32], CheckpointError> {
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

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn hex_u64(value: u64) -> String {
    format!("{value:016x}")
}

fn hex_digest(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
