//! N-API facade for fresh-run startup and bounded native engine boundaries.
//!
//! The production fresh-run session keeps its lifecycle and operation admission
//! here. Specialist children provide wire conversion, existing async tasks,
//! hook-only fixtures and the independent coarse probe bridge. No boundary
//! exposes per-snake, per-layer or per-step neural execution.

#![cfg_attr(
    all(test, feature = "engine-test-hooks"),
    allow(
        dead_code,
        reason = "feature-gated N-API exports are invoked by Node integration tests"
    )
)]

use crate::engine::calculation_workers::validate_calculation_workers;
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
    CheckpointWriteValidationPolicy, HallOfFameWeightsDescriptor, NumericEncoding,
    CHECKPOINT_DESCRIPTOR_VERSION,
};
use crate::engine::contract::{
    CommandBatch, CompletedEvent, EngineCommand, EngineInit, GenerationAssignmentReceiptState,
    InboundLimits, OutputLimits, ReliableEvent, RunningAuthorityEvent, SequencedCommand,
    ENGINE_CONTRACT_VERSION,
};
use crate::engine::error::{truncate_utf8, EngineError, EngineErrorCode, MAX_ERROR_DETAIL_BYTES};
use crate::engine::export_archive::prepare_legacy_sqlite_population_import;
use crate::engine::frame_v1::FrameV1Metadata;
use crate::engine::fresh_run::{
    prepare_default_fresh_run_with_math, prepare_stage6a_p0_checkpoint_restore,
    prepare_stage6a_p0_compatible_checkpoint_restore, Stage6aP0FreshRunRequest,
};
use crate::engine::generation::GenerationCommitRecord;
use crate::engine::physics::PhysicsStepKey;
use crate::engine::queues::WakeSink;
use crate::engine::run_start::PendingRunStartTransition;
use crate::engine::running_loop::RunningAuthorityLoop;
use crate::engine::runtime::{EngineHealth, EngineRuntime};
use crate::engine::scheduler::FixedStepSchedulerPolicy;
use crate::engine::state::ControllerKind;
use crate::engine::state::RunStartPublication;
use crate::engine::GenerationTransitionReason;
use crate::engine::LifecycleState;
#[cfg(feature = "engine-test-hooks")]
use crate::engine::{
    checkpoint_fixture::publish_stage3_fixture,
    contract::{ExternalDeliveryReceipt, RunningAuthorityCommand},
    generation_handoff_fixture::{
        background_generation_handoff_fixture, background_generation_handoff_runtime_init,
        GenerationHandoffAssignment, GenerationHandoffFixtureSession,
        GenerationHandoffRunStartFixture, GenerationHandoffSnapshot, PublishedGenerationHandoff,
    },
    run_start_handoff_fixture::{RunStartHandoffFixtureSession, RunStartHandoffSnapshot},
};
use crate::napi_running_engine::ExperimentalRunningAuthority;

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
/// The durable step-zero authority is being transferred to its background owner.
const FRESH_OPERATION_BACKGROUND: u8 = 7;
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

/// Mutable Rust authority retained behind the experimental session mutex.
#[derive(Default)]
struct ExperimentalFreshRunInner {
    transition: Option<PendingRunStartTransition>,
    background_retry_loop: Option<RunningAuthorityLoop>,
    background_transferred: bool,
    initial_frame_published: bool,
    first_scheduled_frame_published: bool,
    fault_detail: Option<String>,
}

/// Coarse startup owner used by the Rust-authoritative production server.
///
/// Construction, managed-file publication, and initial-world activation run on
/// libuv workers. Optional arithmetic selection is admitted before population allocation.
#[napi(js_name = "ExperimentalStage6aFreshRunSession")]
pub struct ExperimentalStage6aFreshRunSession {
    request: Stage6aP0FreshRunRequest,
    requested_math: Option<crate::engine::inference::InferenceMathBackend>,
    calculation_workers: usize,
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
        calculation_workers: Option<u32>,
        math_backend: Option<JsString<'_>>,
    ) -> Result<Self> {
        let math_backend = math_backend
            .map(|value| bounded_js_string(value, "mathBackend", 64, false))
            .transpose()?;
        let requested_math = parse_requested_math_backend(
            math_backend.as_deref(),
            crate::engine::inference::InferenceMathBackend::is_available,
        )
        .map_err(|error| Error::new(Status::InvalidArg, error))?;
        let calculation_workers = calculation_workers.unwrap_or(1) as usize;
        validate_calculation_workers(calculation_workers)
            .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
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
            requested_math,
            request: Stage6aP0FreshRunRequest {
                run_id,
                seed,
                memory_ceiling_bytes,
            },
            calculation_workers,
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
            #[cfg(feature = "engine-test-hooks")]
            trained_cohort_json: None,
            #[cfg(feature = "engine-test-hooks")]
            capacity_audit_json: None,
            request: self.request.clone(),
            calculation_workers: self.calculation_workers,
            requested_math: self.requested_math,
            source: FreshRunInitialization::Fresh,
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
        }))
    }

    /// Stream a worker-selected immutable checkpoint off-loop before activation.
    #[napi(catch_unwind)]
    pub fn initialize_from_checkpoint(
        &self,
        managed_directory: JsString<'_>,
        descriptor: Object<'_>,
        recovery_branch: Option<bool>,
        compatible_build: Option<bool>,
        population_fork: Option<bool>,
    ) -> Result<AsyncTask<InitializeExperimentalFreshRunTask>> {
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        let parsed = (|| {
            let directory = parse_managed_path(bounded_js_string(
                managed_directory,
                "managedDirectory",
                32_768,
                false,
            )?)?;
            let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
            let recovery = recovery_branch.unwrap_or(false);
            let compatible = compatible_build.unwrap_or(false);
            let population_fork = population_fork.unwrap_or(false);
            if population_fork {
                if recovery
                    || !compatible
                    || self.requested_math.is_none()
                    || descriptor.run_id == self.request.run_id
                {
                    return Err(Error::new(Status::InvalidArg,
                        "population fork requires a distinct new run, explicit backend and compatible source validation"));
                }
            } else if (descriptor.run_id != self.request.run_id) != recovery {
                return Err(Error::new(
                    Status::InvalidArg,
                    "selected checkpoint run differs from session run",
                ));
            }
            ensure_fresh_transition_absent(&self.inner)?;
            Ok((directory, descriptor, recovery, compatible, population_fork))
        })();
        let restore = match parsed {
            Ok(restore) => restore,
            Err(error) => {
                self.active_operation
                    .store(FRESH_OPERATION_IDLE, Ordering::Release);
                return Err(error);
            }
        };
        Ok(AsyncTask::new(InitializeExperimentalFreshRunTask {
            #[cfg(feature = "engine-test-hooks")]
            trained_cohort_json: None,
            #[cfg(feature = "engine-test-hooks")]
            capacity_audit_json: None,
            request: self.request.clone(),
            calculation_workers: self.calculation_workers,
            requested_math: self.requested_math,
            source: FreshRunInitialization::Checkpoint(Box::new(restore)),
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
        }))
    }

    /// Read one worker-selected TypeScript v2 checkpoint directly from SQLite.
    #[napi(catch_unwind)]
    pub fn initialize_from_legacy_sqlite(
        &self,
        database_path: JsString<'_>,
        snapshot_id_hex: JsString<'_>,
    ) -> Result<AsyncTask<InitializeExperimentalFreshRunTask>> {
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        let parsed = (|| {
            let database_path = parse_managed_path(bounded_js_string(
                database_path,
                "databasePath",
                32_768,
                false,
            )?)?;
            let snapshot_id = parse_u64_hex(
                &bounded_js_string(snapshot_id_hex, "snapshotIdHex", 16, false)?,
                "snapshotIdHex",
                false,
            )?;
            let snapshot_id = i64::try_from(snapshot_id).map_err(|_| {
                Error::new(
                    Status::InvalidArg,
                    "legacy snapshot ID exceeds SQLite INTEGER",
                )
            })?;
            ensure_fresh_transition_absent(&self.inner)?;
            Ok((database_path, snapshot_id))
        })();
        let legacy = match parsed {
            Ok(value) => value,
            Err(error) => {
                self.active_operation
                    .store(FRESH_OPERATION_IDLE, Ordering::Release);
                return Err(error);
            }
        };
        Ok(AsyncTask::new(InitializeExperimentalFreshRunTask {
            #[cfg(feature = "engine-test-hooks")]
            trained_cohort_json: None,
            #[cfg(feature = "engine-test-hooks")]
            capacity_audit_json: None,
            request: self.request.clone(),
            calculation_workers: self.calculation_workers,
            requested_math: self.requested_math,
            source: FreshRunInitialization::LegacySqlite(legacy),
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

    /// Move the retained validated candidate after its exact branch commit; no second restore.
    #[napi(catch_unwind)]
    pub fn adopt_recovery_branch(
        &self,
        branch_run_id: JsString<'_>,
        descriptor: Object<'_>,
    ) -> Result<AsyncTask<AdoptRecoveryBranchTask>> {
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        let parsed = (|| {
            let run_id = bounded_js_string(branch_run_id, "branchRunId", 256, false)?;
            let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
            ensure_fresh_transition_present(&self.inner)?;
            Ok((run_id, descriptor))
        })();
        let (run_id, descriptor) = match parsed {
            Ok(value) => value,
            Err(error) => {
                self.active_operation
                    .store(FRESH_OPERATION_IDLE, Ordering::Release);
                return Err(error);
            }
        };
        Ok(AsyncTask::new(AdoptRecoveryBranchTask {
            run_id,
            descriptor,
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
        }))
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

    /// Read bounded welcome metadata before transferring the authority.
    #[napi(catch_unwind)]
    pub fn startup_metadata(&self) -> Result<String> {
        let inner = self
            .inner
            .try_lock()
            .map_err(|_| Error::new(Status::GenericFailure, "fresh-run session is busy"))?;
        if inner.fault_detail.is_some() {
            return Err(Error::new(
                Status::GenericFailure,
                "fresh-run session is faulted",
            ));
        }
        inner
            .transition
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    Status::GenericFailure,
                    "startup metadata requires retained authority",
                )
            })?
            .startup_metadata_json()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Transfer the activated step-zero authority to an unstarted background
    /// runtime off the Node loop. The one-shot scheduler becomes inaccessible.
    #[napi(catch_unwind)]
    pub fn create_background_runtime(
        &self,
        options: ExperimentalEngineOptions,
        wake_callback: Function<'_, (), ()>,
    ) -> Result<AsyncTask<CreateExperimentalBackgroundTask>> {
        let init = options_to_init(options).map_err(engine_error_to_napi)?;
        if init.output.max_event_owned_bytes < std::mem::size_of::<RunningAuthorityEvent>() {
            return Err(Error::new(
                Status::InvalidArg,
                "background output must fit one authority event",
            ));
        }
        let wake = wake_callback
            .build_threadsafe_function::<()>()
            .callee_handled::<false>()
            .weak::<true>()
            .max_queue_size::<1>()
            .build_callback(|_context| Ok(()))?;
        self.begin_operation(FRESH_OPERATION_BACKGROUND)?;
        Ok(AsyncTask::new(CreateExperimentalBackgroundTask {
            inner: Arc::clone(&self.inner),
            calculation_workers: self.calculation_workers,
            memory_ceiling_bytes: self.request.memory_ceiling_bytes,
            active_operation: Arc::clone(&self.active_operation),
            init,
            wake: Arc::new(NapiWakeSink::new(wake)),
        }))
    }
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl ExperimentalStage6aFreshRunSession {
    /// Restore an exact candidate boundary only with explicit sensor/math/ray admission.
    #[napi(catch_unwind)]
    pub fn initialize_sensor_checkpoint_for_test(
        &self,
        managed_directory: JsString<'_>,
        descriptor: Object<'_>,
        admission: Object<'_>,
    ) -> Result<AsyncTask<InitializeExperimentalFreshRunTask>> {
        let keys = [
            "profile",
            "bins",
            "mathBackend",
            "rayMinimum",
            "rayCap",
            "rayBatch",
            "envelopeMiB",
        ];
        let names = admission.get_property_names()?;
        if !matches!(names.get_array_length()?, 6 | 7) {
            return Err(Error::new(
                Status::InvalidArg,
                "qualified restore requires exact fields",
            ));
        }
        for index in 0..names.get_array_length()? {
            let key = bounded_js_string(
                names.get_element::<JsString<'_>>(index)?,
                "restore key",
                32,
                false,
            )?;
            if !keys.contains(&key.as_str()) || !admission.has_own_property(&key)? {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualified restore requires exact own fields",
                ));
            }
        }
        let profile = match bounded_object_string(&admission, "profile", 6)?.as_str() {
            "basic" => crate::engine::sensor_contract::SensorProfile::Basic,
            "motion" => crate::engine::sensor_contract::SensorProfile::Motion,
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualified restore requires Basic/Motion",
                ))
            }
        };
        let math = match bounded_object_string(&admission, "mathBackend", 32)?.as_str() {
            "scalar" => crate::engine::inference::InferenceMathBackend::Scalar,
            "sse2" => crate::engine::inference::InferenceMathBackend::Sse2,
            "i16-matrix-fma128" => crate::engine::inference::InferenceMathBackend::I16MatrixFma128,
            "i16-affine-fma128-sse2" => {
                crate::engine::inference::InferenceMathBackend::I16AffineFma128
            }
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualified restore math backend",
                ))
            }
        };
        let integer = |key, low: f64, high: f64| -> Result<usize> {
            let value = admission
                .get::<f64>(key)?
                .ok_or_else(|| Error::new(Status::InvalidArg, "restore value missing"))?;
            if !value.is_finite() || value.fract() != 0.0 || value < low || value > high {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualified restore integer range",
                ));
            }
            Ok(value as usize)
        };
        let bins = integer("bins", 8.0, 32.0)?;
        crate::engine::sensor_contract::SensorContractIdentity::new(profile, bins)
            .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        let ray_work = crate::engine::ray_work::RayWorkLimits::new(
            integer("rayMinimum", 0.0, 1_000_000.0)? as u32,
            integer("rayCap", 1.0, 1_000_000.0)? as u32,
            integer("rayBatch", 1.0, 10_000_000.0)? as u32,
            512,
        )
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        let capacity_policy = if admission.has_own_property("envelopeMiB")? {
            if profile != crate::engine::sensor_contract::SensorProfile::Motion || bins != 32 {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualified restore capacity envelope requires Motion/B32",
                ));
            }
            Some(
                crate::engine::checkpoint_capacity::CheckpointCapacityPolicy::new(
                    integer("envelopeMiB", 512.0, 768.0)?,
                    false,
                )
                .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?,
            )
        } else {
            None
        };
        let directory = parse_managed_path(bounded_js_string(
            managed_directory,
            "managedDirectory",
            32_768,
            false,
        )?)?;
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        if descriptor.run_id != self.request.run_id {
            return Err(Error::new(
                Status::InvalidArg,
                "qualified restore run differs from session",
            ));
        }
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        if let Err(error) = ensure_fresh_transition_absent(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(InitializeExperimentalFreshRunTask {
            #[cfg(feature = "engine-test-hooks")]
            trained_cohort_json: None,
            #[cfg(feature = "engine-test-hooks")]
            capacity_audit_json: None,
            request: self.request.clone(),
            calculation_workers: self.calculation_workers,
            requested_math: None,
            source: FreshRunInitialization::QualifiedCheckpoint(Box::new(
                QualifiedCheckpointInitialization {
                    directory,
                    descriptor,
                    profile,
                    bins,
                    math,
                    ray_work,
                    capacity_policy,
                },
            )),
            inner: Arc::clone(&self.inner),
            active_operation: Arc::clone(&self.active_operation),
        }))
    }

    /// Construct one explicit Motion/B32 capacity candidate through ordinary native admission.
    #[napi(catch_unwind)]
    pub fn initialize_capacity_for_test(
        &self,
        request: Object<'_>,
    ) -> Result<AsyncTask<InitializeQualifiedFreshRunTask>> {
        let keys = [
            "population",
            "graph",
            "envelopeMiB",
            "rawFallback",
            "rayMinimum",
            "rayCap",
            "rayBatch",
        ];
        let names = request.get_property_names()?;
        if names.get_array_length()? != keys.len() as u32 {
            return Err(Error::new(
                Status::InvalidArg,
                "capacity qualification requires exact fields",
            ));
        }
        for index in 0..names.get_array_length()? {
            let key = bounded_js_string(
                names.get_element::<JsString<'_>>(index)?,
                "capacity key",
                32,
                false,
            )?;
            if !keys.contains(&key.as_str()) || !request.has_own_property(&key)? {
                return Err(Error::new(
                    Status::InvalidArg,
                    "capacity qualification requires exact own fields",
                ));
            }
        }
        let integer = |key, low: f64, high: f64| -> Result<usize> {
            let value = request
                .get::<f64>(key)?
                .ok_or_else(|| Error::new(Status::InvalidArg, "capacity value missing"))?;
            if !value.is_finite() || value.fract() != 0.0 || value < low || value > high {
                return Err(Error::new(
                    Status::InvalidArg,
                    "capacity qualification integer range",
                ));
            }
            Ok(value as usize)
        };
        let population = integer("population", 55.0, 300.0)?;
        if !matches!(population, 55 | 300) {
            return Err(Error::new(
                Status::InvalidArg,
                "capacity population must be 55 or 300",
            ));
        }
        let large_graph = match bounded_object_string(&request, "graph", 6)?.as_str() {
            "modest" => false,
            "large" => true,
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    "capacity graph must be modest/large",
                ))
            }
        };
        let raw = request.get::<bool>("rawFallback")?.ok_or_else(|| {
            Error::new(Status::InvalidArg, "capacity rawFallback boolean missing")
        })?;
        let policy = crate::engine::checkpoint_capacity::CheckpointCapacityPolicy::new(
            integer("envelopeMiB", 512.0, 768.0)?,
            raw,
        )
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        let ray_work = crate::engine::ray_work::RayWorkLimits::new(
            integer("rayMinimum", 0.0, 1_000_000.0)? as u32,
            integer("rayCap", 1.0, 1_000_000.0)? as u32,
            integer("rayBatch", 1.0, 10_000_000.0)? as u32,
            512,
        )
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        if let Err(error) = ensure_fresh_transition_absent(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(InitializeQualifiedFreshRunTask {
            inner: InitializeExperimentalFreshRunTask {
                trained_cohort_json: None,
                capacity_audit_json: None,
                request: self.request.clone(),
                calculation_workers: self.calculation_workers,
                requested_math: None,
                source: FreshRunInitialization::Capacity {
                    population,
                    large_graph,
                    policy,
                    ray_work,
                },
                inner: Arc::clone(&self.inner),
                active_operation: Arc::clone(&self.active_operation),
            },
        }))
    }

    /// Construct an explicitly admitted v4/v2 qualification fixture; absent from production builds.
    #[napi(catch_unwind)]
    pub fn initialize_baseline_v2_for_test(
        &self,
        request: Object<'_>,
    ) -> Result<AsyncTask<InitializeQualifiedFreshRunTask>> {
        let keys = [
            "profile",
            "bins",
            "mode",
            "cadence",
            "rayMinimum",
            "rayCap",
            "rayBatch",
            "workload",
            "trainedCohort",
            "pelletChecks",
            "segmentChecks",
        ];
        let names = request.get_property_names()?;
        if !matches!(names.get_array_length()?, 7..=11) {
            return Err(Error::new(
                Status::InvalidArg,
                "qualification requires exact fields",
            ));
        }
        for index in 0..names.get_array_length()? {
            let key = bounded_js_string(
                names.get_element::<JsString<'_>>(index)?,
                "qualification key",
                32,
                false,
            )?;
            if !keys.contains(&key.as_str()) || !request.has_own_property(&key)? {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualification requires exact own fields",
                ));
            }
        }
        let profile = match bounded_object_string(&request, "profile", 6)?.as_str() {
            "basic" => crate::engine::sensor_contract::SensorProfile::Basic,
            "motion" => crate::engine::sensor_contract::SensorProfile::Motion,
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualification requires v4 Basic/Motion",
                ))
            }
        };
        let integer = |key, low: f64, high: f64| -> Result<usize> {
            let value = request
                .get::<f64>(key)?
                .ok_or_else(|| Error::new(Status::InvalidArg, "qualification value missing"))?;
            if !value.is_finite() || value.fract() != 0.0 || value < low || value > high {
                return Err(Error::new(
                    Status::InvalidArg,
                    "qualification integer range",
                ));
            }
            Ok(value as usize)
        };
        let bins = integer("bins", 8.0, 32.0)?;
        let mode = if integer("mode", 0.0, 1.0)? == 0 {
            crate::engine::baseline_v2::BaselineV2Mode::Deterministic
        } else {
            crate::engine::baseline_v2::BaselineV2Mode::Stochastic
        };
        let cadence = request
            .get::<f64>("cadence")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "qualification cadence missing"))?;
        let baseline = crate::engine::baseline_v2::BaselineV2Settings {
            mode,
            control_interval_seconds: cadence,
        };
        baseline
            .validate()
            .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        let ray_work = crate::engine::ray_work::RayWorkLimits::new(
            integer("rayMinimum", 0.0, 1_000_000.0)? as u32,
            integer("rayCap", 1.0, 1_000_000.0)? as u32,
            integer("rayBatch", 1.0, 10_000_000.0)? as u32,
            512,
        )
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        let workload_population = if request.has_own_property("workload")? {
            Some(
                match bounded_object_string(&request, "workload", 2)?.as_str() {
                    "P0" => 55,
                    "P1" => 300,
                    _ => {
                        return Err(Error::new(
                            Status::InvalidArg,
                            "qualification workload must be P0/P1",
                        ))
                    }
                },
            )
        } else {
            None
        };
        let trained_cohort = if request.has_own_property("trainedCohort")? {
            if workload_population.is_none() {
                return Err(Error::new(
                    Status::InvalidArg,
                    "trained cohort requires P0/P1 workload",
                ));
            }
            Some(PathBuf::from(bounded_object_string(
                &request,
                "trainedCohort",
                32_768,
            )?))
        } else {
            None
        };
        let query_limits = match (
            request.has_own_property("pelletChecks")?,
            request.has_own_property("segmentChecks")?,
        ) {
            (false, false) => None,
            (true, true) if workload_population.is_some() => Some(
                crate::engine::sensor_qualification::QueryDetailLimits::new(
                    integer("pelletChecks", 100.0, 3000.0)?,
                    integer("segmentChecks", 200.0, 4000.0)?,
                )
                .map_err(|error| Error::new(Status::InvalidArg, error))?,
            ),
            _ => return Err(Error::new(
                Status::InvalidArg,
                "query detail limits require both pelletChecks/segmentChecks and a P0/P1 workload",
            )),
        };
        self.begin_operation(FRESH_OPERATION_INITIALIZE)?;
        if let Err(error) = ensure_fresh_transition_absent(&self.inner) {
            self.active_operation
                .store(FRESH_OPERATION_IDLE, Ordering::Release);
            return Err(error);
        }
        Ok(AsyncTask::new(InitializeQualifiedFreshRunTask {
            inner: InitializeExperimentalFreshRunTask {
                #[cfg(feature = "engine-test-hooks")]
                trained_cohort_json: None,
                #[cfg(feature = "engine-test-hooks")]
                capacity_audit_json: None,
                request: self.request.clone(),
                calculation_workers: self.calculation_workers,
                requested_math: None,
                source: FreshRunInitialization::Qualification(
                    profile,
                    bins,
                    baseline,
                    ray_work,
                    workload_population,
                    trained_cohort,
                    query_limits,
                ),
                inner: Arc::clone(&self.inner),
                active_operation: Arc::clone(&self.active_operation),
            },
        }))
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

/// Return the exact experimental engine command/event contract version.
#[napi(js_name = "experimentalEngineContractVersion", catch_unwind)]
pub fn experimental_engine_contract_version() -> u32 {
    ENGINE_CONTRACT_VERSION
}

mod wire_shapes;
pub use wire_shapes::*;
mod fresh_run_tasks;
#[cfg(feature = "engine-test-hooks")]
#[allow(unused_imports)]
pub use fresh_run_tasks::InitializeQualifiedFreshRunTask;
use fresh_run_tasks::*;
#[allow(unused_imports)]
pub use fresh_run_tasks::{
    ActivateExperimentalFreshRunTask, AdoptRecoveryBranchTask, CreateExperimentalBackgroundTask,
    InitializeExperimentalFreshRunTask, PublishExperimentalFreshRunCheckpointTask,
    PublishExperimentalFreshRunFirstScheduledFrameTask,
    PublishExperimentalFreshRunInitialFrameTask,
};
#[cfg(feature = "engine-test-hooks")]
mod test_hook_sessions;
#[cfg(feature = "engine-test-hooks")]
#[allow(unused_imports)] // Keep the existing Rust facade for JS-registered fixture exports.
pub use test_hook_sessions::*;
mod fresh_run_helpers;
use fresh_run_helpers::*;
mod checkpoint_wire;
use checkpoint_wire::*;
#[allow(unused_imports)]
pub(crate) use checkpoint_wire::{
    bounded_js_string, bounded_object_string, checkpoint_descriptor_from_napi,
    checkpoint_descriptor_from_napi_object, checkpoint_descriptor_to_napi,
    hall_of_fame_weights_descriptor_from_napi, hall_of_fame_weights_descriptor_to_napi,
    parse_background_sequence, parse_checkpoint_operation_id,
    parse_managed_checkpoint_publication_options, parse_managed_path, parse_numeric_encoding,
    parse_u64_hex, u64_hex,
};
mod event_wire;
#[allow(unused_imports)]
pub(super) use event_wire::running_authority_event_to_napi;
#[cfg(any(test, feature = "engine-test-hooks"))]
use event_wire::*;
#[allow(unused_imports)]
pub(crate) use event_wire::{
    background_generation_event_to_napi, background_generation_health_to_napi,
    generation_commit_to_napi,
};
mod coarse_probe_bridge;
use coarse_probe_bridge::*;
#[allow(unused_imports)]
pub(crate) use coarse_probe_bridge::{engine_error_to_napi, positive_usize};
#[allow(unused_imports)]
pub use coarse_probe_bridge::{ExperimentalRustEngine, JoinEngineTask};

#[cfg(test)]
mod tests;
