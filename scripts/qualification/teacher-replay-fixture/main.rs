//! Bounded synthetic replay fixture using the production Rust teacher encoder.
//! This emits no committed-gameplay claim and advances no authoritative RNG.

use sha2::{Digest, Sha256};
use slither_native::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
use slither_native::engine::teacher_samples::{
    TeacherChunkV1, TeacherContractV1, TeacherEndReason, TeacherSampleV1, TeacherStartReason,
};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

#[cfg(target_os = "linux")]
fn available_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    let status = rustix::fs::statvfs(path)?;
    status
        .f_bavail
        .checked_mul(status.f_frsize)
        .ok_or_else(|| "filesystem byte overflow".into())
}

#[cfg(target_os = "windows")]
fn available_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: The NUL-terminated UTF-16 input and single writable u64 output live
    // through the call; optional outputs are null and no buffers overlap.
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(available)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("required: NEW_OUTPUT_DIRECTORY CHUNKS".into());
    }
    let output = Path::new(&args[0]);
    let count: u64 = args[1].to_str().ok_or("non-UTF8 count")?.parse()?;
    if !(64..=16384).contains(&count) {
        return Err("chunk count must be 64..16384".into());
    }
    let parent = output
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let required = count
        .checked_mul(168 + 128 + 60 * (25 + 504 * 4) + 4)
        .ok_or("byte overflow")?;
    if available_bytes(parent)? < required + 1024 * 1024 * 1024 {
        return Err("preserve one-GiB physical reserve".into());
    }
    let sensor = SensorContractIdentity::new(SensorProfile::Motion, 32)?;
    let contract = TeacherContractV1 {
        capture_epoch: [17; 16],
        run_id: "replay-capacity-qualification".into(),
        config_sha256: [23; 32],
        sensor_sha256: sensor.sha256()?,
        teacher_sha256: [37; 32],
        teacher_revision: 2,
        teacher_mode: 1,
        control_interval_seconds: 1.0 / 60.0,
        input_size: sensor.input_size(),
    };
    fs::create_dir(output)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("chunks.bin"))?;
    let mut writer = BufWriter::new(file);
    let mut hash = Sha256::new();
    let mut encoded_bytes = 0u64;
    let mut first_digest = String::new();
    let mut last_digest = String::new();
    for sequence in 1..=count {
        let samples = (0..60)
            .map(|offset| {
                let ordinal = (sequence - 1) * 60 + offset + 1;
                let mut observation = vec![-0.5; sensor.input_size()];
                observation[0] = -0.0;
                observation[1] = 0.125;
                observation[21] = -1.0;
                TeacherSampleV1 {
                    step: ordinal,
                    ordinal,
                    observation,
                    applied_turn: 0.25,
                    boost_request: offset % 2 == 0,
                    realized_boost: false,
                    post_step_death: false,
                    first_physical_sample: false,
                    strategy: (offset % 4) as u8,
                    life_stage: (offset % 3) as u8,
                    strata: if offset % 3 == 0 { 0 } else { 63 },
                }
            })
            .collect();
        let chunk = TeacherChunkV1 {
            contract: contract.clone(),
            chunk_sequence: sequence,
            generation: 1,
            baseline_slot: 0,
            physical_snake_id: 1,
            start_reason: if sequence == 1 {
                TeacherStartReason::CaptureStart
            } else {
                TeacherStartReason::Continuation
            },
            end_reason: TeacherEndReason::SimulatedSecond,
            samples,
        };
        let bytes = chunk.encode()?;
        if TeacherChunkV1::decode(&bytes)? != chunk {
            return Err("native codec roundtrip differs".into());
        }
        let prefix = u32::try_from(bytes.len())?.to_le_bytes();
        writer.write_all(&prefix)?;
        writer.write_all(&bytes)?;
        hash.update(prefix);
        hash.update(&bytes);
        encoded_bytes += bytes.len() as u64;
        last_digest = hex(&Sha256::digest(&bytes));
        if sequence == 1 {
            first_digest = last_digest.clone();
        }
        if sequence % 1024 == 0 {
            println!("[replay-fixture] encoded {sequence}/{count}");
        }
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    let receipt = serde_json::json!({"schemaVersion":1,"scope":"Synthetic Rust-encoded replay capacity fixture; no committed-gameplay claim",
        "sourceSha256":slither_native::native_addon_source_sha256(),"buildTarget":slither_native::native_addon_build_target(),
        "hostCanonicalLfSha256":hex(&Sha256::digest(include_str!("main.rs").replace("\r\n", "\n").as_bytes())),
        "chunks":count,"samplesPerChunk":60,"encodedBytes":encoded_bytes,"bundleBytes":encoded_bytes + count * 4,
        "bundleSha256":hex(&hash.finalize()),"firstChunkSha256":first_digest,"lastChunkSha256":last_digest,
        "dataset":{"schemaVersion":1,"runSeed":42,"contract":{"captureEpoch":hex(&contract.capture_epoch),"runId":contract.run_id,
            "configSha256":hex(&contract.config_sha256),"sensorSha256":hex(&contract.sensor_sha256),"teacherSha256":hex(&contract.teacher_sha256),
            "teacherRevision":2,"teacherMode":1,"controlIntervalSeconds":contract.control_interval_seconds,"inputSize":contract.input_size},
            "sensorContractJson":String::from_utf8(sensor.canonical_bytes()?)?,"state":"stopped",
            "productionWatermark":format!("{count:016x}"),"storageWatermark":format!("{count:016x}"),"retentionFloor":"0000000000000001",
            "resolvedThrough":format!("{count:016x}"),"nativeDroppedChunks":"0000000000000000","nativeDroppedSamples":"0000000000000000",
            "nativeDroppedBytes":"0000000000000000","spoolDroppedChunks":"0000000000000000","retainedChunks":count,
            "retainedBytes":format!("{encoded_bytes:016x}"),"error":null}});
    let mut receipt_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("reference.json"))?;
    serde_json::to_writer_pretty(&mut receipt_file, &receipt)?;
    receipt_file.write_all(b"\n")?;
    receipt_file.sync_all()?;
    println!("[replay-fixture] complete {}", output.display());
    Ok(())
}
