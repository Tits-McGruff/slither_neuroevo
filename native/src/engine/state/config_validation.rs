//! Config validation for authoritative state.

use super::*;

/// Validate one complete candidate without mutating it.
pub(super) fn validate_candidate(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
    policy: &StateAdmissionPolicy,
    world_scratch: &mut WorldValidationScratch,
) -> Result<(), StateError> {
    validate_admission_header(candidate, graph, policy)?;
    validate_generation(&candidate.generation)?;
    validate_rng_bundle(&candidate.rng, &candidate.config)?;
    validate_allocators(&candidate.allocators)?;
    validate_population(candidate, graph)?;
    validate_world_with_scratch(candidate, world_scratch)?;
    if matches!(candidate.phase, AuthorityPhase::GenerationBoundary(_)) {
        validate_generation_boundary(candidate, graph)?;
    }
    validate_fixed_step_continuation(candidate)?;
    Ok(())
}

pub(super) fn validate_config(
    config: &NormalizedEngineConfig,
    versions: &ContractVersions,
    identity: &RunIdentity,
    policy: &StateAdmissionPolicy,
) -> Result<(), StateError> {
    if config.version != NORMALIZED_CONFIG_VERSION {
        return invalid("config.version", "unsupported normalized-config version");
    }
    if identity.config_revision == 0 {
        return invalid("identity.config_revision", "revision must be positive");
    }
    validate_positive_f64("config.fixed_step_seconds", config.fixed_step_seconds)?;
    validate_positive_f64("config.requested_sim_speed", config.requested_sim_speed)?;
    validate_positive_f64("config.world_radius", config.world_radius)?;
    if config.population_count == 0 {
        return invalid("config.population_count", "count must be positive");
    }
    let minimum_world_snakes = config
        .population_count
        .checked_add(config.baseline_count)
        .and_then(|count| count.checked_add(config.max_non_population_brains))
        .ok_or(StateError::ArithmeticOverflow {
            context: "maximum world snakes",
        })?;
    if config.max_world_snakes < minimum_world_snakes {
        return invalid(
            "config.max_world_snakes",
            "must cover population, baseline slots, and non-population brains",
        );
    }
    if config.max_body_points == 0 || config.max_pellets == 0 {
        return invalid(
            "config.capacity",
            "body and pellet capacities must be positive",
        );
    }
    if config.controller_input_hold_ms == 0
        || config.controller_disconnect_grace_ms == 0
        || config.controller_input_hold_ms >= config.controller_disconnect_grace_ms
    {
        return invalid(
            "config.controller_wall_time",
            "input hold must be positive and shorter than disconnect grace",
        );
    }
    if config.spatial_index_bytes == 0
        || config.worker_scratch_bytes == 0
        || config.checkpoint_scratch_bytes == 0
    {
        return invalid(
            "config.memory_reservations",
            "spatial, worker, and checkpoint scratch declarations must be positive",
        );
    }
    validate_text(
        "config.graph_architecture_key",
        &config.graph_architecture_key,
    )?;
    if config.settings.is_empty() {
        return invalid(
            "config.settings",
            "settings complete for the admitted schema are required",
        );
    }
    validate_sha256(
        "config.settings_schema_sha256",
        &config.settings_schema_sha256,
    )?;
    let actual_schema = normalized_settings_schema_hash(&config.settings)?;
    if actual_schema != config.settings_schema_sha256
        || (actual_schema != policy.expected_settings_schema_sha256
            && !policy
                .allowed_settings_schema_sha256
                .contains(&actual_schema))
    {
        return invalid(
            "config.settings_schema_sha256",
            "setting paths or value kinds do not match the admitted schema",
        );
    }
    for setting in &config.settings {
        match &setting.value {
            NormalizedSettingValue::Float(value) if !value.is_finite() => {
                return invalid("config.settings.value", "floating value must be finite");
            }
            NormalizedSettingValue::Text(value) => {
                validate_text("config.settings.value", value)?;
            }
            _ => {}
        }
    }
    require_integer_projection(config, "brain.sensorVersion", i64::from(versions.sensor))?;
    require_integer_projection(
        config,
        "baselineBots.count",
        i64::try_from(config.baseline_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "baseline count projection",
        })?,
    )?;
    require_float_projection(config, "simSpeed", config.requested_sim_speed)?;
    require_integer_projection(
        config,
        "snakeCount",
        i64::try_from(config.population_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "population count projection",
        })?,
    )?;
    if config.world_radius.fract() != 0.0
        || config.world_radius > i64::MAX as f64
        || config.world_radius < i64::MIN as f64
    {
        return invalid(
            "config.world_radius",
            "worldRadius integer projection is not representable",
        );
    }
    require_integer_projection(config, "worldRadius", config.world_radius as i64)?;
    let computed_hash = normalized_config_hash(config)?;
    if identity.config_hash != computed_hash {
        return invalid(
            "identity.config_hash",
            "does not match canonical normalized configuration",
        );
    }
    Ok(())
}

pub(super) fn validate_generation(generation: &GenerationState) -> Result<(), StateError> {
    if generation.boundary_version != GENERATION_BOUNDARY_VERSION {
        return invalid(
            "generation.boundary_version",
            "unsupported boundary version",
        );
    }
    if generation.generation == 0 || generation.population_epoch == 0 {
        return invalid(
            "generation",
            "generation and population epoch must be positive",
        );
    }
    validate_nonnegative_f64("generation.elapsed_seconds", generation.elapsed_seconds)?;
    validate_nonnegative_f64(
        "generation.wall_accumulator_seconds",
        generation.wall_accumulator_seconds,
    )?;
    if !generation.best_fitness_ever.is_finite() {
        return invalid("generation.best_fitness_ever", "value must be finite");
    }
    Ok(())
}

pub(super) fn validate_fixed_step_continuation(
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    let continuation = &candidate.fixed_step;
    if continuation.evolved_generation.version
        != crate::engine::evolved_lives::EVOLVED_LIVES_VERSION
    {
        return invalid(
            "fixed_step.evolved_generation",
            "unsupported life accounting version",
        );
    }
    continuation
        .evolved_generation
        .validate_events()
        .map_err(|error| StateError::InvalidField {
            field: "fixed_step.evolved_generation.life_ends",
            reason: error.to_string(),
        })?;
    let placement = continuation.evolved_generation.placement_work;
    placement
        .validate()
        .map_err(|error| StateError::InvalidField {
            field: "fixed_step.evolved_generation.placement_work",
            reason: error.to_string(),
        })?;
    if (continuation.evolved_generation.slots.is_empty()
        && placement != crate::engine::evolved_lives::EvolvedPlacementWork::default())
        || (placement.step != 0 && placement.step != candidate.generation.completed_step)
    {
        return invalid("fixed_step.evolved_generation.placement_work", "placement report must belong to the committed step and be empty before life initialization");
    }
    if !continuation.evolved_generation.slots.is_empty() {
        if matches!(candidate.phase, AuthorityPhase::GenerationBoundary(_)) {
            return invalid(
                "fixed_step.evolved_generation",
                "pre-spawn boundary cannot contain physical lives",
            );
        }
        continuation
            .evolved_generation
            .validate(&candidate.world, candidate.config.population_count)
            .map_err(|error| StateError::InvalidField {
                field: "fixed_step.evolved_generation",
                reason: error.to_string(),
            })?;
    }
    validate_nonnegative_f64(
        "fixed_step.ambient_pellet_accumulator",
        continuation.ambient_pellet_accumulator,
    )?;
    let generation_best = continuation.sensor_generation.best_points_this_generation();
    if !generation_best.is_finite() || generation_best < 0.0 {
        return invalid(
            "fixed_step.sensor_generation",
            "generation best points must be finite and nonnegative",
        );
    }
    let maximum_alive_evolved_points = candidate
        .world
        .snakes
        .iter()
        .filter(|snake| snake.alive && snake.kind == SnakeKind::Evolved)
        .fold(0.0_f64, |maximum, snake| maximum.max(snake.points));
    if generation_best < maximum_alive_evolved_points {
        return invalid(
            "fixed_step.sensor_generation",
            "generation best points cannot trail a live evolved snake",
        );
    }
    let v2 =
        crate::engine::baseline_v2::BaselineV2Settings::from_settings(&candidate.config.settings)
            .map_err(|_| StateError::InvalidField {
                field: "baselineBots.controlVersion",
                reason: "invalid baseline-v2 settings".into(),
            })?
            .is_some();
    for slot in &continuation.baseline_lifecycle.slots {
        if slot.v2_policy.is_some() != v2
            || slot
                .v2_last_control_step
                .is_some_and(|step| step == 0 || step > candidate.generation.completed_step)
        {
            return invalid(
                "fixed_step.baseline_lifecycle",
                "policy/cadence continuation differs from baseline contract",
            );
        }
    }
    continuation
        .baseline_lifecycle
        .validate_authoritative(
            &candidate.world,
            candidate.config.baseline_count,
            matches!(candidate.phase, AuthorityPhase::GenerationBoundary(_)),
        )
        .map_err(|error| StateError::InvalidField {
            field: "fixed_step.baseline_lifecycle",
            reason: error.to_string(),
        })?;
    Ok(())
}

pub(super) fn validate_rng_bundle(
    bundle: &RngStateBundle,
    config: &NormalizedEngineConfig,
) -> Result<(), StateError> {
    if !matches!(
        bundle.version,
        RNG_BUNDLE_VERSION | RNG_BUNDLE_WITH_EVOLVED_VERSION | RNG_BUNDLE_WITH_BASELINE_V2_VERSION
    ) {
        return invalid("rng.version", "unsupported RNG-bundle version");
    }
    if bundle.version == RNG_BUNDLE_VERSION && !bundle.evolved_respawns.is_empty() {
        return invalid(
            "rng.evolved_respawns",
            "frozen RNG bundle cannot carry evolved streams",
        );
    }
    if bundle.version >= RNG_BUNDLE_WITH_EVOLVED_VERSION {
        if bundle.evolved_respawns.len() != config.population_count {
            return invalid("rng.evolved_respawns", "count must match the population");
        }
        for (slot, stream) in bundle.evolved_respawns.iter().enumerate() {
            if stream.slot as usize != slot || stream.runtime().is_err() {
                return invalid(
                    "rng.evolved_respawns",
                    "evolved streams must be dense and nonzero",
                );
            }
        }
    }
    let v2 = crate::engine::baseline_v2::BaselineV2Settings::from_settings(&config.settings)
        .map_err(|_| StateError::InvalidField {
            field: "baselineBots.controlVersion",
            reason: "incomplete or incompatible baseline-v2 settings".into(),
        })?;
    if v2.is_some() != (bundle.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION) {
        return invalid(
            "rng.version",
            "baseline-v2 settings require exactly RNG bundle v3",
        );
    }
    if bundle.version < RNG_BUNDLE_WITH_BASELINE_V2_VERSION && !bundle.baseline_behaviors.is_empty()
    {
        return invalid(
            "rng.baseline_behaviors",
            "frozen RNG bundles cannot carry behavior streams",
        );
    }
    if v2.is_some() {
        if bundle.baseline_behaviors.len() != config.baseline_count {
            return invalid("rng.baseline_behaviors", "count must match baseline slots");
        }
        for (slot, stream) in bundle.baseline_behaviors.iter().enumerate() {
            if stream.slot as usize != slot || stream.runtime().is_err() {
                return invalid(
                    "rng.baseline_behaviors",
                    "behavior slots must be dense and nonzero",
                );
            }
        }
    }
    if bundle.baselines.len() != config.baseline_count {
        return invalid(
            "rng.baselines",
            "count does not match normalized baseline configuration",
        );
    }
    validate_rng("world", &bundle.world)?;
    validate_rng("evolution", &bundle.evolution)?;
    validate_rng("external-controller", &bundle.external_controller)?;
    let mut expected_slot = 0u32;
    for baseline in &bundle.baselines {
        if baseline.slot != expected_slot {
            return invalid("rng.baselines", "baseline slots must be dense and ordered");
        }
        validate_baseline_rng(baseline.slot, &baseline.state)?;
        expected_slot = expected_slot
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "baseline RNG slot",
            })?;
    }
    Ok(())
}

pub(super) fn validate_rng(stream: &str, state: &SerializedRngState) -> Result<(), StateError> {
    StatefulRng::from_state(state)
        .map(|_| ())
        .map_err(|error: RngError| StateError::InvalidRng {
            stream: stream.to_owned(),
            reason: error.to_string(),
        })
}

/// Validate one dense baseline stream without formatting its success-path label.
pub(super) fn validate_baseline_rng(
    slot: u32,
    state: &SerializedRngState,
) -> Result<(), StateError> {
    StatefulRng::from_state(state)
        .map(|_| ())
        .map_err(|error: RngError| StateError::InvalidRng {
            stream: format!("baseline:{slot}"),
            reason: error.to_string(),
        })
}

pub(super) fn validate_allocators(allocators: &AllocatorState) -> Result<(), StateError> {
    if allocators.version != ALLOCATOR_VERSION
        || allocators.next_entity_id == 0
        || allocators.next_brain_id == 0
        || allocators.next_genome_id == 0
        || allocators.next_controller_lease_id == 0
        || allocators.next_external_id == 0
        || allocators.next_baseline_id == 0
        || allocators.next_resurrected_id == 0
    {
        return invalid(
            "allocators",
            "version is unsupported or a next identity is zero",
        );
    }
    validate_next_domain(
        "allocators.next_entity_id",
        allocators.next_entity_id,
        1,
        EXTERNAL_ENTITY_ID_START,
    )?;
    validate_next_domain(
        "allocators.next_external_id",
        allocators.next_external_id,
        EXTERNAL_ENTITY_ID_START,
        BASELINE_ENTITY_ID_START,
    )?;
    validate_next_domain(
        "allocators.next_baseline_id",
        allocators.next_baseline_id,
        BASELINE_ENTITY_ID_START,
        RESURRECTED_ENTITY_ID_START,
    )?;
    validate_next_domain(
        "allocators.next_resurrected_id",
        allocators.next_resurrected_id,
        RESURRECTED_ENTITY_ID_START,
        RESURRECTED_ENTITY_ID_EXHAUSTED,
    )?;
    if allocators.next_frame_v1_id == 0 || allocators.next_frame_v1_id > FRAME_V1_EXHAUSTED_ID {
        return invalid(
            "allocators.next_frame_v1_id",
            "outside frame-v1 exact-ID range",
        );
    }
    Ok(())
}

pub(super) fn require_integer_projection(
    config: &NormalizedEngineConfig,
    path: &'static str,
    expected: i64,
) -> Result<(), StateError> {
    let value = config
        .settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value);
    if value != Some(&NormalizedSettingValue::Integer(expected)) {
        return invalid("config.settings", "required integer projection differs");
    }
    Ok(())
}

pub(super) fn require_float_projection(
    config: &NormalizedEngineConfig,
    path: &'static str,
    expected: f64,
) -> Result<(), StateError> {
    let value = config
        .settings
        .iter()
        .find(|setting| setting.path == path)
        .map(|setting| &setting.value);
    match value {
        Some(NormalizedSettingValue::Float(actual)) if actual.to_bits() == expected.to_bits() => {
            Ok(())
        }
        _ => invalid("config.settings", "required floating projection differs"),
    }
}

pub(super) fn validate_next_domain(
    field: &'static str,
    next: u64,
    start: u64,
    exhausted: u64,
) -> Result<(), StateError> {
    if next < start || next > exhausted {
        return invalid(field, "outside its checked allocator domain");
    }
    Ok(())
}

pub(super) fn validate_next_after(
    field: &'static str,
    next: u64,
    maximum_existing: u64,
    exhausted: u64,
) -> Result<(), StateError> {
    if next != exhausted && next <= maximum_existing {
        return invalid(field, "does not follow existing identities");
    }
    Ok(())
}
