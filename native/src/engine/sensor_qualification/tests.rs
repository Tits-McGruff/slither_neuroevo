use super::*;
use sha2::Digest;

#[test]
fn query_detail_admission_preserves_defaults_and_projects_both_settings() {
    let mut options = super::super::checkpoint_capacity::capacity_workload_options(55, false);
    let original = sensor_workload_spec(&options, None).unwrap().0;
    let value = |updates: &[FreshRunSettingUpdate], path| {
        updates
            .iter()
            .find(|entry| entry.path == path)
            .unwrap()
            .value
    };
    assert_eq!(value(&original, "sense.maxPelletChecks"), 900.0);
    assert_eq!(value(&original, "sense.maxSegmentChecks"), 2200.0);
    for (pellets, segments) in [(100, 200), (3000, 4000)] {
        options.query_limits = Some(QueryDetailLimits::new(pellets, segments).unwrap());
        let selected = sensor_workload_spec(&options, None).unwrap().0;
        assert_eq!(value(&selected, "sense.maxPelletChecks"), pellets as f64);
        assert_eq!(value(&selected, "sense.maxSegmentChecks"), segments as f64);
        for entry in &original {
            if entry.path != "sense.maxPelletChecks" && entry.path != "sense.maxSegmentChecks" {
                assert_eq!(value(&selected, entry.path.as_str()), entry.value);
            }
        }
    }
    for (pellet_checks, segment_checks) in [
        (0, 200),
        (99, 200),
        (3001, 200),
        (100, 199),
        (100, 4001),
        (usize::MAX, usize::MAX),
    ] {
        assert!(QueryDetailLimits::new(pellet_checks, segment_checks).is_err());
        options.query_limits = Some(QueryDetailLimits {
            pellet_checks,
            segment_checks,
        });
        assert!(sensor_workload_spec(&options, None).is_err());
        assert!(
            run_sensor_qualification(&options, Path::new("unused-invalid-query-detail"))
                .unwrap_err()
                .contains("query detail")
        );
    }
    options.query_limits = None;
    let restored = sensor_workload_spec(&options, None).unwrap().0;
    for entry in &original {
        assert_eq!(value(&restored, entry.path.as_str()), entry.value);
    }
}

/// Exclusive native-only scratch; never a user database or retained archive.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "slither-sensor-qualification-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.0)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            && self.0.canonicalize().is_ok_and(|path| path == self.0)
        {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

#[test]
fn evolved_population_selection_and_ordinary_generation_handoff() {
    let mut options = super::super::checkpoint_capacity::capacity_workload_options(55, false);
    options.population = 5;
    options.profile = SensorProfile::Motion;
    options.bins = 16;
    options.ray_limits = Some(RayWorkLimits::new(8, 512, 65536, 512).unwrap());
    options.source_graph = Some(GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: 264 },
            },
            GraphNodeSpec {
                id: "memory".into(),
                kind: GraphNodeKind::Lstm {
                    input_size: 264,
                    hidden_size: 4,
                },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: 4,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".into(),
                to: "memory".into(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "memory".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            },
        ],
        outputs: vec![GraphOutputRef {
            node_id: "head".into(),
            port: None,
        }],
        output_size: 2,
    });
    let (mut updates, spec) = sensor_workload_spec(&options, None).unwrap();
    for entry in &mut updates {
        if entry.path == "generationSeconds" {
            entry.value = 8.0;
        }
        if entry.path == "brain.controlDt" {
            entry.value = 0.017;
        }
    }
    let mut pending = prepare_sensor_qualification_run(
        Stage6aP0FreshRunRequest {
            run_id: "evolved-cohort-selection-test".into(),
            seed: 42,
            memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
        },
        &updates,
        spec,
        options.profile,
        options.bins,
        RunningStepWorkLimits {
            calculation_workers: 1,
            ray_work: options.ray_limits,
            ..RunningStepWorkLimits::provisional_defaults()
        },
        InferenceMathBackend::Sse2,
        None,
        &[],
    )
    .unwrap();
    let expected: Vec<_> = [0, 1, 3]
        .iter()
        .map(|&i| {
            pending.qualification_authority().state().population[i]
                .weights
                .clone()
        })
        .collect();
    let source = Scratch::new();
    let descriptor = pending
        .publish_checkpoint(
            &source.0,
            CheckpointOperationId::parse("11111111111111111111111111111111").unwrap(),
        )
        .unwrap();
    drop(pending);
    let file = source.0.join(&descriptor.relative_filename);
    let source_bytes = std::fs::read(&file).unwrap();
    let cohort = super::super::evolved_cohort::EvolvedCohort::load(&file, 3).unwrap();
    assert_eq!(cohort.weights, expected);
    assert_eq!(cohort.provenance["selection"][2]["sourceSlot"], 3);
    assert!(super::super::evolved_cohort::EvolvedCohort::load_production(&file, 3).is_err());
    drop(cohort);
    options.population = 3;
    options.source_graph = None;
    options.workers = 1;
    options.warmup_steps = 1;
    options.measured_steps = 960;
    let report =
        run_evolved_population_qualification(&options, &Scratch::new().0, &file, false).unwrap();
    assert!(report.admitted, "{:?}", report.rejection);
    assert_eq!(report.completed_steps, 961);
    // Floating fixed-step chronology reaches the first 8-second boundary
    // after step 480; this horizon guarantees at least one real handoff.
    assert!(report.gameplay_boundaries.len() >= 3);
    let settings = &report.evolved_population.unwrap()["normalizedSettings"];
    assert_eq!(
        settings
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["path"] == "snakeCount")
            .unwrap()["value"],
        3.0
    );
    assert_eq!(std::fs::read(&file).unwrap(), source_bytes);
    // Small fixtures check noninterference; measured studies use the mature
    // 85-genome exact-396 cohort supplied separately.
    if InferenceMathBackend::I16MatrixFma128.is_available() {
        options.warmup_steps = 0;
        options.measured_steps = 1200;
        for backend in [
            InferenceMathBackend::Sse2,
            InferenceMathBackend::I16MatrixFma128,
        ] {
            options.math = backend;
            let plain_output = Scratch::new();
            let plain = mature_evolved_population(
                &options,
                &Scratch::new().0,
                &file,
                2,
                &plain_output.0,
                false,
            )
            .unwrap();
            let observed_output = Scratch::new();
            let diagnostics = observed_output.0.join("evolution");
            let observed = qualify_evolved_backend_evolution(
                &options,
                &Scratch::new().0,
                &file,
                2,
                &observed_output.0,
                false,
                &diagnostics,
            )
            .unwrap();
            assert_eq!(observed.continuation_sha256, plain.continuation_sha256);
            let rows: Vec<serde_json::Value> =
                std::fs::read_to_string(diagnostics.join("generations.jsonl"))
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
            assert_eq!(rows.len(), 2);
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(row["generation"], index + 1);
                assert_eq!(row["fitnessBySlot"].as_array().unwrap().len(), 3);
                assert_eq!(row["population"]["genomes"].as_array().unwrap().len(), 3);
                assert_eq!(
                    row["population"]["aggregateQuantization"]["clippingCount"],
                    0
                );
                assert!(row["normalEngineWallSeconds"].as_f64().unwrap() > 0.0);
                assert_eq!(row["reproduction"]["eliteCopies"], 1);
                assert_eq!(row["reproduction"]["offspring"], 2);
                assert_eq!(
                    row["population"]["genomes"][0]["matrices"]
                        .as_array()
                        .unwrap()
                        .len(),
                    9
                );
                assert_eq!(
                    row["population"]["genomes"][0]["separateF32Biases"][0]["length"],
                    16
                );
                assert!(row["population"]["sampledControllerComparisons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["commonPostPublicationState"].as_array().unwrap().len() == 8));
            }
            assert!(!diagnostics
                .join(rows[0]["checkpointFile"].as_str().unwrap())
                .exists());
            let last = diagnostics.join(rows[1]["checkpointFile"].as_str().unwrap());
            let decoded = super::super::evolved_cohort::EvolvedCohort::load(&last, 3).unwrap();
            assert_eq!(decoded.provenance["sourceGeneration"], 3);
            assert!(qualify_evolved_backend_evolution(
                &options,
                &Scratch::new().0,
                &file,
                2,
                &observed_output.0,
                false,
                &diagnostics
            )
            .is_err());
        }
        options.math = InferenceMathBackend::I16AffineFma128;
        assert!(qualify_evolved_backend_evolution(
            &options,
            &Scratch::new().0,
            &file,
            2,
            &source.0,
            false,
            &source.0.join("mixed-rejected")
        )
        .is_err());
    }
    assert_eq!(std::fs::read(&file).unwrap(), source_bytes);
}

#[test]
fn committed_observer_does_not_change_world_rng_lives_weights_or_recurrence_across_workers() {
    for mode in [
        None,
        Some(super::super::baseline_v2::BaselineV2Mode::Deterministic),
        Some(super::super::baseline_v2::BaselineV2Mode::Stochastic),
    ] {
        let mut options = SensorQualificationOptions {
            source_graph: None,
            real_clock: false,
            query_limits: None,
            population: 55,
            profile: SensorProfile::Motion,
            bins: 8,
            large_graph: false,
            trained_artifact: None,
            respawn: true,
            seed: 42,
            workers: 1,
            math: InferenceMathBackend::Scalar,
            ray_limits: Some(RayWorkLimits::new(1, 256, 16384, 512).unwrap()),
            baseline: mode.map(|mode| super::super::baseline_v2::BaselineV2Settings {
                mode,
                control_interval_seconds: 0.06,
            }),
            teacher_respawn_delay_seconds: 20.0,
            capture: mode.map(|_| {
                super::super::capture_qualification::CaptureQualificationOptions {
                    drain_every_steps: 1,
                    queue_chunks: 1024,
                }
            }),
            warmup_steps: 1,
            measured_steps: 12,
            environment: "development".into(),
        };
        let observed = run_with_observer(&options, &Scratch::new().0, true).unwrap();
        let unobserved = run_with_observer(&options, &Scratch::new().0, false).unwrap();
        options.workers = 3;
        let parallel = run_with_observer(&options, &Scratch::new().0, true).unwrap();
        assert!(observed.admitted && unobserved.admitted && parallel.admitted);
        assert_eq!(observed.completed_steps, 13);
        assert!(observed.observations > 0);
        assert_eq!(unobserved.observations, 0);
        assert_eq!(observed.continuation_sha256, unobserved.continuation_sha256);
        assert_eq!(observed.continuation_sha256, parallel.continuation_sha256);
        assert_eq!(observed.observations, parallel.observations);
        assert!(observed.normalized.iter().all(|histogram| histogram.samples
            == observed.observations
            && histogram.nonfinite == 0));
        assert!(observed.raw[11].samples > 0);
        assert!(observed.ray_charged_units <= 12 * 16384);
        assert!(observed.step_cost.is_some() && observed.observer_cost.is_some());
        assert_eq!(observed.native_phase_costs.samples, 12);
        assert_eq!(observed.physics_phase_costs.samples, 12);
        assert_eq!(observed.physics_phase_costs.milliseconds.len(), 10);
        assert!(observed
            .physics_phase_costs
            .milliseconds
            .values()
            .all(|value| value.is_finite() && *value >= 0.0));
        assert_eq!(
            observed.native_phase_costs.component_total(),
            observed.native_phase_costs.total_micros
        );
        assert!(observed.native_phase_costs.total_micros > 0);

        assert_eq!(observed.consumer_work, parallel.consumer_work);
        if mode.is_some() {
            let capture = observed.capture.as_ref().unwrap();
            assert_eq!(capture.exact_samples_match, Some(true));
            assert_eq!(
                capture.samples_observed.iter().sum::<u64>(),
                observed.consumer_work[1].observations
            );
        }

        assert_eq!(
            observed.consumer_work[0].observations,
            observed.observations
        );
        assert!(observed.consumer_work[1].observations > 0);
        assert_eq!(observed.consumer_work[2].observations, 0);
        assert_eq!(observed.shared_batch_budget_held, Some(true));
        if let Some(mode) = mode {
            assert_eq!(
                observed.consumer_work[1].observations,
                observed.consumer_work[1].v4_observations
            );
            assert!(
                observed.consumer_work[1].observations < 12 * 10,
                "v2 cadence holds between deliveries"
            );
            assert_eq!(observed.consumer_work[1].baseline_action_world_reads, 0);
            if mode == super::super::baseline_v2::BaselineV2Mode::Deterministic {
                assert_eq!(observed.consumer_work[1].behavior_uniform_draws, 0);
            }
            // Stochastic mode may take an entirely unsafe/deterministic route;
            // it must not consume a draw merely to advertise that mode.
        } else {
            assert_eq!(observed.consumer_work[1].v4_observations, 0);
        }
    }
}

#[test]
fn trained_cohort_uses_artifact_graph_and_deployment_with_exact_worker_continuation() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/portable-brain-v1.slither-brain");
    let artifact = load_trained_artifact(&path).unwrap();
    let mut options = SensorQualificationOptions {
        source_graph: None,
        real_clock: false,
        query_limits: None,
        population: 55,
        profile: SensorProfile::Basic,
        bins: 8,
        large_graph: false,
        trained_artifact: Some(path),
        respawn: true,
        seed: 42,
        workers: 1,
        math: InferenceMathBackend::Sse2,
        ray_limits: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
        baseline: Some(super::super::baseline_v2::BaselineV2Settings {
            mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
            control_interval_seconds: 0.06,
        }),
        teacher_respawn_delay_seconds: 0.5,
        capture: Some(
            super::super::capture_qualification::CaptureQualificationOptions {
                drain_every_steps: 1,
                queue_chunks: 1024,
            },
        ),
        warmup_steps: 0,
        measured_steps: 12,
        environment: "development".into(),
    };
    let single = run_sensor_qualification(&options, &Scratch::new().0).unwrap();
    assert!(single.admitted);
    assert_eq!(single.version, 6);
    assert_eq!(single.graph_size, "trained-artifact");
    assert_eq!(single.parameter_count, Some(artifact.weights.len()));
    assert_eq!(
        single.trained_artifact.as_ref().unwrap().artifact_sha256,
        artifact.artifact_sha256
    );
    assert!(
        single.observations > 55 && single.observations < 55 * 12,
        "artifact cadence must hold neural evaluations"
    );
    options.workers = 3.min(std::thread::available_parallelism().unwrap().get());
    let parallel = run_sensor_qualification(&options, &Scratch::new().0).unwrap();
    assert_eq!(single.continuation_sha256, parallel.continuation_sha256);
    assert_eq!(single.consumer_work, parallel.consumer_work);
    assert_eq!(
        serde_json::to_value(&single.normalized).unwrap(),
        serde_json::to_value(&parallel.normalized).unwrap()
    );
    assert_eq!(
        single.capture.as_ref().unwrap().source_sample_sha256,
        parallel.capture.as_ref().unwrap().source_sample_sha256
    );
    assert_eq!(
        single.capture.as_ref().unwrap().exact_samples_match,
        Some(true)
    );
    let family_scratch = Scratch::new();
    let mut entries = Vec::new();
    for (index, delta) in [0.0, 0.125].into_iter().enumerate() {
        let bytes = super::super::trained_cohort::fixture_bytes(delta, 0.06);
        let file = format!("family-{index}.slither-brain");
        std::fs::write(family_scratch.0.join(&file), &bytes).unwrap();
        entries.push(
            serde_json::json!({"file":file,"sha256":format!("{:x}",sha2::Sha256::digest(&bytes))}),
        );
    }
    let index = family_scratch.0.join("cohort.json");
    std::fs::write(
        &index,
        serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"artifacts":entries})).unwrap(),
    )
    .unwrap();
    options.trained_artifact = None;
    options.workers = 1;
    let mixed = run_sensor_cohort_qualification(&options, &Scratch::new().0, &index).unwrap();
    options.workers = 3.min(std::thread::available_parallelism().unwrap().get());
    let mixed_parallel =
        run_sensor_cohort_qualification(&options, &Scratch::new().0, &index).unwrap();
    let unobserved = run_with_sources(
        &options,
        &Scratch::new().0,
        false,
        Some(super::super::trained_cohort::load(&index).unwrap()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(unobserved.observations, 0);
    assert_eq!(mixed.continuation_sha256, unobserved.continuation_sha256);
    assert!(mixed.admitted && mixed_parallel.admitted);
    assert_eq!(mixed.version, 7);
    assert_eq!(mixed.graph_size, "trained-cohort");
    assert!(mixed.trained_artifact.is_none());
    let descriptor = mixed.trained_cohort.as_ref().unwrap();
    assert_eq!(descriptor.families[0].population_slots, 28);
    assert_eq!(descriptor.families[1].population_slots, 27);
    assert_eq!(
        mixed.continuation_sha256,
        mixed_parallel.continuation_sha256
    );
    assert_eq!(mixed.consumer_work, mixed_parallel.consumer_work);
    assert_eq!(
        serde_json::to_value(&mixed.normalized).unwrap(),
        serde_json::to_value(&mixed_parallel.normalized).unwrap()
    );
    assert_eq!(
        mixed.capture.as_ref().unwrap().source_sample_sha256,
        mixed_parallel
            .capture
            .as_ref()
            .unwrap()
            .source_sample_sha256
    );
    assert_eq!(
        mixed.capture.as_ref().unwrap().exact_samples_match,
        Some(true)
    );
    let cohort = super::super::trained_cohort::load(&index).unwrap();
    let insufficient = prepare_sensor_cohort_workload(
        &options,
        Stage6aP0FreshRunRequest {
            run_id: "cohort-source-admission".into(),
            seed: 42,
            memory_ceiling_bytes: cohort.source_reservation_bytes - 1,
        },
        &cohort,
    );
    assert!(insufficient
        .err()
        .unwrap()
        .to_string()
        .contains("construction admission"));
    options.profile = SensorProfile::Motion;
    let wrong_profile =
        run_sensor_cohort_qualification(&options, &Scratch::new().0, &index).unwrap();
    assert!(!wrong_profile.admitted && wrong_profile.completed_steps == 0);
    options.profile = SensorProfile::Basic;
    options.trained_artifact = Some(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/portable-brain-v1.slither-brain"),
    );
    options.profile = SensorProfile::Motion;
    let mismatch = run_sensor_qualification(&options, &Scratch::new().0).unwrap();
    assert!(
        !mismatch.admitted
            && mismatch
                .rejection
                .as_ref()
                .unwrap()
                .contains("matching sensors")
    );
    assert_eq!(mismatch.completed_steps, 0);
    options.profile = SensorProfile::Basic;
    options.large_graph = true;
    assert!(
        !run_sensor_qualification(&options, &Scratch::new().0)
            .unwrap()
            .admitted
    );
}

#[test]
fn actual_capture_bytes_and_authority_match_across_fast_slow_and_parallel_runs() {
    use super::super::capture_qualification::CaptureQualificationOptions;
    let mut options = SensorQualificationOptions {
        source_graph: None,
        real_clock: false,
        query_limits: None,
        population: 55,
        profile: SensorProfile::Motion,
        bins: 16,
        large_graph: false,
        trained_artifact: None,
        respawn: true,
        seed: 42,
        workers: 1,
        math: InferenceMathBackend::Sse2,
        ray_limits: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
        baseline: Some(super::super::baseline_v2::BaselineV2Settings {
            mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
            control_interval_seconds: 1.0 / 60.0,
        }),
        teacher_respawn_delay_seconds: 0.5,
        capture: None,
        warmup_steps: 0,
        measured_steps: 180,
        environment: "development".into(),
    };
    // Invalid admission must fail before touching a workspace or allocating a run.
    for delay in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        0.499,
        60.001,
    ] {
        let mut invalid = options.clone();
        invalid.teacher_respawn_delay_seconds = delay;
        assert!(
            run_sensor_qualification(&invalid, Path::new("unused-invalid-delay"))
                .unwrap_err()
                .contains("teacher respawn delay")
        );
    }
    let reference = run_with_observer(&options, &Scratch::new().0, true).unwrap();
    options.capture = Some(CaptureQualificationOptions {
        drain_every_steps: 1,
        queue_chunks: 1024,
    });
    let fast = run_with_observer(&options, &Scratch::new().0, true).unwrap();
    let captured = fast.capture.as_ref().unwrap();
    assert_eq!(captured.exact_samples_match, Some(true));
    assert!(captured.samples_observed.iter().all(|count| *count > 0));
    assert_eq!(
        captured.samples_observed.iter().sum::<u64>(),
        fast.consumer_work[1].observations
    );
    assert_eq!(captured.due_teacher_histogram.iter().sum::<u64>(), 180);
    assert!(captured.chunks_drained > 10 && captured.bytes_drained > 0);
    assert_eq!(captured.dropped_chunks, 0);
    assert_eq!(captured.count_limited_dropped_chunks, 0);
    assert_eq!(captured.byte_limited_dropped_chunks, 0);
    assert!(captured.queue_peak_bytes <= captured.queue_payload_budget_bytes);
    assert!(captured.queue_payload_budget_bytes < captured.queue_budget_bytes);
    assert_eq!(reference.continuation_sha256, fast.continuation_sha256);
    options.workers = 3.min(std::thread::available_parallelism().unwrap().get());
    let parallel = run_with_observer(&options, &Scratch::new().0, true).unwrap();
    assert_eq!(fast.continuation_sha256, parallel.continuation_sha256);
    assert_eq!(
        captured.source_sample_sha256,
        parallel.capture.as_ref().unwrap().source_sample_sha256
    );
    assert_eq!(
        captured.drained_sample_sha256,
        parallel.capture.as_ref().unwrap().drained_sample_sha256
    );
    options.capture = Some(CaptureQualificationOptions {
        drain_every_steps: 180,
        queue_chunks: 1,
    });
    let slow = run_with_observer(&options, &Scratch::new().0, true).unwrap();
    assert_eq!(fast.continuation_sha256, slow.continuation_sha256);
    let dropped = slow.capture.unwrap();
    assert_eq!(captured.source_sample_sha256, dropped.source_sample_sha256);
    assert!(dropped.dropped_chunks > 0 && dropped.dropped_samples > 0);
    assert_eq!(dropped.exact_samples_match, None);
    assert!(dropped.queue_accounting_matches);
    assert_eq!(dropped.queue_peak_chunks, 1);
    assert_eq!(dropped.count_limited_dropped_chunks, dropped.dropped_chunks);
    assert_eq!(dropped.byte_limited_dropped_chunks, 0);
    assert_eq!(dropped.production_watermark, dropped.released_through);
}

#[test]
fn native_phase_attribution_partitions_nested_and_inconsistent_timer_values() {
    let cost = super::super::running_step::RunningStepCostMicros {
        control_selection: 200,
        control_index: 50,
        control_neural: 120,
        control_sensing: 70,
        control_inference: 60,
        world_step: 90,
        ..Default::default()
    };
    let mut totals = NativePhaseCostTotals::default();
    totals.record(500, cost);
    assert_eq!(totals.component_total(), 500);
    assert_eq!(totals.inference_micros, 50);
    assert_eq!(totals.other_control_micros, 30);
    assert_eq!(totals.other_micros, 210);
    totals.record(100, cost);
    assert_eq!(totals.samples, 2);
    assert_eq!(totals.component_total(), 600);
    assert_eq!(totals.total_micros, 600);
    assert_eq!(totals.world_step_micros, 90);
}

#[test]
fn histogram_preserves_extrema_overflow_nonfinite_and_exact_boundaries() {
    let mut histogram = BoundedHistogram::new("test", -1.0, 1.0);
    for value in [-2.0, -1.0, 0.0, 1.0, 2.0, f64::NAN] {
        histogram.observe(value);
    }
    assert_eq!(
        (
            histogram.samples,
            histogram.below,
            histogram.above,
            histogram.nonfinite
        ),
        (6, 1, 1, 1)
    );
    assert_eq!(
        (histogram.minimum, histogram.maximum),
        (Some(-2.0), Some(2.0))
    );
    assert_eq!(histogram.buckets.iter().sum::<u64>(), 3);
    assert_eq!(histogram.buckets[0], 1);
    assert_eq!(histogram.buckets[31], 1);
}

#[test]
fn graph_counts_match_the_named_modest_and_large_profiles() {
    for (profile, bins, large, parameters) in [
        (SensorProfile::V3, 16, false, 13458),
        (SensorProfile::Basic, 16, false, 15826),
        (SensorProfile::Motion, 16, false, 25042),
        (SensorProfile::V3, 32, true, 402914),
        (SensorProfile::Basic, 32, true, 420578),
        (SensorProfile::Motion, 32, true, 494306),
    ] {
        let width = SensorContractIdentity::new(profile, bins)
            .unwrap()
            .input_size();
        let compiled = super::super::graph::compile_graph(
            &graph(width, large),
            &super::super::fresh_run::stage6a_p0_graph_limits(),
        )
        .unwrap();
        assert_eq!(compiled.total_parameters, parameters);
    }
}
#[test]
fn custom_graph_population_is_bounded_and_real_clock_uses_native_scheduler() {
    let mut options = super::super::checkpoint_capacity::capacity_workload_options(3, false);
    options.bins = 16;
    assert!(sensor_workload_spec(&options, None).is_err());
    options.source_graph = Some(graph(264, false));
    assert!(sensor_workload_spec(&options, None).is_ok());
    for population in [0, 301] {
        let mut invalid = options.clone();
        invalid.population = population;
        assert!(sensor_workload_spec(&invalid, None).is_err());
    }
    let mut conflicting = options.clone();
    conflicting.large_graph = true;
    assert!(sensor_workload_spec(&conflicting, None).is_err());
    options.real_clock = true;
    options.warmup_steps = 1;
    options.measured_steps = 2;
    let workspace = Scratch::new();
    let report = run_sensor_qualification(&options, &workspace.0).unwrap();
    assert!(report.admitted, "{:?}", report.rejection);
    assert_eq!(report.population, 3);
    assert_eq!(report.completed_steps, 3);
    assert_eq!(report.graph_size, "custom");
    assert!(report.source_graph.is_some());
    let clock = report.real_clock.unwrap();
    assert!(clock.measured_wall_seconds > 0.0);
    assert_eq!(clock.dropped_wall_seconds, 0.0);
    assert!((clock.measured_simulation_seconds - 2.0 / 60.0).abs() < 1e-12);
    assert_eq!(clock.gameplay_continuation_sha256.len(), 64);
}
#[test]
fn cost_distribution_reports_nearest_rank_p99_without_a_deadline_gate() {
    let mut samples: Vec<_> = (1..=1000).rev().map(f64::from).collect();
    let report = distribution(&mut samples);
    assert_eq!(report.p95_ms, 950.0);
    assert_eq!(report.p99_ms, 990.0);
    assert_eq!(report.maximum_ms, 1000.0);
    let one = distribution(&mut [2.5]);
    assert_eq!(one.p99_ms, 2.5);
}
