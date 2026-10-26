//! Bounded immutable provenance carried by retained native neural owners.

use super::brain_artifact::BrainSourceKind;
use sha2::{Digest, Sha256};

/// Retained provenance is independent of the four physical snake kinds.
/// Fixed-width identities need no per-step text allocation or catalog lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedNeuralOrigin {
    /// An actual retained winner; the managed weight object remains unchanged.
    HallOfFame {
        entry_id: u64,
        weight_digest: [u8; 32],
    },
    /// A library record, including distilled, imported and manually saved brains.
    SavedBrain {
        brain_id: [u8; 32],
        weight_digest: [u8; 32],
        source_kind: BrainSourceKind,
    },
}

impl RetainedNeuralOrigin {
    /// Validate identity and bind the exact finite packed Float32 parameter bytes.
    /// Called on cold admission only; running copies share the immutable Arc.
    pub fn validate_weights(&self, weights: &[f32]) -> Result<(), String> {
        if matches!(self, Self::HallOfFame { entry_id: 0, .. }) {
            return Err("retained Hall-of-Fame origin requires a nonzero entry ID".into());
        }
        if weights.iter().any(|value| !value.is_finite()) {
            return Err("retained origin weights must be finite".into());
        }
        let expected = match self {
            Self::HallOfFame { weight_digest, .. } | Self::SavedBrain { weight_digest, .. } => {
                weight_digest
            }
        };
        if &packed_weight_digest(weights) != expected {
            return Err("retained origin digest does not match packed weights".into());
        }
        Ok(())
    }

    /// Bind all origin fields into opaque world/brain publication proofs.
    pub(crate) fn hash_into(&self, hash: &mut Sha256) {
        match self {
            Self::HallOfFame {
                entry_id,
                weight_digest,
            } => {
                hash.update([0]);
                hash.update(entry_id.to_le_bytes());
                hash.update(weight_digest);
            }
            Self::SavedBrain {
                brain_id,
                weight_digest,
                source_kind,
            } => {
                hash.update([1]);
                hash.update(brain_id);
                hash.update(weight_digest);
                hash.update([match source_kind {
                    BrainSourceKind::Distilled => 0,
                    BrainSourceKind::Imported => 1,
                    BrainSourceKind::Manual => 2,
                }]);
            }
        }
    }
}

/// Logical digest used by existing immutable HoF objects and saved brains.
pub fn packed_weight_digest(weights: &[f32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for value in weights {
        hash.update(value.to_bits().to_le_bytes());
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_bind_identity_source_kind_and_exact_parameter_bits() {
        let weights = [0.0, -0.0, 0.375];
        let origin = RetainedNeuralOrigin::HallOfFame {
            entry_id: 7,
            weight_digest: packed_weight_digest(&weights),
        };
        assert!(origin.validate_weights(&weights).is_ok());
        assert!(origin.validate_weights(&[0.0, 0.0, 0.375]).is_err());
        assert!(origin.validate_weights(&[f32::NAN]).is_err());
        assert!(RetainedNeuralOrigin::HallOfFame {
            entry_id: 0,
            weight_digest: packed_weight_digest(&weights),
        }
        .validate_weights(&weights)
        .is_err());
        let digest = |value: &RetainedNeuralOrigin| {
            let mut hash = Sha256::new();
            value.hash_into(&mut hash);
            <[u8; 32]>::from(hash.finalize())
        };
        let saved = RetainedNeuralOrigin::SavedBrain {
            brain_id: [5; 32],
            weight_digest: packed_weight_digest(&weights),
            source_kind: BrainSourceKind::Distilled,
        };
        assert!(saved.validate_weights(&weights).is_ok());
        assert_ne!(digest(&origin), digest(&saved));
        assert_ne!(
            digest(&saved),
            digest(&RetainedNeuralOrigin::SavedBrain {
                brain_id: [5; 32],
                weight_digest: packed_weight_digest(&weights),
                source_kind: BrainSourceKind::Imported,
            })
        );
    }
}
