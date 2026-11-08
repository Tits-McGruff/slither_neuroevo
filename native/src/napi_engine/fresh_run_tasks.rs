//! Fresh run tasks for the coarse N-API boundary.

use super::*;

/// Wrap the ordinary complete construction task without introducing a second authority.
#[cfg(feature = "engine-test-hooks")]
pub struct InitializeQualifiedFreshRunTask {
    pub(super) inner: InitializeExperimentalFreshRunTask,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for InitializeQualifiedFreshRunTask {
    type Output = FreshRunScalarSnapshot;
    type JsValue = QualifiedFreshRunInitialization;

    fn compute(&mut self) -> Result<Self::Output> {
        self.inner.compute()
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(QualifiedFreshRunInitialization {
            snapshot: fresh_run_snapshot_to_napi(output)?,
            trained_cohort_json: self.inner.trained_cohort_json.take(),
            capacity_audit_json: self.inner.capacity_audit_json.take(),
        })
    }

    fn finally(self, env: Env) -> Result<()> {
        self.inner.finally(env)
    }
}

/// Worker-owned exclusive transfer of the existing fresh-run authority.
pub struct CreateExperimentalBackgroundTask {
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) calculation_workers: usize,
    pub(super) memory_ceiling_bytes: usize,
    pub(super) active_operation: Arc<AtomicU8>,
    pub(super) init: EngineInit,
    pub(super) wake: Arc<NapiWakeSink>,
}

impl Task for CreateExperimentalBackgroundTask {
    type Output = Arc<EngineRuntime>;
    type JsValue = ExperimentalRunningAuthority;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            reject_faulted_fresh_inner(&inner)?;
            if inner.background_transferred {
                return Err(Error::new(
                    Status::GenericFailure,
                    "authority already transferred to the background runtime",
                ));
            }
            let running = if let Some(running) = inner.background_retry_loop.take() {
                running
            } else {
                let transition = inner.transition.take().ok_or_else(|| {
                    Error::new(
                        Status::GenericFailure,
                        "experimental fresh run has not been initialized",
                    )
                })?;
                match transition
                    .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
                {
                    Ok(running) => running,
                    Err(failure) => {
                        let (transition, error) = failure.into_parts();
                        inner.transition = Some(transition);
                        return Err(Error::new(Status::GenericFailure, error.to_string()));
                    }
                }
            };
            let runtime = match EngineRuntime::new_running_authority_with_display(
                self.init,
                running,
                Arc::clone(&self.wake) as Arc<dyn WakeSink>,
            ) {
                Ok(runtime) => Arc::new(runtime),
                Err(failure) => {
                    let error = failure.error().clone();
                    inner.background_retry_loop = Some(failure.into_running_loop());
                    return Err(engine_error_to_napi(error));
                }
            };
            self.wake.attach(&runtime);
            inner.background_transferred = true;
            Ok(runtime)
        })) {
            Ok(result) => result,
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental background transfer panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, runtime: Self::Output) -> Result<Self::JsValue> {
        Ok(ExperimentalRunningAuthority::from_runtime(
            runtime,
            self.calculation_workers,
            self.memory_ceiling_bytes,
        ))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async identity-only adoption of a validated, durably branched checkpoint.
pub struct AdoptRecoveryBranchTask {
    pub(super) run_id: String,
    pub(super) descriptor: CheckpointDescriptor,
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) active_operation: Arc<AtomicU8>,
}

impl Task for AdoptRecoveryBranchTask {
    type Output = FreshRunScalarSnapshot;
    type JsValue = ExperimentalFreshRunSnapshot;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = &inner.fault_detail {
                return Err(detail.clone());
            }
            let transition = inner
                .transition
                .as_ref()
                .ok_or_else(|| "recovery requires a retained candidate".to_owned())?;
            transition
                .validate_recovery_source(&self.descriptor, &self.run_id)
                .map_err(|error| error.to_string())?;
            let transition = inner
                .transition
                .take()
                .expect("validated transition remains present under lock");
            match transition.into_committed_recovery_branch(self.run_id.clone()) {
                Ok(branch) => inner.transition = Some(branch),
                Err(error) => {
                    return Err(retain_experimental_fresh_run_fault(
                        &mut inner,
                        "recovery branch admission failed",
                        &error.to_string(),
                    ));
                }
            }
            fresh_run_scalar_snapshot(&inner, FRESH_OPERATION_IDLE)
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(snapshot)) => Ok(snapshot),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "recovery branch adoption panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        fresh_run_snapshot_to_napi(output)
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Exact qualification-only environment accompanying a selected immutable checkpoint.
#[cfg(feature = "engine-test-hooks")]
pub(super) struct QualifiedCheckpointInitialization {
    pub(super) directory: PathBuf,
    pub(super) descriptor: CheckpointDescriptor,
    pub(super) profile: crate::engine::sensor_contract::SensorProfile,
    pub(super) bins: usize,
    pub(super) math: crate::engine::inference::InferenceMathBackend,
    pub(super) ray_work: crate::engine::ray_work::RayWorkLimits,
    pub(super) capacity_policy:
        Option<crate::engine::checkpoint_capacity::CheckpointCapacityPolicy>,
}

/// Async complete fixed-profile construction for one experimental lineage.
pub(super) enum FreshRunInitialization {
    Fresh,
    #[cfg(feature = "engine-test-hooks")]
    Qualification(
        crate::engine::sensor_contract::SensorProfile,
        usize,
        crate::engine::baseline_v2::BaselineV2Settings,
        crate::engine::ray_work::RayWorkLimits,
        Option<usize>,
        Option<PathBuf>,
        Option<crate::engine::sensor_qualification::QueryDetailLimits>,
    ),
    #[cfg(feature = "engine-test-hooks")]
    Capacity {
        population: usize,
        large_graph: bool,
        policy: crate::engine::checkpoint_capacity::CheckpointCapacityPolicy,
        ray_work: crate::engine::ray_work::RayWorkLimits,
    },
    #[cfg(feature = "engine-test-hooks")]
    QualifiedCheckpoint(Box<QualifiedCheckpointInitialization>),
    Checkpoint(Box<(PathBuf, CheckpointDescriptor, bool, bool, bool)>),
    LegacySqlite((PathBuf, i64)),
}

pub struct InitializeExperimentalFreshRunTask {
    #[cfg(feature = "engine-test-hooks")]
    pub(super) trained_cohort_json: Option<String>,
    #[cfg(feature = "engine-test-hooks")]
    pub(super) capacity_audit_json: Option<String>,
    pub(super) request: Stage6aP0FreshRunRequest,
    pub(super) calculation_workers: usize,
    pub(super) requested_math: Option<crate::engine::inference::InferenceMathBackend>,
    pub(super) source: FreshRunInitialization,
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) active_operation: Arc<AtomicU8>,
}

impl Task for InitializeExperimentalFreshRunTask {
    type Output = FreshRunScalarSnapshot;
    type JsValue = ExperimentalFreshRunSnapshot;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut transition = match &self.source {
                FreshRunInitialization::Checkpoint(restore) => {
                    let (directory, descriptor, recovery, compatible, population_fork) =
                        restore.as_ref();
                    let restored = if *compatible {
                        prepare_stage6a_p0_compatible_checkpoint_restore(
                            directory,
                            descriptor,
                            self.request.memory_ceiling_bytes,
                        )
                    } else {
                        prepare_stage6a_p0_checkpoint_restore(
                            directory,
                            descriptor,
                            self.request.memory_ceiling_bytes,
                        )
                    };
                    restored
                        .and_then(|transition| {
                            if *population_fork {
                                crate::engine::fresh_run::prepare_population_checkpoint_fork(
                                    self.request.clone(),
                                    transition,
                                    self.requested_math.ok_or_else(|| {
                                        crate::engine::fresh_run::FreshRunError::Settings(
                                            "population fork requires an explicit backend".into(),
                                        )
                                    })?,
                                )
                            } else if *recovery {
                                transition
                                    .into_committed_recovery_branch(self.request.run_id.clone())
                                    .map_err(crate::engine::fresh_run::FreshRunError::from)
                            } else {
                                Ok(transition)
                            }
                        })
                        .map_err(|error| error.to_string())?
                }
                #[cfg(feature = "engine-test-hooks")]
                FreshRunInitialization::Capacity {
                    population,
                    large_graph,
                    policy,
                    ray_work,
                } => {
                    let mut options = crate::engine::checkpoint_capacity::capacity_workload_options(
                        *population,
                        *large_graph,
                    );
                    options.seed = self.request.seed;
                    options.workers = self.calculation_workers;
                    options.ray_limits = Some(*ray_work);
                    let qualified = crate::engine::checkpoint_capacity::prepare_live_capacity_run(
                        self.request.clone(),
                        &options,
                        *policy,
                    )
                    .map_err(|error| error.to_string())?;
                    self.capacity_audit_json = Some(
                        serde_json::to_string(&qualified.audit)
                            .map_err(|error| error.to_string())?,
                    );
                    qualified
                        .pending
                        .ok_or_else(|| "capacity candidate has no admitted boundary".to_owned())?
                }
                #[cfg(feature = "engine-test-hooks")]
                FreshRunInitialization::QualifiedCheckpoint(inputs) => {
                    let work_limits = crate::engine::step_config::RunningStepWorkLimits {
                        calculation_workers: self.calculation_workers,
                        ray_work: Some(inputs.ray_work),
                        ..crate::engine::step_config::RunningStepWorkLimits::provisional_defaults()
                    };
                    let restored = if let Some(policy) = inputs.capacity_policy {
                        crate::engine::fresh_run::prepare_sensor_qualification_checkpoint_restore_with_limits(
                            &inputs.directory, &inputs.descriptor, self.request.memory_ceiling_bytes,
                            inputs.profile, inputs.bins, inputs.math, work_limits, policy.limits(),
                        )
                    } else {
                        crate::engine::fresh_run::prepare_sensor_qualification_checkpoint_restore(
                            &inputs.directory,
                            &inputs.descriptor,
                            self.request.memory_ceiling_bytes,
                            inputs.profile,
                            inputs.bins,
                            inputs.math,
                            work_limits,
                        )
                    };
                    restored.map_err(|error| error.to_string())?
                }
                FreshRunInitialization::LegacySqlite((database_path, snapshot_id)) => {
                    prepare_legacy_sqlite_population_import(
                        database_path,
                        *snapshot_id,
                        &self.request.run_id,
                        self.request.seed,
                        self.request.memory_ceiling_bytes,
                    )
                    .map_err(|error| error.to_string())?
                }
                #[cfg(feature = "engine-test-hooks")]
                FreshRunInitialization::Qualification(
                    profile,
                    bins,
                    baseline,
                    ray_work,
                    workload_population,
                    trained_cohort,
                    query_limits,
                ) => {
                    if let Some(population) = workload_population {
                        let options =
                            crate::engine::sensor_qualification::SensorQualificationOptions {
                                source_graph: None,
                                real_clock: false,
                                query_limits: *query_limits,
                                population: *population,
                                profile: *profile,
                                bins: *bins,
                                large_graph: false,
                                trained_artifact: None,
                                respawn: true,
                                seed: self.request.seed,
                                workers: self.calculation_workers,
                                math: crate::engine::inference::InferenceMathBackend::Sse2,
                                ray_limits: Some(*ray_work),
                                baseline: Some(*baseline),
                                teacher_respawn_delay_seconds: 20.0,
                                capture: None,
                                warmup_steps: 0,
                                measured_steps: 1,
                                environment: "development".into(),
                            };
                        if let Some(index) = trained_cohort {
                            let cohort = crate::engine::trained_cohort::load(index)?;
                            let transition = crate::engine::sensor_qualification::prepare_sensor_cohort_workload(
                                &options, self.request.clone(), &cohort,
                            ).map_err(|error| error.to_string())?;
                            self.trained_cohort_json = Some(
                                serde_json::to_string(&cohort.descriptor(*population))
                                    .map_err(|error| error.to_string())?,
                            );
                            transition
                        } else {
                            crate::engine::sensor_qualification::prepare_sensor_workload(
                                &options,
                                self.request.clone(),
                            )
                            .map_err(|error| error.to_string())?
                        }
                    } else {
                        use crate::engine::graph::{
                            GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
                        };
                        let width = crate::engine::sensor_layout::SensorLayout::for_profile(
                            *profile, *bins,
                        )
                        .map_err(|error| error.to_string())?
                        .input_size;
                        let graph = GraphSpec {
                            nodes: vec![
                                GraphNodeSpec {
                                    id: "input".into(),
                                    kind: GraphNodeKind::Input { output_size: width },
                                },
                                GraphNodeSpec {
                                    id: "motor".into(),
                                    kind: GraphNodeKind::Dense {
                                        input_size: width,
                                        output_size: 2,
                                    },
                                },
                            ],
                            edges: vec![GraphEdge {
                                from: "input".into(),
                                to: "motor".into(),
                                from_port: Some(0),
                                to_port: Some(0),
                            }],
                            outputs: vec![GraphOutputRef {
                                node_id: "motor".into(),
                                port: Some(0),
                            }],
                            output_size: 2,
                        };
                        let limits = crate::engine::step_config::RunningStepWorkLimits {
                        ray_work: Some(*ray_work),
                        calculation_workers: self.calculation_workers,
                        ..crate::engine::step_config::RunningStepWorkLimits::provisional_defaults()
                    };
                        crate::engine::fresh_run::prepare_baseline_v2_qualification_run(
                            self.request.clone(),
                            &[],
                            graph,
                            *profile,
                            *bins,
                            *baseline,
                            limits,
                        )
                        .map_err(|error| error.to_string())?
                    }
                }
                FreshRunInitialization::Fresh => prepare_default_fresh_run_with_math(
                    self.request.clone(),
                    self.requested_math
                        .unwrap_or(crate::engine::inference::InferenceMathBackend::Sse2),
                )
                .map_err(|error| error.to_string())?,
            };
            if let Some(requested) = self.requested_math {
                transition.require_math_backend(requested)?;
            }
            transition.configure_calculation_workers(self.calculation_workers)?;
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            if inner.transition.is_some() {
                return Err("experimental fresh run is already initialized".to_owned());
            }
            inner.transition = Some(transition);
            fresh_run_scalar_snapshot(&inner, FRESH_OPERATION_IDLE)
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(snapshot)) => Ok(snapshot),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run construction panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        fresh_run_snapshot_to_napi(output)
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async immutable managed-file publication for the retained fresh boundary.
pub struct PublishExperimentalFreshRunCheckpointTask {
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) active_operation: Arc<AtomicU8>,
    pub(super) managed_directory: PathBuf,
    pub(super) operation_id: CheckpointOperationId,
}

impl Task for PublishExperimentalFreshRunCheckpointTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            inner
                .transition
                .as_mut()
                .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?
                .publish_checkpoint(&self.managed_directory, self.operation_id.clone())
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run checkpoint publication panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async collision-safe running-authority activation for the durable boundary.
pub struct ActivateExperimentalFreshRunTask {
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) active_operation: Arc<AtomicU8>,
}

impl Task for ActivateExperimentalFreshRunTask {
    type Output = RunStartPublication;
    type JsValue = Stage6RunStartPublication;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            inner
                .transition
                .as_mut()
                .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?
                .publish_running_authority()
                .map_err(|error| error.to_string())
        })) {
            Ok(Ok(publication)) => Ok(publication),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run activation panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(run_start_publication_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async one-shot frame-v1 packing from the retained running authority.
pub struct PublishExperimentalFreshRunInitialFrameTask {
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) active_operation: Arc<AtomicU8>,
}

impl Task for PublishExperimentalFreshRunInitialFrameTask {
    type Output = FreshRunFrameV1Output;
    type JsValue = ExperimentalFreshRunFrameV1;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            if inner.initial_frame_published {
                return Err(
                    "experimental fresh-run initial frame-v1 is already published".to_owned(),
                );
            }
            let transition = inner
                .transition
                .as_ref()
                .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?;
            let mut bytes = Vec::new();
            let metadata = transition
                .pack_initial_frame_v1(&mut bytes)
                .map_err(|error| error.to_string())?;
            let output = fresh_run_frame_v1_output(bytes, metadata, transition.completed_step())
                .map_err(|error| error.to_string())?;
            inner.initial_frame_published = true;
            Ok(output)
        })) {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run initial frame-v1 publication panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(fresh_run_frame_v1_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}

/// Async one-shot complete scheduled step plus post-publication frame packing.
pub struct PublishExperimentalFreshRunFirstScheduledFrameTask {
    pub(super) inner: Arc<Mutex<ExperimentalFreshRunInner>>,
    pub(super) active_operation: Arc<AtomicU8>,
}

impl Task for PublishExperimentalFreshRunFirstScheduledFrameTask {
    type Output = FreshRunFrameV1Output;
    type JsValue = ExperimentalFreshRunFrameV1;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            let mut inner = lock_recover(&self.inner);
            if let Some(detail) = inner.fault_detail.as_deref() {
                return Err(format!(
                    "experimental fresh-run session is faulted: {detail}"
                ));
            }
            if !inner.initial_frame_published {
                return Err(
                    "experimental fresh-run first scheduled frame-v1 requires the initial frame"
                        .to_owned(),
                );
            }
            if inner.first_scheduled_frame_published {
                return Err(
                    "experimental fresh-run first scheduled frame-v1 is already published"
                        .to_owned(),
                );
            }

            let result = (|| {
                let transition = inner
                    .transition
                    .as_mut()
                    .ok_or_else(|| "experimental fresh run has not been initialized".to_owned())?;
                let mut bytes = Vec::new();
                let metadata = transition
                    .publish_first_scheduled_frame_v1(&mut bytes)
                    .map_err(|error| error.to_string())?;
                fresh_run_frame_v1_output(bytes, metadata, transition.completed_step())
                    .map_err(|error| error.to_string())
            })();

            match result {
                Ok(output) => {
                    inner.first_scheduled_frame_published = true;
                    Ok(output)
                }
                Err(detail) => {
                    let retained = retain_experimental_fresh_run_fault(
                        &mut inner,
                        "experimental fresh-run first scheduled frame-v1 failed",
                        &detail,
                    );
                    Err(retained)
                }
            }
        })) {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(fault_experimental_fresh_run(
                &self.inner,
                "experimental fresh-run first scheduled frame-v1 panicked",
                payload.as_ref(),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(fresh_run_frame_v1_to_napi(output))
    }

    fn finally(self, _env: Env) -> Result<()> {
        self.active_operation
            .store(FRESH_OPERATION_IDLE, Ordering::Release);
        Ok(())
    }
}
