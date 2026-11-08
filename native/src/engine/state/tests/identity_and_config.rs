use super::*;

#[test]
fn exact_contract_identity_config_projection_and_hash_are_required() {
    let graph = default_graph();

    let mut wrong_version = candidate(&graph, 2);
    wrong_version.versions.protocol += 1;
    assert!(matches!(
        own(wrong_version, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "versions.protocol",
            ..
        })
    ));

    let mismatches: [fn(&mut StateAdmissionPolicy); 9] = [
        |policy| policy.expected_source_revision = "stale-source".into(),
        |policy| policy.expected_engine_build_id = "stale-engine".into(),
        |policy| {
            policy.expected_source_sha256 =
                "2222222222222222222222222222222222222222222222222222222222222222".into();
        },
        |policy| policy.expected_target_triple = "x86_64-unknown-linux-gnu".into(),
        |policy| policy.expected_build_profile = "debug".into(),
        |policy| policy.expected_build_class = "test-hooks".into(),
        |policy| policy.expected_rustc_version = "rustc other".into(),
        |policy| {
            policy.expected_build_contract_sha256 =
                "sha256:4444444444444444444444444444444444444444444444444444444444444444".into();
        },
        |policy| policy.expected_math_backend = "other-math".into(),
    ];
    for mismatch in mismatches {
        let mut wrong_policy = policy(usize::MAX);
        mismatch(&mut wrong_policy);
        assert!(matches!(
            AuthoritativeState::validate_and_own(
                candidate(&graph, 2),
                Arc::clone(&graph),
                &wrong_policy
            ),
            Err(StateError::InvalidField {
                field: "identity",
                ..
            })
        ));
    }

    let mut malformed_source_sha = candidate(&graph, 2);
    malformed_source_sha.identity.source_sha256 = "ABC".into();
    assert!(matches!(
        own(malformed_source_sha, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "identity.source_sha256",
            ..
        })
    ));

    let mut contradictory = candidate(&graph, 2);
    let snake_count = contradictory
        .config
        .settings
        .iter_mut()
        .find(|setting| setting.path == "snakeCount")
        .unwrap();
    snake_count.value = NormalizedSettingValue::Integer(99);
    refresh_config_hash(&mut contradictory);
    assert!(matches!(
        own(contradictory, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "config.settings",
            ..
        })
    ));

    let mut wrong_hash = candidate(&graph, 2);
    wrong_hash.identity.config_hash = "sha256:wrong".into();
    assert!(matches!(
        own(wrong_hash, graph, usize::MAX),
        Err(StateError::InvalidField {
            field: "identity.config_hash",
            ..
        })
    ));
}

#[test]
fn settings_schema_rejects_omissions_unknown_paths_and_type_changes() {
    let graph = default_graph();
    assert_eq!(
        normalized_settings_schema_hash(&test_settings(1)).unwrap(),
        normalized_settings_schema_hash(&test_settings(99)).unwrap()
    );

    let mut omitted = candidate(&graph, 2);
    omitted.config.settings.remove(0);
    refresh_config_hash(&mut omitted);
    assert!(matches!(
        own(omitted, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "config.settings_schema_sha256",
            ..
        })
    ));

    let mut unknown = candidate(&graph, 2);
    unknown.config.settings.insert(
        0,
        NormalizedSetting {
            path: "aaa.unknown".into(),
            value: NormalizedSettingValue::Bool(false),
        },
    );
    refresh_config_hash(&mut unknown);
    assert!(matches!(
        own(unknown, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "config.settings_schema_sha256",
            ..
        })
    ));

    let mut wrong_kind = candidate(&graph, 2);
    wrong_kind
        .config
        .settings
        .iter_mut()
        .find(|setting| setting.path == "simSpeed")
        .unwrap()
        .value = NormalizedSettingValue::Integer(1);
    refresh_config_hash(&mut wrong_kind);
    assert!(matches!(
        own(wrong_kind, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "config.settings_schema_sha256",
            ..
        })
    ));

    let exact = candidate(&graph, 2);
    let mut wrong_policy = policy(usize::MAX);
    wrong_policy.expected_settings_schema_sha256 =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222".into();
    assert!(matches!(
        AuthoritativeState::validate_and_own(exact, graph, &wrong_policy),
        Err(StateError::InvalidField {
            field: "config.settings_schema_sha256",
            ..
        })
    ));
}

#[test]
fn frame_id_wrap_skips_occupied_ranges_without_changing_durable_ids() {
    let graph = default_graph();
    let mut allocators = candidate(&graph, 1).allocators;
    allocators.next_frame_v1_id = FRAME_V1_MAX_EXACT_ID;
    let durable_entity = allocators.next_entity_id;
    let durable_lease = allocators.next_controller_lease_id;
    let range = allocators
        .reserve_frame_v1_ids(3, [1, 3, 4, 6, FRAME_V1_MAX_EXACT_ID].into_iter())
        .unwrap()
        .unwrap();
    assert_eq!(range, FrameV1IdReservation { first: 7, last: 9 });
    assert_eq!(allocators.next_frame_v1_id, 10);
    assert_eq!(allocators.next_entity_id, durable_entity);
    assert_eq!(allocators.next_controller_lease_id, durable_lease);
}

#[test]
fn frame_id_wrap_can_reserve_a_gap_crossing_the_original_cursor() {
    let graph = default_graph();
    let mut allocators = candidate(&graph, 1).allocators;
    allocators.next_frame_v1_id = 3;
    let range = allocators
        .reserve_frame_v1_ids(
            FRAME_V1_MAX_EXACT_ID - 3,
            [FRAME_V1_MAX_EXACT_ID - 1, FRAME_V1_MAX_EXACT_ID].into_iter(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(range.first, 1);
    assert_eq!(range.last, FRAME_V1_MAX_EXACT_ID - 3);
}

#[test]
fn allocator_lineage_domains_and_exhaustion_are_checked() {
    let graph = default_graph();
    let mut zero_parent = candidate(&graph, 1);
    zero_parent.population[0].lineage.parent_a = Some(0);
    assert!(matches!(
        own(zero_parent, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "population.lineage.parent",
            ..
        })
    ));

    let mut stale_genome_allocator = candidate(&graph, 1);
    stale_genome_allocator.allocators.next_genome_id =
        stale_genome_allocator.population[0].lineage.genome_id;
    assert!(matches!(
        own(stale_genome_allocator, Arc::clone(&graph), usize::MAX),
        Err(StateError::InvalidField {
            field: "allocators.next_genome_id",
            ..
        })
    ));

    let mut bad_domain = candidate(&graph, 1);
    bad_domain.allocators.next_external_id = 100_000;
    assert!(matches!(
        own(bad_domain, graph, usize::MAX),
        Err(StateError::InvalidField {
            field: "allocators.next_external_id",
            ..
        })
    ));

    let mut allocators = candidate(&default_graph(), 1).allocators;
    allocators.next_genome_id = u64::MAX - 1;
    assert_eq!(
        allocators.reserve_genome_ids(1).unwrap(),
        Some(InternalIdReservation {
            first: u64::MAX - 1,
            last: u64::MAX - 1,
        })
    );
    let exhausted = allocators.clone();
    assert!(matches!(
        allocators.reserve_genome_ids(1),
        Err(StateError::IdExhausted {
            kind: "genome",
            requested: 1
        })
    ));
    assert_eq!(allocators, exhausted);
}

#[test]
fn baseline_slots_require_matching_unique_rng_state() {
    let graph = default_graph();
    let mut live = candidate(&graph, 1);
    live.phase = AuthorityPhase::Running;
    let position = WorldPoint { x: 4.0, y: 5.0 };
    live.world.body_points.push(position);
    live.world.snakes.push(SnakeState {
        id: BASELINE_ENTITY_ID_START,
        frame_v1_id: 1,
        kind: SnakeKind::Baseline,
        alive: true,
        population_slot: None,
        brain: None,
        baseline_slot: Some(0),
        baseline_strategy: Some(BaselineStrategyState::Roam),
        position,
        previous_position: position,
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
        body: BodyRange { start: 0, len: 1 },
        skin: 0,
    });
    live.allocators.next_baseline_id = BASELINE_ENTITY_ID_START + 1;
    live.allocators.next_frame_v1_id = 2;
    enable_one_live_baseline(&mut live, BASELINE_ENTITY_ID_START, None);
    assert!(own(live.clone(), Arc::clone(&graph), usize::MAX).is_ok());

    live.world.snakes[0].baseline_slot = Some(1);
    assert!(matches!(
        own(live, graph, usize::MAX),
        Err(StateError::InvalidField {
            field: "world.snakes.baseline_slot",
            ..
        })
    ));
}

#[test]
fn malformed_rng_is_rejected_without_partial_state() {
    let graph = default_graph();
    let mut invalid = candidate(&graph, 1);
    invalid.rng.world.state_hex = "0x00000000".into();
    assert!(matches!(
        own(invalid, graph, usize::MAX),
        Err(StateError::InvalidRng { stream, .. }) if stream == "world"
    ));
}
