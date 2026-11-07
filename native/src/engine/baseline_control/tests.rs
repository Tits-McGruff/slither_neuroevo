use super::super::rng::SerializedRngState;
use super::super::state::BodyRange;
use super::*;

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    slot: BaselineSlotRuntime,
    strategy: BaselineStrategyState,
    effective: BaselineStrategyState,
    stage: BaselineLifeStage,
    target_bin: usize,
    target_id: Option<u64>,
    draws: usize,
    rng: SerializedRngState,
}

fn snake(
    id: u64,
    kind: SnakeKind,
    length: usize,
    position: WorldPoint,
    direction: f64,
    speed: f64,
) -> (SnakeState, Vec<WorldPoint>) {
    let baseline = kind == SnakeKind::Baseline;
    let body = (0..length)
        .map(|offset| WorldPoint {
            x: position.x - offset as f64 * 7.5,
            y: position.y,
        })
        .collect::<Vec<_>>();
    (
        SnakeState {
            id,
            frame_v1_id: id as u32,
            kind,
            alive: true,
            population_slot: None,
            brain: None,
            baseline_slot: baseline.then_some(0),
            baseline_strategy: baseline.then_some(BaselineStrategyState::Roam),
            position,
            previous_position: position,
            direction,
            radius: 8.0,
            speed,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: length as f64,
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
                start: 0,
                len: length,
            },
            skin: 0,
        },
        body,
    )
}

fn world(entries: Vec<(SnakeState, Vec<WorldPoint>)>) -> WorldState {
    let mut world = WorldState::default();
    for (mut snake, body) in entries {
        snake.body = BodyRange {
            start: world.body_points.len(),
            len: body.len(),
        };
        world.body_points.extend(body);
        world.snakes.push(snake);
    }
    world
}

fn observation(food: f32, hazard: f32, wall: f32, head: f32) -> Vec<f32> {
    let layout = SensorLayout::new(16).expect("fixture layout");
    let mut values = vec![0.0; layout.input_size];
    values[layout.offsets.food..layout.offsets.food + layout.bins].fill(food);
    values[layout.offsets.hazard..layout.offsets.hazard + layout.bins].fill(hazard);
    values[layout.offsets.wall..layout.offsets.wall + layout.bins].fill(wall);
    values[layout.offsets.head..layout.offsets.head + layout.bins].fill(head);
    values
}

fn runtime(id: u64) -> BaselineSlotRuntime {
    BaselineSlotRuntime {
        v2_policy: None,
        v2_last_control_step: None,
        slot: 0,
        snake_id: id,
        strategy_timer_seconds: 0.0,
        wander_angle: 0.0,
        wander_timer_seconds: 0.0,
        turn: 0.0,
        boost: false,
        respawn_remaining_seconds: None,
    }
}

fn execute(
    world: &WorldState,
    observation: &[f32],
    seed: u32,
) -> Result<Snapshot, BaselineControlError> {
    let slot = runtime(world.snakes[0].id);
    let rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(f64::from(seed)).export_state(),
    };
    let layout = SensorLayout::new(16).expect("fixture layout");
    let mut workspace = BaselineControlWorkspace::new();
    let prepared = workspace.prepare(
        world,
        0,
        &slot,
        &rng,
        observation,
        &layout,
        BaselineControlConfig::typescript_defaults(),
        1.0 / 60.0,
    )?;
    Ok(Snapshot {
        slot: prepared.next_slot(),
        strategy: prepared.next_strategy(),
        effective: prepared.effective_strategy(),
        stage: prepared.life_stage(),
        target_bin: prepared.target_bin(),
        target_id: prepared.diagnostics().target_snake_id,
        draws: prepared.diagnostics().uniform_draws,
        rng: prepared.next_rng().state.clone(),
    })
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn matches_retained_typescript_life_stage_state_action_and_uniform_fixtures() {
    let controlled = |length| {
        snake(
            700,
            SnakeKind::Baseline,
            length,
            WorldPoint { x: 0.0, y: 0.0 },
            0.0,
            120.0,
        )
    };

    let roam_world = world(vec![controlled(12)]);
    let roam = execute(&roam_world, &observation(0.0, 0.8, 0.7, 0.9), 42)
        .expect("small roam should evaluate");
    assert_eq!(roam.stage, BaselineLifeStage::Small);
    assert_eq!(roam.strategy, BaselineStrategyState::Roam);
    assert_eq!(roam.target_bin, 8);
    assert_eq!(roam.draws, 2);
    close(roam.slot.wander_angle, -0.298_413_664_475_083_33, 1.0e-15);
    close(
        roam.slot.wander_timer_seconds,
        1.524_436_768_330_633_5,
        1.0e-15,
    );
    close(f64::from(roam.slot.turn), -0.189_976_039_149_503_34, 2.0e-8);
    assert!(!roam.slot.boost);
    assert_eq!(roam.rng.state_hex, "0xa90a34ac");

    let avoid_world = world(vec![controlled(12)]);
    let mut avoid_observation = observation(0.3, -0.8, 0.8, 0.9);
    let layout = SensorLayout::new(16).expect("fixture layout");
    avoid_observation[layout.offsets.hazard + 10] = 0.6;
    let avoid = execute(&avoid_world, &avoid_observation, 99).expect("small avoid should evaluate");
    assert_eq!(avoid.strategy, BaselineStrategyState::Avoid);
    assert_eq!(avoid.target_bin, 10);
    assert_eq!(avoid.draws, 1);
    close(
        avoid.slot.strategy_timer_seconds,
        0.352_053_050_359_245_4,
        1.0e-15,
    );
    close(f64::from(avoid.slot.turn), 0.5, 0.0);
    assert!(avoid.slot.boost);
    assert_eq!(avoid.rng.state_hex, "0x01806cc5");

    let medium_world = world(vec![
        controlled(40),
        snake(
            701,
            SnakeKind::External,
            20,
            WorldPoint { x: 100.0, y: 100.0 },
            0.0,
            10.0,
        ),
    ]);
    let medium = execute(&medium_world, &observation(0.2, 0.8, 0.7, 0.9), 123)
        .expect("medium cutoff should evaluate");
    assert_eq!(medium.stage, BaselineLifeStage::Medium);
    assert_eq!(medium.strategy, BaselineStrategyState::Seek);
    assert_eq!(medium.target_id, Some(701));
    assert_eq!(medium.target_bin, 10);
    assert_eq!(medium.draws, 0);
    close(f64::from(medium.slot.turn), 0.5, 0.0);
    assert!(medium.slot.boost);
    assert_eq!(medium.rng.state_hex, "0x0000007b");

    let encircle_world = world(vec![
        snake(
            700,
            SnakeKind::Baseline,
            60,
            WorldPoint { x: 0.0, y: 0.0 },
            0.3,
            120.0,
        ),
        snake(
            701,
            SnakeKind::External,
            20,
            WorldPoint { x: 100.0, y: 0.0 },
            0.0,
            10.0,
        ),
    ]);
    let encircle = execute(&encircle_world, &observation(0.2, 0.8, 0.7, 0.9), 321)
        .expect("medium encircle should evaluate");
    assert_eq!(encircle.target_id, Some(701));
    assert_eq!(encircle.target_bin, 10);
    assert_eq!(encircle.draws, 0);
    close(f64::from(encircle.slot.turn), 0.5, 0.0);
    assert!(encircle.slot.boost);
    assert_eq!(encircle.rng.state_hex, "0x00000141");

    let mut boost_world = world(vec![controlled(40)]);
    boost_world.snakes[0].points = 10.0;
    let random_boost = execute(&boost_world, &observation(0.2, 0.8, 0.7, 0.9), 1)
        .expect("medium random boost should evaluate");
    assert_eq!(random_boost.strategy, BaselineStrategyState::Boost);
    assert_eq!(random_boost.target_bin, 8);
    assert_eq!(random_boost.draws, 2);
    close(
        random_boost.slot.strategy_timer_seconds,
        0.203_149_485_634_639_86,
        1.0e-15,
    );
    assert!(random_boost.slot.boost);
    assert_eq!(random_boost.rng.state_hex, "0x04080601");

    let large_world = world(vec![
        controlled(100),
        snake(
            720,
            SnakeKind::External,
            20,
            WorldPoint { x: 100.0, y: 0.0 },
            0.3,
            10.0,
        ),
        snake(
            710,
            SnakeKind::External,
            20,
            WorldPoint { x: 0.0, y: 100.0 },
            0.8,
            12.0,
        ),
    ]);
    let large = execute(&large_world, &observation(0.2, 0.8, 0.7, 0.9), 555)
        .expect("large crowd should evaluate");
    assert_eq!(large.stage, BaselineLifeStage::Large);
    assert_eq!(large.strategy, BaselineStrategyState::Seek);
    assert_eq!(large.target_bin, 10);
    assert_eq!(large.draws, 0);
    close(f64::from(large.slot.turn), 0.5, 0.0);
    assert!(!large.slot.boost);
    assert_eq!(large.rng.state_hex, "0x0000022b");
}

#[test]
fn medium_target_scan_is_stable_id_ordered_and_does_not_rewrite_durable_roam() {
    fn fixture(reverse: bool) -> WorldState {
        let controlled = snake(
            700,
            SnakeKind::Baseline,
            40,
            WorldPoint { x: 0.0, y: 0.0 },
            0.0,
            120.0,
        );
        let low = snake(
            600,
            SnakeKind::External,
            20,
            WorldPoint { x: 100.0, y: 0.0 },
            0.0,
            0.0,
        );
        let high = snake(
            800,
            SnakeKind::External,
            20,
            WorldPoint { x: -100.0, y: 0.0 },
            0.0,
            0.0,
        );
        if reverse {
            world(vec![controlled, high, low])
        } else {
            world(vec![controlled, low, high])
        }
    }
    let values = observation(0.0, 0.8, 0.7, 0.9);
    let forward = execute(&fixture(false), &values, 123).expect("forward target");
    let reversed = execute(&fixture(true), &values, 123).expect("reversed target");
    assert_eq!(forward, reversed);
    assert_eq!(forward.target_id, Some(600));
    assert_eq!(forward.strategy, BaselineStrategyState::Roam);
    assert_eq!(forward.effective, BaselineStrategyState::Seek);
    assert_eq!(forward.draws, 0);
    assert_eq!(forward.slot.wander_timer_seconds, 0.0);
}

#[test]
fn all_vetoed_bins_use_best_clearance_and_small_policy_never_attack_boosts() {
    let fixture = world(vec![snake(
        700,
        SnakeKind::Baseline,
        12,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    let layout = SensorLayout::new(16).expect("fixture layout");
    let mut values = observation(1.0, -0.9, 0.8, 0.9);
    values[layout.offsets.hazard + 6] = -0.5;
    let result = execute(&fixture, &values, 10).expect("all-veto fallback");
    assert_eq!(result.strategy, BaselineStrategyState::Avoid);
    assert_eq!(result.target_bin, 6);
    assert!(!result.slot.boost);
}

#[test]
fn large_centroid_is_container_order_independent() {
    fn fixture(reverse: bool) -> WorldState {
        let mut entries = vec![
            snake(
                700,
                SnakeKind::Baseline,
                100,
                WorldPoint { x: 0.0, y: 0.0 },
                0.2,
                120.0,
            ),
            snake(
                500,
                SnakeKind::External,
                20,
                WorldPoint { x: 125.25, y: 10.5 },
                0.0,
                1.0,
            ),
            snake(
                900,
                SnakeKind::External,
                20,
                WorldPoint {
                    x: -12.75,
                    y: 175.125,
                },
                0.0,
                1.0,
            ),
            snake(
                800,
                SnakeKind::External,
                20,
                WorldPoint { x: 42.0, y: -80.75 },
                0.0,
                1.0,
            ),
        ];
        if reverse {
            let controlled = entries.remove(0);
            entries.reverse();
            entries.insert(0, controlled);
        }
        world(entries)
    }
    let values = observation(0.2, 0.8, 0.7, 0.9);
    let forward = execute(&fixture(false), &values, 555).expect("forward crowd");
    let reversed = execute(&fixture(true), &values, 555).expect("reversed crowd");
    assert_eq!(forward, reversed);
}

#[test]
fn validation_fails_without_mutating_sources_or_exposing_ready_output() {
    let world = world(vec![snake(
        700,
        SnakeKind::Baseline,
        12,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    let before_world = world.clone();
    let slot = runtime(700);
    let before_slot = slot;
    let rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(42.0).export_state(),
    };
    let before_rng = rng.clone();
    let layout = SensorLayout::new(16).expect("fixture layout");
    let mut invalid = observation(0.0, 0.8, 0.7, 0.9);
    invalid[layout.offsets.food] = f32::NAN;
    let before_observation = invalid.clone();
    let mut workspace = BaselineControlWorkspace::new();
    let error = workspace
        .prepare(
            &world,
            0,
            &slot,
            &rng,
            &invalid,
            &layout,
            BaselineControlConfig::typescript_defaults(),
            1.0 / 60.0,
        )
        .expect_err("non-finite observation must fail");
    assert_eq!(
        error,
        BaselineControlError::NonFiniteObservation {
            index: layout.offsets.food
        }
    );
    assert!(!workspace.is_ready());
    assert_eq!(world, before_world);
    assert_eq!(slot, before_slot);
    assert_eq!(rng, before_rng);
    assert_eq!(invalid.len(), before_observation.len());
    assert!(invalid
        .iter()
        .zip(before_observation.iter())
        .all(|(actual, before)| actual.to_bits() == before.to_bits()));
}

#[test]
fn impossible_strategy_timer_and_wander_states_are_rejected() {
    let mut fixture = world(vec![snake(
        700,
        SnakeKind::Baseline,
        40,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    let values = observation(0.2, 0.8, 0.7, 0.9);
    let layout = SensorLayout::new(16).expect("fixture layout");
    let rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(42.0).export_state(),
    };
    let mut workspace = BaselineControlWorkspace::new();
    let cases = [
        (BaselineStrategyState::Roam, 5.0, 0.0, 0.0),
        (BaselineStrategyState::Seek, 0.1, 0.0, 0.0),
        (BaselineStrategyState::Avoid, 0.0, 0.0, 0.0),
        (BaselineStrategyState::Avoid, 0.71, 0.0, 0.0),
        (BaselineStrategyState::Boost, 0.0, 0.0, 0.0),
        (BaselineStrategyState::Boost, 0.41, 0.0, 0.0),
        (BaselineStrategyState::Roam, 0.0, 0.31, 0.0),
        (BaselineStrategyState::Roam, 0.0, 0.0, 2.01),
    ];
    for (strategy, timer, wander_angle, wander_timer) in cases {
        fixture.snakes[0].baseline_strategy = Some(strategy);
        let mut slot = runtime(700);
        slot.strategy_timer_seconds = timer;
        slot.wander_angle = wander_angle;
        slot.wander_timer_seconds = wander_timer;
        let error = workspace
            .prepare(
                &fixture,
                0,
                &slot,
                &rng,
                &values,
                &layout,
                BaselineControlConfig::typescript_defaults(),
                1.0 / 60.0,
            )
            .expect_err("impossible strategy/lifecycle pair must fail");
        assert_eq!(error, BaselineControlError::InvalidSlotState(0));
        assert!(!workspace.is_ready());
    }
}

#[test]
fn finite_extreme_distance_and_centroid_math_fails_closed() {
    let layout = SensorLayout::new(16).expect("fixture layout");
    let values = observation(0.2, 0.8, 0.7, 0.9);
    let slot = runtime(700);
    let rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(42.0).export_state(),
    };
    let mut workspace = BaselineControlWorkspace::new();

    let mut medium = world(vec![snake(
        700,
        SnakeKind::Baseline,
        40,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    medium.snakes[0].radius = f64::MAX;
    assert_eq!(
        workspace
            .prepare(
                &medium,
                0,
                &slot,
                &rng,
                &values,
                &layout,
                BaselineControlConfig::typescript_defaults(),
                1.0 / 60.0,
            )
            .expect_err("overflowing medium radius must fail"),
        BaselineControlError::NonFiniteDerived
    );
    assert!(!workspace.is_ready());

    let extreme = WorldPoint {
        x: -1.0e308,
        y: 0.0,
    };
    let large = world(vec![
        snake(700, SnakeKind::Baseline, 80, extreme, 0.0, 120.0),
        snake(701, SnakeKind::External, 20, extreme, 0.0, 10.0),
        snake(702, SnakeKind::External, 20, extreme, 0.0, 10.0),
    ]);
    assert_eq!(
        workspace
            .prepare(
                &large,
                0,
                &slot,
                &rng,
                &values,
                &layout,
                BaselineControlConfig::typescript_defaults(),
                1.0 / 60.0,
            )
            .expect_err("overflowing centroid sum must fail"),
        BaselineControlError::NonFiniteDerived
    );
    assert!(!workspace.is_ready());
}

#[test]
fn exact_life_stage_edges_and_random_boost_output_policies_are_explicit() {
    let values = observation(0.2, 0.8, 0.7, 0.9);
    for (length, expected) in [
        (24, BaselineLifeStage::Small),
        (25, BaselineLifeStage::Medium),
        (79, BaselineLifeStage::Medium),
        (80, BaselineLifeStage::Large),
    ] {
        let fixture = world(vec![snake(
            700,
            SnakeKind::Baseline,
            length,
            WorldPoint { x: 0.0, y: 0.0 },
            0.0,
            120.0,
        )]);
        assert_eq!(
            execute(&fixture, &values, 123)
                .expect("life-stage edge should evaluate")
                .stage,
            expected
        );
    }

    let mut small = world(vec![snake(
        700,
        SnakeKind::Baseline,
        24,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    small.snakes[0].points = 10.0;
    let small_boost = execute(&small, &values, 1).expect("small random boost");
    assert_eq!(small_boost.strategy, BaselineStrategyState::Boost);
    assert!(
        !small_boost.slot.boost,
        "small policy permits escape boost only"
    );

    let mut large = world(vec![snake(
        700,
        SnakeKind::Baseline,
        80,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    large.snakes[0].points = 10.0;
    let large_boost = execute(&large, &values, 1).expect("large random boost");
    assert_eq!(large_boost.strategy, BaselineStrategyState::Boost);
    assert!(large_boost.slot.boost);
}

#[test]
fn repeated_evaluations_reuse_canonical_order_capacity() {
    let fixture = world(vec![
        snake(
            700,
            SnakeKind::Baseline,
            100,
            WorldPoint { x: 0.0, y: 0.0 },
            0.0,
            120.0,
        ),
        snake(
            701,
            SnakeKind::External,
            20,
            WorldPoint { x: 100.0, y: 100.0 },
            0.0,
            10.0,
        ),
    ]);
    let values = observation(0.2, 0.8, 0.7, 0.9);
    let layout = SensorLayout::new(16).expect("fixture layout");
    let slot = runtime(700);
    let rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(555.0).export_state(),
    };
    let mut workspace = BaselineControlWorkspace::new();
    let warmed = workspace
        .prepare(
            &fixture,
            0,
            &slot,
            &rng,
            &values,
            &layout,
            BaselineControlConfig::typescript_defaults(),
            1.0 / 60.0,
        )
        .expect("warm evaluation")
        .diagnostics()
        .canonical_order_capacity;
    for _ in 0..24 {
        let diagnostics = workspace
            .prepare(
                &fixture,
                0,
                &slot,
                &rng,
                &values,
                &layout,
                BaselineControlConfig::typescript_defaults(),
                1.0 / 60.0,
            )
            .expect("reused evaluation")
            .diagnostics();
        assert_eq!(diagnostics.canonical_order_capacity, warmed);
        assert!(diagnostics.canonical_order_capacity >= fixture.snakes.len());
    }
}

#[test]
fn timed_states_and_failed_boost_chance_preserve_typescript_draw_boundaries() {
    let layout = SensorLayout::new(16).expect("fixture layout");
    let mut fixture = world(vec![snake(
        700,
        SnakeKind::Baseline,
        40,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        120.0,
    )]);
    let values = observation(0.0, 0.8, 0.7, 0.9);
    fixture.snakes[0].baseline_strategy = Some(BaselineStrategyState::Avoid);
    let mut slot = runtime(700);
    slot.strategy_timer_seconds = 0.2;
    let rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(77.0).export_state(),
    };
    let mut workspace = BaselineControlWorkspace::new();
    let held = workspace
        .prepare(
            &fixture,
            0,
            &slot,
            &rng,
            &values,
            &layout,
            BaselineControlConfig::typescript_defaults(),
            1.0 / 60.0,
        )
        .expect("active avoid timer");
    assert_eq!(held.next_strategy(), BaselineStrategyState::Avoid);
    close(
        held.next_slot().strategy_timer_seconds,
        0.2 - 1.0 / 60.0,
        1.0e-15,
    );
    assert_eq!(held.diagnostics().uniform_draws, 0);
    assert_eq!(held.next_rng().state, rng.state);

    slot.strategy_timer_seconds = 1.0 / 60.0;
    let expired = workspace
        .prepare(
            &fixture,
            0,
            &slot,
            &rng,
            &values,
            &layout,
            BaselineControlConfig::typescript_defaults(),
            1.0 / 60.0,
        )
        .expect("exact timer expiry");
    assert_eq!(expired.next_strategy(), BaselineStrategyState::Roam);
    assert_eq!(expired.next_slot().strategy_timer_seconds, 0.0);
    assert_eq!(expired.diagnostics().uniform_draws, 2);

    fixture.snakes[0].baseline_strategy = Some(BaselineStrategyState::Roam);
    fixture.snakes[0].points = 10.0;
    let slot = runtime(700);
    let failed_chance_rng = BaselineRngState {
        slot: 0,
        state: StatefulRng::new(555.0).export_state(),
    };
    let seek_values = observation(0.2, 0.8, 0.7, 0.9);
    let failed_chance = workspace
        .prepare(
            &fixture,
            0,
            &slot,
            &failed_chance_rng,
            &seek_values,
            &layout,
            BaselineControlConfig::typescript_defaults(),
            1.0 / 60.0,
        )
        .expect("failed boost chance");
    assert_eq!(failed_chance.next_strategy(), BaselineStrategyState::Seek);
    assert_eq!(failed_chance.diagnostics().uniform_draws, 1);
    assert_eq!(failed_chance.next_rng().state.state_hex, "0x08e92329");
}
