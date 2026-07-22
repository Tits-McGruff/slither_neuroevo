#![deny(clippy::all)]

mod simd_kernels;

use napi_derive::napi;

pub use simd_kernels::{
    dense_forward_native, gru_step_native, lstm_step_native, mlp_forward_native, rru_step_native,
};

/// Return the crate, source-revision, and source-content identity embedded at build time.
#[napi(js_name = "nativeAddonBuildIdentifier")]
pub fn native_addon_build_identifier() -> String {
    env!("SLITHER_NATIVE_BUILD_IDENTIFIER").to_owned()
}
