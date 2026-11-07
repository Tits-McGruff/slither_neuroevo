use super::*;
use crate::engine::state::{BaselineStrategyState, BodyRange, SnakeState, WorldPoint};

fn baseline(slot: u32, id: u64, alive: bool) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: id as u32,
        kind: SnakeKind::Baseline,
        alive,
        population_slot: None,
        brain: None,
        baseline_slot: Some(slot),
        baseline_strategy: Some(BaselineStrategyState::Roam),
        position: WorldPoint {
            x: id as f64,
            y: 0.0,
        },
        previous_position: WorldPoint {
            x: id as f64,
            y: 0.0,
        },
        direction: 0.0,
        radius: 9.0,
        speed: 0.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: 1.0,
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
        body: BodyRange { start: 0, len: 1 },
        skin: 2,
    }
}

fn world(entries: &[(u32, u64, bool)]) -> WorldState {
    let mut world = WorldState::default();
    for &(slot, id, alive) in entries {
        let mut snake = baseline(slot, id, alive);
        snake.body = BodyRange {
            start: world.body_points.len(),
            len: 1,
        };
        world.body_points.push(snake.position);
        world.snakes.push(snake);
    }
    world
}

#[test]
fn v2_respawn_resets_physical_policy_and_consumed_step_without_touching_v1() {
    use crate::engine::baseline_v2::BaselineV2State;
    let mut slot = BaselineSlotRuntime {
        slot: 0,
        snake_id: 20,
        strategy_timer_seconds: 0.0,
        wander_angle: 0.0,
        wander_timer_seconds: 0.0,
        turn: 0.5,
        boost: true,
        respawn_remaining_seconds: Some(1.0),
        v2_policy: Some(BaselineV2State {
            committed_heading: Some(0.4),
            commitment_remaining_seconds: 0.1,
            wander_angle: 0.2,
            wander_remaining_seconds: 1.0,
            strategy: BaselineStrategyState::Boost,
            ..BaselineV2State::default()
        }),
        v2_last_control_step: Some(77),
    };
    slot.reset_after_respawn(21);
    assert_eq!(slot.snake_id, 21);
    assert_eq!(slot.v2_policy, Some(BaselineV2State::default()));
    assert_eq!(slot.v2_last_control_step, None);
    assert_eq!(slot.turn, 0.0);
    assert!(!slot.boost);
    let mut v1 = BaselineSlotRuntime {
        v2_policy: None,
        ..slot
    };
    v1.reset_after_respawn(22);
    assert_eq!(v1.v2_policy, None);
    assert_eq!(v1.v2_last_control_step, None);
}

fn config(count: usize) -> BaselineLifecycleConfig {
    BaselineLifecycleConfig {
        slot_count: count,
        ..BaselineLifecycleConfig::typescript_defaults()
    }
}

fn key() -> PhysicsStepKey {
    PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 7)
}

fn reset_key() -> PhysicsStepKey {
    PhysicsStepKey::new(2, 2, 3, 4, 5, [6; 32], 8)
}

#[test]
fn generation_initialization_uses_dense_slots_independent_of_world_order() {
    let mut world = world(&[(2, 202, true), (0, 200, true), (1, 201, true)]);
    world.snakes[0].baseline_strategy = Some(BaselineStrategyState::Seek);
    world.snakes[0].turn = -0.25;
    world.snakes[0].input_boost = true;
    let state = BaselineLifecycleState::initialize_after_complete_spawn(config(3), &world).unwrap();
    assert_eq!(
        state
            .slots
            .iter()
            .map(|slot| slot.snake_id)
            .collect::<Vec<_>>(),
        vec![200, 201, 202]
    );
    assert_eq!(
        world.snakes[0].baseline_strategy,
        Some(BaselineStrategyState::Seek)
    );
    assert_eq!(state.slots[2].turn, -0.25);
    assert!(state.slots[2].boost);
    assert!(state
        .slots
        .iter()
        .all(|slot| slot.respawn_remaining_seconds.is_none()));
}

#[test]
fn generation_initialization_rejects_missing_or_out_of_range_slots() {
    let missing = world(&[(0, 200, true)]);
    assert_eq!(
        BaselineLifecycleState::initialize_after_complete_spawn(config(2), &missing),
        Err(BaselineLifecycleError::MissingWorldSlot(1))
    );

    let extra = world(&[(0, 200, true), (1, 201, true)]);
    assert_eq!(
        BaselineLifecycleState::initialize_after_complete_spawn(config(1), &extra),
        Err(BaselineLifecycleError::UnknownSlot(1))
    );

    let invalid_id = world(&[(0, 0, true)]);
    assert_eq!(
        BaselineLifecycleState::initialize_after_complete_spawn(config(1), &invalid_id),
        Err(BaselineLifecycleError::InvalidSnakeIdentity(0))
    );

    let duplicate_id = world(&[(0, 200, true), (1, 200, true)]);
    assert_eq!(
        BaselineLifecycleState::initialize_after_complete_spawn(config(2), &duplicate_id),
        Err(BaselineLifecycleError::DuplicateSnakeIdentity(200))
    );

    let mut missing_strategy = world(&[(0, 200, true)]);
    missing_strategy.snakes[0].baseline_strategy = None;
    assert_eq!(
        BaselineLifecycleState::initialize_after_complete_spawn(config(1), &missing_strategy,),
        Err(BaselineLifecycleError::MissingWorldStrategy {
            slot: 0,
            snake_id: 200,
        })
    );

    let mut timed_strategy_without_timer = world(&[(0, 200, true)]);
    timed_strategy_without_timer.snakes[0].baseline_strategy = Some(BaselineStrategyState::Avoid);
    assert_eq!(
        BaselineLifecycleState::initialize_after_complete_spawn(
            config(1),
            &timed_strategy_without_timer,
        ),
        Err(BaselineLifecycleError::InvalidSlotScalar {
            slot: 0,
            field: "strategy_timer_seconds",
        })
    );
}

#[test]
fn committed_death_starts_full_delay_and_duplicate_does_not_restart_it() {
    let physics_world = world(&[(0, 200, false)]);
    let mut state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 200,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.75,
            boost: true,
            respawn_remaining_seconds: None,
        }],
    };
    let event = [BaselineDeathEvent {
        slot: 0,
        snake_id: 200,
    }];
    let proof = PreparedPhysicsBaselineDeaths::test_fixture(key(), &physics_world, &event);
    let mut workspace = BaselineLifecycleWorkspace::new();
    let prepared = workspace
        .prepare_committed_deaths(proof, key(), &state, config(1))
        .unwrap();
    assert_eq!(prepared.key(), key());
    assert!(std::ptr::eq(prepared.physics_world(), &physics_world));
    assert_eq!(prepared.event_count(), 1);
    let mut target = state.clone();
    prepared
        .apply_to_working_copy(key(), &state, config(1), &mut target)
        .unwrap();
    let after_first = target.clone();
    assert_eq!(
        prepared.apply_to_working_copy(key(), &state, config(1), &mut target),
        Err(BaselineLifecycleError::WorkingCopyChanged)
    );
    assert_eq!(target, after_first);
    state = target;
    assert_eq!(state.slots[0].respawn_remaining_seconds, Some(20.0));
    assert_eq!(state.slots[0].turn, 0.0);
    assert!(!state.slots[0].boost);

    state.slots[0].respawn_remaining_seconds = Some(5.0);
    let repeated_key = PhysicsStepKey::new(1, 2, 4, 4, 5, [6; 32], 8);
    let repeated_proof =
        PreparedPhysicsBaselineDeaths::test_fixture(repeated_key, &physics_world, &event);
    let prepared = workspace
        .prepare_committed_deaths(repeated_proof, repeated_key, &state, config(1))
        .unwrap();
    let mut target = state.clone();
    prepared
        .apply_to_working_copy(repeated_key, &state, config(1), &mut target)
        .unwrap();
    state = target;
    assert_eq!(state.slots[0].respawn_remaining_seconds, Some(5.0));
}

#[test]
fn invalid_later_death_event_rejects_before_any_timer_write() {
    let initial_world = world(&[(0, 200, true), (1, 201, true)]);
    let physics_world = world(&[(0, 200, false), (1, 201, true)]);
    let state =
        BaselineLifecycleState::initialize_after_complete_spawn(config(2), &initial_world).unwrap();
    let before = state.clone();
    let events = [
        BaselineDeathEvent {
            slot: 0,
            snake_id: 200,
        },
        BaselineDeathEvent {
            slot: 1,
            snake_id: 201,
        },
    ];
    let proof = PreparedPhysicsBaselineDeaths::test_fixture(key(), &physics_world, &events);
    let mut workspace = BaselineLifecycleWorkspace::new();
    assert!(matches!(
        workspace.prepare_committed_deaths(proof, key(), &state, config(2)),
        Err(BaselineLifecycleError::InvalidDeathEvent {
            slot: 1,
            snake_id: 201,
        })
    ));
    assert!(!workspace.is_ready());
    assert_eq!(state, before);
}

#[test]
fn death_proposal_rejects_a_stale_world_epoch_with_reused_slot_and_snake_id() {
    let physics_world = world(&[(0, 200, false)]);
    let state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 200,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.25,
            boost: true,
            respawn_remaining_seconds: None,
        }],
    };
    let events = [BaselineDeathEvent {
        slot: 0,
        snake_id: 200,
    }];
    let proof = PreparedPhysicsBaselineDeaths::test_fixture(key(), &physics_world, &events);
    let mut workspace = BaselineLifecycleWorkspace::new();
    assert!(matches!(
        workspace.prepare_committed_deaths(proof, reset_key(), &state, config(1)),
        Err(BaselineLifecycleError::StepKeyMismatch {
            field: PhysicsStepKeyField::WorldEpoch,
        })
    ));
    assert!(!workspace.is_ready());

    let prepared = workspace
        .prepare_committed_deaths(proof, key(), &state, config(1))
        .unwrap();
    let mut target = state.clone();
    assert_eq!(
        prepared.apply_to_working_copy(reset_key(), &state, config(1), &mut target),
        Err(BaselineLifecycleError::StepKeyMismatch {
            field: PhysicsStepKeyField::WorldEpoch,
        })
    );
    assert_eq!(target, state);
}

#[test]
fn notified_and_missed_deaths_have_the_current_distinct_first_tick_timing() {
    let world = world(&[(0, 200, false), (1, 201, false)]);
    let state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(20.0),
            },
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 1,
                snake_id: 201,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.5,
                boost: true,
                respawn_remaining_seconds: None,
            },
        ],
    };
    let mut workspace = BaselineLifecycleWorkspace::new();
    let prepared = workspace
        .prepare_timers(key(), &world, &state, 1.0, config(2))
        .unwrap();
    assert_eq!(
        prepared.next_slots()[0].respawn_remaining_seconds,
        Some(19.0)
    );
    assert_eq!(
        prepared.next_slots()[1].respawn_remaining_seconds,
        Some(20.0)
    );
    assert_eq!(prepared.next_slots()[1].turn, 0.0);
    assert!(!prepared.next_slots()[1].boost);
    assert!(prepared.due_slots().is_empty());
}

#[test]
fn timer_staging_is_independent_of_world_container_order() {
    let forward_world = world(&[(0, 200, false), (1, 201, false)]);
    let reverse_world = world(&[(1, 201, false), (0, 200, false)]);
    let state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(1.0),
            },
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 1,
                snake_id: 201,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.5,
                boost: true,
                respawn_remaining_seconds: None,
            },
        ],
    };
    let mut forward_workspace = BaselineLifecycleWorkspace::new();
    let forward = forward_workspace
        .prepare_timers(key(), &forward_world, &state, 1.0, config(2))
        .unwrap();
    let forward_slots = forward.next_slots().to_vec();
    let forward_due = forward.due_slots().to_vec();
    let mut reverse_workspace = BaselineLifecycleWorkspace::new();
    let reverse = reverse_workspace
        .prepare_timers(key(), &reverse_world, &state, 1.0, config(2))
        .unwrap();
    assert_eq!(reverse.next_slots(), forward_slots);
    assert_eq!(reverse.due_slots(), forward_due);
    assert_eq!(reverse.due_slots(), &[0]);
}

#[test]
fn lowering_delay_caps_then_subtracts_while_raising_never_extends() {
    let world = world(&[(0, 200, false)]);
    let mut state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 200,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: Some(10.0),
        }],
    };
    let mut lowered = config(1);
    lowered.respawn_delay_seconds = 5.0;
    let mut workspace = BaselineLifecycleWorkspace::new();
    let first = workspace
        .prepare_timers(key(), &world, &state, 1.0, lowered)
        .unwrap();
    assert_eq!(first.next_slots()[0].respawn_remaining_seconds, Some(4.0));
    state.slots[0] = first.next_slots()[0];
    let mut raised = config(1);
    raised.respawn_delay_seconds = 30.0;
    let second = workspace
        .prepare_timers(key(), &world, &state, 1.0, raised)
        .unwrap();
    assert_eq!(second.next_slots()[0].respawn_remaining_seconds, Some(3.0));
}

#[test]
fn exact_deadline_requires_explicit_respawn_resolution() {
    let world = world(&[(0, 200, false)]);
    let state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 200,
            strategy_timer_seconds: 0.1,
            wander_angle: 0.2,
            wander_timer_seconds: 0.3,
            turn: 0.4,
            boost: true,
            respawn_remaining_seconds: Some(1.0),
        }],
    };
    let mut workspace = BaselineLifecycleWorkspace::new();
    let prepared = workspace
        .prepare_timers(key(), &world, &state, 1.0, config(1))
        .unwrap();
    assert_eq!(prepared.due_slots(), &[0]);
    assert_eq!(
        prepared.next_slots()[0].respawn_remaining_seconds,
        Some(0.0)
    );
    assert!(prepared.requires_respawn_resolution());
    let mut target = state.clone();
    assert_eq!(
        prepared.apply_without_due_respawns(key(), &world, &state, 1.0, config(1), &mut target),
        Err(BaselineLifecycleError::RespawnsUnresolved { count: 1 })
    );
    assert_eq!(target, state);
}

#[test]
fn no_due_timer_result_applies_once_after_complete_source_validation() {
    let world = world(&[(0, 200, false)]);
    let state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 200,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: Some(10.0),
        }],
    };
    let mut workspace = BaselineLifecycleWorkspace::new();
    let prepared = workspace
        .prepare_timers(key(), &world, &state, 1.0, config(1))
        .unwrap();
    let mut target = state.clone();
    prepared
        .apply_without_due_respawns(key(), &world, &state, 1.0, config(1), &mut target)
        .unwrap();
    assert_eq!(target.slots[0].respawn_remaining_seconds, Some(9.0));
    let after_first = target.clone();
    assert_eq!(
        prepared.apply_without_due_respawns(key(), &world, &state, 1.0, config(1), &mut target),
        Err(BaselineLifecycleError::WorkingCopyChanged)
    );
    assert_eq!(target, after_first);

    let other_world = world.clone();
    let mut fresh_target = state.clone();
    assert_eq!(
        prepared.apply_without_due_respawns(
            key(),
            &other_world,
            &state,
            1.0,
            config(1),
            &mut fresh_target
        ),
        Err(BaselineLifecycleError::SourceChanged { field: "world" })
    );
    assert_eq!(fresh_target, state);
}

#[test]
fn prepared_timer_result_rejects_every_stale_provenance_input_without_writes() {
    let source_world = world(&[(0, 200, false)]);
    let source_state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 200,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: Some(10.0),
        }],
    };
    let mut workspace = BaselineLifecycleWorkspace::new();
    let prepared = workspace
        .prepare_timers(key(), &source_world, &source_state, 1.0, config(1))
        .unwrap();

    let stale_state = source_state.clone();
    let mut target = source_state.clone();
    assert_eq!(
        prepared.apply_without_due_respawns(
            key(),
            &source_world,
            &stale_state,
            1.0,
            config(1),
            &mut target,
        ),
        Err(BaselineLifecycleError::SourceChanged {
            field: "baseline lifecycle",
        })
    );
    assert_eq!(target, source_state);

    let stale_key = PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 8);
    assert_eq!(
        prepared.apply_without_due_respawns(
            stale_key,
            &source_world,
            &source_state,
            1.0,
            config(1),
            &mut target,
        ),
        Err(BaselineLifecycleError::StepKeyMismatch {
            field: PhysicsStepKeyField::OperationEpoch,
        })
    );
    assert_eq!(target, source_state);

    assert_eq!(
        prepared.apply_without_due_respawns(
            key(),
            &source_world,
            &source_state,
            0.5,
            config(1),
            &mut target,
        ),
        Err(BaselineLifecycleError::SourceChanged {
            field: "fixed delta",
        })
    );
    assert_eq!(target, source_state);

    let mut stale_config = config(1);
    stale_config.respawn_delay_seconds = 19.0;
    assert_eq!(
        prepared.apply_without_due_respawns(
            key(),
            &source_world,
            &source_state,
            1.0,
            stale_config,
            &mut target,
        ),
        Err(BaselineLifecycleError::SourceChanged { field: "config" })
    );
    assert_eq!(target, source_state);
}

#[test]
fn failed_preparation_is_unready_and_a_valid_retry_clears_partial_staging() {
    let source_world = world(&[(0, 200, false), (1, 201, true)]);
    let mut invalid_state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(10.0),
            },
            BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot: 1,
                snake_id: 201,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(10.0),
            },
        ],
    };
    let mut workspace = BaselineLifecycleWorkspace::new();
    assert!(matches!(
        workspace.prepare_timers(key(), &source_world, &invalid_state, 1.0, config(2)),
        Err(BaselineLifecycleError::LiveSnakeHasRespawnTimer {
            slot: 1,
            snake_id: 201,
        })
    ));
    assert!(!workspace.is_ready());
    assert_eq!(workspace.diagnostics().slots, 1);

    invalid_state.slots[1].respawn_remaining_seconds = None;
    let prepared = workspace
        .prepare_timers(key(), &source_world, &invalid_state, 1.0, config(2))
        .unwrap();
    assert_eq!(prepared.next_slots().len(), 2);
    assert!(prepared.due_slots().is_empty());
    assert_eq!(
        prepared.next_slots()[0].respawn_remaining_seconds,
        Some(9.0)
    );
    assert_eq!(prepared.next_slots()[1].respawn_remaining_seconds, None);
}

#[test]
fn warm_timer_preparation_reuses_every_owned_vector() {
    let world = world(
        &(0..64)
            .map(|slot| (slot, 200 + u64::from(slot), slot % 2 == 0))
            .collect::<Vec<_>>(),
    );
    let state = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: (0..64)
            .map(|slot| BaselineSlotRuntime {
                v2_policy: None,
                v2_last_control_step: None,
                slot,
                snake_id: 200 + u64::from(slot),
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: (slot % 2 != 0).then_some(10.0),
            })
            .collect(),
    };
    let mut workspace = BaselineLifecycleWorkspace::new();
    let first = workspace
        .prepare_timers(key(), &world, &state, 1.0 / 60.0, config(64))
        .unwrap()
        .diagnostics();
    for _ in 0..24 {
        let next = workspace
            .prepare_timers(key(), &world, &state, 1.0 / 60.0, config(64))
            .unwrap()
            .diagnostics();
        assert_eq!(next, first);
        assert!(next.next_slot_capacity >= 64);
        assert!(next.due_slot_capacity >= 64);
        assert!(next.world_slot_index_capacity >= 64);
    }
}
