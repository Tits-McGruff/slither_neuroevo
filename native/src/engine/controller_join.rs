//! Detached fresh-controller admission at an unprepared authority boundary.

use super::*;
use crate::engine::external_replacement::{
    authority_world_digest, prepare_fresh_external, FreshExternalSource, PreparedFreshExternal,
};

/// Native-stamped fresh join after token/legacy reclaim has been resolved.
pub struct ControllerJoinInput {
    pub kind: ControllerKind,
    pub identity_key: String,
    pub resume_token: String,
    pub connection_id: u64,
    pub arrival_sequence: u64,
    pub received_at_ms: u64,
    pub boundary_at_ms: u64,
}

/// Private prepared buffers and source guards retained until assignment delivery.
#[derive(Debug)]
pub struct PreparedControllerJoin {
    world_epoch: u64,
    operation_epoch: u64,
    completed_step: u64,
    world_digest: [u8; 32],
    expected_allocators: AllocatorState,
    expected_external_rng: SerializedRngState,
    expected_memory: StateMemoryEstimate,
    next_memory: StateMemoryEstimate,
    fresh: PreparedFreshExternal,
    lease: ControllerLease,
}

impl PreparedControllerJoin {
    /// Newly reserved exact controller identity.
    pub fn lease_id(&self) -> u64 {
        self.lease.id
    }
    /// Browser-visible identity supplied by the pending assignment.
    pub fn frame_v1_id(&self) -> u32 {
        self.fresh.snake.frame_v1_id
    }
    /// Exact destination of the pending assignment.
    pub fn connection_id(&self) -> u64 {
        self.lease.connection_id.expect("prepared connected lease")
    }
    /// OS-generated token that becomes valid only after delivery succeeds.
    pub fn resume_token(&self) -> &str {
        &self.lease.resume_token
    }
}

impl AuthoritativeState {
    /// The caller must reserve the complete output reply before preparing a join.
    /// Validation temporarily borrows the appended buffers under exclusive
    /// ownership and restores every source value, including on validation panic.
    pub fn prepare_controller_join(
        &mut self,
        input: ControllerJoinInput,
        limits: RunningStepWorkLimits,
    ) -> Result<PreparedControllerJoin, String> {
        if self.candidate.phase != AuthorityPhase::Running
            || input.connection_id == 0
            || input.arrival_sequence == 0
            || input.received_at_ms > input.boundary_at_ms
            || input.identity_key.is_empty()
            || input.identity_key.len() > 128
            || input.identity_key.contains('\0')
            || input.resume_token.len() != 32
            || !input
                .resume_token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("invalid fresh controller join".into());
        }
        let source = &self.candidate;
        if source.world.controller_leases.iter().any(|lease| {
            lease.connection_id == Some(input.connection_id)
                || lease.resume_token == input.resume_token
        }) {
            return Err("join connection or token is already assigned".into());
        }
        let timing = crate::engine::controllers::ControllerTiming::from_config(&source.config)
            .map_err(|error| error.to_string())?;
        if crate::engine::controllers::select_legacy_reclaim(
            &source.world,
            input.kind,
            &source.identity.run_id,
            &input.identity_key,
            input.boundary_at_ms,
            timing,
        )
        .map_err(|error| error.to_string())?
        .is_some()
        {
            return Err("reserved identity requires reclaim".into());
        }
        let config = self
            .running_step_config(limits)
            .map_err(|error| error.to_string())?
            .world_step
            .external_replacement;
        let mut fresh = prepare_fresh_external(
            FreshExternalSource {
                world: &source.world,
                brains: &source.brains,
                rng: &source.rng,
                allocators: &source.allocators,
                graph: &self.graph,
                brain_epoch: source.generation.population_epoch,
            },
            config,
        )
        .map_err(|error| error.to_string())?;
        let lease = ControllerLease {
            identity_key: input.identity_key,
            id: fresh.lease_id,
            snake_id: fresh.snake.id,
            kind: input.kind,
            scope: source.identity.run_id.clone(),
            resume_token: input.resume_token,
            connection_id: Some(input.connection_id),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                turn: 0.0,
                boost: false,
                client_tick: source.generation.completed_step,
                arrival_sequence: input.arrival_sequence,
                accepted_at_ms: input.received_at_ms,
            },
            last_observed_at_ms: input.boundary_at_ms,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        };
        let world_digest = authority_world_digest(&source.world);
        let expected_allocators = source.allocators.clone();
        let expected_external_rng = source.rng.external_controller.clone();
        let retained_bytes = [
            size_of::<PreparedControllerJoin>(),
            fresh
                .body
                .capacity()
                .checked_mul(size_of::<WorldPoint>())
                .ok_or("join body memory overflow")?,
            fresh
                .brain
                .non_population_weights
                .as_ref()
                .map_or(0, |weights| weights.len())
                .checked_mul(size_of::<f32>())
                .ok_or("join weight memory overflow")?,
            fresh
                .brain
                .recurrent
                .len()
                .checked_mul(size_of::<f32>())
                .ok_or("join recurrent memory overflow")?,
            lease.identity_key.capacity(),
            lease.scope.capacity(),
            lease.resume_token.capacity(),
            fresh.next_external_rng.algorithm.capacity(),
            fresh.next_external_rng.state_hex.capacity(),
            fresh.next_external_rng.gaussian_algorithm.capacity(),
            fresh
                .next_external_rng
                .gaussian_spare_hex
                .as_ref()
                .map_or(0, String::capacity),
            expected_external_rng.algorithm.capacity(),
            expected_external_rng.state_hex.capacity(),
            expected_external_rng.gaussian_algorithm.capacity(),
            expected_external_rng
                .gaussian_spare_hex
                .as_ref()
                .map_or(0, String::capacity),
        ]
        .into_iter()
        .try_fold(0usize, |total, bytes| total.checked_add(bytes))
        .ok_or("join retained memory overflow")?;
        if self
            .memory
            .total_bytes
            .checked_add(retained_bytes)
            .is_none_or(|bytes| bytes > self.memory_ceiling_bytes)
        {
            return Err("join staging exceeds admitted authority memory".into());
        }
        // Reserve all destination slots before an assignment can escape. These
        // capacities are already charged by the admitted maximum-world estimate.
        let candidate = &mut self.candidate;
        candidate
            .world
            .snakes
            .try_reserve_exact(1)
            .map_err(|error| error.to_string())?;
        candidate
            .world
            .controller_leases
            .try_reserve_exact(1)
            .map_err(|error| error.to_string())?;
        candidate
            .brains
            .try_reserve_exact(1)
            .map_err(|error| error.to_string())?;
        candidate
            .world
            .body_points
            .try_reserve_exact(fresh.body.len())
            .map_err(|error| error.to_string())?;
        let body_start = candidate.world.body_points.len();
        candidate.world.body_points.extend_from_slice(&fresh.body);
        candidate.world.snakes.push(fresh.snake);
        candidate.world.controller_leases.push(lease);
        candidate.brains.push(fresh.brain);
        std::mem::swap(&mut candidate.allocators, &mut fresh.next_allocators);
        std::mem::swap(
            &mut candidate.rng.external_controller,
            &mut fresh.next_external_rng,
        );
        let checked = catch_unwind(AssertUnwindSafe(|| {
            validate_running_mutable_state(
                candidate,
                self.graph.compiled(),
                &mut self.world_validation,
            )?;
            let memory = estimate_state_memory(candidate, &self.graph)?;
            let staged_total = memory.total_bytes.checked_add(retained_bytes).ok_or(
                StateError::ArithmeticOverflow {
                    context: "fresh join staging memory",
                },
            )?;
            if staged_total > self.memory_ceiling_bytes {
                return Err(StateError::MemoryCeilingExceeded {
                    estimated_bytes: staged_total,
                    ceiling_bytes: self.memory_ceiling_bytes,
                });
            }
            Ok(memory)
        }));
        std::mem::swap(&mut candidate.allocators, &mut fresh.next_allocators);
        std::mem::swap(
            &mut candidate.rng.external_controller,
            &mut fresh.next_external_rng,
        );
        fresh.snake = candidate.world.snakes.pop().expect("temporary join snake");
        fresh.brain = candidate.brains.pop().expect("temporary join brain");
        let lease = candidate
            .world
            .controller_leases
            .pop()
            .expect("temporary join lease");
        candidate.world.body_points.truncate(body_start);
        let next_memory = match checked {
            Ok(result) => result.map_err(|error| error.to_string())?,
            Err(panic) => resume_unwind(panic),
        };
        Ok(PreparedControllerJoin {
            world_epoch: self.world_epoch,
            operation_epoch: self.latest_operation_epoch,
            completed_step: candidate.generation.completed_step,
            world_digest,
            expected_allocators,
            expected_external_rng,
            expected_memory: self.memory,
            next_memory,
            fresh,
            lease,
        })
    }

    /// Append exactly once after a correlated successful local-send receipt.
    /// Every guard is checked before touching any authoritative value.
    pub fn commit_controller_join(
        &mut self,
        prepared: PreparedControllerJoin,
    ) -> Result<(), String> {
        self.validate_controller_join(&prepared)?;
        let candidate = &mut self.candidate;
        candidate
            .world
            .body_points
            .extend_from_slice(&prepared.fresh.body);
        candidate.world.snakes.push(prepared.fresh.snake);
        candidate.brains.push(prepared.fresh.brain);
        candidate.world.controller_leases.push(prepared.lease);
        candidate.allocators = prepared.fresh.next_allocators;
        candidate.rng.external_controller = prepared.fresh.next_external_rng;
        self.memory = prepared.next_memory;
        Ok(())
    }
    /// Check a retained candidate before the runtime takes ownership for commit.
    pub(crate) fn validate_controller_join(
        &self,
        prepared: &PreparedControllerJoin,
    ) -> Result<(), String> {
        let candidate = &self.candidate;
        if candidate.phase != AuthorityPhase::Running
            || self.world_epoch != prepared.world_epoch
            || self.latest_operation_epoch != prepared.operation_epoch
            || candidate.generation.completed_step != prepared.completed_step
            || self.memory != prepared.expected_memory
            || candidate.allocators != prepared.expected_allocators
            || candidate.rng.external_controller != prepared.expected_external_rng
            || authority_world_digest(&candidate.world) != prepared.world_digest
            || candidate.world.snakes.len() == candidate.world.snakes.capacity()
            || candidate.world.controller_leases.len()
                == candidate.world.controller_leases.capacity()
            || candidate.brains.len() == candidate.brains.capacity()
            || candidate.world.body_points.capacity() - candidate.world.body_points.len()
                < prepared.fresh.body.len()
        {
            return Err("fresh join source boundary changed".into());
        }
        Ok(())
    }
}
