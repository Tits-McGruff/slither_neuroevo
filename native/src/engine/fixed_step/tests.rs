use super::*;
use crate::engine::ambient::AMBIENT_PELLET_ALGORITHM_VERSION;
use crate::engine::baseline::{BaselineSlotRuntime, BASELINE_LIFECYCLE_VERSION};
use crate::engine::rng::StatefulRng;
use crate::engine::state::{
    BaselineStrategyState, BodyRange, ControllerKind, ControllerLeaseStatus,
    LatestControllerAction, PelletState, SnakeKind, SnakeState, WorldPoint, ALLOCATOR_VERSION,
    BASELINE_ENTITY_ID_START, RNG_BUNDLE_VERSION,
};

const DT: f64 = 1.0 / 60.0;

fn key(operation_epoch: u64) -> PhysicsStepKey {
    PhysicsStepKey::new(7, 3, 40, 2, 9, [0x5a; 32], operation_epoch)
}

fn snake(id: u64, kind: SnakeKind, body_start: usize) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).unwrap_or(7),
        kind,
        alive: true,
        population_slot: (kind == SnakeKind::Evolved).then_some(0),
        brain: None,
        baseline_slot: None,
        baseline_strategy: None,
        position: WorldPoint {
            x: (id % 10_000) as f64,
            y: 0.0,
        },
        previous_position: WorldPoint {
            x: (id % 10_000) as f64,
            y: 0.0,
        },
        direction: 0.0,
        radius: 9.0,
        speed: 165.0,
        boost: false,
        age_seconds: 1.0,
        food: 0.0,
        points: 5.0,
        kills: 0,
        target_length: 2.0,
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
            len: 2,
        },
        skin: 0,
    }
}

fn world() -> WorldState {
    let snake = snake(10, SnakeKind::Evolved, 0);
    WorldState {
        ray_cursor: Default::default(),
        body_points: vec![
            snake.position,
            WorldPoint {
                x: snake.position.x - 7.5,
                y: 0.0,
            },
        ],
        snakes: vec![snake],
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    }
}

fn rng_bundle(baseline_count: usize) -> RngStateBundle {
    RngStateBundle {
        baseline_behaviors: Vec::new(),
        evolved_respawns: Vec::new(),
        version: RNG_BUNDLE_VERSION,
        world: StatefulRng::new(101.0).export_state(),
        evolution: StatefulRng::new(202.0).export_state(),
        external_controller: StatefulRng::new(303.0).export_state(),
        baselines: (0..baseline_count)
            .map(|slot| BaselineRngState {
                slot: u32::try_from(slot).expect("test slot must fit"),
                state: StatefulRng::new(404.0 + slot as f64).export_state(),
            })
            .collect(),
    }
}

fn allocators() -> AllocatorState {
    AllocatorState {
        version: ALLOCATOR_VERSION,
        next_entity_id: 100,
        next_brain_id: 1,
        next_genome_id: 1,
        next_controller_lease_id: 1,
        next_frame_v1_id: 100,
        next_external_id: 1_000_000_000_000,
        next_baseline_id: BASELINE_ENTITY_ID_START + 100,
        next_resurrected_id: 3_000_000_000_000,
    }
}

fn lifecycle() -> BaselineLifecycleState {
    BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: Vec::new(),
    }
}

fn world_with_waiting_baseline() -> (WorldState, BaselineLifecycleState) {
    let mut source_world = world();
    let mut baseline = snake(BASELINE_ENTITY_ID_START + 10, SnakeKind::Baseline, 2);
    baseline.alive = false;
    baseline.population_slot = None;
    baseline.baseline_slot = Some(0);
    baseline.baseline_strategy = Some(BaselineStrategyState::Roam);
    source_world.body_points.extend([
        baseline.position,
        WorldPoint {
            x: baseline.position.x - 7.5,
            y: 0.0,
        },
    ]);
    source_world.snakes.push(baseline.clone());
    let lifecycle = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: baseline.id,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: Some(10.0),
        }],
    };
    (source_world, lifecycle)
}

fn config(slot_count: usize) -> FixedStepPrefixConfig {
    let mut ambient = AmbientPelletConfig::typescript_defaults();
    ambient.algorithm_version = AMBIENT_PELLET_ALGORITHM_VERSION;
    ambient.target_count = 1;
    ambient.spawn_per_second = 60.0;
    FixedStepPrefixConfig {
        fixed_dt: DT,
        ambient,
        baseline: BaselineLifecycleConfig {
            slot_count,
            ..BaselineLifecycleConfig::typescript_defaults()
        },
        maximum_snakes: 16,
        maximum_pellets: 16,
        ..FixedStepPrefixConfig::typescript_defaults()
    }
}

fn config_without_ambient(slot_count: usize) -> FixedStepPrefixConfig {
    let mut value = config(slot_count);
    value.ambient.target_count = 0;
    value.ambient.spawn_per_second = 0.0;
    value
}

#[test]
fn prefix_joins_accounting_ambient_and_timers_without_authority_write() {
    let (source_world, source_lifecycle) = world_with_waiting_baseline();
    let source_rng = rng_bundle(1);
    let source_allocators = allocators();
    let original_world = source_world.clone();
    let original_rng = source_rng.clone();
    let original_allocators = source_allocators.clone();
    let original_lifecycle = source_lifecycle.clone();
    let mut workspace = FixedStepPrefixWorkspace::new();

    let prepared = workspace
        .prepare(FixedStepPrefixInputs {
            key: key(1),
            world: &source_world,
            rng: &source_rng,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: config(1),
        })
        .expect("complete prefix must prepare");

    assert_eq!(source_world, original_world);
    assert_eq!(source_rng, original_rng);
    assert_eq!(source_allocators, original_allocators);
    assert_eq!(source_lifecycle, original_lifecycle);
    assert_eq!(prepared.key(), key(1));
    assert_eq!(prepared.world().snakes[0].age_seconds, 1.0 + DT);
    assert_eq!(
        prepared.world().snakes[0].points,
        5.0 + DT * StepAccountingConfig::typescript_defaults().points_per_second_alive
    );
    assert_eq!(prepared.world().pellets.len(), 1);
    assert_eq!(prepared.world().pellets[0].id, 100);
    assert_ne!(prepared.rng().world, source_rng.world);
    assert_eq!(prepared.rng().evolution, source_rng.evolution);
    assert_eq!(
        prepared.rng().external_controller,
        source_rng.external_controller
    );
    assert_eq!(prepared.rng().baselines, source_rng.baselines);
    assert_eq!(prepared.allocators().next_entity_id, 101);
    assert_eq!(prepared.generation_elapsed_seconds(), 4.0 + DT);
    assert_eq!(prepared.ambient_accumulator(), 0.0);
    assert_eq!(
        prepared.baseline_lifecycle().slots[0].respawn_remaining_seconds,
        Some(10.0 - DT)
    );
    assert!(workspace.is_ready());
}

#[test]
fn dense_v2_behavior_rng_copy_reuses_capacity_and_preserves_lifecycle_rng() {
    let mut source = rng_bundle(2);
    source.version = crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
    source.baseline_behaviors =
        crate::engine::rng::derive_baseline_v2_behaviors(42, 1, 9, 2, true).unwrap();
    let mut target = None;
    let mut scratch = RngCopyScratch::default();
    copy_rng_bundle_reusing(&mut target, &mut scratch, &source).unwrap();
    let pointer = target.as_ref().unwrap().baseline_behaviors.as_ptr();
    let capacity = target.as_ref().unwrap().baseline_behaviors.capacity();
    for _ in 0..100 {
        let mut behavior = source.baseline_behaviors[0].runtime().unwrap();
        behavior.next_f64();
        source.baseline_behaviors[0].uniform_state = behavior.uniform_state();
        copy_rng_bundle_reusing(&mut target, &mut scratch, &source).unwrap();
        assert_eq!(target.as_ref().unwrap(), &source);
        assert_eq!(
            target.as_ref().unwrap().baseline_behaviors.as_ptr(),
            pointer
        );
        assert_eq!(
            target.as_ref().unwrap().baseline_behaviors.capacity(),
            capacity
        );
    }
    let lifecycle = source.baselines.clone();
    source.baseline_behaviors.clear();
    source.version = 2;
    copy_rng_bundle_reusing(&mut target, &mut scratch, &source).unwrap();
    assert!(target.as_ref().unwrap().baseline_behaviors.is_empty());
    assert_eq!(target.as_ref().unwrap().baselines, lifecycle);
}

#[test]
fn due_baseline_respawn_completes_before_the_shared_control_boundary() {
    let (mut source_world, mut source_lifecycle) = world_with_waiting_baseline();
    source_lifecycle.slots[0].respawn_remaining_seconds = Some(DT * 0.5);
    let old_baseline_id = source_lifecycle.slots[0].snake_id;
    source_world.pellets.push(PelletState {
        id: 99,
        position: WorldPoint { x: 30.0, y: 40.0 },
        value: 1.0,
        kind: 1,
        color: 77,
        owner: Some(old_baseline_id),
    });
    let source_rng = rng_bundle(1);
    let source_allocators = allocators();
    let original_world = source_world.clone();
    let original_rng = source_rng.clone();
    let original_allocators = source_allocators.clone();
    let original_lifecycle = source_lifecycle.clone();
    let mut workspace = FixedStepPrefixWorkspace::new();

    let prepared = workspace
        .prepare(FixedStepPrefixInputs {
            key: key(2),
            world: &source_world,
            rng: &source_rng,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: config(1),
        })
        .expect("collision-safe due baseline replacement must complete");

    assert_eq!(source_world, original_world);
    assert_eq!(source_rng, original_rng);
    assert_eq!(source_allocators, original_allocators);
    assert_eq!(source_lifecycle, original_lifecycle);
    let baseline = prepared
        .world()
        .snakes
        .iter()
        .find(|snake| snake.baseline_slot == Some(0))
        .expect("stable baseline slot must survive replacement");
    assert_eq!(baseline.id, source_allocators.next_baseline_id);
    assert_eq!(baseline.frame_v1_id, source_allocators.next_frame_v1_id);
    assert_eq!(baseline.kind, SnakeKind::Baseline);
    assert!(baseline.alive);
    assert_eq!(baseline.baseline_slot, Some(0));
    assert_eq!(
        baseline.baseline_strategy,
        Some(BaselineStrategyState::Roam)
    );
    assert_eq!(baseline.body.len, config(1).baseline_spawn.snake_start_len);
    assert_eq!(baseline.age_seconds, DT);
    assert_eq!(
        baseline.points,
        DT * StepAccountingConfig::typescript_defaults().points_per_second_alive
    );
    assert_eq!(baseline.turn, 0.0);
    assert!(!baseline.input_boost);
    assert_eq!(prepared.world().body_points.len(), 2 + baseline.body.len);
    assert_eq!(prepared.world().pellets.len(), 1);
    assert_eq!(prepared.world().pellets[0].owner, None);
    assert_eq!(prepared.world().pellets[0].color, 77);
    assert_ne!(prepared.rng().baselines, source_rng.baselines);
    assert_eq!(
        prepared.allocators().next_baseline_id,
        source_allocators.next_baseline_id + 1
    );
    assert_eq!(
        prepared.allocators().next_frame_v1_id,
        source_allocators.next_frame_v1_id + 1
    );
    let lifecycle = &prepared.baseline_lifecycle().slots[0];
    assert_eq!(lifecycle.snake_id, baseline.id);
    assert_eq!(lifecycle.respawn_remaining_seconds, None);
    assert_eq!(lifecycle.wander_angle, 0.0);
    assert_eq!(lifecycle.turn, 0.0);
    assert!(!lifecycle.boost);
    assert_eq!(prepared.diagnostics().baseline_respawn.due_slots, 1);
    assert_eq!(prepared.diagnostics().baseline_respawn.completed_slots, 1);
    assert_eq!(
        prepared
            .diagnostics()
            .baseline_respawn
            .cleared_pellet_owners,
        1
    );
    assert!(workspace.is_ready());
}

#[test]
fn impossible_due_respawn_is_explicit_atomic_and_retryable_from_the_same_source() {
    let mut source_world = world();
    source_world.snakes[0] = snake(BASELINE_ENTITY_ID_START + 10, SnakeKind::Baseline, 0);
    source_world.snakes[0].alive = false;
    source_world.snakes[0].population_slot = None;
    source_world.snakes[0].baseline_slot = Some(0);
    source_world.snakes[0].baseline_strategy = Some(BaselineStrategyState::Roam);
    let source_rng = rng_bundle(1);
    let source_allocators = allocators();
    let source_lifecycle = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: source_world.snakes[0].id,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: Some(DT * 0.5),
        }],
    };
    let original_world = source_world.clone();
    let original_rng = source_rng.clone();
    let original_allocators = source_allocators.clone();
    let original_lifecycle = source_lifecycle.clone();
    let mut impossible = config_without_ambient(1);
    impossible.ambient.world_radius = 10.0;
    impossible.baseline_spawn.world_radius = 10.0;
    impossible.baseline_spawn.snake_radius = 9.0;
    impossible.baseline_spawn.random_attempts_per_request = 1;
    impossible.baseline_spawn.fallback_position_count = 1;
    impossible.baseline_spawn.fallback_heading_count = 1;
    impossible.baseline_spawn.maximum_candidates_per_request = 2;
    impossible.baseline_spawn.maximum_candidates_per_batch = 2;
    let mut workspace = FixedStepPrefixWorkspace::new();

    let error = workspace
        .prepare(FixedStepPrefixInputs {
            key: key(21),
            world: &source_world,
            rng: &source_rng,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: impossible,
        })
        .expect_err("the complete initial body cannot fit in this arena");
    assert!(matches!(
        error,
        FixedStepPrefixError::BaselineRespawn {
            slot: 0,
            error,
        } if matches!(*error, SpawnError::NoCollisionSafePlacement { .. })
    ));
    assert!(!workspace.is_ready());
    assert_eq!(source_world, original_world);
    assert_eq!(source_rng, original_rng);
    assert_eq!(source_allocators, original_allocators);
    assert_eq!(source_lifecycle, original_lifecycle);

    let prepared = workspace
        .prepare(FixedStepPrefixInputs {
            key: key(22),
            world: &source_world,
            rng: &source_rng,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: config_without_ambient(1),
        })
        .expect("a later explicit retry from unchanged authority is deterministic");
    assert_eq!(
        prepared.world().snakes[0].id,
        source_allocators.next_baseline_id
    );
    assert_eq!(
        prepared.world().snakes[0].frame_v1_id,
        source_allocators.next_frame_v1_id
    );
    assert!(workspace.is_ready());
}

#[test]
fn simultaneous_due_slots_use_stable_slot_order_and_independent_rng_streams() {
    let first = snake(BASELINE_ENTITY_ID_START + 10, SnakeKind::Baseline, 0);
    let second = snake(BASELINE_ENTITY_ID_START + 20, SnakeKind::Baseline, 2);
    let mut first = first;
    first.alive = false;
    first.population_slot = None;
    first.baseline_slot = Some(0);
    first.baseline_strategy = Some(BaselineStrategyState::Roam);
    first.frame_v1_id = 7;
    let mut second = second;
    second.alive = false;
    second.population_slot = None;
    second.baseline_slot = Some(1);
    second.baseline_strategy = Some(BaselineStrategyState::Roam);
    second.frame_v1_id = 8;
    let body_points = vec![
        first.position,
        WorldPoint {
            x: first.position.x - 7.5,
            y: 0.0,
        },
        second.position,
        WorldPoint {
            x: second.position.x - 7.5,
            y: 0.0,
        },
    ];
    let world_a = WorldState {
        ray_cursor: Default::default(),
        snakes: vec![first.clone(), second.clone()],
        body_points: body_points.clone(),
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    };
    let world_b = WorldState {
        ray_cursor: Default::default(),
        snakes: vec![second, first],
        body_points,
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    };
    let lifecycle = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 0,
                snake_id: BASELINE_ENTITY_ID_START + 10,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(DT * 0.5),
            },
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 1,
                snake_id: BASELINE_ENTITY_ID_START + 20,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(DT * 0.5),
            },
        ],
    };
    let rng = rng_bundle(2);
    let allocators = allocators();
    let config = config_without_ambient(2);
    let mut workspace_a = FixedStepPrefixWorkspace::new();
    let prepared_a = workspace_a
        .prepare(FixedStepPrefixInputs {
            key: key(31),
            world: &world_a,
            rng: &rng,
            allocators: &allocators,
            generation_elapsed_seconds: 0.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &lifecycle,
            config,
        })
        .unwrap();
    let by_slot_a: Vec<_> = (0..2)
        .map(|slot| {
            prepared_a
                .world()
                .snakes
                .iter()
                .find(|snake| snake.baseline_slot == Some(slot))
                .map(|snake| (snake.id, snake.frame_v1_id, snake.position, snake.direction))
                .unwrap()
        })
        .collect();
    let rng_a = prepared_a.rng().baselines.clone();
    let allocators_a = prepared_a.allocators().clone();
    let diagnostics_a = prepared_a.diagnostics().baseline_respawn;

    let mut workspace_b = FixedStepPrefixWorkspace::new();
    let prepared_b = workspace_b
        .prepare(FixedStepPrefixInputs {
            key: key(31),
            world: &world_b,
            rng: &rng,
            allocators: &allocators,
            generation_elapsed_seconds: 0.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &lifecycle,
            config,
        })
        .unwrap();
    let by_slot_b: Vec<_> = (0..2)
        .map(|slot| {
            prepared_b
                .world()
                .snakes
                .iter()
                .find(|snake| snake.baseline_slot == Some(slot))
                .map(|snake| (snake.id, snake.frame_v1_id, snake.position, snake.direction))
                .unwrap()
        })
        .collect();
    assert_eq!(by_slot_a, by_slot_b);
    assert_ne!(by_slot_a[0].2, by_slot_a[1].2);
    assert_eq!(rng_a, prepared_b.rng().baselines);
    assert_ne!(rng_a[0].state, rng_a[1].state);
    assert_eq!(allocators_a, *prepared_b.allocators());
    assert!(diagnostics_a.candidates_examined >= 2);
    assert_eq!(
        diagnostics_a.candidates_examined,
        prepared_b
            .diagnostics()
            .baseline_respawn
            .candidates_examined
    );
    assert_eq!(diagnostics_a.completed_slots, 2);

    let original_world = world_a.clone();
    let original_rng = rng.clone();
    let original_allocators = allocators.clone();
    let original_lifecycle = lifecycle.clone();
    let mut limited = config;
    limited.baseline_spawn.maximum_candidates_per_batch = 1;
    let mut limited_workspace = FixedStepPrefixWorkspace::new();
    let error = limited_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(32),
            world: &world_a,
            rng: &rng,
            allocators: &allocators,
            generation_elapsed_seconds: 0.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &lifecycle,
            config: limited,
        })
        .expect_err("the complete due-slot batch budget must be enforced");
    assert!(matches!(
        error,
        FixedStepPrefixError::BaselineRespawn {
            slot: 1,
            error,
        } if matches!(*error, SpawnError::WorkBudgetExceeded { .. })
    ));
    assert!(!limited_workspace.is_ready());
    assert_eq!(
        limited_workspace
            .diagnostics()
            .baseline_respawn
            .candidates_examined,
        1,
        "the second due slot must be rejected before it receives a fresh per-slot budget",
    );
    assert_eq!(world_a, original_world);
    assert_eq!(rng, original_rng);
    assert_eq!(allocators, original_allocators);
    assert_eq!(lifecycle, original_lifecycle);
}

#[test]
fn warmed_due_respawn_reuses_every_reported_prefix_capacity() {
    let (source_world, mut source_lifecycle) = world_with_waiting_baseline();
    source_lifecycle.slots[0].respawn_remaining_seconds = Some(DT * 0.5);
    let source_rng = rng_bundle(1);
    let source_allocators = allocators();
    let config = config_without_ambient(1);
    let mut workspace = FixedStepPrefixWorkspace::new();
    let mut expected = None;

    for pass in 0..26u64 {
        let diagnostics = workspace
            .prepare(FixedStepPrefixInputs {
                key: key(100 + pass),
                world: &source_world,
                rng: &source_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 0.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config,
            })
            .unwrap()
            .diagnostics();
        if pass == 1 {
            expected = Some(diagnostics);
        } else if pass > 1 {
            assert_eq!(diagnostics, expected.unwrap());
        }
        assert_eq!(diagnostics.baseline_respawn.completed_slots, 1);
        assert!(
            diagnostics.baseline_respawn.respawn_body_capacity
                >= config.baseline_spawn.snake_start_len
        );
        assert!(
            diagnostics.baseline_respawn.body_compaction_capacity
                >= diagnostics
                    .body_point_capacity
                    .min(source_world.body_points.len())
        );
    }
}

#[test]
fn complete_source_provenance_rejects_every_stale_join_input() {
    let source_world = world();
    let source_rng = rng_bundle(0);
    let source_allocators = allocators();
    let source_lifecycle = lifecycle();
    let prefix_config = config(0);
    let mut workspace = FixedStepPrefixWorkspace::new();
    let prepared = workspace
        .prepare(FixedStepPrefixInputs {
            key: key(3),
            world: &source_world,
            rng: &source_rng,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: prefix_config,
        })
        .unwrap();

    let other_world = source_world.clone();
    let other_rng = source_rng.clone();
    let other_allocators = source_allocators.clone();
    let other_lifecycle = source_lifecycle.clone();
    let mut changed_config = prefix_config;
    changed_config.maximum_pellets += 1;
    let cases = [
        prepared.validate_current(
            key(4),
            &source_world,
            &source_rng,
            &source_allocators,
            4.0,
            0.0,
            &source_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &other_world,
            &source_rng,
            &source_allocators,
            4.0,
            0.0,
            &source_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &source_world,
            &other_rng,
            &source_allocators,
            4.0,
            0.0,
            &source_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &source_world,
            &source_rng,
            &other_allocators,
            4.0,
            0.0,
            &source_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &source_world,
            &source_rng,
            &source_allocators,
            5.0,
            0.0,
            &source_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &source_world,
            &source_rng,
            &source_allocators,
            4.0,
            1.0,
            &source_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &source_world,
            &source_rng,
            &source_allocators,
            4.0,
            0.0,
            &other_lifecycle,
            prefix_config,
        ),
        prepared.validate_current(
            key(3),
            &source_world,
            &source_rng,
            &source_allocators,
            4.0,
            0.0,
            &source_lifecycle,
            changed_config,
        ),
    ];
    assert!(cases.iter().all(Result::is_err));
}

#[test]
fn warmed_prefix_reuses_every_reported_capacity_with_controller_text() {
    let mut source_world = world();
    let mut external = snake(1_000_000_000_010, SnakeKind::External, 2);
    external.population_slot = None;
    source_world.body_points.extend([
        external.position,
        WorldPoint {
            x: external.position.x - 7.5,
            y: 0.0,
        },
    ]);
    source_world.snakes.push(external.clone());
    source_world.controller_leases.push(ControllerLease {
        reward: None,
        identity_key: "player:legacy-owner".to_owned(),
        id: 1,
        snake_id: external.id,
        kind: ControllerKind::Player,
        connection_id: Some(8),
        scope: "run-with-retained-controller-text".to_owned(),
        resume_token: "0123456789abcdef0123456789abcdef".to_owned(),
        status: ControllerLeaseStatus::Connected,
        latest_action: LatestControllerAction {
            is_explicit: false,
            turn: 0.25,
            boost: false,
            client_tick: 1,
            arrival_sequence: 1,
            accepted_at_ms: 100,
        },
        last_observed_at_ms: 100,
        disconnected_at_ms: None,
        input_hold_expires_at_ms: None,
        grace_expires_at_ms: None,
        takeover_committed_at_ms: None,
    });
    let source_rng = rng_bundle(0);
    let source_allocators = allocators();
    let source_lifecycle = lifecycle();
    let mut prefix_config = config(0);
    prefix_config.ambient.target_count = 0;
    prefix_config.ambient.spawn_per_second = 0.0;
    let mut workspace = FixedStepPrefixWorkspace::new();

    workspace
        .prepare(FixedStepPrefixInputs {
            key: key(10),
            world: &source_world,
            rng: &source_rng,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: prefix_config,
        })
        .unwrap();
    let warmed = workspace.diagnostics();
    for operation_epoch in 11..35 {
        let prepared = workspace
            .prepare(FixedStepPrefixInputs {
                key: key(operation_epoch),
                world: &source_world,
                rng: &source_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: prefix_config,
            })
            .unwrap();
        assert_eq!(
            prepared.world().controller_leases,
            source_world.controller_leases
        );
        assert_eq!(workspace.diagnostics(), warmed);
    }
}

#[test]
fn gaussian_spare_toggles_reuse_retained_string_storage() {
    let (source_world, source_lifecycle) = world_with_waiting_baseline();
    let source_allocators = allocators();
    let mut with_spares = rng_bundle(1);
    let mut world_rng = StatefulRng::new(101.0);
    let mut evolution_rng = StatefulRng::new(202.0);
    let mut external_rng = StatefulRng::new(303.0);
    let mut baseline_rng = StatefulRng::new(404.0);
    let _ = world_rng.gaussian();
    let _ = evolution_rng.gaussian();
    let _ = external_rng.gaussian();
    let _ = baseline_rng.gaussian();
    with_spares.world = world_rng.export_state();
    with_spares.evolution = evolution_rng.export_state();
    with_spares.external_controller = external_rng.export_state();
    with_spares.baselines[0].state = baseline_rng.export_state();
    assert!(with_spares.world.gaussian_spare_hex.is_some());

    let mut without_spares = with_spares.clone();
    for state in [
        &mut without_spares.world,
        &mut without_spares.evolution,
        &mut without_spares.external_controller,
        &mut without_spares.baselines[0].state,
    ] {
        state.gaussian_spare_valid = false;
        state.gaussian_spare_hex = None;
    }

    let mut prefix_config = config(1);
    prefix_config.ambient.target_count = 0;
    prefix_config.ambient.spawn_per_second = 0.0;
    let mut workspace = FixedStepPrefixWorkspace::new();
    workspace
        .prepare(FixedStepPrefixInputs {
            key: key(40),
            world: &source_world,
            rng: &with_spares,
            allocators: &source_allocators,
            generation_elapsed_seconds: 4.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &source_lifecycle,
            config: prefix_config,
        })
        .unwrap();
    let warmed = workspace.diagnostics();
    assert!(warmed.rng_text_capacity > 0);

    for operation_epoch in 41..65 {
        let expected_rng = if operation_epoch % 2 == 0 {
            &with_spares
        } else {
            &without_spares
        };
        let prepared = workspace
            .prepare(FixedStepPrefixInputs {
                key: key(operation_epoch),
                world: &source_world,
                rng: expected_rng,
                allocators: &source_allocators,
                generation_elapsed_seconds: 4.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &source_lifecycle,
                config: prefix_config,
            })
            .unwrap();
        assert_eq!(prepared.rng(), expected_rng);
        assert_eq!(workspace.diagnostics(), warmed);
    }
}
