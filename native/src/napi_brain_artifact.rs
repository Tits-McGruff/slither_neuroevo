//! Cold file validation on the caller's dedicated persistence/diagnostic worker.
use crate::engine::brain_artifact::{validate_brain_artifact, MAX_BRAIN_ARTIFACT_BYTES};
use napi::{Error, Result, Status};
use napi_derive::napi;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Independently compile and validate one bounded immutable portable brain file.
#[napi(js_name = "validateBrainArtifactFile")]
pub fn validate_brain_artifact_file(path: String) -> Result<String> {
    if path.is_empty() || path.len() > 32_768 || path.contains('\0') {
        return Err(Error::new(
            Status::InvalidArg,
            "invalid brain artifact path",
        ));
    }
    let path = Path::new(&path);
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_BRAIN_ARTIFACT_BYTES as u64
    {
        return Err(Error::new(
            Status::InvalidArg,
            "brain artifact file admission",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_BRAIN_ARTIFACT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
    let admitted =
        validate_brain_artifact(&bytes).map_err(|error| Error::new(Status::InvalidArg, error))?;
    serde_json::to_string(&admitted.descriptor())
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
}
