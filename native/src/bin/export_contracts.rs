//! Export shared structural schemas without executing the simulator.
use serde_json::json;
use slither_native::contracts::*;

#[derive(schemars::JsonSchema)]
#[allow(dead_code)]
struct Contracts {
    training: CanonicalTrainingGraphManifestV1,
    graph: FreshRunGraphWire,
    brain: BrainManifestV1,
    teacher: TeacherCaptureContract,
    chunk: TeacherChunkDescriptor,
    dataset: DistillationDataset,
}

fn main() {
    let directory = std::env::args().nth(1).expect("output directory");
    let directory = std::path::Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let schema = schemars::schema_for!(Contracts);
    std::fs::write(
        directory.join("contracts.schema.json"),
        serde_json::to_string_pretty(&schema).unwrap() + "\n",
    )
    .unwrap();
    let constants = json!({
        "brainMagic": std::str::from_utf8(BRAIN_MAGIC).unwrap(),
        "teacherMagic": std::str::from_utf8(TEACHER_MAGIC).unwrap(),
        "brainHeader": BRAIN_HEADER_FORMAT,
        "teacherHeaderTail": TEACHER_HEADER_TAIL_FORMAT,
        "teacherSample": TEACHER_SAMPLE_FORMAT,
        "sensorHashDomain": std::str::from_utf8(SENSOR_HASH_DOMAIN).unwrap(),
        "graphLayoutVersion": CANONICAL_GRAPH_LAYOUT_VERSION,
        "graphHashDomain": std::str::from_utf8(CANONICAL_GRAPH_LAYOUT_DOMAIN).unwrap(),
        "maxManifestBytes": MAX_TRAINING_MANIFEST_BYTES,
        "maxBrainBytes": MAX_BRAIN_ARTIFACT_BYTES,
        "maxTeacherBytes": MAX_TEACHER_CHUNK_BYTES,
        "equationHashDomain": std::str::from_utf8(TRAINING_EQUATION_DOMAIN).unwrap(),
        "equations": TrainingEquations::canonical(),
        "equationFieldOrder": ["revision", "dtype", "affine", "gru", "lstm", "rru", "concat", "split", "arithmetic"],
    });
    std::fs::write(
        directory.join("constants.json"),
        serde_json::to_string_pretty(&constants).unwrap() + "\n",
    )
    .unwrap();
}
