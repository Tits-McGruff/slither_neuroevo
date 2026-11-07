use super::*;

/// Existing-file verification reads all five roles and checks mirrored facts
/// while retaining only codec blocks instead of a second decoded population.
#[test]
fn streamed_existing_verification_rejects_each_corrupt_role_and_manifest_mirror() {
    for compressed in [false, true] {
        let directory = TestDirectory::new("stream-verify");
        let (state, _graph, policy) = admitted_state(3, compressed);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000091",
        );
        let limits = checkpoint_limits();
        let original = fs::read(&path).unwrap();
        let mut file = File::open(&path).unwrap();
        let entries = scan_strict_ustar(&mut file, original.len() as u64, &limits).unwrap();
        let verify = || {
            verify_existing_checkpoint_content(
                &path,
                state.state(),
                state.graph(),
                &descriptor.logical_root_sha256,
                &limits,
            )
        };
        let expected = restore_checkpoint(&path, &limits, &graph_limits(), &policy)
            .unwrap()
            .content;
        assert_eq!(verify().unwrap(), expected);
        for entry in &entries[..5] {
            let mut corrupt = original.clone();
            corrupt[entry.data_offset as usize] ^= 1;
            fs::write(&path, corrupt).unwrap();
            assert!(verify().is_err(), "unverified role {}", entry.name);
            fs::write(&path, &original).unwrap();
        }
        let entry = &entries[5];
        let start = entry.data_offset as usize;
        let end = start + entry.size as usize;
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&original[start..end]).unwrap();
        let generation = manifest["generationHex"].as_str().unwrap().to_owned();
        let replacement = if generation.ends_with('0') { '1' } else { '0' };
        let mut changed = generation;
        changed.pop();
        changed.push(replacement);
        manifest["generationHex"] = serde_json::Value::String(changed);
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(bytes.len(), entry.size as usize);
        let mut corrupt = original.clone();
        corrupt[start..end].copy_from_slice(&bytes);
        fs::write(&path, corrupt).unwrap();
        assert!(verify().is_err());
        fs::write(&path, original).unwrap();
        assert_eq!(verify().unwrap(), expected);
    }
}

/// Outer save validation rejects ignored container bytes before creating import scratch.
#[test]
fn save_import_rejects_noncanonical_padding_trailer_and_hidden_name_bytes() {
    use crate::engine::export_archive::validate_import_archive;

    let fixture = hall_of_fame_import_fixture();
    let original = fs::read(&fixture.archive_path).unwrap();
    let mut file = File::open(&fixture.archive_path).unwrap();
    let entries =
        scan_strict_ustar_roles(&mut file, original.len() as u64, 9, |_, _| Ok(())).unwrap();
    let padded = entries.iter().find(|entry| entry.size % 512 != 0).unwrap();
    for fault in ["padding", "trailer", "name"] {
        let target = TestDirectory::new("save-container-rejection");
        let path = target.path.join("damaged.slither-save");
        let mut damaged = original.clone();
        let expected = match fault {
            "padding" => {
                damaged[(padded.data_offset + padded.size) as usize] = 1;
                "nonzero padding"
            }
            "trailer" => {
                let last = damaged.len() - 1;
                damaged[last] = 1;
                "second USTAR trailer block is nonzero"
            }
            "name" => {
                let nul = damaged[..100].iter().position(|byte| *byte == 0).unwrap();
                damaged[nul + 1] = b'x';
                refresh_test_ustar_checksum(&mut damaged[..512]);
                "hidden bytes after NUL"
            }
            _ => unreachable!(),
        };
        fs::write(&path, &damaged).unwrap();
        let error = validate_import_archive(
            &path,
            &target.path,
            &"d".repeat(32),
            &checkpoint_limits(),
            &graph_limits(),
            &fixture.policy,
        )
        .expect_err("noncanonical outer bytes must be rejected before import extraction");
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(fs::read_dir(&target.path).unwrap().count(), 1);
        assert_eq!(fs::read(path).unwrap(), damaged);
        assert_eq!(fs::read(&fixture.archive_path).unwrap(), original);
        assert_eq!(
            validate_import_archive(
                &fixture.archive_path,
                &target.path,
                &"e".repeat(32),
                &checkpoint_limits(),
                &graph_limits(),
                &fixture.policy,
            )
            .unwrap()
            .hall_of_fame_count_hex,
            encode_u64_hex(1)
        );
        assert_eq!(fs::read_dir(&target.path).unwrap().count(), 1);
    }
}

/// Manifest mirrors are not trusted merely because the logical role root is unchanged.
#[test]
fn manifest_mirror_tampering_is_rejected() {
    let directory = TestDirectory::new("manifest-source");
    let (state, _graph, policy) = admitted_state(3, false);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "0000000000000000000000000000000a",
    );
    let mut file = File::open(&path).unwrap();
    let length = file.metadata().unwrap().len();
    let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
    let manifest = &entries[5];
    let original = fs::read(&path).unwrap();
    assert_mutated_archive_rejected(
        "manifest-tamper",
        &original,
        &descriptor.relative_filename,
        &policy,
        |bytes| {
            let start = manifest.data_offset as usize;
            let end = start + manifest.size as usize;
            let needle = b"\"generationHex\":\"0000000000000001\"";
            let position = bytes[start..end]
                .windows(needle.len())
                .position(|window| window == needle)
                .expect("manifest generation mirror must be present")
                + start;
            bytes[position + needle.len() - 2] = b'2';
        },
    );
}

/// Strict USTAR parsing rejects checksum, trailer, padding, duplicate/order/path, and type faults.
#[test]
fn strict_ustar_fault_matrix_is_rejected() {
    let directory = TestDirectory::new("ustar-source");
    let (state, _graph, policy) = admitted_state(3, false);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "0000000000000000000000000000000b",
    );
    let mut file = File::open(&path).unwrap();
    let length = file.metadata().unwrap().len();
    let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
    let original = fs::read(&path).unwrap();
    let filename = descriptor.relative_filename.as_str();

    assert_mutated_archive_rejected("ustar-checksum", &original, filename, &policy, |bytes| {
        bytes[0] ^= 1;
    });
    assert_mutated_archive_rejected("ustar-trailer", &original, filename, &policy, |bytes| {
        let last = bytes.len() - 1;
        bytes[last] = 1;
    });
    assert_mutated_archive_rejected("ustar-padding", &original, filename, &policy, |bytes| {
        let end = (entries[0].data_offset + entries[0].size) as usize;
        assert_ne!(end % 512, 0);
        bytes[end] = 1;
    });
    assert_mutated_archive_rejected("ustar-duplicate", &original, filename, &policy, |bytes| {
        let start = entries[1].header_offset as usize;
        set_test_ustar_path(&mut bytes[start..start + 512], STATE_PATH);
    });
    assert_mutated_archive_rejected("ustar-path", &original, filename, &policy, |bytes| {
        set_test_ustar_path(&mut bytes[..512], "../evil");
    });
    assert_mutated_archive_rejected("ustar-type", &original, filename, &policy, |bytes| {
        bytes[156] = b'2';
        refresh_test_ustar_checksum(&mut bytes[..512]);
    });
    assert_mutated_archive_rejected(
        "ustar-manifest-order",
        &original,
        filename,
        &policy,
        |bytes| {
            let manifest_header = entries[5].header_offset as usize;
            let recurrent_header = entries[4].header_offset as usize;
            set_test_ustar_path(
                &mut bytes[manifest_header..manifest_header + 512],
                RECURRENT_RAW_PATH,
            );
            set_test_ustar_path(
                &mut bytes[recurrent_header..recurrent_header + 512],
                MANIFEST_PATH,
            );
        },
    );
}

/// A corrupt same-name destination is never overwritten or trusted by filename alone.
#[test]
fn corrupt_existing_digest_file_is_left_untouched() {
    let directory = TestDirectory::new("collision");
    let (state, _graph, policy) = admitted_state(3, false);
    let (path, _) = publish_fixture(
        &directory,
        &state,
        &policy,
        "0000000000000000000000000000000c",
    );
    let mut corrupt = fs::read(&path).unwrap();
    corrupt[0] ^= 1;
    fs::write(&path, &corrupt).unwrap();
    let error = publish_checkpoint(
        &directory.path,
        CheckpointOperationId::parse("0000000000000000000000000000000d").unwrap(),
        2,
        state.checkpoint_boundary().unwrap(),
        &checkpoint_limits(),
        &graph_limits(),
        &policy,
    )
    .expect_err("corrupt digest collision must fail");
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "DIGEST_COLLISION",
            ..
        }
    ));
    assert_eq!(fs::read(&path).unwrap(), corrupt);
}

/// Direct restore rejects non-regular filesystem objects before archive parsing.
#[test]
fn restore_rejects_non_regular_target() {
    let directory = TestDirectory::new("non-regular");
    let error = restore_checkpoint(
        &directory.path,
        &checkpoint_limits(),
        &graph_limits(),
        &admission_policy("sha256:".to_owned() + &"0".repeat(64)),
    )
    .expect_err("directory target must be rejected");
    assert!(matches!(
        error,
        CheckpointError::Format {
            code: "MANAGED_FILE_TYPE",
            ..
        }
    ));
}

/// Unix restores reject a direct symlink even when it points to a valid regular checkpoint.
#[cfg(unix)]
#[test]
fn restore_rejects_symlink_target() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("symlink");
    let (state, _graph, policy) = admitted_state(2, false);
    let (path, descriptor) = publish_fixture(
        &directory,
        &state,
        &policy,
        "00000000000000000000000000000010",
    );
    let link_directory = TestDirectory::new("symlink-link");
    let link = link_directory.path.join(descriptor.relative_filename);
    symlink(path, &link).unwrap();
    assert!(matches!(
        restore_checkpoint(&link, &checkpoint_limits(), &graph_limits(), &policy),
        Err(CheckpointError::Format {
            code: "MANAGED_FILE_TYPE",
            ..
        })
    ));
}
