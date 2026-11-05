//! Constant-sized, sensor-independent accounting for one external physical life.
//!
//! Stage this ledger beside the world transaction and discard it on rollback.
//! Preparing output never consumes counters. Commit the captured marker only
//! with admission of the complete ordered output batch; physics occurring after
//! a pre-movement capture remains available for the next interval or closure.

use super::accounting::SnakeAccountingUpdate;
use super::evolved_lives::EvolvedPhysicsDelta;
use crate::contracts::{
    ExternalRewardDeltasV1, ExternalRewardEndReason, ExternalRewardEnvelopeV1,
    ExternalRewardTransitionKind, EXTERNAL_REWARD_VERSION,
};
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter};

/// Identities have separate roles even when fixture values happen to coincide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewardLifeIdentity {
    pub world_epoch: u64,
    pub lease_id: u64,
    pub physical_snake_id: u64,
    pub frame_snake_id: u32,
}

/// Simulation boundary, never a wall-clock or socket-receipt timestamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RewardBoundary {
    pub completed_step: u64,
    pub elapsed_seconds: f64,
}

/// Invalid accounting must fault preparation without partially changing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewardError(pub &'static str);

impl Display for RewardError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "external reward: {}", self.0)
    }
}

impl std::error::Error for RewardError {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Totals {
    alive_seconds: f64,
    food_value: f64,
    food_count: u64,
    survival_points: f64,
    food_points: f64,
    kill_points: f64,
    other_points: f64,
    boost_spent: f64,
    kills: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Marker {
    revision: u64,
    sequence: u64,
    boundary: RewardBoundary,
    totals: Totals,
    wallet: f64,
    length: i64,
}

/// One life only; replacing a snake requires a new ledger and assignment identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalRewardLedger {
    identity: RewardLifeIdentity,
    start: RewardBoundary,
    starting_length: i64,
    initial_wallet: f64,
    totals: Totals,
    wallet: f64,
    length: i64,
    alive: bool,
    closed: bool,
    admitted: Option<Marker>,
    revision: u64,
}

/// Immutable capture retained until its complete output batch is admitted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedRewardDelivery {
    identity: RewardLifeIdentity,
    predecessor: Option<Marker>,
    marker: Marker,
    kind: ExternalRewardTransitionKind,
    reason: Option<ExternalRewardEndReason>,
    deltas: ExternalRewardDeltasV1,
}

/// Shared pre-movement observation snapshot, retained while physics stages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RewardObservationCapture {
    ledger: ExternalRewardLedger,
    boundary: RewardBoundary,
}

impl RewardObservationCapture {
    /// Bind the snapshot only after the coordinator reserves its output sequence.
    pub fn prepare(self, sequence: u64) -> Result<PreparedRewardDelivery, RewardError> {
        self.ledger.prepare(sequence, self.boundary, None)
    }

    /// Close an immutable old-life snapshot after its assignment event is reserved.
    /// Authoritative death takes precedence over the supplied administrative cutoff.
    pub fn prepare_closure(
        self,
        sequence: u64,
        reason: ExternalRewardEndReason,
    ) -> Result<PreparedRewardDelivery, RewardError> {
        self.ledger.prepare(sequence, self.boundary, Some(reason))
    }
}

impl PreparedRewardDelivery {
    /// Construct wire metadata only at output preparation, outside physics kernels.
    pub fn envelope(&self, run_id: &str) -> Result<ExternalRewardEnvelopeV1, RewardError> {
        if run_id.is_empty() {
            return Err(RewardError("run identity is empty"));
        }
        Ok(ExternalRewardEnvelopeV1 {
            reward_version: EXTERNAL_REWARD_VERSION,
            run_id: run_id.to_owned(),
            world_epoch: format!("{:016x}", self.identity.world_epoch),
            lease_id: format!("{:016x}", self.identity.lease_id),
            physical_snake_id: format!("{:016x}", self.identity.physical_snake_id),
            snake_id: self.identity.frame_snake_id,
            observation_sequence: format!("{:016x}", self.marker.sequence),
            previous_observation_sequence: self
                .predecessor
                .map(|marker| format!("{:016x}", marker.sequence)),
            from_step: format!(
                "{:016x}",
                self.predecessor
                    .map_or(self.marker.boundary.completed_step, |marker| {
                        marker.boundary.completed_step
                    })
            ),
            to_step: format!("{:016x}", self.marker.boundary.completed_step),
            transition_kind: self.kind,
            reason: self.reason,
            deltas: self.deltas,
        })
    }
}

impl ExternalRewardLedger {
    /// Capture counters at the same immutable boundary as the sensor vector.
    pub fn capture(&self, boundary: RewardBoundary) -> RewardObservationCapture {
        RewardObservationCapture {
            ledger: *self,
            boundary,
        }
    }
    /// Include runtime accounting in the existing detached-source mutation guard.
    pub(crate) fn hash_into(&self, hash: &mut Sha256) {
        hash.update(b"external-reward-v1\0");
        for value in [
            self.identity.world_epoch,
            self.identity.lease_id,
            self.identity.physical_snake_id,
            u64::from(self.identity.frame_snake_id),
            self.start.completed_step,
            self.start.elapsed_seconds.to_bits(),
            self.starting_length as u64,
            self.initial_wallet.to_bits(),
            self.wallet.to_bits(),
            self.length as u64,
            self.revision,
        ] {
            hash.update(value.to_le_bytes());
        }
        hash.update([u8::from(self.alive), u8::from(self.closed)]);
        hash_totals(hash, self.totals);
        if let Some(marker) = self.admitted {
            hash.update([1]);
            for value in [
                marker.revision,
                marker.sequence,
                marker.boundary.completed_step,
                marker.boundary.elapsed_seconds.to_bits(),
                marker.wallet.to_bits(),
                marker.length as u64,
            ] {
                hash.update(value.to_le_bytes());
            }
            hash_totals(hash, marker.totals);
        } else {
            hash.update([0]);
        }
    }
    /// Retained identity, independent of the lease's current socket connection.
    pub const fn identity(&self) -> RewardLifeIdentity {
        self.identity
    }

    /// Whether an observation baseline has entered the authoritative output stream.
    pub const fn has_delivery(&self) -> bool {
        self.admitted.is_some()
    }

    /// Generation assignment is prepared against the old authority and becomes
    /// current only after the successor's epoch commits. A newborn may adopt
    /// that epoch before its first observation; an observed life cannot cross it.
    pub fn bind_observation_world(&mut self, world_epoch: u64) -> Result<(), RewardError> {
        if world_epoch == 0 || (self.has_delivery() && self.identity.world_epoch != world_epoch) {
            return Err(RewardError("observed life cannot change world identity"));
        }
        self.identity.world_epoch = world_epoch;
        Ok(())
    }

    /// Initial balances and body length establish a baseline, never earnings.
    pub fn new(
        identity: RewardLifeIdentity,
        start: RewardBoundary,
        starting_length: usize,
        initial_wallet: f64,
    ) -> Result<Self, RewardError> {
        validate_boundary(start)?;
        if identity.world_epoch == 0
            || identity.lease_id == 0
            || identity.physical_snake_id == 0
            || identity.frame_snake_id == 0
            || identity.frame_snake_id > super::state::FRAME_V1_MAX_EXACT_ID
        {
            return Err(RewardError("invalid life identity"));
        }
        nonnegative(initial_wallet)?;
        let length = checked_length(starting_length)?;
        if length == 0 {
            return Err(RewardError("initial body is empty"));
        }
        Ok(Self {
            identity,
            start,
            starting_length: length,
            initial_wallet,
            totals: Totals::default(),
            wallet: initial_wallet,
            length,
            alive: true,
            closed: false,
            admitted: None,
            revision: 0,
        })
    }

    /// Apply the exact prefix live-time credit, once, in the staged transaction.
    pub fn record_live_time(&mut self, seconds: f64, points: f64) -> Result<(), RewardError> {
        if !self.alive || self.closed {
            return Err(RewardError("live-time credit after life end"));
        }
        nonnegative(seconds)?;
        nonnegative(points)?;
        let mut next = *self;
        next.advance_revision()?;
        next.totals.alive_seconds += seconds;
        next.totals.survival_points += points;
        next.wallet += points;
        next.validate_totals()?;
        *self = next;
        Ok(())
    }

    /// Consume the native prefix proposal rather than deriving time from packets.
    pub fn record_accounting(&mut self, update: &SnakeAccountingUpdate) -> Result<(), RewardError> {
        if update.snake_id != self.identity.physical_snake_id
            || !near(update.previous_points, self.wallet)
        {
            return Err(RewardError("prefix accounting identity or wallet mismatch"));
        }
        for value in [
            update.previous_age_seconds,
            update.next_age_seconds,
            update.previous_points,
            update.next_points,
        ] {
            nonnegative(value)?;
        }
        self.record_live_time(
            update.next_age_seconds - update.previous_age_seconds,
            update.next_points - update.previous_points,
        )
    }

    /// Consume one accepted substep's actual food, boost, kill and final-body data.
    /// The coordinator applies each substep once; sensor evaluation is irrelevant.
    pub fn record_physics(&mut self, delta: &EvolvedPhysicsDelta) -> Result<(), RewardError> {
        if self.closed || delta.snake_id != self.identity.physical_snake_id {
            return Err(RewardError("physics identity mismatch or closed life"));
        }
        for value in [
            delta.boost_points_spent,
            delta.credits,
            delta.food,
            delta.food_credits,
            delta.kill_credits,
            delta.wallet_after,
        ] {
            nonnegative(value)?;
        }
        if !near(delta.credits, delta.food_credits + delta.kill_credits)
            || !near(
                self.wallet + delta.credits - delta.boost_points_spent,
                delta.wallet_after,
            )
            || delta.alive_after == delta.death.is_some() && self.alive
        {
            return Err(RewardError("inconsistent physics accounting"));
        }
        if !self.alive {
            // Later collision-only substeps may still enumerate the dead record.
            // They cannot award its last food/kill event a second time.
            if delta.death.is_some()
                || delta.alive_after
                || delta.credits != 0.0
                || delta.food != 0.0
                || delta.food_count != 0
                || delta.kills != 0
                || delta.boost_points_spent != 0.0
            {
                return Err(RewardError("event after death"));
            }
            return Ok(());
        }
        let mut next = *self;
        next.advance_revision()?;
        next.totals.food_value += delta.food;
        next.totals.food_count = add_count(next.totals.food_count, delta.food_count)?;
        next.totals.food_points += delta.food_credits;
        next.totals.kill_points += delta.kill_credits;
        next.totals.boost_spent += delta.boost_points_spent;
        next.totals.kills = add_count(next.totals.kills, delta.kills)?;
        next.wallet = delta.wallet_after;
        // This length is captured by physics before terminal cleanup/replacement.
        next.length = checked_length(delta.length_after)?;
        if next.length == 0 {
            return Err(RewardError("missing pre-cleanup body length"));
        }
        next.alive = delta.alive_after;
        next.validate_totals()?;
        *self = next;
        Ok(())
    }

    /// Explicit ordinary adjustments remain distinct from assignment balances.
    pub fn record_point_adjustment(&mut self, points: f64) -> Result<(), RewardError> {
        if !self.alive || self.closed || !points.is_finite() {
            return Err(RewardError("invalid point adjustment"));
        }
        let mut next = *self;
        next.advance_revision()?;
        next.totals.other_points += points;
        next.wallet += points;
        next.validate_totals()?;
        *self = next;
        Ok(())
    }

    /// Pure capture: repeated preparation/backpressure does not consume deltas.
    /// The first observation establishes its complete current-state baseline.
    /// Events before that observation have no preceding training decision.
    pub fn prepare(
        &self,
        sequence: u64,
        boundary: RewardBoundary,
        cutoff: Option<ExternalRewardEndReason>,
    ) -> Result<PreparedRewardDelivery, RewardError> {
        use ExternalRewardTransitionKind::{Initial, Live, Terminal, Truncated};
        validate_boundary(boundary)?;
        let previous_boundary = self.admitted.map_or(self.start, |m| m.boundary);
        if self.closed
            || sequence == 0
            || self.admitted.is_some_and(|m| sequence <= m.sequence)
            || boundary.completed_step < previous_boundary.completed_step
            || boundary.elapsed_seconds < previous_boundary.elapsed_seconds
        {
            return Err(RewardError("closed or nonmonotonic delivery"));
        }
        let kind = if !self.alive {
            Terminal
        } else if cutoff.is_some() {
            if cutoff == Some(ExternalRewardEndReason::Death) {
                return Err(RewardError("death requested for a live snake"));
            }
            Truncated
        } else if self.admitted.is_none() {
            Initial
        } else {
            Live
        };
        if self.admitted.is_none() && kind != Initial {
            return Err(RewardError(
                "life ended before initial training observation",
            ));
        }
        let previous_totals = self.admitted.map_or(Totals::default(), |m| m.totals);
        let previous_wallet = self.admitted.map_or(self.initial_wallet, |m| m.wallet);
        let previous_length = self.admitted.map_or(self.starting_length, |m| m.length);
        let deltas = if kind == Initial {
            ExternalRewardDeltasV1::default()
        } else {
            ExternalRewardDeltasV1 {
                elapsed_seconds: boundary.elapsed_seconds - previous_boundary.elapsed_seconds,
                alive_seconds_delta: self.totals.alive_seconds - previous_totals.alive_seconds,
                food_value_delta: self.totals.food_value - previous_totals.food_value,
                food_count_delta: self.totals.food_count - previous_totals.food_count,
                length_delta: self.length - previous_length,
                retained_growth_delta: (self.length - self.starting_length).max(0)
                    - (previous_length - self.starting_length).max(0),
                points_delta: self.wallet - previous_wallet,
                survival_points_delta: self.totals.survival_points
                    - previous_totals.survival_points,
                food_points_delta: self.totals.food_points - previous_totals.food_points,
                kill_points_delta: self.totals.kill_points - previous_totals.kill_points,
                other_points_delta: self.totals.other_points - previous_totals.other_points,
                boost_points_spent_delta: self.totals.boost_spent - previous_totals.boost_spent,
                kills_delta: self.totals.kills - previous_totals.kills,
            }
        };
        if !near(
            deltas.points_delta,
            deltas.survival_points_delta
                + deltas.food_points_delta
                + deltas.kill_points_delta
                + deltas.other_points_delta
                - deltas.boost_points_spent_delta,
        ) {
            return Err(RewardError("interval accounting does not conserve points"));
        }
        Ok(PreparedRewardDelivery {
            identity: self.identity,
            predecessor: self.admitted,
            marker: Marker {
                revision: self.revision,
                sequence,
                boundary,
                totals: self.totals,
                wallet: self.wallet,
                length: self.length,
            },
            kind,
            reason: if kind == Terminal {
                Some(ExternalRewardEndReason::Death)
            } else {
                cutoff
            },
            deltas,
        })
    }

    /// Advance only the captured delivery marker; preserve post-capture events.
    /// The caller preflights every ledger before admitting a complete batch.
    pub fn commit_delivery(
        &mut self,
        prepared: &PreparedRewardDelivery,
    ) -> Result<(), RewardError> {
        self.validate_delivery(prepared)?;
        self.admitted = Some(prepared.marker);
        self.closed = matches!(
            prepared.kind,
            ExternalRewardTransitionKind::Terminal | ExternalRewardTransitionKind::Truncated
        );
        Ok(())
    }

    /// No-write check for whole-batch admission before any marker is changed.
    pub fn validate_delivery(&self, prepared: &PreparedRewardDelivery) -> Result<(), RewardError> {
        if self.closed
            || self.identity != prepared.identity
            || self.admitted != prepared.predecessor
            || prepared.marker.revision > self.revision
        {
            return Err(RewardError("stale or mismatched delivery marker"));
        }
        let snapshot = prepared.marker;
        let current = self.totals;
        let captured = snapshot.totals;
        if captured.food_count > current.food_count
            || captured.kills > current.kills
            || captured.alive_seconds > current.alive_seconds
            || captured.food_value > current.food_value
            || captured.survival_points > current.survival_points
            || captured.food_points > current.food_points
            || captured.kill_points > current.kill_points
            || captured.boost_spent > current.boost_spent
        {
            return Err(RewardError(
                "delivery capture is outside the staged transaction",
            ));
        }
        let is_closure = matches!(
            prepared.kind,
            ExternalRewardTransitionKind::Terminal | ExternalRewardTransitionKind::Truncated
        );
        if is_closure && snapshot.revision != self.revision {
            return Err(RewardError("life changed after closure capture"));
        }
        if (snapshot.revision == self.revision || is_closure)
            && (captured != current
                || snapshot.wallet != self.wallet
                || snapshot.length != self.length)
        {
            return Err(RewardError("delivery snapshot does not match staged state"));
        }
        Ok(())
    }

    fn advance_revision(&mut self) -> Result<(), RewardError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(RewardError("accounting revision overflow"))?;
        Ok(())
    }

    fn validate_totals(&self) -> Result<(), RewardError> {
        for value in [
            self.totals.alive_seconds,
            self.totals.food_value,
            self.totals.survival_points,
            self.totals.food_points,
            self.totals.kill_points,
            self.totals.boost_spent,
            self.wallet,
        ] {
            nonnegative(value)?;
        }
        if !self.totals.other_points.is_finite()
            || !near(
                self.wallet - self.initial_wallet,
                self.totals.survival_points
                    + self.totals.food_points
                    + self.totals.kill_points
                    + self.totals.other_points
                    - self.totals.boost_spent,
            )
        {
            return Err(RewardError("point conservation failed"));
        }
        Ok(())
    }
}

fn hash_totals(hash: &mut Sha256, totals: Totals) {
    for value in [
        totals.alive_seconds.to_bits(),
        totals.food_value.to_bits(),
        totals.food_count,
        totals.survival_points.to_bits(),
        totals.food_points.to_bits(),
        totals.kill_points.to_bits(),
        totals.other_points.to_bits(),
        totals.boost_spent.to_bits(),
        totals.kills,
    ] {
        hash.update(value.to_le_bytes());
    }
}

fn checked_length(length: usize) -> Result<i64, RewardError> {
    i64::try_from(length)
        .ok()
        .filter(|length| *length <= 9_007_199_254_740_991)
        .ok_or(RewardError("body length overflow"))
}

fn add_count(left: u64, right: u64) -> Result<u64, RewardError> {
    left.checked_add(right)
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or(RewardError("wire count overflow"))
}

fn validate_boundary(boundary: RewardBoundary) -> Result<(), RewardError> {
    nonnegative(boundary.elapsed_seconds)
}

fn nonnegative(value: f64) -> Result<(), RewardError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(RewardError("nonfinite or negative quantity"))
    }
}

fn near(left: f64, right: f64) -> bool {
    left.is_finite()
        && right.is_finite()
        && (left - right).abs() <= 1.0e-9 * left.abs().max(right.abs()).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::evolved_lives::EvolvedDeathCause;

    fn boundary(step: u64) -> RewardBoundary {
        RewardBoundary {
            completed_step: step,
            elapsed_seconds: step as f64 / 60.0,
        }
    }

    fn ledger() -> ExternalRewardLedger {
        ExternalRewardLedger::new(
            RewardLifeIdentity {
                world_epoch: 1,
                lease_id: 2,
                physical_snake_id: 3,
                frame_snake_id: 4,
            },
            boundary(0),
            10,
            50.0,
        )
        .unwrap()
    }

    fn admitted() -> ExternalRewardLedger {
        let mut ledger = ledger();
        let initial = ledger.prepare(1, boundary(0), None).unwrap();
        assert_eq!(
            initial.envelope("run").unwrap().deltas,
            ExternalRewardDeltasV1::default()
        );
        ledger.commit_delivery(&initial).unwrap();
        ledger
    }

    fn physics(wallet: f64, length: usize) -> EvolvedPhysicsDelta {
        EvolvedPhysicsDelta {
            snake_id: 3,
            wallet_after: wallet,
            length_after: length,
            alive_after: true,
            ..EvolvedPhysicsDelta::default()
        }
    }

    #[test]
    fn survival_accumulates_simulated_time_and_excludes_initial_balance() {
        let mut ledger = admitted();
        for _ in 0..6 {
            ledger.record_live_time(1.0 / 60.0, 0.01).unwrap();
        }
        let delivery = ledger.prepare(2, boundary(6), None).unwrap();
        let envelope = delivery.envelope("run").unwrap();
        assert!(near(envelope.deltas.alive_seconds_delta, 0.1));
        assert!(near(envelope.deltas.points_delta, 0.06));
        assert!(near(envelope.deltas.survival_points_delta, 0.06));
        assert_eq!(
            envelope.previous_observation_sequence.as_deref(),
            Some("0000000000000001")
        );
        assert_eq!(envelope.snake_id, 4);
        assert_eq!(envelope.physical_snake_id, "0000000000000003");
        assert_eq!(envelope.lease_id, "0000000000000002");
        assert_eq!(envelope.reward_version, EXTERNAL_REWARD_VERSION);
    }

    #[test]
    fn prefix_live_credit_does_not_require_physical_clock_to_advance() {
        let mut ledger = admitted();
        ledger.record_live_time(1.0 / 60.0, 0.01).unwrap();
        // Prefix accounting and pre-movement sensors share a physical boundary.
        // Credited live time is an accounting fact, not the observation clock.
        let prefix = ledger.prepare(2, boundary(0), None).unwrap();
        let envelope = prefix.envelope("run").unwrap();
        assert_eq!(envelope.deltas.elapsed_seconds, 0.0);
        assert!(near(envelope.deltas.alive_seconds_delta, 1.0 / 60.0));
        assert!(near(envelope.deltas.survival_points_delta, 0.01));
        ledger.commit_delivery(&prefix).unwrap();

        let cutoff = ledger
            .prepare(3, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap()
            .envelope("run")
            .unwrap();
        assert!(near(cutoff.deltas.elapsed_seconds, 1.0 / 60.0));
        assert_eq!(cutoff.deltas.alive_seconds_delta, 0.0);
        assert_eq!(cutoff.deltas.points_delta, 0.0);
    }

    #[test]
    fn output_capture_is_pure_and_post_capture_physics_remains_in_tail() {
        let mut ledger = admitted();
        ledger.record_live_time(1.0 / 60.0, 0.01).unwrap();
        let captured = ledger.prepare(2, boundary(1), None).unwrap();
        let before = ledger;
        assert_eq!(
            captured.envelope("run").unwrap(),
            ledger
                .prepare(2, boundary(1), None)
                .unwrap()
                .envelope("run")
                .unwrap()
        );
        assert_eq!(ledger, before);
        let mut delta = physics(70.01, 11);
        delta.food = 1.0;
        delta.food_count = 1;
        delta.credits = 20.0;
        delta.food_credits = 20.0;
        ledger.record_physics(&delta).unwrap();
        ledger.commit_delivery(&captured).unwrap();
        let tail = ledger
            .prepare(3, boundary(2), None)
            .unwrap()
            .envelope("run")
            .unwrap();
        assert_eq!(tail.deltas.food_count_delta, 1);
        assert_eq!(tail.deltas.food_value_delta, 1.0);
        assert_eq!(tail.deltas.retained_growth_delta, 1);
        assert!(near(tail.deltas.points_delta, 20.0));
        assert_eq!(tail.deltas.survival_points_delta, 0.0);
    }

    #[test]
    fn food_growth_kill_and_death_share_one_final_interval() {
        let mut ledger = admitted();
        let mut delta = physics(470.0, 11);
        delta.food = 1.0;
        delta.food_count = 1;
        delta.food_credits = 20.0;
        delta.kill_credits = 400.0;
        delta.credits = 420.0;
        delta.kills = 1;
        delta.alive_after = false;
        delta.death = Some(EvolvedDeathCause::Body);
        ledger.record_physics(&delta).unwrap();
        // Death wins even when the generation guard also fires.
        let closure = ledger
            .prepare(2, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap();
        let wire = closure.envelope("run").unwrap();
        assert_eq!(wire.transition_kind, ExternalRewardTransitionKind::Terminal);
        assert_eq!(wire.reason, Some(ExternalRewardEndReason::Death));
        assert_eq!(wire.deltas.retained_growth_delta, 1);
        assert_eq!(wire.deltas.length_delta, 1);
        assert_eq!(wire.deltas.kills_delta, 1);
        assert_eq!(wire.deltas.food_count_delta, 1);
        ledger.commit_delivery(&closure).unwrap();
        assert!(ledger.commit_delivery(&closure).is_err());
        assert!(ledger.prepare(3, boundary(2), None).is_err());
    }

    #[test]
    fn live_generation_cutoff_is_a_truncation_with_full_tail() {
        let mut ledger = admitted();
        ledger.record_live_time(1.0 / 60.0, 0.01).unwrap();
        let closure = ledger
            .prepare(2, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap();
        let wire = closure.envelope("run").unwrap();
        assert_eq!(
            wire.transition_kind,
            ExternalRewardTransitionKind::Truncated
        );
        assert_eq!(wire.reason, Some(ExternalRewardEndReason::Generation));
        assert!(near(wire.deltas.alive_seconds_delta, 1.0 / 60.0));
        ledger.commit_delivery(&closure).unwrap();
        assert!(ledger.record_live_time(1.0 / 60.0, 0.01).is_err());
    }

    #[test]
    fn loss_and_regrowth_only_pay_net_retained_growth() {
        let mut ledger = admitted();
        let mut eat = physics(90.0, 12);
        eat.credits = 40.0;
        eat.food_credits = 40.0;
        eat.food = 2.0;
        eat.food_count = 2;
        ledger.record_physics(&eat).unwrap();
        let first = ledger.prepare(2, boundary(1), None).unwrap();
        assert_eq!(first.deltas.retained_growth_delta, 2);
        ledger.commit_delivery(&first).unwrap();
        let mut burn = physics(80.0, 10);
        burn.boost_points_spent = 10.0;
        ledger.record_physics(&burn).unwrap();
        let second = ledger.prepare(3, boundary(2), None).unwrap();
        assert_eq!(second.deltas.retained_growth_delta, -2);
        assert_eq!(second.deltas.points_delta, -10.0);
        ledger.commit_delivery(&second).unwrap();
        eat.wallet_after = 120.0;
        ledger.record_physics(&eat).unwrap();
        let third = ledger.prepare(4, boundary(3), None).unwrap();
        assert_eq!(
            second.deltas.retained_growth_delta + third.deltas.retained_growth_delta,
            0
        );
    }

    #[test]
    fn duplicate_or_reassigned_markers_never_consume_reward() {
        let mut ledger = admitted();
        let delivery = ledger.prepare(2, boundary(1), None).unwrap();
        let mut replacement = ExternalRewardLedger::new(
            RewardLifeIdentity {
                lease_id: 5,
                physical_snake_id: 6,
                ..ledger.identity
            },
            boundary(1),
            10,
            50.0,
        )
        .unwrap();
        let before = replacement;
        assert!(replacement.commit_delivery(&delivery).is_err());
        assert_eq!(replacement, before);
        ledger.commit_delivery(&delivery).unwrap();
        let before = ledger;
        assert!(ledger.commit_delivery(&delivery).is_err());
        assert_eq!(ledger, before);
    }

    #[test]
    fn invalid_physics_and_numeric_overflow_are_atomic() {
        let mut ledger = admitted();
        let before = ledger;
        let mut bad = physics(70.0, 11);
        bad.food_credits = 20.0;
        assert!(ledger.record_physics(&bad).is_err());
        bad.credits = 20.0;
        bad.snake_id = 999;
        assert!(ledger.record_physics(&bad).is_err());
        assert!(ledger.record_live_time(f64::NAN, 0.0).is_err());
        assert!(ledger.record_point_adjustment(-1000.0).is_err());
        assert_eq!(ledger, before);
    }

    #[test]
    fn initial_delivery_can_be_admitted_after_physics_without_losing_tail() {
        let mut ledger = ledger();
        let initial = ledger.prepare(1, boundary(0), None).unwrap();
        let mut delta = physics(70.0, 11);
        delta.food_credits = 20.0;
        delta.credits = 20.0;
        delta.food = 1.0;
        delta.food_count = 1;
        ledger.record_physics(&delta).unwrap();
        assert_eq!(
            ledger.prepare(1, boundary(1), None).unwrap().deltas,
            ExternalRewardDeltasV1::default()
        );
        ledger.commit_delivery(&initial).unwrap();
        let tail = ledger.prepare(2, boundary(1), None).unwrap();
        assert_eq!(tail.deltas.food_value_delta, 1.0);
    }

    #[test]
    fn discarded_transaction_cannot_advance_live_delivery_markers() {
        let mut ledger = admitted();
        let mut staged = ledger;
        staged.record_live_time(1.0 / 60.0, 0.01).unwrap();
        let delivery = staged.prepare(2, boundary(1), None).unwrap();
        let before = ledger;
        assert!(ledger.commit_delivery(&delivery).is_err());
        assert_eq!(ledger, before);
    }

    #[test]
    fn closure_cannot_discard_events_recorded_after_its_capture() {
        let mut ledger = admitted();
        let closure = ledger
            .prepare(2, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap();
        ledger.record_live_time(1.0 / 60.0, 0.01).unwrap();
        assert!(ledger.commit_delivery(&closure).is_err());
        let complete = ledger
            .prepare(2, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap();
        ledger.commit_delivery(&complete).unwrap();
    }

    #[test]
    fn death_without_economic_change_invalidates_a_prepared_truncation() {
        let mut ledger = admitted();
        let cutoff = ledger
            .prepare(2, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap();
        let mut death = physics(50.0, 10);
        death.alive_after = false;
        death.death = Some(EvolvedDeathCause::Wall);
        ledger.record_physics(&death).unwrap();
        assert!(ledger.commit_delivery(&cutoff).is_err());
        let terminal = ledger
            .prepare(2, boundary(1), Some(ExternalRewardEndReason::Generation))
            .unwrap();
        assert_eq!(terminal.kind, ExternalRewardTransitionKind::Terminal);
        ledger.commit_delivery(&terminal).unwrap();
    }

    #[test]
    fn first_shared_observation_baselines_native_prefix_credits_without_spawn_reward() {
        let mut ledger = ledger();
        let update = SnakeAccountingUpdate {
            snake_id: 3,
            source_index: 99,
            previous_age_seconds: 0.0,
            next_age_seconds: 1.0 / 60.0,
            previous_points: 50.0,
            next_points: 50.01,
        };
        ledger.record_accounting(&update).unwrap();
        let initial = ledger.prepare(1, boundary(1), None).unwrap();
        assert_eq!(initial.deltas, ExternalRewardDeltasV1::default());
        ledger.commit_delivery(&initial).unwrap();
        ledger
            .record_accounting(&SnakeAccountingUpdate {
                previous_age_seconds: update.next_age_seconds,
                next_age_seconds: 2.0 / 60.0,
                previous_points: update.next_points,
                next_points: 50.02,
                ..update
            })
            .unwrap();
        let next = ledger.prepare(2, boundary(2), None).unwrap();
        assert!(near(next.deltas.alive_seconds_delta, 1.0 / 60.0));
        assert!(near(next.deltas.survival_points_delta, 0.01));
        let before = ledger;
        assert!(ledger.record_accounting(&update).is_err());
        assert_eq!(ledger, before);
    }
}
