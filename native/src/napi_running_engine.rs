//! Production-addon handle for one Rust-owned background authority.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use napi::bindgen_prelude::{AsyncTask, Object};
use napi::{Error, JsString, Result, Status};
use napi_derive::napi;

use crate::engine::contract::{
    CommandBatch, EngineCommand, ExternalDeliveryReceipt, RunningAuthorityCommand,
    SequencedCommand, ENGINE_CONTRACT_VERSION,
};
use crate::engine::error::{EngineError, EngineErrorCode};
use crate::engine::runtime::EngineRuntime;
use crate::napi_engine::{
    background_generation_event_to_napi, background_generation_health_to_napi,
    bounded_object_string, checkpoint_descriptor_from_napi_object, engine_error_to_napi,
    parse_background_sequence, parse_managed_checkpoint_publication_options, parse_u64_hex,
    positive_usize, u64_hex, JoinEngineTask, Stage6BackgroundGenerationDrain,
    Stage6BackgroundGenerationHealth,
};

/// The fresh-run session can create this handle only by transferring its sole
/// activated authority. JavaScript cannot construct it or supply a world.
#[napi]
pub struct ExperimentalRunningAuthority {
    runtime: Arc<EngineRuntime>,
    drain_active: AtomicBool,
    join_scheduled: Arc<AtomicBool>,
}

impl ExperimentalRunningAuthority {
    pub(crate) fn from_runtime(runtime: Arc<EngineRuntime>) -> Self {
        Self {
            runtime,
            drain_active: AtomicBool::new(false),
            join_scheduled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn submit(&self, sequence: u64, command: RunningAuthorityCommand) -> Result<()> {
        self.root(|| {
            self.runtime
                .try_submit(CommandBatch {
                    contract_version: ENGINE_CONTRACT_VERSION,
                    commands: vec![SequencedCommand {
                        sequence,
                        command: EngineCommand::RunningAuthority(command),
                    }]
                    .into_boxed_slice(),
                })
                .map_err(engine_error_to_napi)
        })
    }

    fn root<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(result) => result,
            Err(_) => {
                self.runtime.report_bridge_fault(EngineError::new(
                    EngineErrorCode::Faulted,
                    "panic at background authority N-API boundary",
                ));
                Err(Error::new(
                    Status::GenericFailure,
                    "background authority faulted",
                ))
            }
        }
    }
}

#[napi]
impl ExperimentalRunningAuthority {
    /// Start the retained coordinator after Node has attached its output router.
    #[napi(catch_unwind)]
    pub fn start(&self) -> Result<()> {
        self.root(|| self.runtime.start().map_err(engine_error_to_napi))
    }

    /// Queue immutable publication with bounded, server-controlled inputs.
    #[napi(catch_unwind)]
    pub fn submit_generation_checkpoint(
        &self,
        sequence: JsString<'_>,
        options: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let (directory, operation_id) = parse_managed_checkpoint_publication_options(&options)?;
        self.submit(
            sequence,
            RunningAuthorityCommand::PublishGenerationCheckpoint {
                managed_directory: directory
                    .to_str()
                    .ok_or_else(|| {
                        Error::new(Status::InvalidArg, "managed directory must be UTF-8")
                    })?
                    .to_owned(),
                operation_id,
            },
        )
    }

    /// Queue the exact descriptor acknowledged by the dedicated SQLite worker.
    #[napi(catch_unwind)]
    pub fn submit_generation_persistence_acknowledgement(
        &self,
        sequence: JsString<'_>,
        descriptor: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let descriptor = checkpoint_descriptor_from_napi_object(&descriptor)?;
        self.submit(
            sequence,
            RunningAuthorityCommand::AcknowledgeGenerationPersistence {
                descriptor: Box::new(descriptor),
            },
        )
    }

    /// Queue connected-controller reassignment at the durable generation barrier.
    #[napi(catch_unwind)]
    pub fn submit_prepare_generation_reassignments(&self, sequence: JsString<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::PrepareGenerationReassignments,
        )
    }

    /// Return one exact local-send result to the Rust-owned assignment barrier.
    #[napi(catch_unwind)]
    pub fn submit_generation_assignment_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        let sequence = parse_background_sequence(sequence)?;
        let receipt = ExternalDeliveryReceipt {
            operation_epoch: parse_u64_hex(
                &bounded_object_string(&receipt, "operationEpoch", 16)?,
                "operationEpoch",
                false,
            )?,
            event_sequence: parse_u64_hex(
                &bounded_object_string(&receipt, "eventSequence", 16)?,
                "eventSequence",
                false,
            )?,
            connection_id: parse_u64_hex(
                &bounded_object_string(&receipt, "connectionId", 16)?,
                "connectionId",
                false,
            )?,
            lease_id: parse_u64_hex(
                &bounded_object_string(&receipt, "leaseId", 16)?,
                "leaseId",
                false,
            )?,
            accepted: receipt
                .get::<bool>("accepted")?
                .ok_or_else(|| Error::new(Status::InvalidArg, "receipt omits accepted"))?,
        };
        self.submit(
            sequence,
            RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
                receipts: vec![receipt].into_boxed_slice(),
            },
        )
    }

    /// Queue the separately gated final generation swap.
    #[napi(catch_unwind)]
    pub fn submit_publish_generation_start(&self, sequence: JsString<'_>) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::PublishAcknowledgedGenerationStart,
        )
    }

    /// Drain prepared events without blocking on or inspecting the game world.
    #[napi(catch_unwind)]
    pub fn drain_outputs(
        &self,
        max_events: f64,
        max_owned_bytes: f64,
    ) -> Result<Stage6BackgroundGenerationDrain> {
        self.root(|| {
            let max_events =
                positive_usize(max_events, "maxEvents").map_err(engine_error_to_napi)?;
            let max_owned_bytes =
                positive_usize(max_owned_bytes, "maxOwnedBytes").map_err(engine_error_to_napi)?;
            if self.drain_active.swap(true, Ordering::AcqRel) {
                return Err(Error::new(
                    Status::GenericFailure,
                    "another background output drain is active",
                ));
            }
            let _guard = DrainGuard(&self.drain_active);
            let drained = self
                .runtime
                .drain_outputs(max_events, max_owned_bytes)
                .map_err(engine_error_to_napi)?;
            let events = drained
                .events
                .into_iter()
                .map(background_generation_event_to_napi)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| {
                    self.runtime.report_bridge_fault(error.clone());
                    engine_error_to_napi(error)
                })?;
            Ok(Stage6BackgroundGenerationDrain {
                events,
                more_work: drained.more_work,
                generation: u64_hex(drained.generation),
            })
        })
    }

    /// Read bounded health scalars from the background owner.
    #[napi(catch_unwind)]
    pub fn health(&self) -> Result<Stage6BackgroundGenerationHealth> {
        self.root(|| {
            background_generation_health_to_napi(self.runtime.health())
                .map_err(engine_error_to_napi)
        })
    }

    /// Signal shutdown without blocking Node on authority work.
    #[napi(catch_unwind)]
    pub fn request_stop(&self) {
        self.runtime.request_stop();
    }

    /// Join the background coordinator on a libuv worker.
    #[napi(catch_unwind)]
    pub fn join(&self) -> Result<AsyncTask<JoinEngineTask>> {
        if self.join_scheduled.swap(true, Ordering::AcqRel) {
            return Err(Error::new(
                Status::GenericFailure,
                "background authority join already scheduled",
            ));
        }
        // Signal before libuv schedules the join, including a queue-capacity wait.
        self.runtime.request_stop();
        Ok(AsyncTask::new(JoinEngineTask::new(
            Arc::clone(&self.runtime),
            Arc::clone(&self.join_scheduled),
        )))
    }
}

impl Drop for ExperimentalRunningAuthority {
    fn drop(&mut self) {
        self.runtime.request_stop();
    }
}

struct DrainGuard<'guard>(&'guard AtomicBool);
impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
