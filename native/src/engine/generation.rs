//! Durable pre-spawn generation-boundary preparation.
//!
//! A completed world is reduced to fitness and evolution output, but neither
//! the old authority nor the next boundary becomes authoritative here. Packed
//! next-generation weights move once into a new [`StateCandidate`], recurrent
//! state is reset, and the caller retains the prior authority until checkpoint
//! file publication and the small metadata/current-pointer transaction succeed.

use super::checkpoint::{
    publish_checkpoint, publish_hall_of_fame_weights, CheckpointDescriptor, CheckpointError,
    CheckpointLimits, CheckpointOperationId, HallOfFameWeightsDescriptor,
};
use super::evolution::{
    prepare_evolution, prepare_evolution_with_assessments, EvolutionError,
    EvolvedFitnessAssessment, GenerationSummary, HallOfFameCandidate, NextGenomeOrigin,
};
use super::graph::{CompiledGraph, GraphLimits};
use super::physics::PhysicsStepKey;
use super::rng::{derive_seed, hash_seed, StatefulRng};
use super::state::{
    AdmittedGenerationSuccessor, AllocatorState, AuthoritativeState, AuthorityPhase,
    BaselineRngState, BrainHandle, BrainOwner, BrainRuntimeState, FixedStepContinuationState,
    GenerationBoundaryKind, GenerationStartPreflight, GenerationStartPublication,
    GenerationStartReplacement, GenerationState, GenomeLineage, PopulationGenome, RngStateBundle,
    StateCandidate, StateError, WorldState,
};
use super::step_config::{
    project_baseline_generation_config, project_evolution_config, BaselineGenerationConfig,
    StepConfigError,
};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

/// First pre-spawn generation-boundary construction contract.
pub const GENERATION_BOUNDARY_PREPARATION_VERSION: u32 = 1;

/// Small owned metadata retained beside a prepared boundary for its SQLite handoff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedGenerationMetadata {
    /// Compact eight-field result for the completed generation.
    pub summary: GenerationSummary,
    /// Run-scoped best candidate for the completed generation.
    pub hall_of_fame: HallOfFameCandidate,
    /// New-population slot containing the selected genome bit-exactly as an elite.
    pub hall_of_fame_population_slot: u32,
}

/// Exact integer/bit representation of the compact completed-generation row.
///
/// Float64 values are converted to their IEEE-754 bits before any foreign
/// boundary so JavaScript cannot reinterpret authoritative generation results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationSummaryCommitRecord {
    /// Generation whose round completed.
    pub completed_generation: u64,
    /// Maximum fitness as exact finite Float64 bits.
    pub best_f64_bits: u64,
    /// Arithmetic mean fitness as exact finite Float64 bits.
    pub average_f64_bits: u64,
    /// Minimum fitness as exact finite Float64 bits.
    pub minimum_f64_bits: u64,
    /// Greedy RMS-threshold species count.
    pub species_count: u64,
    /// Largest greedy species bucket.
    pub top_species_size: u64,
    /// Mean absolute parameter value as exact finite Float64 bits.
    pub average_weight_f64_bits: u64,
    /// Variance of absolute parameter values as exact finite Float64 bits.
    pub weight_variance_f64_bits: u64,
}

/// Exact Hall-of-Fame reference into the admitted successor checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HallOfFameCommitRecord {
    /// Completed generation that produced the selected genome.
    pub completed_generation: u64,
    /// Stable population slot in the completed source generation.
    pub source_population_slot: u64,
    /// Stable source snake identity used by the completed result.
    pub source_snake_id: u64,
    /// Selected fitness as exact finite Float64 bits.
    pub fitness_f64_bits: u64,
    /// Selected points as exact finite Float64 bits.
    pub points_f64_bits: u64,
    /// Selected terminal body-point count.
    pub length: u64,
    /// Admitted successor slot containing the bit-exact elite.
    pub successor_population_slot: u64,
    /// Genome identity read from that exact admitted successor slot.
    pub successor_genome_id: u64,
}

/// Versioned completed-generation chronology; absent on legacy fixture handoffs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationCompletionRecord {
    pub reason: super::running_step::GenerationTransitionReason,
    pub actual_elapsed_f64_bits: u64,
    pub configured_duration_f64_bits: u64,
    pub completed_step: u64,
}

/// Complete bounded generation metadata constructed solely by Rust admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationCommitRecord {
    /// Fixed history-v1 / assessment-v2 extension, absent on frozen legacy handoffs.
    pub experiment: Option<super::experiment_records::GenerationExperimentRecords>,
    /// Completion contract recorded with the ordinary durable handoff.
    pub completion: Option<GenerationCompletionRecord>,
    /// Compact eight-field generation result.
    pub summary: GenerationSummaryCommitRecord,
    /// Run-scoped reference to the elite already stored in the checkpoint.
    pub hall_of_fame: HallOfFameCommitRecord,
}

/// Complete next boundary staged while the preceding authority remains current.
#[derive(Debug)]
pub struct PreparedGenerationBoundary {
    candidate: StateCandidate,
    metadata: PreparedGenerationMetadata,
    /// Serial evolution duration; diagnostic only and never persisted in the candidate.
    evolution_micros: u64,
    /// Disjoint evolution phases, excluded from candidate and checkpoint metadata.
    evolution_cost_micros: crate::engine::evolution::EvolutionCostMicros,
}

impl PreparedGenerationBoundary {
    /// Read the measured serial evolution cost before consuming this candidate.
    pub(crate) const fn evolution_micros(&self) -> u64 {
        self.evolution_micros
    }

    /// Read nested diagnostic phases before consuming this boundary.
    pub(crate) const fn evolution_cost_micros(
        &self,
    ) -> crate::engine::evolution::EvolutionCostMicros {
        self.evolution_cost_micros
    }

    /// Inspect the exact pre-spawn candidate before ownership/checkpoint admission.
    #[must_use]
    pub const fn candidate(&self) -> &StateCandidate {
        &self.candidate
    }

    /// Inspect the compact history and Hall-of-Fame handoff.
    #[must_use]
    pub const fn metadata(&self) -> PreparedGenerationMetadata {
        self.metadata
    }

    /// Move the population-sized candidate without copying packed weights.
    #[must_use]
    pub fn into_parts(self) -> (StateCandidate, PreparedGenerationMetadata) {
        (self.candidate, self.metadata)
    }
}

/// Fully admitted boundary held beside the still-current terminal source world.
#[derive(Debug)]
pub struct AdmittedGenerationBoundary {
    workload: Option<super::generation_work::GenerationWork>,
    successor: AdmittedGenerationSuccessor,
    metadata: PreparedGenerationMetadata,
    commit_record: GenerationCommitRecord,
}

impl AdmittedGenerationBoundary {
    /// Terminal fixed-step key that produced the staged boundary.
    #[must_use]
    pub const fn source_key(&self) -> PhysicsStepKey {
        self.successor.source_key()
    }

    /// Read the exact next-generation candidate without making it authoritative.
    #[must_use]
    pub fn candidate(&self) -> &StateCandidate {
        self.successor.authority().state()
    }

    /// Read the compact history and Hall-of-Fame handoff.
    #[must_use]
    pub const fn metadata(&self) -> PreparedGenerationMetadata {
        self.metadata
    }

    /// Read the exact scalar record authorized by Rust generation admission.
    #[must_use]
    pub const fn commit_record(&self) -> &GenerationCommitRecord {
        &self.commit_record
    }

    /// Attach terminal chronology before exposing the admitted checkpoint handoff.
    pub(crate) const fn workload(&self) -> Option<&super::generation_work::GenerationWork> {
        self.workload.as_ref()
    }

    pub(crate) fn record_workload(&mut self, record: super::generation_work::GenerationWork) {
        self.workload = Some(record);
    }

    pub(crate) fn record_completion(
        &mut self,
        reason: super::running_step::GenerationTransitionReason,
        actual_elapsed: f64,
        configured_duration: f64,
    ) {
        self.commit_record.completion = Some(GenerationCompletionRecord {
            reason,
            actual_elapsed_f64_bits: actual_elapsed.to_bits(),
            configured_duration_f64_bits: configured_duration.to_bits(),
            completed_step: self.candidate().generation.completed_step,
        });
    }

    /// Attach bounded experiment records before exposing the durable handoff.
    pub(crate) fn record_experiment(
        &mut self,
        world: &WorldState,
        lives: &super::evolved_lives::EvolvedGenerationState,
        policy: super::evolved_lives::EvolvedExperimentConfig,
    ) -> Result<(), GenerationTransitionError> {
        if policy.version == 0 {
            return Ok(());
        }
        let completion = self.commit_record.completion.ok_or(
            GenerationTransitionError::InvalidContinuation {
                field: "experiment completion",
            },
        )?;
        let cumulative_gross = match self
            .candidate()
            .config
            .settings
            .iter()
            .find(|setting| setting.path == "lifeScoring.cumulativeBasis")
            .map(|setting| &setting.value)
        {
            Some(super::state::NormalizedSettingValue::Integer(0)) => false,
            Some(super::state::NormalizedSettingValue::Integer(1)) => true,
            _ => {
                return Err(GenerationTransitionError::InvalidContinuation {
                    field: "experiment cumulative basis",
                })
            }
        };
        let records = super::experiment_records::GenerationExperimentRecords::build(
            super::experiment_records::ExperimentRecordInputs {
                generation: self.commit_record.summary.completed_generation,
                config_revision: self.candidate().identity.config_revision,
                completion,
                world,
                lives,
                policy,
                cumulative_gross,
                evolution: project_evolution_config(
                    &self.candidate().config,
                    self.successor.authority().graph().total_parameters,
                )?,
                winner_slot: self.metadata.hall_of_fame.source_slot,
            },
        )
        .map_err(GenerationTransitionError::Experiment)?;
        if records.hall_of_fame[3] != self.commit_record.hall_of_fame.source_snake_id {
            return Err(GenerationTransitionError::InvalidContinuation {
                field: "experiment physical winner",
            });
        }
        // Old index fields keep their physical meanings. The version-two
        // extension explicitly supplies selected score/bundle/display length.
        self.commit_record.hall_of_fame.points_f64_bits = records.hall_of_fame[20];
        self.commit_record.hall_of_fame.length = records.hall_of_fame[24];
        self.commit_record.experiment = Some(records);
        Ok(())
    }

    /// Conservative current-plus-successor state memory charged during staging.
    #[must_use]
    pub const fn combined_state_bytes(&self) -> usize {
        self.successor.combined_state_bytes()
    }

    /// Complete process ceiling available again after the prior authority is consumed.
    #[must_use]
    pub const fn full_memory_ceiling_bytes(&self) -> usize {
        self.successor.full_memory_ceiling_bytes()
    }

    /// Fully validate the proposed running successor and restore its durable
    /// checkpoint boundary before reliable assignments are exposed.
    pub(crate) fn preflight_running_start(
        &mut self,
        current: &AuthoritativeState,
        replacement: &mut GenerationStartReplacement<'_>,
        unavailable_controller_reservations: &[
            super::external_replacement::UnavailableControllerReservation
        ],
    ) -> Result<GenerationStartPreflight, GenerationTransitionError> {
        Ok(self.successor.preflight_running_start(
            current,
            replacement,
            unavailable_controller_reservations,
        )?)
    }

    /// Publish the exact resolved running successor in one final authority swap.
    pub(crate) fn publish_running_start(
        &mut self,
        current: &mut AuthoritativeState,
        preflight: GenerationStartPreflight,
        resolved: super::running_step::ResolvedGenerationStartReplacement<'_>,
        unavailable_controller_reservations: Vec<
            super::external_replacement::UnavailableControllerReservation,
        >,
    ) -> Result<GenerationStartPublication, GenerationTransitionError> {
        Ok(self.successor.publish_running_start(
            current,
            preflight,
            resolved,
            unavailable_controller_reservations,
        )?)
    }

    /// Publish the immutable managed checkpoint while the source remains current.
    ///
    /// A stale source key fails before filesystem work. The reduced admission
    /// policy keeps the complete old authority charged while the checkpoint
    /// writer uses its bounded workspace. Successful file publication still
    /// does not authorize a SQLite pointer update or authority replacement.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_managed_checkpoint(
        &self,
        current_source: &AuthoritativeState,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
        limits: &CheckpointLimits,
        graph_limits: &GraphLimits,
    ) -> Result<CheckpointDescriptor, GenerationTransitionError> {
        current_source.validate_running_step_key(self.source_key())?;
        let boundary = self.successor.authority().checkpoint_boundary()?;
        Ok(publish_checkpoint(
            managed_directory,
            operation_id,
            self.source_key().operation_epoch(),
            boundary,
            limits,
            graph_limits,
            self.successor.checkpoint_policy(),
        )?)
    }

    /// Publish the selected elite as a deduplicated object independent of checkpoint retention.
    pub fn publish_hall_of_fame_weights(
        &self,
        managed_directory: &Path,
        operation_id: &CheckpointOperationId,
        limits: &CheckpointLimits,
    ) -> Result<HallOfFameWeightsDescriptor, GenerationTransitionError> {
        let slot = usize::try_from(self.metadata.hall_of_fame_population_slot).map_err(|_| {
            GenerationTransitionError::ArithmeticOverflow {
                context: "Hall-of-Fame successor population slot",
            }
        })?;
        let genome = self.candidate().population.get(slot).ok_or(
            GenerationTransitionError::PopulationShape {
                reason: "Hall-of-Fame successor population slot is out of bounds",
            },
        )?;
        Ok(publish_hall_of_fame_weights(
            managed_directory,
            operation_id,
            &genome.weights,
            limits,
        )?)
    }
}

/// Admit one prepared successor against the exact current terminal attempt.
pub(crate) fn admit_prepared_generation_boundary(
    current_source: &AuthoritativeState,
    source_key: PhysicsStepKey,
    prepared: PreparedGenerationBoundary,
) -> Result<AdmittedGenerationBoundary, GenerationTransitionError> {
    admit_generation_boundary_internal(current_source, source_key, prepared, None, 0)
}

/// Admit newborn terminal identities only with the sealed replacement witness.
#[allow(clippy::too_many_arguments)]
pub(crate) fn admit_prepared_generation_boundary_with_evolved(
    current_source: &AuthoritativeState,
    source_key: PhysicsStepKey,
    prepared: PreparedGenerationBoundary,
    completed_world: &WorldState,
    completed_rng: &RngStateBundle,
    completed_allocators: &AllocatorState,
    proof: Option<&super::evolved_replacement::EvolvedReplacementAuthorityProof>,
    retained_bytes: usize,
) -> Result<AdmittedGenerationBoundary, GenerationTransitionError> {
    if let Some(proof) = proof {
        if !proof.matches(
            source_key,
            current_source.state(),
            completed_world,
            completed_rng,
            completed_allocators,
        ) {
            return Err(GenerationTransitionError::InvalidContinuation {
                field: "evolved terminal replacement witness",
            });
        }
    }
    if proof.is_none() && retained_bytes == 0 {
        return admit_prepared_generation_boundary(current_source, source_key, prepared);
    }
    admit_generation_boundary_internal(current_source, source_key, prepared, proof, retained_bytes)
}

fn admit_generation_boundary_internal(
    current_source: &AuthoritativeState,
    source_key: PhysicsStepKey,
    prepared: PreparedGenerationBoundary,
    evolved: Option<&super::evolved_replacement::EvolvedReplacementAuthorityProof>,
    retained_bytes: usize,
) -> Result<AdmittedGenerationBoundary, GenerationTransitionError> {
    let (candidate, metadata) = prepared.into_parts();
    let successor =
        current_source.admit_generation_successor(source_key, candidate, retained_bytes)?;
    let commit_record = generation_commit_record(
        current_source.state(),
        successor.authority().state(),
        metadata,
        evolved,
    )?;
    Ok(AdmittedGenerationBoundary {
        workload: None,
        successor,
        metadata,
        commit_record,
    })
}

/// Bind compact generation metadata to the exact admitted elite identity.
fn generation_commit_record(
    source: &StateCandidate,
    successor: &StateCandidate,
    metadata: PreparedGenerationMetadata,
    evolved: Option<&super::evolved_replacement::EvolvedReplacementAuthorityProof>,
) -> Result<GenerationCommitRecord, GenerationTransitionError> {
    let summary = metadata.summary;
    let hall_of_fame = metadata.hall_of_fame;
    if source.phase != AuthorityPhase::Running
        || summary.generation != source.generation.generation
        || summary.generation != hall_of_fame.generation
    {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit does not identify the running source generation",
        });
    }
    if summary.best.to_bits() != hall_of_fame.fitness.to_bits() {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "generation best and Hall-of-Fame fitness differ",
        });
    }
    if [
        summary.best,
        summary.average,
        summary.minimum,
        summary.average_weight,
        summary.weight_variance,
        hall_of_fame.fitness,
        hall_of_fame.points,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit contains a non-finite Float64 value",
        });
    }
    let expected_successor_generation =
        summary
            .generation
            .checked_add(1)
            .ok_or(GenerationTransitionError::ArithmeticOverflow {
                context: "generation commit successor generation",
            })?;
    let expected_completed_step = source.generation.completed_step.checked_add(1).ok_or(
        GenerationTransitionError::ArithmeticOverflow {
            context: "generation commit completed step",
        },
    )?;
    if successor.generation.generation != expected_successor_generation
        || successor.generation.completed_step != expected_completed_step
        || successor.phase != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation)
    {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit does not describe the admitted successor boundary",
        });
    }

    let source_index = usize::try_from(hall_of_fame.source_slot).map_err(|_| {
        GenerationTransitionError::ArithmeticOverflow {
            context: "generation commit source population slot",
        }
    })?;
    let source_genome =
        source
            .population
            .get(source_index)
            .ok_or(GenerationTransitionError::PopulationShape {
                reason: "generation commit source population slot is out of bounds",
            })?;
    if source_genome.slot != hall_of_fame.source_slot
        || !(source.world.snakes.iter().any(|snake| {
            snake.id == hall_of_fame.snake_id
                && snake.population_slot == Some(hall_of_fame.source_slot)
        }) || evolved
            .and_then(|proof| proof.record(hall_of_fame.source_slot))
            .is_some_and(|record| {
                record.new_id == hall_of_fame.snake_id && record.brain == source_genome.brain
            }))
    {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit source identity is not present in the terminal authority",
        });
    }

    let successor_index = usize::try_from(metadata.hall_of_fame_population_slot).map_err(|_| {
        GenerationTransitionError::ArithmeticOverflow {
            context: "generation commit successor population slot",
        }
    })?;
    let successor_genome = successor.population.get(successor_index).ok_or(
        GenerationTransitionError::PopulationShape {
            reason: "generation commit successor population slot is out of bounds",
        },
    )?;
    if successor_genome.slot != metadata.hall_of_fame_population_slot
        || !f32_slices_equal_bits(&successor_genome.weights, &source_genome.weights)
        || successor_genome.lineage.parent_a != Some(source_genome.lineage.genome_id)
        || successor_genome.lineage.parent_b.is_some()
        || successor_genome.lineage.birth_generation != successor.generation.generation
        || successor_genome.lineage.genome_id == 0
    {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit successor does not identify the admitted Hall-of-Fame elite",
        });
    }

    let length = u64::try_from(hall_of_fame.length).map_err(|_| {
        GenerationTransitionError::ArithmeticOverflow {
            context: "generation commit Hall-of-Fame length",
        }
    })?;
    Ok(GenerationCommitRecord {
        completion: None,
        experiment: None,
        summary: GenerationSummaryCommitRecord {
            completed_generation: summary.generation,
            best_f64_bits: summary.best.to_bits(),
            average_f64_bits: summary.average.to_bits(),
            minimum_f64_bits: summary.minimum.to_bits(),
            species_count: summary.species_count,
            top_species_size: summary.top_species_size,
            average_weight_f64_bits: summary.average_weight.to_bits(),
            weight_variance_f64_bits: summary.weight_variance.to_bits(),
        },
        hall_of_fame: HallOfFameCommitRecord {
            completed_generation: hall_of_fame.generation,
            source_population_slot: u64::from(hall_of_fame.source_slot),
            source_snake_id: hall_of_fame.snake_id,
            fitness_f64_bits: hall_of_fame.fitness.to_bits(),
            points_f64_bits: hall_of_fame.points.to_bits(),
            length,
            successor_population_slot: u64::from(metadata.hall_of_fame_population_slot),
            successor_genome_id: successor_genome.lineage.genome_id,
        },
    })
}

/// Compare packed genome parameters by their exact stored Float32 bits.
fn f32_slices_equal_bits(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left_value, right_value)| left_value.to_bits() == right_value.to_bits())
}

/// Generation projection, staged-world, allocation, or evolution failure.
#[derive(Debug)]
pub enum GenerationTransitionError {
    /// Invalid bounded experiment summary or economic identity.
    Experiment(String),
    /// Required admitted settings were missing, mistyped, or inconsistent.
    Config(Box<StepConfigError>),
    /// Fitness, selection, crossover, mutation, or RNG preparation failed.
    Evolution(Box<EvolutionError>),
    /// Deterministic identity allocation failed.
    State(Box<StateError>),
    /// Managed checkpoint construction or immutable publication failed.
    Checkpoint(Box<CheckpointError>),
    /// The source is not one running authority eligible to end a generation.
    InvalidSource { reason: &'static str },
    /// Post-step RNG or allocator continuation is stale or contaminated.
    InvalidContinuation { field: &'static str },
    /// Checked generation/slot/identity arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// A required bounded vector could not reserve its complete size.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// Prepared parent, slot, or Hall-of-Fame metadata was inconsistent.
    PopulationShape { reason: &'static str },
}

impl Display for GenerationTransitionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Experiment(detail) => write!(formatter, "generation experiment: {detail}"),
            Self::Config(error) => write!(formatter, "invalid generation configuration: {error}"),
            Self::Evolution(error) => write!(formatter, "generation evolution failed: {error}"),
            Self::State(error) => {
                write!(formatter, "generation state transition failed: {error}")
            }
            Self::Checkpoint(error) => write!(formatter, "generation checkpoint failed: {error}"),
            Self::InvalidSource { reason } => {
                write!(formatter, "invalid generation source: {reason}")
            }
            Self::InvalidContinuation { field } => {
                write!(formatter, "invalid completed-step continuation: {field}")
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "generation arithmetic overflow while calculating {context}"
                )
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "unable to reserve {required} entries for generation {context}"
            ),
            Self::PopulationShape { reason } => {
                write!(
                    formatter,
                    "invalid prepared generation population: {reason}"
                )
            }
        }
    }
}

impl Error for GenerationTransitionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(error) => Some(error.as_ref()),
            Self::Evolution(error) => Some(error.as_ref()),
            Self::State(error) => Some(error.as_ref()),
            Self::Checkpoint(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<StepConfigError> for GenerationTransitionError {
    fn from(error: StepConfigError) -> Self {
        Self::Config(Box::new(error))
    }
}

impl From<EvolutionError> for GenerationTransitionError {
    fn from(error: EvolutionError) -> Self {
        Self::Evolution(Box::new(error))
    }
}

impl From<StateError> for GenerationTransitionError {
    fn from(error: StateError) -> Self {
        Self::State(Box::new(error))
    }
}

impl From<CheckpointError> for GenerationTransitionError {
    fn from(error: CheckpointError) -> Self {
        Self::Checkpoint(Box::new(error))
    }
}

/// Build one next-generation checkpoint candidate without publishing authority.
///
/// `completed_world`, `completed_rng`, and `completed_allocators` must be the
/// complete post-physics result for exactly one step after `source`. The
/// terminal elapsed value is checked against the fixed delta. The private
/// running-step coordinator will later supply these values through its keyed
/// staged transaction; this standalone preparation deliberately performs no
/// checkpoint I/O or authority swap.
pub fn prepare_generation_boundary(
    source: &StateCandidate,
    completed_world: &WorldState,
    completed_rng: &RngStateBundle,
    completed_allocators: &AllocatorState,
    completed_elapsed_seconds: f64,
    graph: &CompiledGraph,
) -> Result<PreparedGenerationBoundary, GenerationTransitionError> {
    prepare_generation_boundary_internal(
        source,
        completed_world,
        completed_rng,
        completed_allocators,
        completed_elapsed_seconds,
        graph,
        None,
    )
}

/// Reduce committed life assessments to evolution at the ordinary terminal boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_generation_boundary_with_evolved(
    source: &StateCandidate,
    completed_world: &WorldState,
    completed_rng: &RngStateBundle,
    completed_allocators: &AllocatorState,
    completed_elapsed_seconds: f64,
    graph: &CompiledGraph,
    lives: &super::evolved_lives::EvolvedGenerationState,
    policy: super::evolved_lives::EvolvedExperimentConfig,
) -> Result<PreparedGenerationBoundary, GenerationTransitionError> {
    let use_assessments = policy.uses_life_assessments();
    let mut assessments = reserve_vec(
        if use_assessments {
            lives.slots.len()
        } else {
            0
        },
        "evolved terminal assessments",
    )?;
    if use_assessments {
        lives
            .validate(completed_world, source.population.len())
            .map_err(|_| GenerationTransitionError::InvalidContinuation {
                field: "evolved terminal assessments",
            })?;
        for slot in &lives.slots {
            assessments.push(EvolvedFitnessAssessment {
                assessment: slot.assessment(policy.points, policy.nonpoints),
                deaths: slot.deaths,
                successful_respawns: slot.successful_respawns,
            });
        }
    }
    prepare_generation_boundary_internal(
        source,
        completed_world,
        completed_rng,
        completed_allocators,
        completed_elapsed_seconds,
        graph,
        use_assessments.then_some((&assessments, policy.objective)),
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_generation_boundary_internal(
    source: &StateCandidate,
    completed_world: &WorldState,
    completed_rng: &RngStateBundle,
    completed_allocators: &AllocatorState,
    completed_elapsed_seconds: f64,
    graph: &CompiledGraph,
    evolved: Option<(
        &[EvolvedFitnessAssessment],
        super::evolution::EvolvedFitnessObjective,
    )>,
) -> Result<PreparedGenerationBoundary, GenerationTransitionError> {
    if source.phase != AuthorityPhase::Running {
        return Err(GenerationTransitionError::InvalidSource {
            reason: "generation transition requires running authority",
        });
    }
    let expected_elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;
    if !expected_elapsed.is_finite()
        || !completed_elapsed_seconds.is_finite()
        || completed_elapsed_seconds.to_bits() != expected_elapsed.to_bits()
    {
        return Err(GenerationTransitionError::InvalidContinuation {
            field: "generation elapsed seconds",
        });
    }
    validate_rng_continuation(&source.rng, completed_rng)?;
    validate_allocator_continuation(&source.allocators, completed_allocators)?;

    let evolution_config = project_evolution_config(&source.config, graph.total_parameters)?;
    let baseline_config = project_baseline_generation_config(&source.config)?;
    let evolution_started = std::time::Instant::now();
    let prepared = if let Some((assessments, objective)) = evolved {
        prepare_evolution_with_assessments(
            completed_world,
            &source.population,
            graph,
            &completed_rng.evolution,
            source.generation.generation,
            source.generation.best_fitness_ever,
            evolution_config,
            assessments,
            objective,
        )?
    } else {
        prepare_evolution(
            completed_world,
            &source.population,
            graph,
            &completed_rng.evolution,
            source.generation.generation,
            source.generation.best_fitness_ever,
            evolution_config,
        )?
    };
    let evolution_micros =
        u64::try_from(evolution_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let evolution_cost_micros = prepared.cost_micros();
    let parts = prepared.into_transition_parts();

    let next_generation = source.generation.generation.checked_add(1).ok_or(
        GenerationTransitionError::ArithmeticOverflow {
            context: "next generation",
        },
    )?;
    let completed_step = source.generation.completed_step.checked_add(1).ok_or(
        GenerationTransitionError::ArithmeticOverflow {
            context: "completed step",
        },
    )?;
    let next_epoch = source.generation.population_epoch.checked_add(1).ok_or(
        GenerationTransitionError::ArithmeticOverflow {
            context: "population epoch",
        },
    )?;
    let count = parts.next_population.len();
    if count != source.population.len() || count == 0 {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "prepared population count changed",
        });
    }
    let hall_of_fame = parts.hall_of_fame;
    let hall_of_fame_index = usize::try_from(hall_of_fame.source_slot).map_err(|_| {
        GenerationTransitionError::ArithmeticOverflow {
            context: "Hall-of-Fame source slot",
        }
    })?;
    let hall_of_fame_source = parts.source_population.get(hall_of_fame_index).ok_or(
        GenerationTransitionError::PopulationShape {
            reason: "Hall-of-Fame source slot is out of bounds",
        },
    )?;
    let Some(best_elite) = parts.next_population.first() else {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "prepared population omitted its best elite",
        });
    };
    if best_elite.slot != 0
        || best_elite.origin
            != (NextGenomeOrigin::Elite {
                parent_slot: hall_of_fame.source_slot,
            })
        || best_elite.weights.as_slice() != hall_of_fame_source.weights.as_ref()
    {
        return Err(GenerationTransitionError::PopulationShape {
            reason: "prepared best elite does not preserve the Hall-of-Fame genome",
        });
    }
    let count_u64 =
        u64::try_from(count).map_err(|_| GenerationTransitionError::ArithmeticOverflow {
            context: "population identity count",
        })?;

    let mut allocators = completed_allocators.clone();
    let brain_ids = allocators.reserve_brain_ids(count_u64)?.ok_or(
        GenerationTransitionError::PopulationShape {
            reason: "nonempty population received no brain identity range",
        },
    )?;
    let genome_ids = allocators.reserve_genome_ids(count_u64)?.ok_or(
        GenerationTransitionError::PopulationShape {
            reason: "nonempty population received no genome identity range",
        },
    )?;

    let mut population = reserve_vec(count, "population records")?;
    let mut brains = reserve_vec(count, "population brain records")?;
    for next in parts.next_population {
        let slot = usize::try_from(next.slot).map_err(|_| {
            GenerationTransitionError::ArithmeticOverflow {
                context: "next population slot",
            }
        })?;
        if slot != population.len() {
            return Err(GenerationTransitionError::PopulationShape {
                reason: "prepared slots are not dense and canonical",
            });
        }
        let offset =
            u64::try_from(slot).map_err(|_| GenerationTransitionError::ArithmeticOverflow {
                context: "population identity offset",
            })?;
        let brain_id = brain_ids.first.checked_add(offset).ok_or(
            GenerationTransitionError::ArithmeticOverflow {
                context: "brain identity",
            },
        )?;
        let genome_id = genome_ids.first.checked_add(offset).ok_or(
            GenerationTransitionError::ArithmeticOverflow {
                context: "genome identity",
            },
        )?;
        let (parent_a, parent_b) = parent_lineage(parts.source_population, next.origin)?;
        let handle = BrainHandle {
            id: brain_id,
            epoch: next_epoch,
        };
        population.push(PopulationGenome {
            slot: next.slot,
            brain: handle,
            lineage: GenomeLineage {
                genome_id,
                birth_generation: next_generation,
                parent_a: Some(parent_a),
                parent_b,
            },
            fitness: next.fitness,
            weights: next.weights.into_boxed_slice(),
        });
        brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::PopulationSlot(next.slot),
            retained_origin: None,
            non_population_weights: None,
            recurrent: zeroed_box(graph.total_state_size, "zero recurrent state")?,
        });
    }

    let (baseline_rngs, baseline_behaviors) = derive_baseline_streams_for_generation(
        source.identity.seed,
        next_generation,
        source.config.baseline_count,
        baseline_config,
        completed_rng.version,
    )?;
    let metadata = PreparedGenerationMetadata {
        summary: parts.summary,
        hall_of_fame,
        hall_of_fame_population_slot: 0,
    };
    let candidate = StateCandidate {
        versions: source.versions.clone(),
        identity: source.identity.clone(),
        config: source.config.clone(),
        phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation),
        generation: GenerationState {
            boundary_version: source.generation.boundary_version,
            generation: next_generation,
            completed_step,
            population_epoch: next_epoch,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: parts.next_best_fitness_ever,
        },
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: RngStateBundle {
            baseline_behaviors,
            evolved_respawns: if completed_rng.version
                >= super::state::RNG_BUNDLE_WITH_EVOLVED_VERSION
            {
                super::state::EvolvedRespawnRngState::derive_generation(
                    source.identity.seed,
                    next_generation,
                    source.population.len(),
                )?
            } else {
                Vec::new()
            },
            version: completed_rng.version,
            world: completed_rng.world.clone(),
            evolution: parts.next_evolution_rng,
            external_controller: completed_rng.external_controller.clone(),
            baselines: baseline_rngs,
        },
        allocators,
        population,
        brains,
        world: WorldState::default(),
    };
    Ok(PreparedGenerationBoundary {
        candidate,
        metadata,
        evolution_micros,
        evolution_cost_micros,
    })
}

fn parent_lineage(
    source: &[PopulationGenome],
    origin: NextGenomeOrigin,
) -> Result<(u64, Option<u64>), GenerationTransitionError> {
    let lineage_id = |slot: u32| -> Result<u64, GenerationTransitionError> {
        source
            .get(slot as usize)
            .filter(|genome| genome.slot == slot)
            .map(|genome| genome.lineage.genome_id)
            .ok_or(GenerationTransitionError::PopulationShape {
                reason: "prepared parent slot is out of bounds",
            })
    };
    match origin {
        NextGenomeOrigin::Elite { parent_slot } => Ok((lineage_id(parent_slot)?, None)),
        NextGenomeOrigin::Child {
            parent_a_slot,
            parent_b_slot,
        } => Ok((lineage_id(parent_a_slot)?, Some(lineage_id(parent_b_slot)?))),
    }
}

pub(crate) fn derive_baseline_rngs(
    run_seed: u32,
    generation: u64,
    count: usize,
    config: BaselineGenerationConfig,
) -> Result<Vec<BaselineRngState>, GenerationTransitionError> {
    let mut baselines = reserve_vec(count, "baseline RNG states")?;
    let generation_seed = if config.randomize_seed_per_generation {
        hash_seed(&[f64::from(config.seed), f64::from(generation as u32)])
    } else {
        config.seed
    };
    for slot in 0..count {
        let slot_u32 =
            u32::try_from(slot).map_err(|_| GenerationTransitionError::ArithmeticOverflow {
                context: "baseline slot",
            })?;
        let labelled_seed = derive_seed(f64::from(run_seed), &format!("baseline:{slot_u32}"));
        let seed = hash_seed(&[f64::from(labelled_seed), f64::from(generation_seed)]);
        baselines.push(BaselineRngState {
            slot: slot_u32,
            state: StatefulRng::new(f64::from(seed)).export_state(),
        });
    }
    Ok(baselines)
}

/// Dispatch frozen combined streams or isolated v2 lifecycle/behavior.
fn derive_baseline_streams_for_generation(
    run_seed: u32,
    generation: u64,
    count: usize,
    config: BaselineGenerationConfig,
    version: u32,
) -> Result<
    (
        Vec<BaselineRngState>,
        Vec<super::rng::BaselineBehaviorRngState>,
    ),
    GenerationTransitionError,
> {
    if version == super::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION {
        let behavior = super::rng::derive_baseline_v2_behaviors(
            run_seed,
            config.seed,
            generation,
            count,
            config.randomize_seed_per_generation,
        )
        .map_err(|error| StateError::InvalidRng {
            stream: "baseline-behavior-v2".into(),
            reason: error.to_string(),
        })?;
        let mut lifecycle = reserve_vec(count, "v2 baseline lifecycle RNG states")?;
        for slot in 0..count {
            lifecycle.push(BaselineRngState {
                slot: slot as u32,
                state: super::rng::baseline_v2_lifecycle_rng(
                    run_seed,
                    config.seed,
                    generation,
                    slot as u32,
                    config.randomize_seed_per_generation,
                )
                .export_state(),
            });
        }
        Ok((lifecycle, behavior))
    } else if matches!(
        version,
        super::state::RNG_BUNDLE_VERSION | super::state::RNG_BUNDLE_WITH_EVOLVED_VERSION
    ) {
        Ok((
            derive_baseline_rngs(run_seed, generation, count, config)?,
            Vec::new(),
        ))
    } else {
        Err(GenerationTransitionError::InvalidContinuation {
            field: "RNG bundle version",
        })
    }
}

fn validate_rng_continuation(
    source: &RngStateBundle,
    completed: &RngStateBundle,
) -> Result<(), GenerationTransitionError> {
    if completed.version != source.version {
        return Err(GenerationTransitionError::InvalidContinuation {
            field: "RNG bundle version",
        });
    }
    if completed.evolution != source.evolution {
        return Err(GenerationTransitionError::InvalidContinuation {
            field: "evolution RNG advanced during a world step",
        });
    }
    Ok(())
}

fn validate_allocator_continuation(
    source: &AllocatorState,
    completed: &AllocatorState,
) -> Result<(), GenerationTransitionError> {
    if completed.version != source.version {
        return Err(GenerationTransitionError::InvalidContinuation {
            field: "allocator version",
        });
    }
    if completed.next_genome_id != source.next_genome_id {
        return Err(GenerationTransitionError::InvalidContinuation {
            field: "genome allocator advanced during a world step",
        });
    }
    for (field, before, after) in [
        (
            "entity allocator regressed",
            source.next_entity_id,
            completed.next_entity_id,
        ),
        (
            "brain allocator regressed",
            source.next_brain_id,
            completed.next_brain_id,
        ),
        (
            "controller lease allocator regressed",
            source.next_controller_lease_id,
            completed.next_controller_lease_id,
        ),
        (
            "external allocator regressed",
            source.next_external_id,
            completed.next_external_id,
        ),
        (
            "baseline allocator regressed",
            source.next_baseline_id,
            completed.next_baseline_id,
        ),
        (
            "resurrected allocator regressed",
            source.next_resurrected_id,
            completed.next_resurrected_id,
        ),
    ] {
        if after < before {
            return Err(GenerationTransitionError::InvalidContinuation { field });
        }
    }
    if completed.next_frame_v1_id < source.next_frame_v1_id {
        return Err(GenerationTransitionError::InvalidContinuation {
            field: "frame-v1 allocator regressed",
        });
    }
    Ok(())
}

fn reserve_vec<T>(
    required: usize,
    context: &'static str,
) -> Result<Vec<T>, GenerationTransitionError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(required)
        .map_err(|_| GenerationTransitionError::AllocationFailed { context, required })?;
    Ok(values)
}

fn zeroed_box(
    required: usize,
    context: &'static str,
) -> Result<Box<[f32]>, GenerationTransitionError> {
    let mut values = reserve_vec(required, context)?;
    values.resize(required, 0.0);
    Ok(values.into_boxed_slice())
}

#[cfg(test)]
mod tests;
