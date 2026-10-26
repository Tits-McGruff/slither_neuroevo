//! Offline full-catalog fixtures and actual inspector publication for capacity qualification.
use super::brain_artifact::BRAIN_ARTIFACT_HEADER_BYTES;
use super::graph::GraphBundle;
use super::running_loop::RunningAuthorityLoop;
use super::sensor_layout::SensorLayout;
use super::state::{AuthoritativeState, NormalizedSettingValue};
use super::training_graph::{
    CanonicalTrainingGraphManifestV1, TrainingDeployment, TrainingLifeStateRule,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

pub const LIBRARY_RECORDS: usize = super::saved_brain_archive::MAX_SAVED_BRAINS;

/// Inputs, managed originals/shared weights, archive, and overlapping import staging.
/// The fixture always writes raw originals; one million weights plus the admitted
/// manifest/header bounds each original and its shared weight object.
pub fn additional_disk_reserve_bytes() -> u64 {
    8 * (LIBRARY_RECORDS as u64 + 1)
        * (4_000_000 + 1024 * 1024 + BRAIN_ARTIFACT_HEADER_BYTES as u64)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRecord {
    brain_id: String,
    weights_sha256: String,
    name: String,
    relative_filename: String,
    artifact_byte_count: String,
    parameter_count: usize,
}

/// Emit canonical packed bytes with bounded scratch, never another population.
fn raw_chunks(
    weights: &[f32],
    ordinal: usize,
    mut consume: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut scratch = [0u8; 64 * 1024];
    for (chunk_index, chunk) in weights.chunks(scratch.len() / 4).enumerate() {
        for (index, original) in chunk.iter().enumerate() {
            let value = if chunk_index == 0 && index == 0 {
                ordinal as f32 / 128.0
            } else {
                *original
            };
            scratch[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        consume(&scratch[..chunk.len() * 4])?;
    }
    Ok(())
}

/// Fixture envelope only. Rust's production reader/compiler independently validates
/// every original again during catalog save and archive export/import.
fn write_fixture(
    directory: &Path,
    manifest: &CanonicalTrainingGraphManifestV1,
    weights: &[f32],
    ordinal: usize,
) -> Result<FixtureRecord, Box<dyn std::error::Error>> {
    let graph_manifest: serde_json::Value = serde_json::from_slice(&manifest.canonical_bytes()?)?;
    let metadata = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1, "graphManifest": graph_manifest,
        "provenance": {"schemaVersion":1,"sourceKind":"manual",
            "details":{"qualification":"checkpoint-library-inspector-v1","ordinal":ordinal,"learned":false}}
    }))?;
    let mut raw_hash = Sha256::new();
    raw_chunks(weights, ordinal, |bytes| {
        raw_hash.update(bytes);
        Ok(())
    })?;
    let raw_digest = raw_hash.finalize();
    let raw_bytes = weights
        .len()
        .checked_mul(4)
        .ok_or("fixture weight byte overflow")?;
    let mut header = [0u8; BRAIN_ARTIFACT_HEADER_BYTES];
    header[..8].copy_from_slice(b"SLTHBRN1");
    header[8..12].copy_from_slice(&u32::try_from(metadata.len())?.to_le_bytes());
    header[16..20].copy_from_slice(&u32::try_from(weights.len())?.to_le_bytes());
    header[20..24].copy_from_slice(&u32::try_from(raw_bytes)?.to_le_bytes());
    header[24..56].copy_from_slice(&raw_digest);
    header[56..88].copy_from_slice(&raw_digest);
    let relative_filename = format!("library-inputs/{ordinal:03}.slither-brain");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(&relative_filename))?;
    let mut writer = BufWriter::new(file);
    writer.write_all(&header)?;
    writer.write_all(&metadata)?;
    let mut hash = Sha256::new();
    hash.update(header);
    hash.update(&metadata);
    raw_chunks(weights, ordinal, |bytes| {
        hash.update(bytes);
        writer.write_all(bytes)
    })?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(FixtureRecord {
        brain_id: format!("{:x}", hash.finalize()),
        weights_sha256: format!("{raw_digest:x}"),
        name: format!("Capacity manual brain {ordinal:03}"),
        relative_filename,
        artifact_byte_count: format!("{:016x}", header.len() + metadata.len() + raw_bytes),
        parameter_count: weights.len(),
    })
}

/// Produce the complete catalog plus one valid overflow probe before any metadata save.
/// Only the first 100 are retained; the host must prove the 101st save rejects.
pub(crate) fn write_library_fixtures(
    authority: &AuthoritativeState,
    options: &super::sensor_qualification::SensorQualificationOptions,
    graph_limits: &super::graph::GraphLimits,
    directory: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let graph = GraphBundle::compile(authority.graph_spec().clone(), graph_limits)?;
    let sensors = SensorLayout::for_profile(options.profile, options.bins)?;
    let control_interval_seconds = match authority
        .state()
        .config
        .settings
        .iter()
        .find(|setting| setting.path == "brain.controlDt")
        .map(|setting| &setting.value)
    {
        Some(NormalizedSettingValue::Float(value)) => *value,
        _ => return Err("capacity source has no admitted neural cadence".into()),
    };
    let manifest = CanonicalTrainingGraphManifestV1::build(
        &graph,
        &sensors,
        TrainingDeployment {
            control_interval_seconds,
            life_state_rule: TrainingLifeStateRule::Reset,
            initialization: super::genome::GenomeInitializationConfig::default(),
        },
    )?;
    fs::create_dir(directory.join("library-inputs"))?;
    let mut records = Vec::with_capacity(LIBRARY_RECORDS + 1);
    for ordinal in 0..=LIBRARY_RECORDS {
        let weights =
            &authority.state().population[ordinal % authority.state().population.len()].weights;
        records.push(write_fixture(directory, &manifest, weights, ordinal)?);
    }
    let original_bytes: u64 = records
        .iter()
        .map(|entry| u64::from_str_radix(&entry.artifact_byte_count, 16))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum();
    let index_path = directory.join("library-inputs/index.json");
    let mut index = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(index_path)?;
    serde_json::to_writer(
        &mut index,
        &serde_json::json!({"schemaVersion":1,"records":records}),
    )?;
    index.write_all(b"\n")?;
    index.sync_all()?;
    Ok(
        serde_json::json!({"recordCount":LIBRARY_RECORDS,"overflowProbes":1,
        "originalInputBytes":original_bytes,"indexRelativeFilename":"library-inputs/index.json"}),
    )
}

/// Select the real delivered population observation; this performs no probe or inference.
pub(crate) fn enable_inspector(
    running: &mut RunningAuthorityLoop,
    run_id: &str,
) -> Result<(), String> {
    let selected = running
        .select_entity(
            super::state::EntitySelectionTarget::PopulationSlot(0),
            run_id,
            running.generation(),
        )?
        .ok_or("capacity inspector population target missing")?;
    running.configure_inspector_subscription(
        run_id,
        Some((selected.token, selected.frame_v1_id)),
        super::inspector::MAX_INSPECTOR_BYTES,
        running.generation(),
        false,
    )
}

/// Exercise the production replaceable mailbox and complete wire encoding beside
/// retained population state. Caller keeps one cache alive through import work;
/// the explicit presentation clock is independent of the fixed-step simulation clock.
pub(crate) fn inspector_receipt(
    running: &RunningAuthorityLoop,
    display: &super::display::RunningDisplayCache,
    presentation_ms: u64,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    display.publish_inspector(running, presentation_ms)?;
    let snapshot = display
        .latest_inspector(0)?
        .ok_or("capacity inspector did not publish")?;
    let observation = snapshot
        .observation
        .as_ref()
        .ok_or("capacity inspector observation absent")?;
    if observation.pure_probe
        || observation.normalized_inputs().is_empty()
        || observation.trace.is_none()
    {
        return Err("capacity inspector did not retain the real delivered trace".into());
    }
    if snapshot.world_epoch != running.world_epoch()
        || snapshot.completed_step != running.completed_step()
    {
        return Err("capacity inspector retained a foreign publication".into());
    }
    display.publish_inspector(running, presentation_ms + 99)?;
    if display.latest_inspector(snapshot.sequence)?.is_some() {
        return Err("capacity inspector did not suppress a duplicate publication".into());
    }
    let wire = super::inspector_wire::encode(&snapshot)?;
    let receipt = serde_json::json!({"completedStep":running.completed_step(),
        "inputSize":observation.normalized_inputs().len(),"sampleStep":observation.sample_step(),
        "wireBytes":wire.len(),"pureProbe":false,"tracePresent":true,"duplicatePublicationSuppressed":true});
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_catalog_reserve_prices_originals_and_all_staging_copies() {
        assert_eq!(LIBRARY_RECORDS, 100);
        assert!(additional_disk_reserve_bytes() > 4_000_000 * 101 * 8);
    }

    #[test]
    fn fixture_uses_canonical_native_layout_and_distinct_valid_float_weights() {
        let directory = std::env::temp_dir().join(format!(
            "slither-library-capacity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::create_dir(directory.join("library-inputs")).unwrap();
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let old = fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures/portable-dense-basic8-v1.slither-brain"),
            )?;
            let source = super::super::brain_artifact::validate_brain_artifact(&old)?;
            let a = write_fixture(&directory, &source.canonical_manifest, &source.weights, 0)?;
            let b = write_fixture(&directory, &source.canonical_manifest, &source.weights, 1)?;
            assert_ne!(a.brain_id, b.brain_id);
            assert_ne!(a.weights_sha256, b.weights_sha256);
            for record in [a, b] {
                let bytes = fs::read(directory.join(record.relative_filename))?;
                let validated = super::super::brain_artifact::validate_brain_artifact(&bytes)?;
                assert_eq!(validated.artifact_sha256, record.brain_id);
                assert_eq!(validated.weights_sha256, record.weights_sha256);
                assert_eq!(validated.weights.len(), source.weights.len());
                assert_eq!(validated.weights[1..], source.weights[1..]);
            }
            Ok(())
        })();
        let root = std::env::temp_dir().canonicalize().unwrap();
        let owned = directory.canonicalize().unwrap();
        assert!(
            owned.starts_with(root)
                && owned
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("slither-library-capacity-")
        );
        fs::remove_dir_all(owned).unwrap();
        result.unwrap();
    }
}
