use super::*;

pub(super) const DT: f64 = 1.0 / 60.0;
pub(super) const EPOCH: u64 = 5;

pub(super) struct Fixture {
    pub(super) world: WorldState,
    pub(super) rng: RngStateBundle,
    pub(super) allocators: AllocatorState,
    pub(super) lifecycle: BaselineLifecycleState,
    pub(super) brains: Vec<BrainRuntimeState>,
    pub(super) population: Vec<PopulationGenome>,
}

pub(super) fn key(operation_epoch: u64) -> PhysicsStepKey {
    PhysicsStepKey::new(7, 3, 40, EPOCH, 9, [0x5a; 32], operation_epoch)
}

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

pub(super) fn graph_plan() -> GraphExecutionPlan {
    graph_plan_for_width(51)
}

pub(super) fn graph_plan_for_width(width: usize) -> GraphExecutionPlan {
    let graph = compile_graph(
        &GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: width },
                },
                GraphNodeSpec {
                    id: "memory".to_owned(),
                    kind: GraphNodeKind::Rru {
                        input_size: width,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn snake(
    id: u64,
    frame_id: u32,
    kind: SnakeKind,
    position: WorldPoint,
    body: BodyRange,
    brain: Option<BrainHandle>,
    population_slot: Option<u32>,
    baseline_slot: Option<u32>,
    accumulator: f64,
) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: frame_id,
        kind,
        alive: true,
        population_slot,
        brain,
        baseline_slot,
        baseline_strategy: baseline_slot.map(|_| BaselineStrategyState::Roam),
        position,
        previous_position: WorldPoint {
            x: position.x - 1.0,
            y: position.y,
        },
        direction: f64::from(frame_id) * 0.05,
        radius: 9.0,
        speed: 165.0,
        boost: false,
        age_seconds: 2.0,
        food: 1.0,
        points: 5.0 + f64::from(frame_id),
        kills: 0,
        target_length: 3.0,
        fitness: 0.0,
        turn: -0.2,
        previous_turn: 0.1,
        input_boost: false,
        previous_input_boost: true,
        control_accumulator_seconds: accumulator,
        delivered_observation_points: 1.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body,
        skin: frame_id,
    }
}

pub(super) fn fixture(plan: &GraphExecutionPlan) -> Fixture {
    let evolved = BrainHandle {
        id: 100,
        epoch: EPOCH,
    };
    let connected = BrainHandle {
        id: 101,
        epoch: EPOCH,
    };
    let external_only = BrainHandle {
        id: 102,
        epoch: EPOCH,
    };
    let takeover = BrainHandle {
        id: 103,
        epoch: EPOCH,
    };
    let grace = BrainHandle {
        id: 104,
        epoch: EPOCH,
    };
    let descriptors = [
        (40, 5, SnakeKind::External, Some(takeover), None, None, 0.0),
        (20, 2, SnakeKind::Baseline, None, None, Some(0), 0.0),
        (10, 1, SnakeKind::Evolved, Some(evolved), Some(0), None, DT),
        (
            35,
            4,
            SnakeKind::External,
            Some(external_only),
            None,
            None,
            0.03,
        ),
        (
            30,
            3,
            SnakeKind::External,
            Some(connected),
            None,
            None,
            0.03,
        ),
        (50, 6, SnakeKind::External, Some(grace), None, None, 0.04),
    ];
    let mut body_points = Vec::new();
    let mut snakes = Vec::new();
    for (ordinal, (id, frame_id, kind, brain, population_slot, baseline_slot, accumulator)) in
        descriptors.into_iter().enumerate()
    {
        let position = WorldPoint {
            x: ordinal as f64 * 260.0 - 520.0,
            y: ordinal as f64 * 45.0,
        };
        let start = body_points.len();
        body_points.extend([
            position,
            WorldPoint {
                x: position.x - 15.0,
                y: position.y,
            },
            WorldPoint {
                x: position.x - 30.0,
                y: position.y,
            },
        ]);
        snakes.push(snake(
            id,
            frame_id,
            kind,
            position,
            BodyRange { start, len: 3 },
            brain,
            population_slot,
            baseline_slot,
            accumulator,
        ));
    }
    for snake in &mut snakes {
        if matches!(snake.id, 40 | 50) {
            snake.turn = 0.0;
            snake.input_boost = false;
        }
    }

    let weights = |salt: usize| {
        (0..plan.total_parameters())
            .map(|index| (((index * 19 + salt * 31) % 101) as f32 - 50.0) / 180.0)
            .collect::<Vec<_>>()
            .into_boxed_slice()
    };
    let recurrent = |salt: usize| {
        (0..plan.total_state_size())
            .map(|index| (salt * 5 + index + 1) as f32 / 100.0)
            .collect::<Vec<_>>()
            .into_boxed_slice()
    };
    let brains = vec![
        BrainRuntimeState {
            handle: takeover,
            owner: BrainOwner::Entity(40),
            retained_origin: None,
            non_population_weights: Some(weights(4)),
            recurrent: recurrent(4),
        },
        BrainRuntimeState {
            handle: evolved,
            owner: BrainOwner::PopulationSlot(0),
            retained_origin: None,
            non_population_weights: None,
            recurrent: recurrent(1),
        },
        BrainRuntimeState {
            handle: external_only,
            owner: BrainOwner::Entity(35),
            retained_origin: None,
            non_population_weights: Some(weights(3)),
            recurrent: recurrent(3),
        },
        BrainRuntimeState {
            handle: connected,
            owner: BrainOwner::Entity(30),
            retained_origin: None,
            non_population_weights: Some(weights(2)),
            recurrent: recurrent(2),
        },
        BrainRuntimeState {
            handle: grace,
            owner: BrainOwner::Entity(50),
            retained_origin: None,
            non_population_weights: Some(weights(5)),
            recurrent: recurrent(5),
        },
    ];
    let population = vec![PopulationGenome {
        slot: 0,
        brain: evolved,
        lineage: GenomeLineage {
            genome_id: 500,
            birth_generation: 1,
            parent_a: None,
            parent_b: None,
        },
        fitness: 0.0,
        weights: weights(1),
    }];
    let controller_leases = vec![
        ControllerLease {
            reward: None,
            identity_key: String::new(),
            id: 2,
            snake_id: 40,
            kind: ControllerKind::ReinforcementLearning,
            connection_id: None,
            scope: "run:7:rl".to_owned(),
            resume_token: "takeover-token".to_owned(),
            status: ControllerLeaseStatus::ReservedNeutral,
            latest_action: LatestControllerAction {
                is_explicit: false,
                turn: -0.5,
                boost: true,
                client_tick: 1,
                arrival_sequence: 1,
                accepted_at_ms: 900,
            },
            last_observed_at_ms: 1_000,
            disconnected_at_ms: Some(1_000),
            input_hold_expires_at_ms: Some(1_400),
            grace_expires_at_ms: Some(31_000),
            takeover_committed_at_ms: None,
        },
        ControllerLease {
            reward: None,
            identity_key: String::new(),
            id: 1,
            snake_id: 30,
            kind: ControllerKind::Player,
            connection_id: Some(700),
            scope: "run:7:player".to_owned(),
            resume_token: "connected-token".to_owned(),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                is_explicit: false,
                turn: 0.75,
                boost: true,
                client_tick: 2,
                arrival_sequence: 2,
                accepted_at_ms: 30_800,
            },
            last_observed_at_ms: 30_800,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        },
        ControllerLease {
            reward: None,
            identity_key: String::new(),
            id: 3,
            snake_id: 50,
            kind: ControllerKind::Player,
            connection_id: None,
            scope: "run:7:grace".to_owned(),
            resume_token: "grace-token".to_owned(),
            status: ControllerLeaseStatus::ReservedNeutral,
            latest_action: LatestControllerAction {
                is_explicit: false,
                turn: 0.6,
                boost: true,
                client_tick: 3,
                arrival_sequence: 3,
                accepted_at_ms: 30_400,
            },
            last_observed_at_ms: 30_500,
            disconnected_at_ms: Some(30_500),
            input_hold_expires_at_ms: Some(30_900),
            grace_expires_at_ms: Some(60_500),
            takeover_committed_at_ms: None,
        },
    ];
    let world = WorldState {
        ray_cursor: Default::default(),
        snakes,
        body_points,
        pellets: vec![PelletState {
            id: 80,
            position: WorldPoint { x: 80.0, y: 90.0 },
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        }],
        controller_leases,
    };
    let rng = RngStateBundle {
        baseline_behaviors: Vec::new(),
        evolved_respawns: Vec::new(),
        version: RNG_BUNDLE_VERSION,
        world: StatefulRng::new(11.0).export_state(),
        evolution: StatefulRng::new(12.0).export_state(),
        external_controller: StatefulRng::new(13.0).export_state(),
        baselines: vec![BaselineRngState {
            slot: 0,
            state: StatefulRng::new(14.0).export_state(),
        }],
    };
    let allocators = AllocatorState {
        version: ALLOCATOR_VERSION,
        next_entity_id: 100,
        next_brain_id: 200,
        next_genome_id: 600,
        next_controller_lease_id: 4,
        next_frame_v1_id: 81,
        next_external_id: 1_000_000_000_100,
        next_baseline_id: 2_000_000_000_100,
        next_resurrected_id: 3_000_000_000_100,
    };
    let lifecycle = BaselineLifecycleState {
        version: BASELINE_LIFECYCLE_VERSION,
        slots: vec![BaselineSlotRuntime {
            v2_policy: None,
            v2_last_control_step: None,
            slot: 0,
            snake_id: 20,
            strategy_timer_seconds: 0.0,
            wander_angle: 0.0,
            wander_timer_seconds: 1.0,
            turn: 0.0,
            boost: false,
            respawn_remaining_seconds: None,
        }],
    };
    Fixture {
        world,
        rng,
        allocators,
        lifecycle,
        brains,
        population,
    }
}

pub(super) fn prefix_config() -> FixedStepPrefixConfig {
    let mut ambient = AmbientPelletConfig::typescript_defaults();
    ambient.target_count = 1;
    ambient.spawn_per_second = 0.0;
    FixedStepPrefixConfig {
        fixed_dt: DT,
        ambient,
        baseline: BaselineLifecycleConfig {
            slot_count: 1,
            ..BaselineLifecycleConfig::typescript_defaults()
        },
        maximum_snakes: 8,
        maximum_body_points: 1_000,
        maximum_pellets: 256,
        ..FixedStepPrefixConfig::typescript_defaults()
    }
}

pub(super) fn control_config() -> ControlPhaseConfig {
    ControlPhaseConfig {
        baseline: BaselineControlConfig {
            maximum_world_snakes: 8,
            ..BaselineControlConfig::typescript_defaults()
        },
        maximum_snakes: 8,
        maximum_brains: 8,
        maximum_external_observations: 4,
        sensor_index: SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 10_000,
            maximum_pellet_entries: 1_000,
        },
        ..ControlPhaseConfig::typescript_defaults()
    }
}

pub(super) fn control_workspace(plan: GraphExecutionPlan) -> ControlPhaseWorkspace {
    let sensor = SensorEvaluator::new(SensorConfig {
        bins: (plan.input_size() - 19) / 4,
        ..SensorConfig::default()
    })
    .unwrap();
    let neural = NeuralControlPipeline::try_new(8, sensor, plan, 1, usize::MAX).unwrap();
    ControlPhaseWorkspace::new(neural).unwrap()
}

pub(super) fn world_step_config() -> WorldStepConfig {
    let prefix = prefix_config();
    let control = control_config();
    let mut physics = PhysicsConfig::typescript_defaults();
    physics.maximum_body_points = prefix.maximum_body_points;
    physics.maximum_pellets = prefix.maximum_pellets;
    physics.maximum_pellet_index_entries = 1_000;
    let mut external_replacement = WorldStepConfig::typescript_defaults().external_replacement;
    external_replacement.spawn = prefix.baseline_spawn;
    external_replacement.snake_base_speed = prefix.baseline_snake_base_speed;
    external_replacement.controller_timing = control.controller_timing;
    external_replacement.maximum_snakes = prefix.maximum_snakes;
    external_replacement.maximum_body_points = prefix.maximum_body_points;
    external_replacement.maximum_brains = control.maximum_brains;
    WorldStepConfig {
        prefix,
        control,
        physics,
        baseline: prefix.baseline,
        external_replacement,
        physics_substeps: 3,
        algorithm_version: WORLD_STEP_VERSION,
    }
}

pub(super) fn make_fixture_bodies_physics_ready(world: &mut WorldState) {
    let mut body_points = Vec::with_capacity(world.snakes.len() * 5);
    for snake in &mut world.snakes {
        let start = body_points.len();
        for offset in 0..5 {
            body_points.push(WorldPoint {
                x: snake.position.x - offset as f64 * 15.0,
                y: snake.position.y,
            });
        }
        snake.body = BodyRange { start, len: 5 };
        snake.target_length = 5.0;
    }
    world.body_points = body_points;
}

#[derive(Debug, PartialEq)]
pub(super) struct BaselineSummary {
    pub(super) snake_id: u64,
    pub(super) slot: u32,
    pub(super) next_slot: BaselineSlotRuntime,
    pub(super) next_strategy: BaselineStrategyState,
    pub(super) next_rng: BaselineRngState,
    pub(super) delivery: ObservationDeliveryMarker,
    pub(super) diagnostics: BaselineControlDiagnostics,
    pub(super) sensor_diagnostics: SensorSampleDiagnostics,
}

#[derive(Debug, PartialEq)]
pub(super) struct ExternalSummary {
    pub(super) lease_id: u64,
    pub(super) connection_id: u64,
    pub(super) kind: ControllerKind,
    pub(super) position_x_bits: u64,
    pub(super) position_y_bits: u64,
    pub(super) direction_bits: u64,
    pub(super) delivery: ObservationDeliveryMarker,
    pub(super) diagnostics: SensorSampleDiagnostics,
    pub(super) observation_bits: Vec<u32>,
}

#[derive(Debug, PartialEq)]
pub(super) struct ControlSummary {
    pub(super) calculation_key: CalculationBatchKey,
    pub(super) controls: Vec<(u64, SelectedControlSource, u32, bool, u64)>,
    pub(super) transitions: Vec<ControllerBoundaryProposal>,
    pub(super) baselines: Vec<BaselineSummary>,
    pub(super) external: Vec<ExternalSummary>,
    pub(super) counts: (usize, usize, usize, usize, usize, usize),
}

pub(super) fn prepared_summary_for(plan: GraphExecutionPlan, fixture: &Fixture) -> ControlSummary {
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(12),
            world: &fixture.world,
            rng: &fixture.rng,
            allocators: &fixture.allocators,
            generation_elapsed_seconds: 2.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &fixture.lifecycle,
            config: prefix_config(),
        })
        .unwrap();
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(prefix.world()).unwrap();
    let mut workspace = control_workspace(plan);
    let prepared = workspace
        .prepare(ControlPhaseInputs {
            prefix,
            generation: &generation,
            assessment_points: None,
            v4: None,
            population: &fixture.population,
            brains: &fixture.brains,
            wall_now_ms: 31_000,
            config: control_config(),
        })
        .unwrap();
    let controls = prepared
        .control_updates()
        .iter()
        .map(|update| {
            (
                update.snake_id(),
                update.source(),
                update.turn().to_bits(),
                update.boost(),
                update.next_control_accumulator_seconds().to_bits(),
            )
        })
        .collect();
    let transitions = prepared
        .controller_transitions()
        .iter()
        .map(|transition| transition.proposal())
        .collect();
    let baselines = prepared
        .baseline_controls()
        .iter()
        .map(|result| BaselineSummary {
            snake_id: result.snake_id(),
            slot: result.slot(),
            next_slot: result.next_slot(),
            next_strategy: result.next_strategy(),
            next_rng: result.next_rng().clone(),
            delivery: result.delivery(),
            diagnostics: result.diagnostics(),
            sensor_diagnostics: result.sensor_diagnostics(),
        })
        .collect();
    let external = (0..prepared.external_events().len())
        .map(|index| {
            let (event, observation) = prepared.external_observation(index).unwrap();
            ExternalSummary {
                lease_id: event.lease_id(),
                connection_id: event.connection_id(),
                kind: event.kind(),
                position_x_bits: event.position().x.to_bits(),
                position_y_bits: event.position().y.to_bits(),
                direction_bits: event.direction().to_bits(),
                delivery: event.delivery(),
                diagnostics: event.diagnostics(),
                observation_bits: observation.iter().map(|value| value.to_bits()).collect(),
            }
        })
        .collect();
    let diagnostics = prepared.diagnostics();
    ControlSummary {
        calculation_key: prepared.calculation_key(),
        controls,
        transitions,
        baselines,
        external,
        counts: (
            diagnostics.controls,
            diagnostics.baseline_observations,
            diagnostics.external_observations,
            diagnostics.neural_evaluations,
            diagnostics.neural_held,
            diagnostics.neural_takeovers,
        ),
    }
}

pub(super) fn prepared_summary(reverse_storage: bool) -> ControlSummary {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    if reverse_storage {
        fixture.world.snakes.reverse();
        fixture.world.controller_leases.reverse();
        fixture.brains.reverse();
    }
    prepared_summary_for(plan, &fixture)
}

pub(super) fn summarized_control(
    summary: &ControlSummary,
    snake_id: u64,
) -> &(u64, SelectedControlSource, u32, bool, u64) {
    summary
        .controls
        .iter()
        .find(|control| control.0 == snake_id)
        .unwrap()
}

pub(super) fn v4_workspace(
    profile: crate::engine::sensor_contract::SensorProfile,
    bins: usize,
    workers: usize,
) -> (GraphExecutionPlan, ControlPhaseWorkspace) {
    let sensor = SensorEvaluator::new_v4(
        SensorConfig {
            bins,
            ..SensorConfig::default()
        },
        crate::engine::movement::MovementConfig::typescript_defaults(),
        DT,
        profile,
    )
    .unwrap();
    let plan = graph_plan_for_width(sensor.layout().input_size);
    let neural =
        NeuralControlPipeline::try_new(8, sensor, plan.clone(), workers, usize::MAX).unwrap();
    (plan, ControlPhaseWorkspace::new(neural).unwrap())
}
