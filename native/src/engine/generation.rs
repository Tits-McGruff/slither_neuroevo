//! Durable pre-spawn generation-boundary preparation.
//!
//! A completed world is reduced to fitness and evolution output, but neither
//! the old authority nor the next boundary becomes authoritative here. Packed
//! next-generation weights move once into a new [`StateCandidate`], recurrent
//! state is reset, and the caller retains the prior authority until checkpoint
//! file publication and the small metadata/current-pointer transaction succeed.

use super::checkpoint::{
    publish_checkpoint, CheckpointDescriptor, CheckpointError, CheckpointLimits,
    CheckpointOperationId,
};
use super::evolution::{
    prepare_evolution, EvolutionError, GenerationSummary, HallOfFameCandidate, NextGenomeOrigin,
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

/// Complete bounded generation metadata constructed solely by Rust admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationCommitRecord {
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
}

impl PreparedGenerationBoundary {
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
}

/// Admit one prepared successor against the exact current terminal attempt.
pub(crate) fn admit_prepared_generation_boundary(
    current_source: &AuthoritativeState,
    source_key: PhysicsStepKey,
    prepared: PreparedGenerationBoundary,
) -> Result<AdmittedGenerationBoundary, GenerationTransitionError> {
    let (candidate, metadata) = prepared.into_parts();
    let successor = current_source.admit_generation_successor(source_key, candidate)?;
    let commit_record = generation_commit_record(
        current_source.state(),
        successor.authority().state(),
        metadata,
    )?;
    Ok(AdmittedGenerationBoundary {
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
        || !source.world.snakes.iter().any(|snake| {
            snake.id == hall_of_fame.snake_id
                && snake.population_slot == Some(hall_of_fame.source_slot)
        })
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
    let prepared = prepare_evolution(
        completed_world,
        &source.population,
        graph,
        &completed_rng.evolution,
        source.generation.generation,
        source.generation.best_fitness_ever,
        evolution_config,
    )?;
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
            non_population_weights: None,
            recurrent: zeroed_box(graph.total_state_size, "zero recurrent state")?,
        });
    }

    let baseline_rngs = derive_baseline_rngs(
        source.identity.seed,
        next_generation,
        source.config.baseline_count,
        baseline_config,
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
mod tests {
    use super::*;
    use crate::engine::checkpoint::{
        restore_checkpoint, CheckpointBoundaryKind, CheckpointLimits, CheckpointOperationId,
    };
    use crate::engine::generation_start::{
        GenerationStartConfig, GenerationStartError, GenerationStartWorkspace,
    };
    use crate::engine::graph::{
        GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec, CANONICAL_GRAPH_LAYOUT_VERSION,
    };
    use crate::engine::rng::{derive_seed, SerializedRngState};
    use crate::engine::spawn::{SpawnDomain, SpawnError};
    use crate::engine::state::{
        estimate_state_memory, normalized_config_hash, normalized_settings_schema_hash,
        AuthoritativeState, BaselineStrategyState, BodyRange, ContractVersions,
        NormalizedEngineConfig, NormalizedSettingValue, RunIdentity, SnakeKind, SnakeState,
        StateAdmissionPolicy, WorldPoint, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START,
        CHECKPOINT_VERSION, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START, FRAME_V1_EXHAUSTED_ID,
        GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
        RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, SENSOR_VERSION, SERIALIZER_VERSION,
    };
    use crate::engine::step_config::{
        fixture_default_settings, project_running_step_config, RunningStepWorkLimits,
    };
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "slither-generation-boundary-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("generation test directory must be unique");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let temporary_root = std::env::temp_dir().canonicalize();
            let owned = self.path.canonicalize();
            if temporary_root
                .as_ref()
                .ok()
                .zip(owned.as_ref().ok())
                .is_some_and(|(root, path)| path.starts_with(root) && path != root)
            {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    fn graph_limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 8,
            max_edges: 8,
            max_graph_outputs: 2,
            max_identifier_bytes: 32,
            max_total_referenced_identifier_bytes: 256,
            max_tensor_width: 128,
            max_mlp_hidden_layers: 4,
            max_split_output_ports: 4,
            max_parameter_floats: 10_000,
            max_recurrent_state_floats: 128,
            max_canonical_layout_bytes: 16_384,
            max_architecture_key_bytes: 32_768,
        }
    }

    fn graph_bundle() -> Arc<GraphBundle> {
        let spec = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "gru".into(),
                    kind: GraphNodeKind::Gru {
                        input_size: 83,
                        hidden_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 2,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                GraphEdge {
                    from: "input".into(),
                    to: "gru".into(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "gru".into(),
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
        Arc::new(
            GraphBundle::compile(spec, &graph_limits())
                .expect("generation fixture graph must compile"),
        )
    }

    fn checkpoint_limits() -> CheckpointLimits {
        CheckpointLimits {
            max_archive_bytes: 64 * 1024 * 1024,
            max_manifest_bytes: 1024 * 1024,
            max_state_bytes: 1024 * 1024,
            max_graph_bytes: 1024 * 1024,
            max_population_index_bytes: 1024 * 1024,
            max_population_count: 100,
            max_setting_count: 256,
            max_baseline_rng_count: 128,
            max_string_bytes: 16 * 1024,
            max_total_string_bytes: 1024 * 1024,
            max_weight_floats: 1_000_000,
            max_recurrent_floats: 1_000_000,
            max_numeric_stored_bytes: 64 * 1024 * 1024,
            max_numeric_candidate_bytes: 64 * 1024 * 1024,
            max_total_decoded_bytes: 128 * 1024 * 1024,
        }
    }

    fn stream(seed: u32, label: &str) -> SerializedRngState {
        StatefulRng::new(f64::from(derive_seed(f64::from(seed), label))).export_state()
    }

    fn fixture(graph: &CompiledGraph) -> StateCandidate {
        const COUNT: usize = 4;
        const BASELINES: usize = 0;
        const EPOCH: u64 = 7;
        let settings = fixture_default_settings(COUNT, BASELINES);
        let settings_schema_sha256 =
            normalized_settings_schema_hash(&settings).expect("fixture schema must hash");
        let config = NormalizedEngineConfig {
            version: NORMALIZED_CONFIG_VERSION,
            settings,
            settings_schema_sha256,
            graph_architecture_key: graph.architecture_key.clone(),
            fixed_step_seconds: 1.0 / 60.0,
            requested_sim_speed: 1.0,
            world_radius: 3_500.0,
            population_count: COUNT,
            baseline_count: BASELINES,
            max_world_snakes: 32,
            max_non_population_brains: 16,
            max_body_points: 10_000,
            max_pellets: 10_000,
            spatial_index_bytes: 8 * 1024 * 1024,
            worker_scratch_bytes: 8 * 1024 * 1024,
            checkpoint_scratch_bytes: 8 * 1024 * 1024,
            controller_input_hold_ms: 500,
            controller_disconnect_grace_ms: 30_000,
        };
        let config_hash = normalized_config_hash(&config).expect("fixture config must hash");
        let population = (0..COUNT)
            .map(|slot| PopulationGenome {
                slot: slot as u32,
                brain: BrainHandle {
                    id: slot as u64 + 1,
                    epoch: EPOCH,
                },
                lineage: GenomeLineage {
                    genome_id: slot as u64 + 10,
                    birth_generation: 7,
                    parent_a: None,
                    parent_b: None,
                },
                fitness: 0.0,
                weights: (0..graph.total_parameters)
                    .map(|index| ((slot * 29 + index * 7) % 101) as f32 / 100.0 - 0.5)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
            .collect::<Vec<_>>();
        let brains = population
            .iter()
            .map(|genome| BrainRuntimeState {
                handle: genome.brain,
                owner: BrainOwner::PopulationSlot(genome.slot),
                non_population_weights: None,
                recurrent: vec![0.25; graph.total_state_size].into_boxed_slice(),
            })
            .collect::<Vec<_>>();
        let mut body_points = Vec::new();
        let mut snakes = Vec::new();
        for slot in 0..COUNT {
            let start = body_points.len();
            for point in 0..(5 + slot) {
                body_points.push(WorldPoint {
                    x: slot as f64 * 100.0 - point as f64 * 7.5,
                    y: slot as f64 * 20.0,
                });
            }
            snakes.push(SnakeState {
                id: slot as u64 + 1,
                frame_v1_id: slot as u32 + 1,
                kind: SnakeKind::Evolved,
                alive: slot != 3,
                population_slot: Some(slot as u32),
                brain: Some(population[slot].brain),
                baseline_slot: None,
                baseline_strategy: None,
                position: body_points[start],
                previous_position: body_points[start],
                direction: 0.1 * slot as f64,
                radius: 9.0,
                speed: 165.0,
                boost: false,
                age_seconds: 20.0 + slot as f64,
                food: slot as f64,
                points: [30.0, 18.0, 5.0, 0.0][slot],
                kills: slot as u64,
                target_length: (5 + slot) as f64,
                fitness: 0.0,
                turn: 0.0,
                previous_turn: 0.0,
                input_boost: false,
                previous_input_boost: false,
                control_accumulator_seconds: 0.0,
                delivered_observation_points: 0.0,
                body: BodyRange {
                    start,
                    len: 5 + slot,
                },
                skin: slot as u32,
            });
        }
        let world = WorldState {
            snakes,
            body_points,
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        };
        let mut fixed_step = FixedStepContinuationState::generation_boundary();
        fixed_step
            .sensor_generation
            .update_after_step(&world)
            .expect("fixture sensor continuation must cover every live evolved snake");
        StateCandidate {
            versions: ContractVersions {
                state: ENGINE_STATE_VERSION,
                engine: crate::engine::contract::ENGINE_CONTRACT_VERSION,
                protocol: PROTOCOL_VERSION,
                serializer: SERIALIZER_VERSION,
                sensor: SENSOR_VERSION,
                rng_bundle: RNG_BUNDLE_VERSION,
                checkpoint: CHECKPOINT_VERSION,
                graph_layout: CANONICAL_GRAPH_LAYOUT_VERSION,
            },
            identity: RunIdentity {
                run_id: "generation-run".into(),
                seed: 42,
                config_revision: 3,
                config_hash,
                source_revision: "generation-test-source".into(),
                engine_build_id: "generation-test-engine".into(),
                source_sha256: "1".repeat(64),
                target_triple: "x86_64-pc-windows-msvc".into(),
                build_profile: "release".into(),
                build_class: "production".into(),
                rustc_version: "rustc generation-test".into(),
                build_contract_sha256: format!("sha256:{}", "2".repeat(64)),
                math_backend: "rust-scalar-v1".into(),
            },
            config,
            phase: AuthorityPhase::Running,
            generation: GenerationState {
                boundary_version: GENERATION_BOUNDARY_VERSION,
                generation: 7,
                completed_step: 99,
                population_epoch: EPOCH,
                elapsed_seconds: 12.0,
                wall_accumulator_seconds: 0.25,
                best_fitness_ever: 777.0,
            },
            fixed_step,
            rng: RngStateBundle {
                version: RNG_BUNDLE_VERSION,
                world: stream(42, "world"),
                evolution: stream(42, "evolution"),
                external_controller: stream(42, "external-controller"),
                baselines: (0..BASELINES)
                    .map(|slot| BaselineRngState {
                        slot: slot as u32,
                        state: stream(42, &format!("old-baseline:{slot}")),
                    })
                    .collect(),
            },
            allocators: AllocatorState {
                version: ALLOCATOR_VERSION,
                next_entity_id: 10_000,
                next_brain_id: 20,
                next_genome_id: 100,
                next_controller_lease_id: 30,
                next_frame_v1_id: 40,
                next_external_id: EXTERNAL_ENTITY_ID_START + 50,
                next_baseline_id: BASELINE_ENTITY_ID_START + 60,
                next_resurrected_id: RESURRECTED_ENTITY_ID_START + 70,
            },
            population,
            brains,
            world,
        }
    }

    fn policy(source: &StateCandidate) -> StateAdmissionPolicy {
        StateAdmissionPolicy {
            memory_ceiling_bytes: usize::MAX,
            expected_source_revision: source.identity.source_revision.clone(),
            expected_engine_build_id: source.identity.engine_build_id.clone(),
            expected_source_sha256: source.identity.source_sha256.clone(),
            expected_target_triple: source.identity.target_triple.clone(),
            expected_build_profile: source.identity.build_profile.clone(),
            expected_build_class: source.identity.build_class.clone(),
            expected_rustc_version: source.identity.rustc_version.clone(),
            expected_build_contract_sha256: source.identity.build_contract_sha256.clone(),
            expected_math_backend: source.identity.math_backend.clone(),
            expected_settings_schema_sha256: source.config.settings_schema_sha256.clone(),
        }
    }

    fn advanced(state: &SerializedRngState) -> SerializedRngState {
        let mut rng = StatefulRng::from_state(state).expect("fixture RNG must restore");
        rng.next_f64();
        rng.export_state()
    }

    fn prepared_test_boundary(graph: &CompiledGraph) -> StateCandidate {
        let source = fixture(graph);
        let mut completed_rng = source.rng.clone();
        completed_rng.world = advanced(&completed_rng.world);
        completed_rng.external_controller = advanced(&completed_rng.external_controller);
        let mut completed_allocators = source.allocators.clone();
        completed_allocators.next_entity_id += 3;
        completed_allocators.next_brain_id += 1;
        completed_allocators.next_controller_lease_id += 1;
        completed_allocators.next_frame_v1_id += 2;
        completed_allocators.next_external_id += 1;
        let elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;
        prepare_generation_boundary(
            &source,
            &source.world,
            &completed_rng,
            &completed_allocators,
            elapsed,
            graph,
        )
        .expect("fixture generation must prepare")
        .into_parts()
        .0
    }

    fn replace_setting(candidate: &mut StateCandidate, path: &str, value: NormalizedSettingValue) {
        let setting = candidate
            .config
            .settings
            .iter_mut()
            .find(|setting| setting.path == path)
            .unwrap_or_else(|| panic!("fixture setting {path} must exist"));
        setting.value = value;
    }

    fn refresh_config_identity(candidate: &mut StateCandidate) {
        candidate.config.settings_schema_sha256 =
            normalized_settings_schema_hash(&candidate.config.settings)
                .expect("modified fixture settings must hash");
        candidate.identity.config_hash =
            normalized_config_hash(&candidate.config).expect("modified fixture config must hash");
    }

    #[test]
    fn completed_generation_prepares_one_admissible_zero_state_boundary() {
        let graph = graph_bundle();
        let source = fixture(graph.compiled());
        let admission = policy(&source);
        let mut source_authority =
            AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
                .expect("running source must admit");
        let source_key = source_authority
            .begin_running_step()
            .expect("terminal attempt must have an exact key");
        let source_before = source_authority.state().clone();
        let mut completed_rng = source_before.rng.clone();
        completed_rng.world = advanced(&completed_rng.world);
        completed_rng.external_controller = advanced(&completed_rng.external_controller);
        let completed_rng_before = completed_rng.clone();
        let mut completed_allocators = source_before.allocators.clone();
        completed_allocators.next_entity_id += 3;
        completed_allocators.next_brain_id += 1;
        completed_allocators.next_controller_lease_id += 1;
        completed_allocators.next_frame_v1_id += 2;
        completed_allocators.next_external_id += 1;
        let completed_allocators_before = completed_allocators.clone();
        let elapsed =
            source_before.generation.elapsed_seconds + source_before.config.fixed_step_seconds;

        let prepared = prepare_generation_boundary(
            source_authority.state(),
            &source_before.world,
            &completed_rng,
            &completed_allocators,
            elapsed,
            graph.compiled(),
        )
        .expect("complete generation must prepare");
        assert_eq!(source_authority.state(), &source_before);
        assert_eq!(completed_rng, completed_rng_before);
        assert_eq!(completed_allocators, completed_allocators_before);
        let admitted = admit_prepared_generation_boundary(&source_authority, source_key, prepared)
            .expect("prepared successor must fit beside its source");
        let metadata = admitted.metadata();
        let commit_record = admitted.commit_record();
        assert_eq!(metadata.summary.generation, 7);
        assert_eq!(metadata.hall_of_fame.generation, 7);
        assert_eq!(metadata.hall_of_fame_population_slot, 0);
        let candidate = admitted.candidate();
        assert_eq!(
            commit_record.summary,
            GenerationSummaryCommitRecord {
                completed_generation: metadata.summary.generation,
                best_f64_bits: metadata.summary.best.to_bits(),
                average_f64_bits: metadata.summary.average.to_bits(),
                minimum_f64_bits: metadata.summary.minimum.to_bits(),
                species_count: metadata.summary.species_count,
                top_species_size: metadata.summary.top_species_size,
                average_weight_f64_bits: metadata.summary.average_weight.to_bits(),
                weight_variance_f64_bits: metadata.summary.weight_variance.to_bits(),
            }
        );
        assert_eq!(
            commit_record.hall_of_fame,
            HallOfFameCommitRecord {
                completed_generation: metadata.hall_of_fame.generation,
                source_population_slot: u64::from(metadata.hall_of_fame.source_slot),
                source_snake_id: metadata.hall_of_fame.snake_id,
                fitness_f64_bits: metadata.hall_of_fame.fitness.to_bits(),
                points_f64_bits: metadata.hall_of_fame.points.to_bits(),
                length: u64::try_from(metadata.hall_of_fame.length).unwrap(),
                successor_population_slot: u64::from(metadata.hall_of_fame_population_slot),
                successor_genome_id: candidate.population
                    [metadata.hall_of_fame_population_slot as usize]
                    .lineage
                    .genome_id,
            }
        );
        assert_eq!(
            candidate.population[metadata.hall_of_fame_population_slot as usize].weights,
            source_before.population[metadata.hall_of_fame.source_slot as usize].weights
        );
        assert_eq!(
            candidate.phase,
            AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation)
        );
        assert_eq!(candidate.generation.generation, 8);
        assert_eq!(candidate.generation.completed_step, 100);
        assert_eq!(candidate.generation.population_epoch, 8);
        assert_eq!(candidate.generation.elapsed_seconds, 0.0);
        assert_eq!(candidate.generation.wall_accumulator_seconds, 0.0);
        assert!(candidate.world.snakes.is_empty());
        assert!(candidate.world.body_points.is_empty());
        assert!(candidate.world.pellets.is_empty());
        assert!(candidate.world.controller_leases.is_empty());
        assert_eq!(candidate.rng.world, completed_rng.world);
        assert_eq!(
            candidate.rng.external_controller,
            completed_rng.external_controller
        );
        assert_ne!(candidate.rng.evolution, source_before.rng.evolution);
        assert_eq!(
            candidate.rng.baselines.len(),
            source_before.config.baseline_count
        );
        assert_eq!(candidate.allocators.next_brain_id, 25);
        assert_eq!(candidate.allocators.next_genome_id, 104);
        assert!(candidate
            .brains
            .iter()
            .all(|brain| brain.recurrent.iter().all(|value| value.to_bits() == 0)));
        for (slot, genome) in candidate.population.iter().enumerate() {
            assert_eq!(genome.slot as usize, slot);
            assert_eq!(genome.brain.epoch, 8);
            assert_eq!(genome.lineage.genome_id, 100 + slot as u64);
            assert_eq!(genome.lineage.birth_generation, 8);
            assert!(genome.lineage.parent_a.is_some());
        }
        assert_eq!(
            admitted
                .successor
                .authority()
                .checkpoint_boundary()
                .expect("prepared state must expose a checkpoint boundary")
                .kind(),
            GenerationBoundaryKind::Generation
        );
        assert!(admitted.combined_state_bytes() > source_authority.memory_estimate().total_bytes);
        assert_eq!(source_authority.state(), &source_before);

        let directory = TestDirectory::new();
        let descriptor = admitted
            .publish_managed_checkpoint(
                &source_authority,
                directory.path(),
                CheckpointOperationId::parse("00000000000000000000000000000071")
                    .expect("fixture operation ID"),
                &checkpoint_limits(),
                &graph_limits(),
            )
            .expect("prepared generation must use the production checkpoint writer");
        assert_eq!(descriptor.boundary_kind, CheckpointBoundaryKind::Generation);
        assert_eq!(descriptor.transition_epoch_hex, "0000000000000001");
        assert_eq!(source_authority.state(), &source_before);
        let restored = restore_checkpoint(
            &directory.path().join(&descriptor.relative_filename),
            &checkpoint_limits(),
            &graph_limits(),
            &admission,
        )
        .expect("published generation boundary must restore");
        assert_eq!(
            restored.content.boundary_kind,
            CheckpointBoundaryKind::Generation
        );
        assert_eq!(restored.state.state(), admitted.candidate());

        let projection = project_running_step_config(
            &admitted.candidate().config,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("admitted generation settings must project for construction");
        let start_config =
            GenerationStartConfig::from_work_limits(RunningStepWorkLimits::provisional_defaults());
        let boundary_before_start = admitted.candidate().clone();
        let mut start_workspace = GenerationStartWorkspace::new();
        let started = start_workspace
            .prepare(admitted.candidate(), start_config)
            .expect("durable boundary must construct one collision-safe generation");
        assert_eq!(admitted.candidate(), &boundary_before_start);
        assert_eq!(
            started.world().snakes.len(),
            admitted.candidate().config.population_count
        );
        assert_eq!(
            started.world().body_points.len(),
            admitted.candidate().config.population_count
                * projection.world_step.prefix.baseline_spawn.snake_start_len
        );
        assert_eq!(
            started.world().pellets.len(),
            projection.world_step.prefix.ambient.target_count
        );
        assert!(started.world().controller_leases.is_empty());
        assert!(started
            .world()
            .snakes
            .iter()
            .enumerate()
            .all(|(slot, snake)| {
                snake.kind == SnakeKind::Evolved
                    && snake.population_slot == Some(slot as u32)
                    && snake.brain == Some(admitted.candidate().population[slot].brain)
                    && snake.control_accumulator_seconds.to_bits()
                        == projection
                            .world_step
                            .control
                            .initial_neural_accumulator_seconds()
                            .to_bits()
            }));
        assert_eq!(started.rng().evolution, admitted.candidate().rng.evolution);
        assert_eq!(
            started.rng().external_controller,
            admitted.candidate().rng.external_controller
        );
        assert_ne!(started.rng().world, admitted.candidate().rng.world);
        assert_eq!(
            started.allocators().next_entity_id,
            admitted.candidate().allocators.next_entity_id
                + admitted.candidate().config.population_count as u64
                + projection.world_step.prefix.ambient.target_count as u64
        );
        assert!(started.fixed_step().baseline_lifecycle.slots.is_empty());
        assert_eq!(
            started
                .fixed_step()
                .sensor_generation
                .best_points_this_generation(),
            0.0
        );
        started
            .validate_current(admitted.candidate(), start_config)
            .expect("prepared construction must retain exact source provenance");

        let mut running_candidate = admitted.candidate().clone();
        running_candidate.phase = AuthorityPhase::Running;
        running_candidate.world = started.world().clone();
        running_candidate.rng = started.rng().clone();
        running_candidate.allocators = started.allocators().clone();
        running_candidate.fixed_step = started.fixed_step().clone();
        let running_policy = policy(&running_candidate);
        AuthoritativeState::validate_and_own(
            running_candidate,
            Arc::clone(&graph),
            &running_policy,
        )
        .expect("constructed generation must satisfy complete running-state admission");
    }

    #[test]
    fn generation_commit_rejects_a_forged_successor_elite_slot() {
        let graph = graph_bundle();
        let source = fixture(graph.compiled());
        let admission = policy(&source);
        let mut authority =
            AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
                .expect("running source must admit");
        let source_key = authority
            .begin_running_step()
            .expect("terminal attempt key");
        let source_before = authority.state().clone();
        let mut completed_rng = source_before.rng.clone();
        completed_rng.world = advanced(&completed_rng.world);
        completed_rng.external_controller = advanced(&completed_rng.external_controller);
        let mut completed_allocators = source_before.allocators.clone();
        completed_allocators.next_entity_id += 3;
        completed_allocators.next_brain_id += 1;
        completed_allocators.next_controller_lease_id += 1;
        completed_allocators.next_frame_v1_id += 2;
        completed_allocators.next_external_id += 1;
        let elapsed =
            source_before.generation.elapsed_seconds + source_before.config.fixed_step_seconds;
        let mut prepared = prepare_generation_boundary(
            authority.state(),
            &source_before.world,
            &completed_rng,
            &completed_allocators,
            elapsed,
            graph.compiled(),
        )
        .expect("complete generation must prepare");
        assert!(prepared.candidate.population.len() > 1);
        prepared.metadata.hall_of_fame_population_slot = 1;

        assert!(matches!(
            admit_prepared_generation_boundary(&authority, source_key, prepared),
            Err(GenerationTransitionError::PopulationShape {
                reason:
                    "generation commit successor does not identify the admitted Hall-of-Fame elite"
            })
        ));
        assert_eq!(authority.state(), &source_before);
    }

    #[test]
    fn generation_commit_requires_bit_exact_successor_elite_weights() {
        let graph = graph_bundle();
        let mut source = fixture(graph.compiled());
        for genome in &mut source.population {
            genome.weights[0] = -0.0;
        }
        let admission = policy(&source);
        let mut authority =
            AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
                .expect("running source must admit");
        let source_key = authority
            .begin_running_step()
            .expect("terminal attempt key");
        let source_before = authority.state().clone();
        let mut completed_rng = source_before.rng.clone();
        completed_rng.world = advanced(&completed_rng.world);
        completed_rng.external_controller = advanced(&completed_rng.external_controller);
        let mut completed_allocators = source_before.allocators.clone();
        completed_allocators.next_entity_id += 3;
        completed_allocators.next_brain_id += 1;
        completed_allocators.next_controller_lease_id += 1;
        completed_allocators.next_frame_v1_id += 2;
        completed_allocators.next_external_id += 1;
        let elapsed =
            source_before.generation.elapsed_seconds + source_before.config.fixed_step_seconds;
        let mut prepared = prepare_generation_boundary(
            authority.state(),
            &source_before.world,
            &completed_rng,
            &completed_allocators,
            elapsed,
            graph.compiled(),
        )
        .expect("complete generation must prepare");
        let elite_slot = prepared.metadata.hall_of_fame_population_slot as usize;
        assert_eq!(
            prepared.candidate.population[elite_slot].weights[0].to_bits(),
            (-0.0_f32).to_bits()
        );
        prepared.candidate.population[elite_slot].weights[0] = 0.0;

        assert!(matches!(
            admit_prepared_generation_boundary(&authority, source_key, prepared),
            Err(GenerationTransitionError::PopulationShape {
                reason:
                    "generation commit successor does not identify the admitted Hall-of-Fame elite"
            })
        ));
        assert_eq!(authority.state(), &source_before);
    }

    #[test]
    fn baseline_generation_seed_composition_matches_current_typescript_vectors() {
        let randomized = derive_baseline_rngs(
            42,
            8,
            3,
            BaselineGenerationConfig {
                seed: 123_456_789,
                randomize_seed_per_generation: true,
            },
        )
        .unwrap();
        let fixed = derive_baseline_rngs(
            42,
            8,
            3,
            BaselineGenerationConfig {
                seed: 123_456_789,
                randomize_seed_per_generation: false,
            },
        )
        .unwrap();
        let state = |entry: &BaselineRngState| {
            u32::from_str_radix(
                entry
                    .state
                    .state_hex
                    .strip_prefix("0x")
                    .expect("serialized state prefix"),
                16,
            )
            .expect("serialized Uint32 state")
        };
        assert_eq!(
            randomized.iter().map(state).collect::<Vec<_>>(),
            [3_990_587_595, 3_017_415_498, 633_257_913]
        );
        assert_eq!(
            fixed.iter().map(state).collect::<Vec<_>>(),
            [1_324_131_708, 3_809_538_889, 3_939_651_290]
        );
    }

    #[test]
    fn complete_start_keeps_baseline_streams_separate_and_initializes_every_slot() {
        let graph = graph_bundle();
        let boundary_without_baselines = prepared_test_boundary(graph.compiled());
        let mut base_workspace = GenerationStartWorkspace::new();
        let base = base_workspace
            .prepare(
                &boundary_without_baselines,
                GenerationStartConfig::from_work_limits(
                    RunningStepWorkLimits::provisional_defaults(),
                ),
            )
            .expect("baseline-free comparison world must construct");
        let base_evolved = base.world().snakes.clone();
        let base_bodies = base.world().body_points.clone();
        let base_pellets = base.world().pellets.clone();
        let base_world_rng = base.rng().world.clone();

        let mut boundary = boundary_without_baselines.clone();
        boundary.config.baseline_count = 2;
        boundary.config.settings = fixture_default_settings(boundary.config.population_count, 2);
        boundary.config.settings_schema_sha256 =
            normalized_settings_schema_hash(&boundary.config.settings).unwrap();
        boundary.identity.config_hash = normalized_config_hash(&boundary.config).unwrap();
        boundary.rng.baselines = derive_baseline_rngs(
            boundary.identity.seed,
            boundary.generation.generation,
            2,
            BaselineGenerationConfig {
                seed: 1,
                randomize_seed_per_generation: false,
            },
        )
        .unwrap();
        let baseline_rng_before = boundary.rng.baselines.clone();
        let boundary_policy = policy(&boundary);
        let boundary =
            AuthoritativeState::validate_and_own(boundary, Arc::clone(&graph), &boundary_policy)
                .expect("modified exact boundary must admit");
        let start_config =
            GenerationStartConfig::from_work_limits(RunningStepWorkLimits::provisional_defaults());
        let mut workspace = GenerationStartWorkspace::new();
        let prepared = workspace
            .prepare(boundary.state(), start_config)
            .expect("both baseline slots must construct collision-safely");

        assert_eq!(prepared.world().snakes.len(), 6);
        assert_eq!(&prepared.world().snakes[..4], base_evolved.as_slice());
        assert_eq!(
            &prepared.world().body_points[..base_bodies.len()],
            base_bodies
        );
        assert_eq!(prepared.world().pellets, base_pellets);
        assert_eq!(prepared.rng().world, base_world_rng);
        assert_eq!(prepared.fixed_step().baseline_lifecycle.slots.len(), 2);
        for (slot, source_rng) in baseline_rng_before.iter().enumerate() {
            let snake = &prepared.world().snakes[4 + slot];
            let runtime = &prepared.fixed_step().baseline_lifecycle.slots[slot];
            assert_eq!(snake.kind, SnakeKind::Baseline);
            assert_eq!(snake.baseline_slot, Some(slot as u32));
            assert_eq!(snake.baseline_strategy, Some(BaselineStrategyState::Roam));
            assert_eq!(runtime.slot, slot as u32);
            assert_eq!(runtime.snake_id, snake.id);
            assert_eq!(runtime.respawn_remaining_seconds, None);
            assert_ne!(&prepared.rng().baselines[slot], source_rng);
        }

        let mut running = boundary.state().clone();
        running.phase = AuthorityPhase::Running;
        running.world = prepared.world().clone();
        running.rng = prepared.rng().clone();
        running.allocators = prepared.allocators().clone();
        running.fixed_step = prepared.fixed_step().clone();
        let running_policy = policy(&running);
        AuthoritativeState::validate_and_own(running, Arc::clone(&graph), &running_policy)
            .expect("baseline-complete constructed world must fully admit");
    }

    #[test]
    fn impossible_complete_body_start_is_atomic_and_reports_the_failed_work() {
        let graph = graph_bundle();
        let mut boundary = prepared_test_boundary(graph.compiled());
        boundary.config.world_radius = 800.0;
        replace_setting(
            &mut boundary,
            "worldRadius",
            NormalizedSettingValue::Integer(800),
        );
        replace_setting(
            &mut boundary,
            "snakeSpacing",
            NormalizedSettingValue::Float(20.0),
        );
        replace_setting(
            &mut boundary,
            "snakeStartLen",
            NormalizedSettingValue::Integer(140),
        );
        refresh_config_identity(&mut boundary);
        let admission = policy(&boundary);
        let boundary =
            AuthoritativeState::validate_and_own(boundary, Arc::clone(&graph), &admission)
                .expect("impossible placement is still a valid exact checkpoint boundary");
        let source_before = boundary.state().clone();
        let mut limits = RunningStepWorkLimits::provisional_defaults();
        limits.spawn_random_attempts_per_request = 2;
        limits.spawn_fallback_position_count = 2;
        limits.spawn_fallback_heading_count = 2;
        limits.spawn_candidates_per_request = 8;
        limits.spawn_candidates_per_batch = 32;
        limits.spawn_geometry_checks_per_batch = 100_000;
        let mut workspace = GenerationStartWorkspace::new();

        let error = workspace
            .prepare(
                boundary.state(),
                GenerationStartConfig::from_work_limits(limits),
            )
            .expect_err("a 2,780-unit straight body cannot fit in the 800-unit arena");

        let GenerationStartError::Spawn(spawn_error) = &error else {
            panic!("unexpected generation-start error: {error:?}");
        };
        assert!(
            matches!(
                spawn_error.as_ref(),
                SpawnError::NoCollisionSafePlacement {
                    key: crate::engine::spawn::SpawnKey {
                        domain: SpawnDomain::Evolved,
                        slot: 0,
                    },
                    random_attempts: 2,
                    fallback_candidates: 4,
                }
            ),
            "unexpected spawn error: {spawn_error:?}"
        );
        assert_eq!(boundary.state(), &source_before);
        assert!(!workspace.is_ready());
        assert_eq!(workspace.diagnostics().spawn.candidates_examined, 6);
    }

    #[test]
    fn generation_start_rejects_exhausted_ids_without_mutating_the_boundary() {
        let graph = graph_bundle();
        let mut boundary = prepared_test_boundary(graph.compiled());
        boundary.allocators.next_frame_v1_id = FRAME_V1_EXHAUSTED_ID;
        let admission = policy(&boundary);
        let boundary =
            AuthoritativeState::validate_and_own(boundary, Arc::clone(&graph), &admission)
                .expect("the exact frame exhaustion sentinel is checkpoint-valid");
        let source_before = boundary.state().clone();
        let mut workspace = GenerationStartWorkspace::new();

        let error = workspace
            .prepare(
                boundary.state(),
                GenerationStartConfig::from_work_limits(
                    RunningStepWorkLimits::provisional_defaults(),
                ),
            )
            .expect_err("frame identities cannot silently alias after exhaustion");

        assert!(matches!(error, GenerationStartError::State(_)));
        assert_eq!(boundary.state(), &source_before);
        assert!(!workspace.is_ready());
    }

    #[test]
    fn generation_start_revalidates_source_and_reuses_warmed_storage() {
        let graph = graph_bundle();
        let boundary = prepared_test_boundary(graph.compiled());
        let default_limits = RunningStepWorkLimits::provisional_defaults();
        let config = GenerationStartConfig::from_work_limits(default_limits);
        let mut workspace = GenerationStartWorkspace::new();
        let first = workspace
            .prepare(&boundary, config)
            .expect("first generation construction must prepare");
        let first_world = first.world().clone();
        let first_rng = first.rng().clone();
        let first_allocators = first.allocators().clone();
        let first_fixed_step = first.fixed_step().clone();
        let first_diagnostics = first.diagnostics();
        let distinct_but_equal_source = boundary.clone();
        assert!(matches!(
            first.validate_current(&distinct_but_equal_source, config),
            Err(GenerationStartError::SourceChanged)
        ));
        let mut changed_limits = default_limits;
        changed_limits.spawn_geometry_checks_per_batch -= 1;
        assert!(matches!(
            first.validate_current(
                &boundary,
                GenerationStartConfig::from_work_limits(changed_limits),
            ),
            Err(GenerationStartError::ConfigChanged)
        ));

        let second = workspace
            .prepare(&boundary, config)
            .expect("warmed generation construction must prepare identically");
        assert_eq!(second.world(), &first_world);
        assert_eq!(second.rng(), &first_rng);
        assert_eq!(second.allocators(), &first_allocators);
        assert_eq!(second.fixed_step(), &first_fixed_step);
        let second_diagnostics = second.diagnostics();
        assert_eq!(
            second_diagnostics.request_capacity,
            first_diagnostics.request_capacity
        );
        assert_eq!(
            second_diagnostics.placement_capacity,
            first_diagnostics.placement_capacity
        );
        assert_eq!(
            second_diagnostics.evolved_body_capacity,
            first_diagnostics.evolved_body_capacity
        );
        assert_eq!(
            second_diagnostics.snake_capacity,
            first_diagnostics.snake_capacity
        );
        assert_eq!(
            second_diagnostics.body_point_capacity,
            first_diagnostics.body_point_capacity
        );
        assert_eq!(
            second_diagnostics.pellet_capacity,
            first_diagnostics.pellet_capacity
        );
    }

    #[test]
    fn successor_admission_charges_both_states_and_rejects_a_stale_attempt() {
        let graph = graph_bundle();
        let source = fixture(graph.compiled());
        let elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;
        let prepared = prepare_generation_boundary(
            &source,
            &source.world,
            &source.rng,
            &source.allocators,
            elapsed,
            graph.compiled(),
        )
        .expect("fixture successor must prepare");
        let admitted_probe = AuthoritativeState::validate_and_own(
            source.clone(),
            Arc::clone(&graph),
            &policy(&source),
        )
        .expect("probe source must admit");
        let source_bytes = admitted_probe.memory_estimate().total_bytes;
        drop(admitted_probe);
        let successor_bytes = estimate_state_memory(prepared.candidate(), &graph)
            .expect("successor memory must estimate")
            .total_bytes;
        let constrained_ceiling = source_bytes
            .checked_add(successor_bytes)
            .and_then(|sum| sum.checked_sub(1))
            .expect("fixture combined memory must be positive");
        let mut constrained_policy = policy(&source);
        constrained_policy.memory_ceiling_bytes = constrained_ceiling;
        let mut constrained = AuthoritativeState::validate_and_own(
            source.clone(),
            Arc::clone(&graph),
            &constrained_policy,
        )
        .expect("source alone must fit the constrained process ceiling");
        let constrained_key = constrained.begin_running_step().unwrap();
        assert!(matches!(
            admit_prepared_generation_boundary(&constrained, constrained_key, prepared),
            Err(GenerationTransitionError::State(error))
                if matches!(*error, StateError::MemoryCeilingExceeded { .. })
        ));
        assert_eq!(constrained.state(), &source);

        let prepared = prepare_generation_boundary(
            &source,
            &source.world,
            &source.rng,
            &source.allocators,
            elapsed,
            graph.compiled(),
        )
        .expect("stale-key fixture successor must prepare");
        let mut current = AuthoritativeState::validate_and_own(
            source.clone(),
            Arc::clone(&graph),
            &policy(&source),
        )
        .expect("roomy source must admit");
        let stale_key = current.begin_running_step().unwrap();
        let _newer_key = current.begin_running_step().unwrap();
        assert!(matches!(
            admit_prepared_generation_boundary(&current, stale_key, prepared),
            Err(GenerationTransitionError::State(error))
                if matches!(*error, StateError::StaleFixedStep { .. })
        ));
        assert_eq!(current.state(), &source);
    }

    #[test]
    fn admitted_boundary_refuses_checkpoint_after_source_attempt_changes() {
        let graph = graph_bundle();
        let source = fixture(graph.compiled());
        let admission = policy(&source);
        let mut current =
            AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
                .expect("running source must admit");
        let source_key = current.begin_running_step().expect("terminal attempt key");
        let elapsed =
            current.state().generation.elapsed_seconds + current.state().config.fixed_step_seconds;
        let prepared = prepare_generation_boundary(
            current.state(),
            &current.state().world,
            &current.state().rng,
            &current.state().allocators,
            elapsed,
            graph.compiled(),
        )
        .expect("fixture successor must prepare");
        let admitted = admit_prepared_generation_boundary(&current, source_key, prepared)
            .expect("successor must admit beside the source");
        let source_before = current.state().clone();
        let newer_key = current
            .begin_running_step()
            .expect("new attempt must invalidate the admitted boundary");
        assert_ne!(newer_key, source_key);
        let directory = TestDirectory::new();
        assert!(matches!(
            admitted.publish_managed_checkpoint(
                &current,
                directory.path(),
                CheckpointOperationId::parse("00000000000000000000000000000072")
                    .expect("fixture operation ID"),
                &checkpoint_limits(),
                &graph_limits(),
            ),
            Err(GenerationTransitionError::State(error))
                if matches!(*error, StateError::StaleFixedStep { .. })
        ));
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("test directory must remain readable")
                .count(),
            0
        );
        assert_eq!(current.state(), &source_before);
    }

    #[test]
    fn stale_or_exhausted_continuations_leave_every_source_unchanged() {
        let graph = graph_bundle();
        let source = fixture(graph.compiled());
        let before = source.clone();
        let elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;

        let mut contaminated = source.rng.clone();
        contaminated.evolution = advanced(&contaminated.evolution);
        assert!(matches!(
            prepare_generation_boundary(
                &source,
                &source.world,
                &contaminated,
                &source.allocators,
                elapsed,
                graph.compiled(),
            ),
            Err(GenerationTransitionError::InvalidContinuation {
                field: "evolution RNG advanced during a world step"
            })
        ));

        let mut exhausted = source.allocators.clone();
        exhausted.next_brain_id = u64::MAX - 1;
        assert!(matches!(
            prepare_generation_boundary(
                &source,
                &source.world,
                &source.rng,
                &exhausted,
                elapsed,
                graph.compiled(),
            ),
            Err(GenerationTransitionError::State(_))
        ));
        assert_eq!(source, before);
    }

    #[test]
    fn elapsed_and_allocator_regressions_fail_before_evolution_output() {
        let graph = graph_bundle();
        let source = fixture(graph.compiled());
        assert!(matches!(
            prepare_generation_boundary(
                &source,
                &source.world,
                &source.rng,
                &source.allocators,
                source.generation.elapsed_seconds,
                graph.compiled(),
            ),
            Err(GenerationTransitionError::InvalidContinuation {
                field: "generation elapsed seconds"
            })
        ));

        let mut regressed = source.allocators.clone();
        regressed.next_entity_id -= 1;
        assert!(matches!(
            prepare_generation_boundary(
                &source,
                &source.world,
                &source.rng,
                &regressed,
                source.generation.elapsed_seconds + source.config.fixed_step_seconds,
                graph.compiled(),
            ),
            Err(GenerationTransitionError::InvalidContinuation {
                field: "entity allocator regressed"
            })
        ));
    }
}
