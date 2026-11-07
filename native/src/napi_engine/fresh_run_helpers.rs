//! Fresh run helpers for the coarse N-API boundary.

use super::*;

/// Validate an explicit stable identity before allocating any population.
/// Availability is supplied by runtime CPU detection in production.
pub(super) fn parse_requested_math_backend(
    label: Option<&str>,
    available: impl FnOnce(crate::engine::inference::InferenceMathBackend) -> bool,
) -> std::result::Result<Option<crate::engine::inference::InferenceMathBackend>, String> {
    let Some(label) = label else {
        return Ok(None);
    };
    let backend = crate::engine::inference::InferenceMathBackend::from_label(label)
        .ok_or_else(|| format!("unknown inference math backend: {label}"))?;
    if !available(backend) {
        return Err(format!(
            "inference backend {label} is unavailable on this CPU"
        ));
    }
    Ok(Some(backend))
}

/// Acquire the retained authority for a synchronous root without ever waiting.
pub(super) fn try_lock_fresh_inner(
    inner: &Mutex<ExperimentalFreshRunInner>,
) -> Result<MutexGuard<'_, ExperimentalFreshRunInner>> {
    match inner.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => Err(Error::new(
            Status::GenericFailure,
            "experimental fresh-run authority is busy on a worker thread",
        )),
    }
}

/// Reject every future mutation after one worker panic has faulted authority.
pub(super) fn reject_faulted_fresh_inner(inner: &ExperimentalFreshRunInner) -> Result<()> {
    match inner.fault_detail.as_ref() {
        Some(detail) => Err(Error::new(Status::GenericFailure, detail.clone())),
        None => Ok(()),
    }
}

/// Inspect the permanent fault latch without waiting for a worker-held mutex.
pub(super) fn reject_faulted_fresh_mutex(inner: &Mutex<ExperimentalFreshRunInner>) -> Result<()> {
    let inner = try_lock_fresh_inner(inner)?;
    reject_faulted_fresh_inner(&inner)
}

/// Latch the first caught worker panic and permanently retire the session.
pub(super) fn fault_experimental_fresh_run(
    inner: &Mutex<ExperimentalFreshRunInner>,
    context: &str,
    payload: &(dyn std::any::Any + Send),
) -> Error {
    let detail = bounded_fresh_run_panic_detail(context, payload);
    let retained = {
        let mut inner = lock_recover(inner);
        inner.fault_detail.get_or_insert(detail).clone()
    };
    Error::new(Status::GenericFailure, retained)
}

/// Retain the first bounded non-retryable authority-operation failure.
pub(super) fn retain_experimental_fresh_run_fault(
    inner: &mut ExperimentalFreshRunInner,
    context: &str,
    source: &str,
) -> String {
    let detail = bounded_fresh_run_error_detail(context, source);
    inner.fault_detail.get_or_insert(detail).clone()
}

/// Join bounded context and source text without retaining unbounded diagnostics.
pub(super) fn bounded_fresh_run_error_detail(context: &str, source: &str) -> String {
    let mut detail = truncate_utf8(context, MAX_ERROR_DETAIL_BYTES);
    if detail.len() < MAX_ERROR_DETAIL_BYTES {
        let separator = ": ";
        let separator_bytes = separator.len().min(MAX_ERROR_DETAIL_BYTES - detail.len());
        detail.push_str(&separator[..separator_bytes]);
    }
    if detail.len() < MAX_ERROR_DETAIL_BYTES {
        detail.push_str(&truncate_utf8(
            source,
            MAX_ERROR_DETAIL_BYTES - detail.len(),
        ));
    }
    detail
}

/// Bound panic text before allocating retained or N-API-visible diagnostics.
pub(super) fn bounded_fresh_run_panic_detail(
    context: &str,
    payload: &(dyn std::any::Any + Send),
) -> String {
    let source = if let Some(message) = payload.downcast_ref::<&str>() {
        *message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.as_str()
    } else {
        "non-string panic payload"
    };
    bounded_fresh_run_error_detail(context, source)
}

/// Ensure initialization has not already installed authority before scheduling.
pub(super) fn ensure_fresh_transition_absent(
    inner: &Mutex<ExperimentalFreshRunInner>,
) -> Result<()> {
    let inner = try_lock_fresh_inner(inner)?;
    reject_faulted_fresh_inner(&inner)?;
    if inner.transition.is_some()
        || inner.background_retry_loop.is_some()
        || inner.background_transferred
    {
        return Err(Error::new(
            Status::GenericFailure,
            "experimental fresh run is already initialized",
        ));
    }
    Ok(())
}

/// Ensure an admitted authority exists before scheduling dependent work.
pub(super) fn ensure_fresh_transition_present(
    inner: &Mutex<ExperimentalFreshRunInner>,
) -> Result<()> {
    let inner = try_lock_fresh_inner(inner)?;
    reject_faulted_fresh_inner(&inner)?;
    if inner.transition.is_none() {
        return Err(Error::new(
            Status::GenericFailure,
            "experimental fresh run has not been initialized",
        ));
    }
    Ok(())
}

/// Return the stable scalar phase for one active worker operation.
pub(super) const fn fresh_operation_phase(operation: u8) -> Option<&'static str> {
    match operation {
        FRESH_OPERATION_INITIALIZE => Some("initializing"),
        FRESH_OPERATION_CHECKPOINT => Some("publishingCheckpoint"),
        FRESH_OPERATION_ACTIVATE => Some("activating"),
        FRESH_OPERATION_ACKNOWLEDGE => Some("acknowledgingPersistence"),
        FRESH_OPERATION_INITIAL_FRAME => Some("publishingInitialFrame"),
        FRESH_OPERATION_FIRST_SCHEDULED_FRAME => Some("publishingFirstScheduledFrame"),
        FRESH_OPERATION_BACKGROUND => Some("preparingBackground"),
        _ => None,
    }
}

/// Produce an intentionally metadata-empty snapshot while the mutex is busy.
pub(super) fn busy_fresh_run_snapshot(operation: u8) -> Result<FreshRunScalarSnapshot> {
    let phase = fresh_operation_phase(operation).ok_or_else(|| {
        Error::new(
            Status::GenericFailure,
            "experimental fresh-run authority is unexpectedly busy without an active operation",
        )
    })?;
    Ok(FreshRunScalarSnapshot {
        phase,
        transition_epoch: None,
        generation: None,
        completed_step: None,
        checkpoint_published: None,
        persistence_acknowledged: None,
        authority_published: None,
        initial_frame_published: None,
        first_scheduled_frame_published: None,
        snake_count: None,
        pellet_count: None,
        fault_detail: None,
    })
}

/// Read the retained transition into bounded scalar-only Rust metadata.
pub(super) fn fresh_run_scalar_snapshot(
    inner: &ExperimentalFreshRunInner,
    active_operation: u8,
) -> Result<FreshRunScalarSnapshot> {
    if let Some(detail) = inner.fault_detail.as_ref() {
        return Ok(FreshRunScalarSnapshot {
            phase: "faulted",
            transition_epoch: None,
            generation: None,
            completed_step: None,
            checkpoint_published: None,
            persistence_acknowledged: None,
            authority_published: None,
            initial_frame_published: None,
            first_scheduled_frame_published: None,
            snake_count: None,
            pellet_count: None,
            fault_detail: Some(detail.clone()),
        });
    }
    let phase = if active_operation == FRESH_OPERATION_IDLE {
        match inner.transition.as_ref() {
            None if inner.background_transferred => "background",
            None if inner.background_retry_loop.is_some() => "preparingBackground",
            None => "created",
            Some(transition) if transition.authority_published() => "running",
            Some(transition) if transition.persistence_acknowledged() => "durableBoundary",
            Some(transition) if transition.checkpoint_published() => "awaitingPersistence",
            Some(_) => "pendingDurability",
        }
    } else {
        fresh_operation_phase(active_operation).ok_or_else(|| {
            Error::new(
                Status::GenericFailure,
                "experimental fresh-run operation state is invalid",
            )
        })?
    };
    let Some(transition) = inner.transition.as_ref() else {
        return Ok(FreshRunScalarSnapshot {
            phase,
            transition_epoch: None,
            generation: None,
            completed_step: None,
            checkpoint_published: None,
            persistence_acknowledged: None,
            authority_published: None,
            initial_frame_published: None,
            first_scheduled_frame_published: None,
            snake_count: None,
            pellet_count: None,
            fault_detail: None,
        });
    };
    Ok(FreshRunScalarSnapshot {
        phase,
        transition_epoch: Some(transition.transition_epoch()),
        generation: Some(transition.generation()),
        completed_step: Some(transition.completed_step()),
        checkpoint_published: Some(transition.checkpoint_published()),
        persistence_acknowledged: Some(transition.persistence_acknowledged()),
        authority_published: Some(transition.authority_published()),
        initial_frame_published: Some(inner.initial_frame_published),
        first_scheduled_frame_published: Some(inner.first_scheduled_frame_published),
        snake_count: Some(transition.snake_count()),
        pellet_count: Some(transition.pellet_count()),
        fault_detail: None,
    })
}

/// Convert one internal scalar snapshot without numeric narrowing.
pub(super) fn fresh_run_snapshot_to_napi(
    snapshot: FreshRunScalarSnapshot,
) -> Result<ExperimentalFreshRunSnapshot> {
    let snake_count = snapshot
        .snake_count
        .map(|value| {
            u64::try_from(value).map(u64_hex).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "fresh-run snake count exceeds the N-API scalar domain",
                )
            })
        })
        .transpose()?;
    let pellet_count = snapshot
        .pellet_count
        .map(|value| {
            u64::try_from(value).map(u64_hex).map_err(|_| {
                Error::new(
                    Status::GenericFailure,
                    "fresh-run pellet count exceeds the N-API scalar domain",
                )
            })
        })
        .transpose()?;
    Ok(ExperimentalFreshRunSnapshot {
        phase: snapshot.phase.to_owned(),
        transition_epoch: snapshot.transition_epoch.map(u64_hex),
        generation: snapshot.generation.map(u64_hex),
        completed_step: snapshot.completed_step.map(u64_hex),
        checkpoint_published: snapshot.checkpoint_published,
        persistence_acknowledged: snapshot.persistence_acknowledged,
        authority_published: snapshot.authority_published,
        initial_frame_published: snapshot.initial_frame_published,
        first_scheduled_frame_published: snapshot.first_scheduled_frame_published,
        snake_count,
        pellet_count,
        fault_detail: snapshot.fault_detail,
    })
}

/// Check and convert frame routing metadata before exposing exact hex scalars.
pub(super) fn fresh_run_frame_v1_output(
    bytes: Vec<u8>,
    metadata: FrameV1Metadata,
    completed_step: u64,
) -> Result<FreshRunFrameV1Output> {
    if metadata.byte_length != bytes.len() {
        return Err(Error::new(
            Status::GenericFailure,
            "fresh-run frame-v1 byte metadata does not match its payload",
        ));
    }
    Ok(FreshRunFrameV1Output {
        bytes,
        generation: metadata.generation,
        completed_step,
        total_snakes: frame_usize_to_u64(metadata.total_snakes, "totalSnakes")?,
        alive_snakes: frame_usize_to_u64(metadata.alive_snakes, "aliveSnakes")?,
        pellets: frame_usize_to_u64(metadata.pellets, "pellets")?,
        float_length: frame_usize_to_u64(metadata.float_length, "floatLength")?,
        byte_length: frame_usize_to_u64(metadata.byte_length, "byteLength")?,
    })
}

/// Convert one supported-target frame count without JavaScript Number narrowing.
pub(super) fn frame_usize_to_u64(value: usize, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            format!("fresh-run frame-v1 {field} exceeds the N-API scalar domain"),
        )
    })
}

/// Move one worker-produced frame into a typed array with exact hex metadata.
pub(super) fn fresh_run_frame_v1_to_napi(
    output: FreshRunFrameV1Output,
) -> ExperimentalFreshRunFrameV1 {
    ExperimentalFreshRunFrameV1 {
        bytes: Uint8Array::from(output.bytes),
        generation: u64_hex(output.generation),
        completed_step: u64_hex(output.completed_step),
        total_snakes: u64_hex(output.total_snakes),
        alive_snakes: u64_hex(output.alive_snakes),
        pellets: u64_hex(output.pellets),
        float_length: u64_hex(output.float_length),
        byte_length: u64_hex(output.byte_length),
    }
}

/// Convert one successful run-start activation without exposing state memory.
pub(super) fn run_start_publication_to_napi(
    publication: RunStartPublication,
) -> Stage6RunStartPublication {
    Stage6RunStartPublication {
        world_epoch: u64_hex(publication.world_epoch),
        generation: u64_hex(publication.generation),
        completed_step: u64_hex(publication.completed_step),
        population_epoch: u64_hex(publication.population_epoch),
    }
}

/// Parse one canonical positive u64 epoch without JavaScript Number narrowing.
#[cfg(feature = "engine-test-hooks")]
pub(super) fn parse_test_hook_epoch(value: &str) -> Result<u64> {
    parse_u64_hex(value, "transitionEpoch", false)
}
