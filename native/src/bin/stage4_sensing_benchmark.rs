//! Emit retained Stage 4 corrected Rust sensing/index evidence.

use slither_native::engine::sensing_fixture::{
    run_stage4_sensing_evidence, Stage4SensingEvidenceOptions, Stage4SensingScenarioName,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process allocation operations observed by the standalone evidence runner.
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
    scenario: Stage4SensingScenarioName,
    warmup_passes: usize,
    measured_passes: usize,
    evidence_environment: String,
    output_path: PathBuf,
    command: Vec<String>,
}

/// Parse one bounded pass count.
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
    let mut warmup_passes = 5;
    let mut measured_passes = 30;
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
                scenario = Some(Stage4SensingScenarioName::parse(&value)?);
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
        warmup_passes,
        measured_passes,
        evidence_environment,
        output_path: output_path.ok_or_else(|| "--output is required".to_owned())?,
        command,
    })
}

/// Write and sync one evidence document after every assertion passes.
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

/// Read the monotonic allocator-operation counter.
fn allocation_snapshot() -> u64 {
    ALLOCATION_OPERATIONS.load(Ordering::Relaxed)
}

/// Run one source-shaped scenario through the production Rust sensing path.
fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options().map_err(std::io::Error::other)?;
    let output_path = options.output_path.clone();
    let scenario = options.scenario.label();
    let report = run_stage4_sensing_evidence(
        Stage4SensingEvidenceOptions {
            scenario: options.scenario,
            warmup_passes: options.warmup_passes,
            measured_passes: options.measured_passes,
            evidence_environment: options.evidence_environment,
            command: options.command,
        },
        allocation_snapshot,
    )
    .map_err(std::io::Error::other)?;
    write_report(&output_path, &serde_json::to_vec_pretty(&report)?)?;
    println!(
        "wrote {scenario} corrected sensing evidence to {}",
        output_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_counter_observes_real_heap_growth() {
        let before = allocation_snapshot();
        let mut values = Vec::<u64>::with_capacity(1_024);
        values.push(42);
        std::hint::black_box(&values);
        let after = allocation_snapshot();
        assert!(after > before);
    }
}
