use super::*;

#[test]
fn rl_join_and_native_prefix_stage_reward_without_mutating_authority() {
    use crate::engine::accounting::{StepAccountingConfig, StepAccountingWorkspace};
    use crate::engine::external_reward::RewardBoundary;
    use crate::engine::physics::PhysicsStepKey;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let prepared = authority
        .prepare_controller_join(
            ControllerJoinInput {
                kind: ControllerKind::ReinforcementLearning,
                identity_key: "rl:reward-contract".into(),
                resume_token: "R".repeat(32),
                connection_id: 12,
                arrival_sequence: 2,
                received_at_ms: 200,
                boundary_at_ms: 300,
            },
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
    authority.commit_controller_join(prepared).unwrap();
    let authority_before = authority.state().clone();
    let mut source = authority.state().world.clone();
    let reward = source
        .controller_leases
        .last_mut()
        .unwrap()
        .reward
        .as_mut()
        .unwrap();
    assert!(!reward.has_delivery());
    let initial = reward
        .prepare(
            1,
            RewardBoundary {
                completed_step: 0,
                elapsed_seconds: 0.0,
            },
            None,
        )
        .unwrap();
    reward.commit_delivery(&initial).unwrap();
    let source_before = source.clone();
    let key = PhysicsStepKey::new(authority.world_epoch(), 3, 40, 7, 11, [0x5a; 32], 1);
    let config = StepAccountingConfig::typescript_defaults();
    let mut workspace = StepAccountingWorkspace::new();
    let prepared = workspace
        .prepare(key, &source, 0.0, 1.0 / 60.0, config, source.snakes.len())
        .unwrap();
    let mut working = source.clone();
    prepared
        .apply_to_working_copy(
            key,
            &source,
            0.0,
            1.0 / 60.0,
            config,
            source.snakes.len(),
            &mut working,
        )
        .unwrap();
    let reward = working
        .controller_leases
        .last()
        .unwrap()
        .reward
        .as_ref()
        .unwrap();
    let interval = reward
        .prepare(
            2,
            RewardBoundary {
                completed_step: 1,
                elapsed_seconds: 1.0 / 60.0,
            },
            None,
        )
        .unwrap()
        .envelope("prefix-fixture")
        .unwrap();
    assert!((interval.deltas.alive_seconds_delta - 1.0 / 60.0).abs() < 1e-12);
    assert!((interval.deltas.survival_points_delta - 0.01).abs() < 1e-12);
    assert_eq!(interval.deltas.food_value_delta, 0.0);
    assert_eq!(source, source_before);
    assert_eq!(authority.state(), &authority_before);
    // A cold God Mode kill closes the same native ledger. The following
    // physics step must not reinterpret the already-dead record as live.
    authority
        .candidate
        .world
        .controller_leases
        .last_mut()
        .unwrap()
        .reward
        .as_mut()
        .unwrap()
        .commit_delivery(&initial)
        .unwrap();
    let frame_id = authority.candidate.world.snakes.last().unwrap().frame_v1_id;
    let physics = authority
        .running_step_config(RunningStepWorkLimits::provisional_defaults())
        .unwrap()
        .world_step
        .physics;
    authority.apply_god_mode_kill(frame_id, physics).unwrap();
    let closed = authority
        .candidate
        .world
        .controller_leases
        .last()
        .unwrap()
        .reward
        .as_ref()
        .unwrap()
        .prepare(
            2,
            RewardBoundary {
                completed_step: 0,
                elapsed_seconds: 0.0,
            },
            None,
        )
        .unwrap()
        .envelope("god-mode-fixture")
        .unwrap();
    assert_eq!(
        closed.transition_kind,
        crate::contracts::ExternalRewardTransitionKind::Terminal
    );
    assert_eq!(closed.deltas.retained_growth_delta, 0);
    assert_eq!(closed.deltas.length_delta, 0);
}

#[test]
fn reward_observation_markers_commit_with_complete_native_output_admission() {
    use crate::contracts::ExternalRewardTransitionKind;
    use crate::engine::external_reward::{
        ExternalRewardLedger, RewardBoundary, RewardLifeIdentity,
    };
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let external_id = EXTERNAL_ENTITY_ID_START;
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        external_id,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let world_epoch = authority.world_epoch();
    let snake = authority
        .candidate
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == external_id)
        .unwrap();
    let reward = ExternalRewardLedger::new(
        RewardLifeIdentity {
            world_epoch,
            lease_id: 1,
            physical_snake_id: external_id,
            frame_snake_id: snake.frame_v1_id,
        },
        RewardBoundary {
            completed_step: 0,
            elapsed_seconds: 0.0,
        },
        snake.body.len,
        snake.points,
    )
    .unwrap();
    let lease = authority
        .candidate
        .world
        .controller_leases
        .last_mut()
        .unwrap();
    lease.kind = ControllerKind::ReinforcementLearning;
    lease.reward = Some(reward);
    let before = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    let first = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .unwrap()
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
        _ => panic!("reward observation must wait for native output capacity"),
    };
    let initial = first.reward.unwrap().envelope("admission-fixture").unwrap();
    assert_eq!(
        initial.transition_kind,
        ExternalRewardTransitionKind::Initial
    );
    assert_eq!(
        initial.deltas,
        crate::contracts::ExternalRewardDeltasV1::default()
    );
    assert_eq!(authority.state(), &before);
    coordinator
        .admit_pending_observations(&mut authority)
        .unwrap();
    assert!(authority
        .candidate
        .world
        .controller_leases
        .last()
        .unwrap()
        .reward
        .unwrap()
        .has_delivery());
    let once = authority.state().clone();
    assert!(coordinator
        .admit_pending_observations(&mut authority)
        .is_err());
    assert_eq!(authority.state(), &once);
    let next = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 101,
                wall_accumulator_seconds: 0.0,
            },
        )
        .unwrap()
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
        _ => panic!("next observation must carry the accumulated reward"),
    };
    let interval = next.reward.unwrap().envelope("admission-fixture").unwrap();
    assert_eq!(interval.transition_kind, ExternalRewardTransitionKind::Live);
    assert_eq!(
        interval.previous_observation_sequence,
        Some(initial.observation_sequence)
    );
    assert!((interval.deltas.alive_seconds_delta - 1.0 / 60.0).abs() < 1e-12);
    assert!((interval.deltas.survival_points_delta - 0.01).abs() < 1e-12);
    coordinator
        .admit_pending_observations(&mut authority)
        .unwrap();
    let old_frame_id = authority
        .candidate
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == external_id)
        .unwrap()
        .frame_v1_id;
    let physics = authority
        .running_step_config(RunningStepWorkLimits::provisional_defaults())
        .unwrap()
        .world_step
        .physics;
    authority
        .apply_god_mode_kill(old_frame_id, physics)
        .unwrap();
    let assignment = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 102,
                wall_accumulator_seconds: 0.0,
            },
        )
        .unwrap()
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
        _ => panic!("death must retain its old reward until replacement admission"),
    };
    assert!(matches!(
        assignment.delivery_kind,
        crate::engine::ExternalDeliveryEventKind::ReplacementAssignment { .. }
    ));
    let terminal = assignment
        .reward
        .unwrap()
        .envelope("admission-fixture")
        .unwrap();
    assert_eq!(
        terminal.transition_kind,
        ExternalRewardTransitionKind::Terminal
    );
    assert_eq!(terminal.snake_id, old_frame_id);
    assert_eq!(
        terminal.previous_observation_sequence,
        Some(interval.observation_sequence)
    );
    assert_eq!(terminal.deltas.alive_seconds_delta, 0.0);
    assert_eq!(terminal.deltas.length_delta, 0);
    assert_ne!(terminal.lease_id, format!("{:016x}", assignment.lease_id));
    coordinator
        .submit_external_delivery_results(
            &mut authority,
            &[ExternalDeliveryResult {
                step_key: assignment.step_key,
                event_sequence: assignment.event_sequence,
                connection_id: assignment.connection_id,
                lease_id: assignment.lease_id,
                accepted: true,
            }],
        )
        .unwrap();
    let successor = authority
        .candidate
        .world
        .controller_leases
        .last()
        .unwrap()
        .reward
        .unwrap();
    assert!(!successor.has_delivery());
    assert_eq!(successor.identity().lease_id, assignment.lease_id);
}

#[test]
fn controlled_wall_death_waits_for_reliable_replacement_assignment_before_publication() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let old_snake_id = EXTERNAL_ENTITY_ID_START;
    let old_lease_id = 1;
    let connection_id = 7;
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        old_snake_id,
        2,
        old_lease_id,
        connection_id,
        WorldPoint { x: 3_490.5, y: 0.0 },
    );
    let source = candidate.clone();
    let old_token = source
        .world
        .controller_leases
        .last()
        .unwrap()
        .resume_token
        .clone();
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("controlled wall-death fixture must construct the coordinator");

    let (event, new_token) = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("controlled death must stage one reliable replacement assignment")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => {
            assert_eq!(batch.events().len(), 1);
            assert_eq!(batch.remaining(), 1);
            assert_eq!(batch.observation(0), None);
            let event = batch.events()[0];
            assert!(matches!(
                event.delivery_kind,
                ExternalDeliveryEventKind::ReplacementAssignment { frame_v1_id }
                    if frame_v1_id == source.allocators.next_frame_v1_id
            ));
            assert_eq!(event.snake_id, source.allocators.next_external_id);
            assert_eq!(event.lease_id, source.allocators.next_controller_lease_id);
            assert_eq!(event.connection_id, connection_id);
            assert!(event.position.x.is_finite() && event.position.y.is_finite());
            assert!(event.direction.is_finite());
            let token = batch
                .resume_token(0)
                .expect("replacement assignment must carry one opaque token")
                .to_owned();
            assert_eq!(token.len(), 32);
            assert_ne!(token, old_token);
            (event, token)
        }
        RunningStepProgress::Published(_) => {
            panic!("replacement assignment must resolve before authority publication")
        }
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("controlled wall-death fixture must remain nonterminal")
        }
    };
    assert_eq!(authority.state(), &source);

    let stale = ExternalDeliveryResult {
        step_key: event.step_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: old_lease_id,
        accepted: true,
    };
    let stale_resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[stale])
        .expect("superseded lease result must be ignored");
    assert_eq!(stale_resolution.matched_acceptances, 0);
    assert_eq!(stale_resolution.ignored_results, 1);
    assert!(matches!(
        stale_resolution.state,
        ExternalDeliveryState::Pending(batch) if batch.remaining() == 1
    ));
    assert_eq!(authority.state(), &source);

    let accepted = ExternalDeliveryResult {
        step_key: event.step_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: event.lease_id,
        accepted: true,
    };
    let resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[accepted])
        .expect("accepted replacement assignment must publish the complete fixed step");
    assert_eq!(resolution.matched_acceptances, 1);
    assert_eq!(resolution.matched_failures, 0);
    let outcome = match resolution.state {
        ExternalDeliveryState::Published(outcome) => outcome,
        _ => panic!("the resolved replacement must publish exactly once"),
    };
    assert_eq!(outcome.publication.completed_step, 1);
    assert_eq!(outcome.diagnostics.external_deliveries_pending, 0);

    let published = authority.state();
    assert!(published
        .world
        .snakes
        .iter()
        .all(|snake| snake.id != old_snake_id));
    let replacement = published
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == event.snake_id)
        .expect("fresh replacement snake must become authoritative");
    assert!(replacement.alive);
    assert_eq!(replacement.kind, SnakeKind::External);
    assert_eq!(replacement.position, event.position);
    assert_eq!(replacement.direction, event.direction);
    assert_eq!(replacement.body.len, 5);
    assert_eq!(
        published.world.body_points[replacement.body.start],
        replacement.position
    );
    let lease = published
        .world
        .controller_leases
        .iter()
        .find(|lease| lease.id == event.lease_id)
        .expect("fresh replacement lease must become authoritative");
    assert_eq!(lease.snake_id, event.snake_id);
    assert_eq!(lease.connection_id, Some(connection_id));
    assert_eq!(lease.status, ControllerLeaseStatus::Connected);
    assert_eq!(lease.resume_token, new_token);
    assert_eq!(lease.latest_action.turn, 0.0);
    assert!(!lease.latest_action.boost);

    let brain = published
        .brains
        .iter()
        .find(|brain| brain.owner == BrainOwner::Entity(event.snake_id))
        .expect("fresh replacement brain must become authoritative");
    assert_eq!(brain.handle.id, source.allocators.next_brain_id);
    assert_eq!(
        brain.non_population_weights.as_ref().unwrap().len(),
        graph.total_parameters
    );
    assert!(brain
        .non_population_weights
        .as_ref()
        .unwrap()
        .iter()
        .any(|weight| weight.to_bits() != 0));
    assert_eq!(brain.recurrent.len(), graph.total_state_size);
    assert!(brain.recurrent.iter().all(|value| value.to_bits() == 0));
    assert_ne!(
        published.rng.external_controller,
        source.rng.external_controller
    );
    assert_eq!(published.rng.evolution, source.rng.evolution);
    assert_eq!(
        published.allocators.next_external_id,
        source.allocators.next_external_id + 1
    );
    assert_eq!(
        published.allocators.next_frame_v1_id,
        source.allocators.next_frame_v1_id + 1
    );
    assert_eq!(
        published.allocators.next_brain_id,
        source.allocators.next_brain_id + 1
    );
    assert_eq!(
        published.allocators.next_controller_lease_id,
        source.allocators.next_controller_lease_id + 1
    );

    let published = published.clone();
    let duplicate = coordinator
        .submit_external_delivery_results(&mut authority, &[accepted])
        .expect("duplicate replacement result after publication must be ignored");
    assert_eq!(duplicate.matched_acceptances, 0);
    assert_eq!(duplicate.ignored_results, 1);
    assert!(matches!(duplicate.state, ExternalDeliveryState::Idle));
    assert_eq!(authority.state(), &published);
}

#[test]
fn failed_replacement_assignment_invalidates_the_old_snake_token_and_keeps_grace() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let old_snake_id = EXTERNAL_ENTITY_ID_START;
    let old_lease_id = 1;
    let connection_id = 7;
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        old_snake_id,
        2,
        old_lease_id,
        connection_id,
        WorldPoint { x: 3_490.5, y: 0.0 },
    );
    let old_token = candidate
        .world
        .controller_leases
        .last()
        .unwrap()
        .resume_token
        .clone();
    let source = candidate.clone();
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("failed-assignment fixture must construct the coordinator");
    let (event, rejected_token) = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("controlled death must stage its assignment")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => {
            (batch.events()[0], batch.resume_token(0).unwrap().to_owned())
        }
        RunningStepProgress::Published(_) => panic!("assignment must resolve first"),
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("failed-assignment fixture must remain nonterminal")
        }
    };
    assert_ne!(rejected_token, old_token);
    assert_eq!(authority.state(), &source);

    let rejected = ExternalDeliveryResult {
        step_key: event.step_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: event.lease_id,
        accepted: false,
    };
    let resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[rejected])
        .expect("failed local assignment send must publish the disconnected replacement");
    assert_eq!(resolution.matched_failures, 1);
    assert!(matches!(
        resolution.state,
        ExternalDeliveryState::Published(_)
    ));

    let published = authority.state();
    assert!(published
        .world
        .snakes
        .iter()
        .all(|snake| snake.id != old_snake_id));
    let replacement = published
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == event.snake_id)
        .expect("failed delivery must not discard the fresh snake");
    assert!(replacement.alive);
    assert_eq!(replacement.turn, 0.0);
    assert!(!replacement.input_boost);
    let lease = published
        .world
        .controller_leases
        .iter()
        .find(|lease| lease.id == event.lease_id)
        .expect("fresh lease must enter disconnect grace");
    assert_ne!(lease.resume_token, old_token);
    assert_eq!(lease.resume_token, rejected_token);
    assert_eq!(lease.connection_id, None);
    assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
    assert_eq!(lease.disconnected_at_ms, Some(100));
    assert_eq!(lease.input_hold_expires_at_ms, Some(600));
    assert_eq!(lease.grace_expires_at_ms, Some(30_100));
    assert_eq!(lease.takeover_committed_at_ms, None);

    let before_reclaim = authority.state().clone();
    let failed_reclaim =
        authority.prepare_controller_reclaim(crate::engine::controllers::ReclaimInput {
            kind: lease.kind,
            scope: &lease.scope,
            resume_token: &old_token,
            next_resume_token: "A".repeat(32),
            connection_id: 8,
            arrival_sequence: 2,
            received_at_ms: 200,
            boundary_at_ms: 200,
        });
    assert!(matches!(failed_reclaim, Err(reason) if reason == "invalid"));
    assert_eq!(authority.state(), &before_reclaim);

    let duplicate = coordinator
        .submit_external_delivery_results(
            &mut authority,
            &[ExternalDeliveryResult {
                accepted: true,
                ..rejected
            }],
        )
        .expect("late acceptance after a failed assignment must be ignored");
    assert_eq!(duplicate.matched_acceptances, 0);
    assert_eq!(duplicate.ignored_results, 1);
    assert!(matches!(duplicate.state, ExternalDeliveryState::Idle));
}

#[test]
fn already_disconnected_controlled_death_removes_only_the_dead_lease() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let old_snake_id = EXTERNAL_ENTITY_ID_START;
    let old_lease_id = 1;
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        old_snake_id,
        2,
        old_lease_id,
        7,
        WorldPoint { x: 3_490.5, y: 0.0 },
    );
    let lease = candidate.world.controller_leases.last_mut().unwrap();
    lease.connection_id = None;
    lease.status = ControllerLeaseStatus::HoldingLastInput;
    lease.disconnected_at_ms = Some(100);
    lease.input_hold_expires_at_ms = Some(600);
    lease.grace_expires_at_ms = Some(30_100);
    let snake = candidate.world.snakes.last_mut().unwrap();
    snake.turn = lease.latest_action.turn;
    snake.input_boost = lease.latest_action.boost;
    let source_rng = candidate.rng.clone();
    let source_allocators = candidate.allocators.clone();
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("disconnected wall-death fixture must construct the coordinator");

    let outcome = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("a disconnected controlled death needs no Node assignment result")
    {
        RunningStepProgress::Published(outcome) => outcome,
        RunningStepProgress::ExternalDeliveryPending(_) => {
            panic!("a disconnected dead owner has no socket to receive an assignment")
        }
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("disconnected controlled-death fixture must remain nonterminal")
        }
    };
    assert_eq!(outcome.diagnostics.external_replacement.replacements, 0);
    assert_eq!(
        outcome.diagnostics.external_replacement.removed_dead_leases,
        1
    );
    let published = authority.state();
    let dead = published
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == old_snake_id)
        .expect("the dead body remains part of the running world boundary");
    assert!(!dead.alive);
    assert!(published
        .world
        .controller_leases
        .iter()
        .all(|lease| lease.id != old_lease_id));
    assert_eq!(
        published.rng.external_controller,
        source_rng.external_controller
    );
    assert_eq!(
        published.allocators.next_external_id,
        source_allocators.next_external_id
    );
    assert_eq!(
        published.allocators.next_brain_id,
        source_allocators.next_brain_id
    );
    assert_eq!(
        published.allocators.next_controller_lease_id,
        source_allocators.next_controller_lease_id
    );
}

#[test]
fn regressing_controller_wall_clock_never_starts_a_new_attempt() {
    let graph = default_graph();
    let candidate = complete_running_candidate(&graph);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("complete admitted config must construct the coordinator");
    assert!(matches!(
        coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("first boundary must publish"),
        RunningStepProgress::Published(_)
    ));
    let source = authority.state().clone();
    assert!(matches!(
        coordinator.advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 99,
                wall_accumulator_seconds: 0.0,
            },
        ),
        Err(RunningStepError::RegressingWallClock {
            previous_ms: 100,
            actual_ms: 99,
        })
    ));
    assert_eq!(authority.state(), &source);
}
