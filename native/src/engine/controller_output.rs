//! Reliable ordinary controller output owned by the background authority queue.

use std::mem::size_of;

use super::contract::{ExternalDeliveryReceipt, RunningAuthorityEvent, RunningControllerMessage};
use super::error::{EngineError, EngineErrorCode};
use super::running_loop::{
    RunningAuthorityDeliveryState, RunningAuthorityLoop, RunningAuthorityLoopState,
};
use super::running_step::{
    ExternalDeliveryEventKind, ExternalDeliveryResult, ExternalObservationBatch,
};
use super::state::ControllerLeaseStatus;
use super::world_step::ExternalDeliveryStatus;

fn overflow() -> EngineError {
    EngineError::new(
        EngineErrorCode::QueueByteLimit,
        "controller output exceeds bounded reply storage",
    )
}

/// Cover both a pre-movement observation and a death replacement per connected
/// lease before preparation can consume controller observations or randomness.
pub(crate) fn service_reply_bound(running: &RunningAuthorityLoop) -> Result<usize, EngineError> {
    let connections = running
        .generation_source_controller_leases()
        .iter()
        .filter(|lease| {
            lease.status == ControllerLeaseStatus::Connected && lease.connection_id.is_some()
        })
        .count();
    let sensor_bytes = (super::sensor_layout::SENSOR_SCALAR_COUNT
        + super::sensor_layout::SENSOR_CHANNEL_COUNT * super::sensor_layout::MAX_SENSOR_BINS)
        .checked_mul(size_of::<f32>())
        .ok_or_else(overflow)?;
    let per_event = size_of::<RunningControllerMessage>()
        .checked_add(sensor_bytes)
        .and_then(|bytes| bytes.checked_add(super::external_replacement::RESUME_TOKEN_LENGTH))
        .ok_or_else(overflow)?;
    connections
        .checked_mul(2)
        .and_then(|count| count.checked_mul(per_event))
        .and_then(|bytes| bytes.checked_add(size_of::<RunningAuthorityEvent>()))
        .ok_or_else(overflow)
}

/// Copy only unresolved retained messages after the complete queue reservation.
pub(crate) fn own_pending_messages(
    running: &RunningAuthorityLoop,
) -> Result<Box<[RunningControllerMessage]>, EngineError> {
    let batch = running.pending_external_delivery().ok_or_else(|| {
        EngineError::new(
            EngineErrorCode::Faulted,
            "ordinary controller blocker lost its retained messages",
        )
    })?;
    let mut messages = Vec::new();
    messages
        .try_reserve_exact(batch.remaining())
        .map_err(|_| overflow())?;
    for (index, event) in batch.events().iter().copied().enumerate() {
        if batch.status(index) != Some(ExternalDeliveryStatus::Pending) {
            continue;
        }
        let (frame_v1_id, samples, token) = match event.delivery_kind {
            ExternalDeliveryEventKind::Observation => (
                running.controller_frame_v1_id(event.snake_id),
                batch.observation(index),
                None,
            ),
            ExternalDeliveryEventKind::ReplacementAssignment { frame_v1_id } => {
                (Some(frame_v1_id), Some(&[][..]), batch.resume_token(index))
            }
        };
        let frame_v1_id = frame_v1_id.ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::Faulted,
                "controller observation lost its source frame identity",
            )
        })?;
        let samples = samples.ok_or_else(|| {
            EngineError::new(
                EngineErrorCode::Faulted,
                "controller observation lost its retained sensors",
            )
        })?;
        if matches!(
            event.delivery_kind,
            ExternalDeliveryEventKind::ReplacementAssignment { .. }
        ) && token.is_none()
        {
            return Err(EngineError::new(
                EngineErrorCode::Faulted,
                "controller replacement lost its retained token",
            ));
        }
        let mut sensors = Vec::new();
        sensors
            .try_reserve_exact(samples.len())
            .map_err(|_| overflow())?;
        sensors.extend_from_slice(samples);
        let resume_token = token
            .map(|token| {
                let mut owned = String::new();
                owned
                    .try_reserve_exact(token.len())
                    .map_err(|_| overflow())?;
                owned.push_str(token);
                Ok::<_, EngineError>(owned.into_boxed_str())
            })
            .transpose()?;
        messages.push(RunningControllerMessage {
            event,
            frame_v1_id,
            sensors: sensors.into_boxed_slice(),
            resume_token,
        });
    }
    Ok(messages.into_boxed_slice())
}

/// Reconstruct exact Rust keys only for matching retained receipts. The same
/// matcher serves ordinary and generation barriers, after their phase checks.
pub(crate) fn correlate_receipts(
    batch: ExternalObservationBatch<'_>,
    receipts: &[ExternalDeliveryReceipt],
) -> Result<(Vec<ExternalDeliveryResult>, usize), EngineError> {
    let mut exact = Vec::new();
    exact
        .try_reserve_exact(receipts.len())
        .map_err(|_| overflow())?;
    let mut ignored = 0usize;
    for receipt in receipts {
        let event = batch
            .events()
            .binary_search_by_key(&receipt.event_sequence, |event| event.event_sequence)
            .ok()
            .and_then(|index| batch.events().get(index))
            .filter(|event| {
                event.step_key.operation_epoch() == receipt.operation_epoch
                    && event.connection_id == receipt.connection_id
                    && event.lease_id == receipt.lease_id
            });
        if let Some(event) = event {
            exact.push(ExternalDeliveryResult {
                step_key: event.step_key,
                event_sequence: receipt.event_sequence,
                connection_id: receipt.connection_id,
                lease_id: receipt.lease_id,
                accepted: receipt.accepted,
            });
        } else {
            ignored = ignored.saturating_add(1);
        }
    }
    Ok((exact, ignored))
}

/// An ordinary completion is admitted before it can publish or retire a step.
pub(crate) fn submit_receipts(
    command_sequence: u64,
    receipts: &[ExternalDeliveryReceipt],
    running: &mut RunningAuthorityLoop,
) -> Result<RunningAuthorityEvent, EngineError> {
    if running.state() != RunningAuthorityLoopState::ExternalDeliveryPending {
        return Err(EngineError::new(
            EngineErrorCode::InvalidCommand,
            "ordinary controller receipts require a retained ordinary step",
        ));
    }
    let batch = running.pending_external_delivery().ok_or_else(|| {
        EngineError::new(
            EngineErrorCode::Faulted,
            "ordinary controller batch is missing",
        )
    })?;
    let (exact, bridge_ignored) = correlate_receipts(batch, receipts)?;
    let resolution = running
        .submit_external_delivery_results(&exact, None)
        .map_err(|error| EngineError::new(EngineErrorCode::Faulted, error.to_string()))?;
    let (remaining, published_completed_step) = match resolution.state {
        RunningAuthorityDeliveryState::RunningStepPending { remaining, .. } => (remaining, None),
        RunningAuthorityDeliveryState::RunningStepPublished { publication, .. } => {
            (0, Some(publication.completed_step))
        }
        _ => {
            return Err(EngineError::new(
                EngineErrorCode::Faulted,
                "ordinary receipts resolved outside their step barrier",
            ))
        }
    };
    Ok(RunningAuthorityEvent::ControllerDeliveryReceiptsApplied {
        command_sequence,
        matched_acceptances: resolution.matched_acceptances,
        matched_failures: resolution.matched_failures,
        ignored_receipts: resolution.ignored_results.saturating_add(bridge_ignored),
        remaining,
        published_completed_step,
    })
}
