//! Managed restore for managed checkpoints.

use super::*;

/// Restore only the immutable content selected by the metadata worker. Historical
/// publication tokens remain correlation facts; the restored authority receives
/// its own process-local world epoch. Nothing is published or rewritten here.
pub fn restore_committed_checkpoint(
    managed_directory: &Path,
    expected: &CheckpointDescriptor,
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<RestoredCheckpoint, CheckpointError> {
    parse_digest(&expected.logical_root_sha256, "expected logical root")?;
    let epoch = parse_u64_hex(&expected.transition_epoch_hex, "publication epoch")?;
    if epoch == 0
        || expected.managed_root != "checkpoint-v3"
        || expected.relative_filename != format!("{}.checkpoint-v3", expected.logical_root_sha256)
    {
        return Err(CheckpointError::format(
            "RESTORE_DESCRIPTOR",
            "expected checkpoint has invalid publication identity or managed filename",
        ));
    }
    let path = managed_directory
        .canonicalize()?
        .join(&expected.relative_filename);
    let restored = restore_checkpoint(&path, limits, graph_limits, admission_policy)?;
    let actual = publication_descriptor(
        restored.content.clone(),
        expected.operation_id.clone(),
        epoch,
    );
    if let Some(field) = expected.first_mismatch(&actual) {
        return Err(CheckpointError::format(
            "RESTORE_DESCRIPTOR",
            format!("restored checkpoint differs from committed {field}"),
        ));
    }
    Ok(restored)
}

/// Revalidate and expose the five direct checkpoint role extents for a flat save archive.
pub(in crate::engine) fn validated_checkpoint_archive_layout(
    path: &Path,
    limits: &CheckpointLimits,
) -> Result<CheckpointArchiveLayout, CheckpointError> {
    validate_limits(limits)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.len() > limits.max_archive_bytes
    {
        return Err(CheckpointError::format(
            "MANAGED_FILE_TYPE",
            "checkpoint export source must be one bounded regular file",
        ));
    }
    let mut file = File::open(path)?;
    let entries = scan_strict_ustar(&mut file, metadata.len(), limits)?;
    let manifest_entry = entries
        .last()
        .ok_or_else(|| CheckpointError::format("USTAR_EMPTY", "archive has no entries"))?;
    let manifest_bytes = read_entry_bytes(&mut file, manifest_entry, limits.max_manifest_bytes)?;
    let manifest: CheckpointManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            CheckpointError::format(
                "MANIFEST_JSON",
                format!("invalid bounded manifest JSON: {error}"),
            )
        })?;
    validate_manifest(&manifest, &entries, limits)?;
    let mut roles = Vec::new();
    roles.try_reserve_exact(LOGICAL_ROLE_COUNT).map_err(|_| {
        CheckpointError::format(
            "ALLOCATION",
            "unable to reserve checkpoint export role table",
        )
    })?;
    for (entry, role) in entries[..LOGICAL_ROLE_COUNT]
        .iter()
        .zip(manifest.roles.iter())
    {
        roles.push(CheckpointArchiveRole {
            manifest: role.clone(),
            data_offset: entry.data_offset,
            stored_bytes: entry.size,
        });
    }
    Ok(CheckpointArchiveLayout { manifest, roles })
}

/// Strictly decode, verify, compile, and admit one managed checkpoint.
pub fn restore_checkpoint(
    path: &Path,
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<RestoredCheckpoint, CheckpointError> {
    let _phase = ArchivePhaseScope::enter(ArchivePhase::CheckpointRestore);
    validate_limits(limits)?;
    let minimum_workspace = checkpoint_workspace_bound(limits, 0)?;
    if minimum_workspace > admission_policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: minimum_workspace,
            ceiling_bytes: admission_policy.memory_ceiling_bytes,
        }
        .into());
    }
    let link_metadata = fs::symlink_metadata(path)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return Err(CheckpointError::format(
            "MANAGED_FILE_TYPE",
            "checkpoint target must be a direct regular file, not a symlink or special file",
        ));
    }
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(CheckpointError::format(
            "MANAGED_FILE_TYPE",
            "opened checkpoint handle is not a regular file",
        ));
    }
    let archive_length = file.metadata()?.len();
    if archive_length == 0 || archive_length > limits.max_archive_bytes {
        return Err(CheckpointError::format(
            "ARCHIVE_LIMIT",
            format!(
                "archive length {archive_length} is zero or exceeds limit {}",
                limits.max_archive_bytes
            ),
        ));
    }
    let entries = scan_strict_ustar(&mut file, archive_length, limits)?;
    let manifest_entry = entries
        .last()
        .ok_or_else(|| CheckpointError::format("USTAR_EMPTY", "archive has no entries"))?;
    let manifest_bytes = read_entry_bytes(&mut file, manifest_entry, limits.max_manifest_bytes)?;
    let manifest: CheckpointManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            CheckpointError::format(
                "MANIFEST_JSON",
                format!("invalid bounded manifest JSON: {error}"),
            )
        })?;
    drop(manifest_bytes);
    let validated = validate_manifest(&manifest, &entries, limits)?;
    let predecode_workspace = checkpoint_workspace_bound(limits, validated.population_count)?;
    if predecode_workspace > admission_policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: predecode_workspace,
            ceiling_bytes: admission_policy.memory_ceiling_bytes,
        }
        .into());
    }

    let state_bytes = read_and_verify_small_role(
        &mut file,
        &entries[0],
        &validated.roles[0],
        limits.max_state_bytes,
    )?;
    let graph_bytes = read_and_verify_small_role(
        &mut file,
        &entries[1],
        &validated.roles[1],
        limits.max_graph_bytes,
    )?;
    let index_bytes = read_and_verify_small_role(
        &mut file,
        &entries[2],
        &validated.roles[2],
        limits.max_population_index_bytes,
    )?;
    let state_parts = decode_state(&state_bytes, limits)?;
    let graph = Arc::new(decode_graph(&graph_bytes, limits, graph_limits)?);
    let index = decode_population_index(
        &index_bytes,
        limits,
        graph.total_parameters,
        graph.total_state_size,
    )?;
    drop(state_bytes);
    drop(graph_bytes);
    drop(index_bytes);
    if index.records.len() != validated.population_count
        || index.weight_float_count != validated.weight_float_count
        || index.recurrent_float_count != validated.recurrent_float_count
    {
        return Err(CheckpointError::format(
            "MANIFEST_COUNT_MISMATCH",
            "manifest and population-index counts disagree",
        ));
    }
    let allocation_shell = assemble_allocation_shell(&state_parts, &index)?;
    let allocation_estimate = preflight_generation_boundary_allocation(
        &allocation_shell,
        &graph,
        validated.weight_float_count,
        validated.recurrent_float_count,
        admission_policy,
    )?;
    let checkpoint_workspace = checkpoint_workspace_bound(limits, index.records.len())?;
    enforce_checkpoint_peak(
        allocation_estimate.total_bytes,
        state_parts.config.checkpoint_scratch_bytes,
        checkpoint_workspace,
        admission_policy,
    )?;
    drop(allocation_shell);
    let weights = decode_numeric_role(
        &mut file,
        &entries[3],
        validated.weights_encoding,
        validated.weight_float_count,
        index.records.len(),
        graph.total_parameters,
        validated.roles[3].logical_sha256,
    )?;
    let recurrent = decode_numeric_role(
        &mut file,
        &entries[4],
        validated.recurrent_encoding,
        validated.recurrent_float_count,
        index.records.len(),
        graph.total_state_size,
        validated.roles[4].logical_sha256,
    )?;
    let candidate = assemble_candidate(state_parts, &index, weights, recurrent)?;
    let state =
        AuthoritativeState::validate_and_own(candidate, Arc::clone(&graph), admission_policy)?;
    crosscheck_manifest_state(&manifest, &validated, state.state(), state.graph())?;
    let content = content_descriptor_from_manifest(
        &manifest,
        archive_length,
        path.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                CheckpointError::format("MANAGED_FILENAME", "checkpoint filename is not UTF-8")
            })?
            .to_owned(),
    )?;
    Ok(RestoredCheckpoint { content, state })
}

/// Reconstruct the exact boundary candidate without a transient full flattened copy.
pub(super) fn assemble_candidate(
    state: DecodedStateParts,
    index: &DecodedPopulationIndex,
    weights: Vec<Box<[f32]>>,
    recurrent: Vec<Box<[f32]>>,
) -> Result<StateCandidate, CheckpointError> {
    if weights.len() != index.records.len() || recurrent.len() != index.records.len() {
        return Err(CheckpointError::format(
            "NUMERIC_SEGMENTS",
            "decoded numeric segment counts disagree with index",
        ));
    }
    let mut weights: Vec<Option<Box<[f32]>>> = weights.into_iter().map(Some).collect();
    let mut recurrent: Vec<Option<Box<[f32]>>> = recurrent.into_iter().map(Some).collect();
    let mut population = Vec::new();
    population
        .try_reserve_exact(index.records.len())
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve admitted population")
        })?;
    let mut brains: Vec<Option<BrainRuntimeState>> =
        (0..index.records.len()).map(|_| None).collect();
    for record in &index.records {
        let slot = record.slot as usize;
        let brain_index = record.brain_index as usize;
        let genome_weights = weights
            .get_mut(slot)
            .and_then(Option::take)
            .ok_or_else(|| {
                CheckpointError::format("INDEX_RANGE", "weight segment reused or missing")
            })?;
        let brain_recurrent = recurrent
            .get_mut(brain_index)
            .and_then(Option::take)
            .ok_or_else(|| {
                CheckpointError::format("INDEX_RANGE", "recurrent segment reused or missing")
            })?;
        let expected_weight_offset = slot.checked_mul(record.weight_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "population weight offset overflows")
        })?;
        let expected_recurrent_offset = brain_index
            .checked_mul(record.recurrent_count)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "population recurrent offset overflows")
            })?;
        if genome_weights.len() != record.weight_count
            || brain_recurrent.len() != record.recurrent_count
            || record.weight_offset != expected_weight_offset
            || record.recurrent_offset != expected_recurrent_offset
        {
            return Err(CheckpointError::format(
                "INDEX_RANGE",
                "decoded segment disagrees with population range",
            ));
        }
        population.push(PopulationGenome {
            slot: record.slot,
            brain: record.brain,
            lineage: record.lineage.clone(),
            fitness: record.fitness,
            weights: genome_weights,
        });
        brains[brain_index] = Some(BrainRuntimeState {
            handle: record.brain,
            owner: BrainOwner::PopulationSlot(record.slot),
            retained_origin: None,
            non_population_weights: None,
            recurrent: brain_recurrent,
        });
    }
    let brains = brains
        .into_iter()
        .map(|brain| {
            brain.ok_or_else(|| CheckpointError::format("INDEX_DENSE", "brain index is missing"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StateCandidate {
        versions: state.versions,
        identity: state.identity,
        config: state.config,
        phase: AuthorityPhase::GenerationBoundary(state.boundary_kind),
        generation: state.generation,
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: state.rng,
        allocators: state.allocators,
        population,
        brains,
        world: WorldState::default(),
    })
}

/// Assemble decoded metadata with empty numeric boxes for shared pre-allocation admission.
pub(super) fn assemble_allocation_shell(
    state: &DecodedStateParts,
    index: &DecodedPopulationIndex,
) -> Result<StateCandidate, CheckpointError> {
    let mut population = Vec::new();
    population
        .try_reserve_exact(index.records.len())
        .map_err(|_| {
            CheckpointError::format(
                "ALLOCATION",
                "unable to reserve allocation-shell population",
            )
        })?;
    let mut brains: Vec<Option<BrainRuntimeState>> =
        (0..index.records.len()).map(|_| None).collect();
    for record in &index.records {
        population.push(PopulationGenome {
            slot: record.slot,
            brain: record.brain,
            lineage: record.lineage.clone(),
            fitness: record.fitness,
            weights: Box::new([]),
        });
        let brain_index = record.brain_index as usize;
        brains[brain_index] = Some(BrainRuntimeState {
            handle: record.brain,
            owner: BrainOwner::PopulationSlot(record.slot),
            retained_origin: None,
            non_population_weights: None,
            recurrent: Box::new([]),
        });
    }
    let brains = brains
        .into_iter()
        .map(|brain| {
            brain.ok_or_else(|| {
                CheckpointError::format("INDEX_DENSE", "allocation-shell brain index is missing")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StateCandidate {
        versions: state.versions.clone(),
        identity: state.identity.clone(),
        config: state.config.clone(),
        phase: AuthorityPhase::GenerationBoundary(state.boundary_kind),
        generation: state.generation.clone(),
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: state.rng.clone(),
        allocators: state.allocators.clone(),
        population,
        brains,
        world: WorldState::default(),
    })
}
