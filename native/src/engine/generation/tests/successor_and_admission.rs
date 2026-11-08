use super::*;

#[test]
fn completed_generation_prepares_one_admissible_zero_state_boundary() {
    let graph = graph_bundle();
    let source = fixture(graph.compiled());
    let admission = policy(&source);
    let mut source_authority =
        AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
            .expect("running source must admit");
    let source_key = source_authority
        .begin_running_step()
        .expect("terminal attempt must have an exact key");
    let source_before = source_authority.state().clone();
    let mut completed_rng = source_before.rng.clone();
    completed_rng.world = advanced(&completed_rng.world);
    completed_rng.external_controller = advanced(&completed_rng.external_controller);
    let completed_rng_before = completed_rng.clone();
    let mut completed_allocators = source_before.allocators.clone();
    completed_allocators.next_entity_id += 3;
    completed_allocators.next_brain_id += 1;
    completed_allocators.next_controller_lease_id += 1;
    completed_allocators.next_frame_v1_id += 2;
    completed_allocators.next_external_id += 1;
    let completed_allocators_before = completed_allocators.clone();
    let elapsed =
        source_before.generation.elapsed_seconds + source_before.config.fixed_step_seconds;

    let prepared = prepare_generation_boundary(
        source_authority.state(),
        &source_before.world,
        &completed_rng,
        &completed_allocators,
        elapsed,
        graph.compiled(),
    )
    .expect("complete generation must prepare");
    assert_eq!(source_authority.state(), &source_before);
    assert_eq!(completed_rng, completed_rng_before);
    assert_eq!(completed_allocators, completed_allocators_before);
    let admitted = admit_prepared_generation_boundary(&source_authority, source_key, prepared)
        .expect("prepared successor must fit beside its source");
    let metadata = admitted.metadata();
    let commit_record = admitted.commit_record();
    assert_eq!(metadata.summary.generation, 7);
    assert_eq!(metadata.hall_of_fame.generation, 7);
    assert_eq!(metadata.hall_of_fame_population_slot, 0);
    let candidate = admitted.candidate();
    assert_eq!(
        commit_record.summary,
        GenerationSummaryCommitRecord {
            completed_generation: metadata.summary.generation,
            best_f64_bits: metadata.summary.best.to_bits(),
            average_f64_bits: metadata.summary.average.to_bits(),
            minimum_f64_bits: metadata.summary.minimum.to_bits(),
            species_count: metadata.summary.species_count,
            top_species_size: metadata.summary.top_species_size,
            average_weight_f64_bits: metadata.summary.average_weight.to_bits(),
            weight_variance_f64_bits: metadata.summary.weight_variance.to_bits(),
        }
    );
    assert_eq!(
        commit_record.hall_of_fame,
        HallOfFameCommitRecord {
            completed_generation: metadata.hall_of_fame.generation,
            source_population_slot: u64::from(metadata.hall_of_fame.source_slot),
            source_snake_id: metadata.hall_of_fame.snake_id,
            fitness_f64_bits: metadata.hall_of_fame.fitness.to_bits(),
            points_f64_bits: metadata.hall_of_fame.points.to_bits(),
            length: u64::try_from(metadata.hall_of_fame.length).unwrap(),
            successor_population_slot: u64::from(metadata.hall_of_fame_population_slot),
            successor_genome_id: candidate.population
                [metadata.hall_of_fame_population_slot as usize]
                .lineage
                .genome_id,
        }
    );
    assert_eq!(
        candidate.population[metadata.hall_of_fame_population_slot as usize].weights,
        source_before.population[metadata.hall_of_fame.source_slot as usize].weights
    );
    assert_eq!(
        candidate.phase,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation)
    );
    assert_eq!(candidate.generation.generation, 8);
    assert_eq!(candidate.generation.completed_step, 100);
    assert_eq!(candidate.generation.population_epoch, 8);
    assert_eq!(candidate.generation.elapsed_seconds, 0.0);
    assert_eq!(candidate.generation.wall_accumulator_seconds, 0.0);
    assert!(candidate.world.snakes.is_empty());
    assert!(candidate.world.body_points.is_empty());
    assert!(candidate.world.pellets.is_empty());
    assert!(candidate.world.controller_leases.is_empty());
    assert_eq!(candidate.rng.world, completed_rng.world);
    assert_eq!(
        candidate.rng.external_controller,
        completed_rng.external_controller
    );
    assert_ne!(candidate.rng.evolution, source_before.rng.evolution);
    assert_eq!(
        candidate.rng.baselines.len(),
        source_before.config.baseline_count
    );
    assert_eq!(candidate.allocators.next_brain_id, 25);
    assert_eq!(candidate.allocators.next_genome_id, 104);
    assert!(candidate
        .brains
        .iter()
        .all(|brain| brain.recurrent.iter().all(|value| value.to_bits() == 0)));
    for (slot, genome) in candidate.population.iter().enumerate() {
        assert_eq!(genome.slot as usize, slot);
        assert_eq!(genome.brain.epoch, 8);
        assert_eq!(genome.lineage.genome_id, 100 + slot as u64);
        assert_eq!(genome.lineage.birth_generation, 8);
        assert!(genome.lineage.parent_a.is_some());
    }
    assert_eq!(
        admitted
            .successor
            .authority()
            .checkpoint_boundary()
            .expect("prepared state must expose a checkpoint boundary")
            .kind(),
        GenerationBoundaryKind::Generation
    );
    assert!(admitted.combined_state_bytes() > source_authority.memory_estimate().total_bytes);
    assert_eq!(source_authority.state(), &source_before);

    let directory = TestDirectory::new();
    let descriptor = admitted
        .publish_managed_checkpoint(
            &source_authority,
            directory.path(),
            CheckpointOperationId::parse("00000000000000000000000000000071")
                .expect("fixture operation ID"),
            &checkpoint_limits(),
            &graph_limits(),
        )
        .expect("prepared generation must use the production checkpoint writer");
    assert_eq!(descriptor.boundary_kind, CheckpointBoundaryKind::Generation);
    assert_eq!(descriptor.transition_epoch_hex, "0000000000000001");
    assert_eq!(source_authority.state(), &source_before);
    let restored = restore_checkpoint(
        &directory.path().join(&descriptor.relative_filename),
        &checkpoint_limits(),
        &graph_limits(),
        &admission,
    )
    .expect("published generation boundary must restore");
    assert_eq!(
        restored.content.boundary_kind,
        CheckpointBoundaryKind::Generation
    );
    assert_eq!(restored.state.state(), admitted.candidate());

    let projection = project_running_step_config(
        &admitted.candidate().config,
        RunningStepWorkLimits::provisional_defaults(),
    )
    .expect("admitted generation settings must project for construction");
    let start_config =
        GenerationStartConfig::from_work_limits(RunningStepWorkLimits::provisional_defaults());
    let boundary_before_start = admitted.candidate().clone();
    let mut start_workspace = GenerationStartWorkspace::new();
    let started = start_workspace
        .prepare(admitted.candidate(), start_config)
        .expect("durable boundary must construct one collision-safe generation");
    assert_eq!(admitted.candidate(), &boundary_before_start);
    assert_eq!(
        started.world().snakes.len(),
        admitted.candidate().config.population_count
    );
    assert_eq!(
        started.world().body_points.len(),
        admitted.candidate().config.population_count
            * projection.world_step.prefix.baseline_spawn.snake_start_len
    );
    assert_eq!(
        started.world().pellets.len(),
        projection.world_step.prefix.ambient.target_count
    );
    assert!(started.world().controller_leases.is_empty());
    assert!(started
        .world()
        .snakes
        .iter()
        .enumerate()
        .all(|(slot, snake)| {
            snake.kind == SnakeKind::Evolved
                && snake.population_slot == Some(slot as u32)
                && snake.brain == Some(admitted.candidate().population[slot].brain)
                && snake.control_accumulator_seconds.to_bits()
                    == projection
                        .world_step
                        .control
                        .initial_neural_accumulator_seconds()
                        .to_bits()
        }));
    assert_eq!(started.rng().evolution, admitted.candidate().rng.evolution);
    assert_eq!(
        started.rng().external_controller,
        admitted.candidate().rng.external_controller
    );
    assert_ne!(started.rng().world, admitted.candidate().rng.world);
    assert_eq!(
        started.allocators().next_entity_id,
        admitted.candidate().allocators.next_entity_id
            + admitted.candidate().config.population_count as u64
            + projection.world_step.prefix.ambient.target_count as u64
    );
    assert!(started.fixed_step().baseline_lifecycle.slots.is_empty());
    assert_eq!(
        started
            .fixed_step()
            .sensor_generation
            .best_points_this_generation(),
        0.0
    );
    started
        .validate_current(admitted.candidate(), start_config)
        .expect("prepared construction must retain exact source provenance");

    let mut running_candidate = admitted.candidate().clone();
    running_candidate.phase = AuthorityPhase::Running;
    running_candidate.world = started.world().clone();
    running_candidate.rng = started.rng().clone();
    running_candidate.allocators = started.allocators().clone();
    running_candidate.fixed_step = started.fixed_step().clone();
    let running_policy = policy(&running_candidate);
    AuthoritativeState::validate_and_own(running_candidate, Arc::clone(&graph), &running_policy)
        .expect("constructed generation must satisfy complete running-state admission");
}

#[test]
fn generation_commit_rejects_a_forged_successor_elite_slot() {
    let graph = graph_bundle();
    let source = fixture(graph.compiled());
    let admission = policy(&source);
    let mut authority =
        AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
            .expect("running source must admit");
    let source_key = authority
        .begin_running_step()
        .expect("terminal attempt key");
    let source_before = authority.state().clone();
    let mut completed_rng = source_before.rng.clone();
    completed_rng.world = advanced(&completed_rng.world);
    completed_rng.external_controller = advanced(&completed_rng.external_controller);
    let mut completed_allocators = source_before.allocators.clone();
    completed_allocators.next_entity_id += 3;
    completed_allocators.next_brain_id += 1;
    completed_allocators.next_controller_lease_id += 1;
    completed_allocators.next_frame_v1_id += 2;
    completed_allocators.next_external_id += 1;
    let elapsed =
        source_before.generation.elapsed_seconds + source_before.config.fixed_step_seconds;
    let mut prepared = prepare_generation_boundary(
        authority.state(),
        &source_before.world,
        &completed_rng,
        &completed_allocators,
        elapsed,
        graph.compiled(),
    )
    .expect("complete generation must prepare");
    assert!(prepared.candidate.population.len() > 1);
    prepared.metadata.hall_of_fame_population_slot = 1;

    assert!(matches!(
        admit_prepared_generation_boundary(&authority, source_key, prepared),
        Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit successor does not identify the admitted Hall-of-Fame elite"
        })
    ));
    assert_eq!(authority.state(), &source_before);
}

#[test]
fn generation_commit_requires_bit_exact_successor_elite_weights() {
    let graph = graph_bundle();
    let mut source = fixture(graph.compiled());
    for genome in &mut source.population {
        genome.weights[0] = -0.0;
    }
    let admission = policy(&source);
    let mut authority =
        AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
            .expect("running source must admit");
    let source_key = authority
        .begin_running_step()
        .expect("terminal attempt key");
    let source_before = authority.state().clone();
    let mut completed_rng = source_before.rng.clone();
    completed_rng.world = advanced(&completed_rng.world);
    completed_rng.external_controller = advanced(&completed_rng.external_controller);
    let mut completed_allocators = source_before.allocators.clone();
    completed_allocators.next_entity_id += 3;
    completed_allocators.next_brain_id += 1;
    completed_allocators.next_controller_lease_id += 1;
    completed_allocators.next_frame_v1_id += 2;
    completed_allocators.next_external_id += 1;
    let elapsed =
        source_before.generation.elapsed_seconds + source_before.config.fixed_step_seconds;
    let mut prepared = prepare_generation_boundary(
        authority.state(),
        &source_before.world,
        &completed_rng,
        &completed_allocators,
        elapsed,
        graph.compiled(),
    )
    .expect("complete generation must prepare");
    let elite_slot = prepared.metadata.hall_of_fame_population_slot as usize;
    assert_eq!(
        prepared.candidate.population[elite_slot].weights[0].to_bits(),
        (-0.0_f32).to_bits()
    );
    prepared.candidate.population[elite_slot].weights[0] = 0.0;

    assert!(matches!(
        admit_prepared_generation_boundary(&authority, source_key, prepared),
        Err(GenerationTransitionError::PopulationShape {
            reason: "generation commit successor does not identify the admitted Hall-of-Fame elite"
        })
    ));
    assert_eq!(authority.state(), &source_before);
}

#[test]
fn successor_admission_charges_both_states_and_rejects_a_stale_attempt() {
    let graph = graph_bundle();
    let source = fixture(graph.compiled());
    let elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;
    let prepared = prepare_generation_boundary(
        &source,
        &source.world,
        &source.rng,
        &source.allocators,
        elapsed,
        graph.compiled(),
    )
    .expect("fixture successor must prepare");
    let admitted_probe =
        AuthoritativeState::validate_and_own(source.clone(), Arc::clone(&graph), &policy(&source))
            .expect("probe source must admit");
    let source_bytes = admitted_probe.memory_estimate().total_bytes;
    drop(admitted_probe);
    let successor_bytes = estimate_state_memory(prepared.candidate(), &graph)
        .expect("successor memory must estimate")
        .total_bytes;
    let constrained_ceiling = source_bytes
        .checked_add(successor_bytes)
        .and_then(|sum| sum.checked_sub(1))
        .expect("fixture combined memory must be positive");
    let mut constrained_policy = policy(&source);
    constrained_policy.memory_ceiling_bytes = constrained_ceiling;
    let mut constrained = AuthoritativeState::validate_and_own(
        source.clone(),
        Arc::clone(&graph),
        &constrained_policy,
    )
    .expect("source alone must fit the constrained process ceiling");
    let constrained_key = constrained.begin_running_step().unwrap();
    assert!(matches!(
        admit_prepared_generation_boundary(&constrained, constrained_key, prepared),
        Err(GenerationTransitionError::State(error))
            if matches!(*error, StateError::MemoryCeilingExceeded { .. })
    ));
    assert_eq!(constrained.state(), &source);

    let prepared = prepare_generation_boundary(
        &source,
        &source.world,
        &source.rng,
        &source.allocators,
        elapsed,
        graph.compiled(),
    )
    .expect("stale-key fixture successor must prepare");
    let mut current =
        AuthoritativeState::validate_and_own(source.clone(), Arc::clone(&graph), &policy(&source))
            .expect("roomy source must admit");
    let stale_key = current.begin_running_step().unwrap();
    let _newer_key = current.begin_running_step().unwrap();
    assert!(matches!(
        admit_prepared_generation_boundary(&current, stale_key, prepared),
        Err(GenerationTransitionError::State(error))
            if matches!(*error, StateError::StaleFixedStep { .. })
    ));
    assert_eq!(current.state(), &source);
}

#[test]
fn admitted_boundary_refuses_checkpoint_after_source_attempt_changes() {
    let graph = graph_bundle();
    let source = fixture(graph.compiled());
    let admission = policy(&source);
    let mut current = AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &admission)
        .expect("running source must admit");
    let source_key = current.begin_running_step().expect("terminal attempt key");
    let elapsed =
        current.state().generation.elapsed_seconds + current.state().config.fixed_step_seconds;
    let prepared = prepare_generation_boundary(
        current.state(),
        &current.state().world,
        &current.state().rng,
        &current.state().allocators,
        elapsed,
        graph.compiled(),
    )
    .expect("fixture successor must prepare");
    let admitted = admit_prepared_generation_boundary(&current, source_key, prepared)
        .expect("successor must admit beside the source");
    let source_before = current.state().clone();
    let newer_key = current
        .begin_running_step()
        .expect("new attempt must invalidate the admitted boundary");
    assert_ne!(newer_key, source_key);
    let directory = TestDirectory::new();
    assert!(matches!(
        admitted.publish_managed_checkpoint(
            &current,
            directory.path(),
            CheckpointOperationId::parse("00000000000000000000000000000072")
                .expect("fixture operation ID"),
            &checkpoint_limits(),
            &graph_limits(),
        ),
        Err(GenerationTransitionError::State(error))
            if matches!(*error, StateError::StaleFixedStep { .. })
    ));
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("test directory must remain readable")
            .count(),
        0
    );
    assert_eq!(current.state(), &source_before);
}

#[test]
fn stale_or_exhausted_continuations_leave_every_source_unchanged() {
    let graph = graph_bundle();
    let source = fixture(graph.compiled());
    let before = source.clone();
    let elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;

    let mut contaminated = source.rng.clone();
    contaminated.evolution = advanced(&contaminated.evolution);
    assert!(matches!(
        prepare_generation_boundary(
            &source,
            &source.world,
            &contaminated,
            &source.allocators,
            elapsed,
            graph.compiled(),
        ),
        Err(GenerationTransitionError::InvalidContinuation {
            field: "evolution RNG advanced during a world step"
        })
    ));

    let mut exhausted = source.allocators.clone();
    exhausted.next_brain_id = u64::MAX - 1;
    assert!(matches!(
        prepare_generation_boundary(
            &source,
            &source.world,
            &source.rng,
            &exhausted,
            elapsed,
            graph.compiled(),
        ),
        Err(GenerationTransitionError::State(_))
    ));
    assert_eq!(source, before);
}

#[test]
fn elapsed_and_allocator_regressions_fail_before_evolution_output() {
    let graph = graph_bundle();
    let source = fixture(graph.compiled());
    assert!(matches!(
        prepare_generation_boundary(
            &source,
            &source.world,
            &source.rng,
            &source.allocators,
            source.generation.elapsed_seconds,
            graph.compiled(),
        ),
        Err(GenerationTransitionError::InvalidContinuation {
            field: "generation elapsed seconds"
        })
    ));

    let mut regressed = source.allocators.clone();
    regressed.next_entity_id -= 1;
    assert!(matches!(
        prepare_generation_boundary(
            &source,
            &source.world,
            &source.rng,
            &regressed,
            source.generation.elapsed_seconds + source.config.fixed_step_seconds,
            graph.compiled(),
        ),
        Err(GenerationTransitionError::InvalidContinuation {
            field: "entity allocator regressed"
        })
    ));
}
