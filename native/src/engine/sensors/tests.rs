use super::*;
use crate::engine::spatial::SensorIndexConfig;
use crate::engine::state::{BodyRange, PelletState, SnakeKind};
use serde::Deserialize;

#[test]
fn pellet_direction_classifier_matches_original_bins_across_supported_layouts() {
    for bins in 8..=MAX_SENSOR_BINS {
        for direction in [
            -1.0e20, -1.0e6, -TAU, -PI, -2.1, -0.37, 0.0, 0.37, 2.1, PI, TAU, 1.0e6, 1.0e20,
        ] {
            let classifier = CenteredAngularBins::new(direction, bins);
            for sample in 0..20_000_u64 {
                let angle = (sample.wrapping_mul(65_537) % 20_000) as f64 / 20_000.0 * TAU;
                let (y, x) = angle.sin_cos();
                let expected = angle_to_centered_bin(normalize_angle(y.atan2(x) - direction), bins);
                assert_eq!(
                    classifier.classify(x, y, 1.0),
                    expected,
                    "bins={bins}, direction={direction}, angle={angle}"
                );
            }
            for boundary in 0..bins {
                let angle = direction - PI + (boundary as f64 + 0.5) * TAU / bins as f64;
                for delta in [-1.0e-10, -1.0e-13, 0.0, 1.0e-13, 1.0e-10] {
                    let (y, x) = (angle + delta).sin_cos();
                    let expected =
                        angle_to_centered_bin(normalize_angle(y.atan2(x) - direction), bins);
                    assert_eq!(
                        classifier.classify(x, y, 1.0),
                        expected,
                        "bins={bins}, direction={direction}, boundary={boundary}, delta={delta}"
                    );
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceDocument {
    fixture_version: u32,
    sensor_layout_version: String,
    cases: Vec<ReferenceCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceCase {
    name: String,
    config: ReferenceConfig,
    best_points_this_generation: f64,
    target_id: u64,
    snakes: Vec<ReferenceSnake>,
    pellets: Vec<ReferencePellet>,
    expected: Vec<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceConfig {
    bins: usize,
    world_radius: f64,
    food_value: f64,
    snake_boost_speed: f64,
    snake_start_length: usize,
    snake_max_length: usize,
    minimum_boost_points: f64,
    boost_points_cost_per_second: f64,
    boost_points_cost_size_factor: f64,
    generation_seconds: f64,
    near_radius_base: f64,
    near_radius_scale: f64,
    near_radius_minimum: f64,
    near_radius_maximum: f64,
    far_radius_base: f64,
    far_radius_scale: f64,
    far_radius_minimum: f64,
    far_radius_maximum: f64,
    food_saturation: f64,
    collision_hit_scale: f64,
    maximum_pellet_checks: usize,
    maximum_segment_checks: usize,
}

impl From<ReferenceConfig> for SensorConfig {
    fn from(source: ReferenceConfig) -> Self {
        Self {
            bins: source.bins,
            world_radius: source.world_radius,
            food_value: source.food_value,
            snake_boost_speed: source.snake_boost_speed,
            snake_start_length: source.snake_start_length,
            snake_max_length: source.snake_max_length,
            minimum_boost_points: source.minimum_boost_points,
            boost_points_cost_per_second: source.boost_points_cost_per_second,
            boost_points_cost_size_factor: source.boost_points_cost_size_factor,
            generation_seconds: source.generation_seconds,
            near_radius_base: source.near_radius_base,
            near_radius_scale: source.near_radius_scale,
            near_radius_minimum: source.near_radius_minimum,
            near_radius_maximum: source.near_radius_maximum,
            far_radius_base: source.far_radius_base,
            far_radius_scale: source.far_radius_scale,
            far_radius_minimum: source.far_radius_minimum,
            far_radius_maximum: source.far_radius_maximum,
            food_saturation: source.food_saturation,
            collision_hit_scale: source.collision_hit_scale,
            maximum_pellet_checks: source.maximum_pellet_checks,
            maximum_segment_checks: source.maximum_segment_checks,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct ReferencePoint {
    x: f64,
    y: f64,
}

impl From<ReferencePoint> for WorldPoint {
    fn from(source: ReferencePoint) -> Self {
        Self {
            x: source.x,
            y: source.y,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceSnake {
    id: u64,
    kind: String,
    alive: bool,
    position: ReferencePoint,
    direction: f64,
    radius: f64,
    speed: f64,
    boost: bool,
    age_seconds: f64,
    points: f64,
    delivered_observation_points: f64,
    body: Vec<ReferencePoint>,
}

#[derive(Debug, Deserialize)]
struct ReferencePellet {
    id: u64,
    position: ReferencePoint,
    value: f64,
    kind: u32,
    color: u32,
    owner: Option<u64>,
}

fn snake(id: u64, kind: SnakeKind, body: BodyRange, position: WorldPoint) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("test id should fit"),
        kind,
        alive: true,
        population_slot: None,
        brain: None,
        baseline_slot: None,
        baseline_strategy: None,
        position,
        previous_position: position,
        direction: 0.0,
        radius: 10.0,
        speed: 165.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: body.len as f64,
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
        body,
        skin: 0,
    }
}

fn indexed_world(world: &WorldState) -> IndexedSensorWorld<'_> {
    IndexedSensorWorld::build(
        world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 1_000_000,
            maximum_pellet_entries: 100_000,
        },
    )
    .expect("sensor indexes should build")
}

fn reference_world(case: &ReferenceCase) -> WorldState {
    let mut world = WorldState::default();
    for source in &case.snakes {
        let body_start = world.body_points.len();
        world
            .body_points
            .extend(source.body.iter().copied().map(WorldPoint::from));
        let kind = match source.kind.as_str() {
            "evolved" => SnakeKind::Evolved,
            "baseline" => SnakeKind::Baseline,
            "external" => SnakeKind::External,
            "resurrected" => SnakeKind::Resurrected,
            other => panic!("unsupported reference snake kind {other}"),
        };
        let mut converted = snake(
            source.id,
            kind,
            BodyRange {
                start: body_start,
                len: source.body.len(),
            },
            source.position.into(),
        );
        converted.alive = source.alive;
        converted.direction = source.direction;
        converted.radius = source.radius;
        converted.speed = source.speed;
        converted.boost = source.boost;
        converted.age_seconds = source.age_seconds;
        converted.points = source.points;
        converted.delivered_observation_points = source.delivered_observation_points;
        converted.target_length = source.body.len() as f64;
        world.snakes.push(converted);
    }
    world
        .pellets
        .extend(case.pellets.iter().map(|source| PelletState {
            id: source.id,
            position: source.position.into(),
            value: source.value,
            kind: source.kind,
            color: source.color,
            owner: source.owner,
        }));
    world
}

fn sample(
    evaluator: &SensorEvaluator,
    world: &WorldState,
    snake_index: usize,
    generation: &SensorGenerationState,
) -> (Vec<f32>, SensorSample) {
    let indexed = indexed_world(world);
    let mut output = vec![0.0; evaluator.layout().input_size];
    let mut scratch = SensorScratch::default();
    let sample = evaluator
        .sample(&indexed, generation, snake_index, &mut output, &mut scratch)
        .expect("sensor sample should succeed");
    (output, sample)
}

#[test]
fn life_assessments_change_only_v3_relative_scores_and_preserve_delivered_wallet_delta() {
    let evaluator = SensorEvaluator::new(SensorConfig::default()).unwrap();
    let mut world = WorldState {
        body_points: vec![WorldPoint { x: 0.0, y: 0.0 }],
        ..WorldState::default()
    };
    let mut entity = snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 1 },
        WorldPoint { x: 0.0, y: 0.0 },
    );
    entity.points = 2.0;
    entity.delivered_observation_points = 1.0;
    world.snakes.push(entity);
    let mut generation = SensorGenerationState::new();
    generation.observe_assessments(&[80.0]).unwrap();
    generation.observe_assessments(&[10.0]).unwrap();
    assert_eq!(generation.best_points_this_generation(), 80.0);
    let (legacy, before) = sample(&evaluator, &world, 0, &generation);
    let indexed = indexed_world(&world);
    let mut selected = vec![0.0; legacy.len()];
    let after = evaluator
        .sample_with_assessment(
            &indexed,
            &generation,
            0,
            &mut selected,
            &mut SensorScratch::default(),
            Some(80.0),
        )
        .unwrap();
    assert_eq!(selected[4], 1.0);
    assert_eq!(selected[7], 1.0);
    for index in 0..selected.len() {
        if index != 4 && index != 7 {
            assert_eq!(selected[index], legacy[index]);
        }
    }
    assert_eq!(before.delivery, after.delivery);
    assert_eq!(after.delivery.sampled_points, 2.0);
    assert_eq!(world.snakes[0].delivered_observation_points, 1.0);
}

#[test]
fn default_layout_produces_all_scalars_and_clear_channels() {
    let evaluator = SensorEvaluator::new(SensorConfig::default()).expect("config should validate");
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -7.5, y: 0.0 },
            WorldPoint { x: -15.0, y: 0.0 },
            WorldPoint { x: -22.5, y: 0.0 },
            WorldPoint { x: -30.0, y: 0.0 },
        ],
        ..WorldState::default()
    };
    world.snakes.push(snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 5 },
        WorldPoint { x: 0.0, y: 0.0 },
    ));
    let (output, result) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    assert_eq!(output.len(), 83);
    assert_eq!(output[0], 0.0);
    assert_eq!(output[1], 1.0);
    assert_eq!(output[2], -1.0);
    assert_eq!(output[5], -0.34);
    assert_eq!(output[6], -1.0);
    assert_eq!(output[8], 0.0);
    assert_eq!(output[9], -0.999);
    assert_eq!(output[12], 1.0);
    assert_eq!(output[13], -1.0);
    assert_eq!(output[16], -1.0);
    assert_eq!(output[17], -1.0);
    assert_eq!(output[18], -1.0);
    for index in 0..evaluator.layout().bins {
        assert_eq!(output[evaluator.layout().offsets.food + index], -1.0);
        assert_eq!(output[evaluator.layout().offsets.hazard + index], 1.0);
        assert_eq!(output[evaluator.layout().offsets.head + index], 1.0);
    }
    assert_eq!(result.delivery.sampled_points, 0.0);
}

#[test]
fn long_body_segment_is_visible_near_an_endpoint_not_its_midpoint() {
    let config = SensorConfig {
        bins: 8,
        near_radius_base: 200.0,
        near_radius_scale: 0.0,
        near_radius_minimum: 200.0,
        near_radius_maximum: 200.0,
        ..SensorConfig::default()
    };
    let evaluator = SensorEvaluator::new(config).expect("config should validate");
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -7.5, y: 0.0 },
            WorldPoint { x: 80.0, y: -20.0 },
            WorldPoint {
                x: 1_000.0,
                y: 20.0,
            },
        ],
        ..WorldState::default()
    };
    world.snakes.push(snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 2 },
        WorldPoint { x: 0.0, y: 0.0 },
    ));
    let mut other = snake(
        2,
        SnakeKind::Evolved,
        BodyRange { start: 2, len: 2 },
        WorldPoint { x: 80.0, y: -20.0 },
    );
    other.radius = 10.0;
    world.snakes.push(other);
    let (output, _) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    let forward = angle_to_centered_bin(0.0, evaluator.layout().bins);
    assert!(output[16] > -1.0);
    assert!(output[evaluator.layout().offsets.hazard + forward] < 1.0);
}

#[test]
fn segment_cap_reports_and_saturates_conservatively() {
    let config = SensorConfig {
        bins: 8,
        maximum_segment_checks: 200,
        near_radius_base: 500.0,
        near_radius_scale: 0.0,
        near_radius_minimum: 500.0,
        near_radius_maximum: 500.0,
        ..SensorConfig::default()
    };
    let evaluator = SensorEvaluator::new(config).expect("config should validate");
    let mut world = WorldState::default();
    world.body_points.extend([
        WorldPoint { x: 0.0, y: 0.0 },
        WorldPoint { x: -7.5, y: 0.0 },
    ]);
    world.snakes.push(snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 2 },
        WorldPoint { x: 0.0, y: 0.0 },
    ));
    for offset in 0..201usize {
        let angle = offset as f64 / 201.0 * TAU;
        let start = world.body_points.len();
        let position = WorldPoint {
            x: angle.cos() * 200.0,
            y: angle.sin() * 200.0,
        };
        world.body_points.push(position);
        world.body_points.push(WorldPoint {
            x: position.x + 1.0,
            y: position.y + 1.0,
        });
        world.snakes.push(snake(
            offset as u64 + 2,
            SnakeKind::Evolved,
            BodyRange { start, len: 2 },
            position,
        ));
    }
    let (output, result) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    assert_eq!(result.diagnostics.segment_cap_hits, 1);
    assert_eq!(result.diagnostics.segment_checks, 0);
    assert_eq!(result.diagnostics.body_query.candidates, 200);
    assert!(result.diagnostics.conservative_body_saturation);
    assert_eq!(output[16], 1.0);
    for index in 0..evaluator.layout().bins {
        assert_eq!(output[evaluator.layout().offsets.hazard + index], -1.0);
    }
}

#[test]
fn pellet_cap_is_measured_after_nearest_first_ordering() {
    let config = SensorConfig {
        bins: 8,
        maximum_pellet_checks: 100,
        ..SensorConfig::default()
    };
    let evaluator = SensorEvaluator::new(config).expect("config should validate");
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -7.5, y: 0.0 },
        ],
        ..WorldState::default()
    };
    world.snakes.push(snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 2 },
        WorldPoint { x: 0.0, y: 0.0 },
    ));
    for index in 0..121usize {
        world.pellets.push(PelletState {
            id: index as u64 + 10,
            position: WorldPoint {
                x: index as f64 + 10.0,
                y: 0.0,
            },
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        });
    }
    let (output, result) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    assert_eq!(result.diagnostics.pellet_cap_hits, 1);
    assert_eq!(result.diagnostics.pellet_checks, 120);
    assert!(output[13] > -1.0);
}

#[test]
fn pure_probes_accumulate_until_a_matching_delivery_commits() {
    let evaluator = SensorEvaluator::new(SensorConfig::default()).expect("config should validate");
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -7.5, y: 0.0 },
        ],
        ..WorldState::default()
    };
    let mut observer = snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 2 },
        WorldPoint { x: 0.0, y: 0.0 },
    );
    observer.points = 4.0;
    world.snakes.push(observer);

    let (_, first) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    let (second_output, second) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    assert_eq!(second_output[8], 0.4);
    assert_eq!(world.snakes[0].delivered_observation_points, 0.0);
    second
        .delivery
        .commit(&mut world.snakes[0])
        .expect("accepted observation should commit");
    assert_eq!(world.snakes[0].delivered_observation_points, 4.0);
    assert!(matches!(
        first.delivery.commit(&mut world.snakes[0]),
        Err(SensorError::StaleDeliveryMarker { snake_id: 1 })
    ));
    let (third_output, _) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
    assert_eq!(third_output[8], 0.0);
}

#[test]
fn first_life_delivery_flag_changes_only_for_matching_consumption_even_without_score_delta() {
    let evaluator = SensorEvaluator::new(SensorConfig::default()).unwrap();
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -7.5, y: 0.0 },
        ],
        ..WorldState::default()
    };
    world.snakes.push(snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 2 },
        WorldPoint { x: 0.0, y: 0.0 },
    ));
    let generation = SensorGenerationState::new();
    let (_, probe) = sample(&evaluator, &world, 0, &generation);
    let (_, accepted) = sample(&evaluator, &world, 0, &generation);
    assert!(!world.snakes[0].observation_delivered);
    assert!(!probe.delivery.previous_observation_delivered);
    let mut wrong_life = world.snakes[0].clone();
    wrong_life.id += 1;
    assert!(probe.delivery.commit(&mut wrong_life).is_err());
    assert!(!wrong_life.observation_delivered);
    accepted.delivery.commit(&mut world.snakes[0]).unwrap();
    assert!(world.snakes[0].observation_delivered);
    assert_eq!(world.snakes[0].delivered_observation_points, 0.0);
    assert_eq!(
        probe.delivery.commit(&mut world.snakes[0]),
        Err(SensorError::StaleDeliveryMarker { snake_id: 1 })
    );
    let (_, delivered_life) = sample(&evaluator, &world, 0, &generation);
    assert!(delivered_life.delivery.previous_observation_delivered);
    // A new physical life clears the flag independently of score and recurrence.
    world.snakes[0].id += 1;
    world.snakes[0].observation_delivered = false;
    assert!(delivered_life
        .delivery
        .commit(&mut world.snakes[0])
        .is_err());
    let (_, newborn) = sample(&evaluator, &world, 0, &generation);
    assert!(!newborn.delivery.previous_observation_delivered);
}

#[test]
fn generation_best_is_initialized_and_excludes_non_population_snakes() {
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: 10.0, y: 0.0 },
        ],
        ..WorldState::default()
    };
    let mut evolved = snake(
        1,
        SnakeKind::Evolved,
        BodyRange { start: 0, len: 1 },
        WorldPoint { x: 0.0, y: 0.0 },
    );
    evolved.points = 5.0;
    let mut baseline = snake(
        2,
        SnakeKind::Baseline,
        BodyRange { start: 1, len: 1 },
        WorldPoint { x: 10.0, y: 0.0 },
    );
    baseline.points = 50.0;
    world.snakes.extend([evolved, baseline]);
    let mut generation = SensorGenerationState::new();
    assert_eq!(generation.best_points_this_generation(), 0.0);
    generation
        .update_after_step(&world)
        .expect("valid evolved scores should update");
    assert_eq!(generation.best_points_this_generation(), 5.0);
    generation.reset();
    assert_eq!(generation.best_points_this_generation(), 0.0);
}

#[test]
fn all_supported_bin_counts_produce_finite_exact_length_observations() {
    for bins in 8..=32 {
        let config = SensorConfig {
            bins,
            ..SensorConfig::default()
        };
        let evaluator = SensorEvaluator::new(config).expect("config should validate");
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: -7.5, y: 0.0 },
            ],
            ..WorldState::default()
        };
        world.snakes.push(snake(
            1,
            SnakeKind::Evolved,
            BodyRange { start: 0, len: 2 },
            WorldPoint { x: 0.0, y: 0.0 },
        ));
        let (output, _) = sample(&evaluator, &world, 0, &SensorGenerationState::new());
        assert_eq!(output.len(), 19 + 4 * bins);
        assert!(output.iter().all(|value| value.is_finite()));
    }
}

#[test]
fn corrected_typescript_and_rust_vectors_match_shared_reference_cases() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("sensor-v3-reference.json");
    let fixture = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", fixture_path.display()));
    let document: ReferenceDocument =
        serde_json::from_str(&fixture).expect("shared sensor fixture should parse");
    assert_eq!(document.fixture_version, 1);
    assert_eq!(document.sensor_layout_version, "v3");
    assert_eq!(document.cases.len(), 3);
    for case in document.cases {
        let world = reference_world(&case);
        let evaluator = SensorEvaluator::new(case.config.into())
            .unwrap_or_else(|error| panic!("{} config failed: {error}", case.name));
        let target_index = world
            .snakes
            .iter()
            .position(|snake| snake.id == case.target_id)
            .unwrap_or_else(|| panic!("{} target is absent", case.name));
        let indexed = indexed_world(&world);
        let generation = SensorGenerationState {
            best_points_this_generation: case.best_points_this_generation,
        };
        let mut actual = vec![0.0; evaluator.layout().input_size];
        let mut scratch = SensorScratch::default();
        let plain = evaluator
            .sample(
                &indexed,
                &generation,
                target_index,
                &mut actual,
                &mut scratch,
            )
            .unwrap_or_else(|error| panic!("{} sample failed: {error}", case.name));
        let mut traced_output = vec![0.0; actual.len()];
        let mut trace = super::super::sensor_trace::SensorTrace::default();
        let traced = evaluator
            .sample_with_assessment_traced(
                &indexed,
                &generation,
                target_index,
                &mut traced_output,
                &mut SensorScratch::default(),
                None,
                Some(&mut trace),
            )
            .unwrap();
        assert_eq!(traced_output, actual);
        assert_eq!(traced.diagnostics, plain.diagnostics);
        assert_eq!(traced.delivery, plain.delivery);
        assert!(trace.valid);
        assert_eq!(
            trace.profile,
            super::super::sensor_contract::SensorProfile::V3
        );
        assert_eq!(trace.physical_id, case.target_id);
        assert_eq!(trace.body_length, world.snakes[target_index].body.len);
        assert_eq!(
            trace.generation_best_points,
            Some(case.best_points_this_generation)
        );
        assert!(trace
            .body_rays
            .iter()
            .all(|ray| *ray == super::super::sensor_trace::SensorTraceRay::NotSampled));
        for bin in 0..evaluator.layout().bins {
            assert_eq!(trace.food_strength[bin], f64::from(scratch.food_bins[bin]));
            assert_eq!(
                trace.body_clearance[bin],
                f64::from(scratch.hazard_bins[bin])
            );
            assert_eq!(trace.head_clearance[bin], f64::from(scratch.head_bins[bin]));
        }
        assert_eq!(actual.len(), case.expected.len(), "{} length", case.name);
        for (index, (actual, expected)) in actual.iter().zip(&case.expected).enumerate() {
            let difference = f64::from((*actual - *expected).abs());
            assert!(
                difference <= 2.0e-5,
                "{} value {index}: actual {actual}, expected {expected}, difference {difference}",
                case.name
            );
        }
    }
}
