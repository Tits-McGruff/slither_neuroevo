//! Test hook sessions for the coarse N-API boundary.

use super::*;

/// Publish one real deterministic checkpoint on libuv's worker pool.
///
/// This export exists only in an explicitly feature-gated test-hooks addon.
/// Population, graph, recurrent-state, and archive bytes remain entirely in Rust;
/// JavaScript receives only the scalar descriptor required by the metadata worker.
#[cfg(feature = "engine-test-hooks")]
#[napi(js_name = "publishStage3CheckpointFixture", catch_unwind)]
pub fn publish_stage3_checkpoint_fixture(
    options: Stage3CheckpointFixtureOptions,
) -> Result<AsyncTask<PublishStage3CheckpointFixtureTask>> {
    if options.managed_directory.is_empty() || options.managed_directory.contains('\0') {
        return Err(Error::new(
            Status::InvalidArg,
            "managedDirectory must be a nonempty path without NUL",
        ));
    }
    let transition_epoch = parse_test_hook_epoch(&options.transition_epoch)?;
    Ok(AsyncTask::new(PublishStage3CheckpointFixtureTask {
        managed_directory: PathBuf::from(options.managed_directory),
        operation_id: options.operation_id,
        transition_epoch,
    }))
}

/// Async work item owning all filesystem and codec work for the test hook.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage3CheckpointFixtureTask {
    pub(super) managed_directory: PathBuf,
    pub(super) operation_id: String,
    pub(super) transition_epoch: u64,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage3CheckpointFixtureTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            publish_stage3_fixture(
                &self.managed_directory,
                &self.operation_id,
                self.transition_epoch,
            )
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 3 checkpoint fixture panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Feature-gated retained real fresh run-start used only by integration tests.
#[cfg(feature = "engine-test-hooks")]
#[napi]
pub struct Stage6RunStartHandoffFixtureSession {
    pub(super) session: Arc<Mutex<RunStartHandoffFixtureSession>>,
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl Stage6RunStartHandoffFixtureSession {
    /// Build and retain one real generation-one checkpoint boundary.
    #[napi(constructor, catch_unwind)]
    pub fn new() -> Result<Self> {
        let session = RunStartHandoffFixtureSession::new()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        Ok(Self {
            session: Arc::new(Mutex::new(session)),
        })
    }

    /// Publish or exactly retry the retained run-start checkpoint off the event loop.
    #[napi(catch_unwind)]
    pub fn publish_run_start_checkpoint(
        &self,
        options: ManagedCheckpointPublicationOptions,
    ) -> Result<AsyncTask<PublishStage6PendingRunStartTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        Ok(AsyncTask::new(PublishStage6PendingRunStartTask {
            session: Arc::clone(&self.session),
            managed_directory,
            operation_id,
        }))
    }

    /// Apply the persistence worker's complete committed descriptor to Rust.
    #[napi(catch_unwind)]
    pub fn acknowledge_run_start_persistence(&self, descriptor: Object<'_>) -> Result<()> {
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        lock_recover(&self.session)
            .acknowledge_persistence(&descriptor)
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Construct and publish the running authority off the Node event loop.
    #[napi(catch_unwind)]
    pub fn publish_running_authority(&self) -> Result<AsyncTask<PublishStage6RunningRunStartTask>> {
        Ok(AsyncTask::new(PublishStage6RunningRunStartTask {
            session: Arc::clone(&self.session),
        }))
    }

    /// Return bounded scalar proof without copying world or population state.
    #[napi(catch_unwind)]
    pub fn snapshot(&self) -> Result<Stage6RunStartHandoffSnapshot> {
        run_start_snapshot_to_napi(lock_recover(&self.session).snapshot())
    }
}

/// Async immutable publication for the retained pending run start.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6PendingRunStartTask {
    pub(super) session: Arc<Mutex<RunStartHandoffFixtureSession>>,
    pub(super) managed_directory: PathBuf,
    pub(super) operation_id: CheckpointOperationId,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6PendingRunStartTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session)
                .publish_checkpoint(&self.managed_directory, self.operation_id.clone())
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 pending run-start publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Async collision-safe activation for the retained durable run start.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6RunningRunStartTask {
    pub(super) session: Arc<Mutex<RunStartHandoffFixtureSession>>,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6RunningRunStartTask {
    type Output = RunStartPublication;
    type JsValue = Stage6RunStartPublication;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session).publish_running_authority()
        })) {
            Ok(Ok(publication)) => Ok(publication),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 run-start activation panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(Stage6RunStartPublication {
            world_epoch: u64_hex(output.world_epoch),
            generation: u64_hex(output.generation),
            completed_step: u64_hex(output.completed_step),
            population_epoch: u64_hex(output.population_epoch),
        })
    }
}

/// Feature-gated retained real generation handoff used only by integration tests.
#[cfg(feature = "engine-test-hooks")]
#[napi]
pub struct Stage6GenerationHandoffFixtureSession {
    pub(super) session: Arc<Mutex<GenerationHandoffFixtureSession>>,
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl Stage6GenerationHandoffFixtureSession {
    /// Build and retain one real terminal coordinator transition.
    #[napi(constructor, catch_unwind)]
    pub fn new() -> Result<Self> {
        let session = GenerationHandoffFixtureSession::new()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        Ok(Self {
            session: Arc::new(Mutex::new(session)),
        })
    }

    /// Publish the same-run run-start file on libuv's worker pool.
    #[napi(catch_unwind)]
    pub fn publish_run_start_checkpoint(
        &self,
        options: Stage3CheckpointFixtureOptions,
    ) -> Result<AsyncTask<PublishStage6RunStartTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        let transition_epoch = parse_test_hook_epoch(&options.transition_epoch)?;
        Ok(AsyncTask::new(PublishStage6RunStartTask {
            session: Arc::clone(&self.session),
            managed_directory,
            operation_id,
            transition_epoch,
        }))
    }

    /// Publish or exactly retry the retained generation checkpoint off the event loop.
    #[napi(catch_unwind)]
    pub fn publish_generation_checkpoint(
        &self,
        options: ManagedCheckpointPublicationOptions,
    ) -> Result<AsyncTask<PublishStage6GenerationTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        Ok(AsyncTask::new(PublishStage6GenerationTask {
            session: Arc::clone(&self.session),
            managed_directory,
            operation_id,
        }))
    }

    /// Apply the persistence worker's complete committed descriptor to Rust.
    #[napi(catch_unwind)]
    pub fn acknowledge_generation_persistence(&self, descriptor: Object<'_>) -> Result<()> {
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        lock_recover(&self.session)
            .acknowledge_generation_persistence(&descriptor)
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Stage or reborrow the one required reliable controller assignment.
    #[napi(catch_unwind)]
    pub fn prepare_generation_assignment(&self) -> Result<Stage6GenerationAssignment> {
        let assignment = lock_recover(&self.session)
            .prepare_generation_assignment()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        Ok(generation_assignment_to_napi(assignment))
    }

    /// Apply one exact local-send result to the retained assignment.
    #[napi(catch_unwind)]
    pub fn submit_generation_assignment(
        &self,
        result: Stage6GenerationAssignmentResult,
    ) -> Result<()> {
        let operation_epoch = parse_u64_hex(&result.operation_epoch, "operationEpoch", false)?;
        let event_sequence = parse_u64_hex(&result.event_sequence, "eventSequence", false)?;
        let connection_id = parse_u64_hex(&result.connection_id, "connectionId", false)?;
        let lease_id = parse_u64_hex(&result.lease_id, "leaseId", false)?;
        lock_recover(&self.session)
            .submit_generation_assignment(
                operation_epoch,
                event_sequence,
                connection_id,
                lease_id,
                result.accepted,
            )
            .map_err(|detail| Error::new(Status::GenericFailure, detail))
    }

    /// Perform the one final authority swap after both required barriers.
    #[napi(catch_unwind)]
    pub fn publish_generation_start(&self) -> Result<Stage6GenerationStartPublication> {
        let publication = lock_recover(&self.session)
            .publish_generation_start()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        let external_assignments =
            u32::try_from(publication.external_assignments).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "generation assignment count exceeds the bounded N-API fixture field",
                )
            })?;
        Ok(Stage6GenerationStartPublication {
            world_epoch: u64_hex(publication.world_epoch),
            generation: u64_hex(publication.generation),
            completed_step: u64_hex(publication.completed_step),
            population_epoch: u64_hex(publication.population_epoch),
            external_assignments,
        })
    }

    /// Return bounded scalar proof without copying world or population state.
    #[napi(catch_unwind)]
    pub fn snapshot(&self) -> Stage6GenerationHandoffSnapshot {
        generation_snapshot_to_napi(lock_recover(&self.session).snapshot())
    }
}

/// Feature-gated adapter that drives the retained generation barrier through
/// the real background command/output queues and Rust-owned monotonic clock.
#[cfg(feature = "engine-test-hooks")]
#[napi(js_name = "Stage6BackgroundGenerationHandoffFixtureSession")]
pub struct Stage6BackgroundGenerationHandoffFixtureSession {
    pub(super) runtime: Arc<EngineRuntime>,
    pub(super) run_start: Arc<Mutex<GenerationHandoffRunStartFixture>>,
    pub(super) drain_active: AtomicBool,
    pub(super) join_scheduled: Arc<AtomicBool>,
}

#[cfg(feature = "engine-test-hooks")]
#[napi]
impl Stage6BackgroundGenerationHandoffFixtureSession {
    /// Construct one unstarted real authority thread and its same-run step-zero publisher.
    #[napi(constructor, catch_unwind)]
    pub fn new(wake_callback: Function<'_, (), ()>) -> Result<Self> {
        let fixture = background_generation_handoff_fixture()
            .map_err(|detail| Error::new(Status::GenericFailure, detail))?;
        let wake_tsfn = wake_callback
            .build_threadsafe_function::<()>()
            .callee_handled::<false>()
            .weak::<true>()
            .max_queue_size::<1>()
            .build_callback(|_context| Ok(()))?;
        let wake_sink = Arc::new(NapiWakeSink::new(wake_tsfn));
        let runtime = Arc::new(
            EngineRuntime::new_running_authority(
                background_generation_handoff_runtime_init(),
                fixture.running,
                Arc::clone(&wake_sink) as Arc<dyn WakeSink>,
            )
            .map_err(|failure| engine_error_to_napi(failure.error().clone()))?,
        );
        wake_sink.attach(&runtime);
        Ok(Self {
            runtime,
            run_start: Arc::new(Mutex::new(fixture.run_start)),
            drain_active: AtomicBool::new(false),
            join_scheduled: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Publish the same-run run-start checkpoint on libuv's worker pool.
    #[napi(catch_unwind)]
    pub fn publish_run_start_checkpoint(
        &self,
        options: Stage3CheckpointFixtureOptions,
    ) -> Result<AsyncTask<PublishBackgroundGenerationRunStartTask>> {
        let managed_directory = parse_managed_path(options.managed_directory)?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        let transition_epoch = parse_test_hook_epoch(&options.transition_epoch)?;
        Ok(AsyncTask::new(PublishBackgroundGenerationRunStartTask {
            run_start: Arc::clone(&self.run_start),
            managed_directory,
            operation_id,
            transition_epoch,
        }))
    }

    /// Start the real Rust-owned background authority exactly once.
    #[napi(catch_unwind)]
    pub fn start(&self) -> Result<()> {
        self.run_faulting_root(|| self.runtime.start())
    }

    /// Queue one immutable generation checkpoint publication request.
    #[napi(catch_unwind)]
    pub fn submit_generation_checkpoint(
        &self,
        sequence_hex: JsString<'_>,
        options: ManagedCheckpointPublicationOptions,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence_hex)?;
        let managed_directory = options.managed_directory;
        let _validated_path = parse_managed_path(managed_directory.clone())?;
        let operation_id = parse_checkpoint_operation_id(options.operation_id)?;
        self.submit_control(
            sequence,
            RunningAuthorityCommand::PublishGenerationCheckpoint {
                managed_directory,
                operation_id,
            },
        )
    }

    /// Queue the persistence worker's complete committed descriptor.
    #[napi(catch_unwind)]
    pub fn submit_generation_persistence_acknowledgement(
        &self,
        sequence_hex: JsString<'_>,
        descriptor: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence_hex)?;
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        self.submit_control(
            sequence,
            RunningAuthorityCommand::AcknowledgeGenerationPersistence {
                descriptor: Box::new(descriptor),
            },
        )
    }

    /// Queue deterministic successor reassignment preparation.
    #[napi(catch_unwind)]
    pub fn submit_prepare_generation_reassignments(
        &self,
        sequence_hex: JsString<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence_hex)?;
        self.submit_control(
            sequence,
            RunningAuthorityCommand::PrepareGenerationReassignments,
        )
    }

    /// Queue one exact local-send receipt for the retained fixture assignment.
    #[napi(catch_unwind)]
    pub fn submit_generation_assignment_receipt(
        &self,
        sequence_hex: JsString<'_>,
        result: Stage6GenerationAssignmentResult,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence_hex)?;
        let receipt = ExternalDeliveryReceipt {
            operation_epoch: parse_u64_hex(&result.operation_epoch, "operationEpoch", false)?,
            event_sequence: parse_u64_hex(&result.event_sequence, "eventSequence", false)?,
            connection_id: parse_u64_hex(&result.connection_id, "connectionId", false)?,
            lease_id: parse_u64_hex(&result.lease_id, "leaseId", false)?,
            accepted: result.accepted,
        };
        self.submit_control(
            sequence,
            RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
                receipts: vec![receipt].into_boxed_slice(),
            },
        )
    }

    /// Stage an fresh controller join at an eligible source boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_join(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::JoinController(Box::new(
                crate::napi_running_engine::parse_controller_join(&request)?,
            )),
        )
    }

    /// Resolve only the retained fresh join assignment named by this receipt.
    #[napi(catch_unwind)]
    pub fn submit_controller_join_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerJoinReceipt(
                crate::napi_running_engine::parse_reclaim_receipt(&receipt)?,
            ),
        )
    }

    /// Stage an explicit token reclaim at an eligible source boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_reclaim(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ReclaimController(Box::new(
                crate::napi_running_engine::parse_controller_reclaim(&request)?,
            )),
        )
    }

    /// Resolve only the retained reclaim assignment named by this receipt.
    #[napi(catch_unwind)]
    pub fn submit_controller_reclaim_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerReclaimReceipt(
                crate::napi_running_engine::parse_reclaim_receipt(&receipt)?,
            ),
        )
    }

    /// Queue a socket close without invalidating a retained ordinary step.
    #[napi(catch_unwind)]
    pub fn submit_controller_disconnect(
        &self,
        sequence_hex: JsString<'_>,
        close: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence_hex)?,
            RunningAuthorityCommand::DisconnectController(
                crate::napi_running_engine::parse_controller_disconnect(&close)?,
            ),
        )
    }

    /// Queue steering for the next eligible pre-step boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_action(
        &self,
        sequence_hex: JsString<'_>,
        action: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence_hex)?,
            RunningAuthorityCommand::SubmitControllerAction(Box::new(
                crate::napi_running_engine::parse_controller_action(&action)?,
            )),
        )
    }

    /// Queue an ordinary controller send result against its retained step.
    #[napi(catch_unwind)]
    pub fn submit_controller_delivery_receipt(
        &self,
        sequence_hex: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit_control(
            parse_background_sequence(sequence_hex)?,
            RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                receipts: vec![crate::napi_running_engine::parse_controller_receipt(
                    &receipt,
                )?]
                .into_boxed_slice(),
            },
        )
    }

    /// Queue the separately gated final authority swap.
    #[napi(catch_unwind)]
    pub fn submit_publish_generation_start(&self, sequence_hex: JsString<'_>) -> Result<()> {
        let sequence = parse_background_sequence(sequence_hex)?;
        self.submit_control(
            sequence,
            RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
        )
    }

    /// Drain only typed bounded events from the background authority queue.
    #[napi(catch_unwind)]
    pub fn drain_outputs(
        &self,
        max_events: f64,
        max_owned_bytes: f64,
    ) -> Result<Stage6BackgroundGenerationDrain> {
        self.run_faulting_root(|| {
            let max_events = positive_usize(max_events, "maxEvents")?;
            let max_owned_bytes = positive_usize(max_owned_bytes, "maxOwnedBytes")?;
            let _guard = DrainConsumerGuard::acquire(&self.drain_active)?;
            let drained = self.runtime.drain_outputs(max_events, max_owned_bytes)?;
            let events = drained
                .events
                .into_iter()
                .map(background_generation_event_to_napi)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Stage6BackgroundGenerationDrain {
                events,
                more_work: drained.more_work,
                generation: u64_hex(drained.generation),
            })
        })
    }

    /// Return only bounded authority/lifecycle scalars.
    #[napi(catch_unwind)]
    pub fn health(&self) -> Result<Stage6BackgroundGenerationHealth> {
        self.run_faulting_root(|| {
            background_generation_health_to_napi(self.runtime.health(), 1, true)
        })
    }

    /// Request an orderly stop without waiting on Node's event loop.
    #[napi(catch_unwind)]
    pub fn request_stop(&self) -> Result<()> {
        self.run_faulting_root(|| {
            self.runtime.request_stop();
            Ok(())
        })
    }

    /// Join the real coordinator only on libuv's worker pool.
    #[napi(catch_unwind)]
    pub fn join(&self) -> Result<AsyncTask<JoinEngineTask>> {
        self.run_faulting_root(|| {
            if self.join_scheduled.swap(true, Ordering::AcqRel) {
                return Err(EngineError::new(
                    EngineErrorCode::InvalidLifecycle,
                    "a background generation fixture join is already scheduled",
                ));
            }
            Ok(AsyncTask::new(JoinEngineTask {
                runtime: Arc::clone(&self.runtime),
                join_scheduled: Arc::clone(&self.join_scheduled),
            }))
        })
    }
}

#[cfg(feature = "engine-test-hooks")]
impl Stage6BackgroundGenerationHandoffFixtureSession {
    pub(super) fn submit_control(
        &self,
        sequence: u64,
        command: RunningAuthorityCommand,
    ) -> Result<()> {
        self.run_faulting_root(|| {
            self.runtime.try_submit(CommandBatch {
                contract_version: ENGINE_CONTRACT_VERSION,
                commands: vec![SequencedCommand {
                    sequence,
                    command: EngineCommand::RunningAuthority(command),
                }]
                .into_boxed_slice(),
            })
        })
    }

    pub(super) fn run_faulting_root<T>(
        &self,
        operation: impl FnOnce() -> std::result::Result<T, EngineError>,
    ) -> Result<T> {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(result) => result.map_err(engine_error_to_napi),
            Err(payload) => {
                let detail = panic_detail(payload.as_ref());
                self.runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::Faulted,
                    format!("panic at background generation N-API root: {detail}"),
                ));
                Err(Error::new(
                    Status::GenericFailure,
                    format!("Faulted: panic at background generation N-API root: {detail}"),
                ))
            }
        }
    }
}

#[cfg(feature = "engine-test-hooks")]
impl Drop for Stage6BackgroundGenerationHandoffFixtureSession {
    fn drop(&mut self) {
        self.runtime.request_stop();
    }
}

/// Worker task for the independent same-run run-start managed file.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishBackgroundGenerationRunStartTask {
    pub(super) run_start: Arc<Mutex<GenerationHandoffRunStartFixture>>,
    pub(super) managed_directory: PathBuf,
    pub(super) operation_id: CheckpointOperationId,
    pub(super) transition_epoch: u64,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishBackgroundGenerationRunStartTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.run_start).publish_checkpoint(
                &self.managed_directory,
                self.operation_id.clone(),
                self.transition_epoch,
            )
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "background generation run-start publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Async same-run run-start publication task for the retained session.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6RunStartTask {
    pub(super) session: Arc<Mutex<GenerationHandoffFixtureSession>>,
    pub(super) managed_directory: PathBuf,
    pub(super) operation_id: CheckpointOperationId,
    pub(super) transition_epoch: u64,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6RunStartTask {
    type Output = CheckpointDescriptor;
    type JsValue = ManagedCheckpointDescriptor;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session).publish_run_start_checkpoint(
                &self.managed_directory,
                self.operation_id.clone(),
                self.transition_epoch,
            )
        })) {
            Ok(Ok(descriptor)) => Ok(descriptor),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 run-start publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(checkpoint_descriptor_to_napi(output))
    }
}

/// Async generation publication task retaining the same real coordinator.
#[cfg(feature = "engine-test-hooks")]
pub struct PublishStage6GenerationTask {
    pub(super) session: Arc<Mutex<GenerationHandoffFixtureSession>>,
    pub(super) managed_directory: PathBuf,
    pub(super) operation_id: CheckpointOperationId,
}

#[cfg(feature = "engine-test-hooks")]
impl Task for PublishStage6GenerationTask {
    type Output = PublishedGenerationHandoff;
    type JsValue = Stage6GenerationCheckpointPublication;

    fn compute(&mut self) -> Result<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| {
            lock_recover(&self.session)
                .publish_generation_checkpoint(&self.managed_directory, self.operation_id.clone())
        })) {
            Ok(Ok(publication)) => Ok(publication),
            Ok(Err(detail)) => Err(Error::new(Status::GenericFailure, detail)),
            Err(payload) => Err(Error::new(
                Status::GenericFailure,
                format!(
                    "Stage 6 generation publication panicked: {}",
                    panic_detail(payload.as_ref())
                ),
            )),
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(Stage6GenerationCheckpointPublication {
            workload_json: output.workload.map(|work| work.to_json()),
            descriptor: checkpoint_descriptor_to_napi(output.descriptor),
            hall_of_fame_weights: hall_of_fame_weights_descriptor_to_napi(
                output.hall_of_fame_weights,
            ),
            generation_commit: generation_commit_to_napi(output.commit_record),
        })
    }
}
