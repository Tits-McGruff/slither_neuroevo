//! Private in-place evolved replacement at the shared newborn boundary.

use super::evolved_lives::{
    EvolvedExperimentConfig, EvolvedGenerationState, EvolvedLifeError, EvolvedLifeStatus,
    RecurrentLifeMode, RespawnRngMode,
};
use super::fixed_step::{compact_world_bodies, FixedStepPrefixConfig, FixedStepPrefixError};
use super::physics::PhysicsStepKey;
use super::rng::{SerializedRngState, StatefulRng};
use super::spawn::{EvolvedPlacementStatus, SpawnWorkspace};
use super::state::{
    AllocatorState, BrainHandle, RngStateBundle, SnakeKind, WorldPoint, WorldState,
};
use sha2::{Digest, Sha256};

/// Exact immutable ownership change produced by one successful placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EvolvedReplacementRecord {
    pub slot: u32,
    pub old_id: u64,
    pub new_id: u64,
    pub old_frame_id: u32,
    pub new_frame_id: u32,
    pub brain: BrainHandle,
}

/// Private witness, sealed only after the complete physics/lifecycle result.
#[derive(Debug, Default)]
pub(crate) struct EvolvedReplacementAuthorityProof {
    key: Option<PhysicsStepKey>,
    records: Vec<EvolvedReplacementRecord>,
    source_rng: [u8; 32],
    final_rng: [u8; 32],
    sealed: bool,
}

impl EvolvedReplacementAuthorityProof {
    pub(crate) fn record(&self, slot: u32) -> Option<&EvolvedReplacementRecord> {
        self.records.iter().find(|record| record.slot == slot)
    }

    pub(crate) fn matches(
        &self,
        key: PhysicsStepKey,
        source: &super::state::StateCandidate,
        world: &WorldState,
        rng: &RngStateBundle,
        allocators: &AllocatorState,
    ) -> bool {
        if !self.sealed
            || self.key != Some(key)
            || self.source_rng != respawn_rng_digest(&source.rng)
            || self.final_rng != respawn_rng_digest(rng)
        {
            return false;
        }
        for record in &self.records {
            let Some(old) = source.world.snakes.iter().find(|snake| {
                snake.population_slot == Some(record.slot) && snake.kind == SnakeKind::Evolved
            }) else {
                return false;
            };
            let Some(new) = world.snakes.iter().find(|snake| {
                snake.population_slot == Some(record.slot) && snake.kind == SnakeKind::Evolved
            }) else {
                return false;
            };
            if old.alive
                || old.id != record.old_id
                || old.frame_v1_id != record.old_frame_id
                || old.brain != Some(record.brain)
                || new.id != record.new_id
                || new.frame_v1_id != record.new_frame_id
                || new.brain != Some(record.brain)
                || record.old_frame_id == record.new_frame_id
                || record.new_id < source.allocators.next_entity_id
                || record.new_id >= allocators.next_entity_id
            {
                return false;
            }
        }
        true
    }
}

/// Retained bounded placement, compaction and reset scratch; never owns genomes.
#[derive(Debug, Default)]
pub(crate) struct EvolvedReplacementWorkspace {
    spawn: SpawnWorkspace,
    due: Vec<u32>,
    body: Vec<WorldPoint>,
    compacted: Vec<WorldPoint>,
    resets: Vec<BrainHandle>,
    proof: EvolvedReplacementAuthorityProof,
    active: bool,
}

impl EvolvedReplacementWorkspace {
    pub(crate) fn clear(&mut self) {
        self.active = false;
        self.due.clear();
        self.body.clear();
        self.resets.clear();
        self.proof.records.clear();
        self.proof.key = None;
        self.proof.sealed = false;
    }

    /// Resolve a bounded due batch against the current staged world; successes
    /// become immediate obstacles, failures consume draws but never reserve IDs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        key: PhysicsStepKey,
        world: &mut WorldState,
        rng: &mut RngStateBundle,
        allocators: &mut AllocatorState,
        lives: &mut EvolvedGenerationState,
        policy: EvolvedExperimentConfig,
        config: FixedStepPrefixConfig,
        source_elapsed: f64,
        initial_accumulator: f64,
    ) -> Result<(), FixedStepPrefixError> {
        self.clear();
        let target_step = key.source_completed_step().checked_add(1).ok_or(
            FixedStepPrefixError::ArithmeticOverflow {
                context: "evolved target step",
            },
        )?;
        lives.placement_work = super::evolved_lives::EvolvedPlacementWork {
            step: target_step,
            ..Default::default()
        };
        if !policy.enabled {
            return Ok(());
        }
        self.active = true;
        self.proof.key = Some(key);
        self.proof.source_rng = respawn_rng_digest(rng);
        lives
            .schedule_deaths(
                policy,
                rng,
                key.source_completed_step(),
                source_elapsed,
                config.fixed_dt,
            )
            .map_err(FixedStepPrefixError::Evolved)?;
        let eligible = lives
            .collect_due(target_step, config.fixed_dt, &mut self.due)
            .map_err(FixedStepPrefixError::Evolved)?;
        lives.placement_work.eligible_slots = u16::try_from(eligible)
            .map_err(|_| FixedStepPrefixError::Evolved(EvolvedLifeError::Overflow))?;
        reserve(
            &mut self.proof.records,
            self.due.len(),
            "evolved replacement records",
        )?;
        reserve(&mut self.resets, self.due.len(), "evolved recurrent resets")?;
        reserve(
            &mut self.body,
            config.baseline_spawn.snake_start_len,
            "evolved newborn body",
        )?;
        let mut body_count = world.snakes.iter().try_fold(0usize, |count, snake| {
            count
                .checked_add(snake.body.len)
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "evolved body count",
                })
        })?;
        let mut candidates = 0usize;
        let mut geometry = 0usize;
        for &slot in &self.due {
            if candidates == 2048 || geometry == 250_000 {
                break;
            }
            let index = world
                .snakes
                .iter()
                .position(|snake| {
                    snake.kind == SnakeKind::Evolved && snake.population_slot == Some(slot)
                })
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            let old = &world.snakes[index];
            let owner = &lives.slots[slot as usize];
            if old.alive
                || old.id != owner.physical_id
                || !matches!(
                    owner.status,
                    EvolvedLifeStatus::Waiting | EvolvedLifeStatus::PendingPlacement
                )
            {
                return Err(FixedStepPrefixError::Evolved(EvolvedLifeError::Identity));
            }
            let source_rng = selected_rng(rng, slot, policy.rng_mode)?;
            let (status, cursor, placement, next_rng, used_candidates, used_geometry) = {
                let attempt = self
                    .spawn
                    .prepare_evolved_replacement(
                        world,
                        slot,
                        &source_rng,
                        config.baseline_spawn,
                        owner.fallback_cursor,
                        2048 - candidates,
                        250_000 - geometry,
                        config.baseline_spawn.snake_start_len,
                    )
                    .map_err(|error| FixedStepPrefixError::Spawn(Box::new(error)))?;
                let spawns = attempt.spawns();
                self.body.clear();
                let placement = spawns.placements().first().copied();
                if let Some(placement) = &placement {
                    self.body.extend_from_slice(
                        spawns
                            .body_for(placement)
                            .ok_or(FixedStepPrefixError::InternalShapeMismatch)?,
                    );
                }
                (
                    attempt.status(),
                    attempt.fallback_cursor(),
                    placement,
                    spawns.next_rng().clone(),
                    spawns.diagnostics().candidates_examined,
                    spawns.diagnostics().geometry_checks,
                )
            };
            lives.placement_work.attempted_slots += 1;
            candidates += used_candidates;
            geometry += used_geometry;
            lives.placement_work.candidates = u16::try_from(candidates)
                .map_err(|_| FixedStepPrefixError::Evolved(EvolvedLifeError::Overflow))?;
            lives.placement_work.geometry_checks = u32::try_from(geometry)
                .map_err(|_| FixedStepPrefixError::Evolved(EvolvedLifeError::Overflow))?;
            publish_rng(rng, slot, policy.rng_mode, &next_rng)?;
            if status != EvolvedPlacementStatus::Placed {
                match status {
                    EvolvedPlacementStatus::PendingNoPlacement => {
                        lives.placement_work.pending_no_placement += 1
                    }
                    EvolvedPlacementStatus::PendingWorkLimit => {
                        lives.placement_work.pending_work_limit += 1
                    }
                    EvolvedPlacementStatus::Placed => unreachable!(),
                }
                lives
                    .record_failed_attempt(
                        slot,
                        target_step,
                        source_elapsed + config.fixed_dt,
                        cursor,
                    )
                    .map_err(FixedStepPrefixError::Evolved)?;
                continue;
            }
            let placement = placement.ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            let old = world.snakes[index].clone();
            let final_count = body_count
                .checked_sub(old.body.len)
                .and_then(|count| count.checked_add(self.body.len()))
                .ok_or(FixedStepPrefixError::ArithmeticOverflow {
                    context: "evolved compacted body count",
                })?;
            if final_count > config.maximum_body_points {
                return Err(FixedStepPrefixError::BodyCapacityExceeded {
                    required: final_count,
                    maximum: config.maximum_body_points,
                });
            }
            let temporary = world.body_points.len().checked_add(self.body.len()).ok_or(
                FixedStepPrefixError::ArithmeticOverflow {
                    context: "evolved temporary body count",
                },
            )?;
            reserve(&mut world.body_points, temporary, "evolved appended body")?;
            let id = allocators
                .reserve_entity_ids(1)
                .map_err(|error| FixedStepPrefixError::Allocator(Box::new(error)))?
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?
                .first;
            let frame = allocators
                .reserve_frame_v1_ids(1, world.snakes.iter().map(|snake| snake.frame_v1_id))
                .map_err(|error| FixedStepPrefixError::Allocator(Box::new(error)))?
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?
                .first;
            let brain = old
                .brain
                .ok_or(FixedStepPrefixError::InternalShapeMismatch)?;
            let owner = &mut lives.slots[slot as usize];
            let wallet = owner
                .begin_life(id, self.body.len(), policy.carried_wallet)
                .map_err(FixedStepPrefixError::Evolved)?;
            owner.pending_recurrent_reset = policy.recurrent_state == RecurrentLifeMode::Reset;
            let mut newborn = old.clone();
            newborn.id = id;
            newborn.frame_v1_id = frame;
            newborn.alive = true;
            newborn.position = placement.head;
            newborn.previous_position = placement.head;
            newborn.direction = placement.direction;
            newborn.radius = config.baseline_spawn.snake_radius;
            newborn.speed = config.baseline_snake_base_speed;
            newborn.boost = false;
            newborn.age_seconds = 0.0;
            newborn.food = 0.0;
            newborn.points = wallet;
            newborn.kills = 0;
            newborn.target_length = self.body.len() as f64;
            newborn.fitness = 0.0;
            newborn.turn = 0.0;
            newborn.previous_turn = 0.0;
            newborn.input_boost = false;
            newborn.previous_input_boost = false;
            newborn.control_accumulator_seconds = initial_accumulator;
            newborn.delivered_observation_points = wallet;
            newborn.observation_delivered = false;
            newborn.body = super::state::BodyRange {
                start: world.body_points.len(),
                len: self.body.len(),
            };
            let (age, points) = config
                .accounting
                .advance_live_snake(&newborn, config.fixed_dt)?;
            newborn.age_seconds = age;
            newborn.points = points;
            owner
                .alive_award(
                    config.fixed_dt,
                    config.fixed_dt * config.accounting.points_per_second_alive,
                )
                .map_err(FixedStepPrefixError::Evolved)?;
            world.body_points.extend_from_slice(&self.body);
            world.snakes[index] = newborn;
            self.proof.records.push(EvolvedReplacementRecord {
                slot,
                old_id: old.id,
                new_id: id,
                old_frame_id: old.frame_v1_id,
                new_frame_id: frame,
                brain,
            });
            if owner.pending_recurrent_reset {
                self.resets.push(brain);
            }
            body_count = final_count;
            lives.placement_work.placed_slots += 1;
        }
        lives
            .placement_work
            .validate()
            .map_err(FixedStepPrefixError::Evolved)?;
        if !self.proof.records.is_empty() {
            for pellet in &mut world.pellets {
                if pellet.owner.is_some_and(|owner| {
                    self.proof
                        .records
                        .iter()
                        .any(|record| record.old_id == owner)
                }) {
                    pellet.owner = None;
                }
            }
            compact_world_bodies(world, &mut self.compacted, config.maximum_body_points)?;
        }
        self.resets.sort_unstable();
        lives
            .accrue_pending_time(config.fixed_dt)
            .map_err(FixedStepPrefixError::Evolved)?;
        Ok(())
    }

    pub(crate) fn recurrent_resets(&self) -> &[BrainHandle] {
        &self.resets
    }

    pub(crate) fn seal(
        &mut self,
        key: PhysicsStepKey,
        rng: &RngStateBundle,
    ) -> Result<(), EvolvedLifeError> {
        if !self.active {
            return Ok(());
        }
        if self.proof.key != Some(key) {
            return Err(EvolvedLifeError::Identity);
        }
        self.proof.final_rng = respawn_rng_digest(rng);
        self.proof.sealed = true;
        Ok(())
    }

    pub(crate) fn proof(&self) -> Option<&EvolvedReplacementAuthorityProof> {
        (self.active && self.proof.sealed).then_some(&self.proof)
    }
}

fn selected_rng(
    rng: &RngStateBundle,
    slot: u32,
    mode: RespawnRngMode,
) -> Result<SerializedRngState, FixedStepPrefixError> {
    match mode {
        RespawnRngMode::World => Ok(rng.world.clone()),
        RespawnRngMode::PerSlot => rng
            .evolved_respawns
            .get(slot as usize)
            .filter(|stream| stream.slot == slot)
            .ok_or(FixedStepPrefixError::Evolved(EvolvedLifeError::InvalidRng))?
            .runtime()
            .map(|runtime| runtime.export_state())
            .map_err(|_| FixedStepPrefixError::Evolved(EvolvedLifeError::InvalidRng)),
    }
}

fn publish_rng(
    rng: &mut RngStateBundle,
    slot: u32,
    mode: RespawnRngMode,
    next: &SerializedRngState,
) -> Result<(), FixedStepPrefixError> {
    match mode {
        RespawnRngMode::World => rng.world.clone_from(next),
        RespawnRngMode::PerSlot => {
            let runtime = StatefulRng::from_state(next)
                .map_err(|_| FixedStepPrefixError::Evolved(EvolvedLifeError::InvalidRng))?;
            rng.evolved_respawns
                .get_mut(slot as usize)
                .filter(|stream| stream.slot == slot)
                .ok_or(FixedStepPrefixError::Evolved(EvolvedLifeError::InvalidRng))?
                .uniform_state = runtime.uniform_state();
        }
    }
    Ok(())
}

fn respawn_rng_digest(rng: &RngStateBundle) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"slither-evolved-respawn-rng/v1\0");
    hash.update(rng.version.to_le_bytes());
    hash.update((rng.evolved_respawns.len() as u64).to_le_bytes());
    for stream in &rng.evolved_respawns {
        hash.update(stream.slot.to_le_bytes());
        hash.update(stream.uniform_state.to_le_bytes());
    }
    hash.finalize().into()
}

fn reserve<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), FixedStepPrefixError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| FixedStepPrefixError::AllocationFailed { context, required })?;
    }
    Ok(())
}
