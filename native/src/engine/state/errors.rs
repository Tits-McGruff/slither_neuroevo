//! State admission and publication errors.

use super::*;

/// Validation or checked-memory failure.
#[derive(Clone, Debug, PartialEq)]
pub enum StateError {
    /// Required version/identity/text field is missing or invalid.
    InvalidField { field: &'static str, reason: String },
    /// A finite scalar contract was violated.
    NonFinite { field: &'static str, index: usize },
    /// Checked byte/count arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Reusable validation storage could not reserve its checked size.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// A prepared fixed step no longer names the current authority/operation.
    StaleFixedStep { field: PhysicsStepKeyField },
    /// Estimated state exceeds the caller-approved ceiling.
    MemoryCeilingExceeded {
        /// Checked state estimate.
        estimated_bytes: usize,
        /// Caller-approved ceiling.
        ceiling_bytes: usize,
    },
    /// A monotonic allocator cannot satisfy the requested range.
    IdExhausted {
        /// Allocation namespace.
        kind: &'static str,
        /// Number of identities requested atomically.
        requested: u64,
    },
    /// Population slots must be exactly dense and ordered.
    NonDensePopulationSlot { index: usize, slot: u32 },
    /// A stable identity was repeated.
    DuplicateId { kind: &'static str, id: u64 },
    /// A brain handle was repeated or mismatched.
    DuplicateBrainHandle(BrainHandle),
    /// Genome packed weight length disagrees with the compiled graph.
    WeightLength {
        /// Population slot.
        slot: u32,
        /// Required packed-float count.
        expected: usize,
        /// Supplied packed-float count.
        actual: usize,
    },
    /// A non-population brain's packed weight length is invalid.
    BrainWeightLength {
        /// Stable brain handle.
        handle: BrainHandle,
        /// Required packed-float count.
        expected: usize,
        /// Supplied packed-float count.
        actual: usize,
    },
    /// Brain recurrent-state length disagrees with the compiled graph.
    RecurrentLength {
        /// Brain handle.
        handle: BrainHandle,
        /// Required packed-float count.
        expected: usize,
        /// Supplied packed-float count.
        actual: usize,
    },
    /// Brain owner/epoch does not match its population or entity.
    InvalidBrainOwner(BrainHandle),
    /// Body storage range is invalid or overlaps another snake.
    InvalidBodyRange { snake_id: u64 },
    /// Exact generation boundary contains live/transient state.
    DirtyGenerationBoundary { reason: &'static str },
    /// RNG continuation failed strict algorithm/state validation.
    InvalidRng { stream: String, reason: String },
    /// A lease references no live snake.
    UnknownLeaseSnake(u64),
    /// A controller lease identity is repeated.
    DuplicateLeaseId(u64),
    /// Multiple leases target the same snake.
    DuplicateLeaseSnake(u64),
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => write!(formatter, "invalid {field}: {reason}"),
            Self::NonFinite { field, index } => {
                write!(formatter, "{field}[{index}] must be finite")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "checked arithmetic overflow while calculating {context}")
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "could not reserve {required} entries for {context}"
            ),
            Self::StaleFixedStep { field } => {
                write!(formatter, "stale fixed-step proposal: {field:?} changed")
            }
            Self::MemoryCeilingExceeded {
                estimated_bytes,
                ceiling_bytes,
            } => write!(
                formatter,
                "state requires an estimated {estimated_bytes} bytes, exceeding the {ceiling_bytes}-byte ceiling"
            ),
            Self::IdExhausted { kind, requested } => {
                write!(formatter, "{kind} ID space cannot reserve {requested} identities")
            }
            Self::NonDensePopulationSlot { index, slot } => {
                write!(formatter, "population index {index} contains slot {slot}")
            }
            Self::DuplicateId { kind, id } => write!(formatter, "duplicate {kind} id {id}"),
            Self::DuplicateBrainHandle(handle) => write!(
                formatter,
                "duplicate brain handle {} at epoch {}",
                handle.id, handle.epoch
            ),
            Self::WeightLength {
                slot,
                expected,
                actual,
            } => write!(
                formatter,
                "population slot {slot} has {actual} weights; expected {expected}"
            ),
            Self::BrainWeightLength {
                handle,
                expected,
                actual,
            } => write!(
                formatter,
                "non-population brain {}:{} has {actual} weights; expected {expected}",
                handle.id, handle.epoch
            ),
            Self::RecurrentLength {
                handle,
                expected,
                actual,
            } => write!(
                formatter,
                "brain {}:{} has {actual} recurrent values; expected {expected}",
                handle.id, handle.epoch
            ),
            Self::InvalidBrainOwner(handle) => write!(
                formatter,
                "brain {}:{} has an invalid owner/epoch mapping",
                handle.id, handle.epoch
            ),
            Self::InvalidBodyRange { snake_id } => {
                write!(formatter, "snake {snake_id} has an invalid or overlapping body range")
            }
            Self::DirtyGenerationBoundary { reason } => {
                write!(formatter, "generation boundary is not clean: {reason}")
            }
            Self::InvalidRng { stream, reason } => {
                write!(formatter, "invalid RNG stream {stream}: {reason}")
            }
            Self::UnknownLeaseSnake(id) => write!(formatter, "lease references unknown snake {id}"),
            Self::DuplicateLeaseId(id) => write!(formatter, "duplicate controller lease id {id}"),
            Self::DuplicateLeaseSnake(id) => {
                write!(formatter, "multiple controller leases target snake {id}")
            }
        }
    }
}

impl Error for StateError {}
