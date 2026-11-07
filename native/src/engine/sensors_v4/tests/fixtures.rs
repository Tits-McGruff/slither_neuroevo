use super::*;

pub(super) fn point(x: f64, y: f64) -> WorldPoint {
    WorldPoint { x, y }
}
pub(super) fn add_snake(
    world: &mut WorldState,
    id: u64,
    kind: SnakeKind,
    position: WorldPoint,
    heading: f64,
    length: usize,
) {
    let start = world.body_points.len();
    world.body_points.extend((0..length).map(|i| {
        point(
            position.x - heading.cos() * i as f64 * 7.5,
            position.y - heading.sin() * i as f64 * 7.5,
        )
    }));
    world.snakes.push(SnakeState {
        id,
        frame_v1_id: id as u32,
        kind,
        alive: true,
        population_slot: (kind == SnakeKind::Evolved).then_some(0),
        brain: None,
        baseline_slot: None,
        baseline_strategy: None,
        position,
        previous_position: position,
        direction: heading,
        radius: 9.0,
        speed: 165.0,
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
        control_accumulator_seconds: 0.06,
        delivered_observation_points: 0.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body: BodyRange { start, len: length },
        skin: 0,
    });
}
pub(super) fn evaluator(profile: SensorProfile, bins: usize) -> SensorV4Evaluator {
    SensorV4Evaluator::new(
        profile,
        SensorConfig {
            bins,
            ..SensorConfig::default()
        },
        MovementConfig::typescript_defaults(),
        1.0 / 60.0,
    )
    .unwrap()
}
pub(super) fn sample(
    evaluator: &SensorV4Evaluator,
    world: &WorldState,
    lives: &EvolvedGenerationState,
    policy: EvolvedExperimentConfig,
    grant: u32,
) -> (Vec<f32>, V4SensorSample) {
    let indexed = IndexedSensorWorld::build(
        world,
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
    let mut output = vec![0.0; evaluator.layout().input_size()];
    let result = evaluator
        .sample(
            &indexed,
            view,
            V4ObservationContext {
                generation_elapsed_seconds: 120.0,
                evolved: lives,
                experiment: policy,
            },
            0,
            &mut output,
            &mut RayWorkMeter::new(grant),
            &mut V4SensorScratch::default(),
        )
        .unwrap();
    (output, result)
}

pub(super) fn simple(profile: SensorProfile, world: &WorldState) -> (Vec<f32>, V4SensorSample) {
    sample(
        &evaluator(profile, 16),
        world,
        &EvolvedGenerationState::default(),
        EvolvedExperimentConfig::legacy(240.0),
        1024,
    )
}
pub(super) fn close(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 1e-6,
        "{actual} != {expected}"
    );
}

pub(super) fn baseline_v2_action(
    profile: SensorProfile,
    world: &WorldState,
    grant: u32,
) -> (Vec<f32>, crate::engine::baseline_v2::BaselineV2Action) {
    use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Policy, BaselineV2State};
    let sensors = evaluator(profile, 16);
    let (values, _) = sample(
        &sensors,
        world,
        &EvolvedGenerationState::default(),
        EvolvedExperimentConfig::legacy(240.0),
        grant,
    );
    let layout = crate::engine::sensor_layout::SensorLayout::for_profile(profile, 16).unwrap();
    let policy = BaselineV2Policy::new(
        &layout,
        SensorConfig::default(),
        BaselineV2Mode::Deterministic,
    )
    .unwrap();
    let action = policy
        .evaluate(&values, BaselineV2State::default(), 12345, 1.0 / 60.0)
        .unwrap();
    (values, action)
}
