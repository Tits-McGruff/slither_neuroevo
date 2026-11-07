//! Graph population wire for managed checkpoints.

use super::*;

/// Population-index fields required to reconstruct the boundary candidate.
pub(super) struct DecodedPopulationIndex {
    pub(super) records: Vec<PopulationIndexRecord>,
    pub(super) weight_float_count: usize,
    pub(super) recurrent_float_count: usize,
}

/// One fixed population index record.
pub(super) struct PopulationIndexRecord {
    pub(super) slot: u32,
    pub(super) brain_index: u32,
    pub(super) brain: BrainHandle,
    pub(super) lineage: GenomeLineage,
    pub(super) fitness: f64,
    pub(super) weight_offset: usize,
    pub(super) weight_count: usize,
    pub(super) recurrent_offset: usize,
    pub(super) recurrent_count: usize,
}

/// Encode the source graph plus independently checked compiled identity.
pub(super) fn encode_graph(
    spec: &GraphSpec,
    compiled: &crate::engine::graph::CompiledGraph,
    limits: &CheckpointLimits,
) -> Result<Vec<u8>, CheckpointError> {
    let mut output = BinaryWriter::new(limits.max_graph_bytes, limits);
    output.raw(GRAPH_MAGIC)?;
    output.u32(CANONICAL_GRAPH_LAYOUT_VERSION)?;
    output.u32(usize_to_u32(spec.nodes.len(), "graph node count")?)?;
    for node in &spec.nodes {
        output.string(&node.id)?;
        encode_graph_node_kind(&mut output, &node.kind)?;
    }
    output.u32(usize_to_u32(spec.edges.len(), "graph edge count")?)?;
    for edge in &spec.edges {
        output.string(&edge.from)?;
        output.string(&edge.to)?;
        encode_optional_i64(&mut output, edge.from_port)?;
        encode_optional_i64(&mut output, edge.to_port)?;
    }
    output.u32(usize_to_u32(spec.outputs.len(), "graph output count")?)?;
    for graph_output in &spec.outputs {
        output.string(&graph_output.node_id)?;
        encode_optional_i64(&mut output, graph_output.port)?;
    }
    output.u64(usize_to_u64(spec.output_size, "graph output size")?)?;
    output.string(&compiled.architecture_key)?;
    output.raw(&compiled.layout_digest_sha256)?;
    output.u64(usize_to_u64(
        compiled.total_parameters,
        "graph parameter count",
    )?)?;
    output.u64(usize_to_u64(
        compiled.total_state_size,
        "graph state count",
    )?)?;
    Ok(output.finish())
}

/// Encode one versioned graph operation.
pub(super) fn encode_graph_node_kind(
    output: &mut BinaryWriter,
    kind: &GraphNodeKind,
) -> Result<(), CheckpointError> {
    match kind {
        GraphNodeKind::Input { output_size } => {
            output.u8(1)?;
            output.u64(usize_to_u64(*output_size, "Input output size")?)?;
        }
        GraphNodeKind::Dense {
            input_size,
            output_size,
        } => {
            output.u8(2)?;
            output.u64(usize_to_u64(*input_size, "Dense input size")?)?;
            output.u64(usize_to_u64(*output_size, "Dense output size")?)?;
        }
        GraphNodeKind::Mlp {
            input_size,
            hidden_sizes,
            output_size,
        } => {
            output.u8(3)?;
            output.u64(usize_to_u64(*input_size, "MLP input size")?)?;
            output.u32(usize_to_u32(hidden_sizes.len(), "MLP hidden count")?)?;
            for size in hidden_sizes {
                output.u64(usize_to_u64(*size, "MLP hidden size")?)?;
            }
            output.u64(usize_to_u64(*output_size, "MLP output size")?)?;
        }
        GraphNodeKind::Gru {
            input_size,
            hidden_size,
        } => {
            output.u8(4)?;
            output.u64(usize_to_u64(*input_size, "GRU input size")?)?;
            output.u64(usize_to_u64(*hidden_size, "GRU hidden size")?)?;
        }
        GraphNodeKind::Lstm {
            input_size,
            hidden_size,
        } => {
            output.u8(5)?;
            output.u64(usize_to_u64(*input_size, "LSTM input size")?)?;
            output.u64(usize_to_u64(*hidden_size, "LSTM hidden size")?)?;
        }
        GraphNodeKind::Rru {
            input_size,
            hidden_size,
        } => {
            output.u8(6)?;
            output.u64(usize_to_u64(*input_size, "RRU input size")?)?;
            output.u64(usize_to_u64(*hidden_size, "RRU hidden size")?)?;
        }
        GraphNodeKind::Concat => output.u8(7)?,
        GraphNodeKind::Split { output_sizes } => {
            output.u8(8)?;
            output.u32(usize_to_u32(output_sizes.len(), "Split output count")?)?;
            for size in output_sizes {
                output.u64(usize_to_u64(*size, "Split output size")?)?;
            }
        }
    }
    Ok(())
}

/// Encode an optional signed port without sentinel values.
pub(super) fn encode_optional_i64(
    output: &mut BinaryWriter,
    value: Option<i64>,
) -> Result<(), CheckpointError> {
    match value {
        Some(value) => {
            output.u8(1)?;
            output.i64(value)
        }
        None => output.u8(0),
    }
}

/// Decode, compile, and verify the graph role before any population allocation.
pub(super) fn decode_graph(
    bytes: &[u8],
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
) -> Result<GraphBundle, CheckpointError> {
    let mut input = BinaryReader::new(bytes, limits);
    if input.raw(GRAPH_MAGIC.len())? != GRAPH_MAGIC {
        return Err(CheckpointError::format(
            "GRAPH_MAGIC",
            "unsupported graph role",
        ));
    }
    if input.u32()? != CANONICAL_GRAPH_LAYOUT_VERSION {
        return Err(CheckpointError::format(
            "GRAPH_VERSION",
            "unsupported graph role version",
        ));
    }
    let node_count = u32_to_usize(input.u32()?, "graph node count")?;
    if node_count > graph_limits.max_nodes {
        return Err(CheckpointError::format(
            "GRAPH_LIMIT",
            "declared graph node count exceeds limit",
        ));
    }
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_count)
        .map_err(|_| CheckpointError::format("ALLOCATION", "unable to reserve graph nodes"))?;
    for _ in 0..node_count {
        nodes.push(GraphNodeSpec {
            id: input.string()?,
            kind: decode_graph_node_kind(&mut input, graph_limits)?,
        });
    }
    let edge_count = u32_to_usize(input.u32()?, "graph edge count")?;
    if edge_count > graph_limits.max_edges {
        return Err(CheckpointError::format(
            "GRAPH_LIMIT",
            "declared graph edge count exceeds limit",
        ));
    }
    let mut edges = Vec::new();
    edges
        .try_reserve_exact(edge_count)
        .map_err(|_| CheckpointError::format("ALLOCATION", "unable to reserve graph edges"))?;
    for _ in 0..edge_count {
        edges.push(GraphEdge {
            from: input.string()?,
            to: input.string()?,
            from_port: decode_optional_i64(&mut input)?,
            to_port: decode_optional_i64(&mut input)?,
        });
    }
    let output_count = u32_to_usize(input.u32()?, "graph output count")?;
    if output_count > graph_limits.max_graph_outputs {
        return Err(CheckpointError::format(
            "GRAPH_LIMIT",
            "declared graph output count exceeds limit",
        ));
    }
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(output_count)
        .map_err(|_| CheckpointError::format("ALLOCATION", "unable to reserve graph outputs"))?;
    for _ in 0..output_count {
        outputs.push(GraphOutputRef {
            node_id: input.string()?,
            port: decode_optional_i64(&mut input)?,
        });
    }
    let output_size = u64_to_usize(input.u64()?, "graph output size")?;
    let expected_architecture_key = input.string()?;
    let mut expected_layout_digest = [0u8; 32];
    expected_layout_digest.copy_from_slice(input.raw(32)?);
    let expected_parameters = u64_to_usize(input.u64()?, "graph parameter count")?;
    let expected_state = u64_to_usize(input.u64()?, "graph state count")?;
    input.finish()?;
    let bundle = GraphBundle::compile(
        GraphSpec {
            nodes,
            edges,
            outputs,
            output_size,
        },
        graph_limits,
    )
    .map_err(|error| CheckpointError::format("GRAPH_INVALID", error.to_string()))?;
    let compiled = bundle.compiled();
    if compiled.architecture_key != expected_architecture_key
        || compiled.layout_digest_sha256 != expected_layout_digest
        || compiled.total_parameters != expected_parameters
        || compiled.total_state_size != expected_state
    {
        return Err(CheckpointError::format(
            "GRAPH_IDENTITY",
            "compiled graph does not match encoded layout identity/counts",
        ));
    }
    Ok(bundle)
}

/// Decode one graph operation after its nested counts pass graph limits.
pub(super) fn decode_graph_node_kind(
    input: &mut BinaryReader<'_>,
    limits: &GraphLimits,
) -> Result<GraphNodeKind, CheckpointError> {
    let width = |input: &mut BinaryReader<'_>, label: &'static str| {
        let value = u64_to_usize(input.u64()?, label)?;
        if value > limits.max_tensor_width {
            return Err(CheckpointError::format(
                "GRAPH_LIMIT",
                format!("{label} exceeds max_tensor_width"),
            ));
        }
        Ok(value)
    };
    match input.u8()? {
        1 => Ok(GraphNodeKind::Input {
            output_size: width(input, "Input output size")?,
        }),
        2 => Ok(GraphNodeKind::Dense {
            input_size: width(input, "Dense input size")?,
            output_size: width(input, "Dense output size")?,
        }),
        3 => {
            let input_size = width(input, "MLP input size")?;
            let count = u32_to_usize(input.u32()?, "MLP hidden count")?;
            if count > limits.max_mlp_hidden_layers {
                return Err(CheckpointError::format(
                    "GRAPH_LIMIT",
                    "MLP hidden count exceeds limit",
                ));
            }
            let mut hidden_sizes = Vec::new();
            hidden_sizes.try_reserve_exact(count).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve MLP hidden sizes")
            })?;
            for _ in 0..count {
                hidden_sizes.push(width(input, "MLP hidden size")?);
            }
            Ok(GraphNodeKind::Mlp {
                input_size,
                hidden_sizes,
                output_size: width(input, "MLP output size")?,
            })
        }
        4 => Ok(GraphNodeKind::Gru {
            input_size: width(input, "GRU input size")?,
            hidden_size: width(input, "GRU hidden size")?,
        }),
        5 => Ok(GraphNodeKind::Lstm {
            input_size: width(input, "LSTM input size")?,
            hidden_size: width(input, "LSTM hidden size")?,
        }),
        6 => Ok(GraphNodeKind::Rru {
            input_size: width(input, "RRU input size")?,
            hidden_size: width(input, "RRU hidden size")?,
        }),
        7 => Ok(GraphNodeKind::Concat),
        8 => {
            let count = u32_to_usize(input.u32()?, "Split output count")?;
            if count > limits.max_split_output_ports {
                return Err(CheckpointError::format(
                    "GRAPH_LIMIT",
                    "Split output count exceeds limit",
                ));
            }
            let mut output_sizes = Vec::new();
            output_sizes.try_reserve_exact(count).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve Split output sizes")
            })?;
            for _ in 0..count {
                output_sizes.push(width(input, "Split output size")?);
            }
            Ok(GraphNodeKind::Split { output_sizes })
        }
        _ => Err(CheckpointError::format(
            "GRAPH_NODE_KIND",
            "invalid graph node tag",
        )),
    }
}

/// Decode an explicitly tagged optional port.
pub(super) fn decode_optional_i64(
    input: &mut BinaryReader<'_>,
) -> Result<Option<i64>, CheckpointError> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(input.i64()?)),
        _ => Err(CheckpointError::format(
            "GRAPH_PORT",
            "invalid optional-port tag",
        )),
    }
}

/// Encode the fixed population metadata/index records.
pub(super) fn encode_population_index(
    state: &StateCandidate,
    parameter_count: usize,
    state_count: usize,
    limits: &CheckpointLimits,
) -> Result<Vec<u8>, CheckpointError> {
    if state.population.len() > limits.max_population_count {
        return Err(CheckpointError::format(
            "POPULATION_LIMIT",
            "population exceeds checkpoint limit",
        ));
    }
    let expected_bytes = 40usize
        .checked_add(
            state
                .population
                .len()
                .checked_mul(POPULATION_INDEX_RECORD_BYTES as usize)
                .ok_or_else(|| {
                    CheckpointError::format(
                        "INDEX_OVERFLOW",
                        "population index record bytes overflow",
                    )
                })?,
        )
        .ok_or_else(|| {
            CheckpointError::format("INDEX_OVERFLOW", "population index bytes overflow")
        })?;
    if expected_bytes > limits.max_population_index_bytes {
        return Err(CheckpointError::format(
            "INDEX_LIMIT",
            "population index exceeds checkpoint limit",
        ));
    }
    let mut brain_indices = vec![usize::MAX; state.population.len()];
    for (brain_index, brain) in state.brains.iter().enumerate() {
        let BrainOwner::PopulationSlot(slot) = brain.owner else {
            return Err(CheckpointError::format(
                "DIRTY_BOUNDARY",
                "non-population brain at checkpoint boundary",
            ));
        };
        let index = slot as usize;
        let target = brain_indices.get_mut(index).ok_or_else(|| {
            CheckpointError::format("POPULATION_INDEX", "brain owner slot is outside population")
        })?;
        if *target != usize::MAX {
            return Err(CheckpointError::format(
                "POPULATION_INDEX",
                "duplicate population brain slot",
            ));
        }
        *target = brain_index;
    }
    let weight_float_count = state
        .population
        .len()
        .checked_mul(parameter_count)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "population weight count overflows")
        })?;
    let recurrent_float_count = state.brains.len().checked_mul(state_count).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "recurrent state count overflows")
    })?;
    let mut output = BinaryWriter::new(limits.max_population_index_bytes, limits);
    output.raw(POPULATION_INDEX_MAGIC)?;
    output.u32(POPULATION_INDEX_VERSION)?;
    output.u32(POPULATION_INDEX_RECORD_BYTES)?;
    output.u64(usize_to_u64(state.population.len(), "population count")?)?;
    output.u64(usize_to_u64(weight_float_count, "weight float count")?)?;
    output.u64(usize_to_u64(
        recurrent_float_count,
        "recurrent float count",
    )?)?;
    for genome in &state.population {
        let brain_index = *brain_indices.get(genome.slot as usize).ok_or_else(|| {
            CheckpointError::format("POPULATION_INDEX", "population slot lacks a brain index")
        })?;
        if brain_index == usize::MAX {
            return Err(CheckpointError::format(
                "POPULATION_INDEX",
                "population slot lacks a brain",
            ));
        }
        let brain = &state.brains[brain_index];
        let weight_offset = (genome.slot as usize)
            .checked_mul(parameter_count)
            .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "weight offset overflows"))?;
        let recurrent_offset = brain_index.checked_mul(state_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "recurrent offset overflows")
        })?;
        output.u32(genome.slot)?;
        output.u32(usize_to_u32(brain_index, "brain index")?)?;
        output.u64(genome.brain.id)?;
        output.u64(genome.brain.epoch)?;
        output.u64(genome.lineage.genome_id)?;
        output.u64(genome.lineage.birth_generation)?;
        let parent_flags = u64::from(genome.lineage.parent_a.is_some())
            | (u64::from(genome.lineage.parent_b.is_some()) << 1);
        output.u64(parent_flags)?;
        output.u64(genome.lineage.parent_a.unwrap_or(0))?;
        output.u64(genome.lineage.parent_b.unwrap_or(0))?;
        output.u64(genome.fitness.to_bits())?;
        output.u64(usize_to_u64(weight_offset, "weight offset")?)?;
        output.u64(usize_to_u64(parameter_count, "weight count")?)?;
        output.u64(usize_to_u64(recurrent_offset, "recurrent offset")?)?;
        output.u64(usize_to_u64(brain.recurrent.len(), "recurrent count")?)?;
    }
    let bytes = output.finish();
    if bytes.len() != expected_bytes {
        return Err(CheckpointError::format(
            "INDEX_LENGTH",
            "population index writer length mismatch",
        ));
    }
    Ok(bytes)
}

/// Decode and preflight every population record before numeric payload allocation.
pub(super) fn decode_population_index(
    bytes: &[u8],
    limits: &CheckpointLimits,
    parameter_count: usize,
    state_count: usize,
) -> Result<DecodedPopulationIndex, CheckpointError> {
    let mut input = BinaryReader::new(bytes, limits);
    if input.raw(POPULATION_INDEX_MAGIC.len())? != POPULATION_INDEX_MAGIC {
        return Err(CheckpointError::format(
            "INDEX_MAGIC",
            "unsupported population index",
        ));
    }
    if input.u32()? != POPULATION_INDEX_VERSION || input.u32()? != POPULATION_INDEX_RECORD_BYTES {
        return Err(CheckpointError::format(
            "INDEX_VERSION",
            "unsupported population index version/record size",
        ));
    }
    let population_count = u64_to_usize(input.u64()?, "population index count")?;
    let weight_float_count = u64_to_usize(input.u64()?, "population weight count")?;
    let recurrent_float_count = u64_to_usize(input.u64()?, "population recurrent count")?;
    if population_count > limits.max_population_count
        || weight_float_count > limits.max_weight_floats
        || recurrent_float_count > limits.max_recurrent_floats
    {
        return Err(CheckpointError::format(
            "INDEX_LIMIT",
            "population index counts exceed limits",
        ));
    }
    let expected_length = 40usize
        .checked_add(
            population_count
                .checked_mul(POPULATION_INDEX_RECORD_BYTES as usize)
                .ok_or_else(|| {
                    CheckpointError::format("INDEX_OVERFLOW", "population record bytes overflow")
                })?,
        )
        .ok_or_else(|| {
            CheckpointError::format("INDEX_OVERFLOW", "population index length overflows")
        })?;
    if bytes.len() != expected_length {
        return Err(CheckpointError::format(
            "INDEX_LENGTH",
            "population index length does not match declared count",
        ));
    }
    if weight_float_count
        != population_count
            .checked_mul(parameter_count)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "expected population weights overflow")
            })?
        || recurrent_float_count
            != population_count.checked_mul(state_count).ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "expected recurrent state overflows")
            })?
    {
        return Err(CheckpointError::format(
            "INDEX_COUNT",
            "population numeric counts disagree with graph layout",
        ));
    }
    let mut records = Vec::new();
    records.try_reserve_exact(population_count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve population records")
    })?;
    let mut seen_brains = vec![false; population_count];
    for index in 0..population_count {
        let slot = input.u32()?;
        let brain_index = input.u32()?;
        let brain_index_usize = brain_index as usize;
        if slot as usize != index
            || brain_index_usize >= population_count
            || seen_brains[brain_index_usize]
        {
            return Err(CheckpointError::format(
                "INDEX_DENSE",
                "slot or brain index is not dense and unique",
            ));
        }
        seen_brains[brain_index_usize] = true;
        let brain = BrainHandle {
            id: input.u64()?,
            epoch: input.u64()?,
        };
        let genome_id = input.u64()?;
        let birth_generation = input.u64()?;
        let parent_flags = input.u64()?;
        let parent_a_raw = input.u64()?;
        let parent_b_raw = input.u64()?;
        if parent_flags & !3 != 0
            || (parent_flags & 1 == 0) != (parent_a_raw == 0)
            || (parent_flags & 2 == 0) != (parent_b_raw == 0)
        {
            return Err(CheckpointError::format(
                "INDEX_PARENT",
                "invalid parent flags/identities",
            ));
        }
        let fitness = f64::from_bits(input.u64()?);
        let weight_offset = u64_to_usize(input.u64()?, "weight offset")?;
        let weight_count = u64_to_usize(input.u64()?, "weight count")?;
        let recurrent_offset = u64_to_usize(input.u64()?, "recurrent offset")?;
        let recurrent_count = u64_to_usize(input.u64()?, "recurrent count")?;
        let expected_weight_offset = index.checked_mul(parameter_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "expected weight offset overflows")
        })?;
        let expected_recurrent_offset =
            brain_index_usize.checked_mul(state_count).ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "expected recurrent offset overflows")
            })?;
        if weight_offset != expected_weight_offset
            || weight_count != parameter_count
            || recurrent_offset != expected_recurrent_offset
            || recurrent_count != state_count
        {
            return Err(CheckpointError::format(
                "INDEX_RANGE",
                "population numeric range is not canonical",
            ));
        }
        records.push(PopulationIndexRecord {
            slot,
            brain_index,
            brain,
            lineage: GenomeLineage {
                genome_id,
                birth_generation,
                parent_a: (parent_flags & 1 != 0).then_some(parent_a_raw),
                parent_b: (parent_flags & 2 != 0).then_some(parent_b_raw),
            },
            fitness,
            weight_offset,
            weight_count,
            recurrent_offset,
            recurrent_count,
        });
    }
    input.finish()?;
    Ok(DecodedPopulationIndex {
        records,
        weight_float_count,
        recurrent_float_count,
    })
}
