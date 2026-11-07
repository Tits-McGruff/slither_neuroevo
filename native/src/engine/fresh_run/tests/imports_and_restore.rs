use super::*;

#[test]
fn qualified_fresh_defaults_restore_with_the_recorded_sse2_backend() {
    let mut fresh = prepare_default_fresh_run(request(42)).unwrap();
    let metadata = fresh.startup_metadata_json().unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(decoded["sensorVersion"], 4);
    assert_eq!(decoded["mathBackend"], InferenceMathBackend::Sse2.label());
    let settings = numeric_metadata_settings(&metadata);
    for (path, value) in [
        ("sense.profile", 1.0),
        ("sense.bubbleBins", 16.0),
        ("sense.maxPelletChecks", 3000.0),
        ("sense.maxSegmentChecks", 2200.0),
        ("sense.rayMinimum", 1.0),
        ("sense.rayCap", 256.0),
        ("sense.rayBatch", 65536.0),
        ("sense.rayMaximumDue", 512.0),
        ("baselineBots.controlVersion", 2.0),
        ("baselineBots.mode", 1.0),
        ("baselineBots.controlDt", 1.0 / 60.0),
        ("generationSeconds", 240.0),
        ("evolvedRespawn.enabled", 0.0),
    ] {
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.path == path)
                .unwrap()
                .value,
            value
        );
    }
    let directory = TestDirectory::create("qualified-fresh-defaults");
    let descriptor = fresh
        .publish_checkpoint(
            directory.path(),
            CheckpointOperationId::parse("abcdefabcdefabcdefabcdefabcdefab").unwrap(),
        )
        .unwrap();
    let mut restored =
        prepare_stage6a_p0_checkpoint_restore(directory.path(), &descriptor, TEST_MEMORY_CEILING)
            .unwrap();
    assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
    let recovered = prepare_stage6a_p0_compatible_checkpoint_restore(
        directory.path(),
        &descriptor,
        TEST_MEMORY_CEILING,
    )
    .unwrap()
    .into_committed_recovery_branch("sse2-recovery".into())
    .unwrap();
    let recovered_metadata: serde_json::Value =
        serde_json::from_str(&recovered.startup_metadata_json().unwrap()).unwrap();
    assert_eq!(
        recovered_metadata["mathBackend"],
        InferenceMathBackend::Sse2.label()
    );
    for math in [InferenceMathBackend::Scalar, InferenceMathBackend::Sse2] {
        let reset = prepare_configured_fresh_run_with_math(
            request(42),
            &settings,
            crate::engine::graph::default_stack_graph_spec(264),
            math,
        )
        .unwrap();
        let reset_metadata: serde_json::Value =
            serde_json::from_str(&reset.startup_metadata_json().unwrap()).unwrap();
        assert_eq!(reset_metadata["mathBackend"], math.label());
        assert_eq!(reset_metadata["settings"], decoded["settings"]);
    }
    fresh.acknowledge_persistence(&descriptor).unwrap();
    for transition in [&mut fresh, &mut restored] {
        transition.publish_running_authority().unwrap();
        transition
            .publish_first_scheduled_frame_v1(&mut Vec::new())
            .unwrap();
        assert_eq!(transition.completed_step(), 1);
    }
}

#[test]
fn population_checkpoint_fork_preserves_f32_settings_seed_and_durability() {
    let math = InferenceMathBackend::I16MatrixFma128;
    if !math.is_available() {
        return;
    }
    let mut prepared = prepare_configured_fresh_boundary(
        request(42),
        &[],
        crate::engine::graph::default_stack_graph_spec(264),
        InferenceMathBackend::Scalar,
    )
    .unwrap();
    for (slot, genome) in prepared.candidate.population.iter_mut().enumerate() {
        genome.weights[0] = -5.0;
        genome.weights[1] = 5.0;
        genome.weights[2] = (slot as f32 + 1.0) * 1e-6;
    }
    let expected = prepared
        .candidate
        .population
        .iter()
        .map(|g| g.weights.clone())
        .collect::<Vec<_>>();
    let mut original = PendingRunStartTransition::admit(
        prepared.candidate,
        prepared.graph,
        prepared.admission_policy,
        prepared.checkpoint_limits,
        prepared.graph_limits,
        prepared.work_limits,
    )
    .unwrap();
    let metadata: serde_json::Value =
        serde_json::from_str(&original.startup_metadata_json().unwrap()).unwrap();
    let directory = TestDirectory::create("population-backend-fork");
    let descriptor = original
        .publish_checkpoint(
            directory.path(),
            CheckpointOperationId::parse("abcdefabcdefabcdefabcdefabcdefab").unwrap(),
        )
        .unwrap();
    let original_bytes =
        std::fs::read(directory.path().join(&descriptor.relative_filename)).unwrap();
    let restore = || {
        prepare_stage6a_p0_compatible_checkpoint_restore(
            directory.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
        )
        .unwrap()
    };
    let mut target_request = request(999);
    // The private source world retires before allocating the new boundary,
    // so the ordinary 2 GiB fixture ceiling still admits this transfer.
    target_request.run_id = "population-fork-test".into();
    let mut fork =
        prepare_population_checkpoint_fork(target_request.clone(), restore(), math).unwrap();
    let fork_metadata: serde_json::Value =
        serde_json::from_str(&fork.startup_metadata_json().unwrap()).unwrap();
    for key in [
        "settings",
        "graphSpec",
        "graphKey",
        "parameterCount",
        "seed",
    ] {
        assert_eq!(fork_metadata[key], metadata[key], "fork changed {key}");
    }
    assert_eq!(fork_metadata["mathBackend"], math.label());
    assert_eq!(fork_metadata["runId"], "population-fork-test");
    assert!(fork.publish_running_authority().is_err());
    let fork_descriptor = fork
        .publish_checkpoint(
            directory.path(),
            CheckpointOperationId::parse("12345678123456781234567812345678").unwrap(),
        )
        .unwrap();
    assert_ne!(
        fork_descriptor.logical_root_sha256,
        descriptor.logical_root_sha256
    );
    let inspected = prepare_stage6a_p0_checkpoint_restore(
        directory.path(),
        &fork_descriptor,
        TEST_MEMORY_CEILING,
    )
    .unwrap()
    .into_population_for_new_run()
    .unwrap();
    for (actual, expected) in inspected.state().population.iter().zip(&expected) {
        assert_eq!(&actual.weights, expected);
    }
    assert_eq!(
        std::fs::read(directory.path().join(&descriptor.relative_filename)).unwrap(),
        original_bytes
    );
    fork.acknowledge_persistence(&fork_descriptor).unwrap();
    fork.publish_running_authority().unwrap();
    assert!(original.into_population_for_new_run().is_err()); // fresh is not a validated restore
    target_request.run_id = descriptor.run_id.clone();
    assert!(prepare_population_checkpoint_fork(target_request.clone(), restore(), math).is_err());
    target_request.run_id = "memory-rejected-fork".into();
    target_request.memory_ceiling_bytes = 1;
    assert!(
        prepare_population_checkpoint_fork(target_request, restore(), math)
            .unwrap_err()
            .to_string()
            .contains("memory admission")
    );
}

#[test]
fn production_restore_roundtrips_the_selected_life_contract_and_rejects_unknown_schemas() {
    let managed = TestDirectory::create("life-contract-restore");
    let mut settings = numeric_settings(production_default_settings(3, 0));
    for setting in &mut settings {
        if setting.path == "evolvedRespawn.enabled" {
            setting.value = 1.0;
        }
        if setting.path == "evolvedRespawn.recurrentState" {
            setting.value = 1.0;
        }
    }
    let mut pending = prepare_configured_fresh_run_with_settings_and_graph(
        request(42),
        &settings,
        typescript_default_graph_spec(),
    )
    .unwrap();
    let before: serde_json::Value =
        serde_json::from_str(&pending.startup_metadata_json().unwrap()).unwrap();
    let descriptor = pending
        .publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("89898989898989898989898989898989").unwrap(),
        )
        .unwrap();
    let mut restored =
        prepare_stage6a_p0_checkpoint_restore(managed.path(), &descriptor, TEST_MEMORY_CEILING)
            .unwrap();
    let after: serde_json::Value =
        serde_json::from_str(&restored.startup_metadata_json().unwrap()).unwrap();
    assert_eq!(before["settings"], after["settings"]);
    assert_eq!(before["seed"], after["seed"]);
    restored.publish_running_authority().unwrap();
    assert_eq!(restored.snake_count(), 3);
    restored
        .publish_first_scheduled_frame_v1(&mut Vec::new())
        .unwrap();
    let mut prepared = prepare_fresh_boundary(
        request(42),
        &settings,
        typescript_default_graph_spec(),
        true,
    )
    .unwrap();
    prepared.candidate.config.settings.pop();
    prepared.candidate.config.settings_schema_sha256 =
        normalized_settings_schema_hash(&prepared.candidate.config.settings).unwrap();
    prepared.candidate.identity.config_hash =
        normalized_config_hash(&prepared.candidate.config).unwrap();
    assert!(crate::engine::state::AuthoritativeState::validate_and_own(
        prepared.candidate,
        prepared.graph,
        &production_restore_policy(TEST_MEMORY_CEILING, true).unwrap()
    )
    .is_err());
}

#[test]
fn baseline_v2_cold_constructor_restores_both_profiles_and_modes_with_exact_metadata() {
    use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Settings};
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for mode in [BaselineV2Mode::Deterministic, BaselineV2Mode::Stochastic] {
            let width = SensorContractIdentity::new(profile, 8)
                .unwrap()
                .input_size();
            let graph = GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".into(),
                        kind: GraphNodeKind::Input { output_size: width },
                    },
                    GraphNodeSpec {
                        id: "head".into(),
                        kind: GraphNodeKind::Dense {
                            input_size: width,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![GraphEdge {
                    from: "input".into(),
                    to: "head".into(),
                    from_port: None,
                    to_port: None,
                }],
                outputs: vec![GraphOutputRef {
                    node_id: "head".into(),
                    port: None,
                }],
                output_size: 2,
            };
            let settings = numeric_settings(production_default_settings(12, 2));
            let spec = BaselineV2Settings {
                mode,
                control_interval_seconds: 0.06,
            };
            let limits = RunningStepWorkLimits {
                ray_work: Some(
                    crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap(),
                ),
                calculation_workers: 3,
                ..RunningStepWorkLimits::provisional_defaults()
            };
            assert!(prepare_baseline_v2_qualification_run(
                request(42),
                &settings,
                graph.clone(),
                SensorProfile::V3,
                8,
                spec,
                limits
            )
            .is_err());
            assert!(prepare_baseline_v2_qualification_run(
                request(42),
                &settings,
                graph.clone(),
                profile,
                8,
                spec,
                RunningStepWorkLimits {
                    ray_work: None,
                    ..limits
                }
            )
            .is_err());
            let mut pending = prepare_baseline_v2_qualification_run(
                request(42),
                &settings,
                graph.clone(),
                profile,
                8,
                spec,
                limits,
            )
            .unwrap();
            let metadata = pending.startup_metadata_json().unwrap();
            assert!(metadata.contains("baselineBots.controlVersion"));
            assert!(metadata.contains("baselineBots.controlDt"));
            let explicit_settings = numeric_metadata_settings(&metadata);
            let mut configured = prepare_configured_fresh_run_with_settings_and_graph(
                request(42),
                &explicit_settings,
                graph.clone(),
            )
            .unwrap();
            assert_eq!(configured.startup_metadata_json().unwrap(), metadata);
            for path in [
                "sense.profile",
                "sense.contractVersion",
                "brain.sensorVersion",
                "sense.rayMinimum",
                "sense.rayCap",
                "sense.rayBatch",
                "sense.rayMaximumDue",
                "baselineBots.controlVersion",
                "baselineBots.mode",
                "baselineBots.controlDt",
            ] {
                let partial = explicit_settings
                    .iter()
                    .filter(|setting| setting.path != path)
                    .cloned()
                    .collect::<Vec<_>>();
                assert!(
                    prepare_configured_fresh_run_with_settings_and_graph(
                        request(42),
                        &partial,
                        graph.clone()
                    )
                    .is_err(),
                    "accepted missing {path}"
                );
            }
            for (path, value) in [
                ("sense.profile", 2.0),
                ("brain.sensorVersion", 3.0),
                ("sense.rayMinimum", 0.0),
                ("sense.rayCap", 0.5),
                ("sense.rayMaximumDue", 513.0),
                ("sense.rayMaximumDue", 1.0),
                ("baselineBots.mode", 2.0),
                ("baselineBots.controlDt", 0.0),
            ] {
                let mut invalid = explicit_settings.clone();
                invalid
                    .iter_mut()
                    .find(|setting| setting.path == path)
                    .unwrap()
                    .value = value;
                assert!(
                    prepare_configured_fresh_run_with_settings_and_graph(
                        request(42),
                        &invalid,
                        graph.clone()
                    )
                    .is_err(),
                    "accepted invalid {path}"
                );
            }
            let mut duplicate = explicit_settings.clone();
            duplicate.push(FreshRunSettingUpdate {
                path: "sense.rayMinimum".into(),
                value: 1.0,
            });
            assert!(prepare_configured_fresh_run_with_settings_and_graph(
                request(42),
                &duplicate,
                graph.clone()
            )
            .is_err());
            assert!(prepare_configured_fresh_run_with_settings_and_graph(
                request(42),
                &explicit_settings,
                typescript_default_graph_spec()
            )
            .is_err());
            let directory = TestDirectory::create("v2-cold-restore");
            let descriptor = pending
                .publish_checkpoint(
                    directory.path(),
                    CheckpointOperationId::parse("56565656565656565656565656565656").unwrap(),
                )
                .unwrap();
            let configured_directory = TestDirectory::create("v2-configured-reset");
            let configured_descriptor = configured
                .publish_checkpoint(
                    configured_directory.path(),
                    CheckpointOperationId::parse("67676767676767676767676767676767").unwrap(),
                )
                .unwrap();
            assert_eq!(
                configured_descriptor.logical_root_sha256,
                descriptor.logical_root_sha256
            );
            let restored = PendingRunStartTransition::restore_committed(
                directory.path(),
                &descriptor,
                production_restore_policy(TEST_MEMORY_CEILING, true).unwrap(),
                production_checkpoint_limits(),
                stage6a_p0_graph_limits(),
                limits,
            )
            .unwrap();
            assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
            let mut oversized = production_restore_policy(TEST_MEMORY_CEILING, true).unwrap();
            assert_eq!(oversized.allowed_settings_schema_sha256.len(), 9);
            oversized
                .allowed_settings_schema_sha256
                .push(oversized.expected_settings_schema_sha256.clone());
            assert!(PendingRunStartTransition::restore_committed(
                directory.path(),
                &descriptor,
                oversized,
                production_checkpoint_limits(),
                stage6a_p0_graph_limits(),
                limits
            )
            .is_err());
        }
    }
}

#[test]
fn v4_durable_metadata_restores_recorded_ray_limits_and_runs_equivalent_fixed_steps() {
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    use crate::engine::ray_work::RayWorkLimits;
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in [8, 16, 32] {
            let contract = SensorContractIdentity::new(profile, bins).unwrap();
            let width = contract.input_size();
            let graph = GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".into(),
                        kind: GraphNodeKind::Input { output_size: width },
                    },
                    GraphNodeSpec {
                        id: "memory".into(),
                        kind: GraphNodeKind::Rru {
                            input_size: width,
                            hidden_size: 8,
                        },
                    },
                    GraphNodeSpec {
                        id: "head".into(),
                        kind: GraphNodeKind::Dense {
                            input_size: 8,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![
                    GraphEdge {
                        from: "input".into(),
                        to: "memory".into(),
                        from_port: None,
                        to_port: None,
                    },
                    GraphEdge {
                        from: "memory".into(),
                        to: "head".into(),
                        from_port: None,
                        to_port: None,
                    },
                ],
                outputs: vec![GraphOutputRef {
                    node_id: "head".into(),
                    port: None,
                }],
                output_size: 2,
            };
            let settings = numeric_settings(production_default_settings(12, 2));
            let limits = RunningStepWorkLimits {
                ray_work: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
                calculation_workers: 3,
                ..RunningStepWorkLimits::provisional_defaults()
            };
            let mut pending = prepare_v4_qualification_run(
                request(42),
                &settings,
                graph,
                true,
                profile,
                bins,
                limits,
            )
            .unwrap();
            let metadata = pending.startup_metadata_json().unwrap();
            let decoded: serde_json::Value = serde_json::from_str(&metadata).unwrap();
            assert_eq!(decoded["sensorVersion"], 4);
            assert_eq!(
                decoded["sensorContract"]["sha256"],
                contract.sha256_hex().unwrap()
            );
            assert_eq!(
                decoded["sensorContract"]["canonicalJson"],
                String::from_utf8(contract.canonical_bytes().unwrap()).unwrap()
            );
            let managed = TestDirectory::create("v4-durable-contract");
            let descriptor = pending
                .publish_checkpoint(
                    managed.path(),
                    CheckpointOperationId::parse("112233445566778899aabbccddeeff00").unwrap(),
                )
                .unwrap();
            let policy = production_restore_policy(TEST_MEMORY_CEILING, true).unwrap();
            let mut restored = PendingRunStartTransition::restore_committed(
                managed.path(),
                &descriptor,
                policy,
                production_checkpoint_limits(),
                stage6a_p0_graph_limits(),
                RunningStepWorkLimits {
                    ray_work: None,
                    ..limits
                },
            )
            .unwrap();
            assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
            restored.configure_calculation_workers(1).unwrap();
            let mut conflicting = PendingRunStartTransition::restore_committed(
                managed.path(),
                &descriptor,
                production_restore_policy(TEST_MEMORY_CEILING, true).unwrap(),
                production_checkpoint_limits(),
                stage6a_p0_graph_limits(),
                RunningStepWorkLimits {
                    ray_work: Some(RayWorkLimits::new(1, 128, 65536, 512).unwrap()),
                    ..limits
                },
            )
            .unwrap();
            conflicting.publish_running_authority().unwrap();
            let error = conflicting
                .publish_first_scheduled_frame_v1(&mut Vec::new())
                .unwrap_err();
            assert!(error.to_string().contains("recorded ray limits differ"));
            assert_eq!(conflicting.completed_step(), 0);
            let mut expected_frame = None;
            for transition in [&mut pending, &mut restored] {
                transition.acknowledge_persistence(&descriptor).unwrap();
                transition.publish_running_authority().unwrap();
                let mut frame = Vec::new();
                assert_eq!(
                    transition
                        .pack_initial_frame_v1(&mut frame)
                        .unwrap()
                        .alive_snakes,
                    14
                );
                transition
                    .publish_first_scheduled_frame_v1(&mut frame)
                    .unwrap();
                assert_eq!(transition.completed_step(), 1);
                if let Some(expected) = &expected_frame {
                    assert_eq!(&frame, expected);
                } else {
                    expected_frame = Some(frame);
                }
            }
        }
    }
}

#[cfg(feature = "engine-test-hooks")]
#[test]
fn qualified_checkpoint_restore_preserves_exact_sse2_contract_and_worker_continuation() {
    use crate::engine::checkpoint::CheckpointOperationId;
    use crate::engine::ray_work::RayWorkLimits;
    use crate::engine::sensor_contract::SensorProfile;
    let artifact = crate::engine::brain_artifact::validate_brain_artifact(
        &crate::engine::trained_cohort::fixture_bytes(0.0, 0.06),
    )
    .unwrap();
    let limits = RunningStepWorkLimits {
        ray_work: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
        calculation_workers: 1,
        ..RunningStepWorkLimits::provisional_defaults()
    };
    let mut pending = prepare_sensor_qualification_run(
        request(42),
        &numeric_settings(production_default_settings(12, 2)),
        artifact.graph.spec().clone(),
        SensorProfile::Basic,
        8,
        limits,
        InferenceMathBackend::Sse2,
        Some(crate::engine::baseline_v2::BaselineV2Settings {
            mode: crate::engine::baseline_v2::BaselineV2Mode::Stochastic,
            control_interval_seconds: 0.06,
        }),
        &[],
    )
    .unwrap();
    let metadata = pending.startup_metadata_json().unwrap();
    let managed = TestDirectory::create("qualified-sse2-restore");
    let descriptor = pending
        .publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("112233445566778899aabbccddeeff00").unwrap(),
        )
        .unwrap();
    let original = std::fs::read(managed.path().join(&descriptor.relative_filename)).unwrap();
    // Production supports recorded SSE2; an explicit qualification request remains exact.
    let ordinary =
        prepare_stage6a_p0_checkpoint_restore(managed.path(), &descriptor, TEST_MEMORY_CEILING)
            .unwrap();
    assert_eq!(ordinary.startup_metadata_json().unwrap(), metadata);
    assert!(prepare_sensor_qualification_checkpoint_restore(
        managed.path(),
        &descriptor,
        TEST_MEMORY_CEILING,
        SensorProfile::Basic,
        8,
        InferenceMathBackend::Scalar,
        limits
    )
    .is_err());
    assert!(prepare_sensor_qualification_checkpoint_restore(
        managed.path(),
        &descriptor,
        TEST_MEMORY_CEILING,
        SensorProfile::Motion,
        8,
        InferenceMathBackend::Sse2,
        limits
    )
    .is_err());
    let missing = prepare_sensor_qualification_checkpoint_restore(
        std::path::Path::new("unused-no-ray-admission"),
        &descriptor,
        TEST_MEMORY_CEILING,
        SensorProfile::Basic,
        8,
        InferenceMathBackend::Sse2,
        RunningStepWorkLimits::provisional_defaults(),
    )
    .err()
    .unwrap();
    assert!(missing.to_string().contains("explicit ray limits"));
    let mut restored = prepare_sensor_qualification_checkpoint_restore(
        managed.path(),
        &descriptor,
        TEST_MEMORY_CEILING,
        SensorProfile::Basic,
        8,
        InferenceMathBackend::Sse2,
        limits,
    )
    .unwrap();
    assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
    restored.configure_calculation_workers(3).unwrap();
    let mut expected_frame = None;
    for transition in [&mut pending, &mut restored] {
        transition.acknowledge_persistence(&descriptor).unwrap();
        transition.publish_running_authority().unwrap();
        let mut frame = Vec::new();
        assert_eq!(
            transition
                .pack_initial_frame_v1(&mut frame)
                .unwrap()
                .alive_snakes,
            14
        );
        transition
            .publish_first_scheduled_frame_v1(&mut frame)
            .unwrap();
        assert_eq!(transition.completed_step(), 1);
        if let Some(expected) = &expected_frame {
            assert_eq!(&frame, expected);
        } else {
            expected_frame = Some(frame);
        }
    }
    assert_eq!(
        std::fs::read(managed.path().join(&descriptor.relative_filename)).unwrap(),
        original
    );
}

#[test]
fn default_population_checkpoint_retry_then_exact_ack_activates_once() {
    let managed = TestDirectory::create("managed-checkpoint");
    let operation = CheckpointOperationId::parse("0123456789abcdef0123456789abcdef")
        .expect("fixed operation ID must parse");
    let mut pending = prepare_stage6a_p0_fresh_run(request(FIXTURE_SEED))
        .expect("complete fresh run must enter the durability barrier");
    let mut frame = Vec::new();
    assert!(matches!(
        pending.pack_initial_frame_v1(&mut frame),
        Err(RunStartTransitionError::AuthorityNotPublished)
    ));
    assert!(matches!(
        pending.publish_first_scheduled_frame_v1(&mut frame),
        Err(RunStartTransitionError::AuthorityNotPublished)
    ));
    assert!(!pending.first_scheduled_step_attempted());
    assert!(frame.is_empty());
    assert!(matches!(
        pending.publish_running_authority(),
        Err(RunStartTransitionError::PersistenceNotAcknowledged)
    ));

    let descriptor = pending
        .publish_checkpoint(managed.path(), operation.clone())
        .expect("fixed P0 boundary must fit and publish under its checkpoint ceilings");
    assert_eq!(descriptor.operation_id, operation);
    assert_eq!(descriptor.run_id, request(FIXTURE_SEED).run_id);
    assert_eq!(descriptor.generation_hex, "0000000000000001");
    assert_eq!(descriptor.completed_step_hex, "0000000000000000");
    assert_eq!(descriptor.boundary_kind, CheckpointBoundaryKind::RunStart);
    assert_eq!(
        descriptor.population_count_hex,
        format!("{STAGE6A_P0_POPULATION_COUNT:016x}")
    );
    assert_eq!(
        descriptor.weight_count_hex,
        format!(
            "{:016x}",
            STAGE6A_P0_POPULATION_COUNT * STAGE6A_P0_PARAMETERS_PER_GENOME
        )
    );
    assert_eq!(
        descriptor.recurrent_state_count_hex,
        format!(
            "{:016x}",
            STAGE6A_P0_POPULATION_COUNT * STAGE6A_P0_RECURRENT_PER_BRAIN
        )
    );
    assert!(managed.path().join(&descriptor.relative_filename).is_file());

    let retry = pending
        .publish_checkpoint(managed.path(), operation)
        .expect("exact retry must reuse the retained immutable descriptor");
    assert_eq!(retry, descriptor);
    assert!(pending.checkpoint_published());
    assert!(!pending.persistence_acknowledged());
    assert!(!pending.authority_published());

    pending
        .acknowledge_persistence(&descriptor)
        .expect("exact committed descriptor must retain the durability barrier");
    let publication = pending
        .publish_running_authority()
        .expect("durable fixed P0 boundary must construct one running authority");
    assert_eq!(publication.generation, 1);
    assert_eq!(publication.completed_step, 0);
    assert_eq!(publication.population_epoch, 1);
    assert!(publication.memory.total_bytes <= TEST_MEMORY_CEILING);
    assert!(pending.persistence_acknowledged());
    assert!(pending.authority_published());
    assert_eq!(
        pending.snake_count(),
        STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT
    );
    assert_eq!(pending.pellet_count(), 3_500);
    let frame_metadata = pending
        .pack_initial_frame_v1(&mut frame)
        .expect("running authority must pack one neutral frame-v1 payload");
    assert_eq!(frame_metadata.generation, 1);
    assert_eq!(
        frame_metadata.total_snakes,
        STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT
    );
    assert_eq!(frame_metadata.alive_snakes, frame_metadata.total_snakes);
    assert_eq!(frame_metadata.pellets, 3_500);
    assert_eq!(frame_metadata.byte_length, frame.len());
    assert_eq!(frame_metadata.float_length * size_of::<f32>(), frame.len());
    let initial_frame = frame.clone();
    let scheduled_metadata = pending
        .publish_first_scheduled_frame_v1(&mut frame)
        .expect("Rust must schedule, publish, and pack exactly one complete fixed-P0 step");
    assert_eq!(pending.completed_step(), 1);
    assert!(pending.first_scheduled_step_attempted());
    assert!(pending.first_scheduled_frame_published());
    assert_eq!(scheduled_metadata.generation, 1);
    assert_eq!(
        scheduled_metadata.total_snakes,
        STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT
    );
    assert_eq!(
        scheduled_metadata.alive_snakes,
        scheduled_metadata.total_snakes
    );
    assert_eq!(scheduled_metadata.pellets, 3_496);
    assert_eq!(scheduled_metadata.byte_length, frame.len());
    assert_ne!(
        frame, initial_frame,
        "one complete step must change display state"
    );
    assert!(matches!(
        pending.publish_first_scheduled_frame_v1(&mut frame),
        Err(RunStartTransitionError::FirstScheduledStepAlreadyAttempted)
    ));
    assert_eq!(pending.completed_step(), 1);
    assert!(matches!(
        pending.publish_running_authority(),
        Err(RunStartTransitionError::AuthorityAlreadyPublished)
    ));
    let handoff = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect_err("the one-shot scheduler must exclude a second loop owner");
    assert!(matches!(
        handoff.error(),
        RunStartTransitionError::ExperimentalStepAlreadyOwnsAuthority
    ));
    let pending = handoff.into_transition();
    assert_eq!(pending.completed_step(), 1);
    assert!(pending.first_scheduled_frame_published());
}

#[test]
fn restored_generation_activates_without_resetting_its_chronology() {
    let managed = TestDirectory::create("restored-generation");
    let mut prepared =
        prepare_stage6a_p0_boundary(request(42), &[], typescript_default_graph_spec()).unwrap();
    prepared.candidate.phase =
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation);
    prepared.candidate.generation.generation = 2;
    prepared.candidate.generation.completed_step = 60;
    prepared.candidate.generation.population_epoch = 2;
    for genome in &mut prepared.candidate.population {
        genome.brain.epoch = 2;
    }
    for brain in &mut prepared.candidate.brains {
        brain.handle.epoch = 2;
    }
    let state = AuthoritativeState::validate_and_own(
        prepared.candidate,
        prepared.graph,
        &prepared.admission_policy,
    )
    .unwrap();
    let descriptor = crate::engine::checkpoint::publish_checkpoint(
        managed.path(),
        CheckpointOperationId::parse("41414141414141414141414141414141").unwrap(),
        state.world_epoch(),
        state.checkpoint_boundary().unwrap(),
        &prepared.checkpoint_limits,
        &prepared.graph_limits,
        &prepared.admission_policy,
    )
    .unwrap();
    let mut restored = PendingRunStartTransition::restore_committed(
        managed.path(),
        &descriptor,
        prepared.admission_policy,
        prepared.checkpoint_limits,
        prepared.graph_limits,
        prepared.work_limits,
    )
    .unwrap();
    assert_eq!(restored.generation(), 2);
    assert_eq!(restored.completed_step(), 60);
    assert_eq!(restored.snake_count(), 0);
    assert!(!restored.authority_published());
    assert_ne!(restored.transition_epoch(), state.world_epoch());
    let publication = restored.publish_running_authority().unwrap();
    assert_eq!(publication.generation, 2);
    assert_eq!(publication.completed_step, 60);
    assert_eq!(publication.population_epoch, 2);
    assert!(restored.publish_running_authority().is_err());
    let mut running = restored
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .unwrap();
    let progress = running
        .service_after_command_drain(17, SchedulerServiceMode::Background, None)
        .unwrap();
    assert!(matches!(
        progress,
        RunningAuthorityLoopProgress::Published { .. }
    ));
    assert_eq!(running.completed_step(), 61);
}

#[test]
fn recovery_rebinding_preserves_every_state_field_and_moves_population_storage() {
    let prepared =
        prepare_stage6a_p0_boundary(request(42), &[], typescript_default_graph_spec()).unwrap();
    let mut expected = prepared.candidate.clone();
    let population = prepared.candidate.population.as_ptr();
    let state = AuthoritativeState::validate_and_own(
        prepared.candidate,
        prepared.graph,
        &prepared.admission_policy,
    )
    .unwrap();
    let branch = state
        .into_recovery_branch("different-lineage".into(), &prepared.admission_policy)
        .unwrap();
    expected.identity.run_id = "different-lineage".into();
    assert_eq!(branch.state(), &expected);
    assert_eq!(branch.state().population.as_ptr(), population);
}

#[test]
fn recovery_branch_changes_only_lineage_before_activation() {
    let managed = TestDirectory::create("recovery-lineage");
    let mut source = prepare_stage6a_p0_fresh_run(request(42)).unwrap();
    let descriptor = source
        .publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("42424242424242424242424242424242").unwrap(),
        )
        .unwrap();
    let restore = || {
        prepare_stage6a_p0_compatible_checkpoint_restore(
            managed.path(),
            &descriptor,
            request(42).memory_ceiling_bytes,
        )
        .unwrap()
    };
    let restored = restore();
    let before_rejection = restored.startup_metadata_json().unwrap();
    let mut uncommitted = restore();
    assert!(matches!(
        uncommitted.publish_running_authority(),
        Err(RunStartTransitionError::CompatibleRecoveryBranchNotCommitted)
    ));
    let mut wrong = descriptor.clone();
    wrong.generation_hex = "0000000000000002".into();
    assert!(restored
        .validate_recovery_source(&wrong, "recovery-branch")
        .is_err());
    assert!(restored.validate_recovery_source(&descriptor, "").is_err());
    assert_eq!(restored.startup_metadata_json().unwrap(), before_rejection);
    restored
        .validate_recovery_source(&descriptor, "recovery-branch")
        .unwrap();
    let mut original_metadata: serde_json::Value =
        serde_json::from_str(&restored.startup_metadata_json().unwrap()).unwrap();
    let mut branch = restored
        .into_committed_recovery_branch("recovery-branch".into())
        .unwrap();
    let branch_metadata: serde_json::Value =
        serde_json::from_str(&branch.startup_metadata_json().unwrap()).unwrap();
    original_metadata["runId"] = serde_json::json!("recovery-branch");
    assert_eq!(branch_metadata, original_metadata);
    assert_eq!(branch.generation(), source.generation());
    assert_eq!(branch.completed_step(), source.completed_step());
    assert!(!branch.authority_published());
    assert_eq!(branch.snake_count(), 0);
    branch.publish_running_authority().unwrap();
    assert!(branch
        .validate_recovery_source(&descriptor, "another")
        .is_err());
    assert!(branch
        .into_committed_recovery_branch("another".into())
        .is_err());
    assert!(restore()
        .into_committed_recovery_branch(descriptor.run_id.clone())
        .is_err());
    assert!(restore()
        .into_committed_recovery_branch(String::new())
        .is_err());
}
