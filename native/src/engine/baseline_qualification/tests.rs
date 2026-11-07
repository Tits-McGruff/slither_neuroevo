use super::*;
fn options() -> TacticalOptions {
    TacticalOptions {
        seed_count: 1,
        steps: 12,
        workers: 1,
        cadence_seconds: 0.06,
        bins: BINS,
    }
}
fn constant_student(profile: SensorProfile, recurrent: bool) -> TacticalStudent {
    constant_student_for(profile, BINS, recurrent)
}

fn constant_student_for(profile: SensorProfile, bins: usize, recurrent: bool) -> TacticalStudent {
    use super::super::training_graph::{
        CanonicalTrainingGraphManifestV1, TrainingDeployment, TrainingLifeStateRule,
    };
    let sensors = super::super::sensor_layout::SensorLayout::for_profile(profile, bins).unwrap();
    let width = sensors.input_size;
    let mut nodes = vec![GraphNodeSpec {
        id: "input".into(),
        kind: GraphNodeKind::Input { output_size: width },
    }];
    let mut edges = Vec::new();
    let head_input = if recurrent {
        nodes.push(GraphNodeSpec {
            id: "memory".into(),
            kind: GraphNodeKind::Gru {
                input_size: width,
                hidden_size: 4,
            },
        });
        edges.push(GraphEdge {
            from: "input".into(),
            to: "memory".into(),
            from_port: None,
            to_port: None,
        });
        4
    } else {
        width
    };
    nodes.push(GraphNodeSpec {
        id: "motor".into(),
        kind: GraphNodeKind::Dense {
            input_size: head_input,
            output_size: 2,
        },
    });
    edges.push(GraphEdge {
        from: if recurrent { "memory" } else { "input" }.into(),
        to: "motor".into(),
        from_port: None,
        to_port: None,
    });
    let graph = super::super::graph::GraphBundle::compile(
        GraphSpec {
            nodes,
            edges,
            outputs: vec![GraphOutputRef {
                node_id: "motor".into(),
                port: None,
            }],
            output_size: 2,
        },
        &super::super::fresh_run::stage6a_p0_graph_limits(),
    )
    .unwrap();
    let deployment = TrainingDeployment {
        control_interval_seconds: options().cadence_seconds,
        life_state_rule: TrainingLifeStateRule::Reset,
        initialization: Default::default(),
    };
    let manifest = CanonicalTrainingGraphManifestV1::build(&graph, &sensors, deployment).unwrap();
    let head = graph.nodes.iter().find(|node| node.id == "motor").unwrap();
    let mut weights = vec![0.0; graph.total_parameters];
    weights[head.parameter_offset + head.input_size] = 0.25;
    weights[head.parameter_offset + head.parameter_length - 1] = 1.0;
    let artifact = super::super::brain_artifact::ValidatedBrainArtifact {
        graph,
        sensors,
        deployment,
        weights,
        canonical_manifest: manifest,
        provenance: super::super::brain_artifact::BrainProvenanceV1 {
            schema_version: 1,
            source_kind: super::super::brain_artifact::BrainSourceKind::Distilled,
            details: serde_json::json!({"fixture":"constant tactical student"}),
        },
        artifact_sha256: "12".repeat(32),
        weights_sha256: "34".repeat(32),
    };
    // Profile rejection is tested separately using the otherwise valid owned artifact.
    if profile != SensorProfile::V3 {
        TacticalStudent::from_validated(artifact).unwrap()
    } else {
        TacticalStudent {
            artifact,
            opponent_boost_bias: 0,
        }
    }
}

#[test]
fn student_profiles_and_widths_bind_actual_sensing_and_reject_mismatches() {
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in [8, 9, 16, 31, 32] {
            for recurrent in [false, true] {
                let student = constant_student_for(profile, bins, recurrent);
                assert_eq!(student.sensor_bins(), bins);
                let request = TacticalOptions { bins, ..options() };
                let report = run_student_tactical_qualification(request, &student, 1).unwrap();
                assert_eq!(report.trials.len(), 14);
                for trial in &report.trials {
                    assert!(trial.observations > 0 && trial.shared_batch_budget_held);
                    assert_eq!(trial.policy_world_reads, 0);
                    assert_eq!(trial.behavior_draws, 0);
                    assert!((trial.first_action.turn - 0.25_f32.tanh()).abs() < 1.0e-6);
                }
                let mismatch = TacticalOptions {
                    bins: if bins == 8 { 9 } else { 8 },
                    ..request
                };
                assert!(run_student_tactical_qualification(mismatch, &student, 1).is_err());
            }
        }
    }
    let mut mismatched = constant_student_for(SensorProfile::Motion, 8, false).artifact;
    mismatched.sensors =
        super::super::sensor_layout::SensorLayout::for_profile(SensorProfile::Basic, 8).unwrap();
    assert!(TacticalStudent::from_validated(mismatched).is_err());
}

#[test]
fn student_uses_actual_cadence_physics_and_exact_multiworker_continuation() {
    let student = constant_student(SensorProfile::Motion, true);
    let first = run_student_tactical_qualification(options(), &student, 1).unwrap();
    let mut multi = options();
    multi.workers = 3.min(std::thread::available_parallelism().unwrap().get());
    let parallel = run_student_tactical_qualification(multi, &student, 1).unwrap();
    assert_eq!(first.trials, parallel.trials);
    assert_eq!(first.reference_v1, parallel.reference_v1);
    assert_eq!(first.trials.len(), 14);
    for result in &first.trials {
        assert!(result.observations > 0 && result.observations < u64::from(result.steps));
        assert!(result.shared_batch_budget_held);
        assert_eq!(result.policy_world_reads, 0);
        assert_eq!(result.behavior_draws, 0);
        assert!((result.first_action.turn - 0.25_f32.tanh()).abs() < 1.0e-6);
        assert!(result.first_action.boost);
        assert!(result.boost_points_spent >= 0.0);
        if result.scenario == "open-wander" {
            assert_eq!(result.boost_points_spent, 0.0);
        }
        if result.scenario == "wall-escape" {
            assert!(result.boost_points_spent > 0.0);
        }
        assert_eq!(result.alive, result.death_cause.is_none());
    }
    let mut mismatch = options();
    mismatch.cadence_seconds = 0.05;
    assert!(run_student_tactical_qualification(mismatch, &student, 1).is_err());
    assert!(
        TacticalStudent::from_validated(constant_student(SensorProfile::V3, false).artifact)
            .is_err()
    );
    assert!(TacticalStudent::from_artifact(b"not an artifact").is_err());
    assert!(run_student_tactical_qualification(options(), &student, 0).is_err());
    let held_out = run_student_tactical_qualification(options(), &student, 2).unwrap();
    assert!(held_out.trials.iter().all(|trial| trial.seed == 2));
    assert_eq!(held_out.first_seed, 2);
}

#[test]
fn student_opponents_keep_constant_outputs_across_different_graph_layouts() {
    let dense = constant_student(SensorProfile::Motion, false);
    let recurrent = constant_student(SensorProfile::Motion, true);
    for student in [&dense, &recurrent] {
        let plan = GraphExecutionPlan::build(&student.artifact.graph).unwrap();
        let mut state = fixture(Scenario::BoostEscape, 1, Policy::MotionDet, &plan).unwrap();
        student.install(&mut state, Scenario::BoostEscape).unwrap();
        assert!(state.lifecycle.slots.is_empty() && state.rng.baselines.is_empty());
        assert!(state.rng.baseline_behaviors.is_empty());
        let opponent = state
            .brains
            .iter()
            .find(|brain| brain.handle.id == 2)
            .unwrap();
        let result = super::super::training_runner::evaluate_training_sequence(
            &student.artifact.graph,
            opponent.non_population_weights.as_ref().unwrap(),
            &[0.5; 264],
            &opponent.recurrent,
            super::super::inference::InferenceMathBackend::detect(),
        )
        .unwrap();
        assert_eq!(result.outputs[0], 0.0);
        assert!((result.outputs[1] - 1.0_f32.tanh()).abs() < 1.0e-6);
    }
    for scenario in SCENARIOS {
        let first = trial_with_subject(
            scenario,
            1,
            Policy::MotionDet,
            options(),
            None,
            Some(&dense),
        )
        .unwrap();
        let second = trial_with_subject(
            scenario,
            1,
            Policy::MotionDet,
            options(),
            None,
            Some(&recurrent),
        )
        .unwrap();
        assert_eq!(
            (
                first.alive,
                first.death_cause,
                first.food_earned,
                first.kills,
                first.boost_points_spent
            ),
            (
                second.alive,
                second.death_cause,
                second.food_earned,
                second.kills,
                second.boost_points_spent
            )
        );
    }
}

#[test]
fn committed_corpus_preserves_trajectories_and_exact_delivered_rows() {
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "slither-tactical-corpus-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    assert!(!root.exists());
    let reference = run_tactical_qualification(options()).unwrap();
    let captured = run_tactical_qualification_with_corpus(options(), Some(&root)).unwrap();
    assert_eq!(
        captured.trials, reference.trials,
        "export changed authority or RNG"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("corpus.json")).unwrap()).unwrap();
    assert!(!root.join(".corpus.json.pending").exists());
    assert_eq!(manifest["inputSize"], 264);
    assert_eq!(manifest["sensorSha256"], reference.sensor_sha256[2]);
    let observations = fs::read(root.join("observations.f32")).unwrap();
    let actions = fs::read(root.join("actions.f32")).unwrap();
    let steps = fs::read(root.join("steps.u32")).unwrap();
    for object in manifest["objects"].as_array().unwrap() {
        let bytes = fs::read(root.join(object["file"].as_str().unwrap())).unwrap();
        assert_eq!(object["bytes"].as_u64().unwrap(), bytes.len() as u64);
        assert_eq!(object["sha256"], format!("{:x}", Sha256::digest(&bytes)));
    }
    let sequences = manifest["sequences"].as_array().unwrap();
    assert_eq!(sequences.len(), 28);
    let mut next = 0usize;
    for (sequence, trial) in sequences.iter().zip(
        captured
            .trials
            .iter()
            .filter(|trial| trial.policy.starts_with("motion-")),
    ) {
        let rows = sequence["rows"].as_u64().unwrap() as usize;
        assert_eq!(sequence["rowOffset"].as_u64().unwrap() as usize, next);
        assert_eq!(rows as u64, trial.observations);
        assert_eq!(sequence["scenario"], trial.scenario);
        assert_eq!(sequence["policy"], trial.policy);
        assert_eq!(
            sequence["trajectoryWitnessSha256"],
            trial.trajectory_witness_sha256
        );
        let turn = f32::from_le_bytes(actions[next * 8..next * 8 + 4].try_into().unwrap());
        let boost = f32::from_le_bytes(actions[next * 8 + 4..next * 8 + 8].try_into().unwrap());
        assert_eq!(turn.to_bits(), trial.first_action.turn.to_bits());
        assert_eq!(boost, if trial.first_action.boost { 1.0 } else { 0.0 });
        let first_flag = (next * 264 + 21) * 4;
        assert_eq!(
            f32::from_le_bytes(observations[first_flag..first_flag + 4].try_into().unwrap()),
            1.0
        );
        let delivered_steps: Vec<_> = steps[next * 4..(next + rows) * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
            .collect();
        assert_eq!(delivered_steps[0], 1);
        assert!(delivered_steps.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(delivered_steps.last().unwrap() <= &trial.steps);
        assert!(
            rows < trial.steps as usize,
            "held steps were incorrectly exported"
        );
        next += rows;
    }
    assert_eq!(manifest["rows"].as_u64().unwrap() as usize, next);
    assert_eq!(observations.len(), next * 264 * 4);
    assert_eq!(actions.len(), next * 8);
    assert_eq!(steps.len(), next * 4);
    assert!(run_tactical_qualification_with_corpus(options(), Some(&root)).is_err());
    assert_eq!(
        fs::read(root.join("observations.f32")).unwrap(),
        observations
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn profiled_corpora_keep_exact_trials_and_bind_dynamic_row_bytes() {
    use std::fs;
    for (profile, bins) in [
        (SensorProfile::Basic, 8),
        (SensorProfile::Basic, 9),
        (SensorProfile::Basic, 32),
        (SensorProfile::Motion, 8),
        (SensorProfile::Motion, 31),
        (SensorProfile::Motion, 32),
    ] {
        let request = TacticalOptions {
            bins,
            steps: 6,
            ..options()
        };
        let root = std::env::temp_dir().join(format!(
            "slither-profile-corpus-{}-{}-{}",
            std::process::id(),
            bins,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let reference = run_tactical_qualification(request).unwrap();
        let captured =
            run_tactical_qualification_with_profiled_corpus(request, Some(&root), profile).unwrap();
        assert_eq!(captured.trials, reference.trials);
        assert_eq!(captured.bins, bins);
        let contract =
            super::super::sensor_contract::SensorContractIdentity::new(profile, bins).unwrap();
        let width = contract.input_size();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("corpus.json")).unwrap()).unwrap();
        assert_eq!(manifest["inputSize"], width);
        assert_eq!(
            manifest["sensorCanonicalJson"],
            String::from_utf8(contract.canonical_bytes().unwrap()).unwrap()
        );
        assert_eq!(manifest["sensorSha256"], contract.sha256_hex().unwrap());
        let observations = fs::read(root.join("observations.f32")).unwrap();
        let actions = fs::read(root.join("actions.f32")).unwrap();
        let steps = fs::read(root.join("steps.u32")).unwrap();
        let expected_policy = if profile == SensorProfile::Basic {
            "basic-"
        } else {
            "motion-"
        };
        let expected: Vec<_> = captured
            .trials
            .iter()
            .filter(|trial| trial.policy.starts_with(expected_policy))
            .collect();
        let mut next = 0usize;
        for (sequence, trial) in manifest["sequences"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&expected)
        {
            assert_eq!(sequence["policy"], trial.policy);
            assert_eq!(sequence["rows"], trial.observations);
            assert_eq!(sequence["rowOffset"], next);
            assert_eq!(
                sequence["trajectoryWitnessSha256"],
                trial.trajectory_witness_sha256
            );
            let offset = (next * width + 21) * 4;
            assert_eq!(
                f32::from_le_bytes(observations[offset..offset + 4].try_into().unwrap()),
                1.0
            );
            assert_eq!(
                f32::from_le_bytes(actions[next * 8..next * 8 + 4].try_into().unwrap()),
                trial.first_action.turn
            );
            assert_eq!(
                u32::from_le_bytes(steps[next * 4..next * 4 + 4].try_into().unwrap()),
                1
            );
            next += trial.observations as usize;
        }
        assert_eq!(expected.len(), 28);
        assert_eq!(manifest["sequences"].as_array().unwrap().len(), 28);
        assert_eq!(observations.len(), next * width * 4);
        assert_eq!(manifest["rows"], next);
        for object in manifest["objects"].as_array().unwrap() {
            let bytes = fs::read(root.join(object["file"].as_str().unwrap())).unwrap();
            assert_eq!(object["bytes"], bytes.len());
            assert_eq!(object["sha256"], format!("{:x}", Sha256::digest(bytes)));
        }
        assert!(
            run_tactical_qualification_with_profiled_corpus(request, Some(&root), profile).is_err()
        );
        assert_eq!(
            fs::read(root.join("observations.f32")).unwrap(),
            observations
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn corpus_width_admission_is_priced_before_creation_and_rejects_layout_changes() {
    use super::super::tactical_corpus::TacticalCorpusWriter;
    let root = std::env::temp_dir().join(format!(
        "slither-profile-admission-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    for bins in [0, 7, 33, usize::MAX] {
        assert!(run_tactical_qualification_with_profiled_corpus(
            TacticalOptions { bins, ..options() },
            Some(&root),
            SensorProfile::Motion,
        )
        .is_err());
        assert!(!root.exists());
    }
    assert!(run_tactical_qualification_with_profiled_corpus(
        options(),
        Some(&root),
        SensorProfile::V3
    )
    .is_err());
    assert!(!root.exists());
    let request = TacticalOptions {
        bins: 32,
        seed_count: 16,
        steps: 400,
        ..options()
    };
    assert!(TacticalCorpusWriter::new(&root, request, SensorProfile::Motion).is_err());
    assert!(!root.exists());
    let small = TacticalOptions { bins: 8, ..request };
    let mut writer = TacticalCorpusWriter::new(&root, small, SensorProfile::Basic).unwrap();
    assert!(writer.committed(1, &[0.0; 264], 0.0, false).is_err());
    writer.committed(1, &[0.0; 72], 0.0, false).unwrap();
    let mut wrong_profile = trial(Scenario::Wander, 1, Policy::MotionDet, options()).unwrap();
    wrong_profile.observations = 1;
    assert!(writer.end(&wrong_profile).is_err());
    assert!(writer.finish(TacticalOptions { bins: 9, ..small }).is_err());
    assert!(!root.join("corpus.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corpus_bound_rejects_before_creating_output_and_partial_matrix_cannot_publish() {
    use super::super::tactical_corpus::TacticalCorpusWriter;
    let root = std::env::temp_dir().join(format!(
        "slither-tactical-bound-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut large = options();
    large.seed_count = 64;
    large.steps = 600;
    assert!(run_tactical_qualification_with_corpus(large, Some(&root)).is_err());
    assert!(!root.exists());
    let mut writer = TacticalCorpusWriter::new(&root, options(), SensorProfile::Motion).unwrap();
    writer.begin();
    let mut row = [0.0; 264];
    row[0] = -0.0;
    writer.committed(1, &row, -0.0, true).unwrap();
    assert!(writer.committed(1, &row, 0.0, false).is_err());
    row[3] = f32::NAN;
    assert!(writer.committed(2, &row, 0.0, false).is_err());
    assert!(writer.finish(options()).is_err());
    assert!(!root.join("corpus.json").exists());
    let bytes = std::fs::read(root.join("observations.f32")).unwrap();
    assert_eq!(bytes.len(), 264 * 4);
    assert_eq!(
        f32::from_le_bytes(bytes[..4].try_into().unwrap()).to_bits(),
        (-0.0_f32).to_bits()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn complete_tactical_matrix_is_repeatable_and_parallel_continuations_match() {
    let first = run_tactical_qualification(options()).unwrap();
    assert_eq!(first.trials.len(), 70);
    let mut multi = options();
    multi.workers = 3.min(std::thread::available_parallelism().unwrap().get());
    let parallel = run_tactical_qualification(multi).unwrap();
    assert_eq!(first.trials, parallel.trials);
    for trial in &first.trials {
        assert!(trial.observations > 0 && trial.shared_batch_budget_held);
        assert!(trial.decision_tail.len() <= 32);
        assert!(trial
            .decision_tail
            .windows(2)
            .all(|pair| pair[0].step < pair[1].step));
        if trial.alive {
            assert!(trial.decision_tail.is_empty());
        }
        assert_eq!(trial.alive, trial.death_cause.is_none());
        if trial.policy != "v1" {
            assert_eq!(trial.policy_world_reads, 0);
        }
        if trial.policy.ends_with("deterministic") {
            assert_eq!(trial.behavior_draws, 0);
        }
    }
    for policy in [Policy::BasicStoch, Policy::MotionStoch] {
        let wander = trial(Scenario::Wander, 1, policy, options()).unwrap();
        assert!(wander.behavior_draws > 0 && wander.alive);
        let saturated = trial(Scenario::Saturation, 1, policy, options()).unwrap();
        assert!(saturated.incomplete_observations > 0);
        assert!(!saturated.first_action.boost);
    }
}
#[test]
fn paired_initial_geometry_is_identical_and_starts_without_foreign_body_contact() {
    let compiled = plan(264).unwrap();
    for scenario in SCENARIOS {
        for seed in 1..=16 {
            let original = fixture(scenario, seed, Policy::V1, &compiled).unwrap();
            let paired = fixture(scenario, seed, Policy::MotionStoch, &compiled).unwrap();
            assert_eq!(original.world, paired.world);
            for head in &original.world.snakes {
                assert!(head.position.x.hypot(head.position.y) + head.radius < 3500.0);
                for body in &original.world.snakes {
                    if body.id == head.id {
                        continue;
                    }
                    let points = &original.world.body_points
                        [body.body.start..body.body.start + body.body.len];
                    for segment in points.windows(2) {
                        let dx = segment[1].x - segment[0].x;
                        let dy = segment[1].y - segment[0].y;
                        let t = (((head.position.x - segment[0].x) * dx
                            + (head.position.y - segment[0].y) * dy)
                            / (dx * dx + dy * dy))
                            .clamp(0.0, 1.0);
                        let distance = (head.position.x - segment[0].x - t * dx)
                            .hypot(head.position.y - segment[0].y - t * dy);
                        assert!(
                            distance > 0.82 * (head.radius + body.radius),
                            "{} seed={seed} head={} body={} distance={distance}",
                            scenario.label(),
                            head.id,
                            body.id
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn motion_crossing_escape_changes_the_colliding_velocity_and_survives_mirrored_physics() {
    let extended = TacticalOptions {
        steps: 180,
        cadence_seconds: DT,
        ..options()
    };
    for scenario in [Scenario::Crossing, Scenario::PredictedCrossing] {
        for seed in 1..=4 {
            for policy in [Policy::MotionDet, Policy::MotionStoch] {
                let result = trial(scenario, seed, policy, extended).unwrap();
                assert!(result.first_action.turn.abs() >= 0.9, "{result:?}");
                assert!(result.alive, "{result:?}");
            }
        }
    }
    let escape = trial(Scenario::BoostEscape, 2, Policy::MotionDet, extended).unwrap();
    assert!(escape.first_action.boost && escape.boost_points_spent > 0.0);
}

#[test]
fn geometric_rear_escape_spends_real_boost_budget_and_survives_at_both_cadences() {
    for cadence_seconds in [DT, 0.06] {
        let extended = TacticalOptions {
            steps: 180,
            cadence_seconds,
            ..options()
        };
        for seed in 1..=16 {
            for policy in [
                Policy::BasicDet,
                Policy::BasicStoch,
                Policy::MotionDet,
                Policy::MotionStoch,
            ] {
                let result = trial(Scenario::BoostEscape, seed, policy, extended).unwrap();
                assert!(
                    result.first_action.boost,
                    "cadence={cadence_seconds} {result:?}"
                );
                assert!(
                    result.boost_points_spent > 0.0 && result.alive,
                    "cadence={cadence_seconds} {result:?}"
                );
                assert_eq!(result.policy_world_reads, 0);
                assert!(result.shared_batch_budget_held);
            }
        }
    }
}

#[test]
fn avoidance_turning_clears_real_walls_and_food_traps_at_both_admitted_cadences() {
    for cadence_seconds in [DT, 0.06] {
        let extended = TacticalOptions {
            steps: 180,
            cadence_seconds,
            ..options()
        };
        for scenario in [Scenario::WallEscape, Scenario::FoodTrap] {
            for seed in 1..=16 {
                for policy in [
                    Policy::BasicDet,
                    Policy::BasicStoch,
                    Policy::MotionDet,
                    Policy::MotionStoch,
                ] {
                    let result = trial(scenario, seed, policy, extended).unwrap();
                    assert!(result.alive, "cadence={cadence_seconds} {result:?}");
                }
            }
        }
    }
}

#[test]
fn actual_wall_and_urgent_crossing_first_actions_keep_qualitative_expectations() {
    let wall = trial(Scenario::WallEscape, 2, Policy::MotionDet, options()).unwrap();
    assert!(wall.first_action.turn.abs() > 0.5 && !wall.first_action.boost);
    let crossing = trial(Scenario::Crossing, 2, Policy::MotionDet, options()).unwrap();
    assert!(
        crossing.first_action.urgent_sectors.unwrap() != 0,
        "{crossing:?}"
    );
    assert_eq!(crossing.first_action.unsafe_route, Some(false));
}
