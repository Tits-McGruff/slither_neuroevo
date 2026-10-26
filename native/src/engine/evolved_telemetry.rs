//! Bounded diagnostics projected from committed population-life state.
//! No historical event vectors, new RNG draws or simulation transitions occur here.
use super::evolved_lives::{
    EvolvedGenerationState, EvolvedLifeError, EvolvedLifeStatus, EvolvedPlacementWork,
};

/// Constant-sized replaceable stats. Exact lifetime counters stay u64 at the bridge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvolvedLifecycleTelemetry {
    pub population: u16,
    pub alive: u16,
    pub waiting: u16,
    pub pending_placement: u16,
    pub exhausted: u16,
    pub dead_no_respawn: u16,
    pub lives: u64,
    pub deaths: u64,
    pub respawns: u64,
    pub failed_placements: u64,
    pub pending_seconds: f64,
    pub recent_event_count: u16,
    pub life_end_cursor: u64,
    pub placement: EvolvedPlacementWork,
}
impl EvolvedLifecycleTelemetry {
    /// One population-sized traversal at display cadence; no allocation depends on lives.
    pub fn capture(
        lives: &EvolvedGenerationState,
        completed_step: u64,
    ) -> Result<Self, EvolvedLifeError> {
        if lives.slots.len() > 300
            || lives.recent_life_ends.len() > super::evolved_lives::MAX_RECENT_LIFE_ENDS
        {
            return Err(EvolvedLifeError::Lifecycle);
        }
        lives.placement_work.validate()?;
        if lives.placement_work.step != 0 && lives.placement_work.step != completed_step {
            return Err(EvolvedLifeError::Lifecycle);
        }
        let mut result = Self {
            population: lives.slots.len() as u16,
            recent_event_count: lives.recent_life_ends.len() as u16,
            life_end_cursor: lives.life_end_count,
            placement: lives.placement_work,
            ..Default::default()
        };
        for owner in &lives.slots {
            match owner.status {
                EvolvedLifeStatus::Alive => result.alive += 1,
                EvolvedLifeStatus::Waiting => result.waiting += 1,
                EvolvedLifeStatus::PendingPlacement => result.pending_placement += 1,
                EvolvedLifeStatus::Exhausted => result.exhausted += 1,
                EvolvedLifeStatus::DeadNoRespawn => result.dead_no_respawn += 1,
            }
            result.lives = result
                .lives
                .checked_add(owner.lives_started)
                .ok_or(EvolvedLifeError::Overflow)?;
            result.deaths = result
                .deaths
                .checked_add(owner.deaths)
                .ok_or(EvolvedLifeError::Overflow)?;
            result.respawns = result
                .respawns
                .checked_add(owner.successful_respawns)
                .ok_or(EvolvedLifeError::Overflow)?;
            result.failed_placements = result
                .failed_placements
                .checked_add(owner.failed_placements)
                .ok_or(EvolvedLifeError::Overflow)?;
            result.pending_seconds += owner.pending_seconds;
        }
        if !result.pending_seconds.is_finite() || result.pending_seconds < 0.0 {
            return Err(EvolvedLifeError::Overflow);
        }
        Ok(result)
    }
}
