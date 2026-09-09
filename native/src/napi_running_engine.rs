//! Production-addon handle for one Rust-owned background authority.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use napi::bindgen_prelude::{AsyncTask, Object};
use napi::{Env, Error, JsString, JsValue, Result, Status};
use napi_derive::napi;

use crate::engine::contract::{
    CommandBatch, EngineCommand, ExternalDeliveryReceipt, RunningAuthorityCommand,
    SequencedCommand, ENGINE_CONTRACT_VERSION,
};
use crate::engine::display::{FrameCopyResult, RunningDisplayStatus};
use crate::engine::error::{EngineError, EngineErrorCode};
use crate::engine::runtime::EngineRuntime;
use crate::napi_engine::{
    background_generation_event_to_napi, background_generation_health_to_napi, bounded_js_string,
    bounded_object_string, checkpoint_descriptor_from_napi_object, engine_error_to_napi,
    parse_background_sequence, parse_managed_checkpoint_publication_options, parse_u64_hex,
    positive_usize, u64_hex, JoinEngineTask, Stage6BackgroundGenerationDrain,
    Stage6BackgroundGenerationHealth,
};

/// Cached frame chronology and basic stats; no population or world objects.
#[napi(object)]
pub struct BackgroundDisplayStatus {
    pub sequence: String,
    pub world_epoch: String,
    pub completed_step: String,
    pub generation: String,
    pub generation_time: f64,
    pub alive_population: u32,
    pub baseline_bots_alive: u32,
    pub baseline_bots_total: u32,
    pub total_snakes: u32,
    pub alive_snakes: u32,
    pub pellets: u32,
    pub frame_byte_length: f64,
}

pub(crate) fn display_status_to_napi(status: RunningDisplayStatus) -> BackgroundDisplayStatus {
    BackgroundDisplayStatus {
        sequence: u64_hex(status.sequence),
        world_epoch: u64_hex(status.world_epoch),
        completed_step: u64_hex(status.completed_step),
        generation: u64_hex(status.frame.generation),
        generation_time: status.generation_time,
        alive_population: status.alive_population as u32,
        baseline_bots_alive: status.baseline_bots_alive as u32,
        baseline_bots_total: status.baseline_bots_total as u32,
        total_snakes: status.frame.total_snakes as u32,
        alive_snakes: status.frame.alive_snakes as u32,
        pellets: status.frame.pellets as u32,
        frame_byte_length: status.frame.byte_length as f64,
    }
}

/// A caller can retry busy/too-small copies without consuming the retained frame.
#[napi(object)]
pub struct BackgroundFrameCopy {
    pub status: String,
    pub display: Option<BackgroundDisplayStatus>,
}

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
        let receipt = parse_controller_receipt(&receipt)?;
        self.submit(
            sequence,
            RunningAuthorityCommand::SubmitGenerationAssignmentReceipts {
                receipts: vec![receipt].into_boxed_slice(),
            },
        )
    }

    /// Stage an explicit token reclaim at an eligible source boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_reclaim(
        &self,
        sequence: JsString<'_>,
        request: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::ReclaimController(Box::new(parse_controller_reclaim(
                &request,
            )?)),
        )
    }

    /// Resolve the retained reclaim assignment without touching ordinary receipts.
    #[napi(catch_unwind)]
    pub fn submit_controller_reclaim_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerReclaimReceipt(parse_reclaim_receipt(
                &receipt,
            )?),
        )
    }

    /// Queue a socket close for the next eligible pre-step boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_disconnect(
        &self,
        sequence: JsString<'_>,
        close: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::DisconnectController(parse_controller_disconnect(&close)?),
        )
    }

    /// Queue steering for the next eligible pre-step boundary.
    #[napi(catch_unwind)]
    pub fn submit_controller_action(
        &self,
        sequence: JsString<'_>,
        action: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerAction(parse_controller_action(&action)?),
        )
    }

    /// Resolve an ordinary observation/death-assignment send without touching a generation barrier.
    #[napi(catch_unwind)]
    pub fn submit_controller_delivery_receipt(
        &self,
        sequence: JsString<'_>,
        receipt: Object<'_>,
    ) -> Result<()> {
        self.submit(
            parse_background_sequence(sequence)?,
            RunningAuthorityCommand::SubmitControllerDeliveryReceipts {
                receipts: vec![parse_controller_receipt(&receipt)?].into_boxed_slice(),
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

    /// Read cached welcome metadata without repacking or waiting on the authority.
    #[napi(catch_unwind)]
    pub fn latest_display(&self) -> Result<Option<BackgroundDisplayStatus>> {
        self.root(|| {
            self.runtime
                .latest_display()
                .map(|status| status.map(display_status_to_napi))
                .map_err(engine_error_to_napi)
        })
    }

    /// Copy into a non-shared Uint8Array owned by Node until socket sends complete.
    /// No JS callback runs while its mutable byte slice exists, and Rust retains none.
    #[napi(catch_unwind)]
    pub fn copy_latest_frame(
        &self,
        env: Env,
        destination: Object<'_>,
        after_sequence: JsString<'_>,
    ) -> Result<BackgroundFrameCopy> {
        let after_sequence = parse_u64_hex(
            &bounded_js_string(after_sequence, "afterSequence", 16, false)?,
            "afterSequence",
            true,
        )?;
        self.root(|| {
            let mut typed = false;
            // SAFETY: env and destination are live values on this N-API call's JS thread.
            napi::check_status!(unsafe {
                napi::sys::napi_is_typedarray(env.raw(), destination.raw(), &mut typed)
            })?;
            if !typed {
                return Err(Error::new(
                    Status::InvalidArg,
                    "destination must be a non-shared Uint8Array",
                ));
            }
            let mut kind = 0;
            let mut length = 0;
            let mut data = std::ptr::null_mut();
            let mut backing = std::ptr::null_mut();
            // SAFETY: the intrinsic typed-array query bypasses user properties. Returned
            // backing and data remain rooted by destination throughout this synchronous call.
            napi::check_status!(unsafe {
                napi::sys::napi_get_typedarray_info(
                    env.raw(),
                    destination.raw(),
                    &mut kind,
                    &mut length,
                    &mut data,
                    &mut backing,
                    std::ptr::null_mut(),
                )
            })?;
            let mut ordinary_buffer = false;
            // SAFETY: backing is the live intrinsic buffer returned above. SharedArrayBuffer
            // is not an ArrayBuffer, and must be rejected before constructing a Rust slice.
            napi::check_status!(unsafe {
                napi::sys::napi_is_arraybuffer(env.raw(), backing, &mut ordinary_buffer)
            })?;
            if kind != napi::sys::TypedarrayType::uint8_array
                || !ordinary_buffer
                || (length > 0 && data.is_null())
                || length > isize::MAX as usize
            {
                return Err(Error::new(
                    Status::InvalidArg,
                    "destination must be a non-shared Uint8Array",
                ));
            }
            let destination = if length == 0 {
                &mut []
            } else {
                // SAFETY: the Uint8Array owns length initialized bytes starting at data,
                // including its byte offset. Its non-shared backing cannot be concurrently
                // resized/written; no JS callbacks occur until this borrow ends. The source
                // is private Rust cache storage and cannot alias this destination.
                unsafe { std::slice::from_raw_parts_mut(data.cast::<u8>(), length) }
            };
            let result = self
                .runtime
                .copy_latest_frame(destination, after_sequence)
                .map_err(engine_error_to_napi)?;
            let (status, display) = match result {
                FrameCopyResult::Busy => ("busy", None),
                FrameCopyResult::Unchanged => ("unchanged", None),
                FrameCopyResult::TooSmall(display) => {
                    ("tooSmall", Some(display_status_to_napi(display)))
                }
                FrameCopyResult::Copied(display) => {
                    ("copied", Some(display_status_to_napi(display)))
                }
            };
            Ok(BackgroundFrameCopy {
                status: status.to_owned(),
                display,
            })
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

/// Parse one optional bounded identity without materializing an unbounded string.
fn optional_reclaim_identity(request: &Object<'_>, field: &str, maximum: usize) -> Result<String> {
    match request.get::<JsString<'_>>(field)? {
        Some(value) => bounded_js_string(value, field, maximum, false),
        None => Ok(String::new()),
    }
}

/// Correlate a token or legacy identity and stamp its native receipt time.
pub(crate) fn parse_controller_reclaim(
    request: &Object<'_>,
) -> Result<crate::engine::contract::ControllerReclaimRequest> {
    let kind = match bounded_object_string(request, "controllerKind", 32)?.as_str() {
        "player" => crate::engine::state::ControllerKind::Player,
        "reinforcementLearning" => crate::engine::state::ControllerKind::ReinforcementLearning,
        _ => return Err(Error::new(Status::InvalidArg, "invalid controllerKind")),
    };
    Ok(crate::engine::contract::ControllerReclaimRequest {
        connection_id: parse_u64_hex(
            &bounded_object_string(request, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        kind,
        resume_token: optional_reclaim_identity(request, "resumeToken", 256)?,
        identity_key: optional_reclaim_identity(request, "identityKey", 128)?,
        received_at: std::time::Instant::now(),
    })
}

/// Validate exact reclaim receipt correlation before native queue admission.
pub(crate) fn parse_reclaim_receipt(
    receipt: &Object<'_>,
) -> Result<crate::engine::contract::ControllerReclaimReceipt> {
    Ok(crate::engine::contract::ControllerReclaimReceipt {
        request_sequence: parse_u64_hex(
            &bounded_object_string(receipt, "requestSequence", 16)?,
            "requestSequence",
            false,
        )?,
        connection_id: parse_u64_hex(
            &bounded_object_string(receipt, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        lease_id: parse_u64_hex(
            &bounded_object_string(receipt, "leaseId", 16)?,
            "leaseId",
            false,
        )?,
        accepted: receipt
            .get::<bool>("accepted")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "receipt omits accepted"))?,
    })
}

/// Validate a bounded exact close before stamping its native receipt time.
pub(crate) fn parse_controller_disconnect(
    close: &Object<'_>,
) -> Result<crate::engine::contract::ControllerDisconnectRequest> {
    let lease_id = parse_u64_hex(
        &bounded_object_string(close, "leaseId", 16)?,
        "leaseId",
        false,
    )?;
    let connection_id = parse_u64_hex(
        &bounded_object_string(close, "connectionId", 16)?,
        "connectionId",
        false,
    )?;
    Ok(crate::engine::contract::ControllerDisconnectRequest {
        lease_id,
        connection_id,
        received_at: std::time::Instant::now(),
    })
}

/// Validate wire values before recording the action's Rust-owned receipt time.
pub(crate) fn parse_controller_action(
    action: &Object<'_>,
) -> Result<crate::engine::contract::ControllerActionRequest> {
    let lease_id = parse_u64_hex(
        &bounded_object_string(action, "leaseId", 16)?,
        "leaseId",
        false,
    )?;
    let connection_id = parse_u64_hex(
        &bounded_object_string(action, "connectionId", 16)?,
        "connectionId",
        false,
    )?;
    let client_tick = parse_u64_hex(
        &bounded_object_string(action, "clientTick", 16)?,
        "clientTick",
        true,
    )?;
    let turn = action
        .get::<f64>("turn")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "action omits turn"))?;
    if !turn.is_finite() || !(-1.0..=1.0).contains(&turn) {
        return Err(Error::new(
            Status::InvalidArg,
            "action turn must be finite in [-1, 1]",
        ));
    }
    let boost = action
        .get::<bool>("boost")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "action omits boost"))?;
    Ok(crate::engine::contract::ControllerActionRequest {
        lease_id,
        connection_id,
        turn: turn as f32,
        boost,
        client_tick,
        received_at: std::time::Instant::now(),
    })
}

/// Parse bounded correlation fields before admitting one ordinary send result.
pub(crate) fn parse_controller_receipt(receipt: &Object<'_>) -> Result<ExternalDeliveryReceipt> {
    Ok(ExternalDeliveryReceipt {
        operation_epoch: parse_u64_hex(
            &bounded_object_string(receipt, "operationEpoch", 16)?,
            "operationEpoch",
            false,
        )?,
        event_sequence: parse_u64_hex(
            &bounded_object_string(receipt, "eventSequence", 16)?,
            "eventSequence",
            false,
        )?,
        connection_id: parse_u64_hex(
            &bounded_object_string(receipt, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        lease_id: parse_u64_hex(
            &bounded_object_string(receipt, "leaseId", 16)?,
            "leaseId",
            false,
        )?,
        accepted: receipt
            .get::<bool>("accepted")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "receipt omits accepted"))?,
    })
}
