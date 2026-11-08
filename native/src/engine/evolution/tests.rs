use super::*;
use crate::engine::graph::{
    compile_graph, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
};
use crate::engine::rng::StatefulRng;
use crate::engine::state::{BodyRange, BrainHandle, GenomeLineage, SnakeState, WorldPoint};
use crate::engine::work_progress::ProgressScope;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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
    graph_with_memory(
        GraphNodeKind::Gru {
            input_size: 3,
            hidden_size: 2,
        },
        2,
    )
}

/// Compile mixed feed-forward/recurrent owners at small and large valid sizes.
fn graph_with_memory(memory: GraphNodeKind, hidden: usize) -> CompiledGraph {
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
                kind: memory,
            },
            GraphNodeSpec {
                id: "head".to_owned(),
                kind: GraphNodeKind::Dense {
                    input_size: hidden,
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
            max_tensor_width: 512,
            max_mlp_hidden_layers: 8,
            max_split_output_ports: 8,
            max_parameter_floats: 1_000_000,
            max_recurrent_state_floats: 4_096,
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
            observation_delivered: false,
            completed_step_heading_delta: 0.0,
            completed_step_speed_delta: 0.0,
            completed_step_applied_turn: 0.0,
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
            ray_cursor: Default::default(),
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

/// Probe Float64 neighbors around exact uniform words, including zero/subnormal rates.
#[test]
fn mutation_cutoffs_match_float_decisions_at_probability_boundaries() {
    let mut rates = vec![-0.0, 0.0, f64::from_bits(1), 0.025, 0.03, 0.35, 0.5];
    for word in [1_u32, 2, 123, 128_849_019, 1_073_741_823, 2_147_483_648] {
        let boundary = f64::from(word) / 4_294_967_296.0;
        for bits in [
            boundary.to_bits() - 1,
            boundary.to_bits(),
            boundary.to_bits() + 1,
        ] {
            let rate = f64::from_bits(bits);
            if rate <= 0.5 {
                rates.push(rate);
            }
        }
    }
    for rate in rates {
        let cutoff = mutation_cutoff(rate);
        for word in [0, 1, u32::MAX, cutoff.saturating_sub(1), cutoff, cutoff + 1] {
            assert_eq!(
                word < cutoff,
                f64::from(word) / 4_294_967_296.0 < rate,
                "word={word}, rate={rate}"
            );
        }
    }
}

/// Retained original probability scan, independent from integer threshold selection.
fn mutate_with_float_reference(
    weights: &mut [f32],
    graph: &CompiledGraph,
    config: EvolutionConfig,
    rng: &mut StatefulRng,
) {
    for node in &graph.nodes {
        let recurrent = matches!(
            node.node_type,
            CompiledNodeType::Gru | CompiledNodeType::Lstm | CompiledNodeType::Rru
        );
        let (rate, deviation) = if recurrent {
            (
                config.recurrent_mutation_rate,
                config.recurrent_mutation_std,
            )
        } else {
            (config.mutation_rate, config.mutation_std)
        };
        for weight in
            &mut weights[node.parameter_offset..node.parameter_offset + node.parameter_length]
        {
            if rng.next_f64() < rate {
                *weight = (f64::from(*weight) + rng.gaussian() * deviation).clamp(-5.0, 5.0) as f32;
                assert!(weight.is_finite());
            }
        }
    }
}

/// Large mixed owners preserve every weight bit and the complete Gaussian continuation.
#[test]
fn integer_mutation_selection_preserves_weight_bits_and_gaussian_state() {
    let graphs = [
        graph(),
        graph_with_memory(
            GraphNodeKind::Gru {
                input_size: 3,
                hidden_size: 384,
            },
            384,
        ),
        graph_with_memory(
            GraphNodeKind::Lstm {
                input_size: 3,
                hidden_size: 384,
            },
            384,
        ),
        graph_with_memory(
            GraphNodeKind::Rru {
                input_size: 3,
                hidden_size: 384,
            },
            384,
        ),
    ];
    assert!(graphs.iter().any(|graph| graph.total_parameters > 400_000));
    let pattern = [
        0.0_f32,
        -0.0,
        f32::from_bits(1),
        -f32::from_bits(1),
        0.125,
        -0.125,
        4.9999,
        -4.9999,
        5.0,
        -5.0,
    ];
    for graph in &graphs {
        for (rate, recurrent_rate, deviation, recurrent_deviation) in [
            (0.0, 0.0, 0.0, 0.0),
            (0.03, 0.025, 0.35, 0.22),
            (0.5, 0.35, 2.5, 1.6),
        ] {
            let config = EvolutionConfig {
                mutation_rate: rate,
                recurrent_mutation_rate: recurrent_rate,
                mutation_std: deviation,
                recurrent_mutation_std: recurrent_deviation,
                ..EvolutionConfig::typescript_defaults()
            };
            config.validate().unwrap();
            for seed in [1.0, 42.0, f64::from(u32::MAX)] {
                for cached in [false, true] {
                    let mut source = StatefulRng::new(seed);
                    if cached {
                        source.gaussian();
                    }
                    let continuation = source.export_state();
                    let mut actual_rng = StatefulRng::from_state(&continuation).unwrap();
                    let mut expected_rng = StatefulRng::from_state(&continuation).unwrap();
                    let mut actual = (0..graph.total_parameters)
                        .map(|index| pattern[index % pattern.len()])
                        .collect::<Vec<_>>();
                    let mut expected = actual.clone();
                    mutate(&mut actual, graph, config, &mut actual_rng).unwrap();
                    mutate_with_float_reference(&mut expected, graph, config, &mut expected_rng);
                    assert!(
                        actual
                            .iter()
                            .zip(&expected)
                            .all(|(a, b)| a.to_bits() == b.to_bits()),
                        "weights={}, rate={rate}, seed={seed}, cached={cached}",
                        graph.total_parameters
                    );
                    assert_eq!(actual_rng.export_state(), expected_rng.export_state());
                }
            }
        }
    }
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
    let work_bytes = Arc::new(AtomicU64::new(0));
    let progress_scope = ProgressScope::enter(Arc::clone(&work_bytes));
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
    drop(progress_scope);
    assert!(work_bytes.load(Ordering::Relaxed) > 0);

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
        for (index, (actual, expected)) in actual.weights.iter().zip(expected_weights).enumerate() {
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
fn explicit_one_life_assessments_preserve_legacy_evolution_and_rng() {
    let fixture = fixture();
    let graph = graph();
    let (world, population, rng) = source_state(&fixture);
    let configuration = config(&fixture.evolution, &graph, population.len());
    let legacy =
        prepare_evolution(&world, &population, &graph, &rng, 1, 0.0, configuration).unwrap();
    let indices = map_population_snakes(&world, &population).unwrap();
    let assessments = indices
        .iter()
        .map(|&index| {
            let snake = &world.snakes[index];
            EvolvedFitnessAssessment {
                assessment: EvolvedAssessment {
                    points: snake.points,
                    alive_seconds: snake.age_seconds,
                    food: snake.food,
                    kills: snake.kills,
                    retained_growth: snake
                        .body
                        .len
                        .saturating_sub(configuration.snake_start_length)
                        as f64,
                    peak_length: snake.body.len,
                },
                deaths: u64::from(!snake.alive),
                successful_respawns: 0,
            }
        })
        .collect::<Vec<_>>();
    let evolved = prepare_evolution_with_assessments(
        &world,
        &population,
        &graph,
        &rng,
        1,
        0.0,
        configuration,
        &assessments,
        EvolvedFitnessObjective {
            normalize_to_240_seconds: false,
            configured_duration_seconds: 240.0,
            penalty_amount: 0.0,
            penalize_successful_respawns: false,
        },
    )
    .unwrap();
    assert_eq!(evolved.source_fitness(), legacy.source_fitness());
    assert_eq!(evolved.next_population(), legacy.next_population());
    assert_eq!(evolved.next_evolution_rng(), legacy.next_evolution_rng());
    assert_eq!(evolved.hall_of_fame(), legacy.hall_of_fame());
}

#[test]
fn evolved_objective_scales_only_extensive_terms_and_allows_negative_penalties() {
    let config = EvolutionConfig::typescript_defaults();
    let entries = [EvolvedFitnessAssessment {
        assessment: EvolvedAssessment {
            points: 10.0,
            alive_seconds: 3600.0,
            food: 1.0,
            kills: 1,
            retained_growth: 2.0,
            peak_length: 20,
        },
        deaths: 2,
        successful_respawns: 1,
    }];
    let mut fitness = [0.0];
    let mut objective = EvolvedFitnessObjective {
        normalize_to_240_seconds: true,
        configured_duration_seconds: 3600.0,
        penalty_amount: 1000.0,
        penalize_successful_respawns: false,
    };
    calculate_evolved_fitness(&entries, &mut fitness, config, objective).unwrap();
    let extensive = 3600.0 * 0.70 + 80.0 + 200.0 + 400.0;
    assert_eq!(
        fitness[0],
        extensive * (240.0 / 3600.0) + 42.0 + 600.0 - 2000.0
    );
    assert!(fitness[0] < 0.0);
    objective.penalize_successful_respawns = true;
    let deaths = fitness[0];
    calculate_evolved_fitness(&entries, &mut fitness, config, objective).unwrap();
    assert!((fitness[0] - (deaths + 1000.0)).abs() < 1.0e-10);
    for duration in [240.0, 1800.0, 3600.0, 21600.0, 604800.0] {
        objective.configured_duration_seconds = duration;
        objective.validate().unwrap();
        calculate_evolved_fitness(&entries, &mut fitness, config, objective).unwrap();
        assert!(fitness[0].is_finite());
    }
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
    assert!(!genomes_share_species(&left, &right).unwrap());
}

#[test]
fn species_early_rejection_preserves_complete_distance_decisions() {
    let mut left = source_state(&fixture()).1.remove(0);
    let mut right = left.clone();
    for count in [1, 255, 256, 257, 511, 512, 8_192, 400_003] {
        left.weights = vec![0.0; count].into_boxed_slice();
        for difference in [
            0.1_f32,
            f32::from_bits(0.35_f32.to_bits() - 1),
            0.35,
            f32::from_bits(0.35_f32.to_bits() + 1),
            0.36,
            2.0,
        ] {
            right.weights = vec![difference; count].into_boxed_slice();
            assert_eq!(
                genomes_share_species(&left, &right).unwrap(),
                scalar_species(&left, &right),
                "count={count}, difference={difference}"
            );
        }
        for spike in [0, count / 2, count - 1] {
            right.weights = vec![0.0; count].into_boxed_slice();
            right.weights[spike] = 100.0;
            assert_eq!(
                genomes_share_species(&left, &right).unwrap(),
                scalar_species(&left, &right)
            );
        }
    }
    right.weights = vec![0.0; left.weights.len() - 1].into_boxed_slice();
    assert!(matches!(
        genomes_share_species(&left, &right),
        Err(EvolutionError::PopulationShape { .. })
    ));
}

/// Exercise proof success, guarded fallback and deliberately misleading prefixes.
#[test]
fn species_simd_probe_preserves_scalar_decisions_for_diverse_and_clustered_owners() {
    let mut left = source_state(&fixture()).1.remove(0);
    let mut right = left.clone();
    for count in [4095, 4096, 4097, 8193, 494_306] {
        left.weights = vec![0.0; count].into_boxed_slice();
        right.weights = vec![0.5; count].into_boxed_slice();
        let far_limit = SPECIES_DISTANCE_THRESHOLD.powi(2) * count as f64 * 1.000_001;
        let prefix = 0.25 * SPECIES_DISTANCE_SCAN_BLOCK as f64;
        assert_eq!(
            species_probe_proves_far(&left.weights, &right.weights, prefix, far_limit),
            count >= SPECIES_DISTANCE_PROBE_MIN_WEIGHTS
        );
        for difference in [
            f32::from_bits(1),
            f32::from_bits(0.35_f32.to_bits() - 1),
            0.35,
            f32::from_bits(0.35_f32.to_bits() + 1),
            0.350_01,
            0.350_1,
            0.5,
            f32::MAX,
        ] {
            right.weights = vec![difference; count].into_boxed_slice();
            assert_eq!(
                genomes_share_species(&left, &right).unwrap(),
                scalar_species(&left, &right),
                "count={count}, difference={difference}"
            );
        }
        right.weights = vec![0.0; count].into_boxed_slice();
        right.weights[..SPECIES_DISTANCE_SCAN_BLOCK].fill(1.0);
        assert!(!species_probe_proves_far(
            &left.weights,
            &right.weights,
            SPECIES_DISTANCE_SCAN_BLOCK as f64,
            far_limit
        ));
        assert_eq!(
            genomes_share_species(&left, &right).unwrap(),
            scalar_species(&left, &right)
        );

        // Test-only synthetic values; never advance an authoritative RNG stream.
        for seed in 1..=12_u64 {
            let mut state = seed;
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / 16_777_216.0 - 0.5) * 1.1
            };
            left.weights = (0..count)
                .map(|_| next())
                .collect::<Vec<_>>()
                .into_boxed_slice();
            for clustered in [false, true] {
                right.weights = left
                    .weights
                    .iter()
                    .map(|&value| {
                        if clustered {
                            value + next() * 0.2
                        } else {
                            next()
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                assert_eq!(
                    genomes_share_species(&left, &right).unwrap(),
                    scalar_species(&left, &right),
                    "count={count}, seed={seed}, clustered={clustered}"
                );
            }
        }
    }
}

/// Independent complete scalar distance with no early-exit or block policy.
fn scalar_species(left: &PopulationGenome, right: &PopulationGenome) -> bool {
    let mut sum = 0.0;
    for (&a, &b) in left.weights.iter().zip(right.weights.iter()) {
        let difference = f64::from(a) - f64::from(b);
        sum += difference * difference;
    }
    (sum / left.weights.len() as f64).sqrt() <= SPECIES_DISTANCE_THRESHOLD
}

/// Norm acceptance is checked against complete scalar distance, including fallback cases.
#[test]
fn species_norm_acceptance_preserves_scalar_boundary_decisions() {
    let mut left = source_state(&fixture()).1.remove(0);
    let mut right = left.clone();
    let patterns = [
        (0.0_f32, -0.0_f32),
        (f32::from_bits(1), -f32::from_bits(1)),
        (0.02, -0.08),
        (0.174_98, -0.174_98),
        (0.174_99, -0.174_99),
        (f32::from_bits(0.175_f32.to_bits() - 1), -0.175),
        (0.175, -0.175),
        (f32::from_bits(0.175_f32.to_bits() + 1), -0.175),
        (0.18, -0.18),
        (5.0, 5.0),
        (f32::MAX, f32::MAX),
        (f32::MAX, -f32::MAX),
        (f32::MAX, f32::from_bits(1)),
    ];
    let mut accepted = 0;
    let mut fallback = 0;
    for count in [0, 1, 255, 256, 257, 8_192, 400_003] {
        for &(a, b) in &patterns {
            left.weights = vec![a; count].into_boxed_slice();
            right.weights = vec![b; count].into_boxed_slice();
            for spike in [None, Some(0), Some(count / 2), count.checked_sub(1)] {
                if let Some(index) = spike.filter(|&index| index < count) {
                    left.weights[index] = 5.0;
                }
                let norm = |genome: &PopulationGenome| {
                    genome.weights.iter().fold(0.0, |sum, &weight| {
                        let value = f64::from(weight);
                        sum + value * value
                    })
                };
                let proven = species_norms_prove_close(count, count, norm(&left), norm(&right));
                let expected = scalar_species(&left, &right);
                assert!(
                    !proven || expected,
                    "count={count}, a={a}, b={b}, spike={spike:?}"
                );
                assert_eq!(
                    proven || genomes_share_species(&left, &right).unwrap(),
                    expected,
                    "count={count}, a={a}, b={b}, spike={spike:?}"
                );
                accepted += usize::from(proven);
                fallback += usize::from(!proven);
            }
        }
    }
    assert!(accepted > 0);
    assert!(fallback > 0);
    assert!(species_norms_prove_close(400_003, 400_003, 100.0, 100.0));
    assert!(!species_norms_prove_close(
        1,
        1,
        0.175_f64.powi(2),
        0.175_f64.powi(2)
    ));
    for (a, b) in [(f64::NAN, 0.0), (0.0, f64::INFINITY), (-1.0, 0.0)] {
        assert!(!species_norms_prove_close(1, 1, a, b));
    }
    assert!(!species_norms_prove_close(1, 2, 0.0, 0.0));
    assert!(!species_norms_prove_close(0, 0, 0.0, 0.0));
    assert!(!species_norms_prove_close(
        SPECIES_NORM_MAX_WEIGHTS + 1,
        SPECIES_NORM_MAX_WEIGHTS + 1,
        0.0,
        0.0
    ));
}

/// Large-owner statistics keep exact original bits and greedy species decisions.
#[test]
fn summary_span_counts_preserve_scalar_statistics_and_species() {
    let patterns = [
        0.0_f32,
        -0.0,
        f32::from_bits(1),
        -f32::from_bits(1),
        0.125,
        -0.125,
        0.35,
        -0.35,
        1.0,
        -1.0,
        5.0,
        -5.0,
    ];
    let source = source_state(&fixture()).1.remove(0);
    let sorted = [2, 0, 1];
    let fitness = [2.0, 1.0, 3.0];
    for (count, scale) in [(0, 1.0), (257, 1.0), (262_147, 1.0), (400_003, 0.005)] {
        let population = (0..3)
            .map(|slot| {
                let mut genome = source.clone();
                genome.slot = slot;
                genome.weights = (0..count)
                    .map(|index| {
                        let value = patterns[index % patterns.len()] * scale;
                        match slot {
                            1 => value + 0.01,
                            2 => -value,
                            _ => value,
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                genome
            })
            .collect::<Vec<_>>();
        let mut absolute_sum = 0.0_f64;
        let mut squared_sum = 0.0_f64;
        let mut weight_count = 0usize;
        let mut representatives: Vec<usize> = Vec::new();
        let mut species_sizes: Vec<u64> = Vec::new();
        for &slot in &sorted {
            for &weight in population[slot].weights.iter() {
                let absolute = f64::from(weight).abs();
                absolute_sum += absolute;
                squared_sum += absolute * absolute;
                weight_count += 1;
            }
            if let Some(species) = representatives.iter().position(|&representative| {
                scalar_species(&population[slot], &population[representative])
            }) {
                species_sizes[species] += 1;
            } else {
                representatives.push(slot);
                species_sizes.push(1);
            }
        }
        let (average, variance) = if weight_count == 0 {
            (0.0, 0.0)
        } else {
            let average = absolute_sum / weight_count as f64;
            (
                average,
                (squared_sum / weight_count as f64 - average * average).max(0.0),
            )
        };
        let actual = calculate_summary(&population, &sorted, &fitness, 9).unwrap();
        assert_eq!(actual.average_weight.to_bits(), average.to_bits());
        assert_eq!(actual.weight_variance.to_bits(), variance.to_bits());
        assert_eq!(actual.species_count, representatives.len() as u64);
        assert_eq!(
            actual.top_species_size,
            *species_sizes.iter().max().unwrap()
        );
        assert_eq!(actual.generation, 9);
        assert_eq!(actual.average, 2.0);
        assert_eq!(actual.best, 3.0);
        assert_eq!(actual.minimum, 1.0);
    }
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
