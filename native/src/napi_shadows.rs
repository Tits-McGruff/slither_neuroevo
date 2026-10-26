//! Bounded cold shadow commands and fixed diagnostic metrics, never model execution in JS.
use crate::engine::shadows::{
    registered_candidate, ShadowActivationRequest, ShadowMetricTotals, ShadowSnapshot, MAX_SHADOWS,
};
use crate::napi_engine::{bounded_js_string, bounded_object_string, parse_u64_hex, u64_hex};
use napi::bindgen_prelude::{JsObjectValue, Object};
use napi::{Error, JsString, Result, Status};
use serde_json::{json, Value};

/// Require precisely the bounded own-property command envelope before field allocation.
fn fields(request: &Object<'_>, keys: &[&str]) -> Result<()> {
    let names = request.get_property_names()?;
    if names.get_array_length()? != keys.len() as u32 {
        return Err(Error::new(
            Status::InvalidArg,
            "shadow request has missing/unknown fields",
        ));
    }
    for index in 0..names.get_array_length()? {
        let name = bounded_js_string(
            names.get_element::<JsString<'_>>(index)?,
            "shadow key",
            32,
            false,
        )?;
        if !keys.contains(&name.as_str()) || !request.has_own_property(&name)? {
            return Err(Error::new(
                Status::InvalidArg,
                "shadow request requires exact own fields",
            ));
        }
    }
    Ok(())
}
fn slot(request: &Object<'_>, key: &str, limit: usize) -> Result<u32> {
    let value = request
        .get::<f64>(key)?
        .ok_or_else(|| Error::new(Status::InvalidArg, "missing shadow slot"))?;
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value >= limit as f64 {
        return Err(Error::new(
            Status::InvalidArg,
            "invalid shadow/teacher slot",
        ));
    }
    Ok(value as u32)
}
/// Resolve an already cold-validated immutable native candidate; no file/world work on admission.
pub(crate) fn parse_activation(request: &Object<'_>) -> Result<ShadowActivationRequest> {
    fields(
        request,
        &[
            "expectedRunId",
            "expectedConfigRevision",
            "captureEpoch",
            "shadowSlot",
            "baselineSlot",
            "artifactSha256",
        ],
    )?;
    let identity = bounded_object_string(request, "artifactSha256", 64)?;
    let identity = crate::engine::state::decode_sha256_identity(
        "shadow.artifact",
        &format!("sha256:{identity}"),
    )
    .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
    let result = ShadowActivationRequest {
        expected_run_id: bounded_object_string(request, "expectedRunId", 128)?,
        expected_config_revision: parse_u64_hex(
            &bounded_object_string(request, "expectedConfigRevision", 16)?,
            "expectedConfigRevision",
            false,
        )?,
        capture_epoch: crate::napi_teacher_capture::parse_epoch(&bounded_object_string(
            request,
            "captureEpoch",
            32,
        )?)?,
        shadow_slot: slot(request, "shadowSlot", MAX_SHADOWS)?,
        baseline_slot: slot(request, "baselineSlot", 120)?,
        candidate: registered_candidate(identity)
            .map_err(|detail| Error::new(Status::InvalidArg, detail))?,
    };
    result
        .validate()
        .map_err(|detail| Error::new(Status::InvalidArg, detail))?;
    Ok(result)
}
/// Stop binds the activation sequence so a delayed request cannot disable its replacement.
pub(crate) fn parse_stop(
    request: &Object<'_>,
) -> Result<crate::engine::contract::RunningAuthorityCommand> {
    fields(
        request,
        &["shadowSlot", "captureEpoch", "activationSequence"],
    )?;
    Ok(
        crate::engine::contract::RunningAuthorityCommand::StopShadow {
            shadow_slot: slot(request, "shadowSlot", MAX_SHADOWS)?,
            capture_epoch: crate::napi_teacher_capture::parse_epoch(&bounded_object_string(
                request,
                "captureEpoch",
                32,
            )?)?,
            activation_sequence: parse_u64_hex(
                &bounded_object_string(request, "activationSequence", 16)?,
                "activationSequence",
                false,
            )?,
        },
    )
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn totals(value: ShadowMetricTotals) -> Value {
    let count = value.samples as f64;
    json!({"samples":u64_hex(value.samples), "turnAbsoluteError":value.turn_absolute_error,
        "turnSquaredError":value.turn_squared_error, "boostDisagreements":u64_hex(value.boost_disagreements),
        "turnMae":if count > 0.0 { value.turn_absolute_error / count } else { 0.0 },
        "turnMse":if count > 0.0 { value.turn_squared_error / count } else { 0.0 },
        "boostDisagreementRate":if count > 0.0 { value.boost_disagreements as f64 / count } else { 0.0 }})
}
/// Four fixed summaries are serialized cold; u64 identities and counts remain exact hexadecimal.
pub(crate) fn metrics_json(snapshots: [Option<ShadowSnapshot>; MAX_SHADOWS]) -> Result<String> {
    let slots: [Value; MAX_SHADOWS] = std::array::from_fn(|slot| {
        let Some(snapshot) = snapshots[slot] else {
            return Value::Null;
        };
        json!({"shadowSlot":slot, "artifactSha256":hex(&snapshot.artifact_sha256),
            "captureEpoch":hex(&snapshot.capture_epoch), "baselineSlot":snapshot.baseline_slot,
            "activationSequence":u64_hex(snapshot.activation_sequence), "activationStep":u64_hex(snapshot.activation_step),
            "completedStep":u64_hex(snapshot.completed_step), "lastSampleStep":u64_hex(snapshot.last_sample_step), "generation":u64_hex(snapshot.generation),
            "physicalSnakeId":u64_hex(snapshot.physical_snake_id), "enabled":snapshot.enabled, "error":snapshot.error,
            "rawOutputs":snapshot.raw_outputs, "totals":totals(snapshot.totals),
            "strategies":snapshot.strategies.map(totals), "lifeStages":snapshot.life_stages.map(totals),
            "strata":snapshot.strata.map(totals), "cruising":totals(snapshot.cruising),
            "preDeath":totals(snapshot.pre_death), "deaths":u64_hex(snapshot.deaths)})
    });
    serde_json::to_string(&json!({"schemaVersion":1, "slots":slots}))
        .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
}
