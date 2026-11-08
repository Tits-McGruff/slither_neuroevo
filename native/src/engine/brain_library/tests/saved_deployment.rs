use super::*;

#[test]
fn inspector_runtime_caches_due_rows_holds_their_age_and_stops_on_close() {
    use crate::engine::inspector::{InspectorObservationSource, MAX_INSPECTOR_BYTES};
    use crate::engine::scheduler::SchedulerServiceMode;
    use crate::engine::sensor_contract::SensorProfile;
    use crate::engine::state::EntitySelectionTarget;
    for (profile, bins) in [
        (SensorProfile::Basic, 8),
        (SensorProfile::Basic, 20),
        (SensorProfile::Motion, 8),
        (SensorProfile::Motion, 32),
    ] {
        let width = crate::engine::sensor_contract::SensorContractIdentity::new(profile, bins)
            .unwrap()
            .input_size();
        let artifact = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
        let mut graph = artifact.graph.spec().clone();
        for node in &mut graph.nodes {
            match &mut node.kind {
                crate::engine::graph::GraphNodeKind::Input { output_size } => *output_size = width,
                crate::engine::graph::GraphNodeKind::Gru { input_size, .. } => *input_size = width,
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
                false,
            )
            .unwrap();
        assert!(inspected.inspector_observation().is_none());
        for running in [&mut inspected, &mut reference] {
            running
                .service_after_command_drain(17, SchedulerServiceMode::Background, None)
                .unwrap();
        }
        let display =
            crate::engine::display::RunningDisplayCache::new(inspected.admitted_frame_bytes())
                .unwrap();
        display.publish_inspector(&inspected, 0).unwrap();
        let first_mailbox = display.latest_inspector(0).unwrap().unwrap();
        assert_eq!(first_mailbox.subscription_sequence, 17);
        let wire: serde_json::Value =
            serde_json::from_str(&crate::engine::inspector_wire::encode(&first_mailbox).unwrap())
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
            serde_json::from_str(&crate::engine::inspector_wire::encode(&exact).unwrap()).unwrap();
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
            (life.ledger.life_local_wallet + life.ledger.prior_bank_remaining - sample.raw.points)
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
            serde_json::from_str(&crate::engine::inspector_wire::encode(&aged).unwrap()).unwrap();
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
        assert!(crate::engine::inspector_wire::encode(&invalid).is_err());
        inspected
            .configure_inspector("saved-brain-live-test", None, 0)
            .unwrap();
        display.publish_inspector(&inspected, 101).unwrap();
        let cleared = display.latest_inspector(aged.sequence).unwrap().unwrap();
        assert!(cleared.observation.is_none());
        assert_eq!(cleared.subscription_sequence, 17);
        let clear_wire: serde_json::Value =
            serde_json::from_str(&crate::engine::inspector_wire::encode(&cleared).unwrap())
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
fn inspector_pure_probe_is_one_shot_on_a_held_control_and_preserves_continuation() {
    use crate::engine::inspector::MAX_INSPECTOR_BYTES;
    use crate::engine::scheduler::SchedulerServiceMode;
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
    use crate::engine::state::EntitySelectionTarget;
    for (profile, bins) in [
        (SensorProfile::Basic, 8),
        (SensorProfile::Basic, 20),
        (SensorProfile::Motion, 8),
        (SensorProfile::Motion, 32),
    ] {
        let width = SensorContractIdentity::new(profile, bins)
            .unwrap()
            .input_size();
        let artifact = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
        let mut graph = artifact.graph.spec().clone();
        for node in &mut graph.nodes {
            match &mut node.kind {
                crate::engine::graph::GraphNodeKind::Input { output_size } => *output_size = width,
                crate::engine::graph::GraphNodeKind::Gru { input_size, .. } => *input_size = width,
                _ => {}
            }
        }
        let mut probed = admitted_graph_loop(0.06, profile, bins, graph.clone());
        let mut reference = admitted_graph_loop(0.06, profile, bins, graph);
        for running in [&mut probed, &mut reference] {
            running
                .service_after_command_drain(17, SchedulerServiceMode::Background, None)
                .unwrap();
        }
        let selected = probed
            .select_entity(
                EntitySelectionTarget::PopulationSlot(0),
                "saved-brain-live-test",
                1,
            )
            .unwrap()
            .unwrap();
        probed
            .configure_inspector_subscription(
                "saved-brain-live-test",
                Some((selected.token, selected.frame_v1_id)),
                MAX_INSPECTOR_BYTES,
                22,
                true,
            )
            .unwrap();
        let mut first = None;
        for step in 2..=8 {
            for running in [&mut probed, &mut reference] {
                running
                    .service_after_command_drain(step * 17, SchedulerServiceMode::Background, None)
                    .unwrap();
            }
            let sample = *probed.inspector_observation().expect("one explicit probe");
            assert!(sample.pure_probe);
            assert_eq!(sample.sample_step(), Some(2));
            assert_eq!(sample.age_steps(probed.completed_step()), Some(step - 2));
            assert!(!sample.raw.first_delivery);
            assert!(
                sample.neural_outputs.is_none()
                    && sample.baseline_action.is_none()
                    && sample.baseline_v1_action.is_none()
            );
            assert!(sample.shadow_outputs.iter().all(Option::is_none));
            assert_eq!(sample.normalized_inputs().len(), width);
            assert!(sample.diagnostics.ray_work.charged() <= 256);
            assert!(sample.lifecycle.is_some());
            if let Some(first) = first {
                assert_eq!(sample.trace, Some(first));
            } else {
                first = sample.trace;
            }
            let mut actual = Vec::with_capacity(probed.admitted_frame_bytes());
            let mut expected = Vec::with_capacity(reference.admitted_frame_bytes());
            probed.pack_display_into(1, &mut actual).unwrap();
            reference.pack_display_into(1, &mut expected).unwrap();
            assert_eq!(
                actual, expected,
                "probe leaves physical continuation unchanged"
            );
            #[cfg(feature = "engine-test-hooks")]
            assert_eq!(
                probed.qualification_continuation_sha256(),
                reference.qualification_continuation_sha256(),
                "probe leaves RNG, delivery, recurrence and quota continuation unchanged"
            );
        }
        probed
            .configure_inspector_subscription(
                "saved-brain-live-test",
                Some((selected.token, selected.frame_v1_id)),
                MAX_INSPECTOR_BYTES,
                23,
                true,
            )
            .unwrap();
        probed
            .service_after_command_drain(153, SchedulerServiceMode::Background, None)
            .unwrap();
        assert_eq!(
            probed.inspector_observation().unwrap().sample_step(),
            Some(9)
        );
        probed
            .configure_inspector_subscription(
                "saved-brain-live-test",
                Some((selected.token, selected.frame_v1_id)),
                MAX_INSPECTOR_BYTES,
                24,
                false,
            )
            .unwrap();
        for wall in [170, 187, 204, 221, 238] {
            probed
                .service_after_command_drain(wall, SchedulerServiceMode::Background, None)
                .unwrap();
        }
        let delivered = probed.inspector_observation().unwrap();
        assert!(!delivered.pure_probe && delivered.neural_outputs.is_some());
    }
}

#[test]
fn saved_deployment_is_native_and_continues_after_its_catalog_files_are_deleted() {
    use crate::engine::sensor_contract::SensorProfile;
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
            crate::engine::state::EntitySelectionTarget::Frame(spawn.frame_v1_id),
            &request.expected_run_id,
            1
        )
        .unwrap()
        .is_some());
    fs::remove_file(directory.0.join(publication.artifact_relative_filename)).unwrap();
    fs::remove_file(directory.0.join(publication.weights.relative_filename)).unwrap();
    let selected = running
        .select_entity(
            crate::engine::state::EntitySelectionTarget::Frame(spawn.frame_v1_id),
            &request.expected_run_id,
            1,
        )
        .unwrap()
        .unwrap();
    assert!(matches!(
        selected.retained_origin,
        Some(
            crate::engine::retained_origin::RetainedNeuralOrigin::SavedBrain {
                source_kind: crate::engine::brain_artifact::BrainSourceKind::Manual,
                ..
            }
        )
    ));
    let progress = running
        .service_after_command_drain(
            17,
            crate::engine::scheduler::SchedulerServiceMode::Background,
            None,
        )
        .unwrap();
    assert!(matches!(
        progress,
        crate::engine::running_loop::RunningAuthorityLoopProgress::Published { .. }
    ));
    assert_eq!(running.completed_step(), before + 1);
}

#[test]
fn saved_deployment_rejects_stale_scope_cadence_and_corrupt_shared_weights() {
    use crate::engine::sensor_contract::SensorProfile;
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

#[test]
fn saved_deployment_rejects_equal_width_structural_sensor_and_graph_mismatches() {
    use crate::engine::sensor_contract::SensorProfile;
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
                    crate::engine::graph::GraphNodeKind::Input { output_size } => {
                        *output_size = 144
                    }
                    crate::engine::graph::GraphNodeKind::Gru { input_size, .. } => {
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
