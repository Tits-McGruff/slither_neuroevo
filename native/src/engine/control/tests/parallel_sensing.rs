use super::*;

#[test]
fn independent_sensor_batch_matches_scalar_across_worker_counts() {
    let plan = graph_plan(51);
    let (world, _brains, _population) = fixture_with_count(&plan, EPOCH, 8);
    let indexed_world = indexed(&world);
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let items = (0..8).rev().collect::<Vec<_>>();
    let available = crate::engine::calculation_workers::available_calculation_workers();
    let mut expected = None;
    for workers in [1, 2, 4, 7] {
        if workers > available {
            continue;
        }
        let sensor = SensorEvaluator::new(sensor_config()).unwrap();
        let mut pipeline =
            NeuralControlPipeline::try_new(8, sensor, plan.clone(), workers, usize::MAX).unwrap();
        let mut observations = vec![0.0; items.len() * 51];
        let mut scratch = SensorScratch::default();
        let (deliveries, diagnostics, work) = pipeline
            .sample_independent_observations(
                &items,
                |index| *index,
                &indexed_world,
                &generation,
                None,
                &mut observations,
                &mut scratch,
                None,
            )
            .unwrap();
        assert!(deliveries.iter().all(Option::is_some));
        assert_eq!(work.body_candidates.samples, items.len() as u64);
        assert_eq!(
            work.body_candidates.total,
            diagnostics
                .iter()
                .map(|d| d.body_query.candidates as u64)
                .sum::<u64>()
        );
        assert_eq!(work.ray_candidates.samples, 0);
        let result = (
            observations
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            deliveries.to_vec(),
            diagnostics.to_vec(),
        );
        if let Some(expected) = &expected {
            assert_eq!(&result, expected);
        } else {
            expected = Some(result);
        }
    }
    assert!(expected.is_some());
}

#[test]
fn release_calculation_worker_panic_preserves_uncommitted_world_and_names_partition() {
    let plan = graph_plan(51);
    let (world, brains, population) = fixture_with_count(&plan, EPOCH, 4);
    let original_world = world.clone();
    let original_brains = brains.clone();
    let indexed_world = indexed(&world);
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let sensor = SensorEvaluator::new(sensor_config()).unwrap();
    let mut pipeline = NeuralControlPipeline::try_new(4, sensor, plan, 2, usize::MAX).unwrap();
    pipeline.panic_inference_partition_once = Some(1);
    let candidates = (0..4)
        .map(|index| CalculationCandidateIndex::new(index, index))
        .collect::<Vec<_>>();
    let mut sensor_scratch = SensorScratch::default();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = pipeline.prepare_and_evaluate(
            NeuralControlBatchInputs {
                key: CalculationBatchKey::new(4, 99, EPOCH),
                candidates: &candidates,
                indexed_world: &indexed_world,
                generation: &generation,
                assessment_points: None,
                v4: None,
                population: &population,
                brains: &brains,
                reset_brains: &[],
            },
            &mut sensor_scratch,
        );
    }))
    .expect_err("injected worker panic must reach the coordinator boundary");
    let detail = panic
        .downcast_ref::<String>()
        .expect("worker root must attach a bounded string diagnostic");
    assert!(detail.contains("calculation inference worker partition 1 panicked"));
    assert_eq!(world, original_world);
    assert_eq!(brains, original_brains);
    assert!(pipeline.ready.is_none());
}

#[test]
fn selected_life_scores_match_across_real_sensing_workers_and_keep_wallet_delivery() {
    let plan = graph_plan(51);
    let scores = [80.0, 30.0, 10.0, 5.0];
    let mut reference = None;
    for workers in [1, 2] {
        let (mut world, mut brains, population) = fixture_with_count(&plan, EPOCH, 4);
        let mut generation = SensorGenerationState::new();
        generation.observe_assessments(&scores).unwrap();
        let sensor = SensorEvaluator::new(sensor_config()).unwrap();
        let mut pipeline =
            NeuralControlPipeline::try_new(4, sensor, plan.clone(), workers, usize::MAX).unwrap();
        let indexed = indexed(&world);
        let candidates = (0..4)
            .rev()
            .map(|index| CalculationCandidateIndex::new(index, index))
            .collect::<Vec<_>>();
        let key = CalculationBatchKey::new(4, 99, EPOCH);
        let batch = pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates,
                    indexed_world: &indexed,
                    generation: &generation,
                    assessment_points: Some(&scores),
                    v4: None,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[],
                },
                &mut SensorScratch::default(),
            )
            .unwrap();
        let observations = batch.observations().to_vec();
        let outputs = batch.outputs().to_vec();
        let _ = batch;
        drop(indexed);
        pipeline.commit_state(key, &mut world, &mut brains).unwrap();
        for snake in &world.snakes {
            assert_eq!(snake.delivered_observation_points, snake.points);
        }
        let result = (observations, outputs, world, brains);
        if let Some(expected) = &reference {
            assert_eq!(&result, expected);
        } else {
            reference = Some(result);
        }
    }
}

#[test]
fn sixteen_worker_fixed_staging_fits_the_declared_worker_scratch_budget() {
    let plan = production_scale_graph_plan();
    let required =
        NeuralControlPipeline::required_staging_bytes_for_validated_workers(85, &plan, 16).unwrap();
    assert!(required < 512 * 1024 * 1024);
}

#[test]
fn sixteen_worker_pipeline_constructs_when_the_host_admits_sixteen_workers() {
    if crate::engine::calculation_workers::available_calculation_workers() < 16 {
        return;
    }
    let sensor = SensorEvaluator::new_v4(
        SensorConfig {
            bins: 16,
            ..SensorConfig::default()
        },
        crate::engine::movement::MovementConfig::typescript_defaults(),
        1.0 / 60.0,
        crate::engine::sensor_contract::SensorProfile::Motion,
    )
    .unwrap();
    assert_eq!(sensor.layout().input_size, 264);
    let plan = production_scale_graph_plan();
    let budget = 512 * 1024 * 1024;
    let required =
        NeuralControlPipeline::required_staging_bytes_for_sensor(85, &sensor, &plan, 16).unwrap();
    assert!(required < budget);
    let pipeline = NeuralControlPipeline::try_new(85, sensor, plan, 16, budget).unwrap();
    assert_eq!(pipeline.capacity_diagnostics().calculation_workers, 16);
    assert!(pipeline.allocated_staging_bytes() <= required);
}

#[test]
fn shared_head_storage_is_charged_only_for_v4_and_rejects_an_insufficient_budget() {
    let v3 = SensorEvaluator::new(SensorConfig::default()).unwrap();
    let v3_plan = graph_plan(v3.layout().input_size);
    let v3_required = NeuralControlPipeline::required_staging_bytes(12, &v3_plan, 1).unwrap();
    assert_eq!(
        NeuralControlPipeline::required_staging_bytes_for_sensor(12, &v3, &v3_plan, 1).unwrap(),
        v3_required
    );
    let pipeline = NeuralControlPipeline::try_new(12, v3, v3_plan, 1, v3_required).unwrap();
    assert_eq!(pipeline.capacity_diagnostics().v4_head_capacity, 0);
    let v4 = SensorEvaluator::new_v4(
        SensorConfig::default(),
        crate::engine::movement::MovementConfig::typescript_defaults(),
        1.0 / 60.0,
        crate::engine::sensor_contract::SensorProfile::Motion,
    )
    .unwrap();
    let plan = graph_plan(v4.layout().input_size);
    let base = NeuralControlPipeline::required_staging_bytes(12, &plan, 1).unwrap();
    let required =
        NeuralControlPipeline::required_staging_bytes_for_sensor(12, &v4, &plan, 1).unwrap();
    assert_eq!(required - base, 12 * std::mem::size_of::<WorldPoint>());
    assert!(matches!(
        NeuralControlPipeline::try_new(12, v4.clone(), plan.clone(), 1, required - 1),
        Err(NeuralControlError::StagingBudgetExceeded { .. })
    ));
    let pipeline = NeuralControlPipeline::try_new(12, v4, plan, 1, required).unwrap();
    assert_eq!(pipeline.capacity_diagnostics().v4_head_capacity, 12);
    assert!(pipeline.allocated_staging_bytes() <= required);
}

#[test]
fn v4_actual_parallel_sensing_and_inference_match_scalar_with_fixed_grants_and_warm_capacity() {
    use crate::engine::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState};
    use crate::engine::sensor_contract::SensorProfile;
    use crate::engine::sensor_rays::BodyRayIndexWorkspace;
    use crate::engine::sensors_v4::{PreparedV4Sensing, V4ObservationContext};
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in [8, 16, 32] {
            let sensor = SensorEvaluator::new_v4(
                SensorConfig {
                    bins,
                    ..SensorConfig::default()
                },
                crate::engine::movement::MovementConfig::typescript_defaults(),
                1.0 / 60.0,
                profile,
            )
            .unwrap();
            let plan = graph_plan(sensor.layout().input_size);
            let (world, brains, population) = fixture_with_count(&plan, EPOCH, 12);
            let source_world = world.clone();
            let source_brains = brains.clone();
            let indexed_world = indexed(&world);
            let mut hierarchy = BodyRayIndexWorkspace::default();
            let body_rays = hierarchy.prepare(indexed_world.body_index()).unwrap();
            let lives = EvolvedGenerationState::default();
            let grants: Vec<_> = (0..12).map(|index| 40 + index * 7).collect();
            let mut shared_heads =
                crate::engine::sensors_v4::V4HeadWorkspace::try_new(world.snakes.len()).unwrap();
            let heads = shared_heads.prepare(&world, profile).unwrap();
            let context = PreparedV4Sensing {
                body_rays,
                heads,
                context: V4ObservationContext {
                    generation_elapsed_seconds: 10.0,
                    evolved: &lives,
                    experiment: EvolvedExperimentConfig::legacy(240.0),
                },
                grants_by_snake: &grants,
            };
            let mut generation = SensorGenerationState::new();
            generation.update_after_step(&world).unwrap();
            let mut expected = None;
            let mut expected_trace = None;
            for workers in [1, 2, 3, 4] {
                // Twelve due evaluations exceed both parallel-phase thresholds.
                assert!(workers == 1 || 12 >= 2 * workers);
                let mut pipeline = NeuralControlPipeline::try_new(
                    12,
                    sensor.clone(),
                    plan.clone(),
                    workers,
                    usize::MAX,
                )
                .unwrap();
                let candidates: Vec<_> = (0..12)
                    .rev()
                    .map(|index| CalculationCandidateIndex::new(index, index))
                    .collect();
                let mut scratch = SensorScratch::default();
                let mut retained = None;
                for repeat in 0..4 {
                    let selected_id = world.snakes[9].id;
                    let mut trace = crate::engine::sensor_trace::SensorTrace::default();
                    let trace_request = (repeat != 0).then_some((selected_id, &mut trace));
                    let batch = pipeline
                        .prepare_and_evaluate_traced(
                            NeuralControlBatchInputs {
                                key: CalculationBatchKey::new(7, 101, EPOCH),
                                candidates: &candidates,
                                indexed_world: &indexed_world,
                                generation: &generation,
                                assessment_points: None,
                                v4: Some(context),
                                population: &population,
                                brains: &brains,
                                reset_brains: &[],
                            },
                            &mut scratch,
                            trace_request,
                        )
                        .unwrap();
                    assert_eq!(batch.work().len(), 12);
                    let work = batch.cost_micros().work;
                    assert_eq!(work.neural_deliveries, 12);
                    assert_eq!(work.sensors.body_candidates.samples, 12);
                    assert_eq!(work.sensors.ray_candidates.samples, 12);
                    assert_eq!(
                        work.sensors.body_candidates.total,
                        batch
                            .diagnostics()
                            .iter()
                            .map(|d| d.body_query.candidates as u64)
                            .sum::<u64>()
                    );
                    assert_eq!(
                        work.sensors.ray_candidates.total,
                        batch
                            .diagnostics()
                            .iter()
                            .map(|d| d.ray_work.candidates as u64)
                            .sum::<u64>()
                    );
                    assert_eq!(
                        work.sensors.body_candidates.maximum,
                        batch
                            .diagnostics()
                            .iter()
                            .map(|d| d.body_query.candidates as u64)
                            .max()
                            .unwrap()
                    );

                    for (index, diagnostics) in batch.diagnostics().iter().enumerate() {
                        assert!(diagnostics.ray_work.charged() <= u64::from(grants[index]));
                    }
                    let result = (
                        batch
                            .observations()
                            .iter()
                            .map(|value| value.to_bits())
                            .collect::<Vec<_>>(),
                        batch
                            .outputs()
                            .iter()
                            .map(|value| value.to_bits())
                            .collect::<Vec<_>>(),
                        batch.diagnostics().to_vec(),
                        pipeline.staged_recurrent.clone(),
                    );
                    if let Some(expected) = &expected {
                        assert_eq!(&result, expected);
                    } else {
                        expected = Some(result);
                    }
                    if repeat != 0 {
                        assert!(trace.valid);
                        assert_eq!(trace.physical_id, selected_id);
                        assert_eq!((trace.profile, trace.bins), (profile, bins));
                        if let Some(expected) = &expected_trace {
                            assert_eq!(&trace, expected);
                        } else {
                            expected_trace = Some(trace);
                        }
                    }
                    let capacity = (
                        pipeline.capacity_diagnostics(),
                        scratch.diagnostics(),
                        pipeline
                            .parallel_sensor_scratch
                            .iter()
                            .map(SensorScratch::diagnostics)
                            .collect::<Vec<_>>(),
                    );
                    if let Some(retained) = &retained {
                        assert_eq!(&capacity, retained);
                    } else {
                        retained = Some(capacity);
                    }
                    assert_eq!(world, source_world);
                    assert_eq!(brains, source_brains);
                }
            }
        }
    }
}

#[test]
fn warmed_repeated_batches_reuse_pipeline_and_sensor_capacities() {
    let plan = graph_plan(51);
    let (mut world, mut brains, population) = fixture(&plan, EPOCH);
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let mut pipeline = pipeline(plan);
    let mut sensor_scratch = SensorScratch::default();
    let due = candidates();
    let mut retained = None;

    for step in 1..=24_u64 {
        let key = CalculationBatchKey::new(7, step, EPOCH);
        let current_indexed = indexed(&world);
        pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &due,
                    indexed_world: &current_indexed,
                    generation: &generation,
                    assessment_points: None,
                    v4: None,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[],
                },
                &mut sensor_scratch,
            )
            .unwrap();
        drop(current_indexed);
        pipeline.commit_state(key, &mut world, &mut brains).unwrap();

        let capacities = (
            pipeline.capacity_diagnostics(),
            sensor_scratch.diagnostics(),
        );
        if let Some(expected) = retained {
            assert_eq!(capacities, expected);
        } else {
            retained = Some(capacities);
        }
    }
}
