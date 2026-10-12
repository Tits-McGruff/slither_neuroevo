//! Fixed-width generation experiment history (v1, 64 words) and HoF assessment
//! extension (v2, 32 words). Words are little-endian u64; named floating fields
//! retain Float64 bits. These are boundary summaries, never running snapshots.
use super::evolution::{assessment_fitness_breakdown, EvolutionConfig, EvolvedFitnessAssessment};
use super::evolved_lives::*;
use super::generation::GenerationCompletionRecord;
use super::state::{SnakeKind, SnakeState, WorldState};

pub const HISTORY_WORDS: usize = 64;
pub const HOF_WORDS: usize = 32;
pub const EXPERIMENT_RECORD_BYTES: usize = (HISTORY_WORDS + HOF_WORDS) * 8;

/// Exact fixed-size records. The legacy 56-byte summary/120-byte index stay separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationExperimentRecords {
    pub history: [u64; HISTORY_WORDS],
    pub hall_of_fame: [u64; HOF_WORDS],
}

/// Borrowed, already committed terminal inputs; no allocation grows with life count.
pub(crate) struct ExperimentRecordInputs<'a> {
    pub generation: u64,
    pub config_revision: u64,
    pub completion: GenerationCompletionRecord,
    pub world: &'a WorldState,
    pub lives: &'a EvolvedGenerationState,
    pub policy: EvolvedExperimentConfig,
    pub cumulative_gross: bool,
    pub evolution: EvolutionConfig,
    pub winner_slot: u32,
}

/// Dense population-order floating reduction; extrema start at zero for nonnegative data.
#[derive(Clone, Copy, Default)]
struct Statistic {
    sum: f64,
    maximum: f64,
}
impl Statistic {
    fn add(&mut self, value: f64) -> Result<(), String> {
        finite_nonnegative(value)?;
        self.sum += value;
        self.maximum = self.maximum.max(value);
        finite_nonnegative(self.sum)
    }
    fn words(self, count: usize) -> [u64; 2] {
        [(self.sum / count as f64).to_bits(), self.maximum.to_bits()]
    }
}

impl GenerationExperimentRecords {
    /// Build all summaries once at the terminal boundary, using at most 300 median counters.
    pub(crate) fn build(input: ExperimentRecordInputs<'_>) -> Result<Self, String> {
        let ExperimentRecordInputs {
            generation,
            config_revision,
            completion,
            world,
            lives,
            policy,
            cumulative_gross,
            evolution,
            winner_slot,
        } = input;
        policy.validate().map_err(|e| e.to_string())?;
        evolution.validate().map_err(|e| e.to_string())?;
        if policy.version != 1 || lives.slots.is_empty() || lives.slots.len() > 300 {
            return Err("invalid experiment population/version".into());
        }
        if completion.configured_duration_f64_bits
            != policy.objective.configured_duration_seconds.to_bits()
        {
            return Err("experiment duration policy mismatch".into());
        }
        let count = lives.slots.len();
        lives.validate(world, count).map_err(|e| e.to_string())?;
        let use_assessments = policy.uses_life_assessments();
        let maximum = lives.slots.iter().try_fold(0.0_f64, |maximum, slot| {
            let snake = physical(world, slot)?;
            Ok::<_, String>(
                maximum.max(
                    selected_assessment(slot, snake, policy, evolution)
                        .assessment
                        .points,
                ),
            )
        })?;
        let mut record = Self {
            history: [0; HISTORY_WORDS],
            hall_of_fame: [0; HOF_WORDS],
        };
        let h = &mut record.history;
        h[0] = 1;
        h[1] = policy_bits(policy);
        h[2] = generation;
        h[3] = completion.completed_step;
        h[4] = match completion.reason {
            super::running_step::GenerationTransitionReason::Manual => 0,
            super::running_step::GenerationTransitionReason::Duration => 1,
            super::running_step::GenerationTransitionReason::EarlyAliveCount => 2,
        };
        h[5] = completion.actual_elapsed_f64_bits;
        h[6] = completion.configured_duration_f64_bits;
        h[7] = count as u64;
        h[60] = u64::from(cumulative_gross);
        h[61] = config_revision;
        h[62] = u64::from(use_assessments);
        let mut scores = [Statistic::default(); 7];
        let mut components = [Statistic::default(); 7];
        let mut deaths = [0_u64; 300];
        let mut pending = 0.0;
        let mut economic = [0.0; 3];
        let mut corpse_values = [0.0; 3];
        for (index, slot) in lives.slots.iter().enumerate() {
            let snake = physical(world, slot)?;
            deaths[index] = slot.deaths;
            add_count(&mut h[8], slot.lives_started)?;
            add_count(&mut h[9], slot.deaths)?;
            add_count(&mut h[10], slot.successful_respawns)?;
            h[13] = h[13].max(slot.deaths);
            for (offset, &value) in slot.cause_counts.iter().enumerate() {
                add_count(&mut h[14 + offset], value)?;
            }
            pending += slot.pending_seconds;
            add_count(
                &mut h[19],
                u64::from(slot.status == EvolvedLifeStatus::Exhausted),
            )?;
            add_count(&mut h[63], slot.ledger.operations)?;
            let entry = selected_assessment(slot, snake, policy, evolution);
            let cumulative = if cumulative_gross {
                slot.ledger.generation_gross_earned
            } else {
                slot.ledger.generation_net_earned
            };
            for (stat, value) in scores.iter_mut().zip([
                snake.points,
                slot.ledger.life_net_points(),
                slot.ledger.generation_gross_earned,
                slot.ledger.generation_net_earned,
                cumulative,
                slot.best_point_life.peak_points,
                entry.assessment.points,
            ]) {
                stat.add(value)?;
            }
            for (stat, value) in components.iter_mut().zip(
                assessment_fitness_breakdown(entry, maximum, evolution, policy.objective)
                    .components,
            ) {
                stat.add(value)?;
            }
            for (total, value) in economic.iter_mut().zip([
                slot.ledger.generation_gross_earned,
                slot.ledger.generation_boost_spent,
                slot.ledger.generation_net_earned,
            ]) {
                *total += value;
            }
            add_count(&mut h[38], slot.corpse_emitted_count)?;
            add_count(&mut h[39], slot.corpse_discarded_count)?;
            corpse_values[1] += slot.corpse_emitted_value;
            corpse_values[2] += slot.corpse_discarded_value;
        }
        deaths[..count].sort_unstable();
        h[11] = (h[9] as f64 / count as f64).to_bits();
        h[12] = if count % 2 == 1 {
            deaths[count / 2] as f64
        } else {
            (deaths[count / 2 - 1] as f64 + deaths[count / 2] as f64) / 2.0
        }
        .to_bits();
        finite_nonnegative(pending)?;
        h[18] = pending.to_bits();
        for (index, stat) in scores.into_iter().enumerate() {
            h[20 + index * 2..22 + index * 2].copy_from_slice(&stat.words(count));
        }
        for (index, value) in economic.into_iter().enumerate() {
            finite_nonnegative(value)?;
            h[34 + index] = value.to_bits();
        }
        h[37] = h[38]
            .checked_add(h[39])
            .ok_or("corpse attempt count overflow")?;
        corpse_values[0] = corpse_values[1] + corpse_values[2];
        for (index, value) in corpse_values.into_iter().enumerate() {
            finite_nonnegative(value)?;
            h[40 + index] = value.to_bits();
        }
        for (index, stat) in components.into_iter().enumerate() {
            h[43 + index * 2..45 + index * 2].copy_from_slice(&stat.words(count));
        }
        let winner = lives
            .slots
            .get(winner_slot as usize)
            .ok_or("winner outside population")?;
        let snake = physical(world, winner)?;
        let assessment = selected_assessment(winner, snake, policy, evolution).assessment;
        let (point_ordinal, selected_peak, display_length, length_kind) = match policy.points {
            PointAssessmentMode::Reset => (
                winner.life_ordinal,
                winner.current.peak_length,
                winner.current.final_length,
                0,
            ),
            PointAssessmentMode::CumulativeNet | PointAssessmentMode::CumulativeGross => {
                (0, 0, winner.generation_peak_length, 1)
            }
            PointAssessmentMode::BestLife => (
                winner.best_point_life.ordinal,
                winner.best_point_life.peak_length,
                winner.best_point_life.peak_length,
                2,
            ),
        };
        h[57] = u64::from(winner_slot);
        h[58] = assessment.points.to_bits();
        h[59] = point_ordinal;
        let f = &mut record.hall_of_fame;
        f[0] = 2;
        f[1] = generation;
        f[2] = u64::from(winner_slot);
        f[3] = snake.id;
        f[4] = point_code(policy.points);
        f[5] = if use_assessments {
            nonpoint_code(policy.nonpoints)
        } else {
            2
        };
        f[6] = u64::from(cumulative_gross);
        f[7] = point_ordinal;
        f[8] = assessment.points.to_bits();
        f[9] = assessment.alive_seconds.to_bits();
        f[10] = assessment.food.to_bits();
        f[11] = assessment.kills;
        f[12] = assessment.retained_growth.to_bits();
        f[13] = assessment.peak_length as u64;
        for (index, value) in [
            winner.ledger.life_net_points(),
            winner.ledger.life_gross_earned,
            winner.ledger.life_boost_spent,
            winner.ledger.generation_gross_earned,
            winner.ledger.generation_net_earned,
            winner.ledger.generation_boost_spent,
            snake.points,
        ]
        .into_iter()
        .enumerate()
        {
            f[14 + index] = value.to_bits();
        }
        f[21] = winner.lives_started;
        f[22] = winner.deaths;
        f[23] = winner.successful_respawns;
        f[24] = winner.current.final_length as u64;
        f[25] = winner.current.peak_length as u64;
        f[26] = selected_peak as u64;
        f[27] = winner.generation_peak_length as u64;
        f[28] = display_length as u64;
        f[29] = length_kind;
        f[30] = winner.life_ordinal;
        f[31] = h[1];
        record.validate()?;
        Ok(record)
    }

    /// Validate untrusted fixed records before any import or durable adoption.
    pub fn validate(&self) -> Result<(), String> {
        let h = &self.history;
        let f = &self.hall_of_fame;
        if h[0] != 1
            || f[0] != 2
            || h[2] == 0
            || h[3] == 0
            || h[4] > 2
            || !(1..=300).contains(&h[7])
            || h[61] == 0
            || h[60] > 1
            || h[62] > 1
            || h[1] > 0xffff
            || ((h[1] >> 2) & 3) > 2
            || ((h[1] >> 10) & 3) > 2
            || ((h[1] >> 13) & 3) > 2
        {
            return Err("unsupported experiment header/policy".into());
        }
        let elapsed = f64::from_bits(h[5]);
        let duration = f64::from_bits(h[6]);
        if (h[4] == 1 && elapsed < duration)
            || !elapsed.is_finite()
            || elapsed <= 0.0
            || !duration.is_finite()
            || !(8.0..=604800.0).contains(&duration)
        {
            return Err("invalid experiment times".into());
        }
        if h[7].checked_add(h[10]) != Some(h[8])
            || h[9] > h[8]
            || h[8] - h[9] > h[7]
            || h[13] > h[9]
            || h[19] > h[7]
            || h[38].checked_add(h[39]) != Some(h[37])
            || h[14..18]
                .iter()
                .try_fold(0_u64, |sum, &value| sum.checked_add(value))
                .ok_or("cause overflow")?
                > h[9]
        {
            return Err("invalid experiment count identities".into());
        }
        for index in [11, 12, 18, 34, 35, 36, 40, 41, 42, 58] {
            finite_nonnegative(f64::from_bits(h[index]))?;
        }
        let mean = f64::from_bits(h[11]);
        if !near(mean, h[9] as f64 / h[7] as f64) || f64::from_bits(h[12]) > h[13] as f64 {
            return Err("invalid death summaries".into());
        }
        for start in (20..34).step_by(2).chain((43..57).step_by(2)) {
            let mean = f64::from_bits(h[start]);
            let max = f64::from_bits(h[start + 1]);
            finite_nonnegative(mean)?;
            finite_nonnegative(max)?;
            if mean > max && !near(mean, max) {
                return Err("experiment mean exceeds maximum".into());
            }
        }
        if !near(
            f64::from_bits(h[40]),
            f64::from_bits(h[41]) + f64::from_bits(h[42]),
        ) {
            return Err("invalid corpse value identity".into());
        }
        if f[1] != h[2]
            || f[2] != h[57]
            || f[2] >= h[7]
            || f[3] == 0
            || f[4] != (h[1] & 3)
            || f[5] != if h[62] == 1 { (h[1] >> 2) & 3 } else { 2 }
            || f[6] != h[60]
            || f[7] != h[59]
            || f[8] != h[58]
            || f[31] != h[1]
            || f[21] == 0
            || f[22] > f[21]
            || f[23].checked_add(1) != Some(f[21])
            || f[21] - f[22] > 1
            || f[30] != f[21]
            || f[7] > f[21]
            || f[24] == 0
            || f[25] < f[24]
            || f[27] < f[25]
            || f[27] < f[26]
            || f[13] == 0
        {
            return Err("invalid Hall-of-Fame experiment identity".into());
        }
        for index in [8, 9, 10, 12, 14, 15, 16, 17, 18, 19, 20] {
            finite_nonnegative(f64::from_bits(f[index]))?;
        }
        let (length, kind) = match f[4] {
            0 => (f[24], 0),
            1 | 2 => (f[27], 1),
            3 => (f[26], 2),
            _ => return Err("invalid point method".into()),
        };
        if (f[4] == 1 && h[60] != 0)
            || (f[4] == 2 && h[60] != 1)
            || (f[4] == 0 && f[7] != f[30])
            || f[13] > f[27]
        {
            return Err("inconsistent assessment semantics".into());
        }
        if f[28] != length
            || f[29] != kind
            || (kind == 1 && (f[7] != 0 || f[26] != 0))
            || (kind != 1 && (f[7] == 0 || f[26] == 0))
        {
            return Err("invalid display-length semantics".into());
        }
        Ok(())
    }

    /// Encode exact bytes with no host layout or padding dependence.
    #[must_use]
    pub fn encode(&self) -> [u8; EXPERIMENT_RECORD_BYTES] {
        let mut bytes = [0; EXPERIMENT_RECORD_BYTES];
        for (word, chunk) in self
            .history
            .iter()
            .chain(&self.hall_of_fame)
            .zip(bytes.as_chunks_mut::<8>().0.iter_mut())
        {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
    /// Require the exact record length and validate every identity/value field.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != EXPERIMENT_RECORD_BYTES {
            return Err("invalid experiment record size".into());
        }
        let mut result = Self {
            history: [0; HISTORY_WORDS],
            hall_of_fame: [0; HOF_WORDS],
        };
        for (word, chunk) in result
            .history
            .iter_mut()
            .chain(&mut result.hall_of_fame)
            .zip(bytes.as_chunks::<8>().0.iter())
        {
            *word = u64::from_le_bytes(*chunk);
        }
        result.validate()?;
        Ok(result)
    }
}

fn physical<'a>(
    world: &'a WorldState,
    slot: &EvolvedSlotRuntime,
) -> Result<&'a SnakeState, String> {
    world
        .snakes
        .iter()
        .find(|snake| {
            snake.kind == SnakeKind::Evolved
                && snake.population_slot == Some(slot.population_slot)
                && snake.id == slot.physical_id
        })
        .ok_or_else(|| "experiment physical owner mismatch".into())
}
fn selected_assessment(
    slot: &EvolvedSlotRuntime,
    snake: &SnakeState,
    policy: EvolvedExperimentConfig,
    config: EvolutionConfig,
) -> EvolvedFitnessAssessment {
    let assessment = if policy.uses_life_assessments() {
        slot.assessment(policy.points, policy.nonpoints)
    } else {
        EvolvedAssessment {
            points: snake.points,
            alive_seconds: snake.age_seconds,
            food: snake.food,
            kills: snake.kills,
            retained_growth: snake.body.len.saturating_sub(config.snake_start_length) as f64,
            peak_length: snake.body.len,
        }
    };
    EvolvedFitnessAssessment {
        assessment,
        deaths: slot.deaths,
        successful_respawns: slot.successful_respawns,
    }
}
fn point_code(value: PointAssessmentMode) -> u64 {
    match value {
        PointAssessmentMode::Reset => 0,
        PointAssessmentMode::CumulativeNet => 1,
        PointAssessmentMode::CumulativeGross => 2,
        PointAssessmentMode::BestLife => 3,
    }
}
fn nonpoint_code(value: NonPointAggregation) -> u64 {
    match value {
        NonPointAggregation::GenerationTotals => 0,
        NonPointAggregation::FollowScore => 1,
        NonPointAggregation::LegacyTerminal => 2,
    }
}
fn policy_bits(p: EvolvedExperimentConfig) -> u64 {
    point_code(p.points)
        | (nonpoint_code(p.nonpoints) << 2)
        | (u64::from(p.objective.normalize_to_240_seconds) << 4)
        | (u64::from(p.objective.penalize_successful_respawns) << 5)
        | (u64::from(p.carried_wallet) << 6)
        | (u64::from(p.enabled) << 7)
        | (u64::from(p.recurrent_state == RecurrentLifeMode::Preserve) << 8)
        | (u64::from(p.rng_mode == RespawnRngMode::World) << 9)
        | (match p.corpse_mode {
            EvolvedCorpseMode::Normal => 0,
            EvolvedCorpseMode::None => 1,
            EvolvedCorpseMode::SubtractReplacementMass => 2,
        } << 10)
        | (u64::from(p.pellet_overflow == PelletOverflowMode::DiscardNewExcess) << 12)
        | (match p.early_completion {
            EarlyCompletionMode::Automatic => 0,
            EarlyCompletionMode::PhysicalAlive => 1,
            EarlyCompletionMode::DurationOnly => 2,
        } << 13)
        | (u64::from(p.delay_mode == RespawnDelayMode::Uniform) << 15)
}
fn finite_nonnegative(value: f64) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err("nonfinite/negative experiment value".into())
    }
}
fn add_count(total: &mut u64, value: u64) -> Result<(), String> {
    *total = total
        .checked_add(value)
        .ok_or("experiment counter overflow")?;
    Ok(())
}
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 128.0 * f64::EPSILON * a.abs().max(b.abs()).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> GenerationExperimentRecords {
        let mut r = GenerationExperimentRecords {
            history: [0; HISTORY_WORDS],
            hall_of_fame: [0; HOF_WORDS],
        };
        let h = &mut r.history;
        h[0] = 1;
        h[2] = 1;
        h[3] = 1;
        h[5] = 1.0_f64.to_bits();
        h[6] = 240.0_f64.to_bits();
        h[7] = 1;
        h[8] = 1;
        h[59] = 1;
        h[61] = 1;
        let f = &mut r.hall_of_fame;
        f[0] = 2;
        f[1] = 1;
        f[3] = 1;
        f[5] = 2;
        f[7] = 1;
        f[13] = 5;
        f[21] = 1;
        f[24] = 5;
        f[25] = 5;
        f[26] = 5;
        f[27] = 5;
        f[28] = 5;
        f[30] = 1;
        r
    }
    #[test]
    fn fixed_records_roundtrip_and_reject_truncation_unknown_versions_and_invalid_identities() {
        let r = fixture();
        r.validate().unwrap();
        let bytes = r.encode();
        assert_eq!(bytes.len(), 768);
        assert_eq!(GenerationExperimentRecords::decode(&bytes).unwrap(), r);
        assert!(GenerationExperimentRecords::decode(&bytes[..767]).is_err());
        for (index, value) in [
            (0, 2),
            (1, 1 << 20),
            (3, 0),
            (7, 301),
            (8, 0),
            (9, 2),
            (11, f64::NAN.to_bits()),
            (12, 1.0_f64.to_bits()),
            (20, (-1.0_f64).to_bits()),
            (21, f64::INFINITY.to_bits()),
            (38, u64::MAX),
            (57, 1),
        ] {
            let mut bad = r;
            bad.history[index] = value;
            assert!(bad.validate().is_err(), "history word {index}");
        }
        for (index, value) in [
            (0, 1),
            (1, 2),
            (3, 0),
            (4, 3),
            (7, 0),
            (8, f64::NAN.to_bits()),
            (21, 2),
            (24, 0),
            (26, 0),
            (28, 4),
            (29, 1),
            (31, 1),
        ] {
            let mut bad = r;
            bad.hall_of_fame[index] = value;
            assert!(bad.validate().is_err(), "HoF word {index}");
        }
    }
}
