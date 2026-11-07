use super::*;

/// Decoding alongside an old population uses a reduced budget. After that
/// owner is retired, the imported loop must afford ordinary dual-state evolution.
#[test]
fn imported_loop_reclaims_retired_population_budget_for_generation_handoffs() {
    for branch in [false, true] {
        let managed = TestDirectory::create("import-retired-budget");
        let full_ceiling = 4 * 1024 * MIB;
        let mut input = request(42);
        input.memory_ceiling_bytes = full_ceiling;
        let prepared = prepare_stage6a_p0_boundary(
            input,
            &[FreshRunSettingUpdate {
                path: "snakeCount".into(),
                value: 3.0,
            }],
            typescript_default_graph_spec(),
        )
        .unwrap();
        let mut staging_policy = prepared.admission_policy.clone();
        let probe = AuthoritativeState::validate_and_own(
            prepared.candidate,
            prepared.graph,
            &staging_policy,
        )
        .unwrap();
        staging_policy.memory_ceiling_bytes = probe.memory_estimate().total_bytes + 8 * MIB;
        let operation = CheckpointOperationId::parse("00000000000000000000000000000081").unwrap();
        let descriptor = crate::engine::checkpoint::publish_checkpoint(
            managed.path(),
            operation,
            1,
            probe.checkpoint_boundary().unwrap(),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &staging_policy,
        )
        .unwrap();
        let restored = crate::engine::checkpoint::restore_checkpoint(
            &managed.path().join(&descriptor.relative_filename),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &staging_policy,
        )
        .unwrap();
        drop(probe); // Old population is now retired; its reservation can be reclaimed.
        let mut pending = prepare_stage6a_p0_validated_import(
            restored,
            descriptor.clone(),
            full_ceiling,
            prepared.checkpoint_limits,
            prepared.graph_limits,
        )
        .unwrap();
        pending.acknowledge_import_persistence(&descriptor).unwrap();
        let run_id = if branch {
            let name = "standalone-import-recovery".to_owned();
            pending = pending
                .into_committed_recovery_branch(name.clone())
                .unwrap();
            name
        } else {
            descriptor.run_id.clone()
        };
        assert_eq!(
            pending.admitted_memory_ceiling_bytes(),
            staging_policy.memory_ceiling_bytes,
            "identity rebinding must retain overlap admission until running ownership transfers"
        );
        pending.publish_running_authority().unwrap();
        let mut running = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        running
            .request_finish_generation("finish-import".into(), run_id, 1)
            .unwrap();
        running
            .service_after_command_drain(17, SchedulerServiceMode::Background, None)
            .expect("retired source budget must support the imported generation transition");
        let transition = running.pending_generation_transition().unwrap();
        assert_eq!(transition.memory_ceiling_bytes(), full_ceiling);
        assert!(transition.combined_state_bytes() > staging_policy.memory_ceiling_bytes);
        assert!(transition.combined_state_bytes() <= full_ceiling);
    }
}

/// The real replacement path keeps staging bounded through identity rebinding
/// and coordinator construction, then reclaims budget after dropping the old loop.
#[test]
fn live_import_replacement_reclaims_budget_with_and_without_recovery_branch() {
    for branch in [false, true] {
        let managed = TestDirectory::create("live-import-retirement");
        let full_ceiling = 4 * 1024 * MIB;
        let mut input = request(42);
        input.memory_ceiling_bytes = full_ceiling;
        let prepared = prepare_stage6a_p0_boundary(
            input.clone(),
            &[FreshRunSettingUpdate {
                path: "snakeCount".into(),
                value: 3.0,
            }],
            typescript_default_graph_spec(),
        )
        .unwrap();
        let mut policy = prepared.admission_policy.clone();
        let probe =
            AuthoritativeState::validate_and_own(prepared.candidate, prepared.graph, &policy)
                .unwrap();
        policy.memory_ceiling_bytes = probe.memory_estimate().total_bytes + 8 * MIB;
        let descriptor = crate::engine::checkpoint::publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("00000000000000000000000000000082").unwrap(),
            1,
            probe.checkpoint_boundary().unwrap(),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &policy,
        )
        .unwrap();
        drop(probe);
        let restored = crate::engine::checkpoint::restore_checkpoint(
            &managed.path().join(&descriptor.relative_filename),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &policy,
        )
        .unwrap();
        let pending = prepare_stage6a_p0_validated_import(
            restored,
            descriptor.clone(),
            full_ceiling,
            prepared.checkpoint_limits,
            prepared.graph_limits,
        )
        .unwrap();
        assert_eq!(
            pending.admitted_memory_ceiling_bytes(),
            policy.memory_ceiling_bytes
        );
        let slot = crate::engine::contract::PreparedImportSlot::new();
        slot.put(pending).unwrap();
        input.run_id = "old-live-import-owner".into();
        input.seed = 43;
        let mut old = prepare_stage6a_p0_fresh_run_with_settings_and_graph(
            input,
            &[FreshRunSettingUpdate {
                path: "snakeCount".into(),
                value: 3.0,
            }],
            typescript_default_graph_spec(),
        )
        .unwrap();
        let old_descriptor = old
            .publish_checkpoint(
                managed.path(),
                CheckpointOperationId::parse("00000000000000000000000000000083").unwrap(),
            )
            .unwrap();
        old.acknowledge_persistence(&old_descriptor).unwrap();
        old.publish_running_authority().unwrap();
        let mut running = old
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        let branch_id = branch.then_some("live-import-recovery-branch");
        running.stage_prepared_import(&slot).unwrap();
        running
            .publish_prepared_import(&slot, &descriptor, branch_id, 0)
            .unwrap();
        assert!(!slot.is_some());
        running
            .request_finish_generation(
                "finish-live-import".into(),
                branch_id.unwrap_or(&descriptor.run_id).to_owned(),
                1,
            )
            .unwrap();
        running
            .service_after_command_drain(17, SchedulerServiceMode::Background, None)
            .unwrap();
        let transition = running.pending_generation_transition().unwrap();
        assert_eq!(transition.memory_ceiling_bytes(), full_ceiling);
        assert!(transition.combined_state_bytes() > policy.memory_ceiling_bytes);
        assert!(transition.combined_state_bytes() <= full_ceiling);
    }
}

/// No candidate is required to hold the source, but only that hold's lease
/// can release it; active worker ownership cannot be cancelled out from under it.
#[test]
fn population_work_holds_complete_steps_before_allocation_and_resumes_without_debt() {
    let managed = TestDirectory::create("population-work-hold");
    let mut input = request(42);
    input.memory_ceiling_bytes = 4 * 1024 * MIB;
    let mut pending = prepare_stage6a_p0_fresh_run(input).unwrap();
    let descriptor = pending
        .publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("00000000000000000000000000000091").unwrap(),
        )
        .unwrap();
    pending.acknowledge_persistence(&descriptor).unwrap();
    pending.publish_running_authority().unwrap();
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .unwrap();
    let epoch = running.world_epoch();
    let slot = crate::engine::contract::PreparedImportSlot::new();
    let foreign = crate::engine::contract::PreparedImportSlot::new();
    running.stage_prepared_import(&slot).unwrap();
    let lease = slot.acquire_work(4 * 1024 * MIB).unwrap();
    assert_eq!(lease.admission.source_world_epoch, epoch);
    assert_eq!(
        lease.admission.remaining_bytes + running.authoritative_memory_bytes(),
        4 * 1024 * MIB
    );
    assert!(matches!(
        running
            .service_after_command_drain(1000, SchedulerServiceMode::Background, None)
            .unwrap(),
        RunningAuthorityLoopProgress::ImportPending
    ));
    assert_eq!(running.completed_step(), 0);
    assert!(running.cancel_prepared_import(&foreign, 1000).is_err());
    assert!(running.cancel_prepared_import(&slot, 1000).is_err());
    assert_eq!(running.state(), RunningAuthorityLoopState::ImportPending);
    drop(lease);
    running.cancel_prepared_import(&slot, 1000).unwrap();
    assert!(matches!(
        running
            .service_after_command_drain(1000, SchedulerServiceMode::Background, None)
            .unwrap(),
        RunningAuthorityLoopProgress::Idle { .. }
    ));
    running
        .service_after_command_drain(1017, SchedulerServiceMode::Background, None)
        .unwrap();
    assert_eq!(running.completed_step(), 1);
    assert_eq!(running.world_epoch(), epoch);
}

#[test]
fn retained_loop_services_one_backlogged_step_per_boundary_and_faults_once() {
    let pending = activate_pending_run_start(
        prepare_stage6a_p0_fresh_run(request(0x1020_3040))
            .expect("complete fresh run must enter durability"),
        "retained-loop",
        "102030405060708090a0b0c0d0e0f000",
    );
    let invalid_handoff = pending
        .into_running_loop(
            FixedStepSchedulerPolicy {
                algorithm_version: u32::MAX,
                ..FixedStepSchedulerPolicy::provisional_defaults()
            },
            0,
        )
        .expect_err("invalid scheduler policy must preserve the prior authority owner");
    assert!(matches!(
        invalid_handoff.error(),
        RunStartTransitionError::RunningLoop(error)
            if matches!(error.as_ref(), RunningAuthorityLoopError::Scheduler(_))
    ));
    let pending = invalid_handoff.into_transition();
    assert!(pending.authority_published());
    assert_eq!(pending.completed_step(), 0);
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect("activated step-zero authority must enter one retained loop");
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    let wall_seconds_until_step = match running
        .service_after_command_drain(0, SchedulerServiceMode::Background, None)
        .expect("origin service must be idle")
    {
        RunningAuthorityLoopProgress::Idle {
            simulation_seconds_until_step,
            wall_seconds_until_step,
        } => {
            assert!(simulation_seconds_until_step > 0.0);
            wall_seconds_until_step
        }
        other => panic!("unexpected origin progress: {other:?}"),
    };
    let one_step_wall_ms = (wall_seconds_until_step * 1_000.0).ceil() as u64;
    assert!(one_step_wall_ms > 0);
    let backlog_wall_ms = one_step_wall_ms * 8;
    let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
    let frame_allocation = frame.as_ptr();

    let first_due = match running
        .service_after_command_drain(
            backlog_wall_ms,
            SchedulerServiceMode::Background,
            Some(RunningFramePublication::new(
                FrameV1ViewDescriptor::default(),
                &mut frame,
            )),
        )
        .expect("one backlogged step must publish")
    {
        RunningAuthorityLoopProgress::Published {
            ticket_sequence,
            due_steps,
            publication,
            frame: Some(metadata),
        } => {
            assert_eq!(ticket_sequence, 1);
            assert_eq!(publication.completed_step, 1);
            assert_eq!(metadata.byte_length, frame.len());
            due_steps
        }
        other => panic!("unexpected first publication: {other:?}"),
    };
    assert!(first_due > 1, "the test must create real retained backlog");
    assert_eq!(frame.as_ptr(), frame_allocation);
    let first_frame = frame.clone();

    for expected_step in 2..=3 {
        let frame_request = (expected_step == 3)
            .then(|| RunningFramePublication::new(FrameV1ViewDescriptor::default(), &mut frame));
        match running
            .service_after_command_drain(
                backlog_wall_ms,
                SchedulerServiceMode::Background,
                frame_request,
            )
            .expect("each new service boundary may publish only one retained-debt step")
        {
            RunningAuthorityLoopProgress::Published {
                ticket_sequence,
                publication,
                frame: metadata,
                ..
            } => {
                assert_eq!(ticket_sequence, expected_step);
                assert_eq!(publication.completed_step, expected_step);
                assert_eq!(metadata.is_some(), expected_step == 3);
            }
            other => panic!("unexpected retained-debt publication: {other:?}"),
        }
        assert_eq!(running.completed_step(), expected_step);
    }
    assert_eq!(frame.as_ptr(), frame_allocation);
    assert_ne!(frame, first_frame);
    let diagnostics = running.scheduler_diagnostics();
    assert_eq!(diagnostics.completed_steps, 3);
    assert_eq!(diagnostics.command_service_boundaries, 4);

    let error = running
        .service_after_command_drain(backlog_wall_ms - 1, SchedulerServiceMode::Background, None)
        .expect_err("regressing the Rust clock must terminally fault the loop");
    assert!(matches!(error, RunningAuthorityLoopError::Scheduler(_)));
    assert_eq!(running.state(), RunningAuthorityLoopState::Faulted);
    assert_eq!(running.completed_step(), 3);
    assert!(matches!(
        running.service_after_command_drain(
            backlog_wall_ms + one_step_wall_ms,
            SchedulerServiceMode::Background,
            None,
        ),
        Err(RunningAuthorityLoopError::AlreadyFaulted)
    ));
    assert_eq!(running.completed_step(), 3);
    assert_eq!(
        running.scheduler_diagnostics().command_service_boundaries,
        4
    );
}

#[test]
fn retained_loop_post_publication_frame_failure_faults_without_retry() {
    let pending = activate_pending_run_start(
        prepare_stage6a_p0_fresh_run(request(0x2030_4050))
            .expect("complete fresh run must enter durability"),
        "retained-frame-fault",
        "2030405060708090a0b0c0d0e0f00010",
    );
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect("activated step-zero authority must enter one retained loop");
    let wall_seconds_until_step = match running
        .service_after_command_drain(0, SchedulerServiceMode::Background, None)
        .expect("origin service must be idle")
    {
        RunningAuthorityLoopProgress::Idle {
            wall_seconds_until_step,
            ..
        } => wall_seconds_until_step,
        other => panic!("unexpected origin progress: {other:?}"),
    };
    let due_wall_ms = (wall_seconds_until_step * 1_000.0).ceil() as u64;
    let mut frame = vec![0x6b; 19];
    let frame_before = frame.clone();
    let error = running
        .service_after_command_drain(
            due_wall_ms,
            SchedulerServiceMode::Background,
            Some(RunningFramePublication::new(
                FrameV1ViewDescriptor {
                    camera_x: f64::NAN,
                    ..FrameV1ViewDescriptor::default()
                },
                &mut frame,
            )),
        )
        .expect_err("invalid presentation metadata must fail after step publication");
    assert!(matches!(error, RunningAuthorityLoopError::Frame(_)));
    assert_eq!(frame, frame_before);
    assert_eq!(running.completed_step(), 1);
    assert_eq!(running.scheduler_diagnostics().completed_steps, 1);
    assert_eq!(running.state(), RunningAuthorityLoopState::Faulted);
    assert!(matches!(
        running.service_after_command_drain(
            due_wall_ms * 2,
            SchedulerServiceMode::Background,
            None,
        ),
        Err(RunningAuthorityLoopError::AlreadyFaulted)
    ));
    assert_eq!(running.completed_step(), 1);
    assert_eq!(running.scheduler_diagnostics().completed_steps, 1);
}

#[test]
fn retained_loop_holds_one_terminal_candidate_without_duplicate_evolution() {
    let pending = activate_pending_run_start(
        short_generation_pending(0x5060_7080),
        "retained-generation",
        "5060708090a0b0c0d0e0f00010203040",
    );
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect("activated short generation must enter one retained loop");
    assert!(matches!(
        running
            .service_after_command_drain(0, SchedulerServiceMode::Background, None)
            .expect("origin service must be idle"),
        RunningAuthorityLoopProgress::Idle { .. }
    ));

    let terminal = (1_u64..=8)
        .find_map(|wall_second| {
            let progress = running
                .service_after_command_drain(
                    wall_second * 1_000,
                    SchedulerServiceMode::Background,
                    None,
                )
                .expect("short generation service must either publish or block");
            match progress {
                RunningAuthorityLoopProgress::Published { publication, .. } => {
                    assert_eq!(publication.completed_step, wall_second);
                    None
                }
                RunningAuthorityLoopProgress::GenerationTransitionPending { .. } => Some(progress),
                other => panic!("unexpected short-generation progress: {other:?}"),
            }
        })
        .expect("valid eight-second generation must reach one terminal boundary");
    let (ticket_sequence, source_key, reason, successor_generation, successor_completed_step) =
        match terminal {
            RunningAuthorityLoopProgress::GenerationTransitionPending {
                ticket_sequence,
                source_key,
                reason,
                successor_generation,
                successor_completed_step,
            } => (
                ticket_sequence,
                source_key,
                reason,
                successor_generation,
                successor_completed_step,
            ),
            _ => unreachable!("find_map returns only terminal progress"),
        };
    assert_eq!(ticket_sequence, 8);
    assert_eq!(source_key.source_completed_step(), 7);
    assert_eq!(successor_generation, 2);
    assert_eq!(successor_completed_step, 8);
    assert_eq!(running.completed_step(), 7);
    assert_eq!(
        running.state(),
        RunningAuthorityLoopState::GenerationTransitionPending
    );

    let (candidate_address, commit_address) = {
        let retained = running
            .pending_generation_transition()
            .expect("terminal transition must remain borrowable");
        assert_eq!(retained.source_key(), source_key);
        assert_eq!(retained.reason(), reason);
        (
            std::ptr::from_ref(retained.candidate()).addr(),
            std::ptr::from_ref(retained.commit_record()).addr(),
        )
    };
    let diagnostics_before_retry = running.scheduler_diagnostics();
    assert!(diagnostics_before_retry.step_pending);
    let mut untouched_frame = vec![0x5a; 17];
    let repeated = running
        .service_after_command_drain(
            9_000,
            SchedulerServiceMode::Background,
            Some(RunningFramePublication::new(
                FrameV1ViewDescriptor::default(),
                &mut untouched_frame,
            )),
        )
        .expect("blocked service must reborrow the same terminal transition");
    assert_eq!(repeated, terminal);
    assert_eq!(untouched_frame, vec![0x5a; 17]);
    assert_eq!(running.completed_step(), 7);
    assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);
    let retained = running
        .pending_generation_transition()
        .expect("repeated service must retain one terminal candidate");
    assert_eq!(
        std::ptr::from_ref(retained.candidate()).addr(),
        candidate_address
    );
    assert_eq!(
        std::ptr::from_ref(retained.commit_record()).addr(),
        commit_address
    );

    let source_world_epoch = running.world_epoch();
    assert!(matches!(
        running.publish_acknowledged_generation_start(10_000, None),
        Err(RunningAuthorityLoopError::RunningStep(error))
            if matches!(*error, RunningStepError::GenerationPersistenceNotAcknowledged)
    ));
    assert_eq!(running.world_epoch(), source_world_epoch);
    assert_eq!(running.completed_step(), 7);
    assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);

    let managed = TestDirectory::create("retained-generation-checkpoint");
    let operation = CheckpointOperationId::parse("60708090a0b0c0d0e0f0001020304050")
        .expect("test operation ID must be canonical");
    let publication = running
        .publish_pending_generation_checkpoint(managed.path(), operation.clone())
        .expect("retained transition must publish one immutable checkpoint");
    let retry = running
        .publish_pending_generation_checkpoint(managed.path(), operation)
        .expect("same operation must return the exact prior publication");
    assert_eq!(retry, publication);
    assert_eq!(running.world_epoch(), source_world_epoch);
    assert_eq!(running.completed_step(), 7);
    assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);
    assert_eq!(
        std::ptr::from_ref(
            running
                .pending_generation_transition()
                .expect("checkpoint retry must retain the same transition")
                .candidate()
        )
        .addr(),
        candidate_address
    );
    let stale_before_assignment = running
        .submit_external_delivery_results(
            &[ExternalDeliveryResult {
                step_key: source_key,
                event_sequence: 1,
                connection_id: 1,
                lease_id: 1,
                accepted: true,
            }],
            None,
        )
        .expect("stale result before reassignment staging must be ignored");
    assert_eq!(stale_before_assignment.ignored_results, 1);
    assert_eq!(
        stale_before_assignment.state,
        RunningAuthorityDeliveryState::Idle
    );
    assert_eq!(running.world_epoch(), source_world_epoch);
    assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);

    let mut mismatched = publication.descriptor.clone();
    mismatched.logical_root_sha256 = "0".repeat(64);
    mismatched.relative_filename = format!("{}.checkpoint-v3", mismatched.logical_root_sha256);
    assert!(matches!(
        running.acknowledge_pending_generation_persistence(&mismatched),
        Err(RunningAuthorityLoopError::RunningStep(error))
            if matches!(
                *error,
                RunningStepError::GenerationPersistenceAcknowledgementMismatch { .. }
            )
    ));
    assert_eq!(running.world_epoch(), source_world_epoch);
    assert!(!running
        .pending_generation_transition()
        .expect("mismatched acknowledgement must retain the transition")
        .persistence_acknowledged());

    running
        .acknowledge_pending_generation_persistence(&publication.descriptor)
        .expect("exact committed descriptor must be retained");
    assert!(running
        .pending_generation_transition()
        .expect("acknowledged transition must remain pending")
        .persistence_acknowledged());
    assert_eq!(running.world_epoch(), source_world_epoch);
    assert!(matches!(
        running.publish_acknowledged_generation_start(10_000, None),
        Err(RunningAuthorityLoopError::RunningStep(error))
            if matches!(
                *error,
                RunningStepError::PendingDeliveryStateMismatch {
                    field: "generation delivery context"
                }
            )
    ));
    assert_eq!(running.world_epoch(), source_world_epoch);

    assert!(matches!(
        running
            .prepare_acknowledged_generation_reassignments()
            .expect("controller-free successor must prepare deterministically"),
        GenerationReassignmentProgress::Ready(_)
    ));
    let generation_start = running
        .publish_acknowledged_generation_start(10_000, None)
        .expect("durable assignment-ready successor must publish exactly once");
    assert_eq!(generation_start.ticket_sequence, ticket_sequence);
    assert_eq!(generation_start.publication.source_key, source_key);
    assert_eq!(generation_start.publication.generation, 2);
    assert_eq!(generation_start.publication.completed_step, 8);
    assert_eq!(running.generation(), 2);
    assert_eq!(running.completed_step(), 8);
    assert_ne!(running.world_epoch(), source_world_epoch);
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    let rebound_diagnostics = running.scheduler_diagnostics();
    assert_eq!(rebound_diagnostics.completed_steps, 8);
    assert!(!rebound_diagnostics.step_pending);
    assert!(matches!(
        running.publish_acknowledged_generation_start(10_000, None),
        Err(RunningAuthorityLoopError::InvalidActionState { .. })
    ));
    assert_eq!(running.generation(), 2);
    assert_eq!(running.completed_step(), 8);

    assert!(matches!(
        running
            .service_after_command_drain(10_000, SchedulerServiceMode::Background, None,)
            .expect("resume origin must exclude persistence wait"),
        RunningAuthorityLoopProgress::Idle { .. }
    ));
    let next = running
        .service_after_command_drain(11_000, SchedulerServiceMode::Background, None)
        .expect("rebound coordinator and scheduler must publish the next generation step");
    assert!(matches!(
        next,
        RunningAuthorityLoopProgress::Published { publication, .. }
            if publication.completed_step == 9
    ));
    assert_eq!(running.generation(), 2);
    assert_eq!(running.completed_step(), 9);
    assert_eq!(running.scheduler_diagnostics().completed_steps, 9);
}

#[test]
fn background_runtime_owns_steps_waits_and_restores_one_loop() {
    let running = activate_pending_run_start(
        prepare_stage6a_p0_fresh_run(request(0x6070_8090))
            .expect("complete fresh run must enter durability"),
        "background-running",
        "60708090a0b0c0d0e0f0001020304050",
    )
    .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
    .expect("durable step-zero authority must enter one unserviced loop");
    let expected_memory = running.authoritative_memory_bytes();

    let mut invalid_init = running_runtime_init();
    invalid_init.inbound.max_batches = 0;
    let failed =
        EngineRuntime::new_running_authority(invalid_init, running, Arc::new(NoopWakeSink))
            .expect_err("invalid queue limits must return the exact loop");
    assert_eq!(
        failed.error().code,
        crate::engine::error::EngineErrorCode::InvalidConfiguration
    );
    let running = failed.into_running_loop();
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    assert_eq!(running.completed_step(), 0);

    let runtime = EngineRuntime::new_running_authority(
        running_runtime_init(),
        running,
        Arc::new(NoopWakeSink),
    )
    .expect("valid runtime must retain the exact loop before thread start");
    assert!(runtime.owns_authoritative_state());
    assert_eq!(
        runtime.authoritative_state_memory_bytes(),
        Some(expected_memory)
    );
    assert!(runtime.running_authority_retained_for_test());
    runtime.start().expect("one background thread must start");

    wait_for_runtime(&runtime, || {
        runtime
            .health()
            .running_authority
            .is_some_and(|health| health.wait_calls > 0)
    });
    runtime
        .try_submit(runtime_probe(1, 7))
        .expect("coarse probe must wake the real authority thread");
    wait_for_runtime(&runtime, || {
        let health = runtime.health();
        health.processed_commands == 1
            && health
                .running_authority
                .is_some_and(|running| running.completed_step >= 3)
    });

    runtime.request_stop();
    runtime
        .join()
        .expect("orderly real-authority stop must join");
    let health = runtime.health();
    assert_eq!(health.lifecycle, LifecycleState::Stopped);
    assert!(health.fault.is_none());
    assert_eq!(health.processed_batches, 1);
    assert_eq!(health.processed_commands, 1);
    let running_health = health
        .running_authority
        .expect("real runtime must retain bounded authority health");
    assert_eq!(running_health.loop_state, RunningAuthorityLoopState::Ready);
    assert!(running_health.completed_step >= 3);
    assert_eq!(
        running_health.scheduler_completed_steps,
        running_health.completed_step
    );
    assert!(running_health.command_service_boundaries > running_health.completed_step);
    assert!(running_health.wait_calls > 0);
    assert!(running_health.timeout_wakes > 0);
    assert!(runtime.running_authority_retained_for_test());

    let events = runtime
        .drain_outputs(usize::MAX, usize::MAX)
        .expect("bounded output drain must succeed")
        .events;
    assert!(events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Started))));
    assert!(events.iter().any(|event| matches!(
        event,
        CompletedEvent::Reliable(ReliableEvent::ProbeResult {
            sequence: 1,
            correlation_id: 101,
            payload,
        }) if payload == &[7]
    )));
    assert!(events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
}

#[test]
fn background_runtime_catches_panic_and_restores_faulted_owner_once() {
    let running = activate_pending_run_start(
        prepare_stage6a_p0_fresh_run(request(0x7080_90a0))
            .expect("complete fresh run must enter durability"),
        "background-panic",
        "708090a0b0c0d0e0f000102030405060",
    )
    .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
    .expect("durable step-zero authority must enter one unserviced loop");
    let runtime = EngineRuntime::new_running_authority(
        running_runtime_init(),
        running,
        Arc::new(NoopWakeSink),
    )
    .expect("valid runtime must retain one loop");
    runtime.start().expect("one background thread must start");
    wait_for_runtime(&runtime, || {
        runtime
            .health()
            .running_authority
            .is_some_and(|health| health.wait_calls > 0)
    });
    runtime
        .try_submit(CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::PanicForTest,
            }]
            .into_boxed_slice(),
        })
        .expect("test panic command must enter the coarse queue");
    wait_for_runtime(&runtime, || {
        runtime.health().lifecycle == LifecycleState::Faulted
    });
    let completed_before_join = runtime
        .health()
        .running_authority
        .expect("fault retains bounded running health")
        .completed_step;
    runtime
        .join()
        .expect("caught background panic must still join cleanly");
    let health = runtime.health();
    assert_eq!(health.lifecycle, LifecycleState::Stopped);
    assert!(health.fault.is_some());
    assert_eq!(
        health
            .running_authority
            .expect("joined fault retains bounded running health")
            .completed_step,
        completed_before_join
    );
    assert!(runtime.running_authority_retained_for_test());
    let events = runtime
        .drain_outputs(usize::MAX, usize::MAX)
        .expect("fault output drain must succeed")
        .events;
    assert!(events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Fault(_))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
}
