use super::*;

#[test]
fn authoritative_runtime_owns_the_validated_state_before_start() {
    let graph = default_graph();
    let state = own(candidate(&graph, 2), graph, 32 * 1024 * 1024).unwrap();
    let expected_run_id = state.state().identity.run_id.clone();
    let expected_generation = state.state().generation.generation;
    let expected_memory_bytes = state.memory_estimate().total_bytes;
    let runtime =
        EngineRuntime::new_authoritative(runtime_init(), state, Arc::new(NoopWakeSink)).unwrap();

    assert!(runtime.owns_authoritative_state());
    assert_eq!(
        runtime.authoritative_state_memory_bytes(),
        Some(expected_memory_bytes)
    );
    let retained = runtime
        .authoritative_state_for_test()
        .expect("authoritative runtime retains exactly one validated state");
    assert_eq!(retained.state().identity.run_id, expected_run_id);
    assert_eq!(retained.state().generation.generation, expected_generation);
}

#[test]
fn complete_nonterminal_coordinator_publishes_every_step_continuation_once() {
    let graph = default_graph();
    let candidate = complete_running_candidate(&graph);
    let source = candidate.clone();
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("complete admitted config must construct the coordinator");

    let outcome = match coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.125,
            },
        )
        .expect("ordinary nonterminal step must publish")
    {
        RunningStepProgress::Published(outcome) => outcome,
        RunningStepProgress::ExternalDeliveryPending(_) => {
            panic!("ordinary neural step must not require external delivery")
        }
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("ordinary neural step must not end the generation")
        }
    };

    assert_eq!(outcome.publication.completed_step, 1);
    assert_eq!(outcome.diagnostics.physics.expected_substeps, 3);
    assert_eq!(outcome.diagnostics.physics.completed_substeps, 3);
    assert_eq!(coordinator.last_wall_now_ms(), Some(100));
    assert_eq!(authority.state().generation.completed_step, 1);
    assert_eq!(
        authority.state().generation.elapsed_seconds.to_bits(),
        (1.0_f64 / 60.0).to_bits()
    );
    assert_eq!(
        authority
            .state()
            .generation
            .wall_accumulator_seconds
            .to_bits(),
        0.125_f64.to_bits()
    );
    assert_ne!(authority.state().world, source.world);
    assert_ne!(authority.state().rng, source.rng);
    assert_ne!(authority.state().brains, source.brains);
    assert_eq!(authority.state().identity, source.identity);
    assert_eq!(authority.state().population, source.population);
    assert_eq!(authority.state().phase, AuthorityPhase::Running);
}

#[test]
fn scheduler_requires_a_fresh_command_service_boundary_before_each_overdue_step() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    set_complete_setting(
        &mut candidate,
        "simSpeed",
        NormalizedSettingValue::Float(12.0),
    );
    candidate.config.requested_sim_speed = 12.0;
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let mut running =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("12x fixture must construct the running coordinator");
    let mut scheduler =
        FixedStepScheduler::try_new(&authority, FixedStepSchedulerPolicy::provisional_defaults())
            .expect("running authority must construct the scheduler");

    scheduler
        .reset_wall_clock(&authority, 1_000)
        .expect("async initialization must reset the scheduler clock");
    let initialized = scheduler.diagnostics();
    for repeated_wall_ms in [2_000, 999] {
        assert_eq!(
            scheduler.reset_wall_clock(&authority, repeated_wall_ms),
            Err(SchedulerError::ClockAlreadyInitialized),
            "neither forward nor backward rebasing may hide or manufacture debt"
        );
        assert_eq!(
            scheduler.diagnostics(),
            initialized,
            "a rejected rebase must be atomic"
        );
    }
    let readiness = scheduler
        .service_after_command_drain(&authority, 1_017, SchedulerServiceMode::Interactive)
        .expect("one explicit service boundary must observe current wall debt");
    assert!(matches!(
        readiness,
        SchedulerReadiness::StepDue { due_steps: 12, .. }
    ));
    let step = scheduler
        .prepare_due_step(&authority)
        .expect("one serviced step must become available");
    assert_eq!(step.wall_now_ms(), 1_017);
    assert_eq!(step.service_mode(), SchedulerServiceMode::Interactive);
    let progress = running
        .advance_nonterminal(&mut authority, step.running_step_inputs())
        .expect("the scheduled nonterminal step must publish");
    let publication = match progress {
        RunningStepProgress::Published(outcome) => outcome.publication,
        RunningStepProgress::ExternalDeliveryPending(_) => {
            panic!("the neural-only fixture must not wait for Node delivery")
        }
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("the scheduled fixture must remain nonterminal")
        }
    };

    let key = publication.key;
    let mut different_hash = key.config_hash();
    different_hash[0] ^= 1;
    let forged_keys = [
        PhysicsStepKey::new(
            key.world_epoch().checked_add(1).unwrap(),
            key.generation(),
            key.source_completed_step(),
            key.population_epoch(),
            key.config_revision(),
            key.config_hash(),
            key.operation_epoch(),
        ),
        PhysicsStepKey::new(
            key.world_epoch(),
            key.generation().checked_add(1).unwrap(),
            key.source_completed_step(),
            key.population_epoch(),
            key.config_revision(),
            key.config_hash(),
            key.operation_epoch(),
        ),
        PhysicsStepKey::new(
            key.world_epoch(),
            key.generation(),
            key.source_completed_step().checked_add(1).unwrap(),
            key.population_epoch(),
            key.config_revision(),
            key.config_hash(),
            key.operation_epoch(),
        ),
        PhysicsStepKey::new(
            key.world_epoch(),
            key.generation(),
            key.source_completed_step(),
            key.population_epoch().checked_add(1).unwrap(),
            key.config_revision(),
            key.config_hash(),
            key.operation_epoch(),
        ),
        PhysicsStepKey::new(
            key.world_epoch(),
            key.generation(),
            key.source_completed_step(),
            key.population_epoch(),
            key.config_revision().checked_add(1).unwrap(),
            key.config_hash(),
            key.operation_epoch(),
        ),
        PhysicsStepKey::new(
            key.world_epoch(),
            key.generation(),
            key.source_completed_step(),
            key.population_epoch(),
            key.config_revision(),
            different_hash,
            key.operation_epoch(),
        ),
        PhysicsStepKey::new(
            key.world_epoch(),
            key.generation(),
            key.source_completed_step(),
            key.population_epoch(),
            key.config_revision(),
            key.config_hash(),
            key.operation_epoch().checked_add(1).unwrap(),
        ),
    ];
    let pending = scheduler.diagnostics();
    for forged_key in forged_keys {
        assert_eq!(
            scheduler.commit_step(
                &authority,
                step,
                RunningStepPublication {
                    key: forged_key,
                    ..publication
                },
            ),
            Err(SchedulerError::PublicationMismatch {
                field: "complete authority publication identity",
            })
        );
        assert_eq!(
            scheduler.diagnostics(),
            pending,
            "a forged key must not retire or mutate the pending ticket"
        );
    }
    let mut wrong_memory = publication.memory;
    wrong_memory.total_bytes = wrong_memory.total_bytes.checked_add(1).unwrap();
    assert_eq!(
        scheduler.commit_step(
            &authority,
            step,
            RunningStepPublication {
                memory: wrong_memory,
                ..publication
            },
        ),
        Err(SchedulerError::PublicationMismatch {
            field: "complete authority publication identity",
        })
    );
    assert_eq!(scheduler.diagnostics(), pending);

    scheduler
        .commit_step(&authority, step, publication)
        .expect("the exact publication must retire the scheduler ticket");

    let diagnostics = scheduler.diagnostics();
    assert_eq!(diagnostics.completed_steps, 1);
    assert_eq!(diagnostics.command_service_boundaries, 1);
    assert_eq!(diagnostics.interactive_service_boundaries, 1);
    assert!(diagnostics.pending_simulation_seconds > candidate_fixed_dt(&authority));
    assert_eq!(
        authority
            .state()
            .generation
            .wall_accumulator_seconds
            .to_bits(),
        step.wall_accumulator_after_step().to_bits()
    );
    assert_eq!(
        scheduler.prepare_due_step(&authority),
        Err(SchedulerError::CommandServiceRequired)
    );

    let second_readiness = scheduler
        .service_after_command_drain(&authority, 1_017, SchedulerServiceMode::Interactive)
        .expect("a second overdue step requires a second service boundary");
    assert!(matches!(
        second_readiness,
        SchedulerReadiness::StepDue { due_steps: 11, .. }
    ));
    let rejected = scheduler
        .prepare_due_step(&authority)
        .expect("the newly serviced second step must be available");
    let debt_before_rejection = scheduler.diagnostics().pending_simulation_seconds;
    scheduler
        .reject_step(&authority, rejected)
        .expect("a failed attempt must retain its scheduling debt");
    assert_eq!(
        scheduler.diagnostics().pending_simulation_seconds.to_bits(),
        debt_before_rejection.to_bits()
    );
    assert_eq!(authority.state().generation.completed_step, 1);
    assert_eq!(
        scheduler.prepare_due_step(&authority),
        Err(SchedulerError::CommandServiceRequired)
    );
}

#[test]
fn scheduler_holds_debt_while_external_delivery_blocks_publication() {
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
            x: 1_000.0,
            y: 1_000.0,
        },
    );
    let mut authority = own_complete_running(candidate, Arc::clone(&graph));
    let source = authority.state().clone();
    let mut running =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("external fixture must construct the running coordinator");
    let mut scheduler =
        FixedStepScheduler::try_new(&authority, FixedStepSchedulerPolicy::provisional_defaults())
            .expect("external fixture must construct the scheduler");
    scheduler.reset_wall_clock(&authority, 100).unwrap();
    assert!(matches!(
        scheduler
            .service_after_command_drain(&authority, 117, SchedulerServiceMode::Interactive,)
            .unwrap(),
        SchedulerReadiness::StepDue { due_steps: 1, .. }
    ));
    let step = scheduler.prepare_due_step(&authority).unwrap();
    let debt_before_publication = scheduler.diagnostics().pending_simulation_seconds;
    let event = match running
        .advance_nonterminal(&mut authority, step.running_step_inputs())
        .expect("complete external step must await a local send result")
    {
        RunningStepProgress::ExternalDeliveryPending(batch) => batch.events()[0],
        RunningStepProgress::Published(_) => panic!("external delivery must block the swap"),
        RunningStepProgress::GenerationTransitionPending(_) => {
            panic!("external delivery fixture must remain nonterminal")
        }
    };
    assert_eq!(authority.state(), &source);
    assert!(scheduler.diagnostics().step_pending);
    assert_eq!(
        scheduler.diagnostics().pending_simulation_seconds.to_bits(),
        debt_before_publication.to_bits()
    );
    assert_eq!(
        scheduler.service_after_command_drain(&authority, 118, SchedulerServiceMode::Interactive,),
        Err(SchedulerError::StepPending)
    );

    let accepted = ExternalDeliveryResult {
        step_key: event.step_key,
        event_sequence: event.event_sequence,
        connection_id: event.connection_id,
        lease_id: event.lease_id,
        accepted: true,
    };
    let publication = match running
        .submit_external_delivery_results(&mut authority, &[accepted])
        .expect("matching local result must publish")
        .state
    {
        ExternalDeliveryState::Published(outcome) => outcome.publication,
        _ => panic!("the sole resolved event must publish the step"),
    };
    scheduler
        .commit_step(&authority, step, publication)
        .expect("the delayed exact publication must consume one step of debt");
    assert!(!scheduler.diagnostics().step_pending);
    assert_eq!(scheduler.diagnostics().completed_steps, 1);
    assert_eq!(authority.state().generation.completed_step, 1);
    assert_eq!(
        authority
            .state()
            .generation
            .wall_accumulator_seconds
            .to_bits(),
        step.wall_accumulator_after_step().to_bits()
    );
}

#[test]
fn scheduler_holds_terminal_ticket_and_debt_until_explicit_transition_discard() {
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
    let mut running =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("terminal fixture must construct the running coordinator");
    let mut scheduler =
        FixedStepScheduler::try_new(&authority, FixedStepSchedulerPolicy::provisional_defaults())
            .expect("terminal fixture must construct the scheduler");
    scheduler.reset_wall_clock(&authority, 100).unwrap();
    scheduler
        .service_after_command_drain(&authority, 117, SchedulerServiceMode::Background)
        .unwrap();
    let step = scheduler.prepare_due_step(&authority).unwrap();
    let retained_debt = scheduler.diagnostics().pending_simulation_seconds;
    let transition = match running
        .advance_nonterminal(&mut authority, step.running_step_inputs())
        .expect("terminal result must stage its generation boundary")
    {
        RunningStepProgress::GenerationTransitionPending(transition) => transition,
        other => panic!("expected pending generation transition, got {other:?}"),
    };
    assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
    assert_eq!(transition.candidate().generation.generation, 2);
    let transition_source_key = transition.source_key();
    assert_eq!(authority.state(), &source);
    assert!(scheduler.diagnostics().step_pending);
    assert_eq!(
        scheduler.diagnostics().pending_simulation_seconds.to_bits(),
        retained_debt.to_bits()
    );
    assert_eq!(scheduler.diagnostics().completed_steps, 0);
    assert_eq!(
        scheduler.prepare_due_step(&authority),
        Err(SchedulerError::StepPending)
    );
    assert!(running
        .discard_pending_generation_transition(&authority, transition_source_key)
        .expect("matching explicit discard must succeed"));
    assert!(running.pending_generation_transition().is_none());
    scheduler
        .reject_step(&authority, step)
        .expect("discarded persistence handoff must reject its scheduler ticket");
    assert_eq!(
        scheduler.diagnostics().pending_simulation_seconds.to_bits(),
        retained_debt.to_bits()
    );
    assert_eq!(scheduler.diagnostics().completed_steps, 0);
    assert_eq!(
        scheduler.prepare_due_step(&authority),
        Err(SchedulerError::CommandServiceRequired)
    );
}

#[test]
fn warmed_coordinator_rejects_a_different_authority_with_matching_epochs_and_graph() {
    let graph = default_graph();
    let mut first = complete_running_candidate(&graph);
    let resurrected_id = RESURRECTED_ENTITY_ID_START;
    push_external_snake(
        &mut first,
        &graph,
        resurrected_id,
        2,
        WorldPoint {
            x: 1_000.0,
            y: -1_000.0,
        },
    );
    first.allocators.next_external_id = EXTERNAL_ENTITY_ID_START;
    first.allocators.next_resurrected_id = resurrected_id + 1;
    let resurrected_index = first.world.snakes.len() - 1;
    let body_start = first.world.snakes[resurrected_index].body.start;
    let head = first.world.snakes[resurrected_index].position;
    first
        .world
        .body_points
        .extend((1..5).map(|offset| WorldPoint {
            x: head.x - (offset as f64 * 7.5),
            y: head.y,
        }));
    let resurrected = &mut first.world.snakes[resurrected_index];
    resurrected.kind = SnakeKind::Resurrected;
    resurrected.body = BodyRange {
        start: body_start,
        len: 5,
    };
    resurrected.target_length = 5.0;
    resurrected.radius = 9.0;
    resurrected.speed = 165.0;

    first.brains[1].retained_origin = Some(Arc::new(RetainedNeuralOrigin::HallOfFame {
        entry_id: 1,
        weight_digest: crate::engine::retained_origin::packed_weight_digest(
            first.brains[1].non_population_weights.as_ref().unwrap(),
        ),
    }));
    let mut second = first.clone();
    second.brains[1]
        .non_population_weights
        .as_mut()
        .expect("resurrected brain must own weights")[0] = 0.875;
    second.brains[1].retained_origin = Some(Arc::new(RetainedNeuralOrigin::HallOfFame {
        entry_id: 1,
        weight_digest: crate::engine::retained_origin::packed_weight_digest(
            second.brains[1].non_population_weights.as_ref().unwrap(),
        ),
    }));
    assert_eq!(first.identity, second.identity);
    assert_eq!(first.config, second.config);
    assert_ne!(first.brains[1], second.brains[1]);

    let mut first_authority = own_complete_running(first, Arc::clone(&graph));
    let mut second_authority = own_complete_running(second, Arc::clone(&graph));
    assert_ne!(
        first_authority.world_epoch(),
        second_authority.world_epoch()
    );
    let second_source = second_authority.state().clone();
    let mut coordinator = RunningStepCoordinator::try_new(
        &first_authority,
        RunningStepWorkLimits::provisional_defaults(),
    )
    .expect("first authority must construct the coordinator");
    coordinator
        .advance_nonterminal(
            &mut first_authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        )
        .expect("first authority must warm every control cache");

    assert!(matches!(
        coordinator.advance_nonterminal(
            &mut second_authority,
            RunningStepInputs {
                wall_now_ms: 100,
                wall_accumulator_seconds: 0.0,
            },
        ),
        Err(RunningStepError::AuthorityMismatch {
            field: "world epoch",
        })
    ));
    assert_eq!(second_authority.state(), &second_source);
}

#[test]
fn manual_finish_scope_is_idempotent_and_rejects_stale_or_competing_requests() {
    use crate::engine::running_loop::{RunningAuthorityLoop, RunningAuthorityLoopProgress};
    let graph = default_graph();
    let candidate = complete_running_candidate(&graph);
    let authority = own_complete_running(candidate, Arc::clone(&graph));
    let run_id = authority.state().identity.run_id.clone();
    let generation = authority.state().generation.generation;
    let prepared = RunningAuthorityLoop::prepare(
        &authority,
        RunningStepWorkLimits::provisional_defaults(),
        FixedStepSchedulerPolicy::provisional_defaults(),
        0,
        &generation_handoff_checkpoint_limits(),
        &default_graph_limits(),
    )
    .unwrap();
    let mut running = RunningAuthorityLoop::from_prepared(authority, prepared);
    assert!(running
        .request_finish_generation("finish".into(), "another-run".into(), generation)
        .is_err());
    assert!(running
        .request_finish_generation("finish".into(), run_id.clone(), generation + 1)
        .is_err());
    let step = running
        .request_finish_generation("finish".into(), run_id.clone(), generation)
        .unwrap();
    assert_eq!(
        running
            .request_finish_generation("finish".into(), run_id.clone(), generation)
            .unwrap(),
        step
    );
    assert!(running
        .request_finish_generation("different".into(), run_id, generation)
        .is_err());
    assert!(matches!(
        running
            .service_after_command_drain(17, SchedulerServiceMode::Background, None)
            .unwrap(),
        RunningAuthorityLoopProgress::GenerationTransitionPending {
            reason: GenerationTransitionReason::Manual,
            ..
        }
    ));
}

#[test]
fn seven_day_accumulated_clock_and_actual_native_terminal_step_are_qualified() {
    let graph = default_graph();
    let mut candidate = complete_running_candidate(&graph);
    let dt = candidate.config.fixed_step_seconds;
    let steps = 36_288_000u64;
    let mut clock = 0.0f64;
    let mut maximum_error = 0.0f64;
    for step in 1..steps {
        clock += dt;
        maximum_error = maximum_error.max((clock - step as f64 * dt).abs());
    }
    assert!(
        maximum_error < 0.0002,
        "seven-day clock error {maximum_error}"
    );
    assert!(clock < 604_800.0);
    set_complete_setting(
        &mut candidate,
        "generationSeconds",
        NormalizedSettingValue::Float(604_800.0),
    );
    candidate.generation.elapsed_seconds = clock;
    candidate.generation.completed_step = steps - 1;
    refresh_config_hash(&mut candidate);
    let mut authority = own_complete_running(candidate, graph);
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    let result = coordinator
        .advance_nonterminal(
            &mut authority,
            RunningStepInputs {
                wall_now_ms: 200,
                wall_accumulator_seconds: 0.0,
            },
        )
        .unwrap();
    let RunningStepProgress::GenerationTransitionPending(transition) = result else {
        panic!("seven-day terminal step");
    };
    assert_eq!(transition.reason(), GenerationTransitionReason::Duration);
    assert_eq!(transition.candidate().generation.completed_step, steps);
    let elapsed = f64::from_bits(
        transition
            .commit_record()
            .completion
            .unwrap()
            .actual_elapsed_f64_bits,
    );
    assert!(elapsed >= 604_800.0 && elapsed - 604_800.0 < 0.0002);
}

#[test]
fn invalid_scheduler_accumulator_never_starts_or_changes_a_step() {
    let graph = default_graph();
    let candidate = complete_running_candidate(&graph);
    let mut authority = own_complete_running(candidate, graph);
    let source = authority.state().clone();
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .expect("complete admitted config must construct the coordinator");

    for value in [f64::NAN, f64::INFINITY, -f64::EPSILON] {
        assert!(matches!(
            coordinator.advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 100,
                    wall_accumulator_seconds: value,
                },
            ),
            Err(RunningStepError::InvalidSchedulerAccumulator(actual))
                if actual.to_bits() == value.to_bits()
        ));
        assert_eq!(authority.state(), &source);
        assert_eq!(coordinator.last_wall_now_ms(), None);
    }
}
