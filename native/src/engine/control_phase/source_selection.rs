//! Stable ID order, lookups and source checks on borrowed control-boundary inputs.

use super::*;

pub(super) fn prepare_orders(
    world: &WorldState,
    brains: &[BrainRuntimeState],
    snake_order: &mut Vec<usize>,
    brain_order: &mut Vec<usize>,
    lease_order: &mut Vec<usize>,
    config: ControlPhaseConfig,
) -> Result<(), ControlPhaseError> {
    reserve_for(snake_order, world.snakes.len(), "control snake order")?;
    snake_order.extend(0..world.snakes.len());
    snake_order.sort_unstable_by_key(|index| world.snakes[*index].id);
    for pair in snake_order.windows(2) {
        if world.snakes[pair[0]].id == world.snakes[pair[1]].id {
            return Err(ControlPhaseError::DuplicateSnakeId(
                world.snakes[pair[0]].id,
            ));
        }
    }
    if snake_order
        .first()
        .is_some_and(|index| world.snakes[*index].id == 0)
    {
        return Err(ControlPhaseError::DuplicateSnakeId(0));
    }

    reserve_for(brain_order, brains.len(), "control brain order")?;
    brain_order.extend(0..brains.len());
    brain_order.sort_unstable_by_key(|index| brains[*index].handle);
    for pair in brain_order.windows(2) {
        if brains[pair[0]].handle == brains[pair[1]].handle {
            return Err(ControlPhaseError::DuplicateBrain(brains[pair[0]].handle));
        }
    }

    if world.controller_leases.len() > config.maximum_snakes {
        return Err(ControlPhaseError::CapacityExceeded {
            buffer: "controller leases",
            required: world.controller_leases.len(),
            maximum: config.maximum_snakes,
        });
    }
    reserve_for(
        lease_order,
        world.controller_leases.len(),
        "control lease order",
    )?;
    lease_order.extend(0..world.controller_leases.len());
    lease_order.sort_unstable_by_key(|index| world.controller_leases[*index].snake_id);
    for pair in lease_order.windows(2) {
        if world.controller_leases[pair[0]].snake_id == world.controller_leases[pair[1]].snake_id {
            return Err(ControlPhaseError::DuplicateControllerSnake(
                world.controller_leases[pair[0]].snake_id,
            ));
        }
    }
    Ok(())
}

pub(super) fn find_lease_index(
    world: &WorldState,
    order: &[usize],
    snake_id: u64,
) -> Option<usize> {
    order
        .binary_search_by_key(&snake_id, |index| world.controller_leases[*index].snake_id)
        .ok()
        .map(|position| order[position])
}

pub(super) fn find_brain_index(
    brains: &[BrainRuntimeState],
    order: &[usize],
    handle: BrainHandle,
) -> Option<usize> {
    order
        .binary_search_by_key(&handle, |index| brains[*index].handle)
        .ok()
        .map(|position| order[position])
}

pub(super) fn find_control(
    controls: &[PreparedControlUpdate],
    snake_id: u64,
) -> Result<&PreparedControlUpdate, ControlPhaseError> {
    controls
        .binary_search_by_key(&snake_id, |update| update.snake_id)
        .ok()
        .and_then(|index| controls.get(index))
        .ok_or(ControlPhaseError::CommitShapeMismatch {
            field: "control result lookup",
        })
}

pub(super) fn validate_transition_control(
    proposal: ControllerBoundaryProposal,
    update: &PreparedControlUpdate,
) -> Result<(), ControlPhaseError> {
    let matches = match proposal.source() {
        ExternalControlSource::HeldAction { turn, boost } => {
            update.source == SelectedControlSource::ExternalHeld
                && update.turn.to_bits() == turn.to_bits()
                && update.boost == boost
        }
        ExternalControlSource::ReservedNeutral => {
            update.source == SelectedControlSource::ExternalReservedNeutral
                && update.turn.to_bits() == 0.0_f32.to_bits()
                && !update.boost
        }
        ExternalControlSource::NeuralTakeover if proposal.begins_neural_takeover() => {
            update.source == SelectedControlSource::NeuralTakeover
        }
        ExternalControlSource::NeuralTakeover => matches!(
            update.source,
            SelectedControlSource::NeuralEvaluated | SelectedControlSource::NeuralHeld
        ),
    };
    if matches {
        Ok(())
    } else {
        Err(ControlPhaseError::CommitShapeMismatch {
            field: "controller transition/control result",
        })
    }
}

pub(super) fn next_neural_boundary(
    source_accumulator: f64,
    fixed_dt: f64,
    interval: f64,
    force_due: bool,
) -> Result<(bool, f64), ControlPhaseError> {
    if !source_accumulator.is_finite()
        || !(0.0..=MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS).contains(&source_accumulator)
    {
        return Err(ControlPhaseError::InvalidCadenceState(source_accumulator));
    }
    let first_action_pending =
        source_accumulator.to_bits() == MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS.to_bits();
    let base = if force_due || first_action_pending {
        0.0
    } else {
        source_accumulator
    };
    let accumulated = base + fixed_dt;
    if !accumulated.is_finite() {
        return Err(ControlPhaseError::InvalidCadenceState(accumulated));
    }
    let due = force_due || first_action_pending || accumulated >= interval;
    let next = if due {
        accumulated % interval
    } else {
        accumulated
    };
    if !next.is_finite() || next < 0.0 || next >= interval {
        return Err(ControlPhaseError::InvalidCadenceState(next));
    }
    Ok((due, next))
}
