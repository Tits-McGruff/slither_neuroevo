//! Replaceable committed display data, independent of reliable authority replies.

use std::sync::{Mutex, MutexGuard, TryLockError};

use super::control_phase::FocusedVisualization;
use super::error::{EngineError, EngineErrorCode};
use super::frame_v1::{FrameV1Error, FrameV1Metadata};
use super::queues::OutputQueue;
use super::running_loop::{RunningAuthorityLoop, RunningAuthorityLoopState};

/// Basic stats and exact chronology of one complete cached frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningDisplayStatus {
    pub sequence: u64,
    pub world_epoch: u64,
    pub completed_step: u64,
    pub generation_time: f64,
    pub alive_population: usize,
    pub baseline_bots_alive: usize,
    pub baseline_bots_total: usize,
    pub frame: FrameV1Metadata,
}

/// One compact layer descriptor for the opt-in browser neural visualizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningVisualizationLayer {
    pub count: usize,
    pub has_activations: bool,
    pub recurrent: bool,
}

/// Latest complete single-brain visualization copied from one published step.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningVisualizationStatus {
    pub sequence: u64,
    pub world_epoch: u64,
    pub completed_step: u64,
    pub frame_v1_id: u32,
    pub layers: Vec<RunningVisualizationLayer>,
    pub values: Vec<f32>,
}

/// A failed or skipped copy never changes the caller's destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrameCopyResult {
    /// Packing or priority output currently owns the opportunity to proceed.
    Busy,
    /// No frame newer than the caller's last copy exists yet.
    Unchanged,
    /// Retry with at least this frame's byte length; the cache remains available.
    TooSmall(RunningDisplayStatus),
    /// The destination prefix contains exactly the frame described here.
    Copied(RunningDisplayStatus),
}

#[derive(Debug)]
struct CachedDisplay {
    bytes: Vec<u8>,
    status: Option<RunningDisplayStatus>,
    packed_at_ms: u64,
    visualization_sequence: u64,
    visualization: Option<RunningVisualizationStatus>,
}

/// One reusable Rust buffer charged to authority's admitted frame allocation.
/// Neither producer nor consumer waits for the other. Node copies into its own
/// send buffers, so subsequent packs cannot overwrite an in-flight socket send.
#[derive(Debug)]
pub struct RunningDisplayCache {
    cache: Mutex<CachedDisplay>,
}

impl RunningDisplayCache {
    /// Reserve once before the authority handoff becomes externally visible.
    pub(crate) fn new(maximum_bytes: usize) -> Result<Self, EngineError> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(maximum_bytes).map_err(|_| {
            EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "cannot reserve admitted display storage",
            )
        })?;
        if bytes.capacity() > maximum_bytes {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "display allocation exceeds admitted storage",
            ));
        }
        Ok(Self {
            cache: Mutex::new(CachedDisplay {
                bytes,
                status: None,
                packed_at_ms: 0,
                visualization_sequence: 0,
                visualization: None,
            }),
        })
    }

    /// Read cached welcome metadata without traversing or locking authority.
    pub fn latest(&self) -> Result<Option<RunningDisplayStatus>, EngineError> {
        Ok(self.try_cache()?.and_then(|cache| cache.status))
    }

    /// Copy the newest focused snapshot only when its sequence advanced.
    pub fn latest_visualization(
        &self,
        after_sequence: u64,
    ) -> Result<Option<RunningVisualizationStatus>, EngineError> {
        Ok(self.try_cache()?.and_then(|cache| {
            cache
                .visualization
                .as_ref()
                .filter(|value| value.sequence > after_sequence)
                .cloned()
        }))
    }

    /// Drop a prior subscription's snapshot before capture is disabled.
    pub(crate) fn clear_visualization(&self) -> Result<(), EngineError> {
        if let Some(mut cache) = self.try_cache()? {
            cache.visualization = None;
        }
        Ok(())
    }

    /// Replace the focused snapshot without touching the frame cache.
    pub(crate) fn publish_visualization(
        &self,
        world_epoch: u64,
        visualization: FocusedVisualization<'_>,
    ) -> Result<(), EngineError> {
        let Some(mut cache) = self.try_cache()? else {
            return Ok(());
        };
        if cache.visualization.as_ref().is_some_and(|prior| {
            prior.world_epoch == world_epoch
                && prior.completed_step == visualization.completed_step
                && prior.frame_v1_id == visualization.frame_v1_id
        }) {
            return Ok(());
        }
        cache.visualization_sequence = cache.visualization_sequence.saturating_add(1);
        let sequence = cache.visualization_sequence;
        let mut status = cache
            .visualization
            .take()
            .unwrap_or(RunningVisualizationStatus {
                sequence,
                world_epoch,
                completed_step: visualization.completed_step,
                frame_v1_id: visualization.frame_v1_id,
                layers: Vec::new(),
                values: Vec::new(),
            });
        status.sequence = sequence;
        status.world_epoch = world_epoch;
        status.completed_step = visualization.completed_step;
        status.frame_v1_id = visualization.frame_v1_id;
        status.layers.clear();
        status.layers.extend(
            visualization
                .layers
                .iter()
                .map(|layer| RunningVisualizationLayer {
                    count: layer.count(),
                    has_activations: layer.has_activations(),
                    recurrent: layer.is_recurrent(),
                }),
        );
        status.values.clear();
        status.values.extend_from_slice(visualization.values);
        cache.visualization = Some(status);
        Ok(())
    }

    /// Pin the selected cache before checking reliable-output priority. The
    /// Rust-only predicate must not invoke JavaScript or retain caller memory.
    pub(crate) fn copy_latest(
        &self,
        destination: &mut [u8],
        after_sequence: u64,
        may_copy: impl FnOnce() -> bool,
    ) -> Result<FrameCopyResult, EngineError> {
        let Some(cache) = self.try_cache()? else {
            return Ok(FrameCopyResult::Busy);
        };
        if !may_copy() {
            return Ok(FrameCopyResult::Busy);
        }
        let Some(status) = cache
            .status
            .filter(|status| status.sequence > after_sequence)
        else {
            return Ok(FrameCopyResult::Unchanged);
        };
        if destination.len() < status.frame.byte_length {
            return Ok(FrameCopyResult::TooSmall(status));
        }
        destination[..status.frame.byte_length].copy_from_slice(&cache.bytes);
        Ok(FrameCopyResult::Copied(status))
    }

    /// Sample only complete committed state, at most about 30 times per second.
    /// A new generation or a blocked boundary gets its final sample immediately.
    pub(crate) fn publish_if_due(
        &self,
        running: &RunningAuthorityLoop,
        now_ms: u64,
        output: &OutputQueue,
    ) -> Result<(), EngineError> {
        let Some(mut cache) = self.try_cache()? else {
            return Ok(());
        };
        if let Some(prior) = cache.status {
            let same_world = prior.world_epoch == running.world_epoch();
            if same_world && prior.completed_step == running.completed_step() {
                return Ok(());
            }
            if same_world
                && running.state() == RunningAuthorityLoopState::Ready
                && now_ms.saturating_sub(cache.packed_at_ms) < 33
            {
                return Ok(());
            }
        }
        let sequence = cache
            .status
            .map_or(1, |status| status.sequence.saturating_add(1));
        let status = match running.pack_display_into(sequence, &mut cache.bytes) {
            Ok(status) => status,
            // A successor with a larger display requirement may drop visuals;
            // it cannot allocate outside this cache's original admitted budget.
            Err(FrameV1Error::FrameTooLarge { .. }) => return Ok(()),
            Err(error) => {
                return Err(EngineError::new(
                    EngineErrorCode::Faulted,
                    error.to_string(),
                ))
            }
        };
        cache.status = Some(status);
        cache.packed_at_ms = now_ms;
        drop(cache);
        // Metadata is replaceable. Rejection retains reliable output and never
        // rolls back or advances authority; the latest frame remains readable.
        output.replace_running_display(status)?;
        Ok(())
    }

    fn try_cache(&self) -> Result<Option<MutexGuard<'_, CachedDisplay>>, EngineError> {
        match self.cache.try_lock() {
            Ok(cache) => Ok(Some(cache)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err(EngineError::new(
                EngineErrorCode::Faulted,
                "display cache was interrupted while packing",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::contract::{CompletedEvent, OutputLimits, ReliableEvent};
    use crate::engine::queues::{NoopWakeSink, ReplaceResult};
    use std::mem::size_of;
    use std::sync::Arc;

    fn status(sequence: u64) -> RunningDisplayStatus {
        RunningDisplayStatus {
            sequence,
            world_epoch: 1,
            completed_step: sequence,
            generation_time: 1.0,
            alive_population: 0,
            baseline_bots_alive: 0,
            baseline_bots_total: 0,
            frame: FrameV1Metadata {
                generation: 1,
                total_snakes: 0,
                alive_snakes: 0,
                pellets: 0,
                float_length: 8,
                byte_length: 32,
            },
        }
    }

    #[test]
    fn copy_is_atomic_retryable_and_never_borrows_send_storage() {
        let cache = RunningDisplayCache::new(64).unwrap();
        let allocation = {
            let mut data = cache.cache.lock().unwrap();
            data.bytes.resize(32, 7);
            data.status = Some(status(1));
            data.bytes.as_ptr()
        };
        let mut short = [19; 31];
        assert_eq!(
            cache.copy_latest(&mut short, 0, || true).unwrap(),
            FrameCopyResult::TooSmall(status(1))
        );
        assert_eq!(short, [19; 31]);
        for _ in 0..10 {
            assert_eq!(cache.latest().unwrap(), Some(status(1)));
        }
        let mut send = [19; 40];
        assert_eq!(
            cache.copy_latest(&mut send[4..36], 0, || true).unwrap(),
            FrameCopyResult::Copied(status(1))
        );
        assert_eq!(&send[..4], &[19; 4]);
        assert_eq!(&send[4..36], &[7; 32]);
        assert_eq!(&send[36..], &[19; 4]);
        assert_eq!(
            cache.copy_latest(&mut send, 1, || true).unwrap(),
            FrameCopyResult::Unchanged
        );
        {
            let mut data = cache.cache.lock().unwrap();
            assert_eq!(
                cache.copy_latest(&mut send, 0, || true).unwrap(),
                FrameCopyResult::Busy
            );
            assert!(cache.latest().unwrap().is_none());
            data.bytes.fill(8);
            data.status = Some(status(2));
            assert_eq!(data.bytes.as_ptr(), allocation);
        }
        assert_eq!(&send[4..36], &[7; 32]);
        assert_eq!(
            cache.copy_latest(&mut send[..32], 1, || true).unwrap(),
            FrameCopyResult::Copied(status(2))
        );
        assert_eq!(&send[..32], &[8; 32]);
    }

    #[test]
    fn display_metadata_coalesces_and_restores_behind_reliable_output() {
        let bytes = size_of::<RunningDisplayStatus>();
        let queue = OutputQueue::new(
            OutputLimits {
                max_reliable: 4,
                max_reliable_owned_bytes: bytes,
                max_discrete: 1,
                max_discrete_owned_bytes: bytes,
                max_total_owned_bytes: bytes * 2,
                max_event_owned_bytes: bytes,
                max_frame_connections: 1,
            },
            Arc::new(NoopWakeSink),
        );
        assert_eq!(
            queue.replace_running_display(status(1)).unwrap(),
            ReplaceResult::Inserted
        );
        assert_eq!(
            queue.replace_running_display(status(2)).unwrap(),
            ReplaceResult::Replaced
        );
        assert_eq!(
            queue.replace_running_display(status(1)).unwrap(),
            ReplaceResult::Stale
        );
        assert!(queue.drain(4, bytes - 1).events.is_empty());
        assert_eq!(queue.metrics().total_owned_bytes, bytes);
        queue.push_reliable(ReliableEvent::Started).unwrap();
        assert!(queue.display_copy_blocked());
        assert_eq!(
            queue.drain(1, bytes).events,
            vec![CompletedEvent::Reliable(ReliableEvent::Started)]
        );
        assert!(!queue.display_copy_blocked());
        assert_eq!(
            queue.drain(4, bytes).events,
            vec![CompletedEvent::RunningDisplay(status(2))]
        );
        assert_eq!(queue.metrics().total_owned_bytes, 0);
        queue.replace_running_display(status(3)).unwrap();
        queue
            .push_reliable(ReliableEvent::ProbeResult {
                sequence: 1,
                correlation_id: 1,
                payload: vec![0; bytes],
            })
            .unwrap();
        queue
            .push_discrete(crate::engine::contract::DiscreteEvent {
                sequence: 1,
                payload: vec![0; bytes],
            })
            .unwrap();
        assert_eq!(
            queue.replace_running_display(status(4)).unwrap(),
            ReplaceResult::Rejected
        );
        assert_eq!(queue.metrics().total_owned_bytes, bytes * 2);
        assert!(!queue.metrics().has_stats);
    }

    #[test]
    fn cache_cannot_advance_between_priority_admission_and_copy() {
        let cache = RunningDisplayCache::new(32).unwrap();
        {
            let mut retained = cache.cache.lock().unwrap();
            retained.bytes.resize(32, 9);
            retained.status = Some(status(2));
        }
        let queue = OutputQueue::new(
            OutputLimits {
                max_reliable: 4,
                max_reliable_owned_bytes: 1024,
                max_discrete: 1,
                max_discrete_owned_bytes: 1024,
                max_total_owned_bytes: 2048,
                max_event_owned_bytes: 1024,
                max_frame_connections: 1,
            },
            Arc::new(NoopWakeSink),
        );
        let mut send = [17; 32];
        let result = cache
            .copy_latest(&mut send, 0, || {
                // Inject a lifecycle event at admission. Publication cannot replace
                // the selected cache while this priority check and copy are pending.
                assert!(matches!(
                    cache.cache.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                queue.push_reliable(ReliableEvent::Started).unwrap();
                !queue.display_copy_blocked()
            })
            .unwrap();
        assert_eq!(result, FrameCopyResult::Busy);
        assert_eq!(send, [17; 32]);
        queue.drain(4, 1024);
        assert_eq!(
            cache
                .copy_latest(&mut send, 0, || !queue.display_copy_blocked())
                .unwrap(),
            FrameCopyResult::Copied(status(2))
        );
        assert_eq!(send, [9; 32]);
    }
}
