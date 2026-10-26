//! Continuous native six-hour-equivalent qualification with constant-size observers.
//! This drives the ordinary complete-step loop; it adds no production scheduler.
use super::checkpoint::CheckpointOperationId;
use super::frame_v1::FrameV1ViewDescriptor;
use super::fresh_run::Stage6aP0FreshRunRequest;
use super::running_loop::{
    RunningAuthorityLoop, RunningAuthorityLoopProgress, RunningFramePublication,
};
use super::scheduler::{FixedStepSchedulerPolicy, SchedulerServiceMode};
use super::sensor_qualification::{ConsumerSensorWork, SensorQualificationOptions};
use super::shadows::{ShadowActivationRequest, ShadowCandidate};
use super::teacher_queue::TeacherQueueSnapshot;
use super::teacher_runtime::{TeacherCaptureLimits, TeacherCaptureRequest};
use super::teacher_samples::TeacherChunkV1;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;

pub const SIX_HOUR_STEPS: usize = 6 * 3600 * 60;
const SNAPSHOT_STEPS: usize = 3600;
const MAX_SNAPSHOTS: usize = SIX_HOUR_STEPS / SNAPSHOT_STEPS + 2;
const CAPTURE_EPOCH: [u8; 16] = [39; 16];
const CAPTURE_BYTES: usize = 16 * 1024 * 1024;

/// Current resident memory complements the process lifetime high-water counter.
#[cfg(target_os = "windows")]
pub(crate) fn current_rss_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..Default::default()
    };
    // SAFETY: The borrowed initialized output covers exactly `size` bytes, is uniquely
    // writable for this call, and is not retained. The process pseudo-handle is not owned.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } == 0 {
        return None;
    }
    u64::try_from(counters.WorkingSetSize).ok()
}
#[cfg(target_os = "linux")]
pub(crate) fn current_rss_bytes() -> Option<u64> {
    super::sensing_fixture::linux_process_status_bytes("VmRSS:")
}

/// Exact counts/total/extrema and conservative percentile buckets, without per-step storage.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Timing {
    samples: u64,
    total_micros: u64,
    maximum_micros: u64,
    buckets: [u64; 25],
}
impl Timing {
    fn record(&mut self, micros: u64) {
        let index = super::coordinator::STEP_TIMING_BUCKET_UPPER_MICROS
            .partition_point(|upper| *upper < micros);
        self.buckets[index.min(24)] += 1;
        self.samples += 1;
        self.total_micros += micros;
        self.maximum_micros = self.maximum_micros.max(micros);
    }
}

/// Constant counters and streaming hash; decoded sample ownership lasts only one bounded drain.
#[derive(Default)]
struct Capture {
    observed: [u64; 10],
    drained: [u64; 10],
    last_ordinal: [u64; 10],
    last_sequence: u64,
    chunks: u64,
    bytes: u64,
    peak_chunks: usize,
    peak_bytes: usize,
    hash: Sha256,
    drain_cost: Timing,
}
impl Capture {
    fn status(
        &mut self,
        running: &RunningAuthorityLoop,
        maximum: usize,
    ) -> Result<TeacherQueueSnapshot, String> {
        let (_, queue, _, error) = running
            .teacher_capture_status()
            .ok_or("soak capture status missing")?;
        if let Some(error) = error {
            return Err(error.into());
        }
        if queue.queued_chunks > maximum || queue.queued_bytes > queue.payload_budget_bytes {
            return Err("soak capture exceeded admission".into());
        }
        self.peak_chunks = self.peak_chunks.max(queue.queued_chunks);
        self.peak_bytes = self.peak_bytes.max(queue.queued_bytes);
        Ok(queue)
    }
    fn drain(&mut self, running: &mut RunningAuthorityLoop) -> Result<(), String> {
        let started = Instant::now();
        let drained = running
            .drain_teacher_chunks(64, 1024 * 1024)?
            .ok_or("soak drain missing")?;
        if drained.chunks.is_empty() && drained.snapshot.queued_chunks != 0 {
            return Err("soak drain cannot advance queued object".into());
        }
        for owned in drained.chunks {
            let descriptor = owned.descriptor;
            let chunk = TeacherChunkV1::decode(&owned.bytes).map_err(|e| e.to_string())?;
            let slot = chunk.baseline_slot as usize;
            if slot >= 10
                || chunk.chunk_sequence <= self.last_sequence
                || descriptor.sequence != chunk.chunk_sequence
                || descriptor.count != chunk.samples.len()
                || descriptor.byte_count != owned.bytes.len()
                || descriptor.sha256 != <[u8; 32]>::from(Sha256::digest(&owned.bytes))
                || chunk.contract
                    != *running
                        .teacher_capture_status()
                        .ok_or("soak contract missing")?
                        .0
            {
                return Err("soak chunk chronology/identity mismatch".into());
            }
            for sample in &chunk.samples {
                if sample.ordinal <= self.last_ordinal[slot] {
                    return Err("soak sample order regressed".into());
                }
                self.last_ordinal[slot] = sample.ordinal;
            }
            self.last_sequence = chunk.chunk_sequence;
            self.drained[slot] += chunk.samples.len() as u64;
            self.chunks += 1;
            self.bytes += owned.bytes.len() as u64;
            self.hash.update(descriptor.sequence.to_le_bytes());
            self.hash.update(descriptor.sha256);
        }
        self.drain_cost
            .record(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
        Ok(())
    }
}

/// Run exactly the requested complete steps in one authority, with no checkpoint/restart stitching.
/// The CLI owns the exclusive workspace and externally retained progress/final report paths.
pub fn run_sensor_soak(
    options: &SensorQualificationOptions,
    workspace: &Path,
    shadow_artifact: &Path,
    progress: &mut dyn FnMut(&Value) -> Result<(), String>,
) -> Result<Value, String> {
    let total = options
        .warmup_steps
        .checked_add(options.measured_steps)
        .ok_or("soak step overflow")?;
    let capture_options = options
        .capture
        .ok_or("soak requires explicit capture pressure")?;
    capture_options.validate()?;
    if total == 0
        || total > SIX_HOUR_STEPS
        || options.measured_steps == 0
        || !options.respawn
        || options.baseline.is_none()
        || options.trained_artifact.is_some()
        || options.large_graph
        || !matches!(options.population, 55 | 300)
        || !(0.5..=60.0).contains(&options.teacher_respawn_delay_seconds)
        || !["development", "oxygen-debian"].contains(&options.environment.as_str())
    {
        return Err("soak requires 1..1296000 total steps, P0/P1 modest random respawning population and baseline v2".into());
    }
    if std::fs::symlink_metadata(workspace)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
        || std::fs::read_dir(workspace)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err("soak requires an empty real task workspace".into());
    }
    let artifact = super::sensor_qualification::load_trained_artifact(shadow_artifact)?;
    let artifact_descriptor = artifact.descriptor();
    let candidate = ShadowCandidate::from_artifact(artifact)?;
    let run_id = format!("soak-qualification-{}-{}", options.population, options.seed);
    let mut pending = super::sensor_qualification::prepare_sensor_workload(
        options,
        Stage6aP0FreshRunRequest {
            run_id: run_id.clone(),
            seed: options.seed,
            memory_ceiling_bytes: 4 * 1024usize.pow(3),
        },
    )
    .map_err(|e| e.to_string())?;
    let metadata: Value =
        serde_json::from_str(&pending.startup_metadata_json().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let checkpoint = pending
        .publish_checkpoint(
            workspace,
            CheckpointOperationId::parse("34567890abcdef1234567890abcdef12")
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    pending
        .acknowledge_persistence(&checkpoint)
        .map_err(|e| e.to_string())?;
    pending
        .publish_running_authority()
        .map_err(|e| e.to_string())?;
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .map_err(|e| e.to_string())?;
    let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
    let mut timing = Timing::default();
    let mut capture = Capture::default();
    let mut consumers = [ConsumerSensorWork::default(); 3];
    let mut snapshots = Vec::with_capacity(MAX_SNAPSHOTS);
    let started = Instant::now();
    snapshots.push(running.qualification_storage_snapshot()?);
    for ordinal in 0..total {
        if ordinal == options.warmup_steps {
            running.start_teacher_capture(&TeacherCaptureRequest {
                capture_epoch: CAPTURE_EPOCH,
                expected_run_id: run_id.clone(),
                expected_config_revision: 1,
                baseline_slots: (0..10).collect::<Vec<_>>().into_boxed_slice(),
                limits: TeacherCaptureLimits {
                    pending_bytes: CAPTURE_BYTES,
                    queue_bytes: CAPTURE_BYTES,
                    queue_chunks: capture_options.queue_chunks,
                },
            })?;
            for slot in 0..4 {
                running.activate_shadow(
                    &ShadowActivationRequest {
                        expected_run_id: run_id.clone(),
                        expected_config_revision: 1,
                        capture_epoch: CAPTURE_EPOCH,
                        shadow_slot: slot,
                        baseline_slot: 0,
                        candidate: candidate.clone(),
                    },
                    u64::from(slot) + 1,
                )?;
            }
        }
        let wall_ms = ((ordinal as u64 + 1) * 1000).div_ceil(60);
        let step_started = Instant::now();
        let result = running
            .service_after_command_drain(
                wall_ms,
                SchedulerServiceMode::Background,
                Some(RunningFramePublication::new(
                    FrameV1ViewDescriptor::default(),
                    &mut frame,
                )),
            )
            .map_err(|e| e.to_string())?;
        let micros = step_started
            .elapsed()
            .as_micros()
            .try_into()
            .map_err(|_| "soak duration overflow")?;
        if !matches!(result, RunningAuthorityLoopProgress::Published { .. })
            || running.completed_step() != ordinal as u64 + 1
        {
            return Err("soak must commit exactly one complete native step".into());
        }
        if ordinal >= options.warmup_steps {
            timing.record(micros);
            let boundary = running
                .published_teacher_boundary()
                .ok_or("soak committed boundary missing")?;
            for decision in boundary.decisions {
                let slot = decision.baseline_slot as usize;
                if slot >= 10 {
                    return Err("soak unexpected teacher slot".into());
                }
                capture.observed[slot] += 1;
            }
            let work = running
                .qualification_sensor_work()
                .ok_or("soak sensing work missing")?;
            let charged: u64 = work.iter().map(|w| w.ray_charged_units).sum();
            if options
                .ray_limits
                .is_some_and(|limits| charged > limits.batch() as u64)
            {
                return Err("soak exceeded fair shared ray batch".into());
            }
            for (aggregate, current) in consumers.iter_mut().zip(work) {
                aggregate.merge(current);
            }
            capture.status(&running, capture_options.queue_chunks)?;
            if (ordinal + 1 - options.warmup_steps)
                .is_multiple_of(capture_options.drain_every_steps)
            {
                capture.drain(&mut running)?;
            }
        }
        if (ordinal + 1).is_multiple_of(SNAPSHOT_STEPS) || ordinal + 1 == total {
            if snapshots.len() >= MAX_SNAPSHOTS {
                return Err("soak snapshot admission".into());
            }
            snapshots.push(running.qualification_storage_snapshot()?);
            progress(
                &json!({"schemaVersion":1,"complete":false,"requestedSteps":total,
                "sourceSha256":crate::native_addon_source_sha256(),"target":crate::native_addon_build_target(),
                "completedStep":running.completed_step(),"wallSeconds":started.elapsed().as_secs_f64(),
                "peakRssBytes":super::sensor_qualification::process_peak_rss_bytes(),
                "latestStorage":snapshots.last(),"stepTiming":timing}),
            )?;
        }
    }
    if capture.observed[0] == 0 {
        return Err("soak requires actual shadow teacher samples".into());
    }
    let shadows = running.shadow_snapshots();
    let shadow_reports: Vec<Value> = shadows
        .iter()
        .map(|slot| {
            let slot = slot.as_ref().ok_or("soak shadow missing")?;
            if !slot.enabled || slot.error.is_some() || slot.totals.samples != capture.observed[0] {
                return Err("soak shadow lost exact teacher observation coverage");
            }
            Ok(
                json!({"activationSequence":slot.activation_sequence,"samples":slot.totals.samples,
            "deaths":slot.deaths,"lastSampleStep":slot.last_sample_step,"totals":slot.totals}),
            )
        })
        .collect::<Result<_, _>>()?;
    running.stop_teacher_capture()?;
    while capture
        .status(&running, capture_options.queue_chunks)?
        .queued_chunks
        != 0
    {
        capture.drain(&mut running)?;
    }
    let final_queue = capture.status(&running, capture_options.queue_chunks)?;
    let observed: u64 = capture.observed.iter().sum();
    let drained: u64 = capture.drained.iter().sum();
    if observed != drained + final_queue.dropped_samples
        || final_queue.production_watermark != capture.chunks + final_queue.dropped_chunks
        || final_queue.released_through != final_queue.production_watermark
        || final_queue.count_limited_dropped_chunks + final_queue.byte_limited_dropped_chunks
            != final_queue.dropped_chunks
    {
        return Err("soak capture ownership reconciliation failed".into());
    }
    let capture_report = json!({"observed":capture.observed,"drained":capture.drained,"chunks":capture.chunks,
            "bytes":capture.bytes,"drainedObjectsSha256":format!("{:x}",capture.hash.finalize()),
            "queuePeakChunks":capture.peak_chunks,"queuePeakBytes":capture.peak_bytes,
            "drainEverySteps":capture_options.drain_every_steps,"queueChunkLimit":capture_options.queue_chunks,
            "droppedChunks":final_queue.dropped_chunks,"droppedSamples":final_queue.dropped_samples,
            "productionWatermark":final_queue.production_watermark,"releasedThrough":final_queue.released_through,
            "countLimitedDroppedChunks":final_queue.count_limited_dropped_chunks,
            "byteLimitedDroppedChunks":final_queue.byte_limited_dropped_chunks,"ownershipReconciled":true,
            "drainTiming":capture.drain_cost});
    Ok(
        json!({"schemaVersion":1,"gate":"continuous-native-soak-v1","complete":true,
        "sourceSha256":crate::native_addon_source_sha256(),"target":crate::native_addon_build_target(),
        "buildClass":crate::native_addon_build_class(),"buildProfile":crate::native_addon_build_profile(),
        "seed":options.seed,"workers":options.workers,"requestedSteps":total,"warmupSteps":options.warmup_steps,
        "measuredSteps":options.measured_steps,"completedStep":running.completed_step(),
        "sixHourEquivalent":options.measured_steps==SIX_HOUR_STEPS,"wallSeconds":started.elapsed().as_secs_f64(),
        "metadata":metadata,"shadowArtifact":artifact_descriptor,"stepTiming":timing,
        "timingBucketUpperMicros":super::coordinator::STEP_TIMING_BUCKET_UPPER_MICROS.map(|value|format!("{value:016x}")),
        "consumerWork":consumers,"snapshots":snapshots,"frameBufferCapacity":frame.capacity(),
        "peakRssBytes":super::sensor_qualification::process_peak_rss_bytes(),
        "continuationSha256":running.qualification_continuation_sha256(),"shadows":shadow_reports,
        "capture":capture_report,
        "scope":"One uninterrupted native fixed-step authority with four identical validated candidate shadows and explicit slow capture drain. Complete-step timing includes native capture and shadows; observers/drain/encoding checks are outside that timing. Run-start checkpoint uses native acknowledgement without SQLite. No trainer, HTTP/spool, wall-debt performance, crowded mature population, generation-transition soak or production-default qualification."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_storage_is_constant_and_open_bucket_keeps_long_stalls() {
        let mut timing = Timing::default();
        for value in [0, 100, 101, 16_667, 16_668, 999_999] {
            timing.record(value);
        }
        assert_eq!(timing.samples, 6);
        assert_eq!(timing.buckets.iter().sum::<u64>(), 6);
        assert_eq!(timing.buckets[0], 2);
        assert_eq!(timing.buckets[24], 1);
        assert_eq!(timing.total_micros, 1_033_535);
        assert_eq!(timing.maximum_micros, 999_999);
        assert_eq!(MAX_SNAPSHOTS, 362);
    }

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "slither-native-soak-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            assert_eq!(self.0.canonicalize().unwrap(), self.0);
            assert!(!std::fs::symlink_metadata(&self.0)
                .unwrap()
                .file_type()
                .is_symlink());
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn options() -> SensorQualificationOptions {
        SensorQualificationOptions {
            query_limits: None,
            population: 55,
            profile: super::super::sensor_contract::SensorProfile::Basic,
            bins: 8,
            large_graph: false,
            trained_artifact: None,
            respawn: true,
            seed: 42,
            workers: 1,
            math: super::super::inference::InferenceMathBackend::Scalar,
            ray_limits: Some(
                super::super::ray_work::RayWorkLimits::new(1, 256, 16384, 512).unwrap(),
            ),
            baseline: Some(super::super::baseline_v2::BaselineV2Settings {
                mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
                control_interval_seconds: 0.06,
            }),
            teacher_respawn_delay_seconds: 0.5,
            capture: Some(
                super::super::capture_qualification::CaptureQualificationOptions {
                    drain_every_steps: 600,
                    queue_chunks: 1,
                },
            ),
            warmup_steps: 1,
            measured_steps: 600,
            environment: "development".into(),
        }
    }
    #[test]
    fn slow_consumer_and_four_shadows_preserve_single_multiworker_authority() {
        let artifact = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/portable-brain-v1.slither-brain"
        ));
        let mut options = options();
        let mut publications = 0;
        let serial = run_sensor_soak(&options, &Scratch::new().0, artifact, &mut |_| {
            publications += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(publications, 1);
        assert_eq!(serial["completedStep"], 601);
        assert_eq!(serial["stepTiming"]["samples"], 600);
        assert_eq!(serial["sixHourEquivalent"], false);
        assert_eq!(serial["capture"]["ownershipReconciled"], true);
        assert!(serial["capture"]["droppedChunks"].as_u64().unwrap() > 0);
        assert!(serial["shadows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|slot| slot["samples"].as_u64().unwrap() > 0));
        options.workers = 3;
        let parallel =
            run_sensor_soak(&options, &Scratch::new().0, artifact, &mut |_| Ok(())).unwrap();
        for key in ["continuationSha256", "consumerWork", "shadows"] {
            assert_eq!(serial[key], parallel[key], "{key}");
        }
        let mut serial_storage = serial["snapshots"].clone();
        let mut parallel_storage = parallel["snapshots"].clone();
        for snapshots in [&mut serial_storage, &mut parallel_storage] {
            for snapshot in snapshots.as_array_mut().unwrap() {
                snapshot.as_object_mut().unwrap().remove("currentRssBytes");
            }
        }
        assert_eq!(serial_storage, parallel_storage);
        let mut reference_options = options;
        reference_options.capture = None;
        let reference = super::super::sensor_qualification::run_sensor_qualification(
            &reference_options,
            &Scratch::new().0,
        )
        .unwrap();
        assert_eq!(
            serial["continuationSha256"].as_str(),
            reference.continuation_sha256.as_deref()
        );
    }
    #[test]
    fn refuses_incomplete_duration_overflow_and_foreign_workspace_before_construction() {
        let mut options = options();
        options.measured_steps = SIX_HOUR_STEPS;
        assert!(run_sensor_soak(
            &options,
            &Scratch::new().0,
            Path::new("missing"),
            &mut |_| Ok(())
        )
        .unwrap_err()
        .contains("1296000"));
        options.measured_steps = 12;
        options.respawn = false;
        assert!(run_sensor_soak(
            &options,
            &Scratch::new().0,
            Path::new("missing"),
            &mut |_| Ok(())
        )
        .is_err());
        options.respawn = true;
        let scratch = Scratch::new();
        std::fs::write(scratch.0.join("owner"), b"keep").unwrap();
        assert!(
            run_sensor_soak(&options, &scratch.0, Path::new("missing"), &mut |_| Ok(()))
                .unwrap_err()
                .contains("empty real")
        );
    }
}
