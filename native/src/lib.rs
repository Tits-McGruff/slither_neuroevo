#![deny(clippy::all)]

#[cfg(not(panic = "unwind"))]
compile_error!("the authoritative native addon requires panic=unwind");

/// Rust-owned authoritative engine components under staged migration.
pub mod engine;
mod napi_engine;
mod simd_kernels;

use napi_derive::napi;

pub use simd_kernels::{
    dense_forward_native, gru_step_native, lstm_step_native, mlp_forward_native, rru_step_native,
};

pub use napi_engine::experimental_engine_contract_version;

/// Return the crate, source-revision, and source-content identity embedded at build time.
#[napi(js_name = "nativeAddonBuildIdentifier")]
pub fn native_addon_build_identifier() -> String {
    env!("SLITHER_NATIVE_BUILD_IDENTIFIER").to_owned()
}

/// Return the platform-independent SHA-256 of selected native source inputs.
#[napi(js_name = "nativeAddonSourceSha256")]
pub fn native_addon_source_sha256() -> String {
    env!("SLITHER_NATIVE_SOURCE_SHA256").to_owned()
}

/// Return Cargo's exact compilation target triple for this addon.
#[napi(js_name = "nativeAddonBuildTarget")]
pub fn native_addon_build_target() -> String {
    env!("SLITHER_NATIVE_BUILD_TARGET").to_owned()
}

/// Return Cargo's profile name for diagnostic build provenance.
#[napi(js_name = "nativeAddonBuildProfile")]
pub fn native_addon_build_profile() -> String {
    env!("SLITHER_NATIVE_BUILD_PROFILE").to_owned()
}

/// Distinguish production builds from explicitly enabled test-hook builds.
#[napi(js_name = "nativeAddonBuildClass")]
pub fn native_addon_build_class() -> String {
    env!("SLITHER_NATIVE_BUILD_CLASS").to_owned()
}

/// Return the compiler version captured by the build script for diagnostics.
#[napi(js_name = "nativeAddonRustcVersion")]
pub fn native_addon_rustc_version() -> String {
    env!("SLITHER_NATIVE_RUSTC_VERSION").to_owned()
}

/// Return the versioned digest of effective correctness-relevant build attributes.
#[napi(js_name = "nativeAddonBuildContractSha256")]
pub fn native_addon_build_contract_sha256() -> String {
    env!("SLITHER_NATIVE_BUILD_CONTRACT_SHA256").to_owned()
}

#[cfg(test)]
mod build_metadata_tests {
    use super::*;

    #[test]
    fn build_contract_is_a_versioned_sha256_identity() {
        let identity = native_addon_build_contract_sha256();
        let digest = identity
            .strip_prefix("sha256:")
            .expect("build contract uses the admitted SHA-256 form");
        assert_eq!(digest.len(), 64);
        assert!(digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
}
