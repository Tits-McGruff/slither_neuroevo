use super::*;

#[test]
fn rng_bundle_v3_roundtrips_behavior_words_and_preserves_v1_v2_bytes() {
    let (authority, _, _) = admitted_state(2, false);
    let limits = checkpoint_limits();
    let mut rng = authority.state().rng.clone();
    let mut legacy = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut legacy, &rng, &limits).unwrap();
    let v1 = legacy.bytes;
    rng.version = crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION;
    rng.evolved_respawns = EvolvedRespawnRngState::derive_generation(42, 1, 2).unwrap();
    rng.baselines = (0..3)
        .map(|slot| BaselineRngState {
            slot,
            state: StatefulRng::new(slot as f64 + 1.0).export_state(),
        })
        .collect();
    let mut writer = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut writer, &rng, &limits).unwrap();
    let v2 = writer.bytes;
    rng.version = crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
    rng.baseline_behaviors =
        crate::engine::rng::derive_baseline_v2_behaviors(42, 1, 1, 3, true).unwrap();
    let mut behavior = rng.baseline_behaviors[1].runtime().unwrap();
    for _ in 0..19 {
        behavior.next_f64();
    }
    rng.baseline_behaviors[1].uniform_state = behavior.uniform_state();
    let mut writer = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut writer, &rng, &limits).unwrap();
    let bytes = writer.bytes;
    let restored = decode_rng_bundle(&mut BinaryReader::new(&bytes, &limits), &limits).unwrap();
    assert_eq!(restored, rng);
    let mut next = restored.baseline_behaviors[1].runtime().unwrap();
    for _ in 0..128 {
        assert_eq!(behavior.next_f64().to_bits(), next.next_f64().to_bits());
    }
    rng.baseline_behaviors.clear();
    rng.version = crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION;
    let mut writer = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut writer, &rng, &limits).unwrap();
    assert_eq!(writer.bytes, v2);
    let mut writer = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut writer, &authority.state().rng, &limits).unwrap();
    assert_eq!(writer.bytes, v1);
}

#[test]
fn rng_bundle_v3_rejects_count_slot_zero_truncation_and_frozen_stream_elision() {
    let (authority, _, _) = admitted_state(2, false);
    let limits = checkpoint_limits();
    let mut rng = authority.state().rng.clone();
    rng.version = crate::engine::state::RNG_BUNDLE_WITH_BASELINE_V2_VERSION;
    rng.evolved_respawns = EvolvedRespawnRngState::derive_generation(7, 1, 2).unwrap();
    rng.baselines = (0..2)
        .map(|slot| BaselineRngState {
            slot,
            state: StatefulRng::new(slot as f64 + 1.0).export_state(),
        })
        .collect();
    rng.baseline_behaviors =
        crate::engine::rng::derive_baseline_v2_behaviors(7, 1, 1, 2, true).unwrap();
    let mut writer = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut writer, &rng, &limits).unwrap();
    let bytes = writer.bytes;
    let records = bytes.len() - 16;
    for (offset, value, code) in [
        (records - 4, 121u32, "RNG_LIMIT"),
        (records - 4, 1, "RNG_STATE"),
        (records, 1, "RNG_STATE"),
        (records + 4, 0, "RNG_STATE"),
    ] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(
            matches!(decode_rng_bundle(&mut BinaryReader::new(&bad,&limits),&limits),
            Err(CheckpointError::Format{code:actual,..})if actual==code)
        );
    }
    for cut in 1..=16 {
        assert!(decode_rng_bundle(
            &mut BinaryReader::new(&bytes[..bytes.len() - cut], &limits),
            &limits
        )
        .is_err());
    }
    for version in [
        crate::engine::state::RNG_BUNDLE_VERSION,
        crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION,
    ] {
        rng.version = version;
        assert!(encode_rng_bundle(
            &mut BinaryWriter::new(limits.max_state_bytes, &limits),
            &rng,
            &limits
        )
        .is_err());
    }
}

#[test]
fn rng_bundle_v2_roundtrips_dense_evolved_continuation_without_changing_v1_encoding() {
    let (authority, graph, policy) = admitted_state(2, false);
    let limits = checkpoint_limits();
    let legacy_bytes = encode_state(authority.state(), &limits).unwrap();
    let legacy = decode_state(&legacy_bytes, &limits).unwrap();
    assert!(legacy.rng.evolved_respawns.is_empty());
    assert_eq!(legacy.rng.version, crate::engine::state::RNG_BUNDLE_VERSION);
    let mut candidate = authority.state().clone();
    candidate.versions.rng_bundle = crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION;
    candidate.rng.version = candidate.versions.rng_bundle;
    candidate.rng.evolved_respawns = EvolvedRespawnRngState::derive_generation(
        candidate.identity.seed,
        candidate.generation.generation,
        candidate.population.len(),
    )
    .unwrap();
    let mut runtime = candidate.rng.evolved_respawns[0].runtime().unwrap();
    for _ in 0..17 {
        runtime.next_f64();
    }
    candidate.rng.evolved_respawns[0].uniform_state = runtime.uniform_state();
    let encoded = encode_state(&candidate, &limits).unwrap();
    assert_eq!(decode_state(&encoded, &limits).unwrap().rng, candidate.rng);
    assert_eq!(
        encode_state(authority.state(), &limits).unwrap(),
        legacy_bytes
    );
    let owned = AuthoritativeState::validate_and_own(candidate, graph, &policy).unwrap();
    let directory = TestDirectory::new("evolved-rng-v2");
    let (file, descriptor) = publish_fixture(
        &directory,
        &owned,
        &policy,
        "abababababababababababababababab",
    );
    let restored = restore_checkpoint(&file, &limits, &graph_limits(), &policy).unwrap();
    assert_eq!(restored.state.state().rng, owned.state().rng);
    assert_eq!(
        restored.content.logical_root_sha256,
        descriptor.logical_root_sha256
    );
}

#[test]
fn rng_bundle_v2_rejects_oversized_truncated_and_invalid_packed_records() {
    let (authority, _, _) = admitted_state(2, false);
    let limits = checkpoint_limits();
    let mut rng = authority.state().rng.clone();
    rng.version = crate::engine::state::RNG_BUNDLE_WITH_EVOLVED_VERSION;
    rng.evolved_respawns = EvolvedRespawnRngState::derive_generation(7, 1, 2).unwrap();
    let mut writer = BinaryWriter::new(limits.max_state_bytes, &limits);
    encode_rng_bundle(&mut writer, &rng, &limits).unwrap();
    let original = writer.bytes;
    let records_start = original.len() - 16;
    for (offset, replacement, expected_code) in [
        (records_start - 4, 301_u32, "RNG_LIMIT"),
        (records_start, 1, "RNG_STATE"),
        (records_start + 4, 0, "RNG_STATE"),
    ] {
        let mut bytes = original.clone();
        bytes[offset..offset + 4].copy_from_slice(&replacement.to_le_bytes());
        let error =
            decode_rng_bundle(&mut BinaryReader::new(&bytes, &limits), &limits).unwrap_err();
        assert!(matches!(error, CheckpointError::Format { code, .. } if code == expected_code));
    }
    assert!(decode_rng_bundle(
        &mut BinaryReader::new(&original[..original.len() - 1], &limits),
        &limits
    )
    .is_err());
    rng.version = crate::engine::state::RNG_BUNDLE_VERSION;
    assert!(encode_rng_bundle(
        &mut BinaryWriter::new(limits.max_state_bytes, &limits),
        &rng,
        &limits
    )
    .is_err());
}

/// Origin records survive binary state saves while old states remain readable.
#[test]
fn legacy_origin_state_encoding_preserves_facts_and_reads_previous_encoding() {
    let (authority, _graph, _policy) = admitted_state(2, false);
    let limits = checkpoint_limits();
    let mut old = encode_state(authority.state(), &limits).unwrap();
    assert_eq!(old.pop(), Some(0));
    old[8..12].copy_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    assert_eq!(
        decode_state(&old, &limits).unwrap().identity,
        authority.state().identity
    );

    let mut candidate = authority.state().clone();
    let mut origin = crate::engine::legacy_origin::LegacyPopulationOrigin::new(
        crate::engine::legacy_origin::LegacyPopulationSource::BrowserJson,
    );
    origin.source_seed = Some(1234567);
    origin.source_generation = Some("0000000000000025".into());
    candidate.identity.legacy_conversion = Some(origin);
    let encoded = encode_state(&candidate, &limits).unwrap();
    assert_eq!(
        decode_state(&encoded, &limits).unwrap().identity,
        candidate.identity
    );
    assert_ne!(encoded, encode_state(authority.state(), &limits).unwrap());
    assert!(decode_state(&encoded[..encoded.len() - 1], &limits).is_err());

    let mut oversized = encode_state(authority.state(), &limits).unwrap();
    *oversized.last_mut().unwrap() = 1;
    oversized.extend_from_slice(
        &((crate::engine::legacy_origin::MAX_LEGACY_ORIGIN_BYTES + 1) as u32).to_le_bytes(),
    );
    let error = decode_state(&oversized, &limits).err().unwrap();
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "STRING_LIMIT",
            ..
        }
    ));
}

/// The ordered root bytes must stay cross-language compatible with retained Stage 2.
#[test]
fn logical_root_matches_retained_typescript_fixture() {
    let roles = [
        LogicalRoleDigest {
            role: "checkpoint",
            logical_length: 30,
            logical_sha256: parse_digest(
                "7e5a42a9d9efa796dddccc0ac2d2cbe5ca0131d573353608cf2d29929d8b5bd7",
                "fixture checkpoint",
            )
            .unwrap(),
        },
        LogicalRoleDigest {
            role: "population-weights",
            logical_length: 64,
            logical_sha256: parse_digest(
                "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e",
                "fixture weights",
            )
            .unwrap(),
        },
    ];
    assert_eq!(
        encode_digest(compute_logical_root(&roles).unwrap()),
        "f715b31f2bbd45f0b8a9cc8f48062712bcebe880507e82332eaee0cc2bf7b481"
    );
}

/// A small recurrent checkpoint round-trips every graph/RNG/Float32 bit and is idempotent.
#[test]
fn small_recurrent_checkpoint_round_trips_bit_exactly_and_idempotently() {
    let directory = TestDirectory::new("small");
    let (state, _graph, policy) = admitted_state(3, false);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000001",
    );
    assert_eq!(descriptor.managed_root, "checkpoint-v3");
    assert_eq!(descriptor.protocol_version, 1);
    assert_eq!(descriptor.role_count_hex, "0000000000000005");
    assert_eq!(descriptor.recurrent_state_count_hex, "0000000000000006");
    assert_eq!(
        descriptor.relative_filename,
        format!("{}.checkpoint-v3", descriptor.logical_root_sha256)
    );
    let restored = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy)
        .expect("published checkpoint must strictly restore");
    assert_eq!(restored.state.state(), state.state());
    assert_eq!(restored.state.graph_spec(), state.graph_spec());
    for (left, right) in restored
        .state
        .state()
        .population
        .iter()
        .zip(&state.state().population)
    {
        assert_eq!(
            left.weights
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            right
                .weights
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
    }
    let second = publish_checkpoint(
        &directory.path,
        CheckpointOperationId::parse("00000000000000000000000000000002").unwrap(),
        10,
        state.checkpoint_boundary().unwrap(),
        &checkpoint_limits(),
        &graph_limits(),
        &policy,
    )
    .expect("strictly validated content-addressed file must publish idempotently");
    assert_eq!(second.relative_filename, descriptor.relative_filename);
    assert_eq!(
        second.operation_id.as_str(),
        "00000000000000000000000000000002"
    );
    assert_eq!(second.transition_epoch_hex, "000000000000000a");
}

/// Real evolved export failures preserve colliding files and release only
/// newly created scratch, including a failure inside numeric encoding.
#[test]
fn evolved_export_preserves_preexisting_scratch_and_ready_files() {
    use crate::engine::export_archive::{compose_export_archive, ExportInventoryDescriptor};

    let fixture = hall_of_fame_import_fixture();
    let operation = "e".repeat(32);
    let inventory_bytes = fs::read(
        fixture
            .source
            .path
            .join(format!(".{}.export-inventory-v1", "c".repeat(32))),
    )
    .unwrap();
    let inventory = ExportInventoryDescriptor {
        version: 1,
        relative_filename: format!(".{operation}.export-inventory-v1"),
        sha256: encode_digest(Sha256::digest(&inventory_bytes).into()),
        stored_byte_count_hex: encode_u64_hex(inventory_bytes.len() as u64),
        history_count_hex: encode_u64_hex(1),
        hall_of_fame_count_hex: encode_u64_hex(1),
    };
    fs::write(
        fixture.source.path.join(&inventory.relative_filename),
        inventory_bytes,
    )
    .unwrap();
    let snapshot = || {
        fs::read_dir(&fixture.source.path)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    for suffix in [
        "export-hof-weights.partial",
        "export-hof-weights.codec.partial",
        "slither-save.partial",
        "slither-save.ready",
    ] {
        let collision = fixture.source.path.join(format!(".{operation}.{suffix}"));
        fs::write(&collision, b"earlier task-owned evidence").unwrap();
        let before = snapshot();
        let export = || {
            compose_export_archive(
                &fixture.source.path,
                &operation,
                &fixture.checkpoint,
                &inventory,
                &checkpoint_limits(),
                &graph_limits(),
                &fixture.policy,
            )
        };
        assert!(export().is_err(), "{suffix} must reject before publication");
        assert_eq!(snapshot(), before, "{suffix} must preserve earlier files");
        fs::remove_file(collision).unwrap();
        let retried = export().unwrap();
        let ready = fixture.source.path.join(retried.relative_filename);
        assert_eq!(
            fs::read(&ready).unwrap(),
            fs::read(&fixture.archive_path).unwrap()
        );
        fs::remove_file(ready).unwrap();
    }
}

/// A 55-slot multi-block population selects compression only when measured smaller.
#[test]
fn representative_checkpoint_round_trips_multi_block_weights_bit_exactly() {
    let directory = TestDirectory::new("representative");
    let (state, _graph, policy) = admitted_state(55, true);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000003",
    );
    assert_eq!(
        descriptor.weights_encoding,
        NumericEncoding::F32LeShuffle4ZstdV1
    );
    assert_eq!(
        descriptor.recurrent_state_encoding,
        NumericEncoding::RawF32LeV1
    );
    let restored = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy)
        .expect("representative checkpoint must restore");
    assert_eq!(restored.state.state(), state.state());
    assert_eq!(
        restored.state.graph().layout_digest_sha256,
        state.graph().layout_digest_sha256
    );
}

/// A small feed-forward population round-trips independently from recurrent coverage.
#[test]
fn small_feed_forward_checkpoint_round_trips_bit_exactly() {
    let directory = TestDirectory::new("small-feed-forward");
    let (state, _graph, policy) = admitted_state(2, true);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000011",
    );
    assert_eq!(descriptor.recurrent_state_count_hex, "0000000000000000");
    let restored = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy)
        .expect("small feed-forward checkpoint must restore");
    assert_eq!(restored.state.state(), state.state());
}

#[test]
fn founder_origin_state_encoding_is_bounded_versioned_and_preserves_existing_states() {
    let (authority, _graph, _policy) = admitted_state(2, false);
    let limits = checkpoint_limits();
    let mut candidate = authority.state().clone();
    let original = encode_state(&candidate, &limits).unwrap();
    assert_eq!(
        u32::from_le_bytes(original[8..12].try_into().unwrap()),
        STATE_ENCODING_WITH_LEGACY_ORIGIN
    );
    candidate.identity.founder = Some(crate::engine::founder::FounderOrigin {
        schema_version: 1,
        mode: crate::engine::founder::FounderMode::ExactFounderMutatedDescendantsV1,
        artifact_sha256: "ab".repeat(32),
        weights_sha256: "cd".repeat(32),
    });
    let encoded = encode_state(&candidate, &limits).unwrap();
    assert_eq!(
        u32::from_le_bytes(encoded[8..12].try_into().unwrap()),
        STATE_ENCODING_WITH_FOUNDER
    );
    assert_eq!(
        decode_state(&encoded, &limits).unwrap().identity,
        candidate.identity
    );
    assert!(decode_state(&encoded[..encoded.len() - 1], &limits).is_err());
    let mut invalid = encoded.clone();
    invalid[8..12].copy_from_slice(&6_u32.to_le_bytes());
    assert!(decode_state(&invalid, &limits).is_err());
    candidate.identity.founder.as_mut().unwrap().artifact_sha256 = "x".repeat(64);
    assert!(encode_state(&candidate, &limits).is_err());
    assert_eq!(encode_state(authority.state(), &limits).unwrap(), original);
}
