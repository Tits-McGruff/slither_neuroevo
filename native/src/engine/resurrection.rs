//! Atomic Hall-of-Fame resurrection inside the Rust-owned running authority.

use super::*;
use crate::engine::spawn::{SpawnDomain, SpawnKey, SpawnRequest, SpawnWorkspace};

/// Browser-visible result of one committed retained-winner resurrection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResurrectionPublication {
    /// Exact internal identity from the dedicated resurrected domain.
    pub snake_id: u64,
    /// Exactly representable frame-v1 identity returned to Protocol 2.
    pub frame_v1_id: u32,
}

impl AuthoritativeState {
    /// Add one retained winner without copying the evolved population or existing world.
    pub fn resurrect_hall_of_fame(
        &mut self,
        weights: Box<[f32]>,
        limits: RunningStepWorkLimits,
    ) -> Result<ResurrectionPublication, String> {
        if self.candidate.phase != AuthorityPhase::Running {
            return Err("Hall-of-Fame resurrection requires a running authority".into());
        }
        if weights.len() != self.graph.compiled().total_parameters
            || weights.iter().any(|value| !value.is_finite())
        {
            return Err("Hall-of-Fame weights do not match the active graph".into());
        }
        let config = self
            .running_step_config(limits)
            .map_err(|error| error.to_string())?
            .world_step
            .external_replacement;
        let source = &self.candidate;
        let required_snakes = source.world.snakes.len().saturating_add(1);
        let required_brains = source.brains.len().saturating_add(1);
        let required_body = source
            .world
            .body_points
            .len()
            .saturating_add(config.spawn.snake_start_len);
        if required_snakes > config.maximum_snakes
            || required_brains > config.maximum_brains
            || required_body > config.maximum_body_points
        {
            return Err("Hall-of-Fame resurrection exceeds admitted world capacity".into());
        }

        let mut next_allocators = source.allocators.clone();
        let snake_id = next_allocators
            .reserve_resurrected_ids(1)
            .map_err(|error| error.to_string())?
            .ok_or("resurrected identity allocation failed")?
            .first;
        let frame_v1_id = next_allocators
            .reserve_frame_v1_ids(1)
            .map_err(|error| error.to_string())?
            .ok_or("frame identity allocation failed")?
            .first;
        let brain_handle = BrainHandle {
            id: next_allocators
                .reserve_brain_ids(1)
                .map_err(|error| error.to_string())?
                .ok_or("brain identity allocation failed")?
                .first,
            epoch: source.generation.population_epoch,
        };
        let request = [SpawnRequest {
            key: SpawnKey {
                domain: SpawnDomain::Resurrected,
                slot: snake_id,
            },
        }];
        let mut workspace = SpawnWorkspace::default();
        let prepared = workspace
            .prepare(
                &source.world,
                &request,
                &source.rng.world,
                config.spawn,
                config.spawn.snake_start_len,
            )
            .map_err(|error| error.to_string())?;
        let placement = prepared
            .placements()
            .first()
            .ok_or("resurrection placement is missing")?;
        let body = prepared
            .body_for(placement)
            .ok_or("resurrection body is missing")?
            .to_vec();
        let next_world_rng = prepared.next_rng().clone();
        let body_start = source.world.body_points.len();
        let snake = SnakeState {
            id: snake_id,
            frame_v1_id,
            kind: SnakeKind::Resurrected,
            alive: true,
            population_slot: None,
            brain: Some(brain_handle),
            baseline_slot: None,
            baseline_strategy: None,
            position: placement.head,
            previous_position: placement.head,
            direction: placement.direction,
            radius: config.spawn.snake_radius,
            speed: config.snake_base_speed,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: config.spawn.snake_start_len as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: body_start,
                len: body.len(),
            },
            skin: 1,
        };
        let brain = BrainRuntimeState {
            handle: brain_handle,
            owner: BrainOwner::Entity(snake_id),
            non_population_weights: Some(weights),
            recurrent: vec![0.0; self.graph.compiled().total_state_size].into_boxed_slice(),
        };

        let candidate = &mut self.candidate;
        candidate
            .world
            .snakes
            .try_reserve_exact(1)
            .map_err(|error| error.to_string())?;
        candidate
            .brains
            .try_reserve_exact(1)
            .map_err(|error| error.to_string())?;
        candidate
            .world
            .body_points
            .try_reserve_exact(body.len())
            .map_err(|error| error.to_string())?;
        candidate.world.body_points.extend_from_slice(&body);
        candidate.world.snakes.push(snake);
        candidate.brains.push(brain);
        let old_allocators = std::mem::replace(&mut candidate.allocators, next_allocators);
        let old_world_rng = std::mem::replace(&mut candidate.rng.world, next_world_rng);
        let validation = validate_running_mutable_state(
            candidate,
            self.graph.compiled(),
            &mut self.world_validation,
        )
        .and_then(|()| estimate_state_memory(candidate, &self.graph));
        if let Err(error) = validation {
            candidate.allocators = old_allocators;
            candidate.rng.world = old_world_rng;
            candidate.brains.pop();
            candidate.world.snakes.pop();
            candidate.world.body_points.truncate(body_start);
            return Err(error.to_string());
        }
        let memory = validation.expect("validated resurrection memory");
        if memory.total_bytes > self.memory_ceiling_bytes {
            candidate.allocators = old_allocators;
            candidate.rng.world = old_world_rng;
            candidate.brains.pop();
            candidate.world.snakes.pop();
            candidate.world.body_points.truncate(body_start);
            return Err("Hall-of-Fame resurrection exceeds admitted authority memory".into());
        }
        self.memory = memory;
        Ok(ResurrectionPublication {
            snake_id,
            frame_v1_id,
        })
    }
}
