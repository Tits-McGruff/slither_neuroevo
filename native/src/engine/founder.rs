//! Explicit saved-brain population initialization and durable founder provenance.

use serde::{Deserialize, Serialize};

/// Immutable run origin; descendants evolve normally and never reload this source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FounderOrigin {
    /// Founder provenance schema revision.
    pub schema_version: u32,
    /// Versioned population initialization and RNG ordering.
    pub mode: FounderMode,
    /// SHA-256 of the original portable file.
    pub artifact_sha256: String,
    /// SHA-256 of its canonical little-endian Float32 weights.
    pub weights_sha256: String,
}

/// Slot zero is an exact founder; remaining slots use ordinary configured mutation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FounderMode {
    #[serde(rename = "exact-founder-mutated-descendants-v1")]
    ExactFounderMutatedDescendantsV1,
}

impl FounderOrigin {
    /// Reject unsupported revisions and malformed fixed-width source identities.
    pub fn validate(&self) -> Result<(), &'static str> {
        let digest = |text: &str| {
            text.len() == 64
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        if self.schema_version != 1
            || !digest(&self.artifact_sha256)
            || !digest(&self.weights_sha256)
        {
            return Err("invalid founder source identity");
        }
        Ok(())
    }
}
