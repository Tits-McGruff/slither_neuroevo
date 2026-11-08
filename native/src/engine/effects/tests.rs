use super::*;
use crate::engine::collision::{CollisionConfig, CollisionWorkspace};
use crate::engine::food::FoodWorkspace;
use crate::engine::movement::MovementWorkspace;
use crate::engine::rng::{
    SerializedRngState, LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM, LEGACY_TYPESCRIPT_GAUSSIAN_VERSION,
    RNG_ALGORITHM, RNG_VERSION,
};
use crate::engine::spatial::IndexedPelletWorld;
use crate::engine::state::{
    BaselineRngState, BodyRange, WorldState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START,
    EXTERNAL_ENTITY_ID_START, RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION,
};

const DT: f64 = 1.0 / 180.0;

#[derive(Clone, Debug, PartialEq)]
struct EffectResult {
    pellets: Vec<PelletState>,
    rng: RngStateBundle,
    allocators: AllocatorState,
    baseline_deaths: Vec<BaselineDeathEvent>,
    diagnostics: EffectDiagnostics,
}

fn snake(
    id: u64,
    kind: SnakeKind,
    position: WorldPoint,
    direction: f64,
    length: usize,
) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
        kind,
        alive: true,
        population_slot: (kind == SnakeKind::Evolved).then_some(0),
        brain: None,
        baseline_slot: (kind == SnakeKind::Baseline).then_some(0),
        baseline_strategy: None,
        position,
        previous_position: position,
        direction,
        radius: 9.0,
        speed: 165.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 10.0,
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
    }
}

fn line_body(position: WorldPoint, direction: f64, length: usize) -> Vec<WorldPoint> {
    (0..length)
        .map(|index| WorldPoint {
            x: position.x - direction.cos() * index as f64 * 7.5,
            y: position.y - direction.sin() * index as f64 * 7.5,
        })
        .collect()
}

fn pack_world(entries: Vec<(SnakeState, Vec<WorldPoint>)>) -> WorldState {
    let mut world = WorldState::default();
    for (mut snake, body) in entries {
        snake.body = BodyRange {
            start: world.body_points.len(),
            len: body.len(),
        };
        snake.position = body[0];
        snake.previous_position = body[0];
        world.body_points.extend(body);
        world.snakes.push(snake);
    }
    world
}

fn body_collision_world(victim_kind: SnakeKind, reverse: bool) -> WorldState {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 100.0, y: 0.0 };
    let victim = (
        snake(7, victim_kind, victim_position, 0.0, 12),
        line_body(victim_position, 0.0, 12),
    );
    let owner = (
        snake(20, SnakeKind::Evolved, owner_position, 0.0, 15),
        line_body(owner_position, 0.0, 15),
    );
    if reverse {
        pack_world(vec![owner, victim])
    } else {
        pack_world(vec![victim, owner])
    }
}

#[test]
fn discard_cold_corpse_consumes_all_draws_and_allocates_only_retained_ids() {
    let snake = snake(
        7,
        SnakeKind::Evolved,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        12,
    );
    let body = line_body(snake.position, 0.0, 12);
    let source_rng = rng_bundle();
    let source_ids = allocators();
    let run = |existing, maximum, overflow, ids: &AllocatorState| {
        prepare_single_corpse_with_policy(
            &source_rng,
            ids,
            &snake,
            &body,
            existing,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            DeathDropConfig::typescript_defaults(),
            maximum,
            EvolvedCorpseMode::Normal,
            overflow,
        )
    };
    let full = run(99, 1000, PelletOverflowMode::Fail, &source_ids).unwrap();
    let partial = run(99, 100, PelletOverflowMode::DiscardNewExcess, &source_ids).unwrap();
    assert_eq!(partial.pellets, full.pellets[..1]);
    assert_eq!(partial.rng, full.rng);
    assert_eq!(
        partial.allocators.next_entity_id,
        source_ids.next_entity_id + 1
    );
    assert_eq!(partial.discarded_count, full.pellets.len() - 1);
    assert_eq!(
        partial.discarded_value,
        full.pellets[1..]
            .iter()
            .map(|pellet| pellet.value)
            .sum::<f64>()
    );
    let mut exhausted = source_ids.clone();
    exhausted.next_entity_id = EXTERNAL_ENTITY_ID_START;
    let discarded = run(100, 100, PelletOverflowMode::DiscardNewExcess, &exhausted).unwrap();
    assert!(discarded.pellets.is_empty());
    assert_eq!(discarded.allocators, exhausted);
    assert_eq!(discarded.rng, full.rng);
    assert_eq!(discarded.discarded_count, full.pellets.len());
    assert_eq!(
        discarded.discarded_value,
        full.pellets.iter().map(|pellet| pellet.value).sum::<f64>()
    );
    assert!(matches!(
        run(99, 100, PelletOverflowMode::Fail, &source_ids),
        Err(EffectError::PelletCapacityExceeded { .. })
    ));
    assert!(matches!(
        run(101, 100, PelletOverflowMode::DiscardNewExcess, &source_ids),
        Err(EffectError::PelletCapacityExceeded { .. })
    ));
}

#[test]
fn compensated_corpse_allows_zero_and_caps_actual_randomized_value() {
    let source_rng = rng_bundle();
    let source_ids = allocators();
    let movement = MovementConfig::typescript_defaults();
    let food = FoodConfig::typescript_defaults();
    let death = DeathDropConfig::typescript_defaults();
    for length in [5, 6, 12, 50, 120, 1000] {
        let snake = snake(
            7,
            SnakeKind::Evolved,
            WorldPoint { x: 0.0, y: 0.0 },
            0.0,
            length,
        );
        let body = line_body(snake.position, 0.0, length);
        let plan = death_plan(
            0,
            length,
            movement,
            food,
            death,
            EvolvedCorpseMode::SubtractReplacementMass,
        )
        .unwrap();
        let result = prepare_single_corpse_with_policy(
            &source_rng,
            &source_ids,
            &snake,
            &body,
            0,
            movement,
            food,
            death,
            10000,
            EvolvedCorpseMode::SubtractReplacementMass,
            PelletOverflowMode::Fail,
        )
        .unwrap();
        let total: f64 = result.pellets.iter().map(|pellet| pellet.value).sum();
        assert!(
            total <= plan.value_budget.unwrap(),
            "length={length}, value={total}"
        );
        assert!(result.pellets.iter().all(|pellet| pellet.value > 0.0));
        if length == 5 {
            assert!(result.pellets.is_empty());
            assert_eq!(result.rng, source_rng);
            assert_eq!(result.allocators, source_ids);
        }
        let none = prepare_single_corpse_with_policy(
            &source_rng,
            &source_ids,
            &snake,
            &body,
            0,
            movement,
            food,
            death,
            10000,
            EvolvedCorpseMode::None,
            PelletOverflowMode::Fail,
        )
        .unwrap();
        assert!(none.pellets.is_empty());
        assert_eq!(none.rng, source_rng);
        assert_eq!(none.allocators, source_ids);
    }
}

fn rng_bundle() -> RngStateBundle {
    RngStateBundle {
        baseline_behaviors: Vec::new(),
        evolved_respawns: Vec::new(),
        version: RNG_BUNDLE_VERSION,
        world: StatefulRng::new(11.0).export_state(),
        evolution: StatefulRng::new(22.0).export_state(),
        external_controller: StatefulRng::new(33.0).export_state(),
        baselines: vec![BaselineRngState {
            slot: 0,
            state: StatefulRng::new(44.0).export_state(),
        }],
    }
}

fn allocators() -> AllocatorState {
    AllocatorState {
        version: ALLOCATOR_VERSION,
        next_entity_id: 1_000,
        next_brain_id: 1,
        next_genome_id: 1,
        next_controller_lease_id: 1,
        next_frame_v1_id: 1,
        next_external_id: EXTERNAL_ENTITY_ID_START,
        next_baseline_id: BASELINE_ENTITY_ID_START,
        next_resurrected_id: RESURRECTED_ENTITY_ID_START,
    }
}

fn indexed(world: &WorldState) -> IndexedPelletWorld<'_> {
    IndexedPelletWorld::build(world, 120.0, 1_000_000).expect("fixture world should index")
}

fn execute(
    world: &WorldState,
    rng: &RngStateBundle,
    allocators: &AllocatorState,
    maximum_pellets: usize,
) -> Result<EffectResult, EffectError> {
    let mut effects_workspace = EffectWorkspace::new();
    execute_with_effect_workspace(
        world,
        rng,
        allocators,
        maximum_pellets,
        &mut effects_workspace,
    )
}

fn execute_with_effect_workspace(
    world: &WorldState,
    rng: &RngStateBundle,
    allocators: &AllocatorState,
    maximum_pellets: usize,
    effects_workspace: &mut EffectWorkspace,
) -> Result<EffectResult, EffectError> {
    let movement_config = MovementConfig::typescript_defaults();
    let indexed = indexed(world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(world, movement_config, DT, 100_000, 100_000)
        .expect("fixture movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let food = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100_000,
            100_000,
        )
        .expect("fixture food should prepare");
    let mut collision_workspace = CollisionWorkspace::new();
    let collision = collision_workspace
        .prepare(food, CollisionConfig::typescript_defaults())
        .expect("fixture collision should prepare");
    let prepared = effects_workspace.prepare(
        collision,
        rng,
        allocators,
        movement_config,
        FoodConfig::typescript_defaults(),
        DeathDropConfig::typescript_defaults(),
        maximum_pellets,
    )?;
    Ok(EffectResult {
        pellets: prepared.pellets().to_vec(),
        rng: prepared.rng().clone(),
        allocators: prepared.allocators().clone(),
        baseline_deaths: prepared.baseline_deaths().to_vec(),
        diagnostics: prepared.diagnostics(),
    })
}

fn rng_bundle_with_baselines(count: usize) -> RngStateBundle {
    let mut bundle = rng_bundle();
    bundle.baselines = (0..count)
        .map(|index| BaselineRngState {
            slot: u32::try_from(index).expect("fixture baseline slot should fit u32"),
            state: StatefulRng::new(44.0 + index as f64).export_state(),
        })
        .collect();
    bundle
}

fn close(actual: f64, expected: f64) {
    let tolerance = 1.0e-12_f64.max(expected.abs() * 1.0e-12);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn matches_retained_typescript_death_formula_rng_and_ordering() {
    let legacy = SerializedRngState {
        algorithm: RNG_ALGORITHM.to_owned(),
        version: RNG_VERSION,
        state_hex: "0x25e2bbed".to_owned(),
        gaussian_algorithm: LEGACY_TYPESCRIPT_GAUSSIAN_ALGORITHM.to_owned(),
        gaussian_version: LEGACY_TYPESCRIPT_GAUSSIAN_VERSION,
        gaussian_spare_valid: false,
        gaussian_spare_hex: None,
    };
    let mut rng = StatefulRng::from_legacy_typescript_state(&legacy)
        .expect("retained legacy uniform continuation should decode");
    let mut fixture_snake = snake(
        7,
        SnakeKind::Evolved,
        WorldPoint { x: 0.0, y: 0.0 },
        0.0,
        12,
    );
    fixture_snake.frame_v1_id = 7;
    let body: Vec<WorldPoint> = (0..12)
        .map(|index| WorldPoint {
            x: -(index as f64) * 7.5,
            y: if index % 2 == 0 { 0.0 } else { 1.25 },
        })
        .collect();
    let movement = MovementConfig::typescript_defaults();
    let food = FoodConfig::typescript_defaults();
    let death = DeathDropConfig::typescript_defaults();
    let (big_count, small_count) =
        death_pellet_counts(body.len(), movement, food, death).expect("valid fixture counts");
    assert_eq!((big_count, small_count), (2, 2));
    let mut pellets = Vec::new();
    let mut next_id = 1_000;
    realize_corpse_pellets(
        &mut pellets,
        &mut next_id,
        &fixture_snake,
        &body,
        DeathPlan {
            snake_index: 0,
            big_count,
            small_count,
            value_budget: None,
        },
        death,
        3.0,
        1.0,
        &mut rng,
        usize::MAX,
    )
    .expect("fixture death should realize");

    let expected = [
        (3.7043982185423374, 2.57913351804018, 2.6677111503202466, 1),
        (-75.60867307335138, 8.878263261169195, 3.0139878390356896, 1),
        (6.539477038197219, 12.556274109520018, 0.9609815296716988, 2),
        (
            -42.59541824180633,
            0.4477313784882426,
            0.9449886957183481,
            2,
        ),
    ];
    assert_eq!(pellets.len(), expected.len());
    for (pellet, expected) in pellets.iter().zip(expected) {
        close(pellet.position.x, expected.0);
        close(pellet.position.y, expected.1);
        close(pellet.value, expected.2);
        assert_eq!(pellet.kind, expected.3);
        assert_eq!(pellet.color, 7);
        assert_eq!(pellet.owner, Some(7));
    }
    assert_eq!(rng.export_state().state_hex, "0x78aa3ba5");
}

#[test]
fn gameplay_rng_streams_are_isolated_and_evolution_never_advances() {
    for kind in [
        SnakeKind::Evolved,
        SnakeKind::Resurrected,
        SnakeKind::External,
        SnakeKind::Baseline,
    ] {
        let world = body_collision_world(kind, false);
        let before_rng = rng_bundle();
        let before_world = world.clone();
        let before_allocators = allocators();
        let result = execute(&world, &before_rng, &before_allocators, 100_000)
            .expect("body death should realize effects");
        assert_eq!(
            world, before_world,
            "effect staging mutated source authority"
        );
        assert_eq!(before_allocators.next_entity_id, 1_000);
        assert_eq!(result.rng.evolution, before_rng.evolution);
        assert_eq!(
            result.rng.world != before_rng.world,
            matches!(kind, SnakeKind::Evolved | SnakeKind::Resurrected)
        );
        assert_eq!(
            result.rng.external_controller != before_rng.external_controller,
            kind == SnakeKind::External
        );
        assert_eq!(
            result.rng.baselines[0].state != before_rng.baselines[0].state,
            kind == SnakeKind::Baseline
        );
        assert_eq!(
            result.baseline_deaths,
            if kind == SnakeKind::Baseline {
                vec![BaselineDeathEvent {
                    slot: 0,
                    snake_id: 7,
                }]
            } else {
                Vec::new()
            }
        );
        assert!(result.diagnostics.corpse_big_pellets > 0);
        assert_eq!(
            result.allocators.next_entity_id,
            1_000 + result.pellets.len() as u64
        );
    }
}

#[test]
fn boost_request_uses_two_owner_stream_draws_and_preserves_other_streams() {
    let position = WorldPoint { x: 0.0, y: 0.0 };
    let mut external = snake(7, SnakeKind::External, position, 0.0, 8);
    external.input_boost = true;
    let world = pack_world(vec![(external, line_body(position, 0.0, 8))]);
    let before = rng_bundle();
    let allocators = allocators();
    let result = execute(&world, &before, &allocators, 100_000)
        .expect("one boost-tail request should realize");

    assert_eq!(result.pellets.len(), 1);
    let pellet = &result.pellets[0];
    let mut expected_rng = StatefulRng::from_state(&before.external_controller)
        .expect("fixture external stream should restore");
    let expected_x = -60.5 - 10.0 + expected_rng.next_f64() * 20.0;
    let expected_y = -10.0 + expected_rng.next_f64() * 20.0;
    close(pellet.position.x, expected_x);
    close(pellet.position.y, expected_y);
    close(pellet.value, 0.65);
    assert_eq!(pellet.kind, BOOST_PELLET_KIND);
    assert_eq!(pellet.color, 7);
    assert_eq!(pellet.owner, Some(7));
    assert_eq!(result.rng.external_controller, expected_rng.export_state());
    assert_eq!(result.rng.world, before.world);
    assert_eq!(result.rng.evolution, before.evolution);
    assert_eq!(result.rng.baselines, before.baselines);
    assert_eq!(result.allocators.next_entity_id, 1_001);
    assert_eq!(result.diagnostics.boost_pellets, 1);
}

#[test]
fn stable_snake_order_makes_container_order_irrelevant() {
    let rng = rng_bundle();
    let allocators = allocators();
    let forward = execute(
        &body_collision_world(SnakeKind::External, false),
        &rng,
        &allocators,
        100_000,
    )
    .expect("forward effects");
    let reversed = execute(
        &body_collision_world(SnakeKind::External, true),
        &rng,
        &allocators,
        100_000,
    )
    .expect("reversed effects");
    assert_eq!(forward, reversed);
}

#[test]
fn two_same_stream_deaths_use_stable_id_draw_and_pellet_order() {
    fn world(reverse: bool) -> WorldState {
        let entries = vec![
            (
                snake(
                    7,
                    SnakeKind::Evolved,
                    WorldPoint { x: 0.0, y: 0.0 },
                    0.0,
                    12,
                ),
                line_body(WorldPoint { x: 0.0, y: 0.0 }, 0.0, 12),
            ),
            (
                snake(
                    20,
                    SnakeKind::Evolved,
                    WorldPoint { x: 100.0, y: 0.0 },
                    0.0,
                    15,
                ),
                line_body(WorldPoint { x: 100.0, y: 0.0 }, 0.0, 15),
            ),
            (
                snake(
                    8,
                    SnakeKind::Evolved,
                    WorldPoint { x: 0.0, y: 40.0 },
                    0.0,
                    12,
                ),
                line_body(WorldPoint { x: 0.0, y: 40.0 }, 0.0, 12),
            ),
            (
                snake(
                    21,
                    SnakeKind::Evolved,
                    WorldPoint { x: 100.0, y: 40.0 },
                    0.0,
                    15,
                ),
                line_body(WorldPoint { x: 100.0, y: 40.0 }, 0.0, 15),
            ),
        ];
        if reverse {
            pack_world(entries.into_iter().rev().collect())
        } else {
            pack_world(entries)
        }
    }

    let rng = rng_bundle();
    let allocators = allocators();
    let forward = execute(&world(false), &rng, &allocators, 100_000)
        .expect("two forward deaths should realize");
    let reversed = execute(&world(true), &rng, &allocators, 100_000)
        .expect("two reversed deaths should realize");
    assert_eq!(forward, reversed);
    assert_eq!(forward.diagnostics.corpse_big_pellets, 4);
    assert_eq!(forward.diagnostics.corpse_small_pellets, 4);
    assert_eq!(
        forward
            .pellets
            .iter()
            .map(|pellet| pellet.owner)
            .collect::<Vec<_>>(),
        vec![
            Some(7),
            Some(7),
            Some(7),
            Some(7),
            Some(8),
            Some(8),
            Some(8),
            Some(8),
        ]
    );
    assert_eq!(
        forward
            .pellets
            .iter()
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>(),
        (1_000..1_008).collect::<Vec<_>>()
    );
}

#[test]
fn empty_effect_boundary_preserves_rng_and_allocator_exactly() {
    let first = snake(
        1,
        SnakeKind::Evolved,
        WorldPoint { x: -500.0, y: 0.0 },
        0.0,
        5,
    );
    let second = snake(
        2,
        SnakeKind::Evolved,
        WorldPoint { x: 500.0, y: 0.0 },
        0.0,
        5,
    );
    let world = pack_world(vec![
        (first, line_body(WorldPoint { x: -500.0, y: 0.0 }, 0.0, 5)),
        (second, line_body(WorldPoint { x: 500.0, y: 0.0 }, 0.0, 5)),
    ]);
    let rng = rng_bundle();
    let allocators = allocators();
    let result = execute(&world, &rng, &allocators, 100_000).expect("empty effects");
    assert!(result.pellets.is_empty());
    assert_eq!(result.rng, rng);
    assert_eq!(result.allocators, allocators);
    assert_eq!(result.diagnostics.total_pellets, 0);
}

#[test]
fn wall_killed_baseline_notifies_without_drops_or_rng_draws() {
    let position = WorldPoint { x: 3_495.0, y: 0.0 };
    let baseline = snake(9, SnakeKind::Baseline, position, 0.0, 8);
    let world = pack_world(vec![(baseline, line_body(position, 0.0, 8))]);
    let rng = rng_bundle();
    let allocators = allocators();
    let result = execute(&world, &rng, &allocators, 100_000).expect("wall effects");
    assert!(result.pellets.is_empty());
    assert_eq!(result.rng, rng);
    assert_eq!(result.allocators, allocators);
    assert_eq!(
        result.baseline_deaths,
        vec![BaselineDeathEvent {
            slot: 0,
            snake_id: 9,
        }]
    );
}

#[test]
fn pellet_capacity_and_allocator_exhaustion_reject_before_publication() {
    let world = body_collision_world(SnakeKind::Evolved, false);
    let rng = rng_bundle();
    let allocators = allocators();
    let error = execute(&world, &rng, &allocators, 1)
        .expect_err("complete death effects should exceed one pellet");
    assert!(matches!(
        error,
        EffectError::PelletCapacityExceeded {
            required,
            maximum: 1
        } if required > 1
    ));
    assert_eq!(rng, rng_bundle());
    assert_eq!(allocators, self::allocators());

    let mut exhausted = allocators.clone();
    exhausted.next_entity_id = EXTERNAL_ENTITY_ID_START - 1;
    let error = execute(&world, &rng, &exhausted, 100_000)
        .expect_err("entity domain should reject the whole reservation");
    assert!(matches!(
        error,
        EffectError::Allocator(StateError::IdExhausted { kind: "entity", .. })
    ));
    assert_eq!(exhausted.next_entity_id, EXTERNAL_ENTITY_ID_START - 1);
}

#[test]
fn malformed_baseline_stream_is_rejected_without_ready_output() {
    let world = body_collision_world(SnakeKind::Baseline, false);
    let indexed = indexed(&world);
    let movement_config = MovementConfig::typescript_defaults();
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, movement_config, DT, 100_000, 100_000)
        .expect("movement");
    let mut food_workspace = FoodWorkspace::new();
    let food = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100_000,
            100_000,
        )
        .expect("food");
    let mut collision_workspace = CollisionWorkspace::new();
    let collision = collision_workspace
        .prepare(food, CollisionConfig::typescript_defaults())
        .expect("collision");
    let mut rng = rng_bundle();
    rng.baselines[0].slot = 1;
    let mut workspace = EffectWorkspace::new();
    let error = workspace
        .prepare(
            collision,
            &rng,
            &allocators(),
            movement_config,
            FoodConfig::typescript_defaults(),
            DeathDropConfig::typescript_defaults(),
            100_000,
        )
        .expect_err("non-dense baseline RNG should reject");
    assert_eq!(
        error,
        EffectError::NonDenseBaselineRng { index: 0, slot: 1 }
    );
    assert!(!workspace.is_ready());
}

#[test]
fn post_draw_failure_leaves_inputs_unchanged_and_next_prepare_clears_scratch() {
    let world = body_collision_world(SnakeKind::Evolved, false);
    let source_world = world.clone();
    let indexed = indexed(&world);
    let movement_config = MovementConfig::typescript_defaults();
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, movement_config, DT, 100_000, 100_000)
        .expect("movement");
    let mut food_workspace = FoodWorkspace::new();
    let food = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100_000,
            100_000,
        )
        .expect("food");
    let mut collision_workspace = CollisionWorkspace::new();
    let collision = collision_workspace
        .prepare(food, CollisionConfig::typescript_defaults())
        .expect("collision");
    let rng = rng_bundle();
    let source_rng = rng.clone();
    let allocators = allocators();
    let source_allocators = allocators.clone();
    let mut workspace = EffectWorkspace::new();
    let mut extreme = DeathDropConfig::typescript_defaults();
    extreme.jitter = f64::MAX;
    extreme.cluster_jitter = f64::MAX;
    let error = workspace
        .prepare(
            collision,
            &rng,
            &allocators,
            movement_config,
            FoodConfig::typescript_defaults(),
            extreme,
            100_000,
        )
        .expect_err("finite extreme jitter should fail after the first draw");
    assert!(matches!(error, EffectError::NonFiniteGenerated { .. }));
    assert!(!workspace.is_ready());
    assert_eq!(world, source_world);
    assert_eq!(rng, source_rng);
    assert_eq!(allocators, source_allocators);

    let prepared = workspace
        .prepare(
            collision,
            &rng,
            &allocators,
            movement_config,
            FoodConfig::typescript_defaults(),
            DeathDropConfig::typescript_defaults(),
            100_000,
        )
        .expect("valid retry should clear rejected partial scratch");
    assert_eq!(prepared.pellets().len(), 4);
    assert_eq!(prepared.pellets()[0].id, 1_000);
    assert!(workspace.is_ready());
}

#[test]
fn checked_reservation_and_effect_buffers_reuse_warmed_capacity() {
    let mut values = Vec::with_capacity(5);
    reserve_for(&mut values, 10, "reservation regression")
        .expect("checked reserve should cover the required final length");
    assert!(values.capacity() >= 10);
    let reserved_capacity = values.capacity();
    values.extend(0..10);
    assert_eq!(values.capacity(), reserved_capacity);

    let world = body_collision_world(SnakeKind::Baseline, false);
    let indexed = indexed(&world);
    let movement_config = MovementConfig::typescript_defaults();
    let rng = rng_bundle();
    let allocators = allocators();
    let mut movement_workspace = MovementWorkspace::new();
    let mut food_workspace = FoodWorkspace::new();
    let mut collision_workspace = CollisionWorkspace::new();
    let mut effects_workspace = EffectWorkspace::new();
    let mut warmed: Option<EffectDiagnostics> = None;
    for _ in 0..24 {
        let movement = movement_workspace
            .prepare(&world, movement_config, DT, 100_000, 100_000)
            .expect("movement");
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("food");
        let collision = collision_workspace
            .prepare(food, CollisionConfig::typescript_defaults())
            .expect("collision");
        let prepared = effects_workspace
            .prepare(
                collision,
                &rng,
                &allocators,
                movement_config,
                FoodConfig::typescript_defaults(),
                DeathDropConfig::typescript_defaults(),
                100_000,
            )
            .expect("effects");
        let diagnostics = prepared.diagnostics();
        assert!(diagnostics.snake_order_capacity >= world.snakes.len());
        assert!(diagnostics.death_plan_capacity >= 1);
        assert!(diagnostics.pellet_capacity >= diagnostics.total_pellets);
        assert!(diagnostics.baseline_event_capacity >= 1);
        assert!(diagnostics.baseline_rng_capacity >= rng.baselines.len());
        if let Some(expected) = warmed {
            assert_eq!(diagnostics, expected);
        } else {
            warmed = Some(diagnostics);
        }
    }
}

#[test]
fn generated_effects_reuse_rng_scratch_after_baseline_count_shrinks() {
    let world = body_collision_world(SnakeKind::Baseline, false);
    let allocators = allocators();
    let large_rng = rng_bundle_with_baselines(3);
    let small_rng = rng_bundle_with_baselines(1);
    let mut reused = EffectWorkspace::new();

    let large =
        execute_with_effect_workspace(&world, &large_rng, &allocators, 100_000, &mut reused)
            .expect("generated effects should warm three retained baseline RNG buffers");
    assert!(!large.pellets.is_empty());
    assert_eq!(large.rng.baselines.len(), 3);
    assert!(reused.rng_copy_scratch.baseline_gaussian_spares.len() >= 3);

    let expected = execute(&world, &small_rng, &allocators, 100_000)
        .expect("a fresh one-baseline effect should succeed");
    let actual =
        execute_with_effect_workspace(&world, &small_rng, &allocators, 100_000, &mut reused)
            .expect("retained larger scratch must accept a smaller baseline set");
    assert_eq!(actual.pellets, expected.pellets);
    assert_eq!(actual.rng, expected.rng);
    assert_eq!(actual.allocators, expected.allocators);
    assert_eq!(actual.baseline_deaths, expected.baseline_deaths);
    assert_eq!(
        actual.diagnostics.total_pellets,
        expected.diagnostics.total_pellets
    );
    assert_eq!(actual.rng.baselines.len(), 1);
    assert!(reused.rng_copy_scratch.baseline_gaussian_spares.len() >= 3);

    let expected_continuation = execute(&world, &actual.rng, &actual.allocators, 100_000)
        .expect("fresh continuation should succeed");
    let actual_continuation = execute_with_effect_workspace(
        &world,
        &actual.rng,
        &actual.allocators,
        100_000,
        &mut reused,
    )
    .expect("retained scratch should preserve the exact smaller-set continuation");
    assert_eq!(actual_continuation.pellets, expected_continuation.pellets);
    assert_eq!(actual_continuation.rng, expected_continuation.rng);
    assert_eq!(
        actual_continuation.allocators,
        expected_continuation.allocators
    );
    assert_eq!(
        actual_continuation.baseline_deaths,
        expected_continuation.baseline_deaths
    );
}
