//! Versioned deterministic random-number generation for authoritative engine state.
//!
//! Uniform draws and seed mixing mirror the TypeScript-reference `src/rng.ts`
//! contract exactly. Labelled seeds mix JavaScript UTF-16 code units, not
//! UTF-8 bytes, and serialized Gaussian spares retain their exact IEEE-754
//! bits.
//!
//! Newly generated Gaussian pairs use Rust's target-provided `f64::ln` and
//! `f64::sqrt`. Those operations are version-scoped to the Rust backend build
//! and target; they are not claimed to be bit-identical to V8. An uncached
//! TypeScript state can therefore continue in Rust with the same uniform
//! state and tolerance-based Gaussian compatibility, but not an exact
//! cross-runtime Gaussian continuation.

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;

/// Stable identifier for the xorshift uniform algorithm.
pub const RNG_ALGORITHM: &str = "xorshift32";
/// Serialized-state version for the uniform algorithm.
pub const RNG_VERSION: u32 = 1;
/// Rust-specific identifier for the Gaussian transform and standard-library math implementation.
pub const GAUSSIAN_ALGORITHM: &str = "box-muller-polar-rust-std-f64";
/// Serialized-state version for the Rust Gaussian transform.
pub const GAUSSIAN_VERSION: u32 = 1;
/// Legacy TypeScript/V8 Gaussian identifier accepted only by the explicit migration adapter.
pub const LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM: &str = "box-muller-polar";
/// Legacy TypeScript/V8 Gaussian state version accepted by the migration adapter.
pub const LEGACY_TYPESCRIPT_GAUSSIAN_VERSION: u32 = 1;

/// FNV-1a 32-bit offset basis.
const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
/// FNV-1a 32-bit prime.
const FNV_PRIME: u32 = 0x0100_0193;
/// xorshift32's forbidden all-zero state is replaced with this value.
const NON_ZERO_XORSHIFT_STATE: u32 = 1;
/// The largest exactly representable safe integer in JavaScript.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// JSON-compatible, lossless continuation state for a deterministic stream.
///
/// The coarse bridge serialization adapter is still pending. That adapter must
/// explicitly map these Rust field names to the established TypeScript JSON
/// names and must not silently discard either algorithm identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedRngState {
    /// Uniform random algorithm identifier.
    pub algorithm: String,
    /// Uniform random algorithm state version.
    pub version: u32,
    /// Exact xorshift state as an eight-digit hexadecimal Uint32.
    pub state_hex: String,
    /// Gaussian transform identifier.
    pub gaussian_algorithm: String,
    /// Gaussian transform state version.
    pub gaussian_version: u32,
    /// Whether a cached second Gaussian sample is available.
    pub gaussian_spare_valid: bool,
    /// Exact Float64 bits for the cached Gaussian sample when available.
    pub gaussian_spare_hex: Option<String>,
}

/// Errors raised by invalid RNG requests or serialized state.
#[derive(Clone, Debug, PartialEq)]
pub enum RngError {
    /// The float interval had non-finite endpoints or reversed bounds.
    InvalidFloatBounds { min: f64, max: f64 },
    /// The integer upper bound was not a positive JavaScript safe integer.
    InvalidIntegerBound { max_exclusive: u64 },
    /// The uniform algorithm identifier or version was unsupported.
    UnsupportedUniformState { algorithm: String, version: u32 },
    /// The Gaussian algorithm identifier or version was unsupported.
    UnsupportedGaussianState { algorithm: String, version: u32 },
    /// A Uint32 hexadecimal field did not have the required exact form.
    InvalidUint32State { value: String },
    /// A Float64 hexadecimal field did not have the required exact form.
    InvalidFloat64State { value: String },
    /// The xorshift32 state was the forbidden all-zero state.
    ZeroXorshiftState,
    /// The Gaussian cache flag and optional cached value disagreed.
    InconsistentGaussianSpare,
    /// A decoded Gaussian spare was NaN or infinite.
    NonFiniteGaussianSpare,
}

impl fmt::Display for RngError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFloatBounds { min, max } => {
                write!(formatter, "invalid RNG float bounds: [{min}, {max})")
            }
            Self::InvalidIntegerBound { max_exclusive } => {
                write!(formatter, "invalid RNG integer bound: {max_exclusive}")
            }
            Self::UnsupportedUniformState { algorithm, version } => {
                write!(formatter, "unsupported RNG state {algorithm}@{version}")
            }
            Self::UnsupportedGaussianState { algorithm, version } => {
                write!(
                    formatter,
                    "unsupported Gaussian state {algorithm}@{version}"
                )
            }
            Self::InvalidUint32State { value } => {
                write!(formatter, "invalid Uint32 RNG state: {value}")
            }
            Self::InvalidFloat64State { value } => {
                write!(formatter, "invalid Float64 RNG state: {value}")
            }
            Self::ZeroXorshiftState => write!(formatter, "xorshift RNG state must be non-zero"),
            Self::InconsistentGaussianSpare => {
                write!(
                    formatter,
                    "Gaussian spare validity does not match its serialized value"
                )
            }
            Self::NonFiniteGaussianSpare => write!(formatter, "Gaussian spare must be finite"),
        }
    }
}

impl Error for RngError {}

/// Normalize an IEEE-754 number with JavaScript `Math.floor(value) >>> 0` semantics.
///
/// NaN and infinities normalize to zero.  Finite negative values wrap modulo
/// 2^32 after flooring, exactly like JavaScript's `ToUint32` conversion.
#[must_use]
pub fn normalize_seed(value: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }

    const UINT32_MODULUS: f64 = 4_294_967_296.0;
    let mut normalized = value.floor() % UINT32_MODULUS;
    if normalized < 0.0 {
        normalized += UINT32_MODULUS;
    }
    // The previous modulo operation constrains the value to [0, 2^32), so
    // this cast is lossless and cannot use Rust's saturating float-cast path.
    normalized as u32
}

/// Hash numeric inputs with the TypeScript-reference `hashSeed` FNV contract.
///
/// Each normalized Uint32 is XORed into the hash as one value before the
/// wrapping FNV multiplication. This intentionally differs from byte-wise
/// labelled seed derivation and is used by baseline-bot stream derivation.
#[must_use]
pub fn hash_seed(values: &[f64]) -> u32 {
    values.iter().fold(FNV_OFFSET_BASIS, |hash, value| {
        (hash ^ normalize_seed(*value)).wrapping_mul(FNV_PRIME)
    })
}

/// Derive an independent seed from a run seed and a stable stream label.
///
/// The label is explicitly iterated as UTF-16 code units to match JavaScript
/// `charCodeAt`, including surrogate pairs produced by non-BMP text.
#[must_use]
pub fn derive_seed(run_seed: f64, label: &str) -> u32 {
    derive_seed_units(run_seed, label.encode_utf16())
}

/// Derive a labelled seed from raw JavaScript-compatible UTF-16 code units.
///
/// Unlike Rust `str`, this adapter accepts isolated surrogate code units. It
/// is the explicit compatibility path for labels read from legacy JavaScript
/// state without first applying Unicode replacement.
#[must_use]
pub fn derive_seed_utf16(run_seed: f64, label: &[u16]) -> u32 {
    derive_seed_units(run_seed, label.iter().copied())
}

/// Derive a seed from an arbitrary stream of UTF-16 code units.
fn derive_seed_units(run_seed: f64, label: impl IntoIterator<Item = u16>) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    let seed = normalize_seed(run_seed);
    for byte in seed.to_le_bytes() {
        hash = fnv_mix(hash, byte);
    }
    for code_unit in label {
        hash = fnv_mix(hash, (code_unit & 0x00ff) as u8);
        hash = fnv_mix(hash, (code_unit >> 8) as u8);
    }
    hash
}

/// Construct one labelled stream without coupling it to any other stream's draws.
#[must_use]
pub fn labelled_stream(run_seed: f64, label: &str) -> StatefulRng {
    StatefulRng::new(f64::from(derive_seed(run_seed, label)))
}

/// Mix one byte using FNV-1a's checked wrapping arithmetic contract.
fn fnv_mix(hash: u32, byte: u8) -> u32 {
    (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME)
}

/// Stateful xorshift32 stream with a versioned polar Box-Muller cache.
#[derive(Clone, Debug)]
pub struct StatefulRng {
    state: u32,
    gaussian_spare: Option<f64>,
}

impl StatefulRng {
    /// Construct a stream using JavaScript-compatible seed normalization.
    #[must_use]
    pub fn new(seed: f64) -> Self {
        let state = normalize_seed(seed);
        Self {
            state: if state == 0 {
                NON_ZERO_XORSHIFT_STATE
            } else {
                state
            },
            gaussian_spare: None,
        }
    }

    /// Restore a stream after strict algorithm, version, and bit-pattern validation.
    pub fn from_state(serialized: &SerializedRngState) -> Result<Self, RngError> {
        let mut rng = Self::new(1.0);
        rng.restore_state(serialized)?;
        Ok(rng)
    }

    /// Import a legacy TypeScript/V8 continuation through an explicit compatibility seam.
    ///
    /// A cached Gaussian spare keeps its exact serialized bits. Once Rust must
    /// generate a new pair, `f64::ln` and `f64::sqrt` apply and comparison with
    /// V8 is tolerance-based rather than bit-exact. Exporting the returned RNG
    /// identifies that continuation with [`GAUSSIAN_ALGORITHM`].
    pub fn from_legacy_typescript_state(serialized: &SerializedRngState) -> Result<Self, RngError> {
        let (state, gaussian_spare) = decode_serialized_state(
            serialized,
            LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM,
            LEGACY_TYPESCRIPT_GAUSSIAN_VERSION,
        )?;
        Ok(Self {
            state,
            gaussian_spare,
        })
    }

    /// Return the next uniform sample in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        let mut value = self.state;
        value ^= value.wrapping_shl(13);
        value ^= value >> 17;
        value ^= value.wrapping_shl(5);
        self.state = value;
        f64::from(self.state) / 4_294_967_296.0
    }

    /// Return a uniformly sampled value in `[min, max)`.
    pub fn float(&mut self, min: f64, max: f64) -> Result<f64, RngError> {
        if !min.is_finite() || !max.is_finite() || max < min {
            return Err(RngError::InvalidFloatBounds { min, max });
        }
        if max == min {
            return Ok(min);
        }
        Ok(min + self.next_f64() * (max - min))
    }

    /// Return an integer uniformly sampled in `[0, max_exclusive)`.
    pub fn int(&mut self, max_exclusive: u64) -> Result<u64, RngError> {
        if max_exclusive == 0 || max_exclusive > MAX_SAFE_INTEGER {
            return Err(RngError::InvalidIntegerBound { max_exclusive });
        }
        Ok((self.next_f64() * max_exclusive as f64).floor() as u64)
    }

    /// Return a standard normal sample using the polar Box-Muller transform.
    pub fn gaussian(&mut self) -> f64 {
        if let Some(sample) = self.gaussian_spare.take() {
            return sample;
        }

        let (x, y, radius_squared) = loop {
            let x = self.next_f64() * 2.0 - 1.0;
            let y = self.next_f64() * 2.0 - 1.0;
            let radius_squared = x * x + y * y;
            if radius_squared != 0.0 && radius_squared < 1.0 {
                break (x, y, radius_squared);
            }
        };
        let multiplier = ((-2.0 * radius_squared.ln()) / radius_squared).sqrt();
        self.gaussian_spare = Some(y * multiplier);
        x * multiplier
    }

    /// Export the lossless JSON-compatible continuation state.
    #[must_use]
    pub fn export_state(&self) -> SerializedRngState {
        let mut output = SerializedRngState {
            algorithm: RNG_ALGORITHM.to_owned(),
            version: RNG_VERSION,
            state_hex: String::new(),
            gaussian_algorithm: GAUSSIAN_ALGORITHM.to_owned(),
            gaussian_version: GAUSSIAN_VERSION,
            gaussian_spare_valid: false,
            gaussian_spare_hex: None,
        };
        self.export_state_into(&mut output);
        output
    }

    /// Export into reusable serialized storage without replacing its strings.
    pub fn export_state_into(&self, output: &mut SerializedRngState) {
        let mut discarded_gaussian_spare = String::new();
        self.export_state_into_reusing(output, &mut discarded_gaussian_spare);
    }

    /// Export while retaining storage for a logically absent Gaussian spare.
    ///
    /// Authoritative hot paths use this form because Box-Muller alternates
    /// between cached and uncached states. Moving the spare string into a
    /// retained slot avoids freeing and reallocating the same fixed-size text
    /// whenever that logical option changes.
    pub(crate) fn export_state_into_reusing(
        &self,
        output: &mut SerializedRngState,
        retained_gaussian_spare: &mut String,
    ) {
        replace_text(&mut output.algorithm, RNG_ALGORITHM);
        output.version = RNG_VERSION;
        output.state_hex.clear();
        write!(&mut output.state_hex, "0x{:08x}", self.state)
            .expect("writing a Uint32 into String cannot fail");
        replace_text(&mut output.gaussian_algorithm, GAUSSIAN_ALGORITHM);
        output.gaussian_version = GAUSSIAN_VERSION;
        output.gaussian_spare_valid = self.gaussian_spare.is_some();
        match self.gaussian_spare {
            Some(value) => {
                if output.gaussian_spare_hex.is_none() {
                    output.gaussian_spare_hex = Some(std::mem::take(retained_gaussian_spare));
                }
                let encoded = output
                    .gaussian_spare_hex
                    .as_mut()
                    .expect("Gaussian spare storage was just installed");
                encoded.clear();
                write!(encoded, "0x{:016x}", value.to_bits())
                    .expect("writing Float64 bits into String cannot fail");
            }
            None => {
                if let Some(mut spare) = output.gaussian_spare_hex.take() {
                    spare.clear();
                    if spare.capacity() > retained_gaussian_spare.capacity() {
                        *retained_gaussian_spare = spare;
                    }
                }
            }
        }
    }

    /// Replace the continuation state after strict validation.
    pub fn restore_state(&mut self, serialized: &SerializedRngState) -> Result<(), RngError> {
        let (restored_state, gaussian_spare) =
            decode_serialized_state(serialized, GAUSSIAN_ALGORITHM, GAUSSIAN_VERSION)?;
        self.state = restored_state;
        self.gaussian_spare = gaussian_spare;
        Ok(())
    }
}

fn replace_text(output: &mut String, value: &str) {
    output.clear();
    output.push_str(value);
}

/// Validate and decode state without mutating a live stream.
fn decode_serialized_state(
    serialized: &SerializedRngState,
    expected_gaussian_algorithm: &str,
    expected_gaussian_version: u32,
) -> Result<(u32, Option<f64>), RngError> {
    if serialized.algorithm != RNG_ALGORITHM || serialized.version != RNG_VERSION {
        return Err(RngError::UnsupportedUniformState {
            algorithm: serialized.algorithm.clone(),
            version: serialized.version,
        });
    }
    if serialized.gaussian_algorithm != expected_gaussian_algorithm
        || serialized.gaussian_version != expected_gaussian_version
    {
        return Err(RngError::UnsupportedGaussianState {
            algorithm: serialized.gaussian_algorithm.clone(),
            version: serialized.gaussian_version,
        });
    }

    let state = decode_u32(&serialized.state_hex)?;
    if state == 0 {
        return Err(RngError::ZeroXorshiftState);
    }
    if serialized.gaussian_spare_valid != serialized.gaussian_spare_hex.is_some() {
        return Err(RngError::InconsistentGaussianSpare);
    }
    let gaussian_spare = match &serialized.gaussian_spare_hex {
        Some(value) => Some(decode_f64(value)?),
        None => None,
    };
    Ok((state, gaussian_spare))
}

/// Decode an exact Uint32 state representation.
fn decode_u32(value: &str) -> Result<u32, RngError> {
    let valid = value.len() == 10
        && matches!(value.as_bytes().get(0..2), Some(b"0x") | Some(b"0X"))
        && value.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit);
    if !valid {
        return Err(RngError::InvalidUint32State {
            value: value.to_owned(),
        });
    }
    u32::from_str_radix(&value[2..], 16).map_err(|_| RngError::InvalidUint32State {
        value: value.to_owned(),
    })
}

/// Decode and validate an exact finite Float64 bit pattern.
fn decode_f64(value: &str) -> Result<f64, RngError> {
    let valid = value.len() == 18
        && matches!(value.as_bytes().get(0..2), Some(b"0x") | Some(b"0X"))
        && value.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit);
    if !valid {
        return Err(RngError::InvalidFloat64State {
            value: value.to_owned(),
        });
    }
    let bits = u64::from_str_radix(&value[2..], 16).map_err(|_| RngError::InvalidFloat64State {
        value: value.to_owned(),
    })?;
    let decoded = f64::from_bits(bits);
    if !decoded.is_finite() {
        return Err(RngError::NonFiniteGaussianSpare);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply an invalid state and prove the failed restore was atomic.
    fn restore_error_without_mutation(
        rng: &mut StatefulRng,
        invalid: &SerializedRngState,
    ) -> RngError {
        let before = rng.export_state();
        let error = rng
            .restore_state(invalid)
            .expect_err("invalid state must be rejected");
        assert_eq!(rng.export_state(), before);
        error
    }

    /// The retained TypeScript xorshift32 v1 sequence is a cross-language fixture.
    #[test]
    fn matches_retained_xorshift32_v1_fixture() {
        let mut rng = StatefulRng::new(1.0);
        let states: Vec<String> = (0..6)
            .map(|_| {
                rng.next_f64();
                rng.export_state().state_hex
            })
            .collect();
        assert_eq!(
            states,
            [
                "0x00042021",
                "0x04080601",
                "0x9dcca8c5",
                "0x1255994f",
                "0x8ef917d1",
                "0x2c6f5bd0",
            ]
        );
    }

    #[test]
    fn repeated_serialized_export_reuses_uniform_string_capacities() {
        let mut rng = StatefulRng::new(1.0);
        let mut output = rng.export_state();
        let capacities = (
            output.algorithm.capacity(),
            output.state_hex.capacity(),
            output.gaussian_algorithm.capacity(),
        );
        for _ in 0..24 {
            rng.next_f64();
            rng.export_state_into(&mut output);
            assert_eq!(output, rng.export_state());
            assert_eq!(
                (
                    output.algorithm.capacity(),
                    output.state_hex.capacity(),
                    output.gaussian_algorithm.capacity(),
                ),
                capacities
            );
        }
    }

    #[test]
    fn reusable_export_retains_gaussian_spare_storage_across_logical_absence() {
        let mut rng = StatefulRng::new(7.0);
        let mut output = rng.export_state();
        let mut retained_spare = String::new();

        rng.gaussian();
        rng.export_state_into_reusing(&mut output, &mut retained_spare);
        assert_eq!(output, rng.export_state());
        let cached_capacity = output
            .gaussian_spare_hex
            .as_ref()
            .expect("first Gaussian leaves a cached sample")
            .capacity();

        rng.gaussian();
        rng.export_state_into_reusing(&mut output, &mut retained_spare);
        assert_eq!(output, rng.export_state());
        assert!(output.gaussian_spare_hex.is_none());
        assert!(retained_spare.capacity() >= cached_capacity);

        rng.gaussian();
        rng.export_state_into_reusing(&mut output, &mut retained_spare);
        assert_eq!(output, rng.export_state());
        assert!(
            output
                .gaussian_spare_hex
                .as_ref()
                .expect("next Gaussian restores a cached sample")
                .capacity()
                >= cached_capacity
        );
    }

    /// JavaScript seed normalization retains its floor, wrap, and non-finite rules.
    #[test]
    fn normalizes_zero_nonfinite_and_wrapped_seeds() {
        assert_eq!(normalize_seed(f64::NAN), 0);
        assert_eq!(normalize_seed(f64::INFINITY), 0);
        assert_eq!(normalize_seed(f64::NEG_INFINITY), 0);
        assert_eq!(normalize_seed(-0.0), 0);
        assert_eq!(normalize_seed(-1.0), u32::MAX);
        assert_eq!(normalize_seed(4_294_967_297.75), 1);
        assert_eq!(StatefulRng::new(0.0).export_state().state_hex, "0x00000001");
    }

    /// The baseline-bot seed combiner matches current TypeScript `hashSeed` outputs.
    #[test]
    fn matches_typescript_hash_seed_contract() {
        assert_eq!(hash_seed(&[]), 2_166_136_261);
        assert_eq!(hash_seed(&[1.0]), 67_918_732);
        assert_eq!(hash_seed(&[1.0, 2.0, 3.0]), 1_456_420_779);
        assert_eq!(
            hash_seed(&[f64::from(0xabcdef01_u32), f64::from(0x12345678_u32)]),
            2_126_984_220
        );
        assert_eq!(
            hash_seed(&[-1.0, f64::NAN, f64::INFINITY, 4_294_967_297.75,]),
            1_698_549_101
        );
    }

    /// Label derivation matches current TypeScript for text and raw UTF-16 labels.
    #[test]
    fn derives_typescript_labelled_stream_seeds_with_utf16() {
        assert_eq!(derive_seed(0xabcdef01_u32 as f64, "world"), 3_233_584_343);
        assert_eq!(derive_seed(0xabcdef01_u32 as f64, "evolution"), 804_060_948);
        assert_eq!(derive_seed(0xabcdef01_u32 as f64, "world😀"), 4_084_087_032);
        assert_eq!(
            derive_seed_utf16(
                f64::from(0xabcdef01_u32),
                &[0x0077, 0x006f, 0x0072, 0x006c, 0x0064, 0xd83d, 0xde00]
            ),
            derive_seed(f64::from(0xabcdef01_u32), "world😀")
        );
        assert_eq!(
            derive_seed_utf16(f64::from(0xabcdef01_u32), &[0xd800]),
            2_756_542_421
        );
    }

    /// A busy stream cannot perturb a stream derived from another stable label.
    #[test]
    fn labelled_streams_are_independent() {
        let mut world = labelled_stream(0xabcdef01_u32 as f64, "world");
        let mut evolution = labelled_stream(0xabcdef01_u32 as f64, "evolution");
        let initial_evolution = evolution.export_state();
        for _ in 0..100 {
            world.next_f64();
        }
        assert_eq!(evolution.export_state(), initial_evolution);
        assert_ne!(world.export_state(), evolution.export_state());
        assert_ne!(
            derive_seed(7.0, "baseline:0"),
            derive_seed(7.0, "baseline:1")
        );
        // Keep this mutable use explicit: no draw from `world` can affect it.
        evolution.next_f64();
    }

    /// A current TypeScript cached spare imports exactly through the legacy adapter.
    #[test]
    fn imports_typescript_gaussian_spare_with_exact_bits() {
        let serialized = SerializedRngState {
            algorithm: RNG_ALGORITHM.to_owned(),
            version: RNG_VERSION,
            state_hex: "0x27bf00af".to_owned(),
            gaussian_algorithm: LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM.to_owned(),
            gaussian_version: LEGACY_TYPESCRIPT_GAUSSIAN_VERSION,
            gaussian_spare_valid: true,
            gaussian_spare_hex: Some("0xbfe907a3c280a4de".to_owned()),
        };
        assert!(matches!(
            StatefulRng::from_state(&serialized),
            Err(RngError::UnsupportedGaussianState { .. })
        ));
        let mut rng = StatefulRng::from_legacy_typescript_state(&serialized)
            .expect("current TypeScript state is valid legacy input");
        assert_eq!(rng.gaussian().to_bits(), 0xbfe9_07a3_c280_a4de);
        assert_eq!(rng.next_f64().to_bits(), 0x3fc9_7d76_1d00_0000);
        let exported = rng.export_state();
        assert_eq!(exported.gaussian_algorithm, GAUSSIAN_ALGORITHM);
        assert_eq!(exported.gaussian_spare_hex, None);
    }

    /// Current TypeScript uncached output is a tolerance fixture, not an exact Rust promise.
    #[test]
    fn compares_uncached_gaussian_to_current_typescript_with_one_ulp_tolerance() {
        // Generated directly from the current `src/rng.ts` implementation for
        // seed 9. These values are not a retained Stage 2 artifact. This seed
        // deliberately exercises the observed one-ULP V8/Rust difference.
        const TYPESCRIPT_FIRST_BITS: u64 = 0xbff5_cf23_9e80_67ed;
        const TYPESCRIPT_SPARE_BITS: u64 = 0x3fe2_55dc_3e90_7857;

        let legacy_uncached = SerializedRngState {
            algorithm: RNG_ALGORITHM.to_owned(),
            version: RNG_VERSION,
            state_hex: "0x00000009".to_owned(),
            gaussian_algorithm: LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM.to_owned(),
            gaussian_version: LEGACY_TYPESCRIPT_GAUSSIAN_VERSION,
            gaussian_spare_valid: false,
            gaussian_spare_hex: None,
        };
        let mut rust_rng = StatefulRng::from_legacy_typescript_state(&legacy_uncached)
            .expect("current TypeScript state is valid legacy input");
        let rust_first_bits = rust_rng.gaussian().to_bits();
        let rust_state = rust_rng.export_state();
        let rust_spare_bits = rust_rng
            .gaussian_spare
            .expect("an uncached Gaussian call creates a spare")
            .to_bits();

        assert_eq!(rust_state.state_hex, "0x9cb75935");
        assert_eq!(rust_state.gaussian_algorithm, GAUSSIAN_ALGORITHM);
        assert!(rust_first_bits.abs_diff(TYPESCRIPT_FIRST_BITS) <= 1);
        assert!(rust_spare_bits.abs_diff(TYPESCRIPT_SPARE_BITS) <= 1);
    }

    /// Restoring an exported state preserves both ordinary and cached-Gaussian continuation.
    #[test]
    fn export_restore_continues_exactly() {
        let mut original = StatefulRng::new(0xdecafbad_u32 as f64);
        original.gaussian();
        let state = original.export_state();
        let mut restored = StatefulRng::from_state(&state).expect("exported state is valid");
        assert_eq!(restored.gaussian().to_bits(), original.gaussian().to_bits());
        assert_eq!(restored.next_f64().to_bits(), original.next_f64().to_bits());
        assert_eq!(restored.export_state(), original.export_state());
    }

    /// Invalid bounds and all malformed/version-incompatible state fail atomically.
    #[test]
    fn rejects_invalid_bounds_and_state_atomically() {
        let mut rng = StatefulRng::new(1.0);
        assert!(matches!(
            rng.float(f64::NAN, 1.0),
            Err(RngError::InvalidFloatBounds { .. })
        ));
        assert!(matches!(
            rng.float(2.0, 1.0),
            Err(RngError::InvalidFloatBounds { .. })
        ));
        assert!(matches!(
            rng.int(0),
            Err(RngError::InvalidIntegerBound { .. })
        ));
        assert!(matches!(
            rng.int(MAX_SAFE_INTEGER + 1),
            Err(RngError::InvalidIntegerBound { .. })
        ));

        let initial = rng.export_state();
        let mut invalid = initial.clone();
        invalid.version = RNG_VERSION + 1;
        assert!(matches!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::UnsupportedUniformState { .. }
        ));

        let mut invalid = initial.clone();
        invalid.gaussian_algorithm = LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM.to_owned();
        assert!(matches!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::UnsupportedGaussianState { .. }
        ));

        let mut invalid = initial.clone();
        invalid.gaussian_version = GAUSSIAN_VERSION + 1;
        assert!(matches!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::UnsupportedGaussianState { .. }
        ));

        let mut invalid = initial.clone();
        invalid.state_hex = "0x00000000".to_owned();
        assert_eq!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::ZeroXorshiftState
        );

        let mut invalid = initial.clone();
        invalid.state_hex = "0x12345678".to_owned();
        invalid.gaussian_spare_valid = true;
        assert_eq!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::InconsistentGaussianSpare
        );

        let mut invalid = initial.clone();
        invalid.state_hex = "0x12345678".to_owned();
        invalid.gaussian_spare_hex = Some("0x0000000000000000".to_owned());
        assert_eq!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::InconsistentGaussianSpare
        );

        let mut invalid = initial.clone();
        invalid.state_hex = "0x12345678".to_owned();
        invalid.gaussian_spare_valid = true;
        invalid.gaussian_spare_hex = Some("0xnot-float-bits!".to_owned());
        assert!(matches!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::InvalidFloat64State { .. }
        ));

        let mut invalid = initial.clone();
        invalid.state_hex = "0x12345678".to_owned();
        invalid.gaussian_spare_valid = true;
        invalid.gaussian_spare_hex = Some("0x7ff0000000000000".to_owned());
        assert_eq!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::NonFiniteGaussianSpare
        );

        let mut invalid = initial;
        invalid.state_hex = "0xnot-hex!".to_owned();
        assert!(matches!(
            restore_error_without_mutation(&mut rng, &invalid),
            RngError::InvalidUint32State { .. }
        ));
    }
}
