//! Bounded opaque diagnostic handoff. The authority owner only tries this mutex;
//! slow consumers leave native chunks queued and eventually cause whole drops.

use super::running_loop::RunningAuthorityLoop;
use super::teacher_queue::{OwnedTeacherChunk, TeacherQueueSnapshot};
use super::teacher_samples::TeacherContractV1;
use std::collections::VecDeque;
use std::sync::{Mutex, TryLockError};

/// Handoff ownership is charged inside the admitted native capture queue budget.
pub const TEACHER_MAILBOX_BYTES: usize = 1024 * 1024;
/// Exact sensor metadata is retained once by the loop and once by its mailbox.
pub const TEACHER_METADATA_HEADROOM: usize = 2 * 65_536;

#[derive(Debug, Default)]
struct MailboxState {
    shadows: [Option<super::shadows::ShadowSnapshot>; super::shadows::MAX_SHADOWS],
    contract: Option<TeacherContractV1>,
    sensor_contract_json: Option<String>,
    chunks: VecDeque<OwnedTeacherChunk>,
    snapshot: TeacherQueueSnapshot,
    stopped: bool,
    error: Option<String>,
}

/// Complete bounded response independent of reliable gameplay outputs.
#[derive(Debug)]
pub struct TeacherMailboxDrain {
    pub contract: Option<TeacherContractV1>,
    pub sensor_contract_json: Option<String>,
    pub chunks: Vec<OwnedTeacherChunk>,
    pub snapshot: TeacherQueueSnapshot,
    pub stopped: bool,
    pub error: Option<String>,
    pub more_work: bool,
}

/// Separate single producer handoff; disabled capture retains no chunk buffers.
#[derive(Debug, Default)]
pub struct TeacherMailbox {
    state: Mutex<MailboxState>,
}
impl TeacherMailbox {
    /// Try a bounded transfer from the owner; never wait on the consumer's mutex.
    pub(crate) fn service(&self, running: &mut RunningAuthorityLoop) {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        state.shadows = running.shadow_snapshots();
        let Some((contract, snapshot, stopped, error)) = running.teacher_capture_status() else {
            return;
        };
        let same_epoch = state
            .contract
            .as_ref()
            .is_some_and(|old| old.capture_epoch == contract.capture_epoch);
        if !same_epoch {
            if !state.chunks.is_empty() {
                return;
            }
            state.contract = Some(contract.clone());
            state.sensor_contract_json = running.teacher_sensor_metadata().map(str::to_owned);
        }
        state.snapshot = snapshot;
        state.stopped = stopped;
        if state.error.as_deref() != error {
            state.error = error.map(str::to_owned);
        }
        if state.chunks.is_empty() {
            match running.drain_teacher_chunks(128, TEACHER_MAILBOX_BYTES) {
                Ok(Some(drained)) => {
                    state.snapshot = drained.snapshot;
                    state.chunks = drained.chunks.into();
                }
                Ok(None) => {}
                Err(error) => state.error = Some(error),
            }
        }
    }

    /// Copy four fixed summaries without accessing authority or consuming teacher chunks.
    pub fn shadow_snapshots(
        &self,
    ) -> Result<[Option<super::shadows::ShadowSnapshot>; super::shadows::MAX_SHADOWS], &'static str>
    {
        Ok(self
            .state
            .lock()
            .map_err(|_| "shadow mailbox poisoned")?
            .shadows)
    }

    /// Move only whole objects within the caller's independent count/ownership bounds.
    pub fn drain(
        &self,
        max_chunks: usize,
        max_owned_bytes: usize,
    ) -> Result<TeacherMailboxDrain, &'static str> {
        if !(1..=256).contains(&max_chunks) || max_owned_bytes > 16 * 1024 * 1024 {
            return Err("invalid teacher mailbox drain limits");
        }
        let mut state = self.state.lock().map_err(|_| "teacher mailbox poisoned")?;
        let overhead = std::mem::size_of::<TeacherMailboxDrain>()
            + state
                .contract
                .as_ref()
                .map_or(0, |contract| contract.run_id.len())
            + state.error.as_ref().map_or(0, String::len);
        let overhead = overhead + state.sensor_contract_json.as_ref().map_or(0, String::len);
        if max_owned_bytes < overhead {
            return Err("teacher drain budget cannot fit metadata");
        }
        let mut count = 0;
        let mut owned = overhead;
        for chunk in state.chunks.iter().take(max_chunks) {
            let next = owned + chunk.bytes.len() + std::mem::size_of::<OwnedTeacherChunk>();
            if next > max_owned_bytes {
                break;
            }
            count += 1;
            owned = next;
        }
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(count)
            .map_err(|_| "teacher mailbox drain allocation")?;
        for _ in 0..count {
            chunks.push(state.chunks.pop_front().expect("admitted complete object"));
        }
        let mut snapshot = state.snapshot;
        snapshot.queued_chunks += state.chunks.len();
        snapshot.queued_bytes += state
            .chunks
            .iter()
            .map(|chunk| chunk.bytes.len())
            .sum::<usize>();
        if let Some(front) = state.chunks.front() {
            snapshot.released_through =
                snapshot.released_through.min(front.descriptor.sequence - 1);
        }
        Ok(TeacherMailboxDrain {
            contract: state.contract.clone(),
            sensor_contract_json: state.sensor_contract_json.clone(),
            chunks,
            snapshot,
            stopped: state.stopped,
            error: state.error.clone(),
            more_work: snapshot.queued_chunks > 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::teacher_queue::TeacherChunkQueue;
    use crate::engine::teacher_samples::{
        TeacherChunkV1, TeacherEndReason, TeacherSampleV1, TeacherStartReason,
    };

    fn filled() -> TeacherMailbox {
        let contract = TeacherContractV1 {
            capture_epoch: [17; 16],
            run_id: "run".into(),
            config_sha256: [23; 32],
            sensor_sha256: [31; 32],
            teacher_sha256: [37; 32],
            teacher_revision: 2,
            teacher_mode: 1,
            control_interval_seconds: 0.03,
            input_size: 24,
        };
        let mut queue = TeacherChunkQueue::new([17; 16], 4, 4 * 1024 * 1024).unwrap();
        for ordinal in 1..=2 {
            queue
                .seal(TeacherChunkV1 {
                    contract: contract.clone(),
                    chunk_sequence: 1,
                    generation: 1,
                    baseline_slot: 7,
                    physical_snake_id: 71,
                    start_reason: TeacherStartReason::Continuation,
                    end_reason: TeacherEndReason::SimulatedSecond,
                    samples: vec![TeacherSampleV1 {
                        step: ordinal,
                        ordinal,
                        observation: vec![-0.0; 24],
                        applied_turn: -0.0,
                        boost_request: false,
                        realized_boost: false,
                        post_step_death: false,
                        first_physical_sample: false,
                        strategy: 0,
                        life_stage: 0,
                        strata: 0,
                    }],
                })
                .unwrap();
        }
        let drain = queue.drain(4, 65536).unwrap();
        TeacherMailbox {
            state: Mutex::new(MailboxState {
                contract: Some(contract),
                chunks: drain.chunks.into(),
                snapshot: drain.snapshot,
                ..MailboxState::default()
            }),
        }
    }

    #[test]
    fn partial_drain_does_not_release_untransferred_objects_or_split_bytes() {
        let mailbox = filled();
        let small = mailbox
            .drain(1, std::mem::size_of::<TeacherMailboxDrain>() + 3)
            .unwrap();
        assert!(small.chunks.is_empty());
        assert_eq!(small.snapshot.production_watermark, 2);
        assert_eq!(small.snapshot.released_through, 0);
        assert_eq!(small.snapshot.queued_chunks, 2);
        let first = mailbox.drain(1, 4096).unwrap();
        assert_eq!(first.chunks.len(), 1);
        assert_eq!(first.snapshot.released_through, 1);
        assert_eq!(first.snapshot.queued_chunks, 1);
        assert!(first.more_work);
        let last = mailbox.drain(1, 4096).unwrap();
        assert_eq!(last.chunks.len(), 1);
        assert_eq!(last.snapshot.released_through, 2);
        assert_eq!(last.snapshot.queued_bytes, 0);
        assert!(!last.more_work);
        assert_eq!(
            TeacherChunkV1::decode(&first.chunks[0].bytes)
                .unwrap()
                .samples[0]
                .ordinal,
            1
        );
        assert_eq!(
            TeacherChunkV1::decode(&last.chunks[0].bytes)
                .unwrap()
                .samples[0]
                .ordinal,
            2
        );
    }

    #[test]
    fn malformed_limits_cannot_take_ownership() {
        let mailbox = filled();
        assert!(mailbox.drain(0, 4096).is_err());
        assert!(mailbox.drain(257, 4096).is_err());
        assert!(mailbox.drain(1, 0).is_err());
        assert!(mailbox.drain(1, 16 * 1024 * 1024 + 1).is_err());
        assert_eq!(mailbox.drain(2, 4096).unwrap().chunks.len(), 2);
    }

    #[cfg(feature = "engine-test-hooks")]
    #[test]
    fn authority_owner_yields_immediately_to_a_held_consumer_mutex() {
        let mut running =
            crate::engine::generation_handoff_fixture::background_generation_handoff_fixture()
                .unwrap()
                .running;
        let mailbox = TeacherMailbox::default();
        let guard = mailbox.state.lock().unwrap();
        mailbox.service(&mut running);
        drop(guard);
        assert_eq!(running.completed_step(), 0);
    }
}
