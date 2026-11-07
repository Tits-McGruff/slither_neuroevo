use super::*;

#[test]
fn inspector_probe_waits_for_publication_and_never_consumes_external_delivery() {
    use crate::engine::inspector::*;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let snake = &source.world.snakes[0];
    let unchanged = source.clone();
    let target = InspectorTarget {
        world_epoch: 1,
        generation: 1,
        physical_id: snake.id,
        frame_v1_id: snake.frame_v1_id,
    };
    let key = crate::engine::physics::PhysicsStepKey::new(1, 1, 5, 1, 1, [0; 32], 1);
    let layout =
        InspectorLayout::from(&crate::engine::sensor_layout::SensorLayout::new(8).unwrap());
    let values = vec![0.0; layout.input_size];
    let mut capture = InspectorCapture::new(target, MAX_INSPECTOR_BYTES).unwrap();
    capture.set_pure_probe(true);
    assert!(capture.needs_probe());
    assert!(capture.trace_for(snake.id).is_none());
    assert!(!capture.stage(
        key,
        snake,
        InspectorObservationSource::Neural,
        layout,
        &values,
        Some([0.0; 2]),
        None,
        Default::default()
    ));
    let external = InspectorObservationSource::External {
        lease_id: 3,
        connection_id: 4,
    };
    assert!(capture.stage_probe(key, snake, external, layout, &values, Default::default()));
    assert!(capture.latest(1, 1, 6).is_none());
    capture.discard_pending();
    capture.publish(key, |_, _, _| {
        panic!("probe must not consult an external receipt")
    });
    assert!(capture.latest(1, 1, 6).is_none());
    assert!(
        capture.needs_probe(),
        "failed whole step keeps the request pending"
    );
    assert!(capture.stage_probe(key, snake, external, layout, &values, Default::default()));
    capture.publish(key, |_, _, _| {
        panic!("probe must not consult an external receipt")
    });
    let row = capture.latest(1, 1, 6).unwrap();
    assert!(row.pure_probe && row.raw.first_delivery);
    assert!(row.neural_outputs.is_none());
    assert!(!capture.needs_probe());
    assert!(!capture.stage_probe(key, snake, external, layout, &values, Default::default()));
    assert_eq!(
        source, unchanged,
        "probing never consumes a first/delivery flag, RNG or quota cursor"
    );
}

#[test]
fn external_delivery_acceptance_publishes_once_and_stale_results_are_ignored() {
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
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("external fixture must construct the coordinator");

    let event = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("complete external step must await local Node acceptance")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => {
            assert_eq!(batch.events().len(), 1);
            assert_eq!(batch.observation(0).unwrap().len(), 83);
            assert_eq!(batch.remaining(), 1);
            assert_eq!(batch.is_accepted(0), Some(false));
            batch.events()[0]
        }
        RunningStepProgress::Published(_) => {
            panic!("external score marker must not publish before Node acceptance")
        }
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("external observation fixture must remain nonterminal")
        }
    };
    assert_eq!(authority.state(), &source);
    assert_eq!(coordinator.last_wall_now_ms(), Some(100));
    assert!(matches!(
        coordinator.advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 101,
                wall_accumulator_seconds: 0.0,
            },
        ),
        Err(RunningStepError::ExternalDeliveryPending { count: 1 })
    ));
    assert_eq!(authority.state(), &source);

    let stale = ExternalDeliveryResult {
        step_key: event.step_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: event.lease_id + 1,
        accepted: true,
    };
    let replaced_connection = ExternalDeliveryResult {
        lease_id: event.lease_id,
        connection_id: event.connection_id + 1,
        ..stale
    };
    let stale_resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[stale, replaced_connection])
        .expect("stale assignment result must be ignored");
    assert_eq!(stale_resolution.matched_acceptances, 0);
    assert_eq!(stale_resolution.ignored_results, 2);
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
        .expect("matching accepted result must publish the complete step");
    let outcome = match resolution.state {
        ExternalDeliveryState::Published(outcome) => outcome,
        _ => panic!("the only matching event must complete publication"),
    };
    assert_eq!(resolution.matched_acceptances, 1);
    assert_eq!(resolution.ignored_results, 0);
    assert_eq!(outcome.publication.completed_step, 1);
    let published_external = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == external_id)
        .unwrap();
    assert_eq!(published_external.delivered_observation_points, 10.01);
    assert!(published_external.observation_delivered);

    let published = authority.state().clone();
    let duplicate = coordinator
        .submit_external_delivery_results(&mut authority, &[accepted])
        .expect("duplicate accepted result after publication must be ignored");
    assert_eq!(duplicate.matched_acceptances, 0);
    assert_eq!(duplicate.ignored_results, 1);
    assert!(matches!(duplicate.state, ExternalDeliveryState::Idle));
    assert_eq!(authority.state(), &published);
}

#[test]
fn failed_external_delivery_publishes_the_step_with_a_disconnected_controller() {
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
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("external fixture must construct the coordinator");
    let inputs = RunningStepInputs {
        wall_now_ms: 100,
        wall_accumulator_seconds: 0.0,
    };

    let first_event = match coordinator
        .advance_nonterminal(&mut authority, inputs)
        .expect("first attempt must stage")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
        RunningStepProgress::Published(_) => panic!("external event must defer publication"),
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("external send-failure fixture must remain nonterminal")
        }
    };
    let rejected = ExternalDeliveryResult {
        step_key: first_event.step_key,
        event_sequence: first_event.event_sequence,
        connection_id: first_event.connection_id,
        lease_id: first_event.lease_id,
        accepted: false,
    };
    let resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[rejected])
        .expect("matching failed send must publish the prevalidated disconnect");
    let outcome = match resolution.state {
        ExternalDeliveryState::Published(outcome) => outcome,
        _ => panic!("the failed send must resolve the only pending event"),
    };
    assert_eq!(resolution.matched_acceptances, 0);
    assert_eq!(resolution.matched_failures, 1);
    assert_eq!(resolution.ignored_results, 0);
    assert_eq!(outcome.publication.completed_step, 1);
    assert_eq!(outcome.diagnostics.external_deliveries_pending, 0);
    assert_eq!(authority.state().generation.completed_step, 1);
    assert_eq!(
        authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == external_id)
            .unwrap()
            .delivered_observation_points,
        2.0
    );
    let lease = authority
        .state()
        .world
        .controller_leases
        .iter()
        .find(|lease| lease.id == first_event.lease_id)
        .unwrap();
    assert_eq!(lease.connection_id, None);
    assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
    assert_eq!(lease.disconnected_at_ms, Some(inputs.wall_now_ms));
    assert_eq!(lease.input_hold_expires_at_ms, Some(600));
    assert_eq!(lease.grace_expires_at_ms, Some(30_100));

    let stale_acceptance = ExternalDeliveryResult {
        accepted: true,
        ..rejected
    };
    let stale = coordinator
        .submit_external_delivery_results(&mut authority, &[stale_acceptance])
        .expect("result after publication must be ignored");
    assert_eq!(stale.matched_acceptances, 0);
    assert_eq!(stale.matched_failures, 0);
    assert_eq!(stale.ignored_results, 1);
    assert!(matches!(stale.state, ExternalDeliveryState::Idle));

    assert!(matches!(
        coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 101,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("disconnected grace must no longer emit to the failed socket"),
        RunningStepProgress::Published(_)
    ));
    assert_eq!(authority.state().generation.completed_step, 2);
}

#[test]
fn multiple_external_observations_publish_only_after_every_matching_acceptance() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let first_id = EXTERNAL_ENTITY_ID_START;
    let second_id = EXTERNAL_ENTITY_ID_START + 1;
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        first_id,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        second_id,
        3,
        2,
        8,
        WorldPoint {
            x: -1_000.0,
            y: 1_000.0,
        },
    );
    candidate.world.controller_leases.last_mut().unwrap().kind =
        ControllerKind::ReinforcementLearning;
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("two-controller fixture must construct the coordinator");
    enable_inspector_fixture(&mut coordinator, &authority, first_id);
    let mut observed_row = Vec::new();

    let events = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("complete step must stage both observations")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => {
            assert_eq!(batch.events().len(), 2);
            assert_eq!(batch.remaining(), 2);
            observed_row.extend_from_slice(batch.observation(0).unwrap());
            [batch.events()[0], batch.events()[1]]
        }
        RunningStepProgress::Published(_) => panic!("both events must require acceptance"),
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("two-controller fixture must remain nonterminal")
        }
    };
    assert!(events[0].snake_id < events[1].snake_id);
    assert_eq!(events[0].controller_kind, ControllerKind::Player);
    assert_eq!(
        events[1].controller_kind,
        ControllerKind::ReinforcementLearning
    );

    let first_result = ExternalDeliveryResult {
        step_key: events[0].step_key,
        event_sequence: events[0].event_sequence,
        connection_id: events[0].connection_id,
        lease_id: events[0].lease_id,
        accepted: true,
    };
    let first = coordinator
        .submit_external_delivery_results(&mut authority, &[first_result])
        .expect("first exact result must be retained without publication");
    assert_eq!(first.matched_acceptances, 1);
    assert!(matches!(
        first.state,
        ExternalDeliveryState::Pending(batch)
            if batch.remaining() == 1 && batch.is_accepted(0) == Some(true)
    ));
    assert_eq!(authority.state(), &source);
    assert!(coordinator.inspector_observation(&authority).is_none());

    let duplicate = coordinator
        .submit_external_delivery_results(&mut authority, &[first_result])
        .expect("duplicate acceptance must be ignored");
    assert_eq!(duplicate.matched_acceptances, 0);
    assert_eq!(duplicate.ignored_results, 1);
    assert!(matches!(
        duplicate.state,
        ExternalDeliveryState::Pending(batch) if batch.remaining() == 1
    ));
    assert_eq!(authority.state(), &source);
    assert!(coordinator.inspector_observation(&authority).is_none());

    let duplicate_failure = ExternalDeliveryResult {
        accepted: false,
        ..first_result
    };
    let duplicate_failure_resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[duplicate_failure])
        .expect("a negative duplicate cannot override prior acceptance");
    assert_eq!(duplicate_failure_resolution.matched_acceptances, 0);
    assert_eq!(duplicate_failure_resolution.matched_failures, 0);
    assert_eq!(duplicate_failure_resolution.ignored_results, 1);
    assert!(matches!(
        duplicate_failure_resolution.state,
        ExternalDeliveryState::Pending(batch)
            if batch.remaining() == 1
                && batch.status(0) == Some(ExternalDeliveryStatus::Accepted)
    ));
    assert_eq!(authority.state(), &source);
    assert!(coordinator.inspector_observation(&authority).is_none());

    let second_result = ExternalDeliveryResult {
        step_key: events[1].step_key,
        event_sequence: events[1].event_sequence,
        connection_id: events[1].connection_id,
        lease_id: events[1].lease_id,
        accepted: true,
    };
    let completed = coordinator
        .submit_external_delivery_results(&mut authority, &[second_result])
        .expect("last exact result must publish once");
    let completed_outcome = match completed.state {
        ExternalDeliveryState::Published(outcome) => outcome,
        _ => panic!("last exact acceptance must publish"),
    };
    assert_eq!(completed.matched_failures, 0);
    assert_eq!(completed_outcome.diagnostics.external_deliveries_pending, 0);
    for snake_id in [first_id, second_id] {
        let snake = authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == snake_id)
            .unwrap();
        assert_eq!(snake.delivered_observation_points, 10.01);
    }
    assert_eq!(authority.state().generation.completed_step, 1);
    let observed = coordinator.inspector_observation(&authority).unwrap();
    assert_eq!(observed.normalized_inputs(), observed_row);
    assert_eq!(observed.age_steps(1), Some(0));
    assert_eq!(observed.target.physical_id, first_id);
    assert!(matches!(
        observed.source,
        crate::engine::inspector::InspectorObservationSource::External {
            lease_id: 1,
            connection_id: 7
        }
    ));
}

#[test]
fn mixed_external_results_advance_only_accepted_markers_and_disconnect_failures() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let accepted_id = EXTERNAL_ENTITY_ID_START;
    let failed_id = EXTERNAL_ENTITY_ID_START + 1;
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        accepted_id,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        failed_id,
        3,
        2,
        8,
        WorldPoint {
            x: -1_000.0,
            y: 1_000.0,
        },
    );
    candidate.world.controller_leases.last_mut().unwrap().kind =
        ControllerKind::ReinforcementLearning;
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("mixed-result fixture must construct the coordinator");
    enable_inspector_fixture(&mut coordinator, &authority, failed_id);
    let events = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("both external observations must stage")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => {
            [batch.events()[0], batch.events()[1]]
        }
        RunningStepProgress::Published(_) => panic!("both events require local results"),
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("mixed-result fixture must remain nonterminal")
        }
    };
    let accepted = ExternalDeliveryResult {
        step_key: events[0].step_key,
        event_sequence: events[0].event_sequence,
        connection_id: events[0].connection_id,
        lease_id: events[0].lease_id,
        accepted: true,
    };
    let failed = ExternalDeliveryResult {
        step_key: events[1].step_key,
        event_sequence: events[1].event_sequence,
        connection_id: events[1].connection_id,
        lease_id: events[1].lease_id,
        accepted: false,
    };
    let resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[failed, accepted])
        .expect("prevalidated mixed results must publish atomically");
    assert_eq!(resolution.matched_acceptances, 1);
    assert_eq!(resolution.matched_failures, 1);
    assert_eq!(resolution.ignored_results, 0);
    let outcome = match resolution.state {
        ExternalDeliveryState::Published(outcome) => outcome,
        _ => panic!("all resolved events must publish exactly once"),
    };
    assert_eq!(outcome.diagnostics.external_deliveries_pending, 0);

    let accepted_snake = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == accepted_id)
        .unwrap();
    let failed_snake = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == failed_id)
        .unwrap();
    assert_eq!(accepted_snake.delivered_observation_points, 10.01);
    assert!(accepted_snake.observation_delivered);
    assert!(!failed_snake.observation_delivered);
    assert_eq!(failed_snake.delivered_observation_points, 2.0);
    let accepted_lease = authority
        .state()
        .world
        .controller_leases
        .iter()
        .find(|lease| lease.snake_id == accepted_id)
        .unwrap();
    let failed_lease = authority
        .state()
        .world
        .controller_leases
        .iter()
        .find(|lease| lease.snake_id == failed_id)
        .unwrap();
    assert_eq!(accepted_lease.status, ControllerLeaseStatus::Connected);
    assert_eq!(accepted_lease.connection_id, Some(7));
    assert_eq!(failed_lease.status, ControllerLeaseStatus::HoldingLastInput);
    assert_eq!(failed_lease.connection_id, None);
    assert_eq!(authority.state().generation.completed_step, 1);
    assert!(
        coordinator.inspector_observation(&authority).is_none(),
        "failed local send must never become an inspector delivery"
    );
}

#[test]
fn repeated_external_delivery_reuses_every_reported_bridge_capacity() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("external fixture must construct the coordinator");
    let mut warmed_capacities = None;

    for boundary in 0..24u64 {
        let event = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100 + boundary,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("external boundary must stage")
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
            RunningStepProgress::Published(_) => {
                panic!("external boundary must await local acceptance")
            }
            RunningStepProgress::GenerationTransitionPending(_) => {
                panic!("warmed external boundary must remain nonterminal")
            }
        };
        let pending = coordinator.external_delivery_diagnostics();
        assert_eq!(pending.pending_events, 1);
        assert_eq!(pending.remaining_events, 1);
        let capacities = (
            pending.event_capacity,
            pending.acceptance_capacity,
            pending.disconnect_capacity,
            pending.observation_capacity,
        );
        if let Some(expected) = warmed_capacities {
            assert_eq!(capacities, expected);
        } else {
            warmed_capacities = Some(capacities);
        }

        let accepted = ExternalDeliveryResult {
            step_key: event.step_key,
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            accepted: true,
        };
        assert!(matches!(
            coordinator
                .submit_external_delivery_results(&mut authority, &[accepted])
                .expect("matching result must publish")
                .state,
            ExternalDeliveryState::Published(_)
        ));
        let idle = coordinator.external_delivery_diagnostics();
        assert_eq!(idle.pending_events, 0);
        assert_eq!(idle.remaining_events, 0);
        assert_eq!(
            (
                idle.event_capacity,
                idle.acceptance_capacity,
                idle.disconnect_capacity,
                idle.observation_capacity,
            ),
            warmed_capacities.unwrap()
        );
    }
    assert_eq!(authority.state().generation.completed_step, 24);
}

#[test]
fn superseded_operation_ignores_its_old_external_result_without_publication() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("external fixture must construct the coordinator");
    let event = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("external step must stage")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
        RunningStepProgress::Published(_) => panic!("external event must defer publication"),
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("stale-result fixture must remain nonterminal")
        }
    };
    authority
        .begin_running_step()
        .expect("a newer operation must supersede the retained proposal");
    let result = ExternalDeliveryResult {
        step_key: event.step_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: event.lease_id,
        accepted: true,
    };
    let resolution = coordinator
        .submit_external_delivery_results(&mut authority, &[result])
        .expect("superseded result must be ignored");
    assert_eq!(resolution.matched_acceptances, 0);
    assert_eq!(resolution.ignored_results, 1);
    assert!(matches!(resolution.state, ExternalDeliveryState::Idle));
    assert_eq!(authority.state(), &source);
}

#[test]
fn terminal_or_failed_admission_never_exposes_an_external_delivery_batch() {
    let graph = default_graph();

    let mut terminal = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut terminal,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    set_complete_setting(
        &mut terminal,
        "generationSeconds",
        NormalizedSettingValue::Float(8.0),
    );
    set_complete_setting(
        &mut terminal,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    );
    terminal.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
    refresh_config_hash(&mut terminal);
    let mut terminal_authority = own_complete_running(terminal, Arc::clone(&graph));
    let terminal_source = terminal_authority.state().clone();
    let mut terminal_coordinator = RunningStepCoordinator::try_new(
        &terminal_authority,
        RunningStepWorkLimits::provisional_defaults(),
    )
    .expect("terminal fixture must construct the coordinator");
    let transition = match terminal_coordinator
        .advance_nonterminal(
            &mut terminal_authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("terminal attempt must stage one generation transition")
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected pending generation transition, got {other:?}"),
    };
    assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
    assert_eq!(terminal_authority.state(), &terminal_source);
    assert!(matches!(
        terminal_coordinator
            .submit_external_delivery_results(&mut terminal_authority, &[])
            .expect("terminal attempt must retain no bridge batch")
            .state,
        ExternalDeliveryState::Idle
    ));

    let mut inadmissible = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut inadmissible,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint {
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    let mut inadmissible_authority = own_complete_running(inadmissible, Arc::clone(&graph));
    let inadmissible_source = inadmissible_authority.state().clone();
    let mut inadmissible_coordinator = RunningStepCoordinator::try_new(
        &inadmissible_authority,
        RunningStepWorkLimits::provisional_defaults(),
    )
    .expect("memory fixture must construct the coordinator");
    inadmissible_authority.memory_ceiling_bytes = 1;
    assert!(matches!(
        inadmissible_coordinator.advance_nonterminal(
            &mut inadmissible_authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        ),
        Err(RunningStepError::State(error))
            if matches!(*error, StateError::MemoryCeilingExceeded { .. })
    ));
    assert_eq!(inadmissible_authority.state(), &inadmissible_source);
    assert!(matches!(
        inadmissible_coordinator
            .submit_external_delivery_results(&mut inadmissible_authority, &[])
            .expect("failed admission must expose no bridge batch")
            .state,
        ExternalDeliveryState::Idle
    ));
}
