//! Offline arithmetic ablations. No diagnostic mode is a persisted/runtime backend.
//!
//! Uses compiled tensor/matrix ranges and the production dot/affine primitives.
//! The explicit LSTM equations also expose intermediate values; endpoint parity
//! against ordinary graph evaluation is checked by the harness and unit tests.
use super::*;
use serde::Serialize;

/// Fixed experimental matrix groups; F32 fallbacks use the existing SSE2 kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackMode {
    Sse2,
    Fma128,
    I16,
    I16Affine,
    I16Lstm,
    I16LstmInput,
    I16LstmRecurrent,
}
impl FeedbackMode {
    pub const ALL: [Self; 7] = [
        Self::Sse2,
        Self::Fma128,
        Self::I16,
        Self::I16Affine,
        Self::I16Lstm,
        Self::I16LstmInput,
        Self::I16LstmRecurrent,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sse2 => "sse2",
            Self::Fma128 => "f32-fma128",
            Self::I16 => "i16-all",
            Self::I16Affine => "i16-affine",
            Self::I16Lstm => "i16-lstm",
            Self::I16LstmInput => "i16-lstm-input",
            Self::I16LstmRecurrent => "i16-lstm-recurrent",
        }
    }
    fn affine_i16(self) -> bool {
        matches!(self, Self::I16 | Self::I16Affine)
    }
    fn input_i16(self) -> bool {
        matches!(self, Self::I16 | Self::I16Lstm | Self::I16LstmInput)
    }
    fn recurrent_i16(self) -> bool {
        matches!(self, Self::I16 | Self::I16Lstm | Self::I16LstmRecurrent)
    }
    fn f32_backend(self) -> InferenceMathBackend {
        if self == Self::Fma128 {
            InferenceMathBackend::BenchmarkF32Fma128
        } else {
            InferenceMathBackend::Sse2
        }
    }
}

/// One cell, in the compiled LSTM gate order i/f/o/g. Dot terms are retained
/// separately so input perturbation, recurrent amplification and bias are visible.
#[derive(Clone, Debug, Serialize)]
pub struct LstmCellTrace {
    pub node: String,
    pub cell: usize,
    pub previous_hidden: f32,
    pub previous_cell: f32,
    pub input_dot: [f32; 4],
    pub recurrent_dot: [f32; 4],
    pub bias: [f32; 4],
    pub preactivation: [f32; 4],
    pub gate: [f32; 4],
    pub next_cell: f32,
    pub next_hidden: f32,
}

impl GraphExecutionPlan {
    /// Evaluate only the Dense/MLP/LSTM graph family used by this investigation.
    /// Unsupported nodes fail explicitly. Caller supplies preallocated scratch;
    /// tracing is optional and allocates only in this test-hooks-only API.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_feedback_diagnostic(
        &self,
        mode: FeedbackMode,
        weights: &[f32],
        cache: &QuantizedInferenceWeights,
        before: &[f32],
        after: &mut [f32],
        input: &[f32],
        output: &mut [f32],
        scratch: &mut CalculationScratchView<'_>,
        mut trace: Option<&mut Vec<LstmCellTrace>>,
    ) -> Result<(), InferenceError> {
        if !InferenceMathBackend::I16MatrixFma128.is_available() {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: "diagnostic requires SSE4.1/FMA".into(),
            });
        }
        // This cold ablation API indexes the original full matrix layout.
        // Compact mixed caches/plans are evaluated by the production API instead.
        if cache.layout != self.layout_digest_sha256
            || cache.backend != InferenceMathBackend::I16MatrixFma128
            || self.math_backend == InferenceMathBackend::I16AffineFma128
        {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: "diagnostic requires matching graph and full-i16 cache/plan layout".into(),
            });
        }
        require_length("weights", weights.len(), self.total_parameters)?;
        require_length("before", before.len(), self.total_state_size)?;
        require_length("after", after.len(), self.total_state_size)?;
        require_length("input", input.len(), self.input_size)?;
        require_length("output", output.len(), self.output_size)?;
        require_finite("input", input)?;
        require_finite("before", before)?;
        require_minimum_length(
            "activation",
            scratch.activation.len(),
            self.scratch_layout.activation_floats,
        )?;
        for node in &self.nodes {
            match node.node_type {
                CompiledNodeType::Input => scratch.activation
                    [node.output.offset..node.output.end()?]
                    .copy_from_slice(input),
                CompiledNodeType::Concat | CompiledNodeType::Split => {
                    let mut destination = node.output.offset;
                    for source in &node.inputs {
                        scratch
                            .activation
                            .copy_within(source.offset..source.end()?, destination);
                        destination += source.len;
                    }
                }
                CompiledNodeType::Dense => {
                    let parameters = NodeWeights::new(
                        weights,
                        mode.affine_i16().then_some(cache),
                        node.parameter,
                        node.matrix_start,
                    );
                    let (x, y) = forward_slices(
                        scratch.activation,
                        only_input(node)?,
                        node.output,
                        &node.id,
                    )?;
                    affine_tanh(mode.f32_backend(), parameters, x, y)?;
                }
                CompiledNodeType::Mlp => {
                    self.evaluate_mlp_diagnostic(node, mode, weights, cache, scratch.activation)?
                }
                CompiledNodeType::Lstm => {
                    let state = required_state(node)?;
                    let parameters =
                        NodeWeights::new(weights, None, node.parameter, node.matrix_start);
                    let quantized =
                        NodeWeights::new(weights, Some(cache), node.parameter, node.matrix_start);
                    let (x, y) = forward_slices(
                        scratch.activation,
                        only_input(node)?,
                        node.output,
                        &node.id,
                    )?;
                    lstm_diagnostic(
                        mode,
                        &node.id,
                        parameters,
                        quantized,
                        x,
                        slice_range(before, state)?,
                        slice_range_mut(after, state)?,
                        y,
                        trace.as_deref_mut(),
                    )?;
                }
                _ => {
                    return Err(InferenceError::InvalidCompiledGraph {
                        detail:
                            "feedback diagnostic supports Input/Split/Concat/Dense/MLP/LSTM only"
                                .into(),
                    })
                }
            }
        }
        let mut destination = 0;
        for source in &self.outputs {
            output[destination..destination + source.len]
                .copy_from_slice(&scratch.activation[source.offset..source.end()?]);
            destination += source.len;
        }
        require_finite("output", output)?;
        require_finite("after", after)?;
        Ok(())
    }

    fn evaluate_mlp_diagnostic(
        &self,
        node: &ExecutionNode,
        mode: FeedbackMode,
        weights: &[f32],
        cache: &QuantizedInferenceWeights,
        activation: &mut [f32],
    ) -> Result<(), InferenceError> {
        let mut source = only_input(node)?;
        let mut offset = node.parameter.offset;
        for (layer, destination) in node
            .hidden
            .iter()
            .chain(std::iter::once(&node.output))
            .enumerate()
        {
            let len = (source.len + 1) * destination.len;
            let parameters = NodeWeights::new(
                weights,
                mode.affine_i16().then_some(cache),
                TensorRange { offset, len },
                node.matrix_start + layer,
            );
            let (x, y) = forward_slices(activation, source, *destination, &node.id)?;
            affine_tanh(mode.f32_backend(), parameters, x, y)?;
            offset += len;
            source = *destination;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn lstm_diagnostic(
    mode: FeedbackMode,
    node: &str,
    canonical: NodeWeights<'_>,
    quantized: NodeWeights<'_>,
    input: &[f32],
    before: &[f32],
    after: &mut [f32],
    output: &mut [f32],
    mut trace: Option<&mut Vec<LstmCellTrace>>,
) -> Result<(), InferenceError> {
    let hidden = output.len();
    require_length("LSTM before", before.len(), hidden * 2)?;
    require_length("LSTM after", after.len(), hidden * 2)?;
    let (h, c) = before.split_at(hidden);
    let (next_h, next_c) = after.split_at_mut(hidden);
    let input_block = hidden * input.len();
    let state_block = hidden * hidden;
    let bias_start = 4 * (input_block + state_block);
    let w = if mode.input_i16() {
        quantized
    } else {
        canonical
    };
    let u = if mode.recurrent_i16() {
        quantized
    } else {
        canonical
    };
    for j in 0..hidden {
        let mut input_dot = [0.0; 4];
        let mut recurrent_dot = [0.0; 4];
        let mut bias = [0.0; 4];
        let mut preactivation = [0.0; 4];
        for gate in 0..4 {
            let wi = gate * input_block + j * input.len();
            let ui = 4 * input_block + gate * state_block + j * hidden;
            input_dot[gate] = w.dot(mode.f32_backend(), wi..wi + input.len(), input, gate);
            recurrent_dot[gate] = u.dot(mode.f32_backend(), ui..ui + hidden, h, gate + 4);
            bias[gate] = canonical.bias(bias_start + gate * hidden + j, None);
            preactivation[gate] = (input_dot[gate] + recurrent_dot[gate]) + bias[gate];
        }
        let gates = [
            sigmoid(preactivation[0]),
            sigmoid(preactivation[1]),
            sigmoid(preactivation[2]),
            preactivation[3].tanh(),
        ];
        let cell = gates[1] * c[j] + gates[0] * gates[3];
        let hidden_value = gates[2] * cell.tanh();
        next_c[j] = cell;
        next_h[j] = hidden_value;
        output[j] = hidden_value;
        if let Some(rows) = trace.as_deref_mut() {
            rows.push(LstmCellTrace {
                node: node.into(),
                cell: j,
                previous_hidden: h[j],
                previous_cell: c[j],
                input_dot,
                recurrent_dot,
                bias,
                preactivation,
                gate: gates,
                next_cell: cell,
                next_hidden: hidden_value,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::calculation::CalculationScratch;

    #[test]
    fn traced_endpoints_match_all_existing_backends_and_ablations_isolate_recurrence() {
        if !InferenceMathBackend::I16MatrixFma128.is_available() {
            return;
        }
        let graph = crate::engine::graph_json::compile_training_graph_json(
            br#"{
            "type":"graph","outputSize":2,"nodes":[
            {"id":"input","type":"Input","outputSize":5},
            {"id":"dense","type":"Dense","inputSize":5,"outputSize":3},
            {"id":"lstm","type":"LSTM","inputSize":5,"hiddenSize":7},
            {"id":"concat","type":"Concat"},
            {"id":"head","type":"MLP","inputSize":10,"hiddenSizes":[9,3],"outputSize":2}],
            "edges":[{"from":"input","to":"dense"},{"from":"input","to":"lstm"},
            {"from":"dense","to":"concat","toPort":0},{"from":"lstm","to":"concat","toPort":1},
            {"from":"concat","to":"head"}],"outputs":[{"nodeId":"head"}]}"#,
        )
        .unwrap();
        let plans = [
            InferenceMathBackend::Sse2,
            InferenceMathBackend::BenchmarkF32Fma128,
            InferenceMathBackend::I16MatrixFma128,
        ]
        .map(|b| GraphExecutionPlan::build_with_math_backend(&graph, b).unwrap());
        let weights: Vec<f32> = (0..graph.total_parameters)
            .map(|i| ((i * 17 % 101) as f32 - 50.0) / 73.0)
            .collect();
        let cache = plans[2].quantize(&weights).unwrap();
        let mut scratch =
            CalculationScratch::try_new(plans[0].scratch_layout(), 1024 * 1024).unwrap();
        let mut before = vec![0.0; graph.total_state_size];
        for step in 0..100 {
            let input: Vec<f32> = (0..5)
                .map(|i| ((step * 3 + i * 7) % 31) as f32 / 17.0 - 0.9)
                .collect();
            let mut states = Vec::new();
            for (index, mode) in FeedbackMode::ALL.into_iter().enumerate() {
                let mut after = before.clone();
                let mut output = [0.0; 2];
                let mut trace = Vec::new();
                plans[0]
                    .evaluate_feedback_diagnostic(
                        mode,
                        &weights,
                        &cache,
                        &before,
                        &mut after,
                        &input,
                        &mut output,
                        &mut scratch.view(),
                        Some(&mut trace),
                    )
                    .unwrap();
                assert_eq!(trace.len(), 7);
                for t in &trace {
                    assert_eq!(t.next_hidden, after[t.cell]);
                    assert_eq!(t.next_cell, after[7 + t.cell]);
                    assert_eq!(t.previous_hidden, before[t.cell]);
                    assert_eq!(t.previous_cell, before[7 + t.cell]);
                    for g in 0..4 {
                        assert_eq!(
                            t.preactivation[g],
                            (t.input_dot[g] + t.recurrent_dot[g]) + t.bias[g]
                        );
                    }
                }
                if index < 3 {
                    let mut expected = before.clone();
                    let mut expected_output = [0.0; 2];
                    if index == 2 {
                        plans[index]
                            .evaluate_quantized(
                                &weights,
                                &cache,
                                &before,
                                &mut expected,
                                &input,
                                &mut expected_output,
                                &mut scratch.view(),
                            )
                            .unwrap();
                    } else {
                        plans[index]
                            .evaluate(
                                &weights,
                                &before,
                                &mut expected,
                                &input,
                                &mut expected_output,
                                &mut scratch.view(),
                            )
                            .unwrap();
                    }
                    assert_eq!(after, expected);
                    assert_eq!(output, expected_output);
                }
                states.push(after);
            }
            assert_eq!(
                states[0], states[3],
                "affine-only quantization cannot affect this LSTM's independent input"
            );
            assert_eq!(
                states[2], states[4],
                "LSTM-only quantization preserves all-i16 recurrence for this graph"
            );
            before.copy_from_slice(&states[0]);
        }
    }
}
