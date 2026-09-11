//! Bounded self-contained save assembly from one exact leased checkpoint.
//!
//! SQLite stays outside Rust's archive codec. Its worker publishes one fixed-width
//! inventory beside immutable checkpoint and Hall-of-Fame files; this module
//! validates those files, fully decodes every referenced numeric object, and
//! writes one ordinary USTAR download without copying population data through Node.

use super::checkpoint::{
    decode_adaptive_numeric_file, publication_descriptor_for_restored,
    publish_hall_of_fame_weights, read_validated_hall_of_fame_weights, rename_noreplace,
    restore_committed_checkpoint, select_adaptive_numeric_file, sync_parent_directory,
    validated_checkpoint_archive_layout, CheckpointDescriptor, CheckpointError, CheckpointLimits,
    CheckpointManifest, CheckpointOperationId, HallOfFameWeightsDescriptor, NumericEncoding,
    RestoredCheckpoint,
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
const HISTORY_PATH: &str = "history.bin";
const HOF_INDEX_PATH: &str = "hof/index.bin";
const HOF_WEIGHTS_PATH: &str = "hof/weights.f32le";
const HOF_WEIGHTS_ZSTD_PATH: &str = "hof/weights.f32le.shuf4.zst";
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

/// Scalar identity recovered only after an uploaded save and embedded checkpoint validate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedImportArchive {
    pub run_id: String,
    pub generation_hex: String,
    pub completed_step_hex: String,
    pub checkpoint_id: String,
    pub save_logical_root_sha256: String,
    pub history_count_hex: String,
    pub hall_of_fame_count_hex: String,
    pub stored_byte_count_hex: String,
}

/// Fully admitted private candidate plus its newly published managed descriptor.
/// Population and world state never cross the native boundary.
#[derive(Debug)]
pub struct PreparedImportArchive {
    pub facts: ValidatedImportArchive,
    pub descriptor: CheckpointDescriptor,
    pub inventory: ImportInventoryDescriptor,
    pub startup_metadata_json: String,
    pub transition: super::run_start::PendingRunStartTransition,
}

/// Trusted fixed-width history and Hall-of-Fame inventory published by Rust
/// after every archive record and weight segment validates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportInventoryDescriptor {
    pub version: u32,
    pub relative_filename: String,
    pub sha256: String,
    pub stored_byte_count_hex: String,
    pub history_count_hex: String,
    pub hall_of_fame_count_hex: String,
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
    checkpoint_manifest: CheckpointManifest,
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

struct ScannedImportArchive {
    manifest: SaveManifest,
    entry_paths: Vec<String>,
    entry_sizes: Vec<u64>,
    entry_hashes: Vec<[u8; 32]>,
}

struct ValidatedImportCandidate {
    facts: ValidatedImportArchive,
    restored: RestoredCheckpoint,
    checkpoint_path: PathBuf,
    stage_directory: PathBuf,
    cleanup: ScratchFiles,
    inventory: Option<ImportInventoryDescriptor>,
}

/// Strictly validate an untrusted save upload without changing live or durable state.
pub fn validate_import_archive(
    archive_path: &Path,
    scratch_directory: &Path,
    operation_id: &str,
    checkpoint_limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<ValidatedImportArchive, CheckpointError> {
    Ok(validate_import_candidate(
        archive_path,
        scratch_directory,
        None,
        operation_id,
        checkpoint_limits,
        graph_limits,
        admission_policy,
    )?
    .facts)
}

/// Validate, retain, and publish one imported checkpoint without changing the
/// live authority or SQLite current pointer.
#[allow(clippy::too_many_arguments)]
pub fn prepare_import_archive(
    archive_path: &Path,
    scratch_directory: &Path,
    managed_directory: &Path,
    operation_id: &str,
    checkpoint_limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
    memory_ceiling_bytes: usize,
) -> Result<PreparedImportArchive, CheckpointError> {
    let mut candidate = validate_import_candidate(
        archive_path,
        scratch_directory,
        Some(managed_directory),
        operation_id,
        checkpoint_limits,
        graph_limits,
        admission_policy,
    )?;
    let operation_id = CheckpointOperationId::parse(operation_id.to_owned())?;
    let descriptor = publication_descriptor_for_restored(&candidate.restored, operation_id);
    let managed_directory = managed_directory.canonicalize()?;
    let final_path = managed_directory.join(&descriptor.relative_filename);
    if final_path.exists() {
        let existing = super::checkpoint::restore_checkpoint(
            &final_path,
            checkpoint_limits,
            graph_limits,
            admission_policy,
        )?;
        if existing.content != candidate.restored.content
            || existing.state.state() != candidate.restored.state.state()
            || existing.state.graph_spec() != candidate.restored.state.graph_spec()
        {
            return Err(CheckpointError::format(
                "IMPORT_CHECKPOINT_COLLISION",
                "existing digest-derived checkpoint differs from the imported candidate",
            ));
        }
        fs::remove_file(&candidate.checkpoint_path)?;
    } else {
        rename_noreplace(&candidate.checkpoint_path, &final_path)?;
        sync_parent_directory(&managed_directory)?;
    }
    fs::remove_dir(&candidate.stage_directory)?;
    let transition = super::fresh_run::prepare_stage6a_p0_validated_import(
        candidate.restored,
        descriptor.clone(),
        memory_ceiling_bytes,
    )
    .map_err(|error| CheckpointError::format("IMPORT_CANDIDATE", error.to_string()))?;
    let startup_metadata_json = transition
        .startup_metadata_json()
        .map_err(|error| CheckpointError::format("IMPORT_METADATA", error))?;
    candidate.cleanup.paths.clear();
    Ok(PreparedImportArchive {
        facts: candidate.facts,
        descriptor,
        inventory: candidate.inventory.ok_or_else(|| {
            CheckpointError::format("IMPORT_INVENTORY", "prepared import inventory is missing")
        })?,
        startup_metadata_json,
        transition,
    })
}

fn validate_import_candidate(
    archive_path: &Path,
    scratch_directory: &Path,
    publication_directory: Option<&Path>,
    operation_id: &str,
    checkpoint_limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<ValidatedImportCandidate, CheckpointError> {
    validate_operation_id(operation_id)?;
    let metadata = fs::symlink_metadata(archive_path)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_EXPORT_ARCHIVE_BYTES
    {
        return Err(CheckpointError::format(
            "IMPORT_ARCHIVE_LIMIT",
            "upload must be one nonempty regular file within the four-GiB archive limit",
        ));
    }
    let scanned = scan_import_archive(archive_path)?;
    let manifest = scanned.manifest;
    validate_import_manifest(
        &manifest,
        &scanned.entry_paths,
        &scanned.entry_sizes,
        &scanned.entry_hashes,
        metadata.len(),
    )?;

    let scratch_directory = scratch_directory.canonicalize()?;
    let stage_directory = scratch_directory.join(format!(".{operation_id}.import-validation"));
    fs::create_dir(&stage_directory)?;
    let checkpoint_path = stage_directory.join(format!(
        "{}.checkpoint-v3",
        manifest.checkpoint_logical_root_sha256
    ));
    let mut cleanup = ScratchFiles::new();
    cleanup.track(stage_directory.clone());
    cleanup.track(checkpoint_path.clone());
    let inventory = extract_and_validate_import_roles(
        archive_path,
        &checkpoint_path,
        &manifest,
        publication_directory,
        operation_id,
        checkpoint_limits,
    )?;
    if let (Some(directory), Some(descriptor)) = (publication_directory, &inventory) {
        cleanup.track(
            directory
                .canonicalize()?
                .join(&descriptor.relative_filename),
        );
    }
    let restored = super::checkpoint::restore_checkpoint(
        &checkpoint_path,
        checkpoint_limits,
        graph_limits,
        admission_policy,
    )?;
    if restored.content.run_id != manifest.run_id
        || restored.content.generation_hex != manifest.generation_hex
        || restored.content.completed_step_hex != manifest.completed_step_hex
        || restored.content.logical_root_sha256 != manifest.checkpoint_logical_root_sha256
    {
        return Err(CheckpointError::format(
            "IMPORT_CHECKPOINT_IDENTITY",
            "embedded checkpoint disagrees with the save manifest",
        ));
    }
    let facts = ValidatedImportArchive {
        run_id: manifest.run_id,
        generation_hex: manifest.generation_hex,
        completed_step_hex: manifest.completed_step_hex,
        checkpoint_id: manifest.checkpoint_logical_root_sha256,
        save_logical_root_sha256: manifest.logical_root_sha256,
        history_count_hex: manifest.history_count_hex,
        hall_of_fame_count_hex: manifest.hall_of_fame_count_hex,
        stored_byte_count_hex: hex_u64(metadata.len()),
    };
    Ok(ValidatedImportCandidate {
        facts,
        restored,
        checkpoint_path,
        stage_directory,
        cleanup,
        inventory,
    })
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
        for path in self.paths.iter().rev() {
            if fs::remove_file(path).is_err() {
                let _ = fs::remove_dir(path);
            }
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
    let checkpoint_layout =
        validated_checkpoint_archive_layout(&checkpoint_path, checkpoint_limits)?;
    if checkpoint_layout.manifest.logical_root_sha256 != checkpoint.logical_root_sha256
        || checkpoint_layout.manifest.run_id != checkpoint.run_id
        || checkpoint_layout.manifest.generation_hex != checkpoint.generation_hex
        || checkpoint_layout.manifest.completed_step_hex != checkpoint.completed_step_hex
    {
        return Err(CheckpointError::format(
            "EXPORT_CHECKPOINT",
            "checkpoint manifest identity changed before flat export",
        ));
    }

    let inventory_facts = validate_inventory(&managed_directory, operation_id, inventory)?;
    let expected_history = parse_hex_u64(&checkpoint.generation_hex, "checkpoint generation")?
        .checked_sub(1)
        .ok_or_else(|| {
            CheckpointError::format("EXPORT_GENERATION", "checkpoint generation is zero")
        })?;
    if inventory_facts.history_count != expected_history
        || inventory_facts.hall_of_fame_count > expected_history
    {
        return Err(CheckpointError::format(
            "EXPORT_COVERAGE",
            "inventory history coverage or selected Hall-of-Fame count is invalid",
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
    let hof_numeric = select_adaptive_numeric_file(
        &hof_weights_partial,
        &managed_directory,
        operation_id,
        "export-hof-weights",
        hof_weight_count,
        checkpoint_limits,
    )?;
    if hof_numeric.logical_sha256 != hof_weights_sha256
        || hof_numeric.decoded_bytes != hof_weights_bytes
        || hof_numeric.float_count != hof_weight_count
    {
        return Err(CheckpointError::format(
            "EXPORT_HOF_WEIGHTS",
            "adaptive Hall-of-Fame encoding changed logical weights",
        ));
    }
    if let Some(path) = &hof_numeric.compressed_path {
        scratch.track(path.clone());
    }
    let hof_weights_path = match hof_numeric.encoding {
        NumericEncoding::RawF32LeV1 => HOF_WEIGHTS_PATH,
        NumericEncoding::F32LeShuffle4ZstdV1 => HOF_WEIGHTS_ZSTD_PATH,
    };
    let hof_weights_source = hof_numeric
        .compressed_path
        .as_deref()
        .unwrap_or(&hof_weights_partial);

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
    let mut roles = checkpoint_layout
        .roles
        .iter()
        .map(|role| {
            Ok(save_role(
                &role.manifest.role,
                &role.manifest.path,
                &role.manifest.encoding,
                role.stored_bytes,
                parse_hex_u64(
                    &role.manifest.decoded_bytes_hex,
                    "checkpoint role decoded bytes",
                )?,
                parse_hex_u64(
                    &role.manifest.decoded_count_hex,
                    "checkpoint role decoded count",
                )?,
                role.manifest.record_size,
                parse_digest(
                    &role.manifest.logical_sha256,
                    "checkpoint role logical SHA-256",
                )?,
            ))
        })
        .collect::<Result<Vec<_>, CheckpointError>>()?;
    roles.extend([
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
            hof_weights_path,
            hof_numeric.encoding.as_str(),
            hof_numeric.stored_bytes,
            hof_weights_bytes,
            hof_weight_count,
            4,
            hof_weights_sha256,
        ),
    ]);
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
        checkpoint_manifest: checkpoint_layout.manifest.clone(),
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
    let mut entry_sizes = checkpoint_layout
        .roles
        .iter()
        .map(|role| role.stored_bytes)
        .collect::<Vec<_>>();
    entry_sizes.extend([
        inventory_facts.history_bytes,
        inventory_facts.hall_of_fame_bytes,
        hof_numeric.stored_bytes,
        manifest_bytes.len() as u64,
    ]);
    let expected_archive_bytes = expected_archive_length(&entry_sizes)?;
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
    for role in &checkpoint_layout.roles {
        append_file(
            &mut archive,
            &role.manifest.path,
            &checkpoint_path,
            role.data_offset,
            role.stored_bytes,
        )?;
    }
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
        hof_weights_path,
        hof_weights_source,
        0,
        hof_numeric.stored_bytes,
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
    let validated = validate_import_archive(
        &partial_path,
        &managed_directory,
        operation_id,
        checkpoint_limits,
        graph_limits,
        admission_policy,
    )?;
    if validated.checkpoint_id != checkpoint.logical_root_sha256
        || validated.save_logical_root_sha256 != manifest.logical_root_sha256
    {
        return Err(CheckpointError::format(
            "EXPORT_POSTWRITE",
            "full flat-save validation returned a different identity",
        ));
    }
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
    let mut previous_generation = 0u64;
    for _ in 0..inventory.hall_of_fame_count {
        input.read_exact(&mut record)?;
        let generation = read_u64(&record, 0);
        if generation <= previous_generation
            || generation > inventory.history_count
            || !f64::from_bits(read_u64(&record, 32)).is_finite()
            || !f64::from_bits(read_u64(&record, 40)).is_finite()
            || record[89..96].iter().any(|byte| *byte != 0)
        {
            return Err(CheckpointError::format(
                "EXPORT_HOF_INDEX",
                "Hall-of-Fame records are malformed, duplicated, or unordered",
            ));
        }
        previous_generation = generation;
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

fn scan_import_archive(path: &Path) -> Result<ScannedImportArchive, CheckpointError> {
    let file = File::open(path)?;
    let mut archive = TarArchive::new(BufReader::new(file));
    let mut sizes = Vec::new();
    sizes.try_reserve_exact(9).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve save entry-size table")
    })?;
    let mut hashes = Vec::new();
    hashes.try_reserve_exact(8).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve save entry-hash table")
    })?;
    let mut paths = Vec::new();
    paths.try_reserve_exact(9).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve save entry-path table")
    })?;
    let mut manifest = None;
    let mut seen = 0usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if seen >= 9 || !entry.header().entry_type().is_file() {
            return Err(CheckpointError::format(
                "IMPORT_USTAR",
                "save entries are unknown, unsafe, duplicated, or out of order",
            ));
        }
        let path = entry.path()?.into_owned();
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(CheckpointError::format(
                "IMPORT_USTAR",
                "save entry path is not a safe relative role path",
            ));
        }
        let size = entry.header().size()?;
        let path_text = path.to_str().ok_or_else(|| {
            CheckpointError::format("IMPORT_USTAR", "save entry path is not UTF-8")
        })?;
        paths.push(path_text.to_owned());
        sizes.push(size);
        if seen < 8 {
            let limit = match seen {
                5 => MAX_EXPORT_GENERATIONS * HISTORY_RECORD_BYTES,
                6 => MAX_EXPORT_GENERATIONS * HOF_RECORD_BYTES,
                _ => MAX_EXPORT_ARCHIVE_BYTES,
            };
            if size > limit {
                return Err(CheckpointError::format(
                    "IMPORT_ENTRY_LIMIT",
                    format!("save entry {} exceeds its stored limit", path.display()),
                ));
            }
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
            if path != Path::new(MANIFEST_PATH) {
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
        seen += 1;
    }
    if seen != 9 {
        return Err(CheckpointError::format(
            "IMPORT_USTAR",
            "save archive is missing required entries or its final manifest",
        ));
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

fn validate_import_manifest(
    manifest: &SaveManifest,
    entry_paths: &[String],
    entry_sizes: &[u64],
    entry_hashes: &[[u8; 32]],
    archive_bytes: u64,
) -> Result<(), CheckpointError> {
    if manifest.magic != "slither-neuroevo-save"
        || manifest.archive_version != 1
        || manifest.archive_kind != "exact-generation-boundary-v1"
        || manifest.run_id.is_empty()
        || manifest.run_id.contains('\0')
        || manifest.run_id.len() > 256
        || manifest.roles.len() != 8
        || manifest.checkpoint_manifest.roles.len() != 5
        || entry_paths.len() != 9
        || entry_sizes.len() != 9
        || entry_hashes.len() != 8
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
    if entry_sizes[5] != history_count.saturating_mul(HISTORY_RECORD_BYTES)
        || entry_sizes[6] != hall_of_fame_count.saturating_mul(HOF_RECORD_BYTES)
        || hex_digest(logical_root(&manifest.roles)?) != manifest.logical_root_sha256
        || expected_archive_length(entry_sizes)? != archive_bytes
    {
        return Err(CheckpointError::format(
            "IMPORT_MANIFEST",
            "save aggregate lengths or logical root are invalid",
        ));
    }
    Ok(())
}

fn extract_and_validate_import_roles(
    archive_path: &Path,
    checkpoint_path: &Path,
    manifest: &SaveManifest,
    publication_directory: Option<&Path>,
    operation_id: &str,
    checkpoint_limits: &CheckpointLimits,
) -> Result<Option<ImportInventoryDescriptor>, CheckpointError> {
    let file = File::open(archive_path)?;
    let mut archive = TarArchive::new(BufReader::new(file));
    let mut entries = archive.entries()?;
    let checkpoint = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(checkpoint_path)?;
    let mut checkpoint_archive = TarBuilder::new(BufWriter::new(checkpoint));
    for role in &manifest.checkpoint_manifest.roles {
        let entry = entries.next().transpose()?.ok_or_else(|| {
            CheckpointError::format("IMPORT_USTAR", "flattened checkpoint role is missing")
        })?;
        let size = entry.header().size()?;
        append_reader(&mut checkpoint_archive, &role.path, size, entry)?;
    }
    let checkpoint_manifest_bytes =
        serde_json::to_vec(&manifest.checkpoint_manifest).map_err(|error| {
            CheckpointError::format(
                "IMPORT_CHECKPOINT_MANIFEST",
                format!("checkpoint manifest encoding failed: {error}"),
            )
        })?;
    append_bytes(
        &mut checkpoint_archive,
        MANIFEST_PATH,
        &checkpoint_manifest_bytes,
    )?;
    checkpoint_archive.finish()?;
    let mut checkpoint_writer = checkpoint_archive.into_inner()?;
    checkpoint_writer.flush()?;
    let checkpoint = checkpoint_writer
        .into_inner()
        .map_err(|error| error.into_error())?;
    checkpoint.sync_all()?;
    drop(checkpoint);

    let history_count = parse_hex_u64(&manifest.history_count_hex, "history count")?;
    let hall_of_fame_count = parse_hex_u64(&manifest.hall_of_fame_count_hex, "Hall-of-Fame count")?;
    let publication_directory = publication_directory.map(Path::canonicalize).transpose()?;
    let stage_directory = checkpoint_path.parent().ok_or_else(|| {
        CheckpointError::format("IMPORT_PATH", "checkpoint validation path has no parent")
    })?;
    let inventory_partial_path = stage_directory.join("import-inventory.partial");
    let mut inventory_cleanup = ScratchFiles::new();
    let mut inventory_writer = if publication_directory.is_some() {
        inventory_cleanup.track(inventory_partial_path.clone());
        let mut writer = BufWriter::new(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&inventory_partial_path)?,
        );
        let mut header = [0u8; INVENTORY_HEADER_BYTES as usize];
        header[..13].copy_from_slice(INVENTORY_MAGIC);
        header[16..24].copy_from_slice(&history_count.to_le_bytes());
        header[24..32].copy_from_slice(&hall_of_fame_count.to_le_bytes());
        writer.write_all(&header)?;
        Some(writer)
    } else {
        None
    };
    let mut history = entries
        .next()
        .transpose()?
        .ok_or_else(|| CheckpointError::format("IMPORT_USTAR", "history entry is missing"))?;
    let mut record = [0u8; HISTORY_RECORD_BYTES as usize];
    for generation in 1..=history_count {
        history.read_exact(&mut record)?;
        if read_u64(&record, 0) != generation
            || [8usize, 16, 24, 40, 48]
                .into_iter()
                .any(|offset| !f64::from_bits(read_u64(&record, offset)).is_finite())
        {
            return Err(CheckpointError::format(
                "IMPORT_HISTORY",
                "history records are non-contiguous or contain non-finite values",
            ));
        }
        if let Some(writer) = &mut inventory_writer {
            writer.write_all(&record)?;
        }
    }
    drop(history);

    let mut hall_of_fame = entries
        .next()
        .transpose()?
        .ok_or_else(|| CheckpointError::format("IMPORT_USTAR", "Hall-of-Fame index is missing"))?;
    let mut weight_references: Vec<([u8; HOF_RECORD_BYTES as usize], [u8; 32], u64)> = Vec::new();
    weight_references
        .try_reserve_exact(hall_of_fame_count as usize)
        .map_err(|_| {
            CheckpointError::format(
                "ALLOCATION",
                "unable to reserve bounded Hall-of-Fame import index",
            )
        })?;
    let mut hof_record = [0u8; HOF_RECORD_BYTES as usize];
    let mut indexed_weight_count = 0u64;
    let mut previous_generation = 0u64;
    for _ in 0..hall_of_fame_count {
        hall_of_fame.read_exact(&mut hof_record)?;
        let generation = read_u64(&hof_record, 0);
        let weight_count = read_u64(&hof_record, 112);
        if generation <= previous_generation
            || generation > history_count
            || !f64::from_bits(read_u64(&hof_record, 32)).is_finite()
            || !f64::from_bits(read_u64(&hof_record, 40)).is_finite()
            || !matches!(hof_record[88], 0 | 1)
            || hof_record[89..96].iter().any(|byte| *byte != 0)
            || read_u64(&hof_record, 104) != weight_count.saturating_mul(4)
        {
            return Err(CheckpointError::format(
                "IMPORT_HOF_INDEX",
                "Hall-of-Fame index contains an invalid record",
            ));
        }
        previous_generation = generation;
        indexed_weight_count = indexed_weight_count
            .checked_add(weight_count)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "Hall-of-Fame weight count overflowed")
            })?;
        weight_references.push((
            hof_record,
            hof_record[56..88].try_into().unwrap(),
            weight_count,
        ));
    }
    drop(hall_of_fame);
    let declared_weight_count = parse_hex_u64(
        &manifest.hall_of_fame_weight_count_hex,
        "Hall-of-Fame weight count",
    )?;
    if indexed_weight_count != declared_weight_count {
        return Err(CheckpointError::format(
            "IMPORT_HOF_INDEX",
            "Hall-of-Fame index weight counts disagree with the manifest",
        ));
    }

    let mut weights_entry = entries.next().transpose()?.ok_or_else(|| {
        CheckpointError::format("IMPORT_USTAR", "Hall-of-Fame weights are missing")
    })?;
    let encoded_path = stage_directory.join("hof-weights.encoded");
    let raw_path = stage_directory.join("hof-weights.raw");
    let mut weight_scratch = ScratchFiles::new();
    weight_scratch.track(encoded_path.clone());
    let mut encoded = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&encoded_path)?;
    io::copy(&mut weights_entry, &mut encoded)?;
    encoded.sync_all()?;
    drop(encoded);
    weight_scratch.track(raw_path.clone());
    let mut raw = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&raw_path)?;
    let hof_role = &manifest.roles[7];
    decode_adaptive_numeric_file(
        &encoded_path,
        NumericEncoding::parse(&hof_role.encoding)?,
        parse_hex_u64(&hof_role.stored_bytes_hex, "Hall-of-Fame stored bytes")?,
        declared_weight_count,
        parse_digest(&hof_role.logical_sha256, "Hall-of-Fame logical SHA-256")?,
        &mut raw,
    )?;
    raw.sync_all()?;
    drop(raw);
    let mut weights = BufReader::new(File::open(&raw_path)?);
    let mut bytes = [0u8; 4];
    let parsed_operation_id = CheckpointOperationId::parse(operation_id.to_owned())?;
    for (record, expected_sha256, count) in weight_references {
        let mut hasher = Sha256::new();
        let mut decoded = if publication_directory.is_some() {
            let count = usize::try_from(count).map_err(|_| {
                CheckpointError::format(
                    "IMPORT_HOF_WEIGHTS",
                    "Hall-of-Fame genome is too large for this target",
                )
            })?;
            let mut values = Vec::new();
            values.try_reserve_exact(count).map_err(|_| {
                CheckpointError::format(
                    "ALLOCATION",
                    "unable to reserve one imported Hall-of-Fame genome",
                )
            })?;
            Some(values)
        } else {
            None
        };
        for _ in 0..count {
            weights.read_exact(&mut bytes)?;
            let value = f32::from_bits(u32::from_le_bytes(bytes));
            if !value.is_finite() {
                return Err(CheckpointError::format(
                    "IMPORT_HOF_WEIGHTS",
                    "Hall-of-Fame weights contain a non-finite value",
                ));
            }
            hasher.update(bytes);
            if let Some(values) = &mut decoded {
                values.push(value);
            }
        }
        if <[u8; 32]>::from(hasher.finalize()) != expected_sha256 {
            return Err(CheckpointError::format(
                "IMPORT_HOF_WEIGHTS",
                "Hall-of-Fame weight segment failed its logical SHA-256",
            ));
        }
        if let (Some(directory), Some(values)) = (&publication_directory, decoded) {
            let descriptor = publish_hall_of_fame_weights(
                directory,
                &parsed_operation_id,
                &values,
                checkpoint_limits,
            )?;
            let expected_encoding = match record[88] {
                0 => NumericEncoding::RawF32LeV1,
                1 => NumericEncoding::F32LeShuffle4ZstdV1,
                _ => unreachable!("validated Hall-of-Fame encoding byte"),
            };
            if descriptor.logical_sha256 != hex_digest(expected_sha256)
                || descriptor.encoding != expected_encoding
                || parse_hex_u64(
                    &descriptor.stored_byte_count_hex,
                    "imported Hall-of-Fame stored bytes",
                )? != read_u64(&record, 96)
                || parse_hex_u64(
                    &descriptor.decoded_byte_count_hex,
                    "imported Hall-of-Fame decoded bytes",
                )? != read_u64(&record, 104)
                || parse_hex_u64(
                    &descriptor.weight_count_hex,
                    "imported Hall-of-Fame weight count",
                )? != read_u64(&record, 112)
            {
                return Err(CheckpointError::format(
                    "IMPORT_HOF_OBJECT",
                    "rebuilt Hall-of-Fame object differs from its exact archive descriptor",
                ));
            }
            inventory_writer
                .as_mut()
                .expect("publication creates an inventory writer")
                .write_all(&record)?;
        }
    }
    if weights.read(&mut bytes[..1])? != 0 {
        return Err(CheckpointError::format(
            "IMPORT_HOF_WEIGHTS",
            "Hall-of-Fame weights exceed the indexed segments",
        ));
    }
    drop(weights);
    fs::remove_file(&raw_path)?;
    fs::remove_file(&encoded_path)?;
    weight_scratch.paths.clear();

    let inventory =
        if let (Some(directory), Some(mut writer)) = (publication_directory, inventory_writer) {
            writer.flush()?;
            let file = writer.into_inner().map_err(|error| error.into_error())?;
            file.sync_all()?;
            let expected_bytes = INVENTORY_HEADER_BYTES
                .checked_add(history_count.saturating_mul(HISTORY_RECORD_BYTES))
                .and_then(|bytes| {
                    bytes.checked_add(hall_of_fame_count.saturating_mul(HOF_RECORD_BYTES))
                })
                .ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "import inventory length overflowed")
                })?;
            if file.metadata()?.len() != expected_bytes {
                return Err(CheckpointError::format(
                    "IMPORT_INVENTORY",
                    "trusted import inventory has an unexpected length",
                ));
            }
            drop(file);
            let relative_filename = format!(".{operation_id}.import-inventory-v1");
            let final_path = directory.join(&relative_filename);
            rename_noreplace(&inventory_partial_path, &final_path)?;
            inventory_cleanup.track(final_path.clone());
            sync_parent_directory(&directory)?;
            let sha256 = hex_digest(hash_file_range(&final_path, 0, expected_bytes)?);
            inventory_cleanup.paths.clear();
            Some(ImportInventoryDescriptor {
                version: 1,
                relative_filename,
                sha256,
                stored_byte_count_hex: hex_u64(expected_bytes),
                history_count_hex: hex_u64(history_count),
                hall_of_fame_count_hex: hex_u64(hall_of_fame_count),
            })
        } else {
            None
        };
    Ok(inventory)
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
    let mut seen = 0usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let expected_path = if seen < manifest.roles.len() {
            manifest.roles[seen].path.as_str()
        } else {
            MANIFEST_PATH
        };
        if seen > manifest.roles.len()
            || !entry.header().entry_type().is_file()
            || entry.path()?.as_ref() != Path::new(expected_path)
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
            if matches!(
                role.encoding.as_str(),
                "raw-binary-v1" | "raw-f32le-v1" | "raw-history-v1" | "raw-hof-index-v1"
            ) && hex_digest(hasher.finalize().into()) != role.logical_sha256
            {
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
    if seen != manifest.roles.len() + 1 {
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
