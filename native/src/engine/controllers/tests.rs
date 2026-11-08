use super::*;
use crate::engine::state::{
    BaselineStrategyState, BodyRange, BrainHandle, ControllerKind, WorldPoint,
};

const TIMING: ControllerTiming = ControllerTiming {
    input_hold_ms: 500,
    disconnect_grace_ms: 30_000,
};

fn snake() -> SnakeState {
    SnakeState {
        id: 9,
        frame_v1_id: 9,
        kind: SnakeKind::External,
        alive: true,
        population_slot: None,
        brain: Some(BrainHandle { id: 99, epoch: 4 }),
        baseline_slot: None,
        baseline_strategy: None::<BaselineStrategyState>,
        position: WorldPoint { x: 0.0, y: 0.0 },
        previous_position: WorldPoint { x: 0.0, y: 0.0 },
        direction: 0.0,
        radius: 9.0,
        speed: 165.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: 5.0,
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
        body: BodyRange { start: 0, len: 5 },
        skin: 0,
    }
}

fn lease() -> ControllerLease {
    ControllerLease {
        reward: None,
        identity_key: String::new(),
        id: 7,
        snake_id: 9,
        kind: ControllerKind::Player,
        connection_id: Some(11),
        scope: "run".into(),
        resume_token: "token".into(),
        status: ControllerLeaseStatus::Connected,
        latest_action: LatestControllerAction {
            is_explicit: false,
            turn: 0.25,
            boost: true,
            client_tick: 1,
            arrival_sequence: 1,
            accepted_at_ms: 1_000,
        },
        last_observed_at_ms: 1_000,
        disconnected_at_ms: None,
        input_hold_expires_at_ms: None,
        grace_expires_at_ms: None,
        takeover_committed_at_ms: None,
    }
}

fn action(
    turn: f32,
    boost: bool,
    client_tick: u64,
    arrival_sequence: u64,
    accepted_at_ms: u64,
) -> LatestActionInput {
    LatestActionInput {
        client_action_id: None,
        lease_id: 7,
        connection_id: 11,
        turn,
        boost,
        client_tick,
        arrival_sequence,
        accepted_at_ms,
    }
}

fn reclaim_input(boundary_at_ms: u64) -> ReclaimInput<'static> {
    ReclaimInput {
        kind: ControllerKind::Player,
        scope: "run",
        resume_token: "token",
        next_resume_token: "A".repeat(32),
        connection_id: 12,
        arrival_sequence: 2,
        received_at_ms: boundary_at_ms,
        boundary_at_ms,
    }
}

#[test]
fn legacy_reclaim_requires_one_live_reserved_name_in_the_same_scope_and_kind() {
    let mut lease = lease();
    lease.identity_key = "player:owner".into();
    let mut snake = snake();
    let close = prepare_disconnect(&lease, &snake, 11, 1_200, TIMING).unwrap();
    commit_disconnect(&mut lease, &mut snake, close).unwrap();
    let mut world = super::super::state::WorldState {
        snakes: vec![snake],
        controller_leases: vec![lease],
        ..Default::default()
    };
    let select = |world: &super::super::state::WorldState, scope: &str, kind, at| {
        select_legacy_reclaim(world, kind, scope, "player:owner", at, TIMING)
            .map(|lease| lease.map(|lease| lease.id))
    };
    assert_eq!(
        select(&world, "run", ControllerKind::Player, 1_300),
        Ok(Some(7))
    );
    assert_eq!(
        select(&world, "other-run", ControllerKind::Player, 1_300),
        Ok(None)
    );
    assert_eq!(
        select(&world, "run", ControllerKind::ReinforcementLearning, 1_300),
        Ok(None)
    );
    assert_eq!(
        select(&world, "run", ControllerKind::Player, 31_200),
        Ok(None)
    );
    let mut duplicate = world.controller_leases[0].clone();
    duplicate.id = 8;
    duplicate.snake_id = 10;
    let mut second_snake = world.snakes[0].clone();
    second_snake.id = 10;
    world.controller_leases.push(duplicate);
    world.snakes.push(second_snake);
    for _ in 0..2 {
        assert_eq!(
            select(&world, "run", ControllerKind::Player, 1_300),
            Err(ControllerError::ReclaimRejected("ambiguous"))
        );
        world.controller_leases.reverse();
    }
    world.snakes[1].alive = false;
    assert_eq!(
        select(&world, "run", ControllerKind::Player, 1_300),
        Ok(Some(7))
    );
    world.controller_leases[0].status = ControllerLeaseStatus::Connected;
    assert_eq!(
        select(&world, "run", ControllerKind::Player, 1_300),
        Ok(None)
    );
}

#[test]
fn delayed_reclaim_preserves_newer_queued_action_receive_time() {
    let mut lease = lease();
    let mut snake = snake();
    lease.last_observed_at_ms = 1_500;
    let mut input = reclaim_input(1_800);
    input.received_at_ms = 1_200;
    let prepared = prepare_reclaim(&lease, &snake, input, TIMING).unwrap();
    commit_reclaim(&mut lease, &mut snake, prepared).unwrap();
    assert_eq!(lease.latest_action.accepted_at_ms, 1_200);
    let mut queued = action(0.5, false, 3, 3, 1_300);
    queued.connection_id = 12;
    let proposal = prepare_queued_latest_action(&lease, queued, 1_800).unwrap();
    commit_latest_action(&mut lease, proposal).unwrap();
    assert_eq!(lease.latest_action.accepted_at_ms, 1_300);
    assert_eq!(
        prepare_controller_boundary(&lease, &snake, 1_800, TIMING)
            .unwrap()
            .source(),
        ExternalControlSource::ReservedNeutral
    );
}

#[test]
fn reclaim_keeps_source_until_delivery_then_invalidates_old_connection() {
    let mut lease = lease();
    let mut snake = snake();
    snake.observation_delivered = true;
    let disconnect = prepare_disconnect(&lease, &snake, 11, 1_200, TIMING).unwrap();
    commit_disconnect(&mut lease, &mut snake, disconnect).unwrap();
    let before_lease = lease.clone();
    let before_snake = snake.clone();
    let proposal = prepare_reclaim(&lease, &snake, reclaim_input(1_300), TIMING).unwrap();
    assert_eq!(lease, before_lease);
    assert_eq!(snake, before_snake);
    let mut invalid = reclaim_input(1_300);
    invalid.next_resume_token = "bad".into();
    assert!(prepare_reclaim(&lease, &snake, invalid, TIMING).is_err());
    assert_eq!(lease, before_lease);
    assert_eq!(snake, before_snake);
    commit_reclaim(&mut lease, &mut snake, proposal.clone()).unwrap();
    assert_eq!(lease.connection_id, Some(12));
    assert_eq!(lease.snake_id, before_lease.snake_id);
    assert_eq!(snake.brain, before_snake.brain);
    assert!(snake.observation_delivered);
    assert_eq!(
        snake.delivered_observation_points,
        before_snake.delivered_observation_points
    );
    assert_eq!(lease.status, ControllerLeaseStatus::Connected);
    assert_eq!(lease.grace_expires_at_ms, None);
    assert_eq!(snake.turn, 0.0);
    assert!(!snake.input_boost);
    let committed = lease.clone();
    assert!(commit_reclaim(&mut lease, &mut snake, proposal).is_err());
    assert_eq!(lease, committed);
    assert!(prepare_latest_action(&lease, action(0.5, true, 2, 3, 1_400)).is_err());
    let mut fresh_action = action(0.5, false, 2, 3, 1_400);
    fresh_action.connection_id = 12;
    commit_latest_action(
        &mut lease,
        prepare_latest_action(&committed, fresh_action).unwrap(),
    )
    .unwrap();
    assert!(prepare_reclaim(&lease, &snake, reclaim_input(1_500), TIMING).is_err());
}

#[test]
fn reclaim_counts_the_unapplied_command_once_and_preserves_command_history() {
    use crate::contracts::ExternalRewardControlSourceV1::Held;
    use crate::engine::external_reward::{
        ExternalRewardLedger, RewardBoundary, RewardLifeIdentity,
    };

    let mut lease = lease();
    let mut snake = snake();
    lease.kind = ControllerKind::ReinforcementLearning;
    let boundary = RewardBoundary {
        completed_step: 0,
        elapsed_seconds: 0.0,
    };
    let mut reward = ExternalRewardLedger::new(
        RewardLifeIdentity {
            world_epoch: 1,
            lease_id: lease.id,
            physical_snake_id: snake.id,
            frame_snake_id: snake.frame_v1_id,
        },
        boundary,
        snake.body.len,
        snake.points,
    )
    .unwrap();
    reward.record_command_received(2, Some(1), 0, 0).unwrap();
    reward.record_control_selection(2, Held, 0).unwrap();
    reward.record_command_received(3, Some(2), 0, 0).unwrap();
    lease.reward = Some(reward);
    let latest = prepare_latest_action(&lease, action(0.5, false, 0, 3, 1_100)).unwrap();
    commit_latest_action(&mut lease, latest).unwrap();
    let disconnect = prepare_disconnect(&lease, &snake, 11, 1_200, TIMING).unwrap();
    commit_disconnect(&mut lease, &mut snake, disconnect).unwrap();

    let input = || ReclaimInput {
        kind: ControllerKind::ReinforcementLearning,
        arrival_sequence: 4,
        ..reclaim_input(1_300)
    };
    let snapshot = |lease: &ControllerLease| {
        lease
            .reward
            .unwrap()
            .prepare(1, boundary, None)
            .unwrap()
            .envelope("run")
            .unwrap()
            .control
    };
    let before = snapshot(&lease);
    drop(prepare_reclaim(&lease, &snake, input(), TIMING).unwrap());
    assert_eq!(snapshot(&lease), before);
    let proposal = prepare_reclaim(&lease, &snake, input(), TIMING).unwrap();
    commit_reclaim(&mut lease, &mut snake, proposal.clone()).unwrap();
    let after = snapshot(&lease);
    assert!(!lease.latest_action.is_explicit);
    assert_eq!(after.accepted_count, 2);
    assert_eq!(after.applied_count, 1);
    assert_eq!(after.superseded_count, 1);
    assert_eq!(after.accepted_command, before.accepted_command);
    assert_eq!(after.applied_command, before.applied_command);
    assert!(commit_reclaim(&mut lease, &mut snake, proposal).is_err());
    assert_eq!(snapshot(&lease), after);

    let reward = lease.reward.as_mut().unwrap();
    assert!(reward.record_control_selection(3, Held, 0).is_err());
    reward.record_command_received(5, Some(3), 0, 0).unwrap();
    reward.record_control_selection(5, Held, 0).unwrap();
    let next = snapshot(&lease);
    assert_eq!(
        (
            next.accepted_count,
            next.applied_count,
            next.superseded_count
        ),
        (3, 2, 1)
    );
}

#[test]
fn reclaim_checks_scope_kind_expiry_and_stale_preparation_without_mutation() {
    let mut lease = lease();
    let mut snake = snake();
    let mut input = reclaim_input(1_200);
    input.scope = "another-run";
    assert_eq!(
        prepare_reclaim(&lease, &snake, input, TIMING),
        Err(ControllerError::ReclaimRejected("invalid"))
    );
    let mut wrong_kind = reclaim_input(1_200);
    wrong_kind.kind = ControllerKind::ReinforcementLearning;
    assert_eq!(
        prepare_reclaim(&lease, &snake, wrong_kind, TIMING),
        Err(ControllerError::ReclaimRejected("invalid"))
    );
    let proposal = prepare_reclaim(&lease, &snake, reclaim_input(1_200), TIMING).unwrap();
    lease.scope = "another-run".into();
    let before = lease.clone();
    assert!(commit_reclaim(&mut lease, &mut snake, proposal).is_err());
    assert_eq!(lease, before);
    lease.scope = "run".into();
    let disconnect = prepare_disconnect(&lease, &snake, 11, 1_200, TIMING).unwrap();
    commit_disconnect(&mut lease, &mut snake, disconnect).unwrap();
    assert!(prepare_reclaim(&lease, &snake, reclaim_input(31_199), TIMING).is_ok());
    assert_eq!(
        prepare_reclaim(&lease, &snake, reclaim_input(31_200), TIMING),
        Err(ControllerError::ReclaimRejected("expired"))
    );
    snake.alive = false;
    assert_eq!(
        prepare_reclaim(&lease, &snake, reclaim_input(1_300), TIMING),
        Err(ControllerError::ReclaimRejected("snake-unavailable"))
    );
}

#[test]
fn delayed_disconnect_keeps_original_grace_and_does_not_revive_held_boost() {
    let mut lease = lease();
    let mut snake = snake();
    lease.last_observed_at_ms = 1_700;
    assert!(prepare_disconnect(&lease, &snake, 11, 1_200, TIMING).is_err());
    assert!(prepare_queued_disconnect(&lease, &snake, 10, 1_200, 1_800, TIMING).is_err());
    let proposal = prepare_queued_disconnect(&lease, &snake, 11, 1_200, 1_800, TIMING).unwrap();
    commit_disconnect(&mut lease, &mut snake, proposal).unwrap();
    assert_eq!(lease.disconnected_at_ms, Some(1_200));
    assert_eq!(lease.grace_expires_at_ms, Some(31_200));
    assert_eq!(lease.last_observed_at_ms, 1_800);
    assert_eq!(lease.status, ControllerLeaseStatus::ReservedNeutral);
    assert!(!snake.input_boost);
    assert_eq!(
        prepare_controller_boundary(&lease, &snake, 31_199, TIMING)
            .unwrap()
            .source(),
        ExternalControlSource::ReservedNeutral
    );
    assert_eq!(
        prepare_controller_boundary(&lease, &snake, 31_200, TIMING)
            .unwrap()
            .source(),
        ExternalControlSource::NeuralTakeover
    );
}

#[test]
fn queued_action_keeps_receive_time_and_cannot_refresh_expired_input() {
    let mut lease = lease();
    lease.last_observed_at_ms = 1_600;
    let input = action(0.8, true, 2, 2, 1_100);
    assert!(prepare_latest_action(&lease, input).is_err());
    assert!(prepare_queued_latest_action(&lease, input, 1_599).is_err());
    let proposal = prepare_queued_latest_action(&lease, input, 1_700).unwrap();
    commit_latest_action(&mut lease, proposal).unwrap();
    assert_eq!(lease.latest_action.accepted_at_ms, 1_100);
    assert_eq!(lease.last_observed_at_ms, 1_700);
    assert_eq!(
        prepare_controller_boundary(&lease, &snake(), 1_700, TIMING)
            .unwrap()
            .source(),
        ExternalControlSource::ReservedNeutral
    );
    assert!(prepare_queued_latest_action(&lease, input, 1_700).is_err());
    let release = action(-0.2, false, 3, 3, 1_650);
    let proposal = prepare_queued_latest_action(&lease, release, 1_700).unwrap();
    commit_latest_action(&mut lease, proposal).unwrap();
    assert_eq!(
        prepare_controller_boundary(&lease, &snake(), 1_700, TIMING)
            .unwrap()
            .source(),
        ExternalControlSource::HeldAction {
            turn: -0.2,
            boost: false
        }
    );
}

#[test]
fn latest_value_replaces_old_action_and_rejects_stale_assignment() {
    let mut lease = lease();
    let first = prepare_latest_action(&lease, action(-0.5, true, 2, 2, 1_100))
        .expect("first latest action should stage");
    commit_latest_action(&mut lease, first).expect("first action should commit");
    let newest = prepare_latest_action(&lease, action(0.75, false, 3, 3, 1_120))
        .expect("newest action should stage");
    commit_latest_action(&mut lease, newest).expect("newest action should replace first");

    let proposal = prepare_controller_boundary(&lease, &snake(), 1_200, TIMING)
        .expect("fresh action should select");
    assert_eq!(
        proposal.source(),
        ExternalControlSource::HeldAction {
            turn: 0.75,
            boost: false
        }
    );
    assert!(matches!(
        prepare_latest_action(
            &lease,
            LatestActionInput {
                lease_id: 6,
                ..action(0.0, false, 4, 4, 1_130)
            }
        ),
        Err(ControllerError::StaleLease {
            expected: 7,
            actual: 6
        })
    ));
    assert!(matches!(
        prepare_latest_action(
            &lease,
            LatestActionInput {
                connection_id: 10,
                ..action(0.0, false, 4, 4, 1_130)
            }
        ),
        Err(ControllerError::StaleConnection {
            expected: Some(11),
            actual: 10
        })
    ));
}

#[test]
fn connected_action_holds_for_exactly_500_ms_then_becomes_neutral() {
    let lease = lease();
    let fresh = prepare_controller_boundary(&lease, &snake(), 1_499, TIMING)
        .expect("action before deadline should hold");
    assert!(matches!(
        fresh.source(),
        ExternalControlSource::HeldAction { .. }
    ));
    let expired = prepare_controller_boundary(&lease, &snake(), 1_500, TIMING)
        .expect("action at deadline should neutralize");
    assert_eq!(expired.source(), ExternalControlSource::ReservedNeutral);
    assert!(!expired.begins_neural_takeover());
}

#[test]
fn disconnect_holds_then_neutralizes_then_takes_over_once() {
    let mut lease = lease();
    let mut snake = snake();
    snake.observation_delivered = true;
    let disconnect =
        prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
    commit_disconnect(&mut lease, &mut snake, disconnect).expect("disconnect should commit");
    assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
    assert_eq!(snake.turn, 0.25);
    assert!(snake.input_boost);

    let held = prepare_controller_boundary(&lease, &snake, 1_499, TIMING)
        .expect("held boundary should stage");
    commit_controller_boundary(&mut lease, &mut snake, held).expect("held boundary should commit");
    assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);

    let neutral = prepare_controller_boundary(&lease, &snake, 1_500, TIMING)
        .expect("neutral boundary should stage");
    commit_controller_boundary(&mut lease, &mut snake, neutral)
        .expect("neutral boundary should commit");
    assert_eq!(lease.status, ControllerLeaseStatus::ReservedNeutral);
    assert_eq!(snake.turn, 0.0);
    assert!(!snake.input_boost);

    let still_reserved = prepare_controller_boundary(&lease, &snake, 31_099, TIMING)
        .expect("pre-expiry grace should remain exclusive");
    assert_eq!(
        still_reserved.source(),
        ExternalControlSource::ReservedNeutral
    );
    assert!(!still_reserved.begins_neural_takeover());

    let takeover = prepare_controller_boundary(&lease, &snake, 31_100, TIMING)
        .expect("grace expiry should stage takeover");
    assert_eq!(takeover.source(), ExternalControlSource::NeuralTakeover);
    assert!(takeover.begins_neural_takeover());
    commit_controller_boundary(&mut lease, &mut snake, takeover).expect("takeover should commit");
    assert_eq!(lease.status, ControllerLeaseStatus::NeuralTakeover);
    assert_eq!(lease.takeover_committed_at_ms, Some(31_100));
    assert!(
        snake.observation_delivered,
        "takeover retains the physical-life delivery flag"
    );
    assert_eq!(snake.turn, 0.0);
    assert!(!snake.input_boost);

    let later = prepare_controller_boundary(&lease, &snake, 31_101, TIMING)
        .expect("committed takeover should remain neural");
    assert_eq!(later.source(), ExternalControlSource::NeuralTakeover);
    assert!(!later.begins_neural_takeover());
}

#[test]
fn stale_action_at_disconnect_enters_neutral_grace_immediately() {
    let mut lease = lease();
    let mut snake = snake();
    let disconnect = prepare_disconnect(&lease, &snake, 11, 2_000, TIMING)
        .expect("stale disconnect should stage");
    commit_disconnect(&mut lease, &mut snake, disconnect).expect("stale disconnect should commit");
    assert_eq!(lease.status, ControllerLeaseStatus::ReservedNeutral);
    assert_eq!(lease.input_hold_expires_at_ms, Some(1_500));
    assert_eq!(lease.grace_expires_at_ms, Some(32_000));
    assert_eq!(snake.turn, 0.0);
    assert!(!snake.input_boost);
}

#[test]
fn grace_never_runs_a_brain_and_missing_takeover_brain_is_explicit() {
    let mut lease = lease();
    let mut snake = snake();
    let disconnect =
        prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
    commit_disconnect(&mut lease, &mut snake, disconnect).expect("disconnect should commit");
    snake.brain = None;

    let neutral = prepare_controller_boundary(&lease, &snake, 20_000, TIMING)
        .expect("brainless grace should remain neutral");
    assert_eq!(neutral.source(), ExternalControlSource::ReservedNeutral);
    assert!(matches!(
        prepare_controller_boundary(&lease, &snake, 31_100, TIMING),
        Err(ControllerError::TakeoverBrainMissing(9))
    ));
}

#[test]
fn proposals_are_atomic_against_intervening_actions() {
    let mut lease = lease();
    let mut snake = snake();
    let boundary =
        prepare_controller_boundary(&lease, &snake, 1_100, TIMING).expect("boundary should stage");
    let action = prepare_latest_action(&lease, action(-0.75, false, 2, 2, 1_101))
        .expect("new action should stage");
    commit_latest_action(&mut lease, action).expect("new action should commit");
    let original_snake = snake.clone();
    assert!(matches!(
        commit_controller_boundary(&mut lease, &mut snake, boundary),
        Err(ControllerError::StaleProposal { lease_id: 7 })
    ));
    assert_eq!(snake, original_snake);
}

#[test]
fn equal_millisecond_action_sequence_invalidates_an_older_boundary() {
    let mut lease = lease();
    let mut snake = snake();
    let boundary = prepare_controller_boundary(&lease, &snake, 1_000, TIMING)
        .expect("initial boundary should stage");
    let replacement = prepare_latest_action(&lease, action(-0.9, false, 2, 2, 1_000))
        .expect("same-millisecond newer sequence should stage");
    commit_latest_action(&mut lease, replacement)
        .expect("same-millisecond newer sequence should commit");
    let lease_after_action = lease.clone();
    let snake_before_commit = snake.clone();

    assert!(matches!(
        commit_controller_boundary(&mut lease, &mut snake, boundary),
        Err(ControllerError::StaleProposal { lease_id: 7 })
    ));
    assert_eq!(lease, lease_after_action);
    assert_eq!(snake, snake_before_commit);
    let current = prepare_controller_boundary(&lease, &snake, 1_000, TIMING)
        .expect("replacement should be current");
    assert_eq!(
        current.source(),
        ExternalControlSource::HeldAction {
            turn: -0.9,
            boost: false,
        }
    );
    assert!(matches!(
        prepare_latest_action(&lease, action(0.0, false, 3, 2, 1_001)),
        Err(ControllerError::ArrivalSequenceRegressed {
            previous: 2,
            current: 2
        })
    ));
}

#[test]
fn every_committed_boundary_advances_the_lease_wall_clock() {
    let mut lease = lease();
    let mut snake = snake();
    assert!(matches!(
        prepare_controller_boundary(&lease, &snake, 999, TIMING),
        Err(ControllerError::WallClockRegressed {
            previous_ms: 1_000,
            current_ms: 999
        })
    ));

    let boundary = prepare_controller_boundary(&lease, &snake, 1_200, TIMING)
        .expect("later boundary should stage");
    commit_controller_boundary(&mut lease, &mut snake, boundary)
        .expect("later boundary should commit");
    assert_eq!(lease.last_observed_at_ms, 1_200);
    assert!(matches!(
        prepare_controller_boundary(&lease, &snake, 1_199, TIMING),
        Err(ControllerError::WallClockRegressed {
            previous_ms: 1_200,
            current_ms: 1_199
        })
    ));
    assert!(matches!(
        prepare_latest_action(&lease, action(0.1, false, 2, 2, 1_199)),
        Err(ControllerError::WallClockRegressed {
            previous_ms: 1_200,
            current_ms: 1_199
        })
    ));
}

#[test]
fn takeover_revalidates_alive_snake_and_brain_before_any_write() {
    let prepare_takeover = || {
        let mut lease = lease();
        let mut snake = snake();
        let disconnect =
            prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
        commit_disconnect(&mut lease, &mut snake, disconnect).expect("disconnect should commit");
        let takeover = prepare_controller_boundary(&lease, &snake, 31_100, TIMING)
            .expect("takeover should stage");
        (lease, snake, takeover)
    };

    let (mut dead_lease, mut dead_snake, dead_takeover) = prepare_takeover();
    dead_snake.alive = false;
    let expected_dead_lease = dead_lease.clone();
    let expected_dead_snake = dead_snake.clone();
    assert!(matches!(
        commit_controller_boundary(&mut dead_lease, &mut dead_snake, dead_takeover),
        Err(ControllerError::StaleProposal { lease_id: 7 })
    ));
    assert_eq!(dead_lease, expected_dead_lease);
    assert_eq!(dead_snake, expected_dead_snake);

    let (mut brain_lease, mut brain_snake, brain_takeover) = prepare_takeover();
    brain_snake.brain = None;
    let expected_brain_lease = brain_lease.clone();
    let expected_brain_snake = brain_snake.clone();
    assert!(matches!(
        commit_controller_boundary(&mut brain_lease, &mut brain_snake, brain_takeover),
        Err(ControllerError::StaleProposal { lease_id: 7 })
    ));
    assert_eq!(brain_lease, expected_brain_lease);
    assert_eq!(brain_snake, expected_brain_snake);
}

#[test]
fn disconnect_revalidates_snake_control_state_before_any_write() {
    let mut lease = lease();
    let mut snake = snake();
    let disconnect =
        prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
    snake.turn = -0.5;
    let expected_lease = lease.clone();
    let expected_snake = snake.clone();

    assert!(matches!(
        commit_disconnect(&mut lease, &mut snake, disconnect),
        Err(ControllerError::StaleProposal { lease_id: 7 })
    ));
    assert_eq!(lease, expected_lease);
    assert_eq!(snake, expected_snake);
}

#[test]
fn invalid_timing_deadline_overflow_and_non_external_targets_fail() {
    assert!(ControllerTiming::new(0, 30_000).is_err());
    assert!(ControllerTiming::new(500, 500).is_err());

    let mut overflow = lease();
    overflow.latest_action.accepted_at_ms = u64::MAX - 100;
    overflow.last_observed_at_ms = u64::MAX - 100;
    assert!(matches!(
        prepare_controller_boundary(&overflow, &snake(), u64::MAX, TIMING),
        Err(ControllerError::DeadlineOverflow { .. })
    ));

    let lease = lease();
    let mut evolved = snake();
    evolved.kind = SnakeKind::Evolved;
    assert!(matches!(
        prepare_controller_boundary(&lease, &evolved, 1_100, TIMING),
        Err(ControllerError::SnakeNotExternal(9))
    ));
}
