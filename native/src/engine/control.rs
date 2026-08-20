//! One staged corrected-sensing and heterogeneous-neural control operation.
//!
//! The operation resolves due snakes against stable brain handles, samples all
//! observations from one immutable indexed world, evaluates every distinct
//! genome in Rust, and withholds both recurrent state and delivered-observation
//! boundaries until one complete preflight succeeds. Raw controller outputs
//! remain proposals for the Stage 5 controller-selection boundary.

use super::calculation::{
    CalculationBatchKey, CalculationCandidateIndex, CalculationError, CalculationExecutionBuffers,
    CalculationWorkUnit, CalculationWorkspace,
};
use super::inference::{
    evaluate_heterogeneous_population, evaluate_heterogeneous_population_with_resets,
    publish_heterogeneous_recurrent, validate_heterogeneous_recurrent_commit,
    ActivationCapturePlan, GraphExecutionPlan, HeterogeneousInferenceBuffers,
    HeterogeneousRecurrentReset, InferenceError,
};
use super::sensors::{
    ObservationDeliveryMarker, SensorError, SensorEvaluator, SensorGenerationState,
    SensorSampleDiagnostics, SensorScratch,
};
use super::spatial::IndexedSensorWorld;
use super::state::{BrainHandle, BrainOwner, BrainRuntimeState, PopulationGenome, WorldState};
use std::error::Error;
use std::fmt;
use std::mem::size_of;

/// Authoritative controller output width: turn and boost.
const CONTROLLER_OUTPUT_SIZE: usize = 2;

/// One successfully staged batch retained inside [`NeuralControlPipeline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadyBatch {
    key: CalculationBatchKey,
    active: usize,
}

/// Retained capacities used to verify warm sensing-to-inference stability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeuralControlCapacityDiagnostics {
    /// Packed observation slots.
    pub observations: usize,
    /// Packed controller-output slots.
    pub outputs: usize,
    /// Packed next-recurrent slots.
    pub recurrent: usize,
    /// Separate focused-capture output slots.
    pub capture_output: usize,
    /// Separate focused-capture recurrent slots.
    pub capture_recurrent: usize,
    /// Observation-delivery marker slots.
    pub deliveries: usize,
    /// Per-sample diagnostic slots.
    pub diagnostics: usize,
    /// Per-work-item explicit recurrent-reset flags.
    pub recurrent_reset_mask: usize,
    /// One exact zero recurrent block retained for ownership transitions.
    pub zero_recurrent: usize,
    /// Complete calculation workspace bytes retained at construction.
    pub calculation_workspace_bytes: usize,
}

/// Read-only view of one complete but not-yet-committed neural-control batch.
#[derive(Clone, Copy, Debug)]
pub struct NeuralControlBatchView<'a> {
    key: CalculationBatchKey,
    work: &'a [CalculationWorkUnit],
    observations: &'a [f32],
    outputs: &'a [f32],
    diagnostics: &'a [SensorSampleDiagnostics],
}

/// Immutable inputs for one stable corrected-sensing and inference boundary.
pub struct NeuralControlBatchInputs<'a, 'world> {
    /// Exact generation, step, and population epoch.
    pub key: CalculationBatchKey,
    /// Due snake/brain index pairs collected by the coordinator.
    pub candidates: &'a [CalculationCandidateIndex],
    /// Complete spatial indexes derived from this same immutable world.
    pub indexed_world: &'a IndexedSensorWorld<'world>,
    /// Generation-scoped best-points state initialized before sensing.
    pub generation: &'a SensorGenerationState,
    /// Dense heterogeneous genome slab.
    pub population: &'a [PopulationGenome],
    /// Stable brain handles and recurrent state before this operation.
    pub brains: &'a [BrainRuntimeState],
    /// Canonical brain handles whose first neural evaluation starts from zero.
    pub reset_brains: &'a [BrainHandle],
}

impl NeuralControlBatchView<'_> {
    /// Exact generation, step, and population epoch that produced this batch.
    pub const fn key(&self) -> CalculationBatchKey {
        self.key
    }

    /// Stable deterministic work order used by observations and outputs.
    pub const fn work(&self) -> &[CalculationWorkUnit] {
        self.work
    }

    /// Packed sensor-v3 observations in work order.
    pub const fn observations(&self) -> &[f32] {
        self.observations
    }

    /// Packed raw turn/boost graph outputs in work order.
    pub const fn outputs(&self) -> &[f32] {
        self.outputs
    }

    /// Per-snake sensing work and saturation diagnostics in work order.
    pub const fn diagnostics(&self) -> &[SensorSampleDiagnostics] {
        self.diagnostics
    }
}

/// Reusable single-worker Stage 4 sensing-to-inference staging owner.
///
/// Spatial indexes and [`SensorScratch`] are supplied by the owning world and
/// worker respectively so their separately admitted allocations can be reused.
/// The approved bounded calculation-worker path can later partition the same
/// canonical work and packed buffers without changing commit semantics.
#[derive(Debug)]
pub struct NeuralControlPipeline {
    sensor: SensorEvaluator,
    inference: GraphExecutionPlan,
    workspace: CalculationWorkspace<()>,
    observations: Vec<f32>,
    staged_outputs: Vec<f32>,
    staged_recurrent: Vec<f32>,
    capture_output: Vec<f32>,
    capture_recurrent: Vec<f32>,
    deliveries: Vec<Option<ObservationDeliveryMarker>>,
    diagnostics: Vec<SensorSampleDiagnostics>,
    recurrent_reset_mask: Vec<bool>,
    zero_recurrent: Vec<f32>,
    allocated_staging_bytes: usize,
    ready: Option<ReadyBatch>,
}

impl NeuralControlPipeline {
    /// Calculate fixed calculation and packed-staging bytes for one pipeline.
    pub fn required_staging_bytes(
        max_work: usize,
        inference: &GraphExecutionPlan,
    ) -> Result<usize, NeuralControlError> {
        let workspace =
            CalculationWorkspace::<()>::required_bytes(max_work, 1, inference.scratch_layout())?;
        let observations = checked_float_bytes(
            checked_product(max_work, inference.input_size(), "observation count")?,
            "observation bytes",
        )?;
        let outputs = checked_float_bytes(
            checked_product(max_work, inference.output_size(), "output count")?,
            "output bytes",
        )?;
        let recurrent = checked_float_bytes(
            checked_product(
                max_work,
                inference.total_state_size(),
                "recurrent staging count",
            )?,
            "recurrent staging bytes",
        )?;
        checked_sum(&[
            size_of::<Self>(),
            workspace,
            observations,
            outputs,
            recurrent,
            checked_float_bytes(inference.output_size(), "capture output bytes")?,
            checked_float_bytes(inference.total_state_size(), "capture recurrent bytes")?,
            checked_element_bytes::<Option<ObservationDeliveryMarker>>(
                max_work,
                "delivery marker bytes",
            )?,
            checked_element_bytes::<SensorSampleDiagnostics>(max_work, "sensor diagnostic bytes")?,
            checked_element_bytes::<bool>(max_work, "recurrent reset mask bytes")?,
            checked_float_bytes(inference.total_state_size(), "zero recurrent bytes")?,
        ])
    }

    /// Allocate fixed packed staging after checking the sensor/graph contract.
    ///
    /// `staging_budget_bytes` covers this owner, canonical work, graph scratch,
    /// observations, outputs, recurrent staging, markers and diagnostics. The
    /// caller separately admits its stable world indexes and reusable sensor
    /// scratch as part of the complete engine memory estimate.
    pub fn try_new(
        max_work: usize,
        sensor: SensorEvaluator,
        inference: GraphExecutionPlan,
        staging_budget_bytes: usize,
    ) -> Result<Self, NeuralControlError> {
        if sensor.layout().input_size != inference.input_size() {
            return Err(NeuralControlError::InputWidthMismatch {
                sensor: sensor.layout().input_size,
                graph: inference.input_size(),
            });
        }
        if inference.output_size() != CONTROLLER_OUTPUT_SIZE {
            return Err(NeuralControlError::OutputWidthMismatch {
                graph: inference.output_size(),
                required: CONTROLLER_OUTPUT_SIZE,
            });
        }
        let required = Self::required_staging_bytes(max_work, &inference)?;
        if required > staging_budget_bytes {
            return Err(NeuralControlError::StagingBudgetExceeded {
                required_bytes: required,
                budget_bytes: staging_budget_bytes,
            });
        }

        let workspace = CalculationWorkspace::try_new(
            max_work,
            1,
            inference.scratch_layout(),
            staging_budget_bytes,
        )?;
        let observations = try_filled(
            checked_product(max_work, inference.input_size(), "observation count")?,
            0.0,
            "observations",
        )?;
        let staged_outputs = try_filled(
            checked_product(max_work, inference.output_size(), "output count")?,
            0.0,
            "controller outputs",
        )?;
        let staged_recurrent = try_filled(
            checked_product(
                max_work,
                inference.total_state_size(),
                "recurrent staging count",
            )?,
            0.0,
            "recurrent staging",
        )?;
        let capture_output = try_filled(inference.output_size(), 0.0, "focused capture output")?;
        let capture_recurrent = try_filled(
            inference.total_state_size(),
            0.0,
            "focused capture recurrent staging",
        )?;
        let deliveries = try_filled(max_work, None, "delivery markers")?;
        let diagnostics = try_filled(
            max_work,
            SensorSampleDiagnostics::default(),
            "sensor diagnostics",
        )?;
        let recurrent_reset_mask = try_filled(max_work, false, "recurrent reset mask")?;
        let zero_recurrent = try_filled(inference.total_state_size(), 0.0, "zero recurrent state")?;
        let allocated_staging_bytes = checked_sum(&[
            size_of::<Self>(),
            workspace.allocated_bytes(),
            checked_capacity_bytes(&observations, "observation capacity")?,
            checked_capacity_bytes(&staged_outputs, "output capacity")?,
            checked_capacity_bytes(&staged_recurrent, "recurrent staging capacity")?,
            checked_capacity_bytes(&capture_output, "capture output capacity")?,
            checked_capacity_bytes(&capture_recurrent, "capture recurrent capacity")?,
            checked_capacity_bytes(&deliveries, "delivery marker capacity")?,
            checked_capacity_bytes(&diagnostics, "sensor diagnostic capacity")?,
            checked_capacity_bytes(&recurrent_reset_mask, "recurrent reset mask capacity")?,
            checked_capacity_bytes(&zero_recurrent, "zero recurrent capacity")?,
        ])?;
        if allocated_staging_bytes > staging_budget_bytes {
            return Err(NeuralControlError::StagingBudgetExceeded {
                required_bytes: allocated_staging_bytes,
                budget_bytes: staging_budget_bytes,
            });
        }

        Ok(Self {
            sensor,
            inference,
            workspace,
            observations,
            staged_outputs,
            staged_recurrent,
            capture_output,
            capture_recurrent,
            deliveries,
            diagnostics,
            recurrent_reset_mask,
            zero_recurrent,
            allocated_staging_bytes,
            ready: None,
        })
    }

    /// Actual retained staging bytes charged after allocation.
    pub const fn allocated_staging_bytes(&self) -> usize {
        self.allocated_staging_bytes
    }

    /// Report every retained top-level capacity without allocating or mutation.
    pub fn capacity_diagnostics(&self) -> NeuralControlCapacityDiagnostics {
        NeuralControlCapacityDiagnostics {
            observations: self.observations.capacity(),
            outputs: self.staged_outputs.capacity(),
            recurrent: self.staged_recurrent.capacity(),
            capture_output: self.capture_output.capacity(),
            capture_recurrent: self.capture_recurrent.capacity(),
            deliveries: self.deliveries.capacity(),
            diagnostics: self.diagnostics.capacity(),
            recurrent_reset_mask: self.recurrent_reset_mask.capacity(),
            zero_recurrent: self.zero_recurrent.capacity(),
            calculation_workspace_bytes: self.workspace.allocated_bytes(),
        }
    }

    /// Immutable corrected sensor-v3 evaluator.
    pub const fn sensor(&self) -> &SensorEvaluator {
        &self.sensor
    }

    /// Immutable complete-graph execution plan.
    pub const fn inference(&self) -> &GraphExecutionPlan {
        &self.inference
    }

    /// Sample one stable world boundary and evaluate every due neural brain.
    ///
    /// Failure leaves authoritative world and brain state untouched. A later
    /// call replaces any previously staged uncommitted batch.
    pub fn prepare_and_evaluate<'a>(
        &'a mut self,
        inputs: NeuralControlBatchInputs<'_, '_>,
        sensor_scratch: &mut SensorScratch,
    ) -> Result<NeuralControlBatchView<'a>, NeuralControlError> {
        let NeuralControlBatchInputs {
            key,
            candidates,
            indexed_world,
            generation,
            population,
            brains,
            reset_brains,
        } = inputs;
        self.ready = None;
        self.workspace.begin(key);
        for candidate in candidates {
            self.workspace
                .try_push_candidate(candidate.snake_index(), candidate.brain_index())?;
        }
        self.workspace
            .prepare(&indexed_world.world().snakes, brains, population)?;

        let active = self.workspace.prepared_work()?.len();
        self.prepare_recurrent_resets(reset_brains, active)?;
        let observation_count = checked_product(
            active,
            self.inference.input_size(),
            "active observation count",
        )?;
        let output_count =
            checked_product(active, self.inference.output_size(), "active output count")?;
        let recurrent_count = checked_product(
            active,
            self.inference.total_state_size(),
            "active recurrent count",
        )?;
        self.deliveries[..active].fill(None);
        self.diagnostics[..active].fill(SensorSampleDiagnostics::default());

        let CalculationExecutionBuffers {
            work, scratches, ..
        } = self.workspace.execution_buffers()?;
        let graph_scratch = scratches
            .first_mut()
            .ok_or(NeuralControlError::MissingCalculationScratch)?;
        for (ordinal, unit) in work.iter().enumerate() {
            let offset = ordinal * self.inference.input_size();
            let sample = self.sensor.sample(
                indexed_world,
                generation,
                unit.snake_index(),
                &mut self.observations[offset..offset + self.inference.input_size()],
                sensor_scratch,
            )?;
            self.deliveries[ordinal] = Some(sample.delivery);
            self.diagnostics[ordinal] = sample.diagnostics;
        }
        evaluate_heterogeneous_population_with_resets(
            &self.inference,
            work,
            population,
            brains,
            HeterogeneousRecurrentReset {
                mask: &self.recurrent_reset_mask[..active],
                zero_recurrent: &self.zero_recurrent,
            },
            HeterogeneousInferenceBuffers {
                observations: &self.observations[..observation_count],
                staged_outputs: &mut self.staged_outputs[..output_count],
                staged_recurrent: &mut self.staged_recurrent[..recurrent_count],
            },
            &mut graph_scratch.view(),
        )?;

        self.ready = Some(ReadyBatch { key, active });
        self.batch()
    }

    fn prepare_recurrent_resets(
        &mut self,
        reset_brains: &[BrainHandle],
        active: usize,
    ) -> Result<(), NeuralControlError> {
        self.recurrent_reset_mask[..active].fill(false);
        let work = self.workspace.prepared_work()?;
        let mut previous = None;
        for brain in reset_brains {
            if let Some(prior) = previous {
                if prior >= *brain {
                    return Err(NeuralControlError::NonCanonicalResetBrains {
                        previous: prior,
                        current: *brain,
                    });
                }
            }
            previous = Some(*brain);
            let ordinal = work
                .binary_search_by_key(brain, |unit| unit.brain())
                .map_err(|_| NeuralControlError::ResetBrainNotInBatch { brain: *brain })?;
            self.recurrent_reset_mask[ordinal] = true;
        }
        Ok(())
    }

    /// Read the current complete staged batch.
    pub fn batch(&self) -> Result<NeuralControlBatchView<'_>, NeuralControlError> {
        let ready = self.ready.ok_or(NeuralControlError::BatchNotReady)?;
        let work = self.workspace.prepared_work()?;
        if work.len() != ready.active {
            return Err(NeuralControlError::InternalLengthMismatch {
                buffer: "prepared work",
                actual: work.len(),
                expected: ready.active,
            });
        }
        let observation_count = checked_product(
            ready.active,
            self.inference.input_size(),
            "batch-view observation count",
        )?;
        let output_count = checked_product(
            ready.active,
            self.inference.output_size(),
            "batch-view output count",
        )?;
        Ok(NeuralControlBatchView {
            key: ready.key,
            work,
            observations: &self.observations[..observation_count],
            outputs: &self.staged_outputs[..output_count],
            diagnostics: &self.diagnostics[..ready.active],
        })
    }

    /// Re-evaluate only one explicitly focused due brain and copy one selected
    /// node activation. The ordinary batch path pays no capture overhead.
    pub fn capture_focused_activation(
        &mut self,
        brain: BrainHandle,
        capture: &ActivationCapturePlan,
        destination: &mut [f32],
        population: &[PopulationGenome],
        brains: &[BrainRuntimeState],
    ) -> Result<(), NeuralControlError> {
        let ready = self.ready.ok_or(NeuralControlError::BatchNotReady)?;
        let input_size = self.inference.input_size();
        let output_size = self.inference.output_size();
        let recurrent_size = self.inference.total_state_size();
        let CalculationExecutionBuffers {
            work, scratches, ..
        } = self.workspace.execution_buffers()?;
        let ordinal = work
            .iter()
            .position(|unit| unit.brain() == brain)
            .ok_or(NeuralControlError::FocusedBrainNotInBatch { brain })?;
        if ordinal >= ready.active {
            return Err(NeuralControlError::InternalLengthMismatch {
                buffer: "focused work ordinal",
                actual: ordinal,
                expected: ready.active,
            });
        }
        let reset_recurrent = self.recurrent_reset_mask[ordinal];
        let graph_scratch = scratches
            .first_mut()
            .ok_or(NeuralControlError::MissingCalculationScratch)?;
        let observation_offset = ordinal * input_size;
        let mut scratch_view = graph_scratch.view();
        let buffers = HeterogeneousInferenceBuffers {
            observations: &self.observations[observation_offset..observation_offset + input_size],
            staged_outputs: &mut self.capture_output[..output_size],
            staged_recurrent: &mut self.capture_recurrent[..recurrent_size],
        };
        if reset_recurrent {
            evaluate_heterogeneous_population_with_resets(
                &self.inference,
                &work[ordinal..ordinal + 1],
                population,
                brains,
                HeterogeneousRecurrentReset {
                    mask: std::slice::from_ref(&reset_recurrent),
                    zero_recurrent: &self.zero_recurrent,
                },
                buffers,
                &mut scratch_view,
            )?;
        } else {
            evaluate_heterogeneous_population(
                &self.inference,
                &work[ordinal..ordinal + 1],
                population,
                brains,
                buffers,
                &mut scratch_view,
            )?;
        }
        self.inference
            .capture_activation(capture, &scratch_view, destination)?;
        Ok(())
    }

    /// Atomically publish all delivered-observation and recurrent boundaries.
    ///
    /// The current key and every snake/brain identity are validated before the
    /// first write. Raw controller outputs remain staged for Stage 5 selection.
    pub fn commit_state(
        &mut self,
        current_key: CalculationBatchKey,
        world: &mut WorldState,
        brains: &mut [BrainRuntimeState],
    ) -> Result<(), NeuralControlError> {
        let ready = self.ready.ok_or(NeuralControlError::BatchNotReady)?;
        if ready.key != current_key {
            return Err(NeuralControlError::BatchKeyMismatch {
                staged: ready.key,
                current: current_key,
            });
        }
        let work = self.workspace.prepared_work()?;
        if work.len() != ready.active {
            return Err(NeuralControlError::InternalLengthMismatch {
                buffer: "prepared work",
                actual: work.len(),
                expected: ready.active,
            });
        }
        let recurrent_count = checked_product(
            ready.active,
            self.inference.total_state_size(),
            "commit recurrent count",
        )?;
        let staged_recurrent = &self.staged_recurrent[..recurrent_count];
        validate_heterogeneous_recurrent_commit(&self.inference, work, brains, staged_recurrent)?;

        for (ordinal, unit) in work.iter().enumerate() {
            let snake = world.snakes.get(unit.snake_index()).ok_or(
                NeuralControlError::SnakeIndexOutOfBounds {
                    index: unit.snake_index(),
                    snake_count: world.snakes.len(),
                },
            )?;
            if snake.id != unit.snake_id() {
                return Err(NeuralControlError::SnakeIdentityChanged {
                    index: unit.snake_index(),
                    expected: unit.snake_id(),
                    actual: snake.id,
                });
            }
            if snake.brain != Some(unit.brain()) {
                return Err(NeuralControlError::SnakeBrainChanged {
                    snake_id: snake.id,
                    expected: unit.brain(),
                    actual: snake.brain,
                });
            }
            if snake.population_slot != unit.population_slot() {
                return Err(NeuralControlError::PopulationSlotChanged {
                    snake_id: snake.id,
                    expected: unit.population_slot(),
                    actual: snake.population_slot,
                });
            }
            let brain = &brains[unit.brain_index()];
            let expected_owner = match unit.population_slot() {
                Some(slot) => BrainOwner::PopulationSlot(slot),
                None => BrainOwner::Entity(unit.snake_id()),
            };
            if brain.owner != expected_owner {
                return Err(NeuralControlError::BrainOwnerChanged {
                    brain: unit.brain(),
                    expected: expected_owner,
                    actual: brain.owner,
                });
            }
            let delivery = self.deliveries[ordinal]
                .ok_or(NeuralControlError::MissingDeliveryMarker { ordinal })?;
            delivery.validate(snake)?;
        }

        for (ordinal, unit) in work.iter().enumerate() {
            let delivery = self.deliveries[ordinal]
                .expect("complete neural-control delivery preflight invariant");
            delivery.commit_prevalidated(&mut world.snakes[unit.snake_index()]);
        }
        publish_heterogeneous_recurrent(&self.inference, work, brains, staged_recurrent);
        self.ready = None;
        Ok(())
    }

    /// Discard one complete staged batch without changing authority.
    pub fn discard(&mut self) {
        self.ready = None;
    }
}

/// Sensing-to-inference staging, identity, or commit failure.
#[derive(Debug)]
pub enum NeuralControlError {
    /// Canonical calculation-work failure.
    Calculation(Box<CalculationError>),
    /// Corrected sensor-v3 failure.
    Sensor(Box<SensorError>),
    /// Complete-graph or heterogeneous-inference failure.
    Inference(Box<InferenceError>),
    /// Sensor and graph input widths disagree.
    InputWidthMismatch { sensor: usize, graph: usize },
    /// The graph does not produce the turn/boost pair.
    OutputWidthMismatch { graph: usize, required: usize },
    /// Checked count or byte arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Fixed staging exceeds the caller-approved budget.
    StagingBudgetExceeded {
        required_bytes: usize,
        budget_bytes: usize,
    },
    /// A fixed staging allocation failed.
    AllocationFailed {
        buffer: &'static str,
        elements: usize,
    },
    /// No complete batch is available for inspection or commit.
    BatchNotReady,
    /// A different generation, step, or population epoch now owns authority.
    BatchKeyMismatch {
        staged: CalculationBatchKey,
        current: CalculationBatchKey,
    },
    /// The admitted calculation workspace unexpectedly has no worker scratch.
    MissingCalculationScratch,
    /// A requested focused brain was not due in this batch.
    FocusedBrainNotInBatch { brain: BrainHandle },
    /// Explicit recurrent-reset handles were not strictly canonical.
    NonCanonicalResetBrains {
        previous: BrainHandle,
        current: BrainHandle,
    },
    /// An explicit recurrent reset named a brain that is not due.
    ResetBrainNotInBatch { brain: BrainHandle },
    /// A staged delivery marker is unexpectedly absent.
    MissingDeliveryMarker { ordinal: usize },
    /// A staged snake array index no longer resolves.
    SnakeIndexOutOfBounds { index: usize, snake_count: usize },
    /// The snake at one dense index changed identity.
    SnakeIdentityChanged {
        index: usize,
        expected: u64,
        actual: u64,
    },
    /// The snake now references a different brain or no brain.
    SnakeBrainChanged {
        snake_id: u64,
        expected: BrainHandle,
        actual: Option<BrainHandle>,
    },
    /// The snake moved between evolved/entity ownership classes.
    PopulationSlotChanged {
        snake_id: u64,
        expected: Option<u32>,
        actual: Option<u32>,
    },
    /// The resolved brain changed population/entity ownership after staging.
    BrainOwnerChanged {
        brain: BrainHandle,
        expected: BrainOwner,
        actual: BrainOwner,
    },
    /// An internal fixed-buffer invariant was violated.
    InternalLengthMismatch {
        buffer: &'static str,
        actual: usize,
        expected: usize,
    },
}

impl From<CalculationError> for NeuralControlError {
    fn from(error: CalculationError) -> Self {
        Self::Calculation(Box::new(error))
    }
}

impl From<SensorError> for NeuralControlError {
    fn from(error: SensorError) -> Self {
        Self::Sensor(Box::new(error))
    }
}

impl From<InferenceError> for NeuralControlError {
    fn from(error: InferenceError) -> Self {
        Self::Inference(Box::new(error))
    }
}

impl fmt::Display for NeuralControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Calculation(error) => write!(formatter, "{error}"),
            Self::Sensor(error) => write!(formatter, "{error}"),
            Self::Inference(error) => write!(formatter, "{error}"),
            Self::InputWidthMismatch { sensor, graph } => write!(
                formatter,
                "sensor width {sensor} does not match graph input width {graph}"
            ),
            Self::OutputWidthMismatch { graph, required } => write!(
                formatter,
                "graph output width {graph} does not match controller width {required}"
            ),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "neural-control arithmetic overflow: {context}")
            }
            Self::StagingBudgetExceeded {
                required_bytes,
                budget_bytes,
            } => write!(
                formatter,
                "neural-control staging requires {required_bytes} bytes, exceeding budget {budget_bytes}"
            ),
            Self::AllocationFailed { buffer, elements } => {
                write!(formatter, "could not allocate {elements} elements for {buffer}")
            }
            Self::BatchNotReady => write!(formatter, "no complete neural-control batch is ready"),
            Self::BatchKeyMismatch { staged, current } => write!(
                formatter,
                "staged neural-control batch {staged:?} does not match current {current:?}"
            ),
            Self::MissingCalculationScratch => {
                write!(formatter, "neural-control workspace has no calculation scratch")
            }
            Self::FocusedBrainNotInBatch { brain } => {
                write!(formatter, "focused brain {brain:?} is not due in this batch")
            }
            Self::NonCanonicalResetBrains { previous, current } => write!(
                formatter,
                "recurrent reset brains are not canonical: {previous:?} before {current:?}"
            ),
            Self::ResetBrainNotInBatch { brain } => {
                write!(formatter, "recurrent reset brain {brain:?} is not due in this batch")
            }
            Self::MissingDeliveryMarker { ordinal } => {
                write!(formatter, "neural-control work {ordinal} has no delivery marker")
            }
            Self::SnakeIndexOutOfBounds { index, snake_count } => write!(
                formatter,
                "staged snake index {index} is outside {snake_count} records"
            ),
            Self::SnakeIdentityChanged {
                index,
                expected,
                actual,
            } => write!(
                formatter,
                "snake index {index} changed identity from {expected} to {actual}"
            ),
            Self::SnakeBrainChanged {
                snake_id,
                expected,
                actual,
            } => write!(
                formatter,
                "snake {snake_id} changed brain from {expected:?} to {actual:?}"
            ),
            Self::PopulationSlotChanged {
                snake_id,
                expected,
                actual,
            } => write!(
                formatter,
                "snake {snake_id} changed population slot from {expected:?} to {actual:?}"
            ),
            Self::BrainOwnerChanged {
                brain,
                expected,
                actual,
            } => write!(
                formatter,
                "brain {brain:?} changed owner from {expected:?} to {actual:?}"
            ),
            Self::InternalLengthMismatch {
                buffer,
                actual,
                expected,
            } => write!(
                formatter,
                "internal {buffer} length is {actual}; expected {expected}"
            ),
        }
    }
}

impl Error for NeuralControlError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Calculation(error) => Some(error.as_ref()),
            Self::Sensor(error) => Some(error.as_ref()),
            Self::Inference(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

fn checked_product(
    left: usize,
    right: usize,
    context: &'static str,
) -> Result<usize, NeuralControlError> {
    left.checked_mul(right)
        .ok_or(NeuralControlError::ArithmeticOverflow { context })
}

fn checked_sum(values: &[usize]) -> Result<usize, NeuralControlError> {
    values.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(*value)
            .ok_or(NeuralControlError::ArithmeticOverflow {
                context: "combined staging bytes",
            })
    })
}

fn checked_float_bytes(
    elements: usize,
    context: &'static str,
) -> Result<usize, NeuralControlError> {
    elements
        .checked_mul(size_of::<f32>())
        .ok_or(NeuralControlError::ArithmeticOverflow { context })
}

fn checked_element_bytes<T>(
    elements: usize,
    context: &'static str,
) -> Result<usize, NeuralControlError> {
    elements
        .checked_mul(size_of::<T>())
        .ok_or(NeuralControlError::ArithmeticOverflow { context })
}

fn checked_capacity_bytes<T>(
    values: &Vec<T>,
    context: &'static str,
) -> Result<usize, NeuralControlError> {
    checked_element_bytes::<T>(values.capacity(), context)
}

fn try_filled<T: Clone>(
    length: usize,
    value: T,
    buffer: &'static str,
) -> Result<Vec<T>, NeuralControlError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| NeuralControlError::AllocationFailed {
            buffer,
            elements: length,
        })?;
    values.resize(length, value);
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::{
        compile_graph, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef,
        GraphSpec,
    };
    use crate::engine::sensors::SensorConfig;
    use crate::engine::spatial::SensorIndexConfig;
    use crate::engine::state::{
        BaselineStrategyState, BodyRange, BrainOwner, GenomeLineage, PelletState, SnakeKind,
        SnakeState, WorldPoint,
    };

    const EPOCH: u64 = 31;

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

    fn sensor_config() -> SensorConfig {
        SensorConfig {
            bins: 8,
            ..SensorConfig::default()
        }
    }

    fn graph_plan(input_size: usize) -> GraphExecutionPlan {
        let graph = compile_graph(
            &GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".to_owned(),
                        kind: GraphNodeKind::Input {
                            output_size: input_size,
                        },
                    },
                    GraphNodeSpec {
                        id: "memory".to_owned(),
                        kind: GraphNodeKind::Rru {
                            input_size,
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

    fn snake(index: usize, handle: BrainHandle, body_start: usize) -> SnakeState {
        SnakeState {
            id: 1_000 + index as u64,
            frame_v1_id: u32::try_from(index + 1).unwrap(),
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(u32::try_from(index).unwrap()),
            brain: Some(handle),
            baseline_slot: None,
            baseline_strategy: None::<BaselineStrategyState>,
            position: WorldPoint {
                x: index as f64 * 180.0 - 90.0,
                y: index as f64 * 35.0,
            },
            previous_position: WorldPoint {
                x: index as f64 * 180.0 - 95.0,
                y: index as f64 * 35.0,
            },
            direction: index as f64 * 0.35,
            radius: 11.0,
            speed: 120.0,
            boost: index.is_multiple_of(2),
            age_seconds: 4.0 + index as f64,
            food: 2.0 + index as f64,
            points: 5.0 + index as f64 * 2.0,
            kills: index as u64,
            target_length: 8.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 1.0 + index as f64,
            body: BodyRange {
                start: body_start,
                len: 3,
            },
            skin: u32::try_from(index).unwrap(),
        }
    }

    fn fixture(
        plan: &GraphExecutionPlan,
        epoch: u64,
    ) -> (WorldState, Vec<BrainRuntimeState>, Vec<PopulationGenome>) {
        let mut body_points = Vec::new();
        let mut snakes = Vec::new();
        let mut brains = Vec::new();
        let mut population = Vec::new();
        for index in 0..3usize {
            let handle = BrainHandle {
                id: 400 + index as u64,
                epoch,
            };
            let body_start = body_points.len();
            let head = WorldPoint {
                x: index as f64 * 180.0 - 90.0,
                y: index as f64 * 35.0,
            };
            body_points.extend_from_slice(&[
                head,
                WorldPoint {
                    x: head.x - 18.0,
                    y: head.y - 2.0,
                },
                WorldPoint {
                    x: head.x - 36.0,
                    y: head.y - 5.0,
                },
            ]);
            snakes.push(snake(index, handle, body_start));
            brains.push(BrainRuntimeState {
                handle,
                owner: BrainOwner::PopulationSlot(u32::try_from(index).unwrap()),
                non_population_weights: None,
                recurrent: (0..plan.total_state_size())
                    .map(|state| (index * 5 + state + 1) as f32 / 100.0)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            });
            population.push(PopulationGenome {
                slot: u32::try_from(index).unwrap(),
                brain: handle,
                lineage: GenomeLineage {
                    genome_id: 900 + index as u64,
                    birth_generation: 1,
                    parent_a: None,
                    parent_b: None,
                },
                fitness: 0.0,
                weights: (0..plan.total_parameters())
                    .map(|weight| (((weight * 17 + index * 29) % 97) as f32 - 48.0) / 180.0)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            });
        }
        let pellets = vec![
            PelletState {
                id: 11,
                position: WorldPoint { x: 35.0, y: 80.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 12,
                position: WorldPoint {
                    x: -120.0,
                    y: -40.0,
                },
                value: 2.0,
                kind: 1,
                color: 2,
                owner: None,
            },
        ];
        (
            WorldState {
                snakes,
                body_points,
                pellets,
                controller_leases: Vec::new(),
            },
            brains,
            population,
        )
    }

    fn indexed(world: &WorldState) -> IndexedSensorWorld<'_> {
        IndexedSensorWorld::build(
            world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 10_000,
                maximum_pellet_entries: 1_000,
            },
        )
        .unwrap()
    }

    fn candidates() -> Vec<CalculationCandidateIndex> {
        [2, 0, 1]
            .into_iter()
            .map(|index| CalculationCandidateIndex::new(index, index))
            .collect()
    }

    fn pipeline(plan: GraphExecutionPlan) -> NeuralControlPipeline {
        let sensor = SensorEvaluator::new(sensor_config()).unwrap();
        NeuralControlPipeline::try_new(3, sensor, plan, usize::MAX).unwrap()
    }

    #[test]
    fn complete_batch_stages_observations_outputs_and_commits_both_boundaries() {
        let plan = graph_plan(51);
        let capture = plan.prepare_activation_capture("memory").unwrap();
        let (mut world, mut brains, population) = fixture(&plan, EPOCH);
        let recurrent_before = brains
            .iter()
            .map(|brain| brain.recurrent.clone())
            .collect::<Vec<_>>();
        let delivered_before = world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&world).unwrap();
        let old_indexed = indexed(&world);
        let mut sensor_scratch = SensorScratch::default();
        let mut pipeline = pipeline(plan);
        let key = CalculationBatchKey::new(4, 99, EPOCH);
        let batch = pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &old_indexed,
                    generation: &generation,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[],
                },
                &mut sensor_scratch,
            )
            .unwrap();
        assert_eq!(batch.key(), key);
        assert_eq!(batch.work().len(), 3);
        assert_eq!(batch.observations().len(), 3 * 51);
        assert_eq!(batch.outputs().len(), 3 * 2);
        assert!(batch.outputs().iter().all(|value| value.is_finite()));
        assert_eq!(batch.diagnostics().len(), 3);
        assert_eq!(
            world
                .snakes
                .iter()
                .map(|snake| snake.delivered_observation_points)
                .collect::<Vec<_>>(),
            delivered_before
        );
        assert!(brains
            .iter()
            .zip(&recurrent_before)
            .all(|(brain, before)| brain.recurrent == *before));

        let focused = batch.work()[1].brain();
        let outputs_before_capture = batch.outputs().to_vec();
        let _ = batch;
        let mut captured = vec![f32::NAN; capture.len()];
        pipeline
            .capture_focused_activation(focused, &capture, &mut captured, &population, &brains)
            .unwrap();
        assert!(captured.iter().all(|value| value.is_finite()));
        assert_eq!(pipeline.batch().unwrap().outputs(), outputs_before_capture);

        drop(old_indexed);
        pipeline.commit_state(key, &mut world, &mut brains).unwrap();
        for snake in &world.snakes {
            assert_eq!(snake.delivered_observation_points, snake.points);
        }
        assert!(brains
            .iter()
            .zip(recurrent_before)
            .all(|(brain, before)| brain.recurrent != before));
        assert!(matches!(
            pipeline.batch(),
            Err(NeuralControlError::BatchNotReady)
        ));
    }

    #[test]
    fn explicit_takeover_reset_matches_a_zero_state_brain_without_touching_others() {
        let plan = graph_plan(51);
        let capture = plan.prepare_activation_capture("memory").unwrap();
        let (mut reset_world, mut reset_brains, population) = fixture(&plan, EPOCH);
        let mut reference_world = reset_world.clone();
        let mut reference_brains = reset_brains.clone();
        let reset_handle = reset_brains[1].handle;
        let source_reset_state = reset_brains[1].recurrent.clone();
        reference_brains[1].recurrent.fill(0.0);
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&reset_world).unwrap();
        let reset_indexed = indexed(&reset_world);
        let reference_indexed = indexed(&reference_world);
        let mut reset_pipeline = pipeline(plan.clone());
        let mut reference_pipeline = pipeline(plan);
        let mut reset_sensor_scratch = SensorScratch::default();
        let mut reference_sensor_scratch = SensorScratch::default();
        let key = CalculationBatchKey::new(4, 100, EPOCH);

        let reset_outputs = reset_pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &reset_indexed,
                    generation: &generation,
                    population: &population,
                    brains: &reset_brains,
                    reset_brains: &[reset_handle],
                },
                &mut reset_sensor_scratch,
            )
            .unwrap()
            .outputs()
            .to_vec();
        let reference_outputs = reference_pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &reference_indexed,
                    generation: &generation,
                    population: &population,
                    brains: &reference_brains,
                    reset_brains: &[],
                },
                &mut reference_sensor_scratch,
            )
            .unwrap()
            .outputs()
            .to_vec();
        assert_eq!(reset_outputs, reference_outputs);
        assert_eq!(reset_brains[1].recurrent, source_reset_state);

        let mut reset_activation = vec![f32::NAN; capture.len()];
        let mut reference_activation = vec![f32::NAN; capture.len()];
        reset_pipeline
            .capture_focused_activation(
                reset_handle,
                &capture,
                &mut reset_activation,
                &population,
                &reset_brains,
            )
            .unwrap();
        reference_pipeline
            .capture_focused_activation(
                reset_handle,
                &capture,
                &mut reference_activation,
                &population,
                &reference_brains,
            )
            .unwrap();
        assert_eq!(reset_activation, reference_activation);

        drop(reset_indexed);
        drop(reference_indexed);
        reset_pipeline
            .commit_state(key, &mut reset_world, &mut reset_brains)
            .unwrap();
        reference_pipeline
            .commit_state(key, &mut reference_world, &mut reference_brains)
            .unwrap();
        assert_eq!(reset_world, reference_world);
        assert_eq!(reset_brains, reference_brains);
        assert_ne!(reset_brains[1].recurrent, source_reset_state);
    }

    #[test]
    fn invalid_reset_sets_reject_without_ready_batch_or_authoritative_writes() {
        let plan = graph_plan(51);
        let (world, brains, population) = fixture(&plan, EPOCH);
        let source_world = world.clone();
        let source_brains = brains.clone();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&world).unwrap();
        let indexed = indexed(&world);
        let mut pipeline = pipeline(plan);
        let mut sensor_scratch = SensorScratch::default();
        let key = CalculationBatchKey::new(4, 101, EPOCH);
        let mut handles = brains.iter().map(|brain| brain.handle).collect::<Vec<_>>();
        handles.sort_unstable();

        for invalid in [vec![handles[0], handles[0]], vec![handles[2], handles[0]]] {
            assert!(matches!(
                pipeline.prepare_and_evaluate(
                    NeuralControlBatchInputs {
                        key,
                        candidates: &candidates(),
                        indexed_world: &indexed,
                        generation: &generation,
                        population: &population,
                        brains: &brains,
                        reset_brains: &invalid,
                    },
                    &mut sensor_scratch,
                ),
                Err(NeuralControlError::NonCanonicalResetBrains { .. })
            ));
            assert!(matches!(
                pipeline.batch(),
                Err(NeuralControlError::BatchNotReady)
            ));
            assert_eq!(world, source_world);
            assert_eq!(brains, source_brains);
        }

        let stale = BrainHandle {
            id: u64::MAX - 1,
            epoch: EPOCH,
        };
        assert!(matches!(
            pipeline.prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &indexed,
                    generation: &generation,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[stale],
                },
                &mut sensor_scratch,
            ),
            Err(NeuralControlError::ResetBrainNotInBatch { brain }) if brain == stale
        ));
        assert!(matches!(
            pipeline.batch(),
            Err(NeuralControlError::BatchNotReady)
        ));
        assert_eq!(world, source_world);
        assert_eq!(brains, source_brains);
    }

    #[test]
    fn late_delivery_failure_prevents_every_delivery_and_recurrent_write() {
        let plan = graph_plan(51);
        let (mut world, mut brains, population) = fixture(&plan, EPOCH);
        let recurrent_before = brains
            .iter()
            .map(|brain| brain.recurrent.clone())
            .collect::<Vec<_>>();
        let delivered_before = world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&world).unwrap();
        let original_indexed = indexed(&world);
        let mut pipeline = pipeline(plan);
        let mut sensor_scratch = SensorScratch::default();
        let key = CalculationBatchKey::new(4, 100, EPOCH);
        pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &original_indexed,
                    generation: &generation,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[],
                },
                &mut sensor_scratch,
            )
            .unwrap();
        drop(original_indexed);
        world.snakes[2].delivered_observation_points += 0.5;

        assert!(matches!(
            pipeline.commit_state(key, &mut world, &mut brains),
            Err(NeuralControlError::Sensor(error))
                if matches!(*error, SensorError::StaleDeliveryMarker { .. })
        ));
        assert_eq!(
            world.snakes[0].delivered_observation_points,
            delivered_before[0]
        );
        assert_eq!(
            world.snakes[1].delivered_observation_points,
            delivered_before[1]
        );
        assert_eq!(
            world.snakes[2].delivered_observation_points,
            delivered_before[2] + 0.5
        );
        assert!(brains
            .iter()
            .zip(recurrent_before)
            .all(|(brain, before)| brain.recurrent == before));
    }

    #[test]
    fn malformed_late_genome_leaves_no_ready_batch_or_authoritative_write() {
        let plan = graph_plan(51);
        let (world, brains, mut population) = fixture(&plan, EPOCH);
        population[2].weights = vec![0.0; plan.total_parameters() - 1].into_boxed_slice();
        let recurrent_before = brains
            .iter()
            .map(|brain| brain.recurrent.clone())
            .collect::<Vec<_>>();
        let delivered_before = world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>();
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&world).unwrap();
        let original_indexed = indexed(&world);
        let mut pipeline = pipeline(plan);
        let mut sensor_scratch = SensorScratch::default();
        let key = CalculationBatchKey::new(4, 101, EPOCH);

        assert!(matches!(
            pipeline.prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key,
                    candidates: &candidates(),
                    indexed_world: &original_indexed,
                    generation: &generation,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[],
                },
                &mut sensor_scratch,
            ),
            Err(NeuralControlError::Inference(error))
                if matches!(*error, InferenceError::BufferLength {
                    buffer: "brain weights",
                    ..
                })
        ));
        assert!(matches!(
            pipeline.batch(),
            Err(NeuralControlError::BatchNotReady)
        ));
        assert_eq!(
            world
                .snakes
                .iter()
                .map(|snake| snake.delivered_observation_points)
                .collect::<Vec<_>>(),
            delivered_before
        );
        assert!(brains
            .iter()
            .zip(recurrent_before)
            .all(|(brain, before)| brain.recurrent == before));
    }

    #[test]
    fn warmed_repeated_batches_reuse_pipeline_and_sensor_capacities() {
        let plan = graph_plan(51);
        let (mut world, mut brains, population) = fixture(&plan, EPOCH);
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&world).unwrap();
        let mut pipeline = pipeline(plan);
        let mut sensor_scratch = SensorScratch::default();
        let due = candidates();
        let mut retained = None;

        for step in 1..=24_u64 {
            let key = CalculationBatchKey::new(7, step, EPOCH);
            let current_indexed = indexed(&world);
            pipeline
                .prepare_and_evaluate(
                    NeuralControlBatchInputs {
                        key,
                        candidates: &due,
                        indexed_world: &current_indexed,
                        generation: &generation,
                        population: &population,
                        brains: &brains,
                        reset_brains: &[],
                    },
                    &mut sensor_scratch,
                )
                .unwrap();
            drop(current_indexed);
            pipeline.commit_state(key, &mut world, &mut brains).unwrap();

            let capacities = (
                pipeline.capacity_diagnostics(),
                sensor_scratch.diagnostics(),
            );
            if let Some(expected) = retained {
                assert_eq!(capacities, expected);
            } else {
                retained = Some(capacities);
            }
        }
    }

    #[test]
    fn population_replacement_rejects_old_epoch_then_accepts_zeroed_new_epoch() {
        let plan = graph_plan(51);
        let (world, brains, population) = fixture(&plan, EPOCH);
        let mut generation = SensorGenerationState::new();
        generation.update_after_step(&world).unwrap();
        let original_indexed = indexed(&world);
        let mut pipeline = pipeline(plan.clone());
        let mut sensor_scratch = SensorScratch::default();
        let old_key = CalculationBatchKey::new(5, 1, EPOCH);
        pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key: old_key,
                    candidates: &candidates(),
                    indexed_world: &original_indexed,
                    generation: &generation,
                    population: &population,
                    brains: &brains,
                    reset_brains: &[],
                },
                &mut sensor_scratch,
            )
            .unwrap();
        drop(original_indexed);

        let new_epoch = EPOCH + 1;
        let mut replacement_world = world.clone();
        let mut replacement_brains = brains.clone();
        let mut replacement_population = population.clone();
        for index in 0..replacement_brains.len() {
            let handle = BrainHandle {
                id: replacement_brains[index].handle.id + 10_000,
                epoch: new_epoch,
            };
            replacement_brains[index].handle = handle;
            replacement_brains[index].recurrent.fill(0.0);
            replacement_population[index].brain = handle;
            replacement_world.snakes[index].brain = Some(handle);
        }
        let replacement_recurrent = replacement_brains
            .iter()
            .map(|brain| brain.recurrent.clone())
            .collect::<Vec<_>>();
        let replacement_delivered = replacement_world
            .snakes
            .iter()
            .map(|snake| snake.delivered_observation_points)
            .collect::<Vec<_>>();
        let new_key = CalculationBatchKey::new(6, 1, new_epoch);
        assert!(matches!(
            pipeline.commit_state(new_key, &mut replacement_world, &mut replacement_brains),
            Err(NeuralControlError::BatchKeyMismatch { .. })
        ));
        assert_eq!(
            replacement_world
                .snakes
                .iter()
                .map(|snake| snake.delivered_observation_points)
                .collect::<Vec<_>>(),
            replacement_delivered
        );
        assert!(replacement_brains
            .iter()
            .zip(&replacement_recurrent)
            .all(|(brain, before)| brain.recurrent == *before));
        assert!(replacement_brains
            .iter()
            .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));

        assert!(matches!(
            pipeline.commit_state(
                old_key,
                &mut replacement_world,
                &mut replacement_brains
            ),
            Err(NeuralControlError::Inference(error))
                if matches!(*error, InferenceError::BrainHandleMismatch { .. })
        ));
        assert_eq!(
            replacement_world
                .snakes
                .iter()
                .map(|snake| snake.delivered_observation_points)
                .collect::<Vec<_>>(),
            replacement_delivered
        );
        assert!(replacement_brains
            .iter()
            .all(|brain| brain.recurrent.iter().all(|value| *value == 0.0)));

        let mut replacement_generation = SensorGenerationState::new();
        replacement_generation
            .update_after_step(&replacement_world)
            .unwrap();
        let replacement_indexed = indexed(&replacement_world);
        pipeline
            .prepare_and_evaluate(
                NeuralControlBatchInputs {
                    key: new_key,
                    candidates: &candidates(),
                    indexed_world: &replacement_indexed,
                    generation: &replacement_generation,
                    population: &replacement_population,
                    brains: &replacement_brains,
                    reset_brains: &[],
                },
                &mut sensor_scratch,
            )
            .unwrap();
        drop(replacement_indexed);
        pipeline
            .commit_state(new_key, &mut replacement_world, &mut replacement_brains)
            .unwrap();
        assert!(replacement_world
            .snakes
            .iter()
            .all(|snake| snake.delivered_observation_points == snake.points));
        assert!(replacement_brains
            .iter()
            .zip(replacement_recurrent)
            .all(|(brain, before)| brain.recurrent != before));
    }
}
