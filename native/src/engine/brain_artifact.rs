//! Bounded portable neural artifacts; Rust recompiles every submitted graph/packing contract.

use super::checkpoint::{decode_adaptive_numeric_reader, NumericEncoding};
use super::genome::GenomeInitializationConfig;
use super::graph::GraphBundle;
use super::graph_json::compile_training_graph_json;
use super::sensor_contract::SensorProfile;
use super::sensor_layout::SensorLayout;
use super::training_graph::{
    CanonicalTrainingGraphManifestV1, TrainingDeployment, TrainingLifeStateRule,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Cursor;

/// Full portable file admission, independent of population checkpoint sizes.
pub const MAX_BRAIN_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
/// Header: magic, JSON bytes, encoding/reserved, parameter count, stored bytes, raw/stored digests.
pub const BRAIN_ARTIFACT_HEADER_BYTES: usize = 88;
const MAGIC: &[u8; 8] = b"SLTHBRN1";
const MAX_PROVENANCE_BYTES: usize = 64 * 1024;

/// Provenance class is metadata; it never introduces another runtime snake kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BrainSourceKind {
    Distilled,
    Imported,
    Manual,
}

/// Small immutable user/trainer provenance, distinct from Hall-of-Fame winning status.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrainProvenanceV1 {
    pub schema_version: u32,
    pub source_kind: BrainSourceKind,
    pub details: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrainManifestV1 {
    schema_version: u32,
    graph_manifest: Value,
    provenance: BrainProvenanceV1,
}

/// Cold admitted values; live spawn/activation must additionally match the active authority.
pub struct ValidatedBrainArtifact {
    pub graph: GraphBundle,
    pub sensors: SensorLayout,
    pub deployment: TrainingDeployment,
    pub weights: Vec<f32>,
    pub canonical_manifest: CanonicalTrainingGraphManifestV1,
    pub provenance: BrainProvenanceV1,
    pub artifact_sha256: String,
    pub weights_sha256: String,
}

/// Bounded public metadata returned after complete independent native validation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainArtifactDescriptorV1 {
    pub schema_version: u32,
    pub artifact_sha256: String,
    pub weights_sha256: String,
    pub architecture_key: String,
    pub layout_sha256: String,
    pub sensor_sha256: String,
    pub input_size: usize,
    pub parameter_count: usize,
    pub state_size: usize,
    pub control_interval_seconds: f64,
    pub life_state_rule: TrainingLifeStateRule,
    pub source_kind: BrainSourceKind,
}

impl ValidatedBrainArtifact {
    pub fn descriptor(&self) -> BrainArtifactDescriptorV1 {
        BrainArtifactDescriptorV1 {
            schema_version: 1,
            artifact_sha256: self.artifact_sha256.clone(),
            weights_sha256: self.weights_sha256.clone(),
            architecture_key: self.canonical_manifest.architecture_key.clone(),
            layout_sha256: self.canonical_manifest.layout_sha256.clone(),
            sensor_sha256: self.canonical_manifest.sensor_sha256.clone(),
            input_size: self.canonical_manifest.input_size,
            parameter_count: self.weights.len(),
            state_size: self.canonical_manifest.total_state_size,
            control_interval_seconds: self.deployment.control_interval_seconds,
            life_state_rule: self.deployment.life_state_rule,
            source_kind: self.provenance.source_kind,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn field<'a>(root: &'a Value, name: &str) -> Result<&'a Value, String> {
    root.get(name)
        .ok_or_else(|| format!("brain manifest omitted {name}"))
}

fn number(root: &Value, name: &str) -> Result<f64, String> {
    field(root, name)?
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("invalid brain numeric field {name}"))
}

fn bounded_provenance(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > 16 || *nodes > 4096 {
        return Err("brain provenance shape admission".into());
    }
    match value {
        Value::String(text) => {
            if text.len() > 4096 || text.contains('\0') {
                return Err("brain provenance text admission".into());
            }
        }
        Value::Array(values) => {
            for value in values {
                bounded_provenance(value, depth + 1, nodes)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > 128 || key.contains('\0') {
                    return Err("brain provenance key admission".into());
                }
                bounded_provenance(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate the complete envelope, compile source, rebuild packing/semantics, then admit finite weights.
pub fn validate_brain_artifact(bytes: &[u8]) -> Result<ValidatedBrainArtifact, String> {
    if !(BRAIN_ARTIFACT_HEADER_BYTES..=MAX_BRAIN_ARTIFACT_BYTES).contains(&bytes.len())
        || &bytes[..8] != MAGIC
    {
        return Err("brain artifact byte/version admission".into());
    }
    let read_u32 = |offset| {
        u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("checked header"),
        ) as usize
    };
    let manifest_bytes = read_u32(8);
    let parameter_count = read_u32(16);
    let stored_bytes = read_u32(20);
    if bytes[13..16] != [0, 0, 0] || manifest_bytes > 1024 * 1024 || parameter_count > 1_000_000 {
        return Err("brain artifact header admission".into());
    }
    let encoding = match bytes[12] {
        0 => NumericEncoding::RawF32LeV1,
        1 => NumericEncoding::F32LeShuffle4ZstdV1,
        _ => return Err("unsupported brain numeric encoding".into()),
    };
    let end = BRAIN_ARTIFACT_HEADER_BYTES
        .checked_add(manifest_bytes)
        .and_then(|value| value.checked_add(stored_bytes))
        .ok_or("brain envelope length overflow")?;
    if end != bytes.len() {
        return Err("brain envelope dimensions/trailing data mismatch".into());
    }
    let raw_digest: [u8; 32] = bytes[24..56].try_into().expect("checked header");
    let stored_digest: [u8; 32] = bytes[56..88].try_into().expect("checked header");
    let payload = &bytes[BRAIN_ARTIFACT_HEADER_BYTES + manifest_bytes..];
    if Sha256::digest(payload).as_slice() != stored_digest {
        return Err("brain stored digest mismatch".into());
    }
    let manifest: BrainManifestV1 = serde_json::from_slice(
        &bytes[BRAIN_ARTIFACT_HEADER_BYTES..BRAIN_ARTIFACT_HEADER_BYTES + manifest_bytes],
    )
    .map_err(|error| format!("invalid brain manifest: {error}"))?;
    if manifest.schema_version != 1 || manifest.provenance.schema_version != 1 {
        return Err("unsupported brain manifest/provenance revision".into());
    }
    if !manifest.provenance.details.is_object()
        || serde_json::to_vec(&manifest.provenance)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_PROVENANCE_BYTES
    {
        return Err("brain provenance byte/object admission".into());
    }
    bounded_provenance(&manifest.provenance.details, 0, &mut 0)?;
    let source = field(&manifest.graph_manifest, "graphSpec")?;
    let graph =
        compile_training_graph_json(&serde_json::to_vec(source).map_err(|e| e.to_string())?)?;
    let sensor_text = field(&manifest.graph_manifest, "sensorCanonicalJson")?
        .as_str()
        .ok_or("invalid brain sensor bytes")?;
    let sensor: Value = serde_json::from_str(sensor_text).map_err(|e| e.to_string())?;
    let profile: SensorProfile =
        serde_json::from_value(field(&sensor, "profile")?.clone()).map_err(|e| e.to_string())?;
    let bins = field(&sensor, "bins")?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("invalid brain sensor bins")?;
    let sensors = SensorLayout::for_profile(profile, bins).map_err(|e| e.to_string())?;
    let deployed = field(&manifest.graph_manifest, "deployment")?;
    let life_state_rule = match field(deployed, "lifeStateRule")?.as_str() {
        Some("reset") => TrainingLifeStateRule::Reset,
        Some("preserve") => TrainingLifeStateRule::Preserve,
        _ => return Err("invalid brain physical-life state rule".into()),
    };
    let initialization = field(&manifest.graph_manifest, "initialization")?;
    let deployment = TrainingDeployment {
        control_interval_seconds: number(deployed, "controlIntervalSeconds")?,
        life_state_rule,
        initialization: GenomeInitializationConfig {
            gru_update_bias: number(initialization, "gruUpdateBias")?,
            lstm_forget_bias: number(initialization, "lstmForgetBias")?,
            rru_gate_bias: number(initialization, "rruGateBias")?,
        },
    };
    let canonical = CanonicalTrainingGraphManifestV1::build(&graph, &sensors, deployment)
        .map_err(|e| e.to_string())?;
    if canonical.total_parameters != parameter_count
        || serde_json::from_slice::<Value>(&canonical.canonical_bytes().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            != manifest.graph_manifest
    {
        return Err(
            "brain manifest disagrees with independently compiled native layout/semantics".into(),
        );
    }
    let raw_bytes = parameter_count
        .checked_mul(4)
        .ok_or("brain weight length overflow")?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(raw_bytes)
        .map_err(|_| "brain weight allocation admission")?;
    decode_adaptive_numeric_reader(
        &mut Cursor::new(payload),
        encoding,
        stored_bytes as u64,
        parameter_count as u64,
        raw_digest,
        &mut raw,
    )
    .map_err(|e| e.to_string())?;
    if raw.len() != raw_bytes {
        return Err("brain decoded weight length mismatch".into());
    }
    let weights: Vec<f32> = raw
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four bytes")))
        .collect();
    if weights.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite brain weights".into());
    }
    Ok(ValidatedBrainArtifact {
        graph,
        sensors,
        deployment,
        weights,
        canonical_manifest: canonical,
        provenance: manifest.provenance,
        artifact_sha256: hex(&Sha256::digest(bytes)),
        weights_sha256: hex(&raw_digest),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metadata() -> Value {
        let spec = json!({"type":"graph","nodes":[{"id":"input","type":"Input","outputSize":72},
            {"id":"motor","type":"Dense","inputSize":72,"outputSize":2}],
            "edges":[{"from":"input","to":"motor","fromPort":0,"toPort":0}],
            "outputs":[{"nodeId":"motor"}],"outputSize":2});
        let graph = compile_training_graph_json(&serde_json::to_vec(&spec).unwrap()).unwrap();
        let manifest = CanonicalTrainingGraphManifestV1::build(
            &graph,
            &SensorLayout::for_profile(SensorProfile::Basic, 8).unwrap(),
            TrainingDeployment {
                control_interval_seconds: 0.06,
                life_state_rule: TrainingLifeStateRule::Preserve,
                initialization: GenomeInitializationConfig::default(),
            },
        )
        .unwrap();
        json!({"schemaVersion":1, "graphManifest":serde_json::from_slice::<Value>(&manifest.canonical_bytes().unwrap()).unwrap(),
            "provenance":{"schemaVersion":1,"sourceKind":"distilled","details":{"method":"canonical-distillation-v1","updates":"00000000000000fa"}}})
    }

    fn encode(manifest: &Value, raw: &[u8], payload: &[u8], encoding: u8) -> Vec<u8> {
        let json = serde_json::to_vec(manifest).unwrap();
        let mut output = Vec::from(MAGIC.as_slice());
        output.extend_from_slice(&(json.len() as u32).to_le_bytes());
        output.extend_from_slice(&[encoding, 0, 0, 0]);
        output.extend_from_slice(&((raw.len() / 4) as u32).to_le_bytes());
        output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        output.extend_from_slice(&Sha256::digest(raw));
        output.extend_from_slice(&Sha256::digest(payload));
        output.extend(json);
        output.extend_from_slice(payload);
        output
    }

    fn raw() -> Vec<u8> {
        let mut values = vec![0_u8; 146 * 4];
        values[..4].copy_from_slice(&(-0.0_f32).to_le_bytes());
        values
    }

    #[test]
    fn admits_raw_weights_and_independently_compiles_every_descriptor() {
        let raw = raw();
        let bytes = encode(&metadata(), &raw, &raw, 0);
        let artifact = validate_brain_artifact(&bytes).unwrap();
        assert_eq!(artifact.weights[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(artifact.weights.len(), 146);
        assert_eq!(artifact.descriptor().input_size, 72);
        assert_eq!(artifact.descriptor().control_interval_seconds, 0.06);
        assert_eq!(
            artifact.descriptor().source_kind,
            BrainSourceKind::Distilled
        );
        assert_eq!(
            artifact.descriptor().artifact_sha256,
            hex(&Sha256::digest(&bytes))
        );
    }

    #[test]
    fn reuses_checkpoint_shuffled_zstandard_validation_without_losing_float_bits() {
        let raw = raw();
        let count = raw.len() / 4;
        let mut shuffled = vec![0; raw.len()];
        for index in 0..count {
            for lane in 0..4 {
                shuffled[lane * count + index] = raw[index * 4 + lane];
            }
        }
        let compressed = zstd::bulk::compress(&shuffled, 1).unwrap();
        let mut payload = Vec::from(b"SFZ1".as_slice());
        payload.extend_from_slice(&(count as u32).to_le_bytes());
        payload.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        payload.extend(compressed);
        let encoded = encode(&metadata(), &raw, &payload, 1);
        let artifact = validate_brain_artifact(&encoded).unwrap();
        assert_eq!(artifact.weights[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(artifact.weights_sha256, hex(&Sha256::digest(&raw)));
        payload.push(0);
        assert!(validate_brain_artifact(&encode(&metadata(), &raw, &payload, 1)).is_err());
    }

    #[test]
    fn rejects_changed_metadata_width_views_ports_equations_and_sensor_digest() {
        let raw = raw();
        for field in [
            "totalParameters",
            "inputSize",
            "layoutSha256",
            "sensorSha256",
            "equationsSha256",
        ] {
            let mut manifest = metadata();
            manifest["graphManifest"][field] = match field {
                "totalParameters" => json!(145),
                "inputSize" => json!(144),
                _ => json!("00".repeat(32)),
            };
            assert!(
                validate_brain_artifact(&encode(&manifest, &raw, &raw, 0)).is_err(),
                "{field}"
            );
        }
        let mut manifest = metadata();
        manifest["graphManifest"]["nodes"][1]["parameterBlocks"][0]["offset"] = json!(1);
        assert!(validate_brain_artifact(&encode(&manifest, &raw, &raw, 0)).is_err());
        let mut manifest = metadata();
        manifest["graphManifest"]["outputs"][0]["port"] = json!(1);
        assert!(validate_brain_artifact(&encode(&manifest, &raw, &raw, 0)).is_err());
    }

    #[test]
    fn rejects_truncation_trailing_versions_reserved_flags_nonfinite_and_digest_damage() {
        let raw = raw();
        let bytes = encode(&metadata(), &raw, &raw, 0);
        for size in 0..bytes.len() {
            assert!(validate_brain_artifact(&bytes[..size]).is_err());
        }
        for offset in [0, 12, 13, 16, 20, 24, 56, bytes.len() - 1] {
            let mut changed = bytes.clone();
            changed[offset] ^= 255;
            assert!(
                validate_brain_artifact(&changed).is_err(),
                "offset {offset}"
            );
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(validate_brain_artifact(&trailing).is_err());
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut damaged = raw.clone();
            damaged[..4].copy_from_slice(&value.to_le_bytes());
            assert!(validate_brain_artifact(&encode(&metadata(), &damaged, &damaged, 0)).is_err());
        }
    }

    #[test]
    fn bounds_provenance_and_rejects_unknown_manifest_fields() {
        let raw = raw();
        let mut manifest = metadata();
        manifest["unknown"] = json!(1);
        assert!(validate_brain_artifact(&encode(&manifest, &raw, &raw, 0)).is_err());
        let mut manifest = metadata();
        manifest["provenance"]["details"]["text"] = json!("x".repeat(4097));
        assert!(validate_brain_artifact(&encode(&manifest, &raw, &raw, 0)).is_err());
        let mut manifest = metadata();
        manifest["provenance"]["sourceKind"] = json!("hallOfFame");
        assert!(validate_brain_artifact(&encode(&manifest, &raw, &raw, 0)).is_err());
    }
}
