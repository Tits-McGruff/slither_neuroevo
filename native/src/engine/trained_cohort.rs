//! Cold qualification-only policy families. All execution remains in the ordinary native loop.
use super::brain_artifact::{
    validate_brain_artifact, BrainArtifactDescriptorV1, ValidatedBrainArtifact,
    MAX_BRAIN_ARTIFACT_BYTES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path};

const MAX_INDEX_BYTES: usize = 64 * 1024;
const MAX_FAMILIES: usize = 16;
const MAX_TOTAL_ENCODED: u64 = MAX_BRAIN_ARTIFACT_BYTES as u64;
/// Bounded decoder/JSON/compiler temporaries while already validated sources remain resident.
const VALIDATION_PEAK: usize = 4 * MAX_BRAIN_ARTIFACT_BYTES;
/// Qualification input role only; this does not alter native/checkpoint scratch ceilings.
const MAX_SOURCE_RESERVATION: usize = 512 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Index {
    schema_version: u32,
    artifacts: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    file: String,
    sha256: String,
}

/// Source metadata is produced once; no provenance strings are copied during physical steps.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohortDescriptor {
    pub schema_version: u32,
    pub index_sha256: String,
    pub source_reservation_bytes: usize,
    pub assignment: &'static str,
    pub families: Vec<FamilyDescriptor>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyDescriptor {
    pub artifact: BrainArtifactDescriptorV1,
    pub population_slots: usize,
}

/// Private validated inputs remain owned through cold population construction.
pub(crate) struct TrainedCohort {
    pub artifacts: Vec<ValidatedBrainArtifact>,
    index_sha256: String,
    pub source_reservation_bytes: usize,
}
impl TrainedCohort {
    pub fn descriptor(&self, population: usize) -> CohortDescriptor {
        CohortDescriptor {
            schema_version: 1,
            index_sha256: self.index_sha256.clone(),
            source_reservation_bytes: self.source_reservation_bytes,
            assignment: "populationSlot % familyCount; index order is significant",
            families: self
                .artifacts
                .iter()
                .enumerate()
                .map(|(family, artifact)| FamilyDescriptor {
                    artifact: artifact.descriptor(),
                    population_slots: (0..population)
                        .filter(|slot| slot % self.artifacts.len() == family)
                        .count(),
                })
                .collect(),
        }
    }
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum as u64 {
        return Err("cohort bounded ordinary file required".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > maximum {
        return Err("cohort file grew past admission".into());
    }
    Ok(bytes)
}

/// Graph allocation accounting is shared with authority admission. Canonical/provenance JSON
/// receives conservative 64x dynamic metadata headroom (including Value trees and capacities),
/// plus eight MiB for fixed records/sensor semantics; numeric weight capacity is charged exactly.
fn source_bound(artifact: &ValidatedBrainArtifact) -> Result<usize, String> {
    let graph = super::state::estimate_graph_memory(&artifact.graph).map_err(|e| e.to_string())?;
    let canonical = artifact
        .canonical_manifest
        .canonical_bytes()
        .map_err(|e| e.to_string())?
        .len();
    let provenance = serde_json::to_vec(&artifact.provenance)
        .map_err(|e| e.to_string())?
        .len();
    canonical
        .checked_add(provenance)
        .and_then(|bytes| bytes.checked_mul(64))
        .and_then(|bytes| bytes.checked_add(8 * 1024 * 1024))
        .and_then(|bytes| bytes.checked_add(graph))
        .and_then(|bytes| {
            artifact
                .weights
                .capacity()
                .checked_mul(4)
                .and_then(|weights| bytes.checked_add(weights))
        })
        .ok_or_else(|| "cohort source byte overflow".into())
}

/// Every file is a confined sibling of a bounded index, independently hashed and Rust-validated.
/// All families must share exact graph/sensor/cadence/life contracts and distinct packed weights.
pub(crate) fn load(path: &Path) -> Result<TrainedCohort, String> {
    let bytes = read_bounded(path, MAX_INDEX_BYTES)?;
    let index: Index = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if index.schema_version != 1 || !(2..=MAX_FAMILIES).contains(&index.artifacts.len()) {
        return Err("cohort index requires schema 1 and 2..16 families".into());
    }
    let parent = path
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("cohort parent missing")?
        .to_owned();
    let mut names = BTreeSet::new();
    let mut encoded = 0u64;
    for entry in &index.artifacts {
        let components: Vec<_> = Path::new(&entry.file).components().collect();
        if !matches!(components.as_slice(), [Component::Normal(_)])
            || entry.file.contains(['\0', ':', '\\', '/'])
            || !entry.file.ends_with(".slither-brain")
            || !names.insert(entry.file.clone())
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("cohort requires unique confined filenames and lowercase SHA-256".into());
        }
        let metadata =
            std::fs::symlink_metadata(parent.join(&entry.file)).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("cohort source is not an ordinary file".into());
        }
        encoded = encoded
            .checked_add(metadata.len())
            .ok_or("cohort encoded byte overflow")?;
        if encoded > MAX_TOTAL_ENCODED {
            return Err("cohort combined encoded byte admission".into());
        }
    }
    let mut artifacts: Vec<ValidatedBrainArtifact> = Vec::with_capacity(index.artifacts.len());
    let mut weights = BTreeSet::new();
    let mut resident = bytes.capacity();
    let mut remaining_encoded = MAX_TOTAL_ENCODED as usize;
    for entry in index.artifacts {
        if resident
            .checked_add(VALIDATION_PEAK)
            .is_none_or(|total| total > MAX_SOURCE_RESERVATION)
        {
            return Err("cohort validation source byte admission".into());
        }
        // Enforce the combined admission on actual reads as well as preflight metadata.
        let encoded = read_bounded(
            &parent.join(entry.file),
            remaining_encoded.min(MAX_BRAIN_ARTIFACT_BYTES),
        )?;
        remaining_encoded = remaining_encoded
            .checked_sub(encoded.len())
            .ok_or("cohort encoded byte overflow")?;
        if format!("{:x}", Sha256::digest(&encoded)) != entry.sha256 {
            return Err("cohort artifact digest mismatch".into());
        }
        let artifact = validate_brain_artifact(&encoded)?;
        if !weights.insert(artifact.weights_sha256.clone()) {
            return Err("cohort requires distinct packed policy weights".into());
        }
        if let Some(first) = artifacts.first() {
            if first.canonical_manifest.architecture_key
                != artifact.canonical_manifest.architecture_key
                || first.canonical_manifest.sensor_sha256
                    != artifact.canonical_manifest.sensor_sha256
                || first.deployment.control_interval_seconds
                    != artifact.deployment.control_interval_seconds
                || first.deployment.life_state_rule != artifact.deployment.life_state_rule
            {
                return Err("cohort graph/sensor/cadence/life contract mismatch".into());
            }
        }
        resident = resident
            .checked_add(source_bound(&artifact)?)
            .ok_or("cohort resident byte overflow")?;
        artifacts.push(artifact);
    }
    let source_reservation_bytes = resident
        .checked_add(VALIDATION_PEAK)
        .filter(|total| *total <= MAX_SOURCE_RESERVATION)
        .ok_or("cohort total source byte admission")?;
    Ok(TrainedCohort {
        artifacts,
        source_reservation_bytes,
        index_sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}

/// Synthetic finite variants of the frozen native fixture, encoded and revalidated only in tests.
#[cfg(test)]
pub(crate) fn fixture_bytes(delta: f32, cadence: f64) -> Vec<u8> {
    use super::training_graph::CanonicalTrainingGraphManifestV1;
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/portable-brain-v1.slither-brain"),
    )
    .unwrap();
    let mut source = validate_brain_artifact(&bytes).unwrap();
    source.weights[0] += delta;
    source.deployment.control_interval_seconds = cadence;
    let manifest =
        CanonicalTrainingGraphManifestV1::build(&source.graph, &source.sensors, source.deployment)
            .unwrap();
    let json = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "graphManifest": serde_json::from_slice::<serde_json::Value>(&manifest.canonical_bytes().unwrap()).unwrap(),
        "provenance": source.provenance,
    })).unwrap();
    let raw: Vec<u8> = source
        .weights
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let mut output = Vec::from(b"SLTHBRN1".as_slice());
    output.extend_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(&[0, 0, 0, 0]);
    output.extend_from_slice(&(source.weights.len() as u32).to_le_bytes());
    output.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    output.extend_from_slice(&Sha256::digest(&raw));
    output.extend_from_slice(&Sha256::digest(&raw));
    output.extend(json);
    output.extend(raw);
    validate_brain_artifact(&output).unwrap();
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "slither-cohort-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn write(&self, second: &[u8]) -> Value {
            let first = fixture_bytes(0.0, 0.06);
            std::fs::write(self.0.join("first.slither-brain"), &first).unwrap();
            std::fs::write(self.0.join("second.slither-brain"), second).unwrap();
            json!({"schemaVersion":1,"artifacts":[
                {"file":"first.slither-brain","sha256":format!("{:x}",Sha256::digest(&first))},
                {"file":"second.slither-brain","sha256":format!("{:x}",Sha256::digest(second))}
            ]})
        }
        fn load(&self, index: &Value) -> Result<TrainedCohort, String> {
            std::fs::write(
                self.0.join("cohort.json"),
                serde_json::to_vec(index).unwrap(),
            )
            .unwrap();
            load(&self.0.join("cohort.json"))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            assert_eq!(self.0.canonicalize().unwrap(), self.0);
            assert!(!std::fs::symlink_metadata(&self.0)
                .unwrap()
                .file_type()
                .is_symlink());
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn admits_distinct_compatible_sources_with_explicit_accounting_and_slot_counts() {
        let scratch = Scratch::new();
        let index = scratch.write(&fixture_bytes(0.125, 0.06));
        let cohort = scratch.load(&index).unwrap();
        assert_eq!(cohort.artifacts.len(), 2);
        assert!(cohort.source_reservation_bytes > VALIDATION_PEAK);
        assert!(cohort.source_reservation_bytes <= MAX_SOURCE_RESERVATION);
        let descriptor = cohort.descriptor(55);
        assert_eq!(descriptor.families[0].population_slots, 28);
        assert_eq!(descriptor.families[1].population_slots, 27);
        assert_ne!(
            descriptor.families[0].artifact.weights_sha256,
            descriptor.families[1].artifact.weights_sha256
        );
    }
    #[test]
    fn rejects_aliases_duplicate_weights_digest_damage_contract_mismatch_and_bounds() {
        let scratch = Scratch::new();
        let valid = scratch.write(&fixture_bytes(0.125, 0.06));
        for file in [
            "../first.slither-brain",
            "a/first.slither-brain",
            "a\\first.slither-brain",
            "C:first.slither-brain",
            "first.slither-brain",
        ] {
            let mut index = valid.clone();
            index["artifacts"][1]["file"] = json!(file);
            assert!(scratch.load(&index).is_err(), "{file}");
        }
        let mut index = valid.clone();
        index["artifacts"][1]["sha256"] = json!("0".repeat(64));
        assert!(scratch.load(&index).err().unwrap().contains("digest"));
        index = valid.clone();
        index["unexpected"] = json!(true);
        assert!(scratch.load(&index).is_err());
        for count in [0, 1, 17] {
            index = valid.clone();
            index["artifacts"] = json!(vec![valid["artifacts"][0].clone(); count]);
            assert!(scratch.load(&index).is_err());
        }
        index = scratch.write(&fixture_bytes(0.0, 0.06));
        assert!(scratch
            .load(&index)
            .err()
            .unwrap()
            .contains("distinct packed"));
        index = scratch.write(&fixture_bytes(0.125, 0.03));
        assert!(scratch
            .load(&index)
            .err()
            .unwrap()
            .contains("contract mismatch"));
        std::fs::write(
            scratch.0.join("cohort.json"),
            vec![b' '; MAX_INDEX_BYTES + 1],
        )
        .unwrap();
        assert!(load(&scratch.0.join("cohort.json")).is_err());
        let index = scratch.write(&fixture_bytes(0.125, 0.06));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(scratch.0.join("second.slither-brain"))
            .unwrap();
        file.set_len(MAX_TOTAL_ENCODED + 1).unwrap();
        assert!(scratch.load(&index).err().unwrap().contains("encoded byte"));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_link_sources() {
        let scratch = Scratch::new();
        let index = scratch.write(&fixture_bytes(0.125, 0.06));
        std::fs::remove_file(scratch.0.join("second.slither-brain")).unwrap();
        std::os::unix::fs::symlink(
            "first.slither-brain",
            scratch.0.join("second.slither-brain"),
        )
        .unwrap();
        assert!(scratch
            .load(&index)
            .err()
            .unwrap()
            .contains("ordinary file"));
    }
}
