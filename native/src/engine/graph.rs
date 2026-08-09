//! Owned graph specification, validation, and deterministic compilation.
//!
//! Canonical layout version 1 orders identifiers by their raw UTF-8 bytes,
//! without Unicode normalization. It is intentionally distinct from the
//! historical TypeScript `localeCompare` layout. Legacy layouts are represented
//! only by evidence types in this module; loading their weights requires a
//! future, explicit and independently validated remapping step. In particular,
//! historical gapped destination ports and nonzero ports on single-edge inputs
//! must be normalized by that migration; canonical graphs require contiguous
//! zero-based input ordinals.
//!
//! Limits bound compilation after an owned GraphSpec value exists. The bridge
//! must apply equivalent bounds while decoding so an oversized wire request is
//! rejected before constructing that owned value. Limits bound every admitted
//! structure, and major flat buffers use fallible reservations where practical.
//! `BTreeMap` insertion, nested-vector growth, formatting, and owned-value
//! clones can still abort on process-wide allocator exhaustion.

use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::ops::Deref;

/// Canonical graph-layout version implemented by this compiler.
pub const CANONICAL_GRAPH_LAYOUT_VERSION: u32 = 1;

/// Domain separator included in bytes covered by the layout SHA-256 digest.
pub const CANONICAL_GRAPH_LAYOUT_DOMAIN: &[u8] = b"slither-neuroevo/graph-layout/v1\0";

/// The authoritative graph output contract: turn and boost.
pub const AUTHORITATIVE_OUTPUT_SIZE: usize = 2;

/// Caller-selected limits for one authoritative graph compilation request.
///
/// This type intentionally has no `Default` implementation: the owning bridge
/// must supply reviewed limits derived from normalized configuration. A zero
/// edge, hidden-layer, Split-port, parameter, or recurrent-state limit may be
/// used to disable that feature. Other zero limits cannot admit even the
/// minimum request and are rejected as invalid policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphLimits {
    /// Maximum graph nodes.
    pub max_nodes: usize,
    /// Maximum directed edges.
    pub max_edges: usize,
    /// Maximum ordered graph output references.
    pub max_graph_outputs: usize,
    /// Maximum raw UTF-8 bytes in any node or reference identifier.
    pub max_identifier_bytes: usize,
    /// Maximum raw UTF-8 bytes across every node ID and every edge/output ID
    /// occurrence in the submitted request.
    pub max_total_referenced_identifier_bytes: usize,
    /// Maximum width of any declared or resolved tensor.
    pub max_tensor_width: usize,
    /// Maximum hidden-layer entries on one MLP node.
    pub max_mlp_hidden_layers: usize,
    /// Maximum output ports on one Split node.
    pub max_split_output_ports: usize,
    /// Maximum packed parameter floats for the complete graph.
    pub max_parameter_floats: usize,
    /// Maximum recurrent-state floats for one brain.
    pub max_recurrent_state_floats: usize,
    /// Maximum domain-separated canonical layout bytes.
    pub max_canonical_layout_bytes: usize,
    /// Maximum bytes in the collision-safe architecture-key string.
    pub max_architecture_key_bytes: usize,
}

/// A graph node definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNodeSpec {
    /// Stable node identifier. Empty identifiers are invalid.
    pub id: String,
    /// Operation and declared dimensions for this node.
    pub kind: GraphNodeKind,
}

/// Supported graph operations and their declared dimensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphNodeKind {
    /// The sole graph input.
    Input {
        /// Number of sensor values delivered to the graph.
        output_size: usize,
    },
    /// One affine layer.
    Dense {
        /// Declared input width.
        input_size: usize,
        /// Declared output width.
        output_size: usize,
    },
    /// A feed-forward stack of affine layers.
    Mlp {
        /// Declared input width.
        input_size: usize,
        /// Ordered hidden-layer widths.
        hidden_sizes: Vec<usize>,
        /// Declared output width.
        output_size: usize,
    },
    /// A gated recurrent unit.
    Gru {
        /// Declared input width.
        input_size: usize,
        /// Hidden/output width.
        hidden_size: usize,
    },
    /// A long short-term memory unit.
    Lstm {
        /// Declared input width.
        input_size: usize,
        /// Hidden/output width.
        hidden_size: usize,
    },
    /// A recurrent residual unit.
    Rru {
        /// Declared input width.
        input_size: usize,
        /// Hidden/output width.
        hidden_size: usize,
    },
    /// Concatenation of inputs in resolved input-ordinal order.
    Concat,
    /// Split one input into ordered output ports.
    Split {
        /// Width of every zero-based output port.
        output_sizes: Vec<usize>,
    },
}

/// One directed connection between graph ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    /// Source node identifier.
    pub from: String,
    /// Destination node identifier.
    pub to: String,
    /// Optional zero-based source output port; omission means port zero.
    pub from_port: Option<i64>,
    /// Optional zero-based destination input ordinal; omission requests
    /// canonical implicit ordering.
    pub to_port: Option<i64>,
}

/// One ordered graph output reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphOutputRef {
    /// Producing node identifier.
    pub node_id: String,
    /// Optional zero-based output port; omission means port zero.
    pub port: Option<i64>,
}

/// Complete owned graph definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphSpec {
    /// Nodes in arbitrary input-array order.
    pub nodes: Vec<GraphNodeSpec>,
    /// Edges in arbitrary input-array order.
    pub edges: Vec<GraphEdge>,
    /// Ordered output mapping. Output order is semantically significant.
    pub outputs: Vec<GraphOutputRef>,
    /// Declared aggregate output width; authoritative graphs require two.
    pub output_size: usize,
}

/// A source and port resolved for a compiled node input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledInputRef {
    /// Source node identifier.
    pub from_id: String,
    /// Zero-based source output port.
    pub from_port: usize,
    /// Zero-based destination input ordinal.
    pub input_ordinal: usize,
}

/// Operation tag retained in compiled metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledNodeType {
    /// Graph input.
    Input,
    /// Affine layer.
    Dense,
    /// Feed-forward affine stack.
    Mlp,
    /// Gated recurrent unit.
    Gru,
    /// Long short-term memory unit.
    Lstm,
    /// Recurrent residual unit.
    Rru,
    /// Concatenation.
    Concat,
    /// Split.
    Split,
}

/// Runtime-ready metadata for one graph node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledNode {
    /// Node identifier.
    pub id: String,
    /// Operation tag.
    pub node_type: CompiledNodeType,
    /// Resolved aggregate input width.
    pub input_size: usize,
    /// Resolved aggregate output width.
    pub output_size: usize,
    /// Ordered output-port widths.
    pub output_sizes: Vec<usize>,
    /// Ordered MLP hidden widths, if any.
    pub hidden_sizes: Vec<usize>,
    /// Hidden width for a recurrent node.
    pub hidden_size: Option<usize>,
    /// Offset into one genome's packed parameter vector.
    pub parameter_offset: usize,
    /// Number of packed parameters owned by this node.
    pub parameter_length: usize,
    /// Offset into one brain's recurrent-state vector, when recurrent.
    pub state_offset: Option<usize>,
    /// Number of recurrent-state floats owned by this node.
    pub state_length: usize,
    /// Inputs in their resolved semantic order.
    pub inputs: Vec<CompiledInputRef>,
}

/// Metadata for one recurrent node's isolated state block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrentNodeInfo {
    /// Index into [`CompiledGraph::nodes`].
    pub node_index: usize,
    /// Recurrent operation type.
    pub node_type: CompiledNodeType,
    /// Hidden width.
    pub hidden_size: usize,
    /// Offset into one brain's recurrent-state vector.
    pub state_offset: usize,
    /// State floats: hidden for GRU/RRU, hidden plus cell for LSTM.
    pub state_length: usize,
}

/// Canonical, runtime-ready graph metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledGraph {
    /// Version of the canonical layout rules.
    pub layout_version: u32,
    /// Collision-safe versioned architecture identity. This is an injective
    /// encoding, not the future compact SHA-256 layout digest.
    pub architecture_key: String,
    /// Domain-separated canonical bytes covered by `layout_digest_sha256`.
    pub canonical_layout_bytes: Vec<u8>,
    /// SHA-256 of `canonical_layout_bytes`, stored as exactly 32 bytes.
    pub layout_digest_sha256: [u8; 32],
    /// Nodes in deterministic topological and raw-UTF-8 tie-break order.
    pub nodes: Vec<CompiledNode>,
    /// Node identifiers in the same order as `nodes`.
    pub order: Vec<String>,
    /// Total packed parameter floats.
    pub total_parameters: usize,
    /// Total recurrent-state floats for one brain.
    pub total_state_size: usize,
    /// Resolved aggregate output width.
    pub output_size: usize,
    /// Ordered and validated graph output references.
    pub outputs: Vec<GraphOutputRef>,
    /// Recurrent node blocks in compiled-node order.
    pub recurrent_nodes: Vec<RecurrentNodeInfo>,
}

impl CompiledGraph {
    /// Return the canonical layout SHA-256 as 64 lowercase hexadecimal digits.
    pub fn layout_digest_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(64);
        for byte in self.layout_digest_sha256 {
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
        encoded
    }
}

/// One inseparable source graph and its deterministically derived runtime layout.
///
/// The source definition remains available for checkpoint-v3 construction while
/// execution uses only the compiled metadata.  The fields deliberately stay
/// private: callers cannot pair an arbitrary specification with an unrelated
/// parameter layout.
#[derive(Debug)]
pub struct GraphBundle {
    spec: GraphSpec,
    compiled: CompiledGraph,
}

impl GraphBundle {
    /// Compile an owned source graph and retain both the original definition and result.
    pub fn compile(spec: GraphSpec, limits: &GraphLimits) -> Result<Self, GraphError> {
        let compiled = compile_graph(&spec, limits)?;
        Ok(Self { spec, compiled })
    }

    /// Return the original owned source graph without exposing mutation.
    #[must_use]
    pub fn spec(&self) -> &GraphSpec {
        &self.spec
    }

    /// Return the canonical runtime layout derived from [`Self::spec`].
    #[must_use]
    pub fn compiled(&self) -> &CompiledGraph {
        &self.compiled
    }
}

impl Deref for GraphBundle {
    type Target = CompiledGraph;

    /// Borrow the immutable compiled layout for existing calculation helpers.
    fn deref(&self) -> &Self::Target {
        self.compiled()
    }
}

/// Historical node parameter block captured from a supported legacy compiler.
///
/// This is evidence for an explicit migration. It is not accepted by
/// [`compile_graph`] and does not authorize direct use of legacy packed weights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyParameterBlockEvidence {
    /// Historical node identifier.
    pub node_id: String,
    /// Historical packed-float offset.
    pub parameter_offset: usize,
    /// Historical packed-float length.
    pub parameter_length: usize,
}

/// Historical multi-input feature order captured from a supported legacy compiler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyIncomingOrderEvidence {
    /// Destination node identifier.
    pub node_id: String,
    /// Historical ordered `(source node, source port)` entries.
    pub inputs: Vec<(String, usize)>,
}

/// Evidence-only description of a legacy graph layout.
///
/// A migration layer must verify this manifest, construct an explicit block
/// and feature-order mapping, and then remap weights into canonical layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyLayoutEvidence {
    /// Exact legacy compiler/environment identity used to obtain this evidence.
    pub compiler_identity: String,
    /// Exact stored legacy architecture key.
    pub architecture_key: String,
    /// Historical node block order and ranges.
    pub parameter_blocks: Vec<LegacyParameterBlockEvidence>,
    /// Historical input order for every affected multi-input node.
    pub incoming_orders: Vec<LegacyIncomingOrderEvidence>,
}

/// Deterministic graph validation or arithmetic failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphError {
    message: String,
}

impl GraphError {
    /// Construct an error with stable human-readable context.
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Return the stable error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GraphError {}

/// Compare identifiers lexicographically by their raw UTF-8 bytes.
fn compare_raw_utf8(left: &str, right: &str) -> Ordering {
    left.as_bytes().cmp(right.as_bytes())
}

/// Add two counts without allowing platform-sized arithmetic to wrap.
fn checked_add(left: usize, right: usize, context: &str) -> Result<usize, GraphError> {
    left.checked_add(right)
        .ok_or_else(|| GraphError::new(format!("Graph: {context} overflows usize.")))
}

/// Multiply two counts without allowing platform-sized arithmetic to wrap.
fn checked_mul(left: usize, right: usize, context: &str) -> Result<usize, GraphError> {
    left.checked_mul(right)
        .ok_or_else(|| GraphError::new(format!("Graph: {context} overflows usize.")))
}

/// Reserve one compiler-owned vector without panicking on capacity failure.
fn reserved_vec<T>(capacity: usize, context: &str) -> Result<Vec<T>, GraphError> {
    let mut values = Vec::new();
    values.try_reserve_exact(capacity).map_err(|_| {
        GraphError::new(format!(
            "Graph: unable to reserve {capacity} entries for {context}."
        ))
    })?;
    Ok(values)
}

/// Require a count to fit one caller-selected limit.
fn within_limit(
    value: usize,
    limit: usize,
    context: &str,
    limit_name: &str,
) -> Result<(), GraphError> {
    if value > limit {
        Err(GraphError::new(format!(
            "Graph: {context} {value} exceeds {limit_name} limit {limit}."
        )))
    } else {
        Ok(())
    }
}

/// Require one positive dimension.
fn positive(value: usize, context: &str) -> Result<usize, GraphError> {
    if value == 0 {
        Err(GraphError::new(format!(
            "Graph: {context} must be positive."
        )))
    } else {
        Ok(value)
    }
}

/// Validate that mandatory policy limits can admit a graph request.
fn validate_limits(limits: &GraphLimits) -> Result<(), GraphError> {
    let mandatory = [
        (limits.max_nodes, "max_nodes"),
        (limits.max_graph_outputs, "max_graph_outputs"),
        (limits.max_identifier_bytes, "max_identifier_bytes"),
        (
            limits.max_total_referenced_identifier_bytes,
            "max_total_referenced_identifier_bytes",
        ),
        (limits.max_tensor_width, "max_tensor_width"),
        (
            limits.max_canonical_layout_bytes,
            "max_canonical_layout_bytes",
        ),
        (
            limits.max_architecture_key_bytes,
            "max_architecture_key_bytes",
        ),
    ];
    for (value, name) in mandatory {
        if value == 0 {
            return Err(GraphError::new(format!(
                "Graph: {name} limit must be positive."
            )));
        }
    }
    Ok(())
}

/// Validate and account for one raw UTF-8 identifier occurrence.
fn account_identifier(
    id: &str,
    context: &str,
    limits: &GraphLimits,
    total_bytes: &mut usize,
) -> Result<(), GraphError> {
    if id.is_empty() {
        return Err(GraphError::new(format!(
            "Graph: {context} identifier must be nonempty."
        )));
    }
    within_limit(
        id.len(),
        limits.max_identifier_bytes,
        &format!("{context} identifier byte length"),
        "max_identifier_bytes",
    )?;
    *total_bytes = checked_add(*total_bytes, id.len(), "total referenced identifier bytes")?;
    within_limit(
        *total_bytes,
        limits.max_total_referenced_identifier_bytes,
        "total referenced identifier bytes",
        "max_total_referenced_identifier_bytes",
    )
}

/// Reject an oversized request shape before compiler-owned graph allocations.
fn validate_request_shape(spec: &GraphSpec, limits: &GraphLimits) -> Result<(), GraphError> {
    validate_limits(limits)?;
    within_limit(
        spec.nodes.len(),
        limits.max_nodes,
        "node count",
        "max_nodes",
    )?;
    within_limit(
        spec.edges.len(),
        limits.max_edges,
        "edge count",
        "max_edges",
    )?;
    within_limit(
        spec.outputs.len(),
        limits.max_graph_outputs,
        "graph output-reference count",
        "max_graph_outputs",
    )?;
    within_limit(
        spec.output_size,
        limits.max_tensor_width,
        "declared graph output width",
        "max_tensor_width",
    )?;

    let mut total_identifier_bytes = 0usize;
    for node in &spec.nodes {
        account_identifier(&node.id, "node", limits, &mut total_identifier_bytes)?;
        validate_declared_dimensions(node, limits)?;
    }
    for edge in &spec.edges {
        account_identifier(
            &edge.from,
            "edge source",
            limits,
            &mut total_identifier_bytes,
        )?;
        account_identifier(
            &edge.to,
            "edge destination",
            limits,
            &mut total_identifier_bytes,
        )?;
        resolve_port(
            edge.from_port,
            &format!("from_port on edge {} -> {}", edge.from, edge.to),
        )?;
        if edge.to_port.is_some() {
            resolve_port(
                edge.to_port,
                &format!("to_port on edge {} -> {}", edge.from, edge.to),
            )?;
        }
    }
    for output in &spec.outputs {
        account_identifier(
            &output.node_id,
            "graph output",
            limits,
            &mut total_identifier_bytes,
        )?;
        resolve_port(output.port, &format!("output {}", output.node_id))?;
    }
    Ok(())
}

/// Convert and validate an optional zero-based port.
fn resolve_port(port: Option<i64>, context: &str) -> Result<usize, GraphError> {
    let value = port.unwrap_or(0);
    usize::try_from(value)
        .map_err(|_| GraphError::new(format!("Graph: {context} port {value} is negative.")))
}

/// Count parameters for one affine layer.
fn dense_parameter_count(input: usize, output: usize, id: &str) -> Result<usize, GraphError> {
    let weights = checked_mul(input, output, &format!("Dense {id} parameter count"))?;
    checked_add(weights, output, &format!("Dense {id} parameter count"))
}

/// Count parameters for an ordered MLP layer sequence.
fn mlp_parameter_count(layer_sizes: &[usize], id: &str) -> Result<usize, GraphError> {
    let mut total = 0usize;
    for pair in layer_sizes.windows(2) {
        let layer = dense_parameter_count(pair[0], pair[1], id)?;
        total = checked_add(total, layer, &format!("MLP {id} parameter count"))?;
    }
    Ok(total)
}

/// Count parameters for a recurrent node with the supplied gate factor.
fn recurrent_parameter_count(
    input: usize,
    hidden: usize,
    gates: usize,
    kind: &str,
    id: &str,
) -> Result<usize, GraphError> {
    let input_hidden = checked_add(input, hidden, &format!("{kind} {id} parameter count"))?;
    let row = checked_add(input_hidden, 1, &format!("{kind} {id} parameter count"))?;
    let hidden_rows = checked_mul(hidden, row, &format!("{kind} {id} parameter count"))?;
    checked_mul(gates, hidden_rows, &format!("{kind} {id} parameter count"))
}

/// Return the compiled operation tag for a node kind.
fn compiled_node_type(kind: &GraphNodeKind) -> CompiledNodeType {
    match kind {
        GraphNodeKind::Input { .. } => CompiledNodeType::Input,
        GraphNodeKind::Dense { .. } => CompiledNodeType::Dense,
        GraphNodeKind::Mlp { .. } => CompiledNodeType::Mlp,
        GraphNodeKind::Gru { .. } => CompiledNodeType::Gru,
        GraphNodeKind::Lstm { .. } => CompiledNodeType::Lstm,
        GraphNodeKind::Rru { .. } => CompiledNodeType::Rru,
        GraphNodeKind::Concat => CompiledNodeType::Concat,
        GraphNodeKind::Split { .. } => CompiledNodeType::Split,
    }
}

/// Validate all dimensions declared directly on a node.
fn validate_declared_dimensions(
    node: &GraphNodeSpec,
    limits: &GraphLimits,
) -> Result<(), GraphError> {
    let id = &node.id;
    match &node.kind {
        GraphNodeKind::Input { output_size } => {
            positive(*output_size, &format!("Input {id} output size"))?;
            within_limit(
                *output_size,
                limits.max_tensor_width,
                &format!("Input {id} output width"),
                "max_tensor_width",
            )?;
        }
        GraphNodeKind::Dense {
            input_size,
            output_size,
        }
        | GraphNodeKind::Mlp {
            input_size,
            output_size,
            ..
        } => {
            positive(
                *input_size,
                &format!("{} {id} input size", node_kind_name(&node.kind)),
            )?;
            positive(
                *output_size,
                &format!("{} {id} output size", node_kind_name(&node.kind)),
            )?;
            within_limit(
                *input_size,
                limits.max_tensor_width,
                &format!("{} {id} input width", node_kind_name(&node.kind)),
                "max_tensor_width",
            )?;
            within_limit(
                *output_size,
                limits.max_tensor_width,
                &format!("{} {id} output width", node_kind_name(&node.kind)),
                "max_tensor_width",
            )?;
            if let GraphNodeKind::Mlp { hidden_sizes, .. } = &node.kind {
                within_limit(
                    hidden_sizes.len(),
                    limits.max_mlp_hidden_layers,
                    &format!("MLP {id} hidden-layer count"),
                    "max_mlp_hidden_layers",
                )?;
                for (index, size) in hidden_sizes.iter().enumerate() {
                    positive(*size, &format!("MLP {id} hidden size {index}"))?;
                    within_limit(
                        *size,
                        limits.max_tensor_width,
                        &format!("MLP {id} hidden width {index}"),
                        "max_tensor_width",
                    )?;
                }
            }
        }
        GraphNodeKind::Gru {
            input_size,
            hidden_size,
        }
        | GraphNodeKind::Lstm {
            input_size,
            hidden_size,
        }
        | GraphNodeKind::Rru {
            input_size,
            hidden_size,
        } => {
            positive(
                *input_size,
                &format!("{} {id} input size", node_kind_name(&node.kind)),
            )?;
            positive(
                *hidden_size,
                &format!("{} {id} hidden size", node_kind_name(&node.kind)),
            )?;
            within_limit(
                *input_size,
                limits.max_tensor_width,
                &format!("{} {id} input width", node_kind_name(&node.kind)),
                "max_tensor_width",
            )?;
            within_limit(
                *hidden_size,
                limits.max_tensor_width,
                &format!("{} {id} hidden width", node_kind_name(&node.kind)),
                "max_tensor_width",
            )?;
        }
        GraphNodeKind::Concat => {}
        GraphNodeKind::Split { output_sizes } => {
            if output_sizes.is_empty() {
                return Err(GraphError::new(format!(
                    "Graph: Split {id} must have at least one output."
                )));
            }
            within_limit(
                output_sizes.len(),
                limits.max_split_output_ports,
                &format!("Split {id} output-port count"),
                "max_split_output_ports",
            )?;
            for (index, size) in output_sizes.iter().enumerate() {
                positive(*size, &format!("Split {id} output size {index}"))?;
                within_limit(
                    *size,
                    limits.max_tensor_width,
                    &format!("Split {id} output width {index}"),
                    "max_tensor_width",
                )?;
            }
        }
    }
    Ok(())
}

/// Return a stable operation name for diagnostics.
fn node_kind_name(kind: &GraphNodeKind) -> &'static str {
    match kind {
        GraphNodeKind::Input { .. } => "Input",
        GraphNodeKind::Dense { .. } => "Dense",
        GraphNodeKind::Mlp { .. } => "MLP",
        GraphNodeKind::Gru { .. } => "GRU",
        GraphNodeKind::Lstm { .. } => "LSTM",
        GraphNodeKind::Rru { .. } => "RRU",
        GraphNodeKind::Concat => "Concat",
        GraphNodeKind::Split { .. } => "Split",
    }
}

/// Resolve incoming edges into deterministic semantic input order.
fn order_incoming_edges<'a>(
    edges: &[&'a GraphEdge],
    node_id: &str,
) -> Result<Vec<(&'a GraphEdge, usize)>, GraphError> {
    if edges.is_empty() {
        return Ok(Vec::new());
    }
    let explicit_count = edges.iter().filter(|edge| edge.to_port.is_some()).count();
    if explicit_count != 0 && explicit_count != edges.len() {
        return Err(GraphError::new(format!(
            "Graph: mixed explicit and implicit to_port usage for node {node_id}."
        )));
    }

    let mut ordered = reserved_vec(edges.len(), &format!("{node_id} ordered inputs"))?;
    ordered.extend_from_slice(edges);
    if explicit_count == edges.len() {
        ordered.sort_by(|left, right| {
            let left_port = left.to_port.unwrap_or(0);
            let right_port = right.to_port.unwrap_or(0);
            left_port.cmp(&right_port).then_with(|| {
                compare_raw_utf8(&left.from, &right.from).then_with(|| {
                    left.from_port
                        .unwrap_or(0)
                        .cmp(&right.from_port.unwrap_or(0))
                })
            })
        });
        let mut result = reserved_vec(ordered.len(), &format!("{node_id} resolved inputs"))?;
        for (expected, edge) in ordered.into_iter().enumerate() {
            let actual = resolve_port(edge.to_port, &format!("to_port on {node_id}"))?;
            if actual != expected {
                return Err(GraphError::new(format!(
                    "Graph: explicit to_port values on {node_id} must be contiguous from zero; expected {expected}, got {actual}."
                )));
            }
            result.push((edge, expected));
        }
        Ok(result)
    } else {
        ordered.sort_by(|left, right| {
            compare_raw_utf8(&left.from, &right.from).then_with(|| {
                left.from_port
                    .unwrap_or(0)
                    .cmp(&right.from_port.unwrap_or(0))
            })
        });
        let mut result = reserved_vec(ordered.len(), &format!("{node_id} resolved inputs"))?;
        result.extend(
            ordered
                .into_iter()
                .enumerate()
                .map(|(index, edge)| (edge, index)),
        );
        Ok(result)
    }
}

/// Encode a `usize` as a canonical little-endian `u64`.
fn append_count(bytes: &mut Vec<u8>, value: usize, context: &str) -> Result<(), GraphError> {
    let value = u64::try_from(value)
        .map_err(|_| GraphError::new(format!("Graph: {context} does not fit canonical u64.")))?;
    bytes.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Encode a string as a length-prefixed raw UTF-8 byte sequence.
fn append_string(bytes: &mut Vec<u8>, value: &str, context: &str) -> Result<(), GraphError> {
    append_count(bytes, value.len(), context)?;
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

/// Add the encoded length of one length-prefixed byte sequence.
fn add_encoded_string_length(
    length: &mut usize,
    value: &str,
    context: &str,
) -> Result<(), GraphError> {
    *length = checked_add(*length, 8, context)?;
    *length = checked_add(*length, value.len(), context)?;
    Ok(())
}

/// Calculate the exact canonical byte length without allocating the buffer.
fn canonical_layout_length(
    nodes: &[CompiledNode],
    outputs: &[GraphOutputRef],
) -> Result<usize, GraphError> {
    let mut length = checked_add(
        CANONICAL_GRAPH_LAYOUT_DOMAIN.len(),
        4,
        "canonical layout byte length",
    )?;
    length = checked_add(length, 8, "canonical layout byte length")?;
    for node in nodes {
        add_encoded_string_length(&mut length, &node.id, "canonical layout byte length")?;
        // type tag, input/output widths, output-port count, hidden-layer count
        length = checked_add(length, 1 + 8 + 8 + 8 + 8, "canonical layout byte length")?;
        length = checked_add(
            length,
            checked_mul(
                node.output_sizes.len(),
                8,
                "canonical output-port byte length",
            )?,
            "canonical layout byte length",
        )?;
        length = checked_add(
            length,
            checked_mul(
                node.hidden_sizes.len(),
                8,
                "canonical hidden-layer byte length",
            )?,
            "canonical layout byte length",
        )?;
        // hidden marker/value, parameter offset/length, state marker/value,
        // state length, and input count.
        length = checked_add(length, 1, "canonical layout byte length")?;
        if node.hidden_size.is_some() {
            length = checked_add(length, 8, "canonical layout byte length")?;
        }
        length = checked_add(length, 8 + 8 + 1, "canonical layout byte length")?;
        if node.state_offset.is_some() {
            length = checked_add(length, 8, "canonical layout byte length")?;
        }
        length = checked_add(length, 8 + 8, "canonical layout byte length")?;
        for input in &node.inputs {
            add_encoded_string_length(&mut length, &input.from_id, "canonical layout byte length")?;
            length = checked_add(length, 8 + 8, "canonical layout byte length")?;
        }
    }
    length = checked_add(length, 8, "canonical layout byte length")?;
    for output in outputs {
        add_encoded_string_length(&mut length, &output.node_id, "canonical layout byte length")?;
        length = checked_add(length, 8, "canonical layout byte length")?;
    }
    checked_add(length, 8 + 8 + 8, "canonical layout byte length")
}

/// Encode the compiled layout injectively for architecture identity and hashing.
fn encode_canonical_layout(
    nodes: &[CompiledNode],
    outputs: &[GraphOutputRef],
    total_parameters: usize,
    total_state_size: usize,
    output_size: usize,
    limits: &GraphLimits,
) -> Result<Vec<u8>, GraphError> {
    let exact_length = canonical_layout_length(nodes, outputs)?;
    within_limit(
        exact_length,
        limits.max_canonical_layout_bytes,
        "canonical layout byte length",
        "max_canonical_layout_bytes",
    )?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(exact_length).map_err(|_| {
        GraphError::new(format!(
            "Graph: unable to reserve {exact_length} canonical layout bytes."
        ))
    })?;
    bytes.extend_from_slice(CANONICAL_GRAPH_LAYOUT_DOMAIN);
    bytes.extend_from_slice(&CANONICAL_GRAPH_LAYOUT_VERSION.to_le_bytes());
    append_count(&mut bytes, nodes.len(), "node count")?;
    for node in nodes {
        append_string(&mut bytes, &node.id, "node identifier length")?;
        bytes.push(match node.node_type {
            CompiledNodeType::Input => 0,
            CompiledNodeType::Dense => 1,
            CompiledNodeType::Mlp => 2,
            CompiledNodeType::Gru => 3,
            CompiledNodeType::Lstm => 4,
            CompiledNodeType::Rru => 5,
            CompiledNodeType::Concat => 6,
            CompiledNodeType::Split => 7,
        });
        append_count(&mut bytes, node.input_size, "node input size")?;
        append_count(&mut bytes, node.output_size, "node output size")?;
        append_count(
            &mut bytes,
            node.output_sizes.len(),
            "node output-port count",
        )?;
        for size in &node.output_sizes {
            append_count(&mut bytes, *size, "node output-port size")?;
        }
        append_count(
            &mut bytes,
            node.hidden_sizes.len(),
            "MLP hidden-layer count",
        )?;
        for size in &node.hidden_sizes {
            append_count(&mut bytes, *size, "MLP hidden-layer size")?;
        }
        match node.hidden_size {
            Some(size) => {
                bytes.push(1);
                append_count(&mut bytes, size, "recurrent hidden size")?;
            }
            None => bytes.push(0),
        }
        append_count(&mut bytes, node.parameter_offset, "parameter offset")?;
        append_count(&mut bytes, node.parameter_length, "parameter length")?;
        match node.state_offset {
            Some(offset) => {
                bytes.push(1);
                append_count(&mut bytes, offset, "state offset")?;
            }
            None => bytes.push(0),
        }
        append_count(&mut bytes, node.state_length, "state length")?;
        append_count(&mut bytes, node.inputs.len(), "input count")?;
        for input in &node.inputs {
            append_string(&mut bytes, &input.from_id, "input source identifier length")?;
            append_count(&mut bytes, input.from_port, "input source port")?;
            append_count(&mut bytes, input.input_ordinal, "input ordinal")?;
        }
    }
    append_count(&mut bytes, outputs.len(), "graph output count")?;
    for output in outputs {
        append_string(&mut bytes, &output.node_id, "output identifier length")?;
        append_count(
            &mut bytes,
            resolve_port(output.port, &format!("output {}", output.node_id))?,
            "output port",
        )?;
    }
    append_count(&mut bytes, total_parameters, "total parameter count")?;
    append_count(&mut bytes, total_state_size, "total state size")?;
    append_count(&mut bytes, output_size, "graph output size")?;
    debug_assert_eq!(bytes.len(), exact_length);
    Ok(bytes)
}

/// Hex-encode canonical bytes as an injective, collision-safe architecture key.
fn architecture_key(bytes: &[u8], limits: &GraphLimits) -> Result<String, GraphError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    const PREFIX: &str = "slither-graph-layout-v1:hex:";
    let encoded_length = bytes
        .len()
        .checked_mul(2)
        .ok_or_else(|| GraphError::new("Graph: architecture key length overflows usize."))?;
    let exact_length = checked_add(PREFIX.len(), encoded_length, "architecture key byte length")?;
    within_limit(
        exact_length,
        limits.max_architecture_key_bytes,
        "architecture key byte length",
        "max_architecture_key_bytes",
    )?;
    let mut key = String::new();
    key.try_reserve_exact(exact_length).map_err(|_| {
        GraphError::new(format!(
            "Graph: unable to reserve {exact_length} architecture key bytes."
        ))
    })?;
    key.push_str(PREFIX);
    for byte in bytes {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0x0f) as usize] as char);
    }
    debug_assert_eq!(key.len(), exact_length);
    Ok(key)
}

/// Compile and independently validate one authoritative graph.
///
/// Input node and edge array order do not affect canonical layout. Graph output
/// array order, Split output order, MLP hidden order, and resolved Concat input
/// order are semantic and therefore retained. `limits` must come from the
/// caller's reviewed normalized configuration; this API has no implicit limits.
pub fn compile_graph(spec: &GraphSpec, limits: &GraphLimits) -> Result<CompiledGraph, GraphError> {
    validate_request_shape(spec, limits)?;
    if spec.nodes.is_empty() {
        return Err(GraphError::new("Graph: no nodes defined."));
    }
    if spec.output_size != AUTHORITATIVE_OUTPUT_SIZE {
        return Err(GraphError::new(format!(
            "Graph: authoritative output size must be {AUTHORITATIVE_OUTPUT_SIZE}, got {}.",
            spec.output_size
        )));
    }

    let mut node_by_id: BTreeMap<&str, &GraphNodeSpec> = BTreeMap::new();
    let mut input_count = 0usize;
    for node in &spec.nodes {
        if node_by_id.insert(&node.id, node).is_some() {
            return Err(GraphError::new(format!(
                "Graph: duplicate node id {}.",
                node.id
            )));
        }
        if matches!(node.kind, GraphNodeKind::Input { .. }) {
            input_count = checked_add(input_count, 1, "input-node count")?;
        }
    }
    if input_count != 1 {
        return Err(GraphError::new(format!(
            "Graph: exactly one Input node is required, got {input_count}."
        )));
    }

    let mut incoming: BTreeMap<&str, Vec<&GraphEdge>> = BTreeMap::new();
    let mut outgoing: BTreeMap<&str, Vec<&GraphEdge>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &spec.nodes {
        incoming.insert(&node.id, Vec::new());
        outgoing.insert(&node.id, Vec::new());
        indegree.insert(&node.id, 0);
    }
    for edge in &spec.edges {
        if !node_by_id.contains_key(edge.from.as_str()) {
            return Err(GraphError::new(format!(
                "Graph: edge from unknown node {}.",
                edge.from
            )));
        }
        if !node_by_id.contains_key(edge.to.as_str()) {
            return Err(GraphError::new(format!(
                "Graph: edge to unknown node {}.",
                edge.to
            )));
        }
        resolve_port(
            edge.from_port,
            &format!("from_port on edge {} -> {}", edge.from, edge.to),
        )?;
        if edge.to_port.is_some() {
            resolve_port(
                edge.to_port,
                &format!("to_port on edge {} -> {}", edge.from, edge.to),
            )?;
        }
        incoming
            .get_mut(edge.to.as_str())
            .expect("known destination")
            .push(edge);
        outgoing
            .get_mut(edge.from.as_str())
            .expect("known source")
            .push(edge);
        let degree = indegree
            .get_mut(edge.to.as_str())
            .expect("known destination");
        *degree = checked_add(*degree, 1, "node indegree")?;
    }

    let mut ready = reserved_vec(spec.nodes.len(), "topological ready queue")?;
    ready.extend(
        indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id)),
    );
    ready.sort_by(|left, right| compare_raw_utf8(left, right));
    let mut order = reserved_vec(spec.nodes.len(), "topological order")?;
    while !ready.is_empty() {
        let id = ready.remove(0);
        order.push(id);
        for edge in outgoing.get(id).expect("known source") {
            let degree = indegree
                .get_mut(edge.to.as_str())
                .expect("known destination");
            *degree = degree
                .checked_sub(1)
                .expect("indegree matches outgoing edges");
            if *degree == 0 {
                ready.push(&edge.to);
                ready.sort_by(|left, right| compare_raw_utf8(left, right));
            }
        }
    }
    if order.len() != spec.nodes.len() {
        return Err(GraphError::new("Graph: cycle detected."));
    }

    let mut resolved_outputs: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut compiled_nodes = reserved_vec(order.len(), "compiled nodes")?;
    let mut total_parameters = 0usize;
    let mut total_state_size = 0usize;
    let mut recurrent_nodes = reserved_vec(order.len(), "recurrent node metadata")?;

    for id in order {
        let node = node_by_id.get(id).expect("topology contains known node");
        let incoming_edges = incoming.get(id).expect("known destination");
        let ordered_incoming = order_incoming_edges(incoming_edges, id)?;
        let mut inputs = reserved_vec(ordered_incoming.len(), &format!("{id} compiled inputs"))?;
        let mut input_sizes = reserved_vec(ordered_incoming.len(), &format!("{id} input widths"))?;
        for (edge, input_ordinal) in ordered_incoming {
            let from_port = resolve_port(
                edge.from_port,
                &format!("from_port on edge {} -> {id}", edge.from),
            )?;
            let sizes = resolved_outputs.get(edge.from.as_str()).ok_or_else(|| {
                GraphError::new(format!(
                    "Graph: source {} is not resolved before {id}.",
                    edge.from
                ))
            })?;
            let size = sizes.get(from_port).copied().ok_or_else(|| {
                GraphError::new(format!(
                    "Graph: invalid from_port {from_port} on edge {} -> {id}; source has {} ports.",
                    edge.from,
                    sizes.len()
                ))
            })?;
            inputs.push(CompiledInputRef {
                from_id: edge.from.clone(),
                from_port,
                input_ordinal,
            });
            input_sizes.push(size);
        }

        let mut hidden_sizes = Vec::new();
        let mut hidden_size = None;
        let (input_size, output_sizes, parameter_length, state_length) = match &node.kind {
            GraphNodeKind::Input { output_size } => {
                if !inputs.is_empty() {
                    return Err(GraphError::new(format!(
                        "Graph: Input node {id} has incoming edges."
                    )));
                }
                (*output_size, vec![*output_size], 0, 0)
            }
            GraphNodeKind::Concat => {
                if inputs.is_empty() {
                    return Err(GraphError::new(format!(
                        "Graph: Concat node {id} has no inputs."
                    )));
                }
                let mut size = 0usize;
                for input in input_sizes {
                    size = checked_add(size, input, &format!("Concat {id} input size"))?;
                }
                within_limit(
                    size,
                    limits.max_tensor_width,
                    &format!("Concat {id} resolved width"),
                    "max_tensor_width",
                )?;
                (size, vec![size], 0, 0)
            }
            GraphNodeKind::Split { output_sizes } => {
                if inputs.len() != 1 {
                    return Err(GraphError::new(format!(
                        "Graph: Split node {id} must have exactly one input."
                    )));
                }
                let input_size = input_sizes[0];
                let mut split_total = 0usize;
                for size in output_sizes {
                    split_total =
                        checked_add(split_total, *size, &format!("Split {id} output size"))?;
                }
                if split_total != input_size {
                    return Err(GraphError::new(format!(
                        "Graph: Split {id} output sizes sum to {split_total}, not input size {input_size}."
                    )));
                }
                (input_size, output_sizes.clone(), 0, 0)
            }
            GraphNodeKind::Dense {
                input_size,
                output_size,
            } => {
                require_one_matching_input(id, "Dense", &inputs, &input_sizes, *input_size)?;
                (
                    *input_size,
                    vec![*output_size],
                    dense_parameter_count(*input_size, *output_size, id)?,
                    0,
                )
            }
            GraphNodeKind::Mlp {
                input_size,
                hidden_sizes: declared_hidden,
                output_size,
            } => {
                require_one_matching_input(id, "MLP", &inputs, &input_sizes, *input_size)?;
                hidden_sizes = declared_hidden.clone();
                let layer_count = checked_add(
                    declared_hidden.len(),
                    2,
                    &format!("MLP {id} layer-count allocation"),
                )?;
                let mut layers = reserved_vec(layer_count, &format!("MLP {id} layer sizes"))?;
                layers.push(*input_size);
                layers.extend_from_slice(declared_hidden);
                layers.push(*output_size);
                (
                    *input_size,
                    vec![*output_size],
                    mlp_parameter_count(&layers, id)?,
                    0,
                )
            }
            GraphNodeKind::Gru {
                input_size,
                hidden_size: declared_hidden,
            } => {
                require_one_matching_input(id, "GRU", &inputs, &input_sizes, *input_size)?;
                hidden_size = Some(*declared_hidden);
                (
                    *input_size,
                    vec![*declared_hidden],
                    recurrent_parameter_count(*input_size, *declared_hidden, 3, "GRU", id)?,
                    *declared_hidden,
                )
            }
            GraphNodeKind::Lstm {
                input_size,
                hidden_size: declared_hidden,
            } => {
                require_one_matching_input(id, "LSTM", &inputs, &input_sizes, *input_size)?;
                hidden_size = Some(*declared_hidden);
                let state = checked_mul(*declared_hidden, 2, &format!("LSTM {id} state size"))?;
                (
                    *input_size,
                    vec![*declared_hidden],
                    recurrent_parameter_count(*input_size, *declared_hidden, 4, "LSTM", id)?,
                    state,
                )
            }
            GraphNodeKind::Rru {
                input_size,
                hidden_size: declared_hidden,
            } => {
                require_one_matching_input(id, "RRU", &inputs, &input_sizes, *input_size)?;
                hidden_size = Some(*declared_hidden);
                (
                    *input_size,
                    vec![*declared_hidden],
                    recurrent_parameter_count(*input_size, *declared_hidden, 2, "RRU", id)?,
                    *declared_hidden,
                )
            }
        };
        let output_size = output_sizes.iter().try_fold(0usize, |total, size| {
            checked_add(total, *size, &format!("node {id} output size"))
        })?;
        within_limit(
            output_size,
            limits.max_tensor_width,
            &format!("node {id} resolved output width"),
            "max_tensor_width",
        )?;
        let parameter_offset = total_parameters;
        let next_total_parameters = checked_add(
            total_parameters,
            parameter_length,
            "total graph parameter count",
        )?;
        within_limit(
            next_total_parameters,
            limits.max_parameter_floats,
            "total graph parameter floats",
            "max_parameter_floats",
        )?;
        total_parameters = next_total_parameters;
        let state_offset = (state_length != 0).then_some(total_state_size);
        if state_length != 0 {
            let next_total_state =
                checked_add(total_state_size, state_length, "total recurrent state size")?;
            within_limit(
                next_total_state,
                limits.max_recurrent_state_floats,
                "total recurrent-state floats",
                "max_recurrent_state_floats",
            )?;
            total_state_size = next_total_state;
        }
        let compiled = CompiledNode {
            id: id.to_owned(),
            node_type: compiled_node_type(&node.kind),
            input_size,
            output_size,
            output_sizes: output_sizes.clone(),
            hidden_sizes,
            hidden_size,
            parameter_offset,
            parameter_length,
            state_offset,
            state_length,
            inputs,
        };
        let node_index = compiled_nodes.len();
        if let (Some(hidden_size), Some(state_offset)) =
            (compiled.hidden_size, compiled.state_offset)
        {
            recurrent_nodes.push(RecurrentNodeInfo {
                node_index,
                node_type: compiled.node_type,
                hidden_size,
                state_offset,
                state_length,
            });
        }
        resolved_outputs.insert(id, output_sizes);
        compiled_nodes.push(compiled);
    }

    let mut total_output = 0usize;
    let mut normalized_outputs = reserved_vec(spec.outputs.len(), "normalized graph outputs")?;
    for output in &spec.outputs {
        let sizes = resolved_outputs
            .get(output.node_id.as_str())
            .ok_or_else(|| {
                GraphError::new(format!(
                    "Graph: output references unknown node {}.",
                    output.node_id
                ))
            })?;
        let port = resolve_port(output.port, &format!("output {}", output.node_id))?;
        let size = sizes.get(port).copied().ok_or_else(|| {
            GraphError::new(format!(
                "Graph: output port {port} is out of bounds on {}.",
                output.node_id
            ))
        })?;
        total_output = checked_add(total_output, size, "total graph output size")?;
        normalized_outputs.push(GraphOutputRef {
            node_id: output.node_id.clone(),
            port: Some(
                i64::try_from(port).map_err(|_| {
                    GraphError::new("Graph: canonical output port does not fit i64.")
                })?,
            ),
        });
    }
    if total_output != spec.output_size {
        return Err(GraphError::new(format!(
            "Graph: output size mismatch; declared {}, resolved {total_output}.",
            spec.output_size
        )));
    }

    let canonical_layout_bytes = encode_canonical_layout(
        &compiled_nodes,
        &normalized_outputs,
        total_parameters,
        total_state_size,
        total_output,
        limits,
    )?;
    let architecture_key = architecture_key(&canonical_layout_bytes, limits)?;
    let layout_digest_sha256: [u8; 32] = Sha256::digest(&canonical_layout_bytes).into();
    let mut order = reserved_vec(compiled_nodes.len(), "compiled node identifiers")?;
    order.extend(compiled_nodes.iter().map(|node| node.id.clone()));
    Ok(CompiledGraph {
        layout_version: CANONICAL_GRAPH_LAYOUT_VERSION,
        architecture_key,
        canonical_layout_bytes,
        layout_digest_sha256,
        nodes: compiled_nodes,
        order,
        total_parameters,
        total_state_size,
        output_size: total_output,
        outputs: normalized_outputs,
        recurrent_nodes,
    })
}

/// Require one input whose inferred width agrees with a node declaration.
fn require_one_matching_input(
    id: &str,
    kind: &str,
    inputs: &[CompiledInputRef],
    input_sizes: &[usize],
    declared: usize,
) -> Result<(), GraphError> {
    if inputs.len() != 1 {
        return Err(GraphError::new(format!(
            "Graph: {kind} node {id} must have exactly one input."
        )));
    }
    if input_sizes[0] != declared {
        return Err(GraphError::new(format!(
            "Graph: {kind} {id} input size mismatch; declared {declared}, resolved {}.",
            input_sizes[0]
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construct one edge with implicit source and destination ports.
    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
            from_port: None,
            to_port: None,
        }
    }

    /// Construct one graph output on port zero.
    fn output(node_id: &str) -> GraphOutputRef {
        GraphOutputRef {
            node_id: node_id.into(),
            port: None,
        }
    }

    /// Generous limits used only by this module's focused tests.
    fn test_limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 128,
            max_edges: 1_024,
            max_graph_outputs: 16,
            max_identifier_bytes: 256,
            max_total_referenced_identifier_bytes: 65_536,
            max_tensor_width: 4_096,
            max_mlp_hidden_layers: 32,
            max_split_output_ports: 128,
            max_parameter_floats: 10_000_000,
            max_recurrent_state_floats: 1_000_000,
            max_canonical_layout_bytes: 1_000_000,
            max_architecture_key_bytes: 3_000_000,
        }
    }

    /// Compile with the named test-only limits.
    fn compile_test_graph(spec: &GraphSpec) -> Result<CompiledGraph, GraphError> {
        compile_graph(spec, &test_limits())
    }

    /// Construct the owner-derived default MLP-GRU-Dense fixture.
    fn default_graph() -> GraphSpec {
        GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "mlp".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 83,
                        hidden_sizes: vec![64],
                        output_size: 64,
                    },
                },
                GraphNodeSpec {
                    id: "gru".into(),
                    kind: GraphNodeKind::Gru {
                        input_size: 64,
                        hidden_size: 16,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 16,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                edge("input", "mlp"),
                edge("mlp", "gru"),
                edge("gru", "head"),
            ],
            outputs: vec![output("head")],
            output_size: 2,
        }
    }

    #[test]
    fn default_counts_order_offsets_and_state_are_exact() {
        let graph = compile_test_graph(&default_graph()).unwrap();
        assert_eq!(graph.order, ["input", "mlp", "gru", "head"]);
        assert_eq!(graph.total_parameters, 13_458);
        assert_eq!(graph.total_state_size, 16);
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| (node.parameter_offset, node.parameter_length))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 9_536), (9_536, 3_888), (13_424, 34)]
        );
        assert_eq!(graph.recurrent_nodes[0].state_offset, 0);
        assert_eq!(graph.recurrent_nodes[0].state_length, 16);
        assert!(graph
            .architecture_key
            .starts_with("slither-graph-layout-v1:hex:"));
        assert!(graph
            .canonical_layout_bytes
            .starts_with(CANONICAL_GRAPH_LAYOUT_DOMAIN));
    }

    #[test]
    fn graph_bundle_retains_noncanonical_source_and_exact_derived_layout() {
        let source = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 2,
                        output_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 2 },
                },
            ],
            edges: vec![edge("input", "head")],
            outputs: vec![output("head")],
            output_size: 2,
        };
        let expected_source = source.clone();
        let bundle = GraphBundle::compile(source, &test_limits()).unwrap();
        let independently_compiled = compile_test_graph(&expected_source).unwrap();

        assert_eq!(bundle.spec(), &expected_source);
        assert_eq!(bundle.compiled(), &independently_compiled);
        assert_eq!(bundle.compiled().order, ["input", "head"]);
        assert_ne!(
            bundle
                .spec()
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            bundle
                .compiled()
                .order
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn retained_large_mlp_has_exact_single_node_layout() {
        let retained = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "mlp".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 83,
                        hidden_sizes: vec![512, 512, 512],
                        output_size: 2,
                    },
                },
            ],
            edges: vec![edge("input", "mlp")],
            outputs: vec![output("mlp")],
            output_size: 2,
        };
        let split_shape = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 83 },
                },
                GraphNodeSpec {
                    id: "mlp".into(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 83,
                        hidden_sizes: vec![512, 512],
                        output_size: 512,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 512,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![edge("input", "mlp"), edge("mlp", "head")],
            outputs: vec![output("head")],
            output_size: 2,
        };
        let compiled = compile_test_graph(&retained).unwrap();
        let prior_split = compile_test_graph(&split_shape).unwrap();
        assert_eq!(compiled.total_parameters, 569_346);
        assert_eq!(
            compiled
                .nodes
                .iter()
                .map(|node| (node.parameter_offset, node.parameter_length))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 569_346)]
        );
        assert_eq!(prior_split.total_parameters, 569_346);
        assert_eq!(
            prior_split
                .nodes
                .iter()
                .map(|node| (node.parameter_offset, node.parameter_length))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 568_320), (568_320, 1_026)]
        );
        assert_ne!(
            compiled.canonical_layout_bytes,
            prior_split.canonical_layout_bytes
        );
        assert_ne!(compiled.architecture_key, prior_split.architecture_key);
    }

    /// Construct Split/Concat coverage with optional explicit Concat ordering.
    fn split_concat_graph(explicit: bool) -> GraphSpec {
        let mut left = GraphEdge {
            from: "split".into(),
            to: "concat".into(),
            from_port: Some(0),
            to_port: None,
        };
        let mut right = GraphEdge {
            from: "split".into(),
            to: "concat".into(),
            from_port: Some(1),
            to_port: None,
        };
        if explicit {
            left.to_port = Some(1);
            right.to_port = Some(0);
        }
        GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 4 },
                },
                GraphNodeSpec {
                    id: "split".into(),
                    kind: GraphNodeKind::Split {
                        output_sizes: vec![1, 3],
                    },
                },
                GraphNodeSpec {
                    id: "concat".into(),
                    kind: GraphNodeKind::Concat,
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 4,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![edge("input", "split"), left, right, edge("concat", "head")],
            outputs: vec![output("head")],
            output_size: 2,
        }
    }

    #[test]
    fn split_and_concat_preserve_implicit_and_explicit_input_order() {
        let implicit = compile_test_graph(&split_concat_graph(false)).unwrap();
        let explicit = compile_test_graph(&split_concat_graph(true)).unwrap();
        let implicit_concat = implicit
            .nodes
            .iter()
            .find(|node| node.id == "concat")
            .unwrap();
        let explicit_concat = explicit
            .nodes
            .iter()
            .find(|node| node.id == "concat")
            .unwrap();
        assert_eq!(
            implicit_concat
                .inputs
                .iter()
                .map(|input| input.from_port)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            explicit_concat
                .inputs
                .iter()
                .map(|input| input.from_port)
                .collect::<Vec<_>>(),
            [1, 0]
        );
        assert_ne!(implicit.architecture_key, explicit.architecture_key);
        assert_ne!(implicit.layout_digest_sha256, explicit.layout_digest_sha256);
    }

    #[test]
    fn raw_utf8_order_is_total_and_does_not_normalize_unicode() {
        let ids = [
            "A", "a", "10", "2", "_", "-", "ä", "é", "e\u{301}", "Z", "z",
        ];
        let mut nodes = vec![GraphNodeSpec {
            id: "source".into(),
            kind: GraphNodeKind::Input { output_size: 1 },
        }];
        let mut edges = Vec::new();
        for id in ids {
            nodes.push(GraphNodeSpec {
                id: id.into(),
                kind: GraphNodeKind::Dense {
                    input_size: 1,
                    output_size: 1,
                },
            });
            edges.push(edge("source", id));
            edges.push(edge(id, "merge"));
        }
        nodes.push(GraphNodeSpec {
            id: "merge".into(),
            kind: GraphNodeKind::Concat,
        });
        nodes.push(GraphNodeSpec {
            id: "head".into(),
            kind: GraphNodeKind::Dense {
                input_size: 11,
                output_size: 2,
            },
        });
        edges.push(edge("merge", "head"));
        let compiled = compile_test_graph(&GraphSpec {
            nodes,
            edges,
            outputs: vec![output("head")],
            output_size: 2,
        })
        .unwrap();
        assert_eq!(
            compiled.order,
            [
                "source", "-", "10", "2", "A", "Z", "_", "a", "e\u{301}", "z", "ä", "é", "merge",
                "head"
            ]
        );
        let concat = compiled
            .nodes
            .iter()
            .find(|node| node.id == "merge")
            .unwrap();
        assert_eq!(
            concat
                .inputs
                .iter()
                .map(|input| input.from_id.as_str())
                .collect::<Vec<_>>(),
            ["-", "10", "2", "A", "Z", "_", "a", "e\u{301}", "z", "ä", "é"]
        );
    }

    #[test]
    fn recurrent_state_blocks_are_checked_and_isolated() {
        let graph = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 3 },
                },
                GraphNodeSpec {
                    id: "gru".into(),
                    kind: GraphNodeKind::Gru {
                        input_size: 3,
                        hidden_size: 4,
                    },
                },
                GraphNodeSpec {
                    id: "lstm".into(),
                    kind: GraphNodeKind::Lstm {
                        input_size: 4,
                        hidden_size: 5,
                    },
                },
                GraphNodeSpec {
                    id: "rru".into(),
                    kind: GraphNodeKind::Rru {
                        input_size: 5,
                        hidden_size: 6,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: 6,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                edge("input", "gru"),
                edge("gru", "lstm"),
                edge("lstm", "rru"),
                edge("rru", "head"),
            ],
            outputs: vec![output("head")],
            output_size: 2,
        };
        let compiled = compile_test_graph(&graph).unwrap();
        assert_eq!(compiled.total_state_size, 20);
        assert_eq!(
            compiled
                .recurrent_nodes
                .iter()
                .map(|node| (node.state_offset, node.state_length))
                .collect::<Vec<_>>(),
            [(0, 4), (4, 10), (14, 6)]
        );
    }

    #[test]
    fn canonicalization_ignores_node_and_edge_array_order() {
        let original = default_graph();
        let mut reordered = original.clone();
        reordered.nodes.reverse();
        reordered.edges.reverse();
        let left = compile_test_graph(&original).unwrap();
        let right = compile_test_graph(&reordered).unwrap();
        assert_eq!(left.order, right.order);
        assert_eq!(left.canonical_layout_bytes, right.canonical_layout_bytes);
        assert_eq!(left.architecture_key, right.architecture_key);
        assert_eq!(left.layout_digest_sha256, right.layout_digest_sha256);
        assert_eq!(left.layout_digest_hex(), right.layout_digest_hex());
        assert_eq!(left.layout_digest_hex().len(), 64);
        assert!(left
            .layout_digest_hex()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }

    #[test]
    fn rejects_malformed_cycles_ports_dimensions_and_output_contracts() {
        let mut duplicate = default_graph();
        duplicate.nodes[1].id = "input".into();
        assert!(compile_test_graph(&duplicate)
            .unwrap_err()
            .message()
            .contains("duplicate"));

        let mut empty = default_graph();
        empty.nodes[0].id.clear();
        assert!(compile_test_graph(&empty)
            .unwrap_err()
            .message()
            .contains("nonempty"));

        let mut zero = default_graph();
        zero.nodes[0].kind = GraphNodeKind::Input { output_size: 0 };
        assert!(compile_test_graph(&zero)
            .unwrap_err()
            .message()
            .contains("positive"));

        let mut cycle = default_graph();
        cycle.edges.push(edge("head", "mlp"));
        assert!(compile_test_graph(&cycle)
            .unwrap_err()
            .message()
            .contains("cycle"));

        let mut negative_port = default_graph();
        negative_port.edges[0].from_port = Some(-1);
        assert!(compile_test_graph(&negative_port)
            .unwrap_err()
            .message()
            .contains("negative"));

        let mut invalid_port = default_graph();
        invalid_port.edges[0].from_port = Some(1);
        assert!(compile_test_graph(&invalid_port)
            .unwrap_err()
            .message()
            .contains("invalid from_port"));

        let mut output = default_graph();
        output.output_size = 3;
        assert!(compile_test_graph(&output)
            .unwrap_err()
            .message()
            .contains("must be 2"));

        let mut split = split_concat_graph(false);
        if let GraphNodeKind::Split { output_sizes } = &mut split.nodes[1].kind {
            *output_sizes = vec![1, 2];
        }
        assert!(compile_test_graph(&split)
            .unwrap_err()
            .message()
            .contains("sum"));

        let mut mixed = split_concat_graph(false);
        mixed.edges[1].to_port = Some(0);
        assert!(compile_test_graph(&mixed)
            .unwrap_err()
            .message()
            .contains("mixed"));

        let mut gapped = split_concat_graph(true);
        gapped.edges[1].to_port = Some(2);
        assert!(compile_test_graph(&gapped)
            .unwrap_err()
            .message()
            .contains("contiguous"));
    }

    #[test]
    fn rejects_parameter_and_state_arithmetic_overflow() {
        let mut overflow_limits = test_limits();
        overflow_limits.max_tensor_width = usize::MAX;
        overflow_limits.max_parameter_floats = usize::MAX;
        overflow_limits.max_recurrent_state_floats = usize::MAX;
        let parameter_overflow = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input {
                        output_size: usize::MAX,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: usize::MAX,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![edge("input", "head")],
            outputs: vec![output("head")],
            output_size: 2,
        };
        assert!(compile_graph(&parameter_overflow, &overflow_limits)
            .unwrap_err()
            .message()
            .contains("overflows"));

        let state_overflow = GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: 1 },
                },
                GraphNodeSpec {
                    id: "lstm".into(),
                    kind: GraphNodeKind::Lstm {
                        input_size: 1,
                        hidden_size: usize::MAX,
                    },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: usize::MAX,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![edge("input", "lstm"), edge("lstm", "head")],
            outputs: vec![output("head")],
            output_size: 2,
        };
        assert!(compile_graph(&state_overflow, &overflow_limits)
            .unwrap_err()
            .message()
            .contains("overflows"));
    }

    #[test]
    fn rejects_request_shape_and_identifier_limits_directly() {
        let graph = default_graph();

        let mut limits = test_limits();
        limits.max_nodes = graph.nodes.len() - 1;
        assert!(compile_graph(&graph, &limits)
            .unwrap_err()
            .message()
            .contains("max_nodes"));

        let mut limits = test_limits();
        limits.max_edges = graph.edges.len() - 1;
        assert!(compile_graph(&graph, &limits)
            .unwrap_err()
            .message()
            .contains("max_edges"));

        let mut extra_output = graph.clone();
        extra_output.outputs.push(output("head"));
        let mut limits = test_limits();
        limits.max_graph_outputs = 1;
        assert!(compile_graph(&extra_output, &limits)
            .unwrap_err()
            .message()
            .contains("max_graph_outputs"));

        let mut limits = test_limits();
        limits.max_identifier_bytes = 4;
        assert!(compile_graph(&graph, &limits)
            .unwrap_err()
            .message()
            .contains("max_identifier_bytes"));

        let mut long_edge_reference = graph.clone();
        long_edge_reference.edges[0].from = "source".into();
        let mut limits = test_limits();
        limits.max_identifier_bytes = 5;
        assert!(compile_graph(&long_edge_reference, &limits)
            .unwrap_err()
            .message()
            .contains("edge source identifier byte length"));

        let mut long_output_reference = graph.clone();
        long_output_reference.outputs[0].node_id = "output".into();
        let mut limits = test_limits();
        limits.max_identifier_bytes = 5;
        assert!(compile_graph(&long_output_reference, &limits)
            .unwrap_err()
            .message()
            .contains("graph output identifier byte length"));

        let mut limits = test_limits();
        limits.max_total_referenced_identifier_bytes = 39;
        assert!(compile_graph(&graph, &limits)
            .unwrap_err()
            .message()
            .contains("max_total_referenced_identifier_bytes"));

        let mut limits = test_limits();
        limits.max_tensor_width = 82;
        assert!(compile_graph(&graph, &limits)
            .unwrap_err()
            .message()
            .contains("max_tensor_width"));

        let mut limits = test_limits();
        limits.max_mlp_hidden_layers = 0;
        assert!(compile_graph(&graph, &limits)
            .unwrap_err()
            .message()
            .contains("max_mlp_hidden_layers"));

        let split = split_concat_graph(false);
        let mut limits = test_limits();
        limits.max_split_output_ports = 1;
        assert!(compile_graph(&split, &limits)
            .unwrap_err()
            .message()
            .contains("max_split_output_ports"));

        let mut invalid_limits = test_limits();
        invalid_limits.max_nodes = 0;
        assert!(compile_graph(&graph, &invalid_limits)
            .unwrap_err()
            .message()
            .contains("max_nodes limit must be positive"));
    }

    #[test]
    fn rejects_non_overflowing_parameter_and_state_limits() {
        let graph = default_graph();
        let mut parameter_limits = test_limits();
        parameter_limits.max_parameter_floats = 13_457;
        assert!(compile_graph(&graph, &parameter_limits)
            .unwrap_err()
            .message()
            .contains("max_parameter_floats"));

        let mut state_limits = test_limits();
        state_limits.max_recurrent_state_floats = 15;
        assert!(compile_graph(&graph, &state_limits)
            .unwrap_err()
            .message()
            .contains("max_recurrent_state_floats"));
    }

    #[test]
    fn rejects_canonical_layout_and_architecture_key_ceilings() {
        let graph = default_graph();
        let baseline = compile_test_graph(&graph).unwrap();

        let mut canonical_limits = test_limits();
        canonical_limits.max_canonical_layout_bytes = baseline.canonical_layout_bytes.len() - 1;
        assert!(compile_graph(&graph, &canonical_limits)
            .unwrap_err()
            .message()
            .contains("max_canonical_layout_bytes"));

        let mut key_limits = test_limits();
        key_limits.max_architecture_key_bytes = baseline.architecture_key.len() - 1;
        assert!(compile_graph(&graph, &key_limits)
            .unwrap_err()
            .message()
            .contains("max_architecture_key_bytes"));
    }

    #[test]
    fn legacy_layout_evidence_preserves_exact_fields_and_order() {
        let evidence = LegacyLayoutEvidence {
            compiler_identity: "node-24/icu-77/en-AU".into(),
            architecture_key: "graph|legacy-locale-key".into(),
            parameter_blocks: vec![
                LegacyParameterBlockEvidence {
                    node_id: "ä".into(),
                    parameter_offset: 12,
                    parameter_length: 2,
                },
                LegacyParameterBlockEvidence {
                    node_id: "A".into(),
                    parameter_offset: 0,
                    parameter_length: 7,
                },
            ],
            incoming_orders: vec![LegacyIncomingOrderEvidence {
                node_id: "merge".into(),
                inputs: vec![("ä".into(), 0), ("A".into(), 0)],
            }],
        };
        assert_eq!(evidence.compiler_identity, "node-24/icu-77/en-AU");
        assert_eq!(evidence.architecture_key, "graph|legacy-locale-key");
        assert_eq!(
            evidence
                .parameter_blocks
                .iter()
                .map(|block| (
                    block.node_id.as_str(),
                    block.parameter_offset,
                    block.parameter_length
                ))
                .collect::<Vec<_>>(),
            [("ä", 12, 2), ("A", 0, 7)]
        );
        assert_eq!(evidence.incoming_orders[0].node_id, "merge");
        assert_eq!(
            evidence.incoming_orders[0]
                .inputs
                .iter()
                .map(|(id, port)| (id.as_str(), *port))
                .collect::<Vec<_>>(),
            [("ä", 0), ("A", 0)]
        );
    }
}
