//! Exact ordered-input continuation through the production archive and control paths.
//! SQLite commits and WebSocket replacement/rejoin remain separate integration gates.

use super::execute_running_authority_command;
use crate::engine::checkpoint::{CheckpointOperationId, NumericEncoding};
use crate::engine::contract::{
    ControllerActionRequest, ControllerJoinRequest, ControllerReclaimReceipt,
    ExternalDeliveryReceipt, RunningAuthorityCommand, RunningAuthorityEvent,
};
use crate::engine::controller_output::own_pending_messages;
use crate::engine::export_archive::{
    compose_export_archive, prepare_import_archive, ExportInventoryDescriptor,
};
use crate::engine::fresh_run::{
    prepare_stage6a_p0_fresh_run_with_settings_and_graph, stage6a_p0_archive_validation_contract,
    FreshRunSettingUpdate, Stage6aP0FreshRunRequest,
};
use crate::engine::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec};
use crate::engine::running_loop::{
    RunningAuthorityLoop, RunningAuthorityLoopProgress, RunningGenerationCheckpointPublication,
};
use crate::engine::running_step::ExternalDeliveryEventKind;
use crate::engine::scheduler::{FixedStepSchedulerPolicy, SchedulerServiceMode};
use crate::engine::state::ControllerKind;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MEMORY_CEILING: usize = 4 * 1024 * 1024 * 1024;

/// One private fixture whose source archive remains unchanged throughout every replay.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "slither-archive-input-continuation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn directory(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let root = fs::canonicalize(&self.0).unwrap();
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(root.starts_with(&temp) && root != temp);
        fs::remove_dir_all(root).unwrap();
    }
}

fn operation(value: u128) -> CheckpointOperationId {
    CheckpointOperationId::parse(format!("{value:032x}")).unwrap()
}

/// Mix every recurrent kernel with a differently weighted population and durable bots.
fn graph() -> GraphSpec {
    let kinds = [
        ("input", GraphNodeKind::Input { output_size: 83 }),
        (
            "dense",
            GraphNodeKind::Dense {
                input_size: 83,
                output_size: 8,
            },
        ),
        (
            "gru",
            GraphNodeKind::Gru {
                input_size: 8,
                hidden_size: 4,
            },
        ),
        (
            "lstm",
            GraphNodeKind::Lstm {
                input_size: 4,
                hidden_size: 4,
            },
        ),
        (
            "rru",
            GraphNodeKind::Rru {
                input_size: 4,
                hidden_size: 4,
            },
        ),
        (
            "head",
            GraphNodeKind::Dense {
                input_size: 4,
                output_size: 2,
            },
        ),
    ];
    GraphSpec {
        edges: kinds
            .windows(2)
            .map(|pair| GraphEdge {
                from: pair[0].0.to_owned(),
                to: pair[1].0.to_owned(),
                from_port: None,
                to_port: None,
            })
            .collect(),
        nodes: kinds
            .into_iter()
            .map(|(id, kind)| GraphNodeSpec {
                id: id.to_owned(),
                kind,
            })
            .collect(),
        outputs: vec![GraphOutputRef {
            node_id: "head".to_owned(),
            port: None,
        }],
        output_size: 2,
    }
}

/// Capture gameplay identities and bits; only OS-entropy tokens and process epochs are excluded.
#[derive(Debug, PartialEq, Eq)]
struct Observation {
    step: u64,
    sequence: u64,
    connection: u64,
    lease: u64,
    frame: u32,
    snake: u64,
    kind: ControllerKind,
    delivery: ExternalDeliveryEventKind,
    pose: [u64; 3],
    sensors: Vec<u32>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Trace {
    actions: Vec<(u64, u64, u64, u32, bool)>,
    observations: Vec<Observation>,
}

/// Execute the same validated production command boundary with a bounded reply allowance.
fn command(
    running: &mut RunningAuthorityLoop,
    sequence: &mut u64,
    wall: u64,
    value: RunningAuthorityCommand,
) -> RunningAuthorityEvent {
    *sequence += 1;
    execute_running_authority_command(*sequence, value, running, wall, 1024 * 1024).unwrap()
}

fn join(running: &mut RunningAuthorityLoop, sequence: &mut u64, wall: u64, origin: Instant) {
    for (connection, kind) in [
        (71, ControllerKind::Player),
        (72, ControllerKind::ReinforcementLearning),
    ] {
        let event = command(
            running,
            sequence,
            wall,
            RunningAuthorityCommand::JoinController(Box::new(ControllerJoinRequest {
                connection_id: connection,
                kind,
                identity_key: format!("ordered-{connection}"),
                received_at: origin + Duration::from_millis(wall),
            })),
        );
        let RunningAuthorityEvent::ControllerJoinAssignment {
            request_sequence,
            lease_id,
            ..
        } = event
        else {
            panic!("fresh join rejected: {event:?}")
        };
        let reply = command(
            running,
            sequence,
            wall,
            RunningAuthorityCommand::SubmitControllerJoinReceipt(ControllerReclaimReceipt {
                request_sequence,
                connection_id: connection,
                lease_id,
                accepted: true,
            }),
        );
        assert!(matches!(
            reply,
            RunningAuthorityEvent::ControllerJoinResolved {
                matched: true,
                accepted: true,
                ..
            }
        ));
    }
}

/// Advance one real generation, accepting reliable observations before committing each step.
fn advance(
    running: &mut RunningAuthorityLoop,
    wall: &mut u64,
    origin: Instant,
    controlled: bool,
    invert_player: bool,
) -> Trace {
    running.set_background_clock(origin);
    let start = running.completed_step();
    let mut sequence = 0;
    let mut observed = BTreeMap::new();
    let mut trace = Trace::default();
    if controlled {
        join(running, &mut sequence, *wall, origin);
    }
    for _ in 0..1200 {
        if controlled && (running.completed_step() - start).is_multiple_of(12) {
            for lease in running.generation_source_controller_leases().to_vec() {
                let Some(connection) = lease.connection_id else {
                    continue;
                };
                let Some(&tick) = observed.get(&lease.id) else {
                    continue;
                };
                let phase = (running.completed_step() - start) / 12 + connection;
                let mut turn = [-0.75, 0.25, 0.9, -0.1][phase as usize % 4];
                if invert_player && lease.kind == ControllerKind::Player {
                    turn = -turn;
                }
                let boost = phase % 4 == 1;
                let applied = command(
                    running,
                    &mut sequence,
                    *wall,
                    RunningAuthorityCommand::SubmitControllerAction(ControllerActionRequest {
                        lease_id: lease.id,
                        connection_id: connection,
                        turn,
                        boost,
                        client_tick: tick,
                        received_at: origin + Duration::from_millis(*wall),
                    }),
                );
                let RunningAuthorityEvent::ControllerActionApplied { completed_step, .. } = applied
                else {
                    panic!("ordered action rejected: {applied:?}")
                };
                trace
                    .actions
                    .push((completed_step, connection, tick, turn.to_bits(), boost));
            }
        }
        *wall += 17;
        loop {
            match running
                .service_after_command_drain(*wall, SchedulerServiceMode::Background, None)
                .unwrap()
            {
                RunningAuthorityLoopProgress::GenerationTransitionPending { .. } => return trace,
                RunningAuthorityLoopProgress::ExternalDeliveryPending { .. } => {
                    let messages = own_pending_messages(running).unwrap();
                    let mut receipts = Vec::new();
                    for message in messages.iter() {
                        let event = message.event;
                        let step = event.step_key.source_completed_step();
                        if event.delivery_kind == ExternalDeliveryEventKind::Observation {
                            observed.insert(event.lease_id, step);
                        } else {
                            observed.remove(&event.lease_id);
                        }
                        trace.observations.push(Observation {
                            step,
                            sequence: event.event_sequence,
                            connection: event.connection_id,
                            lease: event.lease_id,
                            frame: message.frame_v1_id,
                            snake: event.snake_id,
                            kind: event.controller_kind,
                            delivery: event.delivery_kind,
                            pose: [
                                event.position.x.to_bits(),
                                event.position.y.to_bits(),
                                event.direction.to_bits(),
                            ],
                            sensors: message
                                .sensors
                                .iter()
                                .map(|value| value.to_bits())
                                .collect(),
                        });
                        receipts.push(ExternalDeliveryReceipt {
                            operation_epoch: event.step_key.operation_epoch(),
                            event_sequence: event.event_sequence,
                            connection_id: event.connection_id,
                            lease_id: event.lease_id,
                            accepted: true,
                        });
                    }
                    let accepted = command(
                        running,
                        &mut sequence,
                        *wall,
                        RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                            receipts: receipts.into_boxed_slice(),
                        },
                    );
                    assert!(
                        matches!(accepted,
                            RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
                                matched_acceptances, matched_failures: 0, ignored_receipts: 0,
                                remaining: 0, published_completed_step: Some(_), ..
                            } if matched_acceptances == messages.len()
                        ),
                        "delivery rejected: {accepted:?}"
                    );
                }
                RunningAuthorityLoopProgress::Idle { .. }
                | RunningAuthorityLoopProgress::Published { .. } => break,
                other => panic!("unexpected replay blocker: {other:?}"),
            }
        }
    }
    panic!("generation failed to reach its bounded durable transition");
}

/// Encode the actual Rust generation record in the fixed SQLite-to-export inventory layout.
fn inventory(
    directory: &Path,
    publication: &RunningGenerationCheckpointPublication,
) -> ExportInventoryDescriptor {
    let mut bytes = vec![0u8; 208];
    bytes[..13].copy_from_slice(b"SLITHER-EXPV1");
    bytes[16..24].copy_from_slice(&1u64.to_le_bytes());
    bytes[24..32].copy_from_slice(&1u64.to_le_bytes());
    let summary = publication.commit_record.summary;
    let history = &mut bytes[32..88];
    for (offset, value) in [
        (0, summary.completed_generation),
        (8, summary.best_f64_bits),
        (16, summary.average_f64_bits),
        (24, summary.minimum_f64_bits),
        (40, summary.average_weight_f64_bits),
        (48, summary.weight_variance_f64_bits),
    ] {
        history[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    history[32..36].copy_from_slice(&u32::try_from(summary.species_count).unwrap().to_le_bytes());
    history[36..40].copy_from_slice(
        &u32::try_from(summary.top_species_size)
            .unwrap()
            .to_le_bytes(),
    );
    let hall = publication.commit_record.hall_of_fame;
    let record = &mut bytes[88..];
    for (offset, value) in [
        (0, hall.completed_generation),
        (16, hall.source_snake_id),
        (24, hall.successor_genome_id),
        (32, hall.fitness_f64_bits),
        (40, hall.points_f64_bits),
        (48, hall.length),
    ] {
        record[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    record[8..12].copy_from_slice(
        &u32::try_from(hall.source_population_slot)
            .unwrap()
            .to_le_bytes(),
    );
    record[12..16].copy_from_slice(
        &u32::try_from(hall.successor_population_slot)
            .unwrap()
            .to_le_bytes(),
    );
    let elite = &publication.hall_of_fame_weights;
    for (index, octet) in record[56..88].iter_mut().enumerate() {
        *octet = u8::from_str_radix(&elite.logical_sha256[index * 2..index * 2 + 2], 16).unwrap();
    }
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
            .copy_from_slice(&u64::from_str_radix(value, 16).unwrap().to_le_bytes());
    }
    let descriptor = ExportInventoryDescriptor {
        version: 1,
        relative_filename: format!(".{}.export-inventory-v1", operation(3).as_str()),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        stored_byte_count_hex: format!("{:016x}", bytes.len()),
        history_count_hex: format!("{:016x}", 1),
        hall_of_fame_count_hex: format!("{:016x}", 1),
    };
    fs::write(directory.join(&descriptor.relative_filename), bytes).unwrap();
    descriptor
}

#[test]
fn archive_import_replays_ordered_player_and_trainer_input_across_worker_counts() {
    let fixture = Fixture::new();
    let source = fixture.directory("source");
    let updates = [
        ("snakeCount", 12.0),
        ("generationSeconds", 8.0),
        ("pelletCountTarget", 100.0),
        ("baselineBots.count", 2.0),
        ("baselineBots.respawnDelay", 1.0),
    ]
    .into_iter()
    .map(|(path, value)| FreshRunSettingUpdate {
        path: path.to_owned(),
        value,
    })
    .collect::<Vec<_>>();
    let mut pending = prepare_stage6a_p0_fresh_run_with_settings_and_graph(
        Stage6aP0FreshRunRequest {
            run_id: "ordered-input-round-trip".to_owned(),
            seed: 42,
            memory_ceiling_bytes: MEMORY_CEILING,
        },
        &updates,
        graph(),
    )
    .unwrap();
    pending.configure_calculation_workers(1).unwrap();
    let initial = pending.publish_checkpoint(&source, operation(1)).unwrap();
    pending.acknowledge_persistence(&initial).unwrap();
    pending.publish_running_authority().unwrap();
    let mut direct = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .unwrap();
    let origin = Instant::now();
    let mut wall = 0;
    advance(&mut direct, &mut wall, origin, false, false);
    let boundary = direct
        .publish_pending_generation_checkpoint(&source, operation(2))
        .unwrap();
    assert_eq!(boundary.descriptor.generation_hex, "0000000000000002");
    assert_ne!(
        boundary.descriptor.recurrent_state_count_hex,
        "0000000000000000"
    );
    let (limits, graphs, policy) =
        stage6a_p0_archive_validation_contract(MEMORY_CEILING, true).unwrap();
    let inventory = inventory(&source, &boundary);
    let archive = compose_export_archive(
        &source,
        operation(3).as_str(),
        &boundary.descriptor,
        &inventory,
        &limits,
        &graphs,
        &policy,
    )
    .unwrap();
    let archive_path = source.join(archive.relative_filename);
    let archive_bytes = fs::read(&archive_path).unwrap();
    direct
        .acknowledge_pending_generation_persistence(&boundary.descriptor)
        .unwrap();
    direct
        .prepare_acknowledged_generation_reassignments()
        .unwrap();
    direct
        .publish_acknowledged_generation_start(wall, None)
        .unwrap();
    let boundary_wall = wall;
    let expected = advance(&mut direct, &mut wall, origin, true, false);
    for connection in [71, 72] {
        assert!(
            expected
                .actions
                .iter()
                .filter(|action| action.1 == connection)
                .count()
                > 5
        );
        assert!(
            expected
                .observations
                .iter()
                .filter(|sample| sample.connection == connection && !sample.sensors.is_empty())
                .count()
                > 20
        );
    }
    let successor = direct
        .publish_pending_generation_checkpoint(&source, operation(4))
        .unwrap();
    for (workers, invert) in [(1, false), (4, false), (5, false), (6, false), (1, true)] {
        let imported = fixture.directory(&format!("import-{workers}-{invert}"));
        let mut restored = prepare_import_archive(
            &archive_path,
            &imported,
            &imported,
            operation(10 + workers as u128 + u128::from(invert)).as_str(),
            "unused-legacy-run",
            1,
            &limits,
            &graphs,
            &policy,
            MEMORY_CEILING,
        )
        .unwrap();
        restored
            .transition
            .configure_calculation_workers(workers)
            .unwrap();
        restored
            .transition
            .acknowledge_import_persistence(&restored.descriptor)
            .unwrap();
        restored.transition.publish_running_authority().unwrap();
        let mut running = restored
            .transition
            .into_running_loop(
                FixedStepSchedulerPolicy::provisional_defaults(),
                boundary_wall,
            )
            .unwrap();
        let mut replay_wall = boundary_wall;
        let actual = advance(&mut running, &mut replay_wall, origin, true, invert);
        if invert {
            assert_ne!(
                actual.observations, expected.observations,
                "changed steering must affect gameplay"
            );
        } else {
            assert_eq!(actual.actions, expected.actions, "worker count {workers}");
            assert_eq!(actual.observations.len(), expected.observations.len());
            for (index, (sample, expected_sample)) in actual
                .observations
                .iter()
                .zip(&expected.observations)
                .enumerate()
            {
                assert_eq!(
                    sample, expected_sample,
                    "worker {workers}, observation {index}"
                );
            }
            // Checkpoints preserve completed steps, not the running process's wall debt.
            let replay = running
                .publish_pending_generation_checkpoint(&imported, operation(30 + workers as u128))
                .unwrap();
            assert_eq!(replay.commit_record, successor.commit_record);
            assert_eq!(
                replay.descriptor.logical_root_sha256,
                successor.descriptor.logical_root_sha256
            );
            assert_eq!(
                fs::read(imported.join(replay.descriptor.relative_filename)).unwrap(),
                fs::read(source.join(&successor.descriptor.relative_filename)).unwrap()
            );
            assert_eq!(
                fs::read(imported.join(replay.hall_of_fame_weights.relative_filename)).unwrap(),
                fs::read(source.join(&successor.hall_of_fame_weights.relative_filename)).unwrap()
            );
        }
        assert_eq!(fs::read(&archive_path).unwrap(), archive_bytes);
    }
}
