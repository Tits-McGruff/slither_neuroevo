//! Offline evolved-cohort matrix diagnostics, inference benchmark and recurrent stress.
use rayon::prelude::*;
use serde_json::{json, Value};
use slither_native::engine::{
    calculation::CalculationScratch,
    calculation_workers::validate_calculation_workers,
    evolved_cohort::EvolvedCohort,
    inference::{GraphExecutionPlan, InferenceMathBackend, QuantizedInferenceWeights},
};
use std::{
    error::Error,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

/// Bounded percentile histogram. Bucket edges are exact positive Float32 bit
/// ranges; returned intervals bound percentiles to <0.4% relative width.
struct Errors {
    count: u64,
    sum: f64,
    squared: f64,
    max: f64,
    bins: Box<[u64]>,
}
impl Default for Errors {
    fn default() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            squared: 0.0,
            max: 0.0,
            bins: vec![0; 65536].into_boxed_slice(),
        }
    }
}
impl Errors {
    fn add(&mut self, left: &[f32], right: &[f32]) {
        assert_eq!(left.len(), right.len());
        for (&a, &b) in left.iter().zip(right) {
            assert!(a.is_finite() && b.is_finite(), "nonfinite inference output");
            let e = (f64::from(a) - f64::from(b)).abs();
            self.count += 1;
            self.sum += e;
            self.squared += e * e;
            self.max = self.max.max(e);
            self.bins[((e as f32).to_bits() >> 15) as usize] += 1;
        }
    }
    fn merge(&mut self, other: &Self) {
        self.count += other.count;
        self.sum += other.sum;
        self.squared += other.squared;
        self.max = self.max.max(other.max);
        for (a, b) in self.bins.iter_mut().zip(other.bins.iter()) {
            *a += *b;
        }
    }
    fn report(&self) -> Value {
        let percentile = |p: u64| {
            let target = (self.count * p).div_ceil(1000);
            let mut cumulative = 0;
            for (i, n) in self.bins.iter().enumerate() {
                cumulative += n;
                if cumulative >= target {
                    // Expand one Float32 ULP to include rounding of the exact
                    // Float64 difference into its histogram key.
                    return json!({"lower":f32::from_bits(((i as u32)<<15).saturating_sub(1)),
                    "upperExclusive":f32::from_bits((((i as u32)+1)<<15)+1)});
                }
            }
            Value::Null
        };
        json!({"samples":self.count,"mae":self.sum/self.count.max(1) as f64,
            "rmse":(self.squared/self.count.max(1) as f64).sqrt(),"max":self.max,
            "p50":percentile(500),"p90":percentile(900),"p99":percentile(990),"p999":percentile(999)})
    }
}
struct Brain {
    weights: Box<[f32]>,
    cache: QuantizedInferenceWeights,
    initial: Box<[f32]>,
    before: Vec<f32>,
    after: Vec<f32>,
    output: Vec<f32>,
    scratch: CalculationScratch,
    rows: Vec<f32>,
    width: usize,
}
impl Brain {
    fn reset(&mut self) {
        self.before.copy_from_slice(&self.initial);
        self.after.fill(0.0);
    }
    fn evaluate(&mut self, plan: &GraphExecutionPlan, step: usize) {
        let row = step % (self.rows.len() / self.width);
        let input = &self.rows[row * self.width..(row + 1) * self.width];
        if plan.math_backend() == InferenceMathBackend::I16MatrixFma128 {
            plan.evaluate_quantized(
                &self.weights,
                &self.cache,
                &self.before,
                &mut self.after,
                input,
                &mut self.output,
                &mut self.scratch.view(),
            )
            .unwrap();
        } else {
            plan.evaluate(
                &self.weights,
                &self.before,
                &mut self.after,
                input,
                &mut self.output,
                &mut self.scratch.view(),
            )
            .unwrap();
        }
        std::mem::swap(&mut self.before, &mut self.after);
        std::hint::black_box(&self.output);
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let mut source = None;
    let mut observation_source = None;
    let mut output = None;
    let mut workers = 7;
    let mut count = 85;
    let mut deliveries = 1000;
    let mut warmup = 30;
    let mut passes = 300;
    let mut repetitions = 3;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing argument")?;
        match flag.as_str() {
            "--checkpoint" => source = Some(PathBuf::from(value)),
            "--observation-bank" => observation_source = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--workers" => workers = value.parse()?,
            "--brains" => count = value.parse()?,
            "--deliveries" => deliveries = value.parse()?,
            "--warmup" => warmup = value.parse()?,
            "--passes" => passes = value.parse()?,
            "--repetitions" => repetitions = value.parse()?,
            _ => return Err("unknown option".into()),
        }
    }
    if !(1..=10000).contains(&deliveries)
        || !(1..=10000).contains(&passes)
        || warmup > 10000
        || !(1..=10).contains(&repetitions)
    {
        return Err("invalid bounded counts".into());
    }
    validate_calculation_workers(workers)?;
    let output = output.ok_or("--output required")?;
    std::fs::create_dir(&output)?;
    let cohort = EvolvedCohort::load(&source.ok_or("--checkpoint required")?, count)?;
    let graph = &cohort.graph;
    let sse = GraphExecutionPlan::build_with_math_backend(graph, InferenceMathBackend::Sse2)?;
    let quantized =
        GraphExecutionPlan::build_with_math_backend(graph, InferenceMathBackend::I16MatrixFma128)?;
    let fma = GraphExecutionPlan::build_with_math_backend(
        graph,
        InferenceMathBackend::BenchmarkF32Fma128,
    )?;
    let width = sse.input_size();
    let observation_bank: Option<Vec<Vec<f32>>> = observation_source
        .as_ref()
        .map(|path| -> Result<_, Box<dyn Error>> {
            let metadata = std::fs::metadata(path)?;
            if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
                return Err("observation source exceeds bounded report size".into());
            }
            let report: Value = serde_json::from_reader(File::open(path)?)?;
            if report["graphLayoutSha256"] != graph.layout_digest_hex() {
                return Err("observation graph mismatch".into());
            }
            let bank: Vec<Vec<f32>> = serde_json::from_value(report["observationBank"].clone())?;
            if bank.len() != count
                || bank.iter().any(|rows| {
                    rows.is_empty()
                        || rows.len() % width != 0
                        || rows.len() > 32 * width
                        || rows.iter().any(|v| !v.is_finite())
                })
            {
                return Err("invalid bounded observation bank".into());
            }
            Ok(bank)
        })
        .transpose()?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()?;
    let mut matrices = Vec::new();
    let mut brains = Vec::new();
    let mut conversion_ms = 0.0;
    for (slot, (weights, initial)) in cohort.weights.into_iter().zip(cohort.recurrent).enumerate() {
        let start = Instant::now();
        let cache = quantized.quantize(&weights)?;
        conversion_ms += start.elapsed().as_secs_f64() * 1000.0;
        matrices.push(json!({"cohortSlot":slot,"matrices":quantized.quantization_diagnostics(&weights,&cache)?}));
        brains.push(Brain {
            weights,
            cache,
            before: initial.to_vec(),
            after: vec![0.0; graph.total_state_size],
            initial,
            output: vec![0.0; graph.output_size],
            scratch: CalculationScratch::try_new(sse.scratch_layout(), 64 * 1024 * 1024)?,
            rows: observation_bank
                .as_ref()
                .map(|bank| bank[slot].clone())
                .unwrap_or_else(|| {
                    (0..32 * width)
                        .map(|i| (((i * 17 + slot * 13) % 997) as f32 / 498.0 - 1.0) * 0.8)
                        .collect()
                }),
            width,
        });
    }
    std::fs::write(
        output.join("matrices.json"),
        serde_json::to_vec_pretty(&matrices)?,
    )?;
    let mut timings = Vec::new();
    for repetition in 0..repetitions {
        // Alternate order to expose warm host/order effects.
        let plans = if repetition % 2 == 0 {
            [&sse, &fma, &quantized]
        } else {
            [&quantized, &fma, &sse]
        };
        for plan in plans {
            brains.iter_mut().for_each(Brain::reset);
            for step in 0..warmup {
                pool.install(|| brains.par_iter_mut().for_each(|b| b.evaluate(plan, step)));
            }
            let start = Instant::now();
            for step in warmup..warmup + passes {
                pool.install(|| brains.par_iter_mut().for_each(|b| b.evaluate(plan, step)));
            }
            let ms = start.elapsed().as_secs_f64() * 1000.0 / passes as f64;
            timings.push(json!({"repetition":repetition,"backend":plan.math_backend().label(),"msPerPopulationPass":ms,"populationPassesPerSecond":1000.0/ms}));
        }
        eprintln!("benchmark repetition {} complete", repetition + 1);
    }
    let time_series: Vec<_> = pool.install(|| {
        brains
            .par_iter_mut()
            .enumerate()
            .map(|(slot, brain)| {
                brain.reset();
                let mut before = brain.initial.to_vec();
                let mut after = before.clone();
                let mut reference = vec![0.0; graph.output_size];
                let mut paired = reference.clone();
                let mut paired_state = before.clone();
                let mut errors: [Errors; 4] = std::array::from_fn(|_| Errors::default());
                let mut trace = Vec::with_capacity(deliveries);
                for step in 0..deliveries {
                    let row = step % (brain.rows.len() / width);
                    let input = &brain.rows[row * width..(row + 1) * width];
                    sse.evaluate(
                        &brain.weights,
                        &before,
                        &mut after,
                        input,
                        &mut reference,
                        &mut brain.scratch.view(),
                    )
                    .unwrap();
                    quantized
                        .evaluate_quantized(
                            &brain.weights,
                            &brain.cache,
                            &before,
                            &mut paired_state,
                            input,
                            &mut paired,
                            &mut brain.scratch.view(),
                        )
                        .unwrap();
                    errors[0].add(&paired, &reference);
                    errors[1].add(&paired_state, &after);
                    brain.evaluate(&quantized, step);
                    errors[2].add(&brain.output, &reference);
                    errors[3].add(&brain.before, &after);
                    let summarize = |a: &[f32], b: &[f32]| {
                        let values: Vec<_> = a
                            .iter()
                            .zip(b)
                            .map(|(&a, &b)| (f64::from(a) - f64::from(b)).abs())
                            .collect();
                        [
                            values.iter().sum::<f64>() / values.len() as f64,
                            (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64)
                                .sqrt(),
                            values.into_iter().fold(0.0, f64::max),
                        ]
                    };
                    trace.push([
                        summarize(&paired, &reference),
                        summarize(&paired_state, &after),
                        summarize(&brain.output, &reference),
                        summarize(&brain.before, &after),
                    ]);
                    std::mem::swap(&mut before, &mut after);
                }
                eprintln!("numerical cohort slot {slot} complete");
                (errors, trace)
            })
            .collect()
    });
    let mut csv = BufWriter::new(File::create(output.join("delivery-errors.csv"))?);
    writeln!(csv,"cohort_slot,delivery,paired_controller_mae,paired_controller_rmse,paired_controller_max,paired_state_mae,paired_state_rmse,paired_state_max,feedback_controller_mae,feedback_controller_rmse,feedback_controller_max,feedback_state_mae,feedback_state_rmse,feedback_state_max")?;
    let mut aggregate: [Errors; 4] = std::array::from_fn(|_| Errors::default());
    let mut per_genome = Vec::new();
    for (slot, (errors, trace)) in time_series.iter().enumerate() {
        for (step, row) in trace.iter().enumerate() {
            write!(csv, "{slot},{}", step + 1)?;
            for value in row.iter().flatten() {
                write!(csv, ",{value}")?;
            }
            writeln!(csv)?;
        }
        for (a, b) in aggregate.iter_mut().zip(errors) {
            a.merge(b);
        }
        per_genome.push(json!({"cohortSlot":slot,"errors":errors.iter().map(Errors::report).collect::<Vec<_>>()}));
    }
    csv.flush()?;
    let report = json!({"sourceSha256":slither_native::native_addon_source_sha256(),"buildContractSha256":slither_native::native_addon_build_contract_sha256(),
        "rustc":slither_native::native_addon_rustc_version(),"target":slither_native::native_addon_build_target(),
        "graphLayoutSha256":graph.layout_digest_hex(),"parametersPerGenome":graph.total_parameters,
        "sourceCohort":cohort.provenance,"brains":count,"workers":workers,"warmup":warmup,"passes":passes,"repetitions":repetitions,"timings":timings,
        "conversionMs":conversion_ms,"cacheBytes":brains.iter().map(|b|b.cache.allocated_bytes()).sum::<usize>(),"canonicalBytes":count*graph.total_parameters*4,
        "deliveries":deliveries,"referenceBackend":sse.math_backend().label(),"candidateBackend":quantized.math_backend().label(),
        "errorOrder":["pairedController","pairedRecurrent","feedbackController","feedbackRecurrent"],
        "errors":aggregate.iter().map(Errors::report).collect::<Vec<_>>(),"perGenome":per_genome,
        "observations":if observation_bank.is_some() {"Actual delivered Motion/B16/ray rows from the same mature cohort, up to 32 per genome, repeated identically for both backends"} else {"32 deterministic bounded synthetic rows per genome, repeated; same external input for both backends"},
        "observationRowsPerGenome":brains.iter().map(|b|b.rows.len()/width).collect::<Vec<_>>(),
        "initialState":"original checkpoint recurrent state keyed by brain handle",
        "percentiles":"bounded Float32-bit histogram intervals; MAE/RMSE/max accumulated in Float64"});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn error_percentile_bounds_and_merge() {
        let mut a = Errors::default();
        a.add(&[0.0, 1.0, -5.0], &[0.0, 0.0, -4.0]);
        let mut b = Errors::default();
        b.add(&[2.0], &[0.0]);
        a.merge(&b);
        assert_eq!(a.count, 4);
        assert_eq!(a.sum, 4.0);
        assert_eq!(a.squared, 6.0);
        let report = a.report();
        assert!(report["p99"]["lower"].as_f64().unwrap() <= 2.0);
        assert!(report["p99"]["upperExclusive"].as_f64().unwrap() > 2.0);
    }
}
