//! Immutable state fingerprints for prepared replacement proofs.

use super::*;

pub(crate) fn authority_world_digest(world: &WorldState) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(world.ray_cursor.witness_bytes());
    hash.update(b"slither-external-replacement-world-v1\0");
    hash_usize(&mut hash, world.snakes.len());
    for snake in &world.snakes {
        hash.update(snake.id.to_le_bytes());
        hash.update(snake.frame_v1_id.to_le_bytes());
        hash.update([snake_kind_tag(snake.kind), u8::from(snake.alive)]);
        hash_option_u32(&mut hash, snake.population_slot);
        hash_option_brain(&mut hash, snake.brain);
        hash_option_u32(&mut hash, snake.baseline_slot);
        hash.update([snake
            .baseline_strategy
            .map_or(u8::MAX, baseline_strategy_tag)]);
        hash_point(&mut hash, snake.position);
        hash_point(&mut hash, snake.previous_position);
        hash.update(snake.direction.to_bits().to_le_bytes());
        hash.update(snake.radius.to_bits().to_le_bytes());
        hash.update(snake.speed.to_bits().to_le_bytes());
        hash.update([u8::from(snake.boost)]);
        hash.update(snake.age_seconds.to_bits().to_le_bytes());
        hash.update(snake.food.to_bits().to_le_bytes());
        hash.update(snake.points.to_bits().to_le_bytes());
        hash.update(snake.kills.to_le_bytes());
        hash.update(snake.target_length.to_bits().to_le_bytes());
        hash.update(snake.fitness.to_bits().to_le_bytes());
        hash.update(snake.turn.to_bits().to_le_bytes());
        hash.update(snake.previous_turn.to_bits().to_le_bytes());
        hash.update([
            u8::from(snake.input_boost),
            u8::from(snake.previous_input_boost),
        ]);
        hash.update(snake.control_accumulator_seconds.to_bits().to_le_bytes());
        hash.update(snake.delivered_observation_points.to_bits().to_le_bytes());
        hash.update([u8::from(snake.observation_delivered)]);
        hash.update(snake.completed_step_heading_delta.to_bits().to_le_bytes());
        hash.update(snake.completed_step_speed_delta.to_bits().to_le_bytes());
        hash.update(snake.completed_step_applied_turn.to_bits().to_le_bytes());
        hash_usize(&mut hash, snake.body.start);
        hash_usize(&mut hash, snake.body.len);
        hash.update(snake.skin.to_le_bytes());
    }
    hash_usize(&mut hash, world.body_points.len());
    for point in &world.body_points {
        hash_point(&mut hash, *point);
    }
    hash_usize(&mut hash, world.pellets.len());
    for pellet in &world.pellets {
        hash.update(pellet.id.to_le_bytes());
        hash_point(&mut hash, pellet.position);
        hash.update(pellet.value.to_bits().to_le_bytes());
        hash.update(pellet.kind.to_le_bytes());
        hash.update(pellet.color.to_le_bytes());
        hash_option_u64(&mut hash, pellet.owner);
    }
    hash_usize(&mut hash, world.controller_leases.len());
    for lease in &world.controller_leases {
        if let Some(reward) = &lease.reward {
            reward.hash_into(&mut hash);
        }
        hash.update(lease.id.to_le_bytes());
        hash.update(lease.snake_id.to_le_bytes());
        hash.update([controller_kind_tag(lease.kind)]);
        hash_option_u64(&mut hash, lease.connection_id);
        hash_string(&mut hash, &lease.scope);
        hash_string(&mut hash, &lease.identity_key);
        hash_string(&mut hash, &lease.resume_token);
        hash.update([controller_status_tag(lease.status)]);
        hash.update(lease.latest_action.turn.to_bits().to_le_bytes());
        hash.update([u8::from(lease.latest_action.boost)]);
        hash.update(lease.latest_action.client_tick.to_le_bytes());
        hash.update([u8::from(lease.latest_action.is_explicit)]);
        hash.update(lease.latest_action.arrival_sequence.to_le_bytes());
        hash.update(lease.latest_action.accepted_at_ms.to_le_bytes());
        hash.update(lease.last_observed_at_ms.to_le_bytes());
        hash_option_u64(&mut hash, lease.disconnected_at_ms);
        hash_option_u64(&mut hash, lease.input_hold_expires_at_ms);
        hash_option_u64(&mut hash, lease.grace_expires_at_ms);
        hash_option_u64(&mut hash, lease.takeover_committed_at_ms);
    }
    hash.finalize().into()
}

pub(super) fn authority_rng_digest(rng: &RngStateBundle) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"slither-external-replacement-rng-v1\0");
    hash.update(rng.version.to_le_bytes());
    hash_serialized_rng(&mut hash, &rng.world);
    hash_serialized_rng(&mut hash, &rng.evolution);
    hash_serialized_rng(&mut hash, &rng.external_controller);
    hash_usize(&mut hash, rng.baselines.len());
    for baseline in &rng.baselines {
        hash.update(baseline.slot.to_le_bytes());
        hash_serialized_rng(&mut hash, &baseline.state);
    }
    if rng.version >= crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION {
        hash_usize(&mut hash, rng.evolved_respawns.len());
        for stream in &rng.evolved_respawns {
            hash.update(stream.slot.to_le_bytes());
            hash.update(stream.uniform_state.to_le_bytes());
        }
    }
    if rng.version == crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION {
        hash_usize(&mut hash, rng.baseline_behaviors.len());
        for stream in &rng.baseline_behaviors {
            hash.update(stream.slot.to_le_bytes());
            hash.update(stream.uniform_state.to_le_bytes());
        }
    }
    hash.finalize().into()
}

pub(super) fn authority_brain_digest(brains: &[BrainRuntimeState]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"slither-external-replacement-brains-v2\0");
    hash_usize(&mut hash, brains.len());
    for brain in brains {
        hash.update(brain.handle.id.to_le_bytes());
        hash.update(brain.handle.epoch.to_le_bytes());
        match brain.owner {
            BrainOwner::PopulationSlot(slot) => {
                hash.update([0]);
                hash.update(slot.to_le_bytes());
            }
            BrainOwner::Entity(id) => {
                hash.update([1]);
                hash.update(id.to_le_bytes());
            }
        }
        match brain.non_population_weights.as_deref() {
            Some(weights) => {
                hash.update([1]);
                hash_f32_slice(&mut hash, weights);
            }
            None => hash.update([0]),
        }
        match &brain.retained_origin {
            Some(origin) => {
                hash.update([1]);
                origin.hash_into(&mut hash);
            }
            None => hash.update([0]),
        }
        hash_f32_slice(&mut hash, &brain.recurrent);
    }
    hash.finalize().into()
}

pub(super) fn hash_serialized_rng(hash: &mut Sha256, rng: &crate::engine::rng::SerializedRngState) {
    hash_string(hash, &rng.algorithm);
    hash.update(rng.version.to_le_bytes());
    hash_string(hash, &rng.state_hex);
    hash_string(hash, &rng.gaussian_algorithm);
    hash.update(rng.gaussian_version.to_le_bytes());
    hash.update([u8::from(rng.gaussian_spare_valid)]);
    match rng.gaussian_spare_hex.as_deref() {
        Some(value) => {
            hash.update([1]);
            hash_string(hash, value);
        }
        None => hash.update([0]),
    }
}

pub(super) fn hash_f32_slice(hash: &mut Sha256, values: &[f32]) {
    hash_usize(hash, values.len());
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
}

pub(super) fn hash_string(hash: &mut Sha256, value: &str) {
    hash_usize(hash, value.len());
    hash.update(value.as_bytes());
}

pub(super) fn hash_usize(hash: &mut Sha256, value: usize) {
    hash.update((value as u64).to_le_bytes());
}

pub(super) fn hash_point(hash: &mut Sha256, point: WorldPoint) {
    hash.update(point.x.to_bits().to_le_bytes());
    hash.update(point.y.to_bits().to_le_bytes());
}

pub(super) fn hash_option_u32(hash: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.to_le_bytes());
        }
        None => hash.update([0]),
    }
}

pub(super) fn hash_option_u64(hash: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.to_le_bytes());
        }
        None => hash.update([0]),
    }
}

pub(super) fn hash_option_brain(hash: &mut Sha256, value: Option<BrainHandle>) {
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.id.to_le_bytes());
            hash.update(value.epoch.to_le_bytes());
        }
        None => hash.update([0]),
    }
}

pub(super) const fn snake_kind_tag(kind: SnakeKind) -> u8 {
    match kind {
        SnakeKind::Evolved => 0,
        SnakeKind::External => 1,
        SnakeKind::Baseline => 2,
        SnakeKind::Resurrected => 3,
    }
}

pub(super) const fn baseline_strategy_tag(
    strategy: crate::engine::state::BaselineStrategyState,
) -> u8 {
    match strategy {
        crate::engine::state::BaselineStrategyState::Roam => 0,
        crate::engine::state::BaselineStrategyState::Seek => 1,
        crate::engine::state::BaselineStrategyState::Avoid => 2,
        crate::engine::state::BaselineStrategyState::Boost => 3,
    }
}

pub(super) const fn controller_kind_tag(kind: ControllerKind) -> u8 {
    match kind {
        ControllerKind::Player => 0,
        ControllerKind::ReinforcementLearning => 1,
    }
}

pub(super) const fn controller_status_tag(status: ControllerLeaseStatus) -> u8 {
    match status {
        ControllerLeaseStatus::Connected => 0,
        ControllerLeaseStatus::HoldingLastInput => 1,
        ControllerLeaseStatus::ReservedNeutral => 2,
        ControllerLeaseStatus::NeuralTakeover => 3,
    }
}
