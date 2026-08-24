//! Shared pre-movement control selection from one immutable indexed boundary.
//!
//! This module joins the already-verified sensor, baseline-controller,
//! wall-time lease, and heterogeneous neural operations without publishing
//! authority. Every alive snake receives exactly one exclusive source decision
//! in stable internal-ID order. External observations remain uncommitted until
//! the thin bridge reports acceptance for the matching connection boundary.

use super::baseline::{
    validate_strategy_runtime, BaselineLifecycleError, BaselineLifecycleState, BaselineSlotRuntime,
};
use super::baseline_control::{
    BaselineControlConfig, BaselineControlDiagnostics, BaselineControlError,
    BaselineControlWorkspace,
};
use super::calculation::{CalculationBatchKey, CalculationCandidateIndex};
use super::control::{
    NeuralControlBatchInputs, NeuralControlCapacityDiagnostics, NeuralControlError,
    NeuralControlPipeline,
};
use super::controllers::{
    commit_controller_boundary_prevalidated, prepare_controller_boundary,
    validate_controller_boundary, ControllerBoundaryProposal, ControllerError, ControllerTiming,
    ExternalControlSource,
};
use super::fixed_step::{
    copy_lifecycle_reusing, copy_rng_bundle_reusing, copy_serialized_rng_reusing,
    copy_world_reusing, rng_text_capacity, FixedStepPrefixConfig, FixedStepPrefixError,
    PreparedFixedStepPrefix, RngCopyScratch,
};
use super::physics::PhysicsStepKey;
use super::sensors::{
    ObservationDeliveryMarker, SensorError, SensorGenerationState, SensorSampleDiagnostics,
    SensorScratch, SensorScratchDiagnostics,
};
use super::spatial::{
    BodyIndexDiagnostics, BodySpatialIndex, IndexedSensorWorld, PelletIndexDiagnostics,
    PelletSpatialIndex, SensorIndexConfig, SpatialIndexError,
};
use super::state::{
    AllocatorState, BaselineRngState, BaselineStrategyState, BrainHandle, BrainRuntimeState,
    ControllerKind, ControllerLeaseStatus, PopulationGenome, RngStateBundle, SnakeKind, WorldPoint,
    WorldState,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First joined controller-selection algorithm identity.
pub const CONTROL_PHASE_VERSION: u32 = 1;
/// Current TypeScript neural boost decision boundary.
const NEURAL_BOOST_THRESHOLD: f32 = 0.35;
/// Smallest admitted live neural-control interval.
const MINIMUM_NEURAL_CONTROL_INTERVAL_SECONDS: f64 = 0.008;
/// Largest admitted interval and versioned pending-first-action sentinel.
const MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS: f64 = 0.06;

/// Complete projected settings and bounds for one control boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPhaseConfig {
    /// Versioned selection/cadence identity.
    pub algorithm_version: u32,
    /// Simulated interval between neural evaluations.
    pub neural_control_interval_seconds: f64,
    /// Owner-selected wall-time input hold and disconnect grace.
    pub controller_timing: ControllerTiming,
    /// Baseline strategy settings.
    pub baseline: BaselineControlConfig,
    /// Complete stable-boundary sensor index settings.
    pub sensor_index: SensorIndexConfig,
    /// Maximum admitted world records and staged controls.
    pub maximum_snakes: usize,
    /// Maximum admitted brain-runtime records.
    pub maximum_brains: usize,
    /// Maximum observations prepared for live external sockets per step.
    pub maximum_external_observations: usize,
}

impl ControlPhaseConfig {
    /// Current TypeScript defaults with explicit P0-P3-capable ceilings.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            algorithm_version: CONTROL_PHASE_VERSION,
            neural_control_interval_seconds: 1.0 / 60.0,
            controller_timing: ControllerTiming::approved_defaults(),
            baseline: BaselineControlConfig::typescript_defaults(),
            sensor_index: SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 200_000,
            },
            maximum_snakes: 512,
            maximum_brains: 512,
            maximum_external_observations: 512,
        }
    }

    /// Initial cadence value for a newly created neural snake.
    ///
    /// The maximum admitted interval is reserved to represent "no neural
    /// action has been produced" without adding a second persistent flag.
    /// Ordinary post-action remainders are always strictly smaller. The first
    /// boundary therefore remains due even if the live interval changes after
    /// creation; after that evaluation the ordinary remainder is stored.
    #[must_use]
    pub const fn initial_neural_accumulator_seconds(self) -> f64 {
        MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS
    }

    fn validate(self) -> Result<(), ControlPhaseError> {
        if self.algorithm_version != CONTROL_PHASE_VERSION {
            return Err(ControlPhaseError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        if !self.neural_control_interval_seconds.is_finite()
            || !(MINIMUM_NEURAL_CONTROL_INTERVAL_SECONDS..=MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS)
                .contains(&self.neural_control_interval_seconds)
        {
            return Err(ControlPhaseError::InvalidConfig {
                field: "neural_control_interval_seconds",
            });
        }
        if self.maximum_snakes == 0
            || self.maximum_brains == 0
            || self.maximum_external_observations > self.maximum_snakes
        {
            return Err(ControlPhaseError::InvalidConfig {
                field: "control capacities",
            });
        }
        self.baseline.validate()?;
        Ok(())
    }
}

/// Immutable inputs from one already-prepared fixed-step prefix.
pub struct ControlPhaseInputs<'prefix, 'source> {
    /// Accounting/ambient/baseline-timer result used as the one sensor world.
    pub prefix: PreparedFixedStepPrefix<'prefix, 'source>,
    /// Generation-best value initialized before the first sample.
    pub generation: &'source SensorGenerationState,
    /// Dense evolved genomes.
    pub population: &'source [PopulationGenome],
    /// Stable runtime brains and recurrent state.
    pub brains: &'source [BrainRuntimeState],
    /// Monotonic wall time read for this boundary.
    pub wall_now_ms: u64,
    /// Exact projected control settings.
    pub config: ControlPhaseConfig,
}

/// Exclusive control source selected for one alive snake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectedControlSource {
    /// Built-in baseline strategy consumed one corrected observation.
    Baseline,
    /// Latest accepted external action remains within its hold window.
    ExternalHeld,
    /// External ownership remains exclusive with neutral steering.
    ExternalReservedNeutral,
    /// External-kind snake has no active lease/takeover policy.
    ExternalOnlyNeutral,
    /// A new graph output was evaluated on this boundary.
    NeuralEvaluated,
    /// Prior graph output remains held until its next cadence boundary.
    NeuralHeld,
    /// Grace expired and this exact boundary starts zero-state neural control.
    NeuralTakeover,
}

/// One stable-ID-ordered chosen action and cadence continuation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedControlUpdate {
    snake_index: usize,
    snake_id: u64,
    source: SelectedControlSource,
    turn: f32,
    boost: bool,
    next_control_accumulator_seconds: f64,
}

impl PreparedControlUpdate {
    /// Source world array index guarded by the stable ID.
    #[must_use]
    pub const fn snake_index(self) -> usize {
        self.snake_index
    }

    /// Exact internal snake identity.
    #[must_use]
    pub const fn snake_id(self) -> u64 {
        self.snake_id
    }

    /// One exclusive selected source.
    #[must_use]
    pub const fn source(self) -> SelectedControlSource {
        self.source
    }

    /// Finite selected steering value in `[-1, 1]`.
    #[must_use]
    pub const fn turn(self) -> f32 {
        self.turn
    }

    /// Selected boost request after source-specific thresholding.
    #[must_use]
    pub const fn boost(self) -> bool {
        self.boost
    }

    /// Neural cadence accumulator after this fixed-step boundary.
    #[must_use]
    pub const fn next_control_accumulator_seconds(self) -> f64 {
        self.next_control_accumulator_seconds
    }
}

/// One staged wall-time lease transition bound to its source indexes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedControllerTransition {
    lease_index: usize,
    snake_index: usize,
    proposal: ControllerBoundaryProposal,
}

impl PreparedControllerTransition {
    /// Source controller-lease array index.
    #[must_use]
    pub const fn lease_index(self) -> usize {
        self.lease_index
    }

    /// Source snake array index.
    #[must_use]
    pub const fn snake_index(self) -> usize {
        self.snake_index
    }

    /// Fully snapshotted lease/source proposal.
    #[must_use]
    pub const fn proposal(self) -> ControllerBoundaryProposal {
        self.proposal
    }
}

/// One internally consumed baseline observation and state continuation.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedBaselineControl {
    snake_index: usize,
    snake_id: u64,
    slot: u32,
    next_slot: BaselineSlotRuntime,
    next_strategy: BaselineStrategyState,
    next_rng: BaselineRngState,
    delivery: ObservationDeliveryMarker,
    diagnostics: BaselineControlDiagnostics,
    sensor_diagnostics: SensorSampleDiagnostics,
}

impl PreparedBaselineControl {
    /// Source snake array index.
    #[must_use]
    pub const fn snake_index(&self) -> usize {
        self.snake_index
    }

    /// Stable snake identity.
    #[must_use]
    pub const fn snake_id(&self) -> u64 {
        self.snake_id
    }

    /// Stable dense baseline slot.
    #[must_use]
    pub const fn slot(&self) -> u32 {
        self.slot
    }

    /// Staged lifecycle behavior/action continuation.
    #[must_use]
    pub const fn next_slot(&self) -> BaselineSlotRuntime {
        self.next_slot
    }

    /// Staged canonical world strategy.
    #[must_use]
    pub const fn next_strategy(&self) -> BaselineStrategyState {
        self.next_strategy
    }

    /// Staged independent per-slot RNG continuation.
    #[must_use]
    pub const fn next_rng(&self) -> &BaselineRngState {
        &self.next_rng
    }

    /// Internally consumed observation marker.
    #[must_use]
    pub const fn delivery(&self) -> ObservationDeliveryMarker {
        self.delivery
    }

    /// Strategy work and RNG-draw diagnostics.
    #[must_use]
    pub const fn diagnostics(&self) -> BaselineControlDiagnostics {
        self.diagnostics
    }

    /// Corrected sensing work diagnostics.
    #[must_use]
    pub const fn sensor_diagnostics(&self) -> SensorSampleDiagnostics {
        self.sensor_diagnostics
    }
}

/// One not-yet-delivered external observation for the thin bridge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedExternalObservation {
    lease_id: u64,
    connection_id: u64,
    kind: ControllerKind,
    lease_index: usize,
    snake_index: usize,
    snake_id: u64,
    position: WorldPoint,
    direction: f64,
    observation_start: usize,
    observation_len: usize,
    delivery: ObservationDeliveryMarker,
    diagnostics: SensorSampleDiagnostics,
}

impl PreparedExternalObservation {
    /// Assignment epoch that owns this observation.
    #[must_use]
    pub const fn lease_id(self) -> u64 {
        self.lease_id
    }

    /// Exact live socket identity that may accept it.
    #[must_use]
    pub const fn connection_id(self) -> u64 {
        self.connection_id
    }

    /// Interactive player or Protocol 2 RL client.
    #[must_use]
    pub const fn kind(self) -> ControllerKind {
        self.kind
    }

    /// Source snake array index.
    #[must_use]
    pub const fn snake_index(self) -> usize {
        self.snake_index
    }

    /// Exact internal snake identity.
    #[must_use]
    pub const fn snake_id(self) -> u64 {
        self.snake_id
    }

    /// Stable-boundary player/RL head position.
    #[must_use]
    pub const fn position(self) -> WorldPoint {
        self.position
    }

    /// Stable-boundary player/RL heading.
    #[must_use]
    pub const fn direction(self) -> f64 {
        self.direction
    }

    /// Marker committed only after matching Node acceptance.
    #[must_use]
    pub const fn delivery(self) -> ObservationDeliveryMarker {
        self.delivery
    }

    /// Corrected sensing work diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> SensorSampleDiagnostics {
        self.diagnostics
    }
}

/// Work counts and retained capacities for one shared control boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlPhaseDiagnostics {
    /// Alive snakes assigned exactly one source.
    pub controls: usize,
    /// Internally consumed baseline observations.
    pub baseline_observations: usize,
    /// Externally delivered observations awaiting Node acceptance.
    pub external_observations: usize,
    /// Complete due neural graph evaluations.
    pub neural_evaluations: usize,
    /// Neural outputs held because cadence was not due.
    pub neural_held: usize,
    /// Explicit external-to-neural transitions in this batch.
    pub neural_takeovers: usize,
    /// Retained complete body-index diagnostics.
    pub body_index: BodyIndexDiagnostics,
    /// Retained complete pellet-index diagnostics.
    pub pellet_index: PelletIndexDiagnostics,
    /// Shared per-sample sensor scratch capacities.
    pub sensor_scratch: SensorScratchDiagnostics,
    /// Coarse neural pipeline capacities.
    pub neural: NeuralControlCapacityDiagnostics,
    /// Stable snake-order capacity.
    pub snake_order_capacity: usize,
    /// Stable brain-order capacity.
    pub brain_order_capacity: usize,
    /// Stable lease-order capacity.
    pub lease_order_capacity: usize,
    /// Chosen-control capacity.
    pub control_capacity: usize,
    /// Lease-proposal capacity.
    pub controller_transition_capacity: usize,
    /// Retained baseline-result record count.
    pub baseline_result_records: usize,
    /// External-event metadata capacity.
    pub external_event_capacity: usize,
    /// Packed external-observation Float32 capacity.
    pub external_observation_capacity: usize,
    /// Neural-candidate capacity.
    pub neural_candidate_capacity: usize,
    /// Takeover reset-handle capacity.
    pub reset_brain_capacity: usize,
}

/// Retained buffers and source work represented by one applied control boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlCommitDiagnostics {
    /// Selection/sensing/inference work that produced the applied boundary.
    pub selection: ControlPhaseDiagnostics,
    /// Retained working snake capacity.
    pub snake_capacity: usize,
    /// Retained working body-point capacity.
    pub body_point_capacity: usize,
    /// Retained working pellet capacity.
    pub pellet_capacity: usize,
    /// Retained controller-lease capacity.
    pub controller_lease_capacity: usize,
    /// Retained brain-record capacity.
    pub brain_capacity: usize,
    /// Immutable non-population weight values copied on this attempt.
    pub brain_weight_values_copied: usize,
    /// Recurrent values copied from the source boundary before publication.
    pub brain_recurrent_values_copied: usize,
    /// Retained baseline-slot capacity.
    pub baseline_slot_capacity: usize,
    /// Retained baseline-RNG capacity.
    pub baseline_rng_capacity: usize,
    /// Retained external-event capacity.
    pub external_event_capacity: usize,
    /// Retained packed external-observation capacity.
    pub external_observation_capacity: usize,
    /// Retained serialized RNG text capacity, including absent Gaussian spares.
    pub rng_text_capacity: usize,
}

/// Complete non-authoritative controller boundary.
#[derive(Debug)]
pub struct PreparedControlPhase<'workspace, 'prefix, 'source> {
    key: PhysicsStepKey,
    calculation_key: CalculationBatchKey,
    prefix: PreparedFixedStepPrefix<'prefix, 'source>,
    generation: &'source SensorGenerationState,
    population: &'source [PopulationGenome],
    brains: &'source [BrainRuntimeState],
    wall_now_ms: u64,
    config: ControlPhaseConfig,
    neural: &'workspace NeuralControlPipeline,
    controls: &'workspace [PreparedControlUpdate],
    controller_transitions: &'workspace [PreparedControllerTransition],
    baseline_controls: &'workspace [PreparedBaselineControl],
    external_events: &'workspace [PreparedExternalObservation],
    external_observations: &'workspace [f32],
    diagnostics: ControlPhaseDiagnostics,
}

impl PreparedControlPhase<'_, '_, '_> {
    /// Complete authority/config/operation identity prepared.
    #[must_use]
    pub const fn key(&self) -> PhysicsStepKey {
        self.key
    }

    /// Neural calculation identity for the next committed step.
    #[must_use]
    pub const fn calculation_key(&self) -> CalculationBatchKey {
        self.calculation_key
    }
}

impl<'workspace, 'prefix, 'source> PreparedControlPhase<'workspace, 'prefix, 'source> {
    /// One exclusive source/action decision per alive snake, in stable-ID order.
    #[must_use]
    pub const fn control_updates(&self) -> &'workspace [PreparedControlUpdate] {
        self.controls
    }

    /// Wall-time lease proposals joined to this boundary.
    #[must_use]
    pub const fn controller_transitions(&self) -> &'workspace [PreparedControllerTransition] {
        self.controller_transitions
    }

    /// Internally consumed baseline results.
    #[must_use]
    pub const fn baseline_controls(&self) -> &'workspace [PreparedBaselineControl] {
        self.baseline_controls
    }

    /// External events awaiting matching thin-bridge acceptance.
    #[must_use]
    pub const fn external_events(&self) -> &'workspace [PreparedExternalObservation] {
        self.external_events
    }

    /// Resolve one event and its packed Float32 observation without copying it.
    #[must_use]
    pub fn external_observation(
        &self,
        event_index: usize,
    ) -> Option<(&'workspace PreparedExternalObservation, &'workspace [f32])> {
        let event = self.external_events.get(event_index)?;
        let end = event.observation_start.checked_add(event.observation_len)?;
        Some((
            event,
            self.external_observations
                .get(event.observation_start..end)?,
        ))
    }

    /// Fixed-step prefix from which every observation/source was derived.
    #[must_use]
    pub const fn prefix(&self) -> PreparedFixedStepPrefix<'prefix, 'source> {
        self.prefix
    }

    /// Exact generation-best state sampled.
    #[must_use]
    pub const fn generation(&self) -> &'source SensorGenerationState {
        self.generation
    }

    /// Exact population source sampled by heterogeneous inference.
    #[must_use]
    pub const fn population(&self) -> &'source [PopulationGenome] {
        self.population
    }

    /// Exact brain/recurrent source sampled by heterogeneous inference.
    #[must_use]
    pub const fn brains(&self) -> &'source [BrainRuntimeState] {
        self.brains
    }

    /// Monotonic wall boundary used for lease selection.
    #[must_use]
    pub const fn wall_now_ms(&self) -> u64 {
        self.wall_now_ms
    }

    /// Exact projected control settings.
    #[must_use]
    pub const fn config(&self) -> ControlPhaseConfig {
        self.config
    }

    /// Work and retained-capacity diagnostics.
    #[must_use]
    pub const fn diagnostics(&self) -> ControlPhaseDiagnostics {
        self.diagnostics
    }
}

/// Complete applied control boundary that remains non-authoritative.
#[derive(Clone, Copy, Debug)]
pub struct PreparedControlCommit<'workspace> {
    key: PhysicsStepKey,
    calculation_key: CalculationBatchKey,
    prefix_config: FixedStepPrefixConfig,
    control_config: ControlPhaseConfig,
    world: &'workspace WorldState,
    rng: &'workspace RngStateBundle,
    allocators: &'workspace AllocatorState,
    lifecycle: &'workspace BaselineLifecycleState,
    brains: &'workspace [BrainRuntimeState],
    sensor_generation: SensorGenerationState,
    generation_elapsed_seconds: f64,
    ambient_accumulator: f64,
    external_events: &'workspace [PreparedExternalObservation],
    external_observations: &'workspace [f32],
    diagnostics: ControlCommitDiagnostics,
}

impl<'workspace> PreparedControlCommit<'workspace> {
    /// Complete authority/config/operation identity prepared.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Exact neural calculation identity committed into this working boundary.
    #[must_use]
    pub const fn calculation_key(self) -> CalculationBatchKey {
        self.calculation_key
    }

    /// Exact pre-control prefix settings used to build this boundary.
    #[must_use]
    pub const fn prefix_config(self) -> FixedStepPrefixConfig {
        self.prefix_config
    }

    /// Exact controller-selection settings used to build this boundary.
    #[must_use]
    pub const fn control_config(self) -> ControlPhaseConfig {
        self.control_config
    }

    /// World after prefix accounting/ambient work and complete control publication.
    #[must_use]
    pub const fn world(self) -> &'workspace WorldState {
        self.world
    }

    /// RNG continuation after ambient and baseline-control draws.
    #[must_use]
    pub const fn rng(self) -> &'workspace RngStateBundle {
        self.rng
    }

    /// Allocator continuation after the fixed-step prefix.
    #[must_use]
    pub const fn allocators(self) -> &'workspace AllocatorState {
        self.allocators
    }

    /// Baseline timers, strategies, actions, and RNG-independent state after control.
    #[must_use]
    pub const fn baseline_lifecycle(self) -> &'workspace BaselineLifecycleState {
        self.lifecycle
    }

    /// Recurrent state after every due neural evaluation.
    #[must_use]
    pub const fn brains(self) -> &'workspace [BrainRuntimeState] {
        self.brains
    }

    /// Generation-best value sampled at this pre-movement boundary.
    #[must_use]
    pub const fn sensor_generation(self) -> SensorGenerationState {
        self.sensor_generation
    }

    /// Generation elapsed seconds after the once-per-step accounting prefix.
    #[must_use]
    pub const fn generation_elapsed_seconds(self) -> f64 {
        self.generation_elapsed_seconds
    }

    /// Fractional ambient-pellet credit after realized prefix spawns.
    #[must_use]
    pub const fn ambient_accumulator(self) -> f64 {
        self.ambient_accumulator
    }

    /// External events that still require matching Node acceptance.
    #[must_use]
    pub const fn external_events(self) -> &'workspace [PreparedExternalObservation] {
        self.external_events
    }

    /// Resolve one retained event and its packed observation without copying it.
    #[must_use]
    pub fn external_observation(
        self,
        event_index: usize,
    ) -> Option<(&'workspace PreparedExternalObservation, &'workspace [f32])> {
        let event = self.external_events.get(event_index)?;
        let end = event.observation_start.checked_add(event.observation_len)?;
        Some((
            event,
            self.external_observations
                .get(event.observation_start..end)?,
        ))
    }

    /// Work counts and retained capacities for the complete control commit.
    #[must_use]
    pub const fn diagnostics(self) -> ControlCommitDiagnostics {
        self.diagnostics
    }
}

/// Reusable working owner that applies one complete control phase without authority writes.
#[derive(Debug, Default)]
pub struct ControlCommitWorkspace {
    world: WorldState,
    rng: Option<RngStateBundle>,
    rng_copy_scratch: RngCopyScratch,
    allocators: Option<AllocatorState>,
    lifecycle: Option<BaselineLifecycleState>,
    brains: Vec<BrainRuntimeState>,
    brain_weight_identity: Option<(u64, u64)>,
    brain_weight_values_copied: usize,
    brain_recurrent_values_copied: usize,
    sensor_generation: SensorGenerationState,
    external_events: Vec<PreparedExternalObservation>,
    external_observations: Vec<f32>,
    ready: bool,
    diagnostics: ControlCommitDiagnostics,
}

impl ControlCommitWorkspace {
    /// Construct empty reusable control-commit scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Copy the prefix into retained scratch and publish every internal control result there.
    ///
    /// Baseline and neural observations are internally consumed, while external
    /// markers stay uncommitted in the returned event batch. Failure may alter
    /// reusable scratch but never any source or authoritative state.
    pub fn prepare<'workspace>(
        &'workspace mut self,
        phase: PreparedControlPhase<'_, '_, '_>,
    ) -> Result<PreparedControlCommit<'workspace>, ControlPhaseError> {
        self.ready = false;
        self.diagnostics = ControlCommitDiagnostics::default();
        let prefix = phase.prefix;
        copy_world_reusing(
            &mut self.world,
            prefix.world(),
            prefix.world().pellets.len(),
        )?;
        copy_rng_bundle_reusing(&mut self.rng, &mut self.rng_copy_scratch, prefix.rng())?;
        copy_lifecycle_reusing(&mut self.lifecycle, prefix.baseline_lifecycle())?;
        match &mut self.allocators {
            Some(current) => current.clone_from(prefix.allocators()),
            None => self.allocators = Some(prefix.allocators().clone()),
        }
        let weight_identity = (phase.key.world_epoch(), phase.key.population_epoch());
        // Authoritative brain weights cannot change within one world/population
        // epoch. Only recurrent blocks are mutable during a fixed step. Reset,
        // New Run, import, or population replacement changes one of these IDs
        // and forces a complete weight copy before reuse is allowed again.
        let reuse_immutable_weights = self.brain_weight_identity == Some(weight_identity);
        if !reuse_immutable_weights {
            self.brain_weight_identity = None;
        }
        let (weight_values, recurrent_values) =
            copy_brains_reusing(&mut self.brains, phase.brains, reuse_immutable_weights)?;
        self.brain_weight_identity = Some(weight_identity);
        self.brain_weight_values_copied = weight_values;
        self.brain_recurrent_values_copied = recurrent_values;
        reserve_for(
            &mut self.external_events,
            phase.external_events.len(),
            "committed external events",
        )?;
        reserve_for(
            &mut self.external_observations,
            phase.external_observations.len(),
            "committed external observations",
        )?;
        self.external_events.clear();
        self.external_events
            .extend_from_slice(phase.external_events);
        self.external_observations.clear();
        self.external_observations
            .extend_from_slice(phase.external_observations);
        self.sensor_generation = *phase.generation;

        self.validate_phase(&phase)?;
        self.copy_baseline_rng_results(&phase)?;
        self.publish_phase(&phase);
        self.ready = true;
        self.diagnostics = self.collect_diagnostics(phase.diagnostics);
        self.prepared(prefix, phase.key, phase.calculation_key, phase.config)
    }

    fn validate_phase(
        &self,
        phase: &PreparedControlPhase<'_, '_, '_>,
    ) -> Result<(), ControlPhaseError> {
        if phase.controls.len() != self.world.snakes.iter().filter(|snake| snake.alive).count() {
            return Err(ControlPhaseError::CommitShapeMismatch {
                field: "one control per alive snake",
            });
        }
        let mut previous_control_id = None;
        for update in phase.controls {
            if previous_control_id.is_some_and(|previous| previous >= update.snake_id) {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "canonical control order",
                });
            }
            previous_control_id = Some(update.snake_id);
            let snake = self.world.snakes.get(update.snake_index).ok_or(
                ControlPhaseError::CommitShapeMismatch {
                    field: "control snake index",
                },
            )?;
            if !snake.alive || snake.id != update.snake_id {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "control snake identity",
                });
            }
            if !update.turn.is_finite()
                || !(-1.0..=1.0).contains(&update.turn)
                || !update.next_control_accumulator_seconds.is_finite()
                || !(0.0..=MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS)
                    .contains(&update.next_control_accumulator_seconds)
            {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "control scalar",
                });
            }
            match update.source {
                SelectedControlSource::NeuralEvaluated
                | SelectedControlSource::NeuralHeld
                | SelectedControlSource::NeuralTakeover => {
                    if snake.brain.is_none() {
                        return Err(ControlPhaseError::CommitShapeMismatch {
                            field: "neural control brain",
                        });
                    }
                }
                SelectedControlSource::Baseline
                | SelectedControlSource::ExternalHeld
                | SelectedControlSource::ExternalReservedNeutral
                | SelectedControlSource::ExternalOnlyNeutral => {
                    if update.next_control_accumulator_seconds.to_bits() != 0.0_f64.to_bits() {
                        return Err(ControlPhaseError::CommitShapeMismatch {
                            field: "non-neural cadence",
                        });
                    }
                }
            }
            if update.source == SelectedControlSource::NeuralHeld
                && (snake.turn.to_bits() != update.turn.to_bits()
                    || snake.input_boost != update.boost)
            {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "held neural action",
                });
            }
        }

        let mut previous_transition_id = None;
        for transition in phase.controller_transitions {
            let lease = self
                .world
                .controller_leases
                .get(transition.lease_index)
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "controller lease index",
                })?;
            let snake = self.world.snakes.get(transition.snake_index).ok_or(
                ControlPhaseError::CommitShapeMismatch {
                    field: "controller snake index",
                },
            )?;
            if previous_transition_id.is_some_and(|previous| previous >= snake.id) {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "canonical controller-transition order",
                });
            }
            previous_transition_id = Some(snake.id);
            validate_controller_boundary(lease, snake, transition.proposal)?;
            let update = find_control(phase.controls, snake.id)?;
            validate_transition_control(transition.proposal, update)?;
        }

        let mut previous_baseline_id = None;
        for result in phase.baseline_controls {
            if previous_baseline_id.is_some_and(|previous| previous >= result.snake_id) {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "canonical baseline-control order",
                });
            }
            previous_baseline_id = Some(result.snake_id);
            let snake = self.world.snakes.get(result.snake_index).ok_or(
                ControlPhaseError::CommitShapeMismatch {
                    field: "baseline snake index",
                },
            )?;
            let slot_index = usize::try_from(result.slot).map_err(|_| {
                ControlPhaseError::ArithmeticOverflow {
                    context: "baseline control slot index",
                }
            })?;
            let lifecycle = self
                .lifecycle
                .as_ref()
                .and_then(|state| state.slots.get(slot_index))
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "baseline lifecycle slot",
                })?;
            let baseline_rng = self
                .rng
                .as_ref()
                .and_then(|rng| rng.baselines.get(slot_index))
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "baseline RNG slot",
                })?;
            if snake.id != result.snake_id
                || snake.kind != SnakeKind::Baseline
                || snake.baseline_slot != Some(result.slot)
                || lifecycle.slot != result.slot
                || lifecycle.snake_id != result.snake_id
                || baseline_rng.slot != result.slot
                || result.next_slot.slot != result.slot
                || result.next_slot.snake_id != result.snake_id
                || result.next_slot.respawn_remaining_seconds.is_some()
                || result.next_rng.slot != result.slot
            {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "baseline result identity",
                });
            }
            validate_strategy_runtime(result.next_slot, result.next_strategy)?;
            result.delivery.validate(snake)?;
            let update = find_control(phase.controls, result.snake_id)?;
            if update.source != SelectedControlSource::Baseline
                || update.turn.to_bits() != result.next_slot.turn.to_bits()
                || update.boost != result.next_slot.boost
            {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "baseline action result",
                });
            }
        }

        let mut observation_end = 0usize;
        let mut previous_external_id = None;
        for event in phase.external_events {
            if previous_external_id.is_some_and(|previous| previous >= event.snake_id) {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "canonical external-event order",
                });
            }
            previous_external_id = Some(event.snake_id);
            let end = event
                .observation_start
                .checked_add(event.observation_len)
                .ok_or(ControlPhaseError::ArithmeticOverflow {
                    context: "external observation range",
                })?;
            let snake = self.world.snakes.get(event.snake_index).ok_or(
                ControlPhaseError::CommitShapeMismatch {
                    field: "external event snake index",
                },
            )?;
            let lease = self.world.controller_leases.get(event.lease_index).ok_or(
                ControlPhaseError::CommitShapeMismatch {
                    field: "external event lease index",
                },
            )?;
            if event.observation_start != observation_end
                || end > phase.external_observations.len()
                || event.observation_len != phase.neural.inference().input_size()
                || snake.id != event.snake_id
                || snake.kind != SnakeKind::External
                || lease.id != event.lease_id
                || lease.snake_id != event.snake_id
                || lease.connection_id != Some(event.connection_id)
                || lease.kind != event.kind
                || lease.status != ControllerLeaseStatus::Connected
            {
                return Err(ControlPhaseError::CommitShapeMismatch {
                    field: "external observation result",
                });
            }
            event.delivery.validate(snake)?;
            observation_end = end;
        }
        if observation_end != phase.external_observations.len() {
            return Err(ControlPhaseError::CommitShapeMismatch {
                field: "packed external observation length",
            });
        }

        phase
            .neural
            .validate_state_commit(phase.calculation_key, &self.world, &self.brains)?;
        Ok(())
    }

    fn copy_baseline_rng_results(
        &mut self,
        phase: &PreparedControlPhase<'_, '_, '_>,
    ) -> Result<(), ControlPhaseError> {
        let rng = self
            .rng
            .as_mut()
            .ok_or(ControlPhaseError::CommitShapeMismatch {
                field: "working RNG bundle",
            })?;
        for result in phase.baseline_controls {
            let slot_index = usize::try_from(result.slot).map_err(|_| {
                ControlPhaseError::ArithmeticOverflow {
                    context: "baseline RNG result slot index",
                }
            })?;
            let target = rng.baselines.get_mut(slot_index).ok_or(
                ControlPhaseError::CommitShapeMismatch {
                    field: "working baseline RNG slot",
                },
            )?;
            let spare = self
                .rng_copy_scratch
                .baseline_gaussian_spares
                .get_mut(slot_index)
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "baseline RNG copy scratch",
                })?;
            copy_serialized_rng_reusing(&mut target.state, &result.next_rng.state, spare)?;
        }
        Ok(())
    }

    fn publish_phase(&mut self, phase: &PreparedControlPhase<'_, '_, '_>) {
        for transition in phase.controller_transitions {
            let lease = &mut self.world.controller_leases[transition.lease_index];
            let snake = &mut self.world.snakes[transition.snake_index];
            commit_controller_boundary_prevalidated(lease, snake, transition.proposal);
        }
        for result in phase.baseline_controls {
            let slot_index =
                usize::try_from(result.slot).expect("prevalidated baseline slot must fit usize");
            self.lifecycle
                .as_mut()
                .expect("prevalidated lifecycle must exist")
                .slots[slot_index] = result.next_slot;
            let snake = &mut self.world.snakes[result.snake_index];
            snake.baseline_strategy = Some(result.next_strategy);
            result.delivery.commit_prevalidated(snake);
        }
        phase
            .neural
            .commit_state_prevalidated(&mut self.world, &mut self.brains);
        for update in phase.controls {
            let snake = &mut self.world.snakes[update.snake_index];
            match update.source {
                SelectedControlSource::ExternalHeld
                | SelectedControlSource::ExternalReservedNeutral
                | SelectedControlSource::NeuralHeld => {}
                SelectedControlSource::NeuralTakeover => {
                    snake.turn = update.turn;
                    snake.input_boost = update.boost;
                }
                SelectedControlSource::Baseline
                | SelectedControlSource::ExternalOnlyNeutral
                | SelectedControlSource::NeuralEvaluated => {
                    snake.previous_turn = snake.turn;
                    snake.previous_input_boost = snake.input_boost;
                    snake.turn = update.turn;
                    snake.input_boost = update.boost;
                }
            }
            snake.control_accumulator_seconds = update.next_control_accumulator_seconds;
        }
    }

    fn collect_diagnostics(&self, selection: ControlPhaseDiagnostics) -> ControlCommitDiagnostics {
        ControlCommitDiagnostics {
            selection,
            snake_capacity: self.world.snakes.capacity(),
            body_point_capacity: self.world.body_points.capacity(),
            pellet_capacity: self.world.pellets.capacity(),
            controller_lease_capacity: self.world.controller_leases.capacity(),
            brain_capacity: self.brains.capacity(),
            brain_weight_values_copied: self.brain_weight_values_copied,
            brain_recurrent_values_copied: self.brain_recurrent_values_copied,
            baseline_slot_capacity: self
                .lifecycle
                .as_ref()
                .map_or(0, |state| state.slots.capacity()),
            baseline_rng_capacity: self.rng.as_ref().map_or(0, |rng| rng.baselines.capacity()),
            external_event_capacity: self.external_events.capacity(),
            external_observation_capacity: self.external_observations.capacity(),
            rng_text_capacity: rng_text_capacity(self.rng.as_ref(), &self.rng_copy_scratch),
        }
    }

    fn prepared<'workspace>(
        &'workspace self,
        prefix: PreparedFixedStepPrefix<'_, '_>,
        key: PhysicsStepKey,
        calculation_key: CalculationBatchKey,
        phase_config: ControlPhaseConfig,
    ) -> Result<PreparedControlCommit<'workspace>, ControlPhaseError> {
        if !self.ready {
            return Err(ControlPhaseError::ResultNotReady);
        }
        Ok(PreparedControlCommit {
            key,
            calculation_key,
            prefix_config: prefix.config(),
            control_config: phase_config,
            world: &self.world,
            rng: self
                .rng
                .as_ref()
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "working RNG bundle",
                })?,
            allocators: self
                .allocators
                .as_ref()
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "working allocators",
                })?,
            lifecycle: self
                .lifecycle
                .as_ref()
                .ok_or(ControlPhaseError::CommitShapeMismatch {
                    field: "working baseline lifecycle",
                })?,
            brains: &self.brains,
            sensor_generation: self.sensor_generation,
            generation_elapsed_seconds: prefix.generation_elapsed_seconds(),
            ambient_accumulator: prefix.ambient_accumulator(),
            external_events: &self.external_events,
            external_observations: &self.external_observations,
            diagnostics: self.diagnostics,
        })
    }

    /// Whether the latest attempt produced one complete applied control boundary.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Latest retained capacities, including after failure.
    #[must_use]
    pub fn diagnostics(&self) -> ControlCommitDiagnostics {
        if self.ready {
            self.diagnostics
        } else {
            self.collect_diagnostics(ControlPhaseDiagnostics::default())
        }
    }
}

/// Reusable owner of shared sensing and exclusive control-source staging.
#[derive(Debug)]
pub struct ControlPhaseWorkspace {
    neural: NeuralControlPipeline,
    baseline: BaselineControlWorkspace,
    sensor_scratch: SensorScratch,
    body_index: BodySpatialIndex,
    pellet_index: PelletSpatialIndex,
    baseline_observation: Vec<f32>,
    external_observations: Vec<f32>,
    controls: Vec<PreparedControlUpdate>,
    controller_transitions: Vec<PreparedControllerTransition>,
    baseline_controls: Vec<PreparedBaselineControl>,
    baseline_control_count: usize,
    external_events: Vec<PreparedExternalObservation>,
    neural_candidates: Vec<CalculationCandidateIndex>,
    reset_brains: Vec<BrainHandle>,
    snake_order: Vec<usize>,
    brain_order: Vec<usize>,
    lease_order: Vec<usize>,
    ready: bool,
    diagnostics: ControlPhaseDiagnostics,
}

impl ControlPhaseWorkspace {
    /// Construct empty retained join scratch around one admitted neural pipeline.
    #[must_use]
    pub fn new(neural: NeuralControlPipeline) -> Self {
        Self {
            neural,
            baseline: BaselineControlWorkspace::new(),
            sensor_scratch: SensorScratch::default(),
            body_index: BodySpatialIndex::empty(),
            pellet_index: PelletSpatialIndex::empty(),
            baseline_observation: Vec::new(),
            external_observations: Vec::new(),
            controls: Vec::new(),
            controller_transitions: Vec::new(),
            baseline_controls: Vec::new(),
            baseline_control_count: 0,
            external_events: Vec::new(),
            neural_candidates: Vec::new(),
            reset_brains: Vec::new(),
            snake_order: Vec::new(),
            brain_order: Vec::new(),
            lease_order: Vec::new(),
            ready: false,
            diagnostics: ControlPhaseDiagnostics::default(),
        }
    }

    /// Build one shared observation boundary and every exclusive source decision.
    pub fn prepare<'workspace, 'prefix, 'source>(
        &'workspace mut self,
        inputs: ControlPhaseInputs<'prefix, 'source>,
    ) -> Result<PreparedControlPhase<'workspace, 'prefix, 'source>, ControlPhaseError> {
        self.clear_active();
        inputs.config.validate()?;
        let prefix_config = inputs.prefix.config();
        if prefix_config.maximum_snakes != inputs.config.maximum_snakes {
            return Err(ControlPhaseError::PrefixConfigMismatch {
                field: "maximum_snakes",
            });
        }
        let world = inputs.prefix.world();
        if world.snakes.len() > inputs.config.maximum_snakes {
            return Err(ControlPhaseError::CapacityExceeded {
                buffer: "world snakes",
                required: world.snakes.len(),
                maximum: inputs.config.maximum_snakes,
            });
        }
        if inputs.brains.len() > inputs.config.maximum_brains {
            return Err(ControlPhaseError::CapacityExceeded {
                buffer: "brain records",
                required: inputs.brains.len(),
                maximum: inputs.config.maximum_brains,
            });
        }
        if self.neural.sensor().layout().input_size != self.neural.inference().input_size() {
            return Err(ControlPhaseError::InternalShapeMismatch {
                field: "sensor/neural input width",
            });
        }
        prepare_orders(
            world,
            inputs.brains,
            &mut self.snake_order,
            &mut self.brain_order,
            &mut self.lease_order,
            inputs.config,
        )?;
        reserve_for(
            &mut self.baseline_observation,
            self.neural.sensor().layout().input_size,
            "baseline observation",
        )?;
        self.baseline_observation
            .resize(self.neural.sensor().layout().input_size, 0.0);
        let maximum_external_values = inputs
            .config
            .maximum_external_observations
            .checked_mul(self.neural.sensor().layout().input_size)
            .ok_or(ControlPhaseError::ArithmeticOverflow {
                context: "maximum packed external observations",
            })?;
        reserve_for(
            &mut self.external_observations,
            maximum_external_values,
            "external observation values",
        )?;

        self.body_index.rebuild(
            world,
            inputs.config.sensor_index.body_cell_size,
            inputs.config.sensor_index.maximum_body_entries,
        )?;
        self.pellet_index.rebuild(
            world,
            inputs.config.sensor_index.pellet_cell_size,
            inputs.config.sensor_index.maximum_pellet_entries,
        )?;
        let indexed = IndexedSensorWorld::from_indexes(
            world,
            std::mem::replace(&mut self.body_index, BodySpatialIndex::empty()),
            std::mem::replace(&mut self.pellet_index, PelletSpatialIndex::empty()),
        );
        let prepare_result = self.prepare_indexed(&inputs, &indexed);
        let (body_index, pellet_index) = indexed.into_indexes();
        self.body_index = body_index;
        self.pellet_index = pellet_index;
        let calculation_key = prepare_result?;
        self.ready = true;
        self.diagnostics = self.collect_diagnostics();
        self.prepared(inputs, calculation_key)
    }

    fn prepare_indexed(
        &mut self,
        inputs: &ControlPhaseInputs<'_, '_>,
        indexed: &IndexedSensorWorld<'_>,
    ) -> Result<CalculationBatchKey, ControlPhaseError> {
        let world = inputs.prefix.world();
        let fixed_dt = inputs.prefix.config().fixed_dt;
        let target_step = inputs
            .prefix
            .key()
            .source_completed_step()
            .checked_add(1)
            .ok_or(ControlPhaseError::ArithmeticOverflow {
                context: "target fixed-step identity",
            })?;
        let calculation_key = CalculationBatchKey::new(
            inputs.prefix.key().generation(),
            target_step,
            inputs.prefix.key().population_epoch(),
        );

        for order_position in 0..self.snake_order.len() {
            let snake_index = self.snake_order[order_position];
            let snake = &world.snakes[snake_index];
            if !snake.alive {
                continue;
            }
            match snake.kind {
                SnakeKind::Baseline => {
                    self.prepare_baseline(inputs, indexed, snake_index)?;
                }
                SnakeKind::External => {
                    self.prepare_external(inputs, indexed, snake_index, fixed_dt)?;
                }
                SnakeKind::Evolved | SnakeKind::Resurrected => {
                    self.prepare_neural(inputs, snake_index, fixed_dt, false)?;
                }
            }
        }

        let alive = world.snakes.iter().filter(|snake| snake.alive).count();
        if self.controls.len() != alive {
            return Err(ControlPhaseError::InternalShapeMismatch {
                field: "one control per alive snake",
            });
        }

        self.reset_brains.sort_unstable();
        let neural_output_size = self.neural.inference().output_size();
        let batch = self.neural.prepare_and_evaluate(
            NeuralControlBatchInputs {
                key: calculation_key,
                candidates: &self.neural_candidates,
                indexed_world: indexed,
                generation: inputs.generation,
                population: inputs.population,
                brains: inputs.brains,
                reset_brains: &self.reset_brains,
            },
            &mut self.sensor_scratch,
        )?;
        for (ordinal, unit) in batch.work().iter().copied().enumerate() {
            let output_offset = ordinal.checked_mul(neural_output_size).ok_or(
                ControlPhaseError::ArithmeticOverflow {
                    context: "neural output offset",
                },
            )?;
            let turn = batch.outputs()[output_offset].clamp(-1.0, 1.0);
            let boost =
                batch.outputs()[output_offset + 1].clamp(-1.0, 1.0) > NEURAL_BOOST_THRESHOLD;
            let update_index = self
                .controls
                .binary_search_by_key(&unit.snake_id(), |update| update.snake_id)
                .map_err(|_| ControlPhaseError::NeuralControlMissing {
                    snake_id: unit.snake_id(),
                })?;
            let update = &mut self.controls[update_index];
            if !matches!(
                update.source,
                SelectedControlSource::NeuralEvaluated | SelectedControlSource::NeuralTakeover
            ) {
                return Err(ControlPhaseError::NeuralControlMissing {
                    snake_id: unit.snake_id(),
                });
            }
            update.turn = turn;
            update.boost = boost;
        }
        Ok(calculation_key)
    }

    fn prepare_baseline(
        &mut self,
        inputs: &ControlPhaseInputs<'_, '_>,
        indexed: &IndexedSensorWorld<'_>,
        snake_index: usize,
    ) -> Result<(), ControlPhaseError> {
        let world = inputs.prefix.world();
        let snake = &world.snakes[snake_index];
        let slot = snake
            .baseline_slot
            .ok_or(ControlPhaseError::BaselineMapping {
                snake_id: snake.id,
                field: "slot",
            })?;
        let slot_index = usize::try_from(slot).map_err(|_| ControlPhaseError::BaselineMapping {
            snake_id: snake.id,
            field: "slot index",
        })?;
        let source_slot = inputs
            .prefix
            .baseline_lifecycle()
            .slots
            .get(slot_index)
            .filter(|runtime| runtime.slot == slot)
            .ok_or(ControlPhaseError::BaselineMapping {
                snake_id: snake.id,
                field: "lifecycle",
            })?;
        let source_rng = inputs
            .prefix
            .rng()
            .baselines
            .get(slot_index)
            .filter(|rng| rng.slot == slot)
            .ok_or(ControlPhaseError::BaselineMapping {
                snake_id: snake.id,
                field: "RNG",
            })?;
        let sample = self.neural.sensor().sample(
            indexed,
            inputs.generation,
            snake_index,
            &mut self.baseline_observation,
            &mut self.sensor_scratch,
        )?;
        let evaluation = self.baseline.prepare(
            world,
            snake_index,
            source_slot,
            source_rng,
            &self.baseline_observation,
            self.neural.sensor().layout(),
            inputs.config.baseline,
            inputs.prefix.config().fixed_dt,
        )?;
        let next_slot = evaluation.next_slot();
        let next_strategy = evaluation.next_strategy();
        let diagnostics = evaluation.diagnostics();
        let result_index = self.baseline_control_count;
        if result_index < self.baseline_controls.len() {
            let result = &mut self.baseline_controls[result_index];
            result.snake_index = snake_index;
            result.snake_id = snake.id;
            result.slot = slot;
            result.next_slot = next_slot;
            result.next_strategy = next_strategy;
            result.next_rng.clone_from(evaluation.next_rng());
            result.delivery = sample.delivery;
            result.diagnostics = diagnostics;
            result.sensor_diagnostics = sample.diagnostics;
        } else {
            reserve_for(
                &mut self.baseline_controls,
                result_index.saturating_add(1),
                "baseline control results",
            )?;
            self.baseline_controls.push(PreparedBaselineControl {
                snake_index,
                snake_id: snake.id,
                slot,
                next_slot,
                next_strategy,
                next_rng: evaluation.next_rng().clone(),
                delivery: sample.delivery,
                diagnostics,
                sensor_diagnostics: sample.diagnostics,
            });
        }
        self.baseline_control_count += 1;
        self.push_control(
            PreparedControlUpdate {
                snake_index,
                snake_id: snake.id,
                source: SelectedControlSource::Baseline,
                turn: next_slot.turn,
                boost: next_slot.boost,
                next_control_accumulator_seconds: 0.0,
            },
            inputs.config.maximum_snakes,
        )
    }

    fn prepare_external(
        &mut self,
        inputs: &ControlPhaseInputs<'_, '_>,
        indexed: &IndexedSensorWorld<'_>,
        snake_index: usize,
        fixed_dt: f64,
    ) -> Result<(), ControlPhaseError> {
        let world = inputs.prefix.world();
        let snake = &world.snakes[snake_index];
        let Some(lease_index) = find_lease_index(world, &self.lease_order, snake.id) else {
            return self.push_control(
                PreparedControlUpdate {
                    snake_index,
                    snake_id: snake.id,
                    source: SelectedControlSource::ExternalOnlyNeutral,
                    turn: 0.0,
                    boost: false,
                    next_control_accumulator_seconds: 0.0,
                },
                inputs.config.maximum_snakes,
            );
        };
        let lease = &world.controller_leases[lease_index];
        let proposal = prepare_controller_boundary(
            lease,
            snake,
            inputs.wall_now_ms,
            inputs.config.controller_timing,
        )?;
        let required_transitions = self.controller_transitions.len().saturating_add(1);
        reserve_for(
            &mut self.controller_transitions,
            required_transitions,
            "controller transitions",
        )?;
        self.controller_transitions
            .push(PreparedControllerTransition {
                lease_index,
                snake_index,
                proposal,
            });

        if lease.status == ControllerLeaseStatus::Connected {
            self.prepare_external_observation(inputs, indexed, snake_index, lease_index)?;
        }
        match proposal.source() {
            ExternalControlSource::HeldAction { turn, boost } => self.push_control(
                PreparedControlUpdate {
                    snake_index,
                    snake_id: snake.id,
                    source: SelectedControlSource::ExternalHeld,
                    turn,
                    boost,
                    next_control_accumulator_seconds: 0.0,
                },
                inputs.config.maximum_snakes,
            ),
            ExternalControlSource::ReservedNeutral => self.push_control(
                PreparedControlUpdate {
                    snake_index,
                    snake_id: snake.id,
                    source: SelectedControlSource::ExternalReservedNeutral,
                    turn: 0.0,
                    boost: false,
                    next_control_accumulator_seconds: 0.0,
                },
                inputs.config.maximum_snakes,
            ),
            ExternalControlSource::NeuralTakeover => self.prepare_neural(
                inputs,
                snake_index,
                fixed_dt,
                proposal.begins_neural_takeover(),
            ),
        }
    }

    fn prepare_external_observation(
        &mut self,
        inputs: &ControlPhaseInputs<'_, '_>,
        indexed: &IndexedSensorWorld<'_>,
        snake_index: usize,
        lease_index: usize,
    ) -> Result<(), ControlPhaseError> {
        if self.external_events.len() >= inputs.config.maximum_external_observations {
            return Err(ControlPhaseError::CapacityExceeded {
                buffer: "external observations",
                required: self.external_events.len().saturating_add(1),
                maximum: inputs.config.maximum_external_observations,
            });
        }
        let world = inputs.prefix.world();
        let snake = &world.snakes[snake_index];
        let lease = &world.controller_leases[lease_index];
        let connection_id = lease
            .connection_id
            .ok_or(ControlPhaseError::ExternalMapping {
                snake_id: snake.id,
                field: "connected socket",
            })?;
        let observation_start = self.external_observations.len();
        let observation_len = self.neural.sensor().layout().input_size;
        let end = observation_start.checked_add(observation_len).ok_or(
            ControlPhaseError::ArithmeticOverflow {
                context: "packed external observation length",
            },
        )?;
        self.external_observations.resize(end, 0.0);
        let sample = self.neural.sensor().sample(
            indexed,
            inputs.generation,
            snake_index,
            &mut self.external_observations[observation_start..end],
            &mut self.sensor_scratch,
        )?;
        let required_events = self.external_events.len().saturating_add(1);
        reserve_for(
            &mut self.external_events,
            required_events,
            "external observation events",
        )?;
        self.external_events.push(PreparedExternalObservation {
            lease_id: lease.id,
            connection_id,
            kind: lease.kind,
            lease_index,
            snake_index,
            snake_id: snake.id,
            position: snake.position,
            direction: snake.direction,
            observation_start,
            observation_len,
            delivery: sample.delivery,
            diagnostics: sample.diagnostics,
        });
        Ok(())
    }

    fn prepare_neural(
        &mut self,
        inputs: &ControlPhaseInputs<'_, '_>,
        snake_index: usize,
        fixed_dt: f64,
        force_due: bool,
    ) -> Result<(), ControlPhaseError> {
        let snake = &inputs.prefix.world().snakes[snake_index];
        let brain = snake
            .brain
            .ok_or(ControlPhaseError::MissingBrain { snake_id: snake.id })?;
        let brain_index = find_brain_index(inputs.brains, &self.brain_order, brain).ok_or(
            ControlPhaseError::UnknownBrain {
                snake_id: snake.id,
                brain,
            },
        )?;
        let (due, next_accumulator) = next_neural_boundary(
            snake.control_accumulator_seconds,
            fixed_dt,
            inputs.config.neural_control_interval_seconds,
            force_due,
        )?;
        let source = if due {
            let required_candidates = self.neural_candidates.len().saturating_add(1);
            reserve_for(
                &mut self.neural_candidates,
                required_candidates,
                "neural candidates",
            )?;
            self.neural_candidates
                .push(CalculationCandidateIndex::new(snake_index, brain_index));
            if force_due {
                let required_resets = self.reset_brains.len().saturating_add(1);
                reserve_for(
                    &mut self.reset_brains,
                    required_resets,
                    "takeover reset brains",
                )?;
                self.reset_brains.push(brain);
                SelectedControlSource::NeuralTakeover
            } else {
                SelectedControlSource::NeuralEvaluated
            }
        } else {
            SelectedControlSource::NeuralHeld
        };
        self.push_control(
            PreparedControlUpdate {
                snake_index,
                snake_id: snake.id,
                source,
                turn: snake.turn,
                boost: snake.input_boost,
                next_control_accumulator_seconds: next_accumulator,
            },
            inputs.config.maximum_snakes,
        )
    }

    fn push_control(
        &mut self,
        update: PreparedControlUpdate,
        maximum: usize,
    ) -> Result<(), ControlPhaseError> {
        if self.controls.len() >= maximum {
            return Err(ControlPhaseError::CapacityExceeded {
                buffer: "control updates",
                required: self.controls.len().saturating_add(1),
                maximum,
            });
        }
        let required_controls = self.controls.len().saturating_add(1);
        reserve_for(&mut self.controls, required_controls, "control updates")?;
        self.controls.push(update);
        Ok(())
    }

    fn collect_diagnostics(&self) -> ControlPhaseDiagnostics {
        ControlPhaseDiagnostics {
            controls: self.controls.len(),
            baseline_observations: self.baseline_control_count,
            external_observations: self.external_events.len(),
            neural_evaluations: self.neural_candidates.len(),
            neural_held: self
                .controls
                .iter()
                .filter(|update| update.source == SelectedControlSource::NeuralHeld)
                .count(),
            neural_takeovers: self
                .controls
                .iter()
                .filter(|update| update.source == SelectedControlSource::NeuralTakeover)
                .count(),
            body_index: self.body_index.diagnostics(),
            pellet_index: self.pellet_index.diagnostics(),
            sensor_scratch: self.sensor_scratch.diagnostics(),
            neural: self.neural.capacity_diagnostics(),
            snake_order_capacity: self.snake_order.capacity(),
            brain_order_capacity: self.brain_order.capacity(),
            lease_order_capacity: self.lease_order.capacity(),
            control_capacity: self.controls.capacity(),
            controller_transition_capacity: self.controller_transitions.capacity(),
            baseline_result_records: self.baseline_controls.len(),
            external_event_capacity: self.external_events.capacity(),
            external_observation_capacity: self.external_observations.capacity(),
            neural_candidate_capacity: self.neural_candidates.capacity(),
            reset_brain_capacity: self.reset_brains.capacity(),
        }
    }

    fn prepared<'workspace, 'prefix, 'source>(
        &'workspace self,
        inputs: ControlPhaseInputs<'prefix, 'source>,
        calculation_key: CalculationBatchKey,
    ) -> Result<PreparedControlPhase<'workspace, 'prefix, 'source>, ControlPhaseError> {
        if !self.ready {
            return Err(ControlPhaseError::ResultNotReady);
        }
        Ok(PreparedControlPhase {
            key: inputs.prefix.key(),
            calculation_key,
            prefix: inputs.prefix,
            generation: inputs.generation,
            population: inputs.population,
            brains: inputs.brains,
            wall_now_ms: inputs.wall_now_ms,
            config: inputs.config,
            neural: &self.neural,
            controls: &self.controls,
            controller_transitions: &self.controller_transitions,
            baseline_controls: &self.baseline_controls[..self.baseline_control_count],
            external_events: &self.external_events,
            external_observations: &self.external_observations,
            diagnostics: self.diagnostics,
        })
    }

    fn clear_active(&mut self) {
        self.ready = false;
        self.neural.discard();
        self.baseline_control_count = 0;
        self.external_observations.clear();
        self.controls.clear();
        self.controller_transitions.clear();
        self.external_events.clear();
        self.neural_candidates.clear();
        self.reset_brains.clear();
        self.snake_order.clear();
        self.brain_order.clear();
        self.lease_order.clear();
        self.diagnostics = ControlPhaseDiagnostics::default();
    }

    /// Whether the latest attempt completed every control source.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Latest work/capacity diagnostics, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> ControlPhaseDiagnostics {
        if self.ready {
            self.diagnostics
        } else {
            self.collect_diagnostics()
        }
    }
}

fn prepare_orders(
    world: &WorldState,
    brains: &[BrainRuntimeState],
    snake_order: &mut Vec<usize>,
    brain_order: &mut Vec<usize>,
    lease_order: &mut Vec<usize>,
    config: ControlPhaseConfig,
) -> Result<(), ControlPhaseError> {
    reserve_for(snake_order, world.snakes.len(), "control snake order")?;
    snake_order.extend(0..world.snakes.len());
    snake_order.sort_unstable_by_key(|index| world.snakes[*index].id);
    for pair in snake_order.windows(2) {
        if world.snakes[pair[0]].id == world.snakes[pair[1]].id {
            return Err(ControlPhaseError::DuplicateSnakeId(
                world.snakes[pair[0]].id,
            ));
        }
    }
    if snake_order
        .first()
        .is_some_and(|index| world.snakes[*index].id == 0)
    {
        return Err(ControlPhaseError::DuplicateSnakeId(0));
    }

    reserve_for(brain_order, brains.len(), "control brain order")?;
    brain_order.extend(0..brains.len());
    brain_order.sort_unstable_by_key(|index| brains[*index].handle);
    for pair in brain_order.windows(2) {
        if brains[pair[0]].handle == brains[pair[1]].handle {
            return Err(ControlPhaseError::DuplicateBrain(brains[pair[0]].handle));
        }
    }

    if world.controller_leases.len() > config.maximum_snakes {
        return Err(ControlPhaseError::CapacityExceeded {
            buffer: "controller leases",
            required: world.controller_leases.len(),
            maximum: config.maximum_snakes,
        });
    }
    reserve_for(
        lease_order,
        world.controller_leases.len(),
        "control lease order",
    )?;
    lease_order.extend(0..world.controller_leases.len());
    lease_order.sort_unstable_by_key(|index| world.controller_leases[*index].snake_id);
    for pair in lease_order.windows(2) {
        if world.controller_leases[pair[0]].snake_id == world.controller_leases[pair[1]].snake_id {
            return Err(ControlPhaseError::DuplicateControllerSnake(
                world.controller_leases[pair[0]].snake_id,
            ));
        }
    }
    Ok(())
}

fn find_lease_index(world: &WorldState, order: &[usize], snake_id: u64) -> Option<usize> {
    order
        .binary_search_by_key(&snake_id, |index| world.controller_leases[*index].snake_id)
        .ok()
        .map(|position| order[position])
}

fn find_brain_index(
    brains: &[BrainRuntimeState],
    order: &[usize],
    handle: BrainHandle,
) -> Option<usize> {
    order
        .binary_search_by_key(&handle, |index| brains[*index].handle)
        .ok()
        .map(|position| order[position])
}

fn find_control(
    controls: &[PreparedControlUpdate],
    snake_id: u64,
) -> Result<&PreparedControlUpdate, ControlPhaseError> {
    controls
        .binary_search_by_key(&snake_id, |update| update.snake_id)
        .ok()
        .and_then(|index| controls.get(index))
        .ok_or(ControlPhaseError::CommitShapeMismatch {
            field: "control result lookup",
        })
}

fn validate_transition_control(
    proposal: ControllerBoundaryProposal,
    update: &PreparedControlUpdate,
) -> Result<(), ControlPhaseError> {
    let matches = match proposal.source() {
        ExternalControlSource::HeldAction { turn, boost } => {
            update.source == SelectedControlSource::ExternalHeld
                && update.turn.to_bits() == turn.to_bits()
                && update.boost == boost
        }
        ExternalControlSource::ReservedNeutral => {
            update.source == SelectedControlSource::ExternalReservedNeutral
                && update.turn.to_bits() == 0.0_f32.to_bits()
                && !update.boost
        }
        ExternalControlSource::NeuralTakeover if proposal.begins_neural_takeover() => {
            update.source == SelectedControlSource::NeuralTakeover
        }
        ExternalControlSource::NeuralTakeover => matches!(
            update.source,
            SelectedControlSource::NeuralEvaluated | SelectedControlSource::NeuralHeld
        ),
    };
    if matches {
        Ok(())
    } else {
        Err(ControlPhaseError::CommitShapeMismatch {
            field: "controller transition/control result",
        })
    }
}

fn copy_brains_reusing(
    target: &mut Vec<BrainRuntimeState>,
    source: &[BrainRuntimeState],
    reuse_immutable_weights: bool,
) -> Result<(usize, usize), ControlPhaseError> {
    reserve_for(target, source.len(), "working brain records")?;
    let mut weight_values = 0usize;
    let mut recurrent_values = 0usize;
    let common = target.len().min(source.len());
    for index in 0..common {
        let same_immutable_identity = reuse_immutable_weights
            && target[index].handle == source[index].handle
            && target[index].owner == source[index].owner;
        target[index].handle = source[index].handle;
        target[index].owner = source[index].owner;
        weight_values = weight_values
            .checked_add(copy_optional_f32_box(
                &mut target[index].non_population_weights,
                source[index].non_population_weights.as_deref(),
                "working non-population weights",
                same_immutable_identity,
            )?)
            .ok_or(ControlPhaseError::ArithmeticOverflow {
                context: "copied brain weight values",
            })?;
        copy_f32_box(
            &mut target[index].recurrent,
            &source[index].recurrent,
            "working recurrent state",
        )?;
        recurrent_values = recurrent_values
            .checked_add(source[index].recurrent.len())
            .ok_or(ControlPhaseError::ArithmeticOverflow {
                context: "copied recurrent values",
            })?;
    }
    for brain in &source[common..] {
        let non_population_weights = match brain.non_population_weights.as_deref() {
            Some(weights) => {
                weight_values = weight_values.checked_add(weights.len()).ok_or(
                    ControlPhaseError::ArithmeticOverflow {
                        context: "copied brain weight values",
                    },
                )?;
                Some(try_f32_box(weights, "working non-population weights")?)
            }
            None => None,
        };
        recurrent_values = recurrent_values.checked_add(brain.recurrent.len()).ok_or(
            ControlPhaseError::ArithmeticOverflow {
                context: "copied recurrent values",
            },
        )?;
        target.push(BrainRuntimeState {
            handle: brain.handle,
            owner: brain.owner,
            non_population_weights,
            recurrent: try_f32_box(&brain.recurrent, "working recurrent state")?,
        });
    }
    target.truncate(source.len());
    Ok((weight_values, recurrent_values))
}

fn copy_optional_f32_box(
    target: &mut Option<Box<[f32]>>,
    source: Option<&[f32]>,
    buffer: &'static str,
    reuse_values: bool,
) -> Result<usize, ControlPhaseError> {
    match (target.as_mut(), source) {
        (Some(current), Some(values)) if current.len() == values.len() && reuse_values => {
            return Ok(0);
        }
        (Some(current), Some(values)) if current.len() == values.len() => {
            current.copy_from_slice(values);
            return Ok(values.len());
        }
        (_, Some(values)) => {
            *target = Some(try_f32_box(values, buffer)?);
            return Ok(values.len());
        }
        (_, None) => *target = None,
    }
    Ok(0)
}

fn copy_f32_box(
    target: &mut Box<[f32]>,
    source: &[f32],
    buffer: &'static str,
) -> Result<(), ControlPhaseError> {
    if target.len() == source.len() {
        target.copy_from_slice(source);
    } else {
        *target = try_f32_box(source, buffer)?;
    }
    Ok(())
}

fn try_f32_box(source: &[f32], buffer: &'static str) -> Result<Box<[f32]>, ControlPhaseError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(source.len())
        .map_err(|_| ControlPhaseError::AllocationFailed {
            buffer,
            required: source.len(),
        })?;
    values.extend_from_slice(source);
    Ok(values.into_boxed_slice())
}

fn next_neural_boundary(
    source_accumulator: f64,
    fixed_dt: f64,
    interval: f64,
    force_due: bool,
) -> Result<(bool, f64), ControlPhaseError> {
    if !source_accumulator.is_finite()
        || !(0.0..=MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS).contains(&source_accumulator)
    {
        return Err(ControlPhaseError::InvalidCadenceState(source_accumulator));
    }
    let first_action_pending =
        source_accumulator.to_bits() == MAXIMUM_NEURAL_CONTROL_INTERVAL_SECONDS.to_bits();
    let base = if force_due || first_action_pending {
        0.0
    } else {
        source_accumulator
    };
    let accumulated = base + fixed_dt;
    if !accumulated.is_finite() {
        return Err(ControlPhaseError::InvalidCadenceState(accumulated));
    }
    let due = force_due || first_action_pending || accumulated >= interval;
    let next = if due {
        accumulated % interval
    } else {
        accumulated
    };
    if !next.is_finite() || next < 0.0 || next >= interval {
        return Err(ControlPhaseError::InvalidCadenceState(next));
    }
    Ok((due, next))
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    buffer: &'static str,
) -> Result<(), ControlPhaseError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| ControlPhaseError::AllocationFailed { buffer, required })?;
    }
    Ok(())
}

/// Shared-control staging, mapping, cadence, or source failure.
#[derive(Debug)]
pub enum ControlPhaseError {
    /// The joined fixed-step prefix could not be copied into working storage.
    Prefix(Box<FixedStepPrefixError>),
    /// Corrected spatial index could not represent the whole boundary.
    Spatial(Box<SpatialIndexError>),
    /// One corrected observation failed.
    Sensor(Box<SensorError>),
    /// Baseline strategy evaluation failed.
    Baseline(Box<BaselineControlError>),
    /// Durable baseline continuation rejected a staged control result.
    BaselineLifecycle(Box<BaselineLifecycleError>),
    /// Wall-time lease selection failed.
    Controller(Box<ControllerError>),
    /// Coarse heterogeneous neural evaluation failed.
    Neural(Box<NeuralControlError>),
    /// Projected settings are invalid.
    InvalidConfig { field: &'static str },
    /// Prefix and control projection disagree.
    PrefixConfigMismatch { field: &'static str },
    /// Checked arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// One retained staging allocation failed.
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    /// A declared staging ceiling would be exceeded.
    CapacityExceeded {
        buffer: &'static str,
        required: usize,
        maximum: usize,
    },
    /// Stable snake identity is duplicated or zero.
    DuplicateSnakeId(u64),
    /// Stable brain identity is duplicated.
    DuplicateBrain(BrainHandle),
    /// More than one lease targets the same snake.
    DuplicateControllerSnake(u64),
    /// A baseline snake does not resolve its dense lifecycle/RNG state.
    BaselineMapping { snake_id: u64, field: &'static str },
    /// A connected external snake has incomplete lease/socket mapping.
    ExternalMapping { snake_id: u64, field: &'static str },
    /// A neural-eligible snake has no brain.
    MissingBrain { snake_id: u64 },
    /// A snake brain handle does not resolve to one runtime record.
    UnknownBrain { snake_id: u64, brain: BrainHandle },
    /// A due neural result did not map back to its staged source decision.
    NeuralControlMissing { snake_id: u64 },
    /// Stored cadence state or derived remainder is invalid.
    InvalidCadenceState(f64),
    /// Internal staged widths or source classes disagree.
    InternalShapeMismatch { field: &'static str },
    /// A complete staged result no longer forms one publishable control boundary.
    CommitShapeMismatch { field: &'static str },
    /// No complete result is available.
    ResultNotReady,
}

impl From<SpatialIndexError> for ControlPhaseError {
    fn from(error: SpatialIndexError) -> Self {
        Self::Spatial(Box::new(error))
    }
}

impl From<FixedStepPrefixError> for ControlPhaseError {
    fn from(error: FixedStepPrefixError) -> Self {
        Self::Prefix(Box::new(error))
    }
}

impl From<SensorError> for ControlPhaseError {
    fn from(error: SensorError) -> Self {
        Self::Sensor(Box::new(error))
    }
}

impl From<BaselineControlError> for ControlPhaseError {
    fn from(error: BaselineControlError) -> Self {
        Self::Baseline(Box::new(error))
    }
}

impl From<BaselineLifecycleError> for ControlPhaseError {
    fn from(error: BaselineLifecycleError) -> Self {
        Self::BaselineLifecycle(Box::new(error))
    }
}

impl From<ControllerError> for ControlPhaseError {
    fn from(error: ControllerError) -> Self {
        Self::Controller(Box::new(error))
    }
}

impl From<NeuralControlError> for ControlPhaseError {
    fn from(error: NeuralControlError) -> Self {
        Self::Neural(Box::new(error))
    }
}

impl Display for ControlPhaseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prefix(error) => write!(formatter, "{error}"),
            Self::Spatial(error) => write!(formatter, "{error}"),
            Self::Sensor(error) => write!(formatter, "{error}"),
            Self::Baseline(error) => write!(formatter, "{error}"),
            Self::BaselineLifecycle(error) => write!(formatter, "{error}"),
            Self::Controller(error) => write!(formatter, "{error}"),
            Self::Neural(error) => write!(formatter, "{error}"),
            Self::InvalidConfig { field } => write!(formatter, "invalid control config {field}"),
            Self::PrefixConfigMismatch { field } => {
                write!(
                    formatter,
                    "fixed-step prefix and control config disagree on {field}"
                )
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "control arithmetic overflowed while computing {context}"
                )
            }
            Self::AllocationFailed { buffer, required } => {
                write!(
                    formatter,
                    "could not reserve {required} values for {buffer}"
                )
            }
            Self::CapacityExceeded {
                buffer,
                required,
                maximum,
            } => write!(
                formatter,
                "{buffer} requires {required} records, exceeding configured maximum {maximum}"
            ),
            Self::DuplicateSnakeId(id) => write!(formatter, "duplicate or zero snake ID {id}"),
            Self::DuplicateBrain(brain) => write!(formatter, "duplicate brain {brain:?}"),
            Self::DuplicateControllerSnake(id) => {
                write!(formatter, "multiple controller leases target snake {id}")
            }
            Self::BaselineMapping { snake_id, field } => {
                write!(
                    formatter,
                    "baseline snake {snake_id} has invalid {field} mapping"
                )
            }
            Self::ExternalMapping { snake_id, field } => {
                write!(
                    formatter,
                    "external snake {snake_id} has invalid {field} mapping"
                )
            }
            Self::MissingBrain { snake_id } => {
                write!(formatter, "neural snake {snake_id} has no brain")
            }
            Self::UnknownBrain { snake_id, brain } => {
                write!(
                    formatter,
                    "snake {snake_id} references unknown brain {brain:?}"
                )
            }
            Self::NeuralControlMissing { snake_id } => {
                write!(
                    formatter,
                    "neural output has no staged control for snake {snake_id}"
                )
            }
            Self::InvalidCadenceState(value) => {
                write!(formatter, "invalid neural cadence state {value}")
            }
            Self::InternalShapeMismatch { field } => {
                write!(formatter, "control staging has inconsistent {field}")
            }
            Self::CommitShapeMismatch { field } => {
                write!(formatter, "control commit has inconsistent {field}")
            }
            Self::ResultNotReady => write!(formatter, "no complete control phase is ready"),
        }
    }
}

impl Error for ControlPhaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Prefix(error) => Some(error.as_ref()),
            Self::Spatial(error) => Some(error.as_ref()),
            Self::Sensor(error) => Some(error.as_ref()),
            Self::Baseline(error) => Some(error.as_ref()),
            Self::BaselineLifecycle(error) => Some(error.as_ref()),
            Self::Controller(error) => Some(error.as_ref()),
            Self::Neural(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ambient::AmbientPelletConfig;
    use crate::engine::baseline::{
        BaselineLifecycleConfig, BaselineLifecycleState, BASELINE_LIFECYCLE_VERSION,
    };
    use crate::engine::control::NeuralControlPipeline;
    use crate::engine::fixed_step::{
        FixedStepPrefixConfig, FixedStepPrefixInputs, FixedStepPrefixWorkspace,
    };
    use crate::engine::graph::{
        compile_graph, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::inference::GraphExecutionPlan;
    use crate::engine::physics::{PhysicsConfig, PhysicsError};
    use crate::engine::rng::StatefulRng;
    use crate::engine::sensors::{SensorConfig, SensorEvaluator};
    use crate::engine::state::{
        AllocatorState, BodyRange, BrainOwner, ControllerLease, GenomeLineage,
        LatestControllerAction, PelletState, RngStateBundle, SnakeState, WorldState,
        ALLOCATOR_VERSION, RNG_BUNDLE_VERSION,
    };
    use crate::engine::world_step::{
        WorldStepConfig, WorldStepError, WorldStepWorkspace, MAXIMUM_PHYSICS_SUBSTEPS,
    };

    const DT: f64 = 1.0 / 60.0;
    const EPOCH: u64 = 5;

    struct Fixture {
        world: WorldState,
        rng: RngStateBundle,
        allocators: AllocatorState,
        lifecycle: BaselineLifecycleState,
        brains: Vec<BrainRuntimeState>,
        population: Vec<PopulationGenome>,
    }

    fn key(operation_epoch: u64) -> PhysicsStepKey {
        PhysicsStepKey::new(7, 3, 40, EPOCH, 9, [0x5a; 32], operation_epoch)
    }

    fn graph_limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 16,
            max_edges: 16,
            max_graph_outputs: 4,
            max_identifier_bytes: 64,
            max_total_referenced_identifier_bytes: 4_096,
            max_tensor_width: 256,
            max_mlp_hidden_layers: 4,
            max_split_output_ports: 4,
            max_parameter_floats: 100_000,
            max_recurrent_state_floats: 1_024,
            max_canonical_layout_bytes: 100_000,
            max_architecture_key_bytes: 200_000,
        }
    }

    fn graph_plan() -> GraphExecutionPlan {
        let graph = compile_graph(
            &GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".to_owned(),
                        kind: GraphNodeKind::Input { output_size: 51 },
                    },
                    GraphNodeSpec {
                        id: "memory".to_owned(),
                        kind: GraphNodeKind::Rru {
                            input_size: 51,
                            hidden_size: 2,
                        },
                    },
                    GraphNodeSpec {
                        id: "head".to_owned(),
                        kind: GraphNodeKind::Dense {
                            input_size: 2,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![
                    GraphEdge {
                        from: "input".to_owned(),
                        to: "memory".to_owned(),
                        from_port: None,
                        to_port: None,
                    },
                    GraphEdge {
                        from: "memory".to_owned(),
                        to: "head".to_owned(),
                        from_port: None,
                        to_port: None,
                    },
                ],
                outputs: vec![GraphOutputRef {
                    node_id: "head".to_owned(),
                    port: None,
                }],
                output_size: 2,
            },
            &graph_limits(),
        )
        .unwrap();
        GraphExecutionPlan::build(&graph).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn snake(
        id: u64,
        frame_id: u32,
        kind: SnakeKind,
        position: WorldPoint,
        body: BodyRange,
        brain: Option<BrainHandle>,
        population_slot: Option<u32>,
        baseline_slot: Option<u32>,
        accumulator: f64,
    ) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: frame_id,
            kind,
            alive: true,
            population_slot,
            brain,
            baseline_slot,
            baseline_strategy: baseline_slot.map(|_| BaselineStrategyState::Roam),
            position,
            previous_position: WorldPoint {
                x: position.x - 1.0,
                y: position.y,
            },
            direction: f64::from(frame_id) * 0.05,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 2.0,
            food: 1.0,
            points: 5.0 + f64::from(frame_id),
            kills: 0,
            target_length: 3.0,
            fitness: 0.0,
            turn: -0.2,
            previous_turn: 0.1,
            input_boost: false,
            previous_input_boost: true,
            control_accumulator_seconds: accumulator,
            delivered_observation_points: 1.0,
            body,
            skin: frame_id,
        }
    }

    fn fixture(plan: &GraphExecutionPlan) -> Fixture {
        let evolved = BrainHandle {
            id: 100,
            epoch: EPOCH,
        };
        let connected = BrainHandle {
            id: 101,
            epoch: EPOCH,
        };
        let external_only = BrainHandle {
            id: 102,
            epoch: EPOCH,
        };
        let takeover = BrainHandle {
            id: 103,
            epoch: EPOCH,
        };
        let grace = BrainHandle {
            id: 104,
            epoch: EPOCH,
        };
        let descriptors = [
            (40, 5, SnakeKind::External, Some(takeover), None, None, 0.0),
            (20, 2, SnakeKind::Baseline, None, None, Some(0), 0.0),
            (10, 1, SnakeKind::Evolved, Some(evolved), Some(0), None, DT),
            (
                35,
                4,
                SnakeKind::External,
                Some(external_only),
                None,
                None,
                0.03,
            ),
            (
                30,
                3,
                SnakeKind::External,
                Some(connected),
                None,
                None,
                0.03,
            ),
            (50, 6, SnakeKind::External, Some(grace), None, None, 0.04),
        ];
        let mut body_points = Vec::new();
        let mut snakes = Vec::new();
        for (ordinal, (id, frame_id, kind, brain, population_slot, baseline_slot, accumulator)) in
            descriptors.into_iter().enumerate()
        {
            let position = WorldPoint {
                x: ordinal as f64 * 260.0 - 520.0,
                y: ordinal as f64 * 45.0,
            };
            let start = body_points.len();
            body_points.extend([
                position,
                WorldPoint {
                    x: position.x - 15.0,
                    y: position.y,
                },
                WorldPoint {
                    x: position.x - 30.0,
                    y: position.y,
                },
            ]);
            snakes.push(snake(
                id,
                frame_id,
                kind,
                position,
                BodyRange { start, len: 3 },
                brain,
                population_slot,
                baseline_slot,
                accumulator,
            ));
        }
        for snake in &mut snakes {
            if matches!(snake.id, 40 | 50) {
                snake.turn = 0.0;
                snake.input_boost = false;
            }
        }

        let weights = |salt: usize| {
            (0..plan.total_parameters())
                .map(|index| (((index * 19 + salt * 31) % 101) as f32 - 50.0) / 180.0)
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        let recurrent = |salt: usize| {
            (0..plan.total_state_size())
                .map(|index| (salt * 5 + index + 1) as f32 / 100.0)
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        let brains = vec![
            BrainRuntimeState {
                handle: takeover,
                owner: BrainOwner::Entity(40),
                non_population_weights: Some(weights(4)),
                recurrent: recurrent(4),
            },
            BrainRuntimeState {
                handle: evolved,
                owner: BrainOwner::PopulationSlot(0),
                non_population_weights: None,
                recurrent: recurrent(1),
            },
            BrainRuntimeState {
                handle: external_only,
                owner: BrainOwner::Entity(35),
                non_population_weights: Some(weights(3)),
                recurrent: recurrent(3),
            },
            BrainRuntimeState {
                handle: connected,
                owner: BrainOwner::Entity(30),
                non_population_weights: Some(weights(2)),
                recurrent: recurrent(2),
            },
            BrainRuntimeState {
                handle: grace,
                owner: BrainOwner::Entity(50),
                non_population_weights: Some(weights(5)),
                recurrent: recurrent(5),
            },
        ];
        let population = vec![PopulationGenome {
            slot: 0,
            brain: evolved,
            lineage: GenomeLineage {
                genome_id: 500,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights: weights(1),
        }];
        let controller_leases = vec![
            ControllerLease {
                id: 2,
                snake_id: 40,
                kind: ControllerKind::ReinforcementLearning,
                connection_id: None,
                scope: "run:7:rl".to_owned(),
                resume_token: "takeover-token".to_owned(),
                status: ControllerLeaseStatus::ReservedNeutral,
                latest_action: LatestControllerAction {
                    turn: -0.5,
                    boost: true,
                    client_tick: 1,
                    arrival_sequence: 1,
                    accepted_at_ms: 900,
                },
                last_observed_at_ms: 1_000,
                disconnected_at_ms: Some(1_000),
                input_hold_expires_at_ms: Some(1_400),
                grace_expires_at_ms: Some(31_000),
                takeover_committed_at_ms: None,
            },
            ControllerLease {
                id: 1,
                snake_id: 30,
                kind: ControllerKind::Player,
                connection_id: Some(700),
                scope: "run:7:player".to_owned(),
                resume_token: "connected-token".to_owned(),
                status: ControllerLeaseStatus::Connected,
                latest_action: LatestControllerAction {
                    turn: 0.75,
                    boost: true,
                    client_tick: 2,
                    arrival_sequence: 2,
                    accepted_at_ms: 30_800,
                },
                last_observed_at_ms: 30_800,
                disconnected_at_ms: None,
                input_hold_expires_at_ms: None,
                grace_expires_at_ms: None,
                takeover_committed_at_ms: None,
            },
            ControllerLease {
                id: 3,
                snake_id: 50,
                kind: ControllerKind::Player,
                connection_id: None,
                scope: "run:7:grace".to_owned(),
                resume_token: "grace-token".to_owned(),
                status: ControllerLeaseStatus::ReservedNeutral,
                latest_action: LatestControllerAction {
                    turn: 0.6,
                    boost: true,
                    client_tick: 3,
                    arrival_sequence: 3,
                    accepted_at_ms: 30_400,
                },
                last_observed_at_ms: 30_500,
                disconnected_at_ms: Some(30_500),
                input_hold_expires_at_ms: Some(30_900),
                grace_expires_at_ms: Some(60_500),
                takeover_committed_at_ms: None,
            },
        ];
        let world = WorldState {
            snakes,
            body_points,
            pellets: vec![PelletState {
                id: 80,
                position: WorldPoint { x: 80.0, y: 90.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            }],
            controller_leases,
        };
        let rng = RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: StatefulRng::new(11.0).export_state(),
            evolution: StatefulRng::new(12.0).export_state(),
            external_controller: StatefulRng::new(13.0).export_state(),
            baselines: vec![BaselineRngState {
                slot: 0,
                state: StatefulRng::new(14.0).export_state(),
            }],
        };
        let allocators = AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 100,
            next_brain_id: 200,
            next_genome_id: 600,
            next_controller_lease_id: 4,
            next_frame_v1_id: 81,
            next_external_id: 1_000_000_000_100,
            next_baseline_id: 2_000_000_000_100,
            next_resurrected_id: 3_000_000_000_100,
        };
        let lifecycle = BaselineLifecycleState {
            version: BASELINE_LIFECYCLE_VERSION,
            slots: vec![BaselineSlotRuntime {
                slot: 0,
                snake_id: 20,
                strategy_timer_seconds: 0.0,
                wander_angle: 0.0,
                wander_timer_seconds: 1.0,
                turn: 0.0,
                boost: false,
                respawn_remaining_seconds: None,
            }],
        };
        Fixture {
            world,
            rng,
            allocators,
            lifecycle,
            brains,
            population,
        }
    }

    fn prefix_config() -> FixedStepPrefixConfig {
        let mut ambient = AmbientPelletConfig::typescript_defaults();
        ambient.target_count = 1;
        ambient.spawn_per_second = 0.0;
        FixedStepPrefixConfig {
            fixed_dt: DT,
            ambient,
            baseline: BaselineLifecycleConfig {
                slot_count: 1,
                ..BaselineLifecycleConfig::typescript_defaults()
            },
            maximum_snakes: 8,
            maximum_pellets: 256,
            ..FixedStepPrefixConfig::typescript_defaults()
        }
    }

    fn control_config() -> ControlPhaseConfig {
        ControlPhaseConfig {
            baseline: BaselineControlConfig {
                maximum_world_snakes: 8,
                ..BaselineControlConfig::typescript_defaults()
            },
            maximum_snakes: 8,
            maximum_brains: 8,
            maximum_external_observations: 4,
            sensor_index: SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 10_000,
                maximum_pellet_entries: 1_000,
            },
            ..ControlPhaseConfig::typescript_defaults()
        }
    }

    fn control_workspace(plan: GraphExecutionPlan) -> ControlPhaseWorkspace {
        let sensor = SensorEvaluator::new(SensorConfig {
            bins: 8,
            ..SensorConfig::default()
        })
        .unwrap();
        let neural = NeuralControlPipeline::try_new(8, sensor, plan, usize::MAX).unwrap();
        ControlPhaseWorkspace::new(neural)
    }

    fn world_step_config() -> WorldStepConfig {
        let prefix = prefix_config();
        let mut physics = PhysicsConfig::typescript_defaults();
        physics.maximum_body_points = 1_000;
        physics.maximum_pellets = prefix.maximum_pellets;
        physics.maximum_pellet_index_entries = 1_000;
        WorldStepConfig {
            prefix,
            control: control_config(),
            physics,
            baseline: prefix.baseline,
            physics_substeps: 3,
            ..WorldStepConfig::typescript_defaults()
        }
    }

    fn make_fixture_bodies_physics_ready(world: &mut WorldState) {
        let mut body_points = Vec::with_capacity(world.snakes.len() * 5);
        for snake in &mut world.snakes {
            let start = body_points.len();
            for offset in 0..5 {
                body_points.push(WorldPoint {
                    x: snake.position.x - offset as f64 * 15.0,
                    y: snake.position.y,
                });
            }
            snake.body = BodyRange { start, len: 5 };
            snake.target_length = 5.0;
        }
        world.body_points = body_points;
    }

    #[derive(Debug, PartialEq)]
    struct BaselineSummary {
        snake_id: u64,
        slot: u32,
        next_slot: BaselineSlotRuntime,
        next_strategy: BaselineStrategyState,
        next_rng: BaselineRngState,
        delivery: ObservationDeliveryMarker,
        diagnostics: BaselineControlDiagnostics,
        sensor_diagnostics: SensorSampleDiagnostics,
    }

    #[derive(Debug, PartialEq)]
    struct ExternalSummary {
        lease_id: u64,
        connection_id: u64,
        kind: ControllerKind,
        position_x_bits: u64,
        position_y_bits: u64,
        direction_bits: u64,
        delivery: ObservationDeliveryMarker,
        diagnostics: SensorSampleDiagnostics,
        observation_bits: Vec<u32>,
    }

    #[derive(Debug, PartialEq)]
    struct ControlSummary {
        calculation_key: CalculationBatchKey,
        controls: Vec<(u64, SelectedControlSource, u32, bool, u64)>,
        transitions: Vec<ControllerBoundaryProposal>,
        baselines: Vec<BaselineSummary>,
        external: Vec<ExternalSummary>,
        counts: (usize, usize, usize, usize, usize, usize),
    }

    fn prepared_summary_for(plan: GraphExecutionPlan, fixture: &Fixture) -> ControlSummary {
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(12),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut workspace = control_workspace(plan);
        let prepared = workspace
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let controls = prepared
            .control_updates()
            .iter()
            .map(|update| {
                (
                    update.snake_id(),
                    update.source(),
                    update.turn().to_bits(),
                    update.boost(),
                    update.next_control_accumulator_seconds().to_bits(),
                )
            })
            .collect();
        let transitions = prepared
            .controller_transitions()
            .iter()
            .map(|transition| transition.proposal())
            .collect();
        let baselines = prepared
            .baseline_controls()
            .iter()
            .map(|result| BaselineSummary {
                snake_id: result.snake_id(),
                slot: result.slot(),
                next_slot: result.next_slot(),
                next_strategy: result.next_strategy(),
                next_rng: result.next_rng().clone(),
                delivery: result.delivery(),
                diagnostics: result.diagnostics(),
                sensor_diagnostics: result.sensor_diagnostics(),
            })
            .collect();
        let external = (0..prepared.external_events().len())
            .map(|index| {
                let (event, observation) = prepared.external_observation(index).unwrap();
                ExternalSummary {
                    lease_id: event.lease_id(),
                    connection_id: event.connection_id(),
                    kind: event.kind(),
                    position_x_bits: event.position().x.to_bits(),
                    position_y_bits: event.position().y.to_bits(),
                    direction_bits: event.direction().to_bits(),
                    delivery: event.delivery(),
                    diagnostics: event.diagnostics(),
                    observation_bits: observation.iter().map(|value| value.to_bits()).collect(),
                }
            })
            .collect();
        let diagnostics = prepared.diagnostics();
        ControlSummary {
            calculation_key: prepared.calculation_key(),
            controls,
            transitions,
            baselines,
            external,
            counts: (
                diagnostics.controls,
                diagnostics.baseline_observations,
                diagnostics.external_observations,
                diagnostics.neural_evaluations,
                diagnostics.neural_held,
                diagnostics.neural_takeovers,
            ),
        }
    }

    fn prepared_summary(reverse_storage: bool) -> ControlSummary {
        let plan = graph_plan();
        let mut fixture = fixture(&plan);
        if reverse_storage {
            fixture.world.snakes.reverse();
            fixture.world.controller_leases.reverse();
            fixture.brains.reverse();
        }
        prepared_summary_for(plan, &fixture)
    }

    fn summarized_control(
        summary: &ControlSummary,
        snake_id: u64,
    ) -> &(u64, SelectedControlSource, u32, bool, u64) {
        summary
            .controls
            .iter()
            .find(|control| control.0 == snake_id)
            .unwrap()
    }

    #[test]
    fn one_indexed_boundary_selects_baseline_external_neural_and_takeover_exclusively() {
        let plan = graph_plan();
        let fixture = fixture(&plan);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_allocators = fixture.allocators.clone();
        let source_lifecycle = fixture.lifecycle.clone();
        let source_brains = fixture.brains.clone();
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(1),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut workspace = control_workspace(plan);
        let prepared = workspace
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();

        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.allocators, source_allocators);
        assert_eq!(fixture.lifecycle, source_lifecycle);
        assert_eq!(fixture.brains, source_brains);
        assert_eq!(prepared.key(), key(1));
        assert_eq!(prepared.calculation_key().step(), 41);
        let controls = prepared.control_updates();
        assert_eq!(
            controls
                .iter()
                .map(|update| update.snake_id())
                .collect::<Vec<_>>(),
            vec![10, 20, 30, 35, 40, 50]
        );
        assert_eq!(controls[0].source(), SelectedControlSource::NeuralEvaluated);
        assert_eq!(controls[1].source(), SelectedControlSource::Baseline);
        assert_eq!(controls[2].source(), SelectedControlSource::ExternalHeld);
        assert_eq!(controls[2].turn(), 0.75);
        assert!(controls[2].boost());
        assert_eq!(
            controls[3].source(),
            SelectedControlSource::ExternalOnlyNeutral
        );
        assert_eq!(controls[3].turn(), 0.0);
        assert!(!controls[3].boost());
        assert_eq!(controls[4].source(), SelectedControlSource::NeuralTakeover);
        assert_eq!(
            controls[5].source(),
            SelectedControlSource::ExternalReservedNeutral
        );
        assert_eq!(controls[5].turn(), 0.0);
        assert!(!controls[5].boost());
        assert_eq!(prepared.controller_transitions().len(), 3);
        assert_eq!(prepared.baseline_controls().len(), 1);
        assert_eq!(prepared.baseline_controls()[0].snake_id(), 20);
        assert_eq!(prepared.external_events().len(), 1);
        let (event, observation) = prepared.external_observation(0).unwrap();
        assert_eq!(event.snake_id(), 30);
        assert_eq!(event.connection_id(), 700);
        assert_eq!(event.kind(), ControllerKind::Player);
        assert_eq!(observation.len(), 51);
        assert!(observation.iter().all(|value| value.is_finite()));
        assert_eq!(event.delivery().previous_delivered_points, 1.0);
        assert_eq!(
            prefix.world().snakes[event.snake_index()].delivered_observation_points,
            1.0
        );
        let diagnostics = prepared.diagnostics();
        assert_eq!(diagnostics.controls, 6);
        assert_eq!(diagnostics.baseline_observations, 1);
        assert_eq!(diagnostics.external_observations, 1);
        assert_eq!(diagnostics.neural_evaluations, 2);
        assert_eq!(diagnostics.neural_takeovers, 1);
        assert!(diagnostics.body_index.segments > 0);
        assert_eq!(diagnostics.pellet_index.pellets, 1);
    }

    #[test]
    fn complete_control_commit_updates_internal_state_and_retains_external_delivery() {
        let plan = graph_plan();
        let fixture = fixture(&plan);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_allocators = fixture.allocators.clone();
        let source_lifecycle = fixture.lifecycle.clone();
        let source_brains = fixture.brains.clone();
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(20),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let prefix_world = prefix.world().clone();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut selection = control_workspace(plan);
        let phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let baseline = phase.baseline_controls()[0].clone();
        let expected_external = phase.external_observation(0).unwrap().1.to_vec();
        let expected_external_marker = phase.external_events()[0].delivery();
        let mut commit_workspace = ControlCommitWorkspace::new();
        let committed = commit_workspace.prepare(phase).unwrap();

        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.allocators, source_allocators);
        assert_eq!(fixture.lifecycle, source_lifecycle);
        assert_eq!(fixture.brains, source_brains);
        assert_eq!(committed.key(), key(20));
        assert_eq!(committed.calculation_key().step(), 41);
        assert_eq!(committed.generation_elapsed_seconds(), 2.0 + DT);
        assert_eq!(committed.ambient_accumulator(), 0.0);
        assert_eq!(committed.allocators(), prefix.allocators());
        assert_eq!(committed.sensor_generation(), generation);
        assert_eq!(committed.external_events().len(), 1);
        assert_eq!(
            committed.external_observation(0).unwrap().1,
            expected_external
        );

        let baseline_snake = committed
            .world()
            .snakes
            .iter()
            .find(|snake| snake.id == 20)
            .unwrap();
        assert_eq!(
            baseline_snake.baseline_strategy,
            Some(baseline.next_strategy())
        );
        assert_eq!(
            baseline_snake.turn.to_bits(),
            baseline.next_slot().turn.to_bits()
        );
        assert_eq!(baseline_snake.input_boost, baseline.next_slot().boost);
        assert_eq!(
            baseline_snake.delivered_observation_points.to_bits(),
            baseline.delivery().sampled_points.to_bits()
        );
        assert_eq!(
            committed.baseline_lifecycle().slots[0],
            baseline.next_slot()
        );
        assert_eq!(committed.rng().baselines[0], *baseline.next_rng());

        let external = committed
            .world()
            .snakes
            .iter()
            .find(|snake| snake.id == 30)
            .unwrap();
        assert_eq!(external.turn, 0.75);
        assert!(external.input_boost);
        assert_eq!(
            external.delivered_observation_points.to_bits(),
            expected_external_marker.previous_delivered_points.to_bits()
        );
        let connected_lease = committed
            .world()
            .controller_leases
            .iter()
            .find(|lease| lease.snake_id == 30)
            .unwrap();
        assert_eq!(connected_lease.last_observed_at_ms, 31_000);

        let takeover = committed
            .world()
            .snakes
            .iter()
            .find(|snake| snake.id == 40)
            .unwrap();
        let takeover_lease = committed
            .world()
            .controller_leases
            .iter()
            .find(|lease| lease.snake_id == 40)
            .unwrap();
        assert_eq!(takeover_lease.status, ControllerLeaseStatus::NeuralTakeover);
        assert_eq!(takeover_lease.takeover_committed_at_ms, Some(31_000));
        assert_ne!((takeover.turn.to_bits(), takeover.input_boost), (0, false));

        let evolved_before = prefix_world
            .snakes
            .iter()
            .find(|snake| snake.id == 10)
            .unwrap();
        let evolved_after = committed
            .world()
            .snakes
            .iter()
            .find(|snake| snake.id == 10)
            .unwrap();
        assert_eq!(
            evolved_after.delivered_observation_points.to_bits(),
            evolved_after.points.to_bits()
        );
        assert_ne!(
            evolved_before.delivered_observation_points.to_bits(),
            evolved_after.delivered_observation_points.to_bits()
        );
        let evolved_handle = evolved_after.brain.unwrap();
        let recurrent_before = fixture
            .brains
            .iter()
            .find(|brain| brain.handle == evolved_handle)
            .unwrap();
        let recurrent_after = committed
            .brains()
            .iter()
            .find(|brain| brain.handle == evolved_handle)
            .unwrap();
        assert_ne!(recurrent_before.recurrent, recurrent_after.recurrent);
    }

    #[test]
    fn complete_control_boundary_advances_through_every_physics_substep() {
        let plan = graph_plan();
        let mut fixture = fixture(&plan);
        make_fixture_bodies_physics_ready(&mut fixture.world);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_allocators = fixture.allocators.clone();
        let source_lifecycle = fixture.lifecycle.clone();
        let source_brains = fixture.brains.clone();

        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(28),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut selection = control_workspace(plan);
        let phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let mut commit_workspace = ControlCommitWorkspace::new();
        let committed = commit_workspace.prepare(phase).unwrap();
        let committed_leases = committed.world().controller_leases.clone();
        let committed_lifecycle = committed.baseline_lifecycle().clone();
        let external_observation = committed.external_observation(0).unwrap().1.to_vec();

        let mut step_workspace = WorldStepWorkspace::new();
        let prepared = step_workspace
            .prepare(committed, world_step_config())
            .unwrap();

        assert_eq!(prepared.key(), key(28));
        assert_eq!(prepared.config(), world_step_config());
        assert_eq!(prepared.calculation_key().step(), 41);
        assert_eq!(prepared.world().controller_leases, committed_leases);
        assert_eq!(prepared.baseline_lifecycle(), &committed_lifecycle);
        assert_eq!(prepared.brains(), committed.brains());
        assert_eq!(
            prepared.external_observation(0).unwrap().1,
            external_observation
        );
        assert_eq!(prepared.generation_elapsed_seconds(), 2.0 + DT);
        assert_eq!(prepared.ambient_accumulator(), 0.0);
        assert!(prepared
            .world()
            .snakes
            .iter()
            .zip(committed.world().snakes.iter())
            .any(|(after, before)| after.position != before.position));
        assert!(
            prepared.sensor_generation().best_points_this_generation()
                >= generation.best_points_this_generation()
        );
        let diagnostics = prepared.diagnostics();
        assert_eq!(diagnostics.physics.expected_substeps, 3);
        assert_eq!(diagnostics.physics.completed_substeps, 3);
        assert!(diagnostics.physics.controller_lease_capacity >= 3);
        assert_eq!(diagnostics.control.selection.controls, 6);

        let warmed = step_workspace
            .prepare(committed, world_step_config())
            .unwrap()
            .diagnostics();
        for _ in 0..8 {
            assert_eq!(
                step_workspace
                    .prepare(committed, world_step_config())
                    .unwrap()
                    .diagnostics(),
                warmed
            );
        }

        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.allocators, source_allocators);
        assert_eq!(fixture.lifecycle, source_lifecycle);
        assert_eq!(fixture.brains, source_brains);
    }

    #[test]
    fn post_physics_baseline_death_starts_full_delay_in_the_same_working_step() {
        let plan = graph_plan();
        let mut fixture = fixture(&plan);
        let evolved_position = fixture
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == 10)
            .unwrap()
            .position;
        let baseline = fixture
            .world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == 20)
            .unwrap();
        baseline.position = evolved_position;
        baseline.previous_position = evolved_position;
        make_fixture_bodies_physics_ready(&mut fixture.world);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_lifecycle = fixture.lifecycle.clone();

        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(29),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut selection = control_workspace(plan);
        let phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let mut commit_workspace = ControlCommitWorkspace::new();
        let committed = commit_workspace.prepare(phase).unwrap();
        let mut step_workspace = WorldStepWorkspace::new();
        let prepared = step_workspace
            .prepare(committed, world_step_config())
            .unwrap();

        let baseline = prepared
            .world()
            .snakes
            .iter()
            .find(|snake| snake.id == 20)
            .unwrap();
        assert!(!baseline.alive);
        let slot = prepared.baseline_lifecycle().slots[0];
        assert_eq!(slot.snake_id, 20);
        assert_eq!(slot.respawn_remaining_seconds, Some(20.0));
        assert_eq!(slot.turn.to_bits(), 0.0_f32.to_bits());
        assert!(!slot.boost);
        assert_eq!(prepared.diagnostics().physics.baseline_deaths, 1);
        assert_ne!(prepared.rng(), &source_rng);
        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.lifecycle, source_lifecycle);
    }

    #[test]
    fn controlled_death_rejects_the_joined_step_without_touching_any_source() {
        let plan = graph_plan();
        let mut fixture = fixture(&plan);
        let evolved_position = fixture
            .world
            .snakes
            .iter()
            .find(|snake| snake.id == 10)
            .unwrap()
            .position;
        let controlled = fixture
            .world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == 30)
            .unwrap();
        controlled.position = evolved_position;
        controlled.previous_position = evolved_position;
        make_fixture_bodies_physics_ready(&mut fixture.world);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_allocators = fixture.allocators.clone();
        let source_lifecycle = fixture.lifecycle.clone();
        let source_brains = fixture.brains.clone();

        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(30),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut selection = control_workspace(plan);
        let phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let mut commit_workspace = ControlCommitWorkspace::new();
        let committed = commit_workspace.prepare(phase).unwrap();
        let committed_world = committed.world().clone();
        let committed_rng = committed.rng().clone();
        let committed_allocators = committed.allocators().clone();
        let committed_lifecycle = committed.baseline_lifecycle().clone();
        let committed_brains = committed.brains().to_vec();
        let mut step_workspace = WorldStepWorkspace::new();

        let mut mismatched_prefix = world_step_config();
        mismatched_prefix.prefix.maximum_pellets -= 1;
        let mut mismatched_config = world_step_config();
        mismatched_config.control.maximum_snakes += 1;
        for (config, expected_field) in [
            (mismatched_prefix, "prefix"),
            (mismatched_config, "control"),
        ] {
            match step_workspace.prepare(committed, config) {
                Err(WorldStepError::ControlConfigMismatch { field }) => {
                    assert_eq!(field, expected_field);
                }
                other => panic!("expected {expected_field} mismatch, got {other:?}"),
            }
            assert!(!step_workspace.is_ready());
        }

        let mut wrong_version = world_step_config();
        wrong_version.algorithm_version += 1;
        let mut mismatched_baseline = world_step_config();
        mismatched_baseline.baseline.respawn_delay_seconds += 1.0;
        let mut mismatched_pellet_limit = world_step_config();
        mismatched_pellet_limit.physics.maximum_pellets -= 1;
        let mut mismatched_world_radius = world_step_config();
        mismatched_world_radius.physics.movement.world_radius -= 1.0;
        let mut zero_substeps = world_step_config();
        zero_substeps.physics_substeps = 0;
        let mut excessive_substeps = world_step_config();
        excessive_substeps.physics_substeps = MAXIMUM_PHYSICS_SUBSTEPS + 1;
        let mut mismatched_substep_sum = world_step_config();
        mismatched_substep_sum.physics.substep_dt *= 0.5;
        for (config, expected_field) in [
            (wrong_version, "algorithm_version"),
            (mismatched_baseline, "baseline lifecycle"),
            (mismatched_pellet_limit, "maximum pellets"),
            (mismatched_world_radius, "world radius"),
            (zero_substeps, "physics substeps"),
            (excessive_substeps, "physics substeps"),
            (mismatched_substep_sum, "physics substep sum"),
        ] {
            match step_workspace.prepare(committed, config) {
                Err(WorldStepError::InvalidConfig { field }) => {
                    assert_eq!(field, expected_field);
                }
                other => panic!("expected invalid {expected_field}, got {other:?}"),
            }
            assert!(!step_workspace.is_ready());
        }
        let error = step_workspace
            .prepare(committed, world_step_config())
            .expect_err("controlled death must wait for atomic replacement staging");

        assert!(matches!(
            error,
            WorldStepError::Physics(error)
                if matches!(*error, PhysicsError::ControllerReplacementRequired { snake_id: 30 })
        ));
        assert!(!step_workspace.is_ready());
        assert_eq!(committed.world(), &committed_world);
        assert_eq!(committed.rng(), &committed_rng);
        assert_eq!(committed.allocators(), &committed_allocators);
        assert_eq!(committed.baseline_lifecycle(), &committed_lifecycle);
        assert_eq!(committed.brains(), committed_brains);
        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.allocators, source_allocators);
        assert_eq!(fixture.lifecycle, source_lifecycle);
        assert_eq!(fixture.brains, source_brains);
    }

    #[test]
    fn committed_takeover_holds_neural_action_without_reapplying_external_neutral() {
        let plan = graph_plan();
        let mut fixture = fixture(&plan);
        let takeover_snake = fixture
            .world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == 40)
            .unwrap();
        takeover_snake.turn = 0.4;
        takeover_snake.previous_turn = -0.3;
        takeover_snake.input_boost = true;
        takeover_snake.previous_input_boost = false;
        takeover_snake.control_accumulator_seconds = 0.0;
        let takeover_lease = fixture
            .world
            .controller_leases
            .iter_mut()
            .find(|lease| lease.snake_id == 40)
            .unwrap();
        takeover_lease.status = ControllerLeaseStatus::NeuralTakeover;
        takeover_lease.takeover_committed_at_ms = Some(31_000);
        takeover_lease.last_observed_at_ms = 31_000;
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(21),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut config = control_config();
        config.neural_control_interval_seconds = 0.05;
        let mut selection = control_workspace(plan);
        let phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_001,
                config,
            })
            .unwrap();
        let update = phase
            .control_updates()
            .iter()
            .find(|update| update.snake_id() == 40)
            .unwrap();
        assert_eq!(update.source(), SelectedControlSource::NeuralHeld);
        let mut commit_workspace = ControlCommitWorkspace::new();
        let committed = commit_workspace.prepare(phase).unwrap();
        let takeover = committed
            .world()
            .snakes
            .iter()
            .find(|snake| snake.id == 40)
            .unwrap();
        assert_eq!(takeover.turn.to_bits(), 0.4_f32.to_bits());
        assert_eq!(takeover.previous_turn.to_bits(), (-0.3_f32).to_bits());
        assert!(takeover.input_boost);
        assert!(!takeover.previous_input_boost);
    }

    #[test]
    fn malformed_control_commit_is_rejected_without_touching_sources() {
        let plan = graph_plan();
        let fixture = fixture(&plan);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_allocators = fixture.allocators.clone();
        let source_lifecycle = fixture.lifecycle.clone();
        let source_brains = fixture.brains.clone();
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(22),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut selection = control_workspace(plan);
        let mut phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let mut malformed = phase.control_updates().to_vec();
        malformed[0].snake_id = 999;
        phase.controls = &malformed;
        let mut commit_workspace = ControlCommitWorkspace::new();
        assert!(matches!(
            commit_workspace.prepare(phase),
            Err(ControlPhaseError::CommitShapeMismatch {
                field: "control snake identity"
            })
        ));
        assert!(!commit_workspace.is_ready());
        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.allocators, source_allocators);
        assert_eq!(fixture.lifecycle, source_lifecycle);
        assert_eq!(fixture.brains, source_brains);
    }

    #[test]
    fn controller_results_are_independent_of_world_lease_and_brain_storage_order() {
        assert_eq!(prepared_summary(false), prepared_summary(true));
    }

    #[test]
    fn joined_takeover_resets_only_the_expired_controller_recurrent_state() {
        let original_plan = graph_plan();
        let original_fixture = fixture(&original_plan);
        let original = prepared_summary_for(original_plan, &original_fixture);

        let takeover_plan = graph_plan();
        let mut changed_takeover = fixture(&takeover_plan);
        changed_takeover
            .brains
            .iter_mut()
            .find(|brain| brain.owner == BrainOwner::Entity(40))
            .unwrap()
            .recurrent
            .fill(9.0);
        let changed_takeover = prepared_summary_for(takeover_plan, &changed_takeover);
        assert_eq!(
            summarized_control(&original, 40),
            summarized_control(&changed_takeover, 40)
        );

        let evolved_plan = graph_plan();
        let mut changed_evolved = fixture(&evolved_plan);
        changed_evolved
            .brains
            .iter_mut()
            .find(|brain| brain.owner == BrainOwner::PopulationSlot(0))
            .unwrap()
            .recurrent
            .fill(9.0);
        let changed_evolved = prepared_summary_for(evolved_plan, &changed_evolved);
        assert_ne!(
            summarized_control(&original, 10).2,
            summarized_control(&changed_evolved, 10).2
        );
    }

    #[test]
    fn cadence_and_takeover_force_have_exact_distinct_boundaries() {
        let mut invalid_config = control_config();
        invalid_config.neural_control_interval_seconds = 0.061;
        assert!(matches!(
            invalid_config.validate(),
            Err(ControlPhaseError::InvalidConfig {
                field: "neural_control_interval_seconds"
            })
        ));
        assert_eq!(
            next_neural_boundary(0.0, DT, 0.05, false).unwrap(),
            (false, DT)
        );
        let (due, remainder) = next_neural_boundary(0.04, DT, 0.05, false).unwrap();
        assert!(due);
        assert!((remainder - (0.04 + DT - 0.05)).abs() < 1.0e-12);
        let (due, takeover_remainder) = next_neural_boundary(0.04, DT, 0.05, true).unwrap();
        assert!(due);
        assert!((takeover_remainder - DT).abs() < 1.0e-12);
        assert!(matches!(
            next_neural_boundary(f64::INFINITY, DT, 0.05, false),
            Err(ControlPhaseError::InvalidCadenceState(value)) if value.is_infinite()
        ));
    }

    #[test]
    fn newly_created_neural_snake_stays_due_after_live_control_interval_increase() {
        let plan = graph_plan();
        let mut fixture = fixture(&plan);
        let mut creation_config = control_config();
        creation_config.neural_control_interval_seconds = 0.008;
        let initial_accumulator = creation_config.initial_neural_accumulator_seconds();
        let mut config = creation_config;
        config.neural_control_interval_seconds = 0.06;
        let evolved = fixture
            .world
            .snakes
            .iter_mut()
            .find(|snake| snake.id == 10)
            .unwrap();
        evolved.control_accumulator_seconds = initial_accumulator;

        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(11),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut workspace = control_workspace(plan);
        let prepared = workspace
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config,
            })
            .unwrap();
        let update = prepared
            .control_updates()
            .iter()
            .find(|update| update.snake_id() == 10)
            .unwrap();

        assert_eq!(update.source(), SelectedControlSource::NeuralEvaluated);
        assert!((update.next_control_accumulator_seconds() - DT).abs() < 1.0e-12);
    }

    #[test]
    fn external_observation_capacity_failure_leaves_every_source_unchanged_and_unready() {
        let plan = graph_plan();
        let fixture = fixture(&plan);
        let source_world = fixture.world.clone();
        let source_rng = fixture.rng.clone();
        let source_allocators = fixture.allocators.clone();
        let source_lifecycle = fixture.lifecycle.clone();
        let source_brains = fixture.brains.clone();
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(2),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut config = control_config();
        config.maximum_external_observations = 0;
        let mut workspace = control_workspace(plan);
        assert!(matches!(
            workspace.prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config,
            }),
            Err(ControlPhaseError::CapacityExceeded {
                buffer: "external observations",
                required: 1,
                maximum: 0,
            })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(fixture.world, source_world);
        assert_eq!(fixture.rng, source_rng);
        assert_eq!(fixture.allocators, source_allocators);
        assert_eq!(fixture.lifecycle, source_lifecycle);
        assert_eq!(fixture.brains, source_brains);
    }

    #[test]
    fn warmed_shared_boundary_reuses_every_reported_capacity() {
        let plan = graph_plan();
        let fixture = fixture(&plan);
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(3),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut workspace = control_workspace(plan);
        let mut expected = None;
        for _ in 0..24 {
            let prepared = workspace
                .prepare(ControlPhaseInputs {
                    prefix,
                    generation: &generation,
                    population: &fixture.population,
                    brains: &fixture.brains,
                    wall_now_ms: 31_000,
                    config: control_config(),
                })
                .unwrap();
            let diagnostics = prepared.diagnostics();
            if let Some(expected) = expected {
                assert_eq!(diagnostics, expected);
            } else {
                expected = Some(diagnostics);
            }
        }
    }

    #[test]
    fn warmed_control_commit_reuses_every_reported_capacity() {
        let plan = graph_plan();
        let fixture = fixture(&plan);
        let mut prefix_workspace = FixedStepPrefixWorkspace::new();
        let prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: key(23),
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(prefix.world()).unwrap();
        let mut selection = control_workspace(plan);
        let mut commit = ControlCommitWorkspace::new();
        let warm_phase = selection
            .prepare(ControlPhaseInputs {
                prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &fixture.brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let warm = commit.prepare(warm_phase).unwrap().diagnostics();
        assert!(warm.brain_weight_values_copied > 0);
        let mut expected = None;
        for _ in 0..24 {
            let phase = selection
                .prepare(ControlPhaseInputs {
                    prefix,
                    generation: &generation,
                    population: &fixture.population,
                    brains: &fixture.brains,
                    wall_now_ms: 31_000,
                    config: control_config(),
                })
                .unwrap();
            let prepared = commit.prepare(phase).unwrap();
            let diagnostics = prepared.diagnostics();
            assert!(diagnostics.rng_text_capacity > 0);
            assert!(diagnostics.brain_capacity >= fixture.brains.len());
            assert_eq!(diagnostics.brain_weight_values_copied, 0);
            assert!(diagnostics.brain_recurrent_values_copied > 0);
            assert!(diagnostics.external_event_capacity >= 1);
            assert!(diagnostics.external_observation_capacity >= 51);
            if let Some(expected) = expected {
                assert_eq!(diagnostics, expected);
            } else {
                expected = Some(diagnostics);
            }
        }

        let mut replacement_brains = fixture.brains.clone();
        replacement_brains
            .iter_mut()
            .find(|brain| brain.owner == BrainOwner::Entity(40))
            .unwrap()
            .non_population_weights
            .as_mut()
            .unwrap()
            .fill(0.875);
        let replacement_key = PhysicsStepKey::new(8, 3, 40, EPOCH, 9, [0x5a; 32], 24);
        let replacement_prefix = prefix_workspace
            .prepare(FixedStepPrefixInputs {
                key: replacement_key,
                world: &fixture.world,
                rng: &fixture.rng,
                allocators: &fixture.allocators,
                generation_elapsed_seconds: 2.0,
                ambient_accumulator: 0.0,
                baseline_lifecycle: &fixture.lifecycle,
                config: prefix_config(),
            })
            .unwrap();
        let replacement_phase = selection
            .prepare(ControlPhaseInputs {
                prefix: replacement_prefix,
                generation: &generation,
                population: &fixture.population,
                brains: &replacement_brains,
                wall_now_ms: 31_000,
                config: control_config(),
            })
            .unwrap();
        let replacement = commit.prepare(replacement_phase).unwrap();
        assert!(replacement.diagnostics().brain_weight_values_copied > 0);
        assert!(replacement
            .brains()
            .iter()
            .find(|brain| brain.owner == BrainOwner::Entity(40))
            .unwrap()
            .non_population_weights
            .as_ref()
            .unwrap()
            .iter()
            .all(|value| value.to_bits() == 0.875_f32.to_bits()));
    }
}
