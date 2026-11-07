//! Publication checks for authoritative state.

use super::*;

pub(super) fn validate_generation_successor_relation(
    source: &StateCandidate,
    successor: &StateCandidate,
) -> Result<(), StateError> {
    if source.phase != AuthorityPhase::Running {
        return invalid(
            "generation.source.phase",
            "a generation successor requires running source authority",
        );
    }
    if successor.phase != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation) {
        return invalid(
            "generation.successor.phase",
            "a terminal fixed step must stage a generation boundary",
        );
    }
    if successor.versions != source.versions
        || successor.identity != source.identity
        || successor.config != source.config
    {
        return invalid(
            "generation.successor.identity",
            "version, run, graph, or normalized configuration identity changed",
        );
    }
    let expected_generation =
        source
            .generation
            .generation
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "staged successor generation",
            })?;
    let expected_completed_step =
        source
            .generation
            .completed_step
            .checked_add(1)
            .ok_or(StateError::ArithmeticOverflow {
                context: "staged successor completed step",
            })?;
    let expected_population_epoch = source.generation.population_epoch.checked_add(1).ok_or(
        StateError::ArithmeticOverflow {
            context: "staged successor population epoch",
        },
    )?;
    if successor.generation.generation != expected_generation
        || successor.generation.completed_step != expected_completed_step
        || successor.generation.population_epoch != expected_population_epoch
    {
        return invalid(
            "generation.successor.sequence",
            "generation, completed-step, or population epoch is not the exact successor",
        );
    }
    if successor.generation.best_fitness_ever < source.generation.best_fitness_ever {
        return invalid(
            "generation.successor.best_fitness_ever",
            "all-generation best fitness regressed",
        );
    }
    Ok(())
}

pub(super) fn successor_admission_policy(
    source: &StateCandidate,
    remaining_memory_ceiling_bytes: usize,
) -> StateAdmissionPolicy {
    StateAdmissionPolicy {
        memory_ceiling_bytes: remaining_memory_ceiling_bytes,
        require_exact_build_identity: true,
        expected_source_revision: source.identity.source_revision.clone(),
        expected_engine_build_id: source.identity.engine_build_id.clone(),
        expected_source_sha256: source.identity.source_sha256.clone(),
        expected_target_triple: source.identity.target_triple.clone(),
        expected_build_profile: source.identity.build_profile.clone(),
        expected_build_class: source.identity.build_class.clone(),
        expected_rustc_version: source.identity.rustc_version.clone(),
        expected_build_contract_sha256: source.identity.build_contract_sha256.clone(),
        expected_math_backend: source.identity.math_backend.clone(),
        allowed_settings_schema_sha256: Vec::new(),
        allowed_math_backends: Vec::new(),
        expected_settings_schema_sha256: source.config.settings_schema_sha256.clone(),
    }
}

pub(super) fn validate_generation_start_replacement_contract(
    current: &StateCandidate,
    boundary: &StateCandidate,
    replacement: &GenerationStartReplacement<'_>,
    external_replacements: usize,
    removed_source_leases: usize,
    unavailable_controller_reservations: &[UnavailableControllerReservation],
) -> Result<(), StateError> {
    if boundary.phase != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation) {
        return invalid(
            "generation.start.phase",
            "running construction requires the exact durable generation boundary",
        );
    }
    if !replacement.wall_accumulator_seconds.is_finite()
        || replacement.wall_accumulator_seconds < 0.0
    {
        return invalid(
            "generation.start.wall_accumulator_seconds",
            "scheduler continuation must be finite and non-negative",
        );
    }
    let accounted_source_leases = external_replacements
        .checked_add(removed_source_leases)
        .ok_or(StateError::ArithmeticOverflow {
            context: "generation controller source accounting",
        })?;
    if accounted_source_leases != current.world.controller_leases.len()
        || replacement.world.controller_leases.len() != external_replacements
        || unavailable_controller_reservations.len() != removed_source_leases
    {
        return invalid(
            "generation.start.controller_leases",
            "connected replacements and unavailable reservations do not account for the old leases",
        );
    }
    let mut unavailable_index = 0usize;
    for lease in &current.world.controller_leases {
        if lease.status == ControllerLeaseStatus::Connected {
            continue;
        }
        let record = unavailable_controller_reservations
            .get(unavailable_index)
            .ok_or_else(|| {
                invalid_error(
                    "generation.start.unavailable_controllers",
                    "an old disconnected lease has no retained reclaim outcome",
                )
            })?;
        let expected_reason = match lease.status {
            ControllerLeaseStatus::HoldingLastInput | ControllerLeaseStatus::ReservedNeutral => {
                UnavailableControllerReason::SnakeUnavailable
            }
            ControllerLeaseStatus::NeuralTakeover => UnavailableControllerReason::GraceExpired,
            ControllerLeaseStatus::Connected => unreachable!("connected leases are skipped"),
        };
        if record.source_lease_id != lease.id
            || record.source_snake_id != lease.snake_id
            || record.controller_kind != lease.kind
            || record.scope != lease.scope
            || record.resume_token != lease.resume_token
            || record.disconnected_at_ms != lease.disconnected_at_ms
            || record.grace_expires_at_ms != lease.grace_expires_at_ms
            || record.reason != expected_reason
        {
            return invalid(
                "generation.start.unavailable_controllers",
                "retained reclaim outcome does not match its old controller lease",
            );
        }
        unavailable_index =
            unavailable_index
                .checked_add(1)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "unavailable controller index",
                })?;
    }
    if unavailable_index != unavailable_controller_reservations.len() {
        return invalid(
            "generation.start.unavailable_controllers",
            "retained reclaim outcomes contain an extra or reordered lease",
        );
    }
    if replacement.rng.version != boundary.rng.version
        || replacement.rng.evolution != boundary.rng.evolution
        || replacement.rng.evolved_respawns != boundary.rng.evolved_respawns
        || replacement.rng.baseline_behaviors != boundary.rng.baseline_behaviors
        || (external_replacements == 0
            && replacement.rng.external_controller != boundary.rng.external_controller)
    {
        return invalid(
            "generation.start.rng",
            "generation construction changed evolution RNG or unexplained external RNG",
        );
    }
    validate_running_allocator_continuation(
        &boundary.allocators,
        replacement.allocators,
        external_replacements,
    )?;
    Ok(())
}

/// Reject immutable or monotonic-continuation changes before any swap.
pub(super) fn validate_running_replacement_contract(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
) -> Result<(), StateError> {
    if candidate.rng.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION
        && replacement.mutation.baseline_v2.is_none()
    {
        return invalid(
            "fixed_step.baseline_v2",
            "complete v2 control seal is required",
        );
    }
    if !replacement.mutation.matches(replacement) {
        return invalid(
            "fixed_step.external_replacement",
            "opaque replacement proof does not match the exact staged buffers",
        );
    }
    validate_running_replacement_contract_with_counts(
        candidate,
        replacement,
        replacement.mutation.external_replacements(),
        replacement.mutation.removed_dead_external_leases(),
    )
}

/// Recheck the final sealed delivery result without trusting the now-stale
/// pre-delivery world digest. Only `engine::running_step` can construct the
/// consumed wrapper that reaches this path.
pub(super) fn validate_resolved_running_replacement_contract(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
    external_replacements: usize,
    removed_dead_external_leases: usize,
) -> Result<(), StateError> {
    if candidate.rng.version == RNG_BUNDLE_WITH_BASELINE_V2_VERSION
        && replacement.mutation.baseline_v2.is_none()
    {
        return invalid(
            "fixed_step.baseline_v2",
            "resolved v2 control seal is required",
        );
    }
    if replacement.mutation.baseline_v2.is_some_and(|proof| {
        !proof.matches(
            replacement.key,
            replacement.rng,
            &*replacement.baseline_lifecycle,
        )
    }) {
        return invalid(
            "fixed_step.baseline_v2",
            "resolved v2 continuation changed after control",
        );
    }

    validate_running_replacement_contract_with_counts(
        candidate,
        replacement,
        external_replacements,
        removed_dead_external_leases,
    )
}

pub(super) fn validate_running_replacement_contract_with_counts(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
    external_replacements: usize,
    removed_dead_external_leases: usize,
) -> Result<(), StateError> {
    if replacement.mutation.evolved.is_some_and(|proof| {
        !proof.matches(
            replacement.key,
            candidate,
            replacement.world,
            replacement.rng,
            replacement.allocators,
        )
    }) {
        return invalid(
            "fixed_step.evolved_replacement",
            "private evolved witness does not match source or staged identity/RNG",
        );
    }
    // Once initialized, a live continuation cannot silently drop its ledger or
    // reassign a population owner. Each evolved replacement carries its own
    // narrow witness independently of external replacement permissions.
    let source_lives = &candidate.fixed_step.evolved_generation;
    if !source_lives.slots.is_empty() {
        replacement
            .evolved_generation
            .validate_event_continuation(source_lives)
            .map_err(|error| StateError::InvalidField {
                field: "fixed_step.evolved_generation.life_ends",
                reason: error.to_string(),
            })?;
        if replacement.evolved_generation.version != source_lives.version
            || replacement.evolved_generation.slots.len() != source_lives.slots.len()
        {
            return invalid(
                "fixed_step.evolved_generation",
                "running life continuation was removed or changed version",
            );
        }
        for (source, staged) in source_lives
            .slots
            .iter()
            .zip(&replacement.evolved_generation.slots)
        {
            let record = replacement
                .mutation
                .evolved
                .and_then(|proof| proof.record(source.population_slot));
            let identity_valid = if let Some(record) = record {
                source.physical_id == record.old_id
                    && staged.physical_id == record.new_id
                    && source.lives_started.checked_add(1) == Some(staged.lives_started)
                    && source.successful_respawns.checked_add(1) == Some(staged.successful_respawns)
            } else {
                source.physical_id == staged.physical_id
                    && source.lives_started == staged.lives_started
                    && source.successful_respawns == staged.successful_respawns
            };
            if source.population_slot != staged.population_slot
                || !identity_valid
                || staged.deaths < source.deaths
                || staged.ledger.operations < source.ledger.operations
                || staged.ledger.generation_gross_earned < source.ledger.generation_gross_earned
                || staged.ledger.generation_boost_spent < source.ledger.generation_boost_spent
            {
                return invalid(
                    "fixed_step.evolved_generation",
                    "stable population life or monotonic accounting changed",
                );
            }
        }
    }
    let expected_elapsed =
        candidate.generation.elapsed_seconds + candidate.config.fixed_step_seconds;
    if !expected_elapsed.is_finite() {
        return invalid(
            "fixed_step.generation_elapsed_seconds",
            "one fixed-delta advance must remain finite",
        );
    }
    if replacement.generation_elapsed_seconds.to_bits() != expected_elapsed.to_bits() {
        return invalid(
            "fixed_step.generation_elapsed_seconds",
            "must advance by exactly one admitted fixed delta",
        );
    }
    if replacement.sensor_generation.best_points_this_generation()
        < candidate
            .fixed_step
            .sensor_generation
            .best_points_this_generation()
    {
        return invalid(
            "fixed_step.sensor_generation",
            "generation best cannot regress",
        );
    }
    if replacement.rng.version != candidate.rng.version
        || (replacement.mutation.baseline_v2.is_none()
            && replacement.rng.baseline_behaviors != candidate.rng.baseline_behaviors)
        || replacement.rng.evolution != candidate.rng.evolution
        || (replacement.mutation.evolved.is_none()
            && replacement.rng.evolved_respawns != candidate.rng.evolved_respawns)
    {
        return invalid(
            "fixed_step.rng",
            "nonterminal steps cannot replace RNG identity or advance evolution",
        );
    }
    validate_running_allocator_continuation(
        &candidate.allocators,
        replacement.allocators,
        external_replacements,
    )?;
    validate_running_brain_continuation(
        candidate,
        replacement,
        external_replacements != 0 || removed_dead_external_leases != 0,
    )?;
    let expected_leases = candidate
        .world
        .controller_leases
        .len()
        .checked_sub(removed_dead_external_leases)
        .ok_or_else(|| {
            invalid_error(
                "fixed_step.controller_leases",
                "removed external lease count exceeds the source lease count",
            )
        })?;
    if replacement.world.controller_leases.len() != expected_leases {
        return invalid(
            "fixed_step.controller_leases",
            "lease count does not match the private controlled-death mutation proof",
        );
    }
    Ok(())
}

/// Enforce the allocator domains that may advance during one nonterminal step.
pub(super) fn validate_running_allocator_continuation(
    source: &AllocatorState,
    staged: &AllocatorState,
    replacement_count: usize,
) -> Result<(), StateError> {
    let replacements =
        u64::try_from(replacement_count).map_err(|_| StateError::ArithmeticOverflow {
            context: "controlled replacement count",
        })?;
    let expected_brain =
        source
            .next_brain_id
            .checked_add(replacements)
            .ok_or(StateError::ArithmeticOverflow {
                context: "controlled replacement brain continuation",
            })?;
    let expected_lease = source
        .next_controller_lease_id
        .checked_add(replacements)
        .ok_or(StateError::ArithmeticOverflow {
            context: "controlled replacement lease continuation",
        })?;
    let expected_external = source.next_external_id.checked_add(replacements).ok_or(
        StateError::ArithmeticOverflow {
            context: "controlled replacement external continuation",
        },
    )?;
    if staged.version != source.version
        || staged.next_brain_id != expected_brain
        || staged.next_genome_id != source.next_genome_id
        || staged.next_controller_lease_id != expected_lease
        || staged.next_external_id != expected_external
        || staged.next_resurrected_id != source.next_resurrected_id
    {
        return invalid(
            "fixed_step.allocators",
            "nonterminal identity domains do not match the private controlled-death mutation proof",
        );
    }
    if staged.next_entity_id < source.next_entity_id
        || staged.next_baseline_id < source.next_baseline_id
    {
        return invalid(
            "fixed_step.allocators",
            "monotonic gameplay allocator regressed",
        );
    }
    // Public IDs are circular. Reservation proofs and world validation enforce live uniqueness;
    // unlike durable counters, their next candidate need not exceed every existing ID.
    validate_allocators(staged)?;
    Ok(())
}

/// Keep all existing brain identities and weights immutable unless an opaque
/// replacement-workspace proof binds the complete staged brain payload.
pub(super) fn validate_running_brain_continuation(
    candidate: &StateCandidate,
    replacement: &RunningStepReplacement<'_>,
    permits_identity_change: bool,
) -> Result<(), StateError> {
    if permits_identity_change {
        return Ok(());
    }
    if replacement.brains.len() != candidate.brains.len() {
        return invalid(
            "fixed_step.brains",
            "a stable-identity running step cannot change brain records",
        );
    }
    for source in &candidate.brains {
        let retained = replacement.brains.iter().any(|staged| {
            source.handle == staged.handle
                && source.owner == staged.owner
                && source.retained_origin == staged.retained_origin
                && optional_f32_bits_equal(
                    source.non_population_weights.as_deref(),
                    staged.non_population_weights.as_deref(),
                )
        });
        if retained {
            continue;
        }
        return invalid(
            "fixed_step.brains",
            "a stable-identity running step may change recurrent state but not brain identity or weights",
        );
    }
    Ok(())
}

/// Compare optional packed Float32 values by exact stored bits.
pub(super) fn optional_f32_bits_equal(left: Option<&[f32]>, right: Option<&[f32]>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| left.to_bits() == right.to_bits())
        }
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// Revalidate every mutable authority component after a staged running step.
///
/// Immutable admission identity, versions, normalized configuration, graph and
/// population weights cannot be supplied by [`RunningStepReplacement`] and
/// remain the values admitted by [`AuthoritativeState::validate_and_own`].
pub(super) fn validate_running_mutable_state(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
    world_scratch: &mut WorldValidationScratch,
) -> Result<(), StateError> {
    if candidate.phase != AuthorityPhase::Running {
        return invalid("phase", "running-step publication changed authority phase");
    }
    validate_generation(&candidate.generation)?;
    validate_rng_bundle(&candidate.rng, &candidate.config)?;
    validate_allocators(&candidate.allocators)?;
    validate_running_recurrent_state(candidate, graph)?;
    validate_world_with_scratch(candidate, world_scratch)?;
    validate_fixed_step_continuation(candidate)?;
    Ok(())
}

/// Validate the only mutable neural payload after a running-step swap.
///
/// `validate_running_replacement_contract` has already proved either that all
/// static brain fields match the admitted source or that the opaque controlled-
/// death proof binds the exact replacement brain bytes. Population metadata and
/// genome weights are not part of the replacement. Only recurrent length and
/// finiteness still require the general post-swap scan.
pub(super) fn validate_running_recurrent_state(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    for (index, brain) in candidate.brains.iter().enumerate() {
        if brain.recurrent.len() != graph.total_state_size {
            return Err(StateError::RecurrentLength {
                handle: brain.handle,
                expected: graph.total_state_size,
                actual: brain.recurrent.len(),
            });
        }
        validate_f32_slice("brains.recurrent", index, &brain.recurrent)?;
    }
    Ok(())
}
