//! Memory budget for authoritative state.

use super::*;

/// Checked `count * item_size` helper used before allocation/admission.
pub fn checked_allocation_bytes(
    count: usize,
    item_size: usize,
    context: &'static str,
) -> Result<usize, StateError> {
    count
        .checked_mul(item_size)
        .ok_or(StateError::ArithmeticOverflow { context })
}

/// Minimum current-plus-successor numeric peak, checked before any output weights exist.
pub(crate) fn generation_numeric_peak_bytes(
    current_bytes: usize,
    population: usize,
    graph: &CompiledGraph,
) -> Result<usize, StateError> {
    let scalars = graph
        .total_parameters
        .checked_add(graph.total_state_size)
        .and_then(|per_genome| per_genome.checked_mul(population))
        .ok_or(StateError::ArithmeticOverflow {
            context: "next generation numeric shape",
        })?;
    checked_add(
        current_bytes,
        checked_allocation_bytes(scalars, size_of::<f32>(), "next generation numeric bytes")?,
        "current plus next generation numeric memory",
    )
}

/// Estimate complete candidate and compiled-graph owned memory without
/// publishing authority.
pub fn estimate_state_memory(
    candidate: &StateCandidate,
    graph: &GraphBundle,
) -> Result<StateMemoryEstimate, StateError> {
    let compiled = graph.compiled();
    let mut estimate = StateMemoryEstimate {
        structural_bytes: checked_add(
            size_of::<Mutex<AuthoritativeState>>(),
            ARC_COUNTER_BYTES,
            "authoritative runtime state wrapper",
        )?,
        ..StateMemoryEstimate::default()
    };

    // Charge the larger of already-owned capacity and declared peak capacity.
    // This prevents an empty generation-boundary world from passing admission
    // while declaring an unsafe later body/pellet/world allocation.
    add_structural::<NormalizedSetting>(&mut estimate, candidate.config.settings.capacity())?;
    add_structural::<PopulationGenome>(&mut estimate, candidate.population.capacity())?;
    let maximum_brains = candidate
        .config
        .population_count
        .checked_add(candidate.config.max_non_population_brains)
        .ok_or(StateError::ArithmeticOverflow {
            context: "maximum brain records",
        })?;
    add_structural::<BrainRuntimeState>(
        &mut estimate,
        candidate.brains.capacity().max(maximum_brains),
    )?;
    // Conservatively reserve one Arc allocation per possible retained owner,
    // including its reference counters; shared copies may only reduce this bound.
    estimate.structural_bytes = checked_add(
        estimate.structural_bytes,
        checked_allocation_bytes(
            candidate.config.max_non_population_brains,
            size_of::<RetainedNeuralOrigin>() + ARC_COUNTER_BYTES,
            "retained brain origins",
        )?,
        "retained origin memory",
    )?;
    add_structural::<BaselineRngState>(&mut estimate, candidate.rng.baselines.capacity())?;
    add_structural::<crate::engine::rng::BaselineBehaviorRngState>(
        &mut estimate,
        candidate.rng.baseline_behaviors.capacity().max(
            if candidate.rng.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION {
                candidate.config.baseline_count
            } else {
                0
            },
        ),
    )?;
    add_structural::<EvolvedRespawnRngState>(
        &mut estimate,
        candidate.rng.evolved_respawns.capacity().max(
            if candidate.rng.version >= RNG_BUNDLE_WITH_EVOLVED_VERSION {
                candidate.config.population_count
            } else {
                0
            },
        ),
    )?;
    add_structural::<crate::engine::evolved_lives::EvolvedSlotRuntime>(
        &mut estimate,
        candidate
            .fixed_step
            .evolved_generation
            .slots
            .capacity()
            .max(candidate.config.population_count),
    )?;
    add_structural::<crate::engine::evolved_lives::EvolvedLifeEndRecord>(
        &mut estimate,
        candidate
            .fixed_step
            .evolved_generation
            .transaction_life_ends
            .capacity()
            .max(candidate.config.population_count),
    )?;
    add_structural::<crate::engine::evolved_lives::EvolvedLifeEndRecord>(
        &mut estimate,
        candidate
            .fixed_step
            .evolved_generation
            .recent_life_ends
            .capacity()
            .max(crate::engine::evolved_lives::MAX_RECENT_LIFE_ENDS),
    )?;
    add_structural::<BaselineSlotRuntime>(
        &mut estimate,
        candidate
            .fixed_step
            .baseline_lifecycle
            .slots
            .capacity()
            .max(candidate.config.baseline_count),
    )?;
    add_structural::<SnakeState>(
        &mut estimate,
        candidate
            .world
            .snakes
            .capacity()
            .max(candidate.config.max_world_snakes),
    )?;
    add_structural::<WorldPoint>(
        &mut estimate,
        candidate
            .world
            .body_points
            .capacity()
            .max(candidate.config.max_body_points),
    )?;
    add_structural::<PelletState>(
        &mut estimate,
        candidate
            .world
            .pellets
            .capacity()
            .max(candidate.config.max_pellets),
    )?;
    add_structural::<ControllerLease>(
        &mut estimate,
        candidate
            .world
            .controller_leases
            .capacity()
            .max(candidate.config.max_world_snakes),
    )?;

    for genome in &candidate.population {
        estimate.weight_bytes = checked_add(
            estimate.weight_bytes,
            checked_allocation_bytes(genome.weights.len(), size_of::<f32>(), "genome weights")?,
            "weight memory",
        )?;
    }
    let mut non_population_weight_bytes = 0usize;
    let mut current_recurrent_bytes = 0usize;
    for brain in &candidate.brains {
        if let Some(weights) = &brain.non_population_weights {
            non_population_weight_bytes = checked_add(
                non_population_weight_bytes,
                checked_allocation_bytes(
                    weights.len(),
                    size_of::<f32>(),
                    "non-population brain weights",
                )?,
                "weight memory",
            )?;
        }
        current_recurrent_bytes = checked_add(
            current_recurrent_bytes,
            checked_allocation_bytes(brain.recurrent.len(), size_of::<f32>(), "recurrent state")?,
            "recurrent memory",
        )?;
    }
    let requested_non_population_weights = checked_allocation_bytes(
        candidate.config.max_non_population_brains,
        checked_allocation_bytes(
            compiled.total_parameters,
            size_of::<f32>(),
            "one non-population weight block",
        )?,
        "maximum non-population weights",
    )?;
    estimate.weight_bytes = checked_add(
        estimate.weight_bytes,
        non_population_weight_bytes.max(requested_non_population_weights),
        "weight memory",
    )?;
    let requested_recurrent_bytes = checked_allocation_bytes(
        maximum_brains,
        checked_allocation_bytes(
            compiled.total_state_size,
            size_of::<f32>(),
            "one recurrent-state block",
        )?,
        "maximum recurrent state",
    )?;
    estimate.recurrent_bytes = current_recurrent_bytes.max(requested_recurrent_bytes);

    add_candidate_text(&mut estimate, candidate)?;
    estimate.graph_bytes = estimate_graph_memory(graph)?;
    estimate.frame_bytes = estimate_frame_v1_bytes(&candidate.config)?;
    estimate.spatial_bytes = candidate.config.spatial_index_bytes;
    estimate.scratch_bytes = checked_add(
        candidate.config.worker_scratch_bytes,
        candidate.config.checkpoint_scratch_bytes,
        "declared engine scratch",
    )?;
    if let Some(backend) =
        crate::engine::inference::InferenceMathBackend::from_label(&candidate.identity.math_backend)
            .filter(|backend| backend.uses_i16_cache())
    {
        let cache_owners = candidate
            .config
            .population_count
            .checked_add(candidate.config.max_non_population_brains)
            .ok_or(StateError::ArithmeticOverflow {
                context: "i16 cache owners",
            })?;
        let cache_bytes =
            if backend == crate::engine::inference::InferenceMathBackend::I16MatrixFma128 {
                crate::engine::inference::matrix_i16_cache_admission_bytes(compiled, cache_owners)
            } else {
                crate::engine::inference::i16_cache_admission_bytes(compiled, cache_owners, backend)
            }
            .ok_or(StateError::ArithmeticOverflow {
                context: "i16 inference cache",
            })?;
        estimate.scratch_bytes =
            checked_add(estimate.scratch_bytes, cache_bytes, "i16 inference cache")?;
    }
    // The species norm cache extends the previous slot/count summary scratch.
    estimate.scratch_bytes = checked_add(
        estimate.scratch_bytes,
        checked_allocation_bytes(
            candidate.config.population_count,
            crate::engine::evolution::SPECIES_NORM_BYTES_PER_GENOME,
            "species norm scratch",
        )?,
        "species norm scratch",
    )?;
    estimate.validation_bytes = estimate_validation_memory(candidate)?;
    estimate.total_bytes = checked_sum(&[
        estimate.structural_bytes,
        estimate.weight_bytes,
        estimate.recurrent_bytes,
        estimate.graph_bytes,
        estimate.text_bytes,
        estimate.frame_bytes,
        estimate.spatial_bytes,
        estimate.scratch_bytes,
        estimate.validation_bytes,
    ])?;
    Ok(estimate)
}

/// Preflight the final generation-boundary allocation from decoded metadata and
/// declared numeric counts before a checkpoint decoder allocates packed Float32 buffers.
///
/// `candidate_shell` must contain the decoded identity/configuration/RNG/allocator,
/// population metadata, and population brain records, but its weight and recurrent
/// boxes may remain empty. The returned estimate charges the exact population
/// weight count plus the existing conservative configured recurrent/non-population
/// peaks used by ordinary state admission.
pub fn preflight_generation_boundary_allocation(
    candidate_shell: &StateCandidate,
    graph: &GraphBundle,
    declared_weight_floats: usize,
    declared_recurrent_floats: usize,
    policy: &StateAdmissionPolicy,
) -> Result<StateMemoryEstimate, StateError> {
    validate_policy(policy)?;
    validate_admission_header(candidate_shell, graph.compiled(), policy)?;
    validate_generation(&candidate_shell.generation)?;
    validate_rng_bundle(&candidate_shell.rng, &candidate_shell.config)?;
    validate_allocators(&candidate_shell.allocators)?;
    validate_generation_boundary(candidate_shell, graph.compiled())?;
    validate_fixed_step_continuation(candidate_shell)?;
    if candidate_shell.population.len() != candidate_shell.config.population_count
        || candidate_shell.brains.len() != candidate_shell.population.len()
    {
        return invalid(
            "population",
            "checkpoint allocation shell must contain one population brain per configured slot",
        );
    }
    if candidate_shell
        .population
        .iter()
        .any(|genome| !genome.weights.is_empty())
        || candidate_shell.brains.iter().any(|brain| {
            brain.non_population_weights.is_some()
                || brain.retained_origin.is_some()
                || !brain.recurrent.is_empty()
        })
    {
        return invalid(
            "population",
            "checkpoint allocation shell must not allocate numeric payloads before preflight",
        );
    }
    let expected_weights = candidate_shell
        .population
        .len()
        .checked_mul(graph.compiled().total_parameters)
        .ok_or(StateError::ArithmeticOverflow {
            context: "declared checkpoint population weights",
        })?;
    let expected_recurrent = candidate_shell
        .brains
        .len()
        .checked_mul(graph.compiled().total_state_size)
        .ok_or(StateError::ArithmeticOverflow {
            context: "declared checkpoint recurrent state",
        })?;
    if declared_weight_floats != expected_weights || declared_recurrent_floats != expected_recurrent
    {
        return invalid(
            "population",
            "declared checkpoint numeric counts disagree with graph/population shape",
        );
    }
    let mut estimate = estimate_state_memory(candidate_shell, graph)?;
    let population_weight_bytes = checked_allocation_bytes(
        declared_weight_floats,
        size_of::<f32>(),
        "declared checkpoint population weight bytes",
    )?;
    estimate.weight_bytes = checked_add(
        estimate.weight_bytes,
        population_weight_bytes,
        "checkpoint preflight weight memory",
    )?;
    estimate.total_bytes = checked_add(
        estimate.total_bytes,
        population_weight_bytes,
        "checkpoint preflight total memory",
    )?;
    if estimate.total_bytes > policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: estimate.total_bytes,
            ceiling_bytes: policy.memory_ceiling_bytes,
        });
    }
    Ok(estimate)
}

pub(super) fn estimate_frame_v1_bytes(
    config: &NormalizedEngineConfig,
) -> Result<usize, StateError> {
    let snake_headers =
        checked_allocation_bytes(config.max_world_snakes, 8, "frame snake headers")?;
    let body_coordinates =
        checked_allocation_bytes(config.max_body_points, 2, "frame body coordinates")?;
    let pellet_fields = checked_allocation_bytes(config.max_pellets, 5, "frame pellet fields")?;
    let float_count = checked_sum(&[7, snake_headers, body_coordinates, 1, pellet_fields])?;
    checked_allocation_bytes(float_count, size_of::<f32>(), "frame-v1 bytes")
}

pub(super) fn estimate_validation_memory(candidate: &StateCandidate) -> Result<usize, StateError> {
    // A conservative BTree allocation allowance. Exact allocator overhead is
    // implementation-specific, so admission uses a deliberately padded node.
    const TREE_NODE_ALLOWANCE: usize = 96;
    let population_nodes =
        candidate
            .config
            .population_count
            .checked_mul(2)
            .ok_or(StateError::ArithmeticOverflow {
                context: "population validation nodes",
            })?;
    let brain_nodes = candidate
        .config
        .population_count
        .checked_add(candidate.config.max_non_population_brains)
        .ok_or(StateError::ArithmeticOverflow {
            context: "brain validation nodes",
        })?;
    let snake_nodes = candidate.config.max_world_snakes.checked_mul(10).ok_or(
        StateError::ArithmeticOverflow {
            context: "world validation nodes",
        },
    )?;
    let nodes = checked_sum(&[
        population_nodes,
        brain_nodes,
        snake_nodes,
        candidate.config.max_pellets,
        candidate.config.baseline_count,
    ])?;
    let trees = checked_allocation_bytes(nodes, TREE_NODE_ALLOWANCE, "validation trees")?;
    let ranges = checked_allocation_bytes(
        candidate.config.max_world_snakes,
        size_of::<(usize, usize, u64)>(),
        "validation body ranges",
    )?;
    checked_add(trees, ranges, "validation scratch")
}

pub(super) fn add_structural<T>(
    estimate: &mut StateMemoryEstimate,
    count: usize,
) -> Result<(), StateError> {
    estimate.structural_bytes = checked_add(
        estimate.structural_bytes,
        checked_allocation_bytes(count, size_of::<T>(), "state vector storage")?,
        "structural memory",
    )?;
    Ok(())
}

pub(super) fn add_text(
    estimate: &mut StateMemoryEstimate,
    value: &String,
) -> Result<(), StateError> {
    estimate.text_bytes = checked_add(estimate.text_bytes, value.capacity(), "text memory")?;
    Ok(())
}

pub(super) fn add_rng_text(
    estimate: &mut StateMemoryEstimate,
    state: &SerializedRngState,
) -> Result<(), StateError> {
    add_text(estimate, &state.algorithm)?;
    add_text(estimate, &state.state_hex)?;
    add_text(estimate, &state.gaussian_algorithm)?;
    if let Some(spare) = &state.gaussian_spare_hex {
        add_text(estimate, spare)?;
    }
    Ok(())
}

pub(super) fn add_candidate_text(
    estimate: &mut StateMemoryEstimate,
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    add_text(estimate, &candidate.identity.run_id)?;
    add_text(estimate, &candidate.identity.config_hash)?;
    add_text(estimate, &candidate.identity.source_revision)?;
    add_text(estimate, &candidate.identity.engine_build_id)?;
    add_text(estimate, &candidate.identity.source_sha256)?;
    add_text(estimate, &candidate.identity.target_triple)?;
    add_text(estimate, &candidate.identity.build_profile)?;
    add_text(estimate, &candidate.identity.build_class)?;
    add_text(estimate, &candidate.identity.rustc_version)?;
    add_text(estimate, &candidate.identity.build_contract_sha256)?;
    add_text(estimate, &candidate.identity.math_backend)?;
    if let Some(origin) = &candidate.identity.legacy_conversion {
        for text in [
            &origin.source_run_id,
            &origin.source_generation,
            &origin.source_sha256,
        ]
        .into_iter()
        .flatten()
        {
            add_text(estimate, text)?;
        }
    }
    for setting in &candidate.config.settings {
        add_text(estimate, &setting.path)?;
        if let NormalizedSettingValue::Text(value) = &setting.value {
            add_text(estimate, value)?;
        }
    }
    add_text(estimate, &candidate.config.settings_schema_sha256)?;
    add_text(estimate, &candidate.config.graph_architecture_key)?;
    add_rng_text(estimate, &candidate.rng.world)?;
    add_rng_text(estimate, &candidate.rng.evolution)?;
    add_rng_text(estimate, &candidate.rng.external_controller)?;
    for baseline in &candidate.rng.baselines {
        add_rng_text(estimate, &baseline.state)?;
    }
    for lease in &candidate.world.controller_leases {
        add_text(estimate, &lease.scope)?;
        add_text(estimate, &lease.identity_key)?;
        add_text(estimate, &lease.resume_token)?;
    }
    Ok(())
}

pub(crate) fn estimate_graph_memory(bundle: &GraphBundle) -> Result<usize, StateError> {
    let spec = bundle.spec();
    let graph = bundle.compiled();
    // The bundle stores both structs inline. Charge each source-spec dynamic
    // allocation as well as the compiled metadata previously retained alone.
    let mut bytes = checked_add(
        size_of::<GraphBundle>(),
        ARC_COUNTER_BYTES,
        "graph Arc control words",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            spec.nodes.capacity(),
            size_of::<GraphNodeSpec>(),
            "source graph nodes",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            spec.edges.capacity(),
            size_of::<GraphEdge>(),
            "source graph edges",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            spec.outputs.capacity(),
            size_of::<GraphOutputRef>(),
            "source graph outputs",
        )?,
        "graph memory",
    )?;
    for node in &spec.nodes {
        bytes = checked_add(bytes, node.id.capacity(), "graph memory")?;
        match &node.kind {
            GraphNodeKind::Mlp { hidden_sizes, .. } => {
                bytes = checked_add(
                    bytes,
                    checked_allocation_bytes(
                        hidden_sizes.capacity(),
                        size_of::<usize>(),
                        "source graph MLP hidden sizes",
                    )?,
                    "graph memory",
                )?;
            }
            GraphNodeKind::Split { output_sizes } => {
                bytes = checked_add(
                    bytes,
                    checked_allocation_bytes(
                        output_sizes.capacity(),
                        size_of::<usize>(),
                        "source graph Split output sizes",
                    )?,
                    "graph memory",
                )?;
            }
            GraphNodeKind::Input { .. }
            | GraphNodeKind::Dense { .. }
            | GraphNodeKind::Gru { .. }
            | GraphNodeKind::Lstm { .. }
            | GraphNodeKind::Rru { .. }
            | GraphNodeKind::Concat => {}
        }
    }
    for edge in &spec.edges {
        bytes = checked_add(bytes, edge.from.capacity(), "graph memory")?;
        bytes = checked_add(bytes, edge.to.capacity(), "graph memory")?;
    }
    for output in &spec.outputs {
        bytes = checked_add(bytes, output.node_id.capacity(), "graph memory")?;
    }
    bytes = checked_add(bytes, graph.architecture_key.capacity(), "graph memory")?;
    bytes = checked_add(
        bytes,
        graph.canonical_layout_bytes.capacity(),
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            graph.nodes.capacity(),
            size_of::<CompiledNode>(),
            "graph nodes",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(graph.order.capacity(), size_of::<String>(), "graph order")?,
        "graph memory",
    )?;
    for node in &graph.nodes {
        bytes = checked_add(bytes, node.id.capacity(), "graph memory")?;
        bytes = checked_add(
            bytes,
            checked_allocation_bytes(
                node.output_sizes.capacity(),
                size_of::<usize>(),
                "graph ports",
            )?,
            "graph memory",
        )?;
        bytes = checked_add(
            bytes,
            checked_allocation_bytes(
                node.hidden_sizes.capacity(),
                size_of::<usize>(),
                "graph hidden sizes",
            )?,
            "graph memory",
        )?;
        bytes = checked_add(
            bytes,
            checked_allocation_bytes(
                node.inputs.capacity(),
                size_of::<crate::engine::graph::CompiledInputRef>(),
                "graph inputs",
            )?,
            "graph memory",
        )?;
        for input in &node.inputs {
            bytes = checked_add(bytes, input.from_id.capacity(), "graph memory")?;
        }
    }
    for id in &graph.order {
        bytes = checked_add(bytes, id.capacity(), "graph memory")?;
    }
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            graph.recurrent_nodes.capacity(),
            size_of::<crate::engine::graph::RecurrentNodeInfo>(),
            "graph recurrent metadata",
        )?,
        "graph memory",
    )?;
    bytes = checked_add(
        bytes,
        checked_allocation_bytes(
            graph.outputs.capacity(),
            size_of::<crate::engine::graph::GraphOutputRef>(),
            "graph outputs",
        )?,
        "graph memory",
    )?;
    for output in &graph.outputs {
        bytes = checked_add(bytes, output.node_id.capacity(), "graph memory")?;
    }
    Ok(bytes)
}
