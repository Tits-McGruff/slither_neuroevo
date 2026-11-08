//! Event wire for the coarse N-API boundary.

use super::*;

/// Convert the Rust-owned compact generation record without numeric narrowing.
pub(crate) fn generation_commit_to_napi(
    record: GenerationCommitRecord,
) -> Stage6GenerationCommitRecord {
    Stage6GenerationCommitRecord {
        experiment: record.experiment.map(|record| {
            let bytes = record.encode();
            let hex = |source: &[u8]| -> String {
                source.iter().map(|byte| format!("{byte:02x}")).collect()
            };
            Stage6GenerationExperimentRecord {
                version: 1,
                history_hex: hex(&bytes[..512]),
                hall_of_fame_hex: hex(&bytes[512..]),
            }
        }),
        completion: record
            .completion
            .map(|completion| Stage6GenerationCompletionRecord {
                version: 1,
                reason: generation_transition_reason_name(completion.reason).to_owned(),
                actual_elapsed_f64_hex: u64_hex(completion.actual_elapsed_f64_bits),
                configured_duration_f64_hex: u64_hex(completion.configured_duration_f64_bits),
                completed_step: u64_hex(completion.completed_step),
            }),
        summary: Stage6GenerationSummaryRecord {
            completed_generation: u64_hex(record.summary.completed_generation),
            best_f64_hex: u64_hex(record.summary.best_f64_bits),
            average_f64_hex: u64_hex(record.summary.average_f64_bits),
            minimum_f64_hex: u64_hex(record.summary.minimum_f64_bits),
            species_count: u64_hex(record.summary.species_count),
            top_species_size: u64_hex(record.summary.top_species_size),
            average_weight_f64_hex: u64_hex(record.summary.average_weight_f64_bits),
            weight_variance_f64_hex: u64_hex(record.summary.weight_variance_f64_bits),
        },
        hall_of_fame: Stage6HallOfFameRecord {
            completed_generation: u64_hex(record.hall_of_fame.completed_generation),
            source_population_slot: u64_hex(record.hall_of_fame.source_population_slot),
            source_snake_id: u64_hex(record.hall_of_fame.source_snake_id),
            fitness_f64_hex: u64_hex(record.hall_of_fame.fitness_f64_bits),
            points_f64_hex: u64_hex(record.hall_of_fame.points_f64_bits),
            length: u64_hex(record.hall_of_fame.length),
            successor_population_slot: u64_hex(record.hall_of_fame.successor_population_slot),
            successor_genome_id: u64_hex(record.hall_of_fame.successor_genome_id),
        },
    }
}

pub(crate) fn background_generation_event_to_napi(
    event: CompletedEvent,
) -> std::result::Result<Stage6BackgroundGenerationEvent, EngineError> {
    let mut output = empty_background_generation_event();
    match event {
        CompletedEvent::Reliable(ReliableEvent::Started) => {
            output.kind = "started".to_owned();
        }
        CompletedEvent::Fault(fault) => {
            output.kind = "fault".to_owned();
            output.fault_code = Some(error_code_name(fault.code()).to_owned());
            output.fault_detail = Some(fault.detail().to_owned());
        }
        CompletedEvent::Reliable(ReliableEvent::Stopped) => {
            output.kind = "stopped".to_owned();
        }
        CompletedEvent::Reliable(ReliableEvent::RunningAuthority(event)) => {
            return running_authority_event_to_napi(*event);
        }
        CompletedEvent::RunningDisplay(status) => {
            output.kind = "display".to_owned();
            output.display = Some(crate::napi_running_engine::display_status_to_napi(status));
        }
        CompletedEvent::Reliable(ReliableEvent::ProbeResult { .. })
        | CompletedEvent::Discrete(_)
        | CompletedEvent::Stats(_)
        | CompletedEvent::Frame(_) => {
            return Err(EngineError::new(
                EngineErrorCode::Faulted,
                "background generation fixture received an unrelated output event",
            ));
        }
    }
    Ok(output)
}

pub(crate) fn running_authority_event_to_napi(
    event: RunningAuthorityEvent,
) -> std::result::Result<Stage6BackgroundGenerationEvent, EngineError> {
    let mut output = empty_background_generation_event();
    match event {
        RunningAuthorityEvent::ShadowChanged {
            command_sequence,
            shadow_slot,
            artifact_sha256,
            enabled,
            effective_step,
        } => {
            output.kind = "shadowChanged".into();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.shadow_change = Some(BackgroundShadowChange {
                shadow_slot,
                artifact_sha256: artifact_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                enabled,
                effective_step: u64_hex(effective_step),
            });
        }
        RunningAuthorityEvent::TeacherCaptureChanged {
            command_sequence,
            capture_epoch,
            enabled,
        } => {
            output.kind = "teacherCaptureChanged".into();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.teacher_capture_epoch = Some(
                capture_epoch
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            );
            output.teacher_capture_enabled = Some(enabled);
        }
        RunningAuthorityEvent::ControllerJoinAssignment {
            request_sequence,
            controller_kind,
            connection_id,
            lease_id,
            frame_v1_id,
            completed_step,
            resume_token,
        } => {
            output.kind = "controllerJoinAssignment".into();
            output.command_sequence = Some(u64_hex(request_sequence));
            output.controller_join_assignment = Some(BackgroundControllerReclaimAssignment {
                request_sequence: u64_hex(request_sequence),
                controller_kind: controller_kind_name(controller_kind).into(),
                connection_id: u64_hex(connection_id),
                lease_id: u64_hex(lease_id),
                snake_id: frame_v1_id,
                completed_step: u64_hex(completed_step),
                resume_token: resume_token.into_string(),
            });
        }
        RunningAuthorityEvent::ControllerJoinResolved {
            command_sequence,
            request_sequence,
            matched,
            accepted,
        } => {
            output.kind = "controllerJoinResolved".into();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.controller_join_resolution = Some(BackgroundControllerReclaimResolution {
                request_sequence: u64_hex(request_sequence),
                matched,
                accepted,
            });
        }
        RunningAuthorityEvent::ControllerReclaimAssignment {
            request_sequence,
            controller_kind,
            connection_id,
            lease_id,
            frame_v1_id,
            completed_step,
            resume_token,
        } => {
            output.kind = "controllerReclaimAssignment".into();
            output.command_sequence = Some(u64_hex(request_sequence));
            output.controller_reclaim_assignment = Some(BackgroundControllerReclaimAssignment {
                request_sequence: u64_hex(request_sequence),
                controller_kind: controller_kind_name(controller_kind).into(),
                connection_id: u64_hex(connection_id),
                lease_id: u64_hex(lease_id),
                snake_id: frame_v1_id,
                completed_step: u64_hex(completed_step),
                resume_token: resume_token.into_string(),
            });
        }
        RunningAuthorityEvent::ControllerReclaimResolved {
            command_sequence,
            request_sequence,
            matched,
            accepted,
        } => {
            output.kind = "controllerReclaimResolved".into();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.controller_reclaim_resolution = Some(BackgroundControllerReclaimResolution {
                request_sequence: u64_hex(request_sequence),
                matched,
                accepted,
            });
        }
        RunningAuthorityEvent::ControllerDisconnected {
            command_sequence,
            lease_id,
            completed_step,
            applied,
        } => {
            output.kind = "controllerDisconnected".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.controller_disconnect = Some(BackgroundControllerDisconnect {
                lease_id: u64_hex(lease_id),
                completed_step: u64_hex(completed_step),
                applied,
            });
        }
        RunningAuthorityEvent::ControllerActionApplied {
            command_sequence,
            lease_id,
            completed_step,
        } => {
            output.kind = "controllerActionApplied".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.controller_action_lease_id = Some(u64_hex(lease_id));
            output.controller_action_completed_step = Some(u64_hex(completed_step));
        }
        RunningAuthorityEvent::LiveSettingsApplied {
            command_sequence,
            config_revision,
            config_hash,
            effective_step,
        } => {
            output.kind = "liveSettingsApplied".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.settings_config_revision = Some(u64_hex(config_revision));
            output.settings_config_hash = Some(config_hash);
            output.settings_effective_step = Some(u64_hex(effective_step));
        }
        RunningAuthorityEvent::FinishGenerationAccepted {
            command_sequence,
            effective_step,
        } => {
            output.kind = "finishGenerationAccepted".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.finish_generation_effective_step = Some(u64_hex(effective_step));
        }
        RunningAuthorityEvent::EntitySelected {
            command_sequence,
            selection,
        } => {
            output.kind = "entitySelected".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.entity_selection = selection.map(|value| BackgroundEntitySelection {
                token: value.token.encode(),
                snake_id: value.frame_v1_id,
                population_slot: value.population_slot,
                life_ordinal: value.life_ordinal.map(u64_hex),
                x: value.x,
                y: value.y,
                radius: value.radius,
                skin: value.skin,
                retained_origin: value.retained_origin.map(BackgroundRetainedOrigin::from),
            });
        }
        RunningAuthorityEvent::GodModeMoved {
            command_sequence,
            frame_v1_id,
            x,
            y,
            effective_step,
        } => {
            output.kind = "godModeMoved".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.god_mode_move = Some(BackgroundGodModeMove {
                snake_id: frame_v1_id,
                x,
                y,
                effective_step: u64_hex(effective_step),
            });
        }
        RunningAuthorityEvent::GodModeKilled {
            command_sequence,
            frame_v1_id,
            pellets_dropped,
            effective_step,
        } => {
            output.kind = "godModeKilled".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.god_mode_kill = Some(BackgroundGodModeKill {
                snake_id: frame_v1_id,
                pellets_dropped: usize_hex(pellets_dropped, "God Mode corpse pellets")?,
                effective_step: u64_hex(effective_step),
            });
        }
        RunningAuthorityEvent::SavedBrainSpawned {
            command_sequence,
            frame_v1_id,
            effective_step,
        } => {
            output.kind = "savedBrainSpawned".into();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.saved_brain_spawn = Some(BackgroundHallOfFameResurrection {
                snake_id: frame_v1_id,
                effective_step: u64_hex(effective_step),
            });
        }
        RunningAuthorityEvent::HallOfFameResurrected {
            command_sequence,
            frame_v1_id,
            effective_step,
        } => {
            output.kind = "hallOfFameResurrected".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.hall_of_fame_resurrection = Some(BackgroundHallOfFameResurrection {
                snake_id: frame_v1_id,
                effective_step: u64_hex(effective_step),
            });
        }
        RunningAuthorityEvent::InspectorChanged {
            command_sequence,
            enabled,
        } => {
            output.kind = "inspectorChanged".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.inspector_enabled = Some(enabled);
        }
        RunningAuthorityEvent::VisualizationChanged {
            command_sequence,
            enabled,
        } => {
            output.kind = "visualizationChanged".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.visualization_enabled = Some(enabled);
        }
        RunningAuthorityEvent::ControllerObservations { messages, .. }
        | RunningAuthorityEvent::ControllerMessages { messages, .. } => {
            let message_count = messages.len();
            for message in messages.into_vec() {
                let event = message.event;
                let observation =
                    event.delivery_kind == crate::engine::ExternalDeliveryEventKind::Observation;
                let (target, kind) = if observation {
                    (&mut output.controller_observations, "observation")
                } else {
                    (&mut output.controller_messages, "replacementAssignment")
                };
                target
                    .get_or_insert_with(|| Vec::with_capacity(message_count))
                    .push(BackgroundControllerMessage {
                        reward_json: message.reward_json.map(|json| json.into_string()),
                        operation_epoch: u64_hex(event.step_key.operation_epoch()),
                        event_sequence: u64_hex(event.event_sequence),
                        connection_id: u64_hex(event.connection_id),
                        lease_id: u64_hex(event.lease_id),
                        controller_kind: controller_kind_name(event.controller_kind).to_owned(),
                        internal_snake_id: u64_hex(event.snake_id),
                        snake_id: message.frame_v1_id,
                        source_completed_step: u64_hex(event.step_key.source_completed_step()),
                        kind: kind.to_owned(),
                        sensors: observation
                            .then(|| message.sensors.iter().copied().map(f64::from).collect()),
                        x: observation.then_some(event.position.x),
                        y: observation.then_some(event.position.y),
                        direction: observation.then_some(event.direction),
                        resume_token: message.resume_token.map(|token| token.into_string()),
                    });
            }
            output.kind = if output.controller_messages.is_some() {
                "controllerMessages"
            } else {
                "controllerObservations"
            }
            .to_owned();
        }
        RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
            command_sequence,
            matched_acceptances,
            matched_failures,
            ignored_receipts,
            remaining,
            published_completed_step,
        } => {
            output.kind = "controllerDeliveryReceiptsApplied".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.controller_receipt_resolution = Some(BackgroundControllerReceiptResolution {
                matched_acceptances: usize_hex(matched_acceptances, "controller acceptances")?,
                matched_failures: usize_hex(matched_failures, "controller failures")?,
                ignored_receipts: usize_hex(ignored_receipts, "ignored controller receipts")?,
                remaining: usize_hex(remaining, "pending controller receipts")?,
                published_completed_step: published_completed_step.map(u64_hex),
            });
        }
        RunningAuthorityEvent::GenerationTransitionPending {
            ticket_sequence,
            source_key,
            reason,
            successor_generation,
            successor_completed_step,
        } => {
            output.kind = "generationTransitionPending".to_owned();
            output.transition = Some(Stage6BackgroundGenerationTransition {
                ticket_sequence: u64_hex(ticket_sequence),
                source_key: background_step_key_to_napi(source_key),
                reason: generation_transition_reason_name(reason).to_owned(),
                successor_generation: u64_hex(successor_generation),
                successor_completed_step: u64_hex(successor_completed_step),
            });
        }
        RunningAuthorityEvent::GenerationCheckpointPublished {
            workload,
            command_sequence,
            descriptor,
            hall_of_fame_weights,
            commit_record,
        } => {
            output.kind = "generationCheckpointPublished".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.checkpoint = Some(Stage6GenerationCheckpointPublication {
                workload_json: workload.map(|work| work.to_json()),
                descriptor: checkpoint_descriptor_to_napi(*descriptor),
                hall_of_fame_weights: hall_of_fame_weights_descriptor_to_napi(
                    *hall_of_fame_weights,
                ),
                generation_commit: generation_commit_to_napi(*commit_record),
            });
        }
        RunningAuthorityEvent::GenerationPersistenceAcknowledged {
            command_sequence,
            operation_id,
        } => {
            output.kind = "generationPersistenceAcknowledged".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.acknowledged_operation_id = Some(operation_id.as_str().to_owned());
        }
        RunningAuthorityEvent::GenerationReassignmentsPrepared {
            command_sequence,
            ready,
            assignments,
        } => {
            output.kind = "generationReassignmentsPrepared".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.reassignments = Some(Stage6BackgroundGenerationReassignments {
                ready,
                assignments: assignments
                    .into_vec()
                    .into_iter()
                    .map(|assignment| Stage6BackgroundGenerationAssignment {
                        reward_json: assignment.reward_json.map(|json| json.into_string()),
                        final_sensors: (!assignment.final_sensors.is_empty()).then(|| {
                            assignment
                                .final_sensors
                                .iter()
                                .copied()
                                .map(f64::from)
                                .collect()
                        }),
                        operation_epoch: u64_hex(assignment.operation_epoch),
                        event_sequence: u64_hex(assignment.event_sequence),
                        connection_id: u64_hex(assignment.connection_id),
                        lease_id: u64_hex(assignment.lease_id),
                        controller_kind: controller_kind_name(assignment.controller_kind)
                            .to_owned(),
                        snake_id: u64_hex(assignment.snake_id),
                        frame_v1_id: u64_hex(u64::from(assignment.frame_v1_id)),
                        resume_token: assignment.resume_token.into_string(),
                    })
                    .collect(),
            });
        }
        RunningAuthorityEvent::GenerationAssignmentReceiptsApplied {
            command_sequence,
            matched_acceptances,
            matched_failures,
            ignored_receipts,
            state,
        } => {
            output.kind = "generationAssignmentReceiptsApplied".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            let (state_name, remaining, source_key, successor_generation, successor_completed_step) =
                match state {
                    GenerationAssignmentReceiptState::Pending { remaining } => (
                        "pending",
                        Some(usize_hex(remaining, "pending generation assignment count")?),
                        None,
                        None,
                        None,
                    ),
                    GenerationAssignmentReceiptState::Ready {
                        source_key,
                        successor_generation,
                        successor_completed_step,
                    } => (
                        "ready",
                        None,
                        Some(background_step_key_to_napi(source_key)),
                        Some(u64_hex(successor_generation)),
                        Some(u64_hex(successor_completed_step)),
                    ),
                };
            output.receipt_resolution = Some(Stage6BackgroundGenerationReceiptResolution {
                matched_acceptances: usize_hex(
                    matched_acceptances,
                    "matched generation assignment acceptances",
                )?,
                matched_failures: usize_hex(
                    matched_failures,
                    "matched generation assignment failures",
                )?,
                ignored_receipts: usize_hex(
                    ignored_receipts,
                    "ignored generation assignment receipts",
                )?,
                state: state_name.to_owned(),
                remaining,
                source_key,
                successor_generation,
                successor_completed_step,
            });
        }
        RunningAuthorityEvent::GenerationStartPublished {
            command_sequence,
            resolution,
        } => {
            output.kind = "generationStartPublished".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            let publication = resolution.publication;
            let external_assignments =
                u32::try_from(publication.external_assignments).map_err(|_| {
                    EngineError::new(
                        EngineErrorCode::Faulted,
                        "generation external assignment count exceeds the N-API field",
                    )
                })?;
            let unavailable_controllers = publication
                .unavailable_controller_reservations
                .into_iter()
                .map(|reservation| Stage6BackgroundUnavailableController {
                    source_lease_id: u64_hex(reservation.source_lease_id),
                    source_snake_id: u64_hex(reservation.source_snake_id),
                    controller_kind: controller_kind_name(reservation.controller_kind).to_owned(),
                    scope: reservation.scope,
                    resume_token: reservation.resume_token,
                    disconnected_at_ms: reservation.disconnected_at_ms.map(u64_hex),
                    grace_expires_at_ms: reservation.grace_expires_at_ms.map(u64_hex),
                    reason: unavailable_controller_reason_name(reservation.reason).to_owned(),
                })
                .collect();
            output.generation_start = Some(Stage6BackgroundGenerationStart {
                ticket_sequence: u64_hex(resolution.ticket_sequence),
                due_steps: usize_hex(resolution.due_steps, "generation transition due steps")?,
                source_key: background_step_key_to_napi(publication.source_key),
                publication: Stage6GenerationStartPublication {
                    world_epoch: u64_hex(publication.world_epoch),
                    generation: u64_hex(publication.generation),
                    completed_step: u64_hex(publication.completed_step),
                    population_epoch: u64_hex(publication.population_epoch),
                    external_assignments,
                },
                unavailable_controllers,
            });
        }
        RunningAuthorityEvent::ImportPublished {
            command_sequence,
            world_epoch,
            generation,
            completed_step,
            population_epoch,
        } => {
            output.kind = "importPublished".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.import_publication = Some(Stage6RunStartPublication {
                world_epoch: u64_hex(world_epoch),
                generation: u64_hex(generation),
                completed_step: u64_hex(completed_step),
                population_epoch: u64_hex(population_epoch),
            });
        }
        RunningAuthorityEvent::ImportStaged { command_sequence } => {
            output.kind = "importStaged".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
        }
        RunningAuthorityEvent::ImportCancelled { command_sequence } => {
            output.kind = "importCancelled".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
        }
        RunningAuthorityEvent::CommandRejected {
            command_sequence,
            code,
            detail,
        } => {
            output.kind = "commandRejected".to_owned();
            output.command_sequence = Some(u64_hex(command_sequence));
            output.rejection_code = Some(error_code_name(code).to_owned());
            output.rejection_detail = Some(detail);
        }
    }
    Ok(output)
}

pub(super) fn empty_background_generation_event() -> Stage6BackgroundGenerationEvent {
    Stage6BackgroundGenerationEvent {
        kind: String::new(),
        command_sequence: None,
        transition: None,
        checkpoint: None,
        acknowledged_operation_id: None,
        reassignments: None,
        receipt_resolution: None,
        generation_start: None,
        import_publication: None,
        display: None,
        controller_observations: None,
        controller_messages: None,
        controller_receipt_resolution: None,
        controller_action_lease_id: None,
        controller_action_completed_step: None,
        settings_config_revision: None,
        settings_config_hash: None,
        settings_effective_step: None,
        finish_generation_effective_step: None,
        entity_selection: None,
        god_mode_move: None,
        god_mode_kill: None,
        hall_of_fame_resurrection: None,
        saved_brain_spawn: None,
        visualization_enabled: None,
        inspector_enabled: None,
        teacher_capture_epoch: None,
        teacher_capture_enabled: None,
        shadow_change: None,
        controller_disconnect: None,
        controller_join_assignment: None,
        controller_join_resolution: None,
        controller_reclaim_assignment: None,
        controller_reclaim_resolution: None,
        rejection_code: None,
        rejection_detail: None,
        fault_code: None,
        fault_detail: None,
    }
}

pub(super) fn background_step_key_to_napi(key: PhysicsStepKey) -> Stage6BackgroundStepKey {
    Stage6BackgroundStepKey {
        world_epoch: u64_hex(key.world_epoch()),
        generation: u64_hex(key.generation()),
        source_completed_step: u64_hex(key.source_completed_step()),
        population_epoch: u64_hex(key.population_epoch()),
        config_revision: u64_hex(key.config_revision()),
        config_sha256: sha256_hex(key.config_hash()),
        operation_epoch: u64_hex(key.operation_epoch()),
    }
}

pub(crate) fn background_generation_health_to_napi(
    health: EngineHealth,
    calculation_workers: usize,
    include_step_timing_histogram: bool,
) -> std::result::Result<Stage6BackgroundGenerationHealth, EngineError> {
    let running = health.running_authority.ok_or_else(|| {
        EngineError::new(
            EngineErrorCode::Faulted,
            "background generation fixture lost its running-authority health",
        )
    })?;
    Ok(Stage6BackgroundGenerationHealth {
        calculation_workers: u32::try_from(calculation_workers).map_err(|_| {
            EngineError::new(
                EngineErrorCode::Faulted,
                "calculation worker count exceeds Uint32",
            )
        })?,
        lifecycle: lifecycle_name(health.lifecycle).to_owned(),
        loop_state: running_loop_state_name(running.loop_state).to_owned(),
        world_epoch: u64_hex(running.world_epoch),
        generation: u64_hex(running.generation),
        completed_step: u64_hex(running.completed_step),
        generation_checkpoint_published: running.generation_checkpoint_published,
        generation_persistence_acknowledged: running.generation_persistence_acknowledged,
        pending_external_deliveries: usize_hex(
            running.pending_external_deliveries,
            "pending external delivery count",
        )?,
        scheduler_completed_steps: u64_hex(running.scheduler_completed_steps),
        scheduler_dropped_wall_micros: u64_hex(running.scheduler_dropped_wall_micros),
        scheduler_overloaded: running.scheduler_overloaded,
        command_service_boundaries: u64_hex(running.command_service_boundaries),
        coordinator_work_bytes: u64_hex(running.coordinator_work_bytes),
        processed_commands: u64_hex(health.processed_commands),
        step_timing_samples: u64_hex(running.step_timing_samples),
        step_timing_total_micros: u64_hex(running.step_timing_total_micros),
        step_timing_max_micros: u64_hex(running.step_timing_max_micros),
        step_timing_p95_micros: u64_hex(running.step_timing_p95_micros),
        step_timing_p99_micros: u64_hex(running.step_timing_p99_micros),
        step_timing_bucket_upper_micros: if include_step_timing_histogram {
            Some(
                running
                    .step_timing_bucket_upper_micros
                    .into_iter()
                    .map(u64_hex)
                    .collect(),
            )
        } else {
            None
        },
        step_timing_bucket_counts: if include_step_timing_histogram {
            Some(
                running
                    .step_timing_bucket_counts
                    .into_iter()
                    .map(u64_hex)
                    .collect(),
            )
        } else {
            None
        },
        step_timing_histogram_consistent: include_step_timing_histogram
            && running.step_timing_histogram_consistent,
        slow_step_samples: u64_hex(running.slow_step_samples),
        slow_step_control_micros: u64_hex(running.slow_step_control_micros),
        slow_step_control_index_micros: u64_hex(running.slow_step_control_index_micros),
        slow_step_control_neural_micros: u64_hex(running.slow_step_control_neural_micros),
        slow_step_control_sensing_micros: u64_hex(running.slow_step_control_sensing_micros),
        slow_step_control_inference_micros: u64_hex(running.slow_step_control_inference_micros),
        slow_step_world_micros: u64_hex(running.slow_step_world_micros),
        slow_step_other_micros: u64_hex(running.slow_step_other_micros),
        terminal_step_samples: u64_hex(running.terminal_step_samples),
        terminal_step_total_micros: u64_hex(running.terminal_step_total_micros),
        terminal_step_max_micros: u64_hex(running.terminal_step_max_micros),
        terminal_step_control_micros: u64_hex(running.terminal_step_control_micros),
        terminal_step_world_micros: u64_hex(running.terminal_step_world_micros),
        terminal_step_preparation_micros: u64_hex(running.terminal_step_preparation_micros),
        terminal_step_evolution_micros: u64_hex(running.terminal_step_evolution_micros),
        terminal_step_evolution_summary_micros: u64_hex(
            running.terminal_step_evolution_summary_micros,
        ),
        terminal_step_evolution_reproduction_micros: u64_hex(
            running.terminal_step_evolution_reproduction_micros,
        ),
        terminal_step_admission_micros: u64_hex(running.terminal_step_admission_micros),
        terminal_step_other_micros: u64_hex(running.terminal_step_other_micros),
        fault_code: health
            .fault
            .as_ref()
            .map(|fault| error_code_name(fault.code()).to_owned()),
        fault_detail: health.fault.map(|fault| fault.detail().to_owned()),
    })
}

pub(super) fn usize_hex(value: usize, field: &str) -> std::result::Result<String, EngineError> {
    u64::try_from(value).map(u64_hex).map_err(|_| {
        EngineError::new(
            EngineErrorCode::Faulted,
            format!("{field} exceeds the exact N-API u64 field"),
        )
    })
}

pub(super) fn sha256_hex(bytes: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

const fn generation_transition_reason_name(reason: GenerationTransitionReason) -> &'static str {
    match reason {
        GenerationTransitionReason::Manual => "manual",
        GenerationTransitionReason::Duration => "duration",
        GenerationTransitionReason::EarlyAliveCount => "earlyAliveCount",
    }
}

const fn controller_kind_name(kind: ControllerKind) -> &'static str {
    match kind {
        ControllerKind::Player => "player",
        ControllerKind::ReinforcementLearning => "reinforcementLearning",
    }
}

const fn unavailable_controller_reason_name(
    reason: crate::engine::external_replacement::UnavailableControllerReason,
) -> &'static str {
    match reason {
        crate::engine::external_replacement::UnavailableControllerReason::SnakeUnavailable => {
            "snakeUnavailable"
        }
        crate::engine::external_replacement::UnavailableControllerReason::GraceExpired => {
            "graceExpired"
        }
    }
}

const fn running_loop_state_name(
    state: crate::engine::running_loop::RunningAuthorityLoopState,
) -> &'static str {
    match state {
        crate::engine::running_loop::RunningAuthorityLoopState::ControllerJoinPending => {
            "controllerJoinPending"
        }
        crate::engine::running_loop::RunningAuthorityLoopState::ControllerReclaimPending => {
            "controllerReclaimPending"
        }
        crate::engine::running_loop::RunningAuthorityLoopState::Ready => "ready",
        crate::engine::running_loop::RunningAuthorityLoopState::ExternalDeliveryPending => {
            "externalDeliveryPending"
        }
        crate::engine::running_loop::RunningAuthorityLoopState::GenerationTransitionPending => {
            "generationTransitionPending"
        }
        crate::engine::running_loop::RunningAuthorityLoopState::ImportPending => "importPending",
        crate::engine::running_loop::RunningAuthorityLoopState::Faulted => "faulted",
    }
}

/// Convert one retained Rust assignment to exact scalar wire fields.
#[cfg(feature = "engine-test-hooks")]
pub(super) fn generation_assignment_to_napi(
    assignment: GenerationHandoffAssignment,
) -> Stage6GenerationAssignment {
    Stage6GenerationAssignment {
        operation_epoch: u64_hex(assignment.operation_epoch),
        event_sequence: u64_hex(assignment.event_sequence),
        connection_id: u64_hex(assignment.connection_id),
        lease_id: u64_hex(assignment.lease_id),
        snake_id: u64_hex(assignment.snake_id),
        resume_token: assignment.resume_token,
    }
}

/// Convert current authority/barrier proof to bounded scalar wire fields.
#[cfg(feature = "engine-test-hooks")]
pub(super) fn generation_snapshot_to_napi(
    snapshot: GenerationHandoffSnapshot,
) -> Stage6GenerationHandoffSnapshot {
    Stage6GenerationHandoffSnapshot {
        world_epoch: u64_hex(snapshot.world_epoch),
        generation: u64_hex(snapshot.generation),
        completed_step: u64_hex(snapshot.completed_step),
        transition_pending: snapshot.transition_pending,
        checkpoint_published: snapshot.checkpoint_published,
        persistence_acknowledged: snapshot.persistence_acknowledged,
        generation_checkpoint_publications: snapshot.generation_checkpoint_publications,
        authority_publications: snapshot.authority_publications,
    }
}

/// Convert fresh run-start barrier proof to bounded exact scalar wire fields.
#[cfg(feature = "engine-test-hooks")]
pub(super) fn run_start_snapshot_to_napi(
    snapshot: RunStartHandoffSnapshot,
) -> Result<Stage6RunStartHandoffSnapshot> {
    let snake_count = u64::try_from(snapshot.snake_count).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "run-start snake count exceeds the bounded N-API fixture field",
        )
    })?;
    let pellet_count = u64::try_from(snapshot.pellet_count).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "run-start pellet count exceeds the bounded N-API fixture field",
        )
    })?;
    Ok(Stage6RunStartHandoffSnapshot {
        transition_epoch: u64_hex(snapshot.transition_epoch),
        generation: u64_hex(snapshot.generation),
        completed_step: u64_hex(snapshot.completed_step),
        checkpoint_published: snapshot.checkpoint_published,
        persistence_acknowledged: snapshot.persistence_acknowledged,
        authority_published: snapshot.authority_published,
        snake_count: u64_hex(snake_count),
        pellet_count: u64_hex(pellet_count),
        checkpoint_publications: snapshot.checkpoint_publications,
        authority_publications: snapshot.authority_publications,
    })
}
