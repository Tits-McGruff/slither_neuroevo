//! Synthetic cross-language codec reference, never a claim of committed gameplay.

use sha2::{Digest, Sha256};
use slither_native::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
use slither_native::engine::teacher_samples::{
    TeacherChunkV1, TeacherContractV1, TeacherEndReason, TeacherSampleV1, TeacherStartReason,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(args.next().ok_or("provide one new output directory")?);
    if args.next().is_some() {
        return Err("provide exactly one output directory".into());
    }
    let sensor = SensorContractIdentity::new(SensorProfile::Basic, 8)?;
    let mut source = Sha256::new();
    source.update(b"slither.baseline-teacher.v2\0");
    source.update(env!("SLITHER_NATIVE_SOURCE_SHA256").as_bytes());
    let contract = TeacherContractV1 {
        capture_epoch: [17; 16],
        run_id: "codec-α".into(),
        config_sha256: [23; 32],
        sensor_sha256: sensor.sha256()?,
        teacher_sha256: source.finalize().into(),
        teacher_revision: 2,
        teacher_mode: 1,
        control_interval_seconds: 0.06,
        input_size: sensor.input_size(),
    };
    let samples = (0..3)
        .map(|index| {
            let mut observation = vec![-1.0; sensor.input_size()];
            observation[0] = -0.0;
            observation[1] = 0.125;
            observation[21] = -1.0;
            TeacherSampleV1 {
                step: 42 + index * 4,
                ordinal: index + 1,
                observation,
                applied_turn: if index == 0 { -0.0 } else { 0.25 },
                boost_request: index != 1,
                realized_boost: index == 2,
                post_step_death: index == 2,
                first_physical_sample: false,
                strategy: index as u8,
                life_stage: index as u8,
                strata: if index == 0 { 0 } else { 63 },
            }
        })
        .collect();
    let chunk = TeacherChunkV1 {
        contract,
        chunk_sequence: 7,
        generation: u64::MAX,
        baseline_slot: 119,
        physical_snake_id: u64::MAX,
        start_reason: TeacherStartReason::CaptureStart,
        end_reason: TeacherEndReason::LifeEnded,
        samples,
    };
    let bytes = chunk.encode()?;
    let hex = |bytes: &[u8]| -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() };
    let metadata = serde_json::json!({
        "schemaVersion": 1, "scope": "synthetic Rust codec reference; no committed-world claim", "runSeed": 42,
        "sourceSha256": env!("SLITHER_NATIVE_SOURCE_SHA256"),
        "sensorContractJson": String::from_utf8(sensor.canonical_bytes()?)?,
        "contract": { "captureEpoch": hex(&chunk.contract.capture_epoch), "runId": chunk.contract.run_id,
            "configSha256": hex(&chunk.contract.config_sha256), "sensorSha256": hex(&chunk.contract.sensor_sha256),
            "teacherSha256": hex(&chunk.contract.teacher_sha256), "teacherRevision": 2, "teacherMode": 1,
            "controlIntervalSeconds": 0.06, "inputSize": sensor.input_size() },
        "descriptor": { "captureEpoch": hex(&chunk.contract.capture_epoch), "sequence": "0000000000000007",
            "generation": "ffffffffffffffff", "baselineSlot": 119, "physicalSnakeId": "ffffffffffffffff",
            "firstSample": "0000000000000001", "lastSample": "0000000000000003",
            "firstStep": "000000000000002a", "lastStep": "0000000000000032", "count": 3,
            "inputSize": sensor.input_size(), "configSha256": hex(&chunk.contract.config_sha256),
            "sensorSha256": hex(&chunk.contract.sensor_sha256), "teacherSha256": hex(&chunk.contract.teacher_sha256),
            "startReason": "captureStart", "endReason": "lifeEnded", "byteCount": bytes.len(),
            "sha256": hex(&Sha256::digest(&bytes)) }
    });
    fs::create_dir(&output)?;
    for (name, data) in [
        ("chunk.bin", bytes),
        ("reference.json", serde_json::to_vec_pretty(&metadata)?),
    ] {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    Ok(())
}
