//! Internal x86_64 SIMD kernels for authoritative inference and diagnostic species bounds.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("Native SIMD kernels require x86_64 (SSE). No non-x86_64 fallback is enabled.");

/// Compute a SIMD-accelerated dot product.
///
/// Long rows use four independent accumulators to hide add latency while reading
/// contiguous weights. Reduction order is part of the source/build identity;
/// scalar/SSE2 comparisons use the established numeric tolerances.
///
/// # Safety
///
/// Pointers must be valid for `in_size` reads.
/// `in_size` must be non-negative.
#[inline]
unsafe fn dense_dot(weights_ptr: *const f32, input_ptr: *const f32, in_size: usize) -> f32 {
    let mut i = 0usize;
    // SAFETY: The caller guarantees both pointers are valid for `in_size` reads;
    // the unrolled loop reads four complete groups only when i + 16 <= in_size,
    // and the following loop reads only complete four-element chunks. Inputs may
    // alias because every pointer access is read-only.
    let mut total = unsafe {
        let mut sum = _mm_setzero_ps();
        if in_size >= 32 {
            let mut b = sum;
            let mut c = sum;
            let mut d = sum;
            while i + 16 <= in_size {
                sum = _mm_add_ps(
                    sum,
                    _mm_mul_ps(
                        _mm_loadu_ps(weights_ptr.add(i)),
                        _mm_loadu_ps(input_ptr.add(i)),
                    ),
                );
                b = _mm_add_ps(
                    b,
                    _mm_mul_ps(
                        _mm_loadu_ps(weights_ptr.add(i + 4)),
                        _mm_loadu_ps(input_ptr.add(i + 4)),
                    ),
                );
                c = _mm_add_ps(
                    c,
                    _mm_mul_ps(
                        _mm_loadu_ps(weights_ptr.add(i + 8)),
                        _mm_loadu_ps(input_ptr.add(i + 8)),
                    ),
                );
                d = _mm_add_ps(
                    d,
                    _mm_mul_ps(
                        _mm_loadu_ps(weights_ptr.add(i + 12)),
                        _mm_loadu_ps(input_ptr.add(i + 12)),
                    ),
                );
                i += 16;
            }
            sum = _mm_add_ps(_mm_add_ps(_mm_add_ps(sum, b), c), d);
        }
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

/// Sum squared Float32 differences in Float64 with a reordered positive reduction.
///
/// This is an approximate bound input, not an inference kernel or an exact scalar
/// distance replacement. Callers must guard any decision against reduction error.
/// Equal safe slices permit read-only aliasing; a mismatch panics before raw loads.
/// SSE2 is guaranteed by the supported x86_64 targets.
pub(crate) fn squared_distance_f64_sse2(left: &[f32], right: &[f32]) -> f64 {
    assert_eq!(
        left.len(),
        right.len(),
        "SIMD distance slice length mismatch"
    );
    let mut index = 0;
    // SAFETY: Both slices have equal lengths. Each iteration reads two complete
    // four-f32 spans per slice only when index + 8 <= len. The only write is to
    // the separate two-f64 stack array; inputs may overlap because all are reads.
    let lanes = unsafe {
        let mut a = _mm_setzero_pd();
        let mut b = a;
        let mut c = a;
        let mut d = a;
        while index + 8 <= left.len() {
            let left0 = _mm_loadu_ps(left.as_ptr().add(index));
            let right0 = _mm_loadu_ps(right.as_ptr().add(index));
            let left1 = _mm_loadu_ps(left.as_ptr().add(index + 4));
            let right1 = _mm_loadu_ps(right.as_ptr().add(index + 4));
            let low0 = _mm_sub_pd(_mm_cvtps_pd(left0), _mm_cvtps_pd(right0));
            let high0 = _mm_sub_pd(
                _mm_cvtps_pd(_mm_movehl_ps(left0, left0)),
                _mm_cvtps_pd(_mm_movehl_ps(right0, right0)),
            );
            let low1 = _mm_sub_pd(_mm_cvtps_pd(left1), _mm_cvtps_pd(right1));
            let high1 = _mm_sub_pd(
                _mm_cvtps_pd(_mm_movehl_ps(left1, left1)),
                _mm_cvtps_pd(_mm_movehl_ps(right1, right1)),
            );
            a = _mm_add_pd(a, _mm_mul_pd(low0, low0));
            b = _mm_add_pd(b, _mm_mul_pd(high0, high0));
            c = _mm_add_pd(c, _mm_mul_pd(low1, low1));
            d = _mm_add_pd(d, _mm_mul_pd(high1, high1));
            index += 8;
        }
        let mut lanes = [0.0_f64; 2];
        _mm_storeu_pd(
            lanes.as_mut_ptr(),
            _mm_add_pd(_mm_add_pd(a, b), _mm_add_pd(c, d)),
        );
        lanes
    };
    let mut total = lanes[0] + lanes[1];
    for (&a, &b) in left[index..].iter().zip(&right[index..]) {
        let difference = f64::from(a) - f64::from(b);
        total += difference * difference;
    }
    total
}

#[cfg(test)]
mod distance_tests {
    use super::squared_distance_f64_sse2;

    #[test]
    fn positive_distance_reduction_covers_tails_aliases_and_float32_extremes() {
        let patterns = [
            0.0_f32,
            -0.0,
            f32::from_bits(1),
            -f32::from_bits(1),
            0.35,
            -0.35,
            1.0,
            -5.0,
            f32::MAX,
            -f32::MAX,
        ];
        for length in 0..=273 {
            let left: Vec<_> = (0..length).map(|i| patterns[i % patterns.len()]).collect();
            let right: Vec<_> = (0..length)
                .map(|i| patterns[(i + 3) % patterns.len()])
                .collect();
            let scalar = left.iter().zip(&right).fold(0.0, |sum, (&a, &b)| {
                let difference = f64::from(a) - f64::from(b);
                sum + difference * difference
            });
            let actual = squared_distance_f64_sse2(&left, &right);
            assert!(actual.is_finite());
            assert!((actual - scalar).abs() <= scalar * 1e-12);
            assert_eq!(squared_distance_f64_sse2(&left, &left), 0.0);
        }
        let tiny = [f32::from_bits(1); 9];
        let expected = 9.0 * f64::from(tiny[0]).powi(2);
        assert_eq!(squared_distance_f64_sse2(&tiny, &[0.0; 9]), expected);
    }

    #[test]
    #[should_panic(expected = "SIMD distance slice length mismatch")]
    fn distance_rejects_unequal_lengths_before_loading() {
        squared_distance_f64_sse2(&[0.0; 9], &[0.0; 8]);
    }
}

#[cfg(test)]
mod dot_tests {
    use super::dot_product_sse2;

    /// Independent Float64 oracle covers vector boundaries, odd tails and read-only aliasing.
    #[test]
    fn independent_accumulators_cover_every_tail_and_admitted_long_row() {
        for length in (0..=135).chain([248, 256, 264, 504, 512, 1024]) {
            let left = (0..length)
                .map(|i| (((i * 3571) % 10007) as f32 - 5003.0) / 16000.0)
                .collect::<Vec<_>>();
            let right = (0..length)
                .map(|i| (((i * 1741 + 193) % 10007) as f32 - 5003.0) / 5003.0)
                .collect::<Vec<_>>();
            for input in [&right, &left] {
                let expected = left
                    .iter()
                    .zip(input)
                    .fold(0.0_f64, |sum, (&a, &b)| sum + f64::from(a) * f64::from(b));
                let actual = f64::from(dot_product_sse2(&left, input));
                assert!(actual.is_finite());
                assert!(
                    (actual - expected).abs() <= 1e-5 + 1e-4 * expected.abs(),
                    "length={length}, actual={actual}, expected={expected}"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "SIMD dot-product slice length mismatch")]
    fn independent_accumulators_reject_mismatch_before_raw_reads() {
        dot_product_sse2(&[0.0; 33], &[0.0; 32]);
    }
}

/// Runtime admission includes OS support for AVX register state through Rust's
/// FMA detector. No AVX2 instructions or 256-bit vectors are used by this path.
pub(crate) fn runtime_i16_fma128_available() -> bool {
    std::arch::is_x86_feature_detected!("sse4.1") && std::arch::is_x86_feature_detected!("fma")
}

/// Safe, equal-slice entry; backend construction must have admitted SSE4.1/FMA.
#[inline]
pub(crate) fn dot_product_i16_fma128(
    weights: &[i16],
    input: &[f32],
    multiplier: Option<&[f32]>,
    scale: f32,
) -> f32 {
    assert_eq!(weights.len(), input.len());
    if let Some(right) = multiplier {
        assert_eq!(right.len(), input.len());
    }
    // SAFETY: Equal slice lengths establish readable, possibly unaligned spans;
    // the execution-plan constructor checks CPU features before selecting this kernel.
    unsafe { dot_i16_matrix_fma128(weights, input, multiplier, scale) }
}

/// Eight signed i16 values per unaligned source load, two four-lane FMA chains.
/// Scale is applied once after reduction and the scalar tail.
///
/// # Safety
/// The CPU must support SSE4.1 and FMA. Slice lengths must match.
#[target_feature(enable = "sse4.1,fma")]
unsafe fn dot_i16_matrix_fma128(
    weights: &[i16],
    input: &[f32],
    multiplier: Option<&[f32]>,
    scale: f32,
) -> f32 {
    let mut index = 0;
    let mut a = _mm_setzero_ps();
    let mut b = a;
    // SAFETY: Each load is bounded by a complete eight-element span. Float
    // loads each cover four elements of that span. No alignment is assumed.
    unsafe {
        while index + 8 <= input.len() {
            let packed = _mm_loadu_si128(weights.as_ptr().add(index).cast());
            let low = _mm_cvtepi32_ps(_mm_cvtepi16_epi32(packed));
            let high = _mm_cvtepi32_ps(_mm_cvtepi16_epi32(_mm_srli_si128::<8>(packed)));
            let mut x = _mm_loadu_ps(input.as_ptr().add(index));
            let mut y = _mm_loadu_ps(input.as_ptr().add(index + 4));
            if let Some(right) = multiplier {
                x = _mm_mul_ps(x, _mm_loadu_ps(right.as_ptr().add(index)));
                y = _mm_mul_ps(y, _mm_loadu_ps(right.as_ptr().add(index + 4)));
            }
            a = _mm_fmadd_ps(low, x, a);
            b = _mm_fmadd_ps(high, y, b);
            index += 8;
        }
        let mut lanes = [0.0_f32; 4];
        _mm_storeu_ps(lanes.as_mut_ptr(), _mm_add_ps(a, b));
        let mut sum = lanes[0] + lanes[1] + lanes[2] + lanes[3];
        while index < input.len() {
            let x = input[index] * multiplier.map_or(1.0, |r| r[index]);
            sum = (weights[index] as f32).mul_add(x, sum);
            index += 1;
        }
        sum * scale
    }
}

/// Benchmark-only F32 comparator; it is never a persisted production backend.
#[cfg(feature = "engine-test-hooks")]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn f32_fma128(weights: &[f32], input: &[f32]) -> f32 {
    let mut i = 0;
    let mut a = _mm_setzero_ps();
    let mut b = a;
    // SAFETY: Equal slices at the caller and full-vector loop bounds prove loads.
    unsafe {
        while i + 8 <= input.len() {
            a = _mm_fmadd_ps(
                _mm_loadu_ps(weights.as_ptr().add(i)),
                _mm_loadu_ps(input.as_ptr().add(i)),
                a,
            );
            b = _mm_fmadd_ps(
                _mm_loadu_ps(weights.as_ptr().add(i + 4)),
                _mm_loadu_ps(input.as_ptr().add(i + 4)),
                b,
            );
            i += 8;
        }
        let mut lanes = [0.0; 4];
        _mm_storeu_ps(lanes.as_mut_ptr(), _mm_add_ps(a, b));
        let mut sum = lanes[0] + lanes[1] + lanes[2] + lanes[3];
        while i < input.len() {
            sum = weights[i].mul_add(input[i], sum);
            i += 1;
        }
        sum
    }
}
#[cfg(feature = "engine-test-hooks")]
pub(crate) fn dot_product_f32_fma128(weights: &[f32], input: &[f32]) -> f32 {
    assert_eq!(weights.len(), input.len());
    // SAFETY: The plan admits FMA/SSE4.1, and equal slices prove load bounds.
    unsafe { f32_fma128(weights, input) }
}
