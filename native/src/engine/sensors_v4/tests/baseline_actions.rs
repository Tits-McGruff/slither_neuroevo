use super::*;

#[test]
fn baseline_v2_actual_crossing_and_wall_samples_avoid_known_routes_without_probes() {
    let mut crossing = WorldState::default();
    add_snake(
        &mut crossing,
        1,
        SnakeKind::External,
        point(0.0, 0.0),
        0.0,
        5,
    );
    crossing.snakes[0].speed = 100.0;
    add_snake(
        &mut crossing,
        2,
        SnakeKind::Baseline,
        point(40.0, 40.0),
        -PI / 2.0,
        5,
    );
    crossing.snakes[1].speed = 100.0;
    let before = crossing.clone();
    let (values, action) = baseline_v2_action(SensorProfile::Motion, &crossing, 1024);
    assert!(action.urgent_sectors != 0);
    assert_eq!(action.urgent_sectors & (1 << action.sector), 0);
    assert!(!action.unsafe_route);
    assert_eq!(crossing, before);
    assert_eq!(values[21], 1.0);
    let mut wall = WorldState::default();
    add_snake(
        &mut wall,
        1,
        SnakeKind::External,
        point(3460.0, 0.0),
        0.0,
        50,
    );
    let (_, escape) = baseline_v2_action(SensorProfile::Motion, &wall, 1024);
    assert_ne!(escape.sector, 8);
    assert!(escape.turn.abs() > 0.5 && !escape.boost);
    assert_eq!(escape.uniform_draws, 0);
}

#[test]
fn baseline_v2_actual_head_on_differs_from_receding_without_privileged_target_identity() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 50);
    world.snakes[0].speed = 100.0;
    world.snakes[0].points = 100.0;
    add_snake(&mut world, 2, SnakeKind::Baseline, point(70.0, 0.0), PI, 10);
    world.snakes[1].speed = 100.0;
    let (approaching, action) = baseline_v2_action(SensorProfile::Motion, &world, 1024);
    assert!(action.urgent_sectors != 0);
    assert_ne!(action.sector, 8);
    world.snakes[1].direction = 0.0;
    let (receding, other) = baseline_v2_action(SensorProfile::Motion, &world, 1024);
    assert_eq!(other.urgent_sectors, 0);
    assert!(approaching[24 + 13 * 16 + 8] > receding[24 + 13 * 16 + 8]);
    world.snakes[1].id = 123456;
    world.snakes[1].frame_v1_id = 123456;
    let (renamed, renamed_action) = baseline_v2_action(SensorProfile::Motion, &world, 1024);
    assert_eq!(receding, renamed);
    assert_eq!(other, renamed_action);
}

#[test]
fn baseline_v2_actual_smaller_pursuit_and_larger_threat_use_visible_size_and_economy() {
    let mut pursuit = WorldState::default();
    add_snake(
        &mut pursuit,
        1,
        SnakeKind::External,
        point(0.0, 0.0),
        0.0,
        50,
    );
    pursuit.snakes[0].points = 100.0;
    add_snake(
        &mut pursuit,
        2,
        SnakeKind::Baseline,
        point(550.0, 0.0),
        0.0,
        5,
    );
    pursuit.snakes[1].speed = 100.0;
    let (_, attack) = baseline_v2_action(SensorProfile::Motion, &pursuit, 1024);
    assert_eq!(attack.sector, 8);
    assert!(attack.boost);
    let mut threat = WorldState::default();
    add_snake(
        &mut threat,
        1,
        SnakeKind::External,
        point(0.0, 0.0),
        0.0,
        50,
    );
    threat.snakes[0].points = 100.0;
    add_snake(
        &mut threat,
        2,
        SnakeKind::Baseline,
        point(550.0, 0.0),
        PI,
        100,
    );
    threat.snakes[1].speed = 100.0;
    let (_, escape) = baseline_v2_action(SensorProfile::Motion, &threat, 1024);
    assert_ne!(escape.sector, 8);
    assert!(!escape.boost && !escape.unsafe_route);
}

#[test]
fn baseline_v2_actual_saturated_capsules_disable_boost_and_remain_finite_for_both_profiles() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 50);
    world.snakes[0].points = 100.0;
    add_snake(
        &mut world,
        2,
        SnakeKind::Baseline,
        point(400.0, 0.0),
        0.0,
        5,
    );
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        let (values, action) = baseline_v2_action(profile, &world, 0);
        assert_eq!(values[23], 1.0);
        assert!(!action.boost);
        assert!(action.turn.is_finite() && action.normalized_score.is_finite());
    }
}
