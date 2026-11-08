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
use super::calculation_workers::{validate_calculation_workers, CalculationWorkerCountError};
use super::inference::{
    evaluate_heterogeneous_population, evaluate_heterogeneous_population_with_resets,
    publish_heterogeneous_recurrent, validate_heterogeneous_recurrent_commit,
    ActivationCapturePlan, GraphExecutionPlan, HeterogeneousInferenceBuffers,
    HeterogeneousRecurrentReset, InferenceError, VisualizationCapturePlan,
};
use super::sensors::{
    ObservationDeliveryMarker, SensorError, SensorEvaluator, SensorGenerationState,
    SensorSampleDiagnostics, SensorScratch,
};
use super::spatial::IndexedSensorWorld;
use super::state::{BrainHandle, BrainOwner, BrainRuntimeState, PopulationGenome, WorldState};
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::error::Error;
use std::fmt;
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

/// Authoritative controller output width: turn and boost.
const CONTROLLER_OUTPUT_SIZE: usize = 2;

/// Borrowed independent sensor receipts plus the same batch's bounded work totals.
pub(crate) type IndependentSensorSampleBatch<'a> = (
    &'a [Option<ObservationDeliveryMarker>],
    &'a [SensorSampleDiagnostics],
    super::generation_work::SensorWork,
);

/// Wall time around the two worker-pool phases in one neural batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NeuralBatchCostMicros {
    pub work: super::generation_work::StepWork,
    pub sensing: u64,
    pub inference: u64,
}

/// Carry calculation phase and partition identity to the coordinator's panic root.
fn run_calculation_worker<T>(
    phase: &'static str,
    partition: usize,
    operation: impl FnOnce() -> T,
) -> T {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(_) => panic!("calculation {phase} worker partition {partition} panicked"),
    }
}

/// One successfully staged batch retained inside [`NeuralControlPipeline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadyBatch {
    key: CalculationBatchKey,
    active: usize,
}

/// Retained capacities used to verify warm sensing-to-inference stability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NeuralControlCapacityDiagnostics {
    /// Persistent calculation threads allocated for this pipeline.
    pub calculation_workers: usize,
    /// Derived i16 numeric storage; canonical Float32 parameters remain separate.
    pub inference_cache_bytes: usize,
    /// One shared cold-admitted v4 head-velocity array; zero for frozen v3.
    pub v4_head_capacity: usize,
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
    cost_micros: NeuralBatchCostMicros,
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
    /// Optional dense life assessments from the same pre-control boundary.
    pub assessment_points: Option<&'a [f64]>,
    /// Bound v4 context and immutable grants; absent for frozen v3.
    pub v4: Option<super::sensors_v4::PreparedV4Sensing<'a>>,
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

    /// Packed active-contract observations in work order.
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

    /// Coarse sensing and inference wall time for this staged batch.
    pub(crate) const fn cost_micros(&self) -> NeuralBatchCostMicros {
        self.cost_micros
    }
}

/// Reusable bounded-worker sensing-to-inference staging owner.
///
/// Spatial indexes and [`SensorScratch`] are supplied by the owning world and
/// worker respectively so their separately admitted allocations can be reused.
/// The approved bounded calculation-worker path can later partition the same
/// canonical work and packed buffers without changing commit semantics.
#[derive(Debug)]
pub struct NeuralControlPipeline {
    sensor: SensorEvaluator,
    v4_heads: super::sensors_v4::V4HeadWorkspace,
    inference: GraphExecutionPlan,
    workspace: CalculationWorkspace<()>,
    calculation_pool: Option<ThreadPool>,
    parallel_sensor_scratch: Vec<SensorScratch>,
    parallel_sensor_errors: Vec<Option<SensorError>>,
    parallel_inference_errors: Vec<Option<InferenceError>>,
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
    last_cost: NeuralBatchCostMicros,
    #[cfg(any(test, feature = "engine-test-hooks"))]
    panic_inference_partition_once: Option<usize>,
}

impl NeuralControlPipeline {
    /// Arm a real Rayon partition only in an explicit test build.
    #[cfg(feature = "engine-test-hooks")]
    pub(crate) fn arm_calculation_panic_for_test(&mut self) -> Result<(), &'static str> {
        if self.calculation_pool.is_none() || self.parallel_inference_errors.len() < 2 {
            return Err("calculation panic fixture requires at least two workers");
        }
        self.panic_inference_partition_once = Some(1);
        Ok(())
    }

    /// Calculate fixed calculation and packed-staging bytes for one pipeline.
    pub fn required_staging_bytes(
        max_work: usize,
        inference: &GraphExecutionPlan,
        calculation_workers: usize,
    ) -> Result<usize, NeuralControlError> {
        validate_calculation_workers(calculation_workers)
            .map_err(NeuralControlError::InvalidCalculationWorkers)?;
        Self::required_staging_bytes_for_validated_workers(max_work, inference, calculation_workers)
    }

    /// Calculate fixed staging after the caller has admitted the worker count.
    ///
    /// Nested sensor-query buffers grow inside the separately declared worker-scratch
    /// reservation. Charging an additional fixed allowance here double-counts that
    /// reservation and can make an otherwise admitted CPU count impossible to start.
    fn required_staging_bytes_for_validated_workers(
        max_work: usize,
        inference: &GraphExecutionPlan,
        calculation_workers: usize,
    ) -> Result<usize, NeuralControlError> {
        debug_assert!(calculation_workers > 0);
        let workspace = CalculationWorkspace::<()>::required_bytes(
            max_work,
            calculation_workers,
            inference.scratch_layout(),
        )?;
        let additional_workers = calculation_workers - 1;
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
            checked_element_bytes::<SensorScratch>(additional_workers, "parallel sensor scratch")?,
            checked_element_bytes::<Option<SensorError>>(
                calculation_workers,
                "parallel sensor errors",
            )?,
            checked_element_bytes::<Option<InferenceError>>(
                calculation_workers,
                "parallel inference errors",
            )?,
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

    /// Include the optional shared v4 head facts in the same unchanged scratch ceiling.
    pub fn required_staging_bytes_for_sensor(
        max_work: usize,
        sensor: &SensorEvaluator,
        inference: &GraphExecutionPlan,
        calculation_workers: usize,
    ) -> Result<usize, NeuralControlError> {
        checked_sum(&[
            Self::required_staging_bytes(max_work, inference, calculation_workers)?,
            if sensor.is_v4() {
                checked_element_bytes::<super::state::WorldPoint>(max_work, "shared v4 head facts")?
            } else {
                0
            },
        ])
    }

    /// Temporarily lend the admitted shared owner to the pre-movement coordinator.
    /// The coordinator returns it after all borrowed worker views have expired.
    pub(crate) fn swap_v4_head_workspace(
        &mut self,
        other: &mut super::sensors_v4::V4HeadWorkspace,
    ) {
        std::mem::swap(&mut self.v4_heads, other);
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
        calculation_workers: usize,
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
        let required = Self::required_staging_bytes_for_sensor(
            max_work,
            &sensor,
            &inference,
            calculation_workers,
        )?;
        if required > staging_budget_bytes {
            return Err(NeuralControlError::StagingBudgetExceeded {
                required_bytes: required,
                budget_bytes: staging_budget_bytes,
            });
        }

        let v4_heads = if sensor.is_v4() {
            super::sensors_v4::V4HeadWorkspace::try_new(max_work).map_err(SensorError::V4)?
        } else {
            super::sensors_v4::V4HeadWorkspace::default()
        };
        let workspace = CalculationWorkspace::try_new(
            max_work,
            calculation_workers,
            inference.scratch_layout(),
            staging_budget_bytes,
        )?;
        let calculation_pool = if calculation_workers == 1 {
            None
        } else {
            Some(
                ThreadPoolBuilder::new()
                    .num_threads(calculation_workers)
                    .thread_name(|index| format!("slither-calc-{index}"))
                    .build()
                    .map_err(|error| NeuralControlError::WorkerPool(error.to_string()))?,
            )
        };
        let parallel_sensor_scratch = try_filled(
            calculation_workers - 1,
            SensorScratch::default(),
            "parallel sensor scratch",
        )?;
        let mut parallel_sensor_errors = Vec::new();
        parallel_sensor_errors
            .try_reserve_exact(calculation_workers)
            .map_err(|_| NeuralControlError::AllocationFailed {
                buffer: "parallel sensor errors",
                elements: calculation_workers,
            })?;
        parallel_sensor_errors.resize_with(calculation_workers, || None);
        let mut parallel_inference_errors = Vec::new();
        parallel_inference_errors
            .try_reserve_exact(calculation_workers)
            .map_err(|_| NeuralControlError::AllocationFailed {
                buffer: "parallel inference errors",
                elements: calculation_workers,
            })?;
        parallel_inference_errors.resize_with(calculation_workers, || None);
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
            checked_element_bytes::<super::state::WorldPoint>(
                v4_heads.capacity(),
                "shared v4 head capacity",
            )?,
            checked_capacity_bytes(&parallel_sensor_scratch, "parallel sensor scratch records")?,
            checked_capacity_bytes(&parallel_sensor_errors, "parallel sensor error records")?,
            checked_capacity_bytes(
                &parallel_inference_errors,
                "parallel inference error records",
            )?,
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
            v4_heads,
            inference,
            workspace,
            calculation_pool,
            parallel_sensor_scratch,
            parallel_sensor_errors,
            parallel_inference_errors,
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
            last_cost: NeuralBatchCostMicros::default(),
            #[cfg(any(test, feature = "engine-test-hooks"))]
            panic_inference_partition_once: None,
        })
    }

    /// Actual retained staging bytes charged after allocation.
    pub const fn allocated_staging_bytes(&self) -> usize {
        self.allocated_staging_bytes
    }

    /// Report every retained top-level capacity without allocating or mutation.
    pub fn capacity_diagnostics(&self) -> NeuralControlCapacityDiagnostics {
        NeuralControlCapacityDiagnostics {
            calculation_workers: self.workspace.worker_count(),
            inference_cache_bytes: self.inference.inference_cache_bytes(),
            v4_head_capacity: self.v4_heads.capacity(),
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

    /// Sample a sensing-only batch through the existing persistent calculation pool.
    ///
    /// Results stay in caller-owned observation storage while delivery markers and diagnostics use
    /// the pipeline's already-admitted staging slots. This lets external controllers share the same
    /// bounded sensor workers without pretending to own local brains or recurrent state.
    // The separate borrowed world/generation/buffer inputs keep this hot-path helper
    // allocation-free and make each ownership boundary explicit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sample_independent_observations<'a, T, F>(
        &'a mut self,
        items: &[T],
        snake_index: F,
        indexed_world: &IndexedSensorWorld<'_>,
        generation: &SensorGenerationState,
        v4: Option<super::sensors_v4::PreparedV4Sensing<'_>>,
        observations: &mut [f32],
        sensor_scratch: &mut SensorScratch,
        mut trace: Option<(u64, &mut super::sensor_trace::SensorTrace)>,
    ) -> Result<IndependentSensorSampleBatch<'a>, NeuralControlError>
    where
        T: Sync,
        F: Fn(&T) -> usize + Sync,
    {
        let active = items.len();
        if active > self.deliveries.len() {
            return Err(NeuralControlError::InternalLengthMismatch {
                buffer: "independent sensor items",
                actual: active,
                expected: self.deliveries.len(),
            });
        }
        let width = self.sensor.layout().input_size;
        let expected_observations =
            checked_product(active, width, "independent observation count")?;
        if observations.len() != expected_observations {
            return Err(NeuralControlError::InternalLengthMismatch {
                buffer: "independent observations",
                actual: observations.len(),
                expected: expected_observations,
            });
        }
        self.deliveries[..active].fill(None);
        self.diagnostics[..active].fill(SensorSampleDiagnostics::default());
        if let Some((_, value)) = trace.as_mut() {
            value.valid = false;
        }
        sensor_scratch.work = Default::default();
        for scratch in &mut self.parallel_sensor_scratch {
            scratch.work = Default::default();
        }
        if active == 0 {
            return Ok((
                &self.deliveries[..0],
                &self.diagnostics[..0],
                Default::default(),
            ));
        }

        let calculation_workers = self.workspace.worker_count();
        if let Some(pool) = self.calculation_pool.as_ref().filter(|_| active >= 2) {
            let partitions = active.min(calculation_workers);
            let base = active / partitions;
            let remainder = active % partitions;
            let sensor = &self.sensor;
            let world = indexed_world.world();
            let snake_index = &snake_index;
            self.parallel_sensor_errors[..partitions].fill(None);
            pool.scope(|scope| {
                let mut remaining_items = items;
                let mut remaining_observations = &mut observations[..expected_observations];
                let mut remaining_deliveries = &mut self.deliveries[..active];
                let mut remaining_diagnostics = &mut self.diagnostics[..active];
                let scratches = std::iter::once(&mut *sensor_scratch)
                    .chain(self.parallel_sensor_scratch.iter_mut());
                let errors = self.parallel_sensor_errors[..partitions].iter_mut();
                for (partition, (scratch, error)) in
                    scratches.zip(errors).take(partitions).enumerate()
                {
                    let count = base + usize::from(partition < remainder);
                    let (partition_items, rest_items) = remaining_items.split_at(count);
                    remaining_items = rest_items;
                    let (partition_observations, rest_observations) =
                        remaining_observations.split_at_mut(count * width);
                    remaining_observations = rest_observations;
                    let (partition_deliveries, rest_deliveries) =
                        remaining_deliveries.split_at_mut(count);
                    remaining_deliveries = rest_deliveries;
                    let (partition_diagnostics, rest_diagnostics) =
                        remaining_diagnostics.split_at_mut(count);
                    remaining_diagnostics = rest_diagnostics;
                    let mut partition_trace = if trace.as_ref().is_some_and(|(id, _)| {
                        partition_items.iter().any(|item| {
                            world
                                .snakes
                                .get(snake_index(item))
                                .is_some_and(|snake| snake.id == *id)
                        })
                    }) {
                        trace.take()
                    } else {
                        None
                    };
                    scope.spawn(move |_| {
                        run_calculation_worker("independent sensing", partition, || {
                            for (ordinal, item) in partition_items.iter().enumerate() {
                                let index = snake_index(item);
                                let offset = ordinal * width;
                                match sensor.sample_prepared_with_trace(
                                    indexed_world,
                                    generation,
                                    index,
                                    &mut partition_observations[offset..offset + width],
                                    scratch,
                                    None,
                                    v4,
                                    partition_trace.as_mut().and_then(|(id, value)| {
                                        world
                                            .snakes
                                            .get(index)
                                            .is_some_and(|snake| snake.id == *id)
                                            .then_some(&mut **value)
                                    }),
                                ) {
                                    Ok(sample) => {
                                        partition_deliveries[ordinal] = Some(sample.delivery);
                                        partition_diagnostics[ordinal] = sample.diagnostics;
                                    }
                                    Err(failure) => {
                                        *error = Some(failure);
                                        break;
                                    }
                                }
                            }
                        });
                    });
                }
            });
            for error in &mut self.parallel_sensor_errors[..partitions] {
                if let Some(failure) = error.take() {
                    return Err(failure.into());
                }
            }
        } else {
            let world = indexed_world.world();
            for (ordinal, item) in items.iter().enumerate() {
                let index = snake_index(item);
                let offset = ordinal * width;
                let sample = self.sensor.sample_prepared_with_trace(
                    indexed_world,
                    generation,
                    index,
                    &mut observations[offset..offset + width],
                    sensor_scratch,
                    None,
                    v4,
                    trace.as_mut().and_then(|(id, value)| {
                        world
                            .snakes
                            .get(index)
                            .is_some_and(|snake| snake.id == *id)
                            .then_some(&mut **value)
                    }),
                )?;
                self.deliveries[ordinal] = Some(sample.delivery);
                self.diagnostics[ordinal] = sample.diagnostics;
            }
        }
        let mut work = sensor_scratch.work;
        for scratch in &self.parallel_sensor_scratch {
            work.merge(scratch.work);
        }
        Ok((
            &self.deliveries[..active],
            &self.diagnostics[..active],
            work,
        ))
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
        self.prepare_and_evaluate_traced(inputs, sensor_scratch, None)
    }

    /// Capture raw diagnostics in the same sampling pass, in exactly one worker.
    pub(crate) fn prepare_and_evaluate_traced<'a>(
        &'a mut self,
        inputs: NeuralControlBatchInputs<'_, '_>,
        sensor_scratch: &mut SensorScratch,
        mut trace: Option<(u64, &mut super::sensor_trace::SensorTrace)>,
    ) -> Result<NeuralControlBatchView<'a>, NeuralControlError> {
        if let Some((_, value)) = trace.as_mut() {
            value.valid = false;
        }
        let NeuralControlBatchInputs {
            key,
            candidates,
            indexed_world,
            generation,
            assessment_points,
            v4,
            population,
            brains,
            reset_brains,
        } = inputs;
        self.ready = None;
        if assessment_points.is_some_and(|points| {
            points.len() != population.len()
                || points
                    .iter()
                    .any(|point| !point.is_finite() || *point < 0.0)
        }) {
            return Err(NeuralControlError::InvalidAssessmentScores);
        }
        self.inference
            .prepare_weight_caches(key.population_epoch(), population, brains)?;
        self.last_cost = NeuralBatchCostMicros::default();
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

        let calculation_workers = self.workspace.worker_count();
        let CalculationExecutionBuffers {
            work, scratches, ..
        } = self.workspace.execution_buffers()?;
        sensor_scratch.work = Default::default();
        for scratch in &mut self.parallel_sensor_scratch {
            scratch.work = Default::default();
        }
        let sensing_started = Instant::now();
        if let Some(pool) = self
            .calculation_pool
            .as_ref()
            .filter(|_| active >= 2 * calculation_workers)
        {
            let chunk_work = active.div_ceil(calculation_workers);
            let chunk_observations = chunk_work * self.inference.input_size();
            let sensor = &self.sensor;
            self.parallel_sensor_errors.fill(None);
            pool.scope(|scope| {
                for (
                    partition,
                    (((((units, observations), deliveries), diagnostics), scratch), error),
                ) in work
                    .chunks(chunk_work)
                    .zip(self.observations[..observation_count].chunks_mut(chunk_observations))
                    .zip(self.deliveries[..active].chunks_mut(chunk_work))
                    .zip(self.diagnostics[..active].chunks_mut(chunk_work))
                    .zip(
                        std::iter::once(&mut *sensor_scratch)
                            .chain(self.parallel_sensor_scratch.iter_mut()),
                    )
                    .zip(self.parallel_sensor_errors.iter_mut())
                    .enumerate()
                {
                    let mut partition_trace = if trace
                        .as_ref()
                        .is_some_and(|(id, _)| units.iter().any(|unit| unit.snake_id() == *id))
                    {
                        trace.take()
                    } else {
                        None
                    };
                    scope.spawn(move |_| {
                        run_calculation_worker("sensing", partition, || {
                            for (ordinal, unit) in units.iter().enumerate() {
                                let offset = ordinal * sensor.layout().input_size;
                                match sensor.sample_prepared_with_trace(
                                    indexed_world,
                                    generation,
                                    unit.snake_index(),
                                    &mut observations[offset..offset + sensor.layout().input_size],
                                    scratch,
                                    unit.population_slot().and_then(|slot| {
                                        assessment_points
                                            .and_then(|points| points.get(slot as usize).copied())
                                    }),
                                    v4,
                                    partition_trace.as_mut().and_then(|(id, value)| {
                                        (*id == unit.snake_id()).then_some(&mut **value)
                                    }),
                                ) {
                                    Ok(sample) => {
                                        deliveries[ordinal] = Some(sample.delivery);
                                        diagnostics[ordinal] = sample.diagnostics;
                                    }
                                    Err(failure) => {
                                        *error = Some(failure);
                                        break;
                                    }
                                }
                            }
                        });
                    });
                }
            });
            for error in &mut self.parallel_sensor_errors {
                if let Some(failure) = error.take() {
                    return Err(failure.into());
                }
            }
        } else {
            for (ordinal, unit) in work.iter().enumerate() {
                let offset = ordinal * self.inference.input_size();
                let sample = self.sensor.sample_prepared_with_trace(
                    indexed_world,
                    generation,
                    unit.snake_index(),
                    &mut self.observations[offset..offset + self.inference.input_size()],
                    sensor_scratch,
                    unit.population_slot().and_then(|slot| {
                        assessment_points.and_then(|points| points.get(slot as usize).copied())
                    }),
                    v4,
                    trace
                        .as_mut()
                        .and_then(|(id, value)| (*id == unit.snake_id()).then_some(&mut **value)),
                )?;
                self.deliveries[ordinal] = Some(sample.delivery);
                self.diagnostics[ordinal] = sample.diagnostics;
            }
        }
        self.last_cost.work.timings.neural_sensing = super::generation_work::nanos(sensing_started);
        self.last_cost.work.sensors = sensor_scratch.work;
        for scratch in &self.parallel_sensor_scratch {
            self.last_cost.work.sensors.merge(scratch.work);
        }
        self.last_cost.work.neural_deliveries = active as u64;
        self.last_cost.sensing =
            u64::try_from(sensing_started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let inference_started = Instant::now();
        if let Some(pool) = self
            .calculation_pool
            .as_ref()
            .filter(|_| active >= 2 * calculation_workers)
        {
            let chunk_work = active.div_ceil(calculation_workers);
            let input_size = self.inference.input_size();
            let output_size = self.inference.output_size();
            let recurrent_size = self.inference.total_state_size();
            let mut outputs = &mut self.staged_outputs[..output_count];
            let mut recurrent = &mut self.staged_recurrent[..recurrent_count];
            self.parallel_inference_errors.fill(None);
            #[cfg(any(test, feature = "engine-test-hooks"))]
            let panic_partition = self.panic_inference_partition_once.take();
            pool.scope(|scope| {
                for (chunk_index, ((units, scratch), error)) in work
                    .chunks(chunk_work)
                    .zip(scratches.iter_mut())
                    .zip(self.parallel_inference_errors.iter_mut())
                    .enumerate()
                {
                    let count = units.len();
                    let (chunk_outputs, remaining_outputs) =
                        outputs.split_at_mut(count * output_size);
                    outputs = remaining_outputs;
                    let (chunk_recurrent, remaining_recurrent) =
                        recurrent.split_at_mut(count * recurrent_size);
                    recurrent = remaining_recurrent;
                    let start = chunk_index * chunk_work;
                    let chunk_observations =
                        &self.observations[start * input_size..(start + count) * input_size];
                    let chunk_resets = &self.recurrent_reset_mask[start..start + count];
                    let inference = &self.inference;
                    let zero_recurrent = &self.zero_recurrent;
                    scope.spawn(move |_| {
                        run_calculation_worker("inference", chunk_index, || {
                            #[cfg(any(test, feature = "engine-test-hooks"))]
                            if panic_partition == Some(chunk_index) {
                                panic!("test-only calculation worker panic");
                            }
                            *error = evaluate_heterogeneous_population_with_resets(
                                inference,
                                units,
                                population,
                                brains,
                                HeterogeneousRecurrentReset {
                                    mask: chunk_resets,
                                    zero_recurrent,
                                },
                                HeterogeneousInferenceBuffers {
                                    observations: chunk_observations,
                                    staged_outputs: chunk_outputs,
                                    staged_recurrent: chunk_recurrent,
                                },
                                &mut scratch.view(),
                            )
                            .err();
                        });
                    });
                }
            });
            for error in &mut self.parallel_inference_errors {
                if let Some(failure) = error.take() {
                    return Err(failure.into());
                }
            }
        } else {
            let graph_scratch = scratches
                .first_mut()
                .ok_or(NeuralControlError::MissingCalculationScratch)?;
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
        }

        self.last_cost.work.timings.neural_inference =
            super::generation_work::nanos(inference_started);
        self.last_cost.inference =
            u64::try_from(inference_started.elapsed().as_micros()).unwrap_or(u64::MAX);
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
            cost_micros: self.last_cost,
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

    /// Re-evaluate one explicitly focused due brain and capture all visible layers.
    pub fn capture_focused_visualization(
        &mut self,
        brain: BrainHandle,
        capture: &VisualizationCapturePlan,
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
                buffer: "focused visualization ordinal",
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
            .capture_visualization(capture, &scratch_view, destination)?;
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
        self.validate_state_commit(current_key, world, brains)?;
        self.commit_state_prevalidated(world, brains);
        self.ready = None;
        Ok(())
    }

    /// Revalidate one staged batch before a larger control transaction writes.
    pub(crate) fn validate_state_commit(
        &self,
        current_key: CalculationBatchKey,
        world: &WorldState,
        brains: &[BrainRuntimeState],
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

        Ok(())
    }

    /// Publish a neural batch already checked against the same exclusive targets.
    pub(crate) fn commit_state_prevalidated(
        &self,
        world: &mut WorldState,
        brains: &mut [BrainRuntimeState],
    ) {
        let ready = self
            .ready
            .expect("prevalidated neural-control batch must remain ready");
        let work = self
            .workspace
            .prepared_work()
            .expect("prevalidated neural-control work must remain available");
        debug_assert_eq!(work.len(), ready.active);
        for (ordinal, unit) in work.iter().enumerate() {
            let delivery = self.deliveries[ordinal]
                .expect("complete neural-control delivery preflight invariant");
            delivery.commit_prevalidated(&mut world.snakes[unit.snake_index()]);
        }
        let recurrent_count = ready
            .active
            .checked_mul(self.inference.total_state_size())
            .expect("prevalidated neural recurrent length must not overflow");
        let staged_recurrent = &self.staged_recurrent[..recurrent_count];
        publish_heterogeneous_recurrent(&self.inference, work, brains, staged_recurrent);
    }

    /// Discard one complete staged batch without changing authority.
    pub fn discard(&mut self) {
        self.ready = None;
    }
}

/// Sensing-to-inference staging, identity, or commit failure.
#[derive(Debug)]
pub enum NeuralControlError {
    /// Life-score context must be a finite, nonnegative dense population slab.
    InvalidAssessmentScores,
    /// Requested calculation-thread count is outside the supported envelope.
    InvalidCalculationWorkers(CalculationWorkerCountError),
    /// The bounded persistent calculation pool could not start.
    WorkerPool(String),
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
            Self::InvalidCalculationWorkers(error) => write!(formatter, "{error}"),
            Self::WorkerPool(detail) => write!(formatter, "calculation worker pool failed: {detail}"),
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
            Self::InvalidAssessmentScores => write!(formatter, "invalid population life-assessment sensor context"),
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
mod tests;
