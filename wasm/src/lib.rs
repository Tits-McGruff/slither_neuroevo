//! WASM kernels for Dense and MLP forward passes.

use core::mem;

/// Convert signed sizes into non-negative `usize` values.
fn to_usize(value: i32) -> usize {
    if value <= 0 {
        0
    } else {
        value as usize
    }
}

/// Compute a Dense forward pass for a batch of inputs.
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

    for b in 0..batch_count {
        let input_base = b * input_stride;
        let output_base = b * output_stride;
        for o in 0..output_stride {
            *output_ptr.add(output_base + o) = 0.0;
        }
        let mut w_index = 0usize;
        for o in 0..out_limit {
            let mut sum = 0.0;
            for i in 0..in_size {
                sum += *weights_ptr.add(w_index) * *input_ptr.add(input_base + i);
                w_index += 1;
            }
            sum += *weights_ptr.add(w_index);
            w_index += 1;
            *output_ptr.add(output_base + o) = sum.tanh();
        }
    }
}

/// Compute an MLP forward pass for a batch of inputs.
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
) {
    if weights_ptr.is_null() || layer_sizes_ptr.is_null() || input_ptr.is_null() || output_ptr.is_null() {
        return;
    }
    let layer_count = to_usize(layer_count);
    if layer_count < 2 {
        return;
    }
    let batch_count = to_usize(batch_count);
    let input_stride = to_usize(input_stride);
    let output_stride = to_usize(output_stride);
    let layer_sizes = core::slice::from_raw_parts(layer_sizes_ptr, layer_count);

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
    let mut cur_buf = vec![0.0f32; max_size];
    let mut next_buf = vec![0.0f32; max_size];

    for b in 0..batch_count {
        let input_base = b * input_stride;
        let input_size = to_usize(layer_sizes[0]);
        for i in 0..input_size {
            cur_buf[i] = *input_ptr.add(input_base + i);
        }
        let mut w_index = 0usize;
        for l in 0..(layer_count - 1) {
            let ins = to_usize(layer_sizes[l]);
            let outs = to_usize(layer_sizes[l + 1]);
            for o in 0..outs {
                let mut sum = 0.0f32;
                for i in 0..ins {
                    sum += *weights_ptr.add(w_index) * cur_buf[i];
                    w_index += 1;
                }
                sum += *weights_ptr.add(w_index);
                w_index += 1;
                next_buf[o] = sum.tanh();
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
        for o in 0..out_limit {
            *output_ptr.add(output_base + o) = cur_buf[o];
        }
    }
}
