//! Independently validate and materialize a portable brain, with no world/RNG/controller.
use slither_native::engine::brain_artifact::{validate_brain_artifact, MAX_BRAIN_ARTIFACT_BYTES};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("provide artifact and absent output directory")?,
    );
    let output = PathBuf::from(args.next().ok_or("provide absent output directory")?);
    if args.next().is_some() {
        return Err("unexpected brain artifact arguments".into());
    }
    let file = File::open(input)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BRAIN_ARTIFACT_BYTES as u64 {
        return Err("brain file admission".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BRAIN_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let brain = validate_brain_artifact(&bytes)?;
    let descriptor = serde_json::json!({"schemaVersion":1,"descriptor":brain.descriptor(),
        "nativeSourceSha256":env!("SLITHER_NATIVE_SOURCE_SHA256"),"nativeTarget":env!("SLITHER_NATIVE_BUILD_TARGET")});
    let weights: Vec<u8> = brain
        .weights
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    fs::create_dir(&output)?;
    for (name, bytes) in [
        ("descriptor.json", serde_json::to_vec(&descriptor)?),
        ("manifest.json", brain.canonical_manifest.canonical_bytes()?),
        ("weights.f32", weights),
    ] {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    println!("{}", output.join("descriptor.json").display());
    Ok(())
}
