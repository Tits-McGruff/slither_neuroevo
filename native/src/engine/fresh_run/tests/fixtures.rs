use super::*;

/// Two GiB admits the conservative fixed P0 scratch envelope in local tests.
pub(super) const TEST_MEMORY_CEILING: usize = 2 * 1024 * MIB;
/// Selected TypeScript seed retained by the compact compatibility fixture.
pub(super) const FIXTURE_SEED: u32 = 0x1234_5678;

/// Automatically removes one process-unique managed-checkpoint test root.
pub(super) struct TestDirectory(pub(super) PathBuf);

impl TestDirectory {
    pub(super) fn create(label: &str) -> Self {
        let unique = format!(
            "slither-fresh-run-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock must follow the Unix epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir(&path).expect("fresh-run test root must be unique and creatable");
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

/// Publish, acknowledge, and activate one disposable real run-start boundary.
pub(super) fn activate_pending_run_start(
    mut pending: PendingRunStartTransition,
    label: &str,
    operation_hex: &str,
) -> PendingRunStartTransition {
    let managed = TestDirectory::create(label);
    let operation =
        CheckpointOperationId::parse(operation_hex).expect("test operation ID must be canonical");
    let descriptor = pending
        .publish_checkpoint(managed.path(), operation)
        .expect("test boundary must publish its immutable checkpoint");
    pending
        .acknowledge_persistence(&descriptor)
        .expect("exact test descriptor must acknowledge");
    pending
        .publish_running_authority()
        .expect("durable test boundary must activate");
    pending
}

/// Build a valid fixed-P0 boundary whose generation ends after eight steps.
pub(super) fn short_generation_pending(seed: u32) -> PendingRunStartTransition {
    let mut prepared = prepare_stage6a_p0_boundary(
        Stage6aP0FreshRunRequest {
            memory_ceiling_bytes: 4 * 1024 * MIB,
            ..request(seed)
        },
        &[],
        typescript_default_graph_spec(),
    )
    .expect("fixed P0 boundary must construct before test-only shortening");
    prepared.candidate.config.fixed_step_seconds = 1.0;
    let generation_seconds = prepared
        .candidate
        .config
        .settings
        .iter_mut()
        .find(|setting| setting.path == "generationSeconds")
        .expect("complete settings must contain generationSeconds");
    generation_seconds.value = NormalizedSettingValue::Float(8.0);
    prepared.candidate.identity.config_hash = normalized_config_hash(&prepared.candidate.config)
        .expect("short valid test configuration must hash");
    PendingRunStartTransition::admit(
        prepared.candidate,
        prepared.graph,
        prepared.admission_policy,
        prepared.checkpoint_limits,
        prepared.graph_limits,
        prepared.work_limits,
    )
    .expect("short valid test boundary must admit")
}

/// Queue limits for one local real-authority thread integration test.
pub(super) fn running_runtime_init() -> EngineInit {
    EngineInit {
        contract_version: ENGINE_CONTRACT_VERSION,
        inbound: InboundLimits {
            max_batches: 4,
            max_commands: 8,
            max_owned_bytes: 64,
            max_batch_commands: 4,
            max_batch_owned_bytes: 32,
        },
        output: OutputLimits {
            max_reliable: 8,
            max_reliable_owned_bytes: 4096,
            max_discrete: 4,
            max_discrete_owned_bytes: 1024,
            max_total_owned_bytes: 8192,
            max_event_owned_bytes: 1024,
            max_frame_connections: 4,
        },
    }
}

/// One exact sequenced probe for coarse queue/wake integration.
pub(super) fn runtime_probe(sequence: u64, value: u8) -> CommandBatch {
    CommandBatch {
        contract_version: ENGINE_CONTRACT_VERSION,
        commands: vec![SequencedCommand {
            sequence,
            command: EngineCommand::Probe {
                correlation_id: sequence + 100,
                payload: vec![value],
            },
        }]
        .into_boxed_slice(),
    }
}

/// Wait only on bounded scalar health; never borrow background authority.
pub(super) fn wait_for_runtime(runtime: &EngineRuntime, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "running runtime condition not reached: {:?}",
        runtime.health()
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FixtureGraph {
    #[serde(rename = "type")]
    pub(super) graph_type: String,
    pub(super) nodes: Vec<FixtureGraphNode>,
    pub(super) edges: Vec<FixtureGraphEdge>,
    pub(super) outputs: Vec<FixtureGraphOutput>,
    pub(super) output_size: usize,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum FixtureGraphNode {
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
pub(super) struct FixtureGraphEdge {
    pub(super) from: String,
    pub(super) to: String,
    pub(super) from_port: Option<i64>,
    pub(super) to_port: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FixtureGraphOutput {
    pub(super) node_id: String,
    pub(super) port: Option<i64>,
}

impl FixtureGraph {
    pub(super) fn to_graph_spec(&self) -> crate::engine::graph::GraphSpec {
        use crate::engine::graph::{
            GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
        };

        let nodes = self
            .nodes
            .iter()
            .map(|node| match node {
                FixtureGraphNode::Input { id, output_size } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Input {
                        output_size: *output_size,
                    },
                },
                FixtureGraphNode::Dense {
                    id,
                    input_size,
                    output_size,
                } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Dense {
                        input_size: *input_size,
                        output_size: *output_size,
                    },
                },
                FixtureGraphNode::Mlp {
                    id,
                    input_size,
                    hidden_sizes,
                    output_size,
                } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Mlp {
                        input_size: *input_size,
                        hidden_sizes: hidden_sizes.clone(),
                        output_size: *output_size,
                    },
                },
                FixtureGraphNode::Gru {
                    id,
                    input_size,
                    hidden_size,
                } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Gru {
                        input_size: *input_size,
                        hidden_size: *hidden_size,
                    },
                },
                FixtureGraphNode::Lstm {
                    id,
                    input_size,
                    hidden_size,
                } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Lstm {
                        input_size: *input_size,
                        hidden_size: *hidden_size,
                    },
                },
                FixtureGraphNode::Rru {
                    id,
                    input_size,
                    hidden_size,
                } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Rru {
                        input_size: *input_size,
                        hidden_size: *hidden_size,
                    },
                },
                FixtureGraphNode::Concat { id } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Concat,
                },
                FixtureGraphNode::Split { id, output_sizes } => GraphNodeSpec {
                    id: id.clone(),
                    kind: GraphNodeKind::Split {
                        output_sizes: output_sizes.clone(),
                    },
                },
            })
            .collect();
        GraphSpec {
            nodes,
            edges: self
                .edges
                .iter()
                .map(|edge| GraphEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    from_port: edge.from_port,
                    to_port: edge.to_port,
                })
                .collect(),
            outputs: self
                .outputs
                .iter()
                .map(|output| GraphOutputRef {
                    node_id: output.node_id.clone(),
                    port: output.port,
                })
                .collect(),
            output_size: self.output_size,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
pub(super) enum FixtureNormalizedSetting {
    #[serde(rename = "bool")]
    Bool { value: bool },
    #[serde(rename = "integer")]
    Integer {
        #[serde(rename = "valueDecimal")]
        value_decimal: String,
    },
    #[serde(rename = "float")]
    Float {
        #[serde(rename = "valueHex")]
        value_hex: String,
    },
}

#[derive(Deserialize)]
pub(super) struct FixtureCompiledNodeRange {
    pub(super) id: String,
    #[serde(rename = "type")]
    pub(super) node_type: String,
    pub(super) offset: usize,
    pub(super) length: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FreshRunFixture {
    pub(super) seed: String,
    pub(super) typescript_graph_key: String,
    pub(super) graph: FixtureGraph,
    pub(super) compiled_node_ranges: Vec<FixtureCompiledNodeRange>,
    pub(super) total_parameters: usize,
    pub(super) recurrent_state_floats: usize,
    pub(super) normalized_settings: BTreeMap<String, FixtureNormalizedSetting>,
    pub(super) population_count: usize,
    pub(super) baseline_count: usize,
    pub(super) genome_weight_sha256: Vec<String>,
    pub(super) population_weight_sha256: String,
    pub(super) world_state_hex: String,
    pub(super) next_evolution_state_hex: String,
    pub(super) external_controller_state_hex: String,
    pub(super) baseline_state_hex: Vec<String>,
}

pub(super) fn request(seed: u32) -> Stage6aP0FreshRunRequest {
    Stage6aP0FreshRunRequest {
        run_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(),
        seed,
        memory_ceiling_bytes: TEST_MEMORY_CEILING,
    }
}

pub(super) fn numeric_settings(
    settings: Vec<crate::engine::state::NormalizedSetting>,
) -> Vec<FreshRunSettingUpdate> {
    settings
        .into_iter()
        .map(|setting| FreshRunSettingUpdate {
            path: setting.path,
            value: match setting.value {
                NormalizedSettingValue::Bool(value) => f64::from(value),
                NormalizedSettingValue::Integer(value) => value as f64,
                NormalizedSettingValue::Float(value) => value,
                NormalizedSettingValue::Text(_) => panic!("numeric fresh setting"),
            },
        })
        .collect()
}

/// Reset carries numeric settings; Rust regenerates the structural digest.
pub(super) fn numeric_metadata_settings(metadata: &str) -> Vec<FreshRunSettingUpdate> {
    let decoded: serde_json::Value = serde_json::from_str(metadata).unwrap();
    decoded["settings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|setting| {
            let value = setting["value"]
                .as_f64()
                .or_else(|| setting["value"].as_bool().map(f64::from))?;
            Some(FreshRunSettingUpdate {
                path: setting["path"].as_str().unwrap().into(),
                value,
            })
        })
        .collect()
}

pub(super) fn fixture() -> FreshRunFixture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("fresh-run-reference.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("fresh-run TypeScript fixture must parse")
}

pub(super) fn weight_sha256(weights: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for weight in weights {
        hasher.update(weight.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub(super) fn fixture_float64_bits(value: &str) -> u64 {
    let digits = value
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("fixture Float64 lacks 0x prefix: {value}"));
    assert_eq!(digits.len(), 16, "fixture Float64 must contain 16 digits");
    u64::from_str_radix(digits, 16)
        .unwrap_or_else(|error| panic!("invalid fixture Float64 {value}: {error}"))
}

pub(super) fn typescript_graph_key(spec: &crate::engine::graph::GraphSpec) -> String {
    use crate::engine::graph::GraphNodeKind;

    let mut nodes = spec
        .nodes
        .iter()
        .map(|node| match &node.kind {
            GraphNodeKind::Input { output_size } => {
                format!("{}:Input:{output_size}", node.id)
            }
            GraphNodeKind::Dense {
                input_size,
                output_size,
            } => format!("{}:Dense:{input_size}x{output_size}", node.id),
            GraphNodeKind::Mlp {
                input_size,
                hidden_sizes,
                output_size,
            } => {
                let hidden = if hidden_sizes.is_empty() {
                    "none".to_owned()
                } else {
                    hidden_sizes
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join("x")
                };
                format!("{}:MLP:{input_size}x{hidden}x{output_size}", node.id)
            }
            GraphNodeKind::Gru {
                input_size,
                hidden_size,
            } => format!("{}:GRU:{input_size}x{hidden_size}", node.id),
            GraphNodeKind::Lstm {
                input_size,
                hidden_size,
            } => format!("{}:LSTM:{input_size}x{hidden_size}", node.id),
            GraphNodeKind::Rru {
                input_size,
                hidden_size,
            } => format!("{}:RRU:{input_size}x{hidden_size}", node.id),
            GraphNodeKind::Concat => format!("{}:Concat", node.id),
            GraphNodeKind::Split { output_sizes } => format!(
                "{}:Split:{}",
                node.id,
                output_sizes
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        })
        .collect::<Vec<_>>();
    nodes.sort_unstable();
    let mut edges = spec
        .edges
        .iter()
        .map(|edge| {
            format!(
                "{}->{}:{}:{}",
                edge.from,
                edge.to,
                edge.from_port.unwrap_or(0),
                edge.to_port.unwrap_or(0)
            )
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    let outputs = spec
        .outputs
        .iter()
        .map(|output| format!("{}:{}", output.node_id, output.port.unwrap_or(0)))
        .collect::<Vec<_>>();
    format!(
        "graph|out:{}|nodes:{}|edges:{}|outs:{}",
        spec.output_size,
        nodes.join(";"),
        edges.join(";"),
        outputs.join(";")
    )
}

pub(super) fn compiled_node_type_label(
    node_type: crate::engine::graph::CompiledNodeType,
) -> &'static str {
    use crate::engine::graph::CompiledNodeType;

    match node_type {
        CompiledNodeType::Input => "Input",
        CompiledNodeType::Dense => "Dense",
        CompiledNodeType::Mlp => "MLP",
        CompiledNodeType::Gru => "GRU",
        CompiledNodeType::Lstm => "LSTM",
        CompiledNodeType::Rru => "RRU",
        CompiledNodeType::Concat => "Concat",
        CompiledNodeType::Split => "Split",
    }
}
