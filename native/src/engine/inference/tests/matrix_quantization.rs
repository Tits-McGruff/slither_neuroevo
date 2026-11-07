use super::*;

#[test]
fn matrix_i16_complete_dense_mlp_and_recurrent_sequences_match_f32() {
    if !InferenceMathBackend::I16MatrixFma128.is_available() {
        return;
    }
    assert_eq!(
        InferenceMathBackend::from_label("rust-i16-matrix-fma128-v1"),
        Some(InferenceMathBackend::I16MatrixFma128)
    );
    assert_eq!(InferenceMathBackend::detect(), InferenceMathBackend::Sse2);
    for graph in [
        complete_graph(),
        wide_recurrent_graph(),
        large_recurrent_graph(),
    ] {
        let reference =
            GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)
                .unwrap();
        let plan = GraphExecutionPlan::build_with_math_backend(
            &graph,
            InferenceMathBackend::I16MatrixFma128,
        )
        .unwrap();
        let weights = parity_weights(plan.total_parameters());
        let original = weights.clone();
        let cache = plan.quantize(&weights).unwrap();
        assert_eq!(
            cache.allocated_bytes(),
            weights.len() * 2 + cache.matrix_count() * 4
        );
        assert_eq!(cache.matrix_count(), plan.matrices.len());
        let mut reference_scratch = scratch(&reference);
        let mut quantized_scratch = scratch(&plan);
        let mut reference_state = vec![0.03; plan.total_state_size()];
        let mut quantized_state = reference_state.clone();
        let mut output_error = 0.0_f64;
        let mut state_error = 0.0_f64;
        let mut output_count = 0;
        let mut state_count = 0;
        for step in 0..128 {
            if step == 64 {
                reference_state.fill(0.0);
                quantized_state.fill(0.0);
            }
            let input: Vec<f32> = (0..plan.input_size())
                .map(|i| ((step * 17 + i * 11) % 97) as f32 / 48.0 - 1.0)
                .collect();
            let mut ref_next = vec![0.0; plan.total_state_size()];
            let mut q_next = ref_next.clone();
            let mut ref_output = vec![0.0; 2];
            let mut q_output = ref_output.clone();
            reference
                .evaluate(
                    &weights,
                    &reference_state,
                    &mut ref_next,
                    &input,
                    &mut ref_output,
                    &mut reference_scratch.view(),
                )
                .unwrap();
            plan.evaluate_quantized(
                &weights,
                &cache,
                &quantized_state,
                &mut q_next,
                &input,
                &mut q_output,
                &mut quantized_scratch.view(),
            )
            .unwrap();
            let mut repeated = vec![0.0; 2];
            let mut repeated_state = q_next.clone();
            plan.evaluate_quantized(
                &weights,
                &cache,
                &quantized_state,
                &mut repeated_state,
                &input,
                &mut repeated,
                &mut quantized_scratch.view(),
            )
            .unwrap();
            assert_eq!(q_output, repeated);
            assert_eq!(q_next, repeated_state);
            for (&a, &b) in q_output.iter().zip(&ref_output) {
                let e = (a - b).abs();
                assert!(e <= 0.0006, "controller error {e}");
                output_error += f64::from(e);
                output_count += 1;
            }
            for (&a, &b) in q_next.iter().zip(&ref_next) {
                let e = (a - b).abs();
                assert!(e <= 0.0001, "recurrent error {e}");
                state_error += f64::from(e);
                state_count += 1;
            }
            reference_state = ref_next;
            quantized_state = q_next;
        }
        assert!(output_error / output_count as f64 <= 0.00005);
        assert!(state_error / state_count as f64 <= 0.00001);
        assert_eq!(weights, original);
        let mut invalid = weights.clone();
        invalid[0] = f32::NAN;
        assert!(plan.quantize(&invalid).is_err());
        assert!(plan.quantize(&weights[..weights.len() - 1]).is_err());
        assert!(plan
            .evaluate(
                &weights,
                &reference_state,
                &mut quantized_state,
                &vec![0.0; plan.input_size()],
                &mut [0.0; 2],
                &mut quantized_scratch.view()
            )
            .is_err());
    }
}

#[test]
fn matrix_i16_binding_reuses_caches_and_refreshes_changed_ownership() {
    if !InferenceMathBackend::I16MatrixFma128.is_available() {
        return;
    }
    for backend in [
        InferenceMathBackend::I16MatrixFma128,
        InferenceMathBackend::I16AffineFma128,
    ] {
        let graph = complete_graph();
        let mut plan = GraphExecutionPlan::build_with_math_backend(&graph, backend).unwrap();
        let (_, mut brains, mut population) = population_fixture(&plan, 3);
        plan.prepare_weight_caches(1, &population, &brains).unwrap();
        let initial = plan.inference_cache_bytes();
        assert!(initial > 0);
        // Identical immutable ownership preserves the cache allocations.
        let address = plan.caches[0].weights.as_ref().unwrap().storage_address();
        plan.prepare_weight_caches(1, &population, &brains).unwrap();
        assert_eq!(
            address,
            plan.caches[0].weights.as_ref().unwrap().storage_address()
        );
        let old = plan.caches[0].weights.clone();
        population[0].weights[0] = 5.0;
        population[0].lineage.genome_id += 1000;
        plan.prepare_weight_caches(2, &population, &brains).unwrap();
        assert_ne!(old, plan.caches[0].weights);
        assert_eq!(initial, plan.inference_cache_bytes());
        // A replaced non-population brain is also quantized, with no population alias.
        brains[0].owner = BrainOwner::Entity(999);
        brains[0].handle.id += 1000;
        brains[0].non_population_weights =
            Some(vec![0.0; plan.total_parameters()].into_boxed_slice());
        plan.prepare_weight_caches(2, &population, &brains).unwrap();
        assert_ne!(plan.caches[0].weights, old);
        assert_eq!(initial, plan.inference_cache_bytes());
        let storage = plan.caches[0].weights.as_ref().unwrap().storage_address();
        let cloned_brains = brains.clone();
        plan.prepare_weight_caches(2, &population, &cloned_brains)
            .unwrap();
        assert_eq!(
            storage,
            plan.caches[0].weights.as_ref().unwrap().storage_address()
        );
        let admitted = i16_cache_admission_bytes(&graph, brains.len(), backend).unwrap();
        assert!(admitted >= initial);
    }
}

#[test]
fn matrix_i16_domains_preserve_zero_graph_and_distinct_matrix_scales() {
    if !InferenceMathBackend::I16MatrixFma128.is_available() {
        return;
    }
    let graph = complete_graph();
    let plan =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::I16MatrixFma128)
            .unwrap();
    let mut weights = vec![0.0; plan.total_parameters()];
    let zero = plan.quantize(&weights).unwrap();
    let mut output = [1.0; 2];
    let mut after = vec![1.0; plan.total_state_size()];
    let mut work = scratch(&plan);
    plan.evaluate_quantized(
        &weights,
        &zero,
        &vec![0.0; plan.total_state_size()],
        &mut after,
        &vec![1.0; plan.input_size()],
        &mut output,
        &mut work.view(),
    )
    .unwrap();
    assert_eq!(output, [0.0; 2]);
    assert!(after.iter().all(|v| *v == 0.0));
    // Different major matrices deliberately have very different ranges;
    // every row of one matrix must share its matrix maximum.
    for (ordinal, matrix) in plan.matrices.iter().enumerate() {
        let magnitude = [0.0001_f32, 0.3, 5.0][ordinal % 3];
        weights[matrix.offset..matrix.offset + matrix.len].fill(magnitude / 3.0);
        weights[matrix.offset] = -magnitude;
    }
    let cache = plan.quantize(&weights).unwrap();
    for (ordinal, matrix) in plan.matrices.iter().enumerate() {
        let view = NodeWeights::new(&weights, Some(&cache), *matrix, ordinal);
        let magnitude = [0.0001_f32, 0.3, 5.0][ordinal % 3];
        assert!(
            (view.dot(InferenceMathBackend::I16MatrixFma128, 0..1, &[1.0], 0) + magnitude).abs()
                < 1e-6
        );
        if matrix.len > 1 {
            assert!(
                (view.dot(InferenceMathBackend::I16MatrixFma128, 1..2, &[1.0], 0)
                    - magnitude / 3.0)
                    .abs()
                    <= magnitude / 32767.0
            );
        }
    }
    let incompatible = GraphExecutionPlan::build_with_math_backend(
        &wide_recurrent_graph(),
        InferenceMathBackend::I16MatrixFma128,
    )
    .unwrap();
    assert!(incompatible
        .evaluate_quantized(
            &weights,
            &cache,
            &[],
            &mut [],
            &[],
            &mut [],
            &mut work.view()
        )
        .is_err());
}
