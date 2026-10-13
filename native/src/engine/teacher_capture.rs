//! Reusable pending teacher rows copied from actual baseline-v2 evaluation.
//! These are proposals, not transport samples. Only a complete-step publication
//! join may attach post-step outcomes and assign committed stream sequences.

use super::baseline_control::BaselineLifeStage;
use super::baseline_v2::BaselineV2Action;
use super::physics::PhysicsStepKey;
use super::state::BaselineStrategyState;

/// Compact pending decision; observation floats live in one retained flat buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedTeacherDecision {
    pub baseline_slot: u32,
    pub physical_snake_id: u64,
    pub applied_turn: f32,
    pub boost_request: bool,
    pub strategy: BaselineStrategyState,
    pub life_stage: BaselineLifeStage,
    pub unsafe_route: bool,
    pub urgent_sectors: u32,
    pub strategy_transition: bool,
}

/// Borrowed exact rows from a successfully prepared control boundary.
#[derive(Clone, Copy, Debug)]
pub struct PreparedTeacherBoundary<'a> {
    pub key: PhysicsStepKey,
    pub input_size: usize,
    pub decisions: &'a [PreparedTeacherDecision],
    observations: &'a [f32],
}
impl PreparedTeacherBoundary<'_> {
    /// Return the already evaluated row, without resampling or consuming a marker.
    pub fn observation(&self, index: usize) -> Option<&[f32]> {
        if index >= self.decisions.len() {
            return None;
        }
        let start = index * self.input_size;
        self.observations.get(start..start + self.input_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::baseline_v2::BaselineV2State;

    fn action() -> BaselineV2Action {
        BaselineV2Action {
            turn: 0.25,
            boost: true,
            sector: 0,
            life_stage: BaselineLifeStage::Medium,
            normalized_score: 0.1,
            unsafe_route: false,
            urgent_sectors: 1,
            next_state: BaselineV2State {
                strategy: BaselineStrategyState::Boost,
                ..BaselineV2State::default()
            },
            next_behavior_rng_word: 17,
            uniform_draws: 0,
        }
    }

    #[test]
    fn selected_exact_rows_reuse_capacity_and_preserve_signed_zero() {
        let mut workspace = TeacherBoundaryWorkspace::default();
        workspace.configure(&[1, 7], 3).unwrap();
        let observations = workspace.observations.as_ptr();
        let decisions = workspace.decisions.as_ptr();
        for _ in 0..100 {
            workspace.clear();
            workspace
                .stage(0, 11, &[0.0; 3], action(), BaselineStrategyState::Roam)
                .unwrap();
            workspace
                .stage(
                    7,
                    77,
                    &[-0.0, 0.5, -1.0],
                    action(),
                    BaselineStrategyState::Roam,
                )
                .unwrap();
            workspace
                .stage(1, 21, &[1.0; 3], action(), BaselineStrategyState::Boost)
                .unwrap();
            assert_eq!(workspace.decisions.len(), 2);
            assert!(workspace.decisions[0].strategy_transition);
            assert!(!workspace.decisions[1].strategy_transition);
            assert_eq!(workspace.observations[0].to_bits(), (-0.0_f32).to_bits());
            assert_eq!(workspace.observations.as_ptr(), observations);
            assert_eq!(workspace.decisions.as_ptr(), decisions);
        }
    }

    #[test]
    fn malformed_selection_cannot_replace_old_capture_and_duplicates_reject() {
        let mut workspace = TeacherBoundaryWorkspace::default();
        workspace.configure(&[1], 3).unwrap();
        for slots in [&[1, 1][..], &[2, 1][..], &[120][..]] {
            assert!(workspace.configure(slots, 3).is_err());
            assert_eq!(workspace.slots, [1]);
        }
        assert!(workspace.configure(&[1], 505).is_err());
        assert!(workspace
            .stage(1, 21, &[0.0; 2], action(), BaselineStrategyState::Roam)
            .is_err());
        workspace
            .stage(1, 21, &[0.0; 3], action(), BaselineStrategyState::Roam)
            .unwrap();
        assert!(workspace
            .stage(1, 21, &[0.0; 3], action(), BaselineStrategyState::Roam)
            .is_err());
        workspace.configure(&[], 3).unwrap();
        assert!(!workspace.enabled());
        assert!(workspace.decisions.is_empty() && workspace.observations.is_empty());
    }
}

/// Cold-admitted pending storage; disabled capture has no row allocations.
#[derive(Debug, Default)]
pub(crate) struct TeacherBoundaryWorkspace {
    slots: Vec<u32>,
    input_size: usize,
    decisions: Vec<PreparedTeacherDecision>,
    observations: Vec<f32>,
}
impl TeacherBoundaryWorkspace {
    /// Allocate complete selected-slot storage before changing the existing selection.
    pub fn configure(&mut self, slots: &[u32], input_size: usize) -> Result<(), &'static str> {
        if slots.len() > 120
            || slots.iter().any(|slot| *slot >= 120)
            || slots.windows(2).any(|pair| pair[0] >= pair[1])
            || !(1..=super::teacher_samples::MAX_TEACHER_INPUTS).contains(&input_size)
        {
            return Err("invalid teacher capture selection");
        }
        let floats = slots
            .len()
            .checked_mul(input_size)
            .ok_or("teacher pending size overflow")?;
        let mut replacement = Self {
            input_size,
            ..Self::default()
        };
        replacement
            .slots
            .try_reserve_exact(slots.len())
            .map_err(|_| "teacher slot allocation")?;
        replacement.slots.extend_from_slice(slots);
        replacement
            .decisions
            .try_reserve_exact(slots.len())
            .map_err(|_| "teacher decision allocation")?;
        replacement
            .observations
            .try_reserve_exact(floats)
            .map_err(|_| "teacher observation allocation")?;
        *self = replacement;
        Ok(())
    }

    pub fn enabled(&self) -> bool {
        !self.slots.is_empty()
    }

    /// Every attempt invalidates old proposals without dropping backing capacities.
    pub fn clear(&mut self) {
        self.decisions.clear();
        self.observations.clear();
    }

    /// Append one selected actual policy decision within the cold reserved bounds.
    pub fn stage(
        &mut self,
        slot: u32,
        snake_id: u64,
        observation: &[f32],
        action: BaselineV2Action,
        before_strategy: BaselineStrategyState,
    ) -> Result<(), &'static str> {
        if self.slots.binary_search(&slot).is_err() {
            return Ok(());
        }
        if observation.len() != self.input_size
            || self.decisions.len() >= self.slots.len()
            || self
                .decisions
                .iter()
                .any(|decision| decision.baseline_slot == slot)
        {
            return Err("teacher pending row ownership/width mismatch");
        }
        // Complete capacity was reserved cold; push/extend cannot allocate here.
        self.decisions.push(PreparedTeacherDecision {
            baseline_slot: slot,
            physical_snake_id: snake_id,
            applied_turn: action.turn,
            boost_request: action.boost,
            strategy: action.next_state.strategy,
            life_stage: action.life_stage,
            unsafe_route: action.unsafe_route,
            urgent_sectors: action.urgent_sectors,
            strategy_transition: before_strategy != action.next_state.strategy,
        });
        self.observations.extend_from_slice(observation);
        Ok(())
    }

    pub fn prepared(&self, key: PhysicsStepKey) -> PreparedTeacherBoundary<'_> {
        PreparedTeacherBoundary {
            key,
            input_size: self.input_size,
            decisions: &self.decisions,
            observations: &self.observations,
        }
    }
}
