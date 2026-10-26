//! Atomic retained neural spawn inside the Rust-owned running authority.

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
    /// Validate the portable source and shared weights against this exact live
    /// authority. Numeric environment provenance remains descriptive only.
    pub fn spawn_saved_brain(
        &mut self,
        request: &crate::engine::brain_library::SavedBrainSpawnRequest,
        limits: RunningStepWorkLimits,
    ) -> Result<ResurrectionPublication, String> {
        request.validate().map_err(str::to_owned)?;
        let state = &self.candidate;
        if state.identity.run_id != request.expected_run_id
            || state.identity.config_revision != request.expected_config_revision
            || self.world_epoch() != request.expected_world_epoch
        {
            return Err("saved brain deployment run/configuration changed".into());
        }
        let directory = std::path::Path::new(&request.managed_directory)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let bytes = crate::engine::brain_library::read_artifact_file(
            &directory.join(format!("{}.slither-brain", request.brain_id)),
        )?;
        let artifact = crate::engine::brain_artifact::validate_brain_artifact(&bytes)?;
        if artifact.artifact_sha256 != request.brain_id
            || artifact.weights_sha256 != request.weights.logical_sha256
        {
            return Err("saved brain artifact/weight identity mismatch".into());
        }
        if artifact.graph.architecture_key != state.config.graph_architecture_key {
            return Err("saved brain graph does not match the active graph".into());
        }
        let sensor = crate::engine::sensor_contract::SensorContractIdentity::from_settings(
            &state.config.settings,
        )
        .map_err(|error| error.to_string())?;
        if sensor.sha256_hex().map_err(|error| error.to_string())?
            != artifact.canonical_manifest.sensor_sha256
        {
            return Err("saved brain structural sensors do not match the active sensors".into());
        }
        let cadence = self
            .running_step_config(limits)
            .map_err(|error| error.to_string())?
            .world_step
            .control
            .neural_control_interval_seconds;
        if cadence.to_bits() != artifact.deployment.control_interval_seconds.to_bits() {
            return Err("saved brain cadence does not match the active neural cadence".into());
        }
        // The portable graph is independently bounded to one million parameters.
        // Bind the shared object's shape before its decoder reserves any storage.
        let parameter_count = artifact.weights.len();
        if request.weights.version != 1
            || request.weights.relative_filename
                != format!("{}.hof-weights-v1", artifact.weights_sha256)
            || request.weights.weight_count_hex != format!("{parameter_count:016x}")
            || request.weights.decoded_byte_count_hex != format!("{:016x}", parameter_count * 4)
            || request.weights.stored_byte_count_hex.len() != 16
            || u64::from_str_radix(&request.weights.stored_byte_count_hex, 16)
                .map_or(true, |bytes| bytes > 4_000_000)
        {
            return Err(
                "saved brain shared weight descriptor disagrees with admitted graph".into(),
            );
        }
        let weights = crate::engine::checkpoint::read_validated_hall_of_fame_weights(
            &directory.join(&request.weights.relative_filename),
            &request.weights,
        )
        .map_err(|error| error.to_string())?;
        let brain_id = decode_sha256_identity(
            "saved brain identity",
            &format!("sha256:{}", request.brain_id),
        )
        .map_err(|error| error.to_string())?;
        let origin = Arc::new(RetainedNeuralOrigin::SavedBrain {
            brain_id,
            weight_digest: crate::engine::retained_origin::packed_weight_digest(&weights),
            source_kind: artifact.provenance.source_kind,
        });
        self.spawn_retained_brain(weights, origin, limits)
    }

    /// Add one retained winner without copying the evolved population or existing world.
    pub fn resurrect_hall_of_fame(
        &mut self,
        entry_id: u64,
        weights: Box<[f32]>,
        limits: RunningStepWorkLimits,
    ) -> Result<ResurrectionPublication, String> {
        let origin = Arc::new(RetainedNeuralOrigin::HallOfFame {
            entry_id,
            weight_digest: crate::engine::retained_origin::packed_weight_digest(&weights),
        });
        self.spawn_retained_brain(weights, origin, limits)
    }

    /// Publish one retained neural owner with cold-validated immutable provenance.
    /// The live Arc remains valid after its catalog record or lease is released.
    pub fn spawn_retained_brain(
        &mut self,
        weights: Box<[f32]>,
        origin: Arc<RetainedNeuralOrigin>,
        limits: RunningStepWorkLimits,
    ) -> Result<ResurrectionPublication, String> {
        origin.validate_weights(&weights)?;
        if self.candidate.phase != AuthorityPhase::Running {
            return Err("Retained neural spawn requires a running authority".into());
        }
        if weights.len() != self.graph.compiled().total_parameters
            || weights.iter().any(|value| !value.is_finite())
        {
            return Err("Retained neural weights do not match the active graph".into());
        }
        let world_config = self
            .running_step_config(limits)
            .map_err(|error| error.to_string())?
            .world_step;
        let config = world_config.external_replacement;
        let source = &self.candidate;
        // Reuse only a dead retained owner. Record order cannot select a live,
        // population, baseline or external owner, even when capacities are full.
        let reuse = source
            .world
            .snakes
            .iter()
            .enumerate()
            .filter(|(_, snake)| {
                !snake.alive
                    && snake.kind == SnakeKind::Resurrected
                    && snake.population_slot.is_none()
                    && !source
                        .world
                        .controller_leases
                        .iter()
                        .any(|lease| lease.snake_id == snake.id)
            })
            .filter_map(|(snake_index, snake)| {
                source
                    .brains
                    .iter()
                    .position(|brain| {
                        Some(brain.handle) == snake.brain
                            && brain.owner == BrainOwner::Entity(snake.id)
                            && brain.retained_origin.is_some()
                    })
                    .map(|brain_index| (snake_index, brain_index))
            })
            .min_by_key(|(snake_index, _)| source.world.snakes[*snake_index].id);
        let old_range = reuse.map(|(index, _)| source.world.snakes[index].body);
        let removed_body_len = old_range.map_or(0, |range| range.len);
        let additional_records = usize::from(reuse.is_none());
        let required_snakes = source.world.snakes.len().saturating_add(additional_records);
        let required_brains = source.brains.len().saturating_add(additional_records);
        let required_body = source
            .world
            .body_points
            .len()
            .checked_sub(removed_body_len)
            .and_then(|count| count.checked_add(config.spawn.snake_start_len))
            .ok_or("retained body capacity arithmetic overflowed")?;
        if required_snakes > config.maximum_snakes
            || required_brains > config.maximum_brains
            || required_body > config.maximum_body_points
        {
            return Err("Retained neural spawn exceeds admitted world capacity".into());
        }

        let mut next_allocators = source.allocators.clone();
        let snake_id = next_allocators
            .reserve_resurrected_ids(1)
            .map_err(|error| error.to_string())?
            .ok_or("resurrected identity allocation failed")?
            .first;
        let frame_v1_id = next_allocators
            .reserve_frame_v1_ids(1, source.world.snakes.iter().map(|snake| snake.frame_v1_id))
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
        let body_start = source.world.body_points.len() - removed_body_len;
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
            control_accumulator_seconds: world_config.control.initial_neural_accumulator_seconds(),
            delivered_observation_points: 0.0,
            observation_delivered: false,
            completed_step_heading_delta: 0.0,
            completed_step_speed_delta: 0.0,
            completed_step_applied_turn: 0.0,
            body: BodyRange {
                start: body_start,
                len: body.len(),
            },
            skin: 1,
        };
        let brain = BrainRuntimeState {
            handle: brain_handle,
            owner: BrainOwner::Entity(snake_id),
            retained_origin: Some(origin),
            non_population_weights: Some(weights),
            recurrent: vec![0.0; self.graph.compiled().total_state_size].into_boxed_slice(),
        };

        let staging = match reuse {
            Some((snake_index, brain_index)) => Some(RetainedReuseStaging::prepare(
                source,
                snake_index,
                brain_index,
                workspace.owned_bytes()?,
                body.len(),
            )?),
            None => None,
        };
        let candidate = &mut self.candidate;
        candidate
            .world
            .snakes
            .try_reserve_exact(additional_records)
            .map_err(|error| error.to_string())?;
        candidate
            .brains
            .try_reserve_exact(additional_records)
            .map_err(|error| error.to_string())?;
        let additional_body = required_body.saturating_sub(candidate.world.body_points.len());
        candidate
            .world
            .body_points
            .try_reserve_exact(additional_body)
            .map_err(|error| error.to_string())?;
        let undo = match staging {
            Some(staging) => staging.apply(candidate, snake, brain),
            None => {
                candidate.world.snakes.push(snake);
                candidate.brains.push(brain);
                RetainedSpawnUndo(None)
            }
        };
        candidate.world.body_points.extend_from_slice(&body);
        let old_allocators = std::mem::replace(&mut candidate.allocators, next_allocators);
        let old_world_rng = std::mem::replace(&mut candidate.rng.world, next_world_rng);
        let validation = catch_unwind(AssertUnwindSafe(|| {
            let memory = validate_running_mutable_state(
                candidate,
                self.graph.compiled(),
                &mut self.world_validation,
            )
            .and_then(|()| estimate_state_memory(candidate, &self.graph))?;
            if memory.total_bytes > self.memory_ceiling_bytes {
                return Err(StateError::MemoryCeilingExceeded {
                    estimated_bytes: memory.total_bytes,
                    ceiling_bytes: self.memory_ceiling_bytes,
                });
            }
            Ok(memory)
        }));
        let memory = match validation {
            Ok(Ok(memory)) => memory,
            failure => {
                candidate.allocators = old_allocators;
                candidate.rng.world = old_world_rng;
                undo.restore(candidate, body_start);
                match failure {
                    Ok(Err(error)) => return Err(error.to_string()),
                    Err(payload) => resume_unwind(payload),
                    Ok(Ok(_)) => unreachable!(),
                }
            }
        };
        self.memory = memory;
        Ok(ResurrectionPublication {
            snake_id,
            frame_v1_id,
        })
    }
}

/// Cold rollback storage is bounded by the already-admitted checkpoint scratch
/// reserve. It never duplicates population weights or the complete world.
struct RetainedReuseStaging {
    snake_index: usize,
    brain_index: usize,
    removed_body: Vec<WorldPoint>,
    body_ranges: Vec<BodyRange>,
    pellet_owners: Vec<usize>,
}

impl RetainedReuseStaging {
    fn prepare(
        source: &StateCandidate,
        snake_index: usize,
        brain_index: usize,
        spawn_bytes: usize,
        new_body_len: usize,
    ) -> Result<Self, String> {
        let old = &source.world.snakes[snake_index];
        let owner_count = source
            .world
            .pellets
            .iter()
            .filter(|pellet| pellet.owner == Some(old.id))
            .count();
        let old_brain = &source.brains[brain_index];
        let old_numeric_count = old_brain
            .non_population_weights
            .as_ref()
            .map_or(0, |weights| weights.len())
            .checked_add(old_brain.recurrent.len())
            .ok_or("retained numeric staging overflowed")?;
        let scratch = [
            (old.body.len, size_of::<WorldPoint>()),
            (new_body_len, size_of::<WorldPoint>()),
            (source.world.snakes.len(), size_of::<BodyRange>()),
            (owner_count, size_of::<usize>()),
            (old_numeric_count, size_of::<f32>()),
        ]
        .into_iter()
        .try_fold(spawn_bytes, |total, (count, width)| {
            count
                .checked_mul(width)
                .and_then(|bytes| total.checked_add(bytes))
        })
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or("retained rollback scratch arithmetic overflowed")?;
        if scratch > source.config.checkpoint_scratch_bytes {
            return Err("retained rollback exceeds admitted cold scratch".into());
        }
        let end = old
            .body
            .start
            .checked_add(old.body.len)
            .ok_or("retained body range overflowed")?;
        let old_body = source
            .world
            .body_points
            .get(old.body.start..end)
            .ok_or("retained body range is invalid")?;
        let mut removed_body = Vec::new();
        removed_body
            .try_reserve_exact(old_body.len())
            .map_err(|error| error.to_string())?;
        removed_body.extend_from_slice(old_body);
        let mut body_ranges = Vec::new();
        body_ranges
            .try_reserve_exact(source.world.snakes.len())
            .map_err(|error| error.to_string())?;
        body_ranges.extend(source.world.snakes.iter().map(|snake| snake.body));
        let mut pellet_owners = Vec::new();
        pellet_owners
            .try_reserve_exact(owner_count)
            .map_err(|error| error.to_string())?;
        pellet_owners.extend(
            source
                .world
                .pellets
                .iter()
                .enumerate()
                .filter_map(|(index, pellet)| (pellet.owner == Some(old.id)).then_some(index)),
        );
        Ok(Self {
            snake_index,
            brain_index,
            removed_body,
            body_ranges,
            pellet_owners,
        })
    }

    /// Every allocation and fallible precondition completed before this mutation.
    fn apply(
        self,
        candidate: &mut StateCandidate,
        snake: SnakeState,
        brain: BrainRuntimeState,
    ) -> RetainedSpawnUndo {
        let old_range = self.body_ranges[self.snake_index];
        let end = old_range.start + old_range.len;
        drop(candidate.world.body_points.drain(old_range.start..end));
        for (index, other) in candidate.world.snakes.iter_mut().enumerate() {
            if index != self.snake_index && other.body.start >= old_range.start {
                other.body.start = if other.body.start >= end {
                    other.body.start - old_range.len
                } else {
                    old_range.start
                };
            }
        }
        for &index in &self.pellet_owners {
            candidate.world.pellets[index].owner = None;
        }
        let old_snake = std::mem::replace(&mut candidate.world.snakes[self.snake_index], snake);
        let old_brain = std::mem::replace(&mut candidate.brains[self.brain_index], brain);
        RetainedSpawnUndo(Some((self, old_snake, old_brain)))
    }
}

/// Own the displaced weights and Arc until complete state/memory validation.
struct RetainedSpawnUndo(Option<(RetainedReuseStaging, SnakeState, BrainRuntimeState)>);

impl RetainedSpawnUndo {
    fn restore(self, candidate: &mut StateCandidate, body_start: usize) {
        candidate.world.body_points.truncate(body_start);
        match self.0 {
            None => {
                candidate.world.snakes.pop();
                candidate.brains.pop();
            }
            Some((staging, old_snake, old_brain)) => {
                let range = staging.body_ranges[staging.snake_index];
                // Draining and appending retained the original capacity, so this
                // exact-size insertion cannot allocate during rollback.
                drop(
                    candidate
                        .world
                        .body_points
                        .splice(range.start..range.start, staging.removed_body),
                );
                for (snake, old_range) in candidate.world.snakes.iter_mut().zip(staging.body_ranges)
                {
                    snake.body = old_range;
                }
                for index in staging.pellet_owners {
                    candidate.world.pellets[index].owner = Some(old_snake.id);
                }
                candidate.world.snakes[staging.snake_index] = old_snake;
                candidate.brains[staging.brain_index] = old_brain;
            }
        }
    }
}
