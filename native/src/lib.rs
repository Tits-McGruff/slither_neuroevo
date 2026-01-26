#![deny(clippy::all)]

#[path = "SIMD_Kernals.rs"]
mod simd_kernels;

pub use simd_kernels::{
  dense_forward_native,
  gru_step_native,
  lstm_step_native,
  mlp_forward_native,
  rru_step_native,
};
