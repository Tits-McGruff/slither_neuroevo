//! Bounded current-default generation-one population construction.
//!
//! This module implements the first explicit Stage 6A P0 profile. The caller
//! supplies only the lineage label, root seed, and hard state-memory ceiling.
//! Rust selects and compiles the fixed current-default graph, constructs the
//! complete normalized configuration, allocates every population/brain/genome
//! identity, initializes every differently weighted genome from the isolated
//! evolution stream, derives baseline streams, and returns a durability-gated
//! [`PendingRunStartTransition`]. No running authority is exposed here.

use super::checkpoint::CheckpointLimits;
use super::contract::ENGINE_CONTRACT_VERSION;
use super::generation::{derive_baseline_rngs, GenerationTransitionError};
use super::genome::{
    initialize_random_genome, GenomeInitializationConfig, GenomeInitializationError,
};
use super::graph::{typescript_default_graph_spec, GraphBundle, GraphError, GraphLimits};
use super::inference::InferenceMathBackend;
use super::rng::labelled_stream;
use super::run_start::{PendingRunStartTransition, RunStartTransitionError};
use super::state::{
    normalized_config_hash, normalized_settings_schema_hash,
    preflight_generation_boundary_allocation, AllocatorState, AuthorityPhase, BrainHandle,
    BrainOwner, BrainRuntimeState, ContractVersions, FixedStepContinuationState,
    GenerationBoundaryKind, GenerationState, GenomeLineage, NormalizedEngineConfig,
    PopulationGenome, RngStateBundle, RunIdentity, StateAdmissionPolicy, StateCandidate,
    StateError, WorldState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START, CHECKPOINT_VERSION,
    ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START, GENERATION_BOUNDARY_VERSION,
    NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION, RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION,
    SENSOR_VERSION, SERIALIZER_VERSION,
};
use super::step_config::{
    project_baseline_generation_config, project_running_step_config, typescript_default_settings,
    RunningStepWorkLimits, StepConfigError,
};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

/// Version of the fixed first-cut Stage 6A P0 fresh-run profile.
pub const STAGE6A_P0_FRESH_RUN_PROFILE_VERSION: u32 = 1;
/// Evolved population in the approved P0 workload.
pub const STAGE6A_P0_POPULATION_COUNT: usize = 55;
/// Durable baseline slots in the approved P0 workload.
pub const STAGE6A_P0_BASELINE_COUNT: usize = 10;
/// Exact packed Float32 parameters in the current default graph.
pub const STAGE6A_P0_PARAMETERS_PER_GENOME: usize = 13_458;
/// Exact recurrent Float32 values per current default brain.
pub const STAGE6A_P0_RECURRENT_PER_BRAIN: usize = 16;
/// One mebibyte in bytes.
const MIB: usize = 1024 * 1024;
/// One mebibyte in the checkpoint codec's exact byte-count domain.
const MIB_U64: u64 = 1024 * 1024;

/// Caller-owned identity and process memory policy for one fresh P0 lineage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage6aP0FreshRunRequest {
    /// Bounded lineage label. The normal server currently uses a UUID.
    pub run_id: String,
    /// Exact normalized Uint32 simulation seed.
    pub seed: u32,
    /// Hard peak state-memory ceiling after process-level admission budgeting.
    pub memory_ceiling_bytes: usize,
}

/// Build and admit one current-default run-start boundary behind durability.
pub fn prepare_stage6a_p0_fresh_run(
    request: Stage6aP0FreshRunRequest,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let prepared = prepare_stage6a_p0_boundary(request)?;
    PendingRunStartTransition::admit(
        prepared.candidate,
        prepared.graph,
        prepared.admission_policy,
        prepared.checkpoint_limits,
        prepared.graph_limits,
        prepared.work_limits,
    )
    .map_err(FreshRunError::from)
}

/// Complete owned inputs immediately before the run-start durability wrapper.
struct PreparedStage6aP0Boundary {
    candidate: StateCandidate,
    graph: Arc<GraphBundle>,
    admission_policy: StateAdmissionPolicy,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
    work_limits: RunningStepWorkLimits,
}

/// Construct and preflight the full boundary before it can become pending.
fn prepare_stage6a_p0_boundary(
    request: Stage6aP0FreshRunRequest,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    let run_id = validated_run_id(&request.run_id)?;
    let graph_limits = stage6a_p0_graph_limits();
    let graph = Arc::new(GraphBundle::compile(
        typescript_default_graph_spec(),
        &graph_limits,
    )?);
    if graph.total_parameters != STAGE6A_P0_PARAMETERS_PER_GENOME
        || graph.total_state_size != STAGE6A_P0_RECURRENT_PER_BRAIN
    {
        return Err(FreshRunError::ProfileInvariant {
            reason: "default graph dimensions changed without a fresh-run profile revision",
        });
    }

    let work_limits = RunningStepWorkLimits::provisional_defaults();
    let settings =
        typescript_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT);
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)?;
    let config = stage6a_p0_config(&graph, settings, settings_schema_sha256.clone());
    let projected = project_running_step_config(&config, work_limits)?;
    let baseline_config = project_baseline_generation_config(&config)?;
    let config_hash = normalized_config_hash(&config)?;
    let admission_policy =
        current_build_policy(request.memory_ceiling_bytes, settings_schema_sha256);
    let checkpoint_limits = stage6a_p0_checkpoint_limits();

    let build_identifier = crate::native_addon_build_identifier();
    let mut candidate = boundary_shell(
        run_id,
        request.seed,
        config,
        config_hash,
        build_identifier,
        &graph,
        baseline_config,
    )?;
    let weight_floats = STAGE6A_P0_POPULATION_COUNT
        .checked_mul(graph.total_parameters)
        .ok_or(FreshRunError::ArithmeticOverflow {
            context: "P0 population weight count",
        })?;
    let recurrent_floats = STAGE6A_P0_POPULATION_COUNT
        .checked_mul(graph.total_state_size)
        .ok_or(FreshRunError::ArithmeticOverflow {
            context: "P0 population recurrent count",
        })?;
    if candidate.population.len() > checkpoint_limits.max_population_count
        || weight_floats > checkpoint_limits.max_weight_floats
        || recurrent_floats > checkpoint_limits.max_recurrent_floats
    {
        return Err(FreshRunError::ProfileInvariant {
            reason: "fresh-run numeric shape exceeds its checkpoint limits",
        });
    }
    preflight_generation_boundary_allocation(
        &candidate,
        &graph,
        weight_floats,
        recurrent_floats,
        &admission_policy,
    )?;
    initialize_population_numeric(
        &mut candidate,
        graph.compiled(),
        projected.genome_initialization,
    )?;

    Ok(PreparedStage6aP0Boundary {
        candidate,
        graph,
        admission_policy,
        checkpoint_limits,
        graph_limits,
        work_limits,
    })
}

/// Validate the opaque lineage label before any graph or population allocation.
fn validated_run_id(value: &str) -> Result<String, FreshRunError> {
    if value.is_empty() || value.contains('\0') || value.len() > 256 {
        return Err(FreshRunError::InvalidRunId);
    }
    Ok(value.to_owned())
}

/// Construct the exact supported P0 normalized configuration.
fn stage6a_p0_config(
    graph: &GraphBundle,
    settings: Vec<super::state::NormalizedSetting>,
    settings_schema_sha256: String,
) -> NormalizedEngineConfig {
    NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256,
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 3_500.0,
        population_count: STAGE6A_P0_POPULATION_COUNT,
        baseline_count: STAGE6A_P0_BASELINE_COUNT,
        max_world_snakes: STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT + 64,
        max_non_population_brains: 64,
        max_body_points: 100_000,
        max_pellets: 25_000,
        spatial_index_bytes: 256 * MIB,
        worker_scratch_bytes: 512 * MIB,
        checkpoint_scratch_bytes: 512 * MIB,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    }
}

/// Bind admission to the currently loaded addon and exact settings schema.
fn current_build_policy(
    memory_ceiling_bytes: usize,
    settings_schema_sha256: String,
) -> StateAdmissionPolicy {
    let build_identifier = crate::native_addon_build_identifier();
    StateAdmissionPolicy {
        memory_ceiling_bytes,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: crate::native_addon_source_sha256(),
        expected_target_triple: crate::native_addon_build_target(),
        expected_build_profile: crate::native_addon_build_profile(),
        expected_build_class: crate::native_addon_build_class(),
        expected_rustc_version: crate::native_addon_rustc_version(),
        expected_build_contract_sha256: crate::native_addon_build_contract_sha256(),
        expected_math_backend: InferenceMathBackend::Scalar.label().to_owned(),
        expected_settings_schema_sha256: settings_schema_sha256,
    }
}

/// Build metadata-only population/brain records suitable for memory preflight.
#[allow(clippy::too_many_arguments)]
fn boundary_shell(
    run_id: String,
    seed: u32,
    config: NormalizedEngineConfig,
    config_hash: String,
    build_identifier: String,
    graph: &GraphBundle,
    baseline_config: super::step_config::BaselineGenerationConfig,
) -> Result<StateCandidate, FreshRunError> {
    let mut population = reserve_vec(STAGE6A_P0_POPULATION_COUNT, "P0 population records")?;
    let mut brains = reserve_vec(STAGE6A_P0_POPULATION_COUNT, "P0 brain records")?;
    for slot in 0..STAGE6A_P0_POPULATION_COUNT {
        let slot_u32 = u32::try_from(slot).map_err(|_| FreshRunError::ArithmeticOverflow {
            context: "P0 population slot",
        })?;
        let id = u64::try_from(slot)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(FreshRunError::ArithmeticOverflow {
                context: "P0 population identity",
            })?;
        let brain = BrainHandle { id, epoch: 1 };
        population.push(PopulationGenome {
            slot: slot_u32,
            brain,
            lineage: GenomeLineage {
                genome_id: id,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights: Box::new([]),
        });
        brains.push(BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(slot_u32),
            non_population_weights: None,
            recurrent: Box::new([]),
        });
    }
    let next_population_id = u64::try_from(STAGE6A_P0_POPULATION_COUNT)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(FreshRunError::ArithmeticOverflow {
            context: "next P0 population identity",
        })?;
    let baselines = derive_baseline_rngs(seed, 1, STAGE6A_P0_BASELINE_COUNT, baseline_config)?;

    Ok(StateCandidate {
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
            run_id,
            seed,
            config_revision: 1,
            config_hash,
            source_revision: build_identifier.clone(),
            engine_build_id: build_identifier,
            source_sha256: crate::native_addon_source_sha256(),
            target_triple: crate::native_addon_build_target(),
            build_profile: crate::native_addon_build_profile(),
            build_class: crate::native_addon_build_class(),
            rustc_version: crate::native_addon_rustc_version(),
            build_contract_sha256: crate::native_addon_build_contract_sha256(),
            math_backend: InferenceMathBackend::Scalar.label().to_owned(),
        },
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
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: labelled_stream(f64::from(seed), "world").export_state(),
            evolution: labelled_stream(f64::from(seed), "evolution").export_state(),
            external_controller: labelled_stream(f64::from(seed), "external-controller")
                .export_state(),
            baselines,
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1,
            next_brain_id: next_population_id,
            next_genome_id: next_population_id,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        },
        population,
        brains,
        world: WorldState::default(),
    })
}

/// Allocate exact numeric payloads only after the complete shell passes preflight.
fn initialize_population_numeric(
    candidate: &mut StateCandidate,
    graph: &super::graph::CompiledGraph,
    initialization: GenomeInitializationConfig,
) -> Result<(), FreshRunError> {
    let mut continuation = candidate.rng.evolution.clone();
    for (genome, brain) in candidate.population.iter_mut().zip(&mut candidate.brains) {
        let initialized = initialize_random_genome(graph, &continuation, initialization)?;
        let (weights, next_rng) = initialized.into_parts();
        genome.weights = weights.into_boxed_slice();
        brain.recurrent = zeroed_box(graph.total_state_size, "P0 recurrent state")?;
        continuation = next_rng;
    }
    candidate.rng.evolution = continuation;
    Ok(())
}

/// Tight graph ceilings for the one fixed P0 profile.
fn stage6a_p0_graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 4,
        max_edges: 3,
        max_graph_outputs: 1,
        max_identifier_bytes: 16,
        max_total_referenced_identifier_bytes: 1_024,
        max_tensor_width: 83,
        max_mlp_hidden_layers: 1,
        max_split_output_ports: 0,
        max_parameter_floats: STAGE6A_P0_PARAMETERS_PER_GENOME,
        max_recurrent_state_floats: STAGE6A_P0_RECURRENT_PER_BRAIN,
        max_canonical_layout_bytes: 16 * 1024,
        max_architecture_key_bytes: 32 * 1024,
    }
}

/// Managed-checkpoint ceilings for one current-default P0 boundary.
fn stage6a_p0_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 64 * MIB_U64,
        max_manifest_bytes: MIB,
        max_state_bytes: 4 * MIB,
        max_graph_bytes: MIB,
        max_population_index_bytes: MIB,
        max_population_count: STAGE6A_P0_POPULATION_COUNT,
        max_setting_count: 128,
        max_baseline_rng_count: STAGE6A_P0_BASELINE_COUNT,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 4 * MIB,
        max_weight_floats: STAGE6A_P0_POPULATION_COUNT * STAGE6A_P0_PARAMETERS_PER_GENOME,
        max_recurrent_floats: STAGE6A_P0_POPULATION_COUNT * STAGE6A_P0_RECURRENT_PER_BRAIN,
        max_numeric_stored_bytes: 16 * MIB_U64,
        max_numeric_candidate_bytes: 16 * MIB_U64,
        max_total_decoded_bytes: 32 * MIB_U64,
    }
}

/// Reserve one exact metadata or numeric vector without an unchecked growth path.
fn reserve_vec<T>(required: usize, context: &'static str) -> Result<Vec<T>, FreshRunError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(required)
        .map_err(|_| FreshRunError::AllocationFailed { context, required })?;
    Ok(values)
}

/// Allocate one zeroed fixed-size Float32 box after successful preflight.
fn zeroed_box(required: usize, context: &'static str) -> Result<Box<[f32]>, FreshRunError> {
    let mut values = reserve_vec(required, context)?;
    values.resize(required, 0.0);
    Ok(values.into_boxed_slice())
}

/// Failure before a fresh P0 run can publish any file or authority.
#[derive(Debug)]
pub enum FreshRunError {
    /// The lineage label is empty, contains NUL, or exceeds the checkpoint contract.
    InvalidRunId,
    /// Fixed profile dimensions and their versioned constants disagree.
    ProfileInvariant { reason: &'static str },
    /// Checked profile arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Exact metadata or numeric allocation failed.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// The fixed graph failed strict compilation.
    Graph(Box<GraphError>),
    /// The complete state shell or final candidate failed admission.
    State(Box<StateError>),
    /// Current normalized settings failed strict projection.
    StepConfig(Box<StepConfigError>),
    /// Baseline stream construction failed.
    Generation(Box<GenerationTransitionError>),
    /// One complete random genome failed before publication.
    Genome(Box<GenomeInitializationError>),
    /// The admitted boundary could not enter the durability wrapper.
    RunStart(Box<RunStartTransitionError>),
}

impl Display for FreshRunError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRunId => write!(
                formatter,
                "fresh-run ID must be nonempty, NUL-free, and at most 256 bytes"
            ),
            Self::ProfileInvariant { reason } => {
                write!(formatter, "Stage 6A P0 profile invariant failed: {reason}")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "fresh-run arithmetic overflow in {context}")
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "fresh-run allocation failed for {required} {context}"
            ),
            Self::Graph(error) => write!(formatter, "fresh-run graph failed: {error}"),
            Self::State(error) => write!(formatter, "fresh-run state failed: {error}"),
            Self::StepConfig(error) => {
                write!(formatter, "fresh-run settings failed: {error}")
            }
            Self::Generation(error) => {
                write!(formatter, "fresh-run baseline streams failed: {error}")
            }
            Self::Genome(error) => write!(formatter, "fresh-run genome failed: {error}"),
            Self::RunStart(error) => write!(formatter, "fresh-run barrier failed: {error}"),
        }
    }
}

impl Error for FreshRunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::State(error) => Some(error),
            Self::StepConfig(error) => Some(error),
            Self::Generation(error) => Some(error),
            Self::Genome(error) => Some(error),
            Self::RunStart(error) => Some(error),
            _ => None,
        }
    }
}

impl From<GraphError> for FreshRunError {
    fn from(error: GraphError) -> Self {
        Self::Graph(Box::new(error))
    }
}

impl From<StateError> for FreshRunError {
    fn from(error: StateError) -> Self {
        Self::State(Box::new(error))
    }
}

impl From<StepConfigError> for FreshRunError {
    fn from(error: StepConfigError) -> Self {
        Self::StepConfig(Box::new(error))
    }
}

impl From<GenerationTransitionError> for FreshRunError {
    fn from(error: GenerationTransitionError) -> Self {
        Self::Generation(Box::new(error))
    }
}

impl From<GenomeInitializationError> for FreshRunError {
    fn from(error: GenomeInitializationError) -> Self {
        Self::Genome(Box::new(error))
    }
}

impl From<RunStartTransitionError> for FreshRunError {
    fn from(error: RunStartTransitionError) -> Self {
        Self::RunStart(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::checkpoint::{CheckpointBoundaryKind, CheckpointOperationId};
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// Two GiB admits the conservative fixed P0 scratch envelope in local tests.
    const TEST_MEMORY_CEILING: usize = 2 * 1024 * MIB;
    /// Selected TypeScript seed retained by the compact compatibility fixture.
    const FIXTURE_SEED: u32 = 0x1234_5678;

    /// Automatically removes one process-unique managed-checkpoint test root.
    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create(label: &str) -> Self {
            let unique = format!(
                "slither-fresh-run-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("test clock must follow the Unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir(&path).expect("fresh-run test root must be unique and creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ignored = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureGraph {
        #[serde(rename = "type")]
        graph_type: String,
        nodes: Vec<FixtureGraphNode>,
        edges: Vec<FixtureGraphEdge>,
        outputs: Vec<FixtureGraphOutput>,
        output_size: usize,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type")]
    enum FixtureGraphNode {
        Input {
            id: String,
            #[serde(rename = "outputSize")]
            output_size: usize,
        },
        Dense {
            id: String,
            #[serde(rename = "inputSize")]
            input_size: usize,
            #[serde(rename = "outputSize")]
            output_size: usize,
        },
        #[serde(rename = "MLP")]
        Mlp {
            id: String,
            #[serde(rename = "inputSize")]
            input_size: usize,
            #[serde(rename = "hiddenSizes", default)]
            hidden_sizes: Vec<usize>,
            #[serde(rename = "outputSize")]
            output_size: usize,
        },
        #[serde(rename = "GRU")]
        Gru {
            id: String,
            #[serde(rename = "inputSize")]
            input_size: usize,
            #[serde(rename = "hiddenSize")]
            hidden_size: usize,
        },
        #[serde(rename = "LSTM")]
        Lstm {
            id: String,
            #[serde(rename = "inputSize")]
            input_size: usize,
            #[serde(rename = "hiddenSize")]
            hidden_size: usize,
        },
        #[serde(rename = "RRU")]
        Rru {
            id: String,
            #[serde(rename = "inputSize")]
            input_size: usize,
            #[serde(rename = "hiddenSize")]
            hidden_size: usize,
        },
        Concat {
            id: String,
        },
        Split {
            id: String,
            #[serde(rename = "outputSizes")]
            output_sizes: Vec<usize>,
        },
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureGraphEdge {
        from: String,
        to: String,
        from_port: Option<i64>,
        to_port: Option<i64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureGraphOutput {
        node_id: String,
        port: Option<i64>,
    }

    impl FixtureGraph {
        fn to_graph_spec(&self) -> super::super::graph::GraphSpec {
            use super::super::graph::{
                GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
            };

            let nodes = self
                .nodes
                .iter()
                .map(|node| match node {
                    FixtureGraphNode::Input { id, output_size } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Input {
                            output_size: *output_size,
                        },
                    },
                    FixtureGraphNode::Dense {
                        id,
                        input_size,
                        output_size,
                    } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Dense {
                            input_size: *input_size,
                            output_size: *output_size,
                        },
                    },
                    FixtureGraphNode::Mlp {
                        id,
                        input_size,
                        hidden_sizes,
                        output_size,
                    } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Mlp {
                            input_size: *input_size,
                            hidden_sizes: hidden_sizes.clone(),
                            output_size: *output_size,
                        },
                    },
                    FixtureGraphNode::Gru {
                        id,
                        input_size,
                        hidden_size,
                    } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Gru {
                            input_size: *input_size,
                            hidden_size: *hidden_size,
                        },
                    },
                    FixtureGraphNode::Lstm {
                        id,
                        input_size,
                        hidden_size,
                    } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Lstm {
                            input_size: *input_size,
                            hidden_size: *hidden_size,
                        },
                    },
                    FixtureGraphNode::Rru {
                        id,
                        input_size,
                        hidden_size,
                    } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Rru {
                            input_size: *input_size,
                            hidden_size: *hidden_size,
                        },
                    },
                    FixtureGraphNode::Concat { id } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Concat,
                    },
                    FixtureGraphNode::Split { id, output_sizes } => GraphNodeSpec {
                        id: id.clone(),
                        kind: GraphNodeKind::Split {
                            output_sizes: output_sizes.clone(),
                        },
                    },
                })
                .collect();
            GraphSpec {
                nodes,
                edges: self
                    .edges
                    .iter()
                    .map(|edge| GraphEdge {
                        from: edge.from.clone(),
                        to: edge.to.clone(),
                        from_port: edge.from_port,
                        to_port: edge.to_port,
                    })
                    .collect(),
                outputs: self
                    .outputs
                    .iter()
                    .map(|output| GraphOutputRef {
                        node_id: output.node_id.clone(),
                        port: output.port,
                    })
                    .collect(),
                output_size: self.output_size,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind")]
    enum FixtureNormalizedSetting {
        #[serde(rename = "bool")]
        Bool { value: bool },
        #[serde(rename = "integer")]
        Integer {
            #[serde(rename = "valueDecimal")]
            value_decimal: String,
        },
        #[serde(rename = "float")]
        Float {
            #[serde(rename = "valueHex")]
            value_hex: String,
        },
    }

    #[derive(Deserialize)]
    struct FixtureCompiledNodeRange {
        id: String,
        #[serde(rename = "type")]
        node_type: String,
        offset: usize,
        length: usize,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FreshRunFixture {
        seed: String,
        typescript_graph_key: String,
        graph: FixtureGraph,
        compiled_node_ranges: Vec<FixtureCompiledNodeRange>,
        total_parameters: usize,
        recurrent_state_floats: usize,
        normalized_settings: BTreeMap<String, FixtureNormalizedSetting>,
        population_count: usize,
        baseline_count: usize,
        genome_weight_sha256: Vec<String>,
        population_weight_sha256: String,
        world_state_hex: String,
        next_evolution_state_hex: String,
        external_controller_state_hex: String,
        baseline_state_hex: Vec<String>,
    }

    fn request(seed: u32) -> Stage6aP0FreshRunRequest {
        Stage6aP0FreshRunRequest {
            run_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(),
            seed,
            memory_ceiling_bytes: TEST_MEMORY_CEILING,
        }
    }

    fn fixture() -> FreshRunFixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("fresh-run-reference.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        serde_json::from_str(&text).expect("fresh-run TypeScript fixture must parse")
    }

    fn weight_sha256(weights: &[f32]) -> String {
        let mut hasher = Sha256::new();
        for weight in weights {
            hasher.update(weight.to_le_bytes());
        }
        format!("{:x}", hasher.finalize())
    }

    fn fixture_float64_bits(value: &str) -> u64 {
        let digits = value
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("fixture Float64 lacks 0x prefix: {value}"));
        assert_eq!(digits.len(), 16, "fixture Float64 must contain 16 digits");
        u64::from_str_radix(digits, 16)
            .unwrap_or_else(|error| panic!("invalid fixture Float64 {value}: {error}"))
    }

    fn typescript_graph_key(spec: &super::super::graph::GraphSpec) -> String {
        use super::super::graph::GraphNodeKind;

        let mut nodes = spec
            .nodes
            .iter()
            .map(|node| match &node.kind {
                GraphNodeKind::Input { output_size } => {
                    format!("{}:Input:{output_size}", node.id)
                }
                GraphNodeKind::Dense {
                    input_size,
                    output_size,
                } => format!("{}:Dense:{input_size}x{output_size}", node.id),
                GraphNodeKind::Mlp {
                    input_size,
                    hidden_sizes,
                    output_size,
                } => {
                    let hidden = if hidden_sizes.is_empty() {
                        "none".to_owned()
                    } else {
                        hidden_sizes
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join("x")
                    };
                    format!("{}:MLP:{input_size}x{hidden}x{output_size}", node.id)
                }
                GraphNodeKind::Gru {
                    input_size,
                    hidden_size,
                } => format!("{}:GRU:{input_size}x{hidden_size}", node.id),
                GraphNodeKind::Lstm {
                    input_size,
                    hidden_size,
                } => format!("{}:LSTM:{input_size}x{hidden_size}", node.id),
                GraphNodeKind::Rru {
                    input_size,
                    hidden_size,
                } => format!("{}:RRU:{input_size}x{hidden_size}", node.id),
                GraphNodeKind::Concat => format!("{}:Concat", node.id),
                GraphNodeKind::Split { output_sizes } => format!(
                    "{}:Split:{}",
                    node.id,
                    output_sizes
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            })
            .collect::<Vec<_>>();
        nodes.sort_unstable();
        let mut edges = spec
            .edges
            .iter()
            .map(|edge| {
                format!(
                    "{}->{}:{}:{}",
                    edge.from,
                    edge.to,
                    edge.from_port.unwrap_or(0),
                    edge.to_port.unwrap_or(0)
                )
            })
            .collect::<Vec<_>>();
        edges.sort_unstable();
        let outputs = spec
            .outputs
            .iter()
            .map(|output| format!("{}:{}", output.node_id, output.port.unwrap_or(0)))
            .collect::<Vec<_>>();
        format!(
            "graph|out:{}|nodes:{}|edges:{}|outs:{}",
            spec.output_size,
            nodes.join(";"),
            edges.join(";"),
            outputs.join(";")
        )
    }

    fn compiled_node_type_label(node_type: super::super::graph::CompiledNodeType) -> &'static str {
        use super::super::graph::CompiledNodeType;

        match node_type {
            CompiledNodeType::Input => "Input",
            CompiledNodeType::Dense => "Dense",
            CompiledNodeType::Mlp => "MLP",
            CompiledNodeType::Gru => "GRU",
            CompiledNodeType::Lstm => "LSTM",
            CompiledNodeType::Rru => "RRU",
            CompiledNodeType::Concat => "Concat",
            CompiledNodeType::Split => "Split",
        }
    }

    #[test]
    fn default_population_and_rngs_match_selected_typescript_source_exactly() {
        let fixture = fixture();
        assert_eq!(
            u32::from_str_radix(fixture.seed.trim_start_matches("0x"), 16).unwrap(),
            FIXTURE_SEED
        );
        let prepared = prepare_stage6a_p0_boundary(request(FIXTURE_SEED))
            .expect("selected P0 boundary must construct");
        let candidate = &prepared.candidate;
        assert_eq!(fixture.graph.graph_type, "graph");
        assert_eq!(prepared.graph.spec(), &fixture.graph.to_graph_spec());
        assert_eq!(
            typescript_graph_key(prepared.graph.spec()),
            fixture.typescript_graph_key
        );
        assert_eq!(
            prepared.graph.nodes.len(),
            fixture.compiled_node_ranges.len()
        );
        for (actual, expected) in prepared
            .graph
            .nodes
            .iter()
            .zip(&fixture.compiled_node_ranges)
        {
            assert_eq!(actual.id, expected.id);
            assert_eq!(
                compiled_node_type_label(actual.node_type),
                expected.node_type
            );
            assert_eq!(actual.parameter_offset, expected.offset, "{}", actual.id);
            assert_eq!(actual.parameter_length, expected.length, "{}", actual.id);
        }
        assert_eq!(prepared.graph.total_parameters, fixture.total_parameters);
        assert_eq!(
            prepared.graph.total_state_size,
            fixture.recurrent_state_floats
        );
        assert_eq!(candidate.population.len(), fixture.population_count);
        assert_eq!(candidate.rng.baselines.len(), fixture.baseline_count);
        assert_eq!(candidate.identity.seed, FIXTURE_SEED);
        assert_eq!(candidate.generation.generation, 1);
        assert_eq!(candidate.generation.completed_step, 0);
        assert_eq!(candidate.generation.population_epoch, 1);
        assert_eq!(
            candidate.config.population_count,
            STAGE6A_P0_POPULATION_COUNT
        );
        assert_eq!(candidate.config.baseline_count, STAGE6A_P0_BASELINE_COUNT);
        assert_eq!(
            candidate.config.settings.len(),
            fixture.normalized_settings.len(),
            "Rust and selected TypeScript must retain the same complete P0 setting set"
        );
        for setting in &candidate.config.settings {
            let expected = fixture
                .normalized_settings
                .get(&setting.path)
                .unwrap_or_else(|| panic!("selected TypeScript fixture lacks {}", setting.path));
            match (&setting.value, expected) {
                (
                    super::super::state::NormalizedSettingValue::Bool(actual),
                    FixtureNormalizedSetting::Bool { value },
                ) => assert_eq!(actual, value, "{}", setting.path),
                (
                    super::super::state::NormalizedSettingValue::Integer(actual),
                    FixtureNormalizedSetting::Integer { value_decimal },
                ) => assert_eq!(
                    actual.to_string(),
                    value_decimal.as_str(),
                    "{}",
                    setting.path
                ),
                (
                    super::super::state::NormalizedSettingValue::Float(actual),
                    FixtureNormalizedSetting::Float { value_hex },
                ) => assert_eq!(
                    actual.to_bits(),
                    fixture_float64_bits(value_hex),
                    "{}",
                    setting.path
                ),
                (actual, _) => panic!(
                    "selected TypeScript scalar kind does not match Rust for {}: {actual:?}",
                    setting.path
                ),
            }
        }

        let actual_genome_hashes = candidate
            .population
            .iter()
            .map(|genome| weight_sha256(&genome.weights))
            .collect::<Vec<_>>();
        assert_eq!(actual_genome_hashes, fixture.genome_weight_sha256);
        let mut population_hasher = Sha256::new();
        for genome in &candidate.population {
            for weight in &genome.weights {
                population_hasher.update(weight.to_le_bytes());
            }
        }
        assert_eq!(
            format!("{:x}", population_hasher.finalize()),
            fixture.population_weight_sha256
        );
        assert_eq!(candidate.rng.world.state_hex, fixture.world_state_hex);
        assert_eq!(
            candidate.rng.evolution.state_hex,
            fixture.next_evolution_state_hex
        );
        assert_eq!(
            candidate.rng.external_controller.state_hex,
            fixture.external_controller_state_hex
        );
        assert_eq!(
            candidate
                .rng
                .baselines
                .iter()
                .map(|baseline| baseline.state.state_hex.clone())
                .collect::<Vec<_>>(),
            fixture.baseline_state_hex
        );
    }

    #[test]
    fn rust_allocates_dense_population_identity_and_returns_only_a_pending_boundary() {
        let opaque_run_id = "  aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee  ";
        let prepared = prepare_stage6a_p0_boundary(Stage6aP0FreshRunRequest {
            run_id: opaque_run_id.to_owned(),
            ..request(7)
        })
        .expect("P0 boundary must construct");
        assert_eq!(prepared.candidate.identity.run_id, opaque_run_id);
        for (slot, (genome, brain)) in prepared
            .candidate
            .population
            .iter()
            .zip(&prepared.candidate.brains)
            .enumerate()
        {
            let expected_id = slot as u64 + 1;
            assert_eq!(genome.slot, slot as u32);
            assert_eq!(genome.brain.id, expected_id);
            assert_eq!(genome.brain.epoch, 1);
            assert_eq!(genome.lineage.genome_id, expected_id);
            assert_eq!(genome.lineage.birth_generation, 1);
            assert_eq!(genome.lineage.parent_a, None);
            assert_eq!(genome.lineage.parent_b, None);
            assert_eq!(genome.fitness.to_bits(), 0.0_f64.to_bits());
            assert_eq!(brain.handle, genome.brain);
            assert_eq!(brain.owner, BrainOwner::PopulationSlot(slot as u32));
            assert_eq!(brain.recurrent.len(), STAGE6A_P0_RECURRENT_PER_BRAIN);
            assert!(brain.recurrent.iter().all(|value| value.to_bits() == 0));
        }
        assert_eq!(prepared.candidate.allocators.next_brain_id, 56);
        assert_eq!(prepared.candidate.allocators.next_genome_id, 56);

        let pending = prepare_stage6a_p0_fresh_run(request(7))
            .expect("complete fresh run must enter the durability barrier");
        assert_eq!(pending.generation(), 1);
        assert_eq!(pending.completed_step(), 0);
        assert!(!pending.checkpoint_published());
        assert!(!pending.persistence_acknowledged());
        assert!(!pending.authority_published());
        assert_eq!(pending.snake_count(), 0);
        assert_eq!(pending.pellet_count(), 0);
    }

    #[test]
    fn default_population_checkpoint_retry_then_exact_ack_activates_once() {
        let managed = TestDirectory::create("managed-checkpoint");
        let operation = CheckpointOperationId::parse("0123456789abcdef0123456789abcdef")
            .expect("fixed operation ID must parse");
        let mut pending = prepare_stage6a_p0_fresh_run(request(FIXTURE_SEED))
            .expect("complete fresh run must enter the durability barrier");
        let mut frame = Vec::new();
        assert!(matches!(
            pending.pack_initial_frame_v1(&mut frame),
            Err(RunStartTransitionError::AuthorityNotPublished)
        ));
        assert!(frame.is_empty());
        assert!(matches!(
            pending.publish_running_authority(),
            Err(RunStartTransitionError::PersistenceNotAcknowledged)
        ));

        let descriptor = pending
            .publish_checkpoint(managed.path(), operation.clone())
            .expect("fixed P0 boundary must fit and publish under its checkpoint ceilings");
        assert_eq!(descriptor.operation_id, operation);
        assert_eq!(descriptor.run_id, request(FIXTURE_SEED).run_id);
        assert_eq!(descriptor.generation_hex, "0000000000000001");
        assert_eq!(descriptor.completed_step_hex, "0000000000000000");
        assert_eq!(descriptor.boundary_kind, CheckpointBoundaryKind::RunStart);
        assert_eq!(
            descriptor.population_count_hex,
            format!("{STAGE6A_P0_POPULATION_COUNT:016x}")
        );
        assert_eq!(
            descriptor.weight_count_hex,
            format!(
                "{:016x}",
                STAGE6A_P0_POPULATION_COUNT * STAGE6A_P0_PARAMETERS_PER_GENOME
            )
        );
        assert_eq!(
            descriptor.recurrent_state_count_hex,
            format!(
                "{:016x}",
                STAGE6A_P0_POPULATION_COUNT * STAGE6A_P0_RECURRENT_PER_BRAIN
            )
        );
        assert!(managed.path().join(&descriptor.relative_filename).is_file());

        let retry = pending
            .publish_checkpoint(managed.path(), operation)
            .expect("exact retry must reuse the retained immutable descriptor");
        assert_eq!(retry, descriptor);
        assert!(pending.checkpoint_published());
        assert!(!pending.persistence_acknowledged());
        assert!(!pending.authority_published());

        pending
            .acknowledge_persistence(&descriptor)
            .expect("exact committed descriptor must retain the durability barrier");
        let publication = pending
            .publish_running_authority()
            .expect("durable fixed P0 boundary must construct one running authority");
        assert_eq!(publication.generation, 1);
        assert_eq!(publication.completed_step, 0);
        assert_eq!(publication.population_epoch, 1);
        assert!(publication.memory.total_bytes <= TEST_MEMORY_CEILING);
        assert!(pending.persistence_acknowledged());
        assert!(pending.authority_published());
        assert_eq!(
            pending.snake_count(),
            STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT
        );
        assert_eq!(pending.pellet_count(), 3_500);
        let frame_metadata = pending
            .pack_initial_frame_v1(&mut frame)
            .expect("running authority must pack one neutral frame-v1 payload");
        assert_eq!(frame_metadata.generation, 1);
        assert_eq!(
            frame_metadata.total_snakes,
            STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT
        );
        assert_eq!(frame_metadata.alive_snakes, frame_metadata.total_snakes);
        assert_eq!(frame_metadata.pellets, 3_500);
        assert_eq!(frame_metadata.byte_length, frame.len());
        assert_eq!(frame_metadata.float_length * size_of::<f32>(), frame.len());
        assert!(matches!(
            pending.publish_running_authority(),
            Err(RunStartTransitionError::AuthorityAlreadyPublished)
        ));
    }

    #[test]
    fn invalid_identity_or_memory_fails_before_a_pending_transition_exists() {
        for run_id in ["", "bad\0id"] {
            let error = prepare_stage6a_p0_fresh_run(Stage6aP0FreshRunRequest {
                run_id: run_id.to_owned(),
                ..request(11)
            })
            .expect_err("invalid run ID must reject");
            assert!(matches!(error, FreshRunError::InvalidRunId));
        }
        let error = prepare_stage6a_p0_fresh_run(Stage6aP0FreshRunRequest {
            memory_ceiling_bytes: 1,
            ..request(11)
        })
        .expect_err("insufficient state ceiling must reject before numeric allocation");
        assert!(matches!(
            error,
            FreshRunError::State(ref state)
                if matches!(state.as_ref(), StateError::MemoryCeilingExceeded { .. })
        ));
    }
}
