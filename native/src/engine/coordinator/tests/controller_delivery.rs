use super::*;

#[test]
fn observation_only_step_publishes_after_bounded_output_admission_without_receipts() {
    let mut running = ordinary_controller_loop();
    let before_step = running.completed_step();
    let bound = crate::engine::controller_output::service_reply_bound(&running).unwrap();
    let output = output_with_cap(bound);
    let reservation = output.reserve_authority_reply(bound).unwrap();

    let progress = running
        .service_after_command_drain(1200, SchedulerServiceMode::Background, None)
        .unwrap();
    let ticket_sequence = match progress {
        RunningAuthorityLoopProgress::ExternalDeliveryPending {
            ticket_sequence, ..
        } => ticket_sequence,
        other => panic!("expected retained external observations: {other:?}"),
    };
    let batch = running.pending_external_delivery().unwrap();
    assert!(!batch.events().is_empty());
    assert!(batch
        .events()
        .iter()
        .all(|event| matches!(event.delivery_kind, ExternalDeliveryEventKind::Observation)));

    let (publication, event) =
        prepare_controller_output(&mut running, bound, progress, ticket_sequence).unwrap();
    assert!(matches!(
        publication,
        RunningAuthorityLoopProgress::Published { .. }
    ));
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    assert_eq!(running.completed_step(), before_step + 1);
    assert!(running.pending_external_delivery().is_none());
    let work: serde_json::Value =
        serde_json::from_str(&running.take_work_for_test().to_json()).unwrap();
    assert_eq!(work["generation"], "0000000000000002");
    assert_eq!(work["steps"], "0000000000000001");
    assert!(work["workload"]["bodyQueryCandidates"].is_object());

    reservation.publish(event).unwrap();
    let drained = output.drain(8, bound);
    assert!(drained.events.iter().any(|event| matches!(
        event,
        CompletedEvent::Reliable(ReliableEvent::RunningAuthority(inner))
            if matches!(inner.as_ref(), RunningAuthorityEvent::ControllerObservations { .. })
    )));
}

#[test]
fn mixed_controller_batch_requires_only_the_replacement_receipt() {
    use crate::engine::contract::{ControllerJoinRequest, ControllerReclaimReceipt};
    use crate::engine::state::{ControllerKind, EntitySelectionTarget};

    let mut running = ordinary_controller_loop();
    let origin = Instant::now();
    running.set_background_clock(origin);
    let victim_id = running.generation_source_controller_leases()[0].snake_id;
    let victim_frame = running.controller_frame_v1_id(victim_id).unwrap();
    let request = ControllerJoinRequest {
        connection_id: 999,
        kind: ControllerKind::ReinforcementLearning,
        identity_key: "bot:mixed-batch".into(),
        received_at: origin + Duration::from_millis(1_200),
    };
    let RunningAuthorityEvent::ControllerJoinAssignment { lease_id, .. } = running
        .prepare_controller_join(100, &request, 1_200)
        .unwrap()
    else {
        panic!("expected a second controller");
    };
    assert!(running
        .resolve_controller_join(&ControllerReclaimReceipt {
            request_sequence: 100,
            connection_id: request.connection_id,
            lease_id,
            accepted: true,
        })
        .unwrap());
    let victim = running
        .select_entity(
            EntitySelectionTarget::Frame(victim_frame),
            running.controller_run_id(),
            2,
        )
        .unwrap()
        .unwrap();
    running
        .apply_god_mode_kill(victim.token, victim_frame)
        .unwrap();

    let before_step = running.completed_step();
    let bound = crate::engine::controller_output::service_reply_bound(&running).unwrap();
    let output = output_with_cap(bound);
    let reservation = output.reserve_authority_reply(bound).unwrap();
    let progress = running
        .service_after_command_drain(1_220, SchedulerServiceMode::Background, None)
        .unwrap();
    let RunningAuthorityLoopProgress::ExternalDeliveryPending {
        ticket_sequence,
        remaining: 2,
    } = progress
    else {
        panic!("expected a replacement and a surviving observation: {progress:?}");
    };
    let (progress, event) =
        prepare_controller_output(&mut running, bound, progress, ticket_sequence).unwrap();
    assert!(matches!(
        progress,
        RunningAuthorityLoopProgress::ExternalDeliveryPending { remaining: 1, .. }
    ));
    assert_eq!(running.completed_step(), before_step);
    let batch = running.pending_external_delivery().unwrap();
    assert_eq!(batch.status(0), Some(ExternalDeliveryStatus::Pending));
    assert_eq!(batch.status(1), Some(ExternalDeliveryStatus::Accepted));
    reservation.publish(event).unwrap();

    let drained = output.drain(8, bound);
    let [CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event))] =
        drained.events.as_slice()
    else {
        panic!("expected one atomic controller output event");
    };
    let RunningAuthorityEvent::ControllerMessages { messages, .. } = event.as_ref() else {
        panic!("expected a mixed controller output");
    };
    let wire = crate::napi_engine::running_authority_event_to_napi(event.as_ref().clone()).unwrap();
    let assignments = wire.controller_messages.unwrap();
    let observations = wire.controller_observations.unwrap();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].kind, "replacementAssignment");
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].kind, "observation");
    let [assignment, observation] = messages.as_ref() else {
        panic!("expected one replacement and one observation");
    };
    assert_eq!(observation.event.lease_id, lease_id);
    let reward = observation.event.reward.unwrap();
    let admitted = reward.envelope(running.controller_run_id()).unwrap();
    let receipt = |event: crate::engine::running_step::ExternalObservationEvent, accepted| {
        ExternalDeliveryReceipt {
            operation_epoch: event.step_key.operation_epoch(),
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted,
        }
    };
    let ignored = crate::engine::controller_output::submit_receipts(
        101,
        &[receipt(observation.event, false)],
        &mut running,
    )
    .unwrap();
    assert!(matches!(
        ignored,
        RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
            ignored_receipts: 1,
            remaining: 1,
            published_completed_step: None,
            ..
        }
    ));
    let resolved = crate::engine::controller_output::submit_receipts(
        102,
        &[receipt(assignment.event, true)],
        &mut running,
    )
    .unwrap();
    assert!(
        matches!(resolved, RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
        matched_acceptances: 1, remaining: 0, published_completed_step: Some(step), ..
    } if step == before_step + 1)
    );

    running
        .service_after_command_drain(1_240, SchedulerServiceMode::Background, None)
        .unwrap();
    let next = running
        .pending_external_delivery()
        .unwrap()
        .events()
        .iter()
        .find(|event| event.lease_id == lease_id)
        .unwrap()
        .reward
        .unwrap()
        .envelope(running.controller_run_id())
        .unwrap();
    assert_eq!(
        next.previous_observation_sequence,
        Some(admitted.observation_sequence)
    );
}

/// Pending vectors, rejected queue admission and terminal batches cannot enter the cached prefix.
#[test]
fn observation_quality_counts_only_ordinary_publication_and_never_changes_continuation() {
    let mut running = ordinary_controller_loop();
    let metrics = RunningAuthorityMetrics::new(&running);
    let empty = metrics.observation_quality_for_test().unwrap();
    let witness = running.qualification_continuation_sha256();
    metrics.observe(&running);
    assert_eq!(metrics.observation_quality_for_test().unwrap(), empty);
    assert_eq!(running.qualification_continuation_sha256(), witness);

    let bound = crate::engine::controller_output::service_reply_bound(&running).unwrap();
    assert!(
        service_running_with_output_capacity(&mut running, &output_with_cap(bound - 1), 1200)
            .is_err()
    );
    metrics.observe(&running);
    assert_eq!(metrics.observation_quality_for_test().unwrap(), empty);
    let output = output_with_cap(1024 * 1024);
    assert!(matches!(
        service_running_with_output_capacity(&mut running, &output, 1200).unwrap(),
        RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
    ));
    metrics.observe(&running);
    assert_eq!(metrics.observation_quality_for_test().unwrap(), empty);
    let messages = crate::engine::controller_output::own_pending_messages(&running).unwrap();
    assert!(!messages.is_empty());
    let receipts: Vec<_> = messages
        .iter()
        .map(|message| ExternalDeliveryReceipt {
            operation_epoch: message.event.step_key.operation_epoch(),
            event_sequence: message.event.event_sequence,
            connection_id: message.event.connection_id,
            lease_id: message.event.lease_id,
            accepted: true,
        })
        .collect();
    let stale = ExternalDeliveryReceipt {
        connection_id: u64::MAX,
        ..receipts[0]
    };
    command_reply(
        &mut running,
        RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
            receipts: vec![stale].into_boxed_slice(),
        },
        1024 * 1024,
    );
    metrics.observe(&running);
    assert_eq!(metrics.observation_quality_for_test().unwrap(), empty);

    // Use the actual command processor: its post-command observe must see this publication.
    let state = CoordinatorState::new();
    process_running_command_batch(
        CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::RunningAuthority(
                    RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                        receipts: receipts.into_boxed_slice(),
                    },
                ),
            }]
            .into_boxed_slice(),
        },
        &output,
        &state,
        &mut running,
        &metrics,
        1200,
    )
    .unwrap();
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    let text = metrics.observation_quality_for_test().unwrap();
    assert!(text.len() <= 4096);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["ordinaryPublishedSteps"], "0000000000000001");
    let expected = running.qualification_observation_counts().unwrap();
    assert!(expected[0].observations > 0);
    assert!(expected[2].observations > 0);
    for (index, counts) in expected.iter().enumerate() {
        assert_eq!(
            parsed["consumers"][index]["observations"],
            format!("{:016x}", counts.observations)
        );
        assert_eq!(
            parsed["consumers"][index]["v4Observations"],
            "0000000000000000"
        );
    }
    let witness = running.qualification_continuation_sha256();
    for _ in 0..8 {
        metrics.observe(&running);
        assert_eq!(metrics.observation_quality_for_test().unwrap(), text);
    }
    assert_eq!(running.qualification_continuation_sha256(), witness);

    let mut terminal = background_generation_handoff_fixture().unwrap().running;
    let terminal_metrics = RunningAuthorityMetrics::new(&terminal);
    advance_to_transition(&mut terminal);
    terminal_metrics.observe(&terminal);
    assert_eq!(
        terminal_metrics.observation_quality_for_test().unwrap(),
        empty
    );
}

#[test]
fn fresh_join_capacity_and_receipts_preserve_source_and_retry_once() {
    use crate::engine::contract::{ControllerJoinRequest, ControllerReclaimReceipt};
    let mut running = ordinary_controller_loop();
    let origin = Instant::now();
    running.set_background_clock(origin);
    let before = running.generation_source_controller_leases().to_vec();
    let request = RunningAuthorityCommand::JoinController(Box::new(ControllerJoinRequest {
        connection_id: 999,
        kind: before[0].kind,
        identity_key: "player:new-owner".into(),
        received_at: origin + Duration::from_millis(1250),
    }));
    let bound = running_response_owned_byte_bound(&request, &running).unwrap();
    assert!(matches!(
        execute_running_authority_command(100, request.clone(), &mut running, 1500, bound - 1)
            .unwrap(),
        RunningAuthorityEvent::CommandRejected {
            code: EngineErrorCode::QueueByteLimit,
            ..
        }
    ));
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    assert_eq!(running.generation_source_controller_leases(), before);
    let assignment =
        execute_running_authority_command(100, request.clone(), &mut running, 1500, bound).unwrap();
    let RunningAuthorityEvent::ControllerJoinAssignment {
        lease_id,
        frame_v1_id,
        ..
    } = assignment
    else {
        panic!("fresh assignment expected: {assignment:?}");
    };
    assert_eq!(running.generation_source_controller_leases(), before);
    let mut receipt = ControllerReclaimReceipt {
        request_sequence: 100,
        connection_id: 999,
        lease_id,
        accepted: false,
    };
    assert!(!running.resolve_controller_reclaim(&receipt).unwrap());
    assert!(running.resolve_controller_join(&receipt).unwrap());
    assert_eq!(running.generation_source_controller_leases(), before);
    let retry = execute_running_authority_command(101, request, &mut running, 1600, bound).unwrap();
    assert!(
        matches!(retry, RunningAuthorityEvent::ControllerJoinAssignment { lease_id: next, frame_v1_id: frame, .. }
        if next == lease_id && frame == frame_v1_id)
    );
    receipt.request_sequence = 101;
    receipt.accepted = true;
    let completion = RunningAuthorityCommand::SubmitControllerJoinReceipt(receipt.clone());
    let completion_bound = running_response_owned_byte_bound(&completion, &running).unwrap();
    assert!(matches!(
        execute_running_authority_command(
            102,
            completion.clone(),
            &mut running,
            1700,
            completion_bound - 1
        )
        .unwrap(),
        RunningAuthorityEvent::CommandRejected {
            code: EngineErrorCode::QueueByteLimit,
            ..
        }
    ));
    assert_eq!(running.generation_source_controller_leases(), before);
    assert_eq!(
        running.state(),
        RunningAuthorityLoopState::ControllerJoinPending
    );
    assert!(matches!(
        running
            .service_after_command_drain(1800, SchedulerServiceMode::Background, None)
            .unwrap(),
        RunningAuthorityLoopProgress::ControllerJoinPending
    ));
    execute_running_authority_command(102, completion, &mut running, 1800, completion_bound)
        .unwrap();
    let after = running.generation_source_controller_leases().to_vec();
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(&after[..before.len()], before);
    assert_eq!(after.last().unwrap().latest_action.accepted_at_ms, 1250);
    assert!(!running.resolve_controller_join(&receipt).unwrap());
    assert_eq!(running.generation_source_controller_leases(), after);
}

#[test]
fn reclaim_capacity_and_receipts_preserve_source_and_retry_once() {
    use crate::engine::contract::{ControllerReclaimReceipt, ControllerReclaimRequest};
    let mut running = ordinary_controller_loop();
    let origin = Instant::now();
    running.set_background_clock(origin);
    let before = running.generation_source_controller_leases()[0].clone();
    let request = RunningAuthorityCommand::ReclaimController(Box::new(ControllerReclaimRequest {
        connection_id: 999,
        kind: before.kind,
        resume_token: before.resume_token.clone(),
        identity_key: String::new(),
        received_at: origin + Duration::from_millis(1250),
    }));
    let bound = running_response_owned_byte_bound(&request, &running).unwrap();
    assert!(matches!(
        execute_running_authority_command(100, request.clone(), &mut running, 1500, bound - 1)
            .unwrap(),
        RunningAuthorityEvent::CommandRejected {
            code: EngineErrorCode::QueueByteLimit,
            ..
        }
    ));
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    assert_eq!(running.generation_source_controller_leases()[0], before);
    let output = output_with_cap(1024 * 1024);
    let fill = || {
        for sequence in 0..background_generation_handoff_runtime_init()
            .output
            .max_reliable
        {
            output
                .push_reliable(ReliableEvent::ProbeResult {
                    sequence: sequence as u64,
                    correlation_id: 0,
                    payload: Vec::new(),
                })
                .unwrap();
        }
    };
    let batch = |sequence, command| CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: vec![SequencedCommand {
            sequence,
            command: EngineCommand::RunningAuthority(command),
        }]
        .into_boxed_slice(),
    };
    let metrics = RunningAuthorityMetrics::new(&running);
    let state = CoordinatorState::new();
    fill();
    assert!(process_running_command_batch(
        batch(100, request.clone()),
        &output,
        &state,
        &mut running,
        &metrics,
        1500
    )
    .is_err());
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    output.drain(usize::MAX, usize::MAX);
    process_running_command_batch(
        batch(100, request.clone()),
        &output,
        &state,
        &mut running,
        &metrics,
        1500,
    )
    .unwrap();
    let event = output.drain(usize::MAX, usize::MAX).events.pop().unwrap();
    assert!(
        matches!(event, CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) if matches!(*event,
        RunningAuthorityEvent::ControllerReclaimAssignment { request_sequence: 100, connection_id: 999, .. }))
    );
    assert_eq!(running.generation_source_controller_leases()[0], before);
    assert_eq!(
        running.state(),
        RunningAuthorityLoopState::ControllerReclaimPending
    );
    assert_eq!(metrics.snapshot().pending_external_deliveries, 1);
    let mut receipt = ControllerReclaimReceipt {
        request_sequence: 100,
        connection_id: 998,
        lease_id: before.id,
        accepted: true,
    };
    assert!(!running.resolve_controller_reclaim(&receipt).unwrap());
    assert!(matches!(
        running
            .service_after_command_drain(1800, SchedulerServiceMode::Background, None)
            .unwrap(),
        RunningAuthorityLoopProgress::ControllerReclaimPending
    ));
    receipt.connection_id = 999;
    receipt.accepted = false;
    assert!(running.resolve_controller_reclaim(&receipt).unwrap());
    assert_eq!(running.generation_source_controller_leases()[0], before);
    process_running_command_batch(
        batch(101, request),
        &output,
        &state,
        &mut running,
        &metrics,
        1800,
    )
    .unwrap();
    output.drain(usize::MAX, usize::MAX);
    receipt.request_sequence = 101;
    receipt.accepted = true;
    let completion = RunningAuthorityCommand::SubmitControllerReclaimReceipt(receipt.clone());
    fill();
    assert!(process_running_command_batch(
        batch(102, completion.clone()),
        &output,
        &state,
        &mut running,
        &metrics,
        1900
    )
    .is_err());
    assert_eq!(running.generation_source_controller_leases()[0], before);
    assert_eq!(
        running.state(),
        RunningAuthorityLoopState::ControllerReclaimPending
    );
    output.drain(usize::MAX, usize::MAX);
    process_running_command_batch(
        batch(102, completion),
        &output,
        &state,
        &mut running,
        &metrics,
        1900,
    )
    .unwrap();
    let after = running.generation_source_controller_leases()[0].clone();
    assert_eq!(after.connection_id, Some(999));
    assert_eq!(after.snake_id, before.snake_id);
    assert_ne!(after.resume_token, before.resume_token);
    assert!(!running.resolve_controller_reclaim(&receipt).unwrap());
    assert_eq!(running.generation_source_controller_leases()[0], after);
}

#[test]
fn disconnect_output_capacity_preserves_lease_and_original_close_time() {
    let mut running = ordinary_controller_loop();
    let origin = Instant::now();
    running.set_background_clock(origin);
    let before = running.generation_source_controller_leases()[0].clone();
    let close = crate::engine::contract::ControllerDisconnectRequest {
        lease_id: before.id,
        connection_id: before.connection_id.unwrap(),
        received_at: origin + Duration::from_millis(1250),
    };
    let batch = CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: vec![SequencedCommand {
            sequence: 100,
            command: EngineCommand::RunningAuthority(
                RunningAuthorityCommand::DisconnectController(close),
            ),
        }]
        .into_boxed_slice(),
    };
    let output = output_with_cap(1024 * 1024);
    for sequence in 0..background_generation_handoff_runtime_init()
        .output
        .max_reliable
    {
        output
            .push_reliable(ReliableEvent::ProbeResult {
                sequence: sequence as u64,
                correlation_id: 0,
                payload: Vec::new(),
            })
            .unwrap();
    }
    let metrics = RunningAuthorityMetrics::new(&running);
    let state = CoordinatorState::new();
    assert!(process_running_command_batch(
        batch.clone(),
        &output,
        &state,
        &mut running,
        &metrics,
        1500
    )
    .is_err());
    assert_eq!(running.generation_source_controller_leases()[0], before);
    output.drain(usize::MAX, usize::MAX);
    process_running_command_batch(batch.clone(), &output, &state, &mut running, &metrics, 1500)
        .unwrap();
    let after = running.generation_source_controller_leases()[0].clone();
    assert_eq!(after.connection_id, None);
    assert_eq!(after.disconnected_at_ms, Some(1250));
    assert_eq!(after.grace_expires_at_ms, Some(31_250));
    assert_eq!(after.last_observed_at_ms, 1500);
    process_running_command_batch(batch, &output, &state, &mut running, &metrics, 1700).unwrap();
    assert_eq!(running.generation_source_controller_leases()[0], after);
    assert_eq!(running.completed_step(), 1);
}

#[test]
fn action_output_capacity_precedes_lease_mutation_and_exact_retry() {
    let mut running = ordinary_controller_loop();
    let origin = Instant::now();
    running.set_background_clock(origin);
    let before = running.generation_source_controller_leases()[0].clone();
    let action = crate::engine::contract::ControllerActionRequest {
        client_action_id: None,
        lease_id: before.id,
        connection_id: before.connection_id.unwrap(),
        turn: 0.75,
        boost: false,
        client_tick: 42,
        received_at: origin + Duration::from_millis(1200),
    };
    let batch = CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: vec![SequencedCommand {
            sequence: 100,
            command: EngineCommand::RunningAuthority(
                RunningAuthorityCommand::SubmitControllerAction(Box::new(action)),
            ),
        }]
        .into_boxed_slice(),
    };
    let output = output_with_cap(1024 * 1024);
    for sequence in 0..background_generation_handoff_runtime_init()
        .output
        .max_reliable
    {
        output
            .push_reliable(ReliableEvent::ProbeResult {
                sequence: sequence as u64,
                correlation_id: 0,
                payload: Vec::new(),
            })
            .unwrap();
    }
    let metrics = RunningAuthorityMetrics::new(&running);
    let state = CoordinatorState::new();
    assert!(process_running_command_batch(
        batch.clone(),
        &output,
        &state,
        &mut running,
        &metrics,
        1200
    )
    .is_err());
    assert_eq!(running.generation_source_controller_leases()[0], before);
    output.drain(usize::MAX, usize::MAX);
    process_running_command_batch(batch, &output, &state, &mut running, &metrics, 1200).unwrap();
    let after = &running.generation_source_controller_leases()[0];
    assert_eq!(after.latest_action.turn, 0.75);
    assert_eq!(after.latest_action.arrival_sequence, 100);
    assert_eq!(after.latest_action.accepted_at_ms, 1200);
    assert_eq!(running.completed_step(), 1);
}

#[test]
fn ordinary_controller_capacity_rejects_before_preparation_and_retries_once() {
    let mut running = ordinary_controller_loop();
    let before = RunningAuthorityMetrics::new(&running).snapshot();
    let bound = crate::engine::controller_output::service_reply_bound(&running).unwrap();
    let small = output_with_cap(bound - 1);
    assert_eq!(
        service_running_with_output_capacity(&mut running, &small, 1200)
            .unwrap_err()
            .code,
        EngineErrorCode::QueueByteLimit
    );
    assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
    assert!(running.pending_external_delivery().is_none());
    let output = output_with_cap(bound);
    assert!(matches!(
        service_running_with_output_capacity(&mut running, &output, 1200).unwrap(),
        RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
    ));
    let messages = crate::engine::controller_output::own_pending_messages(&running).unwrap();
    assert!(!messages.is_empty());
    let event = RunningAuthorityEvent::ControllerMessages {
        ticket_sequence: 2,
        messages: messages.clone(),
    };
    assert!(size_of::<RunningAuthorityEvent>() + event.owned_bytes() <= bound);
    let receipts: Vec<_> = messages
        .iter()
        .map(|message| ExternalDeliveryReceipt {
            operation_epoch: message.event.step_key.operation_epoch(),
            event_sequence: message.event.event_sequence,
            connection_id: message.event.connection_id,
            lease_id: message.event.lease_id,
            accepted: true,
        })
        .collect();
    let stale = ExternalDeliveryReceipt {
        connection_id: u64::MAX,
        ..receipts[0]
    };
    assert!(matches!(
        command_reply(
            &mut running,
            RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                receipts: vec![stale].into_boxed_slice()
            },
            1024 * 1024
        ),
        RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
            matched_acceptances: 0,
            ignored_receipts: 1,
            published_completed_step: None,
            ..
        }
    ));
    assert_eq!(
        crate::engine::controller_output::own_pending_messages(&running).unwrap(),
        messages
    );
    let mut duplicate_receipts = receipts;
    duplicate_receipts.push(duplicate_receipts[0]);
    let command = RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
        receipts: duplicate_receipts.into_boxed_slice(),
    };
    let batch = CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: vec![SequencedCommand {
            sequence: 1,
            command: EngineCommand::RunningAuthority(command.clone()),
        }]
        .into_boxed_slice(),
    };
    let state = CoordinatorState::new();
    let metrics = RunningAuthorityMetrics::new(&running);
    let mut byte_limits = background_generation_handoff_runtime_init().output;
    byte_limits.max_event_owned_bytes = bound;
    byte_limits.max_reliable_owned_bytes = bound;
    let byte_full = OutputQueue::new(byte_limits, Arc::new(NoopWakeSink));
    byte_full
        .push_reliable(ReliableEvent::ProbeResult {
            sequence: 0,
            correlation_id: 0,
            payload: vec![0; bound],
        })
        .unwrap();
    assert_eq!(
        process_running_command_batch(
            batch.clone(),
            &byte_full,
            &state,
            &mut running,
            &metrics,
            1200
        )
        .unwrap_err()
        .code,
        EngineErrorCode::QueueByteLimit
    );
    assert_eq!(running.completed_step(), before.completed_step);
    assert_eq!(
        crate::engine::controller_output::own_pending_messages(&running).unwrap(),
        messages
    );

    // A full queue retains the exact completion for retry before publication.
    let output = output_with_cap(1024 * 1024);
    for sequence in 0..background_generation_handoff_runtime_init()
        .output
        .max_reliable
    {
        output
            .push_reliable(ReliableEvent::ProbeResult {
                sequence: sequence as u64,
                correlation_id: 0,
                payload: Vec::new(),
            })
            .unwrap();
    }
    assert_eq!(
        process_running_command_batch(batch.clone(), &output, &state, &mut running, &metrics, 1200)
            .unwrap_err()
            .code,
        EngineErrorCode::QueueCountLimit
    );
    assert_eq!(running.completed_step(), before.completed_step);
    assert_eq!(
        crate::engine::controller_output::own_pending_messages(&running).unwrap(),
        messages
    );
    output.drain(usize::MAX, usize::MAX);
    process_running_command_batch(batch, &output, &state, &mut running, &metrics, 1200).unwrap();
    let result = output.drain(usize::MAX, usize::MAX).events.pop().unwrap();
    assert!(
        matches!(result, CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) if matches!(*event, RunningAuthorityEvent::ControllerDeliveryReceiptsApplied { matched_acceptances, ignored_receipts: 1, remaining: 0, published_completed_step: Some(_), .. } if matched_acceptances == messages.len()))
    );
    assert_eq!(running.completed_step(), before.completed_step + 1);
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    assert!(matches!(
        command_reply(&mut running, command, 1024 * 1024),
        RunningAuthorityEvent::CommandRejected { .. }
    ));
    assert_eq!(running.completed_step(), before.completed_step + 1);
}
