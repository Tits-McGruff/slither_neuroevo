use super::*;

#[test]
fn production_checkpoint_limits_admit_approved_p3_raw_weight_envelope() {
    // P3 has 300 evolved genomes and the approved large GRU has 402,914 weights.
    // Raw fallback must remain admissible even when shuffle/Zstandard saves nothing.
    const P3_RAW_WEIGHT_BYTES: u64 = 300 * 402_914 * 4;
    let limits = production_checkpoint_limits();
    assert!(limits.max_numeric_stored_bytes >= P3_RAW_WEIGHT_BYTES);
    assert!(limits.max_numeric_candidate_bytes >= P3_RAW_WEIGHT_BYTES);
    assert!(limits.max_total_decoded_bytes >= P3_RAW_WEIGHT_BYTES);
    assert!(limits.max_archive_bytes >= P3_RAW_WEIGHT_BYTES);
}

#[test]
fn metadata_only_checkpoint_size_matches_published_recurrent_population() {
    let managed = TestDirectory::create("predicted-size");
    let prepared = prepare_stage6a_p0_boundary(
        request(42),
        &[FreshRunSettingUpdate {
            path: "snakeCount".into(),
            value: 3.0,
        }],
        typescript_default_graph_spec(),
    )
    .unwrap();
    assert!(prepared.graph.total_state_size > 0);
    let mut shell = prepared.candidate.clone();
    for genome in &mut shell.population {
        genome.weights = Box::new([]);
    }
    for brain in &mut shell.brains {
        brain.recurrent = Box::new([]);
    }
    let predicted = boundary_decoded_byte_count(
        &shell,
        prepared.graph.spec(),
        prepared.graph.compiled(),
        &prepared.checkpoint_limits,
    )
    .unwrap();
    let state = AuthoritativeState::validate_and_own(
        prepared.candidate,
        prepared.graph,
        &prepared.admission_policy,
    )
    .unwrap();
    let descriptor = crate::engine::checkpoint::publish_checkpoint(
        managed.path(),
        CheckpointOperationId::parse("42424242424242424242424242424242").unwrap(),
        state.world_epoch(),
        state.checkpoint_boundary().unwrap(),
        &prepared.checkpoint_limits,
        &prepared.graph_limits,
        &prepared.admission_policy,
    )
    .unwrap();
    assert_eq!(
        predicted,
        u64::from_str_radix(&descriptor.decoded_byte_count_hex, 16).unwrap()
    );
    assert!(
        predicted
            > 4 * 3 * (state.graph().total_parameters + state.graph().total_state_size) as u64
    );
}

#[test]
fn oversized_population_rejects_before_numeric_initialization() {
    use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef};
    let graph = GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: 83 },
            },
            GraphNodeSpec {
                id: "large".into(),
                kind: GraphNodeKind::Mlp {
                    input_size: 83,
                    hidden_sizes: vec![512, 512],
                    output_size: 512,
                },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: 512,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".into(),
                to: "large".into(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "large".into(),
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
    let error = match prepare_stage6a_p0_boundary(
        Stage6aP0FreshRunRequest {
            memory_ceiling_bytes: 4 * 1024 * MIB,
            ..request(42)
        },
        &[
            FreshRunSettingUpdate {
                path: "snakeCount".into(),
                value: 300.0,
            },
            FreshRunSettingUpdate {
                path: "baselineBots.count".into(),
                value: 0.0,
            },
        ],
        graph,
    ) {
        Ok(_) => panic!("oversized population must reject before initialization"),
        Err(error) => error,
    };
    assert!(matches!(error, FreshRunError::CheckpointTooLarge {
        decoded_bytes, limit_bytes: 536_870_912, population_count: 300,
        parameters_per_genome: 569_346,
    } if decoded_bytes > 300 * 569_346 * 4));
    assert!(error
        .to_string()
        .contains("Reduce NPC snakes or brain size"));
}

#[test]
fn invalid_identity_or_memory_fails_before_a_pending_transition_exists() {
    for run_id in ["", "bad\0id"] {
        let error = prepare_stage6a_p0_fresh_run(Stage6aP0FreshRunRequest {
            run_id: run_id.to_owned(),
            ..request(11)
        })
        .expect_err("invalid run ID must reject");
        assert!(matches!(error, FreshRunError::InvalidRunId));
    }
    let error = prepare_stage6a_p0_fresh_run(Stage6aP0FreshRunRequest {
        memory_ceiling_bytes: 1,
        ..request(11)
    })
    .expect_err("insufficient state ceiling must reject before numeric allocation");
    assert!(matches!(
        error,
        FreshRunError::State(ref state)
            if matches!(state.as_ref(), StateError::MemoryCeilingExceeded { .. })
    ));
}
