//! Production-addon handle for one Rust-owned background authority.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    compose_export_archive, prepare_import_archive, validate_import_archive,
    ExportArchiveDescriptor, ExportInventoryDescriptor, PreparedImportArchive,
    ValidatedImportArchive,
};
use crate::engine::fresh_run::{
    prepare_stage6a_p0_fresh_run_with_live_settings, stage6a_p0_export_validation_contract,
    Stage6aP0FreshRunRequest,
};
use crate::engine::run_start::PendingRunStartTransition;
use crate::engine::runtime::EngineRuntime;
use crate::napi_engine::{
    background_generation_event_to_napi, background_generation_health_to_napi, bounded_js_string,
    bounded_object_string, checkpoint_descriptor_from_napi_object, checkpoint_descriptor_to_napi,
    engine_error_to_napi, hall_of_fame_weights_descriptor_from_napi, parse_background_sequence,
    parse_checkpoint_operation_id, parse_managed_checkpoint_publication_options,
    parse_managed_path, parse_u64_hex, positive_usize, u64_hex, JoinEngineTask,
    ManagedHallOfFameWeightsDescriptor, Stage6BackgroundGenerationDrain,
    Stage6BackgroundGenerationHealth,
};

/// Cached frame chronology and basic stats; no population or world objects.
#[napi(object)]
pub struct BackgroundDisplayStatus {
    pub sequence: String,
    pub world_epoch: String,
    pub completed_step: String,
    pub generation: String,
    pub generation_time: f64,
    pub alive_population: u32,
    pub baseline_bots_alive: u32,
    pub baseline_bots_total: u32,
    pub total_snakes: u32,
    pub alive_snakes: u32,
    pub pellets: u32,
    pub frame_byte_length: f64,
}

pub(crate) fn display_status_to_napi(status: RunningDisplayStatus) -> BackgroundDisplayStatus {
    BackgroundDisplayStatus {
        sequence: u64_hex(status.sequence),
        world_epoch: u64_hex(status.world_epoch),
        completed_step: u64_hex(status.completed_step),
        generation: u64_hex(status.frame.generation),
        generation_time: status.generation_time,
        alive_population: status.alive_population as u32,
        baseline_bots_alive: status.baseline_bots_alive as u32,
        baseline_bots_total: status.baseline_bots_total as u32,
        total_snakes: status.frame.total_snakes as u32,
        alive_snakes: status.frame.alive_snakes as u32,
        pellets: status.frame.pellets as u32,
        frame_byte_length: status.frame.byte_length as f64,
    }
}

/// A caller can retry busy/too-small copies without consuming the retained frame.
#[napi(object)]
pub struct BackgroundFrameCopy {
    pub status: String,
    pub display: Option<BackgroundDisplayStatus>,
}

/// One ordered layer returned only for the selected Rust brain.
#[napi(object)]
pub struct BackgroundVisualizationLayer {
    pub count: u32,
    pub has_activations: bool,
    pub activations: Vec<f64>,
    pub is_recurrent: Option<bool>,
}

/// Replaceable complete focused neural snapshot.
#[napi(object)]
pub struct BackgroundVisualization {
    pub sequence: String,
    pub world_epoch: String,
    pub completed_step: String,
    pub snake_id: u32,
    pub kind: String,
    pub layers: Vec<BackgroundVisualizationLayer>,
}

fn visualization_to_napi(status: RunningVisualizationStatus) -> Result<BackgroundVisualization> {
    let mut values = status.values.iter().copied();
    let mut layers = Vec::new();
    layers.try_reserve_exact(status.layers.len()).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "cannot allocate visualization layers",
        )
    })?;
    for layer in status.layers {
        let count = u32::try_from(layer.count).map_err(|_| {
            Error::new(Status::GenericFailure, "visualization layer exceeds Uint32")
        })?;
        let activations = if layer.has_activations {
            let mut output = Vec::new();
            output.try_reserve_exact(layer.count).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "cannot allocate visualization values",
                )
            })?;
            for _ in 0..layer.count {
                let value = values.next().ok_or_else(|| {
                    Error::new(Status::GenericFailure, "visualization values ended early")
                })?;
                if !value.is_finite() {
                    return Err(Error::new(
                        Status::GenericFailure,
                        "visualization contains a non-finite activation",
                    ));
                }
                output.push(f64::from(value));
            }
            output
        } else {
            Vec::new()
        };
        layers.push(BackgroundVisualizationLayer {
            count,
            has_activations: layer.has_activations,
            activations,
            is_recurrent: layer.recurrent.then_some(true),
        });
    }
    if values.next().is_some() {
        return Err(Error::new(
            Status::GenericFailure,
            "visualization contains extra activation values",
        ));
    }
    Ok(BackgroundVisualization {
        sequence: u64_hex(status.sequence),
        world_epoch: u64_hex(status.world_epoch),
        completed_step: u64_hex(status.completed_step),
        snake_id: status.frame_v1_id,
        kind: "graph".to_owned(),
        layers,
    })
}

/// Bounded ready-file facts returned after Rust completes export composition.
#[napi(object)]
pub struct PreparedExportArchive {
    pub operation_id: String,
    pub checkpoint_id: String,
    pub relative_filename: String,
    pub download_filename: String,
    pub stored_byte_count: String,
    pub logical_root_sha256: String,
}

/// Small immutable facts returned after a complete untrusted archive validation.
#[napi(object)]
pub struct ValidatedImportArchiveResult {
    pub run_id: String,
    pub generation: String,
    pub completed_step: String,
    pub checkpoint_id: String,
    pub save_logical_root_sha256: String,
    pub history_count: String,
    pub hall_of_fame_count: String,
    pub stored_byte_count: String,
}

/// Small prepared-import facts. The private candidate remains retained in Rust.
#[napi(object)]
pub struct PreparedImportArchiveResult {
    pub run_id: String,
    pub generation: String,
    pub completed_step: String,
    pub checkpoint_id: String,
    pub save_logical_root_sha256: String,
    pub history_count: String,
    pub hall_of_fame_count: String,
    pub stored_byte_count: String,
    pub descriptor: crate::napi_engine::ManagedCheckpointDescriptor,
    pub inventory: PreparedImportInventoryResult,
    pub startup_metadata: String,
}

/// Trusted fixed-width import inventory consumed only by the SQLite worker.
#[napi(object)]
pub struct PreparedImportInventoryResult {
    pub version: u32,
    pub relative_filename: String,
    pub sha256: String,
    pub stored_byte_count: String,
    pub history_count: String,
    pub hall_of_fame_count: String,
}

/// Small fresh replacement facts. The complete candidate remains in Rust.
#[napi(object)]
pub struct PreparedFreshRunResult {
    pub descriptor: crate::napi_engine::ManagedCheckpointDescriptor,
    pub startup_metadata: String,
}

/// Complete off-loop result before the transition enters the shared slot.
pub struct PreparedFreshRun {
    transition: PendingRunStartTransition,
    descriptor: CheckpointDescriptor,
    startup_metadata_json: String,
}

/// Libuv task for constructing and publishing a private generation-one run.
pub struct PrepareFreshRunTask {
    managed_directory: PathBuf,
    operation_id: CheckpointOperationId,
    request: Stage6aP0FreshRunRequest,
    live_settings: Box<[crate::engine::live_settings::LiveSettingUpdate]>,
    prepared: PreparedImportSlot,
    active: Arc<AtomicBool>,
}

impl Task for PrepareFreshRunTask {
    type Output = PreparedFreshRun;
    type JsValue = PreparedFreshRunResult;

    fn compute(&mut self) -> Result<Self::Output> {
        let mut transition = prepare_stage6a_p0_fresh_run_with_live_settings(
            self.request.clone(),
            &self.live_settings,
        )
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
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
    managed_directory: PathBuf,
    operation_id: String,
    checkpoint: crate::engine::checkpoint::CheckpointDescriptor,
    inventory: ExportInventoryDescriptor,
}

impl Task for PrepareExportArchiveTask {
    type Output = ExportArchiveDescriptor;
    type JsValue = PreparedExportArchive;

    fn compute(&mut self) -> Result<Self::Output> {
        let memory_ceiling = usize::try_from(4u64 * 1024 * 1024 * 1024).map_err(|_| {
            Error::new(
                Status::GenericFailure,
                "P0 export memory ceiling exceeds usize",
            )
        })?;
        let (checkpoint_limits, graph_limits, admission_policy) =
            stage6a_p0_export_validation_contract(memory_ceiling)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
        compose_export_archive(
            &self.managed_directory,
            &self.operation_id,
            &self.checkpoint,
            &self.inventory,
            &checkpoint_limits,
            &graph_limits,
            &admission_policy,
        )
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
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
}

/// Libuv task for validating an untrusted upload without touching authority.
pub struct ValidateImportArchiveTask {
    archive_path: PathBuf,
    scratch_directory: PathBuf,
    operation_id: String,
}

/// Libuv preparation task retaining its admitted candidate in the native handle.
pub struct PrepareImportArchiveTask {
    archive_path: PathBuf,
    scratch_directory: PathBuf,
    managed_directory: PathBuf,
    operation_id: String,
    prepared: PreparedImportSlot,
    active: Arc<AtomicBool>,
}

impl Task for PrepareImportArchiveTask {
    type Output = PreparedImportArchive;
    type JsValue = PreparedImportArchiveResult;

    fn compute(&mut self) -> Result<Self::Output> {
        let memory_ceiling = usize::try_from(4u64 * 1024 * 1024 * 1024).map_err(|_| {
            Error::new(
                Status::GenericFailure,
                "P0 import memory ceiling exceeds usize",
            )
        })?;
        let (checkpoint_limits, graph_limits, admission_policy) =
            stage6a_p0_export_validation_contract(memory_ceiling)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
        prepare_import_archive(
            &self.archive_path,
            &self.scratch_directory,
            &self.managed_directory,
            &self.operation_id,
            &checkpoint_limits,
            &graph_limits,
            &admission_policy,
            memory_ceiling,
        )
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
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
            inventory: PreparedImportInventoryResult {
                version: inventory.version,
                relative_filename: inventory.relative_filename,
                sha256: inventory.sha256,
                stored_byte_count: inventory.stored_byte_count_hex,
                history_count: inventory.history_count_hex,
                hall_of_fame_count: inventory.hall_of_fame_count_hex,
            },
            startup_metadata,
        })
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active.store(false, Ordering::Release);
        Ok(())
    }
}

impl Task for ValidateImportArchiveTask {
    type Output = ValidatedImportArchive;
    type JsValue = ValidatedImportArchiveResult;

    fn compute(&mut self) -> Result<Self::Output> {
        let memory_ceiling = usize::try_from(4u64 * 1024 * 1024 * 1024).map_err(|_| {
            Error::new(
                Status::GenericFailure,
                "P0 import memory ceiling exceeds usize",
            )
        })?;
        let (checkpoint_limits, graph_limits, admission_policy) =
            stage6a_p0_export_validation_contract(memory_ceiling)
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
}

/// The fresh-run session can create this handle only by transferring its sole
/// activated authority. JavaScript cannot construct it or supply a world.
#[napi]
pub struct ExperimentalRunningAuthority {
    runtime: Arc<EngineRuntime>,
    drain_active: AtomicBool,
    join_scheduled: Arc<AtomicBool>,
    prepared_import: PreparedImportSlot,
    import_active: Arc<AtomicBool>,
}

impl ExperimentalRunningAuthority {
    pub(crate) fn from_runtime(runtime: Arc<EngineRuntime>) -> Self {
        Self {
            runtime,
            drain_active: AtomicBool::new(false),
            join_scheduled: Arc::new(AtomicBool::new(false)),
            prepared_import: PreparedImportSlot::new(),
            import_active: Arc::new(AtomicBool::new(false)),
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
}

#[napi]
impl ExperimentalRunningAuthority {
    /// Start the retained coordinator after Node has attached its output router.
    #[napi(catch_unwind)]
    pub fn start(&self) -> Result<()> {
        self.root(|| self.runtime.start().map_err(engine_error_to_napi))
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
        Ok(AsyncTask::new(PrepareExportArchiveTask {
            managed_directory,
            operation_id: operation_id.as_str().to_owned(),
            checkpoint,
            inventory,
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
        Ok(AsyncTask::new(ValidateImportArchiveTask {
            archive_path,
            scratch_directory,
            operation_id: operation_id.as_str().to_owned(),
        }))
    }

    /// Construct and publish one private fixed-P0 generation-one replacement.
    /// The running game and SQLite remain unchanged until later commands commit it.
    #[napi(catch_unwind)]
    pub fn prepare_fresh_run(
        &self,
        managed_directory: JsString<'_>,
        operation_id: JsString<'_>,
        run_id: JsString<'_>,
        seed: u32,
        live_settings: Array<'_>,
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
        let live_settings = parse_live_settings(&live_settings)?;
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
        let memory_ceiling_bytes = match usize::try_from(4u64 * 1024 * 1024 * 1024) {
            Ok(value) => value,
            Err(_) => {
                self.import_active.store(false, Ordering::Release);
                return Err(Error::new(
                    Status::GenericFailure,
                    "P0 fresh-run memory ceiling exceeds usize",
                ));
            }
        };
        Ok(AsyncTask::new(PrepareFreshRunTask {
            managed_directory,
            operation_id,
            request: Stage6aP0FreshRunRequest {
                run_id,
                seed,
                memory_ceiling_bytes,
            },
            live_settings,
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
        Ok(AsyncTask::new(PrepareImportArchiveTask {
            archive_path,
            scratch_directory,
            managed_directory,
            operation_id: operation_id.as_str().to_owned(),
            prepared: self.prepared_import.clone(),
            active: Arc::clone(&self.import_active),
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
            RunningAuthorityCommand::SubmitControllerAction(parse_controller_action(&action)?),
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

    /// Queue one browser-addressed God Mode translation at the next clean boundary.
    #[napi(catch_unwind)]
    pub fn submit_god_mode_move(
        &self,
        sequence: JsString<'_>,
        snake_id: u32,
        x: f64,
        y: f64,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::GodModeMove {
                frame_v1_id: snake_id,
                x,
                y,
            },
        )
    }

    /// Queue one browser-addressed God Mode death through ordinary side effects.
    #[napi(catch_unwind)]
    pub fn submit_god_mode_kill(&self, sequence: JsString<'_>, snake_id: u32) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::GodModeKill {
                frame_v1_id: snake_id,
            },
        )
    }

    /// Queue an aggregate subscriber toggle for focused neural capture.
    #[napi(catch_unwind)]
    pub fn submit_visualization(&self, sequence: JsString<'_>, enabled: bool) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SetVisualization { enabled },
        )
    }

    /// Queue one exact retained winner for Rust-owned decoding and resurrection.
    #[napi(catch_unwind)]
    pub fn submit_hall_of_fame_resurrection(
        &self,
        sequence: JsString<'_>,
        managed_directory: JsString<'_>,
        weights: ManagedHallOfFameWeightsDescriptor,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ResurrectHallOfFame {
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
    pub fn health(&self) -> Result<Stage6BackgroundGenerationHealth> {
        self.root(|| {
            background_generation_health_to_napi(self.runtime.health())
                .map_err(engine_error_to_napi)
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

/// Parse the metadata worker's exact bounded export inventory descriptor.
fn parse_export_inventory_descriptor(
    descriptor: &Object<'_>,
    operation_id: &str,
) -> Result<ExportInventoryDescriptor> {
    const KEYS: [&str; 6] = [
        "version",
        "relativeFilename",
        "sha256",
        "storedByteCount",
        "historyCount",
        "hallOfFameCount",
    ];
    let names = descriptor.get_property_names()?;
    if names.get_array_length()? != KEYS.len() as u32 {
        return Err(Error::new(
            Status::InvalidArg,
            "export inventory has unknown or missing fields",
        ));
    }
    let mut seen = [false; KEYS.len()];
    for index in 0..names.get_array_length()? {
        let key = names.get_element::<JsString<'_>>(index)?;
        let key = bounded_js_string(key, "export inventory key", 32, false)?;
        let position = KEYS
            .iter()
            .position(|expected| *expected == key)
            .ok_or_else(|| {
                Error::new(
                    Status::InvalidArg,
                    "export inventory contains an unknown field",
                )
            })?;
        if seen[position] || !descriptor.has_own_property(KEYS[position])? {
            return Err(Error::new(
                Status::InvalidArg,
                "export inventory has inherited or duplicate fields",
            ));
        }
        seen[position] = true;
    }
    let version = descriptor
        .get::<f64>("version")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "export inventory omits version"))?;
    if version != 1.0 {
        return Err(Error::new(
            Status::InvalidArg,
            "export inventory version is unsupported",
        ));
    }
    let relative_filename = bounded_object_string(
        descriptor,
        "relativeFilename",
        32 + ".export-inventory-v1".len() + 1,
    )?;
    if relative_filename != format!(".{operation_id}.export-inventory-v1") {
        return Err(Error::new(
            Status::InvalidArg,
            "export inventory does not match the operation ID",
        ));
    }
    Ok(ExportInventoryDescriptor {
        version: 1,
        relative_filename,
        sha256: bounded_object_string(descriptor, "sha256", 64)?,
        stored_byte_count_hex: bounded_object_string(descriptor, "storedByteCount", 16)?,
        history_count_hex: bounded_object_string(descriptor, "historyCount", 16)?,
        hall_of_fame_count_hex: bounded_object_string(descriptor, "hallOfFameCount", 16)?,
    })
}

/// Parse one optional bounded identity without materializing an unbounded string.
fn optional_reclaim_identity(request: &Object<'_>, field: &str, maximum: usize) -> Result<String> {
    match request.get::<JsString<'_>>(field)? {
        Some(value) => bounded_js_string(value, field, maximum, false),
        None => Ok(String::new()),
    }
}

/// Bound and copy one small numeric settings array before native queue admission.
fn parse_live_settings(
    updates: &Array<'_>,
) -> Result<Box<[crate::engine::live_settings::LiveSettingUpdate]>> {
    let length = usize::try_from(updates.len())
        .map_err(|_| Error::new(Status::InvalidArg, "live settings length exceeds usize"))?;
    if length == 0 || length > crate::engine::live_settings::MAXIMUM_LIVE_SETTING_UPDATES {
        return Err(Error::new(
            Status::InvalidArg,
            "live settings require 1 to 64 updates",
        ));
    }
    let mut parsed = Vec::new();
    parsed
        .try_reserve_exact(length)
        .map_err(|_| Error::new(Status::GenericFailure, "live settings allocation failed"))?;
    for index in 0..updates.len() {
        let update = updates
            .get::<Object<'_>>(index)?
            .ok_or_else(|| Error::new(Status::InvalidArg, "live setting must be an object"))?;
        let path = bounded_object_string(&update, "path", 128)?;
        let value = update
            .get::<f64>("value")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "live setting omits value"))?;
        if !value.is_finite() {
            return Err(Error::new(
                Status::InvalidArg,
                "live setting value must be finite",
            ));
        }
        parsed.push(crate::engine::live_settings::LiveSettingUpdate { path, value });
    }
    Ok(parsed.into_boxed_slice())
}

/// Validate a fresh legacy identity and stamp its native receipt time.
pub(crate) fn parse_controller_join(
    request: &Object<'_>,
) -> Result<crate::engine::contract::ControllerJoinRequest> {
    let kind = match bounded_object_string(request, "controllerKind", 32)?.as_str() {
        "player" => crate::engine::state::ControllerKind::Player,
        "reinforcementLearning" => crate::engine::state::ControllerKind::ReinforcementLearning,
        _ => return Err(Error::new(Status::InvalidArg, "invalid controllerKind")),
    };
    Ok(crate::engine::contract::ControllerJoinRequest {
        connection_id: parse_u64_hex(
            &bounded_object_string(request, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        kind,
        identity_key: bounded_object_string(request, "identityKey", 128)?,
        received_at: std::time::Instant::now(),
    })
}

/// Correlate a token or legacy identity and stamp its native receipt time.
pub(crate) fn parse_controller_reclaim(
    request: &Object<'_>,
) -> Result<crate::engine::contract::ControllerReclaimRequest> {
    let kind = match bounded_object_string(request, "controllerKind", 32)?.as_str() {
        "player" => crate::engine::state::ControllerKind::Player,
        "reinforcementLearning" => crate::engine::state::ControllerKind::ReinforcementLearning,
        _ => return Err(Error::new(Status::InvalidArg, "invalid controllerKind")),
    };
    Ok(crate::engine::contract::ControllerReclaimRequest {
        connection_id: parse_u64_hex(
            &bounded_object_string(request, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        kind,
        resume_token: optional_reclaim_identity(request, "resumeToken", 256)?,
        identity_key: optional_reclaim_identity(request, "identityKey", 128)?,
        received_at: std::time::Instant::now(),
    })
}

/// Validate exact reclaim receipt correlation before native queue admission.
pub(crate) fn parse_reclaim_receipt(
    receipt: &Object<'_>,
) -> Result<crate::engine::contract::ControllerReclaimReceipt> {
    Ok(crate::engine::contract::ControllerReclaimReceipt {
        request_sequence: parse_u64_hex(
            &bounded_object_string(receipt, "requestSequence", 16)?,
            "requestSequence",
            false,
        )?,
        connection_id: parse_u64_hex(
            &bounded_object_string(receipt, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        lease_id: parse_u64_hex(
            &bounded_object_string(receipt, "leaseId", 16)?,
            "leaseId",
            false,
        )?,
        accepted: receipt
            .get::<bool>("accepted")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "receipt omits accepted"))?,
    })
}

/// Validate a bounded exact close before stamping its native receipt time.
pub(crate) fn parse_controller_disconnect(
    close: &Object<'_>,
) -> Result<crate::engine::contract::ControllerDisconnectRequest> {
    let lease_id = parse_u64_hex(
        &bounded_object_string(close, "leaseId", 16)?,
        "leaseId",
        false,
    )?;
    let connection_id = parse_u64_hex(
        &bounded_object_string(close, "connectionId", 16)?,
        "connectionId",
        false,
    )?;
    Ok(crate::engine::contract::ControllerDisconnectRequest {
        lease_id,
        connection_id,
        received_at: std::time::Instant::now(),
    })
}

/// Validate wire values before recording the action's Rust-owned receipt time.
pub(crate) fn parse_controller_action(
    action: &Object<'_>,
) -> Result<crate::engine::contract::ControllerActionRequest> {
    let lease_id = parse_u64_hex(
        &bounded_object_string(action, "leaseId", 16)?,
        "leaseId",
        false,
    )?;
    let connection_id = parse_u64_hex(
        &bounded_object_string(action, "connectionId", 16)?,
        "connectionId",
        false,
    )?;
    let client_tick = parse_u64_hex(
        &bounded_object_string(action, "clientTick", 16)?,
        "clientTick",
        true,
    )?;
    let turn = action
        .get::<f64>("turn")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "action omits turn"))?;
    if !turn.is_finite() || !(-1.0..=1.0).contains(&turn) {
        return Err(Error::new(
            Status::InvalidArg,
            "action turn must be finite in [-1, 1]",
        ));
    }
    let boost = action
        .get::<bool>("boost")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "action omits boost"))?;
    Ok(crate::engine::contract::ControllerActionRequest {
        lease_id,
        connection_id,
        turn: turn as f32,
        boost,
        client_tick,
        received_at: std::time::Instant::now(),
    })
}

/// Parse bounded correlation fields before admitting one ordinary send result.
pub(crate) fn parse_controller_receipt(receipt: &Object<'_>) -> Result<ExternalDeliveryReceipt> {
    Ok(ExternalDeliveryReceipt {
        operation_epoch: parse_u64_hex(
            &bounded_object_string(receipt, "operationEpoch", 16)?,
            "operationEpoch",
            false,
        )?,
        event_sequence: parse_u64_hex(
            &bounded_object_string(receipt, "eventSequence", 16)?,
            "eventSequence",
            false,
        )?,
        connection_id: parse_u64_hex(
            &bounded_object_string(receipt, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        lease_id: parse_u64_hex(
            &bounded_object_string(receipt, "leaseId", 16)?,
            "leaseId",
            false,
        )?,
        accepted: receipt
            .get::<bool>("accepted")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "receipt omits accepted"))?,
    })
}
