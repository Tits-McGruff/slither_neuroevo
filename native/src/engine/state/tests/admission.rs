use super::*;

#[test]
fn arithmetic_admission_accepts_mixed_but_rejects_unknown_or_duplicate_labels() {
    let mut admission = policy(128 * 1024 * 1024);
    admission.allowed_math_backends = vec![
        "rust-sse2-v1".into(),
        "rust-i16-matrix-fma128-v1".into(),
        "rust-i16-affine-fma128-sse2-v1".into(),
    ];
    validate_policy(&admission).unwrap();
    admission.allowed_math_backends[2] = "unknown-arithmetic-v1".into();
    assert!(validate_policy(&admission).is_err());
    admission.allowed_math_backends[2] = "rust-sse2-v1".into();
    assert!(validate_policy(&admission).is_err());
    admission.allowed_math_backends[2] = admission.expected_math_backend.clone();
    assert!(validate_policy(&admission).is_err());
}

#[test]
fn default_population_owns_distinct_genomes_and_stable_brain_state() {
    let graph = default_graph();
    let candidate = candidate(&graph, 55);
    let estimate = estimate_state_memory(&candidate, &graph).unwrap();
    let state = own(candidate, Arc::clone(&graph), estimate.total_bytes).unwrap();

    assert_eq!(graph.total_parameters, 13_458);
    assert_eq!(graph.total_state_size, 16);
    assert_eq!(state.state().population.len(), 55);
    assert_ne!(
        state.state().population[0].weights[0],
        state.state().population[1].weights[0]
    );
    assert_eq!(
        state.state().brains[54].handle,
        state.state().population[54].brain
    );
    assert_eq!(state.graph().architecture_key, graph.architecture_key);
    assert_eq!(state.graph_spec(), graph.spec());
    assert_eq!(state.graph_bundle().compiled(), graph.compiled());
    let boundary = state.checkpoint_boundary().unwrap();
    assert_eq!(boundary.graph_spec(), graph.spec());
    assert_eq!(boundary.graph(), graph.compiled());
    assert_eq!(boundary.graph_bundle().compiled(), graph.compiled());
    assert!(
        estimate.structural_bytes >= size_of::<Mutex<AuthoritativeState>>() + ARC_COUNTER_BYTES
    );
    assert!(estimate.graph_bytes >= size_of::<GraphBundle>() + ARC_COUNTER_BYTES);
    assert_eq!(state.memory_estimate(), estimate);
}

#[test]
fn compatible_build_admission_rebinds_only_after_a_distinct_recovery_branch() {
    let graph = default_graph();
    let mut compatible = policy(usize::MAX);
    compatible.require_exact_build_identity = false;
    compatible.expected_source_revision = "next-source".into();
    compatible.expected_engine_build_id = "next-engine".into();
    compatible.expected_source_sha256 = "2".repeat(64);
    compatible.expected_rustc_version = "rustc next".into();
    compatible.expected_build_contract_sha256 = format!("sha256:{}", "4".repeat(64));

    let source = candidate(&graph, 2);
    let original_run = source.identity.run_id.clone();
    let admitted = AuthoritativeState::validate_and_own(source, Arc::clone(&graph), &compatible)
        .expect("version/target/math-compatible state must admit privately");
    assert_eq!(admitted.state().identity.run_id, original_run);
    assert_ne!(
        admitted.state().identity.source_revision,
        compatible.expected_source_revision
    );

    let branch = admitted
        .into_recovery_branch("compatible-build-branch".into(), &compatible)
        .expect("durable branch adoption must rebind producer identity");
    assert_eq!(branch.state().identity.run_id, "compatible-build-branch");
    assert_eq!(
        branch.state().identity.source_revision,
        compatible.expected_source_revision
    );
    assert_eq!(
        branch.state().identity.engine_build_id,
        compatible.expected_engine_build_id
    );
    assert_eq!(
        branch.state().identity.source_sha256,
        compatible.expected_source_sha256
    );

    let mut incompatible = compatible;
    incompatible.expected_target_triple = "different-target".into();
    assert!(matches!(
        AuthoritativeState::validate_and_own(candidate(&graph, 2), graph, &incompatible),
        Err(StateError::InvalidField {
            field: "identity",
            ..
        })
    ));
}

#[test]
fn checkpoint_view_rejects_running_state_and_frame_reservation_is_atomic() {
    let graph = default_graph();
    let boundary = own(candidate(&graph, 1), Arc::clone(&graph), usize::MAX).unwrap();
    assert_eq!(
        boundary.checkpoint_boundary().unwrap().kind(),
        GenerationBoundaryKind::RunStart
    );

    let mut running = candidate(&graph, 1);
    running.phase = AuthorityPhase::Running;
    let running = own(running, graph, usize::MAX).unwrap();
    assert!(running.checkpoint_boundary().is_err());

    let mut allocators = running.state().allocators.clone();
    allocators.next_frame_v1_id = FRAME_V1_MAX_EXACT_ID - 1;
    assert_eq!(
        allocators
            .reserve_frame_v1_ids(2, std::iter::empty())
            .unwrap(),
        Some(FrameV1IdReservation {
            first: FRAME_V1_MAX_EXACT_ID - 1,
            last: FRAME_V1_MAX_EXACT_ID,
        })
    );
    assert_eq!(allocators.next_frame_v1_id, FRAME_V1_EXHAUSTED_ID);
    assert_eq!(
        allocators
            .reserve_frame_v1_ids(1, [1, 2, FRAME_V1_MAX_EXACT_ID].into_iter())
            .unwrap(),
        Some(FrameV1IdReservation { first: 3, last: 3 })
    );
    let before = allocators.clone();
    assert!(matches!(
        allocators.reserve_frame_v1_ids(FRAME_V1_EXHAUSTED_ID, std::iter::empty()),
        Err(StateError::IdExhausted {
            kind: "frame-v1",
            requested
        }) if requested == u64::from(FRAME_V1_EXHAUSTED_ID)
    ));
    assert_eq!(allocators, before);
}

#[test]
fn rejects_invalid_lengths_nonfinite_values_and_duplicate_slots_or_ids() {
    let graph = default_graph();

    let mut bad_slot = candidate(&graph, 2);
    bad_slot.population[1].slot = 0;
    assert!(matches!(
        own(bad_slot, Arc::clone(&graph), usize::MAX),
        Err(StateError::NonDensePopulationSlot { .. })
    ));

    let mut bad_weights = candidate(&graph, 2);
    bad_weights.population[0].weights = vec![0.0; graph.total_parameters - 1].into_boxed_slice();
    assert!(matches!(
        own(bad_weights, Arc::clone(&graph), usize::MAX),
        Err(StateError::WeightLength { .. })
    ));

    let mut bad_recurrent = candidate(&graph, 2);
    bad_recurrent.brains[0].recurrent = vec![0.0; graph.total_state_size + 1].into_boxed_slice();
    assert!(matches!(
        own(bad_recurrent, Arc::clone(&graph), usize::MAX),
        Err(StateError::RecurrentLength { .. })
    ));

    let mut nonfinite = candidate(&graph, 2);
    nonfinite.population[0].weights[7] = f32::NAN;
    assert!(matches!(
        own(nonfinite, Arc::clone(&graph), usize::MAX),
        Err(StateError::NonFinite {
            field: "population.weights",
            ..
        })
    ));

    let mut duplicate_id = candidate(&graph, 2);
    duplicate_id.population[1].lineage.genome_id = duplicate_id.population[0].lineage.genome_id;
    assert!(matches!(
        own(duplicate_id, Arc::clone(&graph), usize::MAX),
        Err(StateError::DuplicateId { kind: "genome", .. })
    ));

    let mut duplicate_handle = candidate(&graph, 2);
    duplicate_handle.population[1].brain = duplicate_handle.population[0].brain;
    assert!(matches!(
        own(duplicate_handle, graph, usize::MAX),
        Err(StateError::DuplicateBrainHandle(_))
    ));
}

#[test]
fn exact_boundary_requires_zero_state_and_no_spawned_world() {
    let graph = default_graph();
    let clean = candidate(&graph, 2);
    assert!(own(clean, Arc::clone(&graph), usize::MAX).is_ok());

    let mut advanced_run_start = candidate(&graph, 2);
    advanced_run_start.generation.completed_step = 1;
    assert!(matches!(
        own(advanced_run_start, Arc::clone(&graph), usize::MAX),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));

    let mut unadvanced_generation = candidate(&graph, 2);
    unadvanced_generation.phase =
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation);
    unadvanced_generation.generation.generation = 2;
    assert!(matches!(
        own(
            unadvanced_generation.clone(),
            Arc::clone(&graph),
            usize::MAX
        ),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));
    unadvanced_generation.generation.completed_step = 1;
    assert!(own(unadvanced_generation, Arc::clone(&graph), usize::MAX).is_ok());

    let mut dirty_recurrent = candidate(&graph, 2);
    dirty_recurrent.brains[0].recurrent[0] = 0.5;
    assert!(matches!(
        own(dirty_recurrent, Arc::clone(&graph), usize::MAX),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));

    let mut dirty_time = candidate(&graph, 2);
    dirty_time.generation.elapsed_seconds = 1.0 / 60.0;
    assert!(matches!(
        own(dirty_time, graph, usize::MAX),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));
}

#[test]
fn fixed_step_continuation_is_owned_live_and_reset_at_checkpoint_boundaries() {
    let graph = default_graph();

    let mut dirty_ambient = candidate(&graph, 1);
    dirty_ambient.fixed_step.ambient_pellet_accumulator = 0.25;
    assert!(matches!(
        own(dirty_ambient, Arc::clone(&graph), usize::MAX),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));

    let mut donor = candidate(&graph, 1);
    donor.phase = AuthorityPhase::Running;
    push_evolved_snake(&mut donor, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
    donor.world.snakes[0].points = 12.5;
    let mut sensor_generation = SensorGenerationState::new();
    sensor_generation
        .update_after_step(&donor.world)
        .expect("finite evolved score should update generation best");
    let mut dirty_sensor = candidate(&graph, 1);
    dirty_sensor.fixed_step.sensor_generation = sensor_generation;
    assert!(matches!(
        own(dirty_sensor, Arc::clone(&graph), usize::MAX),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));

    let mut dirty_baseline = candidate(&graph, 1);
    dirty_baseline
        .fixed_step
        .baseline_lifecycle
        .slots
        .push(BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: BASELINE_ENTITY_ID_START,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: None,
        });
    assert!(matches!(
        own(dirty_baseline, Arc::clone(&graph), usize::MAX),
        Err(StateError::DirtyGenerationBoundary { .. })
    ));

    let mut missing_live_baseline = candidate(&graph, 1);
    missing_live_baseline.phase = AuthorityPhase::Running;
    enable_one_live_baseline(&mut missing_live_baseline, BASELINE_ENTITY_ID_START, None);
    assert!(matches!(
        own(missing_live_baseline, graph, usize::MAX),
        Err(StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            ..
        })
    ));
}

#[test]
fn running_world_rejects_duplicate_slots_brains_and_incoherent_bodies() {
    let graph = default_graph();
    let mut duplicate = candidate(&graph, 2);
    duplicate.phase = AuthorityPhase::Running;
    push_evolved_snake(&mut duplicate, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
    push_evolved_snake(&mut duplicate, 0, 2, 2, WorldPoint { x: 3.0, y: 4.0 });
    assert!(matches!(
        own(duplicate, Arc::clone(&graph), usize::MAX),
        Err(StateError::DuplicateId {
            kind: "world population slot",
            ..
        })
    ));

    let mut wrong_head = candidate(&graph, 1);
    wrong_head.phase = AuthorityPhase::Running;
    push_evolved_snake(&mut wrong_head, 0, 1, 1, WorldPoint { x: 1.0, y: 2.0 });
    wrong_head.world.body_points[0].x = 9.0;
    assert!(matches!(
        own(wrong_head, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidBodyRange { .. })
    ));

    let mut empty_alive_body = candidate(&graph, 1);
    empty_alive_body.phase = AuthorityPhase::Running;
    push_evolved_snake(
        &mut empty_alive_body,
        0,
        1,
        1,
        WorldPoint { x: 1.0, y: 2.0 },
    );
    empty_alive_body.world.snakes[0].body.len = 0;
    assert!(matches!(
        own(empty_alive_body, graph, usize::MAX),
        Err(StateError::InvalidBodyRange { .. })
    ));
}

#[test]
fn normalized_wall_death_movement_is_admitted_as_authoritative_state() {
    let graph = default_graph();
    let mut running = candidate(&graph, 1);
    running.phase = AuthorityPhase::Running;
    let start = WorldPoint {
        x: running.config.world_radius - 8.4,
        y: 0.0,
    };
    push_evolved_snake(&mut running, 0, 1, 1, start);

    let mut movement_config = MovementConfig::typescript_defaults();
    movement_config.world_radius = running.config.world_radius;
    let mut workspace = MovementWorkspace::new();
    let prepared = workspace
        .prepare(
            &running.world,
            movement_config,
            1.0 / 180.0,
            running.config.max_body_points,
            running.config.max_pellets,
        )
        .expect("wall movement should prepare");
    assert!(prepared.proposals()[0].wall_death);
    let staged_snakes = prepared.snakes().to_vec();
    let staged_body_points = prepared.body_points().to_vec();
    assert!(!staged_snakes[0].alive);
    assert_eq!(staged_body_points[0], staged_snakes[0].position);

    running.world.snakes = staged_snakes;
    running.world.body_points = staged_body_points;
    assert!(own(running, graph, usize::MAX).is_ok());
}
