use super::*;

#[test]
fn running_step_publication_swaps_every_mutable_continuation_atomically() {
    let graph = default_graph();
    let source = running_candidate(&graph);
    let source_snapshot = source.clone();
    let mut authority = own(source, graph, usize::MAX).expect("running state must admit");
    let key = authority
        .begin_running_step()
        .expect("running step must begin");

    let mut world = authority.state().world.clone();
    world.snakes[0].position = WorldPoint { x: 11.0, y: 20.0 };
    world.body_points[0] = world.snakes[0].position;
    let mut rng = authority.state().rng.clone();
    let mut world_stream = StatefulRng::from_state(&rng.world).expect("source RNG must decode");
    let _ = world_stream.next_f64();
    rng.world = world_stream.export_state();
    let mut allocators = authority.state().allocators.clone();
    allocators.next_entity_id += 1;
    let mut brains = authority.state().brains.clone();
    brains[0].recurrent[0] = 0.25;
    let mut evolved_generation = authority.state().fixed_step.evolved_generation.clone();
    let mut baseline_lifecycle = authority.state().fixed_step.baseline_lifecycle.clone();
    let sensor_generation = SensorGenerationState::new();

    let publication = authority
        .publish_running_step(RunningStepReplacement {
            key,
            world: &mut world,
            rng: &mut rng,
            allocators: &mut allocators,
            brains: &mut brains,
            evolved_generation: &mut evolved_generation,
            baseline_lifecycle: &mut baseline_lifecycle,
            ambient_pellet_accumulator: 0.75,
            sensor_generation,
            generation_elapsed_seconds: 1.0 / 60.0,
            wall_accumulator_seconds: 0.125,
            mutation: RunningStepMutationContract::default(),
        })
        .expect("complete valid step must publish");

    assert_eq!(publication.key, key);
    assert_eq!(publication.completed_step, 1);
    assert_eq!(publication.memory, authority.memory_estimate());
    assert_eq!(authority.state().generation.completed_step, 1);
    assert_eq!(authority.state().generation.elapsed_seconds, 1.0 / 60.0);
    assert_eq!(authority.state().generation.wall_accumulator_seconds, 0.125);
    assert_eq!(authority.state().world.snakes[0].position.x, 11.0);
    assert_eq!(authority.state().allocators.next_entity_id, 3);
    assert_eq!(authority.state().brains[0].recurrent[0], 0.25);
    assert_eq!(
        authority.state().fixed_step.ambient_pellet_accumulator,
        0.75
    );
    assert_eq!(
        authority.state().fixed_step.sensor_generation,
        sensor_generation
    );

    assert_eq!(world, source_snapshot.world);
    assert_eq!(rng, source_snapshot.rng);
    assert_eq!(allocators, source_snapshot.allocators);
    assert_eq!(brains, source_snapshot.brains);
    assert_eq!(
        baseline_lifecycle,
        source_snapshot.fixed_step.baseline_lifecycle
    );
}

#[test]
fn running_step_rejects_every_stale_identity_before_any_swap() {
    let graph = default_graph();
    let mut authority =
        own(running_candidate(&graph), graph, usize::MAX).expect("running state must admit");
    let key = authority
        .begin_running_step()
        .expect("running step must begin");
    let source = authority.state().clone();
    let mut buffers = RunningStepBuffers::from_state(authority.state());
    let stale = [
        (
            PhysicsStepKey::new(
                key.world_epoch() + 1,
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKeyField::WorldEpoch,
        ),
        (
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation() + 1,
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKeyField::Generation,
        ),
        (
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step() + 1,
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKeyField::SourceCompletedStep,
        ),
        (
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch() + 1,
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKeyField::PopulationEpoch,
        ),
        (
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision() + 1,
                key.config_hash(),
                key.operation_epoch(),
            ),
            PhysicsStepKeyField::ConfigRevision,
        ),
        (
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                [0x55; 32],
                key.operation_epoch(),
            ),
            PhysicsStepKeyField::ConfigHash,
        ),
        (
            PhysicsStepKey::new(
                key.world_epoch(),
                key.generation(),
                key.source_completed_step(),
                key.population_epoch(),
                key.config_revision(),
                key.config_hash(),
                key.operation_epoch() + 1,
            ),
            PhysicsStepKeyField::OperationEpoch,
        ),
    ];

    for (stale_key, expected_field) in stale {
        assert_eq!(
            authority.publish_running_step(buffers.replacement(stale_key)),
            Err(StateError::StaleFixedStep {
                field: expected_field
            })
        );
        assert_eq!(authority.state(), &source);
        assert_eq!(buffers.world, source.world);
        assert_eq!(buffers.brains, source.brains);
    }

    let newer_key = authority
        .begin_running_step()
        .expect("newer attempt must begin");
    assert_eq!(
        authority.publish_running_step(buffers.replacement(key)),
        Err(StateError::StaleFixedStep {
            field: PhysicsStepKeyField::OperationEpoch
        })
    );
    authority
        .publish_running_step(buffers.replacement(newer_key))
        .expect("current exact key must publish");
}

#[test]
fn running_step_rejects_immutable_or_regressing_continuations_before_swap() {
    let graph = default_graph();
    let mut source = running_candidate(&graph);
    push_external_snake(
        &mut source,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        WorldPoint { x: 30.0, y: 40.0 },
    );
    source.world.snakes[0].points = 5.0;
    source
        .fixed_step
        .sensor_generation
        .update_after_step(&source.world)
        .expect("source generation best must update");
    let mut authority = own(source.clone(), graph, usize::MAX)
        .expect("running source with external brain must admit");
    let key = authority
        .begin_running_step()
        .expect("running step must begin");

    let mut wrong_elapsed = RunningStepBuffers::from_state(authority.state());
    wrong_elapsed.generation_elapsed_seconds += 0.001;
    assert!(matches!(
        authority.publish_running_step(wrong_elapsed.replacement(key)),
        Err(StateError::InvalidField {
            field: "fixed_step.generation_elapsed_seconds",
            ..
        })
    ));

    let mut regressed_sensor = RunningStepBuffers::from_state(authority.state());
    regressed_sensor.sensor_generation.reset();
    assert!(matches!(
        authority.publish_running_step(regressed_sensor.replacement(key)),
        Err(StateError::InvalidField {
            field: "fixed_step.sensor_generation",
            ..
        })
    ));

    let mut wrong_rng = RunningStepBuffers::from_state(authority.state());
    let mut evolution =
        StatefulRng::from_state(&wrong_rng.rng.evolution).expect("evolution stream must decode");
    let _ = evolution.next_f64();
    wrong_rng.rng.evolution = evolution.export_state();
    assert!(matches!(
        authority.publish_running_step(wrong_rng.replacement(key)),
        Err(StateError::InvalidField {
            field: "fixed_step.rng",
            ..
        })
    ));

    let mut wrong_allocator = RunningStepBuffers::from_state(authority.state());
    wrong_allocator.allocators.next_genome_id += 1;
    assert!(matches!(
        authority.publish_running_step(wrong_allocator.replacement(key)),
        Err(StateError::InvalidField {
            field: "fixed_step.allocators",
            ..
        })
    ));

    let mut wrong_weights = RunningStepBuffers::from_state(authority.state());
    let external_weights = wrong_weights.brains[1]
        .non_population_weights
        .as_mut()
        .expect("external brain must own weights");
    external_weights[0] = 0.5;
    assert!(matches!(
        authority.publish_running_step(wrong_weights.replacement(key)),
        Err(StateError::InvalidField {
            field: "fixed_step.brains",
            ..
        })
    ));

    assert_eq!(authority.state(), &source);
    assert_eq!(authority.state().generation.completed_step, 0);
}

#[test]
fn running_step_accepts_external_gameplay_rng_continuation() {
    let graph = default_graph();
    let mut source = running_candidate(&graph);
    push_external_snake(
        &mut source,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        WorldPoint { x: 30.0, y: 40.0 },
    );
    let source_external_rng = source.rng.external_controller.clone();
    let mut authority =
        own(source, graph, usize::MAX).expect("running source with external brain must admit");
    let key = authority
        .begin_running_step()
        .expect("running step must begin");
    let mut buffers = RunningStepBuffers::from_state(authority.state());
    let mut external = StatefulRng::from_state(&buffers.rng.external_controller)
        .expect("external gameplay stream must decode");
    let _ = external.next_f64();
    let _ = external.next_f64();
    buffers.rng.external_controller = external.export_state();

    authority
        .publish_running_step(buffers.replacement(key))
        .expect("ordinary external effects may advance their isolated gameplay stream");
    assert_ne!(
        authority.state().rng.external_controller,
        source_external_rng
    );
}

#[test]
fn running_step_validation_and_memory_failure_restore_authority_and_scratch() {
    let graph = default_graph();
    let source = running_candidate(&graph);
    let exact_ceiling = estimate_state_memory(&source, &graph)
        .expect("source memory must estimate")
        .total_bytes;
    let mut authority =
        own(source.clone(), graph, exact_ceiling).expect("source must fit its exact estimate");
    let source_memory = authority.memory_estimate();

    let invalid_key = authority
        .begin_running_step()
        .expect("invalid attempt must begin");
    let mut invalid = RunningStepBuffers::from_state(authority.state());
    invalid.world.snakes[0].position.x = f64::NAN;
    assert!(matches!(
        authority.publish_running_step(invalid.replacement(invalid_key)),
        Err(StateError::NonFinite {
            field: "world.snakes.position",
            ..
        })
    ));
    assert_eq!(authority.state(), &source);
    assert_eq!(authority.memory_estimate(), source_memory);
    assert!(invalid.world.snakes[0].position.x.is_nan());

    let invalid_recurrent_key = authority
        .begin_running_step()
        .expect("invalid recurrent attempt must begin");
    let mut invalid_recurrent = RunningStepBuffers::from_state(authority.state());
    invalid_recurrent.brains[0].recurrent[0] = f32::NAN;
    assert!(matches!(
        authority.publish_running_step(invalid_recurrent.replacement(invalid_recurrent_key)),
        Err(StateError::NonFinite {
            field: "brains.recurrent",
            ..
        })
    ));
    assert_eq!(authority.state(), &source);
    assert_eq!(authority.memory_estimate(), source_memory);
    assert!(invalid_recurrent.brains[0].recurrent[0].is_nan());

    let oversized_key = authority
        .begin_running_step()
        .expect("oversized attempt must begin");
    let mut oversized = RunningStepBuffers::from_state(authority.state());
    oversized
        .world
        .pellets
        .try_reserve_exact(100_000)
        .expect("test reserve must succeed");
    let oversized_capacity = oversized.world.pellets.capacity();
    assert!(matches!(
        authority.publish_running_step(oversized.replacement(oversized_key)),
        Err(StateError::MemoryCeilingExceeded { .. })
    ));
    assert_eq!(authority.state(), &source);
    assert_eq!(authority.memory_estimate(), source_memory);
    assert_eq!(oversized.world.pellets.capacity(), oversized_capacity);

    let valid_key = authority
        .begin_running_step()
        .expect("valid retry must begin");
    let mut valid = RunningStepBuffers::from_state(authority.state());
    authority
        .publish_running_step(valid.replacement(valid_key))
        .expect("later valid step must publish");
    assert_eq!(authority.state().generation.completed_step, 1);
}

#[test]
fn invalid_life_ledger_rolls_back_world_and_continuation_in_publication_and_preflight() {
    let graph = default_graph();
    let mut source = running_candidate(&graph);
    source.fixed_step.evolved_generation =
        crate::engine::evolved_lives::EvolvedGenerationState::initialize(
            &source.world,
            source.population.len(),
            5,
        )
        .unwrap();
    let mut authority = own(source.clone(), graph, usize::MAX).unwrap();
    for preflight in [false, true] {
        let key = authority.begin_running_step().unwrap();
        let mut buffers = RunningStepBuffers::from_state(authority.state());
        buffers.evolved_generation.slots[0]
            .ledger
            .generation_gross_earned += 1.0;
        let invalid_ledger = buffers.evolved_generation.clone();
        let result = if preflight {
            authority
                .preflight_running_step(&mut buffers.replacement(key))
                .map(|_| ())
        } else {
            authority
                .publish_running_step(buffers.replacement(key))
                .map(|_| ())
        };
        assert!(result.is_err());
        assert_eq!(authority.state(), &source);
        assert_eq!(buffers.evolved_generation, invalid_ledger);
    }
}
