//! One opt-in diagnostic cache of actual delivered control observations.
//! Pending rows are private until whole-step publication, including external send acceptance.

use super::baseline_v2::BaselineV2Action;
use super::physics::PhysicsStepKey;
use super::sensor_contract::SensorProfile;
use super::sensor_layout::SensorLayout;
use super::sensor_trace::SensorTrace;
use super::sensors::SensorSampleDiagnostics;
use super::state::{SnakeKind, SnakeState};

/// Motion-v4 B32 is the largest admitted structural observation contract.
pub const MAX_INSPECTOR_INPUTS: usize = 504;
/// Independent cold diagnostic admission; the ordinary worker envelope is unchanged.
pub const MAX_INSPECTOR_BYTES: usize = 64 * 1024;
/// Hard bound on one Rust-authored UTF-8 inspector projection.
pub const MAX_INSPECTOR_WIRE_BYTES: usize = 64 * 1024;
/// Projection tree, encoded reply and mailbox-copy peaks are admitted together.
pub const INSPECTOR_TRANSPORT_RESERVE: usize = 8 * MAX_INSPECTOR_WIRE_BYTES;

/// Replaceable native mailbox snapshot; no world or graph traversal occurs on reads.
#[derive(Clone, Copy, Debug)]
pub struct RunningInspectorStatus {
    pub sequence: u64,
    pub subscription_sequence: u64,
    pub world_epoch: u64,
    pub completed_step: u64,
    pub observation: Option<InspectorObservation>,
}

/// Exact physical life selected at a clean authoritative boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InspectorTarget {
    pub world_epoch: u64,
    pub generation: u64,
    pub physical_id: u64,
    pub frame_v1_id: u32,
}

/// The actual consumer of this observation, independent of physical snake kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorObservationSource {
    Neural,
    BaselineV1,
    BaselineV2,
    External { lease_id: u64, connection_id: u64 },
}

/// Exact structural profile is recorded even when different contracts have equal widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InspectorLayout {
    pub profile: SensorProfile,
    pub bins: usize,
    pub input_size: usize,
}
impl From<&SensorLayout> for InspectorLayout {
    fn from(layout: &SensorLayout) -> Self {
        Self {
            profile: layout.profile,
            bins: layout.bins,
            input_size: layout.input_size,
        }
    }
}

/// Raw physical scalars from the same immutable pre-movement observation boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorRawScalars {
    pub kind: SnakeKind,
    pub population_slot: Option<u32>,
    pub points: f64,
    pub previously_delivered_points: f64,
    pub first_delivery: bool,
    pub x: f64,
    pub y: f64,
    pub heading: f64,
    pub radius: f64,
    pub speed: f64,
    pub control_accumulator_seconds: f64,
}

/// Native ledger and the existing evolution formula at the observation boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorLifecycle {
    pub life_ordinal: u64,
    pub lives_started: u64,
    pub deaths: u64,
    pub successful_respawns: u64,
    pub failed_placements: u64,
    pub pending_seconds: f64,
    pub point_mode: super::evolved_lives::PointAssessmentMode,
    pub nonpoint_mode: super::evolved_lives::NonPointAggregation,
    pub current: super::evolved_lives::LifeAssessment,
    pub best_point_life: super::evolved_lives::LifeAssessment,
    pub ledger: super::evolved_lives::EvolvedEconomicLedger,
    pub assessment: super::evolved_lives::EvolvedAssessment,
    pub maximum_assessment_points: f64,
    pub provisional_fitness: f64,
    pub fitness_components: [f64; 7],
}

/// Actual v1 policy result, copied before reusable baseline storage changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorBaselineV1Action {
    pub turn: f32,
    pub boost: bool,
    pub baseline_slot: u32,
    pub strategy: super::state::BaselineStrategyState,
    pub effective_strategy: super::state::BaselineStrategyState,
    pub life_stage: super::baseline_control::BaselineLifeStage,
    pub sector: usize,
    pub target_physical_id: Option<u64>,
    pub uniform_draws: usize,
    pub strategy_timer_seconds: f64,
    pub wander_angle: f64,
    pub wander_remaining_seconds: f64,
}

/// One candidate output from the exact same committed teacher row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorShadowOutput {
    pub shadow_slot: u32,
    pub artifact_sha256: [u8; 32],
    pub capture_epoch: [u8; 16],
    pub activation_sequence: u64,
    pub sample_step: u64,
    pub raw_outputs: [f32; 2],
}

/// Borrowed pre-control life state; this does not create or advance any assessment.
#[derive(Clone, Copy)]
pub(crate) struct InspectorLifecycleContext<'a> {
    pub lives: &'a super::evolved_lives::EvolvedGenerationState,
    pub policy: super::evolved_lives::EvolvedExperimentConfig,
}

/// Fixed-size exact row; graph output and teacher policy remain separately labelled.
#[derive(Clone, Copy, Debug)]
pub struct InspectorObservation {
    pub key: PhysicsStepKey,
    pub subscription_sequence: u64,
    pub target: InspectorTarget,
    pub source: InspectorObservationSource,
    pub raw: InspectorRawScalars,
    pub layout: InspectorLayout,
    normalized_inputs: [f32; MAX_INSPECTOR_INPUTS],
    pub neural_outputs: Option<[f32; 2]>,
    pub baseline_action: Option<BaselineV2Action>,
    pub baseline_v1_action: Option<InspectorBaselineV1Action>,
    pub lifecycle: Option<InspectorLifecycle>,
    pub shadow_outputs: [Option<InspectorShadowOutput>; super::shadows::MAX_SHADOWS],
    pub diagnostics: SensorSampleDiagnostics,
    pub trace: Option<SensorTrace>,
}

impl InspectorObservation {
    /// Only the exact meaningful prefix is exposed; unused fixed capacity stays private.
    pub fn normalized_inputs(&self) -> &[f32] {
        &self.normalized_inputs[..self.layout.input_size]
    }

    /// Observation belongs to control inside the next complete fixed step.
    pub fn sample_step(&self) -> Option<u64> {
        self.key.source_completed_step().checked_add(1)
    }

    /// A held-control step changes age, never the captured observation or its first flag.
    pub fn age_steps(&self, completed_step: u64) -> Option<u64> {
        completed_step.checked_sub(self.sample_step()?)
    }
}

/// Allocated only by explicit cold admission; no allocation occurs while sampling or publishing.
#[derive(Debug)]
pub struct InspectorCapture {
    target: InspectorTarget,
    subscription_sequence: u64,
    latest: Option<InspectorObservation>,
    pending: Option<InspectorObservation>,
    working_trace: SensorTrace,
    evolution: Option<super::evolution::EvolutionConfig>,
}

impl InspectorCapture {
    /// Admit one complete fixed-size double buffer before replacing any previous subscription.
    pub fn new(target: InspectorTarget, budget_bytes: usize) -> Result<Self, &'static str> {
        if target.world_epoch == 0
            || target.generation == 0
            || target.physical_id == 0
            || target.frame_v1_id == 0
            || budget_bytes > MAX_INSPECTOR_BYTES
            || std::mem::size_of::<Self>() > budget_bytes
        {
            return Err("inspector target or diagnostic byte admission");
        }
        Ok(Self {
            target,
            subscription_sequence: 0,
            latest: None,
            pending: None,
            working_trace: SensorTrace::default(),
            evolution: None,
        })
    }

    pub const fn target(&self) -> InspectorTarget {
        self.target
    }

    /// Bind a newly admitted cache to its exact queue command before any sample exists.
    pub(crate) fn bind_subscription(&mut self, sequence: u64) {
        debug_assert!(self.latest.is_none() && self.pending.is_none());
        self.subscription_sequence = sequence;
    }

    /// Borrow only the selected physical owner's actual sampling buffer.
    pub fn trace_for(&mut self, physical_id: u64) -> Option<&mut SensorTrace> {
        if physical_id != self.target.physical_id {
            return None;
        }
        self.working_trace.valid = false;
        Some(&mut self.working_trace)
    }

    /// Move one exclusive buffer into the selected neural worker partition.
    pub fn trace_request(&mut self) -> (u64, &mut SensorTrace) {
        self.working_trace.valid = false;
        (self.target.physical_id, &mut self.working_trace)
    }

    /// Previous successful delivery remains visible while an unrelated or held step is prepared.
    pub fn latest(
        &self,
        world_epoch: u64,
        generation: u64,
        completed_step: u64,
    ) -> Option<&InspectorObservation> {
        let sample = self.latest.as_ref()?;
        (sample.target.world_epoch == world_epoch
            && sample.target.generation == generation
            && sample.age_steps(completed_step).is_some())
        .then_some(sample)
    }

    /// Drop only uncommitted debug data; no world, delivery marker, RNG or quota is touched.
    pub fn discard_pending(&mut self) {
        self.pending = None;
    }

    /// Copy one actual control row, after identity matching and before reusable worker storage changes.
    #[allow(clippy::too_many_arguments)]
    pub fn stage(
        &mut self,
        key: PhysicsStepKey,
        snake: &SnakeState,
        source: InspectorObservationSource,
        layout: InspectorLayout,
        inputs: &[f32],
        outputs: Option<[f32; 2]>,
        baseline_action: Option<BaselineV2Action>,
        diagnostics: SensorSampleDiagnostics,
    ) -> bool {
        if key.world_epoch() != self.target.world_epoch
            || key.generation() != self.target.generation
            || snake.id != self.target.physical_id
            || snake.frame_v1_id != self.target.frame_v1_id
            || inputs.len() != layout.input_size
            || inputs.is_empty()
            || inputs.len() > MAX_INSPECTOR_INPUTS
            || inputs.iter().any(|v| !v.is_finite())
            || [
                snake.points,
                snake.delivered_observation_points,
                snake.position.x,
                snake.position.y,
                snake.direction,
                snake.radius,
                snake.speed,
                snake.control_accumulator_seconds,
            ]
            .iter()
            .any(|v| !v.is_finite())
            || outputs.is_some_and(|values| values.iter().any(|v| !v.is_finite()))
            || key.source_completed_step().checked_add(1).is_none()
        {
            return false;
        }
        let raw = InspectorRawScalars {
            kind: snake.kind,
            population_slot: snake.population_slot,
            points: snake.points,
            previously_delivered_points: snake.delivered_observation_points,
            first_delivery: !snake.observation_delivered,
            x: snake.position.x,
            y: snake.position.y,
            heading: snake.direction,
            radius: snake.radius,
            speed: snake.speed,
            control_accumulator_seconds: snake.control_accumulator_seconds,
        };
        let mut row = InspectorObservation {
            key,
            subscription_sequence: self.subscription_sequence,
            target: self.target,
            source,
            raw,
            layout,
            normalized_inputs: [0.0; MAX_INSPECTOR_INPUTS],
            neural_outputs: outputs,
            baseline_action,
            baseline_v1_action: None,
            lifecycle: None,
            shadow_outputs: [None; super::shadows::MAX_SHADOWS],
            diagnostics,
            trace: (self.working_trace.valid
                && self.working_trace.physical_id == snake.id
                && self.working_trace.profile == layout.profile
                && self.working_trace.bins == layout.bins)
                .then_some(self.working_trace),
        };
        row.normalized_inputs[..inputs.len()].copy_from_slice(inputs);
        self.pending = Some(row);
        true
    }

    /// Cache immutable reward settings only at explicit cold admission.
    pub(crate) fn configure_evolution(&mut self, config: super::evolution::EvolutionConfig) {
        self.evolution = Some(config);
    }

    /// Attach the actual staged v1 action to its exact pending observation.
    pub(crate) fn stage_baseline_v1(
        &mut self,
        key: PhysicsStepKey,
        physical_id: u64,
        action: InspectorBaselineV1Action,
    ) {
        if let Some(row) = self.pending.as_mut().filter(|row| {
            row.key == key
                && row.target.physical_id == physical_id
                && row.source == InspectorObservationSource::BaselineV1
        }) {
            row.baseline_v1_action = Some(action);
        }
    }

    /// Copy ledger values and calculate provisional fitness with the existing native formula.
    pub(crate) fn stage_lifecycle(
        &mut self,
        key: PhysicsStepKey,
        world: &super::state::WorldState,
        context: InspectorLifecycleContext<'_>,
    ) {
        let Some(config) = self.evolution else {
            return;
        };
        let Some(row) = self
            .pending
            .as_mut()
            .filter(|row| row.key == key && row.raw.kind == SnakeKind::Evolved)
        else {
            return;
        };
        let Some(slot) = row
            .raw
            .population_slot
            .and_then(|slot| context.lives.slots.get(slot as usize))
            .filter(|slot| {
                Some(slot.population_slot) == row.raw.population_slot
                    && slot.physical_id == row.target.physical_id
            })
        else {
            return;
        };
        let Some(snake) = world.snakes.iter().find(|snake| {
            snake.id == row.target.physical_id
                && snake.frame_v1_id == row.target.frame_v1_id
                && snake.population_slot == row.raw.population_slot
        }) else {
            return;
        };
        let entry =
            super::experiment_records::selected_assessment(slot, snake, context.policy, config);
        let maximum = if context.policy.uses_life_assessments() {
            context
                .lives
                .slots
                .iter()
                .map(|slot| {
                    slot.assessment(context.policy.points, context.policy.nonpoints)
                        .points
                })
                .fold(0.0_f64, f64::max)
        } else {
            world
                .snakes
                .iter()
                .filter(|snake| snake.kind == SnakeKind::Evolved)
                .map(|snake| snake.points)
                .fold(0.0_f64, f64::max)
        };
        let fitness = super::evolution::assessment_fitness_breakdown(
            entry,
            maximum,
            config,
            context.policy.objective,
        );
        if !fitness.total.is_finite() || fitness.components.iter().any(|value| !value.is_finite()) {
            return;
        }
        row.lifecycle = Some(InspectorLifecycle {
            life_ordinal: slot.life_ordinal,
            lives_started: slot.lives_started,
            deaths: slot.deaths,
            successful_respawns: slot.successful_respawns,
            failed_placements: slot.failed_placements,
            pending_seconds: slot.pending_seconds,
            point_mode: context.policy.points,
            nonpoint_mode: context.policy.nonpoints,
            current: slot.current,
            best_point_life: slot.best_point_life,
            ledger: slot.ledger,
            assessment: entry.assessment,
            maximum_assessment_points: maximum,
            provisional_fitness: fitness.total,
            fitness_components: fitness.components,
        });
    }

    /// Join only outputs actually evaluated on this published teacher row, never nearest-in-time data.
    pub(crate) fn join_shadows(
        &mut self,
        key: PhysicsStepKey,
        snapshots: [Option<super::shadows::ShadowSnapshot>; super::shadows::MAX_SHADOWS],
    ) {
        let Some(row) = self
            .latest
            .as_mut()
            .filter(|row| row.key == key && row.source == InspectorObservationSource::BaselineV2)
        else {
            return;
        };
        for (index, snapshot) in snapshots.iter().enumerate() {
            row.shadow_outputs[index] = snapshot
                .filter(|shadow| {
                    shadow.enabled
                        && shadow.activation_sequence > 0
                        && shadow.capture_epoch != [0; 16]
                        && shadow.generation == row.target.generation
                        && shadow.physical_snake_id == row.target.physical_id
                        && Some(shadow.last_sample_step) == row.sample_step()
                        && shadow.raw_outputs.iter().all(|value| value.is_finite())
                })
                .map(|shadow| InspectorShadowOutput {
                    shadow_slot: index as u32,
                    artifact_sha256: shadow.artifact_sha256,
                    capture_epoch: shadow.capture_epoch,
                    activation_sequence: shadow.activation_sequence,
                    sample_step: shadow.last_sample_step,
                    raw_outputs: shadow.raw_outputs,
                });
        }
    }

    /// Join only the exact successful step and, for external rows, its accepted socket delivery.
    pub fn publish(
        &mut self,
        key: PhysicsStepKey,
        external_accepted: impl FnOnce(u64, u64, u64) -> bool,
    ) {
        let Some(row) = self.pending.take() else {
            return;
        };
        if row.key != key {
            return;
        }
        if let InspectorObservationSource::External {
            lease_id,
            connection_id,
        } = row.source
        {
            if !external_accepted(row.target.physical_id, lease_id, connection_id) {
                return;
            }
        }
        self.latest = Some(row);
    }
}
