//! World validation for authoritative state.

use super::*;

pub(super) fn validate_population(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    if candidate.population.len() != candidate.config.population_count {
        return invalid(
            "population",
            "count does not match normalized configuration",
        );
    }
    let max_population = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
    if candidate.population.len() > max_population {
        return invalid("population", "count exceeds stable u32 slot space");
    }

    let mut genome_ids = BTreeSet::new();
    let mut population_handles = BTreeSet::new();
    let mut max_genome_id = 0u64;
    for (index, genome) in candidate.population.iter().enumerate() {
        let expected_slot = u32::try_from(index).map_err(|_| StateError::ArithmeticOverflow {
            context: "population slot",
        })?;
        if genome.slot != expected_slot {
            return Err(StateError::NonDensePopulationSlot {
                index,
                slot: genome.slot,
            });
        }
        validate_handle(genome.brain, candidate.generation.population_epoch)?;
        if !population_handles.insert(genome.brain) {
            return Err(StateError::DuplicateBrainHandle(genome.brain));
        }
        if genome.lineage.genome_id == 0
            || genome.lineage.genome_id == u64::MAX
            || !genome_ids.insert(genome.lineage.genome_id)
        {
            return Err(StateError::DuplicateId {
                kind: "genome",
                id: genome.lineage.genome_id,
            });
        }
        max_genome_id = max_genome_id.max(genome.lineage.genome_id);
        for parent in [genome.lineage.parent_a, genome.lineage.parent_b]
            .into_iter()
            .flatten()
        {
            if parent == 0 || parent >= candidate.allocators.next_genome_id {
                return invalid(
                    "population.lineage.parent",
                    "parent identity must be nonzero and precede the next genome identity",
                );
            }
        }
        if genome.lineage.birth_generation == 0
            || genome.lineage.birth_generation > candidate.generation.generation
        {
            return invalid("population.lineage", "invalid birth generation");
        }
        if !genome.fitness.is_finite() {
            return Err(StateError::NonFinite {
                field: "population.fitness",
                index,
            });
        }
        if genome.weights.len() != graph.total_parameters {
            return Err(StateError::WeightLength {
                slot: genome.slot,
                expected: graph.total_parameters,
                actual: genome.weights.len(),
            });
        }
        validate_f32_slice("population.weights", index, &genome.weights)?;
    }

    let max_brains = candidate
        .population
        .len()
        .checked_add(candidate.config.max_non_population_brains)
        .ok_or(StateError::ArithmeticOverflow {
            context: "brain capacity",
        })?;
    if candidate.brains.len() > max_brains {
        return invalid("brains", "count exceeds configured capacity");
    }
    let mut all_handles = BTreeSet::new();
    for (index, brain) in candidate.brains.iter().enumerate() {
        validate_handle(brain.handle, candidate.generation.population_epoch)?;
        if !all_handles.insert(brain.handle) {
            return Err(StateError::DuplicateBrainHandle(brain.handle));
        }
        if brain.recurrent.len() != graph.total_state_size {
            return Err(StateError::RecurrentLength {
                handle: brain.handle,
                expected: graph.total_state_size,
                actual: brain.recurrent.len(),
            });
        }
        validate_f32_slice("brains.recurrent", index, &brain.recurrent)?;
        match brain.owner {
            BrainOwner::PopulationSlot(slot) => {
                if brain.non_population_weights.is_some() || brain.retained_origin.is_some() {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                }
                let genome = candidate.population.get(slot as usize);
                if genome.is_none_or(|genome| genome.brain != brain.handle) {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                }
            }
            BrainOwner::Entity(entity_id) => {
                let Some(weights) = &brain.non_population_weights else {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                };
                if entity_id == 0 {
                    return Err(StateError::InvalidBrainOwner(brain.handle));
                }
                if weights.len() != graph.total_parameters {
                    return Err(StateError::BrainWeightLength {
                        handle: brain.handle,
                        expected: graph.total_parameters,
                        actual: weights.len(),
                    });
                }
                validate_f32_slice("brains.non_population_weights", index, weights)?;
                if let Some(origin) = &brain.retained_origin {
                    origin.validate_weights(weights).map_err(|reason| {
                        StateError::InvalidField {
                            field: "brains.retained_origin",
                            reason,
                        }
                    })?;
                }
            }
        }
    }
    for genome in &candidate.population {
        if !all_handles.contains(&genome.brain) {
            return Err(StateError::InvalidBrainOwner(genome.brain));
        }
    }
    if candidate.allocators.next_genome_id != u64::MAX
        && candidate.allocators.next_genome_id <= max_genome_id
    {
        return invalid(
            "allocators.next_genome_id",
            "does not follow existing genome identities",
        );
    }
    Ok(())
}

pub(super) fn validate_handle(handle: BrainHandle, expected_epoch: u64) -> Result<(), StateError> {
    if handle.id == 0
        || handle.id == u64::MAX
        || handle.epoch == 0
        || handle.epoch != expected_epoch
    {
        return Err(StateError::InvalidBrainOwner(handle));
    }
    Ok(())
}

pub(super) fn validate_world_with_scratch(
    candidate: &StateCandidate,
    scratch: &mut WorldValidationScratch,
) -> Result<(), StateError> {
    if candidate.world.body_points.len() > candidate.config.max_body_points
        || candidate.world.pellets.len() > candidate.config.max_pellets
        || candidate.world.snakes.len() > candidate.config.max_world_snakes
        || candidate.world.controller_leases.len() > candidate.config.max_world_snakes
    {
        return invalid("world.capacity", "configured world capacity exceeded");
    }
    for (index, point) in candidate.world.body_points.iter().enumerate() {
        validate_point("world.body_points", index, *point)?;
    }

    let entity_capacity = candidate
        .world
        .snakes
        .len()
        .checked_add(candidate.world.pellets.len())
        .ok_or(StateError::ArithmeticOverflow {
            context: "world validation entity count",
        })?;
    scratch.clear();
    reserve_validation(&mut scratch.entity_ids, entity_capacity, "world entity IDs")?;
    reserve_validation(
        &mut scratch.snake_ids,
        candidate.world.snakes.len(),
        "world snake IDs",
    )?;
    reserve_validation(
        &mut scratch.public_ids,
        candidate.world.snakes.len(),
        "frame-v1 snake IDs",
    )?;
    reserve_validation(
        &mut scratch.evolved_slots,
        candidate.config.population_count,
        "world population slots",
    )?;
    reserve_validation(
        &mut scratch.world_brains,
        candidate.world.snakes.len(),
        "world brain handles",
    )?;
    reserve_validation(
        &mut scratch.baseline_slots,
        candidate.config.baseline_count,
        "world baseline slots",
    )?;
    reserve_validation(
        &mut scratch.ranges,
        candidate.world.snakes.len(),
        "world body ranges",
    )?;
    let entity_ids = &mut scratch.entity_ids;
    let snake_ids = &mut scratch.snake_ids;
    let public_ids = &mut scratch.public_ids;
    let evolved_slots = &mut scratch.evolved_slots;
    let world_brains = &mut scratch.world_brains;
    let baseline_slots = &mut scratch.baseline_slots;
    let ranges = &mut scratch.ranges;
    let mut max_general_id = 0u64;
    let mut max_external_id = 0u64;
    let mut max_baseline_id = 0u64;
    let mut max_resurrected_id = 0u64;
    for (index, snake) in candidate.world.snakes.iter().enumerate() {
        if snake.id == 0 || snake.id == u64::MAX {
            return Err(StateError::DuplicateId {
                kind: "snake",
                id: snake.id,
            });
        }
        entity_ids.push(snake.id);
        snake_ids.push(snake.id);
        if snake.frame_v1_id == 0 || snake.frame_v1_id > FRAME_V1_MAX_EXACT_ID {
            return Err(StateError::DuplicateId {
                kind: "frame-v1 snake",
                id: u64::from(snake.frame_v1_id),
            });
        }
        public_ids.push(snake.frame_v1_id);
        validate_snake(index, snake, candidate)?;
        match snake.kind {
            SnakeKind::Evolved => {
                validate_entity_id_domain("evolved snake", snake.id, 1, EXTERNAL_ENTITY_ID_START)?;
                max_general_id = max_general_id.max(snake.id);
                let Some(slot) = snake.population_slot else {
                    return invalid(
                        "world.snakes.population_slot",
                        "evolved snake requires a population slot",
                    );
                };
                evolved_slots.push(slot);
            }
            SnakeKind::External => {
                validate_entity_id_domain(
                    "external snake",
                    snake.id,
                    EXTERNAL_ENTITY_ID_START,
                    BASELINE_ENTITY_ID_START,
                )?;
                max_external_id = max_external_id.max(snake.id);
            }
            SnakeKind::Baseline => {
                validate_entity_id_domain(
                    "baseline snake",
                    snake.id,
                    BASELINE_ENTITY_ID_START,
                    RESURRECTED_ENTITY_ID_START,
                )?;
                max_baseline_id = max_baseline_id.max(snake.id);
            }
            SnakeKind::Resurrected => {
                validate_entity_id_domain(
                    "resurrected snake",
                    snake.id,
                    RESURRECTED_ENTITY_ID_START,
                    RESURRECTED_ENTITY_ID_EXHAUSTED,
                )?;
                max_resurrected_id = max_resurrected_id.max(snake.id);
            }
        }
        if let Some(handle) = snake.brain {
            world_brains.push(handle);
        }
        if let Some(slot) = snake.baseline_slot {
            if candidate
                .rng
                .baselines
                .get(slot as usize)
                .is_none_or(|baseline| baseline.slot != slot)
            {
                return invalid(
                    "world.snakes.baseline_slot",
                    "baseline slot has no matching RNG stream",
                );
            }
            baseline_slots.push(slot);
        }
        let end = snake
            .body
            .start
            .checked_add(snake.body.len)
            .ok_or(StateError::InvalidBodyRange { snake_id: snake.id })?;
        if end > candidate.world.body_points.len() || (snake.alive && snake.body.len == 0) {
            return Err(StateError::InvalidBodyRange { snake_id: snake.id });
        }
        if snake.body.len != 0 {
            if candidate.world.body_points[snake.body.start] != snake.position {
                return Err(StateError::InvalidBodyRange { snake_id: snake.id });
            }
            ranges.push((snake.body.start, end, snake.id));
        }
    }

    if let Some(id) = first_duplicate_after_sort(snake_ids) {
        return Err(StateError::DuplicateId { kind: "snake", id });
    }
    if let Some(id) = first_duplicate_after_sort(public_ids) {
        return Err(StateError::DuplicateId {
            kind: "frame-v1 snake",
            id: u64::from(id),
        });
    }
    if let Some(slot) = first_duplicate_after_sort(evolved_slots) {
        return Err(StateError::DuplicateId {
            kind: "world population slot",
            id: u64::from(slot),
        });
    }
    if let Some(handle) = first_duplicate_after_sort(world_brains) {
        return Err(StateError::DuplicateBrainHandle(handle));
    }
    if first_duplicate_after_sort(baseline_slots).is_some() {
        return invalid("world.snakes.baseline_slot", "baseline slot must be unique");
    }

    for brain in &candidate.brains {
        if let BrainOwner::Entity(entity_id) = brain.owner {
            let matched = candidate.world.snakes.iter().any(|snake| {
                snake.id == entity_id
                    && snake.brain == Some(brain.handle)
                    && snake.population_slot.is_none()
                    && (brain.retained_origin.is_some() == (snake.kind == SnakeKind::Resurrected))
            });
            if !matched {
                return Err(StateError::InvalidBrainOwner(brain.handle));
            }
        }
    }
    ranges.sort_unstable_by_key(|range| range.0);
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(StateError::InvalidBodyRange {
                snake_id: pair[1].2,
            });
        }
    }

    for (index, pellet) in candidate.world.pellets.iter().enumerate() {
        if pellet.id == 0 || pellet.id >= EXTERNAL_ENTITY_ID_START {
            return Err(StateError::DuplicateId {
                kind: "world entity",
                id: pellet.id,
            });
        }
        entity_ids.push(pellet.id);
        max_general_id = max_general_id.max(pellet.id);
        validate_point("world.pellets.position", index, pellet.position)?;
        if !pellet.value.is_finite() || pellet.value <= 0.0 {
            return Err(StateError::NonFinite {
                field: "world.pellets.value",
                index,
            });
        }
        if pellet
            .owner
            .is_some_and(|owner| snake_ids.binary_search(&owner).is_err())
        {
            return invalid("world.pellets.owner", "owner does not identify a snake");
        }
    }
    if let Some(id) = first_duplicate_after_sort(entity_ids) {
        return Err(StateError::DuplicateId {
            kind: "world entity",
            id,
        });
    }

    validate_next_after(
        "allocators.next_entity_id",
        candidate.allocators.next_entity_id,
        max_general_id,
        EXTERNAL_ENTITY_ID_START,
    )?;
    validate_next_after(
        "allocators.next_external_id",
        candidate.allocators.next_external_id,
        max_external_id,
        BASELINE_ENTITY_ID_START,
    )?;
    validate_next_after(
        "allocators.next_baseline_id",
        candidate.allocators.next_baseline_id,
        max_baseline_id,
        RESURRECTED_ENTITY_ID_START,
    )?;
    validate_next_after(
        "allocators.next_resurrected_id",
        candidate.allocators.next_resurrected_id,
        max_resurrected_id,
        RESURRECTED_ENTITY_ID_EXHAUSTED,
    )?;
    let lease_count = candidate.world.controller_leases.len();
    reserve_validation(&mut scratch.lease_ids, lease_count, "controller lease IDs")?;
    reserve_validation(
        &mut scratch.lease_snakes,
        lease_count,
        "controller lease snake IDs",
    )?;
    reserve_validation(
        &mut scratch.resume_token_order,
        lease_count,
        "controller resume-token order",
    )?;
    reserve_validation(
        &mut scratch.connection_ids,
        lease_count,
        "controller connection IDs",
    )?;
    for (lease_index, lease) in candidate.world.controller_leases.iter().enumerate() {
        let snake = candidate
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == lease.snake_id && snake.alive)
            .ok_or(StateError::UnknownLeaseSnake(lease.snake_id))?;
        validate_lease(lease, snake, candidate)?;
        scratch.lease_ids.push(lease.id);
        scratch.lease_snakes.push(lease.snake_id);
        scratch.resume_token_order.push(lease_index);
        if let Some(connection_id) = lease.connection_id {
            scratch.connection_ids.push(connection_id);
        }
    }
    if let Some(id) = first_duplicate_after_sort(&mut scratch.lease_ids) {
        return Err(StateError::DuplicateLeaseId(id));
    }
    if let Some(id) = first_duplicate_after_sort(&mut scratch.lease_snakes) {
        return Err(StateError::DuplicateLeaseSnake(id));
    }
    scratch.resume_token_order.sort_unstable_by(|left, right| {
        candidate.world.controller_leases[*left]
            .resume_token
            .cmp(&candidate.world.controller_leases[*right].resume_token)
    });
    if scratch.resume_token_order.windows(2).any(|pair| {
        candidate.world.controller_leases[pair[0]].resume_token
            == candidate.world.controller_leases[pair[1]].resume_token
    }) {
        return invalid("controller_lease.resume_token", "token must be unique");
    }
    if first_duplicate_after_sort(&mut scratch.connection_ids).is_some() {
        return invalid(
            "controller_lease.connection_id",
            "connection must own at most one lease",
        );
    }
    validate_next_after(
        "allocators.next_controller_lease_id",
        candidate.allocators.next_controller_lease_id,
        candidate
            .world
            .controller_leases
            .iter()
            .map(|lease| lease.id)
            .max()
            .unwrap_or(0),
        u64::MAX,
    )?;
    validate_next_after(
        "allocators.next_brain_id",
        candidate.allocators.next_brain_id,
        candidate
            .brains
            .iter()
            .map(|brain| brain.handle.id)
            .max()
            .unwrap_or(0),
        u64::MAX,
    )?;
    Ok(())
}

/// Sort one temporary identity list and return its first repeated value.
pub(super) fn first_duplicate_after_sort<T: Copy + Ord>(values: &mut [T]) -> Option<T> {
    values.sort_unstable();
    values
        .windows(2)
        .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
}

pub(super) fn reserve_validation<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), StateError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| StateError::AllocationFailed { context, required })?;
    }
    Ok(())
}

pub(super) fn validate_snake(
    index: usize,
    snake: &SnakeState,
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    validate_point("world.snakes.position", index, snake.position)?;
    validate_point(
        "world.snakes.previous_position",
        index,
        snake.previous_position,
    )?;
    for (field, value) in [
        ("world.snakes.direction", snake.direction),
        ("world.snakes.radius", snake.radius),
        ("world.snakes.speed", snake.speed),
        (
            "world.snakes.completed_step_heading_delta",
            snake.completed_step_heading_delta,
        ),
        (
            "world.snakes.completed_step_speed_delta",
            snake.completed_step_speed_delta,
        ),
        ("world.snakes.age_seconds", snake.age_seconds),
        ("world.snakes.food", snake.food),
        ("world.snakes.points", snake.points),
        ("world.snakes.target_length", snake.target_length),
        ("world.snakes.fitness", snake.fitness),
        (
            "world.snakes.control_accumulator_seconds",
            snake.control_accumulator_seconds,
        ),
        (
            "world.snakes.delivered_observation_points",
            snake.delivered_observation_points,
        ),
    ] {
        if !value.is_finite() {
            return Err(StateError::NonFinite { field, index });
        }
    }
    if snake.radius <= 0.0
        || snake.speed < 0.0
        || snake.age_seconds < 0.0
        || snake.target_length < 0.0
        || snake.control_accumulator_seconds < 0.0
        || !snake.turn.is_finite()
        || !snake.previous_turn.is_finite()
        || !snake.completed_step_applied_turn.is_finite()
        || !(-1.0..=1.0).contains(&snake.completed_step_applied_turn)
        || snake.completed_step_heading_delta.abs() > std::f64::consts::PI
        || !(-1.0..=1.0).contains(&snake.turn)
        || !(-1.0..=1.0).contains(&snake.previous_turn)
    {
        return invalid("world.snakes", "invalid scalar range");
    }
    match snake.kind {
        SnakeKind::Evolved => {
            let slot = snake
                .population_slot
                .ok_or_else(|| StateError::InvalidField {
                    field: "world.snakes.population_slot",
                    reason: "evolved snake requires a population slot".to_owned(),
                })?;
            let genome = candidate.population.get(slot as usize);
            if genome.is_none_or(|genome| Some(genome.brain) != snake.brain) {
                return invalid(
                    "world.snakes.brain",
                    "evolved snake mapping is inconsistent",
                );
            }
            if snake.baseline_slot.is_some() || snake.baseline_strategy.is_some() {
                return invalid(
                    "world.snakes.baseline",
                    "evolved snake cannot own baseline state",
                );
            }
        }
        SnakeKind::Baseline => {
            if snake.population_slot.is_some()
                || snake.baseline_slot.is_none()
                || snake.baseline_strategy.is_none()
            {
                return invalid(
                    "world.snakes.baseline",
                    "baseline snake requires only a stable slot and strategy",
                );
            }
        }
        _ if snake.population_slot.is_some()
            || snake.baseline_slot.is_some()
            || snake.baseline_strategy.is_some() =>
        {
            return invalid(
                "world.snakes.owner_slot",
                "population and baseline slots must match the snake kind",
            );
        }
        _ => {}
    }
    if snake.kind == SnakeKind::Resurrected && snake.brain.is_none() {
        return invalid(
            "world.snakes.brain",
            "retained neural snake requires an owned brain and origin",
        );
    }
    if let Some(handle) = snake.brain {
        let found = candidate.brains.iter().any(|brain| {
            brain.handle == handle
                && brain.owner == BrainOwner::Entity(snake.id)
                && (brain.retained_origin.is_some() == (snake.kind == SnakeKind::Resurrected))
        });
        let evolved_mapping = snake.kind == SnakeKind::Evolved
            && candidate.brains.iter().any(|brain| {
                brain.handle == handle && matches!(brain.owner, BrainOwner::PopulationSlot(_))
            });
        if !found && !evolved_mapping {
            return Err(StateError::InvalidBrainOwner(handle));
        }
    }
    Ok(())
}

pub(super) fn validate_lease(
    lease: &ControllerLease,
    snake: &SnakeState,
    candidate: &StateCandidate,
) -> Result<(), StateError> {
    if lease.id == 0 || lease.id == u64::MAX || lease.snake_id == 0 {
        return invalid(
            "controller_lease",
            "lease and snake identities must be positive",
        );
    }
    if snake.kind != SnakeKind::External {
        return invalid(
            "controller_lease.snake_id",
            "controller leases may target only external snakes",
        );
    }
    validate_text("controller_lease.scope", &lease.scope)?;
    if lease.identity_key.len() > 128 || lease.identity_key.contains('\0') {
        return invalid(
            "controller_lease.identity_key",
            "legacy identity exceeds 128 UTF-8 bytes or contains NUL",
        );
    }
    validate_text("controller_lease.resume_token", &lease.resume_token)?;
    if lease.scope != candidate.identity.run_id {
        return invalid(
            "controller_lease.scope",
            "lease is not scoped to the current run",
        );
    }
    if lease.connection_id == Some(0) {
        return invalid(
            "controller_lease.connection_id",
            "connection identity must be nonzero",
        );
    }
    if !lease.latest_action.turn.is_finite() || !(-1.0..=1.0).contains(&lease.latest_action.turn) {
        return invalid(
            "controller_lease.latest_action.turn",
            "turn must be in [-1, 1]",
        );
    }
    if lease.latest_action.arrival_sequence == 0 {
        return invalid(
            "controller_lease.latest_action.arrival_sequence",
            "arrival sequence must be positive",
        );
    }
    if lease.last_observed_at_ms < lease.latest_action.accepted_at_ms {
        return invalid(
            "controller_lease.last_observed_at_ms",
            "last observed wall time precedes the latest action",
        );
    }
    match lease.status {
        ControllerLeaseStatus::Connected => {
            if lease.connection_id.is_none()
                || lease.disconnected_at_ms.is_some()
                || lease.input_hold_expires_at_ms.is_some()
                || lease.grace_expires_at_ms.is_some()
                || lease.takeover_committed_at_ms.is_some()
            {
                return invalid(
                    "controller_lease",
                    "connected lease requires only a live connection",
                );
            }
        }
        ControllerLeaseStatus::HoldingLastInput => {
            let (Some(disconnected), Some(hold_deadline), Some(grace_deadline)) = (
                lease.disconnected_at_ms,
                lease.input_hold_expires_at_ms,
                lease.grace_expires_at_ms,
            ) else {
                return invalid(
                    "controller_lease",
                    "held-input lease requires disconnect, hold, and grace deadlines",
                );
            };
            let expected_hold_deadline = lease
                .latest_action
                .accepted_at_ms
                .checked_add(candidate.config.controller_input_hold_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller input-hold deadline",
                })?;
            let expected_grace_deadline = disconnected
                .checked_add(candidate.config.controller_disconnect_grace_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller grace deadline",
                })?;
            if lease.connection_id.is_some()
                || lease.takeover_committed_at_ms.is_some()
                || hold_deadline != expected_hold_deadline
                || hold_deadline <= disconnected
                || grace_deadline != expected_grace_deadline
                || lease.latest_action.accepted_at_ms > disconnected
                || lease.last_observed_at_ms < disconnected
                || snake.turn.to_bits() != lease.latest_action.turn.to_bits()
                || snake.input_boost != lease.latest_action.boost
            {
                return invalid(
                    "controller_lease",
                    "held-input ownership must be exclusive and preserve the last accepted action",
                );
            }
        }
        ControllerLeaseStatus::ReservedNeutral => {
            let (Some(disconnected), Some(hold_deadline), Some(grace_deadline)) = (
                lease.disconnected_at_ms,
                lease.input_hold_expires_at_ms,
                lease.grace_expires_at_ms,
            ) else {
                return invalid(
                    "controller_lease",
                    "neutral grace requires disconnect, hold, and grace deadlines",
                );
            };
            let expected_hold_deadline = lease
                .latest_action
                .accepted_at_ms
                .checked_add(candidate.config.controller_input_hold_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller input-hold deadline",
                })?;
            let expected_grace_deadline = disconnected
                .checked_add(candidate.config.controller_disconnect_grace_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller grace deadline",
                })?;
            if lease.connection_id.is_some()
                || lease.takeover_committed_at_ms.is_some()
                || hold_deadline != expected_hold_deadline
                || grace_deadline != expected_grace_deadline
                || lease.latest_action.accepted_at_ms > disconnected
                || lease.last_observed_at_ms < disconnected
                || snake.turn != 0.0
                || snake.input_boost
            {
                return invalid(
                    "controller_lease",
                    "neutral grace must be exclusive with neutral steering and boost input",
                );
            }
        }
        ControllerLeaseStatus::NeuralTakeover => {
            let (Some(disconnected), Some(hold_deadline), Some(grace_deadline), Some(takeover)) = (
                lease.disconnected_at_ms,
                lease.input_hold_expires_at_ms,
                lease.grace_expires_at_ms,
                lease.takeover_committed_at_ms,
            ) else {
                return invalid(
                    "controller_lease",
                    "neural takeover requires disconnect, hold, grace, and commit evidence",
                );
            };
            let expected_hold_deadline = lease
                .latest_action
                .accepted_at_ms
                .checked_add(candidate.config.controller_input_hold_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller input-hold deadline",
                })?;
            let expected_grace_deadline = disconnected
                .checked_add(candidate.config.controller_disconnect_grace_ms)
                .ok_or(StateError::ArithmeticOverflow {
                    context: "controller grace deadline",
                })?;
            if lease.connection_id.is_some()
                || hold_deadline != expected_hold_deadline
                || grace_deadline != expected_grace_deadline
                || takeover < grace_deadline
                || lease.latest_action.accepted_at_ms > disconnected
                || lease.last_observed_at_ms < takeover
                || snake.brain.is_none()
            {
                return invalid(
                    "controller_lease",
                    "neural takeover must occur once after grace and target a neural snake",
                );
            }
        }
    }
    Ok(())
}

pub(super) fn validate_generation_boundary(
    candidate: &StateCandidate,
    graph: &CompiledGraph,
) -> Result<(), StateError> {
    if matches!(
        candidate.phase,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart)
    ) && (candidate.generation.generation != 1 || candidate.generation.completed_step != 0)
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "run-start boundary must describe generation one before any committed step",
        });
    }
    if matches!(
        candidate.phase,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation)
    ) && (candidate.generation.generation == 1 || candidate.generation.completed_step == 0)
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason:
                "post-generation boundary must follow a completed step in generation two or later",
        });
    }
    if candidate.generation.elapsed_seconds != 0.0
        || candidate.generation.wall_accumulator_seconds != 0.0
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "elapsed time and wall accumulator must be zero",
        });
    }
    if candidate.fixed_step.ambient_pellet_accumulator != 0.0
        || candidate
            .fixed_step
            .sensor_generation
            .best_points_this_generation()
            != 0.0
        || !candidate.fixed_step.baseline_lifecycle.slots.is_empty()
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "fixed-step continuation must be reset before spawn or sensing",
        });
    }
    if candidate.world.ray_cursor != crate::engine::ray_work::RayQuotaCursor::default()
        || !candidate.world.snakes.is_empty()
        || !candidate.world.body_points.is_empty()
        || !candidate.world.pellets.is_empty()
        || !candidate.world.controller_leases.is_empty()
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "pre-spawn boundary cannot contain world entities or controller leases",
        });
    }
    if candidate.brains.len() != candidate.population.len() {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "boundary contains non-population brains or misses population brains",
        });
    }
    if graph.total_state_size != 0
        && candidate
            .brains
            .iter()
            .any(|brain| brain.recurrent.iter().any(|value| value.to_bits() != 0))
    {
        return Err(StateError::DirtyGenerationBoundary {
            reason: "recurrent state must be zero before spawn, sensing, or inference",
        });
    }
    Ok(())
}

pub(super) fn validate_entity_id_domain(
    kind: &'static str,
    id: u64,
    start: u64,
    exhausted: u64,
) -> Result<(), StateError> {
    if id < start || id >= exhausted {
        return invalid(kind, "identity is outside its checked domain");
    }
    Ok(())
}
