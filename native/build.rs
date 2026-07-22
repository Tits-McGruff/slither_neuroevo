use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Files whose content defines the native addon's source identity.
const SOURCE_FILES: &[&str] = &[
    "build.rs",
    "Cargo.toml",
    "package.json",
    "src/lib.rs",
    "src/simd_kernels.rs",
];

/// Extend a deterministic FNV-1a digest with one byte slice.
fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Hash the exact native source and manifest inputs compiled into the addon.
fn source_hash(manifest_dir: &Path) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for relative in SOURCE_FILES {
        println!("cargo:rerun-if-changed={relative}");
        hash = hash_bytes(hash, relative.as_bytes());
        let bytes = fs::read(manifest_dir.join(relative)).unwrap_or_else(|error| {
            panic!("failed to read native build input {relative}: {error}")
        });
        hash = hash_bytes(hash, &bytes);
    }
    hash
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

fn main() {
    napi_build::setup();
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is required"),
    );
    let crate_version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is required");
    let revision = git_revision(&manifest_dir);
    let content_hash = source_hash(&manifest_dir);
    let identifier = format!("slither_native/{crate_version}+{revision}.{content_hash:016x}");
    println!("cargo:rustc-env=SLITHER_NATIVE_BUILD_IDENTIFIER={identifier}");
}
