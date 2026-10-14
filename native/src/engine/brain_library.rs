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
    limits.max_weight_floats = 1_000_000;
    limits.max_numeric_stored_bytes = 4_000_000;
    limits.max_numeric_candidate_bytes = 4_000_000;
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
mod tests {
    use super::super::checkpoint::read_validated_hall_of_fame_weights;
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "slither-brain-library-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn source() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/portable-brain-v1.slither-brain")
    }
    fn operation() -> CheckpointOperationId {
        CheckpointOperationId::parse("a1".repeat(16)).unwrap()
    }
    fn digest() -> String {
        validate_brain_artifact(&fs::read(source()).unwrap())
            .unwrap()
            .artifact_sha256
    }

    #[test]
    fn saved_publication_preserves_original_bytes_and_reuses_shared_weight_objects() {
        let directory = TestDirectory::new();
        let publication = publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            2 * 1024 * 1024 * 1024,
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            fs::read(directory.0.join(&publication.artifact_relative_filename)).unwrap(),
            fs::read(source()).unwrap()
        );
        let original = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
        assert_eq!(
            read_validated_hall_of_fame_weights(
                &directory.0.join(&publication.weights.relative_filename),
                &publication.weights
            )
            .unwrap()
            .as_ref(),
            original.weights.as_slice()
        );
        let repeated = publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            0,
            true,
            true,
        )
        .unwrap();
        assert_eq!(publication.weights, repeated.weights);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
        let json = serde_json::to_value(&publication).unwrap();
        assert_eq!(
            json["weights"]["encoding"],
            publication.weights.encoding.as_str()
        );
        assert_eq!(
            json["weights"]["storedByteCount"],
            publication.weights.stored_byte_count_hex
        );
        assert!(json["graphManifest"]["graphSpec"].is_object());
    }

    #[test]
    fn source_identity_and_stored_quota_reject_before_final_objects_are_published() {
        let directory = TestDirectory::new();
        let length = fs::metadata(source()).unwrap().len();
        assert!(publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &"00".repeat(32),
            u64::MAX,
            false,
            false
        )
        .is_err());
        assert!(publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            length - 1,
            false,
            false
        )
        .is_err());
        let error = publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            length,
            false,
            false,
        )
        .err()
        .unwrap();
        assert!(error.contains("allowance"), "{error}");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn library_reuses_older_raw_hof_bytes_when_the_adaptive_codec_prefers_compression() {
        let directory = TestDirectory::new();
        let admitted = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
        let path = directory
            .0
            .join(format!("{}.hof-weights-v1", admitted.weights_sha256));
        let original = admitted
            .weights
            .iter()
            .flat_map(|value| value.to_bits().to_le_bytes())
            .collect::<Vec<_>>();
        fs::write(&path, &original).unwrap();
        let publication = publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            u64::MAX,
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            publication.weights.encoding,
            super::super::checkpoint::NumericEncoding::RawF32LeV1
        );
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn existing_portable_original_is_immutable_even_when_length_matches() {
        let directory = TestDirectory::new();
        let publication = publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            u64::MAX,
            false,
            false,
        )
        .unwrap();
        let path = directory.0.join(&publication.artifact_relative_filename);
        let mut corrupt = fs::read(&path).unwrap();
        corrupt[0] ^= 1;
        fs::write(&path, &corrupt).unwrap();
        assert!(publish_saved_brain_file(
            &source(),
            &directory.0,
            &operation(),
            &digest(),
            u64::MAX,
            true,
            true
        )
        .is_err());
        assert_eq!(fs::read(path).unwrap(), corrupt);
    }
}
