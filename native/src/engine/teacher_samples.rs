//! Portable teacher chunks. Encoding/validation belongs to Rust; Node routes bytes.
//! Construction alone does not prove a committed sample. The running publication
//! join must supply committed records; pending control rows cannot enter transport.

use sha2::{Digest, Sha256};

/// Hard wire ceilings, independent of native queue/spool retention admission.
pub const MAX_TEACHER_SAMPLES_PER_CHUNK: usize = 256;
pub const MAX_TEACHER_INPUTS: usize = 504;
pub const MAX_TEACHER_CHUNK_BYTES: usize = 1024 * 1024;
const MAGIC: &[u8; 8] = b"SLTHTCH1";
const SAMPLE_FIXED_BYTES: usize = 25;
const HEADER_FIXED_BYTES: usize = 168;

/// Why a chunk starts; capture-start is explicitly distinct from physical birth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TeacherStartReason {
    CaptureStart = 0,
    PhysicalLifeStart = 1,
    Continuation = 2,
    Gap = 3,
    GenerationStart = 4,
}

/// Ordinary size/time sealing does not reset recurrent continuity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TeacherEndReason {
    MaximumSamples = 0,
    SimulatedSecond = 1,
    LifeEnded = 2,
    GenerationEnded = 3,
    ContractChanged = 4,
    CaptureStopped = 5,
    Interrupted = 6,
}

/// Immutable capture/contract identity repeated in every self-contained chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct TeacherContractV1 {
    pub capture_epoch: [u8; 16],
    pub run_id: String,
    pub config_sha256: [u8; 32],
    pub sensor_sha256: [u8; 32],
    /// Exact teacher implementation/spec identity, distinct from its numeric revision.
    pub teacher_sha256: [u8; 32],
    pub teacher_revision: u32,
    /// 0 deterministic, 1 stochastic; behavior state remains private to the teacher.
    pub teacher_mode: u8,
    pub control_interval_seconds: f64,
    pub input_size: usize,
}

/// One exact delivered row with committed request and post-step outcomes.
#[derive(Clone, Debug, PartialEq)]
pub struct TeacherSampleV1 {
    pub step: u64,
    /// Monotonic sample sequence within this capture epoch/baseline stream.
    /// A mid-life capture starts a new sequence, not a fabricated physical birth.
    pub ordinal: u64,
    pub observation: Vec<f32>,
    pub applied_turn: f32,
    pub boost_request: bool,
    pub realized_boost: bool,
    pub post_step_death: bool,
    pub first_physical_sample: bool,
    /// 0 Roam, 1 Seek, 2 Avoid, 3 Boost.
    pub strategy: u8,
    /// 0 Small, 1 Medium, 2 Large.
    pub life_stage: u8,
    /// Bits: imminent hazard, pursuit/escape, boost, transition, saturation, respawn.
    pub strata: u16,
}

/// One physical-life/generation stream fragment; chunk sequence is assigned before admission.
#[derive(Clone, Debug, PartialEq)]
pub struct TeacherChunkV1 {
    pub contract: TeacherContractV1,
    pub chunk_sequence: u64,
    pub generation: u64,
    pub baseline_slot: u32,
    pub physical_snake_id: u64,
    pub start_reason: TeacherStartReason,
    pub end_reason: TeacherEndReason,
    pub samples: Vec<TeacherSampleV1>,
}

/// Invalid version, bounds, chronology, numeric data or encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeacherCodecError(pub &'static str);
impl std::fmt::Display for TeacherCodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for TeacherCodecError {}

impl TeacherChunkV1 {
    /// Validate complete ownership/chronology before allocating encoded transport bytes.
    pub fn validate(&self) -> Result<usize, TeacherCodecError> {
        let contract = &self.contract;
        if contract.capture_epoch == [0; 16]
            || contract.run_id.is_empty()
            || contract.run_id.len() > 128
            || contract.teacher_revision != 2
            || contract.teacher_mode > 1
            || !contract.control_interval_seconds.is_finite()
            || !(0.008..=0.06).contains(&contract.control_interval_seconds)
            || !(1..=MAX_TEACHER_INPUTS).contains(&contract.input_size)
        {
            return Err(TeacherCodecError("invalid teacher contract"));
        }
        if self.chunk_sequence == 0
            || self.generation == 0
            || self.physical_snake_id == 0
            || self.baseline_slot >= 120
            || self.samples.is_empty()
            || self.samples.len() > MAX_TEACHER_SAMPLES_PER_CHUNK
        {
            return Err(TeacherCodecError("invalid teacher chunk bounds"));
        }
        let bytes = HEADER_FIXED_BYTES
            + contract.run_id.len()
            + self.samples.len() * (SAMPLE_FIXED_BYTES + contract.input_size * 4);
        if bytes > MAX_TEACHER_CHUNK_BYTES {
            return Err(TeacherCodecError("teacher chunk byte limit"));
        }
        let mut previous: Option<&TeacherSampleV1> = None;
        for sample in &self.samples {
            if sample.step == 0
                || sample.ordinal == 0
                || sample.strategy > 3
                || sample.life_stage > 2
                || sample.strata & !63 != 0
                || !sample.applied_turn.is_finite()
                || sample.applied_turn.abs() > 1.0
                || sample.observation.len() != contract.input_size
                || sample
                    .observation
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > 1.0)
            {
                return Err(TeacherCodecError("invalid teacher sample"));
            }
            if let Some(before) = previous {
                if sample.step <= before.step
                    || before.ordinal.checked_add(1) != Some(sample.ordinal)
                    || before.post_step_death
                    || sample.first_physical_sample
                {
                    return Err(TeacherCodecError("invalid teacher sample chronology"));
                }
            }
            previous = Some(sample);
        }
        if self.start_reason == TeacherStartReason::PhysicalLifeStart
            && !self.samples[0].first_physical_sample
        {
            return Err(TeacherCodecError("physical-life start lacks first sample"));
        }
        if self.start_reason == TeacherStartReason::Continuation
            && self.samples[0].first_physical_sample
        {
            return Err(TeacherCodecError(
                "continuation cannot fabricate physical birth",
            ));
        }
        let last = self.samples.last().expect("nonempty validated above");
        if last.step - self.samples[0].step > 60 {
            return Err(TeacherCodecError(
                "teacher chunk exceeds one simulated second",
            ));
        }
        Ok(bytes)
    }

    /// Encode finite Float32 values bit-for-bit in little-endian; no population/model payloads.
    pub fn encode(&self) -> Result<Vec<u8>, TeacherCodecError> {
        let expected = self.validate()?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(expected)
            .map_err(|_| TeacherCodecError("teacher encoding allocation"))?;
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.contract.capture_epoch);
        bytes.push(self.contract.run_id.len() as u8);
        bytes.extend_from_slice(self.contract.run_id.as_bytes());
        bytes.extend_from_slice(&self.contract.config_sha256);
        bytes.extend_from_slice(&self.contract.sensor_sha256);
        bytes.extend_from_slice(&self.contract.teacher_sha256);
        bytes.extend_from_slice(&self.contract.teacher_revision.to_le_bytes());
        bytes.push(self.contract.teacher_mode);
        bytes.extend_from_slice(&self.contract.control_interval_seconds.to_le_bytes());
        bytes.extend_from_slice(&(self.contract.input_size as u16).to_le_bytes());
        bytes.extend_from_slice(&self.chunk_sequence.to_le_bytes());
        bytes.extend_from_slice(&self.generation.to_le_bytes());
        bytes.extend_from_slice(&self.baseline_slot.to_le_bytes());
        bytes.extend_from_slice(&self.physical_snake_id.to_le_bytes());
        bytes.push(self.start_reason as u8);
        bytes.push(self.end_reason as u8);
        bytes.extend_from_slice(&(self.samples.len() as u16).to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.step.to_le_bytes());
            bytes.extend_from_slice(&sample.ordinal.to_le_bytes());
            bytes.extend_from_slice(&sample.applied_turn.to_le_bytes());
            bytes.push(
                u8::from(sample.boost_request)
                    | (u8::from(sample.realized_boost) << 1)
                    | (u8::from(sample.post_step_death) << 2)
                    | (u8::from(sample.first_physical_sample) << 3),
            );
            bytes.push(sample.strategy);
            bytes.push(sample.life_stage);
            bytes.extend_from_slice(&sample.strata.to_le_bytes());
            for value in &sample.observation {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        if bytes.len() != expected {
            return Err(TeacherCodecError("teacher encoding size mismatch"));
        }
        Ok(bytes)
    }

    /// Bound the complete payload before decoding or reserving observation storage.
    pub fn decode(bytes: &[u8]) -> Result<Self, TeacherCodecError> {
        if bytes.len() < HEADER_FIXED_BYTES || bytes.len() > MAX_TEACHER_CHUNK_BYTES {
            return Err(TeacherCodecError("teacher payload byte limit"));
        }
        let mut input = Reader { bytes, cursor: 0 };
        if &input.array::<8>()? != MAGIC {
            return Err(TeacherCodecError("unsupported teacher chunk version"));
        }
        let capture_epoch = input.array()?;
        let run_size = usize::from(input.byte()?);
        if !(1..=128).contains(&run_size) {
            return Err(TeacherCodecError("teacher run identifier limit"));
        }
        let run_id = std::str::from_utf8(input.take(run_size)?)
            .map_err(|_| TeacherCodecError("teacher run UTF-8"))?
            .to_owned();
        let contract = TeacherContractV1 {
            capture_epoch,
            run_id,
            config_sha256: input.array()?,
            sensor_sha256: input.array()?,
            teacher_sha256: input.array()?,
            teacher_revision: u32::from_le_bytes(input.array()?),
            teacher_mode: input.byte()?,
            control_interval_seconds: f64::from_le_bytes(input.array()?),
            input_size: usize::from(u16::from_le_bytes(input.array()?)),
        };
        let chunk_sequence = u64::from_le_bytes(input.array()?);
        let generation = u64::from_le_bytes(input.array()?);
        let baseline_slot = u32::from_le_bytes(input.array()?);
        let physical_snake_id = u64::from_le_bytes(input.array()?);
        let start_reason = match input.byte()? {
            0 => TeacherStartReason::CaptureStart,
            1 => TeacherStartReason::PhysicalLifeStart,
            2 => TeacherStartReason::Continuation,
            3 => TeacherStartReason::Gap,
            4 => TeacherStartReason::GenerationStart,
            _ => return Err(TeacherCodecError("teacher start reason")),
        };
        let end_reason = match input.byte()? {
            0 => TeacherEndReason::MaximumSamples,
            1 => TeacherEndReason::SimulatedSecond,
            2 => TeacherEndReason::LifeEnded,
            3 => TeacherEndReason::GenerationEnded,
            4 => TeacherEndReason::ContractChanged,
            5 => TeacherEndReason::CaptureStopped,
            6 => TeacherEndReason::Interrupted,
            _ => return Err(TeacherCodecError("teacher end reason")),
        };
        let count = usize::from(u16::from_le_bytes(input.array()?));
        if !(1..=MAX_TEACHER_SAMPLES_PER_CHUNK).contains(&count)
            || !(1..=MAX_TEACHER_INPUTS).contains(&contract.input_size)
            || input.bytes.len() - input.cursor
                != count * (SAMPLE_FIXED_BYTES + contract.input_size * 4)
        {
            return Err(TeacherCodecError(
                "teacher payload dimensions or trailing bytes",
            ));
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(count)
            .map_err(|_| TeacherCodecError("teacher sample allocation"))?;
        for _ in 0..count {
            let step = u64::from_le_bytes(input.array()?);
            let ordinal = u64::from_le_bytes(input.array()?);
            let applied_turn = f32::from_bits(u32::from_le_bytes(input.array()?));
            let flags = input.byte()?;
            if flags & !15 != 0 {
                return Err(TeacherCodecError("teacher sample reserved flags"));
            }
            let strategy = input.byte()?;
            let life_stage = input.byte()?;
            let strata = u16::from_le_bytes(input.array()?);
            let mut observation = Vec::new();
            observation
                .try_reserve_exact(contract.input_size)
                .map_err(|_| TeacherCodecError("teacher row allocation"))?;
            for _ in 0..contract.input_size {
                observation.push(f32::from_bits(u32::from_le_bytes(input.array()?)));
            }
            samples.push(TeacherSampleV1 {
                step,
                ordinal,
                applied_turn,
                observation,
                boost_request: flags & 1 != 0,
                realized_boost: flags & 2 != 0,
                post_step_death: flags & 4 != 0,
                first_physical_sample: flags & 8 != 0,
                strategy,
                life_stage,
                strata,
            });
        }
        let result = Self {
            contract,
            chunk_sequence,
            generation,
            baseline_slot,
            physical_snake_id,
            start_reason,
            end_reason,
            samples,
        };
        result.validate()?;
        Ok(result)
    }

    /// Digest of the exact opaque bytes published to storage and immutable download.
    pub fn encoded_sha256(&self) -> Result<[u8; 32], TeacherCodecError> {
        Ok(Sha256::digest(self.encode()?).into())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], TeacherCodecError> {
        let end = self
            .cursor
            .checked_add(size)
            .ok_or(TeacherCodecError("teacher cursor overflow"))?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(TeacherCodecError("truncated teacher payload"))?;
        self.cursor = end;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], TeacherCodecError> {
        self.take(N)?
            .try_into()
            .map_err(|_| TeacherCodecError("teacher fixed field length"))
    }
    fn byte(&mut self) -> Result<u8, TeacherCodecError> {
        Ok(self.array::<1>()?[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk() -> TeacherChunkV1 {
        TeacherChunkV1 {
            contract: TeacherContractV1 {
                capture_epoch: [17; 16],
                run_id: "run-α".into(),
                config_sha256: [23; 32],
                sensor_sha256: [31; 32],
                teacher_sha256: [37; 32],
                teacher_revision: 2,
                teacher_mode: 1,
                control_interval_seconds: 0.03,
                input_size: 72,
            },
            chunk_sequence: 7,
            generation: u64::MAX,
            baseline_slot: 119,
            physical_snake_id: u64::MAX,
            start_reason: TeacherStartReason::CaptureStart,
            end_reason: TeacherEndReason::SimulatedSecond,
            samples: vec![TeacherSampleV1 {
                step: 42,
                ordinal: 1,
                observation: vec![-0.0; 72],
                applied_turn: -0.0,
                boost_request: true,
                realized_boost: false,
                post_step_death: false,
                first_physical_sample: false,
                strategy: 3,
                life_stage: 2,
                strata: 63,
            }],
        }
    }

    #[test]
    fn exact_float_bits_identity_and_request_outcome_roundtrip() {
        let source = chunk();
        let bytes = source.encode().unwrap();
        assert_eq!(
            bytes.len(),
            HEADER_FIXED_BYTES + source.contract.run_id.len() + SAMPLE_FIXED_BYTES + 72 * 4
        );
        let decoded = TeacherChunkV1::decode(&bytes).unwrap();
        assert_eq!(decoded, source);
        assert_eq!(
            decoded.samples[0].applied_turn.to_bits(),
            (-0.0_f32).to_bits()
        );
        assert!(decoded.samples[0]
            .observation
            .iter()
            .all(|value| value.to_bits() == (-0.0_f32).to_bits()));
        assert!(decoded.samples[0].boost_request);
        assert!(!decoded.samples[0].realized_boost);
        assert_eq!(decoded.encode().unwrap(), bytes);
        assert_eq!(
            source.encoded_sha256().unwrap(),
            <[u8; 32]>::from(Sha256::digest(bytes))
        );
    }

    #[test]
    fn every_truncated_prefix_and_appended_bytes_reject() {
        let bytes = chunk().encode().unwrap();
        for length in 0..bytes.len() {
            assert!(
                TeacherChunkV1::decode(&bytes[..length]).is_err(),
                "length {length}"
            );
        }
        let mut appended = bytes;
        appended.push(0);
        assert!(TeacherChunkV1::decode(&appended).is_err());
    }

    #[test]
    fn unsupported_revisions_dimensions_flags_and_reason_bytes_reject() {
        let source = chunk();
        let bytes = source.encode().unwrap();
        let run_len = source.contract.run_id.len();
        for offset in [
            0,
            121 + run_len,
            134 + run_len,
            164 + run_len,
            165 + run_len,
            166 + run_len,
            HEADER_FIXED_BYTES + run_len + 20,
        ] {
            let mut invalid = bytes.clone();
            invalid[offset] = 255;
            assert!(TeacherChunkV1::decode(&invalid).is_err(), "offset {offset}");
        }
        assert!(TeacherChunkV1::decode(&vec![0; MAX_TEACHER_CHUNK_BYTES + 1]).is_err());
    }

    #[test]
    fn sampled_cadence_chronology_and_physical_death_end_are_explicit() {
        let mut source = chunk();
        let first = source.samples[0].clone();
        for index in 1..20 {
            let mut sample = first.clone();
            sample.step += index * 3;
            sample.ordinal += index;
            source.samples.push(sample);
        }
        assert!(source.encode().is_ok());
        source.samples[5].ordinal += 1;
        assert!(source.encode().is_err());
        source.samples[5].ordinal -= 1;
        source.samples[5].post_step_death = true;
        assert!(source.encode().is_err());
        source.samples[5].post_step_death = false;
        source.samples.last_mut().unwrap().post_step_death = true;
        source.end_reason = TeacherEndReason::LifeEnded;
        assert!(source.encode().is_ok());
        source.samples.last_mut().unwrap().step += 5;
        assert!(source.encode().is_err());
    }

    #[test]
    fn capture_start_never_implies_a_physical_first_delivery() {
        let mut source = chunk();
        assert!(source.encode().is_ok());
        source.start_reason = TeacherStartReason::PhysicalLifeStart;
        assert!(source.encode().is_err());
        source.samples[0].first_physical_sample = true;
        assert!(source.encode().is_ok());
        source.start_reason = TeacherStartReason::Continuation;
        assert!(source.encode().is_err());
    }

    #[test]
    fn cold_life_end_does_not_rewrite_a_previous_post_step_outcome() {
        let mut source = chunk();
        source.end_reason = TeacherEndReason::LifeEnded;
        let decoded = TeacherChunkV1::decode(&source.encode().unwrap()).unwrap();
        assert!(!decoded.samples[0].post_step_death);
        assert_eq!(decoded.end_reason, TeacherEndReason::LifeEnded);
    }

    #[test]
    fn finite_bounded_data_and_dense_baseline_owner_limits_reject() {
        let mut source = chunk();
        source.samples[0].observation[0] = f32::NAN;
        assert!(source.encode().is_err());
        source.samples[0].observation[0] = 1.0001;
        assert!(source.encode().is_err());
        source.samples[0].observation[0] = 1.0;
        source.baseline_slot = 120;
        assert!(source.encode().is_err());
        source.baseline_slot = 0;
        source.contract.input_size = MAX_TEACHER_INPUTS + 1;
        assert!(source.encode().is_err());
        source.contract.input_size = 72;
        source
            .samples
            .resize(MAX_TEACHER_SAMPLES_PER_CHUNK + 1, source.samples[0].clone());
        assert!(source.encode().is_err());
    }
}
