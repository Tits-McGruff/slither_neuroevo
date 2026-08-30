//! Serial deterministic neuroevolution at one completed-generation boundary.
//!
//! This module preserves the current TypeScript selection, crossover, mutation,
//! fitness, species, and network-statistic ordering without publishing a new
//! authority. The source world, population, and evolution RNG remain borrowed
//! and immutable. Stage 6's durable transaction assigns new identities, writes
//! the exact boundary checkpoint, and only then publishes the prepared result.

use super::graph::{CompiledGraph, CompiledNode, CompiledNodeType};
use super::rng::{RngError, SerializedRngState, StatefulRng};
use super::state::{PopulationGenome, SnakeKind, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Version of the TypeScript-compatible serial evolution contract.
pub const EVOLUTION_VERSION: u32 = 1;
/// Current TypeScript tournament size.
pub const TOURNAMENT_SIZE: usize = 5;
/// Current TypeScript RMS threshold used for diagnostic species buckets.
pub const SPECIES_DISTANCE_THRESHOLD: f64 = 0.35;
/// Current mutation clamp applied after Gaussian noise.
const MUTATION_WEIGHT_LIMIT: f64 = 5.0;
/// Current tolerance for awarding the top-points bonus.
const TOP_POINTS_TOLERANCE: f64 = 1.0e-6;

/// Complete behavior settings used by one generation's serial evolution pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvolutionConfig {
    /// Fraction copied without crossover or mutation.
    pub elite_fraction: f64,
    /// Per-parameter mutation probability for non-recurrent nodes.
    pub mutation_rate: f64,
    /// Gaussian mutation standard deviation for non-recurrent nodes.
    pub mutation_std: f64,
    /// Probability of structured crossover rather than one-parent cloning.
    pub crossover_rate: f64,
    /// Per-parameter mutation probability for recurrent nodes.
    pub recurrent_mutation_rate: f64,
    /// Gaussian mutation standard deviation for recurrent nodes.
    pub recurrent_mutation_std: f64,
    /// Recurrent mode: zero selects one whole block; one selects hidden units.
    pub recurrent_crossover_mode: u8,
    /// Starting body length subtracted by the growth-fitness component.
    pub snake_start_length: usize,
    /// Fitness awarded per simulated second survived.
    pub fitness_survival_per_second: f64,
    /// Fitness awarded per accumulated unit of eaten food.
    pub fitness_food: f64,
    /// Fitness awarded per body segment above starting length.
    pub fitness_length_per_segment: f64,
    /// Fitness awarded per credited kill.
    pub fitness_kill: f64,
    /// Fitness weight applied to logarithmically normalized points.
    pub fitness_points_normalized: f64,
    /// Bonus awarded to every snake tied for top points.
    pub fitness_top_points_bonus: f64,
    /// Maximum admitted evolved population records.
    pub maximum_population: usize,
    /// Maximum admitted Float32 output parameters across the population.
    pub maximum_output_weight_floats: usize,
}

impl EvolutionConfig {
    /// Current TypeScript defaults with explicit provisional output ceilings.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            elite_fraction: 0.12,
            mutation_rate: 0.03,
            mutation_std: 0.35,
            crossover_rate: 0.85,
            recurrent_mutation_rate: 0.025,
            recurrent_mutation_std: 0.22,
            recurrent_crossover_mode: 1,
            snake_start_length: 5,
            fitness_survival_per_second: 0.70,
            fitness_food: 80.0,
            fitness_length_per_segment: 100.0,
            fitness_kill: 400.0,
            fitness_points_normalized: 42.0,
            fitness_top_points_bonus: 600.0,
            maximum_population: 300,
            maximum_output_weight_floats: 200_000_000,
        }
    }

    /// Validate current owner-visible setting ranges and output ceilings.
    pub fn validate(self) -> Result<(), EvolutionError> {
        validate_range("eliteFrac", self.elite_fraction, 0.01, 0.5)?;
        validate_range("mutationRate", self.mutation_rate, 0.0, 0.5)?;
        validate_range("mutationStd", self.mutation_std, 0.0, 2.5)?;
        validate_range("crossoverRate", self.crossover_rate, 0.0, 1.0)?;
        validate_range(
            "brain.gruMutationRate",
            self.recurrent_mutation_rate,
            0.0,
            0.35,
        )?;
        validate_range(
            "brain.gruMutationStd",
            self.recurrent_mutation_std,
            0.0,
            1.6,
        )?;
        if self.recurrent_crossover_mode > 1 {
            return Err(EvolutionError::InvalidConfig {
                path: "brain.gruCrossoverMode",
            });
        }
        if !(5..=140).contains(&self.snake_start_length) {
            return Err(EvolutionError::InvalidConfig {
                path: "snakeStartLen",
            });
        }
        for (path, value, maximum) in [
            (
                "reward.fitnessSurvivalPerSecond",
                self.fitness_survival_per_second,
                10.0,
            ),
            ("reward.fitnessFood", self.fitness_food, 80.0),
            (
                "reward.fitnessLengthPerSegment",
                self.fitness_length_per_segment,
                100.0,
            ),
            ("reward.fitnessKill", self.fitness_kill, 400.0),
            (
                "reward.fitnessPointsNorm",
                self.fitness_points_normalized,
                300.0,
            ),
            (
                "reward.fitnessTopPointsBonus",
                self.fitness_top_points_bonus,
                600.0,
            ),
        ] {
            validate_range(path, value, 0.0, maximum)?;
        }
        if self.maximum_population == 0 || self.maximum_output_weight_floats == 0 {
            return Err(EvolutionError::InvalidConfig {
                path: "evolution work ceiling",
            });
        }
        Ok(())
    }
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self::typescript_defaults()
    }
}

/// Origin of one next-generation genome before durable lineage IDs are assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextGenomeOrigin {
    /// Exact elite copy retaining prior-generation fitness.
    Elite {
        /// Stable source slot before fitness sorting.
        parent_slot: u32,
    },
    /// Crossover/mutation child with fitness reset to zero.
    Child {
        /// Stable slot selected by the first tournament.
        parent_a_slot: u32,
        /// Stable slot selected by the second tournament.
        parent_b_slot: u32,
    },
}

/// One complete next-generation genome in its new dense slot.
#[derive(Clone, Debug, PartialEq)]
pub struct NextGenerationGenome {
    /// Dense slot in the new population.
    pub slot: u32,
    /// Elite or child provenance expressed in source slots.
    pub origin: NextGenomeOrigin,
    /// Fitness retained for elites and reset for children.
    pub fitness: f64,
    /// Packed canonical graph parameters.
    pub weights: Vec<f32>,
}

/// Compact eight-field generation history record selected by the owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenerationSummary {
    /// Completed generation number.
    pub generation: u64,
    /// Maximum fitness.
    pub best: f64,
    /// Arithmetic mean fitness.
    pub average: f64,
    /// Minimum fitness.
    pub minimum: f64,
    /// Greedy RMS-threshold species count in sorted-fitness order.
    pub species_count: u64,
    /// Largest greedy species bucket.
    pub top_species_size: u64,
    /// Mean absolute parameter value.
    pub average_weight: f64,
    /// Variance of absolute parameter values.
    pub weight_variance: f64,
}

/// Hall-of-Fame candidate emitted for the completed run-scoped generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HallOfFameCandidate {
    /// Completed generation number.
    pub generation: u64,
    /// Stable source slot owning the selected genome.
    pub source_slot: u32,
    /// Stable snake identity retained as current TypeScript `seed` metadata.
    pub snake_id: u64,
    /// Selected fitness.
    pub fitness: f64,
    /// Selected points score.
    pub points: f64,
    /// Selected body-point count.
    pub length: usize,
}

/// Complete non-authoritative evolution result.
#[derive(Debug)]
pub struct PreparedEvolution<'source> {
    source_world: &'source WorldState,
    source_population: &'source [PopulationGenome],
    source_rng: &'source SerializedRngState,
    source_generation: u64,
    source_best_fitness_ever: f64,
    sorted_source_slots: Vec<usize>,
    source_fitness: Vec<f64>,
    next_population: Vec<NextGenerationGenome>,
    summary: GenerationSummary,
    hall_of_fame: HallOfFameCandidate,
    next_evolution_rng: SerializedRngState,
    next_best_fitness_ever: f64,
}

/// Owned evolution values moved into the durable generation-boundary builder.
///
/// Consuming the prepared result prevents a second population-sized copy of
/// packed weights while the prior authority remains borrowed and unchanged.
pub(crate) struct EvolutionTransitionParts<'source> {
    pub(crate) source_population: &'source [PopulationGenome],
    pub(crate) next_population: Vec<NextGenerationGenome>,
    pub(crate) summary: GenerationSummary,
    pub(crate) hall_of_fame: HallOfFameCandidate,
    pub(crate) next_evolution_rng: SerializedRngState,
    pub(crate) next_best_fitness_ever: f64,
}

impl<'source> PreparedEvolution<'source> {
    /// Read the exact retained source world.
    #[must_use]
    pub const fn source_world(&self) -> &'source WorldState {
        self.source_world
    }
    /// Read the exact retained source population.
    #[must_use]
    pub const fn source_population(&self) -> &'source [PopulationGenome] {
        self.source_population
    }
    /// Read the exact borrowed input evolution continuation.
    #[must_use]
    pub const fn source_rng(&self) -> &'source SerializedRngState {
        self.source_rng
    }
    /// Read the completed source generation.
    #[must_use]
    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }
    /// Read the prior all-generation best fitness.
    #[must_use]
    pub const fn source_best_fitness_ever(&self) -> f64 {
        self.source_best_fitness_ever
    }
    /// Read source slots in descending-fitness/stable-slot order.
    #[must_use]
    pub fn sorted_source_slots(&self) -> &[usize] {
        &self.sorted_source_slots
    }
    /// Read calculated fitness by stable source slot.
    #[must_use]
    pub fn source_fitness(&self) -> &[f64] {
        &self.source_fitness
    }
    /// Read the complete next population.
    #[must_use]
    pub fn next_population(&self) -> &[NextGenerationGenome] {
        &self.next_population
    }
    /// Read the compact generation summary.
    #[must_use]
    pub const fn summary(&self) -> GenerationSummary {
        self.summary
    }
    /// Read the run-scoped Hall-of-Fame candidate.
    #[must_use]
    pub const fn hall_of_fame(&self) -> HallOfFameCandidate {
        self.hall_of_fame
    }
    /// Resolve the Hall-of-Fame candidate's immutable source genome.
    #[must_use]
    pub fn hall_of_fame_genome(&self) -> &PopulationGenome {
        &self.source_population[self.hall_of_fame.source_slot as usize]
    }
    /// Read the exact evolution RNG continuation after every serial draw.
    #[must_use]
    pub const fn next_evolution_rng(&self) -> &SerializedRngState {
        &self.next_evolution_rng
    }
    /// Read the updated all-generation best fitness.
    #[must_use]
    pub const fn next_best_fitness_ever(&self) -> f64 {
        self.next_best_fitness_ever
    }

    /// Consume the preparation without copying its population-sized weights.
    pub(crate) fn into_transition_parts(self) -> EvolutionTransitionParts<'source> {
        EvolutionTransitionParts {
            source_population: self.source_population,
            next_population: self.next_population,
            summary: self.summary,
            hall_of_fame: self.hall_of_fame,
            next_evolution_rng: self.next_evolution_rng,
            next_best_fitness_ever: self.next_best_fitness_ever,
        }
    }
}

/// Validation, allocation, formula, or RNG failure.
#[derive(Debug)]
pub enum EvolutionError {
    /// One projected setting or ceiling is invalid.
    InvalidConfig { path: &'static str },
    /// Source generation or best-fitness continuation is invalid.
    InvalidGeneration,
    /// Population is empty or exceeds its ceiling.
    PopulationLimit { actual: usize, maximum: usize },
    /// Checked output-weight arithmetic exceeded its ceiling.
    WeightLimit { actual: usize, maximum: usize },
    /// Population slots or graph weights are incompatible.
    PopulationShape { reason: &'static str },
    /// An evolved snake is missing, duplicated, or inconsistent.
    SnakeMapping { slot: usize, reason: &'static str },
    /// A source or derived numeric value is invalid.
    NonFinite { context: &'static str, slot: usize },
    /// Checked arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// A required allocation failed.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// Borrowed evolution RNG is invalid.
    Rng(RngError),
}

impl Display for EvolutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { path } => write!(formatter, "invalid evolution setting {path}"),
            Self::InvalidGeneration => write!(formatter, "invalid source generation continuation"),
            Self::PopulationLimit { actual, maximum } => write!(
                formatter,
                "evolution population {actual} exceeds admitted maximum {maximum}"
            ),
            Self::WeightLimit { actual, maximum } => write!(
                formatter,
                "evolution output requires {actual} Float32 weights; maximum is {maximum}"
            ),
            Self::PopulationShape { reason } => {
                write!(formatter, "invalid evolution population: {reason}")
            }
            Self::SnakeMapping { slot, reason } => write!(
                formatter,
                "invalid evolved snake mapping for slot {slot}: {reason}"
            ),
            Self::NonFinite { context, slot } => {
                write!(formatter, "non-finite evolution {context} for slot {slot}")
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "evolution arithmetic overflow while calculating {context}"
                )
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "unable to reserve {required} entries for evolution {context}"
            ),
            Self::Rng(error) => write!(formatter, "invalid evolution RNG: {error}"),
        }
    }
}

impl Error for EvolutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Rng(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RngError> for EvolutionError {
    fn from(error: RngError) -> Self {
        Self::Rng(error)
    }
}

/// Prepare one complete serial evolution pass without mutating authority.
pub fn prepare_evolution<'source>(
    world: &'source WorldState,
    population: &'source [PopulationGenome],
    graph: &CompiledGraph,
    evolution_rng: &'source SerializedRngState,
    source_generation: u64,
    source_best_fitness_ever: f64,
    config: EvolutionConfig,
) -> Result<PreparedEvolution<'source>, EvolutionError> {
    config.validate()?;
    if source_generation == 0 || !source_best_fitness_ever.is_finite() {
        return Err(EvolutionError::InvalidGeneration);
    }
    let count = population.len();
    if count == 0 || count > config.maximum_population {
        return Err(EvolutionError::PopulationLimit {
            actual: count,
            maximum: config.maximum_population,
        });
    }
    validate_population(population, graph)?;
    let total_weights =
        count
            .checked_mul(graph.total_parameters)
            .ok_or(EvolutionError::ArithmeticOverflow {
                context: "complete next-population weights",
            })?;
    if total_weights > config.maximum_output_weight_floats {
        return Err(EvolutionError::WeightLimit {
            actual: total_weights,
            maximum: config.maximum_output_weight_floats,
        });
    }

    let snake_indices = map_population_snakes(world, population)?;
    let mut source_fitness = reserve_vec(count, "source fitness")?;
    source_fitness.resize(count, 0.0);
    calculate_fitness(world, &snake_indices, &mut source_fitness, config)?;

    let mut sorted_source_slots = reserve_vec(count, "sorted source slots")?;
    sorted_source_slots.extend(0..count);
    sorted_source_slots.sort_unstable_by(|left, right| {
        source_fitness[*right]
            .total_cmp(&source_fitness[*left])
            .then_with(|| left.cmp(right))
    });

    let summary = calculate_summary(
        population,
        &sorted_source_slots,
        &source_fitness,
        source_generation,
    )?;
    let best_source_slot = sorted_source_slots[0];
    let best_snake = &world.snakes[snake_indices[best_source_slot]];
    let hall_of_fame = HallOfFameCandidate {
        generation: source_generation,
        source_slot: u32::try_from(best_source_slot).map_err(|_| {
            EvolutionError::ArithmeticOverflow {
                context: "Hall-of-Fame source slot",
            }
        })?,
        snake_id: best_snake.id,
        fitness: source_fitness[best_source_slot],
        points: best_snake.points,
        length: best_snake.body.len,
    };

    let mut rng = StatefulRng::from_state(evolution_rng)?;
    let elite_count = ((config.elite_fraction * count as f64).floor() as usize)
        .max(1)
        .min(count);
    let mut next_population = reserve_vec(count, "next population")?;
    for (new_slot, source_slot) in sorted_source_slots
        .iter()
        .copied()
        .take(elite_count)
        .enumerate()
    {
        let source = &population[source_slot];
        next_population.push(NextGenerationGenome {
            slot: u32::try_from(new_slot).map_err(|_| EvolutionError::ArithmeticOverflow {
                context: "elite output slot",
            })?,
            origin: NextGenomeOrigin::Elite {
                parent_slot: source.slot,
            },
            fitness: source_fitness[source_slot],
            weights: clone_weights(&source.weights, "elite weights")?,
        });
    }
    while next_population.len() < count {
        let parent_a_slot = tournament_pick(&sorted_source_slots, &source_fitness, &mut rng)?;
        let parent_b_slot = tournament_pick(&sorted_source_slots, &source_fitness, &mut rng)?;
        let mut weights = crossover(
            &population[parent_a_slot].weights,
            &population[parent_b_slot].weights,
            graph,
            config,
            &mut rng,
        )?;
        mutate(&mut weights, graph, config, &mut rng)?;
        let new_slot = next_population.len();
        next_population.push(NextGenerationGenome {
            slot: u32::try_from(new_slot).map_err(|_| EvolutionError::ArithmeticOverflow {
                context: "child output slot",
            })?,
            origin: NextGenomeOrigin::Child {
                parent_a_slot: population[parent_a_slot].slot,
                parent_b_slot: population[parent_b_slot].slot,
            },
            fitness: 0.0,
            weights,
        });
    }

    Ok(PreparedEvolution {
        source_world: world,
        source_population: population,
        source_rng: evolution_rng,
        source_generation,
        source_best_fitness_ever,
        sorted_source_slots,
        source_fitness,
        next_population,
        summary,
        hall_of_fame,
        next_evolution_rng: rng.export_state(),
        next_best_fitness_ever: source_best_fitness_ever.max(summary.best),
    })
}

fn validate_range(
    path: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), EvolutionError> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(EvolutionError::InvalidConfig { path });
    }
    Ok(())
}

fn reserve_vec<T>(required: usize, context: &'static str) -> Result<Vec<T>, EvolutionError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(required)
        .map_err(|_| EvolutionError::AllocationFailed { context, required })?;
    Ok(values)
}

fn clone_weights(weights: &[f32], context: &'static str) -> Result<Vec<f32>, EvolutionError> {
    let mut output = reserve_vec(weights.len(), context)?;
    output.extend_from_slice(weights);
    Ok(output)
}

fn validate_population(
    population: &[PopulationGenome],
    graph: &CompiledGraph,
) -> Result<(), EvolutionError> {
    let mut expected_offset = 0usize;
    for node in &graph.nodes {
        if node.parameter_offset != expected_offset {
            return Err(EvolutionError::PopulationShape {
                reason: "compiled parameter ranges are not contiguous",
            });
        }
        expected_offset = expected_offset.checked_add(node.parameter_length).ok_or(
            EvolutionError::ArithmeticOverflow {
                context: "compiled parameter ranges",
            },
        )?;
    }
    if expected_offset != graph.total_parameters {
        return Err(EvolutionError::PopulationShape {
            reason: "compiled total parameter count is inconsistent",
        });
    }
    for (index, genome) in population.iter().enumerate() {
        if genome.slot as usize != index {
            return Err(EvolutionError::PopulationShape {
                reason: "population slots are not dense and canonical",
            });
        }
        if genome.weights.len() != graph.total_parameters {
            return Err(EvolutionError::PopulationShape {
                reason: "a genome weight length does not match the graph",
            });
        }
        if !genome.fitness.is_finite() || genome.weights.iter().any(|value| !value.is_finite()) {
            return Err(EvolutionError::NonFinite {
                context: "source genome",
                slot: index,
            });
        }
    }
    Ok(())
}

fn map_population_snakes(
    world: &WorldState,
    population: &[PopulationGenome],
) -> Result<Vec<usize>, EvolutionError> {
    let mut indices = reserve_vec(population.len(), "population snake mapping")?;
    indices.resize(population.len(), usize::MAX);
    for (snake_index, snake) in world.snakes.iter().enumerate() {
        if snake.kind != SnakeKind::Evolved {
            continue;
        }
        let slot = snake.population_slot.ok_or(EvolutionError::SnakeMapping {
            slot: population.len(),
            reason: "evolved snake has no population slot",
        })? as usize;
        if slot >= population.len() {
            return Err(EvolutionError::SnakeMapping {
                slot,
                reason: "population slot is out of bounds",
            });
        }
        if indices[slot] != usize::MAX {
            return Err(EvolutionError::SnakeMapping {
                slot,
                reason: "population slot is duplicated",
            });
        }
        if snake.brain != Some(population[slot].brain) {
            return Err(EvolutionError::SnakeMapping {
                slot,
                reason: "snake brain does not match its genome",
            });
        }
        indices[slot] = snake_index;
    }
    if let Some(slot) = indices.iter().position(|index| *index == usize::MAX) {
        return Err(EvolutionError::SnakeMapping {
            slot,
            reason: "population snake is missing",
        });
    }
    Ok(indices)
}

fn calculate_fitness(
    world: &WorldState,
    snake_indices: &[usize],
    output: &mut [f64],
    config: EvolutionConfig,
) -> Result<(), EvolutionError> {
    let mut maximum_points = 0.0_f64;
    for &snake_index in snake_indices {
        maximum_points = maximum_points.max(world.snakes[snake_index].points);
    }
    if maximum_points <= 0.0 {
        maximum_points = 1.0;
    }
    let logarithm_denominator = (1.0 + maximum_points).ln();
    if !logarithm_denominator.is_finite() || logarithm_denominator <= 0.0 {
        return Err(EvolutionError::NonFinite {
            context: "points normalization denominator",
            slot: 0,
        });
    }
    for (slot, &snake_index) in snake_indices.iter().enumerate() {
        let snake = &world.snakes[snake_index];
        let points_numerator = (1.0 + snake.points).ln();
        let points_normalized = (points_numerator / logarithm_denominator).clamp(0.0, 1.0);
        let top_bonus = if (snake.points - maximum_points).abs() <= TOP_POINTS_TOLERANCE {
            config.fitness_top_points_bonus
        } else {
            0.0
        };
        let grown_segments = snake.body.len.saturating_sub(config.snake_start_length) as f64;
        let fitness = snake.age_seconds * config.fitness_survival_per_second
            + snake.food * config.fitness_food
            + grown_segments * config.fitness_length_per_segment
            + snake.kills as f64 * config.fitness_kill
            + points_normalized * config.fitness_points_normalized
            + top_bonus;
        if !points_normalized.is_finite() || !fitness.is_finite() {
            return Err(EvolutionError::NonFinite {
                context: "calculated fitness",
                slot,
            });
        }
        output[slot] = fitness;
    }
    Ok(())
}

fn calculate_summary(
    population: &[PopulationGenome],
    sorted_slots: &[usize],
    fitness: &[f64],
    generation: u64,
) -> Result<GenerationSummary, EvolutionError> {
    let first = sorted_slots[0];
    let last = sorted_slots[sorted_slots.len() - 1];
    let average =
        sorted_slots.iter().map(|slot| fitness[*slot]).sum::<f64>() / fitness.len() as f64;
    let mut representatives = reserve_vec(sorted_slots.len(), "species representatives")?;
    let mut species_sizes = reserve_vec(sorted_slots.len(), "species sizes")?;
    for &slot in sorted_slots {
        let mut assigned = false;
        for (species_index, &representative) in representatives.iter().enumerate() {
            if genome_distance_rms(&population[slot], &population[representative])?
                <= SPECIES_DISTANCE_THRESHOLD
            {
                species_sizes[species_index] += 1_u64;
                assigned = true;
                break;
            }
        }
        if !assigned {
            representatives.push(slot);
            species_sizes.push(1_u64);
        }
    }
    let mut sum_absolute = 0.0_f64;
    let mut sum_absolute_squared = 0.0_f64;
    let mut weight_count = 0usize;
    for &slot in sorted_slots {
        for &weight in population[slot].weights.iter() {
            let absolute = f64::from(weight).abs();
            sum_absolute += absolute;
            sum_absolute_squared += absolute * absolute;
            weight_count =
                weight_count
                    .checked_add(1)
                    .ok_or(EvolutionError::ArithmeticOverflow {
                        context: "network statistic weight count",
                    })?;
        }
    }
    let (average_weight, weight_variance) = if weight_count == 0 {
        (0.0, 0.0)
    } else {
        let average_weight = sum_absolute / weight_count as f64;
        (
            average_weight,
            (sum_absolute_squared / weight_count as f64 - average_weight * average_weight).max(0.0),
        )
    };
    let summary = GenerationSummary {
        generation,
        best: fitness[first],
        average,
        minimum: fitness[last],
        species_count: representatives.len() as u64,
        top_species_size: species_sizes.iter().copied().max().unwrap_or(0),
        average_weight,
        weight_variance,
    };
    if [
        summary.best,
        summary.average,
        summary.minimum,
        summary.average_weight,
        summary.weight_variance,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return Err(EvolutionError::NonFinite {
            context: "generation summary",
            slot: first,
        });
    }
    Ok(summary)
}

fn genome_distance_rms(
    left: &PopulationGenome,
    right: &PopulationGenome,
) -> Result<f64, EvolutionError> {
    if left.weights.len() != right.weights.len() {
        return Err(EvolutionError::PopulationShape {
            reason: "species genomes have incompatible weights",
        });
    }
    if left.weights.is_empty() {
        // TypeScript computes `Math.sqrt(0 / 0)`, yielding NaN; its `<=`
        // threshold comparison is then false, so every draw-free genome forms
        // its own diagnostic species without invalidating evolution.
        return Ok(f64::NAN);
    }
    let sum_squared = left
        .weights
        .iter()
        .zip(right.weights.iter())
        .map(|(left, right)| {
            let difference = f64::from(*left) - f64::from(*right);
            difference * difference
        })
        .sum::<f64>();
    Ok((sum_squared / left.weights.len() as f64).sqrt())
}

fn tournament_pick(
    sorted_slots: &[usize],
    fitness: &[f64],
    rng: &mut StatefulRng,
) -> Result<usize, EvolutionError> {
    let count =
        u64::try_from(sorted_slots.len()).map_err(|_| EvolutionError::ArithmeticOverflow {
            context: "tournament population length",
        })?;
    let mut best = None;
    for _ in 0..TOURNAMENT_SIZE {
        let sampled_index =
            usize::try_from(rng.int(count)?).map_err(|_| EvolutionError::ArithmeticOverflow {
                context: "tournament sampled index",
            })?;
        let slot = sorted_slots[sampled_index];
        if best.is_none_or(|best_slot| fitness[slot] > fitness[best_slot]) {
            best = Some(slot);
        }
    }
    best.ok_or(EvolutionError::PopulationShape {
        reason: "tournament produced no parent",
    })
}

fn crossover(
    parent_a: &[f32],
    parent_b: &[f32],
    graph: &CompiledGraph,
    config: EvolutionConfig,
    rng: &mut StatefulRng,
) -> Result<Vec<f32>, EvolutionError> {
    if parent_a.len() != graph.total_parameters || parent_b.len() != graph.total_parameters {
        return Err(EvolutionError::PopulationShape {
            reason: "crossover parent length does not match the graph",
        });
    }
    let mut child = reserve_vec(graph.total_parameters, "child weights")?;
    child.resize(graph.total_parameters, 0.0);
    if rng.next_f64() > config.crossover_rate {
        child.copy_from_slice(if rng.next_f64() < 0.5 {
            parent_a
        } else {
            parent_b
        });
        return Ok(child);
    }
    for node in &graph.nodes {
        let end = node
            .parameter_offset
            .checked_add(node.parameter_length)
            .ok_or(EvolutionError::ArithmeticOverflow {
                context: "crossover node range",
            })?;
        let output =
            child
                .get_mut(node.parameter_offset..end)
                .ok_or(EvolutionError::PopulationShape {
                    reason: "crossover node range is out of bounds",
                })?;
        let left = &parent_a[node.parameter_offset..end];
        let right = &parent_b[node.parameter_offset..end];
        if is_recurrent(node.node_type) {
            crossover_recurrent(node, output, left, right, config, rng)?;
        } else {
            for ((output, left), right) in output.iter_mut().zip(left).zip(right) {
                *output = if rng.next_f64() < 0.5 { *left } else { *right };
            }
        }
    }
    Ok(child)
}

fn crossover_recurrent(
    node: &CompiledNode,
    output: &mut [f32],
    parent_a: &[f32],
    parent_b: &[f32],
    config: EvolutionConfig,
    rng: &mut StatefulRng,
) -> Result<(), EvolutionError> {
    if config.recurrent_crossover_mode == 0 {
        output.copy_from_slice(if rng.next_f64() < 0.5 {
            parent_a
        } else {
            parent_b
        });
        return Ok(());
    }
    let hidden = node.hidden_size.ok_or(EvolutionError::PopulationShape {
        reason: "recurrent node has no hidden size",
    })?;
    let gates = recurrent_gate_count(node.node_type).ok_or(EvolutionError::PopulationShape {
        reason: "unsupported recurrent node type",
    })?;
    let input_block =
        hidden
            .checked_mul(node.input_size)
            .ok_or(EvolutionError::ArithmeticOverflow {
                context: "recurrent input block",
            })?;
    let state_block = hidden
        .checked_mul(hidden)
        .ok_or(EvolutionError::ArithmeticOverflow {
            context: "recurrent state block",
        })?;
    let input_total = gates
        .checked_mul(input_block)
        .ok_or(EvolutionError::ArithmeticOverflow {
            context: "recurrent input ranges",
        })?;
    let state_total = gates
        .checked_mul(state_block)
        .ok_or(EvolutionError::ArithmeticOverflow {
            context: "recurrent state ranges",
        })?;
    let bias_start =
        input_total
            .checked_add(state_total)
            .ok_or(EvolutionError::ArithmeticOverflow {
                context: "recurrent bias start",
            })?;
    let expected = bias_start
        .checked_add(
            gates
                .checked_mul(hidden)
                .ok_or(EvolutionError::ArithmeticOverflow {
                    context: "recurrent bias ranges",
                })?,
        )
        .ok_or(EvolutionError::ArithmeticOverflow {
            context: "recurrent parameter length",
        })?;
    if output.len() != expected || parent_a.len() != expected || parent_b.len() != expected {
        return Err(EvolutionError::PopulationShape {
            reason: "recurrent parameter shape is inconsistent",
        });
    }
    for unit in 0..hidden {
        let source = if rng.next_f64() < 0.5 {
            parent_a
        } else {
            parent_b
        };
        for gate in 0..gates {
            copy_row(
                output,
                source,
                gate * input_block + unit * node.input_size,
                node.input_size,
            );
            copy_row(
                output,
                source,
                input_total + gate * state_block + unit * hidden,
                hidden,
            );
            let bias = bias_start + gate * hidden + unit;
            output[bias] = source[bias];
        }
    }
    Ok(())
}

fn copy_row(output: &mut [f32], source: &[f32], start: usize, length: usize) {
    output[start..start + length].copy_from_slice(&source[start..start + length]);
}

fn mutate(
    weights: &mut [f32],
    graph: &CompiledGraph,
    config: EvolutionConfig,
    rng: &mut StatefulRng,
) -> Result<(), EvolutionError> {
    for node in &graph.nodes {
        let end = node
            .parameter_offset
            .checked_add(node.parameter_length)
            .ok_or(EvolutionError::ArithmeticOverflow {
                context: "mutation node range",
            })?;
        let recurrent = is_recurrent(node.node_type);
        let rate = if recurrent {
            config.recurrent_mutation_rate
        } else {
            config.mutation_rate
        };
        let standard_deviation = if recurrent {
            config.recurrent_mutation_std
        } else {
            config.mutation_std
        };
        for (parameter, weight) in weights[node.parameter_offset..end].iter_mut().enumerate() {
            if rng.next_f64() < rate {
                let mutated = (f64::from(*weight) + rng.gaussian() * standard_deviation)
                    .clamp(-MUTATION_WEIGHT_LIMIT, MUTATION_WEIGHT_LIMIT);
                *weight = mutated as f32;
                if !weight.is_finite() {
                    return Err(EvolutionError::NonFinite {
                        context: "mutated weight",
                        slot: parameter,
                    });
                }
            }
        }
    }
    Ok(())
}

const fn is_recurrent(node_type: CompiledNodeType) -> bool {
    matches!(
        node_type,
        CompiledNodeType::Gru | CompiledNodeType::Lstm | CompiledNodeType::Rru
    )
}

const fn recurrent_gate_count(node_type: CompiledNodeType) -> Option<usize> {
    match node_type {
        CompiledNodeType::Gru => Some(3),
        CompiledNodeType::Lstm => Some(4),
        CompiledNodeType::Rru => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::{
        compile_graph, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::rng::StatefulRng;
    use crate::engine::state::{BodyRange, BrainHandle, GenomeLineage, SnakeState, WorldPoint};
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        evolution: FixtureConfig,
        source: FixtureSource,
        expected: FixtureExpected,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureConfig {
        elite_frac: f64,
        mutation_rate: f64,
        mutation_std: f64,
        crossover_rate: f64,
        gru_mutation_rate: f64,
        gru_mutation_std: f64,
        gru_crossover_mode: u8,
        snake_start_len: usize,
        fitness_survival_per_second: f64,
        fitness_food: f64,
        fitness_length_per_segment: f64,
        fitness_kill: f64,
        fitness_points_norm: f64,
        fitness_top_points_bonus: f64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureSource {
        generation: u64,
        best_fitness_ever: f64,
        evolution_rng: FixtureRng,
        snakes: Vec<FixtureSnake>,
        population: Vec<FixtureGenome>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureSnake {
        slot: u32,
        id: u64,
        age_seconds: f64,
        food: f64,
        points: f64,
        kills: u64,
        body_length: usize,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureGenome {
        #[serde(default)]
        slot: Option<u32>,
        fitness: f64,
        weight_bits: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureRng {
        algorithm: String,
        version: u32,
        state_hex: String,
        gaussian_algorithm: String,
        gaussian_version: u32,
        gaussian_spare_valid: bool,
        gaussian_spare_hex: Option<String>,
    }

    impl FixtureRng {
        fn serialized(&self) -> SerializedRngState {
            SerializedRngState {
                algorithm: self.algorithm.clone(),
                version: self.version,
                state_hex: self.state_hex.clone(),
                gaussian_algorithm: self.gaussian_algorithm.clone(),
                gaussian_version: self.gaussian_version,
                gaussian_spare_valid: self.gaussian_spare_valid,
                gaussian_spare_hex: self.gaussian_spare_hex.clone(),
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureExpected {
        boundary: FixtureBoundary,
        population: Vec<FixtureGenome>,
        history: FixtureHistory,
        hall_of_fame: FixtureHallOfFame,
        best_fitness_ever: f64,
    }

    #[derive(Deserialize)]
    struct FixtureBoundary {
        rng: FixtureRngBundle,
    }

    #[derive(Deserialize)]
    struct FixtureRngBundle {
        evolution: FixtureRng,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureHistory {
        gen: u64,
        best: f64,
        avg: f64,
        min: f64,
        species_count: u64,
        top_species_size: u64,
        avg_weight: f64,
        weight_variance: f64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureHallOfFame {
        gen: u64,
        seed: u64,
        fitness: f64,
        points: f64,
        length: usize,
        weight_bits: Vec<String>,
    }

    fn fixture() -> Fixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("evolution-reference.json");
        let json = std::fs::read_to_string(path).expect("retained evolution fixture must read");
        serde_json::from_str(&json).expect("retained evolution fixture must parse")
    }

    fn graph() -> CompiledGraph {
        let spec = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "features".to_owned(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 83,
                        hidden_sizes: vec![3],
                        output_size: 3,
                    },
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
                edge("input", "features"),
                edge("features", "memory"),
                edge("memory", "head"),
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        };
        compile_graph(
            &spec,
            &GraphLimits {
                max_nodes: 16,
                max_edges: 16,
                max_graph_outputs: 4,
                max_identifier_bytes: 64,
                max_total_referenced_identifier_bytes: 1_024,
                max_tensor_width: 256,
                max_mlp_hidden_layers: 8,
                max_split_output_ports: 8,
                max_parameter_floats: 10_000,
                max_recurrent_state_floats: 1_000,
                max_canonical_layout_bytes: 100_000,
                max_architecture_key_bytes: 4_096,
            },
        )
        .expect("fixture graph compiles")
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            from_port: None,
            to_port: None,
        }
    }

    fn config(source: &FixtureConfig, graph: &CompiledGraph, count: usize) -> EvolutionConfig {
        EvolutionConfig {
            elite_fraction: source.elite_frac,
            mutation_rate: source.mutation_rate,
            mutation_std: source.mutation_std,
            crossover_rate: source.crossover_rate,
            recurrent_mutation_rate: source.gru_mutation_rate,
            recurrent_mutation_std: source.gru_mutation_std,
            recurrent_crossover_mode: source.gru_crossover_mode,
            snake_start_length: source.snake_start_len,
            fitness_survival_per_second: source.fitness_survival_per_second,
            fitness_food: source.fitness_food,
            fitness_length_per_segment: source.fitness_length_per_segment,
            fitness_kill: source.fitness_kill,
            fitness_points_normalized: source.fitness_points_norm,
            fitness_top_points_bonus: source.fitness_top_points_bonus,
            maximum_population: count,
            maximum_output_weight_floats: count * graph.total_parameters,
        }
    }

    fn decode_weight_bits(values: &[String]) -> Vec<f32> {
        values
            .iter()
            .map(|value| {
                let bits =
                    u32::from_str_radix(value.strip_prefix("0x").expect("weight hex prefix"), 16)
                        .expect("weight hex value");
                f32::from_bits(bits)
            })
            .collect()
    }

    fn source_state(fixture: &Fixture) -> (WorldState, Vec<PopulationGenome>, SerializedRngState) {
        let mut population = Vec::with_capacity(fixture.source.population.len());
        for source in &fixture.source.population {
            let slot = source.slot.expect("source genome slot");
            population.push(PopulationGenome {
                slot,
                brain: BrainHandle {
                    id: u64::from(slot) + 1,
                    epoch: 1,
                },
                lineage: GenomeLineage {
                    genome_id: u64::from(slot) + 100,
                    birth_generation: 1,
                    parent_a: None,
                    parent_b: None,
                },
                fitness: source.fitness,
                weights: decode_weight_bits(&source.weight_bits).into_boxed_slice(),
            });
        }
        let mut body_points = Vec::new();
        let mut snakes = Vec::new();
        for source in &fixture.source.snakes {
            let start = body_points.len();
            body_points.extend((0..source.body_length).map(|offset| WorldPoint {
                x: f64::from(source.slot) * 100.0 - offset as f64 * 7.5,
                y: f64::from(source.slot) * 50.0,
            }));
            snakes.push(SnakeState {
                id: source.id,
                frame_v1_id: source.id as u32,
                kind: SnakeKind::Evolved,
                alive: true,
                population_slot: Some(source.slot),
                brain: Some(population[source.slot as usize].brain),
                baseline_slot: None,
                baseline_strategy: None,
                position: body_points[start],
                previous_position: body_points[start],
                direction: 0.0,
                radius: 9.0,
                speed: 165.0,
                boost: false,
                age_seconds: source.age_seconds,
                food: source.food,
                points: source.points,
                kills: source.kills,
                target_length: source.body_length as f64,
                fitness: 0.0,
                turn: 0.0,
                previous_turn: 0.0,
                input_boost: false,
                previous_input_boost: false,
                control_accumulator_seconds: 0.0,
                delivered_observation_points: 0.0,
                body: BodyRange {
                    start,
                    len: source.body_length,
                },
                skin: source.slot,
            });
        }
        let legacy = fixture.source.evolution_rng.serialized();
        let rng = StatefulRng::from_legacy_typescript_state(&legacy)
            .expect("fixture TypeScript RNG migrates")
            .export_state();
        (
            WorldState {
                snakes,
                body_points,
                pellets: Vec::new(),
                controller_leases: Vec::new(),
            },
            population,
            rng,
        )
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64, label: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{label}: actual {actual}, expected {expected}, tolerance {tolerance}"
        );
    }

    #[test]
    fn current_typescript_generation_fixture_matches_fitness_breeding_rng_and_hof() {
        let fixture = fixture();
        let graph = graph();
        assert_eq!(graph.total_parameters, 306);
        let (world, population, rng) = source_state(&fixture);
        let source_world = world.clone();
        let source_population = population.clone();
        let source_rng = rng.clone();
        let prepared = prepare_evolution(
            &world,
            &population,
            &graph,
            &rng,
            fixture.source.generation,
            fixture.source.best_fitness_ever,
            config(&fixture.evolution, &graph, population.len()),
        )
        .expect("fixture evolution prepares");

        assert_eq!(world, source_world);
        assert_eq!(population, source_population);
        assert_eq!(rng, source_rng);
        assert_eq!(prepared.sorted_source_slots(), &[3, 2, 1, 0]);
        assert_eq!(
            prepared.next_population().len(),
            fixture.expected.population.len()
        );
        for (expected_slot, (actual, expected)) in prepared
            .next_population()
            .iter()
            .zip(&fixture.expected.population)
            .enumerate()
        {
            assert_eq!(actual.slot as usize, expected_slot);
            assert_close(actual.fitness, expected.fitness, 1.0e-12, "next fitness");
            let expected_weights = decode_weight_bits(&expected.weight_bits);
            assert_eq!(actual.weights.len(), expected_weights.len());
            for (index, (actual, expected)) in
                actual.weights.iter().zip(expected_weights).enumerate()
            {
                assert!(
                    (actual - expected).abs() <= 1.0e-6,
                    "weight {index}: actual {actual}, expected {expected}"
                );
            }
        }
        let expected_rng = &fixture.expected.boundary.rng.evolution;
        assert_eq!(
            prepared.next_evolution_rng().state_hex,
            expected_rng.state_hex
        );
        assert_eq!(
            prepared.next_evolution_rng().gaussian_spare_valid,
            expected_rng.gaussian_spare_valid
        );
        let summary = prepared.summary();
        let expected = &fixture.expected.history;
        assert_eq!(summary.generation, expected.gen);
        assert_close(summary.best, expected.best, 1.0e-9, "best fitness");
        assert_close(summary.average, expected.avg, 1.0e-9, "average fitness");
        assert_close(summary.minimum, expected.min, 1.0e-9, "minimum fitness");
        assert_eq!(summary.species_count, expected.species_count);
        assert_eq!(summary.top_species_size, expected.top_species_size);
        assert_close(
            summary.average_weight,
            expected.avg_weight,
            1.0e-12,
            "average weight",
        );
        assert_close(
            summary.weight_variance,
            expected.weight_variance,
            1.0e-12,
            "weight variance",
        );
        let hall = prepared.hall_of_fame();
        let expected_hall = &fixture.expected.hall_of_fame;
        assert_eq!(hall.generation, expected_hall.gen);
        assert_eq!(hall.snake_id, expected_hall.seed);
        assert_close(hall.fitness, expected_hall.fitness, 1.0e-9, "HoF fitness");
        assert_close(hall.points, expected_hall.points, 0.0, "HoF points");
        assert_eq!(hall.length, expected_hall.length);
        assert_eq!(
            prepared.hall_of_fame_genome().weights.as_ref(),
            decode_weight_bits(&expected_hall.weight_bits)
        );
        assert_close(
            prepared.next_best_fitness_ever(),
            fixture.expected.best_fitness_ever,
            1.0e-9,
            "best-ever fitness",
        );
    }

    #[test]
    fn source_container_order_cannot_change_evolution_results() {
        let fixture = fixture();
        let graph = graph();
        let (world, population, rng) = source_state(&fixture);
        let mut reversed = world.clone();
        reversed.snakes.reverse();
        let configuration = config(&fixture.evolution, &graph, population.len());
        let canonical =
            prepare_evolution(&world, &population, &graph, &rng, 1, 0.0, configuration).unwrap();
        let reordered =
            prepare_evolution(&reversed, &population, &graph, &rng, 1, 0.0, configuration).unwrap();
        assert_eq!(
            canonical.sorted_source_slots(),
            reordered.sorted_source_slots()
        );
        assert_eq!(canonical.source_fitness(), reordered.source_fitness());
        assert_eq!(canonical.next_population(), reordered.next_population());
        assert_eq!(
            canonical.next_evolution_rng(),
            reordered.next_evolution_rng()
        );
        assert_eq!(canonical.summary(), reordered.summary());
        assert_eq!(canonical.hall_of_fame(), reordered.hall_of_fame());
    }

    #[test]
    fn equal_fitness_uses_stable_source_slot_order() {
        let fixture = fixture();
        let graph = graph();
        let (mut world, population, rng) = source_state(&fixture);
        for snake in &mut world.snakes {
            snake.age_seconds = 0.0;
            snake.food = 0.0;
            snake.points = 1.0;
            snake.kills = 0;
            snake.body.len = 5;
        }
        let prepared = prepare_evolution(
            &world,
            &population,
            &graph,
            &rng,
            1,
            0.0,
            config(&fixture.evolution, &graph, population.len()),
        )
        .unwrap();
        assert_eq!(prepared.sorted_source_slots(), &[0, 1, 2, 3]);
        assert_eq!(prepared.hall_of_fame().source_slot, 0);
    }

    #[test]
    fn invalid_work_or_fitness_fails_without_source_mutation() {
        let fixture = fixture();
        let graph = graph();
        let (mut world, population, rng) = source_state(&fixture);
        let original_population = population.clone();
        let original_rng = rng.clone();
        world.snakes[0].points = -1.0;
        prepare_evolution(
            &world,
            &population,
            &graph,
            &rng,
            1,
            0.0,
            config(&fixture.evolution, &graph, population.len()),
        )
        .expect("TypeScript clamps log(0) normalized points to zero");
        world.snakes[0].points = -2.0;
        let original_world = world.clone();
        let error = prepare_evolution(
            &world,
            &population,
            &graph,
            &rng,
            1,
            0.0,
            config(&fixture.evolution, &graph, population.len()),
        )
        .expect_err("logarithmically invalid points must reject");
        assert!(matches!(error, EvolutionError::NonFinite { .. }));
        assert_eq!(world, original_world);
        assert_eq!(population, original_population);
        assert_eq!(rng, original_rng);

        let mut too_small = config(&fixture.evolution, &graph, population.len());
        too_small.maximum_output_weight_floats -= 1;
        assert!(matches!(
            prepare_evolution(&world, &population, &graph, &rng, 1, 0.0, too_small),
            Err(EvolutionError::WeightLimit { .. })
        ));
    }

    #[test]
    fn draw_free_equal_genomes_form_separate_typescript_diagnostic_species() {
        let mut left = source_state(&fixture()).1.remove(0);
        let mut right = left.clone();
        left.weights = Box::new([]);
        right.weights = Box::new([]);
        assert!(genome_distance_rms(&left, &right).unwrap().is_nan());
    }

    #[test]
    fn recurrent_unit_crossover_selects_one_parent_for_every_gate_row_and_bias() {
        for (node_type, gates) in [
            (CompiledNodeType::Gru, 3usize),
            (CompiledNodeType::Lstm, 4usize),
            (CompiledNodeType::Rru, 2usize),
        ] {
            let input = 3usize;
            let hidden = 2usize;
            let length = gates * hidden * (input + hidden + 1);
            let node = CompiledNode {
                id: format!("{node_type:?}"),
                node_type,
                input_size: input,
                output_size: hidden,
                output_sizes: vec![hidden],
                hidden_sizes: Vec::new(),
                hidden_size: Some(hidden),
                parameter_offset: 0,
                parameter_length: length,
                state_offset: Some(0),
                state_length: if node_type == CompiledNodeType::Lstm {
                    hidden * 2
                } else {
                    hidden
                },
                inputs: Vec::new(),
            };
            let parent_a = (0..length)
                .map(|index| index as f32 + 1.0)
                .collect::<Vec<_>>();
            let parent_b = (0..length)
                .map(|index| -(index as f32 + 1.0))
                .collect::<Vec<_>>();
            let mut output = vec![0.0; length];
            let mut actual_rng = StatefulRng::new(0xabcddcba_u32 as f64);
            let mut expected_rng = StatefulRng::new(0xabcddcba_u32 as f64);
            crossover_recurrent(
                &node,
                &mut output,
                &parent_a,
                &parent_b,
                EvolutionConfig::typescript_defaults(),
                &mut actual_rng,
            )
            .unwrap();

            let input_block = hidden * input;
            let state_block = hidden * hidden;
            let state_start = gates * input_block;
            let bias_start = state_start + gates * state_block;
            for unit in 0..hidden {
                let expected = if expected_rng.next_f64() < 0.5 {
                    &parent_a
                } else {
                    &parent_b
                };
                for gate in 0..gates {
                    let input_start = gate * input_block + unit * input;
                    assert_eq!(
                        &output[input_start..input_start + input],
                        &expected[input_start..input_start + input]
                    );
                    let recurrent_start = state_start + gate * state_block + unit * hidden;
                    assert_eq!(
                        &output[recurrent_start..recurrent_start + hidden],
                        &expected[recurrent_start..recurrent_start + hidden]
                    );
                    let bias = bias_start + gate * hidden + unit;
                    assert_eq!(output[bias], expected[bias]);
                }
            }
            assert_eq!(actual_rng.export_state(), expected_rng.export_state());
        }
    }
}
