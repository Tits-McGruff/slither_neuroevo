//! Bounded shared decoding of canonical source graphs; execution stays in Rust.

use super::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec};
use serde::Deserialize;

/// Compile offline source bytes under the same limits as production fresh runs.
pub fn compile_training_graph_json(encoded: &[u8]) -> Result<super::graph::GraphBundle, String> {
    super::graph::GraphBundle::compile(
        decode_graph_json(encoded)?,
        &super::fresh_run::stage6a_p0_graph_limits(),
    )
    .map_err(|error| error.to_string())
}

/// Bounded JSON graph root shared by production admission and offline tools.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FreshRunGraphWire {
    #[serde(rename = "type")]
    graph_type: String,
    nodes: Vec<FreshRunGraphNodeWire>,
    edges: Vec<FreshRunGraphEdgeWire>,
    outputs: Vec<FreshRunGraphOutputWire>,
    output_size: usize,
}

/// One current browser graph node before conversion to compiler-owned types.
#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum FreshRunGraphNodeWire {
    Input {
        id: String,
        #[serde(rename = "outputSize")]
        output_size: usize,
    },
    Dense {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "outputSize")]
        output_size: usize,
    },
    #[serde(rename = "MLP")]
    Mlp {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "outputSize")]
        output_size: usize,
        #[serde(rename = "hiddenSizes", default)]
        hidden_sizes: Vec<usize>,
    },
    #[serde(rename = "GRU")]
    Gru {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "hiddenSize")]
        hidden_size: usize,
    },
    #[serde(rename = "LSTM")]
    Lstm {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "hiddenSize")]
        hidden_size: usize,
    },
    #[serde(rename = "RRU")]
    Rru {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "hiddenSize")]
        hidden_size: usize,
    },
    Concat {
        id: String,
    },
    Split {
        id: String,
        #[serde(rename = "outputSizes")]
        output_sizes: Vec<usize>,
    },
}

/// One browser graph edge with optional explicit zero-based ports.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FreshRunGraphEdgeWire {
    from: String,
    to: String,
    from_port: Option<i64>,
    to_port: Option<i64>,
}

/// One ordered browser graph output reference.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FreshRunGraphOutputWire {
    node_id: String,
    port: Option<i64>,
}

/// Decode one bounded graph document before the background task owns it.
pub fn decode_graph_json(encoded: &[u8]) -> Result<GraphSpec, String> {
    if encoded.is_empty() || encoded.len() > 1024 * 1024 {
        return Err("graph JSON exceeds its byte limit".into());
    }
    let wire: FreshRunGraphWire = serde_json::from_slice(encoded)
        .map_err(|_| "fresh-run graph JSON is invalid".to_owned())?;
    if wire.graph_type != "graph"
        || wire.nodes.is_empty()
        || wire.nodes.len() > 64
        || wire.edges.len() > 128
        || wire.outputs.is_empty()
        || wire.outputs.len() > 8
    {
        return Err("fresh-run graph exceeds its structural limits".into());
    }
    let nodes = wire
        .nodes
        .into_iter()
        .map(|node| {
            let (id, kind) = match node {
                FreshRunGraphNodeWire::Input { id, output_size } => {
                    (id, GraphNodeKind::Input { output_size })
                }
                FreshRunGraphNodeWire::Dense {
                    id,
                    input_size,
                    output_size,
                } => (
                    id,
                    GraphNodeKind::Dense {
                        input_size,
                        output_size,
                    },
                ),
                FreshRunGraphNodeWire::Mlp {
                    id,
                    input_size,
                    output_size,
                    hidden_sizes,
                } => (
                    id,
                    GraphNodeKind::Mlp {
                        input_size,
                        hidden_sizes,
                        output_size,
                    },
                ),
                FreshRunGraphNodeWire::Gru {
                    id,
                    input_size,
                    hidden_size,
                } => (
                    id,
                    GraphNodeKind::Gru {
                        input_size,
                        hidden_size,
                    },
                ),
                FreshRunGraphNodeWire::Lstm {
                    id,
                    input_size,
                    hidden_size,
                } => (
                    id,
                    GraphNodeKind::Lstm {
                        input_size,
                        hidden_size,
                    },
                ),
                FreshRunGraphNodeWire::Rru {
                    id,
                    input_size,
                    hidden_size,
                } => (
                    id,
                    GraphNodeKind::Rru {
                        input_size,
                        hidden_size,
                    },
                ),
                FreshRunGraphNodeWire::Concat { id } => (id, GraphNodeKind::Concat),
                FreshRunGraphNodeWire::Split { id, output_sizes } => {
                    (id, GraphNodeKind::Split { output_sizes })
                }
            };
            GraphNodeSpec { id, kind }
        })
        .collect();
    Ok(GraphSpec {
        nodes,
        edges: wire
            .edges
            .into_iter()
            .map(|edge| GraphEdge {
                from: edge.from,
                to: edge.to,
                from_port: edge.from_port,
                to_port: edge.to_port,
            })
            .collect(),
        outputs: wire
            .outputs
            .into_iter()
            .map(|output| GraphOutputRef {
                node_id: output.node_id,
                port: output.port,
            })
            .collect(),
        output_size: wire.output_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_decoder_roundtrip_and_strict_admission() {
        let spec = super::super::graph::typescript_default_graph_spec();
        let bytes =
            serde_json::to_vec(&super::super::run_start::graph_spec_metadata(&spec)).unwrap();
        assert_eq!(decode_graph_json(&bytes).unwrap(), spec);
        let bundle = compile_training_graph_json(&bytes).unwrap();
        assert_eq!(bundle.spec(), &spec);
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["nodes"][0]["extra"] = serde_json::json!(true);
        assert!(decode_graph_json(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(decode_graph_json(&[]).is_err());
        assert!(decode_graph_json(&vec![b' '; 1024 * 1024 + 1]).is_err());
        value = serde_json::from_slice(&bytes).unwrap();
        value["nodes"][0]["outputSize"] = serde_json::json!(513);
        assert!(compile_training_graph_json(&serde_json::to_vec(&value).unwrap()).is_err());
        value["nodes"] = serde_json::json!([]);
        assert!(decode_graph_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
