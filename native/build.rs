use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Domain separator for the independently reproduced native source identity.
const SOURCE_HASH_DOMAIN: &[u8] = b"slither-neuroevo/native-source/v1\0";

/// Domain separator for effective compiler/codegen attributes admitted by state.
const BUILD_CONTRACT_DOMAIN: &[u8] = b"slither-neuroevo/native-build-contract/v1\0";

/// Fixed native inputs outside `src/**/*.rs` that affect the built addon.
const FIXED_SOURCE_FILES: &[&str] = &[
    ".cargo/config.toml",
    "Cargo.lock",
    "Cargo.toml",
    "build.rs",
    "fixtures/sensor-v3-reference.json",
    "package-lock.json",
    "package.json",
];

/// Convert one source input to the platform-independent logical-text form.
///
/// Every selected input is source/configuration text. CRLF and LF checkouts of
/// the same repository bytes therefore identify the same source, while lone
/// carriage returns and invalid UTF-8 fail rather than creating an ambiguous
/// cross-platform identity.
fn canonical_source_bytes(relative: &str, bytes: Vec<u8>) -> Vec<u8> {
    let text = String::from_utf8(bytes).unwrap_or_else(|error| {
        panic!("native build input {relative} is not valid UTF-8: {error}")
    });
    if text
        .as_bytes()
        .windows(1)
        .enumerate()
        .any(|(index, byte)| byte[0] == b'\r' && text.as_bytes().get(index + 1) != Some(&b'\n'))
    {
        panic!("native build input {relative} contains a lone carriage return");
    }
    text.replace("\r\n", "\n").into_bytes()
}

/// Extend an FNV-1a digest retained for the existing diagnostic identifier.
fn hash_fnv_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Convert one manifest-relative path to a stable UTF-8 path using `/`.
fn normalized_relative_path(manifest_dir: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(manifest_dir).unwrap_or_else(|error| {
        panic!(
            "native source path {} is outside {}: {error}",
            path.display(),
            manifest_dir.display()
        )
    });
    let mut components = Vec::new();
    for component in relative.components() {
        let text = component.as_os_str().to_str().unwrap_or_else(|| {
            panic!(
                "native source path is not valid UTF-8: {}",
                relative.display()
            )
        });
        components.push(text);
    }
    components.join("/")
}

/// Recursively collect regular Rust source files without following symlinks.
fn collect_rust_sources(manifest_dir: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to read an entry under {}: {error}",
                directory.display()
            )
        });
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("failed to inspect {}: {error}", path.display()));
        if metadata.file_type().is_symlink() {
            panic!("native source identity refuses symlink {}", path.display());
        }
        if metadata.is_dir() {
            collect_rust_sources(manifest_dir, &path, files);
        } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "rs")
        {
            files.push((normalized_relative_path(manifest_dir, &path), path));
        }
    }
}

/// Collect every input included in the recursive source identity.
fn source_files(manifest_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    for relative in FIXED_SOURCE_FILES {
        let mut path = manifest_dir.to_path_buf();
        for component in Path::new(relative).components() {
            path.push(component);
            let metadata = fs::symlink_metadata(&path).unwrap_or_else(|error| {
                panic!("failed to inspect native build input {relative}: {error}")
            });
            if metadata.file_type().is_symlink() {
                panic!("native build input {relative} must not traverse a symlink");
            }
        }
        if !fs::symlink_metadata(&path)
            .unwrap_or_else(|error| {
                panic!("failed to inspect native build input {relative}: {error}")
            })
            .is_file()
        {
            panic!("native build input {relative} must end at one regular file");
        }
        files.push(((*relative).to_owned(), path));
    }
    let source_root = manifest_dir.join("src");
    let source_metadata = fs::symlink_metadata(&source_root).unwrap_or_else(|error| {
        panic!(
            "failed to inspect native source root {}: {error}",
            source_root.display()
        )
    });
    if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
        panic!(
            "native source root {} must be one real directory",
            source_root.display()
        );
    }
    collect_rust_sources(manifest_dir, &source_root, &mut files);
    files.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for pair in files.windows(2) {
        if pair[0].0 == pair[1].0 {
            panic!("duplicate native source identity path {}", pair[0].0);
        }
    }
    files
}

/// Hash exact source paths and contents with explicit framing.
fn source_hashes(manifest_dir: &Path) -> (u64, String) {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=.cargo");
    let files = source_files(manifest_dir);
    let mut diagnostic_hash = 0xcbf2_9ce4_8422_2325;
    let mut source_sha = Sha256::new();
    source_sha.update(SOURCE_HASH_DOMAIN);
    let file_count = u64::try_from(files.len()).expect("native source file count fits u64");
    source_sha.update(file_count.to_le_bytes());
    diagnostic_hash = hash_fnv_bytes(diagnostic_hash, SOURCE_HASH_DOMAIN);
    diagnostic_hash = hash_fnv_bytes(diagnostic_hash, &file_count.to_le_bytes());

    for (relative, path) in files {
        println!("cargo:rerun-if-changed={relative}");
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("failed to read native build input {relative}: {error}")
        });
        let bytes = canonical_source_bytes(&relative, bytes);
        if relative.starts_with("src/")
            && relative.ends_with(".rs")
            && (bytes
                .windows(b"include_bytes!".len())
                .any(|window| window == b"include_bytes!")
                || bytes
                    .windows(b"include_str!".len())
                    .any(|window| window == b"include_str!"))
        {
            panic!(
                "native source {relative} embeds a non-Rust file; add that input to the versioned source-identity policy first"
            );
        }
        let path_length =
            u64::try_from(relative.len()).expect("native source path length fits u64");
        let content_length =
            u64::try_from(bytes.len()).expect("native source content length fits u64");
        for part in [
            path_length.to_le_bytes().as_slice(),
            relative.as_bytes(),
            content_length.to_le_bytes().as_slice(),
            bytes.as_slice(),
        ] {
            source_sha.update(part);
            diagnostic_hash = hash_fnv_bytes(diagnostic_hash, part);
        }
    }

    let digest = source_sha.finalize();
    let mut hexadecimal = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hexadecimal, "{byte:02x}").expect("writing to String cannot fail");
    }
    (diagnostic_hash, hexadecimal)
}

/// Read the containing repository revision without making Git a build requirement.
fn git_revision(manifest_dir: &Path) -> String {
    let repository = manifest_dir.parent().unwrap_or(manifest_dir);
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--short=12", "HEAD"])
        .output();
    match output {
        Ok(result) if result.status.success() => {
            String::from_utf8_lossy(&result.stdout).trim().to_owned()
        }
        _ => "unknown-revision".to_owned(),
    }
}

/// Ask Cargo to rebuild when the containing checkout advances to another Git
/// revision. The content SHA remains authoritative and works without Git;
/// these paths only keep the legacy diagnostic revision fresh when available.
fn emit_git_rerun_inputs(manifest_dir: &Path) {
    let repository = manifest_dir.parent().unwrap_or(manifest_dir);
    let dot_git = repository.join(".git");
    let git_dir = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) if metadata.is_dir() => dot_git,
        Ok(metadata) if metadata.is_file() => {
            let marker = fs::read_to_string(&dot_git).unwrap_or_default();
            let Some(path) = marker.trim().strip_prefix("gitdir:") else {
                return;
            };
            let path = PathBuf::from(path.trim());
            if path.is_absolute() {
                path
            } else {
                repository.join(path)
            }
        }
        _ => return,
    };

    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
    if let Ok(contents) = fs::read_to_string(&head) {
        if let Some(reference) = contents.trim().strip_prefix("ref:") {
            println!(
                "cargo:rerun-if-changed={}",
                git_dir.join(reference.trim()).display()
            );
        }
    }
}

/// Run rustc for one required, UTF-8 build-provenance response.
fn rustc_output(arguments: &[&str], description: &str) -> String {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let result = Command::new(rustc)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("failed to read {description}: {error}"));
    if !result.status.success() {
        panic!(
            "failed to read {description}: rustc exited with {}",
            result.status
        );
    }
    let value = String::from_utf8(result.stdout)
        .unwrap_or_else(|error| panic!("{description} is not valid UTF-8: {error}"));
    let value = value.replace("\r\n", "\n").trim().to_owned();
    if value.is_empty() {
        panic!("{description} must not be empty");
    }
    value
}

/// Read one required Cargo build attribute without accepting non-UTF-8 text.
fn required_build_attribute(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|error| panic!("Cargo {name} is required: {error}"))
}

/// Canonicalize Cargo's comma-separated effective target-feature set.
fn canonical_target_features(value: &str) -> String {
    let mut features = value
        .split(',')
        .filter(|feature| !feature.is_empty())
        .collect::<Vec<_>>();
    features.sort_unstable();
    features.dedup();
    features.join(",")
}

/// Hash one named build attribute with unambiguous length framing.
fn hash_build_attribute(hasher: &mut Sha256, name: &str, value: &str) {
    hasher.update(
        u64::try_from(name.len())
            .expect("build-attribute name length fits u64")
            .to_le_bytes(),
    );
    hasher.update(name.as_bytes());
    hasher.update(
        u64::try_from(value.len())
            .expect("build-attribute value length fits u64")
            .to_le_bytes(),
    );
    hasher.update(value.as_bytes());
}

/// Reject ambient mechanisms whose output is not identified by tracked source
/// plus rustc's effective flags and version.
fn reject_unidentified_build_overrides(profile: &str, build_class: &str) {
    if profile == "release" && build_class == "production" {
        for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
            if std::env::var_os(wrapper).is_some_and(|value| !value.is_empty()) {
                panic!(
                    "{wrapper} is unsupported for a release production native build because wrapper output cannot be identified"
                );
            }
        }
    }
    for (name, value) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("CARGO_PROFILE_") && !value.is_empty() {
            panic!(
                "{} is unsupported; edit the tracked Cargo profile instead of applying an ambient profile override",
                name.to_string_lossy()
            );
        }
    }
}

/// Read one optional UTF-8 build attribute, treating absence as empty.
fn optional_build_attribute(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|error| {
        if matches!(error, std::env::VarError::NotPresent) {
            String::new()
        } else {
            panic!("{name} is not valid UTF-8: {error}")
        }
    })
}

/// Make ambient overrides for the active profile invalidate cached build-script output.
fn emit_profile_override_rerun_inputs(profile: &str) {
    let profile = profile
        .chars()
        .map(|character| match character {
            'a'..='z' => character.to_ascii_uppercase(),
            'A'..='Z' | '0'..='9' => character,
            _ => '_',
        })
        .collect::<String>();
    for setting in [
        "CODEGEN_UNITS",
        "DEBUG",
        "DEBUG_ASSERTIONS",
        "INCREMENTAL",
        "LTO",
        "OPT_LEVEL",
        "OVERFLOW_CHECKS",
        "PANIC",
        "RPATH",
        "STRIP",
    ] {
        println!("cargo:rerun-if-env-changed=CARGO_PROFILE_{profile}_{setting}");
    }
}

/// Calculate the versioned effective build contract used for state admission.
fn build_contract_sha256(
    target: &str,
    profile: &str,
    build_class: &str,
    rustc_verbose: &str,
) -> String {
    let panic_strategy = required_build_attribute("CARGO_CFG_PANIC");
    if panic_strategy != "unwind" {
        panic!("authoritative native builds require panic=unwind; Cargo selected {panic_strategy}");
    }
    let opt_level = required_build_attribute("OPT_LEVEL");
    let debug = required_build_attribute("DEBUG");
    let target_features =
        canonical_target_features(&std::env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default());
    let encoded_rustflags = optional_build_attribute("CARGO_ENCODED_RUSTFLAGS");
    let rustc_wrapper = optional_build_attribute("RUSTC_WRAPPER");
    let rustc_workspace_wrapper = optional_build_attribute("RUSTC_WORKSPACE_WRAPPER");
    let attributes = [
        ("target", target),
        ("profile", profile),
        ("build-class", build_class),
        ("rustc-verbose", rustc_verbose),
        ("panic", panic_strategy.as_str()),
        ("opt-level", opt_level.as_str()),
        ("debug", debug.as_str()),
        ("target-features", target_features.as_str()),
        ("encoded-rustflags", encoded_rustflags.as_str()),
        ("rustc-wrapper", rustc_wrapper.as_str()),
        ("rustc-workspace-wrapper", rustc_workspace_wrapper.as_str()),
    ];
    let mut hasher = Sha256::new();
    hasher.update(BUILD_CONTRACT_DOMAIN);
    hasher.update(
        u64::try_from(attributes.len())
            .expect("build-attribute count fits u64")
            .to_le_bytes(),
    );
    for (name, value) in attributes {
        hash_build_attribute(&mut hasher, name, value);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn main() {
    napi_build::setup();
    for name in [
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "RUSTFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is required"),
    );
    let crate_version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is required");
    let target = std::env::var("TARGET").expect("Cargo TARGET is required");
    let profile = std::env::var("PROFILE").expect("Cargo PROFILE is required");
    let build_class = if std::env::var_os("CARGO_FEATURE_ENGINE_TEST_HOOKS").is_some() {
        "test-hooks"
    } else {
        "production"
    };
    emit_profile_override_rerun_inputs(&profile);
    reject_unidentified_build_overrides(&profile, build_class);
    let rustc_version = rustc_output(&["--version"], "rustc version");
    let rustc_verbose = rustc_output(&["--version", "--verbose"], "verbose rustc version");
    let build_contract_sha256 =
        build_contract_sha256(&target, &profile, build_class, &rustc_verbose);
    emit_git_rerun_inputs(&manifest_dir);
    let revision = git_revision(&manifest_dir);
    let (diagnostic_hash, source_sha256) = source_hashes(&manifest_dir);
    let identifier = format!("slither_native/{crate_version}+{revision}.{diagnostic_hash:016x}");
    println!("cargo:rustc-env=SLITHER_NATIVE_BUILD_IDENTIFIER={identifier}");
    println!("cargo:rustc-env=SLITHER_NATIVE_SOURCE_SHA256={source_sha256}");
    println!("cargo:rustc-env=SLITHER_NATIVE_BUILD_TARGET={target}");
    println!("cargo:rustc-env=SLITHER_NATIVE_BUILD_PROFILE={profile}");
    println!("cargo:rustc-env=SLITHER_NATIVE_BUILD_CLASS={build_class}");
    println!("cargo:rustc-env=SLITHER_NATIVE_RUSTC_VERSION={rustc_version}");
    println!("cargo:rustc-env=SLITHER_NATIVE_BUILD_CONTRACT_SHA256={build_contract_sha256}");
}
