#[test]
fn entity_selector_arguments_reject_coercion() {
    for value in [f64::NAN, f64::INFINITY, -1.0, 1.5, 16_777_217.0] {
        assert!(super::checked_entity_index(value, 16_777_216, false).is_err());
    }
    assert!(super::checked_entity_index(0.0, 16_777_216, false).is_err());
    assert_eq!(
        super::checked_entity_index(16_777_216.0, 16_777_216, false).unwrap(),
        16_777_216
    );
    assert_eq!(super::checked_entity_index(0.0, 299, true).unwrap(), 0);
    assert!(super::checked_entity_index(300.0, 299, true).is_err());
}

use super::{
    catch_background_task_panic, ArchiveProgressJob, ExperimentalRunningAuthority,
    PopulationWorkLease, PrepareExportArchiveTask, PrepareFreshRunTask, PrepareImportArchiveTask,
    ValidateImportArchiveTask,
};
use crate::engine::checkpoint::{CheckpointDescriptor, CheckpointOperationId};
use crate::engine::contract::{
    CommandBatch, EngineCommand, EngineInit, InboundLimits, OutputLimits, PreparedImportSlot,
    SequencedCommand, ENGINE_CONTRACT_VERSION,
};
use crate::engine::export_archive::{compose_export_archive, ExportInventoryDescriptor};
use crate::engine::fresh_run::{
    prepare_default_fresh_run, stage6a_p0_archive_validation_contract, Stage6aP0FreshRunRequest,
};
use crate::engine::graph::default_stack_graph_spec;
use crate::engine::queues::NoopWakeSink;
use crate::engine::runtime::EngineRuntime;
use crate::engine::task_panic_fixture::{PanicInjection, PanicPoint};
use crate::engine::work_progress::{advance, ProgressScope};
use napi::bindgen_prelude::Task;
use napi::{Error, Status};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Own every file used by one task test and remove the whole isolated root.
struct TaskFiles(PathBuf);

impl TaskFiles {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "slither-task-panic-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn request(&self) -> Stage6aP0FreshRunRequest {
        Stage6aP0FreshRunRequest {
            run_id: "private-task-panic-run".to_owned(),
            seed: 42,
            memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
        }
    }

    fn checkpoint(&self) -> CheckpointDescriptor {
        let mut transition = prepare_default_fresh_run(self.request()).unwrap();
        transition
            .publish_checkpoint(
                &self.0,
                CheckpointOperationId::parse("a".repeat(32)).unwrap(),
            )
            .unwrap()
    }

    fn inventory(&self, operation: &str) -> ExportInventoryDescriptor {
        let mut bytes = [0u8; 32];
        bytes[..13].copy_from_slice(b"SLITHER-EXPV1");
        let relative_filename = format!(".{operation}.export-inventory-v1");
        fs::write(self.0.join(&relative_filename), bytes).unwrap();
        ExportInventoryDescriptor {
            version: 1,
            relative_filename,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            stored_byte_count_hex: "0000000000000020".to_owned(),
            history_count_hex: "0000000000000000".to_owned(),
            hall_of_fame_count_hex: "0000000000000000".to_owned(),
        }
    }

    fn archive(&self, checkpoint: &CheckpointDescriptor) -> PathBuf {
        let operation = "b".repeat(32);
        let inventory = self.inventory(&operation);
        let (limits, graphs, admission) =
            stage6a_p0_archive_validation_contract(4 * 1024 * 1024 * 1024, false).unwrap();
        let archive = compose_export_archive(
            &self.0, &operation, checkpoint, &inventory, &limits, &graphs, &admission,
        )
        .unwrap();
        self.0.join(archive.relative_filename)
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TaskFiles {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("task-owned scratch must be removed");
    }
}

/// Assert the actual task root contains the panic and faults its retained owner.
fn assert_task_panic<T>(
    result: napi::Result<T>,
    task_name: &str,
    injection: &PanicInjection,
    runtime: &EngineRuntime,
) {
    assert!(
        injection.reached(),
        "real operation boundary must be reached"
    );
    let error = result.err().expect("task panic must reject its result");
    assert_eq!(error.status, Status::GenericFailure);
    assert!(error.reason.contains(&format!("{task_name} panicked")));
    assert!(!error.reason.contains("sensitive"));
    assert!(runtime
        .health()
        .fault
        .as_ref()
        .is_some_and(|fault| fault.detail().contains(task_name)));
    assert!(runtime
        .try_submit(CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::Probe {
                    correlation_id: 1,
                    payload: vec![1]
                },
            }]
            .into_boxed_slice(),
        })
        .is_err());
    runtime
        .join()
        .expect("faulted coordinator must join cleanly");
}

/// A subsequent job on the same worker must not inherit a panicked job's counter.
fn assert_progress_scope_released() {
    let counter = Arc::new(AtomicU64::new(0));
    let _scope = ProgressScope::enter(Arc::clone(&counter));
    advance(7);
    assert_eq!(counter.load(Ordering::Relaxed), 7);
}

#[test]
fn unstarted_archive_admission_counts_source_and_prevents_concurrent_start() {
    let files = TaskFiles::new();
    let mut pending = prepare_default_fresh_run(files.request()).unwrap();
    let descriptor = pending
        .publish_checkpoint(
            &files.0,
            CheckpointOperationId::parse("e".repeat(32)).unwrap(),
        )
        .unwrap();
    pending.acknowledge_persistence(&descriptor).unwrap();
    pending.publish_running_authority().unwrap();
    let running = pending
        .into_running_loop(
            crate::engine::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
            0,
        )
        .unwrap();
    let probe = running_test_runtime();
    let mut init = probe.queue_limits();
    init.output.max_event_owned_bytes = 4 * 1024 * 1024;
    init.output.max_reliable_owned_bytes = 8 * 1024 * 1024;
    init.output.max_discrete_owned_bytes = 8 * 1024 * 1024;
    init.output.max_total_owned_bytes = 16 * 1024 * 1024;
    probe.request_stop();
    probe.join().unwrap();
    let runtime = Arc::new(
        EngineRuntime::new_running_authority(init, running, Arc::new(NoopWakeSink))
            .unwrap_or_else(|error| panic!("running test runtime: {}", error.error())),
    );
    let handle =
        ExperimentalRunningAuthority::from_runtime(Arc::clone(&runtime), 1, 4 * 1024 * 1024 * 1024);
    assert!(handle.acquire_population_work(false).is_err());
    assert!(super::population_task_ceiling(&runtime, &None, 4 * 1024 * 1024 * 1024).is_err());
    let lease = handle.acquire_population_work(true).unwrap().unwrap();
    assert_eq!(
        lease.admission.remaining_bytes + runtime.authoritative_state_memory_bytes().unwrap(),
        4 * 1024 * 1024 * 1024
    );
    assert!(handle.start().is_err());
    assert_eq!(
        runtime.health().lifecycle,
        crate::engine::LifecycleState::Created
    );
    drop(lease);
    handle.start().unwrap();
    assert!(handle.acquire_population_work(true).is_err());
    runtime.request_stop();
    runtime.join().unwrap();
}

fn task_population_lease(
    runtime: &Arc<EngineRuntime>,
    slot: &PreparedImportSlot,
) -> Option<PopulationWorkLease> {
    if !runtime.owns_authoritative_state() {
        return None;
    }
    let full = 4 * 1024 * 1024 * 1024;
    slot.reserve_admission(full, runtime.authoritative_state_memory_bytes().unwrap(), 1)
        .unwrap();
    Some(slot.acquire_work(full).unwrap())
}

#[test]
fn fresh_run_task_panic_removes_written_checkpoint_and_keeps_candidate_private() {
    let files = TaskFiles::new();
    let runtime = running_test_runtime();
    let prepared = PreparedImportSlot::new();
    let mut task = PrepareFreshRunTask {
        founder_brain_id: None,
        population_work: task_population_lease(&runtime, &prepared),
        runtime: Arc::clone(&runtime),
        managed_directory: files.0.clone(),
        operation_id: CheckpointOperationId::parse("c".repeat(32)).unwrap(),
        request: files.request(),
        calculation_workers: 1,
        settings: Box::new([]),
        graph: default_stack_graph_spec(264),
        prepared: prepared.clone(),
        active: Arc::new(AtomicBool::new(true)),
    };
    let injection = PanicInjection::arm(PanicPoint::CheckpointWritten);
    assert_task_panic(
        task.compute(),
        "fresh-run preparation",
        &injection,
        &runtime,
    );
    assert!(!prepared.is_some());
    assert!(files.names().is_empty());
}

#[test]
fn export_task_panic_removes_written_archive_and_preserves_leased_inputs() {
    let files = TaskFiles::new();
    let checkpoint = files.checkpoint();
    let operation = "c".repeat(32);
    let inventory = files.inventory(&operation);
    let before = files.names();
    let retained = fs::read(files.0.join(&checkpoint.relative_filename)).unwrap();
    let runtime = running_test_runtime();
    let progress = Arc::new(ArchiveProgressJob::new(operation.clone(), "export"));
    let mut task = PrepareExportArchiveTask {
        population_work: task_population_lease(&runtime, &PreparedImportSlot::new()),
        runtime: Arc::clone(&runtime),
        memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
        managed_directory: files.0.clone(),
        operation_id: operation,
        checkpoint: checkpoint.clone(),
        inventory,
        saved_brains: Vec::new(),
        progress: Arc::clone(&progress),
        #[cfg(feature = "engine-test-hooks")]
        export_failure: Arc::new(std::sync::atomic::AtomicU8::new(0)),
    };
    let injection = PanicInjection::arm(PanicPoint::ExportWritten);
    assert_task_panic(task.compute(), "archive export", &injection, &runtime);
    assert!(progress.started.load(Ordering::Acquire));
    assert!(progress.completed_bytes.load(Ordering::Relaxed) > 0);
    assert_eq!(files.names(), before);
    assert_eq!(
        fs::read(files.0.join(&checkpoint.relative_filename)).unwrap(),
        retained
    );
    assert_progress_scope_released();
}

#[test]
fn import_validation_task_panic_removes_extracted_checkpoint_and_preserves_upload() {
    let files = TaskFiles::new();
    let checkpoint = files.checkpoint();
    let archive = files.archive(&checkpoint);
    let retained = fs::read(&archive).unwrap();
    let before = files.names();
    let runtime = running_test_runtime();
    let mut task = ValidateImportArchiveTask {
        population_work: task_population_lease(&runtime, &PreparedImportSlot::new()),
        runtime: Arc::clone(&runtime),
        memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
        archive_path: archive.clone(),
        scratch_directory: files.0.clone(),
        operation_id: "c".repeat(32),
        progress: Arc::new(ArchiveProgressJob::new("c".repeat(32), "validate")),
    };
    let injection = PanicInjection::arm(PanicPoint::ImportExtracted);
    assert_task_panic(
        task.compute(),
        "archive import validation",
        &injection,
        &runtime,
    );
    assert_eq!(files.names(), before);
    assert_eq!(fs::read(&archive).unwrap(), retained);
    assert_progress_scope_released();
}

#[test]
fn import_preparation_task_panic_removes_inventory_and_keeps_candidate_private() {
    let source = TaskFiles::new();
    let checkpoint = source.checkpoint();
    let archive = source.archive(&checkpoint);
    let target = TaskFiles::new();
    let retained = fs::read(&archive).unwrap();
    let runtime = running_test_runtime();
    let prepared = PreparedImportSlot::new();
    let mut task = PrepareImportArchiveTask {
        population_work: task_population_lease(&runtime, &prepared),
        runtime: Arc::clone(&runtime),
        memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
        archive_path: archive.clone(),
        scratch_directory: target.0.clone(),
        managed_directory: target.0.clone(),
        operation_id: "c".repeat(32),
        legacy_run_id: "unused-exact-import".to_owned(),
        legacy_seed: 0,
        calculation_workers: 1,
        prepared: prepared.clone(),
        active: Arc::new(AtomicBool::new(true)),
        progress: Arc::new(ArchiveProgressJob::new("c".repeat(32), "import")),
    };
    let injection = PanicInjection::arm(PanicPoint::ImportExtracted);
    assert_task_panic(
        task.compute(),
        "archive import preparation",
        &injection,
        &runtime,
    );
    assert!(!prepared.is_some());
    assert!(target.names().is_empty());
    assert_eq!(fs::read(&archive).unwrap(), retained);
    assert_progress_scope_released();
}

/// Real partial publication rejects before slot installation; panic also faults the owner.
/// A new task can reuse the valid unreferenced final without altering the original upload.
#[test]
fn import_preparation_task_partial_publication_rejects_and_retries() {
    use crate::engine::task_panic_fixture::PublicationIoErrorInjection;

    let source = TaskFiles::new();
    let checkpoint = source.checkpoint();
    let archive = source.archive(&checkpoint);
    let retained = fs::read(&archive).unwrap();
    let checkpoint_bytes = fs::read(source.0.join(&checkpoint.relative_filename)).unwrap();
    for panic in [false, true] {
        let target = TaskFiles::new();
        let prepared = PreparedImportSlot::new();
        let make_task = |runtime: &Arc<EngineRuntime>, operation: &str| PrepareImportArchiveTask {
            population_work: task_population_lease(runtime, &prepared),
            runtime: Arc::clone(runtime),
            memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
            archive_path: archive.clone(),
            scratch_directory: target.0.clone(),
            managed_directory: target.0.clone(),
            operation_id: operation.to_owned(),
            legacy_run_id: "unused-exact-import".to_owned(),
            legacy_seed: 0,
            calculation_workers: 1,
            prepared: prepared.clone(),
            active: Arc::new(AtomicBool::new(true)),
            progress: Arc::new(ArchiveProgressJob::new(operation.to_owned(), "import")),
        };
        let runtime = running_test_runtime();
        let mut task = make_task(&runtime, &"c".repeat(32));
        if panic {
            let injection = PanicInjection::arm(PanicPoint::ImportFilePublished);
            assert_task_panic(
                task.compute(),
                "archive import preparation",
                &injection,
                &runtime,
            );
        } else {
            let injection = PublicationIoErrorInjection::arm();
            let error = match task.compute() {
                Err(error) => error,
                Ok(_) => panic!("publication must reject"),
            };
            assert!(injection.reached());
            assert!(error
                .reason
                .contains("injected import publication I/O error"));
            assert!(runtime.health().fault.is_none());
            runtime.request_stop();
            runtime.join().unwrap();
        }
        assert!(!prepared.is_some());
        assert_eq!(target.names(), vec![checkpoint.relative_filename.clone()]);
        assert_eq!(fs::read(&archive).unwrap(), retained);
        assert_eq!(
            fs::read(target.0.join(&checkpoint.relative_filename)).unwrap(),
            checkpoint_bytes
        );
        assert_progress_scope_released();

        let restarted = running_test_runtime();
        let mut retry = make_task(&restarted, &"d".repeat(32));
        let result = retry
            .compute()
            .expect("independent retry must reuse the valid final");
        assert_eq!(
            result.descriptor.logical_root_sha256,
            checkpoint.logical_root_sha256
        );
        assert_eq!(target.names().len(), 2);
        assert!(target.0.join(&result.inventory.relative_filename).is_file());
        assert!(!prepared.is_some(), "compute cannot install the candidate");
        assert!(restarted.health().fault.is_none());
        restarted.request_stop();
        restarted.join().unwrap();
        assert_eq!(fs::read(&archive).unwrap(), retained);
        assert_progress_scope_released();
    }
}

fn running_test_runtime() -> Arc<EngineRuntime> {
    let runtime = EngineRuntime::new_experimental_probe(
        EngineInit {
            contract_version: ENGINE_CONTRACT_VERSION,
            inbound: InboundLimits {
                max_batches: 4,
                max_commands: 8,
                max_owned_bytes: 64,
                max_batch_commands: 4,
                max_batch_owned_bytes: 32,
            },
            output: OutputLimits {
                max_reliable: 8,
                max_reliable_owned_bytes: 64,
                max_discrete: 4,
                max_discrete_owned_bytes: 64,
                max_total_owned_bytes: 128,
                max_event_owned_bytes: 64,
                max_frame_connections: 4,
            },
        },
        Arc::new(NoopWakeSink),
    )
    .expect("valid test runtime");
    runtime.start().expect("coordinator must start");
    Arc::new(runtime)
}

#[test]
fn libuv_task_panic_faults_the_retained_engine() {
    let runtime = running_test_runtime();
    let result: napi::Result<()> =
        catch_background_task_panic("archive export", Some(&runtime), || {
            panic!("archive worker panic")
        });
    assert!(result.is_err());
    let health = runtime.health();
    assert!(health.fault.is_some());
    assert!(health
        .fault
        .as_ref()
        .is_some_and(|error| error.detail().contains("archive export")));
    assert!(runtime
        .try_submit(CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::Probe {
                    correlation_id: 1,
                    payload: vec![1],
                },
            }]
            .into_boxed_slice(),
        })
        .is_err());
    runtime
        .join()
        .expect("faulted coordinator must join cleanly");
}

#[test]
fn synchronous_napi_root_panic_faults_the_retained_engine() {
    let runtime = running_test_runtime();
    let handle =
        ExperimentalRunningAuthority::from_runtime(Arc::clone(&runtime), 1, 4 * 1024 * 1024 * 1024);
    let result: napi::Result<()> = handle.root(|| panic!("synchronous N-API root panic"));
    let error = result.expect_err("root panic must become an N-API error");
    assert_eq!(error.status, Status::GenericFailure);
    assert!(runtime.health().fault.is_some());
    assert!(runtime
        .health()
        .fault
        .as_ref()
        .is_some_and(|fault| fault.detail().contains("N-API boundary")));
    runtime
        .join()
        .expect("faulted coordinator must join cleanly");
}

#[test]
fn libuv_task_root_contains_panics_and_preserves_ordinary_errors() {
    let panic_result: napi::Result<()> =
        catch_background_task_panic("archive export", None, || panic!("sensitive panic payload"));
    let panic_error = panic_result.expect_err("panic must become a task failure");
    assert_eq!(panic_error.status, Status::GenericFailure);
    assert!(panic_error.reason.contains("archive export panicked"));
    assert!(!panic_error.reason.contains("sensitive panic payload"));

    let ordinary_result: napi::Result<()> =
        catch_background_task_panic("archive export", None, || {
            Err(Error::new(Status::InvalidArg, "bad archive"))
        });
    let ordinary_error = ordinary_result.expect_err("ordinary errors must survive");
    assert_eq!(ordinary_error.status, Status::InvalidArg);
    assert_eq!(ordinary_error.reason, "bad archive");
}
