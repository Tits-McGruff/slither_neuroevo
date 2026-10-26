//! Structural Slither/PyRL contracts. Semantic admission and execution stay in the engine.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum encoded manifest bytes; checked by the bounded serializer.
pub const MAX_TRAINING_MANIFEST_BYTES: usize = 1024 * 1024;
/// Revision of the exact deployed equations, independent of structural layout.
pub const TRAINING_EQUATION_REVISION: u32 = 1;
/// Domain for the digest of the exact canonical equation metadata below.
pub const TRAINING_EQUATION_DOMAIN: &[u8] = b"slither.training-equations.v1\0";

/// Deployment life rule for recurrent state owned by a population slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum TrainingLifeStateRule {
    Reset,
    Preserve,
}

/// Named logical view into the single canonical parameter vector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingParameterBlock {
    pub name: String,
    /// Offset and strides are measured in Float32 elements, not bytes.
    pub offset: usize,
    /// Logical element count. Strided affine views can have a larger span.
    pub length: usize,
    pub shape: Vec<usize>,
    pub strides: Vec<usize>,
}

/// Explicit hidden/cell view into the brain's packed recurrent state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingStateBlock {
    pub name: &'static str,
    pub offset: usize,
    pub length: usize,
}

/// Resolved incoming edge in semantic concatenation order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingInput {
    pub from_id: String,
    pub from_port: usize,
    pub input_ordinal: usize,
}

/// One compiler-owned node and its canonical numeric views.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingNode {
    pub id: String,
    pub operation: &'static str,
    pub input_size: usize,
    pub output_sizes: Vec<usize>,
    pub hidden_sizes: Vec<usize>,
    pub hidden_size: Option<usize>,
    pub parameter_offset: usize,
    pub parameter_length: usize,
    pub state_offset: Option<usize>,
    pub state_length: usize,
    pub inputs: Vec<TrainingInput>,
    pub parameter_blocks: Vec<TrainingParameterBlock>,
    pub state_blocks: Vec<TrainingStateBlock>,
}

/// Ordered graph output with explicit source port and flattened destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingOutput {
    pub node_id: String,
    pub port: usize,
    pub offset: usize,
    pub length: usize,
}

/// Exact Float32 equations, hashed separately from the graph topology.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingEquations {
    pub revision: u32,
    pub dtype: &'static str,
    pub affine: &'static str,
    pub gru: &'static str,
    pub lstm: &'static str,
    pub rru: &'static str,
    pub concat: &'static str,
    pub split: &'static str,
    pub arithmetic: &'static str,
}

impl TrainingEquations {
    /// Canonical formulas match the complete-graph kernels in `inference.rs`.
    pub fn canonical() -> Self {
        Self {
            revision: TRAINING_EQUATION_REVISION,
            dtype: "Float32",
            affine: "y=tanh(W*x+b); every Dense and every MLP layer, including output; W row-major with row-interleaved bias",
            gru: "z=sigmoid((Wz*x+Uz*h)+bz); r=sigmoid((Wr*x+Ur*h)+br); candidate=tanh((Wh*x+Uh*(r*h))+bh); h'=(1-z)*h+z*candidate",
            lstm: "i=sigmoid((Wi*x+Ui*h)+bi); f=sigmoid((Wf*x+Uf*h)+bf); o=sigmoid((Wo*x+Uo*h)+bo); g=tanh((Wg*x+Ug*h)+bg); c'=f*c+i*g; h'=o*tanh(c'); state=[h,c]",
            rru: "candidate=tanh((Wc*x+Uc*h)+bc); gate=sigmoid((Wr*x+Ur*h)+br); h'=(1-gate)*h+gate*candidate",
            concat: "concatenate source ports in increasing resolved inputOrdinal; fan-out reads the same source values",
            split: "contiguous source slices in outputSizes order; zero-based ports",
            arithmetic: "sigmoid(x)=1/(1+exp(-x)); Float32 intermediates; scalar/SSE2 dot accumulation may differ within parity tolerances",
        }
    }
}

/// Recorded initialization semantics; initial weights must come from Rust.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingInitialization {
    pub algorithm: &'static str,
    pub rng_scope: &'static str,
    pub dense_uniform_scale: f64,
    pub mlp_uniform_scale: f64,
    pub recurrent_input_uniform_scale: f64,
    pub recurrent_state_uniform_scale: f64,
    pub recurrent_bias_uniform_scale: f64,
    pub gru_update_bias: f64,
    pub lstm_forget_bias: f64,
    pub rru_gate_bias: f64,
}

/// Cadence and motor interpretation actually used by native deployment.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingDeploymentManifest {
    pub fixed_step_hz: u32,
    pub control_interval_seconds: f64,
    pub cadence: &'static str,
    pub first_physical_sample: &'static str,
    pub life_state_rule: TrainingLifeStateRule,
    pub generation_state_rule: &'static str,
    pub turn_output_index: usize,
    pub boost_output_index: usize,
    pub turn_mapping: &'static str,
    pub boost_mapping: &'static str,
    pub boost_threshold: f32,
}

/// Rust-generated training description. It is not an admission credential.
/// Consumers must independently validate deployed graph/weights through Rust.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalTrainingGraphManifestV1 {
    pub schema_version: u32,
    pub graph_spec: serde_json::Value,
    pub architecture_key: String,
    pub layout_version: u32,
    pub layout_sha256: String,
    pub equations: TrainingEquations,
    pub equations_sha256: String,
    pub topological_order: Vec<String>,
    pub nodes: Vec<TrainingNode>,
    pub outputs: Vec<TrainingOutput>,
    pub total_parameters: usize,
    pub total_state_size: usize,
    pub input_size: usize,
    pub output_size: usize,
    /// Exact Rust canonical sensor bytes encoded as a UTF-8 JSON string.
    pub sensor_canonical_json: String,
    pub sensor_sha256: String,
    pub deployment: TrainingDeploymentManifest,
    pub initialization: TrainingInitialization,
}

/// Bounded JSON graph root shared by production admission and offline tools.
#[derive(Deserialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreshRunGraphWire {
    #[serde(rename = "type")]
    pub graph_type: String,
    pub nodes: Vec<FreshRunGraphNodeWire>,
    pub edges: Vec<FreshRunGraphEdgeWire>,
    pub outputs: Vec<FreshRunGraphOutputWire>,
    pub output_size: usize,
}

/// One current browser graph node before conversion to compiler-owned types.
#[derive(Deserialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", deny_unknown_fields)]
pub enum FreshRunGraphNodeWire {
    Input {
        id: String,
        #[serde(rename = "outputSize")]
        output_size: usize,
    },
    Dense {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "outputSize")]
        output_size: usize,
    },
    #[serde(rename = "MLP")]
    Mlp {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "outputSize")]
        output_size: usize,
        #[serde(rename = "hiddenSizes", default)]
        hidden_sizes: Vec<usize>,
    },
    #[serde(rename = "GRU")]
    Gru {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "hiddenSize")]
        hidden_size: usize,
    },
    #[serde(rename = "LSTM")]
    Lstm {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "hiddenSize")]
        hidden_size: usize,
    },
    #[serde(rename = "RRU")]
    Rru {
        id: String,
        #[serde(rename = "inputSize")]
        input_size: usize,
        #[serde(rename = "hiddenSize")]
        hidden_size: usize,
    },
    Concat {
        id: String,
    },
    Split {
        id: String,
        #[serde(rename = "outputSizes")]
        output_sizes: Vec<usize>,
    },
}

/// One browser graph edge with optional explicit zero-based ports.
#[derive(Deserialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreshRunGraphEdgeWire {
    pub from: String,
    pub to: String,
    pub from_port: Option<i64>,
    pub to_port: Option<i64>,
}

/// One ordered browser graph output reference.
#[derive(Deserialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FreshRunGraphOutputWire {
    pub node_id: String,
    pub port: Option<i64>,
}

/// Provenance class is metadata; it never introduces another runtime snake kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum BrainSourceKind {
    Distilled,
    Imported,
    Manual,
}

/// Small immutable user/trainer provenance, distinct from Hall-of-Fame winning status.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrainProvenanceV1 {
    pub schema_version: u32,
    pub source_kind: BrainSourceKind,
    pub details: Value,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrainManifestV1 {
    pub schema_version: u32,
    pub graph_manifest: Value,
    pub provenance: BrainProvenanceV1,
}

/// Why a chunk starts; capture-start is explicitly distinct from physical birth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[repr(u8)]
pub enum TeacherStartReason {
    CaptureStart = 0,
    PhysicalLifeStart = 1,
    Continuation = 2,
    Gap = 3,
    GenerationStart = 4,
}

/// Ordinary size/time sealing does not reset recurrent continuity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[repr(u8)]
pub enum TeacherEndReason {
    MaximumSamples = 0,
    SimulatedSecond = 1,
    LifeEnded = 2,
    GenerationEnded = 3,
    ContractChanged = 4,
    CaptureStopped = 5,
    Interrupted = 6,
}

/// Stable v1 binary identities; headers use standard little-endian primitive fields.
pub const BRAIN_MAGIC: &[u8; 8] = b"SLTHBRN1";
pub const TEACHER_MAGIC: &[u8; 8] = b"SLTHTCH1";
pub const BRAIN_HEADER_FORMAT: &str = "<8sIB3xII32s32s";
pub const TEACHER_HEADER_TAIL_FORMAT: &str = "<IBdHQQIQBBH";
pub const TEACHER_SAMPLE_FORMAT: &str = "<QQfBBBH";
pub const SENSOR_HASH_DOMAIN: &[u8] = b"slither.sensor-contract.v1\0";

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
pub struct TeacherCaptureContract {
    pub capture_epoch: String,
    pub run_id: String,
    pub config_sha256: String,
    pub sensor_sha256: String,
    pub teacher_sha256: String,
    pub teacher_revision: f64,
    pub teacher_mode: f64,
    pub control_interval_seconds: f64,
    pub input_size: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
pub struct TeacherChunkDescriptor {
    pub capture_epoch: String,
    pub sequence: String,
    pub generation: String,
    pub baseline_slot: f64,
    pub physical_snake_id: String,
    pub first_sample: String,
    pub last_sample: String,
    pub first_step: String,
    pub last_step: String,
    pub count: f64,
    pub input_size: f64,
    pub config_sha256: String,
    pub sensor_sha256: String,
    pub teacher_sha256: String,
    pub start_reason: TeacherStartReason,
    pub end_reason: TeacherEndReason,
    pub byte_count: f64,
    pub sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
pub struct DistillationDataset {
    pub schema_version: u32,
    pub run_seed: Option<u32>,
    pub contract: TeacherCaptureContract,
    pub sensor_contract_json: String,
    pub state: DatasetState,
    pub production_watermark: String,
    pub storage_watermark: String,
    pub retention_floor: String,
    pub resolved_through: String,
    pub native_dropped_chunks: String,
    pub native_dropped_samples: String,
    pub native_dropped_bytes: String,
    pub spool_dropped_chunks: String,
    pub retained_chunks: f64,
    pub retained_bytes: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
pub enum DatasetState {
    Active,
    Stopped,
    Interrupted,
}

pub const CANONICAL_GRAPH_LAYOUT_VERSION: u32 = 1;

pub const CANONICAL_GRAPH_LAYOUT_DOMAIN: &[u8] = b"slither-neuroevo/graph-layout/v1\0";

pub const AUTHORITATIVE_OUTPUT_SIZE: usize = 2;

pub const MAX_BRAIN_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;

pub const BRAIN_ARTIFACT_HEADER_BYTES: usize = 88;

pub const MAX_TEACHER_SAMPLES_PER_CHUNK: usize = 256;

pub const MAX_TEACHER_INPUTS: usize = 504;

pub const MAX_TEACHER_CHUNK_BYTES: usize = 1024 * 1024;
