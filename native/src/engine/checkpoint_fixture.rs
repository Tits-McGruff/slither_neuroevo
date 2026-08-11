//! Test-hook-only construction of one real managed checkpoint publication.
//!
//! This module is absent from production builds. It deliberately exercises the
//! production graph, state-admission, checkpoint codec, fsync, and immutable
//! rename path while returning only the scalar checkpoint descriptor.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::checkpoint::{
    publish_checkpoint, restore_checkpoint, CheckpointDescriptor, CheckpointLimits,
    CheckpointOperationId,
};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::graph::{
    GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
    CANONICAL_GRAPH_LAYOUT_VERSION,
};
use super::rng::{labelled_stream, SerializedRngState, StatefulRng};
use super::state::{
    normalized_config_hash, normalized_settings_schema_hash, AllocatorState, AuthoritativeState,
    AuthorityPhase, BaselineRngState, BrainHandle, BrainOwner, BrainRuntimeState, ContractVersions,
    GenerationBoundaryKind, GenerationState, GenomeLineage, NormalizedEngineConfig,
    NormalizedSetting, NormalizedSettingValue, PopulationGenome, RngStateBundle, RunIdentity,
    StateAdmissionPolicy, StateCandidate, WorldState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START,
    CHECKPOINT_VERSION, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START,
    GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
    RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, SENSOR_VERSION, SERIALIZER_VERSION,
};

/// Exact small population used by the cross-language publication fixture.
const FIXTURE_POPULATION_COUNT: usize = 4;

/// Version of the retained Stage 3 round-trip evidence document.
const ROUND_TRIP_EVIDENCE_VERSION: u32 = 1;
/// Exact default P0/P2 evolved-population count.
const REPRESENTATIVE_POPULATION_COUNT: usize = 55;
/// Exact default v3 sensor width with 16 bubble bins.
const P0_INPUT_SIZE: usize = 83;
/// Exact v3 sensor width with 32 bubble bins.
const P2_INPUT_SIZE: usize = 147;
/// Source-pinned default-graph parameter count.
const P0_PARAMETER_COUNT: usize = 13_458;
/// Source-pinned Stage 2 large-GRU parameter count.
const P2_PARAMETER_COUNT: usize = 402_914;

/// One source-matched workload exercised by the retained checkpoint runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoundTripScenario {
    /// Small run-start recurrent fixture used to catch structural regressions quickly.
    Small,
    /// Default 55-genome graph and v3/16-bin sensor shape.
    P0,
    /// Large 55-genome graph and v3/32-bin sensor shape.
    P2,
}

impl RoundTripScenario {
    /// Stable evidence label.
    const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::P0 => "P0",
            Self::P2 => "P2",
        }
    }

    /// Plain description that does not claim the synthetic weights came from owner data.
    const fn description(self) -> &'static str {
        match self {
            Self::Small => "4 genomes, small recurrent graph, run-start boundary",
            Self::P0 => {
                "55 deterministic genomes, source-matched default graph, generation boundary"
            }
            Self::P2 => {
                "55 deterministic genomes, source-matched Stage 2 large GRU graph, generation boundary"
            }
        }
    }

    /// Dense evolved-population count.
    const fn population_count(self) -> usize {
        match self {
            Self::Small => FIXTURE_POPULATION_COUNT,
            Self::P0 | Self::P2 => REPRESENTATIVE_POPULATION_COUNT,
        }
    }

    /// Stable first publication operation token.
    const fn operation_id(self) -> &'static str {
        match self {
            Self::Small => "11111111111111111111111111111111",
            Self::P0 => "22222222222222222222222222222222",
            Self::P2 => "33333333333333333333333333333333",
        }
    }

    /// Stable idempotent restored-state publication operation token.
    const fn restored_operation_id(self) -> &'static str {
        match self {
            Self::Small => "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Self::P0 => "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            Self::P2 => "cccccccccccccccccccccccccccccccc",
        }
    }
}

/// Retained cross-platform report for the three source-matched checkpoint fixtures.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage3CheckpointRoundTripEvidence {
    /// Evidence schema version.
    pub schema_version: u32,
    /// Stable artifact kind.
    pub kind: &'static str,
    /// Generation time represented as Unix epoch milliseconds.
    pub generated_at_epoch_ms: u128,
    /// Exact source/build identifier embedded by the native build.
    pub native_build_identifier: String,
    /// Platform-independent selected-native-source digest.
    pub native_source_sha256: String,
    /// Exact Cargo target triple.
    pub target_triple: String,
    /// Exact Rust compiler identity captured by the build script.
    pub rustc_version: String,
    /// Build profile used for the evidence executable.
    pub build_profile: String,
    /// Test-hook build class required by this runner.
    pub build_class: String,
    /// Operating system reported by Rust's compilation target.
    pub operating_system: &'static str,
    /// Architecture reported by Rust's compilation target.
    pub architecture: &'static str,
    /// Linux process high-water RSS when available from `/proc/self/status`.
    pub process_peak_rss_bytes: Option<u64>,
    /// Independently asserted workload results.
    pub scenarios: Vec<Stage3CheckpointScenarioEvidence>,
    /// Honest scope limitation retained beside every measured result.
    pub continuation_scope: &'static str,
    /// A real game step is intentionally unavailable before Stages 4 and 5.
    pub actual_world_step_exercised: bool,
}

/// One exact managed-checkpoint publish/restore/idempotence observation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage3CheckpointScenarioEvidence {
    /// Stable workload label.
    pub scenario: &'static str,
    /// Human-readable fixture shape.
    pub description: &'static str,
    /// Dense population count.
    pub population_count: usize,
    /// Packed parameters per genome.
    pub parameters_per_genome: usize,
    /// Recurrent-state values per brain.
    pub recurrent_state_per_brain: usize,
    /// Aggregate raw packed population weight bytes.
    pub raw_weight_bytes: u64,
    /// Aggregate raw packed recurrent-state bytes.
    pub raw_recurrent_bytes: u64,
    /// Complete managed USTAR bytes.
    pub stored_checkpoint_bytes: u64,
    /// Aggregate decoded logical-role bytes.
    pub decoded_checkpoint_bytes: u64,
    /// Measured adaptive population-weight encoding.
    pub weights_encoding: String,
    /// Measured adaptive recurrent-state encoding.
    pub recurrent_state_encoding: String,
    /// Encoding-independent logical checkpoint root.
    pub logical_root_sha256: String,
    /// Canonical compiled graph-layout digest.
    pub graph_layout_sha256: String,
    /// SHA-256 over the exact little-endian population weight bits.
    pub population_weight_sha256: String,
    /// SHA-256 over the exact little-endian recurrent-state bits.
    pub recurrent_state_sha256: String,
    /// Deterministic next RNG/allocator contract digest before publication.
    pub continuation_probe_sha256: String,
    /// Wall time spent in first publication, excluding idempotence revalidation.
    pub publish_duration_ms: f64,
    /// Wall time spent in strict restore/admission.
    pub restore_duration_ms: f64,
    /// State memory estimate admitted before publication.
    pub admitted_state_bytes: usize,
    /// Restored state memory estimate after strict admission.
    pub restored_state_bytes: usize,
    /// Full state structs and metadata compare equal after restore.
    pub state_equal: bool,
    /// Every packed population weight compares by exact Float32 bits.
    pub weight_bits_equal: bool,
    /// Every recurrent value compares by exact Float32 bits.
    pub recurrent_bits_equal: bool,
    /// Source graph and canonical compiled layout compare equal.
    pub graph_equal: bool,
    /// The next RNG/allocator continuation probe compares exactly.
    pub continuation_probe_equal: bool,
    /// Re-publishing restored content resolves to the same logical file.
    pub restored_republication_idempotent: bool,
}

/// Publish one deterministic real checkpoint through the production codec.
pub(crate) fn publish_stage3_fixture(
    managed_directory: &Path,
    operation_id: &str,
    transition_epoch: u64,
) -> Result<CheckpointDescriptor, String> {
    if env!("SLITHER_NATIVE_BUILD_CLASS") != "test-hooks" {
        return Err("Stage 3 checkpoint fixture requires a test-hooks native build".to_owned());
    }
    let graph_limits = fixture_graph_limits();
    let graph = Arc::new(
        GraphBundle::compile(fixture_graph(), &graph_limits)
            .map_err(|error| format!("fixture graph compilation failed: {error}"))?,
    );
    let settings = fixture_settings();
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)
        .map_err(|error| format!("fixture settings schema failed: {error}"))?;
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256: settings_schema_sha256.clone(),
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 1000.0,
        population_count: FIXTURE_POPULATION_COUNT,
        baseline_count: 0,
        max_world_snakes: FIXTURE_POPULATION_COUNT,
        max_non_population_brains: 0,
        max_body_points: 100_000,
        max_pellets: 10_000,
        spatial_index_bytes: 2 * 1024 * 1024,
        worker_scratch_bytes: 2 * 1024 * 1024,
        checkpoint_scratch_bytes: 16 * 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let build_identifier = env!("SLITHER_NATIVE_BUILD_IDENTIFIER").to_owned();
    let source_sha256 = env!("SLITHER_NATIVE_SOURCE_SHA256").to_owned();
    let target_triple = env!("SLITHER_NATIVE_BUILD_TARGET").to_owned();
    let build_profile = env!("SLITHER_NATIVE_BUILD_PROFILE").to_owned();
    let build_class = env!("SLITHER_NATIVE_BUILD_CLASS").to_owned();
    let rustc_version = env!("SLITHER_NATIVE_RUSTC_VERSION").to_owned();
    let build_contract_sha256 = env!("SLITHER_NATIVE_BUILD_CONTRACT_SHA256").to_owned();
    let config_hash = normalized_config_hash(&config)
        .map_err(|error| format!("fixture configuration hash failed: {error}"))?;
    let identity = RunIdentity {
        run_id: "11111111-2222-4333-8444-555555555555".to_owned(),
        seed: 7,
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
        math_backend: "rust-scalar-v1".to_owned(),
    };
    let (population, brains) = fixture_population(&graph);
    let rng = || StatefulRng::new(7.0).export_state();
    let candidate = StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: CANONICAL_GRAPH_LAYOUT_VERSION,
        },
        identity,
        config,
        phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart),
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation: 1,
            completed_step: 0,
            population_epoch: 1,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: 0.0,
        },
        rng: RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: rng(),
            evolution: rng(),
            external_controller: rng(),
            baselines: Vec::new(),
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1,
            next_brain_id: FIXTURE_POPULATION_COUNT as u64 + 1,
            next_genome_id: FIXTURE_POPULATION_COUNT as u64 + 1,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        },
        population,
        brains,
        world: WorldState::default(),
    };
    let policy = StateAdmissionPolicy {
        memory_ceiling_bytes: 256 * 1024 * 1024,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: source_sha256,
        expected_target_triple: target_triple,
        expected_build_profile: build_profile,
        expected_build_class: build_class,
        expected_rustc_version: rustc_version,
        expected_build_contract_sha256: build_contract_sha256,
        expected_math_backend: "rust-scalar-v1".to_owned(),
        expected_settings_schema_sha256: settings_schema_sha256,
    };
    let state = AuthoritativeState::validate_and_own(candidate, graph, &policy)
        .map_err(|error| format!("fixture state admission failed: {error}"))?;
    let boundary = state
        .checkpoint_boundary()
        .map_err(|error| format!("fixture boundary failed: {error}"))?;
    let operation_id =
        CheckpointOperationId::parse(operation_id.to_owned()).map_err(|error| error.to_string())?;
    publish_checkpoint(
        managed_directory,
        operation_id,
        transition_epoch,
        boundary,
        &fixture_checkpoint_limits(),
        &graph_limits,
        &policy,
    )
    .map_err(|error| error.to_string())
}

/// Small recurrent graph that proves weights and recurrent state both use the real codec.
fn fixture_graph() -> GraphSpec {
    GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".to_owned(),
                kind: GraphNodeKind::Input { output_size: 3 },
            },
            GraphNodeSpec {
                id: "memory".to_owned(),
                kind: GraphNodeKind::Gru {
                    input_size: 3,
                    hidden_size: 2,
                },
            },
            GraphNodeSpec {
                id: "head".to_owned(),
                kind: GraphNodeKind::Dense {
                    input_size: 2,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".to_owned(),
                to: "memory".to_owned(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "memory".to_owned(),
                to: "head".to_owned(),
                from_port: None,
                to_port: None,
            },
        ],
        outputs: vec![GraphOutputRef {
            node_id: "head".to_owned(),
            port: None,
        }],
        output_size: 2,
    }
}

/// Complete path-sorted settings schema retained by the fixture checkpoint.
fn fixture_settings() -> Vec<NormalizedSetting> {
    vec![
        NormalizedSetting {
            path: "baselineBots.count".to_owned(),
            value: NormalizedSettingValue::Integer(0),
        },
        NormalizedSetting {
            path: "brain.sensorVersion".to_owned(),
            value: NormalizedSettingValue::Integer(i64::from(SENSOR_VERSION)),
        },
        NormalizedSetting {
            path: "simSpeed".to_owned(),
            value: NormalizedSettingValue::Float(1.0),
        },
        NormalizedSetting {
            path: "snakeCount".to_owned(),
            value: NormalizedSettingValue::Integer(FIXTURE_POPULATION_COUNT as i64),
        },
        NormalizedSetting {
            path: "worldRadius".to_owned(),
            value: NormalizedSettingValue::Integer(1000),
        },
    ]
}

/// Create distinct packed weights plus exact-zero generation-boundary recurrent buffers.
fn fixture_population(graph: &GraphBundle) -> (Vec<PopulationGenome>, Vec<BrainRuntimeState>) {
    let mut population = Vec::with_capacity(FIXTURE_POPULATION_COUNT);
    let mut brains = Vec::with_capacity(FIXTURE_POPULATION_COUNT);
    for slot in 0..FIXTURE_POPULATION_COUNT {
        let brain = BrainHandle {
            id: slot as u64 + 1,
            epoch: 1,
        };
        let weights = (0..graph.total_parameters)
            .map(|index| {
                let mantissa = ((slot as u32).wrapping_mul(8191) ^ (index as u32).wrapping_mul(37))
                    & 0x007f_ffff;
                f32::from_bits(0x3f00_0000 | mantissa)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let recurrent = vec![0.0_f32; graph.total_state_size].into_boxed_slice();
        population.push(PopulationGenome {
            slot: slot as u32,
            brain,
            lineage: GenomeLineage {
                genome_id: slot as u64 + 1,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: slot as f64 / 10.0,
            weights,
        });
        brains.push(BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(slot as u32),
            non_population_weights: None,
            recurrent,
        });
    }
    (population, brains)
}

/// Reviewed graph limits for the fixed small fixture.
fn fixture_graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 16,
        max_edges: 32,
        max_graph_outputs: 4,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 1024,
        max_tensor_width: 1024,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 16,
        max_parameter_floats: 1_000_000,
        max_recurrent_state_floats: 16_384,
        max_canonical_layout_bytes: 128 * 1024,
        max_architecture_key_bytes: 256 * 1024,
    }
}

/// Reviewed checkpoint bounds for the fixed small fixture.
fn fixture_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 64 * 1024 * 1024,
        max_manifest_bytes: 64 * 1024,
        max_state_bytes: 256 * 1024,
        max_graph_bytes: 512 * 1024,
        max_population_index_bytes: 1024 * 1024,
        max_population_count: 512,
        max_setting_count: 32,
        max_baseline_rng_count: 32,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 2 * 1024 * 1024,
        max_weight_floats: 8 * 1024 * 1024,
        max_recurrent_floats: 1024 * 1024,
        max_numeric_stored_bytes: 48 * 1024 * 1024,
        max_numeric_candidate_bytes: 48 * 1024 * 1024,
        max_total_decoded_bytes: 64 * 1024 * 1024,
    }
}

/// Run the real managed-file codec against small, P0, and P2 generation boundaries.
///
/// This feature-gated evidence path deliberately leaves the immutable checkpoint
/// files beneath `managed_root` until the caller has serialized the report. The
/// standalone runner owns a disposable root and removes it on exit.
pub fn run_stage3_roundtrip_evidence(
    managed_root: &Path,
) -> Result<Stage3CheckpointRoundTripEvidence, String> {
    if env!("SLITHER_NATIVE_BUILD_CLASS") != "test-hooks" {
        return Err("Stage 3 round-trip evidence requires a test-hooks native build".to_owned());
    }
    std::fs::create_dir_all(managed_root)
        .map_err(|error| format!("unable to create evidence managed root: {error}"))?;
    let generated_at_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
        .as_millis();
    let mut scenarios = Vec::with_capacity(3);
    for scenario in [
        RoundTripScenario::Small,
        RoundTripScenario::P0,
        RoundTripScenario::P2,
    ] {
        scenarios.push(run_round_trip_scenario(managed_root, scenario)?);
    }
    Ok(Stage3CheckpointRoundTripEvidence {
        schema_version: ROUND_TRIP_EVIDENCE_VERSION,
        kind: "stage3-managed-checkpoint-roundtrip",
        generated_at_epoch_ms,
        native_build_identifier: env!("SLITHER_NATIVE_BUILD_IDENTIFIER").to_owned(),
        native_source_sha256: env!("SLITHER_NATIVE_SOURCE_SHA256").to_owned(),
        target_triple: env!("SLITHER_NATIVE_BUILD_TARGET").to_owned(),
        rustc_version: env!("SLITHER_NATIVE_RUSTC_VERSION").to_owned(),
        build_profile: env!("SLITHER_NATIVE_BUILD_PROFILE").to_owned(),
        build_class: env!("SLITHER_NATIVE_BUILD_CLASS").to_owned(),
        operating_system: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        process_peak_rss_bytes: process_peak_rss_bytes(),
        scenarios,
        continuation_scope: "exact generation-boundary graph, RNG, allocator, packed-weight and zero-recurrent continuation; no sensing, inference, movement, collision, evolution, or world step",
        actual_world_step_exercised: false,
    })
}

/// Publish, restore, compare, and idempotently republish one exact fixture.
fn run_round_trip_scenario(
    managed_root: &Path,
    scenario: RoundTripScenario,
) -> Result<Stage3CheckpointScenarioEvidence, String> {
    let directory = managed_root.join(scenario.label());
    std::fs::create_dir(&directory).map_err(|error| {
        format!(
            "unable to create {} fixture directory: {error}",
            scenario.label()
        )
    })?;
    let (state, policy) = build_round_trip_state(scenario)?;
    let graph_limits = round_trip_graph_limits();
    let checkpoint_limits = round_trip_checkpoint_limits();
    let original_continuation = continuation_probe_sha256(&state)?;
    let original_weight_sha256 = population_weight_sha256(&state);
    let original_recurrent_sha256 = recurrent_state_sha256(&state);
    let admitted_state_bytes = state.memory_estimate().total_bytes;

    let publish_started = Instant::now();
    let descriptor = publish_checkpoint(
        &directory,
        CheckpointOperationId::parse(scenario.operation_id().to_owned())
            .map_err(|error| error.to_string())?,
        1,
        state
            .checkpoint_boundary()
            .map_err(|error| format!("{} boundary rejected: {error}", scenario.label()))?,
        &checkpoint_limits,
        &graph_limits,
        &policy,
    )
    .map_err(|error| format!("{} publication failed: {error}", scenario.label()))?;
    let publish_duration_ms = publish_started.elapsed().as_secs_f64() * 1000.0;
    let path = directory.join(&descriptor.relative_filename);
    let stored_file_bytes = std::fs::metadata(&path)
        .map_err(|error| format!("{} managed file is missing: {error}", scenario.label()))?
        .len();

    let restore_started = Instant::now();
    let restored = restore_checkpoint(&path, &checkpoint_limits, &graph_limits, &policy)
        .map_err(|error| format!("{} restore failed: {error}", scenario.label()))?;
    let restore_duration_ms = restore_started.elapsed().as_secs_f64() * 1000.0;
    let state_equal = restored.state.state() == state.state();
    let weight_bits_equal = population_weight_bits_equal(&state, &restored.state);
    let recurrent_bits_equal = recurrent_state_bits_equal(&state, &restored.state);
    let graph_equal = restored.state.graph_spec() == state.graph_spec()
        && restored.state.graph() == state.graph();
    let restored_continuation = continuation_probe_sha256(&restored.state)?;
    let continuation_probe_equal = restored_continuation == original_continuation;
    let restored_weight_sha256 = population_weight_sha256(&restored.state);
    let restored_recurrent_sha256 = recurrent_state_sha256(&restored.state);
    if !state_equal
        || !weight_bits_equal
        || !recurrent_bits_equal
        || !graph_equal
        || !continuation_probe_equal
        || restored_weight_sha256 != original_weight_sha256
        || restored_recurrent_sha256 != original_recurrent_sha256
    {
        return Err(format!(
            "{} restored content differs from the admitted boundary",
            scenario.label()
        ));
    }
    if restored.content.logical_root_sha256 != descriptor.logical_root_sha256
        || restored.content.relative_filename != descriptor.relative_filename
        || stored_file_bytes != parse_hex_u64(&descriptor.stored_byte_count_hex, "stored bytes")?
    {
        return Err(format!(
            "{} descriptor and strictly restored content disagree",
            scenario.label()
        ));
    }

    let restored_descriptor = publish_checkpoint(
        &directory,
        CheckpointOperationId::parse(scenario.restored_operation_id().to_owned())
            .map_err(|error| error.to_string())?,
        2,
        restored
            .state
            .checkpoint_boundary()
            .map_err(|error| format!("{} restored boundary rejected: {error}", scenario.label()))?,
        &checkpoint_limits,
        &graph_limits,
        &policy,
    )
    .map_err(|error| {
        format!(
            "{} restored re-publication failed: {error}",
            scenario.label()
        )
    })?;
    let restored_republication_idempotent = restored_descriptor.logical_root_sha256
        == descriptor.logical_root_sha256
        && restored_descriptor.relative_filename == descriptor.relative_filename;
    if !restored_republication_idempotent {
        return Err(format!(
            "{} restored content did not republish idempotently",
            scenario.label()
        ));
    }

    let raw_weight_bytes = checked_float_bytes(
        scenario
            .population_count()
            .checked_mul(state.graph().total_parameters)
            .ok_or_else(|| format!("{} weight count overflows", scenario.label()))?,
        "population weights",
    )?;
    let raw_recurrent_bytes = checked_float_bytes(
        scenario
            .population_count()
            .checked_mul(state.graph().total_state_size)
            .ok_or_else(|| format!("{} recurrent count overflows", scenario.label()))?,
        "recurrent state",
    )?;

    Ok(Stage3CheckpointScenarioEvidence {
        scenario: scenario.label(),
        description: scenario.description(),
        population_count: scenario.population_count(),
        parameters_per_genome: state.graph().total_parameters,
        recurrent_state_per_brain: state.graph().total_state_size,
        raw_weight_bytes,
        raw_recurrent_bytes,
        stored_checkpoint_bytes: stored_file_bytes,
        decoded_checkpoint_bytes: parse_hex_u64(
            &descriptor.decoded_byte_count_hex,
            "decoded bytes",
        )?,
        weights_encoding: descriptor.weights_encoding.as_str().to_owned(),
        recurrent_state_encoding: descriptor.recurrent_state_encoding.as_str().to_owned(),
        logical_root_sha256: descriptor.logical_root_sha256,
        graph_layout_sha256: descriptor.graph_layout_sha256,
        population_weight_sha256: original_weight_sha256,
        recurrent_state_sha256: original_recurrent_sha256,
        continuation_probe_sha256: original_continuation,
        publish_duration_ms,
        restore_duration_ms,
        admitted_state_bytes,
        restored_state_bytes: restored.state.memory_estimate().total_bytes,
        state_equal,
        weight_bits_equal,
        recurrent_bits_equal,
        graph_equal,
        continuation_probe_equal,
        restored_republication_idempotent,
    })
}

/// Construct one exact source-shaped generation-boundary state and matching admission policy.
fn build_round_trip_state(
    scenario: RoundTripScenario,
) -> Result<(AuthoritativeState, StateAdmissionPolicy), String> {
    let graph = Arc::new(
        GraphBundle::compile(round_trip_graph(scenario), &round_trip_graph_limits())
            .map_err(|error| format!("{} graph compilation failed: {error}", scenario.label()))?,
    );
    match scenario {
        RoundTripScenario::Small => {}
        RoundTripScenario::P0 if graph.total_parameters == P0_PARAMETER_COUNT => {}
        RoundTripScenario::P2 if graph.total_parameters == P2_PARAMETER_COUNT => {}
        _ => {
            return Err(format!(
                "{} source-matched graph has unexpected parameter count {}",
                scenario.label(),
                graph.total_parameters
            ));
        }
    }
    let baseline_count = if scenario == RoundTripScenario::Small {
        0
    } else {
        10
    };
    let settings = round_trip_settings(scenario, baseline_count);
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)
        .map_err(|error| format!("{} settings schema failed: {error}", scenario.label()))?;
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256: settings_schema_sha256.clone(),
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 1000.0,
        population_count: scenario.population_count(),
        baseline_count,
        max_world_snakes: scenario
            .population_count()
            .checked_add(baseline_count)
            .ok_or_else(|| format!("{} world snake count overflows", scenario.label()))?,
        max_non_population_brains: 0,
        max_body_points: 250_000,
        max_pellets: 12_000,
        spatial_index_bytes: 4 * 1024 * 1024,
        worker_scratch_bytes: 16 * 1024 * 1024,
        checkpoint_scratch_bytes: 32 * 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let build_identifier = env!("SLITHER_NATIVE_BUILD_IDENTIFIER").to_owned();
    let source_sha256 = env!("SLITHER_NATIVE_SOURCE_SHA256").to_owned();
    let target_triple = env!("SLITHER_NATIVE_BUILD_TARGET").to_owned();
    let build_profile = env!("SLITHER_NATIVE_BUILD_PROFILE").to_owned();
    let build_class = env!("SLITHER_NATIVE_BUILD_CLASS").to_owned();
    let rustc_version = env!("SLITHER_NATIVE_RUSTC_VERSION").to_owned();
    let build_contract_sha256 = env!("SLITHER_NATIVE_BUILD_CONTRACT_SHA256").to_owned();
    let config_hash = normalized_config_hash(&config)
        .map_err(|error| format!("{} configuration hash failed: {error}", scenario.label()))?;
    let generation = if scenario == RoundTripScenario::Small {
        1
    } else {
        25
    };
    let population_epoch = generation;
    let (population, brains) = round_trip_population(&graph, scenario, population_epoch)?;
    let candidate = StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: CANONICAL_GRAPH_LAYOUT_VERSION,
        },
        identity: RunIdentity {
            run_id: match scenario {
                RoundTripScenario::Small => "10000000-0000-4000-8000-000000000001",
                RoundTripScenario::P0 => "10000000-0000-4000-8000-000000000002",
                RoundTripScenario::P2 => "10000000-0000-4000-8000-000000000003",
            }
            .to_owned(),
            seed: 0x5a17_c0de,
            config_revision: generation,
            config_hash,
            source_revision: build_identifier.clone(),
            engine_build_id: build_identifier.clone(),
            source_sha256: source_sha256.clone(),
            target_triple: target_triple.clone(),
            build_profile: build_profile.clone(),
            build_class: build_class.clone(),
            rustc_version: rustc_version.clone(),
            build_contract_sha256: build_contract_sha256.clone(),
            math_backend: "rust-scalar-v1".to_owned(),
        },
        config,
        phase: AuthorityPhase::GenerationBoundary(if scenario == RoundTripScenario::Small {
            GenerationBoundaryKind::RunStart
        } else {
            GenerationBoundaryKind::Generation
        }),
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation,
            completed_step: if scenario == RoundTripScenario::Small {
                0
            } else {
                432_000
            },
            population_epoch,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: if scenario == RoundTripScenario::Small {
                0.0
            } else {
                17_500.25
            },
        },
        rng: round_trip_rng_bundle(baseline_count),
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 4_000_001,
            next_brain_id: scenario.population_count() as u64 + 10_001,
            next_genome_id: scenario.population_count() as u64 + 20_001,
            next_controller_lease_id: 301,
            next_frame_v1_id: 60_001,
            next_external_id: EXTERNAL_ENTITY_ID_START + 301,
            next_baseline_id: BASELINE_ENTITY_ID_START + 301,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START + 301,
        },
        population,
        brains,
        world: WorldState::default(),
    };
    let policy = StateAdmissionPolicy {
        memory_ceiling_bytes: 768 * 1024 * 1024,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: source_sha256,
        expected_target_triple: target_triple,
        expected_build_profile: build_profile,
        expected_build_class: build_class,
        expected_rustc_version: rustc_version,
        expected_build_contract_sha256: build_contract_sha256,
        expected_math_backend: "rust-scalar-v1".to_owned(),
        expected_settings_schema_sha256: settings_schema_sha256,
    };
    let state = AuthoritativeState::validate_and_own(candidate, graph, &policy)
        .map_err(|error| format!("{} state admission failed: {error}", scenario.label()))?;
    Ok((state, policy))
}

/// Build the exact source-shaped graph for one evidence scenario.
fn round_trip_graph(scenario: RoundTripScenario) -> GraphSpec {
    match scenario {
        RoundTripScenario::Small => fixture_graph(),
        RoundTripScenario::P0 => GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input {
                        output_size: P0_INPUT_SIZE,
                    },
                },
                GraphNodeSpec {
                    id: "mlp".to_owned(),
                    kind: GraphNodeKind::Mlp {
                        input_size: P0_INPUT_SIZE,
                        hidden_sizes: vec![64],
                        output_size: 64,
                    },
                },
                GraphNodeSpec {
                    id: "gru".to_owned(),
                    kind: GraphNodeKind::Gru {
                        input_size: 64,
                        hidden_size: 16,
                    },
                },
                GraphNodeSpec {
                    id: "head".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 16,
                        output_size: 2,
                    },
                },
            ],
            edges: linear_edges(&["input", "mlp", "gru", "head"]),
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
        RoundTripScenario::P2 => GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input {
                        output_size: P2_INPUT_SIZE,
                    },
                },
                GraphNodeSpec {
                    id: "features".to_owned(),
                    kind: GraphNodeKind::Mlp {
                        input_size: P2_INPUT_SIZE,
                        hidden_sizes: vec![256, 256, 256, 256],
                        output_size: 256,
                    },
                },
                GraphNodeSpec {
                    id: "memory".to_owned(),
                    kind: GraphNodeKind::Gru {
                        input_size: 256,
                        hidden_size: 96,
                    },
                },
                GraphNodeSpec {
                    id: "output".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 96,
                        output_size: 2,
                    },
                },
            ],
            edges: linear_edges(&["input", "features", "memory", "output"]),
            outputs: vec![GraphOutputRef {
                node_id: "output".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
    }
}

/// Build implicit-port edges for one linear graph chain.
fn linear_edges(ids: &[&str]) -> Vec<GraphEdge> {
    ids.windows(2)
        .map(|pair| GraphEdge {
            from: pair[0].to_owned(),
            to: pair[1].to_owned(),
            from_port: None,
            to_port: None,
        })
        .collect()
}

/// Complete path-sorted normalized settings for one evidence workload.
fn round_trip_settings(
    scenario: RoundTripScenario,
    baseline_count: usize,
) -> Vec<NormalizedSetting> {
    vec![
        NormalizedSetting {
            path: "baselineBots.count".to_owned(),
            value: NormalizedSettingValue::Integer(baseline_count as i64),
        },
        NormalizedSetting {
            path: "brain.bubbleBins".to_owned(),
            value: NormalizedSettingValue::Integer(if scenario == RoundTripScenario::P2 {
                32
            } else {
                16
            }),
        },
        NormalizedSetting {
            path: "brain.profile".to_owned(),
            value: NormalizedSettingValue::Text(scenario.label().to_owned()),
        },
        NormalizedSetting {
            path: "brain.sensorVersion".to_owned(),
            value: NormalizedSettingValue::Integer(i64::from(SENSOR_VERSION)),
        },
        NormalizedSetting {
            path: "simSpeed".to_owned(),
            value: NormalizedSettingValue::Float(1.0),
        },
        NormalizedSetting {
            path: "snakeCount".to_owned(),
            value: NormalizedSettingValue::Integer(scenario.population_count() as i64),
        },
        NormalizedSetting {
            path: "worldRadius".to_owned(),
            value: NormalizedSettingValue::Integer(1000),
        },
    ]
}

/// Construct deterministic finite packed weights and exact-zero boundary recurrent state.
fn round_trip_population(
    graph: &GraphBundle,
    scenario: RoundTripScenario,
    population_epoch: u64,
) -> Result<(Vec<PopulationGenome>, Vec<BrainRuntimeState>), String> {
    let population_count = scenario.population_count();
    let mut population = Vec::new();
    population
        .try_reserve_exact(population_count)
        .map_err(|_| format!("{} population allocation failed", scenario.label()))?;
    let mut brains = Vec::new();
    brains
        .try_reserve_exact(population_count)
        .map_err(|_| format!("{} brain allocation failed", scenario.label()))?;
    for slot in 0..population_count {
        let brain = BrainHandle {
            id: slot as u64 + 10_001,
            epoch: population_epoch,
        };
        let mut weights = Vec::new();
        weights
            .try_reserve_exact(graph.total_parameters)
            .map_err(|_| format!("{} weight allocation failed", scenario.label()))?;
        for index in 0..graph.total_parameters {
            weights.push(deterministic_weight(scenario, slot, index));
        }
        population.push(PopulationGenome {
            slot: slot as u32,
            brain,
            lineage: GenomeLineage {
                genome_id: slot as u64 + 20_001,
                birth_generation: population_epoch,
                parent_a: None,
                parent_b: None,
            },
            fitness: (slot as f64 + 1.0) * 17.25,
            weights: weights.into_boxed_slice(),
        });
        brains.push(BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(slot as u32),
            non_population_weights: None,
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        });
    }
    Ok((population, brains))
}

/// Generate one deterministic finite high-entropy Float32 value without allocations.
fn deterministic_weight(scenario: RoundTripScenario, slot: usize, index: usize) -> f32 {
    let scenario_word = match scenario {
        RoundTripScenario::Small => 0x243f_6a88,
        RoundTripScenario::P0 => 0x85a3_08d3,
        RoundTripScenario::P2 => 0x1319_8a2e,
    };
    let mut word = scenario_word
        ^ (slot as u32 + 1).wrapping_mul(0x9e37_79b9)
        ^ (index as u32 + 1).wrapping_mul(0x7f4a_7c15);
    word ^= word.wrapping_shl(13);
    word ^= word >> 17;
    word ^= word.wrapping_shl(5);
    let magnitude = f32::from_bits(0x3e80_0000 | (word & 0x007f_ffff));
    if word & 0x8000_0000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// Construct distinct advanced authoritative RNG streams, including cached Gaussian state.
fn round_trip_rng_bundle(baseline_count: usize) -> RngStateBundle {
    let mut baselines = Vec::with_capacity(baseline_count);
    for slot in 0..baseline_count {
        baselines.push(BaselineRngState {
            slot: slot as u32,
            state: advanced_rng_state(&format!("baseline:{slot}"), slot + 1, slot % 2 == 0),
        });
    }
    RngStateBundle {
        version: RNG_BUNDLE_VERSION,
        world: advanced_rng_state("world", 31, true),
        evolution: advanced_rng_state("evolution", 47, true),
        external_controller: advanced_rng_state("external-controller", 3, false),
        baselines,
    }
}

/// Advance one labelled stream and optionally retain a generated Gaussian spare.
fn advanced_rng_state(
    label: &str,
    uniform_draws: usize,
    cache_gaussian: bool,
) -> SerializedRngState {
    let mut rng = labelled_stream(f64::from(0x5a17_c0de_u32), label);
    for _ in 0..uniform_draws {
        let _ = rng.next_f64();
    }
    if cache_gaussian {
        let _ = rng.gaussian();
    }
    rng.export_state()
}

/// Bounds sized for exact P2 packed data while retaining one-MiB codec scratch blocks.
fn round_trip_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 160 * 1024 * 1024,
        max_manifest_bytes: 64 * 1024,
        max_state_bytes: 256 * 1024,
        max_graph_bytes: 512 * 1024,
        max_population_index_bytes: 1024 * 1024,
        max_population_count: 512,
        max_setting_count: 64,
        max_baseline_rng_count: 64,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 2 * 1024 * 1024,
        max_weight_floats: 32 * 1024 * 1024,
        max_recurrent_floats: 1024 * 1024,
        max_numeric_stored_bytes: 128 * 1024 * 1024,
        max_numeric_candidate_bytes: 128 * 1024 * 1024,
        max_total_decoded_bytes: 160 * 1024 * 1024,
    }
}

/// Graph bounds covering both the default and Stage 2 large-GRU layouts.
fn round_trip_graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 16,
        max_edges: 32,
        max_graph_outputs: 4,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 1024,
        max_tensor_width: 1024,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 16,
        max_parameter_floats: 1_000_000,
        max_recurrent_state_floats: 16_384,
        max_canonical_layout_bytes: 128 * 1024,
        max_architecture_key_bytes: 256 * 1024,
    }
}

/// Hash the exact next RNG draws and allocator values available at a restored boundary.
fn continuation_probe_sha256(state: &AuthoritativeState) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(b"slither-stage3-boundary-continuation-probe\0v1\0");
    hasher.update(state.graph().layout_digest_sha256);
    hash_probe_u64(&mut hasher, state.state().generation.generation);
    hash_probe_u64(&mut hasher, state.state().generation.completed_step);
    hash_probe_u64(&mut hasher, state.state().generation.population_epoch);
    hash_next_rng(&mut hasher, &state.state().rng.world)?;
    hash_next_rng(&mut hasher, &state.state().rng.evolution)?;
    hash_next_rng(&mut hasher, &state.state().rng.external_controller)?;
    for baseline in &state.state().rng.baselines {
        hash_probe_u64(&mut hasher, u64::from(baseline.slot));
        hash_next_rng(&mut hasher, &baseline.state)?;
    }
    let allocators = &state.state().allocators;
    for value in [
        allocators.next_entity_id,
        allocators.next_brain_id,
        allocators.next_genome_id,
        allocators.next_controller_lease_id,
        u64::from(allocators.next_frame_v1_id),
        allocators.next_external_id,
        allocators.next_baseline_id,
        allocators.next_resurrected_id,
    ] {
        hash_probe_u64(&mut hasher, value);
    }
    Ok(hex_digest(hasher.finalize().as_slice()))
}

/// Hash a fixed deterministic sequence after strict RNG restoration.
fn hash_next_rng(hasher: &mut Sha256, serialized: &SerializedRngState) -> Result<(), String> {
    let mut rng = StatefulRng::from_state(serialized)
        .map_err(|error| format!("continuation RNG restore failed: {error}"))?;
    for _ in 0..4 {
        hash_probe_u64(hasher, rng.next_f64().to_bits());
    }
    hash_probe_u64(hasher, rng.gaussian().to_bits());
    hash_probe_u64(hasher, rng.gaussian().to_bits());
    hash_probe_u64(
        hasher,
        rng.int(1_000_003)
            .map_err(|error| format!("continuation integer draw failed: {error}"))?,
    );
    let advanced = rng.export_state();
    hash_probe_text(hasher, &advanced.state_hex)?;
    hash_probe_u64(hasher, u64::from(advanced.gaussian_spare_valid));
    if let Some(spare) = advanced.gaussian_spare_hex {
        hash_probe_text(hasher, &spare)?;
    }
    Ok(())
}

/// Hash one length-prefixed evidence string.
fn hash_probe_text(hasher: &mut Sha256, value: &str) -> Result<(), String> {
    let length =
        u64::try_from(value.len()).map_err(|_| "probe text length overflows".to_owned())?;
    hash_probe_u64(hasher, length);
    hasher.update(value.as_bytes());
    Ok(())
}

/// Hash one little-endian unsigned scalar.
fn hash_probe_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

/// Hash all packed population weights as explicit little-endian Float32 words.
fn population_weight_sha256(state: &AuthoritativeState) -> String {
    let mut hasher = Sha256::new();
    for genome in &state.state().population {
        for value in genome.weights.iter() {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hex_digest(hasher.finalize().as_slice())
}

/// Hash all packed recurrent state as explicit little-endian Float32 words.
fn recurrent_state_sha256(state: &AuthoritativeState) -> String {
    let mut hasher = Sha256::new();
    for brain in &state.state().brains {
        for value in brain.recurrent.iter() {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hex_digest(hasher.finalize().as_slice())
}

/// Compare every population weight by exact Float32 bits.
fn population_weight_bits_equal(left: &AuthoritativeState, right: &AuthoritativeState) -> bool {
    left.state().population.len() == right.state().population.len()
        && left
            .state()
            .population
            .iter()
            .zip(&right.state().population)
            .all(|(left_genome, right_genome)| {
                left_genome.weights.len() == right_genome.weights.len()
                    && left_genome
                        .weights
                        .iter()
                        .zip(right_genome.weights.iter())
                        .all(|(left_value, right_value)| {
                            left_value.to_bits() == right_value.to_bits()
                        })
            })
}

/// Compare every recurrent-state value by exact Float32 bits.
fn recurrent_state_bits_equal(left: &AuthoritativeState, right: &AuthoritativeState) -> bool {
    left.state().brains.len() == right.state().brains.len()
        && left
            .state()
            .brains
            .iter()
            .zip(&right.state().brains)
            .all(|(left_brain, right_brain)| {
                left_brain.recurrent.len() == right_brain.recurrent.len()
                    && left_brain
                        .recurrent
                        .iter()
                        .zip(right_brain.recurrent.iter())
                        .all(|(left_value, right_value)| {
                            left_value.to_bits() == right_value.to_bits()
                        })
            })
}

/// Convert a Float32 count to raw packed bytes with checked arithmetic.
fn checked_float_bytes(count: usize, label: &str) -> Result<u64, String> {
    let bytes = count
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or_else(|| format!("{label} byte count overflows"))?;
    u64::try_from(bytes).map_err(|_| format!("{label} byte count exceeds u64"))
}

/// Decode one exact lowercase-hex descriptor scalar.
fn parse_hex_u64(value: &str, label: &str) -> Result<u64, String> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not a canonical 16-digit hex scalar"));
    }
    u64::from_str_radix(value, 16).map_err(|error| format!("invalid {label}: {error}"))
}

/// Encode digest bytes as lowercase hexadecimal.
fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

/// Read Linux's process high-water RSS without introducing a platform dependency.
#[cfg(target_os = "linux")]
fn process_peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kibibytes = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kibibytes.checked_mul(1024)
}

/// Windows evidence records peak RSS with the external runner when required.
#[cfg(not(target_os = "linux"))]
const fn process_peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Process-local suffix preventing parallel fixture tests from sharing a directory.
    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    /// Disposable directory removed even when an assertion unwinds.
    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        /// Create one unique directory beneath the operating-system temporary root.
        fn create() -> Self {
            let suffix = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "slither-stage3-checkpoint-fixture-{}-{suffix}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("fixture directory is created");
            Self(path)
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn admitted_generation_boundary_reaches_real_checkpoint_publication() {
        let directory = FixtureDirectory::create();
        let descriptor =
            publish_stage3_fixture(&directory.0, "0123456789abcdef0123456789abcdef", 1)
                .expect("valid fixture publishes through the real checkpoint codec");

        assert_eq!(descriptor.transition_epoch_hex, "0000000000000001");
        assert_ne!(descriptor.recurrent_state_count_hex, "0000000000000000");
        assert!(directory.0.join(descriptor.relative_filename).is_file());
    }

    /// The retained runner uses the exact source-pinned P0/P2 graph shapes and stays honest.
    #[test]
    fn source_matched_round_trip_evidence_is_exact_and_does_not_claim_a_world_step() {
        let directory = FixtureDirectory::create();
        let report = run_stage3_roundtrip_evidence(&directory.0)
            .expect("small, P0, and P2 checkpoints must publish and restore exactly");

        assert_eq!(report.scenarios.len(), 3);
        assert!(!report.actual_world_step_exercised);
        let p0 = report
            .scenarios
            .iter()
            .find(|scenario| scenario.scenario == "P0")
            .expect("P0 result is retained");
        assert_eq!(p0.population_count, REPRESENTATIVE_POPULATION_COUNT);
        assert_eq!(p0.parameters_per_genome, P0_PARAMETER_COUNT);
        assert_eq!(p0.recurrent_state_per_brain, 16);
        let p2 = report
            .scenarios
            .iter()
            .find(|scenario| scenario.scenario == "P2")
            .expect("P2 result is retained");
        assert_eq!(p2.population_count, REPRESENTATIVE_POPULATION_COUNT);
        assert_eq!(p2.parameters_per_genome, P2_PARAMETER_COUNT);
        assert_eq!(p2.recurrent_state_per_brain, 96);
        assert!(report.scenarios.iter().all(|scenario| {
            scenario.state_equal
                && scenario.weight_bits_equal
                && scenario.recurrent_bits_equal
                && scenario.graph_equal
                && scenario.continuation_probe_equal
                && scenario.restored_republication_idempotent
        }));
    }
}
