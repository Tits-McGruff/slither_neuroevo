//! Bounded generation-one population construction.
//!
//! This module grew from the first explicit Stage 6A P0 profile. The caller
//! supplies the lineage label, root seed, complete settings and an optional
//! current graph. Rust independently compiles the graph, constructs the
//! complete normalized configuration, allocates every population/brain/genome
//! identity, initializes every differently weighted genome from the isolated
//! evolution stream, derives baseline streams, and returns a durability-gated
//! [`PendingRunStartTransition`]. No running authority is exposed here.

use super::checkpoint::{boundary_decoded_byte_count, CheckpointError, CheckpointLimits};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::generation::{derive_baseline_rngs, GenerationTransitionError};
use super::genome::{
    initialize_random_genome, GenomeInitializationConfig, GenomeInitializationError,
};
use super::graph::{
    typescript_default_graph_spec, CompiledNodeType, GraphBundle, GraphError, GraphLimits,
    GraphSpec,
};
use super::inference::InferenceMathBackend;
use super::live_settings::LiveSettingUpdate;
use super::rng::labelled_stream;
use super::run_start::{PendingRunStartTransition, RunStartTransitionError};
use super::state::{
    normalized_config_hash, normalized_settings_schema_hash,
    preflight_generation_boundary_allocation, AllocatorState, AuthorityPhase, BrainHandle,
    BrainOwner, BrainRuntimeState, ContractVersions, EvolvedRespawnRngState,
    FixedStepContinuationState, GenerationBoundaryKind, GenerationState, GenomeLineage,
    NormalizedEngineConfig, NormalizedSettingValue, PopulationGenome, RngStateBundle, RunIdentity,
    StateAdmissionPolicy, StateCandidate, StateError, WorldState, ALLOCATOR_VERSION,
    BASELINE_ENTITY_ID_START, CHECKPOINT_VERSION, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START,
    GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
    RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, RNG_BUNDLE_WITH_EVOLVED_VERSION,
    SERIALIZER_VERSION,
};
use super::step_config::{
    production_default_settings, project_baseline_generation_config, project_running_step_config,
    typescript_default_settings, RunningStepWorkLimits, StepConfigError,
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
/// Largest evolved population admitted by the existing settings contract.
const MAXIMUM_FRESH_RUN_POPULATION_COUNT: usize = 300;
/// Largest baseline population admitted by the existing settings contract.
const MAXIMUM_FRESH_RUN_BASELINE_COUNT: usize = 120;
/// Aggregate body storage charged by state admission before any fresh authority activates.
/// This exceeds the historical 200,000-segment grid failure while remaining a finite
/// resource ceiling; the per-snake `snakeMaxLen` setting is independently enforced.
const MAXIMUM_RUNNING_BODY_POINTS: usize = 1_000_000;
/// Aggregate pellet storage, including ambient food, boost trails and complete corpses.
/// The 25,000 ambient target plus a full maximum-snake batch of 420-pellet corpses
/// fits this allowance. Further accumulation still fails atomically at this bound.
const MAXIMUM_RUNNING_PELLETS: usize = 250_000;
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

/// One complete fresh-run setting value crossing the private Node/Rust boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct FreshRunSettingUpdate {
    /// Exact canonical settings path.
    pub path: String,
    /// Finite numeric representation; booleans are zero or one.
    pub value: f64,
}

/// Qualified defaults apply only to genuinely new configurations. Recorded
/// v3/v1 settings continue through their existing explicit constructors.
fn selected_fresh_default_settings() -> Result<Vec<super::state::NormalizedSetting>, FreshRunError>
{
    let mut settings =
        production_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT);
    super::sensor_contract::SensorContractIdentity::new(
        super::sensor_contract::SensorProfile::Motion,
        16,
    )
    .and_then(|contract| contract.record_v4_settings(&mut settings))
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    super::ray_work::RayWorkLimits::new(1, 256, 65536, 512)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?
        .record_settings(&mut settings);
    super::baseline_v2::BaselineV2Settings {
        mode: super::baseline_v2::BaselineV2Mode::Stochastic,
        control_interval_seconds: 1.0 / 60.0,
    }
    .record_settings(&mut settings)
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    settings
        .iter_mut()
        .find(|setting| setting.path == "sense.maxPelletChecks")
        .ok_or_else(|| FreshRunError::Settings("default pellet work setting is missing".into()))?
        .value = NormalizedSettingValue::Integer(3000);
    Ok(settings)
}

/// Only scalar settings cross the fresh-construction interface; Rust derives the digest.
fn numeric_fresh_settings(
    settings: Vec<super::state::NormalizedSetting>,
) -> Vec<FreshRunSettingUpdate> {
    settings
        .into_iter()
        .filter_map(|setting| {
            Some(FreshRunSettingUpdate {
                path: setting.path,
                value: match setting.value {
                    NormalizedSettingValue::Bool(value) => f64::from(value),
                    NormalizedSettingValue::Integer(value) => value as f64,
                    NormalizedSettingValue::Float(value) => value,
                    NormalizedSettingValue::Text(_) => return None,
                },
            })
        })
        .collect()
}

/// One production initial boundary, using the qualified graph/profile and SSE2 path.
pub fn prepare_default_fresh_run(
    request: Stage6aP0FreshRunRequest,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let settings = selected_fresh_default_settings()?;
    let contract = super::sensor_contract::SensorContractIdentity::from_settings(&settings)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    prepare_configured_fresh_run_with_math(
        request,
        &numeric_fresh_settings(settings),
        super::graph::default_stack_graph_spec(contract.input_size()),
        InferenceMathBackend::Sse2,
    )
}

/// One already-decoded legacy genome admitted for a population-only import.
#[derive(Debug)]
pub struct LegacyPopulationGenome {
    /// Packed parameters in the historical TypeScript graph layout.
    pub weights: Box<[f32]>,
}

/// Build and admit one current-default run-start boundary behind durability.
pub fn prepare_stage6a_p0_fresh_run(
    request: Stage6aP0FreshRunRequest,
) -> Result<PendingRunStartTransition, FreshRunError> {
    prepare_stage6a_p0_fresh_run_with_live_settings(request, &[])
}

/// Build a fresh P0 lineage with the current live subset preserved across Reset/New Run.
pub fn prepare_stage6a_p0_fresh_run_with_live_settings(
    request: Stage6aP0FreshRunRequest,
    live_settings: &[LiveSettingUpdate],
) -> Result<PendingRunStartTransition, FreshRunError> {
    let settings = live_settings
        .iter()
        .map(|update| FreshRunSettingUpdate {
            path: update.path.clone(),
            value: update.value,
        })
        .collect::<Vec<_>>();
    prepare_stage6a_p0_fresh_run_with_settings(request, &settings)
}

/// Build a fresh fixed-graph lineage from one complete validated settings projection.
pub fn prepare_stage6a_p0_fresh_run_with_settings(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
) -> Result<PendingRunStartTransition, FreshRunError> {
    prepare_stage6a_p0_fresh_run_with_settings_and_graph(
        request,
        settings,
        typescript_default_graph_spec(),
    )
}

/// Build a fresh lineage from one complete setting set and caller-supplied graph.
pub fn prepare_stage6a_p0_fresh_run_with_settings_and_graph(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let prepared = prepare_stage6a_p0_boundary(request, settings, graph)?;
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

/// Select a complete modern life contract, or preserve explicitly recorded legacy settings.
/// Empty inputs denote a genuinely new default configuration, not a recorded reset.
pub fn prepare_configured_fresh_run_with_settings_and_graph(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
) -> Result<PendingRunStartTransition, FreshRunError> {
    prepare_configured_fresh_run_with_math(request, settings, graph, InferenceMathBackend::Scalar)
}

/// Reset/New Run preserves the arithmetic backend captured with its held source.
pub(crate) fn prepare_configured_fresh_run_with_math(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    math: InferenceMathBackend,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let defaults;
    let settings = if settings.is_empty() {
        defaults = numeric_fresh_settings(selected_fresh_default_settings()?);
        defaults.as_slice()
    } else {
        settings
    };
    let modern = settings.is_empty()
        || settings
            .iter()
            .any(|setting| setting.path == "evolvedRespawn.contractVersion");
    let mut prepared = if settings
        .iter()
        .any(|setting| setting.path == "sense.layoutVersion" && setting.value == 4.0)
    {
        prepare_configured_v4_boundary(request, settings, graph, modern)?
    } else {
        prepare_fresh_boundary(request, settings, graph, modern)?
    };
    prepared.candidate.identity.math_backend = math.label().into();
    prepared.admission_policy.expected_math_backend = math.label().into();
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

/// Explicit v4 Reset/New Run selection. No qualification-only constructor or
/// production default is involved; every structural choice and ray limit is
/// supplied by the caller and admitted before constructing a population.
fn prepare_configured_v4_boundary(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    modern: bool,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    if settings.len() > 128 {
        return Err(FreshRunError::Settings(
            "fresh-run settings exceed 128 entries".into(),
        ));
    }
    let required = |path: &str| -> Result<f64, FreshRunError> {
        let mut matches = settings.iter().filter(|setting| setting.path == path);
        let value = matches
            .next()
            .ok_or_else(|| FreshRunError::Settings(format!("v4 settings require {path}")))?
            .value;
        if matches.next().is_some() || !value.is_finite() {
            return Err(FreshRunError::Settings(format!(
                "duplicate or nonfinite v4 setting {path}"
            )));
        }
        Ok(value)
    };
    let integer = |path: &str| -> Result<u32, FreshRunError> {
        let value = required(path)?;
        if value.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&value) {
            return Err(FreshRunError::Settings(format!(
                "v4 setting {path} must be a Uint32"
            )));
        }
        Ok(value as u32)
    };
    if integer("sense.layoutVersion")? != 4
        || integer("brain.sensorVersion")? != 4
        || integer("sense.contractVersion")? != 1
    {
        return Err(FreshRunError::Settings(
            "v4 sensor version/contract markers disagree".into(),
        ));
    }
    let profile = match integer("sense.profile")? {
        0 => super::sensor_contract::SensorProfile::Basic,
        1 => super::sensor_contract::SensorProfile::Motion,
        _ => {
            return Err(FreshRunError::Settings(
                "v4 profile must be Basic or Motion".into(),
            ))
        }
    };
    let contract = super::sensor_contract::SensorContractIdentity::new(
        profile,
        integer("sense.bubbleBins")? as usize,
    )
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let rays = super::ray_work::RayWorkLimits::new(
        integer("sense.rayMinimum")?,
        integer("sense.rayCap")?,
        integer("sense.rayBatch")?,
        integer("sense.rayMaximumDue")? as usize,
    )
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let baseline = if settings.iter().any(|setting| {
        matches!(
            setting.path.as_str(),
            "baselineBots.controlVersion" | "baselineBots.mode" | "baselineBots.controlDt"
        )
    }) {
        if integer("baselineBots.controlVersion")? != 2 {
            return Err(FreshRunError::Settings(
                "explicit baseline control version must be 2".into(),
            ));
        }
        let mode = match integer("baselineBots.mode")? {
            0 => super::baseline_v2::BaselineV2Mode::Deterministic,
            1 => super::baseline_v2::BaselineV2Mode::Stochastic,
            _ => {
                return Err(FreshRunError::Settings(
                    "baseline v2 mode must be Deterministic or Stochastic".into(),
                ))
            }
        };
        let baseline = super::baseline_v2::BaselineV2Settings {
            mode,
            control_interval_seconds: required("baselineBots.controlDt")?,
        };
        baseline
            .validate()
            .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        Some(baseline)
    } else {
        None
    };
    let replacements = settings
        .iter()
        .filter(|setting| {
            !matches!(
                setting.path.as_str(),
                "sense.contractVersion"
                    | "sense.profile"
                    | "sense.rayMinimum"
                    | "sense.rayCap"
                    | "sense.rayBatch"
                    | "sense.rayMaximumDue"
                    | "baselineBots.controlVersion"
                    | "baselineBots.mode"
                    | "baselineBots.controlDt"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    prepare_fresh_boundary_with_sensor_and_baseline(
        request,
        &replacements,
        graph,
        modern,
        Some(contract),
        RunningStepWorkLimits {
            ray_work: Some(rays),
            ..RunningStepWorkLimits::provisional_defaults()
        },
        baseline,
    )
}

/// Build a new generation-one lineage around an old browser-exported population.
///
/// Legacy JSON does not contain the exact Rust continuation state required for a
/// resume. Its genomes therefore replace only the randomly initialized numeric
/// population of an otherwise normal fresh boundary; chronology, recurrent
/// state, RNG streams, allocators, and world state all begin from that boundary.
pub fn prepare_stage6a_legacy_population_import(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    genomes: Vec<LegacyPopulationGenome>,
    origin: super::legacy_origin::LegacyPopulationOrigin,
) -> Result<PendingRunStartTransition, FreshRunError> {
    if genomes.is_empty() || genomes.len() > MAXIMUM_FRESH_RUN_POPULATION_COUNT {
        return Err(FreshRunError::Settings(
            "legacy import requires 1 to 300 genomes".to_owned(),
        ));
    }
    let mut replacement_settings = settings.to_vec();
    let population_count = genomes.len() as f64;
    if let Some(setting) = replacement_settings
        .iter_mut()
        .find(|setting| setting.path == "snakeCount")
    {
        setting.value = population_count;
    } else {
        replacement_settings.push(FreshRunSettingUpdate {
            path: "snakeCount".to_owned(),
            value: population_count,
        });
    }
    let mut prepared = prepare_stage6a_p0_boundary(request, &replacement_settings, graph)?;
    prepared.candidate.identity.legacy_conversion = Some(origin);
    let expected_weights = prepared.graph.total_parameters;
    if genomes
        .iter()
        .any(|genome| genome.weights.len() != expected_weights)
    {
        return Err(FreshRunError::Settings(format!(
            "legacy genome weight count must equal the graph's {expected_weights} parameters"
        )));
    }
    for (target, imported) in prepared.candidate.population.iter_mut().zip(genomes) {
        target.weights = imported.weights;
        target.fitness = 0.0;
    }
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

/// Restore one worker-selected P0 checkpoint using the loaded build's admission
/// policy. No fresh population, seed, or allocator state is constructed.
pub fn prepare_stage6a_p0_checkpoint_restore(
    managed_directory: &std::path::Path,
    descriptor: &super::checkpoint::CheckpointDescriptor,
    memory_ceiling_bytes: usize,
) -> Result<PendingRunStartTransition, FreshRunError> {
    PendingRunStartTransition::restore_committed(
        managed_directory,
        descriptor,
        production_restore_policy(memory_ceiling_bytes, true)?,
        production_checkpoint_limits(),
        stage6a_p0_graph_limits(),
        RunningStepWorkLimits::provisional_defaults(),
    )
    .map_err(FreshRunError::from)
}

/// Restore a candidate v4 boundary with explicit environment admission and unchanged exact build checks.
/// This test-only entry does not select a production inference or ray-work default.
#[cfg(feature = "engine-test-hooks")]
pub(crate) fn prepare_sensor_qualification_checkpoint_restore(
    managed_directory: &std::path::Path,
    descriptor: &super::checkpoint::CheckpointDescriptor,
    memory_ceiling_bytes: usize,
    profile: super::sensor_contract::SensorProfile,
    bins: usize,
    math: InferenceMathBackend,
    limits: RunningStepWorkLimits,
) -> Result<PendingRunStartTransition, FreshRunError> {
    prepare_sensor_qualification_checkpoint_restore_with_limits(
        managed_directory,
        descriptor,
        memory_ceiling_bytes,
        profile,
        bins,
        math,
        limits,
        production_checkpoint_limits(),
    )
}

/// Explicit test-hook envelope accompanies the same exact-source restore and contract checks.
#[cfg(feature = "engine-test-hooks")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_sensor_qualification_checkpoint_restore_with_limits(
    managed_directory: &std::path::Path,
    descriptor: &super::checkpoint::CheckpointDescriptor,
    memory_ceiling_bytes: usize,
    profile: super::sensor_contract::SensorProfile,
    bins: usize,
    math: InferenceMathBackend,
    limits: RunningStepWorkLimits,
    checkpoint_limits: CheckpointLimits,
) -> Result<PendingRunStartTransition, FreshRunError> {
    if profile == super::sensor_contract::SensorProfile::V3 || limits.ray_work.is_none() {
        return Err(FreshRunError::Settings(
            "qualified restore requires v4 and explicit ray limits".into(),
        ));
    }
    let contract = super::sensor_contract::SensorContractIdentity::new(profile, bins)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let mut policy = production_restore_policy(memory_ceiling_bytes, true)?;
    policy.expected_math_backend = math.label().into();
    policy.allowed_math_backends.clear();
    let pending = PendingRunStartTransition::restore_committed(
        managed_directory,
        descriptor,
        policy,
        checkpoint_limits,
        stage6a_p0_graph_limits(),
        limits,
    )?;
    pending
        .validate_qualification_sensor_contract(&contract)
        .map_err(FreshRunError::Settings)?;
    Ok(pending)
}

/// Restore one version/target/math-compatible checkpoint from another exact
/// build before immediately rebinding it to a durable recovery branch.
pub fn prepare_stage6a_p0_compatible_checkpoint_restore(
    managed_directory: &std::path::Path,
    descriptor: &super::checkpoint::CheckpointDescriptor,
    memory_ceiling_bytes: usize,
) -> Result<PendingRunStartTransition, FreshRunError> {
    PendingRunStartTransition::restore_committed(
        managed_directory,
        descriptor,
        production_restore_policy(memory_ceiling_bytes, false)?,
        production_checkpoint_limits(),
        stage6a_p0_graph_limits(),
        RunningStepWorkLimits::provisional_defaults(),
    )
    .map_err(FreshRunError::from)
}

/// Wrap one fully validated import candidate in the normal durability-gated
/// run-start owner without reading its checkpoint file again.
pub(crate) fn prepare_stage6a_p0_validated_import(
    restored: super::checkpoint::RestoredCheckpoint,
    descriptor: super::checkpoint::CheckpointDescriptor,
    memory_ceiling_bytes: usize,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
) -> Result<PendingRunStartTransition, FreshRunError> {
    PendingRunStartTransition::restore_validated_import(
        restored,
        descriptor,
        production_restore_policy(memory_ceiling_bytes, true)?,
        checkpoint_limits,
        graph_limits,
        RunningStepWorkLimits::provisional_defaults(),
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
    replacement_settings: &[FreshRunSettingUpdate],
    graph_spec: GraphSpec,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    prepare_fresh_boundary(request, replacement_settings, graph_spec, false)
}

/// Construct one explicit schema without changing the frozen fixture constructor.
fn prepare_fresh_boundary(
    request: Stage6aP0FreshRunRequest,
    replacement_settings: &[FreshRunSettingUpdate],
    graph_spec: GraphSpec,
    modern: bool,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    prepare_fresh_boundary_with_sensor(
        request,
        replacement_settings,
        graph_spec,
        modern,
        None,
        RunningStepWorkLimits::provisional_defaults(),
    )
}

/// Explicit candidate construction for native workload/continuation qualification.
/// Public fresh defaults stay recorded v3 until the v4 gates select ray limits.
#[cfg(any(test, feature = "engine-test-hooks"))]
#[allow(clippy::too_many_arguments)]
pub fn prepare_v4_qualification_run(
    request: Stage6aP0FreshRunRequest,
    replacement_settings: &[FreshRunSettingUpdate],
    graph_spec: GraphSpec,
    modern: bool,
    profile: super::sensor_contract::SensorProfile,
    bins: usize,
    limits: RunningStepWorkLimits,
) -> Result<PendingRunStartTransition, FreshRunError> {
    if profile == super::sensor_contract::SensorProfile::V3 || limits.ray_work.is_none() {
        return Err(FreshRunError::Settings(
            "v4 qualification requires a profile and explicit ray limits".into(),
        ));
    }
    let contract = super::sensor_contract::SensorContractIdentity::new(profile, bins)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let prepared = prepare_fresh_boundary_with_sensor(
        request,
        replacement_settings,
        graph_spec,
        modern,
        Some(contract),
        limits,
    )?;
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

/// Construct a measured v3/v4 comparison with one shared modern experiment and explicit arithmetic.
#[cfg(feature = "engine-test-hooks")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_sensor_qualification_run(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    profile: super::sensor_contract::SensorProfile,
    bins: usize,
    limits: RunningStepWorkLimits,
    math: InferenceMathBackend,
    baseline: Option<super::baseline_v2::BaselineV2Settings>,
    trained: &[super::brain_artifact::ValidatedBrainArtifact],
) -> Result<PendingRunStartTransition, FreshRunError> {
    let identity = super::sensor_contract::SensorContractIdentity::new(profile, bins)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    if (profile != super::sensor_contract::SensorProfile::V3) != limits.ray_work.is_some() {
        return Err(FreshRunError::Settings(
            "v4 requires explicit ray limits; v3 does not use them".into(),
        ));
    }
    if let Some(baseline) = baseline {
        baseline
            .validate()
            .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        if profile == super::sensor_contract::SensorProfile::V3 {
            return Err(FreshRunError::Settings("baseline v2 requires v4".into()));
        }
    }
    let mut prepared = prepare_fresh_boundary_with_sensor_and_baseline(
        request,
        settings,
        graph,
        true,
        (profile != super::sensor_contract::SensorProfile::V3).then_some(identity),
        limits,
        baseline,
    )?;
    if !trained.is_empty() {
        install_qualified_population_artifacts(&mut prepared, trained)?;
    }
    prepared.candidate.identity.math_backend = math.label().into();
    prepared.admission_policy.expected_math_backend = math.label().into();
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

/// Cold qualification-only population values; ordinary fresh-run construction stays random.
#[cfg(feature = "engine-test-hooks")]
fn validate_qualified_population_artifact(
    prepared: &PreparedStage6aP0Boundary,
    artifact: &super::brain_artifact::ValidatedBrainArtifact,
) -> Result<(), FreshRunError> {
    let contract = super::sensor_contract::SensorContractIdentity::from_settings(
        &prepared.candidate.config.settings,
    )
    .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let life_rule = match artifact.deployment.life_state_rule {
        super::training_graph::TrainingLifeStateRule::Reset => 0,
        super::training_graph::TrainingLifeStateRule::Preserve => 1,
    };
    if artifact.canonical_manifest.layout_sha256 != prepared.graph.layout_digest_hex()
        || artifact.canonical_manifest.sensor_sha256
            != contract
                .sha256_hex()
                .map_err(|error| FreshRunError::Settings(error.to_string()))?
        || artifact.deployment.control_interval_seconds
            != fresh_float(&prepared.candidate.config, "brain.controlDt")?
        || life_rule != fresh_integer(&prepared.candidate.config, "evolvedRespawn.recurrentState")?
        || prepared
            .candidate
            .population
            .iter()
            .any(|genome| genome.weights.len() != artifact.weights.len())
    {
        return Err(FreshRunError::Settings(
            "trained qualification artifact graph/sensors/deployment mismatch".into(),
        ));
    }
    Ok(())
}

/// Validate every source before copying; stable population slots select families, never snake IDs.
#[cfg(feature = "engine-test-hooks")]
fn install_qualified_population_artifacts(
    prepared: &mut PreparedStage6aP0Boundary,
    artifacts: &[super::brain_artifact::ValidatedBrainArtifact],
) -> Result<(), FreshRunError> {
    if artifacts.is_empty() || artifacts.len() > 16 {
        return Err(FreshRunError::Settings(
            "qualification requires 1..16 policy families".into(),
        ));
    }
    for artifact in artifacts {
        validate_qualified_population_artifact(prepared, artifact)?;
    }
    if prepared
        .candidate
        .population
        .iter()
        .enumerate()
        .any(|(index, genome)| genome.slot as usize != index)
    {
        return Err(FreshRunError::Settings(
            "qualification population slots are not dense".into(),
        ));
    }
    // All parameter boxes and recurrent owners were admitted by ordinary fresh construction.
    // Preserve their pointers, lineages and initialization RNG continuation.
    for genome in &mut prepared.candidate.population {
        genome
            .weights
            .copy_from_slice(&artifacts[genome.slot as usize % artifacts.len()].weights);
    }
    Ok(())
}

#[cfg(all(test, feature = "engine-test-hooks"))]
fn install_qualified_population_artifact(
    prepared: &mut PreparedStage6aP0Boundary,
    artifact: &super::brain_artifact::ValidatedBrainArtifact,
) -> Result<(), FreshRunError> {
    install_qualified_population_artifacts(prepared, std::slice::from_ref(artifact))
}

fn prepare_fresh_boundary_with_sensor(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    modern: bool,
    contract: Option<super::sensor_contract::SensorContractIdentity>,
    limits: RunningStepWorkLimits,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    prepare_fresh_boundary_with_sensor_and_baseline(
        request, settings, graph, modern, contract, limits, None,
    )
}

/// Explicit native v2 candidate construction; ordinary defaults remain qualified separately.
#[cfg(any(test, feature = "engine-test-hooks"))]
#[allow(clippy::too_many_arguments)]
pub fn prepare_baseline_v2_qualification_run(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    profile: super::sensor_contract::SensorProfile,
    bins: usize,
    baseline: super::baseline_v2::BaselineV2Settings,
    limits: RunningStepWorkLimits,
) -> Result<PendingRunStartTransition, FreshRunError> {
    if profile == super::sensor_contract::SensorProfile::V3 || limits.ray_work.is_none() {
        return Err(FreshRunError::Settings(
            "baseline-v2 requires v4 and explicit ray limits".into(),
        ));
    }
    let contract = super::sensor_contract::SensorContractIdentity::new(profile, bins)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let prepared = prepare_fresh_boundary_with_sensor_and_baseline(
        request,
        settings,
        graph,
        true,
        Some(contract),
        limits,
        Some(baseline),
    )?;
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

fn prepare_fresh_boundary_with_sensor_and_baseline(
    request: Stage6aP0FreshRunRequest,
    replacement_settings: &[FreshRunSettingUpdate],
    graph_spec: GraphSpec,
    modern: bool,
    sensor_contract: Option<super::sensor_contract::SensorContractIdentity>,
    work_limits: RunningStepWorkLimits,
    baseline_v2: Option<super::baseline_v2::BaselineV2Settings>,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    initialize_fresh_boundary(prepare_fresh_boundary_shell(
        request,
        replacement_settings,
        graph_spec,
        modern,
        sensor_contract,
        work_limits,
        baseline_v2,
        production_checkpoint_limits(),
    )?)
}

/// Metadata-only inputs; numeric boxes are initialized only after complete shape admission.
struct FreshBoundaryShell {
    candidate: StateCandidate,
    graph: Arc<GraphBundle>,
    admission_policy: StateAdmissionPolicy,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
    work_limits: RunningStepWorkLimits,
    initialization: GenomeInitializationConfig,
}

#[allow(clippy::too_many_arguments)]
fn prepare_fresh_boundary_shell(
    request: Stage6aP0FreshRunRequest,
    replacement_settings: &[FreshRunSettingUpdate],
    graph_spec: GraphSpec,
    modern: bool,
    sensor_contract: Option<super::sensor_contract::SensorContractIdentity>,
    work_limits: RunningStepWorkLimits,
    baseline_v2: Option<super::baseline_v2::BaselineV2Settings>,
    checkpoint_limits: CheckpointLimits,
) -> Result<FreshBoundaryShell, FreshRunError> {
    let run_id = validated_run_id(&request.run_id)?;
    let graph_limits = stage6a_p0_graph_limits();
    let graph = Arc::new(GraphBundle::compile(graph_spec, &graph_limits)?);

    let legacy =
        typescript_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT);
    let mut settings = if modern {
        production_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT)
    } else {
        legacy.clone()
    };
    if modern && !replacement_settings.is_empty() {
        for setting in settings
            .iter()
            .filter(|setting| !legacy.iter().any(|old| old.path == setting.path))
        {
            if !replacement_settings
                .iter()
                .any(|update| update.path == setting.path)
            {
                return Err(FreshRunError::Settings(format!(
                    "modern settings require {}",
                    setting.path
                )));
            }
        }
    }
    apply_fresh_run_settings(&mut settings, replacement_settings)?;
    if let Some(contract) = &sensor_contract {
        contract
            .record_v4_settings(&mut settings)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        if let Some(ray_work) = work_limits.ray_work {
            ray_work.record_settings(&mut settings);
        }
    }
    if let Some(spec) = baseline_v2 {
        spec.record_settings(&mut settings)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    }
    let population_count = fresh_setting_integer(&settings, "snakeCount")?;
    let baseline_count = fresh_setting_integer(&settings, "baselineBots.count")?;
    if population_count == 0 || population_count > MAXIMUM_FRESH_RUN_POPULATION_COUNT {
        return Err(FreshRunError::Settings(
            "fresh-run snakeCount must be an integer from 1 to 300".to_owned(),
        ));
    }
    if baseline_count > MAXIMUM_FRESH_RUN_BASELINE_COUNT {
        return Err(FreshRunError::Settings(
            "fresh-run baselineBots.count must be an integer from 0 to 120".to_owned(),
        ));
    }
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)?;
    let mut config = stage6a_p0_config(
        &graph,
        settings,
        settings_schema_sha256.clone(),
        population_count,
        baseline_count,
    );
    config.requested_sim_speed = fresh_float(&config, "simSpeed")?;
    config.world_radius = fresh_integer(&config, "worldRadius")? as f64;
    if work_limits
        .ray_work
        .is_some_and(|rays| rays.maximum_due() < config.max_world_snakes)
    {
        return Err(FreshRunError::Settings(
            "ray maximum-due admission must cover every configured snake".into(),
        ));
    }
    let projected = project_running_step_config(&config, work_limits)?;
    let graph_input_size = graph
        .nodes
        .iter()
        .find(|node| node.node_type == CompiledNodeType::Input)
        .map(|node| node.output_size)
        .ok_or(FreshRunError::ProfileInvariant {
            reason: "compiled graph omits its input node",
        })?;
    let sensor_input_size =
        super::sensor_contract::SensorContractIdentity::from_settings(&config.settings)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?
            .input_size();
    if graph_input_size != sensor_input_size {
        return Err(FreshRunError::GraphInputMismatch {
            graph: graph_input_size,
            sensor: sensor_input_size,
        });
    }
    let baseline_config = project_baseline_generation_config(&config)?;
    let config_hash = normalized_config_hash(&config)?;
    let admission_policy =
        current_build_policy(request.memory_ceiling_bytes, settings_schema_sha256, true);

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
    if modern {
        candidate.versions.rng_bundle = RNG_BUNDLE_WITH_EVOLVED_VERSION;
        candidate.rng.version = RNG_BUNDLE_WITH_EVOLVED_VERSION;
        candidate.rng.evolved_respawns =
            EvolvedRespawnRngState::derive_generation(request.seed, 1, population_count)?;
    }
    if baseline_v2.is_some() {
        candidate.rng.version = super::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
        candidate.versions.rng_bundle = candidate.rng.version;
        candidate.rng.evolved_respawns =
            EvolvedRespawnRngState::derive_generation(request.seed, 1, population_count)?;
        candidate.rng.baseline_behaviors = super::rng::derive_baseline_v2_behaviors(
            request.seed,
            baseline_config.seed,
            1,
            baseline_count,
            baseline_config.randomize_seed_per_generation,
        )
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        for stream in &mut candidate.rng.baselines {
            stream.state = super::rng::baseline_v2_lifecycle_rng(
                request.seed,
                baseline_config.seed,
                1,
                stream.slot,
                baseline_config.randomize_seed_per_generation,
            )
            .export_state();
        }
    }
    Ok(FreshBoundaryShell {
        candidate,
        graph,
        admission_policy,
        checkpoint_limits,
        graph_limits,
        work_limits,
        initialization: projected.genome_initialization,
    })
}

/// Enforce the existing production preflight before allocating a single numeric box.
fn initialize_fresh_boundary(
    shell: FreshBoundaryShell,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    let FreshBoundaryShell {
        mut candidate,
        graph,
        admission_policy,
        checkpoint_limits,
        graph_limits,
        work_limits,
        initialization,
    } = shell;
    let population_count = candidate.config.population_count;
    let weight_floats = population_count.checked_mul(graph.total_parameters).ok_or(
        FreshRunError::ArithmeticOverflow {
            context: "fresh-run population weight count",
        },
    )?;
    let recurrent_floats = population_count.checked_mul(graph.total_state_size).ok_or(
        FreshRunError::ArithmeticOverflow {
            context: "fresh-run population recurrent count",
        },
    )?;
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
    let decoded_bytes = boundary_decoded_byte_count(
        &candidate,
        graph.spec(),
        graph.compiled(),
        &checkpoint_limits,
    )?;
    if decoded_bytes > checkpoint_limits.max_total_decoded_bytes {
        return Err(FreshRunError::CheckpointTooLarge {
            decoded_bytes,
            limit_bytes: checkpoint_limits.max_total_decoded_bytes,
            population_count,
            parameters_per_genome: graph.total_parameters,
        });
    }
    initialize_population_numeric(&mut candidate, graph.compiled(), initialization)?;

    Ok(PreparedStage6aP0Boundary {
        candidate,
        graph,
        admission_policy,
        checkpoint_limits,
        graph_limits,
        work_limits,
    })
}

/// Audit or allocate an explicit offline checkpoint candidate through the ordinary constructor.
#[cfg(feature = "engine-test-hooks")]
pub(crate) fn prepare_checkpoint_capacity_run(
    request: Stage6aP0FreshRunRequest,
    options: &super::sensor_qualification::SensorQualificationOptions,
    policy: super::checkpoint_capacity::CheckpointCapacityPolicy,
    allocate: bool,
) -> Result<super::checkpoint_capacity::QualifiedCheckpointRun, FreshRunError> {
    use super::checkpoint_capacity::{CheckpointCapacityAudit, QualifiedCheckpointRun};
    if options.trained_artifact.is_some()
        || options.ray_limits.is_none()
        || options.profile == super::sensor_contract::SensorProfile::V3
    {
        return Err(FreshRunError::Settings(
            "capacity qualification requires fresh v4 and explicit rays".into(),
        ));
    }
    let (mut updates, spec) = super::sensor_qualification::sensor_workload_spec(options, None)?;
    // Capacity/live fixtures use the declared teacher cadence for their neural
    // controls too, including explicitly requested nondefault teacher intervals.
    if let Some(baseline) = options.baseline {
        updates
            .iter_mut()
            .find(|entry| entry.path == "brain.controlDt")
            .ok_or_else(|| FreshRunError::Settings("capacity neural cadence is missing".into()))?
            .value = baseline.control_interval_seconds;
    }
    let contract =
        super::sensor_contract::SensorContractIdentity::new(options.profile, options.bins)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    let work_limits = RunningStepWorkLimits {
        calculation_workers: options.workers,
        ray_work: options.ray_limits,
        ..RunningStepWorkLimits::provisional_defaults()
    };
    let limits = policy.limits();
    let mut shell = prepare_fresh_boundary_shell(
        request,
        &updates,
        spec,
        true,
        Some(contract),
        work_limits,
        options.baseline,
        limits.clone(),
    )?;
    shell.candidate.identity.math_backend = options.math.label().into();
    shell.admission_policy.expected_math_backend = options.math.label().into();
    let count = shell.candidate.population.len();
    let parameters = shell.graph.total_parameters;
    let recurrent = shell.graph.total_state_size;
    let checked = |value: Option<usize>| {
        value.ok_or(FreshRunError::ArithmeticOverflow {
            context: "checkpoint capacity audit",
        })
    };
    let weight_floats = checked(count.checked_mul(parameters))?;
    let recurrent_floats = checked(count.checked_mul(recurrent))?;
    let state_memory = preflight_generation_boundary_allocation(
        &shell.candidate,
        &shell.graph,
        weight_floats,
        recurrent_floats,
        &shell.admission_policy,
    )?;
    let roles = super::checkpoint::boundary_decoded_role_byte_counts(
        &shell.candidate,
        shell.graph.spec(),
        shell.graph.compiled(),
        &limits,
    )?;
    let decoded_bytes = roles
        .iter()
        .try_fold(0u64, |sum, value| sum.checked_add(*value))
        .ok_or(FreshRunError::ArithmeticOverflow {
            context: "checkpoint decoded audit",
        })?;
    let workspace = super::checkpoint::checkpoint_workspace_bound(&limits, count)?;
    let publication_peak = checked(
        state_memory
            .total_bytes
            .checked_sub(shell.candidate.config.checkpoint_scratch_bytes)
            .and_then(|value| value.checked_add(workspace)),
    )?;
    let pair = checked(state_memory.total_bytes.checked_mul(2))?;
    let minimum_numeric_peak = super::state::generation_numeric_peak_bytes(
        state_memory.total_bytes,
        count,
        shell.graph.compiled(),
    )?;
    let numeric_payload_allocated = shell
        .candidate
        .population
        .iter()
        .any(|genome| !genome.weights.is_empty())
        || shell
            .candidate
            .brains
            .iter()
            .any(|brain| !brain.recurrent.is_empty() || brain.non_population_weights.is_some());
    let audit = CheckpointCapacityAudit {
        population: count,
        parameters_per_genome: parameters,
        recurrent_per_brain: recurrent,
        role_names: ["state", "graph", "population-index", "weights", "recurrent"],
        decoded_role_bytes: roles,
        decoded_bytes,
        limits: limits.clone(),
        state_memory,
        checkpoint_workspace_bytes: workspace,
        checkpoint_publication_peak_bytes: publication_peak,
        conservative_two_state_bytes: pair,
        minimum_successor_numeric_peak_bytes: minimum_numeric_peak,
        process_memory_ceiling_bytes: shell.admission_policy.memory_ceiling_bytes,
        decoded_envelope_fits: decoded_bytes <= limits.max_total_decoded_bytes,
        two_state_budget_fits: pair <= shell.admission_policy.memory_ceiling_bytes,
        numeric_payload_allocated,
        worker_scratch_bytes: shell.candidate.config.worker_scratch_bytes,
        checkpoint_scratch_bytes: shell.candidate.config.checkpoint_scratch_bytes,
    };
    let admission_policy = shell.admission_policy.clone();
    let graph_limits = shell.graph_limits.clone();
    let pending = if allocate && audit.decoded_envelope_fits {
        let prepared = initialize_fresh_boundary(shell)?;
        Some(PendingRunStartTransition::admit(
            prepared.candidate,
            prepared.graph,
            prepared.admission_policy,
            prepared.checkpoint_limits,
            prepared.graph_limits,
            prepared.work_limits,
        )?)
    } else {
        None
    };
    Ok(QualifiedCheckpointRun {
        audit,
        pending,
        limits,
        graph_limits,
        admission_policy,
        work_limits,
    })
}

/// Apply exact typed numeric replacements before projection and state admission.
fn apply_fresh_run_settings(
    settings: &mut [super::state::NormalizedSetting],
    updates: &[FreshRunSettingUpdate],
) -> Result<(), FreshRunError> {
    if updates.len() > 128 {
        return Err(FreshRunError::Settings(
            "fresh-run settings exceed 128 entries".to_owned(),
        ));
    }
    for (index, update) in updates.iter().enumerate() {
        if !update.value.is_finite() || update.path.is_empty() || update.path.len() > 128 {
            return Err(FreshRunError::Settings(
                "fresh-run setting is not bounded and finite".to_owned(),
            ));
        }
        if updates[..index]
            .iter()
            .any(|prior| prior.path == update.path)
        {
            return Err(FreshRunError::Settings(format!(
                "duplicate fresh-run setting {}",
                update.path
            )));
        }
        let setting = settings
            .binary_search_by(|setting| setting.path.as_str().cmp(update.path.as_str()))
            .ok()
            .and_then(|position| settings.get_mut(position))
            .ok_or_else(|| {
                FreshRunError::Settings(format!("unknown fresh-run setting {}", update.path))
            })?;
        let replacement = match &setting.value {
            NormalizedSettingValue::Bool(_) if update.value == 0.0 || update.value == 1.0 => {
                NormalizedSettingValue::Bool(update.value == 1.0)
            }
            NormalizedSettingValue::Integer(_)
                if update.value.fract() == 0.0
                    && update.value >= i64::MIN as f64
                    && update.value <= i64::MAX as f64 =>
            {
                NormalizedSettingValue::Integer(update.value as i64)
            }
            NormalizedSettingValue::Float(_) => NormalizedSettingValue::Float(update.value),
            _ => {
                return Err(FreshRunError::Settings(format!(
                    "fresh-run setting {} has the wrong numeric type",
                    update.path
                )))
            }
        };
        setting.value = replacement;
    }
    Ok(())
}

/// Read one required Float64 setting after typed replacement.
fn fresh_float(config: &NormalizedEngineConfig, path: &str) -> Result<f64, FreshRunError> {
    match config
        .settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value)
    {
        Some(NormalizedSettingValue::Float(value)) => Ok(*value),
        _ => Err(FreshRunError::Settings(format!(
            "missing floating fresh-run setting {path}"
        ))),
    }
}

/// Read one required non-negative integer setting after typed replacement.
fn fresh_integer(config: &NormalizedEngineConfig, path: &str) -> Result<usize, FreshRunError> {
    fresh_setting_integer(&config.settings, path)
}

/// Read one required non-negative integer directly from a normalized setting set.
fn fresh_setting_integer(
    settings: &[super::state::NormalizedSetting],
    path: &str,
) -> Result<usize, FreshRunError> {
    match settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value)
    {
        Some(NormalizedSettingValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            FreshRunError::Settings(format!("invalid integer fresh-run setting {path}"))
        }),
        _ => Err(FreshRunError::Settings(format!(
            "missing integer fresh-run setting {path}"
        ))),
    }
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
    population_count: usize,
    baseline_count: usize,
) -> NormalizedEngineConfig {
    NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256,
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 3_500.0,
        population_count,
        baseline_count,
        max_world_snakes: population_count + baseline_count + 64,
        max_non_population_brains: 64,
        max_body_points: MAXIMUM_RUNNING_BODY_POINTS,
        max_pellets: MAXIMUM_RUNNING_PELLETS,
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
    require_exact_build_identity: bool,
) -> StateAdmissionPolicy {
    let build_identifier = crate::native_addon_build_identifier();
    StateAdmissionPolicy {
        memory_ceiling_bytes,
        require_exact_build_identity,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: crate::native_addon_source_sha256(),
        expected_target_triple: crate::native_addon_build_target(),
        expected_build_profile: crate::native_addon_build_profile(),
        expected_build_class: crate::native_addon_build_class(),
        expected_rustc_version: crate::native_addon_rustc_version(),
        expected_build_contract_sha256: crate::native_addon_build_contract_sha256(),
        expected_math_backend: InferenceMathBackend::Scalar.label().to_owned(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
        expected_settings_schema_sha256: settings_schema_sha256,
    }
}

/// Production restore recognizes only complete frozen or complete life-contract schemas.
fn production_restore_policy(
    memory_ceiling_bytes: usize,
    require_exact_build_identity: bool,
) -> Result<StateAdmissionPolicy, FreshRunError> {
    let legacy = normalized_settings_schema_hash(&typescript_default_settings(
        STAGE6A_P0_POPULATION_COUNT,
        STAGE6A_P0_BASELINE_COUNT,
    ))?;
    let modern = normalized_settings_schema_hash(&production_default_settings(
        STAGE6A_P0_POPULATION_COUNT,
        STAGE6A_P0_BASELINE_COUNT,
    ))?;
    let mut policy =
        current_build_policy(memory_ceiling_bytes, legacy, require_exact_build_identity);
    policy
        .allowed_math_backends
        .push(InferenceMathBackend::Sse2.label().into());
    policy.allowed_settings_schema_sha256.push(modern);
    for mut settings in [
        typescript_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT),
        production_default_settings(STAGE6A_P0_POPULATION_COUNT, STAGE6A_P0_BASELINE_COUNT),
    ] {
        super::sensor_contract::SensorContractIdentity::new(
            super::sensor_contract::SensorProfile::Motion,
            16,
        )
        .and_then(|contract| contract.record_v4_settings(&mut settings))
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&settings)?);
        let mut recorded_rays = settings.clone();
        // Only field types enter this hash; these values select no defaults.
        super::ray_work::RayWorkLimits::new(1, 1, 512, 512)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?
            .record_settings(&mut recorded_rays);
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&recorded_rays)?);
        super::baseline_v2::BaselineV2Settings {
            mode: super::baseline_v2::BaselineV2Mode::Stochastic,
            control_interval_seconds: 0.06,
        }
        .record_settings(&mut settings)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&settings)?);
        super::ray_work::RayWorkLimits::new(1, 1, 512, 512)
            .map_err(|error| FreshRunError::Settings(error.to_string()))?
            .record_settings(&mut settings);
        policy
            .allowed_settings_schema_sha256
            .push(normalized_settings_schema_hash(&settings)?);
    }
    Ok(policy)
}

/// Rebuild the fixed P0 archive validation contract without a second population.
pub(crate) fn stage6a_p0_archive_validation_contract(
    memory_ceiling_bytes: usize,
    require_exact_build_identity: bool,
) -> Result<(CheckpointLimits, GraphLimits, StateAdmissionPolicy), FreshRunError> {
    Ok((
        production_checkpoint_limits(),
        stage6a_p0_graph_limits(),
        production_restore_policy(memory_ceiling_bytes, require_exact_build_identity)?,
    ))
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
    let population_count = config.population_count;
    let baseline_count = config.baseline_count;
    let mut population = reserve_vec(population_count, "fresh-run population records")?;
    let mut brains = reserve_vec(population_count, "fresh-run brain records")?;
    for slot in 0..population_count {
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
            retained_origin: None,
            non_population_weights: None,
            recurrent: Box::new([]),
        });
    }
    let next_population_id = u64::try_from(population_count)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(FreshRunError::ArithmeticOverflow {
            context: "next P0 population identity",
        })?;
    let baselines = derive_baseline_rngs(seed, 1, baseline_count, baseline_config)?;

    Ok(StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: fresh_integer(&config, "brain.sensorVersion")? as u32,
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
            legacy_conversion: None,
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
            baseline_behaviors: Vec::new(),
            evolved_respawns: Vec::new(),
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

/// Production graph ceilings covering current editor graphs without unbounded allocation.
pub(crate) fn stage6a_p0_graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 64,
        max_edges: 128,
        max_graph_outputs: 8,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 16 * 1024,
        max_tensor_width: 512,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 16,
        max_parameter_floats: 1_000_000,
        max_recurrent_state_floats: 16_384,
        max_canonical_layout_bytes: 128 * 1024,
        max_architecture_key_bytes: 256 * 1024,
    }
}

/// Managed-checkpoint ceilings for the current population and admitted custom graphs.
/// The 512 MiB envelope admits P3's roughly 461 MiB raw weights and bounds decoded
/// allocation and codec work. Raising it requires remeasuring memory and aligning
/// Node's candidate/final-file disk reserves; the retention budget is independent.
pub(crate) fn production_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 512 * MIB_U64,
        max_manifest_bytes: MIB,
        max_state_bytes: 4 * MIB,
        max_graph_bytes: MIB,
        max_population_index_bytes: MIB,
        max_population_count: MAXIMUM_FRESH_RUN_POPULATION_COUNT,
        max_setting_count: 128,
        max_baseline_rng_count: MAXIMUM_FRESH_RUN_BASELINE_COUNT,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 4 * MIB,
        max_weight_floats: MAXIMUM_FRESH_RUN_POPULATION_COUNT * 1_000_000,
        max_recurrent_floats: MAXIMUM_FRESH_RUN_POPULATION_COUNT * 16_384,
        max_numeric_stored_bytes: 512 * MIB_U64,
        max_numeric_candidate_bytes: 512 * MIB_U64,
        max_total_decoded_bytes: 512 * MIB_U64,
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
    /// The graph input width does not match the selected sensor layout.
    GraphInputMismatch { graph: usize, sensor: usize },
    /// A supplied complete setting projection was malformed or incompatible.
    Settings(String),
    /// Checked profile arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Exact metadata or numeric allocation failed.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// Exact decoded checkpoint size exceeds the existing cap before numeric initialization.
    CheckpointTooLarge {
        decoded_bytes: u64,
        limit_bytes: u64,
        population_count: usize,
        parameters_per_genome: usize,
    },
    /// Bounded metadata encoding failed during checkpoint-size admission.
    Checkpoint(Box<CheckpointError>),
    /// The supplied or default graph failed strict compilation.
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
            Self::GraphInputMismatch { graph, sensor } => write!(
                formatter,
                "fresh-run graph input width {graph} does not match sensor width {sensor}"
            ),
            Self::Settings(detail) => write!(formatter, "fresh-run settings failed: {detail}"),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "fresh-run arithmetic overflow in {context}")
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "fresh-run allocation failed for {required} {context}"
            ),
            Self::CheckpointTooLarge { decoded_bytes, limit_bytes, population_count, parameters_per_genome } => write!(
                formatter,
                "proposed population checkpoint needs {decoded_bytes} decoded bytes ({:.2} MiB) for {population_count} NPC snakes with {parameters_per_genome} parameters each; limit is {limit_bytes} bytes ({:.2} MiB). Reduce NPC snakes or brain size.",
                *decoded_bytes as f64 / MIB_U64 as f64,
                *limit_bytes as f64 / MIB_U64 as f64
            ),
            Self::Checkpoint(error) => write!(formatter, "fresh-run checkpoint sizing failed: {error}"),
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
            Self::Checkpoint(error) => Some(error),
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

impl From<CheckpointError> for FreshRunError {
    fn from(error: CheckpointError) -> Self {
        Self::Checkpoint(Box::new(error))
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
    use crate::engine::contract::{
        CommandBatch, CompletedEvent, EngineCommand, EngineInit, InboundLimits, OutputLimits,
        ReliableEvent, SequencedCommand,
    };
    use crate::engine::frame_v1::FrameV1ViewDescriptor;
    use crate::engine::queues::NoopWakeSink;
    use crate::engine::running_loop::{
        RunningAuthorityDeliveryState, RunningAuthorityLoopError, RunningAuthorityLoopProgress,
        RunningAuthorityLoopState, RunningFramePublication,
    };
    use crate::engine::runtime::EngineRuntime;
    use crate::engine::scheduler::{FixedStepSchedulerPolicy, SchedulerServiceMode};
    use crate::engine::state::{AuthoritativeState, NormalizedSettingValue};
    use crate::engine::{
        ExternalDeliveryResult, GenerationReassignmentProgress, LifecycleState, RunningStepError,
    };
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::mem::size_of;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// Two GiB admits the conservative fixed P0 scratch envelope in local tests.
    const TEST_MEMORY_CEILING: usize = 2 * 1024 * MIB;
    /// Selected TypeScript seed retained by the compact compatibility fixture.
    const FIXTURE_SEED: u32 = 0x1234_5678;

    #[test]
    fn production_checkpoint_limits_admit_approved_p3_raw_weight_envelope() {
        // P3 has 300 evolved genomes and the approved large GRU has 402,914 weights.
        // Raw fallback must remain admissible even when shuffle/Zstandard saves nothing.
        const P3_RAW_WEIGHT_BYTES: u64 = 300 * 402_914 * 4;
        let limits = production_checkpoint_limits();
        assert!(limits.max_numeric_stored_bytes >= P3_RAW_WEIGHT_BYTES);
        assert!(limits.max_numeric_candidate_bytes >= P3_RAW_WEIGHT_BYTES);
        assert!(limits.max_total_decoded_bytes >= P3_RAW_WEIGHT_BYTES);
        assert!(limits.max_archive_bytes >= P3_RAW_WEIGHT_BYTES);
    }

    /// Decoding alongside an old population uses a reduced budget. After that
    /// owner is retired, the imported loop must afford ordinary dual-state evolution.
    #[test]
    fn imported_loop_reclaims_retired_population_budget_for_generation_handoffs() {
        for branch in [false, true] {
            let managed = TestDirectory::create("import-retired-budget");
            let full_ceiling = 4 * 1024 * MIB;
            let mut input = request(42);
            input.memory_ceiling_bytes = full_ceiling;
            let prepared = prepare_stage6a_p0_boundary(
                input,
                &[FreshRunSettingUpdate {
                    path: "snakeCount".into(),
                    value: 3.0,
                }],
                typescript_default_graph_spec(),
            )
            .unwrap();
            let mut staging_policy = prepared.admission_policy.clone();
            let probe = AuthoritativeState::validate_and_own(
                prepared.candidate,
                prepared.graph,
                &staging_policy,
            )
            .unwrap();
            staging_policy.memory_ceiling_bytes = probe.memory_estimate().total_bytes + 8 * MIB;
            let operation =
                CheckpointOperationId::parse("00000000000000000000000000000081").unwrap();
            let descriptor = super::super::checkpoint::publish_checkpoint(
                managed.path(),
                operation,
                1,
                probe.checkpoint_boundary().unwrap(),
                &prepared.checkpoint_limits,
                &prepared.graph_limits,
                &staging_policy,
            )
            .unwrap();
            let restored = super::super::checkpoint::restore_checkpoint(
                &managed.path().join(&descriptor.relative_filename),
                &prepared.checkpoint_limits,
                &prepared.graph_limits,
                &staging_policy,
            )
            .unwrap();
            drop(probe); // Old population is now retired; its reservation can be reclaimed.
            let mut pending = prepare_stage6a_p0_validated_import(
                restored,
                descriptor.clone(),
                full_ceiling,
                prepared.checkpoint_limits,
                prepared.graph_limits,
            )
            .unwrap();
            pending.acknowledge_import_persistence(&descriptor).unwrap();
            let run_id = if branch {
                let name = "standalone-import-recovery".to_owned();
                pending = pending
                    .into_committed_recovery_branch(name.clone())
                    .unwrap();
                name
            } else {
                descriptor.run_id.clone()
            };
            assert_eq!(pending.admitted_memory_ceiling_bytes(), staging_policy.memory_ceiling_bytes,
            "identity rebinding must retain overlap admission until running ownership transfers");
            pending.publish_running_authority().unwrap();
            let mut running = pending
                .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
                .unwrap();
            running
                .request_finish_generation("finish-import".into(), run_id, 1)
                .unwrap();
            running
                .service_after_command_drain(17, SchedulerServiceMode::Background, None)
                .expect("retired source budget must support the imported generation transition");
            let transition = running.pending_generation_transition().unwrap();
            assert_eq!(transition.memory_ceiling_bytes(), full_ceiling);
            assert!(transition.combined_state_bytes() > staging_policy.memory_ceiling_bytes);
            assert!(transition.combined_state_bytes() <= full_ceiling);
        }
    }

    /// The real replacement path keeps staging bounded through identity rebinding
    /// and coordinator construction, then reclaims budget after dropping the old loop.
    #[test]
    fn live_import_replacement_reclaims_budget_with_and_without_recovery_branch() {
        for branch in [false, true] {
            let managed = TestDirectory::create("live-import-retirement");
            let full_ceiling = 4 * 1024 * MIB;
            let mut input = request(42);
            input.memory_ceiling_bytes = full_ceiling;
            let prepared = prepare_stage6a_p0_boundary(
                input.clone(),
                &[FreshRunSettingUpdate {
                    path: "snakeCount".into(),
                    value: 3.0,
                }],
                typescript_default_graph_spec(),
            )
            .unwrap();
            let mut policy = prepared.admission_policy.clone();
            let probe =
                AuthoritativeState::validate_and_own(prepared.candidate, prepared.graph, &policy)
                    .unwrap();
            policy.memory_ceiling_bytes = probe.memory_estimate().total_bytes + 8 * MIB;
            let descriptor = super::super::checkpoint::publish_checkpoint(
                managed.path(),
                CheckpointOperationId::parse("00000000000000000000000000000082").unwrap(),
                1,
                probe.checkpoint_boundary().unwrap(),
                &prepared.checkpoint_limits,
                &prepared.graph_limits,
                &policy,
            )
            .unwrap();
            drop(probe);
            let restored = super::super::checkpoint::restore_checkpoint(
                &managed.path().join(&descriptor.relative_filename),
                &prepared.checkpoint_limits,
                &prepared.graph_limits,
                &policy,
            )
            .unwrap();
            let pending = prepare_stage6a_p0_validated_import(
                restored,
                descriptor.clone(),
                full_ceiling,
                prepared.checkpoint_limits,
                prepared.graph_limits,
            )
            .unwrap();
            assert_eq!(
                pending.admitted_memory_ceiling_bytes(),
                policy.memory_ceiling_bytes
            );
            let slot = super::super::contract::PreparedImportSlot::new();
            slot.put(pending).unwrap();
            input.run_id = "old-live-import-owner".into();
            input.seed = 43;
            let mut old = prepare_stage6a_p0_fresh_run_with_settings_and_graph(
                input,
                &[FreshRunSettingUpdate {
                    path: "snakeCount".into(),
                    value: 3.0,
                }],
                typescript_default_graph_spec(),
            )
            .unwrap();
            let old_descriptor = old
                .publish_checkpoint(
                    managed.path(),
                    CheckpointOperationId::parse("00000000000000000000000000000083").unwrap(),
                )
                .unwrap();
            old.acknowledge_persistence(&old_descriptor).unwrap();
            old.publish_running_authority().unwrap();
            let mut running = old
                .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
                .unwrap();
            let branch_id = branch.then_some("live-import-recovery-branch");
            running.stage_prepared_import(&slot).unwrap();
            running
                .publish_prepared_import(&slot, &descriptor, branch_id, 0)
                .unwrap();
            assert!(!slot.is_some());
            running
                .request_finish_generation(
                    "finish-live-import".into(),
                    branch_id.unwrap_or(&descriptor.run_id).to_owned(),
                    1,
                )
                .unwrap();
            running
                .service_after_command_drain(17, SchedulerServiceMode::Background, None)
                .unwrap();
            let transition = running.pending_generation_transition().unwrap();
            assert_eq!(transition.memory_ceiling_bytes(), full_ceiling);
            assert!(transition.combined_state_bytes() > policy.memory_ceiling_bytes);
            assert!(transition.combined_state_bytes() <= full_ceiling);
        }
    }

    /// No candidate is required to hold the source, but only that hold's lease
    /// can release it; active worker ownership cannot be cancelled out from under it.
    #[test]
    fn population_work_holds_complete_steps_before_allocation_and_resumes_without_debt() {
        let managed = TestDirectory::create("population-work-hold");
        let mut input = request(42);
        input.memory_ceiling_bytes = 4 * 1024 * MIB;
        let mut pending = prepare_stage6a_p0_fresh_run(input).unwrap();
        let descriptor = pending
            .publish_checkpoint(
                managed.path(),
                CheckpointOperationId::parse("00000000000000000000000000000091").unwrap(),
            )
            .unwrap();
        pending.acknowledge_persistence(&descriptor).unwrap();
        pending.publish_running_authority().unwrap();
        let mut running = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        let epoch = running.world_epoch();
        let slot = super::super::contract::PreparedImportSlot::new();
        let foreign = super::super::contract::PreparedImportSlot::new();
        running.stage_prepared_import(&slot).unwrap();
        let lease = slot.acquire_work(4 * 1024 * MIB).unwrap();
        assert_eq!(lease.admission.source_world_epoch, epoch);
        assert_eq!(
            lease.admission.remaining_bytes + running.authoritative_memory_bytes(),
            4 * 1024 * MIB
        );
        assert!(matches!(
            running
                .service_after_command_drain(1000, SchedulerServiceMode::Background, None)
                .unwrap(),
            RunningAuthorityLoopProgress::ImportPending
        ));
        assert_eq!(running.completed_step(), 0);
        assert!(running.cancel_prepared_import(&foreign, 1000).is_err());
        assert!(running.cancel_prepared_import(&slot, 1000).is_err());
        assert_eq!(running.state(), RunningAuthorityLoopState::ImportPending);
        drop(lease);
        running.cancel_prepared_import(&slot, 1000).unwrap();
        assert!(matches!(
            running
                .service_after_command_drain(1000, SchedulerServiceMode::Background, None)
                .unwrap(),
            RunningAuthorityLoopProgress::Idle { .. }
        ));
        running
            .service_after_command_drain(1017, SchedulerServiceMode::Background, None)
            .unwrap();
        assert_eq!(running.completed_step(), 1);
        assert_eq!(running.world_epoch(), epoch);
    }

    #[test]
    fn metadata_only_checkpoint_size_matches_published_recurrent_population() {
        let managed = TestDirectory::create("predicted-size");
        let prepared = prepare_stage6a_p0_boundary(
            request(42),
            &[FreshRunSettingUpdate {
                path: "snakeCount".into(),
                value: 3.0,
            }],
            typescript_default_graph_spec(),
        )
        .unwrap();
        assert!(prepared.graph.total_state_size > 0);
        let mut shell = prepared.candidate.clone();
        for genome in &mut shell.population {
            genome.weights = Box::new([]);
        }
        for brain in &mut shell.brains {
            brain.recurrent = Box::new([]);
        }
        let predicted = boundary_decoded_byte_count(
            &shell,
            prepared.graph.spec(),
            prepared.graph.compiled(),
            &prepared.checkpoint_limits,
        )
        .unwrap();
        let state = AuthoritativeState::validate_and_own(
            prepared.candidate,
            prepared.graph,
            &prepared.admission_policy,
        )
        .unwrap();
        let descriptor = crate::engine::checkpoint::publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("42424242424242424242424242424242").unwrap(),
            state.world_epoch(),
            state.checkpoint_boundary().unwrap(),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &prepared.admission_policy,
        )
        .unwrap();
        assert_eq!(
            predicted,
            u64::from_str_radix(&descriptor.decoded_byte_count_hex, 16).unwrap()
        );
        assert!(
            predicted
                > 4 * 3 * (state.graph().total_parameters + state.graph().total_state_size) as u64
        );
    }

    #[test]
    fn oversized_population_rejects_before_numeric_initialization() {
        use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
        let graph = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "large".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 83,
                        hidden_sizes: vec![512, 512],
                        output_size: 512,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 512,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                GraphEdge {
                    from: "input".into(),
                    to: "large".into(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "large".into(),
                    to: "head".into(),
                    from_port: None,
                    to_port: None,
                },
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".into(),
                port: None,
            }],
            output_size: 2,
        };
        let error = match prepare_stage6a_p0_boundary(
            Stage6aP0FreshRunRequest {
                memory_ceiling_bytes: 4 * 1024 * MIB,
                ..request(42)
            },
            &[
                FreshRunSettingUpdate {
                    path: "snakeCount".into(),
                    value: 300.0,
                },
                FreshRunSettingUpdate {
                    path: "baselineBots.count".into(),
                    value: 0.0,
                },
            ],
            graph,
        ) {
            Ok(_) => panic!("oversized population must reject before initialization"),
            Err(error) => error,
        };
        assert!(matches!(error, FreshRunError::CheckpointTooLarge {
            decoded_bytes, limit_bytes: 536_870_912, population_count: 300,
            parameters_per_genome: 569_346,
        } if decoded_bytes > 300 * 569_346 * 4));
        assert!(error
            .to_string()
            .contains("Reduce NPC snakes or brain size"));
    }

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

    /// Publish, acknowledge, and activate one disposable real run-start boundary.
    fn activate_pending_run_start(
        mut pending: PendingRunStartTransition,
        label: &str,
        operation_hex: &str,
    ) -> PendingRunStartTransition {
        let managed = TestDirectory::create(label);
        let operation = CheckpointOperationId::parse(operation_hex)
            .expect("test operation ID must be canonical");
        let descriptor = pending
            .publish_checkpoint(managed.path(), operation)
            .expect("test boundary must publish its immutable checkpoint");
        pending
            .acknowledge_persistence(&descriptor)
            .expect("exact test descriptor must acknowledge");
        pending
            .publish_running_authority()
            .expect("durable test boundary must activate");
        pending
    }

    /// Build a valid fixed-P0 boundary whose generation ends after eight steps.
    fn short_generation_pending(seed: u32) -> PendingRunStartTransition {
        let mut prepared = prepare_stage6a_p0_boundary(
            Stage6aP0FreshRunRequest {
                memory_ceiling_bytes: 4 * 1024 * MIB,
                ..request(seed)
            },
            &[],
            typescript_default_graph_spec(),
        )
        .expect("fixed P0 boundary must construct before test-only shortening");
        prepared.candidate.config.fixed_step_seconds = 1.0;
        let generation_seconds = prepared
            .candidate
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == "generationSeconds")
            .expect("complete settings must contain generationSeconds");
        generation_seconds.value = NormalizedSettingValue::Float(8.0);
        prepared.candidate.identity.config_hash =
            normalized_config_hash(&prepared.candidate.config)
                .expect("short valid test configuration must hash");
        PendingRunStartTransition::admit(
            prepared.candidate,
            prepared.graph,
            prepared.admission_policy,
            prepared.checkpoint_limits,
            prepared.graph_limits,
            prepared.work_limits,
        )
        .expect("short valid test boundary must admit")
    }

    /// Queue limits for one local real-authority thread integration test.
    fn running_runtime_init() -> EngineInit {
        EngineInit {
            contract_version: ENGINE_CONTRACT_VERSION,
            inbound: InboundLimits {
                max_batches: 4,
                max_commands: 8,
                max_owned_bytes: 64,
                max_batch_commands: 4,
                max_batch_owned_bytes: 32,
            },
            output: OutputLimits {
                max_reliable: 8,
                max_reliable_owned_bytes: 4096,
                max_discrete: 4,
                max_discrete_owned_bytes: 1024,
                max_total_owned_bytes: 8192,
                max_event_owned_bytes: 1024,
                max_frame_connections: 4,
            },
        }
    }

    /// One exact sequenced probe for coarse queue/wake integration.
    fn runtime_probe(sequence: u64, value: u8) -> CommandBatch {
        CommandBatch {
            contract_version: ENGINE_CONTRACT_VERSION,
            commands: vec![SequencedCommand {
                sequence,
                command: EngineCommand::Probe {
                    correlation_id: sequence + 100,
                    payload: vec![value],
                },
            }]
            .into_boxed_slice(),
        }
    }

    /// Wait only on bounded scalar health; never borrow background authority.
    fn wait_for_runtime(runtime: &EngineRuntime, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            std::thread::yield_now();
        }
        panic!(
            "running runtime condition not reached: {:?}",
            runtime.health()
        );
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

    #[cfg(feature = "engine-test-hooks")]
    #[test]
    fn trained_qualification_values_reuse_admitted_boxes_and_preserve_initialization_rng() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/portable-brain-v1.slither-brain"),
        )
        .unwrap();
        let artifact = super::super::brain_artifact::validate_brain_artifact(&bytes).unwrap();
        let contract = super::super::sensor_contract::SensorContractIdentity::new(
            super::super::sensor_contract::SensorProfile::Basic,
            8,
        )
        .unwrap();
        let mut settings = production_default_settings(55, 10);
        for setting in &mut settings {
            match setting.path.as_str() {
                "sense.bubbleBins" => setting.value = NormalizedSettingValue::Integer(8),
                "brain.controlDt" => {
                    setting.value =
                        NormalizedSettingValue::Float(artifact.deployment.control_interval_seconds)
                }
                "evolvedRespawn.recurrentState" => {
                    setting.value = NormalizedSettingValue::Integer(1)
                }
                _ => {}
            }
        }
        let mut prepared = prepare_fresh_boundary_with_sensor_and_baseline(
            request(42),
            &numeric_settings(settings),
            artifact.graph.spec().clone(),
            true,
            Some(contract),
            RunningStepWorkLimits {
                ray_work: Some(
                    super::super::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap(),
                ),
                ..RunningStepWorkLimits::provisional_defaults()
            },
            None,
        )
        .unwrap();
        let pointers: Vec<_> = prepared
            .candidate
            .population
            .iter()
            .map(|genome| genome.weights.as_ptr())
            .collect();
        let rng = prepared.candidate.rng.clone();
        let ids: Vec<_> = prepared
            .candidate
            .population
            .iter()
            .map(|genome| (genome.slot, genome.brain, genome.lineage.clone()))
            .collect();
        install_qualified_population_artifact(&mut prepared, &artifact).unwrap();
        assert_eq!(prepared.candidate.rng, rng);
        for (index, genome) in prepared.candidate.population.iter().enumerate() {
            assert_eq!(&*genome.weights, artifact.weights.as_slice());
            assert_eq!(genome.weights.as_ptr(), pointers[index]);
            assert_eq!(
                (genome.slot, genome.brain, genome.lineage.clone()),
                ids[index]
            );
        }
        assert!(prepared
            .candidate
            .brains
            .iter()
            .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));
        let mut families = vec![
            artifact,
            super::super::brain_artifact::validate_brain_artifact(
                &super::super::trained_cohort::fixture_bytes(0.125, 0.06),
            )
            .unwrap(),
        ];
        // A bad later source must reject atomically before touching even slot zero.
        let original = prepared.candidate.population[0].weights.to_vec();
        families[1].deployment.control_interval_seconds = 0.03;
        assert!(install_qualified_population_artifacts(&mut prepared, &families).is_err());
        assert_eq!(
            &*prepared.candidate.population[0].weights,
            original.as_slice()
        );
        families[1].deployment.control_interval_seconds = 0.06;
        install_qualified_population_artifacts(&mut prepared, &families).unwrap();
        assert_eq!(prepared.candidate.rng, rng);
        for (index, genome) in prepared.candidate.population.iter().enumerate() {
            assert_eq!(
                &*genome.weights,
                families[genome.slot as usize % 2].weights.as_slice()
            );
            assert_eq!(genome.weights.as_ptr(), pointers[index]);
            assert_eq!(
                (genome.slot, genome.brain, genome.lineage.clone()),
                ids[index]
            );
        }
        assert!(prepared
            .candidate
            .brains
            .iter()
            .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));
        prepared
            .candidate
            .config
            .settings
            .iter_mut()
            .find(|entry| entry.path == "brain.controlDt")
            .unwrap()
            .value = NormalizedSettingValue::Float(0.03);
        assert!(install_qualified_population_artifacts(&mut prepared, &families).is_err());
    }

    fn request(seed: u32) -> Stage6aP0FreshRunRequest {
        Stage6aP0FreshRunRequest {
            run_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(),
            seed,
            memory_ceiling_bytes: TEST_MEMORY_CEILING,
        }
    }

    fn numeric_settings(
        settings: Vec<super::super::state::NormalizedSetting>,
    ) -> Vec<FreshRunSettingUpdate> {
        settings
            .into_iter()
            .map(|setting| FreshRunSettingUpdate {
                path: setting.path,
                value: match setting.value {
                    NormalizedSettingValue::Bool(value) => f64::from(value),
                    NormalizedSettingValue::Integer(value) => value as f64,
                    NormalizedSettingValue::Float(value) => value,
                    NormalizedSettingValue::Text(_) => panic!("numeric fresh setting"),
                },
            })
            .collect()
    }

    /// Reset carries numeric settings; Rust regenerates the structural digest.
    fn numeric_metadata_settings(metadata: &str) -> Vec<FreshRunSettingUpdate> {
        let decoded: serde_json::Value = serde_json::from_str(metadata).unwrap();
        decoded["settings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|setting| {
                let value = setting["value"]
                    .as_f64()
                    .or_else(|| setting["value"].as_bool().map(f64::from))?;
                Some(FreshRunSettingUpdate {
                    path: setting["path"].as_str().unwrap().into(),
                    value,
                })
            })
            .collect()
    }

    #[test]
    fn qualified_fresh_defaults_restore_with_the_recorded_sse2_backend() {
        let mut fresh = prepare_default_fresh_run(request(42)).unwrap();
        let metadata = fresh.startup_metadata_json().unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(decoded["sensorVersion"], 4);
        assert_eq!(decoded["mathBackend"], InferenceMathBackend::Sse2.label());
        let settings = numeric_metadata_settings(&metadata);
        for (path, value) in [
            ("sense.profile", 1.0),
            ("sense.bubbleBins", 16.0),
            ("sense.maxPelletChecks", 3000.0),
            ("sense.maxSegmentChecks", 2200.0),
            ("sense.rayMinimum", 1.0),
            ("sense.rayCap", 256.0),
            ("sense.rayBatch", 65536.0),
            ("sense.rayMaximumDue", 512.0),
            ("baselineBots.controlVersion", 2.0),
            ("baselineBots.mode", 1.0),
            ("baselineBots.controlDt", 1.0 / 60.0),
            ("generationSeconds", 240.0),
            ("evolvedRespawn.enabled", 0.0),
        ] {
            assert_eq!(
                settings
                    .iter()
                    .find(|setting| setting.path == path)
                    .unwrap()
                    .value,
                value
            );
        }
        let directory = TestDirectory::create("qualified-fresh-defaults");
        let descriptor = fresh
            .publish_checkpoint(
                directory.path(),
                CheckpointOperationId::parse("abcdefabcdefabcdefabcdefabcdefab").unwrap(),
            )
            .unwrap();
        let mut restored = prepare_stage6a_p0_checkpoint_restore(
            directory.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
        )
        .unwrap();
        assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
        let recovered = prepare_stage6a_p0_compatible_checkpoint_restore(
            directory.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
        )
        .unwrap()
        .into_committed_recovery_branch("sse2-recovery".into())
        .unwrap();
        let recovered_metadata: serde_json::Value =
            serde_json::from_str(&recovered.startup_metadata_json().unwrap()).unwrap();
        assert_eq!(
            recovered_metadata["mathBackend"],
            InferenceMathBackend::Sse2.label()
        );
        for math in [InferenceMathBackend::Scalar, InferenceMathBackend::Sse2] {
            let reset = prepare_configured_fresh_run_with_math(
                request(42),
                &settings,
                super::super::graph::default_stack_graph_spec(264),
                math,
            )
            .unwrap();
            let reset_metadata: serde_json::Value =
                serde_json::from_str(&reset.startup_metadata_json().unwrap()).unwrap();
            assert_eq!(reset_metadata["mathBackend"], math.label());
            assert_eq!(reset_metadata["settings"], decoded["settings"]);
        }
        fresh.acknowledge_persistence(&descriptor).unwrap();
        for transition in [&mut fresh, &mut restored] {
            transition.publish_running_authority().unwrap();
            transition
                .publish_first_scheduled_frame_v1(&mut Vec::new())
                .unwrap();
            assert_eq!(transition.completed_step(), 1);
        }
    }

    #[test]
    fn configured_fresh_life_contract_is_complete_and_preserves_frozen_numeric_initialization() {
        let legacy =
            prepare_stage6a_p0_boundary(request(42), &[], typescript_default_graph_spec()).unwrap();
        let modern =
            prepare_fresh_boundary(request(42), &[], typescript_default_graph_spec(), true)
                .unwrap();
        assert_eq!(
            modern.candidate.rng.version,
            RNG_BUNDLE_WITH_EVOLVED_VERSION
        );
        assert_eq!(
            modern.candidate.versions.rng_bundle,
            RNG_BUNDLE_WITH_EVOLVED_VERSION
        );
        assert_eq!(
            modern.candidate.rng.evolved_respawns.len(),
            STAGE6A_P0_POPULATION_COUNT
        );
        assert_eq!(modern.candidate.population, legacy.candidate.population);
        assert_eq!(modern.candidate.rng.world, legacy.candidate.rng.world);
        assert_eq!(
            modern.candidate.rng.evolution,
            legacy.candidate.rng.evolution
        );
        assert_eq!(
            modern.candidate.rng.baselines,
            legacy.candidate.rng.baselines
        );
        let old = numeric_settings(typescript_default_settings(
            STAGE6A_P0_POPULATION_COUNT,
            STAGE6A_P0_BASELINE_COUNT,
        ));
        let configured = prepare_configured_fresh_run_with_settings_and_graph(
            request(42),
            &old,
            typescript_default_graph_spec(),
        )
        .unwrap();
        let metadata: serde_json::Value =
            serde_json::from_str(&configured.startup_metadata_json().unwrap()).unwrap();
        assert!(!metadata["settings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|setting| setting["path"] == "evolvedRespawn.contractVersion"));
        for partial in [
            vec![FreshRunSettingUpdate {
                path: "evolvedRespawn.contractVersion".into(),
                value: 1.0,
            }],
            vec![FreshRunSettingUpdate {
                path: "evolvedRespawn.enabled".into(),
                value: 1.0,
            }],
        ] {
            assert!(prepare_configured_fresh_run_with_settings_and_graph(
                request(42),
                &partial,
                typescript_default_graph_spec()
            )
            .is_err());
        }
    }

    #[test]
    fn production_restore_roundtrips_the_selected_life_contract_and_rejects_unknown_schemas() {
        let managed = TestDirectory::create("life-contract-restore");
        let mut settings = numeric_settings(production_default_settings(3, 0));
        for setting in &mut settings {
            if setting.path == "evolvedRespawn.enabled" {
                setting.value = 1.0;
            }
            if setting.path == "evolvedRespawn.recurrentState" {
                setting.value = 1.0;
            }
        }
        let mut pending = prepare_configured_fresh_run_with_settings_and_graph(
            request(42),
            &settings,
            typescript_default_graph_spec(),
        )
        .unwrap();
        let before: serde_json::Value =
            serde_json::from_str(&pending.startup_metadata_json().unwrap()).unwrap();
        let descriptor = pending
            .publish_checkpoint(
                managed.path(),
                CheckpointOperationId::parse("89898989898989898989898989898989").unwrap(),
            )
            .unwrap();
        let mut restored =
            prepare_stage6a_p0_checkpoint_restore(managed.path(), &descriptor, TEST_MEMORY_CEILING)
                .unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&restored.startup_metadata_json().unwrap()).unwrap();
        assert_eq!(before["settings"], after["settings"]);
        assert_eq!(before["seed"], after["seed"]);
        restored.publish_running_authority().unwrap();
        assert_eq!(restored.snake_count(), 3);
        restored
            .publish_first_scheduled_frame_v1(&mut Vec::new())
            .unwrap();
        let mut prepared = prepare_fresh_boundary(
            request(42),
            &settings,
            typescript_default_graph_spec(),
            true,
        )
        .unwrap();
        prepared.candidate.config.settings.pop();
        prepared.candidate.config.settings_schema_sha256 =
            normalized_settings_schema_hash(&prepared.candidate.config.settings).unwrap();
        prepared.candidate.identity.config_hash =
            normalized_config_hash(&prepared.candidate.config).unwrap();
        assert!(super::super::state::AuthoritativeState::validate_and_own(
            prepared.candidate,
            prepared.graph,
            &production_restore_policy(TEST_MEMORY_CEILING, true).unwrap()
        )
        .is_err());
    }

    #[test]
    fn baseline_v2_cold_constructor_restores_both_profiles_and_modes_with_exact_metadata() {
        use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Settings};
        use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
        use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for mode in [BaselineV2Mode::Deterministic, BaselineV2Mode::Stochastic] {
                let width = SensorContractIdentity::new(profile, 8)
                    .unwrap()
                    .input_size();
                let graph = GraphSpec {
                    nodes: vec![
                        GraphNodeSpec {
                            id: "input".into(),
                            kind: GraphNodeKind::Input { output_size: width },
                        },
                        GraphNodeSpec {
                            id: "head".into(),
                            kind: GraphNodeKind::Dense {
                                input_size: width,
                                output_size: 2,
                            },
                        },
                    ],
                    edges: vec![GraphEdge {
                        from: "input".into(),
                        to: "head".into(),
                        from_port: None,
                        to_port: None,
                    }],
                    outputs: vec![GraphOutputRef {
                        node_id: "head".into(),
                        port: None,
                    }],
                    output_size: 2,
                };
                let settings = numeric_settings(production_default_settings(12, 2));
                let spec = BaselineV2Settings {
                    mode,
                    control_interval_seconds: 0.06,
                };
                let limits = RunningStepWorkLimits {
                    ray_work: Some(
                        crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap(),
                    ),
                    calculation_workers: 3,
                    ..RunningStepWorkLimits::provisional_defaults()
                };
                assert!(prepare_baseline_v2_qualification_run(
                    request(42),
                    &settings,
                    graph.clone(),
                    SensorProfile::V3,
                    8,
                    spec,
                    limits
                )
                .is_err());
                assert!(prepare_baseline_v2_qualification_run(
                    request(42),
                    &settings,
                    graph.clone(),
                    profile,
                    8,
                    spec,
                    RunningStepWorkLimits {
                        ray_work: None,
                        ..limits
                    }
                )
                .is_err());
                let mut pending = prepare_baseline_v2_qualification_run(
                    request(42),
                    &settings,
                    graph.clone(),
                    profile,
                    8,
                    spec,
                    limits,
                )
                .unwrap();
                let metadata = pending.startup_metadata_json().unwrap();
                assert!(metadata.contains("baselineBots.controlVersion"));
                assert!(metadata.contains("baselineBots.controlDt"));
                let explicit_settings = numeric_metadata_settings(&metadata);
                let mut configured = prepare_configured_fresh_run_with_settings_and_graph(
                    request(42),
                    &explicit_settings,
                    graph.clone(),
                )
                .unwrap();
                assert_eq!(configured.startup_metadata_json().unwrap(), metadata);
                for path in [
                    "sense.profile",
                    "sense.contractVersion",
                    "brain.sensorVersion",
                    "sense.rayMinimum",
                    "sense.rayCap",
                    "sense.rayBatch",
                    "sense.rayMaximumDue",
                    "baselineBots.controlVersion",
                    "baselineBots.mode",
                    "baselineBots.controlDt",
                ] {
                    let partial = explicit_settings
                        .iter()
                        .filter(|setting| setting.path != path)
                        .cloned()
                        .collect::<Vec<_>>();
                    assert!(
                        prepare_configured_fresh_run_with_settings_and_graph(
                            request(42),
                            &partial,
                            graph.clone()
                        )
                        .is_err(),
                        "accepted missing {path}"
                    );
                }
                for (path, value) in [
                    ("sense.profile", 2.0),
                    ("brain.sensorVersion", 3.0),
                    ("sense.rayMinimum", 0.0),
                    ("sense.rayCap", 0.5),
                    ("sense.rayMaximumDue", 513.0),
                    ("sense.rayMaximumDue", 1.0),
                    ("baselineBots.mode", 2.0),
                    ("baselineBots.controlDt", 0.0),
                ] {
                    let mut invalid = explicit_settings.clone();
                    invalid
                        .iter_mut()
                        .find(|setting| setting.path == path)
                        .unwrap()
                        .value = value;
                    assert!(
                        prepare_configured_fresh_run_with_settings_and_graph(
                            request(42),
                            &invalid,
                            graph.clone()
                        )
                        .is_err(),
                        "accepted invalid {path}"
                    );
                }
                let mut duplicate = explicit_settings.clone();
                duplicate.push(FreshRunSettingUpdate {
                    path: "sense.rayMinimum".into(),
                    value: 1.0,
                });
                assert!(prepare_configured_fresh_run_with_settings_and_graph(
                    request(42),
                    &duplicate,
                    graph.clone()
                )
                .is_err());
                assert!(prepare_configured_fresh_run_with_settings_and_graph(
                    request(42),
                    &explicit_settings,
                    typescript_default_graph_spec()
                )
                .is_err());
                let directory = TestDirectory::create("v2-cold-restore");
                let descriptor = pending
                    .publish_checkpoint(
                        directory.path(),
                        CheckpointOperationId::parse("56565656565656565656565656565656").unwrap(),
                    )
                    .unwrap();
                let configured_directory = TestDirectory::create("v2-configured-reset");
                let configured_descriptor = configured
                    .publish_checkpoint(
                        configured_directory.path(),
                        CheckpointOperationId::parse("67676767676767676767676767676767").unwrap(),
                    )
                    .unwrap();
                assert_eq!(
                    configured_descriptor.logical_root_sha256,
                    descriptor.logical_root_sha256
                );
                let restored = PendingRunStartTransition::restore_committed(
                    directory.path(),
                    &descriptor,
                    production_restore_policy(TEST_MEMORY_CEILING, true).unwrap(),
                    production_checkpoint_limits(),
                    stage6a_p0_graph_limits(),
                    limits,
                )
                .unwrap();
                assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
                let mut oversized = production_restore_policy(TEST_MEMORY_CEILING, true).unwrap();
                assert_eq!(oversized.allowed_settings_schema_sha256.len(), 9);
                oversized
                    .allowed_settings_schema_sha256
                    .push(oversized.expected_settings_schema_sha256.clone());
                assert!(PendingRunStartTransition::restore_committed(
                    directory.path(),
                    &descriptor,
                    oversized,
                    production_checkpoint_limits(),
                    stage6a_p0_graph_limits(),
                    limits
                )
                .is_err());
            }
        }
    }

    #[test]
    fn baseline_v2_rng_boundary_roundtrips_managed_checkpoint_and_rejects_mixed_contracts() {
        use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Settings};
        use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
        use crate::engine::rng::{baseline_v2_lifecycle_rng, derive_baseline_v2_behaviors};
        use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
        use crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
        let graph = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 144 },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 144,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![GraphEdge {
                from: "input".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            }],
            outputs: vec![GraphOutputRef {
                node_id: "head".into(),
                port: None,
            }],
            output_size: 2,
        };
        let mut prepared = prepare_fresh_boundary_with_sensor(
            request(42),
            &numeric_settings(production_default_settings(12, 2)),
            graph,
            true,
            Some(SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap()),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        BaselineV2Settings {
            mode: BaselineV2Mode::Stochastic,
            control_interval_seconds: 0.06,
        }
        .record_settings(&mut prepared.candidate.config.settings)
        .unwrap();
        prepared.candidate.config.settings_schema_sha256 =
            normalized_settings_schema_hash(&prepared.candidate.config.settings).unwrap();
        prepared.candidate.identity.config_hash =
            normalized_config_hash(&prepared.candidate.config).unwrap();
        prepared.admission_policy.expected_settings_schema_sha256 =
            prepared.candidate.config.settings_schema_sha256.clone();
        prepared.candidate.rng.version = RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
        prepared.candidate.versions.rng_bundle = RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
        prepared.candidate.rng.baseline_behaviors =
            derive_baseline_v2_behaviors(42, 1, 1, 2, false).unwrap();
        for stream in &mut prepared.candidate.rng.baselines {
            stream.state = baseline_v2_lifecycle_rng(42, 1, 1, stream.slot, false).export_state();
        }
        let expected = prepared.candidate.rng.clone();
        let mut behavior = expected.baseline_behaviors[0].runtime().unwrap();
        let mut lifecycle =
            crate::engine::rng::StatefulRng::from_state(&expected.baselines[0].state).unwrap();
        assert_ne!(
            behavior.next_f64().to_bits(),
            lifecycle.next_f64().to_bits()
        );
        let authority = AuthoritativeState::validate_and_own(
            prepared.candidate.clone(),
            Arc::clone(&prepared.graph),
            &prepared.admission_policy,
        )
        .unwrap();
        prepared.work_limits.ray_work =
            Some(crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap());
        crate::engine::running_step::RunningStepCoordinator::try_new(
            &authority,
            prepared.work_limits,
        )
        .unwrap();
        let directory = TestDirectory::create("baseline-v2-rng-boundary");
        let descriptor = crate::engine::checkpoint::publish_checkpoint(
            directory.path(),
            CheckpointOperationId::parse("23232323232323232323232323232323").unwrap(),
            1,
            authority.checkpoint_boundary().unwrap(),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &prepared.admission_policy,
        )
        .unwrap();
        let restored = crate::engine::checkpoint::restore_checkpoint(
            &directory.path().join(&descriptor.relative_filename),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &prepared.admission_policy,
        )
        .unwrap();
        assert_eq!(restored.state.state().rng, expected);
        #[cfg(feature = "engine-test-hooks")]
        {
            let mut continuations = Vec::new();
            let mut captured_chunks = Vec::new();
            for (workers, capture, shadow) in [
                (1, false, false),
                (3, false, false),
                (1, true, false),
                (3, true, false),
                (1, true, true),
                (3, true, true),
            ] {
                let limits = RunningStepWorkLimits {
                    calculation_workers: workers,
                    ..prepared.work_limits
                };
                let mut pending = PendingRunStartTransition::admit(
                    prepared.candidate.clone(),
                    Arc::clone(&prepared.graph),
                    current_build_policy(
                        TEST_MEMORY_CEILING,
                        prepared.candidate.config.settings_schema_sha256.clone(),
                        true,
                    ),
                    prepared.checkpoint_limits.clone(),
                    prepared.graph_limits.clone(),
                    limits,
                )
                .unwrap();
                let run_directory = TestDirectory::create("v2-joined-control");
                let checkpoint = pending
                    .publish_checkpoint(
                        run_directory.path(),
                        CheckpointOperationId::parse("34343434343434343434343434343434").unwrap(),
                    )
                    .unwrap();
                pending.acknowledge_persistence(&checkpoint).unwrap();
                pending.publish_running_authority().unwrap();
                let mut running = pending
                    .into_running_loop(
                        crate::engine::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
                        0,
                    )
                    .unwrap();
                assert!(running.published_teacher_boundary().is_none());
                if capture {
                    assert!(running.configure_teacher_capture(&[0], 1).is_err());
                    assert!(running.configure_teacher_capture(&[2], 65536).is_err());
                    let request = crate::engine::teacher_runtime::TeacherCaptureRequest {
                        capture_epoch: [17; 16],
                        expected_run_id: prepared.candidate.identity.run_id.clone(),
                        expected_config_revision: prepared.candidate.identity.config_revision,
                        baseline_slots: vec![0].into_boxed_slice(),
                        limits: crate::engine::teacher_runtime::TeacherCaptureLimits {
                            pending_bytes: 1024 * 1024,
                            queue_chunks: 4,
                            queue_bytes: 4 * 1024 * 1024,
                        },
                    };
                    let mut bad = request.clone();
                    bad.expected_run_id = "wrong-run".into();
                    assert!(running.start_teacher_capture(&bad).is_err());
                    let contract = running.start_teacher_capture(&request).unwrap();
                    assert_eq!(contract.teacher_mode, 1);
                    assert_eq!(contract.control_interval_seconds, 0.06);
                    assert_eq!(
                        contract.sensor_sha256,
                        SensorContractIdentity::new(SensorProfile::Motion, 8)
                            .unwrap()
                            .sha256()
                            .unwrap()
                    );
                    assert!(running.start_teacher_capture(&request).is_err());
                    assert!(running.configure_teacher_capture(&[], 0).is_err());
                    if shadow {
                        let candidate = crate::engine::shadows::ShadowCandidate::synthetic(
                            prepared.graph.compiled(),
                            contract.sensor_sha256,
                            0.06,
                        );
                        let mut activation = crate::engine::shadows::ShadowActivationRequest {
                            expected_run_id: prepared.candidate.identity.run_id.clone(),
                            expected_config_revision: prepared.candidate.identity.config_revision,
                            capture_epoch: [17; 16],
                            shadow_slot: 0,
                            baseline_slot: 0,
                            candidate,
                        };
                        let mut stale = activation.clone();
                        stale.capture_epoch = [18; 16];
                        assert!(running.activate_shadow(&stale, 1).is_err());
                        for slot in 0..4 {
                            activation.shadow_slot = slot;
                            running
                                .activate_shadow(&activation, u64::from(slot) + 1)
                                .unwrap();
                        }
                        activation.shadow_slot = 4;
                        assert!(running.activate_shadow(&activation, 5).is_err());
                    }
                }
                let mut captured_rows = 0;
                for step in 1..=16 {
                    let outcome = running
                        .service_after_command_drain(
                            (step * 1000u64).div_ceil(60),
                            crate::engine::scheduler::SchedulerServiceMode::Background,
                            None,
                        )
                        .unwrap();
                    assert!(matches!(
                        outcome,
                        crate::engine::running_loop::RunningAuthorityLoopProgress::Published { .. }
                    ));
                    assert_eq!(running.completed_step(), step);
                    if shadow && step == 1 {
                        // Resolve the spawned baseline through real frame IDs and native full identities.
                        let physical = running.published_teacher_boundary().unwrap().decisions[0]
                            .physical_snake_id;
                        let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
                        running.pack_display_into(1, &mut frame).unwrap();
                        let read = |offset: usize| {
                            f32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap())
                        };
                        let mut offset = 7 * 4;
                        let mut selected = None;
                        for _ in 0..read(2 * 4) as usize {
                            let frame_id = read(offset) as u32;
                            if let Some(entity) = running
                                .select_entity(
                                    crate::engine::state::EntitySelectionTarget::Frame(frame_id),
                                    &prepared.candidate.identity.run_id,
                                    1,
                                )
                                .unwrap()
                            {
                                if entity.token.physical_id() == physical {
                                    selected = Some(entity);
                                    break;
                                }
                            }
                            offset += (8 + 2 * read(offset + 7 * 4) as usize) * 4;
                        }
                        let selected = selected.expect("spawned teacher in native frame");
                        running
                            .configure_inspector(
                                &prepared.candidate.identity.run_id,
                                Some((selected.token, selected.frame_v1_id)),
                                crate::engine::inspector::MAX_INSPECTOR_BYTES,
                            )
                            .unwrap();
                    }
                    if let Some(rows) = running.published_teacher_boundary() {
                        assert!(capture);
                        assert_eq!(rows.key.source_completed_step() + 1, step);
                        for (index, decision) in rows.decisions.iter().enumerate() {
                            assert_eq!(decision.baseline_slot, 0);
                            assert_eq!(rows.observation(index).unwrap().len(), 144);
                            assert_eq!(
                                rows.outcome(index).unwrap().physical_snake_id,
                                decision.physical_snake_id
                            );
                            assert!(rows.outcome(index + rows.decisions.len()).is_none());
                            captured_rows += 1;
                        }
                    }
                }
                if shadow {
                    for snapshot in running.shadow_snapshots().iter().flatten() {
                        assert!(snapshot.enabled, "{:?}", snapshot.error);
                        assert_eq!(snapshot.totals.samples, captured_rows as u64);
                        assert_eq!(snapshot.completed_step, 16);
                    }
                    let observed = running
                        .inspector_observation()
                        .expect("selected actual teacher row");
                    assert_eq!(
                        observed.source,
                        crate::engine::inspector::InspectorObservationSource::BaselineV2
                    );
                    assert!(observed.lifecycle.is_none());
                    for (index, snapshot) in running.shadow_snapshots().iter().enumerate() {
                        let snapshot = snapshot.unwrap();
                        let output =
                            observed.shadow_outputs[index].expect("exact joined shadow output");
                        assert_eq!(output.sample_step, snapshot.last_sample_step);
                        assert_eq!(Some(output.sample_step), observed.sample_step());
                        assert_eq!(output.raw_outputs, snapshot.raw_outputs);
                        assert_eq!(output.capture_epoch, [17; 16]);
                        assert_eq!(output.activation_sequence, index as u64 + 1);
                        assert_eq!(snapshot.physical_snake_id, observed.target.physical_id);
                    }
                    let before = running.qualification_continuation_sha256();
                    assert!(running.stop_shadow(0, [17; 16], 99).is_err());
                    assert!(running.shadow_snapshots()[0].unwrap().enabled);
                    running.stop_shadow(0, [17; 16], 1).unwrap();
                    assert!(!running.shadow_snapshots()[0].unwrap().enabled);
                    assert!(running.shadow_snapshots()[1].unwrap().enabled);
                    assert_eq!(before, running.qualification_continuation_sha256());
                }
                if capture {
                    assert!(captured_rows > 0);
                    running.stop_teacher_capture().unwrap();
                    assert!(running
                        .shadow_snapshots()
                        .iter()
                        .flatten()
                        .all(|snapshot| !snapshot.enabled));
                    let status = running.teacher_capture_status().unwrap();
                    assert!(status.2 && status.3.is_none());
                    let drained = running.drain_teacher_chunks(4, 65536).unwrap().unwrap();
                    assert_eq!(drained.chunks.len(), 1);
                    let decoded = crate::engine::teacher_samples::TeacherChunkV1::decode(
                        &drained.chunks[0].bytes,
                    )
                    .unwrap();
                    assert_eq!(decoded.samples.len(), captured_rows);
                    captured_chunks.push(drained.chunks[0].bytes.clone());
                }
                continuations.push(running.qualification_continuation_sha256());
            }
            assert!(continuations.iter().all(|value| value == &continuations[0]));
            assert_eq!(captured_chunks.len(), 4);
            assert!(captured_chunks
                .iter()
                .all(|bytes| bytes == &captured_chunks[0]));

            // Exercise the actual background owner with no teacher consumer and
            // no controller/generation acknowledgements during normal stepping.
            let mut candidate = prepared.candidate.clone();
            candidate.config.requested_sim_speed = 12.0;
            // This tiny graph/population needs bounded scratch, while the existing
            // 2 GiB test ceiling must still charge both generation authorities.
            candidate.config.worker_scratch_bytes = 64 * 1024 * 1024;
            candidate.config.checkpoint_scratch_bytes = 64 * 1024 * 1024;
            candidate
                .config
                .settings
                .iter_mut()
                .find(|setting| setting.path == "simSpeed")
                .unwrap()
                .value = NormalizedSettingValue::Float(12.0);
            candidate.identity.config_hash = normalized_config_hash(&candidate.config).unwrap();
            let request = crate::engine::teacher_runtime::TeacherCaptureRequest {
                capture_epoch: [19; 16],
                expected_run_id: candidate.identity.run_id.clone(),
                expected_config_revision: candidate.identity.config_revision,
                baseline_slots: vec![0].into_boxed_slice(),
                limits: crate::engine::teacher_runtime::TeacherCaptureLimits {
                    pending_bytes: 1024 * 1024,
                    queue_chunks: 1,
                    queue_bytes: 4 * 1024 * 1024,
                },
            };
            let mut pending = PendingRunStartTransition::admit(
                candidate,
                Arc::clone(&prepared.graph),
                current_build_policy(
                    TEST_MEMORY_CEILING,
                    prepared.candidate.config.settings_schema_sha256.clone(),
                    true,
                ),
                prepared.checkpoint_limits.clone(),
                prepared.graph_limits.clone(),
                prepared.work_limits,
            )
            .unwrap();
            let runtime_directory = TestDirectory::create("v2-background-capture");
            let checkpoint = pending
                .publish_checkpoint(
                    runtime_directory.path(),
                    CheckpointOperationId::parse("35353535353535353535353535353535").unwrap(),
                )
                .unwrap();
            pending.acknowledge_persistence(&checkpoint).unwrap();
            pending.publish_running_authority().unwrap();
            let running = pending
                .into_running_loop(
                    crate::engine::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
                    0,
                )
                .unwrap();
            let runtime = crate::engine::runtime::EngineRuntime::new_running_authority(
                crate::engine::generation_handoff_fixture::background_generation_handoff_runtime_init(),
                running, Arc::new(crate::engine::queues::NoopWakeSink),
            ).unwrap();
            runtime.start().unwrap();
            runtime
                .try_submit(crate::engine::contract::CommandBatch {
                    contract_version: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                    commands: vec![crate::engine::contract::SequencedCommand {
                        sequence: 1,
                        command: crate::engine::contract::EngineCommand::RunningAuthority(
                            crate::engine::contract::RunningAuthorityCommand::StartTeacherCapture(
                                Box::new(request),
                            ),
                        ),
                    }]
                    .into_boxed_slice(),
                })
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while runtime
                .health()
                .running_authority
                .as_ref()
                .unwrap()
                .completed_step
                < 240
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "background capture never reached target: {:?}",
                    runtime.health()
                );
                assert!(runtime.health().fault.is_none());
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let activation = crate::engine::shadows::ShadowActivationRequest {
                expected_run_id: prepared.candidate.identity.run_id.clone(),
                expected_config_revision: prepared.candidate.identity.config_revision,
                capture_epoch: [19; 16],
                shadow_slot: 0,
                baseline_slot: 0,
                candidate: crate::engine::shadows::ShadowCandidate::synthetic(
                    prepared.graph.compiled(),
                    SensorContractIdentity::new(SensorProfile::Motion, 8)
                        .unwrap()
                        .sha256()
                        .unwrap(),
                    0.06,
                ),
            };
            runtime
                .try_submit(crate::engine::contract::CommandBatch {
                    contract_version: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                    commands: vec![crate::engine::contract::SequencedCommand {
                        sequence: 2,
                        command: crate::engine::contract::EngineCommand::RunningAuthority(
                            crate::engine::contract::RunningAuthorityCommand::ActivateShadow(
                                Box::new(activation),
                            ),
                        ),
                    }]
                    .into_boxed_slice(),
                })
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while runtime.shadow_snapshots().unwrap()[0]
                .is_none_or(|snapshot| snapshot.totals.samples == 0)
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "shadow command/mailbox made no progress: {:?}",
                    runtime.health()
                );
                assert!(runtime.health().fault.is_none());
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            // Retain a real generation barrier without persistence acknowledgement;
            // diagnostic wake/drain must still make progress and cannot publish it.
            runtime
                .try_submit(crate::engine::contract::CommandBatch {
                    contract_version: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                    commands: vec![crate::engine::contract::SequencedCommand {
                        sequence: 3,
                        command: crate::engine::contract::EngineCommand::RunningAuthority(
                            crate::engine::contract::RunningAuthorityCommand::FinishGeneration {
                                request_id: "capture-retained-barrier".into(),
                                expected_run_id: prepared.candidate.identity.run_id.clone(),
                                expected_generation: 1,
                            },
                        ),
                    }]
                    .into_boxed_slice(),
                })
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while runtime.health().running_authority.as_ref().unwrap().loop_state
                != crate::engine::running_loop::RunningAuthorityLoopState::GenerationTransitionPending {
                assert!(std::time::Instant::now() < deadline, "manual barrier not reached: {:?}", runtime.health());
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let completed_step = runtime
                .health()
                .running_authority
                .as_ref()
                .unwrap()
                .completed_step;
            let teacher = runtime.drain_teacher_chunks(1, 65536).unwrap();
            assert!(teacher.error.is_none(), "{:?}", teacher.error);
            assert_eq!(teacher.contract.as_ref().unwrap().capture_epoch, [19; 16]);
            assert_eq!(teacher.chunks.len(), 1);
            assert!(teacher.snapshot.production_watermark >= 3);
            assert!(teacher.snapshot.dropped_chunks > 0);
            assert!(teacher.more_work);
            let sensor = SensorContractIdentity::decode_supported(
                teacher.sensor_contract_json.as_ref().unwrap().as_bytes(),
            )
            .unwrap();
            assert_eq!(
                sensor.sha256().unwrap(),
                teacher.contract.as_ref().unwrap().sensor_sha256
            );
            let decoded =
                crate::engine::teacher_samples::TeacherChunkV1::decode(&teacher.chunks[0].bytes)
                    .unwrap();
            assert_eq!(
                decoded.contract.teacher_sha256,
                teacher.contract.as_ref().unwrap().teacher_sha256
            );
            assert_eq!(runtime.health().processed_commands, 3);
            let mut blocked_chunks = 0;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while blocked_chunks == 0 {
                let drained = runtime.drain_teacher_chunks(16, 65536).unwrap();
                blocked_chunks += drained.chunks.len();
                assert!(
                    std::time::Instant::now() < deadline,
                    "diagnostic wake failed at retained generation barrier"
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert_eq!(
                runtime
                    .health()
                    .running_authority
                    .as_ref()
                    .unwrap()
                    .completed_step,
                completed_step
            );
            assert_eq!(
                runtime
                    .health()
                    .running_authority
                    .as_ref()
                    .unwrap()
                    .generation,
                1
            );
            runtime.request_stop();
            runtime.join().unwrap();
            let mut count = teacher.chunks.len() + blocked_chunks;
            for _ in 0..8 {
                let drained = runtime.drain_teacher_chunks(16, 65536).unwrap();
                count += drained.chunks.len();
                if !drained.more_work {
                    break;
                }
            }
            assert!(count >= 2);
            assert!(runtime.health().fault.is_none());
        }

        for change in 0..4 {
            let mut bad = prepared.candidate.clone();
            match change {
                0 => {
                    bad.rng.version = 2;
                    bad.versions.rng_bundle = 2;
                    bad.rng.baseline_behaviors.clear();
                }
                1 => {
                    bad.rng.baseline_behaviors.pop();
                }
                2 => bad.rng.baseline_behaviors[0].uniform_state = 0,
                _ => bad.rng.baseline_behaviors[1].slot = 0,
            };
            assert!(AuthoritativeState::validate_and_own(
                bad,
                Arc::clone(&prepared.graph),
                &prepared.admission_policy
            )
            .is_err());
        }
    }

    #[test]
    fn v4_state_admission_rejects_missing_digest_and_graph_mismatch_without_reinterpreting_numeric_environment(
    ) {
        use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
        use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};
        let contract = SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap();
        let graph = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 144 },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 144,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![GraphEdge {
                from: "input".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            }],
            outputs: vec![GraphOutputRef {
                node_id: "head".into(),
                port: None,
            }],
            output_size: 2,
        };
        let prepared = prepare_fresh_boundary_with_sensor(
            request(42),
            &[],
            graph,
            false,
            Some(contract.clone()),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        for path in [
            "sense.contractVersion",
            "sense.profile",
            "sense.contractSha256",
        ] {
            let mut missing = prepared.candidate.clone();
            missing.config.settings.retain(|entry| entry.path != path);
            missing.config.settings_schema_sha256 =
                normalized_settings_schema_hash(&missing.config.settings).unwrap();
            missing.identity.config_hash = normalized_config_hash(&missing.config).unwrap();
            let mut policy = current_build_policy(
                TEST_MEMORY_CEILING,
                missing.config.settings_schema_sha256.clone(),
                true,
            );
            policy.expected_settings_schema_sha256 = missing.config.settings_schema_sha256.clone();
            assert!(matches!(
                AuthoritativeState::validate_and_own(missing, Arc::clone(&prepared.graph), &policy),
                Err(StateError::InvalidField {
                    field: "config.sensor_contract",
                    ..
                })
            ));
        }
        let mut modified = prepared.candidate.clone();
        modified
            .config
            .settings
            .iter_mut()
            .find(|entry| entry.path == "sense.profile")
            .unwrap()
            .value = NormalizedSettingValue::Integer(0);
        modified.identity.config_hash = normalized_config_hash(&modified.config).unwrap();
        assert!(matches!(
            AuthoritativeState::validate_and_own(
                modified,
                Arc::clone(&prepared.graph),
                &prepared.admission_policy
            ),
            Err(StateError::InvalidField {
                field: "config.sensor_contract",
                ..
            })
        ));
        let mut changed_environment = prepared.candidate.clone();
        changed_environment
            .config
            .settings
            .iter_mut()
            .find(|entry| entry.path == "sense.rNearBase")
            .unwrap()
            .value = NormalizedSettingValue::Float(600.0);
        changed_environment.identity.config_hash =
            normalized_config_hash(&changed_environment.config).unwrap();
        let authority = AuthoritativeState::validate_and_own(
            changed_environment,
            Arc::clone(&prepared.graph),
            &prepared.admission_policy,
        )
        .unwrap();
        assert_eq!(
            SensorContractIdentity::from_settings(&authority.state().config.settings).unwrap(),
            contract
        );
        assert!(
            matches!(crate::engine::running_step::RunningStepCoordinator::try_new(&authority,RunningStepWorkLimits::provisional_defaults()),
            Err(crate::engine::running_step::RunningStepError::Config(error)) if matches!(*error,crate::engine::step_config::StepConfigError::MissingRayWorkLimits))
        );
        assert!(matches!(
            prepare_fresh_boundary_with_sensor(
                request(42),
                &[],
                typescript_default_graph_spec(),
                false,
                Some(contract),
                RunningStepWorkLimits::provisional_defaults()
            ),
            Err(FreshRunError::GraphInputMismatch { .. })
        ));
    }

    #[test]
    fn v4_durable_metadata_restores_recorded_ray_limits_and_runs_equivalent_fixed_steps() {
        use super::super::ray_work::RayWorkLimits;
        use super::super::sensor_contract::{SensorContractIdentity, SensorProfile};
        use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
        for profile in [SensorProfile::Basic, SensorProfile::Motion] {
            for bins in [8, 16, 32] {
                let contract = SensorContractIdentity::new(profile, bins).unwrap();
                let width = contract.input_size();
                let graph = GraphSpec {
                    nodes: vec![
                        GraphNodeSpec {
                            id: "input".into(),
                            kind: GraphNodeKind::Input { output_size: width },
                        },
                        GraphNodeSpec {
                            id: "memory".into(),
                            kind: GraphNodeKind::Rru {
                                input_size: width,
                                hidden_size: 8,
                            },
                        },
                        GraphNodeSpec {
                            id: "head".into(),
                            kind: GraphNodeKind::Dense {
                                input_size: 8,
                                output_size: 2,
                            },
                        },
                    ],
                    edges: vec![
                        GraphEdge {
                            from: "input".into(),
                            to: "memory".into(),
                            from_port: None,
                            to_port: None,
                        },
                        GraphEdge {
                            from: "memory".into(),
                            to: "head".into(),
                            from_port: None,
                            to_port: None,
                        },
                    ],
                    outputs: vec![GraphOutputRef {
                        node_id: "head".into(),
                        port: None,
                    }],
                    output_size: 2,
                };
                let settings = numeric_settings(production_default_settings(12, 2));
                let limits = RunningStepWorkLimits {
                    ray_work: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
                    calculation_workers: 3,
                    ..RunningStepWorkLimits::provisional_defaults()
                };
                let mut pending = prepare_v4_qualification_run(
                    request(42),
                    &settings,
                    graph,
                    true,
                    profile,
                    bins,
                    limits,
                )
                .unwrap();
                let metadata = pending.startup_metadata_json().unwrap();
                let decoded: serde_json::Value = serde_json::from_str(&metadata).unwrap();
                assert_eq!(decoded["sensorVersion"], 4);
                assert_eq!(
                    decoded["sensorContract"]["sha256"],
                    contract.sha256_hex().unwrap()
                );
                assert_eq!(
                    decoded["sensorContract"]["canonicalJson"],
                    String::from_utf8(contract.canonical_bytes().unwrap()).unwrap()
                );
                let managed = TestDirectory::create("v4-durable-contract");
                let descriptor = pending
                    .publish_checkpoint(
                        managed.path(),
                        CheckpointOperationId::parse("112233445566778899aabbccddeeff00").unwrap(),
                    )
                    .unwrap();
                let policy = production_restore_policy(TEST_MEMORY_CEILING, true).unwrap();
                let mut restored = PendingRunStartTransition::restore_committed(
                    managed.path(),
                    &descriptor,
                    policy,
                    production_checkpoint_limits(),
                    stage6a_p0_graph_limits(),
                    RunningStepWorkLimits {
                        ray_work: None,
                        ..limits
                    },
                )
                .unwrap();
                assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
                restored.configure_calculation_workers(1).unwrap();
                let mut conflicting = PendingRunStartTransition::restore_committed(
                    managed.path(),
                    &descriptor,
                    production_restore_policy(TEST_MEMORY_CEILING, true).unwrap(),
                    production_checkpoint_limits(),
                    stage6a_p0_graph_limits(),
                    RunningStepWorkLimits {
                        ray_work: Some(RayWorkLimits::new(1, 128, 65536, 512).unwrap()),
                        ..limits
                    },
                )
                .unwrap();
                conflicting.publish_running_authority().unwrap();
                let error = conflicting
                    .publish_first_scheduled_frame_v1(&mut Vec::new())
                    .unwrap_err();
                assert!(error.to_string().contains("recorded ray limits differ"));
                assert_eq!(conflicting.completed_step(), 0);
                let mut expected_frame = None;
                for transition in [&mut pending, &mut restored] {
                    transition.acknowledge_persistence(&descriptor).unwrap();
                    transition.publish_running_authority().unwrap();
                    let mut frame = Vec::new();
                    assert_eq!(
                        transition
                            .pack_initial_frame_v1(&mut frame)
                            .unwrap()
                            .alive_snakes,
                        14
                    );
                    transition
                        .publish_first_scheduled_frame_v1(&mut frame)
                        .unwrap();
                    assert_eq!(transition.completed_step(), 1);
                    if let Some(expected) = &expected_frame {
                        assert_eq!(&frame, expected);
                    } else {
                        expected_frame = Some(frame);
                    }
                }
            }
        }
    }

    #[cfg(feature = "engine-test-hooks")]
    #[test]
    fn qualified_checkpoint_restore_preserves_exact_sse2_contract_and_worker_continuation() {
        use super::super::checkpoint::CheckpointOperationId;
        use super::super::ray_work::RayWorkLimits;
        use super::super::sensor_contract::SensorProfile;
        let artifact = super::super::brain_artifact::validate_brain_artifact(
            &super::super::trained_cohort::fixture_bytes(0.0, 0.06),
        )
        .unwrap();
        let limits = RunningStepWorkLimits {
            ray_work: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
            calculation_workers: 1,
            ..RunningStepWorkLimits::provisional_defaults()
        };
        let mut pending = prepare_sensor_qualification_run(
            request(42),
            &numeric_settings(production_default_settings(12, 2)),
            artifact.graph.spec().clone(),
            SensorProfile::Basic,
            8,
            limits,
            InferenceMathBackend::Sse2,
            Some(super::super::baseline_v2::BaselineV2Settings {
                mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
                control_interval_seconds: 0.06,
            }),
            &[],
        )
        .unwrap();
        let metadata = pending.startup_metadata_json().unwrap();
        let managed = TestDirectory::create("qualified-sse2-restore");
        let descriptor = pending
            .publish_checkpoint(
                managed.path(),
                CheckpointOperationId::parse("112233445566778899aabbccddeeff00").unwrap(),
            )
            .unwrap();
        let original = std::fs::read(managed.path().join(&descriptor.relative_filename)).unwrap();
        // Production supports recorded SSE2; an explicit qualification request remains exact.
        let ordinary =
            prepare_stage6a_p0_checkpoint_restore(managed.path(), &descriptor, TEST_MEMORY_CEILING)
                .unwrap();
        assert_eq!(ordinary.startup_metadata_json().unwrap(), metadata);
        assert!(prepare_sensor_qualification_checkpoint_restore(
            managed.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
            SensorProfile::Basic,
            8,
            InferenceMathBackend::Scalar,
            limits
        )
        .is_err());
        assert!(prepare_sensor_qualification_checkpoint_restore(
            managed.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
            SensorProfile::Motion,
            8,
            InferenceMathBackend::Sse2,
            limits
        )
        .is_err());
        let missing = prepare_sensor_qualification_checkpoint_restore(
            std::path::Path::new("unused-no-ray-admission"),
            &descriptor,
            TEST_MEMORY_CEILING,
            SensorProfile::Basic,
            8,
            InferenceMathBackend::Sse2,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .err()
        .unwrap();
        assert!(missing.to_string().contains("explicit ray limits"));
        let mut restored = prepare_sensor_qualification_checkpoint_restore(
            managed.path(),
            &descriptor,
            TEST_MEMORY_CEILING,
            SensorProfile::Basic,
            8,
            InferenceMathBackend::Sse2,
            limits,
        )
        .unwrap();
        assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
        restored.configure_calculation_workers(3).unwrap();
        let mut expected_frame = None;
        for transition in [&mut pending, &mut restored] {
            transition.acknowledge_persistence(&descriptor).unwrap();
            transition.publish_running_authority().unwrap();
            let mut frame = Vec::new();
            assert_eq!(
                transition
                    .pack_initial_frame_v1(&mut frame)
                    .unwrap()
                    .alive_snakes,
                14
            );
            transition
                .publish_first_scheduled_frame_v1(&mut frame)
                .unwrap();
            assert_eq!(transition.completed_step(), 1);
            if let Some(expected) = &expected_frame {
                assert_eq!(&frame, expected);
            } else {
                expected_frame = Some(frame);
            }
        }
        assert_eq!(
            std::fs::read(managed.path().join(&descriptor.relative_filename)).unwrap(),
            original
        );
    }

    #[test]
    fn startup_metadata_preserves_the_unpublished_run_boundary() {
        let transition = prepare_stage6a_p0_fresh_run(request(42)).unwrap();
        let encoded = transition.startup_metadata_json().unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(metadata["seed"], 42);
        assert_eq!(metadata["serializerVersion"], 1);
        assert_eq!(metadata["sensorVersion"], 3);
        assert_eq!(metadata["parameterCount"], STAGE6A_P0_PARAMETERS_PER_GENOME);
        assert!(metadata["maximumFrameBytes"].as_u64().unwrap() > 0);
        assert!(metadata["population"].is_null());
        assert!(metadata["world"].is_null());
        assert_eq!(transition.startup_metadata_json().unwrap(), encoded);
        assert_eq!(transition.completed_step(), 0);
        assert!(!transition.checkpoint_published());
        assert!(!transition.authority_published());
        assert_eq!(transition.snake_count(), 0);
    }

    #[test]
    fn maximum_ambient_target_preserves_complete_death_effects_and_next_step() {
        let pending = activate_pending_run_start(
            prepare_stage6a_p0_fresh_run_with_settings(
                request(42),
                &[
                    FreshRunSettingUpdate {
                        path: "snakeCount".into(),
                        value: 1.0,
                    },
                    FreshRunSettingUpdate {
                        path: "baselineBots.count".into(),
                        value: 0.0,
                    },
                    FreshRunSettingUpdate {
                        path: "pelletCountTarget".into(),
                        value: 25_000.0,
                    },
                ],
            )
            .unwrap(),
            "maximum-ambient-death",
            "112233445566778899aabbccddeeff00",
        );
        let mut frame = Vec::new();
        let initial = pending.pack_initial_frame_v1(&mut frame).unwrap();
        assert_eq!(initial.pellets, 25_000);
        assert_eq!(initial.alive_snakes, 1);
        let snake_id = f32::from_le_bytes(frame[28..32].try_into().unwrap()) as u32;
        let mut running = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        let token = running
            .select_entity(
                crate::engine::state::EntitySelectionTarget::Frame(snake_id),
                &request(42).run_id,
                1,
            )
            .unwrap()
            .unwrap()
            .token;
        let (death, effective_step) = running
            .apply_god_mode_kill(token, snake_id)
            .expect("the supported ambient target must leave space for the full normal corpse");
        assert!(death.pellets_dropped > 0);
        assert_eq!(effective_step, 1);
        let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
        let after_death = running.pack_display_into(1, &mut frame).unwrap();
        assert_eq!(after_death.frame.alive_snakes, 0);
        assert_eq!(after_death.frame.pellets, 25_000 + death.pellets_dropped);
        assert!(matches!(
            running.service_after_command_drain(17, SchedulerServiceMode::Background, None).unwrap(),
            RunningAuthorityLoopProgress::Published { publication, .. } if publication.completed_step == 1
        ));
    }

    #[test]
    fn fresh_replacement_applies_reset_only_and_live_settings_before_hashing() {
        let prepared = prepare_stage6a_p0_boundary(
            request(43),
            &[
                FreshRunSettingUpdate {
                    path: "worldRadius".to_owned(),
                    value: 4_200.0,
                },
                FreshRunSettingUpdate {
                    path: "generationSeconds".to_owned(),
                    value: 90.0,
                },
                FreshRunSettingUpdate {
                    path: "simSpeed".to_owned(),
                    value: 3.0,
                },
                FreshRunSettingUpdate {
                    path: "foodSpawn.edgeFalloffEnabled".to_owned(),
                    value: 0.0,
                },
                FreshRunSettingUpdate {
                    path: "snakeCount".to_owned(),
                    value: 54.0,
                },
                FreshRunSettingUpdate {
                    path: "baselineBots.count".to_owned(),
                    value: 2.0,
                },
            ],
            typescript_default_graph_spec(),
        )
        .expect("compatible fixed-graph settings must build one private boundary");
        assert_eq!(prepared.candidate.config.world_radius, 4_200.0);
        assert_eq!(prepared.candidate.config.requested_sim_speed, 3.0);
        assert_eq!(
            fresh_float(&prepared.candidate.config, "generationSeconds").unwrap(),
            90.0
        );
        assert!(matches!(
            prepared
                .candidate
                .config
                .settings
                .iter()
                .find(|setting| setting.path == "foodSpawn.edgeFalloffEnabled")
                .map(|setting| &setting.value),
            Some(NormalizedSettingValue::Bool(false))
        ));
        assert_eq!(
            prepared.candidate.identity.config_hash,
            normalized_config_hash(&prepared.candidate.config).unwrap()
        );
        assert_eq!(prepared.candidate.config.population_count, 54);
        assert_eq!(prepared.candidate.population.len(), 54);
        assert_eq!(prepared.candidate.brains.len(), 54);
        assert_eq!(prepared.candidate.config.baseline_count, 2);
        assert_eq!(prepared.candidate.rng.baselines.len(), 2);
    }

    #[test]
    fn fresh_replacement_compiles_and_initializes_a_supplied_graph() {
        use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
        let graph = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 51 },
                },
                GraphNodeSpec {
                    id: "head".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 51,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![GraphEdge {
                from: "input".to_owned(),
                to: "head".to_owned(),
                from_port: None,
                to_port: None,
            }],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        };
        let bins = [FreshRunSettingUpdate {
            path: "sense.bubbleBins".to_owned(),
            value: 8.0,
        }];
        let transition =
            prepare_stage6a_p0_fresh_run_with_settings_and_graph(request(45), &bins, graph)
                .expect("a bounded sensor-compatible graph must build a private boundary");
        let metadata: serde_json::Value =
            serde_json::from_str(&transition.startup_metadata_json().unwrap()).unwrap();
        assert_eq!(metadata["parameterCount"], 104);
        assert_eq!(metadata["graphSpec"]["nodes"][1]["type"], "Dense");

        let mismatch = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 50 },
                },
                GraphNodeSpec {
                    id: "head".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 50,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![GraphEdge {
                from: "input".to_owned(),
                to: "head".to_owned(),
                from_port: None,
                to_port: None,
            }],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        };
        let error = match prepare_stage6a_p0_fresh_run_with_settings_and_graph(
            request(46),
            &bins,
            mismatch,
        ) {
            Ok(_) => panic!("a graph with the wrong sensor width must reject before publication"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("does not match sensor width"));
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
        let prepared = prepare_stage6a_p0_boundary(
            request(FIXTURE_SEED),
            &[],
            typescript_default_graph_spec(),
        )
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
        let prepared = prepare_stage6a_p0_boundary(
            Stage6aP0FreshRunRequest {
                run_id: opaque_run_id.to_owned(),
                ..request(7)
            },
            &[],
            typescript_default_graph_spec(),
        )
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
        let handoff = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .expect_err("undurable authority must not move into the retained loop");
        assert!(matches!(
            handoff.error(),
            RunStartTransitionError::AuthorityNotPublished
        ));
        let pending = handoff.into_transition();
        assert_eq!(pending.generation(), 1);
        assert_eq!(pending.completed_step(), 0);
        assert!(!pending.checkpoint_published());
        assert!(!pending.persistence_acknowledged());
        assert!(!pending.authority_published());
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
        assert!(matches!(
            pending.publish_first_scheduled_frame_v1(&mut frame),
            Err(RunStartTransitionError::AuthorityNotPublished)
        ));
        assert!(!pending.first_scheduled_step_attempted());
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
        let initial_frame = frame.clone();
        let scheduled_metadata = pending
            .publish_first_scheduled_frame_v1(&mut frame)
            .expect("Rust must schedule, publish, and pack exactly one complete fixed-P0 step");
        assert_eq!(pending.completed_step(), 1);
        assert!(pending.first_scheduled_step_attempted());
        assert!(pending.first_scheduled_frame_published());
        assert_eq!(scheduled_metadata.generation, 1);
        assert_eq!(
            scheduled_metadata.total_snakes,
            STAGE6A_P0_POPULATION_COUNT + STAGE6A_P0_BASELINE_COUNT
        );
        assert_eq!(
            scheduled_metadata.alive_snakes,
            scheduled_metadata.total_snakes
        );
        assert_eq!(scheduled_metadata.pellets, 3_496);
        assert_eq!(scheduled_metadata.byte_length, frame.len());
        assert_ne!(
            frame, initial_frame,
            "one complete step must change display state"
        );
        assert!(matches!(
            pending.publish_first_scheduled_frame_v1(&mut frame),
            Err(RunStartTransitionError::FirstScheduledStepAlreadyAttempted)
        ));
        assert_eq!(pending.completed_step(), 1);
        assert!(matches!(
            pending.publish_running_authority(),
            Err(RunStartTransitionError::AuthorityAlreadyPublished)
        ));
        let handoff = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .expect_err("the one-shot scheduler must exclude a second loop owner");
        assert!(matches!(
            handoff.error(),
            RunStartTransitionError::ExperimentalStepAlreadyOwnsAuthority
        ));
        let pending = handoff.into_transition();
        assert_eq!(pending.completed_step(), 1);
        assert!(pending.first_scheduled_frame_published());
    }

    #[test]
    fn restored_generation_activates_without_resetting_its_chronology() {
        let managed = TestDirectory::create("restored-generation");
        let mut prepared =
            prepare_stage6a_p0_boundary(request(42), &[], typescript_default_graph_spec()).unwrap();
        prepared.candidate.phase =
            AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation);
        prepared.candidate.generation.generation = 2;
        prepared.candidate.generation.completed_step = 60;
        prepared.candidate.generation.population_epoch = 2;
        for genome in &mut prepared.candidate.population {
            genome.brain.epoch = 2;
        }
        for brain in &mut prepared.candidate.brains {
            brain.handle.epoch = 2;
        }
        let state = AuthoritativeState::validate_and_own(
            prepared.candidate,
            prepared.graph,
            &prepared.admission_policy,
        )
        .unwrap();
        let descriptor = crate::engine::checkpoint::publish_checkpoint(
            managed.path(),
            CheckpointOperationId::parse("41414141414141414141414141414141").unwrap(),
            state.world_epoch(),
            state.checkpoint_boundary().unwrap(),
            &prepared.checkpoint_limits,
            &prepared.graph_limits,
            &prepared.admission_policy,
        )
        .unwrap();
        let mut restored = PendingRunStartTransition::restore_committed(
            managed.path(),
            &descriptor,
            prepared.admission_policy,
            prepared.checkpoint_limits,
            prepared.graph_limits,
            prepared.work_limits,
        )
        .unwrap();
        assert_eq!(restored.generation(), 2);
        assert_eq!(restored.completed_step(), 60);
        assert_eq!(restored.snake_count(), 0);
        assert!(!restored.authority_published());
        assert_ne!(restored.transition_epoch(), state.world_epoch());
        let publication = restored.publish_running_authority().unwrap();
        assert_eq!(publication.generation, 2);
        assert_eq!(publication.completed_step, 60);
        assert_eq!(publication.population_epoch, 2);
        assert!(restored.publish_running_authority().is_err());
        let mut running = restored
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .unwrap();
        let progress = running
            .service_after_command_drain(17, SchedulerServiceMode::Background, None)
            .unwrap();
        assert!(matches!(
            progress,
            RunningAuthorityLoopProgress::Published { .. }
        ));
        assert_eq!(running.completed_step(), 61);
    }

    #[test]
    fn recovery_rebinding_preserves_every_state_field_and_moves_population_storage() {
        let prepared =
            prepare_stage6a_p0_boundary(request(42), &[], typescript_default_graph_spec()).unwrap();
        let mut expected = prepared.candidate.clone();
        let population = prepared.candidate.population.as_ptr();
        let state = AuthoritativeState::validate_and_own(
            prepared.candidate,
            prepared.graph,
            &prepared.admission_policy,
        )
        .unwrap();
        let branch = state
            .into_recovery_branch("different-lineage".into(), &prepared.admission_policy)
            .unwrap();
        expected.identity.run_id = "different-lineage".into();
        assert_eq!(branch.state(), &expected);
        assert_eq!(branch.state().population.as_ptr(), population);
    }

    #[test]
    fn recovery_branch_changes_only_lineage_before_activation() {
        let managed = TestDirectory::create("recovery-lineage");
        let mut source = prepare_stage6a_p0_fresh_run(request(42)).unwrap();
        let descriptor = source
            .publish_checkpoint(
                managed.path(),
                CheckpointOperationId::parse("42424242424242424242424242424242").unwrap(),
            )
            .unwrap();
        let restore = || {
            prepare_stage6a_p0_compatible_checkpoint_restore(
                managed.path(),
                &descriptor,
                request(42).memory_ceiling_bytes,
            )
            .unwrap()
        };
        let restored = restore();
        let before_rejection = restored.startup_metadata_json().unwrap();
        let mut uncommitted = restore();
        assert!(matches!(
            uncommitted.publish_running_authority(),
            Err(RunStartTransitionError::CompatibleRecoveryBranchNotCommitted)
        ));
        let mut wrong = descriptor.clone();
        wrong.generation_hex = "0000000000000002".into();
        assert!(restored
            .validate_recovery_source(&wrong, "recovery-branch")
            .is_err());
        assert!(restored.validate_recovery_source(&descriptor, "").is_err());
        assert_eq!(restored.startup_metadata_json().unwrap(), before_rejection);
        restored
            .validate_recovery_source(&descriptor, "recovery-branch")
            .unwrap();
        let mut original_metadata: serde_json::Value =
            serde_json::from_str(&restored.startup_metadata_json().unwrap()).unwrap();
        let mut branch = restored
            .into_committed_recovery_branch("recovery-branch".into())
            .unwrap();
        let branch_metadata: serde_json::Value =
            serde_json::from_str(&branch.startup_metadata_json().unwrap()).unwrap();
        original_metadata["runId"] = serde_json::json!("recovery-branch");
        assert_eq!(branch_metadata, original_metadata);
        assert_eq!(branch.generation(), source.generation());
        assert_eq!(branch.completed_step(), source.completed_step());
        assert!(!branch.authority_published());
        assert_eq!(branch.snake_count(), 0);
        branch.publish_running_authority().unwrap();
        assert!(branch
            .validate_recovery_source(&descriptor, "another")
            .is_err());
        assert!(branch
            .into_committed_recovery_branch("another".into())
            .is_err());
        assert!(restore()
            .into_committed_recovery_branch(descriptor.run_id.clone())
            .is_err());
        assert!(restore()
            .into_committed_recovery_branch(String::new())
            .is_err());
    }

    #[test]
    fn retained_loop_services_one_backlogged_step_per_boundary_and_faults_once() {
        let pending = activate_pending_run_start(
            prepare_stage6a_p0_fresh_run(request(0x1020_3040))
                .expect("complete fresh run must enter durability"),
            "retained-loop",
            "102030405060708090a0b0c0d0e0f000",
        );
        let invalid_handoff = pending
            .into_running_loop(
                FixedStepSchedulerPolicy {
                    algorithm_version: u32::MAX,
                    ..FixedStepSchedulerPolicy::provisional_defaults()
                },
                0,
            )
            .expect_err("invalid scheduler policy must preserve the prior authority owner");
        assert!(matches!(
            invalid_handoff.error(),
            RunStartTransitionError::RunningLoop(error)
                if matches!(error.as_ref(), RunningAuthorityLoopError::Scheduler(_))
        ));
        let pending = invalid_handoff.into_transition();
        assert!(pending.authority_published());
        assert_eq!(pending.completed_step(), 0);
        let mut running = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .expect("activated step-zero authority must enter one retained loop");
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        let wall_seconds_until_step = match running
            .service_after_command_drain(0, SchedulerServiceMode::Background, None)
            .expect("origin service must be idle")
        {
            RunningAuthorityLoopProgress::Idle {
                simulation_seconds_until_step,
                wall_seconds_until_step,
            } => {
                assert!(simulation_seconds_until_step > 0.0);
                wall_seconds_until_step
            }
            other => panic!("unexpected origin progress: {other:?}"),
        };
        let one_step_wall_ms = (wall_seconds_until_step * 1_000.0).ceil() as u64;
        assert!(one_step_wall_ms > 0);
        let backlog_wall_ms = one_step_wall_ms * 8;
        let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
        let frame_allocation = frame.as_ptr();

        let first_due = match running
            .service_after_command_drain(
                backlog_wall_ms,
                SchedulerServiceMode::Background,
                Some(RunningFramePublication::new(
                    FrameV1ViewDescriptor::default(),
                    &mut frame,
                )),
            )
            .expect("one backlogged step must publish")
        {
            RunningAuthorityLoopProgress::Published {
                ticket_sequence,
                due_steps,
                publication,
                frame: Some(metadata),
            } => {
                assert_eq!(ticket_sequence, 1);
                assert_eq!(publication.completed_step, 1);
                assert_eq!(metadata.byte_length, frame.len());
                due_steps
            }
            other => panic!("unexpected first publication: {other:?}"),
        };
        assert!(first_due > 1, "the test must create real retained backlog");
        assert_eq!(frame.as_ptr(), frame_allocation);
        let first_frame = frame.clone();

        for expected_step in 2..=3 {
            let frame_request = (expected_step == 3).then(|| {
                RunningFramePublication::new(FrameV1ViewDescriptor::default(), &mut frame)
            });
            match running
                .service_after_command_drain(
                    backlog_wall_ms,
                    SchedulerServiceMode::Background,
                    frame_request,
                )
                .expect("each new service boundary may publish only one retained-debt step")
            {
                RunningAuthorityLoopProgress::Published {
                    ticket_sequence,
                    publication,
                    frame: metadata,
                    ..
                } => {
                    assert_eq!(ticket_sequence, expected_step);
                    assert_eq!(publication.completed_step, expected_step);
                    assert_eq!(metadata.is_some(), expected_step == 3);
                }
                other => panic!("unexpected retained-debt publication: {other:?}"),
            }
            assert_eq!(running.completed_step(), expected_step);
        }
        assert_eq!(frame.as_ptr(), frame_allocation);
        assert_ne!(frame, first_frame);
        let diagnostics = running.scheduler_diagnostics();
        assert_eq!(diagnostics.completed_steps, 3);
        assert_eq!(diagnostics.command_service_boundaries, 4);

        let error = running
            .service_after_command_drain(
                backlog_wall_ms - 1,
                SchedulerServiceMode::Background,
                None,
            )
            .expect_err("regressing the Rust clock must terminally fault the loop");
        assert!(matches!(error, RunningAuthorityLoopError::Scheduler(_)));
        assert_eq!(running.state(), RunningAuthorityLoopState::Faulted);
        assert_eq!(running.completed_step(), 3);
        assert!(matches!(
            running.service_after_command_drain(
                backlog_wall_ms + one_step_wall_ms,
                SchedulerServiceMode::Background,
                None,
            ),
            Err(RunningAuthorityLoopError::AlreadyFaulted)
        ));
        assert_eq!(running.completed_step(), 3);
        assert_eq!(
            running.scheduler_diagnostics().command_service_boundaries,
            4
        );
    }

    #[test]
    fn retained_loop_post_publication_frame_failure_faults_without_retry() {
        let pending = activate_pending_run_start(
            prepare_stage6a_p0_fresh_run(request(0x2030_4050))
                .expect("complete fresh run must enter durability"),
            "retained-frame-fault",
            "2030405060708090a0b0c0d0e0f00010",
        );
        let mut running = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .expect("activated step-zero authority must enter one retained loop");
        let wall_seconds_until_step = match running
            .service_after_command_drain(0, SchedulerServiceMode::Background, None)
            .expect("origin service must be idle")
        {
            RunningAuthorityLoopProgress::Idle {
                wall_seconds_until_step,
                ..
            } => wall_seconds_until_step,
            other => panic!("unexpected origin progress: {other:?}"),
        };
        let due_wall_ms = (wall_seconds_until_step * 1_000.0).ceil() as u64;
        let mut frame = vec![0x6b; 19];
        let frame_before = frame.clone();
        let error = running
            .service_after_command_drain(
                due_wall_ms,
                SchedulerServiceMode::Background,
                Some(RunningFramePublication::new(
                    FrameV1ViewDescriptor {
                        camera_x: f64::NAN,
                        ..FrameV1ViewDescriptor::default()
                    },
                    &mut frame,
                )),
            )
            .expect_err("invalid presentation metadata must fail after step publication");
        assert!(matches!(error, RunningAuthorityLoopError::Frame(_)));
        assert_eq!(frame, frame_before);
        assert_eq!(running.completed_step(), 1);
        assert_eq!(running.scheduler_diagnostics().completed_steps, 1);
        assert_eq!(running.state(), RunningAuthorityLoopState::Faulted);
        assert!(matches!(
            running.service_after_command_drain(
                due_wall_ms * 2,
                SchedulerServiceMode::Background,
                None,
            ),
            Err(RunningAuthorityLoopError::AlreadyFaulted)
        ));
        assert_eq!(running.completed_step(), 1);
        assert_eq!(running.scheduler_diagnostics().completed_steps, 1);
    }

    #[test]
    fn retained_loop_holds_one_terminal_candidate_without_duplicate_evolution() {
        let pending = activate_pending_run_start(
            short_generation_pending(0x5060_7080),
            "retained-generation",
            "5060708090a0b0c0d0e0f00010203040",
        );
        let mut running = pending
            .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
            .expect("activated short generation must enter one retained loop");
        assert!(matches!(
            running
                .service_after_command_drain(0, SchedulerServiceMode::Background, None)
                .expect("origin service must be idle"),
            RunningAuthorityLoopProgress::Idle { .. }
        ));

        let terminal = (1_u64..=8)
            .find_map(|wall_second| {
                let progress = running
                    .service_after_command_drain(
                        wall_second * 1_000,
                        SchedulerServiceMode::Background,
                        None,
                    )
                    .expect("short generation service must either publish or block");
                match progress {
                    RunningAuthorityLoopProgress::Published { publication, .. } => {
                        assert_eq!(publication.completed_step, wall_second);
                        None
                    }
                    RunningAuthorityLoopProgress::GenerationTransitionPending { .. } => {
                        Some(progress)
                    }
                    other => panic!("unexpected short-generation progress: {other:?}"),
                }
            })
            .expect("valid eight-second generation must reach one terminal boundary");
        let (ticket_sequence, source_key, reason, successor_generation, successor_completed_step) =
            match terminal {
                RunningAuthorityLoopProgress::GenerationTransitionPending {
                    ticket_sequence,
                    source_key,
                    reason,
                    successor_generation,
                    successor_completed_step,
                } => (
                    ticket_sequence,
                    source_key,
                    reason,
                    successor_generation,
                    successor_completed_step,
                ),
                _ => unreachable!("find_map returns only terminal progress"),
            };
        assert_eq!(ticket_sequence, 8);
        assert_eq!(source_key.source_completed_step(), 7);
        assert_eq!(successor_generation, 2);
        assert_eq!(successor_completed_step, 8);
        assert_eq!(running.completed_step(), 7);
        assert_eq!(
            running.state(),
            RunningAuthorityLoopState::GenerationTransitionPending
        );

        let (candidate_address, commit_address) = {
            let retained = running
                .pending_generation_transition()
                .expect("terminal transition must remain borrowable");
            assert_eq!(retained.source_key(), source_key);
            assert_eq!(retained.reason(), reason);
            (
                std::ptr::from_ref(retained.candidate()).addr(),
                std::ptr::from_ref(retained.commit_record()).addr(),
            )
        };
        let diagnostics_before_retry = running.scheduler_diagnostics();
        assert!(diagnostics_before_retry.step_pending);
        let mut untouched_frame = vec![0x5a; 17];
        let repeated = running
            .service_after_command_drain(
                9_000,
                SchedulerServiceMode::Background,
                Some(RunningFramePublication::new(
                    FrameV1ViewDescriptor::default(),
                    &mut untouched_frame,
                )),
            )
            .expect("blocked service must reborrow the same terminal transition");
        assert_eq!(repeated, terminal);
        assert_eq!(untouched_frame, vec![0x5a; 17]);
        assert_eq!(running.completed_step(), 7);
        assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);
        let retained = running
            .pending_generation_transition()
            .expect("repeated service must retain one terminal candidate");
        assert_eq!(
            std::ptr::from_ref(retained.candidate()).addr(),
            candidate_address
        );
        assert_eq!(
            std::ptr::from_ref(retained.commit_record()).addr(),
            commit_address
        );

        let source_world_epoch = running.world_epoch();
        assert!(matches!(
            running.publish_acknowledged_generation_start(10_000, None),
            Err(RunningAuthorityLoopError::RunningStep(error))
                if matches!(*error, RunningStepError::GenerationPersistenceNotAcknowledged)
        ));
        assert_eq!(running.world_epoch(), source_world_epoch);
        assert_eq!(running.completed_step(), 7);
        assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);

        let managed = TestDirectory::create("retained-generation-checkpoint");
        let operation = CheckpointOperationId::parse("60708090a0b0c0d0e0f0001020304050")
            .expect("test operation ID must be canonical");
        let publication = running
            .publish_pending_generation_checkpoint(managed.path(), operation.clone())
            .expect("retained transition must publish one immutable checkpoint");
        let retry = running
            .publish_pending_generation_checkpoint(managed.path(), operation)
            .expect("same operation must return the exact prior publication");
        assert_eq!(retry, publication);
        assert_eq!(running.world_epoch(), source_world_epoch);
        assert_eq!(running.completed_step(), 7);
        assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);
        assert_eq!(
            std::ptr::from_ref(
                running
                    .pending_generation_transition()
                    .expect("checkpoint retry must retain the same transition")
                    .candidate()
            )
            .addr(),
            candidate_address
        );
        let stale_before_assignment = running
            .submit_external_delivery_results(
                &[ExternalDeliveryResult {
                    step_key: source_key,
                    event_sequence: 1,
                    connection_id: 1,
                    lease_id: 1,
                    accepted: true,
                }],
                None,
            )
            .expect("stale result before reassignment staging must be ignored");
        assert_eq!(stale_before_assignment.ignored_results, 1);
        assert_eq!(
            stale_before_assignment.state,
            RunningAuthorityDeliveryState::Idle
        );
        assert_eq!(running.world_epoch(), source_world_epoch);
        assert_eq!(running.scheduler_diagnostics(), diagnostics_before_retry);

        let mut mismatched = publication.descriptor.clone();
        mismatched.logical_root_sha256 = "0".repeat(64);
        mismatched.relative_filename = format!("{}.checkpoint-v3", mismatched.logical_root_sha256);
        assert!(matches!(
            running.acknowledge_pending_generation_persistence(&mismatched),
            Err(RunningAuthorityLoopError::RunningStep(error))
                if matches!(
                    *error,
                    RunningStepError::GenerationPersistenceAcknowledgementMismatch { .. }
                )
        ));
        assert_eq!(running.world_epoch(), source_world_epoch);
        assert!(!running
            .pending_generation_transition()
            .expect("mismatched acknowledgement must retain the transition")
            .persistence_acknowledged());

        running
            .acknowledge_pending_generation_persistence(&publication.descriptor)
            .expect("exact committed descriptor must be retained");
        assert!(running
            .pending_generation_transition()
            .expect("acknowledged transition must remain pending")
            .persistence_acknowledged());
        assert_eq!(running.world_epoch(), source_world_epoch);
        assert!(matches!(
            running.publish_acknowledged_generation_start(10_000, None),
            Err(RunningAuthorityLoopError::RunningStep(error))
                if matches!(
                    *error,
                    RunningStepError::PendingDeliveryStateMismatch {
                        field: "generation delivery context"
                    }
                )
        ));
        assert_eq!(running.world_epoch(), source_world_epoch);

        assert!(matches!(
            running
                .prepare_acknowledged_generation_reassignments()
                .expect("controller-free successor must prepare deterministically"),
            GenerationReassignmentProgress::Ready(_)
        ));
        let generation_start = running
            .publish_acknowledged_generation_start(10_000, None)
            .expect("durable assignment-ready successor must publish exactly once");
        assert_eq!(generation_start.ticket_sequence, ticket_sequence);
        assert_eq!(generation_start.publication.source_key, source_key);
        assert_eq!(generation_start.publication.generation, 2);
        assert_eq!(generation_start.publication.completed_step, 8);
        assert_eq!(running.generation(), 2);
        assert_eq!(running.completed_step(), 8);
        assert_ne!(running.world_epoch(), source_world_epoch);
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        let rebound_diagnostics = running.scheduler_diagnostics();
        assert_eq!(rebound_diagnostics.completed_steps, 8);
        assert!(!rebound_diagnostics.step_pending);
        assert!(matches!(
            running.publish_acknowledged_generation_start(10_000, None),
            Err(RunningAuthorityLoopError::InvalidActionState { .. })
        ));
        assert_eq!(running.generation(), 2);
        assert_eq!(running.completed_step(), 8);

        assert!(matches!(
            running
                .service_after_command_drain(10_000, SchedulerServiceMode::Background, None,)
                .expect("resume origin must exclude persistence wait"),
            RunningAuthorityLoopProgress::Idle { .. }
        ));
        let next = running
            .service_after_command_drain(11_000, SchedulerServiceMode::Background, None)
            .expect("rebound coordinator and scheduler must publish the next generation step");
        assert!(matches!(
            next,
            RunningAuthorityLoopProgress::Published { publication, .. }
                if publication.completed_step == 9
        ));
        assert_eq!(running.generation(), 2);
        assert_eq!(running.completed_step(), 9);
        assert_eq!(running.scheduler_diagnostics().completed_steps, 9);
    }

    #[test]
    fn background_runtime_owns_steps_waits_and_restores_one_loop() {
        let running = activate_pending_run_start(
            prepare_stage6a_p0_fresh_run(request(0x6070_8090))
                .expect("complete fresh run must enter durability"),
            "background-running",
            "60708090a0b0c0d0e0f0001020304050",
        )
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect("durable step-zero authority must enter one unserviced loop");
        let expected_memory = running.authoritative_memory_bytes();

        let mut invalid_init = running_runtime_init();
        invalid_init.inbound.max_batches = 0;
        let failed =
            EngineRuntime::new_running_authority(invalid_init, running, Arc::new(NoopWakeSink))
                .expect_err("invalid queue limits must return the exact loop");
        assert_eq!(
            failed.error().code,
            crate::engine::error::EngineErrorCode::InvalidConfiguration
        );
        let running = failed.into_running_loop();
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        assert_eq!(running.completed_step(), 0);

        let runtime = EngineRuntime::new_running_authority(
            running_runtime_init(),
            running,
            Arc::new(NoopWakeSink),
        )
        .expect("valid runtime must retain the exact loop before thread start");
        assert!(runtime.owns_authoritative_state());
        assert_eq!(
            runtime.authoritative_state_memory_bytes(),
            Some(expected_memory)
        );
        assert!(runtime.running_authority_retained_for_test());
        runtime.start().expect("one background thread must start");

        wait_for_runtime(&runtime, || {
            runtime
                .health()
                .running_authority
                .is_some_and(|health| health.wait_calls > 0)
        });
        runtime
            .try_submit(runtime_probe(1, 7))
            .expect("coarse probe must wake the real authority thread");
        wait_for_runtime(&runtime, || {
            let health = runtime.health();
            health.processed_commands == 1
                && health
                    .running_authority
                    .is_some_and(|running| running.completed_step >= 3)
        });

        runtime.request_stop();
        runtime
            .join()
            .expect("orderly real-authority stop must join");
        let health = runtime.health();
        assert_eq!(health.lifecycle, LifecycleState::Stopped);
        assert!(health.fault.is_none());
        assert_eq!(health.processed_batches, 1);
        assert_eq!(health.processed_commands, 1);
        let running_health = health
            .running_authority
            .expect("real runtime must retain bounded authority health");
        assert_eq!(running_health.loop_state, RunningAuthorityLoopState::Ready);
        assert!(running_health.completed_step >= 3);
        assert_eq!(
            running_health.scheduler_completed_steps,
            running_health.completed_step
        );
        assert!(running_health.command_service_boundaries > running_health.completed_step);
        assert!(running_health.wait_calls > 0);
        assert!(running_health.timeout_wakes > 0);
        assert!(runtime.running_authority_retained_for_test());

        let events = runtime
            .drain_outputs(usize::MAX, usize::MAX)
            .expect("bounded output drain must succeed")
            .events;
        assert!(events
            .iter()
            .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Started))));
        assert!(events.iter().any(|event| matches!(
            event,
            CompletedEvent::Reliable(ReliableEvent::ProbeResult {
                sequence: 1,
                correlation_id: 101,
                payload,
            }) if payload == &[7]
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
    }

    #[test]
    fn background_runtime_catches_panic_and_restores_faulted_owner_once() {
        let running = activate_pending_run_start(
            prepare_stage6a_p0_fresh_run(request(0x7080_90a0))
                .expect("complete fresh run must enter durability"),
            "background-panic",
            "708090a0b0c0d0e0f000102030405060",
        )
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect("durable step-zero authority must enter one unserviced loop");
        let runtime = EngineRuntime::new_running_authority(
            running_runtime_init(),
            running,
            Arc::new(NoopWakeSink),
        )
        .expect("valid runtime must retain one loop");
        runtime.start().expect("one background thread must start");
        wait_for_runtime(&runtime, || {
            runtime
                .health()
                .running_authority
                .is_some_and(|health| health.wait_calls > 0)
        });
        runtime
            .try_submit(CommandBatch {
                contract_version: ENGINE_CONTRACT_VERSION,
                commands: vec![SequencedCommand {
                    sequence: 1,
                    command: EngineCommand::PanicForTest,
                }]
                .into_boxed_slice(),
            })
            .expect("test panic command must enter the coarse queue");
        wait_for_runtime(&runtime, || {
            runtime.health().lifecycle == LifecycleState::Faulted
        });
        let completed_before_join = runtime
            .health()
            .running_authority
            .expect("fault retains bounded running health")
            .completed_step;
        runtime
            .join()
            .expect("caught background panic must still join cleanly");
        let health = runtime.health();
        assert_eq!(health.lifecycle, LifecycleState::Stopped);
        assert!(health.fault.is_some());
        assert_eq!(
            health
                .running_authority
                .expect("joined fault retains bounded running health")
                .completed_step,
            completed_before_join
        );
        assert!(runtime.running_authority_retained_for_test());
        let events = runtime
            .drain_outputs(usize::MAX, usize::MAX)
            .expect("fault output drain must succeed")
            .events;
        assert!(events
            .iter()
            .any(|event| matches!(event, CompletedEvent::Fault(_))));
        assert!(!events
            .iter()
            .any(|event| matches!(event, CompletedEvent::Reliable(ReliableEvent::Stopped))));
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
