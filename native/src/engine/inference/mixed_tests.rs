use super::*;
use crate::engine::{calculation::CalculationScratch, graph_json::compile_training_graph_json};

fn plan(json: &[u8], backend: InferenceMathBackend) -> GraphExecutionPlan {
    GraphExecutionPlan::build_with_math_backend(
        &compile_training_graph_json(json).unwrap(),
        backend,
    )
    .unwrap()
}
fn scratch(plan: &GraphExecutionPlan) -> CalculationScratch {
    CalculationScratch::try_new(plan.scratch_layout(), 64 * 1024 * 1024).unwrap()
}
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}
const EXACT: &[u8] = br#"{"type":"graph","outputSize":2,"nodes":[
{"id":"input","type":"Input","outputSize":264},
{"id":"mlp-a","type":"Dense","inputSize":264,"outputSize":396},
{"id":"LSTM","type":"LSTM","inputSize":264,"hiddenSize":396},
{"id":"concat","type":"Concat"},
{"id":"mlp-b","type":"MLP","inputSize":792,"hiddenSizes":[264,134],"outputSize":2}],
"edges":[{"from":"input","to":"mlp-a"},{"from":"input","to":"LSTM"},
{"from":"mlp-a","to":"concat","toPort":0},{"from":"LSTM","to":"concat","toPort":1},
{"from":"concat","to":"mlp-b"}],"outputs":[{"nodeId":"mlp-b"}]}"#;

#[test]
fn mixed_compact_cache_has_only_affine_values_and_a_distinct_identity() {
    if !InferenceMathBackend::I16AffineFma128.is_available() {
        return;
    }
    let mixed = plan(EXACT, InferenceMathBackend::I16AffineFma128);
    let full = plan(EXACT, InferenceMathBackend::I16MatrixFma128);
    assert_eq!(mixed.total_parameters(), 1_397_096);
    assert_eq!(mixed.matrices.len(), 4);
    assert_eq!(mixed.matrices.iter().map(|m| m.len).sum::<usize>(), 350_072);
    let weights = vec![0.0; mixed.total_parameters()];
    let cache = mixed.quantize(&weights).unwrap();
    assert_eq!(cache.matrix_count(), 4);
    assert_eq!(cache.allocated_bytes(), 700_160);
    assert_eq!(cache.allocated_bytes() * 85, 59_513_600);
    assert_eq!(
        full.quantize(&weights).unwrap().allocated_bytes(),
        2 * weights.len() + 12 * 4
    );
    assert_eq!(
        InferenceMathBackend::from_label("rust-i16-affine-fma128-sse2-v1"),
        Some(InferenceMathBackend::I16AffineFma128)
    );
    assert_eq!(InferenceMathBackend::detect(), InferenceMathBackend::Sse2);
    let mut after = vec![123.0; mixed.total_state_size()];
    let mut output = [123.0; 2];
    assert!(mixed
        .evaluate_quantized(
            &weights,
            &full.quantize(&weights).unwrap(),
            &vec![0.0; after.len()],
            &mut after,
            &vec![0.0; mixed.input_size()],
            &mut output,
            &mut scratch(&mixed).view()
        )
        .is_err());
    assert!(after.iter().all(|v| *v == 123.0));
    assert_eq!(output, [123.0; 2]);
    mixed
        .evaluate_quantized(
            &weights,
            &cache,
            &vec![0.0; after.len()],
            &mut after,
            &vec![1.0; mixed.input_size()],
            &mut output,
            &mut scratch(&mixed).view(),
        )
        .unwrap();
    assert!(after.iter().all(|v| *v == 0.0));
    assert_eq!(output, [0.0; 2]);
}

#[cfg(feature = "engine-test-hooks")]
#[test]
fn mixed_diagnostic_rejects_compact_cache_or_plan_before_writes() {
    use super::feedback_diagnostics::FeedbackMode;
    if !InferenceMathBackend::I16AffineFma128.is_available() {
        return;
    }
    // Equal cache sizes make backend/layout rejection independent of length.
    let json = br#"{"type":"graph","outputSize":2,"nodes":[
        {"id":"input","type":"Input","outputSize":3},
        {"id":"head","type":"Dense","inputSize":3,"outputSize":2}],
        "edges":[{"from":"input","to":"head"}],"outputs":[{"nodeId":"head"}]}"#;
    let sse = plan(json, InferenceMathBackend::Sse2);
    let mixed = plan(json, InferenceMathBackend::I16AffineFma128);
    let weights = vec![0.25; sse.total_parameters()];
    let full_cache = sse.quantize(&weights).unwrap();
    let compact_cache = mixed.quantize(&weights).unwrap();
    assert_eq!(
        full_cache.allocated_bytes(),
        compact_cache.allocated_bytes()
    );
    for (selected, cache) in [(&sse, &compact_cache), (&mixed, &full_cache)] {
        let mut output = [123.0; 2];
        let mut workspace = scratch(selected);
        let mut traces = Vec::new();
        assert!(selected
            .evaluate_feedback_diagnostic(
                FeedbackMode::I16Affine,
                &weights,
                cache,
                &[],
                &mut [],
                &[0.5; 3],
                &mut output,
                &mut workspace.view(),
                Some(&mut traces),
            )
            .is_err());
        assert_eq!(output, [123.0; 2]);
        assert!(traces.is_empty());
    }
}

#[cfg(feature = "engine-test-hooks")]
#[test]
fn mixed_exact_graph_matches_the_affine_ablation_and_bit_exact_sse2_recurrence() {
    use super::feedback_diagnostics::FeedbackMode;
    if !InferenceMathBackend::I16AffineFma128.is_available() {
        return;
    }
    let sse = plan(EXACT, InferenceMathBackend::Sse2);
    let mixed = plan(EXACT, InferenceMathBackend::I16AffineFma128);
    let weights: Vec<f32> = (0..sse.total_parameters())
        .map(|i| ((i * 17 % 997) as f32 - 498.0) / 5000.0)
        .collect();
    let original = weights.clone();
    let cache = mixed.quantize(&weights).unwrap();
    let full_cache = sse.quantize(&weights).unwrap();
    let mut sse_scratch = scratch(&sse);
    let mut mixed_scratch = scratch(&mixed);
    let mut oracle_scratch = scratch(&sse);
    let mut state = vec![0.03; sse.total_state_size()];
    for step in 0..128 {
        let input: Vec<f32> = (0..264)
            .map(|i| ((step * 17 + i * 11) % 97) as f32 / 48.0 - 1.0)
            .collect();
        let mut reference = state.clone();
        let mut actual = state.clone();
        let mut oracle = state.clone();
        let mut y_ref = [0.0; 2];
        let mut y = [0.0; 2];
        let mut y_oracle = [0.0; 2];
        sse.evaluate(
            &weights,
            &state,
            &mut reference,
            &input,
            &mut y_ref,
            &mut sse_scratch.view(),
        )
        .unwrap();
        mixed
            .evaluate_quantized(
                &weights,
                &cache,
                &state,
                &mut actual,
                &input,
                &mut y,
                &mut mixed_scratch.view(),
            )
            .unwrap();
        sse.evaluate_feedback_diagnostic(
            FeedbackMode::I16Affine,
            &weights,
            &full_cache,
            &state,
            &mut oracle,
            &input,
            &mut y_oracle,
            &mut oracle_scratch.view(),
            None,
        )
        .unwrap();
        assert_eq!(bits(&reference), bits(&actual));
        assert_eq!(bits(&oracle), bits(&actual));
        assert_eq!(bits(&y), bits(&y_oracle));
        assert!(y.iter().zip(y_ref).all(|(a, b)| (a - b).abs() < 0.0001));
        state = actual;
    }
    assert_eq!(weights, original);
}

#[test]
fn mixed_recurrent_only_graphs_use_sse2_without_any_i16_values() {
    if !InferenceMathBackend::I16AffineFma128.is_available() {
        return;
    }
    for kind in ["LSTM", "GRU", "RRU"] {
        let graph = format!(
            r#"{{"type":"graph","outputSize":2,"nodes":[{{"id":"input","type":"Input","outputSize":3}},{{"id":"memory","type":"{kind}","inputSize":3,"hiddenSize":2}}],"edges":[{{"from":"input","to":"memory"}}],"outputs":[{{"nodeId":"memory"}}]}}"#
        );
        let mixed = plan(graph.as_bytes(), InferenceMathBackend::I16AffineFma128);
        let sse = plan(graph.as_bytes(), InferenceMathBackend::Sse2);
        let weights: Vec<f32> = (0..mixed.total_parameters())
            .map(|i| (i % 13) as f32 / 17.0 - 0.3)
            .collect();
        let cache = mixed.quantize(&weights).unwrap();
        assert_eq!(cache.allocated_bytes(), 0);
        assert_eq!(cache.matrix_count(), 0);
        let mut state = vec![0.03; mixed.total_state_size()];
        for step in 0..32 {
            let x = [0.1, step as f32 / 100.0, -0.2];
            let mut actual = state.clone();
            let mut expected = state.clone();
            let mut y = [0.0; 2];
            let mut y_ref = y;
            mixed
                .evaluate_quantized(
                    &weights,
                    &cache,
                    &state,
                    &mut actual,
                    &x,
                    &mut y,
                    &mut scratch(&mixed).view(),
                )
                .unwrap();
            sse.evaluate(
                &weights,
                &state,
                &mut expected,
                &x,
                &mut y_ref,
                &mut scratch(&sse).view(),
            )
            .unwrap();
            assert_eq!(bits(&actual), bits(&expected));
            assert_eq!(bits(&y), bits(&y_ref));
            state = actual;
        }
    }
}

#[test]
fn mixed_upstream_affine_changes_inputs_but_lstm_uses_canonical_sse2_math() {
    if !InferenceMathBackend::I16AffineFma128.is_available() {
        return;
    }
    let graph=br#"{"type":"graph","outputSize":2,"nodes":[{"id":"input","type":"Input","outputSize":3},{"id":"affine","type":"Dense","inputSize":3,"outputSize":5},{"id":"memory","type":"LSTM","inputSize":5,"hiddenSize":2}],"edges":[{"from":"input","to":"affine"},{"from":"affine","to":"memory"}],"outputs":[{"nodeId":"memory"}]}"#;
    let mixed = plan(graph, InferenceMathBackend::I16AffineFma128);
    let weights: Vec<f32> = (0..mixed.total_parameters())
        .map(|i| (i % 17) as f32 / 23.0 - 0.3)
        .collect();
    let cache = mixed.quantize(&weights).unwrap();
    let mut work = scratch(&mixed);
    let state = vec![0.03; 4];
    let mut actual = state.clone();
    let mut output = [0.0; 2];
    mixed
        .evaluate_quantized(
            &weights,
            &cache,
            &state,
            &mut actual,
            &[0.2, -0.1, 0.7],
            &mut output,
            &mut work.view(),
        )
        .unwrap();
    let dense = mixed
        .nodes
        .iter()
        .find(|n| n.node_type == CompiledNodeType::Dense)
        .unwrap();
    let lstm = mixed
        .nodes
        .iter()
        .find(|n| n.node_type == CompiledNodeType::Lstm)
        .unwrap();
    let view = work.view();
    let x = &view.activation[dense.output.offset..dense.output.end().unwrap()];
    let mut expected = state.clone();
    let mut y = [0.0; 2];
    evaluate_lstm(
        InferenceMathBackend::Sse2,
        NodeWeights::new(&weights, None, lstm.parameter, 0),
        x,
        &state,
        &mut expected,
        &mut y,
    )
    .unwrap();
    assert_eq!(bits(&actual), bits(&expected));
    assert_eq!(bits(&output), bits(&y));
}
