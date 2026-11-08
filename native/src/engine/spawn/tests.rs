use super::*;
use crate::engine::state::{BrainHandle, SnakeKind, SnakeState};

fn key(slot: u64) -> SpawnKey {
    SpawnKey {
        domain: SpawnDomain::Evolved,
        slot,
    }
}

fn request(slot: u64) -> SpawnRequest {
    SpawnRequest { key: key(slot) }
}

fn snake(id: u64, start: usize, body: &[WorldPoint], radius: f64) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("test ID should fit"),
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(u32::try_from(id - 1).expect("test slot should fit")),
        brain: Some(BrainHandle { id, epoch: 1 }),
        baseline_slot: None,
        baseline_strategy: None,
        position: body[0],
        previous_position: body[0],
        direction: 0.0,
        radius,
        speed: 0.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: body.len() as f64,
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
            len: body.len(),
        },
        skin: 0,
    }
}

fn world_with_body(body: &[WorldPoint], radius: f64) -> WorldState {
    WorldState {
        ray_cursor: Default::default(),
        snakes: vec![snake(1, 0, body, radius)],
        body_points: body.to_vec(),
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    }
}

fn assert_batch_clear(prepared: &PreparedSpawns<'_, '_>, config: SpawnConfig) {
    for (index, placement) in prepared.placements().iter().enumerate() {
        let body = prepared.body_for(placement).expect("placement body");
        let wall_limit = config.world_radius - config.snake_radius - config.wall_clearance;
        for point in body {
            assert!(point.x * point.x + point.y * point.y < wall_limit * wall_limit);
        }
        for other in prepared.placements().iter().skip(index + 1) {
            let other_body = prepared.body_for(other).expect("other placement body");
            let mut geometry_checks = 0;
            assert!(!polylines_within(
                body,
                other_body,
                config.snake_radius * 2.0 + config.body_clearance,
                &mut geometry_checks,
                config,
                placement.key,
            )
            .expect("prepared bodies should fit the test work budget"));
        }
    }
}

#[test]
fn complete_body_crossing_is_rejected_even_when_heads_are_clear() {
    let existing = [
        WorldPoint { x: 0.0, y: -30.0 },
        WorldPoint { x: 0.0, y: 30.0 },
    ];
    let world = world_with_body(&existing, 2.0);
    let candidate = [
        WorldPoint { x: 40.0, y: 0.0 },
        WorldPoint { x: -40.0, y: 0.0 },
    ];
    let config = SpawnConfig {
        snake_radius: 2.0,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let mut geometry_checks = 0;
    assert!(!candidate_is_valid(
        &candidate,
        SpawnObstacles {
            source_world: &world,
            source_order: &[SourceObstacle {
                index: 0,
                bounds: BodyBounds::for_body(&existing)
            }],
            placements: &[],
            staged_body_points: &[],
        },
        config,
        key(0),
        &mut geometry_checks,
    )
    .expect("finite source should validate"));
}

#[test]
fn cached_bounds_include_bent_obstacles_and_keep_threshold_tangencies_exact() {
    let bent = [
        WorldPoint {
            x: -100.0,
            y: -100.0,
        },
        WorldPoint { x: 500.0, y: 0.0 },
        WorldPoint {
            x: -100.0,
            y: 100.0,
        },
    ];
    let near_middle = [WorldPoint { x: 499.0, y: 0.0 }];
    assert!(!BodyBounds::for_body(&near_middle).separated_from(BodyBounds::for_body(&bent), 2.0));
    let world = world_with_body(&bent, 1.0);
    let mut workspace = SpawnWorkspace::default();
    workspace
        .prepare_source_order(&world)
        .expect("finite bent obstacle");
    let config = SpawnConfig {
        snake_radius: 1.0,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    assert!(!candidate_is_valid(
        &near_middle,
        SpawnObstacles {
            source_world: &world,
            source_order: &workspace.source_order,
            placements: &[],
            staged_body_points: &[]
        },
        config,
        key(0),
        &mut 0
    )
    .expect("middle segment must remain on the exact collision path"));
    let origin = BodyBounds::for_body(&[WorldPoint { x: 0.0, y: 0.0 }]);
    for gap in [6.0 - f64::EPSILON * 8.0, 6.0, 6.0 + f64::EPSILON * 8.0] {
        assert!(!origin.separated_from(BodyBounds::for_body(&[WorldPoint { x: gap, y: 0.0 }]), 6.0));
    }
    let huge = BodyBounds::for_body(&[WorldPoint {
        x: f64::MAX,
        y: 0.0,
    }]);
    assert!(!origin.separated_from(huge, f64::MAX));
}

#[test]
fn separated_bounds_never_skip_a_hit_found_by_the_unfiltered_segment_oracle() {
    let mut rng = StatefulRng::new(9921.0);
    let config = SpawnConfig::typescript_geometry_defaults();
    for _ in 0..512 {
        let mut body = || {
            (0..12)
                .map(|_| WorldPoint {
                    x: (rng.next_f64() - 0.5) * 1000.0,
                    y: (rng.next_f64() - 0.5) * 1000.0,
                })
                .collect::<Vec<_>>()
        };
        let first = body();
        let mut second = body();
        let shift = (rng.next_f64() - 0.5) * 4000.0;
        for point in &mut second {
            point.x += shift;
        }
        let exact_hit = polylines_within(&first, &second, 6.0, &mut 0, config, key(0))
            .expect("small oracle pair fits its work budget");
        if BodyBounds::for_body(&first).separated_from(BodyBounds::for_body(&second), 6.0) {
            assert!(
                !exact_hit,
                "a cached bounding rejection must not hide a segment hit"
            );
        }
    }
}

#[test]
fn long_body_prefix_matches_unfiltered_placements_and_rng_continuation() {
    let source = StatefulRng::new(1_511_506_142.0).export_state();
    let config = SpawnConfig {
        world_radius: 10_000.0,
        snake_radius: 3.0,
        snake_spacing: 3.0,
        snake_start_len: 140,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let world = WorldState::default();
    let requests = (0..32).map(request).collect::<Vec<_>>();
    let mut workspace = SpawnWorkspace::default();
    let prepared = workspace
        .prepare(&world, &requests, &source, config, 4480)
        .expect("long prefix fits the ordinary budget with cached bounds");
    let mut rng = StatefulRng::from_state(&source).expect("oracle RNG");
    let mut bodies: Vec<Vec<WorldPoint>> = Vec::new();
    let mut comparisons = 0;
    let oracle_config = SpawnConfig {
        maximum_geometry_checks_per_batch: 100_000_000,
        ..config
    };
    for placement in prepared.placements() {
        let mut accepted = false;
        for attempt in 1..=config.random_attempts_per_request {
            let angle = rng.next_f64() * TAU;
            let radius =
                rng.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
            let head = WorldPoint {
                x: angle.cos() * radius,
                y: angle.sin() * radius,
            };
            let direction = rng.next_f64() * TAU;
            let mut candidate = Vec::new();
            build_body(&mut candidate, head, direction, config).expect("finite oracle body");
            let wall = config.world_radius - config.snake_radius - config.wall_clearance;
            if candidate
                .iter()
                .any(|point| point.x * point.x + point.y * point.y >= wall * wall)
            {
                continue;
            }
            let mut blocked = false;
            for body in &bodies {
                if polylines_within(
                    &candidate,
                    body,
                    2.0 * config.snake_radius + config.body_clearance,
                    &mut comparisons,
                    oracle_config,
                    placement.key,
                )
                .expect("oracle budget")
                {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                continue;
            }
            assert_eq!(placement.direction, direction);
            assert_eq!(placement.candidates_examined, attempt);
            assert!(!placement.used_fallback);
            assert_eq!(
                prepared.body_for(placement).expect("prepared body"),
                candidate
            );
            bodies.push(candidate);
            accepted = true;
            break;
        }
        assert!(
            accepted,
            "the unfiltered oracle must accept the same random candidate"
        );
    }
    assert_eq!(prepared.next_rng(), &rng.export_state());
}

#[test]
fn stable_request_order_owns_the_same_locations_and_rng_continuation() {
    fn run(
        requests: &[SpawnRequest],
    ) -> (Vec<SpawnPlacement>, Vec<WorldPoint>, SerializedRngState) {
        let mut workspace = SpawnWorkspace::default();
        let world = WorldState::default();
        let source = StatefulRng::new(0x5afe as f64).export_state();
        let prepared = workspace
            .prepare(
                &world,
                requests,
                &source,
                SpawnConfig::typescript_geometry_defaults(),
                requests.len() * 5,
            )
            .expect("default world should fit");
        (
            prepared.placements().to_vec(),
            prepared.body_points().to_vec(),
            prepared.next_rng().clone(),
        )
    }

    let forward = [request(0), request(1), request(2), request(3)];
    let reversed = [request(3), request(2), request(1), request(0)];
    assert_eq!(run(&forward), run(&reversed));
}

#[test]
fn first_random_candidate_matches_the_typescript_three_draw_formula() {
    let source = StatefulRng::new(0x5afe as f64).export_state();
    let config = SpawnConfig::typescript_geometry_defaults();
    // Captured by scripts/stage5/generate-spawn-fixtures.ts and retained in
    // Generator archived at archive/ts-reference-and-rust-2026-10-10.
    // docs/archive/rust-authoritative-migration/evidence/stage5/typescript-spawn-fixtures.json. Keep these
    // literal values independent of Rust's candidate/body helpers.
    let expected_direction = 3.799_256_610_584_831_8;
    let expected_body = [
        WorldPoint {
            x: -641.684_079_186_302_6,
            y: 620.807_787_646_342_3,
        },
        WorldPoint {
            x: -635.748_411_621_326_1,
            y: 625.392_310_583_146_7,
        },
        WorldPoint {
            x: -629.812_744_056_349_6,
            y: 629.976_833_519_951_1,
        },
        WorldPoint {
            x: -623.877_076_491_373_1,
            y: 634.561_356_456_755_6,
        },
        WorldPoint {
            x: -617.941_408_926_396_7,
            y: 639.145_879_393_56,
        },
    ];

    let world = WorldState::default();
    let mut workspace = SpawnWorkspace::default();
    let prepared = workspace
        .prepare(&world, &[request(4)], &source, config, 5)
        .expect("first random candidate should fit an empty default world");
    let placement = prepared.placements()[0];
    assert!(!placement.used_fallback);
    assert_eq!(placement.candidates_examined, 1);
    assert!((placement.direction - expected_direction).abs() <= 1.0e-12);
    let actual_body = prepared.body_for(&placement).expect("prepared body");
    for (actual, expected) in actual_body.iter().zip(expected_body) {
        assert!((actual.x - expected.x).abs() <= 1.0e-9);
        assert!((actual.y - expected.y).abs() <= 1.0e-9);
    }
    assert_eq!(placement.head, actual_body[0]);
    assert_eq!(prepared.next_rng().state_hex, "0x9acbaf14");
    assert!(!prepared.next_rng().gaussian_spare_valid);
}

#[test]
fn source_snake_container_order_cannot_change_placement() {
    fn source_world(reverse: bool) -> WorldState {
        let first = [
            WorldPoint {
                x: -300.0,
                y: -30.0,
            },
            WorldPoint { x: -300.0, y: 30.0 },
        ];
        let second = [
            WorldPoint { x: 300.0, y: -30.0 },
            WorldPoint { x: 300.0, y: 30.0 },
        ];
        let mut world = WorldState {
            ray_cursor: Default::default(),
            snakes: vec![snake(1, 0, &first, 9.0), snake(2, 2, &second, 9.0)],
            body_points: first.into_iter().chain(second).collect(),
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        };
        if reverse {
            world.snakes.reverse();
        }
        world
    }

    fn run(world: &WorldState) -> (Vec<SpawnPlacement>, Vec<WorldPoint>, SerializedRngState) {
        let source = StatefulRng::new(604.0).export_state();
        let mut workspace = SpawnWorkspace::default();
        let prepared = workspace
            .prepare(
                world,
                &[request(8), request(9)],
                &source,
                SpawnConfig::typescript_geometry_defaults(),
                10,
            )
            .expect("fixture placements should fit");
        (
            prepared.placements().to_vec(),
            prepared.body_points().to_vec(),
            prepared.next_rng().clone(),
        )
    }

    assert_eq!(run(&source_world(false)), run(&source_world(true)));
}

#[test]
fn blocked_candidate_and_tight_budget_remain_source_order_invariant() {
    fn run(reverse: bool) -> (Vec<SpawnPlacement>, Vec<WorldPoint>, SerializedRngState) {
        let source = StatefulRng::new(91.0).export_state();
        let mut probe = StatefulRng::from_state(&source).expect("fixture RNG");
        let config = SpawnConfig {
            world_radius: 500.0,
            snake_radius: 2.0,
            snake_spacing: 3.0,
            snake_start_len: 5,
            random_attempts_per_request: 1,
            fallback_position_count: 1,
            fallback_heading_count: 1,
            maximum_geometry_checks_per_batch: 600,
            ..SpawnConfig::typescript_geometry_defaults()
        };
        let angle = probe.next_f64() * TAU;
        let radius = probe.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
        let blocked_head = WorldPoint {
            x: angle.cos() * radius,
            y: angle.sin() * radius,
        };
        let _direction = probe.next_f64();
        let long_clear_body = (0..100)
            .map(|index| WorldPoint {
                x: 10_000.0,
                y: index as f64 * 3.0,
            })
            .collect::<Vec<_>>();
        let mut body_points = vec![blocked_head];
        body_points.extend_from_slice(&long_clear_body);
        let mut world = WorldState {
            ray_cursor: Default::default(),
            snakes: vec![
                snake(1, 0, &[blocked_head], 2.0),
                snake(2, 1, &long_clear_body, 2.0),
            ],
            body_points,
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        };
        if reverse {
            world.snakes.reverse();
        }
        let mut workspace = SpawnWorkspace::default();
        let prepared = workspace
            .prepare(&world, &[request(7)], &source, config, 5)
            .expect("canonical obstacle order should fit the tight budget");
        assert!(prepared.placements()[0].used_fallback);
        (
            prepared.placements().to_vec(),
            prepared.body_points().to_vec(),
            prepared.next_rng().clone(),
        )
    }

    assert_eq!(run(false), run(true));
}

#[test]
fn exact_wall_tangency_is_rejected_before_the_first_movement_boundary() {
    let config = SpawnConfig::typescript_geometry_defaults();
    let candidate = [WorldPoint {
        x: config.world_radius - config.snake_radius - config.wall_clearance,
        y: 0.0,
    }];
    let mut geometry_checks = 0;
    let empty_world = WorldState::default();
    assert!(!candidate_is_valid(
        &candidate,
        SpawnObstacles {
            source_world: &empty_world,
            source_order: &[],
            placements: &[],
            staged_body_points: &[],
        },
        config,
        key(0),
        &mut geometry_checks,
    )
    .expect("tangent fixture is finite"));
}

#[test]
fn sixty_four_snakes_are_complete_body_safe_or_the_batch_fails() {
    let requests = (0..64).map(request).collect::<Vec<_>>();
    let source = StatefulRng::new(0x5afe as f64).export_state();
    let world = WorldState::default();
    let config = SpawnConfig::typescript_geometry_defaults();
    let mut workspace = SpawnWorkspace::default();
    let prepared = workspace
        .prepare(&world, &requests, &source, config, requests.len() * 5)
        .expect("default arena should fit the correction fixture");
    assert_eq!(prepared.placements().len(), requests.len());
    assert_batch_clear(&prepared, config);
}

#[test]
fn deterministic_fallback_is_used_after_a_blocked_random_candidate() {
    let source = StatefulRng::new(91.0).export_state();
    let mut probe = StatefulRng::from_state(&source).expect("valid fixture RNG");
    let config = SpawnConfig {
        world_radius: 500.0,
        snake_radius: 2.0,
        snake_spacing: 3.0,
        snake_start_len: 5,
        random_attempts_per_request: 1,
        fallback_position_count: 128,
        fallback_heading_count: 8,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let angle = probe.next_f64() * TAU;
    let radius = probe.next_f64().sqrt() * (config.world_radius * config.spawn_radius_fraction);
    let blocked_head = WorldPoint {
        x: angle.cos() * radius,
        y: angle.sin() * radius,
    };
    let _heading = probe.next_f64();
    let world = world_with_body(&[blocked_head], 20.0);
    let mut workspace = SpawnWorkspace::default();
    let prepared = workspace
        .prepare(&world, &[request(7)], &source, config, 5)
        .expect("fallback should find clear space");
    assert!(prepared.placements()[0].used_fallback);
    assert!(prepared.placements()[0].candidates_examined > 1);
    assert_eq!(prepared.next_rng(), &probe.export_state());
}

#[test]
fn aggregate_geometry_budget_fails_promptly_and_atomically() {
    let source = StatefulRng::new(71.0).export_state();
    let world = WorldState::default();
    let config = SpawnConfig {
        maximum_geometry_checks_per_batch: 1,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let mut workspace = SpawnWorkspace::default();
    let error = workspace
        .prepare(&world, &[request(3)], &source, config, 5)
        .expect_err("five-point wall validation must exceed one geometry check");
    assert_eq!(
        error,
        SpawnError::WorkBudgetExceeded {
            key: key(3),
            work: "geometry checks",
            required: 2,
            maximum: 1,
        }
    );
    assert!(!workspace.is_ready());
    assert_eq!(source, StatefulRng::new(71.0).export_state());
    assert_eq!(world, WorldState::default());
}

#[test]
fn supported_long_initial_population_fits_the_existing_geometry_budget() {
    let source = StatefulRng::new(1_511_506_142.0).export_state();
    let world = WorldState::default();
    let requests = (0..300).map(request).collect::<Vec<_>>();
    let config = SpawnConfig {
        world_radius: 10_000.0,
        snake_radius: 3.0,
        snake_spacing: 3.0,
        snake_start_len: 140,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let mut workspace = SpawnWorkspace::default();
    let prepared = workspace
        .prepare(&world, &requests, &source, config, 42_000)
        .expect("supported long initial population should fit the ordinary work budget");
    assert_eq!(prepared.placements().len(), 300);
    assert_eq!(prepared.body_points().len(), 42_000);
    assert!(prepared.diagnostics().geometry_checks <= config.maximum_geometry_checks_per_batch);
    eprintln!(
        "long initial population: {} candidates, {} geometry checks",
        prepared.diagnostics().candidates_examined,
        prepared.diagnostics().geometry_checks
    );
    assert_eq!(world, WorldState::default());
    assert_eq!(source, StatefulRng::new(1_511_506_142.0).export_state());
}

#[test]
fn aggregate_candidate_budget_fails_promptly_and_atomically() {
    let source = StatefulRng::new(72.0).export_state();
    let world = WorldState::default();
    let config = SpawnConfig {
        world_radius: 20.0,
        snake_radius: 9.0,
        snake_spacing: 7.5,
        snake_start_len: 5,
        random_attempts_per_request: 1,
        fallback_position_count: 1,
        fallback_heading_count: 1,
        maximum_candidates_per_batch: 1,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let mut workspace = SpawnWorkspace::default();
    let error = workspace
        .prepare(&world, &[request(4)], &source, config, 5)
        .expect_err("the second candidate must exceed the aggregate budget");
    assert_eq!(
        error,
        SpawnError::WorkBudgetExceeded {
            key: key(4),
            work: "spawn candidates",
            required: 2,
            maximum: 1,
        }
    );
    assert!(!workspace.is_ready());
    assert_eq!(source, StatefulRng::new(72.0).export_state());
    assert_eq!(world, WorldState::default());
}

#[test]
fn evolved_impossible_placement_preserves_failure_draws_and_continues_fallback() {
    let world = WorldState::default();
    let source = StatefulRng::new(123.0).export_state();
    let mut config = SpawnConfig::typescript_geometry_defaults();
    config.world_radius = 1.0;
    let mut workspace = SpawnWorkspace::default();
    let attempt = workspace
        .prepare_evolved_replacement(&world, 7, &source, config, 0, 2048, 250000, 5)
        .unwrap();
    assert_eq!(attempt.status(), EvolvedPlacementStatus::PendingNoPlacement);
    assert_eq!(attempt.spawns().diagnostics().candidates_examined, 256);
    assert_eq!(attempt.fallback_cursor(), 224);
    assert!(attempt.spawns().placements().is_empty());
    let mut expected_rng = StatefulRng::from_state(&source).unwrap();
    for _ in 0..96 {
        expected_rng.next_f64();
    }
    assert_eq!(attempt.spawns().next_rng(), &expected_rng.export_state());
    let continuation = attempt.spawns().next_rng().clone();
    let cursor = attempt.fallback_cursor();
    let attempt = workspace
        .prepare_evolved_replacement(&world, 7, &continuation, config, cursor, 2048, 250000, 5)
        .unwrap();
    assert_eq!(attempt.fallback_cursor(), 448);
    assert_eq!(world, WorldState::default());
    assert_eq!(source, StatefulRng::new(123.0).export_state());
}

#[test]
fn evolved_geometry_exhaustion_is_pending_and_generic_spawn_still_errors() {
    let world = WorldState::default();
    let source = StatefulRng::new(123.0).export_state();
    let mut config = SpawnConfig::typescript_geometry_defaults();
    let mut workspace = SpawnWorkspace::default();
    let attempt = workspace
        .prepare_evolved_replacement(&world, 1, &source, config, 0, 2048, 1, 5)
        .unwrap();
    assert_eq!(attempt.status(), EvolvedPlacementStatus::PendingWorkLimit);
    assert_eq!(attempt.spawns().diagnostics().geometry_checks, 1);
    assert_eq!(attempt.spawns().diagnostics().candidates_examined, 1);
    assert_ne!(attempt.spawns().next_rng(), &source);
    let idle = workspace
        .prepare_evolved_replacement(&world, 1, &source, config, 0, 2048, 0, 5)
        .unwrap();
    assert_eq!(idle.status(), EvolvedPlacementStatus::PendingWorkLimit);
    assert_eq!(idle.spawns().next_rng(), &source);
    assert_eq!(idle.spawns().diagnostics().candidates_examined, 0);
    config.maximum_geometry_checks_per_batch = 1;
    assert!(matches!(
        workspace.prepare(&world, &[request(1)], &source, config, 5),
        Err(SpawnError::WorkBudgetExceeded { .. })
    ));
}

#[test]
fn evolved_placement_is_safe_and_invalid_source_is_never_pending() {
    let world = WorldState::default();
    let source = StatefulRng::new(123.0).export_state();
    let config = SpawnConfig::typescript_geometry_defaults();
    let mut workspace = SpawnWorkspace::default();
    let attempt = workspace
        .prepare_evolved_replacement(&world, 1, &source, config, 0, 2048, 250000, 5)
        .unwrap();
    assert_eq!(attempt.status(), EvolvedPlacementStatus::Placed);
    let placement = &attempt.spawns().placements()[0];
    assert_eq!(placement.key, key(1));
    assert_eq!(attempt.spawns().body_for(placement).unwrap().len(), 5);
    for point in attempt.spawns().body_for(placement).unwrap() {
        assert!(
            point.x * point.x + point.y * point.y
                < (config.world_radius - config.snake_radius).powi(2)
        );
    }
    assert!(workspace
        .prepare_evolved_replacement(&world, 1, &source, config, u64::MAX, 2048, 250000, 5)
        .is_err());
    assert!(workspace
        .prepare_evolved_replacement(&world, 1, &source, config, 0, 2048, 250000, 4)
        .is_err());
    let mut corrupt = source;
    corrupt.state_hex = "00000000".into();
    assert!(workspace
        .prepare_evolved_replacement(&world, 1, &corrupt, config, 0, 0, 0, 5)
        .is_err());
}

#[test]
fn impossible_request_fails_without_world_or_source_rng_mutation() {
    let source = StatefulRng::new(17.0).export_state();
    let world = WorldState::default();
    let world_before = world.clone();
    let source_before = source.clone();
    let config = SpawnConfig {
        world_radius: 20.0,
        spawn_radius_fraction: 0.60,
        snake_radius: 9.0,
        snake_spacing: 7.5,
        snake_start_len: 5,
        random_attempts_per_request: 2,
        fallback_position_count: 8,
        fallback_heading_count: 4,
        ..SpawnConfig::typescript_geometry_defaults()
    };
    let mut workspace = SpawnWorkspace::default();
    let error = workspace
        .prepare(&world, &[request(0)], &source, config, 5)
        .expect_err("body cannot fit inside the tiny wall");
    assert!(matches!(
        error,
        SpawnError::NoCollisionSafePlacement { key: failed, .. } if failed == key(0)
    ));
    assert!(!workspace.is_ready());
    assert_eq!(world, world_before);
    assert_eq!(source, source_before);
}

#[test]
fn duplicate_and_capacity_failures_happen_before_any_rng_result() {
    let source = StatefulRng::new(31.0).export_state();
    let world = WorldState::default();
    let mut workspace = SpawnWorkspace::default();
    let duplicate = workspace
        .prepare(
            &world,
            &[request(2), request(2)],
            &source,
            SpawnConfig::typescript_geometry_defaults(),
            10,
        )
        .expect_err("duplicate stable keys must fail");
    assert_eq!(duplicate, SpawnError::DuplicateRequestKey(key(2)));
    assert!(!workspace.is_ready());

    let capacity = workspace
        .prepare(
            &world,
            &[request(0), request(1)],
            &source,
            SpawnConfig::typescript_geometry_defaults(),
            9,
        )
        .expect_err("declared body ceiling is too small");
    assert_eq!(
        capacity,
        SpawnError::BodyCapacityExceeded {
            required: 10,
            maximum: 9,
        }
    );
    assert!(!workspace.is_ready());
}

#[test]
fn repeated_batches_reuse_every_reported_capacity() {
    let source = StatefulRng::new(501.0).export_state();
    let world = WorldState::default();
    let requests = (0..24).map(request).collect::<Vec<_>>();
    let config = SpawnConfig::typescript_geometry_defaults();
    let mut workspace = SpawnWorkspace::default();
    let first = workspace
        .prepare(&world, &requests, &source, config, requests.len() * 5)
        .expect("warm batch")
        .diagnostics();
    assert!(first.order_capacity >= requests.len());
    assert!(first.placement_capacity >= requests.len());
    assert!(first.body_point_capacity >= requests.len() * 5);
    assert!(first.candidate_body_capacity >= 5);

    for _ in 0..24 {
        let next = workspace
            .prepare(&world, &requests, &source, config, requests.len() * 5)
            .expect("reused batch")
            .diagnostics();
        assert_eq!(next.order_capacity, first.order_capacity);
        assert_eq!(next.placement_capacity, first.placement_capacity);
        assert_eq!(next.body_point_capacity, first.body_point_capacity);
        assert_eq!(next.candidate_body_capacity, first.candidate_body_capacity);
    }
}

#[test]
fn invalid_source_geometry_and_rng_fail_closed() {
    let body = [WorldPoint { x: 0.0, y: 0.0 }];
    let mut world = world_with_body(&body, 9.0);
    world.snakes[0].radius = f64::NAN;
    let source = StatefulRng::new(9.0).export_state();
    let mut workspace = SpawnWorkspace::default();
    assert!(matches!(
        workspace.prepare(
            &world,
            &[request(0)],
            &source,
            SpawnConfig::typescript_geometry_defaults(),
            5,
        ),
        Err(SpawnError::InvalidSourceSnake { snake_id: 1 })
    ));
    assert!(!workspace.is_ready());

    let mut invalid_rng = source;
    invalid_rng.state_hex = "0x00000000".to_owned();
    assert!(matches!(
        workspace.prepare(
            &WorldState::default(),
            &[request(0)],
            &invalid_rng,
            SpawnConfig::typescript_geometry_defaults(),
            5,
        ),
        Err(SpawnError::InvalidRng(RngError::ZeroXorshiftState))
    ));
    assert!(!workspace.is_ready());
}
