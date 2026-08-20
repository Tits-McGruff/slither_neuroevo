//! Deterministic Stage 4 corrected-sensing plus heterogeneous-control evidence.
//!
//! This test-hook-only fixture joins the production Rust spatial indexes,
//! sensor-v3 evaluator, distinct genome/state resolution, complete graph
//! executor, delivered-observation boundary and recurrent commit. It remains
//! deliberately short of a complete game step: no movement, food, collision,
//! controller selection, frame, Node bridge, browser or RL client is present.

use super::calculation::{CalculationBatchKey, CalculationCandidateIndex};
use super::control::{
    NeuralControlBatchInputs, NeuralControlCapacityDiagnostics, NeuralControlPipeline,
};
use super::graph::compile_graph;
use super::inference::GraphExecutionPlan;
use super::inference_fixture::{
    fixture_value, graph_limits, scenario_graph, FixtureValueKind, Stage4InferenceScenarioName,
};
use super::sensing_fixture::{
    allocation_distribution, build_world, cpu_usage, digest_world, distribution, hex_bytes,
    linux_os_release_value, linux_process_status_bytes, linux_total_memory_bytes,
    process_cpu_snapshot, system_cpu_model, system_hostname, SensorWorkReport,
    Stage4AllocationDistribution, Stage4CpuUsage, Stage4SensingDistribution,
    Stage4SensingScenarioSpec,
};
use super::sensors::{
    SensorConfig, SensorEvaluator, SensorGenerationState, SensorScratch, SensorScratchDiagnostics,
};
use super::spatial::{
    BodyIndexDiagnostics, IndexedSensorWorld, PelletIndexDiagnostics, SensorIndexConfig,
};
use super::state::{BrainHandle, BrainOwner, BrainRuntimeState, GenomeLineage, PopulationGenome};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::mem::size_of;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Version of the combined deterministic control-boundary fixture.
pub const STAGE4_CONTROL_FIXTURE_VERSION: u32 = 1;
/// Population epoch used by the combined evidence fixture.
const FIXTURE_EPOCH: u64 = 2;
/// Default body/pellet index widths inherited from the sensing checkpoint.
const BODY_CELL_SIZE: f64 = 70.0;
const PELLET_CELL_SIZE: f64 = 120.0;
/// Complete derived-index ceilings for short-body control fixtures.
const MAXIMUM_BODY_ENTRIES: usize = 100_000;
const MAXIMUM_PELLET_ENTRIES: usize = 25_000;
/// Non-neural baseline snakes present in the source-shaped world.
const BASELINE_SNAKES: usize = 10;
/// Current default short-body fixture length.
const BODY_POINTS_PER_SNAKE: usize = 5;
/// Current default pellet load.
const PELLETS: usize = 3_500;

/// Operator-supplied combined benchmark controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage4ControlEvidenceOptions {
    /// Approved P0-P3 scenario.
    pub scenario: Stage4InferenceScenarioName,
    /// Untimed stateful boundaries.
    pub warmup_passes: usize,
    /// Individually timed stateful boundaries.
    pub measured_passes: usize,
    /// Development or owner-target-vm provenance declaration.
    pub evidence_environment: String,
    /// Original executable arguments retained with the report.
    pub command: Vec<String>,
}

/// Complete combined Stage 4 evidence document.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlEvidence {
    /// Stable document family.
    pub schema: &'static str,
    /// Evidence schema version.
    pub version: u32,
    /// Honest measured-result classification.
    pub evidence_class: String,
    /// Important scope limitation.
    pub caveat: &'static str,
    /// Exact source/build identity.
    pub source: Stage4ControlSource,
    /// Target and process environment.
    pub environment: Stage4ControlEnvironment,
    /// Deterministic workload identity and dimensions.
    pub workload: Stage4ControlWorkload,
    /// Coarse Rust ownership facts.
    pub path: Stage4ControlPath,
    /// Retained allocation and memory facts.
    pub memory: Stage4ControlMemory,
    /// Combined timings, allocations, hashes and corrected-boundary proof.
    pub result: Stage4ControlResult,
    /// Original evidence command.
    pub command: Vec<String>,
}

/// Compiled source and target identity.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlSource {
    pub native_build_identifier: String,
    pub native_source_sha256: String,
    pub native_build_contract_sha256: String,
    pub target_triple: String,
    pub rustc_version: String,
    pub build_profile: String,
    pub build_class: String,
}

/// Target and process facts captured with one run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlEnvironment {
    pub captured_at_epoch_ms: u128,
    pub declaration: String,
    pub operating_system: &'static str,
    pub architecture: &'static str,
    pub hostname: Option<String>,
    pub available_parallelism: Option<usize>,
    pub distribution_id: Option<String>,
    pub cpu_model: Option<String>,
    pub total_memory_bytes: Option<u64>,
    pub owner_target_vm_validated: bool,
    pub fixture_rss_bytes: Option<u64>,
    pub process_peak_rss_bytes: Option<u64>,
    pub final_rss_bytes: Option<u64>,
}

/// Exact deterministic source-shaped workload.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlWorkload {
    pub fixture_version: u32,
    pub fixture_class: &'static str,
    pub scenario: &'static str,
    pub description: String,
    pub evolved_population: usize,
    pub baseline_snakes: usize,
    pub total_live_snakes: usize,
    pub due_neural_brains: usize,
    pub body_points_per_snake: usize,
    pub pellets: usize,
    pub sensor_bins: usize,
    pub sensor_input_size: usize,
    pub parameters_per_brain: usize,
    pub recurrent_floats_per_brain: usize,
    pub output_floats_per_brain: usize,
    pub graph_layout_sha256: String,
    pub world_sha256: String,
    pub weights_sha256: String,
    pub initial_recurrent_sha256: String,
    pub unique_weight_blocks: usize,
    pub unique_initial_recurrent_blocks: usize,
    pub warmup_passes: usize,
    pub measured_passes: usize,
    pub actual_fresh_or_evolved_world: bool,
    pub actual_corrected_sensor_observations: bool,
    pub actual_distinct_genomes: bool,
    pub actual_stateful_recurrent_commit: bool,
    pub actual_complete_world_step: bool,
}

/// Language-boundary and worker facts for the measured operation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlPath {
    pub owner: &'static str,
    pub math_backend: &'static str,
    pub calculation_workers: usize,
    pub napi_calls_per_boundary: usize,
    pub graph_traversal_owner: &'static str,
    pub distinct_weights_per_brain: bool,
    pub staged_atomic_state_commit: bool,
    pub focused_activation_capture_enabled: bool,
}

/// Fixed staging, packed-state, index and process memory facts.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlMemory {
    pub packed_weight_bytes: usize,
    pub authoritative_recurrent_bytes: usize,
    pub pipeline_staging_bytes: usize,
    pub body_index_estimated_bytes: usize,
    pub pellet_index_estimated_bytes: usize,
    pub pipeline_capacity_after_warmup: ControlCapacityReport,
    pub pipeline_capacity_after_measurement: ControlCapacityReport,
    pub sensor_scratch_after_warmup: ControlSensorScratchReport,
    pub sensor_scratch_after_measurement: ControlSensorScratchReport,
    pub capacities_stable_after_warmup: bool,
}

/// Serializable fixed pipeline capacities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlCapacityReport {
    pub observations: usize,
    pub outputs: usize,
    pub recurrent: usize,
    pub capture_output: usize,
    pub capture_recurrent: usize,
    pub deliveries: usize,
    pub diagnostics: usize,
    pub calculation_workspace_bytes: usize,
}

impl From<NeuralControlCapacityDiagnostics> for ControlCapacityReport {
    fn from(source: NeuralControlCapacityDiagnostics) -> Self {
        Self {
            observations: source.observations,
            outputs: source.outputs,
            recurrent: source.recurrent,
            capture_output: source.capture_output,
            capture_recurrent: source.capture_recurrent,
            deliveries: source.deliveries,
            diagnostics: source.diagnostics,
            calculation_workspace_bytes: source.calculation_workspace_bytes,
        }
    }
}

/// Serializable sensor/query scratch capacities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSensorScratchReport {
    pub food_bin_capacity: usize,
    pub hazard_bin_capacity: usize,
    pub head_bin_capacity: usize,
    pub body_duplicate_marker_capacity: usize,
    pub body_candidate_capacity: usize,
    pub pellet_candidate_capacity: usize,
}

impl From<SensorScratchDiagnostics> for ControlSensorScratchReport {
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

/// Timings, allocations, identities and corrected commit proof.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage4ControlResult {
    pub complete_control_boundary_ms: Stage4SensingDistribution,
    pub index_build_ms: Stage4SensingDistribution,
    pub sensing_and_inference_ms: Stage4SensingDistribution,
    pub recurrent_and_delivery_commit_ms: Stage4SensingDistribution,
    pub allocator_operations_per_complete_boundary: Stage4AllocationDistribution,
    pub allocator_operations_per_index_build: Stage4AllocationDistribution,
    pub allocator_operations_per_sensing_and_inference: Stage4AllocationDistribution,
    pub allocator_operations_per_commit: Stage4AllocationDistribution,
    pub cpu: Stage4CpuUsage,
    pub body_index: ControlBodyIndexReport,
    pub pellet_index: ControlPelletIndexReport,
    pub proof_sensor_work: SensorWorkReport,
    pub proof_observations_sha256: String,
    pub proof_outputs_sha256: String,
    pub final_recurrent_sha256: String,
    pub distinct_proof_output_pairs: usize,
    pub anti_optimization_output_accumulator_including_proof: f64,
    pub proof_delivery_boundaries_changed: usize,
    pub every_due_delivery_boundary_committed: bool,
}

/// Complete body-index counts retained from the proof boundary.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlBodyIndexReport {
    pub segments: usize,
    pub entries: usize,
    pub occupied_cells: usize,
    pub lookup_cells: usize,
    pub estimated_bytes: usize,
    pub maximum_entries: usize,
}

impl From<BodyIndexDiagnostics> for ControlBodyIndexReport {
    fn from(source: BodyIndexDiagnostics) -> Self {
        Self {
            segments: source.segments,
            entries: source.entries,
            occupied_cells: source.occupied_cells,
            lookup_cells: source.lookup_cells,
            estimated_bytes: source.estimated_bytes,
            maximum_entries: source.maximum_entries,
        }
    }
}

/// Complete pellet-index counts retained from the proof boundary.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlPelletIndexReport {
    pub pellets: usize,
    pub occupied_cells: usize,
    pub lookup_cells: usize,
    pub estimated_bytes: usize,
    pub maximum_entries: usize,
}

impl From<PelletIndexDiagnostics> for ControlPelletIndexReport {
    fn from(source: PelletIndexDiagnostics) -> Self {
        Self {
            pellets: source.pellets,
            occupied_cells: source.occupied_cells,
            lookup_cells: source.lookup_cells,
            estimated_bytes: source.estimated_bytes,
            maximum_entries: source.maximum_entries,
        }
    }
}

/// Owned source-shaped state and reusable operation storage.
struct ControlFixture {
    world: super::state::WorldState,
    population: Vec<PopulationGenome>,
    brains: Vec<BrainRuntimeState>,
    generation: SensorGenerationState,
    candidates: Vec<CalculationCandidateIndex>,
    pipeline: NeuralControlPipeline,
    sensor_scratch: SensorScratch,
    index_config: SensorIndexConfig,
    world_sha256: String,
    weights_sha256: String,
    initial_recurrent_sha256: String,
    unique_weight_blocks: usize,
    unique_initial_recurrent_blocks: usize,
}

/// One measured complete boundary's phase and allocation observations.
struct BoundarySample {
    complete_ms: f64,
    index_ms: f64,
    evaluate_ms: f64,
    commit_ms: f64,
    complete_allocations: u64,
    index_allocations: u64,
    evaluate_allocations: u64,
    commit_allocations: u64,
    consumed_output: f64,
    body_index: BodyIndexDiagnostics,
    pellet_index: PelletIndexDiagnostics,
}

/// Optional proof-only hashes and work outside measured samples.
struct ProofCapture {
    observation_hash: Sha256,
    output_hash: Sha256,
    output_pairs: BTreeSet<(u32, u32)>,
    sensor_work: SensorWorkReport,
}

impl Default for ProofCapture {
    fn default() -> Self {
        Self {
            observation_hash: Sha256::new(),
            output_hash: Sha256::new(),
            output_pairs: BTreeSet::new(),
            sensor_work: SensorWorkReport::default(),
        }
    }
}

/// Run one stateful combined Stage 4 benchmark.
pub fn run_stage4_control_evidence(
    options: Stage4ControlEvidenceOptions,
    allocation_snapshot: impl Fn() -> u64,
) -> Result<Stage4ControlEvidence, String> {
    validate_options(&options)?;
    let target_triple = crate::native_addon_build_target();
    let build_profile = crate::native_addon_build_profile();
    let build_class = crate::native_addon_build_class();
    if build_profile != "release" {
        return Err("Stage 4 control evidence requires a release build".to_owned());
    }
    if build_class != "test-hooks" {
        return Err("Stage 4 control evidence requires a test-hooks build".to_owned());
    }
    if target_triple != "x86_64-pc-windows-msvc" && target_triple != "x86_64-unknown-linux-gnu" {
        return Err(format!(
            "Stage 4 control evidence does not support target {target_triple}"
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

    let mut fixture = build_fixture(options.scenario)?;
    let fixture_rss_bytes = linux_process_status_bytes("VmRSS:");
    let packed_weight_bytes = checked_product(
        options.scenario.population_count(),
        fixture.pipeline.inference().total_parameters(),
        "packed weight float count",
    )?
    .checked_mul(size_of::<f32>())
    .ok_or_else(|| "packed weight bytes overflowed".to_owned())?;
    let authoritative_recurrent_bytes = checked_product(
        options.scenario.population_count(),
        fixture.pipeline.inference().total_state_size(),
        "authoritative recurrent float count",
    )?
    .checked_mul(size_of::<f32>())
    .ok_or_else(|| "authoritative recurrent bytes overflowed".to_owned())?;

    for step in 0..options.warmup_passes {
        let key = CalculationBatchKey::new(1, step as u64 + 1, FIXTURE_EPOCH);
        execute_boundary(&mut fixture, key, &allocation_snapshot, None)?;
    }
    let pipeline_capacity_after_warmup =
        ControlCapacityReport::from(fixture.pipeline.capacity_diagnostics());
    let sensor_scratch_after_warmup =
        ControlSensorScratchReport::from(fixture.sensor_scratch.diagnostics());

    let mut complete_samples = reserve_samples(options.measured_passes, "complete timings")?;
    let mut index_samples = reserve_samples(options.measured_passes, "index timings")?;
    let mut evaluate_samples = reserve_samples(options.measured_passes, "evaluation timings")?;
    let mut commit_samples = reserve_samples(options.measured_passes, "commit timings")?;
    let mut complete_allocations =
        reserve_allocation_samples(options.measured_passes, "complete allocations")?;
    let mut index_allocations =
        reserve_allocation_samples(options.measured_passes, "index allocations")?;
    let mut evaluate_allocations =
        reserve_allocation_samples(options.measured_passes, "evaluation allocations")?;
    let mut commit_allocations =
        reserve_allocation_samples(options.measured_passes, "commit allocations")?;
    let mut anti_optimization_output_accumulator = 0.0_f64;
    let cpu_before = process_cpu_snapshot();
    let measured_started = Instant::now();
    for pass in 0..options.measured_passes {
        let step = options
            .warmup_passes
            .checked_add(pass)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| "measured step overflowed".to_owned())?;
        let sample = execute_boundary(
            &mut fixture,
            CalculationBatchKey::new(1, step as u64, FIXTURE_EPOCH),
            &allocation_snapshot,
            None,
        )?;
        complete_samples.push(sample.complete_ms);
        index_samples.push(sample.index_ms);
        evaluate_samples.push(sample.evaluate_ms);
        commit_samples.push(sample.commit_ms);
        complete_allocations.push(sample.complete_allocations);
        index_allocations.push(sample.index_allocations);
        evaluate_allocations.push(sample.evaluate_allocations);
        commit_allocations.push(sample.commit_allocations);
        anti_optimization_output_accumulator += sample.consumed_output;
    }
    let measured_elapsed = measured_started.elapsed();
    let cpu = cpu_usage(cpu_before, process_cpu_snapshot(), measured_elapsed);
    if options.evidence_environment == "owner-target-vm" && cpu.process_cpu_seconds.is_none() {
        return Err("owner-target-vm CPU evidence could not be sampled".to_owned());
    }

    let pipeline_capacity_after_measurement =
        ControlCapacityReport::from(fixture.pipeline.capacity_diagnostics());
    let sensor_scratch_after_measurement =
        ControlSensorScratchReport::from(fixture.sensor_scratch.diagnostics());
    let capacities_stable_after_warmup = pipeline_capacity_after_warmup
        == pipeline_capacity_after_measurement
        && sensor_scratch_after_warmup == sensor_scratch_after_measurement;
    if !capacities_stable_after_warmup {
        return Err("combined pipeline or sensor scratch capacity changed after warmup".to_owned());
    }

    let proof_step = options
        .warmup_passes
        .checked_add(options.measured_passes)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "proof step overflowed".to_owned())?;
    for snake in &mut fixture.world.snakes[..options.scenario.population_count()] {
        snake.delivered_observation_points = snake.points + 0.5;
    }
    let proof_delivery_boundaries_changed = fixture.world.snakes
        [..options.scenario.population_count()]
        .iter()
        .filter(|snake| snake.delivered_observation_points.to_bits() != snake.points.to_bits())
        .count();
    if proof_delivery_boundaries_changed != options.scenario.population_count() {
        return Err("proof boundary did not begin with one visible delta per due snake".to_owned());
    }
    let mut proof = ProofCapture::default();
    let proof_sample = execute_boundary(
        &mut fixture,
        CalculationBatchKey::new(1, proof_step as u64, FIXTURE_EPOCH),
        &allocation_snapshot,
        Some(&mut proof),
    )?;
    anti_optimization_output_accumulator += proof_sample.consumed_output;
    if proof.sensor_work.samples != options.scenario.population_count() {
        return Err("proof boundary did not evaluate every due neural brain".to_owned());
    }
    let every_due_delivery_boundary_committed = fixture.world.snakes
        [..options.scenario.population_count()]
        .iter()
        .all(|snake| snake.delivered_observation_points.to_bits() == snake.points.to_bits());
    if !every_due_delivery_boundary_committed {
        return Err("a due observation-delivery boundary did not commit".to_owned());
    }
    if proof.output_pairs.len() <= 1 {
        return Err("proof boundary did not produce distinct population outputs".to_owned());
    }
    let final_recurrent_sha256 = digest_f32(
        fixture
            .brains
            .iter()
            .flat_map(|brain| brain.recurrent.iter().copied()),
    );
    let spec = control_world_spec(options.scenario);
    let evidence_class = if options.evidence_environment == "owner-target-vm" {
        "new measured target-VM combined Rust sensing/inference result"
    } else {
        "new measured development-machine combined Rust sensing/inference result"
    };

    Ok(Stage4ControlEvidence {
        schema: "slither-stage4-rust-control-pipeline-benchmark",
        version: 1,
        evidence_class: evidence_class.to_owned(),
        caveat: "Source-shaped deterministic synthetic combined control-boundary benchmark. It includes complete index construction, corrected delivered sensor observations, differently weighted stateful population inference, and atomic recurrent/delivery commit. It is not an owner save, movement/collision/game step, controller selection, frame, Node bridge, browser, RL client, generation, sustained run, bounded persistent-worker result, or production-cutover result.",
        source: Stage4ControlSource {
            native_build_identifier: crate::native_addon_build_identifier(),
            native_source_sha256: crate::native_addon_source_sha256(),
            native_build_contract_sha256: crate::native_addon_build_contract_sha256(),
            target_triple,
            rustc_version: crate::native_addon_rustc_version(),
            build_profile,
            build_class,
        },
        environment: Stage4ControlEnvironment {
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
        workload: Stage4ControlWorkload {
            fixture_version: STAGE4_CONTROL_FIXTURE_VERSION,
            fixture_class: "source-shaped deterministic synthetic world and genomes",
            scenario: options.scenario.label(),
            description: control_description(options.scenario),
            evolved_population: spec.evolved_snakes,
            baseline_snakes: spec.baseline_snakes,
            total_live_snakes: spec.total_snakes(),
            due_neural_brains: options.scenario.population_count(),
            body_points_per_snake: spec.body_points_per_snake,
            pellets: spec.pellets,
            sensor_bins: spec.sensor_bins,
            sensor_input_size: fixture.pipeline.inference().input_size(),
            parameters_per_brain: fixture.pipeline.inference().total_parameters(),
            recurrent_floats_per_brain: fixture.pipeline.inference().total_state_size(),
            output_floats_per_brain: fixture.pipeline.inference().output_size(),
            graph_layout_sha256: hex_bytes(fixture.pipeline.inference().layout_digest_sha256()),
            world_sha256: fixture.world_sha256,
            weights_sha256: fixture.weights_sha256,
            initial_recurrent_sha256: fixture.initial_recurrent_sha256,
            unique_weight_blocks: fixture.unique_weight_blocks,
            unique_initial_recurrent_blocks: fixture.unique_initial_recurrent_blocks,
            warmup_passes: options.warmup_passes,
            measured_passes: options.measured_passes,
            actual_fresh_or_evolved_world: false,
            actual_corrected_sensor_observations: true,
            actual_distinct_genomes: true,
            actual_stateful_recurrent_commit: true,
            actual_complete_world_step: false,
        },
        path: Stage4ControlPath {
            owner: "Rust",
            math_backend: fixture.pipeline.inference().math_backend().label(),
            calculation_workers: 1,
            napi_calls_per_boundary: 0,
            graph_traversal_owner: "Rust",
            distinct_weights_per_brain: true,
            staged_atomic_state_commit: true,
            focused_activation_capture_enabled: false,
        },
        memory: Stage4ControlMemory {
            packed_weight_bytes,
            authoritative_recurrent_bytes,
            pipeline_staging_bytes: fixture.pipeline.allocated_staging_bytes(),
            body_index_estimated_bytes: proof_sample.body_index.estimated_bytes,
            pellet_index_estimated_bytes: proof_sample.pellet_index.estimated_bytes,
            pipeline_capacity_after_warmup,
            pipeline_capacity_after_measurement,
            sensor_scratch_after_warmup,
            sensor_scratch_after_measurement,
            capacities_stable_after_warmup,
        },
        result: Stage4ControlResult {
            complete_control_boundary_ms: distribution(complete_samples)?,
            index_build_ms: distribution(index_samples)?,
            sensing_and_inference_ms: distribution(evaluate_samples)?,
            recurrent_and_delivery_commit_ms: distribution(commit_samples)?,
            allocator_operations_per_complete_boundary: allocation_distribution(
                &complete_allocations,
            )?,
            allocator_operations_per_index_build: allocation_distribution(&index_allocations)?,
            allocator_operations_per_sensing_and_inference: allocation_distribution(
                &evaluate_allocations,
            )?,
            allocator_operations_per_commit: allocation_distribution(&commit_allocations)?,
            cpu,
            body_index: proof_sample.body_index.into(),
            pellet_index: proof_sample.pellet_index.into(),
            proof_sensor_work: proof.sensor_work,
            proof_observations_sha256: hex_bytes(proof.observation_hash.finalize()),
            proof_outputs_sha256: hex_bytes(proof.output_hash.finalize()),
            final_recurrent_sha256,
            distinct_proof_output_pairs: proof.output_pairs.len(),
            anti_optimization_output_accumulator_including_proof:
                anti_optimization_output_accumulator,
            proof_delivery_boundaries_changed,
            every_due_delivery_boundary_committed,
        },
        command: options.command,
    })
}

fn build_fixture(scenario: Stage4InferenceScenarioName) -> Result<ControlFixture, String> {
    let spec = control_world_spec(scenario);
    let mut world = build_world(spec)?;
    let graph = compile_graph(&scenario_graph(scenario), &graph_limits())
        .map_err(|error| format!("{} graph compilation failed: {error}", scenario.label()))?;
    let plan = GraphExecutionPlan::build(&graph)
        .map_err(|error| format!("{} execution planning failed: {error}", scenario.label()))?;
    if plan.input_size() != 19 + 4 * spec.sensor_bins {
        return Err("sensor and graph input widths disagree".to_owned());
    }
    let count = scenario.population_count();
    let mut population = Vec::new();
    population
        .try_reserve_exact(count)
        .map_err(|_| "population allocation failed".to_owned())?;
    let mut brains = Vec::new();
    brains
        .try_reserve_exact(count)
        .map_err(|_| "brain allocation failed".to_owned())?;
    let mut candidates = Vec::new();
    candidates
        .try_reserve_exact(count)
        .map_err(|_| "candidate allocation failed".to_owned())?;
    let mut weight_hash = Sha256::new();
    let mut recurrent_hash = Sha256::new();
    let mut unique_weight_blocks = BTreeSet::new();
    let mut unique_initial_recurrent_blocks = BTreeSet::new();
    for slot in 0..count {
        let handle = BrainHandle {
            id: slot as u64 + 1,
            epoch: FIXTURE_EPOCH,
        };
        world.snakes[slot].brain = Some(handle);
        let slot_u32 = u32::try_from(slot).map_err(|_| "population slot exceeds u32".to_owned())?;
        let mut weights = try_zeroed(plan.total_parameters(), "genome weights")?;
        let mut genome_hash = Sha256::new();
        for (index, value) in weights.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Weight, slot, index);
            weight_hash.update(value.to_le_bytes());
            genome_hash.update(value.to_le_bytes());
        }
        let mut recurrent = try_zeroed(plan.total_state_size(), "brain recurrent state")?;
        let mut brain_recurrent_hash = Sha256::new();
        for (index, value) in recurrent.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Recurrent, slot, index);
            recurrent_hash.update(value.to_le_bytes());
            brain_recurrent_hash.update(value.to_le_bytes());
        }
        unique_weight_blocks.insert(<[u8; 32]>::from(genome_hash.finalize()));
        unique_initial_recurrent_blocks.insert(<[u8; 32]>::from(brain_recurrent_hash.finalize()));
        population.push(PopulationGenome {
            slot: slot_u32,
            brain: handle,
            lineage: GenomeLineage {
                genome_id: slot as u64 + 20_001,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: slot as f64,
            weights: weights.into_boxed_slice(),
        });
        brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::PopulationSlot(slot_u32),
            non_population_weights: None,
            recurrent: recurrent.into_boxed_slice(),
        });
    }
    for slot in (0..count).rev() {
        candidates.push(CalculationCandidateIndex::new(slot, slot));
    }
    if unique_weight_blocks.len() != count {
        return Err("combined fixture does not have one unique weight block per brain".to_owned());
    }
    if plan.total_state_size() != 0 && unique_initial_recurrent_blocks.len() != count {
        return Err(
            "combined fixture does not have one unique initial recurrent block per brain"
                .to_owned(),
        );
    }

    let sensor = SensorEvaluator::new(SensorConfig {
        bins: spec.sensor_bins,
        ..SensorConfig::default()
    })
    .map_err(|error| format!("sensor construction failed: {error}"))?;
    let mut generation = SensorGenerationState::new();
    generation
        .update_after_step(&world)
        .map_err(|error| format!("generation sensor initialization failed: {error}"))?;
    let pipeline = NeuralControlPipeline::try_new(count, sensor, plan, usize::MAX)
        .map_err(|error| format!("control pipeline construction failed: {error}"))?;
    let world_sha256 = digest_world(&world);
    Ok(ControlFixture {
        world,
        population,
        brains,
        generation,
        candidates,
        pipeline,
        sensor_scratch: SensorScratch::default(),
        index_config: SensorIndexConfig {
            body_cell_size: BODY_CELL_SIZE,
            pellet_cell_size: PELLET_CELL_SIZE,
            maximum_body_entries: MAXIMUM_BODY_ENTRIES,
            maximum_pellet_entries: MAXIMUM_PELLET_ENTRIES,
        },
        world_sha256,
        weights_sha256: hex_bytes(weight_hash.finalize()),
        initial_recurrent_sha256: hex_bytes(recurrent_hash.finalize()),
        unique_weight_blocks: unique_weight_blocks.len(),
        unique_initial_recurrent_blocks: unique_initial_recurrent_blocks.len(),
    })
}

fn execute_boundary(
    fixture: &mut ControlFixture,
    key: CalculationBatchKey,
    allocation_snapshot: &impl Fn() -> u64,
    proof: Option<&mut ProofCapture>,
) -> Result<BoundarySample, String> {
    let complete_allocations_before = allocation_snapshot();
    let complete_started = Instant::now();

    let index_allocations_before = allocation_snapshot();
    let index_started = Instant::now();
    let indexed = IndexedSensorWorld::build(&fixture.world, fixture.index_config)
        .map_err(|error| format!("combined index build failed: {error}"))?;
    let index_elapsed = index_started.elapsed();
    let index_allocations_after = allocation_snapshot();
    let body_index = indexed.body_index().diagnostics();
    let pellet_index = indexed.pellet_index().diagnostics();

    let evaluate_allocations_before = allocation_snapshot();
    let evaluate_started = Instant::now();
    let batch = fixture
        .pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &fixture.candidates,
                indexed_world: &indexed,
                generation: &fixture.generation,
                population: &fixture.population,
                brains: &fixture.brains,
                reset_brains: &[],
            },
            &mut fixture.sensor_scratch,
        )
        .map_err(|error| format!("combined sensing/inference failed: {error}"))?;
    let consumed_output = batch
        .outputs()
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>();
    if !consumed_output.is_finite() {
        return Err("combined output accumulator is not finite".to_owned());
    }
    if let Some(capture) = proof {
        for value in batch.observations() {
            capture.observation_hash.update(value.to_le_bytes());
        }
        for values in batch.outputs().chunks_exact(2) {
            capture.output_hash.update(values[0].to_le_bytes());
            capture.output_hash.update(values[1].to_le_bytes());
            capture
                .output_pairs
                .insert((values[0].to_bits(), values[1].to_bits()));
        }
        for diagnostics in batch.diagnostics() {
            capture.sensor_work.add(*diagnostics);
        }
    }
    let evaluate_elapsed = evaluate_started.elapsed();
    let evaluate_allocations_after = allocation_snapshot();
    drop(indexed);

    let commit_allocations_before = allocation_snapshot();
    let commit_started = Instant::now();
    fixture
        .pipeline
        .commit_state(key, &mut fixture.world, &mut fixture.brains)
        .map_err(|error| format!("combined state commit failed: {error}"))?;
    let commit_elapsed = commit_started.elapsed();
    let commit_allocations_after = allocation_snapshot();
    let complete_elapsed = complete_started.elapsed();
    let complete_allocations_after = allocation_snapshot();

    Ok(BoundarySample {
        complete_ms: duration_ms(complete_elapsed),
        index_ms: duration_ms(index_elapsed),
        evaluate_ms: duration_ms(evaluate_elapsed),
        commit_ms: duration_ms(commit_elapsed),
        complete_allocations: complete_allocations_after
            .saturating_sub(complete_allocations_before),
        index_allocations: index_allocations_after.saturating_sub(index_allocations_before),
        evaluate_allocations: evaluate_allocations_after
            .saturating_sub(evaluate_allocations_before),
        commit_allocations: commit_allocations_after.saturating_sub(commit_allocations_before),
        consumed_output,
        body_index,
        pellet_index,
    })
}

fn control_world_spec(scenario: Stage4InferenceScenarioName) -> Stage4SensingScenarioSpec {
    Stage4SensingScenarioSpec {
        evolved_snakes: scenario.population_count(),
        baseline_snakes: BASELINE_SNAKES,
        body_points_per_snake: BODY_POINTS_PER_SNAKE,
        pellets: PELLETS,
        sensor_bins: if scenario.is_large() { 32 } else { 16 },
        description: "combined corrected sensor-v3 and heterogeneous neural-control fixture",
    }
}

fn control_description(scenario: Stage4InferenceScenarioName) -> String {
    let graph = if scenario.is_large() {
        "four-hidden-layer 256-wide MLP, GRU-96 and Dense-2"
    } else {
        "83-64-64 MLP, GRU-16 and Dense-2"
    };
    format!(
        "{} due differently weighted evolved brains, {} baseline snakes, five-point bodies, 3,500 pellets, corrected v3/{}-bin sensing, {graph}",
        scenario.population_count(),
        BASELINE_SNAKES,
        if scenario.is_large() { 32 } else { 16 }
    )
}

fn validate_options(options: &Stage4ControlEvidenceOptions) -> Result<(), String> {
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

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn checked_product(left: usize, right: usize, label: &'static str) -> Result<usize, String> {
    left.checked_mul(right)
        .ok_or_else(|| format!("{label} overflowed"))
}

fn try_zeroed(count: usize, label: &'static str) -> Result<Vec<f32>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed for {count} floats"))?;
    values.resize(count, 0.0);
    Ok(values)
}

fn reserve_samples(count: usize, label: &'static str) -> Result<Vec<f64>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed"))?;
    Ok(values)
}

fn reserve_allocation_samples(count: usize, label: &'static str) -> Result<Vec<u64>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed"))?;
    Ok(values)
}

fn digest_f32(values: impl IntoIterator<Item = f32>) -> String {
    let mut digest = Sha256::new();
    for value in values {
        digest.update(value.to_le_bytes());
    }
    hex_bytes(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_scenario_shapes_match_sensor_and_graph_contracts() {
        for scenario in [
            Stage4InferenceScenarioName::P0,
            Stage4InferenceScenarioName::P1,
            Stage4InferenceScenarioName::P2,
            Stage4InferenceScenarioName::P3,
        ] {
            let fixture = build_fixture(scenario).unwrap();
            let spec = control_world_spec(scenario);
            assert_eq!(fixture.population.len(), scenario.population_count());
            assert_eq!(fixture.brains.len(), scenario.population_count());
            assert_eq!(fixture.world.snakes.len(), spec.total_snakes());
            assert_eq!(
                fixture.pipeline.inference().input_size(),
                scenario.input_size()
            );
            assert_eq!(fixture.pipeline.inference().output_size(), 2);
            assert!(fixture
                .population
                .iter()
                .zip(&fixture.brains)
                .all(|(genome, brain)| genome.brain == brain.handle));
        }
    }

    #[test]
    fn p0_combined_boundary_uses_real_corrected_stateful_path() {
        let report = run_stage4_control_evidence(
            Stage4ControlEvidenceOptions {
                scenario: Stage4InferenceScenarioName::P0,
                warmup_passes: 1,
                measured_passes: 2,
                evidence_environment: "development".to_owned(),
                command: vec!["test".to_owned()],
            },
            || 0,
        )
        .unwrap();
        assert_eq!(report.workload.due_neural_brains, 55);
        assert_eq!(report.workload.unique_weight_blocks, 55);
        assert_eq!(report.workload.unique_initial_recurrent_blocks, 55);
        assert_eq!(report.result.proof_sensor_work.samples, 55);
        assert_eq!(report.result.proof_delivery_boundaries_changed, 55);
        assert!(report.result.distinct_proof_output_pairs > 1);
        assert!(report.result.every_due_delivery_boundary_committed);
        assert!(report.memory.capacities_stable_after_warmup);
        assert_eq!(report.path.napi_calls_per_boundary, 0);
        assert!(!report.workload.actual_complete_world_step);
    }
}
