use super::*;
use crate::engine::contract::RunningAuthorityCommand;
use crate::engine::queues::WakeSink;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

#[test]
fn explicit_inference_selection_rejects_unavailable_cpu_and_unknown_identity() {
    use crate::engine::inference::InferenceMathBackend;
    assert_eq!(
        parse_requested_math_backend(None, |_| panic!("omission needs no CPU override")),
        Ok(None)
    );
    for backend in [
        InferenceMathBackend::Scalar,
        InferenceMathBackend::Sse2,
        InferenceMathBackend::I16MatrixFma128,
        InferenceMathBackend::I16AffineFma128,
    ] {
        assert_eq!(
            parse_requested_math_backend(Some(backend.label()), |_| true),
            Ok(Some(backend))
        );
        let error = parse_requested_math_backend(Some(backend.label()), |_| false).unwrap_err();
        assert!(error.contains(backend.label()) && error.contains("unavailable on this CPU"));
    }
    for label in ["unknown-v1", "benchmark-f32-fma128-v1", "", "constructor"] {
        assert!(parse_requested_math_backend(Some(label), |_| panic!(
            "unknown identities never reach CPU admission"
        ))
        .unwrap_err()
        .contains("unknown inference math backend"));
    }
}

#[test]
fn retained_origin_wire_metadata_preserves_full_width_ids_and_source_classes() {
    use crate::engine::brain_artifact::BrainSourceKind;
    use crate::engine::retained_origin::RetainedNeuralOrigin;
    let winner = BackgroundRetainedOrigin::from(RetainedNeuralOrigin::HallOfFame {
        entry_id: u64::MAX,
        weight_digest: [0xab; 32],
    });
    assert_eq!(winner.kind, "hall-of-fame");
    assert_eq!(winner.entry_id.as_deref(), Some("ffffffffffffffff"));
    assert_eq!(winner.weights_sha256, "ab".repeat(32));
    assert!(winner.brain_id.is_none() && winner.source_kind.is_none());
    for (source_kind, label) in [
        (BrainSourceKind::Distilled, "distilled"),
        (BrainSourceKind::Imported, "imported"),
        (BrainSourceKind::Manual, "manual"),
    ] {
        let saved = BackgroundRetainedOrigin::from(RetainedNeuralOrigin::SavedBrain {
            brain_id: [0xcd; 32],
            weight_digest: [0xab; 32],
            source_kind,
        });
        assert_eq!(saved.kind, "saved-brain");
        assert_eq!(saved.brain_id, Some("cd".repeat(32)));
        assert_eq!(saved.weights_sha256, "ab".repeat(32));
        assert_eq!(saved.source_kind.as_deref(), Some(label));
        assert!(saved.entry_id.is_none());
    }
}

fn valid_safety_init() -> EngineInit {
    EngineInit {
        contract_version: ENGINE_CONTRACT_VERSION,
        inbound: InboundLimits {
            max_batches: 4,
            max_commands: 8,
            max_owned_bytes: 4_096,
            max_batch_commands: 4,
            max_batch_owned_bytes: 2_048,
        },
        output: OutputLimits {
            max_reliable: 8,
            max_reliable_owned_bytes: 4_096,
            max_discrete: 4,
            max_discrete_owned_bytes: 4_096,
            max_total_owned_bytes: 8_192,
            max_event_owned_bytes: 1_024,
            max_frame_connections: 4,
        },
    }
}

#[test]
fn bridge_safety_rejects_count_and_byte_ceiling_plus_one() {
    let mut count = valid_safety_init();
    count.inbound.max_commands = MAX_NAPI_QUEUE_ENTRY_COUNT + 1;
    let count_error = validate_bridge_safety_limits(&count).expect_err("count must fail");
    assert_eq!(count_error.code, EngineErrorCode::InvalidConfiguration);
    assert!(count_error.detail.contains("maxInboundCommands"));

    let mut batch_count = valid_safety_init();
    batch_count.inbound.max_batch_commands = MAX_NAPI_QUEUE_ENTRY_COUNT + 1;
    let batch_error =
        validate_bridge_safety_limits(&batch_count).expect_err("batch count must fail");
    assert!(batch_error.detail.contains("maxBatchCommands"));

    let mut event_bytes = valid_safety_init();
    event_bytes.output.max_event_owned_bytes = MAX_NAPI_EVENT_OWNED_BYTES + 1;
    let event_error =
        validate_bridge_safety_limits(&event_bytes).expect_err("event bytes must fail");
    assert!(event_error.detail.contains("maxOutputEventOwnedBytes"));

    let mut batch_bytes = valid_safety_init();
    batch_bytes.inbound.max_batch_owned_bytes = MAX_NAPI_BATCH_OWNED_BYTES + 1;
    let byte_error =
        validate_bridge_safety_limits(&batch_bytes).expect_err("batch bytes must fail");
    assert!(byte_error.detail.contains("maxBatchOwnedBytes"));
}

#[test]
fn command_metadata_and_payload_copy_are_concretely_bounded() {
    let metadata_bytes = MAX_NAPI_QUEUE_ENTRY_COUNT
        .checked_mul(std::mem::size_of::<SequencedCommand>())
        .expect("supported command metadata multiplication is representable");
    assert!(metadata_bytes <= MAX_NAPI_BATCH_METADATA_BYTES);
    let token = crate::engine::state::EntitySelectionToken::parse(&"1".repeat(64)).unwrap();
    let batch = CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: vec![SequencedCommand {
            sequence: 1,
            command: EngineCommand::RunningAuthority(RunningAuthorityCommand::GodModeKill {
                frame_v1_id: 1,
                token: Box::new(token),
            }),
        }]
        .into_boxed_slice(),
    };
    assert_eq!(
        batch.validate().unwrap().owned_bytes,
        std::mem::size_of::<crate::engine::state::EntitySelectionToken>()
    );
    assert_eq!(
        metadata_bytes
            .checked_add(MAX_NAPI_BATCH_OWNED_BYTES)
            .expect("combined bound is representable"),
        metadata_bytes + MAX_NAPI_BATCH_OWNED_BYTES
    );
    assert!(metadata_bytes + MAX_NAPI_BATCH_OWNED_BYTES <= MAX_NAPI_BATCH_RESERVATION_BYTES);

    let mut reserved = Vec::<SequencedCommand>::new();
    reserved
        .try_reserve_exact(1_024)
        .expect("small command metadata reservation succeeds");
    validate_batch_reservation(reserved.capacity(), 4_096)
        .expect("actual allocator-returned capacity is charged and bounded");
    let actual_metadata_bytes = reserved.capacity() * std::mem::size_of::<SequencedCommand>();
    assert!(actual_metadata_bytes + 4_096 <= MAX_NAPI_BATCH_RESERVATION_BYTES);

    let copied = copy_probe_payload(&[1, 2, 3, 4], 0).expect("small payload copies");
    assert_eq!(copied, [1, 2, 3, 4]);
    assert!(copied.capacity() >= copied.len());
    assert!(validate_probe_payload_len(MAX_NAPI_EVENT_OWNED_BYTES + 1).is_err());

    assert!(validate_payload_before_copy(5, 6, 10, 1, 0).is_err());
    let metadata = command_metadata_bytes(1).expect("one command metadata is bounded");
    let nearly_full = MAX_NAPI_BATCH_RESERVATION_BYTES - metadata - 1;
    let hard_error = validate_payload_before_copy(2, nearly_full, usize::MAX, 1, 0)
        .expect_err("logical bytes exceeding the hard remainder fail before copy");
    assert!(hard_error.detail.contains("hard temporary bytes remaining"));
}

#[test]
fn queue_full_is_a_coalesced_wake_and_closing_is_terminal() {
    assert_eq!(wake_status_to_result(Status::QueueFull), Ok(()));
    let closing = wake_status_to_result(Status::Closing).expect_err("closing must fail");
    assert_eq!(closing.code, EngineErrorCode::WakeDelivery);
}

struct ClosingWake {
    runtime: Mutex<Weak<EngineRuntime>>,
    notifications: AtomicU64,
}

impl ClosingWake {
    fn new() -> Self {
        Self {
            runtime: Mutex::new(Weak::new()),
            notifications: AtomicU64::new(0),
        }
    }
}

impl WakeSink for ClosingWake {
    fn notify(&self) -> std::result::Result<(), EngineError> {
        self.notifications.fetch_add(1, Ordering::Relaxed);
        apply_wake_status(Status::Closing, &lock_recover(&self.runtime))
    }
}

#[test]
fn closing_wake_faults_runtime_once_without_recursive_notification() {
    let wake = Arc::new(ClosingWake::new());
    let init = EngineInit {
        contract_version: ENGINE_CONTRACT_VERSION,
        inbound: InboundLimits {
            max_batches: 2,
            max_commands: 4,
            max_owned_bytes: 32,
            max_batch_commands: 2,
            max_batch_owned_bytes: 16,
        },
        output: OutputLimits {
            max_reliable: 4,
            max_reliable_owned_bytes: 32,
            max_discrete: 2,
            max_discrete_owned_bytes: 32,
            max_total_owned_bytes: 64,
            max_event_owned_bytes: 16,
            max_frame_connections: 2,
        },
    };
    let runtime = Arc::new(
        EngineRuntime::new_experimental_probe(init, Arc::clone(&wake) as Arc<dyn WakeSink>)
            .expect("valid runtime"),
    );
    *lock_recover(&wake.runtime) = Arc::downgrade(&runtime);
    runtime.start().expect("coordinator starts");

    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.health().lifecycle != LifecycleState::Faulted && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let health = runtime.health();
    assert_eq!(health.lifecycle, LifecycleState::Faulted);
    assert_eq!(
        health.fault.as_ref().map(|fault| fault.code()),
        Some(EngineErrorCode::WakeDelivery)
    );
    assert_eq!(wake.notifications.load(Ordering::Relaxed), 1);
    runtime.join().expect("faulted coordinator joins");
}

#[test]
fn bigint_conversion_preserves_maximum_u64() {
    assert_eq!(
        require_lossless_u64(u64::MAX, true, "value").expect("u64 max remains exact"),
        u64::MAX
    );
    assert!(require_lossless_u64(u64::MAX, false, "value").is_err());
}

#[test]
fn fresh_run_exact_hex_inputs_never_accept_number_shaped_shortcuts() {
    assert_eq!(parse_u32_hex("00000000", "seedHex").expect("zero seed"), 0);
    assert_eq!(
        parse_u32_hex("ffffffff", "seedHex").expect("maximum seed"),
        u32::MAX
    );
    assert_eq!(
        parse_u64_hex("0000000000000001", "memoryCeilingBytesHex", false)
            .expect("positive ceiling"),
        1
    );
    for invalid in ["0", "0000000A", "100000000", "gggggggg"] {
        assert!(
            parse_u32_hex(invalid, "seedHex").is_err(),
            "accepted invalid seed {invalid}"
        );
    }
    for invalid in [
        "0000000000000000",
        "1",
        "000000000000000A",
        "10000000000000000",
    ] {
        assert!(
            parse_u64_hex(invalid, "memoryCeilingBytesHex", false).is_err(),
            "accepted invalid ceiling {invalid}"
        );
    }
}

#[test]
fn fresh_run_session_exposes_one_nonblocking_operation_owner() {
    let session = ExperimentalStage6aFreshRunSession {
        requested_math: None,
        request: Stage6aP0FreshRunRequest {
            run_id: "unit-run".to_owned(),
            seed: 1,
            memory_ceiling_bytes: 1,
        },
        calculation_workers: 1,
        inner: Arc::new(Mutex::new(ExperimentalFreshRunInner::default())),
        active_operation: Arc::new(AtomicU8::new(FRESH_OPERATION_IDLE)),
    };
    let created = fresh_run_scalar_snapshot(
        &lock_recover(&session.inner),
        session.active_operation.load(Ordering::Acquire),
    )
    .expect("created snapshot");
    assert_eq!(created.phase, "created");
    assert_eq!(created.transition_epoch, None);

    session
        .begin_operation(FRESH_OPERATION_INITIALIZE)
        .expect("first owner acquires operation");
    assert!(session.begin_operation(FRESH_OPERATION_CHECKPOINT).is_err());
    let initializing = fresh_run_scalar_snapshot(
        &lock_recover(&session.inner),
        session.active_operation.load(Ordering::Acquire),
    )
    .expect("initializing snapshot");
    assert_eq!(initializing.phase, "initializing");
    assert_eq!(initializing.authority_published, None);
    assert!(session
        .begin_synchronous_operation(FRESH_OPERATION_ACKNOWLEDGE)
        .is_err());

    session
        .active_operation
        .store(FRESH_OPERATION_IDLE, Ordering::Release);
    {
        let _acknowledgement = session
            .begin_synchronous_operation(FRESH_OPERATION_ACKNOWLEDGE)
            .expect("synchronous acknowledgement owns the slot");
        let acknowledging = fresh_run_scalar_snapshot(
            &lock_recover(&session.inner),
            session.active_operation.load(Ordering::Acquire),
        )
        .expect("acknowledgement snapshot");
        assert_eq!(acknowledging.phase, "acknowledgingPersistence");
        assert!(session.begin_operation(FRESH_OPERATION_ACTIVATE).is_err());
    }
    assert_eq!(
        session.active_operation.load(Ordering::Acquire),
        FRESH_OPERATION_IDLE
    );
    session
        .begin_operation(FRESH_OPERATION_CHECKPOINT)
        .expect("operation slot is reusable after completion");
    session
        .active_operation
        .store(FRESH_OPERATION_IDLE, Ordering::Release);
    session
        .begin_operation(FRESH_OPERATION_INITIAL_FRAME)
        .expect("initial frame owns the same coarse operation slot");
    let publishing_frame = fresh_run_scalar_snapshot(
        &lock_recover(&session.inner),
        session.active_operation.load(Ordering::Acquire),
    )
    .expect("initial frame snapshot");
    assert_eq!(publishing_frame.phase, "publishingInitialFrame");
    assert_eq!(publishing_frame.initial_frame_published, None);
    session
        .active_operation
        .store(FRESH_OPERATION_IDLE, Ordering::Release);
    session
        .begin_operation(FRESH_OPERATION_FIRST_SCHEDULED_FRAME)
        .expect("first scheduled frame owns the same coarse operation slot");
    let publishing_scheduled = fresh_run_scalar_snapshot(
        &lock_recover(&session.inner),
        session.active_operation.load(Ordering::Acquire),
    )
    .expect("first scheduled frame snapshot");
    assert_eq!(publishing_scheduled.phase, "publishingFirstScheduledFrame");
    assert_eq!(publishing_scheduled.first_scheduled_frame_published, None);
}

#[test]
fn fresh_run_worker_panic_permanently_faults_with_bounded_first_detail() {
    let session = ExperimentalStage6aFreshRunSession {
        requested_math: None,
        request: Stage6aP0FreshRunRequest {
            run_id: "unit-fault-run".to_owned(),
            seed: 1,
            memory_ceiling_bytes: 1,
        },
        calculation_workers: 1,
        inner: Arc::new(Mutex::new(ExperimentalFreshRunInner::default())),
        active_operation: Arc::new(AtomicU8::new(FRESH_OPERATION_IDLE)),
    };
    let oversized_payload = "🐍".repeat(MAX_ERROR_DETAIL_BYTES);
    let _ = fault_experimental_fresh_run(
        &session.inner,
        "experimental fresh-run unit panic",
        &oversized_payload,
    );
    let faulted = fresh_run_scalar_snapshot(
        &lock_recover(&session.inner),
        session.active_operation.load(Ordering::Acquire),
    )
    .expect("fault snapshot");
    let first_detail = faulted.fault_detail.expect("retained fault detail");
    assert_eq!(faulted.phase, "faulted");
    assert!(first_detail.starts_with("experimental fresh-run unit panic: "));
    assert!(first_detail.len() <= MAX_ERROR_DETAIL_BYTES);
    assert_eq!(faulted.transition_epoch, None);
    assert_eq!(faulted.authority_published, None);

    assert!(session.begin_operation(FRESH_OPERATION_INITIALIZE).is_err());
    assert_eq!(
        session.active_operation.load(Ordering::Acquire),
        FRESH_OPERATION_IDLE
    );
    assert!(ensure_fresh_transition_absent(&session.inner).is_err());
    assert!(ensure_fresh_transition_present(&session.inner).is_err());
    assert!(session
        .begin_synchronous_operation(FRESH_OPERATION_ACKNOWLEDGE)
        .is_err());

    let replacement_payload = "replacement panic".to_owned();
    let _ = fault_experimental_fresh_run(&session.inner, "different context", &replacement_payload);
    let retained = fresh_run_scalar_snapshot(&lock_recover(&session.inner), FRESH_OPERATION_IDLE)
        .expect("retained first fault")
        .fault_detail
        .expect("retained first detail");
    assert_eq!(retained, first_detail);
}

#[cfg(feature = "engine-test-hooks")]
#[test]
fn test_hook_epoch_requires_canonical_positive_lowercase_hex() {
    assert_eq!(
        parse_test_hook_epoch("0000000000000001").expect("one is canonical"),
        1
    );
    assert_eq!(
        parse_test_hook_epoch("ffffffffffffffff").expect("u64 max is canonical"),
        u64::MAX
    );
    for invalid in [
        "0000000000000000",
        "1",
        "000000000000000A",
        "-000000000000001",
        "gggggggggggggggg",
    ] {
        assert!(
            parse_test_hook_epoch(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn numeric_limits_reject_fractional_nonfinite_zero_and_oversized_values() {
    for value in [0.0, -1.0, 1.5, f64::NAN, f64::INFINITY] {
        assert!(positive_usize(value, "limit").is_err());
    }
    assert_eq!(positive_usize(1.0, "limit").expect("one is valid"), 1);
    assert_eq!(
        positive_usize(f64::from(u32::MAX), "limit").expect("u32 max is valid"),
        usize::try_from(u32::MAX).expect("supported targets represent u32")
    );
    assert!(positive_usize(f64::from(u32::MAX) + 1.0, "limit").is_err());
}

#[test]
fn drain_guard_enforces_one_consumer_and_recovers_after_drop() {
    let active = AtomicBool::new(false);
    let first = DrainConsumerGuard::acquire(&active).expect("first consumer admitted");
    assert!(DrainConsumerGuard::acquire(&active).is_err());
    drop(first);
    assert!(DrainConsumerGuard::acquire(&active).is_ok());
}
