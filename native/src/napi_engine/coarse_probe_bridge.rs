//! Coarse probe bridge for the coarse N-API boundary.

use super::*;

/// Explicit, non-production adapter around one Rust-owned engine runtime.
#[napi]
pub struct ExperimentalRustEngine {
    pub(super) runtime: Arc<EngineRuntime>,
    pub(super) max_batch_commands: usize,
    pub(super) max_batch_owned_bytes: usize,
    pub(super) max_event_owned_bytes: usize,
    pub(super) drain_active: AtomicBool,
    pub(super) join_scheduled: Arc<AtomicBool>,
}

#[napi]
impl ExperimentalRustEngine {
    /// Create an unstarted engine and a weak one-slot JavaScript wake bridge.
    #[napi(constructor, catch_unwind)]
    pub fn new(
        options: ExperimentalEngineOptions,
        wake_callback: Function<'_, (), ()>,
    ) -> Result<Self> {
        let init = options_to_init(options).map_err(engine_error_to_napi)?;
        let wake_tsfn = wake_callback
            .build_threadsafe_function::<()>()
            .callee_handled::<false>()
            .weak::<true>()
            .max_queue_size::<1>()
            .build_callback(|_context| Ok(()))?;
        let wake_sink = Arc::new(NapiWakeSink::new(wake_tsfn));
        let runtime = Arc::new(
            EngineRuntime::new_experimental_probe(
                init,
                Arc::clone(&wake_sink) as Arc<dyn WakeSink>,
            )
            .map_err(engine_error_to_napi)?,
        );
        wake_sink.attach(&runtime);
        Ok(Self {
            runtime,
            max_batch_commands: init.inbound.max_batch_commands,
            max_batch_owned_bytes: init.inbound.max_batch_owned_bytes,
            max_event_owned_bytes: init.output.max_event_owned_bytes,
            drain_active: AtomicBool::new(false),
            join_scheduled: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Start the one background coordinator exactly once.
    #[napi(catch_unwind)]
    pub fn start(&self) -> Result<()> {
        self.run_faulting_root(|| self.runtime.start())
    }

    /// Submit one array of probe commands as an all-or-nothing queue operation.
    #[napi(catch_unwind)]
    pub fn submit_probe_batch(&self, commands: Array<'_>) -> Result<()> {
        self.run_faulting_root(|| {
            let command_count = usize::try_from(commands.len()).map_err(|_| {
                EngineError::new(
                    EngineErrorCode::InvalidCommand,
                    "probe command count cannot be represented by this build",
                )
            })?;
            if command_count == 0 {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidCommand,
                    "probe command batch must not be empty",
                ));
            }
            if command_count > self.max_batch_commands {
                return Err(EngineError::new(
                    EngineErrorCode::QueueCountLimit,
                    "probe command batch exceeds the configured command limit",
                ));
            }

            let mut parsed = Vec::new();
            parsed.try_reserve_exact(command_count).map_err(|error| {
                EngineError::new(
                    EngineErrorCode::QueueCountLimit,
                    format!("failed to reserve bounded probe command metadata: {error}"),
                )
            })?;
            validate_batch_reservation(parsed.capacity(), 0)?;
            let mut owned_bytes = 0usize;
            for index in 0..commands.len() {
                let object = commands
                    .get::<Object<'_>>(index)
                    .map_err(napi_error_to_engine)?
                    .ok_or_else(|| {
                        EngineError::new(
                            EngineErrorCode::InvalidCommand,
                            format!("missing probe command at index {index}"),
                        )
                    })?;
                let sequence = required_property::<InputBigInt>(&object, "sequence", index)?;
                let correlation_id =
                    required_property::<InputBigInt>(&object, "correlationId", index)?;
                let payload = required_property::<Uint8Array>(&object, "payload", index)?;
                let payload_len = payload.len();
                validate_probe_payload_len(payload_len)?;
                if payload_len > self.max_event_owned_bytes {
                    return Err(EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        format!("probe payload at index {index} exceeds the event byte limit"),
                    ));
                }
                validate_payload_before_copy(
                    payload_len,
                    owned_bytes,
                    self.max_batch_owned_bytes,
                    parsed.capacity(),
                    index,
                )?;
                let payload = copy_probe_payload(payload.as_ref(), index)?;
                if payload.capacity() > self.max_event_owned_bytes {
                    return Err(EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        format!(
                            "probe payload allocation at index {index} exceeds the event byte limit"
                        ),
                    ));
                }
                owned_bytes = owned_bytes.checked_add(payload.capacity()).ok_or_else(|| {
                    EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "probe batch byte accounting overflowed",
                    )
                })?;
                if owned_bytes > self.max_batch_owned_bytes {
                    return Err(EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "probe command batch exceeds the configured byte limit",
                    ));
                }
                validate_batch_reservation(parsed.capacity(), owned_bytes)?;
                parsed.push(SequencedCommand {
                    sequence: exact_u64(&sequence, "sequence")?,
                    command: EngineCommand::Probe {
                        correlation_id: exact_u64(&correlation_id, "correlationId")?,
                        payload,
                    },
                });
            }

            self.runtime.try_submit(CommandBatch {
                contract_version: ENGINE_CONTRACT_VERSION,
                commands: parsed.into_boxed_slice(),
            })
        })
    }

    /// Drain prepared output from the adapter's sole consumer surface.
    #[napi(catch_unwind)]
    pub fn drain_outputs(
        &self,
        max_events: f64,
        max_owned_bytes: f64,
    ) -> Result<ExperimentalEngineDrain> {
        self.run_faulting_root(|| {
            let max_events = positive_usize(max_events, "maxEvents")?;
            let max_owned_bytes = positive_usize(max_owned_bytes, "maxOwnedBytes")?;
            let _guard = DrainConsumerGuard::acquire(&self.drain_active)?;
            let drained = self.runtime.drain_outputs(max_events, max_owned_bytes)?;
            let events = drained
                .events
                .into_iter()
                .map(event_to_napi)
                .collect::<Vec<_>>();
            Ok(ExperimentalEngineDrain {
                events,
                more_work: drained.more_work,
                generation: BigInt::from(drained.generation),
            })
        })
    }

    /// Return bounded queue, wake, lifecycle, and fault state.
    #[napi(catch_unwind)]
    pub fn health(&self) -> Result<ExperimentalEngineHealth> {
        self.run_faulting_root(|| health_to_napi(self.runtime.health()))
    }

    /// Make a TypeScript-side drain or handler failure terminal in Rust.
    #[napi(catch_unwind)]
    pub fn report_bridge_fault(&self, detail: JsString<'_>) -> Result<()> {
        let detail = bounded_error_detail(detail)?;
        self.run_faulting_root(|| {
            self.runtime
                .report_bridge_fault(EngineError::new(EngineErrorCode::WakeDelivery, detail));
            Ok(())
        })
    }

    /// Request an orderly stop without waiting for the coordinator.
    #[napi(catch_unwind)]
    pub fn request_stop(&self) -> Result<()> {
        self.run_faulting_root(|| {
            self.runtime.request_stop();
            Ok(())
        })
    }

    /// Stop and join on libuv's worker pool, never on the Node event loop.
    #[napi(catch_unwind)]
    pub fn join(&self) -> Result<AsyncTask<JoinEngineTask>> {
        self.run_faulting_root(|| {
            if self.join_scheduled.swap(true, Ordering::AcqRel) {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidLifecycle,
                    "an engine join is already scheduled",
                ));
            }
            Ok(AsyncTask::new(JoinEngineTask {
                runtime: Arc::clone(&self.runtime),
                join_scheduled: Arc::clone(&self.join_scheduled),
            }))
        })
    }
}

impl ExperimentalRustEngine {
    pub(super) fn run_faulting_root<T>(
        &self,
        operation: impl FnOnce() -> std::result::Result<T, EngineError>,
    ) -> Result<T> {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(result) => result.map_err(engine_error_to_napi),
            Err(payload) => {
                let detail = panic_detail(payload.as_ref());
                self.runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::Faulted,
                    format!("panic at experimental N-API root: {detail}"),
                ));
                Err(Error::new(
                    Status::GenericFailure,
                    format!("Faulted: panic at experimental N-API root: {detail}"),
                ))
            }
        }
    }
}

impl Drop for ExperimentalRustEngine {
    fn drop(&mut self) {
        let runtime = Arc::clone(&self.runtime);
        let stopped = catch_unwind(AssertUnwindSafe(|| runtime.request_stop()));
        if stopped.is_err() {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::Faulted,
                    "panic while finalizing experimental N-API engine",
                ));
            }));
        }
    }
}

/// Async work item that contains both the join and any panic it triggers.
pub struct JoinEngineTask {
    pub(super) runtime: Arc<EngineRuntime>,
    pub(super) join_scheduled: Arc<AtomicBool>,
}

impl JoinEngineTask {
    pub(crate) fn new(runtime: Arc<EngineRuntime>, join_scheduled: Arc<AtomicBool>) -> Self {
        Self {
            runtime,
            join_scheduled,
        }
    }
}

impl Task for JoinEngineTask {
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| self.runtime.join())) {
            Ok(result) => result.map_err(engine_error_to_napi),
            Err(payload) => {
                let detail = panic_detail(payload.as_ref());
                self.runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::ThreadJoin,
                    format!("panic while joining engine coordinator: {detail}"),
                ));
                Err(Error::new(
                    Status::GenericFailure,
                    format!("ThreadJoin: panic while joining engine coordinator: {detail}"),
                ))
            }
        }
    }

    fn resolve(&mut self, _env: Env, _output: Self::Output) -> Result<Self::JsValue> {
        Ok(())
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.join_scheduled.store(false, Ordering::Release);
        Ok(())
    }
}

/// Wake adapter whose JavaScript callback cannot keep Node alive.
pub(super) struct NapiWakeSink {
    pub(super) tsfn: WakeThreadsafeFunction,
    pub(super) runtime: Mutex<Weak<EngineRuntime>>,
}

impl NapiWakeSink {
    pub(super) fn new(tsfn: WakeThreadsafeFunction) -> Self {
        Self {
            tsfn,
            runtime: Mutex::new(Weak::new()),
        }
    }

    pub(super) fn attach(&self, runtime: &Arc<EngineRuntime>) {
        *lock_recover(&self.runtime) = Arc::downgrade(runtime);
    }

    pub(super) fn runtime(&self) -> Weak<EngineRuntime> {
        lock_recover(&self.runtime).clone()
    }
}

impl WakeSink for NapiWakeSink {
    fn notify(&self) -> std::result::Result<(), EngineError> {
        let runtime = self.runtime();
        let callback_runtime = runtime.clone();
        // OutputQueue normally issues only one notification until the sole
        // consumer drains/re-arms it, so QueueFull is exceptional. napi-rs
        // 3.10.5 boxes call data before invoking N-API and does not visibly
        // reclaim that allocation on QueueFull/Closing; avoid retry loops here.
        let status = self.tsfn.call_with_return_value(
            (),
            ThreadsafeFunctionCallMode::NonBlocking,
            move |callback_result, _env| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    if let Err(error) = callback_result {
                        if let Some(runtime) = callback_runtime.upgrade() {
                            runtime.report_bridge_fault(EngineError::new(
                                EngineErrorCode::WakeDelivery,
                                format!("JavaScript wake callback failed: {error}"),
                            ));
                        }
                    }
                }));
                // Returning an error here would route the JS exception through
                // napi_fatal_exception. The engine fault above is the safe path.
                Ok(())
            },
        );
        apply_wake_status(status, &runtime)
    }
}

pub(super) fn apply_wake_status(
    status: Status,
    runtime: &Weak<EngineRuntime>,
) -> std::result::Result<(), EngineError> {
    let result = wake_status_to_result(status);
    if let Err(error) = &result {
        if let Some(runtime) = runtime.upgrade() {
            // OutputQueue has already marked this wake as outstanding. Fault
            // publication therefore cannot recursively schedule another wake;
            // the reserved first-fault slot also makes repeated reports inert.
            runtime.report_bridge_fault(error.clone());
        }
    }
    result
}

pub(super) fn wake_status_to_result(status: Status) -> std::result::Result<(), EngineError> {
    match status {
        Status::Ok | Status::QueueFull => Ok(()),
        Status::Closing => Err(EngineError::new(
            EngineErrorCode::WakeDelivery,
            "JavaScript wake bridge is closing",
        )),
        other => Err(EngineError::new(
            EngineErrorCode::WakeDelivery,
            format!("JavaScript wake scheduling failed with {other}"),
        )),
    }
}

pub(super) struct DrainConsumerGuard<'a> {
    pub(super) active: &'a AtomicBool,
}

impl<'a> DrainConsumerGuard<'a> {
    pub(super) fn acquire(active: &'a AtomicBool) -> std::result::Result<Self, EngineError> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                EngineError::new(
                    EngineErrorCode::InvalidLifecycle,
                    "output drain already has an active consumer",
                )
            })?;
        Ok(Self { active })
    }
}

impl Drop for DrainConsumerGuard<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

pub(super) fn options_to_init(
    options: ExperimentalEngineOptions,
) -> std::result::Result<EngineInit, EngineError> {
    let contract_version = positive_u32(options.contract_version, "contractVersion")?;
    let init = EngineInit {
        contract_version,
        inbound: InboundLimits {
            max_batches: positive_usize(options.max_inbound_batches, "maxInboundBatches")?,
            max_commands: positive_usize(options.max_inbound_commands, "maxInboundCommands")?,
            max_owned_bytes: positive_usize(
                options.max_inbound_owned_bytes,
                "maxInboundOwnedBytes",
            )?,
            max_batch_commands: positive_usize(options.max_batch_commands, "maxBatchCommands")?,
            max_batch_owned_bytes: positive_usize(
                options.max_batch_owned_bytes,
                "maxBatchOwnedBytes",
            )?,
        },
        output: OutputLimits {
            max_reliable: positive_usize(options.max_output_reliable, "maxOutputReliable")?,
            max_reliable_owned_bytes: positive_usize(
                options.max_output_reliable_owned_bytes,
                "maxOutputReliableOwnedBytes",
            )?,
            max_discrete: positive_usize(options.max_output_discrete, "maxOutputDiscrete")?,
            max_discrete_owned_bytes: positive_usize(
                options.max_output_discrete_owned_bytes,
                "maxOutputDiscreteOwnedBytes",
            )?,
            max_total_owned_bytes: positive_usize(
                options.max_output_total_owned_bytes,
                "maxOutputTotalOwnedBytes",
            )?,
            max_event_owned_bytes: positive_usize(
                options.max_output_event_owned_bytes,
                "maxOutputEventOwnedBytes",
            )?,
            max_frame_connections: positive_usize(
                options.max_output_frame_connections,
                "maxOutputFrameConnections",
            )?,
        },
    };
    validate_bridge_safety_limits(&init)?;
    init.validate()?;
    Ok(init)
}

pub(super) fn validate_bridge_safety_limits(
    init: &EngineInit,
) -> std::result::Result<(), EngineError> {
    let count_limits = [
        ("maxInboundBatches", init.inbound.max_batches),
        ("maxInboundCommands", init.inbound.max_commands),
        ("maxBatchCommands", init.inbound.max_batch_commands),
        ("maxOutputReliable", init.output.max_reliable),
        ("maxOutputDiscrete", init.output.max_discrete),
        (
            "maxOutputFrameConnections",
            init.output.max_frame_connections,
        ),
    ];
    for (field, value) in count_limits {
        if value > MAX_NAPI_QUEUE_ENTRY_COUNT {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                format!(
                    "{field} exceeds the experimental N-API metadata safety ceiling of {MAX_NAPI_QUEUE_ENTRY_COUNT} entries"
                ),
            ));
        }
    }

    let byte_limits = [
        (
            "maxInboundOwnedBytes",
            init.inbound.max_owned_bytes,
            MAX_NAPI_QUEUE_OWNED_BYTES,
        ),
        (
            "maxBatchOwnedBytes",
            init.inbound.max_batch_owned_bytes,
            MAX_NAPI_BATCH_OWNED_BYTES,
        ),
        (
            "maxOutputReliableOwnedBytes",
            init.output.max_reliable_owned_bytes,
            MAX_NAPI_QUEUE_OWNED_BYTES,
        ),
        (
            "maxOutputDiscreteOwnedBytes",
            init.output.max_discrete_owned_bytes,
            MAX_NAPI_QUEUE_OWNED_BYTES,
        ),
        (
            "maxOutputTotalOwnedBytes",
            init.output.max_total_owned_bytes,
            MAX_NAPI_QUEUE_OWNED_BYTES,
        ),
        (
            "maxOutputEventOwnedBytes",
            init.output.max_event_owned_bytes,
            MAX_NAPI_EVENT_OWNED_BYTES,
        ),
    ];
    for (field, value, ceiling) in byte_limits {
        if value > ceiling {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                format!(
                    "{field} exceeds the experimental N-API allocation safety ceiling of {ceiling} bytes"
                ),
            ));
        }
    }

    validate_batch_reservation(
        init.inbound.max_batch_commands,
        init.inbound.max_batch_owned_bytes,
    )
}

pub(super) fn validate_batch_reservation(
    command_capacity: usize,
    owned_bytes: usize,
) -> std::result::Result<(), EngineError> {
    let command_metadata_bytes = command_metadata_bytes(command_capacity)?;
    let batch_reservation_bytes =
        command_metadata_bytes
            .checked_add(owned_bytes)
            .ok_or_else(|| {
                EngineError::new(
                    EngineErrorCode::InvalidConfiguration,
                    "experimental N-API batch reservation accounting overflowed",
                )
            })?;
    if batch_reservation_bytes > MAX_NAPI_BATCH_RESERVATION_BYTES {
        return Err(EngineError::new(
            EngineErrorCode::InvalidConfiguration,
            format!(
                "command metadata plus owned payload require {batch_reservation_bytes} bytes; the experimental N-API combined ceiling is {MAX_NAPI_BATCH_RESERVATION_BYTES}"
            ),
        ));
    }
    Ok(())
}

pub(super) fn command_metadata_bytes(
    command_capacity: usize,
) -> std::result::Result<usize, EngineError> {
    let bytes = command_capacity
        .checked_mul(std::mem::size_of::<SequencedCommand>())
        .ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "experimental N-API command metadata accounting overflowed",
            )
        })?;
    if bytes > MAX_NAPI_BATCH_METADATA_BYTES {
        return Err(EngineError::new(
            EngineErrorCode::InvalidConfiguration,
            format!(
                "command capacity requires {bytes} metadata bytes; the experimental N-API ceiling is {MAX_NAPI_BATCH_METADATA_BYTES}"
            ),
        ));
    }
    Ok(bytes)
}

pub(super) fn validate_payload_before_copy(
    logical_bytes: usize,
    already_owned_bytes: usize,
    configured_batch_bytes: usize,
    command_capacity: usize,
    index: u32,
) -> std::result::Result<(), EngineError> {
    let configured_remaining = configured_batch_bytes
        .checked_sub(already_owned_bytes)
        .ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "probe batch already exceeds its configured byte limit",
            )
        })?;
    if logical_bytes > configured_remaining {
        return Err(EngineError::new(
            EngineErrorCode::QueueByteLimit,
            format!(
                "probe payload at index {index} exceeds the {configured_remaining} configured batch bytes remaining"
            ),
        ));
    }

    let current_temporary_bytes = command_metadata_bytes(command_capacity)?
        .checked_add(already_owned_bytes)
        .ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "temporary probe batch byte accounting overflowed",
            )
        })?;
    let hard_remaining = MAX_NAPI_BATCH_RESERVATION_BYTES
        .checked_sub(current_temporary_bytes)
        .ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "temporary probe batch already exceeds the hard allocation allowance",
            )
        })?;
    if logical_bytes > hard_remaining {
        return Err(EngineError::new(
            EngineErrorCode::QueueByteLimit,
            format!(
                "probe payload at index {index} exceeds the {hard_remaining} hard temporary bytes remaining"
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_probe_payload_len(
    payload_len: usize,
) -> std::result::Result<(), EngineError> {
    if payload_len > MAX_NAPI_EVENT_OWNED_BYTES {
        return Err(EngineError::new(
            EngineErrorCode::QueueByteLimit,
            format!(
                "probe payload exceeds the experimental N-API allocation safety ceiling of {MAX_NAPI_EVENT_OWNED_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

pub(super) fn copy_probe_payload(
    payload: &[u8],
    index: u32,
) -> std::result::Result<Vec<u8>, EngineError> {
    validate_probe_payload_len(payload.len())?;
    let mut owned = Vec::new();
    owned.try_reserve_exact(payload.len()).map_err(|error| {
        EngineError::new(
            EngineErrorCode::QueueByteLimit,
            format!("failed to reserve bounded probe payload at index {index}: {error}"),
        )
    })?;
    owned.extend_from_slice(payload);
    Ok(owned)
}

pub(super) fn positive_u32(value: f64, field: &str) -> std::result::Result<u32, EngineError> {
    if !value.is_finite() || value.fract() != 0.0 || value < 1.0 || value > f64::from(u32::MAX) {
        return Err(EngineError::new(
            EngineErrorCode::InvalidConfiguration,
            format!(
                "{field} must be a positive integer no greater than {}",
                u32::MAX
            ),
        ));
    }
    Ok(value as u32)
}

pub(crate) fn positive_usize(value: f64, field: &str) -> std::result::Result<usize, EngineError> {
    if !value.is_finite() || value.fract() != 0.0 || value < 1.0 || value > f64::from(u32::MAX) {
        return Err(EngineError::new(
            EngineErrorCode::InvalidConfiguration,
            format!(
                "{field} must be a positive integer no greater than {}",
                u32::MAX
            ),
        ));
    }
    usize::try_from(value as u32).map_err(|_| {
        EngineError::new(
            EngineErrorCode::InvalidConfiguration,
            format!("{field} cannot be represented by this build"),
        )
    })
}

pub(super) fn bounded_error_detail(detail: JsString<'_>) -> Result<String> {
    let utf8_len = detail.utf8_len()?;
    if utf8_len > MAX_ERROR_DETAIL_BYTES {
        return Err(Error::new(
            Status::InvalidArg,
            format!(
                "bridge fault detail is {utf8_len} UTF-8 bytes; the limit is {MAX_ERROR_DETAIL_BYTES}"
            ),
        ));
    }
    let utf8 = detail.into_utf8()?;
    Ok(utf8.as_str()?.to_owned())
}

#[allow(deprecated)]
pub(super) fn exact_u64(value: &InputBigInt, field: &str) -> std::result::Result<u64, EngineError> {
    let (value, lossless) = value.get_u64().map_err(napi_error_to_engine)?;
    require_lossless_u64(value, lossless, field)
}

pub(super) fn require_lossless_u64(
    value: u64,
    lossless: bool,
    field: &str,
) -> std::result::Result<u64, EngineError> {
    if !lossless {
        return Err(EngineError::new(
            EngineErrorCode::InvalidCommand,
            format!("{field} must be a non-negative, lossless unsigned 64-bit BigInt"),
        ));
    }
    Ok(value)
}

pub(super) fn required_property<T: napi::bindgen_prelude::FromNapiValue>(
    object: &Object<'_>,
    field: &str,
    index: u32,
) -> std::result::Result<T, EngineError> {
    object
        .get::<T>(field)
        .map_err(napi_error_to_engine)?
        .ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::InvalidCommand,
                format!("probe command at index {index} is missing {field}"),
            )
        })
}

pub(super) fn event_to_napi(event: CompletedEvent) -> ExperimentalEngineEvent {
    let mut output = ExperimentalEngineEvent {
        kind: String::new(),
        sequence: None,
        correlation_id: None,
        connection_id: None,
        payload: None,
        fault_code: None,
        fault_detail: None,
    };
    match event {
        CompletedEvent::Fault(fault) => {
            output.kind = "fault".to_owned();
            output.fault_code = Some(error_code_name(fault.code()).to_owned());
            output.fault_detail = Some(fault.detail().to_owned());
        }
        CompletedEvent::Reliable(ReliableEvent::Started) => {
            output.kind = "started".to_owned();
        }
        CompletedEvent::Reliable(ReliableEvent::Stopped) => {
            output.kind = "stopped".to_owned();
        }
        CompletedEvent::Reliable(ReliableEvent::ProbeResult {
            sequence,
            correlation_id,
            payload,
        }) => {
            output.kind = "probeResult".to_owned();
            output.sequence = Some(BigInt::from(sequence));
            output.correlation_id = Some(BigInt::from(correlation_id));
            output.payload = Some(Uint8Array::from(payload));
        }
        CompletedEvent::Reliable(ReliableEvent::RunningAuthority(_)) => {
            // The public experimental probe constructor cannot create the
            // retained-authority runtime mode. A dedicated typed adapter owns
            // these events; keep this generic converter exhaustive without
            // flattening authority metadata into the probe payload surface.
            output.kind = "runningAuthority".to_owned();
        }
        CompletedEvent::RunningDisplay(_) => {
            output.kind = "runningDisplay".to_owned();
        }
        CompletedEvent::Discrete(event) => {
            output.kind = "discrete".to_owned();
            output.sequence = Some(BigInt::from(event.sequence));
            output.payload = Some(Uint8Array::from(event.payload));
        }
        CompletedEvent::Stats(event) => {
            output.kind = "stats".to_owned();
            output.sequence = Some(BigInt::from(event.sequence));
            output.payload = Some(Uint8Array::from(event.payload));
        }
        CompletedEvent::Frame(event) => {
            output.kind = "frame".to_owned();
            output.sequence = Some(BigInt::from(event.sequence));
            output.connection_id = Some(BigInt::from(event.connection_id));
            output.payload = Some(Uint8Array::from(event.payload));
        }
    }
    output
}

pub(super) fn health_to_napi(
    health: EngineHealth,
) -> std::result::Result<ExperimentalEngineHealth, EngineError> {
    let fault_code = health
        .fault
        .as_ref()
        .map(|fault| error_code_name(fault.code()).to_owned());
    let fault_detail = health.fault.as_ref().map(|fault| fault.detail().to_owned());
    Ok(ExperimentalEngineHealth {
        lifecycle: lifecycle_name(health.lifecycle).to_owned(),
        inbound_batches: usize_bigint(health.inbound.batches, "inbound batch depth")?,
        inbound_commands: usize_bigint(health.inbound.commands, "inbound command depth")?,
        inbound_owned_bytes: usize_bigint(health.inbound.owned_bytes, "inbound owned bytes")?,
        inbound_high_water_batches: usize_bigint(
            health.inbound.high_water_batches,
            "inbound high-water batches",
        )?,
        inbound_high_water_commands: usize_bigint(
            health.inbound.high_water_commands,
            "inbound high-water commands",
        )?,
        inbound_high_water_owned_bytes: usize_bigint(
            health.inbound.high_water_owned_bytes,
            "inbound high-water owned bytes",
        )?,
        inbound_rejections: BigInt::from(health.inbound.rejections),
        inbound_fault_discarded_batches: BigInt::from(health.inbound.fault_discarded_batches),
        inbound_fault_discarded_commands: BigInt::from(health.inbound.fault_discarded_commands),
        inbound_fault_discarded_owned_bytes: BigInt::from(
            health.inbound.fault_discarded_owned_bytes,
        ),
        inbound_last_accepted_sequence: health.inbound.last_accepted_sequence.map(BigInt::from),
        inbound_stop_requested: health.inbound.stop_requested,
        output_reliable: usize_bigint(health.output.reliable, "reliable output depth")?,
        output_reliable_owned_bytes: usize_bigint(
            health.output.reliable_owned_bytes,
            "reliable output owned bytes",
        )?,
        output_discrete: usize_bigint(health.output.discrete, "discrete output depth")?,
        output_discrete_owned_bytes: usize_bigint(
            health.output.discrete_owned_bytes,
            "discrete output owned bytes",
        )?,
        output_has_stats: health.output.has_stats,
        output_frames: usize_bigint(health.output.frames, "frame output depth")?,
        output_owned_bytes: usize_bigint(health.output.total_owned_bytes, "output owned bytes")?,
        output_high_water_count: usize_bigint(
            health.output.high_water_count,
            "output high-water count",
        )?,
        output_high_water_owned_bytes: usize_bigint(
            health.output.high_water_owned_bytes,
            "output high-water owned bytes",
        )?,
        output_priority_overflows: BigInt::from(health.output.priority_overflows),
        output_stats_replacements: BigInt::from(health.output.stats_replacements),
        output_frame_replacements: BigInt::from(health.output.frame_replacements),
        output_stale_stats: BigInt::from(health.output.stale_stats),
        output_stale_frames: BigInt::from(health.output.stale_frames),
        output_stats_rejections: BigInt::from(health.output.stats_rejections),
        output_frame_rejections: BigInt::from(health.output.frame_rejections),
        output_stats_evictions: BigInt::from(health.output.stats_evictions),
        output_frame_evictions: BigInt::from(health.output.frame_evictions),
        output_has_reserved_fault: health.output.has_reserved_fault,
        processed_batches: BigInt::from(health.processed_batches),
        processed_commands: BigInt::from(health.processed_commands),
        wake_generation: BigInt::from(health.wake.generation),
        wake_attempts: BigInt::from(health.wake.notification_attempts),
        wake_notifications: BigInt::from(health.wake.notifications),
        wake_failures: BigInt::from(health.wake.notification_failures),
        wake_rearm_races: BigInt::from(health.wake.rearm_races),
        wake_pending: health.wake.notified,
        fault_code,
        fault_detail,
    })
}

pub(super) fn usize_bigint(value: usize, field: &str) -> std::result::Result<BigInt, EngineError> {
    u64::try_from(value).map(BigInt::from).map_err(|_| {
        EngineError::new(
            EngineErrorCode::Faulted,
            format!("{field} exceeds the exact unsigned 64-bit health representation"),
        )
    })
}

pub(super) fn lifecycle_name(lifecycle: LifecycleState) -> &'static str {
    match lifecycle {
        LifecycleState::Created => "created",
        LifecycleState::Running => "running",
        LifecycleState::StopRequested => "stopRequested",
        LifecycleState::Faulted => "faulted",
        LifecycleState::Stopped => "stopped",
    }
}

pub(super) fn error_code_name(code: EngineErrorCode) -> &'static str {
    match code {
        EngineErrorCode::InvalidConfiguration => "InvalidConfiguration",
        EngineErrorCode::InvalidLifecycle => "InvalidLifecycle",
        EngineErrorCode::InvalidCommand => "InvalidCommand",
        EngineErrorCode::SequenceRegression => "SequenceRegression",
        EngineErrorCode::QueueCountLimit => "QueueCountLimit",
        EngineErrorCode::QueueByteLimit => "QueueByteLimit",
        EngineErrorCode::ThreadSpawn => "ThreadSpawn",
        EngineErrorCode::ThreadJoin => "ThreadJoin",
        EngineErrorCode::WakeDelivery => "WakeDelivery",
        EngineErrorCode::Faulted => "Faulted",
    }
}

pub(crate) fn engine_error_to_napi(error: EngineError) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("{}: {}", error_code_name(error.code), error.detail),
    )
}

pub(super) fn napi_error_to_engine(error: Error) -> EngineError {
    EngineError::new(
        EngineErrorCode::InvalidCommand,
        format!("invalid JavaScript probe command: {error}"),
    )
}

pub(super) fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

pub(super) fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
