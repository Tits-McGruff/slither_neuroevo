//! Independent bounded shadow inference over exact committed teacher observations.

use super::brain_artifact::ValidatedBrainArtifact;
use super::calculation::CalculationScratch;
use super::inference::GraphExecutionPlan;
use super::teacher_capture::{PreparedTeacherDecision, PublishedTeacherBoundary};
use super::training_graph::TrainingLifeStateRule;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

pub const MAX_SHADOWS: usize = 4;
pub const SHADOW_WORKSPACE_BYTES: usize = 64 * 1024 * 1024;
const CANDIDATE_METADATA_HEADROOM: usize = 8 * 1024 * 1024;
const PRE_DEATH_ROWS: usize = 32;

/// Immutable native values shared by cold candidate registration and active diagnostics.
#[derive(Debug)]
pub struct ShadowCandidate {
    pub artifact_sha256: [u8; 32],
    pub sensor_sha256: [u8; 32],
    pub cadence: f64,
    pub life_rule: TrainingLifeStateRule,
    plan: GraphExecutionPlan,
    weights: Box<[f32]>,
    charged_bytes: usize,
}
impl PartialEq for ShadowCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.artifact_sha256 == other.artifact_sha256
    }
}

impl ShadowCandidate {
    #[cfg(test)]
    pub(crate) fn synthetic(
        graph: &super::graph::CompiledGraph,
        sensor_sha256: [u8; 32],
        cadence: f64,
    ) -> Arc<Self> {
        Arc::new(Self {
            artifact_sha256: [47; 32],
            sensor_sha256,
            cadence,
            life_rule: TrainingLifeStateRule::Preserve,
            plan: GraphExecutionPlan::build(graph).unwrap(),
            weights: vec![0.125; graph.total_parameters].into_boxed_slice(),
            charged_bytes: CANDIDATE_METADATA_HEADROOM + graph.total_parameters * 4,
        })
    }

    pub fn from_artifact(artifact: ValidatedBrainArtifact) -> Result<Arc<Self>, String> {
        let plan = GraphExecutionPlan::build(&artifact.graph).map_err(|error| error.to_string())?;
        let charged_bytes = artifact
            .weights
            .capacity()
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(CANDIDATE_METADATA_HEADROOM))
            .ok_or("shadow candidate byte overflow")?;
        Ok(Arc::new(Self {
            artifact_sha256: super::state::decode_sha256_identity(
                "shadow.artifact",
                &format!("sha256:{}", artifact.artifact_sha256),
            )
            .map_err(|e| e.to_string())?,
            sensor_sha256: super::state::decode_sha256_identity(
                "shadow.sensor",
                &format!("sha256:{}", artifact.canonical_manifest.sensor_sha256),
            )
            .map_err(|e| e.to_string())?,
            cadence: artifact.deployment.control_interval_seconds,
            life_rule: artifact.deployment.life_state_rule,
            plan,
            weights: artifact.weights.into_boxed_slice(),
            charged_bytes,
        }))
    }
}

/// Registry admission is cold; pinned commands/active shadows remain charged until released.
#[derive(Default)]
struct Registry {
    entries: VecDeque<Arc<ShadowCandidate>>,
    bytes: usize,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

pub fn register_candidate(candidate: Arc<ShadowCandidate>) -> Result<(), String> {
    let mut registry = REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        .map_err(|_| "shadow registry poisoned")?;
    registry.insert(candidate)
}

impl Registry {
    fn insert(&mut self, candidate: Arc<ShadowCandidate>) -> Result<(), String> {
        if candidate.charged_bytes > SHADOW_WORKSPACE_BYTES {
            return Err("shadow candidate registry admission".into());
        }
        if self
            .entries
            .iter()
            .any(|old| old.artifact_sha256 == candidate.artifact_sha256)
        {
            return Ok(());
        }
        while self.entries.len() >= 8
            || self.bytes + candidate.charged_bytes > SHADOW_WORKSPACE_BYTES
        {
            let index = self
                .entries
                .iter()
                .position(|entry| Arc::strong_count(entry) == 1)
                .ok_or("shadow registry full with pinned candidates")?;
            let old = self
                .entries
                .remove(index)
                .ok_or("shadow registry byte admission")?;
            self.bytes -= old.charged_bytes;
        }
        self.bytes += candidate.charged_bytes;
        self.entries.push_back(candidate);
        Ok(())
    }
}

pub fn registered_candidate(identity: [u8; 32]) -> Result<Arc<ShadowCandidate>, String> {
    REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        .map_err(|_| "shadow registry poisoned")?
        .entries
        .iter()
        .find(|entry| entry.artifact_sha256 == identity)
        .cloned()
        .ok_or_else(|| "shadow candidate unavailable; validate its stored artifact first".into())
}

/// Ordered activation is bound to one admitted capture/teacher and an immutable candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowActivationRequest {
    pub expected_run_id: String,
    pub expected_config_revision: u64,
    pub capture_epoch: [u8; 16],
    pub shadow_slot: u32,
    pub baseline_slot: u32,
    pub candidate: Arc<ShadowCandidate>,
}
impl ShadowActivationRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.expected_run_id.is_empty()
            || self.expected_run_id.len() > 128
            || self.expected_run_id.contains('\0')
            || self.expected_config_revision == 0
            || self.capture_epoch == [0; 16]
            || self.shadow_slot as usize >= MAX_SHADOWS
            || self.baseline_slot >= 120
        {
            return Err("invalid shadow activation owner/slot");
        }
        Ok(())
    }
}

/// Raw unweighted metric sufficient statistics, never controlling fitness or actions.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowMetricTotals {
    pub samples: u64,
    pub turn_absolute_error: f64,
    pub turn_squared_error: f64,
    pub boost_disagreements: u64,
}
impl ShadowMetricTotals {
    fn add(&mut self, turn: f64, disagreement: bool) {
        self.samples += 1;
        self.turn_absolute_error += turn.abs();
        self.turn_squared_error += turn * turn;
        self.boost_disagreements += u64::from(disagreement);
    }
    fn merge(&mut self, other: Self) {
        self.samples += other.samples;
        self.turn_absolute_error += other.turn_absolute_error;
        self.turn_squared_error += other.turn_squared_error;
        self.boost_disagreements += other.boost_disagreements;
    }
}

/// Fixed small snapshot; retained histories cannot grow with deaths or sample count.
#[derive(Clone, Copy, Debug)]
pub struct ShadowSnapshot {
    pub artifact_sha256: [u8; 32],
    pub capture_epoch: [u8; 16],
    pub baseline_slot: u32,
    pub activation_sequence: u64,
    pub activation_step: u64,
    pub completed_step: u64,
    pub generation: u64,
    pub physical_snake_id: u64,
    pub enabled: bool,
    pub error: Option<&'static str>,
    pub raw_outputs: [f32; 2],
    pub totals: ShadowMetricTotals,
    pub strategies: [ShadowMetricTotals; 4],
    pub life_stages: [ShadowMetricTotals; 3],
    pub strata: [ShadowMetricTotals; 6],
    pub cruising: ShadowMetricTotals,
    pub pre_death: ShadowMetricTotals,
    pub deaths: u64,
}

#[derive(Debug)]
struct Shadow {
    candidate: Arc<ShadowCandidate>,
    snapshot: ShadowSnapshot,
    before: Vec<f32>,
    after: Vec<f32>,
    scratch: CalculationScratch,
    recent: [ShadowMetricTotals; PRE_DEATH_ROWS],
    recent_count: usize,
    recent_cursor: usize,
    charged_bytes: usize,
    last_dead_id: u64,
}

/// Four diagnostic slots are empty by default; warm inference uses only admitted reusable arrays.
#[derive(Debug, Default)]
pub struct ShadowWorkspace {
    slots: [Option<Box<Shadow>>; MAX_SHADOWS],
}
impl ShadowWorkspace {
    pub fn activate(
        &mut self,
        request: &ShadowActivationRequest,
        sequence: u64,
        step: u64,
        sensor: [u8; 32],
        cadence: f64,
        input_size: usize,
    ) -> Result<(), String> {
        request.validate().map_err(str::to_owned)?;
        let candidate = &request.candidate;
        if candidate.sensor_sha256 != sensor
            || candidate.cadence != cadence
            || candidate.plan.input_size() != input_size
        {
            return Err("shadow candidate sensor/cadence mismatch".into());
        }
        let state_size = candidate.plan.total_state_size();
        let current_bytes: usize = self
            .slots
            .iter()
            .flatten()
            .map(|slot| slot.charged_bytes)
            .sum();
        let estimate = candidate
            .charged_bytes
            .checked_add(
                candidate
                    .plan
                    .scratch_layout()
                    .required_bytes()
                    .map_err(|e| e.to_string())?,
            )
            .and_then(|bytes| bytes.checked_add(state_size * 8 + std::mem::size_of::<Shadow>()))
            .ok_or("shadow cold admission overflow")?;
        if current_bytes
            .checked_add(estimate)
            .is_none_or(|total| total > SHADOW_WORKSPACE_BYTES)
        {
            return Err("shadow replacement staging exceeds workspace admission".into());
        }
        let scratch =
            CalculationScratch::try_new(candidate.plan.scratch_layout(), SHADOW_WORKSPACE_BYTES)
                .map_err(|e| e.to_string())?;
        let mut before = Vec::new();
        let mut after = Vec::new();
        for state in [&mut before, &mut after] {
            state
                .try_reserve_exact(state_size)
                .map_err(|_| "shadow state allocation admission")?;
            state.resize(state_size, 0.0);
        }
        let charged_bytes = candidate
            .charged_bytes
            .checked_add(scratch.allocated_bytes())
            .and_then(|value| {
                value.checked_add(
                    (before.capacity() + after.capacity()) * 4 + std::mem::size_of::<Shadow>(),
                )
            })
            .ok_or("shadow workspace overflow")?;
        let old_bytes: usize = self
            .slots
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != request.shadow_slot as usize)
            .filter_map(|(_, slot)| slot.as_ref())
            .map(|slot| slot.charged_bytes)
            .sum();
        if old_bytes
            .checked_add(charged_bytes)
            .is_none_or(|total| total > SHADOW_WORKSPACE_BYTES)
        {
            return Err("shadow workspace byte admission".into());
        }
        let snapshot = ShadowSnapshot {
            artifact_sha256: candidate.artifact_sha256,
            capture_epoch: request.capture_epoch,
            baseline_slot: request.baseline_slot,
            activation_sequence: sequence,
            activation_step: step,
            completed_step: step,
            generation: 0,
            physical_snake_id: 0,
            enabled: true,
            error: None,
            raw_outputs: [0.0; 2],
            totals: ShadowMetricTotals::default(),
            strategies: [ShadowMetricTotals::default(); 4],
            life_stages: [ShadowMetricTotals::default(); 3],
            strata: [ShadowMetricTotals::default(); 6],
            cruising: ShadowMetricTotals::default(),
            pre_death: ShadowMetricTotals::default(),
            deaths: 0,
        };
        self.slots[request.shadow_slot as usize] = Some(Box::new(Shadow {
            candidate: candidate.clone(),
            snapshot,
            before,
            after,
            scratch,
            recent: [ShadowMetricTotals::default(); PRE_DEATH_ROWS],
            recent_count: 0,
            recent_cursor: 0,
            charged_bytes,
            last_dead_id: 0,
        }));
        Ok(())
    }
    pub fn stop(&mut self, slot: u32) -> Result<(), &'static str> {
        let slot = self
            .slots
            .get_mut(slot as usize)
            .ok_or("invalid shadow slot")?;
        if let Some(shadow) = slot {
            shadow.snapshot.enabled = false;
        }
        Ok(())
    }
    pub fn stop_all(&mut self, reason: &'static str) {
        for shadow in self.slots.iter_mut().flatten() {
            shadow.snapshot.enabled = false;
            shadow.snapshot.error = Some(reason);
        }
    }
    pub fn snapshots(&self) -> [Option<ShadowSnapshot>; MAX_SHADOWS] {
        std::array::from_fn(|index| self.slots[index].as_ref().map(|shadow| shadow.snapshot))
    }
    pub fn observe_cold_deaths(&mut self, world: &super::state::WorldState) {
        for shadow in self
            .slots
            .iter_mut()
            .flatten()
            .filter(|shadow| shadow.snapshot.enabled)
        {
            if world
                .snakes
                .iter()
                .any(|snake| snake.id == shadow.snapshot.physical_snake_id && !snake.alive)
            {
                shadow.finish_life(shadow.snapshot.physical_snake_id);
            }
        }
    }
    pub fn observe(&mut self, boundary: PublishedTeacherBoundary<'_>) {
        for shadow in self
            .slots
            .iter_mut()
            .flatten()
            .filter(|shadow| shadow.snapshot.enabled)
        {
            if let Err(reason) = shadow.observe(boundary) {
                shadow.snapshot.enabled = false;
                shadow.snapshot.error = Some(reason);
            }
        }
    }
}

impl Shadow {
    fn finish_life(&mut self, identity: u64) {
        if identity == 0 || identity == self.last_dead_id {
            return;
        }
        for row in self.recent.iter().take(self.recent_count) {
            self.snapshot.pre_death.merge(*row);
        }
        self.snapshot.deaths += 1;
        self.last_dead_id = identity;
        self.recent_count = 0;
        self.recent_cursor = 0;
    }
    fn observe(&mut self, boundary: PublishedTeacherBoundary<'_>) -> Result<(), &'static str> {
        let step = boundary
            .key
            .source_completed_step()
            .checked_add(1)
            .ok_or("shadow step exhausted")?;
        if step <= self.snapshot.completed_step {
            return Err("shadow publication chronology");
        }
        if let Some(index) = boundary
            .decisions
            .iter()
            .position(|decision| decision.baseline_slot == self.snapshot.baseline_slot)
        {
            let decision = boundary.decisions[index];
            let input = boundary
                .observation(index)
                .ok_or("shadow exact observation unavailable")?;
            if input.len() != self.candidate.plan.input_size() {
                return Err("shadow input width changed");
            }
            let generation_changed = self.snapshot.generation != boundary.key.generation();
            let life_changed = self.snapshot.physical_snake_id != decision.physical_snake_id;
            if generation_changed
                || (life_changed && self.candidate.life_rule == TrainingLifeStateRule::Reset)
            {
                self.before.fill(0.0);
            }
            if generation_changed || life_changed {
                self.recent_count = 0;
                self.recent_cursor = 0;
            }
            let mut output = [0.0; 2];
            self.candidate
                .plan
                .evaluate(
                    &self.candidate.weights,
                    &self.before,
                    &mut self.after,
                    input,
                    &mut output,
                    &mut self.scratch.view(),
                )
                .map_err(|_| "shadow inference failed")?;
            if output
                .iter()
                .chain(&self.after)
                .any(|value| !value.is_finite())
            {
                return Err("shadow nonfinite inference");
            }
            std::mem::swap(&mut self.before, &mut self.after);
            self.snapshot.generation = boundary.key.generation();
            self.snapshot.physical_snake_id = decision.physical_snake_id;
            self.snapshot.raw_outputs = output;
            let error = f64::from(output[0].clamp(-1.0, 1.0) - decision.applied_turn);
            let disagreement = (output[1] > 0.35) != decision.boost_request;
            let mut row = ShadowMetricTotals::default();
            row.add(error, disagreement);
            self.snapshot.totals.merge(row);
            self.snapshot.strategies[strategy(decision)].merge(row);
            self.snapshot.life_stages[stage(decision)].merge(row);
            let flags = [
                decision.unsafe_route || decision.urgent_sectors > 0,
                decision.escape_or_pursuit,
                decision.boost_request,
                decision.strategy_transition,
                input[22] > 0.0 || input[23] > 0.0,
                input[21] > 0.0,
            ];
            for (index, flag) in flags.iter().enumerate() {
                if *flag {
                    self.snapshot.strata[index].merge(row);
                }
            }
            if !flags.iter().any(|flag| *flag) {
                self.snapshot.cruising.merge(row);
            }
            self.recent[self.recent_cursor] = row;
            self.recent_cursor = (self.recent_cursor + 1) % PRE_DEATH_ROWS;
            self.recent_count = (self.recent_count + 1).min(PRE_DEATH_ROWS);
        }
        if let Some(outcome) = boundary.slot_outcome(self.snapshot.baseline_slot) {
            if outcome.physical_snake_id == self.snapshot.physical_snake_id
                && outcome.post_step_death
            {
                self.finish_life(outcome.physical_snake_id);
            }
        }
        self.snapshot.completed_step = step;
        Ok(())
    }
}
fn strategy(decision: PreparedTeacherDecision) -> usize {
    use super::state::BaselineStrategyState;
    match decision.strategy {
        BaselineStrategyState::Roam => 0,
        BaselineStrategyState::Seek => 1,
        BaselineStrategyState::Avoid => 2,
        BaselineStrategyState::Boost => 3,
    }
}
fn stage(decision: PreparedTeacherDecision) -> usize {
    use super::baseline_control::BaselineLifeStage;
    match decision.life_stage {
        BaselineLifeStage::Small => 0,
        BaselineLifeStage::Medium => 1,
        BaselineLifeStage::Large => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::baseline_control::BaselineLifeStage;
    use crate::engine::baseline_v2::{BaselineV2Action, BaselineV2State};
    use crate::engine::physics::PhysicsStepKey;
    use crate::engine::state::BaselineStrategyState;
    use crate::engine::teacher_capture::{
        TeacherBoundaryWorkspace, TeacherPhysicalOutcome, TeacherPhysicalOutcomes,
    };

    fn candidate(rule: TrainingLifeStateRule) -> Arc<ShadowCandidate> {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/portable-brain-v1.slither-brain"),
        )
        .unwrap();
        let artifact = super::super::brain_artifact::validate_brain_artifact(&bytes).unwrap();
        let mut candidate = ShadowCandidate::from_artifact(artifact).unwrap();
        Arc::get_mut(&mut candidate).unwrap().life_rule = rule;
        candidate
    }
    fn request(candidate: Arc<ShadowCandidate>) -> ShadowActivationRequest {
        ShadowActivationRequest {
            expected_run_id: "synthetic-run".into(),
            expected_config_revision: 1,
            capture_epoch: [13; 16],
            shadow_slot: 0,
            baseline_slot: 7,
            candidate,
        }
    }
    fn activate(
        workspace: &mut ShadowWorkspace,
        request: &ShadowActivationRequest,
        sequence: u64,
        step: u64,
    ) {
        workspace
            .activate(
                request,
                sequence,
                step,
                request.candidate.sensor_sha256,
                request.candidate.cadence,
                request.candidate.plan.input_size(),
            )
            .unwrap();
    }
    fn publish(
        workspace: &mut ShadowWorkspace,
        step: u64,
        generation: u64,
        identity: u64,
        due: bool,
        dead: bool,
    ) {
        let mut rows = TeacherBoundaryWorkspace::default();
        rows.configure(&[7], 72).unwrap();
        if due {
            let mut input = [0.125; 72];
            input[21] = if identity == 77 { -1.0 } else { 1.0 };
            rows.stage(
                7,
                identity,
                &input,
                BaselineV2Action {
                    turn: 0.25,
                    boost: true,
                    sector: 0,
                    life_stage: BaselineLifeStage::Medium,
                    normalized_score: 0.1,
                    unsafe_route: false,
                    escape_or_pursuit: true,
                    urgent_sectors: 1,
                    next_state: BaselineV2State {
                        strategy: BaselineStrategyState::Boost,
                        ..BaselineV2State::default()
                    },
                    next_behavior_rng_word: 17,
                    uniform_draws: 0,
                },
                BaselineStrategyState::Roam,
            )
            .unwrap();
        }
        let mut outcomes = TeacherPhysicalOutcomes::default();
        outcomes.0[7] = TeacherPhysicalOutcome {
            physical_snake_id: identity,
            realized_boost: true,
            post_step_death: dead,
        };
        let key = PhysicsStepKey::new(1, generation, step - 1, 1, 1, [3; 32], 1);
        workspace.observe(PublishedTeacherBoundary::new(rows.prepared(key), &outcomes));
    }
    #[test]
    fn registry_never_evicts_pinned_commands_and_admits_again_after_release() {
        let mut registry = Registry::default();
        let mut pins = Vec::new();
        for id in 1..=7 {
            let mut entry = candidate(TrainingLifeStateRule::Preserve);
            Arc::get_mut(&mut entry).unwrap().artifact_sha256 = [id; 32];
            registry.insert(entry.clone()).unwrap();
            pins.push(entry);
        }
        assert!(registry.bytes <= SHADOW_WORKSPACE_BYTES);
        let mut next = candidate(TrainingLifeStateRule::Preserve);
        Arc::get_mut(&mut next).unwrap().artifact_sha256 = [8; 32];
        assert!(registry.insert(next.clone()).is_err());
        assert_eq!(registry.entries.len(), 7);
        pins.remove(0);
        registry.insert(next).unwrap();
        assert!(registry.bytes <= SHADOW_WORKSPACE_BYTES);
        assert!(registry
            .entries
            .iter()
            .all(|entry| entry.artifact_sha256 != [1; 32]));
        assert!(pins
            .iter()
            .all(|pin| registry.entries.iter().any(|entry| Arc::ptr_eq(pin, entry))));
    }

    #[test]
    fn exact_native_recurrence_advances_only_on_delivered_rows_and_retains_buffers() {
        let req = request(candidate(TrainingLifeStateRule::Preserve));
        let mut workspace = ShadowWorkspace::default();
        activate(&mut workspace, &req, 5, 0);
        let pointers = {
            let slot = workspace.slots[0].as_ref().unwrap();
            (
                slot.before.as_ptr(),
                slot.after.as_ptr(),
                slot.scratch.allocated_bytes(),
            )
        };
        publish(&mut workspace, 1, 1, 77, true, false);
        let before = workspace.slots[0].as_ref().unwrap().before.clone();
        let raw = workspace.snapshots()[0].unwrap().raw_outputs;
        publish(&mut workspace, 2, 1, 77, false, false);
        assert_eq!(workspace.slots[0].as_ref().unwrap().before, before);
        assert_eq!(workspace.snapshots()[0].unwrap().raw_outputs, raw);
        for step in 3..=256 {
            publish(&mut workspace, step, 1, 77, step % 2 != 0, false);
        }
        let slot = workspace.slots[0].as_ref().unwrap();
        assert!(slot.snapshot.enabled);
        assert_eq!(slot.snapshot.totals.samples, 128);
        assert_eq!(slot.snapshot.strategies[3].samples, 128);
        assert_eq!(slot.snapshot.life_stages[1].samples, 128);
        assert_eq!(slot.snapshot.strata[0].samples, 128);
        assert_eq!(slot.snapshot.cruising.samples, 0);
        assert_eq!(
            (
                slot.before.as_ptr(),
                slot.after.as_ptr(),
                slot.scratch.allocated_bytes()
            ),
            pointers
        );
    }
    #[test]
    fn physical_birth_obeys_candidate_life_rule_and_generation_always_resets() {
        let mut reset = ShadowWorkspace::default();
        let mut preserve = ShadowWorkspace::default();
        activate(
            &mut reset,
            &request(candidate(TrainingLifeStateRule::Reset)),
            1,
            0,
        );
        activate(
            &mut preserve,
            &request(candidate(TrainingLifeStateRule::Preserve)),
            1,
            0,
        );
        for step in 1..=3 {
            publish(&mut reset, step, 1, 77, true, false);
            publish(&mut preserve, step, 1, 77, true, false);
        }
        assert_eq!(
            reset.snapshots()[0].unwrap().raw_outputs,
            preserve.snapshots()[0].unwrap().raw_outputs
        );
        publish(&mut reset, 4, 1, 78, true, false);
        publish(&mut preserve, 4, 1, 78, true, false);
        assert_ne!(
            reset.snapshots()[0].unwrap().raw_outputs,
            preserve.snapshots()[0].unwrap().raw_outputs
        );
        publish(&mut reset, 5, 2, 79, true, false);
        publish(&mut preserve, 5, 2, 79, true, false);
        assert_eq!(
            reset.snapshots()[0].unwrap().raw_outputs,
            preserve.snapshots()[0].unwrap().raw_outputs
        );
    }
    #[test]
    fn held_step_death_seals_only_latest_32_rows_and_deduplicates() {
        let req = request(candidate(TrainingLifeStateRule::Preserve));
        let mut workspace = ShadowWorkspace::default();
        activate(&mut workspace, &req, 1, 0);
        for step in 1..=40 {
            publish(&mut workspace, step, 1, 77, true, false);
        }
        publish(&mut workspace, 41, 1, 77, false, true);
        publish(&mut workspace, 42, 1, 77, false, true);
        let snapshot = workspace.snapshots()[0].unwrap();
        assert_eq!(snapshot.deaths, 1);
        assert_eq!(snapshot.pre_death.samples, 32);
        publish(&mut workspace, 43, 1, 78, true, true);
        let snapshot = workspace.snapshots()[0].unwrap();
        assert_eq!(snapshot.deaths, 2);
        assert_eq!(snapshot.pre_death.samples, 33);
        assert_eq!(snapshot.strata[5].samples, 1);
    }
    #[test]
    fn rejected_activation_preserves_segment_and_explicit_version_resets_it() {
        let req = request(candidate(TrainingLifeStateRule::Preserve));
        let mut workspace = ShadowWorkspace::default();
        activate(&mut workspace, &req, 1, 0);
        publish(&mut workspace, 1, 1, 77, true, false);
        assert!(workspace
            .activate(&req, 2, 1, [0; 32], req.candidate.cadence, 72)
            .is_err());
        assert!(workspace
            .activate(&req, 2, 1, req.candidate.sensor_sha256, 0.07, 72)
            .is_err());
        assert_eq!(workspace.snapshots()[0].unwrap().totals.samples, 1);
        activate(&mut workspace, &req, 3, 1);
        let snapshot = workspace.snapshots()[0].unwrap();
        assert_eq!(snapshot.totals.samples, 0);
        assert_eq!(snapshot.activation_sequence, 3);
        assert!(workspace.slots[0]
            .as_ref()
            .unwrap()
            .before
            .iter()
            .all(|value| *value == 0.0));
        let mut invalid = req.clone();
        invalid.shadow_slot = 4;
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn nonfinite_inference_disables_only_affected_slot_without_recurrent_publication() {
        let mut bad = candidate(TrainingLifeStateRule::Preserve);
        Arc::get_mut(&mut bad).unwrap().weights.fill(f32::NAN);
        let mut workspace = ShadowWorkspace::default();
        let req = request(bad);
        activate(&mut workspace, &req, 1, 0);
        let mut good = request(candidate(TrainingLifeStateRule::Preserve));
        good.shadow_slot = 1;
        activate(&mut workspace, &good, 2, 0);
        publish(&mut workspace, 1, 1, 77, true, false);
        let snapshots = workspace.snapshots();
        assert!(!snapshots[0].unwrap().enabled);
        assert!(snapshots[0].unwrap().error.is_some());
        assert!(snapshots[1].unwrap().enabled);
        assert_eq!(snapshots[1].unwrap().totals.samples, 1);
        assert!(workspace.slots[0]
            .as_ref()
            .unwrap()
            .before
            .iter()
            .all(|value| *value == 0.0));
    }
}
