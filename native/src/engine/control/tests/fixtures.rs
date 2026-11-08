use super::*;

pub(super) const EPOCH: u64 = 31;

pub(super) fn graph_limits() -> GraphLimits {
    GraphLimits {
        max_nodes: 16,
        max_edges: 16,
        max_graph_outputs: 4,
        max_identifier_bytes: 64,
        max_total_referenced_identifier_bytes: 4_096,
        max_tensor_width: 1_024,
        max_mlp_hidden_layers: 4,
        max_split_output_ports: 4,
        max_parameter_floats: 100_000,
        max_recurrent_state_floats: 1_024,
        max_canonical_layout_bytes: 100_000,
        max_architecture_key_bytes: 200_000,
    }
}

pub(super) fn sensor_config() -> SensorConfig {
    SensorConfig {
        bins: 8,
        ..SensorConfig::default()
    }
}

pub(super) fn graph_plan(input_size: usize) -> GraphExecutionPlan {
    let graph = compile_graph(
        &GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input {
                        output_size: input_size,
                    },
                },
                GraphNodeSpec {
                    id: "memory".to_owned(),
                    kind: GraphNodeKind::Rru {
                        input_size,
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
        },
        &graph_limits(),
    )
    .unwrap();
    GraphExecutionPlan::build(&graph).unwrap()
}

pub(super) fn production_scale_graph_plan() -> GraphExecutionPlan {
    let graph = compile_graph(
        &GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 264 },
                },
                GraphNodeSpec {
                    id: "mlp-a".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 264,
                        output_size: 396,
                    },
                },
                GraphNodeSpec {
                    id: "LSTM".to_owned(),
                    kind: GraphNodeKind::Lstm {
                        input_size: 264,
                        hidden_size: 396,
                    },
                },
                GraphNodeSpec {
                    id: "concat".to_owned(),
                    kind: GraphNodeKind::Concat,
                },
                GraphNodeSpec {
                    id: "mlp-b".to_owned(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 792,
                        hidden_sizes: vec![264, 134],
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                GraphEdge {
                    from: "input".to_owned(),
                    to: "mlp-a".to_owned(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "input".to_owned(),
                    to: "LSTM".to_owned(),
                    from_port: None,
                    to_port: None,
                },
                GraphEdge {
                    from: "mlp-a".to_owned(),
                    to: "concat".to_owned(),
                    from_port: None,
                    to_port: Some(0),
                },
                GraphEdge {
                    from: "LSTM".to_owned(),
                    to: "concat".to_owned(),
                    from_port: None,
                    to_port: Some(1),
                },
                GraphEdge {
                    from: "concat".to_owned(),
                    to: "mlp-b".to_owned(),
                    from_port: None,
                    to_port: None,
                },
            ],
            outputs: vec![GraphOutputRef {
                node_id: "mlp-b".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
        &crate::engine::fresh_run::stage6a_p0_graph_limits(),
    )
    .unwrap();
    assert_eq!(graph.total_parameters, 1_397_096);
    GraphExecutionPlan::build(&graph).unwrap()
}

pub(super) fn snake(index: usize, handle: BrainHandle, body_start: usize) -> SnakeState {
    SnakeState {
        id: 1_000 + index as u64,
        frame_v1_id: u32::try_from(index + 1).unwrap(),
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(u32::try_from(index).unwrap()),
        brain: Some(handle),
        baseline_slot: None,
        baseline_strategy: None::<BaselineStrategyState>,
        position: WorldPoint {
            x: index as f64 * 180.0 - 90.0,
            y: index as f64 * 35.0,
        },
        previous_position: WorldPoint {
            x: index as f64 * 180.0 - 95.0,
            y: index as f64 * 35.0,
        },
        direction: index as f64 * 0.35,
        radius: 11.0,
        speed: 120.0,
        boost: index.is_multiple_of(2),
        age_seconds: 4.0 + index as f64,
        food: 2.0 + index as f64,
        points: 5.0 + index as f64 * 2.0,
        kills: index as u64,
        target_length: 8.0,
        fitness: 0.0,
        turn: 0.0,
        previous_turn: 0.0,
        input_boost: false,
        previous_input_boost: false,
        control_accumulator_seconds: 0.0,
        delivered_observation_points: 1.0 + index as f64,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body: BodyRange {
            start: body_start,
            len: 3,
        },
        skin: u32::try_from(index).unwrap(),
    }
}

pub(super) fn fixture(
    plan: &GraphExecutionPlan,
    epoch: u64,
) -> (WorldState, Vec<BrainRuntimeState>, Vec<PopulationGenome>) {
    fixture_with_count(plan, epoch, 3)
}

pub(super) fn fixture_with_count(
    plan: &GraphExecutionPlan,
    epoch: u64,
    count: usize,
) -> (WorldState, Vec<BrainRuntimeState>, Vec<PopulationGenome>) {
    let mut body_points = Vec::new();
    let mut snakes = Vec::new();
    let mut brains = Vec::new();
    let mut population = Vec::new();
    for index in 0..count {
        let handle = BrainHandle {
            id: 400 + index as u64,
            epoch,
        };
        let body_start = body_points.len();
        let head = WorldPoint {
            x: index as f64 * 180.0 - 90.0,
            y: index as f64 * 35.0,
        };
        body_points.extend_from_slice(&[
            head,
            WorldPoint {
                x: head.x - 18.0,
                y: head.y - 2.0,
            },
            WorldPoint {
                x: head.x - 36.0,
                y: head.y - 5.0,
            },
        ]);
        snakes.push(snake(index, handle, body_start));
        brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::PopulationSlot(u32::try_from(index).unwrap()),
            retained_origin: None,
            non_population_weights: None,
            recurrent: (0..plan.total_state_size())
                .map(|state| (index * 5 + state + 1) as f32 / 100.0)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        });
        population.push(PopulationGenome {
            slot: u32::try_from(index).unwrap(),
            brain: handle,
            lineage: GenomeLineage {
                genome_id: 900 + index as u64,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights: (0..plan.total_parameters())
                .map(|weight| (((weight * 17 + index * 29) % 97) as f32 - 48.0) / 180.0)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        });
    }
    let pellets = vec![
        PelletState {
            id: 11,
            position: WorldPoint { x: 35.0, y: 80.0 },
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        },
        PelletState {
            id: 12,
            position: WorldPoint {
                x: -120.0,
                y: -40.0,
            },
            value: 2.0,
            kind: 1,
            color: 2,
            owner: None,
        },
    ];
    (
        WorldState {
            ray_cursor: Default::default(),
            snakes,
            body_points,
            pellets,
            controller_leases: Vec::new(),
        },
        brains,
        population,
    )
}

pub(super) fn indexed(world: &WorldState) -> IndexedSensorWorld<'_> {
    IndexedSensorWorld::build(
        world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 10_000,
            maximum_pellet_entries: 1_000,
        },
    )
    .unwrap()
}

pub(super) fn candidates() -> Vec<CalculationCandidateIndex> {
    [2, 0, 1]
        .into_iter()
        .map(|index| CalculationCandidateIndex::new(index, index))
        .collect()
}

pub(super) fn pipeline(plan: GraphExecutionPlan) -> NeuralControlPipeline {
    let sensor = SensorEvaluator::new(sensor_config()).unwrap();
    NeuralControlPipeline::try_new(3, sensor, plan, 1, usize::MAX).unwrap()
}
