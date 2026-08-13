//! Safe scalar/SIMD execution for complete neural graphs and heterogeneous populations.
//!
//! The approved runtime owns graph traversal, per-genome parameters, and recurrent
//! state in Rust. This module deliberately operates below N-API: one call evaluates
//! one complete graph, and the batch entry point walks a due population whose members
//! have different observations, weights, and state. Recurrent results are staged in
//! caller-owned memory and become authoritative only through an explicit commit after
//! the complete batch succeeds.

use super::calculation::{CalculationScratchLayout, CalculationScratchView, CalculationWorkUnit};
use super::graph::{CompiledGraph, CompiledNode, CompiledNodeType};
use super::state::{BrainOwner, BrainRuntimeState, PopulationGenome};
use crate::simd_kernels;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

/// One precomputed contiguous range in graph-activation storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TensorRange {
    offset: usize,
    len: usize,
}

impl TensorRange {
    /// Exclusive end of this checked range.
    fn end(self) -> Result<usize, InferenceError> {
        self.offset
            .checked_add(self.len)
            .ok_or(InferenceError::ArithmeticOverflow {
                context: "tensor range end",
            })
    }
}

/// Runtime-only metadata for one already compiled graph node.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecutionNode {
    id: String,
    node_type: CompiledNodeType,
    inputs: Vec<TensorRange>,
    output: TensorRange,
    hidden: Vec<TensorRange>,
    parameter: TensorRange,
    state: Option<TensorRange>,
}

/// Numeric implementation selected once for one immutable execution plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InferenceMathBackend {
    /// Reference implementation using ordered scalar `f32` accumulation.
    Scalar,
    /// Existing four-lane SSE dot kernels selected after runtime detection.
    Sse2,
}

impl InferenceMathBackend {
    /// Select the fastest admitted implementation available to this process.
    pub fn detect() -> Self {
        if simd_kernels::runtime_sse2_available() {
            Self::Sse2
        } else {
            Self::Scalar
        }
    }

    /// Stable evidence and health label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scalar => "rust-scalar-v1",
            Self::Sse2 => "rust-sse2-v1",
        }
    }

    /// Return whether this exact implementation is admitted on the current CPU.
    pub fn is_available(self) -> bool {
        match self {
            Self::Scalar => true,
            Self::Sse2 => simd_kernels::runtime_sse2_available(),
        }
    }
}

/// Immutable allocation and traversal plan derived once from a compiled graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphExecutionPlan {
    layout_digest_sha256: [u8; 32],
    input_size: usize,
    output_size: usize,
    total_parameters: usize,
    total_state_size: usize,
    math_backend: InferenceMathBackend,
    nodes: Vec<ExecutionNode>,
    outputs: Vec<TensorRange>,
    scratch_layout: CalculationScratchLayout,
}

/// One explicitly selected node activation from one compatible graph plan.
///
/// Capturing is a separate opt-in operation after graph evaluation. The
/// ordinary population path performs no capture lookup, copy, or allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationCapturePlan {
    layout_digest_sha256: [u8; 32],
    node_id: String,
    ranges: Vec<TensorRange>,
    widths: Vec<usize>,
    total_len: usize,
}

impl ActivationCapturePlan {
    /// Selected graph-node identifier.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Exact number of Float32 activation values captured for the node.
    pub const fn len(&self) -> usize {
        self.total_len
    }

    /// Whether the selected node produces no values.
    pub const fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    /// Ordered captured widths: every MLP hidden layer followed by its output,
    /// or the single output width for any other node.
    pub fn layer_widths(&self) -> &[usize] {
        &self.widths
    }
}

impl GraphExecutionPlan {
    /// Derive all ranges once and select the fastest runtime-admitted math backend.
    pub fn build(graph: &CompiledGraph) -> Result<Self, InferenceError> {
        Self::build_with_math_backend(graph, InferenceMathBackend::detect())
    }

    /// Derive all ranges once with an explicit backend for parity and evidence.
    pub fn build_with_math_backend(
        graph: &CompiledGraph,
        math_backend: InferenceMathBackend,
    ) -> Result<Self, InferenceError> {
        if !math_backend.is_available() {
            return Err(InferenceError::UnavailableMathBackend {
                backend: math_backend.label(),
            });
        }
        let mut activation_cursor = 0usize;
        let mut outputs_by_id: BTreeMap<String, Vec<TensorRange>> = BTreeMap::new();
        let mut execution_nodes = Vec::new();
        execution_nodes
            .try_reserve_exact(graph.nodes.len())
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "graph execution nodes",
                elements: graph.nodes.len(),
            })?;
        let mut input_size = None;
        let mut temporary_floats = 0usize;
        let mut parameter_cursor = 0usize;
        let mut state_cursor = 0usize;

        for node in &graph.nodes {
            let mut inputs = Vec::new();
            inputs.try_reserve_exact(node.inputs.len()).map_err(|_| {
                InferenceError::AllocationFailed {
                    buffer: "graph execution inputs",
                    elements: node.inputs.len(),
                }
            })?;
            for (expected_ordinal, input) in node.inputs.iter().enumerate() {
                if input.input_ordinal != expected_ordinal {
                    return Err(InferenceError::InvalidCompiledGraph {
                        detail: format!(
                            "node {} input ordinal {} appears at position {expected_ordinal}",
                            node.id, input.input_ordinal
                        ),
                    });
                }
                let ports = outputs_by_id.get(&input.from_id).ok_or_else(|| {
                    InferenceError::InvalidCompiledGraph {
                        detail: format!(
                            "node {} refers to unavailable source {}",
                            node.id, input.from_id
                        ),
                    }
                })?;
                let source = ports.get(input.from_port).copied().ok_or_else(|| {
                    InferenceError::InvalidCompiledGraph {
                        detail: format!(
                            "node {} refers to missing port {} on {}",
                            node.id, input.from_port, input.from_id
                        ),
                    }
                })?;
                inputs.push(source);
            }

            let mut hidden = Vec::new();
            hidden
                .try_reserve_exact(node.hidden_sizes.len())
                .map_err(|_| InferenceError::AllocationFailed {
                    buffer: "MLP hidden activation ranges",
                    elements: node.hidden_sizes.len(),
                })?;
            for width in &node.hidden_sizes {
                hidden.push(allocate_tensor(&mut activation_cursor, *width)?);
            }
            let output = allocate_tensor(&mut activation_cursor, node.output_size)?;
            let mut ports = Vec::new();
            ports
                .try_reserve_exact(node.output_sizes.len())
                .map_err(|_| InferenceError::AllocationFailed {
                    buffer: "graph output-port ranges",
                    elements: node.output_sizes.len(),
                })?;
            let mut port_offset = output.offset;
            for width in &node.output_sizes {
                ports.push(TensorRange {
                    offset: port_offset,
                    len: *width,
                });
                port_offset =
                    port_offset
                        .checked_add(*width)
                        .ok_or(InferenceError::ArithmeticOverflow {
                            context: "graph output-port offset",
                        })?;
            }
            if port_offset != output.end()? {
                return Err(InferenceError::InvalidCompiledGraph {
                    detail: format!("node {} output ports do not cover its output", node.id),
                });
            }

            let parameter = TensorRange {
                offset: node.parameter_offset,
                len: node.parameter_length,
            };
            if parameter.offset != parameter_cursor {
                return Err(InferenceError::InvalidCompiledGraph {
                    detail: format!(
                        "node {} parameter offset {} does not follow {parameter_cursor}",
                        node.id, parameter.offset
                    ),
                });
            }
            if parameter.end()? > graph.total_parameters {
                return Err(InferenceError::InvalidCompiledGraph {
                    detail: format!("node {} parameter range is out of bounds", node.id),
                });
            }
            parameter_cursor = parameter.end()?;
            let state = match (node.state_offset, node.state_length) {
                (Some(offset), length) if length != 0 => {
                    let range = TensorRange {
                        offset,
                        len: length,
                    };
                    if range.offset != state_cursor {
                        return Err(InferenceError::InvalidCompiledGraph {
                            detail: format!(
                                "node {} state offset {} does not follow {state_cursor}",
                                node.id, range.offset
                            ),
                        });
                    }
                    if range.end()? > graph.total_state_size {
                        return Err(InferenceError::InvalidCompiledGraph {
                            detail: format!("node {} recurrent range is out of bounds", node.id),
                        });
                    }
                    state_cursor = range.end()?;
                    Some(range)
                }
                (None, 0) => None,
                _ => {
                    return Err(InferenceError::InvalidCompiledGraph {
                        detail: format!("node {} has inconsistent recurrent metadata", node.id),
                    });
                }
            };
            if node.node_type == CompiledNodeType::Input
                && input_size.replace(node.output_size).is_some()
            {
                return Err(InferenceError::InvalidCompiledGraph {
                    detail: "more than one Input node reached execution planning".to_owned(),
                });
            }
            if node.node_type == CompiledNodeType::Gru {
                let hidden_size =
                    node.hidden_size
                        .ok_or_else(|| InferenceError::InvalidCompiledGraph {
                            detail: format!("GRU node {} has no hidden width", node.id),
                        })?;
                temporary_floats = temporary_floats.max(hidden_size.checked_mul(2).ok_or(
                    InferenceError::ArithmeticOverflow {
                        context: "GRU temporary floats",
                    },
                )?);
            }
            validate_node_shape(node, &inputs, state)?;

            outputs_by_id.insert(node.id.clone(), ports.clone());
            execution_nodes.push(ExecutionNode {
                id: node.id.clone(),
                node_type: node.node_type,
                inputs,
                output,
                hidden,
                parameter,
                state,
            });
        }
        if parameter_cursor != graph.total_parameters {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: format!(
                    "node parameter ranges end at {parameter_cursor}, not {}",
                    graph.total_parameters
                ),
            });
        }
        if state_cursor != graph.total_state_size {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: format!(
                    "node state ranges end at {state_cursor}, not {}",
                    graph.total_state_size
                ),
            });
        }

        let input_size = input_size.ok_or_else(|| InferenceError::InvalidCompiledGraph {
            detail: "execution plan has no Input node".to_owned(),
        })?;
        let mut graph_outputs = Vec::new();
        graph_outputs
            .try_reserve_exact(graph.outputs.len())
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "graph result ranges",
                elements: graph.outputs.len(),
            })?;
        let mut resolved_output_size = 0usize;
        for output in &graph.outputs {
            let port = output
                .port
                .ok_or_else(|| InferenceError::InvalidCompiledGraph {
                    detail: format!("graph output {} has no canonical port", output.node_id),
                })?;
            let port = usize::try_from(port).map_err(|_| InferenceError::InvalidCompiledGraph {
                detail: format!("graph output {} has a negative port", output.node_id),
            })?;
            let ranges = outputs_by_id.get(&output.node_id).ok_or_else(|| {
                InferenceError::InvalidCompiledGraph {
                    detail: format!("graph output source {} is unavailable", output.node_id),
                }
            })?;
            let range =
                ranges
                    .get(port)
                    .copied()
                    .ok_or_else(|| InferenceError::InvalidCompiledGraph {
                        detail: format!(
                            "graph output {} port {port} is unavailable",
                            output.node_id
                        ),
                    })?;
            resolved_output_size = resolved_output_size.checked_add(range.len).ok_or(
                InferenceError::ArithmeticOverflow {
                    context: "resolved graph output size",
                },
            )?;
            graph_outputs.push(range);
        }
        if resolved_output_size != graph.output_size {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: format!(
                    "resolved output width {resolved_output_size} differs from compiled width {}",
                    graph.output_size
                ),
            });
        }

        Ok(Self {
            layout_digest_sha256: graph.layout_digest_sha256,
            input_size,
            output_size: graph.output_size,
            total_parameters: graph.total_parameters,
            total_state_size: graph.total_state_size,
            math_backend,
            nodes: execution_nodes,
            outputs: graph_outputs,
            scratch_layout: CalculationScratchLayout {
                activation_floats: activation_cursor,
                gather_floats: 0,
                temporary_floats,
            },
        })
    }

    /// Exact graph-layout digest from which this plan was derived.
    pub const fn layout_digest_sha256(&self) -> [u8; 32] {
        self.layout_digest_sha256
    }

    /// Required sensor/input width.
    pub const fn input_size(&self) -> usize {
        self.input_size
    }

    /// Complete mapped graph-output width.
    pub const fn output_size(&self) -> usize {
        self.output_size
    }

    /// Packed parameter floats required for each brain.
    pub const fn total_parameters(&self) -> usize {
        self.total_parameters
    }

    /// Packed recurrent floats required for each brain.
    pub const fn total_state_size(&self) -> usize {
        self.total_state_size
    }

    /// Selected scalar or runtime-detected SIMD implementation.
    pub const fn math_backend(&self) -> InferenceMathBackend {
        self.math_backend
    }

    /// Fixed reusable calculation scratch required by this plan.
    pub const fn scratch_layout(&self) -> CalculationScratchLayout {
        self.scratch_layout
    }

    /// Resolve one node once for later explicit focused activation capture.
    pub fn prepare_activation_capture(
        &self,
        node_id: &str,
    ) -> Result<ActivationCapturePlan, InferenceError> {
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| InferenceError::ActivationNodeNotFound {
                node_id: node_id.to_owned(),
            })?;
        let mut ranges = Vec::new();
        ranges
            .try_reserve_exact(node.hidden.len().saturating_add(1))
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "activation capture ranges",
                elements: node.hidden.len().saturating_add(1),
            })?;
        ranges.extend_from_slice(&node.hidden);
        ranges.push(node.output);
        let mut widths = Vec::new();
        widths
            .try_reserve_exact(ranges.len())
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "activation capture widths",
                elements: ranges.len(),
            })?;
        let total_len = ranges.iter().try_fold(0usize, |total, range| {
            widths.push(range.len);
            total
                .checked_add(range.len)
                .ok_or(InferenceError::ArithmeticOverflow {
                    context: "activation capture width",
                })
        })?;
        Ok(ActivationCapturePlan {
            layout_digest_sha256: self.layout_digest_sha256,
            node_id: node.id.clone(),
            ranges,
            widths,
            total_len,
        })
    }

    /// Copy one requested node activation from the most recent evaluation in
    /// this exact scratch view. Callers invoke this only for the focused brain.
    pub fn capture_activation(
        &self,
        capture: &ActivationCapturePlan,
        scratch: &CalculationScratchView<'_>,
        destination: &mut [f32],
    ) -> Result<(), InferenceError> {
        if capture.layout_digest_sha256 != self.layout_digest_sha256 {
            return Err(InferenceError::ActivationLayoutMismatch {
                node_id: capture.node_id.clone(),
            });
        }
        require_length("activation capture", destination.len(), capture.total_len)?;
        let mut destination_offset = 0usize;
        for range in &capture.ranges {
            let end = range.end()?;
            require_minimum_length("activation scratch", scratch.activation.len(), end)?;
            let destination_end = destination_offset.checked_add(range.len).ok_or(
                InferenceError::ArithmeticOverflow {
                    context: "activation capture destination",
                },
            )?;
            destination[destination_offset..destination_end]
                .copy_from_slice(&scratch.activation[range.offset..end]);
            destination_offset = destination_end;
        }
        Ok(())
    }

    /// Evaluate one complete graph with its own weights and staged recurrent state.
    ///
    /// `recurrent_before` remains unchanged. Every recurrent node writes its complete
    /// block to `recurrent_after`; callers publish that buffer only after the enclosing
    /// population operation succeeds.
    pub fn evaluate(
        &self,
        weights: &[f32],
        recurrent_before: &[f32],
        recurrent_after: &mut [f32],
        input: &[f32],
        output: &mut [f32],
        scratch: &mut CalculationScratchView<'_>,
    ) -> Result<(), InferenceError> {
        require_length("weights", weights.len(), self.total_parameters)?;
        require_length(
            "recurrent_before",
            recurrent_before.len(),
            self.total_state_size,
        )?;
        require_length(
            "recurrent_after",
            recurrent_after.len(),
            self.total_state_size,
        )?;
        require_length("graph input", input.len(), self.input_size)?;
        require_length("graph output", output.len(), self.output_size)?;
        require_finite("recurrent_before", recurrent_before)?;
        require_finite("graph input", input)?;
        require_minimum_length(
            "activation scratch",
            scratch.activation.len(),
            self.scratch_layout.activation_floats,
        )?;
        require_minimum_length(
            "gather scratch",
            scratch.gather.len(),
            self.scratch_layout.gather_floats,
        )?;
        require_minimum_length(
            "temporary scratch",
            scratch.temporary.len(),
            self.scratch_layout.temporary_floats,
        )?;

        for node in &self.nodes {
            match node.node_type {
                CompiledNodeType::Input => {
                    scratch.activation[node.output.offset..node.output.end()?]
                        .copy_from_slice(input);
                }
                CompiledNodeType::Concat => {
                    let mut destination = node.output.offset;
                    for source in &node.inputs {
                        let source_end = source.end()?;
                        let destination_end = destination.checked_add(source.len).ok_or(
                            InferenceError::ArithmeticOverflow {
                                context: "Concat destination end",
                            },
                        )?;
                        scratch
                            .activation
                            .copy_within(source.offset..source_end, destination);
                        destination = destination_end;
                    }
                    if destination != node.output.end()? {
                        return Err(InferenceError::InvalidExecutionPlan {
                            node_id: node.id.clone(),
                            detail: "Concat inputs do not fill the destination".to_owned(),
                        });
                    }
                }
                CompiledNodeType::Split => {
                    let source = only_input(node)?;
                    if source.len != node.output.len {
                        return Err(InferenceError::InvalidExecutionPlan {
                            node_id: node.id.clone(),
                            detail: "Split source and destination widths differ".to_owned(),
                        });
                    }
                    scratch
                        .activation
                        .copy_within(source.offset..source.end()?, node.output.offset);
                }
                CompiledNodeType::Dense => {
                    let source = only_input(node)?;
                    let parameters = slice_range(weights, node.parameter)?;
                    let (input_values, output_values) =
                        forward_slices(scratch.activation, source, node.output, &node.id)?;
                    affine_tanh(self.math_backend, parameters, input_values, output_values)?;
                }
                CompiledNodeType::Mlp => {
                    self.evaluate_mlp(node, weights, scratch.activation)?;
                }
                CompiledNodeType::Gru => {
                    let source = only_input(node)?;
                    let parameters = slice_range(weights, node.parameter)?;
                    let state = required_state(node)?;
                    let before = slice_range(recurrent_before, state)?;
                    let after = slice_range_mut(recurrent_after, state)?;
                    let (input_values, output_values) =
                        forward_slices(scratch.activation, source, node.output, &node.id)?;
                    evaluate_gru(
                        self.math_backend,
                        parameters,
                        input_values,
                        before,
                        after,
                        output_values,
                        scratch.temporary,
                    )?;
                }
                CompiledNodeType::Lstm => {
                    let source = only_input(node)?;
                    let parameters = slice_range(weights, node.parameter)?;
                    let state = required_state(node)?;
                    let before = slice_range(recurrent_before, state)?;
                    let after = slice_range_mut(recurrent_after, state)?;
                    let (input_values, output_values) =
                        forward_slices(scratch.activation, source, node.output, &node.id)?;
                    evaluate_lstm(
                        self.math_backend,
                        parameters,
                        input_values,
                        before,
                        after,
                        output_values,
                    )?;
                }
                CompiledNodeType::Rru => {
                    let source = only_input(node)?;
                    let parameters = slice_range(weights, node.parameter)?;
                    let state = required_state(node)?;
                    let before = slice_range(recurrent_before, state)?;
                    let after = slice_range_mut(recurrent_after, state)?;
                    let (input_values, output_values) =
                        forward_slices(scratch.activation, source, node.output, &node.id)?;
                    evaluate_rru(
                        self.math_backend,
                        parameters,
                        input_values,
                        before,
                        after,
                        output_values,
                    )?;
                }
            }
        }

        let mut destination = 0usize;
        for source in &self.outputs {
            let end =
                destination
                    .checked_add(source.len)
                    .ok_or(InferenceError::ArithmeticOverflow {
                        context: "graph result destination",
                    })?;
            output[destination..end]
                .copy_from_slice(&scratch.activation[source.offset..source.end()?]);
            destination = end;
        }
        debug_assert_eq!(destination, output.len());
        require_finite("graph output", output)?;
        require_finite("next recurrent state", recurrent_after)?;
        Ok(())
    }

    /// Execute one MLP node using preallocated hidden activation ranges.
    fn evaluate_mlp(
        &self,
        node: &ExecutionNode,
        weights: &[f32],
        activation: &mut [f32],
    ) -> Result<(), InferenceError> {
        let mut source = only_input(node)?;
        let mut parameter_cursor = node.parameter.offset;
        for destination in node.hidden.iter().chain(std::iter::once(&node.output)) {
            let layer_parameters = destination
                .len
                .checked_mul(source.len.checked_add(1).ok_or(
                    InferenceError::ArithmeticOverflow {
                        context: "MLP affine row",
                    },
                )?)
                .ok_or(InferenceError::ArithmeticOverflow {
                    context: "MLP affine parameters",
                })?;
            let parameter_end = parameter_cursor.checked_add(layer_parameters).ok_or(
                InferenceError::ArithmeticOverflow {
                    context: "MLP parameter end",
                },
            )?;
            if parameter_end > node.parameter.end()? {
                return Err(InferenceError::InvalidExecutionPlan {
                    node_id: node.id.clone(),
                    detail: "MLP layer exceeds its parameter block".to_owned(),
                });
            }
            let (input_values, output_values) =
                forward_slices(activation, source, *destination, &node.id)?;
            affine_tanh(
                self.math_backend,
                &weights[parameter_cursor..parameter_end],
                input_values,
                output_values,
            )?;
            parameter_cursor = parameter_end;
            source = *destination;
        }
        if parameter_cursor != node.parameter.end()? {
            return Err(InferenceError::InvalidExecutionPlan {
                node_id: node.id.clone(),
                detail: "MLP did not consume its complete parameter block".to_owned(),
            });
        }
        Ok(())
    }
}

/// Packed caller-owned inputs and staging destinations for one due population.
pub struct HeterogeneousInferenceBuffers<'a> {
    /// Observations packed in prepared-work order.
    pub observations: &'a [f32],
    /// Complete graph outputs packed in prepared-work order.
    pub staged_outputs: &'a mut [f32],
    /// Complete next recurrent blocks packed in prepared-work order.
    pub staged_recurrent: &'a mut [f32],
}

/// Evaluate every prepared due brain with distinct observations, parameters, and state.
///
/// Inputs, outputs, and staged recurrent values are packed in the supplied work order.
/// The function validates the complete batch before writing and never mutates
/// authoritative brain state. If it returns an error, discard both staging buffers.
pub fn evaluate_heterogeneous_population(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    population: &[PopulationGenome],
    brains: &[BrainRuntimeState],
    buffers: HeterogeneousInferenceBuffers<'_>,
    scratch: &mut CalculationScratchView<'_>,
) -> Result<(), InferenceError> {
    let HeterogeneousInferenceBuffers {
        observations,
        staged_outputs,
        staged_recurrent,
    } = buffers;
    let expected_observations = checked_product(work.len(), plan.input_size, "batch observations")?;
    let expected_outputs = checked_product(work.len(), plan.output_size, "batch outputs")?;
    let expected_recurrent =
        checked_product(work.len(), plan.total_state_size, "batch recurrent staging")?;
    require_length(
        "batch observations",
        observations.len(),
        expected_observations,
    )?;
    require_length("batch outputs", staged_outputs.len(), expected_outputs)?;
    require_length(
        "batch recurrent staging",
        staged_recurrent.len(),
        expected_recurrent,
    )?;
    require_finite("batch observations", observations)?;
    require_minimum_length(
        "activation scratch",
        scratch.activation.len(),
        plan.scratch_layout.activation_floats,
    )?;
    require_minimum_length(
        "temporary scratch",
        scratch.temporary.len(),
        plan.scratch_layout.temporary_floats,
    )?;
    preflight_work(plan, work, population, brains)?;

    for (index, unit) in work.iter().enumerate() {
        let brain = &brains[unit.brain_index()];
        let weights = weights_for(unit, brain, population)?;
        let input_offset = index * plan.input_size;
        let output_offset = index * plan.output_size;
        let recurrent_offset = index * plan.total_state_size;
        plan.evaluate(
            weights,
            &brain.recurrent,
            &mut staged_recurrent[recurrent_offset..recurrent_offset + plan.total_state_size],
            &observations[input_offset..input_offset + plan.input_size],
            &mut staged_outputs[output_offset..output_offset + plan.output_size],
            scratch,
        )?;
    }
    Ok(())
}

/// Atomically publish one already successful batch's staged recurrent values.
///
/// All handles, lengths, and ordering are checked before the first copy, so an error
/// leaves every authoritative recurrent block unchanged.
pub fn commit_heterogeneous_recurrent(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    brains: &mut [BrainRuntimeState],
    staged_recurrent: &[f32],
) -> Result<(), InferenceError> {
    validate_heterogeneous_recurrent_commit(plan, work, brains, staged_recurrent)?;
    publish_heterogeneous_recurrent(plan, work, brains, staged_recurrent);
    Ok(())
}

/// Validate a complete recurrent-state publication without writing authority.
///
/// This is exposed so a coordinator can preflight recurrent state together
/// with observation-delivery markers and then publish both as one infallible
/// commit section.
pub fn validate_heterogeneous_recurrent_commit(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    brains: &[BrainRuntimeState],
    staged_recurrent: &[f32],
) -> Result<(), InferenceError> {
    let expected = checked_product(
        work.len(),
        plan.total_state_size,
        "commit recurrent staging",
    )?;
    require_length("commit recurrent staging", staged_recurrent.len(), expected)?;
    preflight_brains(plan, work, brains)?;
    require_finite("commit recurrent staging", staged_recurrent)?;
    Ok(())
}

/// Publish recurrent state after complete validation under exclusive authority.
pub(super) fn publish_heterogeneous_recurrent(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    brains: &mut [BrainRuntimeState],
    staged_recurrent: &[f32],
) {
    for (index, unit) in work.iter().enumerate() {
        let offset = index * plan.total_state_size;
        brains[unit.brain_index()]
            .recurrent
            .copy_from_slice(&staged_recurrent[offset..offset + plan.total_state_size]);
    }
}

/// Validate complete work-to-brain/weight resolution before any staged writes.
fn preflight_work(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    population: &[PopulationGenome],
    brains: &[BrainRuntimeState],
) -> Result<(), InferenceError> {
    preflight_brains(plan, work, brains)?;
    for unit in work {
        let brain = &brains[unit.brain_index()];
        let weights = weights_for(unit, brain, population)?;
        require_length("brain weights", weights.len(), plan.total_parameters)?;
    }
    Ok(())
}

/// Validate canonical work ordering and all recurrent destinations.
fn preflight_brains(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    brains: &[BrainRuntimeState],
) -> Result<(), InferenceError> {
    let mut previous = None;
    for (index, unit) in work.iter().enumerate() {
        if let Some(previous_handle) = previous {
            if previous_handle >= unit.brain() {
                return Err(InferenceError::NonCanonicalWorkOrder {
                    index,
                    previous: previous_handle,
                    current: unit.brain(),
                });
            }
        }
        previous = Some(unit.brain());
        let brain =
            brains
                .get(unit.brain_index())
                .ok_or(InferenceError::BrainIndexOutOfBounds {
                    index: unit.brain_index(),
                    brain_count: brains.len(),
                })?;
        if brain.handle != unit.brain() {
            return Err(InferenceError::BrainHandleMismatch {
                index: unit.brain_index(),
                expected: unit.brain(),
                actual: brain.handle,
            });
        }
        require_length(
            "brain recurrent state",
            brain.recurrent.len(),
            plan.total_state_size,
        )?;
    }
    Ok(())
}

/// Resolve the sole parameter owner for one prepared brain.
fn weights_for<'a>(
    unit: &CalculationWorkUnit,
    brain: &'a BrainRuntimeState,
    population: &'a [PopulationGenome],
) -> Result<&'a [f32], InferenceError> {
    match unit.population_slot() {
        Some(slot) => {
            if brain.owner != BrainOwner::PopulationSlot(slot) {
                return Err(InferenceError::BrainOwnerMismatch {
                    brain: brain.handle,
                    detail: format!("expected population slot {slot}"),
                });
            }
            let slot_index =
                usize::try_from(slot).map_err(|_| InferenceError::PopulationSlotOutOfBounds {
                    slot,
                    population_count: population.len(),
                })?;
            let genome =
                population
                    .get(slot_index)
                    .ok_or(InferenceError::PopulationSlotOutOfBounds {
                        slot,
                        population_count: population.len(),
                    })?;
            if genome.slot != slot || genome.brain != brain.handle {
                return Err(InferenceError::PopulationMappingMismatch {
                    slot,
                    brain: brain.handle,
                });
            }
            if brain.non_population_weights.is_some() {
                return Err(InferenceError::BrainOwnerMismatch {
                    brain: brain.handle,
                    detail: "population brain unexpectedly owns duplicate parameters".to_owned(),
                });
            }
            Ok(&genome.weights)
        }
        None => {
            if brain.owner != BrainOwner::Entity(unit.snake_id()) {
                return Err(InferenceError::BrainOwnerMismatch {
                    brain: brain.handle,
                    detail: format!("expected entity {}", unit.snake_id()),
                });
            }
            brain
                .non_population_weights
                .as_deref()
                .ok_or(InferenceError::MissingEntityWeights {
                    brain: brain.handle,
                })
        }
    }
}

/// Allocate one activation range with checked arithmetic.
fn allocate_tensor(cursor: &mut usize, len: usize) -> Result<TensorRange, InferenceError> {
    let range = TensorRange {
        offset: *cursor,
        len,
    };
    *cursor = range.end()?;
    Ok(range)
}

/// Recheck operation-specific runtime ranges before admitting an execution plan.
fn validate_node_shape(
    node: &CompiledNode,
    inputs: &[TensorRange],
    state: Option<TensorRange>,
) -> Result<(), InferenceError> {
    let invalid = |detail: String| InferenceError::InvalidCompiledGraph {
        detail: format!("node {} {detail}", node.id),
    };
    let require_inputs = |count: usize| {
        if inputs.len() == count {
            Ok(())
        } else {
            Err(invalid(format!(
                "has {} execution inputs; expected {count}",
                inputs.len()
            )))
        }
    };
    match node.node_type {
        CompiledNodeType::Input => {
            require_inputs(0)?;
            if node.parameter_length != 0
                || state.is_some()
                || !node.hidden_sizes.is_empty()
                || node.output_sizes.as_slice() != [node.output_size]
            {
                return Err(invalid(
                    "has parameters, state, or hidden layers".to_owned(),
                ));
            }
        }
        CompiledNodeType::Concat => {
            if inputs.is_empty() {
                return Err(invalid("has no execution inputs".to_owned()));
            }
            let width = inputs.iter().try_fold(0usize, |total, input| {
                total
                    .checked_add(input.len)
                    .ok_or(InferenceError::ArithmeticOverflow {
                        context: "Concat execution width",
                    })
            })?;
            if width != node.output_size
                || node.parameter_length != 0
                || state.is_some()
                || !node.hidden_sizes.is_empty()
                || node.output_sizes.as_slice() != [node.output_size]
            {
                return Err(invalid(
                    "has inconsistent width, parameters, or state".to_owned(),
                ));
            }
        }
        CompiledNodeType::Split => {
            require_inputs(1)?;
            if inputs[0].len != node.output_size
                || node.parameter_length != 0
                || state.is_some()
                || !node.hidden_sizes.is_empty()
            {
                return Err(invalid(
                    "has inconsistent width, parameters, or state".to_owned(),
                ));
            }
        }
        CompiledNodeType::Dense => {
            require_inputs(1)?;
            let row = node
                .input_size
                .checked_add(1)
                .ok_or(InferenceError::ArithmeticOverflow {
                    context: "Dense execution row",
                })?;
            let expected = checked_product(node.output_size, row, "Dense execution parameters")?;
            if inputs[0].len != node.input_size
                || node.parameter_length != expected
                || state.is_some()
                || !node.hidden_sizes.is_empty()
                || node.output_sizes.as_slice() != [node.output_size]
            {
                return Err(invalid(
                    "has inconsistent input, parameters, hidden layers, or state".to_owned(),
                ));
            }
        }
        CompiledNodeType::Mlp => {
            require_inputs(1)?;
            let mut expected = 0usize;
            let mut source_width = node.input_size;
            for destination_width in node
                .hidden_sizes
                .iter()
                .copied()
                .chain(std::iter::once(node.output_size))
            {
                let row =
                    source_width
                        .checked_add(1)
                        .ok_or(InferenceError::ArithmeticOverflow {
                            context: "MLP execution row",
                        })?;
                expected = expected
                    .checked_add(checked_product(
                        destination_width,
                        row,
                        "MLP execution layer parameters",
                    )?)
                    .ok_or(InferenceError::ArithmeticOverflow {
                        context: "MLP execution parameters",
                    })?;
                source_width = destination_width;
            }
            if inputs[0].len != node.input_size
                || node.parameter_length != expected
                || state.is_some()
                || node.output_sizes.as_slice() != [node.output_size]
            {
                return Err(invalid(
                    "has inconsistent input, parameters, or state".to_owned(),
                ));
            }
        }
        CompiledNodeType::Gru | CompiledNodeType::Lstm | CompiledNodeType::Rru => {
            require_inputs(1)?;
            let hidden = node
                .hidden_size
                .ok_or_else(|| invalid("has no hidden width".to_owned()))?;
            let gates = match node.node_type {
                CompiledNodeType::Gru => 3,
                CompiledNodeType::Lstm => 4,
                CompiledNodeType::Rru => 2,
                _ => unreachable!("outer match admits only recurrent types"),
            };
            let expected_parameters =
                recurrent_parameter_count(node.input_size, hidden, gates, "recurrent execution")?;
            let expected_state = if node.node_type == CompiledNodeType::Lstm {
                hidden
                    .checked_mul(2)
                    .ok_or(InferenceError::ArithmeticOverflow {
                        context: "LSTM execution state",
                    })?
            } else {
                hidden
            };
            if inputs[0].len != node.input_size
                || node.output_size != hidden
                || node.parameter_length != expected_parameters
                || state.map(|range| range.len) != Some(expected_state)
                || !node.hidden_sizes.is_empty()
                || node.output_sizes.as_slice() != [node.output_size]
            {
                return Err(invalid(
                    "has inconsistent input, output, parameters, hidden layers, or state"
                        .to_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// Return the sole source range required by non-Concat operations.
fn only_input(node: &ExecutionNode) -> Result<TensorRange, InferenceError> {
    if node.inputs.len() != 1 {
        return Err(InferenceError::InvalidExecutionPlan {
            node_id: node.id.clone(),
            detail: format!("expected one input, found {}", node.inputs.len()),
        });
    }
    Ok(node.inputs[0])
}

/// Return required recurrent metadata for a recurrent operation.
fn required_state(node: &ExecutionNode) -> Result<TensorRange, InferenceError> {
    node.state
        .ok_or_else(|| InferenceError::InvalidExecutionPlan {
            node_id: node.id.clone(),
            detail: "recurrent node has no state range".to_owned(),
        })
}

/// Borrow an earlier immutable activation and a later disjoint destination.
fn forward_slices<'a>(
    activation: &'a mut [f32],
    source: TensorRange,
    destination: TensorRange,
    node_id: &str,
) -> Result<(&'a [f32], &'a mut [f32]), InferenceError> {
    let source_end = source.end()?;
    let destination_end = destination.end()?;
    if source_end > destination.offset || destination_end > activation.len() {
        return Err(InferenceError::InvalidExecutionPlan {
            node_id: node_id.to_owned(),
            detail: "source is not wholly before its destination".to_owned(),
        });
    }
    let (prefix, destination_and_tail) = activation.split_at_mut(destination.offset);
    Ok((
        &prefix[source.offset..source_end],
        &mut destination_and_tail[..destination.len],
    ))
}

/// Apply one backend-selected dot product over equal, prevalidated slices.
#[inline]
fn dot_product(backend: InferenceMathBackend, weights: &[f32], input: &[f32]) -> f32 {
    debug_assert_eq!(weights.len(), input.len());
    match backend {
        InferenceMathBackend::Scalar => {
            let mut sum = 0.0f32;
            for index in 0..input.len() {
                sum += weights[index] * input[index];
            }
            sum
        }
        InferenceMathBackend::Sse2 => simd_kernels::dot_product_sse2(weights, input),
    }
}

/// Apply one backend-selected weighted-product dot over equal slices.
#[inline]
fn dot_product_mul(
    backend: InferenceMathBackend,
    weights: &[f32],
    left: &[f32],
    right: &[f32],
) -> f32 {
    debug_assert_eq!(weights.len(), left.len());
    debug_assert_eq!(left.len(), right.len());
    match backend {
        InferenceMathBackend::Scalar => {
            let mut sum = 0.0f32;
            for index in 0..left.len() {
                sum += weights[index] * (left[index] * right[index]);
            }
            sum
        }
        InferenceMathBackend::Sse2 => simd_kernels::dot_product_mul_sse2(weights, left, right),
    }
}

/// Apply one row-major affine transform followed by tanh.
fn affine_tanh(
    backend: InferenceMathBackend,
    weights: &[f32],
    input: &[f32],
    output: &mut [f32],
) -> Result<(), InferenceError> {
    let row = input
        .len()
        .checked_add(1)
        .ok_or(InferenceError::ArithmeticOverflow {
            context: "affine row width",
        })?;
    require_length(
        "affine weights",
        weights.len(),
        checked_product(output.len(), row, "affine weight count")?,
    )?;
    let mut cursor = 0usize;
    for value in output {
        let row_end = cursor + input.len();
        let sum = dot_product(backend, &weights[cursor..row_end], input) + weights[row_end];
        cursor = row_end + 1;
        *value = sum.tanh();
    }
    Ok(())
}

/// Apply the TypeScript-compatible sigmoid formula in `f32` arithmetic.
#[inline]
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

/// Evaluate one GRU block using the established Wz/Wr/Wh/Uz/Ur/Uh/bz/br/bh order.
fn evaluate_gru(
    backend: InferenceMathBackend,
    weights: &[f32],
    input: &[f32],
    before: &[f32],
    after: &mut [f32],
    output: &mut [f32],
    temporary: &mut [f32],
) -> Result<(), InferenceError> {
    let input_size = input.len();
    let hidden = before.len();
    require_length("GRU next state", after.len(), hidden)?;
    require_length("GRU output", output.len(), hidden)?;
    require_minimum_length(
        "GRU temporary",
        temporary.len(),
        checked_product(hidden, 2, "GRU temporary count")?,
    )?;
    let expected = recurrent_parameter_count(input_size, hidden, 3, "GRU")?;
    require_length("GRU weights", weights.len(), expected)?;

    let input_block = hidden * input_size;
    let state_block = hidden * hidden;
    let wz = 0usize;
    let wr = wz + input_block;
    let wh = wr + input_block;
    let uz = wh + input_block;
    let ur = uz + state_block;
    let uh = ur + state_block;
    let bz = uh + state_block;
    let br = bz + hidden;
    let bh = br + hidden;
    let (z, remainder) = temporary.split_at_mut(hidden);
    let r = &mut remainder[..hidden];

    for j in 0..hidden {
        let wz_row = wz + j * input_size;
        let wr_row = wr + j * input_size;
        let uz_row = uz + j * hidden;
        let ur_row = ur + j * hidden;
        let sum_z = dot_product(backend, &weights[wz_row..wz_row + input_size], input)
            + dot_product(backend, &weights[uz_row..uz_row + hidden], before);
        let sum_r = dot_product(backend, &weights[wr_row..wr_row + input_size], input)
            + dot_product(backend, &weights[ur_row..ur_row + hidden], before);
        z[j] = sigmoid(sum_z + weights[bz + j]);
        r[j] = sigmoid(sum_r + weights[br + j]);
    }
    for j in 0..hidden {
        let wh_row = wh + j * input_size;
        let uh_row = uh + j * hidden;
        let sum_h = dot_product(backend, &weights[wh_row..wh_row + input_size], input)
            + dot_product_mul(backend, &weights[uh_row..uh_row + hidden], r, before);
        let candidate = (sum_h + weights[bh + j]).tanh();
        let next = (1.0 - z[j]) * before[j] + z[j] * candidate;
        after[j] = next;
        output[j] = next;
    }
    Ok(())
}

/// Evaluate one LSTM block using the established Wi/Wf/Wo/Wg/Ui/Uf/Uo/Ug order.
fn evaluate_lstm(
    backend: InferenceMathBackend,
    weights: &[f32],
    input: &[f32],
    before: &[f32],
    after: &mut [f32],
    output: &mut [f32],
) -> Result<(), InferenceError> {
    if !before.len().is_multiple_of(2) {
        return Err(InferenceError::InvalidRecurrentShape {
            kind: "LSTM",
            state_floats: before.len(),
        });
    }
    let input_size = input.len();
    let hidden = before.len() / 2;
    require_length("LSTM next state", after.len(), before.len())?;
    require_length("LSTM output", output.len(), hidden)?;
    let expected = recurrent_parameter_count(input_size, hidden, 4, "LSTM")?;
    require_length("LSTM weights", weights.len(), expected)?;
    let (previous_h, previous_c) = before.split_at(hidden);
    let (next_h, next_c) = after.split_at_mut(hidden);

    let input_block = hidden * input_size;
    let state_block = hidden * hidden;
    let wi = 0usize;
    let wf = wi + input_block;
    let wo = wf + input_block;
    let wg = wo + input_block;
    let ui = wg + input_block;
    let uf = ui + state_block;
    let uo = uf + state_block;
    let ug = uo + state_block;
    let bi = ug + state_block;
    let bf = bi + hidden;
    let bo = bf + hidden;
    let bg = bo + hidden;

    for j in 0..hidden {
        let wi_row = wi + j * input_size;
        let wf_row = wf + j * input_size;
        let wo_row = wo + j * input_size;
        let wg_row = wg + j * input_size;
        let ui_row = ui + j * hidden;
        let uf_row = uf + j * hidden;
        let uo_row = uo + j * hidden;
        let ug_row = ug + j * hidden;
        let sum_i = dot_product(backend, &weights[wi_row..wi_row + input_size], input)
            + dot_product(backend, &weights[ui_row..ui_row + hidden], previous_h);
        let sum_f = dot_product(backend, &weights[wf_row..wf_row + input_size], input)
            + dot_product(backend, &weights[uf_row..uf_row + hidden], previous_h);
        let sum_o = dot_product(backend, &weights[wo_row..wo_row + input_size], input)
            + dot_product(backend, &weights[uo_row..uo_row + hidden], previous_h);
        let sum_g = dot_product(backend, &weights[wg_row..wg_row + input_size], input)
            + dot_product(backend, &weights[ug_row..ug_row + hidden], previous_h);
        let input_gate = sigmoid(sum_i + weights[bi + j]);
        let forget_gate = sigmoid(sum_f + weights[bf + j]);
        let output_gate = sigmoid(sum_o + weights[bo + j]);
        let candidate = (sum_g + weights[bg + j]).tanh();
        let cell = forget_gate * previous_c[j] + input_gate * candidate;
        let hidden_value = output_gate * cell.tanh();
        next_c[j] = cell;
        next_h[j] = hidden_value;
        output[j] = hidden_value;
    }
    Ok(())
}

/// Evaluate one RRU block using the established Wc/Wr/Uc/Ur/bc/br order.
fn evaluate_rru(
    backend: InferenceMathBackend,
    weights: &[f32],
    input: &[f32],
    before: &[f32],
    after: &mut [f32],
    output: &mut [f32],
) -> Result<(), InferenceError> {
    let input_size = input.len();
    let hidden = before.len();
    require_length("RRU next state", after.len(), hidden)?;
    require_length("RRU output", output.len(), hidden)?;
    let expected = recurrent_parameter_count(input_size, hidden, 2, "RRU")?;
    require_length("RRU weights", weights.len(), expected)?;

    let input_block = hidden * input_size;
    let state_block = hidden * hidden;
    let wc = 0usize;
    let wr = wc + input_block;
    let uc = wr + input_block;
    let ur = uc + state_block;
    let bc = ur + state_block;
    let br = bc + hidden;
    for j in 0..hidden {
        let wc_row = wc + j * input_size;
        let wr_row = wr + j * input_size;
        let uc_row = uc + j * hidden;
        let ur_row = ur + j * hidden;
        let sum_c = dot_product(backend, &weights[wc_row..wc_row + input_size], input)
            + dot_product(backend, &weights[uc_row..uc_row + hidden], before);
        let sum_r = dot_product(backend, &weights[wr_row..wr_row + input_size], input)
            + dot_product(backend, &weights[ur_row..ur_row + hidden], before);
        let candidate = (sum_c + weights[bc + j]).tanh();
        let gate = sigmoid(sum_r + weights[br + j]);
        let next = (1.0 - gate) * before[j] + gate * candidate;
        after[j] = next;
        output[j] = next;
    }
    Ok(())
}

/// Checked recurrent parameter count shared by scalar operations.
fn recurrent_parameter_count(
    input: usize,
    hidden: usize,
    gates: usize,
    context: &'static str,
) -> Result<usize, InferenceError> {
    let row = input
        .checked_add(hidden)
        .and_then(|value| value.checked_add(1))
        .ok_or(InferenceError::ArithmeticOverflow { context })?;
    checked_product(gates, checked_product(hidden, row, context)?, context)
}

/// Borrow an immutable checked range.
fn slice_range(values: &[f32], range: TensorRange) -> Result<&[f32], InferenceError> {
    values
        .get(range.offset..range.end()?)
        .ok_or(InferenceError::InvalidExecutionPlan {
            node_id: "<range>".to_owned(),
            detail: "immutable range is out of bounds".to_owned(),
        })
}

/// Borrow a mutable checked range.
fn slice_range_mut(values: &mut [f32], range: TensorRange) -> Result<&mut [f32], InferenceError> {
    let end = range.end()?;
    values
        .get_mut(range.offset..end)
        .ok_or(InferenceError::InvalidExecutionPlan {
            node_id: "<range>".to_owned(),
            detail: "mutable range is out of bounds".to_owned(),
        })
}

/// Require one exact buffer length.
fn require_length(
    buffer: &'static str,
    actual: usize,
    expected: usize,
) -> Result<(), InferenceError> {
    if actual != expected {
        return Err(InferenceError::BufferLength {
            buffer,
            actual,
            expected,
        });
    }
    Ok(())
}

/// Require at least one fixed scratch length.
fn require_minimum_length(
    buffer: &'static str,
    actual: usize,
    required: usize,
) -> Result<(), InferenceError> {
    if actual < required {
        return Err(InferenceError::ScratchTooSmall {
            buffer,
            actual,
            required,
        });
    }
    Ok(())
}

/// Reject a non-finite observation or staged result before authoritative commit.
fn require_finite(buffer: &'static str, values: &[f32]) -> Result<(), InferenceError> {
    if let Some((index, value)) = values
        .iter()
        .copied()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(InferenceError::NonFiniteValue {
            buffer,
            index,
            bits: value.to_bits(),
        });
    }
    Ok(())
}

/// Checked product for all packed batch sizes.
fn checked_product(
    left: usize,
    right: usize,
    context: &'static str,
) -> Result<usize, InferenceError> {
    left.checked_mul(right)
        .ok_or(InferenceError::ArithmeticOverflow { context })
}

/// Checked graph and heterogeneous-batch execution failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceError {
    /// An explicitly requested numeric backend is unavailable on this CPU.
    UnavailableMathBackend {
        /// Stable requested backend label.
        backend: &'static str,
    },
    /// A derived count or range overflowed.
    ArithmeticOverflow {
        /// Static operation being calculated.
        context: &'static str,
    },
    /// A fixed runtime-plan allocation failed.
    AllocationFailed {
        /// Allocation role.
        buffer: &'static str,
        /// Requested elements.
        elements: usize,
    },
    /// Compiled metadata was internally inconsistent.
    InvalidCompiledGraph {
        /// Human-readable inconsistency.
        detail: String,
    },
    /// A runtime node range violated the precomputed plan.
    InvalidExecutionPlan {
        /// Affected node or synthetic range label.
        node_id: String,
        /// Human-readable inconsistency.
        detail: String,
    },
    /// A requested focused node is absent from the immutable plan.
    ActivationNodeNotFound {
        /// Requested graph-node identifier.
        node_id: String,
    },
    /// A capture token came from a different compiled graph layout.
    ActivationLayoutMismatch {
        /// Selected graph-node identifier.
        node_id: String,
    },
    /// One exact typed buffer had the wrong length.
    BufferLength {
        /// Buffer role.
        buffer: &'static str,
        /// Actual floats.
        actual: usize,
        /// Required floats.
        expected: usize,
    },
    /// Reusable scratch was smaller than its admitted execution plan.
    ScratchTooSmall {
        /// Scratch role.
        buffer: &'static str,
        /// Supplied floats.
        actual: usize,
        /// Minimum floats.
        required: usize,
    },
    /// An observation or staged result contained NaN or infinity.
    NonFiniteValue {
        /// Buffer role.
        buffer: &'static str,
        /// First invalid float index.
        index: usize,
        /// Exact invalid IEEE-754 bits.
        bits: u32,
    },
    /// A recurrent block had an impossible structure.
    InvalidRecurrentShape {
        /// Recurrent operation.
        kind: &'static str,
        /// Supplied state floats.
        state_floats: usize,
    },
    /// Due work was not in the canonical strictly increasing brain order.
    NonCanonicalWorkOrder {
        /// First invalid array position.
        index: usize,
        /// Prior stable brain.
        previous: super::state::BrainHandle,
        /// Current stable brain.
        current: super::state::BrainHandle,
    },
    /// A prepared brain index no longer resolved.
    BrainIndexOutOfBounds {
        /// Requested index.
        index: usize,
        /// Available records.
        brain_count: usize,
    },
    /// Prepared work and the brain slab disagreed on stable identity.
    BrainHandleMismatch {
        /// Brain array index.
        index: usize,
        /// Prepared handle.
        expected: super::state::BrainHandle,
        /// Current handle.
        actual: super::state::BrainHandle,
    },
    /// Brain ownership disagreed with the prepared work class.
    BrainOwnerMismatch {
        /// Stable brain identity.
        brain: super::state::BrainHandle,
        /// Expected ownership description.
        detail: String,
    },
    /// A dense population slot did not resolve.
    PopulationSlotOutOfBounds {
        /// Requested slot.
        slot: u32,
        /// Available genomes.
        population_count: usize,
    },
    /// Population record and brain slab did not retain one mapping.
    PopulationMappingMismatch {
        /// Dense slot.
        slot: u32,
        /// Stable brain handle.
        brain: super::state::BrainHandle,
    },
    /// An entity-owned brain had no parameters.
    MissingEntityWeights {
        /// Stable brain handle.
        brain: super::state::BrainHandle,
    },
}

impl fmt::Display for InferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnavailableMathBackend { backend } => {
                write!(formatter, "inference math backend {backend} is unavailable")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "inference arithmetic overflow: {context}")
            }
            Self::AllocationFailed { buffer, elements } => {
                write!(
                    formatter,
                    "unable to allocate {elements} elements for {buffer}"
                )
            }
            Self::InvalidCompiledGraph { detail } => {
                write!(formatter, "invalid compiled graph: {detail}")
            }
            Self::InvalidExecutionPlan { node_id, detail } => {
                write!(formatter, "invalid execution plan at {node_id}: {detail}")
            }
            Self::ActivationNodeNotFound { node_id } => {
                write!(
                    formatter,
                    "activation node {node_id} is absent from the graph"
                )
            }
            Self::ActivationLayoutMismatch { node_id } => write!(
                formatter,
                "activation node {node_id} was prepared for a different graph layout"
            ),
            Self::BufferLength {
                buffer,
                actual,
                expected,
            } => write!(
                formatter,
                "{buffer} has {actual} floats; expected exactly {expected}"
            ),
            Self::ScratchTooSmall {
                buffer,
                actual,
                required,
            } => write!(
                formatter,
                "{buffer} has {actual} floats; at least {required} are required"
            ),
            Self::NonFiniteValue {
                buffer,
                index,
                bits,
            } => write!(
                formatter,
                "{buffer}[{index}] is not finite (f32 bits 0x{bits:08x})"
            ),
            Self::InvalidRecurrentShape { kind, state_floats } => {
                write!(formatter, "{kind} state has invalid length {state_floats}")
            }
            Self::NonCanonicalWorkOrder {
                index,
                previous,
                current,
            } => write!(
                formatter,
                "work item {index} brain {current:?} does not follow {previous:?}"
            ),
            Self::BrainIndexOutOfBounds { index, brain_count } => write!(
                formatter,
                "brain index {index} is outside {brain_count} records"
            ),
            Self::BrainHandleMismatch {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "brain index {index} maps to {actual:?}, not prepared handle {expected:?}"
            ),
            Self::BrainOwnerMismatch { brain, detail } => {
                write!(formatter, "brain {brain:?} ownership mismatch: {detail}")
            }
            Self::PopulationSlotOutOfBounds {
                slot,
                population_count,
            } => write!(
                formatter,
                "population slot {slot} is outside {population_count} genomes"
            ),
            Self::PopulationMappingMismatch { slot, brain } => write!(
                formatter,
                "population slot {slot} does not map to brain {brain:?}"
            ),
            Self::MissingEntityWeights { brain } => {
                write!(formatter, "entity-owned brain {brain:?} has no parameters")
            }
        }
    }
}

impl Error for InferenceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::calculation::{
        CalculationBatchKey, CalculationExecutionBuffers, CalculationScratch, CalculationWorkspace,
    };
    use crate::engine::graph::{
        compile_graph, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::state::{
        BaselineStrategyState, BodyRange, BrainHandle, GenomeLineage, SnakeKind, SnakeState,
        WorldPoint,
    };
    use std::collections::BTreeSet;

    const EPOCH: u64 = 7;

    /// Limits large enough for deterministic unit fixtures without implying production defaults.
    fn limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 64,
            max_edges: 128,
            max_graph_outputs: 8,
            max_identifier_bytes: 64,
            max_total_referenced_identifier_bytes: 16_384,
            max_tensor_width: 512,
            max_mlp_hidden_layers: 8,
            max_split_output_ports: 8,
            max_parameter_floats: 1_000_000,
            max_recurrent_state_floats: 16_384,
            max_canonical_layout_bytes: 1_000_000,
            max_architecture_key_bytes: 2_000_000,
        }
    }

    /// Graph covering Input, Split, Dense, MLP, Concat, GRU, LSTM, RRU, and output mapping.
    fn complete_graph() -> CompiledGraph {
        compile_graph(
            &GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "in".to_owned(),
                        kind: GraphNodeKind::Input { output_size: 2 },
                    },
                    GraphNodeSpec {
                        id: "split".to_owned(),
                        kind: GraphNodeKind::Split {
                            output_sizes: vec![1, 1],
                        },
                    },
                    GraphNodeSpec {
                        id: "denseA".to_owned(),
                        kind: GraphNodeKind::Dense {
                            input_size: 1,
                            output_size: 1,
                        },
                    },
                    GraphNodeSpec {
                        id: "mlpB".to_owned(),
                        kind: GraphNodeKind::Mlp {
                            input_size: 1,
                            hidden_sizes: vec![2],
                            output_size: 1,
                        },
                    },
                    GraphNodeSpec {
                        id: "features".to_owned(),
                        kind: GraphNodeKind::Concat,
                    },
                    GraphNodeSpec {
                        id: "gru".to_owned(),
                        kind: GraphNodeKind::Gru {
                            input_size: 2,
                            hidden_size: 1,
                        },
                    },
                    GraphNodeSpec {
                        id: "lstm".to_owned(),
                        kind: GraphNodeKind::Lstm {
                            input_size: 2,
                            hidden_size: 1,
                        },
                    },
                    GraphNodeSpec {
                        id: "rru".to_owned(),
                        kind: GraphNodeKind::Rru {
                            input_size: 2,
                            hidden_size: 1,
                        },
                    },
                    GraphNodeSpec {
                        id: "memory".to_owned(),
                        kind: GraphNodeKind::Concat,
                    },
                    GraphNodeSpec {
                        id: "head".to_owned(),
                        kind: GraphNodeKind::Dense {
                            input_size: 3,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![
                    edge("in", "split", None, None),
                    edge("split", "denseA", Some(0), None),
                    edge("split", "mlpB", Some(1), None),
                    edge("denseA", "features", None, Some(0)),
                    edge("mlpB", "features", None, Some(1)),
                    edge("features", "gru", None, None),
                    edge("features", "lstm", None, None),
                    edge("features", "rru", None, None),
                    edge("gru", "memory", None, Some(0)),
                    edge("lstm", "memory", None, Some(1)),
                    edge("rru", "memory", None, Some(2)),
                    edge("memory", "head", None, None),
                ],
                outputs: vec![GraphOutputRef {
                    node_id: "head".to_owned(),
                    port: None,
                }],
                output_size: 2,
            },
            &limits(),
        )
        .expect("complete graph compiles")
    }

    /// Hidden-width-two graph that exposes cross-hidden recurrent indexing mistakes.
    fn wide_recurrent_graph() -> CompiledGraph {
        compile_graph(
            &GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".to_owned(),
                        kind: GraphNodeKind::Input { output_size: 3 },
                    },
                    GraphNodeSpec {
                        id: "gru".to_owned(),
                        kind: GraphNodeKind::Gru {
                            input_size: 3,
                            hidden_size: 2,
                        },
                    },
                    GraphNodeSpec {
                        id: "lstm".to_owned(),
                        kind: GraphNodeKind::Lstm {
                            input_size: 3,
                            hidden_size: 2,
                        },
                    },
                    GraphNodeSpec {
                        id: "rru".to_owned(),
                        kind: GraphNodeKind::Rru {
                            input_size: 3,
                            hidden_size: 2,
                        },
                    },
                    GraphNodeSpec {
                        id: "memory".to_owned(),
                        kind: GraphNodeKind::Concat,
                    },
                    GraphNodeSpec {
                        id: "head".to_owned(),
                        kind: GraphNodeKind::Dense {
                            input_size: 6,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![
                    edge("input", "gru", None, None),
                    edge("input", "lstm", None, None),
                    edge("input", "rru", None, None),
                    edge("gru", "memory", None, Some(0)),
                    edge("lstm", "memory", None, Some(1)),
                    edge("rru", "memory", None, Some(2)),
                    edge("memory", "head", None, None),
                ],
                outputs: vec![GraphOutputRef {
                    node_id: "head".to_owned(),
                    port: None,
                }],
                output_size: 2,
            },
            &limits(),
        )
        .expect("wide recurrent graph compiles")
    }

    /// Construct one graph edge with explicit optional ports.
    fn edge(from: &str, to: &str, from_port: Option<i64>, to_port: Option<i64>) -> GraphEdge {
        GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            from_port,
            to_port,
        }
    }

    /// Current-TypeScript parity weights, quantized exactly through `Float32Array.from`.
    fn parity_weights(count: usize) -> Vec<f32> {
        (0..count)
            .map(|index| (((index * 37) % 101) as i32 - 50) as f32 / 200.0)
            .collect()
    }

    /// Allocate admitted scratch for one execution plan.
    fn scratch(plan: &GraphExecutionPlan) -> CalculationScratch {
        CalculationScratch::try_new(plan.scratch_layout(), usize::MAX)
            .expect("fixture scratch allocation")
    }

    /// Compare explicit `f32` tolerance fixtures with useful failure context.
    fn assert_close(label: &str, actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len(), "{label} length");
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            let difference = (*actual - *expected).abs();
            assert!(
                difference <= 2.0e-6,
                "{label}[{index}] actual={actual} expected={expected} difference={difference}"
            );
        }
    }

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
            GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)
                .unwrap();
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
            GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)
                .unwrap();
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
            GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)
                .unwrap();
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
            let simd =
                GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Sse2)
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

    /// One fully populated evolved snake record for calculation-work resolution.
    fn snake(slot: usize, handle: BrainHandle) -> SnakeState {
        SnakeState {
            id: 10_000 + slot as u64,
            frame_v1_id: u32::try_from(slot + 1).unwrap(),
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(u32::try_from(slot).unwrap()),
            brain: Some(handle),
            baseline_slot: None,
            baseline_strategy: None::<BaselineStrategyState>,
            position: WorldPoint { x: 0.0, y: 0.0 },
            previous_position: WorldPoint { x: 0.0, y: 0.0 },
            direction: 0.0,
            radius: 1.0,
            speed: 0.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange { start: 0, len: 0 },
            skin: 0,
        }
    }

    /// Build 55 distinct genomes and recurrent states using stable dense slots.
    fn population_fixture(
        plan: &GraphExecutionPlan,
        count: usize,
    ) -> (
        Vec<SnakeState>,
        Vec<BrainRuntimeState>,
        Vec<PopulationGenome>,
    ) {
        let base = parity_weights(plan.total_parameters());
        let mut snakes = Vec::with_capacity(count);
        let mut brains = Vec::with_capacity(count);
        let mut population = Vec::with_capacity(count);
        for slot in 0..count {
            let handle = BrainHandle {
                id: 100 + slot as u64,
                epoch: EPOCH,
            };
            snakes.push(snake(slot, handle));
            brains.push(BrainRuntimeState {
                handle,
                owner: BrainOwner::PopulationSlot(u32::try_from(slot).unwrap()),
                non_population_weights: None,
                recurrent: (0..plan.total_state_size())
                    .map(|state_index| (slot * 7 + state_index) as f32 / 10_000.0)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            });
            population.push(PopulationGenome {
                slot: u32::try_from(slot).unwrap(),
                brain: handle,
                lineage: GenomeLineage {
                    genome_id: 1_000 + slot as u64,
                    birth_generation: 1,
                    parent_a: None,
                    parent_b: None,
                },
                fitness: 0.0,
                weights: base
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        *value + (slot as f32 + 1.0) * 0.000_13 + index as f32 * 0.000_001
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            });
        }
        (snakes, brains, population)
    }

    /// Prepare deterministic work after accepting candidates in the supplied order.
    fn prepared_workspace(
        plan: &GraphExecutionPlan,
        order: &[usize],
        snakes: &[SnakeState],
        brains: &[BrainRuntimeState],
        population: &[PopulationGenome],
    ) -> CalculationWorkspace<[f32; 2]> {
        let mut workspace =
            CalculationWorkspace::try_new(snakes.len(), 1, plan.scratch_layout(), usize::MAX)
                .unwrap();
        workspace.begin(CalculationBatchKey::new(1, 1, EPOCH));
        for index in order {
            workspace.try_push_candidate(*index, *index).unwrap();
        }
        workspace.prepare(snakes, brains, population).unwrap();
        workspace
    }

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
            non_population_weights: Some(
                parity_weights(plan.total_parameters()).into_boxed_slice(),
            ),
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
}
