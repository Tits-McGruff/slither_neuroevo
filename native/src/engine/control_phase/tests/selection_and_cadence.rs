use super::*;

#[test]
fn low_level_world_step_requires_replacement_context_for_controlled_death() {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    let evolved_position = fixture
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == 10)
        .unwrap()
        .position;
    let controlled = fixture
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == 30)
        .unwrap();
    controlled.position = evolved_position;
    controlled.previous_position = evolved_position;
    make_fixture_bodies_physics_ready(&mut fixture.world);
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_allocators = fixture.allocators.clone();
    let source_lifecycle = fixture.lifecycle.clone();
    let source_brains = fixture.brains.clone();

    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(30),
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
    let committed_world = committed.world().clone();
    let committed_rng = committed.rng().clone();
    let committed_allocators = committed.allocators().clone();
    let committed_lifecycle = committed.baseline_lifecycle().clone();
    let committed_brains = committed.brains().to_vec();
    let mut step_workspace = WorldStepWorkspace::new();

    let mut mismatched_prefix = world_step_config();
    mismatched_prefix.prefix.maximum_pellets -= 1;
    let mut mismatched_config = world_step_config();
    mismatched_config.control.maximum_snakes += 1;
    for (config, expected_field) in [
        (mismatched_prefix, "prefix"),
        (mismatched_config, "control"),
    ] {
        match step_workspace.prepare(committed, config) {
            Err(WorldStepError::ControlConfigMismatch { field }) => {
                assert_eq!(field, expected_field);
            }
            other => panic!("expected {expected_field} mismatch, got {other:?}"),
        }
        assert!(!step_workspace.is_ready());
    }

    let mut wrong_version = world_step_config();
    wrong_version.algorithm_version += 1;
    let mut mismatched_baseline = world_step_config();
    mismatched_baseline.baseline.respawn_delay_seconds += 1.0;
    let mut mismatched_pellet_limit = world_step_config();
    mismatched_pellet_limit.physics.maximum_pellets -= 1;
    let mut mismatched_world_radius = world_step_config();
    mismatched_world_radius.physics.movement.world_radius -= 1.0;
    let mut zero_substeps = world_step_config();
    zero_substeps.physics_substeps = 0;
    let mut excessive_substeps = world_step_config();
    excessive_substeps.physics_substeps = MAXIMUM_PHYSICS_SUBSTEPS + 1;
    let mut mismatched_substep_sum = world_step_config();
    mismatched_substep_sum.physics.substep_dt *= 0.5;
    for (config, expected_field) in [
        (wrong_version, "algorithm_version"),
        (mismatched_baseline, "baseline lifecycle"),
        (mismatched_pellet_limit, "maximum pellets"),
        (mismatched_world_radius, "world radius"),
        (zero_substeps, "physics substeps"),
        (excessive_substeps, "physics substeps"),
        (mismatched_substep_sum, "physics substep sum"),
    ] {
        match step_workspace.prepare(committed, config) {
            Err(WorldStepError::InvalidConfig { field }) => {
                assert_eq!(field, expected_field);
            }
            other => panic!("expected invalid {expected_field}, got {other:?}"),
        }
        assert!(!step_workspace.is_ready());
    }
    let error = step_workspace
        .prepare(committed, world_step_config())
        .expect_err("low-level caller omitted the replacement graph and wall boundary");

    assert!(matches!(
        error,
        WorldStepError::ExternalReplacementContextRequired
    ));
    assert!(!step_workspace.is_ready());
    assert_eq!(committed.world(), &committed_world);
    assert_eq!(committed.rng(), &committed_rng);
    assert_eq!(committed.allocators(), &committed_allocators);
    assert_eq!(committed.baseline_lifecycle(), &committed_lifecycle);
    assert_eq!(committed.brains(), committed_brains);
    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
    assert_eq!(fixture.brains, source_brains);
}

#[test]
fn controller_results_are_independent_of_world_lease_and_brain_storage_order() {
    assert_eq!(prepared_summary(false), prepared_summary(true));
}

#[test]
fn joined_takeover_resets_only_the_expired_controller_recurrent_state() {
    let original_plan = graph_plan();
    let original_fixture = fixture(&original_plan);
    let original = prepared_summary_for(original_plan, &original_fixture);

    let takeover_plan = graph_plan();
    let mut changed_takeover = fixture(&takeover_plan);
    changed_takeover
        .brains
        .iter_mut()
        .find(|brain| brain.owner == BrainOwner::Entity(40))
        .unwrap()
        .recurrent
        .fill(9.0);
    let changed_takeover = prepared_summary_for(takeover_plan, &changed_takeover);
    assert_eq!(
        summarized_control(&original, 40),
        summarized_control(&changed_takeover, 40)
    );

    let evolved_plan = graph_plan();
    let mut changed_evolved = fixture(&evolved_plan);
    changed_evolved
        .brains
        .iter_mut()
        .find(|brain| brain.owner == BrainOwner::PopulationSlot(0))
        .unwrap()
        .recurrent
        .fill(9.0);
    let changed_evolved = prepared_summary_for(evolved_plan, &changed_evolved);
    assert_ne!(
        summarized_control(&original, 10).2,
        summarized_control(&changed_evolved, 10).2
    );
}

#[test]
fn cadence_and_takeover_force_have_exact_distinct_boundaries() {
    let mut invalid_config = control_config();
    invalid_config.neural_control_interval_seconds = 0.061;
    assert!(matches!(
        invalid_config.validate(),
        Err(ControlPhaseError::InvalidConfig {
            field: "neural_control_interval_seconds"
        })
    ));
    assert_eq!(
        next_neural_boundary(0.0, DT, 0.05, false).unwrap(),
        (false, DT)
    );
    let (due, remainder) = next_neural_boundary(0.04, DT, 0.05, false).unwrap();
    assert!(due);
    assert!((remainder - (0.04 + DT - 0.05)).abs() < 1.0e-12);
    let (due, takeover_remainder) = next_neural_boundary(0.04, DT, 0.05, true).unwrap();
    assert!(due);
    assert!((takeover_remainder - DT).abs() < 1.0e-12);
    assert!(matches!(
        next_neural_boundary(f64::INFINITY, DT, 0.05, false),
        Err(ControlPhaseError::InvalidCadenceState(value)) if value.is_infinite()
    ));
}

#[test]
fn newly_created_neural_snake_stays_due_after_live_control_interval_increase() {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    let mut creation_config = control_config();
    creation_config.neural_control_interval_seconds = 0.008;
    let initial_accumulator = creation_config.initial_neural_accumulator_seconds();
    let mut config = creation_config;
    config.neural_control_interval_seconds = 0.06;
    let evolved = fixture
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == 10)
        .unwrap();
    evolved.control_accumulator_seconds = initial_accumulator;

    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(11),
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
            config,
        })
        .unwrap();
    let update = prepared
        .control_updates()
        .iter()
        .find(|update| update.snake_id() == 10)
        .unwrap();

    assert_eq!(update.source(), SelectedControlSource::NeuralEvaluated);
    assert!((update.next_control_accumulator_seconds() - DT).abs() < 1.0e-12);
}
