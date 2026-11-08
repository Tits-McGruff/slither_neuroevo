use super::*;

#[test]
fn exact_scalar_prefix_uses_local_economics_whole_step_motion_and_actual_cost() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 25);
    let snake = &mut world.snakes[0];
    snake.points = 140.0;
    snake.delivered_observation_points = 130.0;
    snake.age_seconds = 60.0;
    snake.input_boost = true;
    snake.turn = 0.75;
    snake.completed_step_applied_turn = -0.25;
    snake.completed_step_heading_delta = 3.2 / 60.0 * 0.5;
    snake.completed_step_speed_delta = 50.0;
    let before = world.clone();
    let (values, result) = simple(SensorProfile::Motion, &world);
    close(values[0], 0.0);
    close(values[1], 1.0);
    close(values[2], -1.0 / 3.0);
    close(values[3], 0.0);
    close(values[4], -0.34);
    close(values[5], 0.0);
    let cost = 7.0 * (1.0 + 1.1 * 20.0 / 9995.0);
    close(values[6], 2.0 * 138.8 / (138.8 + 2.0 * cost) - 1.0);
    close(values[7], 2.0 * cost / (cost + 7.0) - 1.0);
    assert_eq!(&values[8..11], &[1.0, -1.0, 1.0]);
    close(values[11], 0.75);
    close(values[12], -0.25);
    close(values[13], 0.5);
    close(values[14], 0.1);
    close(values[15], 0.5);
    close(values[16], 2.0 * 140.0 / 340.0 - 1.0);
    close(values[17], 2.0 * 140.0 / 1140.0 - 1.0);
    close(values[18], 0.0);
    close(values[19], 0.5);
    close(values[20], 0.0);
    assert_eq!(&values[21..24], &[1.0, -1.0, -1.0]);
    assert!((result.radii.near - (520.0 + 260.0 / 3.0)).abs() < 1e-9);
    assert_eq!(world, before);
    assert!(!world.snakes[0].observation_delivered);
    result.delivery.commit(&mut world.snakes[0]).unwrap();
    let (second, _) = simple(SensorProfile::Motion, &world);
    assert_eq!(second[21], -1.0);
    assert_eq!(second[15], 0.0);
}

#[test]
fn all_profiles_bins_and_kind_sentinels_have_finite_exact_widths_and_absence() {
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in 8..=32 {
            for (kind, sentinel) in [
                (SnakeKind::Evolved, -1.0),
                (SnakeKind::External, 0.0),
                (SnakeKind::Baseline, 0.0),
                (SnakeKind::Resurrected, 0.0),
            ] {
                let mut world = WorldState::default();
                add_snake(&mut world, 1, kind, point(0.0, 0.0), 0.0, 5);
                let (values, result) = sample(
                    &evaluator(profile, bins),
                    &world,
                    &EvolvedGenerationState::default(),
                    EvolvedExperimentConfig::legacy(240.0),
                    1024,
                );
                assert_eq!(
                    values.len(),
                    24 + if profile == SensorProfile::Basic {
                        6
                    } else {
                        15
                    } * bins
                );
                assert_eq!(values[20], sentinel);
                assert!(values
                    .iter()
                    .all(|value| value.is_finite() && (-1.0..=1.0).contains(value)));
                assert_eq!(result.representative_head_ids, [0; 32]);
                if profile == SensorProfile::Motion {
                    assert!(values[24 + 6 * bins..24 + 7 * bins]
                        .iter()
                        .all(|v| *v == -1.0));
                    assert!(values[24 + 7 * bins..].iter().all(|v| *v == 0.0));
                }
            }
        }
    }
}

#[test]
fn failed_ray_and_detail_queries_are_visible_and_do_not_consume_life_delivery() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
    add_snake(
        &mut world,
        2,
        SnakeKind::Baseline,
        point(150.0, 0.0),
        PI / 2.0,
        2202,
    );
    // Keep all other segments in the near detail range, proving the cap.
    for p in &mut world.body_points[5..] {
        *p = point(150.0, 0.0);
    }
    for id in 10..1010 {
        world.pellets.push(PelletState {
            id,
            position: point(20.0, 0.0),
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        });
    }
    let before = world.clone();
    let (values, result) = sample(
        &evaluator(SensorProfile::Basic, 16),
        &world,
        &EvolvedGenerationState::default(),
        EvolvedExperimentConfig::legacy(240.0),
        0,
    );
    assert_eq!(&values[21..24], &[1.0, 1.0, 1.0]);
    assert!(result.diagnostics.body.candidate_limit_reached);
    assert_eq!(result.diagnostics.incomplete_rays, 16);
    assert!(values[24 + 2 * 16..24 + 4 * 16].iter().all(|v| *v == -1.0));
    assert_eq!(world, before);
}

#[test]
fn zero_cost_has_unbounded_budget_and_retains_actual_no_positive_spend_eligibility() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 25);
    world.snakes[0].points = 100.0;
    world.snakes[0].input_boost = true;
    let evaluator = SensorV4Evaluator::new(
        SensorProfile::Basic,
        SensorConfig {
            boost_points_cost_per_second: 0.0,
            ..SensorConfig::default()
        },
        MovementConfig {
            boost_points_cost_per_second: 0.0,
            ..MovementConfig::typescript_defaults()
        },
        1.0 / 60.0,
    )
    .unwrap();
    let (values, _) = sample(
        &evaluator,
        &world,
        &EvolvedGenerationState::default(),
        EvolvedExperimentConfig::legacy(240.0),
        128,
    );
    assert_eq!(values[6], 1.0);
    assert_eq!(values[7], -1.0);
    assert_eq!(values[8], -1.0);
    assert_eq!(values[9], -1.0);
    assert_eq!(values[10], 1.0);
}

#[test]
fn evolved_carried_wallet_and_finite_unlimited_budgets_use_actual_owner_ledger() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::Evolved, point(0.0, 0.0), 0.0, 5);
    world.snakes[0].points = 80.0;
    let mut lives = EvolvedGenerationState::initialize(&world, 1, 5).unwrap();
    let owner = &mut lives.slots[0];
    owner
        .finalize(crate::engine::evolved_lives::EvolvedDeathCause::GodMode)
        .unwrap();
    assert_eq!(owner.begin_life(2, 5, true).unwrap(), 80.0);
    owner.alive_award(1.0, 20.0).unwrap();
    world.snakes[0].id = 2;
    world.snakes[0].frame_v1_id = 2;
    world.snakes[0].points = 100.0;
    world.snakes[0].age_seconds = 1.0;
    let policy = EvolvedExperimentConfig {
        version: 1,
        enabled: true,
        max_lives: 4,
        points: crate::engine::evolved_lives::PointAssessmentMode::CumulativeNet,
        ..EvolvedExperimentConfig::legacy(240.0)
    };
    let (values, _) = sample(
        &evaluator(SensorProfile::Motion, 16),
        &world,
        &lives,
        policy,
        512,
    );
    close(values[5], 2.0 * 100.0 / 240.0 - 1.0);
    close(values[16], 2.0 * 20.0 / 220.0 - 1.0);
    close(values[17], 2.0 * 100.0 / 1100.0 - 1.0);
    assert_eq!(values[20], -0.5);
    let (unlimited, _) = sample(
        &evaluator(SensorProfile::Motion, 16),
        &world,
        &lives,
        EvolvedExperimentConfig {
            max_lives: 0,
            ..policy
        },
        512,
    );
    assert_eq!(unlimited[20], 1.0);
    let (off, _) = sample(
        &evaluator(SensorProfile::Motion, 16),
        &world,
        &lives,
        EvolvedExperimentConfig {
            enabled: false,
            ..policy
        },
        512,
    );
    assert_eq!(off[20], -1.0);
}

#[test]
fn malformed_shapes_and_mixed_source_indexes_reject_before_output_or_delivery_changes() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
    let config = SensorIndexConfig {
        body_cell_size: 70.0,
        pellet_cell_size: 120.0,
        maximum_body_entries: 1000,
        maximum_pellet_entries: 1000,
    };
    let indexed = IndexedSensorWorld::build(&world, config).unwrap();
    let other_indexed = IndexedSensorWorld::build(&world, config).unwrap();
    let mut workspace = BodyRayIndexWorkspace::default();
    let mixed_view = workspace.prepare(other_indexed.body_index()).unwrap();
    let lives = EvolvedGenerationState::default();
    let context = V4ObservationContext {
        generation_elapsed_seconds: 0.0,
        evolved: &lives,
        experiment: EvolvedExperimentConfig::legacy(240.0),
    };
    let evaluator = evaluator(SensorProfile::Motion, 16);
    let mut output = vec![99.0; 264];
    let mut meter = RayWorkMeter::new(512);
    assert_eq!(
        evaluator.sample(
            &indexed,
            mixed_view,
            context,
            0,
            &mut output,
            &mut meter,
            &mut V4SensorScratch::default()
        ),
        Err(V4SensorError::Shape)
    );
    assert!(output.iter().all(|value| *value == 99.0));
    assert_eq!(meter.remaining(), 512);
    let view = workspace.prepare(indexed.body_index()).unwrap();
    assert_eq!(
        evaluator.sample(
            &indexed,
            view,
            context,
            0,
            &mut output[..263],
            &mut meter,
            &mut V4SensorScratch::default()
        ),
        Err(V4SensorError::Shape)
    );
    assert!(output.iter().all(|value| *value == 99.0));
    assert!(!world.snakes[0].observation_delivered);
    assert!(SensorV4Evaluator::new(
        SensorProfile::V3,
        SensorConfig::default(),
        MovementConfig::typescript_defaults(),
        1.0 / 60.0
    )
    .is_err());
    assert!(SensorV4Evaluator::new(
        SensorProfile::Basic,
        SensorConfig::default(),
        MovementConfig {
            snake_boost_speed: 400.0,
            ..MovementConfig::typescript_defaults()
        },
        1.0 / 60.0
    )
    .is_err());
}

#[test]
fn serial_and_parallel_workers_receive_identical_complete_candidate_observations() {
    use rayon::prelude::*;
    let mut world = WorldState::default();
    for id in 1..=24 {
        add_snake(
            &mut world,
            id,
            SnakeKind::External,
            point((id % 6) as f64 * 150.0, (id / 6) as f64 * 175.0),
            id as f64 * 0.25,
            5 + id as usize % 20,
        );
    }
    let before = world.clone();
    let indexed = IndexedSensorWorld::build(
        &world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 1_000_000,
            maximum_pellet_entries: 1000,
        },
    )
    .unwrap();
    let mut rays = BodyRayIndexWorkspace::default();
    let view = rays.prepare(indexed.body_index()).unwrap();
    let lives = EvolvedGenerationState::default();
    let context = V4ObservationContext {
        generation_elapsed_seconds: 0.0,
        evolved: &lives,
        experiment: EvolvedExperimentConfig::legacy(240.0),
    };
    let evaluator = evaluator(SensorProfile::Motion, 32);
    let mut reference = None;
    let available = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    for workers in [1, available] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let result: Vec<_> = pool.install(|| {
            (0..world.snakes.len())
                .into_par_iter()
                .map_init(V4SensorScratch::default, |scratch, index| {
                    let mut output = vec![0.0; evaluator.layout().input_size()];
                    let sample = evaluator
                        .sample(
                            &indexed,
                            view,
                            context,
                            index,
                            &mut output,
                            &mut RayWorkMeter::new(1024),
                            scratch,
                        )
                        .unwrap();
                    (output, sample)
                })
                .collect()
        });
        if let Some(expected) = &reference {
            assert_eq!(&result, expected);
        } else {
            reference = Some(result);
        }
    }
    assert_eq!(world, before);
}

#[cfg(feature = "engine-test-hooks")]
#[test]
fn prototype_budget_quality_reports_all_candidates_on_existing_native_source_shapes() {
    use crate::engine::calculation::CalculationBatchKey;
    use crate::engine::ray_work::{
        prepare_ray_quotas, RayObservationOwner, RayQuotaCursor, RayWorkLimits,
    };
    use crate::engine::sensing_fixture::{build_world, Stage4SensingScenarioName};
    for scenario in [Stage4SensingScenarioName::P0, Stage4SensingScenarioName::P1] {
        let world = build_world(scenario.spec()).unwrap();
        let indexed = IndexedSensorWorld::build(
            &world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 100_000,
            },
        )
        .unwrap();
        let mut rays = BodyRayIndexWorkspace::default();
        let view = rays.prepare(indexed.body_index()).unwrap();
        let lives = EvolvedGenerationState::default();
        let context = V4ObservationContext {
            generation_elapsed_seconds: 120.0,
            evolved: &lives,
            experiment: EvolvedExperimentConfig::legacy(240.0),
        };
        let mut owners: Vec<_> = world
            .snakes
            .iter()
            .enumerate()
            .map(|(index, snake)| (RayObservationOwner::of(snake).unwrap(), index))
            .collect();
        owners.sort_unstable_by_key(|entry| entry.0);
        let keys: Vec<_> = owners.iter().map(|entry| entry.0).collect();
        let mut grants = vec![0; keys.len()];
        for bins in [16, 32] {
            let evaluator = evaluator(SensorProfile::Motion, bins);
            let mut output = vec![0.0; evaluator.layout().input_size()];
            let mut scratch = V4SensorScratch::default();
            for batch in [16_384, 32_768, 65_536] {
                for cap in [128, 256, 512, 1024] {
                    // Exploration floor only. Production minimum is not selected here.
                    let limits = RayWorkLimits::new(1, cap, batch, 512).unwrap();
                    let proposal = prepare_ray_quotas(
                        CalculationBatchKey::new(1, 1, 1),
                        limits,
                        RayQuotaCursor::default(),
                        &keys,
                        &mut grants,
                    )
                    .unwrap();
                    let mut saturated = 0;
                    let mut ray_count = 0;
                    let mut charged = 0;
                    for ((_, index), grant) in owners.iter().zip(&grants) {
                        let mut meter = RayWorkMeter::new(*grant);
                        let sample = evaluator
                            .sample(
                                &indexed,
                                view,
                                context,
                                *index,
                                &mut output,
                                &mut meter,
                                &mut scratch,
                            )
                            .unwrap();
                        saturated += usize::from(sample.diagnostics.incomplete_rays > 0);
                        ray_count += sample.diagnostics.incomplete_rays;
                        charged += grant - meter.remaining();
                    }
                    assert!(charged <= proposal.granted());
                    println!("prototype-v4 {scenario:?} bins={bins} batch={batch} cap={cap} samples={} saturated={saturated} incomplete_rays={ray_count} charged={charged}", owners.len());
                }
            }
        }
    }
}
