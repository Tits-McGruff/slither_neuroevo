//! Explicit experimental coarse N-API adapter for the Rust engine spine.
//!
//! This module is deliberately disconnected from normal server startup. It
//! moves bounded command batches and already-prepared events across N-API; it
//! never exposes per-snake, per-layer, or per-step subsystem calls.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use napi::bindgen_prelude::{Array, AsyncTask, BigInt, Function, Object, Task, Uint8Array};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, Error, JsString, Result, Status};
use napi_derive::napi;

use crate::engine::contract::{
    CommandBatch, CompletedEvent, EngineCommand, EngineInit, InboundLimits, OutputLimits,
    ReliableEvent, SequencedCommand, ENGINE_CONTRACT_VERSION,
};
use crate::engine::error::{EngineError, EngineErrorCode, MAX_ERROR_DETAIL_BYTES};
use crate::engine::queues::WakeSink;
use crate::engine::runtime::{EngineHealth, EngineRuntime};
use crate::engine::LifecycleState;

/// The one-slot, weak, nonblocking wake notification used by the bridge.
type WakeThreadsafeFunction = ThreadsafeFunction<(), (), (), Status, false, true, 1>;
/// Non-owning JavaScript BigInt input handle; unlike bindgen `BigInt`, this
/// queries a u64 directly without allocating every limb supplied by a caller.
#[allow(deprecated)]
type InputBigInt = napi::JsBigInt;

/// Maximum count admitted by this temporary N-API metadata bridge.
///
/// This is not an owner workload limit. At 65,536 entries, the temporary
/// parsed-command vector occupies at most four MiB on supported builds (the
/// exact check uses `size_of::<SequencedCommand>()`) before payload storage.
const MAX_NAPI_QUEUE_ENTRY_COUNT: usize = 65_536;
/// Maximum metadata bytes reserved for one parsed command batch.
const MAX_NAPI_BATCH_METADATA_BYTES: usize = 4 * 1024 * 1024;
/// Maximum bytes copied for one temporary probe event.
const MAX_NAPI_EVENT_OWNED_BYTES: usize = 64 * 1024 * 1024;
/// Maximum payload bytes copied for one temporary atomic probe batch.
const MAX_NAPI_BATCH_OWNED_BYTES: usize = 256 * 1024 * 1024;
/// Maximum payload bytes admitted across one temporary queue or output class.
const MAX_NAPI_QUEUE_OWNED_BYTES: usize = 512 * 1024 * 1024;
/// Maximum combined fixed command metadata and payload bytes for one batch.
const MAX_NAPI_BATCH_RESERVATION_BYTES: usize =
    MAX_NAPI_BATCH_METADATA_BYTES + MAX_NAPI_BATCH_OWNED_BYTES;

/// Return the exact experimental engine command/event contract version.
#[napi(js_name = "experimentalEngineContractVersion", catch_unwind)]
pub fn experimental_engine_contract_version() -> u32 {
    ENGINE_CONTRACT_VERSION
}

/// JavaScript initialization limits for the experimental engine.
///
/// The adapter additionally caps queue-entry metadata, individual payloads,
/// batch payloads, and total queued bytes. Those caps prevent a JavaScript
/// caller from turning a sparse array or nominal limit into an attempted
/// multi-gigabyte allocation; they are not simulation workload promises.
#[napi(object)]
pub struct ExperimentalEngineOptions {
    /// Must equal the exported Stage 3 engine contract version.
    pub contract_version: f64,
    /// Maximum queued command batches.
    pub max_inbound_batches: f64,
    /// Maximum queued commands across all batches.
    pub max_inbound_commands: f64,
    /// Maximum queued payload bytes across all batches.
    pub max_inbound_owned_bytes: f64,
    /// Maximum commands in one atomic batch.
    pub max_batch_commands: f64,
    /// Maximum payload bytes in one atomic batch.
    pub max_batch_owned_bytes: f64,
    /// Maximum queued reliable output events.
    pub max_output_reliable: f64,
    /// Maximum queued reliable output payload bytes.
    pub max_output_reliable_owned_bytes: f64,
    /// Maximum queued discrete output events.
    pub max_output_discrete: f64,
    /// Maximum queued discrete output payload bytes.
    pub max_output_discrete_owned_bytes: f64,
    /// Maximum bytes across all normal output classes.
    pub max_output_total_owned_bytes: f64,
    /// Maximum bytes owned by one output event.
    pub max_output_event_owned_bytes: f64,
    /// Maximum connections retaining replaceable frames.
    pub max_output_frame_connections: f64,
}

/// One prepared event returned by a bounded output drain.
#[napi(object)]
pub struct ExperimentalEngineEvent {
    /// Stable event kind.
    pub kind: String,
    /// Exact command or publication sequence when the event has one.
    pub sequence: Option<BigInt>,
    /// Exact probe correlation identifier when applicable.
    pub correlation_id: Option<BigInt>,
    /// Exact connection identifier for a frame event.
    pub connection_id: Option<BigInt>,
    /// Already-prepared event bytes when applicable.
    pub payload: Option<Uint8Array>,
    /// Stable fault category when this is a fault event.
    pub fault_code: Option<String>,
    /// Bounded fault diagnostic when this is a fault event.
    pub fault_detail: Option<String>,
}

/// Result of one bounded output drain.
#[napi(object)]
pub struct ExperimentalEngineDrain {
    /// Events removed by this call in engine priority order.
    pub events: Vec<ExperimentalEngineEvent>,
    /// Whether the same consumer should drain again without waiting for a wake.
    pub more_work: bool,
    /// Exact output-generation counter observed after the drain.
    pub generation: BigInt,
}

/// Small operational state that never copies authoritative world data.
#[napi(object)]
pub struct ExperimentalEngineHealth {
    /// One-shot lifecycle name.
    pub lifecycle: String,
    /// Current inbound batch depth.
    pub inbound_batches: BigInt,
    /// Current inbound command depth.
    pub inbound_commands: BigInt,
    /// Current inbound owned bytes.
    pub inbound_owned_bytes: BigInt,
    /// Highest observed inbound batch depth.
    pub inbound_high_water_batches: BigInt,
    /// Highest observed inbound command depth.
    pub inbound_high_water_commands: BigInt,
    /// Highest observed inbound payload bytes.
    pub inbound_high_water_owned_bytes: BigInt,
    /// Exact rejected-submission count.
    pub inbound_rejections: BigInt,
    /// Exact accepted batches discarded by the first terminal fault.
    pub inbound_fault_discarded_batches: BigInt,
    /// Exact accepted commands discarded by the first terminal fault.
    pub inbound_fault_discarded_commands: BigInt,
    /// Exact accepted payload bytes discarded by the first terminal fault.
    pub inbound_fault_discarded_owned_bytes: BigInt,
    /// Exact last admitted command sequence.
    pub inbound_last_accepted_sequence: Option<BigInt>,
    /// Whether the out-of-band stop flag is set.
    pub inbound_stop_requested: bool,
    /// Current reliable output count.
    pub output_reliable: BigInt,
    /// Current reliable output-owned bytes.
    pub output_reliable_owned_bytes: BigInt,
    /// Current discrete output count.
    pub output_discrete: BigInt,
    /// Current discrete output-owned bytes.
    pub output_discrete_owned_bytes: BigInt,
    /// Whether one replaceable stats item is retained.
    pub output_has_stats: bool,
    /// Current replaceable frame count.
    pub output_frames: BigInt,
    /// Current total normal output-owned bytes.
    pub output_owned_bytes: BigInt,
    /// Highest observed normal output event count.
    pub output_high_water_count: BigInt,
    /// Highest observed normal output-owned bytes.
    pub output_high_water_owned_bytes: BigInt,
    /// Exact reliable/discrete output overflow attempts.
    pub output_priority_overflows: BigInt,
    /// Exact stats replacement count.
    pub output_stats_replacements: BigInt,
    /// Exact frame replacement count.
    pub output_frame_replacements: BigInt,
    /// Exact stale stats publication count.
    pub output_stale_stats: BigInt,
    /// Exact stale frame publication count.
    pub output_stale_frames: BigInt,
    /// Exact stats rejection count.
    pub output_stats_rejections: BigInt,
    /// Exact frame rejection count.
    pub output_frame_rejections: BigInt,
    /// Exact stats eviction count.
    pub output_stats_evictions: BigInt,
    /// Exact frame eviction count.
    pub output_frame_evictions: BigInt,
    /// Whether the out-of-capacity first-fault slot is occupied.
    pub output_has_reserved_fault: bool,
    /// Exact processed-batch count.
    pub processed_batches: BigInt,
    /// Exact processed-command count.
    pub processed_commands: BigInt,
    /// Exact output generation.
    pub wake_generation: BigInt,
    /// Exact attempted-wake count.
    pub wake_attempts: BigInt,
    /// Exact accepted-wake count.
    pub wake_notifications: BigInt,
    /// Exact failed-wake count.
    pub wake_failures: BigInt,
    /// Exact wake re-arm race count.
    pub wake_rearm_races: BigInt,
    /// Whether one coalesced wake is currently outstanding.
    pub wake_pending: bool,
    /// First terminal fault category, when present.
    pub fault_code: Option<String>,
    /// First bounded terminal fault detail, when present.
    pub fault_detail: Option<String>,
}

/// Explicit, non-production adapter around one Rust-owned engine runtime.
#[napi]
pub struct ExperimentalRustEngine {
    runtime: Arc<EngineRuntime>,
    max_batch_commands: usize,
    max_batch_owned_bytes: usize,
    max_event_owned_bytes: usize,
    drain_active: AtomicBool,
    join_scheduled: Arc<AtomicBool>,
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
    fn run_faulting_root<T>(
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
    runtime: Arc<EngineRuntime>,
    join_scheduled: Arc<AtomicBool>,
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
struct NapiWakeSink {
    tsfn: WakeThreadsafeFunction,
    runtime: Mutex<Weak<EngineRuntime>>,
}

impl NapiWakeSink {
    fn new(tsfn: WakeThreadsafeFunction) -> Self {
        Self {
            tsfn,
            runtime: Mutex::new(Weak::new()),
        }
    }

    fn attach(&self, runtime: &Arc<EngineRuntime>) {
        *lock_recover(&self.runtime) = Arc::downgrade(runtime);
    }

    fn runtime(&self) -> Weak<EngineRuntime> {
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

fn apply_wake_status(
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

fn wake_status_to_result(status: Status) -> std::result::Result<(), EngineError> {
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

struct DrainConsumerGuard<'a> {
    active: &'a AtomicBool,
}

impl<'a> DrainConsumerGuard<'a> {
    fn acquire(active: &'a AtomicBool) -> std::result::Result<Self, EngineError> {
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

fn options_to_init(
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

fn validate_bridge_safety_limits(init: &EngineInit) -> std::result::Result<(), EngineError> {
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

fn validate_batch_reservation(
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

fn command_metadata_bytes(command_capacity: usize) -> std::result::Result<usize, EngineError> {
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

fn validate_payload_before_copy(
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

fn validate_probe_payload_len(payload_len: usize) -> std::result::Result<(), EngineError> {
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

fn copy_probe_payload(payload: &[u8], index: u32) -> std::result::Result<Vec<u8>, EngineError> {
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

fn positive_u32(value: f64, field: &str) -> std::result::Result<u32, EngineError> {
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

fn positive_usize(value: f64, field: &str) -> std::result::Result<usize, EngineError> {
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

fn bounded_error_detail(detail: JsString<'_>) -> Result<String> {
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
fn exact_u64(value: &InputBigInt, field: &str) -> std::result::Result<u64, EngineError> {
    let (value, lossless) = value.get_u64().map_err(napi_error_to_engine)?;
    require_lossless_u64(value, lossless, field)
}

fn require_lossless_u64(
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

fn required_property<T: napi::bindgen_prelude::FromNapiValue>(
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

fn event_to_napi(event: CompletedEvent) -> ExperimentalEngineEvent {
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

fn health_to_napi(
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

fn usize_bigint(value: usize, field: &str) -> std::result::Result<BigInt, EngineError> {
    u64::try_from(value).map(BigInt::from).map_err(|_| {
        EngineError::new(
            EngineErrorCode::Faulted,
            format!("{field} exceeds the exact unsigned 64-bit health representation"),
        )
    })
}

fn lifecycle_name(lifecycle: LifecycleState) -> &'static str {
    match lifecycle {
        LifecycleState::Created => "created",
        LifecycleState::Running => "running",
        LifecycleState::StopRequested => "stopRequested",
        LifecycleState::Faulted => "faulted",
        LifecycleState::Stopped => "stopped",
    }
}

fn error_code_name(code: EngineErrorCode) -> &'static str {
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

fn engine_error_to_napi(error: EngineError) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("{}: {}", error_code_name(error.code), error.detail),
    )
}

fn napi_error_to_engine(error: Error) -> EngineError {
    EngineError::new(
        EngineErrorCode::InvalidCommand,
        format!("invalid JavaScript probe command: {error}"),
    )
}

fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::queues::WakeSink;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

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
}
