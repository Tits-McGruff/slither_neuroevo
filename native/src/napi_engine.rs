//! Explicit experimental coarse N-API adapter for the Rust engine spine.
//!
//! This module is deliberately disconnected from normal server startup. It
//! moves bounded command batches and already-prepared events across N-API; it
//! never exposes per-snake, per-layer, or per-step subsystem calls.

#![cfg_attr(
    all(test, feature = "engine-test-hooks"),
    allow(
        dead_code,
        reason = "feature-gated N-API exports are invoked by Node integration tests"
    )
)]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, Weak};

use napi::bindgen_prelude::{
    Array, AsyncTask, BigInt, Function, JsObjectValue, Object, Task, Uint8Array,
};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, Error, JsString, Result, Status};
use napi_derive::napi;

use crate::engine::checkpoint::{
    CheckpointBoundaryKind, CheckpointDescriptor, CheckpointOperationId,
    CheckpointWriteValidationPolicy, NumericEncoding, CHECKPOINT_DESCRIPTOR_VERSION,
};
use crate::engine::contract::{
    CommandBatch, CompletedEvent, EngineCommand, EngineInit, InboundLimits, OutputLimits,
    ReliableEvent, SequencedCommand, ENGINE_CONTRACT_VERSION,
};
use crate::engine::error::{truncate_utf8, EngineError, EngineErrorCode, MAX_ERROR_DETAIL_BYTES};
use crate::engine::frame_v1::FrameV1Metadata;
use crate::engine::fresh_run::{prepare_stage6a_p0_fresh_run, Stage6aP0FreshRunRequest};
use crate::engine::queues::WakeSink;
use crate::engine::run_start::PendingRunStartTransition;
use crate::engine::runtime::{EngineHealth, EngineRuntime};
use crate::engine::state::RunStartPublication;
use crate::engine::LifecycleState;
#[cfg(feature = "engine-test-hooks")]
use crate::engine::{
    checkpoint_fixture::publish_stage3_fixture,
    generation::GenerationCommitRecord,
    generation_handoff_fixture::{
        GenerationHandoffAssignment, GenerationHandoffFixtureSession, GenerationHandoffSnapshot,
        PublishedGenerationHandoff,
    },
    run_start_handoff_fixture::{RunStartHandoffFixtureSession, RunStartHandoffSnapshot},
};

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
/// Maximum UTF-8 bytes admitted for the opaque experimental lineage label.
const MAX_EXPERIMENTAL_RUN_ID_BYTES: usize = 256;
/// No asynchronous fresh-run operation is currently scheduled.
const FRESH_OPERATION_IDLE: u8 = 0;
/// Fixed-profile population construction is running off the Node event loop.
const FRESH_OPERATION_INITIALIZE: u8 = 1;
/// Immutable checkpoint publication is running off the Node event loop.
const FRESH_OPERATION_CHECKPOINT: u8 = 2;
/// Collision-safe world activation is running off the Node event loop.
const FRESH_OPERATION_ACTIVATE: u8 = 3;
/// Exact persistence acknowledgement owns the synchronous mutation root.
const FRESH_OPERATION_ACKNOWLEDGE: u8 = 4;
/// One neutral-view frame-v1 payload is being packed from running authority.
const FRESH_OPERATION_INITIAL_FRAME: u8 = 5;
/// One complete scheduled step and its resulting frame are being published.
const FRESH_OPERATION_FIRST_SCHEDULED_FRAME: u8 = 6;
/// Exact enumerable string keys admitted by a persistence acknowledgement.
const CHECKPOINT_DESCRIPTOR_INPUT_KEYS: [&str; 23] = [
    "protocolVersion",
    "operationId",
    "transitionEpoch",
    "runId",
    "generation",
    "completedStep",
    "boundaryKind",
    "checkpointFormatVersion",
    "stateVersion",
    "graphLayoutVersion",
    "managedRoot",
    "relativeFilename",
    "logicalRootSha256",
    "storedByteCount",
    "decodedByteCount",
    "roleCount",
    "populationCount",
    "weightCount",
    "recurrentStateCount",
    "weightsEncoding",
    "recurrentStateEncoding",
    "graphLayoutSha256",
    "writeValidationPolicy",
];

/// Test-hook-only request for one real managed checkpoint publication.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage3CheckpointFixtureOptions {
    /// Disposable server-controlled directory receiving the immutable file.
    pub managed_directory: String,
    /// Exact 32-digit lowercase hexadecimal operation identifier.
    pub operation_id: String,
    /// Exact positive transition epoch as 16 lowercase hexadecimal digits.
    pub transition_epoch: String,
}

/// Scalar-only descriptor returned by a real Rust checkpoint publisher.
#[napi(object)]
pub struct ManagedCheckpointDescriptor {
    /// Descriptor protocol version.
    pub protocol_version: u32,
    /// Exact operation identifier.
    pub operation_id: String,
    /// Exact transition epoch.
    pub transition_epoch: String,
    /// Exact run identity.
    pub run_id: String,
    /// Exact generation.
    pub generation: String,
    /// Exact completed fixed-step count.
    pub completed_step: String,
    /// Generation-boundary kind.
    pub boundary_kind: String,
    /// Managed checkpoint format version.
    pub checkpoint_format_version: String,
    /// Authoritative state contract version.
    pub state_version: String,
    /// Compiled graph-layout version.
    pub graph_layout_version: String,
    /// Fixed controlled managed-root label.
    pub managed_root: String,
    /// Digest-derived immutable filename.
    pub relative_filename: String,
    /// Encoding-independent logical root.
    pub logical_root_sha256: String,
    /// Complete stored archive bytes.
    pub stored_byte_count: String,
    /// Aggregate decoded logical bytes.
    pub decoded_byte_count: String,
    /// Logical role count.
    pub role_count: String,
    /// Dense population count.
    pub population_count: String,
    /// Packed population weight count.
    pub weight_count: String,
    /// Packed recurrent-state count.
    pub recurrent_state_count: String,
    /// Selected population-weight encoding.
    pub weights_encoding: String,
    /// Selected recurrent-state encoding.
    pub recurrent_state_encoding: String,
    /// Ordered graph-layout digest.
    pub graph_layout_sha256: String,
    /// Completed write-validation policy.
    pub write_validation_policy: String,
}

/// Controlled immutable-checkpoint publication request.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct ManagedCheckpointPublicationOptions {
    /// Disposable server-controlled directory receiving the immutable file.
    pub managed_directory: String,
    /// Exact 32-digit lowercase hexadecimal operation identifier.
    pub operation_id: String,
}

/// Exact Rust-owned eight-field generation summary wire record.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationSummaryRecord {
    pub completed_generation: String,
    pub best_f64_hex: String,
    pub average_f64_hex: String,
    pub minimum_f64_hex: String,
    pub species_count: String,
    pub top_species_size: String,
    pub average_weight_f64_hex: String,
    pub weight_variance_f64_hex: String,
}

/// Exact Rust-owned Hall-of-Fame successor reference wire record.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6HallOfFameRecord {
    pub completed_generation: String,
    pub source_population_slot: String,
    pub source_snake_id: String,
    pub fitness_f64_hex: String,
    pub points_f64_hex: String,
    pub length: String,
    pub successor_population_slot: String,
    pub successor_genome_id: String,
}

/// Complete scalar-only generation commit assembled by Rust admission.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationCommitRecord {
    pub summary: Stage6GenerationSummaryRecord,
    pub hall_of_fame: Stage6HallOfFameRecord,
}

/// Real retained generation publication returned to the persistence client.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationCheckpointPublication {
    pub descriptor: ManagedCheckpointDescriptor,
    pub generation_commit: Stage6GenerationCommitRecord,
}

/// Scalar proof of the fixture's current authority and retained barrier.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationHandoffSnapshot {
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub transition_pending: bool,
    pub checkpoint_published: bool,
    pub persistence_acknowledged: bool,
    pub generation_checkpoint_publications: u32,
    pub authority_publications: u32,
}

/// One reliable fresh-snake assignment generated by the retained Rust session.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationAssignment {
    pub operation_epoch: String,
    pub event_sequence: String,
    pub connection_id: String,
    pub lease_id: String,
    pub snake_id: String,
    pub resume_token: String,
}

/// Exact local-send result for the retained generation assignment.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationAssignmentResult {
    pub operation_epoch: String,
    pub event_sequence: String,
    pub connection_id: String,
    pub lease_id: String,
    pub accepted: bool,
}

/// Scalar result of the one final old-to-new authority swap.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6GenerationStartPublication {
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub population_epoch: String,
    pub external_assignments: u32,
}

/// Scalar proof of the fixture's staged or activated fresh run.
#[cfg(feature = "engine-test-hooks")]
#[napi(object)]
pub struct Stage6RunStartHandoffSnapshot {
    pub transition_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub checkpoint_published: bool,
    pub persistence_acknowledged: bool,
    pub authority_published: bool,
    pub snake_count: String,
    pub pellet_count: String,
    pub checkpoint_publications: u32,
    pub authority_publications: u32,
}

/// Scalar result of one durable-boundary-to-running activation.
#[napi(object)]
pub struct Stage6RunStartPublication {
    pub world_epoch: String,
    pub generation: String,
    pub completed_step: String,
    pub population_epoch: String,
}

/// One replaceable browser frame packed directly from retained Rust authority.
#[napi(object)]
pub struct ExperimentalFreshRunFrameV1 {
    /// Complete little-endian frame-v1 bytes; no population/archive bytes appear here.
    pub bytes: Uint8Array,
    /// Exact authoritative generation written into the frame.
    pub generation: String,
    /// Exact completed fixed-step count for the packed authority.
    pub completed_step: String,
    /// Exact number of authoritative snake records.
    pub total_snakes: String,
    /// Exact number of alive snake records present in the frame.
    pub alive_snakes: String,
    /// Exact number of pellet records present in the frame.
    pub pellets: String,
    /// Exact number of Float32 entries in the frame.
    pub float_length: String,
    /// Exact number of bytes in the frame.
    pub byte_length: String,
}

/// Rust-only frame payload and checked scalar metadata produced by a worker.
pub struct FreshRunFrameV1Output {
    bytes: Vec<u8>,
    generation: u64,
    completed_step: u64,
    total_snakes: u64,
    alive_snakes: u64,
    pellets: u64,
    float_length: u64,
    byte_length: u64,
}

/// Bounded scalar view of the experimental fixed-P0 fresh-run session.
#[napi(object)]
pub struct ExperimentalFreshRunSnapshot {
    /// Stable lifecycle phase; an active worker operation takes precedence.
    pub phase: String,
    /// Exact process-local transition token once construction has completed.
    pub transition_epoch: Option<String>,
    /// Exact current generation once construction has completed.
    pub generation: Option<String>,
    /// Exact completed-step count once construction has completed.
    pub completed_step: Option<String>,
    /// Whether the immutable managed file has published.
    pub checkpoint_published: Option<bool>,
    /// Whether Rust retained the worker's exact committed descriptor.
    pub persistence_acknowledged: Option<bool>,
    /// Whether the collision-safe running authority has published.
    pub authority_published: Option<bool>,
    /// Whether the one initial neutral-view frame has been packed.
    pub initial_frame_published: Option<bool>,
    /// Whether one complete Rust-scheduled step and its frame have published.
    pub first_scheduled_frame_published: Option<bool>,
    /// Exact current authoritative snake count once construction has completed.
    pub snake_count: Option<String>,
    /// Exact current authoritative pellet count once construction has completed.
    pub pellet_count: Option<String>,
    /// First bounded terminal panic detail, when the session is faulted.
    pub fault_detail: Option<String>,
}

/// Rust-only scalar snapshot used between a libuv worker and N-API resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshRunScalarSnapshot {
    phase: &'static str,
    transition_epoch: Option<u64>,
    generation: Option<u64>,
    completed_step: Option<u64>,
    checkpoint_published: Option<bool>,
    persistence_acknowledged: Option<bool>,
    authority_published: Option<bool>,
    initial_frame_published: Option<bool>,
    first_scheduled_frame_published: Option<bool>,
    snake_count: Option<usize>,
    pellet_count: Option<usize>,
    fault_detail: Option<String>,
}

/// Mutable Rust authority retained behind the experimental session mutex.
#[derive(Default)]
struct ExperimentalFreshRunInner {
    transition: Option<PendingRunStartTransition>,
    initial_frame_published: bool,
    first_scheduled_frame_published: bool,
    fault_detail: Option<String>,
}

/// Explicitly experimental owner for one real fixed-P0 fresh Rust lineage.
///
/// Construction, managed-file publication, and initial-world activation run on
/// libuv workers. The normal server does not instantiate this class yet.
#[napi(js_name = "ExperimentalStage6aFreshRunSession")]
pub struct ExperimentalStage6aFreshRunSession {
    request: Stage6aP0FreshRunRequest,
    inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    active_operation: Arc<AtomicU8>,
}

/// Unwind-safe ownership of one synchronous experimental mutation root.
struct FreshSynchronousOperation {
    active_operation: Arc<AtomicU8>,
}

impl Drop for FreshSynchronousOperation {
    fn drop(&mut self) {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
    }
}

#[napi]
impl ExperimentalStage6aFreshRunSession {
    /// Validate and retain only the fixed profile's bounded identity inputs.
    #[napi(constructor, catch_unwind)]
    pub fn new(
        run_id: JsString<'_>,
        seed_hex: JsString<'_>,
        memory_ceiling_bytes_hex: JsString<'_>,
    ) -> Result<Self> {
        let run_id = bounded_js_string(run_id, "runId", MAX_EXPERIMENTAL_RUN_ID_BYTES, false)?;
        if run_id.contains('\0') {
            return Err(Error::new(Status::InvalidArg, "runId must not contain NUL"));
        }
        let seed_hex = bounded_js_string(seed_hex, "seedHex", 8, false)?;
        let seed = parse_u32_hex(&seed_hex, "seedHex")?;
        let memory_ceiling_bytes_hex =
            bounded_js_string(memory_ceiling_bytes_hex, "memoryCeilingBytesHex", 16, false)?;
        let memory_ceiling_u64 =
            parse_u64_hex(&memory_ceiling_bytes_hex, "memoryCeilingBytesHex", false)?;
        let memory_ceiling_bytes = usize::try_from(memory_ceiling_u64).map_err(|_| {
            Error::new(
                Status::InvalidArg,
                "memoryCeilingBytesHex cannot be represented by this native target",
            )
        })?;
        Ok(Self {
            request: Stage6aP0FreshRunRequest {
                run_id,
                seed,
                memory_ceiling_bytes,
            },
            inner: Arc::new(Mutex::new(ExperimentalFreshRunInner::default())),
            active_operation: Arc::new(AtomicU8::new(FRESH_OPERATION_IDLE)),
        })
    }

    /// Construct and admit the complete fixed-P0 generation-one boundary off-loop.
    #[napi(catch_unwind)]
    pub fn initialize(&self) -> Result<AsyncTask<InitializeExperimentalFreshRunTask>> {
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        if let Err(error) = ensure_fresh_transition_absent(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(InitializeExperimentalFreshRunTask {
            request: self.request.clone(),
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
        }))
    }

    /// Publish or exactly retry the admitted managed checkpoint off-loop.
    #[napi(catch_unwind)]
    pub fn publish_run_start_checkpoint(
        &self,
        options: Object<'_>,
    ) -> Result<AsyncTask<PublishExperimentalFreshRunCheckpointTask>> {
        let (managed_directory, operation_id) =
            parse_managed_checkpoint_publication_options(&options)?;
        self.begin_operation(FRESH_OPERATION_CHECKPOINT)?;
        if let Err(error) = ensure_fresh_transition_present(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(PublishExperimentalFreshRunCheckpointTask {
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
            managed_directory,
            operation_id,
        }))
    }

    /// Retain only the persistence worker's exact complete descriptor echo.
    #[napi(catch_unwind)]
    pub fn acknowledge_run_start_persistence(&self, descriptor: Object<'_>) -> Result<()> {
        let _operation = self.begin_synchronous_operation(FRESH_OPERATION_ACKNOWLEDGE)?;
        match catch_unwind(AssertUnwindSafe(|| {
            let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
            let mut inner = try_lock_fresh_inner(&self.inner)?;
            reject_faulted_fresh_inner(&inner)?;
            let transition = inner.transition.as_mut().ok_or_else(|| {
                Error::new(
                    Status::GenericFailure,
                    "experimental fresh run has not been initialized",
                )
            })?;
            transition
                .acknowledge_persistence(&descriptor)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })) {
            Ok(result) => result,
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run persistence acknowledgement panicked",
                payload.as_ref(),
            )),
        }
    }

    /// Construct and publish the running authority off-loop after durability.
    #[napi(catch_unwind)]
    pub fn activate_running_authority(
        &self,
    ) -> Result<AsyncTask<ActivateExperimentalFreshRunTask>> {
        self.begin_operation(FRESH_OPERATION_ACTIVATE)?;
        if let Err(error) = ensure_fresh_transition_present(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(ActivateExperimentalFreshRunTask {
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
        }))
    }

    /// Pack exactly one bounded neutral-view frame from running Rust authority.
    #[napi(catch_unwind)]
    pub fn publish_initial_frame_v1(
        &self,
    ) -> Result<AsyncTask<PublishExperimentalFreshRunInitialFrameTask>> {
        self.begin_operation(FRESH_OPERATION_INITIAL_FRAME)?;
        if let Err(error) = ensure_fresh_transition_present(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(
            PublishExperimentalFreshRunInitialFrameTask {
                inner: Arc::clone(&self.inner),
                active_operation: Arc::clone(&self.active_operation),
            },
        ))
    }

    /// Execute the first Rust-scheduled fixed step and pack its resulting frame.
    #[napi(catch_unwind)]
    pub fn publish_first_scheduled_frame_v1(
        &self,
    ) -> Result<AsyncTask<PublishExperimentalFreshRunFirstScheduledFrameTask>> {
        self.begin_operation(FRESH_OPERATION_FIRST_SCHEDULED_FRAME)?;
        if let Err(error) = ensure_fresh_transition_present(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(
            PublishExperimentalFreshRunFirstScheduledFrameTask {
                inner: Arc::clone(&self.inner),
                active_operation: Arc::clone(&self.active_operation),
            },
        ))
    }

    /// Return bounded scalar proof without blocking the Node event loop.
    #[napi(catch_unwind)]
    pub fn snapshot(&self) -> Result<ExperimentalFreshRunSnapshot> {
        let active_operation = self.active_operation.load(Ordering::Acquire);
        match self.inner.try_lock() {
            Ok(inner) => fresh_run_scalar_snapshot(&inner, active_operation)
                .and_then(fresh_run_snapshot_to_napi),
            Err(TryLockError::Poisoned(poisoned)) => {
                fresh_run_scalar_snapshot(&poisoned.into_inner(), active_operation)
                    .and_then(fresh_run_snapshot_to_napi)
            }
            Err(TryLockError::WouldBlock) => {
                fresh_run_snapshot_to_napi(busy_fresh_run_snapshot(active_operation)?)
            }
        }
    }
}

impl ExperimentalStage6aFreshRunSession {
    /// Acquire the one asynchronous-operation slot without waiting.
    fn begin_operation(&self, operation: u8) -> Result<()> {
        if let Err(active) = self.active_operation.compare_exchange(
            FRESH_OPERATION_IDLE,
            operation,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            return Err(Error::new(
                Status::GenericFailure,
                format!(
                    "experimental fresh-run operation {} is already in flight",
                    fresh_operation_phase(active).unwrap_or("unknown")
                ),
            ));
        }
        if let Err(error) = reject_faulted_fresh_mutex(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    /// Reserve one synchronous root before inspecting caller-controlled objects.
    fn begin_synchronous_operation(&self, operation: u8) -> Result<FreshSynchronousOperation> {
        self.begin_operation(operation)?;
        Ok(FreshSynchronousOperation {
            active_operation: Arc::clone(&self.active_operation),
        })
    }
}

/// Async complete fixed-profile construction for one experimental lineage.
pub struct InitializeExperimentalFreshRunTask {
    request: Stage6aP0FreshRunRequest,
    inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    active_operation: Arc<AtomicU8>,
}

impl Task for InitializeExperimentalFreshRunTask {
    type Output = FreshRunScalarSnapshot;
    type JsValue = ExperimentalFreshRunSnapshot;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let transition = prepare_stage6a_p0_fresh_run(self.request.clone())
                .map_err(|error| error.to_string())?;
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            if inner.transition.is_some() {
                return Err("experimental fresh run is already initialized".to_owned());
            }
            inner.transition = Some(transition);
            fresh_run_scalar_snapshot(&inner, FRESH_OPERATION_IDLE)
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(snapshot)) => Ok(snapshot),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run construction panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        fresh_run_snapshot_to_napi(output)
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async immutable managed-file publication for the retained fresh boundary.
pub struct PublishExperimentalFreshRunCheckpointTask {
    inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    active_operation: Arc<AtomicU8>,
    managed_directory: PathBuf,
    operation_id: CheckpointOperationId,
}

impl Task for PublishExperimentalFreshRunCheckpointTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            inner
                .transition
                .as_mut()
                .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?
                .publish_checkpoint(&self.managed_directory, self.operation_id.clone())
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run checkpoint publication panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async collision-safe running-authority activation for the durable boundary.
pub struct ActivateExperimentalFreshRunTask {
    inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    active_operation: Arc<AtomicU8>,
}

impl Task for ActivateExperimentalFreshRunTask {
    type Output = RunStartPublication;
    type JsValue = Stage6RunStartPublication;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            inner
                .transition
                .as_mut()
                .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?
                .publish_running_authority()
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(publication)) => Ok(publication),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run activation panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(run_start_publication_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async one-shot frame-v1 packing from the retained running authority.
pub struct PublishExperimentalFreshRunInitialFrameTask {
    inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    active_operation: Arc<AtomicU8>,
}

impl Task for PublishExperimentalFreshRunInitialFrameTask {
    type Output = FreshRunFrameV1Output;
    type JsValue = ExperimentalFreshRunFrameV1;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            if inner.initial_frame_published {
                return Err(
                    "experimental fresh-run initial frame-v1 is already published".to_owned(),
                );
            }
            let transition = inner
                .transition
                .as_ref()
                .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?;
            let mut bytes = Vec::new();
            let metadata = transition
                .pack_initial_frame_v1(&mut bytes)
                .map_err(|error| error.to_string())?;
            let output = fresh_run_frame_v1_output(bytes, metadata, transition.completed_step())
                .map_err(|error| error.to_string())?;
            inner.initial_frame_published = true;
            Ok(output)
        })) {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run initial frame-v1 publication panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(fresh_run_frame_v1_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async one-shot complete scheduled step plus post-publication frame packing.
pub struct PublishExperimentalFreshRunFirstScheduledFrameTask {
    inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    active_operation: Arc<AtomicU8>,
}

impl Task for PublishExperimentalFreshRunFirstScheduledFrameTask {
    type Output = FreshRunFrameV1Output;
    type JsValue = ExperimentalFreshRunFrameV1;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            if !inner.initial_frame_published {
                return Err(
                    "experimental fresh-run first scheduled frame-v1 requires the initial frame"
                        .to_owned(),
                );
            }
            if inner.first_scheduled_frame_published {
                return Err(
                    "experimental fresh-run first scheduled frame-v1 is already published"
                        .to_owned(),
                );
            }

            let result = (|| {
                let transition = inner
                    .transition
                    .as_mut()
                    .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?;
                let mut bytes = Vec::new();
                let metadata = transition
                    .publish_first_scheduled_frame_v1(&mut bytes)
                    .map_err(|error| error.to_string())?;
                fresh_run_frame_v1_output(bytes, metadata, transition.completed_step())
                    .map_err(|error| error.to_string())
            })();

            match result {
                Ok(output) => {
                    inner.first_scheduled_frame_published = true;
                    Ok(output)
                }
                Err(detail) => {
                    let retained = retain_experimental_fresh_run_fault(
                        &mut inner,
                        "experimental fresh-run first scheduled frame-v1 failed",
                        &detail,
                    );
                    Err(retained)
                }
            }
        })) {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run first scheduled frame-v1 panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(fresh_run_frame_v1_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Return the exact experimental engine command/event contract version.
#[napi(js_name = "experimentalEngineContractVersion", catch_unwind)]
pub fn experimental_engine_contract_version() -> u32 {
    ENGINE_CONTRACT_VERSION
}

/// Publish one real deterministic checkpoint on libuv's worker pool.
///
/// This export exists only in an explicitly feature-gated test-hooks addon.
/// Population, graph, recurrent-state, and archive bytes remain entirely in Rust;
/// JavaScript receives only the scalar descriptor required by the metadata worker.
#[cfg(feature = "engine-test-hooks")]
#[napi(js_name = "publishStage3CheckpointFixture", catch_unwind)]
pub fn publish_stage3_checkpoint_fixture(
    options: Stage3CheckpointFixtureOptions,
) -> Result<AsyncTask<PublishStage3CheckpointFixtureTask>> {
    if options.managed_directory.is_empty() || options.managed_directory.contains('\0') {
        return Err(Error::new(
            Status::InvalidArg,
            "managedDirectory must be a nonempty path without NUL",
        ));
    }
    let transition_epoch = parse_test_hook_epoch(&options.transition_epoch)?;
    Ok(AsyncTask::new(PublishStage3CheckpointFixtureTask {
        managed_directory: PathBuf::from(options.managed_directory),
        operation_id: options.operation_id,
        transition_epoch,
    }))
}

/// Async work item owning all filesystem and codec work for the test hook.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage3CheckpointFixtureTask {
    managed_directory: PathBuf,
    operation_id: String,
    transition_epoch: u64,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage3CheckpointFixtureTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            publish_stage3_fixture(
                &self.managed_directory,
                &self.operation_id,
                self.transition_epoch,
            )
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 3 checkpoint fixture panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Feature-gated retained real fresh run-start used only by integration tests.
#[cfg(feature = "engine-test-hooks")]
#[napi]
pub struct Stage6RunStartHandoffFixtureSession {
    session: Arc<Mutex<RunStartHandoffFixtureSession>>,
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl Stage6RunStartHandoffFixtureSession {
    /// Build and retain one real generation-one checkpoint boundary.
    #[napi(constructor, catch_unwind)]
    pub fn new() -> Result<Self> {
        let session = RunStartHandoffFixtureSession::new()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        Ok(Self {
            session: Arc::new(Mutex::new(session)),
        })
    }

    /// Publish or exactly retry the retained run-start checkpoint off the event loop.
    #[napi(catch_unwind)]
    pub fn publish_run_start_checkpoint(
        &self,
        options: ManagedCheckpointPublicationOptions,
    ) -> Result<AsyncTask<PublishStage6PendingRunStartTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        Ok(AsyncTask::new(PublishStage6PendingRunStartTask {
            session: Arc::clone(&self.session),
            managed_directory,
            operation_id,
        }))
    }

    /// Apply the persistence worker's complete committed descriptor to Rust.
    #[napi(catch_unwind)]
    pub fn acknowledge_run_start_persistence(&self, descriptor: Object<'_>) -> Result<()> {
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        lock_recover(&self.session)
            .acknowledge_persistence(&descriptor)
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Construct and publish the running authority off the Node event loop.
    #[napi(catch_unwind)]
    pub fn publish_running_authority(&self) -> Result<AsyncTask<PublishStage6RunningRunStartTask>> {
        Ok(AsyncTask::new(PublishStage6RunningRunStartTask {
            session: Arc::clone(&self.session),
        }))
    }

    /// Return bounded scalar proof without copying world or population state.
    #[napi(catch_unwind)]
    pub fn snapshot(&self) -> Result<Stage6RunStartHandoffSnapshot> {
        run_start_snapshot_to_napi(lock_recover(&self.session).snapshot())
    }
}

/// Async immutable publication for the retained pending run start.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6PendingRunStartTask {
    session: Arc<Mutex<RunStartHandoffFixtureSession>>,
    managed_directory: PathBuf,
    operation_id: CheckpointOperationId,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6PendingRunStartTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session)
                .publish_checkpoint(&self.managed_directory, self.operation_id.clone())
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 pending run-start publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Async collision-safe activation for the retained durable run start.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6RunningRunStartTask {
    session: Arc<Mutex<RunStartHandoffFixtureSession>>,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6RunningRunStartTask {
    type Output = RunStartPublication;
    type JsValue = Stage6RunStartPublication;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session).publish_running_authority()
        })) {
            Ok(Ok(publication)) => Ok(publication),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 run-start activation panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(Stage6RunStartPublication {
            world_epoch: u64_hex(output.world_epoch),
            generation: u64_hex(output.generation),
            completed_step: u64_hex(output.completed_step),
            population_epoch: u64_hex(output.population_epoch),
        })
    }
}

/// Feature-gated retained real generation handoff used only by integration tests.
#[cfg(feature = "engine-test-hooks")]
#[napi]
pub struct Stage6GenerationHandoffFixtureSession {
    session: Arc<Mutex<GenerationHandoffFixtureSession>>,
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl Stage6GenerationHandoffFixtureSession {
    /// Build and retain one real terminal coordinator transition.
    #[napi(constructor, catch_unwind)]
    pub fn new() -> Result<Self> {
        let session = GenerationHandoffFixtureSession::new()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        Ok(Self {
            session: Arc::new(Mutex::new(session)),
        })
    }

    /// Publish the same-run run-start file on libuv's worker pool.
    #[napi(catch_unwind)]
    pub fn publish_run_start_checkpoint(
        &self,
        options: Stage3CheckpointFixtureOptions,
    ) -> Result<AsyncTask<PublishStage6RunStartTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        let transition_epoch = parse_test_hook_epoch(&options.transition_epoch)?;
        Ok(AsyncTask::new(PublishStage6RunStartTask {
            session: Arc::clone(&self.session),
            managed_directory,
            operation_id,
            transition_epoch,
        }))
    }

    /// Publish or exactly retry the retained generation checkpoint off the event loop.
    #[napi(catch_unwind)]
    pub fn publish_generation_checkpoint(
        &self,
        options: ManagedCheckpointPublicationOptions,
    ) -> Result<AsyncTask<PublishStage6GenerationTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        Ok(AsyncTask::new(PublishStage6GenerationTask {
            session: Arc::clone(&self.session),
            managed_directory,
            operation_id,
        }))
    }

    /// Apply the persistence worker's complete committed descriptor to Rust.
    #[napi(catch_unwind)]
    pub fn acknowledge_generation_persistence(&self, descriptor: Object<'_>) -> Result<()> {
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        lock_recover(&self.session)
            .acknowledge_generation_persistence(&descriptor)
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Stage or reborrow the one required reliable controller assignment.
    #[napi(catch_unwind)]
    pub fn prepare_generation_assignment(&self) -> Result<Stage6GenerationAssignment> {
        let assignment = lock_recover(&self.session)
            .prepare_generation_assignment()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        Ok(generation_assignment_to_napi(assignment))
    }

    /// Apply one exact local-send result to the retained assignment.
    #[napi(catch_unwind)]
    pub fn submit_generation_assignment(
        &self,
        result: Stage6GenerationAssignmentResult,
    ) -> Result<()> {
        let operation_epoch = parse_u64_hex(&result.operation_epoch, "operationEpoch", false)?;
        let event_sequence = parse_u64_hex(&result.event_sequence, "eventSequence", false)?;
        let connection_id = parse_u64_hex(&result.connection_id, "connectionId", false)?;
        let lease_id = parse_u64_hex(&result.lease_id, "leaseId", false)?;
        lock_recover(&self.session)
            .submit_generation_assignment(
                operation_epoch,
                event_sequence,
                connection_id,
                lease_id,
                result.accepted,
            )
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Perform the one final authority swap after both required barriers.
    #[napi(catch_unwind)]
    pub fn publish_generation_start(&self) -> Result<Stage6GenerationStartPublication> {
        let publication = lock_recover(&self.session)
            .publish_generation_start()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        let external_assignments =
            u32::try_from(publication.external_assignments).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "generation assignment count exceeds the bounded N-API fixture field",
                )
            })?;
        Ok(Stage6GenerationStartPublication {
            world_epoch: u64_hex(publication.world_epoch),
            generation: u64_hex(publication.generation),
            completed_step: u64_hex(publication.completed_step),
            population_epoch: u64_hex(publication.population_epoch),
            external_assignments,
        })
    }

    /// Return bounded scalar proof without copying world or population state.
    #[napi(catch_unwind)]
    pub fn snapshot(&self) -> Stage6GenerationHandoffSnapshot {
        generation_snapshot_to_napi(lock_recover(&self.session).snapshot())
    }
}

/// Async same-run run-start publication task for the retained session.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6RunStartTask {
    session: Arc<Mutex<GenerationHandoffFixtureSession>>,
    managed_directory: PathBuf,
    operation_id: CheckpointOperationId,
    transition_epoch: u64,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6RunStartTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session).publish_run_start_checkpoint(
                &self.managed_directory,
                self.operation_id.clone(),
                self.transition_epoch,
            )
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 run-start publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Async generation publication task retaining the same real coordinator.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6GenerationTask {
    session: Arc<Mutex<GenerationHandoffFixtureSession>>,
    managed_directory: PathBuf,
    operation_id: CheckpointOperationId,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6GenerationTask {
    type Output = PublishedGenerationHandoff;
    type JsValue = Stage6GenerationCheckpointPublication;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session)
                .publish_generation_checkpoint(&self.managed_directory, self.operation_id.clone())
        })) {
            Ok(Ok(publication)) => Ok(publication),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 generation publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(Stage6GenerationCheckpointPublication {
            descriptor: checkpoint_descriptor_to_napi(output.descriptor),
            generation_commit: generation_commit_to_napi(output.commit_record),
        })
    }
}

/// Validate one controlled path before moving publication to a worker thread.
fn parse_managed_path(value: String) -> Result<PathBuf> {
    if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
        return Err(Error::new(
            Status::InvalidArg,
            "managedDirectory must be a nonempty NUL-free path of at most 32768 UTF-8 bytes",
        ));
    }
    Ok(PathBuf::from(value))
}

/// Validate one exact operation token before any file work.
fn parse_checkpoint_operation_id(value: String) -> Result<CheckpointOperationId> {
    CheckpointOperationId::parse(value)
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))
}

/// Read controlled publication options without allocating unbounded JS strings.
fn parse_managed_checkpoint_publication_options(
    options: &Object<'_>,
) -> Result<(PathBuf, CheckpointOperationId)> {
    let managed_directory = options
        .get::<JsString<'_>>("managedDirectory")?
        .ok_or_else(|| {
            Error::new(
                Status::InvalidArg,
                "checkpoint publication options omit managedDirectory",
            )
        })?;
    let operation_id = options.get::<JsString<'_>>("operationId")?.ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            "checkpoint publication options omit operationId",
        )
    })?;
    let managed_directory =
        bounded_js_string(managed_directory, "managedDirectory", 32_768, false)?;
    let operation_id = bounded_js_string(operation_id, "operationId", 32, false)?;
    Ok((
        parse_managed_path(managed_directory)?,
        parse_checkpoint_operation_id(operation_id)?,
    ))
}

/// Copy one JavaScript string only after bounded well-formed UTF-16 validation.
fn bounded_js_string(
    value: JsString<'_>,
    field: &str,
    max_utf8_bytes: usize,
    allow_empty: bool,
) -> Result<String> {
    let utf16_len = value.utf16_len()?;
    if utf16_len > max_utf8_bytes {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} exceeds its {max_utf8_bytes}-byte limit"),
        ));
    }
    let utf8_len = value.utf8_len()?;
    if utf8_len > max_utf8_bytes {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} exceeds its {max_utf8_bytes}-byte limit"),
        ));
    }
    let decoded = value.into_utf16()?.as_str().map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{field} must be a well-formed UTF-16 string"),
        )
    })?;
    if (!allow_empty && decoded.is_empty()) || decoded.len() > max_utf8_bytes {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be nonempty and at most {max_utf8_bytes} UTF-8 bytes"),
        ));
    }
    Ok(decoded)
}

/// Parse one canonical Uint32 wire value without JavaScript Number narrowing.
fn parse_u32_hex(value: &str, field: &str) -> Result<u32> {
    if value.len() != 8
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be 8 lowercase hexadecimal digits"),
        ));
    }
    u32::from_str_radix(value, 16).map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{field} is not a canonical unsigned 32-bit value"),
        )
    })
}

/// Acquire the retained authority for a synchronous root without ever waiting.
fn try_lock_fresh_inner(
    inner: &Mutex<ExperimentalFreshRunInner>,
) -> Result<MutexGuard<'_, ExperimentalFreshRunInner>> {
    match inner.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => Err(Error::new(
            Status::GenericFailure,
            "experimental fresh-run authority is busy on a worker thread",
        )),
    }
}

/// Reject every future mutation after one worker panic has faulted authority.
fn reject_faulted_fresh_inner(inner: &ExperimentalFreshRunInner) -> Result<()> {
    match inner.fault_detail.as_ref() {
        Some(detail) => Err(Error::new(Status::GenericFailure, detail.clone())),
        None => Ok(()),
    }
}

/// Inspect the permanent fault latch without waiting for a worker-held mutex.
fn reject_faulted_fresh_mutex(inner: &Mutex<ExperimentalFreshRunInner>) -> Result<()> {
    let inner = try_lock_fresh_inner(inner)?;
    reject_faulted_fresh_inner(&inner)
}

/// Latch the first caught worker panic and permanently retire the session.
fn fault_experimental_fresh_run(
    inner: &Mutex<ExperimentalFreshRunInner>,
    context: &str,
    payload: &(dyn std::any::Any + Send),
) -> Error {
    let detail = bounded_fresh_run_panic_detail(context, payload);
    let retained = {
        let mut inner = lock_recover(inner);
        inner.fault_detail.get_or_insert(detail).clone()
    };
    Error::new(Status::GenericFailure, retained)
}

/// Retain the first bounded non-retryable authority-operation failure.
fn retain_experimental_fresh_run_fault(
    inner: &mut ExperimentalFreshRunInner,
    context: &str,
    source: &str,
) -> String {
    let detail = bounded_fresh_run_error_detail(context, source);
    inner.fault_detail.get_or_insert(detail).clone()
}

/// Join bounded context and source text without retaining unbounded diagnostics.
fn bounded_fresh_run_error_detail(context: &str, source: &str) -> String {
    let mut detail = truncate_utf8(context, MAX_ERROR_DETAIL_BYTES);
    if detail.len() < MAX_ERROR_DETAIL_BYTES {
        let separator = ": ";
        let separator_bytes = separator.len().min(MAX_ERROR_DETAIL_BYTES - detail.len());
        detail.push_str(&separator[..separator_bytes]);
    }
    if detail.len() < MAX_ERROR_DETAIL_BYTES {
        detail.push_str(&truncate_utf8(
            source,
            MAX_ERROR_DETAIL_BYTES - detail.len(),
        ));
    }
    detail
}

/// Bound panic text before allocating retained or N-API-visible diagnostics.
fn bounded_fresh_run_panic_detail(context: &str, payload: &(dyn std::any::Any + Send)) -> String {
    let source = if let Some(message) = payload.downcast_ref::<&str>() {
        *message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.as_str()
    } else {
        "non-string panic payload"
    };
    bounded_fresh_run_error_detail(context, source)
}

/// Ensure initialization has not already installed authority before scheduling.
fn ensure_fresh_transition_absent(inner: &Mutex<ExperimentalFreshRunInner>) -> Result<()> {
    let inner = try_lock_fresh_inner(inner)?;
    reject_faulted_fresh_inner(&inner)?;
    if inner.transition.is_some() {
        return Err(Error::new(
            Status::GenericFailure,
            "experimental fresh run is already initialized",
        ));
    }
    Ok(())
}

/// Ensure an admitted authority exists before scheduling dependent work.
fn ensure_fresh_transition_present(inner: &Mutex<ExperimentalFreshRunInner>) -> Result<()> {
    let inner = try_lock_fresh_inner(inner)?;
    reject_faulted_fresh_inner(&inner)?;
    if inner.transition.is_none() {
        return Err(Error::new(
            Status::GenericFailure,
            "experimental fresh run has not been initialized",
        ));
    }
    Ok(())
}

/// Return the stable scalar phase for one active worker operation.
const fn fresh_operation_phase(operation: u8) -> Option<&'static str> {
    match operation {
        FRESH_OPERATION_INITIALIZE => Some("initializing"),
        FRESH_OPERATION_CHECKPOINT => Some("publishingCheckpoint"),
        FRESH_OPERATION_ACTIVATE => Some("activating"),
        FRESH_OPERATION_ACKNOWLEDGE => Some("acknowledgingPersistence"),
        FRESH_OPERATION_INITIAL_FRAME => Some("publishingInitialFrame"),
        FRESH_OPERATION_FIRST_SCHEDULED_FRAME => Some("publishingFirstScheduledFrame"),
        _ => None,
    }
}

/// Produce an intentionally metadata-empty snapshot while the mutex is busy.
fn busy_fresh_run_snapshot(operation: u8) -> Result<FreshRunScalarSnapshot> {
    let phase = fresh_operation_phase(operation).ok_or_else(|| {
        Error::new(
            Status::GenericFailure,
            "experimental fresh-run authority is unexpectedly busy without an active operation",
        )
    })?;
    Ok(FreshRunScalarSnapshot {
        phase,
        transition_epoch: None,
        generation: None,
        completed_step: None,
        checkpoint_published: None,
        persistence_acknowledged: None,
        authority_published: None,
        initial_frame_published: None,
        first_scheduled_frame_published: None,
        snake_count: None,
        pellet_count: None,
        fault_detail: None,
    })
}

/// Read the retained transition into bounded scalar-only Rust metadata.
fn fresh_run_scalar_snapshot(
    inner: &ExperimentalFreshRunInner,
    active_operation: u8,
) -> Result<FreshRunScalarSnapshot> {
    if let Some(detail) = inner.fault_detail.as_ref() {
        return Ok(FreshRunScalarSnapshot {
            phase: "faulted",
            transition_epoch: None,
            generation: None,
            completed_step: None,
            checkpoint_published: None,
            persistence_acknowledged: None,
            authority_published: None,
            initial_frame_published: None,
            first_scheduled_frame_published: None,
            snake_count: None,
            pellet_count: None,
            fault_detail: Some(detail.clone()),
        });
    }
    let phase = if active_operation == FRESH_OPERATION_IDLE {
        match inner.transition.as_ref() {
            None => "created",
            Some(transition) if transition.authority_published() => "running",
            Some(transition) if transition.persistence_acknowledged() => "durableBoundary",
            Some(transition) if transition.checkpoint_published() => "awaitingPersistence",
            Some(_) => "pendingDurability",
        }
    } else {
        fresh_operation_phase(active_operation).ok_or_else(|| {
            Error::new(
                Status::GenericFailure,
                "experimental fresh-run operation state is invalid",
            )
        })?
    };
    let Some(transition) = inner.transition.as_ref() else {
        return Ok(FreshRunScalarSnapshot {
            phase,
            transition_epoch: None,
            generation: None,
            completed_step: None,
            checkpoint_published: None,
            persistence_acknowledged: None,
            authority_published: None,
            initial_frame_published: None,
            first_scheduled_frame_published: None,
            snake_count: None,
            pellet_count: None,
            fault_detail: None,
        });
    };
    Ok(FreshRunScalarSnapshot {
        phase,
        transition_epoch: Some(transition.transition_epoch()),
        generation: Some(transition.generation()),
        completed_step: Some(transition.completed_step()),
        checkpoint_published: Some(transition.checkpoint_published()),
        persistence_acknowledged: Some(transition.persistence_acknowledged()),
        authority_published: Some(transition.authority_published()),
        initial_frame_published: Some(inner.initial_frame_published),
        first_scheduled_frame_published: Some(inner.first_scheduled_frame_published),
        snake_count: Some(transition.snake_count()),
        pellet_count: Some(transition.pellet_count()),
        fault_detail: None,
    })
}

/// Convert one internal scalar snapshot without numeric narrowing.
fn fresh_run_snapshot_to_napi(
    snapshot: FreshRunScalarSnapshot,
) -> Result<ExperimentalFreshRunSnapshot> {
    let snake_count = snapshot
        .snake_count
        .map(|value| {
            u64::try_from(value).map(u64_hex).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "fresh-run snake count exceeds the N-API scalar domain",
                )
            })
        })
        .transpose()?;
    let pellet_count = snapshot
        .pellet_count
        .map(|value| {
            u64::try_from(value).map(u64_hex).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "fresh-run pellet count exceeds the N-API scalar domain",
                )
            })
        })
        .transpose()?;
    Ok(ExperimentalFreshRunSnapshot {
        phase: snapshot.phase.to_owned(),
        transition_epoch: snapshot.transition_epoch.map(u64_hex),
        generation: snapshot.generation.map(u64_hex),
        completed_step: snapshot.completed_step.map(u64_hex),
        checkpoint_published: snapshot.checkpoint_published,
        persistence_acknowledged: snapshot.persistence_acknowledged,
        authority_published: snapshot.authority_published,
        initial_frame_published: snapshot.initial_frame_published,
        first_scheduled_frame_published: snapshot.first_scheduled_frame_published,
        snake_count,
        pellet_count,
        fault_detail: snapshot.fault_detail,
    })
}

/// Check and convert frame routing metadata before exposing exact hex scalars.
fn fresh_run_frame_v1_output(
    bytes: Vec<u8>,
    metadata: FrameV1Metadata,
    completed_step: u64,
) -> Result<FreshRunFrameV1Output> {
    if metadata.byte_length != bytes.len() {
        return Err(Error::new(
            Status::GenericFailure,
            "fresh-run frame-v1 byte metadata does not match its payload",
        ));
    }
    Ok(FreshRunFrameV1Output {
        bytes,
        generation: metadata.generation,
        completed_step,
        total_snakes: frame_usize_to_u64(metadata.total_snakes, "totalSnakes")?,
        alive_snakes: frame_usize_to_u64(metadata.alive_snakes, "aliveSnakes")?,
        pellets: frame_usize_to_u64(metadata.pellets, "pellets")?,
        float_length: frame_usize_to_u64(metadata.float_length, "floatLength")?,
        byte_length: frame_usize_to_u64(metadata.byte_length, "byteLength")?,
    })
}

/// Convert one supported-target frame count without JavaScript Number narrowing.
fn frame_usize_to_u64(value: usize, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            format!("fresh-run frame-v1 {field} exceeds the N-API scalar domain"),
        )
    })
}

/// Move one worker-produced frame into a typed array with exact hex metadata.
fn fresh_run_frame_v1_to_napi(output: FreshRunFrameV1Output) -> ExperimentalFreshRunFrameV1 {
    ExperimentalFreshRunFrameV1 {
        bytes: Uint8Array::from(output.bytes),
        generation: u64_hex(output.generation),
        completed_step: u64_hex(output.completed_step),
        total_snakes: u64_hex(output.total_snakes),
        alive_snakes: u64_hex(output.alive_snakes),
        pellets: u64_hex(output.pellets),
        float_length: u64_hex(output.float_length),
        byte_length: u64_hex(output.byte_length),
    }
}

/// Convert one successful run-start activation without exposing state memory.
fn run_start_publication_to_napi(publication: RunStartPublication) -> Stage6RunStartPublication {
    Stage6RunStartPublication {
        world_epoch: u64_hex(publication.world_epoch),
        generation: u64_hex(publication.generation),
        completed_step: u64_hex(publication.completed_step),
        population_epoch: u64_hex(publication.population_epoch),
    }
}

/// Parse one canonical positive u64 epoch without JavaScript Number narrowing.
#[cfg(feature = "engine-test-hooks")]
fn parse_test_hook_epoch(value: &str) -> Result<u64> {
    parse_u64_hex(value, "transitionEpoch", false)
}

/// Parse one canonical u64 wire value without JavaScript Number narrowing.
fn parse_u64_hex(value: &str, field: &str, allow_zero: bool) -> Result<u64> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be 16 lowercase hexadecimal digits"),
        ));
    }
    let parsed = u64::from_str_radix(value, 16).map_err(|_| {
        Error::new(
            Status::InvalidArg,
            format!("{field} is not a canonical unsigned 64-bit value"),
        )
    })?;
    if !allow_zero && parsed == 0 {
        return Err(Error::new(
            Status::InvalidArg,
            format!("{field} must be positive"),
        ));
    }
    Ok(parsed)
}

/// Convert the Rust descriptor without exposing any authoritative payload bytes.
fn checkpoint_descriptor_to_napi(descriptor: CheckpointDescriptor) -> ManagedCheckpointDescriptor {
    ManagedCheckpointDescriptor {
        protocol_version: descriptor.protocol_version,
        operation_id: descriptor.operation_id.as_str().to_owned(),
        transition_epoch: descriptor.transition_epoch_hex,
        run_id: descriptor.run_id,
        generation: descriptor.generation_hex,
        completed_step: descriptor.completed_step_hex,
        boundary_kind: descriptor.boundary_kind.as_str().to_owned(),
        checkpoint_format_version: descriptor.checkpoint_format_version_hex,
        state_version: descriptor.state_version_hex,
        graph_layout_version: descriptor.graph_layout_version_hex,
        managed_root: descriptor.managed_root,
        relative_filename: descriptor.relative_filename,
        logical_root_sha256: descriptor.logical_root_sha256,
        stored_byte_count: descriptor.stored_byte_count_hex,
        decoded_byte_count: descriptor.decoded_byte_count_hex,
        role_count: descriptor.role_count_hex,
        population_count: descriptor.population_count_hex,
        weight_count: descriptor.weight_count_hex,
        recurrent_state_count: descriptor.recurrent_state_count_hex,
        weights_encoding: descriptor.weights_encoding.as_str().to_owned(),
        recurrent_state_encoding: descriptor.recurrent_state_encoding.as_str().to_owned(),
        graph_layout_sha256: descriptor.graph_layout_sha256,
        write_validation_policy: descriptor.write_validation_policy.as_str().to_owned(),
    }
}

/// Parse a persistence acknowledgement before napi-rs can allocate its strings.
fn checkpoint_descriptor_from_napi_object(descriptor: &Object<'_>) -> Result<CheckpointDescriptor> {
    require_exact_checkpoint_descriptor_keys(descriptor)?;
    let protocol_version = descriptor.get::<f64>("protocolVersion")?.ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            "checkpoint descriptor omits protocolVersion",
        )
    })?;
    if !protocol_version.is_finite()
        || protocol_version.fract() != 0.0
        || protocol_version != f64::from(CHECKPOINT_DESCRIPTOR_VERSION)
    {
        return Err(Error::new(
            Status::InvalidArg,
            "protocolVersion must be the exact supported integer",
        ));
    }
    checkpoint_descriptor_from_napi(ManagedCheckpointDescriptor {
        protocol_version: CHECKPOINT_DESCRIPTOR_VERSION,
        operation_id: bounded_object_string(descriptor, "operationId", 32)?,
        transition_epoch: bounded_object_string(descriptor, "transitionEpoch", 16)?,
        run_id: bounded_object_string(descriptor, "runId", MAX_EXPERIMENTAL_RUN_ID_BYTES)?,
        generation: bounded_object_string(descriptor, "generation", 16)?,
        completed_step: bounded_object_string(descriptor, "completedStep", 16)?,
        boundary_kind: bounded_object_string(descriptor, "boundaryKind", 10)?,
        checkpoint_format_version: bounded_object_string(
            descriptor,
            "checkpointFormatVersion",
            16,
        )?,
        state_version: bounded_object_string(descriptor, "stateVersion", 16)?,
        graph_layout_version: bounded_object_string(descriptor, "graphLayoutVersion", 16)?,
        managed_root: bounded_object_string(descriptor, "managedRoot", 13)?,
        relative_filename: bounded_object_string(descriptor, "relativeFilename", 78)?,
        logical_root_sha256: bounded_object_string(descriptor, "logicalRootSha256", 64)?,
        stored_byte_count: bounded_object_string(descriptor, "storedByteCount", 16)?,
        decoded_byte_count: bounded_object_string(descriptor, "decodedByteCount", 16)?,
        role_count: bounded_object_string(descriptor, "roleCount", 16)?,
        population_count: bounded_object_string(descriptor, "populationCount", 16)?,
        weight_count: bounded_object_string(descriptor, "weightCount", 16)?,
        recurrent_state_count: bounded_object_string(descriptor, "recurrentStateCount", 16)?,
        weights_encoding: bounded_object_string(descriptor, "weightsEncoding", 32)?,
        recurrent_state_encoding: bounded_object_string(descriptor, "recurrentStateEncoding", 32)?,
        graph_layout_sha256: bounded_object_string(descriptor, "graphLayoutSha256", 64)?,
        write_validation_policy: bounded_object_string(descriptor, "writeValidationPolicy", 64)?,
    })
}

/// Require the complete enumerable own-key set before reading any field value.
fn require_exact_checkpoint_descriptor_keys(descriptor: &Object<'_>) -> Result<()> {
    let names = descriptor.get_property_names()?;
    let length = names.get_array_length()?;
    if usize::try_from(length).ok() != Some(CHECKPOINT_DESCRIPTOR_INPUT_KEYS.len()) {
        return Err(Error::new(
            Status::InvalidArg,
            "checkpoint descriptor has unknown or missing fields",
        ));
    }
    let mut seen = [false; CHECKPOINT_DESCRIPTOR_INPUT_KEYS.len()];
    for index in 0..length {
        let key = names.get_element::<JsString<'_>>(index)?;
        let key = bounded_js_string(key, "checkpoint descriptor key", 64, false)?;
        let position = CHECKPOINT_DESCRIPTOR_INPUT_KEYS
            .iter()
            .position(|expected| *expected == key)
            .ok_or_else(|| {
                Error::new(
                    Status::InvalidArg,
                    format!("checkpoint descriptor contains unknown field {key}"),
                )
            })?;
        if seen[position]
            || !descriptor.has_own_property(CHECKPOINT_DESCRIPTOR_INPUT_KEYS[position])?
        {
            return Err(Error::new(
                Status::InvalidArg,
                "checkpoint descriptor has duplicate or inherited fields",
            ));
        }
        seen[position] = true;
    }
    if seen.iter().any(|present| !present) {
        return Err(Error::new(
            Status::InvalidArg,
            "checkpoint descriptor has unknown or missing fields",
        ));
    }
    Ok(())
}

/// Read one required bounded string property through a raw JavaScript handle.
fn bounded_object_string(
    object: &Object<'_>,
    field: &str,
    max_utf8_bytes: usize,
) -> Result<String> {
    let value = object.get::<JsString<'_>>(field)?.ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            format!("checkpoint descriptor omits {field}"),
        )
    })?;
    bounded_js_string(value, field, max_utf8_bytes, false)
}

/// Reconstruct one scalar descriptor supplied by the persistence worker.
fn checkpoint_descriptor_from_napi(
    descriptor: ManagedCheckpointDescriptor,
) -> Result<CheckpointDescriptor> {
    if descriptor.protocol_version != CHECKPOINT_DESCRIPTOR_VERSION {
        return Err(Error::new(
            Status::InvalidArg,
            "protocolVersion is not the supported checkpoint descriptor version",
        ));
    }
    if descriptor.managed_root != "checkpoint-v3" {
        return Err(Error::new(
            Status::InvalidArg,
            "managedRoot is not the controlled checkpoint-v3 root",
        ));
    }
    for (field, value) in [
        ("transitionEpoch", descriptor.transition_epoch.as_str()),
        ("generation", descriptor.generation.as_str()),
        ("completedStep", descriptor.completed_step.as_str()),
        (
            "checkpointFormatVersion",
            descriptor.checkpoint_format_version.as_str(),
        ),
        ("stateVersion", descriptor.state_version.as_str()),
        (
            "graphLayoutVersion",
            descriptor.graph_layout_version.as_str(),
        ),
        ("storedByteCount", descriptor.stored_byte_count.as_str()),
        ("decodedByteCount", descriptor.decoded_byte_count.as_str()),
        ("roleCount", descriptor.role_count.as_str()),
        ("populationCount", descriptor.population_count.as_str()),
        ("weightCount", descriptor.weight_count.as_str()),
        (
            "recurrentStateCount",
            descriptor.recurrent_state_count.as_str(),
        ),
    ] {
        let allow_zero = field != "transitionEpoch";
        parse_u64_hex(value, field, allow_zero)?;
    }
    let operation_id = parse_checkpoint_operation_id(descriptor.operation_id)?;
    let boundary_kind = match descriptor.boundary_kind.as_str() {
        "run-start" => CheckpointBoundaryKind::RunStart,
        "generation" => CheckpointBoundaryKind::Generation,
        _ => {
            return Err(Error::new(
                Status::InvalidArg,
                "boundaryKind is not a supported checkpoint boundary",
            ));
        }
    };
    let generation = parse_u64_hex(&descriptor.generation, "generation", true)?;
    let completed_step = parse_u64_hex(&descriptor.completed_step, "completedStep", true)?;
    match boundary_kind {
        CheckpointBoundaryKind::RunStart if generation != 1 || completed_step != 0 => {
            return Err(Error::new(
                Status::InvalidArg,
                "run-start descriptor must be generation one at completed step zero",
            ));
        }
        CheckpointBoundaryKind::Generation if completed_step == 0 => {
            return Err(Error::new(
                Status::InvalidArg,
                "generation descriptor must have a positive completed-step count",
            ));
        }
        _ => {}
    }
    let weights_encoding = parse_numeric_encoding(&descriptor.weights_encoding)?;
    let recurrent_state_encoding = parse_numeric_encoding(&descriptor.recurrent_state_encoding)?;
    if descriptor.write_validation_policy
        != CheckpointWriteValidationPolicy::SinglePassLogicalHashesFsyncRenameV1.as_str()
    {
        return Err(Error::new(
            Status::InvalidArg,
            "writeValidationPolicy is not the approved checkpoint policy",
        ));
    }
    for (field, value, maximum) in [
        (
            "runId",
            descriptor.run_id.as_str(),
            MAX_EXPERIMENTAL_RUN_ID_BYTES,
        ),
        (
            "relativeFilename",
            descriptor.relative_filename.as_str(),
            78usize,
        ),
        (
            "logicalRootSha256",
            descriptor.logical_root_sha256.as_str(),
            64usize,
        ),
        (
            "graphLayoutSha256",
            descriptor.graph_layout_sha256.as_str(),
            64usize,
        ),
    ] {
        if value.is_empty() || value.len() > maximum || value.contains('\0') {
            return Err(Error::new(
                Status::InvalidArg,
                format!("{field} exceeds the bounded descriptor fixture field"),
            ));
        }
    }
    for (field, value) in [
        ("logicalRootSha256", descriptor.logical_root_sha256.as_str()),
        ("graphLayoutSha256", descriptor.graph_layout_sha256.as_str()),
    ] {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::new(
                Status::InvalidArg,
                format!("{field} must be a lowercase SHA-256 hex string"),
            ));
        }
    }
    if descriptor.relative_filename != format!("{}.checkpoint-v3", descriptor.logical_root_sha256) {
        return Err(Error::new(
            Status::InvalidArg,
            "relativeFilename is not derived from logicalRootSha256",
        ));
    }
    Ok(CheckpointDescriptor {
        protocol_version: descriptor.protocol_version,
        managed_root: descriptor.managed_root,
        operation_id,
        transition_epoch_hex: descriptor.transition_epoch,
        run_id: descriptor.run_id,
        generation_hex: descriptor.generation,
        completed_step_hex: descriptor.completed_step,
        boundary_kind,
        checkpoint_format_version_hex: descriptor.checkpoint_format_version,
        state_version_hex: descriptor.state_version,
        graph_layout_version_hex: descriptor.graph_layout_version,
        logical_root_sha256: descriptor.logical_root_sha256,
        relative_filename: descriptor.relative_filename,
        stored_byte_count_hex: descriptor.stored_byte_count,
        decoded_byte_count_hex: descriptor.decoded_byte_count,
        population_count_hex: descriptor.population_count,
        role_count_hex: descriptor.role_count,
        weight_count_hex: descriptor.weight_count,
        recurrent_state_count_hex: descriptor.recurrent_state_count,
        weights_encoding,
        recurrent_state_encoding,
        graph_layout_sha256: descriptor.graph_layout_sha256,
        write_validation_policy:
            CheckpointWriteValidationPolicy::SinglePassLogicalHashesFsyncRenameV1,
    })
}

/// Parse one stable numeric encoding used by the checkpoint descriptor.
fn parse_numeric_encoding(value: &str) -> Result<NumericEncoding> {
    match value {
        "raw-f32le-v1" => Ok(NumericEncoding::RawF32LeV1),
        "f32le-shuffle4-zstd-v1" => Ok(NumericEncoding::F32LeShuffle4ZstdV1),
        _ => Err(Error::new(
            Status::InvalidArg,
            "checkpoint descriptor contains an unsupported numeric encoding",
        )),
    }
}

/// Convert one exact u64 or Float64-bit word to canonical wire hexadecimal.
fn u64_hex(value: u64) -> String {
    format!("{value:016x}")
}

/// Convert the Rust-owned compact generation record without numeric narrowing.
#[cfg(feature = "engine-test-hooks")]
fn generation_commit_to_napi(record: GenerationCommitRecord) -> Stage6GenerationCommitRecord {
    Stage6GenerationCommitRecord {
        summary: Stage6GenerationSummaryRecord {
            completed_generation: u64_hex(record.summary.completed_generation),
            best_f64_hex: u64_hex(record.summary.best_f64_bits),
            average_f64_hex: u64_hex(record.summary.average_f64_bits),
            minimum_f64_hex: u64_hex(record.summary.minimum_f64_bits),
            species_count: u64_hex(record.summary.species_count),
            top_species_size: u64_hex(record.summary.top_species_size),
            average_weight_f64_hex: u64_hex(record.summary.average_weight_f64_bits),
            weight_variance_f64_hex: u64_hex(record.summary.weight_variance_f64_bits),
        },
        hall_of_fame: Stage6HallOfFameRecord {
            completed_generation: u64_hex(record.hall_of_fame.completed_generation),
            source_population_slot: u64_hex(record.hall_of_fame.source_population_slot),
            source_snake_id: u64_hex(record.hall_of_fame.source_snake_id),
            fitness_f64_hex: u64_hex(record.hall_of_fame.fitness_f64_bits),
            points_f64_hex: u64_hex(record.hall_of_fame.points_f64_bits),
            length: u64_hex(record.hall_of_fame.length),
            successor_population_slot: u64_hex(record.hall_of_fame.successor_population_slot),
            successor_genome_id: u64_hex(record.hall_of_fame.successor_genome_id),
        },
    }
}

/// Convert one retained Rust assignment to exact scalar wire fields.
#[cfg(feature = "engine-test-hooks")]
fn generation_assignment_to_napi(
    assignment: GenerationHandoffAssignment,
) -> Stage6GenerationAssignment {
    Stage6GenerationAssignment {
        operation_epoch: u64_hex(assignment.operation_epoch),
        event_sequence: u64_hex(assignment.event_sequence),
        connection_id: u64_hex(assignment.connection_id),
        lease_id: u64_hex(assignment.lease_id),
        snake_id: u64_hex(assignment.snake_id),
        resume_token: assignment.resume_token,
    }
}

/// Convert current authority/barrier proof to bounded scalar wire fields.
#[cfg(feature = "engine-test-hooks")]
fn generation_snapshot_to_napi(
    snapshot: GenerationHandoffSnapshot,
) -> Stage6GenerationHandoffSnapshot {
    Stage6GenerationHandoffSnapshot {
        world_epoch: u64_hex(snapshot.world_epoch),
        generation: u64_hex(snapshot.generation),
        completed_step: u64_hex(snapshot.completed_step),
        transition_pending: snapshot.transition_pending,
        checkpoint_published: snapshot.checkpoint_published,
        persistence_acknowledged: snapshot.persistence_acknowledged,
        generation_checkpoint_publications: snapshot.generation_checkpoint_publications,
        authority_publications: snapshot.authority_publications,
    }
}

/// Convert fresh run-start barrier proof to bounded exact scalar wire fields.
#[cfg(feature = "engine-test-hooks")]
fn run_start_snapshot_to_napi(
    snapshot: RunStartHandoffSnapshot,
) -> Result<Stage6RunStartHandoffSnapshot> {
    let snake_count = u64::try_from(snapshot.snake_count).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "run-start snake count exceeds the bounded N-API fixture field",
        )
    })?;
    let pellet_count = u64::try_from(snapshot.pellet_count).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "run-start pellet count exceeds the bounded N-API fixture field",
        )
    })?;
    Ok(Stage6RunStartHandoffSnapshot {
        transition_epoch: u64_hex(snapshot.transition_epoch),
        generation: u64_hex(snapshot.generation),
        completed_step: u64_hex(snapshot.completed_step),
        checkpoint_published: snapshot.checkpoint_published,
        persistence_acknowledged: snapshot.persistence_acknowledged,
        authority_published: snapshot.authority_published,
        snake_count: u64_hex(snake_count),
        pellet_count: u64_hex(pellet_count),
        checkpoint_publications: snapshot.checkpoint_publications,
        authority_publications: snapshot.authority_publications,
    })
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
            request: Stage6aP0FreshRunRequest {
                run_id: "unit-run".to_owned(),
                seed: 1,
                memory_ceiling_bytes: 1,
            },
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
            request: Stage6aP0FreshRunRequest {
                run_id: "unit-fault-run".to_owned(),
                seed: 1,
                memory_ceiling_bytes: 1,
            },
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
        let _ =
            fault_experimental_fresh_run(&session.inner, "different context", &replacement_payload);
        let retained =
            fresh_run_scalar_snapshot(&lock_recover(&session.inner), FRESH_OPERATION_IDLE)
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
}
