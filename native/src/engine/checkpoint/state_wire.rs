//! State wire for managed checkpoints.

use super::*;

/// Decoded non-population state fields; numeric population buffers arrive separately.
#[derive(Clone)]
pub(super) struct DecodedStateParts {
    pub(super) versions: ContractVersions,
    pub(super) identity: RunIdentity,
    pub(super) config: NormalizedEngineConfig,
    pub(super) boundary_kind: GenerationBoundaryKind,
    pub(super) generation: GenerationState,
    pub(super) rng: RngStateBundle,
    pub(super) allocators: AllocatorState,
}

/// Bounded explicit little-endian binary role encoder.
pub(super) struct BinaryWriter {
    pub(super) bytes: Vec<u8>,
    pub(super) max_bytes: usize,
    pub(super) max_string_bytes: usize,
    pub(super) max_total_string_bytes: usize,
    pub(super) total_string_bytes: usize,
}

impl BinaryWriter {
    /// Construct a role encoder with exact byte and text budgets.
    pub(super) fn new(max_bytes: usize, limits: &CheckpointLimits) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
            max_string_bytes: limits.max_string_bytes,
            max_total_string_bytes: limits.max_total_string_bytes,
            total_string_bytes: 0,
        }
    }

    /// Reserve before extending the role buffer.
    pub(super) fn reserve(&mut self, additional: usize) -> Result<(), CheckpointError> {
        let new_len = self.bytes.len().checked_add(additional).ok_or_else(|| {
            CheckpointError::format("BINARY_OVERFLOW", "binary role length overflows usize")
        })?;
        if new_len > self.max_bytes {
            return Err(CheckpointError::format(
                "BINARY_LIMIT",
                format!(
                    "binary role bytes {new_len} exceed limit {}",
                    self.max_bytes
                ),
            ));
        }
        self.bytes.try_reserve_exact(additional).map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve binary role bytes")
        })
    }

    /// Append exact bytes.
    pub(super) fn raw(&mut self, value: &[u8]) -> Result<(), CheckpointError> {
        self.reserve(value.len())?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    /// Append one byte.
    pub(super) fn u8(&mut self, value: u8) -> Result<(), CheckpointError> {
        self.raw(&[value])
    }

    /// Append a little-endian `u32`.
    pub(super) fn u32(&mut self, value: u32) -> Result<(), CheckpointError> {
        self.raw(&value.to_le_bytes())
    }

    /// Append a little-endian `u64`.
    pub(super) fn u64(&mut self, value: u64) -> Result<(), CheckpointError> {
        self.raw(&value.to_le_bytes())
    }

    /// Append a little-endian `i64`.
    pub(super) fn i64(&mut self, value: i64) -> Result<(), CheckpointError> {
        self.raw(&value.to_le_bytes())
    }

    /// Append exact `f64` bits.
    pub(super) fn f64(&mut self, value: f64) -> Result<(), CheckpointError> {
        self.u64(value.to_bits())
    }

    /// Append a bounded length-prefixed UTF-8 string.
    pub(super) fn string(&mut self, value: &str) -> Result<(), CheckpointError> {
        if value.len() > self.max_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                format!(
                    "string bytes {} exceed limit {}",
                    value.len(),
                    self.max_string_bytes
                ),
            ));
        }
        self.total_string_bytes = self
            .total_string_bytes
            .checked_add(value.len())
            .ok_or_else(|| {
                CheckpointError::format("STRING_OVERFLOW", "aggregate string bytes overflow")
            })?;
        if self.total_string_bytes > self.max_total_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                "aggregate string bytes exceed limit",
            ));
        }
        self.u32(usize_to_u32(value.len(), "string byte length")?)?;
        self.raw(value.as_bytes())
    }

    /// Finish one already-bounded role.
    pub(super) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Preflighted explicit little-endian binary role decoder.
pub(super) struct BinaryReader<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) offset: usize,
    pub(super) max_string_bytes: usize,
    pub(super) max_total_string_bytes: usize,
    pub(super) total_string_bytes: usize,
}

impl<'a> BinaryReader<'a> {
    /// Construct a reader after the outer role-size limit has passed.
    pub(super) fn new(bytes: &'a [u8], limits: &CheckpointLimits) -> Self {
        Self {
            bytes,
            offset: 0,
            max_string_bytes: limits.max_string_bytes,
            max_total_string_bytes: limits.max_total_string_bytes,
            total_string_bytes: 0,
        }
    }

    /// Borrow an exact number of remaining bytes.
    pub(super) fn raw(&mut self, length: usize) -> Result<&'a [u8], CheckpointError> {
        let end = self.offset.checked_add(length).ok_or_else(|| {
            CheckpointError::format("BINARY_OVERFLOW", "binary read offset overflows")
        })?;
        let value = self.bytes.get(self.offset..end).ok_or_else(|| {
            CheckpointError::format("BINARY_TRUNCATED", "binary role ends inside a field")
        })?;
        self.offset = end;
        Ok(value)
    }

    /// Decode one byte.
    pub(super) fn u8(&mut self) -> Result<u8, CheckpointError> {
        Ok(self.raw(1)?[0])
    }

    /// Decode one little-endian `u32`.
    pub(super) fn u32(&mut self) -> Result<u32, CheckpointError> {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(self.raw(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    /// Decode one little-endian `u64`.
    pub(super) fn u64(&mut self) -> Result<u64, CheckpointError> {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(self.raw(8)?);
        Ok(u64::from_le_bytes(bytes))
    }

    /// Decode one little-endian `i64`.
    pub(super) fn i64(&mut self) -> Result<i64, CheckpointError> {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(self.raw(8)?);
        Ok(i64::from_le_bytes(bytes))
    }

    /// Decode exact `f64` bits; state admission performs semantic validation.
    pub(super) fn f64(&mut self) -> Result<f64, CheckpointError> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// Decode a bounded UTF-8 string only after all declared lengths pass.
    pub(super) fn string(&mut self) -> Result<String, CheckpointError> {
        self.string_bounded(self.max_string_bytes)
    }

    /// Enforce a role-specific limit before allocating a UTF-8 string.
    pub(super) fn string_bounded(&mut self, maximum: usize) -> Result<String, CheckpointError> {
        let length = u32_to_usize(self.u32()?, "string byte length")?;
        let maximum = maximum.min(self.max_string_bytes);
        if length > maximum {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                format!("declared string bytes {length} exceed limit {}", maximum),
            ));
        }
        self.total_string_bytes = self.total_string_bytes.checked_add(length).ok_or_else(|| {
            CheckpointError::format("STRING_OVERFLOW", "aggregate string bytes overflow")
        })?;
        if self.total_string_bytes > self.max_total_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                "aggregate declared string bytes exceed limit",
            ));
        }
        let bytes = self.raw(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| {
            CheckpointError::format("STRING_UTF8", "binary string field is not valid UTF-8")
        })?;
        Ok(value.to_owned())
    }

    /// Require exact role exhaustion.
    pub(super) fn finish(self) -> Result<(), CheckpointError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(CheckpointError::format(
                "BINARY_TRAILING",
                format!(
                    "binary role has {} trailing bytes",
                    self.bytes.len() - self.offset
                ),
            ))
        }
    }
}

/// Encode the exact nonnumeric generation-boundary state.
pub(super) fn encode_state(
    state: &StateCandidate,
    limits: &CheckpointLimits,
) -> Result<Vec<u8>, CheckpointError> {
    let mut output = BinaryWriter::new(limits.max_state_bytes, limits);
    output.raw(STATE_MAGIC)?;
    output.u32(if state.identity.founder.is_some() {
        STATE_ENCODING_WITH_FOUNDER
    } else {
        STATE_ENCODING_WITH_LEGACY_ORIGIN
    })?;
    let versions = &state.versions;
    for value in [
        versions.state,
        versions.engine,
        versions.protocol,
        versions.serializer,
        versions.sensor,
        versions.rng_bundle,
        versions.checkpoint,
        versions.graph_layout,
    ] {
        output.u32(value)?;
    }
    encode_identity(&mut output, &state.identity)?;
    encode_config(&mut output, &state.config, limits)?;
    let kind = match state.phase {
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart) => 1,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation) => 2,
        AuthorityPhase::Running => {
            return Err(CheckpointError::format(
                "DIRTY_BOUNDARY",
                "running state cannot be checkpoint encoded",
            ));
        }
    };
    output.u8(kind)?;
    encode_generation(&mut output, &state.generation)?;
    encode_rng_bundle(&mut output, &state.rng, limits)?;
    encode_allocators(&mut output, &state.allocators)?;
    match &state.identity.legacy_conversion {
        None => output.u8(0)?,
        Some(origin) => {
            output.u8(1)?;
            let encoded = origin
                .encode()
                .map_err(|error| CheckpointError::format("LEGACY_ORIGIN", error))?;
            output.string(&encoded)?;
        }
    }
    if let Some(origin) = &state.identity.founder {
        origin
            .validate()
            .map_err(|error| CheckpointError::format("FOUNDER_ORIGIN", error))?;
        output.string(
            &serde_json::to_string(origin)
                .map_err(|error| CheckpointError::format("FOUNDER_ORIGIN", error.to_string()))?,
        )?;
    }
    Ok(output.finish())
}

/// Encode run/build identity fields in stable order.
pub(super) fn encode_identity(
    output: &mut BinaryWriter,
    identity: &RunIdentity,
) -> Result<(), CheckpointError> {
    output.string(&identity.run_id)?;
    output.u32(identity.seed)?;
    output.u64(identity.config_revision)?;
    for value in [
        identity.config_hash.as_str(),
        identity.source_revision.as_str(),
        identity.engine_build_id.as_str(),
        identity.source_sha256.as_str(),
        identity.target_triple.as_str(),
        identity.build_profile.as_str(),
        identity.build_class.as_str(),
        identity.rustc_version.as_str(),
        identity.build_contract_sha256.as_str(),
        identity.math_backend.as_str(),
    ] {
        output.string(value)?;
    }
    Ok(())
}

/// Encode complete normalized configuration without a generic serialization seam.
pub(super) fn encode_config(
    output: &mut BinaryWriter,
    config: &NormalizedEngineConfig,
    limits: &CheckpointLimits,
) -> Result<(), CheckpointError> {
    if config.settings.len() > limits.max_setting_count {
        return Err(CheckpointError::format(
            "SETTING_LIMIT",
            "normalized setting count exceeds checkpoint limit",
        ));
    }
    output.u32(config.version)?;
    output.u32(usize_to_u32(config.settings.len(), "setting count")?)?;
    for setting in &config.settings {
        output.string(&setting.path)?;
        match &setting.value {
            NormalizedSettingValue::Bool(value) => {
                output.u8(1)?;
                output.u8(u8::from(*value))?;
            }
            NormalizedSettingValue::Integer(value) => {
                output.u8(2)?;
                output.i64(*value)?;
            }
            NormalizedSettingValue::Float(value) => {
                output.u8(3)?;
                output.f64(*value)?;
            }
            NormalizedSettingValue::Text(value) => {
                output.u8(4)?;
                output.string(value)?;
            }
        }
    }
    output.string(&config.settings_schema_sha256)?;
    output.string(&config.graph_architecture_key)?;
    output.f64(config.fixed_step_seconds)?;
    output.f64(config.requested_sim_speed)?;
    output.f64(config.world_radius)?;
    for value in [
        config.population_count,
        config.baseline_count,
        config.max_world_snakes,
        config.max_non_population_brains,
        config.max_body_points,
        config.max_pellets,
        config.spatial_index_bytes,
        config.worker_scratch_bytes,
        config.checkpoint_scratch_bytes,
    ] {
        output.u64(usize_to_u64(value, "normalized configuration count")?)?;
    }
    output.u64(config.controller_input_hold_ms)?;
    output.u64(config.controller_disconnect_grace_ms)?;
    Ok(())
}

/// Encode the minimal scheduler/generation continuation.
pub(super) fn encode_generation(
    output: &mut BinaryWriter,
    value: &GenerationState,
) -> Result<(), CheckpointError> {
    output.u32(value.boundary_version)?;
    output.u64(value.generation)?;
    output.u64(value.completed_step)?;
    output.u64(value.population_epoch)?;
    output.f64(value.elapsed_seconds)?;
    output.f64(value.wall_accumulator_seconds)?;
    output.f64(value.best_fitness_ever)
}

/// Encode all independent authoritative RNG continuations.
pub(super) fn encode_rng_bundle(
    output: &mut BinaryWriter,
    value: &RngStateBundle,
    limits: &CheckpointLimits,
) -> Result<(), CheckpointError> {
    if !matches!(
        value.version,
        crate::engine::state::RNG_BUNDLE_VERSION
            | crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION
            | crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION
    ) {
        return Err(CheckpointError::format(
            "RNG_VERSION",
            "unsupported RNG bundle encoding",
        ));
    }
    if value.version == crate::engine::state::RNG_BUNDLE_VERSION
        && !value.evolved_respawns.is_empty()
    {
        return Err(CheckpointError::format(
            "RNG_STATE",
            "frozen RNG bundle has evolved streams",
        ));
    }
    if value.version < crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION
        && !value.baseline_behaviors.is_empty()
    {
        return Err(CheckpointError::format(
            "RNG_STATE",
            "frozen RNG bundles cannot carry baseline behavior",
        ));
    }
    if value.baselines.len() > limits.max_baseline_rng_count {
        return Err(CheckpointError::format(
            "RNG_LIMIT",
            "baseline RNG count exceeds checkpoint limit",
        ));
    }
    output.u32(value.version)?;
    encode_rng(output, &value.world)?;
    encode_rng(output, &value.evolution)?;
    encode_rng(output, &value.external_controller)?;
    output.u32(usize_to_u32(value.baselines.len(), "baseline RNG count")?)?;
    for baseline in &value.baselines {
        output.u32(baseline.slot)?;
        encode_rng(output, &baseline.state)?;
    }
    if value.version >= crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION {
        if value.evolved_respawns.len() > limits.max_population_count
            || value.evolved_respawns.len() > 300
        {
            return Err(CheckpointError::format(
                "RNG_LIMIT",
                "evolved RNG count exceeds checkpoint limit",
            ));
        }
        output.u32(usize_to_u32(
            value.evolved_respawns.len(),
            "evolved RNG count",
        )?)?;
        for (slot, stream) in value.evolved_respawns.iter().enumerate() {
            if stream.slot as usize != slot || stream.uniform_state == 0 {
                return Err(CheckpointError::format(
                    "RNG_STATE",
                    "evolved RNG slots must be dense and nonzero",
                ));
            }
            output.u32(stream.slot)?;
            output.u32(stream.uniform_state)?;
        }
    }
    if value.version == crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION {
        let streams = &value.baseline_behaviors;
        if streams.len() > limits.max_baseline_rng_count || streams.len() > 120 {
            return Err(CheckpointError::format(
                "RNG_LIMIT",
                "baseline behavior count exceeds checkpoint limit",
            ));
        }
        if streams.len() != value.baselines.len() {
            return Err(CheckpointError::format(
                "RNG_STATE",
                "behavior count must match baseline slots",
            ));
        }
        output.u32(usize_to_u32(streams.len(), "baseline behavior count")?)?;
        for (slot, stream) in streams.iter().enumerate() {
            if stream.slot as usize != slot || stream.uniform_state == 0 {
                return Err(CheckpointError::format(
                    "RNG_STATE",
                    "behavior slots must be dense and nonzero",
                ));
            }
            output.u32(stream.slot)?;
            output.u32(stream.uniform_state)?;
        }
    }
    Ok(())
}

/// Encode one lossless RNG continuation.
pub(super) fn encode_rng(
    output: &mut BinaryWriter,
    value: &SerializedRngState,
) -> Result<(), CheckpointError> {
    output.string(&value.algorithm)?;
    output.u32(value.version)?;
    output.string(&value.state_hex)?;
    output.string(&value.gaussian_algorithm)?;
    output.u32(value.gaussian_version)?;
    output.u8(u8::from(value.gaussian_spare_valid))?;
    match &value.gaussian_spare_hex {
        Some(spare) => {
            output.u8(1)?;
            output.string(spare)?;
        }
        None => output.u8(0)?,
    }
    Ok(())
}

/// Encode monotonic allocator continuations.
pub(super) fn encode_allocators(
    output: &mut BinaryWriter,
    value: &AllocatorState,
) -> Result<(), CheckpointError> {
    output.u32(value.version)?;
    for next in [
        value.next_entity_id,
        value.next_brain_id,
        value.next_genome_id,
        value.next_controller_lease_id,
    ] {
        output.u64(next)?;
    }
    output.u32(value.next_frame_v1_id)?;
    for next in [
        value.next_external_id,
        value.next_baseline_id,
        value.next_resurrected_id,
    ] {
        output.u64(next)?;
    }
    Ok(())
}

/// Decode the exact nonnumeric state role after its outer size/digest checks.
pub(super) fn decode_state(
    bytes: &[u8],
    limits: &CheckpointLimits,
) -> Result<DecodedStateParts, CheckpointError> {
    let mut input = BinaryReader::new(bytes, limits);
    if input.raw(STATE_MAGIC.len())? != STATE_MAGIC {
        return Err(CheckpointError::format(
            "STATE_MAGIC",
            "unsupported checkpoint state role",
        ));
    }
    let state_encoding = input.u32()?;
    if ![
        CHECKPOINT_VERSION,
        STATE_ENCODING_WITH_LEGACY_ORIGIN,
        STATE_ENCODING_WITH_FOUNDER,
    ]
    .contains(&state_encoding)
    {
        return Err(CheckpointError::format(
            "STATE_VERSION",
            "unsupported checkpoint state encoding",
        ));
    }
    let versions = ContractVersions {
        state: input.u32()?,
        engine: input.u32()?,
        protocol: input.u32()?,
        serializer: input.u32()?,
        sensor: input.u32()?,
        rng_bundle: input.u32()?,
        checkpoint: input.u32()?,
        graph_layout: input.u32()?,
    };
    let mut identity = decode_identity(&mut input)?;
    let config = decode_config(&mut input, limits)?;
    let boundary_kind = match input.u8()? {
        1 => GenerationBoundaryKind::RunStart,
        2 => GenerationBoundaryKind::Generation,
        _ => {
            return Err(CheckpointError::format(
                "BOUNDARY_KIND",
                "invalid state boundary tag",
            ))
        }
    };
    let generation = decode_generation(&mut input)?;
    let rng = decode_rng_bundle(&mut input, limits)?;
    let allocators = decode_allocators(&mut input)?;
    if state_encoding >= STATE_ENCODING_WITH_LEGACY_ORIGIN {
        identity.legacy_conversion = match input.u8()? {
            0 => None,
            1 => Some(
                crate::engine::legacy_origin::LegacyPopulationOrigin::decode(
                    &input.string_bounded(crate::engine::legacy_origin::MAX_LEGACY_ORIGIN_BYTES)?,
                )
                .map_err(|error| CheckpointError::format("LEGACY_ORIGIN", error))?,
            ),
            _ => {
                return Err(CheckpointError::format(
                    "LEGACY_ORIGIN",
                    "unsupported legacy origin tag",
                ))
            }
        };
    }
    if state_encoding == STATE_ENCODING_WITH_FOUNDER {
        let origin: crate::engine::founder::FounderOrigin =
            serde_json::from_str(&input.string_bounded(512)?)
                .map_err(|error| CheckpointError::format("FOUNDER_ORIGIN", error.to_string()))?;
        origin
            .validate()
            .map_err(|error| CheckpointError::format("FOUNDER_ORIGIN", error))?;
        identity.founder = Some(origin);
    }
    input.finish()?;
    Ok(DecodedStateParts {
        versions,
        identity,
        config,
        boundary_kind,
        generation,
        rng,
        allocators,
    })
}

/// Decode run/build identity fields.
pub(super) fn decode_identity(
    input: &mut BinaryReader<'_>,
) -> Result<RunIdentity, CheckpointError> {
    Ok(RunIdentity {
        legacy_conversion: None,
        founder: None,
        run_id: input.string()?,
        seed: input.u32()?,
        config_revision: input.u64()?,
        config_hash: input.string()?,
        source_revision: input.string()?,
        engine_build_id: input.string()?,
        source_sha256: input.string()?,
        target_triple: input.string()?,
        build_profile: input.string()?,
        build_class: input.string()?,
        rustc_version: input.string()?,
        build_contract_sha256: input.string()?,
        math_backend: input.string()?,
    })
}

/// Decode normalized configuration after preflighting its record count.
pub(super) fn decode_config(
    input: &mut BinaryReader<'_>,
    limits: &CheckpointLimits,
) -> Result<NormalizedEngineConfig, CheckpointError> {
    let version = input.u32()?;
    let count = u32_to_usize(input.u32()?, "setting count")?;
    if count > limits.max_setting_count {
        return Err(CheckpointError::format(
            "SETTING_LIMIT",
            "declared setting count exceeds limit",
        ));
    }
    let mut settings = Vec::new();
    settings.try_reserve_exact(count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve normalized settings")
    })?;
    for _ in 0..count {
        let path = input.string()?;
        let value = match input.u8()? {
            1 => NormalizedSettingValue::Bool(match input.u8()? {
                0 => false,
                1 => true,
                _ => {
                    return Err(CheckpointError::format(
                        "SETTING_VALUE",
                        "invalid boolean byte",
                    ))
                }
            }),
            2 => NormalizedSettingValue::Integer(input.i64()?),
            3 => NormalizedSettingValue::Float(input.f64()?),
            4 => NormalizedSettingValue::Text(input.string()?),
            _ => {
                return Err(CheckpointError::format(
                    "SETTING_VALUE",
                    "invalid setting value tag",
                ))
            }
        };
        settings.push(NormalizedSetting { path, value });
    }
    Ok(NormalizedEngineConfig {
        version,
        settings,
        settings_schema_sha256: input.string()?,
        graph_architecture_key: input.string()?,
        fixed_step_seconds: input.f64()?,
        requested_sim_speed: input.f64()?,
        world_radius: input.f64()?,
        population_count: u64_to_usize(input.u64()?, "population count")?,
        baseline_count: u64_to_usize(input.u64()?, "baseline count")?,
        max_world_snakes: u64_to_usize(input.u64()?, "max world snakes")?,
        max_non_population_brains: u64_to_usize(input.u64()?, "max non-population brains")?,
        max_body_points: u64_to_usize(input.u64()?, "max body points")?,
        max_pellets: u64_to_usize(input.u64()?, "max pellets")?,
        spatial_index_bytes: u64_to_usize(input.u64()?, "spatial index bytes")?,
        worker_scratch_bytes: u64_to_usize(input.u64()?, "worker scratch bytes")?,
        checkpoint_scratch_bytes: u64_to_usize(input.u64()?, "checkpoint scratch bytes")?,
        controller_input_hold_ms: input.u64()?,
        controller_disconnect_grace_ms: input.u64()?,
    })
}

/// Decode generation continuation scalars.
pub(super) fn decode_generation(
    input: &mut BinaryReader<'_>,
) -> Result<GenerationState, CheckpointError> {
    Ok(GenerationState {
        boundary_version: input.u32()?,
        generation: input.u64()?,
        completed_step: input.u64()?,
        population_epoch: input.u64()?,
        elapsed_seconds: input.f64()?,
        wall_accumulator_seconds: input.f64()?,
        best_fitness_ever: input.f64()?,
    })
}

/// Decode independent RNG continuations after count admission.
pub(super) fn decode_rng_bundle(
    input: &mut BinaryReader<'_>,
    limits: &CheckpointLimits,
) -> Result<RngStateBundle, CheckpointError> {
    let version = input.u32()?;
    if !matches!(
        version,
        crate::engine::state::RNG_BUNDLE_VERSION
            | crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION
            | crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION
    ) {
        return Err(CheckpointError::format(
            "RNG_VERSION",
            "unsupported RNG bundle encoding",
        ));
    }
    let world = decode_rng(input)?;
    let evolution = decode_rng(input)?;
    let external_controller = decode_rng(input)?;
    let count = u32_to_usize(input.u32()?, "baseline RNG count")?;
    if count > limits.max_baseline_rng_count {
        return Err(CheckpointError::format(
            "RNG_LIMIT",
            "declared baseline RNG count exceeds limit",
        ));
    }
    let mut baselines = Vec::new();
    baselines.try_reserve_exact(count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve baseline RNG states")
    })?;
    for _ in 0..count {
        baselines.push(BaselineRngState {
            slot: input.u32()?,
            state: decode_rng(input)?,
        });
    }
    let mut evolved_respawns = Vec::new();
    if version >= crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION {
        let count = u32_to_usize(input.u32()?, "evolved RNG count")?;
        if count > limits.max_population_count || count > 300 {
            return Err(CheckpointError::format(
                "RNG_LIMIT",
                "declared evolved RNG count exceeds limit",
            ));
        }
        let packed_length = count
            .checked_mul(8)
            .ok_or_else(|| CheckpointError::format("ARITHMETIC", "evolved RNG bytes overflow"))?;
        // Prove the complete packed record range exists before reserving storage.
        let packed = input.raw(packed_length)?;
        evolved_respawns.try_reserve_exact(count).map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve evolved RNG states")
        })?;
        for (slot, record) in packed.as_chunks::<8>().0.iter().enumerate() {
            let actual_slot = u32::from_le_bytes([record[0], record[1], record[2], record[3]]);
            let uniform_state = u32::from_le_bytes([record[4], record[5], record[6], record[7]]);
            if actual_slot as usize != slot || uniform_state == 0 {
                return Err(CheckpointError::format(
                    "RNG_STATE",
                    "evolved RNG slots must be dense and nonzero",
                ));
            }
            evolved_respawns.push(EvolvedRespawnRngState {
                slot: actual_slot,
                uniform_state,
            });
        }
    }
    let mut baseline_behaviors = Vec::new();
    if version == crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION {
        let count = u32_to_usize(input.u32()?, "baseline behavior count")?;
        if count > limits.max_baseline_rng_count || count > 120 {
            return Err(CheckpointError::format(
                "RNG_LIMIT",
                "declared behavior count exceeds limit",
            ));
        }
        if count != baselines.len() {
            return Err(CheckpointError::format(
                "RNG_STATE",
                "behavior count must match baseline slots",
            ));
        }
        let length = count.checked_mul(8).ok_or_else(|| {
            CheckpointError::format("ARITHMETIC", "baseline behavior bytes overflow")
        })?;
        let packed = input.raw(length)?; // Prove the whole record range before reserving.
        baseline_behaviors.try_reserve_exact(count).map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve behavior streams")
        })?;
        for (slot, record) in packed.as_chunks::<8>().0.iter().enumerate() {
            let actual_slot = u32::from_le_bytes([record[0], record[1], record[2], record[3]]);
            let uniform_state = u32::from_le_bytes([record[4], record[5], record[6], record[7]]);
            if actual_slot as usize != slot || uniform_state == 0 {
                return Err(CheckpointError::format(
                    "RNG_STATE",
                    "behavior slots must be dense and nonzero",
                ));
            }
            baseline_behaviors.push(crate::engine::rng::BaselineBehaviorRngState {
                slot: actual_slot,
                uniform_state,
            });
        }
    }
    Ok(RngStateBundle {
        baseline_behaviors,
        evolved_respawns,
        version,
        world,
        evolution,
        external_controller,
        baselines,
    })
}

/// Decode one lossless RNG continuation.
pub(super) fn decode_rng(
    input: &mut BinaryReader<'_>,
) -> Result<SerializedRngState, CheckpointError> {
    let algorithm = input.string()?;
    let version = input.u32()?;
    let state_hex = input.string()?;
    let gaussian_algorithm = input.string()?;
    let gaussian_version = input.u32()?;
    let gaussian_spare_valid = match input.u8()? {
        0 => false,
        1 => true,
        _ => {
            return Err(CheckpointError::format(
                "RNG_STATE",
                "invalid Gaussian-valid byte",
            ))
        }
    };
    let gaussian_spare_hex = match input.u8()? {
        0 => None,
        1 => Some(input.string()?),
        _ => {
            return Err(CheckpointError::format(
                "RNG_STATE",
                "invalid Gaussian-option byte",
            ))
        }
    };
    Ok(SerializedRngState {
        algorithm,
        version,
        state_hex,
        gaussian_algorithm,
        gaussian_version,
        gaussian_spare_valid,
        gaussian_spare_hex,
    })
}

/// Decode monotonic allocator continuations.
pub(super) fn decode_allocators(
    input: &mut BinaryReader<'_>,
) -> Result<AllocatorState, CheckpointError> {
    Ok(AllocatorState {
        version: input.u32()?,
        next_entity_id: input.u64()?,
        next_brain_id: input.u64()?,
        next_genome_id: input.u64()?,
        next_controller_lease_id: input.u64()?,
        next_frame_v1_id: input.u32()?,
        next_external_id: input.u64()?,
        next_baseline_id: input.u64()?,
        next_resurrected_id: input.u64()?,
    })
}
