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

#[cfg(feature = "engine-test-hooks")]
pub mod feedback_diagnostics;
mod matrix_i16;
#[cfg(test)]
mod mixed_tests;
pub(crate) use matrix_i16::i16_cache_admission_bytes;
pub(crate) use matrix_i16::matrix_i16_cache_admission_bytes;
pub use matrix_i16::QuantizedInferenceWeights;
use matrix_i16::{BoundCache, NodeWeights};

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
    matrix_start: usize,
    /// Packed inference-cache start; canonical parameter offsets remain unchanged.
    cache_offset: usize,
}

/// Numeric implementation selected once for one immutable execution plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InferenceMathBackend {
    /// Float32 scalar reference with compensated long rows and ordered short rows.
    Scalar,
    /// Four-lane SSE dot kernels; exact reduction order is source/build-bound.
    Sse2,
    /// Matrix-scaled symmetric i16 storage, four-lane Float32 FMA accumulation.
    I16MatrixFma128,
    /// Dense/MLP matrix-i16 FMA128; all recurrent nodes use canonical F32/SSE2.
    I16AffineFma128,
    /// Benchmark comparator only; never admitted from persisted backend metadata.
    #[cfg(feature = "engine-test-hooks")]
    BenchmarkF32Fma128,
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

    /// Stable evidence/health category; source/build identity pins exact arithmetic.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scalar => "rust-scalar-v1",
            Self::Sse2 => "rust-sse2-v1",
            Self::I16MatrixFma128 => "rust-i16-matrix-fma128-v1",
            Self::I16AffineFma128 => "rust-i16-affine-fma128-sse2-v1",
            #[cfg(feature = "engine-test-hooks")]
            Self::BenchmarkF32Fma128 => "benchmark-f32-fma128-v1",
        }
    }

    /// Resolve the persisted backend category under its recorded source/build contract.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "rust-scalar-v1" => Some(Self::Scalar),
            "rust-sse2-v1" => Some(Self::Sse2),
            "rust-i16-matrix-fma128-v1" => Some(Self::I16MatrixFma128),
            "rust-i16-affine-fma128-sse2-v1" => Some(Self::I16AffineFma128),
            _ => None,
        }
    }

    /// Return whether this exact implementation is admitted on the current CPU.
    pub fn is_available(self) -> bool {
        match self {
            Self::Scalar => true,
            Self::Sse2 => simd_kernels::runtime_sse2_available(),
            Self::I16MatrixFma128 | Self::I16AffineFma128 => {
                simd_kernels::runtime_i16_fma128_available()
            }
            #[cfg(feature = "engine-test-hooks")]
            Self::BenchmarkF32Fma128 => simd_kernels::runtime_i16_fma128_available(),
        }
    }

    /// Both opt-in variants require a prepared, backend-specific inference cache.
    pub const fn uses_i16_cache(self) -> bool {
        matches!(self, Self::I16MatrixFma128 | Self::I16AffineFma128)
    }
}

/// Immutable allocation and traversal plan derived once from a compiled graph.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphExecutionPlan {
    layout_digest_sha256: [u8; 32],
    input_size: usize,
    output_size: usize,
    total_parameters: usize,
    total_state_size: usize,
    math_backend: InferenceMathBackend,
    nodes: Vec<ExecutionNode>,
    outputs: Vec<TensorRange>,
    matrices: Vec<TensorRange>,
    caches: Vec<BoundCache>,
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

/// One browser-visible layer in a complete focused visualization plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualizationLayerPlan {
    source: Option<TensorRange>,
    activation_offset: Option<usize>,
    count: usize,
    recurrent: bool,
}

impl VisualizationLayerPlan {
    /// Number of neurons drawn in this layer.
    pub const fn count(self) -> usize {
        self.count
    }

    /// Whether this layer represents recurrent memory.
    pub const fn is_recurrent(self) -> bool {
        self.recurrent
    }

    /// Offset into the packed captured values, or none for a structure-only layer.
    pub const fn has_activations(self) -> bool {
        self.source.is_some()
    }
}

/// Precomputed ranges for one opt-in complete-graph visualization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualizationCapturePlan {
    layout_digest_sha256: [u8; 32],
    layers: Vec<VisualizationLayerPlan>,
    total_len: usize,
}

impl VisualizationCapturePlan {
    /// Ordered browser-visible layer layouts.
    pub fn layers(&self) -> &[VisualizationLayerPlan] {
        &self.layers
    }

    /// Exact number of captured Float32 activation values.
    pub const fn len(&self) -> usize {
        self.total_len
    }

    /// Whether the graph exposes no visualizable activations.
    pub const fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
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
                matrix_start: 0,
                cache_offset: 0,
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

        let matrices = matrix_i16::matrix_layout(&mut execution_nodes, math_backend);
        Ok(Self {
            matrices,
            caches: Vec::new(),
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

    /// Precompute every layer needed by the existing browser visualizer.
    pub fn prepare_visualization_capture(
        &self,
    ) -> Result<VisualizationCapturePlan, InferenceError> {
        let mut layers = Vec::new();
        let maximum_layers = self.nodes.iter().try_fold(0usize, |count, node| {
            let added = match node.node_type {
                CompiledNodeType::Mlp => node.hidden.len().saturating_add(2),
                CompiledNodeType::Dense
                | CompiledNodeType::Gru
                | CompiledNodeType::Lstm
                | CompiledNodeType::Rru => 1,
                CompiledNodeType::Input | CompiledNodeType::Concat | CompiledNodeType::Split => 0,
            };
            count
                .checked_add(added)
                .ok_or(InferenceError::ArithmeticOverflow {
                    context: "visualization layer count",
                })
        })?;
        layers
            .try_reserve_exact(maximum_layers)
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "visualization layer plans",
                elements: maximum_layers,
            })?;
        let mut value_offset = 0usize;
        for node in &self.nodes {
            match node.node_type {
                CompiledNodeType::Mlp => {
                    let input_count = node.inputs.iter().try_fold(0usize, |count, input| {
                        count
                            .checked_add(input.len)
                            .ok_or(InferenceError::ArithmeticOverflow {
                                context: "visualization MLP input width",
                            })
                    })?;
                    layers.push(VisualizationLayerPlan {
                        source: None,
                        activation_offset: None,
                        count: input_count,
                        recurrent: false,
                    });
                    for range in node.hidden.iter().chain(std::iter::once(&node.output)) {
                        let activation_offset = value_offset;
                        value_offset = value_offset.checked_add(range.len).ok_or(
                            InferenceError::ArithmeticOverflow {
                                context: "visualization activation width",
                            },
                        )?;
                        layers.push(VisualizationLayerPlan {
                            source: Some(*range),
                            activation_offset: Some(activation_offset),
                            count: range.len,
                            recurrent: false,
                        });
                    }
                }
                CompiledNodeType::Dense
                | CompiledNodeType::Gru
                | CompiledNodeType::Lstm
                | CompiledNodeType::Rru => {
                    let activation_offset = value_offset;
                    value_offset = value_offset.checked_add(node.output.len).ok_or(
                        InferenceError::ArithmeticOverflow {
                            context: "visualization activation width",
                        },
                    )?;
                    layers.push(VisualizationLayerPlan {
                        source: Some(node.output),
                        activation_offset: Some(activation_offset),
                        count: node.output.len,
                        recurrent: matches!(
                            node.node_type,
                            CompiledNodeType::Gru | CompiledNodeType::Lstm | CompiledNodeType::Rru
                        ),
                    });
                }
                CompiledNodeType::Input | CompiledNodeType::Concat | CompiledNodeType::Split => {}
            }
        }
        Ok(VisualizationCapturePlan {
            layout_digest_sha256: self.layout_digest_sha256,
            layers,
            total_len: value_offset,
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

    /// Copy every browser-visible activation from one completed focused evaluation.
    pub fn capture_visualization(
        &self,
        capture: &VisualizationCapturePlan,
        scratch: &CalculationScratchView<'_>,
        destination: &mut [f32],
    ) -> Result<(), InferenceError> {
        if capture.layout_digest_sha256 != self.layout_digest_sha256 {
            return Err(InferenceError::ActivationLayoutMismatch {
                node_id: "complete visualization".to_owned(),
            });
        }
        require_length(
            "visualization capture",
            destination.len(),
            capture.total_len,
        )?;
        for layer in capture.layers.iter().filter(|layer| layer.source.is_some()) {
            let source = layer.source.expect("filtered activation layer");
            let destination_offset = layer
                .activation_offset
                .expect("activation layer must have a packed offset");
            let source_end = source.end()?;
            let destination_end = destination_offset.checked_add(source.len).ok_or(
                InferenceError::ArithmeticOverflow {
                    context: "visualization capture destination",
                },
            )?;
            require_minimum_length("activation scratch", scratch.activation.len(), source_end)?;
            destination[destination_offset..destination_end]
                .copy_from_slice(&scratch.activation[source.offset..source_end]);
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
        self.evaluate_inner(
            weights,
            None,
            recurrent_before,
            recurrent_after,
            input,
            output,
            scratch,
        )
    }

    /// Evaluate using a cache built once from these canonical parameters.
    /// The cache is graph-bound; callers must rebuild it whenever weights change.
    // The existing complete-graph staging API plus one explicit cache argument.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_quantized(
        &self,
        weights: &[f32],
        cache: &QuantizedInferenceWeights,
        recurrent_before: &[f32],
        recurrent_after: &mut [f32],
        input: &[f32],
        output: &mut [f32],
        scratch: &mut CalculationScratchView<'_>,
    ) -> Result<(), InferenceError> {
        if cache.layout != self.layout_digest_sha256 || cache.backend != self.math_backend {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: "i16 cache graph/backend mismatch".into(),
            });
        }
        self.evaluate_inner(
            weights,
            Some(cache),
            recurrent_before,
            recurrent_after,
            input,
            output,
            scratch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_inner(
        &self,
        weights: &[f32],
        cache: Option<&QuantizedInferenceWeights>,
        recurrent_before: &[f32],
        recurrent_after: &mut [f32],
        input: &[f32],
        output: &mut [f32],
        scratch: &mut CalculationScratchView<'_>,
    ) -> Result<(), InferenceError> {
        if self.math_backend.uses_i16_cache() != cache.is_some() {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: "i16 execution requires a prepared cache and the i16 backend".into(),
            });
        }
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
                    let parameters = NodeWeights::new_at(
                        weights,
                        cache,
                        node.parameter,
                        node.matrix_start,
                        node.cache_offset,
                    );
                    let (input_values, output_values) =
                        forward_slices(scratch.activation, source, node.output, &node.id)?;
                    affine_tanh(self.math_backend, parameters, input_values, output_values)?;
                }
                CompiledNodeType::Mlp => {
                    self.evaluate_mlp(node, weights, cache, scratch.activation)?;
                }
                CompiledNodeType::Gru => {
                    let source = only_input(node)?;
                    let parameters = NodeWeights::new(
                        weights,
                        self.recurrent_cache(cache),
                        node.parameter,
                        node.matrix_start,
                    );
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
                    let parameters = NodeWeights::new(
                        weights,
                        self.recurrent_cache(cache),
                        node.parameter,
                        node.matrix_start,
                    );
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
                    let parameters = NodeWeights::new(
                        weights,
                        self.recurrent_cache(cache),
                        node.parameter,
                        node.matrix_start,
                    );
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
        cache: Option<&QuantizedInferenceWeights>,
        activation: &mut [f32],
    ) -> Result<(), InferenceError> {
        let mut source = only_input(node)?;
        let mut parameter_cursor = node.parameter.offset;
        for (layer, destination) in node
            .hidden
            .iter()
            .chain(std::iter::once(&node.output))
            .enumerate()
        {
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
                NodeWeights::new_at(
                    weights,
                    cache,
                    TensorRange {
                        offset: parameter_cursor,
                        len: layer_parameters,
                    },
                    node.matrix_start + layer,
                    node.cache_offset + parameter_cursor - node.parameter.offset,
                ),
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

    /// The mixed backend never exposes quantized values to a recurrent kernel.
    fn recurrent_cache<'a>(
        &self,
        cache: Option<&'a QuantizedInferenceWeights>,
    ) -> Option<&'a QuantizedInferenceWeights> {
        if self.math_backend == InferenceMathBackend::I16AffineFma128 {
            None
        } else {
            cache
        }
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

/// Exact recurrent-source substitution for an explicit ownership transition.
pub(crate) struct HeterogeneousRecurrentReset<'a> {
    /// One flag per prepared work item.
    pub mask: &'a [bool],
    /// One graph-sized exact-zero recurrent block.
    pub zero_recurrent: &'a [f32],
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
    evaluate_heterogeneous_population_inner(plan, population, brains, work, None, buffers, scratch)
}

/// Evaluate a heterogeneous batch while selected work items begin from exact
/// zero recurrent state for one explicit ownership transition.
pub(crate) fn evaluate_heterogeneous_population_with_resets(
    plan: &GraphExecutionPlan,
    work: &[CalculationWorkUnit],
    population: &[PopulationGenome],
    brains: &[BrainRuntimeState],
    reset: HeterogeneousRecurrentReset<'_>,
    buffers: HeterogeneousInferenceBuffers<'_>,
    scratch: &mut CalculationScratchView<'_>,
) -> Result<(), InferenceError> {
    if reset.mask.len() != work.len() {
        return Err(InferenceError::BufferLength {
            buffer: "recurrent reset mask",
            actual: reset.mask.len(),
            expected: work.len(),
        });
    }
    require_length(
        "zero recurrent state",
        reset.zero_recurrent.len(),
        plan.total_state_size,
    )?;
    debug_assert!(reset
        .zero_recurrent
        .iter()
        .all(|value| value.to_bits() == 0));
    evaluate_heterogeneous_population_inner(
        plan,
        population,
        brains,
        work,
        Some((reset.mask, reset.zero_recurrent)),
        buffers,
        scratch,
    )
}

fn evaluate_heterogeneous_population_inner(
    plan: &GraphExecutionPlan,
    population: &[PopulationGenome],
    brains: &[BrainRuntimeState],
    work: &[CalculationWorkUnit],
    reset: Option<(&[bool], &[f32])>,
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
        let recurrent_before = match reset {
            Some((mask, zero_recurrent)) if mask[index] => zero_recurrent,
            _ => &brain.recurrent,
        };
        let input_offset = index * plan.input_size;
        let output_offset = index * plan.output_size;
        let recurrent_offset = index * plan.total_state_size;
        plan.evaluate_inner(
            weights,
            plan.caches
                .get(unit.brain_index())
                .and_then(|entry| entry.weights.as_ref()),
            recurrent_before,
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
        if plan.math_backend.uses_i16_cache() {
            let valid = plan.caches.get(unit.brain_index()).is_some_and(|c| {
                c.handle == brain.handle
                    && c.genome_id
                        == unit
                            .population_slot()
                            .map(|slot| population[slot as usize].lineage.genome_id)
                    && c.weights.is_some()
            });
            if !valid {
                return Err(InferenceError::InvalidCompiledGraph { detail: "missing or stale i16 inference binding; prepare caches at the ownership boundary".into() });
            }
        }
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

/// Reduce long reference rows with Float32 Neumaier compensation.
///
/// Products, correction and the returned value all remain Float32. Compensation
/// avoids avoidable first-delivery error in deep, wide recurrent graphs while
/// short rows retain their original ordered sum. Nonfinite partial sums retain
/// ordinary Float32 infinity/NaN propagation rather than poisoning saturation
/// with an invalid correction. No storage is allocated by these iterators.
#[inline]
fn compensated_f32_sum(terms: impl Iterator<Item = f32>) -> f32 {
    let mut sum = 0.0_f32;
    let mut correction = 0.0_f32;
    for term in terms {
        let next = sum + term;
        if next.is_finite() && sum.is_finite() && term.is_finite() {
            correction += if sum.abs() >= term.abs() {
                (sum - next) + term
            } else {
                (term - next) + sum
            };
        } else {
            correction = 0.0;
        }
        sum = next;
    }
    if sum.is_finite() {
        sum + correction
    } else {
        sum
    }
}

/// Apply one backend-selected dot product over equal, prevalidated slices.
#[inline]
fn dot_product(backend: InferenceMathBackend, weights: &[f32], input: &[f32]) -> f32 {
    debug_assert_eq!(weights.len(), input.len());
    match backend {
        InferenceMathBackend::Scalar => {
            if input.len() >= 32 {
                return compensated_f32_sum(
                    (0..input.len()).map(|index| weights[index] * input[index]),
                );
            }
            let mut sum = 0.0f32;
            for index in 0..input.len() {
                sum += weights[index] * input[index];
            }
            sum
        }
        InferenceMathBackend::Sse2 | InferenceMathBackend::I16AffineFma128 => {
            simd_kernels::dot_product_sse2(weights, input)
        }
        InferenceMathBackend::I16MatrixFma128 => unreachable!("i16 dot requires cache"),
        #[cfg(feature = "engine-test-hooks")]
        InferenceMathBackend::BenchmarkF32Fma128 => {
            simd_kernels::dot_product_f32_fma128(weights, input)
        }
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
            if left.len() >= 32 {
                return compensated_f32_sum(
                    (0..left.len()).map(|index| weights[index] * (left[index] * right[index])),
                );
            }
            let mut sum = 0.0f32;
            for index in 0..left.len() {
                sum += weights[index] * (left[index] * right[index]);
            }
            sum
        }
        InferenceMathBackend::Sse2 | InferenceMathBackend::I16AffineFma128 => {
            simd_kernels::dot_product_mul_sse2(weights, left, right)
        }
        InferenceMathBackend::I16MatrixFma128 => unreachable!("i16 dot requires cache"),
        #[cfg(feature = "engine-test-hooks")]
        InferenceMathBackend::BenchmarkF32Fma128 => {
            compensated_f32_sum((0..left.len()).map(|i| weights[i] * (left[i] * right[i])))
        }
    }
}

/// Apply one row-major affine transform followed by tanh.
fn affine_tanh(
    backend: InferenceMathBackend,
    weights: NodeWeights<'_>,
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
        let sum = weights.dot(backend, cursor..row_end, input, 0) + weights.bias(row_end, Some(0));
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
    weights: NodeWeights<'_>,
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
        let sum_z = weights.dot(backend, wz_row..wz_row + input_size, input, 0)
            + weights.dot(backend, uz_row..uz_row + hidden, before, 3);
        let sum_r = weights.dot(backend, wr_row..wr_row + input_size, input, 1)
            + weights.dot(backend, ur_row..ur_row + hidden, before, 4);
        z[j] = sigmoid(sum_z + weights.bias(bz + j, None));
        r[j] = sigmoid(sum_r + weights.bias(br + j, None));
    }
    for j in 0..hidden {
        let wh_row = wh + j * input_size;
        let uh_row = uh + j * hidden;
        let sum_h = weights.dot(backend, wh_row..wh_row + input_size, input, 2)
            + weights.dot_mul(backend, uh_row..uh_row + hidden, r, before, 5);
        let candidate = (sum_h + weights.bias(bh + j, None)).tanh();
        let next = (1.0 - z[j]) * before[j] + z[j] * candidate;
        after[j] = next;
        output[j] = next;
    }
    Ok(())
}

/// Evaluate one LSTM block using the established Wi/Wf/Wo/Wg/Ui/Uf/Uo/Ug order.
fn evaluate_lstm(
    backend: InferenceMathBackend,
    weights: NodeWeights<'_>,
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
        let sum_i = weights.dot(backend, wi_row..wi_row + input_size, input, 0)
            + weights.dot(backend, ui_row..ui_row + hidden, previous_h, 4);
        let sum_f = weights.dot(backend, wf_row..wf_row + input_size, input, 1)
            + weights.dot(backend, uf_row..uf_row + hidden, previous_h, 5);
        let sum_o = weights.dot(backend, wo_row..wo_row + input_size, input, 2)
            + weights.dot(backend, uo_row..uo_row + hidden, previous_h, 6);
        let sum_g = weights.dot(backend, wg_row..wg_row + input_size, input, 3)
            + weights.dot(backend, ug_row..ug_row + hidden, previous_h, 7);
        let input_gate = sigmoid(sum_i + weights.bias(bi + j, None));
        let forget_gate = sigmoid(sum_f + weights.bias(bf + j, None));
        let output_gate = sigmoid(sum_o + weights.bias(bo + j, None));
        let candidate = (sum_g + weights.bias(bg + j, None)).tanh();
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
    weights: NodeWeights<'_>,
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
        let sum_c = weights.dot(backend, wc_row..wc_row + input_size, input, 0)
            + weights.dot(backend, uc_row..uc_row + hidden, before, 2);
        let sum_r = weights.dot(backend, wr_row..wr_row + input_size, input, 1)
            + weights.dot(backend, ur_row..ur_row + hidden, before, 3);
        let candidate = (sum_c + weights.bias(bc + j, None)).tanh();
        let gate = sigmoid(sum_r + weights.bias(br + j, None));
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
        .ok_or_else(|| InferenceError::InvalidExecutionPlan {
            node_id: "<range>".to_owned(),
            detail: "immutable range is out of bounds".to_owned(),
        })
}

/// Borrow a mutable checked range.
fn slice_range_mut(values: &mut [f32], range: TensorRange) -> Result<&mut [f32], InferenceError> {
    let end = range.end()?;
    values
        .get_mut(range.offset..end)
        .ok_or_else(|| InferenceError::InvalidExecutionPlan {
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
    /// A persisted or configured backend label is unknown to this engine.
    UnknownMathBackend {
        /// Supplied versioned backend label.
        backend: String,
    },
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
            Self::UnknownMathBackend { backend } => {
                write!(formatter, "inference math backend {backend} is unknown")
            }
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
mod tests;
