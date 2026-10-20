//! Explicit offline checkpoint capacity candidates; production limits remain frozen.
use super::checkpoint::CheckpointLimits;
use super::fresh_run::FreshRunError;
use super::state::StateMemoryEstimate;
use serde::Serialize;

/// Only the reviewed current/candidate envelopes are available to this test-hook surface.
#[derive(Clone, Copy, Debug)]
pub struct CheckpointCapacityPolicy {
    envelope_mib: usize,
    raw_fallback: bool,
}
impl CheckpointCapacityPolicy {
    pub fn new(envelope_mib: usize, raw_fallback: bool) -> Result<Self, FreshRunError> {
        if !matches!(envelope_mib, 512 | 768) {
            return Err(FreshRunError::Settings(
                "capacity envelope must be 512 or 768 MiB".into(),
            ));
        }
        Ok(Self {
            envelope_mib,
            raw_fallback,
        })
    }
    /// Preserve small roles, shape limits and the ordinary candidate ceiling.
    /// Raw-fallback qualification only lowers the temporary candidate cap.
    pub(crate) fn limits(self) -> CheckpointLimits {
        let mut limits = super::fresh_run::production_checkpoint_limits();
        let bytes = (self.envelope_mib as u64) * 1024 * 1024;
        limits.max_archive_bytes = bytes;
        limits.max_numeric_stored_bytes = bytes;
        limits.max_total_decoded_bytes = bytes;
        if self.raw_fallback {
            limits.max_numeric_candidate_bytes = 1024 * 1024;
        }
        limits
    }
}

/// Exact metadata-only preflight, before any population numeric allocation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointCapacityAudit {
    pub population: usize,
    pub parameters_per_genome: usize,
    pub recurrent_per_brain: usize,
    pub role_names: [&'static str; 5],
    pub decoded_role_bytes: [u64; 5],
    pub decoded_bytes: u64,
    pub limits: CheckpointLimits,
    pub state_memory: StateMemoryEstimate,
    pub checkpoint_workspace_bytes: usize,
    pub checkpoint_publication_peak_bytes: usize,
    pub conservative_two_state_bytes: usize,
    pub minimum_successor_numeric_peak_bytes: usize,
    pub process_memory_ceiling_bytes: usize,
    pub decoded_envelope_fits: bool,
    pub two_state_budget_fits: bool,
    pub numeric_payload_allocated: bool,
    pub worker_scratch_bytes: usize,
    pub checkpoint_scratch_bytes: usize,
}

/// Exact native contracts accompanying the exclusively offline experiment.
pub(crate) struct QualifiedCheckpointRun {
    pub audit: CheckpointCapacityAudit,
    pub pending: Option<super::run_start::PendingRunStartTransition>,
    pub limits: CheckpointLimits,
    pub graph_limits: super::graph::GraphLimits,
    pub admission_policy: super::state::StateAdmissionPolicy,
    pub work_limits: super::step_config::RunningStepWorkLimits,
}

/// Fixed P0/P3-style native workload; callers select the full population and graph.
pub fn capacity_workload_options(
    population: usize,
    large_graph: bool,
) -> super::sensor_qualification::SensorQualificationOptions {
    super::sensor_qualification::SensorQualificationOptions {
        population,
        profile: super::sensor_contract::SensorProfile::Motion,
        bins: 32,
        large_graph,
        trained_artifact: None,
        respawn: true,
        seed: 42,
        workers: 3,
        math: super::inference::InferenceMathBackend::Sse2,
        ray_limits: Some(
            super::ray_work::RayWorkLimits::new(1, 256, 65536, 512)
                .expect("fixed qualification rays"),
        ),
        baseline: Some(super::baseline_v2::BaselineV2Settings {
            mode: super::baseline_v2::BaselineV2Mode::Stochastic,
            control_interval_seconds: 1.0 / 60.0,
        }),
        teacher_respawn_delay_seconds: 20.0,
        capture: None,
        warmup_steps: 0,
        measured_steps: 1,
        environment: "development".into(),
    }
}

/// Exercise actual codec, exact retry, native evolution and independently restored
/// continuation. Acknowledgements here are offline test receipts; SQLite commits
/// are a separate host qualification and must never be inferred from this report.
pub fn run_checkpoint_capacity(
    options: &super::sensor_qualification::SensorQualificationOptions,
    workspace: &std::path::Path,
    policy: CheckpointCapacityPolicy,
    memory_ceiling_bytes: usize,
    allocate: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use super::checkpoint::{restore_checkpoint, CheckpointOperationId};
    use super::run_start::PendingRunStartTransition;
    use super::scheduler::{FixedStepSchedulerPolicy, SchedulerServiceMode};
    use std::time::Instant;
    let started = Instant::now();
    let run_id = "checkpoint-capacity-qualified".to_owned();
    let prepared = super::fresh_run::prepare_checkpoint_capacity_run(
        super::fresh_run::Stage6aP0FreshRunRequest {
            run_id: run_id.clone(),
            seed: options.seed,
            memory_ceiling_bytes,
        },
        options,
        policy,
        allocate,
    )?;
    let mut report = serde_json::json!({"schemaVersion": 1, "sourceSha256": crate::native_addon_source_sha256(),
        "target": crate::native_addon_build_target(), "audit": prepared.audit,
        "sqliteCommitted": false, "offlineReceipts": true, "continuationExcludesWallDebt": true});
    let Some(mut pending) = prepared.pending else {
        return Ok(report);
    };
    let operation = |value: u128| CheckpointOperationId::parse(format!("{value:032x}"));
    report["populationAllocated"] = true.into();
    let descriptor = pending.publish_checkpoint(workspace, operation(1)?)?;
    report["runStart"] = serde_json::to_value(crate::napi_engine::checkpoint_descriptor_to_napi(
        descriptor.clone(),
    ))?;
    report["initialPublishSeconds"] = started.elapsed().as_secs_f64().into();
    let mut overlap_policy = prepared.admission_policy.clone();
    overlap_policy.memory_ceiling_bytes = memory_ceiling_bytes
        .checked_sub(
            pending
                .qualification_authority()
                .memory_estimate()
                .total_bytes,
        )
        .ok_or("no restore overlap budget")?;
    let restored = restore_checkpoint(
        &workspace.join(&descriptor.relative_filename),
        &prepared.limits,
        &prepared.graph_limits,
        &overlap_policy,
    )?;
    if restored.state.state() != pending.qualification_authority().state()
        || restored.state.graph_spec() != pending.qualification_authority().graph_spec()
    {
        return Err("initial restore differs from admitted source".into());
    }
    // Different operation ID forces the actual existing-file verification path.
    let retry = super::checkpoint::publish_checkpoint(
        workspace,
        operation(2)?,
        2,
        restored.state.checkpoint_boundary()?,
        &prepared.limits,
        &prepared.graph_limits,
        &overlap_policy,
    )?;
    if descriptor.first_content_mismatch(&retry).is_some() {
        return Err("idempotent publication differs".into());
    }
    let mut independent = PendingRunStartTransition::restore_validated_import(
        restored,
        descriptor.clone(),
        prepared.admission_policy.clone(),
        prepared.limits.clone(),
        prepared.graph_limits.clone(),
        prepared.work_limits,
    )?;
    pending.acknowledge_persistence(&descriptor)?;
    pending.publish_running_authority()?;
    independent.acknowledge_persistence(&descriptor)?;
    independent.publish_running_authority()?;
    let scheduler = FixedStepSchedulerPolicy::provisional_defaults();
    let mut running = pending.into_running_loop(scheduler, 0)?;
    let mut reference = independent.into_running_loop(scheduler, 0)?;
    running.service_after_command_drain(17, SchedulerServiceMode::Background, None)?;
    reference.service_after_command_drain(17, SchedulerServiceMode::Background, None)?;
    if running.completed_step() != 1
        || reference.completed_step() != 1
        || running.qualification_restored_continuation_sha256()
            != reference.qualification_restored_continuation_sha256()
    {
        return Err("initial restored continuation differs".into());
    }
    drop(reference);
    report["initialRestoreExact"] = true.into();
    let generation_started = Instant::now();
    running.request_finish_generation("capacity-finish".into(), run_id, 1)?;
    running.service_after_command_drain(34, SchedulerServiceMode::Background, None)?;
    let transition = running
        .pending_generation_transition()
        .ok_or("missing generation transition")?;
    report["combinedGenerationStateBytes"] = transition.combined_state_bytes().into();
    let publication = running.publish_pending_generation_checkpoint(workspace, operation(3)?)?;
    report["generationPublishSeconds"] = generation_started.elapsed().as_secs_f64().into();
    report["generation"] = serde_json::to_value(
        crate::napi_engine::checkpoint_descriptor_to_napi(publication.descriptor.clone()),
    )?;
    report["hallOfFameWeights"] =
        serde_json::to_value(crate::napi_engine::hall_of_fame_weights_descriptor_to_napi(
            publication.hall_of_fame_weights.clone(),
        ))?;
    report["generationCommit"] = serde_json::to_value(
        crate::napi_engine::generation_commit_to_napi(publication.commit_record),
    )?;
    running.acknowledge_pending_generation_persistence(&publication.descriptor)?;
    running.prepare_acknowledged_generation_reassignments()?;
    running.publish_acknowledged_generation_start(34, None)?;
    running.service_after_command_drain(51, SchedulerServiceMode::Background, None)?;
    let expected = running.qualification_restored_continuation_sha256();
    let expected_step = running.completed_step();
    drop(running);
    let mut restored = PendingRunStartTransition::restore_committed(
        workspace,
        &publication.descriptor,
        prepared.admission_policy,
        prepared.limits,
        prepared.graph_limits,
        prepared.work_limits,
    )?;
    restored.acknowledge_persistence(&publication.descriptor)?;
    restored.publish_running_authority()?;
    let mut restored =
        restored.into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)?;
    restored.service_after_command_drain(17, SchedulerServiceMode::Background, None)?;
    if restored.completed_step() != expected_step
        || restored.qualification_restored_continuation_sha256() != expected
    {
        return Err("generation restored continuation differs".into());
    }
    report["generationRestoreExact"] = true.into();
    report["continuationSha256"] = expected.into();
    report["peakRssBytes"] = super::sensor_qualification::process_peak_rss_bytes().into();
    report["totalSeconds"] = started.elapsed().as_secs_f64().into();
    Ok(report)
}

/// Bounded scalar-only lease emitted by the real metadata worker.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapacityExportLease {
    operation_id: String,
    run_id: String,
    descriptor: crate::napi_engine::ManagedCheckpointDescriptor,
    inventory: CapacityExportInventory,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapacityExportInventory {
    version: u32,
    relative_filename: String,
    sha256: String,
    stored_byte_count: String,
    history_count: String,
    hall_of_fame_count: String,
}

/// Exercise actual self-contained export and two imports, including an existing
/// digest-derived destination, with exactly admitted overlapping populations.
pub fn run_checkpoint_archive_capacity(
    options: &super::sensor_qualification::SensorQualificationOptions,
    source_directory: &std::path::Path,
    workspace: &std::path::Path,
    lease_json: &str,
    policy: CheckpointCapacityPolicy,
    memory_ceiling_bytes: usize,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use super::export_archive::{
        compose_export_archive, prepare_import_archive, ExportInventoryDescriptor,
    };
    use super::scheduler::{FixedStepSchedulerPolicy, SchedulerServiceMode};
    let started = std::time::Instant::now();
    if lease_json.len() > 64 * 1024 {
        return Err("capacity lease exceeds 64 KiB".into());
    }
    let lease: CapacityExportLease = serde_json::from_str(lease_json)?;
    let descriptor = crate::napi_engine::checkpoint_descriptor_from_napi(lease.descriptor)?;
    if lease.run_id != descriptor.run_id {
        return Err("export lease run differs from checkpoint".into());
    }
    let inventory = ExportInventoryDescriptor {
        version: lease.inventory.version,
        relative_filename: lease.inventory.relative_filename,
        sha256: lease.inventory.sha256,
        stored_byte_count_hex: lease.inventory.stored_byte_count,
        history_count_hex: lease.inventory.history_count,
        hall_of_fame_count_hex: lease.inventory.hall_of_fame_count,
    };
    let prepared = super::fresh_run::prepare_checkpoint_capacity_run(
        super::fresh_run::Stage6aP0FreshRunRequest {
            run_id: descriptor.run_id.clone(),
            seed: options.seed,
            memory_ceiling_bytes,
        },
        options,
        policy,
        false,
    )?;
    let archive = compose_export_archive(
        source_directory,
        &lease.operation_id,
        &descriptor,
        &inventory,
        &prepared.limits,
        &prepared.graph_limits,
        &prepared.admission_policy,
    )?;
    let archive_path = source_directory.join(&archive.relative_filename);
    let estimate = super::export_archive::estimate_import_disk_bytes(&archive_path)?;
    let imported_directory = workspace.join("managed");
    std::fs::create_dir(&imported_directory)?;
    let first = prepare_import_archive(
        &archive_path,
        workspace,
        &imported_directory,
        "00000000000000000000000000000004",
        "unused-legacy",
        42,
        &prepared.limits,
        &prepared.graph_limits,
        &prepared.admission_policy,
        memory_ceiling_bytes,
    )?;
    if first.transition.qualification_checkpoint_limits() != &prepared.limits {
        return Err("import discarded selected checkpoint limits".into());
    }
    let mut overlap_policy = prepared.admission_policy.clone();
    overlap_policy.memory_ceiling_bytes = memory_ceiling_bytes
        .checked_sub(
            first
                .transition
                .qualification_authority()
                .memory_estimate()
                .total_bytes,
        )
        .ok_or("no import overlap budget")?;
    let mut second = prepare_import_archive(
        &archive_path,
        workspace,
        &imported_directory,
        "00000000000000000000000000000005",
        "unused-legacy",
        42,
        &prepared.limits,
        &prepared.graph_limits,
        &overlap_policy,
        memory_ceiling_bytes,
    )?;
    if first.transition.qualification_authority().state()
        != second.transition.qualification_authority().state()
        || first
            .descriptor
            .first_content_mismatch(&second.descriptor)
            .is_some()
    {
        return Err("idempotent import differs".into());
    }
    drop(first);
    second
        .transition
        .configure_qualification_work_limits(prepared.work_limits)?;
    second
        .transition
        .acknowledge_import_persistence(&second.descriptor)?;
    second.transition.publish_running_authority()?;
    let mut running = second
        .transition
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)?;
    running.service_after_command_drain(17, SchedulerServiceMode::Background, None)?;
    let import_inventory = second.inventory;
    Ok(
        serde_json::json!({"schemaVersion":1,"sourceSha256": crate::native_addon_source_sha256(),
        "target":crate::native_addon_build_target(), "sqliteCommitted": false, "offlineReceipts": true,
        "archiveBytes": u64::from_str_radix(&archive.stored_byte_count_hex,16)?,
        "importCandidateSpoolBytes":estimate.candidate_spool_bytes,"importFinalManagedBytes":estimate.final_managed_bytes,
        "importLimitsPreserved":true,"idempotentImportExact":true,
        "continuationSha256":running.qualification_restored_continuation_sha256(),
        "descriptor":crate::napi_engine::checkpoint_descriptor_to_napi(second.descriptor),
        "inventory":{"version":import_inventory.version,"relativeFilename":import_inventory.relative_filename,
            "sha256":import_inventory.sha256,"storedByteCount":import_inventory.stored_byte_count_hex,
            "historyCount":import_inventory.history_count_hex,"hallOfFameCount":import_inventory.hall_of_fame_count_hex},
        "peakRssBytes":super::sensor_qualification::process_peak_rss_bytes(),"totalSeconds":started.elapsed().as_secs_f64()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn p3_resource_audit_uses_metadata_only_and_preserves_production_limits() {
        let options = super::super::sensor_qualification::SensorQualificationOptions {
            population: 300,
            profile: super::super::sensor_contract::SensorProfile::Motion,
            bins: 32,
            large_graph: true,
            trained_artifact: None,
            respawn: true,
            seed: 42,
            workers: 3,
            math: super::super::inference::InferenceMathBackend::Sse2,
            ray_limits: Some(
                super::super::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap(),
            ),
            baseline: Some(super::super::baseline_v2::BaselineV2Settings {
                mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
                control_interval_seconds: 1.0 / 60.0,
            }),
            teacher_respawn_delay_seconds: 20.0,
            capture: None,
            warmup_steps: 0,
            measured_steps: 1,
            environment: "development".into(),
        };
        for envelope in [512, 768] {
            let prepared = super::super::fresh_run::prepare_checkpoint_capacity_run(
                super::super::fresh_run::Stage6aP0FreshRunRequest {
                    run_id: "checkpoint-capacity-audit".into(),
                    seed: 42,
                    memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
                },
                &options,
                CheckpointCapacityPolicy::new(envelope, false).unwrap(),
                false,
            )
            .unwrap();
            assert!(prepared.pending.is_none());
            assert!(!prepared.audit.numeric_payload_allocated);
            assert_eq!(prepared.audit.parameters_per_genome, 494306);
            assert_eq!(prepared.audit.decoded_role_bytes[3], 300 * 494306 * 4);
            assert_eq!(prepared.audit.worker_scratch_bytes, 512 * 1024 * 1024);
            assert_eq!(prepared.audit.checkpoint_scratch_bytes, 512 * 1024 * 1024);
            assert_eq!(prepared.audit.decoded_envelope_fits, envelope == 768);
            assert_eq!(
                super::super::fresh_run::production_checkpoint_limits().max_total_decoded_bytes,
                512 * 1024 * 1024
            );
            println!("{}", serde_json::to_string(&prepared.audit).unwrap());
        }
    }
}
