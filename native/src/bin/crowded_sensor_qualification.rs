//! Standalone bounded in-arena v4 sensing pressure; test-hook build only.
use slither_native::engine::crowded_sensor_qualification::{
    run_crowded_sensor_qualification, CrowdedSensorOptions,
};
use slither_native::engine::ray_work::RayWorkLimits;
use slither_native::engine::sensing_fixture::Stage4SensingScenarioName;
use slither_native::engine::sensor_contract::SensorProfile;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
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

/// Parse explicit ray admission without altering the production constructor or defaults.
fn parse() -> Result<(CrowdedSensorOptions, PathBuf), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut values = BTreeMap::new();
    while let Some(flag) = arguments.next() {
        if ![
            "--scenario",
            "--profile",
            "--bins",
            "--workers",
            "--minimum",
            "--cap",
            "--batch",
            "--warmup",
            "--passes",
            "--output",
        ]
        .contains(&flag.as_str())
        {
            return Err(format!("unknown option {flag}").into());
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if values.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate option {flag}").into());
        }
    }
    let required = |name: &str| {
        values
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {name}"))
    };
    let integer = |name: &str, default: &str| {
        values
            .get(name)
            .map(String::as_str)
            .unwrap_or(default)
            .parse::<usize>()
    };
    let profile = match required("--profile")? {
        "basic" => SensorProfile::Basic,
        "motion" => SensorProfile::Motion,
        _ => return Err("--profile must be basic or motion".into()),
    };
    let options = CrowdedSensorOptions {
        scenario: Stage4SensingScenarioName::parse(required("--scenario")?)?,
        profile,
        bins: required("--bins")?.parse()?,
        workers: integer("--workers", "1")?,
        warmup_passes: integer("--warmup", "5")?,
        measured_passes: integer("--passes", "100")?,
        ray_limits: RayWorkLimits::new(
            required("--minimum")?.parse()?,
            required("--cap")?.parse()?,
            required("--batch")?.parse()?,
            512,
        )?,
    };
    Ok((options, PathBuf::from(required("--output")?)))
}

/// Sync one create-new report only after every native work, geometry and scratch check succeeds.
fn main() -> Result<(), Box<dyn Error>> {
    let (options, output) = parse()?;
    if output.try_exists()? {
        return Err("output already exists".into());
    }
    if slither_native::native_addon_build_profile() != "release" {
        return Err("crowded sensing measurements require a release build".into());
    }
    let report = run_crowded_sensor_qualification(&options, || {
        ALLOCATION_OPERATIONS.load(Ordering::Relaxed)
    })?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    if bytes.len() > 1024 * 1024 {
        return Err("crowded report exceeds one MiB".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
