use super::super::contract::{EngineCommand, SequencedCommand, ENGINE_CONTRACT_VERSION};
use super::*;
use std::sync::Barrier;
use std::thread;

fn inbound_limits() -> InboundLimits {
    InboundLimits {
        max_batches: 2,
        max_commands: 3,
        max_owned_bytes: 8,
        max_batch_commands: 2,
        max_batch_owned_bytes: 6,
    }
}

fn output_limits() -> OutputLimits {
    OutputLimits {
        max_reliable: 3,
        max_reliable_owned_bytes: 12,
        max_discrete: 3,
        max_discrete_owned_bytes: 12,
        max_total_owned_bytes: 20,
        max_event_owned_bytes: 12,
        max_frame_connections: 2,
    }
}

fn probe(sequence: u64, bytes: usize) -> SequencedCommand {
    SequencedCommand {
        sequence,
        command: EngineCommand::Probe {
            correlation_id: sequence,
            payload: vec![sequence as u8; bytes],
        },
    }
}

fn batch(commands: Vec<SequencedCommand>) -> CommandBatch {
    CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: commands.into_boxed_slice(),
    }
}

#[test]
fn deferred_actions_leave_receipt_capacity_and_do_not_wake_a_blocked_step() {
    use super::super::contract::{ControllerActionRequest, RunningAuthorityCommand};
    let queue = InboundQueue::new(InboundLimits {
        max_batches: 3,
        max_commands: 3,
        max_owned_bytes: 16_384,
        max_batch_commands: 1,
        max_batch_owned_bytes: 4096,
    });
    let action = |sequence| {
        batch(vec![SequencedCommand {
            sequence,
            command: EngineCommand::RunningAuthority(
                RunningAuthorityCommand::SubmitControllerAction(Box::new(
                    ControllerActionRequest {
                        client_action_id: None,
                        lease_id: 1,
                        connection_id: 2,
                        turn: 0.5,
                        boost: false,
                        client_tick: 0,
                        received_at: std::time::Instant::now(),
                    },
                )),
            ),
        }])
    };
    queue.try_push(action(1)).unwrap();
    queue.try_push(action(2)).unwrap();
    assert!(queue.try_push(action(3)).is_err());
    let before = queue.metrics();
    assert_eq!(
        queue.wait_until_eligible(Some(Duration::from_millis(1)), false),
        InboundWaitResult::TimedOut
    );
    queue.try_push(batch(vec![probe(3, 0)])).unwrap();
    let mut drained = Vec::new();
    assert!(!queue.drain_eligible_boundary(&mut drained, false));
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].commands[0].sequence, 3);
    assert_eq!(queue.metrics().owned_bytes, before.owned_bytes);
    assert_eq!(queue.metrics().commands, 2);
    assert!(!queue.drain_step_boundary(&mut drained));
    assert_eq!(
        drained
            .iter()
            .map(|item| item.commands[0].sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(queue.metrics().owned_bytes, 0);
}

#[test]
fn reclaim_drain_retains_following_controls_until_its_receipt() {
    use crate::engine::contract::{
        ControllerActionRequest, ControllerReclaimReceipt, ControllerReclaimRequest,
        RunningAuthorityCommand,
    };
    let queue = InboundQueue::new(InboundLimits {
        max_batches: 8,
        max_commands: 8,
        max_owned_bytes: 4096,
        max_batch_commands: 1,
        max_batch_owned_bytes: 512,
    });
    let command = |sequence, command| {
        batch(vec![SequencedCommand {
            sequence,
            command: EngineCommand::RunningAuthority(command),
        }])
    };
    queue
        .try_push(command(
            1,
            RunningAuthorityCommand::ReclaimController(Box::new(ControllerReclaimRequest {
                connection_id: 12,
                kind: crate::engine::state::ControllerKind::Player,
                resume_token: "token".into(),
                identity_key: String::new(),
                received_at: std::time::Instant::now(),
            })),
        ))
        .assert_ok();
    queue
        .try_push(command(
            2,
            RunningAuthorityCommand::SubmitControllerAction(Box::new(ControllerActionRequest {
                client_action_id: None,
                lease_id: 7,
                connection_id: 12,
                turn: 0.5,
                boost: false,
                client_tick: 1,
                received_at: std::time::Instant::now(),
            })),
        ))
        .assert_ok();
    let mut drained = Vec::new();
    queue.drain_step_boundary(&mut drained);
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].commands[0].sequence, 1);
    assert_eq!(queue.metrics().commands, 1);
    queue
        .try_push(command(
            3,
            RunningAuthorityCommand::SubmitControllerReclaimReceipt(ControllerReclaimReceipt {
                request_sequence: 1,
                connection_id: 12,
                lease_id: 7,
                accepted: true,
            }),
        ))
        .assert_ok();
    queue.drain_eligible_boundary(&mut drained, false);
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].commands[0].sequence, 3);
    assert_eq!(queue.metrics().commands, 1);
    queue.drain_step_boundary(&mut drained);
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].commands[0].sequence, 2);
}

#[test]
fn inbound_batch_is_atomic_and_fifo() {
    let queue = InboundQueue::new(inbound_limits());
    queue
        .try_push(batch(vec![probe(1, 2), probe(2, 2)]))
        .assert_ok();
    let before = queue.metrics();
    assert_eq!(
        queue
            .try_push(batch(vec![probe(3, 3), probe(4, 3)]))
            .err()
            .map(|e| e.code),
        Some(EngineErrorCode::QueueCountLimit)
    );
    assert_eq!(queue.metrics().commands, before.commands);
    assert_eq!(
        queue.wait_pop().map(|item| item.commands[0].sequence),
        Some(1)
    );
}

#[test]
fn inbound_enforces_byte_and_sequence_limits() {
    let queue = InboundQueue::new(inbound_limits());
    assert_eq!(
        queue
            .try_push(batch(vec![probe(1, 7)]))
            .err()
            .map(|e| e.code),
        Some(EngineErrorCode::QueueByteLimit)
    );
    queue.try_push(batch(vec![probe(2, 1)])).assert_ok();
    assert_eq!(
        queue
            .try_push(batch(vec![probe(2, 1)]))
            .err()
            .map(|e| e.code),
        Some(EngineErrorCode::SequenceRegression)
    );
}

#[test]
fn full_queue_does_not_block_stop() {
    let queue = Arc::new(InboundQueue::new(InboundLimits {
        max_batches: 1,
        max_commands: 1,
        max_owned_bytes: 1,
        max_batch_commands: 1,
        max_batch_owned_bytes: 1,
    }));
    queue.try_push(batch(vec![probe(1, 1)])).assert_ok();
    queue.request_stop();
    assert!(queue.metrics().stop_requested);
    assert!(queue.wait_pop().is_some());
    assert!(queue.wait_pop().is_none());
}

#[test]
fn timed_wait_and_atomic_step_boundary_drain_preserve_queue_order() {
    let queue = InboundQueue::new(inbound_limits());
    assert_eq!(
        queue.wait_until_ready(Some(Duration::from_millis(1))),
        InboundWaitResult::TimedOut
    );
    queue
        .try_push(batch(vec![probe(1, 1), probe(2, 1)]))
        .assert_ok();
    queue.try_push(batch(vec![probe(3, 1)])).assert_ok();
    assert_eq!(
        queue.wait_until_ready(Some(Duration::from_secs(1))),
        InboundWaitResult::CommandsReady
    );

    let mut drained = Vec::with_capacity(2);
    assert!(!queue.drain_step_boundary(&mut drained));
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].commands[0].sequence, 1);
    assert_eq!(drained[1].commands[0].sequence, 3);
    let metrics = queue.metrics();
    assert_eq!(metrics.batches, 0);
    assert_eq!(metrics.commands, 0);
    assert_eq!(metrics.owned_bytes, 0);

    queue.request_stop();
    assert_eq!(queue.wait_until_ready(None), InboundWaitResult::Stopped);
}

#[test]
fn diagnostic_wake_is_coalesced_and_consumes_no_command_or_reserved_bytes() {
    let queue = InboundQueue::new(inbound_limits());
    queue.request_diagnostic_service();
    queue.request_diagnostic_service();
    assert_eq!(
        queue.wait_until_eligible(Some(Duration::from_millis(1)), false),
        InboundWaitResult::CommandsReady
    );
    let metrics = queue.metrics();
    assert_eq!(metrics.commands, 0);
    assert_eq!(metrics.owned_bytes, 0);
    assert!(metrics.last_accepted_sequence.is_none());
    assert_eq!(
        queue.wait_until_eligible(Some(Duration::from_millis(1)), false),
        InboundWaitResult::TimedOut
    );
}

#[test]
fn fault_stop_discards_accepted_work_and_closes_admission() {
    let queue = InboundQueue::new(inbound_limits());
    queue
        .try_push(batch(vec![probe(1, 2), probe(2, 2)]))
        .assert_ok();
    queue.request_fault_stop(|| {
        assert!(queue.fault_stop_initiated.load(Ordering::Acquire));
        assert!(queue.state.try_lock().is_err());
        true
    });
    assert!(lock_recover(&queue.state).fault_stop_requested);
    let metrics = queue.metrics();
    assert_eq!(metrics.batches, 0);
    assert_eq!(metrics.commands, 0);
    assert_eq!(metrics.owned_bytes, 0);
    assert_eq!(metrics.fault_discarded_batches, 1);
    assert_eq!(metrics.fault_discarded_commands, 2);
    assert_eq!(metrics.fault_discarded_owned_bytes, 4);
    assert!(queue.wait_pop().is_none());
    assert_eq!(
        queue.wait_until_ready(Some(Duration::from_millis(1))),
        InboundWaitResult::Stopped
    );
    let mut raced_drain = Vec::new();
    assert!(queue.drain_step_boundary(&mut raced_drain));
    assert!(raced_drain.is_empty());
    assert!(queue.try_push(batch(vec![probe(4, 1)])).is_err());
}

#[test]
fn failed_fault_claim_restores_orderly_drain_and_allows_one_retry() {
    let queue = InboundQueue::new(inbound_limits());
    queue.try_push(batch(vec![probe(1, 2)])).assert_ok();
    queue.request_fault_stop(|| false);

    assert!(!lock_recover(&queue.state).fault_stop_requested);
    assert!(!queue.fault_stop_initiated.load(Ordering::Acquire));
    assert_eq!(queue.metrics().fault_discarded_batches, 0);
    assert_eq!(
        queue
            .wait_pop()
            .expect("ordinary stop drains accepted work")
            .commands[0]
            .sequence,
        1
    );
    assert!(queue.wait_pop().is_none());

    queue.request_fault_stop(|| true);
    assert!(lock_recover(&queue.state).fault_stop_requested);
    assert!(queue.fault_stop_initiated.load(Ordering::Acquire));
}

#[test]
fn fault_commit_waits_for_preheld_queue_mutex_then_accounts_work() {
    let queue = Arc::new(InboundQueue::new(inbound_limits()));
    queue.try_push(batch(vec![probe(1, 2)])).assert_ok();
    let held_state = lock_recover(&queue.state);
    let fault_thread = {
        let queue = Arc::clone(&queue);
        thread::spawn(move || queue.request_fault_stop(|| true))
    };

    while !queue.fault_stop_initiated.load(Ordering::Acquire) {
        thread::yield_now();
    }
    assert!(!held_state.fault_stop_requested);
    assert_eq!(held_state.queue.len(), 1);
    drop(held_state);
    assert!(fault_thread.join().is_ok());

    let state = lock_recover(&queue.state);
    assert!(state.fault_stop_requested);
    assert!(state.queue.is_empty());
    assert_eq!(state.fault_discarded_batches, 1);
    assert_eq!(state.fault_discarded_commands, 1);
    assert_eq!(state.fault_discarded_owned_bytes, 2);
}

#[test]
fn fault_terminal_gate_precedes_racing_orderly_stop() {
    let inbound = Arc::new(InboundQueue::new(inbound_limits()));
    let output = Arc::new(OutputQueue::new(output_limits(), Arc::new(NoopWakeSink)));
    let fault_entered = Arc::new(Barrier::new(2));
    let release_fault = Arc::new(Barrier::new(2));
    let fault_thread = {
        let inbound = Arc::clone(&inbound);
        let output = Arc::clone(&output);
        let fault_entered = Arc::clone(&fault_entered);
        let release_fault = Arc::clone(&release_fault);
        thread::spawn(move || {
            inbound.request_fault_stop(|| {
                fault_entered.wait();
                release_fault.wait();
                output.retain_reserved_fault(
                    EngineError::new(EngineErrorCode::Faulted, "terminal gate").into(),
                )
            });
        })
    };
    fault_entered.wait();
    let orderly_thread = {
        let inbound = Arc::clone(&inbound);
        let output = Arc::clone(&output);
        thread::spawn(move || inbound.publish_orderly_stopped(&output))
    };
    assert!(!orderly_thread.is_finished());
    release_fault.wait();

    assert!(fault_thread.join().is_ok());
    assert!(matches!(orderly_thread.join(), Ok(Ok(()))));
    let events = output.drain(8, 100).events;
    assert!(events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Fault(_))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
}

#[derive(Default)]
struct CountWake(AtomicU64);
impl WakeSink for CountWake {
    fn notify(&self) -> Result<(), EngineError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct FailingWake;
impl WakeSink for FailingWake {
    fn notify(&self) -> Result<(), EngineError> {
        Err(EngineError::new(
            EngineErrorCode::WakeDelivery,
            "adapter closing",
        ))
    }
}

struct PanickingWake;
impl WakeSink for PanickingWake {
    fn notify(&self) -> Result<(), EngineError> {
        panic!("wake panic must be contained")
    }
}

struct BlockingWake {
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl WakeSink for BlockingWake {
    fn notify(&self) -> Result<(), EngineError> {
        self.entered.wait();
        self.release.wait();
        Ok(())
    }
}

#[test]
fn output_priority_replacement_and_reserved_fault_are_observable() {
    let wake = Arc::new(CountWake::default());
    let queue = OutputQueue::new(output_limits(), wake.clone());
    queue
        .replace_frame(FrameEvent {
            connection_id: 2,
            sequence: 1,
            payload: vec![1],
        })
        .assert_ok();
    assert_eq!(
        queue
            .replace_frame(FrameEvent {
                connection_id: 2,
                sequence: 2,
                payload: vec![2]
            })
            .assert_ok(),
        ReplaceResult::Replaced
    );
    queue
        .replace_stats(StatsEvent {
            sequence: 1,
            payload: vec![3],
        })
        .assert_ok();
    queue
        .replace_stats(StatsEvent {
            sequence: 2,
            payload: vec![4],
        })
        .assert_ok();
    queue
        .push_discrete(DiscreteEvent {
            sequence: 3,
            payload: vec![5],
        })
        .assert_ok();
    queue.push_reliable(ReliableEvent::Started).assert_ok();
    queue.publish_reserved_fault(EngineError::new(EngineErrorCode::Faulted, "fault").into());
    assert_eq!(wake.0.load(Ordering::Relaxed), 1);
    let drained = queue.drain(8, 100);
    assert!(matches!(drained.events[0], CompletedEvent::Fault(_)));
    assert!(matches!(drained.events[1], CompletedEvent::Reliable(_)));
    assert!(matches!(drained.events[2], CompletedEvent::Discrete(_)));
    assert!(matches!(
        drained.events[3],
        CompletedEvent::Stats(StatsEvent { sequence: 2, .. })
    ));
    assert!(matches!(
        drained.events[4],
        CompletedEvent::Frame(FrameEvent { sequence: 2, .. })
    ));
    assert_eq!(queue.metrics().stats_replacements, 1);
    assert_eq!(queue.metrics().frame_replacements, 1);
}

#[test]
fn separate_output_class_peaks_survive_drain_and_different_peak_boundaries() {
    let queue = OutputQueue::new(
        OutputLimits {
            max_reliable: 4,
            max_reliable_owned_bytes: 32,
            max_discrete: 4,
            max_discrete_owned_bytes: 32,
            max_total_owned_bytes: 64,
            max_event_owned_bytes: 32,
            max_frame_connections: 4,
        },
        Arc::new(NoopWakeSink),
    );
    for sequence in 1..=2 {
        queue
            .push_reliable(ReliableEvent::ProbeResult {
                sequence,
                correlation_id: sequence,
                payload: vec![1; 8],
            })
            .assert_ok();
    }
    assert_eq!(queue.drain(8, 64).events.len(), 2);
    for (connection_id, bytes) in [(1, 12), (2, 4)] {
        queue
            .replace_frame(FrameEvent {
                connection_id,
                sequence: 3,
                payload: vec![2; bytes],
            })
            .assert_ok();
    }
    queue
        .replace_stats(StatsEvent {
            sequence: 4,
            payload: vec![3; 8],
        })
        .assert_ok();
    queue
        .push_discrete(DiscreteEvent {
            sequence: 5,
            payload: vec![4; 4],
        })
        .assert_ok();
    queue.push_reliable(ReliableEvent::Started).assert_ok();
    assert_eq!(queue.drain(8, 64).events.len(), 5);
    let metrics = queue.metrics();
    assert_eq!(metrics.total_owned_bytes, 0);
    assert_eq!(metrics.high_water_count, 5);
    assert_eq!(metrics.high_water_owned_bytes, 28);
    assert_eq!(metrics.high_water_reliable, 2);
    assert_eq!(metrics.high_water_reliable_owned_bytes, 16);
    assert_eq!(metrics.high_water_discrete, 1);
    assert_eq!(metrics.high_water_discrete_owned_bytes, 4);
    assert_eq!(metrics.high_water_frames, 2);
    assert_eq!(metrics.high_water_frame_owned_bytes, 16);
    assert_eq!(metrics.high_water_stats, 1);
    assert_eq!(metrics.high_water_stats_owned_bytes, 8);
}

#[test]
fn reliable_overflow_never_uses_reserved_fault_capacity() {
    let queue = OutputQueue::new(
        OutputLimits {
            max_reliable: 1,
            ..output_limits()
        },
        Arc::new(NoopWakeSink),
    );
    queue.push_reliable(ReliableEvent::Started).assert_ok();
    assert!(queue.push_reliable(ReliableEvent::Stopped).is_err());
    queue.publish_reserved_fault(
        EngineError::new(EngineErrorCode::QueueCountLimit, "overflow").into(),
    );
    let metrics = queue.metrics();
    assert_eq!(metrics.reliable, 1);
    assert!(metrics.has_reserved_fault);
    assert_eq!(metrics.priority_overflows, 1);
}

#[test]
fn reliable_preflight_rejects_count_and_bytes_without_enqueuing() {
    let count_queue = OutputQueue::new(
        OutputLimits {
            max_reliable: 1,
            ..output_limits()
        },
        Arc::new(NoopWakeSink),
    );
    count_queue
        .push_reliable(ReliableEvent::Started)
        .assert_ok();
    let before_count = count_queue.metrics();
    assert_eq!(
        count_queue
            .preflight_reliable_reservation(&[0])
            .expect_err("full reliable count must reject the reservation")
            .code,
        EngineErrorCode::QueueCountLimit
    );
    let after_count = count_queue.metrics();
    assert_eq!(after_count.reliable, before_count.reliable);
    assert_eq!(
        after_count.total_owned_bytes,
        before_count.total_owned_bytes
    );

    let byte_queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    byte_queue
        .push_reliable(ReliableEvent::ProbeResult {
            sequence: 1,
            correlation_id: 1,
            payload: vec![0; 8],
        })
        .assert_ok();
    let before_bytes = byte_queue.metrics();
    assert_eq!(
        byte_queue
            .preflight_reliable_reservation(&[5])
            .expect_err("reliable byte total must reject the reservation")
            .code,
        EngineErrorCode::QueueByteLimit
    );
    let after_bytes = byte_queue.metrics();
    assert_eq!(after_bytes.reliable, before_bytes.reliable);
    assert_eq!(
        after_bytes.total_owned_bytes,
        before_bytes.total_owned_bytes
    );
    assert_eq!(
        byte_queue
            .preflight_reliable_reservation(&[])
            .expect_err("empty reservation is not a command response")
            .code,
        EngineErrorCode::InvalidCommand
    );
}

#[test]
fn authority_reply_reservation_orders_completion_before_racing_fault_visibility() {
    let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    let reservation = queue.reserve_authority_reply(8).assert_ok();
    assert!(queue.push_reliable(ReliableEvent::Started).is_err());
    assert!(queue.publish_orderly_stopped().is_err());
    queue.publish_reserved_fault(EngineFault::new(
        EngineErrorCode::WakeDelivery,
        "racing fault",
    ));
    assert!(queue.drain(8, 1024).events.is_empty());
    reservation
        .publish(ReliableEvent::ProbeResult {
            sequence: 1,
            correlation_id: 1,
            payload: vec![0; 8],
        })
        .assert_ok();
    let events = queue.drain(8, 1024).events;
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], CompletedEvent::Fault(_)));
    assert!(matches!(
        &events[1],
        CompletedEvent::Reliable(ReliableEvent::ProbeResult { sequence: 1, .. })
    ));
    assert!(queue.reserve_authority_reply(8).is_err());
    assert_eq!(queue.metrics().total_owned_bytes, 0);
}

#[test]
fn dropping_an_unused_authority_reservation_releases_a_deferred_fault() {
    let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    let reservation = queue.reserve_authority_reply(8).assert_ok();
    queue.publish_reserved_fault(EngineFault::new(
        EngineErrorCode::Faulted,
        "cancelled operation",
    ));
    assert!(queue.drain(8, 1024).events.is_empty());
    drop(reservation);
    let events = queue.drain(8, 1024).events;
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], CompletedEvent::Fault(_)));
    assert!(queue.reserve_authority_reply(8).is_err());
}

#[test]
fn rejected_priority_output_does_not_evict_replaceable_state() {
    let queue = OutputQueue::new(
        OutputLimits {
            max_reliable: 1,
            ..output_limits()
        },
        Arc::new(NoopWakeSink),
    );
    queue
        .replace_stats(StatsEvent {
            sequence: 1,
            payload: vec![1; 4],
        })
        .assert_ok();
    queue
        .replace_frame(FrameEvent {
            connection_id: 1,
            sequence: 2,
            payload: vec![2; 4],
        })
        .assert_ok();
    queue.push_reliable(ReliableEvent::Started).assert_ok();
    let before = queue.metrics();
    assert!(queue.push_reliable(ReliableEvent::Stopped).is_err());
    let after = queue.metrics();
    assert!(after.has_stats);
    assert_eq!(after.frames, 1);
    assert_eq!(after.total_owned_bytes, before.total_owned_bytes);
    assert_eq!(after.stats_evictions, before.stats_evictions);
    assert_eq!(after.frame_evictions, before.frame_evictions);
}

#[test]
fn reliable_batch_publication_is_all_or_nothing() {
    let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    queue.push_reliable(ReliableEvent::Started).assert_ok();
    let events = (1..=3)
        .map(|sequence| ReliableEvent::ProbeResult {
            sequence,
            correlation_id: sequence,
            payload: vec![sequence as u8],
        })
        .collect();
    assert!(queue.push_reliable_batch(events).is_err());
    let drained = queue.drain(8, 100);
    assert_eq!(drained.events.len(), 1);
    assert!(matches!(
        drained.events[0],
        CompletedEvent::Reliable(ReliableEvent::Started)
    ));
}

#[test]
fn stale_replaceable_publications_cannot_regress_retained_state() {
    let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    queue
        .replace_stats(StatsEvent {
            sequence: 10,
            payload: vec![10],
        })
        .assert_ok();
    assert_eq!(
        queue
            .replace_stats(StatsEvent {
                sequence: 9,
                payload: vec![9],
            })
            .assert_ok(),
        ReplaceResult::Stale
    );
    queue
        .replace_frame(FrameEvent {
            connection_id: 1,
            sequence: 10,
            payload: vec![10],
        })
        .assert_ok();
    assert_eq!(
        queue
            .replace_frame(FrameEvent {
                connection_id: 1,
                sequence: 10,
                payload: vec![9],
            })
            .assert_ok(),
        ReplaceResult::Stale
    );
    let drained = queue.drain(8, 100);
    assert!(drained.events.iter().any(|event| matches!(
        event,
        CompletedEvent::Stats(StatsEvent { sequence: 10, .. })
    )));
    assert!(drained.events.iter().any(|event| matches!(
        event,
        CompletedEvent::Frame(FrameEvent { sequence: 10, .. })
    )));
    assert_eq!(queue.metrics().stale_stats, 1);
    assert_eq!(queue.metrics().stale_frames, 1);
}

#[test]
fn oldest_frame_publication_prevents_low_id_starvation() {
    let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    for connection_id in [1, 2] {
        queue
            .replace_frame(FrameEvent {
                connection_id,
                sequence: 1,
                payload: vec![connection_id as u8],
            })
            .assert_ok();
    }
    let first = queue.drain(1, 100);
    assert!(matches!(
        first.events[0],
        CompletedEvent::Frame(FrameEvent {
            connection_id: 1,
            ..
        })
    ));
    queue
        .replace_frame(FrameEvent {
            connection_id: 1,
            sequence: 2,
            payload: vec![1],
        })
        .assert_ok();
    let second = queue.drain(1, 100);
    assert!(matches!(
        second.events[0],
        CompletedEvent::Frame(FrameEvent {
            connection_id: 2,
            ..
        })
    ));
}

#[test]
fn equal_sequence_frame_refreshes_rotate_across_connections() {
    let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    let mut drained_connections = Vec::new();

    for sequence in 1..=4 {
        for connection_id in [1, 2] {
            queue
                .replace_frame(FrameEvent {
                    connection_id,
                    sequence,
                    payload: vec![connection_id as u8],
                })
                .assert_ok();
        }
        let drained = queue.drain(1, 100);
        let CompletedEvent::Frame(frame) = &drained.events[0] else {
            panic!("expected one frame");
        };
        drained_connections.push(frame.connection_id);
    }

    assert_eq!(drained_connections, vec![1, 2, 1, 2]);
}

#[test]
fn failed_or_panicking_wake_is_retryable_and_never_unwinds() {
    for sink in [
        Arc::new(FailingWake) as Arc<dyn WakeSink>,
        Arc::new(PanickingWake) as Arc<dyn WakeSink>,
    ] {
        let queue = OutputQueue::new(output_limits(), sink);
        let error = queue
            .push_reliable(ReliableEvent::Started)
            .expect_err("wake failure must be reported");
        assert_eq!(error.code, EngineErrorCode::WakeDelivery);
        let wake = queue.wake_metrics();
        assert_eq!(wake.notification_attempts, 1);
        assert_eq!(wake.notifications, 0);
        assert_eq!(wake.notification_failures, 1);
        assert!(!wake.notified);
        let drained = queue.drain(1, 100);
        assert_eq!(drained.events.len(), 1);
        assert!(!drained.more_work);
    }
}

#[test]
fn orderly_stop_wake_failure_rolls_back_before_reserved_fault() {
    let queue = OutputQueue::new(output_limits(), Arc::new(FailingWake));
    let error = queue
        .publish_orderly_stopped()
        .expect_err("an undelivered stop must roll back");
    assert_eq!(error.code, EngineErrorCode::WakeDelivery);
    assert!(!queue
        .drain(8, 100)
        .events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));

    queue.publish_reserved_fault(EngineError::new(EngineErrorCode::Faulted, "fault").into());
    let events = queue.drain(8, 100).events;
    assert!(events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Fault(_))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
}

#[test]
fn output_mutex_linearizes_orderly_stop_against_reserved_fault() {
    let fault_first = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
    fault_first.publish_reserved_fault(EngineError::new(EngineErrorCode::Faulted, "first").into());
    assert!(!fault_first
        .publish_orderly_stopped()
        .expect("fault must suppress, not fail, a later stop"));
    let fault_events = fault_first.drain(8, 100).events;
    assert!(fault_events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Fault(_))));
    assert!(!fault_events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));

    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let stop_first = Arc::new(OutputQueue::new(
        output_limits(),
        Arc::new(BlockingWake {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        }),
    ));
    let producer = {
        let queue = Arc::clone(&stop_first);
        thread::spawn(move || queue.publish_orderly_stopped())
    };
    entered.wait();
    stop_first.publish_reserved_fault(EngineError::new(EngineErrorCode::Faulted, "late").into());
    release.wait();
    assert!(matches!(producer.join(), Ok(Ok(true))));
    let stop_events = stop_first.drain(8, 100).events;
    assert!(stop_events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
    assert!(!stop_events
        .iter()
        .any(|event| matches!(event, CompletedEvent::Fault(_))));
}

#[test]
fn terminal_outcome_rejects_every_later_output_publication() {
    for queue in [
        {
            let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
            queue
                .push_reliable(ReliableEvent::Stopped)
                .expect("one orderly stop must publish");
            queue
        },
        {
            let queue = OutputQueue::new(output_limits(), Arc::new(NoopWakeSink));
            queue.publish_reserved_fault(
                EngineError::new(EngineErrorCode::Faulted, "terminal fault").into(),
            );
            queue
        },
    ] {
        assert!(queue.push_reliable(ReliableEvent::Started).is_err());
        assert!(queue
            .push_discrete(DiscreteEvent {
                sequence: 1,
                payload: vec![1],
            })
            .is_err());
        assert!(queue
            .replace_stats(StatsEvent {
                sequence: 1,
                payload: vec![1],
            })
            .is_err());
        assert!(queue
            .replace_frame(FrameEvent {
                connection_id: 1,
                sequence: 1,
                payload: vec![1],
            })
            .is_err());
    }
}

#[test]
fn wake_is_coalesced_and_rearmed_after_empty_drain() {
    let wake = Arc::new(CountWake::default());
    let queue = OutputQueue::new(output_limits(), wake.clone());
    queue.push_reliable(ReliableEvent::Started).assert_ok();
    queue
        .push_reliable(ReliableEvent::ProbeResult {
            sequence: 1,
            correlation_id: 1,
            payload: vec![1],
        })
        .assert_ok();
    assert_eq!(wake.0.load(Ordering::Relaxed), 1);
    assert!(!queue.drain(8, 100).more_work);
    queue
        .push_reliable(ReliableEvent::ProbeResult {
            sequence: 2,
            correlation_id: 2,
            payload: vec![2],
        })
        .assert_ok();
    assert_eq!(wake.0.load(Ordering::Relaxed), 2);
}

#[test]
fn producer_racing_rearm_cannot_strand_output() {
    for sequence in 1..=200u64 {
        let wake = Arc::new(CountWake::default());
        let queue = Arc::new(OutputQueue::new(output_limits(), wake));
        queue.push_reliable(ReliableEvent::Started).assert_ok();
        let barrier = Arc::new(Barrier::new(2));
        let producer_queue = queue.clone();
        let producer_barrier = barrier.clone();
        let producer = thread::spawn(move || {
            producer_barrier.wait();
            producer_queue.push_reliable(ReliableEvent::ProbeResult {
                sequence,
                correlation_id: sequence,
                payload: vec![1],
            })
        });
        barrier.wait();
        let first = queue.drain(1, 100);
        let produced = producer.join();
        assert!(matches!(produced, Ok(Ok(()))));
        let mut saw_probe = first.events.iter().any(|event| {
            matches!(
                event,
                CompletedEvent::Reliable(ReliableEvent::ProbeResult { .. })
            )
        });
        if !saw_probe {
            let second = queue.drain(4, 100);
            saw_probe = second.events.iter().any(|event| {
                matches!(
                    event,
                    CompletedEvent::Reliable(ReliableEvent::ProbeResult { .. })
                )
            });
        }
        assert!(saw_probe, "probe stranded on iteration {sequence}");
    }
}

trait AssertOk<T> {
    fn assert_ok(self) -> T;
}
impl<T, E: std::fmt::Debug> AssertOk<T> for Result<T, E> {
    fn assert_ok(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("unexpected error: {error:?}"),
        }
    }
}
