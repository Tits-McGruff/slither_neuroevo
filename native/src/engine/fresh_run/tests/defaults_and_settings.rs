use super::*;

#[test]
fn production_fresh_math_selection_is_explicit_and_rejects_retained_mismatch() {
    for backend in [
        InferenceMathBackend::Scalar,
        InferenceMathBackend::Sse2,
        InferenceMathBackend::I16MatrixFma128,
        InferenceMathBackend::I16AffineFma128,
    ] {
        let result = prepare_default_fresh_run_with_math(request(42), backend);
        if !backend.is_available() {
            assert!(result.unwrap_err().to_string().contains("unavailable"));
            continue;
        }
        let fresh = result.unwrap();
        fresh.require_math_backend(backend).unwrap();
        let other = if backend == InferenceMathBackend::Scalar {
            InferenceMathBackend::Sse2
        } else {
            InferenceMathBackend::Scalar
        };
        assert!(fresh
            .require_math_backend(other)
            .unwrap_err()
            .contains("explicit inference backend mismatch:"));
        let metadata: serde_json::Value =
            serde_json::from_str(&fresh.startup_metadata_json().unwrap()).unwrap();
        assert_eq!(metadata["mathBackend"], backend.label());
        assert!(!fresh.checkpoint_published());
    }
}

#[test]
fn matrix_i16_backend_checkpoint_preserves_f32_and_exact_worker_replay() {
    if !InferenceMathBackend::I16MatrixFma128.is_available() {
        return;
    }
    for backend in [
        InferenceMathBackend::I16MatrixFma128,
        InferenceMathBackend::I16AffineFma128,
    ] {
        let settings = numeric_fresh_settings(selected_fresh_default_settings().unwrap());
        let mut fresh = prepare_configured_fresh_run_with_math(
            request(42),
            &settings,
            crate::engine::graph::default_stack_graph_spec(264),
            backend,
        )
        .unwrap();
        let original = fresh.startup_metadata_json().unwrap();
        let directory = TestDirectory::create("matrix-i16-checkpoint");
        let descriptor = fresh
            .publish_checkpoint(
                directory.path(),
                CheckpointOperationId::parse("abcdefabcdefabcdefabcdefabcdefab").unwrap(),
            )
            .unwrap();
        let mut restored = prepare_stage6a_p0_checkpoint_restore(
            directory.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
        )
        .unwrap();
        assert_eq!(restored.startup_metadata_json().unwrap(), original);
        fresh.acknowledge_persistence(&descriptor).unwrap();
        fresh.configure_calculation_workers(1).unwrap();
        restored
            .configure_calculation_workers(
                3.min(crate::engine::calculation_workers::available_calculation_workers()),
            )
            .unwrap();
        for transition in [&mut fresh, &mut restored] {
            transition.publish_running_authority().unwrap();
        }
        let mut fresh = fresh
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        let mut restored = restored
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        let mut left = Vec::with_capacity(fresh.admitted_frame_bytes());
        let mut right = Vec::with_capacity(restored.admitted_frame_bytes());
        for now in (17..=170).step_by(17) {
            fresh
                .service_after_command_drain(now, SchedulerServiceMode::Background, None)
                .unwrap();
            restored
                .service_after_command_drain(now, SchedulerServiceMode::Background, None)
                .unwrap();
            assert_eq!(fresh.completed_step(), restored.completed_step());
            fresh.pack_display_into(now, &mut left).unwrap();
            restored.pack_display_into(now, &mut right).unwrap();
            assert_eq!(left, right);
        }
    }
}

#[test]
fn configured_fresh_life_contract_is_complete_and_preserves_frozen_numeric_initialization() {
    let legacy =
        prepare_stage6a_p0_boundary(request(42), &[], typescript_default_graph_spec()).unwrap();
    let modern =
        prepare_fresh_boundary(request(42), &[], typescript_default_graph_spec(), true).unwrap();
    assert_eq!(
        modern.candidate.rng.version,
        RNG_BUNDLE_WITH_EVOLVED_VERSION
    );
    assert_eq!(
        modern.candidate.versions.rng_bundle,
        RNG_BUNDLE_WITH_EVOLVED_VERSION
    );
    assert_eq!(
        modern.candidate.rng.evolved_respawns.len(),
        STAGE6A_P0_POPULATION_COUNT
    );
    assert_eq!(modern.candidate.population, legacy.candidate.population);
    assert_eq!(modern.candidate.rng.world, legacy.candidate.rng.world);
    assert_eq!(
        modern.candidate.rng.evolution,
        legacy.candidate.rng.evolution
    );
    assert_eq!(
        modern.candidate.rng.baselines,
        legacy.candidate.rng.baselines
    );
    let old = numeric_settings(typescript_default_settings(
        STAGE6A_P0_POPULATION_COUNT,
        STAGE6A_P0_BASELINE_COUNT,
    ));
    let configured = prepare_configured_fresh_run_with_settings_and_graph(
        request(42),
        &old,
        typescript_default_graph_spec(),
    )
    .unwrap();
    let metadata: serde_json::Value =
        serde_json::from_str(&configured.startup_metadata_json().unwrap()).unwrap();
    assert!(!metadata["settings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|setting| setting["path"] == "evolvedRespawn.contractVersion"));
    for partial in [
        vec![FreshRunSettingUpdate {
            path: "evolvedRespawn.contractVersion".into(),
            value: 1.0,
        }],
        vec![FreshRunSettingUpdate {
            path: "evolvedRespawn.enabled".into(),
            value: 1.0,
        }],
    ] {
        assert!(prepare_configured_fresh_run_with_settings_and_graph(
            request(42),
            &partial,
            typescript_default_graph_spec()
        )
        .is_err());
    }
}

#[test]
fn baseline_v2_rng_boundary_roundtrips_managed_checkpoint_and_rejects_mixed_contracts() {
    use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Settings};
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    use crate::engine::rng::{baseline_v2_lifecycle_rng, derive_baseline_v2_behaviors};
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
    use crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
    let graph = GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: 144 },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: 144,
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
    let mut prepared = prepare_fresh_boundary_with_sensor(
        request(42),
        &numeric_settings(production_default_settings(12, 2)),
        graph,
        true,
        Some(SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap()),
        RunningStepWorkLimits::provisional_defaults(),
    )
    .unwrap();
    BaselineV2Settings {
        mode: BaselineV2Mode::Stochastic,
        control_interval_seconds: 0.06,
    }
    .record_settings(&mut prepared.candidate.config.settings)
    .unwrap();
    prepared.candidate.config.settings_schema_sha256 =
        normalized_settings_schema_hash(&prepared.candidate.config.settings).unwrap();
    prepared.candidate.identity.config_hash =
        normalized_config_hash(&prepared.candidate.config).unwrap();
    prepared.admission_policy.expected_settings_schema_sha256 =
        prepared.candidate.config.settings_schema_sha256.clone();
    prepared.candidate.rng.version = RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
    prepared.candidate.versions.rng_bundle = RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
    prepared.candidate.rng.baseline_behaviors =
        derive_baseline_v2_behaviors(42, 1, 1, 2, false).unwrap();
    for stream in &mut prepared.candidate.rng.baselines {
        stream.state = baseline_v2_lifecycle_rng(42, 1, 1, stream.slot, false).export_state();
    }
    let expected = prepared.candidate.rng.clone();
    let mut behavior = expected.baseline_behaviors[0].runtime().unwrap();
    let mut lifecycle =
        crate::engine::rng::StatefulRng::from_state(&expected.baselines[0].state).unwrap();
    assert_ne!(
        behavior.next_f64().to_bits(),
        lifecycle.next_f64().to_bits()
    );
    let authority = AuthoritativeState::validate_and_own(
        prepared.candidate.clone(),
        Arc::clone(&prepared.graph),
        &prepared.admission_policy,
    )
    .unwrap();
    prepared.work_limits.ray_work =
        Some(crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap());
    crate::engine::running_step::RunningStepCoordinator::try_new(&authority, prepared.work_limits)
        .unwrap();
    let directory = TestDirectory::create("baseline-v2-rng-boundary");
    let descriptor = crate::engine::checkpoint::publish_checkpoint(
        directory.path(),
        CheckpointOperationId::parse("23232323232323232323232323232323").unwrap(),
        1,
        authority.checkpoint_boundary().unwrap(),
        &prepared.checkpoint_limits,
        &prepared.graph_limits,
        &prepared.admission_policy,
    )
    .unwrap();
    let restored = crate::engine::checkpoint::restore_checkpoint(
        &directory.path().join(&descriptor.relative_filename),
        &prepared.checkpoint_limits,
        &prepared.graph_limits,
        &prepared.admission_policy,
    )
    .unwrap();
    assert_eq!(restored.state.state().rng, expected);
    #[cfg(feature = "engine-test-hooks")]
    {
        let mut continuations = Vec::new();
        let mut captured_chunks = Vec::new();
        for (workers, capture, shadow) in [
            (1, false, false),
            (3, false, false),
            (1, true, false),
            (3, true, false),
            (1, true, true),
            (3, true, true),
        ] {
            let limits = RunningStepWorkLimits {
                calculation_workers: workers,
                ..prepared.work_limits
            };
            let mut pending = PendingRunStartTransition::admit(
                prepared.candidate.clone(),
                Arc::clone(&prepared.graph),
                current_build_policy(
                    TEST_MEMORY_CEILING,
                    prepared.candidate.config.settings_schema_sha256.clone(),
                    true,
                ),
                prepared.checkpoint_limits.clone(),
                prepared.graph_limits.clone(),
                limits,
            )
            .unwrap();
            let run_directory = TestDirectory::create("v2-joined-control");
            let checkpoint = pending
                .publish_checkpoint(
                    run_directory.path(),
                    CheckpointOperationId::parse("34343434343434343434343434343434").unwrap(),
                )
                .unwrap();
            pending.acknowledge_persistence(&checkpoint).unwrap();
            pending.publish_running_authority().unwrap();
            let mut running = pending
                .into_running_loop(
                    crate::engine::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
                    0,
                )
                .unwrap();
            assert!(running.published_teacher_boundary().is_none());
            if capture {
                assert!(running.configure_teacher_capture(&[0], 1).is_err());
                assert!(running.configure_teacher_capture(&[2], 65536).is_err());
                let request = crate::engine::teacher_runtime::TeacherCaptureRequest {
                    capture_epoch: [17; 16],
                    expected_run_id: prepared.candidate.identity.run_id.clone(),
                    expected_config_revision: prepared.candidate.identity.config_revision,
                    baseline_slots: vec![0].into_boxed_slice(),
                    limits: crate::engine::teacher_runtime::TeacherCaptureLimits {
                        pending_bytes: 1024 * 1024,
                        queue_chunks: 4,
                        queue_bytes: 4 * 1024 * 1024,
                    },
                };
                let mut bad = request.clone();
                bad.expected_run_id = "wrong-run".into();
                assert!(running.start_teacher_capture(&bad).is_err());
                let contract = running.start_teacher_capture(&request).unwrap();
                assert_eq!(contract.teacher_mode, 1);
                assert_eq!(contract.control_interval_seconds, 0.06);
                assert_eq!(
                    contract.sensor_sha256,
                    SensorContractIdentity::new(SensorProfile::Motion, 8)
                        .unwrap()
                        .sha256()
                        .unwrap()
                );
                assert!(running.start_teacher_capture(&request).is_err());
                assert!(running.configure_teacher_capture(&[], 0).is_err());
                if shadow {
                    let candidate = crate::engine::shadows::ShadowCandidate::synthetic(
                        prepared.graph.compiled(),
                        contract.sensor_sha256,
                        0.06,
                    );
                    let mut activation = crate::engine::shadows::ShadowActivationRequest {
                        expected_run_id: prepared.candidate.identity.run_id.clone(),
                        expected_config_revision: prepared.candidate.identity.config_revision,
                        capture_epoch: [17; 16],
                        shadow_slot: 0,
                        baseline_slot: 0,
                        candidate,
                    };
                    let mut stale = activation.clone();
                    stale.capture_epoch = [18; 16];
                    assert!(running.activate_shadow(&stale, 1).is_err());
                    for slot in 0..4 {
                        activation.shadow_slot = slot;
                        running
                            .activate_shadow(&activation, u64::from(slot) + 1)
                            .unwrap();
                    }
                    activation.shadow_slot = 4;
                    assert!(running.activate_shadow(&activation, 5).is_err());
                }
            }
            let mut captured_rows = 0;
            for step in 1..=16 {
                let outcome = running
                    .service_after_command_drain(
                        (step * 1000u64).div_ceil(60),
                        crate::engine::scheduler::SchedulerServiceMode::Background,
                        None,
                    )
                    .unwrap();
                assert!(matches!(
                    outcome,
                    crate::engine::running_loop::RunningAuthorityLoopProgress::Published { .. }
                ));
                assert_eq!(running.completed_step(), step);
                if shadow && step == 1 {
                    // Resolve the spawned baseline through real frame IDs and native full identities.
                    let physical = running.published_teacher_boundary().unwrap().decisions[0]
                        .physical_snake_id;
                    let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
                    running.pack_display_into(1, &mut frame).unwrap();
                    let read = |offset: usize| {
                        f32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap())
                    };
                    let mut offset = 7 * 4;
                    let mut selected = None;
                    for _ in 0..read(2 * 4) as usize {
                        let frame_id = read(offset) as u32;
                        if let Some(entity) = running
                            .select_entity(
                                crate::engine::state::EntitySelectionTarget::Frame(frame_id),
                                &prepared.candidate.identity.run_id,
                                1,
                            )
                            .unwrap()
                        {
                            if entity.token.physical_id() == physical {
                                selected = Some(entity);
                                break;
                            }
                        }
                        offset += (8 + 2 * read(offset + 7 * 4) as usize) * 4;
                    }
                    let selected = selected.expect("spawned teacher in native frame");
                    running
                        .configure_inspector(
                            &prepared.candidate.identity.run_id,
                            Some((selected.token, selected.frame_v1_id)),
                            crate::engine::inspector::MAX_INSPECTOR_BYTES,
                        )
                        .unwrap();
                }
                if let Some(rows) = running.published_teacher_boundary() {
                    assert!(capture);
                    assert_eq!(rows.key.source_completed_step() + 1, step);
                    for (index, decision) in rows.decisions.iter().enumerate() {
                        assert_eq!(decision.baseline_slot, 0);
                        assert_eq!(rows.observation(index).unwrap().len(), 144);
                        assert_eq!(
                            rows.outcome(index).unwrap().physical_snake_id,
                            decision.physical_snake_id
                        );
                        assert!(rows.outcome(index + rows.decisions.len()).is_none());
                        captured_rows += 1;
                    }
                }
            }
            if shadow {
                for snapshot in running.shadow_snapshots().iter().flatten() {
                    assert!(snapshot.enabled, "{:?}", snapshot.error);
                    assert_eq!(snapshot.totals.samples, captured_rows as u64);
                    assert_eq!(snapshot.completed_step, 16);
                }
                let observed = running
                    .inspector_observation()
                    .expect("selected actual teacher row");
                assert_eq!(
                    observed.source,
                    crate::engine::inspector::InspectorObservationSource::BaselineV2
                );
                assert!(observed.lifecycle.is_none());
                for (index, snapshot) in running.shadow_snapshots().iter().enumerate() {
                    let snapshot = snapshot.unwrap();
                    let output =
                        observed.shadow_outputs[index].expect("exact joined shadow output");
                    assert_eq!(output.sample_step, snapshot.last_sample_step);
                    assert_eq!(Some(output.sample_step), observed.sample_step());
                    assert_eq!(output.raw_outputs, snapshot.raw_outputs);
                    assert_eq!(output.capture_epoch, [17; 16]);
                    assert_eq!(output.activation_sequence, index as u64 + 1);
                    assert_eq!(snapshot.physical_snake_id, observed.target.physical_id);
                }
                let before = running.qualification_continuation_sha256();
                assert!(running.stop_shadow(0, [17; 16], 99).is_err());
                assert!(running.shadow_snapshots()[0].unwrap().enabled);
                running.stop_shadow(0, [17; 16], 1).unwrap();
                assert!(!running.shadow_snapshots()[0].unwrap().enabled);
                assert!(running.shadow_snapshots()[1].unwrap().enabled);
                assert_eq!(before, running.qualification_continuation_sha256());
            }
            if capture {
                assert!(captured_rows > 0);
                running.stop_teacher_capture().unwrap();
                assert!(running
                    .shadow_snapshots()
                    .iter()
                    .flatten()
                    .all(|snapshot| !snapshot.enabled));
                let status = running.teacher_capture_status().unwrap();
                assert!(status.2 && status.3.is_none());
                let drained = running.drain_teacher_chunks(4, 65536).unwrap().unwrap();
                assert_eq!(drained.chunks.len(), 1);
                let decoded = crate::engine::teacher_samples::TeacherChunkV1::decode(
                    &drained.chunks[0].bytes,
                )
                .unwrap();
                assert_eq!(decoded.samples.len(), captured_rows);
                captured_chunks.push(drained.chunks[0].bytes.clone());
            }
            continuations.push(running.qualification_continuation_sha256());
        }
        assert!(continuations.iter().all(|value| value == &continuations[0]));
        assert_eq!(captured_chunks.len(), 4);
        assert!(captured_chunks
            .iter()
            .all(|bytes| bytes == &captured_chunks[0]));

        // Exercise the actual background owner with no teacher consumer and
        // no controller/generation acknowledgements during normal stepping.
        let mut candidate = prepared.candidate.clone();
        candidate.config.requested_sim_speed = 12.0;
        // This tiny graph/population needs bounded scratch, while the existing
        // 2 GiB test ceiling must still charge both generation authorities.
        candidate.config.worker_scratch_bytes = 64 * 1024 * 1024;
        candidate.config.checkpoint_scratch_bytes = 64 * 1024 * 1024;
        candidate
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == "simSpeed")
            .unwrap()
            .value = NormalizedSettingValue::Float(12.0);
        candidate.identity.config_hash = normalized_config_hash(&candidate.config).unwrap();
        let request = crate::engine::teacher_runtime::TeacherCaptureRequest {
            capture_epoch: [19; 16],
            expected_run_id: candidate.identity.run_id.clone(),
            expected_config_revision: candidate.identity.config_revision,
            baseline_slots: vec![0].into_boxed_slice(),
            limits: crate::engine::teacher_runtime::TeacherCaptureLimits {
                pending_bytes: 1024 * 1024,
                queue_chunks: 1,
                queue_bytes: 4 * 1024 * 1024,
            },
        };
        let mut pending = PendingRunStartTransition::admit(
            candidate,
            Arc::clone(&prepared.graph),
            current_build_policy(
                TEST_MEMORY_CEILING,
                prepared.candidate.config.settings_schema_sha256.clone(),
                true,
            ),
            prepared.checkpoint_limits.clone(),
            prepared.graph_limits.clone(),
            prepared.work_limits,
        )
        .unwrap();
        let runtime_directory = TestDirectory::create("v2-background-capture");
        let checkpoint = pending
            .publish_checkpoint(
                runtime_directory.path(),
                CheckpointOperationId::parse("35353535353535353535353535353535").unwrap(),
            )
            .unwrap();
        pending.acknowledge_persistence(&checkpoint).unwrap();
        pending.publish_running_authority().unwrap();
        let running = pending
            .into_running_loop(
                crate::engine::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
                0,
            )
            .unwrap();
        let runtime = crate::engine::runtime::EngineRuntime::new_running_authority(
            crate::engine::generation_handoff_fixture::background_generation_handoff_runtime_init(),
            running,
            Arc::new(crate::engine::queues::NoopWakeSink),
        )
        .unwrap();
        runtime.start().unwrap();
        runtime
            .try_submit(crate::engine::contract::CommandBatch {
                contract_version: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                commands: vec![crate::engine::contract::SequencedCommand {
                    sequence: 1,
                    command: crate::engine::contract::EngineCommand::RunningAuthority(
                        crate::engine::contract::RunningAuthorityCommand::StartTeacherCapture(
                            Box::new(request),
                        ),
                    ),
                }]
                .into_boxed_slice(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while runtime
            .health()
            .running_authority
            .as_ref()
            .unwrap()
            .completed_step
            < 240
        {
            assert!(
                std::time::Instant::now() < deadline,
                "background capture never reached target: {:?}",
                runtime.health()
            );
            assert!(runtime.health().fault.is_none());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let activation = crate::engine::shadows::ShadowActivationRequest {
            expected_run_id: prepared.candidate.identity.run_id.clone(),
            expected_config_revision: prepared.candidate.identity.config_revision,
            capture_epoch: [19; 16],
            shadow_slot: 0,
            baseline_slot: 0,
            candidate: crate::engine::shadows::ShadowCandidate::synthetic(
                prepared.graph.compiled(),
                SensorContractIdentity::new(SensorProfile::Motion, 8)
                    .unwrap()
                    .sha256()
                    .unwrap(),
                0.06,
            ),
        };
        runtime
            .try_submit(crate::engine::contract::CommandBatch {
                contract_version: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                commands: vec![crate::engine::contract::SequencedCommand {
                    sequence: 2,
                    command: crate::engine::contract::EngineCommand::RunningAuthority(
                        crate::engine::contract::RunningAuthorityCommand::ActivateShadow(Box::new(
                            activation,
                        )),
                    ),
                }]
                .into_boxed_slice(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while runtime.shadow_snapshots().unwrap()[0]
            .is_none_or(|snapshot| snapshot.totals.samples == 0)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "shadow command/mailbox made no progress: {:?}",
                runtime.health()
            );
            assert!(runtime.health().fault.is_none());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Retain a real generation barrier without persistence acknowledgement;
        // diagnostic wake/drain must still make progress and cannot publish it.
        runtime
            .try_submit(crate::engine::contract::CommandBatch {
                contract_version: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                commands: vec![crate::engine::contract::SequencedCommand {
                    sequence: 3,
                    command: crate::engine::contract::EngineCommand::RunningAuthority(
                        crate::engine::contract::RunningAuthorityCommand::FinishGeneration {
                            request_id: "capture-retained-barrier".into(),
                            expected_run_id: prepared.candidate.identity.run_id.clone(),
                            expected_generation: 1,
                        },
                    ),
                }]
                .into_boxed_slice(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while runtime
            .health()
            .running_authority
            .as_ref()
            .unwrap()
            .loop_state
            != crate::engine::running_loop::RunningAuthorityLoopState::GenerationTransitionPending
        {
            assert!(
                std::time::Instant::now() < deadline,
                "manual barrier not reached: {:?}",
                runtime.health()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let completed_step = runtime
            .health()
            .running_authority
            .as_ref()
            .unwrap()
            .completed_step;
        let teacher = runtime.drain_teacher_chunks(1, 65536).unwrap();
        assert!(teacher.error.is_none(), "{:?}", teacher.error);
        assert_eq!(teacher.contract.as_ref().unwrap().capture_epoch, [19; 16]);
        assert_eq!(teacher.chunks.len(), 1);
        assert!(teacher.snapshot.production_watermark >= 3);
        assert!(teacher.snapshot.dropped_chunks > 0);
        assert!(teacher.more_work);
        let sensor = SensorContractIdentity::decode_supported(
            teacher.sensor_contract_json.as_ref().unwrap().as_bytes(),
        )
        .unwrap();
        assert_eq!(
            sensor.sha256().unwrap(),
            teacher.contract.as_ref().unwrap().sensor_sha256
        );
        let decoded =
            crate::engine::teacher_samples::TeacherChunkV1::decode(&teacher.chunks[0].bytes)
                .unwrap();
        assert_eq!(
            decoded.contract.teacher_sha256,
            teacher.contract.as_ref().unwrap().teacher_sha256
        );
        assert_eq!(runtime.health().processed_commands, 3);
        let mut blocked_chunks = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while blocked_chunks == 0 {
            let drained = runtime.drain_teacher_chunks(16, 65536).unwrap();
            blocked_chunks += drained.chunks.len();
            assert!(
                std::time::Instant::now() < deadline,
                "diagnostic wake failed at retained generation barrier"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            runtime
                .health()
                .running_authority
                .as_ref()
                .unwrap()
                .completed_step,
            completed_step
        );
        assert_eq!(
            runtime
                .health()
                .running_authority
                .as_ref()
                .unwrap()
                .generation,
            1
        );
        runtime.request_stop();
        runtime.join().unwrap();
        let mut count = teacher.chunks.len() + blocked_chunks;
        for _ in 0..8 {
            let drained = runtime.drain_teacher_chunks(16, 65536).unwrap();
            count += drained.chunks.len();
            if !drained.more_work {
                break;
            }
        }
        assert!(count >= 2);
        assert!(runtime.health().fault.is_none());
    }

    for change in 0..4 {
        let mut bad = prepared.candidate.clone();
        match change {
            0 => {
                bad.rng.version = 2;
                bad.versions.rng_bundle = 2;
                bad.rng.baseline_behaviors.clear();
            }
            1 => {
                bad.rng.baseline_behaviors.pop();
            }
            2 => bad.rng.baseline_behaviors[0].uniform_state = 0,
            _ => bad.rng.baseline_behaviors[1].slot = 0,
        };
        assert!(AuthoritativeState::validate_and_own(
            bad,
            Arc::clone(&prepared.graph),
            &prepared.admission_policy
        )
        .is_err());
    }
}

#[test]
fn v4_state_admission_rejects_missing_digest_and_graph_mismatch_without_reinterpreting_numeric_environment(
) {
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
    let contract = SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap();
    let graph = GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: 144 },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: 144,
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
    let prepared = prepare_fresh_boundary_with_sensor(
        request(42),
        &[],
        graph,
        false,
        Some(contract.clone()),
        RunningStepWorkLimits::provisional_defaults(),
    )
    .unwrap();
    for path in [
        "sense.contractVersion",
        "sense.profile",
        "sense.contractSha256",
    ] {
        let mut missing = prepared.candidate.clone();
        missing.config.settings.retain(|entry| entry.path != path);
        missing.config.settings_schema_sha256 =
            normalized_settings_schema_hash(&missing.config.settings).unwrap();
        missing.identity.config_hash = normalized_config_hash(&missing.config).unwrap();
        let mut policy = current_build_policy(
            TEST_MEMORY_CEILING,
            missing.config.settings_schema_sha256.clone(),
            true,
        );
        policy.expected_settings_schema_sha256 = missing.config.settings_schema_sha256.clone();
        assert!(matches!(
            AuthoritativeState::validate_and_own(missing, Arc::clone(&prepared.graph), &policy),
            Err(StateError::InvalidField {
                field: "config.sensor_contract",
                ..
            })
        ));
    }
    let mut modified = prepared.candidate.clone();
    modified
        .config
        .settings
        .iter_mut()
        .find(|entry| entry.path == "sense.profile")
        .unwrap()
        .value = NormalizedSettingValue::Integer(0);
    modified.identity.config_hash = normalized_config_hash(&modified.config).unwrap();
    assert!(matches!(
        AuthoritativeState::validate_and_own(
            modified,
            Arc::clone(&prepared.graph),
            &prepared.admission_policy
        ),
        Err(StateError::InvalidField {
            field: "config.sensor_contract",
            ..
        })
    ));
    let mut changed_environment = prepared.candidate.clone();
    changed_environment
        .config
        .settings
        .iter_mut()
        .find(|entry| entry.path == "sense.rNearBase")
        .unwrap()
        .value = NormalizedSettingValue::Float(600.0);
    changed_environment.identity.config_hash =
        normalized_config_hash(&changed_environment.config).unwrap();
    let authority = AuthoritativeState::validate_and_own(
        changed_environment,
        Arc::clone(&prepared.graph),
        &prepared.admission_policy,
    )
    .unwrap();
    assert_eq!(
        SensorContractIdentity::from_settings(&authority.state().config.settings).unwrap(),
        contract
    );
    assert!(
        matches!(crate::engine::running_step::RunningStepCoordinator::try_new(&authority,RunningStepWorkLimits::provisional_defaults()),
        Err(crate::engine::running_step::RunningStepError::Config(error)) if matches!(*error,crate::engine::step_config::StepConfigError::MissingRayWorkLimits))
    );
    assert!(matches!(
        prepare_fresh_boundary_with_sensor(
            request(42),
            &[],
            typescript_default_graph_spec(),
            false,
            Some(contract),
            RunningStepWorkLimits::provisional_defaults()
        ),
        Err(FreshRunError::GraphInputMismatch { .. })
    ));
}

#[test]
fn startup_metadata_preserves_the_unpublished_run_boundary() {
    let transition = prepare_stage6a_p0_fresh_run(request(42)).unwrap();
    let encoded = transition.startup_metadata_json().unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(metadata["seed"], 42);
    assert_eq!(metadata["serializerVersion"], 1);
    assert_eq!(metadata["sensorVersion"], 3);
    assert_eq!(metadata["parameterCount"], STAGE6A_P0_PARAMETERS_PER_GENOME);
    assert!(metadata["maximumFrameBytes"].as_u64().unwrap() > 0);
    assert!(metadata["population"].is_null());
    assert!(metadata["world"].is_null());
    assert_eq!(transition.startup_metadata_json().unwrap(), encoded);
    assert_eq!(transition.completed_step(), 0);
    assert!(!transition.checkpoint_published());
    assert!(!transition.authority_published());
    assert_eq!(transition.snake_count(), 0);
}

#[test]
fn maximum_ambient_target_preserves_complete_death_effects_and_next_step() {
    let pending = activate_pending_run_start(
        prepare_stage6a_p0_fresh_run_with_settings(
            request(42),
            &[
                FreshRunSettingUpdate {
                    path: "snakeCount".into(),
                    value: 1.0,
                },
                FreshRunSettingUpdate {
                    path: "baselineBots.count".into(),
                    value: 0.0,
                },
                FreshRunSettingUpdate {
                    path: "pelletCountTarget".into(),
                    value: 25_000.0,
                },
            ],
        )
        .unwrap(),
        "maximum-ambient-death",
        "112233445566778899aabbccddeeff00",
    );
    let mut frame = Vec::new();
    let initial = pending.pack_initial_frame_v1(&mut frame).unwrap();
    assert_eq!(initial.pellets, 25_000);
    assert_eq!(initial.alive_snakes, 1);
    let snake_id = f32::from_le_bytes(frame[28..32].try_into().unwrap()) as u32;
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .unwrap();
    let token = running
        .select_entity(
            crate::engine::state::EntitySelectionTarget::Frame(snake_id),
            &request(42).run_id,
            1,
        )
        .unwrap()
        .unwrap()
        .token;
    let (death, effective_step) = running
        .apply_god_mode_kill(token, snake_id)
        .expect("the supported ambient target must leave space for the full normal corpse");
    assert!(death.pellets_dropped > 0);
    assert_eq!(effective_step, 1);
    let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
    let after_death = running.pack_display_into(1, &mut frame).unwrap();
    assert_eq!(after_death.frame.alive_snakes, 0);
    assert_eq!(after_death.frame.pellets, 25_000 + death.pellets_dropped);
    assert!(matches!(
        running.service_after_command_drain(17, SchedulerServiceMode::Background, None).unwrap(),
        RunningAuthorityLoopProgress::Published { publication, .. } if publication.completed_step == 1
    ));
}

#[test]
fn fresh_replacement_applies_reset_only_and_live_settings_before_hashing() {
    let prepared = prepare_stage6a_p0_boundary(
        request(43),
        &[
            FreshRunSettingUpdate {
                path: "worldRadius".to_owned(),
                value: 4_200.0,
            },
            FreshRunSettingUpdate {
                path: "generationSeconds".to_owned(),
                value: 90.0,
            },
            FreshRunSettingUpdate {
                path: "simSpeed".to_owned(),
                value: 3.0,
            },
            FreshRunSettingUpdate {
                path: "foodSpawn.edgeFalloffEnabled".to_owned(),
                value: 0.0,
            },
            FreshRunSettingUpdate {
                path: "snakeCount".to_owned(),
                value: 54.0,
            },
            FreshRunSettingUpdate {
                path: "baselineBots.count".to_owned(),
                value: 2.0,
            },
        ],
        typescript_default_graph_spec(),
    )
    .expect("compatible fixed-graph settings must build one private boundary");
    assert_eq!(prepared.candidate.config.world_radius, 4_200.0);
    assert_eq!(prepared.candidate.config.requested_sim_speed, 3.0);
    assert_eq!(
        fresh_float(&prepared.candidate.config, "generationSeconds").unwrap(),
        90.0
    );
    assert!(matches!(
        prepared
            .candidate
            .config
            .settings
            .iter()
            .find(|setting| setting.path == "foodSpawn.edgeFalloffEnabled")
            .map(|setting| &setting.value),
        Some(NormalizedSettingValue::Bool(false))
    ));
    assert_eq!(
        prepared.candidate.identity.config_hash,
        normalized_config_hash(&prepared.candidate.config).unwrap()
    );
    assert_eq!(prepared.candidate.config.population_count, 54);
    assert_eq!(prepared.candidate.population.len(), 54);
    assert_eq!(prepared.candidate.brains.len(), 54);
    assert_eq!(prepared.candidate.config.baseline_count, 2);
    assert_eq!(prepared.candidate.rng.baselines.len(), 2);
}

#[test]
fn fresh_replacement_compiles_and_initializes_a_supplied_graph() {
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    let graph = GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".to_owned(),
                kind: GraphNodeKind::Input { output_size: 51 },
            },
            GraphNodeSpec {
                id: "head".to_owned(),
                kind: GraphNodeKind::Dense {
                    input_size: 51,
                    output_size: 2,
                },
            },
        ],
        edges: vec![GraphEdge {
            from: "input".to_owned(),
            to: "head".to_owned(),
            from_port: None,
            to_port: None,
        }],
        outputs: vec![GraphOutputRef {
            node_id: "head".to_owned(),
            port: None,
        }],
        output_size: 2,
    };
    let bins = [FreshRunSettingUpdate {
        path: "sense.bubbleBins".to_owned(),
        value: 8.0,
    }];
    let transition =
        prepare_stage6a_p0_fresh_run_with_settings_and_graph(request(45), &bins, graph)
            .expect("a bounded sensor-compatible graph must build a private boundary");
    let metadata: serde_json::Value =
        serde_json::from_str(&transition.startup_metadata_json().unwrap()).unwrap();
    assert_eq!(metadata["parameterCount"], 104);
    assert_eq!(metadata["graphSpec"]["nodes"][1]["type"], "Dense");

    let mismatch = GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".to_owned(),
                kind: GraphNodeKind::Input { output_size: 50 },
            },
            GraphNodeSpec {
                id: "head".to_owned(),
                kind: GraphNodeKind::Dense {
                    input_size: 50,
                    output_size: 2,
                },
            },
        ],
        edges: vec![GraphEdge {
            from: "input".to_owned(),
            to: "head".to_owned(),
            from_port: None,
            to_port: None,
        }],
        outputs: vec![GraphOutputRef {
            node_id: "head".to_owned(),
            port: None,
        }],
        output_size: 2,
    };
    let error =
        match prepare_stage6a_p0_fresh_run_with_settings_and_graph(request(46), &bins, mismatch) {
            Ok(_) => panic!("a graph with the wrong sensor width must reject before publication"),
            Err(error) => error,
        };
    assert!(error.to_string().contains("does not match sensor width"));
}

#[test]
fn production_v4_admits_wide_hidden_layers_within_parameter_limit() {
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};

    let contract = SensorContractIdentity::new(SensorProfile::Motion, 32).unwrap();
    let input_size = contract.input_size();
    assert_eq!(input_size, 504);
    let mut settings = selected_fresh_default_settings().unwrap();
    contract.record_v4_settings(&mut settings).unwrap();
    let mut settings = numeric_fresh_settings(settings);
    settings
        .iter_mut()
        .find(|setting| setting.path == "snakeCount")
        .unwrap()
        .value = 1.0;

    for hidden_size in [768, 1024, 2048, 3200, 4096] {
        let graph = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input {
                        output_size: input_size,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size,
                        hidden_sizes: vec![hidden_size],
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
        let parameters = (input_size + 1) * hidden_size + (hidden_size + 1) * 2;
        let result =
            prepare_configured_fresh_run_with_settings_and_graph(request(45), &settings, graph);
        if hidden_size == 4096 {
            assert!(parameters > 2_000_000);
            let error = match result {
                Ok(_) => panic!("a graph exceeding the parameter limit must reject"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("max_parameter_floats"));
        } else {
            assert!(parameters < 2_000_000);
            let transition =
                result.expect("a bounded wide v4 graph must pass production admission");
            let metadata: serde_json::Value =
                serde_json::from_str(&transition.startup_metadata_json().unwrap()).unwrap();
            assert_eq!(metadata["parameterCount"], parameters);
        }
    }
}
