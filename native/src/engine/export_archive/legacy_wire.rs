//! Bounded legacy wire decoding and graph/settings conversion.

use super::*;

/// Old browser population file decoded directly from its upload stream.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyPopulationFile {
    #[serde(default)]
    pub(super) run_id: Option<String>,
    #[serde(default)]
    pub(super) generation: Option<u64>,
    #[serde(default)]
    pub(super) arch_key: Option<String>,
    pub(super) genomes: BoundedLegacyGenomes,
    #[serde(default)]
    pub(super) world_seed: Option<u32>,
    #[serde(default)]
    pub(super) graph_spec: Option<LegacyGraphWire>,
    #[serde(default)]
    pub(super) settings: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) updates: Vec<LegacySettingWire>,
}

/// Bounded compatibility fields read from a TypeScript v2 parent row.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyDatabaseMetadata {
    pub(super) format_version: u32,
    pub(super) boundary_version: u32,
    pub(super) boundary_kind: String,
    pub(super) resumable: bool,
    pub(super) generation: u64,
    pub(super) run_id: String,
    pub(super) world_seed: u32,
    pub(super) arch_key: String,
    pub(super) graph_spec: LegacyGraphWire,
    pub(super) population_count: usize,
    pub(super) settings: serde_json::Value,
    #[serde(default)]
    pub(super) updates: Vec<LegacySettingWire>,
}

pub(super) struct BoundedLegacyGenomes(pub(super) Vec<LegacyGenomeWire>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyGenomeWire {
    pub(super) arch_key: String,
    pub(super) weights: BoundedLegacyWeights,
}

pub(super) struct BoundedLegacyWeights(pub(super) Vec<f32>);

#[derive(Deserialize)]
pub(super) struct LegacySettingWire {
    pub(super) path: String,
    pub(super) value: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyGraphWire {
    #[serde(rename = "type")]
    pub(super) graph_type: String,
    pub(super) nodes: Vec<LegacyGraphNodeWire>,
    pub(super) edges: Vec<LegacyGraphEdgeWire>,
    pub(super) outputs: Vec<LegacyGraphOutputWire>,
    pub(super) output_size: usize,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum LegacyGraphNodeWire {
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
        #[serde(rename = "hiddenSizes", default)]
        hidden_sizes: Vec<usize>,
        #[serde(rename = "outputSize")]
        output_size: usize,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyGraphEdgeWire {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) from_port: Option<i64>,
    pub(super) to_port: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyGraphOutputWire {
    pub(super) node_id: String,
    pub(super) port: Option<i64>,
}

impl<'de> Deserialize<'de> for BoundedLegacyGenomes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct GenomeVisitor;

        impl<'de> Visitor<'de> for GenomeVisitor {
            type Value = BoundedLegacyGenomes;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an array containing at most 300 legacy genomes")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut genomes = Vec::new();
                genomes
                    .try_reserve_exact(sequence.size_hint().unwrap_or(0).min(MAX_LEGACY_GENOMES))
                    .map_err(|_| de::Error::custom("legacy genome allocation failed"))?;
                while let Some(genome) = sequence.next_element()? {
                    if genomes.len() == MAX_LEGACY_GENOMES {
                        return Err(de::Error::custom("legacy population exceeds 300 genomes"));
                    }
                    genomes.push(genome);
                }
                Ok(BoundedLegacyGenomes(genomes))
            }
        }

        deserializer.deserialize_seq(GenomeVisitor)
    }
}

impl<'de> Deserialize<'de> for BoundedLegacyWeights {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct WeightVisitor;

        impl<'de> Visitor<'de> for WeightVisitor {
            type Value = BoundedLegacyWeights;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an array containing at most one million finite weights")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut weights = Vec::new();
                weights
                    .try_reserve_exact(
                        sequence
                            .size_hint()
                            .unwrap_or(0)
                            .min(MAX_LEGACY_WEIGHTS_PER_GENOME),
                    )
                    .map_err(|_| de::Error::custom("legacy weight allocation failed"))?;
                while let Some(weight) = sequence.next_element::<f32>()? {
                    if weights.len() == MAX_LEGACY_WEIGHTS_PER_GENOME {
                        return Err(de::Error::custom(
                            "legacy genome exceeds one million weights",
                        ));
                    }
                    if !weight.is_finite() {
                        return Err(de::Error::custom(
                            "legacy genome contains a non-finite weight",
                        ));
                    }
                    weights.push(weight);
                }
                if weights.is_empty() {
                    return Err(de::Error::custom("legacy genome has no weights"));
                }
                Ok(BoundedLegacyWeights(weights))
            }
        }

        deserializer.deserialize_seq(WeightVisitor)
    }
}

pub(super) fn legacy_graph_spec(wire: LegacyGraphWire) -> Result<GraphSpec, CheckpointError> {
    if wire.graph_type != "graph"
        || wire.nodes.is_empty()
        || wire.nodes.len() > 64
        || wire.edges.len() > 128
        || wire.outputs.is_empty()
        || wire.outputs.len() > 8
        || wire.nodes.iter().any(|node| {
            let (id, extra_count) = match node {
                LegacyGraphNodeWire::Input { id, .. }
                | LegacyGraphNodeWire::Dense { id, .. }
                | LegacyGraphNodeWire::Gru { id, .. }
                | LegacyGraphNodeWire::Lstm { id, .. }
                | LegacyGraphNodeWire::Rru { id, .. }
                | LegacyGraphNodeWire::Concat { id } => (id, 0),
                LegacyGraphNodeWire::Mlp {
                    id, hidden_sizes, ..
                } => (id, hidden_sizes.len()),
                LegacyGraphNodeWire::Split { id, output_sizes } => (id, output_sizes.len()),
            };
            id.is_empty() || id.len() > 64 || !id.is_ascii() || extra_count > 16
        })
        || wire.edges.iter().any(|edge| {
            edge.from.len() > 64
                || edge.to.len() > 64
                || !edge.from.is_ascii()
                || !edge.to.is_ascii()
        })
        || wire
            .outputs
            .iter()
            .any(|output| output.node_id.len() > 64 || !output.node_id.is_ascii())
    {
        return Err(CheckpointError::format(
            "LEGACY_IMPORT_GRAPH",
            "legacy graph exceeds safe TypeScript-layout compatibility limits",
        ));
    }
    let nodes = wire
        .nodes
        .into_iter()
        .map(|node| {
            let (id, kind) = match node {
                LegacyGraphNodeWire::Input { id, output_size } => {
                    (id, GraphNodeKind::Input { output_size })
                }
                LegacyGraphNodeWire::Dense {
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
                LegacyGraphNodeWire::Mlp {
                    id,
                    input_size,
                    hidden_sizes,
                    output_size,
                } => (
                    id,
                    GraphNodeKind::Mlp {
                        input_size,
                        hidden_sizes,
                        output_size,
                    },
                ),
                LegacyGraphNodeWire::Gru {
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
                LegacyGraphNodeWire::Lstm {
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
                LegacyGraphNodeWire::Rru {
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
                LegacyGraphNodeWire::Concat { id } => (id, GraphNodeKind::Concat),
                LegacyGraphNodeWire::Split { id, output_sizes } => {
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

pub(super) fn legacy_settings(
    root: Option<&serde_json::Value>,
    updates: Vec<LegacySettingWire>,
) -> Result<Vec<FreshRunSettingUpdate>, CheckpointError> {
    let defaults = typescript_default_settings(55, 10);
    let mut result = Vec::new();
    result
        .try_reserve_exact(defaults.len())
        .map_err(|_| CheckpointError::format("ALLOCATION", "legacy settings allocation failed"))?;
    if let Some(root) = root {
        for setting in &defaults {
            if let Some(value) = json_path(root, &setting.path) {
                if let Some(value) = legacy_setting_value(value, &setting.value) {
                    result.push(FreshRunSettingUpdate {
                        path: setting.path.clone(),
                        value,
                    });
                }
            }
        }
    }
    for update in updates {
        if update.path.len() > 128 {
            return Err(CheckpointError::format(
                "LEGACY_IMPORT_SETTINGS",
                "legacy setting path exceeds 128 bytes",
            ));
        }
        let Some(expected) = defaults.iter().find(|setting| setting.path == update.path) else {
            continue;
        };
        let Some(value) = legacy_setting_value(&update.value, &expected.value) else {
            return Err(CheckpointError::format(
                "LEGACY_IMPORT_SETTINGS",
                format!("legacy setting {} has the wrong type", update.path),
            ));
        };
        if let Some(existing) = result
            .iter_mut()
            .find(|setting| setting.path == update.path)
        {
            existing.value = value;
        } else {
            result.push(FreshRunSettingUpdate {
                path: update.path,
                value,
            });
        }
    }
    Ok(result)
}

pub(super) fn json_path<'a>(
    root: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(root, |value, segment| value.get(segment))
}

pub(super) fn legacy_setting_value(
    value: &serde_json::Value,
    expected: &NormalizedSettingValue,
) -> Option<f64> {
    match expected {
        NormalizedSettingValue::Bool(_) => value
            .as_bool()
            .map(u8::from)
            .or_else(|| {
                value
                    .as_u64()
                    .and_then(|number| u8::try_from(number).ok())
                    .filter(|number| *number <= 1)
            })
            .map(f64::from),
        NormalizedSettingValue::Integer(_) => {
            value.as_i64().map(|number| number as f64).or_else(|| {
                value
                    .as_u64()
                    .and_then(|number| u32::try_from(number).ok())
                    .map(f64::from)
            })
        }
        NormalizedSettingValue::Float(_) => value.as_f64().filter(|number| number.is_finite()),
        NormalizedSettingValue::Text(_) => None,
    }
}
