//! Deterministic once-per-fixed-step generation and snake accounting.
//!
//! The current TypeScript world advances generation time, then increments the
//! age and survival points of every live snake before ambient generation and
//! controller sampling. This module preserves that boundary while staging all
//! scalar changes against one immutable authority/config/operation identity.

use super::physics::{PhysicsStepKey, PhysicsStepKeyField};
use super::state::{SnakeState, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First Rust fixed-step accounting algorithm identity.
pub const STEP_ACCOUNTING_ALGORITHM_VERSION: u32 = 1;

/// Versioned settings used by once-per-step scalar accounting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepAccountingConfig {
    /// Versioned formula and ordering identity.
    pub algorithm_version: u32,
    /// Points awarded per simulated second that a snake remains alive.
    pub points_per_second_alive: f64,
}

impl StepAccountingConfig {
    /// Current TypeScript defaults.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            algorithm_version: STEP_ACCOUNTING_ALGORITHM_VERSION,
            points_per_second_alive: 0.6,
        }
    }

    fn validate(self) -> Result<(), StepAccountingError> {
        if self.algorithm_version != STEP_ACCOUNTING_ALGORITHM_VERSION {
            return Err(StepAccountingError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        if !self.points_per_second_alive.is_finite()
            || !(0.0..=10.0).contains(&self.points_per_second_alive)
        {
            return Err(StepAccountingError::InvalidConfig {
                field: "points_per_second_alive",
            });
        }
        Ok(())
    }

    /// Compute the age and survival-points result for one live snake.
    ///
    /// Baseline respawn uses the same formula for a newborn admitted before
    /// the shared pre-movement observation boundary.
    pub fn advance_live_snake(
        self,
        snake: &SnakeState,
        fixed_dt: f64,
    ) -> Result<(f64, f64), StepAccountingError> {
        self.validate()?;
        validate_fixed_dt(fixed_dt)?;
        self.advance_live_snake_prevalidated(snake, fixed_dt)
    }

    fn advance_live_snake_prevalidated(
        self,
        snake: &SnakeState,
        fixed_dt: f64,
    ) -> Result<(f64, f64), StepAccountingError> {
        if !snake.alive {
            return Err(StepAccountingError::SnakeNotAlive { snake_id: snake.id });
        }
        if snake.body.len == 0 {
            return Err(StepAccountingError::AliveSnakeHasNoBody { snake_id: snake.id });
        }
        if !snake.age_seconds.is_finite() || snake.age_seconds < 0.0 {
            return Err(StepAccountingError::InvalidSnakeScalar {
                snake_id: snake.id,
                field: "age_seconds",
            });
        }
        if !snake.points.is_finite() || snake.points < 0.0 {
            return Err(StepAccountingError::InvalidSnakeScalar {
                snake_id: snake.id,
                field: "points",
            });
        }
        let next_age = snake.age_seconds + fixed_dt;
        let awarded_points = fixed_dt * self.points_per_second_alive;
        let next_points = snake.points + awarded_points;
        if !next_age.is_finite() {
            return Err(StepAccountingError::NonFiniteGenerated {
                snake_id: snake.id,
                field: "age_seconds",
            });
        }
        if !awarded_points.is_finite() || !next_points.is_finite() {
            return Err(StepAccountingError::NonFiniteGenerated {
                snake_id: snake.id,
                field: "points",
            });
        }
        Ok((next_age, next_points))
    }
}

/// One stable-ID-ordered live-snake scalar update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnakeAccountingUpdate {
    /// Stable internal snake identity.
    pub snake_id: u64,
    /// Source-array position used only for the matching transaction copy.
    pub source_index: usize,
    /// Source age captured at preparation.
    pub previous_age_seconds: f64,
    /// Source points captured at preparation.
    pub previous_points: f64,
    /// Age after one fixed step of accounting.
    pub next_age_seconds: f64,
    /// Points after one fixed step of survival reward.
    pub next_points: f64,
}

/// Size and retained-allocation diagnostics for the latest preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepAccountingDiagnostics {
    /// Total source snake records inspected.
    pub source_snakes: usize,
    /// Live snakes receiving one update.
    pub updated_snakes: usize,
    /// Retained canonical-order capacity.
    pub order_capacity: usize,
    /// Retained scalar-update capacity.
    pub update_capacity: usize,
}

/// Read-only complete scalar proposal for the future whole-step transaction.
#[derive(Clone, Copy, Debug)]
pub struct PreparedStepAccounting<'accounting, 'source> {
    key: PhysicsStepKey,
    source_world: &'source WorldState,
    source_elapsed_seconds: f64,
    fixed_dt: f64,
    config: StepAccountingConfig,
    maximum_snakes: usize,
    updates: &'accounting [SnakeAccountingUpdate],
    next_elapsed_seconds: f64,
    diagnostics: StepAccountingDiagnostics,
}

impl<'accounting, 'source> PreparedStepAccounting<'accounting, 'source> {
    /// Complete authority/config/operation identity prepared.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Immutable source boundary used to compute the proposal.
    #[must_use]
    pub const fn source_world(self) -> &'source WorldState {
        self.source_world
    }

    /// Stable-ID-ordered live-snake changes.
    #[must_use]
    pub const fn updates(self) -> &'accounting [SnakeAccountingUpdate] {
        self.updates
    }

    /// Generation elapsed time after exactly one fixed increment.
    #[must_use]
    pub const fn next_elapsed_seconds(self) -> f64 {
        self.next_elapsed_seconds
    }

    /// Current size and retained-allocation diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> StepAccountingDiagnostics {
        self.diagnostics
    }

    /// Revalidate every source input before joining this result to later work.
    #[allow(clippy::too_many_arguments)]
    pub fn validate_current(
        self,
        current_key: PhysicsStepKey,
        current_world: &WorldState,
        current_elapsed_seconds: f64,
        current_fixed_dt: f64,
        current_config: StepAccountingConfig,
        current_maximum_snakes: usize,
    ) -> Result<(), StepAccountingError> {
        if let Some(field) = self.key.first_mismatch(current_key) {
            return Err(StepAccountingError::StepKeyMismatch { field });
        }
        if !std::ptr::eq(self.source_world, current_world) {
            return Err(StepAccountingError::SourceChanged { field: "world" });
        }
        if self.source_elapsed_seconds.to_bits() != current_elapsed_seconds.to_bits() {
            return Err(StepAccountingError::SourceChanged {
                field: "generation elapsed time",
            });
        }
        if self.fixed_dt.to_bits() != current_fixed_dt.to_bits() {
            return Err(StepAccountingError::SourceChanged {
                field: "fixed delta",
            });
        }
        if self.config != current_config {
            return Err(StepAccountingError::SourceChanged { field: "config" });
        }
        if self.maximum_snakes != current_maximum_snakes {
            return Err(StepAccountingError::SourceChanged {
                field: "snake capacity",
            });
        }
        Ok(())
    }

    /// Apply this proposal to the coordinator's matching pre-step world copy
    /// after a full no-write preflight of every field this phase reads/writes.
    ///
    /// This does not publish authority. The later fixed-step coordinator owns
    /// the working copy and performs the one authoritative swap only after all
    /// phases succeed.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_to_working_copy(
        self,
        current_key: PhysicsStepKey,
        current_source_world: &WorldState,
        current_elapsed_seconds: f64,
        current_fixed_dt: f64,
        current_config: StepAccountingConfig,
        current_maximum_snakes: usize,
        target_world: &mut WorldState,
    ) -> Result<f64, StepAccountingError> {
        self.validate_current(
            current_key,
            current_source_world,
            current_elapsed_seconds,
            current_fixed_dt,
            current_config,
            current_maximum_snakes,
        )?;
        if target_world.snakes.len() != self.source_world.snakes.len()
            || target_world.body_points.len() != self.source_world.body_points.len()
            || target_world.pellets.len() != self.source_world.pellets.len()
            || target_world.controller_leases != self.source_world.controller_leases
        {
            return Err(StepAccountingError::WorkingCopyChanged {
                field: "world shape or leases",
            });
        }
        if target_world.snakes != self.source_world.snakes {
            return Err(StepAccountingError::WorkingCopyChanged { field: "snakes" });
        }
        for update in self.updates {
            let snake = target_world.snakes.get(update.source_index).ok_or(
                StepAccountingError::WorkingCopyChanged {
                    field: "snake index",
                },
            )?;
            if snake.id != update.snake_id
                || !snake.alive
                || snake.age_seconds.to_bits() != update.previous_age_seconds.to_bits()
                || snake.points.to_bits() != update.previous_points.to_bits()
            {
                return Err(StepAccountingError::WorkingCopyChanged {
                    field: "snake scalar snapshot",
                });
            }
        }
        for update in self.updates {
            let snake = &mut target_world.snakes[update.source_index];
            snake.age_seconds = update.next_age_seconds;
            snake.points = update.next_points;
        }
        Ok(self.next_elapsed_seconds)
    }
}

/// Reusable, non-authoritative pre-step accounting scratch.
#[derive(Debug, Default)]
pub struct StepAccountingWorkspace {
    order: Vec<usize>,
    updates: Vec<SnakeAccountingUpdate>,
    next_elapsed_seconds: f64,
    source_snakes: usize,
    ready: bool,
}

impl StepAccountingWorkspace {
    /// Construct empty reusable scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage generation time, age and survival points for one fixed step.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'accounting, 'source>(
        &'accounting mut self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_elapsed_seconds: f64,
        fixed_dt: f64,
        config: StepAccountingConfig,
        maximum_snakes: usize,
    ) -> Result<PreparedStepAccounting<'accounting, 'source>, StepAccountingError> {
        self.clear();
        validate_step_key(key)?;
        config.validate()?;
        validate_fixed_dt(fixed_dt)?;
        if !source_elapsed_seconds.is_finite() || source_elapsed_seconds < 0.0 {
            return Err(StepAccountingError::InvalidBoundary {
                field: "generation elapsed time",
            });
        }
        if source_world.snakes.len() > maximum_snakes {
            return Err(StepAccountingError::SnakeCapacityExceeded {
                actual: source_world.snakes.len(),
                maximum: maximum_snakes,
            });
        }
        reserve_for(
            &mut self.order,
            source_world.snakes.len(),
            "canonical snake order",
        )?;
        reserve_for(
            &mut self.updates,
            source_world.snakes.len(),
            "snake scalar updates",
        )?;
        self.order.extend(0..source_world.snakes.len());
        self.order
            .sort_unstable_by_key(|&index| source_world.snakes[index].id);
        let mut previous_id = None;
        for &source_index in &self.order {
            let snake = &source_world.snakes[source_index];
            if snake.id == 0 {
                return Err(StepAccountingError::InvalidSnakeId);
            }
            if previous_id == Some(snake.id) {
                return Err(StepAccountingError::DuplicateSnakeId(snake.id));
            }
            previous_id = Some(snake.id);
            if !snake.age_seconds.is_finite() || snake.age_seconds < 0.0 {
                return Err(StepAccountingError::InvalidSnakeScalar {
                    snake_id: snake.id,
                    field: "age_seconds",
                });
            }
            if !snake.points.is_finite() || snake.points < 0.0 {
                return Err(StepAccountingError::InvalidSnakeScalar {
                    snake_id: snake.id,
                    field: "points",
                });
            }
            if !snake.alive {
                continue;
            }
            let (next_age_seconds, next_points) =
                config.advance_live_snake_prevalidated(snake, fixed_dt)?;
            self.updates.push(SnakeAccountingUpdate {
                snake_id: snake.id,
                source_index,
                previous_age_seconds: snake.age_seconds,
                previous_points: snake.points,
                next_age_seconds,
                next_points,
            });
        }
        let next_elapsed_seconds = source_elapsed_seconds + fixed_dt;
        if !next_elapsed_seconds.is_finite() {
            return Err(StepAccountingError::NonFiniteGenerationTime);
        }
        self.source_snakes = source_world.snakes.len();
        self.next_elapsed_seconds = next_elapsed_seconds;
        self.ready = true;
        self.prepared(
            key,
            source_world,
            source_elapsed_seconds,
            fixed_dt,
            config,
            maximum_snakes,
        )
    }

    /// Whether the latest preparation produced a complete proposal.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Current sizes and retained allocation, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> StepAccountingDiagnostics {
        StepAccountingDiagnostics {
            source_snakes: self.source_snakes,
            updated_snakes: self.updates.len(),
            order_capacity: self.order.capacity(),
            update_capacity: self.updates.capacity(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepared<'accounting, 'source>(
        &'accounting self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_elapsed_seconds: f64,
        fixed_dt: f64,
        config: StepAccountingConfig,
        maximum_snakes: usize,
    ) -> Result<PreparedStepAccounting<'accounting, 'source>, StepAccountingError> {
        if !self.ready {
            return Err(StepAccountingError::ResultNotReady);
        }
        Ok(PreparedStepAccounting {
            key,
            source_world,
            source_elapsed_seconds,
            fixed_dt,
            config,
            maximum_snakes,
            updates: &self.updates,
            next_elapsed_seconds: self.next_elapsed_seconds,
            diagnostics: self.diagnostics(),
        })
    }

    fn clear(&mut self) {
        self.order.clear();
        self.updates.clear();
        self.next_elapsed_seconds = 0.0;
        self.source_snakes = 0;
        self.ready = false;
    }
}

fn validate_step_key(key: PhysicsStepKey) -> Result<(), StepAccountingError> {
    if key.world_epoch() == 0
        || key.generation() == 0
        || key.population_epoch() == 0
        || key.config_revision() == 0
        || key.operation_epoch() == 0
    {
        return Err(StepAccountingError::InvalidStepKey);
    }
    Ok(())
}

fn validate_fixed_dt(fixed_dt: f64) -> Result<(), StepAccountingError> {
    if !fixed_dt.is_finite() || fixed_dt <= 0.0 {
        return Err(StepAccountingError::InvalidBoundary {
            field: "fixed delta",
        });
    }
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), StepAccountingError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| StepAccountingError::AllocationFailed { context, required })?;
    }
    Ok(())
}

/// Checked pre-step accounting failure. No variant publishes partial authority.
#[derive(Clone, Debug, PartialEq)]
pub enum StepAccountingError {
    /// One fixed-step identity component is zero or otherwise invalid.
    InvalidStepKey,
    /// A newer complete authority/config/operation identity superseded staging.
    StepKeyMismatch { field: PhysicsStepKeyField },
    /// A non-key source input changed after preparation.
    SourceChanged { field: &'static str },
    /// The coordinator's mutable scratch no longer matches the admitted source.
    WorkingCopyChanged { field: &'static str },
    /// A projected setting is invalid.
    InvalidConfig { field: &'static str },
    /// A fixed-step boundary scalar is invalid.
    InvalidBoundary { field: &'static str },
    /// Source snake count exceeds admitted storage.
    SnakeCapacityExceeded { actual: usize, maximum: usize },
    /// Stable snake identity zero is invalid.
    InvalidSnakeId,
    /// Two source records reuse one stable snake identity.
    DuplicateSnakeId(u64),
    /// A live-only helper received a dead snake.
    SnakeNotAlive { snake_id: u64 },
    /// Admitted live snakes must already own at least one body point.
    AliveSnakeHasNoBody { snake_id: u64 },
    /// One source scalar is invalid.
    InvalidSnakeScalar { snake_id: u64, field: &'static str },
    /// One derived snake scalar became NaN or infinite.
    NonFiniteGenerated { snake_id: u64, field: &'static str },
    /// Generation elapsed time overflowed.
    NonFiniteGenerationTime,
    /// Reusable scratch could not reserve its checked capacity.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// No complete scalar result is currently staged.
    ResultNotReady,
}

impl Display for StepAccountingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStepKey => write!(formatter, "invalid fixed-step accounting key"),
            Self::StepKeyMismatch { field } => {
                write!(formatter, "fixed-step accounting key changed at {field:?}")
            }
            Self::SourceChanged { field } => {
                write!(formatter, "fixed-step accounting source changed at {field}")
            }
            Self::WorkingCopyChanged { field } => {
                write!(formatter, "fixed-step working copy changed at {field}")
            }
            Self::InvalidConfig { field } => {
                write!(formatter, "invalid fixed-step accounting config: {field}")
            }
            Self::InvalidBoundary { field } => {
                write!(formatter, "invalid fixed-step accounting boundary: {field}")
            }
            Self::SnakeCapacityExceeded { actual, maximum } => write!(
                formatter,
                "source snake count {actual} exceeds admitted maximum {maximum}"
            ),
            Self::InvalidSnakeId => write!(formatter, "snake identity zero is invalid"),
            Self::DuplicateSnakeId(snake_id) => {
                write!(formatter, "duplicate stable snake identity {snake_id}")
            }
            Self::SnakeNotAlive { snake_id } => {
                write!(formatter, "snake {snake_id} is not alive")
            }
            Self::AliveSnakeHasNoBody { snake_id } => {
                write!(formatter, "live snake {snake_id} has no admitted body")
            }
            Self::InvalidSnakeScalar { snake_id, field } => {
                write!(formatter, "snake {snake_id} has invalid {field}")
            }
            Self::NonFiniteGenerated { snake_id, field } => {
                write!(formatter, "snake {snake_id} produced non-finite {field}")
            }
            Self::NonFiniteGenerationTime => {
                write!(formatter, "generation elapsed time became non-finite")
            }
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "fixed-step accounting allocation failed for {context} ({required} values)"
            ),
            Self::ResultNotReady => write!(formatter, "fixed-step accounting result is not ready"),
        }
    }
}

impl Error for StepAccountingError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{BodyRange, SnakeKind, WorldPoint};

    fn snake(id: u64, alive: bool, age_seconds: f64, points: f64) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: id as u32,
            kind: SnakeKind::Evolved,
            alive,
            population_slot: Some(id as u32),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
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
            age_seconds,
            food: 0.0,
            points,
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
            skin: 0,
        }
    }

    fn world(mut snakes: Vec<SnakeState>) -> WorldState {
        let mut world = WorldState::default();
        for snake in &mut snakes {
            snake.body = BodyRange {
                start: world.body_points.len(),
                len: 1,
            };
            world.body_points.push(snake.position);
        }
        world.snakes = snakes;
        world
    }

    fn key() -> PhysicsStepKey {
        PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 7)
    }

    #[test]
    fn current_typescript_formula_updates_only_live_snakes_once_in_stable_id_order() {
        let source = world(vec![
            snake(7, true, 1.0, 5.0),
            snake(3, false, 4.0, 8.0),
            snake(2, true, 2.0, 10.0),
        ]);
        let source_copy = source.clone();
        let mut workspace = StepAccountingWorkspace::new();
        let prepared = workspace
            .prepare(
                key(),
                &source,
                12.0,
                0.25,
                StepAccountingConfig::typescript_defaults(),
                3,
            )
            .expect("current TypeScript scalar formula should stage");
        assert_eq!(
            prepared
                .updates()
                .iter()
                .map(|update| update.snake_id)
                .collect::<Vec<_>>(),
            vec![2, 7]
        );
        assert_eq!(prepared.updates()[0].next_age_seconds, 2.25);
        assert_eq!(prepared.updates()[0].next_points, 10.15);
        assert_eq!(prepared.updates()[1].next_age_seconds, 1.25);
        assert_eq!(prepared.updates()[1].next_points, 5.15);
        assert_eq!(prepared.next_elapsed_seconds(), 12.25);
        assert_eq!(source, source_copy);

        let mut target = source.clone();
        let elapsed = prepared
            .apply_to_working_copy(
                key(),
                &source,
                12.0,
                0.25,
                StepAccountingConfig::typescript_defaults(),
                3,
                &mut target,
            )
            .expect("matching transaction copy should accept all scalar changes");
        assert_eq!(elapsed, 12.25);
        assert_eq!(target.snakes[0].age_seconds, 1.25);
        assert_eq!(target.snakes[0].points, 5.15);
        assert_eq!(target.snakes[1], source.snakes[1]);
        assert_eq!(target.snakes[2].age_seconds, 2.25);
        assert_eq!(target.snakes[2].points, 10.15);
        assert_eq!(target.body_points, source.body_points);
    }

    #[test]
    fn reversed_container_order_preserves_per_id_results() {
        let run = |mut snakes: Vec<SnakeState>| {
            let source = world(std::mem::take(&mut snakes));
            let mut workspace = StepAccountingWorkspace::new();
            workspace
                .prepare(
                    key(),
                    &source,
                    0.0,
                    1.0 / 60.0,
                    StepAccountingConfig::typescript_defaults(),
                    3,
                )
                .unwrap()
                .updates()
                .iter()
                .map(|update| (update.snake_id, update.next_age_seconds, update.next_points))
                .collect::<Vec<_>>()
        };
        let forward = vec![
            snake(11, true, 0.0, 0.0),
            snake(4, true, 3.0, 9.0),
            snake(8, false, 5.0, 7.0),
        ];
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_eq!(run(forward), run(reversed));
    }

    #[test]
    fn stale_source_or_working_copy_rejects_before_any_write() {
        let source = world(vec![snake(1, true, 0.0, 1.0), snake(2, true, 1.0, 2.0)]);
        let mut workspace = StepAccountingWorkspace::new();
        let prepared = workspace
            .prepare(
                key(),
                &source,
                0.0,
                0.5,
                StepAccountingConfig::typescript_defaults(),
                2,
            )
            .unwrap();
        let mut changed_config = StepAccountingConfig::typescript_defaults();
        changed_config.points_per_second_alive = 0.7;
        for (result, field) in [
            (
                prepared.validate_current(
                    key(),
                    &source,
                    1.0,
                    0.5,
                    StepAccountingConfig::typescript_defaults(),
                    2,
                ),
                "generation elapsed time",
            ),
            (
                prepared.validate_current(
                    key(),
                    &source,
                    0.0,
                    0.25,
                    StepAccountingConfig::typescript_defaults(),
                    2,
                ),
                "fixed delta",
            ),
            (
                prepared.validate_current(key(), &source, 0.0, 0.5, changed_config, 2),
                "config",
            ),
            (
                prepared.validate_current(
                    key(),
                    &source,
                    0.0,
                    0.5,
                    StepAccountingConfig::typescript_defaults(),
                    3,
                ),
                "snake capacity",
            ),
        ] {
            assert_eq!(result, Err(StepAccountingError::SourceChanged { field }));
        }
        let other_source = source.clone();
        let mut target = source.clone();
        let target_before = target.clone();
        assert_eq!(
            prepared.apply_to_working_copy(
                key(),
                &other_source,
                0.0,
                0.5,
                StepAccountingConfig::typescript_defaults(),
                2,
                &mut target,
            ),
            Err(StepAccountingError::SourceChanged { field: "world" })
        );
        assert_eq!(target, target_before);

        target.snakes[1].points = 99.0;
        let stale_target = target.clone();
        assert_eq!(
            prepared.apply_to_working_copy(
                key(),
                &source,
                0.0,
                0.5,
                StepAccountingConfig::typescript_defaults(),
                2,
                &mut target,
            ),
            Err(StepAccountingError::WorkingCopyChanged { field: "snakes" })
        );
        assert_eq!(target, stale_target);

        target = source.clone();
        assert_eq!(
            prepared.apply_to_working_copy(
                PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 8),
                &source,
                0.0,
                0.5,
                StepAccountingConfig::typescript_defaults(),
                2,
                &mut target,
            ),
            Err(StepAccountingError::StepKeyMismatch {
                field: PhysicsStepKeyField::OperationEpoch,
            })
        );
        assert_eq!(target, source);

        prepared
            .apply_to_working_copy(
                key(),
                &source,
                0.0,
                0.5,
                StepAccountingConfig::typescript_defaults(),
                2,
                &mut target,
            )
            .expect("the first application should update the matching copy");
        let after_first_application = target.clone();
        assert_eq!(
            prepared.apply_to_working_copy(
                key(),
                &source,
                0.0,
                0.5,
                StepAccountingConfig::typescript_defaults(),
                2,
                &mut target,
            ),
            Err(StepAccountingError::WorkingCopyChanged { field: "snakes" })
        );
        assert_eq!(target, after_first_application);
    }

    #[test]
    fn malformed_or_overflowing_boundaries_never_become_ready() {
        let mut source = world(vec![snake(1, true, 0.0, 0.0)]);
        let mut workspace = StepAccountingWorkspace::new();
        source.snakes[0].body.len = 0;
        assert!(matches!(
            workspace.prepare(
                key(),
                &source,
                0.0,
                1.0 / 60.0,
                StepAccountingConfig::typescript_defaults(),
                1,
            ),
            Err(StepAccountingError::AliveSnakeHasNoBody { snake_id: 1 })
        ));
        assert!(!workspace.is_ready());

        source.snakes[0].body.len = 1;
        source.snakes[0].age_seconds = f64::MAX;
        assert!(matches!(
            workspace.prepare(
                key(),
                &source,
                0.0,
                f64::MAX,
                StepAccountingConfig::typescript_defaults(),
                1,
            ),
            Err(StepAccountingError::NonFiniteGenerated {
                snake_id: 1,
                field: "age_seconds"
            })
        ));
        assert!(!workspace.is_ready());

        source.snakes[0].age_seconds = 0.0;
        assert!(matches!(
            workspace.prepare(
                key(),
                &source,
                f64::MAX,
                f64::MAX,
                StepAccountingConfig::typescript_defaults(),
                1,
            ),
            Err(StepAccountingError::NonFiniteGenerationTime)
        ));
        assert!(!workspace.is_ready());
    }

    #[test]
    fn newborn_helper_uses_the_same_survival_formula() {
        let newborn = snake(20, true, 0.0, 0.0);
        assert_eq!(
            StepAccountingConfig::typescript_defaults()
                .advance_live_snake(&newborn, 1.0 / 60.0)
                .unwrap(),
            (1.0 / 60.0, 0.01)
        );
    }

    #[test]
    fn warmed_preparation_reuses_order_and_update_capacity() {
        let source = world(
            (1..=64)
                .map(|id| snake(id, id % 3 != 0, id as f64, id as f64 * 2.0))
                .collect(),
        );
        let mut workspace = StepAccountingWorkspace::new();
        let first = workspace
            .prepare(
                key(),
                &source,
                0.0,
                1.0 / 60.0,
                StepAccountingConfig::typescript_defaults(),
                64,
            )
            .unwrap()
            .diagnostics();
        for _ in 0..24 {
            let next = workspace
                .prepare(
                    key(),
                    &source,
                    0.0,
                    1.0 / 60.0,
                    StepAccountingConfig::typescript_defaults(),
                    64,
                )
                .unwrap()
                .diagnostics();
            assert_eq!(next, first);
            assert!(next.order_capacity >= 64);
            assert!(next.update_capacity >= 43);
        }
    }
}
