//! Test-hook-only construction of one real managed checkpoint publication.
//!
//! This module is absent from production builds. It deliberately exercises the
//! production graph, state-admission, checkpoint codec, fsync, and immutable
//! rename path while returning only the scalar checkpoint descriptor.

use std::path::Path;
use std::sync::Arc;

use super::checkpoint::{
    publish_checkpoint, CheckpointDescriptor, CheckpointLimits, CheckpointOperationId,
};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::graph::{
    GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
    CANONICAL_GRAPH_LAYOUT_VERSION,
};
use super::rng::StatefulRng;
use super::state::{
    normalized_config_hash, normalized_settings_schema_hash, AllocatorState, AuthoritativeState,
    AuthorityPhase, BrainHandle, BrainOwner, BrainRuntimeState, ContractVersions,
    GenerationBoundaryKind, GenerationState, GenomeLineage, NormalizedEngineConfig,
    NormalizedSetting, NormalizedSettingValue, PopulationGenome, RngStateBundle, RunIdentity,
    StateAdmissionPolicy, StateCandidate, WorldState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START,
    CHECKPOINT_VERSION, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START,
    GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
    RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, SENSOR_VERSION, SERIALIZER_VERSION,
};

/// Exact small population used by the cross-language publication fixture.
const FIXTURE_POPULATION_COUNT: usize = 4;

/// Publish one deterministic real checkpoint through the production codec.
pub(crate) fn publish_stage3_fixture(
    managed_directory: &Path,
    operation_id: &str,
    transition_epoch: u64,
) -> Result<CheckpointDescriptor, String> {
    if env!("SLITHER_NATIVE_BUILD_CLASS") != "test-hooks" {
        return Err("Stage 3 checkpoint fixture requires a test-hooks native build".to_owned());
    }
    let graph_limits = fixture_graph_limits();
    let graph = Arc::new(
        GraphBundle::compile(fixture_graph(), &graph_limits)
            .map_err(|error| format!("fixture graph compilation failed: {error}"))?,
    );
    let settings = fixture_settings();
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)
        .map_err(|error| format!("fixture settings schema failed: {error}"))?;
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256: settings_schema_sha256.clone(),
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 1000.0,
        population_count: FIXTURE_POPULATION_COUNT,
        baseline_count: 0,
        max_world_snakes: FIXTURE_POPULATION_COUNT,
        max_non_population_brains: 0,
        max_body_points: 100_000,
        max_pellets: 10_000,
        spatial_index_bytes: 2 * 1024 * 1024,
        worker_scratch_bytes: 2 * 1024 * 1024,
        checkpoint_scratch_bytes: 16 * 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let build_identifier = env!("SLITHER_NATIVE_BUILD_IDENTIFIER").to_owned();
    let source_sha256 = env!("SLITHER_NATIVE_SOURCE_SHA256").to_owned();
    let target_triple = env!("SLITHER_NATIVE_BUILD_TARGET").to_owned();
    let build_profile = env!("SLITHER_NATIVE_BUILD_PROFILE").to_owned();
    let build_class = env!("SLITHER_NATIVE_BUILD_CLASS").to_owned();
    let rustc_version = env!("SLITHER_NATIVE_RUSTC_VERSION").to_owned();
    let build_contract_sha256 = env!("SLITHER_NATIVE_BUILD_CONTRACT_SHA256").to_owned();
    let config_hash = normalized_config_hash(&config)
        .map_err(|error| format!("fixture configuration hash failed: {error}"))?;
    let identity = RunIdentity {
        run_id: "11111111-2222-4333-8444-555555555555".to_owned(),
        seed: 7,
        config_revision: 1,
        config_hash,
        source_revision: build_identifier.clone(),
        engine_build_id: build_identifier.clone(),
        source_sha256: source_sha256.clone(),
        target_triple: target_triple.clone(),
        build_profile: build_profile.clone(),
        build_class: build_class.clone(),
        rustc_version: rustc_version.clone(),
        build_contract_sha256: build_contract_sha256.clone(),
        math_backend: "rust-scalar-v1".to_owned(),
    };
    let (population, brains) = fixture_population(&graph);
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
        rng: RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: rng(),
            evolution: rng(),
            external_controller: rng(),
            baselines: Vec::new(),
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1,
            next_brain_id: FIXTURE_POPULATION_COUNT as u64 + 1,
            next_genome_id: FIXTURE_POPULATION_COUNT as u64 + 1,
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
    let policy = StateAdmissionPolicy {
        memory_ceiling_bytes: 256 * 1024 * 1024,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: source_sha256,
        expected_target_triple: target_triple,
        expected_build_profile: build_profile,
        expected_build_class: build_class,
        expected_rustc_version: rustc_version,
        expected_build_contract_sha256: build_contract_sha256,
        expected_math_backend: "rust-scalar-v1".to_owned(),
        expected_settings_schema_sha256: settings_schema_sha256,
    };
    let state = AuthoritativeState::validate_and_own(candidate, graph, &policy)
        .map_err(|error| format!("fixture state admission failed: {error}"))?;
    let boundary = state
        .checkpoint_boundary()
        .map_err(|error| format!("fixture boundary failed: {error}"))?;
    let operation_id =
        CheckpointOperationId::parse(operation_id.to_owned()).map_err(|error| error.to_string())?;
    publish_checkpoint(
        managed_directory,
        operation_id,
        transition_epoch,
        boundary,
        &fixture_checkpoint_limits(),
        &graph_limits,
        &policy,
    )
    .map_err(|error| error.to_string())
}

/// Small recurrent graph that proves weights and recurrent state both use the real codec.
fn fixture_graph() -> GraphSpec {
    GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".to_owned(),
                kind: GraphNodeKind::Input { output_size: 3 },
            },
            GraphNodeSpec {
                id: "memory".to_owned(),
                kind: GraphNodeKind::Gru {
                    input_size: 3,
                    hidden_size: 2,
                },
            },
            GraphNodeSpec {
                id: "head".to_owned(),
                kind: GraphNodeKind::Dense {
                    input_size: 2,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".to_owned(),
                to: "memory".to_owned(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "memory".to_owned(),
                to: "head".to_owned(),
                from_port: None,
                to_port: None,
            },
        ],
        outputs: vec![GraphOutputRef {
            node_id: "head".to_owned(),
            port: None,
        }],
        output_size: 2,
    }
}

/// Complete path-sorted settings schema retained by the fixture checkpoint.
fn fixture_settings() -> Vec<NormalizedSetting> {
    vec![
        NormalizedSetting {
            path: "baselineBots.count".to_owned(),
            value: NormalizedSettingValue::Integer(0),
        },
        NormalizedSetting {
            path: "brain.sensorVersion".to_owned(),
            value: NormalizedSettingValue::Integer(i64::from(SENSOR_VERSION)),
        },
        NormalizedSetting {
            path: "simSpeed".to_owned(),
            value: NormalizedSettingValue::Float(1.0),
        },
        NormalizedSetting {
            path: "snakeCount".to_owned(),
            value: NormalizedSettingValue::Integer(FIXTURE_POPULATION_COUNT as i64),
        },
        NormalizedSetting {
            path: "worldRadius".to_owned(),
            value: NormalizedSettingValue::Integer(1000),
        },
    ]
}

/// Create distinct packed weights plus exact-zero generation-boundary recurrent buffers.
fn fixture_population(graph: &GraphBundle) -> (Vec<PopulationGenome>, Vec<BrainRuntimeState>) {
    let mut population = Vec::with_capacity(FIXTURE_POPULATION_COUNT);
    let mut brains = Vec::with_capacity(FIXTURE_POPULATION_COUNT);
    for slot in 0..FIXTURE_POPULATION_COUNT {
        let brain = BrainHandle {
            id: slot as u64 + 1,
            epoch: 1,
        };
        let weights = (0..graph.total_parameters)
            .map(|index| {
                let mantissa = ((slot as u32).wrapping_mul(8191) ^ (index as u32).wrapping_mul(37))
                    & 0x007f_ffff;
                f32::from_bits(0x3f00_0000 | mantissa)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let recurrent = vec![0.0_f32; graph.total_state_size].into_boxed_slice();
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
            weights,
        });
        brains.push(BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(slot as u32),
            non_population_weights: None,
            recurrent,
        });
    }
    (population, brains)
}

/// Reviewed graph limits for the fixed small fixture.
fn fixture_graph_limits() -> GraphLimits {
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

/// Reviewed checkpoint bounds for the fixed small fixture.
fn fixture_checkpoint_limits() -> CheckpointLimits {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Process-local suffix preventing parallel fixture tests from sharing a directory.
    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    /// Disposable directory removed even when an assertion unwinds.
    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        /// Create one unique directory beneath the operating-system temporary root.
        fn create() -> Self {
            let suffix = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "slither-stage3-checkpoint-fixture-{}-{suffix}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("fixture directory is created");
            Self(path)
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn admitted_generation_boundary_reaches_real_checkpoint_publication() {
        let directory = FixtureDirectory::create();
        let descriptor =
            publish_stage3_fixture(&directory.0, "0123456789abcdef0123456789abcdef", 1)
                .expect("valid fixture publishes through the real checkpoint codec");

        assert_eq!(descriptor.transition_epoch_hex, "0000000000000001");
        assert_ne!(descriptor.recurrent_state_count_hex, "0000000000000000");
        assert!(directory.0.join(descriptor.relative_filename).is_file());
    }
}
