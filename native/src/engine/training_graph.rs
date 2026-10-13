//! Cold canonical metadata for differentiable views over deployed Float32 weights.
//!
//! This module exports compiler facts, not an alternate executor. Affine biases
//! are interleaved with each matrix row; recurrent matrices and biases are
//! contiguous. Explicit strides keep both layouts representable without copying
//! or independently packing parameters in a trainer.

use super::genome::{GenomeInitializationConfig, GENOME_INITIALIZATION_ALGORITHM};
use super::graph::{CompiledNode, CompiledNodeType, GraphBundle};
use super::inference::GraphExecutionPlan;
use super::sensor_layout::SensorLayout;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter, Write};

/// Maximum encoded manifest bytes; checked by the bounded serializer.
pub const MAX_TRAINING_MANIFEST_BYTES: usize = 1024 * 1024;
/// Revision of the exact deployed equations, independent of structural layout.
pub const TRAINING_EQUATION_REVISION: u32 = 1;
/// Domain for the digest of the exact canonical equation metadata below.
pub const TRAINING_EQUATION_DOMAIN: &[u8] = b"slither.training-equations.v1\0";

/// Deployment life rule for recurrent state owned by a population slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrainingLifeStateRule {
    Reset,
    Preserve,
}

/// Environment-dependent deployment rules, kept separate from sensor identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrainingDeployment {
    /// Requested cadence; actual deliveries use the existing fixed-step scheduler.
    pub control_interval_seconds: f64,
    /// Physical-life replacement handling for the deployed recurrent state.
    pub life_state_rule: TrainingLifeStateRule,
    /// Canonical random initializer bias settings; the trainer seed is isolated.
    pub initialization: GenomeInitializationConfig,
}

/// Named logical view into the single canonical parameter vector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingParameterBlock {
    pub name: String,
    /// Offset and strides are measured in Float32 elements, not bytes.
    pub offset: usize,
    /// Logical element count. Strided affine views can have a larger span.
    pub length: usize,
    pub shape: Vec<usize>,
    pub strides: Vec<usize>,
}

/// Explicit hidden/cell view into the brain's packed recurrent state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingStateBlock {
    pub name: &'static str,
    pub offset: usize,
    pub length: usize,
}

/// Resolved incoming edge in semantic concatenation order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingInput {
    pub from_id: String,
    pub from_port: usize,
    pub input_ordinal: usize,
}

/// One compiler-owned node and its canonical numeric views.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingNode {
    pub id: String,
    pub operation: &'static str,
    pub input_size: usize,
    pub output_sizes: Vec<usize>,
    pub hidden_sizes: Vec<usize>,
    pub hidden_size: Option<usize>,
    pub parameter_offset: usize,
    pub parameter_length: usize,
    pub state_offset: Option<usize>,
    pub state_length: usize,
    pub inputs: Vec<TrainingInput>,
    pub parameter_blocks: Vec<TrainingParameterBlock>,
    pub state_blocks: Vec<TrainingStateBlock>,
}

/// Ordered graph output with explicit source port and flattened destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingOutput {
    pub node_id: String,
    pub port: usize,
    pub offset: usize,
    pub length: usize,
}

/// Exact Float32 equations, hashed separately from the graph topology.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingEquations {
    pub revision: u32,
    pub dtype: &'static str,
    pub affine: &'static str,
    pub gru: &'static str,
    pub lstm: &'static str,
    pub rru: &'static str,
    pub concat: &'static str,
    pub split: &'static str,
    pub arithmetic: &'static str,
}

impl TrainingEquations {
    /// Canonical formulas match the complete-graph kernels in `inference.rs`.
    fn canonical() -> Self {
        Self {
            revision: TRAINING_EQUATION_REVISION,
            dtype: "Float32",
            affine: "y=tanh(W*x+b); every Dense and every MLP layer, including output; W row-major with row-interleaved bias",
            gru: "z=sigmoid((Wz*x+Uz*h)+bz); r=sigmoid((Wr*x+Ur*h)+br); candidate=tanh((Wh*x+Uh*(r*h))+bh); h'=(1-z)*h+z*candidate",
            lstm: "i=sigmoid((Wi*x+Ui*h)+bi); f=sigmoid((Wf*x+Uf*h)+bf); o=sigmoid((Wo*x+Uo*h)+bo); g=tanh((Wg*x+Ug*h)+bg); c'=f*c+i*g; h'=o*tanh(c'); state=[h,c]",
            rru: "candidate=tanh((Wc*x+Uc*h)+bc); gate=sigmoid((Wr*x+Ur*h)+br); h'=(1-gate)*h+gate*candidate",
            concat: "concatenate source ports in increasing resolved inputOrdinal; fan-out reads the same source values",
            split: "contiguous source slices in outputSizes order; zero-based ports",
            arithmetic: "sigmoid(x)=1/(1+exp(-x)); Float32 intermediates; scalar/SSE2 dot accumulation may differ within parity tolerances",
        }
    }
}

/// Recorded initialization semantics; initial weights must come from Rust.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingInitialization {
    pub algorithm: &'static str,
    pub rng_scope: &'static str,
    pub dense_uniform_scale: f64,
    pub mlp_uniform_scale: f64,
    pub recurrent_input_uniform_scale: f64,
    pub recurrent_state_uniform_scale: f64,
    pub recurrent_bias_uniform_scale: f64,
    pub gru_update_bias: f64,
    pub lstm_forget_bias: f64,
    pub rru_gate_bias: f64,
}

/// Cadence and motor interpretation actually used by native deployment.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingDeploymentManifest {
    pub fixed_step_hz: u32,
    pub control_interval_seconds: f64,
    pub cadence: &'static str,
    pub first_physical_sample: &'static str,
    pub life_state_rule: TrainingLifeStateRule,
    pub generation_state_rule: &'static str,
    pub turn_output_index: usize,
    pub boost_output_index: usize,
    pub turn_mapping: &'static str,
    pub boost_mapping: &'static str,
    pub boost_threshold: f32,
}

/// Rust-generated training description. It is not an admission credential.
/// Consumers must independently validate deployed graph/weights through Rust.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalTrainingGraphManifestV1 {
    pub schema_version: u32,
    pub graph_spec: serde_json::Value,
    pub architecture_key: String,
    pub layout_version: u32,
    pub layout_sha256: String,
    pub equations: TrainingEquations,
    pub equations_sha256: String,
    pub topological_order: Vec<String>,
    pub nodes: Vec<TrainingNode>,
    pub outputs: Vec<TrainingOutput>,
    pub total_parameters: usize,
    pub total_state_size: usize,
    pub input_size: usize,
    pub output_size: usize,
    /// Exact Rust canonical sensor bytes encoded as a UTF-8 JSON string.
    pub sensor_canonical_json: String,
    pub sensor_sha256: String,
    pub deployment: TrainingDeploymentManifest,
    pub initialization: TrainingInitialization,
}

/// Cold construction/encoding failure, before a trainer receives any manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingManifestError(pub String);
impl Display for TrainingManifestError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for TrainingManifestError {}

impl CanonicalTrainingGraphManifestV1 {
    /// Derive all layout facts from an inseparable admitted source/compiled graph.
    pub fn build(
        graph: &GraphBundle,
        sensors: &SensorLayout,
        deployment: TrainingDeployment,
    ) -> Result<Self, TrainingManifestError> {
        let failure = |error: &dyn Display| TrainingManifestError(error.to_string());
        if !deployment.control_interval_seconds.is_finite()
            || !(0.008..=0.06).contains(&deployment.control_interval_seconds)
        {
            return Err(TrainingManifestError("invalid deployment cadence".into()));
        }
        deployment
            .initialization
            .validate()
            .map_err(|e| failure(&e))?;
        let plan = GraphExecutionPlan::build(graph).map_err(|e| failure(&e))?;
        let sensor = sensors.contract_identity().map_err(|e| failure(&e))?;
        if sensor.input_size() != plan.input_size() {
            return Err(TrainingManifestError(
                "graph/sensor input width mismatch".into(),
            ));
        }
        let equations = TrainingEquations::canonical();
        let mut hash = Sha256::new();
        hash.update(TRAINING_EQUATION_DOMAIN);
        hash.update(serde_json::to_vec(&equations).map_err(|e| failure(&e))?);
        let equations_sha256 = hex_digest(&hash.finalize());
        let nodes = graph
            .nodes
            .iter()
            .map(training_node)
            .collect::<Result<Vec<_>, _>>()?;
        let mut output_offset = 0;
        let mut outputs = Vec::with_capacity(graph.outputs.len());
        for output in &graph.outputs {
            let port = output.port.unwrap_or(0) as usize;
            let node = graph
                .nodes
                .iter()
                .find(|node| node.id == output.node_id)
                .ok_or_else(|| TrainingManifestError("missing compiled output".into()))?;
            let length = node.output_sizes[port];
            outputs.push(TrainingOutput {
                node_id: node.id.clone(),
                port,
                offset: output_offset,
                length,
            });
            output_offset += length;
        }
        let bias = deployment.initialization;
        let manifest = Self {
            schema_version: 1,
            graph_spec: super::run_start::graph_spec_metadata(graph.spec()),
            architecture_key: graph.architecture_key.clone(),
            layout_version: graph.layout_version,
            layout_sha256: graph.layout_digest_hex(),
            equations,
            equations_sha256,
            topological_order: graph.order.clone(),
            nodes,
            outputs,
            total_parameters: graph.total_parameters,
            total_state_size: graph.total_state_size,
            input_size: plan.input_size(),
            output_size: graph.output_size,
            sensor_canonical_json: String::from_utf8(sensor.canonical_bytes().map_err(|e| failure(&e))?)
                .map_err(|e| failure(&e))?,
            sensor_sha256: sensor.sha256_hex().map_err(|e| failure(&e))?,
            deployment: TrainingDeploymentManifest {
                fixed_step_hz: 60,
                control_interval_seconds: deployment.control_interval_seconds,
                cadence: "shared next_neural_boundary accumulator v1; at most one delivery per complete fixed step; use actual delivery gaps",
                first_physical_sample: "forced due on first admitted control boundary, independent of cadence; v4 first_sample=+1; v3 has no first_sample channel",
                life_state_rule: deployment.life_state_rule,
                generation_state_rule: "zero recurrent state before generation spawn and sensing",
                turn_output_index: 0, boost_output_index: 1,
                turn_mapping: "clamp(raw_turn,-1,1)",
                boost_mapping: "clamp(raw_boost,-1,1)>threshold; strict comparison; request distinct from realized boost",
                boost_threshold: 0.35,
            },
            initialization: TrainingInitialization {
                algorithm: GENOME_INITIALIZATION_ALGORITHM,
                rng_scope: "Rust initializer with an isolated trainer seed; never advance authoritative evolution RNG",
                dense_uniform_scale: 0.45, mlp_uniform_scale: 0.6,
                recurrent_input_uniform_scale: 0.35, recurrent_state_uniform_scale: 0.18,
                recurrent_bias_uniform_scale: 0.10,
                gru_update_bias: bias.gru_update_bias,
                lstm_forget_bias: bias.lstm_forget_bias,
                rru_gate_bias: bias.rru_gate_bias,
            },
        };
        // Check the transport ceiling now, not after a consumer starts export.
        manifest.canonical_bytes()?;
        Ok(manifest)
    }

    /// Serialize deterministically and abort before writing beyond the ceiling.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, TrainingManifestError> {
        struct BoundedWriter(Vec<u8>);
        impl std::io::Write for BoundedWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_TRAINING_MANIFEST_BYTES.saturating_sub(self.0.len()) {
                    return Err(std::io::Error::other("training manifest byte limit"));
                }
                self.0
                    .try_reserve(bytes.len())
                    .map_err(std::io::Error::other)?;
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = BoundedWriter(Vec::new());
        serde_json::to_writer(&mut writer, self)
            .map_err(|e| TrainingManifestError(e.to_string()))?;
        Ok(writer.0)
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("String formatting");
    }
    encoded
}

fn training_node(node: &CompiledNode) -> Result<TrainingNode, TrainingManifestError> {
    use CompiledNodeType::*;
    let mut blocks = Vec::new();
    let mut cursor = node.parameter_offset;
    let operation = match node.node_type {
        Input => "Input",
        Dense => "Dense",
        Mlp => "MLP",
        Gru => "GRU",
        Lstm => "LSTM",
        Rru => "RRU",
        Concat => "Concat",
        Split => "Split",
    };
    let mut add =
        |name: String, rows: usize, cols: usize, row_stride: usize, offset: usize, vector: bool| {
            blocks.push(TrainingParameterBlock {
                name,
                offset,
                length: rows * cols,
                shape: if vector { vec![rows] } else { vec![rows, cols] },
                strides: if vector {
                    vec![row_stride]
                } else {
                    vec![row_stride, 1]
                },
            });
        };
    match node.node_type {
        Dense | Mlp => {
            let mut input = node.input_size;
            for (layer, output) in node
                .hidden_sizes
                .iter()
                .copied()
                .chain(std::iter::once(node.output_size))
                .enumerate()
            {
                add(
                    format!("layer{layer}.W"),
                    output,
                    input,
                    input + 1,
                    cursor,
                    false,
                );
                add(
                    format!("layer{layer}.b"),
                    output,
                    1,
                    input + 1,
                    cursor + input,
                    true,
                );
                cursor += output * (input + 1);
                input = output;
            }
        }
        Gru | Lstm | Rru => {
            let hidden = node
                .hidden_size
                .ok_or_else(|| TrainingManifestError("missing recurrent width".into()))?;
            let gates: &[&str] = match node.node_type {
                Gru => &["z", "r", "h"],
                Lstm => &["i", "f", "o", "g"],
                _ => &["c", "r"],
            };
            for (prefix, columns) in [("W", node.input_size), ("U", hidden), ("b", 1)] {
                for gate in gates {
                    add(
                        format!("{prefix}{gate}"),
                        hidden,
                        columns,
                        columns,
                        cursor,
                        prefix == "b",
                    );
                    cursor += hidden * columns;
                }
            }
        }
        _ => {}
    }
    if cursor != node.parameter_offset + node.parameter_length {
        return Err(TrainingManifestError(
            "parameter views disagree with compiler".into(),
        ));
    }
    let mut state_blocks = Vec::new();
    if let Some(offset) = node.state_offset {
        let hidden = node
            .hidden_size
            .ok_or_else(|| TrainingManifestError("missing state width".into()))?;
        state_blocks.push(TrainingStateBlock {
            name: "hidden",
            offset,
            length: hidden,
        });
        if node.node_type == Lstm {
            state_blocks.push(TrainingStateBlock {
                name: "cell",
                offset: offset + hidden,
                length: hidden,
            });
        }
    }
    Ok(TrainingNode {
        id: node.id.clone(),
        operation,
        input_size: node.input_size,
        output_sizes: node.output_sizes.clone(),
        hidden_sizes: node.hidden_sizes.clone(),
        hidden_size: node.hidden_size,
        parameter_offset: node.parameter_offset,
        parameter_length: node.parameter_length,
        state_offset: node.state_offset,
        state_length: node.state_length,
        inputs: node
            .inputs
            .iter()
            .map(|edge| TrainingInput {
                from_id: edge.from_id.clone(),
                from_port: edge.from_port,
                input_ordinal: edge.input_ordinal,
            })
            .collect(),
        parameter_blocks: blocks,
        state_blocks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::{
        GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
    };
    use crate::engine::sensor_contract::{SensorContractIdentity, SensorProfile};

    fn deployment() -> TrainingDeployment {
        TrainingDeployment {
            control_interval_seconds: 0.03,
            life_state_rule: TrainingLifeStateRule::Preserve,
            initialization: GenomeInitializationConfig::default(),
        }
    }

    fn mixed_graph(input: usize) -> GraphBundle {
        use GraphNodeKind::*;
        let node = |id: &str, kind| GraphNodeSpec {
            id: id.into(),
            kind,
        };
        let edge = |from: &str, to: &str, from_port, to_port| GraphEdge {
            from: from.into(),
            to: to.into(),
            from_port: Some(from_port),
            to_port: Some(to_port),
        };
        let spec = GraphSpec {
            // Deliberately not in topological order.
            nodes: vec![
                node(
                    "split",
                    Split {
                        output_sizes: vec![1, 1],
                    },
                ),
                node(
                    "rru",
                    Rru {
                        input_size: 4,
                        hidden_size: 2,
                    },
                ),
                node("input", Input { output_size: input }),
                node(
                    "lstm",
                    Lstm {
                        input_size: 3,
                        hidden_size: 2,
                    },
                ),
                node(
                    "head",
                    Dense {
                        input_size: 2,
                        output_size: 2,
                    },
                ),
                node(
                    "mlp",
                    Mlp {
                        input_size: input,
                        hidden_sizes: vec![5, 4],
                        output_size: 3,
                    },
                ),
                node(
                    "gru",
                    Gru {
                        input_size: 3,
                        hidden_size: 2,
                    },
                ),
                node("concat", Concat),
            ],
            edges: vec![
                edge("input", "mlp", 0, 0),
                edge("mlp", "lstm", 0, 0),
                edge("mlp", "gru", 0, 0),
                edge("lstm", "concat", 0, 0),
                edge("gru", "concat", 0, 1),
                edge("concat", "rru", 0, 0),
                edge("rru", "head", 0, 0),
                edge("head", "split", 0, 0),
            ],
            // Reverse output ports to prove ordering is not inferred by node ID.
            outputs: vec![
                GraphOutputRef {
                    node_id: "split".into(),
                    port: Some(1),
                },
                GraphOutputRef {
                    node_id: "split".into(),
                    port: Some(0),
                },
            ],
            output_size: 2,
        };
        GraphBundle::compile(spec, &crate::engine::fresh_run::stage6a_p0_graph_limits()).unwrap()
    }

    #[test]
    fn named_views_cover_every_parameter_and_state_exactly_once() {
        for profile in [
            SensorProfile::V3,
            SensorProfile::Basic,
            SensorProfile::Motion,
        ] {
            for bins in [8, 16, 32] {
                let sensor = SensorLayout::for_profile(profile, bins).unwrap();
                let graph = mixed_graph(sensor.input_size);
                let manifest =
                    CanonicalTrainingGraphManifestV1::build(&graph, &sensor, deployment()).unwrap();
                let mut parameter_owners = vec![0_u8; graph.total_parameters];
                let mut state_owners = vec![0_u8; graph.total_state_size];
                for node in &manifest.nodes {
                    for block in &node.parameter_blocks {
                        assert_eq!(block.length, block.shape.iter().product::<usize>());
                        for logical in 0..block.length {
                            let index = if block.shape.len() == 1 {
                                block.offset + logical * block.strides[0]
                            } else {
                                block.offset
                                    + (logical / block.shape[1]) * block.strides[0]
                                    + (logical % block.shape[1]) * block.strides[1]
                            };
                            assert!(index >= node.parameter_offset);
                            assert!(index < node.parameter_offset + node.parameter_length);
                            parameter_owners[index] += 1;
                        }
                    }
                    for block in &node.state_blocks {
                        for owner in &mut state_owners[block.offset..block.offset + block.length] {
                            *owner += 1;
                        }
                    }
                }
                assert!(parameter_owners.iter().all(|count| *count == 1));
                assert!(state_owners.iter().all(|count| *count == 1));
                assert_eq!(manifest.layout_sha256, graph.layout_digest_hex());
                assert_eq!(manifest.topological_order, graph.order);
                assert_eq!(
                    manifest
                        .outputs
                        .iter()
                        .map(|out| (out.port, out.offset, out.length))
                        .collect::<Vec<_>>(),
                    [(1, 0, 1), (0, 1, 1)]
                );
                let concat = manifest
                    .nodes
                    .iter()
                    .find(|node| node.id == "concat")
                    .unwrap();
                assert_eq!(
                    concat
                        .inputs
                        .iter()
                        .map(|edge| (edge.from_id.as_str(), edge.input_ordinal))
                        .collect::<Vec<_>>(),
                    [("lstm", 0), ("gru", 1)]
                );
                let lstm = manifest
                    .nodes
                    .iter()
                    .find(|node| node.id == "lstm")
                    .unwrap();
                assert_eq!(lstm.state_blocks[1].offset, lstm.state_blocks[0].offset + 2);
                let decoded = SensorContractIdentity::decode_supported(
                    manifest.sensor_canonical_json.as_bytes(),
                )
                .unwrap();
                assert_eq!(manifest.sensor_sha256, decoded.sha256_hex().unwrap());
            }
        }
    }

    #[test]
    fn recurrent_gate_names_and_offsets_match_deployed_packing() {
        let sensor = SensorLayout::new(16).unwrap();
        let manifest = CanonicalTrainingGraphManifestV1::build(
            &mixed_graph(sensor.input_size),
            &sensor,
            deployment(),
        )
        .unwrap();
        for (id, expected) in [
            (
                "gru",
                vec!["Wz", "Wr", "Wh", "Uz", "Ur", "Uh", "bz", "br", "bh"],
            ),
            (
                "lstm",
                vec![
                    "Wi", "Wf", "Wo", "Wg", "Ui", "Uf", "Uo", "Ug", "bi", "bf", "bo", "bg",
                ],
            ),
            ("rru", vec!["Wc", "Wr", "Uc", "Ur", "bc", "br"]),
        ] {
            let node = manifest.nodes.iter().find(|node| node.id == id).unwrap();
            assert_eq!(
                node.parameter_blocks
                    .iter()
                    .map(|block| block.name.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            let mut offset = node.parameter_offset;
            for block in &node.parameter_blocks {
                assert_eq!(block.offset, offset);
                offset += block.length;
            }
            assert_eq!(offset, node.parameter_offset + node.parameter_length);
        }
        let mlp = manifest.nodes.iter().find(|node| node.id == "mlp").unwrap();
        assert_eq!(mlp.parameter_blocks[0].strides, [sensor.input_size + 1, 1]);
        assert_eq!(
            mlp.parameter_blocks[1].offset,
            mlp.parameter_offset + sensor.input_size
        );
        assert_eq!(
            mlp.parameter_blocks[2].offset,
            mlp.parameter_offset + 5 * (sensor.input_size + 1)
        );
    }

    #[test]
    fn deterministic_equations_and_sensor_identity_are_separate_from_deployment() {
        let sensor = SensorLayout::new(16).unwrap();
        let graph = mixed_graph(sensor.input_size);
        let before =
            CanonicalTrainingGraphManifestV1::build(&graph, &sensor, deployment()).unwrap();
        let mut changed = deployment();
        changed.control_interval_seconds = 0.06;
        changed.life_state_rule = TrainingLifeStateRule::Reset;
        changed.initialization.gru_update_bias = -1.0;
        let after = CanonicalTrainingGraphManifestV1::build(&graph, &sensor, changed).unwrap();
        assert_eq!(before.equations_sha256, after.equations_sha256);
        assert_eq!(before.layout_sha256, after.layout_sha256);
        assert_eq!(before.sensor_sha256, after.sensor_sha256);
        assert_ne!(
            before.canonical_bytes().unwrap(),
            after.canonical_bytes().unwrap()
        );
        assert_eq!(
            before.canonical_bytes().unwrap(),
            before.canonical_bytes().unwrap()
        );
        assert!(before.equations.gru.contains("Uh*(r*h)"));
        assert!(before.equations.gru.contains("(1-z)*h+z*candidate"));
        assert_eq!(before.deployment.boost_threshold, 0.35_f32);
    }

    #[test]
    fn invalid_sensor_cadence_bias_and_oversized_transport_reject() {
        let mut sensor = SensorLayout::new(16).unwrap();
        let graph = mixed_graph(sensor.input_size);
        for interval in [f64::NAN, f64::INFINITY, 0.0, 0.0079, 0.0601] {
            let mut config = deployment();
            config.control_interval_seconds = interval;
            assert!(CanonicalTrainingGraphManifestV1::build(&graph, &sensor, config).is_err());
        }
        let mut config = deployment();
        config.initialization.gru_update_bias = f64::NAN;
        assert!(CanonicalTrainingGraphManifestV1::build(&graph, &sensor, config).is_err());
        sensor.order.swap(0, 1);
        assert!(CanonicalTrainingGraphManifestV1::build(&graph, &sensor, deployment()).is_err());
        assert!(CanonicalTrainingGraphManifestV1::build(
            &graph,
            &SensorLayout::new(8).unwrap(),
            deployment()
        )
        .is_err());
        let sensor = SensorLayout::new(16).unwrap();
        let mut manifest =
            CanonicalTrainingGraphManifestV1::build(&graph, &sensor, deployment()).unwrap();
        manifest.architecture_key = "x".repeat(MAX_TRAINING_MANIFEST_BYTES);
        assert!(manifest
            .canonical_bytes()
            .unwrap_err()
            .0
            .contains("byte limit"));
    }
}
