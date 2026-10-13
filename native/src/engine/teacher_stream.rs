//! Committed diagnostic sequence assembly. Physical death and elapsed time are
//! processed even on fixed steps with no due control observation.

use super::teacher_capture::PublishedTeacherBoundary;
use super::teacher_queue::{TeacherChunkQueue, TeacherQueueDrain, TeacherQueueSnapshot};
use super::teacher_samples::{
    TeacherChunkV1, TeacherCodecError, TeacherContractV1, TeacherEndReason, TeacherSampleV1,
    TeacherStartReason,
};

/// At most sixty delivered controls plus the sample at the sealing boundary.
const STREAM_ROWS: usize = 61;

/// Reusable cold-admitted rows for one baseline stream; sample ordinal spans lives.
#[derive(Debug)]
struct TeacherStream {
    slot: u32,
    generation: u64,
    physical_id: u64,
    next_ordinal: u64,
    rows: Vec<TeacherSampleV1>,
    count: usize,
    next_start: TeacherStartReason,
    active_start: TeacherStartReason,
}

/// Capture-local assembly and separate queue, with no authoritative fields or RNG.
#[derive(Debug)]
pub struct TeacherStreamCapture {
    contract: TeacherContractV1,
    streams: Vec<TeacherStream>,
    queue: TeacherChunkQueue,
    last_step: Option<u64>,
    stopped: bool,
}
impl TeacherStreamCapture {
    /// Immutable identity available to the coarse diagnostic transport.
    pub fn contract(&self) -> &TeacherContractV1 {
        &self.contract
    }

    /// A stopped epoch retains its queue until opaque ownership is drained.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// Ordered cold deaths seal old observations without changing their outcome labels.
    pub fn observe_cold_deaths(
        &mut self,
        world: &super::state::WorldState,
    ) -> Result<(), TeacherCodecError> {
        if self.stopped {
            return Ok(());
        }
        for index in 0..self.streams.len() {
            let stream = &self.streams[index];
            if stream.count > 0
                && !world.snakes.iter().any(|snake| {
                    snake.id == stream.physical_id
                        && snake.baseline_slot == Some(stream.slot)
                        && snake.alive
                })
            {
                self.seal(index, TeacherEndReason::LifeEnded)?;
            }
        }
        Ok(())
    }
    /// Admit all reusable rows and encoding-copy headroom before capture begins.
    pub fn new(
        contract: TeacherContractV1,
        sensor: &super::sensor_contract::SensorContractIdentity,
        slots: &[u32],
        pending_budget: usize,
        queue_chunks: usize,
        queue_budget: usize,
    ) -> Result<Self, TeacherCodecError> {
        if slots.is_empty()
            || slots.len() > 120
            || slots.iter().any(|slot| *slot >= 120)
            || slots.windows(2).any(|pair| pair[0] >= pair[1])
            || pending_budget > 16 * 1024 * 1024
        {
            return Err(TeacherCodecError("invalid teacher stream selection"));
        }
        // Admit exact Rust canonical metadata, including scalar index meanings.
        if sensor.profile() == super::sensor_contract::SensorProfile::V3
            || sensor.input_size() != contract.input_size
            || sensor
                .sha256()
                .map_err(|_| TeacherCodecError("teacher sensor digest"))?
                != contract.sensor_sha256
            || sensor
                .order()
                .get(21)
                .is_none_or(|label| label != "first_sample_of_life")
            || sensor
                .order()
                .get(22)
                .is_none_or(|label| label != "pellet_query_saturated")
            || sensor
                .order()
                .get(23)
                .is_none_or(|label| label != "body_or_ray_query_saturated")
        {
            return Err(TeacherCodecError("teacher stream requires v4 width"));
        }
        let required = slots
            .len()
            .checked_mul(STREAM_ROWS)
            .and_then(|rows| {
                rows.checked_mul(contract.input_size * 4 + std::mem::size_of::<TeacherSampleV1>())
            })
            .and_then(|bytes| bytes.checked_mul(2))
            .and_then(|bytes| {
                bytes.checked_add(
                    slots.len()
                        * (contract.input_size * 4
                            + std::mem::size_of::<super::teacher_capture::PreparedTeacherDecision>(
                            )
                            + 4),
                )
            })
            .and_then(|bytes| {
                bytes.checked_add(
                    2 * std::mem::size_of::<super::teacher_capture::TeacherPhysicalOutcomes>(),
                )
            })
            .ok_or(TeacherCodecError("teacher stream byte arithmetic"))?;
        if required > pending_budget {
            return Err(TeacherCodecError("teacher stream pending budget"));
        }
        let queue = TeacherChunkQueue::new(contract.capture_epoch, queue_chunks, queue_budget)?;
        let mut streams = Vec::new();
        streams
            .try_reserve_exact(slots.len())
            .map_err(|_| TeacherCodecError("teacher stream allocation"))?;
        for slot in slots {
            let mut rows = Vec::new();
            rows.try_reserve_exact(STREAM_ROWS)
                .map_err(|_| TeacherCodecError("teacher row allocation"))?;
            for _ in 0..STREAM_ROWS {
                let mut observation = Vec::new();
                observation
                    .try_reserve_exact(contract.input_size)
                    .map_err(|_| TeacherCodecError("teacher observation allocation"))?;
                observation.resize(contract.input_size, 0.0);
                rows.push(TeacherSampleV1 {
                    step: 0,
                    ordinal: 0,
                    observation,
                    applied_turn: 0.0,
                    boost_request: false,
                    realized_boost: false,
                    post_step_death: false,
                    first_physical_sample: false,
                    strategy: 0,
                    life_stage: 0,
                    strata: 0,
                });
            }
            streams.push(TeacherStream {
                slot: *slot,
                generation: 0,
                physical_id: 0,
                next_ordinal: 1,
                rows,
                count: 0,
                next_start: TeacherStartReason::CaptureStart,
                active_start: TeacherStartReason::CaptureStart,
            });
        }
        let capture = Self {
            contract,
            streams,
            queue,
            last_step: None,
            stopped: false,
        };
        // Reuse the wire validator cold; malformed identity cannot survive until a seal.
        let mut probe = capture.empty_chunk(0, TeacherEndReason::CaptureStopped);
        probe.generation = 1;
        probe.physical_snake_id = 1;
        probe.samples.push(TeacherSampleV1 {
            step: 1,
            ordinal: 1,
            observation: vec![0.0; capture.contract.input_size],
            applied_turn: 0.0,
            boost_request: false,
            realized_boost: false,
            post_step_death: false,
            first_physical_sample: false,
            strategy: 0,
            life_stage: 0,
            strata: 0,
        });
        probe.validate()?;
        Ok(capture)
    }

    /// Consume one complete published step, including held-control death/time boundaries.
    /// No pending prepare can construct the publication type through its public API.
    pub fn observe(
        &mut self,
        boundary: PublishedTeacherBoundary<'_>,
        generation_ended: bool,
    ) -> Result<(), TeacherCodecError> {
        let step = boundary
            .key
            .source_completed_step()
            .checked_add(1)
            .ok_or(TeacherCodecError("teacher step exhausted"))?;
        if self.stopped
            || self
                .last_step
                .is_some_and(|last| last.checked_add(1) != Some(step))
            || boundary.input_size != self.contract.input_size
            || boundary.key.config_hash() != self.contract.config_sha256
        {
            return Err(TeacherCodecError(
                "teacher publication contract/chronology mismatch",
            ));
        }
        // Validate the entire diagnostic join before appending or sealing anything.
        for (index, decision) in boundary.decisions.iter().enumerate() {
            if !self
                .streams
                .iter()
                .any(|stream| stream.slot == decision.baseline_slot)
                || boundary.decisions[..index]
                    .iter()
                    .any(|before| before.baseline_slot == decision.baseline_slot)
                || boundary.outcome(index).is_none()
                || boundary.observation(index).is_none_or(|row| {
                    row.len() != self.contract.input_size
                        || row
                            .iter()
                            .any(|value| !value.is_finite() || value.abs() > 1.0)
                })
            {
                return Err(TeacherCodecError("teacher publication row ownership"));
            }
        }
        for stream_index in 0..self.streams.len() {
            let slot = self.streams[stream_index].slot;
            let decision_index = boundary
                .decisions
                .iter()
                .position(|decision| decision.baseline_slot == slot);
            if let Some(index) = decision_index {
                let decision = boundary.decisions[index];
                let observation = boundary.observation(index).expect("prevalidated row");
                let outcome = boundary
                    .outcome(index)
                    .expect("prevalidated physical identity");
                let generation = boundary.key.generation();
                if self.streams[stream_index].count > 0
                    && (self.streams[stream_index].physical_id != decision.physical_snake_id
                        || self.streams[stream_index].generation != generation)
                {
                    let reason = if self.streams[stream_index].generation != generation {
                        TeacherEndReason::GenerationEnded
                    } else {
                        TeacherEndReason::LifeEnded
                    };
                    self.seal(stream_index, reason)?;
                }
                // A sample at the elapsed boundary belongs to the next chunk when
                // an earlier chunk already spans one second. No synthetic rows.
                if self.streams[stream_index].count > 0
                    && step - self.streams[stream_index].rows[0].step > 60
                {
                    self.seal(stream_index, TeacherEndReason::SimulatedSecond)?;
                }
                let stream = &mut self.streams[stream_index];
                if stream.count == 0 {
                    let first = observation[21] == 1.0;
                    stream.active_start = if (stream.next_start == TeacherStartReason::CaptureStart
                        || (stream.next_start == TeacherStartReason::Continuation
                            && stream.physical_id != decision.physical_snake_id))
                        && first
                    {
                        TeacherStartReason::PhysicalLifeStart
                    } else {
                        stream.next_start
                    };
                    stream.physical_id = decision.physical_snake_id;
                    stream.generation = generation;
                }
                let row = &mut stream.rows[stream.count];
                row.observation.copy_from_slice(observation);
                row.step = step;
                row.ordinal = stream.next_ordinal;
                stream.next_ordinal = stream
                    .next_ordinal
                    .checked_add(1)
                    .ok_or(TeacherCodecError("teacher sample sequence exhausted"))?;
                row.applied_turn = decision.applied_turn;
                row.boost_request = decision.boost_request;
                row.realized_boost = outcome.realized_boost;
                row.post_step_death = outcome.post_step_death;
                row.first_physical_sample = observation[21] == 1.0;
                row.strategy = match decision.strategy {
                    super::state::BaselineStrategyState::Roam => 0,
                    super::state::BaselineStrategyState::Seek => 1,
                    super::state::BaselineStrategyState::Avoid => 2,
                    super::state::BaselineStrategyState::Boost => 3,
                };
                row.life_stage = match decision.life_stage {
                    super::baseline_control::BaselineLifeStage::Small => 0,
                    super::baseline_control::BaselineLifeStage::Medium => 1,
                    super::baseline_control::BaselineLifeStage::Large => 2,
                };
                row.strata = u16::from(decision.unsafe_route || decision.urgent_sectors != 0)
                    | (u16::from(decision.escape_or_pursuit) << 1)
                    | (u16::from(decision.boost_request) << 2)
                    | (u16::from(decision.strategy_transition) << 3)
                    | (u16::from(observation[22] == 1.0 || observation[23] == 1.0) << 4)
                    | (u16::from(row.first_physical_sample) << 5);
                stream.count += 1;
            }
            let stream = &self.streams[stream_index];
            let outcome = boundary.slot_outcome(slot);
            let death = outcome.is_some_and(|outcome| {
                outcome.physical_snake_id == stream.physical_id && outcome.post_step_death
            });
            let elapsed = stream.count > 0 && step - stream.rows[0].step >= 60;
            if stream.count > 0
                && (generation_ended || death || elapsed || stream.count == STREAM_ROWS)
            {
                let end = if generation_ended {
                    TeacherEndReason::GenerationEnded
                } else if death {
                    TeacherEndReason::LifeEnded
                } else {
                    TeacherEndReason::SimulatedSecond
                };
                self.seal(stream_index, end)?;
            }
        }
        self.last_step = Some(step);
        Ok(())
    }

    /// Seal only complete committed data; stop is irreversible for this epoch.
    pub fn stop(&mut self, reason: TeacherEndReason) -> Result<(), TeacherCodecError> {
        if !matches!(
            reason,
            TeacherEndReason::CaptureStopped
                | TeacherEndReason::ContractChanged
                | TeacherEndReason::Interrupted
        ) {
            return Err(TeacherCodecError("invalid capture stop reason"));
        }
        self.stopped = true;
        for index in 0..self.streams.len() {
            self.seal(index, reason)?;
        }
        Ok(())
    }

    /// Independently bounded opaque ownership transfer to the spool worker.
    pub fn drain(
        &mut self,
        max_chunks: usize,
        max_owned_bytes: usize,
    ) -> Result<TeacherQueueDrain, TeacherCodecError> {
        self.queue.drain(max_chunks, max_owned_bytes)
    }
    pub fn snapshot(&self) -> TeacherQueueSnapshot {
        self.queue.snapshot()
    }

    fn empty_chunk(&self, index: usize, end: TeacherEndReason) -> TeacherChunkV1 {
        let stream = &self.streams[index];
        TeacherChunkV1 {
            contract: self.contract.clone(),
            chunk_sequence: 1,
            generation: stream.generation,
            baseline_slot: stream.slot,
            physical_snake_id: stream.physical_id,
            start_reason: stream.active_start,
            end_reason: end,
            samples: Vec::new(),
        }
    }

    fn seal(&mut self, index: usize, end: TeacherEndReason) -> Result<(), TeacherCodecError> {
        let stream = &self.streams[index];
        if stream.count == 0 {
            return Ok(());
        }
        let mut chunk = self.empty_chunk(index, end);
        chunk
            .samples
            .try_reserve_exact(stream.count)
            .map_err(|_| TeacherCodecError("teacher sealing row allocation"))?;
        for source in &stream.rows[..stream.count] {
            let mut observation = Vec::new();
            observation
                .try_reserve_exact(source.observation.len())
                .map_err(|_| TeacherCodecError("teacher sealing observation allocation"))?;
            observation.extend_from_slice(&source.observation);
            chunk.samples.push(TeacherSampleV1 {
                step: source.step,
                ordinal: source.ordinal,
                observation,
                applied_turn: source.applied_turn,
                boost_request: source.boost_request,
                realized_boost: source.realized_boost,
                post_step_death: source.post_step_death,
                first_physical_sample: source.first_physical_sample,
                strategy: source.strategy,
                life_stage: source.life_stage,
                strata: source.strata,
            });
        }
        let admitted = self.queue.seal(chunk)?;
        let stream = &mut self.streams[index];
        stream.count = 0;
        stream.next_start = if !admitted {
            TeacherStartReason::Gap
        } else {
            match end {
                TeacherEndReason::LifeEnded => TeacherStartReason::PhysicalLifeStart,
                TeacherEndReason::GenerationEnded => TeacherStartReason::GenerationStart,
                _ => TeacherStartReason::Continuation,
            }
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::baseline_control::BaselineLifeStage;
    use crate::engine::baseline_v2::{BaselineV2Action, BaselineV2State};
    use crate::engine::physics::PhysicsStepKey;
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
    use crate::engine::state::BaselineStrategyState;
    use crate::engine::teacher_capture::{
        TeacherBoundaryWorkspace, TeacherPhysicalOutcome, TeacherPhysicalOutcomes,
    };

    fn capture(queue_chunks: usize) -> TeacherStreamCapture {
        let sensor = SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap();
        TeacherStreamCapture::new(
            TeacherContractV1 {
                capture_epoch: [17; 16],
                run_id: "run".into(),
                config_sha256: [23; 32],
                sensor_sha256: sensor.sha256().unwrap(),
                teacher_sha256: [37; 32],
                teacher_revision: 2,
                teacher_mode: 1,
                control_interval_seconds: 0.06,
                input_size: sensor.input_size(),
            },
            &sensor,
            &[7],
            1024 * 1024,
            queue_chunks,
            2 * 1024 * 1024,
        )
        .unwrap()
    }

    struct Tick {
        step: u64,
        generation: u64,
        id: u64,
        delivered: bool,
        first: bool,
        dead: bool,
        end: bool,
    }
    impl Default for Tick {
        fn default() -> Self {
            Self {
                step: 1,
                generation: 1,
                id: 71,
                delivered: true,
                first: false,
                dead: false,
                end: false,
            }
        }
    }
    fn feed(capture: &mut TeacherStreamCapture, tick: Tick) -> Result<(), TeacherCodecError> {
        let mut rows = TeacherBoundaryWorkspace::default();
        rows.configure(&[7], 144).unwrap();
        let mut observation = vec![-0.0; 144];
        observation[21] = if tick.first { 1.0 } else { -1.0 };
        observation[22] = 1.0;
        observation[23] = -1.0;
        if tick.delivered {
            rows.stage(
                7,
                tick.id,
                &observation,
                BaselineV2Action {
                    turn: -0.0,
                    boost: true,
                    sector: 0,
                    life_stage: BaselineLifeStage::Small,
                    normalized_score: 0.0,
                    unsafe_route: false,
                    escape_or_pursuit: true,
                    urgent_sectors: 0,
                    next_state: BaselineV2State {
                        strategy: BaselineStrategyState::Boost,
                        ..BaselineV2State::default()
                    },
                    next_behavior_rng_word: 17,
                    uniform_draws: 0,
                },
                BaselineStrategyState::Roam,
            )
            .unwrap();
        }
        let mut outcomes = TeacherPhysicalOutcomes::default();
        outcomes.0[7] = TeacherPhysicalOutcome {
            physical_snake_id: tick.id,
            realized_boost: true,
            post_step_death: tick.dead,
        };
        let key = PhysicsStepKey::new(1, tick.generation, tick.step - 1, 1, 1, [23; 32], tick.step);
        capture.observe(
            PublishedTeacherBoundary::new(rows.prepared(key), &outcomes),
            tick.end,
        )
    }

    #[test]
    fn one_second_seals_on_held_steps_and_chunk_continuity_does_not_reset() {
        let mut capture = capture(4);
        for step in 1..=61 {
            feed(
                &mut capture,
                Tick {
                    step,
                    delivered: step == 1,
                    ..Tick::default()
                },
            )
            .unwrap();
        }
        let first = capture.drain(4, 65536).unwrap();
        assert_eq!(first.chunks.len(), 1);
        let decoded = TeacherChunkV1::decode(&first.chunks[0].bytes).unwrap();
        assert_eq!(decoded.start_reason, TeacherStartReason::CaptureStart);
        assert_eq!(decoded.end_reason, TeacherEndReason::SimulatedSecond);
        assert_eq!(decoded.samples.len(), 1);
        assert!(!decoded.samples[0].first_physical_sample);
        assert_eq!(
            decoded.samples[0].applied_turn.to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(
            decoded.samples[0].observation[0].to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(decoded.samples[0].strata, 2 | 4 | 8 | 16);
        feed(
            &mut capture,
            Tick {
                step: 62,
                ..Tick::default()
            },
        )
        .unwrap();
        capture.stop(TeacherEndReason::CaptureStopped).unwrap();
        let second = capture.drain(4, 65536).unwrap();
        let decoded = TeacherChunkV1::decode(&second.chunks[0].bytes).unwrap();
        assert_eq!(decoded.start_reason, TeacherStartReason::Continuation);
        assert_eq!(decoded.samples[0].ordinal, 2);
        assert_eq!(decoded.samples[0].step, 62);
        assert!(feed(
            &mut capture,
            Tick {
                step: 63,
                ..Tick::default()
            }
        )
        .is_err());
    }

    #[test]
    fn held_control_death_seals_without_fabricated_sample_and_birth_generation_are_explicit() {
        let mut capture = capture(4);
        feed(&mut capture, Tick::default()).unwrap();
        feed(
            &mut capture,
            Tick {
                step: 2,
                delivered: false,
                dead: true,
                ..Tick::default()
            },
        )
        .unwrap();
        feed(
            &mut capture,
            Tick {
                step: 3,
                id: 72,
                first: true,
                end: true,
                ..Tick::default()
            },
        )
        .unwrap();
        feed(
            &mut capture,
            Tick {
                step: 4,
                generation: 2,
                id: 73,
                first: true,
                ..Tick::default()
            },
        )
        .unwrap();
        capture.stop(TeacherEndReason::CaptureStopped).unwrap();
        let drained = capture.drain(4, 65536).unwrap();
        let chunks = drained
            .chunks
            .iter()
            .map(|chunk| TeacherChunkV1::decode(&chunk.bytes).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].end_reason, TeacherEndReason::LifeEnded);
        assert!(!chunks[0].samples[0].post_step_death);
        assert_eq!(chunks[0].samples.len(), 1);
        assert_eq!(
            chunks[1].start_reason,
            TeacherStartReason::PhysicalLifeStart
        );
        assert_eq!(chunks[1].end_reason, TeacherEndReason::GenerationEnded);
        assert_eq!(chunks[1].samples[0].ordinal, 2);
        assert_eq!(chunks[2].start_reason, TeacherStartReason::GenerationStart);
        assert_eq!(chunks[2].samples[0].ordinal, 3);
        assert!(
            chunks[1].samples[0].first_physical_sample
                && chunks[2].samples[0].first_physical_sample
        );
    }

    #[test]
    fn overflow_consumes_chunk_and_sample_sequences_then_labels_gap() {
        let mut capture = capture(1);
        for step in 1..=2 {
            feed(
                &mut capture,
                Tick {
                    step,
                    generation: step,
                    id: 70 + step,
                    first: true,
                    end: true,
                    ..Tick::default()
                },
            )
            .unwrap();
        }
        assert_eq!(capture.snapshot().production_watermark, 2);
        assert_eq!(capture.snapshot().dropped_chunks, 1);
        assert_eq!(
            capture.drain(1, 65536).unwrap().snapshot.released_through,
            2
        );
        feed(
            &mut capture,
            Tick {
                step: 3,
                generation: 3,
                id: 73,
                first: true,
                end: true,
                ..Tick::default()
            },
        )
        .unwrap();
        let drained = capture.drain(1, 65536).unwrap();
        let chunk = TeacherChunkV1::decode(&drained.chunks[0].bytes).unwrap();
        assert_eq!(chunk.chunk_sequence, 3);
        assert_eq!(chunk.samples[0].ordinal, 3);
        assert_eq!(chunk.start_reason, TeacherStartReason::Gap);
    }

    #[test]
    fn malformed_contract_budget_duplicate_and_missing_step_reject() {
        let mut capture = capture(2);
        let sensor = SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap();
        let mut bad = capture.contract.clone();
        bad.sensor_sha256 = [0; 32];
        assert!(
            TeacherStreamCapture::new(bad, &sensor, &[7], 1024 * 1024, 2, 2 * 1024 * 1024).is_err()
        );
        assert!(TeacherStreamCapture::new(
            capture.contract.clone(),
            &sensor,
            &[7],
            1,
            2,
            2 * 1024 * 1024
        )
        .is_err());
        feed(&mut capture, Tick::default()).unwrap();
        assert!(feed(&mut capture, Tick::default()).is_err());
        assert!(feed(
            &mut capture,
            Tick {
                step: 3,
                ..Tick::default()
            }
        )
        .is_err());
        assert_eq!(capture.streams[0].count, 1);
        assert_eq!(capture.snapshot().production_watermark, 0);
        capture.stop(TeacherEndReason::Interrupted).unwrap();
        assert_eq!(
            capture.drain(2, 65536).unwrap().chunks[0]
                .descriptor
                .end_reason,
            TeacherEndReason::Interrupted
        );
    }
}
