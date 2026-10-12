//! Fair, transaction-staged work grants for sensor-v4 capsule rays.
//! Qualification supplies the limits; this module does not select production
//! defaults or redistribute unused work according to worker completion order.

use super::calculation::CalculationBatchKey;
use super::state::{BrainHandle, SnakeKind, SnakeState};
use std::fmt::{Display, Formatter};

/// Stable observation ownership. Physical respawn IDs and array positions
/// deliberately do not identify evolved or baseline owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RayObservationOwner {
    Evolved(u32),
    Baseline(u32),
    /// Retained and external observations share their stable brain allocation.
    Brain(BrainHandle),
}

impl RayObservationOwner {
    /// Resolve a life-independent owner from an admitted source snake.
    pub fn of(snake: &SnakeState) -> Result<Self, RayWorkError> {
        match snake.kind {
            SnakeKind::Evolved => snake.population_slot.map(Self::Evolved),
            SnakeKind::Baseline => snake.baseline_slot.map(Self::Baseline),
            SnakeKind::External | SnakeKind::Resurrected => snake.brain.map(Self::Brain),
        }
        .filter(|owner| owner.valid())
        .ok_or(RayWorkError::Owner)
    }

    fn valid(self) -> bool {
        match self {
            Self::Evolved(slot) => slot < 300,
            Self::Baseline(slot) => slot < 512,
            Self::Brain(handle) => handle.id != 0 && handle.epoch != 0,
        }
    }
}

/// Validated per-observation and batch bounds. Maximum due includes every
/// admitted class and coincident first/takeover observation, not just evolveds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayWorkLimits {
    minimum: u32,
    cap: u32,
    batch: u32,
    maximum_due: usize,
}

impl RayWorkLimits {
    /// Admit an explicitly measured candidate; no arbitrary minimum is chosen.
    pub fn new(
        minimum: u32,
        cap: u32,
        batch: u32,
        maximum_due: usize,
    ) -> Result<Self, RayWorkError> {
        if minimum == 0 || minimum > cap || maximum_due == 0 || maximum_due > 512 {
            return Err(RayWorkError::Limits);
        }
        if u64::from(minimum) * maximum_due as u64 > u64::from(batch) {
            return Err(RayWorkError::Limits);
        }
        Ok(Self {
            minimum,
            cap,
            batch,
            maximum_due,
        })
    }

    pub const fn minimum(self) -> u32 {
        self.minimum
    }
    pub const fn cap(self) -> u32 {
        self.cap
    }
    pub const fn batch(self) -> u32 {
        self.batch
    }
    pub const fn maximum_due(self) -> usize {
        self.maximum_due
    }
}

/// Continuation committed with a complete observation transaction. Revision
/// guards even a full-ring grant whose next owner equals its previous owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RayQuotaCursor {
    next_owner: Option<RayObservationOwner>,
    revision: u64,
}

impl RayQuotaCursor {
    pub const fn next_owner(self) -> Option<RayObservationOwner> {
        self.next_owner
    }
    pub const fn revision(self) -> u64 {
        self.revision
    }
}

/// Private continuation proposal. Discarding a failed step leaves the cursor
/// unchanged; a caller must publish only with its exact calculation batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayQuotaProposal {
    batch: CalculationBatchKey,
    source: RayQuotaCursor,
    next: RayQuotaCursor,
    granted: u32,
}

impl RayQuotaProposal {
    pub const fn granted(self) -> u32 {
        self.granted
    }

    /// Validate before any authority writes in the enclosing transaction.
    pub fn validate(
        self,
        batch: CalculationBatchKey,
        cursor: RayQuotaCursor,
    ) -> Result<(), RayWorkError> {
        if self.batch != batch || self.source != cursor {
            return Err(RayWorkError::StaleProposal);
        }
        Ok(())
    }

    /// Publish the small continuation after the whole boundary has succeeded.
    pub fn commit(
        self,
        batch: CalculationBatchKey,
        cursor: &mut RayQuotaCursor,
    ) -> Result<(), RayWorkError> {
        self.validate(batch, *cursor)?;
        *cursor = self.next;
        Ok(())
    }
}

/// Prepare equal base quotas plus single-unit rotating remainder grants.
/// Owners must be canonical, unique and sorted; output is caller-owned scratch.
/// All validation precedes writes, and no authoritative cursor changes here.
pub fn prepare_ray_quotas(
    batch: CalculationBatchKey,
    limits: RayWorkLimits,
    cursor: RayQuotaCursor,
    owners: &[RayObservationOwner],
    grants: &mut [u32],
) -> Result<RayQuotaProposal, RayWorkError> {
    if owners.len() > limits.maximum_due || grants.len() != owners.len() {
        return Err(RayWorkError::Shape);
    }
    if owners.iter().any(|owner| !owner.valid()) || owners.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(RayWorkError::Owner);
    }
    if owners.is_empty() {
        return Ok(RayQuotaProposal {
            batch,
            source: cursor,
            next: cursor,
            granted: 0,
        });
    }
    let revision = cursor
        .revision
        .checked_add(1)
        .ok_or(RayWorkError::Overflow)?;
    let count = owners.len() as u64;
    let total = u64::from(limits.batch).min(count * u64::from(limits.cap));
    let base = (total / count) as u32;
    debug_assert!(base >= limits.minimum && base <= limits.cap);
    let remainder = (total % count) as usize;
    let first = cursor.next_owner.map_or(0, |owner| {
        let lower = owners.partition_point(|candidate| *candidate < owner);
        if lower == owners.len() {
            0
        } else {
            lower
        }
    });
    grants.fill(base);
    for offset in 0..remainder {
        grants[(first + offset) % owners.len()] += 1;
    }
    let next_owner = if remainder == 0 {
        cursor.next_owner
    } else {
        Some(owners[(first + remainder) % owners.len()])
    };
    Ok(RayQuotaProposal {
        batch,
        source: cursor,
        next: RayQuotaCursor {
            next_owner,
            revision,
        },
        granted: total as u32,
    })
}

/// Charged traversal operations. Candidate visits include duplicate/broad-phase
/// rejection; exact intersections and cell lookups cannot bypass the grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayWorkUnit {
    Cell,
    Candidate,
    Intersection,
}

/// Fixed-size work diagnostics, independent of world or life history length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RayWorkDiagnostics {
    pub cells: u32,
    pub candidates: u32,
    pub intersections: u32,
    pub saturated: bool,
}

/// Worker-local meter admitted by the coordinator's immutable grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayWorkMeter {
    remaining: u32,
    diagnostics: RayWorkDiagnostics,
}

impl RayWorkMeter {
    pub const fn new(grant: u32) -> Self {
        Self {
            remaining: grant,
            diagnostics: RayWorkDiagnostics {
                cells: 0,
                candidates: 0,
                intersections: 0,
                saturated: false,
            },
        }
    }
    pub const fn remaining(self) -> u32 {
        self.remaining
    }
    pub const fn diagnostics(self) -> RayWorkDiagnostics {
        self.diagnostics
    }

    /// Charge before an operation. False leaves the operation unexecuted and
    /// requires conservative incomplete-ray output, never a false-clear route.
    pub fn charge(&mut self, unit: RayWorkUnit) -> bool {
        if self.remaining == 0 {
            self.diagnostics.saturated = true;
            return false;
        }
        self.remaining -= 1;
        match unit {
            RayWorkUnit::Cell => self.diagnostics.cells += 1,
            RayWorkUnit::Candidate => self.diagnostics.candidates += 1,
            RayWorkUnit::Intersection => self.diagnostics.intersections += 1,
        }
        true
    }
}

/// Bounded admission/continuation errors, without retained owner inventories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayWorkError {
    Limits,
    Shape,
    Owner,
    Overflow,
    StaleProposal,
}
impl Display for RayWorkError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid ray work: {self:?}")
    }
}
impl std::error::Error for RayWorkError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch(step: u64) -> CalculationBatchKey {
        CalculationBatchKey::new(1, step, 1)
    }

    #[test]
    fn admission_accounts_for_every_simultaneously_due_observation() {
        assert!(RayWorkLimits::new(128, 512, 65_536, 512).is_ok());
        for invalid in [
            (0, 512, 65_536, 512),
            (513, 512, 65_536, 512),
            (129, 512, 65_536, 512),
            (1, 512, 65_536, 513),
            (1, 512, 65_536, 0),
        ] {
            assert_eq!(
                RayWorkLimits::new(invalid.0, invalid.1, invalid.2, invalid.3),
                Err(RayWorkError::Limits)
            );
        }
    }

    #[test]
    fn continuous_saturation_keeps_all_classes_within_one_quantum() {
        let mut owners: Vec<_> = (0..300).map(RayObservationOwner::Evolved).collect();
        owners.extend((0..30).map(RayObservationOwner::Baseline));
        owners.extend((1..=182).map(|id| RayObservationOwner::Brain(BrainHandle { id, epoch: 1 })));
        let limits = RayWorkLimits::new(16, 512, 65_535, 512).unwrap();
        let mut cursor = RayQuotaCursor::default();
        let mut grants = vec![0; owners.len()];
        let mut totals = vec![0u64; owners.len()];
        for step in 1..=1024 {
            let proposal =
                prepare_ray_quotas(batch(step), limits, cursor, &owners, &mut grants).unwrap();
            assert_eq!(proposal.granted(), 65_535);
            assert_eq!(grants.iter().sum::<u32>(), 65_535);
            assert!(grants.iter().all(|grant| (127..=128).contains(grant)));
            for (total, grant) in totals.iter_mut().zip(&grants) {
                *total += u64::from(*grant);
            }
            assert!(totals.iter().max().unwrap() - totals.iter().min().unwrap() <= 1);
            proposal.commit(batch(step), &mut cursor).unwrap();
        }
    }

    #[test]
    fn capped_unused_budget_is_not_redistributed_and_idle_does_not_rotate() {
        let limits = RayWorkLimits::new(1, 128, 65_536, 512).unwrap();
        let owners = [
            RayObservationOwner::Evolved(9),
            RayObservationOwner::Baseline(0),
        ];
        let mut grants = [0; 2];
        let cursor = RayQuotaCursor::default();
        let proposal = prepare_ray_quotas(batch(1), limits, cursor, &owners, &mut grants).unwrap();
        assert_eq!(grants, [128, 128]);
        assert_eq!(proposal.granted(), 256);
        let mut idle = cursor;
        prepare_ray_quotas(batch(2), limits, cursor, &[], &mut [])
            .unwrap()
            .commit(batch(2), &mut idle)
            .unwrap();
        assert_eq!(idle, cursor);
    }

    #[test]
    fn all_due_sizes_obey_batch_cap_and_equal_grants() {
        let limits = RayWorkLimits::new(128, 1024, 65_536, 512).unwrap();
        for count in 1..=512 {
            let owners: Vec<_> = (0..count)
                .map(|slot| {
                    if slot < 300 {
                        RayObservationOwner::Evolved(slot as u32)
                    } else {
                        RayObservationOwner::Brain(BrainHandle {
                            id: slot as u64,
                            epoch: 1,
                        })
                    }
                })
                .collect();
            let mut grants = vec![0; count];
            let proposal = prepare_ray_quotas(
                batch(1),
                limits,
                RayQuotaCursor::default(),
                &owners,
                &mut grants,
            )
            .unwrap();
            assert!(proposal.granted() <= limits.batch());
            assert!(grants
                .iter()
                .all(|grant| (limits.minimum()..=limits.cap()).contains(grant)));
            assert!(grants.iter().max().unwrap() - grants.iter().min().unwrap() <= 1);
        }
    }

    #[test]
    fn failed_or_stale_transactions_preserve_the_cursor_and_output() {
        let limits = RayWorkLimits::new(1, 10, 11, 4).unwrap();
        let owners = [
            RayObservationOwner::Evolved(0),
            RayObservationOwner::Evolved(1),
        ];
        let cursor = RayQuotaCursor::default();
        let mut grants = [99, 99];
        assert_eq!(
            prepare_ray_quotas(
                batch(1),
                limits,
                cursor,
                &[owners[1], owners[0]],
                &mut grants
            ),
            Err(RayWorkError::Owner)
        );
        assert_eq!(grants, [99, 99]);
        let proposal = prepare_ray_quotas(batch(1), limits, cursor, &owners, &mut grants).unwrap();
        let mut published = cursor;
        assert_eq!(
            proposal.commit(batch(2), &mut published),
            Err(RayWorkError::StaleProposal)
        );
        assert_eq!(published, cursor);
        proposal.commit(batch(1), &mut published).unwrap();
        let saved = published;
        assert_eq!(
            proposal.commit(batch(1), &mut published),
            Err(RayWorkError::StaleProposal)
        );
        assert_eq!(published, saved);
        let exhausted = RayQuotaCursor {
            revision: u64::MAX,
            ..cursor
        };
        assert_eq!(
            prepare_ray_quotas(batch(2), limits, exhausted, &owners, &mut grants),
            Err(RayWorkError::Overflow)
        );
        assert_eq!(grants, [6, 5]);
    }

    #[test]
    fn disappearing_next_owner_resumes_at_its_canonical_successor() {
        let limits = RayWorkLimits::new(1, 8, 5, 4).unwrap();
        let owners = [
            RayObservationOwner::Evolved(0),
            RayObservationOwner::Evolved(2),
        ];
        let mut cursor = RayQuotaCursor {
            next_owner: Some(RayObservationOwner::Evolved(1)),
            revision: 8,
        };
        let mut grants = [0; 2];
        let proposal = prepare_ray_quotas(batch(1), limits, cursor, &owners, &mut grants).unwrap();
        assert_eq!(grants, [2, 3]);
        proposal.commit(batch(1), &mut cursor).unwrap();
        assert_eq!(cursor.next_owner(), Some(owners[0]));
    }

    #[test]
    fn physical_respawn_ids_do_not_change_canonical_observation_ownership() {
        use crate::engine::state::{BodyRange, WorldPoint};
        let origin = WorldPoint { x: 0.0, y: 0.0 };
        let mut snake = SnakeState {
            id: 7,
            frame_v1_id: 7,
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(2),
            brain: Some(BrainHandle { id: 8, epoch: 3 }),
            baseline_slot: None,
            baseline_strategy: None,
            position: origin,
            previous_position: origin,
            direction: 0.0,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 5.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.06,
            delivered_observation_points: 0.0,
            observation_delivered: false,
            completed_step_heading_delta: 0.0,
            completed_step_speed_delta: 0.0,
            completed_step_applied_turn: 0.0,
            body: BodyRange { start: 0, len: 5 },
            skin: 0,
        };
        assert_eq!(
            RayObservationOwner::of(&snake).unwrap(),
            RayObservationOwner::Evolved(2)
        );
        snake.id = (1u64 << 60) + 9;
        snake.frame_v1_id = 900;
        assert_eq!(
            RayObservationOwner::of(&snake).unwrap(),
            RayObservationOwner::Evolved(2)
        );
        snake.kind = SnakeKind::Baseline;
        snake.population_slot = None;
        snake.baseline_slot = Some(4);
        assert_eq!(
            RayObservationOwner::of(&snake).unwrap(),
            RayObservationOwner::Baseline(4)
        );
        snake.id += 1;
        assert_eq!(
            RayObservationOwner::of(&snake).unwrap(),
            RayObservationOwner::Baseline(4)
        );
        snake.baseline_slot = None;
        for kind in [SnakeKind::External, SnakeKind::Resurrected] {
            snake.kind = kind;
            assert_eq!(
                RayObservationOwner::of(&snake).unwrap(),
                RayObservationOwner::Brain(snake.brain.unwrap())
            );
        }
        snake.brain = None;
        assert_eq!(RayObservationOwner::of(&snake), Err(RayWorkError::Owner));
    }

    #[test]
    fn every_operation_is_charged_before_execution() {
        let mut meter = RayWorkMeter::new(3);
        assert!(meter.charge(RayWorkUnit::Cell));
        assert!(meter.charge(RayWorkUnit::Candidate));
        assert!(meter.charge(RayWorkUnit::Intersection));
        assert_eq!(meter.remaining(), 0);
        assert!(!meter.charge(RayWorkUnit::Intersection));
        assert_eq!(
            meter.diagnostics(),
            RayWorkDiagnostics {
                cells: 1,
                candidates: 1,
                intersections: 1,
                saturated: true
            }
        );
    }
}
