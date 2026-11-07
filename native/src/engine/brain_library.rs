//! Native admission and immutable publication for explicitly saved neural brains.

use super::brain_artifact::{
    validate_brain_artifact, BrainArtifactDescriptorV1, BrainProvenanceV1, MAX_BRAIN_ARTIFACT_BYTES,
};
use super::checkpoint::{
    publish_managed_neural_weights, rename_noreplace, sync_parent_directory, CheckpointOperationId,
    ManagedNeuralWeightsDescriptor,
};
use super::training_graph::CanonicalTrainingGraphManifestV1;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// One exact catalog lease, observed run/configuration and independently verified weights.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedBrainSpawnRequest {
    pub managed_directory: String,
    pub brain_id: String,
    pub expected_run_id: String,
    pub expected_config_revision: u64,
    pub expected_world_epoch: u64,
    pub weights: Box<ManagedNeuralWeightsDescriptor>,
}

impl SavedBrainSpawnRequest {
    /// Bound all owned scalar metadata before queue admission; files stay cold.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.managed_directory.is_empty()
            || self.managed_directory.len() > 32768
            || self.managed_directory.contains('\0')
            || self.brain_id.len() != 64
            || !self
                .brain_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.expected_run_id.is_empty()
            || self.expected_run_id.len() > 256
            || self.expected_run_id.contains('\0')
            || self.expected_config_revision == 0
            || self.expected_world_epoch == 0
        {
            return Err("invalid saved brain deployment scope");
        }
        Ok(())
    }

    /// Exact queued allocations, independent of later decoded neural storage.
    pub(crate) fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.managed_directory.capacity()
            + self.brain_id.capacity()
            + self.expected_run_id.capacity()
            + std::mem::size_of::<ManagedNeuralWeightsDescriptor>()
            + self.weights.owned_bytes()
    }
}

/// Rust-derived small metadata; no population/parameter arrays cross into Node.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedBrainPublicationV1 {
    pub schema_version: u32,
    pub artifact: BrainArtifactDescriptorV1,
    pub graph_manifest: CanonicalTrainingGraphManifestV1,
    pub provenance: BrainProvenanceV1,
    pub weights: ManagedNeuralWeightsDescriptor,
    pub artifact_relative_filename: String,
    pub artifact_byte_count: String,
}

/// Read one regular, bounded file into one immutable validation snapshot.
pub(crate) fn read_artifact_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_BRAIN_ARTIFACT_BYTES as u64
    {
        return Err("brain artifact file admission".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_BRAIN_ARTIFACT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BRAIN_ARTIFACT_BYTES {
        return Err("brain artifact byte admission".into());
    }
    Ok(bytes)
}

/// Validate all source contracts before publishing the portable original and
/// shared adaptive weights. Quota counts physical objects already referenced by
/// the library only once; caller supplies its exact metadata-derived allowance.
pub fn publish_saved_brain_file(
    source: &Path,
    directory: &Path,
    operation: &CheckpointOperationId,
    expected_artifact_sha256: &str,
    available_bytes: u64,
    artifact_counted: bool,
    weights_counted: bool,
) -> Result<SavedBrainPublicationV1, String> {
    let bytes = read_artifact_file(source)?;
    let admitted = validate_brain_artifact(&bytes)?;
    if admitted.artifact_sha256 != expected_artifact_sha256 {
        return Err("saved brain source changed after selection".into());
    }
    // Bound the complete native metadata response before creating either final
    // object. Reserve ample room for shared descriptors and escaped owner labels.
    let metadata_bytes = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1, "artifact": admitted.descriptor(),
        "graphManifest": &admitted.canonical_manifest, "provenance": &admitted.provenance,
    }))
    .map_err(|error| error.to_string())?
    .len();
    if metadata_bytes > 1200 * 1024 - 4096 {
        return Err("saved brain metadata exceeds catalog admission".into());
    }
    let artifact_bytes = if artifact_counted {
        0
    } else {
        bytes.len() as u64
    };
    let remaining = available_bytes
        .checked_sub(artifact_bytes)
        .ok_or("saved brain exceeds library stored-byte allowance")?;
    let directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let filename = format!("{}.slither-brain", admitted.artifact_sha256);
    let final_path = directory.join(&filename);
    // A collision with an invalid object must fail before any new publication.
    if final_path.exists() {
        validate_existing(&final_path, &admitted.artifact_sha256, bytes.len())?;
    }
    let mut limits = super::fresh_run::production_checkpoint_limits();
    limits.max_weight_floats = crate::contracts::MAX_BRAIN_PARAMETERS;
    limits.max_numeric_stored_bytes = crate::contracts::MAX_BRAIN_WEIGHT_BYTES;
    limits.max_numeric_candidate_bytes = crate::contracts::MAX_BRAIN_NUMERIC_CANDIDATE_BYTES;
    let weights = publish_managed_neural_weights(
        &directory,
        operation,
        &admitted.weights,
        &limits,
        if weights_counted { u64::MAX } else { remaining },
    )
    .map_err(|error| error.to_string())?;
    if !final_path.exists() {
        let partial_path = directory.join(format!(".{}.saved-brain.partial", operation.as_str()));
        let mut partial = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial_path)
            .map_err(|error| error.to_string())?;
        let cleanup = PartialArtifact(partial_path);
        partial
            .write_all(&bytes)
            .and_then(|()| partial.sync_all())
            .map_err(|error| error.to_string())?;
        drop(partial);
        match rename_noreplace(&cleanup.0, &final_path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.to_string()),
        }
        sync_parent_directory(&directory).map_err(|error| error.to_string())?;
    }
    validate_existing(&final_path, &admitted.artifact_sha256, bytes.len())?;
    Ok(SavedBrainPublicationV1 {
        schema_version: 1,
        artifact: admitted.descriptor(),
        graph_manifest: admitted.canonical_manifest,
        provenance: admitted.provenance,
        weights,
        artifact_relative_filename: filename,
        artifact_byte_count: format!("{:016x}", bytes.len()),
    })
}

/// Fully hash an existing original; size and regular-file checks precede reading.
fn validate_existing(path: &Path, digest: &str, size: usize) -> Result<(), String> {
    let existing = read_artifact_file(path)?;
    if existing.len() != size || format!("{:x}", Sha256::digest(&existing)) != digest {
        return Err("saved brain immutable artifact identity mismatch".into());
    }
    Ok(())
}

/// Remove only this operation's partial path on every return/unwind.
struct PartialArtifact(PathBuf);
impl Drop for PartialArtifact {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests;
