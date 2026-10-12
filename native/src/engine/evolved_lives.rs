//! Population-owned physical-life accounting, independent of physical wallets.
//!
//! Deltas come from the prefix and accepted physics substeps. Callers stage a
//! retained copy and publish it with the same reversible swap as the world.

use super::state::{SnakeKind, SnakeState, WorldState};
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};

/// Current population-life accounting formulas and chronology.
pub const EVOLVED_LIVES_VERSION: u32 = 1;

/// Physical lifecycle of one persistent population owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvolvedLifeStatus {
    Alive,
    Waiting,
    PendingPlacement,
    Exhausted,
    DeadNoRespawn,
}

/// Selected point assessment, independent of spendable capital.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointAssessmentMode {
    Reset,
    CumulativeNet,
    CumulativeGross,
    BestLife,
}

/// Independent source of non-point fitness terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonPointAggregation {
    GenerationTotals,
    FollowScore,
    LegacyTerminal,
}

/// Cause of a life ending; simultaneous contacts retain all bits separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvolvedDeathCause {
    Wall,
    HeadToHead,
    Body,
    GodMode,
}

/// Recorded respawn delay distribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RespawnDelayMode {
    Fixed,
    Uniform,
}
/// Selected lifecycle RNG owner; evolution is never a lifecycle source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RespawnRngMode {
    PerSlot,
    World,
}
/// Source recurrence for the forced first evaluation of a new physical life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecurrentLifeMode {
    Reset,
    Preserve,
}
/// Ecology rule for a replaceable evolved death.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvolvedCorpseMode {
    Normal,
    None,
    SubtractReplacementMass,
}
/// Independent policy for storage saturation by new effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PelletOverflowMode {
    Fail,
    DiscardNewExcess,
}
/// Early-end experiment, independent of manual and duration completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyCompletionMode {
    Automatic,
    PhysicalAlive,
    DurationOnly,
}

/// Versioned experiment policy. Version zero explicitly denotes recorded legacy
/// settings; modern policies require the complete version-one settings extension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvolvedExperimentConfig {
    pub version: u32,
    pub enabled: bool,
    pub delay_seconds: f64,
    pub delay_mode: RespawnDelayMode,
    pub max_lives: u64,
    pub recurrent_state: RecurrentLifeMode,
    pub rng_mode: RespawnRngMode,
    pub points: PointAssessmentMode,
    pub carried_wallet: bool,
    pub nonpoints: NonPointAggregation,
    pub objective: super::evolution::EvolvedFitnessObjective,
    pub corpse_mode: EvolvedCorpseMode,
    pub pellet_overflow: PelletOverflowMode,
    pub early_completion: EarlyCompletionMode,
}

impl EvolvedExperimentConfig {
    /// Preserve the frozen one-life objective for an explicitly recorded old schema.
    #[must_use]
    pub const fn legacy(duration: f64) -> Self {
        Self {
            version: 0,
            enabled: false,
            delay_seconds: 0.0,
            delay_mode: RespawnDelayMode::Fixed,
            max_lives: 0,
            recurrent_state: RecurrentLifeMode::Reset,
            rng_mode: RespawnRngMode::PerSlot,
            points: PointAssessmentMode::Reset,
            carried_wallet: false,
            nonpoints: NonPointAggregation::GenerationTotals,
            objective: super::evolution::EvolvedFitnessObjective {
                normalize_to_240_seconds: false,
                configured_duration_seconds: duration,
                penalty_amount: 0.0,
                penalize_successful_respawns: false,
            },
            corpse_mode: EvolvedCorpseMode::Normal,
            pellet_overflow: PelletOverflowMode::Fail,
            early_completion: EarlyCompletionMode::Automatic,
        }
    }

    /// Check the complete lifecycle/objective policy before any draw or mutation.
    pub fn validate(self) -> Result<(), EvolvedLifeError> {
        if self.version > 1
            || (self.version == 0
                && self != Self::legacy(self.objective.configured_duration_seconds))
            || !self.delay_seconds.is_finite()
            || !(0.0..=3600.0).contains(&self.delay_seconds)
            || self.max_lives > 10000
            || self.objective.validate().is_err()
        {
            return Err(EvolvedLifeError::InvalidPolicy);
        }
        Ok(())
    }

    /// Whether evolution uses selected life tuples instead of the exact legacy
    /// physical-snake calculation. Reports use the same dispatch.
    #[must_use]
    pub const fn uses_life_assessments(self) -> bool {
        self.version != 0
            && (self.enabled
                || !matches!(self.points, PointAssessmentMode::Reset)
                || self.objective.normalize_to_240_seconds
                || self.objective.penalty_amount != 0.0)
    }

    /// Whether a death has another life available, including unlimited experiments.
    #[must_use]
    pub const fn replaceable(self, lives_started: u64) -> bool {
        self.enabled && (self.max_lives == 0 || lives_started < self.max_lives)
    }
}

/// Compact latest-life or best-point-life fitness tuple.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LifeAssessment {
    pub ordinal: u64,
    pub peak_points: f64,
    pub alive_seconds: f64,
    pub food: f64,
    pub kills: u64,
    pub starting_length: usize,
    pub final_length: usize,
    pub peak_length: usize,
}

/// Input to evolution without treating a terminal physical snake as all lives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvolvedAssessment {
    pub points: f64,
    pub alive_seconds: f64,
    pub food: f64,
    pub kills: u64,
    pub retained_growth: f64,
    pub peak_length: usize,
}

/// Constant-sized ledger maintained even when respawn and score overrides are off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvolvedEconomicLedger {
    pub life_gross_earned: f64,
    pub life_boost_spent: f64,
    pub life_boost_spent_from_life_credits: f64,
    pub life_boost_spent_from_prior_bank: f64,
    pub life_local_wallet: f64,
    pub prior_bank_remaining: f64,
    pub generation_gross_earned: f64,
    pub generation_boost_spent: f64,
    pub generation_net_earned: f64,
    /// Number of independently rounded ledger operations for validation bounds.
    pub operations: u64,
}

impl EvolvedEconomicLedger {
    /// Life-local net points, maintained directly so inherited expenditure cannot erase them.
    #[must_use]
    pub const fn life_net_points(&self) -> f64 {
        self.life_local_wallet
    }

    /// Award an authoritative prefix, food or collision credit.
    pub fn credit(&mut self, amount: f64) -> Result<(), EvolvedLifeError> {
        nonnegative(amount)?;
        let mut next = *self;
        next.life_gross_earned += amount;
        next.life_local_wallet += amount;
        next.generation_gross_earned += amount;
        next.generation_net_earned += amount;
        next.count_operation()?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Attribute actual movement burn before later credits, life-local funds first.
    pub fn spend(&mut self, amount: f64) -> Result<(), EvolvedLifeError> {
        nonnegative(amount)?;
        let mut next = *self;
        let life_spend = amount.min(next.life_local_wallet);
        let bank_spend = amount - life_spend;
        // Accept only roundoff at the already independently rounded physical
        // wallet boundary. Genuine overdraw always rejects without changing state.
        let tolerance = 32.0
            * f64::EPSILON
            * (next.life_local_wallet + next.prior_bank_remaining)
                .max(amount)
                .max(1.0);
        if bank_spend > next.prior_bank_remaining + tolerance {
            return Err(EvolvedLifeError::Overdraw);
        }
        next.life_local_wallet -= life_spend;
        if bank_spend > next.prior_bank_remaining {
            next.prior_bank_remaining = 0.0;
        } else {
            next.prior_bank_remaining -= bank_spend;
        }
        next.life_boost_spent += amount;
        next.life_boost_spent_from_life_credits += life_spend;
        next.life_boost_spent_from_prior_bank += bank_spend;
        next.generation_boost_spent += amount;
        next.generation_net_earned -= amount;
        if next.generation_net_earned < 0.0 {
            if next.generation_net_earned < -tolerance {
                return Err(EvolvedLifeError::Overdraw);
            }
            next.generation_net_earned = 0.0;
        }
        next.count_operation()?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Open a successfully placed life; gross score never becomes capital.
    pub fn begin_life(&mut self, carried: bool) -> Result<f64, EvolvedLifeError> {
        self.validate()?;
        self.life_gross_earned = 0.0;
        self.life_boost_spent = 0.0;
        self.life_boost_spent_from_life_credits = 0.0;
        self.life_boost_spent_from_prior_bank = 0.0;
        self.life_local_wallet = 0.0;
        self.prior_bank_remaining = if carried {
            self.generation_net_earned
        } else {
            0.0
        };
        Ok(self.prior_bank_remaining)
    }

    fn count_operation(&mut self) -> Result<(), EvolvedLifeError> {
        self.operations = self
            .operations
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        Ok(())
    }

    /// Conservative Float64 forward-error allowance, scaling with independent
    /// sums and operation count; this is a validation allowance, not spendable debt.
    #[must_use]
    pub fn rounding_bound(&self, extra: f64) -> f64 {
        let scale = self
            .generation_gross_earned
            .max(self.generation_boost_spent)
            .max(self.life_gross_earned)
            .max(extra)
            .max(1.0);
        32.0 * f64::EPSILON * (self.operations as f64 + 1.0) * scale
    }

    /// Check independently accumulated identities and finite nonnegative balances.
    pub fn validate(&self) -> Result<(), EvolvedLifeError> {
        for value in [
            self.life_gross_earned,
            self.life_boost_spent,
            self.life_boost_spent_from_life_credits,
            self.life_boost_spent_from_prior_bank,
            self.life_local_wallet,
            self.prior_bank_remaining,
            self.generation_gross_earned,
            self.generation_boost_spent,
            self.generation_net_earned,
        ] {
            nonnegative(value)?;
        }
        let tolerance = self.rounding_bound(0.0);
        for (left, right) in [
            (
                self.life_boost_spent,
                self.life_boost_spent_from_life_credits + self.life_boost_spent_from_prior_bank,
            ),
            (
                self.life_local_wallet,
                self.life_gross_earned - self.life_boost_spent_from_life_credits,
            ),
            (
                self.generation_net_earned,
                self.generation_gross_earned - self.generation_boost_spent,
            ),
        ] {
            if (left - right).abs() > tolerance {
                return Err(EvolvedLifeError::LedgerIdentity);
            }
        }
        Ok(())
    }
}

/// Accepted substep delta; burn precedes all credits, then physical finalization.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvolvedPhysicsDelta {
    pub population_slot: Option<u32>,
    pub snake_id: u64,
    pub boost_points_spent: f64,
    pub credits: f64,
    pub food: f64,
    pub kills: u64,
    pub wallet_after: f64,
    pub length_after: usize,
    pub alive_after: bool,
    pub death: Option<EvolvedDeathCause>,
    /// Simultaneous Wall/Head/Body contact bits; God Mode uses bit 3.
    pub death_flags: u8,
    pub body_owner_id: Option<u64>,
    pub corpse_emitted_count: u64,
    pub corpse_emitted_value: f64,
    pub corpse_discarded_count: u64,
    pub corpse_discarded_value: f64,
}

/// Bounded, ephemeral life-end diagnostics. Historical physical/body owner IDs
/// are values and must never be dereferenced into the current world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvolvedLifeEndRecord {
    pub event_ordinal: u64,
    pub population_slot: u32,
    pub physical_id: u64,
    pub life_ordinal: u64,
    pub cause: EvolvedDeathCause,
    pub contact_flags: u8,
    pub body_owner_id: Option<u64>,
    pub life_net_points: f64,
    pub life_peak_points: f64,
    pub generation_net_points: f64,
    pub generation_gross_points: f64,
    pub best_life_points: f64,
    pub wallet: f64,
    pub life_gross_earned: f64,
    pub life_boost_spent: f64,
    pub prior_bank_remaining: f64,
    pub food: f64,
    pub kills: u64,
    pub final_length: usize,
    pub peak_length: usize,
    pub alive_seconds: f64,
    pub corpse_emitted_count: u64,
    pub corpse_emitted_value: f64,
    pub corpse_discarded_count: u64,
    pub corpse_discarded_value: f64,
}

/// Maximum recent life ends retained independently of generation duration.
pub const MAX_RECENT_LIFE_ENDS: usize = 256;

/// Dense population owner with a bounded current/best tuple, never a life history.
#[derive(Clone, Debug, PartialEq)]
pub struct EvolvedSlotRuntime {
    pub population_slot: u32,
    pub physical_id: u64,
    pub status: EvolvedLifeStatus,
    pub life_ordinal: u64,
    pub lives_started: u64,
    pub deaths: u64,
    pub successful_respawns: u64,
    pub current: LifeAssessment,
    pub best_point_life: LifeAssessment,
    pub ledger: EvolvedEconomicLedger,
    pub total_alive_seconds: f64,
    pub total_food: f64,
    pub total_kills: u64,
    pub completed_life_growth: f64,
    pub generation_peak_length: usize,
    pub pending_seconds: f64,
    pub respawn_deadline: Option<f64>,
    pub respawn_deadline_step: Option<u64>,
    pub last_attempt_step: Option<u64>,
    pub pending_recurrent_reset: bool,
    pub last_attempt: Option<f64>,
    pub fallback_cursor: u64,
    pub failed_placements: u64,
    pub last_cause: Option<EvolvedDeathCause>,
    pub last_death_flags: u8,
    pub last_body_owner_id: Option<u64>,
    pub cause_counts: [u64; 4],
    pub corpse_emitted_count: u64,
    pub corpse_emitted_value: f64,
    pub corpse_discarded_count: u64,
    pub corpse_discarded_value: f64,
}

impl EvolvedSlotRuntime {
    /// Construct the initial generation life from its actual spawned physical state.
    pub fn initial(snake: &SnakeState, starting_length: usize) -> Result<Self, EvolvedLifeError> {
        let population_slot = snake.population_slot.ok_or(EvolvedLifeError::Identity)?;
        if snake.kind != SnakeKind::Evolved || snake.id == 0 {
            return Err(EvolvedLifeError::Identity);
        }
        let current = LifeAssessment {
            ordinal: 1,
            peak_points: snake.points,
            alive_seconds: snake.age_seconds,
            food: snake.food,
            kills: snake.kills,
            starting_length,
            final_length: snake.body.len,
            peak_length: snake.body.len,
        };
        let mut ledger = EvolvedEconomicLedger::default();
        // Nonzero initial points occur only in frozen synthetic continuation fixtures.
        // Production generation construction supplies a zero wallet.
        ledger.credit(snake.points)?;
        Ok(Self {
            population_slot,
            physical_id: snake.id,
            status: if snake.alive {
                EvolvedLifeStatus::Alive
            } else {
                EvolvedLifeStatus::DeadNoRespawn
            },
            life_ordinal: 1,
            lives_started: 1,
            deaths: u64::from(!snake.alive),
            successful_respawns: 0,
            current,
            best_point_life: current,
            ledger,
            total_alive_seconds: snake.age_seconds,
            total_food: snake.food,
            total_kills: snake.kills,
            completed_life_growth: if snake.alive {
                0.0
            } else {
                snake.body.len.saturating_sub(starting_length) as f64
            },
            generation_peak_length: snake.body.len,
            pending_seconds: 0.0,
            respawn_deadline: None,
            respawn_deadline_step: None,
            last_attempt_step: None,
            pending_recurrent_reset: false,
            last_attempt: None,
            fallback_cursor: 0,
            failed_placements: 0,
            last_cause: None,
            last_death_flags: 0,
            last_body_owner_id: None,
            cause_counts: [0; 4],
            corpse_emitted_count: 0,
            corpse_emitted_value: 0.0,
            corpse_discarded_count: 0,
            corpse_discarded_value: 0.0,
        })
    }

    /// Apply exactly one source-live prefix award before shared control.
    pub fn alive_award(&mut self, dt: f64, credit: f64) -> Result<(), EvolvedLifeError> {
        if self.status != EvolvedLifeStatus::Alive {
            return Err(EvolvedLifeError::Lifecycle);
        }
        nonnegative(dt)?;
        self.ledger.credit(credit)?;
        self.current.alive_seconds += dt;
        self.total_alive_seconds += dt;
        nonnegative(self.total_alive_seconds)?;
        self.update_best();
        Ok(())
    }

    /// Apply an accepted physics substep with independently computed actual deltas.
    pub fn apply_physics(&mut self, delta: EvolvedPhysicsDelta) -> Result<(), EvolvedLifeError> {
        if delta.snake_id != self.physical_id
            || delta.population_slot != Some(self.population_slot)
            || self.status != EvolvedLifeStatus::Alive
        {
            return Err(EvolvedLifeError::Identity);
        }
        self.ledger.spend(delta.boost_points_spent)?;
        self.ledger.credit(delta.credits)?;
        nonnegative(delta.food)?;
        self.current.food += delta.food;
        self.total_food += delta.food;
        self.current.kills = self
            .current
            .kills
            .checked_add(delta.kills)
            .ok_or(EvolvedLifeError::Overflow)?;
        self.total_kills = self
            .total_kills
            .checked_add(delta.kills)
            .ok_or(EvolvedLifeError::Overflow)?;
        self.current.final_length = delta.length_after;
        self.current.peak_length = self.current.peak_length.max(delta.length_after);
        self.generation_peak_length = self.generation_peak_length.max(delta.length_after);
        self.check_wallet(delta.wallet_after)?;
        nonnegative(delta.corpse_emitted_value)?;
        self.corpse_emitted_count = self
            .corpse_emitted_count
            .checked_add(delta.corpse_emitted_count)
            .ok_or(EvolvedLifeError::Overflow)?;
        self.corpse_emitted_value += delta.corpse_emitted_value;
        nonnegative(self.corpse_emitted_value)?;
        nonnegative(delta.corpse_discarded_value)?;
        self.corpse_discarded_count = self
            .corpse_discarded_count
            .checked_add(delta.corpse_discarded_count)
            .ok_or(EvolvedLifeError::Overflow)?;
        self.corpse_discarded_value += delta.corpse_discarded_value;
        nonnegative(self.corpse_discarded_value)?;
        self.update_best();
        if !delta.alive_after {
            self.finalize(delta.death.unwrap_or(EvolvedDeathCause::GodMode))?;
            self.last_death_flags = delta.death_flags;
            self.last_body_owner_id = delta.body_owner_id;
        }
        Ok(())
    }

    /// Finalize once after all simultaneous food/kill awards for this life.
    pub fn finalize(&mut self, cause: EvolvedDeathCause) -> Result<(), EvolvedLifeError> {
        if self.status != EvolvedLifeStatus::Alive {
            return Err(EvolvedLifeError::Lifecycle);
        }
        let deaths = self
            .deaths
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        let cause_index = match cause {
            EvolvedDeathCause::Wall => 0,
            EvolvedDeathCause::HeadToHead => 1,
            EvolvedDeathCause::Body => 2,
            EvolvedDeathCause::GodMode => 3,
        };
        self.cause_counts[cause_index] = self.cause_counts[cause_index]
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        self.completed_life_growth +=
            self.current
                .final_length
                .saturating_sub(self.current.starting_length) as f64;
        self.deaths = deaths;
        self.last_cause = Some(cause);
        self.last_death_flags = 1 << cause_index;
        self.last_body_owner_id = None;
        self.status = EvolvedLifeStatus::DeadNoRespawn;
        self.update_best();
        Ok(())
    }

    /// Open a physically committed successor life without changing brain/genome ownership.
    pub fn begin_life(
        &mut self,
        physical_id: u64,
        length: usize,
        carried: bool,
    ) -> Result<f64, EvolvedLifeError> {
        if self.status == EvolvedLifeStatus::Alive
            || physical_id == 0
            || physical_id == self.physical_id
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        let ordinal = self
            .life_ordinal
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        let respawns = self
            .successful_respawns
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        let wallet = self.ledger.begin_life(carried)?;
        self.physical_id = physical_id;
        self.life_ordinal = ordinal;
        self.lives_started = ordinal;
        self.successful_respawns = respawns;
        self.current = LifeAssessment {
            ordinal,
            starting_length: length,
            final_length: length,
            peak_length: length,
            ..LifeAssessment::default()
        };
        self.status = EvolvedLifeStatus::Alive;
        self.respawn_deadline = None;
        self.respawn_deadline_step = None;
        self.last_attempt_step = None;
        self.last_attempt = None;
        self.fallback_cursor = 0;
        Ok(wallet)
    }

    fn update_best(&mut self) {
        self.current.peak_points = self.current.peak_points.max(self.ledger.life_net_points());
        if self.best_point_life.ordinal == self.current.ordinal
            || self.current.peak_points > self.best_point_life.peak_points
        {
            self.best_point_life = self.current;
        }
    }

    /// Validate the ledger against the independently maintained physical wallet.
    pub fn check_wallet(&self, wallet: f64) -> Result<(), EvolvedLifeError> {
        nonnegative(wallet)?;
        let expected = self.ledger.life_local_wallet + self.ledger.prior_bank_remaining;
        if (wallet - expected).abs() > self.ledger.rounding_bound(wallet) {
            return Err(EvolvedLifeError::WalletMismatch);
        }
        Ok(())
    }

    /// Resolve point assessment and its independently selected non-point tuple.
    #[must_use]
    pub fn assessment(
        &self,
        points: PointAssessmentMode,
        nonpoints: NonPointAggregation,
    ) -> EvolvedAssessment {
        let assessment_points = match points {
            PointAssessmentMode::Reset => self.ledger.life_net_points(),
            PointAssessmentMode::CumulativeNet => self.ledger.generation_net_earned,
            PointAssessmentMode::CumulativeGross => self.ledger.generation_gross_earned,
            PointAssessmentMode::BestLife => self.best_point_life.peak_points,
        };
        let totals = nonpoints == NonPointAggregation::GenerationTotals
            || (nonpoints == NonPointAggregation::FollowScore
                && matches!(
                    points,
                    PointAssessmentMode::CumulativeNet | PointAssessmentMode::CumulativeGross
                ));
        if totals {
            EvolvedAssessment {
                points: assessment_points,
                alive_seconds: self.total_alive_seconds,
                food: self.total_food,
                kills: self.total_kills,
                retained_growth: self.completed_life_growth
                    + if self.status == EvolvedLifeStatus::Alive {
                        self.current
                            .final_length
                            .saturating_sub(self.current.starting_length)
                            as f64
                    } else {
                        0.0
                    },
                peak_length: self.generation_peak_length,
            }
        } else {
            let life = if nonpoints == NonPointAggregation::FollowScore
                && points == PointAssessmentMode::BestLife
            {
                self.best_point_life
            } else {
                self.current
            };
            EvolvedAssessment {
                points: assessment_points,
                alive_seconds: life.alive_seconds,
                food: life.food,
                kills: life.kills,
                retained_growth: life.final_length.saturating_sub(life.starting_length) as f64,
                peak_length: life.peak_length,
            }
        }
    }
}

/// Dense generation continuation, empty at durable pre-spawn checkpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct EvolvedGenerationState {
    /// Explicit formula version; boundary archives contain no running lives.
    pub version: u32,
    pub slots: Vec<EvolvedSlotRuntime>,
    /// Total recorded events, including those evicted from the recent ring.
    pub life_end_count: u64,
    /// At most one end per owner in the current complete step or cold command.
    pub transaction_life_ends: Vec<EvolvedLifeEndRecord>,
    /// Oldest-first diagnostic ring; never encoded into a boundary checkpoint.
    pub recent_life_ends: VecDeque<EvolvedLifeEndRecord>,
}

impl Default for EvolvedGenerationState {
    fn default() -> Self {
        Self {
            version: EVOLVED_LIVES_VERSION,
            slots: Vec::new(),
            life_end_count: 0,
            transaction_life_ends: Vec::new(),
            recent_life_ends: VecDeque::new(),
        }
    }
}

/// Convert a simulated delay into an eligible whole-step deadline. Even zero
/// waits until the next newborn boundary; overflow is an atomic staging error.
fn delay_steps(delay: f64, fixed_dt: f64) -> Result<u64, EvolvedLifeError> {
    if !fixed_dt.is_finite() || fixed_dt <= 0.0 || !delay.is_finite() || delay < 0.0 {
        return Err(EvolvedLifeError::InvalidDeadline);
    }
    let steps = (delay / fixed_dt).ceil().max(1.0);
    if !steps.is_finite() || steps >= u64::MAX as f64 {
        return Err(EvolvedLifeError::Overflow);
    }
    Ok(steps as u64)
}

impl EvolvedGenerationState {
    /// Reuse retained storage for a new complete-step transaction, preserving
    /// published diagnostics but clearing only the previous transaction's events.
    pub fn stage_from(&mut self, source: &Self) -> Result<(), EvolvedLifeError> {
        self.version = source.version;
        self.slots.clone_from(&source.slots);
        self.life_end_count = source.life_end_count;
        self.recent_life_ends.clone_from(&source.recent_life_ends);
        self.transaction_life_ends.clear();
        self.reserve_events()
    }

    /// Reserve both bounded buffers before processing any physical events.
    fn reserve_events(&mut self) -> Result<(), EvolvedLifeError> {
        if self.transaction_life_ends.capacity() < self.slots.len() {
            self.transaction_life_ends
                .try_reserve_exact(self.slots.len() - self.transaction_life_ends.len())
                .map_err(|_| EvolvedLifeError::Allocation)?;
        }
        if self.recent_life_ends.capacity() < MAX_RECENT_LIFE_ENDS {
            self.recent_life_ends
                .try_reserve_exact(MAX_RECENT_LIFE_ENDS - self.recent_life_ends.len())
                .map_err(|_| EvolvedLifeError::Allocation)?;
        }
        Ok(())
    }

    /// Capture after all accepted awards and finalization, while simultaneous
    /// contacts and actual corpse emission still belong to this physical life.
    fn record_life_end(
        &mut self,
        slot: usize,
        corpse: (u64, f64, u64, f64),
    ) -> Result<(), EvolvedLifeError> {
        if self.transaction_life_ends.len() >= self.slots.len()
            || self
                .transaction_life_ends
                .iter()
                .any(|event| event.population_slot as usize == slot)
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        self.reserve_events()?;
        let owner = self.slots.get(slot).ok_or(EvolvedLifeError::Identity)?;
        let event_ordinal = self
            .life_end_count
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        let cause = owner.last_cause.ok_or(EvolvedLifeError::Lifecycle)?;
        let event = EvolvedLifeEndRecord {
            event_ordinal,
            population_slot: owner.population_slot,
            physical_id: owner.physical_id,
            life_ordinal: owner.life_ordinal,
            cause,
            contact_flags: owner.last_death_flags,
            body_owner_id: owner.last_body_owner_id,
            life_net_points: owner.ledger.life_net_points(),
            life_peak_points: owner.current.peak_points,
            generation_net_points: owner.ledger.generation_net_earned,
            generation_gross_points: owner.ledger.generation_gross_earned,
            best_life_points: owner.best_point_life.peak_points,
            wallet: owner.ledger.life_local_wallet + owner.ledger.prior_bank_remaining,
            life_gross_earned: owner.ledger.life_gross_earned,
            life_boost_spent: owner.ledger.life_boost_spent,
            prior_bank_remaining: owner.ledger.prior_bank_remaining,
            food: owner.current.food,
            kills: owner.current.kills,
            final_length: owner.current.final_length,
            peak_length: owner.current.peak_length,
            alive_seconds: owner.current.alive_seconds,
            corpse_emitted_count: corpse.0,
            corpse_emitted_value: corpse.1,
            corpse_discarded_count: corpse.2,
            corpse_discarded_value: corpse.3,
        };
        if self.recent_life_ends.len() == MAX_RECENT_LIFE_ENDS {
            self.recent_life_ends.pop_front();
        }
        self.recent_life_ends.push_back(event);
        self.transaction_life_ends.push(event);
        self.life_end_count = event_ordinal;
        Ok(())
    }

    /// Finalize a cold God Mode command through the same bounded event capture.
    pub fn finalize_god_mode(
        &mut self,
        slot: usize,
        corpse: (u64, f64, u64, f64),
    ) -> Result<(), EvolvedLifeError> {
        self.transaction_life_ends.clear();
        let runtime = self.slots.get_mut(slot).ok_or(EvolvedLifeError::Identity)?;
        runtime.finalize(EvolvedDeathCause::GodMode)?;
        nonnegative(corpse.1)?;
        nonnegative(corpse.3)?;
        runtime.corpse_emitted_count = runtime
            .corpse_emitted_count
            .checked_add(corpse.0)
            .ok_or(EvolvedLifeError::Overflow)?;
        runtime.corpse_discarded_count = runtime
            .corpse_discarded_count
            .checked_add(corpse.2)
            .ok_or(EvolvedLifeError::Overflow)?;
        runtime.corpse_emitted_value += corpse.1;
        runtime.corpse_discarded_value += corpse.3;
        nonnegative(runtime.corpse_emitted_value)?;
        nonnegative(runtime.corpse_discarded_value)?;
        self.record_life_end(slot, corpse)
    }

    /// Sample a newly finalized death exactly once and retain its deadline.
    /// Failed placements leave the ordinal, deadline and selected continuation intact.
    pub fn schedule_deaths(
        &mut self,
        policy: EvolvedExperimentConfig,
        rng: &mut super::state::RngStateBundle,
        completed_step: u64,
        elapsed_seconds: f64,
        fixed_dt: f64,
    ) -> Result<(), EvolvedLifeError> {
        policy.validate()?;
        nonnegative(elapsed_seconds)?;
        delay_steps(0.0, fixed_dt)?;
        for slot in &mut self.slots {
            if slot.status != EvolvedLifeStatus::DeadNoRespawn || !policy.enabled {
                continue;
            }
            if !policy.replaceable(slot.lives_started) {
                slot.status = EvolvedLifeStatus::Exhausted;
                continue;
            }
            let delay = if policy.delay_mode == RespawnDelayMode::Fixed {
                policy.delay_seconds
            } else {
                let uniform = match policy.rng_mode {
                    RespawnRngMode::PerSlot => {
                        let stream = rng
                            .evolved_respawns
                            .get_mut(slot.population_slot as usize)
                            .filter(|stream| stream.slot == slot.population_slot)
                            .ok_or(EvolvedLifeError::InvalidRng)?;
                        let mut runtime =
                            stream.runtime().map_err(|_| EvolvedLifeError::InvalidRng)?;
                        let draw = runtime.next_f64();
                        stream.uniform_state = runtime.uniform_state();
                        draw
                    }
                    RespawnRngMode::World => {
                        let mut runtime = super::rng::StatefulRng::from_state(&rng.world)
                            .map_err(|_| EvolvedLifeError::InvalidRng)?;
                        let draw = runtime.next_f64();
                        runtime.export_state_into(&mut rng.world);
                        draw
                    }
                };
                policy.delay_seconds * (0.5 + uniform)
            };
            let deadline = completed_step
                .checked_add(delay_steps(delay, fixed_dt)?)
                .ok_or(EvolvedLifeError::Overflow)?;
            let deadline_seconds = elapsed_seconds + delay;
            nonnegative(deadline_seconds)?;
            slot.respawn_deadline = Some(deadline_seconds);
            slot.respawn_deadline_step = Some(deadline);
            slot.last_attempt = None;
            slot.last_attempt_step = None;
            slot.status = EvolvedLifeStatus::Waiting;
        }
        Ok(())
    }

    /// Collect up to eight due owners in deadline/last-attempt/slot order.
    pub fn collect_due(
        &self,
        target_step: u64,
        fixed_dt: f64,
        due: &mut Vec<u32>,
    ) -> Result<(), EvolvedLifeError> {
        let retry_steps = delay_steps(0.25, fixed_dt)?;
        due.clear();
        if due.capacity() < self.slots.len() {
            due.try_reserve_exact(self.slots.len())
                .map_err(|_| EvolvedLifeError::Allocation)?;
        }
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.population_slot as usize != index {
                return Err(EvolvedLifeError::Identity);
            }
            if !matches!(
                slot.status,
                EvolvedLifeStatus::Waiting | EvolvedLifeStatus::PendingPlacement
            ) {
                continue;
            }
            let deadline = slot
                .respawn_deadline_step
                .ok_or(EvolvedLifeError::InvalidDeadline)?;
            let retry_due = match slot.last_attempt_step {
                Some(attempt) => {
                    attempt
                        .checked_add(retry_steps)
                        .ok_or(EvolvedLifeError::Overflow)?
                        <= target_step
                }
                None => true,
            };
            if deadline <= target_step && retry_due {
                due.push(slot.population_slot);
            }
        }
        due.sort_unstable_by_key(|&slot| {
            let owner = &self.slots[slot as usize];
            (
                owner.respawn_deadline_step,
                owner.last_attempt_step,
                owner.population_slot,
            )
        });
        due.truncate(8);
        Ok(())
    }

    /// Record one unsuccessful admitted attempt without consuming a physical life.
    pub fn record_failed_attempt(
        &mut self,
        slot: u32,
        step: u64,
        elapsed: f64,
        cursor: u64,
    ) -> Result<(), EvolvedLifeError> {
        let owner = self
            .slots
            .get_mut(slot as usize)
            .ok_or(EvolvedLifeError::Identity)?;
        if !matches!(
            owner.status,
            EvolvedLifeStatus::Waiting | EvolvedLifeStatus::PendingPlacement
        ) || owner
            .last_attempt_step
            .is_some_and(|previous| previous >= step)
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        nonnegative(elapsed)?;
        owner.failed_placements = owner
            .failed_placements
            .checked_add(1)
            .ok_or(EvolvedLifeError::Overflow)?;
        owner.last_attempt_step = Some(step);
        owner.last_attempt = Some(elapsed);
        owner.fallback_cursor = cursor;
        owner.status = EvolvedLifeStatus::PendingPlacement;
        Ok(())
    }

    /// Account only owners still waiting after the newborn boundary. A
    /// participating newborn never also receives a full pending interval.
    pub fn accrue_pending_time(&mut self, fixed_dt: f64) -> Result<(), EvolvedLifeError> {
        delay_steps(0.0, fixed_dt)?;
        for slot in &mut self.slots {
            if matches!(
                slot.status,
                EvolvedLifeStatus::Waiting | EvolvedLifeStatus::PendingPlacement
            ) {
                slot.pending_seconds += fixed_dt;
                nonnegative(slot.pending_seconds)?;
            }
        }
        Ok(())
    }

    /// Build a complete dense slot mapping, rejecting duplicate or absent owners.
    pub fn initialize(
        world: &WorldState,
        population_count: usize,
        starting_length: usize,
    ) -> Result<Self, EvolvedLifeError> {
        let mut state = Self::default();
        state
            .slots
            .try_reserve_exact(population_count)
            .map_err(|_| EvolvedLifeError::Allocation)?;
        for slot in 0..population_count {
            let slot_u32 = u32::try_from(slot).map_err(|_| EvolvedLifeError::Overflow)?;
            let mut matches = world.snakes.iter().filter(|snake| {
                snake.kind == SnakeKind::Evolved && snake.population_slot == Some(slot_u32)
            });
            let snake = matches.next().ok_or(EvolvedLifeError::Identity)?;
            if matches.next().is_some() {
                return Err(EvolvedLifeError::Identity);
            }
            state
                .slots
                .push(EvolvedSlotRuntime::initial(snake, starting_length)?);
        }
        state.reserve_events()?;
        Ok(state)
    }

    /// Stage source-live awards and ordered out-of-band deaths before physics.
    pub fn apply_prefix(
        &mut self,
        world: &WorldState,
        dt: f64,
        credit: f64,
    ) -> Result<(), EvolvedLifeError> {
        for snake in &world.snakes {
            if snake.kind != SnakeKind::Evolved {
                continue;
            }
            let slot = snake.population_slot.ok_or(EvolvedLifeError::Identity)? as usize;
            let runtime = self.slots.get_mut(slot).ok_or(EvolvedLifeError::Identity)?;
            if runtime.physical_id != snake.id {
                return Err(EvolvedLifeError::Identity);
            }
            if snake.alive {
                runtime.alive_award(dt, credit)?;
            } else if runtime.status == EvolvedLifeStatus::Alive {
                runtime.finalize(EvolvedDeathCause::GodMode)?;
                self.record_life_end(slot, (0, 0.0, 0, 0.0))?;
            }
        }
        Ok(())
    }

    /// Apply only live evolved participants from each accepted physics substep.
    pub fn apply_physics(
        &mut self,
        deltas: &[EvolvedPhysicsDelta],
    ) -> Result<(), EvolvedLifeError> {
        for &delta in deltas {
            if let Some(slot) = delta.population_slot {
                self.slots
                    .get_mut(slot as usize)
                    .ok_or(EvolvedLifeError::Identity)?
                    .apply_physics(delta)?;
                if !delta.alive_after {
                    self.record_life_end(
                        slot as usize,
                        (
                            delta.corpse_emitted_count,
                            delta.corpse_emitted_value,
                            delta.corpse_discarded_count,
                            delta.corpse_discarded_value,
                        ),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Validate bounded chronology and event values independently of current
    /// world references; old IDs remain diagnostic values after replacement.
    pub fn validate_events(&self) -> Result<(), EvolvedLifeError> {
        let retained = usize::try_from(self.life_end_count.min(MAX_RECENT_LIFE_ENDS as u64))
            .map_err(|_| EvolvedLifeError::Overflow)?;
        if self.recent_life_ends.len() != retained
            || self.transaction_life_ends.len() > self.slots.len()
            || self.transaction_life_ends.len() as u64 > self.life_end_count
            || (self.slots.is_empty() && self.life_end_count != 0)
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        let first = self.life_end_count - retained as u64;
        for (index, event) in self
            .recent_life_ends
            .iter()
            .chain(&self.transaction_life_ends)
            .enumerate()
        {
            let expected_ordinal = if index < retained {
                first + index as u64 + 1
            } else {
                self.life_end_count - self.transaction_life_ends.len() as u64
                    + (index - retained) as u64
                    + 1
            };
            let owner = self
                .slots
                .get(event.population_slot as usize)
                .ok_or(EvolvedLifeError::Identity)?;
            let primary = match event.cause {
                EvolvedDeathCause::Wall => 1,
                EvolvedDeathCause::HeadToHead => 2,
                EvolvedDeathCause::Body => 4,
                EvolvedDeathCause::GodMode => 8,
            };
            let valid_flags = if primary == 8 {
                event.contact_flags == 8 && event.body_owner_id.is_none()
            } else {
                event.contact_flags != 0
                    && event.contact_flags < 8
                    && (1 << event.contact_flags.trailing_zeros()) == primary
                    && (event.body_owner_id.is_some() == (event.contact_flags & 4 != 0))
            };
            if event.event_ordinal != expected_ordinal
                || event.physical_id == 0
                || event.life_ordinal == 0
                || event.life_ordinal > owner.life_ordinal
                || !valid_flags
                || event.body_owner_id == Some(0)
                || (event.life_ordinal == owner.life_ordinal
                    && (event.physical_id != owner.physical_id
                        || owner.status == EvolvedLifeStatus::Alive))
                || event.final_length == 0
                || event.peak_length < event.final_length
                || event.life_peak_points < event.life_net_points
                || event.best_life_points < event.life_peak_points
            {
                return Err(EvolvedLifeError::Lifecycle);
            }
            for value in [
                event.life_net_points,
                event.life_peak_points,
                event.generation_net_points,
                event.generation_gross_points,
                event.best_life_points,
                event.wallet,
                event.life_gross_earned,
                event.life_boost_spent,
                event.prior_bank_remaining,
                event.food,
                event.alive_seconds,
                event.corpse_emitted_value,
                event.corpse_discarded_value,
            ] {
                nonnegative(value)?;
            }
        }
        let shared = retained.min(self.transaction_life_ends.len());
        if !self
            .transaction_life_ends
            .iter()
            .skip(self.transaction_life_ends.len() - shared)
            .eq(self.recent_life_ends.iter().skip(retained - shared))
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        for (index, event) in self.transaction_life_ends.iter().enumerate() {
            if self.transaction_life_ends[..index]
                .iter()
                .any(|other| other.population_slot == event.population_slot)
            {
                return Err(EvolvedLifeError::Lifecycle);
            }
        }
        Ok(())
    }

    /// A complete step may append its actual newly finalized lives and evict
    /// only the oldest recent diagnostics. Existing published events are immutable.
    pub fn validate_event_continuation(&self, source: &Self) -> Result<(), EvolvedLifeError> {
        self.validate_events()?;
        if self.slots.len() != source.slots.len()
            || source
                .life_end_count
                .checked_add(self.transaction_life_ends.len() as u64)
                != Some(self.life_end_count)
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        let joined = source.recent_life_ends.len() + self.transaction_life_ends.len();
        if !source
            .recent_life_ends
            .iter()
            .chain(&self.transaction_life_ends)
            .skip(joined.saturating_sub(MAX_RECENT_LIFE_ENDS))
            .eq(&self.recent_life_ends)
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        for (old, new) in source.slots.iter().zip(&self.slots) {
            let ends = self
                .transaction_life_ends
                .iter()
                .filter(|event| event.population_slot == new.population_slot)
                .count() as u64;
            if old.deaths.checked_add(ends) != Some(new.deaths) {
                return Err(EvolvedLifeError::Lifecycle);
            }
        }
        Ok(())
    }

    /// Check the dense current physical mapping without assuming array order.
    pub fn validate(
        &self,
        world: &WorldState,
        population_count: usize,
    ) -> Result<(), EvolvedLifeError> {
        self.validate_events()?;
        if self.version != EVOLVED_LIVES_VERSION || self.slots.len() != population_count {
            return Err(EvolvedLifeError::Identity);
        }
        let mut physical_count = 0;
        for snake in &world.snakes {
            if snake.kind != SnakeKind::Evolved {
                continue;
            }
            let index = snake.population_slot.ok_or(EvolvedLifeError::Identity)? as usize;
            let slot = self.slots.get(index).ok_or(EvolvedLifeError::Identity)?;
            if slot.population_slot as usize != index
                || slot.life_ordinal == 0
                || slot.lives_started != slot.life_ordinal
                || slot.successful_respawns != slot.life_ordinal - 1
                || snake.id != slot.physical_id
                || (snake.alive && slot.status != EvolvedLifeStatus::Alive)
            {
                return Err(EvolvedLifeError::Identity);
            }
            if snake.alive != (slot.status == EvolvedLifeStatus::Alive)
                || slot.deaths.checked_add(u64::from(snake.alive)) != Some(slot.lives_started)
                || slot.current.ordinal != slot.life_ordinal
                || slot.best_point_life.ordinal > slot.life_ordinal
                || slot.current.final_length != snake.body.len
                || slot.current.starting_length == 0
                || slot.best_point_life.starting_length == 0
                || slot.best_point_life.peak_points < slot.current.peak_points
            {
                return Err(EvolvedLifeError::Lifecycle);
            }
            for value in [
                slot.total_alive_seconds,
                slot.total_food,
                slot.completed_life_growth,
                slot.pending_seconds,
                slot.corpse_emitted_value,
                slot.corpse_discarded_value,
                slot.current.alive_seconds,
                slot.current.food,
                slot.current.peak_points,
                slot.best_point_life.alive_seconds,
                slot.best_point_life.food,
                slot.best_point_life.peak_points,
            ] {
                nonnegative(value)?;
            }
            if slot.current.kills != snake.kills
                || slot.total_kills < slot.current.kills
                || (slot.current.alive_seconds - snake.age_seconds).abs()
                    > slot.ledger.rounding_bound(snake.age_seconds)
                || (slot.current.food - snake.food).abs() > slot.ledger.rounding_bound(snake.food)
            {
                return Err(EvolvedLifeError::LedgerIdentity);
            }
            slot.ledger.validate()?;
            slot.check_wallet(snake.points)?;
            physical_count += 1;
        }
        if physical_count != population_count {
            return Err(EvolvedLifeError::Identity);
        }
        Ok(())
    }
}

fn nonnegative(value: f64) -> Result<(), EvolvedLifeError> {
    if !value.is_finite() || value < 0.0 {
        Err(EvolvedLifeError::InvalidBalance)
    } else {
        Ok(())
    }
}

/// Checked accounting failure; authoritative callers retain the source continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvolvedLifeError {
    InvalidPolicy,
    InvalidDeadline,
    InvalidRng,
    InvalidBalance,
    LedgerIdentity,
    WalletMismatch,
    Overdraw,
    Identity,
    Lifecycle,
    Overflow,
    Allocation,
}
impl Display for EvolvedLifeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "evolved life accounting: {self:?}")
    }
}
impl std::error::Error for EvolvedLifeError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> EvolvedSlotRuntime {
        let current = LifeAssessment {
            ordinal: 1,
            starting_length: 5,
            final_length: 5,
            peak_length: 5,
            ..LifeAssessment::default()
        };
        EvolvedSlotRuntime {
            population_slot: 0,
            physical_id: 1,
            status: EvolvedLifeStatus::Alive,
            life_ordinal: 1,
            lives_started: 1,
            deaths: 0,
            successful_respawns: 0,
            current,
            best_point_life: current,
            ledger: EvolvedEconomicLedger::default(),
            total_alive_seconds: 0.0,
            total_food: 0.0,
            total_kills: 0,
            completed_life_growth: 0.0,
            generation_peak_length: 5,
            pending_seconds: 0.0,
            respawn_deadline: None,
            respawn_deadline_step: None,
            last_attempt_step: None,
            pending_recurrent_reset: false,
            last_attempt: None,
            fallback_cursor: 0,
            failed_placements: 0,
            last_cause: None,
            last_death_flags: 0,
            last_body_owner_id: None,
            cause_counts: [0; 4],
            corpse_emitted_count: 0,
            corpse_emitted_value: 0.0,
            corpse_discarded_count: 0,
            corpse_discarded_value: 0.0,
        }
    }

    fn delta(
        runtime: &EvolvedSlotRuntime,
        spent: f64,
        credits: f64,
        length: usize,
        alive: bool,
    ) -> EvolvedPhysicsDelta {
        EvolvedPhysicsDelta {
            population_slot: Some(runtime.population_slot),
            snake_id: runtime.physical_id,
            boost_points_spent: spent,
            credits,
            length_after: length,
            alive_after: alive,
            wallet_after: runtime.ledger.life_local_wallet + runtime.ledger.prior_bank_remaining
                - spent
                + credits,
            food: credits / 20.0,
            kills: 0,
            death: Some(EvolvedDeathCause::Body),
            death_flags: 4,
            body_owner_id: Some(99),
            corpse_emitted_count: 0,
            corpse_emitted_value: 0.0,
            corpse_discarded_count: 0,
            corpse_discarded_value: 0.0,
        }
    }

    #[test]
    fn all_point_and_nonpoint_bundles_include_unfinished_lives_and_earliest_best_tie() {
        let mut slot = runtime();
        slot.alive_award(2.0, 80.0).unwrap();
        let first = delta(&slot, 50.0, 0.0, 8, false);
        slot.apply_physics(first).unwrap();
        assert_eq!(slot.ledger.generation_gross_earned, 80.0);
        assert_eq!(slot.ledger.generation_net_earned, 30.0);
        assert_eq!(
            slot.assessment(PointAssessmentMode::Reset, NonPointAggregation::FollowScore)
                .points,
            30.0
        );
        assert_eq!(
            slot.assessment(
                PointAssessmentMode::BestLife,
                NonPointAggregation::FollowScore
            )
            .points,
            80.0
        );
        assert_eq!(slot.begin_life(2, 5, false).unwrap(), 0.0);
        slot.alive_award(3.0, 80.0).unwrap();
        let second = delta(&slot, 10.0, 0.0, 9, true);
        slot.apply_physics(second).unwrap();
        assert_eq!(slot.best_point_life.ordinal, 1);
        assert_eq!(slot.best_point_life.alive_seconds, 2.0);
        for (mode, expected) in [
            (PointAssessmentMode::Reset, 70.0),
            (PointAssessmentMode::CumulativeNet, 100.0),
            (PointAssessmentMode::CumulativeGross, 160.0),
            (PointAssessmentMode::BestLife, 80.0),
        ] {
            for aggregation in [
                NonPointAggregation::GenerationTotals,
                NonPointAggregation::FollowScore,
                NonPointAggregation::LegacyTerminal,
            ] {
                let assessment = slot.assessment(mode, aggregation);
                assert_eq!(assessment.points, expected);
                let totals = aggregation == NonPointAggregation::GenerationTotals
                    || (aggregation == NonPointAggregation::FollowScore
                        && matches!(
                            mode,
                            PointAssessmentMode::CumulativeNet
                                | PointAssessmentMode::CumulativeGross
                        ));
                let best = aggregation == NonPointAggregation::FollowScore
                    && mode == PointAssessmentMode::BestLife;
                assert_eq!(
                    assessment.alive_seconds,
                    if totals {
                        5.0
                    } else if best {
                        2.0
                    } else {
                        3.0
                    }
                );
                assert_eq!(
                    assessment.retained_growth,
                    if totals {
                        7.0
                    } else if best {
                        3.0
                    } else {
                        4.0
                    }
                );
            }
        }
        slot.alive_award(1.0, 11.0).unwrap();
        assert_eq!(slot.best_point_life.ordinal, 2);
        assert_eq!(slot.best_point_life.peak_points, 81.0);
        assert_eq!(slot.best_point_life.alive_seconds, 4.0);
    }

    #[test]
    fn peak_growth_is_separate_from_retained_growth_and_death_finalizes_once() {
        let mut slot = runtime();
        let grow = delta(&slot, 0.0, 20.0, 12, true);
        slot.apply_physics(grow).unwrap();
        let shrink = delta(&slot, 10.0, 0.0, 7, false);
        slot.apply_physics(shrink).unwrap();
        assert_eq!(slot.completed_life_growth, 2.0);
        assert_eq!(slot.current.peak_length, 12);
        assert_eq!(
            slot.finalize(EvolvedDeathCause::Body),
            Err(EvolvedLifeError::Lifecycle)
        );
        assert_eq!(slot.deaths, 1);
        slot.begin_life(2, 5, false).unwrap();
        let end = delta(&slot, 0.0, 5.0, 4, false);
        slot.apply_physics(end).unwrap();
        assert_eq!(slot.completed_life_growth, 2.0);
        assert_eq!(slot.generation_peak_length, 12);
        assert_eq!(
            slot.assessment(
                PointAssessmentMode::CumulativeNet,
                NonPointAggregation::GenerationTotals
            )
            .retained_growth,
            2.0
        );
    }

    #[test]
    fn successive_substeps_attribute_spend_before_new_food_and_simultaneous_death_credit() {
        let mut slot = runtime();
        slot.alive_award(1.0, 50.0).unwrap();
        slot.finalize(EvolvedDeathCause::GodMode).unwrap();
        assert_eq!(slot.begin_life(2, 5, true).unwrap(), 50.0);
        let first = delta(&slot, 50.0, 10.0, 6, true);
        slot.apply_physics(first).unwrap();
        assert_eq!(slot.ledger.life_net_points(), 10.0);
        let mut second = delta(&slot, 5.0, 400.0, 6, false);
        second.kills = 1;
        slot.apply_physics(second).unwrap();
        assert_eq!(slot.ledger.life_boost_spent_from_prior_bank, 50.0);
        assert_eq!(slot.ledger.life_boost_spent_from_life_credits, 5.0);
        assert_eq!(slot.ledger.life_net_points(), 405.0);
        assert_eq!(slot.current.kills, 1);
        assert_eq!(slot.deaths, 2);
    }

    #[test]
    fn life_end_capture_keeps_simultaneous_contacts_and_actual_economics() {
        let mut state = EvolvedGenerationState {
            slots: vec![runtime()],
            ..EvolvedGenerationState::default()
        };
        state.slots[0].alive_award(2.0, 50.0).unwrap();
        let mut end = delta(&state.slots[0], 10.0, 400.0, 7, false);
        end.death = Some(EvolvedDeathCause::Wall);
        end.death_flags = 7;
        end.body_owner_id = Some(900);
        end.kills = 1;
        end.corpse_emitted_count = 2;
        end.corpse_emitted_value = 3.0;
        end.corpse_discarded_count = 4;
        end.corpse_discarded_value = 5.0;
        state.apply_physics(&[end]).unwrap();
        state.validate_events().unwrap();
        let record = state.transaction_life_ends[0];
        assert_eq!(state.recent_life_ends.front(), Some(&record));
        assert_eq!(record.cause, EvolvedDeathCause::Wall);
        assert_eq!(record.contact_flags, 7);
        assert_eq!(record.body_owner_id, Some(900));
        assert_eq!(record.physical_id, 1);
        assert_eq!(record.life_ordinal, 1);
        assert_eq!(record.life_net_points, 440.0);
        assert_eq!(record.life_gross_earned, 450.0);
        assert_eq!(record.life_boost_spent, 10.0);
        assert_eq!(record.wallet, 440.0);
        assert_eq!(record.food, 20.0);
        assert_eq!(record.kills, 1);
        assert_eq!(record.final_length, 7);
        assert_eq!(record.alive_seconds, 2.0);
        assert_eq!(record.corpse_discarded_value, 5.0);
        let mut altered = state.clone();
        altered.recent_life_ends[0].cause = EvolvedDeathCause::Body;
        assert_eq!(altered.validate_events(), Err(EvolvedLifeError::Lifecycle));
        let mut altered = state.clone();
        altered.transaction_life_ends[0].food += 1.0;
        assert_eq!(altered.validate_events(), Err(EvolvedLifeError::Lifecycle));
    }

    #[test]
    fn population_sized_transaction_retains_only_the_latest_256_ends() {
        let slots = (0..300)
            .map(|index| {
                let mut owner = runtime();
                owner.population_slot = index;
                owner.physical_id = u64::from(index) + 1;
                owner
            })
            .collect();
        let source = EvolvedGenerationState {
            slots,
            ..EvolvedGenerationState::default()
        };
        let mut staged = EvolvedGenerationState::default();
        staged.stage_from(&source).unwrap();
        let deltas: Vec<_> = staged
            .slots
            .iter()
            .map(|owner| delta(owner, 0.0, 0.0, 5, false))
            .collect();
        staged.apply_physics(&deltas).unwrap();
        staged.validate_event_continuation(&source).unwrap();
        assert_eq!(staged.transaction_life_ends.len(), 300);
        assert_eq!(staged.recent_life_ends.len(), MAX_RECENT_LIFE_ENDS);
        assert_eq!(staged.recent_life_ends.front().unwrap().event_ordinal, 45);
        assert_eq!(staged.recent_life_ends.back().unwrap().event_ordinal, 300);
        let transaction_capacity = staged.transaction_life_ends.capacity();
        let ring_capacity = staged.recent_life_ends.capacity();
        let published = staged.clone();
        staged.stage_from(&published).unwrap();
        staged.validate_event_continuation(&published).unwrap();
        assert!(staged.transaction_life_ends.is_empty());
        assert_eq!(
            staged.transaction_life_ends.capacity(),
            transaction_capacity
        );
        assert_eq!(staged.recent_life_ends.capacity(), ring_capacity);
        let mut altered = staged.clone();
        altered.recent_life_ends[0].food = 1.0;
        assert_eq!(
            altered.validate_event_continuation(&published),
            Err(EvolvedLifeError::Lifecycle)
        );
    }

    #[test]
    fn long_life_history_reuses_storage_and_rejects_unrecorded_deaths() {
        let mut published = EvolvedGenerationState {
            slots: vec![runtime()],
            ..EvolvedGenerationState::default()
        };
        let mut staged = EvolvedGenerationState::default();
        for ordinal in 1..=600 {
            staged.stage_from(&published).unwrap();
            if ordinal > 1 {
                staged.slots[0].begin_life(ordinal, 5, false).unwrap();
            }
            staged.slots[0].alive_award(0.25, 0.1).unwrap();
            let end = delta(&staged.slots[0], 0.0, 0.2, 5, false);
            staged.apply_physics(&[end]).unwrap();
            staged.validate_event_continuation(&published).unwrap();
            std::mem::swap(&mut staged, &mut published);
        }
        assert_eq!(published.life_end_count, 600);
        assert_eq!(published.recent_life_ends.len(), 256);
        assert_eq!(
            published.recent_life_ends.front().unwrap().life_ordinal,
            345
        );
        assert_eq!(published.recent_life_ends.back().unwrap().physical_id, 600);
        let ring_capacity = published.recent_life_ends.capacity();
        staged.stage_from(&published).unwrap();
        staged.slots[0].begin_life(601, 5, false).unwrap();
        staged.slots[0]
            .finalize(EvolvedDeathCause::GodMode)
            .unwrap();
        assert_eq!(
            staged.validate_event_continuation(&published),
            Err(EvolvedLifeError::Lifecycle)
        );
        assert_eq!(staged.recent_life_ends.capacity(), ring_capacity);
        let boundary = EvolvedGenerationState::default();
        boundary.validate_events().unwrap();
        assert_eq!(boundary.life_end_count, 0);
    }

    fn lifecycle_fixture(
        count: usize,
    ) -> (
        EvolvedGenerationState,
        super::super::state::RngStateBundle,
        EvolvedExperimentConfig,
    ) {
        let slots = (0..count)
            .map(|index| {
                let mut slot = runtime();
                slot.population_slot = index as u32;
                slot.physical_id = index as u64 + 1;
                slot.finalize(EvolvedDeathCause::GodMode).unwrap();
                slot
            })
            .collect();
        let stream = super::super::rng::StatefulRng::new(42.0).export_state();
        let rng = super::super::state::RngStateBundle {
            version: super::super::state::RNG_BUNDLE_WITH_EVOLVED_VERSION,
            world: stream.clone(),
            evolution: stream.clone(),
            external_controller: stream,
            baselines: Vec::new(),
            evolved_respawns: super::super::state::EvolvedRespawnRngState::derive_generation(
                7, 1, count,
            )
            .unwrap(),
        };
        let mut policy = EvolvedExperimentConfig::legacy(240.0);
        policy.version = 1;
        policy.enabled = true;
        (
            EvolvedGenerationState {
                version: EVOLVED_LIVES_VERSION,
                slots,
                ..EvolvedGenerationState::default()
            },
            rng,
            policy,
        )
    }

    #[test]
    fn zero_delay_waits_for_next_boundary_and_uniform_zero_draws_exactly_once() {
        let (mut lives, mut rng, mut policy) = lifecycle_fixture(1);
        policy.delay_mode = RespawnDelayMode::Uniform;
        let original = rng.clone();
        let mut expected = rng.evolved_respawns[0].runtime().unwrap();
        expected.next_f64();
        lives
            .schedule_deaths(policy, &mut rng, 20, 1.0, 1.0 / 60.0)
            .unwrap();
        assert_eq!(
            rng.evolved_respawns[0].uniform_state,
            expected.uniform_state()
        );
        assert_eq!(rng.world, original.world);
        assert_eq!(rng.evolution, original.evolution);
        assert_eq!(lives.slots[0].respawn_deadline_step, Some(21));
        let sampled = rng.clone();
        lives
            .schedule_deaths(policy, &mut rng, 20, 1.0, 1.0 / 60.0)
            .unwrap();
        assert_eq!(rng, sampled);
        let mut due = Vec::new();
        lives.collect_due(20, 1.0 / 60.0, &mut due).unwrap();
        assert!(due.is_empty());
        lives.collect_due(21, 1.0 / 60.0, &mut due).unwrap();
        assert_eq!(due, [0]);
        lives.record_failed_attempt(0, 21, 1.0, 224).unwrap();
        assert_eq!(lives.slots[0].life_ordinal, 1);
        assert_eq!(lives.slots[0].successful_respawns, 0);
        assert_eq!(lives.slots[0].fallback_cursor, 224);
        lives.collect_due(35, 1.0 / 60.0, &mut due).unwrap();
        assert!(due.is_empty());
        lives.collect_due(36, 1.0 / 60.0, &mut due).unwrap();
        assert_eq!(due, [0]);
    }

    #[test]
    fn finite_exhaustion_does_not_sample_a_delay_and_world_uniform_preserves_evolution() {
        let (mut lives, mut rng, mut policy) = lifecycle_fixture(1);
        let original = rng.clone();
        policy.max_lives = 1;
        policy.delay_mode = RespawnDelayMode::Uniform;
        lives
            .schedule_deaths(policy, &mut rng, 1, 0.0, 1.0 / 60.0)
            .unwrap();
        assert_eq!(lives.slots[0].status, EvolvedLifeStatus::Exhausted);
        assert_eq!(rng, original);
        let (mut lives, mut rng, mut policy) = lifecycle_fixture(1);
        let original = rng.clone();
        policy.delay_mode = RespawnDelayMode::Uniform;
        policy.rng_mode = RespawnRngMode::World;
        policy.delay_seconds = 10.0;
        lives
            .schedule_deaths(policy, &mut rng, 1, 100.0, 1.0 / 60.0)
            .unwrap();
        let deadline = lives.slots[0].respawn_deadline.unwrap();
        assert!((105.0..=115.0).contains(&deadline));
        assert_ne!(rng.world, original.world);
        assert_eq!(rng.evolved_respawns, original.evolved_respawns);
        assert_eq!(rng.evolution, original.evolution);
    }

    #[test]
    fn three_hundred_due_slots_get_bounded_ordered_attempts_without_consuming_lives() {
        let (mut lives, mut rng, policy) = lifecycle_fixture(300);
        lives
            .schedule_deaths(policy, &mut rng, 0, 0.0, 1.0 / 60.0)
            .unwrap();
        let mut due = Vec::new();
        for step in 1..=38 {
            lives.collect_due(step, 1.0 / 60.0, &mut due).unwrap();
            assert!(due.len() <= 8);
            for &slot in &due {
                lives
                    .record_failed_attempt(slot, step, step as f64 / 60.0, 0)
                    .unwrap();
            }
        }
        assert!(lives.slots.iter().all(|slot| slot.failed_placements >= 1
            && slot.life_ordinal == 1
            && slot.successful_respawns == 0));
        lives.collect_due(39, 1.0 / 60.0, &mut due).unwrap();
        assert_eq!(due, [4, 5, 6, 7, 8, 9, 10, 11]);
        let capacity = due.capacity();
        lives.collect_due(39, 1.0 / 60.0, &mut due).unwrap();
        assert_eq!(due.capacity(), capacity);
    }

    #[test]
    fn pending_time_excludes_participating_newborns_and_deadline_overflow_rejects() {
        let (mut lives, mut rng, policy) = lifecycle_fixture(2);
        lives
            .schedule_deaths(policy, &mut rng, 1, 0.0, 1.0 / 60.0)
            .unwrap();
        lives.slots[0].begin_life(100, 5, false).unwrap();
        lives.slots[0].alive_award(1.0 / 60.0, 0.01).unwrap();
        lives.accrue_pending_time(1.0 / 60.0).unwrap();
        assert_eq!(lives.slots[0].pending_seconds, 0.0);
        assert_eq!(lives.slots[1].pending_seconds, 1.0 / 60.0);
        let (mut lives, mut rng, policy) = lifecycle_fixture(1);
        assert_eq!(
            lives.schedule_deaths(policy, &mut rng, u64::MAX, 0.0, 1.0 / 60.0),
            Err(EvolvedLifeError::Overflow)
        );
    }

    #[test]
    fn inherited_burn_before_credit_preserves_life_net() {
        let mut ledger = EvolvedEconomicLedger::default();
        ledger.credit(50.0).unwrap();
        assert_eq!(ledger.begin_life(true).unwrap(), 50.0);
        ledger.spend(50.0).unwrap();
        ledger.credit(10.0).unwrap();
        assert_eq!(ledger.life_net_points(), 10.0);
        assert_eq!(ledger.life_boost_spent_from_prior_bank, 50.0);
        assert_eq!(ledger.life_boost_spent_from_life_credits, 0.0);
        assert_eq!(ledger.generation_net_earned, 10.0);
        ledger.validate().unwrap();
    }

    #[test]
    fn life_credits_spend_first_and_gross_is_never_capital() {
        let mut ledger = EvolvedEconomicLedger::default();
        ledger.credit(50.0).unwrap();
        ledger.begin_life(true).unwrap();
        ledger.credit(10.0).unwrap();
        ledger.spend(15.0).unwrap();
        assert_eq!(ledger.life_local_wallet, 0.0);
        assert_eq!(ledger.prior_bank_remaining, 45.0);
        assert_eq!(ledger.life_boost_spent_from_life_credits, 10.0);
        assert_eq!(ledger.life_boost_spent_from_prior_bank, 5.0);
        assert_eq!(ledger.generation_gross_earned, 60.0);
        assert_eq!(ledger.begin_life(false).unwrap(), 0.0);
        let before = ledger;
        assert_eq!(ledger.spend(1.0), Err(EvolvedLifeError::Overdraw));
        assert_eq!(ledger, before);
    }

    #[test]
    fn invalid_overflow_and_overdraw_leave_the_ledger_unchanged() {
        let mut ledger = EvolvedEconomicLedger::default();
        ledger.credit(1.0).unwrap();
        for amount in [f64::NAN, f64::INFINITY, -1.0] {
            let before = ledger;
            assert!(ledger.credit(amount).is_err());
            assert!(ledger.spend(amount).is_err());
            assert_eq!(ledger, before);
        }
        let before = ledger;
        assert_eq!(ledger.spend(1.0001), Err(EvolvedLifeError::Overdraw));
        assert_eq!(ledger, before);
    }

    #[test]
    fn a_long_accumulation_history_does_not_increase_overdraw_allowance() {
        let mut ledger = EvolvedEconomicLedger::default();
        ledger.credit(1.0e12).unwrap();
        ledger.operations = 100_000_000;
        let before = ledger;
        assert_eq!(ledger.spend(1.0e12 + 1.0), Err(EvolvedLifeError::Overdraw));
        assert_eq!(ledger, before);
    }

    #[test]
    fn repeated_fractional_credit_and_full_burn_remain_coherent() {
        let mut ledger = EvolvedEconomicLedger::default();
        let mut physical_wallet = 0.0;
        for step in 0..200_000 {
            let credit = if step % 17 == 0 { 3.125031 } else { 0.01 };
            physical_wallet += credit;
            ledger.credit(credit).unwrap();
            let spend = physical_wallet.min(0.333333333333);
            physical_wallet -= spend;
            ledger.spend(spend).unwrap();
            assert!(
                (physical_wallet - ledger.life_local_wallet).abs()
                    <= ledger.rounding_bound(physical_wallet)
            );
        }
        ledger.validate().unwrap();
    }
}
