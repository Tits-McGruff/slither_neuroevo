//! Bounded inbound and priority-aware output queues for the engine spine.

use std::collections::{BTreeMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::contract::{
    CommandBatch, CompletedEvent, DiscreteEvent, EngineFault, FrameEvent, InboundLimits,
    OutputLimits, ReliableEvent, StatsEvent,
};
use super::display::RunningDisplayStatus;
use super::error::{EngineError, EngineErrorCode};

/// Probe payloads and real authority metadata share one replaceable status slot.
#[derive(Debug)]
enum ReplaceableStats {
    Probe(StatsEvent),
    Running(RunningDisplayStatus),
}

impl ReplaceableStats {
    fn sequence(&self) -> u64 {
        match self {
            Self::Probe(event) => event.sequence,
            Self::Running(event) => event.sequence,
        }
    }

    fn owned_bytes(&self) -> usize {
        match self {
            Self::Probe(event) => event.payload.capacity(),
            Self::Running(_) => std::mem::size_of::<RunningDisplayStatus>(),
        }
    }
}

/// Sink for one coalesced, payload-free notification to the future Node adapter.
pub trait WakeSink: Send + Sync + 'static {
    /// Notify the consumer that output may be ready without blocking.
    fn notify(&self) -> Result<(), EngineError>;
}

/// No-op sink useful before the N-API thread-safe function adapter exists.
#[derive(Debug, Default)]
pub struct NoopWakeSink;

impl WakeSink for NoopWakeSink {
    fn notify(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

/// Observable inbound queue counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InboundMetrics {
    /// Queued batches.
    pub batches: usize,
    /// Queued commands.
    pub commands: usize,
    /// Queued owned bytes.
    pub owned_bytes: usize,
    /// Highest queued batch count.
    pub high_water_batches: usize,
    /// Highest queued command count.
    pub high_water_commands: usize,
    /// Highest queued owned bytes.
    pub high_water_owned_bytes: usize,
    /// Rejected submissions.
    pub rejections: u64,
    /// Accepted batches discarded when the engine faulted.
    pub fault_discarded_batches: u64,
    /// Accepted commands discarded when the engine faulted.
    pub fault_discarded_commands: u64,
    /// Accepted payload bytes discarded when the engine faulted.
    pub fault_discarded_owned_bytes: u64,
    /// Last accepted sequence, if any.
    pub last_accepted_sequence: Option<u64>,
    /// Whether the out-of-band stop flag is set.
    pub stop_requested: bool,
}

#[derive(Debug)]
struct InboundState {
    queue: VecDeque<QueuedBatch>,
    commands: usize,
    owned_bytes: usize,
    high_water_batches: usize,
    high_water_commands: usize,
    high_water_owned_bytes: usize,
    rejections: u64,
    fault_discarded_batches: u64,
    fault_discarded_commands: u64,
    fault_discarded_owned_bytes: u64,
    last_accepted_sequence: Option<u64>,
    fault_stop_requested: bool,
    diagnostic_ready: bool,
}

#[derive(Debug)]
struct QueuedBatch {
    batch: CommandBatch,
    command_count: usize,
    owned_bytes: usize,
}

/// One bounded inbound queue whose stop signal consumes no queue capacity.
#[derive(Debug)]
pub struct InboundQueue {
    limits: InboundLimits,
    state: Mutex<InboundState>,
    ready: Condvar,
    fault_stop_initiated: AtomicBool,
    stop_requested: AtomicBool,
}

/// Reason the background coordinator's condition-variable wait completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InboundWaitResult {
    /// One or more command batches are ready for the next atomic drain.
    CommandsReady,
    /// The Rust-owned scheduler timeout elapsed without an inbound command.
    TimedOut,
    /// Stop was requested after ordinary draining or fault-mode discard gating.
    Stopped,
}

impl InboundQueue {
    /// Create an empty inbound queue from already validated limits.
    pub fn new(limits: InboundLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(InboundState {
                queue: VecDeque::new(),
                commands: 0,
                owned_bytes: 0,
                high_water_batches: 0,
                high_water_commands: 0,
                high_water_owned_bytes: 0,
                rejections: 0,
                fault_discarded_batches: 0,
                fault_discarded_commands: 0,
                fault_discarded_owned_bytes: 0,
                last_accepted_sequence: None,
                fault_stop_requested: false,
                diagnostic_ready: false,
            }),
            ready: Condvar::new(),
            fault_stop_initiated: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
        }
    }

    /// Validate and enqueue a whole batch atomically without waiting for capacity.
    pub fn try_push(&self, batch: CommandBatch) -> Result<(), EngineError> {
        let shape = match batch.validate() {
            Ok(shape) => shape,
            Err(error) => return self.reject(error),
        };
        if shape.command_count > self.limits.max_batch_commands {
            return self.reject(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "command batch exceeds the configured per-batch count limit",
            ));
        }
        if shape.owned_bytes > self.limits.max_batch_owned_bytes {
            return self.reject(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "command batch exceeds the configured per-batch byte limit",
            ));
        }

        let mut state = lock_recover(&self.state);
        if self.stop_requested.load(Ordering::Acquire) {
            state.rejections = state.rejections.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::InvalidLifecycle,
                "engine stop has already been requested",
            ));
        }
        if let Some(last) = state.last_accepted_sequence {
            if shape.first_sequence <= last {
                state.rejections = state.rejections.saturating_add(1);
                return Err(EngineError::new(
                    EngineErrorCode::SequenceRegression,
                    format!(
                        "first command sequence {} does not advance beyond {}",
                        shape.first_sequence, last
                    ),
                ));
            }
        }
        let Some(next_commands) = state.commands.checked_add(shape.command_count) else {
            state.rejections = state.rejections.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "inbound command count accounting overflowed",
            ));
        };
        let Some(next_bytes) = state.owned_bytes.checked_add(shape.owned_bytes) else {
            state.rejections = state.rejections.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "inbound byte accounting overflowed",
            ));
        };
        if state.queue.len() >= self.limits.max_batches || next_commands > self.limits.max_commands
        {
            state.rejections = state.rejections.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "inbound queue count limit reached",
            ));
        }
        if next_bytes > self.limits.max_owned_bytes {
            state.rejections = state.rejections.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "inbound queue byte limit reached",
            ));
        }

        // Deferred actions must leave one complete control batch available to
        // resolve the delivery barrier that prevents those actions executing.
        if batch
            .commands
            .iter()
            .any(|item| item.command.requires_ready_boundary())
            && (state.queue.len() >= self.limits.max_batches.saturating_sub(1)
                || next_commands
                    > self
                        .limits
                        .max_commands
                        .saturating_sub(self.limits.max_batch_commands)
                || next_bytes
                    > self
                        .limits
                        .max_owned_bytes
                        .saturating_sub(self.limits.max_batch_owned_bytes))
        {
            state.rejections = state.rejections.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "deferred actions must leave capacity for a delivery completion",
            ));
        }

        state.commands = next_commands;
        state.owned_bytes = next_bytes;
        state.last_accepted_sequence = Some(shape.last_sequence);
        state.queue.push_back(QueuedBatch {
            batch,
            command_count: shape.command_count,
            owned_bytes: shape.owned_bytes,
        });
        state.high_water_batches = state.high_water_batches.max(state.queue.len());
        state.high_water_commands = state.high_water_commands.max(state.commands);
        state.high_water_owned_bytes = state.high_water_owned_bytes.max(state.owned_bytes);
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    fn reject<T>(&self, error: EngineError) -> Result<T, EngineError> {
        let mut state = lock_recover(&self.state);
        state.rejections = state.rejections.saturating_add(1);
        Err(error)
    }

    /// Block until one batch is available or the out-of-band stop flag is set.
    pub fn wait_pop(&self) -> Option<CommandBatch> {
        let mut state = lock_recover(&self.state);
        loop {
            if state.fault_stop_requested {
                return None;
            }
            if let Some(queued) = state.queue.pop_front() {
                state.commands = state.commands.saturating_sub(queued.command_count);
                state.owned_bytes = state.owned_bytes.saturating_sub(queued.owned_bytes);
                return Some(queued.batch);
            }
            if self.stop_requested.load(Ordering::Acquire) {
                return None;
            }
            state = wait_recover(&self.ready, state);
        }
    }

    /// Wait without busy polling for commands, stop, or an optional Rust timer.
    ///
    /// This operation never removes a batch. The running coordinator follows a
    /// `CommandsReady` or `TimedOut` result with [`Self::drain_step_boundary`],
    /// whose mutex release linearizes the command/action cutoff for one step.
    pub(crate) fn wait_until_ready(&self, timeout: Option<Duration>) -> InboundWaitResult {
        self.wait_until_eligible(timeout, true)
    }

    /// A retained step ignores deferred actions without spinning on their queue entries.
    pub(crate) fn wait_until_eligible(
        &self,
        timeout: Option<Duration>,
        allow_actions: bool,
    ) -> InboundWaitResult {
        let eligible = |current: &InboundState| {
            current.diagnostic_ready
                || current.queue.iter().any(|queued| {
                    allow_actions
                        || !queued
                            .batch
                            .commands
                            .iter()
                            .any(|item| item.command.requires_ready_boundary())
                })
        };
        let state = lock_recover(&self.state);
        let mut state = match timeout {
            Some(timeout) => {
                let (state, _) =
                    wait_timeout_while_recover(&self.ready, state, timeout, |current| {
                        !eligible(current) && !self.stop_requested.load(Ordering::Acquire)
                    });
                state
            }
            None => {
                let mut state = state;
                while !eligible(&state) && !self.stop_requested.load(Ordering::Acquire) {
                    state = wait_recover(&self.ready, state);
                }
                state
            }
        };

        if state.fault_stop_requested {
            InboundWaitResult::Stopped
        } else if eligible(&state) {
            state.diagnostic_ready = false;
            InboundWaitResult::CommandsReady
        } else if self.stop_requested.load(Ordering::Acquire) {
            InboundWaitResult::Stopped
        } else {
            InboundWaitResult::TimedOut
        }
    }

    /// Wake diagnostic handoff service even while a gameplay transaction is retained.
    /// The handshake consumes no command sequence, payload or reliable reservation.
    pub(crate) fn request_diagnostic_service(&self) {
        let mut state = lock_recover(&self.state);
        state.diagnostic_ready = true;
        self.ready.notify_one();
    }

    /// Drain every command accepted before one fixed-step boundary.
    ///
    /// The caller owns and reuses `output`. Commands accepted after this
    /// method releases the queue mutex belong to the following boundary.
    pub(crate) fn drain_step_boundary(&self, output: &mut Vec<CommandBatch>) -> bool {
        self.drain_eligible_boundary(output, true)
    }

    /// Retain actions, including their original receive times and byte charges,
    /// while allowing exact delivery/persistence receipts to unblock authority.
    pub(crate) fn drain_eligible_boundary(
        &self,
        output: &mut Vec<CommandBatch>,
        allow_actions: bool,
    ) -> bool {
        output.clear();
        let mut state = lock_recover(&self.state);
        if state.fault_stop_requested {
            return true;
        }
        output.reserve(state.queue.len());
        let mut index = 0;
        while index < state.queue.len() {
            if !allow_actions
                && state.queue[index]
                    .batch
                    .commands
                    .iter()
                    .any(|item| item.command.requires_ready_boundary())
            {
                index += 1;
                continue;
            }
            let queued = state.queue.remove(index).expect("checked queue index");
            // A reconnect can establish a new delivery barrier at this boundary.
            // Later controls must remain charged and queued until it resolves.
            let starts_barrier = queued.batch.commands.iter().any(|item| {
                matches!(
                    item.command,
                    super::contract::EngineCommand::RunningAuthority(
                        super::contract::RunningAuthorityCommand::ReclaimController(_)
                            | super::contract::RunningAuthorityCommand::JoinController(_)
                    )
                )
            });
            state.commands = state.commands.saturating_sub(queued.command_count);
            state.owned_bytes = state.owned_bytes.saturating_sub(queued.owned_bytes);
            output.push(queued.batch);
            if starts_barrier {
                break;
            }
        }
        self.stop_requested.load(Ordering::Acquire)
    }

    /// Set the out-of-band stop flag and wake the coordinator.
    pub fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        // Briefly synchronize with the mutex paired to the condition variable.
        // The flag remains out of queue capacity, while this handshake prevents
        // a notification from racing between the waiter's check and its wait.
        drop(lock_recover(&self.state));
        self.ready.notify_all();
    }

    /// Close faulted admission, publish terminal fault state, discard every
    /// still-queued batch, and only then wake the coordinator.
    ///
    /// The actual fault-stop marker and terminal callback are committed while
    /// owning the same mutex used by every dequeue and orderly-stop decision.
    /// The separate initiation bit makes recursive wake-fault reports inert
    /// instead of trying to re-enter that mutex from the callback.
    pub(crate) fn request_fault_stop(&self, retain_terminal_fault: impl FnOnce() -> bool) {
        if self.fault_stop_initiated.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut state = lock_recover(&self.state);
        state.fault_stop_requested = true;
        self.stop_requested.store(true, Ordering::Release);
        if !retain_terminal_fault() {
            state.fault_stop_requested = false;
            self.fault_stop_initiated.store(false, Ordering::Release);
            drop(state);
            self.ready.notify_all();
            return;
        }
        state.fault_discarded_batches = state
            .fault_discarded_batches
            .saturating_add(state.queue.len() as u64);
        state.fault_discarded_commands = state
            .fault_discarded_commands
            .saturating_add(u64::try_from(state.commands).unwrap_or(u64::MAX));
        state.fault_discarded_owned_bytes = state
            .fault_discarded_owned_bytes
            .saturating_add(u64::try_from(state.owned_bytes).unwrap_or(u64::MAX));
        state.queue.clear();
        state.commands = 0;
        state.owned_bytes = 0;
        drop(state);
        self.ready.notify_all();
    }

    /// Publish one orderly stop only if no fault owns the same terminal gate.
    /// Retaining the output terminal while holding the inbound mutex makes the
    /// queue decision and output decision one indivisible ordering point. The
    /// external wake happens after releasing the inbound mutex so a synchronous
    /// wake failure can safely report a fault.
    pub(crate) fn publish_orderly_stopped(&self, output: &OutputQueue) -> Result<(), EngineError> {
        let retained = {
            let state = lock_recover(&self.state);
            if state.fault_stop_requested {
                false
            } else {
                output.retain_orderly_stopped()?
            }
        };
        if retained {
            output.signal_orderly_stopped().map(|_| ())
        } else {
            Ok(())
        }
    }

    /// Return a consistent queue snapshot.
    pub fn metrics(&self) -> InboundMetrics {
        let state = lock_recover(&self.state);
        InboundMetrics {
            batches: state.queue.len(),
            commands: state.commands,
            owned_bytes: state.owned_bytes,
            high_water_batches: state.high_water_batches,
            high_water_commands: state.high_water_commands,
            high_water_owned_bytes: state.high_water_owned_bytes,
            rejections: state.rejections,
            fault_discarded_batches: state.fault_discarded_batches,
            fault_discarded_commands: state.fault_discarded_commands,
            fault_discarded_owned_bytes: state.fault_discarded_owned_bytes,
            last_accepted_sequence: state.last_accepted_sequence,
            stop_requested: self.stop_requested.load(Ordering::Acquire),
        }
    }
}

/// Result of inserting replaceable output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaceResult {
    /// A previously empty slot accepted the item.
    Inserted,
    /// An older item was replaced by this item.
    Replaced,
    /// The item could not fit the configured output limits.
    Rejected,
    /// The retained item has the same or a newer sequence.
    Stale,
}

/// Observable output queue counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutputMetrics {
    /// Authority waits for a complete reliable reply to fit after a consumer drain.
    pub capacity_waits: u64,
    /// Normally queued reliable events.
    pub reliable: usize,
    /// Reliable owned bytes.
    pub reliable_owned_bytes: usize,
    /// Normally queued discrete events.
    pub discrete: usize,
    /// Discrete owned bytes.
    pub discrete_owned_bytes: usize,
    /// Whether a replaceable stats item exists.
    pub has_stats: bool,
    /// Connections retaining replaceable frames.
    pub frames: usize,
    /// Total normal owned bytes.
    pub total_owned_bytes: usize,
    /// Highest normal event count.
    pub high_water_count: usize,
    /// Highest normal owned bytes.
    pub high_water_owned_bytes: usize,
    /// Highest reliable event count, retained after drain.
    pub high_water_reliable: usize,
    /// Highest reliable owned bytes, retained after drain.
    pub high_water_reliable_owned_bytes: usize,
    /// Highest discrete event count, retained after drain.
    pub high_water_discrete: usize,
    /// Highest discrete owned bytes, retained after drain.
    pub high_water_discrete_owned_bytes: usize,
    /// Highest replaceable frame connection count.
    pub high_water_frames: usize,
    /// Highest replaceable frame owned bytes.
    pub high_water_frame_owned_bytes: usize,
    /// Highest replaceable stats slot occupancy, zero or one.
    pub high_water_stats: usize,
    /// Highest replaceable stats owned bytes.
    pub high_water_stats_owned_bytes: usize,
    /// Reliable/discrete overflow attempts.
    pub priority_overflows: u64,
    /// Stats replacements.
    pub stats_replacements: u64,
    /// Frame replacements.
    pub frame_replacements: u64,
    /// Stale stats publications ignored.
    pub stale_stats: u64,
    /// Stale frame publications ignored.
    pub stale_frames: u64,
    /// Stats rejected without replacing the retained value.
    pub stats_rejections: u64,
    /// Frames rejected without replacing the retained value.
    pub frame_rejections: u64,
    /// Stats evicted for higher-priority capacity.
    pub stats_evictions: u64,
    /// Frames evicted for higher-priority capacity.
    pub frame_evictions: u64,
    /// Whether the reserved fault slot is occupied.
    pub has_reserved_fault: bool,
}

#[derive(Debug)]
struct OutputState {
    authority_reply_reserved: bool,
    deferred_fault: Option<EngineFault>,
    capacity_wait_cancelled: bool,
    capacity_waits: u64,
    reliable: VecDeque<ReliableEvent>,
    reliable_owned_bytes: usize,
    discrete: VecDeque<DiscreteEvent>,
    discrete_owned_bytes: usize,
    stats: Option<ReplaceableStats>,
    frames: BTreeMap<u64, FrameEvent>,
    last_drained_frame_connection: Option<u64>,
    total_owned_bytes: usize,
    high_water_count: usize,
    high_water_owned_bytes: usize,
    high_water_reliable: usize,
    high_water_reliable_owned_bytes: usize,
    high_water_discrete: usize,
    high_water_discrete_owned_bytes: usize,
    high_water_frames: usize,
    high_water_frame_owned_bytes: usize,
    high_water_stats: usize,
    high_water_stats_owned_bytes: usize,
    priority_overflows: u64,
    stats_replacements: u64,
    frame_replacements: u64,
    stale_stats: u64,
    stale_frames: u64,
    stats_rejections: u64,
    frame_rejections: u64,
    stats_evictions: u64,
    frame_evictions: u64,
    reserved_fault: Option<EngineFault>,
    terminal: OutputTerminalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputTerminalState {
    Open,
    Stopped,
    Faulted,
}

/// Priority-aware bounded output with coalesced wake notifications.
pub struct OutputQueue {
    limits: OutputLimits,
    state: Mutex<OutputState>,
    capacity_ready: Condvar,
    sink: Arc<dyn WakeSink>,
    generation: AtomicU64,
    notified: AtomicBool,
    notifications: AtomicU64,
    notification_attempts: AtomicU64,
    notification_failures: AtomicU64,
    rearm_races: AtomicU64,
}

impl std::fmt::Debug for OutputQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OutputQueue")
            .field("limits", &self.limits)
            .field("metrics", &self.metrics())
            .finish_non_exhaustive()
    }
}

/// One reply admitted before an authoritative operation starts. A racing fault
/// closes future admission, but cannot revoke this reply's count/byte capacity.
pub(crate) struct ReliableReplyReservation<'queue> {
    queue: &'queue OutputQueue,
    bytes: usize,
    completed: bool,
}

impl ReliableReplyReservation<'_> {
    /// Fill the admitted slot, then expose any terminal fault ordered after it.
    pub(crate) fn publish(mut self, event: ReliableEvent) -> Result<(), EngineError> {
        let bytes = event.owned_bytes();
        if bytes > self.bytes || matches!(event, ReliableEvent::Stopped) {
            return Err(EngineError::new(
                EngineErrorCode::Faulted,
                "authoritative reply exceeded its admitted reservation",
            ));
        }
        let mut state = lock_recover(&self.queue.state);
        debug_assert!(state.authority_reply_reserved);
        evict_replaceable_for(&mut state, bytes, self.queue.limits.max_total_owned_bytes);
        state.reliable_owned_bytes += bytes;
        state.total_owned_bytes += bytes;
        state.reliable.push_back(event);
        state.authority_reply_reserved = false;
        if let Some(fault) = state.deferred_fault.take() {
            state.reserved_fault = Some(fault);
        }
        update_output_high_water(&mut state);
        self.completed = true;
        drop(state);
        self.queue.signal_change()
    }
}

impl Drop for ReliableReplyReservation<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut state = lock_recover(&self.queue.state);
        state.authority_reply_reserved = false;
        let deferred = state.deferred_fault.take();
        let signal_fault = deferred.is_some();
        if let Some(fault) = deferred {
            state.reserved_fault = Some(fault);
        }
        drop(state);
        if signal_fault {
            self.queue.signal_retained_fault();
        }
    }
}

impl OutputQueue {
    /// Create an empty output queue from already validated limits.
    pub fn new(limits: OutputLimits, sink: Arc<dyn WakeSink>) -> Self {
        Self {
            limits,
            state: Mutex::new(OutputState {
                authority_reply_reserved: false,
                deferred_fault: None,
                capacity_wait_cancelled: false,
                capacity_waits: 0,
                reliable: VecDeque::new(),
                reliable_owned_bytes: 0,
                discrete: VecDeque::new(),
                discrete_owned_bytes: 0,
                stats: None,
                frames: BTreeMap::new(),
                last_drained_frame_connection: None,
                total_owned_bytes: 0,
                high_water_count: 0,
                high_water_owned_bytes: 0,
                high_water_reliable: 0,
                high_water_reliable_owned_bytes: 0,
                high_water_discrete: 0,
                high_water_discrete_owned_bytes: 0,
                high_water_frames: 0,
                high_water_frame_owned_bytes: 0,
                high_water_stats: 0,
                high_water_stats_owned_bytes: 0,
                priority_overflows: 0,
                stats_replacements: 0,
                frame_replacements: 0,
                stale_stats: 0,
                stale_frames: 0,
                stats_rejections: 0,
                frame_rejections: 0,
                stats_evictions: 0,
                frame_evictions: 0,
                reserved_fault: None,
                terminal: OutputTerminalState::Open,
            }),
            sink,
            capacity_ready: Condvar::new(),
            generation: AtomicU64::new(0),
            notified: AtomicBool::new(false),
            notifications: AtomicU64::new(0),
            notification_attempts: AtomicU64::new(0),
            notification_failures: AtomicU64::new(0),
            rearm_races: AtomicU64::new(0),
        }
    }

    /// Configured per-event byte reservation used before authority mutations.
    #[must_use]
    pub(crate) const fn max_event_owned_bytes(&self) -> usize {
        self.limits.max_event_owned_bytes
    }

    /// Enqueue one reliable event or return an observable overflow error.
    pub fn push_reliable(&self, event: ReliableEvent) -> Result<(), EngineError> {
        if matches!(event, ReliableEvent::Stopped) {
            return self.publish_orderly_stopped().map(|_| ());
        }
        self.push_reliable_batch(vec![event])
    }

    /// Publish one orderly terminal event with wake-failure rollback.
    ///
    /// The output mutex linearizes orderly stop against the reserved fault.
    /// If the wake fails before any consumer drains this event, it is removed
    /// so the coordinator can publish a fault without a contradictory stop.
    /// If a polling consumer already drained it, orderly stop is complete and
    /// the wake failure cannot retroactively replace that delivered outcome.
    pub(crate) fn publish_orderly_stopped(&self) -> Result<bool, EngineError> {
        if !self.retain_orderly_stopped()? {
            return Ok(false);
        }
        self.signal_orderly_stopped()
    }

    /// Retain an orderly terminal event without invoking the external wake.
    fn retain_orderly_stopped(&self) -> Result<bool, EngineError> {
        let mut state = lock_recover(&self.state);
        match state.terminal {
            OutputTerminalState::Faulted => return Ok(false),
            OutputTerminalState::Stopped => {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidLifecycle,
                    "orderly stop was already published",
                ))
            }
            OutputTerminalState::Open => {}
        }
        ensure_output_open(&state)?;
        if state.reliable.len() >= self.limits.max_reliable {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "reliable output queue limit reached before orderly stop",
            ));
        }
        if state
            .reliable
            .iter()
            .any(|event| matches!(event, ReliableEvent::Stopped))
        {
            return Err(EngineError::new(
                EngineErrorCode::InvalidLifecycle,
                "untracked orderly stop already exists in reliable output",
            ));
        }
        state.reliable.push_back(ReliableEvent::Stopped);
        state.terminal = OutputTerminalState::Stopped;
        update_output_high_water(&mut state);
        Ok(true)
    }

    /// Signal a retained orderly stop and roll it back if no consumer could
    /// have received it before the wake failed.
    fn signal_orderly_stopped(&self) -> Result<bool, EngineError> {
        match self.signal_change() {
            Ok(()) => Ok(true),
            Err(error) => {
                let mut state = lock_recover(&self.state);
                let retained = state
                    .reliable
                    .iter()
                    .position(|event| matches!(event, ReliableEvent::Stopped));
                if let Some(position) = retained {
                    let removed = state.reliable.remove(position);
                    debug_assert!(matches!(removed, Some(ReliableEvent::Stopped)));
                    state.terminal = OutputTerminalState::Open;
                    Err(error)
                } else {
                    Ok(true)
                }
            }
        }
    }

    /// Enqueue a complete reliable result batch or leave all queued output
    /// unchanged. Authoritative command implementations must stage mutations
    /// until this publication preflight has succeeded.
    pub fn push_reliable_batch(&self, events: Vec<ReliableEvent>) -> Result<(), EngineError> {
        if events.is_empty() {
            return Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                "reliable output batch must not be empty",
            ));
        }
        let mut incoming_bytes = 0usize;
        for event in &events {
            if matches!(event, ReliableEvent::Stopped) {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidLifecycle,
                    "orderly stop must be published as one terminal event",
                ));
            }
            let bytes = reliable_owned_bytes(event);
            self.validate_event_size(bytes)?;
            incoming_bytes = incoming_bytes.checked_add(bytes).ok_or_else(|| {
                EngineError::new(
                    EngineErrorCode::QueueByteLimit,
                    "reliable output batch byte accounting overflowed",
                )
            })?;
        }

        let mut state = lock_recover(&self.state);
        ensure_output_open(&state)?;
        let next_count = state.reliable.len().checked_add(events.len());
        let next_class = state.reliable_owned_bytes.checked_add(incoming_bytes);
        let nonreplaceable_bytes = state
            .reliable_owned_bytes
            .checked_add(state.discrete_owned_bytes)
            .and_then(|bytes| bytes.checked_add(incoming_bytes));
        if next_count.is_none_or(|count| count > self.limits.max_reliable) {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "reliable output queue limit reached",
            ));
        }
        if next_class.is_none_or(|bytes| bytes > self.limits.max_reliable_owned_bytes)
            || nonreplaceable_bytes.is_none_or(|bytes| bytes > self.limits.max_total_owned_bytes)
        {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "reliable output byte limit reached",
            ));
        }

        evict_replaceable_for(
            &mut state,
            incoming_bytes,
            self.limits.max_total_owned_bytes,
        );
        let next_total = state
            .total_owned_bytes
            .checked_add(incoming_bytes)
            .ok_or_else(|| {
                EngineError::new(
                    EngineErrorCode::QueueByteLimit,
                    "reliable output total-byte accounting overflowed",
                )
            })?;
        debug_assert!(next_total <= self.limits.max_total_owned_bytes);
        state.reliable_owned_bytes = next_class.unwrap_or(state.reliable_owned_bytes);
        state.total_owned_bytes = next_total;
        state.reliable.extend(events);
        update_output_high_water(&mut state);
        drop(state);
        self.signal_change()
    }

    /// Verify that one authority-command response can publish before the
    /// command mutates retained state. The authority thread is the sole normal
    /// reliable producer, so a successful check remains valid unless an
    /// out-of-band terminal fault closes the queue; mutation paths must use
    /// `reserve_authority_reply` to order admission against that closure.
    #[cfg(test)]
    pub(crate) fn preflight_reliable_reservation(
        &self,
        event_owned_bytes: &[usize],
    ) -> Result<(), EngineError> {
        if event_owned_bytes.is_empty() {
            return Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                "reliable output reservation must not be empty",
            ));
        }
        let mut incoming_bytes = 0usize;
        for bytes in event_owned_bytes {
            self.validate_event_size(*bytes)?;
            incoming_bytes = incoming_bytes.checked_add(*bytes).ok_or_else(|| {
                EngineError::new(
                    EngineErrorCode::QueueByteLimit,
                    "reliable output reservation byte accounting overflowed",
                )
            })?;
        }

        let mut state = lock_recover(&self.state);
        ensure_output_open(&state)?;
        self.check_reliable_reservation(&mut state, event_owned_bytes.len(), incoming_bytes)
    }

    /// Admit one authoritative reply atomically against terminal closure. The
    /// coordinator is the sole normal producer; consumers remain free to drain
    /// during checkpoint IO. Other publication attempts cannot steal the slot.
    pub(crate) fn reserve_authority_reply(
        &self,
        bytes: usize,
    ) -> Result<ReliableReplyReservation<'_>, EngineError> {
        self.validate_event_size(bytes)?;
        let mut state = lock_recover(&self.state);
        self.check_reliable_reservation(&mut state, 1, bytes)?;
        state.authority_reply_reserved = true;
        Ok(ReliableReplyReservation {
            queue: self,
            bytes,
            completed: false,
        })
    }

    fn check_reliable_reservation(
        &self,
        state: &mut OutputState,
        count: usize,
        incoming_bytes: usize,
    ) -> Result<(), EngineError> {
        ensure_output_open(state)?;
        let next_count = state.reliable.len().checked_add(count);
        let next_class = state.reliable_owned_bytes.checked_add(incoming_bytes);
        let next_total = state
            .reliable_owned_bytes
            .checked_add(state.discrete_owned_bytes)
            .and_then(|bytes| bytes.checked_add(incoming_bytes));
        if next_count.is_none_or(|count| count > self.limits.max_reliable) {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "reliable output reservation exceeds the queue count limit",
            ));
        }
        if next_class.is_none_or(|bytes| bytes > self.limits.max_reliable_owned_bytes)
            || next_total.is_none_or(|bytes| bytes > self.limits.max_total_owned_bytes)
        {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "reliable output reservation exceeds the queue byte limit",
            ));
        }
        Ok(())
    }

    /// Wait on the authority thread until its next complete reply fits. No
    /// authoritative state is touched, and the caller retains the exact command.
    /// Replaceable output can be evicted by the subsequent reliable publication.
    /// Returns whether a wait occurred so a step can take a fresh command cutoff.
    pub(crate) fn wait_reliable_capacity(&self, bytes: usize) -> Result<bool, EngineError> {
        self.validate_event_size(bytes)?;
        if bytes > self.limits.max_reliable_owned_bytes || bytes > self.limits.max_total_owned_bytes
        {
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "reliable response cannot fit even an empty output queue",
            ));
        }
        let mut state = lock_recover(&self.state);
        let mut waited = false;
        loop {
            ensure_output_open(&state)?;
            let class_bytes = state.reliable_owned_bytes.checked_add(bytes);
            let total_bytes =
                class_bytes.and_then(|value| value.checked_add(state.discrete_owned_bytes));
            if state.reliable.len() < self.limits.max_reliable
                && class_bytes.is_some_and(|value| value <= self.limits.max_reliable_owned_bytes)
                && total_bytes.is_some_and(|value| value <= self.limits.max_total_owned_bytes)
            {
                return Ok(waited);
            }
            if state.capacity_wait_cancelled {
                return Err(EngineError::new(
                    EngineErrorCode::QueueCountLimit,
                    "shutdown interrupted an output-capacity wait before authority mutation",
                ));
            }
            state.capacity_waits = state.capacity_waits.saturating_add(1);
            waited = true;
            state = wait_recover(&self.capacity_ready, state);
        }
    }

    /// Wake a blocked authority so stop/join cannot depend on a future Node drain.
    pub(crate) fn cancel_capacity_wait(&self) {
        lock_recover(&self.state).capacity_wait_cancelled = true;
        self.capacity_ready.notify_all();
    }

    /// Enqueue one non-replaceable discrete event or return an observable overflow.
    pub fn push_discrete(&self, event: DiscreteEvent) -> Result<(), EngineError> {
        let bytes = event.payload.capacity();
        let mut state = lock_recover(&self.state);
        ensure_output_open(&state)?;
        if let Err(error) = self.validate_event_size(bytes) {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(error);
        }
        let next_class = state.discrete_owned_bytes.checked_add(bytes);
        let nonreplaceable_bytes = state
            .reliable_owned_bytes
            .checked_add(state.discrete_owned_bytes)
            .and_then(|value| value.checked_add(bytes));
        if state.discrete.len() >= self.limits.max_discrete {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "discrete output queue limit reached",
            ));
        }
        if next_class.is_none_or(|value| value > self.limits.max_discrete_owned_bytes)
            || nonreplaceable_bytes.is_none_or(|value| value > self.limits.max_total_owned_bytes)
        {
            state.priority_overflows = state.priority_overflows.saturating_add(1);
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "discrete output byte limit reached",
            ));
        }
        evict_replaceable_for(&mut state, bytes, self.limits.max_total_owned_bytes);
        let next_total = state.total_owned_bytes.checked_add(bytes).ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "discrete output total-byte accounting overflowed",
            )
        })?;
        debug_assert!(next_total <= self.limits.max_total_owned_bytes);
        state.discrete_owned_bytes = next_class.unwrap_or(state.discrete_owned_bytes);
        state.total_owned_bytes = next_total;
        state.discrete.push_back(event);
        update_output_high_water(&mut state);
        drop(state);
        self.signal_change()
    }

    /// Retain only the newest stats item.
    pub fn replace_stats(&self, event: StatsEvent) -> Result<ReplaceResult, EngineError> {
        self.replace_status(ReplaceableStats::Probe(event))
    }

    /// Coalesce fixed-size display metadata behind every reliable/discrete result.
    pub(crate) fn replace_running_display(
        &self,
        event: RunningDisplayStatus,
    ) -> Result<ReplaceResult, EngineError> {
        self.replace_status(ReplaceableStats::Running(event))
    }

    /// Display copies yield to queued priority output and terminal closure.
    /// A reply reservation belongs to an unfinished command or step; it cannot
    /// change the independent cache's preceding committed frame. Blocking on
    /// that reservation would starve viewers throughout sustained long steps.
    pub(crate) fn display_copy_blocked(&self) -> bool {
        let state = lock_recover(&self.state);
        state.terminal != OutputTerminalState::Open
            || !state.reliable.is_empty()
            || !state.discrete.is_empty()
    }

    fn replace_status(&self, event: ReplaceableStats) -> Result<ReplaceResult, EngineError> {
        let bytes = event.owned_bytes();
        self.validate_event_size(bytes)?;
        let mut state = lock_recover(&self.state);
        ensure_output_open(&state)?;
        if state
            .stats
            .as_ref()
            .is_some_and(|retained| event.sequence() <= retained.sequence())
        {
            state.stale_stats = state.stale_stats.saturating_add(1);
            return Ok(ReplaceResult::Stale);
        }
        let old_bytes = state
            .stats
            .as_ref()
            .map_or(0, ReplaceableStats::owned_bytes);
        let base = state.total_owned_bytes.saturating_sub(old_bytes);
        let Some(next_total) = base.checked_add(bytes) else {
            state.stats_rejections = state.stats_rejections.saturating_add(1);
            return Ok(ReplaceResult::Rejected);
        };
        if next_total > self.limits.max_total_owned_bytes {
            state.stats_rejections = state.stats_rejections.saturating_add(1);
            return Ok(ReplaceResult::Rejected);
        }
        let replaced = state.stats.replace(event).is_some();
        state.total_owned_bytes = next_total;
        if replaced {
            state.stats_replacements = state.stats_replacements.saturating_add(1);
        }
        update_output_high_water(&mut state);
        drop(state);
        let result = if replaced {
            ReplaceResult::Replaced
        } else {
            ReplaceResult::Inserted
        };
        self.signal_change()?;
        Ok(result)
    }

    /// Retain only the newest frame for one connection.
    pub fn replace_frame(&self, event: FrameEvent) -> Result<ReplaceResult, EngineError> {
        let bytes = event.payload.capacity();
        self.validate_event_size(bytes)?;
        let mut state = lock_recover(&self.state);
        ensure_output_open(&state)?;
        if event.connection_id == 0 {
            return Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                "frame connection identity must be positive",
            ));
        }
        if state
            .frames
            .get(&event.connection_id)
            .is_some_and(|retained| event.sequence <= retained.sequence)
        {
            state.stale_frames = state.stale_frames.saturating_add(1);
            return Ok(ReplaceResult::Stale);
        }
        let existing = state.frames.get(&event.connection_id);
        let old_bytes = existing.map_or(0, |old| old.payload.capacity());
        if existing.is_none() && state.frames.len() >= self.limits.max_frame_connections {
            state.frame_rejections = state.frame_rejections.saturating_add(1);
            return Ok(ReplaceResult::Rejected);
        }
        let base = state.total_owned_bytes.saturating_sub(old_bytes);
        let Some(next_total) = base.checked_add(bytes) else {
            state.frame_rejections = state.frame_rejections.saturating_add(1);
            return Ok(ReplaceResult::Rejected);
        };
        if next_total > self.limits.max_total_owned_bytes {
            state.frame_rejections = state.frame_rejections.saturating_add(1);
            return Ok(ReplaceResult::Rejected);
        }
        let replaced = state.frames.insert(event.connection_id, event).is_some();
        state.total_owned_bytes = next_total;
        if replaced {
            state.frame_replacements = state.frame_replacements.saturating_add(1);
        }
        update_output_high_water(&mut state);
        drop(state);
        let result = if replaced {
            ReplaceResult::Replaced
        } else {
            ReplaceResult::Inserted
        };
        self.signal_change()?;
        Ok(result)
    }

    /// Retain the first fault and suppress any later orderly-stop publication.
    pub(crate) fn retain_reserved_fault(&self, fault: EngineFault) -> bool {
        let mut state = lock_recover(&self.state);
        if state.terminal != OutputTerminalState::Open || state.reserved_fault.is_some() {
            return false;
        }
        state.terminal = OutputTerminalState::Faulted;
        state
            .reliable
            .retain(|event| !matches!(event, ReliableEvent::Stopped));
        if state.authority_reply_reserved {
            state.deferred_fault = Some(fault);
        } else {
            state.reserved_fault = Some(fault);
        }
        self.capacity_ready.notify_all();
        true
    }

    /// Signal a fault already retained outside normal output capacity.
    pub(crate) fn signal_retained_fault(&self) {
        if lock_recover(&self.state).deferred_fault.is_some() {
            return;
        }
        // The retained fault and health state remain observable even when the
        // external wake mechanism is already closing or broken.
        let _ = self.signal_change();
    }

    /// Publish the first fault in a reserved slot outside normal queue capacity.
    pub fn publish_reserved_fault(&self, fault: EngineFault) {
        if self.retain_reserved_fault(fault) {
            self.signal_retained_fault();
        }
    }

    /// Drain a bounded batch in strict priority order and safely re-arm wakes.
    pub fn drain(&self, max_events: usize, max_owned_bytes: usize) -> DrainResult {
        if max_events == 0 || max_owned_bytes == 0 {
            return DrainResult {
                events: Vec::new(),
                more_work: !self.is_empty(),
                generation: self.generation.load(Ordering::Acquire),
            };
        }
        let start_generation = self.generation.load(Ordering::Acquire);
        let mut events = Vec::new();
        let mut drained_bytes = 0usize;
        {
            let mut state = lock_recover(&self.state);
            while events.len() < max_events {
                let Some(next) = pop_next(&mut state) else {
                    break;
                };
                let bytes = next.owned_bytes();
                if drained_bytes
                    .checked_add(bytes)
                    .is_none_or(|total| total > max_owned_bytes)
                {
                    restore_front(&mut state, next);
                    break;
                }
                drained_bytes = drained_bytes.saturating_add(bytes);
                if let CompletedEvent::Frame(frame) = &next {
                    state.last_drained_frame_connection = Some(frame.connection_id);
                }
                events.push(next);
            }
        }

        if !events.is_empty() {
            self.capacity_ready.notify_one();
        }
        let mut more_work = !self.is_empty();
        if !more_work {
            self.notified.store(false, Ordering::Release);
            let after_generation = self.generation.load(Ordering::Acquire);
            let generation_changed = after_generation != start_generation;
            let queue_nonempty = !self.is_empty();
            more_work = queue_nonempty;
            if more_work && !self.notified.swap(true, Ordering::AcqRel) && generation_changed {
                self.rearm_races.fetch_add(1, Ordering::Relaxed);
            }
        }
        DrainResult {
            events,
            more_work,
            generation: self.generation.load(Ordering::Acquire),
        }
    }

    /// Return a consistent output snapshot.
    pub fn metrics(&self) -> OutputMetrics {
        let state = lock_recover(&self.state);
        OutputMetrics {
            capacity_waits: state.capacity_waits,
            reliable: state.reliable.len(),
            reliable_owned_bytes: state.reliable_owned_bytes,
            discrete: state.discrete.len(),
            discrete_owned_bytes: state.discrete_owned_bytes,
            has_stats: state.stats.is_some(),
            frames: state.frames.len(),
            total_owned_bytes: state.total_owned_bytes,
            high_water_count: state.high_water_count,
            high_water_owned_bytes: state.high_water_owned_bytes,
            priority_overflows: state.priority_overflows,
            high_water_reliable: state.high_water_reliable,
            high_water_reliable_owned_bytes: state.high_water_reliable_owned_bytes,
            high_water_discrete: state.high_water_discrete,
            high_water_discrete_owned_bytes: state.high_water_discrete_owned_bytes,
            high_water_frames: state.high_water_frames,
            high_water_frame_owned_bytes: state.high_water_frame_owned_bytes,
            high_water_stats: state.high_water_stats,
            high_water_stats_owned_bytes: state.high_water_stats_owned_bytes,
            stats_replacements: state.stats_replacements,
            frame_replacements: state.frame_replacements,
            stale_stats: state.stale_stats,
            stale_frames: state.stale_frames,
            stats_rejections: state.stats_rejections,
            frame_rejections: state.frame_rejections,
            stats_evictions: state.stats_evictions,
            frame_evictions: state.frame_evictions,
            has_reserved_fault: state.reserved_fault.is_some(),
        }
    }

    /// Return wake-generation and notification counters.
    pub fn wake_metrics(&self) -> WakeMetrics {
        WakeMetrics {
            generation: self.generation.load(Ordering::Acquire),
            notification_attempts: self.notification_attempts.load(Ordering::Relaxed),
            notifications: self.notifications.load(Ordering::Relaxed),
            notification_failures: self.notification_failures.load(Ordering::Relaxed),
            rearm_races: self.rearm_races.load(Ordering::Relaxed),
            notified: self.notified.load(Ordering::Acquire),
        }
    }

    fn validate_event_size(&self, bytes: usize) -> Result<(), EngineError> {
        if bytes > self.limits.max_event_owned_bytes {
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "output event exceeds the configured per-event byte limit",
            ));
        }
        Ok(())
    }

    fn signal_change(&self) -> Result<(), EngineError> {
        // Keep the pre-rename API for the supported Rust 1.92 toolchain.
        #[allow(deprecated)]
        let _ = self
            .generation
            .fetch_update(Ordering::Release, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
        if !self.notified.swap(true, Ordering::AcqRel) {
            self.notification_attempts.fetch_add(1, Ordering::Relaxed);
            let outcome = catch_unwind(AssertUnwindSafe(|| self.sink.notify()));
            match outcome {
                Ok(Ok(())) => {
                    self.notifications.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Err(error)) => {
                    self.notification_failures.fetch_add(1, Ordering::Relaxed);
                    self.notified.store(false, Ordering::Release);
                    return Err(EngineError::new(
                        EngineErrorCode::WakeDelivery,
                        format!("wake adapter rejected notification: {error}"),
                    ));
                }
                Err(_) => {
                    self.notification_failures.fetch_add(1, Ordering::Relaxed);
                    self.notified.store(false, Ordering::Release);
                    return Err(EngineError::new(
                        EngineErrorCode::WakeDelivery,
                        "wake adapter panicked while scheduling a drain",
                    ));
                }
            }
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        let state = lock_recover(&self.state);
        state.reserved_fault.is_none()
            && state.reliable.is_empty()
            && state.discrete.is_empty()
            && state.stats.is_none()
            && state.frames.is_empty()
    }
}

/// Wake-state counters used by health reporting and race tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WakeMetrics {
    /// Changes published to the output state.
    pub generation: u64,
    /// Payload-free notification attempts.
    pub notification_attempts: u64,
    /// Payload-free notifications successfully scheduled.
    pub notifications: u64,
    /// Failed or panicking notification attempts.
    pub notification_failures: u64,
    /// Re-arm checks that found raced work for the current consumer to drain.
    pub rearm_races: u64,
    /// Whether a notification is currently armed/coalescing producers.
    pub notified: bool,
}

/// One bounded output drain result.
#[derive(Clone, Debug, PartialEq)]
pub struct DrainResult {
    /// Events in strict priority order.
    pub events: Vec<CompletedEvent>,
    /// The consumer must continue before sleeping when true.
    pub more_work: bool,
    /// Current publication generation.
    pub generation: u64,
}

fn reliable_owned_bytes(event: &ReliableEvent) -> usize {
    event.owned_bytes()
}

fn ensure_output_open(state: &OutputState) -> Result<(), EngineError> {
    if state.authority_reply_reserved {
        return Err(EngineError::new(
            EngineErrorCode::InvalidLifecycle,
            "an admitted authority operation owns the next reliable reply",
        ));
    }
    if state.terminal == OutputTerminalState::Open {
        Ok(())
    } else {
        Err(EngineError::new(
            EngineErrorCode::InvalidLifecycle,
            "engine output is already terminal",
        ))
    }
}

fn evict_replaceable_for(state: &mut OutputState, incoming: usize, limit: usize) {
    if state
        .total_owned_bytes
        .checked_add(incoming)
        .is_some_and(|total| total <= limit)
    {
        return;
    }
    if let Some(stats) = state.stats.take() {
        state.total_owned_bytes = state.total_owned_bytes.saturating_sub(stats.owned_bytes());
        state.stats_evictions = state.stats_evictions.saturating_add(1);
    }
    if state
        .total_owned_bytes
        .checked_add(incoming)
        .is_some_and(|total| total <= limit)
    {
        return;
    }
    let removed = state.frames.len() as u64;
    let frame_bytes = state.frames.values().fold(0usize, |total, frame| {
        total.saturating_add(frame.payload.capacity())
    });
    state.frames.clear();
    state.total_owned_bytes = state.total_owned_bytes.saturating_sub(frame_bytes);
    state.frame_evictions = state.frame_evictions.saturating_add(removed);
}

fn update_output_high_water(state: &mut OutputState) {
    let stats_bytes = state
        .stats
        .as_ref()
        .map_or(0, ReplaceableStats::owned_bytes);
    // The normal total is exactly reliable + discrete + stats + frame payload capacities.
    // Derive frame bytes from those maintained scalars without scanning the frame map.
    let frame_bytes = state.total_owned_bytes
        - state.reliable_owned_bytes
        - state.discrete_owned_bytes
        - stats_bytes;
    state.high_water_reliable = state.high_water_reliable.max(state.reliable.len());
    state.high_water_reliable_owned_bytes = state
        .high_water_reliable_owned_bytes
        .max(state.reliable_owned_bytes);
    state.high_water_discrete = state.high_water_discrete.max(state.discrete.len());
    state.high_water_discrete_owned_bytes = state
        .high_water_discrete_owned_bytes
        .max(state.discrete_owned_bytes);
    state.high_water_frames = state.high_water_frames.max(state.frames.len());
    state.high_water_frame_owned_bytes = state.high_water_frame_owned_bytes.max(frame_bytes);
    state.high_water_stats = state
        .high_water_stats
        .max(usize::from(state.stats.is_some()));
    state.high_water_stats_owned_bytes = state.high_water_stats_owned_bytes.max(stats_bytes);
    let count = state.reliable.len()
        + state.discrete.len()
        + usize::from(state.stats.is_some())
        + state.frames.len();
    state.high_water_count = state.high_water_count.max(count);
    state.high_water_owned_bytes = state.high_water_owned_bytes.max(state.total_owned_bytes);
}

fn pop_next(state: &mut OutputState) -> Option<CompletedEvent> {
    if let Some(fault) = state.reserved_fault.take() {
        return Some(CompletedEvent::Fault(fault));
    }
    if let Some(event) = state.reliable.pop_front() {
        let bytes = reliable_owned_bytes(&event);
        state.reliable_owned_bytes = state.reliable_owned_bytes.saturating_sub(bytes);
        state.total_owned_bytes = state.total_owned_bytes.saturating_sub(bytes);
        return Some(CompletedEvent::Reliable(event));
    }
    if let Some(event) = state.discrete.pop_front() {
        let bytes = event.payload.capacity();
        state.discrete_owned_bytes = state.discrete_owned_bytes.saturating_sub(bytes);
        state.total_owned_bytes = state.total_owned_bytes.saturating_sub(bytes);
        return Some(CompletedEvent::Discrete(event));
    }
    if let Some(event) = state.stats.take() {
        state.total_owned_bytes = state.total_owned_bytes.saturating_sub(event.owned_bytes());
        return Some(match event {
            ReplaceableStats::Probe(event) => CompletedEvent::Stats(event),
            ReplaceableStats::Running(event) => CompletedEvent::RunningDisplay(event),
        });
    }
    let oldest_sequence = state.frames.values().map(|frame| frame.sequence).min()?;
    let after_cursor = state.last_drained_frame_connection.and_then(|cursor| {
        state
            .frames
            .range((
                std::ops::Bound::Excluded(cursor),
                std::ops::Bound::Unbounded,
            ))
            .find(|(_, frame)| frame.sequence == oldest_sequence)
            .map(|(connection_id, _)| *connection_id)
    });
    let connection_id = after_cursor.or_else(|| {
        state
            .frames
            .iter()
            .find(|(_, frame)| frame.sequence == oldest_sequence)
            .map(|(connection_id, _)| *connection_id)
    })?;
    let event = state.frames.remove(&connection_id)?;
    state.total_owned_bytes = state
        .total_owned_bytes
        .saturating_sub(event.payload.capacity());
    Some(CompletedEvent::Frame(event))
}

fn restore_front(state: &mut OutputState, event: CompletedEvent) {
    match event {
        CompletedEvent::Fault(fault) => state.reserved_fault = Some(fault),
        CompletedEvent::Reliable(event) => {
            let bytes = reliable_owned_bytes(&event);
            state.reliable_owned_bytes = state.reliable_owned_bytes.saturating_add(bytes);
            state.total_owned_bytes = state.total_owned_bytes.saturating_add(bytes);
            state.reliable.push_front(event);
        }
        CompletedEvent::Discrete(event) => {
            let bytes = event.payload.capacity();
            state.discrete_owned_bytes = state.discrete_owned_bytes.saturating_add(bytes);
            state.total_owned_bytes = state.total_owned_bytes.saturating_add(bytes);
            state.discrete.push_front(event);
        }
        CompletedEvent::Stats(event) => {
            state.total_owned_bytes = state
                .total_owned_bytes
                .saturating_add(event.payload.capacity());
            state.stats = Some(ReplaceableStats::Probe(event));
        }
        CompletedEvent::RunningDisplay(event) => {
            state.total_owned_bytes = state
                .total_owned_bytes
                .saturating_add(std::mem::size_of::<RunningDisplayStatus>());
            state.stats = Some(ReplaceableStats::Running(event));
        }
        CompletedEvent::Frame(event) => {
            state.total_owned_bytes = state
                .total_owned_bytes
                .saturating_add(event.payload.capacity());
            state.frames.insert(event.connection_id, event);
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wait_recover<'a, T>(condition: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    match condition.wait(guard) {
        Ok(next) => next,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wait_timeout_while_recover<'a, T>(
    condition: &Condvar,
    guard: MutexGuard<'a, T>,
    timeout: Duration,
    mut predicate: impl FnMut(&mut T) -> bool,
) -> (MutexGuard<'a, T>, std::sync::WaitTimeoutResult) {
    match condition.wait_timeout_while(guard, timeout, &mut predicate) {
        Ok(result) => result,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests;
