//! Scalar queue occupancy and configured limits; never inspect authoritative world data.

use napi_derive::napi;

use crate::engine::runtime::EngineRuntime;

/// Exact hexadecimal inbound counters and the limits that govern this runtime.
#[napi(object)]
pub struct InboundQueueDiagnostics {
    pub batches: String,
    pub commands: String,
    pub owned_bytes: String,
    pub high_water_batches: String,
    pub high_water_commands: String,
    pub high_water_owned_bytes: String,
    pub max_batches: String,
    pub max_commands: String,
    pub max_owned_bytes: String,
    pub max_batch_commands: String,
    pub max_batch_owned_bytes: String,
    pub rejections: String,
    pub fault_discarded_commands: String,
}

/// Exact hexadecimal output counters, including lifetime peaks and capacity waits.
#[napi(object)]
pub struct OutputQueueDiagnostics {
    pub reliable: String,
    pub reliable_owned_bytes: String,
    pub discrete: String,
    pub discrete_owned_bytes: String,
    pub frames: String,
    pub has_stats: bool,
    pub owned_bytes: String,
    pub high_water_count: String,
    pub high_water_owned_bytes: String,
    pub max_reliable: String,
    pub max_reliable_owned_bytes: String,
    pub max_discrete: String,
    pub max_discrete_owned_bytes: String,
    pub max_frames: String,
    pub max_owned_bytes: String,
    pub max_event_owned_bytes: String,
    pub capacity_waits: String,
    pub priority_overflows: String,
    pub has_reserved_fault: bool,
    pub stats_replacements: String,
    pub frame_replacements: String,
    pub stale_stats: String,
    pub stale_frames: String,
    pub stats_rejections: String,
    pub frame_rejections: String,
    pub stats_evictions: String,
    pub frame_evictions: String,
}

/// Read-only operational evidence, separate from frequently sampled step health.
#[napi(object)]
pub struct RuntimeQueueDiagnostics {
    pub inbound: InboundQueueDiagnostics,
    pub output: OutputQueueDiagnostics,
}

/// Supported x86_64 counters fit exactly in the existing sixteen-digit wire identity.
pub(crate) fn queue_diagnostics(runtime: &EngineRuntime) -> RuntimeQueueDiagnostics {
    let limits = runtime.queue_limits();
    let health = runtime.health();
    let inbound = health.inbound;
    let output = health.output;
    RuntimeQueueDiagnostics {
        inbound: InboundQueueDiagnostics {
            batches: format!("{:016x}", inbound.batches),
            commands: format!("{:016x}", inbound.commands),
            owned_bytes: format!("{:016x}", inbound.owned_bytes),
            high_water_batches: format!("{:016x}", inbound.high_water_batches),
            high_water_commands: format!("{:016x}", inbound.high_water_commands),
            high_water_owned_bytes: format!("{:016x}", inbound.high_water_owned_bytes),
            max_batches: format!("{:016x}", limits.inbound.max_batches),
            max_commands: format!("{:016x}", limits.inbound.max_commands),
            max_owned_bytes: format!("{:016x}", limits.inbound.max_owned_bytes),
            max_batch_commands: format!("{:016x}", limits.inbound.max_batch_commands),
            max_batch_owned_bytes: format!("{:016x}", limits.inbound.max_batch_owned_bytes),
            rejections: format!("{:016x}", inbound.rejections),
            fault_discarded_commands: format!("{:016x}", inbound.fault_discarded_commands),
        },
        output: OutputQueueDiagnostics {
            reliable: format!("{:016x}", output.reliable),
            reliable_owned_bytes: format!("{:016x}", output.reliable_owned_bytes),
            discrete: format!("{:016x}", output.discrete),
            discrete_owned_bytes: format!("{:016x}", output.discrete_owned_bytes),
            frames: format!("{:016x}", output.frames),
            has_stats: output.has_stats,
            owned_bytes: format!("{:016x}", output.total_owned_bytes),
            high_water_count: format!("{:016x}", output.high_water_count),
            high_water_owned_bytes: format!("{:016x}", output.high_water_owned_bytes),
            max_reliable: format!("{:016x}", limits.output.max_reliable),
            max_reliable_owned_bytes: format!("{:016x}", limits.output.max_reliable_owned_bytes),
            max_discrete: format!("{:016x}", limits.output.max_discrete),
            max_discrete_owned_bytes: format!("{:016x}", limits.output.max_discrete_owned_bytes),
            max_frames: format!("{:016x}", limits.output.max_frame_connections),
            max_owned_bytes: format!("{:016x}", limits.output.max_total_owned_bytes),
            max_event_owned_bytes: format!("{:016x}", limits.output.max_event_owned_bytes),
            capacity_waits: format!("{:016x}", output.capacity_waits),
            priority_overflows: format!("{:016x}", output.priority_overflows),
            has_reserved_fault: output.has_reserved_fault,
            stats_replacements: format!("{:016x}", output.stats_replacements),
            frame_replacements: format!("{:016x}", output.frame_replacements),
            stale_stats: format!("{:016x}", output.stale_stats),
            stale_frames: format!("{:016x}", output.stale_frames),
            stats_rejections: format!("{:016x}", output.stats_rejections),
            frame_rejections: format!("{:016x}", output.frame_rejections),
            stats_evictions: format!("{:016x}", output.stats_evictions),
            frame_evictions: format!("{:016x}", output.frame_evictions),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::contract::{
        CommandBatch, EngineCommand, EngineInit, InboundLimits, OutputLimits, SequencedCommand,
        ENGINE_CONTRACT_VERSION,
    };
    use crate::engine::error::{EngineError, EngineErrorCode};
    use crate::engine::queues::WakeSink;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Hold the real coordinator's first notification while its inbound queue fills.
    struct FirstWake {
        first: AtomicBool,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl WakeSink for FirstWake {
        fn notify(&self) -> Result<(), EngineError> {
            if !self.first.swap(false, Ordering::AcqRel) {
                return Ok(());
            }
            self.entered.send(()).expect("test observer remains alive");
            self.release
                .lock()
                .expect("test channel mutex is unpoisoned")
                .recv_timeout(Duration::from_secs(5))
                .expect("test releases the coordinator");
            Ok(())
        }
    }

    fn batch(first_sequence: u64) -> CommandBatch {
        CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: (first_sequence..first_sequence + 2)
                .map(|sequence| SequencedCommand {
                    sequence,
                    command: EngineCommand::Probe {
                        correlation_id: sequence,
                        payload: vec![1; 64],
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn diagnostics_preserve_real_queue_saturation_and_peaks_after_drain(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let runtime = EngineRuntime::new_experimental_probe(
            EngineInit {
                contract_version: ENGINE_CONTRACT_VERSION,
                inbound: InboundLimits {
                    max_batches: 2,
                    max_commands: 4,
                    max_owned_bytes: 4096,
                    max_batch_commands: 2,
                    max_batch_owned_bytes: 2048,
                },
                output: OutputLimits {
                    max_reliable: 8,
                    max_reliable_owned_bytes: 4096,
                    max_discrete: 2,
                    max_discrete_owned_bytes: 4096,
                    max_total_owned_bytes: 8192,
                    max_event_owned_bytes: 1024,
                    max_frame_connections: 2,
                },
            },
            Arc::new(FirstWake {
                first: AtomicBool::new(true),
                entered: entered_tx,
                release: Mutex::new(release_rx),
            }),
        )?;
        runtime.start()?;
        entered_rx.recv_timeout(Duration::from_secs(2))?;
        runtime.try_submit(batch(1))?;
        runtime.try_submit(batch(3))?;
        let rejection = runtime.try_submit(batch(5));
        let full = queue_diagnostics(&runtime);
        release_tx.send(())?;
        assert_eq!(
            rejection
                .expect_err("bounded queue rejects the third batch")
                .code,
            EngineErrorCode::QueueCountLimit
        );
        assert_eq!(full.inbound.batches, "0000000000000002");
        assert_eq!(full.inbound.commands, "0000000000000004");
        assert_eq!(full.inbound.owned_bytes, "0000000000000100");
        assert_eq!(full.inbound.max_commands, "0000000000000004");
        assert_eq!(full.inbound.max_owned_bytes, "0000000000001000");
        assert_eq!(full.inbound.rejections, "0000000000000001");
        let deadline = Instant::now() + Duration::from_secs(2);
        while runtime.health().processed_commands < 4 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(runtime.health().processed_commands, 4);
        runtime.request_stop();
        runtime.join()?;
        let before = queue_diagnostics(&runtime);
        assert!(!runtime.drain_outputs(32, 8192)?.events.is_empty());
        let drained = queue_diagnostics(&runtime);
        assert_eq!(drained.inbound.commands, "0000000000000000");
        assert_eq!(drained.inbound.high_water_commands, full.inbound.commands);
        assert_eq!(
            drained.inbound.high_water_owned_bytes,
            full.inbound.owned_bytes
        );
        assert_eq!(drained.inbound.rejections, full.inbound.rejections);
        assert_eq!(drained.output.reliable, "0000000000000000");
        assert_eq!(drained.output.owned_bytes, "0000000000000000");
        assert_eq!(
            drained.output.high_water_count,
            before.output.high_water_count
        );
        assert_eq!(
            drained.output.high_water_owned_bytes,
            before.output.high_water_owned_bytes
        );
        assert!(u64::from_str_radix(&drained.output.high_water_owned_bytes, 16)? >= 256);
        assert_eq!(drained.output.max_reliable, "0000000000000008");
        assert_eq!(drained.output.priority_overflows, "0000000000000000");
        Ok(())
    }
}
