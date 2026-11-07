//! Lease-source checks and resume-token encoding; replacement ownership stays in the parent.

use super::*;

/// Bytes of operating-system entropy used by one replacement resume token.
pub(super) const RESUME_TOKEN_BYTES: usize = 24;

/// Unpadded base64url characters produced from exactly 24 bytes.
pub(crate) const RESUME_TOKEN_LENGTH: usize = 32;

/// Bounded attempts to avoid the vanishingly unlikely live-token collision.
pub(super) const TOKEN_ATTEMPTS: usize = 8;

/// Base64url alphabet shared with Node's current `randomBytes(...).toString('base64url')`.
pub(super) const BASE64URL: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub(super) fn find_unique_snake(
    world: &WorldState,
    snake_id: u64,
) -> Result<&SnakeState, ExternalReplacementError> {
    let index = find_unique_snake_index(world, snake_id)?;
    Ok(&world.snakes[index])
}

pub(super) fn validate_generation_controller_source(
    world: &WorldState,
) -> Result<(usize, usize), ExternalReplacementError> {
    let mut connected = 0usize;
    for (index, lease) in world.controller_leases.iter().enumerate() {
        if world.controller_leases[..index]
            .iter()
            .any(|prior| prior.id == lease.id)
        {
            return Err(ExternalReplacementError::DuplicateLeaseId(lease.id));
        }
        if world.controller_leases[..index]
            .iter()
            .any(|prior| prior.snake_id == lease.snake_id)
        {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: lease.id,
                snake_id: lease.snake_id,
            });
        }
        let snake = find_unique_snake(world, lease.snake_id)?;
        // A God Mode death may already be committed before the generation's
        // final step. The lease still owns that dead external life and must
        // receive its successor; liveness does not invalidate ownership.
        if snake.kind != SnakeKind::External {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: lease.id,
                snake_id: lease.snake_id,
            });
        }
        match (lease.status, lease.connection_id) {
            (ControllerLeaseStatus::Connected, Some(connection_id)) if connection_id != 0 => {
                if world.controller_leases[..index]
                    .iter()
                    .any(|prior| prior.connection_id == Some(connection_id))
                {
                    return Err(ExternalReplacementError::InvalidLeaseTarget {
                        lease_id: lease.id,
                        snake_id: lease.snake_id,
                    });
                }
                connected = connected.checked_add(1).ok_or(
                    ExternalReplacementError::ArithmeticOverflow {
                        context: "connected generation controller count",
                    },
                )?;
            }
            (
                ControllerLeaseStatus::HoldingLastInput
                | ControllerLeaseStatus::ReservedNeutral
                | ControllerLeaseStatus::NeuralTakeover,
                None,
            ) => {}
            _ => {
                return Err(ExternalReplacementError::InvalidLeaseTarget {
                    lease_id: lease.id,
                    snake_id: lease.snake_id,
                });
            }
        }
    }
    let omitted = world.controller_leases.len().checked_sub(connected).ok_or(
        ExternalReplacementError::ArithmeticOverflow {
            context: "omitted generation controller count",
        },
    )?;
    Ok((connected, omitted))
}

pub(super) fn unavailable_reservation(
    lease: &crate::engine::state::ControllerLease,
) -> Result<UnavailableControllerReservation, ExternalReplacementError> {
    let reason = match (lease.status, lease.connection_id) {
        (
            ControllerLeaseStatus::HoldingLastInput | ControllerLeaseStatus::ReservedNeutral,
            None,
        ) => UnavailableControllerReason::SnakeUnavailable,
        (ControllerLeaseStatus::NeuralTakeover, None) => UnavailableControllerReason::GraceExpired,
        _ => {
            return Err(ExternalReplacementError::InvalidLeaseTarget {
                lease_id: lease.id,
                snake_id: lease.snake_id,
            });
        }
    };
    Ok(UnavailableControllerReservation {
        source_lease_id: lease.id,
        source_snake_id: lease.snake_id,
        controller_kind: lease.kind,
        scope: lease.scope.clone(),
        resume_token: lease.resume_token.clone(),
        disconnected_at_ms: lease.disconnected_at_ms,
        grace_expires_at_ms: lease.grace_expires_at_ms,
        reason,
    })
}

pub(super) fn find_unique_snake_index(
    world: &WorldState,
    snake_id: u64,
) -> Result<usize, ExternalReplacementError> {
    let mut found = None;
    for (index, snake) in world.snakes.iter().enumerate() {
        if snake.id != snake_id {
            continue;
        }
        if found.replace(index).is_some() {
            return Err(ExternalReplacementError::DuplicateSnakeId(snake_id));
        }
    }
    found.ok_or(ExternalReplacementError::UnknownSnake(snake_id))
}

/// Reuse the same bounded OS-entropy token format for an explicit reconnect.
pub(crate) fn fresh_resume_token(
    leases: &[crate::engine::state::ControllerLease],
) -> Result<String, ExternalReplacementError> {
    let mut token = String::new();
    for _ in 0..TOKEN_ATTEMPTS {
        let mut bytes = [0u8; RESUME_TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| ExternalReplacementError::EntropyUnavailable)?;
        encode_base64url_24(&bytes, &mut token)?;
        if !leases.iter().any(|lease| lease.resume_token == token) {
            return Ok(token);
        }
    }
    Err(ExternalReplacementError::TokenCollision)
}

pub(super) fn encode_base64url_24(
    bytes: &[u8; RESUME_TOKEN_BYTES],
    output: &mut String,
) -> Result<(), ExternalReplacementError> {
    reserve_string(output, RESUME_TOKEN_LENGTH, "replacement resume token")?;
    output.clear();
    for chunk in bytes.as_chunks::<3>().0 {
        let bits = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        output.push(BASE64URL[((bits >> 18) & 0x3f) as usize] as char);
        output.push(BASE64URL[((bits >> 12) & 0x3f) as usize] as char);
        output.push(BASE64URL[((bits >> 6) & 0x3f) as usize] as char);
        output.push(BASE64URL[(bits & 0x3f) as usize] as char);
    }
    if output.len() != RESUME_TOKEN_LENGTH {
        return Err(ExternalReplacementError::InternalShapeMismatch);
    }
    Ok(())
}
