//! Emit retained Stage 5 single-worker complete fixed-step evidence.

use slither_native::engine::inference::InferenceMathBackend;
use slither_native::engine::inference_fixture::Stage4InferenceScenarioName;
use slither_native::engine::step_fixture::{run_stage5_step_evidence, Stage5StepEvidenceOptions};
use std::alloc::{GlobalAlloc, Layout, System};
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process allocation operations observed by this evidence runner.
static ALLOCATION_OPERATIONS: AtomicU64 = AtomicU64::new(0);

/// System allocator wrapper used only by this test-hook benchmark binary.
struct CountingAllocator;

// SAFETY: Every operation delegates to `System` with the original pointer and
// layout. The relaxed counter is diagnostic only and does not affect validity.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegating the caller-provided valid allocation layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegating the caller-provided valid allocation layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: Delegating the original pointer and allocation layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegating the original pointer/layout and requested size.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

/// Fully parsed runner arguments.
struct CliOptions {
    scenario: Stage4InferenceScenarioName,
    math_backend: InferenceMathBackend,
    warmup_steps: usize,
    measured_steps: usize,
    evidence_environment: String,
    output_path: PathBuf,
    command: Vec<String>,
}

fn parse_count(value: Option<OsString>, name: &str, allow_zero: bool) -> Result<usize, String> {
    let text = value
        .ok_or_else(|| format!("{name} requires a value"))?
        .into_string()
        .map_err(|_| format!("{name} must be valid UTF-8"))?;
    let parsed = text
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an integer"))?;
    let minimum = usize::from(!allow_zero);
    if parsed < minimum || parsed > 300 {
        return Err(format!("{name} must be an integer from {minimum} to 300"));
    }
    Ok(parsed)
}

fn parse_options() -> Result<CliOptions, String> {
    let command = env::args().collect::<Vec<_>>();
    let mut arguments = env::args_os().skip(1);
    let mut scenario = None;
    let mut math_backend = None;
    let mut warmup_steps = 3;
    let mut measured_steps = 30;
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
            Some("--warmup-steps") => {
                warmup_steps = parse_count(arguments.next(), "--warmup-steps", true)?;
            }
            Some("--steps") => {
                measured_steps = parse_count(arguments.next(), "--steps", false)?;
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
        warmup_steps,
        measured_steps,
        evidence_environment,
        output_path: output_path.ok_or_else(|| "--output is required".to_owned())?,
        command,
    })
}

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

fn allocation_snapshot() -> u64 {
    ALLOCATION_OPERATIONS.load(Ordering::Relaxed)
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options().map_err(std::io::Error::other)?;
    let output_path = options.output_path.clone();
    let scenario = options.scenario.label();
    let math_backend = options.math_backend.label();
    let report = run_stage5_step_evidence(
        Stage5StepEvidenceOptions {
            scenario: options.scenario,
            math_backend: options.math_backend,
            warmup_steps: options.warmup_steps,
            measured_steps: options.measured_steps,
            evidence_environment: options.evidence_environment,
            command: options.command,
        },
        allocation_snapshot,
    )
    .map_err(std::io::Error::other)?;
    write_report(&output_path, &serde_json::to_vec_pretty(&report)?)?;
    println!(
        "wrote {scenario} {math_backend} single-worker complete-step evidence to {}",
        output_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p0_binary_counts_allocations_while_publishing_complete_steps() {
        let report = run_stage5_step_evidence(
            Stage5StepEvidenceOptions {
                scenario: Stage4InferenceScenarioName::P0,
                math_backend: InferenceMathBackend::Scalar,
                warmup_steps: 1,
                measured_steps: 1,
                evidence_environment: "development".to_owned(),
                command: vec!["test".to_owned()],
            },
            allocation_snapshot,
        )
        .unwrap();
        assert_eq!(report.result.final_completed_step, 2);
        assert_eq!(
            report.result.allocator_operations_per_complete_step.count,
            1
        );
    }
}
