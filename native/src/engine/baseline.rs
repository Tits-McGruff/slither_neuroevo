//! Durable baseline-slot lifecycle and pre-control respawn timing.
//!
//! Baseline strategy evaluation and collision-safe placement are joined by the
//! later fixed-step coordinator. This module owns the stable slot identity,
//! behavior scratch, death-delay semantics, and exact due-slot proposal needed
//! before that join. A due placement is intentionally not treated as success
//! or failure here: the owner-visible policy for an impossible mid-generation
//! placement remains unresolved and must not be silently selected by a timer.

use super::effects::BaselineDeathEvent;
use super::physics::{PhysicsStepKey, PhysicsStepKeyField, PreparedPhysicsBaselineDeaths};
use super::state::{BaselineStrategyState, SnakeKind, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First Rust baseline lifecycle/timer algorithm identity.
pub const BASELINE_LIFECYCLE_VERSION: u32 = 1;
/// Maximum baseline slots admitted by the current settings contract.
const MAXIMUM_BASELINE_SLOTS: usize = 120;
/// Sentinel for one absent reusable world-slot index.
const MISSING_INDEX: usize = usize::MAX;
/// Largest durable roam offset produced by the current baseline controller.
const MAXIMUM_WANDER_ANGLE: f64 = 0.3;
/// Largest durable roam timer produced by the current baseline controller.
const MAXIMUM_WANDER_TIMER_SECONDS: f64 = 2.0;
/// Largest durable avoid timer produced by the current baseline controller.
const MAXIMUM_AVOID_TIMER_SECONDS: f64 = 0.70;
/// Largest durable boost timer produced by the current baseline controller.
const MAXIMUM_BOOST_TIMER_SECONDS: f64 = 0.40;

/// Versioned baseline lifecycle settings projected from admitted config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineLifecycleConfig {
    /// Versioned timer and slot-state identity.
    pub algorithm_version: u32,
    /// Exact reset-only durable baseline slot count.
    pub slot_count: usize,
    /// Live configurable respawn delay in simulated seconds.
    pub respawn_delay_seconds: f64,
}

impl BaselineLifecycleConfig {
    /// Current TypeScript defaults.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            algorithm_version: BASELINE_LIFECYCLE_VERSION,
            slot_count: 10,
            respawn_delay_seconds: 20.0,
        }
    }

    fn validate(self) -> Result<(), BaselineLifecycleError> {
        if self.algorithm_version != BASELINE_LIFECYCLE_VERSION {
            return Err(BaselineLifecycleError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        if self.slot_count > MAXIMUM_BASELINE_SLOTS {
            return Err(BaselineLifecycleError::InvalidConfig {
                field: "slot_count",
            });
        }
        if !self.respawn_delay_seconds.is_finite()
            || !(0.5..=60.0).contains(&self.respawn_delay_seconds)
        {
            return Err(BaselineLifecycleError::InvalidConfig {
                field: "respawn_delay_seconds",
            });
        }
        Ok(())
    }
}

/// Transient timer/action scratch for one durable baseline slot.
///
/// [`super::state::SnakeState::baseline_strategy`] is the one canonical
/// strategy value. It is deliberately not duplicated here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaselineSlotRuntime {
    /// Dense stable slot, equal to this record's array index.
    pub slot: u32,
    /// Current dead-or-alive snake entity assigned to the slot.
    pub snake_id: u64,
    /// Remaining time for a timed avoid/boost strategy.
    pub strategy_timer_seconds: f64,
    /// Current roam wander offset in radians.
    pub wander_angle: f64,
    /// Remaining time before the roam offset is regenerated.
    pub wander_timer_seconds: f64,
    /// Latest selected turn action.
    pub turn: f32,
    /// Latest selected boost action.
    pub boost: bool,
    /// Remaining respawn wait. `None` means the current snake is expected alive.
    pub respawn_remaining_seconds: Option<f64>,
}

impl BaselineSlotRuntime {
    fn neutralize(&mut self) {
        self.turn = 0.0;
        self.boost = false;
    }
}

/// Generation-scoped baseline slots owned by the future Rust coordinator.
#[derive(Clone, Debug, PartialEq)]
pub struct BaselineLifecycleState {
    /// Versioned state layout.
    pub version: u32,
    /// Dense stable slot records.
    pub slots: Vec<BaselineSlotRuntime>,
}

impl BaselineLifecycleState {
    /// Construct the empty pre-spawn state implied by an exact checkpoint boundary.
    #[must_use]
    pub fn generation_boundary() -> Self {
        Self {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: Vec::new(),
        }
    }

    /// Initialize baseline runtime only after a complete collision-safe initial
    /// spawn has produced one live snake per configured slot.
    pub fn initialize_after_complete_spawn(
        config: BaselineLifecycleConfig,
        world: &WorldState,
    ) -> Result<Self, BaselineLifecycleError> {
        config.validate()?;
        validate_world_slot_claims(world, config.slot_count)?;
        let mut slots = Vec::new();
        slots.try_reserve_exact(config.slot_count).map_err(|_| {
            BaselineLifecycleError::AllocationFailed {
                buffer: "baseline slots",
                required: config.slot_count,
            }
        })?;
        for index in 0..config.slot_count {
            let slot =
                u32::try_from(index).map_err(|_| BaselineLifecycleError::ArithmeticOverflow {
                    context: "baseline slot identity",
                })?;
            let snake_index = find_world_slot(world, slot)?;
            let snake = &world.snakes[snake_index];
            if !snake.alive {
                return Err(BaselineLifecycleError::InitialSnakeNotAlive {
                    slot,
                    snake_id: snake.id,
                });
            }
            let strategy =
                snake
                    .baseline_strategy
                    .ok_or(BaselineLifecycleError::MissingWorldStrategy {
                        slot,
                        snake_id: snake.id,
                    })?;
            if slots
                .iter()
                .any(|runtime: &BaselineSlotRuntime| runtime.snake_id == snake.id)
            {
                return Err(BaselineLifecycleError::DuplicateSnakeIdentity(snake.id));
            }
            let runtime = BaselineSlotRuntime {
                slot,
                snake_id: snake.id,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: snake.turn,
                boost: snake.input_boost,
                respawn_remaining_seconds: None,
            };
            validate_slot_runtime(runtime, index)?;
            validate_strategy_runtime(runtime, strategy)?;
            slots.push(runtime);
        }
        Ok(Self {
            version: BASELINE_LIFECYCLE_VERSION,
            slots,
        })
    }

    /// Validate baseline continuation as part of complete engine-state admission.
    pub(crate) fn validate_authoritative(
        &self,
        world: &WorldState,
        configured_slots: usize,
        generation_boundary: bool,
    ) -> Result<(), BaselineLifecycleError> {
        if self.version != BASELINE_LIFECYCLE_VERSION {
            return Err(BaselineLifecycleError::InvalidStateVersion(self.version));
        }
        if generation_boundary {
            if self.slots.is_empty() {
                return Ok(());
            }
            return Err(BaselineLifecycleError::SlotCountMismatch {
                actual: self.slots.len(),
                expected: 0,
            });
        }
        if self.slots.len() != configured_slots {
            return Err(BaselineLifecycleError::SlotCountMismatch {
                actual: self.slots.len(),
                expected: configured_slots,
            });
        }
        validate_world_slot_claims(world, configured_slots)?;
        for (index, runtime) in self.slots.iter().copied().enumerate() {
            validate_slot_runtime(runtime, index)?;
            let world_index = find_world_slot(world, runtime.slot)?;
            let snake = &world.snakes[world_index];
            if snake.id != runtime.snake_id {
                return Err(BaselineLifecycleError::SnakeIdentityMismatch {
                    slot: runtime.slot,
                    expected: runtime.snake_id,
                    actual: snake.id,
                });
            }
            if snake.baseline_strategy.is_none() {
                return Err(BaselineLifecycleError::MissingWorldStrategy {
                    slot: runtime.slot,
                    snake_id: snake.id,
                });
            }
            validate_strategy_runtime(
                runtime,
                snake
                    .baseline_strategy
                    .expect("strategy presence was checked immediately above"),
            )?;
            if snake.alive && runtime.respawn_remaining_seconds.is_some() {
                return Err(BaselineLifecycleError::LiveSnakeHasRespawnTimer {
                    slot: runtime.slot,
                    snake_id: snake.id,
                });
            }
            if snake.alive
                && (snake.turn.to_bits() != runtime.turn.to_bits()
                    || snake.input_boost != runtime.boost)
            {
                return Err(BaselineLifecycleError::WorldActionMismatch {
                    slot: runtime.slot,
                    snake_id: snake.id,
                });
            }
            if !snake.alive && runtime.respawn_remaining_seconds.is_none() {
                return Err(BaselineLifecycleError::DeadSnakeMissingRespawnTimer {
                    slot: runtime.slot,
                    snake_id: snake.id,
                });
            }
        }
        Ok(())
    }
}

/// Validate strategy-dependent durable timer and wander combinations.
///
/// State admission and the baseline evaluator share this contract so an
/// authoritative state cannot be accepted only to fail before its first
/// baseline control boundary.
pub(crate) fn validate_strategy_runtime(
    slot: BaselineSlotRuntime,
    strategy: BaselineStrategyState,
) -> Result<(), BaselineLifecycleError> {
    if slot.wander_angle.abs() > MAXIMUM_WANDER_ANGLE {
        return Err(BaselineLifecycleError::InvalidSlotScalar {
            slot: slot.slot,
            field: "wander_angle",
        });
    }
    if slot.wander_timer_seconds > MAXIMUM_WANDER_TIMER_SECONDS {
        return Err(BaselineLifecycleError::InvalidSlotScalar {
            slot: slot.slot,
            field: "wander_timer_seconds",
        });
    }
    let valid_strategy_timer = match strategy {
        BaselineStrategyState::Roam | BaselineStrategyState::Seek => {
            slot.strategy_timer_seconds == 0.0
        }
        BaselineStrategyState::Avoid => {
            slot.strategy_timer_seconds > 0.0
                && slot.strategy_timer_seconds <= MAXIMUM_AVOID_TIMER_SECONDS
        }
        BaselineStrategyState::Boost => {
            slot.strategy_timer_seconds > 0.0
                && slot.strategy_timer_seconds <= MAXIMUM_BOOST_TIMER_SECONDS
        }
    };
    if !valid_strategy_timer {
        return Err(BaselineLifecycleError::InvalidSlotScalar {
            slot: slot.slot,
            field: "strategy_timer_seconds",
        });
    }
    Ok(())
}

/// Size and retained-allocation diagnostics for the latest timer preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaselineLifecycleDiagnostics {
    /// Configured durable slots examined.
    pub slots: usize,
    /// Dead/missing slots with active waits after preparation.
    pub waiting_slots: usize,
    /// Slots whose delay reached zero and require a placement decision.
    pub due_slots: usize,
    /// Retained next-slot capacity.
    pub next_slot_capacity: usize,
    /// Retained due-slot capacity.
    pub due_slot_capacity: usize,
    /// Retained world-slot index capacity.
    pub world_slot_index_capacity: usize,
}

/// Read-only timer result for the stable pre-movement boundary.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBaselineTimers<'lifecycle, 'source> {
    key: PhysicsStepKey,
    source_world: &'source WorldState,
    source_state: &'source BaselineLifecycleState,
    fixed_dt: f64,
    config: BaselineLifecycleConfig,
    next_slots: &'lifecycle [BaselineSlotRuntime],
    due_slots: &'lifecycle [u32],
    diagnostics: BaselineLifecycleDiagnostics,
}

impl<'lifecycle, 'source> PreparedBaselineTimers<'lifecycle, 'source> {
    /// Complete authority/config/operation identity prepared.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Immutable world boundary inspected for slot liveness.
    #[must_use]
    pub const fn source_world(self) -> &'source WorldState {
        self.source_world
    }

    /// Immutable baseline lifecycle boundary advanced.
    #[must_use]
    pub const fn source_state(self) -> &'source BaselineLifecycleState {
        self.source_state
    }

    /// Dense timer/behavior state after this boundary's timer work.
    #[must_use]
    pub const fn next_slots(self) -> &'lifecycle [BaselineSlotRuntime] {
        self.next_slots
    }

    /// Stable slot IDs that now require collision-safe placement.
    #[must_use]
    pub const fn due_slots(self) -> &'lifecycle [u32] {
        self.due_slots
    }

    /// Timer result may join directly only when no placement choice remains.
    #[must_use]
    pub const fn requires_respawn_resolution(self) -> bool {
        !self.due_slots.is_empty()
    }

    /// Current sizes and retained-allocation diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> BaselineLifecycleDiagnostics {
        self.diagnostics
    }

    /// Revalidate every source input before joining the proposal.
    pub fn validate_current(
        self,
        current_key: PhysicsStepKey,
        current_world: &WorldState,
        current_state: &BaselineLifecycleState,
        current_fixed_dt: f64,
        current_config: BaselineLifecycleConfig,
    ) -> Result<(), BaselineLifecycleError> {
        if let Some(field) = self.key.first_mismatch(current_key) {
            return Err(BaselineLifecycleError::StepKeyMismatch { field });
        }
        if !std::ptr::eq(self.source_world, current_world) {
            return Err(BaselineLifecycleError::SourceChanged { field: "world" });
        }
        if !std::ptr::eq(self.source_state, current_state) {
            return Err(BaselineLifecycleError::SourceChanged {
                field: "baseline lifecycle",
            });
        }
        if self.fixed_dt.to_bits() != current_fixed_dt.to_bits() {
            return Err(BaselineLifecycleError::SourceChanged {
                field: "fixed delta",
            });
        }
        if self.config != current_config {
            return Err(BaselineLifecycleError::SourceChanged { field: "config" });
        }
        Ok(())
    }

    /// Apply a timer-only result to a matching coordinator-owned working copy.
    /// Due slots are deliberately rejected until the reviewed placement policy
    /// and a successful spawn result resolve every one of them.
    pub fn apply_without_due_respawns(
        self,
        current_key: PhysicsStepKey,
        current_world: &WorldState,
        current_state: &BaselineLifecycleState,
        current_fixed_dt: f64,
        current_config: BaselineLifecycleConfig,
        target_state: &mut BaselineLifecycleState,
    ) -> Result<(), BaselineLifecycleError> {
        self.validate_current(
            current_key,
            current_world,
            current_state,
            current_fixed_dt,
            current_config,
        )?;
        if !self.due_slots.is_empty() {
            return Err(BaselineLifecycleError::RespawnsUnresolved {
                count: self.due_slots.len(),
            });
        }
        if target_state != self.source_state || target_state.slots.len() != self.next_slots.len() {
            return Err(BaselineLifecycleError::WorkingCopyChanged);
        }
        for (target, next) in target_state.slots.iter_mut().zip(self.next_slots) {
            *target = *next;
        }
        Ok(())
    }
}

/// Read-only lifecycle update derived from one complete keyed physics result.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBaselineDeaths<'lifecycle, 'source, 'physics> {
    key: PhysicsStepKey,
    physics_world: &'physics WorldState,
    source_state: &'source BaselineLifecycleState,
    config: BaselineLifecycleConfig,
    next_slots: &'lifecycle [BaselineSlotRuntime],
    event_count: usize,
}

impl<'lifecycle, 'source, 'physics> PreparedBaselineDeaths<'lifecycle, 'source, 'physics> {
    /// Exact fixed-step identity shared with the producing physics result.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Immutable post-physics world carried by the keyed physics proof.
    #[must_use]
    pub const fn physics_world(self) -> &'physics WorldState {
        self.physics_world
    }

    /// Number of canonical death events applied to the staged lifecycle state.
    #[must_use]
    pub const fn event_count(self) -> usize {
        self.event_count
    }

    /// Lifecycle slots after applying every validated notification.
    #[must_use]
    pub const fn next_slots(self) -> &'lifecycle [BaselineSlotRuntime] {
        self.next_slots
    }

    /// Apply to the exact lifecycle working copy after revalidating authority.
    pub fn apply_to_working_copy(
        self,
        current_key: PhysicsStepKey,
        current_state: &BaselineLifecycleState,
        current_config: BaselineLifecycleConfig,
        target_state: &mut BaselineLifecycleState,
    ) -> Result<(), BaselineLifecycleError> {
        if let Some(field) = self.key.first_mismatch(current_key) {
            return Err(BaselineLifecycleError::StepKeyMismatch { field });
        }
        if !std::ptr::eq(self.source_state, current_state) {
            return Err(BaselineLifecycleError::SourceChanged {
                field: "baseline lifecycle",
            });
        }
        if self.config != current_config {
            return Err(BaselineLifecycleError::SourceChanged { field: "config" });
        }
        if target_state != self.source_state || target_state.slots.len() != self.next_slots.len() {
            return Err(BaselineLifecycleError::WorkingCopyChanged);
        }
        for (target, next) in target_state.slots.iter_mut().zip(self.next_slots) {
            *target = *next;
        }
        Ok(())
    }
}

/// Reusable, non-authoritative baseline timer scratch.
#[derive(Debug, Default)]
pub struct BaselineLifecycleWorkspace {
    next_slots: Vec<BaselineSlotRuntime>,
    due_slots: Vec<u32>,
    world_slot_indices: Vec<usize>,
    waiting_slots: usize,
    ready: bool,
}

impl BaselineLifecycleWorkspace {
    /// Construct empty reusable timer scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage one pre-control baseline timer boundary without spawning.
    pub fn prepare_timers<'lifecycle, 'source>(
        &'lifecycle mut self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_state: &'source BaselineLifecycleState,
        fixed_dt: f64,
        config: BaselineLifecycleConfig,
    ) -> Result<PreparedBaselineTimers<'lifecycle, 'source>, BaselineLifecycleError> {
        self.clear();
        validate_step_key(key)?;
        config.validate()?;
        validate_fixed_dt(fixed_dt)?;
        validate_runtime_shape(source_state, config)?;
        self.prepare_world_slot_indices(source_world, config.slot_count)?;
        reserve_for(
            &mut self.next_slots,
            config.slot_count,
            "next baseline slots",
        )?;
        reserve_for(&mut self.due_slots, config.slot_count, "due baseline slots")?;

        for index in 0..config.slot_count {
            let source = source_state.slots[index];
            validate_slot_runtime(source, index)?;
            let world_index = self.world_slot_indices[index];
            if world_index == MISSING_INDEX {
                return Err(BaselineLifecycleError::MissingWorldSlot(source.slot));
            }
            let snake = &source_world.snakes[world_index];
            if snake.id != source.snake_id {
                return Err(BaselineLifecycleError::SnakeIdentityMismatch {
                    slot: source.slot,
                    expected: source.snake_id,
                    actual: snake.id,
                });
            }
            if snake.baseline_strategy.is_none() {
                return Err(BaselineLifecycleError::MissingWorldStrategy {
                    slot: source.slot,
                    snake_id: snake.id,
                });
            }
            let mut next = source;
            if snake.alive {
                if source.respawn_remaining_seconds.is_some() {
                    return Err(BaselineLifecycleError::LiveSnakeHasRespawnTimer {
                        slot: source.slot,
                        snake_id: snake.id,
                    });
                }
            } else {
                next.neutralize();
                match source.respawn_remaining_seconds {
                    None => {
                        // A missed death notification starts the full delay and
                        // deliberately does not subtract until the next step.
                        next.respawn_remaining_seconds = Some(config.respawn_delay_seconds);
                        self.waiting_slots += 1;
                    }
                    Some(remaining) => {
                        let capped = remaining.min(config.respawn_delay_seconds);
                        let next_remaining = capped - fixed_dt;
                        if !next_remaining.is_finite() {
                            return Err(BaselineLifecycleError::NonFiniteTimer {
                                slot: source.slot,
                            });
                        }
                        if next_remaining <= 0.0 {
                            next.respawn_remaining_seconds = Some(0.0);
                            self.due_slots.push(source.slot);
                        } else {
                            next.respawn_remaining_seconds = Some(next_remaining);
                            self.waiting_slots += 1;
                        }
                    }
                }
            }
            self.next_slots.push(next);
        }
        self.ready = true;
        self.prepared(key, source_world, source_state, fixed_dt, config)
    }

    /// Stage committed baseline death notifications from one complete physics proof.
    pub fn prepare_committed_deaths<'lifecycle, 'source, 'physics>(
        &'lifecycle mut self,
        physics: PreparedPhysicsBaselineDeaths<'physics>,
        current_key: PhysicsStepKey,
        source_state: &'source BaselineLifecycleState,
        config: BaselineLifecycleConfig,
    ) -> Result<PreparedBaselineDeaths<'lifecycle, 'source, 'physics>, BaselineLifecycleError> {
        self.clear();
        validate_step_key(current_key)?;
        if let Some(field) = physics.key().first_mismatch(current_key) {
            return Err(BaselineLifecycleError::StepKeyMismatch { field });
        }
        config.validate()?;
        validate_runtime_shape(source_state, config)?;
        self.prepare_world_slot_indices(physics.world(), config.slot_count)?;
        for (index, runtime) in source_state.slots.iter().copied().enumerate() {
            let world_index = self.world_slot_indices[index];
            if world_index == MISSING_INDEX {
                return Err(BaselineLifecycleError::MissingWorldSlot(runtime.slot));
            }
            let snake = &physics.world().snakes[world_index];
            if snake.id != runtime.snake_id {
                return Err(BaselineLifecycleError::SnakeIdentityMismatch {
                    slot: runtime.slot,
                    expected: runtime.snake_id,
                    actual: snake.id,
                });
            }
            if snake.baseline_strategy.is_none() {
                return Err(BaselineLifecycleError::MissingWorldStrategy {
                    slot: runtime.slot,
                    snake_id: snake.id,
                });
            }
            if snake.alive && runtime.respawn_remaining_seconds.is_some() {
                return Err(BaselineLifecycleError::LiveSnakeHasRespawnTimer {
                    slot: runtime.slot,
                    snake_id: snake.id,
                });
            }
        }
        validate_death_events(physics.world(), physics.events(), source_state)?;
        reserve_for(
            &mut self.next_slots,
            source_state.slots.len(),
            "next baseline slots",
        )?;
        self.next_slots.extend_from_slice(&source_state.slots);
        for event in physics.events() {
            let slot_index = usize::try_from(event.slot).map_err(|_| {
                BaselineLifecycleError::ArithmeticOverflow {
                    context: "baseline death slot index",
                }
            })?;
            let runtime = &mut self.next_slots[slot_index];
            if runtime.respawn_remaining_seconds.is_none() {
                runtime.respawn_remaining_seconds = Some(config.respawn_delay_seconds);
            }
            runtime.neutralize();
        }
        self.waiting_slots = self
            .next_slots
            .iter()
            .filter(|slot| slot.respawn_remaining_seconds.is_some())
            .count();
        self.ready = true;
        Ok(PreparedBaselineDeaths {
            key: current_key,
            physics_world: physics.world(),
            source_state,
            config,
            next_slots: &self.next_slots,
            event_count: physics.events().len(),
        })
    }

    /// Whether the latest timer preparation produced a complete proposal.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Current sizes and retained allocation, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> BaselineLifecycleDiagnostics {
        BaselineLifecycleDiagnostics {
            slots: self.next_slots.len(),
            waiting_slots: self.waiting_slots,
            due_slots: self.due_slots.len(),
            next_slot_capacity: self.next_slots.capacity(),
            due_slot_capacity: self.due_slots.capacity(),
            world_slot_index_capacity: self.world_slot_indices.capacity(),
        }
    }

    fn prepared<'lifecycle, 'source>(
        &'lifecycle self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_state: &'source BaselineLifecycleState,
        fixed_dt: f64,
        config: BaselineLifecycleConfig,
    ) -> Result<PreparedBaselineTimers<'lifecycle, 'source>, BaselineLifecycleError> {
        if !self.ready {
            return Err(BaselineLifecycleError::ResultNotReady);
        }
        Ok(PreparedBaselineTimers {
            key,
            source_world,
            source_state,
            fixed_dt,
            config,
            next_slots: &self.next_slots,
            due_slots: &self.due_slots,
            diagnostics: self.diagnostics(),
        })
    }

    fn prepare_world_slot_indices(
        &mut self,
        world: &WorldState,
        slot_count: usize,
    ) -> Result<(), BaselineLifecycleError> {
        reserve_for(
            &mut self.world_slot_indices,
            slot_count,
            "baseline world-slot indexes",
        )?;
        self.world_slot_indices.resize(slot_count, MISSING_INDEX);
        for (snake_index, snake) in world.snakes.iter().enumerate() {
            if snake.kind != SnakeKind::Baseline {
                if snake.baseline_slot.is_some() {
                    return Err(BaselineLifecycleError::NonBaselineClaimsSlot {
                        snake_id: snake.id,
                    });
                }
                continue;
            }
            let slot = snake
                .baseline_slot
                .ok_or(BaselineLifecycleError::BaselineMissingSlot { snake_id: snake.id })?;
            let index =
                usize::try_from(slot).map_err(|_| BaselineLifecycleError::ArithmeticOverflow {
                    context: "baseline world slot index",
                })?;
            if index >= slot_count {
                return Err(BaselineLifecycleError::UnknownSlot(slot));
            }
            if self.world_slot_indices[index] != MISSING_INDEX {
                return Err(BaselineLifecycleError::DuplicateWorldSlot(slot));
            }
            self.world_slot_indices[index] = snake_index;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.next_slots.clear();
        self.due_slots.clear();
        self.world_slot_indices.clear();
        self.waiting_slots = 0;
        self.ready = false;
    }
}

fn validate_runtime_shape(
    state: &BaselineLifecycleState,
    config: BaselineLifecycleConfig,
) -> Result<(), BaselineLifecycleError> {
    if state.version != BASELINE_LIFECYCLE_VERSION {
        return Err(BaselineLifecycleError::InvalidStateVersion(state.version));
    }
    if state.slots.len() != config.slot_count {
        return Err(BaselineLifecycleError::SlotCountMismatch {
            actual: state.slots.len(),
            expected: config.slot_count,
        });
    }
    for (index, slot) in state.slots.iter().copied().enumerate() {
        validate_slot_runtime(slot, index)?;
    }
    Ok(())
}

fn validate_slot_runtime(
    slot: BaselineSlotRuntime,
    expected_index: usize,
) -> Result<(), BaselineLifecycleError> {
    let expected_slot =
        u32::try_from(expected_index).map_err(|_| BaselineLifecycleError::ArithmeticOverflow {
            context: "dense baseline slot identity",
        })?;
    if slot.slot != expected_slot {
        return Err(BaselineLifecycleError::NonDenseSlot {
            index: expected_index,
            slot: slot.slot,
        });
    }
    if slot.snake_id == 0 {
        return Err(BaselineLifecycleError::InvalidSnakeIdentity(slot.slot));
    }
    for (field, value) in [
        ("strategy_timer_seconds", slot.strategy_timer_seconds),
        ("wander_angle", slot.wander_angle),
        ("wander_timer_seconds", slot.wander_timer_seconds),
    ] {
        if !value.is_finite()
            || (field != "wander_angle" && value < 0.0)
            || (field == "wander_angle" && value.abs() > std::f64::consts::TAU)
        {
            return Err(BaselineLifecycleError::InvalidSlotScalar {
                slot: slot.slot,
                field,
            });
        }
    }
    if !slot.turn.is_finite() || !(-1.0..=1.0).contains(&slot.turn) {
        return Err(BaselineLifecycleError::InvalidSlotScalar {
            slot: slot.slot,
            field: "turn",
        });
    }
    if let Some(remaining) = slot.respawn_remaining_seconds {
        if !remaining.is_finite() || remaining <= 0.0 {
            return Err(BaselineLifecycleError::InvalidSlotScalar {
                slot: slot.slot,
                field: "respawn_remaining_seconds",
            });
        }
    }
    Ok(())
}

fn validate_world_slot_claims(
    world: &WorldState,
    slot_count: usize,
) -> Result<(), BaselineLifecycleError> {
    for snake in &world.snakes {
        if snake.kind != SnakeKind::Baseline {
            if snake.baseline_slot.is_none() {
                continue;
            }
            return Err(BaselineLifecycleError::NonBaselineClaimsSlot { snake_id: snake.id });
        }
        let slot = snake
            .baseline_slot
            .ok_or(BaselineLifecycleError::BaselineMissingSlot { snake_id: snake.id })?;
        if usize::try_from(slot).map_or(true, |index| index >= slot_count) {
            return Err(BaselineLifecycleError::UnknownSlot(slot));
        }
    }
    Ok(())
}

fn find_world_slot(world: &WorldState, slot: u32) -> Result<usize, BaselineLifecycleError> {
    let mut found = None;
    for (index, snake) in world.snakes.iter().enumerate() {
        if snake.kind != SnakeKind::Baseline || snake.baseline_slot != Some(slot) {
            continue;
        }
        if found.is_some() {
            return Err(BaselineLifecycleError::DuplicateWorldSlot(slot));
        }
        found = Some(index);
    }
    found.ok_or(BaselineLifecycleError::MissingWorldSlot(slot))
}

fn validate_death_events(
    world_after_physics: &WorldState,
    events: &[BaselineDeathEvent],
    source_state: &BaselineLifecycleState,
) -> Result<(), BaselineLifecycleError> {
    for pair in events.windows(2) {
        if pair[0].snake_id >= pair[1].snake_id {
            return Err(BaselineLifecycleError::NonCanonicalDeathEvents);
        }
    }
    for (event_index, event) in events.iter().enumerate() {
        if events[..event_index]
            .iter()
            .any(|prior| prior.slot == event.slot)
        {
            return Err(BaselineLifecycleError::DuplicateDeathSlot(event.slot));
        }
        let slot_index = usize::try_from(event.slot).map_err(|_| {
            BaselineLifecycleError::ArithmeticOverflow {
                context: "baseline death slot index",
            }
        })?;
        let runtime = source_state
            .slots
            .get(slot_index)
            .ok_or(BaselineLifecycleError::UnknownSlot(event.slot))?;
        if runtime.snake_id != event.snake_id {
            return Err(BaselineLifecycleError::SnakeIdentityMismatch {
                slot: event.slot,
                expected: runtime.snake_id,
                actual: event.snake_id,
            });
        }
        let world_index = find_world_slot(world_after_physics, event.slot)?;
        let snake = &world_after_physics.snakes[world_index];
        if snake.id != event.snake_id || snake.alive {
            return Err(BaselineLifecycleError::InvalidDeathEvent {
                slot: event.slot,
                snake_id: event.snake_id,
            });
        }
    }
    Ok(())
}

fn validate_step_key(key: PhysicsStepKey) -> Result<(), BaselineLifecycleError> {
    if key.world_epoch() == 0
        || key.generation() == 0
        || key.population_epoch() == 0
        || key.config_revision() == 0
        || key.operation_epoch() == 0
    {
        return Err(BaselineLifecycleError::InvalidStepKey);
    }
    Ok(())
}

fn validate_fixed_dt(fixed_dt: f64) -> Result<(), BaselineLifecycleError> {
    if !fixed_dt.is_finite() || fixed_dt <= 0.0 {
        return Err(BaselineLifecycleError::InvalidFixedDelta);
    }
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), BaselineLifecycleError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| BaselineLifecycleError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

/// Checked baseline lifecycle failure. No variant publishes partial authority.
#[derive(Clone, Debug, PartialEq)]
pub enum BaselineLifecycleError {
    InvalidStepKey,
    StepKeyMismatch {
        field: PhysicsStepKeyField,
    },
    SourceChanged {
        field: &'static str,
    },
    WorkingCopyChanged,
    InvalidConfig {
        field: &'static str,
    },
    InvalidFixedDelta,
    InvalidStateVersion(u32),
    SlotCountMismatch {
        actual: usize,
        expected: usize,
    },
    NonDenseSlot {
        index: usize,
        slot: u32,
    },
    InvalidSnakeIdentity(u32),
    InvalidSlotScalar {
        slot: u32,
        field: &'static str,
    },
    BaselineMissingSlot {
        snake_id: u64,
    },
    NonBaselineClaimsSlot {
        snake_id: u64,
    },
    UnknownSlot(u32),
    MissingWorldSlot(u32),
    DuplicateWorldSlot(u32),
    SnakeIdentityMismatch {
        slot: u32,
        expected: u64,
        actual: u64,
    },
    InitialSnakeNotAlive {
        slot: u32,
        snake_id: u64,
    },
    MissingWorldStrategy {
        slot: u32,
        snake_id: u64,
    },
    DuplicateSnakeIdentity(u64),
    LiveSnakeHasRespawnTimer {
        slot: u32,
        snake_id: u64,
    },
    WorldActionMismatch {
        slot: u32,
        snake_id: u64,
    },
    DeadSnakeMissingRespawnTimer {
        slot: u32,
        snake_id: u64,
    },
    NonFiniteTimer {
        slot: u32,
    },
    NonCanonicalDeathEvents,
    DuplicateDeathSlot(u32),
    InvalidDeathEvent {
        slot: u32,
        snake_id: u64,
    },
    RespawnsUnresolved {
        count: usize,
    },
    ArithmeticOverflow {
        context: &'static str,
    },
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    ResultNotReady,
}

impl Display for BaselineLifecycleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStepKey => write!(formatter, "invalid baseline fixed-step key"),
            Self::StepKeyMismatch { field } => {
                write!(formatter, "baseline fixed-step key changed at {field:?}")
            }
            Self::SourceChanged { field } => write!(formatter, "baseline source changed at {field}"),
            Self::WorkingCopyChanged => write!(formatter, "baseline working copy changed"),
            Self::InvalidConfig { field } => write!(formatter, "invalid baseline config: {field}"),
            Self::InvalidFixedDelta => write!(formatter, "invalid baseline fixed delta"),
            Self::InvalidStateVersion(version) => write!(formatter, "unsupported baseline state version {version}"),
            Self::SlotCountMismatch { actual, expected } => write!(formatter, "baseline state contains {actual} slots but config requires {expected}"),
            Self::NonDenseSlot { index, slot } => write!(formatter, "baseline slot index {index} contains identity {slot}"),
            Self::InvalidSnakeIdentity(slot) => write!(formatter, "baseline slot {slot} has invalid snake identity"),
            Self::InvalidSlotScalar { slot, field } => write!(formatter, "baseline slot {slot} has invalid {field}"),
            Self::BaselineMissingSlot { snake_id } => write!(formatter, "baseline snake {snake_id} has no stable slot"),
            Self::NonBaselineClaimsSlot { snake_id } => write!(formatter, "non-baseline snake {snake_id} claims a baseline slot"),
            Self::UnknownSlot(slot) => write!(formatter, "baseline slot {slot} is outside configured range"),
            Self::MissingWorldSlot(slot) => write!(formatter, "baseline slot {slot} has no world snake record"),
            Self::DuplicateWorldSlot(slot) => write!(formatter, "baseline slot {slot} has multiple world snake records"),
            Self::SnakeIdentityMismatch { slot, expected, actual } => write!(formatter, "baseline slot {slot} expected snake {expected} but found {actual}"),
            Self::InitialSnakeNotAlive { slot, snake_id } => write!(formatter, "initial baseline slot {slot} snake {snake_id} is not alive"),
            Self::MissingWorldStrategy { slot, snake_id } => write!(formatter, "baseline slot {slot} snake {snake_id} has no canonical world strategy"),
            Self::DuplicateSnakeIdentity(snake_id) => write!(formatter, "baseline lifecycle repeats snake identity {snake_id}"),
            Self::LiveSnakeHasRespawnTimer { slot, snake_id } => write!(formatter, "live baseline slot {slot} snake {snake_id} retains a respawn timer"),
            Self::WorldActionMismatch { slot, snake_id } => write!(formatter, "live baseline slot {slot} snake {snake_id} disagrees with its lifecycle action"),
            Self::DeadSnakeMissingRespawnTimer { slot, snake_id } => write!(formatter, "dead baseline slot {slot} snake {snake_id} has no respawn timer"),
            Self::NonFiniteTimer { slot } => write!(formatter, "baseline slot {slot} produced a non-finite timer"),
            Self::NonCanonicalDeathEvents => write!(formatter, "baseline death events are not in strict snake-ID order"),
            Self::DuplicateDeathSlot(slot) => write!(formatter, "baseline death events repeat slot {slot}"),
            Self::InvalidDeathEvent { slot, snake_id } => write!(formatter, "baseline death event for slot {slot} snake {snake_id} does not match a dead world record"),
            Self::RespawnsUnresolved { count } => write!(formatter, "{count} baseline respawns still require placement resolution"),
            Self::ArithmeticOverflow { context } => write!(formatter, "baseline arithmetic overflow: {context}"),
            Self::AllocationFailed { buffer, required } => write!(formatter, "failed to reserve {required} entries for {buffer}"),
            Self::ResultNotReady => write!(formatter, "baseline timer result is not ready"),
        }
    }
}

impl Error for BaselineLifecycleError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{BaselineStrategyState, BodyRange, SnakeState, WorldPoint};

    fn baseline(slot: u32, id: u64, alive: bool) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: id as u32,
            kind: SnakeKind::Baseline,
            alive,
            population_slot: None,
            brain: None,
            baseline_slot: Some(slot),
            baseline_strategy: Some(BaselineStrategyState::Roam),
            position: WorldPoint {
                x: id as f64,
                y: 0.0,
            },
            previous_position: WorldPoint {
                x: id as f64,
                y: 0.0,
            },
            direction: 0.0,
            radius: 9.0,
            speed: 0.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange { start: 0, len: 1 },
            skin: 2,
        }
    }

    fn world(entries: &[(u32, u64, bool)]) -> WorldState {
        let mut world = WorldState::default();
        for &(slot, id, alive) in entries {
            let mut snake = baseline(slot, id, alive);
            snake.body = BodyRange {
                start: world.body_points.len(),
                len: 1,
            };
            world.body_points.push(snake.position);
            world.snakes.push(snake);
        }
        world
    }

    fn config(count: usize) -> BaselineLifecycleConfig {
        BaselineLifecycleConfig {
            slot_count: count,
            ..BaselineLifecycleConfig::typescript_defaults()
        }
    }

    fn key() -> PhysicsStepKey {
        PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 7)
    }

    fn reset_key() -> PhysicsStepKey {
        PhysicsStepKey::new(2, 2, 3, 4, 5, [6; 32], 8)
    }

    #[test]
    fn generation_initialization_uses_dense_slots_independent_of_world_order() {
        let mut world = world(&[(2, 202, true), (0, 200, true), (1, 201, true)]);
        world.snakes[0].baseline_strategy = Some(BaselineStrategyState::Seek);
        world.snakes[0].turn = -0.25;
        world.snakes[0].input_boost = true;
        let state =
            BaselineLifecycleState::initialize_after_complete_spawn(config(3), &world).unwrap();
        assert_eq!(
            state
                .slots
                .iter()
                .map(|slot| slot.snake_id)
                .collect::<Vec<_>>(),
            vec![200, 201, 202]
        );
        assert_eq!(
            world.snakes[0].baseline_strategy,
            Some(BaselineStrategyState::Seek)
        );
        assert_eq!(state.slots[2].turn, -0.25);
        assert!(state.slots[2].boost);
        assert!(state
            .slots
            .iter()
            .all(|slot| slot.respawn_remaining_seconds.is_none()));
    }

    #[test]
    fn generation_initialization_rejects_missing_or_out_of_range_slots() {
        let missing = world(&[(0, 200, true)]);
        assert_eq!(
            BaselineLifecycleState::initialize_after_complete_spawn(config(2), &missing),
            Err(BaselineLifecycleError::MissingWorldSlot(1))
        );

        let extra = world(&[(0, 200, true), (1, 201, true)]);
        assert_eq!(
            BaselineLifecycleState::initialize_after_complete_spawn(config(1), &extra),
            Err(BaselineLifecycleError::UnknownSlot(1))
        );

        let invalid_id = world(&[(0, 0, true)]);
        assert_eq!(
            BaselineLifecycleState::initialize_after_complete_spawn(config(1), &invalid_id),
            Err(BaselineLifecycleError::InvalidSnakeIdentity(0))
        );

        let duplicate_id = world(&[(0, 200, true), (1, 200, true)]);
        assert_eq!(
            BaselineLifecycleState::initialize_after_complete_spawn(config(2), &duplicate_id),
            Err(BaselineLifecycleError::DuplicateSnakeIdentity(200))
        );

        let mut missing_strategy = world(&[(0, 200, true)]);
        missing_strategy.snakes[0].baseline_strategy = None;
        assert_eq!(
            BaselineLifecycleState::initialize_after_complete_spawn(config(1), &missing_strategy,),
            Err(BaselineLifecycleError::MissingWorldStrategy {
                slot: 0,
                snake_id: 200,
            })
        );

        let mut timed_strategy_without_timer = world(&[(0, 200, true)]);
        timed_strategy_without_timer.snakes[0].baseline_strategy =
            Some(BaselineStrategyState::Avoid);
        assert_eq!(
            BaselineLifecycleState::initialize_after_complete_spawn(
                config(1),
                &timed_strategy_without_timer,
            ),
            Err(BaselineLifecycleError::InvalidSlotScalar {
                slot: 0,
                field: "strategy_timer_seconds",
            })
        );
    }

    #[test]
    fn committed_death_starts_full_delay_and_duplicate_does_not_restart_it() {
        let physics_world = world(&[(0, 200, false)]);
        let mut state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.75,
                boost: true,
                respawn_remaining_seconds: None,
            }],
        };
        let event = [BaselineDeathEvent {
            slot: 0,
            snake_id: 200,
        }];
        let proof = PreparedPhysicsBaselineDeaths::test_fixture(key(), &physics_world, &event);
        let mut workspace = BaselineLifecycleWorkspace::new();
        let prepared = workspace
            .prepare_committed_deaths(proof, key(), &state, config(1))
            .unwrap();
        assert_eq!(prepared.key(), key());
        assert!(std::ptr::eq(prepared.physics_world(), &physics_world));
        assert_eq!(prepared.event_count(), 1);
        let mut target = state.clone();
        prepared
            .apply_to_working_copy(key(), &state, config(1), &mut target)
            .unwrap();
        let after_first = target.clone();
        assert_eq!(
            prepared.apply_to_working_copy(key(), &state, config(1), &mut target),
            Err(BaselineLifecycleError::WorkingCopyChanged)
        );
        assert_eq!(target, after_first);
        state = target;
        assert_eq!(state.slots[0].respawn_remaining_seconds, Some(20.0));
        assert_eq!(state.slots[0].turn, 0.0);
        assert!(!state.slots[0].boost);

        state.slots[0].respawn_remaining_seconds = Some(5.0);
        let repeated_key = PhysicsStepKey::new(1, 2, 4, 4, 5, [6; 32], 8);
        let repeated_proof =
            PreparedPhysicsBaselineDeaths::test_fixture(repeated_key, &physics_world, &event);
        let prepared = workspace
            .prepare_committed_deaths(repeated_proof, repeated_key, &state, config(1))
            .unwrap();
        let mut target = state.clone();
        prepared
            .apply_to_working_copy(repeated_key, &state, config(1), &mut target)
            .unwrap();
        state = target;
        assert_eq!(state.slots[0].respawn_remaining_seconds, Some(5.0));
    }

    #[test]
    fn invalid_later_death_event_rejects_before_any_timer_write() {
        let initial_world = world(&[(0, 200, true), (1, 201, true)]);
        let physics_world = world(&[(0, 200, false), (1, 201, true)]);
        let state =
            BaselineLifecycleState::initialize_after_complete_spawn(config(2), &initial_world)
                .unwrap();
        let before = state.clone();
        let events = [
            BaselineDeathEvent {
                slot: 0,
                snake_id: 200,
            },
            BaselineDeathEvent {
                slot: 1,
                snake_id: 201,
            },
        ];
        let proof = PreparedPhysicsBaselineDeaths::test_fixture(key(), &physics_world, &events);
        let mut workspace = BaselineLifecycleWorkspace::new();
        assert!(matches!(
            workspace.prepare_committed_deaths(proof, key(), &state, config(2)),
            Err(BaselineLifecycleError::InvalidDeathEvent {
                slot: 1,
                snake_id: 201,
            })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(state, before);
    }

    #[test]
    fn death_proposal_rejects_a_stale_world_epoch_with_reused_slot_and_snake_id() {
        let physics_world = world(&[(0, 200, false)]);
        let state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.25,
                boost: true,
                respawn_remaining_seconds: None,
            }],
        };
        let events = [BaselineDeathEvent {
            slot: 0,
            snake_id: 200,
        }];
        let proof = PreparedPhysicsBaselineDeaths::test_fixture(key(), &physics_world, &events);
        let mut workspace = BaselineLifecycleWorkspace::new();
        assert!(matches!(
            workspace.prepare_committed_deaths(proof, reset_key(), &state, config(1)),
            Err(BaselineLifecycleError::StepKeyMismatch {
                field: PhysicsStepKeyField::WorldEpoch,
            })
        ));
        assert!(!workspace.is_ready());

        let prepared = workspace
            .prepare_committed_deaths(proof, key(), &state, config(1))
            .unwrap();
        let mut target = state.clone();
        assert_eq!(
            prepared.apply_to_working_copy(reset_key(), &state, config(1), &mut target),
            Err(BaselineLifecycleError::StepKeyMismatch {
                field: PhysicsStepKeyField::WorldEpoch,
            })
        );
        assert_eq!(target, state);
    }

    #[test]
    fn notified_and_missed_deaths_have_the_current_distinct_first_tick_timing() {
        let world = world(&[(0, 200, false), (1, 201, false)]);
        let state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![
                BaselineSlotRuntime {
                    slot: 0,
                    snake_id: 200,
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.0,
                    boost: false,
                    respawn_remaining_seconds: Some(20.0),
                },
                BaselineSlotRuntime {
                    slot: 1,
                    snake_id: 201,
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.5,
                    boost: true,
                    respawn_remaining_seconds: None,
                },
            ],
        };
        let mut workspace = BaselineLifecycleWorkspace::new();
        let prepared = workspace
            .prepare_timers(key(), &world, &state, 1.0, config(2))
            .unwrap();
        assert_eq!(
            prepared.next_slots()[0].respawn_remaining_seconds,
            Some(19.0)
        );
        assert_eq!(
            prepared.next_slots()[1].respawn_remaining_seconds,
            Some(20.0)
        );
        assert_eq!(prepared.next_slots()[1].turn, 0.0);
        assert!(!prepared.next_slots()[1].boost);
        assert!(prepared.due_slots().is_empty());
    }

    #[test]
    fn timer_staging_is_independent_of_world_container_order() {
        let forward_world = world(&[(0, 200, false), (1, 201, false)]);
        let reverse_world = world(&[(1, 201, false), (0, 200, false)]);
        let state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![
                BaselineSlotRuntime {
                    slot: 0,
                    snake_id: 200,
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.0,
                    boost: false,
                    respawn_remaining_seconds: Some(1.0),
                },
                BaselineSlotRuntime {
                    slot: 1,
                    snake_id: 201,
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.5,
                    boost: true,
                    respawn_remaining_seconds: None,
                },
            ],
        };
        let mut forward_workspace = BaselineLifecycleWorkspace::new();
        let forward = forward_workspace
            .prepare_timers(key(), &forward_world, &state, 1.0, config(2))
            .unwrap();
        let forward_slots = forward.next_slots().to_vec();
        let forward_due = forward.due_slots().to_vec();
        let mut reverse_workspace = BaselineLifecycleWorkspace::new();
        let reverse = reverse_workspace
            .prepare_timers(key(), &reverse_world, &state, 1.0, config(2))
            .unwrap();
        assert_eq!(reverse.next_slots(), forward_slots);
        assert_eq!(reverse.due_slots(), forward_due);
        assert_eq!(reverse.due_slots(), &[0]);
    }

    #[test]
    fn lowering_delay_caps_then_subtracts_while_raising_never_extends() {
        let world = world(&[(0, 200, false)]);
        let mut state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(10.0),
            }],
        };
        let mut lowered = config(1);
        lowered.respawn_delay_seconds = 5.0;
        let mut workspace = BaselineLifecycleWorkspace::new();
        let first = workspace
            .prepare_timers(key(), &world, &state, 1.0, lowered)
            .unwrap();
        assert_eq!(first.next_slots()[0].respawn_remaining_seconds, Some(4.0));
        state.slots[0] = first.next_slots()[0];
        let mut raised = config(1);
        raised.respawn_delay_seconds = 30.0;
        let second = workspace
            .prepare_timers(key(), &world, &state, 1.0, raised)
            .unwrap();
        assert_eq!(second.next_slots()[0].respawn_remaining_seconds, Some(3.0));
    }

    #[test]
    fn exact_deadline_requires_explicit_respawn_resolution() {
        let world = world(&[(0, 200, false)]);
        let state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.1,
                wander_angle: 0.2,
                wander_timer_seconds: 0.3,
                turn: 0.4,
                boost: true,
                respawn_remaining_seconds: Some(1.0),
            }],
        };
        let mut workspace = BaselineLifecycleWorkspace::new();
        let prepared = workspace
            .prepare_timers(key(), &world, &state, 1.0, config(1))
            .unwrap();
        assert_eq!(prepared.due_slots(), &[0]);
        assert_eq!(
            prepared.next_slots()[0].respawn_remaining_seconds,
            Some(0.0)
        );
        assert!(prepared.requires_respawn_resolution());
        let mut target = state.clone();
        assert_eq!(
            prepared.apply_without_due_respawns(key(), &world, &state, 1.0, config(1), &mut target),
            Err(BaselineLifecycleError::RespawnsUnresolved { count: 1 })
        );
        assert_eq!(target, state);
    }

    #[test]
    fn no_due_timer_result_applies_once_after_complete_source_validation() {
        let world = world(&[(0, 200, false)]);
        let state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(10.0),
            }],
        };
        let mut workspace = BaselineLifecycleWorkspace::new();
        let prepared = workspace
            .prepare_timers(key(), &world, &state, 1.0, config(1))
            .unwrap();
        let mut target = state.clone();
        prepared
            .apply_without_due_respawns(key(), &world, &state, 1.0, config(1), &mut target)
            .unwrap();
        assert_eq!(target.slots[0].respawn_remaining_seconds, Some(9.0));
        let after_first = target.clone();
        assert_eq!(
            prepared.apply_without_due_respawns(key(), &world, &state, 1.0, config(1), &mut target),
            Err(BaselineLifecycleError::WorkingCopyChanged)
        );
        assert_eq!(target, after_first);

        let other_world = world.clone();
        let mut fresh_target = state.clone();
        assert_eq!(
            prepared.apply_without_due_respawns(
                key(),
                &other_world,
                &state,
                1.0,
                config(1),
                &mut fresh_target
            ),
            Err(BaselineLifecycleError::SourceChanged { field: "world" })
        );
        assert_eq!(fresh_target, state);
    }

    #[test]
    fn prepared_timer_result_rejects_every_stale_provenance_input_without_writes() {
        let source_world = world(&[(0, 200, false)]);
        let source_state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 200,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 0.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: Some(10.0),
            }],
        };
        let mut workspace = BaselineLifecycleWorkspace::new();
        let prepared = workspace
            .prepare_timers(key(), &source_world, &source_state, 1.0, config(1))
            .unwrap();

        let stale_state = source_state.clone();
        let mut target = source_state.clone();
        assert_eq!(
            prepared.apply_without_due_respawns(
                key(),
                &source_world,
                &stale_state,
                1.0,
                config(1),
                &mut target,
            ),
            Err(BaselineLifecycleError::SourceChanged {
                field: "baseline lifecycle",
            })
        );
        assert_eq!(target, source_state);

        let stale_key = PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 8);
        assert_eq!(
            prepared.apply_without_due_respawns(
                stale_key,
                &source_world,
                &source_state,
                1.0,
                config(1),
                &mut target,
            ),
            Err(BaselineLifecycleError::StepKeyMismatch {
                field: PhysicsStepKeyField::OperationEpoch,
            })
        );
        assert_eq!(target, source_state);

        assert_eq!(
            prepared.apply_without_due_respawns(
                key(),
                &source_world,
                &source_state,
                0.5,
                config(1),
                &mut target,
            ),
            Err(BaselineLifecycleError::SourceChanged {
                field: "fixed delta",
            })
        );
        assert_eq!(target, source_state);

        let mut stale_config = config(1);
        stale_config.respawn_delay_seconds = 19.0;
        assert_eq!(
            prepared.apply_without_due_respawns(
                key(),
                &source_world,
                &source_state,
                1.0,
                stale_config,
                &mut target,
            ),
            Err(BaselineLifecycleError::SourceChanged { field: "config" })
        );
        assert_eq!(target, source_state);
    }

    #[test]
    fn failed_preparation_is_unready_and_a_valid_retry_clears_partial_staging() {
        let source_world = world(&[(0, 200, false), (1, 201, true)]);
        let mut invalid_state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![
                BaselineSlotRuntime {
                    slot: 0,
                    snake_id: 200,
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.0,
                    boost: false,
                    respawn_remaining_seconds: Some(10.0),
                },
                BaselineSlotRuntime {
                    slot: 1,
                    snake_id: 201,
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.0,
                    boost: false,
                    respawn_remaining_seconds: Some(10.0),
                },
            ],
        };
        let mut workspace = BaselineLifecycleWorkspace::new();
        assert!(matches!(
            workspace.prepare_timers(key(), &source_world, &invalid_state, 1.0, config(2)),
            Err(BaselineLifecycleError::LiveSnakeHasRespawnTimer {
                slot: 1,
                snake_id: 201,
            })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(workspace.diagnostics().slots, 1);

        invalid_state.slots[1].respawn_remaining_seconds = None;
        let prepared = workspace
            .prepare_timers(key(), &source_world, &invalid_state, 1.0, config(2))
            .unwrap();
        assert_eq!(prepared.next_slots().len(), 2);
        assert!(prepared.due_slots().is_empty());
        assert_eq!(
            prepared.next_slots()[0].respawn_remaining_seconds,
            Some(9.0)
        );
        assert_eq!(prepared.next_slots()[1].respawn_remaining_seconds, None);
    }

    #[test]
    fn warm_timer_preparation_reuses_every_owned_vector() {
        let world = world(
            &(0..64)
                .map(|slot| (slot, 200 + u64::from(slot), slot % 2 == 0))
                .collect::<Vec<_>>(),
        );
        let state = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: (0..64)
                .map(|slot| BaselineSlotRuntime {
                    slot,
                    snake_id: 200 + u64::from(slot),
                    strategy_timer_seconds: 0.0,
                    wander_angle: 0.0,
                    wander_timer_seconds: 0.0,
                    turn: 0.0,
                    boost: false,
                    respawn_remaining_seconds: (slot % 2 != 0).then_some(10.0),
                })
                .collect(),
        };
        let mut workspace = BaselineLifecycleWorkspace::new();
        let first = workspace
            .prepare_timers(key(), &world, &state, 1.0 / 60.0, config(64))
            .unwrap()
            .diagnostics();
        for _ in 0..24 {
            let next = workspace
                .prepare_timers(key(), &world, &state, 1.0 / 60.0, config(64))
                .unwrap()
                .diagnostics();
            assert_eq!(next, first);
            assert!(next.next_slot_capacity >= 64);
            assert!(next.due_slot_capacity >= 64);
            assert!(next.world_slot_index_capacity >= 64);
        }
    }
}
