//! Versioned, N-API-independent contracts for the Rust engine spine.

use super::error::{truncate_utf8, MAX_ERROR_DETAIL_BYTES};
use super::error::{EngineError, EngineErrorCode};

/// First supported engine-spine contract version.
pub const ENGINE_CONTRACT_VERSION: u32 = 1;

/// Caller-supplied inbound queue limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InboundLimits {
    /// Maximum queued command batches.
    pub max_batches: usize,
    /// Maximum queued commands across all batches.
    pub max_commands: usize,
    /// Maximum owned command payload bytes across all batches.
    pub max_owned_bytes: usize,
    /// Maximum commands accepted in one atomic batch.
    pub max_batch_commands: usize,
    /// Maximum owned payload bytes accepted in one atomic batch.
    pub max_batch_owned_bytes: usize,
}

/// Caller-supplied outbound queue limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputLimits {
    /// Maximum normally queued reliable events.
    pub max_reliable: usize,
    /// Maximum owned bytes in normally queued reliable events.
    pub max_reliable_owned_bytes: usize,
    /// Maximum normally queued discrete events.
    pub max_discrete: usize,
    /// Maximum owned bytes in normally queued discrete events.
    pub max_discrete_owned_bytes: usize,
    /// Maximum total bytes owned by all normal output classes.
    pub max_total_owned_bytes: usize,
    /// Maximum owned payload bytes in one output event.
    pub max_event_owned_bytes: usize,
    /// Maximum number of connections retaining a replaceable frame.
    pub max_frame_connections: usize,
}

/// Versioned initialization contract for the minimum engine spine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineInit {
    /// Must equal [`ENGINE_CONTRACT_VERSION`].
    pub contract_version: u32,
    /// Inbound limits.
    pub inbound: InboundLimits,
    /// Outbound limits.
    pub output: OutputLimits,
}

impl EngineInit {
    /// Validate every caller-supplied limit before allocating runtime state.
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.contract_version != ENGINE_CONTRACT_VERSION {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                format!(
                    "unsupported engine contract version {}; expected {}",
                    self.contract_version, ENGINE_CONTRACT_VERSION
                ),
            ));
        }
        let positive = [
            self.inbound.max_batches,
            self.inbound.max_commands,
            self.inbound.max_owned_bytes,
            self.inbound.max_batch_commands,
            self.inbound.max_batch_owned_bytes,
            self.output.max_reliable,
            self.output.max_reliable_owned_bytes,
            self.output.max_discrete,
            self.output.max_discrete_owned_bytes,
            self.output.max_total_owned_bytes,
            self.output.max_event_owned_bytes,
            self.output.max_frame_connections,
        ];
        if positive.contains(&0) {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "engine queue, count, and byte limits must all be positive",
            ));
        }
        if self.inbound.max_batch_commands > self.inbound.max_commands
            || self.inbound.max_batch_owned_bytes > self.inbound.max_owned_bytes
        {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "one-batch inbound limits cannot exceed total inbound limits",
            ));
        }
        if self.inbound.max_batch_commands >= self.output.max_reliable
            || self.inbound.max_batch_owned_bytes > self.output.max_reliable_owned_bytes
            || self.inbound.max_batch_owned_bytes > self.output.max_total_owned_bytes
        {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "one inbound batch must fit beside a lifecycle event in an empty reliable output queue",
            ));
        }
        if self.output.max_event_owned_bytes > self.output.max_total_owned_bytes
            || self.output.max_event_owned_bytes > self.output.max_reliable_owned_bytes
            || self.output.max_event_owned_bytes > self.output.max_discrete_owned_bytes
        {
            return Err(EngineError::new(
                EngineErrorCode::InvalidConfiguration,
                "one-event byte limit cannot exceed its output byte limits",
            ));
        }
        Ok(())
    }
}

/// One command with its exact internal 64-bit arrival sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequencedCommand {
    /// Strictly increasing sequence assigned at the bridge boundary.
    pub sequence: u64,
    /// Supported or explicitly unsupported command body.
    pub command: EngineCommand,
}

/// Commands understood by the minimum Stage 3 coordinator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineCommand {
    /// Bounded correlated payload used to exercise the coarse bridge.
    Probe {
        /// Correlates the response without narrowing the value through JavaScript.
        correlation_id: u64,
        /// Payload retained and echoed by the background coordinator.
        payload: Vec<u8>,
    },
    /// Explicit representation for a command kind that this contract cannot execute.
    Unsupported {
        /// Numeric kind retained for a clear rejection at a future parser boundary.
        kind: u32,
        /// Declared owned size used only for bounded preflight accounting.
        declared_owned_bytes: usize,
    },
    /// Test-only coordinator panic injection; never compiled into production.
    #[cfg(any(test, feature = "engine-test-hooks"))]
    PanicForTest,
}

impl EngineCommand {
    /// Return owned payload bytes used for queue accounting.
    pub fn owned_bytes(&self) -> Result<usize, EngineError> {
        match self {
            Self::Probe { payload, .. } => Ok(payload.capacity()),
            Self::Unsupported {
                declared_owned_bytes,
                ..
            } => Ok(*declared_owned_bytes),
            #[cfg(any(test, feature = "engine-test-hooks"))]
            Self::PanicForTest => Ok(0),
        }
    }

    /// Reject command kinds absent from the current version.
    pub fn validate_supported(&self) -> Result<(), EngineError> {
        match self {
            Self::Probe { .. } => Ok(()),
            Self::Unsupported { kind, .. } => Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                format!("unsupported engine command kind {kind}"),
            )),
            #[cfg(any(test, feature = "engine-test-hooks"))]
            Self::PanicForTest => Ok(()),
        }
    }
}

/// One all-or-nothing inbound command batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandBatch {
    /// Must equal [`ENGINE_CONTRACT_VERSION`].
    pub contract_version: u32,
    /// Commands accepted or rejected as one queue operation.
    pub commands: Box<[SequencedCommand]>,
}

impl CommandBatch {
    /// Validate version, non-emptiness, supported commands, and internal ordering.
    pub fn validate(&self) -> Result<BatchShape, EngineError> {
        if self.contract_version != ENGINE_CONTRACT_VERSION {
            return Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                format!(
                    "unsupported command-batch version {}; expected {}",
                    self.contract_version, ENGINE_CONTRACT_VERSION
                ),
            ));
        }
        let Some(first) = self.commands.first() else {
            return Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                "command batch must not be empty",
            ));
        };
        if first.sequence == 0 {
            return Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                "command sequences start at one",
            ));
        }
        let mut prior = None;
        let mut owned_bytes = 0usize;
        for command in &self.commands {
            command.command.validate_supported()?;
            if let Some(previous) = prior {
                if command.sequence <= previous {
                    return Err(EngineError::new(
                        EngineErrorCode::SequenceRegression,
                        "command sequences must increase strictly within a batch",
                    ));
                }
            }
            prior = Some(command.sequence);
            owned_bytes = owned_bytes
                .checked_add(command.command.owned_bytes()?)
                .ok_or_else(|| {
                    EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "command-batch owned-byte accounting overflowed",
                    )
                })?;
        }
        Ok(BatchShape {
            command_count: self.commands.len(),
            owned_bytes,
            first_sequence: first.sequence,
            last_sequence: prior.unwrap_or(first.sequence),
        })
    }

    /// Verify that this currently supported batch can publish its complete
    /// response atomically when the normal output queue is otherwise empty.
    pub fn validate_output_shape(&self, limits: &OutputLimits) -> Result<BatchShape, EngineError> {
        let shape = self.validate()?;
        if shape.command_count >= limits.max_reliable {
            return Err(EngineError::new(
                EngineErrorCode::QueueCountLimit,
                "command batch leaves no reliable lifecycle-event capacity",
            ));
        }
        if shape.owned_bytes > limits.max_reliable_owned_bytes
            || shape.owned_bytes > limits.max_total_owned_bytes
        {
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "command-batch responses exceed reliable output byte limits",
            ));
        }
        for command in &self.commands {
            if let EngineCommand::Probe { payload, .. } = &command.command {
                if payload.capacity() > limits.max_event_owned_bytes {
                    return Err(EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "one command response exceeds the output event byte limit",
                    ));
                }
            }
        }
        Ok(shape)
    }
}

/// Validated dimensions of one batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchShape {
    /// Number of commands.
    pub command_count: usize,
    /// Owned payload bytes.
    pub owned_bytes: usize,
    /// First sequence in the batch.
    pub first_sequence: u64,
    /// Last sequence in the batch.
    pub last_sequence: u64,
}

/// Reliable coordinator output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReliableEvent {
    /// Coordinator accepted its one-shot start.
    Started,
    /// Correlated probe result.
    ProbeResult {
        /// Original command sequence.
        sequence: u64,
        /// Original correlation identifier.
        correlation_id: u64,
        /// Echoed bounded payload.
        payload: Vec<u8>,
    },
    /// Coordinator stopped without a caught fault.
    Stopped,
}

/// Non-replaceable discrete event placeholder for later generation/Hall-of-Fame work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscreteEvent {
    /// Exact event sequence.
    pub sequence: u64,
    /// Opaque bounded payload for the future typed bridge adapter.
    pub payload: Vec<u8>,
}

/// Replaceable status payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatsEvent {
    /// Exact publication sequence.
    pub sequence: u64,
    /// Prepared bounded payload.
    pub payload: Vec<u8>,
}

/// Replaceable display payload for one connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameEvent {
    /// Exact transport connection identifier.
    pub connection_id: u64,
    /// Exact publication sequence.
    pub sequence: u64,
    /// Prepared frame bytes.
    pub payload: Vec<u8>,
}

/// Bounded fault record stored outside normal output capacity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineFault {
    /// Stable fault category.
    code: EngineErrorCode,
    /// Bounded diagnostic detail.
    detail: Box<str>,
}

impl EngineFault {
    /// Construct a fault whose retained diagnostic cannot exceed the reserve.
    pub fn new(code: EngineErrorCode, detail: impl AsRef<str>) -> Self {
        Self {
            code,
            detail: truncate_utf8(detail.as_ref(), MAX_ERROR_DETAIL_BYTES).into_boxed_str(),
        }
    }

    /// Read the stable fault category.
    #[must_use]
    pub fn code(&self) -> EngineErrorCode {
        self.code
    }

    /// Read the bounded human diagnostic.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl From<EngineError> for EngineFault {
    fn from(value: EngineError) -> Self {
        Self::new(value.code, value.detail)
    }
}

/// Drained output in priority order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletedEvent {
    /// Reserved fault publication, always ahead of normal traffic.
    Fault(EngineFault),
    /// Reliable lifecycle/control output.
    Reliable(ReliableEvent),
    /// Non-replaceable discrete output.
    Discrete(DiscreteEvent),
    /// Latest status output.
    Stats(StatsEvent),
    /// Latest display frame for a connection.
    Frame(FrameEvent),
}

impl CompletedEvent {
    /// Return bytes owned by payload/detail data.
    pub fn owned_bytes(&self) -> usize {
        match self {
            Self::Fault(fault) => fault.detail.len(),
            Self::Reliable(ReliableEvent::ProbeResult { payload, .. }) => payload.capacity(),
            Self::Reliable(ReliableEvent::Started | ReliableEvent::Stopped) => 0,
            Self::Discrete(event) => event.payload.capacity(),
            Self::Stats(event) => event.payload.capacity(),
            Self::Frame(event) => event.payload.capacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_command_is_rejected_not_ignored() {
        let batch = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::Unsupported {
                    kind: 91,
                    declared_owned_bytes: 4,
                },
            }]
            .into_boxed_slice(),
        };
        assert_eq!(
            batch.validate().err().map(|error| error.code),
            Some(EngineErrorCode::InvalidCommand)
        );
    }

    #[test]
    fn exact_u64_sequences_are_not_narrowed() {
        let batch = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: u64::MAX,
                command: EngineCommand::Probe {
                    correlation_id: u64::MAX - 1,
                    payload: vec![1],
                },
            }]
            .into_boxed_slice(),
        };
        let shape = batch
            .validate()
            .expect("maximum u64 sequence remains valid");
        assert_eq!(shape.first_sequence, u64::MAX);
        assert_eq!(shape.last_sequence, u64::MAX);
    }

    #[test]
    fn command_storage_has_no_hidden_spare_capacity() {
        let mut commands = Vec::with_capacity(8);
        commands.push(SequencedCommand {
            sequence: 1,
            command: EngineCommand::Probe {
                correlation_id: 1,
                payload: Vec::with_capacity(16),
            },
        });
        let batch = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: commands.into_boxed_slice(),
        };
        assert_eq!(batch.commands.len(), 1);
        assert!(batch.validate().is_ok());
    }

    #[test]
    fn response_shape_rejects_per_event_and_batch_overflow_before_queueing() {
        let limits = OutputLimits {
            max_reliable: 3,
            max_reliable_owned_bytes: 8,
            max_discrete: 1,
            max_discrete_owned_bytes: 8,
            max_total_owned_bytes: 12,
            max_event_owned_bytes: 4,
            max_frame_connections: 1,
        };
        let oversized_event = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence: 1,
                command: EngineCommand::Probe {
                    correlation_id: 1,
                    payload: vec![0; 5],
                },
            }]
            .into_boxed_slice(),
        };
        assert_eq!(
            oversized_event
                .validate_output_shape(&limits)
                .expect_err("event does not fit")
                .code,
            EngineErrorCode::QueueByteLimit
        );

        let too_many = CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![
                SequencedCommand {
                    sequence: 1,
                    command: EngineCommand::Probe {
                        correlation_id: 1,
                        payload: vec![1],
                    },
                },
                SequencedCommand {
                    sequence: 2,
                    command: EngineCommand::Probe {
                        correlation_id: 2,
                        payload: vec![2],
                    },
                },
                SequencedCommand {
                    sequence: 3,
                    command: EngineCommand::Probe {
                        correlation_id: 3,
                        payload: vec![3],
                    },
                },
            ]
            .into_boxed_slice(),
        };
        assert_eq!(
            too_many
                .validate_output_shape(&limits)
                .expect_err("lifecycle reserve is preserved")
                .code,
            EngineErrorCode::QueueCountLimit
        );
    }

    #[test]
    fn fault_diagnostics_are_utf8_bounded_at_construction() {
        let fault = EngineFault::new(EngineErrorCode::Faulted, "é".repeat(400));
        assert_eq!(fault.code(), EngineErrorCode::Faulted);
        assert!(fault.detail().len() <= MAX_ERROR_DETAIL_BYTES);
        assert!(fault.detail().is_char_boundary(fault.detail().len()));
    }
}
