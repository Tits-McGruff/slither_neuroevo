//! Background coordinator loop for the minimum Rust engine spine.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::contract::{EngineCommand, EngineFault, ReliableEvent};
use super::error::{EngineError, EngineErrorCode};
use super::queues::{InboundQueue, OutputQueue};

/// Observable lifecycle of the one-shot engine coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleState {
    /// Constructed but not started.
    Created,
    /// Background coordinator is accepting work.
    Running,
    /// An orderly stop has been requested.
    StopRequested,
    /// A caught panic or unrecoverable bounded-queue failure stopped authority.
    Faulted,
    /// Coordinator is no longer running and cannot restart.
    Stopped,
}

/// State shared between runtime control methods and the coordinator root.
#[derive(Debug)]
pub(crate) struct CoordinatorState {
    lifecycle: Mutex<LifecycleState>,
    fault: Mutex<Option<EngineFault>>,
    processed_batches: AtomicU64,
    processed_commands: AtomicU64,
}

impl CoordinatorState {
    /// Create state for an unstarted runtime.
    pub(crate) fn new() -> Self {
        Self {
            lifecycle: Mutex::new(LifecycleState::Created),
            fault: Mutex::new(None),
            processed_batches: AtomicU64::new(0),
            processed_commands: AtomicU64::new(0),
        }
    }

    /// Read the current lifecycle.
    pub(crate) fn lifecycle(&self) -> LifecycleState {
        *lock_recover(&self.lifecycle)
    }

    /// Execute a small lifecycle transition under its dedicated lock.
    pub(crate) fn transition(
        &self,
        transition: impl FnOnce(LifecycleState) -> Result<LifecycleState, EngineError>,
    ) -> Result<LifecycleState, EngineError> {
        let mut state = lock_recover(&self.lifecycle);
        let next = transition(*state)?;
        *state = next;
        Ok(next)
    }

    /// Mark normal coordinator completion without overwriting a fault.
    pub(crate) fn mark_normal_stopped(&self) {
        let mut lifecycle = lock_recover(&self.lifecycle);
        if *lifecycle != LifecycleState::Faulted {
            *lifecycle = LifecycleState::Stopped;
        }
    }

    /// Convert a faulted coordinator to terminal stopped state after joining.
    pub(crate) fn mark_joined(&self) {
        let mut lifecycle = lock_recover(&self.lifecycle);
        *lifecycle = LifecycleState::Stopped;
    }

    /// Return the retained first fault.
    pub(crate) fn fault(&self) -> Option<EngineFault> {
        lock_recover(&self.fault).clone()
    }

    /// Publish the first fault without using normal output capacity.
    pub(crate) fn publish_fault(&self, output: &OutputQueue, error: EngineError) {
        let fault = EngineFault::from(error);
        let first = {
            let mut retained = lock_recover(&self.fault);
            if retained.is_none() {
                *retained = Some(fault.clone());
                true
            } else {
                false
            }
        };
        {
            let mut lifecycle = lock_recover(&self.lifecycle);
            if *lifecycle != LifecycleState::Stopped {
                *lifecycle = LifecycleState::Faulted;
            }
        }
        if first {
            output.publish_reserved_fault(fault);
        }
    }

    /// Snapshot processed work counters.
    pub(crate) fn processed(&self) -> (u64, u64) {
        (
            self.processed_batches.load(Ordering::Relaxed),
            self.processed_commands.load(Ordering::Relaxed),
        )
    }
}

/// Atomically close future admission, discard unapplied work, and retain the
/// first fault. A racing successful submit was linearized before this closure
/// and is reported by the discarded-work counters rather than as applied.
pub(crate) fn fault_and_stop(
    inbound: &InboundQueue,
    output: &OutputQueue,
    state: &CoordinatorState,
    error: EngineError,
) {
    inbound.request_fault_stop();
    state.publish_fault(output, error);
}

/// Run until out-of-band stop, a caught outer panic, or an output fault.
pub(crate) fn run_coordinator(
    inbound: &Arc<InboundQueue>,
    output: &Arc<OutputQueue>,
    state: &Arc<CoordinatorState>,
) {
    if let Err(error) = output.push_reliable(ReliableEvent::Started) {
        fault_and_stop(inbound, output, state, error);
        return;
    }

    while let Some(batch) = inbound.wait_pop() {
        let command_count = batch.commands.len();
        let mut results = Vec::with_capacity(command_count);
        for sequenced in batch.commands {
            match sequenced.command {
                EngineCommand::Probe {
                    correlation_id,
                    payload,
                } => {
                    results.push(ReliableEvent::ProbeResult {
                        sequence: sequenced.sequence,
                        correlation_id,
                        payload,
                    });
                }
                EngineCommand::Unsupported { kind, .. } => {
                    fault_and_stop(
                        inbound,
                        output,
                        state,
                        EngineError::new(
                            EngineErrorCode::InvalidCommand,
                            format!("unsupported command kind {kind} reached coordinator"),
                        ),
                    );
                    return;
                }
                #[cfg(any(test, feature = "engine-test-hooks"))]
                EngineCommand::PanicForTest => {
                    panic!("test-only coordinator panic injection");
                }
            }
        }
        if let Err(error) = output.push_reliable_batch(results) {
            fault_and_stop(inbound, output, state, error);
            return;
        }
        saturating_increment(
            &state.processed_commands,
            u64::try_from(command_count).unwrap_or(u64::MAX),
        );
        saturating_increment(&state.processed_batches, 1);
    }

    if state.lifecycle() != LifecycleState::Faulted {
        if let Err(error) = output.push_reliable(ReliableEvent::Stopped) {
            fault_and_stop(inbound, output, state, error);
        }
    }
}

fn saturating_increment(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}

/// Convert a caught panic payload into bounded generic diagnostic detail.
pub(crate) fn panic_error(payload: &(dyn Any + Send)) -> EngineError {
    let detail = if let Some(message) = payload.downcast_ref::<&str>() {
        format!("coordinator panic: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("coordinator panic: {message}")
    } else {
        "coordinator panic with non-string payload".to_owned()
    };
    EngineError::new(EngineErrorCode::Faulted, detail)
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
