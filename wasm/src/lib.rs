//! WASM kernels for Dense, MLP, and recurrent forward passes.

use core::arch::wasm32::*;
use core::mem;

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
    let mut sum = f32x4_splat(0.0);
    // Safety: Caller guarantees pointers are valid for in_size
    unsafe {
        while i + 4 <= in_size {
            let w = v128_load(weights_ptr.add(i) as *const v128);
            let x = v128_load(input_ptr.add(i) as *const v128);
            sum = f32x4_add(sum, f32x4_mul(w, x));
            i += 4;
        }
        let mut total = f32x4_extract_lane::<0>(sum)
            + f32x4_extract_lane::<1>(sum)
            + f32x4_extract_lane::<2>(sum)
            + f32x4_extract_lane::<3>(sum);
        while i < in_size {
            total += *weights_ptr.add(i) * *input_ptr.add(i);
            i += 1;
        }
        total
    }
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
    let mut sum = f32x4_splat(0.0);
    // Safety: Caller guarantees pointers are valid for len
    unsafe {
        while i + 4 <= len {
            let w = v128_load(weights_ptr.add(i) as *const v128);
            let a = v128_load(a_ptr.add(i) as *const v128);
            let b = v128_load(b_ptr.add(i) as *const v128);
            let ab = f32x4_mul(a, b);
            sum = f32x4_add(sum, f32x4_mul(w, ab));
            i += 4;
        }
        let mut total = f32x4_extract_lane::<0>(sum)
            + f32x4_extract_lane::<1>(sum)
            + f32x4_extract_lane::<2>(sum)
            + f32x4_extract_lane::<3>(sum);
        while i < len {
            total += *weights_ptr.add(i) * (*a_ptr.add(i) * *b_ptr.add(i));
            i += 1;
        }
        total
    }
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
/// Pointers must be valid for the specified dimensions and strides.
/// Output buffer must be mutable and large enough.
#[no_mangle]
pub unsafe extern "C" fn dense_forward(
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

    // Safety: Pointers are checked generally, but specific bounds are caller's responsibility.
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
/// Pointers must be valid. `scratch_ptr` must point to sufficient scratch memory.
/// `layer_sizes_ptr` must point to `layer_count` integers.
#[no_mangle]
pub unsafe extern "C" fn mlp_forward(
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
    // Safety: We trust the layer_count passed from JS
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
    // Safety: We trust scratch_len passed from JS
    let scratch = unsafe { core::slice::from_raw_parts_mut(scratch_ptr, scratch_len) };
    let (mut cur_buf, mut next_buf) = scratch.split_at_mut(max_size);

    // Safety: Main computation loop involving pointer offsets
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
/// Pointers must be valid. State pointers must point to buffers of size `hidden_size * batch_count`.
#[no_mangle]
pub unsafe extern "C" fn gru_step(
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

    // Safety: Main computation loop involving pointer offsets
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
/// Pointers must be valid. State pointers must point to buffers of size `hidden_size * batch_count`.
#[no_mangle]
pub unsafe extern "C" fn lstm_step(
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

    // Safety: Main computation loop involving pointer offsets
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
/// Pointers must be valid. State pointers must point to buffers of size `hidden_size * batch_count`.
#[no_mangle]
pub unsafe extern "C" fn rru_step(
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

    // Safety: Main computation loop involving pointer offsets
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
