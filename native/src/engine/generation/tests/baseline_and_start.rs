use super::*;

#[test]
fn baseline_successor_streams_preserve_v1_v2_and_separate_v3_full_generation_scope() {
    use super::*;
    let config = BaselineGenerationConfig {
        seed: 1,
        randomize_seed_per_generation: true,
    };
    for version in [1, 2] {
        let (lifecycle, behavior) =
            derive_baseline_streams_for_generation(42, 1, 12, config, version).unwrap();
        assert_eq!(lifecycle, derive_baseline_rngs(42, 1, 12, config).unwrap());
        assert!(behavior.is_empty());
    }
    let (life1, behavior1) = derive_baseline_streams_for_generation(42, 1, 12, config, 3).unwrap();
    let (life2, behavior2) =
        derive_baseline_streams_for_generation(42, (1u64 << 32) + 1, 12, config, 3).unwrap();
    assert_ne!(life1, life2);
    assert_ne!(behavior1, behavior2);
    let fixed = BaselineGenerationConfig {
        randomize_seed_per_generation: false,
        ..config
    };
    assert_eq!(
        derive_baseline_streams_for_generation(42, 1, 12, fixed, 3).unwrap(),
        derive_baseline_streams_for_generation(42, (1u64 << 32) + 1, 12, fixed, 3).unwrap()
    );
    assert!(derive_baseline_streams_for_generation(42, 0, 12, config, 3).is_err());
    assert!(derive_baseline_streams_for_generation(42, 1, 121, config, 3).is_err());
    assert!(derive_baseline_streams_for_generation(42, 1, 12, config, 4).is_err());
}

#[test]
fn baseline_generation_seed_composition_matches_current_typescript_vectors() {
    let randomized = derive_baseline_rngs(
        42,
        8,
        3,
        BaselineGenerationConfig {
            seed: 123_456_789,
            randomize_seed_per_generation: true,
        },
    )
    .unwrap();
    let fixed = derive_baseline_rngs(
        42,
        8,
        3,
        BaselineGenerationConfig {
            seed: 123_456_789,
            randomize_seed_per_generation: false,
        },
    )
    .unwrap();
    let state = |entry: &BaselineRngState| {
        u32::from_str_radix(
            entry
                .state
                .state_hex
                .strip_prefix("0x")
                .expect("serialized state prefix"),
            16,
        )
        .expect("serialized Uint32 state")
    };
    assert_eq!(
        randomized.iter().map(state).collect::<Vec<_>>(),
        [3_990_587_595, 3_017_415_498, 633_257_913]
    );
    assert_eq!(
        fixed.iter().map(state).collect::<Vec<_>>(),
        [1_324_131_708, 3_809_538_889, 3_939_651_290]
    );
}

#[test]
fn complete_start_keeps_baseline_streams_separate_and_initializes_every_slot() {
    let graph = graph_bundle();
    let boundary_without_baselines = prepared_test_boundary(graph.compiled());
    let mut base_workspace = GenerationStartWorkspace::new();
    let base = base_workspace
        .prepare(
            &boundary_without_baselines,
            GenerationStartConfig::from_work_limits(RunningStepWorkLimits::provisional_defaults()),
        )
        .expect("baseline-free comparison world must construct");
    let base_evolved = base.world().snakes.clone();
    let base_bodies = base.world().body_points.clone();
    let base_pellets = base.world().pellets.clone();
    let base_world_rng = base.rng().world.clone();

    let mut boundary = boundary_without_baselines.clone();
    boundary.config.baseline_count = 2;
    boundary.config.settings = fixture_default_settings(boundary.config.population_count, 2);
    boundary.config.settings_schema_sha256 =
        normalized_settings_schema_hash(&boundary.config.settings).unwrap();
    boundary.identity.config_hash = normalized_config_hash(&boundary.config).unwrap();
    boundary.rng.baselines = derive_baseline_rngs(
        boundary.identity.seed,
        boundary.generation.generation,
        2,
        BaselineGenerationConfig {
            seed: 1,
            randomize_seed_per_generation: false,
        },
    )
    .unwrap();
    let baseline_rng_before = boundary.rng.baselines.clone();
    let boundary_policy = policy(&boundary);
    let boundary =
        AuthoritativeState::validate_and_own(boundary, Arc::clone(&graph), &boundary_policy)
            .expect("modified exact boundary must admit");
    let start_config =
        GenerationStartConfig::from_work_limits(RunningStepWorkLimits::provisional_defaults());
    let mut workspace = GenerationStartWorkspace::new();
    let prepared = workspace
        .prepare(boundary.state(), start_config)
        .expect("both baseline slots must construct collision-safely");

    assert_eq!(prepared.world().snakes.len(), 6);
    assert_eq!(&prepared.world().snakes[..4], base_evolved.as_slice());
    assert_eq!(
        &prepared.world().body_points[..base_bodies.len()],
        base_bodies
    );
    assert_eq!(prepared.world().pellets, base_pellets);
    assert_eq!(prepared.rng().world, base_world_rng);
    assert_eq!(prepared.fixed_step().baseline_lifecycle.slots.len(), 2);
    for (slot, source_rng) in baseline_rng_before.iter().enumerate() {
        let snake = &prepared.world().snakes[4 + slot];
        let runtime = &prepared.fixed_step().baseline_lifecycle.slots[slot];
        assert_eq!(snake.kind, SnakeKind::Baseline);
        assert_eq!(snake.baseline_slot, Some(slot as u32));
        assert_eq!(snake.baseline_strategy, Some(BaselineStrategyState::Roam));
        assert_eq!(runtime.slot, slot as u32);
        assert_eq!(runtime.snake_id, snake.id);
        assert_eq!(runtime.respawn_remaining_seconds, None);
        assert_ne!(&prepared.rng().baselines[slot], source_rng);
    }

    let mut running = boundary.state().clone();
    running.phase = AuthorityPhase::Running;
    running.world = prepared.world().clone();
    running.rng = prepared.rng().clone();
    running.allocators = prepared.allocators().clone();
    running.fixed_step = prepared.fixed_step().clone();
    let running_policy = policy(&running);
    AuthoritativeState::validate_and_own(running, Arc::clone(&graph), &running_policy)
        .expect("baseline-complete constructed world must fully admit");
}

#[test]
fn impossible_complete_body_start_is_atomic_and_reports_the_failed_work() {
    let graph = graph_bundle();
    let mut boundary = prepared_test_boundary(graph.compiled());
    boundary.config.world_radius = 800.0;
    replace_setting(
        &mut boundary,
        "worldRadius",
        NormalizedSettingValue::Integer(800),
    );
    replace_setting(
        &mut boundary,
        "snakeSpacing",
        NormalizedSettingValue::Float(20.0),
    );
    replace_setting(
        &mut boundary,
        "snakeStartLen",
        NormalizedSettingValue::Integer(140),
    );
    refresh_config_identity(&mut boundary);
    let admission = policy(&boundary);
    let boundary = AuthoritativeState::validate_and_own(boundary, Arc::clone(&graph), &admission)
        .expect("impossible placement is still a valid exact checkpoint boundary");
    let source_before = boundary.state().clone();
    let mut limits = RunningStepWorkLimits::provisional_defaults();
    limits.spawn_random_attempts_per_request = 2;
    limits.spawn_fallback_position_count = 2;
    limits.spawn_fallback_heading_count = 2;
    limits.spawn_candidates_per_request = 8;
    limits.spawn_candidates_per_batch = 32;
    limits.spawn_geometry_checks_per_batch = 100_000;
    let mut workspace = GenerationStartWorkspace::new();

    let error = workspace
        .prepare(
            boundary.state(),
            GenerationStartConfig::from_work_limits(limits),
        )
        .expect_err("a 2,780-unit straight body cannot fit in the 800-unit arena");

    let GenerationStartError::Spawn(spawn_error) = &error else {
        panic!("unexpected generation-start error: {error:?}");
    };
    assert!(
        matches!(
            spawn_error.as_ref(),
            SpawnError::NoCollisionSafePlacement {
                key: crate::engine::spawn::SpawnKey {
                    domain: SpawnDomain::Evolved,
                    slot: 0,
                },
                random_attempts: 2,
                fallback_candidates: 4,
            }
        ),
        "unexpected spawn error: {spawn_error:?}"
    );
    assert_eq!(boundary.state(), &source_before);
    assert!(!workspace.is_ready());
    assert_eq!(workspace.diagnostics().spawn.candidates_examined, 6);
}

#[test]
fn generation_start_recycles_checkpoint_frame_cursor_without_mutating_the_boundary() {
    let graph = graph_bundle();
    let mut boundary = prepared_test_boundary(graph.compiled());
    boundary.allocators.next_frame_v1_id = FRAME_V1_EXHAUSTED_ID;
    let admission = policy(&boundary);
    let boundary = AuthoritativeState::validate_and_own(boundary, Arc::clone(&graph), &admission)
        .expect("the one-past-limit frame cursor is checkpoint-valid");
    let source_before = boundary.state().clone();
    let mut workspace = GenerationStartWorkspace::new();

    let prepared = workspace
        .prepare(
            boundary.state(),
            GenerationStartConfig::from_work_limits(RunningStepWorkLimits::provisional_defaults()),
        )
        .expect("retired generation IDs must be reusable at the next boundary");

    for (index, snake) in prepared.world().snakes.iter().enumerate() {
        assert_eq!(snake.frame_v1_id as usize, index + 1);
    }
    assert_eq!(
        prepared.allocators().next_frame_v1_id as usize,
        prepared.world().snakes.len() + 1
    );
    assert_eq!(boundary.state(), &source_before);
    assert!(workspace.is_ready());
}

#[test]
fn generation_start_revalidates_source_and_reuses_warmed_storage() {
    let graph = graph_bundle();
    let boundary = prepared_test_boundary(graph.compiled());
    let default_limits = RunningStepWorkLimits::provisional_defaults();
    let config = GenerationStartConfig::from_work_limits(default_limits);
    let mut workspace = GenerationStartWorkspace::new();
    let first = workspace
        .prepare(&boundary, config)
        .expect("first generation construction must prepare");
    let first_world = first.world().clone();
    let first_rng = first.rng().clone();
    let first_allocators = first.allocators().clone();
    let first_fixed_step = first.fixed_step().clone();
    let first_diagnostics = first.diagnostics();
    let distinct_but_equal_source = boundary.clone();
    assert!(matches!(
        first.validate_current(&distinct_but_equal_source, config),
        Err(GenerationStartError::SourceChanged)
    ));
    let mut changed_limits = default_limits;
    changed_limits.spawn_geometry_checks_per_batch -= 1;
    assert!(matches!(
        first.validate_current(
            &boundary,
            GenerationStartConfig::from_work_limits(changed_limits),
        ),
        Err(GenerationStartError::ConfigChanged)
    ));

    let second = workspace
        .prepare(&boundary, config)
        .expect("warmed generation construction must prepare identically");
    assert_eq!(second.world(), &first_world);
    assert_eq!(second.rng(), &first_rng);
    assert_eq!(second.allocators(), &first_allocators);
    assert_eq!(second.fixed_step(), &first_fixed_step);
    let second_diagnostics = second.diagnostics();
    assert_eq!(
        second_diagnostics.request_capacity,
        first_diagnostics.request_capacity
    );
    assert_eq!(
        second_diagnostics.placement_capacity,
        first_diagnostics.placement_capacity
    );
    assert_eq!(
        second_diagnostics.evolved_body_capacity,
        first_diagnostics.evolved_body_capacity
    );
    assert_eq!(
        second_diagnostics.snake_capacity,
        first_diagnostics.snake_capacity
    );
    assert_eq!(
        second_diagnostics.body_point_capacity,
        first_diagnostics.body_point_capacity
    );
    assert_eq!(
        second_diagnostics.pellet_capacity,
        first_diagnostics.pellet_capacity
    );
}
