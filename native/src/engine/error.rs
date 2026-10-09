//! Errors exposed by the pure-Rust engine spine.

use std::error::Error;
use std::fmt::{Display, Formatter};

/// Maximum diagnostic length retained by the engine spine.
pub const MAX_ERROR_DETAIL_BYTES: usize = 512;

/// Stable categories for failures at the coarse engine boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineErrorCode {
    /// A caller-supplied limit or version is invalid.
    InvalidConfiguration,
    /// The requested operation is invalid for the current lifecycle state.
    InvalidLifecycle,
    /// A command batch or command is invalid.
    InvalidCommand,
    /// A command sequence did not advance monotonically.
    SequenceRegression,
    /// A bounded queue cannot accept another item by count.
    QueueCountLimit,
    /// A bounded queue cannot accept another item by owned bytes.
    QueueByteLimit,
    /// A coordinator thread could not be created.
    ThreadSpawn,
    /// A coordinator thread terminated with an uncaught join failure.
    ThreadJoin,
    /// The external wake adapter failed or panicked while scheduling a drain.
    WakeDelivery,
    /// The engine has faulted and cannot accept authoritative work.
    Faulted,
}

/// One bounded, cloneable engine error suitable for later N-API translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineError {
    /// Stable error category.
    pub code: EngineErrorCode,
    /// Human-readable bounded detail.
    pub detail: String,
}

impl EngineError {
    /// Construct an error while bounding untrusted or panic-derived detail.
    pub fn new(code: EngineErrorCode, detail: impl AsRef<str>) -> Self {
        Self {
            code,
            detail: truncate_utf8(detail.as_ref(), MAX_ERROR_DETAIL_BYTES),
        }
    }
}

impl Display for EngineError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.detail)
    }
}

impl Error for EngineError {}

/// Truncate text at a valid UTF-8 boundary.
pub fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_preserves_utf8_boundaries() {
        let value = "a".repeat(MAX_ERROR_DETAIL_BYTES - 1) + "é";
        let truncated = truncate_utf8(&value, MAX_ERROR_DETAIL_BYTES);
        assert_eq!(truncated.len(), MAX_ERROR_DETAIL_BYTES - 1);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
