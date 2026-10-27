//! Derived inference storage. Canonical parameters and persistence remain Float32.
use super::*;
use std::ops::Range;

/// Compact graph-bound inference cache; never serialized or used by evolution.
#[derive(Clone, Debug, PartialEq)]
pub struct QuantizedInferenceWeights {
    pub(super) layout: [u8; 32],
    values: Box<[i16]>,
    scales: Box<[f32]>,
}
impl QuantizedInferenceWeights {
    #[cfg(test)]
    pub(super) fn storage_address(&self) -> *const i16 {
        self.values.as_ptr()
    }
    /// Numeric allocation bytes, including zero placeholders for separate recurrent biases.
    pub fn allocated_bytes(&self) -> usize {
        self.values.len() * 2 + self.scales.len() * 4
    }
    /// Number of major matrices, independent of row count.
    pub fn matrix_count(&self) -> usize {
        self.scales.len()
    }
}

/// Identity of immutable weights at a complete population/brain ownership boundary.
/// Physical life replacement does not invalidate population weights.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BoundCache {
    epoch: u64,
    pub(super) handle: super::super::state::BrainHandle,
    pub(super) genome_id: Option<u64>,
    pub(super) weights: Option<QuantizedInferenceWeights>,
}

/// Compile matrix domains from the validated existing execution ranges.
/// Dense/MLP biases are interleaved and share that affine matrix's scale.
/// Recurrent biases are separate and read directly from canonical Float32 storage.
pub(super) fn matrix_layout(nodes: &mut [ExecutionNode]) -> Vec<TensorRange> {
    let mut matrices = Vec::new();
    for node in nodes {
        node.matrix_start = matrices.len();
        let mut offset = node.parameter.offset;
        let mut add = |len| {
            matrices.push(TensorRange { offset, len });
            offset += len;
        };
        match node.node_type {
            CompiledNodeType::Dense | CompiledNodeType::Mlp => {
                let mut input = node.inputs[0].len;
                for output in node.hidden.iter().chain(std::iter::once(&node.output)) {
                    add((input + 1) * output.len);
                    input = output.len;
                }
            }
            CompiledNodeType::Gru | CompiledNodeType::Lstm | CompiledNodeType::Rru => {
                let gates = match node.node_type {
                    CompiledNodeType::Gru => 3,
                    CompiledNodeType::Lstm => 4,
                    _ => 2,
                };
                for _ in 0..gates {
                    add(node.inputs[0].len * node.output.len);
                }
                for _ in 0..gates {
                    add(node.output.len * node.output.len);
                }
            }
            _ => {}
        }
    }
    matrices
}

impl GraphExecutionPlan {
    /// Cold diagnostic of the exact production quantization domains and values.
    /// Bias counterfactuals are measurements only; they do not change the cache.
    #[cfg(feature = "engine-test-hooks")]
    pub fn quantization_diagnostics(
        &self,
        weights: &[f32],
        cache: &QuantizedInferenceWeights,
    ) -> Result<serde_json::Value, InferenceError> {
        require_length("diagnostic weights", weights.len(), self.total_parameters)?;
        require_length(
            "diagnostic cache",
            cache.values.len(),
            self.total_parameters,
        )?;
        require_length("diagnostic scales", cache.scales.len(), self.matrices.len())?;
        if cache.layout != self.layout_digest_sha256 {
            return Err(InferenceError::InvalidCompiledGraph {
                detail: "diagnostic cache layout mismatch".into(),
            });
        }
        let mut rows = Vec::new();
        for node in &self.nodes {
            let end = self
                .nodes
                .iter()
                .find(|next| next.matrix_start > node.matrix_start)
                .map_or(self.matrices.len(), |next| next.matrix_start);
            let affine = matches!(
                node.node_type,
                CompiledNodeType::Dense | CompiledNodeType::Mlp
            );
            let mut input = node.inputs.first().map_or(0, |range| range.len);
            for index in node.matrix_start..end {
                let matrix = self.matrices[index];
                // Nodes without matrices can share matrix_start with the following node.
                if matrix.offset < node.parameter.offset
                    || matrix.offset >= node.parameter.offset + node.parameter.len
                {
                    break;
                }
                let source = &weights[matrix.offset..matrix.offset + matrix.len];
                let quantized = &cache.values[matrix.offset..matrix.offset + matrix.len];
                let scale = cache.scales[index];
                let mut abs: Vec<f32> = source.iter().map(|v| v.abs()).collect();
                abs.sort_unstable_by(f32::total_cmp);
                let max = f64::from(*abs.last().unwrap());
                let mut sum = 0.0;
                let mut squared = 0.0;
                let mut error = 0.0;
                let mut error_squared = 0.0;
                let mut error_max = 0.0_f64;
                let mut zeros = 0;
                let mut saturated = 0;
                let mut bias_max = 0.0_f32;
                let mut weight_max = 0.0_f32;
                let mut non_bias_error = 0.0;
                let mut non_bias_error_squared = 0.0;
                let mut non_bias_count = 0usize;
                for (offset, (&w, &q)) in source.iter().zip(quantized).enumerate() {
                    let value = f64::from(w);
                    let difference = (value - f64::from(f32::from(q) * scale)).abs();
                    sum += value.abs();
                    squared += value * value;
                    error += difference;
                    error_squared += difference * difference;
                    error_max = error_max.max(difference);
                    zeros += usize::from(q == 0);
                    saturated += usize::from(q.unsigned_abs() == 32767);
                    if affine && offset % (input + 1) == input {
                        bias_max = bias_max.max(w.abs());
                    } else {
                        weight_max = weight_max.max(w.abs());
                        non_bias_error += difference;
                        non_bias_error_squared += difference * difference;
                        non_bias_count += 1;
                    }
                }
                // Measurement only. A temporary affine weight-only domain uses
                // the unchanged production quantizer, to measure bias-induced
                // loss of precision. Neither canonical weights nor cache change.
                let weight_only_diagnostic = affine.then(|| {
                    let source: Vec<f32> = source
                        .iter()
                        .enumerate()
                        .filter_map(|(offset, &w)| (offset % (input + 1) != input).then_some(w))
                        .collect();
                    let mut values = vec![0; source.len()];
                    let scale = quantize_matrix(&source, &mut values);
                    let (sum, squared) =
                        source
                            .iter()
                            .zip(values)
                            .fold((0.0, 0.0), |(sum, squared), (&w, q)| {
                                let error = (f64::from(w) - f64::from(f32::from(q) * scale)).abs();
                                (sum + error, squared + error * error)
                            });
                    (
                        scale,
                        sum / source.len() as f64,
                        (squared / source.len() as f64).sqrt(),
                    )
                });
                let n = source.len() as f64;
                let p99 = abs[(source.len() * 99).div_ceil(100).saturating_sub(1)];
                let p999 = abs[(source.len() * 999).div_ceil(1000).saturating_sub(1)];
                rows.push(serde_json::json!({"node":node.id,"matrixIndex":index,
                    "matrixWithinNode":index-node.matrix_start,"length":source.len(),
                    "maxAbs":max,"meanAbs":sum/n,"rms":(squared/n).sqrt(),
                    "scale":scale,"mae":error/n,"maxError":error_max,"rmse":(error_squared/n).sqrt(),
                    "fractionZero":zeros as f64/n,"fractionAtSymmetricLimit":saturated as f64/n,
                    "p99Abs":p99,"maxToP99":if p99 > 0.0 {Some(max/f64::from(p99))} else {None},
                    "p999Abs":p999,
                    "nonBiasMae":non_bias_error/non_bias_count as f64,"nonBiasRmse":(non_bias_error_squared/non_bias_count as f64).sqrt(),
                    "weightOnlyDiagnosticScale":weight_only_diagnostic.map(|v|v.0),
                    "weightOnlyDiagnosticMae":weight_only_diagnostic.map(|v|v.1),
                    "weightOnlyDiagnosticRmse":weight_only_diagnostic.map(|v|v.2),
                    "biasSharesScale":affine,"maxAbsBias":bias_max,"maxAbsNonBias":weight_max,
                    "biasStrictlyDominates":affine && bias_max > weight_max,
                    "biasScaleInflation":if affine && weight_max > 0.0 {Some(f64::from(bias_max.max(weight_max)/weight_max))} else {None}}));
                if affine {
                    input = node
                        .hidden
                        .iter()
                        .chain(std::iter::once(&node.output))
                        .nth(index - node.matrix_start)
                        .unwrap()
                        .len;
                }
            }
        }
        Ok(serde_json::Value::Array(rows))
    }

    /// Quantize once at parameter publication, using deterministic nearest rounding
    /// (halfway values away from zero). Reject nonfinite canonical parameters.
    pub fn quantize(&self, weights: &[f32]) -> Result<QuantizedInferenceWeights, InferenceError> {
        require_length("quantization weights", weights.len(), self.total_parameters)?;
        require_finite("quantization weights", weights)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(weights.len())
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "i16 inference weights",
                elements: weights.len(),
            })?;
        values.resize(weights.len(), 0);
        let mut scales = Vec::new();
        scales.try_reserve_exact(self.matrices.len()).map_err(|_| {
            InferenceError::AllocationFailed {
                buffer: "i16 matrix scales",
                elements: self.matrices.len(),
            }
        })?;
        for matrix in &self.matrices {
            let range = matrix.offset..matrix.offset + matrix.len;
            let source = &weights[range.clone()];
            scales.push(quantize_matrix(source, &mut values[range]));
        }
        Ok(QuantizedInferenceWeights {
            layout: self.layout_digest_sha256,
            values: values.into_boxed_slice(),
            scales: scales.into_boxed_slice(),
        })
    }

    /// Bind immutable authority weights before dispatching calculation workers.
    /// A new population epoch rebuilds caches; added/replaced entity brain handles
    /// rebuild only their entries. Ordinary repeated evaluations never convert.
    /// Callers must advance the epoch when mutating canonical population weights.
    /// Ordinary brain-buffer swaps retain immutable weights by handle/owner, so
    /// allocation addresses deliberately do not participate in invalidation.
    pub fn prepare_weight_caches(
        &mut self,
        epoch: u64,
        population: &[PopulationGenome],
        brains: &[BrainRuntimeState],
    ) -> Result<(), InferenceError> {
        if self.math_backend != InferenceMathBackend::I16MatrixFma128 {
            return Ok(());
        }
        self.caches.truncate(brains.len());
        self.caches
            .try_reserve_exact(brains.len().saturating_sub(self.caches.len()))
            .map_err(|_| InferenceError::AllocationFailed {
                buffer: "i16 cache bindings",
                elements: brains.len(),
            })?;
        for (index, brain) in brains.iter().enumerate() {
            let (weights, genome_id) = match brain.owner {
                BrainOwner::PopulationSlot(slot) => {
                    let genome = population.get(slot as usize).ok_or(
                        InferenceError::PopulationSlotOutOfBounds {
                            slot,
                            population_count: population.len(),
                        },
                    )?;
                    (
                        Some(genome.weights.as_ref()),
                        Some(genome.lineage.genome_id),
                    )
                }
                BrainOwner::Entity(_) => (brain.non_population_weights.as_deref(), None),
            };
            if self.caches.get(index).is_some_and(|c| {
                c.epoch == epoch && c.handle == brain.handle && c.genome_id == genome_id
            }) {
                continue;
            }
            // Drop retired storage before allocating its replacement.
            if let Some(c) = self.caches.get_mut(index) {
                c.weights = None;
            }
            let entry = BoundCache {
                epoch,
                handle: brain.handle,
                genome_id,
                weights: weights.map(|w| self.quantize(w)).transpose()?,
            };
            if index < self.caches.len() {
                self.caches[index] = entry;
            } else {
                self.caches.push(entry);
            }
        }
        Ok(())
    }

    /// Derived numeric cache bytes retained by this plan, for existing diagnostics.
    pub fn inference_cache_bytes(&self) -> usize {
        self.caches
            .iter()
            .filter_map(|c| c.weights.as_ref())
            .map(QuantizedInferenceWeights::allocated_bytes)
            .sum()
    }
}

/// Copyable view of one node/layer's existing parameter region. Scale indexes
/// are fixed matrix ordinals compiled once, never searched in the row loop.
#[derive(Clone, Copy)]
pub(super) struct NodeWeights<'a> {
    canonical: &'a [f32],
    quantized: Option<&'a [i16]>,
    scales: &'a [f32],
}
impl<'a> NodeWeights<'a> {
    pub(super) fn new(
        canonical: &'a [f32],
        cache: Option<&'a QuantizedInferenceWeights>,
        range: TensorRange,
        matrix: usize,
    ) -> Self {
        let end = range.offset + range.len;
        Self {
            canonical: &canonical[range.offset..end],
            quantized: cache.map(|c| &c.values[range.offset..end]),
            scales: cache.map_or(&[], |c| &c.scales[matrix..]),
        }
    }
    pub(super) fn len(self) -> usize {
        self.canonical.len()
    }
    #[inline]
    pub(super) fn dot(
        self,
        backend: InferenceMathBackend,
        range: Range<usize>,
        input: &[f32],
        matrix: usize,
    ) -> f32 {
        match self.quantized {
            Some(q) => {
                simd_kernels::dot_product_i16_fma128(&q[range], input, None, self.scales[matrix])
            }
            None => dot_product(backend, &self.canonical[range], input),
        }
    }
    #[inline]
    pub(super) fn dot_mul(
        self,
        backend: InferenceMathBackend,
        range: Range<usize>,
        left: &[f32],
        right: &[f32],
        matrix: usize,
    ) -> f32 {
        match self.quantized {
            Some(q) => simd_kernels::dot_product_i16_fma128(
                &q[range],
                left,
                Some(right),
                self.scales[matrix],
            ),
            None => dot_product_mul(backend, &self.canonical[range], left, right),
        }
    }
    #[inline]
    pub(super) fn bias(self, index: usize, matrix: Option<usize>) -> f32 {
        match (self.quantized, matrix) {
            (Some(q), Some(m)) => q[index] as f32 * self.scales[m],
            _ => self.canonical[index],
        }
    }
}

/// Conservative peak cache admission, including binding capacity and matrix metadata.
/// This runs before background population allocation, including metadata-only restore.
pub(crate) fn matrix_i16_cache_admission_bytes(
    graph: &CompiledGraph,
    owners: usize,
) -> Option<usize> {
    let matrices = graph.nodes.iter().try_fold(0usize, |count, node| {
        count.checked_add(match node.node_type {
            CompiledNodeType::Dense => 1,
            CompiledNodeType::Mlp => node.hidden_sizes.len().checked_add(1)?,
            CompiledNodeType::Gru => 6,
            CompiledNodeType::Lstm => 8,
            CompiledNodeType::Rru => 4,
            _ => 0,
        })
    })?;
    graph
        .total_parameters
        .checked_mul(2)?
        .checked_add(matrices.checked_mul(4)?)?
        .checked_add(std::mem::size_of::<BoundCache>())?
        .checked_mul(owners)?
        .checked_add(matrices.checked_mul(std::mem::size_of::<TensorRange>())?)
}

/// Symmetric nearest quantization of one complete matrix, including interleaved
/// affine biases. The quantization rule and tie policy belong to backend v1.
fn quantize_matrix(source: &[f32], destination: &mut [i16]) -> f32 {
    let maximum = source.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
    // Preserve finite subnormals without division by a scale rounded to zero.
    let scale = if maximum == 0.0 {
        1.0
    } else {
        (maximum / 32767.0).max(f32::from_bits(1))
    };
    for (q, &value) in destination.iter_mut().zip(source) {
        *q = (value / scale).round().clamp(-32767.0, 32767.0) as i16;
    }
    scale
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "engine-test-hooks")]
    #[test]
    fn matrix_diagnostics_measure_interleaved_bias_outlier_without_changing_cache() {
        let graph = crate::engine::graph_json::compile_training_graph_json(
            br#"{
            "type":"graph","outputSize":2,"nodes":[{"id":"input","type":"Input","outputSize":2},
            {"id":"head","type":"Dense","inputSize":2,"outputSize":2}],
            "edges":[{"from":"input","to":"head"}],"outputs":[{"nodeId":"head"}]}"#,
        )
        .unwrap();
        let plan =
            GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)
                .unwrap();
        let weights = [0.5, -1.0, 5.0, -0.25, 0.75, -2.0];
        let cache = plan.quantize(&weights).unwrap();
        let before = cache.clone();
        let report = plan.quantization_diagnostics(&weights, &cache).unwrap();
        assert_eq!(report.as_array().unwrap().len(), 1);
        assert_eq!(report[0]["length"], 6);
        assert_eq!(report[0]["biasStrictlyDominates"], true);
        assert_eq!(report[0]["biasScaleInflation"], 5.0);
        assert_eq!(report[0]["maxAbsNonBias"], 1.0);
        assert!(
            report[0]["nonBiasMae"].as_f64().unwrap()
                > report[0]["weightOnlyDiagnosticMae"].as_f64().unwrap()
        );
        assert!(
            report[0]["nonBiasRmse"].as_f64().unwrap()
                > report[0]["weightOnlyDiagnosticRmse"].as_f64().unwrap()
        );
        assert_eq!(cache, before);
    }
    #[test]
    fn symmetric_quantization_zero_sign_extrema_ties_and_subnormals() {
        let mut q = [99; 11];
        assert_eq!(quantize_matrix(&[0.0; 11], &mut q), 1.0);
        assert_eq!(q, [0; 11]);
        let source = [
            -5.0, 5.0, -4.99999, 4.99999, -0.000001, 0.000001, 0.0, -0.3, 0.3, -2.5, 2.5,
        ];
        let scale = quantize_matrix(&source, &mut q);
        assert_eq!(scale, 5.0 / 32767.0);
        assert_eq!(&q[..6], &[-32767, 32767, -32767, 32767, 0, 0]);
        for (&value, &integer) in source.iter().zip(&q) {
            assert_ne!(integer, i16::MIN);
            assert!((value - integer as f32 * scale).abs() <= scale * 0.501);
        }
        let mut again = [0; 11];
        assert_eq!(quantize_matrix(&source, &mut again), scale);
        assert_eq!(q, again);
        let mut ties = [0; 3];
        assert_eq!(quantize_matrix(&[32767.0, 0.5, -0.5], &mut ties), 1.0);
        assert_eq!(ties, [32767, 1, -1]);
        let mut tiny = [0; 2];
        assert_eq!(
            quantize_matrix(&[f32::from_bits(1), -f32::from_bits(1)], &mut tiny),
            f32::from_bits(1)
        );
        assert_eq!(tiny, [1, -1]);
    }

    #[test]
    fn fma128_sign_extension_unaligned_vectors_short_rows_and_tails() {
        if !simd_kernels::runtime_i16_fma128_available() {
            return;
        }
        for length in 0..66 {
            let weights: Vec<i16> = (0..length + 1)
                .map(|i| [-32767, -123, 0, 1, 32767][i % 5])
                .collect();
            let inputs: Vec<f32> = (0..length + 1).map(|i| (i as f32 - 17.0) / 31.0).collect();
            let right: Vec<f32> = inputs.iter().map(|x| x * 0.2).collect();
            // Deliberately start at an unaligned i16 and Float32 address.
            for multiplier in [None, Some(&right[1..])] {
                let actual = simd_kernels::dot_product_i16_fma128(
                    &weights[1..],
                    &inputs[1..],
                    multiplier,
                    0.0001,
                );
                let expected: f64 = (0..length)
                    .map(|i| {
                        f64::from(weights[i + 1])
                            * f64::from(inputs[i + 1] * multiplier.map_or(1.0, |r| r[i]))
                    })
                    .sum::<f64>()
                    * 0.0001;
                assert!(
                    (f64::from(actual) - expected).abs() < 0.00002,
                    "length={length} actual={actual} expected={expected}"
                );
            }
        }
    }
}
