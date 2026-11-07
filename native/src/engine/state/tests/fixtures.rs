use super::*;

pub(super) static NEXT_GENERATION_HANDOFF_DIRECTORY: TestAtomicU64 = TestAtomicU64::new(1);

pub(super) struct GenerationHandoffDirectory {
    pub(super) path: PathBuf,
}

impl GenerationHandoffDirectory {
    pub(super) fn new() -> Self {
        let sequence = NEXT_GENERATION_HANDOFF_DIRECTORY.fetch_add(1, TestOrdering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "slither-generation-handoff-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("generation handoff directory must be unique");
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for GenerationHandoffDirectory {
    fn drop(&mut self) {
        let expected_prefix = format!("slither-generation-handoff-{}-", std::process::id());
        let is_owned = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&expected_prefix));
        if is_owned {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn default_graph_spec() -> GraphSpec {
    GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: 83 },
            },
            GraphNodeSpec {
                id: "mlp".into(),
                kind: GraphNodeKind::Mlp {
                    input_size: 83,
                    hidden_sizes: vec![64],
                    output_size: 64,
                },
            },
            GraphNodeSpec {
                id: "gru".into(),
                kind: GraphNodeKind::Gru {
                    input_size: 64,
                    hidden_size: 16,
                },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: 16,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".into(),
                to: "mlp".into(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "mlp".into(),
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
    }
}

pub(super) fn default_graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 16,
        max_edges: 32,
        max_graph_outputs: 4,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 1_024,
        max_tensor_width: 1_024,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 16,
        max_parameter_floats: 1_000_000,
        max_recurrent_state_floats: 16_384,
        max_canonical_layout_bytes: 65_536,
        max_architecture_key_bytes: 200_000,
    }
}

pub(super) fn generation_handoff_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 64 * 1024 * 1024,
        max_manifest_bytes: 1024 * 1024,
        max_state_bytes: 4 * 1024 * 1024,
        max_graph_bytes: 4 * 1024 * 1024,
        max_population_index_bytes: 4 * 1024 * 1024,
        max_population_count: 300,
        max_setting_count: 512,
        max_baseline_rng_count: 128,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 4 * 1024 * 1024,
        max_weight_floats: 8_000_000,
        max_recurrent_floats: 1_000_000,
        max_numeric_stored_bytes: 64 * 1024 * 1024,
        max_numeric_candidate_bytes: 64 * 1024 * 1024,
        max_total_decoded_bytes: 128 * 1024 * 1024,
    }
}

pub(super) fn default_graph() -> Arc<GraphBundle> {
    Arc::new(
        GraphBundle::compile(default_graph_spec(), &default_graph_limits())
            .expect("default graph must compile"),
    )
}

pub(super) fn runtime_init() -> EngineInit {
    EngineInit {
        contract_version: ENGINE_CONTRACT_VERSION,
        inbound: InboundLimits {
            max_batches: 4,
            max_commands: 8,
            max_owned_bytes: 64,
            max_batch_commands: 4,
            max_batch_owned_bytes: 32,
        },
        output: OutputLimits {
            max_reliable: 8,
            max_reliable_owned_bytes: 64,
            max_discrete: 4,
            max_discrete_owned_bytes: 64,
            max_total_owned_bytes: 128,
            max_event_owned_bytes: 64,
            max_frame_connections: 4,
        },
    }
}

pub(super) fn rng(seed: u32, label: &str) -> SerializedRngState {
    StatefulRng::new(f64::from(derive_seed(f64::from(seed), label))).export_state()
}

pub(super) fn test_settings(count: usize) -> Vec<NormalizedSetting> {
    vec![
        NormalizedSetting {
            path: "baselineBots.count".into(),
            value: NormalizedSettingValue::Integer(0),
        },
        NormalizedSetting {
            path: "brain.sensorVersion".into(),
            value: NormalizedSettingValue::Integer(i64::from(SENSOR_VERSION)),
        },
        NormalizedSetting {
            path: "simSpeed".into(),
            value: NormalizedSettingValue::Float(1.0),
        },
        NormalizedSetting {
            path: "snakeCount".into(),
            value: NormalizedSettingValue::Integer(count as i64),
        },
        NormalizedSetting {
            path: "worldRadius".into(),
            value: NormalizedSettingValue::Integer(2_500),
        },
    ]
}

pub(super) fn policy(memory_ceiling_bytes: usize) -> StateAdmissionPolicy {
    StateAdmissionPolicy {
        memory_ceiling_bytes,
        require_exact_build_identity: true,
        expected_source_revision: "test-source".into(),
        expected_engine_build_id: "test-engine".into(),
        expected_source_sha256: "1111111111111111111111111111111111111111111111111111111111111111"
            .into(),
        expected_target_triple: "x86_64-pc-windows-msvc".into(),
        expected_build_profile: "release".into(),
        expected_build_class: "production".into(),
        expected_rustc_version: "rustc 1.92.0 (test)".into(),
        expected_build_contract_sha256:
            "sha256:3333333333333333333333333333333333333333333333333333333333333333".into(),
        expected_math_backend: "rust-scalar-v1".into(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
        expected_settings_schema_sha256: normalized_settings_schema_hash(&test_settings(1))
            .expect("test settings schema must hash"),
    }
}

pub(super) fn own(
    candidate: StateCandidate,
    graph: Arc<GraphBundle>,
    memory_ceiling_bytes: usize,
) -> Result<AuthoritativeState, StateError> {
    AuthoritativeState::validate_and_own(candidate, graph, &policy(memory_ceiling_bytes))
}

pub(super) fn candidate(graph: &CompiledGraph, count: usize) -> StateCandidate {
    let epoch = 1;
    let population = (0..count)
        .map(|slot| PopulationGenome {
            slot: slot as u32,
            brain: BrainHandle {
                id: slot as u64 + 1,
                epoch,
            },
            lineage: GenomeLineage {
                genome_id: 100 + slot as u64,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: slot as f64,
            weights: vec![slot as f32 + 0.25; graph.total_parameters].into_boxed_slice(),
        })
        .collect::<Vec<_>>();
    let brains = population
        .iter()
        .map(|genome| BrainRuntimeState {
            handle: genome.brain,
            owner: BrainOwner::PopulationSlot(genome.slot),
            retained_origin: None,
            non_population_weights: None,
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        })
        .collect();
    let settings = test_settings(count);
    let settings_schema_sha256 =
        normalized_settings_schema_hash(&settings).expect("test settings schema must hash");
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256,
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 2_500.0,
        population_count: count,
        baseline_count: 0,
        max_world_snakes: count + 17,
        max_non_population_brains: 16,
        max_body_points: 100_000,
        max_pellets: 10_000,
        spatial_index_bytes: 2 * 1024 * 1024,
        worker_scratch_bytes: 1024 * 1024,
        checkpoint_scratch_bytes: 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let config_hash = normalized_config_hash(&config).expect("test config must hash");
    StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: graph.layout_version,
        },
        identity: RunIdentity {
            run_id: "run-test".into(),
            seed: 42,
            config_revision: 1,
            config_hash,
            source_revision: "test-source".into(),
            engine_build_id: "test-engine".into(),
            source_sha256: "1111111111111111111111111111111111111111111111111111111111111111"
                .into(),
            target_triple: "x86_64-pc-windows-msvc".into(),
            build_profile: "release".into(),
            build_class: "production".into(),
            rustc_version: "rustc 1.92.0 (test)".into(),
            build_contract_sha256:
                "sha256:3333333333333333333333333333333333333333333333333333333333333333".into(),
            math_backend: "rust-scalar-v1".into(),
            legacy_conversion: None,
        },
        config,
        phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart),
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation: 1,
            completed_step: 0,
            population_epoch: epoch,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: 0.0,
        },
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: RngStateBundle {
            baseline_behaviors: Vec::new(),
            evolved_respawns: Vec::new(),
            version: RNG_BUNDLE_VERSION,
            world: rng(42, "world"),
            evolution: rng(42, "evolution"),
            external_controller: rng(42, "external-controller"),
            baselines: Vec::new(),
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1,
            next_brain_id: count as u64 + 1,
            next_genome_id: 100 + count as u64,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        },
        population,
        brains,
        world: WorldState::default(),
    }
}

pub(super) fn refresh_config_hash(candidate: &mut StateCandidate) {
    candidate.identity.config_hash =
        normalized_config_hash(&candidate.config).expect("test config must hash");
}

pub(super) fn enable_one_live_baseline(
    candidate: &mut StateCandidate,
    snake_id: u64,
    respawn_remaining_seconds: Option<f64>,
) {
    candidate.config.baseline_count = 1;
    candidate
        .config
        .settings
        .iter_mut()
        .find(|setting| setting.path == "baselineBots.count")
        .expect("test baseline-count setting")
        .value = NormalizedSettingValue::Integer(1);
    candidate.rng.baselines = vec![BaselineRngState {
        slot: 0,
        state: rng(42, "baseline:0"),
    }];
    candidate.fixed_step.baseline_lifecycle = BaselineLifecycleState {
        version: crate::engine::baseline::BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 0.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds,
        }],
    };
    refresh_config_hash(candidate);
}

pub(super) fn push_evolved_snake(
    candidate: &mut StateCandidate,
    slot: u32,
    id: u64,
    frame_v1_id: u32,
    position: WorldPoint,
) {
    let body_start = candidate.world.body_points.len();
    candidate.world.body_points.push(position);
    candidate.world.snakes.push(SnakeState {
        id,
        frame_v1_id,
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(slot),
        brain: Some(candidate.population[slot as usize].brain),
        baseline_slot: None,
        baseline_strategy: None,
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
        body: BodyRange {
            start: body_start,
            len: 1,
        },
        skin: 0,
    });
    candidate.allocators.next_entity_id = candidate.allocators.next_entity_id.max(id + 1);
    candidate.allocators.next_frame_v1_id =
        candidate.allocators.next_frame_v1_id.max(frame_v1_id + 1);
}

pub(super) fn running_candidate(graph: &CompiledGraph) -> StateCandidate {
    let mut running = candidate(graph, 1);
    running.phase = AuthorityPhase::Running;
    push_evolved_snake(&mut running, 0, 1, 1, WorldPoint { x: 10.0, y: 20.0 });
    running
}

pub(super) fn complete_running_candidate(graph: &CompiledGraph) -> StateCandidate {
    let mut running = running_candidate(graph);
    running.config.settings = crate::engine::step_config::tests::default_settings(1, 0);
    running.config.settings_schema_sha256 =
        normalized_settings_schema_hash(&running.config.settings)
            .expect("complete running settings schema must hash");
    running.config.world_radius = 3_500.0;
    running.config.max_world_snakes = 32;
    running.config.max_non_population_brains = 31;
    running.config.max_pellets = 10_000;
    running.config.spatial_index_bytes = 8 * 1024 * 1024;
    running.config.worker_scratch_bytes = 8 * 1024 * 1024;

    let head = running.world.snakes[0].position;
    running.world.body_points = (0..5)
        .map(|offset| WorldPoint {
            x: head.x - (offset as f64 * 7.5),
            y: head.y,
        })
        .collect();
    let snake = &mut running.world.snakes[0];
    snake.body = BodyRange { start: 0, len: 5 };
    snake.target_length = 5.0;
    snake.radius = 9.0;
    snake.speed = 165.0;
    refresh_config_hash(&mut running);
    running
}

pub(super) fn complete_running_policy(
    candidate: &StateCandidate,
    memory_ceiling_bytes: usize,
) -> StateAdmissionPolicy {
    let mut admission = policy(memory_ceiling_bytes);
    admission.expected_settings_schema_sha256 = candidate.config.settings_schema_sha256.clone();
    admission
}

pub(super) fn set_complete_setting(
    candidate: &mut StateCandidate,
    path: &str,
    value: NormalizedSettingValue,
) {
    candidate
        .config
        .settings
        .iter_mut()
        .find(|setting| setting.path == path)
        .expect("complete running setting must exist")
        .value = value;
}

pub(super) fn own_complete_running(
    candidate: StateCandidate,
    graph: Arc<GraphBundle>,
) -> AuthoritativeState {
    let estimated = estimate_state_memory(&candidate, &graph)
        .expect("complete running fixture memory must estimate");
    let memory_ceiling = estimated
        .total_bytes
        .checked_add(64 * 1024 * 1024)
        .expect("test memory ceiling must fit");
    let admission = complete_running_policy(&candidate, memory_ceiling);
    AuthoritativeState::validate_and_own(candidate, graph, &admission)
        .expect("complete running fixture must admit")
}

pub(super) struct RunningStepBuffers {
    pub(super) evolved_generation: crate::engine::evolved_lives::EvolvedGenerationState,
    pub(super) world: WorldState,
    pub(super) rng: RngStateBundle,
    pub(super) allocators: AllocatorState,
    pub(super) brains: Vec<BrainRuntimeState>,
    pub(super) baseline_lifecycle: BaselineLifecycleState,
    pub(super) ambient_pellet_accumulator: f64,
    pub(super) sensor_generation: SensorGenerationState,
    pub(super) generation_elapsed_seconds: f64,
    pub(super) wall_accumulator_seconds: f64,
}

impl RunningStepBuffers {
    pub(super) fn from_state(state: &StateCandidate) -> Self {
        Self {
            world: state.world.clone(),
            rng: state.rng.clone(),
            allocators: state.allocators.clone(),
            brains: state.brains.clone(),
            evolved_generation: state.fixed_step.evolved_generation.clone(),
            baseline_lifecycle: state.fixed_step.baseline_lifecycle.clone(),
            ambient_pellet_accumulator: state.fixed_step.ambient_pellet_accumulator,
            sensor_generation: state.fixed_step.sensor_generation,
            generation_elapsed_seconds: state.generation.elapsed_seconds + 1.0 / 60.0,
            wall_accumulator_seconds: state.generation.wall_accumulator_seconds,
        }
    }

    pub(super) fn replacement(&mut self, key: PhysicsStepKey) -> RunningStepReplacement<'_> {
        RunningStepReplacement {
            key,
            world: &mut self.world,
            rng: &mut self.rng,
            allocators: &mut self.allocators,
            brains: &mut self.brains,
            evolved_generation: &mut self.evolved_generation,
            baseline_lifecycle: &mut self.baseline_lifecycle,
            ambient_pellet_accumulator: self.ambient_pellet_accumulator,
            sensor_generation: self.sensor_generation,
            generation_elapsed_seconds: self.generation_elapsed_seconds,
            wall_accumulator_seconds: self.wall_accumulator_seconds,
            mutation: RunningStepMutationContract::default(),
        }
    }
}

pub(super) fn push_external_snake(
    candidate: &mut StateCandidate,
    graph: &CompiledGraph,
    id: u64,
    frame_v1_id: u32,
    position: WorldPoint,
) {
    let handle = BrainHandle {
        id: candidate.allocators.next_brain_id,
        epoch: candidate.generation.population_epoch,
    };
    candidate.allocators.next_brain_id += 1;
    candidate.brains.push(BrainRuntimeState {
        handle,
        owner: BrainOwner::Entity(id),
        retained_origin: None,
        non_population_weights: Some(vec![0.0; graph.total_parameters].into_boxed_slice()),
        recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
    });
    let body_start = candidate.world.body_points.len();
    candidate.world.body_points.push(position);
    candidate.world.snakes.push(SnakeState {
        id,
        frame_v1_id,
        kind: SnakeKind::External,
        alive: true,
        population_slot: None,
        brain: Some(handle),
        baseline_slot: None,
        baseline_strategy: None,
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
        body: BodyRange {
            start: body_start,
            len: 1,
        },
        skin: 0,
    });
    candidate.allocators.next_external_id = candidate.allocators.next_external_id.max(id + 1);
    candidate.allocators.next_frame_v1_id =
        candidate.allocators.next_frame_v1_id.max(frame_v1_id + 1);
}

pub(super) fn connected_lease(
    id: u64,
    snake_id: u64,
    connection_id: u64,
    token: &str,
) -> ControllerLease {
    ControllerLease {
        reward: None,
        identity_key: String::new(),
        id,
        snake_id,
        kind: ControllerKind::Player,
        connection_id: Some(connection_id),
        scope: "run-test".into(),
        resume_token: token.into(),
        status: ControllerLeaseStatus::Connected,
        latest_action: LatestControllerAction {
            is_explicit: false,
            turn: 0.25,
            boost: true,
            client_tick: 0,
            arrival_sequence: 1,
            accepted_at_ms: 100,
        },
        last_observed_at_ms: 100,
        disconnected_at_ms: None,
        input_hold_expires_at_ms: None,
        grace_expires_at_ms: None,
        takeover_committed_at_ms: None,
    }
}

pub(super) fn push_connected_external_fixture(
    candidate: &mut StateCandidate,
    graph: &CompiledGraph,
    snake_id: u64,
    frame_v1_id: u32,
    lease_id: u64,
    connection_id: u64,
    position: WorldPoint,
) {
    push_external_snake(candidate, graph, snake_id, frame_v1_id, position);
    let snake_index = candidate.world.snakes.len() - 1;
    let body_start = candidate.world.snakes[snake_index].body.start;
    candidate
        .world
        .body_points
        .extend((1..5).map(|offset| WorldPoint {
            x: position.x - (offset as f64 * 7.5),
            y: position.y,
        }));
    let snake = &mut candidate.world.snakes[snake_index];
    snake.body = BodyRange {
        start: body_start,
        len: 5,
    };
    snake.target_length = 5.0;
    snake.radius = 9.0;
    snake.speed = 165.0;
    snake.points = 10.0;
    snake.delivered_observation_points = 2.0;
    candidate.world.controller_leases.push(connected_lease(
        lease_id,
        snake_id,
        connection_id,
        &format!("external-delivery-{lease_id}"),
    ));
    candidate.allocators.next_controller_lease_id = candidate
        .allocators
        .next_controller_lease_id
        .max(lease_id + 1);
}

pub(super) fn enable_inspector_fixture(
    coordinator: &mut RunningStepCoordinator,
    authority: &AuthoritativeState,
    physical_id: u64,
) {
    let snake = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == physical_id)
        .unwrap();
    coordinator
        .configure_inspector(
            authority,
            Some(crate::engine::inspector::InspectorTarget {
                world_epoch: authority.world_epoch(),
                generation: authority.state().generation.generation,
                physical_id,
                frame_v1_id: snake.frame_v1_id,
            }),
            crate::engine::inspector::MAX_INSPECTOR_BYTES,
        )
        .unwrap();
}

/// A full retained pool with a dead middle body and a pellet referencing it.
pub(super) fn retained_reuse_fixture() -> (AuthoritativeState, Arc<GraphBundle>) {
    let graph = default_graph();
    let mut source = complete_running_candidate(&graph);
    source.config.max_world_snakes = 3;
    source.config.max_non_population_brains = 2;
    source.identity.config_hash = normalized_config_hash(&source.config).unwrap();
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let weights = vec![0.25; graph.compiled().total_parameters].into_boxed_slice();
    let first = authority
        .resurrect_hall_of_fame(
            1,
            weights.clone(),
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
    authority
        .resurrect_hall_of_fame(2, weights, RunningStepWorkLimits::provisional_defaults())
        .unwrap();
    let dead = authority
        .candidate
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == first.snake_id)
        .unwrap();
    dead.alive = false;
    let position = dead.position;
    let id = authority.candidate.allocators.next_entity_id;
    authority.candidate.allocators.next_entity_id += 1;
    authority.candidate.world.pellets.push(PelletState {
        id,
        position,
        value: 1.0,
        kind: 1,
        color: 0,
        owner: Some(first.snake_id),
    });
    (authority, graph)
}

pub(super) fn respawn_candidate(
    graph: &CompiledGraph,
    preserve: bool,
    recurrence: f32,
) -> StateCandidate {
    let mut source = complete_running_candidate(graph);
    source.config.settings = crate::engine::step_config::production_default_settings(1, 0);
    source.config.settings_schema_sha256 =
        normalized_settings_schema_hash(&source.config.settings).unwrap();
    set_complete_setting(
        &mut source,
        "evolvedRespawn.enabled",
        NormalizedSettingValue::Bool(true),
    );
    set_complete_setting(
        &mut source,
        "evolvedRespawn.recurrentState",
        NormalizedSettingValue::Integer(i64::from(preserve)),
    );
    source.versions.rng_bundle = RNG_BUNDLE_WITH_EVOLVED_VERSION;
    source.rng.version = RNG_BUNDLE_WITH_EVOLVED_VERSION;
    source.rng.evolved_respawns =
        EvolvedRespawnRngState::derive_generation(source.identity.seed, 1, 1).unwrap();
    source.world.snakes[0].alive = false;
    source.world.snakes[0].turn = 0.7;
    source.world.snakes[0].previous_turn = -0.4;
    source.world.snakes[0].input_boost = true;
    source.world.snakes[0].previous_input_boost = true;
    source.population[0].weights.fill(0.0);
    source.brains[0].recurrent.fill(recurrence);
    source.fixed_step.evolved_generation =
        crate::engine::evolved_lives::EvolvedGenerationState::initialize(&source.world, 1, 5)
            .unwrap();
    refresh_config_hash(&mut source);
    source
}

pub(super) fn one_respawn_step(
    source: StateCandidate,
    graph: Arc<GraphBundle>,
) -> (AuthoritativeState, WorldStepDiagnostics) {
    let mut authority = own_complete_running(source, graph);
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    let diagnostics = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .unwrap()
    {
        RunningStepProgress::Published(outcome) => *outcome.diagnostics,
        other => panic!("expected a published newborn step, got {other:?}"),
    };
    (authority, diagnostics)
}

pub(super) fn candidate_fixed_dt(authority: &AuthoritativeState) -> f64 {
    authority.state().config.fixed_step_seconds
}
