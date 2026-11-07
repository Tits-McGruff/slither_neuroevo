use super::*;

#[test]
fn reward_cutoff_probes_preserve_markers_and_agree_across_sensor_profiles() {
    use crate::contracts::ExternalRewardEndReason;
    use crate::engine::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState};
    use crate::engine::external_reward::{
        ExternalRewardLedger, RewardBoundary, RewardLifeIdentity,
    };
    use crate::engine::ray_work::RayWorkLimits;
    use crate::engine::sensor_contract::SensorProfile;

    let mut expected_reward = None;
    for profile in [
        SensorProfile::V3,
        SensorProfile::Basic,
        SensorProfile::Motion,
    ] {
        let (plan, mut workspace) = if profile == SensorProfile::V3 {
            let plan = graph_plan_for_width(83);
            let workspace = control_workspace(plan.clone());
            (plan, workspace)
        } else {
            v4_workspace(profile, 16, 1)
        };
        let mut fixture = fixture(&plan);
        let snake = fixture
            .world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == 30)
            .unwrap();
        let mut ledger = ExternalRewardLedger::new(
            RewardLifeIdentity {
                world_epoch: 7,
                lease_id: 1,
                physical_snake_id: snake.id,
                frame_snake_id: snake.frame_v1_id,
            },
            RewardBoundary {
                completed_step: 0,
                elapsed_seconds: 0.0,
            },
            snake.body.len,
            snake.points,
        )
        .unwrap();
        let initial = ledger
            .prepare(
                1,
                RewardBoundary {
                    completed_step: 0,
                    elapsed_seconds: 0.0,
                },
                None,
            )
            .unwrap();
        ledger.commit_delivery(&initial).unwrap();
        ledger.record_live_time(DT, 0.01).unwrap();
        snake.points += 0.01;
        snake.age_seconds += DT;
        let lease = fixture
            .world
            .controller_leases
            .iter_mut()
            .find(|lease| lease.snake_id == 30)
            .unwrap();
        lease.kind = ControllerKind::ReinforcementLearning;
        lease.reward = Some(ledger);
        let before = fixture.world.clone();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&fixture.world).unwrap();
        let lives = EvolvedGenerationState::default();
        let v4 = (profile != SensorProfile::V3).then_some(V4BoundaryContext {
            context: crate::engine::sensors_v4::V4ObservationContext {
                generation_elapsed_seconds: DT,
                evolved: &lives,
                experiment: EvolvedExperimentConfig::legacy(240.0),
            },
            limits: RayWorkLimits::new(1, 256, 767, 8).unwrap(),
        });
        let boundary = RewardBoundary {
            completed_step: 1,
            elapsed_seconds: DT,
        };
        let captured = workspace
            .capture_reward_cutoffs(&fixture.world, &generation, v4, control_config(), boundary)
            .unwrap();
        assert_eq!(captured.records.len(), 1);
        assert_eq!(captured.observations.len(), plan.input_size());
        assert!(captured.observations.iter().all(|value| value.is_finite()));
        let envelope = captured.records[0]
            .reward
            .prepare_closure(2, ExternalRewardEndReason::Generation)
            .unwrap()
            .envelope("profile-fixture")
            .unwrap();
        assert!((envelope.deltas.alive_seconds_delta - DT).abs() < 1e-12);
        if let Some(expected) = &expected_reward {
            assert_eq!(&envelope, expected);
        } else {
            expected_reward = Some(envelope);
        }
        let retried = workspace
            .capture_reward_cutoffs(&fixture.world, &generation, v4, control_config(), boundary)
            .unwrap();
        assert_eq!(retried.observations, captured.observations);
        assert_eq!(fixture.world, before);
        // Capacity errors are detected before a probe or delivery marker can commit.
        assert!(workspace
            .capture_reward_cutoffs(
                &fixture.world,
                &generation,
                v4,
                ControlPhaseConfig {
                    maximum_external_observations: 0,
                    ..control_config()
                },
                boundary
            )
            .is_err());
        assert_eq!(fixture.world, before);
    }
}

#[test]
fn baseline_v2_due_quota_commit_holds_and_late_failures_preserve_source() {
    use crate::engine::baseline_v2::{BaselineV2Mode, BaselineV2Settings, BaselineV2State};
    use crate::engine::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState};
    use crate::engine::ray_work::{RayObservationOwner, RayWorkLimits};
    use crate::engine::sensor_contract::SensorProfile;
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for mode in [BaselineV2Mode::Deterministic, BaselineV2Mode::Stochastic] {
            let (plan, mut selection) = v4_workspace(profile, 16, 1);
            selection.configure_teacher_capture(&[0]).unwrap();
            let (_, mut uncaptured_selection) = v4_workspace(profile, 16, 1);
            let mut fixture = fixture(&plan);
            make_fixture_bodies_physics_ready(&mut fixture.world);
            fixture.world.pellets.clear();
            fixture.rng.version = crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
            fixture.rng.baseline_behaviors =
                crate::engine::rng::derive_baseline_v2_behaviors(42, 1, 3, 1, false).unwrap();
            fixture.lifecycle.slots[0].v2_policy = Some(BaselineV2State::default());
            fixture.lifecycle.slots[0].wander_timer_seconds = 0.0;
            let baseline_id = fixture.lifecycle.slots[0].snake_id;
            let baseline = fixture
                .world
                .snakes
                .iter()
                .find(|snake| snake.id == baseline_id)
                .unwrap();
            selection
                .configure_inspector(
                    Some(crate::engine::inspector::InspectorTarget {
                        world_epoch: 7,
                        generation: 3,
                        physical_id: baseline_id,
                        frame_v1_id: baseline.frame_v1_id,
                    }),
                    crate::engine::inspector::MAX_INSPECTOR_BYTES,
                )
                .unwrap();
            let config = ControlPhaseConfig {
                baseline_v2: Some(BaselineV2Settings {
                    mode,
                    control_interval_seconds: 0.06,
                }),
                ..control_config()
            };
            let lives = EvolvedGenerationState::default();
            let mut commits = ControlCommitWorkspace::new();
            let mut delivered = Vec::new();
            for ordinal in 0..12u64 {
                let step_key =
                    PhysicsStepKey::new(7, 3, 40 + ordinal, EPOCH, 9, [0x5a; 32], 80 + ordinal);
                let source_world = fixture.world.clone();
                let source_rng = fixture.rng.clone();
                let source_lifecycle = fixture.lifecycle.clone();
                let mut prefix_workspace = FixedStepPrefixWorkspace::new();
                let prefix = prefix_workspace
                    .prepare(FixedStepPrefixInputs {
                        key: step_key,
                        world: &fixture.world,
                        rng: &fixture.rng,
                        allocators: &fixture.allocators,
                        generation_elapsed_seconds: 2.0 + ordinal as f64 * DT,
                        ambient_accumulator: 0.0,
                        baseline_lifecycle: &fixture.lifecycle,
                        config: prefix_config(),
                    })
                    .unwrap();
                let mut generation = SensorGenerationState::new();
                generation.update_after_step(prefix.world()).unwrap();
                let boundary = V4BoundaryContext {
                    context: crate::engine::sensors_v4::V4ObservationContext {
                        generation_elapsed_seconds: prefix.generation_elapsed_seconds(),
                        evolved: &lives,
                        experiment: EvolvedExperimentConfig::legacy(240.0),
                    },
                    limits: RayWorkLimits::new(1, 256, 767, 8).unwrap(),
                };
                if ordinal == 0 {
                    let mut malformed = fixture.population.clone();
                    malformed[0].weights[0] = f32::NAN;
                    assert!(selection
                        .prepare(ControlPhaseInputs {
                            prefix,
                            generation: &generation,
                            assessment_points: None,
                            v4: Some(boundary),
                            population: &malformed,
                            brains: &fixture.brains,
                            wall_now_ms: 31000,
                            config
                        })
                        .is_err());
                    assert!(!selection.is_ready());
                    assert_eq!(fixture.world, source_world);
                    assert_eq!(fixture.rng, source_rng);
                    assert_eq!(fixture.lifecycle, source_lifecycle);
                }
                let reference = uncaptured_selection
                    .prepare(ControlPhaseInputs {
                        prefix,
                        generation: &generation,
                        assessment_points: None,
                        v4: Some(boundary),
                        population: &fixture.population,
                        brains: &fixture.brains,
                        wall_now_ms: 31000,
                        config,
                    })
                    .unwrap();
                let expected_controls = reference.control_updates().to_vec();
                let expected_baselines = reference.baseline_controls().to_vec();
                let _ = reference;
                let expected_row = uncaptured_selection.baseline_observation.clone();
                let phase = selection
                    .prepare(ControlPhaseInputs {
                        prefix,
                        generation: &generation,
                        assessment_points: None,
                        v4: Some(boundary),
                        population: &fixture.population,
                        brains: &fixture.brains,
                        wall_now_ms: 31000,
                        config,
                    })
                    .unwrap();
                let due = phase.baseline_controls().len() == 1;
                assert_eq!(phase.cost_micros().work.baseline_deliveries, u64::from(due));
                assert_eq!(phase.cost_micros().work.ray_preparations, 1);
                assert_eq!(phase.control_updates(), expected_controls);
                assert_eq!(phase.baseline_controls(), expected_baselines);
                let pending = phase.teacher_samples();
                assert_eq!(pending.key, step_key);
                assert_eq!(pending.decisions.len(), usize::from(due));
                if due {
                    assert_eq!(
                        pending
                            .observation(0)
                            .unwrap()
                            .iter()
                            .map(|v| v.to_bits())
                            .collect::<Vec<_>>(),
                        expected_row.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                    );
                    assert_eq!(pending.decisions[0].physical_snake_id, baseline_id);
                    assert_eq!(
                        pending.decisions[0].applied_turn,
                        phase.baseline_controls()[0].next_slot().turn
                    );
                    assert_eq!(
                        pending.decisions[0].boost_request,
                        phase.baseline_controls()[0].next_slot().boost
                    );
                } else {
                    assert!(pending.observation(0).is_none());
                }

                let grant = phase.ray_grant(RayObservationOwner::Baseline(0));
                if due {
                    delivered.push(ordinal);
                    let result = &phase.baseline_controls()[0];
                    assert!(result.sensor_diagnostics.ray_work.charged() > 0);
                    assert_eq!(result.diagnostics.world_snakes, 0);
                    assert_eq!(result.diagnostics.canonical_order_capacity, 0);
                    assert_eq!(result.next_rng, source_rng.baselines[0]);
                    assert_eq!(result.next_slot.v2_last_control_step, Some(41 + ordinal));
                    if mode == BaselineV2Mode::Deterministic {
                        assert_eq!(
                            result.next_behavior_word,
                            Some(source_rng.baseline_behaviors[0].uniform_state)
                        );
                    }
                    assert!(grant.is_some());
                } else {
                    assert!(grant.is_none());
                    assert_eq!(
                        find_control(phase.control_updates(), baseline_id)
                            .unwrap()
                            .source(),
                        SelectedControlSource::BaselineHeld
                    );
                }
                let committed = commits.prepare(phase).unwrap();
                let world = committed.world().clone();
                let rng = committed.rng().clone();
                let lifecycle = committed.baseline_lifecycle().clone();
                let brains = committed.brains().to_vec();
                let _ = committed;
                #[cfg(feature = "engine-test-hooks")]
                {
                    let counts = selection.qualification_observation_counts();
                    assert_eq!(counts[1].observations, u64::from(due));
                    assert_eq!(counts[1].v4_observations, u64::from(due));
                }
                // This layer acknowledges a successful outer publication; real-step tests cover the barrier.
                selection.publish_inspector(step_key, |_, _, _| false);
                let observed = selection
                    .inspector()
                    .unwrap()
                    .latest(7, 3, 41 + ordinal)
                    .unwrap();
                assert_eq!(
                    observed.source,
                    crate::engine::inspector::InspectorObservationSource::BaselineV2
                );
                assert_eq!(observed.layout.profile, profile);
                let trace = observed
                    .trace
                    .as_ref()
                    .expect("baseline-v2 actual raw trace");
                assert_eq!((trace.profile, trace.bins), (profile, 16));
                assert_eq!(trace.physical_id, observed.target.physical_id);
                assert!(observed.baseline_action.is_some());
                if due {
                    assert_eq!(observed.normalized_inputs(), expected_row);
                    assert_eq!(observed.age_steps(41 + ordinal), Some(0));
                } else {
                    assert!(observed.age_steps(41 + ordinal).unwrap() > 0);
                }
                let proof = commits.baseline_v2_proof().unwrap();
                assert!(proof.matches(step_key, &rng, &lifecycle));
                let mut stale_rng = rng.clone();
                stale_rng.baseline_behaviors[0].uniform_state ^= 1;
                assert!(!proof.matches(step_key, &stale_rng, &lifecycle));
                let mut stale_policy = lifecycle.clone();
                stale_policy.slots[0].v2_last_control_step = None;
                assert!(!proof.matches(step_key, &rng, &stale_policy));
                assert_eq!(rng.baselines, source_rng.baselines);
                if !due {
                    assert_eq!(rng.baseline_behaviors, source_rng.baseline_behaviors);
                    assert_eq!(lifecycle.slots[0], source_lifecycle.slots[0]);
                }
                fixture.world = world;
                fixture.rng = rng;
                fixture.lifecycle = lifecycle;
                fixture.brains = brains;
            }
            assert_eq!(delivered, vec![0, 3, 7, 10]);
        }
    }
}

#[test]
fn v4_profiles_share_neural_external_boundary_and_preserve_v1_baseline_across_workers() {
    use crate::engine::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState};
    use crate::engine::ray_work::{RayObservationOwner, RayWorkLimits};
    use crate::engine::sensor_contract::SensorProfile;
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in [8, 16, 32] {
            let legacy_plan = graph_plan_for_width(19 + 4 * bins);
            let mut legacy_fixture = fixture(&legacy_plan);
            make_fixture_bodies_physics_ready(&mut legacy_fixture.world);
            let legacy = prepared_summary_for(legacy_plan, &legacy_fixture);
            let mut expected = None;
            for workers in [1, 3] {
                let (plan, mut selection) = v4_workspace(profile, bins, workers);
                let mut fixture = fixture(&plan);
                make_fixture_bodies_physics_ready(&mut fixture.world);
                if workers > 1 {
                    fixture.world.snakes.reverse();
                    fixture.world.controller_leases.reverse();
                    fixture.brains.reverse();
                }
                let baseline = fixture
                    .world
                    .snakes
                    .iter()
                    .find(|snake| snake.kind == SnakeKind::Baseline)
                    .unwrap();
                selection
                    .configure_inspector(
                        Some(crate::engine::inspector::InspectorTarget {
                            world_epoch: key(81).world_epoch(),
                            generation: key(81).generation(),
                            physical_id: baseline.id,
                            frame_v1_id: baseline.frame_v1_id,
                        }),
                        crate::engine::inspector::MAX_INSPECTOR_BYTES,
                    )
                    .unwrap();
                let source_world = fixture.world.clone();
                let source_rng = fixture.rng.clone();
                let source_brains = fixture.brains.clone();
                let lives = EvolvedGenerationState::default();
                let mut prefix_workspace = FixedStepPrefixWorkspace::new();
                let prefix = prefix_workspace
                    .prepare(FixedStepPrefixInputs {
                        key: key(81),
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
                let boundary = V4BoundaryContext {
                    context: crate::engine::sensors_v4::V4ObservationContext {
                        generation_elapsed_seconds: prefix.generation_elapsed_seconds(),
                        evolved: &lives,
                        experiment: EvolvedExperimentConfig::legacy(240.0),
                    },
                    limits: RayWorkLimits::new(1, 256, 767, 8).unwrap(),
                };
                let mut commit_workspace = ControlCommitWorkspace::new();
                let mut previous_capacity = None;
                for _ in 0..3 {
                    let phase = selection
                        .prepare(ControlPhaseInputs {
                            prefix,
                            generation: &generation,
                            assessment_points: None,
                            v4: Some(boundary),
                            population: &fixture.population,
                            brains: &fixture.brains,
                            wall_now_ms: 31_000,
                            config: control_config(),
                        })
                        .unwrap();
                    assert_eq!(phase.diagnostics().neural_evaluations, 2);
                    assert_eq!(phase.diagnostics().baseline_observations, 1);
                    let baseline = &phase.baseline_controls()[0];
                    assert_eq!(baseline.next_rng(), &legacy.baselines[0].next_rng);
                    assert_eq!(baseline.next_slot(), legacy.baselines[0].next_slot);
                    assert_eq!(baseline.next_strategy(), legacy.baselines[0].next_strategy);
                    assert_eq!(baseline.sensor_diagnostics().ray_work.charged(), 0);
                    let (event, observation) = phase.external_observation(0).unwrap();
                    let width = 24
                        + if profile == SensorProfile::Basic {
                            6
                        } else {
                            15
                        } * bins;
                    assert_eq!(observation.len(), width);
                    assert!(observation.iter().all(|value| value.is_finite()));
                    assert_eq!(event.snake_id(), 30);
                    assert!(event.diagnostics().ray_work.charged() > 0);
                    assert!(event.diagnostics().ray_work.charged() <= 255);
                    assert!(!event.delivery().previous_observation_delivered);
                    let controls: Vec<_> = phase
                        .control_updates()
                        .iter()
                        .map(|update| {
                            (
                                update.snake_id(),
                                update.source(),
                                update.turn().to_bits(),
                                update.boost(),
                            )
                        })
                        .collect();
                    let observations: Vec<_> =
                        observation.iter().map(|value| value.to_bits()).collect();
                    let snapshot = (controls, observations, event.diagnostics());
                    if let Some(expected) = &expected {
                        assert_eq!(&snapshot, expected);
                    } else {
                        expected = Some(snapshot);
                    }
                    let capacities = phase.diagnostics();
                    if let Some(previous) = previous_capacity {
                        assert_eq!(capacities, previous);
                    }
                    previous_capacity = Some(capacities);
                    let committed = commit_workspace.prepare(phase).unwrap();
                    assert_eq!(committed.world().ray_cursor.revision(), 1);
                    assert_eq!(
                        committed.world().ray_cursor.next_owner(),
                        Some(RayObservationOwner::Brain(BrainHandle {
                            id: 103,
                            epoch: EPOCH
                        }))
                    );
                    for id in [10, 20, 40] {
                        assert!(
                            committed
                                .world()
                                .snakes
                                .iter()
                                .find(|snake| snake.id == id)
                                .unwrap()
                                .observation_delivered
                        );
                    }
                    assert!(
                        !committed
                            .world()
                            .snakes
                            .iter()
                            .find(|snake| snake.id == 30)
                            .unwrap()
                            .observation_delivered
                    );
                    let mut step_workspace = WorldStepWorkspace::new();
                    let step = step_workspace
                        .prepare(committed, world_step_config())
                        .unwrap();
                    assert_eq!(step.world().ray_cursor, committed.world().ray_cursor);
                    assert_eq!(step.world().ray_cursor.revision(), 1);
                    assert_eq!(fixture.world, source_world);
                    assert_eq!(fixture.rng, source_rng);
                    assert_eq!(fixture.brains, source_brains);
                    let _ = step;
                    selection.publish_inspector(key(81), |_, _, _| false);
                    let observed = selection
                        .inspector()
                        .unwrap()
                        .latest(
                            key(81).world_epoch(),
                            key(81).generation(),
                            key(81).source_completed_step() + 1,
                        )
                        .unwrap();
                    assert_eq!(
                        observed.source,
                        crate::engine::inspector::InspectorObservationSource::BaselineV1
                    );
                    assert_eq!(observed.layout.profile, SensorProfile::V3);
                    let trace = observed
                        .trace
                        .as_ref()
                        .expect("legacy baseline actual raw trace");
                    assert_eq!((trace.profile, trace.bins), (SensorProfile::V3, bins));
                    assert_eq!(trace.physical_id, observed.target.physical_id);
                    assert_eq!(observed.normalized_inputs().len(), 19 + 4 * bins);
                    assert!(observed.baseline_action.is_none());
                    let action = observed
                        .baseline_v1_action
                        .expect("actual frozen v1 policy result");
                    let committed_baseline = committed
                        .world()
                        .snakes
                        .iter()
                        .find(|snake| snake.id == observed.target.physical_id)
                        .unwrap();
                    assert_eq!(action.turn, committed_baseline.turn);
                    assert_eq!(action.boost, committed_baseline.input_boost);
                    assert_eq!(Some(action.strategy), committed_baseline.baseline_strategy);
                    assert!(action.sector < bins);
                }
            }
        }
    }
}

#[test]
fn v4_failed_late_control_and_physics_discard_cursor_and_delivery_publication() {
    use crate::engine::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState};
    use crate::engine::ray_work::RayWorkLimits;
    use crate::engine::sensor_contract::SensorProfile;
    let (plan, mut selection) = v4_workspace(SensorProfile::Motion, 32, 3);
    let mut fixture = fixture(&plan);
    make_fixture_bodies_physics_ready(&mut fixture.world);
    fixture.world.pellets.push(PelletState {
        id: 81,
        position: WorldPoint {
            x: 1000.0,
            y: -500.0,
        },
        value: 1.0,
        kind: 0,
        color: 0,
        owner: None,
    });
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_brains = fixture.brains.clone();
    let lives = EvolvedGenerationState::default();
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(82),
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
    let boundary = V4BoundaryContext {
        context: crate::engine::sensors_v4::V4ObservationContext {
            generation_elapsed_seconds: prefix.generation_elapsed_seconds(),
            evolved: &lives,
            experiment: EvolvedExperimentConfig::legacy(240.0),
        },
        limits: RayWorkLimits::new(1, 256, 767, 8).unwrap(),
    };
    let mut commit_workspace = ControlCommitWorkspace::new();
    let mut malformed_population = fixture.population.clone();
    malformed_population[0].weights[0] = f32::NAN;
    assert!(selection
        .prepare(ControlPhaseInputs {
            prefix,
            generation: &generation,
            assessment_points: None,
            v4: Some(boundary),
            population: &malformed_population,
            brains: &fixture.brains,
            wall_now_ms: 31_000,
            config: control_config(),
        })
        .is_err());
    assert!(!selection.is_ready());
    assert_eq!(fixture.world, source_world);
    for malformed_commit in [true, false] {
        let mut phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                assessment_points: None,
                v4: Some(boundary),
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        if malformed_commit {
            phase.calculation_key = CalculationBatchKey::new(7, 42, EPOCH);
            assert!(commit_workspace.prepare(phase).is_err());
            assert!(!commit_workspace.is_ready());
        } else {
            let committed = commit_workspace.prepare(phase).unwrap();
            assert_eq!(committed.world().ray_cursor.revision(), 1);
            let mut step_workspace = WorldStepWorkspace::new();
            let mut bad_physics = world_step_config();
            bad_physics.physics.maximum_pellet_index_entries = 1;
            assert!(matches!(
                step_workspace.prepare(committed, bad_physics),
                Err(WorldStepError::Physics(_))
            ));
            assert!(!step_workspace.is_ready());
            let step = step_workspace
                .prepare(committed, world_step_config())
                .unwrap();
            assert_eq!(step.world().ray_cursor.revision(), 1);
        }
        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.brains, source_brains);
    }
}

#[test]
fn one_indexed_boundary_selects_baseline_external_neural_and_takeover_exclusively() {
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
            key: key(1),
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

    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
    assert_eq!(fixture.brains, source_brains);
    assert_eq!(prepared.key(), key(1));
    assert_eq!(prepared.calculation_key().step(), 41);
    let controls = prepared.control_updates();
    assert_eq!(
        controls
            .iter()
            .map(|update| update.snake_id())
            .collect::<Vec<_>>(),
        vec![10, 20, 30, 35, 40, 50]
    );
    assert_eq!(controls[0].source(), SelectedControlSource::NeuralEvaluated);
    assert_eq!(controls[1].source(), SelectedControlSource::Baseline);
    assert_eq!(controls[2].source(), SelectedControlSource::ExternalHeld);
    assert_eq!(controls[2].turn(), 0.75);
    assert!(controls[2].boost());
    assert_eq!(
        controls[3].source(),
        SelectedControlSource::ExternalOnlyNeutral
    );
    assert_eq!(controls[3].turn(), 0.0);
    assert!(!controls[3].boost());
    assert_eq!(controls[4].source(), SelectedControlSource::NeuralTakeover);
    assert_eq!(
        controls[5].source(),
        SelectedControlSource::ExternalReservedNeutral
    );
    assert_eq!(controls[5].turn(), 0.0);
    assert!(!controls[5].boost());
    assert_eq!(prepared.controller_transitions().len(), 3);
    assert_eq!(prepared.baseline_controls().len(), 1);
    assert_eq!(prepared.baseline_controls()[0].snake_id(), 20);
    assert_eq!(prepared.external_events().len(), 1);
    let (event, observation) = prepared.external_observation(0).unwrap();
    assert_eq!(event.snake_id(), 30);
    assert_eq!(event.connection_id(), 700);
    assert_eq!(event.kind(), ControllerKind::Player);
    assert_eq!(observation.len(), 51);
    assert!(observation.iter().all(|value| value.is_finite()));
    assert_eq!(event.delivery().previous_delivered_points, 1.0);
    assert_eq!(
        prefix.world().snakes[event.snake_index()].delivered_observation_points,
        1.0
    );
    let diagnostics = prepared.diagnostics();
    assert_eq!(diagnostics.controls, 6);
    assert_eq!(diagnostics.baseline_observations, 1);
    assert_eq!(diagnostics.external_observations, 1);
    assert_eq!(diagnostics.neural_evaluations, 2);
    assert_eq!(diagnostics.neural_takeovers, 1);
    assert!(diagnostics.body_index.segments > 0);
    assert_eq!(diagnostics.pellet_index.pellets, 1);
}

#[test]
fn due_baseline_respawn_is_sampled_by_the_same_shared_control_boundary() {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    let source_baseline_id = BASELINE_ENTITY_ID_START + 10;
    let baseline = fixture
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.baseline_slot == Some(0))
        .unwrap();
    baseline.id = source_baseline_id;
    baseline.alive = false;
    baseline.turn = 0.0;
    baseline.previous_turn = 0.0;
    baseline.input_boost = false;
    baseline.previous_input_boost = false;
    fixture.lifecycle.slots[0].snake_id = source_baseline_id;
    fixture.lifecycle.slots[0].respawn_remaining_seconds = Some(DT * 0.5);
    fixture.allocators.next_baseline_id = BASELINE_ENTITY_ID_START + 100;
    let replacement_baseline_id = fixture.allocators.next_baseline_id;

    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_allocators = fixture.allocators.clone();
    let source_lifecycle = fixture.lifecycle.clone();
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
    assert_eq!(prefix.diagnostics().baseline_respawn.completed_slots, 1);
    let replacement = prefix
        .world()
        .snakes
        .iter()
        .find(|snake| snake.baseline_slot == Some(0))
        .unwrap();
    assert_eq!(replacement.id, replacement_baseline_id);
    assert!(replacement.alive);

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
    assert!(prepared.control_updates().iter().any(|update| {
        update.snake_id() == replacement_baseline_id
            && update.source() == SelectedControlSource::Baseline
    }));
    assert_eq!(prepared.baseline_controls().len(), 1);
    assert_eq!(
        prepared.baseline_controls()[0].snake_id(),
        replacement_baseline_id
    );
    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.allocators, source_allocators);
    assert_eq!(fixture.lifecycle, source_lifecycle);
}

#[test]
fn post_physics_baseline_death_starts_full_delay_in_the_same_working_step() {
    let plan = graph_plan();
    let mut fixture = fixture(&plan);
    let evolved_position = fixture
        .world
        .snakes
        .iter()
        .find(|snake| snake.id == 10)
        .unwrap()
        .position;
    let baseline = fixture
        .world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == 20)
        .unwrap();
    baseline.position = evolved_position;
    baseline.previous_position = evolved_position;
    make_fixture_bodies_physics_ready(&mut fixture.world);
    let source_world = fixture.world.clone();
    let source_rng = fixture.rng.clone();
    let source_lifecycle = fixture.lifecycle.clone();

    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let prefix = prefix_workspace
        .prepare(FixedStepPrefixInputs {
            key: key(29),
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
    let mut step_workspace = WorldStepWorkspace::new();
    let prepared = step_workspace
        .prepare(committed, world_step_config())
        .unwrap();

    let baseline = prepared
        .world()
        .snakes
        .iter()
        .find(|snake| snake.id == 20)
        .unwrap();
    assert!(!baseline.alive);
    let slot = prepared.baseline_lifecycle().slots[0];
    assert_eq!(slot.snake_id, 20);
    assert_eq!(slot.respawn_remaining_seconds, Some(20.0));
    assert_eq!(slot.turn.to_bits(), 0.0_f32.to_bits());
    assert!(!slot.boost);
    assert_eq!(prepared.diagnostics().physics.baseline_deaths, 1);
    assert_ne!(prepared.rng(), &source_rng);
    assert_eq!(fixture.world, source_world);
    assert_eq!(fixture.rng, source_rng);
    assert_eq!(fixture.lifecycle, source_lifecycle);
}
