//! Separate diagnostic ownership queue. No controller reservation, authority
//! acknowledgement, disk operation or consumer wait belongs to this queue.

use super::teacher_samples::{
    TeacherChunkV1, TeacherCodecError, TeacherEndReason, TeacherStartReason,
    MAX_TEACHER_CHUNK_BYTES,
};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

/// Complete identity for one immutable opaque object, constructed only by Rust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TeacherChunkDescriptorV1 {
    pub capture_epoch: [u8; 16],
    pub sequence: u64,
    pub generation: u64,
    pub baseline_slot: u32,
    pub physical_snake_id: u64,
    pub first_sample: u64,
    pub last_sample: u64,
    pub first_step: u64,
    pub last_step: u64,
    pub count: usize,
    pub input_size: usize,
    pub config_sha256: [u8; 32],
    pub sensor_sha256: [u8; 32],
    pub teacher_sha256: [u8; 32],
    pub start_reason: TeacherStartReason,
    pub end_reason: TeacherEndReason,
    pub byte_count: usize,
    pub sha256: [u8; 32],
}

/// One whole immutable transfer; Node may store bytes but must not parse samples.
#[derive(Debug)]
pub struct OwnedTeacherChunk {
    pub descriptor: TeacherChunkDescriptorV1,
    pub bytes: Vec<u8>,
}

/// Coherent counters independent of spool durability and its resolvedThrough.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TeacherQueueSnapshot {
    pub production_watermark: u64,
    /// All sequences through here have left native ownership or were dropped.
    /// This is not durable storage settlement and is never a trainer ACK.
    pub released_through: u64,
    pub queued_chunks: usize,
    pub queued_bytes: usize,
    pub dropped_chunks: u64,
    pub dropped_samples: u64,
    pub dropped_bytes: u64,
}

/// One bounded drain, including counters even when no chunk fits the byte limit.
#[derive(Debug)]
pub struct TeacherQueueDrain {
    pub chunks: Vec<OwnedTeacherChunk>,
    pub snapshot: TeacherQueueSnapshot,
}

/// Explicit admitted ownership ceilings; encoding headroom is charged separately.
#[derive(Debug)]
pub struct TeacherChunkQueue {
    epoch: [u8; 16],
    chunks: VecDeque<OwnedTeacherChunk>,
    max_chunks: usize,
    payload_budget: usize,
    counters: TeacherQueueSnapshot,
}
impl TeacherChunkQueue {
    /// Reserve the descriptor ring and one maximum encoded staging object cold.
    pub fn new(
        epoch: [u8; 16],
        max_chunks: usize,
        owned_budget: usize,
    ) -> Result<Self, TeacherCodecError> {
        if epoch == [0; 16]
            || !(1..=65_536).contains(&max_chunks)
            || owned_budget > 16 * 1024 * 1024
        {
            return Err(TeacherCodecError("invalid teacher queue admission"));
        }
        let overhead = max_chunks
            .checked_mul(std::mem::size_of::<OwnedTeacherChunk>())
            .and_then(|bytes| bytes.checked_add(MAX_TEACHER_CHUNK_BYTES))
            .ok_or(TeacherCodecError("teacher queue byte arithmetic"))?;
        let payload_budget = owned_budget
            .checked_sub(overhead)
            .filter(|bytes| *bytes > 0)
            .ok_or(TeacherCodecError(
                "teacher queue budget lacks encoding headroom",
            ))?;
        let mut chunks = VecDeque::new();
        chunks
            .try_reserve_exact(max_chunks)
            .map_err(|_| TeacherCodecError("teacher queue ring allocation"))?;
        Ok(Self {
            epoch,
            chunks,
            max_chunks,
            payload_budget,
            counters: TeacherQueueSnapshot::default(),
        })
    }

    /// Assign committed sequence before admission; overflow drops the whole new chunk.
    /// Only the committed capture assembler may call this production entry point.
    pub(crate) fn seal(&mut self, mut chunk: TeacherChunkV1) -> Result<bool, TeacherCodecError> {
        if chunk.contract.capture_epoch != self.epoch {
            return Err(TeacherCodecError("teacher queue capture epoch mismatch"));
        }
        let sequence = self
            .counters
            .production_watermark
            .checked_add(1)
            .ok_or(TeacherCodecError("teacher chunk sequence exhausted"))?;
        chunk.chunk_sequence = sequence;
        let bytes = chunk.encode()?;
        let first = &chunk.samples[0];
        let last = chunk.samples.last().expect("validated nonempty chunk");
        let descriptor = TeacherChunkDescriptorV1 {
            capture_epoch: self.epoch,
            sequence,
            generation: chunk.generation,
            baseline_slot: chunk.baseline_slot,
            physical_snake_id: chunk.physical_snake_id,
            first_sample: first.ordinal,
            last_sample: last.ordinal,
            first_step: first.step,
            last_step: last.step,
            count: chunk.samples.len(),
            input_size: chunk.contract.input_size,
            config_sha256: chunk.contract.config_sha256,
            sensor_sha256: chunk.contract.sensor_sha256,
            teacher_sha256: chunk.contract.teacher_sha256,
            start_reason: chunk.start_reason,
            end_reason: chunk.end_reason,
            byte_count: bytes.len(),
            sha256: Sha256::digest(&bytes).into(),
        };
        self.counters.production_watermark = sequence;
        if self.chunks.len() == self.max_chunks
            || bytes.len() > self.payload_budget - self.counters.queued_bytes
        {
            self.counters.dropped_chunks = self.counters.dropped_chunks.saturating_add(1);
            self.counters.dropped_samples = self
                .counters
                .dropped_samples
                .saturating_add(descriptor.count as u64);
            self.counters.dropped_bytes = self
                .counters
                .dropped_bytes
                .saturating_add(bytes.len() as u64);
            self.refresh_release_cursor();
            return Ok(false);
        }
        self.counters.queued_bytes += bytes.len();
        self.chunks
            .push_back(OwnedTeacherChunk { descriptor, bytes });
        self.counters.queued_chunks = self.chunks.len();
        Ok(true)
    }

    /// Transfer whole chunks only, within independently bounded count and ownership.
    pub fn drain(
        &mut self,
        max_chunks: usize,
        max_owned_bytes: usize,
    ) -> Result<TeacherQueueDrain, TeacherCodecError> {
        if !(1..=256).contains(&max_chunks) || max_owned_bytes > 16 * 1024 * 1024 {
            return Err(TeacherCodecError("invalid teacher drain bounds"));
        }
        let mut count = 0;
        let mut admitted_bytes = 0usize;
        for chunk in self.chunks.iter().take(max_chunks) {
            let next =
                admitted_bytes + std::mem::size_of::<OwnedTeacherChunk>() + chunk.bytes.len();
            if next > max_owned_bytes {
                break;
            }
            admitted_bytes = next;
            count += 1;
        }
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(count)
            .map_err(|_| TeacherCodecError("teacher drain allocation"))?;
        let mut owned = count * std::mem::size_of::<OwnedTeacherChunk>();
        while chunks.len() < count {
            let Some(front) = self.chunks.front() else {
                break;
            };
            if owned > max_owned_bytes || front.bytes.len() > max_owned_bytes - owned {
                break;
            }
            let chunk = self
                .chunks
                .pop_front()
                .expect("peeked complete queue entry");
            owned += chunk.bytes.len();
            self.counters.queued_bytes -= chunk.bytes.len();
            chunks.push(chunk);
        }
        self.counters.queued_chunks = self.chunks.len();
        self.refresh_release_cursor();
        Ok(TeacherQueueDrain {
            chunks,
            snapshot: self.counters,
        })
    }

    /// Coherent bounded ownership diagnostics; storage settlement belongs to Node.
    pub fn snapshot(&self) -> TeacherQueueSnapshot {
        self.counters
    }

    fn refresh_release_cursor(&mut self) {
        self.counters.released_through = self
            .chunks
            .front()
            .map_or(self.counters.production_watermark, |front| {
                front.descriptor.sequence - 1
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::teacher_samples::{TeacherContractV1, TeacherSampleV1};

    fn chunk() -> TeacherChunkV1 {
        TeacherChunkV1 {
            contract: TeacherContractV1 {
                capture_epoch: [17; 16],
                run_id: "run".into(),
                config_sha256: [23; 32],
                sensor_sha256: [31; 32],
                teacher_sha256: [37; 32],
                teacher_revision: 2,
                teacher_mode: 1,
                control_interval_seconds: 0.03,
                input_size: 24,
            },
            chunk_sequence: 999,
            generation: 1,
            baseline_slot: 7,
            physical_snake_id: 71,
            start_reason: TeacherStartReason::CaptureStart,
            end_reason: TeacherEndReason::CaptureStopped,
            samples: vec![TeacherSampleV1 {
                step: 42,
                ordinal: 1,
                observation: vec![-0.0; 24],
                applied_turn: -0.0,
                boost_request: true,
                realized_boost: true,
                post_step_death: false,
                first_physical_sample: false,
                strategy: 3,
                life_stage: 2,
                strata: 63,
            }],
        }
    }

    #[test]
    fn assigns_before_whole_chunk_drop_and_releases_visible_gaps() {
        let mut queue = TeacherChunkQueue::new([17; 16], 1, 2 * MAX_TEACHER_CHUNK_BYTES).unwrap();
        assert!(queue.seal(chunk()).unwrap());
        assert!(!queue.seal(chunk()).unwrap());
        let full = queue.snapshot();
        assert_eq!(full.production_watermark, 2);
        assert_eq!(full.released_through, 0);
        assert_eq!(full.queued_chunks, 1);
        assert_eq!(full.dropped_chunks, 1);
        assert_eq!(full.dropped_samples, 1);
        assert_eq!(full.dropped_bytes, full.queued_bytes as u64);
        assert!(queue.drain(1, 0).unwrap().chunks.is_empty());
        assert_eq!(queue.snapshot(), full);
        assert!(queue.drain(0, 4096).is_err());
        assert!(queue.drain(257, 4096).is_err());
        assert!(queue.drain(1, 16 * 1024 * 1024 + 1).is_err());
        assert_eq!(queue.snapshot(), full);
        let drained = queue.drain(1, 4096).unwrap();
        assert_eq!(drained.snapshot.released_through, 2);
        assert_eq!(drained.chunks[0].descriptor.sequence, 1);
        let decoded = TeacherChunkV1::decode(&drained.chunks[0].bytes).unwrap();
        assert_eq!(decoded.chunk_sequence, 1);
        assert_eq!(
            decoded.samples[0].observation[0].to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(
            drained.chunks[0].descriptor.sha256,
            <[u8; 32]>::from(Sha256::digest(&drained.chunks[0].bytes))
        );
        assert_eq!(
            drained.chunks[0].descriptor.byte_count,
            drained.chunks[0].bytes.len()
        );
        assert!(queue.seal(chunk()).unwrap());
        assert_eq!(
            queue.drain(1, 4096).unwrap().chunks[0].descriptor.sequence,
            3
        );
    }

    #[test]
    fn invalid_input_does_not_consume_sequence_and_payload_ceiling_drops_whole() {
        let overhead = MAX_TEACHER_CHUNK_BYTES + std::mem::size_of::<OwnedTeacherChunk>();
        assert!(TeacherChunkQueue::new([0; 16], 1, overhead + 1).is_err());
        assert!(TeacherChunkQueue::new([17; 16], 0, overhead + 1).is_err());
        assert!(TeacherChunkQueue::new([17; 16], 1, overhead).is_err());
        let mut queue = TeacherChunkQueue::new([17; 16], 1, overhead + 1).unwrap();
        let mut bad = chunk();
        bad.contract.capture_epoch = [18; 16];
        assert!(queue.seal(bad).is_err());
        let mut bad = chunk();
        bad.samples[0].observation[0] = f32::NAN;
        assert!(queue.seal(bad).is_err());
        assert_eq!(queue.snapshot(), TeacherQueueSnapshot::default());
        assert!(!queue.seal(chunk()).unwrap());
        assert_eq!(queue.snapshot().production_watermark, 1);
        assert_eq!(queue.snapshot().released_through, 1);
        assert_eq!(queue.snapshot().dropped_chunks, 1);
        assert_eq!(queue.snapshot().queued_bytes, 0);
        queue.counters.production_watermark = u64::MAX;
        let before = queue.snapshot();
        assert!(queue.seal(chunk()).is_err());
        assert_eq!(queue.snapshot(), before);
    }
}
