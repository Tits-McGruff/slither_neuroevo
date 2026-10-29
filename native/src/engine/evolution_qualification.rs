//! Cold, read-only evolutionary qualification attached to the existing native loop.
//! No authoritative RNG is drawn and no diagnostic output is fed back to control.
use super::RunningAuthorityLoop;
use crate::engine::{
    calculation::CalculationScratch,
    evolution::assessment_fitness_breakdown,
    experiment_records::selected_assessment,
    inference::{GraphExecutionPlan, InferenceMathBackend},
    state::NormalizedSettingValue,
    step_config::project_evolution_config,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

const SAMPLE_SLOTS: usize = 8;
const SAMPLE_STRATA: usize = 4;
const DIVERSITY_COORDINATES: usize = 4096;

struct Sample {
    slot: usize,
    stratum: usize,
    completed_step: u64,
    input: Vec<f32>,
    common_state: Vec<f32>,
}

/// One bounded observer per explicitly requested long qualification run.
pub(crate) struct EvolutionObserver {
    directory: PathBuf,
    stream: File,
    generation: u64,
    duration: f64,
    selected_slots: Vec<usize>,
    samples: Vec<Sample>,
    sse: GraphExecutionPlan,
    i16: GraphExecutionPlan,
    scratch: CalculationScratch,
    latest_checkpoint: Option<PathBuf>,
    sample_seconds: f64,
}

fn weights_digest(weights: &[f32]) -> String {
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 4096];
    for chunk in weights.chunks(1024) {
        for (index, value) in chunk.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_bits().to_le_bytes());
        }
        hash.update(&bytes[..chunk.len() * 4]);
    }
    format!("{:x}", hash.finalize())
}

fn moments(values: &[f64]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    let n = sorted.len();
    if n == 0 {
        return Value::Null;
    }
    json!({"count":n,"mean":values.iter().sum::<f64>()/n as f64,
        "median":if n%2==1 {sorted[n/2]} else {(sorted[n/2-1]+sorted[n/2])*0.5},
        "min":sorted[0],"max":sorted[n-1]})
}

fn errors(left: &[f32], right: &[f32]) -> Result<Value, String> {
    if left.len() != right.len() {
        return Err("diagnostic output shape".into());
    }
    if left.is_empty() {
        return Ok(json!({"samples":0,"mae":0.0,"rmse":0.0,"max":0.0}));
    }
    let mut absolute = 0.0;
    let mut squared = 0.0;
    let mut maximum = 0.0_f64;
    for (&a, &b) in left.iter().zip(right) {
        if !a.is_finite() || !b.is_finite() {
            return Err("nonfinite diagnostic inference".into());
        }
        let e = (f64::from(a) - f64::from(b)).abs();
        absolute += e;
        squared += e * e;
        maximum = maximum.max(e);
    }
    Ok(
        json!({"samples":left.len(),"mae":absolute/left.len() as f64,"rmse":(squared/left.len() as f64).sqrt(),"max":maximum}),
    )
}

impl EvolutionObserver {
    pub(crate) fn create(directory: &Path, running: &RunningAuthorityLoop) -> Result<Self, String> {
        if !matches!(
            running.authority.state().identity.math_backend.as_str(),
            "rust-sse2-v1" | "rust-i16-matrix-fma128-v1"
        ) {
            return Err("evolution study accepts only SSE2 and full matrix-i16".into());
        }
        std::fs::create_dir(directory).map_err(|e| e.to_string())?;
        let stream = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("generations.jsonl"))
            .map_err(|e| e.to_string())?;
        let graph = running.authority.graph();
        let sse = GraphExecutionPlan::build_with_math_backend(graph, InferenceMathBackend::Sse2)
            .map_err(|e| e.to_string())?;
        let i16 = GraphExecutionPlan::build_with_math_backend(
            graph,
            InferenceMathBackend::I16MatrixFma128,
        )
        .map_err(|e| e.to_string())?;
        let scratch = CalculationScratch::try_new(sse.scratch_layout(), 64 * 1024 * 1024)
            .map_err(|e| e.to_string())?;
        let state = running.authority.state();
        if state.population.is_empty() || graph.total_parameters == 0 {
            return Err("evolution diagnostics require a nonempty parameterized population".into());
        }
        let duration = state
            .config
            .settings
            .iter()
            .find(|v| v.path == "generationSeconds")
            .and_then(|v| match v.value {
                NormalizedSettingValue::Float(n) => Some(n),
                NormalizedSettingValue::Integer(n) => Some(n as f64),
                _ => None,
            })
            .ok_or("generation duration setting absent")?;
        let n = state.population.len();
        let selected_slots = (0..n.min(SAMPLE_SLOTS))
            .map(|i| i * n / n.min(SAMPLE_SLOTS))
            .collect();
        let mut result = Self {
            directory: directory.to_owned(),
            stream,
            generation: state.generation.generation,
            duration,
            selected_slots,
            samples: Vec::with_capacity(SAMPLE_SLOTS * SAMPLE_STRATA),
            sse,
            i16,
            scratch,
            latest_checkpoint: None,
            sample_seconds: 0.0,
        };
        let audit = result.population_audit(running)?;
        let settings:Vec<_>=state.config.settings.iter().map(|s|json!({"path":s.path,"value":match &s.value {
            NormalizedSettingValue::Bool(v)=>json!(v),NormalizedSettingValue::Integer(v)=>json!(v),
            NormalizedSettingValue::Float(v)=>json!(v),NormalizedSettingValue::Text(v)=>json!(v),
        }})).collect();
        let initial = json!({"nativeSourceSha256":crate::native_addon_source_sha256(),"buildIdentifier":crate::native_addon_build_identifier(),
            "buildContractSha256":crate::native_addon_build_contract_sha256(),"target":crate::native_addon_build_target(),"rustc":crate::native_addon_rustc_version(),
            "graphLayoutSha256":running.authority.graph().layout_digest_sha256.iter().map(|v|format!("{v:02x}")).collect::<String>(),
            "settings":settings,"parametersPerGenome":graph.total_parameters,
            "backend":state.identity.math_backend,"seed":state.identity.seed,"initialGeneration":result.generation,
            "generationSeconds":duration,"sampleSlots":result.selected_slots,"sampleStrata":SAMPLE_STRATA,
            "sampleMethod":"First delivered row per selected slot at or after 0/25/50/75% configured generation duration; common post-publication recurrent state. Pure paired forward diagnostics; no feedback to world.",
            "diversityCoordinates":DIVERSITY_COORDINATES,"population":audit});
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("initial.json"))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut output, &initial).map_err(|e| e.to_string())?;
        output
            .write_all(b"\n")
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(result)
    }

    /// Only a few bounded row/state copies per generation; called after publication.
    pub(crate) fn sample(&mut self, running: &RunningAuthorityLoop) -> Result<(), String> {
        let started = Instant::now();
        let state = running.authority.state();
        if state.generation.generation != self.generation {
            return Err("sample generation mismatch".into());
        }
        let stratum = ((state.generation.elapsed_seconds / self.duration * SAMPLE_STRATA as f64)
            as usize)
            .min(SAMPLE_STRATA - 1);
        if let Some((batch, _)) = running.qualification_batch() {
            for unit in batch.work() {
                let Some(slot) = unit.population_slot().map(|v| v as usize) else {
                    continue;
                };
                if !self.selected_slots.contains(&slot)
                    || self
                        .samples
                        .iter()
                        .any(|v| v.slot == slot && v.stratum == stratum)
                {
                    continue;
                }
                let Some(brain) = state
                    .brains
                    .get(unit.brain_index())
                    .filter(|v| v.handle == unit.brain())
                else {
                    continue;
                };
                let width = self.sse.input_size();
                self.samples.push(Sample {
                    slot,
                    stratum,
                    completed_step: running.completed_step(),
                    input: batch.observations()
                        [unit.ordinal() * width..(unit.ordinal() + 1) * width]
                        .to_vec(),
                    common_state: brain.recurrent.to_vec(),
                });
            }
        }
        self.sample_seconds += started.elapsed().as_secs_f64();
        Ok(())
    }

    fn population_audit(&mut self, running: &RunningAuthorityLoop) -> Result<Value, String> {
        let population = &running.authority.state().population;
        let mut genomes = Vec::with_capacity(population.len());
        let mut hashes = Vec::with_capacity(population.len());
        let mut pairs = Vec::new();
        let mut aggregate = [0.0_f64; 6]; // count, |w|, w², |error|, error², max error.
        let mut clipping = 0_u64;
        let mut endpoints = 0_u64;
        for genome in population {
            let cache = self
                .i16
                .quantize(&genome.weights)
                .map_err(|e| e.to_string())?;
            let matrices = self
                .i16
                .quantization_diagnostics(&genome.weights, &cache)
                .map_err(|e| e.to_string())?;
            for matrix in matrices.as_array().ok_or("matrix report shape")? {
                let n = matrix["length"].as_u64().ok_or("matrix length")? as f64;
                let value = |key: &str| matrix[key].as_f64().ok_or_else(|| format!("matrix {key}"));
                aggregate[0] += n;
                aggregate[1] += n * value("meanAbs")?;
                aggregate[2] += n * value("rms")?.powi(2);
                aggregate[3] += n * value("mae")?;
                aggregate[4] += n * value("rmse")?.powi(2);
                aggregate[5] = aggregate[5].max(value("maxError")?);
                clipping += matrix["clippingCount"].as_u64().ok_or("clipping count")?;
                endpoints += matrix["symmetricEndpointCount"]
                    .as_u64()
                    .ok_or("endpoint count")?;
            }
            let hash = weights_digest(&genome.weights);
            hashes.push(hash.clone());
            // Separate recurrent biases are canonical F32 and are never scale setters.
            let mut biases = Vec::new();
            for node in &running.authority.graph().nodes {
                let matrix_len: usize = matrices
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["node"].as_str() == Some(node.id.as_str()))
                    .map(|m| m["length"].as_u64().unwrap() as usize)
                    .sum();
                if matrix_len < node.parameter_length {
                    let values = &genome.weights[node.parameter_offset + matrix_len
                        ..node.parameter_offset + node.parameter_length];
                    biases.push(json!({"node":node.id,"length":values.len(),"quantized":false,
                        "maxAbs":values.iter().fold(0.0_f32,|a,v|a.max(v.abs())),
                        "rms":(values.iter().map(|v|f64::from(*v).powi(2)).sum::<f64>()/values.len() as f64).sqrt()}));
                }
            }
            for sample in self
                .samples
                .iter()
                .filter(|v| v.slot == genome.slot as usize)
            {
                let mut reference = vec![0.0; self.sse.output_size()];
                let mut candidate = reference.clone();
                let mut a = vec![0.0; self.sse.total_state_size()];
                let mut b = a.clone();
                self.sse
                    .evaluate(
                        &genome.weights,
                        &sample.common_state,
                        &mut a,
                        &sample.input,
                        &mut reference,
                        &mut self.scratch.view(),
                    )
                    .map_err(|e| e.to_string())?;
                self.i16
                    .evaluate_quantized(
                        &genome.weights,
                        &cache,
                        &sample.common_state,
                        &mut b,
                        &sample.input,
                        &mut candidate,
                        &mut self.scratch.view(),
                    )
                    .map_err(|e| e.to_string())?;
                pairs.push(json!({"slot":sample.slot,"stratum":sample.stratum,"completedStep":sample.completed_step,
                    "input":sample.input,"commonPostPublicationState":sample.common_state,"sse2Output":reference,"i16Output":candidate,
                    "controllerError":errors(&reference,&candidate)?,"oneStepStateError":errors(&a,&b)?}));
            }
            genomes.push(json!({"slot":genome.slot,"genomeId":genome.lineage.genome_id.to_string(),"canonicalWeightsSha256":hash,
                "matrices":matrices,"separateF32Biases":biases}));
        }
        let unique = hashes.iter().collect::<BTreeSet<_>>().len();
        let coordinates = population[0].weights.len().min(DIVERSITY_COORDINATES);
        let mut variance = 0.0;
        let mut zero = 0;
        for i in 0..coordinates {
            let index = i * population[0].weights.len() / coordinates;
            let mean = population
                .iter()
                .map(|g| f64::from(g.weights[index]))
                .sum::<f64>()
                / population.len() as f64;
            let v = population
                .iter()
                .map(|g| (f64::from(g.weights[index]) - mean).powi(2))
                .sum::<f64>()
                / population.len() as f64;
            variance += v;
            zero += usize::from(v == 0.0);
        }
        let rms = (aggregate[2] / aggregate[0]).sqrt();
        let rmse = (aggregate[4] / aggregate[0]).sqrt();
        Ok(
            json!({"genomes":genomes,"sampledControllerComparisons":pairs,"uniqueCanonicalWeightHashes":unique,
            "diversity":{"sampledCoordinates":coordinates,"meanCoordinateVariance":variance/coordinates as f64,"zeroVarianceCoordinates":zero},
            "aggregateQuantization":{"values":aggregate[0] as u64,"meanAbs":aggregate[1]/aggregate[0],"rms":rms,"mae":aggregate[3]/aggregate[0],
                "rmse":rmse,"maxError":aggregate[5],"relativeRmse":if rms>0.0 {Some(rmse/rms)} else {None},
                "clippingCount":clipping,"symmetricEndpointCount":endpoints}}),
        )
    }

    /// Full generation row, reduced from exact held terminal inputs and admitted successor.
    pub(crate) fn generation(
        &mut self,
        running: &RunningAuthorityLoop,
        service_wall: f64,
        generation_wall: f64,
    ) -> Result<Value, String> {
        let started = Instant::now();
        let state = running.authority.state();
        let transition = running
            .pending_generation_transition()
            .ok_or("generation row requires terminal barrier")?;
        let (world, lives, policy) = running.coordinator.qualification_terminal_inputs()?;
        let config =
            project_evolution_config(&state.config, running.authority.graph().total_parameters)
                .map_err(|e| e.to_string())?;
        let mut assessments = Vec::new();
        let mut outcomes = Vec::new();
        for life in &lives.slots {
            let snake = world
                .snakes
                .iter()
                .find(|s| {
                    s.population_slot == Some(life.population_slot) && s.id == life.physical_id
                })
                .ok_or("terminal physical owner")?;
            let entry = selected_assessment(life, snake, policy, config);
            assessments.push(entry);
            outcomes.push(json!({"slot":life.population_slot,"aliveAtBoundary":snake.alive,"censored":snake.alive,
                "physicalAgeAtBoundarySeconds":snake.age_seconds,
                "assessmentFood":entry.assessment.food,"assessmentLifetimeSeconds":entry.assessment.alive_seconds,"assessmentPoints":entry.assessment.points,
                "kills":entry.assessment.kills,"generationNetPoints":life.ledger.generation_net_earned,"generationGrossPoints":life.ledger.generation_gross_earned,
                "livesStarted":life.lives_started,"deaths":life.deaths,"successfulRespawns":life.successful_respawns}));
        }
        let maximum = assessments
            .iter()
            .fold(0.0_f64, |m, a| m.max(a.assessment.points));
        let fitness: Vec<_> = assessments
            .iter()
            .map(|&a| {
                assessment_fitness_breakdown(
                    a,
                    if maximum > 0.0 { maximum } else { 1.0 },
                    config,
                    policy.objective,
                )
                .total
            })
            .collect();
        let summary = transition.metadata().summary;
        if fitness.iter().any(|v| !v.is_finite()) {
            return Err("nonfinite diagnostic fitness".into());
        }
        let computed = moments(&fitness);
        for (actual, expected) in [
            (computed["mean"].as_f64().unwrap(), summary.average),
            (computed["max"].as_f64().unwrap(), summary.best),
            (computed["min"].as_f64().unwrap(), summary.minimum),
        ] {
            if (actual - expected).abs()
                > 128.0 * f64::EPSILON * actual.abs().max(expected.abs()).max(1.0)
            {
                return Err(
                    "diagnostic fitness disagrees with authoritative generation summary".into(),
                );
            }
        }
        let mut parent_counts = BTreeMap::<u64, usize>::new();
        let mut elite = 0;
        let mut self_pair = 0;
        for next in &transition.candidate().population {
            let a = next.lineage.parent_a.ok_or("missing next parent")?;
            if let Some(b) = next.lineage.parent_b {
                *parent_counts.entry(a).or_default() += 1;
                *parent_counts.entry(b).or_default() += 1;
                self_pair += usize::from(a == b);
            } else {
                elite += 1;
                let parent = state
                    .population
                    .iter()
                    .find(|g| g.lineage.genome_id == a)
                    .ok_or("elite parent absent")?;
                if !parent
                    .weights
                    .iter()
                    .zip(&next.weights)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
                {
                    return Err("elite F32 weights changed".into());
                }
            }
        }
        let selections = parent_counts.values().sum::<usize>() as f64;
        let concentration = parent_counts
            .values()
            .map(|&v| (v as f64 / selections).powi(2))
            .sum::<f64>();
        let audit = self.population_audit(running)?;
        let completed_lives:Vec<_>=lives.recent_life_ends.iter().map(|v|json!({"slot":v.population_slot,"lifeOrdinal":v.life_ordinal,
            "cause":format!("{:?}",v.cause),"lifetimeSeconds":v.alive_seconds,"food":v.food,"netPoints":v.life_net_points,
            "grossEarned":v.life_gross_earned,"boostSpent":v.life_boost_spent,"kills":v.kills,"finalLength":v.final_length,"peakLength":v.peak_length})).collect();
        Ok(
            json!({"generation":summary.generation,"completedStep":transition.source_key().source_completed_step()+1,
            "simulatedSeconds":transition.elapsed_seconds(),"completionReason":format!("{:?}",transition.reason()),
            "nativeServiceWallSeconds":service_wall,"generationWallSecondsBeforeDiagnostics":generation_wall,
            "fitness":computed,"fitnessBySlot":fitness,"outcomes":outcomes,
            "completedLives":completed_lives,"totalCompletedLives":lives.life_end_count,
            "completedLifeRingTruncated":lives.life_end_count>lives.recent_life_ends.len() as u64,
            "speciesCount":summary.species_count,"topSpeciesSize":summary.top_species_size,
            "averageAbsoluteWeight":summary.average_weight,"absoluteWeightVariance":summary.weight_variance,
            "reproduction":{"eliteCopies":elite,"offspring":state.population.len()-elite,"offspringFraction":(state.population.len()-elite) as f64/state.population.len() as f64,
                "uniqueSelectedParents":parent_counts.len(),"parentSelections":parent_counts,"effectiveParentCount":if concentration>0.0 {Some(1.0/concentration)} else {None},"selfParentPairs":self_pair,
                "configuredMutationRate":config.mutation_rate,"configuredMutationStd":config.mutation_std,
                "configuredRecurrentMutationRate":config.recurrent_mutation_rate,"configuredRecurrentMutationStd":config.recurrent_mutation_std,"configuredCrossoverRate":config.crossover_rate},
            "population":audit,"sampleCollectionWallSeconds":self.sample_seconds,"boundaryDiagnosticWallSeconds":started.elapsed().as_secs_f64(),
            "experimentHistoryWords":transition.commit_record().experiment.map(|v|v.history.to_vec())}),
        )
    }

    /// Preserve the ordinary checkpoint, flush its row, then retire only the previous task-owned boundary.
    pub(crate) fn commit(
        &mut self,
        mut row: Value,
        workspace: &Path,
        descriptor: &crate::engine::checkpoint::CheckpointDescriptor,
    ) -> Result<(), String> {
        let started = Instant::now();
        let source = workspace.join(&descriptor.relative_filename);
        let destination = self.directory.join(&descriptor.relative_filename);
        if destination.exists() {
            return Err("diagnostic checkpoint destination exists".into());
        }
        std::fs::rename(&source, &destination).map_err(|e| e.to_string())?;
        row["checkpointFile"] = json!(descriptor.relative_filename);
        row["checkpointLogicalRootSha256"] = json!(descriptor.logical_root_sha256);
        row["checkpointRetentionWallSeconds"] = json!(started.elapsed().as_secs_f64());
        serde_json::to_writer(&mut self.stream, &row).map_err(|e| e.to_string())?;
        self.stream
            .write_all(b"\n")
            .and_then(|_| self.stream.flush())
            .and_then(|_| self.stream.sync_all())
            .map_err(|e| e.to_string())?;
        if let Some(previous) = self.latest_checkpoint.replace(destination) {
            std::fs::remove_file(previous).map_err(|e| e.to_string())?;
        }
        self.samples.clear();
        self.generation += 1;
        self.sample_seconds = 0.0;
        Ok(())
    }
}
