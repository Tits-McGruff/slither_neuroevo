use super::*;

/// Payload corruption is diagnosed before an invalid candidate can become authoritative.
#[test]
fn corrupted_numeric_payload_is_rejected() {
    let directory = TestDirectory::new("corrupt");
    let (state, _graph, policy) = admitted_state(4, false);
    let (path, _descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000004",
    );
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let length = file.metadata().unwrap().len();
    let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
    let target = &entries[3];
    file.seek(SeekFrom::Start(target.data_offset)).unwrap();
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x80;
    file.seek(SeekFrom::Start(target.data_offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy).is_err());
}

/// Malformed SFZ1 envelope data is rejected before invoking Zstandard decode.
#[test]
fn malformed_sfz1_envelope_is_rejected() {
    let directory = TestDirectory::new("sfz1-malformed");
    let (state, _graph, policy) = admitted_state(55, true);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000012",
    );
    assert_eq!(
        descriptor.weights_encoding,
        NumericEncoding::F32LeShuffle4ZstdV1
    );
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let length = file.metadata().unwrap().len();
    let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
    file.seek(SeekFrom::Start(entries[3].data_offset)).unwrap();
    file.write_all(b"BAD!").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(matches!(
        restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy),
        Err(CheckpointError::Format {
            code: "SHUFFLED_MAGIC",
            ..
        })
    ));
}

/// A declared decoder window over the one-MiB contract fails during preflight.
#[test]
fn oversized_zstandard_window_is_rejected_before_numeric_allocation() {
    let mut frame_header = vec![0x28, 0xb5, 0x2f, 0xfd, 0x80, 0x58];
    frame_header.extend_from_slice(&(SHUFFLED_BLOCK_BYTES as u32).to_le_bytes());
    let error = validate_zstd_frame_header(&frame_header, SHUFFLED_BLOCK_BYTES)
        .expect_err("two-MiB decoder window must fail the one-MiB preflight bound");
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "ZSTD_WINDOW",
            ..
        }
    ));
}

/// The selected raw representation remains valid when the discarded SFZ1 candidate expands.
#[test]
fn raw_at_selected_entry_limit_succeeds_when_sfz1_candidate_expands() {
    let directory = TestDirectory::new("raw-limit");
    let mut word = 0x1234_5678u32;
    let mut values = Vec::new();
    for _ in 0..16 {
        word ^= word << 13;
        word ^= word >> 17;
        word ^= word << 5;
        if word & 0x7f80_0000 == 0x7f80_0000 {
            word ^= 0x0080_0000;
        }
        values.push(f32::from_bits(word));
    }
    let source = FloatSource::new(vec![values.as_slice()], values.len(), "raw-limit").unwrap();
    let mut limits = checkpoint_limits();
    limits.max_numeric_stored_bytes = source.raw_bytes().unwrap();
    limits.max_numeric_candidate_bytes = 4096;
    let mut artifacts = TemporaryArtifacts::new();
    let candidate = select_numeric_candidate(
        &directory.path,
        "00000000000000000000000000000006",
        "raw-limit",
        &source,
        &limits,
        &mut artifacts,
    )
    .expect("raw at the selected limit must not be rejected by an expanding candidate");
    assert_eq!(candidate.encoding, NumericEncoding::RawF32LeV1);
    assert_eq!(candidate.stored_bytes, limits.max_numeric_stored_bytes);
    assert!(candidate.encoded_blocks >= 1);
    assert_eq!(candidate.scratch_capacity_growths, 0);
}

/// Multi-block encoding reuses its three bounded scratch allocations without capacity growth.
#[test]
fn multi_block_codec_reuses_bounded_scratch() {
    let directory = TestDirectory::new("scratch");
    let count = SHUFFLED_BLOCK_BYTES / 4 * 3 + 17;
    let values = vec![f32::from_bits(0x3f12_3456); count];
    let source = FloatSource::new(vec![values.as_slice()], count, "scratch").unwrap();
    let mut artifacts = TemporaryArtifacts::new();
    let candidate = select_numeric_candidate(
        &directory.path,
        "00000000000000000000000000000007",
        "scratch",
        &source,
        &checkpoint_limits(),
        &mut artifacts,
    )
    .unwrap();
    assert_eq!(candidate.encoded_blocks, 4);
    assert_eq!(candidate.scratch_capacity_growths, 0);
    assert_eq!(candidate.encoding, NumericEncoding::F32LeShuffle4ZstdV1);
}

/// Variable-sized reads preserve exact packed bits across owner and partial-float boundaries.
#[test]
fn float_byte_reader_bulk_and_partial_reads_preserve_exact_bytes() {
    let patterns: [u32; 12] = [
        0,
        0x8000_0000,
        1,
        0x8000_0001,
        0x3f12_3456,
        0x7f7f_ffff,
        0xff7f_ffff,
        0x7fc1_2345,
        0x7f80_0000,
        0xff80_0000,
        0x7fa1_2345,
        0xffff_ffff,
    ];
    for count in [37, SHUFFLED_BLOCK_BYTES / 4 + 17] {
        let bits = (0..count)
            .map(|index| patterns[index % patterns.len()])
            .collect::<Vec<_>>();
        let values = bits
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect::<Vec<_>>();
        let expected = bits
            .iter()
            .flat_map(|bits| bits.to_le_bytes())
            .collect::<Vec<_>>();
        let source = FloatSource::new(
            vec![
                &[],
                &values[..3],
                &[],
                &values[3..count - 2],
                &values[count - 2..],
                &[],
            ],
            count,
            "packed-reader-test",
        )
        .unwrap();
        for sizes in [
            vec![1, 2, 3, 4, 5, 7, 8, 13],
            vec![3, SHUFFLED_BLOCK_BYTES + 1, 1, 4097, 2],
        ] {
            let mut reader = FloatByteReader::new(&source);
            let mut actual = Vec::new();
            let mut reads = 0usize;
            loop {
                assert_eq!(reader.read(&mut []).unwrap(), 0);
                let capacity = sizes[reads % sizes.len()];
                let mut buffer = vec![0xa5; capacity + 2];
                let written = reader.read(&mut buffer[1..capacity + 1]).unwrap();
                assert!(written <= capacity);
                assert_eq!(buffer[0], 0xa5);
                assert!(buffer[written + 1..].iter().all(|byte| *byte == 0xa5));
                actual.extend_from_slice(&buffer[1..written + 1]);
                if written == 0 {
                    break;
                }
                reads += 1;
            }
            assert_eq!(actual, expected);
            let mut tail = [0xa5; 17];
            assert_eq!(reader.read(&mut tail).unwrap(), 0);
            assert_eq!(tail, [0xa5; 17]);
        }
    }
}

/// All-empty owners and an absent population return EOF without touching the caller's buffer.
#[test]
fn float_byte_reader_empty_sources_leave_output_untouched() {
    for slices in [Vec::new(), vec![&[][..], &[][..], &[][..]]] {
        let source = FloatSource::new(slices, 0, "empty-packed-reader-test").unwrap();
        let mut reader = FloatByteReader::new(&source);
        let mut output = [0xa5; 11];
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(reader.read(&mut output).unwrap(), 0);
        assert_eq!(reader.read(&mut output[..3]).unwrap(), 0);
        assert_eq!(output, [0xa5; 11]);
    }
}

/// Packed unshuffle preserves known byte order and never grows its admitted scratch.
#[test]
fn unshuffle_reuses_bounded_scratch_and_checks_shape() {
    let shuffled = [1, 5, 9, 2, 6, 10, 3, 7, 11, 4, 8, 12];
    let mut raw = Vec::with_capacity(SHUFFLED_BLOCK_BYTES);
    let pointer = raw.as_ptr();
    let capacity = raw.capacity();
    unshuffle_f32_bytes_into(&shuffled, &mut raw).unwrap();
    assert_eq!(raw, (1u8..=12).collect::<Vec<_>>());
    let full = vec![0xa5; SHUFFLED_BLOCK_BYTES];
    unshuffle_f32_bytes_into(&full, &mut raw).unwrap();
    assert_eq!(raw, full);
    unshuffle_f32_bytes_into(&shuffled, &mut raw).unwrap();
    assert_eq!(raw.as_ptr(), pointer);
    assert_eq!(raw.capacity(), capacity);
    assert!(matches!(
        unshuffle_f32_bytes_into(&shuffled[..3], &mut raw),
        Err(CheckpointError::Format {
            code: "SHUFFLED_BLOCK",
            ..
        })
    ));
    assert!(matches!(
        unshuffle_f32_bytes_into(&vec![0; SHUFFLED_BLOCK_BYTES + 4], &mut raw),
        Err(CheckpointError::Format {
            code: "SHUFFLED_BLOCK",
            ..
        })
    ));
    let mut too_small = Vec::with_capacity(4);
    assert!(matches!(
        unshuffle_f32_bytes_into(&shuffled, &mut too_small),
        Err(CheckpointError::Format {
            code: "SHUFFLED_SCRATCH",
            ..
        })
    ));
    assert_eq!(too_small.capacity(), 4);
}

/// Packed appends cross owner boundaries without reallocating or accepting partial bad chunks.
#[test]
fn segmented_packed_appends_preserve_bits_and_reserved_owners() {
    let patterns = [0, 0x8000_0000, 1, 0x7fa1_2345, 0xff80_0000, 0xffff_ffff];
    let bytes = patterns
        .iter()
        .flat_map(|bits: &u32| bits.to_le_bytes())
        .collect::<Vec<_>>();
    let mut builder = SegmentedF32Builder::new(3, 2, patterns.len()).unwrap();
    let allocations = builder
        .segments
        .iter()
        .map(|segment| (segment.as_ptr(), segment.capacity()))
        .collect::<Vec<_>>();
    builder.append_packed(&bytes[..4]).unwrap();
    assert!(matches!(
        builder.append_packed(&bytes[4..7]),
        Err(CheckpointError::Format {
            code: "NUMERIC_LENGTH",
            ..
        })
    ));
    assert!(matches!(
        builder.append_packed(&bytes),
        Err(CheckpointError::Format {
            code: "NUMERIC_COUNT",
            ..
        })
    ));
    assert_eq!(builder.total_written, 1);
    assert_eq!(
        builder.segments.iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 0, 0]
    );
    builder.append_packed(&bytes[4..20]).unwrap();
    builder.append_packed(&bytes[20..]).unwrap();
    builder.append_packed(&[]).unwrap();
    assert!(matches!(
        builder.append_packed(&bytes[..4]),
        Err(CheckpointError::Format {
            code: "NUMERIC_COUNT",
            ..
        })
    ));
    assert_eq!(
        builder
            .segments
            .iter()
            .map(|segment| (segment.as_ptr(), segment.capacity()))
            .collect::<Vec<_>>(),
        allocations
    );
    let decoded = builder.finish().unwrap();
    assert_eq!(
        decoded
            .iter()
            .map(|segment| segment.len())
            .collect::<Vec<_>>(),
        [2, 2, 2]
    );
    assert_eq!(
        decoded
            .iter()
            .flat_map(|segment| segment.iter().map(|value| value.to_bits()))
            .collect::<Vec<_>>(),
        patterns
    );
}

/// Empty owners remain empty, incomplete output fails, and overflowing shapes never allocate.
#[test]
fn segmented_packed_appends_check_empty_incomplete_and_overflow_shapes() {
    let mut empty = SegmentedF32Builder::new(3, 0, 0).unwrap();
    empty.append_packed(&[]).unwrap();
    assert!(matches!(
        empty.append_packed(&[0; 4]),
        Err(CheckpointError::Format {
            code: "NUMERIC_COUNT",
            ..
        })
    ));
    assert!(empty
        .finish()
        .unwrap()
        .iter()
        .all(|segment| segment.is_empty()));
    let mut incomplete = SegmentedF32Builder::new(2, 3, 6).unwrap();
    incomplete.append_packed(&[0; 20]).unwrap();
    assert!(matches!(
        incomplete.finish(),
        Err(CheckpointError::Format {
            code: "NUMERIC_COUNT",
            ..
        })
    ));
    assert!(matches!(
        SegmentedF32Builder::new(usize::MAX, 2, 0),
        Err(CheckpointError::Format {
            code: "NUMERIC_SEGMENTS",
            ..
        })
    ));
}

/// Bulk hashing preserves bits across block/owner boundaries and rejects the wrong digest.
#[test]
fn compressed_segmented_decode_preserves_bits_and_requires_digest() {
    let directory = TestDirectory::new("segmented-block-hash");
    let segment_length = SHUFFLED_BLOCK_BYTES / 4 + 17;
    let count = 3 * segment_length;
    let patterns = [
        0,
        0x8000_0000,
        1,
        0x8000_0001,
        0x3f12_3456,
        0x7f7f_ffff,
        0xff7f_ffff,
        0x7fc1_2345,
        0x7f80_0000,
        0xff80_0000,
        0x7fa1_2345,
        0xffff_ffff,
    ];
    let values = (0..count)
        .map(|index| f32::from_bits(patterns[index % patterns.len()]))
        .collect::<Vec<_>>();
    let source = FloatSource::new(vec![values.as_slice()], count, "segmented-block-hash").unwrap();
    let mut artifacts = TemporaryArtifacts::new();
    let candidate = select_numeric_candidate(
        &directory.path,
        "00000000000000000000000000000010",
        "segmented-block-hash",
        &source,
        &checkpoint_limits(),
        &mut artifacts,
    )
    .unwrap();
    assert_eq!(candidate.encoding, NumericEncoding::F32LeShuffle4ZstdV1);
    assert_eq!(candidate.encoded_blocks, 4);
    let entry = ScannedEntry {
        name: WEIGHTS_ZSTD_PATH.to_owned(),
        header_offset: 0,
        data_offset: 0,
        size: candidate.stored_bytes,
    };
    let mut file = File::open(candidate.compressed_path.as_ref().unwrap()).unwrap();
    let decoded = decode_numeric_role(
        &mut file,
        &entry,
        candidate.encoding,
        count,
        3,
        segment_length,
        candidate.logical_sha256,
    )
    .unwrap();
    assert_eq!(decoded.len(), 3);
    for segment in &decoded {
        assert_eq!(segment.len(), segment_length);
    }
    for (actual, expected) in decoded
        .iter()
        .flat_map(|segment| segment.iter())
        .zip(&values)
    {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    let error = decode_numeric_role(
        &mut file,
        &entry,
        candidate.encoding,
        count,
        3,
        segment_length,
        [0; 32],
    )
    .expect_err("a completed decode still requires its exact logical digest");
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "LOGICAL_ROLE_SHA256",
            ..
        }
    ));
}

/// An encoded stream decodes directly to identical logical bytes.
#[test]
fn adaptive_numeric_stream_round_trips_compressed_bytes() {
    let directory = TestDirectory::new("adaptive-file");
    let values = vec![f32::from_bits(0x3f12_3456); 4096];
    let raw_bytes = values
        .iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect::<Vec<_>>();
    let raw_path = directory.path.join("source.f32le");
    fs::write(&raw_path, &raw_bytes).unwrap();
    let candidate = select_adaptive_numeric_file(
        &raw_path,
        &directory.path,
        "00000000000000000000000000000009",
        "adaptive-file",
        values.len() as u64,
        &checkpoint_limits(),
    )
    .unwrap();
    assert_eq!(candidate.encoding, NumericEncoding::F32LeShuffle4ZstdV1);
    let compressed_path = candidate.compressed_path.as_ref().unwrap();
    let mut compressed = File::open(compressed_path).unwrap();
    let mut decoded = Vec::new();
    decode_adaptive_numeric_reader(
        &mut compressed,
        candidate.encoding,
        candidate.stored_bytes,
        candidate.float_count,
        candidate.logical_sha256,
        &mut decoded,
    )
    .unwrap();
    assert_eq!(decoded, raw_bytes);
}

/// Archive length admission happens before an archive partial is created.
#[test]
fn undersized_archive_limit_leaves_no_partial_or_candidate() {
    let directory = TestDirectory::new("archive-limit");
    let (state, _graph, policy) = admitted_state(3, false);
    let mut limits = checkpoint_limits();
    limits.max_archive_bytes = 1024;
    let error = publish_checkpoint(
        &directory.path,
        CheckpointOperationId::parse("00000000000000000000000000000008").unwrap(),
        1,
        state.checkpoint_boundary().unwrap(),
        &limits,
        &graph_limits(),
        &policy,
    )
    .expect_err("undersized archive limit must reject before publication");
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "ARCHIVE_LIMIT",
            ..
        }
    ));
    assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
}

/// Shared state admission rejects an oversized candidate before touching corrupt numeric bytes.
#[test]
fn memory_ceiling_is_enforced_before_numeric_decode_allocation() {
    let directory = TestDirectory::new("memory-preflight");
    let (state, _graph, policy) = admitted_state(8, false);
    let (path, _) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000009",
    );
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let length = file.metadata().unwrap().len();
    let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
    file.seek(SeekFrom::Start(entries[3].data_offset)).unwrap();
    file.write_all(b"BROKEN").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let mut constrained = admission_policy(policy.expected_settings_schema_sha256.clone());
    constrained.memory_ceiling_bytes = 1;
    let error = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &constrained)
        .expect_err("memory ceiling must reject before corrupt numeric decode");
    assert!(matches!(
        error,
        CheckpointError::State(StateError::MemoryCeilingExceeded { .. })
    ));
}

/// Manifest-declared population workspace is charged before index bytes are decoded.
#[test]
fn population_workspace_ceiling_is_enforced_before_index_decode() {
    let directory = TestDirectory::new("population-predecode");
    let (state, _graph, policy) = admitted_state(480, true);
    let (path, _) = publish_fixture(
        &directory,
        &state,
        &policy,
        "0000000000000000000000000000001a",
    );
    let mut file = File::open(&path).unwrap();
    let length = file.metadata().unwrap().len();
    let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes[entries[2].data_offset as usize] ^= 0xff;
    fs::write(&path, bytes).unwrap();

    let minimum = checkpoint_workspace_bound(&checkpoint_limits(), 0).unwrap();
    let declared = checkpoint_workspace_bound(&checkpoint_limits(), 480).unwrap();
    assert!(declared > minimum);
    let mut constrained = admission_policy(policy.expected_settings_schema_sha256.clone());
    constrained.memory_ceiling_bytes = minimum + (declared - minimum) / 2;
    let error = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &constrained)
        .expect_err("population workspace must fail before the corrupt index is read");
    assert!(
        matches!(
            error,
            CheckpointError::State(StateError::MemoryCeilingExceeded { .. })
        ),
        "unexpected predecode error: {error:?}"
    );
}

/// Codec-derived working memory replaces an underdeclared configuration reservation.
#[test]
fn publication_charges_actual_checkpoint_workspace_against_memory_ceiling() {
    let directory = TestDirectory::new("workspace-accounting");
    let (state, _graph, policy) = admitted_state(8, false);
    let boundary = state.checkpoint_boundary().unwrap();
    let workspace = checkpoint_workspace_bound(&checkpoint_limits(), 8).unwrap();
    let declared = boundary.state().config.checkpoint_scratch_bytes;
    assert!(workspace > declared);
    let actual_peak = boundary
        .memory_estimate()
        .total_bytes
        .checked_sub(declared)
        .and_then(|value| value.checked_add(workspace))
        .unwrap();
    let mut constrained = admission_policy(policy.expected_settings_schema_sha256.clone());
    constrained.memory_ceiling_bytes = actual_peak - 1;
    let error = publish_checkpoint(
        &directory.path,
        CheckpointOperationId::parse("00000000000000000000000000000019").unwrap(),
        1,
        boundary,
        &checkpoint_limits(),
        &graph_limits(),
        &constrained,
    )
    .expect_err("actual checkpoint workspace must be charged before encoding");
    assert!(matches!(
        error,
        CheckpointError::State(StateError::MemoryCeilingExceeded { .. })
    ));
    assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
}
