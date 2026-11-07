//! Numeric sources for managed checkpoints.

use super::*;

/// Borrowed flattened Float32 slices with checked aggregate count.
pub(super) struct FloatSource<'a> {
    pub(super) slices: Vec<&'a [f32]>,
    pub(super) total_floats: usize,
}

impl<'a> FloatSource<'a> {
    /// Construct a checked flattened source without copying its Float32 values.
    pub(super) fn new(
        slices: Vec<&'a [f32]>,
        limit: usize,
        label: &'static str,
    ) -> Result<Self, CheckpointError> {
        let mut total_floats = 0usize;
        for slice in &slices {
            total_floats = total_floats.checked_add(slice.len()).ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", format!("{label} count overflows usize"))
            })?;
        }
        if total_floats > limit {
            return Err(CheckpointError::format(
                "COUNT_LIMIT",
                format!("{label} count {total_floats} exceeds limit {limit}"),
            ));
        }
        Ok(Self {
            slices,
            total_floats,
        })
    }

    /// Checked raw packed byte length.
    pub(super) fn raw_bytes(&self) -> Result<u64, CheckpointError> {
        u64::try_from(self.total_floats)
            .ok()
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "numeric raw length overflows u64")
            })
    }
}

/// Reader that packs whole borrowed Float32 spans directly and retains only partial-float bytes.
pub(super) struct FloatByteReader<'a> {
    pub(super) source: &'a FloatSource<'a>,
    pub(super) slice_index: usize,
    pub(super) value_index: usize,
    pub(super) pending: [u8; 4],
    pub(super) pending_offset: usize,
    pub(super) pending_valid: bool,
}

impl<'a> FloatByteReader<'a> {
    /// Create a reader at the beginning of one logical numeric role.
    pub(super) fn new(source: &'a FloatSource<'a>) -> Self {
        Self {
            source,
            slice_index: 0,
            value_index: 0,
            pending: [0; 4],
            pending_offset: 0,
            pending_valid: false,
        }
    }

    /// Load the next Float32 bit pattern into the pending byte cell.
    pub(super) fn load_pending(&mut self) -> bool {
        while let Some(slice) = self.source.slices.get(self.slice_index) {
            if let Some(value) = slice.get(self.value_index) {
                self.pending = value.to_bits().to_le_bytes();
                self.pending_offset = 0;
                self.pending_valid = true;
                self.value_index += 1;
                return true;
            }
            self.slice_index += 1;
            self.value_index = 0;
        }
        false
    }
}

impl Read for FloatByteReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut written = 0usize;
        while written < output.len() {
            if !self.pending_valid && output.len() - written >= 4 {
                let Some(slice) = self.source.slices.get(self.slice_index) else {
                    break;
                };
                if self.value_index == slice.len() {
                    self.slice_index += 1;
                    self.value_index = 0;
                    continue;
                }
                let take = (slice.len() - self.value_index).min((output.len() - written) / 4);
                let byte_count = take * 4;
                let destinations = output[written..written + byte_count].as_chunks_mut::<4>().0;
                for (destination, value) in destinations
                    .iter_mut()
                    .zip(&slice[self.value_index..self.value_index + take])
                {
                    *destination = value.to_bits().to_le_bytes();
                }
                self.value_index += take;
                written += byte_count;
                continue;
            }
            if !self.pending_valid && !self.load_pending() {
                break;
            }
            let remaining = 4 - self.pending_offset;
            let take = remaining.min(output.len() - written);
            output[written..written + take]
                .copy_from_slice(&self.pending[self.pending_offset..self.pending_offset + take]);
            written += take;
            self.pending_offset += take;
            if self.pending_offset == 4 {
                self.pending_valid = false;
            }
        }
        Ok(written)
    }
}
