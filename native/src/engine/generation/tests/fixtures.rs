use super::*;

pub(super) static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

pub(super) struct TestDirectory {
    pub(super) path: PathBuf,
}

impl TestDirectory {
    pub(super) fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "slither-generation-boundary-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("generation test directory must be unique");
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let temporary_root = std::env::temp_dir().canonicalize();
        let owned = self.path.canonicalize();
        if temporary_root
            .as_ref()
            .ok()
            .zip(owned.as_ref().ok())
            .is_some_and(|(root, path)| path.starts_with(root) && path != root)
        {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 8,
        max_edges: 8,
        max_graph_outputs: 2,
        max_identifier_bytes: 32,
        max_total_referenced_identifier_bytes: 256,
        max_tensor_width: 128,
        max_mlp_hidden_layers: 4,
        max_split_output_ports: 4,
        max_parameter_floats: 10_000,
        max_recurrent_state_floats: 128,
        max_canonical_layout_bytes: 16_384,
        max_architecture_key_bytes: 32_768,
    }
}

pub(super) fn graph_bundle() -> Arc<GraphBundle> {
    let spec = GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: 83 },
            },
            GraphNodeSpec {
                id: "gru".into(),
                kind: GraphNodeKind::Gru {
                    input_size: 83,
                    hidden_size: 2,
                },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: 2,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".into(),
                to: "gru".into(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "gru".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            },
        ],
        outputs: vec![GraphOutputRef {
            node_id: "head".into(),
            port: None,
        }],
        output_size: 2,
    };
    Arc::new(
        GraphBundle::compile(spec, &graph_limits()).expect("generation fixture graph must compile"),
    )
}

pub(super) fn checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 64 * 1024 * 1024,
        max_manifest_bytes: 1024 * 1024,
        max_state_bytes: 1024 * 1024,
        max_graph_bytes: 1024 * 1024,
        max_population_index_bytes: 1024 * 1024,
        max_population_count: 100,
        max_setting_count: 256,
        max_baseline_rng_count: 128,
        max_string_bytes: 16 * 1024,
        max_total_string_bytes: 1024 * 1024,
        max_weight_floats: 1_000_000,
        max_recurrent_floats: 1_000_000,
        max_numeric_stored_bytes: 64 * 1024 * 1024,
        max_numeric_candidate_bytes: 64 * 1024 * 1024,
        max_total_decoded_bytes: 128 * 1024 * 1024,
    }
}

pub(super) fn stream(seed: u32, label: &str) -> SerializedRngState {
    StatefulRng::new(f64::from(derive_seed(f64::from(seed), label))).export_state()
}

pub(super) fn fixture(graph: &CompiledGraph) -> StateCandidate {
    const COUNT: usize = 4;
    const BASELINES: usize = 0;
    const EPOCH: u64 = 7;
    let settings = fixture_default_settings(COUNT, BASELINES);
    let settings_schema_sha256 =
        normalized_settings_schema_hash(&settings).expect("fixture schema must hash");
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256,
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 3_500.0,
        population_count: COUNT,
        baseline_count: BASELINES,
        max_world_snakes: 32,
        max_non_population_brains: 16,
        max_body_points: 10_000,
        max_pellets: 10_000,
        spatial_index_bytes: 8 * 1024 * 1024,
        worker_scratch_bytes: 8 * 1024 * 1024,
        checkpoint_scratch_bytes: 8 * 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let config_hash = normalized_config_hash(&config).expect("fixture config must hash");
    let population = (0..COUNT)
        .map(|slot| PopulationGenome {
            slot: slot as u32,
            brain: BrainHandle {
                id: slot as u64 + 1,
                epoch: EPOCH,
            },
            lineage: GenomeLineage {
                genome_id: slot as u64 + 10,
                birth_generation: 7,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights: (0..graph.total_parameters)
                .map(|index| ((slot * 29 + index * 7) % 101) as f32 / 100.0 - 0.5)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
        .collect::<Vec<_>>();
    let brains = population
        .iter()
        .map(|genome| BrainRuntimeState {
            handle: genome.brain,
            owner: BrainOwner::PopulationSlot(genome.slot),
            retained_origin: None,
            non_population_weights: None,
            recurrent: vec![0.25; graph.total_state_size].into_boxed_slice(),
        })
        .collect::<Vec<_>>();
    let mut body_points = Vec::new();
    let mut snakes = Vec::new();
    for slot in 0..COUNT {
        let start = body_points.len();
        for point in 0..(5 + slot) {
            body_points.push(WorldPoint {
                x: slot as f64 * 100.0 - point as f64 * 7.5,
                y: slot as f64 * 20.0,
            });
        }
        snakes.push(SnakeState {
            id: slot as u64 + 1,
            frame_v1_id: slot as u32 + 1,
            kind: SnakeKind::Evolved,
            alive: slot != 3,
            population_slot: Some(slot as u32),
            brain: Some(population[slot].brain),
            baseline_slot: None,
            baseline_strategy: None,
            position: body_points[start],
            previous_position: body_points[start],
            direction: 0.1 * slot as f64,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 20.0 + slot as f64,
            food: slot as f64,
            points: [30.0, 18.0, 5.0, 0.0][slot],
            kills: slot as u64,
            target_length: (5 + slot) as f64,
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
                start,
                len: 5 + slot,
            },
            skin: slot as u32,
        });
    }
    let world = WorldState {
        ray_cursor: Default::default(),
        snakes,
        body_points,
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    };
    let mut fixed_step = FixedStepContinuationState::generation_boundary();
    fixed_step
        .sensor_generation
        .update_after_step(&world)
        .expect("fixture sensor continuation must cover every live evolved snake");
    StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: crate::engine::contract::ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: CANONICAL_GRAPH_LAYOUT_VERSION,
        },
        identity: RunIdentity {
            run_id: "generation-run".into(),
            seed: 42,
            config_revision: 3,
            config_hash,
            source_revision: "generation-test-source".into(),
            engine_build_id: "generation-test-engine".into(),
            source_sha256: "1".repeat(64),
            target_triple: "x86_64-pc-windows-msvc".into(),
            build_profile: "release".into(),
            build_class: "production".into(),
            rustc_version: "rustc generation-test".into(),
            build_contract_sha256: format!("sha256:{}", "2".repeat(64)),
            math_backend: "rust-scalar-v1".into(),
            legacy_conversion: None,
            founder: None,
        },
        config,
        phase: AuthorityPhase::Running,
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation: 7,
            completed_step: 99,
            population_epoch: EPOCH,
            elapsed_seconds: 12.0,
            wall_accumulator_seconds: 0.25,
            best_fitness_ever: 777.0,
        },
        fixed_step,
        rng: RngStateBundle {
            baseline_behaviors: Vec::new(),
            evolved_respawns: Vec::new(),
            version: RNG_BUNDLE_VERSION,
            world: stream(42, "world"),
            evolution: stream(42, "evolution"),
            external_controller: stream(42, "external-controller"),
            baselines: (0..BASELINES)
                .map(|slot| BaselineRngState {
                    slot: slot as u32,
                    state: stream(42, &format!("old-baseline:{slot}")),
                })
                .collect(),
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 10_000,
            next_brain_id: 20,
            next_genome_id: 100,
            next_controller_lease_id: 30,
            next_frame_v1_id: 40,
            next_external_id: EXTERNAL_ENTITY_ID_START + 50,
            next_baseline_id: BASELINE_ENTITY_ID_START + 60,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START + 70,
        },
        population,
        brains,
        world,
    }
}

pub(super) fn policy(source: &StateCandidate) -> StateAdmissionPolicy {
    StateAdmissionPolicy {
        memory_ceiling_bytes: usize::MAX,
        require_exact_build_identity: true,
        expected_source_revision: source.identity.source_revision.clone(),
        expected_engine_build_id: source.identity.engine_build_id.clone(),
        expected_source_sha256: source.identity.source_sha256.clone(),
        expected_target_triple: source.identity.target_triple.clone(),
        expected_build_profile: source.identity.build_profile.clone(),
        expected_build_class: source.identity.build_class.clone(),
        expected_rustc_version: source.identity.rustc_version.clone(),
        expected_build_contract_sha256: source.identity.build_contract_sha256.clone(),
        expected_math_backend: source.identity.math_backend.clone(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
        expected_settings_schema_sha256: source.config.settings_schema_sha256.clone(),
    }
}

pub(super) fn advanced(state: &SerializedRngState) -> SerializedRngState {
    let mut rng = StatefulRng::from_state(state).expect("fixture RNG must restore");
    rng.next_f64();
    rng.export_state()
}

pub(super) fn prepared_test_boundary(graph: &CompiledGraph) -> StateCandidate {
    let source = fixture(graph);
    let mut completed_rng = source.rng.clone();
    completed_rng.world = advanced(&completed_rng.world);
    completed_rng.external_controller = advanced(&completed_rng.external_controller);
    let mut completed_allocators = source.allocators.clone();
    completed_allocators.next_entity_id += 3;
    completed_allocators.next_brain_id += 1;
    completed_allocators.next_controller_lease_id += 1;
    completed_allocators.next_frame_v1_id += 2;
    completed_allocators.next_external_id += 1;
    let elapsed = source.generation.elapsed_seconds + source.config.fixed_step_seconds;
    prepare_generation_boundary(
        &source,
        &source.world,
        &completed_rng,
        &completed_allocators,
        elapsed,
        graph,
    )
    .expect("fixture generation must prepare")
    .into_parts()
    .0
}

pub(super) fn replace_setting(
    candidate: &mut StateCandidate,
    path: &str,
    value: NormalizedSettingValue,
) {
    let setting = candidate
        .config
        .settings
        .iter_mut()
        .find(|setting| setting.path == path)
        .unwrap_or_else(|| panic!("fixture setting {path} must exist"));
    setting.value = value;
}

pub(super) fn refresh_config_identity(candidate: &mut StateCandidate) {
    candidate.config.settings_schema_sha256 =
        normalized_settings_schema_hash(&candidate.config.settings)
            .expect("modified fixture settings must hash");
    candidate.identity.config_hash =
        normalized_config_hash(&candidate.config).expect("modified fixture config must hash");
}
