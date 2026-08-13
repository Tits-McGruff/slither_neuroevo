//! Deterministic Stage 4 corrected-sensing performance evidence.
//!
//! These source-shaped synthetic worlds exercise the production Rust sensor
//! evaluator and spatial indexes without claiming to be owner saves or a full
//! fixed step. The retained runner records index construction separately from
//! allocation-stable whole-population sensing.

use super::sensors::{
    SensorConfig, SensorEvaluator, SensorGenerationState, SensorSampleDiagnostics, SensorScratch,
    SensorScratchDiagnostics,
};
use super::spatial::{
    BodyIndexDiagnostics, IndexedSensorWorld, PelletIndexDiagnostics, SensorIndexConfig,
};
use super::state::{BodyRange, PelletState, SnakeKind, SnakeState, WorldPoint, WorldState};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::env;
#[cfg(target_os = "linux")]
use std::fs;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Version of the deterministic sensing-world generator.
pub const STAGE4_SENSING_FIXTURE_VERSION: u32 = 1;
/// Default collision/body spatial cell width.
const BODY_CELL_SIZE: f64 = 70.0;
/// Default pellet spatial cell width.
const PELLET_CELL_SIZE: f64 = 120.0;
/// Explicit complete body-entry ceiling for these fixtures.
const MAXIMUM_BODY_ENTRIES: usize = 2_000_000;
/// Explicit complete pellet-entry ceiling for these fixtures.
const MAXIMUM_PELLET_ENTRIES: usize = 25_000;
/// Current source-shaped baseline-bot count.
const BASELINE_COUNT: usize = 10;
/// Default short body-point count.
const DEFAULT_BODY_POINTS: usize = 5;
/// Stage 2 P4 body-point count.
const DENSE_BODY_POINTS: usize = 700;
/// Golden-angle fraction used by the deterministic disk layout.
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

/// Approved sensing-only workload name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage4SensingScenarioName {
    /// 55 evolved snakes, 10 baseline bots, 3,500 pellets, and 16 bins.
    P0,
    /// 300 evolved snakes, 10 baseline bots, 3,500 pellets, and 16 bins.
    P1,
    /// P0 world load with the supported large-brain 32-bin sensor layout.
    P2,
    /// Stage 2 P4-shaped 300+10 snake, 700-point-body, 12,000-pellet load.
    DenseBody,
    /// 55+10 short-body snakes and the configured maximum 25,000 pellets.
    DensePellet,
}

impl Stage4SensingScenarioName {
    /// Parse one stable public scenario label.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "P0" => Ok(Self::P0),
            "P1" => Ok(Self::P1),
            "P2" => Ok(Self::P2),
            "dense-body" => Ok(Self::DenseBody),
            "dense-pellet" => Ok(Self::DensePellet),
            _ => Err(format!(
                "scenario must be P0, P1, P2, dense-body, or dense-pellet; got {value}"
            )),
        }
    }

    /// Stable evidence label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::DenseBody => "dense-body",
            Self::DensePellet => "dense-pellet",
        }
    }

    /// Exact source-shaped workload dimensions.
    #[must_use]
    pub const fn spec(self) -> Stage4SensingScenarioSpec {
        match self {
            Self::P0 => Stage4SensingScenarioSpec {
                evolved_snakes: 55,
                baseline_snakes: BASELINE_COUNT,
                body_points_per_snake: DEFAULT_BODY_POINTS,
                pellets: 3_500,
                sensor_bins: 16,
                description: "55 evolved snakes, 10 baseline bots, five-point bodies, 3,500 pellets, v3/16-bin sensing",
            },
            Self::P1 => Stage4SensingScenarioSpec {
                evolved_snakes: 300,
                baseline_snakes: BASELINE_COUNT,
                body_points_per_snake: DEFAULT_BODY_POINTS,
                pellets: 3_500,
                sensor_bins: 16,
                description: "300 evolved snakes, 10 baseline bots, five-point bodies, 3,500 pellets, v3/16-bin sensing",
            },
            Self::P2 => Stage4SensingScenarioSpec {
                evolved_snakes: 55,
                baseline_snakes: BASELINE_COUNT,
                body_points_per_snake: DEFAULT_BODY_POINTS,
                pellets: 3_500,
                sensor_bins: 32,
                description: "55 evolved snakes, 10 baseline bots, five-point bodies, 3,500 pellets, v3/32-bin sensing",
            },
            Self::DenseBody => Stage4SensingScenarioSpec {
                evolved_snakes: 300,
                baseline_snakes: BASELINE_COUNT,
                body_points_per_snake: DENSE_BODY_POINTS,
                pellets: 12_000,
                sensor_bins: 16,
                description: "Stage 2 P4-shaped 300 evolved snakes, 10 baseline bots, 700-point bodies, 12,000 pellets, v3/16-bin sensing",
            },
            Self::DensePellet => Stage4SensingScenarioSpec {
                evolved_snakes: 55,
                baseline_snakes: BASELINE_COUNT,
                body_points_per_snake: DEFAULT_BODY_POINTS,
                pellets: 25_000,
                sensor_bins: 16,
                description: "55 evolved snakes, 10 baseline bots, five-point bodies, 25,000 pellets, v3/16-bin sensing",
            },
        }
    }
}

/// Exact logical dimensions for one synthetic sensing scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage4SensingScenarioSpec {
    /// Evolved population members.
    pub evolved_snakes: usize,
    /// Baseline bots.
    pub baseline_snakes: usize,
    /// Body points per live snake.
    pub body_points_per_snake: usize,
    /// Ambient pellets.
    pub pellets: usize,
    /// Sensor-v3 angular bins.
    pub sensor_bins: usize,
    /// Stable human-readable shape.
    pub description: &'static str,
}

impl Stage4SensingScenarioSpec {
    /// Total alive snakes sampled by a whole-population pass.
    #[must_use]
    pub const fn total_snakes(self) -> usize {
        self.evolved_snakes + self.baseline_snakes
    }
}

/// Operator-supplied benchmark controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage4SensingEvidenceOptions {
    /// Approved scenario.
    pub scenario: Stage4SensingScenarioName,
    /// Untimed index builds and sensing passes.
    pub warmup_passes: usize,
    /// Individually timed index builds and sensing passes.
    pub measured_passes: usize,
    /// Development or owner-target-vm provenance declaration.
    pub evidence_environment: String,
    /// Original executable arguments retained with the artifact.
    pub command: Vec<String>,
}

/// Complete corrected-sensing evidence document.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4SensingEvidence {
    /// Stable document family.
    pub schema: &'static str,
    /// Evidence schema version.
    pub version: u32,
    /// Honest measured-result classification.
    pub evidence_class: String,
    /// Important scope limitation.
    pub caveat: &'static str,
    /// Exact source/build identity.
    pub source: Stage4SensingSource,
    /// Target and process environment.
    pub environment: Stage4SensingEnvironment,
    /// Deterministic workload identity and dimensions.
    pub workload: Stage4SensingWorkload,
    /// Spatial-index construction evidence.
    pub indexes: Stage4IndexEvidence,
    /// Corrected whole-population sensing evidence.
    pub sensing: Stage4SensingResult,
    /// Original evidence command.
    pub command: Vec<String>,
}

/// Compiled native source identity.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4SensingSource {
    /// Source-derived native build identifier.
    pub native_build_identifier: String,
    /// Platform-independent native source digest.
    pub native_source_sha256: String,
    /// Build correctness-contract digest.
    pub native_build_contract_sha256: String,
    /// Exact Cargo target triple.
    pub target_triple: String,
    /// Exact compiler identity.
    pub rustc_version: String,
    /// Cargo profile.
    pub build_profile: String,
    /// Test-hook class proving the runner is absent from production builds.
    pub build_class: String,
}

/// Target and process facts captured with one run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4SensingEnvironment {
    /// Unix epoch milliseconds.
    pub captured_at_epoch_ms: u128,
    /// Operator provenance declaration.
    pub declaration: String,
    /// Rust compilation operating system.
    pub operating_system: &'static str,
    /// Rust compilation architecture.
    pub architecture: &'static str,
    /// Operating-system hostname when available.
    pub hostname: Option<String>,
    /// Logical parallelism visible to the process.
    pub available_parallelism: Option<usize>,
    /// Linux distribution identifier when available.
    pub distribution_id: Option<String>,
    /// First Linux CPU model when available.
    pub cpu_model: Option<String>,
    /// Linux physical-memory total when available.
    pub total_memory_bytes: Option<u64>,
    /// True only when all approved Oxygen identity checks pass.
    pub owner_target_vm_validated: bool,
    /// Resident set after deterministic world construction when available.
    pub fixture_rss_bytes: Option<u64>,
    /// Whole-process high-water resident set when available.
    pub process_peak_rss_bytes: Option<u64>,
    /// Resident set after all measurements when available.
    pub final_rss_bytes: Option<u64>,
}

/// Exact deterministic sensing-world shape.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4SensingWorkload {
    /// Numeric fixture version.
    pub fixture_version: u32,
    /// Honest fixture classification.
    pub fixture_class: &'static str,
    /// Stable scenario label.
    pub scenario: &'static str,
    /// Stable scenario description.
    pub description: &'static str,
    /// Evolved population members.
    pub evolved_snakes: usize,
    /// Baseline bots.
    pub baseline_snakes: usize,
    /// Total live snakes sampled per pass.
    pub total_snakes: usize,
    /// Body points per snake.
    pub body_points_per_snake: usize,
    /// Total body points.
    pub total_body_points: usize,
    /// Total logical body segments.
    pub total_body_segments: usize,
    /// Ambient pellets.
    pub pellets: usize,
    /// Sensor-v3 angular bins.
    pub sensor_bins: usize,
    /// Floats per observation.
    pub sensor_input_size: usize,
    /// Stable digest of authoritative fixture fields.
    pub world_sha256: String,
    /// Untimed complete passes.
    pub warmup_passes: usize,
    /// Timed complete passes.
    pub measured_passes: usize,
    /// This is not an owner save or evolved population.
    pub actual_fresh_or_evolved_world: bool,
    /// Every sample uses the corrected production Rust formula path.
    pub actual_corrected_sensor_observations: bool,
}

/// Body and pellet index build results.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4IndexEvidence {
    /// Body cell width.
    pub body_cell_size: f64,
    /// Pellet cell width.
    pub pellet_cell_size: f64,
    /// Complete body-entry admission ceiling.
    pub maximum_body_entries: usize,
    /// Complete pellet-entry admission ceiling.
    pub maximum_pellet_entries: usize,
    /// Exact body index diagnostics from the retained build.
    pub body: BodyIndexReport,
    /// Exact pellet index diagnostics from the retained build.
    pub pellets: PelletIndexReport,
    /// Full index-build latency distribution.
    pub whole_index_build_ms: Stage4SensingDistribution,
    /// Counted allocator operations per complete index build.
    pub allocator_operations_per_build: Stage4AllocationDistribution,
    /// Process CPU consumed by the measured index-build window.
    pub cpu: Stage4CpuUsage,
}

/// Serializable body-index counts.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyIndexReport {
    /// Unique body segments.
    pub segments: usize,
    /// Complete cell entries.
    pub entries: usize,
    /// Occupied cells.
    pub occupied_cells: usize,
    /// Estimated owned vector bytes.
    pub estimated_bytes: usize,
}

/// Serializable pellet-index counts.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PelletIndexReport {
    /// Indexed pellets.
    pub pellets: usize,
    /// Occupied cells.
    pub occupied_cells: usize,
    /// Estimated owned vector bytes.
    pub estimated_bytes: usize,
}

/// Timed sensing pass, bounded-work, and allocation evidence.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4SensingResult {
    /// Whole live-snake population sensing latency.
    pub whole_population_pass_ms: Stage4SensingDistribution,
    /// Mean microseconds per sampled live snake.
    pub mean_microseconds_per_snake: f64,
    /// Counted allocator operations per warmed sensing pass.
    pub allocator_operations_per_pass: Stage4AllocationDistribution,
    /// Process CPU consumed by the measured sensing window.
    pub cpu: Stage4CpuUsage,
    /// Effective pellet-detail cap applied to each sample.
    pub maximum_pellet_checks: usize,
    /// Effective body-segment-detail cap applied to each sample.
    pub maximum_segment_checks: usize,
    /// Reusable capacities immediately after warmup.
    pub scratch_after_warmup: SensorScratchReport,
    /// Reusable capacities after all measured passes.
    pub scratch_after_measurement: SensorScratchReport,
    /// True when no reusable scratch capacity changed during measurement.
    pub scratch_capacity_stable: bool,
    /// Work diagnostics from one untimed proof pass.
    pub proof_pass_work: SensorWorkReport,
    /// SHA-256 of every Float32 observation in the proof pass.
    pub observations_sha256: String,
    /// Finite output accumulator consumed across timed and proof passes.
    pub consumed_output: f64,
    /// Delivery markers are constructed for every real sample.
    pub delivery_markers_produced: bool,
    /// Pure benchmark samples do not advance authoritative score boundaries.
    pub delivery_markers_committed: bool,
}

/// Serializable sensor scratch capacities.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SensorScratchReport {
    /// Food-bin slots.
    pub food_bin_capacity: usize,
    /// Hazard-bin slots.
    pub hazard_bin_capacity: usize,
    /// Head-bin slots.
    pub head_bin_capacity: usize,
    /// Body duplicate-marker slots.
    pub body_duplicate_marker_capacity: usize,
    /// Bounded body candidate slots.
    pub body_candidate_capacity: usize,
    /// Bounded pellet candidate slots.
    pub pellet_candidate_capacity: usize,
}

impl From<SensorScratchDiagnostics> for SensorScratchReport {
    fn from(source: SensorScratchDiagnostics) -> Self {
        Self {
            food_bin_capacity: source.food_bin_capacity,
            hazard_bin_capacity: source.hazard_bin_capacity,
            head_bin_capacity: source.head_bin_capacity,
            body_duplicate_marker_capacity: source.body_duplicate_marker_capacity,
            body_candidate_capacity: source.body_candidate_capacity,
            pellet_candidate_capacity: source.pellet_candidate_capacity,
        }
    }
}

/// Aggregated spatial and cap work from one complete proof pass.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorWorkReport {
    /// Samples evaluated.
    pub samples: usize,
    /// Pellet cells visited.
    pub pellet_cells_visited: usize,
    /// Pellet entries visited.
    pub pellet_entries_visited: usize,
    /// Pellet candidates retained and evaluated.
    pub pellet_checks: usize,
    /// Pellet candidates retained by the bounded spatial query.
    pub pellet_candidates_retained: usize,
    /// Pellet-cap hits.
    pub pellet_cap_hits: u64,
    /// Body cells visited.
    pub body_cells_visited: usize,
    /// Body entries visited.
    pub body_entries_visited: usize,
    /// Body candidates evaluated after an uncapped query.
    pub segment_checks: usize,
    /// Body candidates retained by the bounded spatial query.
    pub body_candidates_retained: usize,
    /// Body-cap hits.
    pub segment_cap_hits: u64,
    /// Samples conservatively saturated because of body caps.
    pub conservative_body_saturations: usize,
    /// Other heads checked.
    pub head_checks: usize,
    /// Samples whose head was outside the configured world.
    pub targets_outside_world: usize,
}

impl SensorWorkReport {
    fn add(&mut self, diagnostics: SensorSampleDiagnostics) {
        self.samples = self.samples.saturating_add(1);
        self.pellet_cells_visited = self
            .pellet_cells_visited
            .saturating_add(diagnostics.pellet_query.cells_visited);
        self.pellet_entries_visited = self
            .pellet_entries_visited
            .saturating_add(diagnostics.pellet_query.entries_visited);
        self.pellet_checks = self.pellet_checks.saturating_add(diagnostics.pellet_checks);
        self.pellet_candidates_retained = self
            .pellet_candidates_retained
            .saturating_add(diagnostics.pellet_query.candidates);
        self.pellet_cap_hits = self
            .pellet_cap_hits
            .saturating_add(diagnostics.pellet_cap_hits);
        self.body_cells_visited = self
            .body_cells_visited
            .saturating_add(diagnostics.body_query.cells_visited);
        self.body_entries_visited = self
            .body_entries_visited
            .saturating_add(diagnostics.body_query.entries_visited);
        self.segment_checks = self
            .segment_checks
            .saturating_add(diagnostics.segment_checks);
        self.body_candidates_retained = self
            .body_candidates_retained
            .saturating_add(diagnostics.body_query.candidates);
        self.segment_cap_hits = self
            .segment_cap_hits
            .saturating_add(diagnostics.segment_cap_hits);
        self.conservative_body_saturations = self
            .conservative_body_saturations
            .saturating_add(usize::from(diagnostics.conservative_body_saturation));
        self.head_checks = self.head_checks.saturating_add(diagnostics.head_checks);
        self.targets_outside_world = self
            .targets_outside_world
            .saturating_add(usize::from(diagnostics.target_outside_world));
    }
}

/// Millisecond distribution for independently timed passes.
#[derive(Debug, Serialize)]
pub struct Stage4SensingDistribution {
    /// Number of samples.
    pub count: usize,
    /// Minimum.
    pub min: f64,
    /// Median.
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
    /// Maximum.
    pub max: f64,
    /// Arithmetic mean.
    pub mean: f64,
}

/// Integer distribution for counted allocator operations.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4AllocationDistribution {
    /// Number of measured passes.
    pub count: usize,
    /// Minimum allocator calls in one pass.
    pub min: u64,
    /// Maximum allocator calls in one pass.
    pub max: u64,
    /// Arithmetic mean allocator calls per pass.
    pub mean: f64,
    /// Number of passes with at least one allocator call.
    pub nonzero_passes: usize,
}

/// Whole-process CPU use across one measured benchmark window.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4CpuUsage {
    /// Operating-system counter used to obtain process CPU time.
    pub source: Option<&'static str>,
    /// Wall-clock duration of the complete measured window.
    pub wall_seconds: f64,
    /// User plus kernel process CPU consumed during the window.
    pub process_cpu_seconds: Option<f64>,
    /// Average use expressed as a percentage of one logical CPU.
    pub average_one_core_utilization_percent: Option<f64>,
}

/// Monotonic operating-system process CPU counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessCpuSnapshot {
    nanoseconds: u128,
    source: &'static str,
}

/// Run one deterministic corrected-sensing benchmark.
///
/// `allocation_snapshot` must return a monotonic process allocation-operation
/// counter. The standalone evidence binary supplies the counter; tests may
/// supply a deterministic stub when allocator counts are not under assertion.
pub fn run_stage4_sensing_evidence(
    options: Stage4SensingEvidenceOptions,
    allocation_snapshot: impl Fn() -> u64,
) -> Result<Stage4SensingEvidence, String> {
    validate_options(&options)?;
    let target_triple = crate::native_addon_build_target();
    let build_profile = crate::native_addon_build_profile();
    let build_class = crate::native_addon_build_class();
    if build_profile != "release" {
        return Err("Stage 4 sensing evidence requires a release build".to_owned());
    }
    if build_class != "test-hooks" {
        return Err("Stage 4 sensing evidence requires a test-hooks build".to_owned());
    }
    if target_triple != "x86_64-pc-windows-msvc" && target_triple != "x86_64-unknown-linux-gnu" {
        return Err(format!(
            "Stage 4 sensing evidence does not support target {target_triple}"
        ));
    }

    let hostname = system_hostname();
    let available_parallelism = std::thread::available_parallelism().ok().map(usize::from);
    let distribution_id = linux_os_release_value("ID");
    let cpu_model = system_cpu_model();
    let total_memory_bytes = linux_total_memory_bytes();
    let owner_target_vm_validated = env::consts::OS == "linux"
        && distribution_id.as_deref() == Some("debian")
        && hostname.as_deref() == Some("oxygen")
        && cpu_model
            .as_deref()
            .is_some_and(|model| model.contains("AMD Ryzen 7 2700"))
        && available_parallelism == Some(8)
        && total_memory_bytes.is_some_and(|bytes| {
            ((15_u64 * 1024 * 1024 * 1024)..=(17_u64 * 1024 * 1024 * 1024)).contains(&bytes)
        });
    if options.evidence_environment == "owner-target-vm" && !owner_target_vm_validated {
        return Err("owner-target-vm was declared, but the Debian/Oxygen/Ryzen-2700/8-thread/16-GiB identity checks did not all pass".to_owned());
    }

    let spec = options.scenario.spec();
    let world = build_world(spec)?;
    let world_sha256 = digest_world(&world);
    let fixture_rss_bytes = linux_process_status_bytes("VmRSS:");
    let index_config = SensorIndexConfig {
        body_cell_size: BODY_CELL_SIZE,
        pellet_cell_size: PELLET_CELL_SIZE,
        maximum_body_entries: MAXIMUM_BODY_ENTRIES,
        maximum_pellet_entries: MAXIMUM_PELLET_ENTRIES,
    };
    let sensor_config = SensorConfig {
        bins: spec.sensor_bins,
        ..SensorConfig::default()
    };
    let evaluator = SensorEvaluator::new(sensor_config)
        .map_err(|error| format!("sensor evaluator construction failed: {error}"))?;
    let mut generation = SensorGenerationState::new();
    generation
        .update_after_step(&world)
        .map_err(|error| format!("generation sensor initialization failed: {error}"))?;

    for _ in 0..options.warmup_passes {
        let indexed = IndexedSensorWorld::build(&world, index_config)
            .map_err(|error| format!("warmup index build failed: {error}"))?;
        std::hint::black_box(indexed.body_index().diagnostics().entries);
    }
    let mut index_samples = Vec::new();
    index_samples
        .try_reserve_exact(options.measured_passes)
        .map_err(|_| "index timing allocation failed".to_owned())?;
    let mut index_allocations = Vec::new();
    index_allocations
        .try_reserve_exact(options.measured_passes)
        .map_err(|_| "index allocation-sample reservation failed".to_owned())?;
    let mut retained_body = None;
    let mut retained_pellets = None;
    let index_cpu_before = process_cpu_snapshot();
    let index_window_started = Instant::now();
    for _ in 0..options.measured_passes {
        let allocations_before = allocation_snapshot();
        let started = Instant::now();
        let indexed = IndexedSensorWorld::build(&world, index_config)
            .map_err(|error| format!("measured index build failed: {error}"))?;
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        let allocations_after = allocation_snapshot();
        retained_body = Some(indexed.body_index().diagnostics());
        retained_pellets = Some(indexed.pellet_index().diagnostics());
        std::hint::black_box(indexed.body_index().diagnostics().estimated_bytes);
        index_samples.push(elapsed);
        index_allocations.push(allocations_after.saturating_sub(allocations_before));
    }
    let index_cpu_after = process_cpu_snapshot();
    let index_window_elapsed = index_window_started.elapsed();
    let index_cpu = cpu_usage(index_cpu_before, index_cpu_after, index_window_elapsed);
    let body_diagnostics = retained_body.ok_or_else(|| "body diagnostics are absent".to_owned())?;
    let pellet_diagnostics =
        retained_pellets.ok_or_else(|| "pellet diagnostics are absent".to_owned())?;

    let indexed = IndexedSensorWorld::build(&world, index_config)
        .map_err(|error| format!("retained sensor index build failed: {error}"))?;
    let mut scratch = SensorScratch::default();
    let mut output = vec![0.0_f32; evaluator.layout().input_size];
    let mut consumed_output = 0.0_f64;
    for _ in 0..options.warmup_passes {
        consumed_output += execute_sensor_pass(
            &evaluator,
            &indexed,
            &generation,
            &mut output,
            &mut scratch,
            None,
            None,
        )?;
    }
    let scratch_after_warmup = SensorScratchReport::from(scratch.diagnostics());
    let mut sensing_samples = Vec::new();
    sensing_samples
        .try_reserve_exact(options.measured_passes)
        .map_err(|_| "sensing timing allocation failed".to_owned())?;
    let mut sensing_allocations = Vec::new();
    sensing_allocations
        .try_reserve_exact(options.measured_passes)
        .map_err(|_| "sensing allocation-sample reservation failed".to_owned())?;
    let sensing_cpu_before = process_cpu_snapshot();
    let sensing_window_started = Instant::now();
    for _ in 0..options.measured_passes {
        let allocations_before = allocation_snapshot();
        let started = Instant::now();
        consumed_output += execute_sensor_pass(
            &evaluator,
            &indexed,
            &generation,
            &mut output,
            &mut scratch,
            None,
            None,
        )?;
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        let allocations_after = allocation_snapshot();
        sensing_samples.push(elapsed);
        sensing_allocations.push(allocations_after.saturating_sub(allocations_before));
    }
    let sensing_cpu_after = process_cpu_snapshot();
    let sensing_window_elapsed = sensing_window_started.elapsed();
    let sensing_cpu = cpu_usage(
        sensing_cpu_before,
        sensing_cpu_after,
        sensing_window_elapsed,
    );
    if options.evidence_environment == "owner-target-vm"
        && (index_cpu.process_cpu_seconds.is_none() || sensing_cpu.process_cpu_seconds.is_none())
    {
        return Err(
            "owner-target-vm CPU evidence could not be sampled for every measured window"
                .to_owned(),
        );
    }
    let scratch_after_measurement = SensorScratchReport::from(scratch.diagnostics());
    let scratch_capacity_stable = scratch_after_warmup == scratch_after_measurement;
    if !scratch_capacity_stable {
        return Err("sensor scratch capacity changed after warmup".to_owned());
    }

    let mut proof_work = SensorWorkReport::default();
    let mut observation_hasher = Sha256::new();
    consumed_output += execute_sensor_pass(
        &evaluator,
        &indexed,
        &generation,
        &mut output,
        &mut scratch,
        Some(&mut proof_work),
        Some(&mut observation_hasher),
    )?;
    if proof_work.samples != spec.total_snakes() {
        return Err("proof pass did not sample every live fixture snake".to_owned());
    }
    let sensing_distribution = distribution(sensing_samples)?;
    let mean_microseconds_per_snake =
        sensing_distribution.mean * 1_000.0 / spec.total_snakes() as f64;
    let total_body_points = spec
        .total_snakes()
        .checked_mul(spec.body_points_per_snake)
        .ok_or_else(|| "body-point count overflowed".to_owned())?;
    let total_body_segments = spec
        .total_snakes()
        .checked_mul(spec.body_points_per_snake.saturating_sub(1))
        .ok_or_else(|| "body-segment count overflowed".to_owned())?;
    let evidence_class = if options.evidence_environment == "owner-target-vm" {
        "new measured target-VM corrected Rust sensing result"
    } else {
        "new measured development-machine corrected Rust sensing result"
    };

    Ok(Stage4SensingEvidence {
        schema: "slither-stage4-rust-sensing-benchmark",
        version: 1,
        evidence_class: evidence_class.to_owned(),
        caveat: "Source-shaped deterministic synthetic sensing-only benchmark. It exercises corrected Rust sensor formulas and complete Rust indexes, but it is not an owner save, evolved population, complete brain pass, movement/collision step, frame, Node bridge, browser, RL client, or final real-time acceptance result.",
        source: Stage4SensingSource {
            native_build_identifier: crate::native_addon_build_identifier(),
            native_source_sha256: crate::native_addon_source_sha256(),
            native_build_contract_sha256: crate::native_addon_build_contract_sha256(),
            target_triple,
            rustc_version: crate::native_addon_rustc_version(),
            build_profile,
            build_class,
        },
        environment: Stage4SensingEnvironment {
            captured_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
                .as_millis(),
            declaration: options.evidence_environment,
            operating_system: env::consts::OS,
            architecture: env::consts::ARCH,
            hostname,
            available_parallelism,
            distribution_id,
            cpu_model,
            total_memory_bytes,
            owner_target_vm_validated,
            fixture_rss_bytes,
            process_peak_rss_bytes: linux_process_status_bytes("VmHWM:"),
            final_rss_bytes: linux_process_status_bytes("VmRSS:"),
        },
        workload: Stage4SensingWorkload {
            fixture_version: STAGE4_SENSING_FIXTURE_VERSION,
            fixture_class: "source-shaped deterministic synthetic world",
            scenario: options.scenario.label(),
            description: spec.description,
            evolved_snakes: spec.evolved_snakes,
            baseline_snakes: spec.baseline_snakes,
            total_snakes: spec.total_snakes(),
            body_points_per_snake: spec.body_points_per_snake,
            total_body_points,
            total_body_segments,
            pellets: spec.pellets,
            sensor_bins: spec.sensor_bins,
            sensor_input_size: evaluator.layout().input_size,
            world_sha256,
            warmup_passes: options.warmup_passes,
            measured_passes: options.measured_passes,
            actual_fresh_or_evolved_world: false,
            actual_corrected_sensor_observations: true,
        },
        indexes: Stage4IndexEvidence {
            body_cell_size: BODY_CELL_SIZE,
            pellet_cell_size: PELLET_CELL_SIZE,
            maximum_body_entries: MAXIMUM_BODY_ENTRIES,
            maximum_pellet_entries: MAXIMUM_PELLET_ENTRIES,
            body: BodyIndexReport::from(body_diagnostics),
            pellets: PelletIndexReport::from(pellet_diagnostics),
            whole_index_build_ms: distribution(index_samples)?,
            allocator_operations_per_build: allocation_distribution(&index_allocations)?,
            cpu: index_cpu,
        },
        sensing: Stage4SensingResult {
            whole_population_pass_ms: sensing_distribution,
            mean_microseconds_per_snake,
            allocator_operations_per_pass: allocation_distribution(&sensing_allocations)?,
            cpu: sensing_cpu,
            maximum_pellet_checks: evaluator.effective_pellet_limit(),
            maximum_segment_checks: evaluator.effective_segment_limit(),
            scratch_after_warmup,
            scratch_after_measurement,
            scratch_capacity_stable,
            proof_pass_work: proof_work,
            observations_sha256: hex_bytes(observation_hasher.finalize()),
            consumed_output,
            delivery_markers_produced: true,
            delivery_markers_committed: false,
        },
        command: options.command,
    })
}

impl From<BodyIndexDiagnostics> for BodyIndexReport {
    fn from(source: BodyIndexDiagnostics) -> Self {
        Self {
            segments: source.segments,
            entries: source.entries,
            occupied_cells: source.occupied_cells,
            estimated_bytes: source.estimated_bytes,
        }
    }
}

impl From<PelletIndexDiagnostics> for PelletIndexReport {
    fn from(source: PelletIndexDiagnostics) -> Self {
        Self {
            pellets: source.pellets,
            occupied_cells: source.occupied_cells,
            estimated_bytes: source.estimated_bytes,
        }
    }
}

fn validate_options(options: &Stage4SensingEvidenceOptions) -> Result<(), String> {
    if options.measured_passes == 0 || options.measured_passes > 100_000 {
        return Err("measured passes must be from 1 to 100000".to_owned());
    }
    if options.warmup_passes > 100_000 {
        return Err("warmup passes cannot exceed 100000".to_owned());
    }
    if options.evidence_environment != "development"
        && options.evidence_environment != "owner-target-vm"
    {
        return Err("environment must be development or owner-target-vm".to_owned());
    }
    Ok(())
}

fn build_world(spec: Stage4SensingScenarioSpec) -> Result<WorldState, String> {
    let total_snakes = spec.total_snakes();
    let total_body_points = total_snakes
        .checked_mul(spec.body_points_per_snake)
        .ok_or_else(|| "fixture body-point count overflowed".to_owned())?;
    let mut world = WorldState::default();
    world
        .snakes
        .try_reserve_exact(total_snakes)
        .map_err(|_| "fixture snake allocation failed".to_owned())?;
    world
        .body_points
        .try_reserve_exact(total_body_points)
        .map_err(|_| "fixture body allocation failed".to_owned())?;
    world
        .pellets
        .try_reserve_exact(spec.pellets)
        .map_err(|_| "fixture pellet allocation failed".to_owned())?;

    for snake_index in 0..total_snakes {
        let head = disk_point(snake_index, total_snakes, 2_650.0, 0.25);
        let direction = normalize_angle(snake_index as f64 * 0.618_033_988_749_894_8);
        let body_start = world.body_points.len();
        for point_index in 0..spec.body_points_per_snake {
            let point = if spec.body_points_per_snake == DENSE_BODY_POINTS {
                let angle = direction + point_index as f64 * 0.025;
                let radius = (point_index as f64 * 0.75).min(600.0);
                WorldPoint {
                    x: head.x - angle.cos() * radius,
                    y: head.y - angle.sin() * radius,
                }
            } else {
                let distance = point_index as f64 * 7.5;
                let angle = direction + point_index as f64 * 0.01;
                WorldPoint {
                    x: head.x - angle.cos() * distance,
                    y: head.y - angle.sin() * distance,
                }
            };
            world.body_points.push(point);
        }
        world.body_points[body_start] = head;
        let id =
            u64::try_from(snake_index + 1).map_err(|_| "fixture snake ID overflowed".to_owned())?;
        let kind = if snake_index < spec.evolved_snakes {
            SnakeKind::Evolved
        } else {
            SnakeKind::Baseline
        };
        let population_slot = if kind == SnakeKind::Evolved {
            Some(
                u32::try_from(snake_index)
                    .map_err(|_| "fixture population slot overflowed".to_owned())?,
            )
        } else {
            None
        };
        let baseline_slot = if kind == SnakeKind::Baseline {
            Some(
                u32::try_from(snake_index - spec.evolved_snakes)
                    .map_err(|_| "fixture baseline slot overflowed".to_owned())?,
            )
        } else {
            None
        };
        let points = (snake_index % 41) as f64 * 0.75;
        world.snakes.push(SnakeState {
            id,
            frame_v1_id: u32::try_from(id).map_err(|_| "frame ID overflowed".to_owned())?,
            kind,
            alive: true,
            population_slot,
            brain: None,
            baseline_slot,
            baseline_strategy: None,
            position: head,
            previous_position: head,
            direction,
            radius: 9.0 + (snake_index % 10) as f64 * 0.4,
            speed: 165.0,
            boost: snake_index % 7 == 0,
            age_seconds: (snake_index % 180) as f64,
            food: points,
            points,
            kills: u64::try_from(snake_index % 5)
                .map_err(|_| "fixture kill count overflowed".to_owned())?,
            target_length: spec.body_points_per_snake as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: (points - (snake_index % 4) as f64 * 0.5).max(0.0),
            body: BodyRange {
                start: body_start,
                len: spec.body_points_per_snake,
            },
            skin: u32::try_from(snake_index % 8)
                .map_err(|_| "fixture skin overflowed".to_owned())?,
        });
    }

    for pellet_index in 0..spec.pellets {
        let position = disk_point(pellet_index, spec.pellets, 3_300.0, 1.75);
        let id = u64::try_from(total_snakes + pellet_index + 1)
            .map_err(|_| "fixture pellet ID overflowed".to_owned())?;
        world.pellets.push(PelletState {
            id,
            position,
            value: 0.5 + (pellet_index % 9) as f64 * 0.25,
            kind: u32::try_from(pellet_index % 3)
                .map_err(|_| "fixture pellet kind overflowed".to_owned())?,
            color: u32::try_from(pellet_index % 8)
                .map_err(|_| "fixture pellet color overflowed".to_owned())?,
            owner: None,
        });
    }
    Ok(world)
}

fn disk_point(index: usize, count: usize, radius: f64, phase: f64) -> WorldPoint {
    let fraction = (index as f64 + 0.5) / count.max(1) as f64;
    let radial = fraction.sqrt() * radius;
    let angle = index as f64 * GOLDEN_ANGLE + phase;
    WorldPoint {
        x: angle.cos() * radial,
        y: angle.sin() * radial,
    }
}

fn normalize_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

#[allow(clippy::too_many_arguments)]
fn execute_sensor_pass(
    evaluator: &SensorEvaluator,
    indexed: &IndexedSensorWorld<'_>,
    generation: &SensorGenerationState,
    output: &mut [f32],
    scratch: &mut SensorScratch,
    mut work: Option<&mut SensorWorkReport>,
    mut hasher: Option<&mut Sha256>,
) -> Result<f64, String> {
    let mut consumed = 0.0_f64;
    for snake_index in 0..indexed.world().snakes.len() {
        let sample = evaluator
            .sample(indexed, generation, snake_index, output, scratch)
            .map_err(|error| format!("sensor sample {snake_index} failed: {error}"))?;
        consumed += output.iter().map(|value| f64::from(*value)).sum::<f64>();
        consumed += sample.delivery.sampled_points * 1.0e-12;
        if let Some(report) = work.as_deref_mut() {
            report.add(sample.diagnostics);
        }
        if let Some(digest) = hasher.as_deref_mut() {
            for value in output.iter().copied() {
                digest.update(value.to_bits().to_le_bytes());
            }
        }
    }
    if !consumed.is_finite() {
        return Err("sensor output accumulator is not finite".to_owned());
    }
    Ok(consumed)
}

fn digest_world(world: &WorldState) -> String {
    let mut digest = Sha256::new();
    digest.update(b"slither-stage4-sensing-world-v1\0");
    digest.update((world.snakes.len() as u64).to_le_bytes());
    digest.update((world.body_points.len() as u64).to_le_bytes());
    digest.update((world.pellets.len() as u64).to_le_bytes());
    for snake in &world.snakes {
        digest.update(snake.id.to_le_bytes());
        digest.update([match snake.kind {
            SnakeKind::Evolved => 0,
            SnakeKind::Baseline => 1,
            SnakeKind::External => 2,
            SnakeKind::Resurrected => 3,
        }]);
        digest.update([u8::from(snake.alive), u8::from(snake.boost)]);
        for value in [
            snake.position.x,
            snake.position.y,
            snake.direction,
            snake.radius,
            snake.speed,
            snake.age_seconds,
            snake.points,
            snake.delivered_observation_points,
        ] {
            digest.update(value.to_bits().to_le_bytes());
        }
        digest.update((snake.body.start as u64).to_le_bytes());
        digest.update((snake.body.len as u64).to_le_bytes());
    }
    for point in &world.body_points {
        digest.update(point.x.to_bits().to_le_bytes());
        digest.update(point.y.to_bits().to_le_bytes());
    }
    for pellet in &world.pellets {
        digest.update(pellet.id.to_le_bytes());
        digest.update(pellet.position.x.to_bits().to_le_bytes());
        digest.update(pellet.position.y.to_bits().to_le_bytes());
        digest.update(pellet.value.to_bits().to_le_bytes());
    }
    hex_bytes(digest.finalize())
}

fn distribution(mut samples: Vec<f64>) -> Result<Stage4SensingDistribution, String> {
    if samples.is_empty()
        || samples
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("timing samples must be nonempty, finite, and non-negative".to_owned());
    }
    samples.sort_unstable_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let percentile = |fraction: f64| {
        let position = fraction * (samples.len() - 1) as f64;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        let weight = position - lower as f64;
        samples[lower] * (1.0 - weight) + samples[upper] * weight
    };
    Ok(Stage4SensingDistribution {
        count: samples.len(),
        min: samples[0],
        p50: percentile(0.5),
        p95: percentile(0.95),
        p99: percentile(0.99),
        max: samples[samples.len() - 1],
        mean,
    })
}

fn allocation_distribution(samples: &[u64]) -> Result<Stage4AllocationDistribution, String> {
    let Some(min) = samples.iter().copied().min() else {
        return Err("allocation samples are empty".to_owned());
    };
    let max = samples.iter().copied().max().unwrap_or(min);
    let total = samples.iter().try_fold(0_u128, |sum, value| {
        sum.checked_add(u128::from(*value))
            .ok_or_else(|| "allocation sample sum overflowed".to_owned())
    })?;
    Ok(Stage4AllocationDistribution {
        count: samples.len(),
        min,
        max,
        mean: total as f64 / samples.len() as f64,
        nonzero_passes: samples.iter().filter(|value| **value != 0).count(),
    })
}

fn cpu_usage(
    before: Option<ProcessCpuSnapshot>,
    after: Option<ProcessCpuSnapshot>,
    wall: Duration,
) -> Stage4CpuUsage {
    let wall_seconds = wall.as_secs_f64();
    let measured = before.zip(after).and_then(|(before, after)| {
        if before.source != after.source || after.nanoseconds < before.nanoseconds {
            return None;
        }
        let process_cpu_seconds = (after.nanoseconds - before.nanoseconds) as f64 / 1_000_000_000.0;
        let utilization =
            (wall_seconds > 0.0).then_some(process_cpu_seconds / wall_seconds * 100.0);
        Some((before.source, process_cpu_seconds, utilization))
    });
    Stage4CpuUsage {
        source: measured.map(|(source, _, _)| source),
        wall_seconds,
        process_cpu_seconds: measured.map(|(_, seconds, _)| seconds),
        average_one_core_utilization_percent: measured.and_then(|(_, _, utilization)| utilization),
    }
}

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LinuxTimespec {
    seconds: i64,
    nanoseconds: i64,
}

#[cfg(target_os = "linux")]
extern "C" {
    fn clock_gettime(clock_id: i32, time: *mut LinuxTimespec) -> i32;
}

#[cfg(target_os = "linux")]
fn process_cpu_snapshot() -> Option<ProcessCpuSnapshot> {
    const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;
    let mut time = LinuxTimespec::default();
    // SAFETY: `time` is a live writable value with the x86_64 Linux
    // `timespec` C layout. The selected standard process CPU clock requires no
    // additional lifetime or ownership contract.
    let succeeded = unsafe { clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &mut time) };
    if succeeded != 0 || time.seconds < 0 || !(0..1_000_000_000).contains(&time.nanoseconds) {
        return None;
    }
    let nanoseconds = u128::try_from(time.seconds)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u128::try_from(time.nanoseconds).ok()?)?;
    Some(ProcessCpuSnapshot {
        nanoseconds,
        source: "Linux clock_gettime(CLOCK_PROCESS_CPUTIME_ID)",
    })
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WindowsFileTime {
    low_date_time: u32,
    high_date_time: u32,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn GetProcessTimes(
        process: isize,
        creation_time: *mut WindowsFileTime,
        exit_time: *mut WindowsFileTime,
        kernel_time: *mut WindowsFileTime,
        user_time: *mut WindowsFileTime,
    ) -> i32;
}

#[cfg(target_os = "windows")]
fn windows_file_time_ticks(value: WindowsFileTime) -> u64 {
    (u64::from(value.high_date_time) << 32) | u64::from(value.low_date_time)
}

#[cfg(target_os = "windows")]
fn process_cpu_snapshot() -> Option<ProcessCpuSnapshot> {
    let mut creation_time = WindowsFileTime::default();
    let mut exit_time = WindowsFileTime::default();
    let mut kernel_time = WindowsFileTime::default();
    let mut user_time = WindowsFileTime::default();
    // SAFETY: `GetCurrentProcess` returns the caller's always-valid pseudo
    // handle. Every `GetProcessTimes` output points to a live, writable
    // `WindowsFileTime` with the required C layout for the duration of the call.
    let succeeded = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };
    if succeeded == 0 {
        return None;
    }
    let ticks_100ns =
        windows_file_time_ticks(kernel_time).checked_add(windows_file_time_ticks(user_time))?;
    Some(ProcessCpuSnapshot {
        nanoseconds: u128::from(ticks_100ns).checked_mul(100)?,
        source: "Windows GetProcessTimes",
    })
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn process_cpu_snapshot() -> Option<ProcessCpuSnapshot> {
    None
}

fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut text, "{byte:02x}").expect("String formatting cannot fail");
    }
    text
}

fn system_hostname() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(raw_hostname) = fs::read_to_string("/proc/sys/kernel/hostname") {
            let hostname = raw_hostname.trim();
            if !hostname.is_empty() {
                return Some(hostname.to_owned());
            }
        }
    }
    env::var("COMPUTERNAME")
        .or_else(|_| env::var("HOSTNAME"))
        .ok()
        .map(|hostname| hostname.trim().to_owned())
        .filter(|hostname| !hostname.is_empty())
}

fn linux_os_release_value(key: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let document = fs::read_to_string("/etc/os-release").ok()?;
        for line in document.lines() {
            if let Some(value) = line.strip_prefix(&format!("{key}=")) {
                return Some(value.trim_matches('"').to_owned());
            }
        }
    }
    let _ = key;
    None
}

#[cfg(target_os = "linux")]
fn system_cpu_model() -> Option<String> {
    let document = fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in document.lines() {
        if let Some(value) = line.strip_prefix("model name") {
            return value
                .split_once(':')
                .map(|(_, model)| model.trim().to_owned());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn system_cpu_model() -> Option<String> {
    env::var("PROCESSOR_IDENTIFIER")
        .ok()
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn system_cpu_model() -> Option<String> {
    None
}

fn linux_total_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let document = fs::read_to_string("/proc/meminfo").ok()?;
        let line = document
            .lines()
            .find(|line| line.starts_with("MemTotal:"))?;
        let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        return kib.checked_mul(1024);
    }
    #[cfg(not(target_os = "linux"))]
    None
}

fn linux_process_status_bytes(label: &str) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let document = fs::read_to_string("/proc/self/status").ok()?;
        let line = document.lines().find(|line| line.starts_with(label))?;
        let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        return kib.checked_mul(1024);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = label;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_scenario_shapes_are_exact() {
        assert_eq!(Stage4SensingScenarioName::P0.spec().total_snakes(), 65);
        assert_eq!(Stage4SensingScenarioName::P1.spec().total_snakes(), 310);
        assert_eq!(Stage4SensingScenarioName::P2.spec().sensor_bins, 32);
        assert_eq!(
            Stage4SensingScenarioName::DenseBody
                .spec()
                .body_points_per_snake,
            700
        );
        assert_eq!(
            Stage4SensingScenarioName::DensePellet.spec().pellets,
            25_000
        );
    }

    #[test]
    fn p0_evidence_runs_corrected_real_sensor_path() {
        let report = run_stage4_sensing_evidence(
            Stage4SensingEvidenceOptions {
                scenario: Stage4SensingScenarioName::P0,
                warmup_passes: 1,
                measured_passes: 2,
                evidence_environment: "development".to_owned(),
                command: vec!["test".to_owned()],
            },
            || 0,
        )
        .expect("P0 evidence should run");
        assert_eq!(report.workload.total_snakes, 65);
        assert_eq!(report.sensing.proof_pass_work.samples, 65);
        assert!(report.sensing.scratch_capacity_stable);
        assert_eq!(report.sensing.observations_sha256.len(), 64);
        assert_eq!(report.indexes.body.segments, 65 * 4);
        assert_eq!(report.indexes.pellets.pellets, 3_500);
        assert_eq!(report.sensing.maximum_pellet_checks, 900);
        assert_eq!(report.sensing.maximum_segment_checks, 2_200);
        assert_eq!(
            report.sensing.proof_pass_work.pellet_candidates_retained,
            report.sensing.proof_pass_work.pellet_checks
        );
    }

    #[test]
    fn cpu_usage_reports_process_time_and_one_core_percentage() {
        let report = cpu_usage(
            Some(ProcessCpuSnapshot {
                nanoseconds: 100,
                source: "test counter",
            }),
            Some(ProcessCpuSnapshot {
                nanoseconds: 400,
                source: "test counter",
            }),
            Duration::from_nanos(600),
        );
        assert_eq!(report.source, Some("test counter"));
        assert_eq!(report.process_cpu_seconds, Some(0.000_000_3));
        assert_eq!(report.average_one_core_utilization_percent, Some(50.0));
    }
}
