//! Strict projection of one admitted normalized configuration into hot-step settings.
//!
//! The temporary TypeScript runtime reads mutable global configuration from many
//! call sites. The Rust engine instead projects every running-step formula once
//! from the path-sorted authoritative settings record. Missing, wrongly typed,
//! unsupported, or internally inconsistent values reject the projection; no
//! gameplay value silently falls back to a Rust default.

use super::ambient::AmbientPelletConfig;
use super::baseline::BaselineLifecycleConfig;
use super::baseline_control::BaselineControlConfig;
use super::collision::CollisionConfig;
use super::control_phase::ControlPhaseConfig;
use super::controllers::ControllerTiming;
use super::effects::DeathDropConfig;
use super::evolution::EvolutionConfig;
use super::fixed_step::FixedStepPrefixConfig;
use super::food::FoodConfig;
use super::genome::GenomeInitializationConfig;
use super::movement::MovementConfig;
use super::physics::PhysicsConfig;
use super::sensors::{SensorConfig, SensorError};
use super::spatial::SensorIndexConfig;
use super::state::{
    NormalizedEngineConfig, NormalizedSetting, NormalizedSettingValue, SENSOR_VERSION,
};
use super::world_step::{WorldStepConfig, WorldStepError, MAXIMUM_PHYSICS_SUBSTEPS};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First strict normalized-settings-to-running-step projection contract.
pub const RUNNING_STEP_CONFIG_PROJECTION_VERSION: u32 = 1;
/// Current TypeScript collision-substep count ceiling.
const TYPESCRIPT_MAXIMUM_PHYSICS_SUBSTEPS: usize = 64;
/// Current TypeScript lower clamp for the configured maximum collision delta.
const TYPESCRIPT_MINIMUM_SUBSTEP_SECONDS: f64 = 0.001;
/// Slowest fixed-step rate admitted by the current server (`tickRateHz = 1`).
const TYPESCRIPT_MAXIMUM_FIXED_STEP_SECONDS: f64 = 1.0;
/// Fastest fixed-step rate admitted by the current server (`tickRateHz = 240`).
const TYPESCRIPT_MINIMUM_FIXED_STEP_SECONDS: f64 = 1.0 / 240.0;

/// Non-gameplay work ceilings retained separately from authoritative settings.
///
/// Hitting one of these limits rejects a step instead of changing collision,
/// sensing, or spawn truth. The defaults are provisional implementation limits
/// and remain subject to P0-P3 measurement; callers may supply reviewed larger
/// or smaller values without changing experiment configuration identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningStepWorkLimits {
    /// Complete body-cell entries admitted for shared sensing.
    pub sensor_body_entries: usize,
    /// Complete swept segment-to-cell entries admitted per collision substep.
    pub collision_index_entries: usize,
    /// Broad-phase cells admitted for one swept head query.
    pub collision_query_cells: usize,
    /// Random spawn candidates attempted for one due baseline slot.
    pub spawn_random_attempts_per_request: usize,
    /// Draw-free fallback positions considered for one due baseline slot.
    pub spawn_fallback_position_count: usize,
    /// Draw-free headings considered at each fallback position.
    pub spawn_fallback_heading_count: usize,
    /// Total random and fallback candidates admitted for one slot.
    pub spawn_candidates_per_request: usize,
    /// Total candidates admitted across all due baseline slots in one step.
    pub spawn_candidates_per_batch: usize,
    /// Complete wall/body geometry comparisons admitted across one respawn batch.
    pub spawn_geometry_checks_per_batch: usize,
}

impl RunningStepWorkLimits {
    /// Current reviewed safety ceilings, pending complete-step P0-P3 evidence.
    #[must_use]
    pub const fn provisional_defaults() -> Self {
        Self {
            sensor_body_entries: 1_000_000,
            collision_index_entries: 2_000_000,
            collision_query_cells: 262_144,
            spawn_random_attempts_per_request: 32,
            spawn_fallback_position_count: 2_048,
            spawn_fallback_heading_count: 16,
            spawn_candidates_per_request: 40_000,
            spawn_candidates_per_batch: 500_000,
            spawn_geometry_checks_per_batch: 10_000_000,
        }
    }
}

/// Build the complete current-TypeScript settings record for one explicit profile.
///
/// The first Stage 6A P0 constructor and deterministic fixtures use this exact
/// record. It is never used as a fallback after a supplied configuration fails
/// validation; every caller still hashes and admits the complete record.
pub(crate) fn typescript_default_settings(
    population_count: usize,
    baseline_count: usize,
) -> Vec<NormalizedSetting> {
    let integer = |path: &str, value: i64| NormalizedSetting {
        path: path.to_owned(),
        value: NormalizedSettingValue::Integer(value),
    };
    let float = |path: &str, value: f64| NormalizedSetting {
        path: path.to_owned(),
        value: NormalizedSettingValue::Float(value),
    };
    let boolean = |path: &str, value: bool| NormalizedSetting {
        path: path.to_owned(),
        value: NormalizedSettingValue::Bool(value),
    };
    let mut settings = vec![
        integer("baselineBots.count", baseline_count as i64),
        boolean("baselineBots.randomizeSeedPerGen", false),
        float("baselineBots.respawnDelay", 20.0),
        integer("baselineBots.seed", 1),
        float("boost.lenLossPerPoint", 0.16),
        float("boost.minPointsToBoost", 1.2),
        float("boost.pelletJitter", 10.0),
        float("boost.pelletValueFactor", 0.65),
        float("boost.pointsCostPerSecond", 7.0),
        float("boost.pointsCostSizeFactor", 1.1),
        float("brain.controlDt", 1.0 / 60.0),
        integer("brain.gruCrossoverMode", 1),
        float("brain.gruInitUpdateBias", -0.7),
        float("brain.gruMutationRate", 0.025),
        float("brain.gruMutationStd", 0.22),
        float("brain.lstmInitForgetBias", 0.6),
        float("brain.rruInitGateBias", 0.1),
        integer("brain.sensorVersion", 3),
        float("collision.cellSize", 70.0),
        float("collision.hitScale", 0.82),
        integer("collision.neighborRange", 1),
        integer("collision.skipSegments", 0),
        float("collision.substepMaxDt", 0.006),
        float("crossoverRate", 0.85),
        float("death.bigPelletValueFactor", 3.0),
        float("death.bigShare", 0.78),
        float("death.clusterJitter", 14.0),
        float("death.dropFracLarge", 0.33),
        float("death.dropFracPow", 1.6),
        float("death.dropFracSmall", 0.95),
        float("death.jitter", 8.0),
        integer("death.maxPellets", 420),
        float("death.smallPelletValueFactor", 1.0),
        boolean("death.useSnakeColor", true),
        boolean("foodSpawn.edgeFalloffEnabled", true),
        float("foodSpawn.edgeFadePower", 2.6),
        float("foodSpawn.edgeFadeStart", 0.35),
        float("foodSpawn.filamentPower", 4.2),
        float("foodSpawn.freqLarge", 0.0026),
        float("foodSpawn.freqMedium", 0.0042),
        float("foodSpawn.freqSmall", 0.0068),
        float("foodSpawn.dustStrength", 0.35),
        float("foodSpawn.warpFreq", 0.0013),
        float("foodSpawn.warpScale", 0.08),
        float("foodValue", 1.0),
        float("generationSeconds", 240.0),
        float("growPerFood", 1.0),
        float("eliteFrac", 0.12),
        float("mutationRate", 0.03),
        float("mutationStd", 0.35),
        integer("observer.earlyEndAliveThreshold", 2),
        float("observer.earlyEndMinSeconds", 8.0),
        float("pelletGrid.cellSize", 120.0),
        integer("pelletCountTarget", 3_500),
        float("pelletSpawnPerSecond", 170.0),
        float("reward.fitnessFood", 80.0),
        float("reward.fitnessKill", 400.0),
        float("reward.fitnessLengthPerSegment", 100.0),
        float("reward.fitnessPointsNorm", 42.0),
        float("reward.fitnessSurvivalPerSecond", 0.70),
        float("reward.fitnessTopPointsBonus", 600.0),
        float("reward.pointsPerFood", 20.0),
        float("reward.pointsPerKill", 400.0),
        float("reward.pointsPerSecondAlive", 0.6),
        integer("sense.bubbleBins", 16),
        float("sense.foodKBase", 4.0),
        integer("sense.layoutVersion", 3),
        integer("sense.maxPelletChecks", 900),
        integer("sense.maxSegmentChecks", 2_200),
        float("sense.rFarBase", 1_200.0),
        float("sense.rFarMax", 2_400.0),
        float("sense.rFarMin", 900.0),
        float("sense.rFarScale", 520.0),
        float("sense.rNearBase", 520.0),
        float("sense.rNearMax", 1_100.0),
        float("sense.rNearMin", 420.0),
        float("sense.rNearScale", 260.0),
        float("simSpeed", 1.0),
        float("snakeBaseSpeed", 165.0),
        float("snakeBoostSizePenalty", 0.28),
        float("snakeBoostSpeed", 500.0),
        integer("snakeCount", population_count as i64),
        integer("snakeMaxLen", 10_000),
        integer("snakeMinLen", 4),
        float("snakeRadius", 9.0),
        float("snakeRadiusMax", 18.0),
        float("snakeSizeSpeedPenalty", 0.18),
        float("snakeSpacing", 7.5),
        integer("snakeStartLen", 5),
        float("snakeThicknessLogDiv", 30.0),
        float("snakeThicknessScale", 2.9),
        float("snakeTurnPenalty", 1.4),
        float("snakeTurnRate", 3.2),
        integer("worldRadius", 3_500),
    ];
    settings.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    settings
}

/// Preserve the named test-fixture helper while sharing the production profile record.
#[cfg(any(test, feature = "engine-test-hooks"))]
pub(crate) fn fixture_default_settings(
    population_count: usize,
    baseline_count: usize,
) -> Vec<NormalizedSetting> {
    typescript_default_settings(population_count, baseline_count)
}

/// Complete immutable settings needed to construct one running-step pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningStepConfigProjection {
    /// Prefix, control, physics, lifecycle, and substep settings.
    pub world_step: WorldStepConfig,
    /// Exact corrected sensor-v3 formula configuration owned by the control pipeline.
    pub sensor: SensorConfig,
    /// Values that prove whether a completed step is safely nonterminal.
    pub generation_guard: GenerationGuardConfig,
    /// Reset-only random genome initialization formulas needed by replacement/evolution.
    pub genome_initialization: GenomeInitializationConfig,
}

/// Current TypeScript generation-ending conditions used as a fail-closed guard.
///
/// Stage 6 owns evolution and the actual transition. Until then, the running
/// coordinator may publish only a step for which neither condition is true.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenerationGuardConfig {
    /// Ordinary configured duration of one generation.
    pub generation_seconds: f64,
    /// Earliest simulated generation time at which the alive-count rule applies.
    pub early_end_minimum_seconds: f64,
    /// Maximum alive evolved population members that triggers early end.
    pub early_end_alive_threshold: usize,
}

/// Reset-only values that derive stable per-baseline RNG streams each generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaselineGenerationConfig {
    /// Owner-configured Uint32 base seed.
    pub seed: u32,
    /// Whether the generation number participates in the per-slot seed.
    pub randomize_seed_per_generation: bool,
}

/// Project the current TypeScript baseline-generation seed contract.
pub(crate) fn project_baseline_generation_config(
    config: &NormalizedEngineConfig,
) -> Result<BaselineGenerationConfig, StepConfigError> {
    let settings = SettingReader::new(config)?;
    let configured_seed = settings.integer("baselineBots.seed", 0, i64::from(u32::MAX))?;
    let seed = u32::try_from(configured_seed).map_err(|_| StepConfigError::ArithmeticOverflow {
        context: "baseline generation seed",
    })?;
    Ok(BaselineGenerationConfig {
        seed,
        randomize_seed_per_generation: settings.boolean("baselineBots.randomizeSeedPerGen")?,
    })
}

/// Project every selection-pressure value from one admitted normalized run.
///
/// The output ceiling is the exact dense population multiplied by the
/// compiled graph parameter count. No Rust default may replace a missing,
/// mistyped, or out-of-range owner setting.
pub(crate) fn project_evolution_config(
    config: &NormalizedEngineConfig,
    graph_parameter_count: usize,
) -> Result<EvolutionConfig, StepConfigError> {
    let settings = SettingReader::new(config)?;
    let population_count = settings.usize("snakeCount", 1, 300)?;
    if population_count != config.population_count {
        return Err(StepConfigError::ProjectionMismatch { path: "snakeCount" });
    }
    let maximum_output_weight_floats = population_count.checked_mul(graph_parameter_count).ok_or(
        StepConfigError::ArithmeticOverflow {
            context: "complete evolved population weights",
        },
    )?;
    if maximum_output_weight_floats == 0 {
        return Err(StepConfigError::InvalidEvolution);
    }
    let recurrent_crossover_mode =
        u8::try_from(settings.integer("brain.gruCrossoverMode", 0, 1)?).map_err(|_| {
            StepConfigError::ArithmeticOverflow {
                context: "recurrent crossover mode",
            }
        })?;
    let projected = EvolutionConfig {
        elite_fraction: settings.float("eliteFrac", 0.01, 0.5)?,
        mutation_rate: settings.float("mutationRate", 0.0, 0.5)?,
        mutation_std: settings.float("mutationStd", 0.0, 2.5)?,
        crossover_rate: settings.float("crossoverRate", 0.0, 1.0)?,
        recurrent_mutation_rate: settings.float("brain.gruMutationRate", 0.0, 0.35)?,
        recurrent_mutation_std: settings.float("brain.gruMutationStd", 0.0, 1.6)?,
        recurrent_crossover_mode,
        snake_start_length: settings.usize("snakeStartLen", 5, 140)?,
        fitness_survival_per_second: settings.float(
            "reward.fitnessSurvivalPerSecond",
            0.0,
            10.0,
        )?,
        fitness_food: settings.float("reward.fitnessFood", 0.0, 80.0)?,
        fitness_length_per_segment: settings.float("reward.fitnessLengthPerSegment", 0.0, 100.0)?,
        fitness_kill: settings.float("reward.fitnessKill", 0.0, 400.0)?,
        fitness_points_normalized: settings.float("reward.fitnessPointsNorm", 0.0, 300.0)?,
        fitness_top_points_bonus: settings.float("reward.fitnessTopPointsBonus", 0.0, 600.0)?,
        maximum_population: population_count,
        maximum_output_weight_floats,
    };
    projected
        .validate()
        .map_err(|_| StepConfigError::InvalidEvolution)?;
    Ok(projected)
}

/// Project every current running-step input from one normalized authority.
///
/// Values represented in the TypeScript `CFG` object but not exposed as UI
/// settings (`pelletGrid.cellSize`, `snakeTurnPenalty`, and `death.*`) are still
/// required because they alter authoritative behavior. The future Node/Rust
/// construction boundary must therefore normalize the complete configuration,
/// not only the browser slider list.
pub(crate) fn project_running_step_config(
    config: &NormalizedEngineConfig,
    limits: RunningStepWorkLimits,
) -> Result<RunningStepConfigProjection, StepConfigError> {
    let settings = SettingReader::new(config)?;
    let fixed_dt = checked_top_level_float(
        "fixed_step_seconds",
        config.fixed_step_seconds,
        TYPESCRIPT_MINIMUM_FIXED_STEP_SECONDS,
        TYPESCRIPT_MAXIMUM_FIXED_STEP_SECONDS,
    )?;
    let requested_sim_speed = settings.float("simSpeed", 0.1, 12.0)?;
    if requested_sim_speed.to_bits() != config.requested_sim_speed.to_bits() {
        return Err(StepConfigError::ProjectionMismatch { path: "simSpeed" });
    }
    let world_radius_setting = settings.integer("worldRadius", 800, 10_000)? as f64;
    if world_radius_setting.to_bits() != config.world_radius.to_bits() {
        return Err(StepConfigError::ProjectionMismatch {
            path: "worldRadius",
        });
    }
    let population_count = settings.usize("snakeCount", 1, 300)?;
    if population_count != config.population_count {
        return Err(StepConfigError::ProjectionMismatch { path: "snakeCount" });
    }
    let baseline_count = settings.usize("baselineBots.count", 0, 120)?;
    if baseline_count != config.baseline_count {
        return Err(StepConfigError::ProjectionMismatch {
            path: "baselineBots.count",
        });
    }
    if settings.integer("brain.sensorVersion", 3, 3)? != i64::from(SENSOR_VERSION)
        || settings.integer("sense.layoutVersion", 3, 3)? != i64::from(SENSOR_VERSION)
    {
        return Err(StepConfigError::ProjectionMismatch {
            path: "sense.layoutVersion",
        });
    }

    let food_value = settings.float("foodValue", 0.1, 8.0)?;
    let growth_per_food = settings.float("growPerFood", 0.1, 10.0)?;
    let snake_base_speed = settings.float("snakeBaseSpeed", 30.0, 650.0)?;
    let snake_boost_speed = settings.float("snakeBoostSpeed", 40.0, 1_200.0)?;
    let snake_radius = settings.float("snakeRadius", 3.0, 30.0)?;
    let snake_spacing = settings.float("snakeSpacing", 3.0, 20.0)?;
    let snake_start_len = settings.usize("snakeStartLen", 5, 140)?;
    let snake_max_len = settings.usize("snakeMaxLen", 60, 100_000)?;
    let snake_min_len = settings.usize("snakeMinLen", 4, 80)?;
    let boost_min_points = settings.float("boost.minPointsToBoost", 0.0, 60.0)?;
    let boost_cost = settings.float("boost.pointsCostPerSecond", 0.0, 80.0)?;
    let boost_size_cost = settings.float("boost.pointsCostSizeFactor", 0.0, 4.0)?;
    let collision_cell_size = settings.float("collision.cellSize", 20.0, 200.0)?;
    let collision_hit_scale = settings.float("collision.hitScale", 0.45, 1.2)?;
    let pellet_cell_size = settings.positive_float("pelletGrid.cellSize")?;

    let mut accounting = super::accounting::StepAccountingConfig::typescript_defaults();
    accounting.points_per_second_alive =
        settings.float("reward.pointsPerSecondAlive", 0.0, 10.0)?;

    let mut ambient = AmbientPelletConfig::typescript_defaults();
    ambient.target_count = settings.usize("pelletCountTarget", 100, 25_000)?;
    ambient.spawn_per_second = settings.float("pelletSpawnPerSecond", 5.0, 3_500.0)?;
    ambient.world_radius = config.world_radius;
    ambient.food_value = food_value;
    ambient.edge_falloff_enabled = settings.boolean("foodSpawn.edgeFalloffEnabled")?;
    ambient.edge_fade_start = settings.float("foodSpawn.edgeFadeStart", 0.05, 0.85)?;
    ambient.edge_fade_power = settings.float("foodSpawn.edgeFadePower", 1.0, 6.0)?;
    ambient.filament_power = settings.float("foodSpawn.filamentPower", 1.5, 8.0)?;
    ambient.warp_frequency = settings.float("foodSpawn.warpFreq", 0.0003, 0.003)?;
    ambient.warp_scale_fraction = settings.float("foodSpawn.warpScale", 0.0, 0.2)?;
    ambient.large_frequency = settings.float("foodSpawn.freqLarge", 0.001, 0.006)?;
    ambient.medium_frequency = settings.float("foodSpawn.freqMedium", 0.0015, 0.01)?;
    ambient.small_frequency = settings.float("foodSpawn.freqSmall", 0.0025, 0.02)?;
    ambient.dust_strength = settings.float("foodSpawn.dustStrength", 0.0, 1.0)?;

    let baseline = BaselineLifecycleConfig {
        slot_count: baseline_count,
        respawn_delay_seconds: settings.float("baselineBots.respawnDelay", 0.5, 60.0)?,
        ..BaselineLifecycleConfig::typescript_defaults()
    };

    let mut baseline_spawn = super::spawn::SpawnConfig::typescript_geometry_defaults();
    baseline_spawn.world_radius = config.world_radius;
    baseline_spawn.snake_radius = snake_radius;
    baseline_spawn.snake_spacing = snake_spacing;
    baseline_spawn.snake_start_len = snake_start_len;
    baseline_spawn.random_attempts_per_request = limits.spawn_random_attempts_per_request;
    baseline_spawn.fallback_position_count = limits.spawn_fallback_position_count;
    baseline_spawn.fallback_heading_count = limits.spawn_fallback_heading_count;
    baseline_spawn.maximum_candidates_per_request = limits.spawn_candidates_per_request;
    baseline_spawn.maximum_candidates_per_batch = limits.spawn_candidates_per_batch;
    baseline_spawn.maximum_geometry_checks_per_batch = limits.spawn_geometry_checks_per_batch;

    let prefix = FixedStepPrefixConfig {
        fixed_dt,
        accounting,
        ambient,
        baseline,
        baseline_spawn,
        baseline_snake_base_speed: snake_base_speed,
        maximum_snakes: config.max_world_snakes,
        maximum_body_points: config.max_body_points,
        maximum_pellets: config.max_pellets,
        ..FixedStepPrefixConfig::typescript_defaults()
    };

    let maximum_brains = config
        .population_count
        .checked_add(config.max_non_population_brains)
        .ok_or(StepConfigError::ArithmeticOverflow {
            context: "maximum running brain records",
        })?;
    let controller_timing = ControllerTiming::from_config(config)
        .map_err(|_| StepConfigError::InvalidControllerTiming)?;
    let control = ControlPhaseConfig {
        neural_control_interval_seconds: settings.float("brain.controlDt", 0.008, 0.06)?,
        controller_timing,
        baseline: BaselineControlConfig {
            minimum_points_to_boost: boost_min_points,
            maximum_world_snakes: config.max_world_snakes,
            ..BaselineControlConfig::typescript_defaults()
        },
        sensor_index: SensorIndexConfig {
            body_cell_size: collision_cell_size,
            pellet_cell_size,
            maximum_body_entries: limits.sensor_body_entries,
            maximum_pellet_entries: config.max_pellets,
        },
        maximum_snakes: config.max_world_snakes,
        maximum_brains,
        maximum_external_observations: config.max_world_snakes,
        ..ControlPhaseConfig::typescript_defaults()
    };

    let movement = MovementConfig {
        world_radius: config.world_radius,
        snake_base_speed,
        snake_boost_speed,
        snake_turn_rate: settings.float("snakeTurnRate", 0.4, 14.0)?,
        snake_radius,
        snake_radius_max: settings.float("snakeRadiusMax", 4.0, 50.0)?,
        snake_thickness_scale: settings.float("snakeThicknessScale", 0.0, 20.0)?,
        snake_thickness_log_div: settings.float("snakeThicknessLogDiv", 1.0, 240.0)?,
        snake_spacing,
        snake_start_len,
        snake_max_len,
        snake_min_len,
        snake_size_speed_penalty: settings.float("snakeSizeSpeedPenalty", 0.0, 0.7)?,
        snake_boost_size_penalty: settings.float("snakeBoostSizePenalty", 0.0, 0.95)?,
        snake_turn_penalty: settings.nonnegative_float("snakeTurnPenalty")?,
        boost_min_points,
        boost_points_cost_per_second: boost_cost,
        boost_points_cost_size_factor: boost_size_cost,
        boost_len_loss_per_point: settings.float("boost.lenLossPerPoint", 0.0, 2.0)?,
        food_value,
        boost_pellet_value_factor: settings.float("boost.pelletValueFactor", 0.0, 1.5)?,
        boost_pellet_jitter: settings.float("boost.pelletJitter", 0.0, 80.0)?,
    };
    let food = FoodConfig {
        eat_radius_padding: FoodConfig::typescript_defaults().eat_radius_padding,
        points_per_food: settings.float("reward.pointsPerFood", 0.0, 20.0)?,
        growth_per_food,
    };
    let collision = CollisionConfig {
        cell_size: collision_cell_size,
        hit_scale: collision_hit_scale,
        skip_segments: settings.usize("collision.skipSegments", 0, 30)?,
        maximum_index_entries: limits.collision_index_entries,
        maximum_query_cells: limits.collision_query_cells,
    };
    // The corrected swept-cell implementation does not use the old neighbor
    // stencil, but the setting remains required and range-checked for config
    // compatibility until that obsolete input is retired explicitly.
    let _neighbor_range = settings.usize("collision.neighborRange", 1, 3)?;

    let death = DeathDropConfig {
        drop_fraction_small: settings.nonnegative_float("death.dropFracSmall")?,
        drop_fraction_large: settings.nonnegative_float("death.dropFracLarge")?,
        drop_fraction_power: settings.nonnegative_float("death.dropFracPow")?,
        big_pellet_value_factor: settings.nonnegative_float("death.bigPelletValueFactor")?,
        small_pellet_value_factor: settings.nonnegative_float("death.smallPelletValueFactor")?,
        big_share: settings.nonnegative_float("death.bigShare")?,
        jitter: settings.nonnegative_float("death.jitter")?,
        cluster_jitter: settings.nonnegative_float("death.clusterJitter")?,
        maximum_pellets: settings.usize("death.maxPellets", 1, config.max_pellets)?,
    };
    if !settings.boolean("death.useSnakeColor")? {
        return Err(StepConfigError::UnsupportedSetting {
            path: "death.useSnakeColor",
        });
    }

    let configured_max_substep = settings.float("collision.substepMaxDt", 0.006, 0.05)?;
    let clamped_max_substep = configured_max_substep
        .max(TYPESCRIPT_MINIMUM_SUBSTEP_SECONDS)
        .min(fixed_dt);
    let requested_substeps = (fixed_dt / clamped_max_substep).ceil() as usize;
    let physics_substeps = requested_substeps
        .clamp(1, TYPESCRIPT_MAXIMUM_PHYSICS_SUBSTEPS)
        .min(MAXIMUM_PHYSICS_SUBSTEPS);
    let physics = PhysicsConfig {
        movement,
        food,
        collision,
        death,
        pellet_index_cell_size: pellet_cell_size,
        maximum_pellet_index_entries: config.max_pellets,
        substep_dt: fixed_dt / physics_substeps as f64,
        maximum_body_points: config.max_body_points,
        maximum_pellets: config.max_pellets,
        points_per_kill: settings.float("reward.pointsPerKill", 0.0, 400.0)?,
    };

    let genome_initialization = GenomeInitializationConfig {
        gru_update_bias: settings.float("brain.gruInitUpdateBias", -2.5, 1.5)?,
        lstm_forget_bias: settings.float("brain.lstmInitForgetBias", -1.5, 3.0)?,
        rru_gate_bias: settings.float("brain.rruInitGateBias", -1.5, 2.0)?,
    };
    genome_initialization
        .validate()
        .map_err(|_| StepConfigError::InvalidGenomeInitialization)?;
    let external_replacement = super::external_replacement::ExternalReplacementConfig {
        spawn: baseline_spawn,
        genome: genome_initialization,
        snake_base_speed,
        controller_timing,
        maximum_snakes: config.max_world_snakes,
        maximum_body_points: config.max_body_points,
        maximum_brains,
        ..super::external_replacement::ExternalReplacementConfig::typescript_defaults()
    };

    let world_step = WorldStepConfig {
        prefix,
        control,
        physics,
        baseline,
        external_replacement,
        physics_substeps,
        ..WorldStepConfig::typescript_defaults()
    };
    world_step.validate_shape()?;

    let generation_seconds = settings.float("generationSeconds", 8.0, 480.0)?;
    let sensor = SensorConfig {
        bins: settings.usize("sense.bubbleBins", 8, 32)?,
        world_radius: config.world_radius,
        food_value,
        snake_boost_speed,
        snake_start_length: snake_start_len,
        snake_max_length: snake_max_len,
        minimum_boost_points: boost_min_points,
        boost_points_cost_per_second: boost_cost,
        boost_points_cost_size_factor: boost_size_cost,
        generation_seconds,
        near_radius_base: settings.float("sense.rNearBase", 200.0, 900.0)?,
        near_radius_scale: settings.float("sense.rNearScale", 0.0, 600.0)?,
        near_radius_minimum: settings.float("sense.rNearMin", 150.0, 900.0)?,
        near_radius_maximum: settings.float("sense.rNearMax", 200.0, 1_200.0)?,
        far_radius_base: settings.float("sense.rFarBase", 400.0, 2_000.0)?,
        far_radius_scale: settings.float("sense.rFarScale", 0.0, 1_200.0)?,
        far_radius_minimum: settings.float("sense.rFarMin", 400.0, 2_200.0)?,
        far_radius_maximum: settings.float("sense.rFarMax", 600.0, 3_000.0)?,
        food_saturation: settings.float("sense.foodKBase", 0.5, 12.0)?,
        collision_hit_scale,
        maximum_pellet_checks: settings.usize("sense.maxPelletChecks", 100, 3_000)?,
        maximum_segment_checks: settings.usize("sense.maxSegmentChecks", 200, 4_000)?,
    };
    sensor.validate()?;

    let generation_guard = GenerationGuardConfig {
        generation_seconds,
        early_end_minimum_seconds: settings.float("observer.earlyEndMinSeconds", 0.0, 50.0)?,
        early_end_alive_threshold: settings.usize("observer.earlyEndAliveThreshold", 1, 25)?,
    };
    Ok(RunningStepConfigProjection {
        world_step,
        sensor,
        generation_guard,
        genome_initialization,
    })
}

fn checked_top_level_float(
    path: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<f64, StepConfigError> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(StepConfigError::OutOfRange { path });
    }
    Ok(value)
}

struct SettingReader<'config> {
    config: &'config NormalizedEngineConfig,
}

impl<'config> SettingReader<'config> {
    fn new(config: &'config NormalizedEngineConfig) -> Result<Self, StepConfigError> {
        if config
            .settings
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
        {
            return Err(StepConfigError::NonCanonicalSettings);
        }
        Ok(Self { config })
    }

    fn value(&self, path: &'static str) -> Result<&NormalizedSettingValue, StepConfigError> {
        self.config
            .settings
            .binary_search_by(|setting| setting.path.as_str().cmp(path))
            .ok()
            .and_then(|index| self.config.settings.get(index))
            .map(|setting| &setting.value)
            .ok_or(StepConfigError::MissingSetting { path })
    }

    fn boolean(&self, path: &'static str) -> Result<bool, StepConfigError> {
        let NormalizedSettingValue::Bool(value) = self.value(path)? else {
            return Err(StepConfigError::WrongSettingType {
                path,
                expected: "boolean",
            });
        };
        Ok(*value)
    }

    fn integer(
        &self,
        path: &'static str,
        minimum: i64,
        maximum: i64,
    ) -> Result<i64, StepConfigError> {
        let NormalizedSettingValue::Integer(value) = self.value(path)? else {
            return Err(StepConfigError::WrongSettingType {
                path,
                expected: "integer",
            });
        };
        if !(minimum..=maximum).contains(value) {
            return Err(StepConfigError::OutOfRange { path });
        }
        Ok(*value)
    }

    fn usize(
        &self,
        path: &'static str,
        minimum: usize,
        maximum: usize,
    ) -> Result<usize, StepConfigError> {
        let minimum_i64 =
            i64::try_from(minimum).map_err(|_| StepConfigError::ArithmeticOverflow {
                context: "setting lower bound",
            })?;
        let maximum_i64 =
            i64::try_from(maximum).map_err(|_| StepConfigError::ArithmeticOverflow {
                context: "setting upper bound",
            })?;
        usize::try_from(self.integer(path, minimum_i64, maximum_i64)?).map_err(|_| {
            StepConfigError::ArithmeticOverflow {
                context: "integer setting projection",
            }
        })
    }

    fn float(
        &self,
        path: &'static str,
        minimum: f64,
        maximum: f64,
    ) -> Result<f64, StepConfigError> {
        let NormalizedSettingValue::Float(value) = self.value(path)? else {
            return Err(StepConfigError::WrongSettingType {
                path,
                expected: "floating",
            });
        };
        if !value.is_finite() || !(minimum..=maximum).contains(value) {
            return Err(StepConfigError::OutOfRange { path });
        }
        Ok(*value)
    }

    fn nonnegative_float(&self, path: &'static str) -> Result<f64, StepConfigError> {
        let NormalizedSettingValue::Float(value) = self.value(path)? else {
            return Err(StepConfigError::WrongSettingType {
                path,
                expected: "floating",
            });
        };
        if !value.is_finite() || *value < 0.0 {
            return Err(StepConfigError::OutOfRange { path });
        }
        Ok(*value)
    }

    fn positive_float(&self, path: &'static str) -> Result<f64, StepConfigError> {
        let value = self.nonnegative_float(path)?;
        if value == 0.0 {
            return Err(StepConfigError::OutOfRange { path });
        }
        Ok(value)
    }
}

/// Rejection while deriving the complete running-step formula contract.
#[derive(Debug)]
pub enum StepConfigError {
    /// The normalized settings vector is not strictly path-sorted and unique.
    NonCanonicalSettings,
    /// One correctness-relevant running-step setting is absent.
    MissingSetting { path: &'static str },
    /// One normalized setting uses a different scalar representation.
    WrongSettingType {
        path: &'static str,
        expected: &'static str,
    },
    /// One value is non-finite or outside the admitted product range.
    OutOfRange { path: &'static str },
    /// A duplicate top-level/config-map projection disagrees.
    ProjectionMismatch { path: &'static str },
    /// The current Rust implementation cannot preserve this admitted value.
    UnsupportedSetting { path: &'static str },
    /// Owner-selected controller wall-time values are inconsistent.
    InvalidControllerTiming,
    /// Random genome reset-only values are internally inconsistent.
    InvalidGenomeInitialization,
    /// Evolution settings or their exact output bounds are inconsistent.
    InvalidEvolution,
    /// Checked projection arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// The assembled fixed-step phases reject the resulting shape.
    WorldStep(Box<WorldStepError>),
    /// The assembled corrected sensor-v3 configuration is invalid.
    Sensor(Box<SensorError>),
}

impl From<WorldStepError> for StepConfigError {
    fn from(error: WorldStepError) -> Self {
        Self::WorldStep(Box::new(error))
    }
}

impl From<SensorError> for StepConfigError {
    fn from(error: SensorError) -> Self {
        Self::Sensor(Box::new(error))
    }
}

impl Display for StepConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonCanonicalSettings => {
                write!(
                    formatter,
                    "normalized settings are not strictly path-sorted"
                )
            }
            Self::MissingSetting { path } => {
                write!(formatter, "missing running-step setting {path}")
            }
            Self::WrongSettingType { path, expected } => {
                write!(formatter, "running-step setting {path} must be {expected}")
            }
            Self::OutOfRange { path } => {
                write!(
                    formatter,
                    "running-step setting {path} is outside its admitted range"
                )
            }
            Self::ProjectionMismatch { path } => {
                write!(
                    formatter,
                    "running-step setting {path} disagrees with authoritative config"
                )
            }
            Self::UnsupportedSetting { path } => {
                write!(
                    formatter,
                    "running-step setting {path} is not supported by this engine"
                )
            }
            Self::InvalidControllerTiming => {
                write!(formatter, "invalid controller wall-time settings")
            }
            Self::InvalidGenomeInitialization => {
                write!(formatter, "invalid random genome initialization settings")
            }
            Self::InvalidEvolution => {
                write!(formatter, "invalid generation-evolution settings")
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "running-step config overflow while calculating {context}"
                )
            }
            Self::WorldStep(error) => {
                write!(formatter, "running-step phase config failed: {error}")
            }
            Self::Sensor(error) => write!(formatter, "running-step sensor config failed: {error}"),
        }
    }
}

impl Error for StepConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorldStep(error) => Some(error.as_ref()),
            Self::Sensor(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::state::{NormalizedSetting, NORMALIZED_CONFIG_VERSION};

    pub(crate) fn default_settings(
        population_count: usize,
        baseline_count: usize,
    ) -> Vec<NormalizedSetting> {
        fixture_default_settings(population_count, baseline_count)
    }

    fn config() -> NormalizedEngineConfig {
        NormalizedEngineConfig {
            version: NORMALIZED_CONFIG_VERSION,
            settings: default_settings(55, 10),
            settings_schema_sha256: "sha256:test".to_owned(),
            graph_architecture_key: "graph:test".to_owned(),
            fixed_step_seconds: 1.0 / 60.0,
            requested_sim_speed: 1.0,
            world_radius: 3_500.0,
            population_count: 55,
            baseline_count: 10,
            max_world_snakes: 512,
            max_non_population_brains: 447,
            max_body_points: 100_000,
            max_pellets: 200_000,
            spatial_index_bytes: 64 * 1024 * 1024,
            worker_scratch_bytes: 64 * 1024 * 1024,
            checkpoint_scratch_bytes: 64 * 1024 * 1024,
            controller_input_hold_ms: 500,
            controller_disconnect_grace_ms: 30_000,
        }
    }

    fn setting_mut<'a>(
        config: &'a mut NormalizedEngineConfig,
        path: &str,
    ) -> &'a mut NormalizedSettingValue {
        &mut config
            .settings
            .iter_mut()
            .find(|setting| setting.path == path)
            .expect("test setting must exist")
            .value
    }

    #[test]
    fn default_projection_matches_every_running_typescript_formula() {
        let projected =
            project_running_step_config(&config(), RunningStepWorkLimits::provisional_defaults())
                .expect("complete default configuration must project");
        let world = projected.world_step;
        assert_eq!(world.physics_substeps, 3);
        assert_eq!(world.prefix.fixed_dt.to_bits(), (1.0_f64 / 60.0).to_bits());
        assert_eq!(
            world.prefix.accounting,
            crate::engine::accounting::StepAccountingConfig::typescript_defaults()
        );
        assert_eq!(
            world.prefix.ambient,
            AmbientPelletConfig::typescript_defaults()
        );
        assert_eq!(
            world.prefix.baseline,
            BaselineLifecycleConfig::typescript_defaults()
        );
        assert_eq!(
            world.prefix.baseline_spawn,
            crate::engine::spawn::SpawnConfig::typescript_geometry_defaults()
        );
        assert_eq!(
            world.control.baseline,
            BaselineControlConfig::typescript_defaults()
        );
        assert_eq!(
            world.control.sensor_index,
            ControlPhaseConfig::typescript_defaults().sensor_index
        );
        assert_eq!(
            world.physics.movement,
            MovementConfig::typescript_defaults()
        );
        assert_eq!(world.physics.food, FoodConfig::typescript_defaults());
        assert_eq!(
            world.physics.collision,
            CollisionConfig::typescript_defaults()
        );
        assert_eq!(world.physics.death, DeathDropConfig::typescript_defaults());
        assert_eq!(world.physics.pellet_index_cell_size, 120.0);
        assert_eq!(
            world.physics.substep_dt.to_bits(),
            (1.0_f64 / 180.0).to_bits()
        );
        assert_eq!(world.physics.points_per_kill, 400.0);
        assert_eq!(projected.sensor, SensorConfig::default());
        assert_eq!(
            projected.generation_guard,
            GenerationGuardConfig {
                generation_seconds: 240.0,
                early_end_minimum_seconds: 8.0,
                early_end_alive_threshold: 2,
            }
        );
        assert_eq!(world.control.controller_timing.input_hold_ms(), 500);
        assert_eq!(
            world.control.controller_timing.disconnect_grace_ms(),
            30_000
        );
        assert_eq!(world.prefix.maximum_pellets, 200_000);
        assert_eq!(world.control.maximum_brains, 502);
        assert_eq!(
            projected.genome_initialization,
            GenomeInitializationConfig::typescript_defaults()
        );

        let evolution = project_evolution_config(&config(), 123)
            .expect("complete default evolution configuration must project");
        assert_eq!(
            evolution,
            EvolutionConfig {
                maximum_population: 55,
                maximum_output_weight_floats: 55 * 123,
                ..EvolutionConfig::typescript_defaults()
            }
        );
        assert_eq!(
            project_baseline_generation_config(&config())
                .expect("complete default baseline generation configuration must project"),
            BaselineGenerationConfig {
                seed: 1,
                randomize_seed_per_generation: false,
            }
        );
    }

    #[test]
    fn baseline_generation_seed_values_are_required_and_exact() {
        let mut changed = config();
        *setting_mut(&mut changed, "baselineBots.seed") =
            NormalizedSettingValue::Integer(i64::from(u32::MAX));
        *setting_mut(&mut changed, "baselineBots.randomizeSeedPerGen") =
            NormalizedSettingValue::Bool(true);
        assert_eq!(
            project_baseline_generation_config(&changed)
                .expect("changed baseline generation values must project"),
            BaselineGenerationConfig {
                seed: u32::MAX,
                randomize_seed_per_generation: true,
            }
        );

        let mut missing = config();
        missing
            .settings
            .retain(|setting| setting.path != "baselineBots.seed");
        assert!(matches!(
            project_baseline_generation_config(&missing),
            Err(StepConfigError::MissingSetting {
                path: "baselineBots.seed"
            })
        ));

        let mut wrong_type = config();
        *setting_mut(&mut wrong_type, "baselineBots.randomizeSeedPerGen") =
            NormalizedSettingValue::Integer(0);
        assert!(matches!(
            project_baseline_generation_config(&wrong_type),
            Err(StepConfigError::WrongSettingType {
                path: "baselineBots.randomizeSeedPerGen",
                ..
            })
        ));
    }

    #[test]
    fn evolution_values_are_required_and_never_fall_back_to_rust_defaults() {
        let mut changed = config();
        *setting_mut(&mut changed, "eliteFrac") = NormalizedSettingValue::Float(0.25);
        *setting_mut(&mut changed, "brain.gruCrossoverMode") = NormalizedSettingValue::Integer(0);
        *setting_mut(&mut changed, "reward.fitnessKill") = NormalizedSettingValue::Float(123.0);
        let projected = project_evolution_config(&changed, 306)
            .expect("changed admitted evolution values must project");
        assert_eq!(projected.elite_fraction, 0.25);
        assert_eq!(projected.recurrent_crossover_mode, 0);
        assert_eq!(projected.fitness_kill, 123.0);
        assert_eq!(projected.maximum_output_weight_floats, 55 * 306);

        let mut missing = config();
        missing
            .settings
            .retain(|setting| setting.path != "mutationRate");
        assert!(matches!(
            project_evolution_config(&missing, 306),
            Err(StepConfigError::MissingSetting {
                path: "mutationRate"
            })
        ));

        let mut wrong_type = config();
        *setting_mut(&mut wrong_type, "reward.fitnessFood") = NormalizedSettingValue::Integer(80);
        assert!(matches!(
            project_evolution_config(&wrong_type, 306),
            Err(StepConfigError::WrongSettingType {
                path: "reward.fitnessFood",
                ..
            })
        ));

        assert!(matches!(
            project_evolution_config(&config(), 0),
            Err(StepConfigError::InvalidEvolution)
        ));
    }

    #[test]
    fn live_values_and_types_project_without_silent_defaults() {
        let mut config = config();
        *setting_mut(&mut config, "brain.controlDt") = NormalizedSettingValue::Float(0.03);
        *setting_mut(&mut config, "collision.substepMaxDt") = NormalizedSettingValue::Float(0.01);
        *setting_mut(&mut config, "reward.pointsPerFood") = NormalizedSettingValue::Float(7.5);
        *setting_mut(&mut config, "sense.rFarMax") = NormalizedSettingValue::Float(2_750.0);
        *setting_mut(&mut config, "brain.gruInitUpdateBias") = NormalizedSettingValue::Float(-0.25);
        let projected =
            project_running_step_config(&config, RunningStepWorkLimits::provisional_defaults())
                .expect("changed admitted values must project");
        assert_eq!(projected.world_step.physics_substeps, 2);
        assert_eq!(
            projected.world_step.control.neural_control_interval_seconds,
            0.03
        );
        assert_eq!(projected.world_step.physics.food.points_per_food, 7.5);
        assert_eq!(projected.sensor.far_radius_maximum, 2_750.0);
        assert_eq!(projected.genome_initialization.gru_update_bias, -0.25);

        *setting_mut(&mut config, "reward.pointsPerFood") = NormalizedSettingValue::Integer(7);
        assert!(matches!(
            project_running_step_config(&config, RunningStepWorkLimits::provisional_defaults()),
            Err(StepConfigError::WrongSettingType {
                path: "reward.pointsPerFood",
                ..
            })
        ));
    }

    #[test]
    fn missing_out_of_range_and_unsupported_values_fail_closed() {
        let mut missing = config();
        missing
            .settings
            .retain(|setting| setting.path != "snakeTurnPenalty");
        assert!(matches!(
            project_running_step_config(&missing, RunningStepWorkLimits::provisional_defaults()),
            Err(StepConfigError::MissingSetting {
                path: "snakeTurnPenalty"
            })
        ));

        let mut range = config();
        *setting_mut(&mut range, "collision.hitScale") = NormalizedSettingValue::Float(2.0);
        assert!(matches!(
            project_running_step_config(&range, RunningStepWorkLimits::provisional_defaults()),
            Err(StepConfigError::OutOfRange {
                path: "collision.hitScale"
            })
        ));

        let mut color = config();
        *setting_mut(&mut color, "death.useSnakeColor") = NormalizedSettingValue::Bool(false);
        assert!(matches!(
            project_running_step_config(&color, RunningStepWorkLimits::provisional_defaults()),
            Err(StepConfigError::UnsupportedSetting {
                path: "death.useSnakeColor"
            })
        ));
    }

    #[test]
    fn duplicated_top_level_values_and_work_limits_are_revalidated() {
        for (path, mutate) in [
            (
                "worldRadius",
                (|config: &mut NormalizedEngineConfig| config.world_radius = 3_600.0)
                    as fn(&mut NormalizedEngineConfig),
            ),
            (
                "snakeCount",
                (|config: &mut NormalizedEngineConfig| config.population_count = 56)
                    as fn(&mut NormalizedEngineConfig),
            ),
            (
                "baselineBots.count",
                (|config: &mut NormalizedEngineConfig| config.baseline_count = 11)
                    as fn(&mut NormalizedEngineConfig),
            ),
            (
                "simSpeed",
                (|config: &mut NormalizedEngineConfig| config.requested_sim_speed = 2.0)
                    as fn(&mut NormalizedEngineConfig),
            ),
        ] {
            let mut mismatch = config();
            mutate(&mut mismatch);
            assert!(matches!(
                project_running_step_config(
                    &mismatch,
                    RunningStepWorkLimits::provisional_defaults()
                ),
                Err(StepConfigError::ProjectionMismatch { path: actual }) if actual == path
            ));
        }

        let mut invalid_tick_rate = config();
        invalid_tick_rate.fixed_step_seconds = 1.0 / 241.0;
        assert!(matches!(
            project_running_step_config(
                &invalid_tick_rate,
                RunningStepWorkLimits::provisional_defaults()
            ),
            Err(StepConfigError::OutOfRange {
                path: "fixed_step_seconds"
            })
        ));

        let mut limits = RunningStepWorkLimits::provisional_defaults();
        limits.collision_index_entries = 0;
        assert!(matches!(
            project_running_step_config(&config(), limits),
            Err(StepConfigError::WorldStep(_))
        ));
    }
}
