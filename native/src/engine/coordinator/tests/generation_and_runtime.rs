use super::*;

#[test]
fn generation_checkpoint_and_ack_byte_limits_reject_before_mutation_and_allow_exact_retry() {
    let managed = TestDirectory::create();
    let mut running = background_generation_handoff_fixture().unwrap().running;
    let old_epoch = running.world_epoch();
    advance_to_transition(&mut running);
    let command = managed.publish_command();
    let bound = running_response_owned_byte_bound(&command, &running).unwrap();
    assert_byte_rejection(command_reply(&mut running, command, bound - 1));
    assert_eq!(managed.file_count(), 0);
    assert!(running
        .pending_generation_transition()
        .unwrap()
        .checkpoint_descriptor()
        .is_none());
    assert_eq!(running.world_epoch(), old_epoch);
    let publication = command_reply(&mut running, managed.publish_command(), bound);
    assert_eq!(
        size_of::<RunningAuthorityEvent>() + publication.owned_bytes(),
        bound
    );
    let (descriptor, record) = match publication {
        RunningAuthorityEvent::GenerationCheckpointPublished {
            descriptor,
            commit_record,
            ..
        } => (*descriptor, commit_record),
        other => panic!("checkpoint did not publish: {other:?}"),
    };
    assert_eq!(
        descriptor.owned_bytes(),
        CheckpointDescriptor::publication_owned_byte_bound(&descriptor.run_id).unwrap()
    );
    assert!(matches!(
        command_reply(&mut running, managed.publish_command(), bound),
        RunningAuthorityEvent::GenerationCheckpointPublished { descriptor: retry, commit_record, .. }
            if *retry == descriptor && commit_record == record
    ));
    assert_eq!(managed.file_count(), 2);
    let ack = RunningAuthorityCommand::AcknowledgeGenerationPersistence {
        descriptor: Box::new(descriptor),
    };
    let ack_bound = running_response_owned_byte_bound(&ack, &running).unwrap();
    assert_byte_rejection(command_reply(&mut running, ack.clone(), ack_bound - 1));
    assert!(!running
        .pending_generation_transition()
        .unwrap()
        .persistence_acknowledged());
    assert!(running.pending_external_delivery().is_none());
    assert_eq!(running.world_epoch(), old_epoch);
    assert!(matches!(
        command_reply(&mut running, ack, ack_bound),
        RunningAuthorityEvent::GenerationPersistenceAcknowledged { .. }
    ));
    assert!(running
        .pending_generation_transition()
        .unwrap()
        .persistence_acknowledged());
    assert_eq!(running.world_epoch(), old_epoch);
}

#[test]
fn generation_assignment_limit_rejects_before_staging_and_accepts_exact_sized_output() {
    let managed = TestDirectory::create();
    let mut running = background_generation_handoff_fixture().unwrap().running;
    let old_epoch = running.world_epoch();
    advance_to_transition(&mut running);
    publish_and_acknowledge(&mut running, &managed);
    let command = RunningAuthorityCommand::PrepareGenerationReassignments;
    let bound = running_response_owned_byte_bound(&command, &running).unwrap();
    assert_byte_rejection(command_reply(&mut running, command.clone(), bound - 1));
    assert!(running.pending_external_delivery().is_none());
    assert_eq!(running.world_epoch(), old_epoch);
    let reply = command_reply(&mut running, command, bound);
    assert_eq!(
        size_of::<RunningAuthorityEvent>() + reply.owned_bytes(),
        bound
    );
    let assignment = match reply {
        RunningAuthorityEvent::GenerationReassignmentsPrepared {
            ready: false,
            assignments,
            ..
        } => {
            assert_eq!(assignments.len(), 1);
            assignments[0].clone()
        }
        other => panic!("assignment did not stage: {other:?}"),
    };
    assert_eq!(running.pending_external_delivery().unwrap().remaining(), 1);
    let receipts = RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
        receipts: vec![ExternalDeliveryReceipt {
            operation_epoch: assignment.operation_epoch,
            event_sequence: assignment.event_sequence,
            connection_id: assignment.connection_id,
            lease_id: assignment.lease_id,
            accepted: true,
        }]
        .into_boxed_slice(),
    };
    command_reply(&mut running, receipts.clone(), 1024 * 1024);
    assert!(matches!(
        command_reply(
            &mut running,
            RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
            1024 * 1024
        ),
        RunningAuthorityEvent::GenerationStartPublished { .. }
    ));
    assert_ne!(running.world_epoch(), old_epoch);
    assert_eq!(running.generation(), 2);
    let output = output_with_cap(1024 * 1024);
    assert!(matches!(
        service_running_with_output_capacity(&mut running, &output, 1200).unwrap(),
        RunningAuthorityLoopProgress::ExternalDeliveryPending { .. }
    ));
    let before = RunningAuthorityMetrics::new(&running).snapshot();
    let pending = running.pending_external_delivery().unwrap();
    let before_events = pending.events().to_vec();
    let before_observations: Vec<_> = (0..before_events.len())
        .map(|index| pending.observation(index).map(<[f32]>::to_vec))
        .collect();
    let before_statuses: Vec<_> = (0..before_events.len())
        .map(|index| pending.status(index))
        .collect();
    assert!(matches!(
        command_reply(&mut running, receipts, 1024 * 1024),
        RunningAuthorityEvent::CommandRejected {
            code: EngineErrorCode::InvalidCommand,
            ..
        }
    ));
    assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
    let pending = running.pending_external_delivery().unwrap();
    assert_eq!(pending.events(), before_events);
    for index in 0..before_events.len() {
        assert_eq!(
            pending.observation(index),
            before_observations[index].as_deref()
        );
        assert_eq!(pending.status(index), before_statuses[index]);
    }
}

#[test]
fn generation_final_reply_limit_preserves_old_authority_and_retained_unavailable_token() {
    let managed = TestDirectory::create();
    let mut running = background_generation_handoff_disconnected_fixture()
        .unwrap()
        .running;
    let old_epoch = running.world_epoch();
    let old_token = running.generation_source_controller_leases()[0]
        .resume_token
        .clone();
    advance_to_transition(&mut running);
    publish_and_acknowledge(&mut running, &managed);
    assert!(matches!(
        command_reply(
            &mut running,
            RunningAuthorityCommand::PrepareGenerationReassignments,
            1024 * 1024
        ),
        RunningAuthorityEvent::GenerationReassignmentsPrepared { ready: true, .. }
    ));
    let command = RunningAuthorityCommand::PublishAcknowledgedGenerationStart;
    let bound = running_response_owned_byte_bound(&command, &running).unwrap();
    assert_byte_rejection(command_reply(&mut running, command.clone(), bound - 1));
    assert_eq!(running.world_epoch(), old_epoch);
    assert_eq!(running.generation(), 1);
    assert_eq!(running.completed_step(), 0);
    assert!(running
        .pending_generation_transition()
        .unwrap()
        .persistence_acknowledged());
    let reply = command_reply(&mut running, command.clone(), bound);
    assert_eq!(
        size_of::<RunningAuthorityEvent>() + reply.owned_bytes(),
        bound
    );
    match reply {
        RunningAuthorityEvent::GenerationStartPublished { resolution, .. } => {
            let unavailable = resolution.publication.unavailable_controller_reservations;
            assert_eq!(unavailable.len(), 1);
            assert_eq!(unavailable[0].resume_token, old_token);
        }
        other => panic!("generation did not publish: {other:?}"),
    }
    assert_eq!(running.generation(), 2);
    assert_eq!(running.completed_step(), 1);
    let next_epoch = running.world_epoch();
    assert_ne!(next_epoch, old_epoch);
    assert!(matches!(
        command_reply(&mut running, command, bound),
        RunningAuthorityEvent::CommandRejected { .. }
    ));
    assert_eq!(running.world_epoch(), next_epoch);
    assert_eq!(managed.file_count(), 2);
}

#[test]
fn generation_announcement_reserves_count_before_scheduler_or_evolution_mutates() {
    let mut running = background_generation_handoff_fixture().unwrap().running;
    let mut limits = background_generation_handoff_runtime_init().output;
    limits.max_reliable = 2;
    let output = OutputQueue::new(limits, Arc::new(NoopWakeSink));
    service_running_with_output_capacity(&mut running, &output, 0).unwrap();
    output.push_reliable(ReliableEvent::Started).unwrap();
    output
        .push_reliable(ReliableEvent::ProbeResult {
            sequence: 1,
            correlation_id: 1,
            payload: Vec::new(),
        })
        .unwrap();
    let before = RunningAuthorityMetrics::new(&running).snapshot();
    assert_eq!(
        service_running_with_output_capacity(&mut running, &output, 500)
            .unwrap_err()
            .code,
        EngineErrorCode::QueueCountLimit
    );
    assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
    assert!(running.pending_generation_transition().is_none());
    assert_eq!(output.drain(usize::MAX, usize::MAX).events.len(), 2);
    assert!(matches!(
        service_running_with_output_capacity(&mut running, &output, 500).unwrap(),
        RunningAuthorityLoopProgress::GenerationTransitionPending {
            successor_generation: 2,
            ..
        }
    ));
}

#[test]
fn running_runtime_rejects_undersized_scalar_event_limit_without_consuming_authority() {
    let running = background_generation_handoff_fixture().unwrap().running;
    let before = RunningAuthorityMetrics::new(&running).snapshot();
    let mut init = background_generation_handoff_runtime_init();
    init.output.max_event_owned_bytes = size_of::<RunningAuthorityEvent>() - 1;
    let failure =
        EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink)).unwrap_err();
    assert_eq!(failure.error().code, EngineErrorCode::InvalidConfiguration);
    let running = failure.into_running_loop();
    assert_eq!(RunningAuthorityMetrics::new(&running).snapshot(), before);
}

#[test]
fn background_queue_drain_retries_retained_publication_after_count_or_byte_pressure() {
    for byte_pressure in [false, true] {
        let managed = TestDirectory::create();
        let running = background_generation_handoff_fixture().unwrap().running;
        let mut init = background_generation_handoff_runtime_init();
        init.output.max_reliable = if byte_pressure { 8 } else { 2 };
        let mut publication_probe = background_generation_handoff_fixture().unwrap().running;
        advance_to_transition(&mut publication_probe);
        let publication_bound =
            running_response_owned_byte_bound(&managed.publish_command(), &publication_probe)
                .unwrap();
        drop(publication_probe);
        // Admission must fit both reply shapes. The pressure probe below
        // fills that actual capacity, rather than assuming a generation
        // publication is always larger than a reward-capable observation.
        let output_bound = publication_bound
            .max(crate::engine::controller_output::service_reply_bound(&running).unwrap());
        init.output.max_event_owned_bytes = output_bound;
        init.output.max_reliable_owned_bytes = output_bound;
        init.inbound.max_batch_owned_bytes = output_bound;
        let runtime =
            EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink)).unwrap();
        runtime.start().unwrap();
        wait_for_runtime(&runtime, || runtime.health().output.reliable == 2);
        if byte_pressure {
            submit_runtime_command(
                &runtime,
                1,
                EngineCommand::Probe {
                    correlation_id: 1,
                    payload: vec![0; output_bound - size_of::<RunningAuthorityEvent>()],
                },
            );
            wait_for_runtime(&runtime, || runtime.health().processed_commands == 1);
        }
        let before = runtime.health().running_authority.unwrap();
        submit_runtime_command(
            &runtime,
            2,
            EngineCommand::RunningAuthority(managed.publish_command()),
        );
        wait_for_runtime(&runtime, || runtime.health().output.capacity_waits > 0);
        let blocked = runtime.health();
        assert!(blocked.fault.is_none());
        let authority = blocked.running_authority.unwrap();
        assert_eq!(authority.world_epoch, before.world_epoch);
        assert_eq!(authority.generation, 1);
        assert!(!authority.generation_checkpoint_published);
        assert_eq!(managed.file_count(), 0);
        runtime.drain_outputs(usize::MAX, usize::MAX).unwrap();
        wait_for_runtime(&runtime, || {
            runtime
                .health()
                .running_authority
                .unwrap()
                .generation_checkpoint_published
        });
        wait_for_runtime(&runtime, || {
            runtime.health().processed_commands == if byte_pressure { 2 } else { 1 }
        });
        let events = runtime
            .drain_outputs(usize::MAX, usize::MAX)
            .unwrap()
            .events;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event))
            if matches!(**event, RunningAuthorityEvent::GenerationCheckpointPublished { command_sequence: 2, .. }))
        );
        assert_eq!(managed.file_count(), 2);
        assert_eq!(
            runtime.health().running_authority.unwrap().world_epoch,
            before.world_epoch
        );
        runtime.join().unwrap();
        assert!(runtime.health().fault.is_none());
    }
}

#[test]
fn stopping_a_full_background_output_queue_unblocks_join_without_publication() {
    let managed = TestDirectory::create();
    let running = background_generation_handoff_fixture().unwrap().running;
    let mut init = background_generation_handoff_runtime_init();
    init.output.max_reliable = 2;
    let runtime =
        EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink)).unwrap();
    runtime.start().unwrap();
    wait_for_runtime(&runtime, || runtime.health().output.reliable == 2);
    submit_runtime_command(
        &runtime,
        1,
        EngineCommand::RunningAuthority(managed.publish_command()),
    );
    wait_for_runtime(&runtime, || runtime.health().output.capacity_waits > 0);
    runtime.join().unwrap();
    assert!(runtime.health().fault.is_some());
    assert_eq!(managed.file_count(), 0);
    assert!(
        !runtime
            .health()
            .running_authority
            .unwrap()
            .generation_checkpoint_published
    );
    assert!(runtime.running_authority_retained_for_test());
}
