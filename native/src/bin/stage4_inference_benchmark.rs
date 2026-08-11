//! Emit retained Stage 4 Rust scalar/SIMD heterogeneous-inference evidence.

use slither_native::engine::inference::InferenceMathBackend;
use slither_native::engine::inference_fixture::{
    run_stage4_inference_evidence, Stage4InferenceEvidenceOptions, Stage4InferenceScenarioName,
};
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Fully parsed runner arguments.
struct CliOptions {
    scenario: Stage4InferenceScenarioName,
    math_backend: InferenceMathBackend,
    warmup_passes: usize,
    measured_passes: usize,
    evidence_environment: String,
    output_path: PathBuf,
    command: Vec<String>,
}

/// Parse one bounded count.
fn parse_count(value: Option<OsString>, name: &str, allow_zero: bool) -> Result<usize, String> {
    let text = value
        .ok_or_else(|| format!("{name} requires a value"))?
        .into_string()
        .map_err(|_| format!("{name} must be valid UTF-8"))?;
    let parsed = text
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an integer"))?;
    let minimum = usize::from(!allow_zero);
    if parsed < minimum || parsed > 100_000 {
        return Err(format!(
            "{name} must be an integer from {minimum} to 100000"
        ));
    }
    Ok(parsed)
}

/// Parse explicit scenario, timing, provenance, and output arguments.
fn parse_options() -> Result<CliOptions, String> {
    let command = env::args().collect::<Vec<_>>();
    let mut arguments = env::args_os().skip(1);
    let mut scenario = None;
    let mut math_backend = None;
    let mut warmup_passes = 10;
    let mut measured_passes = 60;
    let mut evidence_environment = "development".to_owned();
    let mut output_path = None;
    while let Some(flag) = arguments.next() {
        match flag.to_str() {
            Some("--scenario") => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--scenario requires a value".to_owned())?
                    .into_string()
                    .map_err(|_| "--scenario must be valid UTF-8".to_owned())?;
                scenario = Some(Stage4InferenceScenarioName::parse(&value)?);
            }
            Some("--math-backend") => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--math-backend requires a value".to_owned())?
                    .into_string()
                    .map_err(|_| "--math-backend must be valid UTF-8".to_owned())?;
                math_backend = Some(match value.as_str() {
                    "scalar" => InferenceMathBackend::Scalar,
                    "sse2" => InferenceMathBackend::Sse2,
                    _ => return Err("--math-backend must be scalar or sse2".to_owned()),
                });
            }
            Some("--warmup-passes") => {
                warmup_passes = parse_count(arguments.next(), "--warmup-passes", true)?;
            }
            Some("--passes") => {
                measured_passes = parse_count(arguments.next(), "--passes", false)?;
            }
            Some("--environment") => {
                evidence_environment = arguments
                    .next()
                    .ok_or_else(|| "--environment requires a value".to_owned())?
                    .into_string()
                    .map_err(|_| "--environment must be valid UTF-8".to_owned())?;
                if evidence_environment != "development"
                    && evidence_environment != "owner-target-vm"
                {
                    return Err("--environment must be development or owner-target-vm".to_owned());
                }
            }
            Some("--output") => {
                output_path = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--output requires a path".to_owned())?,
                ));
            }
            Some(other) => return Err(format!("unknown option {other}")),
            None => return Err("options must be valid UTF-8".to_owned()),
        }
    }
    Ok(CliOptions {
        scenario: scenario.ok_or_else(|| "--scenario is required".to_owned())?,
        math_backend: math_backend.ok_or_else(|| "--math-backend is required".to_owned())?,
        warmup_passes,
        measured_passes,
        evidence_environment,
        output_path: output_path.ok_or_else(|| "--output is required".to_owned())?,
        command,
    })
}

/// Write and sync one evidence document only after all assertions pass.
fn write_report(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Run one source-shaped scenario through an explicit Rust population implementation.
fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options().map_err(std::io::Error::other)?;
    let output_path = options.output_path.clone();
    let scenario = options.scenario.label();
    let math_backend = options.math_backend.label();
    let report = run_stage4_inference_evidence(Stage4InferenceEvidenceOptions {
        scenario: options.scenario,
        math_backend: options.math_backend,
        warmup_passes: options.warmup_passes,
        measured_passes: options.measured_passes,
        evidence_environment: options.evidence_environment,
        command: options.command,
    })
    .map_err(std::io::Error::other)?;
    write_report(&output_path, &serde_json::to_vec_pretty(&report)?)?;
    println!(
        "wrote {scenario} {math_backend} inference evidence to {}",
        output_path.display()
    );
    Ok(())
}
