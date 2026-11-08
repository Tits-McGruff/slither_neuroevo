use super::*;

/// Collision-free per-process temporary directory counter.
pub(super) static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

/// Exact test-owned directory removed on scope exit.
pub(super) struct TestDirectory {
    pub(super) path: PathBuf,
}

impl TestDirectory {
    /// Create one unique directory below the OS temporary root.
    pub(super) fn new(label: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "slither-checkpoint-v3-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory must be unique and creatable");
        Self { path }
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
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Reviewed checkpoint bounds used only by local codec fixtures.
pub(super) fn checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 64 * 1024 * 1024,
        max_manifest_bytes: 64 * 1024,
        max_state_bytes: 256 * 1024,
        max_graph_bytes: 512 * 1024,
        max_population_index_bytes: 1024 * 1024,
        max_population_count: 512,
        max_setting_count: 32,
        max_baseline_rng_count: 32,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 2 * 1024 * 1024,
        max_weight_floats: 8 * 1024 * 1024,
        max_recurrent_floats: 1024 * 1024,
        max_numeric_stored_bytes: 48 * 1024 * 1024,
        max_numeric_candidate_bytes: 48 * 1024 * 1024,
        max_total_decoded_bytes: 64 * 1024 * 1024,
    }
}

/// Reviewed graph bounds used by the small and representative fixtures.
pub(super) fn graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 16,
        max_edges: 32,
        max_graph_outputs: 4,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 1024,
        max_tensor_width: 1024,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 16,
        max_parameter_floats: 1_000_000,
        max_recurrent_state_floats: 16_384,
        max_canonical_layout_bytes: 128 * 1024,
        max_architecture_key_bytes: 256 * 1024,
    }
}

/// Source graph with either recurrent-state coverage or representative P0-sized weights.
pub(super) fn graph_spec(representative: bool) -> GraphSpec {
    if representative {
        GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 3 },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 3,
                        hidden_sizes: vec![128, 128],
                        output_size: 2,
                    },
                },
            ],
            edges: vec![GraphEdge {
                from: "input".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            }],
            outputs: vec![GraphOutputRef {
                node_id: "head".into(),
                port: None,
            }],
            output_size: 2,
        }
    } else {
        GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 3 },
                },
                GraphNodeSpec {
                    id: "memory".into(),
                    kind: GraphNodeKind::Gru {
                        input_size: 3,
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
                    to: "memory".into(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "memory".into(),
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
}

/// Admission policy exactly matching the fixture identity and settings schema.
pub(super) fn admission_policy(settings_schema_sha256: String) -> StateAdmissionPolicy {
    StateAdmissionPolicy {
        memory_ceiling_bytes: 128 * 1024 * 1024,
        require_exact_build_identity: true,
        expected_source_revision: "test-revision".into(),
        expected_engine_build_id: "test-engine-build".into(),
        expected_source_sha256: "1".repeat(64),
        expected_target_triple: "x86_64-pc-windows-msvc".into(),
        expected_build_profile: "release".into(),
        expected_build_class: "test-hooks".into(),
        expected_rustc_version: "rustc-test".into(),
        expected_build_contract_sha256: format!("sha256:{}", "2".repeat(64)),
        expected_math_backend: "scalar-test".into(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
        expected_settings_schema_sha256: settings_schema_sha256,
    }
}

/// Construct a fully admitted exact run-start boundary.
pub(super) fn admitted_state(
    population_count: usize,
    representative: bool,
) -> (AuthoritativeState, Arc<GraphBundle>, StateAdmissionPolicy) {
    let graph = Arc::new(
        GraphBundle::compile(graph_spec(representative), &graph_limits())
            .expect("fixture graph must compile"),
    );
    let settings = vec![
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
            value: NormalizedSettingValue::Integer(population_count as i64),
        },
        NormalizedSetting {
            path: "worldRadius".into(),
            value: NormalizedSettingValue::Integer(1000),
        },
    ];
    let settings_schema_sha256 =
        normalized_settings_schema_hash(&settings).expect("fixture setting schema must hash");
    let mut config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256: settings_schema_sha256.clone(),
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 1000.0,
        population_count,
        baseline_count: 0,
        max_world_snakes: population_count,
        max_non_population_brains: 0,
        max_body_points: 100_000,
        max_pellets: 10_000,
        spatial_index_bytes: 1024,
        worker_scratch_bytes: 1024,
        checkpoint_scratch_bytes: 2 * 1024 * 1024,
        controller_input_hold_ms: 200,
        controller_disconnect_grace_ms: 5000,
    };
    let config_hash = normalized_config_hash(&config).expect("fixture config must hash");
    let identity = RunIdentity {
        run_id: "11111111-2222-4333-8444-555555555555".into(),
        seed: 7,
        config_revision: 1,
        config_hash,
        source_revision: "test-revision".into(),
        engine_build_id: "test-engine-build".into(),
        source_sha256: "1".repeat(64),
        target_triple: "x86_64-pc-windows-msvc".into(),
        build_profile: "release".into(),
        build_class: "test-hooks".into(),
        rustc_version: "rustc-test".into(),
        build_contract_sha256: format!("sha256:{}", "2".repeat(64)),
        math_backend: "scalar-test".into(),
        legacy_conversion: None,
        founder: None,
    };
    // Keep the assignment explicit if config construction changes in a future fixture.
    config.graph_architecture_key = graph.architecture_key.clone();
    let mut population = Vec::with_capacity(population_count);
    let mut brains = Vec::with_capacity(population_count);
    for slot in 0..population_count {
        let brain = BrainHandle {
            id: slot as u64 + 1,
            epoch: 1,
        };
        let weights: Vec<f32> = (0..graph.total_parameters)
            .map(|index| {
                let mantissa = ((slot as u32).wrapping_mul(8191) ^ (index as u32).wrapping_mul(37))
                    & 0x007f_ffff;
                f32::from_bits(0x3f00_0000 | mantissa)
            })
            .collect();
        population.push(PopulationGenome {
            slot: slot as u32,
            brain,
            lineage: GenomeLineage {
                genome_id: slot as u64 + 1,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: slot as f64 / 10.0,
            weights: weights.into_boxed_slice(),
        });
        brains.push(BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(slot as u32),
            retained_origin: None,
            non_population_weights: None,
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        });
    }
    let rng = || StatefulRng::new(7.0).export_state();
    let candidate = StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: CANONICAL_GRAPH_LAYOUT_VERSION,
        },
        identity,
        config,
        phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart),
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation: 1,
            completed_step: 0,
            population_epoch: 1,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: 0.0,
        },
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: RngStateBundle {
            baseline_behaviors: Vec::new(),
            evolved_respawns: Vec::new(),
            version: RNG_BUNDLE_VERSION,
            world: rng(),
            evolution: rng(),
            external_controller: rng(),
            baselines: Vec::new(),
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1,
            next_brain_id: population_count as u64 + 1,
            next_genome_id: population_count as u64 + 1,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        },
        population,
        brains,
        world: WorldState::default(),
    };
    let policy = admission_policy(settings_schema_sha256);
    let admitted = AuthoritativeState::validate_and_own(candidate, Arc::clone(&graph), &policy)
        .expect("fixture state must be admissible");
    (admitted, graph, policy)
}

/// Publish one fixture and return its exact managed path and descriptor.
pub(super) fn publish_fixture(
    directory: &TestDirectory,
    state: &AuthoritativeState,
    policy: &StateAdmissionPolicy,
    operation: &str,
) -> (PathBuf, CheckpointDescriptor) {
    let descriptor = publish_checkpoint(
        &directory.path,
        CheckpointOperationId::parse(operation).unwrap(),
        9,
        state.checkpoint_boundary().unwrap(),
        &checkpoint_limits(),
        &graph_limits(),
        policy,
    )
    .expect("checkpoint publication must succeed");
    let path = directory.path.join(&descriptor.relative_filename);
    (path, descriptor)
}

/// Recalculate one mutated USTAR header's standard checksum.
pub(super) fn refresh_test_ustar_checksum(header: &mut [u8]) {
    header[148..156].fill(b' ');
    let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
    let encoded = format!("{sum:06o}\0 ");
    assert_eq!(encoded.len(), 8);
    header[148..156].copy_from_slice(encoded.as_bytes());
}

/// Replace a USTAR header's fixed short path and refresh its checksum.
pub(super) fn set_test_ustar_path(header: &mut [u8], path: &str) {
    assert!(path.len() <= 100 && path.is_ascii());
    header[..100].fill(0);
    header[..path.len()].copy_from_slice(path.as_bytes());
    refresh_test_ustar_checksum(header);
}

/// Write a mutated archive under its expected digest filename and require rejection.
pub(super) fn assert_mutated_archive_rejected(
    label: &str,
    original: &[u8],
    filename: &str,
    policy: &StateAdmissionPolicy,
    mutate: impl FnOnce(&mut Vec<u8>),
) {
    let directory = TestDirectory::new(label);
    let path = directory.path.join(filename);
    let mut bytes = original.to_vec();
    mutate(&mut bytes);
    fs::write(&path, bytes).unwrap();
    assert!(
        restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), policy).is_err(),
        "mutated archive {label} must be rejected"
    );
}

/// One independently published evolved save with real winner weights.
pub(super) struct HallOfFameImportFixture {
    pub(super) source: TestDirectory,
    pub(super) policy: StateAdmissionPolicy,
    pub(super) checkpoint: CheckpointDescriptor,
    pub(super) elite: HallOfFameWeightsDescriptor,
    pub(super) archive_path: PathBuf,
}

/// Build an exact archive through the production checkpoint and export writers.
pub(super) fn hall_of_fame_import_fixture() -> HallOfFameImportFixture {
    use crate::engine::export_archive::{compose_export_archive, ExportInventoryDescriptor};

    let source = TestDirectory::new("import-hof-source");
    let (initial, graph, policy) = admitted_state(2, false);
    let mut evolved = initial.state().clone();
    evolved.phase = AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation);
    evolved.generation.generation = 2;
    evolved.generation.completed_step = 480;
    let state = AuthoritativeState::validate_and_own(evolved, graph, &policy).unwrap();
    let (_, checkpoint) = publish_fixture(&source, &state, &policy, &"a".repeat(32));
    let weights = &state.state().population[0].weights;
    let elite = publish_hall_of_fame_weights(
        &source.path,
        &CheckpointOperationId::parse("b".repeat(32)).unwrap(),
        weights,
        &checkpoint_limits(),
    )
    .unwrap();
    let mut inventory_bytes = vec![0u8; 32 + 56 + 120];
    inventory_bytes[..13].copy_from_slice(b"SLITHER-EXPV1");
    inventory_bytes[16..24].copy_from_slice(&1u64.to_le_bytes());
    inventory_bytes[24..32].copy_from_slice(&1u64.to_le_bytes());
    inventory_bytes[32..40].copy_from_slice(&1u64.to_le_bytes());
    let record = &mut inventory_bytes[88..];
    record[..8].copy_from_slice(&1u64.to_le_bytes());
    record[56..88].copy_from_slice(&parse_digest(&elite.logical_sha256, "elite").unwrap());
    record[88] = match elite.encoding {
        NumericEncoding::RawF32LeV1 => 0,
        NumericEncoding::F32LeShuffle4ZstdV1 => 1,
    };
    for (offset, value) in [
        (96, &elite.stored_byte_count_hex),
        (104, &elite.decoded_byte_count_hex),
        (112, &elite.weight_count_hex),
    ] {
        record[offset..offset + 8]
            .copy_from_slice(&parse_u64_hex(value, "elite size").unwrap().to_le_bytes());
    }
    let operation = "c".repeat(32);
    let inventory = ExportInventoryDescriptor {
        version: 1,
        relative_filename: format!(".{operation}.export-inventory-v1"),
        sha256: encode_digest(Sha256::digest(&inventory_bytes).into()),
        stored_byte_count_hex: encode_u64_hex(inventory_bytes.len() as u64),
        history_count_hex: encode_u64_hex(1),
        hall_of_fame_count_hex: encode_u64_hex(1),
    };
    fs::write(
        source.path.join(&inventory.relative_filename),
        inventory_bytes,
    )
    .unwrap();
    let archive = compose_export_archive(
        &source.path,
        &operation,
        &checkpoint,
        &inventory,
        &checkpoint_limits(),
        &graph_limits(),
        &policy,
    )
    .unwrap();
    let archive_path = source.path.join(archive.relative_filename);
    HallOfFameImportFixture {
        source,
        policy,
        checkpoint,
        elite,
        archive_path,
    }
}
