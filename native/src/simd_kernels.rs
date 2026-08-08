//! x86_64 SIMD kernels for Dense, MLP, and recurrent forward passes.
//!
//! These are native-only kernels used from Node via N-API. No scalar or non-x86 fallback exists.

use core::mem;

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("Native SIMD kernels require x86_64 (SSE). No scalar fallback is enabled.");

use napi::bindgen_prelude::{Float32Array, Int32Array};
use napi::{Error, Result, Status};
use napi_derive::napi;

/// Byte range occupied by one JavaScript typed-array view.
#[derive(Clone, Copy)]
struct BufferRange {
    name: &'static str,
    start: usize,
    end: usize,
}

/// Build a structured invalid-argument error for one exported kernel.
fn invalid_argument(kernel: &str, message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, format!("{kernel}: {}", message.into()))
}

/// Convert one strictly positive signed dimension to `usize`.
fn positive_dimension(kernel: &str, name: &str, value: i32) -> Result<usize> {
    if value <= 0 {
        return Err(invalid_argument(
            kernel,
            format!("{name} must be greater than zero"),
        ));
    }
    Ok(value as usize)
}

/// Multiply dimensions without allowing address-space overflow.
fn checked_product(kernel: &str, label: &str, values: &[usize]) -> Result<usize> {
    values.iter().try_fold(1usize, |product, value| {
        product.checked_mul(*value).ok_or_else(|| {
            invalid_argument(kernel, format!("{label} overflows addressable length"))
        })
    })
}

/// Add dimensions without allowing address-space overflow.
fn checked_sum(kernel: &str, label: &str, values: &[usize]) -> Result<usize> {
    values.iter().try_fold(0usize, |sum, value| {
        sum.checked_add(*value).ok_or_else(|| {
            invalid_argument(kernel, format!("{label} overflows addressable length"))
        })
    })
}

/// Compute the minimum buffer length for a strided batch.
fn required_strided_length(
    kernel: &str,
    count: usize,
    stride: usize,
    item_size: usize,
) -> Result<usize> {
    if stride < item_size {
        return Err(invalid_argument(
            kernel,
            format!("stride {stride} is smaller than item size {item_size}"),
        ));
    }
    let prior = checked_product(kernel, "strided batch length", &[count - 1, stride])?;
    checked_sum(kernel, "strided batch length", &[prior, item_size])
}

/// Require an array to contain at least the validated number of elements.
fn require_length(kernel: &str, name: &str, actual: usize, required: usize) -> Result<()> {
    if actual < required {
        return Err(invalid_argument(
            kernel,
            format!("{name} length {actual} is smaller than required {required}"),
        ));
    }
    Ok(())
}

/// Convert a typed-array slice into a checked byte range for alias validation.
fn buffer_range<T>(kernel: &str, name: &'static str, slice: &[T]) -> Result<BufferRange> {
    let start = slice.as_ptr() as usize;
    let byte_length = slice
        .len()
        .checked_mul(core::mem::size_of::<T>())
        .ok_or_else(|| invalid_argument(kernel, format!("{name} byte length overflows")))?;
    let end = start
        .checked_add(byte_length)
        .ok_or_else(|| invalid_argument(kernel, format!("{name} address range overflows")))?;
    Ok(BufferRange { name, start, end })
}

/// Return whether two nonempty byte ranges overlap.
fn ranges_overlap(left: BufferRange, right: BufferRange) -> bool {
    left.start < right.end && right.start < left.end
}

/// Reject writable buffers that overlap any other buffer used by the call.
fn reject_aliases(kernel: &str, writable: &[BufferRange], readonly: &[BufferRange]) -> Result<()> {
    for (index, left) in writable.iter().enumerate() {
        for right in writable.iter().skip(index + 1).chain(readonly.iter()) {
            if ranges_overlap(*left, *right) {
                return Err(invalid_argument(
                    kernel,
                    format!("{} overlaps {}", left.name, right.name),
                ));
            }
        }
    }
    Ok(())
}

/// Validated dimensions and buffer lengths shared by recurrent kernels.
struct RecurrentShape {
    required_weights: usize,
    required_inputs: usize,
    required_state: usize,
}

/// Validate recurrent dimensions, strides, and multiplication bounds.
fn recurrent_shape(
    kernel: &str,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
    gate_count: usize,
) -> Result<RecurrentShape> {
    let input_size = positive_dimension(kernel, "inSize", in_size)?;
    let hidden = positive_dimension(kernel, "hiddenSize", hidden_size)?;
    let count = positive_dimension(kernel, "batchCount", batch_count)?;
    let input_step = positive_dimension(kernel, "inputStride", input_stride)?;
    let unit_width = checked_sum(kernel, "recurrent unit width", &[input_size, hidden, 1])?;
    let required_weights =
        checked_product(kernel, "weights length", &[gate_count, hidden, unit_width])?;
    let required_inputs = required_strided_length(kernel, count, input_step, input_size)?;
    let required_state = checked_product(kernel, "state length", &[count, hidden])?;
    Ok(RecurrentShape {
        required_weights,
        required_inputs,
        required_state,
    })
}

/// Convert signed sizes into non-negative `usize` values.
fn to_usize(value: i32) -> usize {
    if value <= 0 {
        0
    } else {
        value as usize
    }
}

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

/// Sigmoid activation function.
#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Compute a Dense forward pass for a batch of inputs.
///
/// # Safety
///
/// The dimensions and derived offsets must not overflow. `weights_ptr` must be
/// valid for `out_size * (in_size + 1)` reads, each input row for `in_size`
/// reads at `input_stride`, and each output row for `output_stride` writes.
/// The writable output range must not overlap either input range.
#[allow(clippy::too_many_arguments)]
unsafe fn dense_forward(
    weights_ptr: *const f32,
    input_ptr: *const f32,
    output_ptr: *mut f32,
    in_size: i32,
    out_size: i32,
    batch_count: i32,
    input_stride: i32,
    output_stride: i32,
) {
    if weights_ptr.is_null() || input_ptr.is_null() || output_ptr.is_null() {
        return;
    }
    let in_size = to_usize(in_size);
    let out_size = to_usize(out_size);
    let batch_count = to_usize(batch_count);
    let input_stride = to_usize(input_stride);
    let output_stride = to_usize(output_stride);
    let out_limit = if out_size < output_stride {
        out_size
    } else {
        output_stride
    };

    // SAFETY: The caller guarantees the documented lengths, non-overlap, and
    // non-overflowing dimensions; every derived offset stays within them.
    unsafe {
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let output_base = b * output_stride;
            for o in 0..output_stride {
                *output_ptr.add(output_base + o) = 0.0;
            }
            let mut w_index = 0usize;
            for o in 0..out_limit {
                let sum = dense_dot(weights_ptr.add(w_index), input_ptr.add(input_base), in_size);
                let bias = *weights_ptr.add(w_index + in_size);
                w_index += in_size + 1;
                *output_ptr.add(output_base + o) = (sum + bias).tanh();
            }
        }
    }
}

/// Compute an MLP forward pass for a batch of inputs.
///
/// # Safety
///
/// Dimensions and all derived offsets must not overflow. `layer_sizes_ptr` must
/// be valid for `layer_count` positive sizes, weights must cover every layer,
/// inputs and outputs must cover their strided batches, and scratch must cover
/// twice the largest layer. Writable buffers must not overlap any argument.
#[allow(clippy::too_many_arguments)]
unsafe fn mlp_forward(
    weights_ptr: *const f32,
    layer_sizes_ptr: *const i32,
    input_ptr: *const f32,
    output_ptr: *mut f32,
    layer_count: i32,
    batch_count: i32,
    input_stride: i32,
    output_stride: i32,
    scratch_ptr: *mut f32,
    scratch_len: i32,
) {
    if weights_ptr.is_null()
        || layer_sizes_ptr.is_null()
        || input_ptr.is_null()
        || output_ptr.is_null()
    {
        return;
    }
    if scratch_ptr.is_null() {
        return;
    }
    let layer_count = to_usize(layer_count);
    if layer_count < 2 {
        return;
    }
    let batch_count = to_usize(batch_count);
    let input_stride = to_usize(input_stride);
    let output_stride = to_usize(output_stride);
    // SAFETY: The caller guarantees `layer_sizes_ptr` covers `layer_count`
    // elements. The exported wrapper validates this before calling.
    let layer_sizes = unsafe { core::slice::from_raw_parts(layer_sizes_ptr, layer_count) };
    let scratch_len = to_usize(scratch_len);

    let mut max_size = 0usize;
    for &size in layer_sizes {
        let size = to_usize(size);
        if size > max_size {
            max_size = size;
        }
    }
    if max_size == 0 {
        return;
    }
    if scratch_len < max_size * 2 {
        return;
    }
    // SAFETY: The caller guarantees `scratch_ptr` covers `scratch_len` mutable
    // elements and does not overlap any other buffer.
    let scratch = unsafe { core::slice::from_raw_parts_mut(scratch_ptr, scratch_len) };
    let (mut cur_buf, mut next_buf) = scratch.split_at_mut(max_size);

    // SAFETY: The caller guarantees all documented ranges, dimensions, and
    // non-overlap; the checked wrapper proves every offset used by this loop.
    unsafe {
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let input_size = to_usize(layer_sizes[0]);
            let input_slice = core::slice::from_raw_parts(input_ptr.add(input_base), input_size);
            cur_buf[..input_size].copy_from_slice(input_slice);

            let mut w_index = 0usize;
            for l in 0..(layer_count - 1) {
                let ins = to_usize(layer_sizes[l]);
                let outs = to_usize(layer_sizes[l + 1]);

                for out_val in next_buf.iter_mut().take(outs) {
                    let sum = dense_dot(weights_ptr.add(w_index), cur_buf.as_ptr(), ins);
                    let bias = *weights_ptr.add(w_index + ins);
                    w_index += ins + 1;
                    *out_val = (sum + bias).tanh();
                }
                mem::swap(&mut cur_buf, &mut next_buf);
            }
            let out_size = to_usize(layer_sizes[layer_count - 1]);
            let out_limit = if out_size < output_stride {
                out_size
            } else {
                output_stride
            };
            let output_base = b * output_stride;
            for o in 0..output_stride {
                *output_ptr.add(output_base + o) = 0.0;
            }

            for (o, &val) in cur_buf.iter().enumerate().take(out_limit) {
                *output_ptr.add(output_base + o) = val;
            }
        }
    }
}

/// Compute a GRU step for a batch of inputs.
///
/// # Safety
///
/// Dimensions and derived offsets must not overflow. Weights and inputs must
/// cover the recurrent shape, each state pointer must cover
/// `hidden_size * batch_count`, and writable ranges must be pairwise disjoint
/// and must not overlap weights or inputs.
#[allow(clippy::too_many_arguments)]
unsafe fn gru_step(
    weights_ptr: *const f32,
    input_ptr: *const f32,
    h_ptr: *mut f32,
    z_ptr: *mut f32,
    r_ptr: *mut f32,
    h_prev_ptr: *mut f32,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
) {
    if weights_ptr.is_null()
        || input_ptr.is_null()
        || h_ptr.is_null()
        || z_ptr.is_null()
        || r_ptr.is_null()
        || h_prev_ptr.is_null()
    {
        return;
    }
    let in_size = to_usize(in_size);
    let hidden_size = to_usize(hidden_size);
    let batch_count = to_usize(batch_count);
    let input_stride = to_usize(input_stride);
    if in_size == 0 || hidden_size == 0 || batch_count == 0 {
        return;
    }
    let wsz = hidden_size * in_size;
    let usz = hidden_size * hidden_size;
    let wz = 0usize;
    let wr = wz + wsz;
    let wh = wr + wsz;
    let uz = wh + wsz;
    let ur = uz + usz;
    let uh = ur + usz;
    let bz = uh + usz;
    let br = bz + hidden_size;
    let bh = br + hidden_size;

    // SAFETY: The caller guarantees all documented ranges and non-overlap; the
    // checked wrapper proves every derived pointer offset used by this loop.
    unsafe {
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let state_base = b * hidden_size;
            for j in 0..hidden_size {
                *h_prev_ptr.add(state_base + j) = *h_ptr.add(state_base + j);
            }
            for j in 0..hidden_size {
                let wz_row = wz + j * in_size;
                let wr_row = wr + j * in_size;
                let uz_row = uz + j * hidden_size;
                let ur_row = ur + j * hidden_size;
                let mut sum_z =
                    dense_dot(weights_ptr.add(wz_row), input_ptr.add(input_base), in_size);
                let mut sum_r =
                    dense_dot(weights_ptr.add(wr_row), input_ptr.add(input_base), in_size);
                sum_z += dense_dot(
                    weights_ptr.add(uz_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_r += dense_dot(
                    weights_ptr.add(ur_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_z += *weights_ptr.add(bz + j);
                sum_r += *weights_ptr.add(br + j);
                *z_ptr.add(state_base + j) = sigmoid(sum_z);
                *r_ptr.add(state_base + j) = sigmoid(sum_r);
            }
            for j in 0..hidden_size {
                let wh_row = wh + j * in_size;
                let uh_row = uh + j * hidden_size;
                let mut sum_h =
                    dense_dot(weights_ptr.add(wh_row), input_ptr.add(input_base), in_size);
                sum_h += dense_dot_mul(
                    weights_ptr.add(uh_row),
                    r_ptr.add(state_base),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_h += *weights_ptr.add(bh + j);
                let h_tilde = (sum_h).tanh();
                let z_val = *z_ptr.add(state_base + j);
                let prev_h = *h_prev_ptr.add(state_base + j);
                *h_ptr.add(state_base + j) = (1.0 - z_val) * prev_h + z_val * h_tilde;
            }
        }
    }
}

/// Compute an LSTM step for a batch of inputs.
///
/// # Safety
///
/// Dimensions and derived offsets must not overflow. Weights and inputs must
/// cover the recurrent shape, each state pointer must cover
/// `hidden_size * batch_count`, and writable ranges must be pairwise disjoint
/// and must not overlap weights or inputs.
#[allow(clippy::too_many_arguments)]
unsafe fn lstm_step(
    weights_ptr: *const f32,
    input_ptr: *const f32,
    h_ptr: *mut f32,
    c_ptr: *mut f32,
    h_prev_ptr: *mut f32,
    c_prev_ptr: *mut f32,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
) {
    if weights_ptr.is_null()
        || input_ptr.is_null()
        || h_ptr.is_null()
        || c_ptr.is_null()
        || h_prev_ptr.is_null()
        || c_prev_ptr.is_null()
    {
        return;
    }
    let in_size = to_usize(in_size);
    let hidden_size = to_usize(hidden_size);
    let batch_count = to_usize(batch_count);
    let input_stride = to_usize(input_stride);
    if in_size == 0 || hidden_size == 0 || batch_count == 0 {
        return;
    }
    let wsz = hidden_size * in_size;
    let usz = hidden_size * hidden_size;
    let wi = 0usize;
    let wf = wi + wsz;
    let wo = wf + wsz;
    let wg = wo + wsz;
    let ui = wg + wsz;
    let uf = ui + usz;
    let uo = uf + usz;
    let ug = uo + usz;
    let bi = ug + usz;
    let bf = bi + hidden_size;
    let bo = bf + hidden_size;
    let bg = bo + hidden_size;

    // SAFETY: The caller guarantees all documented ranges and non-overlap; the
    // checked wrapper proves every derived pointer offset used by this loop.
    unsafe {
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let state_base = b * hidden_size;
            for j in 0..hidden_size {
                *h_prev_ptr.add(state_base + j) = *h_ptr.add(state_base + j);
                *c_prev_ptr.add(state_base + j) = *c_ptr.add(state_base + j);
            }
            for j in 0..hidden_size {
                let wi_row = wi + j * in_size;
                let wf_row = wf + j * in_size;
                let wo_row = wo + j * in_size;
                let wg_row = wg + j * in_size;
                let ui_row = ui + j * hidden_size;
                let uf_row = uf + j * hidden_size;
                let uo_row = uo + j * hidden_size;
                let ug_row = ug + j * hidden_size;
                let mut sum_i =
                    dense_dot(weights_ptr.add(wi_row), input_ptr.add(input_base), in_size);
                let mut sum_f =
                    dense_dot(weights_ptr.add(wf_row), input_ptr.add(input_base), in_size);
                let mut sum_o =
                    dense_dot(weights_ptr.add(wo_row), input_ptr.add(input_base), in_size);
                let mut sum_g =
                    dense_dot(weights_ptr.add(wg_row), input_ptr.add(input_base), in_size);
                sum_i += dense_dot(
                    weights_ptr.add(ui_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_f += dense_dot(
                    weights_ptr.add(uf_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_o += dense_dot(
                    weights_ptr.add(uo_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_g += dense_dot(
                    weights_ptr.add(ug_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_i += *weights_ptr.add(bi + j);
                sum_f += *weights_ptr.add(bf + j);
                sum_o += *weights_ptr.add(bo + j);
                sum_g += *weights_ptr.add(bg + j);
                let i_gate = sigmoid(sum_i);
                let f_gate = sigmoid(sum_f);
                let o_gate = sigmoid(sum_o);
                let g_gate = (sum_g).tanh();
                let prev_c = *c_prev_ptr.add(state_base + j);
                let next_c = f_gate * prev_c + i_gate * g_gate;
                *c_ptr.add(state_base + j) = next_c;
                *h_ptr.add(state_base + j) = o_gate * (next_c).tanh();
            }
        }
    }
}

/// Compute an RRU step for a batch of inputs.
///
/// # Safety
///
/// Dimensions and derived offsets must not overflow. Weights and inputs must
/// cover the recurrent shape, each state pointer must cover
/// `hidden_size * batch_count`, and writable ranges must be pairwise disjoint
/// and must not overlap weights or inputs.
#[allow(clippy::too_many_arguments)]
unsafe fn rru_step(
    weights_ptr: *const f32,
    input_ptr: *const f32,
    h_ptr: *mut f32,
    h_prev_ptr: *mut f32,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
) {
    if weights_ptr.is_null() || input_ptr.is_null() || h_ptr.is_null() || h_prev_ptr.is_null() {
        return;
    }
    let in_size = to_usize(in_size);
    let hidden_size = to_usize(hidden_size);
    let batch_count = to_usize(batch_count);
    let input_stride = to_usize(input_stride);
    if in_size == 0 || hidden_size == 0 || batch_count == 0 {
        return;
    }
    let wsz = hidden_size * in_size;
    let usz = hidden_size * hidden_size;
    let wc = 0usize;
    let wr = wc + wsz;
    let uc = wr + wsz;
    let ur = uc + usz;
    let bc = ur + usz;
    let br = bc + hidden_size;

    // SAFETY: The caller guarantees all documented ranges and non-overlap; the
    // checked wrapper proves every derived pointer offset used by this loop.
    unsafe {
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let state_base = b * hidden_size;
            for j in 0..hidden_size {
                *h_prev_ptr.add(state_base + j) = *h_ptr.add(state_base + j);
            }
            for j in 0..hidden_size {
                let wc_row = wc + j * in_size;
                let wr_row = wr + j * in_size;
                let uc_row = uc + j * hidden_size;
                let ur_row = ur + j * hidden_size;
                let mut sum_c =
                    dense_dot(weights_ptr.add(wc_row), input_ptr.add(input_base), in_size);
                let mut sum_r =
                    dense_dot(weights_ptr.add(wr_row), input_ptr.add(input_base), in_size);
                sum_c += dense_dot(
                    weights_ptr.add(uc_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_r += dense_dot(
                    weights_ptr.add(ur_row),
                    h_prev_ptr.add(state_base),
                    hidden_size,
                );
                sum_c += *weights_ptr.add(bc + j);
                sum_r += *weights_ptr.add(br + j);
                let cand = (sum_c).tanh();
                let gate = sigmoid(sum_r);
                let prev = *h_prev_ptr.add(state_base + j);
                *h_ptr.add(state_base + j) = (1.0 - gate) * prev + gate * cand;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[napi(js_name = "denseForwardNative")]
pub fn dense_forward_native(
    weights: Float32Array,
    inputs: Float32Array,
    mut outputs: Float32Array,
    in_size: i32,
    out_size: i32,
    batch_count: i32,
    input_stride: i32,
    output_stride: i32,
) -> Result<()> {
    const KERNEL: &str = "denseForwardNative";
    let in_len = positive_dimension(KERNEL, "inSize", in_size)?;
    let out_len = positive_dimension(KERNEL, "outSize", out_size)?;
    let count = positive_dimension(KERNEL, "batchCount", batch_count)?;
    let input_step = positive_dimension(KERNEL, "inputStride", input_stride)?;
    let output_step = positive_dimension(KERNEL, "outputStride", output_stride)?;
    let weight_row = checked_sum(KERNEL, "weight row", &[in_len, 1])?;
    let required_weights = checked_product(KERNEL, "weights length", &[out_len, weight_row])?;
    let required_inputs = required_strided_length(KERNEL, count, input_step, in_len)?;
    let required_outputs = required_strided_length(KERNEL, count, output_step, output_step)?;
    require_length(KERNEL, "weights", weights.len(), required_weights)?;
    require_length(KERNEL, "inputs", inputs.len(), required_inputs)?;
    require_length(KERNEL, "outputs", outputs.len(), required_outputs)?;
    reject_aliases(
        KERNEL,
        &[buffer_range(KERNEL, "outputs", outputs.as_ref())?],
        &[
            buffer_range(KERNEL, "weights", weights.as_ref())?,
            buffer_range(KERNEL, "inputs", inputs.as_ref())?,
        ],
    )?;
    let weights = weights.as_ref();
    let inputs = inputs.as_ref();
    // SAFETY: Aliasing and the complete mutable output range were validated above.
    let outputs = unsafe { outputs.as_mut() };
    // SAFETY: Positive dimensions, strides, lengths, and multiplication bounds were validated above.
    unsafe {
        dense_forward(
            weights.as_ptr(),
            inputs.as_ptr(),
            outputs.as_mut_ptr(),
            in_size,
            out_size,
            batch_count,
            input_stride,
            output_stride,
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[napi(js_name = "mlpForwardNative")]
pub fn mlp_forward_native(
    weights: Float32Array,
    layer_sizes: Int32Array,
    inputs: Float32Array,
    mut outputs: Float32Array,
    layer_count: i32,
    batch_count: i32,
    input_stride: i32,
    output_stride: i32,
    mut scratch: Float32Array,
) -> Result<()> {
    const KERNEL: &str = "mlpForwardNative";
    let layers = positive_dimension(KERNEL, "layerCount", layer_count)?;
    if layers < 2 {
        return Err(invalid_argument(KERNEL, "layerCount must be at least two"));
    }
    if layer_sizes.len() != layers {
        return Err(invalid_argument(
            KERNEL,
            format!(
                "layerSizes length {} does not equal layerCount {layers}",
                layer_sizes.len()
            ),
        ));
    }
    let count = positive_dimension(KERNEL, "batchCount", batch_count)?;
    let input_step = positive_dimension(KERNEL, "inputStride", input_stride)?;
    let output_step = positive_dimension(KERNEL, "outputStride", output_stride)?;
    let layer_values = layer_sizes.as_ref();
    let mut sizes = Vec::with_capacity(layers);
    let mut max_size = 0usize;
    for (index, value) in layer_values.iter().enumerate() {
        let size = positive_dimension(KERNEL, &format!("layerSizes[{index}]"), *value)?;
        max_size = max_size.max(size);
        sizes.push(size);
    }
    let mut required_weights = 0usize;
    for pair in sizes.windows(2) {
        let row = checked_sum(KERNEL, "MLP weight row", &[pair[0], 1])?;
        let layer_weights = checked_product(KERNEL, "MLP layer weights", &[pair[1], row])?;
        required_weights = checked_sum(
            KERNEL,
            "MLP weights length",
            &[required_weights, layer_weights],
        )?;
    }
    let input_size = sizes[0];
    let output_size = sizes[layers - 1];
    let required_inputs = required_strided_length(KERNEL, count, input_step, input_size)?;
    let required_outputs = required_strided_length(KERNEL, count, output_step, output_step)?;
    if output_step < output_size {
        return Err(invalid_argument(
            KERNEL,
            format!("outputStride {output_step} is smaller than output size {output_size}"),
        ));
    }
    let required_scratch = checked_product(KERNEL, "scratch length", &[max_size, 2])?;
    let scratch_len_i32 = i32::try_from(required_scratch)
        .map_err(|_| invalid_argument(KERNEL, "scratch length exceeds the native ABI"))?;
    require_length(KERNEL, "weights", weights.len(), required_weights)?;
    require_length(KERNEL, "inputs", inputs.len(), required_inputs)?;
    require_length(KERNEL, "outputs", outputs.len(), required_outputs)?;
    require_length(KERNEL, "scratch", scratch.len(), required_scratch)?;
    reject_aliases(
        KERNEL,
        &[
            buffer_range(KERNEL, "outputs", outputs.as_ref())?,
            buffer_range(KERNEL, "scratch", scratch.as_ref())?,
        ],
        &[
            buffer_range(KERNEL, "weights", weights.as_ref())?,
            buffer_range(KERNEL, "layerSizes", layer_sizes.as_ref())?,
            buffer_range(KERNEL, "inputs", inputs.as_ref())?,
        ],
    )?;
    let weights = weights.as_ref();
    let layer_sizes = layer_sizes.as_ref();
    let inputs = inputs.as_ref();
    // SAFETY: Writable buffer ranges are complete and disjoint from every other argument.
    let outputs = unsafe { outputs.as_mut() };
    // SAFETY: Writable buffer ranges are complete and disjoint from every other argument.
    let scratch = unsafe { scratch.as_mut() };
    // SAFETY: Dimensions, strides, weights, scratch, output, and input ranges were validated above.
    unsafe {
        mlp_forward(
            weights.as_ptr(),
            layer_sizes.as_ptr(),
            inputs.as_ptr(),
            outputs.as_mut_ptr(),
            layer_count,
            batch_count,
            input_stride,
            output_stride,
            scratch.as_mut_ptr(),
            scratch_len_i32,
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[napi(js_name = "gruStepNative")]
pub fn gru_step_native(
    weights: Float32Array,
    inputs: Float32Array,
    mut h: Float32Array,
    mut z: Float32Array,
    mut r: Float32Array,
    mut h_prev: Float32Array,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
) -> Result<()> {
    const KERNEL: &str = "gruStepNative";
    let shape = recurrent_shape(KERNEL, in_size, hidden_size, batch_count, input_stride, 3)?;
    require_length(KERNEL, "weights", weights.len(), shape.required_weights)?;
    require_length(KERNEL, "inputs", inputs.len(), shape.required_inputs)?;
    require_length(KERNEL, "h", h.len(), shape.required_state)?;
    require_length(KERNEL, "z", z.len(), shape.required_state)?;
    require_length(KERNEL, "r", r.len(), shape.required_state)?;
    require_length(KERNEL, "hPrev", h_prev.len(), shape.required_state)?;
    reject_aliases(
        KERNEL,
        &[
            buffer_range(KERNEL, "h", h.as_ref())?,
            buffer_range(KERNEL, "z", z.as_ref())?,
            buffer_range(KERNEL, "r", r.as_ref())?,
            buffer_range(KERNEL, "hPrev", h_prev.as_ref())?,
        ],
        &[
            buffer_range(KERNEL, "weights", weights.as_ref())?,
            buffer_range(KERNEL, "inputs", inputs.as_ref())?,
        ],
    )?;
    let weights = weights.as_ref();
    let inputs = inputs.as_ref();
    // SAFETY: The complete mutable state range was validated and is disjoint.
    let h = unsafe { h.as_mut() };
    // SAFETY: The complete mutable scratch range was validated and is disjoint.
    let z = unsafe { z.as_mut() };
    // SAFETY: The complete mutable scratch range was validated and is disjoint.
    let r = unsafe { r.as_mut() };
    // SAFETY: The complete mutable previous-state range was validated and is disjoint.
    let h_prev = unsafe { h_prev.as_mut() };
    // SAFETY: All pointer ranges, dimensions, strides, and parameter lengths were validated above.
    unsafe {
        gru_step(
            weights.as_ptr(),
            inputs.as_ptr(),
            h.as_mut_ptr(),
            z.as_mut_ptr(),
            r.as_mut_ptr(),
            h_prev.as_mut_ptr(),
            in_size,
            hidden_size,
            batch_count,
            input_stride,
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[napi(js_name = "lstmStepNative")]
pub fn lstm_step_native(
    weights: Float32Array,
    inputs: Float32Array,
    mut h: Float32Array,
    mut c: Float32Array,
    mut h_prev: Float32Array,
    mut c_prev: Float32Array,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
) -> Result<()> {
    const KERNEL: &str = "lstmStepNative";
    let shape = recurrent_shape(KERNEL, in_size, hidden_size, batch_count, input_stride, 4)?;
    require_length(KERNEL, "weights", weights.len(), shape.required_weights)?;
    require_length(KERNEL, "inputs", inputs.len(), shape.required_inputs)?;
    require_length(KERNEL, "h", h.len(), shape.required_state)?;
    require_length(KERNEL, "c", c.len(), shape.required_state)?;
    require_length(KERNEL, "hPrev", h_prev.len(), shape.required_state)?;
    require_length(KERNEL, "cPrev", c_prev.len(), shape.required_state)?;
    reject_aliases(
        KERNEL,
        &[
            buffer_range(KERNEL, "h", h.as_ref())?,
            buffer_range(KERNEL, "c", c.as_ref())?,
            buffer_range(KERNEL, "hPrev", h_prev.as_ref())?,
            buffer_range(KERNEL, "cPrev", c_prev.as_ref())?,
        ],
        &[
            buffer_range(KERNEL, "weights", weights.as_ref())?,
            buffer_range(KERNEL, "inputs", inputs.as_ref())?,
        ],
    )?;
    let weights = weights.as_ref();
    let inputs = inputs.as_ref();
    // SAFETY: The complete mutable hidden-state range was validated and is disjoint.
    let h = unsafe { h.as_mut() };
    // SAFETY: The complete mutable cell-state range was validated and is disjoint.
    let c = unsafe { c.as_mut() };
    // SAFETY: The complete mutable previous-hidden range was validated and is disjoint.
    let h_prev = unsafe { h_prev.as_mut() };
    // SAFETY: The complete mutable previous-cell range was validated and is disjoint.
    let c_prev = unsafe { c_prev.as_mut() };
    // SAFETY: All pointer ranges, dimensions, strides, and parameter lengths were validated above.
    unsafe {
        lstm_step(
            weights.as_ptr(),
            inputs.as_ptr(),
            h.as_mut_ptr(),
            c.as_mut_ptr(),
            h_prev.as_mut_ptr(),
            c_prev.as_mut_ptr(),
            in_size,
            hidden_size,
            batch_count,
            input_stride,
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[napi(js_name = "rruStepNative")]
pub fn rru_step_native(
    weights: Float32Array,
    inputs: Float32Array,
    mut h: Float32Array,
    mut h_prev: Float32Array,
    in_size: i32,
    hidden_size: i32,
    batch_count: i32,
    input_stride: i32,
) -> Result<()> {
    const KERNEL: &str = "rruStepNative";
    let shape = recurrent_shape(KERNEL, in_size, hidden_size, batch_count, input_stride, 2)?;
    require_length(KERNEL, "weights", weights.len(), shape.required_weights)?;
    require_length(KERNEL, "inputs", inputs.len(), shape.required_inputs)?;
    require_length(KERNEL, "h", h.len(), shape.required_state)?;
    require_length(KERNEL, "hPrev", h_prev.len(), shape.required_state)?;
    reject_aliases(
        KERNEL,
        &[
            buffer_range(KERNEL, "h", h.as_ref())?,
            buffer_range(KERNEL, "hPrev", h_prev.as_ref())?,
        ],
        &[
            buffer_range(KERNEL, "weights", weights.as_ref())?,
            buffer_range(KERNEL, "inputs", inputs.as_ref())?,
        ],
    )?;
    let weights = weights.as_ref();
    let inputs = inputs.as_ref();
    // SAFETY: The complete mutable hidden-state range was validated and is disjoint.
    let h = unsafe { h.as_mut() };
    // SAFETY: The complete mutable previous-state range was validated and is disjoint.
    let h_prev = unsafe { h_prev.as_mut() };
    // SAFETY: All pointer ranges, dimensions, strides, and parameter lengths were validated above.
    unsafe {
        rru_step(
            weights.as_ptr(),
            inputs.as_ptr(),
            h.as_mut_ptr(),
            h_prev.as_mut_ptr(),
            in_size,
            hidden_size,
            batch_count,
            input_stride,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Keep the reference helper's arguments aligned with the kernel contract
    // so each dimension and stride remains explicit in the differential test.
    #[allow(clippy::too_many_arguments)]
    fn dense_forward_reference(
        weights: &[f32],
        inputs: &[f32],
        outputs: &mut [f32],
        in_size: usize,
        out_size: usize,
        batch_count: usize,
        input_stride: usize,
        output_stride: usize,
    ) {
        let out_limit = out_size.min(output_stride);
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let output_base = b * output_stride;
            for o in 0..output_stride {
                outputs[output_base + o] = 0.0;
            }
            let mut w_index = 0usize;
            for o in 0..out_limit {
                let mut sum = 0.0_f32;
                for i in 0..in_size {
                    sum += weights[w_index + i] * inputs[input_base + i];
                }
                let bias = weights[w_index + in_size];
                w_index += in_size + 1;
                outputs[output_base + o] = (sum + bias).tanh();
            }
        }
    }

    fn mlp_forward_reference(
        weights: &[f32],
        layer_sizes: &[i32],
        inputs: &[f32],
        outputs: &mut [f32],
        batch_count: usize,
        input_stride: usize,
        output_stride: usize,
    ) {
        let layer_count = layer_sizes.len();
        if layer_count < 2 {
            return;
        }
        let mut max_size = 0usize;
        for &size in layer_sizes {
            max_size = max_size.max(to_usize(size));
        }
        let mut buf_a = vec![0.0_f32; max_size.max(1)];
        let mut buf_b = vec![0.0_f32; max_size.max(1)];
        for b in 0..batch_count {
            let input_base = b * input_stride;
            let input_size = to_usize(layer_sizes[0]);
            buf_a[..input_size].copy_from_slice(&inputs[input_base..input_base + input_size]);
            let mut cur = &mut buf_a;
            let mut next = &mut buf_b;
            let mut w_index = 0usize;
            for l in 1..layer_count {
                let in_size = to_usize(layer_sizes[l - 1]);
                let out_size = to_usize(layer_sizes[l]);
                for output in next.iter_mut().take(out_size) {
                    let mut sum = 0.0_f32;
                    for i in 0..in_size {
                        sum += weights[w_index + i] * cur[i];
                    }
                    let bias = weights[w_index + in_size];
                    w_index += in_size + 1;
                    *output = (sum + bias).tanh();
                }
                mem::swap(&mut cur, &mut next);
            }
            let out_size = to_usize(layer_sizes[layer_count - 1]);
            let out_limit = out_size.min(output_stride);
            let output_base = b * output_stride;
            for o in 0..output_stride {
                outputs[output_base + o] = 0.0;
            }
            outputs[output_base..output_base + out_limit].copy_from_slice(&cur[..out_limit]);
        }
    }

    #[test]
    fn dense_forward_matches_reference() {
        let in_size = 3;
        let out_size = 2;
        let count = 2;
        let input_stride = 3;
        let output_stride = 2;
        let weights = vec![
            0.1, 0.2, 0.3, 0.01, // o0
            -0.2, 0.4, 0.05, -0.03, // o1
        ];
        let inputs = vec![1.0, -2.0, 0.5, 0.25, 0.75, -1.5];
        let mut out_native = vec![0.0_f32; count * output_stride];
        let mut out_ref = vec![0.0_f32; count * output_stride];
        // SAFETY: These owned vectors exactly satisfy the tested dense shape,
        // stride, length, and non-overlap contract.
        unsafe {
            dense_forward(
                weights.as_ptr(),
                inputs.as_ptr(),
                out_native.as_mut_ptr(),
                in_size as i32,
                out_size as i32,
                count as i32,
                input_stride as i32,
                output_stride as i32,
            );
        }
        dense_forward_reference(
            &weights,
            &inputs,
            &mut out_ref,
            in_size,
            out_size,
            count,
            input_stride,
            output_stride,
        );
        for i in 0..out_native.len() {
            assert!((out_native[i] - out_ref[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn mlp_forward_matches_reference() {
        let layer_sizes = vec![3, 4, 2];
        let count = 2;
        let input_stride = 3;
        let output_stride = 2;
        let param_count = (3 + 1) * 4 + (4 + 1) * 2;
        let mut weights = Vec::with_capacity(param_count);
        for i in 0..param_count {
            weights.push((i as f32 * 0.01) - 0.2);
        }
        let inputs = vec![1.0, -2.0, 0.5, 0.25, 0.75, -1.5];
        let mut out_native = vec![0.0_f32; count * output_stride];
        let mut out_ref = vec![0.0_f32; count * output_stride];
        let mut scratch = vec![0.0_f32; 8];
        // SAFETY: These owned vectors exactly satisfy the tested MLP shape,
        // stride, scratch, length, and non-overlap contract.
        unsafe {
            mlp_forward(
                weights.as_ptr(),
                layer_sizes.as_ptr(),
                inputs.as_ptr(),
                out_native.as_mut_ptr(),
                layer_sizes.len() as i32,
                count as i32,
                input_stride as i32,
                output_stride as i32,
                scratch.as_mut_ptr(),
                scratch.len() as i32,
            );
        }
        mlp_forward_reference(
            &weights,
            &layer_sizes,
            &inputs,
            &mut out_ref,
            count,
            input_stride,
            output_stride,
        );
        for i in 0..out_native.len() {
            assert!((out_native[i] - out_ref[i]).abs() < 1e-6);
        }
    }

    // This mirrors the GRU kernel's explicit state and scratch-buffer surface.
    #[allow(clippy::too_many_arguments)]
    fn gru_step_reference(
        weights: &[f32],
        inputs: &[f32],
        h: &mut [f32],
        z: &mut [f32],
        r: &mut [f32],
        h_prev: &mut [f32],
        in_size: usize,
        hidden_size: usize,
        batch_count: usize,
        input_stride: usize,
    ) {
        let wsz = hidden_size * in_size;
        let usz = hidden_size * hidden_size;
        let wz = 0usize;
        let wr = wz + wsz;
        let wh = wr + wsz;
        let uz = wh + wsz;
        let ur = uz + usz;
        let uh = ur + usz;
        let bz = uh + usz;
        let br = bz + hidden_size;
        let bh = br + hidden_size;

        for b in 0..batch_count {
            let input_base = b * input_stride;
            let state_base = b * hidden_size;
            h_prev[state_base..state_base + hidden_size]
                .copy_from_slice(&h[state_base..state_base + hidden_size]);
            for j in 0..hidden_size {
                let mut sum_z = 0.0_f32;
                let mut sum_r = 0.0_f32;
                let wz_row = wz + j * in_size;
                let wr_row = wr + j * in_size;
                let uz_row = uz + j * hidden_size;
                let ur_row = ur + j * hidden_size;
                for i in 0..in_size {
                    sum_z += weights[wz_row + i] * inputs[input_base + i];
                    sum_r += weights[wr_row + i] * inputs[input_base + i];
                }
                for i in 0..hidden_size {
                    sum_z += weights[uz_row + i] * h_prev[state_base + i];
                    sum_r += weights[ur_row + i] * h_prev[state_base + i];
                }
                sum_z += weights[bz + j];
                sum_r += weights[br + j];
                z[state_base + j] = sigmoid(sum_z);
                r[state_base + j] = sigmoid(sum_r);
            }
            for j in 0..hidden_size {
                let wh_row = wh + j * in_size;
                let uh_row = uh + j * hidden_size;
                let mut sum_h = 0.0_f32;
                for i in 0..in_size {
                    sum_h += weights[wh_row + i] * inputs[input_base + i];
                }
                for i in 0..hidden_size {
                    sum_h += weights[uh_row + i] * (r[state_base + i] * h_prev[state_base + i]);
                }
                sum_h += weights[bh + j];
                let h_tilde = sum_h.tanh();
                let z_val = z[state_base + j];
                let prev = h_prev[state_base + j];
                h[state_base + j] = (1.0 - z_val) * prev + z_val * h_tilde;
            }
        }
    }

    // This mirrors the LSTM kernel's explicit hidden/cell state contract.
    #[allow(clippy::too_many_arguments)]
    fn lstm_step_reference(
        weights: &[f32],
        inputs: &[f32],
        h: &mut [f32],
        c: &mut [f32],
        h_prev: &mut [f32],
        c_prev: &mut [f32],
        in_size: usize,
        hidden_size: usize,
        batch_count: usize,
        input_stride: usize,
    ) {
        let wsz = hidden_size * in_size;
        let usz = hidden_size * hidden_size;
        let wi = 0usize;
        let wf = wi + wsz;
        let wo = wf + wsz;
        let wg = wo + wsz;
        let ui = wg + wsz;
        let uf = ui + usz;
        let uo = uf + usz;
        let ug = uo + usz;
        let bi = ug + usz;
        let bf = bi + hidden_size;
        let bo = bf + hidden_size;
        let bg = bo + hidden_size;

        for b in 0..batch_count {
            let input_base = b * input_stride;
            let state_base = b * hidden_size;
            h_prev[state_base..state_base + hidden_size]
                .copy_from_slice(&h[state_base..state_base + hidden_size]);
            c_prev[state_base..state_base + hidden_size]
                .copy_from_slice(&c[state_base..state_base + hidden_size]);
            for j in 0..hidden_size {
                let wi_row = wi + j * in_size;
                let wf_row = wf + j * in_size;
                let wo_row = wo + j * in_size;
                let wg_row = wg + j * in_size;
                let ui_row = ui + j * hidden_size;
                let uf_row = uf + j * hidden_size;
                let uo_row = uo + j * hidden_size;
                let ug_row = ug + j * hidden_size;
                let mut sum_i = 0.0_f32;
                let mut sum_f = 0.0_f32;
                let mut sum_o = 0.0_f32;
                let mut sum_g = 0.0_f32;
                for i in 0..in_size {
                    let x = inputs[input_base + i];
                    sum_i += weights[wi_row + i] * x;
                    sum_f += weights[wf_row + i] * x;
                    sum_o += weights[wo_row + i] * x;
                    sum_g += weights[wg_row + i] * x;
                }
                for i in 0..hidden_size {
                    let prev = h_prev[state_base + i];
                    sum_i += weights[ui_row + i] * prev;
                    sum_f += weights[uf_row + i] * prev;
                    sum_o += weights[uo_row + i] * prev;
                    sum_g += weights[ug_row + i] * prev;
                }
                sum_i += weights[bi + j];
                sum_f += weights[bf + j];
                sum_o += weights[bo + j];
                sum_g += weights[bg + j];
                let i_gate = sigmoid(sum_i);
                let f_gate = sigmoid(sum_f);
                let o_gate = sigmoid(sum_o);
                let g_gate = sum_g.tanh();
                let prev_c = c_prev[state_base + j];
                let next_c = f_gate * prev_c + i_gate * g_gate;
                c[state_base + j] = next_c;
                h[state_base + j] = o_gate * next_c.tanh();
            }
        }
    }

    // This mirrors the RRU kernel's explicit state and scratch-buffer surface.
    #[allow(clippy::too_many_arguments)]
    fn rru_step_reference(
        weights: &[f32],
        inputs: &[f32],
        h: &mut [f32],
        h_prev: &mut [f32],
        in_size: usize,
        hidden_size: usize,
        batch_count: usize,
        input_stride: usize,
    ) {
        let wsz = hidden_size * in_size;
        let usz = hidden_size * hidden_size;
        let wc = 0usize;
        let wr = wc + wsz;
        let uc = wr + wsz;
        let ur = uc + usz;
        let bc = ur + usz;
        let br = bc + hidden_size;

        for b in 0..batch_count {
            let input_base = b * input_stride;
            let state_base = b * hidden_size;
            h_prev[state_base..state_base + hidden_size]
                .copy_from_slice(&h[state_base..state_base + hidden_size]);
            for j in 0..hidden_size {
                let wc_row = wc + j * in_size;
                let wr_row = wr + j * in_size;
                let uc_row = uc + j * hidden_size;
                let ur_row = ur + j * hidden_size;
                let mut sum_c = 0.0_f32;
                let mut sum_r = 0.0_f32;
                for i in 0..in_size {
                    let x = inputs[input_base + i];
                    sum_c += weights[wc_row + i] * x;
                    sum_r += weights[wr_row + i] * x;
                }
                for i in 0..hidden_size {
                    let prev = h_prev[state_base + i];
                    sum_c += weights[uc_row + i] * prev;
                    sum_r += weights[ur_row + i] * prev;
                }
                sum_c += weights[bc + j];
                sum_r += weights[br + j];
                let cand = sum_c.tanh();
                let gate = sigmoid(sum_r);
                let prev = h_prev[state_base + j];
                h[state_base + j] = (1.0 - gate) * prev + gate * cand;
            }
        }
    }

    fn assert_all_close(label: &str, a: &[f32], b: &[f32]) {
        assert_eq!(a.len(), b.len(), "{label} length mismatch");
        for i in 0..a.len() {
            let diff = (a[i] - b[i]).abs();
            assert!(diff < 1e-6, "{label} idx {i} diff {diff}");
        }
    }

    #[test]
    fn recurrent_steps_match_reference() {
        let in_size = 2usize;
        let hidden_size = 2usize;
        let batch_count = 2usize;
        let input_stride = 2usize;

        let gru_weight_count =
            3 * (hidden_size * in_size + hidden_size * hidden_size + hidden_size);
        let mut gru_weights = Vec::with_capacity(gru_weight_count);
        for i in 0..gru_weight_count {
            gru_weights.push((i as f32 * 0.01) - 0.15);
        }
        let inputs = vec![0.5, -1.0, 1.5, 0.25];
        let mut h = vec![0.2, -0.1, 0.05, -0.2];
        let mut z = vec![0.0; hidden_size * batch_count];
        let mut r = vec![0.0; hidden_size * batch_count];
        let mut h_prev = vec![0.0; hidden_size * batch_count];
        let mut h_ref = h.clone();
        let mut z_ref = z.clone();
        let mut r_ref = r.clone();
        let mut h_prev_ref = h_prev.clone();
        // SAFETY: These owned vectors exactly satisfy the tested GRU shape,
        // stride, state, length, and non-overlap contract.
        unsafe {
            gru_step(
                gru_weights.as_ptr(),
                inputs.as_ptr(),
                h.as_mut_ptr(),
                z.as_mut_ptr(),
                r.as_mut_ptr(),
                h_prev.as_mut_ptr(),
                in_size as i32,
                hidden_size as i32,
                batch_count as i32,
                input_stride as i32,
            );
        }
        gru_step_reference(
            &gru_weights,
            &inputs,
            &mut h_ref,
            &mut z_ref,
            &mut r_ref,
            &mut h_prev_ref,
            in_size,
            hidden_size,
            batch_count,
            input_stride,
        );
        assert_all_close("gru_h", &h, &h_ref);
        assert_all_close("gru_z", &z, &z_ref);
        assert_all_close("gru_r", &r, &r_ref);
        assert_all_close("gru_prev", &h_prev, &h_prev_ref);

        let lstm_weight_count =
            4 * (hidden_size * in_size + hidden_size * hidden_size + hidden_size);
        let mut lstm_weights = Vec::with_capacity(lstm_weight_count);
        for i in 0..lstm_weight_count {
            lstm_weights.push((i as f32 * 0.008) - 0.12);
        }
        let mut h_l = vec![0.1, 0.2, -0.1, 0.05];
        let mut c_l = vec![0.02, -0.03, 0.01, -0.02];
        let mut h_prev_l = vec![0.0; hidden_size * batch_count];
        let mut c_prev_l = vec![0.0; hidden_size * batch_count];
        let mut h_l_ref = h_l.clone();
        let mut c_l_ref = c_l.clone();
        let mut h_prev_l_ref = h_prev_l.clone();
        let mut c_prev_l_ref = c_prev_l.clone();
        // SAFETY: These owned vectors exactly satisfy the tested LSTM shape,
        // stride, state, length, and non-overlap contract.
        unsafe {
            lstm_step(
                lstm_weights.as_ptr(),
                inputs.as_ptr(),
                h_l.as_mut_ptr(),
                c_l.as_mut_ptr(),
                h_prev_l.as_mut_ptr(),
                c_prev_l.as_mut_ptr(),
                in_size as i32,
                hidden_size as i32,
                batch_count as i32,
                input_stride as i32,
            );
        }
        lstm_step_reference(
            &lstm_weights,
            &inputs,
            &mut h_l_ref,
            &mut c_l_ref,
            &mut h_prev_l_ref,
            &mut c_prev_l_ref,
            in_size,
            hidden_size,
            batch_count,
            input_stride,
        );
        assert_all_close("lstm_h", &h_l, &h_l_ref);
        assert_all_close("lstm_c", &c_l, &c_l_ref);
        assert_all_close("lstm_prev_h", &h_prev_l, &h_prev_l_ref);
        assert_all_close("lstm_prev_c", &c_prev_l, &c_prev_l_ref);

        let rru_weight_count =
            2 * (hidden_size * in_size + hidden_size * hidden_size + hidden_size);
        let mut rru_weights = Vec::with_capacity(rru_weight_count);
        for i in 0..rru_weight_count {
            rru_weights.push((i as f32 * 0.012) - 0.08);
        }
        let mut h_r = vec![0.15, -0.05, 0.2, -0.1];
        let mut h_prev_r = vec![0.0; hidden_size * batch_count];
        let mut h_r_ref = h_r.clone();
        let mut h_prev_r_ref = h_prev_r.clone();
        // SAFETY: These owned vectors exactly satisfy the tested RRU shape,
        // stride, state, length, and non-overlap contract.
        unsafe {
            rru_step(
                rru_weights.as_ptr(),
                inputs.as_ptr(),
                h_r.as_mut_ptr(),
                h_prev_r.as_mut_ptr(),
                in_size as i32,
                hidden_size as i32,
                batch_count as i32,
                input_stride as i32,
            );
        }
        rru_step_reference(
            &rru_weights,
            &inputs,
            &mut h_r_ref,
            &mut h_prev_r_ref,
            in_size,
            hidden_size,
            batch_count,
            input_stride,
        );
        assert_all_close("rru_h", &h_r, &h_r_ref);
        assert_all_close("rru_prev", &h_prev_r, &h_prev_r_ref);
    }
}
