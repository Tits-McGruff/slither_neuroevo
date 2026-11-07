use super::*;

#[test]
fn complete_control_commit_updates_internal_state_and_retains_external_delivery() {
    let plan = graph_plan();
    let fixture = fixture(&plan);
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_allocators = fixture.allocators.clone();
    let source_lifecycle = fixture.lifecycle.clone();
    let source_brains = fixture.brains.clone();
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(20),
            world: &fixture.world,
            rng: &fixture.rng,
            allocators: &fixture.allocators,
            generation_elapsed_seconds: 2.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &fixture.lifecycle,
            config: prefix_config(),
        })
        .unwrap();
    let prefix_world = prefix.world().clone();
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(prefix.world()).unwrap();
    let mut selection = control_workspace(plan);
    let phase = selection
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
    let baseline = phase.baseline_controls()[0].clone();
    let expected_external = phase.external_observation(0).unwrap().1.to_vec();
    let expected_external_marker = phase.external_events()[0].delivery();
    let mut commit_workspace = ControlCommitWorkspace::new();
    let committed = commit_workspace.prepare(phase).unwrap();

    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
    assert_eq!(fixture.brains, source_brains);
    assert_eq!(committed.key(), key(20));
    assert_eq!(committed.calculation_key().step(), 41);
    assert_eq!(committed.generation_elapsed_seconds(), 2.0 + DT);
    assert_eq!(committed.ambient_accumulator(), 0.0);
    assert_eq!(committed.allocators(), prefix.allocators());
    assert_eq!(committed.sensor_generation(), generation);
    assert_eq!(committed.external_events().len(), 1);
    assert_eq!(
        committed.external_observation(0).unwrap().1,
        expected_external
    );

    let baseline_snake = committed
        .world()
        .snakes
        .iter()
        .find(|snake| snake.id == 20)
        .unwrap();
    assert_eq!(
        baseline_snake.baseline_strategy,
        Some(baseline.next_strategy())
    );
    assert_eq!(
        baseline_snake.turn.to_bits(),
        baseline.next_slot().turn.to_bits()
    );
    assert_eq!(baseline_snake.input_boost, baseline.next_slot().boost);
    assert_eq!(
        baseline_snake.delivered_observation_points.to_bits(),
        baseline.delivery().sampled_points.to_bits()
    );
    assert_eq!(
        committed.baseline_lifecycle().slots[0],
        baseline.next_slot()
    );
    assert_eq!(committed.rng().baselines[0], *baseline.next_rng());

    let external = committed
        .world()
        .snakes
        .iter()
        .find(|snake| snake.id == 30)
        .unwrap();
    assert_eq!(external.turn, 0.75);
    assert!(external.input_boost);
    assert_eq!(
        external.delivered_observation_points.to_bits(),
        expected_external_marker.previous_delivered_points.to_bits()
    );
    let connected_lease = committed
        .world()
        .controller_leases
        .iter()
        .find(|lease| lease.snake_id == 30)
        .unwrap();
    assert_eq!(connected_lease.last_observed_at_ms, 31_000);

    let takeover = committed
        .world()
        .snakes
        .iter()
        .find(|snake| snake.id == 40)
        .unwrap();
    let takeover_lease = committed
        .world()
        .controller_leases
        .iter()
        .find(|lease| lease.snake_id == 40)
        .unwrap();
    assert_eq!(takeover_lease.status, ControllerLeaseStatus::NeuralTakeover);
    assert_eq!(takeover_lease.takeover_committed_at_ms, Some(31_000));
    assert_ne!((takeover.turn.to_bits(), takeover.input_boost), (0, false));

    let evolved_before = prefix_world
        .snakes
        .iter()
        .find(|snake| snake.id == 10)
        .unwrap();
    let evolved_after = committed
        .world()
        .snakes
        .iter()
        .find(|snake| snake.id == 10)
        .unwrap();
    assert_eq!(
        evolved_after.delivered_observation_points.to_bits(),
        evolved_after.points.to_bits()
    );
    assert_ne!(
        evolved_before.delivered_observation_points.to_bits(),
        evolved_after.delivered_observation_points.to_bits()
    );
    let evolved_handle = evolved_after.brain.unwrap();
    let recurrent_before = fixture
        .brains
        .iter()
        .find(|brain| brain.handle == evolved_handle)
        .unwrap();
    let recurrent_after = committed
        .brains()
        .iter()
        .find(|brain| brain.handle == evolved_handle)
        .unwrap();
    assert_ne!(recurrent_before.recurrent, recurrent_after.recurrent);
}

#[test]
fn complete_control_boundary_advances_through_every_physics_substep() {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    make_fixture_bodies_physics_ready(&mut fixture.world);
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_allocators = fixture.allocators.clone();
    let source_lifecycle = fixture.lifecycle.clone();
    let source_brains = fixture.brains.clone();

    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(28),
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
    let mut selection = control_workspace(plan);
    let phase = selection
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
    let mut commit_workspace = ControlCommitWorkspace::new();
    let committed = commit_workspace.prepare(phase).unwrap();
    let committed_leases = committed.world().controller_leases.clone();
    let committed_lifecycle = committed.baseline_lifecycle().clone();
    let external_observation = committed.external_observation(0).unwrap().1.to_vec();

    let mut step_workspace = WorldStepWorkspace::new();
    let prepared = step_workspace
        .prepare(committed, world_step_config())
        .unwrap();

    assert_eq!(prepared.key(), key(28));
    assert_eq!(prepared.config(), world_step_config());
    assert_eq!(prepared.calculation_key().step(), 41);
    assert_eq!(prepared.world().controller_leases, committed_leases);
    assert_eq!(prepared.baseline_lifecycle(), &committed_lifecycle);
    assert_eq!(prepared.brains(), committed.brains());
    assert_eq!(
        prepared.external_observation(0).unwrap().1,
        external_observation
    );
    assert_eq!(prepared.generation_elapsed_seconds(), 2.0 + DT);
    assert_eq!(prepared.ambient_accumulator(), 0.0);
    assert!(prepared
        .world()
        .snakes
        .iter()
        .zip(committed.world().snakes.iter())
        .any(|(after, before)| after.position != before.position));
    assert!(
        prepared.sensor_generation().best_points_this_generation()
            >= generation.best_points_this_generation()
    );
    let diagnostics = prepared.diagnostics();
    assert_eq!(diagnostics.physics.expected_substeps, 3);
    assert_eq!(diagnostics.physics.completed_substeps, 3);
    assert!(diagnostics.physics.controller_lease_capacity >= 3);
    assert_eq!(diagnostics.control.selection.controls, 6);

    let warmed = step_workspace
        .prepare(committed, world_step_config())
        .unwrap()
        .diagnostics();
    for _ in 0..8 {
        assert_eq!(
            step_workspace
                .prepare(committed, world_step_config())
                .unwrap()
                .diagnostics(),
            warmed
        );
    }

    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
    assert_eq!(fixture.brains, source_brains);
}

#[test]
fn committed_takeover_holds_neural_action_without_reapplying_external_neutral() {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    let takeover_snake = fixture
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == 40)
        .unwrap();
    takeover_snake.turn = 0.4;
    takeover_snake.previous_turn = -0.3;
    takeover_snake.input_boost = true;
    takeover_snake.previous_input_boost = false;
    takeover_snake.control_accumulator_seconds = 0.0;
    let takeover_lease = fixture
        .world
        .controller_leases
        .iter_mut()
        .find(|lease| lease.snake_id == 40)
        .unwrap();
    takeover_lease.status = ControllerLeaseStatus::NeuralTakeover;
    takeover_lease.takeover_committed_at_ms = Some(31_000);
    takeover_lease.last_observed_at_ms = 31_000;
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(21),
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
    let mut config = control_config();
    config.neural_control_interval_seconds = 0.05;
    let mut selection = control_workspace(plan);
    let phase = selection
        .prepare(ControlPhaseInputs {
            prefix,
            generation: &generation,
            assessment_points: None,
            v4: None,
            population: &fixture.population,
            brains: &fixture.brains,
            wall_now_ms: 31_001,
            config,
        })
        .unwrap();
    let update = phase
        .control_updates()
        .iter()
        .find(|update| update.snake_id() == 40)
        .unwrap();
    assert_eq!(update.source(), SelectedControlSource::NeuralHeld);
    let mut commit_workspace = ControlCommitWorkspace::new();
    let committed = commit_workspace.prepare(phase).unwrap();
    let takeover = committed
        .world()
        .snakes
        .iter()
        .find(|snake| snake.id == 40)
        .unwrap();
    assert_eq!(takeover.turn.to_bits(), 0.4_f32.to_bits());
    assert_eq!(takeover.previous_turn.to_bits(), (-0.3_f32).to_bits());
    assert!(takeover.input_boost);
    assert!(!takeover.previous_input_boost);
}

#[test]
fn malformed_control_commit_is_rejected_without_touching_sources() {
    let plan = graph_plan();
    let fixture = fixture(&plan);
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_allocators = fixture.allocators.clone();
    let source_lifecycle = fixture.lifecycle.clone();
    let source_brains = fixture.brains.clone();
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(22),
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
    let mut selection = control_workspace(plan);
    let mut phase = selection
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
    let mut malformed = phase.control_updates().to_vec();
    malformed[0].snake_id = 999;
    phase.controls = &malformed;
    let mut commit_workspace = ControlCommitWorkspace::new();
    assert!(matches!(
        commit_workspace.prepare(phase),
        Err(ControlPhaseError::CommitShapeMismatch {
            field: "control snake identity"
        })
    ));
    assert!(!commit_workspace.is_ready());
    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
    assert_eq!(fixture.brains, source_brains);
}

#[test]
fn external_observation_capacity_failure_leaves_every_source_unchanged_and_unready() {
    let plan = graph_plan();
    let fixture = fixture(&plan);
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_allocators = fixture.allocators.clone();
    let source_lifecycle = fixture.lifecycle.clone();
    let source_brains = fixture.brains.clone();
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(2),
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
    let mut config = control_config();
    config.maximum_external_observations = 0;
    let mut workspace = control_workspace(plan);
    assert!(matches!(
        workspace.prepare(ControlPhaseInputs {
            prefix,
            generation: &generation,
            assessment_points: None,
            v4: None,
            population: &fixture.population,
            brains: &fixture.brains,
            wall_now_ms: 31_000,
            config,
        }),
        Err(ControlPhaseError::CapacityExceeded {
            buffer: "external observations",
            required: 1,
            maximum: 0,
        })
    ));
    assert!(!workspace.is_ready());
    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
    assert_eq!(fixture.brains, source_brains);
}

#[test]
fn warmed_shared_boundary_reuses_every_reported_capacity() {
    let plan = graph_plan();
    let fixture = fixture(&plan);
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(3),
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
    let mut expected = None;
    for _ in 0..24 {
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
        let diagnostics = prepared.diagnostics();
        if let Some(expected) = expected {
            assert_eq!(diagnostics, expected);
        } else {
            expected = Some(diagnostics);
        }
    }
}

#[test]
fn warmed_control_commit_reuses_every_reported_capacity() {
    let plan = graph_plan();
    let fixture = fixture(&plan);
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(23),
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
    let mut selection = control_workspace(plan);
    let mut commit = ControlCommitWorkspace::new();
    let warm_phase = selection
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
    let warm = commit.prepare(warm_phase).unwrap().diagnostics();
    assert!(warm.brain_weight_values_copied > 0);
    let mut expected = None;
    for _ in 0..24 {
        let phase = selection
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
        let prepared = commit.prepare(phase).unwrap();
        let diagnostics = prepared.diagnostics();
        assert!(diagnostics.rng_text_capacity > 0);
        assert!(diagnostics.brain_capacity >= fixture.brains.len());
        assert_eq!(diagnostics.brain_weight_values_copied, 0);
        assert!(diagnostics.brain_recurrent_values_copied > 0);
        assert!(diagnostics.external_event_capacity >= 1);
        assert!(diagnostics.external_observation_capacity >= 51);
        if let Some(expected) = expected {
            assert_eq!(diagnostics, expected);
        } else {
            expected = Some(diagnostics);
        }
    }

    let mut replacement_brains = fixture.brains.clone();
    replacement_brains
        .iter_mut()
        .find(|brain| brain.owner == BrainOwner::Entity(40))
        .unwrap()
        .non_population_weights
        .as_mut()
        .unwrap()
        .fill(0.875);
    let replacement_key = PhysicsStepKey::new(8, 3, 40, EPOCH, 9, [0x5a; 32], 24);
    let replacement_prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: replacement_key,
            world: &fixture.world,
            rng: &fixture.rng,
            allocators: &fixture.allocators,
            generation_elapsed_seconds: 2.0,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &fixture.lifecycle,
            config: prefix_config(),
        })
        .unwrap();
    let replacement_phase = selection
        .prepare(ControlPhaseInputs {
            prefix: replacement_prefix,
            generation: &generation,
            assessment_points: None,
            v4: None,
            population: &fixture.population,
            brains: &replacement_brains,
            wall_now_ms: 31_000,
            config: control_config(),
        })
        .unwrap();
    let replacement = commit.prepare(replacement_phase).unwrap();
    assert!(replacement.diagnostics().brain_weight_values_copied > 0);
    assert!(replacement
        .brains()
        .iter()
        .find(|brain| brain.owner == BrainOwner::Entity(40))
        .unwrap()
        .non_population_weights
        .as_ref()
        .unwrap()
        .iter()
        .all(|value| value.to_bits() == 0.875_f32.to_bits()));
}
