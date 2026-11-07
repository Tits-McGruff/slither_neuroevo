use super::*;

#[test]
fn fresh_join_preserves_source_until_delivery_and_rejects_duplicate_commit() {
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let input = || ControllerJoinInput {
        kind: ControllerKind::Player,
        identity_key: "player:new-owner".into(),
        resume_token: "A".repeat(32),
        connection_id: 12,
        arrival_sequence: 2,
        received_at_ms: 200,
        boundary_at_ms: 300,
    };
    let before = authority.state().clone();
    let prepared = authority
        .prepare_controller_join(input(), RunningStepWorkLimits::provisional_defaults())
        .unwrap();
    let duplicate = authority
        .prepare_controller_join(input(), RunningStepWorkLimits::provisional_defaults())
        .unwrap();
    assert_eq!(prepared.frame_v1_id(), duplicate.frame_v1_id());
    assert_eq!(prepared.lease_id(), duplicate.lease_id());
    assert_eq!(authority.state(), &before);
    authority.commit_controller_join(prepared).unwrap();
    let after = authority.state().clone();
    assert_eq!(after.world.snakes.len(), before.world.snakes.len() + 1);
    assert_eq!(after.population, before.population);
    assert_eq!(after.rng.world, before.rng.world);
    assert_eq!(after.rng.evolution, before.rng.evolution);
    assert_eq!(after.rng.baselines, before.rng.baselines);
    assert_eq!(
        after
            .world
            .controller_leases
            .last()
            .unwrap()
            .latest_action
            .accepted_at_ms,
        200
    );
    assert_eq!(
        authority.memory_estimate(),
        estimate_state_memory(authority.state(), &graph).unwrap()
    );
    assert!(authority.commit_controller_join(duplicate).is_err());
    assert_eq!(authority.state(), &after);
}

#[test]
fn fresh_join_budget_failure_and_changed_source_preserve_authority() {
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let input = || ControllerJoinInput {
        kind: ControllerKind::Player,
        identity_key: "player:new-owner".into(),
        resume_token: "A".repeat(32),
        connection_id: 12,
        arrival_sequence: 2,
        received_at_ms: 200,
        boundary_at_ms: 300,
    };
    let before = authority.state().clone();
    let ceiling = authority.memory_ceiling_bytes;
    authority.memory_ceiling_bytes = authority.memory.total_bytes;
    assert!(authority
        .prepare_controller_join(input(), RunningStepWorkLimits::provisional_defaults())
        .is_err());
    assert_eq!(authority.state(), &before);
    authority.memory_ceiling_bytes = ceiling;
    let prepared = authority
        .prepare_controller_join(input(), RunningStepWorkLimits::provisional_defaults())
        .unwrap();
    authority.candidate.world.snakes[0].points += 1.0;
    let changed = authority.state().clone();
    assert!(authority.commit_controller_join(prepared).is_err());
    assert_eq!(authority.state(), &changed);
}

#[test]
fn retained_origin_survives_catalog_release_and_reused_brain_copies() {
    use crate::engine::brain_artifact::BrainSourceKind;
    use crate::engine::control_phase::copy_brains_reusing;
    use crate::engine::retained_origin::packed_weight_digest;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let weights = vec![0.25; graph.compiled().total_parameters].into_boxed_slice();
    let origin = Arc::new(RetainedNeuralOrigin::SavedBrain {
        brain_id: [13; 32],
        weight_digest: packed_weight_digest(&weights),
        source_kind: BrainSourceKind::Distilled,
    });
    let weak = Arc::downgrade(&origin);
    authority
        .spawn_retained_brain(
            weights,
            Arc::clone(&origin),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
    let retained = authority.state().brains.last().unwrap();
    assert!(Arc::ptr_eq(
        retained.retained_origin.as_ref().unwrap(),
        &origin
    ));
    assert_eq!(
        authority
            .state()
            .world
            .snakes
            .last()
            .unwrap()
            .control_accumulator_seconds,
        crate::engine::control_phase::ControlPhaseConfig::typescript_defaults()
            .initial_neural_accumulator_seconds()
    );
    drop(origin); // Deleting a catalog record cannot revoke the native Arc.
    let mut copies = Vec::new();
    copy_brains_reusing(&mut copies, &authority.state().brains, false).unwrap();
    for _ in 0..10 {
        let (weights_copied, _) =
            copy_brains_reusing(&mut copies, &authority.state().brains, true).unwrap();
        assert_eq!(weights_copied, 0);
        assert!(Arc::ptr_eq(
            copies.last().unwrap().retained_origin.as_ref().unwrap(),
            authority
                .state()
                .brains
                .last()
                .unwrap()
                .retained_origin
                .as_ref()
                .unwrap()
        ));
    }
    copies.clear();
    assert!(weak.upgrade().is_some());
    drop(authority);
    assert!(weak.upgrade().is_none());
}

#[test]
fn retained_reuse_releases_old_origin_compacts_body_and_clears_owner_at_full_capacity() {
    let (mut authority, graph) = retained_reuse_fixture();
    let before = authority.state().clone();
    let old = &before.world.snakes[1];
    let weak = Arc::downgrade(
        before
            .brains
            .iter()
            .find(|brain| Some(brain.handle) == old.brain)
            .unwrap()
            .retained_origin
            .as_ref()
            .unwrap(),
    );
    let survivor = &before.world.snakes[2];
    let survivor_body = before.world.body_points
        [survivor.body.start..survivor.body.start + survivor.body.len]
        .to_vec();
    let weights = vec![0.5; graph.compiled().total_parameters].into_boxed_slice();
    let publication = authority
        .resurrect_hall_of_fame(3, weights, RunningStepWorkLimits::provisional_defaults())
        .unwrap();
    let after = authority.state();
    assert_eq!(after.population, before.population);
    assert_eq!(after.brains.len(), before.brains.len());
    assert_eq!(after.world.snakes.len(), before.world.snakes.len());
    assert_eq!(
        after.world.body_points.len(),
        before.world.body_points.len()
    );
    assert_ne!(publication.snake_id, old.id);
    assert_ne!(publication.frame_v1_id, old.frame_v1_id);
    assert_eq!(after.world.snakes[1].id, publication.snake_id);
    assert!(after.world.snakes[1].alive);
    assert_eq!(after.world.snakes[1].control_accumulator_seconds, 0.06);
    let next = &after.world.snakes[2];
    assert_eq!(next.id, survivor.id);
    assert_eq!(next.body.start, survivor.body.start - old.body.len);
    assert_eq!(
        &after.world.body_points[next.body.start..next.body.start + next.body.len],
        survivor_body.as_slice()
    );
    assert_eq!(after.world.pellets.last().unwrap().owner, None);
    assert!(!after
        .brains
        .iter()
        .any(|brain| brain.owner == BrainOwner::Entity(old.id)));
    let brain = after
        .brains
        .iter()
        .find(|brain| Some(brain.handle) == after.world.snakes[1].brain)
        .unwrap();
    assert!(brain.recurrent.iter().all(|value| *value == 0.0));
    assert!(matches!(
        brain.retained_origin.as_deref(),
        Some(RetainedNeuralOrigin::HallOfFame { entry_id: 3, .. })
    ));
    drop(before);
    assert!(weak.upgrade().is_none());
    let full = authority.state().clone();
    assert!(authority
        .resurrect_hall_of_fame(
            4,
            vec![0.0; graph.compiled().total_parameters].into_boxed_slice(),
            RunningStepWorkLimits::provisional_defaults()
        )
        .is_err());
    assert_eq!(authority.state(), &full);
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    assert!(matches!(
        coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0
                }
            )
            .unwrap(),
        RunningStepProgress::Published(_)
    ));
    let newborn = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == publication.snake_id)
        .unwrap();
    assert!(newborn.observation_delivered);
}

#[test]
fn reclaim_transaction_preserves_authority_until_exact_commit() {
    let graph = default_graph();
    let mut source = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut source,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        11,
        WorldPoint { x: 200.0, y: 0.0 },
    );
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    authority.disconnect_controller(1, 11, 200, 200).unwrap();
    let before = authority.state().clone();
    let input = crate::engine::controllers::ReclaimInput {
        kind: ControllerKind::Player,
        scope: "run-test",
        resume_token: "external-delivery-1",
        next_resume_token: "A".repeat(32),
        connection_id: 12,
        arrival_sequence: 2,
        received_at_ms: 300,
        boundary_at_ms: 300,
    };
    let prepared = authority.prepare_controller_reclaim(input).unwrap();
    assert_eq!(prepared.lease_id(), 1);
    assert_eq!(prepared.frame_v1_id(), 2);
    assert_eq!(authority.state(), &before);
    // A failed send drops only its candidate; a successful retry retains
    // the same identity and does not reinitialize any gameplay storage.
    drop(prepared.clone());
    authority
        .commit_controller_reclaim(prepared.clone())
        .unwrap();
    let after = authority.state().clone();
    assert_eq!(after.rng, before.rng);
    assert_eq!(after.allocators, before.allocators);
    assert_eq!(after.brains, before.brains);
    assert_eq!(after.population, before.population);
    assert_eq!(after.world.body_points, before.world.body_points);
    assert_eq!(after.world.controller_leases[0].connection_id, Some(12));
    assert_eq!(
        authority.memory_estimate(),
        estimate_state_memory(authority.state(), &graph).unwrap()
    );
    assert!(authority.commit_controller_reclaim(prepared).is_err());
    assert_eq!(authority.state(), &after);
}

#[test]
fn reclaim_rejects_collisions_budget_and_changed_source_before_any_write() {
    let graph = default_graph();
    let mut source = complete_running_candidate(&graph);
    for (offset, connection) in [(0, 11), (1, 12)] {
        push_connected_external_fixture(
            &mut source,
            &graph,
            EXTERNAL_ENTITY_ID_START + offset,
            2 + offset as u32,
            1 + offset,
            connection,
            WorldPoint {
                x: 200.0 + offset as f64 * 100.0,
                y: 0.0,
            },
        );
    }
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let input = || crate::engine::controllers::ReclaimInput {
        kind: ControllerKind::Player,
        scope: "run-test",
        resume_token: "external-delivery-1",
        next_resume_token: "A".repeat(32),
        connection_id: 13,
        arrival_sequence: 2,
        received_at_ms: 300,
        boundary_at_ms: 300,
    };
    let before = authority.state().clone();
    let mut collision = input();
    collision.connection_id = 12;
    assert!(authority.prepare_controller_reclaim(collision).is_err());
    let mut collision = input();
    collision.next_resume_token = "external-delivery-2".into();
    assert!(authority.prepare_controller_reclaim(collision).is_err());
    let ceiling = authority.memory_ceiling_bytes;
    authority.memory_ceiling_bytes = authority.memory.total_bytes;
    assert!(authority.prepare_controller_reclaim(input()).is_err());
    authority.memory_ceiling_bytes = ceiling;
    assert_eq!(authority.state(), &before);
    let prepared = authority.prepare_controller_reclaim(input()).unwrap();
    authority.candidate.generation.completed_step += 1;
    let changed = authority.state().clone();
    assert!(authority.commit_controller_reclaim(prepared).is_err());
    assert_eq!(authority.state(), &changed);
}

#[test]
fn validating_connected_controller_state_is_rng_pure() {
    let graph = default_graph();
    let mut live = candidate(&graph, 1);
    live.phase = AuthorityPhase::Running;
    push_external_snake(
        &mut live,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        1,
        WorldPoint { x: 1.0, y: 2.0 },
    );
    live.allocators.next_controller_lease_id = 2;
    live.world.controller_leases.push(connected_lease(
        1,
        EXTERNAL_ENTITY_ID_START,
        7,
        "os-entropy-token",
    ));
    let before = live.rng.clone();
    let estimate = estimate_state_memory(&live, &graph).unwrap();
    let state = own(live, graph, estimate.total_bytes).unwrap();
    assert_eq!(state.state().rng, before);
}

#[test]
fn controller_lease_status_tokens_connections_and_grace_are_consistent() {
    let graph = default_graph();
    let mut live = candidate(&graph, 2);
    live.phase = AuthorityPhase::Running;
    let first_id = EXTERNAL_ENTITY_ID_START;
    let second_id = first_id + 1;
    push_external_snake(
        &mut live,
        &graph,
        first_id,
        1,
        WorldPoint { x: 1.0, y: 2.0 },
    );
    push_external_snake(
        &mut live,
        &graph,
        second_id,
        2,
        WorldPoint { x: 3.0, y: 4.0 },
    );
    live.world.controller_leases = vec![
        connected_lease(1, first_id, 7, "same-token"),
        connected_lease(2, second_id, 8, "same-token"),
    ];
    live.allocators.next_controller_lease_id = 3;
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease.resume_token",
            ..
        })
    ));

    live.world.controller_leases[1].resume_token = "other-token".into();
    live.world.controller_leases[1].connection_id = Some(7);
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease.connection_id",
            ..
        })
    ));

    live.world.controller_leases.truncate(1);
    live.world.controller_leases[0].scope = "other-run".into();
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease.scope",
            ..
        })
    ));
    live.world.controller_leases[0].scope = "run-test".into();
    let lease = &mut live.world.controller_leases[0];
    lease.connection_id = None;
    lease.status = ControllerLeaseStatus::HoldingLastInput;
    lease.latest_action.accepted_at_ms = 800;
    lease.last_observed_at_ms = 1_000;
    lease.disconnected_at_ms = Some(1_000);
    lease.input_hold_expires_at_ms = Some(1_300);
    lease.grace_expires_at_ms = Some(31_000);
    live.world.snakes[0].turn = lease.latest_action.turn;
    live.world.snakes[0].input_boost = lease.latest_action.boost;
    assert!(own(live.clone(), Arc::clone(&graph), usize::MAX).is_ok());

    live.world.snakes[0].turn = 0.5;
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease",
            ..
        })
    ));
    live.world.snakes[0].turn = 0.0;
    live.world.snakes[0].input_boost = false;

    live.world.controller_leases[0].latest_action.accepted_at_ms = 100;
    live.world.controller_leases[0].input_hold_expires_at_ms = Some(600);
    live.world.snakes[0].turn = live.world.controller_leases[0].latest_action.turn;
    live.world.snakes[0].input_boost = live.world.controller_leases[0].latest_action.boost;
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease",
            ..
        })
    ));
    live.world.snakes[0].turn = 0.0;
    live.world.snakes[0].input_boost = false;
    live.world.controller_leases[0].status = ControllerLeaseStatus::ReservedNeutral;
    assert!(own(live.clone(), Arc::clone(&graph), usize::MAX).is_ok());

    live.world.controller_leases[0].status = ControllerLeaseStatus::NeuralTakeover;
    live.world.controller_leases[0].takeover_committed_at_ms = Some(30_999);
    live.world.controller_leases[0].last_observed_at_ms = 30_999;
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease",
            ..
        })
    ));
    live.world.controller_leases[0].takeover_committed_at_ms = Some(31_000);
    live.world.controller_leases[0].last_observed_at_ms = 31_000;
    assert!(own(live, graph, usize::MAX).is_ok());
}

#[test]
fn controller_leases_reject_non_external_snake_kinds() {
    let graph = default_graph();
    for (kind, id) in [
        (SnakeKind::Evolved, 1),
        (SnakeKind::Baseline, BASELINE_ENTITY_ID_START),
        (SnakeKind::Resurrected, RESURRECTED_ENTITY_ID_START),
    ] {
        let mut live = candidate(&graph, 1);
        live.phase = AuthorityPhase::Running;
        push_evolved_snake(&mut live, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
        if kind != SnakeKind::Evolved {
            let snake = &mut live.world.snakes[0];
            snake.id = id;
            snake.kind = kind;
            snake.population_slot = None;
            snake.brain = None;
            if kind == SnakeKind::Baseline {
                snake.baseline_slot = Some(0);
                snake.baseline_strategy = Some(BaselineStrategyState::Roam);
                live.allocators.next_baseline_id = id + 1;
                enable_one_live_baseline(&mut live, id, None);
            } else {
                live.allocators.next_resurrected_id = id + 1;
                let handle = BrainHandle {
                    id: live.allocators.next_brain_id,
                    epoch: live.generation.population_epoch,
                };
                live.allocators.next_brain_id += 1;
                snake.brain = Some(handle);
                let weights = vec![0.0; graph.compiled().total_parameters].into_boxed_slice();
                live.brains.push(BrainRuntimeState {
                    handle,
                    owner: BrainOwner::Entity(id),
                    retained_origin: Some(Arc::new(RetainedNeuralOrigin::HallOfFame {
                        entry_id: 1,
                        weight_digest: crate::engine::retained_origin::packed_weight_digest(
                            &weights,
                        ),
                    })),
                    non_population_weights: Some(weights),
                    recurrent: vec![0.0; graph.compiled().total_state_size].into_boxed_slice(),
                });
            }
            live.allocators.next_entity_id = 1;
        }
        live.world
            .controller_leases
            .push(connected_lease(1, id, 7, "non-external-token"));
        live.allocators.next_controller_lease_id = 2;
        assert!(matches!(
            own(live, Arc::clone(&graph), usize::MAX),
            Err(StateError::InvalidField {
                field: "controller_lease.snake_id",
                ..
            })
        ));
    }
}

#[test]
fn neural_takeover_rejects_a_brainless_external_snake() {
    let graph = default_graph();
    let mut live = candidate(&graph, 1);
    live.phase = AuthorityPhase::Running;
    push_evolved_snake(&mut live, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
    let snake = &mut live.world.snakes[0];
    snake.id = EXTERNAL_ENTITY_ID_START;
    snake.kind = SnakeKind::External;
    snake.population_slot = None;
    snake.brain = None;
    live.allocators.next_entity_id = 1;
    live.allocators.next_external_id = EXTERNAL_ENTITY_ID_START + 1;
    live.world.controller_leases.push(ControllerLease {
        reward: None,
        identity_key: String::new(),
        id: 1,
        snake_id: EXTERNAL_ENTITY_ID_START,
        kind: ControllerKind::ReinforcementLearning,
        connection_id: None,
        scope: "run-test".into(),
        resume_token: "rl-token".into(),
        status: ControllerLeaseStatus::NeuralTakeover,
        latest_action: LatestControllerAction {
            is_explicit: false,
            turn: 0.0,
            boost: false,
            client_tick: 0,
            arrival_sequence: 1,
            accepted_at_ms: 100,
        },
        last_observed_at_ms: 31_000,
        disconnected_at_ms: Some(1_000),
        input_hold_expires_at_ms: Some(600),
        grace_expires_at_ms: Some(31_000),
        takeover_committed_at_ms: Some(31_000),
    });
    live.allocators.next_controller_lease_id = 2;
    assert!(matches!(
        own(live, graph, usize::MAX),
        Err(StateError::InvalidField {
            field: "controller_lease",
            ..
        })
    ));
}
