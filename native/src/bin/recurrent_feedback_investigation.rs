//! Repeat the retained mature observation bank with independent recurrent state.
use flate2::{write::GzEncoder, Compression};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use slither_native::engine::{
    calculation::CalculationScratch,
    calculation_workers::validate_calculation_workers,
    evolved_cohort::EvolvedCohort,
    inference::{
        feedback_diagnostics::{FeedbackMode, LstmCellTrace},
        GraphExecutionPlan, InferenceMathBackend,
    },
};
use std::{
    error::Error,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

#[derive(Clone, Copy, Default, Serialize)]
struct ErrorStats {
    count: u64,
    sum: f64,
    squared: f64,
    max: f64,
}
impl ErrorStats {
    fn between(a: &[f32], b: &[f32]) -> Self {
        let mut s = Self::default();
        for (&a, &b) in a.iter().zip(b) {
            assert!(a.is_finite() && b.is_finite());
            let e = (f64::from(a) - f64::from(b)).abs();
            s.count += 1;
            s.sum += e;
            s.squared += e * e;
            s.max = s.max.max(e);
        }
        s
    }
    fn merge(&mut self, b: Self) {
        self.count += b.count;
        self.sum += b.sum;
        self.squared += b.squared;
        self.max = self.max.max(b.max);
    }
    fn mae(self) -> f64 {
        self.sum / self.count.max(1) as f64
    }
    fn report(self) -> Value {
        json!({"samples":self.count,"mae":self.mae(),"rmse":(self.squared/self.count.max(1) as f64).sqrt(),"max":self.max})
    }
}
#[derive(Deserialize)]
struct TraceWindow {
    slot: usize,
    first: usize,
    last: usize,
}
type AnyError = Box<dyn Error + Send + Sync>;

fn main() -> Result<(), AnyError> {
    let mut args = std::env::args().skip(1);
    let mut checkpoint = None;
    let mut bank = None;
    let mut output = None;
    let mut deliveries = 10000usize;
    let mut workers = 7;
    let mut slots = (0..85).collect::<Vec<usize>>();
    let mut windows = Vec::<TraceWindow>::new();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing argument")?;
        match flag.as_str() {
            "--checkpoint" => checkpoint = Some(PathBuf::from(value)),
            "--observation-bank" => bank = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--workers" => workers = value.parse()?,
            "--deliveries" => deliveries = value.parse()?,
            "--slots" => slots = value.split(',').map(str::parse).collect::<Result<_, _>>()?,
            "--trace-windows" => windows = serde_json::from_reader(File::open(value)?)?,
            _ => return Err("unknown option".into()),
        }
    }
    if !(1..=10000).contains(&deliveries) || slots.is_empty() || slots.iter().any(|s| *s >= 85) {
        return Err("invalid bounded counts".into());
    }
    if windows
        .iter()
        .any(|w| w.slot >= 85 || w.first == 0 || w.last < w.first || w.last > deliveries)
    {
        return Err("invalid trace window".into());
    }
    validate_calculation_workers(workers)?;
    let output = output.ok_or("--output required")?;
    std::fs::create_dir(&output)?;
    let checkpoint = checkpoint.ok_or("--checkpoint required")?;
    let bank = bank.ok_or("--observation-bank required")?;
    let cohort = EvolvedCohort::load(&checkpoint, 85).map_err(|e| e.to_string())?;
    let report: Value = serde_json::from_reader(File::open(&bank)?)?;
    if report["graphLayoutSha256"] != cohort.graph.layout_digest_hex()
        || cohort.graph.total_parameters != 1_397_096
        || cohort.graph.total_state_size != 792
        || cohort.provenance["sourceGeneration"] != 21
    {
        return Err("exact mature cohort mismatch".into());
    }
    let observation_bank: Vec<Vec<f32>> =
        serde_json::from_value(report["observationBank"].clone())?;
    if observation_bank.len() != 85
        || observation_bank
            .iter()
            .any(|r| r.len() != 32 * 264 || r.iter().any(|x| !x.is_finite()))
    {
        return Err("expected exact original 32 x 264 observation bank".into());
    }
    let sse =
        GraphExecutionPlan::build_with_math_backend(&cohort.graph, InferenceMathBackend::Sse2)?;
    let fma = GraphExecutionPlan::build_with_math_backend(
        &cohort.graph,
        InferenceMathBackend::BenchmarkF32Fma128,
    )?;
    let quantized = GraphExecutionPlan::build_with_math_backend(
        &cohort.graph,
        InferenceMathBackend::I16MatrixFma128,
    )?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()?;
    let started = Instant::now();
    let results: Vec<Value> = pool.install(|| slots.par_iter().map(|&slot| -> Result<Value, AnyError> {
        let weights = &cohort.weights[slot];
        let cache = quantized.quantize(weights)?;
        let mut scratch = CalculationScratch::try_new(sse.scratch_layout(), 64*1024*1024)?;
        let mut before: Vec<Vec<f32>> = FeedbackMode::ALL.iter().map(|_| cohort.recurrent[slot].to_vec()).collect();
        let mut after = before.clone();
        let mut outputs = [[0.0f32; 2]; 7];
        let mut totals = [[ErrorStats::default(); 3]; 7];
        let mut csv = GzEncoder::new(BufWriter::new(File::create(output.join(format!("slot-{slot:02}-errors.csv.gz")))?), Compression::fast());
        writeln!(csv,"slot,delivery,pair,controller_mae,controller_rmse,controller_max,hidden_mae,hidden_rmse,hidden_max,cell_mae,cell_rmse,cell_max,turn,boost,reference_turn,reference_boost")?;
        let mut trace_file = GzEncoder::new(BufWriter::new(File::create(output.join(format!("slot-{slot:02}-lstm.jsonl.gz")))?), Compression::fast());
        let mut trace_count = 0;
        let mut parity_checks = 0;
        for step in 0..deliveries {
            let row = step % 32;
            let input = &observation_bank[slot][row*264..(row+1)*264];
            let capture = windows.iter().any(|w| w.slot == slot && w.first <= step+1 && step+1 <= w.last);
            let mut mode_traces = Vec::new();
            for (index, mode) in FeedbackMode::ALL.into_iter().enumerate() {
                match mode {
                    FeedbackMode::Sse2 => sse.evaluate(weights, &before[index], &mut after[index], input, &mut outputs[index], &mut scratch.view())?,
                    FeedbackMode::Fma128 => fma.evaluate(weights, &before[index], &mut after[index], input, &mut outputs[index], &mut scratch.view())?,
                    FeedbackMode::I16 => quantized.evaluate_quantized(weights, &cache, &before[index], &mut after[index], input, &mut outputs[index], &mut scratch.view())?,
                    _ => sse.evaluate_feedback_diagnostic(mode, weights, &cache, &before[index], &mut after[index], input, &mut outputs[index], &mut scratch.view(), None)?,
                }
                if capture || (index < 3 && (step < 32 || step % 1024 == 0)) {
                    let mut check_state = vec![0.0; 792]; let mut check_output = [0.0; 2];
                    let mut lstm = Vec::<LstmCellTrace>::new();
                    sse.evaluate_feedback_diagnostic(mode, weights, &cache, &before[index], &mut check_state, input, &mut check_output, &mut scratch.view(), capture.then_some(&mut lstm))?;
                    assert_eq!(check_state, after[index], "diagnostic state differs slot {slot} step {step} mode {mode:?}");
                    assert_eq!(check_output, outputs[index], "diagnostic controller differs");
                    parity_checks += 1;
                    if capture {
                        let mut counterfactual = Vec::<LstmCellTrace>::new();
                        sse.evaluate_feedback_diagnostic(FeedbackMode::Sse2, weights, &cache, &before[index], &mut check_state, input, &mut check_output, &mut scratch.view(), Some(&mut counterfactual))?;
                        mode_traces.push(json!({"mode":mode.label(),"controller":outputs[index],"lstm":lstm,
                            "sseOnSameStateController":check_output,"sseOnSameStateLstm":counterfactual}));
                    }
                }
            }
            if capture {
                serde_json::to_writer(&mut trace_file, &json!({"slot":slot,"delivery":step+1,"observationRow":row,"modes":mode_traces}))?;
                writeln!(trace_file)?; trace_count += 1;
            }
            // Six candidates against SSE2, plus i16 against F32-FMA128.
            for pair in 0..7 {
                let (candidate, reference) = if pair < 6 { (pair+1, 0) } else { (2, 1) };
                let errors = [ErrorStats::between(&outputs[candidate], &outputs[reference]),
                    ErrorStats::between(&after[candidate][..396], &after[reference][..396]),
                    ErrorStats::between(&after[candidate][396..], &after[reference][396..])];
                write!(csv,"{slot},{},{}:{}", step+1, FeedbackMode::ALL[candidate].label(), FeedbackMode::ALL[reference].label())?;
                for (total, e) in totals[pair].iter_mut().zip(errors) {
                    total.merge(e); write!(csv,",{},{},{}",e.mae(),(e.squared/e.count as f64).sqrt(),e.max)?;
                }
                writeln!(csv,",{},{},{},{}",outputs[candidate][0],outputs[candidate][1],outputs[reference][0],outputs[reference][1])?;
            }
            std::mem::swap(&mut before, &mut after);
        }
        csv.finish()?.flush()?; trace_file.finish()?.flush()?;
        eprintln!("slot {slot} finished {deliveries} deliveries, {trace_count} internal traces");
        Ok(json!({"slot":slot,"totals":totals.map(|metrics| metrics.map(ErrorStats::report)),"diagnosticEndpointParityChecks":parity_checks,"traceDeliveries":trace_count}))
    }).collect::<Result<Vec<_>,_>>())?;
    let metadata = json!({"nativeSourceSha256":slither_native::native_addon_source_sha256(),"buildContractSha256":slither_native::native_addon_build_contract_sha256(),
        "rustc":slither_native::native_addon_rustc_version(),"target":slither_native::native_addon_build_target(),
        "graphLayoutSha256":cohort.graph.layout_digest_hex(),"sourceCohort":cohort.provenance,"brains":85,"selectedSlots":slots,
        "workers":workers,"deliveries":deliveries,"wallSeconds":started.elapsed().as_secs_f64(),"modes":FeedbackMode::ALL.map(FeedbackMode::label),
        "metrics":["controller","hidden","cell"],"results":results,
        "initialState":"same original checkpoint recurrent state keyed by brain handle; no burn-in",
        "observations":"identical original 32 actual Motion/B16/ray observation rows repeated cyclically per genome",
        "fallbackArithmetic":"ablations keep unquantized matrices on existing SSE2; all scales/biases unchanged",
        "defaultBackend":"rust-sse2-v1","productionConfigurationChanged":false});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_accumulate_component_counts_and_float64_differences() {
        let a = ErrorStats::between(&[1.0, -2.0], &[0.0, 0.0]);
        assert_eq!(a.count, 2);
        assert_eq!(a.sum, 3.0);
        assert_eq!(a.squared, 5.0);
        assert_eq!(a.max, 2.0);
    }
}
