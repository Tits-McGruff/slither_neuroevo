use super::*;

#[test]
fn production_large_publication_reload_and_native_deployment() {
    use crate::engine::sensor_contract::SensorProfile;
    for upper in [false, true] {
        let directory = TestDirectory::new();
        let bytes = crate::engine::brain_artifact::tests::large_artifact(upper);
        let admitted = validate_brain_artifact(&bytes).unwrap();
        let input = directory.0.join("input.slither-brain");
        fs::write(&input, &bytes).unwrap();
        // An old raw content-addressed object must remain usable after upgrading.
        let raw: Vec<u8> = admitted
            .weights
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        fs::write(
            directory
                .0
                .join(format!("{}.hof-weights-v1", admitted.weights_sha256)),
            &raw,
        )
        .unwrap();
        let publication = publish_saved_brain_file(
            &input,
            &directory.0,
            &operation(),
            &admitted.artifact_sha256,
            u64::MAX,
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            publication.weights.stored_byte_count_hex,
            format!("{:016x}", raw.len())
        );
        let reloaded = read_validated_hall_of_fame_weights(
            &directory.0.join(&publication.weights.relative_filename),
            &publication.weights,
        )
        .unwrap();
        assert_eq!(&*reloaded, &*admitted.weights);
        let mut running = admitted_graph_loop(
            1.0 / 60.0,
            SensorProfile::Motion,
            16,
            admitted.graph.spec().clone(),
        );
        let request = SavedBrainSpawnRequest {
            managed_directory: directory.0.to_string_lossy().into_owned(),
            brain_id: admitted.artifact_sha256.clone(),
            expected_run_id: "saved-brain-live-test".into(),
            expected_config_revision: 1,
            expected_world_epoch: running.world_epoch(),
            weights: Box::new(publication.weights.clone()),
        };
        let (_, step) = running.spawn_saved_brain(&request).unwrap();
        assert_eq!(step, 1);
        running
            .service_after_command_drain(
                17,
                crate::engine::scheduler::SchedulerServiceMode::Background,
                None,
            )
            .unwrap();
        assert_eq!(running.completed_step(), 1);
        let mut wrong = request.clone();
        wrong.weights.logical_sha256 = "00".repeat(32);
        assert!(running
            .spawn_saved_brain(&wrong)
            .err()
            .unwrap()
            .contains("identity"));
        let mut wrong = request.clone();
        wrong.weights.stored_byte_count_hex =
            format!("{:016x}", crate::contracts::MAX_BRAIN_WEIGHT_BYTES + 1);
        assert!(running
            .spawn_saved_brain(&wrong)
            .err()
            .unwrap()
            .contains("descriptor"));
        let mut cadence = admitted_graph_loop(
            0.06,
            SensorProfile::Motion,
            16,
            admitted.graph.spec().clone(),
        );
        let mut wrong = request.clone();
        wrong.expected_world_epoch = cadence.world_epoch();
        assert!(cadence
            .spawn_saved_brain(&wrong)
            .err()
            .unwrap()
            .contains("cadence"));
    }
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
        crate::engine::checkpoint::NumericEncoding::RawF32LeV1
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
