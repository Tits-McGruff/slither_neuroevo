use super::*;

#[test]
fn hall_of_fame_resurrection_adds_one_owned_brain_and_preserves_population() {
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let before = authority.state().clone();
    let weights = vec![0.375; graph.compiled().total_parameters].into_boxed_slice();
    let publication = authority
        .resurrect_hall_of_fame(
            7,
            weights.clone(),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("retained winner should resurrect");
    let after = authority.state();
    assert_eq!(after.population, before.population);
    assert_eq!(after.world.snakes.len(), before.world.snakes.len() + 1);
    assert_eq!(after.brains.len(), before.brains.len() + 1);
    let snake = after.world.snakes.last().expect("resurrected snake");
    assert_eq!(snake.kind, SnakeKind::Resurrected);
    assert_eq!(snake.id, publication.snake_id);
    assert_eq!(snake.frame_v1_id, publication.frame_v1_id);
    let brain = after.brains.last().expect("resurrected brain");
    assert_eq!(brain.owner, BrainOwner::Entity(publication.snake_id));
    assert_eq!(
        brain.non_population_weights.as_deref(),
        Some(weights.as_ref())
    );
    assert!(brain.recurrent.iter().all(|value| *value == 0.0));
    let committed = after.clone();
    assert!(authority
        .resurrect_hall_of_fame(
            7,
            vec![0.0; graph.compiled().total_parameters - 1].into_boxed_slice(),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .is_err());
    assert_eq!(authority.state(), &committed);
    authority.memory_ceiling_bytes = 0;
    assert!(authority
        .resurrect_hall_of_fame(7, weights, RunningStepWorkLimits::provisional_defaults())
        .is_err());
    assert_eq!(authority.state(), &committed);
}

#[test]
fn inspector_cache_is_publication_gated_bounded_and_keeps_last_delivered_age() {
    use crate::engine::inspector::*;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let unchanged = source.clone();
    let snake = source
        .world
        .snakes
        .iter()
        .find(|snake| snake.alive)
        .unwrap();
    let target = InspectorTarget {
        world_epoch: 1,
        generation: 1,
        physical_id: snake.id,
        frame_v1_id: snake.frame_v1_id,
    };
    assert!(InspectorCapture::new(target, 0).is_err());
    assert!(InspectorCapture::new(target, MAX_INSPECTOR_BYTES + 1).is_err());
    let mut capture = InspectorCapture::new(target, MAX_INSPECTOR_BYTES).unwrap();
    let key = crate::engine::physics::PhysicsStepKey::new(1, 1, 5, 1, 1, [0; 32], 1);
    let layout =
        InspectorLayout::from(&crate::engine::sensor_layout::SensorLayout::new(16).unwrap());
    let row = vec![0.25; layout.input_size];
    assert!(capture.stage(
        key,
        snake,
        InspectorObservationSource::Neural,
        layout,
        &row,
        Some([0.5, -0.5]),
        None,
        Default::default()
    ));
    assert!(capture.latest(1, 1, 6).is_none());
    let wrong = crate::engine::physics::PhysicsStepKey::new(1, 1, 5, 1, 1, [0; 32], 2);
    capture.publish(wrong, |_, _, _| true);
    assert!(capture.latest(1, 1, 6).is_none());
    assert!(capture.stage(
        key,
        snake,
        InspectorObservationSource::Neural,
        layout,
        &row,
        Some([0.5, -0.5]),
        None,
        Default::default()
    ));
    capture.publish(key, |_, _, _| {
        panic!("internal consumption never requires an external receipt")
    });
    let delivered = capture.latest(1, 1, 6).unwrap();
    assert_eq!(delivered.normalized_inputs(), row);
    assert_eq!(delivered.age_steps(6), Some(0));
    assert_eq!(delivered.age_steps(9), Some(3));
    assert!(capture.latest(2, 1, 9).is_none());
    assert!(capture.latest(1, 2, 9).is_none());
    assert!(capture.latest(1, 1, 5).is_none());
    assert!(capture.stage(
        key,
        snake,
        InspectorObservationSource::External {
            lease_id: 3,
            connection_id: 4
        },
        layout,
        &row,
        None,
        None,
        Default::default()
    ));
    capture.publish(key, |id, lease, connection| {
        assert_eq!((id, lease, connection), (snake.id, 3, 4));
        false
    });
    assert_eq!(
        capture.latest(1, 1, 9).unwrap().source,
        InspectorObservationSource::Neural
    );
    let mut invalid = row.clone();
    invalid[0] = f32::NAN;
    assert!(!capture.stage(
        key,
        snake,
        InspectorObservationSource::Neural,
        layout,
        &invalid,
        None,
        None,
        Default::default()
    ));
    assert!(capture.stage(
        key,
        snake,
        InspectorObservationSource::External {
            lease_id: 3,
            connection_id: 4
        },
        layout,
        &row,
        None,
        None,
        Default::default()
    ));
    capture.discard_pending();
    capture.publish(key, |_, _, _| true);
    assert_eq!(
        capture.latest(1, 1, 9).unwrap().source,
        InspectorObservationSource::Neural
    );
    assert_eq!(
        source, unchanged,
        "debug caching cannot consume delivery, RNG, first flag or quota cursor"
    );
}

#[test]
fn inspector_does_not_raise_the_existing_worker_scratch_admission() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let limits = RunningStepWorkLimits::provisional_defaults();
    let plan = crate::engine::inference::GraphExecutionPlan::build(&graph).unwrap();
    candidate.config.worker_scratch_bytes =
        crate::engine::control::NeuralControlPipeline::required_staging_bytes(
            candidate.config.max_world_snakes,
            &plan,
            limits.calculation_workers,
        )
        .unwrap();
    refresh_config_hash(&mut candidate);
    let authority = own_complete_running(candidate, graph);
    let before = authority.state().clone();
    let mut coordinator = RunningStepCoordinator::try_new(&authority, limits).unwrap();
    let snake = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.alive)
        .unwrap();
    let error = coordinator
        .configure_inspector(
            &authority,
            Some(crate::engine::inspector::InspectorTarget {
                world_epoch: authority.world_epoch(),
                generation: authority.state().generation.generation,
                physical_id: snake.id,
                frame_v1_id: snake.frame_v1_id,
            }),
            crate::engine::inspector::MAX_INSPECTOR_BYTES,
        )
        .unwrap_err();
    assert!(error.to_string().contains("worker scratch admission"));
    assert!(coordinator.inspector_observation(&authority).is_none());
    assert_eq!(authority.state(), &before);
}

#[test]
fn selected_retained_origins_are_native_owned_fixed_width_and_do_not_advance_rng() {
    use crate::engine::brain_artifact::BrainSourceKind;
    use crate::engine::retained_origin::packed_weight_digest;
    let graph = default_graph();
    let mut authority =
        own_complete_running(complete_running_candidate(&graph), Arc::clone(&graph));
    let population = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.alive && snake.kind == SnakeKind::Evolved)
        .unwrap();
    assert!(authority
        .select_entity(EntitySelectionTarget::Frame(population.frame_v1_id))
        .unwrap()
        .retained_origin
        .is_none());
    let weights = vec![0.25; graph.compiled().total_parameters].into_boxed_slice();
    let digest = packed_weight_digest(&weights);
    let saved = RetainedNeuralOrigin::SavedBrain {
        brain_id: [13; 32],
        weight_digest: digest,
        source_kind: BrainSourceKind::Distilled,
    };
    let origin = Arc::new(saved);
    authority
        .spawn_retained_brain(
            weights.clone(),
            Arc::clone(&origin),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
    let frame = authority.state().world.snakes.last().unwrap().frame_v1_id;
    let before = authority.state().clone();
    let references = Arc::strong_count(&origin);
    let selected = authority
        .select_entity(EntitySelectionTarget::Frame(frame))
        .unwrap();
    assert_eq!(selected.retained_origin, Some(saved));
    assert_eq!(Arc::strong_count(&origin), references);
    assert_eq!(authority.state(), &before);
    drop(before);
    drop(origin);
    assert_eq!(
        authority
            .select_entity(EntitySelectionTarget::Frame(frame))
            .unwrap()
            .retained_origin,
        Some(saved)
    );
    authority
        .resurrect_hall_of_fame(
            u64::MAX,
            weights,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
    let frame = authority.state().world.snakes.last().unwrap().frame_v1_id;
    assert_eq!(
        authority
            .select_entity(EntitySelectionTarget::Frame(frame))
            .unwrap()
            .retained_origin,
        Some(RetainedNeuralOrigin::HallOfFame {
            entry_id: u64::MAX,
            weight_digest: digest
        })
    );
}

#[test]
fn retained_origin_admission_and_failed_publication_preserve_native_ownership() {
    use crate::engine::retained_origin::packed_weight_digest;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let weights = vec![0.375; graph.compiled().total_parameters].into_boxed_slice();
    authority
        .resurrect_hall_of_fame(
            7,
            weights.clone(),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
    let committed = authority.state().clone();
    let origin = Arc::clone(
        committed
            .brains
            .last()
            .unwrap()
            .retained_origin
            .as_ref()
            .unwrap(),
    );
    let mut missing = committed.clone();
    missing.brains.last_mut().unwrap().retained_origin = None;
    assert!(own(missing, Arc::clone(&graph), usize::MAX).is_err());
    let mut wrong_weights = committed.clone();
    wrong_weights
        .brains
        .last_mut()
        .unwrap()
        .non_population_weights
        .as_mut()
        .unwrap()[0] = 0.5;
    assert!(own(wrong_weights, Arc::clone(&graph), usize::MAX).is_err());
    let mut wrong_population = committed.clone();
    wrong_population.brains[0].retained_origin = Some(Arc::clone(&origin));
    assert!(own(wrong_population, Arc::clone(&graph), usize::MAX).is_err());
    let before_count = Arc::strong_count(&origin);
    authority.memory_ceiling_bytes = 0;
    assert!(authority
        .spawn_retained_brain(
            weights.clone(),
            Arc::clone(&origin),
            RunningStepWorkLimits::provisional_defaults()
        )
        .is_err());
    assert_eq!(authority.state(), &committed);
    assert_eq!(Arc::strong_count(&origin), before_count);
    authority.memory_ceiling_bytes = usize::MAX;
    let key = authority.begin_running_step().unwrap();
    let mut changed = RunningStepBuffers::from_state(authority.state());
    changed.brains.last_mut().unwrap().retained_origin =
        Some(Arc::new(RetainedNeuralOrigin::HallOfFame {
            entry_id: 8,
            weight_digest: packed_weight_digest(&weights),
        }));
    assert!(matches!(
        authority.publish_running_step(changed.replacement(key)),
        Err(StateError::InvalidField {
            field: "fixed_step.brains",
            ..
        })
    ));
    assert_eq!(authority.state().brains, committed.brains);
}

#[test]
fn retained_reuse_failure_restores_body_owners_brain_origin_rng_and_allocators() {
    let (mut authority, graph) = retained_reuse_fixture();
    let before = authority.state().clone();
    let ceiling = authority.memory_ceiling_bytes;
    authority.memory_ceiling_bytes = 0;
    assert!(authority
        .resurrect_hall_of_fame(
            3,
            vec![0.5; graph.compiled().total_parameters].into_boxed_slice(),
            RunningStepWorkLimits::provisional_defaults()
        )
        .is_err());
    assert_eq!(authority.state(), &before);
    for (current, previous) in authority.state().brains.iter().zip(&before.brains) {
        if let (Some(current), Some(previous)) =
            (&current.retained_origin, &previous.retained_origin)
        {
            assert!(Arc::ptr_eq(current, previous));
        }
    }
    authority.memory_ceiling_bytes = ceiling;
    authority.candidate.config.checkpoint_scratch_bytes = 1;
    let scratch_limited = authority.state().clone();
    let error = authority
        .resurrect_hall_of_fame(
            3,
            vec![0.5; graph.compiled().total_parameters].into_boxed_slice(),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap_err();
    assert!(error.contains("cold scratch"), "{error}");
    assert_eq!(authority.state(), &scratch_limited);
}

#[test]
fn retained_source_graph_capacity_is_charged_before_state_publication() {
    let compact_graph = default_graph();
    let compact_candidate = candidate(&compact_graph, 1);
    let compact_estimate = estimate_state_memory(&compact_candidate, &compact_graph).unwrap();

    let mut expanded_source = default_graph_spec();
    expanded_source.nodes.reserve(256);
    expanded_source.edges.reserve(512);
    expanded_source.outputs.reserve(128);
    expanded_source.nodes[1].id.reserve(16_384);
    expanded_source.edges[0].from.reserve(8_192);
    if let GraphNodeKind::Mlp { hidden_sizes, .. } = &mut expanded_source.nodes[1].kind {
        hidden_sizes.reserve(2_048);
    } else {
        panic!("default fixture's second node must be an MLP");
    }
    let expanded_graph = Arc::new(
        GraphBundle::compile(expanded_source, &default_graph_limits())
            .expect("expanded source graph must remain valid"),
    );
    assert_eq!(expanded_graph.compiled(), compact_graph.compiled());
    let expanded_candidate = candidate(&expanded_graph, 1);
    let expanded_estimate = estimate_state_memory(&expanded_candidate, &expanded_graph).unwrap();

    assert!(expanded_estimate.graph_bytes > compact_estimate.graph_bytes);
    assert!(matches!(
        own(
            expanded_candidate,
            expanded_graph,
            compact_estimate.total_bytes
        ),
        Err(StateError::MemoryCeilingExceeded { .. })
    ));
}

#[test]
fn live_fixed_step_continuation_survives_ownership_and_charges_retained_capacity() {
    let graph = default_graph();
    let mut live = candidate(&graph, 1);
    live.phase = AuthorityPhase::Running;
    push_evolved_snake(&mut live, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
    live.world.snakes[0].points = 12.5;
    assert!(matches!(
        own(live.clone(), Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "fixed_step.sensor_generation",
            ..
        })
    ));

    let baseline_position = WorldPoint { x: 8.0, y: 9.0 };
    let body_start = live.world.body_points.len();
    live.world.body_points.push(baseline_position);
    live.world.snakes.push(SnakeState {
        id: BASELINE_ENTITY_ID_START,
        frame_v1_id: 2,
        kind: SnakeKind::Baseline,
        alive: true,
        population_slot: None,
        brain: None,
        baseline_slot: Some(0),
        baseline_strategy: Some(BaselineStrategyState::Roam),
        position: baseline_position,
        previous_position: baseline_position,
        direction: 0.0,
        radius: 8.0,
        speed: 100.0,
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
        body: BodyRange {
            start: body_start,
            len: 1,
        },
        skin: 0,
    });
    live.allocators.next_baseline_id = BASELINE_ENTITY_ID_START + 1;
    live.allocators.next_frame_v1_id = 3;
    enable_one_live_baseline(&mut live, BASELINE_ENTITY_ID_START, None);
    live.fixed_step.ambient_pellet_accumulator = 0.75;
    live.fixed_step
        .sensor_generation
        .update_after_step(&live.world)
        .expect("finite evolved score should update generation best");

    let expected_continuation = live.fixed_step.clone();
    let compact_estimate = estimate_state_memory(&live, &graph).unwrap();
    let owned = own(
        live.clone(),
        Arc::clone(&graph),
        compact_estimate.total_bytes,
    )
    .expect("complete live continuation should be admitted");
    assert_eq!(owned.state().fixed_step, expected_continuation);
    assert_eq!(
        owned
            .state()
            .fixed_step
            .sensor_generation
            .best_points_this_generation(),
        12.5
    );

    let mut invalid_strategy_timer = live.clone();
    invalid_strategy_timer.fixed_step.baseline_lifecycle.slots[0].strategy_timer_seconds = 0.1;
    assert!(matches!(
        own(invalid_strategy_timer, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            ..
        })
    ));

    let mut invalid_wander = live.clone();
    invalid_wander.fixed_step.baseline_lifecycle.slots[0].wander_angle = 0.31;
    assert!(matches!(
        own(invalid_wander, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            ..
        })
    ));

    let mut mismatched_turn = live.clone();
    mismatched_turn.fixed_step.baseline_lifecycle.slots[0].turn = 0.25;
    assert!(matches!(
        own(mismatched_turn, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            ..
        })
    ));

    let mut mismatched_boost = live.clone();
    mismatched_boost.fixed_step.baseline_lifecycle.slots[0].boost = true;
    assert!(matches!(
        own(mismatched_boost, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            ..
        })
    ));

    let mut dead_action_divergence = live.clone();
    let baseline_index = dead_action_divergence
        .world
        .snakes
        .iter()
        .position(|snake| snake.id == BASELINE_ENTITY_ID_START)
        .expect("test baseline must exist");
    dead_action_divergence.world.snakes[baseline_index].alive = false;
    dead_action_divergence.world.snakes[baseline_index].turn = 0.5;
    dead_action_divergence.world.snakes[baseline_index].input_boost = true;
    dead_action_divergence.fixed_step.baseline_lifecycle.slots[0].respawn_remaining_seconds =
        Some(3.0);
    assert!(own(dead_action_divergence, Arc::clone(&graph), usize::MAX,).is_ok());

    let mut over_reserved = live;
    over_reserved
        .fixed_step
        .baseline_lifecycle
        .slots
        .reserve_exact(32);
    let reserved_estimate = estimate_state_memory(&over_reserved, &graph).unwrap();
    assert!(reserved_estimate.structural_bytes > compact_estimate.structural_bytes);
    assert!(matches!(
        own(over_reserved, graph, compact_estimate.total_bytes),
        Err(StateError::MemoryCeilingExceeded { .. })
    ));
}

#[test]
fn entity_selection_rejects_restarted_worlds_and_reused_frame_ids_without_rng_draws() {
    let graph = default_graph();
    let mut source = respawn_candidate(&graph, false, 0.0);
    source.world.snakes[0].alive = true;
    source.fixed_step.evolved_generation =
        crate::engine::evolved_lives::EvolvedGenerationState::default();
    source.allocators.next_frame_v1_id = FRAME_V1_MAX_EXACT_ID;
    set_complete_setting(
        &mut source,
        "evolvedRespawn.corpseMode",
        NormalizedSettingValue::Integer(1),
    );
    refresh_config_hash(&mut source);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let original = authority.state().clone();
    let frame = original.world.snakes[0].frame_v1_id;
    let selected = authority
        .select_entity(EntitySelectionTarget::Frame(frame))
        .unwrap();
    assert_eq!(selected.life_ordinal, Some(1));
    assert_eq!(
        authority.select_entity(EntitySelectionTarget::PopulationSlot(0)),
        Some(selected)
    );
    authority
        .validate_entity_selection(selected.token, frame)
        .unwrap();
    assert!(authority
        .validate_entity_selection(selected.token, frame + 1)
        .is_err());
    assert_eq!(authority.state(), &original);
    let restarted = own_complete_running(original.clone(), Arc::clone(&graph));
    assert_eq!(restarted.state(), &original);
    assert!(restarted
        .validate_entity_selection(selected.token, frame)
        .is_err());
    assert_ne!(
        restarted
            .select_entity(EntitySelectionTarget::Frame(frame))
            .unwrap()
            .token,
        selected.token
    );
    let physics = authority
        .running_step_config(RunningStepWorkLimits::provisional_defaults())
        .unwrap()
        .world_step
        .physics;
    authority.apply_god_mode_kill(frame, physics).unwrap();
    assert!(authority
        .select_entity(EntitySelectionTarget::PopulationSlot(0))
        .is_none());
    assert!(authority
        .validate_entity_selection(selected.token, frame)
        .is_err());
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
    assert_eq!(
        authority.state().world.snakes[0].frame_v1_id,
        FRAME_V1_MAX_EXACT_ID
    );
    authority
        .apply_god_mode_kill(FRAME_V1_MAX_EXACT_ID, physics)
        .unwrap();
    assert!(matches!(
        coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 200,
                    wall_accumulator_seconds: 0.0
                }
            )
            .unwrap(),
        RunningStepProgress::Published(_)
    ));
    assert_eq!(authority.state().world.snakes[0].frame_v1_id, frame);
    assert_ne!(
        authority.state().world.snakes[0].id,
        original.world.snakes[0].id
    );
    let after = authority.state().clone();
    assert!(authority
        .validate_entity_selection(selected.token, frame)
        .is_err());
    let replacement = authority
        .select_entity(EntitySelectionTarget::PopulationSlot(0))
        .unwrap();
    assert_eq!(replacement.life_ordinal, Some(3));
    assert_ne!(replacement.token, selected.token);
    authority
        .validate_entity_selection(replacement.token, frame)
        .unwrap();
    assert_eq!(authority.state(), &after);
}
