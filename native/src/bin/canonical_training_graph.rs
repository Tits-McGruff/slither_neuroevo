//! Offline canonical metadata, initialization and whole-graph parity runner.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use slither_native::engine::genome::{initialize_random_genome, GenomeInitializationConfig};
use slither_native::engine::graph_json::compile_training_graph_json;
use slither_native::engine::inference::InferenceMathBackend;
use slither_native::engine::rng::labelled_stream;
use slither_native::engine::sensor_contract::SensorProfile;
use slither_native::engine::sensor_layout::SensorLayout;
use slither_native::engine::training_graph::{
    CanonicalTrainingGraphManifestV1, TrainingDeployment, TrainingLifeStateRule,
};
use slither_native::engine::training_runner::{
    evaluate_training_sequence, MAX_TRAINING_RUNNER_STEPS,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Explicit bounded offline request. Relative input paths resolve beside this file.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema_version: u32,
    graph_spec: serde_json::Value,
    sensor_profile: SensorProfile,
    bins: usize,
    control_interval_seconds: f64,
    life_state_rule: LifeRule,
    math_backend: String,
    /// Exactly one of weightsPath and initializationSeed is required.
    weights_path: Option<PathBuf>,
    initialization_seed: Option<u32>,
    /// Optional overrides are validated with ordinary native initializer bounds.
    initialization_biases: Option<Biases>,
    observations_path: PathBuf,
    /// Omission selects exact zero state; present files must have exact length.
    initial_state_path: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum LifeRule {
    Reset,
    Preserve,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Biases {
    gru_update_bias: f64,
    lstm_forget_bias: f64,
    rru_gate_bias: f64,
}

/// Bound both metadata and actual reads so concurrent file growth cannot bypass admission.
fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| format!("input open failed: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("offline input must be an ordinary file".into());
    }
    if file.metadata().map_err(|error| error.to_string())?.len() > maximum as u64 {
        return Err("offline input exceeds its byte limit".into());
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > maximum {
        return Err("offline input grew beyond its byte limit".into());
    }
    Ok(bytes)
}

fn read_floats(path: &Path, maximum: usize) -> Result<Vec<f32>, String> {
    let bytes = read_bounded(
        path,
        maximum.checked_mul(4).ok_or("input byte count overflow")?,
    )?;
    if !bytes.len().is_multiple_of(4) {
        return Err("Float32 input has a partial trailing value".into());
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|part| f32::from_le_bytes(*part))
        .collect())
}

/// Create one immutable output and sync its complete content before reporting success.
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())
}

/// Preserve exact Float32 bits in little-endian deployment/parity files.
fn float_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn run(request_path: &Path, output_directory: &Path) -> Result<(), String> {
    let bytes = read_bounded(request_path, 1024 * 1024)?;
    let request: Request = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid offline request: {error}"))?;
    if request.schema_version != 1 {
        return Err("unsupported offline request version".into());
    }
    if request.weights_path.is_some() == request.initialization_seed.is_some() {
        return Err("select exactly one weightsPath or initializationSeed".into());
    }
    let graph = compile_training_graph_json(
        &serde_json::to_vec(&request.graph_spec).map_err(|error| error.to_string())?,
    )?;
    let sensors = SensorLayout::for_profile(request.sensor_profile, request.bins)
        .map_err(|error| error.to_string())?;
    let initialization =
        request
            .initialization_biases
            .map_or_else(GenomeInitializationConfig::default, |bias| {
                GenomeInitializationConfig {
                    gru_update_bias: bias.gru_update_bias,
                    lstm_forget_bias: bias.lstm_forget_bias,
                    rru_gate_bias: bias.rru_gate_bias,
                }
            });
    let manifest = CanonicalTrainingGraphManifestV1::build(
        &graph,
        &sensors,
        TrainingDeployment {
            control_interval_seconds: request.control_interval_seconds,
            life_state_rule: match request.life_state_rule {
                LifeRule::Reset => TrainingLifeStateRule::Reset,
                LifeRule::Preserve => TrainingLifeStateRule::Preserve,
            },
            initialization,
        },
    )
    .map_err(|error| error.to_string())?;
    let backend = InferenceMathBackend::from_label(&request.math_backend)
        .ok_or("unsupported offline math backend")?;
    let base = request_path.parent().unwrap_or(Path::new("."));
    let weights = match request.weights_path {
        Some(path) => read_floats(&base.join(path), graph.total_parameters)?,
        None => initialize_random_genome(
            &graph,
            &labelled_stream(
                f64::from(
                    request
                        .initialization_seed
                        .expect("exclusive seed validated"),
                ),
                "canonical-training-initialization/v1",
            )
            .export_state(),
            initialization,
        )
        .map_err(|error| error.to_string())?
        .weights()
        .to_vec(),
    };
    let initial_state = match request.initial_state_path {
        Some(path) => read_floats(&base.join(path), graph.total_state_size)?,
        None => vec![0.0; graph.total_state_size],
    };
    let observations = read_floats(
        &base.join(request.observations_path),
        sensors.input_size * MAX_TRAINING_RUNNER_STEPS,
    )?;
    let result =
        evaluate_training_sequence(&graph, &weights, &observations, &initial_state, backend)?;
    let weights = float_bytes(&weights);
    let outputs = float_bytes(&result.outputs);
    let states = float_bytes(&result.recurrent_after_each_step);
    let receipt = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1, "steps": result.steps, "mathBackend": backend.label(),
        "layoutSha256": manifest.layout_sha256, "sensorSha256": manifest.sensor_sha256,
        "equationsSha256": manifest.equations_sha256,
        "weightsSha256": sha256(&weights), "outputsSha256": sha256(&outputs), "statesSha256": sha256(&states),
        "observationsSha256": sha256(&float_bytes(&observations)),
        "initialStateSha256": sha256(&float_bytes(&initial_state)),
        "initializationSeed": request.initialization_seed,
        "sourceSha256": env!("SLITHER_NATIVE_SOURCE_SHA256"),
        "buildTarget": env!("SLITHER_NATIVE_BUILD_TARGET"),
        "rustcVersion": env!("SLITHER_NATIVE_RUSTC_VERSION"),
        "floatEncoding": "Float32 little-endian, step-major; states contain continuation after every delivery"
    })).map_err(|error| error.to_string())?;
    // Claim a new directory only after all validation/inference succeeds.
    fs::create_dir(output_directory)
        .map_err(|error| format!("output directory must be new: {error}"))?;
    let files = [
        (
            "manifest.json",
            manifest
                .canonical_bytes()
                .map_err(|error| error.to_string())?,
        ),
        ("weights.f32", weights),
        ("outputs.f32", outputs),
        ("states.f32", states),
        ("receipt.json", receipt),
    ];
    for (name, bytes) in &files {
        if let Err(error) = write_bytes(&output_directory.join(name), bytes) {
            // Exact filenames in the exclusively created directory, no recursive deletion.
            for (created, _) in &files {
                let _ = fs::remove_file(output_directory.join(created));
            }
            let _ = fs::remove_dir(output_directory);
            return Err(error);
        }
    }
    println!("{}", output_directory.join("receipt.json").display());
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        eprintln!("Usage: canonical-training-graph REQUEST.json NEW_OUTPUT_DIRECTORY\nRequest v1: graphSpec, sensorProfile (v3/basic/motion), bins, controlIntervalSeconds, lifeStateRule (reset/preserve), mathBackend (rust-scalar-v1/rust-sse2-v1), observationsPath; exactly one weightsPath or initializationSeed; optional initialStatePath and initializationBiases. All numeric files are raw Float32 little-endian. Input paths resolve beside the request file. Output: manifest.json, weights.f32, outputs.f32, states.f32, receipt.json. At most 256 contiguous delivery rows. No world/control/RNG is advanced.");
        std::process::exit(2);
    }
    if let Err(error) = run(Path::new(&args[0]), Path::new(&args[1])) {
        eprintln!("canonical training graph: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "slither-training-graph-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for directory in ["first", "second"] {
                for name in [
                    "manifest.json",
                    "weights.f32",
                    "outputs.f32",
                    "states.f32",
                    "receipt.json",
                ] {
                    let _ = fs::remove_file(self.0.join(directory).join(name));
                }
                let _ = fs::remove_dir(self.0.join(directory));
            }
            for name in ["request.json", "observations.f32", "bad.f32"] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    fn request() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 1,
            "graphSpec": {
                "type": "graph", "outputSize": 2,
                "nodes": [{ "id": "input", "type": "Input", "outputSize": 51 }, { "id": "head", "type": "Dense", "inputSize": 51, "outputSize": 2 }],
                "edges": [{ "from": "input", "to": "head" }], "outputs": [{ "nodeId": "head" }]
            },
            "sensorProfile": "v3", "bins": 8, "controlIntervalSeconds": 0.03,
            "lifeStateRule": "reset", "mathBackend": "rust-scalar-v1",
            "initializationSeed": 42, "observationsPath": "observations.f32"
        })
    }

    #[test]
    fn initialized_export_and_exact_byte_reimport_use_deployed_inference() {
        let fixture = Fixture::new();
        let request_path = fixture.0.join("request.json");
        fs::write(&request_path, serde_json::to_vec(&request()).unwrap()).unwrap();
        let observations: Vec<_> = (0..51 * 256).map(|i| (i as f32 * 0.17).sin()).collect();
        fs::write(
            fixture.0.join("observations.f32"),
            float_bytes(&observations),
        )
        .unwrap();
        run(&request_path, &fixture.0.join("first")).unwrap();
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.0.join("first/receipt.json")).unwrap())
                .unwrap();
        assert_eq!(receipt["steps"], 256);
        assert_eq!(receipt["initializationSeed"], 42);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.0.join("first/manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["totalParameters"], 104);
        assert_eq!(manifest["totalStateSize"], 0);
        assert_eq!(manifest["layoutSha256"], receipt["layoutSha256"]);
        let mut reimport = request();
        reimport
            .as_object_mut()
            .unwrap()
            .remove("initializationSeed");
        reimport["weightsPath"] = serde_json::json!("first/weights.f32");
        fs::write(&request_path, serde_json::to_vec(&reimport).unwrap()).unwrap();
        run(&request_path, &fixture.0.join("second")).unwrap();
        for name in ["weights.f32", "outputs.f32", "states.f32", "manifest.json"] {
            assert_eq!(
                fs::read(fixture.0.join("first").join(name)).unwrap(),
                fs::read(fixture.0.join("second").join(name)).unwrap()
            );
        }
        let original = fs::read(fixture.0.join("second/weights.f32")).unwrap();
        assert!(run(&request_path, &fixture.0.join("second")).is_err());
        assert_eq!(
            fs::read(fixture.0.join("second/weights.f32")).unwrap(),
            original
        );
    }

    #[test]
    fn malformed_numeric_and_request_inputs_fail_before_output_claim() {
        let fixture = Fixture::new();
        let request_path = fixture.0.join("request.json");
        fs::write(fixture.0.join("observations.f32"), float_bytes(&[0.0; 51])).unwrap();
        let mut value = request();
        value["weightsPath"] = serde_json::json!("bad.f32");
        fs::write(&request_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(run(&request_path, &fixture.0.join("first"))
            .unwrap_err()
            .contains("exactly one"));
        value.as_object_mut().unwrap().remove("initializationSeed");
        fs::write(&request_path, serde_json::to_vec(&value).unwrap()).unwrap();
        fs::write(fixture.0.join("bad.f32"), [0_u8; 3]).unwrap();
        assert!(run(&request_path, &fixture.0.join("first"))
            .unwrap_err()
            .contains("trailing"));
        fs::write(fixture.0.join("bad.f32"), float_bytes(&[f32::NAN; 104])).unwrap();
        assert!(run(&request_path, &fixture.0.join("first")).is_err());
        assert!(!fixture.0.join("first").exists());
        assert!(read_bounded(&fixture.0.join("bad.f32"), 1).is_err());
    }
}
