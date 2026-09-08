//! Versioned, N-API-independent contracts for the Rust engine spine.

use super::checkpoint::{CheckpointDescriptor, CheckpointOperationId};
use super::display::RunningDisplayStatus;
use super::error::{truncate_utf8, MAX_ERROR_DETAIL_BYTES};
use super::error::{EngineError, EngineErrorCode};
use super::external_replacement::UnavailableControllerReservation;
use super::generation::GenerationCommitRecord;
use super::physics::PhysicsStepKey;
use super::running_loop::RunningGenerationStartResolution;
use super::running_step::GenerationTransitionReason;
use super::state::ControllerKind;
use std::mem::size_of;

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
    /// One typed control operation for the retained Rust-owned authority loop.
    RunningAuthority(RunningAuthorityCommand),
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
            Self::RunningAuthority(command) => command.owned_bytes(),
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
            Self::RunningAuthority(command) => command.validate(),
            Self::Unsupported { kind, .. } => Err(EngineError::new(
                EngineErrorCode::InvalidCommand,
                format!("unsupported engine command kind {kind}"),
            )),
            #[cfg(any(test, feature = "engine-test-hooks"))]
            Self::PanicForTest => Ok(()),
        }
    }

    /// Whether this command may execute only while the background thread owns
    /// a retained authoritative loop.
    #[must_use]
    pub const fn is_running_authority_control(&self) -> bool {
        matches!(self, Self::RunningAuthority(_))
    }

    /// Conservative reliable-output bytes reserved before this command may
    /// mutate retained authority state.
    fn response_reserved_owned_bytes(&self, limits: &OutputLimits) -> usize {
        match self {
            Self::Probe { payload, .. } => payload.capacity(),
            Self::RunningAuthority(_) => limits.max_event_owned_bytes,
            Self::Unsupported { .. } => 0,
            #[cfg(any(test, feature = "engine-test-hooks"))]
            Self::PanicForTest => 0,
        }
    }
}

/// One exact local-send receipt. Rust reconstructs the full retained step key;
/// JavaScript supplies only correlation fields that it previously received.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalDeliveryReceipt {
    /// Process-local operation epoch emitted by Rust.
    pub operation_epoch: u64,
    /// Monotonic reliable-event sequence emitted by Rust.
    pub event_sequence: u64,
    /// Exact live socket epoch to which Node attempted delivery.
    pub connection_id: u64,
    /// Exact controller lease epoch to which Node attempted delivery.
    pub lease_id: u64,
    /// Whether the local socket send path accepted the event.
    pub accepted: bool,
}

/// Typed commands that can resume a blocked retained generation transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunningAuthorityCommand {
    /// Publish or exactly retry the Rust-admitted immutable generation file.
    PublishGenerationCheckpoint {
        /// Server-controlled managed directory encoded as one bounded UTF-8 path.
        managed_directory: String,
        /// Exact bounded file-publication correlation token.
        operation_id: CheckpointOperationId,
    },
    /// Retain only the complete descriptor returned by the SQLite worker.
    AcknowledgeGenerationPersistence {
        /// Exact worker-committed descriptor; Rust compares every field.
        descriptor: Box<CheckpointDescriptor>,
    },
    /// Construct or reborrow deterministic connected-controller assignments.
    PrepareGenerationReassignments,
    /// Apply local delivery receipts without accepting a JavaScript-made step key.
    SubmitGenerationAssignmentReceipts {
        /// Bounded receipts correlated to the retained Rust events.
        receipts: Box<[ExternalDeliveryReceipt]>,
    },
    /// Perform the final swap only after persistence and delivery barriers pass.
    PublishAcknowledgedGenerationStart,
}

impl RunningAuthorityCommand {
    fn validate(&self) -> Result<(), EngineError> {
        match self {
            Self::PublishGenerationCheckpoint {
                managed_directory, ..
            } if managed_directory.is_empty()
                || managed_directory.len() > 32_768
                || managed_directory.contains('\0') =>
            {
                Err(EngineError::new(
                    EngineErrorCode::InvalidCommand,
                    "managed checkpoint directory must be nonempty, NUL-free, and at most 32768 UTF-8 bytes",
                ))
            }
            Self::SubmitGenerationAssignmentReceipts { receipts } if receipts.is_empty() => {
                Err(EngineError::new(
                    EngineErrorCode::InvalidCommand,
                    "generation assignment receipt batch must not be empty",
                ))
            }
            _ => Ok(()),
        }
    }

    fn owned_bytes(&self) -> Result<usize, EngineError> {
        match self {
            Self::PublishGenerationCheckpoint {
                managed_directory,
                operation_id,
            } => managed_directory
                .capacity()
                .checked_add(operation_id.owned_bytes())
                .ok_or_else(|| {
                    EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "generation checkpoint command byte accounting overflowed",
                    )
                }),
            Self::AcknowledgeGenerationPersistence { descriptor } => {
                Ok(size_of::<CheckpointDescriptor>().saturating_add(descriptor.owned_bytes()))
            }
            Self::PrepareGenerationReassignments | Self::PublishAcknowledgedGenerationStart => {
                Ok(0)
            }
            Self::SubmitGenerationAssignmentReceipts { receipts } => receipts
                .len()
                .checked_mul(size_of::<ExternalDeliveryReceipt>())
                .ok_or_else(|| {
                    EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "generation assignment receipt byte accounting overflowed",
                    )
                }),
        }
    }
}

/// One Rust-owned controller reassignment envelope. No population or archive
/// bytes are copied into this bridge record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunningGenerationAssignment {
    /// Exact source operation epoch.
    pub operation_epoch: u64,
    /// Monotonic external event identity.
    pub event_sequence: u64,
    /// Exact live socket epoch.
    pub connection_id: u64,
    /// Exact controller lease epoch.
    pub lease_id: u64,
    /// Browser player or separate Protocol 2 client.
    pub controller_kind: ControllerKind,
    /// Fresh successor snake identity.
    pub snake_id: u64,
    /// Fresh browser/frame-v1 exact identity.
    pub frame_v1_id: u32,
    /// Fresh Rust-generated opaque reclaim token.
    pub resume_token: Box<str>,
}

/// Exact result state after applying one batch of generation assignment receipts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationAssignmentReceiptState {
    /// At least one retained assignment still needs a local result.
    Pending {
        /// Exact unresolved assignment count.
        remaining: usize,
    },
    /// Every required assignment resolved while the old authority remains current.
    Ready {
        /// Exact terminal source step identity.
        source_key: PhysicsStepKey,
        /// Fully admitted successor generation.
        successor_generation: u64,
        /// Fully admitted successor completed-step chronology.
        successor_completed_step: u64,
    },
}

/// Reliable events emitted by the background Rust authority path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunningAuthorityEvent {
    /// The retained terminal step is waiting for its generation handoff.
    GenerationTransitionPending {
        /// Retained scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact terminal source step identity.
        source_key: PhysicsStepKey,
        /// Rule that ended the generation.
        reason: GenerationTransitionReason,
        /// Fully admitted successor generation.
        successor_generation: u64,
        /// Fully admitted successor completed-step chronology.
        successor_completed_step: u64,
    },
    /// Rust published the immutable file and its authoritative compact metadata.
    GenerationCheckpointPublished {
        /// Inbound command sequence.
        command_sequence: u64,
        /// Exact immutable descriptor and Rust-constructed commit record.
        descriptor: Box<CheckpointDescriptor>,
        /// Exact compact history and Hall-of-Fame reference.
        commit_record: GenerationCommitRecord,
    },
    /// Rust retained the worker's complete matching descriptor.
    GenerationPersistenceAcknowledged {
        /// Inbound command sequence.
        command_sequence: u64,
        /// Exact acknowledged operation token.
        operation_id: CheckpointOperationId,
    },
    /// Rust staged or reborrowed every required fresh-snake assignment.
    GenerationReassignmentsPrepared {
        /// Inbound command sequence.
        command_sequence: u64,
        /// Whether no local delivery remains before final publication.
        ready: bool,
        /// Canonically ordered Rust-owned assignments.
        assignments: Box<[RunningGenerationAssignment]>,
    },
    /// Rust applied one bounded receipt batch without swapping authority.
    GenerationAssignmentReceiptsApplied {
        /// Inbound command sequence.
        command_sequence: u64,
        /// Newly accepted exact assignments.
        matched_acceptances: usize,
        /// Newly failed exact assignments.
        matched_failures: usize,
        /// Stale, duplicate, or mismatched receipts ignored.
        ignored_receipts: usize,
        /// Retained barrier state after applying the receipts.
        state: GenerationAssignmentReceiptState,
    },
    /// The one final old-to-new authority swap and scheduler rebind succeeded.
    GenerationStartPublished {
        /// Inbound command sequence.
        command_sequence: u64,
        /// Complete Rust publication and retired scheduler ticket.
        resolution: RunningGenerationStartResolution,
    },
    /// A recoverable premature, stale, or mismatched control changed no authority.
    CommandRejected {
        /// Inbound command sequence.
        command_sequence: u64,
        /// Stable boundary error category.
        code: EngineErrorCode,
        /// Bounded human diagnostic.
        detail: String,
    },
}

impl RunningAuthorityEvent {
    /// Heap bytes retained by this reliable bridge event.
    #[must_use]
    pub fn owned_bytes(&self) -> usize {
        match self {
            Self::GenerationTransitionPending { .. }
            | Self::GenerationAssignmentReceiptsApplied { .. } => 0,
            Self::GenerationCheckpointPublished { descriptor, .. } => {
                size_of::<CheckpointDescriptor>().saturating_add(descriptor.owned_bytes())
            }
            Self::GenerationPersistenceAcknowledged { operation_id, .. } => {
                operation_id.owned_bytes()
            }
            Self::GenerationReassignmentsPrepared { assignments, .. } => assignments
                .len()
                .saturating_mul(size_of::<RunningGenerationAssignment>())
                .saturating_add(assignments.iter().fold(0usize, |bytes, assignment| {
                    bytes.saturating_add(assignment.resume_token.len())
                })),
            Self::GenerationStartPublished { resolution, .. } => {
                let reservations = &resolution.publication.unavailable_controller_reservations;
                generation_start_owned_bytes(reservations, reservations.capacity())
            }
            Self::CommandRejected { detail, .. } => detail.capacity(),
        }
    }
}

fn generation_start_owned_bytes(
    reservations: &[UnavailableControllerReservation],
    capacity: usize,
) -> usize {
    capacity
        .saturating_mul(size_of::<UnavailableControllerReservation>())
        .saturating_add(reservations.iter().fold(0usize, |bytes, reservation| {
            bytes
                .saturating_add(reservation.scope.capacity())
                .saturating_add(reservation.resume_token.capacity())
        }))
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
        let response_owned_bytes = self.commands.iter().try_fold(0usize, |bytes, command| {
            bytes
                .checked_add(command.command.response_reserved_owned_bytes(limits))
                .ok_or_else(|| {
                    EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "command-batch response byte accounting overflowed",
                    )
                })
        })?;
        if response_owned_bytes > limits.max_reliable_owned_bytes
            || response_owned_bytes > limits.max_total_owned_bytes
        {
            return Err(EngineError::new(
                EngineErrorCode::QueueByteLimit,
                "command-batch responses exceed reliable output byte limits",
            ));
        }
        for command in &self.commands {
            if command.command.response_reserved_owned_bytes(limits) > limits.max_event_owned_bytes
            {
                return Err(EngineError::new(
                    EngineErrorCode::QueueByteLimit,
                    "one command response exceeds the output event byte limit",
                ));
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
    /// Typed retained-authority control or lifecycle output.
    RunningAuthority(Box<RunningAuthorityEvent>),
    /// Coordinator stopped without a caught fault.
    Stopped,
}

impl ReliableEvent {
    /// Return heap bytes retained by this reliable event.
    #[must_use]
    pub fn owned_bytes(&self) -> usize {
        match self {
            Self::ProbeResult { payload, .. } => payload.capacity(),
            Self::RunningAuthority(event) => {
                size_of::<RunningAuthorityEvent>().saturating_add(event.owned_bytes())
            }
            Self::Started | Self::Stopped => 0,
        }
    }
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
#[derive(Clone, Debug, PartialEq)]
pub enum CompletedEvent {
    /// Reserved fault publication, always ahead of normal traffic.
    Fault(EngineFault),
    /// Reliable lifecycle/control output.
    Reliable(ReliableEvent),
    /// Non-replaceable discrete output.
    Discrete(DiscreteEvent),
    /// Latest status output.
    Stats(StatsEvent),
    /// Latest committed frame metadata and basic authoritative stats.
    RunningDisplay(RunningDisplayStatus),
    /// Latest display frame for a connection.
    Frame(FrameEvent),
}

impl CompletedEvent {
    /// Return bytes owned by payload/detail data.
    pub fn owned_bytes(&self) -> usize {
        match self {
            Self::Fault(fault) => fault.detail.len(),
            Self::Reliable(event) => event.owned_bytes(),
            Self::Discrete(event) => event.payload.capacity(),
            Self::Stats(event) => event.payload.capacity(),
            Self::RunningDisplay(_) => size_of::<RunningDisplayStatus>(),
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
