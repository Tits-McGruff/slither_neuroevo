use super::*;

#[test]
fn complete_batch_stages_observations_outputs_and_commits_both_boundaries() {
    let plan = graph_plan(51);
    let capture = plan.prepare_activation_capture("memory").unwrap();
    let (mut world, mut brains, population) = fixture(&plan, EPOCH);
    let recurrent_before = brains
        .iter()
        .map(|brain| brain.recurrent.clone())
        .collect::<Vec<_>>();
    let delivered_before = world
        .snakes
        .iter()
        .map(|snake| snake.delivered_observation_points)
        .collect::<Vec<_>>();
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let old_indexed = indexed(&world);
    let mut sensor_scratch = SensorScratch::default();
    let mut pipeline = pipeline(plan);
    let key = CalculationBatchKey::new(4, 99, EPOCH);
    let batch = pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &candidates(),
                indexed_world: &old_indexed,
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
    assert_eq!(batch.key(), key);
    assert_eq!(batch.work().len(), 3);
    assert_eq!(batch.observations().len(), 3 * 51);
    assert_eq!(batch.outputs().len(), 3 * 2);
    assert!(batch.outputs().iter().all(|value| value.is_finite()));
    assert_eq!(batch.diagnostics().len(), 3);
    assert_eq!(
        world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>(),
        delivered_before
    );
    assert!(brains
        .iter()
        .zip(&recurrent_before)
        .all(|(brain, before)| brain.recurrent == *before));

    let focused = batch.work()[1].brain();
    let outputs_before_capture = batch.outputs().to_vec();
    let _ = batch;
    let mut captured = vec![f32::NAN; capture.len()];
    pipeline
        .capture_focused_activation(focused, &capture, &mut captured, &population, &brains)
        .unwrap();
    assert!(captured.iter().all(|value| value.is_finite()));
    assert_eq!(pipeline.batch().unwrap().outputs(), outputs_before_capture);

    drop(old_indexed);
    pipeline.commit_state(key, &mut world, &mut brains).unwrap();
    for snake in &world.snakes {
        assert_eq!(snake.delivered_observation_points, snake.points);
    }
    assert!(brains
        .iter()
        .zip(recurrent_before)
        .all(|(brain, before)| brain.recurrent != before));
    assert!(matches!(
        pipeline.batch(),
        Err(NeuralControlError::BatchNotReady)
    ));
}

#[test]
fn explicit_takeover_reset_matches_a_zero_state_brain_without_touching_others() {
    let plan = graph_plan(51);
    let capture = plan.prepare_activation_capture("memory").unwrap();
    let (mut reset_world, mut reset_brains, population) = fixture(&plan, EPOCH);
    let mut reference_world = reset_world.clone();
    let mut reference_brains = reset_brains.clone();
    let reset_handle = reset_brains[1].handle;
    let source_reset_state = reset_brains[1].recurrent.clone();
    reference_brains[1].recurrent.fill(0.0);
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&reset_world).unwrap();
    let reset_indexed = indexed(&reset_world);
    let reference_indexed = indexed(&reference_world);
    let mut reset_pipeline = pipeline(plan.clone());
    let mut reference_pipeline = pipeline(plan);
    let mut reset_sensor_scratch = SensorScratch::default();
    let mut reference_sensor_scratch = SensorScratch::default();
    let key = CalculationBatchKey::new(4, 100, EPOCH);

    let reset_outputs = reset_pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &candidates(),
                indexed_world: &reset_indexed,
                generation: &generation,
                assessment_points: None,
                v4: None,
                population: &population,
                brains: &reset_brains,
                reset_brains: &[reset_handle],
            },
            &mut reset_sensor_scratch,
        )
        .unwrap()
        .outputs()
        .to_vec();
    let reference_outputs = reference_pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &candidates(),
                indexed_world: &reference_indexed,
                generation: &generation,
                assessment_points: None,
                v4: None,
                population: &population,
                brains: &reference_brains,
                reset_brains: &[],
            },
            &mut reference_sensor_scratch,
        )
        .unwrap()
        .outputs()
        .to_vec();
    assert_eq!(reset_outputs, reference_outputs);
    assert_eq!(reset_brains[1].recurrent, source_reset_state);

    let mut reset_activation = vec![f32::NAN; capture.len()];
    let mut reference_activation = vec![f32::NAN; capture.len()];
    reset_pipeline
        .capture_focused_activation(
            reset_handle,
            &capture,
            &mut reset_activation,
            &population,
            &reset_brains,
        )
        .unwrap();
    reference_pipeline
        .capture_focused_activation(
            reset_handle,
            &capture,
            &mut reference_activation,
            &population,
            &reference_brains,
        )
        .unwrap();
    assert_eq!(reset_activation, reference_activation);

    drop(reset_indexed);
    drop(reference_indexed);
    reset_pipeline
        .commit_state(key, &mut reset_world, &mut reset_brains)
        .unwrap();
    reference_pipeline
        .commit_state(key, &mut reference_world, &mut reference_brains)
        .unwrap();
    assert_eq!(reset_world, reference_world);
    assert_eq!(reset_brains, reference_brains);
    assert_ne!(reset_brains[1].recurrent, source_reset_state);
}

#[test]
fn invalid_reset_sets_reject_without_ready_batch_or_authoritative_writes() {
    let plan = graph_plan(51);
    let (world, brains, population) = fixture(&plan, EPOCH);
    let source_world = world.clone();
    let source_brains = brains.clone();
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let indexed = indexed(&world);
    let mut pipeline = pipeline(plan);
    let mut sensor_scratch = SensorScratch::default();
    let key = CalculationBatchKey::new(4, 101, EPOCH);
    let mut handles = brains.iter().map(|brain| brain.handle).collect::<Vec<_>>();
    handles.sort_unstable();

    for invalid in [vec![handles[0], handles[0]], vec![handles[2], handles[0]]] {
        assert!(matches!(
            pipeline.prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &indexed,
                    generation: &generation,
                    assessment_points: None,
                    v4: None,
                    population: &population,
                    brains: &brains,
                    reset_brains: &invalid,
                },
                &mut sensor_scratch,
            ),
            Err(NeuralControlError::NonCanonicalResetBrains { .. })
        ));
        assert!(matches!(
            pipeline.batch(),
            Err(NeuralControlError::BatchNotReady)
        ));
        assert_eq!(world, source_world);
        assert_eq!(brains, source_brains);
    }

    let stale = BrainHandle {
        id: u64::MAX - 1,
        epoch: EPOCH,
    };
    assert!(matches!(
        pipeline.prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &candidates(),
                indexed_world: &indexed,
                generation: &generation,
            assessment_points: None,
            v4: None,
                population: &population,
                brains: &brains,
                reset_brains: &[stale],
            },
            &mut sensor_scratch,
        ),
        Err(NeuralControlError::ResetBrainNotInBatch { brain }) if brain == stale
    ));
    assert!(matches!(
        pipeline.batch(),
        Err(NeuralControlError::BatchNotReady)
    ));
    assert_eq!(world, source_world);
    assert_eq!(brains, source_brains);
}

#[test]
fn late_delivery_failure_prevents_every_delivery_and_recurrent_write() {
    let plan = graph_plan(51);
    let (mut world, mut brains, population) = fixture(&plan, EPOCH);
    let recurrent_before = brains
        .iter()
        .map(|brain| brain.recurrent.clone())
        .collect::<Vec<_>>();
    let delivered_before = world
        .snakes
        .iter()
        .map(|snake| snake.delivered_observation_points)
        .collect::<Vec<_>>();
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let original_indexed = indexed(&world);
    let mut pipeline = pipeline(plan);
    let mut sensor_scratch = SensorScratch::default();
    let key = CalculationBatchKey::new(4, 100, EPOCH);
    pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &candidates(),
                indexed_world: &original_indexed,
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
    drop(original_indexed);
    world.snakes[2].delivered_observation_points += 0.5;

    assert!(matches!(
        pipeline.commit_state(key, &mut world, &mut brains),
        Err(NeuralControlError::Sensor(error))
            if matches!(*error, SensorError::StaleDeliveryMarker { .. })
    ));
    assert_eq!(
        world.snakes[0].delivered_observation_points,
        delivered_before[0]
    );
    assert_eq!(
        world.snakes[1].delivered_observation_points,
        delivered_before[1]
    );
    assert_eq!(
        world.snakes[2].delivered_observation_points,
        delivered_before[2] + 0.5
    );
    assert!(brains
        .iter()
        .zip(recurrent_before)
        .all(|(brain, before)| brain.recurrent == before));
}

#[test]
fn malformed_late_genome_leaves_no_ready_batch_or_authoritative_write() {
    let plan = graph_plan(51);
    let (world, brains, mut population) = fixture(&plan, EPOCH);
    population[2].weights = vec![0.0; plan.total_parameters() - 1].into_boxed_slice();
    let recurrent_before = brains
        .iter()
        .map(|brain| brain.recurrent.clone())
        .collect::<Vec<_>>();
    let delivered_before = world
        .snakes
        .iter()
        .map(|snake| snake.delivered_observation_points)
        .collect::<Vec<_>>();
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let original_indexed = indexed(&world);
    let mut pipeline = pipeline(plan);
    let mut sensor_scratch = SensorScratch::default();
    let key = CalculationBatchKey::new(4, 101, EPOCH);

    assert!(matches!(
        pipeline.prepare_and_evaluate(
            NeuralControlBatchInputs {
                key,
                candidates: &candidates(),
                indexed_world: &original_indexed,
                generation: &generation,
            assessment_points: None,
            v4: None,
                population: &population,
                brains: &brains,
                reset_brains: &[],
            },
            &mut sensor_scratch,
        ),
        Err(NeuralControlError::Inference(error))
            if matches!(*error, InferenceError::BufferLength {
                buffer: "brain weights",
                ..
            })
    ));
    assert!(matches!(
        pipeline.batch(),
        Err(NeuralControlError::BatchNotReady)
    ));
    assert_eq!(
        world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>(),
        delivered_before
    );
    assert!(brains
        .iter()
        .zip(recurrent_before)
        .all(|(brain, before)| brain.recurrent == before));
}

#[test]
fn population_replacement_rejects_old_epoch_then_accepts_zeroed_new_epoch() {
    let plan = graph_plan(51);
    let (world, brains, population) = fixture(&plan, EPOCH);
    let mut generation = SensorGenerationState::new();
    generation.update_after_step(&world).unwrap();
    let original_indexed = indexed(&world);
    let mut pipeline = pipeline(plan.clone());
    let mut sensor_scratch = SensorScratch::default();
    let old_key = CalculationBatchKey::new(5, 1, EPOCH);
    pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key: old_key,
                candidates: &candidates(),
                indexed_world: &original_indexed,
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
    drop(original_indexed);

    let new_epoch = EPOCH + 1;
    let mut replacement_world = world.clone();
    let mut replacement_brains = brains.clone();
    let mut replacement_population = population.clone();
    for index in 0..replacement_brains.len() {
        let handle = BrainHandle {
            id: replacement_brains[index].handle.id + 10_000,
            epoch: new_epoch,
        };
        replacement_brains[index].handle = handle;
        replacement_brains[index].recurrent.fill(0.0);
        replacement_population[index].brain = handle;
        replacement_world.snakes[index].brain = Some(handle);
    }
    let replacement_recurrent = replacement_brains
        .iter()
        .map(|brain| brain.recurrent.clone())
        .collect::<Vec<_>>();
    let replacement_delivered = replacement_world
        .snakes
        .iter()
        .map(|snake| snake.delivered_observation_points)
        .collect::<Vec<_>>();
    let new_key = CalculationBatchKey::new(6, 1, new_epoch);
    assert!(matches!(
        pipeline.commit_state(new_key, &mut replacement_world, &mut replacement_brains),
        Err(NeuralControlError::BatchKeyMismatch { .. })
    ));
    assert_eq!(
        replacement_world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>(),
        replacement_delivered
    );
    assert!(replacement_brains
        .iter()
        .zip(&replacement_recurrent)
        .all(|(brain, before)| brain.recurrent == *before));
    assert!(replacement_brains
        .iter()
        .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));

    assert!(matches!(
        pipeline.commit_state(
            old_key,
            &mut replacement_world,
            &mut replacement_brains
        ),
        Err(NeuralControlError::Inference(error))
            if matches!(*error, InferenceError::BrainHandleMismatch { .. })
    ));
    assert_eq!(
        replacement_world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>(),
        replacement_delivered
    );
    assert!(replacement_brains
        .iter()
        .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));

    let mut replacement_generation = SensorGenerationState::new();
    replacement_generation
        .update_after_step(&replacement_world)
        .unwrap();
    let replacement_indexed = indexed(&replacement_world);
    pipeline
        .prepare_and_evaluate(
            NeuralControlBatchInputs {
                key: new_key,
                candidates: &candidates(),
                indexed_world: &replacement_indexed,
                generation: &replacement_generation,
                assessment_points: None,
                v4: None,
                population: &replacement_population,
                brains: &replacement_brains,
                reset_brains: &[],
            },
            &mut sensor_scratch,
        )
        .unwrap();
    drop(replacement_indexed);
    pipeline
        .commit_state(new_key, &mut replacement_world, &mut replacement_brains)
        .unwrap();
    assert!(replacement_world
        .snakes
        .iter()
        .all(|snake| snake.delivered_observation_points == snake.points));
    assert!(replacement_brains
        .iter()
        .zip(replacement_recurrent)
        .all(|(brain, before)| brain.recurrent != before));
}
