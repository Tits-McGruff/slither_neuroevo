//! Bounded portable originals explicitly associated with a save archive.

use super::brain_artifact::{validate_brain_artifact, MAX_BRAIN_ARTIFACT_BYTES};
use super::brain_library::read_artifact_file;
use super::checkpoint::CheckpointError;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) const MAX_SAVED_BRAINS: usize = 100;
pub(crate) const MAX_SAVED_BRAIN_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Catalog membership only; graph, weights and provenance stay in the original artifact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedBrainArchiveEntryV1 {
    pub brain_id: String,
    pub name: String,
    pub created_at_ms: u64,
    pub artifact_byte_count: String,
}

impl SavedBrainArchiveEntryV1 {
    pub(crate) fn bytes(&self) -> Result<u64, CheckpointError> {
        if self.artifact_byte_count.len() != 16 || !self.artifact_byte_count.bytes().all(is_hex) {
            return Err(invalid("invalid original byte count"));
        }
        u64::from_str_radix(&self.artifact_byte_count, 16)
            .map_err(|_| invalid("invalid original byte count"))
    }

    pub(crate) fn path(&self) -> String {
        format!("brains/{}.slither-brain", self.brain_id)
    }

    pub(crate) fn filename(&self) -> String {
        format!("{}.slither-brain", self.brain_id)
    }
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

pub(crate) fn invalid(message: impl Into<String>) -> CheckpointError {
    CheckpointError::format("SAVED_BRAIN_ARCHIVE", message)
}

/// Admit the complete selection before opening any object or creating scratch files.
pub(crate) fn validate_selection(
    entries: &[SavedBrainArchiveEntryV1],
) -> Result<u64, CheckpointError> {
    if entries.len() > MAX_SAVED_BRAINS {
        return Err(invalid("saved brain selection exceeds 100 records"));
    }
    let mut total = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        let bytes = entry.bytes()?;
        if entry.brain_id.len() != 64
            || !entry.brain_id.bytes().all(is_hex)
            || (index > 0 && entries[index - 1].brain_id >= entry.brain_id)
            || entry.name.trim().is_empty()
            || entry.name.trim() != entry.name
            || entry.name.contains('\0')
            || entry.name.len() > 256
            || entry.created_at_ms > 9_007_199_254_740_991
            || !(88..=MAX_BRAIN_ARTIFACT_BYTES as u64).contains(&bytes)
        {
            return Err(invalid(
                "saved brain metadata, order or source size is invalid",
            ));
        }
        total = total
            .checked_add(bytes)
            .ok_or_else(|| invalid("original byte total overflow"))?;
        if total > MAX_SAVED_BRAIN_BYTES {
            return Err(invalid("selected originals exceed two GiB"));
        }
    }
    Ok(total)
}

/// Native graph/sensor/finite-weight admission plus exact original-byte identity.
pub(crate) fn validate_original(
    bytes: &[u8],
    entry: &SavedBrainArchiveEntryV1,
) -> Result<(), CheckpointError> {
    if bytes.len() as u64 != entry.bytes()? {
        return Err(invalid("saved brain original length changed"));
    }
    let admitted = validate_brain_artifact(bytes).map_err(invalid)?;
    if admitted.artifact_sha256 != entry.brain_id {
        return Err(invalid("saved brain original identity changed"));
    }
    Ok(())
}

/// Check a managed source independently of metadata-worker admission.
pub(crate) fn validate_file(
    directory: &Path,
    entry: &SavedBrainArchiveEntryV1,
) -> Result<(), CheckpointError> {
    let bytes = read_artifact_file(&directory.join(entry.filename())).map_err(invalid)?;
    validate_original(&bytes, entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture() -> (Vec<u8>, SavedBrainArchiveEntryV1) {
        let bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/portable-dense-basic8-v1.slither-brain"),
        )
        .unwrap();
        let entry = SavedBrainArchiveEntryV1 {
            brain_id: format!("{:x}", Sha256::digest(&bytes)),
            name: "Portable dense".into(),
            created_at_ms: 42,
            artifact_byte_count: format!("{:016x}", bytes.len()),
        };
        (bytes, entry)
    }

    #[test]
    fn selection_rejects_identity_duplicates_labels_counts_and_total_bytes() {
        let (_, entry) = fixture();
        assert_eq!(validate_selection(&[]).unwrap(), 0);
        assert_eq!(
            validate_selection(std::slice::from_ref(&entry)).unwrap(),
            entry.bytes().unwrap()
        );
        assert!(validate_selection(&[entry.clone(), entry.clone()]).is_err());
        for name in ["", " x", "x\0", &"x".repeat(257)] {
            let mut bad = entry.clone();
            bad.name = name.into();
            assert!(validate_selection(&[bad]).is_err());
        }
        let mut many = Vec::new();
        for id in 0..100 {
            let mut next = entry.clone();
            next.brain_id = format!("{id:064x}");
            next.artifact_byte_count = format!("{:016x}", MAX_BRAIN_ARTIFACT_BYTES);
            many.push(next);
        }
        assert!(validate_selection(&many).is_err());
        for next in &mut many {
            next.artifact_byte_count = "0000000000000058".into();
        }
        assert!(validate_selection(&many).is_ok());
        many.push(entry);
        assert!(validate_selection(&many).is_err());
    }

    #[test]
    fn original_validation_requires_exact_bytes_and_native_contracts() {
        let (mut bytes, mut entry) = fixture();
        validate_original(&bytes, &entry).unwrap();
        entry.brain_id = "0".repeat(64);
        assert!(validate_original(&bytes, &entry).is_err());
        bytes[0] ^= 1;
        entry.brain_id = format!("{:x}", Sha256::digest(&bytes));
        assert!(validate_original(&bytes, &entry).is_err());
    }
}
