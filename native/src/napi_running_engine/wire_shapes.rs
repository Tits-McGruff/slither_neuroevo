//! Cached display and archive wire shapes.

use super::*;

/// Fixed-size work facts, copied only after complete authoritative publication.
#[napi(object)]
pub struct BackgroundEvolvedPlacement {
    pub step: String,
    pub eligible: u32,
    pub attempted: u32,
    pub placed: u32,
    pub pending_no_placement: u32,
    pub pending_work_limit: u32,
    pub deferred: u32,
    pub candidates: u32,
    pub geometry_checks: u32,
}

/// Compact life counters; hexadecimal words preserve exact long-generation values.
#[napi(object)]
pub struct BackgroundEvolvedLifeStatus {
    pub version: u32,
    pub population: u32,
    pub alive: u32,
    pub waiting: u32,
    pub pending_placement: u32,
    pub exhausted: u32,
    pub dead_no_respawn: u32,
    pub lives: String,
    pub deaths: String,
    pub respawns: String,
    pub failed_placements: String,
    pub pending_seconds: f64,
    pub recent_event_count: u32,
    pub life_end_cursor: String,
    pub placement: BackgroundEvolvedPlacement,
}

/// Cached frame chronology and basic stats; no population or world objects.
#[napi(object)]
pub struct BackgroundDisplayStatus {
    pub sequence: String,
    pub world_epoch: String,
    pub completed_step: String,
    pub generation: String,
    pub generation_time: f64,
    pub evolved: Option<BackgroundEvolvedLifeStatus>,
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
        evolved: status.evolved.map(evolved_status_to_napi),
        alive_population: status.alive_population as u32,
        baseline_bots_alive: status.baseline_bots_alive as u32,
        baseline_bots_total: status.baseline_bots_total as u32,
        total_snakes: status.frame.total_snakes as u32,
        alive_snakes: status.frame.alive_snakes as u32,
        pellets: status.frame.pellets as u32,
        frame_byte_length: status.frame.byte_length as f64,
    }
}

/// Serialize already captured facts; no traversal or simulation runs on the Node thread.
pub(super) fn evolved_status_to_napi(
    status: crate::engine::evolved_telemetry::EvolvedLifecycleTelemetry,
) -> BackgroundEvolvedLifeStatus {
    let placement = status.placement;
    BackgroundEvolvedLifeStatus {
        version: 1,
        population: status.population as u32,
        alive: status.alive as u32,
        waiting: status.waiting as u32,
        pending_placement: status.pending_placement as u32,
        exhausted: status.exhausted as u32,
        dead_no_respawn: status.dead_no_respawn as u32,
        lives: u64_hex(status.lives),
        deaths: u64_hex(status.deaths),
        respawns: u64_hex(status.respawns),
        failed_placements: u64_hex(status.failed_placements),
        pending_seconds: status.pending_seconds,
        recent_event_count: status.recent_event_count as u32,
        life_end_cursor: u64_hex(status.life_end_cursor),
        placement: BackgroundEvolvedPlacement {
            step: u64_hex(placement.step),
            eligible: placement.eligible_slots as u32,
            attempted: placement.attempted_slots as u32,
            placed: placement.placed_slots as u32,
            pending_no_placement: placement.pending_no_placement as u32,
            pending_work_limit: placement.pending_work_limit as u32,
            deferred: (placement.eligible_slots - placement.attempted_slots) as u32,
            candidates: placement.candidates as u32,
            geometry_checks: placement.geometry_checks,
        },
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

/// Reject public selector narrowing before constructing any authoritative command.
pub(super) fn checked_entity_index(value: f64, maximum: u32, allow_zero: bool) -> Result<u32> {
    let minimum = if allow_zero { 0.0 } else { 1.0 };
    if !value.is_finite()
        || value.fract() != 0.0
        || !(minimum..=f64::from(maximum)).contains(&value)
    {
        return Err(Error::new(
            Status::InvalidArg,
            "entity selector must be an exact bounded integer",
        ));
    }
    Ok(value as u32)
}

/// Replaceable complete focused neural snapshot.
#[napi(object)]
pub struct BackgroundVisualization {
    pub cleared: bool,
    pub sequence: String,
    pub world_epoch: String,
    pub completed_step: String,
    pub snake_id: u32,
    pub kind: String,
    pub layers: Vec<BackgroundVisualizationLayer>,
}

pub(super) fn visualization_to_napi(
    status: RunningVisualizationStatus,
) -> Result<BackgroundVisualization> {
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
        cleared: status.cleared,
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

/// Read-only snapshot of one native archive job, including a completed one.
#[napi(object)]
pub struct ArchiveWorkProgress {
    pub operation_id: String,
    pub kind: String,
    pub completed_bytes: String,
    pub started: bool,
    pub finished: bool,
    pub phase_trace: Option<ArchivePhaseDiagnostics>,
}

/// Optional timing facts; populated only with SLITHER_TRACE_ARCHIVE_PHASES=1.
#[napi(object)]
pub struct ArchivePhaseDiagnostics {
    pub elapsed_micros: String,
    pub truncated: bool,
    pub intervals: Vec<ArchivePhaseTiming>,
    pub rss_sampler_started: bool,
    pub requested_rss_sample_interval_micros: String,
}

/// One bounded phase interval, including still-open outer stages.
#[napi(object)]
pub struct ArchivePhaseTiming {
    pub phase: String,
    pub started_micros: String,
    pub finished_micros: Option<String>,
    pub start_rss_bytes: Option<String>,
    pub finish_rss_bytes: Option<String>,
    pub sampled_peak_rss_bytes: Option<String>,
    pub rss_samples: String,
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

/// Conservative manifest-derived disk terms before decoded import staging.
#[napi(object)]
pub struct ImportDiskEstimateResult {
    pub candidate_spool_bytes: String,
    pub final_managed_bytes: String,
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
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
#[napi(object)]
pub struct PreparedImportInventoryResult {
    pub version: u32,
    pub relative_filename: String,
    pub sha256: String,
    pub stored_byte_count: String,
    pub history_count: String,
    pub hall_of_fame_count: String,
    #[cfg_attr(
        feature = "engine-test-hooks",
        serde(skip_serializing_if = "Option::is_none")
    )]
    pub saved_brains_json: Option<String>,
}

/// Convert the complete trusted inventory once for both production and qualification hosts.
pub(crate) fn import_inventory_to_napi(
    inventory: crate::engine::export_archive::ImportInventoryDescriptor,
) -> std::result::Result<PreparedImportInventoryResult, serde_json::Error> {
    Ok(PreparedImportInventoryResult {
        version: inventory.version,
        relative_filename: inventory.relative_filename,
        sha256: inventory.sha256,
        stored_byte_count: inventory.stored_byte_count_hex,
        history_count: inventory.history_count_hex,
        hall_of_fame_count: inventory.hall_of_fame_count_hex,
        saved_brains_json: if inventory.saved_brains.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&inventory.saved_brains)?)
        },
    })
}

/// Small fresh replacement facts. The complete candidate remains in Rust.
#[napi(object)]
pub struct PreparedFreshRunResult {
    pub descriptor: crate::napi_engine::ManagedCheckpointDescriptor,
    pub startup_metadata: String,
}
