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
    typescript_default_graph_spec()
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
