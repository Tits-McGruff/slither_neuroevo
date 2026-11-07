use super::*;

#[test]
fn complete_scalar_graph_matches_current_typescript_for_two_recurrent_steps() {
    let graph = complete_graph();
    assert_eq!(
        graph.order,
        ["in", "split", "denseA", "mlpB", "features", "gru", "lstm", "rru", "memory", "head"]
    );
    assert_eq!(graph.total_parameters, 53);
    assert_eq!(graph.total_state_size, 4);
    let plan =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar).unwrap();
    assert_eq!(plan.math_backend(), InferenceMathBackend::Scalar);
    let weights = parity_weights(plan.total_parameters());
    let zero_state = vec![0.0; plan.total_state_size()];
    let mut first_state = vec![f32::NAN; plan.total_state_size()];
    let mut first_output = vec![f32::NAN; plan.output_size()];
    let mut execution_scratch = scratch(&plan);

    plan.evaluate(
        &weights,
        &zero_state,
        &mut first_state,
        &[0.25, -0.75],
        &mut first_output,
        &mut execution_scratch.view(),
    )
    .unwrap();
    assert_eq!(zero_state, vec![0.0; plan.total_state_size()]);
    assert_close(
        "first output",
        &first_output,
        &[0.024_871_822, -0.226_584_79],
    );
    assert_close(
        "first recurrent state",
        &first_state,
        &[-0.077_934_14, -0.049_185_96, -0.090_083_58, 0.083_280_325],
    );

    let mut second_state = vec![f32::NAN; plan.total_state_size()];
    let mut second_output = vec![f32::NAN; plan.output_size()];
    plan.evaluate(
        &weights,
        &first_state,
        &mut second_state,
        &[-0.4, 0.6],
        &mut second_output,
        &mut execution_scratch.view(),
    )
    .unwrap();
    assert_close(
        "second output",
        &second_output,
        &[0.016_863_106, -0.226_446_15],
    );
    assert_close(
        "second recurrent state",
        &second_state,
        &[-0.098_801_37, -0.066_673_01, -0.123_771, 0.118_772_58],
    );

    let mut reset_state = vec![f32::NAN; plan.total_state_size()];
    let mut reset_output = vec![f32::NAN; plan.output_size()];
    plan.evaluate(
        &weights,
        &zero_state,
        &mut reset_state,
        &[0.25, -0.75],
        &mut reset_output,
        &mut execution_scratch.view(),
    )
    .unwrap();
    assert_eq!(reset_output, first_output);
    assert_eq!(reset_state, first_state);
}

#[test]
fn focused_activation_capture_is_explicit_layout_bound_and_exact() {
    let graph = complete_graph();
    let plan =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar).unwrap();
    let head_capture = plan.prepare_activation_capture("head").unwrap();
    assert_eq!(head_capture.node_id(), "head");
    assert_eq!(head_capture.len(), plan.output_size());
    assert_eq!(head_capture.layer_widths(), &[plan.output_size()]);
    assert!(!head_capture.is_empty());
    assert!(matches!(
        plan.prepare_activation_capture("missing"),
        Err(InferenceError::ActivationNodeNotFound { .. })
    ));

    let weights = parity_weights(plan.total_parameters());
    let mut recurrent_after = vec![0.0; plan.total_state_size()];
    let mut output = vec![0.0; plan.output_size()];
    let mut execution_scratch = scratch(&plan);
    let mut scratch_view = execution_scratch.view();
    plan.evaluate(
        &weights,
        &vec![0.0; plan.total_state_size()],
        &mut recurrent_after,
        &[0.25, -0.75],
        &mut output,
        &mut scratch_view,
    )
    .unwrap();
    let mut captured = vec![f32::NAN; head_capture.len()];
    plan.capture_activation(&head_capture, &scratch_view, &mut captured)
        .unwrap();
    assert_eq!(captured, output);

    let visualization = plan.prepare_visualization_capture().unwrap();
    assert_eq!(
        visualization
            .layers()
            .iter()
            .map(|layer| (layer.count(), layer.has_activations(), layer.is_recurrent()))
            .collect::<Vec<_>>(),
        [
            (1, true, false),
            (1, false, false),
            (2, true, false),
            (1, true, false),
            (1, true, true),
            (1, true, true),
            (1, true, true),
            (2, true, false),
        ]
    );
    let mut visualization_values = vec![f32::NAN; visualization.len()];
    plan.capture_visualization(&visualization, &scratch_view, &mut visualization_values)
        .unwrap();
    assert_eq!(
        &visualization_values[visualization_values.len() - output.len()..],
        output
    );
    assert!(visualization_values.iter().all(|value| value.is_finite()));

    let mlp_capture = plan.prepare_activation_capture("mlpB").unwrap();
    assert_eq!(mlp_capture.layer_widths(), &[2, 1]);
    let mut mlp_values = vec![f32::NAN; mlp_capture.len()];
    plan.capture_activation(&mlp_capture, &scratch_view, &mut mlp_values)
        .unwrap();
    assert!(mlp_values.iter().all(|value| value.is_finite()));

    let other = GraphExecutionPlan::build(&wide_recurrent_graph()).unwrap();
    let mut rejected = vec![0.0; head_capture.len()];
    assert!(matches!(
        other.capture_activation(&head_capture, &scratch_view, &mut rejected),
        Err(InferenceError::ActivationLayoutMismatch { .. })
    ));
    assert!(matches!(
        plan.capture_activation(&head_capture, &scratch_view, &mut [0.0]),
        Err(InferenceError::BufferLength {
            buffer: "activation capture",
            ..
        })
    ));
}

#[test]
fn hidden_width_two_recurrent_indexing_matches_current_typescript_for_two_steps() {
    let graph = wide_recurrent_graph();
    assert_eq!(
        graph.order,
        ["input", "gru", "lstm", "rru", "memory", "head"]
    );
    assert_eq!(graph.total_parameters, 122);
    assert_eq!(graph.total_state_size, 8);
    let plan =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar).unwrap();
    assert_eq!(plan.math_backend(), InferenceMathBackend::Scalar);
    let weights = parity_weights(plan.total_parameters());
    let initial_state = [0.1, -0.2, 0.05, -0.07, 0.2, -0.15, -0.11, 0.09];
    let mut first_state = vec![f32::NAN; plan.total_state_size()];
    let mut first_output = vec![f32::NAN; plan.output_size()];
    let mut execution_scratch = scratch(&plan);
    plan.evaluate(
        &weights,
        &initial_state,
        &mut first_state,
        &[0.25, -0.75, 0.5],
        &mut first_output,
        &mut execution_scratch.view(),
    )
    .unwrap();
    assert_close(
        "wide first output",
        &first_output,
        &[0.106_320_71, -0.039_742_753],
    );
    assert_close(
        "wide first state",
        &first_state,
        &[
            -0.089_948_736,
            -0.128_406_96,
            0.022_501_018,
            -0.014_859_255,
            0.052_411_668,
            -0.026_057_417,
            -0.050_840_516,
            0.106_251_605,
        ],
    );

    let mut second_state = vec![f32::NAN; plan.total_state_size()];
    let mut second_output = vec![f32::NAN; plan.output_size()];
    plan.evaluate(
        &weights,
        &first_state,
        &mut second_state,
        &[-0.4, 0.6, -0.2],
        &mut second_output,
        &mut execution_scratch.view(),
    )
    .unwrap();
    assert_close(
        "wide second output",
        &second_output,
        &[0.202_137_57, -0.116_188_236],
    );
    assert_close(
        "wide second state",
        &second_state,
        &[
            0.019_432_05,
            0.079_606_08,
            -0.091_889_31,
            -0.058_004_79,
            -0.181_314_2,
            -0.120_778_87,
            -0.193_776_22,
            -0.039_415_892,
        ],
    );
}

#[test]
fn runtime_detected_sse2_matches_scalar_across_complete_recurrent_graphs() {
    assert!(InferenceMathBackend::Sse2.is_available());
    assert_eq!(InferenceMathBackend::detect(), InferenceMathBackend::Sse2);
    for graph in [complete_graph(), wide_recurrent_graph()] {
        let scalar =
            GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)
                .unwrap();
        let simd = GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Sse2)
            .unwrap();
        let detected = GraphExecutionPlan::build(&graph).unwrap();
        assert_eq!(detected.math_backend(), InferenceMathBackend::Sse2);
        assert_eq!(scalar.scratch_layout(), simd.scratch_layout());
        let weights = parity_weights(scalar.total_parameters());
        let mut scalar_state = (0..scalar.total_state_size())
            .map(|index| (index as f32 - 3.0) / 29.0)
            .collect::<Vec<_>>();
        let mut simd_state = scalar_state.clone();
        let mut scalar_scratch = scratch(&scalar);
        let mut simd_scratch = scratch(&simd);
        for step in 0..32usize {
            let input = (0..scalar.input_size())
                .map(|index| (((step + 1) * 17 + (index + 3) * 11) % 97) as f32 / 48.0 - 1.0)
                .collect::<Vec<_>>();
            let mut next_scalar = vec![f32::NAN; scalar.total_state_size()];
            let mut next_simd = vec![f32::NAN; simd.total_state_size()];
            let mut scalar_output = vec![f32::NAN; scalar.output_size()];
            let mut simd_output = vec![f32::NAN; simd.output_size()];
            scalar
                .evaluate(
                    &weights,
                    &scalar_state,
                    &mut next_scalar,
                    &input,
                    &mut scalar_output,
                    &mut scalar_scratch.view(),
                )
                .unwrap();
            simd.evaluate(
                &weights,
                &simd_state,
                &mut next_simd,
                &input,
                &mut simd_output,
                &mut simd_scratch.view(),
            )
            .unwrap();
            assert_close("scalar/SSE2 output", &simd_output, &scalar_output);
            assert_close("scalar/SSE2 recurrent", &next_simd, &next_scalar);
            scalar_state = next_scalar;
            simd_state = next_simd;
        }
    }
}

/// Compare all outputs/states over 256 deliveries, nonzero starts and a life reset.
#[test]
fn long_simd_rows_match_scalar_through_large_mixed_recurrence_and_reset() {
    let graph = large_recurrent_graph();
    let scalar =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar).unwrap();
    let simd =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Sse2).unwrap();
    for seed in [1_u64, 7, 42, 4096] {
        // Independent test-only stream; authoritative RNGs are not involved.
        let mut state = seed;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / 16_777_216.0 - 0.5
        };
        let weights = (0..scalar.total_parameters())
            .map(|_| next() * 0.3)
            .collect::<Vec<_>>();
        for initial in [0.0_f32, 0.13] {
            let mut scalar_state = vec![initial; scalar.total_state_size()];
            let mut simd_state = scalar_state.clone();
            let mut scalar_scratch = scratch(&scalar);
            let mut simd_scratch = scratch(&simd);
            for step in 0..256 {
                if step == 128 {
                    scalar_state.fill(0.0);
                    simd_state.fill(0.0);
                }
                let input = (0..scalar.input_size())
                    .map(|_| next() * 2.0)
                    .collect::<Vec<_>>();
                let mut scalar_next = vec![f32::NAN; scalar.total_state_size()];
                let mut simd_next = scalar_next.clone();
                let mut scalar_output = vec![f32::NAN; scalar.output_size()];
                let mut simd_output = scalar_output.clone();
                scalar
                    .evaluate(
                        &weights,
                        &scalar_state,
                        &mut scalar_next,
                        &input,
                        &mut scalar_output,
                        &mut scalar_scratch.view(),
                    )
                    .unwrap();
                simd.evaluate(
                    &weights,
                    &simd_state,
                    &mut simd_next,
                    &input,
                    &mut simd_output,
                    &mut simd_scratch.view(),
                )
                .unwrap();
                assert_close("large scalar/SSE2 output", &simd_output, &scalar_output);
                assert_close("large scalar/SSE2 recurrent", &simd_next, &scalar_next);
                scalar_state = scalar_next;
                simd_state = simd_next;
            }
        }
    }
}

/// Compensation recovers small long-row terms lost by naive Float32 accumulation.
#[test]
fn scalar_long_rows_recover_cancellation_without_widening_products() {
    let mut weights = vec![1.0_f32; 32];
    weights[0] = 100_000_000.0;
    weights[31] = -100_000_000.0;
    let input = vec![1.0; 32];
    let mut naive = 0.0_f32;
    for &weight in &weights {
        naive += weight;
    }
    assert_eq!(naive, 0.0);
    assert_eq!(
        dot_product(InferenceMathBackend::Scalar, &weights, &input),
        30.0
    );
    assert_eq!(
        dot_product_mul(InferenceMathBackend::Scalar, &weights, &input, &input),
        30.0
    );

    let short = [100_000_000.0_f32, 1.0, -100_000_000.0];
    assert_eq!(
        dot_product(InferenceMathBackend::Scalar, &short, &[1.0; 3]),
        0.0
    );
    assert_eq!(
        compensated_f32_sum([0.0, -0.0].into_iter()).to_bits(),
        0.0_f32.to_bits()
    );
}

/// Saturating activations still receive ordinary infinity; invalid cancellation remains NaN.
#[test]
fn scalar_compensation_preserves_nonfinite_partial_sum_propagation() {
    let maximum = vec![f32::MAX; 32];
    let input = vec![1.0; 32];
    assert_eq!(
        dot_product(InferenceMathBackend::Scalar, &maximum, &input),
        f32::INFINITY
    );
    assert_eq!(
        dot_product_mul(InferenceMathBackend::Scalar, &maximum, &input, &input),
        f32::INFINITY
    );
    assert_eq!(
        compensated_f32_sum([f32::NEG_INFINITY, 1.0].into_iter()),
        f32::NEG_INFINITY
    );
    assert!(compensated_f32_sum([f32::INFINITY, f32::NEG_INFINITY].into_iter()).is_nan());
    assert!(compensated_f32_sum([f32::NAN, 1.0].into_iter()).is_nan());
}
