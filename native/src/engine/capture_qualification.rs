//! Native capture load evidence from exact committed gameplay, without a spool or trainer.
use super::baseline_control::BaselineLifeStage;
use super::running_loop::RunningAuthorityLoop;
use super::sensor_qualification::{CostDistribution, MAX_QUALIFICATION_STEPS};
use super::state::BaselineStrategyState;
use super::teacher_queue::TeacherQueueSnapshot;
use super::teacher_runtime::{TeacherCaptureLimits, TeacherCaptureRequest};
use super::teacher_samples::{TeacherChunkV1, TeacherSampleV1};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::Instant;

const TEACHERS: usize = 10;
const BUDGET: usize = 16 * 1024 * 1024;
const CAPTURE_CAVEAT: &str = "Actual ten-slot native gameplay capture after warmup, not fabricated full-rate rows. Counts include only due living teachers; deaths and cadence can reduce sustained throughput. Native encoding/queue publication are inside step cost; hashing, drain/decoding and reporting are outside it. No Node spool, HTTP, trainer, shadows, disk rotation or wall-debt scheduling is qualified. Queue drops remove complete chunks; exact-sample equality has no pass when any data was dropped. This does not select production resource defaults or prove a six-hour soak.";

/// Explicit diagnostic pressure; native pending/queue ceilings remain 16 MiB each.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureQualificationOptions {
    pub drain_every_steps: usize,
    pub queue_chunks: usize,
}
impl CaptureQualificationOptions {
    pub(crate) fn validate(self) -> Result<(), String> {
        if !(1..=MAX_QUALIFICATION_STEPS).contains(&self.drain_every_steps)
            || !(1..=65_536).contains(&self.queue_chunks)
        {
            return Err("capture requires drain steps 1..60000 and queue chunks 1..65536".into());
        }
        Ok(())
    }
}

/// Counts refer only to the measured interval; warmup precedes capture admission.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureQualificationReport {
    pub options: CaptureQualificationOptions,
    pub pending_budget_bytes: usize,
    pub queue_budget_bytes: usize,
    /// Exact encoded payload allowance after mailbox, metadata, ring and staging reserves.
    pub queue_payload_budget_bytes: usize,
    pub samples_observed: [u64; TEACHERS],
    pub samples_drained: [u64; TEACHERS],
    pub due_teacher_histogram: [u64; TEACHERS + 1],
    pub longest_ten_teacher_streak_steps: usize,
    pub chunks_drained: u64,
    pub bytes_drained: u64,
    pub queue_peak_chunks: usize,
    pub queue_peak_bytes: usize,
    pub dropped_chunks: u64,
    pub dropped_samples: u64,
    pub dropped_bytes: u64,
    pub count_limited_dropped_chunks: u64,
    pub byte_limited_dropped_chunks: u64,
    pub production_watermark: u64,
    pub released_through: u64,
    pub samples_per_simulated_second: f64,
    pub encoded_bytes_per_simulated_second: f64,
    /// Serialized data divided by native step and final-stop service time, excluding this observer/drain.
    pub encoded_bytes_per_native_service_second: f64,
    pub source_sample_sha256: [String; TEACHERS],
    pub drained_sample_sha256: [String; TEACHERS],
    /// None for zero samples or any dropped chunk; loss never manufactures an exactness pass.
    pub exact_samples_match: Option<bool>,
    pub queue_accounting_matches: bool,
    pub drain_cost: Option<CostDistribution>,
    /// Final partial-chunk publication is cold and lies outside complete-step cost.
    pub stop_cost_ms: f64,
    pub caveat: &'static str,
}

/// Streaming per-slot witnesses and fixed counters; no historical observation retention.
pub(crate) struct CaptureQualificationObserver {
    report: CaptureQualificationReport,
    source: [Sha256; TEACHERS],
    drained: [Sha256; TEACHERS],
    last_ordinal: [u64; TEACHERS],
    last_chunk_sequence: u64,
    ten_teacher_streak: usize,
    drain_times: Vec<f64>,
}

/// One independently constructed sample view; no shared stream assembly/codec mapping.
struct SampleView<'a> {
    generation: u64,
    physical_id: u64,
    step: u64,
    ordinal: u64,
    observation: &'a [f32],
    applied_turn: f32,
    flags: [bool; 4],
    strategy: u8,
    life_stage: u8,
    strata: u16,
}
impl SampleView<'_> {
    fn digest(&self, hash: &mut Sha256) {
        for value in [self.generation, self.physical_id, self.step, self.ordinal] {
            hash.update(value.to_le_bytes());
        }
        hash.update((self.observation.len() as u64).to_le_bytes());
        for value in self.observation {
            hash.update(value.to_bits().to_le_bytes());
        }
        hash.update(self.applied_turn.to_bits().to_le_bytes());
        hash.update(self.flags.map(u8::from));
        hash.update([self.strategy, self.life_stage]);
        hash.update(self.strata.to_le_bytes());
    }
}

impl CaptureQualificationObserver {
    /// Arm the ordinary capture only at a clean boundary, after warmup.
    pub(crate) fn start(
        running: &mut RunningAuthorityLoop,
        options: CaptureQualificationOptions,
        run_id: &str,
        measured_steps: usize,
    ) -> Result<Self, String> {
        options.validate()?;
        running.start_teacher_capture(&TeacherCaptureRequest {
            capture_epoch: [37; 16],
            expected_run_id: run_id.into(),
            expected_config_revision: 1,
            baseline_slots: (0..TEACHERS as u32).collect::<Vec<_>>().into_boxed_slice(),
            limits: TeacherCaptureLimits {
                pending_bytes: BUDGET,
                queue_chunks: options.queue_chunks,
                queue_bytes: BUDGET,
            },
        })?;
        Ok(Self {
            report: CaptureQualificationReport {
                options,
                pending_budget_bytes: BUDGET,
                queue_budget_bytes: BUDGET,
                queue_payload_budget_bytes: running
                    .teacher_capture_status()
                    .ok_or("capture status missing after admission")?
                    .1
                    .payload_budget_bytes,
                samples_observed: [0; TEACHERS],
                samples_drained: [0; TEACHERS],
                due_teacher_histogram: [0; TEACHERS + 1],
                longest_ten_teacher_streak_steps: 0,
                chunks_drained: 0,
                bytes_drained: 0,
                queue_peak_chunks: 0,
                queue_peak_bytes: 0,
                dropped_chunks: 0,
                dropped_samples: 0,
                dropped_bytes: 0,
                count_limited_dropped_chunks: 0,
                byte_limited_dropped_chunks: 0,
                production_watermark: 0,
                released_through: 0,
                samples_per_simulated_second: 0.0,
                encoded_bytes_per_simulated_second: 0.0,
                encoded_bytes_per_native_service_second: 0.0,
                source_sample_sha256: std::array::from_fn(|_| String::new()),
                drained_sample_sha256: std::array::from_fn(|_| String::new()),
                exact_samples_match: None,
                queue_accounting_matches: false,
                drain_cost: None,
                stop_cost_ms: 0.0,
                caveat: CAPTURE_CAVEAT,
            },
            source: std::array::from_fn(|_| Sha256::new()),
            drained: std::array::from_fn(|_| Sha256::new()),
            last_ordinal: [0; TEACHERS],
            last_chunk_sequence: 0,
            ten_teacher_streak: 0,
            drain_times: Vec::with_capacity(
                measured_steps / options.drain_every_steps + options.queue_chunks + 1,
            ),
        })
    }

    /// Read the exact completed step once; held steps contribute no fabricated rows.
    pub(crate) fn observe(
        &mut self,
        running: &mut RunningAuthorityLoop,
        measured_ordinal: usize,
    ) -> Result<(), String> {
        let boundary = running
            .published_teacher_boundary()
            .ok_or("capture boundary missing")?;
        let due = boundary.decisions.len();
        if due > TEACHERS {
            return Err("unexpected capture teacher count".into());
        }
        self.report.due_teacher_histogram[due] += 1;
        self.ten_teacher_streak = if due == TEACHERS {
            self.ten_teacher_streak + 1
        } else {
            0
        };
        self.report.longest_ten_teacher_streak_steps = self
            .report
            .longest_ten_teacher_streak_steps
            .max(self.ten_teacher_streak);
        for (index, decision) in boundary.decisions.iter().enumerate() {
            let slot = decision.baseline_slot as usize;
            if slot >= TEACHERS {
                return Err("unexpected capture slot".into());
            }
            let row = boundary
                .observation(index)
                .ok_or("capture source row missing")?;
            let outcome = boundary
                .outcome(index)
                .ok_or("capture source outcome missing")?;
            let first = row[21] == 1.0;
            self.report.samples_observed[slot] += 1;
            SampleView {
                generation: boundary.key.generation(),
                physical_id: decision.physical_snake_id,
                step: boundary.key.source_completed_step() + 1,
                ordinal: self.report.samples_observed[slot],
                observation: row,
                applied_turn: decision.applied_turn,
                flags: [
                    decision.boost_request,
                    outcome.realized_boost,
                    outcome.post_step_death,
                    first,
                ],
                strategy: match decision.strategy {
                    BaselineStrategyState::Roam => 0,
                    BaselineStrategyState::Seek => 1,
                    BaselineStrategyState::Avoid => 2,
                    BaselineStrategyState::Boost => 3,
                },
                life_stage: match decision.life_stage {
                    BaselineLifeStage::Small => 0,
                    BaselineLifeStage::Medium => 1,
                    BaselineLifeStage::Large => 2,
                },
                strata: u16::from(decision.unsafe_route || decision.urgent_sectors != 0)
                    | (u16::from(decision.escape_or_pursuit) << 1)
                    | (u16::from(decision.boost_request) << 2)
                    | (u16::from(decision.strategy_transition) << 3)
                    | (u16::from(row[22] == 1.0 || row[23] == 1.0) << 4)
                    | (u16::from(first) << 5),
            }
            .digest(&mut self.source[slot]);
        }
        self.snapshot(running)?;
        if measured_ordinal.is_multiple_of(self.report.options.drain_every_steps) {
            self.drain(running)?;
        }
        Ok(())
    }

    fn snapshot(&mut self, running: &RunningAuthorityLoop) -> Result<TeacherQueueSnapshot, String> {
        let (_, snapshot, _, error) = running
            .teacher_capture_status()
            .ok_or("capture status missing")?;
        if let Some(error) = error {
            return Err(error.into());
        }
        self.report.queue_peak_chunks = self.report.queue_peak_chunks.max(snapshot.queued_chunks);
        self.report.queue_peak_bytes = self.report.queue_peak_bytes.max(snapshot.queued_bytes);
        if snapshot.queued_chunks > self.report.options.queue_chunks
            || snapshot.payload_budget_bytes != self.report.queue_payload_budget_bytes
            || snapshot.queued_bytes > snapshot.payload_budget_bytes
        {
            return Err("capture exceeded admitted queue".into());
        }
        Ok(snapshot)
    }

    fn drain(&mut self, running: &mut RunningAuthorityLoop) -> Result<(), String> {
        let started = Instant::now();
        // One bounded transfer per cadence; finalization repeats it only after production stops.
        let drain = running
            .drain_teacher_chunks(64, 1024 * 1024)?
            .ok_or("capture drain missing")?;
        if drain.chunks.is_empty() && drain.snapshot.queued_chunks != 0 {
            return Err("capture drain budget cannot advance the queued chunk".into());
        }
        for owned in drain.chunks {
            let descriptor = owned.descriptor;
            let chunk = TeacherChunkV1::decode(&owned.bytes).map_err(|error| error.to_string())?;
            let slot = chunk.baseline_slot as usize;
            if slot >= TEACHERS
                || chunk.chunk_sequence <= self.last_chunk_sequence
                || descriptor.sequence != chunk.chunk_sequence
                || descriptor.baseline_slot != chunk.baseline_slot
                || descriptor.generation != chunk.generation
                || descriptor.physical_snake_id != chunk.physical_snake_id
                || descriptor.capture_epoch != chunk.contract.capture_epoch
                || descriptor.config_sha256 != chunk.contract.config_sha256
                || descriptor.sensor_sha256 != chunk.contract.sensor_sha256
                || descriptor.teacher_sha256 != chunk.contract.teacher_sha256
                || descriptor.input_size != chunk.contract.input_size
                || descriptor.start_reason != chunk.start_reason
                || descriptor.end_reason != chunk.end_reason
                || descriptor.first_sample != chunk.samples.first().ok_or("empty chunk")?.ordinal
                || descriptor.last_sample != chunk.samples.last().ok_or("empty chunk")?.ordinal
                || descriptor.first_step != chunk.samples.first().ok_or("empty chunk")?.step
                || descriptor.last_step != chunk.samples.last().ok_or("empty chunk")?.step
                || descriptor.count != chunk.samples.len()
                || descriptor.byte_count != owned.bytes.len()
                || descriptor.sha256 != <[u8; 32]>::from(Sha256::digest(&owned.bytes))
                || chunk.contract
                    != *running
                        .teacher_capture_status()
                        .ok_or("capture contract missing")?
                        .0
            {
                return Err("capture chunk descriptor/contract mismatch".into());
            }
            self.last_chunk_sequence = chunk.chunk_sequence;
            self.report.chunks_drained += 1;
            self.report.bytes_drained += owned.bytes.len() as u64;
            for row in &chunk.samples {
                self.decoded_sample(&chunk, row)?;
            }
        }
        self.drain_times
            .push(started.elapsed().as_secs_f64() * 1000.0);
        Ok(())
    }

    fn decoded_sample(
        &mut self,
        chunk: &TeacherChunkV1,
        row: &TeacherSampleV1,
    ) -> Result<(), String> {
        let slot = chunk.baseline_slot as usize;
        if row.ordinal <= self.last_ordinal[slot] {
            return Err("capture sample order regressed".into());
        }
        self.last_ordinal[slot] = row.ordinal;
        self.report.samples_drained[slot] += 1;
        SampleView {
            generation: chunk.generation,
            physical_id: chunk.physical_snake_id,
            step: row.step,
            ordinal: row.ordinal,
            observation: &row.observation,
            applied_turn: row.applied_turn,
            flags: [
                row.boost_request,
                row.realized_boost,
                row.post_step_death,
                row.first_physical_sample,
            ],
            strategy: row.strategy,
            life_stage: row.life_stage,
            strata: row.strata,
        }
        .digest(&mut self.drained[slot]);
        Ok(())
    }

    /// Seal final partial chunks before reconciling all observed/drained/dropped ownership.
    pub(crate) fn finish(
        mut self,
        running: &mut RunningAuthorityLoop,
        measured_steps: usize,
        native_ms: f64,
    ) -> Result<CaptureQualificationReport, String> {
        let stop = Instant::now();
        running.stop_teacher_capture()?;
        self.report.stop_cost_ms = stop.elapsed().as_secs_f64() * 1000.0;
        self.snapshot(running)?;
        while self.snapshot(running)?.queued_chunks != 0 {
            self.drain(running)?;
        }
        let final_state = self.snapshot(running)?;
        self.report.dropped_chunks = final_state.dropped_chunks;
        self.report.dropped_samples = final_state.dropped_samples;
        self.report.dropped_bytes = final_state.dropped_bytes;
        self.report.count_limited_dropped_chunks = final_state.count_limited_dropped_chunks;
        self.report.byte_limited_dropped_chunks = final_state.byte_limited_dropped_chunks;
        self.report.production_watermark = final_state.production_watermark;
        self.report.released_through = final_state.released_through;
        let observed: u64 = self.report.samples_observed.iter().sum();
        let drained: u64 = self.report.samples_drained.iter().sum();
        self.report.queue_accounting_matches = final_state
            .count_limited_dropped_chunks
            .checked_add(final_state.byte_limited_dropped_chunks)
            == Some(final_state.dropped_chunks)
            && observed == drained + final_state.dropped_samples
            && final_state.production_watermark
                == self.report.chunks_drained + final_state.dropped_chunks
            && final_state.released_through == final_state.production_watermark;
        self.report.source_sample_sha256 = self.source.map(|hash| format!("{:x}", hash.finalize()));
        self.report.drained_sample_sha256 =
            self.drained.map(|hash| format!("{:x}", hash.finalize()));
        if observed != 0 && final_state.dropped_chunks == 0 {
            self.report.exact_samples_match = Some(
                self.report.samples_observed == self.report.samples_drained
                    && self.report.source_sample_sha256 == self.report.drained_sample_sha256,
            );
        }
        let encoded = self.report.bytes_drained + final_state.dropped_bytes;
        let seconds = measured_steps as f64 / 60.0;
        self.report.samples_per_simulated_second = observed as f64 / seconds;
        self.report.encoded_bytes_per_simulated_second = encoded as f64 / seconds;
        self.report.encoded_bytes_per_native_service_second = encoded as f64
            / ((native_ms + self.report.stop_cost_ms) / 1000.0).max(f64::MIN_POSITIVE);
        if !self.drain_times.is_empty() {
            self.report.drain_cost = Some(super::sensor_qualification::distribution(
                &mut self.drain_times,
            ));
        }
        if !self.report.queue_accounting_matches || self.report.exact_samples_match == Some(false) {
            return Err("capture source/transport accounting or exact sample mismatch".into());
        }
        Ok(self.report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_capture_pressure_rejects_outside_the_bounded_envelope() {
        for (drain_every_steps, queue_chunks) in [(0, 1), (60001, 1), (1, 0), (1, 65537)] {
            assert!(CaptureQualificationOptions {
                drain_every_steps,
                queue_chunks
            }
            .validate()
            .is_err());
        }
        for (drain_every_steps, queue_chunks) in [(1, 1), (60000, 65536)] {
            assert!(CaptureQualificationOptions {
                drain_every_steps,
                queue_chunks
            }
            .validate()
            .is_ok());
        }
    }
}
