//! Population inference comparison using the production graph executor and kernels.
//! RUSTFLAGS='-C target-cpu=native' cargo run --release --features engine-test-hooks
//! --bin matrix-i16-benchmark -- --workers 7 --brains 85 --warmup 20 --passes 100
use rayon::prelude::*;
use serde_json::json;
use slither_native::engine::{
    calculation::CalculationScratch,
    calculation_workers::validate_calculation_workers,
    genome::{initialize_random_genome, GenomeInitializationConfig},
    graph::GraphBundle,
    inference::{GraphExecutionPlan, InferenceMathBackend, QuantizedInferenceWeights},
    rng::StatefulRng,
};
use std::{error::Error, time::Instant};

/// Exact owner-supplied graph, decoded through the ordinary production JSON path.
fn graph() -> GraphBundle {
    slither_native::engine::graph_json::compile_training_graph_json(
        br#"{
      "type":"graph","outputSize":2,
      "nodes":[
        {"id":"input","outputSize":264,"type":"Input"},
        {"id":"mlp-a","inputSize":264,"outputSize":396,"type":"Dense"},
        {"hiddenSize":396,"id":"LSTM","inputSize":264,"type":"LSTM"},
        {"id":"concat","type":"Concat"},
        {"hiddenSizes":[264,134],"id":"mlp-b","inputSize":792,"outputSize":2,"type":"MLP"}
      ],
      "edges":[
        {"from":"input","to":"mlp-a"},{"from":"input","to":"LSTM"},
        {"from":"mlp-a","to":"concat","toPort":0},
        {"from":"LSTM","to":"concat","toPort":1},
        {"from":"concat","to":"mlp-b"}
      ],
      "outputs":[{"nodeId":"mlp-b"}]
    }"#,
    )
    .expect("owner benchmark graph compiles under production admission")
}

struct Brain {
    weights: Vec<f32>,
    cache: QuantizedInferenceWeights,
    rows: Vec<f32>,
    before: Vec<f32>,
    after: Vec<f32>,
    output: Vec<f32>,
    scratch: CalculationScratch,
}
impl Brain {
    fn reset(&mut self) {
        self.before.fill(0.0);
        self.after.fill(0.0);
    }
    fn evaluate(&mut self, plan: &GraphExecutionPlan, step: usize) {
        let input = &self.rows[(step % 32) * 264..(step % 32 + 1) * 264];
        if plan.math_backend().uses_i16_cache() {
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
#[derive(Default)]
struct Errors {
    sum: f64,
    max: f64,
    count: usize,
}
impl Errors {
    fn add(&mut self, left: &[f32], right: &[f32]) {
        for (&a, &b) in left.iter().zip(right) {
            let error = (f64::from(a) - f64::from(b)).abs();
            self.sum += error;
            self.max = self.max.max(error);
            self.count += 1;
        }
    }
    fn report(&self) -> serde_json::Value {
        json!({"mae": self.sum/self.count.max(1) as f64, "max": self.max, "samples":self.count})
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let mut workers = 7;
    let mut count = 85;
    let mut warmup = 20;
    let mut passes = 100;
    let mut error_steps = 64;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value: usize = args.next().ok_or("missing count")?.parse()?;
        match flag.as_str() {
            "--workers" => workers = value,
            "--brains" => count = value,
            "--warmup" => warmup = value,
            "--passes" => passes = value,
            "--error-steps" => error_steps = value,
            _ => return Err("unknown option".into()),
        }
    }
    if count == 0
        || count > 300
        || passes == 0
        || passes > 10000
        || warmup > 10000
        || error_steps == 0
        || error_steps > 256
    {
        return Err("invalid bounded benchmark counts".into());
    }
    validate_calculation_workers(workers)?;
    let graph = graph();
    assert_eq!(graph.total_parameters, 1_397_096);
    let scalar = GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Scalar)?;
    let sse = GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::Sse2)?;
    let quantized =
        GraphExecutionPlan::build_with_math_backend(&graph, InferenceMathBackend::I16MatrixFma128)?;
    let fma = GraphExecutionPlan::build_with_math_backend(
        &graph,
        InferenceMathBackend::BenchmarkF32Fma128,
    )?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()?;
    let mut rng = StatefulRng::new(42.0).export_state();
    let mut conversion_ms = 0.0;
    let mut brains = Vec::new();
    for slot in 0..count {
        let (weights, next) =
            initialize_random_genome(&graph, &rng, GenomeInitializationConfig::default())?
                .into_parts();
        rng = next;
        let start = Instant::now();
        let cache = quantized.quantize(&weights)?;
        conversion_ms += start.elapsed().as_secs_f64() * 1000.0;
        let rows = (0..32 * 264)
            .map(|i| (((i * 17 + slot * 13) % 997) as f32 / 498.0 - 1.0) * 0.8)
            .collect();
        brains.push(Brain {
            weights,
            cache,
            rows,
            before: vec![0.0; graph.total_state_size],
            after: vec![0.0; graph.total_state_size],
            output: vec![0.0; 2],
            scratch: CalculationScratch::try_new(sse.scratch_layout(), 64 * 1024 * 1024)?,
        });
    }
    let cache_bytes: usize = brains.iter().map(|b| b.cache.allocated_bytes()).sum();
    let mut results = Vec::new();
    let mut baseline_ms = 0.0;
    for plan in [&sse, &fma, &quantized] {
        brains.iter_mut().for_each(Brain::reset);
        for step in 0..warmup {
            pool.install(|| brains.par_iter_mut().for_each(|b| b.evaluate(plan, step)));
        }
        let start = Instant::now();
        for step in warmup..warmup + passes {
            pool.install(|| brains.par_iter_mut().for_each(|b| b.evaluate(plan, step)));
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / passes as f64;
        if baseline_ms == 0.0 {
            baseline_ms = ms;
        }
        results.push(json!({"backend":plan.math_backend().label(),"msPerPopulationPass":ms,"populationPassesPerSecond":1000.0/ms,"speedupVsSse2":baseline_ms/ms}));
    }
    let mut controller = Errors::default();
    let mut recurrent = Errors::default();
    let mut feedback_controller = Errors::default();
    let mut feedback_recurrent = Errors::default();
    // Compare independent sequences, including recurrent feedback, against the
    // compensated Float32 reference. No weight copy or dequantized F32 cache.
    for brain in &mut brains {
        brain.reset();
        let mut ref_before = vec![0.0; graph.total_state_size];
        let mut ref_after = ref_before.clone();
        let mut ref_output = [0.0; 2];
        let mut paired_output = [0.0; 2];
        let mut paired_state = ref_before.clone();
        let mut scratch = CalculationScratch::try_new(scalar.scratch_layout(), 64 * 1024 * 1024)?;
        for step in 0..error_steps {
            scalar.evaluate(
                &brain.weights,
                &ref_before,
                &mut ref_after,
                &brain.rows[(step % 32) * 264..(step % 32 + 1) * 264],
                &mut ref_output,
                &mut scratch.view(),
            )?;
            // Direct quantization error uses identical initial state and input;
            // separately report propagation through independent recurrent feedback.
            quantized.evaluate_quantized(
                &brain.weights,
                &brain.cache,
                &ref_before,
                &mut paired_state,
                &brain.rows[(step % 32) * 264..(step % 32 + 1) * 264],
                &mut paired_output,
                &mut scratch.view(),
            )?;
            controller.add(&paired_output, &ref_output);
            recurrent.add(&paired_state, &ref_after);
            brain.evaluate(&quantized, step);
            feedback_controller.add(&brain.output, &ref_output);
            feedback_recurrent.add(&brain.before, &ref_after);
            std::mem::swap(&mut ref_before, &mut ref_after);
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"seed":42,"rustcVersion":slither_native::native_addon_rustc_version(),"graphLayoutSha256":graph.layout_digest_sha256.iter().map(|b|format!("{b:02x}")).collect::<String>(),"sourceSha256":slither_native::native_addon_source_sha256(),"buildContractSha256":slither_native::native_addon_build_contract_sha256(),"target":slither_native::native_addon_build_target(),"brains":count,"workers":workers,"parametersPerGenome":graph.total_parameters,"matricesPerGenome":brains[0].cache.matrix_count(),"warmupPasses":warmup,"measuredPasses":passes,"errorSteps":error_steps,"conversionMs":conversion_ms,"inferenceCacheBytes":cache_bytes,"canonicalWeightBytes":count*graph.total_parameters*4,"controllerError":controller.report(),"recurrentError":recurrent.report(),"independentSequenceControllerError":feedback_controller.report(),"independentSequenceRecurrentError":feedback_recurrent.report(),"results":results,"caveat":"Production executor with seeded fresh genomes and synthetic bounded observations; inference only. Not a real-time simulation or evolved-cohort qualification. F32 FMA is a benchmark-only comparator."})
        )?
    );
    Ok(())
}
