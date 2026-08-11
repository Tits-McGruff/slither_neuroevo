//! Deterministic Stage 4 whole-population inference performance evidence.
//!
//! This test-hook-only module uses the production graph compiler, scalar executor,
//! heterogeneous weight/state resolution, staging, and recurrent commit. Its numeric
//! generator is mirrored by scripts/stage4/inferenceFixture.ts so Rust and the
//! current TypeScript paths can report matching input digests without checking large
//! fixtures into Git.

use super::calculation::{
    CalculationBatchKey, CalculationExecutionBuffers, CalculationScratch, CalculationWorkUnit,
    CalculationWorkspace,
};
use super::graph::{
    compile_graph, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
};
use super::inference::{
    commit_heterogeneous_recurrent, evaluate_heterogeneous_population, GraphExecutionPlan,
    HeterogeneousInferenceBuffers,
};
use super::state::{
    BodyRange, BrainHandle, BrainOwner, BrainRuntimeState, GenomeLineage, PopulationGenome,
    SnakeKind, SnakeState, WorldPoint,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::mem::size_of;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Version shared with the current TypeScript fixture generator.
pub const STAGE4_INFERENCE_FIXTURE_VERSION: u32 = 1;
/// Population/brain epoch used by the isolated evidence fixture.
const FIXTURE_EPOCH: u64 = 1;

/// Approved inference-only workload name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage4InferenceScenarioName {
    /// 55 synthetic population brains with the default graph.
    P0,
    /// 300 synthetic population brains with the default graph.
    P1,
    /// 55 synthetic population brains with the large graph.
    P2,
    /// 300 synthetic population brains with the large graph.
    P3,
}

impl Stage4InferenceScenarioName {
    /// Parse the exact public workload label.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "P0" => Ok(Self::P0),
            "P1" => Ok(Self::P1),
            "P2" => Ok(Self::P2),
            "P3" => Ok(Self::P3),
            _ => Err(format!("scenario must be P0, P1, P2, or P3; got {value}")),
        }
    }

    /// Stable evidence label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }

    /// Whether this scenario uses the source-shaped large graph.
    const fn is_large(self) -> bool {
        matches!(self, Self::P2 | Self::P3)
    }

    /// Number of differently weighted synthetic population brains in one due pass.
    const fn population_count(self) -> usize {
        if matches!(self, Self::P1 | Self::P3) {
            300
        } else {
            55
        }
    }

    /// Complete v3 sensor width.
    const fn input_size(self) -> usize {
        if self.is_large() {
            147
        } else {
            83
        }
    }

    /// Stable human-readable workload description.
    fn description(self) -> String {
        if self.is_large() {
            format!(
                "{} differently weighted synthetic population brains, v3/32-bin input shape, five-layer 256-wide MLP, GRU-96, Dense-2",
                self.population_count()
            )
        } else {
            format!(
                "{} differently weighted synthetic population brains, v3/16-bin input shape, 64/64 MLP, GRU-16, Dense-2",
                self.population_count()
            )
        }
    }
}

/// Operator-supplied benchmark controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage4InferenceEvidenceOptions {
    /// Approved scenario.
    pub scenario: Stage4InferenceScenarioName,
    /// Untimed complete population passes.
    pub warmup_passes: usize,
    /// Individually timed complete population passes.
    pub measured_passes: usize,
    /// Development or owner-target-vm provenance declaration.
    pub evidence_environment: String,
    /// Original executable arguments retained with the artifact.
    pub command: Vec<String>,
}

/// Complete Rust scalar inference evidence artifact.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4InferenceEvidence {
    /// Stable document family.
    pub schema: &'static str,
    /// Evidence schema version.
    pub version: u32,
    /// Honest measured-result classification.
    pub evidence_class: String,
    /// Important scope limitation.
    pub caveat: &'static str,
    /// Exact native source/build identity.
    pub source: Stage4InferenceSource,
    /// Target and process environment.
    pub environment: Stage4InferenceEnvironment,
    /// Exact deterministic workload.
    pub workload: Stage4InferenceWorkload,
    /// Execution-boundary description.
    pub path: Stage4InferencePath,
    /// Measured result and retained output proof.
    pub result: Stage4InferenceResult,
    /// Original evidence command.
    pub command: Vec<String>,
}

/// Exact native source identity compiled into the runner.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4InferenceSource {
    /// Source-derived native build identifier.
    pub native_build_identifier: String,
    /// Platform-independent selected-source digest.
    pub native_source_sha256: String,
    /// Build correctness-contract digest.
    pub native_build_contract_sha256: String,
    /// Exact Cargo target triple.
    pub target_triple: String,
    /// Exact compiler identity captured by build.rs.
    pub rustc_version: String,
    /// Cargo profile.
    pub build_profile: String,
    /// Test-hook class proving this runner is absent from production builds.
    pub build_class: String,
}

/// Target and process facts captured with one run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4InferenceEnvironment {
    /// Unix epoch milliseconds.
    pub captured_at_epoch_ms: u128,
    /// Operator declaration.
    pub declaration: String,
    /// Rust compilation operating system.
    pub operating_system: &'static str,
    /// Rust compilation architecture.
    pub architecture: &'static str,
    /// Hostname when supplied by the operating system environment.
    pub hostname: Option<String>,
    /// Logical parallelism visible to the process.
    pub available_parallelism: Option<usize>,
    /// Linux distribution identifier when available.
    pub distribution_id: Option<String>,
    /// First Linux CPU model name when available.
    pub cpu_model: Option<String>,
    /// Linux physical memory total when available.
    pub total_memory_bytes: Option<u64>,
    /// True only when every owner-target VM identity check passes.
    pub owner_target_vm_validated: bool,
    /// Linux resident set after fixture construction when available.
    pub fixture_rss_bytes: Option<u64>,
    /// Linux high-water resident set after measurement when available.
    pub process_peak_rss_bytes: Option<u64>,
}

/// Cross-language workload identity and logical sizes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4InferenceWorkload {
    /// Numeric fixture version.
    pub fixture_version: u32,
    /// Honest classification of the generated numeric data.
    pub fixture_class: &'static str,
    /// P0-P3 label.
    pub scenario: &'static str,
    /// Plain workload description.
    pub description: String,
    /// Differently weighted due brains.
    pub population_count: usize,
    /// Floats per observation.
    pub input_size: usize,
    /// Floats per genome.
    pub total_parameters_per_brain: usize,
    /// Recurrent floats per brain.
    pub recurrent_floats_per_brain: usize,
    /// Complete controller outputs per brain.
    pub output_floats_per_brain: usize,
    /// Collision-safe Rust graph-layout digest.
    pub graph_layout_sha256: String,
    /// Little-endian logical weight digest shared with TypeScript.
    pub weights_sha256: String,
    /// Little-endian logical observation digest shared with TypeScript.
    pub observations_sha256: String,
    /// Little-endian nonzero initial recurrent-state digest shared with TypeScript.
    pub initial_recurrent_sha256: String,
    /// Exact packed parameter bytes retained once by Rust.
    pub packed_weight_bytes: usize,
    /// Exact packed observation bytes.
    pub observation_bytes: usize,
    /// Exact authoritative recurrent bytes.
    pub recurrent_bytes: usize,
    /// Exact staged output/recurrent bytes.
    pub staging_bytes: usize,
    /// Reused executor scratch bytes.
    pub scratch_bytes: usize,
    /// Untimed complete passes.
    pub warmup_passes: usize,
    /// Timed complete passes.
    pub measured_passes: usize,
    /// This microbenchmark does not load a fresh/evolved user population.
    pub actual_fresh_or_evolved_genomes: bool,
    /// This microbenchmark does not run the sensing subsystem.
    pub actual_delivered_sensor_observations: bool,
}

/// Coarse-boundary facts for this execution path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4InferencePath {
    /// Stable path label.
    pub name: &'static str,
    /// Language owning graph traversal and the due-population loop.
    pub graph_owner: &'static str,
    /// N-API transitions inside a complete population pass.
    pub native_calls_per_whole_pass: usize,
    /// Whether the path incorrectly shares one weight block across brains.
    pub shared_weight_batch: bool,
    /// Whether recurrent results publish only after the whole operation succeeds.
    pub staged_recurrent_commit: bool,
}

/// Timings, work checksum, and final result identity.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4InferenceResult {
    /// Complete population-pass distribution.
    pub whole_population_pass_ms: Stage4Distribution,
    /// One complete shared-initial-state pass retained for numeric comparison.
    pub one_step_comparison_probe: Stage4ComparisonProbe,
    /// Finite output accumulator consumed on every pass.
    pub consumed_output: f64,
    /// Final staged outputs digest.
    pub outputs_sha256: String,
    /// Final authoritative recurrent-state digest.
    pub final_recurrent_sha256: String,
    /// Number of distinct controller outputs in the final pass.
    pub distinct_output_pairs: usize,
    /// Linux resident set after measurement when available.
    pub final_rss_bytes: Option<u64>,
}

/// Small raw cross-path proof excluded from performance samples.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ComparisonProbe {
    /// Existing native-backend absolute tolerance applied by the comparator.
    pub absolute_tolerance: f64,
    /// Raw little-endian Float32 controller outputs encoded as lowercase hex.
    pub outputs_f32_le_hex: String,
    /// Raw little-endian Float32 recurrent state encoded as lowercase hex.
    pub recurrent_f32_le_hex: String,
    /// Controller-output Float32 count.
    pub output_floats: usize,
    /// Recurrent-state Float32 count.
    pub recurrent_floats: usize,
    /// SHA-256 of raw little-endian controller outputs.
    pub outputs_sha256: String,
    /// SHA-256 of raw little-endian recurrent state.
    pub recurrent_sha256: String,
    /// Finite complete-population output sum.
    pub consumed_output: f64,
    /// Explicit timing exclusion and reset semantics.
    pub scope: &'static str,
}

/// Millisecond distribution for timed whole-population passes.
#[derive(Debug, Serialize)]
pub struct Stage4Distribution {
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

/// Role-specific deterministic numeric stream.
#[derive(Clone, Copy)]
enum FixtureValueKind {
    Weight,
    Observation,
    Recurrent,
}

/// Owned workload buffers constructed outside the timed interval.
struct Fixture {
    scenario: Stage4InferenceScenarioName,
    plan: GraphExecutionPlan,
    snakes: Vec<SnakeState>,
    population: Vec<PopulationGenome>,
    brains: Vec<BrainRuntimeState>,
    observations: Vec<f32>,
    staged_outputs: Vec<f32>,
    staged_recurrent: Vec<f32>,
    weights_sha256: String,
    observations_sha256: String,
    initial_recurrent_sha256: String,
}

/// Run one measured scalar heterogeneous-population workload.
pub fn run_stage4_inference_evidence(
    options: Stage4InferenceEvidenceOptions,
) -> Result<Stage4InferenceEvidence, String> {
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
    let build_profile = crate::native_addon_build_profile();
    let build_class = crate::native_addon_build_class();
    let target_triple = crate::native_addon_build_target();
    if build_profile != "release" {
        return Err("Stage 4 performance evidence requires a release build".to_owned());
    }
    if build_class != "test-hooks" {
        return Err("Stage 4 performance evidence requires a test-hooks build".to_owned());
    }
    if target_triple != "x86_64-pc-windows-msvc" && target_triple != "x86_64-unknown-linux-gnu" {
        return Err(format!(
            "Stage 4 performance evidence does not support target {target_triple}"
        ));
    }
    let hostname = env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .ok();
    let available_parallelism = std::thread::available_parallelism().ok().map(usize::from);
    let distribution_id = linux_os_release_value("ID");
    let cpu_model = linux_cpu_model();
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

    let mut fixture = build_fixture(options.scenario)?;
    let fixture_rss_bytes = linux_process_status_bytes("VmRSS:");
    let scratch_layout = fixture.plan.scratch_layout();
    let scratch_bytes = scratch_layout
        .required_bytes()
        .map_err(|error| format!("scratch byte calculation failed: {error}"))?;
    let mut workspace = CalculationWorkspace::<()>::try_new(
        fixture.scenario.population_count(),
        1,
        scratch_layout,
        usize::MAX,
    )
    .map_err(|error| format!("calculation workspace allocation failed: {error}"))?;
    let batch = CalculationBatchKey::new(1, 1, FIXTURE_EPOCH);
    workspace.begin(batch);
    for index in 0..fixture.scenario.population_count() {
        workspace
            .try_push_candidate(index, index)
            .map_err(|error| format!("candidate preparation failed: {error}"))?;
    }
    workspace
        .prepare(&fixture.snakes, &fixture.brains, &fixture.population)
        .map_err(|error| format!("work resolution failed: {error}"))?;
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace
        .execution_buffers()
        .map_err(|error| format!("execution borrow failed: {error}"))?;
    let scratch = scratches
        .first_mut()
        .ok_or_else(|| "fixture calculation scratch is absent".to_owned())?;
    let one_step_consumed_output = execute_pass(&mut fixture, work, scratch)?;
    let one_step_recurrent = fixture
        .brains
        .iter()
        .map(|brain| brain.recurrent.as_ref())
        .collect::<Vec<_>>();
    let one_step_comparison_probe = Stage4ComparisonProbe {
        absolute_tolerance: 1.0e-4,
        outputs_f32_le_hex: hex_f32_slices([fixture.staged_outputs.as_slice()]),
        recurrent_f32_le_hex: hex_f32_slices(one_step_recurrent.iter().copied()),
        output_floats: fixture.staged_outputs.len(),
        recurrent_floats: one_step_recurrent.iter().map(|values| values.len()).sum(),
        outputs_sha256: digest_f32_slices([fixture.staged_outputs.as_slice()]),
        recurrent_sha256: digest_f32_slices(one_step_recurrent.iter().copied()),
        consumed_output: one_step_consumed_output,
        scope: "raw complete-population Float32 result from the shared initial state; excluded from timed samples and reset before warmup; compare element by element rather than comparing rounded hashes",
    };
    reset_fixture_recurrent(&mut fixture);
    let mut consumed_output = 0.0_f64;

    for _ in 0..options.warmup_passes {
        consumed_output += execute_pass(&mut fixture, work, scratch)?;
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(options.measured_passes)
        .map_err(|_| "timing sample allocation failed".to_owned())?;
    for _ in 0..options.measured_passes {
        let started = Instant::now();
        consumed_output += execute_pass(&mut fixture, work, scratch)?;
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    let outputs_sha256 = digest_f32_slices([fixture.staged_outputs.as_slice()]);
    let final_recurrent_sha256 =
        digest_f32_slices(fixture.brains.iter().map(|brain| brain.recurrent.as_ref()));
    if final_recurrent_sha256 == fixture.initial_recurrent_sha256 {
        return Err("recurrent state did not advance during the benchmark".to_owned());
    }
    let distinct_output_pairs = fixture
        .staged_outputs
        .chunks_exact(fixture.plan.output_size())
        .map(|output| {
            output
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>()
        .len();
    if distinct_output_pairs < 2 {
        return Err("heterogeneous fixture produced fewer than two distinct outputs".to_owned());
    }

    let population_count = fixture.scenario.population_count();
    let packed_weight_bytes = checked_float_bytes(
        population_count
            .checked_mul(fixture.plan.total_parameters())
            .ok_or_else(|| "packed weight count overflowed".to_owned())?,
        "packed weights",
    )?;
    let observation_bytes = checked_float_bytes(fixture.observations.len(), "observations")?;
    let recurrent_bytes = checked_float_bytes(
        population_count
            .checked_mul(fixture.plan.total_state_size())
            .ok_or_else(|| "recurrent count overflowed".to_owned())?,
        "recurrent state",
    )?;
    let staging_bytes = checked_float_bytes(
        fixture
            .staged_outputs
            .len()
            .checked_add(fixture.staged_recurrent.len())
            .ok_or_else(|| "staging count overflowed".to_owned())?,
        "staging",
    )?;
    let peak_rss = linux_process_status_bytes("VmHWM:");
    let final_rss = linux_process_status_bytes("VmRSS:");
    let evidence_class = if options.evidence_environment == "owner-target-vm" {
        "new measured target-VM Rust scalar result"
    } else {
        "new measured development-machine Rust scalar result"
    };

    Ok(Stage4InferenceEvidence {
        schema: "slither-stage4-rust-inference-benchmark",
        version: 1,
        evidence_class: evidence_class.to_owned(),
        caveat: "Source-shaped synthetic inference-only microbenchmark. It excludes actual fresh/evolved genomes and sensor observations, sensing, physics, frames, Node, N-API, the live coordinator, SIMD, parallelism, and end-to-end server latency; it cannot by itself satisfy the Stage 4 production-workload gate.",
        source: Stage4InferenceSource {
            native_build_identifier: crate::native_addon_build_identifier(),
            native_source_sha256: crate::native_addon_source_sha256(),
            native_build_contract_sha256: crate::native_addon_build_contract_sha256(),
            target_triple,
            rustc_version: crate::native_addon_rustc_version(),
            build_profile,
            build_class,
        },
        environment: Stage4InferenceEnvironment {
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
            process_peak_rss_bytes: peak_rss,
        },
        workload: Stage4InferenceWorkload {
            fixture_version: STAGE4_INFERENCE_FIXTURE_VERSION,
            fixture_class: "source-shaped deterministic synthetic numeric data",
            scenario: fixture.scenario.label(),
            description: fixture.scenario.description(),
            population_count,
            input_size: fixture.plan.input_size(),
            total_parameters_per_brain: fixture.plan.total_parameters(),
            recurrent_floats_per_brain: fixture.plan.total_state_size(),
            output_floats_per_brain: fixture.plan.output_size(),
            graph_layout_sha256: hex_bytes(fixture.plan.layout_digest_sha256()),
            weights_sha256: fixture.weights_sha256,
            observations_sha256: fixture.observations_sha256,
            initial_recurrent_sha256: fixture.initial_recurrent_sha256,
            packed_weight_bytes,
            observation_bytes,
            recurrent_bytes,
            staging_bytes,
            scratch_bytes,
            warmup_passes: options.warmup_passes,
            measured_passes: options.measured_passes,
            actual_fresh_or_evolved_genomes: false,
            actual_delivered_sensor_observations: false,
        },
        path: Stage4InferencePath {
            name: "rust-scalar-coarse-heterogeneous",
            graph_owner: "Rust",
            native_calls_per_whole_pass: 0,
            shared_weight_batch: false,
            staged_recurrent_commit: true,
        },
        result: Stage4InferenceResult {
            whole_population_pass_ms: distribution(samples)?,
            one_step_comparison_probe,
            consumed_output,
            outputs_sha256,
            final_recurrent_sha256,
            distinct_output_pairs,
            final_rss_bytes: final_rss,
        },
        command: options.command,
    })
}

/// Execute and commit one complete due population using fixed buffers.
fn execute_pass(
    fixture: &mut Fixture,
    work: &[CalculationWorkUnit],
    scratch: &mut CalculationScratch,
) -> Result<f64, String> {
    evaluate_heterogeneous_population(
        &fixture.plan,
        work,
        &fixture.population,
        &fixture.brains,
        HeterogeneousInferenceBuffers {
            observations: &fixture.observations,
            staged_outputs: &mut fixture.staged_outputs,
            staged_recurrent: &mut fixture.staged_recurrent,
        },
        &mut scratch.view(),
    )
    .map_err(|error| format!("heterogeneous evaluation failed: {error}"))?;
    let consumed = fixture
        .staged_outputs
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>();
    if !consumed.is_finite() {
        return Err("Rust scalar path produced a non-finite output accumulator".to_owned());
    }
    commit_heterogeneous_recurrent(
        &fixture.plan,
        work,
        &mut fixture.brains,
        &fixture.staged_recurrent,
    )
    .map_err(|error| format!("recurrent commit failed: {error}"))?;
    Ok(consumed)
}

/// Restore the generated initial recurrent state after the untimed comparison probe.
fn reset_fixture_recurrent(fixture: &mut Fixture) {
    for (slot, brain) in fixture.brains.iter_mut().enumerate() {
        for (index, value) in brain.recurrent.iter_mut().enumerate() {
            *value = fixture_value(fixture.scenario, FixtureValueKind::Recurrent, slot, index);
        }
    }
    fixture.staged_outputs.fill(0.0);
    fixture.staged_recurrent.fill(0.0);
}

/// Construct the complete source-shaped workload outside the measured interval.
fn build_fixture(scenario: Stage4InferenceScenarioName) -> Result<Fixture, String> {
    let graph = compile_graph(&scenario_graph(scenario), &graph_limits())
        .map_err(|error| format!("{} graph compilation failed: {error}", scenario.label()))?;
    let plan = GraphExecutionPlan::build(&graph)
        .map_err(|error| format!("{} execution planning failed: {error}", scenario.label()))?;
    let count = scenario.population_count();
    let mut snakes = Vec::new();
    let mut population = Vec::new();
    let mut brains = Vec::new();
    snakes
        .try_reserve_exact(count)
        .map_err(|_| "snake fixture allocation failed".to_owned())?;
    population
        .try_reserve_exact(count)
        .map_err(|_| "population fixture allocation failed".to_owned())?;
    brains
        .try_reserve_exact(count)
        .map_err(|_| "brain fixture allocation failed".to_owned())?;
    let mut weight_hash = Sha256::new();
    let mut recurrent_hash = Sha256::new();

    for slot in 0..count {
        let brain = BrainHandle {
            id: slot as u64 + 1,
            epoch: FIXTURE_EPOCH,
        };
        let mut weights = try_zeroed_floats(plan.total_parameters(), "genome weights")?;
        for (index, value) in weights.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Weight, slot, index);
        }
        update_f32_hash(&mut weight_hash, &weights);
        let mut recurrent = try_zeroed_floats(plan.total_state_size(), "brain recurrent state")?;
        for (index, value) in recurrent.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Recurrent, slot, index);
        }
        update_f32_hash(&mut recurrent_hash, &recurrent);
        let slot_u32 = u32::try_from(slot).map_err(|_| "population slot exceeds u32".to_owned())?;
        population.push(PopulationGenome {
            slot: slot_u32,
            brain,
            lineage: GenomeLineage {
                genome_id: slot as u64 + 10_001,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: slot as f64,
            weights: weights.into_boxed_slice(),
        });
        brains.push(BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(slot_u32),
            non_population_weights: None,
            recurrent: recurrent.into_boxed_slice(),
        });
        snakes.push(fixture_snake(slot, brain)?);
    }
    if population.len() > 1 && population[0].weights == population[1].weights {
        return Err("fixture genomes are not differently weighted".to_owned());
    }

    let observation_count = count
        .checked_mul(plan.input_size())
        .ok_or_else(|| "observation count overflowed".to_owned())?;
    let mut observations = try_zeroed_floats(observation_count, "observations")?;
    let mut observation_hash = Sha256::new();
    for slot in 0..count {
        let start = slot * plan.input_size();
        let observation = &mut observations[start..start + plan.input_size()];
        for (index, value) in observation.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Observation, slot, index);
        }
        update_f32_hash(&mut observation_hash, observation);
    }
    let output_count = count
        .checked_mul(plan.output_size())
        .ok_or_else(|| "output staging count overflowed".to_owned())?;
    let recurrent_count = count
        .checked_mul(plan.total_state_size())
        .ok_or_else(|| "recurrent staging count overflowed".to_owned())?;

    Ok(Fixture {
        scenario,
        plan,
        snakes,
        population,
        brains,
        observations,
        staged_outputs: try_zeroed_floats(output_count, "output staging")?,
        staged_recurrent: try_zeroed_floats(recurrent_count, "recurrent staging")?,
        weights_sha256: hex_bytes(weight_hash.finalize()),
        observations_sha256: hex_bytes(observation_hash.finalize()),
        initial_recurrent_sha256: hex_bytes(recurrent_hash.finalize()),
    })
}

/// Construct one valid evolved snake mapped to the same-index brain and slot.
fn fixture_snake(slot: usize, brain: BrainHandle) -> Result<SnakeState, String> {
    let slot_u32 = u32::try_from(slot).map_err(|_| "fixture slot exceeds u32".to_owned())?;
    Ok(SnakeState {
        id: slot as u64 + 1,
        frame_v1_id: slot_u32 + 1,
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(slot_u32),
        brain: Some(brain),
        baseline_slot: None,
        baseline_strategy: None,
        position: WorldPoint { x: 0.0, y: 0.0 },
        previous_position: WorldPoint { x: 0.0, y: 0.0 },
        direction: 0.0,
        radius: 1.0,
        speed: 1.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: 2.0,
        fitness: 0.0,
        turn: 0.0,
        previous_turn: 0.0,
        input_boost: false,
        previous_input_boost: false,
        control_accumulator_seconds: 0.0,
        delivered_observation_points: 0.0,
        body: BodyRange { start: 0, len: 0 },
        skin: slot_u32,
    })
}

/// Build the exact current P0/P1 or P2/P3 linear graph.
fn scenario_graph(scenario: Stage4InferenceScenarioName) -> GraphSpec {
    let input_size = scenario.input_size();
    let (feature_id, feature_kind, memory_id, memory_kind, output_id, output_kind) =
        if scenario.is_large() {
            (
                "features",
                GraphNodeKind::Mlp {
                    input_size,
                    hidden_sizes: vec![256, 256, 256, 256],
                    output_size: 256,
                },
                "memory",
                GraphNodeKind::Gru {
                    input_size: 256,
                    hidden_size: 96,
                },
                "output",
                GraphNodeKind::Dense {
                    input_size: 96,
                    output_size: 2,
                },
            )
        } else {
            (
                "mlp",
                GraphNodeKind::Mlp {
                    input_size,
                    hidden_sizes: vec![64],
                    output_size: 64,
                },
                "gru",
                GraphNodeKind::Gru {
                    input_size: 64,
                    hidden_size: 16,
                },
                "head",
                GraphNodeKind::Dense {
                    input_size: 16,
                    output_size: 2,
                },
            )
        };
    GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".to_owned(),
                kind: GraphNodeKind::Input {
                    output_size: input_size,
                },
            },
            GraphNodeSpec {
                id: feature_id.to_owned(),
                kind: feature_kind,
            },
            GraphNodeSpec {
                id: memory_id.to_owned(),
                kind: memory_kind,
            },
            GraphNodeSpec {
                id: output_id.to_owned(),
                kind: output_kind,
            },
        ],
        edges: [
            ("input", feature_id),
            (feature_id, memory_id),
            (memory_id, output_id),
        ]
        .into_iter()
        .map(|(from, to)| GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            from_port: None,
            to_port: None,
        })
        .collect(),
        outputs: vec![GraphOutputRef {
            node_id: output_id.to_owned(),
            port: None,
        }],
        output_size: 2,
    }
}

/// Source-shaped graph limits admitting P0-P3 without becoming runtime defaults.
fn graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 16,
        max_edges: 16,
        max_graph_outputs: 4,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 4_096,
        max_tensor_width: 512,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 8,
        max_parameter_floats: 500_000,
        max_recurrent_state_floats: 1_024,
        max_canonical_layout_bytes: 128 * 1024,
        max_architecture_key_bytes: 256 * 1024,
    }
}

/// Produce one deterministic xorshift word shared with TypeScript.
fn fixture_word(
    scenario: Stage4InferenceScenarioName,
    kind: FixtureValueKind,
    slot: usize,
    index: usize,
) -> u32 {
    let scenario_word = if scenario.is_large() {
        0x1319_8a2e
    } else {
        0x85a3_08d3
    };
    let kind_word = match kind {
        FixtureValueKind::Weight => 0x243f_6a88,
        FixtureValueKind::Observation => 0xb7e1_5162,
        FixtureValueKind::Recurrent => 0x9e37_79b9,
    };
    let mut word = scenario_word
        ^ kind_word
        ^ (slot as u32 + 1).wrapping_mul(0x9e37_79b9)
        ^ (index as u32 + 1).wrapping_mul(0x7f4a_7c15);
    word ^= word.wrapping_shl(13);
    word ^= word >> 17;
    word ^= word.wrapping_shl(5);
    word
}

/// Convert one generated word to a bounded nonzero Float32 with exact bits.
fn fixture_value(
    scenario: Stage4InferenceScenarioName,
    kind: FixtureValueKind,
    slot: usize,
    index: usize,
) -> f32 {
    let word = fixture_word(scenario, kind, slot, index);
    let exponent = match kind {
        FixtureValueKind::Weight => 0x3d00_0000,
        FixtureValueKind::Observation => 0x3e00_0000,
        FixtureValueKind::Recurrent => 0x3c00_0000,
    };
    f32::from_bits((word & 0x8000_0000) | exponent | (word & 0x007f_ffff))
}

/// Allocate a zeroed Float32 vector through fallible capacity admission.
fn try_zeroed_floats(count: usize, label: &'static str) -> Result<Vec<f32>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed for {count} floats"))?;
    values.resize(count, 0.0);
    Ok(values)
}

/// Update one logical digest with canonical little-endian Float32 bytes.
fn update_f32_hash(hash: &mut Sha256, values: &[f32]) {
    const CHUNK_FLOATS: usize = 16_384;
    let mut bytes = vec![0_u8; CHUNK_FLOATS * 4];
    for chunk in values.chunks(CHUNK_FLOATS) {
        for (index, value) in chunk.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        hash.update(&bytes[..chunk.len() * 4]);
    }
}

/// Digest an ordered sequence of Float32 slices.
fn digest_f32_slices<'a>(slices: impl IntoIterator<Item = &'a [f32]>) -> String {
    let mut hash = Sha256::new();
    for values in slices {
        update_f32_hash(&mut hash, values);
    }
    hex_bytes(hash.finalize())
}

/// Encode an ordered sequence of Float32 slices as raw little-endian hexadecimal.
fn hex_f32_slices<'a>(slices: impl IntoIterator<Item = &'a [f32]>) -> String {
    let mut output = String::new();
    for values in slices {
        for value in values {
            output.push_str(&hex_bytes(value.to_le_bytes()));
        }
    }
    output
}

/// Lowercase hexadecimal encoding for digest byte sequences.
fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

/// Checked logical bytes for a Float32 count.
fn checked_float_bytes(count: usize, label: &'static str) -> Result<usize, String> {
    count
        .checked_mul(size_of::<f32>())
        .ok_or_else(|| format!("{label} byte count overflowed"))
}

/// Summarize nonempty finite millisecond samples with interpolated percentiles.
fn distribution(mut values: Vec<f64>) -> Result<Stage4Distribution, String> {
    if values.is_empty()
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("timing samples must be nonempty, finite, and nonnegative".to_owned());
    }
    values.sort_by(f64::total_cmp);
    let percentile = |fraction: f64| {
        let position = (values.len() - 1) as f64 * fraction;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
    };
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    Ok(Stage4Distribution {
        count: values.len(),
        min: values[0],
        p50: percentile(0.5),
        p95: percentile(0.95),
        p99: percentile(0.99),
        max: values[values.len() - 1],
        mean,
    })
}

/// Read one KiB-valued field from Linux /proc/self/status as bytes.
fn linux_process_status_bytes(field: &str) -> Option<u64> {
    if env::consts::OS != "linux" {
        return None;
    }
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with(field))?;
    let kibibytes = line.split_ascii_whitespace().nth(1)?.parse::<u64>().ok()?;
    kibibytes.checked_mul(1024)
}

/// Read one unquoted key from Linux /etc/os-release.
fn linux_os_release_value(key: &str) -> Option<String> {
    if env::consts::OS != "linux" {
        return None;
    }
    let release = fs::read_to_string("/etc/os-release").ok()?;
    let prefix = format!("{key}=");
    release
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.trim_matches('"').to_owned())
}

/// Read the first Linux CPU model name.
fn linux_cpu_model() -> Option<String> {
    if env::consts::OS != "linux" {
        return None;
    }
    fs::read_to_string("/proc/cpuinfo")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("model name\t:"))
        .map(|value| value.trim().to_owned())
}

/// Read Linux MemTotal as bytes.
fn linux_total_memory_bytes() -> Option<u64> {
    if env::consts::OS != "linux" {
        return None;
    }
    let memory = fs::read_to_string("/proc/meminfo").ok()?;
    let line = memory.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kibibytes = line.split_ascii_whitespace().nth(1)?.parse::<u64>().ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_scenario_shapes_compile_to_exact_counts() {
        for (scenario, population, input, parameters, recurrent) in [
            (Stage4InferenceScenarioName::P0, 55, 83, 13_458, 16),
            (Stage4InferenceScenarioName::P1, 300, 83, 13_458, 16),
            (Stage4InferenceScenarioName::P2, 55, 147, 402_914, 96),
            (Stage4InferenceScenarioName::P3, 300, 147, 402_914, 96),
        ] {
            let graph = compile_graph(&scenario_graph(scenario), &graph_limits()).unwrap();
            assert_eq!(scenario.population_count(), population);
            assert_eq!(scenario.input_size(), input);
            assert_eq!(graph.total_parameters, parameters);
            assert_eq!(graph.total_state_size, recurrent);
        }
    }

    #[test]
    fn p0_evidence_runs_synthetic_heterogeneous_stateful_population() {
        let report = run_stage4_inference_evidence(Stage4InferenceEvidenceOptions {
            scenario: Stage4InferenceScenarioName::P0,
            warmup_passes: 1,
            measured_passes: 2,
            evidence_environment: "development".to_owned(),
            command: vec!["unit-test".to_owned()],
        })
        .unwrap();
        assert_eq!(report.workload.population_count, 55);
        assert_eq!(report.workload.total_parameters_per_brain, 13_458);
        assert_eq!(report.result.one_step_comparison_probe.output_floats, 110);
        assert_eq!(
            report.result.one_step_comparison_probe.recurrent_floats,
            880
        );
        assert_eq!(
            report
                .result
                .one_step_comparison_probe
                .outputs_f32_le_hex
                .len(),
            110 * 8
        );
        assert_eq!(
            report
                .result
                .one_step_comparison_probe
                .recurrent_f32_le_hex
                .len(),
            880 * 8
        );
        assert_eq!(
            report.workload.weights_sha256,
            "5d08fee2550b4c438e96608fac993845967fb5c5edc2e5ac09a40ea23cd18d69"
        );
        assert_eq!(
            report.workload.observations_sha256,
            "c10e2266960c4ba346855f4114d80c27f25970eb46a30cc948de0d6f886aeb5b"
        );
        assert_eq!(
            report.workload.initial_recurrent_sha256,
            "686f728d7c1057e652ae0f69b359ddc2d4318885cfcedde08477b775049fecd1"
        );
        assert_eq!(report.result.whole_population_pass_ms.count, 2);
        assert!(report.result.distinct_output_pairs > 1);
        assert_ne!(
            report.workload.initial_recurrent_sha256,
            report.result.final_recurrent_sha256
        );
        assert_eq!(report.path.native_calls_per_whole_pass, 0);
        assert!(!report.path.shared_weight_batch);
    }
}
