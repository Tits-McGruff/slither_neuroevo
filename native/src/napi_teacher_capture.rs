//! Coarse diagnostic metadata and opaque bytes; no sample/model execution in JS.

use crate::engine::teacher_mailbox::TeacherMailboxDrain;
use crate::engine::teacher_runtime::{TeacherCaptureLimits, TeacherCaptureRequest};
use crate::engine::teacher_samples::{TeacherEndReason, TeacherStartReason};
use crate::napi_engine::{bounded_object_string, parse_u64_hex, positive_usize, u64_hex};
use napi::bindgen_prelude::{Array, Buffer, JsObjectValue, Object};
use napi::{Error, JsString, Result, Status};
use napi_derive::napi;

/// Immutable identity repeated by the Rust binary format.
#[napi(object)]
pub struct TeacherCaptureContract {
    pub capture_epoch: String,
    pub run_id: String,
    pub config_sha256: String,
    pub sensor_sha256: String,
    pub teacher_sha256: String,
    pub teacher_revision: u32,
    pub teacher_mode: u32,
    pub control_interval_seconds: f64,
    pub input_size: u32,
}

/// Rust-built bounded descriptor; all long counters retain exact hexadecimal words.
#[napi(object)]
pub struct TeacherChunkDescriptor {
    pub capture_epoch: String,
    pub sequence: String,
    pub generation: String,
    pub baseline_slot: u32,
    pub physical_snake_id: String,
    pub first_sample: String,
    pub last_sample: String,
    pub first_step: String,
    pub last_step: String,
    pub count: u32,
    pub input_size: u32,
    pub config_sha256: String,
    pub sensor_sha256: String,
    pub teacher_sha256: String,
    pub start_reason: String,
    pub end_reason: String,
    pub byte_count: u32,
    pub sha256: String,
}

/// Independently owned immutable bytes and their descriptor.
#[napi(object)]
pub struct TeacherChunkTransfer {
    pub descriptor: TeacherChunkDescriptor,
    pub bytes: Buffer,
}

/// Coherent native handoff progress; releasedThrough is not spool durability.
#[napi(object)]
pub struct TeacherCaptureDrain {
    pub contract: Option<TeacherCaptureContract>,
    pub sensor_contract_json: Option<String>,
    pub chunks: Vec<TeacherChunkTransfer>,
    pub production_watermark: String,
    pub released_through: String,
    pub queued_chunks: u32,
    pub queued_bytes: String,
    pub dropped_chunks: String,
    pub dropped_samples: String,
    pub dropped_bytes: String,
    pub stopped: bool,
    pub error: Option<String>,
    pub more_work: bool,
}

/// Lowercase bytes preserve structural identities independently of number conversion.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Validate an epoch before copying its fixed-width native ownership token.
pub(crate) fn parse_epoch(value: &str) -> Result<[u8; 16]> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::new(
            Status::InvalidArg,
            "capture epoch must be 32 lowercase hexadecimal digits",
        ));
    }
    let mut result = [0; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::new(Status::InvalidArg, "invalid capture epoch"))?;
    }
    if result == [0; 16] {
        return Err(Error::new(
            Status::InvalidArg,
            "capture epoch cannot be zero",
        ));
    }
    Ok(result)
}

/// Strict own fields and bounded slot count precede queue/storage allocation.
pub(crate) fn parse_request(request: &Object<'_>) -> Result<TeacherCaptureRequest> {
    const KEYS: [&str; 7] = [
        "captureEpoch",
        "expectedRunId",
        "expectedConfigRevision",
        "baselineSlots",
        "pendingBytes",
        "queueChunks",
        "queueBytes",
    ];
    let names = request.get_property_names()?;
    if names.get_array_length()? != KEYS.len() as u32 {
        return Err(Error::new(
            Status::InvalidArg,
            "teacher capture request has unknown/missing fields",
        ));
    }
    for index in 0..names.get_array_length()? {
        let key = names.get_element::<JsString<'_>>(index)?;
        let key = crate::napi_engine::bounded_js_string(key, "teacher request key", 32, false)?;
        if !KEYS.contains(&key.as_str()) || !request.has_own_property(&key)? {
            return Err(Error::new(
                Status::InvalidArg,
                "teacher capture request requires exact own fields",
            ));
        }
    }
    let slots = request
        .get::<Array<'_>>("baselineSlots")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "teacher capture slots missing"))?;
    if slots.len() == 0 || slots.len() > 120 {
        return Err(Error::new(
            Status::InvalidArg,
            "teacher capture requires 1 to 120 slots",
        ));
    }
    let mut baseline_slots = Vec::new();
    baseline_slots
        .try_reserve_exact(slots.len() as usize)
        .map_err(|_| Error::new(Status::GenericFailure, "teacher capture slot allocation"))?;
    for index in 0..slots.len() {
        let slot = slots
            .get::<f64>(index)?
            .ok_or_else(|| Error::new(Status::InvalidArg, "teacher slot missing"))?;
        if !slot.is_finite() || slot.fract() != 0.0 || !(0.0..120.0).contains(&slot) {
            return Err(Error::new(
                Status::InvalidArg,
                "teacher slot must be an integer from 0 to 119",
            ));
        }
        baseline_slots.push(slot as u32);
    }
    let size = |key| -> Result<usize> {
        let value = request
            .get::<f64>(key)?
            .ok_or_else(|| Error::new(Status::InvalidArg, "teacher budget missing"))?;
        positive_usize(value, key).map_err(crate::napi_engine::engine_error_to_napi)
    };
    let result = TeacherCaptureRequest {
        capture_epoch: parse_epoch(&bounded_object_string(request, "captureEpoch", 32)?)?,
        expected_run_id: bounded_object_string(request, "expectedRunId", 128)?,
        expected_config_revision: parse_u64_hex(
            &bounded_object_string(request, "expectedConfigRevision", 16)?,
            "expectedConfigRevision",
            false,
        )?,
        baseline_slots: baseline_slots.into_boxed_slice(),
        limits: TeacherCaptureLimits {
            pending_bytes: size("pendingBytes")?,
            queue_chunks: size("queueChunks")?,
            queue_bytes: size("queueBytes")?,
        },
    };
    result
        .validate()
        .map_err(|detail| Error::new(Status::InvalidArg, detail))?;
    Ok(result)
}

/// Convert already bounded native objects after leaving the handoff mutex.
pub(crate) fn drain_to_napi(drain: TeacherMailboxDrain) -> TeacherCaptureDrain {
    let contract = drain.contract.map(|contract| TeacherCaptureContract {
        capture_epoch: hex(&contract.capture_epoch),
        run_id: contract.run_id,
        config_sha256: hex(&contract.config_sha256),
        sensor_sha256: hex(&contract.sensor_sha256),
        teacher_sha256: hex(&contract.teacher_sha256),
        teacher_revision: contract.teacher_revision,
        teacher_mode: u32::from(contract.teacher_mode),
        control_interval_seconds: contract.control_interval_seconds,
        input_size: contract.input_size as u32,
    });
    let chunks = drain
        .chunks
        .into_iter()
        .map(|chunk| {
            let d = chunk.descriptor;
            TeacherChunkTransfer {
                descriptor: TeacherChunkDescriptor {
                    capture_epoch: hex(&d.capture_epoch),
                    sequence: u64_hex(d.sequence),
                    generation: u64_hex(d.generation),
                    baseline_slot: d.baseline_slot,
                    physical_snake_id: u64_hex(d.physical_snake_id),
                    first_sample: u64_hex(d.first_sample),
                    last_sample: u64_hex(d.last_sample),
                    first_step: u64_hex(d.first_step),
                    last_step: u64_hex(d.last_step),
                    count: d.count as u32,
                    input_size: d.input_size as u32,
                    config_sha256: hex(&d.config_sha256),
                    sensor_sha256: hex(&d.sensor_sha256),
                    teacher_sha256: hex(&d.teacher_sha256),
                    start_reason: match d.start_reason {
                        TeacherStartReason::CaptureStart => "captureStart",
                        TeacherStartReason::PhysicalLifeStart => "physicalLifeStart",
                        TeacherStartReason::Continuation => "continuation",
                        TeacherStartReason::Gap => "gap",
                        TeacherStartReason::GenerationStart => "generationStart",
                    }
                    .into(),
                    end_reason: match d.end_reason {
                        TeacherEndReason::MaximumSamples => "maximumSamples",
                        TeacherEndReason::SimulatedSecond => "simulatedSecond",
                        TeacherEndReason::LifeEnded => "lifeEnded",
                        TeacherEndReason::GenerationEnded => "generationEnded",
                        TeacherEndReason::ContractChanged => "contractChanged",
                        TeacherEndReason::CaptureStopped => "captureStopped",
                        TeacherEndReason::Interrupted => "interrupted",
                    }
                    .into(),
                    byte_count: d.byte_count as u32,
                    sha256: hex(&d.sha256),
                },
                bytes: chunk.bytes.into(),
            }
        })
        .collect();
    let s = drain.snapshot;
    TeacherCaptureDrain {
        contract,
        sensor_contract_json: drain.sensor_contract_json,
        chunks,
        production_watermark: u64_hex(s.production_watermark),
        released_through: u64_hex(s.released_through),
        queued_chunks: s.queued_chunks as u32,
        queued_bytes: u64_hex(s.queued_bytes as u64),
        dropped_chunks: u64_hex(s.dropped_chunks),
        dropped_samples: u64_hex(s.dropped_samples),
        dropped_bytes: u64_hex(s.dropped_bytes),
        stopped: drain.stopped,
        error: drain.error,
        more_work: drain.more_work,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epoch_parser_rejects_zero_unicode_uppercase_and_wrong_width() {
        assert_eq!(parse_epoch(&"12".repeat(16)).unwrap(), [18; 16]);
        for invalid in [
            "00".repeat(16),
            "AB".repeat(16),
            "é".repeat(16),
            "12".repeat(15),
            "12".repeat(17),
        ] {
            assert!(parse_epoch(&invalid).is_err());
        }
    }
}
