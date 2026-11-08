use super::super::contract::{
    CommandBatch, EngineCommand, ExternalDeliveryReceipt, ReliableEvent, RunningAuthorityCommand,
    RunningAuthorityEvent, SequencedCommand,
};
use super::super::queues::NoopWakeSink;
use super::super::running_loop::{RunningAuthorityLoopError, RunningAuthorityLoopState};
use super::super::runtime::EngineRuntime;
use super::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Process-unique managed root removed after each fixture test.
struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> Self {
        let path = std::env::temp_dir().join(format!(
            "slither-generation-loop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock must follow Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&path).expect("test managed root must be creatable");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

fn submit_control(runtime: &EngineRuntime, sequence: u64, command: RunningAuthorityCommand) {
    runtime
        .try_submit(CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence,
                command: EngineCommand::RunningAuthority(command),
            }]
            .into_boxed_slice(),
        })
        .expect("background generation control must enter the bounded queue");
}

fn wait_for_running_event(
    runtime: &EngineRuntime,
    predicate: impl Fn(&RunningAuthorityEvent) -> bool,
) -> RunningAuthorityEvent {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let drained = runtime
            .drain_outputs(usize::MAX, usize::MAX)
            .expect("background fixture output drain must succeed");
        for completed in drained.events {
            if let super::super::contract::CompletedEvent::Reliable(
                ReliableEvent::RunningAuthority(event),
            ) = completed
            {
                if predicate(&event) {
                    return *event;
                }
            }
        }
        std::thread::yield_now();
    }
    panic!(
        "background authority event did not arrive: {:?}",
        runtime.health()
    );
}

fn wait_for_health(runtime: &EngineRuntime, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "background authority health did not advance: {:?}",
        runtime.health()
    );
}

#[test]
fn retained_fixture_starts_with_one_real_unpublished_terminal_transition() {
    let fixture = GenerationHandoffFixtureSession::new().expect("fixture must construct");
    let snapshot = fixture.snapshot();
    assert!(snapshot.world_epoch > 0);
    assert_eq!(snapshot.generation, 1);
    assert_eq!(snapshot.completed_step, 0);
    assert!(snapshot.transition_pending);
    assert!(!snapshot.checkpoint_published);
    assert!(!snapshot.persistence_acknowledged);
    assert_eq!(snapshot.generation_checkpoint_publications, 0);
    assert_eq!(snapshot.authority_publications, 0);
}

#[test]
fn background_runtime_routes_one_exact_generation_barrier_without_duplicate_publication() {
    let managed = TestDirectory::create();
    let BackgroundGenerationHandoffFixture { run_start, running } =
        background_generation_handoff_fixture().expect("background fixture must construct");
    let run_start_descriptor = run_start
        .publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("31313131313131313131313131313131")
                .expect("run-start operation must parse"),
            1,
        )
        .expect("run-start checkpoint must publish");
    let runtime = EngineRuntime::new_running_authority(
        background_generation_handoff_runtime_init(),
        running,
        Arc::new(NoopWakeSink),
    )
    .expect("background fixture must enter the real runtime");
    let source_health = runtime
        .health()
        .running_authority
        .expect("running health must exist before start");
    runtime
        .start()
        .expect("background fixture runtime must start");

    let transition = wait_for_running_event(&runtime, |event| {
        matches!(
            event,
            RunningAuthorityEvent::GenerationTransitionPending { .. }
        )
    });
    assert!(matches!(
        transition,
        RunningAuthorityEvent::GenerationTransitionPending {
            successor_generation: 2,
            successor_completed_step: 1,
            ..
        }
    ));
    let blocked = runtime
        .health()
        .running_authority
        .expect("running health must remain present");
    assert_eq!(
        blocked.loop_state,
        RunningAuthorityLoopState::GenerationTransitionPending
    );
    assert_eq!(blocked.world_epoch, source_health.world_epoch);
    assert_eq!(blocked.generation, 1);
    assert!(!blocked.generation_checkpoint_published);

    submit_control(
        &runtime,
        1,
        RunningAuthorityCommand::AcknowledgeGenerationPersistence {
            descriptor: Box::new(run_start_descriptor),
        },
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(event, RunningAuthorityEvent::CommandRejected { command_sequence: 1, .. })),
        RunningAuthorityEvent::CommandRejected { detail, .. }
            if detail.contains("checkpoint") && detail.contains("publication")
    ));
    assert_eq!(
        runtime
            .health()
            .running_authority
            .expect("rejected acknowledgement retains health")
            .world_epoch,
        source_health.world_epoch
    );

    let generation_operation = CheckpointOperationId::parse("41414141414141414141414141414141")
        .expect("generation operation must parse");
    submit_control(
        &runtime,
        2,
        RunningAuthorityCommand::PublishGenerationCheckpoint {
            managed_directory: managed.path().to_string_lossy().into_owned(),
            operation_id: generation_operation.clone(),
        },
    );
    let (descriptor, commit_record) = match wait_for_running_event(&runtime, |event| {
        matches!(
            event,
            RunningAuthorityEvent::GenerationCheckpointPublished {
                command_sequence: 2,
                ..
            }
        )
    }) {
        RunningAuthorityEvent::GenerationCheckpointPublished {
            descriptor,
            commit_record,
            ..
        } => (*descriptor, commit_record),
        other => panic!("unexpected checkpoint event: {other:?}"),
    };
    assert_eq!(descriptor.operation_id, generation_operation);
    assert_eq!(commit_record.summary.completed_generation, 1);
    assert_eq!(commit_record.hall_of_fame.completed_generation, 1);
    assert_ne!(commit_record.hall_of_fame.successor_genome_id, 0);

    submit_control(
        &runtime,
        3,
        RunningAuthorityCommand::PublishGenerationCheckpoint {
            managed_directory: managed.path().to_string_lossy().into_owned(),
            operation_id: generation_operation,
        },
    );
    let retry_descriptor = match wait_for_running_event(&runtime, |event| {
        matches!(
            event,
            RunningAuthorityEvent::GenerationCheckpointPublished {
                command_sequence: 3,
                ..
            }
        )
    }) {
        RunningAuthorityEvent::GenerationCheckpointPublished {
            descriptor,
            commit_record: retry_record,
            ..
        } => {
            assert_eq!(retry_record, commit_record);
            *descriptor
        }
        other => panic!("unexpected retry event: {other:?}"),
    };
    assert_eq!(retry_descriptor, descriptor);

    let mut mismatch = descriptor.clone();
    mismatch.logical_root_sha256 = "f".repeat(64);
    mismatch.relative_filename = format!("{}.checkpoint-v3", mismatch.logical_root_sha256);
    submit_control(
        &runtime,
        4,
        RunningAuthorityCommand::AcknowledgeGenerationPersistence {
            descriptor: Box::new(mismatch),
        },
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(event, RunningAuthorityEvent::CommandRejected { command_sequence: 4, .. })),
        RunningAuthorityEvent::CommandRejected { detail, .. } if detail.contains("logical root")
    ));
    let mismatched = runtime
        .health()
        .running_authority
        .expect("mismatch retains running health");
    assert_eq!(mismatched.world_epoch, source_health.world_epoch);
    assert!(!mismatched.generation_persistence_acknowledged);

    submit_control(
        &runtime,
        5,
        RunningAuthorityCommand::AcknowledgeGenerationPersistence {
            descriptor: Box::new(descriptor.clone()),
        },
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(event, RunningAuthorityEvent::GenerationPersistenceAcknowledged { command_sequence: 5, .. })),
        RunningAuthorityEvent::GenerationPersistenceAcknowledged { operation_id, .. }
            if operation_id == descriptor.operation_id
    ));
    assert!(
        runtime
            .health()
            .running_authority
            .expect("exact acknowledgement updates bounded health")
            .generation_persistence_acknowledged
    );

    submit_control(
        &runtime,
        6,
        RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(
            event,
            RunningAuthorityEvent::CommandRejected {
                command_sequence: 6,
                ..
            }
        )),
        RunningAuthorityEvent::CommandRejected { .. }
    ));
    assert_eq!(
        runtime
            .health()
            .running_authority
            .expect("premature finalization retains old authority")
            .world_epoch,
        source_health.world_epoch
    );

    submit_control(
        &runtime,
        7,
        RunningAuthorityCommand::PrepareGenerationReassignments,
    );
    let assignment = match wait_for_running_event(&runtime, |event| {
        matches!(
            event,
            RunningAuthorityEvent::GenerationReassignmentsPrepared {
                command_sequence: 7,
                ..
            }
        )
    }) {
        RunningAuthorityEvent::GenerationReassignmentsPrepared {
            ready: false,
            assignments,
            ..
        } if assignments.len() == 1 => assignments[0].clone(),
        other => panic!("unexpected generation assignment event: {other:?}"),
    };
    assert_eq!(assignment.controller_kind, ControllerKind::Player);
    assert!(!assignment.resume_token.is_empty());

    submit_control(
        &runtime,
        8,
        RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
            receipts: vec![ExternalDeliveryReceipt {
                operation_epoch: assignment.operation_epoch,
                event_sequence: assignment.event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id.saturating_add(1),
                accepted: true,
            }]
            .into_boxed_slice(),
        },
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(
            event,
            RunningAuthorityEvent::GenerationAssignmentReceiptsApplied {
                command_sequence: 8,
                ..
            }
        )),
        RunningAuthorityEvent::GenerationAssignmentReceiptsApplied {
            matched_acceptances: 0,
            ignored_receipts: 1,
            state: super::super::contract::GenerationAssignmentReceiptState::Pending {
                remaining: 1
            },
            ..
        }
    ));
    assert_eq!(
        runtime
            .health()
            .running_authority
            .expect("mismatched receipt retains old authority")
            .world_epoch,
        source_health.world_epoch
    );

    submit_control(
        &runtime,
        9,
        RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
            receipts: vec![ExternalDeliveryReceipt {
                operation_epoch: assignment.operation_epoch,
                event_sequence: assignment.event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id,
                accepted: true,
            }]
            .into_boxed_slice(),
        },
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(
            event,
            RunningAuthorityEvent::GenerationAssignmentReceiptsApplied {
                command_sequence: 9,
                ..
            }
        )),
        RunningAuthorityEvent::GenerationAssignmentReceiptsApplied {
            matched_acceptances: 1,
            ignored_receipts: 0,
            state: super::super::contract::GenerationAssignmentReceiptState::Ready {
                successor_generation: 2,
                successor_completed_step: 1,
                ..
            },
            ..
        }
    ));

    submit_control(
        &runtime,
        10,
        RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
    );
    let final_resolution = match wait_for_running_event(&runtime, |event| {
        matches!(
            event,
            RunningAuthorityEvent::GenerationStartPublished {
                command_sequence: 10,
                ..
            }
        )
    }) {
        RunningAuthorityEvent::GenerationStartPublished { resolution, .. } => resolution,
        other => panic!("unexpected final publication event: {other:?}"),
    };
    assert_eq!(final_resolution.publication.generation, 2);
    assert_eq!(final_resolution.publication.completed_step, 1);
    assert_eq!(final_resolution.publication.external_assignments, 1);
    assert_ne!(
        final_resolution.publication.world_epoch,
        source_health.world_epoch
    );
    wait_for_health(&runtime, || {
        runtime.health().running_authority.is_some_and(|health| {
            health.generation == 2
                && health.completed_step == 1
                && health.world_epoch == final_resolution.publication.world_epoch
        })
    });

    submit_control(
        &runtime,
        11,
        RunningAuthorityCommand::PublishGenerationCheckpoint {
            managed_directory: managed.path().to_string_lossy().into_owned(),
            operation_id: CheckpointOperationId::parse("51515151515151515151515151515151")
                .expect("duplicate operation must parse"),
        },
    );
    assert!(matches!(
        wait_for_running_event(&runtime, |event| matches!(
            event,
            RunningAuthorityEvent::CommandRejected {
                command_sequence: 11,
                ..
            }
        )),
        RunningAuthorityEvent::CommandRejected { .. }
    ));
    assert_eq!(
        std::fs::read_dir(managed.path())
            .expect("managed directory must remain readable")
            .count(),
        3
    );

    runtime.request_stop();
    runtime.join().expect("background fixture must join");
    let final_health = runtime.health();
    assert!(final_health.fault.is_none());
    assert_eq!(final_health.processed_commands, 11);
}

#[test]
fn retained_fixture_finishes_one_connected_generation_barrier_and_rebinds() {
    let managed = TestDirectory::create();
    let mut fixture = GenerationHandoffFixtureSession::new().expect("fixture must construct");
    let source = fixture.snapshot();
    let terminal_diagnostics = fixture.running.scheduler_diagnostics();
    assert!(terminal_diagnostics.step_pending);
    assert_eq!(terminal_diagnostics.completed_steps, 0);

    assert!(matches!(
        fixture.publish_generation_start(),
        Err(detail) if detail.contains("requires a successful persistence acknowledgement")
    ));
    assert_eq!(fixture.snapshot(), source);

    let operation = CheckpointOperationId::parse("71717171717171717171717171717171")
        .expect("fixture operation ID must be canonical");
    let publication = fixture
        .publish_generation_checkpoint(managed.path(), operation.clone())
        .expect("fixture generation checkpoint must publish");
    let retry = fixture
        .publish_generation_checkpoint(managed.path(), operation)
        .expect("fixture checkpoint retry must be exact");
    assert_eq!(retry, publication);
    let work: serde_json::Value =
        serde_json::from_str(&publication.workload.unwrap().to_json()).unwrap();
    assert_eq!(work["generation"], "0000000000000001");
    assert_eq!(work["steps"], "0000000000000001");
    assert_eq!(work["partial"], true);
    assert!(work["timingsNs"]["bodyRayPrepare"].is_null());
    assert_eq!(fixture.snapshot().generation_checkpoint_publications, 1);

    let mut mismatch = publication.descriptor.clone();
    mismatch.logical_root_sha256 = "f".repeat(64);
    mismatch.relative_filename = format!("{}.checkpoint-v3", mismatch.logical_root_sha256);
    assert!(matches!(
        fixture.acknowledge_generation_persistence(&mismatch),
        Err(detail) if detail.contains("logical root")
    ));
    assert!(!fixture.snapshot().persistence_acknowledged);
    fixture
        .acknowledge_generation_persistence(&publication.descriptor)
        .expect("exact descriptor must acknowledge");
    assert!(fixture.snapshot().persistence_acknowledged);
    assert_eq!(fixture.snapshot().world_epoch, source.world_epoch);

    let assignment = fixture
        .prepare_generation_assignment()
        .expect("connected controller must receive one retained assignment");
    assert!(matches!(
        fixture.publish_generation_start(),
        Err(detail) if detail.contains("still require matching Node delivery results")
    ));
    assert_eq!(fixture.snapshot().world_epoch, source.world_epoch);
    fixture
        .submit_generation_assignment(
            assignment.operation_epoch,
            assignment.event_sequence,
            assignment.connection_id,
            assignment.lease_id,
            true,
        )
        .expect("exact connected assignment result must resolve");
    let published = fixture
        .publish_generation_start()
        .expect("durable assignment-ready generation must publish once");
    assert_eq!(published.generation, 2);
    assert_eq!(published.completed_step, 1);
    assert_eq!(published.external_assignments, 1);
    let final_snapshot = fixture.snapshot();
    assert_ne!(final_snapshot.world_epoch, source.world_epoch);
    assert_eq!(final_snapshot.generation, 2);
    assert_eq!(final_snapshot.completed_step, 1);
    assert!(!final_snapshot.transition_pending);
    assert_eq!(final_snapshot.generation_checkpoint_publications, 1);
    assert_eq!(final_snapshot.authority_publications, 1);
    assert_eq!(fixture.running.state(), RunningAuthorityLoopState::Ready);
    let rebound = fixture.running.scheduler_diagnostics();
    assert_eq!(rebound.completed_steps, 1);
    assert!(!rebound.step_pending);
    assert!(matches!(
        fixture
            .running
            .publish_acknowledged_generation_start(FIXTURE_RESUME_WALL_MS, None),
        Err(RunningAuthorityLoopError::InvalidActionState { .. })
    ));
    assert_eq!(fixture.snapshot(), final_snapshot);
    let old_assignment = fixture
        .pending_assignment
        .expect("fixture must retain the already-resolved test envelope");
    let stale = fixture
        .running
        .submit_external_delivery_results(
            &[ExternalDeliveryResult {
                step_key: old_assignment.step_key,
                event_sequence: old_assignment.event_sequence,
                connection_id: old_assignment.connection_id,
                lease_id: old_assignment.lease_id,
                accepted: true,
            }],
            None,
        )
        .expect("late resolved assignment must be ignored without changing authority");
    assert_eq!(stale.matched_acceptances, 0);
    assert_eq!(stale.matched_failures, 0);
    assert_eq!(stale.ignored_results, 1);
    assert_eq!(stale.state, RunningAuthorityDeliveryState::Idle);
    assert_eq!(fixture.snapshot(), final_snapshot);

    let mut wall_now_ms = FIXTURE_RESUME_WALL_MS;
    let pending_event = (0..=60)
        .find_map(|_| {
            let progress = fixture
                .running
                .service_after_command_drain(wall_now_ms, SchedulerServiceMode::Background, None)
                .expect("rebound loop must continue through generation two");
            match progress {
                RunningAuthorityLoopProgress::Idle { .. } => {
                    wall_now_ms = wall_now_ms.saturating_add(17);
                    None
                }
                RunningAuthorityLoopProgress::Published { .. } => None,
                RunningAuthorityLoopProgress::ExternalDeliveryPending { .. } => Some(
                    fixture
                        .running
                        .pending_external_delivery()
                        .expect("external blocker must retain one batch")
                        .events()[0],
                ),
                RunningAuthorityLoopProgress::ControllerReclaimPending
                | RunningAuthorityLoopProgress::ControllerJoinPending
                | RunningAuthorityLoopProgress::ImportPending => {
                    panic!("no lifecycle command was requested in this fixture")
                }
                RunningAuthorityLoopProgress::GenerationTransitionPending { .. } => {
                    panic!("generation two must not terminate during delivery-resume proof")
                }
            }
        })
        .expect("connected successor must eventually request one external observation");
    let completed_before_delivery = fixture.running.completed_step();
    let resolution = fixture
        .running
        .submit_external_delivery_results(
            &[ExternalDeliveryResult {
                step_key: pending_event.step_key,
                event_sequence: pending_event.event_sequence,
                connection_id: pending_event.connection_id,
                lease_id: pending_event.lease_id,
                accepted: true,
            }],
            None,
        )
        .expect("exact generation-two observation result must resume publication");
    assert_eq!(resolution.matched_acceptances, 1);
    assert_eq!(resolution.matched_failures, 0);
    assert_eq!(resolution.ignored_results, 0);
    assert!(matches!(
        resolution.state,
        RunningAuthorityDeliveryState::RunningStepPublished { publication, .. }
            if publication.completed_step == completed_before_delivery + 1
    ));
    assert_eq!(fixture.running.state(), RunningAuthorityLoopState::Ready);
    assert_eq!(
        fixture.running.scheduler_diagnostics().completed_steps,
        fixture.running.completed_step()
    );
}
