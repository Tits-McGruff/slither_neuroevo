use super::*;

#[test]
fn inspector_shadow_join_rejects_other_lives_generations_steps_and_activations() {
    use crate::engine::inspector::*;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut snake = source.world.snakes[0].clone();
    snake.kind = SnakeKind::Baseline;
    snake.population_slot = None;
    let target = InspectorTarget {
        world_epoch: 1,
        generation: 1,
        physical_id: snake.id,
        frame_v1_id: snake.frame_v1_id,
    };
    let key = crate::engine::physics::PhysicsStepKey::new(1, 1, 5, 1, 1, [0; 32], 1);
    let mut capture = InspectorCapture::new(target, MAX_INSPECTOR_BYTES).unwrap();
    let layout = InspectorLayout::from(
        &crate::engine::sensor_layout::SensorLayout::for_profile(
            crate::engine::sensor_contract::SensorProfile::Basic,
            8,
        )
        .unwrap(),
    );
    capture.stage(
        key,
        &snake,
        InspectorObservationSource::BaselineV2,
        layout,
        &vec![0.0; layout.input_size],
        None,
        None,
        Default::default(),
    );
    capture.publish(key, |_, _, _| false);
    let good = crate::engine::shadows::ShadowSnapshot {
        artifact_sha256: [2; 32],
        capture_epoch: [1; 16],
        baseline_slot: 7,
        activation_sequence: 9,
        activation_step: 0,
        completed_step: 6,
        last_sample_step: 6,
        generation: 1,
        physical_snake_id: snake.id,
        enabled: true,
        error: None,
        raw_outputs: [0.25, 0.75],
        totals: Default::default(),
        strategies: [Default::default(); 4],
        life_stages: [Default::default(); 3],
        strata: [Default::default(); 6],
        cruising: Default::default(),
        pre_death: Default::default(),
        deaths: 0,
    };
    let mut mismatches = [good; 7];
    mismatches[0].physical_snake_id += 1;
    mismatches[1].generation += 1;
    mismatches[2].last_sample_step += 1;
    mismatches[3].enabled = false;
    mismatches[4].activation_sequence = 0;
    mismatches[5].capture_epoch = [0; 16];
    mismatches[6].raw_outputs[0] = f32::NAN;
    for invalid in mismatches {
        capture.join_shadows(key, [Some(invalid), None, None, None]);
        assert!(capture
            .latest(1, 1, 6)
            .unwrap()
            .shadow_outputs
            .iter()
            .all(Option::is_none));
    }
    let wrong_key = crate::engine::physics::PhysicsStepKey::new(2, 1, 5, 1, 1, [0; 32], 1);
    capture.join_shadows(wrong_key, [Some(good), None, None, None]);
    assert!(capture.latest(1, 1, 6).unwrap().shadow_outputs[0].is_none());
    capture.join_shadows(key, [Some(good), None, None, None]);
    assert_eq!(
        capture.latest(1, 1, 6).unwrap().shadow_outputs[0]
            .unwrap()
            .raw_outputs,
        [0.25, 0.75]
    );
}

#[test]
fn running_step_cannot_begin_from_a_generation_boundary() {
    let graph = default_graph();
    let mut boundary =
        own(candidate(&graph, 1), graph, usize::MAX).expect("generation boundary must admit");
    assert!(matches!(
        boundary.begin_running_step(),
        Err(StateError::InvalidField { field: "phase", .. })
    ));
}

#[test]
fn scheduler_rebind_excludes_generation_persistence_wait_from_wall_debt() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    set_complete_setting(
        &mut candidate,
        "generationSeconds",
        NormalizedSettingValue::Float(8.0),
    );
    set_complete_setting(
        &mut candidate,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    );
    candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("terminal fixture must construct the coordinator");
    let mut scheduler =
        FixedStepScheduler::try_new(&authority, FixedStepSchedulerPolicy::provisional_defaults())
            .expect("terminal fixture must construct the scheduler");
    scheduler.reset_wall_clock(&authority, 100).unwrap();
    scheduler
        .service_after_command_drain(&authority, 117, SchedulerServiceMode::Background)
        .unwrap();
    let step = scheduler.prepare_due_step(&authority).unwrap();
    match coordinator
        .advance_nonterminal(&mut authority, step.running_step_inputs())
        .expect("terminal scheduled step must stage its generation boundary")
    {
        RunningStepProgress::GenerationTransitionPending(_) => {}
        other => panic!("expected pending generation transition, got {other:?}"),
    }
    let directory = GenerationHandoffDirectory::new();
    let descriptor = coordinator
        .publish_pending_generation_checkpoint(
            &authority,
            directory.path(),
            CheckpointOperationId::parse("00000000000000000000000000000083").unwrap(),
            &generation_handoff_checkpoint_limits(),
            &default_graph_limits(),
        )
        .expect("terminal checkpoint must publish");
    coordinator
        .acknowledge_pending_generation_persistence(&authority, &descriptor)
        .expect("exact metadata commit must construct the base world");
    assert!(matches!(
        coordinator
            .prepare_acknowledged_generation_reassignments(&authority)
            .expect("no-controller generation needs no delivery"),
        GenerationReassignmentProgress::Ready(_)
    ));
    let publication = coordinator
        .publish_acknowledged_generation_start(&mut authority)
        .expect("fully durable no-controller generation must publish");
    assert_eq!(publication.external_assignments, 0);
    assert!(publication.unavailable_controller_reservations.is_empty());
    assert!(scheduler.diagnostics().step_pending);
    scheduler
        .commit_generation_transition(&authority, step, &publication, 10_000)
        .expect("scheduler must rebind to the exact running successor");
    let after_rebind = scheduler.diagnostics();
    assert!(!after_rebind.step_pending);
    assert_eq!(after_rebind.completed_steps, 1);
    assert_eq!(
        after_rebind.pending_simulation_seconds.to_bits(),
        step.wall_accumulator_after_step().to_bits()
    );
    assert!((after_rebind.observed_wall_seconds - 0.017).abs() < 1.0e-12);

    scheduler
        .service_after_command_drain(&authority, 10_017, SchedulerServiceMode::Background)
        .expect("new generation must accept the next command-service boundary");
    let after_next_wall = scheduler.diagnostics();
    assert!((after_next_wall.observed_wall_seconds - 0.034).abs() < 1.0e-12);
    assert!(after_next_wall.pending_simulation_seconds < 0.1);
}

#[test]
fn exact_persistence_acknowledgement_precedes_next_world_construction() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    set_complete_setting(
        &mut candidate,
        "generationSeconds",
        NormalizedSettingValue::Float(8.0),
    );
    set_complete_setting(
        &mut candidate,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    );
    candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let limits = RunningStepWorkLimits::provisional_defaults();
    let expected_projection = authority
        .running_step_config(limits)
        .expect("terminal settings must project");
    let mut coordinator = RunningStepCoordinator::try_new(&authority, limits)
        .expect("terminal fixture must construct the coordinator");

    let transition = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("terminal result must stage its exact boundary")
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected pending generation transition, got {other:?}"),
    };
    let source_key = transition.source_key();
    assert!(transition.checkpoint_descriptor().is_none());
    assert!(!transition.persistence_acknowledged());
    assert!(matches!(
        coordinator.prepare_acknowledged_generation_start(&authority),
        Err(RunningStepError::GenerationPersistenceNotAcknowledged)
    ));

    let directory = GenerationHandoffDirectory::new();
    let operation = CheckpointOperationId::parse("00000000000000000000000000000081").unwrap();
    let descriptor = coordinator
        .publish_pending_generation_checkpoint(
            &authority,
            directory.path(),
            operation.clone(),
            &generation_handoff_checkpoint_limits(),
            &default_graph_limits(),
        )
        .expect("managed checkpoint publication must succeed");
    assert!(directory
        .path()
        .join(&descriptor.relative_filename)
        .is_file());
    assert_eq!(
        coordinator
            .publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                operation,
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            )
            .expect("an exact retry must reuse the retained descriptor"),
        descriptor
    );
    assert!(matches!(
        coordinator.publish_pending_generation_checkpoint(
            &authority,
            directory.path(),
            CheckpointOperationId::parse("00000000000000000000000000000082").unwrap(),
            &generation_handoff_checkpoint_limits(),
            &default_graph_limits(),
        ),
        Err(RunningStepError::GenerationCheckpointAlreadyPublished { .. })
    ));

    let mut wrong = descriptor.clone();
    let replacement = if wrong.logical_root_sha256.ends_with('0') {
        "1"
    } else {
        "0"
    };
    wrong.logical_root_sha256.replace_range(63..64, replacement);
    assert!(matches!(
        coordinator.acknowledge_pending_generation_persistence(&authority, &wrong),
        Err(
            RunningStepError::GenerationPersistenceAcknowledgementMismatch {
                field: "logical root"
            }
        )
    ));
    assert_eq!(authority.state(), &source);
    assert!(!coordinator
        .pending_generation_transition()
        .expect("wrong acknowledgement must retain the transition")
        .persistence_acknowledged());

    let (first_world, first_rng, first_allocators) = {
        let prepared = coordinator
            .acknowledge_pending_generation_persistence(&authority, &descriptor)
            .expect("the exact committed descriptor may construct the next world");
        let expected_snakes = prepared
            .source()
            .config
            .population_count
            .checked_add(prepared.source().config.baseline_count)
            .unwrap();
        assert_eq!(prepared.world().snakes.len(), expected_snakes);
        assert_eq!(
            prepared.world().pellets.len(),
            expected_projection.world_step.prefix.ambient.target_count
        );
        (
            prepared.world().clone(),
            prepared.rng().clone(),
            prepared.allocators().clone(),
        )
    };
    assert_eq!(authority.state(), &source);
    let retained = coordinator
        .pending_generation_transition()
        .expect("acknowledged transition must remain pending until final publication");
    assert_eq!(retained.source_key(), source_key);
    assert_eq!(retained.checkpoint_descriptor(), Some(&descriptor));
    assert!(retained.persistence_acknowledged());
    assert!(matches!(
        coordinator.discard_pending_generation_transition(&authority, source_key),
        Err(RunningStepError::GenerationPersistenceAlreadyCommitted)
    ));

    let retried = coordinator
        .prepare_acknowledged_generation_start(&authority)
        .expect("a successful construction must be reborrowed without new draws");
    assert_eq!(retried.world(), &first_world);
    assert_eq!(retried.rng(), &first_rng);
    assert_eq!(retried.allocators(), &first_allocators);
    assert_eq!(authority.state(), &source);
}

#[test]
fn durable_generation_boundary_reassigns_connected_controller_before_authority_swap() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint {
            x: 1_200.0,
            y: -1_200.0,
        },
    );
    // A persisted one-past-limit cursor must still construct and publish the successor.
    candidate.allocators.next_frame_v1_id = FRAME_V1_EXHAUSTED_ID;
    set_complete_setting(
        &mut candidate,
        "generationSeconds",
        NormalizedSettingValue::Float(8.0),
    );
    set_complete_setting(
        &mut candidate,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    );
    candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("terminal external fixture must construct the coordinator");
    let transition = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 500,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("terminal external fixture must stage a generation boundary")
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected pending generation transition, got {other:?}"),
    };
    let source_key = transition.source_key();
    let directory = GenerationHandoffDirectory::new();
    let operation = CheckpointOperationId::parse("00000000000000000000000000000091")
        .expect("test operation ID");
    let descriptor = coordinator
        .publish_pending_generation_checkpoint(
            &authority,
            directory.path(),
            operation,
            &generation_handoff_checkpoint_limits(),
            &default_graph_limits(),
        )
        .expect("generation checkpoint file must publish");
    coordinator
        .acknowledge_pending_generation_persistence(&authority, &descriptor)
        .expect("exact metadata acknowledgement must construct the generation base");

    let (event, token) = match coordinator
        .prepare_acknowledged_generation_reassignments(&authority)
        .expect("connected owner must stage one reliable generation assignment")
    {
        GenerationReassignmentProgress::DeliveryPending(batch) => {
            assert_eq!(batch.events().len(), 1);
            assert_eq!(batch.remaining(), 1);
            let event = batch.events()[0];
            assert_eq!(event.step_key, source_key);
            assert_eq!(event.connection_id, 7);
            assert_eq!(event.controller_kind, ControllerKind::Player);
            assert!(matches!(
                event.delivery_kind,
                ExternalDeliveryEventKind::ReplacementAssignment { .. }
            ));
            assert!(event.snake_id >= EXTERNAL_ENTITY_ID_START);
            let token = batch
                .resume_token(0)
                .expect("assignment must carry a fresh token")
                .to_owned();
            assert!(!token.is_empty());
            assert_ne!(token, "external-delivery-1");
            (event, token)
        }
        GenerationReassignmentProgress::Ready(_) => {
            panic!("connected owner must wait for its exact local send result")
        }
    };
    assert_eq!(authority.state(), &source);

    let stale = ExternalDeliveryResult {
        step_key: source_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id + 1,
        lease_id: event.lease_id,
        accepted: true,
    };
    let pending = coordinator
        .submit_external_delivery_results(&mut authority, &[stale])
        .expect("stale connection result must be ignored");
    assert_eq!(pending.ignored_results, 1);
    assert!(matches!(pending.state, ExternalDeliveryState::Pending(_)));
    assert_eq!(authority.state(), &source);

    let accepted = ExternalDeliveryResult {
        step_key: source_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: event.lease_id,
        accepted: true,
    };
    let resolved = coordinator
        .submit_external_delivery_results(&mut authority, &[accepted])
        .expect("exact assignment result must resolve the generation handoff");
    assert_eq!(resolved.matched_acceptances, 1);
    assert!(matches!(
        resolved.state,
        ExternalDeliveryState::GenerationAssignmentsReady(transition)
            if transition.source_key() == source_key
                && transition.persistence_acknowledged()
    ));
    assert_eq!(authority.state(), &source);
    assert!(matches!(
        coordinator
            .prepare_acknowledged_generation_reassignments(&authority)
            .expect("resolved assignment must be reborrowed without new entropy"),
        GenerationReassignmentProgress::Ready(transition)
            if transition.source_key() == source_key
    ));
    let publication = coordinator
        .publish_acknowledged_generation_start(&mut authority)
        .expect("durable and assignment-resolved generation must publish atomically");
    assert_eq!(publication.source_key, source_key);
    assert_eq!(publication.world_epoch, authority.world_epoch());
    assert_eq!(publication.generation, source.generation.generation + 1);
    assert_eq!(
        publication.completed_step,
        source.generation.completed_step + 1
    );
    assert_eq!(
        publication.population_epoch,
        source.generation.population_epoch + 1
    );
    assert_eq!(publication.memory, authority.memory_estimate());
    assert_eq!(publication.external_assignments, 1);
    assert!(authority.state().allocators.next_frame_v1_id < FRAME_V1_EXHAUSTED_ID);
    let mut visible_ids: Vec<_> = authority
        .state()
        .world
        .snakes
        .iter()
        .map(|snake| snake.frame_v1_id)
        .collect();
    visible_ids.sort_unstable();
    assert!(visible_ids.windows(2).all(|pair| pair[0] != pair[1]));
    assert!(publication.unavailable_controller_reservations.is_empty());
    assert_eq!(authority.state().phase, AuthorityPhase::Running);
    assert_eq!(
        authority
            .state()
            .generation
            .wall_accumulator_seconds
            .to_bits(),
        0.0_f64.to_bits()
    );
    let replacement = authority
        .state()
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == event.snake_id)
        .expect("published generation must contain the assigned snake");
    assert!(replacement.alive);
    assert_eq!(replacement.kind, SnakeKind::External);
    let lease = authority
        .state()
        .world
        .controller_leases
        .iter()
        .find(|lease| lease.id == event.lease_id)
        .expect("published generation must contain the accepted lease");
    assert_eq!(lease.connection_id, Some(event.connection_id));
    assert_eq!(lease.resume_token, token);
    assert!(coordinator.pending_generation_transition().is_none());
    assert!(matches!(
        coordinator.advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 501,
                wall_accumulator_seconds: 0.0,
            }
        ),
        Err(RunningStepError::AuthorityMismatch {
            field: "world epoch"
        })
    ));
    RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
        .expect("the published successor must admit a fresh running coordinator");
}

#[test]
fn reward_generation_closure_retains_cutoff_state_and_death_precedence() {
    use crate::contracts::{ExternalRewardEndReason, ExternalRewardTransitionKind};
    use crate::engine::external_reward::{
        ExternalRewardLedger, RewardBoundary, RewardLifeIdentity,
    };

    // Live cutoff, previously committed God Mode death, and collision on
    // the very step that ends the generation must remain distinguishable.
    for death_case in 0..3 {
        let graph = default_graph();
        let mut candidate = complete_running_candidate(&graph);
        let external_id = EXTERNAL_ENTITY_ID_START;
        push_connected_external_fixture(
            &mut candidate,
            &graph,
            external_id,
            2,
            1,
            7,
            WorldPoint {
                x: 1_200.0,
                y: -1_200.0,
            },
        );
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let snake = authority
            .state()
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == external_id)
            .unwrap();
        let old_frame_id = snake.frame_v1_id;
        let ledger = ExternalRewardLedger::new(
            RewardLifeIdentity {
                world_epoch: authority.world_epoch(),
                lease_id: 1,
                physical_snake_id: external_id,
                frame_snake_id: old_frame_id,
            },
            RewardBoundary {
                completed_step: 0,
                elapsed_seconds: 0.0,
            },
            snake.body.len,
            snake.points,
        )
        .unwrap();
        let lease = authority
            .candidate
            .world
            .controller_leases
            .last_mut()
            .unwrap();
        lease.kind = ControllerKind::ReinforcementLearning;
        lease.reward = Some(ledger);
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .unwrap();
        let (initial, initial_observation) = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .unwrap()
        {
            RunningStepProgress::ExternalDeliveryPending(batch) => (
                batch.events()[0]
                    .reward
                    .unwrap()
                    .envelope("cutoff-fixture")
                    .unwrap(),
                batch.observation(0).unwrap().to_vec(),
            ),
            _ => panic!("initial observation must await output admission"),
        };
        coordinator
            .admit_pending_observations(&mut authority)
            .unwrap();
        if death_case == 1 {
            let physics = authority
                .running_step_config(RunningStepWorkLimits::provisional_defaults())
                .unwrap()
                .world_step
                .physics;
            authority
                .apply_god_mode_kill(old_frame_id, physics)
                .unwrap();
        }
        if death_case == 2 {
            authority
                .apply_god_mode_move(old_frame_id, 3_490.5, 0.0)
                .unwrap();
        }
        let source = authority.state().clone();
        coordinator.request_manual_finish();
        let full_ceiling = authority.memory_ceiling_bytes;
        let cutoff_bound = initial_observation.len() * std::mem::size_of::<f32>()
            + std::mem::size_of::<crate::engine::control_phase::ExternalRewardCutoff>();
        let numeric_peak = generation_numeric_peak_bytes(
            authority.memory.total_bytes,
            source.population.len(),
            &graph,
        )
        .unwrap();
        authority.memory_ceiling_bytes = numeric_peak + cutoff_bound - 1;
        assert!(matches!(
            coordinator.advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 101,
                    wall_accumulator_seconds: 0.0
                }
            ),
            Err(RunningStepError::State(error))
                if matches!(*error, StateError::MemoryCeilingExceeded { .. })
        ));
        assert_eq!(authority.state(), &source);
        assert!(coordinator.pending_generation_transition().is_none());
        authority.memory_ceiling_bytes = full_ceiling;
        assert!(matches!(
            coordinator
                .advance_nonterminal(
                    &mut authority,
                    RunningStepInputs {
                        wall_now_ms: 101,
                        wall_accumulator_seconds: 0.0
                    },
                )
                .unwrap(),
            RunningStepProgress::GenerationTransitionPending(_)
        ));
        assert_eq!(authority.state(), &source);
        let directory = GenerationHandoffDirectory::new();
        let operation = CheckpointOperationId::parse("00000000000000000000000000000092").unwrap();
        let descriptor = coordinator
            .publish_pending_generation_checkpoint(
                &authority,
                directory.path(),
                operation,
                &generation_handoff_checkpoint_limits(),
                &default_graph_limits(),
            )
            .unwrap();
        coordinator
            .acknowledge_pending_generation_persistence(&authority, &descriptor)
            .unwrap();
        let event = match coordinator
            .prepare_acknowledged_generation_reassignments(&authority)
            .unwrap()
        {
            GenerationReassignmentProgress::DeliveryPending(batch) => {
                assert_eq!(batch.events().len(), 1);
                let event = batch.events()[0];
                let closure = event.reward.unwrap().envelope("cutoff-fixture").unwrap();
                let owned = crate::engine::coordinator::own_pending_generation_assignments(
                    batch,
                    "cutoff-fixture",
                )
                .unwrap();
                let bridged = &owned[0];
                assert_eq!(
                    bridged.reward_json.as_deref().unwrap(),
                    serde_json::to_string(&closure).unwrap()
                );
                assert_eq!(
                    &*bridged.final_sensors,
                    batch.final_observation(0).unwrap_or(&[])
                );
                assert_eq!(closure.snake_id, old_frame_id);
                assert_eq!(closure.lease_id, initial.lease_id);
                assert_eq!(closure.physical_snake_id, initial.physical_snake_id);
                assert_eq!(
                    closure.previous_observation_sequence,
                    Some(initial.observation_sequence.clone())
                );
                assert_eq!(closure.from_step, initial.to_step);
                assert_eq!(closure.to_step, "0000000000000002");
                assert!((closure.deltas.elapsed_seconds - 2.0 / 60.0).abs() < 1e-12);
                assert_ne!(closure.lease_id, format!("{:016x}", event.lease_id));
                if death_case != 0 {
                    assert_eq!(
                        closure.transition_kind,
                        ExternalRewardTransitionKind::Terminal
                    );
                    assert_eq!(closure.reason, Some(ExternalRewardEndReason::Death));
                    assert!(batch.final_observation(0).is_none());
                    let expected_live_time = if death_case == 1 { 0.0 } else { 1.0 / 60.0 };
                    assert!(
                        (closure.deltas.alive_seconds_delta - expected_live_time).abs() < 1e-12
                    );
                } else {
                    assert_eq!(
                        closure.transition_kind,
                        ExternalRewardTransitionKind::Truncated
                    );
                    assert_eq!(
                        closure.reason,
                        Some(ExternalRewardEndReason::ManualGeneration)
                    );
                    assert!((closure.deltas.alive_seconds_delta - 1.0 / 60.0).abs() < 1e-12);
                    let final_state = batch
                        .final_observation(0)
                        .expect("live cutoff requires bootstrap sensors");
                    assert_eq!(final_state.len(), initial_observation.len());
                    assert!(final_state.iter().all(|value| value.is_finite()));
                    assert_ne!(final_state, initial_observation);
                }
                assert!(batch.observation(0).is_none());
                event
            }
            _ => panic!("generation closure must precede assignment publication"),
        };
        // Retrying a held assignment is byte/marker stable and leaves authority unchanged.
        let retry = match coordinator
            .prepare_acknowledged_generation_reassignments(&authority)
            .unwrap()
        {
            GenerationReassignmentProgress::DeliveryPending(batch) => batch.events()[0],
            _ => panic!("unacknowledged assignment must remain pending"),
        };
        assert_eq!(retry, event);
        assert_eq!(authority.state(), &source);
        coordinator
            .submit_external_delivery_results(
                &mut authority,
                &[ExternalDeliveryResult {
                    step_key: event.step_key,
                    event_sequence: event.event_sequence,
                    connection_id: event.connection_id,
                    lease_id: event.lease_id,
                    accepted: true,
                }],
            )
            .unwrap();
        coordinator
            .publish_acknowledged_generation_start(&mut authority)
            .unwrap();
        let successor = authority
            .state()
            .world
            .controller_leases
            .iter()
            .find(|lease| lease.id == event.lease_id)
            .unwrap();
        assert!(!successor.reward.unwrap().has_delivery());
    }
}

#[test]
fn durable_generation_boundary_retains_disconnected_token_without_hidden_reassignment() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint {
            x: 1_200.0,
            y: -1_200.0,
        },
    );
    let source_snake_id = candidate.world.controller_leases[0].snake_id;
    let source_token = candidate.world.controller_leases[0].resume_token.clone();
    let source_scope = candidate.world.controller_leases[0].scope.clone();
    let external_snake = candidate
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == source_snake_id)
        .expect("external fixture snake must exist");
    external_snake.turn = 0.0;
    external_snake.input_boost = false;
    let lease = &mut candidate.world.controller_leases[0];
    lease.connection_id = None;
    lease.status = ControllerLeaseStatus::ReservedNeutral;
    lease.last_observed_at_ms = 200;
    lease.disconnected_at_ms = Some(200);
    lease.input_hold_expires_at_ms = Some(600);
    lease.grace_expires_at_ms = Some(30_200);
    set_complete_setting(
        &mut candidate,
        "generationSeconds",
        NormalizedSettingValue::Float(8.0),
    );
    set_complete_setting(
        &mut candidate,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    );
    candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("terminal disconnected fixture must construct the coordinator");
    let transition = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 1_000,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("terminal disconnected fixture must stage a generation boundary")
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected pending generation transition, got {other:?}"),
    };
    let source_key = transition.source_key();
    let directory = GenerationHandoffDirectory::new();
    let descriptor = coordinator
        .publish_pending_generation_checkpoint(
            &authority,
            directory.path(),
            CheckpointOperationId::parse("00000000000000000000000000000092").unwrap(),
            &generation_handoff_checkpoint_limits(),
            &default_graph_limits(),
        )
        .expect("generation checkpoint file must publish");
    coordinator
        .acknowledge_pending_generation_persistence(&authority, &descriptor)
        .expect("exact metadata acknowledgement must construct the generation base");

    assert!(matches!(
        coordinator
            .prepare_acknowledged_generation_reassignments(&authority)
            .expect("disconnected owner needs no assignment delivery"),
        GenerationReassignmentProgress::Ready(ready)
            if ready.source_key() == source_key
    ));
    let unavailable = coordinator
        .pending_unavailable_controller_reservations()
        .expect("disconnected token outcome must remain inspectable before publication");
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].source_lease_id, 1);
    assert_eq!(unavailable[0].source_snake_id, source_snake_id);
    assert_eq!(unavailable[0].controller_kind, ControllerKind::Player);
    assert_eq!(unavailable[0].scope, source_scope);
    assert_eq!(unavailable[0].resume_token, source_token);
    assert_eq!(unavailable[0].disconnected_at_ms, Some(200));
    assert_eq!(unavailable[0].grace_expires_at_ms, Some(30_200));
    assert_eq!(
        unavailable[0].reason,
        crate::engine::external_replacement::UnavailableControllerReason::SnakeUnavailable
    );
    let expected_unavailable = unavailable[0].clone();
    assert_eq!(authority.state(), &source);

    let publication = coordinator
        .publish_acknowledged_generation_start(&mut authority)
        .expect("durable disconnected generation must publish without a hidden snake");
    assert_eq!(publication.source_key, source_key);
    assert_eq!(publication.external_assignments, 0);
    assert_eq!(
        publication.unavailable_controller_reservations,
        vec![expected_unavailable]
    );
    assert!(authority.state().world.controller_leases.is_empty());
    assert!(authority
        .state()
        .world
        .snakes
        .iter()
        .all(|snake| snake.kind != SnakeKind::External));
    assert!(coordinator.pending_generation_transition().is_none());
    RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
        .expect("the published successor must admit a fresh running coordinator");
}

#[test]
fn generation_guards_refuse_nonterminal_publication_without_mutation() {
    let graph = default_graph();
    let cases = [
        (
            GenerationTransitionReason::Duration,
            604_800.0,
            50.0,
            604_800.0 - (1.0 / 120.0),
        ),
        (
            GenerationTransitionReason::Manual,
            8.0,
            8.0,
            8.0 - (1.0 / 120.0),
        ),
        (GenerationTransitionReason::Manual, 240.0, 8.0, 0.0),
        (
            GenerationTransitionReason::EarlyAliveCount,
            240.0,
            8.0,
            8.0 - (1.0 / 120.0),
        ),
        (
            GenerationTransitionReason::Duration,
            8.0,
            50.0,
            8.0 - (1.0 / 120.0),
        ),
    ];

    for (expected_reason, generation_seconds, early_seconds, elapsed_seconds) in cases {
        let mut candidate = complete_running_candidate(&graph);
        set_complete_setting(
            &mut candidate,
            "generationSeconds",
            NormalizedSettingValue::Float(generation_seconds),
        );
        set_complete_setting(
            &mut candidate,
            "observer.earlyEndMinSeconds",
            NormalizedSettingValue::Float(early_seconds),
        );
        candidate.generation.elapsed_seconds = elapsed_seconds;
        refresh_config_hash(&mut candidate);
        let mut authority = own_complete_running(candidate, Arc::clone(&graph));
        let source = authority.state().clone();
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .expect("terminal fixture must construct the coordinator");
        if expected_reason == GenerationTransitionReason::Manual {
            coordinator.request_manual_finish();
        }

        let transition = match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 200,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .expect("terminal guard must stage a generation transition")
        {
            RunningStepProgress::GenerationTransitionPending(transition) => transition,
            other => panic!("expected pending generation transition, got {other:?}"),
        };
        assert_eq!(transition.reason(), expected_reason);
        let completion = transition
            .commit_record()
            .completion
            .expect("terminal chronology");
        assert_eq!(completion.reason, expected_reason);
        assert_eq!(
            f64::from_bits(completion.actual_elapsed_f64_bits),
            elapsed_seconds + 1.0 / 60.0
        );
        assert_eq!(
            f64::from_bits(completion.configured_duration_f64_bits),
            generation_seconds
        );
        assert_eq!(transition.alive_evolved(), 1);
        assert_eq!(authority.state(), &source);
    }
}

#[test]
fn terminal_guard_runs_before_old_generation_external_replacement() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    push_connected_external_fixture(
        &mut candidate,
        &graph,
        EXTERNAL_ENTITY_ID_START,
        2,
        1,
        7,
        WorldPoint { x: 3_490.5, y: 0.0 },
    );
    set_complete_setting(
        &mut candidate,
        "generationSeconds",
        NormalizedSettingValue::Float(8.0),
    );
    set_complete_setting(
        &mut candidate,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    );
    candidate.generation.elapsed_seconds = 8.0 - (1.0 / 120.0);
    candidate.allocators.next_external_id = BASELINE_ENTITY_ID_START;
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("terminal controlled-death fixture must construct the coordinator");

    let transition = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("terminal wall death must stage a generation transition")
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected pending generation transition, got {other:?}"),
    };
    assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
    assert_eq!(authority.state(), &source);
    assert!(matches!(
        coordinator
            .submit_external_delivery_results(&mut authority, &[])
            .expect("terminal attempt must expose no replacement assignment")
            .state,
        ExternalDeliveryState::Idle
    ));
}
