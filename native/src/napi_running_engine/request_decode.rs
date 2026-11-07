//! Bounded request decoding without authority ownership.

use super::*;

/// Parse the metadata worker's exact bounded export inventory descriptor.
pub(super) fn parse_export_inventory_descriptor(
    descriptor: &Object<'_>,
    operation_id: &str,
) -> Result<ExportInventoryDescriptor> {
    const KEYS: [&str; 6] = [
        "version",
        "relativeFilename",
        "sha256",
        "storedByteCount",
        "historyCount",
        "hallOfFameCount",
    ];
    let names = descriptor.get_property_names()?;
    if names.get_array_length()? != KEYS.len() as u32 {
        return Err(Error::new(
            Status::InvalidArg,
            "export inventory has unknown or missing fields",
        ));
    }
    let mut seen = [false; KEYS.len()];
    for index in 0..names.get_array_length()? {
        let key = names.get_element::<JsString<'_>>(index)?;
        let key = bounded_js_string(key, "export inventory key", 32, false)?;
        let position = KEYS
            .iter()
            .position(|expected| *expected == key)
            .ok_or_else(|| {
                Error::new(
                    Status::InvalidArg,
                    "export inventory contains an unknown field",
                )
            })?;
        if seen[position] || !descriptor.has_own_property(KEYS[position])? {
            return Err(Error::new(
                Status::InvalidArg,
                "export inventory has inherited or duplicate fields",
            ));
        }
        seen[position] = true;
    }
    let version = descriptor
        .get::<f64>("version")?
        .ok_or_else(|| Error::new(Status::InvalidArg, "export inventory omits version"))?;
    if version != 1.0 && version != 2.0 && version != 3.0 {
        return Err(Error::new(
            Status::InvalidArg,
            "export inventory version is unsupported",
        ));
    }
    let relative_filename = bounded_object_string(
        descriptor,
        "relativeFilename",
        32 + ".export-inventory-v1".len() + 1,
    )?;
    if relative_filename != format!(".{operation_id}.export-inventory-v{}", version as u32) {
        return Err(Error::new(
            Status::InvalidArg,
            "export inventory does not match the operation ID",
        ));
    }
    Ok(ExportInventoryDescriptor {
        version: version as u32,
        relative_filename,
        sha256: bounded_object_string(descriptor, "sha256", 64)?,
        stored_byte_count_hex: bounded_object_string(descriptor, "storedByteCount", 16)?,
        history_count_hex: bounded_object_string(descriptor, "historyCount", 16)?,
        hall_of_fame_count_hex: bounded_object_string(descriptor, "hallOfFameCount", 16)?,
    })
}

/// Parse one optional bounded identity without materializing an unbounded string.
pub(super) fn optional_reclaim_identity(
    request: &Object<'_>,
    field: &str,
    maximum: usize,
) -> Result<String> {
    match request.get::<JsString<'_>>(field)? {
        Some(value) => bounded_js_string(value, field, maximum, false),
        None => Ok(String::new()),
    }
}

/// Bound and copy one small numeric settings array before native queue admission.
pub(super) fn parse_live_settings(
    updates: &Array<'_>,
) -> Result<Box<[crate::engine::live_settings::LiveSettingUpdate]>> {
    let length = usize::try_from(updates.len())
        .map_err(|_| Error::new(Status::InvalidArg, "live settings length exceeds usize"))?;
    if length == 0 || length > crate::engine::live_settings::MAXIMUM_LIVE_SETTING_UPDATES {
        return Err(Error::new(
            Status::InvalidArg,
            "live settings require 1 to 64 updates",
        ));
    }
    let mut parsed = Vec::new();
    parsed
        .try_reserve_exact(length)
        .map_err(|_| Error::new(Status::GenericFailure, "live settings allocation failed"))?;
    for index in 0..updates.len() {
        let update = updates
            .get::<Object<'_>>(index)?
            .ok_or_else(|| Error::new(Status::InvalidArg, "live setting must be an object"))?;
        let path = bounded_object_string(&update, "path", 128)?;
        let value = update
            .get::<f64>("value")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "live setting omits value"))?;
        if !value.is_finite() {
            return Err(Error::new(
                Status::InvalidArg,
                "live setting value must be finite",
            ));
        }
        parsed.push(crate::engine::live_settings::LiveSettingUpdate { path, value });
    }
    Ok(parsed.into_boxed_slice())
}

/// Bound one complete fresh-run settings projection before libuv task admission.
pub(super) fn parse_fresh_run_settings(
    updates: &Array<'_>,
) -> Result<Box<[FreshRunSettingUpdate]>> {
    let length = usize::try_from(updates.len()).map_err(|_| {
        Error::new(
            Status::InvalidArg,
            "fresh-run settings length exceeds usize",
        )
    })?;
    if length == 0 || length > 128 {
        return Err(Error::new(
            Status::InvalidArg,
            "fresh-run settings require 1 to 128 updates",
        ));
    }
    let mut parsed = Vec::new();
    parsed.try_reserve_exact(length).map_err(|_| {
        Error::new(
            Status::GenericFailure,
            "fresh-run settings allocation failed",
        )
    })?;
    for index in 0..updates.len() {
        let update = updates
            .get::<Object<'_>>(index)?
            .ok_or_else(|| Error::new(Status::InvalidArg, "fresh-run setting must be an object"))?;
        let path = bounded_object_string(&update, "path", 128)?;
        let value = update
            .get::<f64>("value")?
            .ok_or_else(|| Error::new(Status::InvalidArg, "fresh-run setting omits value"))?;
        if !value.is_finite() {
            return Err(Error::new(
                Status::InvalidArg,
                "fresh-run setting value must be finite",
            ));
        }
        parsed.push(FreshRunSettingUpdate { path, value });
    }
    Ok(parsed.into_boxed_slice())
}

/// Decode one bounded graph document before the background task owns it.
pub(super) fn parse_fresh_run_graph(encoded: JsString<'_>) -> Result<GraphSpec> {
    let encoded = bounded_js_string(encoded, "graphSpecJson", 1024 * 1024, false)?;
    crate::engine::graph_json::decode_graph_json(encoded.as_bytes())
        .map_err(|message| Error::new(Status::InvalidArg, message))
}

/// Validate a fresh legacy identity and stamp its native receipt time.
pub(crate) fn parse_controller_join(
    request: &Object<'_>,
) -> Result<crate::engine::contract::ControllerJoinRequest> {
    let kind = match bounded_object_string(request, "controllerKind", 32)?.as_str() {
        "player" => crate::engine::state::ControllerKind::Player,
        "reinforcementLearning" => crate::engine::state::ControllerKind::ReinforcementLearning,
        _ => return Err(Error::new(Status::InvalidArg, "invalid controllerKind")),
    };
    Ok(crate::engine::contract::ControllerJoinRequest {
        connection_id: parse_u64_hex(
            &bounded_object_string(request, "connectionId", 16)?,
            "connectionId",
            false,
        )?,
        kind,
        identity_key: bounded_object_string(request, "identityKey", 128)?,
        received_at: std::time::Instant::now(),
    })
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
    let client_id = optional_reclaim_identity(action, "clientActionId", 16)?;
    let client_action_id = if client_id.is_empty() {
        None
    } else {
        Some(parse_u64_hex(&client_id, "clientActionId", false)?)
    };
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
        client_action_id,
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
