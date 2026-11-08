use super::*;

#[test]
fn whole_step_motion_proprioception_uses_all_substeps_and_held_actions_across_wrap() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    set_complete_setting(
        &mut candidate,
        "brain.controlDt",
        NormalizedSettingValue::Float(0.06),
    );
    candidate.world.snakes[0].control_accumulator_seconds = 0.0;
    candidate.world.snakes[0].direction = std::f64::consts::PI - 0.01;
    candidate.world.snakes[0].turn = 0.75;
    candidate.world.snakes[0].speed = 150.0;
    candidate.world.snakes[0].observation_delivered = true;
    refresh_config_hash(&mut candidate);
    let projection = crate::engine::step_config::project_running_step_config(
        &candidate.config,
        RunningStepWorkLimits::provisional_defaults(),
    )
    .unwrap();
    assert!(projection.world_step.physics_substeps > 1);
    let mut authority = own_complete_running(candidate, graph);
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    for step in 1..=2 {
        let before = authority.state().world.snakes[0].clone();
        assert!(matches!(
            coordinator
                .advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: step * 100,
                        wall_accumulator_seconds: 0.0
                    }
                )
                .unwrap(),
            RunningStepProgress::Published(_)
        ));
        let after = &authority.state().world.snakes[0];
        assert!(after.alive);
        assert_eq!(after.completed_step_applied_turn, 0.75);
        assert!((after.completed_step_heading_delta - 0.75 * 3.2 / 60.0).abs() < 1e-12);
        assert_eq!(after.completed_step_speed_delta, after.speed - before.speed);
        assert!(after.completed_step_speed_delta > 0.0);
        if step == 1 {
            assert!(before.direction > 0.0 && after.direction < 0.0);
        }
    }
}

#[test]
fn evolved_replacement_reopens_first_delivery_under_both_recurrent_life_modes() {
    use crate::engine::fixed_step::{FixedStepPrefixInputs, FixedStepPrefixWorkspace};
    for preserve in [false, true] {
        let graph = default_graph();
        let mut candidate = respawn_candidate(&graph, preserve, 0.8);
        candidate.world.snakes[0].observation_delivered = true;
        let authority = own_complete_running(candidate, graph);
        let state = authority.state();
        let before = state.clone();
        let projection = crate::engine::step_config::project_running_step_config(
            &state.config,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        let mut lives = state.fixed_step.evolved_generation.clone();
        let mut workspace = FixedStepPrefixWorkspace::new();
        let prefix = workspace
            .prepare_with_evolved(
                FixedStepPrefixInputs {
                    key: authority.running_step_key(1).unwrap(),
                    world: &state.world,
                    rng: &state.rng,
                    allocators: &state.allocators,
                    generation_elapsed_seconds: state.generation.elapsed_seconds,
                    ambient_accumulator: state.fixed_step.ambient_pellet_accumulator,
                    baseline_lifecycle: &state.fixed_step.baseline_lifecycle,
                    config: projection.world_step.prefix,
                },
                &mut lives,
                projection.evolved,
                projection
                    .world_step
                    .control
                    .initial_neural_accumulator_seconds(),
            )
            .unwrap();
        let newborn = &prefix.world().snakes[0];
        assert!(newborn.alive);
        assert_ne!(newborn.id, state.world.snakes[0].id);
        assert_eq!(newborn.brain, state.world.snakes[0].brain);
        assert!(
            !newborn.observation_delivered,
            "new physical life must start unsampled even when recurrence is preserved"
        );
        assert_eq!(authority.state(), &before);
    }
}

#[test]
fn evolved_newborn_replaces_in_place_preserves_brain_owner_and_receives_one_award() {
    let graph = default_graph();
    let mut source = respawn_candidate(&graph, false, 0.8);
    let original_id = source.world.snakes[0].id;
    let original_frame = source.world.snakes[0].frame_v1_id;
    let original_brain = source.population[0].brain;
    let pellet_id = source.allocators.next_entity_id;
    source.allocators.next_entity_id += 1;
    source.world.pellets.push(PelletState {
        id: pellet_id,
        position: WorldPoint { x: 3300.0, y: 0.0 },
        value: 1.0,
        kind: 0,
        color: 123,
        owner: Some(original_id),
    });
    let genomes = source.population.clone();
    let (authority, diagnostics) = one_respawn_step(source, graph);
    let state = authority.state();
    let placement = state.fixed_step.evolved_generation.placement_work;
    assert_eq!(
        (
            placement.step,
            placement.eligible_slots,
            placement.attempted_slots,
            placement.placed_slots
        ),
        (state.generation.completed_step, 1, 1, 1)
    );
    assert!(placement.candidates <= 256 && placement.geometry_checks <= 250_000);
    assert!(
        placement.geometry_checks >= 5,
        "newborn wall checks must be reported"
    );
    assert_eq!(state.world.snakes.len(), 1);
    let newborn = &state.world.snakes[0];
    assert!(newborn.alive);
    assert_ne!(newborn.id, original_id);
    assert_ne!(newborn.frame_v1_id, original_frame);
    assert_eq!(newborn.brain, Some(original_brain));
    assert_eq!(state.brains[0].owner, BrainOwner::PopulationSlot(0));
    assert_eq!(state.population, genomes);
    let life = &state.fixed_step.evolved_generation.slots[0];
    assert_eq!(life.life_ordinal, 2);
    assert_eq!(life.successful_respawns, 1);
    assert_eq!(life.deaths, 1);
    assert_eq!(life.current.alive_seconds, 1.0 / 60.0);
    assert_eq!(life.pending_seconds, 0.0);
    assert_eq!(life.ledger.life_gross_earned, 0.01);
    assert!(!life.pending_recurrent_reset);
    assert_eq!(newborn.delivered_observation_points, 0.01);
    assert_eq!(newborn.previous_turn, 0.0);
    assert!(!newborn.previous_input_boost);
    assert_eq!(diagnostics.control.selection.neural_evaluations, 1);
    let pellet = state
        .world
        .pellets
        .iter()
        .find(|pellet| pellet.id == pellet_id)
        .unwrap();
    assert_eq!(pellet.owner, None);
    assert_eq!(pellet.color, 123);
    assert_eq!(pellet.value, 1.0);
}

#[test]
fn evolved_newborn_manual_terminal_step_admits_new_identity_and_negative_life_fitness() {
    let graph = default_graph();
    let mut source = respawn_candidate(&graph, false, 0.8);
    set_complete_setting(
        &mut source,
        "reward.lifePenalty",
        NormalizedSettingValue::Float(10000.0),
    );
    refresh_config_hash(&mut source);
    let old_id = source.world.snakes[0].id;
    let weights = source.population[0].weights.clone();
    let mut authority = own_complete_running(source, graph);
    let original = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    coordinator.request_manual_finish();
    let transition = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .unwrap()
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected newborn terminal boundary, got {other:?}"),
    };
    assert_eq!(transition.reason(), GenerationTransitionReason::Manual);
    let hof = transition.metadata().hall_of_fame;
    assert_ne!(hof.snake_id, old_id);
    assert_eq!(hof.source_slot, 0);
    assert!((hof.fitness - (642.0 + 0.70 / 60.0 - 10000.0)).abs() < 1e-10);
    assert_eq!(
        transition.commit_record().hall_of_fame.source_snake_id,
        hof.snake_id
    );
    let experiment = transition.commit_record().experiment.unwrap();
    assert_eq!(experiment.history[8], 2);
    assert_eq!(experiment.history[9], 1);
    assert_eq!(experiment.history[10], 1);
    assert_eq!(experiment.history[4], 0);
    assert_eq!(f64::from_bits(experiment.history[55]), 10000.0);
    assert_eq!(experiment.hall_of_fame[3], hof.snake_id);
    assert_eq!(experiment.hall_of_fame[7], 2);
    assert_eq!(
        crate::engine::experiment_records::GenerationExperimentRecords::decode(
            &experiment.encode()
        )
        .unwrap(),
        experiment
    );
    assert_eq!(transition.candidate().population[0].weights, weights);
    assert!(transition
        .candidate()
        .fixed_step
        .evolved_generation
        .slots
        .is_empty());
    assert!(transition.candidate().brains[0]
        .recurrent
        .iter()
        .all(|value| *value == 0.0));
    assert_eq!(authority.state(), &original);
}

#[test]
fn evolved_waiting_unlimited_is_not_extinction_and_finite_exhaustion_can_end() {
    let graph = default_graph();
    for (maximum, early_mode, terminal) in [
        (0, 0, false),
        (2, 0, false),
        (1, 0, true),
        (0, 1, true),
        (1, 2, false),
    ] {
        let mut source = respawn_candidate(&graph, false, 0.0);
        source.generation.elapsed_seconds = 8.0;
        set_complete_setting(
            &mut source,
            "evolvedRespawn.delaySeconds",
            NormalizedSettingValue::Float(10.0),
        );
        set_complete_setting(
            &mut source,
            "evolvedRespawn.maxLives",
            NormalizedSettingValue::Integer(maximum),
        );
        set_complete_setting(
            &mut source,
            "observer.earlyEndMode",
            NormalizedSettingValue::Integer(early_mode),
        );
        refresh_config_hash(&mut source);
        let mut authority = own_complete_running(source, Arc::clone(&graph));
        let original = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        let progress = coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .unwrap();
        if terminal {
            let RunningStepProgress::GenerationTransitionPending(transition) = progress else {
                panic!("expected early terminal");
            };
            assert_eq!(
                transition.reason(),
                GenerationTransitionReason::EarlyAliveCount
            );
            assert_eq!(transition.alive_evolved(), 0);
            assert_eq!(authority.state(), &original);
        } else {
            assert!(matches!(progress, RunningStepProgress::Published(_)));
            let life = &authority.state().fixed_step.evolved_generation.slots[0];
            assert_eq!(life.lives_started, 1);
            assert_eq!(life.total_alive_seconds, 0.0);
            assert_eq!(
                life.pending_seconds,
                if maximum == 1 { 0.0 } else { 1.0 / 60.0 }
            );
        }
    }
}

#[test]
fn evolved_collision_death_respawns_only_at_the_next_newborn_boundary() {
    let graph = default_graph();
    let mut source = respawn_candidate(&graph, false, 0.8);
    let snake = &mut source.world.snakes[0];
    snake.alive = true;
    snake.position = WorldPoint { x: 3490.5, y: 0.0 };
    snake.previous_position = snake.position;
    snake.direction = 0.0;
    snake.turn = 0.0;
    snake.input_boost = false;
    snake.previous_input_boost = false;
    for point in &mut source.world.body_points {
        point.x += 3480.5;
        point.y -= 20.0;
    }
    source.fixed_step.evolved_generation =
        crate::engine::evolved_lives::EvolvedGenerationState::initialize(&source.world, 1, 5)
            .unwrap();
    let old_id = source.world.snakes[0].id;
    let mut authority = own_complete_running(source, graph);
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
    let dead = authority.state();
    assert!(!dead.world.snakes[0].alive);
    assert_eq!(dead.world.snakes[0].id, old_id);
    let life = &dead.fixed_step.evolved_generation.slots[0];
    assert_eq!(life.lives_started, 1);
    assert_eq!(life.deaths, 1);
    assert_eq!(life.successful_respawns, 0);
    assert_eq!(
        life.status,
        crate::engine::evolved_lives::EvolvedLifeStatus::Waiting
    );
    assert_eq!(
        life.respawn_deadline_step,
        Some(dead.generation.completed_step + 1)
    );
    assert!(matches!(
        coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 117,
                    wall_accumulator_seconds: 0.0
                }
            )
            .unwrap(),
        RunningStepProgress::Published(_)
    ));
    let newborn = authority.state();
    assert!(newborn.world.snakes[0].alive);
    assert_ne!(newborn.world.snakes[0].id, old_id);
    assert_eq!(
        newborn.fixed_step.evolved_generation.slots[0].lives_started,
        2
    );
    assert_eq!(newborn.fixed_step.evolved_generation.slots[0].deaths, 1);
}

#[test]
fn evolved_cold_god_death_samples_once_before_prefix_and_final_exhaustion_uses_normal_corpse() {
    use crate::engine::evolved_lives::{
        EvolvedDeathCause, EvolvedGenerationState, EvolvedLifeStatus,
    };
    let graph = default_graph();
    let mut source = respawn_candidate(&graph, false, 0.0);
    source.world.snakes[0].alive = true;
    source.fixed_step.evolved_generation = EvolvedGenerationState::default();
    for (path, value) in [
        ("evolvedRespawn.maxLives", 2),
        ("evolvedRespawn.corpseMode", 1),
        ("evolvedRespawn.delayMode", 1),
        ("evolvedRespawn.rngMode", 1),
    ] {
        set_complete_setting(&mut source, path, NormalizedSettingValue::Integer(value));
    }
    refresh_config_hash(&mut source);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let physics = authority
        .running_step_config(RunningStepWorkLimits::provisional_defaults())
        .unwrap()
        .world_step
        .physics;
    let old_rng = authority.state().rng.clone();
    let mut expected = StatefulRng::from_state(&old_rng.world).unwrap();
    expected.next_f64();
    let frame_id = authority.state().world.snakes[0].frame_v1_id;
    let killed = authority.apply_god_mode_kill(frame_id, physics).unwrap();
    assert_eq!(killed.pellets_dropped, 0);
    let life = &authority.state().fixed_step.evolved_generation.slots[0];
    assert_eq!(life.status, EvolvedLifeStatus::Waiting);
    assert_eq!(life.deaths, 1);
    assert_eq!(life.last_cause, Some(EvolvedDeathCause::GodMode));
    let event = authority
        .state()
        .fixed_step
        .evolved_generation
        .transaction_life_ends[0];
    assert_eq!(event.contact_flags, 8);
    assert_eq!(event.physical_id, authority.state().world.snakes[0].id);
    assert_eq!(event.corpse_emitted_count, 0);
    assert_eq!(event.life_ordinal, 1);
    assert_eq!(
        authority
            .state()
            .fixed_step
            .evolved_generation
            .life_end_count,
        1
    );

    assert_eq!(
        life.respawn_deadline_step,
        Some(authority.state().generation.completed_step + 1)
    );
    assert_eq!(authority.state().rng.world, expected.export_state());
    assert_eq!(authority.state().rng.evolution, old_rng.evolution);
    assert_eq!(
        authority.state().rng.evolved_respawns,
        old_rng.evolved_respawns
    );
    let dead = authority.state().clone();
    assert!(authority.apply_god_mode_kill(frame_id, physics).is_err());
    assert_eq!(authority.state(), &dead);
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
        authority.state().fixed_step.evolved_generation.slots[0].lives_started,
        2
    );
    let frame_id = authority.state().world.snakes[0].frame_v1_id;
    let before = authority.state().rng.world.clone();
    let last = authority.apply_god_mode_kill(frame_id, physics).unwrap();
    assert_eq!(last.pellets_dropped, 2);
    let life = &authority.state().fixed_step.evolved_generation.slots[0];
    assert_eq!(life.status, EvolvedLifeStatus::Exhausted);
    assert_eq!(life.deaths, 2);
    assert_eq!(life.successful_respawns, 1);
    assert_eq!(life.respawn_deadline_step, None);
    let mut expected = StatefulRng::from_state(&before).unwrap();
    for _ in 0..6 {
        expected.next_f64();
    }
    assert_eq!(authority.state().rng.world, expected.export_state());
}

#[test]
fn evolved_cold_god_overflow_fail_is_atomic_and_discard_counts_actual_values() {
    for discard in [false, true] {
        let graph = default_graph();
        let mut source = respawn_candidate(&graph, false, 0.0);
        source.world.snakes[0].alive = true;
        source.fixed_step.evolved_generation =
            crate::engine::evolved_lives::EvolvedGenerationState::default();
        source.config.max_pellets = 100;
        set_complete_setting(
            &mut source,
            "death.maxPellets",
            NormalizedSettingValue::Integer(100),
        );
        set_complete_setting(
            &mut source,
            "pelletCountTarget",
            NormalizedSettingValue::Integer(100),
        );
        for _ in 0..100 {
            source.world.pellets.push(PelletState {
                id: source.allocators.next_entity_id,
                position: WorldPoint { x: 3000.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 123,
                owner: None,
            });
            source.allocators.next_entity_id += 1;
        }
        set_complete_setting(
            &mut source,
            "world.pelletOverflow",
            NormalizedSettingValue::Integer(i64::from(discard)),
        );
        refresh_config_hash(&mut source);
        let mut authority = own_complete_running(source, graph);
        let original = authority.state().clone();
        let physics = authority
            .running_step_config(RunningStepWorkLimits::provisional_defaults())
            .unwrap()
            .world_step
            .physics;
        let result = authority.apply_god_mode_kill(original.world.snakes[0].frame_v1_id, physics);
        if !discard {
            assert!(result.is_err());
            assert_eq!(authority.state(), &original);
        } else {
            assert_eq!(result.unwrap().pellets_dropped, 0);
            assert_eq!(authority.state().world.pellets, original.world.pellets);
            assert_eq!(authority.state().allocators, original.allocators);
            let life = &authority.state().fixed_step.evolved_generation.slots[0];
            assert_eq!(life.corpse_emitted_count, 0);
            assert_eq!(life.corpse_discarded_count, 2);
            assert!(life.corpse_discarded_value > 0.0);
            assert_eq!(life.deaths, 1);
            assert_ne!(authority.state().rng.world, original.rng.world);
            assert_eq!(authority.state().rng.evolution, original.rng.evolution);
        }
    }
}

#[test]
fn evolved_running_overflow_publishes_whole_death_or_rolls_back_the_whole_step() {
    for discard in [false, true] {
        let graph = default_graph();
        let mut source = respawn_candidate(&graph, false, 0.0);
        source.world.snakes[0].alive = true;
        source.world.snakes[0].turn = 0.0;
        source.world.snakes[0].previous_turn = 0.0;
        source.world.snakes[0].input_boost = false;
        source.world.snakes[0].previous_input_boost = false;
        source.config.population_count = 2;
        source.config.max_world_snakes = 33;
        source.config.max_pellets = 100;
        set_complete_setting(
            &mut source,
            "snakeCount",
            NormalizedSettingValue::Integer(2),
        );
        set_complete_setting(
            &mut source,
            "death.maxPellets",
            NormalizedSettingValue::Integer(100),
        );
        set_complete_setting(
            &mut source,
            "pelletCountTarget",
            NormalizedSettingValue::Integer(100),
        );
        set_complete_setting(
            &mut source,
            "world.pelletOverflow",
            NormalizedSettingValue::Integer(i64::from(discard)),
        );
        let mut owner_genome = source.population[0].clone();
        owner_genome.slot = 1;
        owner_genome.brain.id = source.allocators.next_brain_id;
        owner_genome.lineage.genome_id = source.allocators.next_genome_id;
        source.brains.push(BrainRuntimeState {
            handle: owner_genome.brain,
            owner: BrainOwner::PopulationSlot(1),
            retained_origin: None,
            non_population_weights: None,
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        });
        source.population.push(owner_genome);
        source.allocators.next_brain_id += 1;
        source.allocators.next_genome_id += 1;
        let id = source.allocators.next_entity_id;
        push_evolved_snake(&mut source, 1, id, 2, WorldPoint { x: 100.0, y: 20.0 });
        source
            .world
            .body_points
            .extend((1..15).map(|index| WorldPoint {
                x: 100.0 - index as f64 * 7.5,
                y: 20.0,
            }));
        source.world.snakes[1].body.len = 15;
        source.world.snakes[1].target_length = 15.0;
        source.world.snakes[1].radius = 9.0;
        for _ in 0..100 {
            source.world.pellets.push(PelletState {
                id: source.allocators.next_entity_id,
                position: WorldPoint { x: 3000.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 123,
                owner: None,
            });
            source.allocators.next_entity_id += 1;
        }
        source.rng.evolved_respawns =
            EvolvedRespawnRngState::derive_generation(source.identity.seed, 1, 2).unwrap();
        source.fixed_step.evolved_generation =
            crate::engine::evolved_lives::EvolvedGenerationState::initialize(&source.world, 2, 5)
                .unwrap();
        refresh_config_hash(&mut source);
        let mut authority = own_complete_running(source, graph);
        let original = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        let result = coordinator.advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        );
        if !discard {
            assert!(result.is_err());
            assert_eq!(authority.state(), &original);
        } else {
            let RunningStepProgress::Published(outcome) = result.unwrap() else {
                panic!("expected complete saturated death");
            };
            assert_eq!(outcome.diagnostics.emissions.attempted_count, 2);
            assert_eq!(outcome.diagnostics.emissions.discarded_count, 2);
            assert_eq!(outcome.diagnostics.emissions.emitted_count, 0);
            assert!(outcome.diagnostics.emissions.discarded_value > 0.0);
            let state = authority.state();
            assert!(!state.world.snakes[0].alive);
            assert_eq!(state.world.snakes[1].kills, 1);
            assert_eq!(state.world.pellets, original.world.pellets);
            assert_eq!(state.allocators, original.allocators);
            assert_eq!(state.population, original.population);
            let life = &state.fixed_step.evolved_generation.slots[0];
            assert_eq!(life.deaths, 1);
            assert_eq!(life.corpse_emitted_count, 0);
            assert_eq!(life.corpse_discarded_count, 2);
            assert_eq!(state.rng.evolution, original.rng.evolution);
            assert_eq!(state.rng.evolved_respawns, original.rng.evolved_respawns);
        }
    }
}

#[test]
fn evolved_identity_exhaustion_rolls_back_life_rng_ledger_and_recurrence() {
    let graph = default_graph();
    let mut source = respawn_candidate(&graph, false, 0.8);
    source.allocators.next_entity_id = EXTERNAL_ENTITY_ID_START;
    set_complete_setting(
        &mut source,
        "pelletSpawnPerSecond",
        NormalizedSettingValue::Float(5.0),
    );
    refresh_config_hash(&mut source);
    let mut authority = own_complete_running(source, graph);
    let original = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    for _ in 0..2 {
        let error = coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .unwrap_err();
        assert!(
            matches!(&error, RunningStepError::Prefix(error)
            if matches!(error.as_ref(), crate::engine::fixed_step::FixedStepPrefixError::Allocator(_))),
            "{error}"
        );
        assert_eq!(authority.state(), &original);
    }
}

#[test]
fn evolved_wallet_choice_is_independent_of_terminal_point_assessment() {
    let graph = default_graph();
    for (mode, basis, carried, wallet, points) in [
        (0, 0, 0, 0.01, 0.02),
        (1, 0, 0, 0.01, 10.02),
        (1, 0, 1, 10.01, 10.02),
        (1, 1, 1, 10.01, 10.02),
        (2, 0, 1, 0.01, 10.0),
    ] {
        let mut source = respawn_candidate(&graph, false, 0.0);
        source.world.snakes[0].points = 10.0;
        source.fixed_step.evolved_generation =
            crate::engine::evolved_lives::EvolvedGenerationState::initialize(&source.world, 1, 5)
                .unwrap();
        set_complete_setting(
            &mut source,
            "lifeScoring.mode",
            NormalizedSettingValue::Integer(mode),
        );
        set_complete_setting(
            &mut source,
            "lifeScoring.cumulativeBasis",
            NormalizedSettingValue::Integer(basis),
        );
        set_complete_setting(
            &mut source,
            "lifeScoring.cumulativeWallet",
            NormalizedSettingValue::Integer(carried),
        );
        refresh_config_hash(&mut source);
        let (mut authority, _) = one_respawn_step(source, Arc::clone(&graph));
        // Inspect the immutable newborn ledger directly; no extra physical step or sensor delivery.
        let state = authority.state();
        let life_policy =
            crate::engine::step_config::project_evolved_experiment_config(&state.config).unwrap();
        let target = crate::engine::inspector::InspectorTarget {
            world_epoch: authority.world_epoch(),
            generation: state.generation.generation,
            physical_id: state.world.snakes[0].id,
            frame_v1_id: state.world.snakes[0].frame_v1_id,
        };
        let mut capture = crate::engine::inspector::InspectorCapture::new(
            target,
            crate::engine::inspector::MAX_INSPECTOR_BYTES,
        )
        .unwrap();
        capture.configure_evolution(authority.evolution_config().unwrap());
        let key = crate::engine::physics::PhysicsStepKey::new(
            target.world_epoch,
            target.generation,
            state.generation.completed_step,
            1,
            state.identity.config_revision,
            [0; 32],
            1,
        );
        let layout = crate::engine::inspector::InspectorLayout::from(
            &crate::engine::sensor_layout::SensorLayout::new(8).unwrap(),
        );
        capture.stage(
            key,
            &state.world.snakes[0],
            crate::engine::inspector::InspectorObservationSource::Neural,
            layout,
            &vec![0.0; layout.input_size],
            Some([0.0; 2]),
            None,
            Default::default(),
        );
        capture.stage_lifecycle(
            key,
            &state.world,
            crate::engine::inspector::InspectorLifecycleContext {
                lives: &state.fixed_step.evolved_generation,
                policy: life_policy,
            },
        );
        capture.publish(key, |_, _, _| false);
        let observed = capture
            .latest(
                target.world_epoch,
                target.generation,
                state.generation.completed_step + 1,
            )
            .unwrap();
        let life = observed.lifecycle.unwrap();
        let expected = if mode == 2 { points } else { points - 0.01 };
        assert!(
            (life.assessment.points - expected).abs() < 1e-12,
            "assessment {mode}/{basis}/{carried}"
        );
        assert!(
            (life.ledger.life_local_wallet + life.ledger.prior_bank_remaining - wallet).abs()
                < 1e-12
        );
        assert_eq!(life.life_ordinal, 2);
        assert_eq!(life.deaths, 1);
        assert_eq!(life.successful_respawns, 1);
        assert_eq!(
            life.ledger,
            state.fixed_step.evolved_generation.slots[0].ledger
        );
        let newborn = &authority.state().world.snakes[0];
        assert!((newborn.points - wallet).abs() < 1e-12);
        assert_eq!(newborn.delivered_observation_points, newborn.points);
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        coordinator.request_manual_finish();
        let RunningStepProgress::GenerationTransitionPending(transition) = coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 117,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .unwrap()
        else {
            panic!("expected terminal assessment");
        };
        assert!(
            (transition.metadata().hall_of_fame.points - points).abs() < 1e-12,
            "mode={mode}, basis={basis}, carried={carried}"
        );
        let record = transition.commit_record();
        let experiment = record.experiment.unwrap();
        assert!((f64::from_bits(experiment.hall_of_fame[8]) - points).abs() < 1e-12);
        assert_eq!(
            record.hall_of_fame.points_f64_bits,
            experiment.hall_of_fame[20]
        );
        assert_eq!(record.hall_of_fame.length, experiment.hall_of_fame[24]);
        assert_eq!(
            experiment.hall_of_fame[29],
            if mode == 0 {
                0
            } else if mode == 1 {
                1
            } else {
                2
            }
        );
        assert_eq!(
            crate::engine::experiment_records::GenerationExperimentRecords::decode(
                &experiment.encode()
            )
            .unwrap(),
            experiment
        );
    }
}

#[test]
fn evolved_newborn_reset_uses_zero_source_while_preserve_retains_cross_life_memory() {
    let graph = default_graph();
    let (reset_a, _) = one_respawn_step(respawn_candidate(&graph, false, 0.2), Arc::clone(&graph));
    let (reset_b, _) = one_respawn_step(respawn_candidate(&graph, false, 0.8), Arc::clone(&graph));
    assert_eq!(
        reset_a.state().brains[0].recurrent,
        reset_b.state().brains[0].recurrent
    );
    let (preserve_a, _) =
        one_respawn_step(respawn_candidate(&graph, true, 0.2), Arc::clone(&graph));
    let (preserve_b, _) = one_respawn_step(respawn_candidate(&graph, true, 0.8), graph);
    assert_ne!(
        preserve_a.state().brains[0].recurrent,
        preserve_b.state().brains[0].recurrent
    );
}
