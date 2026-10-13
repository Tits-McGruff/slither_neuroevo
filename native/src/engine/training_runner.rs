//! Bounded offline whole-graph sequence inference for canonical trainer parity.
//! No world, controller, authoritative RNG, or production per-layer bridge exists here.

use super::calculation::CalculationScratch;
use super::graph::GraphBundle;
use super::inference::{GraphExecutionPlan, InferenceMathBackend};

/// Long recurrence parity gate and maximum accepted offline sequence length.
pub const MAX_TRAINING_RUNNER_STEPS: usize = 256;
/// Complete numeric staging envelope, independent of graph admission ceilings.
pub const MAX_TRAINING_RUNNER_BYTES: usize = 64 * 1024 * 1024;

/// Step-major output and recurrent-state continuations, without input aliases.
#[derive(Debug, PartialEq)]
pub struct TrainingSequenceResult {
    pub steps: usize,
    pub outputs: Vec<f32>,
    /// Includes the next state after every delivery, not only the final state.
    pub recurrent_after_each_step: Vec<f32>,
}

/// Evaluate one contiguous bounded sequence using the deployed Rust executor.
/// Every row is an actual delivery; cadence gaps/life resets are caller boundaries.
pub fn evaluate_training_sequence(
    graph: &GraphBundle,
    weights: &[f32],
    observations: &[f32],
    recurrent_before: &[f32],
    backend: InferenceMathBackend,
) -> Result<TrainingSequenceResult, String> {
    let plan = GraphExecutionPlan::build_with_math_backend(graph, backend)
        .map_err(|error| error.to_string())?;
    if weights.len() != plan.total_parameters() {
        return Err("offline weight count does not match canonical graph".into());
    }
    // Use the same finite-value validator as ordinary population admission.
    super::state::validate_f32_slice("population.weights", 0, weights)
        .map_err(|error| error.to_string())?;
    if recurrent_before.len() != plan.total_state_size() {
        return Err("offline initial state count does not match canonical graph".into());
    }
    if !observations.len().is_multiple_of(plan.input_size()) {
        return Err("offline observations must contain complete input rows".into());
    }
    let steps = observations.len() / plan.input_size();
    if steps == 0 || steps > MAX_TRAINING_RUNNER_STEPS {
        return Err("offline sequence must contain 1..256 deliveries".into());
    }
    let states = steps
        .checked_mul(plan.total_state_size())
        .ok_or("offline state count overflow")?;
    let outputs = steps
        .checked_mul(plan.output_size())
        .ok_or("offline output count overflow")?;
    let floats = weights
        .len()
        .checked_add(observations.len())
        .and_then(|total| total.checked_add(recurrent_before.len()))
        .and_then(|total| total.checked_add(states))
        .and_then(|total| total.checked_add(outputs))
        .ok_or("offline numeric staging count overflow")?;
    let bytes = floats
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(plan.scratch_layout().required_bytes().ok()?))
        .ok_or("offline numeric staging byte count overflow")?;
    if bytes > MAX_TRAINING_RUNNER_BYTES {
        return Err("offline numeric staging exceeds 64 MiB".into());
    }
    let zeroed = |count: usize| -> Result<Vec<f32>, String> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| "offline numeric allocation failed")?;
        values.resize(count, 0.0);
        Ok(values)
    };
    let mut result = TrainingSequenceResult {
        steps,
        outputs: zeroed(outputs)?,
        recurrent_after_each_step: zeroed(states)?,
    };
    let mut scratch = CalculationScratch::try_new(plan.scratch_layout(), MAX_TRAINING_RUNNER_BYTES)
        .map_err(|error| error.to_string())?;
    for step in 0..steps {
        let state_offset = step * plan.total_state_size();
        let (prefix, suffix) = result.recurrent_after_each_step.split_at_mut(state_offset);
        let before = if step == 0 {
            recurrent_before
        } else {
            &prefix[prefix.len() - plan.total_state_size()..]
        };
        plan.evaluate(
            weights,
            before,
            &mut suffix[..plan.total_state_size()],
            &observations[step * plan.input_size()..(step + 1) * plan.input_size()],
            &mut result.outputs[step * plan.output_size()..(step + 1) * plan.output_size()],
            &mut scratch.view(),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::{
        GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
    };

    fn recurrent_graph() -> GraphBundle {
        GraphBundle::compile(
            GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".into(),
                        kind: GraphNodeKind::Input { output_size: 2 },
                    },
                    GraphNodeSpec {
                        id: "gru".into(),
                        kind: GraphNodeKind::Gru {
                            input_size: 2,
                            hidden_size: 2,
                        },
                    },
                ],
                edges: vec![GraphEdge {
                    from: "input".into(),
                    to: "gru".into(),
                    from_port: None,
                    to_port: None,
                }],
                outputs: vec![GraphOutputRef {
                    node_id: "gru".into(),
                    port: None,
                }],
                output_size: 2,
            },
            &crate::engine::fresh_run::stage6a_p0_graph_limits(),
        )
        .unwrap()
    }

    #[test]
    fn long_recurrence_matches_scalar_and_sse2_and_preserves_input_bytes() {
        let graph = recurrent_graph();
        let weights: Vec<_> = (0..graph.total_parameters)
            .map(|i| ((i as f32 * 0.13).sin()) * 0.2)
            .collect();
        let observations: Vec<_> = (0..512).map(|i| (i as f32 * 0.17).cos()).collect();
        let before = [0.17, -0.31];
        let source = (weights.clone(), observations.clone(), before);
        let scalar = evaluate_training_sequence(
            &graph,
            &weights,
            &observations,
            &before,
            InferenceMathBackend::Scalar,
        )
        .unwrap();
        let sse2 = evaluate_training_sequence(
            &graph,
            &weights,
            &observations,
            &before,
            InferenceMathBackend::Sse2,
        )
        .unwrap();
        for (a, b) in scalar
            .outputs
            .iter()
            .chain(&scalar.recurrent_after_each_step)
            .zip(sse2.outputs.iter().chain(&sse2.recurrent_after_each_step))
        {
            assert!((a - b).abs() <= 1e-5 + 1e-4 * b.abs());
        }
        assert_eq!(scalar.steps, 256);
        assert_eq!(scalar.outputs, scalar.recurrent_after_each_step);
        assert_eq!((weights, observations, before), source);
    }

    #[test]
    fn invalid_lengths_nonfinite_and_sequence_bounds_reject() {
        let graph = recurrent_graph();
        let mut weights = vec![0.0; graph.total_parameters];
        let run = |weights: &[f32], rows: &[f32], state: &[f32]| {
            evaluate_training_sequence(&graph, weights, rows, state, InferenceMathBackend::Scalar)
        };
        assert!(run(&weights[..weights.len() - 1], &[0.0; 2], &[0.0; 2]).is_err());
        assert!(run(&weights, &[0.0; 3], &[0.0; 2]).is_err());
        assert!(run(&weights, &[], &[0.0; 2]).is_err());
        assert!(run(&weights, &[0.0; 514], &[0.0; 2]).is_err());
        assert!(run(&weights, &[0.0; 2], &[0.0; 1]).is_err());
        assert!(run(&weights, &[f32::NAN; 2], &[0.0; 2]).is_err());
        assert!(run(&weights, &[0.0; 2], &[f32::INFINITY; 2]).is_err());
        weights[0] = f32::NAN;
        assert!(run(&weights, &[0.0; 2], &[0.0; 2]).is_err());
    }
}
