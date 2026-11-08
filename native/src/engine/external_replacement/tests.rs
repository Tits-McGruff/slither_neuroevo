use super::*;
use crate::engine::graph::{
    GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
};
use crate::engine::rng::StatefulRng;
use crate::engine::state::{
    PelletState, ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START, EXTERNAL_ENTITY_ID_START,
    RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION,
};

const OLD_SNAKE_ID: u64 = EXTERNAL_ENTITY_ID_START + 1;
const OLD_BRAIN_ID: u64 = 20;
const OLD_LEASE_ID: u64 = 40;
const CONNECTION_ID: u64 = 77;
const WALL_NOW_MS: u64 = 1_000;

fn graph() -> GraphBundle {
    GraphBundle::compile(
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
            edges: vec![edge("input", "memory"), edge("memory", "head")],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        },
        &GraphLimits {
            max_nodes: 8,
            max_edges: 8,
            max_graph_outputs: 2,
            max_identifier_bytes: 32,
            max_total_referenced_identifier_bytes: 512,
            max_tensor_width: 16,
            max_mlp_hidden_layers: 2,
            max_split_output_ports: 2,
            max_parameter_floats: 1_000,
            max_recurrent_state_floats: 100,
            max_canonical_layout_bytes: 10_000,
            max_architecture_key_bytes: 10_000,
        },
    )
    .expect("replacement fixture graph must compile")
}

fn edge(from: &str, to: &str) -> GraphEdge {
    GraphEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        from_port: None,
        to_port: None,
    }
}

fn key() -> PhysicsStepKey {
    PhysicsStepKey::new(7, 3, 10, 4, 9, [0x5a; 32], 12)
}

fn rng() -> RngStateBundle {
    RngStateBundle {
        baseline_behaviors: Vec::new(),
        evolved_respawns: Vec::new(),
        version: RNG_BUNDLE_VERSION,
        world: StatefulRng::new(11.0).export_state(),
        evolution: StatefulRng::new(12.0).export_state(),
        external_controller: StatefulRng::new(13.0).export_state(),
        baselines: Vec::new(),
    }
}

fn allocators() -> AllocatorState {
    AllocatorState {
        version: ALLOCATOR_VERSION,
        next_entity_id: 100,
        next_brain_id: 500,
        next_genome_id: 600,
        next_controller_lease_id: 700,
        next_frame_v1_id: 1_000,
        next_external_id: EXTERNAL_ENTITY_ID_START + 100,
        next_baseline_id: BASELINE_ENTITY_ID_START + 100,
        next_resurrected_id: RESURRECTED_ENTITY_ID_START + 100,
    }
}

fn snake(
    id: u64,
    frame_v1_id: u32,
    kind: SnakeKind,
    alive: bool,
    position: WorldPoint,
    body: BodyRange,
    brain: Option<BrainHandle>,
) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id,
        kind,
        alive,
        population_slot: None,
        brain,
        baseline_slot: None,
        baseline_strategy: None,
        position,
        previous_position: position,
        direction: 0.25,
        radius: 9.0,
        speed: 165.0,
        boost: false,
        age_seconds: 3.0,
        food: 2.0,
        points: 4.0,
        kills: 1,
        target_length: body.len as f64,
        fitness: 5.0,
        turn: 0.75,
        previous_turn: 0.5,
        input_boost: true,
        previous_input_boost: false,
        control_accumulator_seconds: 0.0,
        delivered_observation_points: 1.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body,
        skin: 2,
    }
}

fn connected_lease(
    id: u64,
    snake_id: u64,
    connection_id: u64,
) -> super::super::state::ControllerLease {
    super::super::state::ControllerLease {
        reward: None,
        identity_key: String::new(),
        id,
        snake_id,
        kind: ControllerKind::Player,
        connection_id: Some(connection_id),
        scope: "run-a".to_owned(),
        resume_token: format!("old-token-{id}"),
        status: ControllerLeaseStatus::Connected,
        latest_action: LatestControllerAction {
            is_explicit: false,
            turn: 0.75,
            boost: true,
            client_tick: 9,
            arrival_sequence: 10,
            accepted_at_ms: 900,
        },
        last_observed_at_ms: 900,
        disconnected_at_ms: None,
        input_hold_expires_at_ms: None,
        grace_expires_at_ms: None,
        takeover_committed_at_ms: None,
    }
}

fn fixture(graph: &CompiledGraph) -> (WorldState, Vec<BrainRuntimeState>) {
    let obstacle_body = [
        WorldPoint { x: -20.0, y: 0.0 },
        WorldPoint { x: -27.5, y: 0.0 },
    ];
    let dead_body = [
        WorldPoint { x: 100.0, y: 100.0 },
        WorldPoint { x: 92.5, y: 100.0 },
    ];
    let world = WorldState {
        ray_cursor: Default::default(),
        snakes: vec![
            snake(
                1,
                1,
                SnakeKind::Evolved,
                true,
                obstacle_body[0],
                BodyRange { start: 0, len: 2 },
                None,
            ),
            snake(
                OLD_SNAKE_ID,
                10,
                SnakeKind::External,
                false,
                dead_body[0],
                BodyRange { start: 2, len: 2 },
                Some(BrainHandle {
                    id: OLD_BRAIN_ID,
                    epoch: key().population_epoch(),
                }),
            ),
        ],
        body_points: obstacle_body.into_iter().chain(dead_body).collect(),
        pellets: vec![PelletState {
            id: 8,
            position: WorldPoint { x: 10.0, y: 10.0 },
            value: 1.0,
            kind: 2,
            color: 3,
            owner: Some(OLD_SNAKE_ID),
        }],
        controller_leases: vec![connected_lease(OLD_LEASE_ID, OLD_SNAKE_ID, CONNECTION_ID)],
    };
    let brains = vec![BrainRuntimeState {
        handle: BrainHandle {
            id: OLD_BRAIN_ID,
            epoch: key().population_epoch(),
        },
        owner: BrainOwner::Entity(OLD_SNAKE_ID),
        retained_origin: None,
        non_population_weights: Some(vec![0.25; graph.total_parameters].into_boxed_slice()),
        recurrent: vec![0.5; graph.total_state_size].into_boxed_slice(),
    }];
    (world, brains)
}

fn generation_base(graph: &CompiledGraph) -> (WorldState, Vec<BrainRuntimeState>) {
    let epoch = key().population_epoch() + 1;
    let evolved_head = WorldPoint { x: -80.0, y: -40.0 };
    let baseline_head = WorldPoint { x: 80.0, y: 40.0 };
    let mut evolved = snake(
        1,
        20,
        SnakeKind::Evolved,
        true,
        evolved_head,
        BodyRange { start: 0, len: 2 },
        Some(BrainHandle { id: 21, epoch }),
    );
    evolved.population_slot = Some(0);
    let mut baseline = snake(
        BASELINE_ENTITY_ID_START + 1,
        21,
        SnakeKind::Baseline,
        true,
        baseline_head,
        BodyRange { start: 2, len: 2 },
        None,
    );
    baseline.baseline_slot = Some(0);
    baseline.baseline_strategy = Some(super::super::state::BaselineStrategyState::Roam);
    let world = WorldState {
        ray_cursor: Default::default(),
        snakes: vec![evolved, baseline],
        body_points: vec![
            evolved_head,
            WorldPoint { x: -87.5, y: -40.0 },
            baseline_head,
            WorldPoint { x: 72.5, y: 40.0 },
        ],
        pellets: vec![PelletState {
            id: 9,
            position: WorldPoint { x: 0.0, y: 180.0 },
            value: 1.0,
            kind: 1,
            color: 2,
            owner: None,
        }],
        controller_leases: Vec::new(),
    };
    let brains = vec![BrainRuntimeState {
        handle: BrainHandle { id: 21, epoch },
        owner: BrainOwner::PopulationSlot(0),
        retained_origin: None,
        non_population_weights: None,
        recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
    }];
    (world, brains)
}

#[test]
fn fresh_external_retry_preserves_source_and_isolates_rng_and_ids() {
    let graph = graph();
    let (world, brains) = generation_base(&graph);
    let rng = rng();
    let allocators = allocators();
    let before = (
        world.clone(),
        brains.clone(),
        rng.clone(),
        allocators.clone(),
    );
    let prepare = || {
        prepare_fresh_external(
            FreshExternalSource {
                world: &world,
                brains: &brains,
                rng: &rng,
                allocators: &allocators,
                graph: &graph,
                brain_epoch: key().population_epoch(),
            },
            ExternalReplacementConfig::typescript_defaults(),
        )
        .unwrap()
    };
    let first = prepare();
    let retry = prepare();
    assert_eq!(first.snake, retry.snake);
    assert_eq!(first.body, retry.body);
    assert_eq!(first.brain, retry.brain);
    assert_eq!(first.next_external_rng, retry.next_external_rng);
    assert_eq!(first.next_allocators, retry.next_allocators);
    assert_eq!(first.snake.id, allocators.next_external_id);
    assert_eq!(first.snake.frame_v1_id, allocators.next_frame_v1_id);
    assert_eq!(first.lease_id, allocators.next_controller_lease_id);
    assert_eq!(first.snake.body.start, world.body_points.len());
    assert_eq!(first.snake.body.len, first.body.len());
    assert!(first.brain.recurrent.iter().all(|value| *value == 0.0));
    assert_eq!(first.snake.delivered_observation_points, 0.0);
    let mut expected_allocators = allocators.clone();
    expected_allocators.next_external_id += 1;
    expected_allocators.next_frame_v1_id += 1;
    expected_allocators.next_brain_id += 1;
    expected_allocators.next_controller_lease_id += 1;
    assert_eq!(first.next_allocators, expected_allocators);
    assert_ne!(first.next_external_rng, rng.external_controller);
    assert_eq!((world, brains, rng, allocators), before);
}

#[test]
fn fresh_external_capacity_and_exhausted_ids_reject_before_publication() {
    let graph = graph();
    let (world, brains) = generation_base(&graph);
    let rng = rng();
    let mut allocators = allocators();
    let config = ExternalReplacementConfig::typescript_defaults();
    let prepare = |config, allocators: &AllocatorState| {
        prepare_fresh_external(
            FreshExternalSource {
                world: &world,
                brains: &brains,
                rng: &rng,
                allocators,
                graph: &graph,
                brain_epoch: key().population_epoch(),
            },
            config,
        )
    };
    assert!(matches!(
        prepare(
            ExternalReplacementConfig {
                maximum_snakes: world.snakes.len(),
                ..config
            },
            &allocators
        ),
        Err(ExternalReplacementError::SnakeCapacityExceeded { .. })
    ));
    assert!(matches!(
        prepare(
            ExternalReplacementConfig {
                maximum_brains: brains.len(),
                ..config
            },
            &allocators
        ),
        Err(ExternalReplacementError::BrainCapacityExceeded { .. })
    ));
    assert!(matches!(
        prepare(
            ExternalReplacementConfig {
                maximum_body_points: world.body_points.len(),
                ..config
            },
            &allocators
        ),
        Err(ExternalReplacementError::BodyCapacityExceeded { .. })
    ));
    allocators.next_frame_v1_id = super::super::state::FRAME_V1_EXHAUSTED_ID;
    let recycled = prepare(config, &allocators).expect("public IDs must recycle after wrapping");
    assert!(world
        .snakes
        .iter()
        .all(|snake| snake.frame_v1_id != recycled.snake.frame_v1_id));
    assert_eq!(
        recycled.snake.frame_v1_id as f32 as u32,
        recycled.snake.frame_v1_id
    );
    allocators.next_external_id = BASELINE_ENTITY_ID_START;
    let before = allocators.clone();
    assert!(matches!(
        prepare(config, &allocators),
        Err(ExternalReplacementError::Allocator(_))
    ));
    assert_eq!(allocators, before);
}

fn generation_controller_source() -> WorldState {
    let graph = graph();
    let (mut world, _) = fixture(&graph);
    let connected = world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == OLD_SNAKE_ID)
        .expect("connected external source snake");
    connected.alive = true;

    let disconnected_id = OLD_SNAKE_ID + 1;
    let disconnected_head = WorldPoint { x: 300.0, y: 300.0 };
    let body_start = world.body_points.len();
    world
        .body_points
        .extend([disconnected_head, WorldPoint { x: 292.5, y: 300.0 }]);
    world.snakes.push(snake(
        disconnected_id,
        11,
        SnakeKind::External,
        true,
        disconnected_head,
        BodyRange {
            start: body_start,
            len: 2,
        },
        Some(BrainHandle {
            id: OLD_BRAIN_ID + 1,
            epoch: key().population_epoch(),
        }),
    ));
    let mut disconnected = connected_lease(OLD_LEASE_ID + 1, disconnected_id, CONNECTION_ID + 1);
    disconnected.connection_id = None;
    disconnected.status = ControllerLeaseStatus::ReservedNeutral;
    disconnected.disconnected_at_ms = Some(800);
    disconnected.input_hold_expires_at_ms = Some(1_300);
    disconnected.grace_expires_at_ms = Some(30_800);
    world.controller_leases.push(disconnected);
    world
}

fn deterministic_entropy(seed: u8) -> impl FnMut(&mut [u8]) -> Result<(), ()> {
    let mut call = 0u8;
    move |bytes| {
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = seed
                .wrapping_add(call)
                .wrapping_add(u8::try_from(index).unwrap_or(0));
        }
        call = call.wrapping_add(1);
        Ok(())
    }
}

#[test]
fn generation_reassignment_preserves_the_base_and_replaces_only_connected_owners() {
    let graph = graph();
    let (base_world, base_brains) = generation_base(&graph);
    let controller_source = generation_controller_source();
    let base_world_before = base_world.clone();
    let base_brains_before = base_brains.clone();
    let controller_source_before = controller_source.clone();
    let base_rng = rng();
    let base_rng_before = base_rng.clone();
    let base_allocators = allocators();
    let base_allocators_before = base_allocators.clone();
    let successor_epoch = key().population_epoch() + 1;
    let mut workspace = ExternalReplacementWorkspace::new();
    let (assignment, new_token) = {
        let prepared = workspace
            .prepare_generation_reassignments_with_entropy(
                key(),
                &base_world,
                &base_rng,
                &base_allocators,
                &base_brains,
                &controller_source,
                &graph,
                successor_epoch,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                deterministic_entropy(31),
            )
            .expect("one connected generation owner must receive a replacement");
        assert_eq!(prepared.assignments().len(), 1);
        assert_eq!(prepared.statuses(), &[AssignmentDeliveryStatus::Pending]);
        assert_eq!(prepared.diagnostics().replacements, 1);
        assert_eq!(prepared.diagnostics().removed_dead_leases, 1);
        assert_eq!(prepared.unavailable_reservations().len(), 1);
        let unavailable = &prepared.unavailable_reservations()[0];
        assert_eq!(unavailable.source_lease_id, OLD_LEASE_ID + 1);
        assert_eq!(unavailable.source_snake_id, OLD_SNAKE_ID + 1);
        assert_eq!(
            unavailable.reason,
            UnavailableControllerReason::SnakeUnavailable
        );
        assert_eq!(
            unavailable.resume_token,
            format!("old-token-{}", OLD_LEASE_ID + 1)
        );
        assert_eq!(unavailable.grace_expires_at_ms, Some(30_800));
        (
            prepared.assignments()[0],
            prepared.resume_token(0).unwrap().to_owned(),
        )
    };

    assert_eq!(base_world, base_world_before);
    assert_eq!(base_brains, base_brains_before);
    assert_eq!(controller_source, controller_source_before);
    assert_eq!(base_rng, base_rng_before);
    assert_eq!(base_allocators, base_allocators_before);
    assert_eq!(assignment.source_lease_id, OLD_LEASE_ID);
    assert!(controller_source
        .controller_leases
        .iter()
        .any(|lease| lease.id == OLD_LEASE_ID + 1));
    assert!(!workspace
        .assignments(key())
        .unwrap()
        .iter()
        .any(|event| event.source_lease_id == OLD_LEASE_ID + 1));

    assert_eq!(
        workspace
            .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, true,)
            .unwrap(),
        AssignmentResolution::Accepted
    );
    let staged_world = workspace.world(key()).unwrap();
    assert_eq!(
        &staged_world.snakes[..base_world.snakes.len()],
        &base_world.snakes
    );
    assert_eq!(staged_world.pellets, base_world.pellets);
    assert_eq!(staged_world.controller_leases.len(), 1);
    let lease = &staged_world.controller_leases[0];
    assert_eq!(lease.id, assignment.lease_id);
    assert_eq!(lease.resume_token, new_token);
    assert_eq!(lease.connection_id, Some(CONNECTION_ID));
    assert_eq!(lease.status, ControllerLeaseStatus::Connected);
    let replacement = staged_world
        .snakes
        .iter()
        .find(|snake| snake.id == assignment.snake_id)
        .expect("connected owner replacement snake");
    assert!(replacement.alive);
    assert_eq!(replacement.kind, SnakeKind::External);

    let staged_brains = workspace.brains(key()).unwrap();
    assert_eq!(&staged_brains[..base_brains.len()], &base_brains);
    let external_brain = staged_brains
        .iter()
        .find(|brain| brain.owner == BrainOwner::Entity(assignment.snake_id))
        .expect("replacement external brain");
    assert_eq!(external_brain.handle.epoch, successor_epoch);
    let staged_rng = workspace.rng(key()).unwrap();
    assert_eq!(staged_rng.world, base_rng.world);
    assert_eq!(staged_rng.evolution, base_rng.evolution);
    assert_eq!(staged_rng.baselines, base_rng.baselines);
    assert_ne!(staged_rng.external_controller, base_rng.external_controller);
    let staged_allocators = workspace.allocators(key()).unwrap();
    assert_eq!(
        staged_allocators.next_entity_id,
        base_allocators.next_entity_id
    );
    assert_eq!(
        staged_allocators.next_genome_id,
        base_allocators.next_genome_id
    );
    assert_eq!(
        staged_allocators.next_baseline_id,
        base_allocators.next_baseline_id
    );
    assert_eq!(
        staged_allocators.next_external_id,
        base_allocators.next_external_id + 1
    );
    assert_eq!(
        staged_allocators.next_controller_lease_id,
        base_allocators.next_controller_lease_id + 1
    );
}

#[test]
fn generation_reassignment_without_connected_owners_consumes_no_rng_or_ids() {
    let graph = graph();
    let (base_world, base_brains) = generation_base(&graph);
    let mut controller_source = generation_controller_source();
    controller_source.controller_leases.remove(0);
    controller_source
        .snakes
        .retain(|snake| snake.id != OLD_SNAKE_ID);
    let base_rng = rng();
    let base_allocators = allocators();
    let mut workspace = ExternalReplacementWorkspace::new();
    {
        let prepared = workspace
            .prepare_generation_reassignments_with_entropy(
                key(),
                &base_world,
                &base_rng,
                &base_allocators,
                &base_brains,
                &controller_source,
                &graph,
                key().population_epoch() + 1,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                |_| panic!("no entropy is allowed without a connected owner"),
            )
            .expect("a disconnected reservation is omitted without replacement");
        assert!(prepared.assignments().is_empty());
        assert_eq!(prepared.diagnostics().removed_dead_leases, 1);
        assert_eq!(prepared.unavailable_reservations().len(), 1);
    }
    assert_eq!(workspace.world(key()).unwrap(), &base_world);
    assert_eq!(workspace.brains(key()).unwrap(), &base_brains);
    assert_eq!(workspace.rng(key()).unwrap(), &base_rng);
    assert_eq!(workspace.allocators(key()).unwrap(), &base_allocators);
}

#[test]
fn invalid_generation_controller_source_is_atomic() {
    let graph = graph();
    let (base_world, base_brains) = generation_base(&graph);
    let mut controller_source = generation_controller_source();
    controller_source.controller_leases[0].connection_id = None;
    let base_rng = rng();
    let base_allocators = allocators();
    let mut workspace = ExternalReplacementWorkspace::new();
    let error = workspace
        .prepare_generation_reassignments_with_entropy(
            key(),
            &base_world,
            &base_rng,
            &base_allocators,
            &base_brains,
            &controller_source,
            &graph,
            key().population_epoch() + 1,
            WALL_NOW_MS,
            ExternalReplacementConfig::typescript_defaults(),
            deterministic_entropy(41),
        )
        .expect_err("a connected lease without a live connection must fail");
    assert!(matches!(
        error,
        ExternalReplacementError::InvalidLeaseTarget {
            lease_id: OLD_LEASE_ID,
            snake_id: OLD_SNAKE_ID
        }
    ));
    assert!(!workspace.is_ready());
    assert_eq!(base_world, generation_base(&graph).0);
    assert_eq!(base_rng, rng());
    assert_eq!(base_allocators, allocators());
}

#[test]
fn connected_death_stages_fresh_external_only_state_and_waits_for_delivery() {
    let graph = graph();
    let (world, brains) = fixture(&graph);
    let source_world = world.clone();
    let source_rng = rng();
    let source_allocators = allocators();
    let config = ExternalReplacementConfig::typescript_defaults();
    let mut workspace = ExternalReplacementWorkspace::new();
    let (assignment, token) = {
        let prepared = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                config,
                deterministic_entropy(1),
            )
            .expect("connected death must stage one replacement");
        assert_eq!(prepared.assignments().len(), 1);
        assert_eq!(prepared.statuses(), &[AssignmentDeliveryStatus::Pending]);
        let assignment = prepared.assignments()[0];
        let token = prepared.resume_token(0).unwrap().to_owned();
        assert_eq!(token.len(), RESUME_TOKEN_LENGTH);
        assert_eq!(assignment.source_lease_id, OLD_LEASE_ID);
        assert_eq!(assignment.snake_id, source_allocators.next_external_id);
        assert_eq!(assignment.frame_v1_id, source_allocators.next_frame_v1_id);
        assert_eq!(
            assignment.lease_id,
            source_allocators.next_controller_lease_id
        );
        assert_eq!(world, source_world);
        assert_eq!(source_rng, rng());
        assert_eq!(source_allocators, allocators());
        (assignment, token)
    };

    assert!(matches!(
        workspace.world(key()),
        Err(ExternalReplacementError::AssignmentsPending)
    ));
    assert_eq!(
        workspace
            .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, true,)
            .unwrap(),
        AssignmentResolution::Accepted
    );

    let staged_world = workspace.world(key()).unwrap();
    let replacement = staged_world
        .snakes
        .iter()
        .find(|snake| snake.id == assignment.snake_id)
        .expect("replacement snake");
    assert!(replacement.alive);
    assert_eq!(replacement.kind, SnakeKind::External);
    assert_eq!(replacement.body.len, config.spawn.snake_start_len);
    assert_eq!(
        staged_world.body_points[replacement.body.start],
        replacement.position
    );
    assert!(staged_world
        .pellets
        .iter()
        .all(|pellet| pellet.owner.is_none()));
    let lease = staged_world
        .controller_leases
        .iter()
        .find(|lease| lease.id == assignment.lease_id)
        .expect("replacement lease");
    assert_eq!(lease.resume_token, token);
    assert_eq!(lease.connection_id, Some(CONNECTION_ID));
    assert_eq!(lease.status, ControllerLeaseStatus::Connected);
    assert_eq!(lease.latest_action.turn, 0.0);
    assert!(!lease.latest_action.boost);

    let brain = workspace
        .brains(key())
        .unwrap()
        .iter()
        .find(|brain| brain.owner == BrainOwner::Entity(assignment.snake_id))
        .expect("replacement brain");
    assert_eq!(brain.handle.id, source_allocators.next_brain_id);
    assert_eq!(
        brain.non_population_weights.as_ref().unwrap().len(),
        graph.total_parameters
    );
    assert!(brain
        .non_population_weights
        .as_ref()
        .unwrap()
        .iter()
        .any(|value| value.to_bits() != 0));
    assert_eq!(brain.recurrent.len(), graph.total_state_size);
    assert!(brain.recurrent.iter().all(|value| value.to_bits() == 0));

    let staged_rng = workspace.rng(key()).unwrap();
    assert_eq!(staged_rng.world, source_rng.world);
    assert_eq!(staged_rng.evolution, source_rng.evolution);
    assert_eq!(staged_rng.baselines, source_rng.baselines);
    assert_ne!(
        staged_rng.external_controller,
        source_rng.external_controller
    );
    let staged_allocators = workspace.allocators(key()).unwrap();
    assert_eq!(
        staged_allocators.next_external_id,
        source_allocators.next_external_id + 1
    );
    assert_eq!(
        staged_allocators.next_frame_v1_id,
        source_allocators.next_frame_v1_id + 1
    );
    assert_eq!(
        staged_allocators.next_brain_id,
        source_allocators.next_brain_id + 1
    );
    assert_eq!(
        staged_allocators.next_controller_lease_id,
        source_allocators.next_controller_lease_id + 1
    );
    assert_eq!(
        staged_allocators.next_entity_id,
        source_allocators.next_entity_id
    );
    assert_eq!(
        staged_allocators.next_genome_id,
        source_allocators.next_genome_id
    );
}

#[test]
fn failed_assignment_invalidates_the_old_snake_token_and_enters_exact_grace() {
    let graph = graph();
    let (world, brains) = fixture(&graph);
    let source_rng = rng();
    let source_allocators = allocators();
    let mut workspace = ExternalReplacementWorkspace::new();
    let (assignment, new_token) = {
        let prepared = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                deterministic_entropy(2),
            )
            .unwrap();
        (
            prepared.assignments()[0],
            prepared.resume_token(0).unwrap().to_owned(),
        )
    };
    assert_eq!(
        workspace
            .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, false,)
            .unwrap(),
        AssignmentResolution::Failed
    );
    assert_eq!(
        workspace
            .resolve_assignment(key(), assignment.lease_id, assignment.connection_id, true,)
            .unwrap(),
        AssignmentResolution::Ignored
    );
    let staged = workspace.world(key()).unwrap();
    let lease = staged
        .controller_leases
        .iter()
        .find(|lease| lease.id == assignment.lease_id)
        .unwrap();
    let snake = staged
        .snakes
        .iter()
        .find(|snake| snake.id == assignment.snake_id)
        .unwrap();
    assert_ne!(lease.resume_token, format!("old-token-{OLD_LEASE_ID}"));
    assert_eq!(lease.resume_token, new_token);
    assert_eq!(lease.connection_id, None);
    assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
    assert_eq!(lease.disconnected_at_ms, Some(WALL_NOW_MS));
    assert_eq!(lease.input_hold_expires_at_ms, Some(WALL_NOW_MS + 500));
    assert_eq!(lease.grace_expires_at_ms, Some(WALL_NOW_MS + 30_000));
    assert_eq!(snake.turn, 0.0);
    assert!(!snake.input_boost);
    assert!(snake.alive);
}

#[test]
fn opaque_authority_proof_rejects_any_post_validation_buffer_substitution() {
    let graph = graph();
    let (world, brains) = fixture(&graph);
    let source_rng = rng();
    let source_allocators = allocators();
    let mut workspace = ExternalReplacementWorkspace::new();
    {
        let prepared = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                deterministic_entropy(9),
            )
            .expect("valid replacement must issue one opaque authority proof");
        assert_eq!(prepared.assignments().len(), 1);
    }

    let buffers = workspace.validation_buffers(key()).unwrap();
    assert!(buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains,
    ));

    let brain_index = buffers
        .brains
        .iter()
        .position(|brain| brain.owner == BrainOwner::Entity(allocators().next_external_id))
        .unwrap();
    let original_weight = buffers.brains[brain_index]
        .non_population_weights
        .as_ref()
        .unwrap()[0];
    buffers.brains[brain_index]
        .non_population_weights
        .as_mut()
        .unwrap()[0] = f32::from_bits(original_weight.to_bits() ^ 1);
    assert!(!buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains,
    ));
    buffers.brains[brain_index]
        .non_population_weights
        .as_mut()
        .unwrap()[0] = original_weight;

    buffers.brains[brain_index].retained_origin = Some(std::sync::Arc::new(
        super::super::retained_origin::RetainedNeuralOrigin::HallOfFame {
            entry_id: 1,
            weight_digest: super::super::retained_origin::packed_weight_digest(
                buffers.brains[brain_index]
                    .non_population_weights
                    .as_ref()
                    .unwrap(),
            ),
        },
    ));
    assert!(!buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains
    ));
    buffers.brains[brain_index].retained_origin = None;
    assert!(buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains
    ));

    let original_rng_state = buffers.rng.external_controller.state_hex.clone();
    buffers.rng.external_controller.state_hex = "12345678".into();
    assert!(!buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains,
    ));
    buffers.rng.external_controller.state_hex = original_rng_state;

    let original_external_id = buffers.allocators.next_external_id;
    buffers.allocators.next_external_id += 1;
    assert!(!buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains,
    ));
    buffers.allocators.next_external_id = original_external_id;

    let removed_lease = buffers.world.controller_leases.pop().unwrap();
    assert!(!buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains,
    ));
    buffers.world.controller_leases.push(removed_lease);
    assert!(buffers.proof.matches(
        key(),
        buffers.world,
        buffers.rng,
        buffers.allocators,
        buffers.brains,
    ));
}

#[test]
fn disconnected_dead_lease_is_removed_without_rng_or_allocator_use() {
    let graph = graph();
    let (mut world, brains) = fixture(&graph);
    let lease = &mut world.controller_leases[0];
    lease.status = ControllerLeaseStatus::ReservedNeutral;
    lease.connection_id = None;
    lease.disconnected_at_ms = Some(900);
    lease.input_hold_expires_at_ms = Some(1_400);
    lease.grace_expires_at_ms = Some(30_900);
    lease.last_observed_at_ms = 900;
    let source_rng = rng();
    let source_allocators = allocators();
    let mut workspace = ExternalReplacementWorkspace::new();
    {
        let prepared = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                |_| panic!("no token entropy is allowed without a live connection"),
            )
            .unwrap();
        assert!(prepared.assignments().is_empty());
        assert_eq!(prepared.diagnostics().removed_dead_leases, 1);
        assert_eq!(prepared.unavailable_reservations().len(), 1);
        assert_eq!(
            prepared.unavailable_reservations()[0].resume_token,
            format!("old-token-{OLD_LEASE_ID}")
        );
    }
    assert!(workspace.world(key()).unwrap().controller_leases.is_empty());
    assert_eq!(workspace.rng(key()).unwrap(), &source_rng);
    assert_eq!(workspace.allocators(key()).unwrap(), &source_allocators);
}

#[test]
fn entropy_and_aggregate_work_failures_expose_no_result_and_retry_cleanly() {
    let graph = graph();
    let (world, brains) = fixture(&graph);
    let source_rng = rng();
    let source_allocators = allocators();
    let config = ExternalReplacementConfig::typescript_defaults();
    let mut workspace = ExternalReplacementWorkspace::new();
    let error = workspace
        .prepare_with_entropy(
            key(),
            &world,
            &source_rng,
            &source_allocators,
            &brains,
            &graph,
            WALL_NOW_MS,
            config,
            |_| Err(()),
        )
        .expect_err("OS entropy failure must reject staging");
    assert!(matches!(
        error,
        ExternalReplacementError::EntropyUnavailable
    ));
    assert!(!workspace.is_ready());
    assert!(matches!(
        workspace.world(key()),
        Err(ExternalReplacementError::ResultNotReady)
    ));

    {
        let retry = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                config,
                deterministic_entropy(9),
            )
            .expect("a clean retry must succeed");
        assert_eq!(retry.assignments().len(), 1);
    }

    let mut two_world = world.clone();
    let second_id = OLD_SNAKE_ID + 1;
    let second_start = two_world.body_points.len();
    let second_head = WorldPoint { x: 300.0, y: 300.0 };
    two_world
        .body_points
        .extend([second_head, WorldPoint { x: 292.5, y: 300.0 }]);
    two_world.snakes.push(snake(
        second_id,
        11,
        SnakeKind::External,
        false,
        second_head,
        BodyRange {
            start: second_start,
            len: 2,
        },
        None,
    ));
    two_world.controller_leases.push(connected_lease(
        OLD_LEASE_ID + 1,
        second_id,
        CONNECTION_ID + 1,
    ));
    let mut tight = config;
    tight.spawn.maximum_candidates_per_batch = 1;
    let error = workspace
        .prepare_with_entropy(
            key(),
            &two_world,
            &source_rng,
            &source_allocators,
            &brains,
            &graph,
            WALL_NOW_MS,
            tight,
            deterministic_entropy(11),
        )
        .expect_err("the second replacement must not exceed the shared work budget");
    assert!(matches!(
        error,
        ExternalReplacementError::Spawn(inner)
            if matches!(*inner, SpawnError::WorkBudgetExceeded { .. })
    ));
    assert!(!workspace.is_ready());
    assert_eq!(world, fixture(&graph).0);
    assert_eq!(source_rng, rng());
    assert_eq!(source_allocators, allocators());
}

#[test]
fn stale_or_duplicate_assignment_results_cannot_resolve_the_batch() {
    let graph = graph();
    let (world, brains) = fixture(&graph);
    let source_rng = rng();
    let source_allocators = allocators();
    let mut workspace = ExternalReplacementWorkspace::new();
    let assignment = {
        let prepared = workspace
            .prepare_with_entropy(
                key(),
                &world,
                &source_rng,
                &source_allocators,
                &brains,
                &graph,
                WALL_NOW_MS,
                ExternalReplacementConfig::typescript_defaults(),
                deterministic_entropy(13),
            )
            .unwrap();
        prepared.assignments()[0]
    };
    assert_eq!(
        workspace
            .resolve_assignment(
                key(),
                assignment.lease_id,
                assignment.connection_id + 1,
                true,
            )
            .unwrap(),
        AssignmentResolution::Ignored
    );
    assert_eq!(
        workspace.statuses(key()).unwrap(),
        &[AssignmentDeliveryStatus::Pending]
    );
    assert!(matches!(
        workspace.world(key()),
        Err(ExternalReplacementError::AssignmentsPending)
    ));
    assert!(matches!(
        workspace.resolve_assignment(
            PhysicsStepKey::new(8, 3, 10, 4, 9, [0x5a; 32], 12),
            assignment.lease_id,
            assignment.connection_id,
            true,
        ),
        Err(ExternalReplacementError::ResultNotReady)
    ));
}
