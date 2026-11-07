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

mod configuration;
use configuration::*;
mod restore_policy;
use restore_policy::*;
#[allow(unused_imports)]
pub(crate) use restore_policy::{
    production_checkpoint_limits, stage6a_p0_archive_validation_contract, stage6a_p0_graph_limits,
};

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

/// One production initial boundary, using the qualified graph/profile and SSE2 path.
pub fn prepare_default_fresh_run(
    request: Stage6aP0FreshRunRequest,
) -> Result<PendingRunStartTransition, FreshRunError> {
    prepare_default_fresh_run_with_math(request, InferenceMathBackend::Sse2)
}

/// Select arithmetic explicitly while retaining the ordinary production graph/settings.
pub fn prepare_default_fresh_run_with_math(
    request: Stage6aP0FreshRunRequest,
    math: InferenceMathBackend,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let settings = selected_fresh_default_settings()?;
    let contract = super::sensor_contract::SensorContractIdentity::from_settings(&settings)
        .map_err(|error| FreshRunError::Settings(error.to_string()))?;
    prepare_configured_fresh_run_with_math(
        request,
        &numeric_fresh_settings(settings),
        super::graph::default_stack_graph_spec(contract.input_size()),
        math,
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
pub fn prepare_configured_fresh_run_with_math(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    math: InferenceMathBackend,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let prepared = prepare_configured_fresh_boundary(request, settings, graph, math)?;
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

/// Start a new lineage using an admitted checkpoint's unchanged F32 population.
/// The source world retires before construction; retained genomes and metadata
/// remain charged. Lives, recurrent state, fitness and RNG chronology do not cross.
pub(crate) fn prepare_population_checkpoint_fork(
    mut request: Stage6aP0FreshRunRequest,
    source: PendingRunStartTransition,
    math: InferenceMathBackend,
) -> Result<PendingRunStartTransition, FreshRunError> {
    let source = source.into_population_for_new_run()?;
    if request.run_id == source.state().identity.run_id {
        return Err(FreshRunError::InvalidRunId);
    }
    let full_ceiling = request.memory_ceiling_bytes;
    let memory = source.memory_estimate();
    let checked = |value: Option<usize>| {
        value.ok_or(FreshRunError::ArithmeticOverflow {
            context: "population fork retained source memory",
        })
    };
    // Two settings projections remain live while the fresh constructor runs.
    // The admitted graph/text estimates conservatively cover their cloned strings
    // and graph spec; population records have no additional heap-owned metadata.
    let metadata_bytes = checked(memory.text_bytes.checked_mul(2))?;
    let metadata_bytes = checked(metadata_bytes.checked_add(memory.graph_bytes))?;
    let settings_bytes = checked(source.state().config.settings.capacity().checked_mul(
        std::mem::size_of::<super::state::NormalizedSetting>()
            + std::mem::size_of::<FreshRunSettingUpdate>(),
    ))?;
    let metadata_bytes = checked(metadata_bytes.checked_add(settings_bytes))?;
    if checked(memory.total_bytes.checked_add(metadata_bytes))? > full_ceiling {
        return Err(FreshRunError::Settings(
            "population fork source exceeds memory admission".into(),
        ));
    }
    let population_bytes = checked(
        source
            .state()
            .population
            .capacity()
            .checked_mul(std::mem::size_of::<PopulationGenome>()),
    )?;
    let retained_bytes = checked(memory.weight_bytes.checked_add(population_bytes))?;
    let retained_bytes = checked(retained_bytes.checked_add(metadata_bytes))?;
    request.memory_ceiling_bytes = full_ceiling.checked_sub(retained_bytes).ok_or_else(|| {
        FreshRunError::Settings("population fork source exceeds memory admission".into())
    })?;
    request.seed = source.state().identity.seed;
    let expected_settings = source.state().config.settings.clone();
    let settings = numeric_fresh_settings(expected_settings.clone());
    let graph_spec = source.graph_spec().clone();
    let layout = source.graph().layout_digest_sha256;
    // Consuming the private authority releases world, recurrence, validation and
    // compiled graph owners. No second running-world budget is retained.
    let population = source.into_population_for_new_run();
    let mut prepared = prepare_configured_fresh_boundary(request, &settings, graph_spec, math)?;
    if prepared.candidate.config.settings != expected_settings
        || prepared.graph.layout_digest_sha256 != layout
        || prepared.candidate.population.len() != population.len()
        || prepared
            .candidate
            .population
            .iter()
            .zip(&population)
            .any(|(target, original)| target.weights.len() != original.weights.len())
    {
        return Err(FreshRunError::Settings(
            "population fork must preserve source settings, graph and population shape".into(),
        ));
    }
    for (target, original) in prepared.candidate.population.iter_mut().zip(population) {
        target.weights = original.weights;
    }
    drop(settings);
    drop(expected_settings);
    // Retained input metadata and temporary initialized weights have now retired.
    prepared.admission_policy.memory_ceiling_bytes = full_ceiling;
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

fn prepare_configured_fresh_boundary(
    request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    math: InferenceMathBackend,
) -> Result<PreparedStage6aP0Boundary, FreshRunError> {
    if !math.is_available() {
        return Err(FreshRunError::Settings(format!(
            "inference backend {} is unavailable on this CPU",
            math.label()
        )));
    }
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
    Ok(prepared)
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
    prepare_sensor_qualification_population(
        request, settings, graph, profile, bins, limits, math, baseline, trained, None,
    )
}

/// Qualification-only transfer of distinct canonical population members.
#[cfg(feature = "engine-test-hooks")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_sensor_qualification_population(
    mut request: Stage6aP0FreshRunRequest,
    settings: &[FreshRunSettingUpdate],
    graph: GraphSpec,
    profile: super::sensor_contract::SensorProfile,
    bins: usize,
    limits: RunningStepWorkLimits,
    math: InferenceMathBackend,
    baseline: Option<super::baseline_v2::BaselineV2Settings>,
    trained: &[super::brain_artifact::ValidatedBrainArtifact],
    evolved: Option<&super::evolved_cohort::EvolvedCohort>,
) -> Result<PendingRunStartTransition, FreshRunError> {
    if let Some(cohort) = evolved {
        let source_bytes = cohort
            .weights
            .iter()
            .chain(&cohort.recurrent)
            .try_fold(1024 * 1024usize, |sum, w| sum.checked_add(w.len() * 4))
            .ok_or_else(|| FreshRunError::Settings("cohort reservation overflow".into()))?;
        request.memory_ceiling_bytes = request
            .memory_ceiling_bytes
            .checked_sub(source_bytes)
            .ok_or_else(|| {
                FreshRunError::Settings("cohort exhausted construction admission".into())
            })?;
    }
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
        (profile != super::sensor_contract::SensorProfile::V3).then_some(identity.clone()),
        limits,
        baseline,
    )?;
    if !trained.is_empty() {
        install_qualified_population_artifacts(&mut prepared, trained)?;
    }
    if let Some(cohort) = evolved {
        let source_contract =
            super::sensor_contract::SensorContractIdentity::from_settings(&cohort.settings)
                .map_err(|e| FreshRunError::Settings(e.to_string()))?;
        if !trained.is_empty()
            || source_contract != identity
            || prepared.graph.layout_digest_sha256 != cohort.graph.layout_digest_sha256
            || prepared.candidate.population.len() != cohort.weights.len()
            || prepared
                .candidate
                .population
                .iter()
                .zip(&cohort.weights)
                .any(|(g, w)| g.weights.len() != w.len())
        {
            return Err(FreshRunError::Settings(
                "evolved population graph/sensors/count mismatch".into(),
            ));
        }
        for (genome, weights) in prepared
            .candidate
            .population
            .iter_mut()
            .zip(&cohort.weights)
        {
            genome.weights.copy_from_slice(weights);
        }
        // Fresh physical lives use the ordinary reset state rule. Controlled numerical
        // comparisons separately start from the source checkpoint recurrent states.
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
mod tests;
