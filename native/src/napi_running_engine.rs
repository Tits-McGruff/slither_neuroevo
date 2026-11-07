//! Production-addon handle for one Rust-owned background authority.

use crate::engine::population_work::PopulationWorkLease;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
#[cfg(feature = "engine-test-hooks")]
use std::sync::atomic::AtomicU8;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::{Array, AsyncTask, JsObjectValue, Object, Task};
use napi::{Env, Error, JsString, JsValue, Result, Status};
use napi_derive::napi;

use crate::engine::checkpoint::{CheckpointDescriptor, CheckpointOperationId};
use crate::engine::contract::{
    CommandBatch, EngineCommand, ExternalDeliveryReceipt, PreparedImportSlot,
    RunningAuthorityCommand, SequencedCommand, ENGINE_CONTRACT_VERSION,
};
use crate::engine::display::{FrameCopyResult, RunningDisplayStatus, RunningVisualizationStatus};
use crate::engine::error::{EngineError, EngineErrorCode};
use crate::engine::export_archive::{
    compose_export_archive_with_saved_brains, estimate_import_disk_bytes, prepare_import_archive,
    validate_import_archive, ExportArchiveDescriptor, ExportInventoryDescriptor,
    ImportDiskEstimate, PreparedImportArchive, ValidatedImportArchive,
};
use crate::engine::fresh_run::{
    prepare_configured_fresh_run_with_math, stage6a_p0_archive_validation_contract,
    FreshRunSettingUpdate, Stage6aP0FreshRunRequest,
};
use crate::engine::graph::GraphSpec;
use crate::engine::run_start::PendingRunStartTransition;
use crate::engine::runtime::EngineRuntime;
use crate::engine::work_progress::{ArchivePhaseTrace, ProgressScope};
use crate::napi_engine::{
    background_generation_event_to_napi, background_generation_health_to_napi, bounded_js_string,
    bounded_object_string, checkpoint_descriptor_from_napi_object, checkpoint_descriptor_to_napi,
    engine_error_to_napi, hall_of_fame_weights_descriptor_from_napi, parse_background_sequence,
    parse_checkpoint_operation_id, parse_managed_checkpoint_publication_options,
    parse_managed_path, parse_u64_hex, positive_usize, u64_hex, JoinEngineTask,
    ManagedHallOfFameWeightsDescriptor, Stage6BackgroundGenerationDrain,
    Stage6BackgroundGenerationHealth,
};

mod wire_shapes;
use wire_shapes::*;
#[allow(unused_imports)]
pub(crate) use wire_shapes::{display_status_to_napi, import_inventory_to_napi};
#[allow(unused_imports)]
pub use wire_shapes::{
    ArchivePhaseDiagnostics, ArchivePhaseTiming, ArchiveWorkProgress, BackgroundDisplayStatus,
    BackgroundEvolvedLifeStatus, BackgroundEvolvedPlacement, BackgroundFrameCopy,
    BackgroundVisualization, BackgroundVisualizationLayer, ImportDiskEstimateResult,
    PreparedExportArchive, PreparedFreshRunResult, PreparedImportArchiveResult,
    PreparedImportInventoryResult, ValidatedImportArchiveResult,
};
mod request_decode;
use request_decode::*;
#[allow(unused_imports)]
pub(crate) use request_decode::{
    parse_controller_action, parse_controller_disconnect, parse_controller_join,
    parse_controller_receipt, parse_controller_reclaim, parse_reclaim_receipt,
};

struct ArchiveProgressJob {
    operation_id: String,
    kind: &'static str,
    completed_bytes: Arc<AtomicU64>,
    started: AtomicBool,
    finished: AtomicBool,
    phase_trace: Option<Arc<ArchivePhaseTrace>>,
}

impl ArchiveProgressJob {
    fn new(operation_id: String, kind: &'static str) -> Self {
        Self {
            operation_id,
            kind,
            completed_bytes: Arc::new(AtomicU64::new(0)),
            started: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            phase_trace: (std::env::var_os("SLITHER_TRACE_ARCHIVE_PHASES").as_deref()
                == Some(std::ffi::OsStr::new("1")))
            .then(|| Arc::new(ArchivePhaseTrace::new())),
        }
    }

    fn snapshot(&self) -> ArchiveWorkProgress {
        ArchiveWorkProgress {
            operation_id: self.operation_id.clone(),
            kind: self.kind.to_owned(),
            completed_bytes: u64_hex(self.completed_bytes.load(Ordering::Relaxed)),
            started: self.started.load(Ordering::Acquire),
            finished: self.finished.load(Ordering::Acquire),
            phase_trace: self.phase_trace.as_ref().map(|trace| {
                let (elapsed, truncated, intervals) = trace.snapshot();
                ArchivePhaseDiagnostics {
                    elapsed_micros: u64_hex(elapsed),
                    truncated,
                    rss_sampler_started: trace.rss_sampler_started.load(Ordering::Acquire),
                    requested_rss_sample_interval_micros: u64_hex(
                        crate::engine::work_progress::ARCHIVE_RSS_SAMPLE_INTERVAL_MICROS,
                    ),
                    intervals: intervals
                        .into_iter()
                        .map(|interval| ArchivePhaseTiming {
                            phase: interval.phase.name().to_owned(),
                            started_micros: u64_hex(interval.started_micros),
                            finished_micros: interval.finished_micros.map(u64_hex),
                            start_rss_bytes: interval.start_rss_bytes.map(u64_hex),
                            finish_rss_bytes: interval.finish_rss_bytes.map(u64_hex),
                            sampled_peak_rss_bytes: interval.sampled_peak_rss_bytes.map(u64_hex),
                            rss_samples: u64_hex(interval.rss_samples),
                        })
                        .collect(),
                }
            }),
        }
    }
}

/// Complete off-loop result before the transition enters the shared slot.
pub struct PreparedFreshRun {
    transition: PendingRunStartTransition,
    descriptor: CheckpointDescriptor,
    startup_metadata_json: String,
}

/// Keep a Rust panic inside one libuv task instead of unwinding through N-API.
/// The task's `finally` hook still releases its busy/progress marker.
fn catch_background_task_panic<T>(
    task: &'static str,
    runtime: Option<&EngineRuntime>,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(_) => {
            if let Some(runtime) = runtime {
                runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::Faulted,
                    format!("{task} panicked at the background task root"),
                ));
            }
            Err(Error::new(
                Status::GenericFailure,
                format!("{task} panicked; background authority faulted"),
            ))
        }
    }
}

/// A probe has no concurrent world. Every owned-world task requires a live lease.
fn population_task_ceiling(
    runtime: &EngineRuntime,
    lease: &Option<PopulationWorkLease>,
    full: usize,
) -> Result<usize> {
    match lease {
        Some(lease) => Ok(lease.admission.remaining_bytes),
        None if !runtime.owns_authoritative_state() => Ok(full),
        None => Err(Error::new(
            Status::GenericFailure,
            "stage population work before background allocation",
        )),
    }
}

/// Libuv task for constructing and publishing a private generation-one run.
pub struct PrepareFreshRunTask {
    population_work: Option<PopulationWorkLease>,
    runtime: Arc<EngineRuntime>,
    managed_directory: PathBuf,
    operation_id: CheckpointOperationId,
    request: Stage6aP0FreshRunRequest,
    calculation_workers: usize,
    settings: Box<[FreshRunSettingUpdate]>,
    graph: GraphSpec,
    prepared: PreparedImportSlot,
    active: Arc<AtomicBool>,
}

impl Task for PrepareFreshRunTask {
    type Output = PreparedFreshRun;
    type JsValue = PreparedFreshRunResult;

    fn compute(&mut self) -> Result<Self::Output> {
        catch_background_task_panic("fresh-run preparation", Some(&self.runtime), || {
            let mut request = self.request.clone();
            request.memory_ceiling_bytes = population_task_ceiling(
                &self.runtime,
                &self.population_work,
                self.request.memory_ceiling_bytes,
            )?;
            let math = self
                .population_work
                .as_ref()
                .and_then(|lease| lease.admission.math_backend)
                .unwrap_or(crate::engine::inference::InferenceMathBackend::Scalar);
            let mut transition = prepare_configured_fresh_run_with_math(
                request,
                &self.settings,
                self.graph.clone(),
                math,
            )
            .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            transition
                .select_retired_source_memory_ceiling(self.request.memory_ceiling_bytes)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            transition
                .configure_calculation_workers(self.calculation_workers)
                .map_err(|error| Error::new(Status::GenericFailure, error))?;
            let descriptor = transition
                .publish_checkpoint(&self.managed_directory, self.operation_id.clone())
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            let startup_metadata_json = transition
                .startup_metadata_json()
                .map_err(|error| Error::new(Status::GenericFailure, error))?;
            Ok(PreparedFreshRun {
                transition,
                descriptor,
                startup_metadata_json,
            })
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        self.prepared
            .put(output.transition)
            .map_err(|_| Error::new(Status::GenericFailure, "prepared replacement slot changed"))?;
        Ok(PreparedFreshRunResult {
            descriptor: checkpoint_descriptor_to_napi(output.descriptor),
            startup_metadata: output.startup_metadata_json,
        })
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active.store(false, Ordering::Release);
        Ok(())
    }
}

/// Libuv task for file/codec work that must not block the Node event loop.
pub struct PrepareExportArchiveTask {
    population_work: Option<PopulationWorkLease>,
    runtime: Arc<EngineRuntime>,
    memory_ceiling_bytes: usize,
    managed_directory: PathBuf,
    operation_id: String,
    checkpoint: crate::engine::checkpoint::CheckpointDescriptor,
    inventory: ExportInventoryDescriptor,
    saved_brains: Vec<crate::engine::saved_brain_archive::SavedBrainArchiveEntryV1>,
    progress: Arc<ArchiveProgressJob>,
    #[cfg(feature = "engine-test-hooks")]
    export_failure: Arc<AtomicU8>,
}

impl Task for PrepareExportArchiveTask {
    type Output = ExportArchiveDescriptor;
    type JsValue = PreparedExportArchive;

    fn compute(&mut self) -> Result<Self::Output> {
        catch_background_task_panic("archive export", Some(&self.runtime), || {
            #[cfg(feature = "engine-test-hooks")]
            let _failure = crate::engine::export_failure_fixture::ExportFailureScope::enter(
                self.export_failure.swap(0, Ordering::AcqRel),
            );
            self.progress.started.store(true, Ordering::Release);
            let _progress = ProgressScope::enter_with_trace(
                Arc::clone(&self.progress.completed_bytes),
                self.progress.phase_trace.clone(),
            );
            let memory_ceiling = population_task_ceiling(
                &self.runtime,
                &self.population_work,
                self.memory_ceiling_bytes,
            )?;
            let (checkpoint_limits, graph_limits, admission_policy) =
                stage6a_p0_archive_validation_contract(memory_ceiling, false)
                    .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            compose_export_archive_with_saved_brains(
                &self.managed_directory,
                &self.operation_id,
                &self.checkpoint,
                &self.inventory,
                &checkpoint_limits,
                &graph_limits,
                &admission_policy,
                &self.saved_brains,
            )
            .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(PreparedExportArchive {
            operation_id: output.operation_id,
            checkpoint_id: output.checkpoint_id,
            relative_filename: output.relative_filename,
            download_filename: output.download_filename,
            stored_byte_count: output.stored_byte_count_hex,
            logical_root_sha256: output.logical_root_sha256,
        })
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.progress.finished.store(true, Ordering::Release);
        Ok(())
    }
}

/// Libuv task for validating an untrusted upload without touching authority.
pub struct ValidateImportArchiveTask {
    population_work: Option<PopulationWorkLease>,
    runtime: Arc<EngineRuntime>,
    memory_ceiling_bytes: usize,
    archive_path: PathBuf,
    scratch_directory: PathBuf,
    operation_id: String,
    progress: Arc<ArchiveProgressJob>,
}

/// Bounded off-loop USTAR-header and final-manifest inspection.
pub struct EstimateImportDiskTask {
    runtime: Arc<EngineRuntime>,
    archive_path: PathBuf,
}

impl Task for EstimateImportDiskTask {
    type Output = ImportDiskEstimate;
    type JsValue = ImportDiskEstimateResult;

    fn compute(&mut self) -> Result<Self::Output> {
        catch_background_task_panic("import disk estimate", Some(&self.runtime), || {
            estimate_import_disk_bytes(&self.archive_path)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(ImportDiskEstimateResult {
            candidate_spool_bytes: u64_hex(output.candidate_spool_bytes),
            final_managed_bytes: u64_hex(output.final_managed_bytes),
        })
    }
}

/// Libuv preparation task retaining its admitted candidate in the native handle.
pub struct PrepareImportArchiveTask {
    population_work: Option<PopulationWorkLease>,
    runtime: Arc<EngineRuntime>,
    memory_ceiling_bytes: usize,
    archive_path: PathBuf,
    scratch_directory: PathBuf,
    managed_directory: PathBuf,
    operation_id: String,
    legacy_run_id: String,
    legacy_seed: u32,
    calculation_workers: usize,
    prepared: PreparedImportSlot,
    active: Arc<AtomicBool>,
    progress: Arc<ArchiveProgressJob>,
}

impl Task for PrepareImportArchiveTask {
    type Output = PreparedImportArchive;
    type JsValue = PreparedImportArchiveResult;

    fn compute(&mut self) -> Result<Self::Output> {
        catch_background_task_panic("archive import preparation", Some(&self.runtime), || {
            self.progress.started.store(true, Ordering::Release);
            let _progress = ProgressScope::enter_with_trace(
                Arc::clone(&self.progress.completed_bytes),
                self.progress.phase_trace.clone(),
            );
            let memory_ceiling = population_task_ceiling(
                &self.runtime,
                &self.population_work,
                self.memory_ceiling_bytes,
            )?;
            let (checkpoint_limits, graph_limits, admission_policy) =
                stage6a_p0_archive_validation_contract(memory_ceiling, true)
                    .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            let mut prepared = prepare_import_archive(
                &self.archive_path,
                &self.scratch_directory,
                &self.managed_directory,
                &self.operation_id,
                &self.legacy_run_id,
                self.legacy_seed,
                &checkpoint_limits,
                &graph_limits,
                &admission_policy,
                self.memory_ceiling_bytes,
            )
            .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            prepared
                .transition
                .configure_calculation_workers(self.calculation_workers)
                .map_err(|error| Error::new(Status::GenericFailure, error))?;
            Ok(prepared)
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        let facts = output.facts;
        let descriptor = output.descriptor;
        let inventory = output.inventory;
        let startup_metadata = output.startup_metadata_json;
        self.prepared
            .put(output.transition)
            .map_err(|_| Error::new(Status::GenericFailure, "prepared import slot changed"))?;
        Ok(PreparedImportArchiveResult {
            run_id: facts.run_id,
            generation: facts.generation_hex,
            completed_step: facts.completed_step_hex,
            checkpoint_id: facts.checkpoint_id,
            save_logical_root_sha256: facts.save_logical_root_sha256,
            history_count: facts.history_count_hex,
            hall_of_fame_count: facts.hall_of_fame_count_hex,
            stored_byte_count: facts.stored_byte_count_hex,
            descriptor: checkpoint_descriptor_to_napi(descriptor),
            inventory: import_inventory_to_napi(inventory)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?,
            startup_metadata,
        })
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active.store(false, Ordering::Release);
        self.progress.finished.store(true, Ordering::Release);
        Ok(())
    }
}

impl Task for ValidateImportArchiveTask {
    type Output = ValidatedImportArchive;
    type JsValue = ValidatedImportArchiveResult;

    fn compute(&mut self) -> Result<Self::Output> {
        catch_background_task_panic("archive import validation", Some(&self.runtime), || {
            self.progress.started.store(true, Ordering::Release);
            let _progress = ProgressScope::enter_with_trace(
                Arc::clone(&self.progress.completed_bytes),
                self.progress.phase_trace.clone(),
            );
            let memory_ceiling = population_task_ceiling(
                &self.runtime,
                &self.population_work,
                self.memory_ceiling_bytes,
            )?;
            let (checkpoint_limits, graph_limits, admission_policy) =
                stage6a_p0_archive_validation_contract(memory_ceiling, true)
                    .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            validate_import_archive(
                &self.archive_path,
                &self.scratch_directory,
                &self.operation_id,
                &checkpoint_limits,
                &graph_limits,
                &admission_policy,
            )
            .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(ValidatedImportArchiveResult {
            run_id: output.run_id,
            generation: output.generation_hex,
            completed_step: output.completed_step_hex,
            checkpoint_id: output.checkpoint_id,
            save_logical_root_sha256: output.save_logical_root_sha256,
            history_count: output.history_count_hex,
            hall_of_fame_count: output.hall_of_fame_count_hex,
            stored_byte_count: output.stored_byte_count_hex,
        })
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.progress.finished.store(true, Ordering::Release);
        Ok(())
    }
}

/// The fresh-run session can create this handle only by transferring its sole
/// activated authority. JavaScript cannot construct it or supply a world.
#[napi]
pub struct ExperimentalRunningAuthority {
    runtime: Arc<EngineRuntime>,
    calculation_workers: usize,
    /// Admission ceiling inherited from the session that owns the running authority.
    memory_ceiling_bytes: usize,
    drain_active: AtomicBool,
    join_scheduled: Arc<AtomicBool>,
    prepared_import: PreparedImportSlot,
    import_active: Arc<AtomicBool>,
    archive_progress: Mutex<Option<Arc<ArchiveProgressJob>>>,
    #[cfg(feature = "engine-test-hooks")]
    export_failure: Arc<AtomicU8>,
}

impl ExperimentalRunningAuthority {
    pub(crate) fn from_runtime(
        runtime: Arc<EngineRuntime>,
        calculation_workers: usize,
        memory_ceiling_bytes: usize,
    ) -> Self {
        Self {
            runtime,
            calculation_workers,
            memory_ceiling_bytes,
            drain_active: AtomicBool::new(false),
            join_scheduled: Arc::new(AtomicBool::new(false)),
            prepared_import: PreparedImportSlot::new(),
            import_active: Arc::new(AtomicBool::new(false)),
            archive_progress: Mutex::new(None),
            #[cfg(feature = "engine-test-hooks")]
            export_failure: Arc::new(AtomicU8::new(0)),
        }
    }

    fn submit(&self, sequence: u64, command: RunningAuthorityCommand) -> Result<()> {
        self.root(|| {
            self.runtime
                .try_submit(CommandBatch {
                    contract_version: ENGINE_CONTRACT_VERSION,
                    commands: vec![SequencedCommand {
                        sequence,
                        command: EngineCommand::RunningAuthority(command),
                    }]
                    .into_boxed_slice(),
                })
                .map_err(engine_error_to_napi)
        })
    }

    fn root<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(result) => result,
            Err(_) => {
                self.runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::Faulted,
                    "panic at background authority N-API boundary",
                ));
                Err(Error::new(
                    Status::GenericFailure,
                    "background authority faulted",
                ))
            }
        }
    }

    fn acquire_population_work(
        &self,
        allow_unstarted: bool,
    ) -> Result<Option<PopulationWorkLease>> {
        if !self.runtime.owns_authoritative_state() {
            return Ok(None);
        }
        if allow_unstarted
            && self.runtime.health().lifecycle == crate::engine::LifecycleState::Created
        {
            let health = self.runtime.health();
            let epoch = health
                .running_authority
                .as_ref()
                .ok_or_else(|| {
                    Error::new(Status::GenericFailure, "population source is unavailable")
                })?
                .world_epoch;
            let source = self
                .runtime
                .authoritative_state_memory_bytes()
                .ok_or_else(|| {
                    Error::new(
                        Status::GenericFailure,
                        "population source memory is unavailable",
                    )
                })?;
            return self
                .prepared_import
                .acquire_unstarted_work(self.memory_ceiling_bytes, source, epoch)
                .map(Some)
                .map_err(|detail| Error::new(Status::GenericFailure, detail));
        }
        self.prepared_import
            .acquire_work(self.memory_ceiling_bytes)
            .map(Some)
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    fn begin_archive_job(
        &self,
        operation_id: String,
        kind: &'static str,
    ) -> Result<Arc<ArchiveProgressJob>> {
        let mut current = self
            .archive_progress
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "archive progress lock is poisoned"))?;
        if current
            .as_ref()
            .is_some_and(|job| !job.finished.load(Ordering::Acquire))
        {
            return Err(Error::new(
                Status::GenericFailure,
                "another native archive job is still running",
            ));
        }
        let job = Arc::new(ArchiveProgressJob::new(operation_id, kind));
        *current = Some(Arc::clone(&job));
        Ok(job)
    }
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl ExperimentalRunningAuthority {
    /// Read bounded hexadecimal quality counters from published ordinary observations.
    #[napi(catch_unwind)]
    pub fn observation_quality_for_test(&self) -> Result<String> {
        self.root(|| {
            self.runtime
                .observation_quality_for_test()
                .map_err(engine_error_to_napi)
        })
    }

    /// Test-addon-only one-shot failure for the next export on this handle.
    #[napi(catch_unwind)]
    pub fn arm_export_failure_for_test(&self, mode: u32) -> Result<()> {
        if !(1..=3).contains(&mode) {
            return Err(Error::new(
                Status::InvalidArg,
                "export failure mode must be 1, 2 or 3",
            ));
        }
        self.export_failure
            .compare_exchange(0, mode as u8, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                Error::new(Status::GenericFailure, "an export failure is already armed")
            })?;
        Ok(())
    }

    /// Test-addon-only trigger; production addons expose no panic injection method.
    #[napi(catch_unwind)]
    pub fn arm_calculation_panic_for_test(&self) -> Result<()> {
        self.root(|| {
            self.runtime
                .arm_calculation_panic_for_test()
                .map_err(engine_error_to_napi)
        })
    }
}

#[napi]
impl ExperimentalRunningAuthority {
    /// Read native archive bytes completed without waiting on its worker thread.
    #[napi(catch_unwind)]
    pub fn archive_work_progress(&self) -> Result<Option<ArchiveWorkProgress>> {
        self.root(|| {
            let current = self.archive_progress.lock().map_err(|_| {
                Error::new(Status::GenericFailure, "archive progress lock is poisoned")
            })?;
            Ok(current.as_ref().map(|job| job.snapshot()))
        })
    }

    /// Start the retained coordinator after Node has attached its output router.
    #[napi(catch_unwind)]
    pub fn start(&self) -> Result<()> {
        self.root(|| {
            if self.prepared_import.has_admission() {
                return Err(Error::new(
                    Status::GenericFailure,
                    "population work must finish before coordinator startup",
                ));
            }
            self.runtime.start().map_err(engine_error_to_napi)
        })
    }

    /// Queue immutable publication with bounded, server-controlled inputs.
    #[napi(catch_unwind)]
    pub fn submit_generation_checkpoint(
        &self,
        sequence: JsString<'_>,
        options: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let (directory, operation_id) = parse_managed_checkpoint_publication_options(&options)?;
        self.submit(
            sequence,
            RunningAuthorityCommand::PublishGenerationCheckpoint {
                managed_directory: directory
                    .to_str()
                    .ok_or_else(|| {
                        Error::new(Status::InvalidArg, "managed directory must be UTF-8")
                    })?
                    .to_owned(),
                operation_id,
            },
        )
    }

    /// Build one exact leased checkpoint export on libuv's worker pool.
    #[napi(catch_unwind)]
    pub fn prepare_export_archive(
        &self,
        managed_directory: JsString<'_>,
        operation_id: JsString<'_>,
        checkpoint: Object<'_>,
        inventory: Object<'_>,
        saved_brains_json: Option<JsString<'_>>,
    ) -> Result<AsyncTask<PrepareExportArchiveTask>> {
        let managed_directory = parse_managed_path(bounded_js_string(
            managed_directory,
            "managedDirectory",
            32 * 1024,
            false,
        )?)?;
        let operation_id = parse_checkpoint_operation_id(bounded_js_string(
            operation_id,
            "operationId",
            32,
            false,
        )?)?;
        let checkpoint = checkpoint_descriptor_from_napi_object(&checkpoint)?;
        let inventory = parse_export_inventory_descriptor(&inventory, operation_id.as_str())?;
        let saved_brains = match saved_brains_json {
            None => Vec::new(),
            Some(json) => serde_json::from_str::<
                Vec<crate::engine::saved_brain_archive::SavedBrainArchiveEntryV1>,
            >(&bounded_js_string(
                json,
                "savedBrainsJson",
                128 * 1024,
                false,
            )?)
            .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?,
        };
        crate::engine::saved_brain_archive::validate_selection(&saved_brains)
            .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
        let population_work = self.acquire_population_work(true)?;
        let progress = self.begin_archive_job(operation_id.as_str().to_owned(), "export")?;
        Ok(AsyncTask::new(PrepareExportArchiveTask {
            population_work,
            runtime: Arc::clone(&self.runtime),
            memory_ceiling_bytes: self.memory_ceiling_bytes,
            managed_directory,
            operation_id: operation_id.as_str().to_owned(),
            checkpoint,
            inventory,
            saved_brains,
            progress,
            #[cfg(feature = "engine-test-hooks")]
            export_failure: Arc::clone(&self.export_failure),
        }))
    }

    /// Fully validate one untrusted save archive without changing live or durable state.
    #[napi(catch_unwind)]
    pub fn validate_import_archive(
        &self,
        archive_path: JsString<'_>,
        scratch_directory: JsString<'_>,
        operation_id: JsString<'_>,
    ) -> Result<AsyncTask<ValidateImportArchiveTask>> {
        let archive_path = parse_managed_path(bounded_js_string(
            archive_path,
            "archivePath",
            32 * 1024,
            false,
        )?)?;
        let scratch_directory = parse_managed_path(bounded_js_string(
            scratch_directory,
            "scratchDirectory",
            32 * 1024,
            false,
        )?)?;
        let operation_id = parse_checkpoint_operation_id(bounded_js_string(
            operation_id,
            "operationId",
            32,
            false,
        )?)?;
        let population_work = self.acquire_population_work(true)?;
        let progress =
            self.begin_archive_job(operation_id.as_str().to_owned(), "validate-import")?;
        Ok(AsyncTask::new(ValidateImportArchiveTask {
            population_work,
            runtime: Arc::clone(&self.runtime),
            memory_ceiling_bytes: self.memory_ceiling_bytes,
            archive_path,
            scratch_directory,
            operation_id: operation_id.as_str().to_owned(),
            progress,
        }))
    }

    /// Inspect bounded archive metadata on a worker before admitting disk work.
    #[napi(catch_unwind)]
    pub fn estimate_import_disk(
        &self,
        archive_path: JsString<'_>,
    ) -> Result<AsyncTask<EstimateImportDiskTask>> {
        let archive_path = parse_managed_path(bounded_js_string(
            archive_path,
            "archivePath",
            32 * 1024,
            false,
        )?)?;
        Ok(AsyncTask::new(EstimateImportDiskTask {
            runtime: Arc::clone(&self.runtime),
            archive_path,
        }))
    }

    /// Construct and publish one private generation-one replacement.
    /// The running game and SQLite remain unchanged until later commands commit it.
    #[napi(catch_unwind)]
    pub fn prepare_fresh_run(
        &self,
        managed_directory: JsString<'_>,
        operation_id: JsString<'_>,
        run_id: JsString<'_>,
        seed: u32,
        settings: Array<'_>,
        graph_spec_json: JsString<'_>,
    ) -> Result<AsyncTask<PrepareFreshRunTask>> {
        let managed_directory = parse_managed_path(bounded_js_string(
            managed_directory,
            "managedDirectory",
            32 * 1024,
            false,
        )?)?;
        let operation_id = parse_checkpoint_operation_id(bounded_js_string(
            operation_id,
            "operationId",
            32,
            false,
        )?)?;
        let run_id = bounded_js_string(run_id, "runId", 256, false)?;
        let settings = parse_fresh_run_settings(&settings)?;
        let graph = parse_fresh_run_graph(graph_spec_json)?;
        if self.import_active.swap(true, Ordering::AcqRel) {
            return Err(Error::new(
                Status::GenericFailure,
                "another replacement preparation is already running",
            ));
        }
        if self.prepared_import.is_some() {
            self.import_active.store(false, Ordering::Release);
            return Err(Error::new(
                Status::GenericFailure,
                "another prepared replacement is awaiting its durability decision",
            ));
        }
        let memory_ceiling_bytes = self.memory_ceiling_bytes;
        let population_work = match self.acquire_population_work(false) {
            Ok(lease) => lease,
            Err(error) => {
                self.import_active.store(false, Ordering::Release);
                return Err(error);
            }
        };
        Ok(AsyncTask::new(PrepareFreshRunTask {
            population_work,
            runtime: Arc::clone(&self.runtime),
            managed_directory,
            operation_id,
            request: Stage6aP0FreshRunRequest {
                run_id,
                seed,
                memory_ceiling_bytes,
            },
            calculation_workers: self.calculation_workers,
            settings,
            graph,
            prepared: self.prepared_import.clone(),
            active: Arc::clone(&self.import_active),
        }))
    }

    /// Fully validate and retain an imported authority while publishing only
    /// its immutable managed checkpoint. Live state and SQLite remain unchanged.
    #[napi(catch_unwind)]
    pub fn prepare_import_archive(
        &self,
        archive_path: JsString<'_>,
        scratch_directory: JsString<'_>,
        managed_directory: JsString<'_>,
        operation_id: JsString<'_>,
        legacy_run_id: JsString<'_>,
        legacy_seed: u32,
    ) -> Result<AsyncTask<PrepareImportArchiveTask>> {
        let archive_path = parse_managed_path(bounded_js_string(
            archive_path,
            "archivePath",
            32 * 1024,
            false,
        )?)?;
        let scratch_directory = parse_managed_path(bounded_js_string(
            scratch_directory,
            "scratchDirectory",
            32 * 1024,
            false,
        )?)?;
        let managed_directory = parse_managed_path(bounded_js_string(
            managed_directory,
            "managedDirectory",
            32 * 1024,
            false,
        )?)?;
        let operation_id = parse_checkpoint_operation_id(bounded_js_string(
            operation_id,
            "operationId",
            32,
            false,
        )?)?;
        let legacy_run_id = bounded_js_string(legacy_run_id, "legacyRunId", 256, false)?;
        if self.import_active.swap(true, Ordering::AcqRel) {
            return Err(Error::new(
                Status::GenericFailure,
                "another import preparation is already running",
            ));
        }
        if self.prepared_import.is_some() {
            self.import_active.store(false, Ordering::Release);
            return Err(Error::new(
                Status::GenericFailure,
                "another prepared import is awaiting its durability decision",
            ));
        }
        let progress = match self.begin_archive_job(operation_id.as_str().to_owned(), "import") {
            Ok(job) => job,
            Err(error) => {
                self.import_active.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let population_work = match self.acquire_population_work(false) {
            Ok(lease) => lease,
            Err(error) => {
                self.import_active.store(false, Ordering::Release);
                progress.finished.store(true, Ordering::Release);
                return Err(error);
            }
        };
        Ok(AsyncTask::new(PrepareImportArchiveTask {
            population_work,
            runtime: Arc::clone(&self.runtime),
            memory_ceiling_bytes: self.memory_ceiling_bytes,
            archive_path,
            scratch_directory,
            managed_directory,
            operation_id: operation_id.as_str().to_owned(),
            legacy_run_id,
            legacy_seed,
            calculation_workers: self.calculation_workers,
            prepared: self.prepared_import.clone(),
            active: Arc::clone(&self.import_active),
            progress,
        }))
    }

    /// Drop one private candidate after upload validation or metadata commit
    /// fails. Published content-addressed files remain harmless and reusable.
    #[napi(catch_unwind)]
    pub fn discard_prepared_import(&self) -> Result<()> {
        if self.import_active.load(Ordering::Acquire) {
            return Err(Error::new(
                Status::GenericFailure,
                "import preparation is still running",
            ));
        }
        let removed = self.prepared_import.take();
        if removed.is_none() {
            return Err(Error::new(
                Status::GenericFailure,
                "no prepared import is retained",
            ));
        }
        Ok(())
    }

    /// Queue the pre-commit pause at the next untouched step boundary.
    #[napi(catch_unwind)]
    pub fn submit_stage_prepared_import(&self, sequence: JsString<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::StagePreparedImport {
                slot: self.prepared_import.clone(),
            },
        )
    }

    /// Resume the unchanged game after a failure before SQLite commits.
    #[napi(catch_unwind)]
    pub fn submit_cancel_prepared_import(&self, sequence: JsString<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::CancelPreparedImport {
                slot: self.prepared_import.clone(),
            },
        )
    }

    /// Queue the final import swap only after the SQLite worker has selected
    /// and returned the complete committed checkpoint descriptor.
    #[napi(catch_unwind)]
    pub fn submit_import_persistence_acknowledgement(
        &self,
        sequence: JsString<'_>,
        descriptor: Object<'_>,
        branch_run_id: Option<JsString<'_>>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        let branch_run_id = branch_run_id
            .map(|value| bounded_js_string(value, "branchRunId", 256, false))
            .transpose()?;
        self.submit(
            sequence,
            RunningAuthorityCommand::PublishPreparedImport {
                slot: self.prepared_import.clone(),
                descriptor: Box::new(descriptor),
                branch_run_id,
            },
        )
    }

    /// Queue the exact descriptor acknowledged by the dedicated SQLite worker.
    #[napi(catch_unwind)]
    pub fn submit_generation_persistence_acknowledgement(
        &self,
        sequence: JsString<'_>,
        descriptor: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        self.submit(
            sequence,
            RunningAuthorityCommand::AcknowledgeGenerationPersistence {
                descriptor: Box::new(descriptor),
            },
        )
    }

    /// Queue connected-controller reassignment at the durable generation barrier.
    #[napi(catch_unwind)]
    pub fn submit_prepare_generation_reassignments(&self, sequence: JsString<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::PrepareGenerationReassignments,
        )
    }

    /// Return one exact local-send result to the Rust-owned assignment barrier.
    #[napi(catch_unwind)]
    pub fn submit_generation_assignment_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let receipt = parse_controller_receipt(&receipt)?;
        self.submit(
            sequence,
            RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
                receipts: vec![receipt].into_boxed_slice(),
            },
        )
    }

    /// Stage an fresh controller join at an eligible source boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_join(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::JoinController(Box::new(parse_controller_join(&request)?)),
        )
    }

    /// Resolve the retained fresh join assignment without touching ordinary receipts.
    #[napi(catch_unwind)]
    pub fn submit_controller_join_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerJoinReceipt(parse_reclaim_receipt(&receipt)?),
        )
    }

    /// Stage an explicit token reclaim at an eligible source boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_reclaim(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ReclaimController(Box::new(parse_controller_reclaim(
                &request,
            )?)),
        )
    }

    /// Resolve the retained reclaim assignment without touching ordinary receipts.
    #[napi(catch_unwind)]
    pub fn submit_controller_reclaim_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerReclaimReceipt(parse_reclaim_receipt(
                &receipt,
            )?),
        )
    }

    /// Queue a socket close for the next eligible pre-step boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_disconnect(
        &self,
        sequence: JsString<'_>,
        close: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::DisconnectController(parse_controller_disconnect(&close)?),
        )
    }

    /// Queue steering for the next eligible pre-step boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_action(
        &self,
        sequence: JsString<'_>,
        action: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerAction(Box::new(parse_controller_action(
                &action,
            )?)),
        )
    }

    /// Queue one bounded atomic live-settings batch at the next clean boundary.
    #[napi(catch_unwind)]
    pub fn submit_live_settings(&self, sequence: JsString<'_>, updates: Array<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ApplyLiveSettings {
                updates: parse_live_settings(&updates)?,
            },
        )
    }

    /// Queue one bounded generation-scoped manual completion request.
    #[napi(catch_unwind)]
    pub fn submit_finish_generation(
        &self,
        sequence: JsString<'_>,
        request_id: JsString<'_>,
        expected_run_id: JsString<'_>,
        expected_generation: JsString<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::FinishGeneration {
                request_id: bounded_js_string(request_id, "requestId", 128, false)?,
                expected_run_id: bounded_js_string(expected_run_id, "expectedRunId", 256, false)?,
                expected_generation: parse_background_sequence(expected_generation)?,
            },
        )
    }

    /// Resolve a bounded frame/slot selector through the authoritative command queue.
    #[napi(catch_unwind)]
    pub fn submit_entity_selection(
        &self,
        sequence: JsString<'_>,
        target_kind: JsString<'_>,
        target_index: f64,
        expected_run_id: JsString<'_>,
        expected_generation: JsString<'_>,
    ) -> Result<()> {
        let kind = bounded_js_string(target_kind, "targetKind", 8, false)?;
        let target = match kind.as_str() {
            "frame" => crate::engine::state::EntitySelectionTarget::Frame(checked_entity_index(
                target_index,
                crate::engine::state::FRAME_V1_MAX_EXACT_ID,
                false,
            )?),
            "slot" => crate::engine::state::EntitySelectionTarget::PopulationSlot(
                checked_entity_index(target_index, 299, true)?,
            ),
            _ => return Err(napi::Error::from_reason("targetKind must be frame or slot")),
        };
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SelectEntity {
                target,
                expected_run_id: bounded_js_string(expected_run_id, "expectedRunId", 256, false)?,
                expected_generation: parse_background_sequence(expected_generation)?,
            },
        )
    }

    /// Admit or close one selected diagnostic cache through the ordinary command queue.
    #[napi(catch_unwind)]
    pub fn submit_inspector(
        &self,
        sequence: JsString<'_>,
        expected_run_id: JsString<'_>,
        token: Option<JsString<'_>>,
        frame_v1_id: Option<f64>,
        pure_probe: Option<bool>,
    ) -> Result<()> {
        let selection = match (token, frame_v1_id) {
            (None, None) => None,
            (Some(token), Some(frame)) => Some(Box::new((
                crate::engine::state::EntitySelectionToken::parse(&bounded_js_string(
                    token,
                    "selectionToken",
                    64,
                    false,
                )?)
                .map_err(napi::Error::from_reason)?,
                checked_entity_index(frame, crate::engine::state::FRAME_V1_MAX_EXACT_ID, false)?,
            ))),
            _ => {
                return Err(napi::Error::from_reason(
                    "inspector needs both token and frame ID, or neither",
                ))
            }
        };
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ConfigureInspector {
                expected_run_id: bounded_js_string(expected_run_id, "expectedRunId", 256, false)?,
                selection,
                pure_probe: pure_probe.unwrap_or(false),
            },
        )
    }

    /// Queue one browser-addressed God Mode translation at the next clean boundary.
    #[napi(catch_unwind)]
    pub fn submit_god_mode_move(
        &self,
        sequence: JsString<'_>,
        snake_id: f64,
        x: f64,
        y: f64,
        token: JsString<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::GodModeMove {
                token: Box::new(
                    crate::engine::state::EntitySelectionToken::parse(&bounded_js_string(
                        token,
                        "selectionToken",
                        64,
                        false,
                    )?)
                    .map_err(napi::Error::from_reason)?,
                ),
                frame_v1_id: checked_entity_index(
                    snake_id,
                    crate::engine::state::FRAME_V1_MAX_EXACT_ID,
                    false,
                )?,
                x,
                y,
            },
        )
    }

    /// Queue one browser-addressed God Mode death through ordinary side effects.
    #[napi(catch_unwind)]
    pub fn submit_god_mode_kill(
        &self,
        sequence: JsString<'_>,
        snake_id: f64,
        token: JsString<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::GodModeKill {
                token: Box::new(
                    crate::engine::state::EntitySelectionToken::parse(&bounded_js_string(
                        token,
                        "selectionToken",
                        64,
                        false,
                    )?)
                    .map_err(napi::Error::from_reason)?,
                ),
                frame_v1_id: checked_entity_index(
                    snake_id,
                    crate::engine::state::FRAME_V1_MAX_EXACT_ID,
                    false,
                )?,
            },
        )
    }

    /// Activate one cold-validated immutable shadow through the ordinary ordered command queue.
    #[napi(catch_unwind)]
    pub fn submit_activate_shadow(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ActivateShadow(Box::new(
                crate::napi_shadows::parse_activation(&request)?,
            )),
        )
    }

    /// Disable only the identified activation at a clean authoritative boundary.
    #[napi(catch_unwind)]
    pub fn submit_stop_shadow(&self, sequence: JsString<'_>, request: Object<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            crate::napi_shadows::parse_stop(&request)?,
        )
    }

    /// Read bounded published diagnostic summaries without world locks or teacher drains.
    #[napi(catch_unwind)]
    pub fn get_shadow_metrics(&self) -> Result<String> {
        self.root(|| {
            crate::napi_shadows::metrics_json(
                self.runtime
                    .shadow_snapshots()
                    .map_err(engine_error_to_napi)?,
            )
        })
    }

    /// Queue cold admission of automatic exact teacher rows and a separate bounded queue.
    #[napi(catch_unwind)]
    pub fn submit_start_teacher_capture(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::StartTeacherCapture(Box::new(
                crate::napi_teacher_capture::parse_request(&request)?,
            )),
        )
    }

    /// Seal one matching diagnostic epoch at the next clean boundary.
    #[napi(catch_unwind)]
    pub fn submit_stop_teacher_capture(
        &self,
        sequence: JsString<'_>,
        capture_epoch: JsString<'_>,
    ) -> Result<()> {
        let epoch = bounded_js_string(capture_epoch, "captureEpoch", 32, false)?;
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::StopTeacherCapture {
                capture_epoch: crate::napi_teacher_capture::parse_epoch(&epoch)?,
            },
        )
    }

    /// Drain complete opaque objects from the independent diagnostic queue.
    #[napi(catch_unwind)]
    pub fn drain_teacher_chunks(
        &self,
        max_chunks: f64,
        max_owned_bytes: f64,
    ) -> Result<crate::napi_teacher_capture::TeacherCaptureDrain> {
        self.root(|| {
            let chunks = positive_usize(max_chunks, "maxChunks").map_err(engine_error_to_napi)?;
            let bytes =
                positive_usize(max_owned_bytes, "maxOwnedBytes").map_err(engine_error_to_napi)?;
            self.runtime
                .drain_teacher_chunks(chunks, bytes)
                .map(crate::napi_teacher_capture::drain_to_napi)
                .map_err(engine_error_to_napi)
        })
    }

    /// Queue an aggregate subscriber toggle for focused neural capture.
    #[napi(catch_unwind)]
    pub fn submit_visualization(&self, sequence: JsString<'_>, enabled: bool) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SetVisualization { enabled },
        )
    }

    /// Queue exact library deployment; native admission owns graph/sensor/cadence checks.
    #[napi(catch_unwind)]
    pub fn submit_saved_brain_spawn(
        &self,
        sequence: JsString<'_>,
        managed_directory: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        let weights =
            request.get_named_property::<ManagedHallOfFameWeightsDescriptor>("weights")?;
        let input = crate::engine::brain_library::SavedBrainSpawnRequest {
            managed_directory: bounded_js_string(
                managed_directory,
                "managedDirectory",
                32768,
                false,
            )?,
            brain_id: bounded_object_string(&request, "brainId", 64)?,
            expected_run_id: bounded_object_string(&request, "expectedRunId", 256)?,
            expected_config_revision: parse_u64_hex(
                &bounded_object_string(&request, "expectedConfigRevision", 16)?,
                "expectedConfigRevision",
                false,
            )?,
            expected_world_epoch: parse_u64_hex(
                &bounded_object_string(&request, "expectedWorldEpoch", 16)?,
                "expectedWorldEpoch",
                false,
            )?,
            weights: Box::new(hall_of_fame_weights_descriptor_from_napi(weights)?),
        };
        input
            .validate()
            .map_err(|detail| Error::new(Status::InvalidArg, detail))?;
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SpawnSavedBrain(Box::new(input)),
        )
    }

    /// Queue one exact retained winner for Rust-owned decoding and resurrection.
    #[napi(catch_unwind)]
    pub fn submit_hall_of_fame_resurrection(
        &self,
        sequence: JsString<'_>,
        managed_directory: JsString<'_>,
        entry_id: JsString<'_>,
        weights: ManagedHallOfFameWeightsDescriptor,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ResurrectHallOfFame {
                entry_id: parse_u64_hex(
                    &bounded_js_string(entry_id, "entryId", 16, false)?,
                    "entryId",
                    false,
                )?,
                managed_directory: bounded_js_string(
                    managed_directory,
                    "managedDirectory",
                    32_768,
                    false,
                )?,
                weights: Box::new(hall_of_fame_weights_descriptor_from_napi(weights)?),
            },
        )
    }

    /// Resolve an ordinary observation/death-assignment send without touching a generation barrier.
    #[napi(catch_unwind)]
    pub fn submit_controller_delivery_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                receipts: vec![parse_controller_receipt(&receipt)?].into_boxed_slice(),
            },
        )
    }

    /// Queue the separately gated final generation swap.
    #[napi(catch_unwind)]
    pub fn submit_publish_generation_start(&self, sequence: JsString<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
        )
    }

    /// Drain prepared events without blocking on or inspecting the game world.
    #[napi(catch_unwind)]
    pub fn drain_outputs(
        &self,
        max_events: f64,
        max_owned_bytes: f64,
    ) -> Result<Stage6BackgroundGenerationDrain> {
        self.root(|| {
            let max_events =
                positive_usize(max_events, "maxEvents").map_err(engine_error_to_napi)?;
            let max_owned_bytes =
                positive_usize(max_owned_bytes, "maxOwnedBytes").map_err(engine_error_to_napi)?;
            if self.drain_active.swap(true, Ordering::AcqRel) {
                return Err(Error::new(
                    Status::GenericFailure,
                    "another background output drain is active",
                ));
            }
            let _guard = DrainGuard(&self.drain_active);
            let drained = self
                .runtime
                .drain_outputs(max_events, max_owned_bytes)
                .map_err(engine_error_to_napi)?;
            let events = drained
                .events
                .into_iter()
                .map(background_generation_event_to_napi)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| {
                    self.runtime.report_bridge_fault(error.clone());
                    engine_error_to_napi(error)
                })?;
            Ok(Stage6BackgroundGenerationDrain {
                events,
                more_work: drained.more_work,
                generation: u64_hex(drained.generation),
            })
        })
    }

    /// Read bounded health scalars from the background owner.
    #[napi(catch_unwind)]
    pub fn health(
        &self,
        include_step_timing_histogram: Option<bool>,
    ) -> Result<Stage6BackgroundGenerationHealth> {
        self.root(|| {
            background_generation_health_to_napi(
                self.runtime.health(),
                self.calculation_workers,
                include_step_timing_histogram.unwrap_or(false),
            )
            .map_err(engine_error_to_napi)
        })
    }

    /// Observe queue occupancy and peaks only when explicitly requested by diagnostics.
    #[napi(catch_unwind)]
    pub fn queue_diagnostics(
        &self,
    ) -> Result<crate::napi_queue_diagnostics::RuntimeQueueDiagnostics> {
        self.root(|| {
            Ok(crate::napi_queue_diagnostics::queue_diagnostics(
                &self.runtime,
            ))
        })
    }

    /// Read cached welcome metadata without repacking or waiting on the authority.
    #[napi(catch_unwind)]
    pub fn latest_display(&self) -> Result<Option<BackgroundDisplayStatus>> {
        self.root(|| {
            self.runtime
                .latest_display()
                .map(|status| status.map(display_status_to_napi))
                .map_err(engine_error_to_napi)
        })
    }

    /// Return one bounded Rust-authored projection; never sample or execute a model here.
    #[napi(catch_unwind)]
    pub fn latest_inspector(&self, after_sequence: JsString<'_>) -> Result<Option<String>> {
        let after_sequence = parse_u64_hex(
            &bounded_js_string(after_sequence, "afterSequence", 16, false)?,
            "afterSequence",
            true,
        )?;
        self.root(|| {
            self.runtime
                .latest_inspector(after_sequence)
                .map_err(engine_error_to_napi)?
                .map(|snapshot| {
                    crate::engine::inspector_wire::encode(&snapshot)
                        .map_err(napi::Error::from_reason)
                })
                .transpose()
        })
    }

    /// Copy only a newer cached focused snapshot; never inspect the live authority.
    #[napi(catch_unwind)]
    pub fn latest_visualization(
        &self,
        after_sequence: JsString<'_>,
    ) -> Result<Option<BackgroundVisualization>> {
        let after_sequence = parse_u64_hex(
            &bounded_js_string(after_sequence, "afterSequence", 16, false)?,
            "afterSequence",
            true,
        )?;
        self.root(|| {
            self.runtime
                .latest_visualization(after_sequence)
                .map_err(engine_error_to_napi)?
                .map(visualization_to_napi)
                .transpose()
        })
    }

    /// Copy into a non-shared Uint8Array owned by Node until socket sends complete.
    /// No JS callback runs while its mutable byte slice exists, and Rust retains none.
    #[napi(catch_unwind)]
    pub fn copy_latest_frame(
        &self,
        env: Env,
        destination: Object<'_>,
        after_sequence: JsString<'_>,
    ) -> Result<BackgroundFrameCopy> {
        let after_sequence = parse_u64_hex(
            &bounded_js_string(after_sequence, "afterSequence", 16, false)?,
            "afterSequence",
            true,
        )?;
        self.root(|| {
            let mut typed = false;
            // SAFETY: env and destination are live values on this N-API call's JS thread.
            napi::check_status!(unsafe {
                napi::sys::napi_is_typedarray(env.raw(), destination.raw(), &mut typed)
            })?;
            if !typed {
                return Err(Error::new(
                    Status::InvalidArg,
                    "destination must be a non-shared Uint8Array",
                ));
            }
            let mut kind = 0;
            let mut length = 0;
            let mut data = std::ptr::null_mut();
            let mut backing = std::ptr::null_mut();
            // SAFETY: the intrinsic typed-array query bypasses user properties. Returned
            // backing and data remain rooted by destination throughout this synchronous call.
            napi::check_status!(unsafe {
                napi::sys::napi_get_typedarray_info(
                    env.raw(),
                    destination.raw(),
                    &mut kind,
                    &mut length,
                    &mut data,
                    &mut backing,
                    std::ptr::null_mut(),
                )
            })?;
            let mut ordinary_buffer = false;
            // SAFETY: backing is the live intrinsic buffer returned above. SharedArrayBuffer
            // is not an ArrayBuffer, and must be rejected before constructing a Rust slice.
            napi::check_status!(unsafe {
                napi::sys::napi_is_arraybuffer(env.raw(), backing, &mut ordinary_buffer)
            })?;
            if kind != napi::sys::TypedarrayType::uint8_array
                || !ordinary_buffer
                || (length > 0 && data.is_null())
                || length > isize::MAX as usize
            {
                return Err(Error::new(
                    Status::InvalidArg,
                    "destination must be a non-shared Uint8Array",
                ));
            }
            let destination = if length == 0 {
                &mut []
            } else {
                // SAFETY: the Uint8Array owns length initialized bytes starting at data,
                // including its byte offset. Its non-shared backing cannot be concurrently
                // resized/written; no JS callbacks occur until this borrow ends. The source
                // is private Rust cache storage and cannot alias this destination.
                unsafe { std::slice::from_raw_parts_mut(data.cast::<u8>(), length) }
            };
            let result = self
                .runtime
                .copy_latest_frame(destination, after_sequence)
                .map_err(engine_error_to_napi)?;
            let (status, display) = match result {
                FrameCopyResult::Busy => ("busy", None),
                FrameCopyResult::Unchanged => ("unchanged", None),
                FrameCopyResult::TooSmall(display) => {
                    ("tooSmall", Some(display_status_to_napi(display)))
                }
                FrameCopyResult::Copied(display) => {
                    ("copied", Some(display_status_to_napi(display)))
                }
            };
            Ok(BackgroundFrameCopy {
                status: status.to_owned(),
                display,
            })
        })
    }

    /// Signal shutdown without blocking Node on authority work.
    #[napi(catch_unwind)]
    pub fn request_stop(&self) {
        self.runtime.request_stop();
    }

    /// Join the background coordinator on a libuv worker.
    #[napi(catch_unwind)]
    pub fn join(&self) -> Result<AsyncTask<JoinEngineTask>> {
        if self.join_scheduled.swap(true, Ordering::AcqRel) {
            return Err(Error::new(
                Status::GenericFailure,
                "background authority join already scheduled",
            ));
        }
        // Signal before libuv schedules the join, including a queue-capacity wait.
        self.runtime.request_stop();
        Ok(AsyncTask::new(JoinEngineTask::new(
            Arc::clone(&self.runtime),
            Arc::clone(&self.join_scheduled),
        )))
    }
}

impl Drop for ExperimentalRunningAuthority {
    fn drop(&mut self) {
        self.runtime.request_stop();
    }
}

struct DrainGuard<'guard>(&'guard AtomicBool);
impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod task_panic_tests;

#[cfg(all(test, feature = "engine-test-hooks"))]
mod inventory_wire_tests;
