//! Deterministic Stage 5 complete single-worker fixed-step evidence.
//!
//! This test-hook-only fixture assembles one admitted Rust authority from the
//! same P0-P3 graph and world shapes used by the Stage 4 measurements, then
//! drives the real nonterminal coordinator through control, corrected sensing,
//! heterogeneous inference, movement, food, collision, accounting, and the
//! one authoritative publication. It intentionally excludes Node, N-API,
//! browser/RL delivery, generation transition, frame packing, persistence, and
//! the later persistent calculation-worker pool.

use super::baseline::{BaselineLifecycleConfig, BaselineLifecycleState};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::graph::GraphBundle;
use super::inference::InferenceMathBackend;
use super::inference_fixture::{
    fixture_value, graph_limits, scenario_graph, FixtureValueKind, Stage4InferenceScenarioName,
};
use super::physics::PhysicsPhaseAllocations;
use super::rng::labelled_stream;
use super::running_step::{
    RunningStepCoordinator, RunningStepInputs, RunningStepPhaseAllocations,
    RunningStepPhaseTimings, RunningStepProgress,
};
use super::sensing_fixture::{
    allocation_distribution, build_world, cpu_usage, digest_world, distribution, hex_bytes,
    linux_os_release_value, linux_process_status_bytes, linux_total_memory_bytes,
    process_cpu_snapshot, system_cpu_model, system_hostname, Stage4AllocationDistribution,
    Stage4CpuUsage, Stage4SensingDistribution, Stage4SensingScenarioSpec,
};
use super::sensors::SensorGenerationState;
use super::state::{
    estimate_state_memory, normalized_config_hash, normalized_settings_schema_hash, AllocatorState,
    AuthoritativeState, AuthorityPhase, BaselineRngState, BaselineStrategyState, BrainHandle,
    BrainOwner, BrainRuntimeState, ContractVersions, FixedStepContinuationState, GenerationState,
    GenomeLineage, NormalizedEngineConfig, NormalizedSettingValue, PopulationGenome,
    RngStateBundle, RunIdentity, SnakeKind, StateAdmissionPolicy, StateCandidate,
    StateMemoryEstimate, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START, CHECKPOINT_VERSION,
    ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START, GENERATION_BOUNDARY_VERSION,
    NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION, RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION,
    SENSOR_VERSION, SERIALIZER_VERSION,
};
use super::step_config::{fixture_default_settings, RunningStepWorkLimits};
use super::world_step::WorldStepDiagnostics;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::mem::size_of;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Version of the complete single-worker-step benchmark fixture.
pub const STAGE5_STEP_FIXTURE_VERSION: u32 = 2;
/// Population/brain epoch used by every P0-P3 fixture.
const FIXTURE_POPULATION_EPOCH: u64 = 2;
/// Stable root seed used by every fixture stream.
const FIXTURE_SEED: u32 = 42;
/// Current source-shaped baseline count.
const BASELINE_SNAKES: usize = 10;
/// Current source-shaped body length.
const BODY_POINTS_PER_SNAKE: usize = 5;
/// Current source-shaped pellet load.
const PELLETS: usize = 3_500;
/// Largest number of warm plus measured stateful steps kept below early-end time.
const MAXIMUM_TOTAL_STEPS: usize = 300;

/// Operator-supplied complete-step benchmark controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage5StepEvidenceOptions {
    /// Approved P0-P3 scenario.
    pub scenario: Stage4InferenceScenarioName,
    /// Explicit neural arithmetic implementation bound into run identity.
    pub math_backend: InferenceMathBackend,
    /// Untimed stateful fixed steps.
    pub warmup_steps: usize,
    /// Individually timed stateful fixed steps.
    pub measured_steps: usize,
    /// Development or owner-target-vm provenance declaration.
    pub evidence_environment: String,
    /// Original executable arguments retained with the report.
    pub command: Vec<String>,
}

/// Complete Stage 5 single-worker fixed-step evidence document.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepEvidence {
    /// Stable document family.
    pub schema: &'static str,
    /// Evidence schema version.
    pub version: u32,
    /// Honest measured-result classification.
    pub evidence_class: String,
    /// Explicitly excluded production boundaries.
    pub caveat: &'static str,
    /// Exact source/build identity.
    pub source: Stage5StepSource,
    /// Target and process environment.
    pub environment: Stage5StepEnvironment,
    /// Deterministic workload shape and proof identity.
    pub workload: Stage5StepWorkload,
    /// Exact runtime path exercised.
    pub path: Stage5StepPath,
    /// Admitted and packed-memory facts.
    pub memory: Stage5StepMemory,
    /// Complete-step measurements and continuation proof.
    pub result: Stage5StepResult,
    /// Original evidence command.
    pub command: Vec<String>,
}

/// Compiled source and target identity.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepSource {
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
pub struct Stage5StepEnvironment {
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

/// Deterministic P0-P3 state identity and dimensions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepWorkload {
    pub fixture_version: u32,
    pub fixture_class: &'static str,
    pub scenario: &'static str,
    pub description: String,
    pub evolved_population: usize,
    pub baseline_snakes: usize,
    pub total_live_snakes: usize,
    pub body_points_per_snake: usize,
    pub initial_pellets: usize,
    pub sensor_bins: usize,
    pub sensor_input_size: usize,
    pub parameters_per_brain: usize,
    pub recurrent_floats_per_brain: usize,
    pub graph_layout_sha256: String,
    pub initial_world_sha256: String,
    pub weights_sha256: String,
    pub initial_recurrent_sha256: String,
    pub unique_weight_blocks: usize,
    pub unique_initial_recurrent_blocks: usize,
    pub warmup_steps: usize,
    pub measured_steps: usize,
}

/// Coarse ownership and boundary facts for the measured path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepPath {
    pub authority_owner: &'static str,
    pub graph_traversal_owner: &'static str,
    pub math_backend: &'static str,
    pub calculation_workers: usize,
    pub napi_calls_per_step: usize,
    pub complete_corrected_sensing: bool,
    pub complete_heterogeneous_inference: bool,
    pub complete_movement_food_collision: bool,
    pub complete_authority_publication: bool,
    pub scheduler_in_measured_interval: bool,
    pub node_bridge_in_measured_interval: bool,
    pub browser_or_rl_in_measured_interval: bool,
    pub generation_transition_in_measured_interval: bool,
}

/// Checked memory estimate and packed neural payload size.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepMemory {
    pub packed_weight_bytes: usize,
    pub authoritative_recurrent_bytes: usize,
    pub admitted_structural_bytes: usize,
    pub admitted_graph_bytes: usize,
    pub admitted_spatial_bytes: usize,
    pub admitted_scratch_bytes: usize,
    pub admitted_total_bytes: usize,
    pub admission_ceiling_bytes: usize,
}

/// Selected complete-step work counts and retained capacities.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepDiagnostics {
    pub controls: usize,
    pub baseline_observations: usize,
    pub external_observations: usize,
    pub neural_evaluations: usize,
    pub neural_held: usize,
    pub physics_substeps_expected: usize,
    pub physics_substeps_completed: usize,
    pub snakes: usize,
    pub body_points: usize,
    pub pellets: usize,
    pub deaths_in_last_substep: usize,
    pub awards_in_last_substep: usize,
    pub control_snake_capacity: usize,
    pub control_brain_capacity: usize,
    pub physics_snake_capacity: usize,
    pub physics_body_capacity: usize,
    pub physics_pellet_capacity: usize,
    pub pellet_index_estimated_bytes: usize,
}

impl From<WorldStepDiagnostics> for Stage5StepDiagnostics {
    fn from(source: WorldStepDiagnostics) -> Self {
        Self {
            controls: source.control.selection.controls,
            baseline_observations: source.control.selection.baseline_observations,
            external_observations: source.control.selection.external_observations,
            neural_evaluations: source.control.selection.neural_evaluations,
            neural_held: source.control.selection.neural_held,
            physics_substeps_expected: source.physics.expected_substeps,
            physics_substeps_completed: source.physics.completed_substeps,
            snakes: source.physics.snakes,
            body_points: source.physics.body_points,
            pellets: source.physics.pellets,
            deaths_in_last_substep: source.last_substep.deaths,
            awards_in_last_substep: source.last_substep.awards,
            control_snake_capacity: source.control.snake_capacity,
            control_brain_capacity: source.control.brain_capacity,
            physics_snake_capacity: source.physics.snake_capacity,
            physics_body_capacity: source.physics.body_point_capacity,
            physics_pellet_capacity: source.physics.pellet_capacity,
            pellet_index_estimated_bytes: source.pellet_index.estimated_bytes,
        }
    }
}

/// Measured single-worker-step result and proof of real authority progression.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepResult {
    pub complete_fixed_step_ms: Stage4SensingDistribution,
    pub coarse_phase_ms: Stage5StepPhaseDistributions,
    pub allocator_operations_per_complete_step: Stage4AllocationDistribution,
    pub coarse_phase_allocator_operations: Stage5StepPhaseAllocationDistributions,
    pub physics_phase_allocator_operations: Stage5PhysicsAllocationDistributions,
    pub cpu: Stage4CpuUsage,
    pub measured_wall_seconds: f64,
    pub measured_simulated_seconds: f64,
    pub achieved_simulated_seconds_per_wall_second: f64,
    pub initial_completed_step: u64,
    pub final_completed_step: u64,
    pub final_generation_elapsed_seconds: f64,
    pub final_alive_evolved: usize,
    pub final_alive_baselines: usize,
    pub final_pellets: usize,
    pub distinct_final_population_actions: usize,
    pub final_world_sha256: String,
    pub final_recurrent_sha256: String,
    pub authority_changed: bool,
    pub recurrent_state_changed: bool,
    pub first_measured_step: Stage5StepDiagnostics,
    pub final_measured_step: Stage5StepDiagnostics,
    pub anti_optimization_state_accumulator: f64,
}

/// Coarse non-overlapping phase distributions captured by test hooks.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepPhaseDistributions {
    pub authority_begin_ms: Stage4SensingDistribution,
    pub prefix_ms: Stage4SensingDistribution,
    pub control_selection_ms: Stage4SensingDistribution,
    pub control_commit_ms: Stage4SensingDistribution,
    pub world_step_ms: Stage4SensingDistribution,
    pub generation_guard_ms: Stage4SensingDistribution,
    pub publication_ms: Stage4SensingDistribution,
    pub unattributed_ms: Stage4SensingDistribution,
}

/// Allocation-operation distributions for the same coarse phase boundaries.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5StepPhaseAllocationDistributions {
    pub authority_begin: Stage4AllocationDistribution,
    pub prefix: Stage4AllocationDistribution,
    pub control_selection: Stage4AllocationDistribution,
    pub control_commit: Stage4AllocationDistribution,
    pub world_step: Stage4AllocationDistribution,
    pub generation_guard: Stage4AllocationDistribution,
    pub publication: Stage4AllocationDistribution,
    pub unattributed: Stage4AllocationDistribution,
}

/// Fine-grained allocation-operation distributions inside physics staging.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage5PhysicsAllocationDistributions {
    pub begin: Stage4AllocationDistribution,
    pub pellet_index: Stage4AllocationDistribution,
    pub movement: Stage4AllocationDistribution,
    pub food: Stage4AllocationDistribution,
    pub collision: Stage4AllocationDistribution,
    pub effects: Stage4AllocationDistribution,
    pub result_application: Stage4AllocationDistribution,
    pub accept: Stage4AllocationDistribution,
    pub finalize: Stage4AllocationDistribution,
}

/// State and reusable coordinator owned outside measured intervals.
struct StepFixture {
    authority: AuthoritativeState,
    coordinator: RunningStepCoordinator,
    initial_world_sha256: String,
    weights_sha256: String,
    initial_recurrent_sha256: String,
    unique_weight_blocks: usize,
    unique_initial_recurrent_blocks: usize,
    admitted_memory: StateMemoryEstimate,
    admission_ceiling_bytes: usize,
}

/// One complete measured authority publication.
struct StepSample {
    elapsed_ms: f64,
    allocations: u64,
    phases: RunningStepPhaseTimings,
    phase_allocations: RunningStepPhaseAllocations,
    physics_phase_allocations: PhysicsPhaseAllocations,
    diagnostics: WorldStepDiagnostics,
    consumed_state: f64,
}

/// Run one stateful single-worker complete-step benchmark.
pub fn run_stage5_step_evidence(
    options: Stage5StepEvidenceOptions,
    allocation_snapshot: fn() -> u64,
) -> Result<Stage5StepEvidence, String> {
    validate_options(&options)?;
    let target_triple = crate::native_addon_build_target();
    let build_profile = crate::native_addon_build_profile();
    let build_class = crate::native_addon_build_class();
    if build_profile != "release" {
        return Err("Stage 5 complete-step evidence requires a release build".to_owned());
    }
    if build_class != "test-hooks" {
        return Err("Stage 5 complete-step evidence requires a test-hooks build".to_owned());
    }
    if target_triple != "x86_64-pc-windows-msvc" && target_triple != "x86_64-unknown-linux-gnu" {
        return Err(format!(
            "Stage 5 complete-step evidence does not support target {target_triple}"
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

    if !options.math_backend.is_available() {
        return Err(format!(
            "requested math backend {} is unavailable on this CPU",
            options.math_backend.label()
        ));
    }
    let mut fixture = build_fixture(options.scenario, options.math_backend)?;
    fixture
        .coordinator
        .set_allocation_snapshot(allocation_snapshot);
    let actual_math_backend = fixture.coordinator.math_backend();
    if actual_math_backend != options.math_backend {
        return Err(format!(
            "coordinator selected {} but evidence requested {}",
            actual_math_backend.label(),
            options.math_backend.label()
        ));
    }
    let fixture_rss_bytes = linux_process_status_bytes("VmRSS:");
    let initial_completed_step = fixture.authority.state().generation.completed_step;
    for _ in 0..options.warmup_steps {
        execute_step(&mut fixture, allocation_snapshot)?;
    }

    let mut timing_samples = reserve_f64(options.measured_steps, "step timings")?;
    let mut authority_begin_samples = reserve_f64(options.measured_steps, "authority timings")?;
    let mut prefix_samples = reserve_f64(options.measured_steps, "prefix timings")?;
    let mut control_selection_samples =
        reserve_f64(options.measured_steps, "control-selection timings")?;
    let mut control_commit_samples = reserve_f64(options.measured_steps, "control-commit timings")?;
    let mut world_step_samples = reserve_f64(options.measured_steps, "world-step timings")?;
    let mut generation_guard_samples =
        reserve_f64(options.measured_steps, "generation-guard timings")?;
    let mut publication_samples = reserve_f64(options.measured_steps, "publication timings")?;
    let mut unattributed_samples = reserve_f64(options.measured_steps, "unattributed timings")?;
    let mut allocation_samples = reserve_u64(options.measured_steps, "step allocations")?;
    let mut authority_begin_allocation_samples =
        reserve_u64(options.measured_steps, "authority allocation samples")?;
    let mut prefix_allocation_samples =
        reserve_u64(options.measured_steps, "prefix allocation samples")?;
    let mut control_selection_allocation_samples = reserve_u64(
        options.measured_steps,
        "control-selection allocation samples",
    )?;
    let mut control_commit_allocation_samples =
        reserve_u64(options.measured_steps, "control-commit allocation samples")?;
    let mut world_step_allocation_samples =
        reserve_u64(options.measured_steps, "world-step allocation samples")?;
    let mut generation_guard_allocation_samples = reserve_u64(
        options.measured_steps,
        "generation-guard allocation samples",
    )?;
    let mut publication_allocation_samples =
        reserve_u64(options.measured_steps, "publication allocation samples")?;
    let mut unattributed_allocation_samples =
        reserve_u64(options.measured_steps, "unattributed allocation samples")?;
    let mut physics_begin_allocation_samples =
        reserve_u64(options.measured_steps, "physics-begin allocation samples")?;
    let mut pellet_index_allocation_samples =
        reserve_u64(options.measured_steps, "pellet-index allocation samples")?;
    let mut movement_allocation_samples =
        reserve_u64(options.measured_steps, "movement allocation samples")?;
    let mut food_allocation_samples =
        reserve_u64(options.measured_steps, "food allocation samples")?;
    let mut collision_allocation_samples =
        reserve_u64(options.measured_steps, "collision allocation samples")?;
    let mut effects_allocation_samples =
        reserve_u64(options.measured_steps, "effects allocation samples")?;
    let mut result_application_allocation_samples = reserve_u64(
        options.measured_steps,
        "physics-result-application allocation samples",
    )?;
    let mut physics_accept_allocation_samples =
        reserve_u64(options.measured_steps, "physics-accept allocation samples")?;
    let mut physics_finalize_allocation_samples = reserve_u64(
        options.measured_steps,
        "physics-finalize allocation samples",
    )?;
    let mut first_diagnostics = None;
    let mut final_diagnostics = None;
    let mut anti_optimization_state_accumulator = 0.0_f64;
    let cpu_before = process_cpu_snapshot();
    let measured_started = Instant::now();
    for _ in 0..options.measured_steps {
        let sample = execute_step(&mut fixture, allocation_snapshot)?;
        timing_samples.push(sample.elapsed_ms);
        authority_begin_samples.push(sample.phases.authority_begin_ms);
        prefix_samples.push(sample.phases.prefix_ms);
        control_selection_samples.push(sample.phases.control_selection_ms);
        control_commit_samples.push(sample.phases.control_commit_ms);
        world_step_samples.push(sample.phases.world_step_ms);
        generation_guard_samples.push(sample.phases.generation_guard_ms);
        publication_samples.push(sample.phases.publication_ms);
        let attributed = phase_total(sample.phases);
        unattributed_samples.push((sample.elapsed_ms - attributed).max(0.0));
        allocation_samples.push(sample.allocations);
        authority_begin_allocation_samples.push(sample.phase_allocations.authority_begin);
        prefix_allocation_samples.push(sample.phase_allocations.prefix);
        control_selection_allocation_samples.push(sample.phase_allocations.control_selection);
        control_commit_allocation_samples.push(sample.phase_allocations.control_commit);
        world_step_allocation_samples.push(sample.phase_allocations.world_step);
        generation_guard_allocation_samples.push(sample.phase_allocations.generation_guard);
        publication_allocation_samples.push(sample.phase_allocations.publication);
        unattributed_allocation_samples.push(
            sample
                .allocations
                .saturating_sub(phase_allocation_total(sample.phase_allocations)),
        );
        physics_begin_allocation_samples.push(sample.physics_phase_allocations.begin);
        pellet_index_allocation_samples.push(sample.physics_phase_allocations.pellet_index);
        movement_allocation_samples.push(sample.physics_phase_allocations.movement);
        food_allocation_samples.push(sample.physics_phase_allocations.food);
        collision_allocation_samples.push(sample.physics_phase_allocations.collision);
        effects_allocation_samples.push(sample.physics_phase_allocations.effects);
        result_application_allocation_samples
            .push(sample.physics_phase_allocations.result_application);
        physics_accept_allocation_samples.push(sample.physics_phase_allocations.accept);
        physics_finalize_allocation_samples.push(sample.physics_phase_allocations.finalize);
        first_diagnostics.get_or_insert(sample.diagnostics);
        final_diagnostics = Some(sample.diagnostics);
        anti_optimization_state_accumulator += sample.consumed_state;
    }
    let measured_elapsed = measured_started.elapsed();
    let cpu = cpu_usage(cpu_before, process_cpu_snapshot(), measured_elapsed);
    if options.evidence_environment == "owner-target-vm" && cpu.process_cpu_seconds.is_none() {
        return Err("owner-target-vm CPU evidence could not be sampled".to_owned());
    }

    let state = fixture.authority.state();
    let expected_completed = u64::try_from(options.warmup_steps)
        .ok()
        .and_then(|warmup| warmup.checked_add(options.measured_steps as u64))
        .and_then(|steps| initial_completed_step.checked_add(steps))
        .ok_or_else(|| "expected completed-step count overflowed".to_owned())?;
    if state.generation.completed_step != expected_completed {
        return Err(format!(
            "authority completed step {} but fixture expected {expected_completed}",
            state.generation.completed_step
        ));
    }
    let final_alive_evolved = alive_count(state, SnakeKind::Evolved);
    let final_alive_baselines = alive_count(state, SnakeKind::Baseline);
    if final_alive_evolved != options.scenario.population_count()
        || final_alive_baselines != BASELINE_SNAKES
    {
        return Err(format!(
            "source-shaped benchmark lost live workload members (evolved {final_alive_evolved}/{}, baseline {final_alive_baselines}/{BASELINE_SNAKES})",
            options.scenario.population_count()
        ));
    }
    let final_world_sha256 = digest_world(&state.world);
    let final_recurrent_sha256 = digest_recurrent(&state.brains);
    let authority_changed = final_world_sha256 != fixture.initial_world_sha256;
    let recurrent_state_changed = final_recurrent_sha256 != fixture.initial_recurrent_sha256;
    if !authority_changed || !recurrent_state_changed {
        return Err(
            "complete-step benchmark did not advance both world and recurrent authority".to_owned(),
        );
    }
    let distinct_final_population_actions = state
        .world
        .snakes
        .iter()
        .filter(|snake| snake.kind == SnakeKind::Evolved)
        .map(|snake| (snake.turn.to_bits(), snake.input_boost))
        .collect::<BTreeSet<_>>()
        .len();
    if distinct_final_population_actions <= 1 {
        return Err("complete-step fixture did not retain distinct population actions".to_owned());
    }
    if !anti_optimization_state_accumulator.is_finite() {
        return Err("anti-optimization state accumulator is not finite".to_owned());
    }

    let measured_wall_seconds = measured_elapsed.as_secs_f64();
    let measured_simulated_seconds =
        options.measured_steps as f64 * state.config.fixed_step_seconds;
    let achieved_simulated_seconds_per_wall_second = if measured_wall_seconds > 0.0 {
        measured_simulated_seconds / measured_wall_seconds
    } else {
        0.0
    };
    let graph = fixture.authority.graph();
    let packed_weight_bytes = checked_product(
        options.scenario.population_count(),
        graph.total_parameters,
        "packed weight float count",
    )?
    .checked_mul(size_of::<f32>())
    .ok_or_else(|| "packed weight byte count overflowed".to_owned())?;
    let authoritative_recurrent_bytes = checked_product(
        options.scenario.population_count(),
        graph.total_state_size,
        "recurrent float count",
    )?
    .checked_mul(size_of::<f32>())
    .ok_or_else(|| "recurrent byte count overflowed".to_owned())?;
    let spec = scenario_world_spec(options.scenario);
    let evidence_class = if options.evidence_environment == "owner-target-vm" {
        format!(
            "new measured target-VM single-worker Rust complete-step result ({})",
            actual_math_backend.label()
        )
    } else {
        format!(
            "new measured development-machine single-worker Rust complete-step result ({})",
            actual_math_backend.label()
        )
    };

    Ok(Stage5StepEvidence {
        schema: "slither-stage5-rust-single-worker-complete-step-benchmark-v2",
        version: 2,
        evidence_class,
        caveat: "Source-shaped deterministic synthetic single-worker fixed-step benchmark with an explicitly recorded neural math backend. It drives one admitted Rust authority through corrected sensing, distinct stateful population brains, baseline control, movement, food, continuous collision, effects, accounting, and atomic publication. Allocation distributions count allocation operations by measured phase; they do not correlate individual operations with retained-capacity changes and therefore do not prove either a fixed allocation floor or an allocation-free steady state. The benchmark does not include the scheduler pump, N-API/Node bridge, browser or Protocol 2 RL client, frame packing, generation transition/evolution, persistence, a sustained round, the persistent calculation-worker pool, or production cutover.",
        source: Stage5StepSource {
            native_build_identifier: crate::native_addon_build_identifier(),
            native_source_sha256: crate::native_addon_source_sha256(),
            native_build_contract_sha256: crate::native_addon_build_contract_sha256(),
            target_triple,
            rustc_version: crate::native_addon_rustc_version(),
            build_profile,
            build_class,
        },
        environment: Stage5StepEnvironment {
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
        workload: Stage5StepWorkload {
            fixture_version: STAGE5_STEP_FIXTURE_VERSION,
            fixture_class: "source-shaped deterministic synthetic admitted authority",
            scenario: options.scenario.label(),
            description: scenario_description(options.scenario),
            evolved_population: spec.evolved_snakes,
            baseline_snakes: spec.baseline_snakes,
            total_live_snakes: spec.total_snakes(),
            body_points_per_snake: spec.body_points_per_snake,
            initial_pellets: spec.pellets,
            sensor_bins: spec.sensor_bins,
            sensor_input_size: 19 + 4 * spec.sensor_bins,
            parameters_per_brain: graph.total_parameters,
            recurrent_floats_per_brain: graph.total_state_size,
            graph_layout_sha256: hex_bytes(graph.layout_digest_sha256),
            initial_world_sha256: fixture.initial_world_sha256,
            weights_sha256: fixture.weights_sha256,
            initial_recurrent_sha256: fixture.initial_recurrent_sha256,
            unique_weight_blocks: fixture.unique_weight_blocks,
            unique_initial_recurrent_blocks: fixture.unique_initial_recurrent_blocks,
            warmup_steps: options.warmup_steps,
            measured_steps: options.measured_steps,
        },
        path: Stage5StepPath {
            authority_owner: "Rust",
            graph_traversal_owner: "Rust",
            math_backend: actual_math_backend.label(),
            calculation_workers: 1,
            napi_calls_per_step: 0,
            complete_corrected_sensing: true,
            complete_heterogeneous_inference: true,
            complete_movement_food_collision: true,
            complete_authority_publication: true,
            scheduler_in_measured_interval: false,
            node_bridge_in_measured_interval: false,
            browser_or_rl_in_measured_interval: false,
            generation_transition_in_measured_interval: false,
        },
        memory: Stage5StepMemory {
            packed_weight_bytes,
            authoritative_recurrent_bytes,
            admitted_structural_bytes: fixture.admitted_memory.structural_bytes,
            admitted_graph_bytes: fixture.admitted_memory.graph_bytes,
            admitted_spatial_bytes: fixture.admitted_memory.spatial_bytes,
            admitted_scratch_bytes: fixture.admitted_memory.scratch_bytes,
            admitted_total_bytes: fixture.admitted_memory.total_bytes,
            admission_ceiling_bytes: fixture.admission_ceiling_bytes,
        },
        result: Stage5StepResult {
            complete_fixed_step_ms: distribution(timing_samples)?,
            coarse_phase_ms: Stage5StepPhaseDistributions {
                authority_begin_ms: distribution(authority_begin_samples)?,
                prefix_ms: distribution(prefix_samples)?,
                control_selection_ms: distribution(control_selection_samples)?,
                control_commit_ms: distribution(control_commit_samples)?,
                world_step_ms: distribution(world_step_samples)?,
                generation_guard_ms: distribution(generation_guard_samples)?,
                publication_ms: distribution(publication_samples)?,
                unattributed_ms: distribution(unattributed_samples)?,
            },
            allocator_operations_per_complete_step: allocation_distribution(&allocation_samples)?,
            coarse_phase_allocator_operations: Stage5StepPhaseAllocationDistributions {
                authority_begin: allocation_distribution(&authority_begin_allocation_samples)?,
                prefix: allocation_distribution(&prefix_allocation_samples)?,
                control_selection: allocation_distribution(
                    &control_selection_allocation_samples,
                )?,
                control_commit: allocation_distribution(&control_commit_allocation_samples)?,
                world_step: allocation_distribution(&world_step_allocation_samples)?,
                generation_guard: allocation_distribution(
                    &generation_guard_allocation_samples,
                )?,
                publication: allocation_distribution(&publication_allocation_samples)?,
                unattributed: allocation_distribution(&unattributed_allocation_samples)?,
            },
            physics_phase_allocator_operations: Stage5PhysicsAllocationDistributions {
                begin: allocation_distribution(&physics_begin_allocation_samples)?,
                pellet_index: allocation_distribution(&pellet_index_allocation_samples)?,
                movement: allocation_distribution(&movement_allocation_samples)?,
                food: allocation_distribution(&food_allocation_samples)?,
                collision: allocation_distribution(&collision_allocation_samples)?,
                effects: allocation_distribution(&effects_allocation_samples)?,
                result_application: allocation_distribution(
                    &result_application_allocation_samples,
                )?,
                accept: allocation_distribution(&physics_accept_allocation_samples)?,
                finalize: allocation_distribution(&physics_finalize_allocation_samples)?,
            },
            cpu,
            measured_wall_seconds,
            measured_simulated_seconds,
            achieved_simulated_seconds_per_wall_second,
            initial_completed_step,
            final_completed_step: state.generation.completed_step,
            final_generation_elapsed_seconds: state.generation.elapsed_seconds,
            final_alive_evolved,
            final_alive_baselines,
            final_pellets: state.world.pellets.len(),
            distinct_final_population_actions,
            final_world_sha256,
            final_recurrent_sha256,
            authority_changed,
            recurrent_state_changed,
            first_measured_step: first_diagnostics
                .ok_or_else(|| "first measured diagnostics missing".to_owned())?
                .into(),
            final_measured_step: final_diagnostics
                .ok_or_else(|| "final measured diagnostics missing".to_owned())?
                .into(),
            anti_optimization_state_accumulator,
        },
        command: options.command,
    })
}

fn validate_options(options: &Stage5StepEvidenceOptions) -> Result<(), String> {
    if options.measured_steps == 0 {
        return Err("measured steps must be positive".to_owned());
    }
    let total = options
        .warmup_steps
        .checked_add(options.measured_steps)
        .ok_or_else(|| "total benchmark steps overflowed".to_owned())?;
    if total > MAXIMUM_TOTAL_STEPS {
        return Err(format!(
            "warmup plus measured steps must not exceed {MAXIMUM_TOTAL_STEPS}"
        ));
    }
    if options.evidence_environment != "development"
        && options.evidence_environment != "owner-target-vm"
    {
        return Err("environment must be development or owner-target-vm".to_owned());
    }
    Ok(())
}

fn build_fixture(
    scenario: Stage4InferenceScenarioName,
    math_backend: InferenceMathBackend,
) -> Result<StepFixture, String> {
    let spec = scenario_world_spec(scenario);
    let mut world = build_world(spec)?;
    for (offset, snake) in world.snakes[spec.evolved_snakes..].iter_mut().enumerate() {
        snake.id = BASELINE_ENTITY_ID_START
            .checked_add(offset as u64)
            .ok_or_else(|| "baseline fixture ID overflowed".to_owned())?;
        snake.baseline_strategy = Some(BaselineStrategyState::Roam);
    }

    let graph = Arc::new(
        GraphBundle::compile(scenario_graph(scenario), &graph_limits())
            .map_err(|error| format!("{} graph compilation failed: {error}", scenario.label()))?,
    );
    let mut population = Vec::new();
    population
        .try_reserve_exact(spec.evolved_snakes)
        .map_err(|_| "population fixture allocation failed".to_owned())?;
    let mut brains = Vec::new();
    brains
        .try_reserve_exact(spec.evolved_snakes)
        .map_err(|_| "brain fixture allocation failed".to_owned())?;
    let mut weights_sha = Sha256::new();
    let mut unique_weights = BTreeSet::new();
    let mut unique_recurrent = BTreeSet::new();
    for slot in 0..spec.evolved_snakes {
        let slot_u32 = u32::try_from(slot).map_err(|_| "fixture slot exceeds u32".to_owned())?;
        let handle = BrainHandle {
            id: slot as u64 + 1,
            epoch: FIXTURE_POPULATION_EPOCH,
        };
        world.snakes[slot].brain = Some(handle);
        let mut weights = try_f32_buffer(graph.total_parameters, "genome weights")?;
        let mut one_weight_sha = Sha256::new();
        for (index, value) in weights.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Weight, slot, index);
            weights_sha.update(value.to_le_bytes());
            one_weight_sha.update(value.to_le_bytes());
        }
        unique_weights.insert(<[u8; 32]>::from(one_weight_sha.finalize()));
        let mut recurrent = try_f32_buffer(graph.total_state_size, "recurrent state")?;
        let mut one_recurrent_sha = Sha256::new();
        for (index, value) in recurrent.iter_mut().enumerate() {
            *value = fixture_value(scenario, FixtureValueKind::Recurrent, slot, index);
            one_recurrent_sha.update(value.to_le_bytes());
        }
        unique_recurrent.insert(<[u8; 32]>::from(one_recurrent_sha.finalize()));
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
    if unique_weights.len() != spec.evolved_snakes
        || (graph.total_state_size != 0 && unique_recurrent.len() != spec.evolved_snakes)
    {
        return Err(
            "fixture does not contain one distinct weight/state block per brain".to_owned(),
        );
    }

    let baseline_lifecycle = BaselineLifecycleState::initialize_after_complete_spawn(
        BaselineLifecycleConfig {
            slot_count: BASELINE_SNAKES,
            ..BaselineLifecycleConfig::typescript_defaults()
        },
        &world,
    )
    .map_err(|error| format!("baseline fixture initialization failed: {error}"))?;
    let mut sensor_generation = SensorGenerationState::new();
    sensor_generation
        .update_after_step(&world)
        .map_err(|error| format!("sensor generation initialization failed: {error}"))?;

    let mut settings = fixture_default_settings(spec.evolved_snakes, BASELINE_SNAKES);
    if scenario.is_large() {
        let setting = settings
            .iter_mut()
            .find(|setting| setting.path == "sense.bubbleBins")
            .ok_or_else(|| "fixture bubble-bin setting missing".to_owned())?;
        setting.value = NormalizedSettingValue::Integer(32);
    }
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)
        .map_err(|error| format!("fixture settings-schema hash failed: {error}"))?;
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256: settings_schema_sha256.clone(),
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 3_500.0,
        population_count: spec.evolved_snakes,
        baseline_count: BASELINE_SNAKES,
        max_world_snakes: spec.total_snakes() + 64,
        max_non_population_brains: 64,
        max_body_points: 100_000,
        max_pellets: 25_000,
        spatial_index_bytes: 256 * 1024 * 1024,
        worker_scratch_bytes: 512 * 1024 * 1024,
        checkpoint_scratch_bytes: 512 * 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let config_hash = normalized_config_hash(&config)
        .map_err(|error| format!("fixture config hash failed: {error}"))?;
    let build_identifier = crate::native_addon_build_identifier();
    let source_sha256 = crate::native_addon_source_sha256();
    let target_triple = crate::native_addon_build_target();
    let build_profile = crate::native_addon_build_profile();
    let build_class = crate::native_addon_build_class();
    let rustc_version = crate::native_addon_rustc_version();
    let build_contract_sha256 = crate::native_addon_build_contract_sha256();
    let baseline_rngs = (0..BASELINE_SNAKES)
        .map(|slot| BaselineRngState {
            slot: slot as u32,
            state: labelled_stream(f64::from(FIXTURE_SEED), &format!("baseline:{slot}"))
                .export_state(),
        })
        .collect::<Vec<_>>();
    let max_general_id = world
        .snakes
        .iter()
        .filter(|snake| snake.id < EXTERNAL_ENTITY_ID_START)
        .map(|snake| snake.id)
        .chain(world.pellets.iter().map(|pellet| pellet.id))
        .max()
        .unwrap_or(0);
    let max_frame_id = world
        .snakes
        .iter()
        .map(|snake| snake.frame_v1_id)
        .max()
        .unwrap_or(0);
    let initial_world_sha256 = digest_world(&world);
    let initial_recurrent_sha256 = digest_recurrent(&brains);
    let candidate = StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: graph.layout_version,
        },
        identity: RunIdentity {
            run_id: format!("stage5-step-fixture-{}", scenario.label()),
            seed: FIXTURE_SEED,
            config_revision: 1,
            config_hash,
            source_revision: build_identifier.clone(),
            engine_build_id: build_identifier.clone(),
            source_sha256: source_sha256.clone(),
            target_triple: target_triple.clone(),
            build_profile: build_profile.clone(),
            build_class: build_class.clone(),
            rustc_version: rustc_version.clone(),
            build_contract_sha256: build_contract_sha256.clone(),
            math_backend: math_backend.label().to_owned(),
        },
        config,
        phase: AuthorityPhase::Running,
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation: 1,
            completed_step: 0,
            population_epoch: FIXTURE_POPULATION_EPOCH,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: 0.0,
        },
        fixed_step: FixedStepContinuationState {
            ambient_pellet_accumulator: 0.0,
            baseline_lifecycle,
            sensor_generation,
        },
        rng: RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: labelled_stream(f64::from(FIXTURE_SEED), "world").export_state(),
            evolution: labelled_stream(f64::from(FIXTURE_SEED), "evolution").export_state(),
            external_controller: labelled_stream(f64::from(FIXTURE_SEED), "external-controller")
                .export_state(),
            baselines: baseline_rngs,
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: max_general_id
                .checked_add(1)
                .ok_or_else(|| "general entity continuation overflowed".to_owned())?,
            next_brain_id: spec.evolved_snakes as u64 + 1,
            next_genome_id: spec.evolved_snakes as u64 + 20_001,
            next_controller_lease_id: 1,
            next_frame_v1_id: max_frame_id
                .checked_add(1)
                .ok_or_else(|| "frame continuation overflowed".to_owned())?,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START + BASELINE_SNAKES as u64,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        },
        population,
        brains,
        world,
    };
    let admitted_memory = estimate_state_memory(&candidate, &graph)
        .map_err(|error| format!("fixture memory estimation failed: {error}"))?;
    let admission_ceiling_bytes = admitted_memory
        .total_bytes
        .checked_add(1024 * 1024 * 1024)
        .ok_or_else(|| "fixture admission ceiling overflowed".to_owned())?;
    let policy = StateAdmissionPolicy {
        memory_ceiling_bytes: admission_ceiling_bytes,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: source_sha256,
        expected_target_triple: target_triple,
        expected_build_profile: build_profile,
        expected_build_class: build_class,
        expected_rustc_version: rustc_version,
        expected_build_contract_sha256: build_contract_sha256,
        expected_math_backend: math_backend.label().to_owned(),
        expected_settings_schema_sha256: settings_schema_sha256,
    };
    let authority = AuthoritativeState::validate_and_own(candidate, graph, &policy)
        .map_err(|error| format!("fixture state admission failed: {error}"))?;
    let coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .map_err(|error| format!("fixture coordinator construction failed: {error}"))?;
    Ok(StepFixture {
        authority,
        coordinator,
        initial_world_sha256,
        weights_sha256: hex_bytes(weights_sha.finalize()),
        initial_recurrent_sha256,
        unique_weight_blocks: unique_weights.len(),
        unique_initial_recurrent_blocks: unique_recurrent.len(),
        admitted_memory,
        admission_ceiling_bytes,
    })
}

fn execute_step(
    fixture: &mut StepFixture,
    allocation_snapshot: fn() -> u64,
) -> Result<StepSample, String> {
    let source_completed_step = fixture.authority.state().generation.completed_step;
    let wall_now_ms = source_completed_step
        .checked_add(1)
        .and_then(|step| step.checked_mul(17))
        .ok_or_else(|| "fixture wall clock overflowed".to_owned())?;
    let allocations_before = allocation_snapshot();
    let started = Instant::now();
    let progress = fixture
        .coordinator
        .advance_nonterminal(
            &mut fixture.authority,
            RunningStepInputs {
                wall_now_ms,
                wall_accumulator_seconds: 0.0,
            },
        )
        .map_err(|error| format!("complete fixed step failed: {error}"))?;
    let (publication, diagnostics) = match progress {
        RunningStepProgress::Published(outcome) => (outcome.publication, *outcome.diagnostics),
        RunningStepProgress::ExternalDeliveryPending(batch) => {
            return Err(format!(
                "population/baseline fixture unexpectedly emitted {} external observations",
                batch.events().len()
            ));
        }
        RunningStepProgress::GenerationTransitionPending(_) => {
            return Err("step fixture unexpectedly reached a generation boundary".to_owned());
        }
    };
    let phases = fixture.coordinator.last_phase_timings();
    let phase_allocations = fixture.coordinator.last_phase_allocations();
    let physics_phase_allocations = fixture.coordinator.last_physics_phase_allocations();
    let elapsed = started.elapsed();
    let allocations_after = allocation_snapshot();
    let expected_completed_step = source_completed_step
        .checked_add(1)
        .ok_or_else(|| "completed step overflowed".to_owned())?;
    if publication.completed_step != expected_completed_step
        || fixture.authority.state().generation.completed_step != expected_completed_step
    {
        return Err(
            "complete fixed step did not publish the expected authority boundary".to_owned(),
        );
    }
    let selection = diagnostics.control.selection;
    if selection.neural_evaluations != alive_count(fixture.authority.state(), SnakeKind::Evolved)
        || selection.baseline_observations
            != alive_count(fixture.authority.state(), SnakeKind::Baseline)
        || selection.external_observations != 0
    {
        return Err(
            "complete fixed step did not exercise the expected control workload".to_owned(),
        );
    }
    if diagnostics.physics.expected_substeps != diagnostics.physics.completed_substeps {
        return Err("complete fixed step did not accept every collision substep".to_owned());
    }
    let consumed_state =
        fixture
            .authority
            .state()
            .world
            .snakes
            .iter()
            .fold(0.0_f64, |sum, snake| {
                sum + snake.position.x * 1.0e-9
                    + snake.position.y * 1.0e-9
                    + f64::from(snake.turn) * 1.0e-6
                    + snake.points * 1.0e-12
            });
    Ok(StepSample {
        elapsed_ms: elapsed.as_secs_f64() * 1_000.0,
        allocations: allocations_after.saturating_sub(allocations_before),
        phases,
        phase_allocations,
        physics_phase_allocations,
        diagnostics,
        consumed_state,
    })
}

fn phase_total(phases: RunningStepPhaseTimings) -> f64 {
    phases.authority_begin_ms
        + phases.prefix_ms
        + phases.control_selection_ms
        + phases.control_commit_ms
        + phases.world_step_ms
        + phases.generation_guard_ms
        + phases.publication_ms
}

fn phase_allocation_total(phases: RunningStepPhaseAllocations) -> u64 {
    phases
        .authority_begin
        .saturating_add(phases.prefix)
        .saturating_add(phases.control_selection)
        .saturating_add(phases.control_commit)
        .saturating_add(phases.world_step)
        .saturating_add(phases.generation_guard)
        .saturating_add(phases.publication)
}

fn scenario_world_spec(scenario: Stage4InferenceScenarioName) -> Stage4SensingScenarioSpec {
    Stage4SensingScenarioSpec {
        evolved_snakes: scenario.population_count(),
        baseline_snakes: BASELINE_SNAKES,
        body_points_per_snake: BODY_POINTS_PER_SNAKE,
        pellets: PELLETS,
        sensor_bins: if scenario.is_large() { 32 } else { 16 },
        description: "Stage 5 source-shaped complete single-worker fixed-step fixture",
    }
}

fn scenario_description(scenario: Stage4InferenceScenarioName) -> String {
    let brain = if scenario.is_large() {
        "147-input five-layer 256-wide MLP, GRU-96, Dense-2"
    } else {
        "83-input 64/64 MLP, GRU-16, Dense-2"
    };
    format!(
        "{} evolved snakes with different weights and recurrent state, {BASELINE_SNAKES} baseline bots, five-point bodies, {PELLETS} pellets, {brain}",
        scenario.population_count()
    )
}

fn try_f32_buffer(count: usize, label: &'static str) -> Result<Vec<f32>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed for {count} floats"))?;
    values.resize(count, 0.0);
    Ok(values)
}

fn digest_recurrent(brains: &[BrainRuntimeState]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"slither-stage5-recurrent-v1\0");
    digest.update((brains.len() as u64).to_le_bytes());
    for brain in brains {
        digest.update(brain.handle.id.to_le_bytes());
        digest.update(brain.handle.epoch.to_le_bytes());
        digest.update((brain.recurrent.len() as u64).to_le_bytes());
        for value in &brain.recurrent {
            digest.update(value.to_le_bytes());
        }
    }
    hex_bytes(digest.finalize())
}

fn alive_count(state: &StateCandidate, kind: SnakeKind) -> usize {
    state
        .world
        .snakes
        .iter()
        .filter(|snake| snake.alive && snake.kind == kind)
        .count()
}

fn checked_product(left: usize, right: usize, context: &'static str) -> Result<usize, String> {
    left.checked_mul(right)
        .ok_or_else(|| format!("{context} overflowed"))
}

fn reserve_f64(count: usize, label: &'static str) -> Result<Vec<f64>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed"))?;
    Ok(values)
}

fn reserve_u64(count: usize, label: &'static str) -> Result<Vec<u64>, String> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| format!("{label} allocation failed"))?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p0_fixture_publishes_real_complete_steps() {
        let report = run_stage5_step_evidence(
            Stage5StepEvidenceOptions {
                scenario: Stage4InferenceScenarioName::P0,
                math_backend: InferenceMathBackend::Scalar,
                warmup_steps: 1,
                measured_steps: 2,
                evidence_environment: "development".to_owned(),
                command: vec!["test".to_owned()],
            },
            || 0,
        )
        .unwrap();
        assert_eq!(report.result.final_completed_step, 3);
        assert_eq!(report.result.final_alive_evolved, 55);
        assert_eq!(report.result.final_alive_baselines, 10);
        assert_eq!(report.result.first_measured_step.neural_evaluations, 55);
        assert_eq!(report.result.first_measured_step.baseline_observations, 10);
        assert_eq!(report.path.math_backend, "rust-scalar-v1");
        assert!(report.result.authority_changed);
        assert!(report.result.recurrent_state_changed);
    }

    #[test]
    fn fixture_recurrent_digest_and_math_identity_match_the_admitted_source() {
        let fixture = build_fixture(
            Stage4InferenceScenarioName::P0,
            InferenceMathBackend::Scalar,
        )
        .unwrap();
        assert_eq!(
            fixture.coordinator.math_backend(),
            InferenceMathBackend::Scalar
        );
        assert_eq!(
            fixture.initial_recurrent_sha256,
            digest_recurrent(&fixture.authority.state().brains)
        );
    }

    #[test]
    fn excessive_stateful_window_is_rejected() {
        let error = validate_options(&Stage5StepEvidenceOptions {
            scenario: Stage4InferenceScenarioName::P0,
            math_backend: InferenceMathBackend::Scalar,
            warmup_steps: 1,
            measured_steps: MAXIMUM_TOTAL_STEPS,
            evidence_environment: "development".to_owned(),
            command: Vec::new(),
        })
        .unwrap_err();
        assert!(error.contains("must not exceed"));
    }
}
