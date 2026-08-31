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
use super::error::{EngineError, EngineErrorCode};

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
    stop_requested: AtomicBool,
}

/// Reason the background coordinator's condition-variable wait completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InboundWaitResult {
    /// One or more command batches are ready for the next atomic drain.
    CommandsReady,
    /// The Rust-owned scheduler timeout elapsed without an inbound command.
    TimedOut,
    /// Stop was requested after every previously accepted batch was drained.
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
            }),
            ready: Condvar::new(),
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
        let state = lock_recover(&self.state);
        let state = match timeout {
            Some(timeout) => {
                let (state, _) =
                    wait_timeout_while_recover(&self.ready, state, timeout, |current| {
                        current.queue.is_empty() && !self.stop_requested.load(Ordering::Acquire)
                    });
                state
            }
            None => {
                let mut state = state;
                while state.queue.is_empty() && !self.stop_requested.load(Ordering::Acquire) {
                    state = wait_recover(&self.ready, state);
                }
                state
            }
        };

        if !state.queue.is_empty() {
            InboundWaitResult::CommandsReady
        } else if self.stop_requested.load(Ordering::Acquire) {
            InboundWaitResult::Stopped
        } else {
            InboundWaitResult::TimedOut
        }
    }

    /// Drain every command accepted before one fixed-step boundary.
    ///
    /// The caller owns and reuses `output`. Commands accepted after this
    /// method releases the queue mutex belong to the following boundary.
    pub(crate) fn drain_step_boundary(&self, output: &mut Vec<CommandBatch>) -> bool {
        output.clear();
        let mut state = lock_recover(&self.state);
        output.reserve(state.queue.len());
        while let Some(queued) = state.queue.pop_front() {
            state.commands = state.commands.saturating_sub(queued.command_count);
            state.owned_bytes = state.owned_bytes.saturating_sub(queued.owned_bytes);
            output.push(queued.batch);
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

    /// Close admission and discard work that cannot run after an engine fault.
    pub fn request_fault_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        let mut state = lock_recover(&self.state);
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
    reliable: VecDeque<ReliableEvent>,
    reliable_owned_bytes: usize,
    discrete: VecDeque<DiscreteEvent>,
    discrete_owned_bytes: usize,
    stats: Option<StatsEvent>,
    frames: BTreeMap<u64, FrameEvent>,
    last_drained_frame_connection: Option<u64>,
    total_owned_bytes: usize,
    high_water_count: usize,
    high_water_owned_bytes: usize,
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

impl OutputQueue {
    /// Create an empty output queue from already validated limits.
    pub fn new(limits: OutputLimits, sink: Arc<dyn WakeSink>) -> Self {
        Self {
            limits,
            state: Mutex::new(OutputState {
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
            generation: AtomicU64::new(0),
            notified: AtomicBool::new(false),
            notifications: AtomicU64::new(0),
            notification_attempts: AtomicU64::new(0),
            notification_failures: AtomicU64::new(0),
            rearm_races: AtomicU64::new(0),
        }
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
        {
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
        }

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
        let bytes = event.payload.capacity();
        self.validate_event_size(bytes)?;
        let mut state = lock_recover(&self.state);
        ensure_output_open(&state)?;
        if state
            .stats
            .as_ref()
            .is_some_and(|retained| event.sequence <= retained.sequence)
        {
            state.stale_stats = state.stale_stats.saturating_add(1);
            return Ok(ReplaceResult::Stale);
        }
        let old_bytes = state.stats.as_ref().map_or(0, |old| old.payload.capacity());
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
        state.reserved_fault = Some(fault);
        true
    }

    /// Signal a fault already retained outside normal output capacity.
    pub(crate) fn signal_retained_fault(&self) {
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrainResult {
    /// Events in strict priority order.
    pub events: Vec<CompletedEvent>,
    /// The consumer must continue before sleeping when true.
    pub more_work: bool,
    /// Current publication generation.
    pub generation: u64,
}

fn reliable_owned_bytes(event: &ReliableEvent) -> usize {
    match event {
        ReliableEvent::ProbeResult { payload, .. } => payload.capacity(),
        ReliableEvent::Started | ReliableEvent::Stopped => 0,
    }
}

fn ensure_output_open(state: &OutputState) -> Result<(), EngineError> {
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
        state.total_owned_bytes = state
            .total_owned_bytes
            .saturating_sub(stats.payload.capacity());
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
        state.total_owned_bytes = state
            .total_owned_bytes
            .saturating_sub(event.payload.capacity());
        return Some(CompletedEvent::Stats(event));
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
            state.stats = Some(event);
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
mod tests {
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
    fn fault_stop_discards_accepted_work_and_closes_admission() {
        let queue = InboundQueue::new(inbound_limits());
        queue
            .try_push(batch(vec![probe(1, 2), probe(2, 2)]))
            .assert_ok();
        queue.request_fault_stop();
        let metrics = queue.metrics();
        assert_eq!(metrics.batches, 0);
        assert_eq!(metrics.commands, 0);
        assert_eq!(metrics.owned_bytes, 0);
        assert_eq!(metrics.fault_discarded_batches, 1);
        assert_eq!(metrics.fault_discarded_commands, 2);
        assert_eq!(metrics.fault_discarded_owned_bytes, 4);
        assert!(queue.wait_pop().is_none());
        assert!(queue.try_push(batch(vec![probe(3, 1)])).is_err());
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
        fault_first
            .publish_reserved_fault(EngineError::new(EngineErrorCode::Faulted, "first").into());
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
        stop_first
            .publish_reserved_fault(EngineError::new(EngineErrorCode::Faulted, "late").into());
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
}
