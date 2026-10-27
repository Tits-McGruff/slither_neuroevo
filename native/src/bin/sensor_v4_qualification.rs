//! Actual native sensor workload evidence; all v4 ray limits are operator inputs.

use slither_native::engine::inference::InferenceMathBackend;
use slither_native::engine::ray_work::RayWorkLimits;
use slither_native::engine::sensor_contract::SensorProfile;
use slither_native::engine::sensor_qualification::{
    run_sensor_qualification, SensorQualificationOptions,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// One exclusively created scratch tree; only this exact resolved tree is removable.
struct Workspace {
    path: PathBuf,
}

impl Workspace {
    fn create(path: &Path) -> Result<Self, Box<dyn Error>> {
        fs::create_dir(path)?;
        let path = path.canonicalize()?;
        Ok(Self { path })
    }
    fn cleanup(&self) -> Result<(), Box<dyn Error>> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || self.path.canonicalize()? != self.path
        {
            return Err("task workspace identity changed before cleanup".into());
        }
        fs::remove_dir_all(&self.path)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn available_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    let status = rustix::fs::statvfs(path)?;
    status
        .f_bavail
        .checked_mul(status.f_frsize)
        .ok_or_else(|| "filesystem byte count overflow".into())
}

#[cfg(target_os = "windows")]
fn available_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: NUL-terminated UTF-16 input lives through the call; the single
    // non-null output points to one writable u64, and optional outputs are null.
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

/// Complete CLI request, including an opt-in shadow artifact for continuous soak mode.
struct Arguments {
    options: SensorQualificationOptions,
    path: PathBuf,
    output: PathBuf,
    soak_shadow: Option<PathBuf>,
    cohort: Option<PathBuf>,
    evolved_checkpoint: Option<PathBuf>,
    evolve_generations: Option<u64>,
    production_source: bool,
}

fn parse() -> Result<Arguments, Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut values = BTreeMap::new();
    while let Some(flag) = arguments.next() {
        if ![
            "--graph-json",
            "--evolved-checkpoint",
            "--evolve-generations",
            "--source-build-class",
            "--population",
            "--clock",
            "--soak-shadow-artifact",
            "--trained-cohort",
            "--capture",
            "--capture-drain-steps",
            "--capture-queue-chunks",
            "--baseline",
            "--baseline-cadence",
            "--teacher-respawn-delay",
            "--scenario",
            "--profile",
            "--bins",
            "--pellet-checks",
            "--segment-checks",
            "--graph",
            "--trained-artifact",
            "--respawn",
            "--seed",
            "--workers",
            "--math-backend",
            "--minimum",
            "--cap",
            "--batch",
            "--warmup",
            "--steps",
            "--environment",
            "--workspace",
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
        "v3" => SensorProfile::V3,
        "basic" => SensorProfile::Basic,
        "motion" => SensorProfile::Motion,
        _ => return Err("--profile must be v3/basic/motion".into()),
    };
    let ray_limits = if profile == SensorProfile::V3 {
        if ["--minimum", "--cap", "--batch"]
            .iter()
            .any(|key| values.contains_key(*key))
        {
            return Err("v3 does not use capsule-ray grants".into());
        }
        None
    } else {
        Some(RayWorkLimits::new(
            required("--minimum")?.parse()?,
            required("--cap")?.parse()?,
            required("--batch")?.parse()?,
            512,
        )?)
    };
    let baseline = match values.get("--baseline").map(String::as_str).unwrap_or("v1") {
        "v1" => {
            if values.contains_key("--baseline-cadence") {
                return Err("v1 has fixed-step cadence".into());
            }
            None
        }
        name @ ("v2-deterministic" | "v2-stochastic") => {
            if profile == SensorProfile::V3 {
                return Err("baseline v2 requires v4".into());
            }
            let settings = slither_native::engine::baseline_v2::BaselineV2Settings {
                mode: if name == "v2-deterministic" {
                    slither_native::engine::baseline_v2::BaselineV2Mode::Deterministic
                } else {
                    slither_native::engine::baseline_v2::BaselineV2Mode::Stochastic
                },
                control_interval_seconds: required("--baseline-cadence")?.parse()?,
            };
            settings.validate()?;
            Some(settings)
        }
        _ => return Err("baseline must be v1/v2-deterministic/v2-stochastic".into()),
    };
    let capture = match values.get("--capture").map(String::as_str).unwrap_or("off") {
        "off" => {
            if ["--capture-drain-steps", "--capture-queue-chunks"]
                .iter()
                .any(|key| values.contains_key(*key))
            {
                return Err("capture pressure requires --capture on".into());
            }
            None
        }
        "on" => {
            if baseline.is_none() {
                return Err("capture requires baseline v2".into());
            }
            Some(
                slither_native::engine::capture_qualification::CaptureQualificationOptions {
                    drain_every_steps: integer("--capture-drain-steps", "1")?,
                    queue_chunks: integer("--capture-queue-chunks", "1024")?,
                },
            )
        }
        _ => return Err("--capture must be off/on".into()),
    };
    if values.contains_key("--trained-artifact") && values.contains_key("--graph") {
        return Err("--trained-artifact supplies the graph; omit --graph".into());
    }
    if values.contains_key("--trained-cohort")
        && ["--trained-artifact", "--graph", "--soak-shadow-artifact"]
            .iter()
            .any(|key| values.contains_key(*key))
    {
        return Err(
            "--trained-cohort supplies its graph; omit artifact, graph and soak overrides".into(),
        );
    }
    let query_limits = match (
        values.get("--pellet-checks"),
        values.get("--segment-checks"),
    ) {
        (None, None) => None,
        (Some(pellets), Some(segments)) => Some(
            slither_native::engine::sensor_qualification::QueryDetailLimits::new(
                pellets.parse()?,
                segments.parse()?,
            )?,
        ),
        _ => return Err("--pellet-checks and --segment-checks must be supplied together".into()),
    };
    let source_graph = values
        .get("--graph-json")
        .map(|path| -> Result<_, Box<dyn Error>> {
            if [
                "--graph",
                "--trained-artifact",
                "--trained-cohort",
                "--soak-shadow-artifact",
            ]
            .iter()
            .any(|key| values.contains_key(*key))
            {
                return Err(
                    "custom graph cannot be combined with canned, trained or shadow graph sources"
                        .into(),
                );
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            slither_native::engine::graph_json::decode_graph_json(&bytes).map_err(Into::into)
        })
        .transpose()?;
    let real_clock = match values
        .get("--clock")
        .map(String::as_str)
        .unwrap_or("controlled")
    {
        "controlled" => false,
        "real" => true,
        _ => return Err("--clock must be controlled or real".into()),
    };
    if real_clock && values.contains_key("--soak-shadow-artifact") {
        return Err("continuous capture soak uses its existing controlled-clock mode".into());
    }
    if values.contains_key("--evolve-generations") && !values.contains_key("--evolved-checkpoint") {
        return Err("--evolve-generations requires --evolved-checkpoint".into());
    }
    let production_source = match values
        .get("--source-build-class")
        .map(String::as_str)
        .unwrap_or("test-hooks")
    {
        "production" => true,
        "test-hooks" => false,
        _ => return Err("--source-build-class requires production or test-hooks".into()),
    };
    if values.contains_key("--source-build-class") && !values.contains_key("--evolved-checkpoint") {
        return Err("source class applies only to explicit population transfer".into());
    }
    if values.contains_key("--evolved-checkpoint")
        && [
            "--graph-json",
            "--graph",
            "--trained-artifact",
            "--trained-cohort",
            "--soak-shadow-artifact",
            "--capture",
        ]
        .iter()
        .any(|key| values.contains_key(*key))
    {
        return Err("evolved checkpoint cannot be combined with other source/capture modes".into());
    }
    let population = match (values.get("--population"), values.get("--scenario")) {
        (Some(value), None)
            if source_graph.is_some() || values.contains_key("--evolved-checkpoint") =>
        {
            let count: usize = value.parse()?;
            if !(1..=300).contains(&count) {
                return Err("custom population must be 1..300".into());
            }
            count
        }
        (None, Some(value)) => match value.as_str() {
            "P0" => 55,
            "P1" => 300,
            _ => return Err("--scenario must be P0/P1".into()),
        },
        _ => {
            return Err("supply --scenario P0/P1, or --graph-json with --population 1..300".into())
        }
    };
    let options = SensorQualificationOptions {
        source_graph,
        real_clock,
        query_limits,
        trained_artifact: values.get("--trained-artifact").map(PathBuf::from),
        population,
        profile,
        bins: integer("--bins", "16")?,
        ray_limits,
        baseline,
        teacher_respawn_delay_seconds: values
            .get("--teacher-respawn-delay")
            .map(String::as_str)
            .unwrap_or("20")
            .parse()?,
        capture,
        large_graph: match values
            .get("--graph")
            .map(String::as_str)
            .unwrap_or("modest")
        {
            "modest" => false,
            "large" => true,
            _ => return Err("--graph must be modest/large".into()),
        },
        respawn: match values.get("--respawn").map(String::as_str).unwrap_or("off") {
            "off" => false,
            "on" => true,
            _ => return Err("--respawn must be off/on".into()),
        },
        seed: values
            .get("--seed")
            .map(String::as_str)
            .unwrap_or("42")
            .parse()?,
        workers: integer("--workers", "1")?,
        math: match values
            .get("--math-backend")
            .map(String::as_str)
            .unwrap_or("scalar")
        {
            "scalar" => InferenceMathBackend::Scalar,
            "sse2" => InferenceMathBackend::Sse2,
            "i16-matrix-fma128" => InferenceMathBackend::I16MatrixFma128,
            _ => return Err("invalid arithmetic backend".into()),
        },
        warmup_steps: integer("--warmup", "3")?,
        measured_steps: integer("--steps", "300")?,
        environment: values
            .get("--environment")
            .cloned()
            .unwrap_or("development".into()),
    };
    if !["development", "oxygen-debian"].contains(&options.environment.as_str()) {
        return Err("unsupported environment label".into());
    }
    Ok(Arguments {
        options,
        path: PathBuf::from(required("--workspace")?),
        output: PathBuf::from(required("--output")?),
        soak_shadow: values.get("--soak-shadow-artifact").map(PathBuf::from),
        cohort: values.get("--trained-cohort").map(PathBuf::from),
        evolved_checkpoint: values.get("--evolved-checkpoint").map(PathBuf::from),
        evolve_generations: values
            .get("--evolve-generations")
            .map(|v| v.parse())
            .transpose()?,
        production_source,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let Arguments {
        options,
        path,
        output,
        soak_shadow,
        cohort,
        evolved_checkpoint,
        evolve_generations,
        production_source,
    } = parse()?;
    let workspace = Workspace::create(&path)?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        let output_parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if output_parent.canonicalize()?.starts_with(&workspace.path) {
            return Err("retained report must be outside disposable workspace".into());
        }
        // One immutable checkpoint can approach the current 512 MiB admission
        // ceiling. Preserve one GiB of free space after a conservative double
        // ceiling for raw/shuffled/encoded construction, not merely after its final.
        const REQUIRED_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
        if available_bytes(&workspace.path)? < REQUIRED_FREE_BYTES {
            return Err("qualification requires two GiB free to preserve its disk reserve".into());
        }
        let report = if let Some(shadow) = &soak_shadow {
            let progress_path = output.with_extension("progress.json");
            if progress_path == output {
                return Err("soak report and progress paths must differ".into());
            }
            let mut progress_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(progress_path)?;
            slither_native::engine::soak_qualification::run_sensor_soak(
                &options,
                &workspace.path,
                shadow,
                &mut |value| {
                    (|| -> Result<(), Box<dyn Error>> {
                        progress_file.seek(SeekFrom::Start(0))?;
                        progress_file.set_len(0)?;
                        serde_json::to_writer(&mut progress_file, value)?;
                        progress_file.write_all(b"\n")?;
                        progress_file.sync_all()?;
                        Ok(())
                    })()
                    .map_err(|error| error.to_string())
                },
            )
            .map_err(std::io::Error::other)?
        } else if let Some(checkpoint) = &evolved_checkpoint {
            if let Some(generations) = evolve_generations {
                serde_json::to_value(
                    slither_native::engine::sensor_qualification::mature_evolved_population(
                        &options,
                        &workspace.path,
                        checkpoint,
                        generations,
                        output_parent,
                        production_source,
                    )
                    .map_err(std::io::Error::other)?,
                )?
            } else {
                serde_json::to_value(
                slither_native::engine::sensor_qualification::run_evolved_population_qualification(
                    &options,
                    &workspace.path,
                    checkpoint,
                            production_source,
                )
                .map_err(std::io::Error::other)?,
            )?
            }
        } else if let Some(index) = &cohort {
            serde_json::to_value(
                slither_native::engine::sensor_qualification::run_sensor_cohort_qualification(
                    &options,
                    &workspace.path,
                    index,
                )
                .map_err(std::io::Error::other)?,
            )?
        } else {
            serde_json::to_value(
                run_sensor_qualification(&options, &workspace.path)
                    .map_err(std::io::Error::other)?,
            )?
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)?;
        serde_json::to_writer_pretty(&mut file, &report)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        println!(
            "admitted={} report={}",
            report
                .get("admitted")
                .or_else(|| report.get("complete"))
                .unwrap_or(&serde_json::Value::Bool(false)),
            output.display()
        );
        Ok(())
    })();
    let cleanup = workspace.cleanup();
    result?;
    cleanup?;
    Ok(())
}
