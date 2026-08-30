//! Versioned TypeScript-compatible random genome initialization.
//!
//! Every parameter-bearing compiled node consumes one uniform draw per packed
//! Float32 in canonical node/range order. Input, Split, and Concat nodes consume
//! no draws. The source RNG state is borrowed and immutable; a complete weight
//! vector plus the exact continuation is returned only after validation and
//! fallible output reservation succeed.

use super::graph::{CompiledGraph, CompiledNode, CompiledNodeType};
use super::rng::{RngError, SerializedRngState, StatefulRng};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Version of the packed random-initialization contract.
pub const GENOME_INITIALIZATION_VERSION: u32 = 1;
/// Stable identity for the current TypeScript `Genome.random` draw/formula order.
pub const GENOME_INITIALIZATION_ALGORITHM: &str = "typescript-uniform-v1";

/// Reset-only recurrent bias settings used by random genome initialization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenomeInitializationConfig {
    /// Initial GRU update-gate bias before the small random perturbation.
    pub gru_update_bias: f64,
    /// Initial LSTM forget-gate bias before the small random perturbation.
    pub lstm_forget_bias: f64,
    /// Initial RRU reset-gate bias before the small random perturbation.
    pub rru_gate_bias: f64,
}

impl GenomeInitializationConfig {
    /// Current TypeScript defaults.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            gru_update_bias: -0.7,
            lstm_forget_bias: 0.6,
            rru_gate_bias: 0.1,
        }
    }

    /// Validate the same admitted ranges exposed by reset-only settings.
    pub fn validate(self) -> Result<(), GenomeInitializationError> {
        validate_bias("brain.gruInitUpdateBias", self.gru_update_bias, -2.5, 1.5)?;
        validate_bias("brain.lstmInitForgetBias", self.lstm_forget_bias, -1.5, 3.0)?;
        validate_bias("brain.rruInitGateBias", self.rru_gate_bias, -1.5, 2.0)
    }
}

impl Default for GenomeInitializationConfig {
    fn default() -> Self {
        Self::typescript_defaults()
    }
}

/// Complete random genome and exact RNG continuation.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializedGenome {
    weights: Vec<f32>,
    next_rng: SerializedRngState,
}

impl InitializedGenome {
    /// Borrow packed Float32 weights in canonical compiled-node order.
    #[must_use]
    pub fn weights(&self) -> &[f32] {
        &self.weights
    }

    /// Borrow the exact continuation after every attempted parameter draw.
    #[must_use]
    pub const fn next_rng(&self) -> &SerializedRngState {
        &self.next_rng
    }

    /// Consume the staged result for later authority-level joining.
    #[must_use]
    pub fn into_parts(self) -> (Vec<f32>, SerializedRngState) {
        (self.weights, self.next_rng)
    }
}

/// Failure before a complete initialized genome is available.
#[derive(Debug)]
pub enum GenomeInitializationError {
    /// One reset-only bias is non-finite or outside its admitted product range.
    InvalidBias { path: &'static str },
    /// Compiled parameter ranges are not contiguous and internally consistent.
    InvalidCompiledLayout {
        node_id: String,
        reason: &'static str,
    },
    /// Checked parameter-count arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Output storage could not be reserved without an unchecked growth path.
    AllocationFailed { parameter_floats: usize },
    /// The source continuation is not an admitted engine RNG state.
    Rng(RngError),
    /// A generated Float32 was unexpectedly NaN or infinite.
    NonFiniteGeneratedWeight { node_id: String, parameter: usize },
}

impl Display for GenomeInitializationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBias { path } => {
                write!(
                    formatter,
                    "genome initialization setting {path} is out of range"
                )
            }
            Self::InvalidCompiledLayout { node_id, reason } => {
                write!(formatter, "compiled genome node {node_id} has {reason}")
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "genome initialization overflow while calculating {context}"
                )
            }
            Self::AllocationFailed { parameter_floats } => write!(
                formatter,
                "unable to reserve {parameter_floats} Float32 genome parameters"
            ),
            Self::Rng(error) => write!(formatter, "invalid genome RNG continuation: {error}"),
            Self::NonFiniteGeneratedWeight { node_id, parameter } => write!(
                formatter,
                "genome node {node_id} generated non-finite parameter {parameter}"
            ),
        }
    }
}

impl Error for GenomeInitializationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Rng(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RngError> for GenomeInitializationError {
    fn from(error: RngError) -> Self {
        Self::Rng(error)
    }
}

/// Initialize one complete differently weighted brain without mutating source state.
pub fn initialize_random_genome(
    graph: &CompiledGraph,
    source_rng: &SerializedRngState,
    config: GenomeInitializationConfig,
) -> Result<InitializedGenome, GenomeInitializationError> {
    config.validate()?;
    validate_compiled_layout(graph)?;
    let mut weights = Vec::new();
    weights
        .try_reserve_exact(graph.total_parameters)
        .map_err(|_| GenomeInitializationError::AllocationFailed {
            parameter_floats: graph.total_parameters,
        })?;
    weights.resize(graph.total_parameters, 0.0);
    let mut rng = StatefulRng::from_state(source_rng)?;

    for node in &graph.nodes {
        let range_end = node
            .parameter_offset
            .checked_add(node.parameter_length)
            .ok_or(GenomeInitializationError::ArithmeticOverflow {
                context: "compiled node parameter range",
            })?;
        let node_weights = weights
            .get_mut(node.parameter_offset..range_end)
            .ok_or_else(|| GenomeInitializationError::InvalidCompiledLayout {
                node_id: node.id.clone(),
                reason: "an out-of-bounds parameter range",
            })?;
        initialize_node(node, node_weights, &mut rng, config)?;
        if let Some((parameter, _)) = node_weights
            .iter()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(GenomeInitializationError::NonFiniteGeneratedWeight {
                node_id: node.id.clone(),
                parameter,
            });
        }
    }

    Ok(InitializedGenome {
        weights,
        next_rng: rng.export_state(),
    })
}

fn validate_bias(
    path: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), GenomeInitializationError> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(GenomeInitializationError::InvalidBias { path });
    }
    Ok(())
}

fn validate_compiled_layout(graph: &CompiledGraph) -> Result<(), GenomeInitializationError> {
    let mut expected_offset = 0usize;
    for node in &graph.nodes {
        if node.parameter_offset != expected_offset {
            return Err(GenomeInitializationError::InvalidCompiledLayout {
                node_id: node.id.clone(),
                reason: "a non-contiguous parameter offset",
            });
        }
        let expected_length = expected_parameter_length(node)?;
        if node.parameter_length != expected_length {
            return Err(GenomeInitializationError::InvalidCompiledLayout {
                node_id: node.id.clone(),
                reason: "a parameter length inconsistent with its node shape",
            });
        }
        expected_offset = expected_offset.checked_add(expected_length).ok_or(
            GenomeInitializationError::ArithmeticOverflow {
                context: "complete compiled parameter length",
            },
        )?;
    }
    if expected_offset != graph.total_parameters {
        return Err(GenomeInitializationError::InvalidCompiledLayout {
            node_id: "<graph>".to_owned(),
            reason: "a total parameter count inconsistent with its node ranges",
        });
    }
    Ok(())
}

fn expected_parameter_length(node: &CompiledNode) -> Result<usize, GenomeInitializationError> {
    match node.node_type {
        CompiledNodeType::Input | CompiledNodeType::Concat | CompiledNodeType::Split => Ok(0),
        CompiledNodeType::Dense => affine_parameter_count(node.input_size, node.output_size),
        CompiledNodeType::Mlp => {
            let mut total = 0usize;
            let mut input = node.input_size;
            for output in node
                .hidden_sizes
                .iter()
                .copied()
                .chain(std::iter::once(node.output_size))
            {
                total = total
                    .checked_add(affine_parameter_count(input, output)?)
                    .ok_or(GenomeInitializationError::ArithmeticOverflow {
                        context: "MLP parameter length",
                    })?;
                input = output;
            }
            Ok(total)
        }
        CompiledNodeType::Gru => recurrent_parameter_count(node, 3),
        CompiledNodeType::Lstm => recurrent_parameter_count(node, 4),
        CompiledNodeType::Rru => recurrent_parameter_count(node, 2),
    }
}

fn affine_parameter_count(input: usize, output: usize) -> Result<usize, GenomeInitializationError> {
    input
        .checked_add(1)
        .and_then(|row| row.checked_mul(output))
        .ok_or(GenomeInitializationError::ArithmeticOverflow {
            context: "affine parameter length",
        })
}

fn recurrent_parameter_count(
    node: &CompiledNode,
    gates: usize,
) -> Result<usize, GenomeInitializationError> {
    let hidden =
        node.hidden_size
            .ok_or_else(|| GenomeInitializationError::InvalidCompiledLayout {
                node_id: node.id.clone(),
                reason: "a missing recurrent hidden size",
            })?;
    if node.output_size != hidden {
        return Err(GenomeInitializationError::InvalidCompiledLayout {
            node_id: node.id.clone(),
            reason: "a recurrent output width different from its hidden width",
        });
    }
    node.input_size
        .checked_add(hidden)
        .and_then(|width| width.checked_add(1))
        .and_then(|width| width.checked_mul(hidden))
        .and_then(|per_gate| per_gate.checked_mul(gates))
        .ok_or(GenomeInitializationError::ArithmeticOverflow {
            context: "recurrent parameter length",
        })
}

fn initialize_node(
    node: &CompiledNode,
    weights: &mut [f32],
    rng: &mut StatefulRng,
    config: GenomeInitializationConfig,
) -> Result<(), GenomeInitializationError> {
    match node.node_type {
        CompiledNodeType::Input | CompiledNodeType::Concat | CompiledNodeType::Split => Ok(()),
        CompiledNodeType::Mlp => {
            fill_scaled(weights, rng, 0.6);
            Ok(())
        }
        CompiledNodeType::Dense => {
            for weight in weights {
                let sampled = signed_uniform(rng, 0.45).clamp(-5.0, 5.0);
                *weight = sampled as f32;
            }
            Ok(())
        }
        CompiledNodeType::Gru => {
            initialize_recurrent(node, weights, rng, 3, Some((0, config.gru_update_bias)))
        }
        CompiledNodeType::Lstm => {
            initialize_recurrent(node, weights, rng, 4, Some((1, config.lstm_forget_bias)))
        }
        CompiledNodeType::Rru => {
            initialize_recurrent(node, weights, rng, 2, Some((1, config.rru_gate_bias)))
        }
    }
}

fn initialize_recurrent(
    node: &CompiledNode,
    weights: &mut [f32],
    rng: &mut StatefulRng,
    gates: usize,
    initialized_bias: Option<(usize, f64)>,
) -> Result<(), GenomeInitializationError> {
    let hidden =
        node.hidden_size
            .ok_or_else(|| GenomeInitializationError::InvalidCompiledLayout {
                node_id: node.id.clone(),
                reason: "a missing recurrent hidden size",
            })?;
    let input_weights = gates
        .checked_mul(hidden)
        .and_then(|value| value.checked_mul(node.input_size))
        .ok_or(GenomeInitializationError::ArithmeticOverflow {
            context: "recurrent input-weight range",
        })?;
    let recurrent_weights = gates
        .checked_mul(hidden)
        .and_then(|value| value.checked_mul(hidden))
        .ok_or(GenomeInitializationError::ArithmeticOverflow {
            context: "recurrent state-weight range",
        })?;
    let bias_count =
        gates
            .checked_mul(hidden)
            .ok_or(GenomeInitializationError::ArithmeticOverflow {
                context: "recurrent bias range",
            })?;
    let recurrent_end = input_weights.checked_add(recurrent_weights).ok_or(
        GenomeInitializationError::ArithmeticOverflow {
            context: "recurrent weight ranges",
        },
    )?;
    let expected = recurrent_end.checked_add(bias_count).ok_or(
        GenomeInitializationError::ArithmeticOverflow {
            context: "complete recurrent range",
        },
    )?;
    if weights.len() != expected {
        return Err(GenomeInitializationError::InvalidCompiledLayout {
            node_id: node.id.clone(),
            reason: "an inconsistent recurrent parameter slice",
        });
    }

    fill_scaled(&mut weights[..input_weights], rng, 0.35);
    fill_scaled(&mut weights[input_weights..recurrent_end], rng, 0.18);
    for gate in 0..gates {
        let base = initialized_bias
            .filter(|(initialized_gate, _)| *initialized_gate == gate)
            .map_or(0.0, |(_, value)| value);
        let start = recurrent_end + gate * hidden;
        let end = start + hidden;
        for bias in &mut weights[start..end] {
            *bias = (base + signed_uniform(rng, 0.10)) as f32;
        }
    }
    Ok(())
}

fn fill_scaled(weights: &mut [f32], rng: &mut StatefulRng, scale: f64) {
    for weight in weights {
        *weight = signed_uniform(rng, scale) as f32;
    }
}

fn signed_uniform(rng: &mut StatefulRng, scale: f64) -> f64 {
    (rng.next_f64() * 2.0 - 1.0) * scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::{
        GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        seed: String,
        biases: FixtureBiases,
        compiled_node_ranges: Vec<FixtureNodeRange>,
        total_parameters: usize,
        weight_bits: Vec<String>,
        next_rng: FixtureRng,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureBiases {
        gru_init_update_bias: f64,
        lstm_init_forget_bias: f64,
        rru_init_gate_bias: f64,
    }

    #[derive(Deserialize)]
    struct FixtureNodeRange {
        id: String,
        offset: usize,
        length: usize,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureRng {
        state_hex: String,
        gaussian_spare_valid: bool,
        gaussian_spare_hex: Option<String>,
    }

    fn limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 32,
            max_edges: 64,
            max_graph_outputs: 4,
            max_identifier_bytes: 64,
            max_total_referenced_identifier_bytes: 4_096,
            max_tensor_width: 128,
            max_mlp_hidden_layers: 8,
            max_split_output_ports: 8,
            max_parameter_floats: 100_000,
            max_recurrent_state_floats: 10_000,
            max_canonical_layout_bytes: 100_000,
            max_architecture_key_bytes: 100_000,
        }
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            from_port: None,
            to_port: None,
        }
    }

    fn complete_graph() -> GraphSpec {
        GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".to_owned(),
                    kind: GraphNodeKind::Input { output_size: 4 },
                },
                GraphNodeSpec {
                    id: "split".to_owned(),
                    kind: GraphNodeKind::Split {
                        output_sizes: vec![2, 2],
                    },
                },
                GraphNodeSpec {
                    id: "concat".to_owned(),
                    kind: GraphNodeKind::Concat,
                },
                GraphNodeSpec {
                    id: "mlp".to_owned(),
                    kind: GraphNodeKind::Mlp {
                        input_size: 4,
                        hidden_sizes: vec![3],
                        output_size: 3,
                    },
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
                        input_size: 2,
                        hidden_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "rru".to_owned(),
                    kind: GraphNodeKind::Rru {
                        input_size: 2,
                        hidden_size: 2,
                    },
                },
                GraphNodeSpec {
                    id: "head".to_owned(),
                    kind: GraphNodeKind::Dense {
                        input_size: 2,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![
                edge("input", "split"),
                GraphEdge {
                    from: "split".to_owned(),
                    to: "concat".to_owned(),
                    from_port: Some(0),
                    to_port: Some(0),
                },
                GraphEdge {
                    from: "split".to_owned(),
                    to: "concat".to_owned(),
                    from_port: Some(1),
                    to_port: Some(1),
                },
                edge("concat", "mlp"),
                edge("mlp", "gru"),
                edge("gru", "lstm"),
                edge("lstm", "rru"),
                edge("rru", "head"),
            ],
            outputs: vec![GraphOutputRef {
                node_id: "head".to_owned(),
                port: None,
            }],
            output_size: 2,
        }
    }

    fn graph() -> GraphBundle {
        GraphBundle::compile(complete_graph(), &limits()).expect("fixture graph must compile")
    }

    fn decode_seed(value: &str) -> u32 {
        u32::from_str_radix(value.trim_start_matches("0x"), 16).expect("fixture seed must parse")
    }

    fn decode_weight_bits(value: &str) -> u32 {
        u32::from_str_radix(value.trim_start_matches("0x"), 16)
            .expect("fixture Float32 bits must parse")
    }

    #[test]
    fn matches_retained_typescript_genome_random_fixture_bit_exactly() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("genome-init-reference.json");
        let fixture_text = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", fixture_path.display()));
        let fixture: Fixture =
            serde_json::from_str(&fixture_text).expect("retained TypeScript fixture must parse");
        let graph = graph();
        assert_eq!(graph.total_parameters, fixture.total_parameters);
        assert_eq!(graph.nodes.len(), fixture.compiled_node_ranges.len());
        for (node, expected) in graph.nodes.iter().zip(&fixture.compiled_node_ranges) {
            assert_eq!(node.id, expected.id);
            assert_eq!(node.parameter_offset, expected.offset);
            assert_eq!(node.parameter_length, expected.length);
        }
        let source = StatefulRng::new(f64::from(decode_seed(&fixture.seed))).export_state();
        let initialized = initialize_random_genome(
            &graph,
            &source,
            GenomeInitializationConfig {
                gru_update_bias: fixture.biases.gru_init_update_bias,
                lstm_forget_bias: fixture.biases.lstm_init_forget_bias,
                rru_gate_bias: fixture.biases.rru_init_gate_bias,
            },
        )
        .expect("Rust initialization must succeed");
        let actual_bits = initialized
            .weights()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        let expected_bits = fixture
            .weight_bits
            .iter()
            .map(|value| decode_weight_bits(value))
            .collect::<Vec<_>>();
        assert_eq!(actual_bits, expected_bits);
        assert_eq!(initialized.next_rng().state_hex, fixture.next_rng.state_hex);
        assert_eq!(
            initialized.next_rng().gaussian_spare_valid,
            fixture.next_rng.gaussian_spare_valid
        );
        assert_eq!(
            initialized.next_rng().gaussian_spare_hex,
            fixture.next_rng.gaussian_spare_hex
        );
        assert_eq!(
            source,
            StatefulRng::new(f64::from(decode_seed(&fixture.seed))).export_state()
        );
    }

    #[test]
    fn canonical_source_array_order_produces_the_same_weights_and_continuation() {
        let original = complete_graph();
        let mut reversed = original.clone();
        reversed.nodes.reverse();
        reversed.edges.reverse();
        let left = GraphBundle::compile(original, &limits()).unwrap();
        let right = GraphBundle::compile(reversed, &limits()).unwrap();
        assert_eq!(left.order, right.order);
        let source = StatefulRng::new(91.0).export_state();
        let left = initialize_random_genome(&left, &source, GenomeInitializationConfig::default())
            .unwrap();
        let right =
            initialize_random_genome(&right, &source, GenomeInitializationConfig::default())
                .unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn uniform_initialization_preserves_a_cached_gaussian_spare() {
        let mut rng = StatefulRng::new(77.0);
        let _ = rng.gaussian();
        let source = rng.export_state();
        assert!(source.gaussian_spare_valid);
        let initialized =
            initialize_random_genome(&graph(), &source, GenomeInitializationConfig::default())
                .unwrap();
        assert!(initialized.next_rng().gaussian_spare_valid);
        assert_eq!(
            initialized.next_rng().gaussian_spare_hex,
            source.gaussian_spare_hex
        );
        assert_ne!(initialized.next_rng().state_hex, source.state_hex);
        assert_eq!(source, rng.export_state());
    }

    #[test]
    fn invalid_bias_rng_or_layout_returns_no_partial_result() {
        let graph = graph();
        let source = StatefulRng::new(17.0).export_state();
        let invalid_bias = initialize_random_genome(
            &graph,
            &source,
            GenomeInitializationConfig {
                gru_update_bias: f64::NAN,
                ..GenomeInitializationConfig::default()
            },
        );
        assert!(matches!(
            invalid_bias,
            Err(GenomeInitializationError::InvalidBias {
                path: "brain.gruInitUpdateBias"
            })
        ));

        let mut invalid_rng = source.clone();
        invalid_rng.state_hex = "0x00000000".to_owned();
        assert!(matches!(
            initialize_random_genome(&graph, &invalid_rng, GenomeInitializationConfig::default()),
            Err(GenomeInitializationError::Rng(RngError::ZeroXorshiftState))
        ));

        let mut invalid_layout = graph.compiled().clone();
        invalid_layout.nodes[3].parameter_length -= 1;
        assert!(matches!(
            initialize_random_genome(
                &invalid_layout,
                &source,
                GenomeInitializationConfig::default()
            ),
            Err(GenomeInitializationError::InvalidCompiledLayout { .. })
        ));
        assert_eq!(source, StatefulRng::new(17.0).export_state());
    }
}
