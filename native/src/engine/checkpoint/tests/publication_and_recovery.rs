use super::*;

/// Legacy conversion must honor the same reduced overlap budget as native archives.
#[test]
fn legacy_json_import_uses_remaining_admission_before_decode_or_publication() {
    let target = TestDirectory::new("legacy-overlap-admission");
    let upload = target.path.join("population.json");
    let bytes = br#"{"genomes":[{"archKey":"legacy-default-graph","weights":[0]}]}"#;
    fs::write(&upload, bytes).unwrap();
    let mut policy = admission_policy(String::new());
    policy.memory_ceiling_bytes = 64 * 1024;
    let error = crate::engine::export_archive::prepare_import_archive(
        &upload,
        &target.path,
        &target.path,
        &"d".repeat(32),
        "legacy-overlap",
        42,
        &checkpoint_limits(),
        &graph_limits(),
        &policy,
        4 * 1024 * 1024 * 1024,
    )
    .err()
    .unwrap();
    assert!(
        error.to_string().contains("LEGACY_IMPORT_RESOURCE"),
        "{error}"
    );
    assert_eq!(fs::read_dir(&target.path).unwrap().count(), 1);
    assert_eq!(fs::read(&upload).unwrap(), bytes);
}

/// A valid evolved save reaches HOF extraction before build rejection or panic.
/// Neither failure may publish elite objects or delete a pre-existing shared elite.
#[test]
fn rejected_import_with_hall_of_fame_cleans_only_its_private_stage() {
    use crate::engine::export_archive::{prepare_import_archive, validate_import_archive};
    use crate::engine::task_panic_fixture::{PanicInjection, PanicPoint};

    let HallOfFameImportFixture {
        source,
        policy,
        elite,
        archive_path,
        ..
    } = hall_of_fame_import_fixture();
    let archive_before = fs::read(&archive_path).unwrap();
    let elite_before = fs::read(source.path.join(&elite.relative_filename)).unwrap();
    for existing_elite in [false, true] {
        let target = TestDirectory::new("import-hof-rejected");
        if existing_elite {
            fs::write(target.path.join(&elite.relative_filename), &elite_before).unwrap();
        }
        let mut foreign = admission_policy(policy.expected_settings_schema_sha256.clone());
        foreign.expected_engine_build_id = "incompatible-engine-build".into();
        let error = match prepare_import_archive(
            &archive_path,
            &target.path,
            &target.path,
            &"d".repeat(32),
            "unused-exact-import",
            0,
            &checkpoint_limits(),
            &graph_limits(),
            &foreign,
            policy.memory_ceiling_bytes,
        ) {
            Err(error) => error,
            Ok(_) => panic!("foreign build must remain rejected"),
        };
        assert!(error.to_string().contains("build identity is not admitted"));
        let injection = PanicInjection::arm(PanicPoint::ImportExtracted);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prepare_import_archive(
                &archive_path,
                &target.path,
                &target.path,
                &"e".repeat(32),
                "unused-exact-import",
                0,
                &checkpoint_limits(),
                &graph_limits(),
                &policy,
                policy.memory_ceiling_bytes,
            )
        }));
        assert!(injection.reached() && panicked.is_err());
        drop(injection);
        let names: Vec<_> = fs::read_dir(&target.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), usize::from(existing_elite));
        if existing_elite {
            assert_eq!(
                fs::read(target.path.join(&elite.relative_filename)).unwrap(),
                elite_before
            );
        }
        assert_eq!(fs::read(&archive_path).unwrap(), archive_before);
        assert_eq!(
            validate_import_archive(
                &archive_path,
                &target.path,
                &"f".repeat(32),
                &checkpoint_limits(),
                &graph_limits(),
                &policy,
            )
            .unwrap()
            .hall_of_fame_count_hex,
            encode_u64_hex(1)
        );
        assert_eq!(
            fs::read_dir(&target.path).unwrap().count(),
            usize::from(existing_elite)
        );
    }
}

/// Late error/unwind preserves shared finals, removes private scratch, and permits exact retry.
/// New immutable finals stay available for the reference-aware worker's safe-boundary scan.
#[test]
fn partial_import_publication_preserves_shared_files_and_retries_exactly() {
    use crate::engine::export_archive::prepare_import_archive;
    use crate::engine::task_panic_fixture::{
        PanicInjection, PanicPoint, PublicationIoErrorInjection,
    };

    let fixture = hall_of_fame_import_fixture();
    let archive_before = fs::read(&fixture.archive_path).unwrap();
    let checkpoint_before = fs::read(
        fixture
            .source
            .path
            .join(&fixture.checkpoint.relative_filename),
    )
    .unwrap();
    let elite_before =
        fs::read(fixture.source.path.join(&fixture.elite.relative_filename)).unwrap();
    for shared in [
        None,
        Some(&fixture.checkpoint.relative_filename),
        Some(&fixture.elite.relative_filename),
    ] {
        for panic in [false, true] {
            let target = TestDirectory::new("import-partial-publication");
            let shared_bytes = shared.map(|name| {
                let bytes = fs::read(fixture.source.path.join(name)).unwrap();
                fs::write(target.path.join(name), &bytes).unwrap();
                bytes
            });
            fs::write(
                target.path.join("owner-notes.txt"),
                b"preserve unknown files",
            )
            .unwrap();
            let prepare = |operation: &str| {
                prepare_import_archive(
                    &fixture.archive_path,
                    &target.path,
                    &target.path,
                    operation,
                    "unused-exact-import",
                    0,
                    &checkpoint_limits(),
                    &graph_limits(),
                    &fixture.policy,
                    fixture.policy.memory_ceiling_bytes,
                )
            };
            if panic {
                let injection = PanicInjection::arm(PanicPoint::ImportFilePublished);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    prepare(&"d".repeat(32))
                }));
                assert!(injection.reached() && result.is_err());
            } else {
                let injection = PublicationIoErrorInjection::arm();
                let error = prepare(&"d".repeat(32)).err().unwrap();
                assert!(injection.reached());
                assert!(error
                    .to_string()
                    .contains("injected import publication I/O error"));
            }
            let names: Vec<_> = fs::read_dir(&target.path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            assert_eq!(names.len(), 2 + usize::from(shared.is_some()));
            assert!(names.iter().all(|name| name == "owner-notes.txt"
                || name == &fixture.checkpoint.relative_filename
                || name == &fixture.elite.relative_filename));
            if let Some(name) = shared {
                assert_eq!(
                    fs::read(target.path.join(name)).unwrap(),
                    shared_bytes.unwrap()
                );
            }
            assert_eq!(
                fs::read(target.path.join("owner-notes.txt")).unwrap(),
                b"preserve unknown files"
            );
            assert_eq!(fs::read(&fixture.archive_path).unwrap(), archive_before);

            let retry =
                prepare(&"e".repeat(32)).expect("real publication retry must reuse valid finals");
            assert_eq!(retry.facts.generation_hex, encode_u64_hex(2));
            assert_eq!(retry.facts.hall_of_fame_count_hex, encode_u64_hex(1));
            assert_eq!(
                retry.descriptor.logical_root_sha256,
                fixture.checkpoint.logical_root_sha256
            );
            assert_eq!(
                fs::read(target.path.join(&fixture.checkpoint.relative_filename)).unwrap(),
                checkpoint_before
            );
            assert_eq!(
                fs::read(target.path.join(&fixture.elite.relative_filename)).unwrap(),
                elite_before
            );
            assert_eq!(fs::read_dir(&target.path).unwrap().count(), 4);
            assert!(target
                .path
                .join(&retry.inventory.relative_filename)
                .is_file());
            let restored = restore_checkpoint(
                &target.path.join(&retry.descriptor.relative_filename),
                &checkpoint_limits(),
                &graph_limits(),
                &fixture.policy,
            )
            .unwrap();
            let original = restore_checkpoint(
                &fixture
                    .source
                    .path
                    .join(&fixture.checkpoint.relative_filename),
                &checkpoint_limits(),
                &graph_limits(),
                &fixture.policy,
            )
            .unwrap();
            assert_eq!(restored.state.state(), original.state.state());
        }
    }
}

/// Winner weights are deduplicated by decoded bits and corrupt reuse is rejected.
#[test]
fn hall_of_fame_weights_publish_once_and_validate_before_reuse() {
    let directory = TestDirectory::new("hall-of-fame-weights");
    let weights = [1.25_f32, -0.0, f32::from_bits(0x7fc0_0001), -8.5];
    let first = publish_hall_of_fame_weights(
        &directory.path,
        &CheckpointOperationId::parse("00000000000000000000000000000071").unwrap(),
        &weights,
        &checkpoint_limits(),
    )
    .expect("first winner object must publish");
    let second = publish_hall_of_fame_weights(
        &directory.path,
        &CheckpointOperationId::parse("00000000000000000000000000000072").unwrap(),
        &weights,
        &checkpoint_limits(),
    )
    .expect("equal decoded weights must reuse the immutable object");
    assert_eq!(first, second);
    assert_eq!(first.decoded_byte_count_hex, "0000000000000010");
    assert_eq!(first.weight_count_hex, "0000000000000004");
    assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 1);

    let empty = publish_hall_of_fame_weights(
        &directory.path,
        &CheckpointOperationId::parse("00000000000000000000000000000074").unwrap(),
        &[],
        &checkpoint_limits(),
    )
    .expect("a valid parameterless genome must publish an empty raw object");
    assert_eq!(empty.logical_sha256, encode_digest(sha256(&[])));
    assert_eq!(empty.stored_byte_count_hex, "0000000000000000");
    assert_eq!(empty.weight_count_hex, "0000000000000000");
    assert_eq!(
        fs::metadata(directory.path.join(empty.relative_filename))
            .unwrap()
            .len(),
        0
    );

    let path = directory.path.join(&first.relative_filename);
    let mut corrupt = fs::read(&path).unwrap();
    corrupt[0] ^= 0x80;
    fs::write(&path, corrupt).unwrap();
    assert!(publish_hall_of_fame_weights(
        &directory.path,
        &CheckpointOperationId::parse("00000000000000000000000000000073").unwrap(),
        &weights,
        &checkpoint_limits(),
    )
    .is_err());
}

/// An unusable zero transition epoch is rejected before any managed artifact exists.
#[test]
fn zero_transition_epoch_is_rejected_before_publication() {
    let directory = TestDirectory::new("zero-transition-epoch");
    let (state, _graph, policy) = admitted_state(2, false);
    let error = publish_checkpoint(
        &directory.path,
        CheckpointOperationId::parse("00000000000000000000000000000018").unwrap(),
        0,
        state.checkpoint_boundary().unwrap(),
        &checkpoint_limits(),
        &graph_limits(),
        &policy,
    )
    .expect_err("zero transition epoch must fail before publication");
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "TRANSITION_EPOCH",
            ..
        }
    ));
    assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
}

#[test]
fn committed_restore_binds_all_content_to_the_selected_metadata() {
    let directory = TestDirectory::new("committed-restore");
    let (state, _graph, policy) = admitted_state(2, false);
    let (_path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000027",
    );
    let restore = |expected: &CheckpointDescriptor| {
        restore_committed_checkpoint(
            &directory.path,
            expected,
            &checkpoint_limits(),
            &graph_limits(),
            &policy,
        )
    };
    let restored = restore(&descriptor).unwrap();
    assert_eq!(restored.state.state().identity, state.state().identity);
    for field in 0..4 {
        let mut wrong = descriptor.clone();
        match field {
            0 => wrong.generation_hex = "0000000000000002".into(),
            1 => wrong.stored_byte_count_hex = "0000000000000001".into(),
            2 => wrong.graph_layout_sha256 = "0".repeat(64),
            _ => wrong.relative_filename = "../outside.checkpoint-v3".into(),
        }
        assert!(matches!(
            restore(&wrong),
            Err(CheckpointError::Format {
                code: "RESTORE_DESCRIPTOR",
                ..
            })
        ));
    }
    assert!(restore(&descriptor).is_ok());
}

/// A truncated partial is invalid while the same complete unreferenced file remains restorable.
#[test]
fn partial_is_rejected_and_complete_orphan_is_restorable() {
    let directory = TestDirectory::new("partial");
    let orphan_directory = TestDirectory::new("orphan");
    let (state, _graph, policy) = admitted_state(2, false);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000005",
    );
    let partial = directory.path.join("checkpoint-v3-test.partial");
    fs::copy(&path, &partial).unwrap();
    let partial_file = OpenOptions::new().write(true).open(&partial).unwrap();
    partial_file
        .set_len(partial_file.metadata().unwrap().len() - 512)
        .unwrap();
    drop(partial_file);
    assert!(restore_checkpoint(&partial, &checkpoint_limits(), &graph_limits(), &policy).is_err());

    let orphan = orphan_directory.path.join(&descriptor.relative_filename);
    fs::copy(&path, &orphan).unwrap();
    let restored = restore_checkpoint(&orphan, &checkpoint_limits(), &graph_limits(), &policy)
        .expect("complete metadata-orphan file remains a valid immutable checkpoint");
    assert_eq!(restored.state.state(), state.state());
}

/// An existing destination must retain different bytes and the rejected source stays retryable.
#[test]
fn no_replace_rename_preserves_existing_destination_and_source() {
    let directory = TestDirectory::new("rename-no-replace");
    let source = directory.path.join("candidate.partial");
    let destination = directory.path.join("retained.checkpoint-v3");
    fs::write(&source, b"new candidate").unwrap();
    fs::write(&destination, b"retained immutable bytes").unwrap();
    let error = rename_noreplace(&source, &destination).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&source).unwrap(), b"new candidate");
    assert_eq!(fs::read(&destination).unwrap(), b"retained immutable bytes");
    fs::remove_file(&destination).unwrap();
    rename_noreplace(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"new candidate");
}

/// Two different concurrent candidates produce one immutable winner and one intact loser.
#[test]
fn no_replace_rename_has_one_winner_for_different_concurrent_candidates() {
    let directory = TestDirectory::new("rename-different-race");
    let sources = [
        directory.path.join("a.partial"),
        directory.path.join("b.partial"),
    ];
    let payloads = [b"candidate a", b"candidate b"];
    let destination = directory.path.join("shared.checkpoint-v3");
    for (source, payload) in sources.iter().zip(payloads) {
        fs::write(source, payload).unwrap();
    }
    let start = std::sync::Barrier::new(3);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .iter()
            .map(|source| {
                let destination = &destination;
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    rename_noreplace(source, destination)
                })
            })
            .collect();
        start.wait();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for (index, result) in results.into_iter().enumerate() {
        if let Err(error) = result {
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(fs::read(&sources[index]).unwrap(), payloads[index]);
        } else {
            assert!(!sources[index].exists());
            assert_eq!(fs::read(&destination).unwrap(), payloads[index]);
        }
    }
}

/// The Windows API boundary supports long Unicode paths and rejects interior NUL before mutation.
#[cfg(target_os = "windows")]
#[test]
fn no_replace_rename_handles_extended_windows_paths_and_rejects_nul() {
    use std::os::windows::ffi::OsStrExt;

    let directory = TestDirectory::new("rename-long-unicode");
    let nested = directory.path.join("x".repeat(100)).join("y".repeat(100));
    fs::create_dir_all(&nested).unwrap();
    let source = nested.join("candidate-🐍.partial");
    let destination = nested.join("retained-🐍.checkpoint-v3");
    assert!(source.as_os_str().encode_wide().count() > 260);
    fs::write(&source, b"long Unicode candidate").unwrap();
    let invalid_destination = nested.join("invalid\0checkpoint");
    assert_eq!(
        rename_noreplace(&source, &invalid_destination)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read(&source).unwrap(), b"long Unicode candidate");
    rename_noreplace(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"long Unicode candidate");
}

/// Concurrent same-root publishers cannot replace each other and both validate one immutable file.
#[test]
fn concurrent_same_root_publication_is_no_replace_and_idempotent() {
    let directory = TestDirectory::new("race");
    let (state, _graph, policy) = admitted_state(3, false);
    let results = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            publish_checkpoint(
                &directory.path,
                CheckpointOperationId::parse("0000000000000000000000000000000e").unwrap(),
                14,
                state.checkpoint_boundary().unwrap(),
                &checkpoint_limits(),
                &graph_limits(),
                &policy,
            )
        });
        let right = scope.spawn(|| {
            publish_checkpoint(
                &directory.path,
                CheckpointOperationId::parse("0000000000000000000000000000000f").unwrap(),
                15,
                state.checkpoint_boundary().unwrap(),
                &checkpoint_limits(),
                &graph_limits(),
                &policy,
            )
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    let left = results.0.unwrap();
    let right = results.1.unwrap();
    assert_eq!(left.relative_filename, right.relative_filename);
    assert_eq!(left.logical_root_sha256, right.logical_root_sha256);
    assert_eq!(
        fs::read_dir(&directory.path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "checkpoint-v3"))
            .count(),
        1
    );
}
