use super::*;

/// A process-unique test-owned directory; no owner save path is used.
pub(super) struct TestDirectory(pub(super) PathBuf);

impl TestDirectory {
    pub(super) fn create() -> Self {
        let path = std::env::temp_dir().join(format!(
            "slither-generation-output-capacity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn publish_command(&self) -> RunningAuthorityCommand {
        RunningAuthorityCommand::PublishGenerationCheckpoint {
            managed_directory: self.0.to_str().unwrap().to_owned(),
            operation_id: CheckpointOperationId::parse("51515151515151515151515151515151").unwrap(),
        }
    }

    pub(super) fn file_count(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn output_with_cap(cap: usize) -> OutputQueue {
    let mut limits = background_generation_handoff_runtime_init().output;
    limits.max_event_owned_bytes = cap;
    OutputQueue::new(limits, Arc::new(NoopWakeSink))
}

pub(super) fn advance_to_transition(running: &mut RunningAuthorityLoop) {
    let output = output_with_cap(1024 * 1024);
    assert!(matches!(
        service_running_with_output_capacity(running, &output, 0).unwrap(),
        RunningAuthorityLoopProgress::Idle { .. }
    ));
    assert!(matches!(
        service_running_with_output_capacity(running, &output, 500).unwrap(),
        RunningAuthorityLoopProgress::GenerationTransitionPending { .. }
    ));
}

/// Exercise the real command processor and queue byte accounting together.
pub(super) fn command_reply(
    running: &mut RunningAuthorityLoop,
    command: RunningAuthorityCommand,
    cap: usize,
) -> RunningAuthorityEvent {
    let output = output_with_cap(cap);
    let state = CoordinatorState::new();
    let metrics = RunningAuthorityMetrics::new(running);
    process_running_command_batch(
        CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::RunningAuthority(command),
            }]
            .into_boxed_slice(),
        },
        &output,
        &state,
        running,
        &metrics,
        1000,
    )
    .unwrap();
    assert_eq!(state.processed(), (1, 1));
    let mut events = output.drain(usize::MAX, usize::MAX).events;
    assert_eq!(events.len(), 1);
    let event = events.pop().unwrap();
    assert!(event.owned_bytes() <= cap);
    match event {
        CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) => *event,
        other => panic!("unexpected output: {other:?}"),
    }
}

pub(super) fn assert_byte_rejection(event: RunningAuthorityEvent) {
    assert!(matches!(
        event,
        RunningAuthorityEvent::CommandRejected {
            code: EngineErrorCode::QueueByteLimit,
            ..
        }
    ));
}

/// Resume the existing connected-controller fixture into a normal generation.
pub(super) fn ordinary_controller_loop() -> RunningAuthorityLoop {
    let managed = TestDirectory::create();
    let mut running = background_generation_handoff_fixture().unwrap().running;
    advance_to_transition(&mut running);
    publish_and_acknowledge(&mut running, &managed);
    let assignments = match command_reply(
        &mut running,
        RunningAuthorityCommand::PrepareGenerationReassignments,
        1024 * 1024,
    ) {
        RunningAuthorityEvent::GenerationReassignmentsPrepared { assignments, .. } => assignments,
        other => panic!("missing assignments: {other:?}"),
    };
    let receipts = assignments
        .iter()
        .map(|assignment| ExternalDeliveryReceipt {
            operation_epoch: assignment.operation_epoch,
            event_sequence: assignment.event_sequence,
            connection_id: assignment.connection_id,
            lease_id: assignment.lease_id,
            accepted: true,
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    assert!(matches!(
        command_reply(
            &mut running,
            RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                receipts: receipts.clone()
            },
            1024 * 1024
        ),
        RunningAuthorityEvent::CommandRejected { .. }
    ));
    command_reply(
        &mut running,
        RunningAuthorityCommand::SubmitGenerationAssignmentReceipts { receipts },
        1024 * 1024,
    );
    command_reply(
        &mut running,
        RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
        1024 * 1024,
    );
    assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
    running
}

pub(super) fn publish_and_acknowledge(
    running: &mut RunningAuthorityLoop,
    managed: &TestDirectory,
) -> CheckpointDescriptor {
    let command = managed.publish_command();
    let bound = running_response_owned_byte_bound(&command, running).unwrap();
    let descriptor = match command_reply(running, command, bound) {
        RunningAuthorityEvent::GenerationCheckpointPublished { descriptor, .. } => *descriptor,
        other => panic!("checkpoint did not publish: {other:?}"),
    };
    assert!(matches!(
        command_reply(
            running,
            RunningAuthorityCommand::AcknowledgeGenerationPersistence {
                descriptor: Box::new(descriptor.clone()),
            },
            1024 * 1024,
        ),
        RunningAuthorityEvent::GenerationPersistenceAcknowledged { .. }
    ));
    descriptor
}

/// Observe an actual background wait or result without assuming host speed.
pub(super) fn wait_for_runtime(runtime: &EngineRuntime, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "background runtime did not progress: {:?}",
        runtime.health()
    );
}

/// Submit a single bounded command through normal runtime admission.
pub(super) fn submit_runtime_command(
    runtime: &EngineRuntime,
    sequence: u64,
    command: EngineCommand,
) {
    runtime
        .try_submit(CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand { sequence, command }].into_boxed_slice(),
        })
        .unwrap();
}
