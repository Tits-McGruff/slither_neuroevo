//! Internal x86_64 SIMD dot products for authoritative heterogeneous inference.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("Native SIMD kernels require x86_64 (SSE). No non-x86_64 fallback is enabled.");

/// Compute a SIMD-accelerated dot product.
///
/// # Safety
///
/// Pointers must be valid for `in_size` reads.
/// `in_size` must be non-negative.
#[inline]
unsafe fn dense_dot(weights_ptr: *const f32, input_ptr: *const f32, in_size: usize) -> f32 {
    let mut i = 0usize;
    // SAFETY: The caller guarantees both pointers are valid for `in_size` reads;
    // this loop accesses only complete four-element chunks below that bound.
    let mut total = unsafe {
        let mut sum = _mm_setzero_ps();
        while i + 4 <= in_size {
            let w = _mm_loadu_ps(weights_ptr.add(i));
            let x = _mm_loadu_ps(input_ptr.add(i));
            sum = _mm_add_ps(sum, _mm_mul_ps(w, x));
            i += 4;
        }
        let mut buf = [0.0_f32; 4];
        _mm_storeu_ps(buf.as_mut_ptr(), sum);
        buf[0] + buf[1] + buf[2] + buf[3]
    };
    // SAFETY: The caller guarantees both pointers are valid for `in_size` reads,
    // and the scalar tail is bounded by `i < in_size`.
    unsafe {
        while i < in_size {
            total += *weights_ptr.add(i) * *input_ptr.add(i);
            i += 1;
        }
    }
    total
}

/// Compute a SIMD-accelerated dot product with two inputs multiplied together.
///
/// # Safety
///
/// Pointers must be valid for `len` reads.
#[inline]
unsafe fn dense_dot_mul(
    weights_ptr: *const f32,
    a_ptr: *const f32,
    b_ptr: *const f32,
    len: usize,
) -> f32 {
    let mut i = 0usize;
    // SAFETY: The caller guarantees all three pointers are valid for `len`
    // reads; this loop accesses only complete four-element chunks below it.
    let mut total = unsafe {
        let mut sum = _mm_setzero_ps();
        while i + 4 <= len {
            let w = _mm_loadu_ps(weights_ptr.add(i));
            let a = _mm_loadu_ps(a_ptr.add(i));
            let b = _mm_loadu_ps(b_ptr.add(i));
            let ab = _mm_mul_ps(a, b);
            sum = _mm_add_ps(sum, _mm_mul_ps(w, ab));
            i += 4;
        }
        let mut buf = [0.0_f32; 4];
        _mm_storeu_ps(buf.as_mut_ptr(), sum);
        buf[0] + buf[1] + buf[2] + buf[3]
    };
    // SAFETY: The caller guarantees all pointers are valid for `len` reads,
    // and the scalar tail is bounded by `i < len`.
    unsafe {
        while i < len {
            total += *weights_ptr.add(i) * (*a_ptr.add(i) * *b_ptr.add(i));
            i += 1;
        }
    }
    total
}

/// Return whether the process may select the existing SSE2-backed dot kernels.
pub(crate) fn runtime_sse2_available() -> bool {
    std::arch::is_x86_feature_detected!("sse2")
}

/// Apply the existing SIMD dot product to two equally sized safe slices.
///
/// A length mismatch is an internal programming error and panics before any raw
/// pointer is formed. Runtime callers select this helper only after
/// [`runtime_sse2_available`] succeeds.
#[inline]
pub(crate) fn dot_product_sse2(weights: &[f32], input: &[f32]) -> f32 {
    assert_eq!(
        weights.len(),
        input.len(),
        "SIMD dot-product slice length mismatch"
    );
    // SAFETY: Equal safe slices prove both pointers are valid for `input.len()`
    // reads. The runtime backend selector has admitted SSE2 for this process.
    unsafe { dense_dot(weights.as_ptr(), input.as_ptr(), input.len()) }
}

/// Apply the existing SIMD weighted-product dot to three equal safe slices.
///
/// Length mismatches panic before a pointer is formed. Runtime callers select
/// this helper only after [`runtime_sse2_available`] succeeds.
#[inline]
pub(crate) fn dot_product_mul_sse2(weights: &[f32], left: &[f32], right: &[f32]) -> f32 {
    assert_eq!(
        weights.len(),
        left.len(),
        "SIMD multiplied-dot weight/input length mismatch"
    );
    assert_eq!(
        left.len(),
        right.len(),
        "SIMD multiplied-dot input length mismatch"
    );
    // SAFETY: Equal safe slices prove every pointer is valid for `left.len()`
    // reads. The runtime backend selector has admitted SSE2 for this process.
    unsafe { dense_dot_mul(weights.as_ptr(), left.as_ptr(), right.as_ptr(), left.len()) }
}
