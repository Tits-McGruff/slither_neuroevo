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
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "slither-brain-library-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

    fn admitted_loop(
        cadence: f64,
        profile: super::super::sensor_contract::SensorProfile,
    ) -> super::super::running_loop::RunningAuthorityLoop {
        let artifact = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
        admitted_graph_loop(cadence, profile, 8, artifact.graph.spec().clone())
    }

    fn admitted_graph_loop(
        cadence: f64,
        profile: super::super::sensor_contract::SensorProfile,
        bins: usize,
        graph: super::super::graph::GraphSpec,
    ) -> super::super::running_loop::RunningAuthorityLoop {
        use super::super::fresh_run::{
            prepare_v4_qualification_run, FreshRunSettingUpdate, Stage6aP0FreshRunRequest,
        };
        use super::super::state::NormalizedSettingValue;
        let settings = super::super::step_config::production_default_settings(12, 0)
            .into_iter()
            .filter_map(|setting| match setting.value {
                NormalizedSettingValue::Float(value) => Some(FreshRunSettingUpdate {
                    path: setting.path,
                    value,
                }),
                NormalizedSettingValue::Integer(value) => Some(FreshRunSettingUpdate {
                    path: setting.path,
                    value: value as f64,
                }),
                NormalizedSettingValue::Bool(value) => Some(FreshRunSettingUpdate {
                    path: setting.path,
                    value: f64::from(u8::from(value)),
                }),
                NormalizedSettingValue::Text(_) => None,
            })
            .map(|mut setting| {
                if setting.path == "brain.controlDt" {
                    setting.value = cadence;
                }
                setting
            })
            .collect::<Vec<_>>();
        let limits = super::super::step_config::RunningStepWorkLimits {
            ray_work: Some(super::super::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
            ..super::super::step_config::RunningStepWorkLimits::provisional_defaults()
        };
        let mut pending = prepare_v4_qualification_run(
            Stage6aP0FreshRunRequest {
                run_id: "saved-brain-live-test".into(),
                seed: 42,
                memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
            },
            &settings,
            graph,
            true,
            profile,
            bins,
            limits,
        )
        .unwrap();
        let directory = TestDirectory::new();
        let checkpoint = pending
            .publish_checkpoint(&directory.0, operation())
            .unwrap();
        pending.acknowledge_persistence(&checkpoint).unwrap();
        pending.publish_running_authority().unwrap();
        pending
            .into_running_loop(
                super::super::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
                0,
            )
            .unwrap()
    }

    #[test]
    fn inspector_runtime_caches_due_rows_holds_their_age_and_stops_on_close() {
        use super::super::inspector::{InspectorObservationSource, MAX_INSPECTOR_BYTES};
        use super::super::scheduler::SchedulerServiceMode;
        use super::super::sensor_contract::SensorProfile;
        use super::super::state::EntitySelectionTarget;
        for (profile, bins) in [
            (SensorProfile::Basic, 8),
            (SensorProfile::Basic, 20),
            (SensorProfile::Motion, 8),
            (SensorProfile::Motion, 32),
        ] {
            let width = super::super::sensor_contract::SensorContractIdentity::new(profile, bins)
                .unwrap()
                .input_size();
            let artifact = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
            let mut graph = artifact.graph.spec().clone();
            for node in &mut graph.nodes {
                match &mut node.kind {
                    super::super::graph::GraphNodeKind::Input { output_size } => {
                        *output_size = width
                    }
                    super::super::graph::GraphNodeKind::Gru { input_size, .. } => {
                        *input_size = width
                    }
                    _ => {}
                }
            }
            let mut inspected = admitted_graph_loop(0.06, profile, bins, graph.clone());
            let mut reference = admitted_graph_loop(0.06, profile, bins, graph);
            let selected = inspected
                .select_entity(
                    EntitySelectionTarget::PopulationSlot(0),
                    "saved-brain-live-test",
                    1,
                )
                .unwrap()
                .unwrap();
            inspected
                .configure_inspector_subscription(
                    "saved-brain-live-test",
                    Some((selected.token, selected.frame_v1_id)),
                    MAX_INSPECTOR_BYTES,
                    17,
                )
                .unwrap();
            assert!(inspected.inspector_observation().is_none());
            for running in [&mut inspected, &mut reference] {
                running
                    .service_after_command_drain(17, SchedulerServiceMode::Background, None)
                    .unwrap();
            }
            let display =
                super::super::display::RunningDisplayCache::new(inspected.admitted_frame_bytes())
                    .unwrap();
            display.publish_inspector(&inspected, 0).unwrap();
            let first_mailbox = display.latest_inspector(0).unwrap().unwrap();
            assert_eq!(first_mailbox.subscription_sequence, 17);
            let wire: serde_json::Value = serde_json::from_str(
                &super::super::inspector_wire::encode(&first_mailbox).unwrap(),
            )
            .unwrap();
            let mut exact = first_mailbox;
            exact.sequence = u64::MAX;
            let exact_sample = exact.observation.as_mut().unwrap();
            exact_sample.target.physical_id = u64::MAX;
            let exact_trace = exact_sample.trace.as_mut().unwrap();
            exact_trace.physical_id = u64::MAX;
            exact_trace.kills = u64::MAX;
            exact_trace.head_ids[..bins].fill(u64::MAX);
            let exact_wire: serde_json::Value =
                serde_json::from_str(&super::super::inspector_wire::encode(&exact).unwrap())
                    .unwrap();
            assert_eq!(exact_wire["sequence"], "ffffffffffffffff");
            assert_eq!(exact_wire["sample"]["physicalId"], "ffffffffffffffff");
            assert_eq!(exact_wire["sample"]["trace"]["kills"], "ffffffffffffffff");
            assert_eq!(
                exact_wire["sample"]["trace"]["headIds"][0],
                "ffffffffffffffff"
            );
            assert_eq!(wire["schemaVersion"], 1);
            assert_eq!(wire["sample"]["sampleStep"], "0000000000000001");
            assert_eq!(wire["sample"]["ageSteps"], "0000000000000000");
            assert_eq!(
                wire["sample"]["normalizedInputs"].as_array().unwrap().len(),
                width
            );
            assert_eq!(
                wire["sample"]["trace"]["bodyRays"]
                    .as_array()
                    .unwrap()
                    .len(),
                bins
            );
            let sample = *inspected.inspector_observation().unwrap();
            assert_eq!(sample.source, InspectorObservationSource::Neural);
            assert_eq!(sample.layout.profile, profile);
            assert_eq!(sample.layout.bins, bins);
            assert_eq!(sample.normalized_inputs().len(), width);
            let trace = sample.trace.as_ref().expect("actual due sensor trace");
            assert!(trace.valid);
            assert_eq!(trace.physical_id, sample.target.physical_id);
            assert_eq!((trace.profile, trace.bins), (profile, bins));
            assert!(trace.near_radius > 0.0 && trace.far_radius >= trace.near_radius);
            let life = sample.lifecycle.expect("selected native life assessment");
            assert_eq!(life.life_ordinal, 1);
            assert_eq!(life.deaths, 0);
            assert_eq!(life.assessment.points, sample.raw.points);
            assert!(
                (life.ledger.life_local_wallet + life.ledger.prior_bank_remaining
                    - sample.raw.points)
                    .abs()
                    < 1e-12
            );
            assert!(life.provisional_fitness.is_finite());
            assert_eq!(
                wire["sample"]["lifecycle"]["lifeOrdinal"],
                "0000000000000001"
            );
            assert_eq!(sample.sample_step(), Some(1));
            assert!(sample.raw.first_delivery && sample.neural_outputs.is_some());
            assert!(inspected
                .configure_inspector(
                    "saved-brain-live-test",
                    Some((selected.token, selected.frame_v1_id)),
                    0
                )
                .is_err());
            assert_eq!(
                inspected.inspector_observation().unwrap().sample_step(),
                Some(1)
            );
            for running in [&mut inspected, &mut reference] {
                running
                    .service_after_command_drain(34, SchedulerServiceMode::Background, None)
                    .unwrap();
            }
            assert_eq!(inspected.completed_step(), 2);
            let held = inspected.inspector_observation().unwrap();
            assert_eq!(held.sample_step(), sample.sample_step());
            assert_eq!(held.normalized_inputs(), sample.normalized_inputs());
            assert_eq!(held.age_steps(inspected.completed_step()), Some(1));
            assert_eq!(held.raw, sample.raw);
            assert_eq!(held.trace, sample.trace);
            assert_eq!(held.lifecycle, sample.lifecycle);
            display.publish_inspector(&inspected, 99).unwrap();
            assert!(display
                .latest_inspector(first_mailbox.sequence)
                .unwrap()
                .is_none());
            display.publish_inspector(&inspected, 100).unwrap();
            let aged = display
                .latest_inspector(first_mailbox.sequence)
                .unwrap()
                .unwrap();
            let aged_wire: serde_json::Value =
                serde_json::from_str(&super::super::inspector_wire::encode(&aged).unwrap())
                    .unwrap();
            assert_eq!(
                aged_wire["sample"]["sampleStep"],
                wire["sample"]["sampleStep"]
            );
            assert_eq!(aged_wire["sample"]["ageSteps"], "0000000000000001");
            assert_eq!(aged_wire["sample"]["trace"], wire["sample"]["trace"]);
            let mut invalid = aged;
            invalid
                .observation
                .as_mut()
                .unwrap()
                .trace
                .as_mut()
                .unwrap()
                .fitness = f64::NAN;
            assert!(super::super::inspector_wire::encode(&invalid).is_err());
            inspected
                .configure_inspector("saved-brain-live-test", None, 0)
                .unwrap();
            display.publish_inspector(&inspected, 101).unwrap();
            let cleared = display.latest_inspector(aged.sequence).unwrap().unwrap();
            assert!(cleared.observation.is_none());
            assert_eq!(cleared.subscription_sequence, 17);
            let clear_wire: serde_json::Value =
                serde_json::from_str(&super::super::inspector_wire::encode(&cleared).unwrap())
                    .unwrap();
            assert!(clear_wire["sample"].is_null());
            display.publish_inspector(&inspected, 102).unwrap();
            assert!(display
                .latest_inspector(cleared.sequence)
                .unwrap()
                .is_none());
            for wall in [51, 68, 85, 102] {
                for running in [&mut inspected, &mut reference] {
                    running
                        .service_after_command_drain(wall, SchedulerServiceMode::Background, None)
                        .unwrap();
                }
            }
            assert!(inspected.inspector_observation().is_none());
            assert_eq!(inspected.completed_step(), reference.completed_step());
            let mut actual = Vec::with_capacity(inspected.admitted_frame_bytes());
            let mut expected = Vec::with_capacity(reference.admitted_frame_bytes());
            inspected.pack_display_into(1, &mut actual).unwrap();
            reference.pack_display_into(1, &mut expected).unwrap();
            assert_eq!(
                actual, expected,
                "inspection must preserve native continuation for {profile:?} B{bins}"
            );
        }
    }

    #[test]
    fn saved_deployment_is_native_and_continues_after_its_catalog_files_are_deleted() {
        use super::super::sensor_contract::SensorProfile;
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
        let mut running = admitted_loop(0.06, SensorProfile::Basic);
        let request = SavedBrainSpawnRequest {
            managed_directory: directory.0.to_string_lossy().into_owned(),
            brain_id: digest(),
            expected_run_id: "saved-brain-live-test".into(),
            expected_config_revision: 1,
            expected_world_epoch: running.world_epoch(),
            weights: Box::new(publication.weights.clone()),
        };
        let before = running.completed_step();
        let (spawn, step) = running.spawn_saved_brain(&request).unwrap();
        assert_eq!(step, before + 1);
        assert!(running
            .select_entity(
                super::super::state::EntitySelectionTarget::Frame(spawn.frame_v1_id),
                &request.expected_run_id,
                1
            )
            .unwrap()
            .is_some());
        fs::remove_file(directory.0.join(publication.artifact_relative_filename)).unwrap();
        fs::remove_file(directory.0.join(publication.weights.relative_filename)).unwrap();
        let selected = running
            .select_entity(
                super::super::state::EntitySelectionTarget::Frame(spawn.frame_v1_id),
                &request.expected_run_id,
                1,
            )
            .unwrap()
            .unwrap();
        assert!(matches!(
            selected.retained_origin,
            Some(
                super::super::retained_origin::RetainedNeuralOrigin::SavedBrain {
                    source_kind: super::super::brain_artifact::BrainSourceKind::Manual,
                    ..
                }
            )
        ));
        let progress = running
            .service_after_command_drain(
                17,
                super::super::scheduler::SchedulerServiceMode::Background,
                None,
            )
            .unwrap();
        assert!(matches!(
            progress,
            super::super::running_loop::RunningAuthorityLoopProgress::Published { .. }
        ));
        assert_eq!(running.completed_step(), before + 1);
    }

    #[test]
    fn saved_deployment_rejects_stale_scope_cadence_and_corrupt_shared_weights() {
        use super::super::sensor_contract::SensorProfile;
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
        let mut running = admitted_loop(0.04, SensorProfile::Basic);
        let mut request = SavedBrainSpawnRequest {
            managed_directory: directory.0.to_string_lossy().into_owned(),
            brain_id: digest(),
            expected_run_id: "another-run".into(),
            expected_config_revision: 1,
            expected_world_epoch: running.world_epoch(),
            weights: Box::new(publication.weights.clone()),
        };
        assert!(running
            .spawn_saved_brain(&request)
            .err()
            .unwrap()
            .contains("run/configuration"));
        request.expected_run_id = "saved-brain-live-test".into();
        assert!(running
            .spawn_saved_brain(&request)
            .err()
            .unwrap()
            .contains("cadence"));
        request.expected_world_epoch = running.world_epoch() + 1;
        assert!(running
            .spawn_saved_brain(&request)
            .err()
            .unwrap()
            .contains("run/configuration"));
        let mut matching = admitted_loop(0.06, SensorProfile::Basic);
        request.expected_world_epoch = matching.world_epoch();
        let original = request.weights.clone();
        request.weights.weight_count_hex = "0000000040000000".into();
        request.weights.decoded_byte_count_hex = "0000000100000000".into();
        assert!(matching
            .spawn_saved_brain(&request)
            .err()
            .unwrap()
            .contains("shared weight descriptor"));
        request.weights = original;
        fs::write(
            directory.0.join(&publication.weights.relative_filename),
            b"corrupt",
        )
        .unwrap();
        assert!(matching.spawn_saved_brain(&request).is_err());
        assert_eq!(matching.completed_step(), 0);
    }

    /// Build a test-only portable envelope from the actual native compiler/manifest.
    fn portable_source(directory: &TestDirectory, renamed: bool) -> PathBuf {
        use super::super::graph::{GraphBundle, GraphNodeKind};
        use super::super::sensor_contract::SensorProfile;
        use super::super::sensor_layout::SensorLayout;
        let fixture = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
        let mut spec = fixture.graph.spec().clone();
        for node in &mut spec.nodes {
            match &mut node.kind {
                GraphNodeKind::Input { output_size } => *output_size = 144,
                GraphNodeKind::Gru { input_size, .. } => *input_size = 144,
                _ => {}
            }
        }
        if renamed {
            let old = spec.nodes[0].id.clone();
            spec.nodes[0].id = "different-node-identity".into();
            for edge in &mut spec.edges {
                if edge.from == old {
                    edge.from = spec.nodes[0].id.clone();
                }
                if edge.to == old {
                    edge.to = spec.nodes[0].id.clone();
                }
            }
            for output in &mut spec.outputs {
                if output.node_id == old {
                    output.node_id = spec.nodes[0].id.clone();
                }
            }
        }
        let graph = GraphBundle::compile(spec, &super::super::fresh_run::stage6a_p0_graph_limits())
            .unwrap();
        let sensors = SensorLayout::v4(SensorProfile::Basic, 20).unwrap();
        let manifest =
            CanonicalTrainingGraphManifestV1::build(&graph, &sensors, fixture.deployment).unwrap();
        let weights = vec![0u8; manifest.total_parameters * 4];
        let digest = Sha256::digest(&weights);
        let json = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "graphManifest": serde_json::from_slice::<serde_json::Value>(&manifest.canonical_bytes().unwrap()).unwrap(), "provenance": fixture.provenance
        }))
        .unwrap();
        let mut bytes = vec![0; 88];
        bytes[..8].copy_from_slice(b"SLTHBRN1");
        bytes[8..12].copy_from_slice(&(json.len() as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&((weights.len() / 4) as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(weights.len() as u32).to_le_bytes());
        bytes[24..56].copy_from_slice(&digest);
        bytes[56..88].copy_from_slice(&digest);
        bytes.extend(json);
        bytes.extend(weights);
        validate_brain_artifact(&bytes).unwrap();
        let path = directory.0.join("source.slither-brain");
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn saved_deployment_rejects_equal_width_structural_sensor_and_graph_mismatches() {
        use super::super::sensor_contract::SensorProfile;
        for renamed in [false, true] {
            let directory = TestDirectory::new();
            let path = portable_source(&directory, renamed);
            let artifact = validate_brain_artifact(&fs::read(&path).unwrap()).unwrap();
            let publication = publish_saved_brain_file(
                &path,
                &directory.0,
                &operation(),
                &artifact.artifact_sha256,
                u64::MAX,
                false,
                false,
            )
            .unwrap();
            let mut spec = artifact.graph.spec().clone();
            if renamed {
                // The graph has equal numeric dimensions but a different persistence identity.
                let original = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
                spec = original.graph.spec().clone();
                for node in &mut spec.nodes {
                    match &mut node.kind {
                        super::super::graph::GraphNodeKind::Input { output_size } => {
                            *output_size = 144
                        }
                        super::super::graph::GraphNodeKind::Gru { input_size, .. } => {
                            *input_size = 144
                        }
                        _ => {}
                    }
                }
            }
            let mut running = admitted_graph_loop(0.06, SensorProfile::Motion, 8, spec);
            let request = SavedBrainSpawnRequest {
                managed_directory: directory.0.to_string_lossy().into_owned(),
                brain_id: artifact.artifact_sha256,
                expected_run_id: "saved-brain-live-test".into(),
                expected_config_revision: 1,
                expected_world_epoch: running.world_epoch(),
                weights: Box::new(publication.weights),
            };
            let error = running.spawn_saved_brain(&request).err().unwrap();
            assert!(
                error.contains(if renamed {
                    "graph"
                } else {
                    "structural sensors"
                }),
                "{error}"
            );
            assert_eq!(running.completed_step(), 0);
        }
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
