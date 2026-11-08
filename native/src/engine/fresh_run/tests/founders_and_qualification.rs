use super::*;

#[cfg(feature = "engine-test-hooks")]
#[test]
fn trained_qualification_values_reuse_admitted_boxes_and_preserve_initialization_rng() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/portable-brain-v1.slither-brain"),
    )
    .unwrap();
    let artifact = crate::engine::brain_artifact::validate_brain_artifact(&bytes).unwrap();
    let contract = crate::engine::sensor_contract::SensorContractIdentity::new(
        crate::engine::sensor_contract::SensorProfile::Basic,
        8,
    )
    .unwrap();
    let mut settings = production_default_settings(55, 10);
    for setting in &mut settings {
        match setting.path.as_str() {
            "sense.bubbleBins" => setting.value = NormalizedSettingValue::Integer(8),
            "brain.controlDt" => {
                setting.value =
                    NormalizedSettingValue::Float(artifact.deployment.control_interval_seconds)
            }
            "evolvedRespawn.recurrentState" => setting.value = NormalizedSettingValue::Integer(1),
            _ => {}
        }
    }
    let mut prepared = prepare_fresh_boundary_with_sensor_and_baseline(
        request(42),
        &numeric_settings(settings),
        artifact.graph.spec().clone(),
        true,
        Some(contract),
        RunningStepWorkLimits {
            ray_work: Some(
                crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap(),
            ),
            ..RunningStepWorkLimits::provisional_defaults()
        },
        None,
    )
    .unwrap();
    let pointers: Vec<_> = prepared
        .candidate
        .population
        .iter()
        .map(|genome| genome.weights.as_ptr())
        .collect();
    let rng = prepared.candidate.rng.clone();
    let ids: Vec<_> = prepared
        .candidate
        .population
        .iter()
        .map(|genome| (genome.slot, genome.brain, genome.lineage.clone()))
        .collect();
    install_qualified_population_artifact(&mut prepared, &artifact).unwrap();
    assert_eq!(prepared.candidate.rng, rng);
    for (index, genome) in prepared.candidate.population.iter().enumerate() {
        assert_eq!(&*genome.weights, artifact.weights.as_slice());
        assert_eq!(genome.weights.as_ptr(), pointers[index]);
        assert_eq!(
            (genome.slot, genome.brain, genome.lineage.clone()),
            ids[index]
        );
    }
    assert!(prepared
        .candidate
        .brains
        .iter()
        .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));
    let mut families = vec![
        artifact,
        crate::engine::brain_artifact::validate_brain_artifact(
            &crate::engine::trained_cohort::fixture_bytes(0.125, 0.06),
        )
        .unwrap(),
    ];
    // A bad later source must reject atomically before touching even slot zero.
    let original = prepared.candidate.population[0].weights.to_vec();
    families[1].deployment.control_interval_seconds = 0.03;
    assert!(install_qualified_population_artifacts(&mut prepared, &families).is_err());
    assert_eq!(
        &*prepared.candidate.population[0].weights,
        original.as_slice()
    );
    families[1].deployment.control_interval_seconds = 0.06;
    install_qualified_population_artifacts(&mut prepared, &families).unwrap();
    assert_eq!(prepared.candidate.rng, rng);
    for (index, genome) in prepared.candidate.population.iter().enumerate() {
        assert_eq!(
            &*genome.weights,
            families[genome.slot as usize % 2].weights.as_slice()
        );
        assert_eq!(genome.weights.as_ptr(), pointers[index]);
        assert_eq!(
            (genome.slot, genome.brain, genome.lineage.clone()),
            ids[index]
        );
    }
    assert!(prepared
        .candidate
        .brains
        .iter()
        .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));
    prepared
        .candidate
        .config
        .settings
        .iter_mut()
        .find(|entry| entry.path == "brain.controlDt")
        .unwrap()
        .value = NormalizedSettingValue::Float(0.03);
    assert!(install_qualified_population_artifacts(&mut prepared, &families).is_err());
}

#[test]
fn default_population_and_rngs_match_selected_typescript_source_exactly() {
    let fixture = fixture();
    assert_eq!(
        u32::from_str_radix(fixture.seed.trim_start_matches("0x"), 16).unwrap(),
        FIXTURE_SEED
    );
    let prepared =
        prepare_stage6a_p0_boundary(request(FIXTURE_SEED), &[], typescript_default_graph_spec())
            .expect("selected P0 boundary must construct");
    let candidate = &prepared.candidate;
    assert_eq!(fixture.graph.graph_type, "graph");
    assert_eq!(prepared.graph.spec(), &fixture.graph.to_graph_spec());
    assert_eq!(
        typescript_graph_key(prepared.graph.spec()),
        fixture.typescript_graph_key
    );
    assert_eq!(
        prepared.graph.nodes.len(),
        fixture.compiled_node_ranges.len()
    );
    for (actual, expected) in prepared
        .graph
        .nodes
        .iter()
        .zip(&fixture.compiled_node_ranges)
    {
        assert_eq!(actual.id, expected.id);
        assert_eq!(
            compiled_node_type_label(actual.node_type),
            expected.node_type
        );
        assert_eq!(actual.parameter_offset, expected.offset, "{}", actual.id);
        assert_eq!(actual.parameter_length, expected.length, "{}", actual.id);
    }
    assert_eq!(prepared.graph.total_parameters, fixture.total_parameters);
    assert_eq!(
        prepared.graph.total_state_size,
        fixture.recurrent_state_floats
    );
    assert_eq!(candidate.population.len(), fixture.population_count);
    assert_eq!(candidate.rng.baselines.len(), fixture.baseline_count);
    assert_eq!(candidate.identity.seed, FIXTURE_SEED);
    assert_eq!(candidate.generation.generation, 1);
    assert_eq!(candidate.generation.completed_step, 0);
    assert_eq!(candidate.generation.population_epoch, 1);
    assert_eq!(
        candidate.config.population_count,
        STAGE6A_P0_POPULATION_COUNT
    );
    assert_eq!(candidate.config.baseline_count, STAGE6A_P0_BASELINE_COUNT);
    assert_eq!(
        candidate.config.settings.len(),
        fixture.normalized_settings.len(),
        "Rust and selected TypeScript must retain the same complete P0 setting set"
    );
    for setting in &candidate.config.settings {
        let expected = fixture
            .normalized_settings
            .get(&setting.path)
            .unwrap_or_else(|| panic!("selected TypeScript fixture lacks {}", setting.path));
        match (&setting.value, expected) {
            (
                crate::engine::state::NormalizedSettingValue::Bool(actual),
                FixtureNormalizedSetting::Bool { value },
            ) => assert_eq!(actual, value, "{}", setting.path),
            (
                crate::engine::state::NormalizedSettingValue::Integer(actual),
                FixtureNormalizedSetting::Integer { value_decimal },
            ) => assert_eq!(
                actual.to_string(),
                value_decimal.as_str(),
                "{}",
                setting.path
            ),
            (
                crate::engine::state::NormalizedSettingValue::Float(actual),
                FixtureNormalizedSetting::Float { value_hex },
            ) => assert_eq!(
                actual.to_bits(),
                fixture_float64_bits(value_hex),
                "{}",
                setting.path
            ),
            (actual, _) => panic!(
                "selected TypeScript scalar kind does not match Rust for {}: {actual:?}",
                setting.path
            ),
        }
    }

    let actual_genome_hashes = candidate
        .population
        .iter()
        .map(|genome| weight_sha256(&genome.weights))
        .collect::<Vec<_>>();
    assert_eq!(actual_genome_hashes, fixture.genome_weight_sha256);
    let mut population_hasher = Sha256::new();
    for genome in &candidate.population {
        for weight in &genome.weights {
            population_hasher.update(weight.to_le_bytes());
        }
    }
    assert_eq!(
        format!("{:x}", population_hasher.finalize()),
        fixture.population_weight_sha256
    );
    assert_eq!(candidate.rng.world.state_hex, fixture.world_state_hex);
    assert_eq!(
        candidate.rng.evolution.state_hex,
        fixture.next_evolution_state_hex
    );
    assert_eq!(
        candidate.rng.external_controller.state_hex,
        fixture.external_controller_state_hex
    );
    assert_eq!(
        candidate
            .rng
            .baselines
            .iter()
            .map(|baseline| baseline.state.state_hex.clone())
            .collect::<Vec<_>>(),
        fixture.baseline_state_hex
    );
}

#[test]
fn rust_allocates_dense_population_identity_and_returns_only_a_pending_boundary() {
    let opaque_run_id = "  aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee  ";
    let prepared = prepare_stage6a_p0_boundary(
        Stage6aP0FreshRunRequest {
            run_id: opaque_run_id.to_owned(),
            ..request(7)
        },
        &[],
        typescript_default_graph_spec(),
    )
    .expect("P0 boundary must construct");
    assert_eq!(prepared.candidate.identity.run_id, opaque_run_id);
    for (slot, (genome, brain)) in prepared
        .candidate
        .population
        .iter()
        .zip(&prepared.candidate.brains)
        .enumerate()
    {
        let expected_id = slot as u64 + 1;
        assert_eq!(genome.slot, slot as u32);
        assert_eq!(genome.brain.id, expected_id);
        assert_eq!(genome.brain.epoch, 1);
        assert_eq!(genome.lineage.genome_id, expected_id);
        assert_eq!(genome.lineage.birth_generation, 1);
        assert_eq!(genome.lineage.parent_a, None);
        assert_eq!(genome.lineage.parent_b, None);
        assert_eq!(genome.fitness.to_bits(), 0.0_f64.to_bits());
        assert_eq!(brain.handle, genome.brain);
        assert_eq!(brain.owner, BrainOwner::PopulationSlot(slot as u32));
        assert_eq!(brain.recurrent.len(), STAGE6A_P0_RECURRENT_PER_BRAIN);
        assert!(brain.recurrent.iter().all(|value| value.to_bits() == 0));
    }
    assert_eq!(prepared.candidate.allocators.next_brain_id, 56);
    assert_eq!(prepared.candidate.allocators.next_genome_id, 56);

    let pending = prepare_stage6a_p0_fresh_run(request(7))
        .expect("complete fresh run must enter the durability barrier");
    assert_eq!(pending.generation(), 1);
    assert_eq!(pending.completed_step(), 0);
    assert!(!pending.checkpoint_published());
    assert!(!pending.persistence_acknowledged());
    assert!(!pending.authority_published());
    assert_eq!(pending.snake_count(), 0);
    assert_eq!(pending.pellet_count(), 0);
    let handoff = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .expect_err("undurable authority must not move into the retained loop");
    assert!(matches!(
        handoff.error(),
        RunStartTransitionError::AuthorityNotPublished
    ));
    let pending = handoff.into_transition();
    assert_eq!(pending.generation(), 1);
    assert_eq!(pending.completed_step(), 0);
    assert!(!pending.checkpoint_published());
    assert!(!pending.persistence_acknowledged());
    assert!(!pending.authority_published());
}

fn founder_settings() -> Vec<FreshRunSettingUpdate> {
    let mut settings = numeric_fresh_settings(selected_fresh_default_settings().unwrap());
    for (path, value) in [
        ("snakeCount", 3.0),
        ("baselineBots.count", 0.0),
        ("brain.controlDt", 1.0 / 60.0),
        ("evolvedRespawn.recurrentState", 0.0),
    ] {
        settings
            .iter_mut()
            .find(|setting| setting.path == path)
            .unwrap()
            .value = value;
    }
    settings
}

#[test]
fn founder_population_uses_exact_weights_and_ordinary_mutation_at_production_sizes() {
    for upper in [false, true] {
        let bytes = crate::engine::brain_artifact::tests::large_artifact(upper);
        let artifact = crate::engine::brain_artifact::validate_brain_artifact(&bytes).unwrap();
        let prepared = prepare_configured_boundary_with_founder(
            request(42),
            &founder_settings(),
            artifact.graph.spec().clone(),
            InferenceMathBackend::Scalar,
            Some(&artifact),
        )
        .unwrap();
        let state = &prepared.candidate;
        assert_eq!(&*state.population[0].weights, &artifact.weights);
        assert_eq!(state.population[0].lineage.parent_a, None);
        let config = crate::engine::step_config::project_evolution_config(
            &state.config,
            artifact.weights.len(),
        )
        .unwrap();
        let mut rng = labelled_stream(42.0, "evolution");
        for genome in &state.population[1..] {
            let mut expected = artifact.weights.clone();
            crate::engine::evolution::mutate(
                &mut expected,
                artifact.graph.compiled(),
                config,
                &mut rng,
            )
            .unwrap();
            assert_eq!(&*genome.weights, &expected);
            assert_ne!(&*genome.weights, &artifact.weights);
            assert_eq!(
                genome.lineage.parent_a,
                Some(state.population[0].lineage.genome_id)
            );
        }
        assert_eq!(state.rng.evolution, rng.export_state());
        assert_eq!(
            state.rng.world,
            labelled_stream(42.0, "world").export_state()
        );
        assert!(state
            .brains
            .iter()
            .all(|brain| brain.recurrent.iter().all(|value| value.to_bits() == 0)));
        assert_eq!(
            state.identity.founder.as_ref().unwrap().artifact_sha256,
            artifact.artifact_sha256
        );
        let repeated = prepare_configured_boundary_with_founder(
            request(42),
            &founder_settings(),
            artifact.graph.spec().clone(),
            InferenceMathBackend::Scalar,
            Some(&artifact),
        )
        .unwrap();
        assert_eq!(state.population, repeated.candidate.population);
        assert_eq!(state.rng, repeated.candidate.rng);
    }
}

#[test]
fn founder_source_is_verified_and_checkpoint_restore_does_not_need_the_library() {
    let directory = TestDirectory::create("founder-checkpoint");
    let bytes = crate::engine::brain_artifact::tests::large_artifact(false);
    let artifact = crate::engine::brain_artifact::validate_brain_artifact(&bytes).unwrap();
    let path = directory
        .path()
        .join(format!("{}.slither-brain", artifact.artifact_sha256));
    std::fs::write(&path, &bytes).unwrap();
    let make = |input, id: &str| {
        prepare_saved_brain_founder_run(
            input,
            &founder_settings(),
            artifact.graph.spec().clone(),
            InferenceMathBackend::Scalar,
            directory.path(),
            id,
        )
    };
    let mut input = request(42);
    // A running generation transition owns both complete source and successor.
    input.memory_ceiling_bytes = 4 * 1024 * MIB;
    let mut fresh = make(input.clone(), &artifact.artifact_sha256).unwrap();
    assert!(fresh.publish_running_authority().is_err());
    let descriptor = fresh
        .publish_checkpoint(
            directory.path(),
            CheckpointOperationId::parse("12345678123456781234567812345678").unwrap(),
        )
        .unwrap();
    let metadata = fresh.startup_metadata_json().unwrap();
    let mut insufficient = request(42);
    insufficient.memory_ceiling_bytes = 1;
    assert!(make(insufficient, &artifact.artifact_sha256).is_err());
    assert!(make(request(42), "../source").is_err());
    std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    assert!(make(request(42), &artifact.artifact_sha256).is_err());
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&path, corrupt).unwrap();
    assert!(make(request(42), &artifact.artifact_sha256).is_err());
    std::fs::remove_file(&path).unwrap();
    let restored = prepare_stage6a_p0_checkpoint_restore(
        directory.path(),
        &descriptor,
        input.memory_ceiling_bytes,
    )
    .unwrap();
    assert_eq!(restored.startup_metadata_json().unwrap(), metadata);
    let source = restored.into_population_for_new_run().unwrap();
    assert_eq!(&*source.state().population[0].weights, &artifact.weights);
    assert_eq!(
        source
            .state()
            .identity
            .founder
            .as_ref()
            .unwrap()
            .weights_sha256,
        artifact.weights_sha256
    );
    fresh.acknowledge_persistence(&descriptor).unwrap();
    fresh.publish_running_authority().unwrap();
    assert_eq!(fresh.snake_count(), 3);
    let mut running = fresh
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .unwrap();
    running
        .request_finish_generation("finish-founder".into(), descriptor.run_id.clone(), 1)
        .unwrap();
    running
        .service_after_command_drain(17, SchedulerServiceMode::Background, None)
        .unwrap();
    assert!(running.pending_generation_transition().is_some());
}

#[test]
fn founder_rejects_incompatible_graph_sensors_cadence_and_life_rule() {
    let bytes = crate::engine::brain_artifact::tests::large_artifact(false);
    let artifact = crate::engine::brain_artifact::validate_brain_artifact(&bytes).unwrap();
    for (path, value, reason) in [
        ("brain.controlDt", 0.017, "cadence"),
        ("evolvedRespawn.recurrentState", 1.0, "memory"),
    ] {
        let mut settings = founder_settings();
        settings
            .iter_mut()
            .find(|setting| setting.path == path)
            .unwrap()
            .value = value;
        let error = prepare_configured_boundary_with_founder(
            request(42),
            &settings,
            artifact.graph.spec().clone(),
            InferenceMathBackend::Scalar,
            Some(&artifact),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains(reason), "{error}");
    }
    let mut wrong = crate::engine::brain_artifact::validate_brain_artifact(&bytes).unwrap();
    wrong.canonical_manifest.sensor_sha256 = "00".repeat(32);
    assert!(prepare_configured_boundary_with_founder(
        request(42),
        &founder_settings(),
        artifact.graph.spec().clone(),
        InferenceMathBackend::Scalar,
        Some(&wrong)
    )
    .is_err());
    let other = crate::engine::brain_artifact::validate_brain_artifact(
        &crate::engine::brain_artifact::tests::large_artifact(true),
    )
    .unwrap();
    assert!(prepare_configured_boundary_with_founder(
        request(42),
        &founder_settings(),
        other.graph.spec().clone(),
        InferenceMathBackend::Scalar,
        Some(&artifact)
    )
    .is_err());
}

#[test]
fn small_founder_respects_zero_mutation_settings_without_changing_the_saved_weights() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/portable-dense-basic8-v1.slither-brain"),
    )
    .unwrap();
    let artifact = crate::engine::brain_artifact::validate_brain_artifact(&bytes).unwrap();
    let mut settings = founder_settings();
    for (path, value) in [
        ("sense.profile", 0.0),
        ("sense.bubbleBins", 8.0),
        (
            "brain.controlDt",
            artifact.deployment.control_interval_seconds,
        ),
        (
            "evolvedRespawn.recurrentState",
            f64::from(
                artifact.deployment.life_state_rule
                    == crate::engine::training_graph::TrainingLifeStateRule::Preserve,
            ),
        ),
        ("mutationRate", 0.0),
        ("brain.gruMutationRate", 0.0),
    ] {
        settings
            .iter_mut()
            .find(|entry| entry.path == path)
            .unwrap()
            .value = value;
    }
    let prepared = prepare_configured_boundary_with_founder(
        request(42),
        &settings,
        artifact.graph.spec().clone(),
        InferenceMathBackend::Scalar,
        Some(&artifact),
    )
    .unwrap();
    assert!(prepared
        .candidate
        .population
        .iter()
        .all(|genome| &*genome.weights == artifact.weights.as_slice()));
    assert_eq!(
        crate::engine::brain_artifact::validate_brain_artifact(&bytes)
            .unwrap()
            .weights,
        artifact.weights
    );
}
