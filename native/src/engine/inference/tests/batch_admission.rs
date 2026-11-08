use super::*;

#[test]
fn one_operation_evaluates_55_distinct_genomes_without_mutating_state_before_commit() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let (snakes, mut brains, population) = population_fixture(&plan, 55);
    let original_state = brains
        .iter()
        .map(|brain| brain.recurrent.clone())
        .collect::<Vec<_>>();
    let order = (0..55).rev().collect::<Vec<_>>();
    let mut workspace = prepared_workspace(&plan, &order, &snakes, &brains, &population);
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace.execution_buffers().unwrap();
    let mut observations = Vec::with_capacity(work.len() * plan.input_size());
    for unit in work.iter() {
        let slot = unit.population_slot().unwrap() as f32;
        observations.extend_from_slice(&[slot / 55.0, -(slot + 1.0) / 56.0]);
    }
    let mut outputs = vec![f32::NAN; work.len() * plan.output_size()];
    let mut next_state = vec![f32::NAN; work.len() * plan.total_state_size()];
    evaluate_heterogeneous_population(
        &plan,
        work,
        &population,
        &brains,
        HeterogeneousInferenceBuffers {
            observations: &observations,
            staged_outputs: &mut outputs,
            staged_recurrent: &mut next_state,
        },
        &mut scratches[0].view(),
    )
    .unwrap();

    assert!(brains
        .iter()
        .zip(&original_state)
        .all(|(brain, before)| brain.recurrent.as_ref() == before.as_ref()));
    let distinct = outputs
        .chunks_exact(plan.output_size())
        .map(|pair| (pair[0].to_bits(), pair[1].to_bits()))
        .collect::<BTreeSet<_>>();
    assert_eq!(distinct.len(), 55);

    commit_heterogeneous_recurrent(&plan, work, &mut brains, &next_state).unwrap();
    for (work_index, unit) in work.iter().enumerate() {
        let offset = work_index * plan.total_state_size();
        assert_eq!(
            &*brains[unit.brain_index()].recurrent,
            &next_state[offset..offset + plan.total_state_size()]
        );
    }
}

#[test]
fn batch_weight_resolution_matches_55_direct_brains_with_equal_inputs_and_state() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let (snakes, mut brains, population) = population_fixture(&plan, 55);
    for brain in &mut brains {
        brain.recurrent.fill(0.0);
    }
    let mut workspace = prepared_workspace(
        &plan,
        &(0..55).rev().collect::<Vec<_>>(),
        &snakes,
        &brains,
        &population,
    );
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace.execution_buffers().unwrap();
    let observations = [0.25, -0.75].repeat(work.len());
    let mut outputs = vec![0.0; work.len() * plan.output_size()];
    let mut next_state = vec![0.0; work.len() * plan.total_state_size()];
    evaluate_heterogeneous_population(
        &plan,
        work,
        &population,
        &brains,
        HeterogeneousInferenceBuffers {
            observations: &observations,
            staged_outputs: &mut outputs,
            staged_recurrent: &mut next_state,
        },
        &mut scratches[0].view(),
    )
    .unwrap();
    let distinct = outputs
        .chunks_exact(plan.output_size())
        .map(|pair| (pair[0].to_bits(), pair[1].to_bits()))
        .collect::<BTreeSet<_>>();
    assert_eq!(distinct.len(), 55);

    let mut direct_scratch = scratch(&plan);
    for (work_index, unit) in work.iter().enumerate() {
        let slot = usize::try_from(unit.population_slot().unwrap()).unwrap();
        let mut expected_output = vec![0.0; plan.output_size()];
        let mut expected_state = vec![0.0; plan.total_state_size()];
        plan.evaluate(
            &population[slot].weights,
            &brains[unit.brain_index()].recurrent,
            &mut expected_state,
            &[0.25, -0.75],
            &mut expected_output,
            &mut direct_scratch.view(),
        )
        .unwrap();
        let output_offset = work_index * plan.output_size();
        let state_offset = work_index * plan.total_state_size();
        assert_eq!(
            &outputs[output_offset..output_offset + plan.output_size()],
            expected_output
        );
        assert_eq!(
            &next_state[state_offset..state_offset + plan.total_state_size()],
            expected_state
        );
    }
}

#[test]
fn shrinking_due_lists_keep_recurrent_state_attached_to_stable_brain_handles() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let (snakes, mut brains, population) = population_fixture(&plan, 55);
    let untouched_before = brains[3].recurrent.clone();
    let mut workspace = prepared_workspace(&plan, &[54, 2, 17], &snakes, &brains, &population);
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace.execution_buffers().unwrap();
    assert_eq!(
        work.iter()
            .map(|unit| unit.population_slot().unwrap())
            .collect::<Vec<_>>(),
        [2, 17, 54]
    );
    let observations = [0.2, -0.2, 0.4, -0.4, 0.6, -0.6];
    let mut outputs = vec![0.0; work.len() * plan.output_size()];
    let mut next_state = vec![0.0; work.len() * plan.total_state_size()];
    evaluate_heterogeneous_population(
        &plan,
        work,
        &population,
        &brains,
        HeterogeneousInferenceBuffers {
            observations: &observations,
            staged_outputs: &mut outputs,
            staged_recurrent: &mut next_state,
        },
        &mut scratches[0].view(),
    )
    .unwrap();
    commit_heterogeneous_recurrent(&plan, work, &mut brains, &next_state).unwrap();
    assert_eq!(brains[3].recurrent, untouched_before);
    assert_ne!(brains[2].recurrent, untouched_before);
}

#[test]
fn entity_owned_resurrected_brain_uses_its_own_weights_and_exact_owner() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let handle = BrainHandle {
        id: 900,
        epoch: EPOCH,
    };
    let mut entity_snake = snake(0, handle);
    entity_snake.id = 77_777;
    entity_snake.kind = SnakeKind::Resurrected;
    entity_snake.population_slot = None;
    let snakes = vec![entity_snake];
    let mut brains = vec![BrainRuntimeState {
        handle,
        owner: BrainOwner::Entity(77_777),
        retained_origin: None,
        non_population_weights: Some(parity_weights(plan.total_parameters()).into_boxed_slice()),
        recurrent: vec![0.0; plan.total_state_size()].into_boxed_slice(),
    }];
    let mut workspace = prepared_workspace(&plan, &[0], &snakes, &brains, &[]);
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace.execution_buffers().unwrap();
    let mut outputs = vec![0.0; plan.output_size()];
    let mut next_state = vec![0.0; plan.total_state_size()];
    evaluate_heterogeneous_population(
        &plan,
        work,
        &[],
        &brains,
        HeterogeneousInferenceBuffers {
            observations: &[0.25, -0.75],
            staged_outputs: &mut outputs,
            staged_recurrent: &mut next_state,
        },
        &mut scratches[0].view(),
    )
    .unwrap();
    assert_close(
        "resurrected output",
        &outputs,
        &[0.024_871_822, -0.226_584_79],
    );
    commit_heterogeneous_recurrent(&plan, work, &mut brains, &next_state).unwrap();
    assert_eq!(brains[0].recurrent.as_ref(), next_state);

    let committed = brains[0].recurrent.clone();
    let mut invalid = next_state;
    invalid[0] = f32::NAN;
    assert!(matches!(
        commit_heterogeneous_recurrent(&plan, work, &mut brains, &invalid),
        Err(InferenceError::NonFiniteValue {
            buffer: "commit recurrent staging",
            index: 0,
            ..
        })
    ));
    assert_eq!(brains[0].recurrent, committed);

    brains[0].owner = BrainOwner::Entity(77_778);
    let mut rejected_outputs = vec![0.0; plan.output_size()];
    let mut rejected_state = vec![0.0; plan.total_state_size()];
    assert!(matches!(
        evaluate_heterogeneous_population(
            &plan,
            work,
            &[],
            &brains,
            HeterogeneousInferenceBuffers {
                observations: &[0.25, -0.75],
                staged_outputs: &mut rejected_outputs,
                staged_recurrent: &mut rejected_state,
            },
            &mut scratches[0].view(),
        ),
        Err(InferenceError::BrainOwnerMismatch { .. })
    ));
}

#[test]
fn malformed_late_genome_fails_preflight_without_staging_or_authoritative_mutation() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let (snakes, brains, mut population) = population_fixture(&plan, 3);
    population[2].weights = vec![0.0; plan.total_parameters() - 1].into_boxed_slice();
    let before = brains
        .iter()
        .map(|brain| brain.recurrent.clone())
        .collect::<Vec<_>>();
    let mut workspace = prepared_workspace(&plan, &[0, 1, 2], &snakes, &brains, &population);
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace.execution_buffers().unwrap();
    let observations = vec![0.25; work.len() * plan.input_size()];
    let mut outputs = vec![123.0; work.len() * plan.output_size()];
    let mut next_state = vec![456.0; work.len() * plan.total_state_size()];
    let error = evaluate_heterogeneous_population(
        &plan,
        work,
        &population,
        &brains,
        HeterogeneousInferenceBuffers {
            observations: &observations,
            staged_outputs: &mut outputs,
            staged_recurrent: &mut next_state,
        },
        &mut scratches[0].view(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        InferenceError::BufferLength {
            buffer: "brain weights",
            ..
        }
    ));
    assert_eq!(outputs, vec![123.0; work.len() * plan.output_size()]);
    assert_eq!(
        next_state,
        vec![456.0; work.len() * plan.total_state_size()]
    );
    assert!(brains
        .iter()
        .zip(before)
        .all(|(brain, prior)| brain.recurrent == prior));
}

#[test]
fn nonfinite_late_observation_fails_before_any_staging_write() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let (snakes, brains, population) = population_fixture(&plan, 3);
    let mut workspace = prepared_workspace(&plan, &[0, 1, 2], &snakes, &brains, &population);
    let CalculationExecutionBuffers {
        work, scratches, ..
    } = workspace.execution_buffers().unwrap();
    let mut observations = vec![0.25; work.len() * plan.input_size()];
    *observations.last_mut().unwrap() = f32::NAN;
    let mut outputs = vec![123.0; work.len() * plan.output_size()];
    let mut next_state = vec![456.0; work.len() * plan.total_state_size()];
    assert!(matches!(
        evaluate_heterogeneous_population(
            &plan,
            work,
            &population,
            &brains,
            HeterogeneousInferenceBuffers {
                observations: &observations,
                staged_outputs: &mut outputs,
                staged_recurrent: &mut next_state,
            },
            &mut scratches[0].view(),
        ),
        Err(InferenceError::NonFiniteValue {
            buffer: "batch observations",
            ..
        })
    ));
    assert_eq!(outputs, vec![123.0; work.len() * plan.output_size()]);
    assert_eq!(
        next_state,
        vec![456.0; work.len() * plan.total_state_size()]
    );
}

#[test]
fn exact_lengths_and_admitted_scratch_are_enforced() {
    let graph = complete_graph();
    let plan = GraphExecutionPlan::build(&graph).unwrap();
    let weights = parity_weights(plan.total_parameters());
    let state = vec![0.0; plan.total_state_size()];
    let mut next_state = vec![0.0; plan.total_state_size()];
    let mut output = vec![0.0; plan.output_size()];
    let mut too_small = CalculationScratch::try_new(
        CalculationScratchLayout {
            activation_floats: plan.scratch_layout().activation_floats - 1,
            ..plan.scratch_layout()
        },
        usize::MAX,
    )
    .unwrap();
    assert!(matches!(
        plan.evaluate(
            &weights,
            &state,
            &mut next_state,
            &[0.0, 0.0],
            &mut output,
            &mut too_small.view(),
        ),
        Err(InferenceError::ScratchTooSmall {
            buffer: "activation scratch",
            ..
        })
    ));

    let mut execution_scratch = scratch(&plan);
    assert!(matches!(
        plan.evaluate(
            &weights[..weights.len() - 1],
            &state,
            &mut next_state,
            &[0.0, 0.0],
            &mut output,
            &mut execution_scratch.view(),
        ),
        Err(InferenceError::BufferLength {
            buffer: "weights",
            ..
        })
    ));
    assert!(matches!(
        plan.evaluate(
            &weights,
            &state,
            &mut next_state,
            &[0.0],
            &mut output,
            &mut execution_scratch.view(),
        ),
        Err(InferenceError::BufferLength {
            buffer: "graph input",
            ..
        })
    ));
    assert!(matches!(
        plan.evaluate(
            &weights,
            &state,
            &mut next_state,
            &[f32::NAN, 0.0],
            &mut output,
            &mut execution_scratch.view(),
        ),
        Err(InferenceError::NonFiniteValue {
            buffer: "graph input",
            index: 0,
            ..
        })
    ));
}

#[test]
fn execution_planning_rejects_tampered_parameter_and_recurrent_ranges() {
    let mut parameter_gap = complete_graph();
    parameter_gap.nodes[2].parameter_offset += 1;
    assert!(matches!(
        GraphExecutionPlan::build(&parameter_gap),
        Err(InferenceError::InvalidCompiledGraph { .. })
    ));

    let mut wrong_state_shape = complete_graph();
    let gru = wrong_state_shape
        .nodes
        .iter_mut()
        .find(|node| node.node_type == CompiledNodeType::Gru)
        .unwrap();
    gru.state_length += 1;
    assert!(matches!(
        GraphExecutionPlan::build(&wrong_state_shape),
        Err(InferenceError::InvalidCompiledGraph { .. })
    ));
}
