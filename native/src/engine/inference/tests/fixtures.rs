use super::*;

pub(super) const EPOCH: u64 = 7;

/// Limits large enough for deterministic unit fixtures without implying production defaults.
pub(super) fn limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 64,
        max_edges: 128,
        max_graph_outputs: 8,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 16_384,
        max_tensor_width: 512,
        max_mlp_hidden_layers: 8,
        max_split_output_ports: 8,
        max_parameter_floats: 1_000_000,
        max_recurrent_state_floats: 16_384,
        max_canonical_layout_bytes: 1_000_000,
        max_architecture_key_bytes: 2_000_000,
    }
}

/// Graph covering Input, Split, Dense, MLP, Concat, GRU, LSTM, RRU, and output mapping.
pub(super) fn complete_graph() -> CompiledGraph {
    compile_graph(
        &GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "in".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 2 },
                },
                GraphNodeSpec {
                    id: "split".to_owned(),
                    kind: GraphNodeKind::Split {
                        output_sizes: vec![1, 1],
                    },
                },
                GraphNodeSpec {
                    id: "denseA".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 1,
                        output_size: 1,
                    },
                },
                GraphNodeSpec {
                    id: "mlpB".to_owned(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 1,
                        hidden_sizes: vec![2],
                        output_size: 1,
                    },
                },
                GraphNodeSpec {
                    id: "features".to_owned(),
                    kind: GraphNodeKind::Concat,
                },
                GraphNodeSpec {
                    id: "gru".to_owned(),
                    kind: GraphNodeKind::Gru {
                        input_size: 2,
                        hidden_size: 1,
                    },
                },
                GraphNodeSpec {
                    id: "lstm".to_owned(),
                    kind: GraphNodeKind::Lstm {
                        input_size: 2,
                        hidden_size: 1,
                    },
                },
                GraphNodeSpec {
                    id: "rru".to_owned(),
                    kind: GraphNodeKind::Rru {
                        input_size: 2,
                        hidden_size: 1,
                    },
                },
                GraphNodeSpec {
                    id: "memory".to_owned(),
                    kind: GraphNodeKind::Concat,
                },
                GraphNodeSpec {
                    id: "head".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 3,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                edge("in", "split", None, None),
                edge("split", "denseA", Some(0), None),
                edge("split", "mlpB", Some(1), None),
                edge("denseA", "features", None, Some(0)),
                edge("mlpB", "features", None, Some(1)),
                edge("features", "gru", None, None),
                edge("features", "lstm", None, None),
                edge("features", "rru", None, None),
                edge("gru", "memory", None, Some(0)),
                edge("lstm", "memory", None, Some(1)),
                edge("rru", "memory", None, Some(2)),
                edge("memory", "head", None, None),
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
        &limits(),
    )
    .expect("complete graph compiles")
}

/// Hidden-width-two graph that exposes cross-hidden recurrent indexing mistakes.
pub(super) fn wide_recurrent_graph() -> CompiledGraph {
    compile_graph(
        &GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 3 },
                },
                GraphNodeSpec {
                    id: "gru".to_owned(),
                    kind: GraphNodeKind::Gru {
                        input_size: 3,
                        hidden_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "lstm".to_owned(),
                    kind: GraphNodeKind::Lstm {
                        input_size: 3,
                        hidden_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "rru".to_owned(),
                    kind: GraphNodeKind::Rru {
                        input_size: 3,
                        hidden_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "memory".to_owned(),
                    kind: GraphNodeKind::Concat,
                },
                GraphNodeSpec {
                    id: "head".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 6,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                edge("input", "gru", None, None),
                edge("input", "lstm", None, None),
                edge("input", "rru", None, None),
                edge("gru", "memory", None, Some(0)),
                edge("lstm", "memory", None, Some(1)),
                edge("rru", "memory", None, Some(2)),
                edge("memory", "head", None, None),
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
        &limits(),
    )
    .expect("wide recurrent graph compiles")
}

/// Large mixed-recurrent DAG forces long/tail SIMD rows through every graph operation.
pub(super) fn large_recurrent_graph() -> CompiledGraph {
    let nodes = [
        ("in", GraphNodeKind::Input { output_size: 504 }),
        (
            "split",
            GraphNodeKind::Split {
                output_sizes: vec![248, 256],
            },
        ),
        (
            "denseA",
            GraphNodeKind::Dense {
                input_size: 248,
                output_size: 32,
            },
        ),
        (
            "mlpB",
            GraphNodeKind::Mlp {
                input_size: 256,
                hidden_sizes: vec![64],
                output_size: 32,
            },
        ),
        ("features", GraphNodeKind::Concat),
        (
            "gru",
            GraphNodeKind::Gru {
                input_size: 64,
                hidden_size: 32,
            },
        ),
        (
            "lstm",
            GraphNodeKind::Lstm {
                input_size: 64,
                hidden_size: 32,
            },
        ),
        (
            "rru",
            GraphNodeKind::Rru {
                input_size: 64,
                hidden_size: 32,
            },
        ),
        ("memory", GraphNodeKind::Concat),
        (
            "head",
            GraphNodeKind::Dense {
                input_size: 96,
                output_size: 2,
            },
        ),
    ]
    .into_iter()
    .map(|(id, kind)| GraphNodeSpec {
        id: id.to_owned(),
        kind,
    })
    .collect();
    compile_graph(
        &GraphSpec {
            nodes,
            edges: vec![
                edge("in", "split", None, None),
                edge("split", "denseA", Some(0), None),
                edge("split", "mlpB", Some(1), None),
                edge("denseA", "features", None, Some(0)),
                edge("mlpB", "features", None, Some(1)),
                edge("features", "gru", None, None),
                edge("features", "lstm", None, None),
                edge("features", "rru", None, None),
                edge("gru", "memory", None, Some(0)),
                edge("lstm", "memory", None, Some(1)),
                edge("rru", "memory", None, Some(2)),
                edge("memory", "head", None, None),
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
        &limits(),
    )
    .unwrap()
}

/// Construct one graph edge with explicit optional ports.
pub(super) fn edge(
    from: &str,
    to: &str,
    from_port: Option<i64>,
    to_port: Option<i64>,
) -> GraphEdge {
    GraphEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        from_port,
        to_port,
    }
}

/// Current-TypeScript parity weights, quantized exactly through `Float32Array.from`.
pub(super) fn parity_weights(count: usize) -> Vec<f32> {
    (0..count)
        .map(|index| (((index * 37) % 101) as i32 - 50) as f32 / 200.0)
        .collect()
}

/// Allocate admitted scratch for one execution plan.
pub(super) fn scratch(plan: &GraphExecutionPlan) -> CalculationScratch {
    CalculationScratch::try_new(plan.scratch_layout(), usize::MAX)
        .expect("fixture scratch allocation")
}

/// Compare explicit `f32` tolerance fixtures with useful failure context.
pub(super) fn assert_close(label: &str, actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len(), "{label} length");
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let difference = (*actual - *expected).abs();
        assert!(
            difference <= 2.0e-6,
            "{label}[{index}] actual={actual} expected={expected} difference={difference}"
        );
    }
}

/// One fully populated evolved snake record for calculation-work resolution.
pub(super) fn snake(slot: usize, handle: BrainHandle) -> SnakeState {
    SnakeState {
        id: 10_000 + slot as u64,
        frame_v1_id: u32::try_from(slot + 1).unwrap(),
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(u32::try_from(slot).unwrap()),
        brain: Some(handle),
        baseline_slot: None,
        baseline_strategy: None::<BaselineStrategyState>,
        position: WorldPoint { x: 0.0, y: 0.0 },
        previous_position: WorldPoint { x: 0.0, y: 0.0 },
        direction: 0.0,
        radius: 1.0,
        speed: 0.0,
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
        body: BodyRange { start: 0, len: 0 },
        skin: 0,
    }
}

/// Build 55 distinct genomes and recurrent states using stable dense slots.
pub(super) fn population_fixture(
    plan: &GraphExecutionPlan,
    count: usize,
) -> (
    Vec<SnakeState>,
    Vec<BrainRuntimeState>,
    Vec<PopulationGenome>,
) {
    let base = parity_weights(plan.total_parameters());
    let mut snakes = Vec::with_capacity(count);
    let mut brains = Vec::with_capacity(count);
    let mut population = Vec::with_capacity(count);
    for slot in 0..count {
        let handle = BrainHandle {
            id: 100 + slot as u64,
            epoch: EPOCH,
        };
        snakes.push(snake(slot, handle));
        brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::PopulationSlot(u32::try_from(slot).unwrap()),
            retained_origin: None,
            non_population_weights: None,
            recurrent: (0..plan.total_state_size())
                .map(|state_index| (slot * 7 + state_index) as f32 / 10_000.0)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        });
        population.push(PopulationGenome {
            slot: u32::try_from(slot).unwrap(),
            brain: handle,
            lineage: GenomeLineage {
                genome_id: 1_000 + slot as u64,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights: base
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    *value + (slot as f32 + 1.0) * 0.000_13 + index as f32 * 0.000_001
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        });
    }
    (snakes, brains, population)
}

/// Prepare deterministic work after accepting candidates in the supplied order.
pub(super) fn prepared_workspace(
    plan: &GraphExecutionPlan,
    order: &[usize],
    snakes: &[SnakeState],
    brains: &[BrainRuntimeState],
    population: &[PopulationGenome],
) -> CalculationWorkspace<[f32; 2]> {
    let mut workspace =
        CalculationWorkspace::try_new(snakes.len(), 1, plan.scratch_layout(), usize::MAX).unwrap();
    workspace.begin(CalculationBatchKey::new(1, 1, EPOCH));
    for index in order {
        workspace.try_push_candidate(*index, *index).unwrap();
    }
    workspace.prepare(snakes, brains, population).unwrap();
    workspace
}
