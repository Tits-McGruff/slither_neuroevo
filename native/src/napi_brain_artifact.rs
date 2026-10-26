//! Cold file validation on the caller's dedicated persistence/diagnostic worker.
use crate::engine::brain_artifact::validate_brain_artifact;
use crate::engine::brain_library::{publish_saved_brain_file, read_artifact_file};
use crate::engine::checkpoint::CheckpointOperationId;
use napi::{Error, Result, Status};
use napi_derive::napi;
use std::path::Path;

/// Independently compile and validate one bounded immutable portable brain file.
#[napi(js_name = "validateBrainArtifactFile")]
pub fn validate_brain_artifact_file(path: String, register_shadow: Option<bool>) -> Result<String> {
    if path.is_empty() || path.len() > 32_768 || path.contains('\0') {
        return Err(Error::new(
            Status::InvalidArg,
            "invalid brain artifact path",
        ));
    }
    let path = Path::new(&path);
    let bytes = read_artifact_file(path).map_err(|error| Error::new(Status::InvalidArg, error))?;
    let admitted =
        validate_brain_artifact(&bytes).map_err(|error| Error::new(Status::InvalidArg, error))?;
    let descriptor = serde_json::to_string(&admitted.descriptor())
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
    if register_shadow == Some(true) {
        let candidate = crate::engine::shadows::ShadowCandidate::from_artifact(admitted)
            .map_err(|error| Error::new(Status::InvalidArg, error))?;
        crate::engine::shadows::register_candidate(candidate)
            .map_err(|error| Error::new(Status::InvalidArg, error))?;
    }
    Ok(descriptor)
}

/// Publish one explicitly saved portable brain on the isolated metadata worker.
#[napi(js_name = "publishSavedBrainFile", catch_unwind)]
pub fn publish_saved_brain(
    path: String,
    managed_directory: String,
    operation_id: String,
    expected_artifact_sha256: String,
    available_byte_count: String,
    artifact_counted: bool,
    weights_counted: bool,
) -> Result<String> {
    for value in [&path, &managed_directory] {
        if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
            return Err(Error::new(Status::InvalidArg, "invalid saved brain path"));
        }
    }
    if available_byte_count.len() != 16
        || !available_byte_count
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            "invalid library byte allowance",
        ));
    }
    if expected_artifact_sha256.len() != 64
        || !expected_artifact_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            "invalid selected artifact identity",
        ));
    }
    let available = u64::from_str_radix(&available_byte_count, 16)
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
    if available > 2 * 1024 * 1024 * 1024 {
        return Err(Error::new(
            Status::InvalidArg,
            "library byte allowance exceeds admission",
        ));
    }
    let operation = CheckpointOperationId::parse(operation_id)
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
    let publication = publish_saved_brain_file(
        Path::new(&path),
        Path::new(&managed_directory),
        &operation,
        &expected_artifact_sha256,
        available,
        artifact_counted,
        weights_counted,
    )
    .map_err(|error| Error::new(Status::InvalidArg, error))?;
    serde_json::to_string(&publication)
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
}
