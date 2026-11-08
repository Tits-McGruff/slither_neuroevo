use super::*;
use crate::engine::movement::MovementWorkspace;
use crate::engine::state::{SnakeKind, WorldState};

const DT: f64 = 1.0 / 180.0;
type NormalizedSnake = (u64, SnakeState, Vec<WorldPoint>);
type NormalizedClaim = (u64, u64, u64);
type NormalizedFoodResult = (Vec<NormalizedSnake>, Vec<NormalizedClaim>, Vec<u64>);

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2.0e-12,
        "actual {actual:.17} differs from expected {expected:.17}"
    );
}

fn snake(
    id: u64,
    body_start: usize,
    length: usize,
    position: WorldPoint,
    direction: f64,
) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(u32::try_from(id - 1).expect("fixture slot should fit")),
        brain: None,
        baseline_slot: None,
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
            start: body_start,
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

fn world_with_one(length: usize) -> WorldState {
    let position = WorldPoint { x: 0.0, y: 0.0 };
    WorldState {
        ray_cursor: Default::default(),
        snakes: vec![snake(1, 0, length, position, 0.0)],
        body_points: line_body(position, 0.0, length),
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    }
}

fn pellet(id: u64, x: f64, y: f64, value: f64) -> PelletState {
    PelletState {
        id,
        position: WorldPoint { x, y },
        value,
        kind: 0,
        color: 0,
        owner: None,
    }
}

fn index(world: &WorldState) -> IndexedPelletWorld<'_> {
    IndexedPelletWorld::build(world, 120.0, 10_000).expect("fixture indexes should build")
}

#[test]
fn no_food_finishes_the_retained_typescript_boost_fixture() {
    let mut world = world_with_one(8);
    world.snakes[0].turn = -0.5;
    world.snakes[0].input_boost = true;
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let prepared = food_workspace
        .prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        )
        .expect("food finalization should prepare");
    let staged = &prepared.snakes()[0];
    let body = prepared.body_for(staged).expect("final body should exist");

    assert!(std::ptr::eq(prepared.source_world(), &world));
    assert_eq!(prepared.movement_proposals(), movement.proposals());
    close(staged.position.x, 0.982_628_430_314_376_3);
    close(staged.position.y, -0.008_732_258_602_803_35);
    close(staged.direction, -0.008_886_399_452_323_957);
    close(staged.speed, 176.880_101_352_972_45);
    close(staged.points, 9.961_098_271_357_901);
    close(staged.target_length, 7.993_775_723_417_264);
    close(staged.radius, 9.187_161_711_298_957);
    assert_eq!(body.len(), 7);
    close(body[6].x, -44.017_367_541_676_194);
    close(body[6].y, -0.000_000_021_100_342_633_887_367);
    assert!(prepared.claims().is_empty());
    assert!(prepared.remaining_pellets().is_empty());
    assert_eq!(prepared.boost_drops().len(), 1);
}

#[test]
fn food_updates_score_target_radius_and_body_after_movement() {
    let mut world = world_with_one(5);
    world.pellets.push(pellet(20, 0.0, 0.0, 1.0));
    let authority_before = world.clone();
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let prepared = food_workspace
        .prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        )
        .expect("food should prepare");
    let staged = &prepared.snakes()[0];

    close(staged.food, 1.0);
    close(staged.points, 30.0);
    close(staged.target_length, 6.0);
    close(staged.radius, 9.095_090_486_186_674);
    assert_eq!(staged.body.len, 6);
    let body = prepared.body_for(staged).expect("grown body should exist");
    for (actual, expected) in body.iter().zip([
        WorldPoint {
            x: 0.916_666_666_666_666_7,
            y: 0.0,
        },
        WorldPoint {
            x: -6.583_333_333_333_334,
            y: 0.0,
        },
        WorldPoint {
            x: -14.083_333_333_333_334,
            y: 0.0,
        },
        WorldPoint {
            x: -21.583_333_333_333_332,
            y: 0.0,
        },
        WorldPoint {
            x: -29.083_333_333_333_332,
            y: 0.0,
        },
        WorldPoint {
            x: -36.583_333_333_333_33,
            y: 0.0,
        },
    ]) {
        close(actual.x, expected.x);
        close(actual.y, expected.y);
    }
    assert_eq!(prepared.claims().len(), 1);
    assert_eq!(prepared.claims()[0].pellet_id, 20);
    assert_eq!(prepared.claims()[0].snake_id, 1);
    assert!(prepared.remaining_pellets().is_empty());
    assert_eq!(world, authority_before);
}

#[test]
fn same_substep_food_prevents_premature_ordinary_shrink() {
    let mut world = world_with_one(5);
    world.snakes[0].target_length = 4.0;
    world.pellets.push(pellet(20, 0.0, 0.0, 1.0));
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("movement should defer ordinary shrink");
    assert_eq!(movement.snakes()[0].body.len, 5);
    let mut food_workspace = FoodWorkspace::new();
    let prepared = food_workspace
        .prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        )
        .expect("food should cancel the pending shrink");

    assert_eq!(prepared.snakes()[0].body.len, 5);
    close(prepared.snakes()[0].target_length, 5.0);
}

fn contested_world(reverse: bool) -> WorldState {
    let first_position = WorldPoint { x: 0.0, y: 0.0 };
    let second_position = WorldPoint { x: 2.0, y: 0.0 };
    let first = snake(1, 0, 5, first_position, 0.0);
    let mut second = snake(2, 5, 5, second_position, 0.0);
    let first_body = line_body(first_position, 0.0, 5);
    let second_body = line_body(second_position, 0.0, 5);
    if !reverse {
        WorldState {
            ray_cursor: Default::default(),
            snakes: vec![first, second],
            body_points: [first_body, second_body].concat(),
            pellets: vec![pellet(20, 1.0, 0.0, 1.0), pellet(10, 2.0, 0.0, 0.5)],
            controller_leases: Vec::new(),
        }
    } else {
        second.body.start = 0;
        let mut reversed_first = first;
        reversed_first.body.start = 5;
        WorldState {
            ray_cursor: Default::default(),
            snakes: vec![second, reversed_first],
            body_points: [second_body, first_body].concat(),
            pellets: vec![pellet(10, 2.0, 0.0, 0.5), pellet(20, 1.0, 0.0, 1.0)],
            controller_leases: Vec::new(),
        }
    }
}

fn normalized_food_result(prepared: PreparedFood<'_, '_>) -> NormalizedFoodResult {
    let mut snakes = prepared
        .snakes()
        .iter()
        .map(|snake| {
            let mut normalized = snake.clone();
            let body = prepared
                .body_for(snake)
                .expect("body should resolve")
                .to_vec();
            normalized.body.start = 0;
            (snake.id, normalized, body)
        })
        .collect::<Vec<_>>();
    snakes.sort_by_key(|entry| entry.0);
    let claims = prepared
        .claims()
        .iter()
        .map(|claim| {
            (
                claim.pellet_id,
                claim.snake_id,
                claim.distance_squared.to_bits(),
            )
        })
        .collect();
    let remaining = prepared
        .remaining_pellets()
        .iter()
        .map(|pellet| pellet.id)
        .collect();
    (snakes, claims, remaining)
}

#[test]
fn contested_food_uses_nearest_then_stable_id_independent_of_container_order() {
    fn execute(world: &WorldState) -> NormalizedFoodResult {
        let indexed = index(world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(world, MovementConfig::typescript_defaults(), DT, 100, 100)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("contested food should prepare");
        normalized_food_result(prepared)
    }

    let forward = execute(&contested_world(false));
    let reversed = execute(&contested_world(true));
    assert_eq!(forward, reversed);
    assert_eq!(
        forward
            .1
            .iter()
            .map(|claim| (claim.0, claim.1))
            .collect::<Vec<_>>(),
        vec![(10, 2), (20, 1)]
    );

    let tied_position = WorldPoint { x: 0.0, y: 0.0 };
    let mut tied = WorldState {
        ray_cursor: Default::default(),
        snakes: vec![
            snake(2, 0, 5, tied_position, 0.0),
            snake(1, 5, 5, tied_position, 0.0),
        ],
        body_points: [
            line_body(tied_position, 0.0, 5),
            line_body(tied_position, 0.0, 5),
        ]
        .concat(),
        pellets: vec![pellet(30, 1.0, 0.0, 1.0)],
        controller_leases: Vec::new(),
    };
    tied.snakes[1].population_slot = Some(0);
    tied.snakes[0].population_slot = Some(1);
    let tied_result = execute(&tied);
    assert_eq!(tied_result.1.len(), 1);
    assert_eq!((tied_result.1[0].0, tied_result.1[0].1), (30, 1));
}

#[test]
fn boost_shrink_food_eligibility_uses_the_pre_finalization_radius() {
    let movement_config = MovementConfig::typescript_defaults();
    let mut world = world_with_one(8);
    world.snakes[0].input_boost = true;
    world.snakes[0].radius = radius_for_length(8, movement_config);
    let retained_eat_radius = world.snakes[0].radius + 6.0;
    let finalized_eat_radius = radius_for_length(7, movement_config) + 6.0;
    let pellet_distance = (retained_eat_radius + finalized_eat_radius) * 0.5;
    world.pellets.push(pellet(
        20,
        0.982_628_430_314_376_3 + pellet_distance,
        -0.008_732_258_602_803_35,
        1.0,
    ));
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, movement_config, DT, 100, 100)
        .expect("boost movement should prepare");
    let staged_head = movement.snakes()[0].position;
    let pellet_position = world.pellets[0].position;
    let staged_distance =
        (pellet_position.x - staged_head.x).hypot(pellet_position.y - staged_head.y);
    assert!(staged_distance > finalized_eat_radius);
    assert!(staged_distance < retained_eat_radius);

    let mut food_workspace = FoodWorkspace::new();
    let prepared = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100,
            100,
        )
        .expect("pre-finalization radius should admit the pellet");

    assert_eq!(prepared.claims().len(), 1);
    assert_eq!(prepared.claims()[0].snake_id, 1);
    assert!(prepared.remaining_pellets().is_empty());
}

#[test]
fn body_capacity_failure_leaves_authority_untouched_and_staging_unready() {
    let mut world = world_with_one(5);
    world.snakes[0].target_length = 6.0;
    let authority_before = world.clone();
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 5, 100)
        .expect("movement should defer final growth");
    let mut food_workspace = FoodWorkspace::new();

    assert!(matches!(
        food_workspace.prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            5,
            100,
        ),
        Err(FoodError::BodyCapacityExceeded {
            required: 6,
            maximum: 5
        })
    ));
    assert!(!food_workspace.is_ready());
    assert_eq!(world, authority_before);
}

#[test]
fn consumed_food_frees_capacity_for_a_staged_boost_drop_atomically() {
    fn execute(pellet_x: f64) -> Result<(usize, usize), FoodError> {
        let mut world = world_with_one(8);
        world.snakes[0].input_boost = true;
        world.pellets.push(pellet(20, pellet_x, 0.0, 1.0));
        let indexed = index(&world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 1)
            .expect("one bounded boost request should stage");
        assert_eq!(movement.boost_drops().len(), 1);
        let mut food_workspace = FoodWorkspace::new();
        let prepared = food_workspace.prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            1,
        )?;
        Ok((
            prepared.remaining_pellets().len(),
            prepared.boost_drops().len(),
        ))
    }

    assert_eq!(execute(0.0), Ok((0, 1)));
    assert!(matches!(
        execute(1_000.0),
        Err(FoodError::PelletCapacityExceeded {
            required: 2,
            maximum: 1
        })
    ));
}

#[test]
fn wall_dead_snake_cannot_claim_food() {
    let mut world = world_with_one(5);
    world.snakes[0].position = WorldPoint { x: 3_490.2, y: 0.0 };
    world.snakes[0].previous_position = world.snakes[0].position;
    for (index, point) in world.body_points.iter_mut().enumerate() {
        point.x = 3_490.2 - index as f64 * 7.5;
    }
    world.pellets.push(pellet(20, 3_491.0, 0.0, 1.0));
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("wall movement should prepare");
    assert!(!movement.snakes()[0].alive);
    let mut food_workspace = FoodWorkspace::new();
    let prepared = food_workspace
        .prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        )
        .expect("dead-snake food phase should prepare");

    assert!(prepared.claims().is_empty());
    assert_eq!(prepared.remaining_pellets().len(), 1);
    assert_eq!(prepared.snakes()[0].food, 0.0);
}

#[test]
fn source_world_identity_is_required() {
    let first = world_with_one(5);
    let second = first.clone();
    let indexed = index(&second);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&first, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();

    assert!(matches!(
        food_workspace.prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        ),
        Err(FoodError::SourceWorldMismatch)
    ));
}

#[test]
fn finite_extreme_food_values_cannot_publish_non_finite_staging() {
    let mut world = world_with_one(5);
    world.pellets.push(pellet(20, 0.0, 0.0, f64::MAX));
    let authority_before = world.clone();
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();

    assert!(matches!(
        food_workspace.prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        ),
        Err(FoodError::NonFiniteDerived { snake_id: 1, .. })
    ));
    assert!(!food_workspace.is_ready());
    assert_eq!(world, authority_before);
}

#[test]
fn warmed_preparation_reuses_all_food_and_body_capacities() {
    let mut world = world_with_one(5);
    world.pellets.push(pellet(20, 0.0, 0.0, 1.0));
    let indexed = index(&world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, MovementConfig::typescript_defaults(), DT, 100, 100)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let first = food_workspace
        .prepare(
            &indexed,
            movement,
            MovementConfig::typescript_defaults(),
            FoodConfig::typescript_defaults(),
            100,
            100,
        )
        .expect("warm food pass should prepare")
        .diagnostics();

    for _ in 0..24 {
        let next = food_workspace
            .prepare(
                &indexed,
                movement,
                MovementConfig::typescript_defaults(),
                FoodConfig::typescript_defaults(),
                100,
                100,
            )
            .expect("reused food pass should prepare")
            .diagnostics();
        assert_eq!(next.snake_order_capacity, first.snake_order_capacity);
        assert_eq!(next.pellet_order_capacity, first.pellet_order_capacity);
        assert_eq!(next.winner_capacity, first.winner_capacity);
        assert_eq!(next.snake_capacity, first.snake_capacity);
        assert_eq!(next.final_length_capacity, first.final_length_capacity);
        assert_eq!(next.body_point_capacity, first.body_point_capacity);
        assert_eq!(
            next.movement_body_range_capacity,
            first.movement_body_range_capacity
        );
        assert_eq!(
            next.movement_body_point_capacity,
            first.movement_body_point_capacity
        );
        assert_eq!(
            next.movement_radius_capacity,
            first.movement_radius_capacity
        );
        assert_eq!(next.claim_capacity, first.claim_capacity);
        assert_eq!(
            next.remaining_pellet_capacity,
            first.remaining_pellet_capacity
        );
        assert_eq!(next.boost_drop_capacity, first.boost_drop_capacity);
        assert_eq!(
            next.movement_proposal_capacity,
            first.movement_proposal_capacity
        );
        assert_eq!(
            next.query_candidate_capacity,
            first.query_candidate_capacity
        );
    }
}
