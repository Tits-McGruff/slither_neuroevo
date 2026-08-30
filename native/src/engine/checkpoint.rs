//! Strict checkpoint-v3 encoding, restore, and immutable managed-file publication.
//!
//! The container is an intentionally small USTAR subset with fixed role paths,
//! ordinary regular files only, and `manifest.json` as the final entry. Engine
//! state, graph, and population records use explicit bounded little-endian
//! binary contracts. Only the small private manifest uses Serde.

use super::graph::{
    GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
    CANONICAL_GRAPH_LAYOUT_VERSION,
};
use super::rng::SerializedRngState;
use super::state::{
    preflight_generation_boundary_allocation, AllocatorState, AuthoritativeState, AuthorityPhase,
    BaselineRngState, BrainHandle, BrainOwner, BrainRuntimeState, ContractVersions,
    FixedStepContinuationState, GenerationBoundaryKind, GenerationBoundaryView, GenerationState,
    GenomeLineage, NormalizedEngineConfig, NormalizedSetting, NormalizedSettingValue,
    PopulationGenome, RngStateBundle, RunIdentity, StateAdmissionPolicy, StateCandidate,
    StateError, WorldState, CHECKPOINT_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::mem::{size_of, MaybeUninit};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::{Builder as TarBuilder, EntryType, Header};
use zstd::bulk::{Compressor as ZstdCompressor, Decompressor as ZstdDecompressor};
use zstd::zstd_safe;

/// Version of the bounded descriptor handed to the persistence boundary.
pub const CHECKPOINT_DESCRIPTOR_VERSION: u32 = 1;
/// Version of the strict USTAR role manifest.
pub const CHECKPOINT_CONTAINER_VERSION: u32 = 1;
/// Exact decoded bytes in each measured shuffled-Zstandard block.
pub const SHUFFLED_BLOCK_BYTES: usize = 1024 * 1024;
/// Zstandard compression level selected by the approved plan.
const ZSTD_LEVEL: i32 = 3;
/// Zstandard window-log ceiling matching the one-MiB decoded block contract.
const ZSTD_WINDOW_LOG_MAX: u32 = 20;
/// Conservative non-payload allowance for tar, buffered I/O, hash, and role-table state.
const CHECKPOINT_IO_OVERHEAD_BYTES: usize = 64 * 1024;
/// One USTAR header or alignment block.
const USTAR_BLOCK_BYTES: u64 = 512;
/// Two exact terminal zero blocks.
const USTAR_TRAILER_BYTES: u64 = USTAR_BLOCK_BYTES * 2;
/// Stage-2-compatible shuffled-frame envelope marker.
const SHUFFLED_BLOCK_MAGIC: &[u8; 4] = b"SFZ1";
/// Bytes in `magic + float_count + frame_bytes`.
const SHUFFLED_BLOCK_HEADER_BYTES: usize = 12;
/// Logical-root domain retained from the measured Stage 2 contract.
const LOGICAL_ROOT_DOMAIN: &[u8] = b"slither-neuroevo-logical-checkpoint-root\0v1\0";
/// Fixed state-role path.
const STATE_PATH: &str = "checkpoint.bin";
/// Fixed graph-role path.
const GRAPH_PATH: &str = "graph.bin";
/// Fixed population-index path.
const POPULATION_INDEX_PATH: &str = "population/index.bin";
/// Raw population-weight path.
const WEIGHTS_RAW_PATH: &str = "population/weights.f32le";
/// Shuffled-Zstandard population-weight path.
const WEIGHTS_ZSTD_PATH: &str = "population/weights.f32le.shuf4.zst";
/// Raw recurrent-state path.
const RECURRENT_RAW_PATH: &str = "population/recurrent.f32le";
/// Shuffled-Zstandard recurrent-state path.
const RECURRENT_ZSTD_PATH: &str = "population/recurrent.f32le.shuf4.zst";
/// Required final manifest path.
const MANIFEST_PATH: &str = "manifest.json";
/// State-role binary magic.
const STATE_MAGIC: &[u8; 8] = b"SLCSTV3\0";
/// Graph-role binary magic.
const GRAPH_MAGIC: &[u8; 8] = b"SLCGRV1\0";
/// Population-index binary magic.
const POPULATION_INDEX_MAGIC: &[u8; 8] = b"SLCPIv1\0";
/// Population-index contract version.
const POPULATION_INDEX_VERSION: u32 = 1;
/// Fixed bytes in one population index record.
const POPULATION_INDEX_RECORD_BYTES: u32 = 104;
/// Exact number of logical roles before the final manifest.
const LOGICAL_ROLE_COUNT: usize = 5;
/// Exact number of USTAR entries including the final manifest.
const USTAR_ENTRY_COUNT: usize = LOGICAL_ROLE_COUNT + 1;

/// Caller-reviewed pre-allocation and file-size bounds for checkpoint I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointLimits {
    /// Maximum complete managed checkpoint bytes.
    pub max_archive_bytes: u64,
    /// Maximum final JSON manifest bytes.
    pub max_manifest_bytes: usize,
    /// Maximum decoded state metadata bytes.
    pub max_state_bytes: usize,
    /// Maximum decoded graph-role bytes.
    pub max_graph_bytes: usize,
    /// Maximum decoded population-index bytes.
    pub max_population_index_bytes: usize,
    /// Maximum dense population records.
    pub max_population_count: usize,
    /// Maximum normalized settings.
    pub max_setting_count: usize,
    /// Maximum baseline RNG continuations.
    pub max_baseline_rng_count: usize,
    /// Maximum UTF-8 bytes in one decoded string.
    pub max_string_bytes: usize,
    /// Maximum aggregate UTF-8 bytes decoded from one binary role.
    pub max_total_string_bytes: usize,
    /// Maximum logical population parameter floats.
    pub max_weight_floats: usize,
    /// Maximum logical recurrent-state floats.
    pub max_recurrent_floats: usize,
    /// Maximum stored bytes for either numeric role.
    pub max_numeric_stored_bytes: u64,
    /// Maximum transient shuffled-Zstandard candidate bytes before raw is forced.
    pub max_numeric_candidate_bytes: u64,
    /// Maximum aggregate decoded logical role bytes.
    pub max_total_decoded_bytes: u64,
}

/// Validated exact operation token used for partial-file isolation and handoff correlation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointOperationId(String);

impl CheckpointOperationId {
    /// Validate a bounded printable token without accepting path syntax.
    pub fn parse(value: impl Into<String>) -> Result<Self, CheckpointError> {
        let value = value.into();
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(CheckpointError::format(
                "OPERATION_ID",
                "operation ID must be exactly 32 lowercase hexadecimal digits",
            ));
        }
        Ok(Self(value))
    }

    /// Borrow the exact validated operation token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Versioned numeric-role encoding selected from measured stored bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericEncoding {
    /// Packed four-byte little-endian Float32 values.
    RawF32LeV1,
    /// One-MiB byte-shuffled Float32 blocks in independent Zstandard frames.
    F32LeShuffle4ZstdV1,
}

impl NumericEncoding {
    /// Return the stable manifest encoding name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RawF32LeV1 => "raw-f32le-v1",
            Self::F32LeShuffle4ZstdV1 => "f32le-shuffle4-zstd-v1",
        }
    }

    /// Parse the exact stable manifest encoding name.
    fn parse(value: &str) -> Result<Self, CheckpointError> {
        match value {
            "raw-f32le-v1" => Ok(Self::RawF32LeV1),
            "f32le-shuffle4-zstd-v1" => Ok(Self::F32LeShuffle4ZstdV1),
            _ => Err(CheckpointError::format(
                "NUMERIC_ENCODING",
                format!("unsupported numeric encoding {value:?}"),
            )),
        }
    }
}

/// Exact boundary kind retained by a managed checkpoint descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointBoundaryKind {
    /// New run before its first spawn.
    RunStart,
    /// Evolved generation before its first spawn.
    Generation,
}

impl CheckpointBoundaryKind {
    /// Return the stable manifest label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunStart => "run-start",
            Self::Generation => "generation",
        }
    }

    /// Convert the validated state boundary kind.
    fn from_state(value: GenerationBoundaryKind) -> Self {
        match value {
            GenerationBoundaryKind::RunStart => Self::RunStart,
            GenerationBoundaryKind::Generation => Self::Generation,
        }
    }

    /// Parse the stable manifest label.
    fn parse(value: &str) -> Result<Self, CheckpointError> {
        match value {
            "run-start" => Ok(Self::RunStart),
            "generation" => Ok(Self::Generation),
            _ => Err(CheckpointError::format(
                "BOUNDARY_KIND",
                format!("unsupported boundary kind {value:?}"),
            )),
        }
    }
}

/// Measured write-validation policy for ordinary automatic checkpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointWriteValidationPolicy {
    /// One source hash/count pass, codec/container completion, fsync, length check, and rename.
    SinglePassLogicalHashesFsyncRenameV1,
}

impl CheckpointWriteValidationPolicy {
    /// Return the exact manifest policy name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        "write-hash-count-fsync-rename-v1"
    }

    /// Parse the only approved automatic-checkpoint policy.
    fn parse(value: &str) -> Result<Self, CheckpointError> {
        if value == Self::SinglePassLogicalHashesFsyncRenameV1.as_str() {
            Ok(Self::SinglePassLogicalHashesFsyncRenameV1)
        } else {
            Err(CheckpointError::format(
                "WRITE_POLICY",
                format!("unsupported write validation policy {value:?}"),
            ))
        }
    }
}

/// Small bounded metadata handed across the future persistence boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointDescriptor {
    /// Descriptor protocol version.
    pub protocol_version: u32,
    /// Fixed server-controlled managed root identifier.
    pub managed_root: String,
    /// Exact operation token.
    pub operation_id: CheckpointOperationId,
    /// Exact transition epoch as 16 lowercase hexadecimal digits.
    pub transition_epoch_hex: String,
    /// Bounded run identity string.
    pub run_id: String,
    /// Exact generation as 16 lowercase hexadecimal digits.
    pub generation_hex: String,
    /// Exact completed-step count as 16 lowercase hexadecimal digits.
    pub completed_step_hex: String,
    /// Exact generation-boundary kind.
    pub boundary_kind: CheckpointBoundaryKind,
    /// Managed checkpoint format version.
    pub checkpoint_format_version_hex: String,
    /// Rust authoritative-state version.
    pub state_version_hex: String,
    /// Canonical compiled-graph layout version.
    pub graph_layout_version_hex: String,
    /// Encoding-independent logical checkpoint root.
    pub logical_root_sha256: String,
    /// Fixed digest-derived filename relative to the managed directory.
    pub relative_filename: String,
    /// Complete USTAR bytes as 16 lowercase hexadecimal digits.
    pub stored_byte_count_hex: String,
    /// Aggregate decoded logical role bytes as 16 lowercase hexadecimal digits.
    pub decoded_byte_count_hex: String,
    /// Dense population count as 16 lowercase hexadecimal digits.
    pub population_count_hex: String,
    /// Fixed logical role count.
    pub role_count_hex: String,
    /// Packed population-weight Float32 count as 16 lowercase hexadecimal digits.
    pub weight_count_hex: String,
    /// Packed recurrent-state Float32 count as 16 lowercase hexadecimal digits.
    pub recurrent_state_count_hex: String,
    /// Selected population-weight encoding.
    pub weights_encoding: NumericEncoding,
    /// Selected recurrent-state encoding.
    pub recurrent_state_encoding: NumericEncoding,
    /// Compact compiled graph identity.
    pub graph_layout_sha256: String,
    /// Exact single-pass automatic-checkpoint write policy.
    pub write_validation_policy: CheckpointWriteValidationPolicy,
}

impl CheckpointDescriptor {
    /// Return the first identity-bearing field that differs from a committed echo.
    ///
    /// The final structural equality check covers every remaining bounded
    /// descriptor field. Keeping this comparison beside the descriptor prevents
    /// run-start and generation acknowledgement barriers from drifting apart.
    #[must_use]
    pub fn first_mismatch(&self, actual: &Self) -> Option<&'static str> {
        if self.protocol_version != actual.protocol_version {
            return Some("protocol version");
        }
        if self.operation_id != actual.operation_id {
            return Some("operation ID");
        }
        if self.transition_epoch_hex != actual.transition_epoch_hex {
            return Some("transition epoch");
        }
        if self.run_id != actual.run_id {
            return Some("run ID");
        }
        if self.generation_hex != actual.generation_hex {
            return Some("generation");
        }
        if self.completed_step_hex != actual.completed_step_hex {
            return Some("completed step");
        }
        if self.logical_root_sha256 != actual.logical_root_sha256 {
            return Some("logical root");
        }
        if self.relative_filename != actual.relative_filename {
            return Some("relative filename");
        }
        if self != actual {
            return Some("descriptor content");
        }
        None
    }
}

/// Immutable content facts recovered from a checkpoint without publication correlation metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointContentDescriptor {
    /// Bounded run identity string.
    pub run_id: String,
    /// Exact generation as 16 lowercase hexadecimal digits.
    pub generation_hex: String,
    /// Exact completed-step count as 16 lowercase hexadecimal digits.
    pub completed_step_hex: String,
    /// Exact generation-boundary kind.
    pub boundary_kind: CheckpointBoundaryKind,
    /// Managed checkpoint format version as exact `u64` hex.
    pub checkpoint_format_version_hex: String,
    /// Authoritative state version as exact `u64` hex.
    pub state_version_hex: String,
    /// Graph-layout version as exact `u64` hex.
    pub graph_layout_version_hex: String,
    /// Fixed controlled managed root identifier.
    pub managed_root: String,
    /// Digest-derived immutable relative filename.
    pub relative_filename: String,
    /// Encoding-independent logical root.
    pub logical_root_sha256: String,
    /// Complete stored file bytes as exact `u64` hex.
    pub stored_byte_count_hex: String,
    /// Aggregate decoded logical bytes as exact `u64` hex.
    pub decoded_byte_count_hex: String,
    /// Fixed logical role count as exact `u64` hex.
    pub role_count_hex: String,
    /// Dense population count as exact `u64` hex.
    pub population_count_hex: String,
    /// Packed population weight count as exact `u64` hex.
    pub weight_count_hex: String,
    /// Packed recurrent-state count as exact `u64` hex.
    pub recurrent_state_count_hex: String,
    /// Selected population-weight encoding.
    pub weights_encoding: NumericEncoding,
    /// Selected recurrent-state encoding.
    pub recurrent_state_encoding: NumericEncoding,
    /// Compact compiled graph identity.
    pub graph_layout_sha256: String,
    /// Exact completed write-validation policy.
    pub write_validation_policy: CheckpointWriteValidationPolicy,
}

/// Strictly decoded and fully admitted checkpoint candidate.
#[derive(Debug)]
pub struct RestoredCheckpoint {
    /// Immutable content facts reconstructed from the validated file.
    pub content: CheckpointContentDescriptor,
    /// State admitted through `AuthoritativeState::validate_and_own`.
    pub state: AuthoritativeState,
}

/// Stable checkpoint diagnosis with bounded context.
#[derive(Debug)]
pub enum CheckpointError {
    /// Filesystem or stream failure.
    Io(io::Error),
    /// Strict container/codec/record validation failure.
    Format { code: &'static str, detail: String },
    /// Candidate failed authoritative-state admission.
    State(StateError),
}

impl CheckpointError {
    /// Construct one stable bounded format diagnosis.
    fn format(code: &'static str, detail: impl Into<String>) -> Self {
        let mut detail = detail.into();
        if detail.len() > 512 {
            detail.truncate(512);
        }
        Self::Format { code, detail }
    }
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "checkpoint I/O failed: {error}"),
            Self::Format { code, detail } => write!(formatter, "checkpoint {code}: {detail}"),
            Self::State(error) => write!(formatter, "checkpoint state admission failed: {error}"),
        }
    }
}

impl Error for CheckpointError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::State(error) => Some(error),
            Self::Format { .. } => None,
        }
    }
}

impl From<io::Error> for CheckpointError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<StateError> for CheckpointError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

/// Private bounded manifest for the fixed checkpoint role set.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckpointManifest {
    magic: String,
    container_version: u32,
    archive_kind: String,
    checkpoint_format_version: u32,
    state_version: u32,
    graph_layout_version: u32,
    run_id: String,
    generation_hex: String,
    completed_step_hex: String,
    boundary_kind: String,
    logical_root_sha256: String,
    roles: Vec<ManifestRole>,
    role_stored_bytes_hex: String,
    role_decoded_bytes_hex: String,
    population_count_hex: String,
    weight_float_count_hex: String,
    recurrent_float_count_hex: String,
    weights_encoding: String,
    recurrent_encoding: String,
    graph_architecture_key: String,
    graph_layout_sha256: String,
    write_validation_policy: String,
}

/// One encoding-independent role declaration in manifest order.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestRole {
    role: String,
    path: String,
    encoding: String,
    stored_bytes_hex: String,
    decoded_bytes_hex: String,
    decoded_count_hex: String,
    record_size: u32,
    logical_sha256: String,
}

/// Encoding-independent logical tuple used by the root hash.
#[derive(Clone, Debug)]
struct LogicalRoleDigest {
    role: &'static str,
    logical_length: u64,
    logical_sha256: [u8; 32],
}

/// Fixed-role bytes prepared before USTAR assembly.
struct SmallRole {
    role: &'static str,
    path: &'static str,
    bytes: Vec<u8>,
    record_size: u32,
}

/// Borrowed flattened Float32 slices with checked aggregate count.
struct FloatSource<'a> {
    slices: Vec<&'a [f32]>,
    total_floats: usize,
}

impl<'a> FloatSource<'a> {
    /// Construct a checked flattened source without copying its Float32 values.
    fn new(
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

    /// Iterate logical Float32 values in packed-role order.
    fn values(&self) -> impl Iterator<Item = f32> + '_ {
        self.slices.iter().flat_map(|slice| slice.iter().copied())
    }

    /// Checked raw packed byte length.
    fn raw_bytes(&self) -> Result<u64, CheckpointError> {
        u64::try_from(self.total_floats)
            .ok()
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "numeric raw length overflows u64")
            })
    }
}

/// Reader that streams borrowed Float32 bits as packed little-endian bytes.
struct FloatByteReader<'a> {
    source: &'a FloatSource<'a>,
    slice_index: usize,
    value_index: usize,
    pending: [u8; 4],
    pending_offset: usize,
    pending_valid: bool,
}

impl<'a> FloatByteReader<'a> {
    /// Create a reader at the beginning of one logical numeric role.
    fn new(source: &'a FloatSource<'a>) -> Self {
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
    fn load_pending(&mut self) -> bool {
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

/// Measured candidate for one numeric role.
struct NumericCandidate {
    encoding: NumericEncoding,
    stored_bytes: u64,
    raw_bytes: u64,
    float_count: usize,
    logical_sha256: [u8; 32],
    compressed_path: Option<PathBuf>,
    /// Encoded source block count retained for codec regression evidence.
    encoded_blocks: usize,
    /// Unexpected reusable-scratch capacity changes; valid encoding keeps this zero.
    scratch_capacity_growths: usize,
}

/// Removes only operation-owned unpublished artifacts on ordinary error paths.
struct TemporaryArtifacts {
    paths: Vec<PathBuf>,
}

impl TemporaryArtifacts {
    /// Start an empty operation-local cleanup set.
    fn new() -> Self {
        Self { paths: Vec::new() }
    }

    /// Record one newly created unpublished path.
    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for TemporaryArtifacts {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// Publish one exact generation-boundary checkpoint as an immutable managed file.
///
/// Ordinary publication deliberately performs no mandatory second full decode.
/// A pre-existing content-addressed destination is the exception: it is decoded
/// and admitted before being accepted as an idempotent publication.
pub fn publish_checkpoint(
    managed_directory: &Path,
    operation_id: CheckpointOperationId,
    transition_epoch: u64,
    boundary: GenerationBoundaryView<'_>,
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<CheckpointDescriptor, CheckpointError> {
    validate_limits(limits)?;
    if transition_epoch == 0 {
        return Err(CheckpointError::format(
            "TRANSITION_EPOCH",
            "transition epoch must be positive before checkpoint publication",
        ));
    }
    fs::create_dir_all(managed_directory)?;
    let managed_directory = managed_directory.canonicalize()?;

    let state = boundary.state();
    if state.identity.run_id.is_empty()
        || state.identity.run_id.contains('\0')
        || state.identity.run_id.len() > 256
    {
        return Err(CheckpointError::format(
            "RUN_ID",
            "run ID must be nonempty UTF-8 without NUL and no more than 256 bytes",
        ));
    }
    let checkpoint_workspace = checkpoint_workspace_bound(limits, state.population.len())?;
    enforce_checkpoint_peak(
        boundary.memory_estimate().total_bytes,
        state.config.checkpoint_scratch_bytes,
        checkpoint_workspace,
        admission_policy,
    )?;
    let state_role = SmallRole {
        role: "checkpoint",
        path: STATE_PATH,
        bytes: encode_state(state, limits)?,
        record_size: 0,
    };
    let graph_role = SmallRole {
        role: "graph",
        path: GRAPH_PATH,
        bytes: encode_graph(boundary.graph_spec(), boundary.graph(), limits)?,
        record_size: 0,
    };
    let population_role = SmallRole {
        role: "population-index",
        path: POPULATION_INDEX_PATH,
        bytes: encode_population_index(
            state,
            boundary.graph().total_parameters,
            boundary.graph().total_state_size,
            limits,
        )?,
        record_size: POPULATION_INDEX_RECORD_BYTES,
    };

    let weight_source = FloatSource::new(
        state
            .population
            .iter()
            .map(|genome| genome.weights.as_ref())
            .collect(),
        limits.max_weight_floats,
        "population weight",
    )?;
    let recurrent_source = FloatSource::new(
        state
            .brains
            .iter()
            .map(|brain| brain.recurrent.as_ref())
            .collect(),
        limits.max_recurrent_floats,
        "recurrent state",
    )?;
    let preflight_decoded_bytes = [
        state_role.bytes.len() as u64,
        graph_role.bytes.len() as u64,
        population_role.bytes.len() as u64,
        weight_source.raw_bytes()?,
        recurrent_source.raw_bytes()?,
    ]
    .into_iter()
    .try_fold(0u64, |total, value| total.checked_add(value))
    .ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "preflight decoded-byte total overflows")
    })?;
    if preflight_decoded_bytes > limits.max_total_decoded_bytes {
        return Err(CheckpointError::format(
            "DECODED_LIMIT",
            format!(
                "decoded role bytes {preflight_decoded_bytes} exceed limit {}",
                limits.max_total_decoded_bytes
            ),
        ));
    }

    let mut artifacts = TemporaryArtifacts::new();
    let weights_candidate = select_numeric_candidate(
        managed_directory.as_path(),
        operation_id.as_str(),
        "weights",
        &weight_source,
        limits,
        &mut artifacts,
    )?;
    let recurrent_candidate = select_numeric_candidate(
        managed_directory.as_path(),
        operation_id.as_str(),
        "recurrent",
        &recurrent_source,
        limits,
        &mut artifacts,
    )?;

    let weights_path = numeric_path(true, weights_candidate.encoding);
    let recurrent_path = numeric_path(false, recurrent_candidate.encoding);
    let small_roles = [&state_role, &graph_role, &population_role];
    let mut role_digests = Vec::new();
    role_digests
        .try_reserve_exact(LOGICAL_ROLE_COUNT)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve logical role digest table")
        })?;
    for role in small_roles {
        role_digests.push(LogicalRoleDigest {
            role: role.role,
            logical_length: usize_to_u64(role.bytes.len(), "small role length")?,
            logical_sha256: sha256(&role.bytes),
        });
    }
    role_digests.push(LogicalRoleDigest {
        role: "population-weights",
        logical_length: weights_candidate.raw_bytes,
        logical_sha256: weights_candidate.logical_sha256,
    });
    role_digests.push(LogicalRoleDigest {
        role: "population-recurrent",
        logical_length: recurrent_candidate.raw_bytes,
        logical_sha256: recurrent_candidate.logical_sha256,
    });
    let logical_root = compute_logical_root(&role_digests)?;
    let logical_root_hex = encode_digest(logical_root);
    let relative_filename = format!("{logical_root_hex}.checkpoint-v3");
    let final_path = managed_directory.join(&relative_filename);

    let role_stored_bytes = small_roles
        .iter()
        .try_fold(0u64, |total, role| {
            total.checked_add(role.bytes.len() as u64)
        })
        .and_then(|total| total.checked_add(weights_candidate.stored_bytes))
        .and_then(|total| total.checked_add(recurrent_candidate.stored_bytes))
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "role stored-byte total overflows")
        })?;
    let role_decoded_bytes = role_digests
        .iter()
        .try_fold(0u64, |total, role| total.checked_add(role.logical_length))
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "role decoded-byte total overflows")
        })?;
    if role_decoded_bytes > limits.max_total_decoded_bytes {
        return Err(CheckpointError::format(
            "DECODED_LIMIT",
            format!(
                "decoded role bytes {role_decoded_bytes} exceed limit {}",
                limits.max_total_decoded_bytes
            ),
        ));
    }

    let roles = vec![
        manifest_role(&state_role, &role_digests[0], 1),
        manifest_role(&graph_role, &role_digests[1], 1),
        manifest_role(
            &population_role,
            &role_digests[2],
            usize_to_u64(state.population.len(), "population index count")?,
        ),
        manifest_numeric_role(
            "population-weights",
            weights_path,
            &weights_candidate,
            &role_digests[3],
        ),
        manifest_numeric_role(
            "population-recurrent",
            recurrent_path,
            &recurrent_candidate,
            &role_digests[4],
        ),
    ];
    let boundary_kind = CheckpointBoundaryKind::from_state(boundary.kind());
    let manifest = CheckpointManifest {
        magic: "slither-neuroevo-checkpoint".to_owned(),
        container_version: CHECKPOINT_CONTAINER_VERSION,
        archive_kind: "managed-checkpoint-v3".to_owned(),
        checkpoint_format_version: CHECKPOINT_VERSION,
        state_version: state.versions.state,
        graph_layout_version: boundary.graph().layout_version,
        run_id: state.identity.run_id.clone(),
        generation_hex: encode_u64_hex(state.generation.generation),
        completed_step_hex: encode_u64_hex(state.generation.completed_step),
        boundary_kind: boundary_kind.as_str().to_owned(),
        logical_root_sha256: logical_root_hex.clone(),
        roles,
        role_stored_bytes_hex: encode_u64_hex(role_stored_bytes),
        role_decoded_bytes_hex: encode_u64_hex(role_decoded_bytes),
        population_count_hex: encode_u64_hex(usize_to_u64(
            state.population.len(),
            "population count",
        )?),
        weight_float_count_hex: encode_u64_hex(usize_to_u64(
            weight_source.total_floats,
            "weight count",
        )?),
        recurrent_float_count_hex: encode_u64_hex(usize_to_u64(
            recurrent_source.total_floats,
            "recurrent count",
        )?),
        weights_encoding: weights_candidate.encoding.as_str().to_owned(),
        recurrent_encoding: recurrent_candidate.encoding.as_str().to_owned(),
        graph_architecture_key: boundary.graph().architecture_key.clone(),
        graph_layout_sha256: boundary.graph().layout_digest_hex(),
        write_validation_policy:
            CheckpointWriteValidationPolicy::SinglePassLogicalHashesFsyncRenameV1
                .as_str()
                .to_owned(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| {
        CheckpointError::format(
            "MANIFEST_JSON",
            format!("manifest encoding failed: {error}"),
        )
    })?;
    if manifest_bytes.len() > limits.max_manifest_bytes {
        return Err(CheckpointError::format(
            "MANIFEST_LIMIT",
            format!(
                "manifest bytes {} exceed limit {}",
                manifest_bytes.len(),
                limits.max_manifest_bytes
            ),
        ));
    }
    let expected_length = expected_archive_length([
        state_role.bytes.len() as u64,
        graph_role.bytes.len() as u64,
        population_role.bytes.len() as u64,
        weights_candidate.stored_bytes,
        recurrent_candidate.stored_bytes,
        manifest_bytes.len() as u64,
    ])?;
    if expected_length > limits.max_archive_bytes {
        return Err(CheckpointError::format(
            "ARCHIVE_LIMIT",
            format!(
                "preflight archive length {expected_length} exceeds limit {}",
                limits.max_archive_bytes
            ),
        ));
    }

    if final_path.exists() {
        sync_parent_directory(&managed_directory)?;
        return validate_existing_idempotent(
            &final_path,
            &operation_id,
            transition_epoch,
            state,
            boundary.graph_spec(),
            &logical_root_hex,
            limits,
            graph_limits,
            admission_policy,
        );
    }

    let partial_name = format!("checkpoint-v3-{}.partial", operation_id.as_str());
    let partial_path = managed_directory.join(partial_name);
    let partial_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial_path)?;
    artifacts.track(partial_path.clone());
    let writer = BufWriter::new(partial_file);
    let mut archive = TarBuilder::new(writer);
    for role in small_roles {
        append_ustar(
            &mut archive,
            role.path,
            role.bytes.len() as u64,
            Cursor::new(role.bytes.as_slice()),
        )?;
    }
    append_numeric_ustar(
        &mut archive,
        weights_path,
        &weights_candidate,
        &weight_source,
    )?;
    append_numeric_ustar(
        &mut archive,
        recurrent_path,
        &recurrent_candidate,
        &recurrent_source,
    )?;
    append_ustar(
        &mut archive,
        MANIFEST_PATH,
        manifest_bytes.len() as u64,
        Cursor::new(manifest_bytes.as_slice()),
    )?;
    let mut writer = archive.into_inner()?;
    writer.flush()?;
    let partial_file = writer.into_inner().map_err(|error| error.into_error())?;
    partial_file.sync_all()?;
    let actual_length = partial_file.metadata()?.len();
    if actual_length != expected_length || actual_length > limits.max_archive_bytes {
        return Err(CheckpointError::format(
            "ARCHIVE_LENGTH",
            format!("completed archive length {actual_length} does not match expected {expected_length} or configured limit"),
        ));
    }
    drop(partial_file);

    match rename_noreplace(&partial_path, &final_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists || final_path.exists() => {
            // A concurrent publisher may have completed its no-replace rename
            // but not yet synced the directory. Establish that durability here
            // before this loser can hand the shared descriptor to SQLite.
            sync_parent_directory(&managed_directory)?;
            return validate_existing_idempotent(
                &final_path,
                &operation_id,
                transition_epoch,
                state,
                boundary.graph_spec(),
                &logical_root_hex,
                limits,
                graph_limits,
                admission_policy,
            );
        }
        Err(error) => return Err(error.into()),
    }
    sync_parent_directory(&managed_directory)?;
    let content = content_descriptor_from_manifest(&manifest, actual_length, relative_filename)?;
    Ok(publication_descriptor(
        content,
        operation_id,
        transition_epoch,
    ))
}

/// Strictly decode, verify, compile, and admit one managed checkpoint.
pub fn restore_checkpoint(
    path: &Path,
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<RestoredCheckpoint, CheckpointError> {
    validate_limits(limits)?;
    let minimum_workspace = checkpoint_workspace_bound(limits, 0)?;
    if minimum_workspace > admission_policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: minimum_workspace,
            ceiling_bytes: admission_policy.memory_ceiling_bytes,
        }
        .into());
    }
    let link_metadata = fs::symlink_metadata(path)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
        return Err(CheckpointError::format(
            "MANAGED_FILE_TYPE",
            "checkpoint target must be a direct regular file, not a symlink or special file",
        ));
    }
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(CheckpointError::format(
            "MANAGED_FILE_TYPE",
            "opened checkpoint handle is not a regular file",
        ));
    }
    let archive_length = file.metadata()?.len();
    if archive_length == 0 || archive_length > limits.max_archive_bytes {
        return Err(CheckpointError::format(
            "ARCHIVE_LIMIT",
            format!(
                "archive length {archive_length} is zero or exceeds limit {}",
                limits.max_archive_bytes
            ),
        ));
    }
    let entries = scan_strict_ustar(&mut file, archive_length, limits)?;
    let manifest_entry = entries
        .last()
        .ok_or_else(|| CheckpointError::format("USTAR_EMPTY", "archive has no entries"))?;
    let manifest_bytes = read_entry_bytes(&mut file, manifest_entry, limits.max_manifest_bytes)?;
    let manifest: CheckpointManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            CheckpointError::format(
                "MANIFEST_JSON",
                format!("invalid bounded manifest JSON: {error}"),
            )
        })?;
    drop(manifest_bytes);
    let validated = validate_manifest(&manifest, &entries, limits)?;
    let predecode_workspace = checkpoint_workspace_bound(limits, validated.population_count)?;
    if predecode_workspace > admission_policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: predecode_workspace,
            ceiling_bytes: admission_policy.memory_ceiling_bytes,
        }
        .into());
    }

    let state_bytes = read_and_verify_small_role(
        &mut file,
        &entries[0],
        &validated.roles[0],
        limits.max_state_bytes,
    )?;
    let graph_bytes = read_and_verify_small_role(
        &mut file,
        &entries[1],
        &validated.roles[1],
        limits.max_graph_bytes,
    )?;
    let index_bytes = read_and_verify_small_role(
        &mut file,
        &entries[2],
        &validated.roles[2],
        limits.max_population_index_bytes,
    )?;
    let state_parts = decode_state(&state_bytes, limits)?;
    let graph = Arc::new(decode_graph(&graph_bytes, limits, graph_limits)?);
    let index = decode_population_index(
        &index_bytes,
        limits,
        graph.total_parameters,
        graph.total_state_size,
    )?;
    drop(state_bytes);
    drop(graph_bytes);
    drop(index_bytes);
    if index.records.len() != validated.population_count
        || index.weight_float_count != validated.weight_float_count
        || index.recurrent_float_count != validated.recurrent_float_count
    {
        return Err(CheckpointError::format(
            "MANIFEST_COUNT_MISMATCH",
            "manifest and population-index counts disagree",
        ));
    }
    let allocation_shell = assemble_allocation_shell(&state_parts, &index)?;
    let allocation_estimate = preflight_generation_boundary_allocation(
        &allocation_shell,
        &graph,
        validated.weight_float_count,
        validated.recurrent_float_count,
        admission_policy,
    )?;
    let checkpoint_workspace = checkpoint_workspace_bound(limits, index.records.len())?;
    enforce_checkpoint_peak(
        allocation_estimate.total_bytes,
        state_parts.config.checkpoint_scratch_bytes,
        checkpoint_workspace,
        admission_policy,
    )?;
    drop(allocation_shell);
    let weights = decode_numeric_role(
        &mut file,
        &entries[3],
        validated.weights_encoding,
        validated.weight_float_count,
        index.records.len(),
        graph.total_parameters,
        validated.roles[3].logical_sha256,
    )?;
    let recurrent = decode_numeric_role(
        &mut file,
        &entries[4],
        validated.recurrent_encoding,
        validated.recurrent_float_count,
        index.records.len(),
        graph.total_state_size,
        validated.roles[4].logical_sha256,
    )?;
    let candidate = assemble_candidate(state_parts, &index, weights, recurrent)?;
    let state =
        AuthoritativeState::validate_and_own(candidate, Arc::clone(&graph), admission_policy)?;
    crosscheck_manifest_state(&manifest, &validated, &state)?;
    let content = content_descriptor_from_manifest(
        &manifest,
        archive_length,
        path.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                CheckpointError::format("MANAGED_FILENAME", "checkpoint filename is not UTF-8")
            })?
            .to_owned(),
    )?;
    Ok(RestoredCheckpoint { content, state })
}

/// Internal manifest admission result with decoded exact counts and digests.
struct ValidatedManifest {
    roles: Vec<ValidatedRole>,
    population_count: usize,
    weight_float_count: usize,
    recurrent_float_count: usize,
    weights_encoding: NumericEncoding,
    recurrent_encoding: NumericEncoding,
}

/// One fully checked manifest role tuple.
struct ValidatedRole {
    logical_length: u64,
    logical_sha256: [u8; 32],
}

/// Population-index fields required to reconstruct the boundary candidate.
struct DecodedPopulationIndex {
    records: Vec<PopulationIndexRecord>,
    weight_float_count: usize,
    recurrent_float_count: usize,
}

/// One fixed population index record.
struct PopulationIndexRecord {
    slot: u32,
    brain_index: u32,
    brain: BrainHandle,
    lineage: GenomeLineage,
    fitness: f64,
    weight_offset: usize,
    weight_count: usize,
    recurrent_offset: usize,
    recurrent_count: usize,
}

/// Decoded non-population state fields; numeric population buffers arrive separately.
#[derive(Clone)]
struct DecodedStateParts {
    versions: ContractVersions,
    identity: RunIdentity,
    config: NormalizedEngineConfig,
    boundary_kind: GenerationBoundaryKind,
    generation: GenerationState,
    rng: RngStateBundle,
    allocators: AllocatorState,
}

/// Convert a Zstandard size estimate into one checked Rust allocation bound.
fn checked_zstd_estimate(value: usize, label: &'static str) -> Result<usize, CheckpointError> {
    // SAFETY: `ZSTD_isError` only inspects the value returned by another
    // Zstandard API and neither dereferences pointers nor retains state.
    if unsafe { zstd_safe::zstd_sys::ZSTD_isError(value) } != 0 {
        return Err(CheckpointError::format(
            "ZSTD_WORKSPACE",
            format!("unable to estimate {label}"),
        ));
    }
    Ok(value)
}

/// Calculate a conservative bound for all checkpoint-only live allocations.
///
/// The authoritative state estimate already includes the configured checkpoint
/// scratch reservation. Callers replace that declaration with this codec-derived
/// bound before comparing against the hard process admission ceiling.
fn checkpoint_workspace_bound(
    limits: &CheckpointLimits,
    population_count: usize,
) -> Result<usize, CheckpointError> {
    // SAFETY: these functions are pure size estimators from the linked Zstandard
    // library. They accept no pointers and allocate no caller-visible memory.
    let compressor_context = checked_zstd_estimate(
        unsafe { zstd_safe::zstd_sys::ZSTD_estimateCCtxSize(ZSTD_LEVEL) },
        "Zstandard compression context",
    )?;
    // SAFETY: see the preceding estimator call; this function has no arguments.
    let decompressor_context = checked_zstd_estimate(
        unsafe { zstd_safe::zstd_sys::ZSTD_estimateDCtxSize() },
        "Zstandard decompression context",
    )?;
    let frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
    let encoder_workspace = [
        SHUFFLED_BLOCK_BYTES,
        SHUFFLED_BLOCK_BYTES,
        frame_bytes,
        compressor_context,
    ]
    .into_iter()
    .try_fold(0usize, |total, value| total.checked_add(value))
    .ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "encoder workspace bound overflows")
    })?;
    let decoder_workspace = [SHUFFLED_BLOCK_BYTES, frame_bytes, decompressor_context]
        .into_iter()
        .try_fold(0usize, |total, value| total.checked_add(value))
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "decoder workspace bound overflows")
        })?;
    let role_buffers = [
        limits.max_manifest_bytes,
        limits.max_state_bytes,
        limits.max_graph_bytes,
        limits.max_population_index_bytes,
    ]
    .into_iter()
    .try_fold(0usize, |total, value| total.checked_add(value))
    .ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "small-role workspace bound overflows")
    })?;
    let source_slice_tables = population_count
        .checked_mul(2)
        .and_then(|count| count.checked_mul(size_of::<&[f32]>()));
    let per_population_transient = size_of::<PopulationIndexRecord>()
        .checked_add(2 * size_of::<Vec<f32>>())
        .and_then(|value| value.checked_add(3 * size_of::<Option<Box<[f32]>>>()));
    let population_transients =
        per_population_transient.and_then(|bytes| population_count.checked_mul(bytes));
    [
        role_buffers,
        // Raw small-role bytes coexist briefly with decoded state/graph/index
        // allocations before the exact admitted-state estimate is available.
        role_buffers,
        encoder_workspace.max(decoder_workspace),
        source_slice_tables.ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "source-slice workspace bound overflows")
        })?,
        population_transients.ok_or_else(|| {
            CheckpointError::format(
                "COUNT_OVERFLOW",
                "population transient workspace bound overflows",
            )
        })?,
        CHECKPOINT_IO_OVERHEAD_BYTES,
    ]
    .into_iter()
    .try_fold(0usize, |total, value| total.checked_add(value))
    .ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "checkpoint workspace bound overflows")
    })
}

/// Replace an untrusted/configurable scratch declaration with the actual codec bound.
fn enforce_checkpoint_peak(
    admitted_state_bytes: usize,
    declared_checkpoint_scratch: usize,
    checkpoint_workspace: usize,
    policy: &StateAdmissionPolicy,
) -> Result<(), CheckpointError> {
    let without_declared_scratch = admitted_state_bytes
        .checked_sub(declared_checkpoint_scratch)
        .ok_or_else(|| {
            CheckpointError::format(
                "MEMORY_ACCOUNTING",
                "state estimate is smaller than its checkpoint scratch declaration",
            )
        })?;
    let actual_peak = without_declared_scratch
        .checked_add(checkpoint_workspace)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "checkpoint peak memory overflows")
        })?;
    if actual_peak > policy.memory_ceiling_bytes {
        return Err(StateError::MemoryCeilingExceeded {
            estimated_bytes: actual_peak,
            ceiling_bytes: policy.memory_ceiling_bytes,
        }
        .into());
    }
    Ok(())
}

/// Validate that every caller-supplied checkpoint limit is usable.
fn validate_limits(limits: &CheckpointLimits) -> Result<(), CheckpointError> {
    let mandatory = [
        (limits.max_archive_bytes, "max_archive_bytes"),
        (limits.max_manifest_bytes as u64, "max_manifest_bytes"),
        (limits.max_state_bytes as u64, "max_state_bytes"),
        (limits.max_graph_bytes as u64, "max_graph_bytes"),
        (
            limits.max_population_index_bytes as u64,
            "max_population_index_bytes",
        ),
        (limits.max_population_count as u64, "max_population_count"),
        (limits.max_string_bytes as u64, "max_string_bytes"),
        (
            limits.max_total_string_bytes as u64,
            "max_total_string_bytes",
        ),
        (limits.max_numeric_stored_bytes, "max_numeric_stored_bytes"),
        (
            limits.max_numeric_candidate_bytes,
            "max_numeric_candidate_bytes",
        ),
        (limits.max_total_decoded_bytes, "max_total_decoded_bytes"),
    ];
    for (value, name) in mandatory {
        if value == 0 {
            return Err(CheckpointError::format(
                "INVALID_LIMITS",
                format!("{name} must be positive"),
            ));
        }
    }
    if limits.max_string_bytes > limits.max_total_string_bytes {
        return Err(CheckpointError::format(
            "INVALID_LIMITS",
            "max_string_bytes exceeds max_total_string_bytes",
        ));
    }
    Ok(())
}

/// Bounded explicit little-endian binary role encoder.
struct BinaryWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    max_string_bytes: usize,
    max_total_string_bytes: usize,
    total_string_bytes: usize,
}

impl BinaryWriter {
    /// Construct a role encoder with exact byte and text budgets.
    fn new(max_bytes: usize, limits: &CheckpointLimits) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
            max_string_bytes: limits.max_string_bytes,
            max_total_string_bytes: limits.max_total_string_bytes,
            total_string_bytes: 0,
        }
    }

    /// Reserve before extending the role buffer.
    fn reserve(&mut self, additional: usize) -> Result<(), CheckpointError> {
        let new_len = self.bytes.len().checked_add(additional).ok_or_else(|| {
            CheckpointError::format("BINARY_OVERFLOW", "binary role length overflows usize")
        })?;
        if new_len > self.max_bytes {
            return Err(CheckpointError::format(
                "BINARY_LIMIT",
                format!(
                    "binary role bytes {new_len} exceed limit {}",
                    self.max_bytes
                ),
            ));
        }
        self.bytes.try_reserve_exact(additional).map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve binary role bytes")
        })
    }

    /// Append exact bytes.
    fn raw(&mut self, value: &[u8]) -> Result<(), CheckpointError> {
        self.reserve(value.len())?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    /// Append one byte.
    fn u8(&mut self, value: u8) -> Result<(), CheckpointError> {
        self.raw(&[value])
    }

    /// Append a little-endian `u32`.
    fn u32(&mut self, value: u32) -> Result<(), CheckpointError> {
        self.raw(&value.to_le_bytes())
    }

    /// Append a little-endian `u64`.
    fn u64(&mut self, value: u64) -> Result<(), CheckpointError> {
        self.raw(&value.to_le_bytes())
    }

    /// Append a little-endian `i64`.
    fn i64(&mut self, value: i64) -> Result<(), CheckpointError> {
        self.raw(&value.to_le_bytes())
    }

    /// Append exact `f64` bits.
    fn f64(&mut self, value: f64) -> Result<(), CheckpointError> {
        self.u64(value.to_bits())
    }

    /// Append a bounded length-prefixed UTF-8 string.
    fn string(&mut self, value: &str) -> Result<(), CheckpointError> {
        if value.len() > self.max_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                format!(
                    "string bytes {} exceed limit {}",
                    value.len(),
                    self.max_string_bytes
                ),
            ));
        }
        self.total_string_bytes = self
            .total_string_bytes
            .checked_add(value.len())
            .ok_or_else(|| {
                CheckpointError::format("STRING_OVERFLOW", "aggregate string bytes overflow")
            })?;
        if self.total_string_bytes > self.max_total_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                "aggregate string bytes exceed limit",
            ));
        }
        self.u32(usize_to_u32(value.len(), "string byte length")?)?;
        self.raw(value.as_bytes())
    }

    /// Finish one already-bounded role.
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Preflighted explicit little-endian binary role decoder.
struct BinaryReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    max_string_bytes: usize,
    max_total_string_bytes: usize,
    total_string_bytes: usize,
}

impl<'a> BinaryReader<'a> {
    /// Construct a reader after the outer role-size limit has passed.
    fn new(bytes: &'a [u8], limits: &CheckpointLimits) -> Self {
        Self {
            bytes,
            offset: 0,
            max_string_bytes: limits.max_string_bytes,
            max_total_string_bytes: limits.max_total_string_bytes,
            total_string_bytes: 0,
        }
    }

    /// Borrow an exact number of remaining bytes.
    fn raw(&mut self, length: usize) -> Result<&'a [u8], CheckpointError> {
        let end = self.offset.checked_add(length).ok_or_else(|| {
            CheckpointError::format("BINARY_OVERFLOW", "binary read offset overflows")
        })?;
        let value = self.bytes.get(self.offset..end).ok_or_else(|| {
            CheckpointError::format("BINARY_TRUNCATED", "binary role ends inside a field")
        })?;
        self.offset = end;
        Ok(value)
    }

    /// Decode one byte.
    fn u8(&mut self) -> Result<u8, CheckpointError> {
        Ok(self.raw(1)?[0])
    }

    /// Decode one little-endian `u32`.
    fn u32(&mut self) -> Result<u32, CheckpointError> {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(self.raw(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    /// Decode one little-endian `u64`.
    fn u64(&mut self) -> Result<u64, CheckpointError> {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(self.raw(8)?);
        Ok(u64::from_le_bytes(bytes))
    }

    /// Decode one little-endian `i64`.
    fn i64(&mut self) -> Result<i64, CheckpointError> {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(self.raw(8)?);
        Ok(i64::from_le_bytes(bytes))
    }

    /// Decode exact `f64` bits; state admission performs semantic validation.
    fn f64(&mut self) -> Result<f64, CheckpointError> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// Decode a bounded UTF-8 string only after all declared lengths pass.
    fn string(&mut self) -> Result<String, CheckpointError> {
        let length = u32_to_usize(self.u32()?, "string byte length")?;
        if length > self.max_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                format!(
                    "declared string bytes {length} exceed limit {}",
                    self.max_string_bytes
                ),
            ));
        }
        self.total_string_bytes = self.total_string_bytes.checked_add(length).ok_or_else(|| {
            CheckpointError::format("STRING_OVERFLOW", "aggregate string bytes overflow")
        })?;
        if self.total_string_bytes > self.max_total_string_bytes {
            return Err(CheckpointError::format(
                "STRING_LIMIT",
                "aggregate declared string bytes exceed limit",
            ));
        }
        let bytes = self.raw(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| {
            CheckpointError::format("STRING_UTF8", "binary string field is not valid UTF-8")
        })?;
        Ok(value.to_owned())
    }

    /// Require exact role exhaustion.
    fn finish(self) -> Result<(), CheckpointError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(CheckpointError::format(
                "BINARY_TRAILING",
                format!(
                    "binary role has {} trailing bytes",
                    self.bytes.len() - self.offset
                ),
            ))
        }
    }
}

/// Encode the exact nonnumeric generation-boundary state.
fn encode_state(
    state: &StateCandidate,
    limits: &CheckpointLimits,
) -> Result<Vec<u8>, CheckpointError> {
    let mut output = BinaryWriter::new(limits.max_state_bytes, limits);
    output.raw(STATE_MAGIC)?;
    output.u32(CHECKPOINT_VERSION)?;
    let versions = &state.versions;
    for value in [
        versions.state,
        versions.engine,
        versions.protocol,
        versions.serializer,
        versions.sensor,
        versions.rng_bundle,
        versions.checkpoint,
        versions.graph_layout,
    ] {
        output.u32(value)?;
    }
    encode_identity(&mut output, &state.identity)?;
    encode_config(&mut output, &state.config, limits)?;
    let kind = match state.phase {
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart) => 1,
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::Generation) => 2,
        AuthorityPhase::Running => {
            return Err(CheckpointError::format(
                "DIRTY_BOUNDARY",
                "running state cannot be checkpoint encoded",
            ));
        }
    };
    output.u8(kind)?;
    encode_generation(&mut output, &state.generation)?;
    encode_rng_bundle(&mut output, &state.rng, limits)?;
    encode_allocators(&mut output, &state.allocators)?;
    Ok(output.finish())
}

/// Encode run/build identity fields in stable order.
fn encode_identity(
    output: &mut BinaryWriter,
    identity: &RunIdentity,
) -> Result<(), CheckpointError> {
    output.string(&identity.run_id)?;
    output.u32(identity.seed)?;
    output.u64(identity.config_revision)?;
    for value in [
        identity.config_hash.as_str(),
        identity.source_revision.as_str(),
        identity.engine_build_id.as_str(),
        identity.source_sha256.as_str(),
        identity.target_triple.as_str(),
        identity.build_profile.as_str(),
        identity.build_class.as_str(),
        identity.rustc_version.as_str(),
        identity.build_contract_sha256.as_str(),
        identity.math_backend.as_str(),
    ] {
        output.string(value)?;
    }
    Ok(())
}

/// Encode complete normalized configuration without a generic serialization seam.
fn encode_config(
    output: &mut BinaryWriter,
    config: &NormalizedEngineConfig,
    limits: &CheckpointLimits,
) -> Result<(), CheckpointError> {
    if config.settings.len() > limits.max_setting_count {
        return Err(CheckpointError::format(
            "SETTING_LIMIT",
            "normalized setting count exceeds checkpoint limit",
        ));
    }
    output.u32(config.version)?;
    output.u32(usize_to_u32(config.settings.len(), "setting count")?)?;
    for setting in &config.settings {
        output.string(&setting.path)?;
        match &setting.value {
            NormalizedSettingValue::Bool(value) => {
                output.u8(1)?;
                output.u8(u8::from(*value))?;
            }
            NormalizedSettingValue::Integer(value) => {
                output.u8(2)?;
                output.i64(*value)?;
            }
            NormalizedSettingValue::Float(value) => {
                output.u8(3)?;
                output.f64(*value)?;
            }
            NormalizedSettingValue::Text(value) => {
                output.u8(4)?;
                output.string(value)?;
            }
        }
    }
    output.string(&config.settings_schema_sha256)?;
    output.string(&config.graph_architecture_key)?;
    output.f64(config.fixed_step_seconds)?;
    output.f64(config.requested_sim_speed)?;
    output.f64(config.world_radius)?;
    for value in [
        config.population_count,
        config.baseline_count,
        config.max_world_snakes,
        config.max_non_population_brains,
        config.max_body_points,
        config.max_pellets,
        config.spatial_index_bytes,
        config.worker_scratch_bytes,
        config.checkpoint_scratch_bytes,
    ] {
        output.u64(usize_to_u64(value, "normalized configuration count")?)?;
    }
    output.u64(config.controller_input_hold_ms)?;
    output.u64(config.controller_disconnect_grace_ms)?;
    Ok(())
}

/// Encode the minimal scheduler/generation continuation.
fn encode_generation(
    output: &mut BinaryWriter,
    value: &GenerationState,
) -> Result<(), CheckpointError> {
    output.u32(value.boundary_version)?;
    output.u64(value.generation)?;
    output.u64(value.completed_step)?;
    output.u64(value.population_epoch)?;
    output.f64(value.elapsed_seconds)?;
    output.f64(value.wall_accumulator_seconds)?;
    output.f64(value.best_fitness_ever)
}

/// Encode all independent authoritative RNG continuations.
fn encode_rng_bundle(
    output: &mut BinaryWriter,
    value: &RngStateBundle,
    limits: &CheckpointLimits,
) -> Result<(), CheckpointError> {
    if value.baselines.len() > limits.max_baseline_rng_count {
        return Err(CheckpointError::format(
            "RNG_LIMIT",
            "baseline RNG count exceeds checkpoint limit",
        ));
    }
    output.u32(value.version)?;
    encode_rng(output, &value.world)?;
    encode_rng(output, &value.evolution)?;
    encode_rng(output, &value.external_controller)?;
    output.u32(usize_to_u32(value.baselines.len(), "baseline RNG count")?)?;
    for baseline in &value.baselines {
        output.u32(baseline.slot)?;
        encode_rng(output, &baseline.state)?;
    }
    Ok(())
}

/// Encode one lossless RNG continuation.
fn encode_rng(
    output: &mut BinaryWriter,
    value: &SerializedRngState,
) -> Result<(), CheckpointError> {
    output.string(&value.algorithm)?;
    output.u32(value.version)?;
    output.string(&value.state_hex)?;
    output.string(&value.gaussian_algorithm)?;
    output.u32(value.gaussian_version)?;
    output.u8(u8::from(value.gaussian_spare_valid))?;
    match &value.gaussian_spare_hex {
        Some(spare) => {
            output.u8(1)?;
            output.string(spare)?;
        }
        None => output.u8(0)?,
    }
    Ok(())
}

/// Encode monotonic allocator continuations.
fn encode_allocators(
    output: &mut BinaryWriter,
    value: &AllocatorState,
) -> Result<(), CheckpointError> {
    output.u32(value.version)?;
    for next in [
        value.next_entity_id,
        value.next_brain_id,
        value.next_genome_id,
        value.next_controller_lease_id,
    ] {
        output.u64(next)?;
    }
    output.u32(value.next_frame_v1_id)?;
    for next in [
        value.next_external_id,
        value.next_baseline_id,
        value.next_resurrected_id,
    ] {
        output.u64(next)?;
    }
    Ok(())
}

/// Decode the exact nonnumeric state role after its outer size/digest checks.
fn decode_state(
    bytes: &[u8],
    limits: &CheckpointLimits,
) -> Result<DecodedStateParts, CheckpointError> {
    let mut input = BinaryReader::new(bytes, limits);
    if input.raw(STATE_MAGIC.len())? != STATE_MAGIC {
        return Err(CheckpointError::format(
            "STATE_MAGIC",
            "unsupported checkpoint state role",
        ));
    }
    if input.u32()? != CHECKPOINT_VERSION {
        return Err(CheckpointError::format(
            "STATE_VERSION",
            "unsupported checkpoint state encoding",
        ));
    }
    let versions = ContractVersions {
        state: input.u32()?,
        engine: input.u32()?,
        protocol: input.u32()?,
        serializer: input.u32()?,
        sensor: input.u32()?,
        rng_bundle: input.u32()?,
        checkpoint: input.u32()?,
        graph_layout: input.u32()?,
    };
    let identity = decode_identity(&mut input)?;
    let config = decode_config(&mut input, limits)?;
    let boundary_kind = match input.u8()? {
        1 => GenerationBoundaryKind::RunStart,
        2 => GenerationBoundaryKind::Generation,
        _ => {
            return Err(CheckpointError::format(
                "BOUNDARY_KIND",
                "invalid state boundary tag",
            ))
        }
    };
    let generation = decode_generation(&mut input)?;
    let rng = decode_rng_bundle(&mut input, limits)?;
    let allocators = decode_allocators(&mut input)?;
    input.finish()?;
    Ok(DecodedStateParts {
        versions,
        identity,
        config,
        boundary_kind,
        generation,
        rng,
        allocators,
    })
}

/// Decode run/build identity fields.
fn decode_identity(input: &mut BinaryReader<'_>) -> Result<RunIdentity, CheckpointError> {
    Ok(RunIdentity {
        run_id: input.string()?,
        seed: input.u32()?,
        config_revision: input.u64()?,
        config_hash: input.string()?,
        source_revision: input.string()?,
        engine_build_id: input.string()?,
        source_sha256: input.string()?,
        target_triple: input.string()?,
        build_profile: input.string()?,
        build_class: input.string()?,
        rustc_version: input.string()?,
        build_contract_sha256: input.string()?,
        math_backend: input.string()?,
    })
}

/// Decode normalized configuration after preflighting its record count.
fn decode_config(
    input: &mut BinaryReader<'_>,
    limits: &CheckpointLimits,
) -> Result<NormalizedEngineConfig, CheckpointError> {
    let version = input.u32()?;
    let count = u32_to_usize(input.u32()?, "setting count")?;
    if count > limits.max_setting_count {
        return Err(CheckpointError::format(
            "SETTING_LIMIT",
            "declared setting count exceeds limit",
        ));
    }
    let mut settings = Vec::new();
    settings.try_reserve_exact(count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve normalized settings")
    })?;
    for _ in 0..count {
        let path = input.string()?;
        let value = match input.u8()? {
            1 => NormalizedSettingValue::Bool(match input.u8()? {
                0 => false,
                1 => true,
                _ => {
                    return Err(CheckpointError::format(
                        "SETTING_VALUE",
                        "invalid boolean byte",
                    ))
                }
            }),
            2 => NormalizedSettingValue::Integer(input.i64()?),
            3 => NormalizedSettingValue::Float(input.f64()?),
            4 => NormalizedSettingValue::Text(input.string()?),
            _ => {
                return Err(CheckpointError::format(
                    "SETTING_VALUE",
                    "invalid setting value tag",
                ))
            }
        };
        settings.push(NormalizedSetting { path, value });
    }
    Ok(NormalizedEngineConfig {
        version,
        settings,
        settings_schema_sha256: input.string()?,
        graph_architecture_key: input.string()?,
        fixed_step_seconds: input.f64()?,
        requested_sim_speed: input.f64()?,
        world_radius: input.f64()?,
        population_count: u64_to_usize(input.u64()?, "population count")?,
        baseline_count: u64_to_usize(input.u64()?, "baseline count")?,
        max_world_snakes: u64_to_usize(input.u64()?, "max world snakes")?,
        max_non_population_brains: u64_to_usize(input.u64()?, "max non-population brains")?,
        max_body_points: u64_to_usize(input.u64()?, "max body points")?,
        max_pellets: u64_to_usize(input.u64()?, "max pellets")?,
        spatial_index_bytes: u64_to_usize(input.u64()?, "spatial index bytes")?,
        worker_scratch_bytes: u64_to_usize(input.u64()?, "worker scratch bytes")?,
        checkpoint_scratch_bytes: u64_to_usize(input.u64()?, "checkpoint scratch bytes")?,
        controller_input_hold_ms: input.u64()?,
        controller_disconnect_grace_ms: input.u64()?,
    })
}

/// Decode generation continuation scalars.
fn decode_generation(input: &mut BinaryReader<'_>) -> Result<GenerationState, CheckpointError> {
    Ok(GenerationState {
        boundary_version: input.u32()?,
        generation: input.u64()?,
        completed_step: input.u64()?,
        population_epoch: input.u64()?,
        elapsed_seconds: input.f64()?,
        wall_accumulator_seconds: input.f64()?,
        best_fitness_ever: input.f64()?,
    })
}

/// Decode independent RNG continuations after count admission.
fn decode_rng_bundle(
    input: &mut BinaryReader<'_>,
    limits: &CheckpointLimits,
) -> Result<RngStateBundle, CheckpointError> {
    let version = input.u32()?;
    let world = decode_rng(input)?;
    let evolution = decode_rng(input)?;
    let external_controller = decode_rng(input)?;
    let count = u32_to_usize(input.u32()?, "baseline RNG count")?;
    if count > limits.max_baseline_rng_count {
        return Err(CheckpointError::format(
            "RNG_LIMIT",
            "declared baseline RNG count exceeds limit",
        ));
    }
    let mut baselines = Vec::new();
    baselines.try_reserve_exact(count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve baseline RNG states")
    })?;
    for _ in 0..count {
        baselines.push(BaselineRngState {
            slot: input.u32()?,
            state: decode_rng(input)?,
        });
    }
    Ok(RngStateBundle {
        version,
        world,
        evolution,
        external_controller,
        baselines,
    })
}

/// Decode one lossless RNG continuation.
fn decode_rng(input: &mut BinaryReader<'_>) -> Result<SerializedRngState, CheckpointError> {
    let algorithm = input.string()?;
    let version = input.u32()?;
    let state_hex = input.string()?;
    let gaussian_algorithm = input.string()?;
    let gaussian_version = input.u32()?;
    let gaussian_spare_valid = match input.u8()? {
        0 => false,
        1 => true,
        _ => {
            return Err(CheckpointError::format(
                "RNG_STATE",
                "invalid Gaussian-valid byte",
            ))
        }
    };
    let gaussian_spare_hex = match input.u8()? {
        0 => None,
        1 => Some(input.string()?),
        _ => {
            return Err(CheckpointError::format(
                "RNG_STATE",
                "invalid Gaussian-option byte",
            ))
        }
    };
    Ok(SerializedRngState {
        algorithm,
        version,
        state_hex,
        gaussian_algorithm,
        gaussian_version,
        gaussian_spare_valid,
        gaussian_spare_hex,
    })
}

/// Decode monotonic allocator continuations.
fn decode_allocators(input: &mut BinaryReader<'_>) -> Result<AllocatorState, CheckpointError> {
    Ok(AllocatorState {
        version: input.u32()?,
        next_entity_id: input.u64()?,
        next_brain_id: input.u64()?,
        next_genome_id: input.u64()?,
        next_controller_lease_id: input.u64()?,
        next_frame_v1_id: input.u32()?,
        next_external_id: input.u64()?,
        next_baseline_id: input.u64()?,
        next_resurrected_id: input.u64()?,
    })
}

/// Encode the source graph plus independently checked compiled identity.
fn encode_graph(
    spec: &GraphSpec,
    compiled: &super::graph::CompiledGraph,
    limits: &CheckpointLimits,
) -> Result<Vec<u8>, CheckpointError> {
    let mut output = BinaryWriter::new(limits.max_graph_bytes, limits);
    output.raw(GRAPH_MAGIC)?;
    output.u32(CANONICAL_GRAPH_LAYOUT_VERSION)?;
    output.u32(usize_to_u32(spec.nodes.len(), "graph node count")?)?;
    for node in &spec.nodes {
        output.string(&node.id)?;
        encode_graph_node_kind(&mut output, &node.kind)?;
    }
    output.u32(usize_to_u32(spec.edges.len(), "graph edge count")?)?;
    for edge in &spec.edges {
        output.string(&edge.from)?;
        output.string(&edge.to)?;
        encode_optional_i64(&mut output, edge.from_port)?;
        encode_optional_i64(&mut output, edge.to_port)?;
    }
    output.u32(usize_to_u32(spec.outputs.len(), "graph output count")?)?;
    for graph_output in &spec.outputs {
        output.string(&graph_output.node_id)?;
        encode_optional_i64(&mut output, graph_output.port)?;
    }
    output.u64(usize_to_u64(spec.output_size, "graph output size")?)?;
    output.string(&compiled.architecture_key)?;
    output.raw(&compiled.layout_digest_sha256)?;
    output.u64(usize_to_u64(
        compiled.total_parameters,
        "graph parameter count",
    )?)?;
    output.u64(usize_to_u64(
        compiled.total_state_size,
        "graph state count",
    )?)?;
    Ok(output.finish())
}

/// Encode one versioned graph operation.
fn encode_graph_node_kind(
    output: &mut BinaryWriter,
    kind: &GraphNodeKind,
) -> Result<(), CheckpointError> {
    match kind {
        GraphNodeKind::Input { output_size } => {
            output.u8(1)?;
            output.u64(usize_to_u64(*output_size, "Input output size")?)?;
        }
        GraphNodeKind::Dense {
            input_size,
            output_size,
        } => {
            output.u8(2)?;
            output.u64(usize_to_u64(*input_size, "Dense input size")?)?;
            output.u64(usize_to_u64(*output_size, "Dense output size")?)?;
        }
        GraphNodeKind::Mlp {
            input_size,
            hidden_sizes,
            output_size,
        } => {
            output.u8(3)?;
            output.u64(usize_to_u64(*input_size, "MLP input size")?)?;
            output.u32(usize_to_u32(hidden_sizes.len(), "MLP hidden count")?)?;
            for size in hidden_sizes {
                output.u64(usize_to_u64(*size, "MLP hidden size")?)?;
            }
            output.u64(usize_to_u64(*output_size, "MLP output size")?)?;
        }
        GraphNodeKind::Gru {
            input_size,
            hidden_size,
        } => {
            output.u8(4)?;
            output.u64(usize_to_u64(*input_size, "GRU input size")?)?;
            output.u64(usize_to_u64(*hidden_size, "GRU hidden size")?)?;
        }
        GraphNodeKind::Lstm {
            input_size,
            hidden_size,
        } => {
            output.u8(5)?;
            output.u64(usize_to_u64(*input_size, "LSTM input size")?)?;
            output.u64(usize_to_u64(*hidden_size, "LSTM hidden size")?)?;
        }
        GraphNodeKind::Rru {
            input_size,
            hidden_size,
        } => {
            output.u8(6)?;
            output.u64(usize_to_u64(*input_size, "RRU input size")?)?;
            output.u64(usize_to_u64(*hidden_size, "RRU hidden size")?)?;
        }
        GraphNodeKind::Concat => output.u8(7)?,
        GraphNodeKind::Split { output_sizes } => {
            output.u8(8)?;
            output.u32(usize_to_u32(output_sizes.len(), "Split output count")?)?;
            for size in output_sizes {
                output.u64(usize_to_u64(*size, "Split output size")?)?;
            }
        }
    }
    Ok(())
}

/// Encode an optional signed port without sentinel values.
fn encode_optional_i64(
    output: &mut BinaryWriter,
    value: Option<i64>,
) -> Result<(), CheckpointError> {
    match value {
        Some(value) => {
            output.u8(1)?;
            output.i64(value)
        }
        None => output.u8(0),
    }
}

/// Decode, compile, and verify the graph role before any population allocation.
fn decode_graph(
    bytes: &[u8],
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
) -> Result<GraphBundle, CheckpointError> {
    let mut input = BinaryReader::new(bytes, limits);
    if input.raw(GRAPH_MAGIC.len())? != GRAPH_MAGIC {
        return Err(CheckpointError::format(
            "GRAPH_MAGIC",
            "unsupported graph role",
        ));
    }
    if input.u32()? != CANONICAL_GRAPH_LAYOUT_VERSION {
        return Err(CheckpointError::format(
            "GRAPH_VERSION",
            "unsupported graph role version",
        ));
    }
    let node_count = u32_to_usize(input.u32()?, "graph node count")?;
    if node_count > graph_limits.max_nodes {
        return Err(CheckpointError::format(
            "GRAPH_LIMIT",
            "declared graph node count exceeds limit",
        ));
    }
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(node_count)
        .map_err(|_| CheckpointError::format("ALLOCATION", "unable to reserve graph nodes"))?;
    for _ in 0..node_count {
        nodes.push(GraphNodeSpec {
            id: input.string()?,
            kind: decode_graph_node_kind(&mut input, graph_limits)?,
        });
    }
    let edge_count = u32_to_usize(input.u32()?, "graph edge count")?;
    if edge_count > graph_limits.max_edges {
        return Err(CheckpointError::format(
            "GRAPH_LIMIT",
            "declared graph edge count exceeds limit",
        ));
    }
    let mut edges = Vec::new();
    edges
        .try_reserve_exact(edge_count)
        .map_err(|_| CheckpointError::format("ALLOCATION", "unable to reserve graph edges"))?;
    for _ in 0..edge_count {
        edges.push(GraphEdge {
            from: input.string()?,
            to: input.string()?,
            from_port: decode_optional_i64(&mut input)?,
            to_port: decode_optional_i64(&mut input)?,
        });
    }
    let output_count = u32_to_usize(input.u32()?, "graph output count")?;
    if output_count > graph_limits.max_graph_outputs {
        return Err(CheckpointError::format(
            "GRAPH_LIMIT",
            "declared graph output count exceeds limit",
        ));
    }
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(output_count)
        .map_err(|_| CheckpointError::format("ALLOCATION", "unable to reserve graph outputs"))?;
    for _ in 0..output_count {
        outputs.push(GraphOutputRef {
            node_id: input.string()?,
            port: decode_optional_i64(&mut input)?,
        });
    }
    let output_size = u64_to_usize(input.u64()?, "graph output size")?;
    let expected_architecture_key = input.string()?;
    let mut expected_layout_digest = [0u8; 32];
    expected_layout_digest.copy_from_slice(input.raw(32)?);
    let expected_parameters = u64_to_usize(input.u64()?, "graph parameter count")?;
    let expected_state = u64_to_usize(input.u64()?, "graph state count")?;
    input.finish()?;
    let bundle = GraphBundle::compile(
        GraphSpec {
            nodes,
            edges,
            outputs,
            output_size,
        },
        graph_limits,
    )
    .map_err(|error| CheckpointError::format("GRAPH_INVALID", error.to_string()))?;
    let compiled = bundle.compiled();
    if compiled.architecture_key != expected_architecture_key
        || compiled.layout_digest_sha256 != expected_layout_digest
        || compiled.total_parameters != expected_parameters
        || compiled.total_state_size != expected_state
    {
        return Err(CheckpointError::format(
            "GRAPH_IDENTITY",
            "compiled graph does not match encoded layout identity/counts",
        ));
    }
    Ok(bundle)
}

/// Decode one graph operation after its nested counts pass graph limits.
fn decode_graph_node_kind(
    input: &mut BinaryReader<'_>,
    limits: &GraphLimits,
) -> Result<GraphNodeKind, CheckpointError> {
    let width = |input: &mut BinaryReader<'_>, label: &'static str| {
        let value = u64_to_usize(input.u64()?, label)?;
        if value > limits.max_tensor_width {
            return Err(CheckpointError::format(
                "GRAPH_LIMIT",
                format!("{label} exceeds max_tensor_width"),
            ));
        }
        Ok(value)
    };
    match input.u8()? {
        1 => Ok(GraphNodeKind::Input {
            output_size: width(input, "Input output size")?,
        }),
        2 => Ok(GraphNodeKind::Dense {
            input_size: width(input, "Dense input size")?,
            output_size: width(input, "Dense output size")?,
        }),
        3 => {
            let input_size = width(input, "MLP input size")?;
            let count = u32_to_usize(input.u32()?, "MLP hidden count")?;
            if count > limits.max_mlp_hidden_layers {
                return Err(CheckpointError::format(
                    "GRAPH_LIMIT",
                    "MLP hidden count exceeds limit",
                ));
            }
            let mut hidden_sizes = Vec::new();
            hidden_sizes.try_reserve_exact(count).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve MLP hidden sizes")
            })?;
            for _ in 0..count {
                hidden_sizes.push(width(input, "MLP hidden size")?);
            }
            Ok(GraphNodeKind::Mlp {
                input_size,
                hidden_sizes,
                output_size: width(input, "MLP output size")?,
            })
        }
        4 => Ok(GraphNodeKind::Gru {
            input_size: width(input, "GRU input size")?,
            hidden_size: width(input, "GRU hidden size")?,
        }),
        5 => Ok(GraphNodeKind::Lstm {
            input_size: width(input, "LSTM input size")?,
            hidden_size: width(input, "LSTM hidden size")?,
        }),
        6 => Ok(GraphNodeKind::Rru {
            input_size: width(input, "RRU input size")?,
            hidden_size: width(input, "RRU hidden size")?,
        }),
        7 => Ok(GraphNodeKind::Concat),
        8 => {
            let count = u32_to_usize(input.u32()?, "Split output count")?;
            if count > limits.max_split_output_ports {
                return Err(CheckpointError::format(
                    "GRAPH_LIMIT",
                    "Split output count exceeds limit",
                ));
            }
            let mut output_sizes = Vec::new();
            output_sizes.try_reserve_exact(count).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve Split output sizes")
            })?;
            for _ in 0..count {
                output_sizes.push(width(input, "Split output size")?);
            }
            Ok(GraphNodeKind::Split { output_sizes })
        }
        _ => Err(CheckpointError::format(
            "GRAPH_NODE_KIND",
            "invalid graph node tag",
        )),
    }
}

/// Decode an explicitly tagged optional port.
fn decode_optional_i64(input: &mut BinaryReader<'_>) -> Result<Option<i64>, CheckpointError> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(input.i64()?)),
        _ => Err(CheckpointError::format(
            "GRAPH_PORT",
            "invalid optional-port tag",
        )),
    }
}

/// Encode the fixed population metadata/index records.
fn encode_population_index(
    state: &StateCandidate,
    parameter_count: usize,
    state_count: usize,
    limits: &CheckpointLimits,
) -> Result<Vec<u8>, CheckpointError> {
    if state.population.len() > limits.max_population_count {
        return Err(CheckpointError::format(
            "POPULATION_LIMIT",
            "population exceeds checkpoint limit",
        ));
    }
    let expected_bytes = 40usize
        .checked_add(
            state
                .population
                .len()
                .checked_mul(POPULATION_INDEX_RECORD_BYTES as usize)
                .ok_or_else(|| {
                    CheckpointError::format(
                        "INDEX_OVERFLOW",
                        "population index record bytes overflow",
                    )
                })?,
        )
        .ok_or_else(|| {
            CheckpointError::format("INDEX_OVERFLOW", "population index bytes overflow")
        })?;
    if expected_bytes > limits.max_population_index_bytes {
        return Err(CheckpointError::format(
            "INDEX_LIMIT",
            "population index exceeds checkpoint limit",
        ));
    }
    let mut brain_indices = vec![usize::MAX; state.population.len()];
    for (brain_index, brain) in state.brains.iter().enumerate() {
        let BrainOwner::PopulationSlot(slot) = brain.owner else {
            return Err(CheckpointError::format(
                "DIRTY_BOUNDARY",
                "non-population brain at checkpoint boundary",
            ));
        };
        let index = slot as usize;
        let target = brain_indices.get_mut(index).ok_or_else(|| {
            CheckpointError::format("POPULATION_INDEX", "brain owner slot is outside population")
        })?;
        if *target != usize::MAX {
            return Err(CheckpointError::format(
                "POPULATION_INDEX",
                "duplicate population brain slot",
            ));
        }
        *target = brain_index;
    }
    let weight_float_count = state
        .population
        .len()
        .checked_mul(parameter_count)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "population weight count overflows")
        })?;
    let recurrent_float_count = state.brains.len().checked_mul(state_count).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "recurrent state count overflows")
    })?;
    let mut output = BinaryWriter::new(limits.max_population_index_bytes, limits);
    output.raw(POPULATION_INDEX_MAGIC)?;
    output.u32(POPULATION_INDEX_VERSION)?;
    output.u32(POPULATION_INDEX_RECORD_BYTES)?;
    output.u64(usize_to_u64(state.population.len(), "population count")?)?;
    output.u64(usize_to_u64(weight_float_count, "weight float count")?)?;
    output.u64(usize_to_u64(
        recurrent_float_count,
        "recurrent float count",
    )?)?;
    for genome in &state.population {
        let brain_index = *brain_indices.get(genome.slot as usize).ok_or_else(|| {
            CheckpointError::format("POPULATION_INDEX", "population slot lacks a brain index")
        })?;
        if brain_index == usize::MAX {
            return Err(CheckpointError::format(
                "POPULATION_INDEX",
                "population slot lacks a brain",
            ));
        }
        let brain = &state.brains[brain_index];
        let weight_offset = (genome.slot as usize)
            .checked_mul(parameter_count)
            .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "weight offset overflows"))?;
        let recurrent_offset = brain_index.checked_mul(state_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "recurrent offset overflows")
        })?;
        output.u32(genome.slot)?;
        output.u32(usize_to_u32(brain_index, "brain index")?)?;
        output.u64(genome.brain.id)?;
        output.u64(genome.brain.epoch)?;
        output.u64(genome.lineage.genome_id)?;
        output.u64(genome.lineage.birth_generation)?;
        let parent_flags = u64::from(genome.lineage.parent_a.is_some())
            | (u64::from(genome.lineage.parent_b.is_some()) << 1);
        output.u64(parent_flags)?;
        output.u64(genome.lineage.parent_a.unwrap_or(0))?;
        output.u64(genome.lineage.parent_b.unwrap_or(0))?;
        output.u64(genome.fitness.to_bits())?;
        output.u64(usize_to_u64(weight_offset, "weight offset")?)?;
        output.u64(usize_to_u64(parameter_count, "weight count")?)?;
        output.u64(usize_to_u64(recurrent_offset, "recurrent offset")?)?;
        output.u64(usize_to_u64(brain.recurrent.len(), "recurrent count")?)?;
    }
    let bytes = output.finish();
    if bytes.len() != expected_bytes {
        return Err(CheckpointError::format(
            "INDEX_LENGTH",
            "population index writer length mismatch",
        ));
    }
    Ok(bytes)
}

/// Decode and preflight every population record before numeric payload allocation.
fn decode_population_index(
    bytes: &[u8],
    limits: &CheckpointLimits,
    parameter_count: usize,
    state_count: usize,
) -> Result<DecodedPopulationIndex, CheckpointError> {
    let mut input = BinaryReader::new(bytes, limits);
    if input.raw(POPULATION_INDEX_MAGIC.len())? != POPULATION_INDEX_MAGIC {
        return Err(CheckpointError::format(
            "INDEX_MAGIC",
            "unsupported population index",
        ));
    }
    if input.u32()? != POPULATION_INDEX_VERSION || input.u32()? != POPULATION_INDEX_RECORD_BYTES {
        return Err(CheckpointError::format(
            "INDEX_VERSION",
            "unsupported population index version/record size",
        ));
    }
    let population_count = u64_to_usize(input.u64()?, "population index count")?;
    let weight_float_count = u64_to_usize(input.u64()?, "population weight count")?;
    let recurrent_float_count = u64_to_usize(input.u64()?, "population recurrent count")?;
    if population_count > limits.max_population_count
        || weight_float_count > limits.max_weight_floats
        || recurrent_float_count > limits.max_recurrent_floats
    {
        return Err(CheckpointError::format(
            "INDEX_LIMIT",
            "population index counts exceed limits",
        ));
    }
    let expected_length = 40usize
        .checked_add(
            population_count
                .checked_mul(POPULATION_INDEX_RECORD_BYTES as usize)
                .ok_or_else(|| {
                    CheckpointError::format("INDEX_OVERFLOW", "population record bytes overflow")
                })?,
        )
        .ok_or_else(|| {
            CheckpointError::format("INDEX_OVERFLOW", "population index length overflows")
        })?;
    if bytes.len() != expected_length {
        return Err(CheckpointError::format(
            "INDEX_LENGTH",
            "population index length does not match declared count",
        ));
    }
    if weight_float_count
        != population_count
            .checked_mul(parameter_count)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "expected population weights overflow")
            })?
        || recurrent_float_count
            != population_count.checked_mul(state_count).ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "expected recurrent state overflows")
            })?
    {
        return Err(CheckpointError::format(
            "INDEX_COUNT",
            "population numeric counts disagree with graph layout",
        ));
    }
    let mut records = Vec::new();
    records.try_reserve_exact(population_count).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve population records")
    })?;
    let mut seen_brains = vec![false; population_count];
    for index in 0..population_count {
        let slot = input.u32()?;
        let brain_index = input.u32()?;
        let brain_index_usize = brain_index as usize;
        if slot as usize != index
            || brain_index_usize >= population_count
            || seen_brains[brain_index_usize]
        {
            return Err(CheckpointError::format(
                "INDEX_DENSE",
                "slot or brain index is not dense and unique",
            ));
        }
        seen_brains[brain_index_usize] = true;
        let brain = BrainHandle {
            id: input.u64()?,
            epoch: input.u64()?,
        };
        let genome_id = input.u64()?;
        let birth_generation = input.u64()?;
        let parent_flags = input.u64()?;
        let parent_a_raw = input.u64()?;
        let parent_b_raw = input.u64()?;
        if parent_flags & !3 != 0
            || (parent_flags & 1 == 0) != (parent_a_raw == 0)
            || (parent_flags & 2 == 0) != (parent_b_raw == 0)
        {
            return Err(CheckpointError::format(
                "INDEX_PARENT",
                "invalid parent flags/identities",
            ));
        }
        let fitness = f64::from_bits(input.u64()?);
        let weight_offset = u64_to_usize(input.u64()?, "weight offset")?;
        let weight_count = u64_to_usize(input.u64()?, "weight count")?;
        let recurrent_offset = u64_to_usize(input.u64()?, "recurrent offset")?;
        let recurrent_count = u64_to_usize(input.u64()?, "recurrent count")?;
        let expected_weight_offset = index.checked_mul(parameter_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "expected weight offset overflows")
        })?;
        let expected_recurrent_offset =
            brain_index_usize.checked_mul(state_count).ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "expected recurrent offset overflows")
            })?;
        if weight_offset != expected_weight_offset
            || weight_count != parameter_count
            || recurrent_offset != expected_recurrent_offset
            || recurrent_count != state_count
        {
            return Err(CheckpointError::format(
                "INDEX_RANGE",
                "population numeric range is not canonical",
            ));
        }
        records.push(PopulationIndexRecord {
            slot,
            brain_index,
            brain,
            lineage: GenomeLineage {
                genome_id,
                birth_generation,
                parent_a: (parent_flags & 1 != 0).then_some(parent_a_raw),
                parent_b: (parent_flags & 2 != 0).then_some(parent_b_raw),
            },
            fitness,
            weight_offset,
            weight_count,
            recurrent_offset,
            recurrent_count,
        });
    }
    input.finish()?;
    Ok(DecodedPopulationIndex {
        records,
        weight_float_count,
        recurrent_float_count,
    })
}

/// Stream one shuffled-Zstandard candidate to an operation-local bounded file.
fn select_numeric_candidate(
    directory: &Path,
    operation_id: &str,
    label: &'static str,
    source: &FloatSource<'_>,
    limits: &CheckpointLimits,
    artifacts: &mut TemporaryArtifacts,
) -> Result<NumericCandidate, CheckpointError> {
    let raw_bytes = source.raw_bytes()?;
    if raw_bytes > limits.max_total_decoded_bytes {
        return Err(CheckpointError::format(
            "DECODED_LIMIT",
            format!("{label} raw bytes exceed aggregate decoded limit"),
        ));
    }
    if source.total_floats == 0 {
        return Ok(NumericCandidate {
            encoding: NumericEncoding::RawF32LeV1,
            stored_bytes: 0,
            raw_bytes: 0,
            float_count: 0,
            logical_sha256: sha256(&[]),
            compressed_path: None,
            encoded_blocks: 0,
            scratch_capacity_growths: 0,
        });
    }
    let candidate_path = directory.join(format!(".{operation_id}.{label}.codec.partial"));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&candidate_path)?;
    artifacts.track(candidate_path.clone());
    let mut writer = BufWriter::new(file);
    let mut compressor = ZstdCompressor::new(ZSTD_LEVEL)?;
    compressor.include_checksum(false)?;
    compressor.include_contentsize(true)?;
    compressor.window_log(ZSTD_WINDOW_LOG_MAX)?;
    let mut hasher = Sha256::new();
    let mut values = source.values();
    let block_float_limit = SHUFFLED_BLOCK_BYTES / 4;
    let mut encoded_floats = 0usize;
    let mut candidate_bytes = 0u64;
    let mut candidate_abandoned = false;
    let mut encoded_blocks = 0usize;
    let mut scratch_capacity_growths = 0usize;
    let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
    let mut raw = Vec::new();
    raw.try_reserve_exact(SHUFFLED_BLOCK_BYTES).map_err(|_| {
        CheckpointError::format(
            "ALLOCATION",
            "unable to reserve reusable numeric input scratch",
        )
    })?;
    let mut shuffled = Vec::new();
    shuffled
        .try_reserve_exact(SHUFFLED_BLOCK_BYTES)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve reusable shuffled scratch")
        })?;
    let mut frame = Vec::new();
    frame.try_reserve_exact(max_frame_bytes).map_err(|_| {
        CheckpointError::format(
            "ALLOCATION",
            "unable to reserve reusable Zstandard frame scratch",
        )
    })?;
    let initial_capacities = (raw.capacity(), shuffled.capacity(), frame.capacity());
    loop {
        let remaining = source.total_floats.saturating_sub(encoded_floats);
        if remaining == 0 {
            break;
        }
        let count = remaining.min(block_float_limit);
        let block_bytes = count.checked_mul(4).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "numeric block byte count overflows")
        })?;
        raw.clear();
        for _ in 0..count {
            let value = values.next().ok_or_else(|| {
                CheckpointError::format(
                    "NUMERIC_SOURCE",
                    "numeric source ended before declared count",
                )
            })?;
            raw.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        hasher.update(&raw);
        if !candidate_abandoned {
            shuffle_f32_bytes_into(&raw, &mut shuffled)?;
            frame.clear();
            let written = compressor.compress_to_buffer(&shuffled, &mut frame)?;
            if written != frame.len() || frame.len() > max_frame_bytes {
                return Err(CheckpointError::format(
                    "ZSTD_FRAME",
                    "writer produced an invalid bounded frame length",
                ));
            }
            validate_zstd_frame_header(&frame, block_bytes)?;
            let next_candidate_bytes = candidate_bytes
                .checked_add(SHUFFLED_BLOCK_HEADER_BYTES as u64)
                .and_then(|value| value.checked_add(frame.len() as u64))
                .ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "codec candidate bytes overflow")
                })?;
            if next_candidate_bytes > limits.max_numeric_candidate_bytes {
                candidate_abandoned = true;
            } else {
                writer.write_all(SHUFFLED_BLOCK_MAGIC)?;
                writer
                    .write_all(&usize_to_u32(count, "shuffled block float count")?.to_le_bytes())?;
                writer.write_all(
                    &usize_to_u32(frame.len(), "Zstandard frame bytes")?.to_le_bytes(),
                )?;
                writer.write_all(&frame)?;
                candidate_bytes = next_candidate_bytes;
            }
        }
        encoded_floats = encoded_floats.checked_add(count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "encoded numeric count overflows")
        })?;
        encoded_blocks += 1;
        if (raw.capacity(), shuffled.capacity(), frame.capacity()) != initial_capacities {
            scratch_capacity_growths += 1;
        }
    }
    if values.next().is_some() || encoded_floats != source.total_floats {
        return Err(CheckpointError::format(
            "NUMERIC_SOURCE",
            "numeric source count changed during encoding",
        ));
    }
    writer.flush()?;
    let file = writer.into_inner().map_err(|error| error.into_error())?;
    let actual_candidate_bytes = file.metadata()?.len();
    drop(file);
    if actual_candidate_bytes != candidate_bytes {
        return Err(CheckpointError::format(
            "NUMERIC_CANDIDATE",
            "codec candidate file length disagrees with streamed count",
        ));
    }
    let logical_sha256: [u8; 32] = hasher.finalize().into();
    if !candidate_abandoned
        && candidate_bytes < raw_bytes
        && candidate_bytes <= limits.max_numeric_stored_bytes
    {
        Ok(NumericCandidate {
            encoding: NumericEncoding::F32LeShuffle4ZstdV1,
            stored_bytes: candidate_bytes,
            raw_bytes,
            float_count: source.total_floats,
            logical_sha256,
            compressed_path: Some(candidate_path),
            encoded_blocks,
            scratch_capacity_growths,
        })
    } else {
        fs::remove_file(&candidate_path)?;
        if raw_bytes > limits.max_numeric_stored_bytes {
            return Err(CheckpointError::format(
                "NUMERIC_STORED_LIMIT",
                format!("{label} raw bytes exceed numeric stored limit"),
            ));
        }
        Ok(NumericCandidate {
            encoding: NumericEncoding::RawF32LeV1,
            stored_bytes: raw_bytes,
            raw_bytes,
            float_count: source.total_floats,
            logical_sha256,
            compressed_path: None,
            encoded_blocks,
            scratch_capacity_growths,
        })
    }
}

/// Shuffle packed Float32 bytes into four contiguous byte planes using reusable scratch.
fn shuffle_f32_bytes_into(raw: &[u8], shuffled: &mut Vec<u8>) -> Result<(), CheckpointError> {
    if !raw.len().is_multiple_of(4) || raw.len() > SHUFFLED_BLOCK_BYTES {
        return Err(CheckpointError::format(
            "SHUFFLED_BLOCK",
            "invalid raw Float32 block length",
        ));
    }
    let count = raw.len() / 4;
    shuffled.clear();
    if shuffled.capacity() < raw.len() {
        return Err(CheckpointError::format(
            "SHUFFLED_SCRATCH",
            "reusable shuffled scratch capacity is too small",
        ));
    }
    shuffled.resize(raw.len(), 0);
    for byte_index in 0..4 {
        for value_index in 0..count {
            shuffled[byte_index * count + value_index] = raw[value_index * 4 + byte_index];
        }
    }
    Ok(())
}

/// Final packed buffers allocated in their authoritative per-brain ownership shape.
struct SegmentedF32Builder {
    segments: Vec<Vec<f32>>,
    segment_length: usize,
    total_expected: usize,
    total_written: usize,
}

impl SegmentedF32Builder {
    /// Preflight and allocate each final segment before decompression starts.
    fn new(
        segment_count: usize,
        segment_length: usize,
        expected: usize,
    ) -> Result<Self, CheckpointError> {
        if segment_count.checked_mul(segment_length) != Some(expected) {
            return Err(CheckpointError::format(
                "NUMERIC_SEGMENTS",
                "numeric segment shape disagrees with expected float count",
            ));
        }
        let mut segments = Vec::new();
        segments.try_reserve_exact(segment_count).map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve numeric segments")
        })?;
        for _ in 0..segment_count {
            let mut segment = Vec::new();
            segment.try_reserve_exact(segment_length).map_err(|_| {
                CheckpointError::format("ALLOCATION", "unable to reserve numeric segment floats")
            })?;
            segments.push(segment);
        }
        Ok(Self {
            segments,
            segment_length,
            total_expected: expected,
            total_written: 0,
        })
    }

    /// Append one decoded bit pattern to its final segment.
    fn push_bits(&mut self, bits: u32) -> Result<(), CheckpointError> {
        if self.total_written >= self.total_expected {
            return Err(CheckpointError::format(
                "NUMERIC_COUNT",
                "decoded numeric role exceeds declared count",
            ));
        }
        if self.segment_length == 0 {
            return Err(CheckpointError::format(
                "NUMERIC_SEGMENTS",
                "nonempty payload has zero-length segments",
            ));
        }
        let segment_index = self.total_written / self.segment_length;
        self.segments[segment_index].push(f32::from_bits(bits));
        self.total_written += 1;
        Ok(())
    }

    /// Finish only after every segment received its exact count.
    fn finish(self) -> Result<Vec<Box<[f32]>>, CheckpointError> {
        if self.total_written != self.total_expected
            || self
                .segments
                .iter()
                .any(|segment| segment.len() != self.segment_length)
        {
            return Err(CheckpointError::format(
                "NUMERIC_COUNT",
                "decoded numeric count is incomplete",
            ));
        }
        Ok(self
            .segments
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect())
    }
}

/// Inspect and admit one complete ordinary Zstandard frame before output allocation.
fn validate_zstd_frame_header(
    frame: &[u8],
    expected_decoded_bytes: usize,
) -> Result<(), CheckpointError> {
    let mut header = MaybeUninit::<zstd_safe::zstd_sys::ZSTD_FrameHeader>::uninit();
    // SAFETY: `header` points to writable uninitialized storage of exactly the
    // required C type, while `frame` remains alive and immutable for the call.
    // Zstandard retains neither pointer. We call `assume_init` only on the
    // documented zero-success result below.
    let result = unsafe {
        zstd_safe::zstd_sys::ZSTD_getFrameHeader(
            header.as_mut_ptr(),
            frame.as_ptr().cast(),
            frame.len(),
        )
    };
    // SAFETY: `ZSTD_isError` only classifies the returned integer.
    if unsafe { zstd_safe::zstd_sys::ZSTD_isError(result) } != 0 {
        return Err(CheckpointError::format(
            "ZSTD_FRAME",
            "invalid Zstandard frame header",
        ));
    }
    if result != 0 {
        return Err(CheckpointError::format(
            "ZSTD_FRAME",
            "incomplete Zstandard frame header",
        ));
    }
    // SAFETY: a zero `ZSTD_getFrameHeader` result guarantees every field was initialized.
    let header = unsafe { header.assume_init() };
    if header.frameType != zstd_safe::zstd_sys::ZSTD_FrameType_e::ZSTD_frame || header.dictID != 0 {
        return Err(CheckpointError::format(
            "ZSTD_FRAME",
            "SFZ1 requires one ordinary dictionary-free Zstandard frame",
        ));
    }
    if header.frameContentSize != expected_decoded_bytes as u64 {
        return Err(CheckpointError::format(
            "ZSTD_CONTENT_SIZE",
            "Zstandard frame must declare its exact decoded block size",
        ));
    }
    let maximum_window = 1u64.checked_shl(ZSTD_WINDOW_LOG_MAX).ok_or_else(|| {
        CheckpointError::format("COUNT_OVERFLOW", "Zstandard window bound overflows")
    })?;
    if header.windowSize == 0 || header.windowSize > maximum_window {
        return Err(CheckpointError::format(
            "ZSTD_WINDOW",
            format!(
                "Zstandard decoder window {} exceeds bound {maximum_window}",
                header.windowSize
            ),
        ));
    }
    Ok(())
}

/// Validate one numeric role's complete stored structure before final allocations.
fn preflight_numeric_role(
    file: &mut File,
    entry: &ScannedEntry,
    encoding: NumericEncoding,
    expected_floats: usize,
) -> Result<(), CheckpointError> {
    let expected_bytes = usize_to_u64(expected_floats, "numeric float count")?
        .checked_mul(4)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "numeric decoded bytes overflow")
        })?;
    match encoding {
        NumericEncoding::RawF32LeV1 => {
            if entry.size != expected_bytes {
                return Err(CheckpointError::format(
                    "NUMERIC_LENGTH",
                    "raw numeric stored length disagrees with count",
                ));
            }
        }
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
            let mut remaining = entry.size;
            let mut decoded_total = 0usize;
            let mut frame = Vec::new();
            frame.try_reserve_exact(max_frame_bytes).map_err(|_| {
                CheckpointError::format(
                    "ALLOCATION",
                    "unable to reserve reusable bounded Zstandard preflight frame",
                )
            })?;
            file.seek(SeekFrom::Start(entry.data_offset))?;
            while remaining > 0 {
                if remaining < SHUFFLED_BLOCK_HEADER_BYTES as u64 {
                    return Err(CheckpointError::format(
                        "SHUFFLED_HEADER",
                        "truncated SFZ1 block header",
                    ));
                }
                let mut header = [0u8; SHUFFLED_BLOCK_HEADER_BYTES];
                file.read_exact(&mut header)?;
                remaining -= SHUFFLED_BLOCK_HEADER_BYTES as u64;
                if &header[..4] != SHUFFLED_BLOCK_MAGIC {
                    return Err(CheckpointError::format(
                        "SHUFFLED_MAGIC",
                        "unsupported SFZ1 block marker",
                    ));
                }
                let float_count = u32_to_usize(
                    u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
                    "SFZ1 float count",
                )?;
                let frame_bytes = u32_to_usize(
                    u32::from_le_bytes([header[8], header[9], header[10], header[11]]),
                    "SFZ1 frame bytes",
                )?;
                let decoded_bytes = float_count.checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 decoded block bytes overflow")
                })?;
                if float_count == 0
                    || decoded_bytes > SHUFFLED_BLOCK_BYTES
                    || frame_bytes == 0
                    || frame_bytes > max_frame_bytes
                    || frame_bytes as u64 > remaining
                {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 block declaration exceeds bounds",
                    ));
                }
                decoded_total = decoded_total.checked_add(float_count).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 aggregate count overflows")
                })?;
                if decoded_total > expected_floats {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 decoded count exceeds manifest",
                    ));
                }
                frame.clear();
                frame.resize(frame_bytes, 0);
                file.read_exact(&mut frame)?;
                remaining -= frame_bytes as u64;
                validate_zstd_frame_header(&frame, decoded_bytes)?;
                let compressed_size =
                    zstd_safe::find_frame_compressed_size(&frame).map_err(|_| {
                        CheckpointError::format(
                            "ZSTD_FRAME",
                            "unable to determine Zstandard frame boundary",
                        )
                    })?;
                if compressed_size != frame.len() {
                    return Err(CheckpointError::format(
                        "ZSTD_FRAME",
                        "SFZ1 envelope contains trailing frame bytes",
                    ));
                }
            }
            if decoded_total != expected_floats || expected_bytes != (decoded_total as u64) * 4 {
                return Err(CheckpointError::format(
                    "NUMERIC_COUNT",
                    "SFZ1 decoded count is incomplete",
                ));
            }
        }
    }
    Ok(())
}

/// Decode one preflighted raw or shuffled numeric role into final per-owner allocations.
fn decode_numeric_role(
    file: &mut File,
    entry: &ScannedEntry,
    encoding: NumericEncoding,
    expected_floats: usize,
    segment_count: usize,
    segment_length: usize,
    expected_sha256: [u8; 32],
) -> Result<Vec<Box<[f32]>>, CheckpointError> {
    let expected_bytes = usize_to_u64(expected_floats, "numeric float count")?
        .checked_mul(4)
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "numeric decoded bytes overflow")
        })?;
    preflight_numeric_role(file, entry, encoding, expected_floats)?;
    let mut output = SegmentedF32Builder::new(segment_count, segment_length, expected_floats)?;
    let mut hasher = Sha256::new();
    file.seek(SeekFrom::Start(entry.data_offset))?;
    match encoding {
        NumericEncoding::RawF32LeV1 => {
            if entry.size != expected_bytes {
                return Err(CheckpointError::format(
                    "NUMERIC_LENGTH",
                    "raw numeric stored length disagrees with count",
                ));
            }
            let mut remaining = entry.size;
            let mut buffer = vec![
                0u8;
                SHUFFLED_BLOCK_BYTES
                    .min(usize::try_from(remaining).unwrap_or(usize::MAX))
            ];
            while remaining > 0 {
                let take = buffer
                    .len()
                    .min(u64_to_usize(remaining, "raw numeric remaining bytes")?);
                file.read_exact(&mut buffer[..take])?;
                if take % 4 != 0 {
                    return Err(CheckpointError::format(
                        "NUMERIC_LENGTH",
                        "raw numeric chunk is not Float32 aligned",
                    ));
                }
                hasher.update(&buffer[..take]);
                for bytes in buffer[..take].chunks_exact(4) {
                    output
                        .push_bits(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))?;
                }
                remaining -= take as u64;
            }
        }
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let max_frame_bytes = zstd_safe::compress_bound(SHUFFLED_BLOCK_BYTES);
            let mut remaining = entry.size;
            let mut decoded_total = 0usize;
            let mut decompressor = ZstdDecompressor::new()?;
            decompressor.window_log_max(ZSTD_WINDOW_LOG_MAX)?;
            let mut frame = Vec::new();
            frame.try_reserve_exact(max_frame_bytes).map_err(|_| {
                CheckpointError::format(
                    "ALLOCATION",
                    "unable to reserve reusable bounded Zstandard frame",
                )
            })?;
            let mut shuffled = Vec::new();
            shuffled
                .try_reserve_exact(SHUFFLED_BLOCK_BYTES)
                .map_err(|_| {
                    CheckpointError::format(
                        "ALLOCATION",
                        "unable to reserve reusable decoded Zstandard scratch",
                    )
                })?;
            while remaining > 0 {
                if remaining < SHUFFLED_BLOCK_HEADER_BYTES as u64 {
                    return Err(CheckpointError::format(
                        "SHUFFLED_HEADER",
                        "truncated SFZ1 block header",
                    ));
                }
                let mut header = [0u8; SHUFFLED_BLOCK_HEADER_BYTES];
                file.read_exact(&mut header)?;
                remaining -= SHUFFLED_BLOCK_HEADER_BYTES as u64;
                if &header[..4] != SHUFFLED_BLOCK_MAGIC {
                    return Err(CheckpointError::format(
                        "SHUFFLED_MAGIC",
                        "unsupported SFZ1 block marker",
                    ));
                }
                let float_count = u32_to_usize(
                    u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
                    "SFZ1 float count",
                )?;
                let frame_bytes = u32_to_usize(
                    u32::from_le_bytes([header[8], header[9], header[10], header[11]]),
                    "SFZ1 frame bytes",
                )?;
                let decoded_bytes = float_count.checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 decoded block bytes overflow")
                })?;
                if float_count == 0
                    || decoded_bytes > SHUFFLED_BLOCK_BYTES
                    || frame_bytes == 0
                    || frame_bytes > max_frame_bytes
                    || frame_bytes as u64 > remaining
                {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 block declaration exceeds bounds",
                    ));
                }
                decoded_total = decoded_total.checked_add(float_count).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "SFZ1 aggregate count overflows")
                })?;
                if decoded_total > expected_floats {
                    return Err(CheckpointError::format(
                        "SHUFFLED_LIMIT",
                        "SFZ1 decoded count exceeds manifest",
                    ));
                }
                frame.clear();
                frame.resize(frame_bytes, 0);
                file.read_exact(&mut frame)?;
                remaining -= frame_bytes as u64;
                validate_zstd_frame_header(&frame, decoded_bytes)?;
                let compressed_size =
                    zstd_safe::find_frame_compressed_size(&frame).map_err(|_| {
                        CheckpointError::format(
                            "ZSTD_FRAME",
                            "unable to determine Zstandard frame boundary",
                        )
                    })?;
                if compressed_size != frame.len() {
                    return Err(CheckpointError::format(
                        "ZSTD_FRAME",
                        "SFZ1 envelope contains trailing frame bytes",
                    ));
                }
                shuffled.clear();
                let written = decompressor.decompress_to_buffer(&frame, &mut shuffled)?;
                if written != decoded_bytes || shuffled.len() != decoded_bytes {
                    return Err(CheckpointError::format(
                        "ZSTD_DECODED_LENGTH",
                        "decoded Zstandard block length mismatch",
                    ));
                }
                for value_index in 0..float_count {
                    let bytes = [
                        shuffled[value_index],
                        shuffled[float_count + value_index],
                        shuffled[2 * float_count + value_index],
                        shuffled[3 * float_count + value_index],
                    ];
                    hasher.update(bytes);
                    output.push_bits(u32::from_le_bytes(bytes))?;
                }
            }
            if decoded_total != expected_floats || expected_bytes != (decoded_total as u64) * 4 {
                return Err(CheckpointError::format(
                    "NUMERIC_COUNT",
                    "SFZ1 decoded count is incomplete",
                ));
            }
        }
    }
    let actual_sha256: [u8; 32] = hasher.finalize().into();
    if actual_sha256 != expected_sha256 {
        return Err(CheckpointError::format(
            "LOGICAL_ROLE_SHA256",
            "numeric logical SHA-256 mismatch",
        ));
    }
    output.finish()
}

/// Reconstruct the exact boundary candidate without a transient full flattened copy.
fn assemble_candidate(
    state: DecodedStateParts,
    index: &DecodedPopulationIndex,
    weights: Vec<Box<[f32]>>,
    recurrent: Vec<Box<[f32]>>,
) -> Result<StateCandidate, CheckpointError> {
    if weights.len() != index.records.len() || recurrent.len() != index.records.len() {
        return Err(CheckpointError::format(
            "NUMERIC_SEGMENTS",
            "decoded numeric segment counts disagree with index",
        ));
    }
    let mut weights: Vec<Option<Box<[f32]>>> = weights.into_iter().map(Some).collect();
    let mut recurrent: Vec<Option<Box<[f32]>>> = recurrent.into_iter().map(Some).collect();
    let mut population = Vec::new();
    population
        .try_reserve_exact(index.records.len())
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve admitted population")
        })?;
    let mut brains: Vec<Option<BrainRuntimeState>> =
        (0..index.records.len()).map(|_| None).collect();
    for record in &index.records {
        let slot = record.slot as usize;
        let brain_index = record.brain_index as usize;
        let genome_weights = weights
            .get_mut(slot)
            .and_then(Option::take)
            .ok_or_else(|| {
                CheckpointError::format("INDEX_RANGE", "weight segment reused or missing")
            })?;
        let brain_recurrent = recurrent
            .get_mut(brain_index)
            .and_then(Option::take)
            .ok_or_else(|| {
                CheckpointError::format("INDEX_RANGE", "recurrent segment reused or missing")
            })?;
        let expected_weight_offset = slot.checked_mul(record.weight_count).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "population weight offset overflows")
        })?;
        let expected_recurrent_offset = brain_index
            .checked_mul(record.recurrent_count)
            .ok_or_else(|| {
                CheckpointError::format("COUNT_OVERFLOW", "population recurrent offset overflows")
            })?;
        if genome_weights.len() != record.weight_count
            || brain_recurrent.len() != record.recurrent_count
            || record.weight_offset != expected_weight_offset
            || record.recurrent_offset != expected_recurrent_offset
        {
            return Err(CheckpointError::format(
                "INDEX_RANGE",
                "decoded segment disagrees with population range",
            ));
        }
        population.push(PopulationGenome {
            slot: record.slot,
            brain: record.brain,
            lineage: record.lineage.clone(),
            fitness: record.fitness,
            weights: genome_weights,
        });
        brains[brain_index] = Some(BrainRuntimeState {
            handle: record.brain,
            owner: BrainOwner::PopulationSlot(record.slot),
            non_population_weights: None,
            recurrent: brain_recurrent,
        });
    }
    let brains = brains
        .into_iter()
        .map(|brain| {
            brain.ok_or_else(|| CheckpointError::format("INDEX_DENSE", "brain index is missing"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StateCandidate {
        versions: state.versions,
        identity: state.identity,
        config: state.config,
        phase: AuthorityPhase::GenerationBoundary(state.boundary_kind),
        generation: state.generation,
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: state.rng,
        allocators: state.allocators,
        population,
        brains,
        world: WorldState::default(),
    })
}

/// Assemble decoded metadata with empty numeric boxes for shared pre-allocation admission.
fn assemble_allocation_shell(
    state: &DecodedStateParts,
    index: &DecodedPopulationIndex,
) -> Result<StateCandidate, CheckpointError> {
    let mut population = Vec::new();
    population
        .try_reserve_exact(index.records.len())
        .map_err(|_| {
            CheckpointError::format(
                "ALLOCATION",
                "unable to reserve allocation-shell population",
            )
        })?;
    let mut brains: Vec<Option<BrainRuntimeState>> =
        (0..index.records.len()).map(|_| None).collect();
    for record in &index.records {
        population.push(PopulationGenome {
            slot: record.slot,
            brain: record.brain,
            lineage: record.lineage.clone(),
            fitness: record.fitness,
            weights: Box::new([]),
        });
        let brain_index = record.brain_index as usize;
        brains[brain_index] = Some(BrainRuntimeState {
            handle: record.brain,
            owner: BrainOwner::PopulationSlot(record.slot),
            non_population_weights: None,
            recurrent: Box::new([]),
        });
    }
    let brains = brains
        .into_iter()
        .map(|brain| {
            brain.ok_or_else(|| {
                CheckpointError::format("INDEX_DENSE", "allocation-shell brain index is missing")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StateCandidate {
        versions: state.versions.clone(),
        identity: state.identity.clone(),
        config: state.config.clone(),
        phase: AuthorityPhase::GenerationBoundary(state.boundary_kind),
        generation: state.generation.clone(),
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: state.rng.clone(),
        allocators: state.allocators.clone(),
        population,
        brains,
        world: WorldState::default(),
    })
}

/// Select the fixed presentation path for one numeric logical role.
fn numeric_path(weights: bool, encoding: NumericEncoding) -> &'static str {
    match (weights, encoding) {
        (true, NumericEncoding::RawF32LeV1) => WEIGHTS_RAW_PATH,
        (true, NumericEncoding::F32LeShuffle4ZstdV1) => WEIGHTS_ZSTD_PATH,
        (false, NumericEncoding::RawF32LeV1) => RECURRENT_RAW_PATH,
        (false, NumericEncoding::F32LeShuffle4ZstdV1) => RECURRENT_ZSTD_PATH,
    }
}

/// Construct a manifest declaration for one raw binary role.
fn manifest_role(role: &SmallRole, digest: &LogicalRoleDigest, decoded_count: u64) -> ManifestRole {
    ManifestRole {
        role: role.role.to_owned(),
        path: role.path.to_owned(),
        encoding: "raw-binary-v1".to_owned(),
        stored_bytes_hex: encode_u64_hex(role.bytes.len() as u64),
        decoded_bytes_hex: encode_u64_hex(digest.logical_length),
        decoded_count_hex: encode_u64_hex(decoded_count),
        record_size: role.record_size,
        logical_sha256: encode_digest(digest.logical_sha256),
    }
}

/// Construct a manifest declaration for one adaptive numeric role.
fn manifest_numeric_role(
    role: &'static str,
    path: &'static str,
    candidate: &NumericCandidate,
    digest: &LogicalRoleDigest,
) -> ManifestRole {
    ManifestRole {
        role: role.to_owned(),
        path: path.to_owned(),
        encoding: candidate.encoding.as_str().to_owned(),
        stored_bytes_hex: encode_u64_hex(candidate.stored_bytes),
        decoded_bytes_hex: encode_u64_hex(candidate.raw_bytes),
        decoded_count_hex: encode_u64_hex(candidate.float_count as u64),
        record_size: 4,
        logical_sha256: encode_digest(digest.logical_sha256),
    }
}

/// Append one fixed-path regular USTAR entry.
fn append_ustar<W: Write, R: Read>(
    archive: &mut TarBuilder<W>,
    path: &'static str,
    size: u64,
    reader: R,
) -> Result<(), CheckpointError> {
    let mut header = Header::new_ustar();
    header.set_entry_type(EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(size);
    header.set_cksum();
    archive.append_data(&mut header, path, reader)?;
    Ok(())
}

/// Append one selected numeric role without materializing a second flat buffer.
fn append_numeric_ustar<W: Write>(
    archive: &mut TarBuilder<W>,
    path: &'static str,
    candidate: &NumericCandidate,
    source: &FloatSource<'_>,
) -> Result<(), CheckpointError> {
    if candidate.scratch_capacity_growths != 0
        || (candidate.float_count != 0 && candidate.encoded_blocks == 0)
    {
        return Err(CheckpointError::format(
            "NUMERIC_CANDIDATE",
            "numeric candidate did not preserve reusable bounded scratch invariants",
        ));
    }
    match candidate.encoding {
        NumericEncoding::RawF32LeV1 => append_ustar(
            archive,
            path,
            candidate.stored_bytes,
            FloatByteReader::new(source),
        ),
        NumericEncoding::F32LeShuffle4ZstdV1 => {
            let compressed_path = candidate.compressed_path.as_ref().ok_or_else(|| {
                CheckpointError::format("NUMERIC_CANDIDATE", "compressed candidate path is missing")
            })?;
            append_ustar(
                archive,
                path,
                candidate.stored_bytes,
                BufReader::new(File::open(compressed_path)?),
            )
        }
    }
}

/// Compute the exact completed USTAR length for fixed entry sizes.
fn expected_archive_length<const N: usize>(sizes: [u64; N]) -> Result<u64, CheckpointError> {
    let mut total = USTAR_TRAILER_BYTES;
    for size in sizes {
        let padding = (USTAR_BLOCK_BYTES - (size % USTAR_BLOCK_BYTES)) % USTAR_BLOCK_BYTES;
        total = total
            .checked_add(USTAR_BLOCK_BYTES)
            .and_then(|value| value.checked_add(size))
            .and_then(|value| value.checked_add(padding))
            .ok_or_else(|| {
                CheckpointError::format("ARCHIVE_OVERFLOW", "USTAR length overflows u64")
            })?;
    }
    Ok(total)
}

/// Atomically publish without replacing an existing content-addressed file on Linux.
#[cfg(target_os = "linux")]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};

    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

/// Windows `MoveFileEx` semantics used by `std::fs::rename` do not replace an existing file.
#[cfg(target_os = "windows")]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

/// Unsupported development targets use an atomic exclusive hard-link publication.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(source, destination)?;
    fs::remove_file(source)
}

/// Flush the containing directory on Unix after the same-directory rename.
#[cfg(unix)]
fn sync_parent_directory(directory: &Path) -> Result<(), CheckpointError> {
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// Windows has no portable directory handle `sync_all`; file fsync and rename still apply.
#[cfg(not(unix))]
fn sync_parent_directory(_directory: &Path) -> Result<(), CheckpointError> {
    Ok(())
}

/// One strictly scanned ordinary regular-file entry.
#[derive(Clone, Debug)]
struct ScannedEntry {
    name: String,
    #[cfg(test)]
    header_offset: u64,
    data_offset: u64,
    size: u64,
}

/// Scan the complete archive structure before allocating or decoding any role.
fn scan_strict_ustar(
    file: &mut File,
    archive_length: u64,
    limits: &CheckpointLimits,
) -> Result<Vec<ScannedEntry>, CheckpointError> {
    file.seek(SeekFrom::Start(0))?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(USTAR_ENTRY_COUNT).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve fixed USTAR entry table")
    })?;
    let mut offset = 0u64;
    loop {
        let header_end = offset.checked_add(USTAR_BLOCK_BYTES).ok_or_else(|| {
            CheckpointError::format("USTAR_OVERFLOW", "USTAR header offset overflows")
        })?;
        if header_end > archive_length {
            return Err(CheckpointError::format(
                "USTAR_TRUNCATED_HEADER",
                "archive ends inside USTAR header",
            ));
        }
        let mut header = [0u8; USTAR_BLOCK_BYTES as usize];
        file.read_exact(&mut header)?;
        if header.iter().all(|byte| *byte == 0) {
            let trailer_end = offset.checked_add(USTAR_TRAILER_BYTES).ok_or_else(|| {
                CheckpointError::format("USTAR_OVERFLOW", "USTAR trailer offset overflows")
            })?;
            if trailer_end != archive_length {
                return Err(CheckpointError::format(
                    "USTAR_TRAILER",
                    "archive must end with exactly two zero blocks",
                ));
            }
            let mut second = [0u8; USTAR_BLOCK_BYTES as usize];
            file.read_exact(&mut second)?;
            if second.iter().any(|byte| *byte != 0) {
                return Err(CheckpointError::format(
                    "USTAR_TRAILER",
                    "second USTAR trailer block is nonzero",
                ));
            }
            break;
        }
        if entries.len() >= USTAR_ENTRY_COUNT {
            return Err(CheckpointError::format(
                "USTAR_ENTRY_COUNT",
                "archive contains too many entries",
            ));
        }
        let (name, size) = parse_ustar_header(&header)?;
        if entries
            .iter()
            .any(|entry: &ScannedEntry| entry.name == name)
        {
            return Err(CheckpointError::format(
                "USTAR_DUPLICATE",
                format!("duplicate USTAR entry {name}"),
            ));
        }
        validate_scanned_entry_size(&name, size, limits)?;
        let data_offset = header_end;
        let padding = (USTAR_BLOCK_BYTES - (size % USTAR_BLOCK_BYTES)) % USTAR_BLOCK_BYTES;
        let next = data_offset
            .checked_add(size)
            .and_then(|value| value.checked_add(padding))
            .ok_or_else(|| {
                CheckpointError::format("USTAR_OVERFLOW", "USTAR entry offset overflows")
            })?;
        if next > archive_length {
            return Err(CheckpointError::format(
                "USTAR_TRUNCATED_ENTRY",
                format!("entry {name} exceeds archive"),
            ));
        }
        file.seek(SeekFrom::Start(data_offset + size))?;
        if padding > 0 {
            let mut pad = [0u8; USTAR_BLOCK_BYTES as usize];
            file.read_exact(&mut pad[..padding as usize])?;
            if pad[..padding as usize].iter().any(|byte| *byte != 0) {
                return Err(CheckpointError::format(
                    "USTAR_PADDING",
                    format!("entry {name} has nonzero padding"),
                ));
            }
        }
        entries.push(ScannedEntry {
            name,
            #[cfg(test)]
            header_offset: offset,
            data_offset,
            size,
        });
        offset = next;
        file.seek(SeekFrom::Start(offset))?;
    }
    if entries.len() != USTAR_ENTRY_COUNT
        || entries
            .last()
            .is_none_or(|entry| entry.name != MANIFEST_PATH)
    {
        return Err(CheckpointError::format(
            "USTAR_ENTRY_SET",
            "archive must contain exactly five logical roles followed by manifest.json",
        ));
    }
    Ok(entries)
}

/// Parse the retained strict USTAR v1 header subset and verify its standard checksum.
fn parse_ustar_header(header: &[u8; 512]) -> Result<(String, u64), CheckpointError> {
    let expected_checksum = parse_ustar_octal(&header[148..156], "checksum")?;
    let actual_checksum = header.iter().enumerate().fold(0u64, |sum, (index, byte)| {
        sum + if (148..156).contains(&index) {
            0x20
        } else {
            u64::from(*byte)
        }
    });
    if expected_checksum != actual_checksum {
        return Err(CheckpointError::format(
            "USTAR_CHECKSUM",
            "USTAR header checksum mismatch",
        ));
    }
    if &header[257..263] != b"ustar\0" || &header[263..265] != b"00" {
        return Err(CheckpointError::format(
            "USTAR_VERSION",
            "missing USTAR magic/version",
        ));
    }
    if header[156] != b'0' {
        return Err(CheckpointError::format(
            "USTAR_ENTRY_TYPE",
            "only explicit regular-file entries are supported",
        ));
    }
    if header[157..257].iter().any(|byte| *byte != 0)
        || header[329..345].iter().any(|byte| *byte != 0)
        || header[345..500].iter().any(|byte| *byte != 0)
        || header[500..512].iter().any(|byte| *byte != 0)
    {
        return Err(CheckpointError::format(
            "USTAR_UNSUPPORTED",
            "links, devices, prefixes, and extension/reserved fields are unsupported",
        ));
    }
    let name_end = header[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    if header[name_end..100].iter().any(|byte| *byte != 0) {
        return Err(CheckpointError::format(
            "USTAR_PATH",
            "USTAR name has hidden bytes after NUL",
        ));
    }
    let name_bytes = &header[..name_end];
    if name_bytes.is_empty() || name_bytes.iter().any(|byte| !(0x20..=0x7e).contains(byte)) {
        return Err(CheckpointError::format(
            "USTAR_PATH",
            "USTAR entry path is not printable ASCII",
        ));
    }
    let name = std::str::from_utf8(name_bytes)
        .map_err(|_| CheckpointError::format("USTAR_PATH", "USTAR entry path is not UTF-8"))?
        .to_owned();
    validate_fixed_safe_path(&name)?;
    let size = parse_ustar_octal(&header[124..136], "size")?;
    Ok((name, size))
}

/// Parse one strict trailing-NUL/space USTAR octal field.
fn parse_ustar_octal(field: &[u8], label: &'static str) -> Result<u64, CheckpointError> {
    let end = field
        .iter()
        .rposition(|byte| !matches!(byte, 0 | b' '))
        .map_or(0, |index| index + 1);
    let digits = &field[..end];
    if digits.is_empty() || digits.iter().any(|byte| !(b'0'..=b'7').contains(byte)) {
        return Err(CheckpointError::format(
            "USTAR_OCTAL",
            format!("invalid {label} octal field"),
        ));
    }
    let mut value = 0u64;
    for digit in digits {
        value = value
            .checked_mul(8)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or_else(|| {
                CheckpointError::format("USTAR_OCTAL", format!("{label} octal field overflows"))
            })?;
    }
    Ok(value)
}

/// Accept only the fixed role paths and reject all generic path semantics.
fn validate_fixed_safe_path(path: &str) -> Result<(), CheckpointError> {
    let allowed = [
        STATE_PATH,
        GRAPH_PATH,
        POPULATION_INDEX_PATH,
        WEIGHTS_RAW_PATH,
        WEIGHTS_ZSTD_PATH,
        RECURRENT_RAW_PATH,
        RECURRENT_ZSTD_PATH,
        MANIFEST_PATH,
    ];
    if !allowed.contains(&path)
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(CheckpointError::format(
            "USTAR_PATH",
            format!("unsupported or unsafe fixed role path {path:?}"),
        ));
    }
    Ok(())
}

/// Apply path-specific stored-byte limits during the structural scan.
fn validate_scanned_entry_size(
    path: &str,
    size: u64,
    limits: &CheckpointLimits,
) -> Result<(), CheckpointError> {
    let limit = match path {
        STATE_PATH => limits.max_state_bytes as u64,
        GRAPH_PATH => limits.max_graph_bytes as u64,
        POPULATION_INDEX_PATH => limits.max_population_index_bytes as u64,
        WEIGHTS_RAW_PATH | WEIGHTS_ZSTD_PATH | RECURRENT_RAW_PATH | RECURRENT_ZSTD_PATH => {
            limits.max_numeric_stored_bytes
        }
        MANIFEST_PATH => limits.max_manifest_bytes as u64,
        _ => {
            return Err(CheckpointError::format(
                "USTAR_PATH",
                "unknown fixed role path",
            ))
        }
    };
    if size > limit {
        return Err(CheckpointError::format(
            "ENTRY_LIMIT",
            format!("entry {path} bytes {size} exceed limit {limit}"),
        ));
    }
    Ok(())
}

/// Read one already-size-bounded entry into memory.
fn read_entry_bytes(
    file: &mut File,
    entry: &ScannedEntry,
    limit: usize,
) -> Result<Vec<u8>, CheckpointError> {
    let size = u64_to_usize(entry.size, "entry byte length")?;
    if size > limit {
        return Err(CheckpointError::format(
            "ENTRY_LIMIT",
            "entry exceeds role byte limit",
        ));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve bounded entry bytes")
    })?;
    bytes.resize(size, 0);
    file.seek(SeekFrom::Start(entry.data_offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Validate every manifest field and role tuple before reading role bodies.
fn validate_manifest(
    manifest: &CheckpointManifest,
    entries: &[ScannedEntry],
    limits: &CheckpointLimits,
) -> Result<ValidatedManifest, CheckpointError> {
    if manifest.magic != "slither-neuroevo-checkpoint"
        || manifest.container_version != CHECKPOINT_CONTAINER_VERSION
        || manifest.archive_kind != "managed-checkpoint-v3"
        || manifest.checkpoint_format_version != CHECKPOINT_VERSION
        || manifest.graph_layout_version != CANONICAL_GRAPH_LAYOUT_VERSION
    {
        return Err(CheckpointError::format(
            "MANIFEST_VERSION",
            "unsupported checkpoint manifest identity/version",
        ));
    }
    parse_u64_hex(&manifest.generation_hex, "generation")?;
    parse_u64_hex(&manifest.completed_step_hex, "completed step")?;
    CheckpointBoundaryKind::parse(&manifest.boundary_kind)?;
    CheckpointWriteValidationPolicy::parse(&manifest.write_validation_policy)?;
    if manifest.run_id.is_empty()
        || manifest.run_id.contains('\0')
        || manifest.run_id.len() > 256
        || manifest.run_id.len() > limits.max_string_bytes
    {
        return Err(CheckpointError::format(
            "MANIFEST_RUN_ID",
            "run ID is empty or exceeds string limit",
        ));
    }
    if manifest.graph_architecture_key.is_empty()
        || manifest.graph_architecture_key.len() > limits.max_total_string_bytes
    {
        return Err(CheckpointError::format(
            "MANIFEST_GRAPH_KEY",
            "graph architecture key is empty or exceeds text limit",
        ));
    }
    parse_digest(&manifest.graph_layout_sha256, "graph layout SHA-256")?;
    let weights_encoding = NumericEncoding::parse(&manifest.weights_encoding)?;
    let recurrent_encoding = NumericEncoding::parse(&manifest.recurrent_encoding)?;
    let population_count = u64_to_usize(
        parse_u64_hex(&manifest.population_count_hex, "population count")?,
        "population count",
    )?;
    let weight_float_count = u64_to_usize(
        parse_u64_hex(&manifest.weight_float_count_hex, "weight count")?,
        "weight count",
    )?;
    let recurrent_float_count = u64_to_usize(
        parse_u64_hex(&manifest.recurrent_float_count_hex, "recurrent count")?,
        "recurrent count",
    )?;
    if population_count > limits.max_population_count
        || weight_float_count > limits.max_weight_floats
        || recurrent_float_count > limits.max_recurrent_floats
    {
        return Err(CheckpointError::format(
            "MANIFEST_LIMIT",
            "manifest counts exceed checkpoint limits",
        ));
    }
    if manifest.roles.len() != LOGICAL_ROLE_COUNT || entries.len() != USTAR_ENTRY_COUNT {
        return Err(CheckpointError::format(
            "MANIFEST_ROLES",
            "manifest has an invalid role count",
        ));
    }
    let expected_roles = [
        ("checkpoint", STATE_PATH, "raw-binary-v1", 0u32, 1u64),
        ("graph", GRAPH_PATH, "raw-binary-v1", 0u32, 1u64),
        (
            "population-index",
            POPULATION_INDEX_PATH,
            "raw-binary-v1",
            POPULATION_INDEX_RECORD_BYTES,
            population_count as u64,
        ),
        (
            "population-weights",
            numeric_path(true, weights_encoding),
            weights_encoding.as_str(),
            4u32,
            weight_float_count as u64,
        ),
        (
            "population-recurrent",
            numeric_path(false, recurrent_encoding),
            recurrent_encoding.as_str(),
            4u32,
            recurrent_float_count as u64,
        ),
    ];
    let mut validated_roles = Vec::new();
    validated_roles
        .try_reserve_exact(LOGICAL_ROLE_COUNT)
        .map_err(|_| {
            CheckpointError::format("ALLOCATION", "unable to reserve validated role tuples")
        })?;
    let mut stored_total = 0u64;
    let mut decoded_total = 0u64;
    for (index, role) in manifest.roles.iter().enumerate() {
        let expected = expected_roles[index];
        if role.role != expected.0
            || role.path != expected.1
            || role.encoding != expected.2
            || role.record_size != expected.3
            || entries[index].name != role.path
        {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                format!("role {index} identity/path/encoding/record size mismatch"),
            ));
        }
        let stored = parse_u64_hex(&role.stored_bytes_hex, "role stored bytes")?;
        let decoded = parse_u64_hex(&role.decoded_bytes_hex, "role decoded bytes")?;
        let decoded_count = parse_u64_hex(&role.decoded_count_hex, "role decoded count")?;
        if stored != entries[index].size || decoded_count != expected.4 {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "role stored length or logical count mismatch",
            ));
        }
        if index < 3 && stored != decoded {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "raw binary role stored/decoded lengths differ",
            ));
        }
        if index == 3
            && decoded
                != (weight_float_count as u64).checked_mul(4).ok_or_else(|| {
                    CheckpointError::format("COUNT_OVERFLOW", "weight decoded bytes overflow")
                })?
        {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "weight decoded length disagrees with count",
            ));
        }
        if index == 4
            && decoded
                != (recurrent_float_count as u64)
                    .checked_mul(4)
                    .ok_or_else(|| {
                        CheckpointError::format(
                            "COUNT_OVERFLOW",
                            "recurrent decoded bytes overflow",
                        )
                    })?
        {
            return Err(CheckpointError::format(
                "MANIFEST_ROLE",
                "recurrent decoded length disagrees with count",
            ));
        }
        let logical_sha256 = parse_digest(&role.logical_sha256, "role logical SHA-256")?;
        stored_total = stored_total.checked_add(stored).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "role stored total overflows")
        })?;
        decoded_total = decoded_total.checked_add(decoded).ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "role decoded total overflows")
        })?;
        validated_roles.push(ValidatedRole {
            logical_length: decoded,
            logical_sha256,
        });
    }
    if stored_total != parse_u64_hex(&manifest.role_stored_bytes_hex, "role stored total")?
        || decoded_total != parse_u64_hex(&manifest.role_decoded_bytes_hex, "role decoded total")?
        || decoded_total > limits.max_total_decoded_bytes
    {
        return Err(CheckpointError::format(
            "MANIFEST_TOTAL",
            "manifest role byte totals disagree or exceed limit",
        ));
    }
    let root_roles: Vec<LogicalRoleDigest> = expected_roles
        .iter()
        .zip(&validated_roles)
        .map(|(expected, role)| LogicalRoleDigest {
            role: expected.0,
            logical_length: role.logical_length,
            logical_sha256: role.logical_sha256,
        })
        .collect();
    let computed_root = compute_logical_root(&root_roles)?;
    if parse_digest(&manifest.logical_root_sha256, "logical root SHA-256")? != computed_root {
        return Err(CheckpointError::format(
            "LOGICAL_ROOT",
            "manifest logical root does not match ordered role tuples",
        ));
    }
    Ok(ValidatedManifest {
        roles: validated_roles,
        population_count,
        weight_float_count,
        recurrent_float_count,
        weights_encoding,
        recurrent_encoding,
    })
}

/// Cross-check every manifest fact mirrored from authoritative decoded content.
fn crosscheck_manifest_state(
    manifest: &CheckpointManifest,
    validated: &ValidatedManifest,
    state: &AuthoritativeState,
) -> Result<(), CheckpointError> {
    let candidate = state.state();
    let boundary = match candidate.phase {
        AuthorityPhase::GenerationBoundary(kind) => CheckpointBoundaryKind::from_state(kind),
        AuthorityPhase::Running => {
            return Err(CheckpointError::format(
                "MANIFEST_STATE_MISMATCH",
                "restored checkpoint unexpectedly contains running state",
            ));
        }
    };
    let actual_weight_count = candidate
        .population
        .iter()
        .try_fold(0usize, |total, genome| {
            total.checked_add(genome.weights.len())
        })
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "admitted weight count overflows")
        })?;
    let actual_recurrent_count = candidate
        .brains
        .iter()
        .try_fold(0usize, |total, brain| {
            total.checked_add(brain.recurrent.len())
        })
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "admitted recurrent count overflows")
        })?;
    if manifest.checkpoint_format_version != candidate.versions.checkpoint
        || manifest.state_version != candidate.versions.state
        || manifest.graph_layout_version != candidate.versions.graph_layout
        || manifest.run_id != candidate.identity.run_id
        || parse_u64_hex(&manifest.generation_hex, "generation")? != candidate.generation.generation
        || parse_u64_hex(&manifest.completed_step_hex, "completed step")?
            != candidate.generation.completed_step
        || CheckpointBoundaryKind::parse(&manifest.boundary_kind)? != boundary
        || validated.population_count != candidate.population.len()
        || validated.population_count != candidate.config.population_count
        || validated.weight_float_count != actual_weight_count
        || validated.recurrent_float_count != actual_recurrent_count
        || manifest.graph_architecture_key != candidate.config.graph_architecture_key
        || manifest.graph_architecture_key != state.graph().architecture_key
        || manifest.graph_layout_sha256 != state.graph().layout_digest_hex()
    {
        return Err(CheckpointError::format(
            "MANIFEST_STATE_MISMATCH",
            "manifest mirrors do not match admitted authoritative state/graph/counts",
        ));
    }
    Ok(())
}

/// Read and verify a raw small binary role using its one logical digest.
fn read_and_verify_small_role(
    file: &mut File,
    entry: &ScannedEntry,
    role: &ValidatedRole,
    limit: usize,
) -> Result<Vec<u8>, CheckpointError> {
    let bytes = read_entry_bytes(file, entry, limit)?;
    if role.logical_length != bytes.len() as u64 || sha256(&bytes) != role.logical_sha256 {
        return Err(CheckpointError::format(
            "LOGICAL_ROLE_SHA256",
            format!("logical role {} failed length/SHA-256", entry.name),
        ));
    }
    Ok(bytes)
}

/// Map a validated private manifest to the bounded Node-facing descriptor contract.
fn content_descriptor_from_manifest(
    manifest: &CheckpointManifest,
    archive_length: u64,
    actual_filename: String,
) -> Result<CheckpointContentDescriptor, CheckpointError> {
    let expected_filename = format!("{}.checkpoint-v3", manifest.logical_root_sha256);
    if actual_filename != expected_filename {
        return Err(CheckpointError::format(
            "MANAGED_FILENAME",
            format!("managed filename {actual_filename:?} does not match digest-derived {expected_filename:?}"),
        ));
    }
    Ok(CheckpointContentDescriptor {
        run_id: manifest.run_id.clone(),
        generation_hex: canonical_u64_hex(&manifest.generation_hex, "generation")?,
        completed_step_hex: canonical_u64_hex(&manifest.completed_step_hex, "completed step")?,
        boundary_kind: CheckpointBoundaryKind::parse(&manifest.boundary_kind)?,
        checkpoint_format_version_hex: encode_u64_hex(u64::from(
            manifest.checkpoint_format_version,
        )),
        state_version_hex: encode_u64_hex(u64::from(manifest.state_version)),
        graph_layout_version_hex: encode_u64_hex(u64::from(manifest.graph_layout_version)),
        managed_root: "checkpoint-v3".to_owned(),
        logical_root_sha256: encode_digest(parse_digest(
            &manifest.logical_root_sha256,
            "logical root",
        )?),
        relative_filename: expected_filename,
        stored_byte_count_hex: encode_u64_hex(archive_length),
        decoded_byte_count_hex: canonical_u64_hex(
            &manifest.role_decoded_bytes_hex,
            "decoded role bytes",
        )?,
        population_count_hex: canonical_u64_hex(
            &manifest.population_count_hex,
            "population count",
        )?,
        role_count_hex: encode_u64_hex(LOGICAL_ROLE_COUNT as u64),
        weight_count_hex: canonical_u64_hex(&manifest.weight_float_count_hex, "weight count")?,
        recurrent_state_count_hex: canonical_u64_hex(
            &manifest.recurrent_float_count_hex,
            "recurrent count",
        )?,
        weights_encoding: NumericEncoding::parse(&manifest.weights_encoding)?,
        recurrent_state_encoding: NumericEncoding::parse(&manifest.recurrent_encoding)?,
        graph_layout_sha256: encode_digest(parse_digest(
            &manifest.graph_layout_sha256,
            "graph layout",
        )?),
        write_validation_policy: CheckpointWriteValidationPolicy::parse(
            &manifest.write_validation_policy,
        )?,
    })
}

/// Combine immutable content facts with the current publication correlation values.
fn publication_descriptor(
    content: CheckpointContentDescriptor,
    operation_id: CheckpointOperationId,
    transition_epoch: u64,
) -> CheckpointDescriptor {
    CheckpointDescriptor {
        protocol_version: CHECKPOINT_DESCRIPTOR_VERSION,
        managed_root: content.managed_root,
        operation_id,
        transition_epoch_hex: encode_u64_hex(transition_epoch),
        run_id: content.run_id,
        generation_hex: content.generation_hex,
        completed_step_hex: content.completed_step_hex,
        boundary_kind: content.boundary_kind,
        checkpoint_format_version_hex: content.checkpoint_format_version_hex,
        state_version_hex: content.state_version_hex,
        graph_layout_version_hex: content.graph_layout_version_hex,
        logical_root_sha256: content.logical_root_sha256,
        relative_filename: content.relative_filename,
        stored_byte_count_hex: content.stored_byte_count_hex,
        decoded_byte_count_hex: content.decoded_byte_count_hex,
        population_count_hex: content.population_count_hex,
        role_count_hex: content.role_count_hex,
        weight_count_hex: content.weight_count_hex,
        recurrent_state_count_hex: content.recurrent_state_count_hex,
        weights_encoding: content.weights_encoding,
        recurrent_state_encoding: content.recurrent_state_encoding,
        graph_layout_sha256: content.graph_layout_sha256,
        write_validation_policy: content.write_validation_policy,
    }
}

/// Accept an existing digest-derived file only after a complete strict restore and state comparison.
#[allow(clippy::too_many_arguments)]
fn validate_existing_idempotent(
    final_path: &Path,
    operation_id: &CheckpointOperationId,
    transition_epoch: u64,
    expected_state: &StateCandidate,
    expected_graph: &GraphSpec,
    expected_root: &str,
    limits: &CheckpointLimits,
    graph_limits: &GraphLimits,
    admission_policy: &StateAdmissionPolicy,
) -> Result<CheckpointDescriptor, CheckpointError> {
    let restored = restore_checkpoint(final_path, limits, graph_limits, admission_policy).map_err(
        |error| {
            CheckpointError::format(
                "DIGEST_COLLISION",
                format!("existing digest-derived file failed strict validation: {error}"),
            )
        },
    )?;
    if restored.content.logical_root_sha256 != expected_root
        || restored.state.state() != expected_state
        || restored.state.graph_spec() != expected_graph
    {
        return Err(CheckpointError::format(
            "DIGEST_COLLISION",
            "existing digest-derived file is not the same admitted logical checkpoint",
        ));
    }
    Ok(publication_descriptor(
        restored.content,
        operation_id.clone(),
        transition_epoch,
    ))
}

/// Compute the Stage-2-compatible ordered logical root.
fn compute_logical_root(roles: &[LogicalRoleDigest]) -> Result<[u8; 32], CheckpointError> {
    let mut hasher = Sha256::new();
    hasher.update(LOGICAL_ROOT_DOMAIN);
    hasher.update(usize_to_u32(roles.len(), "logical role count")?.to_le_bytes());
    let mut seen = Vec::new();
    seen.try_reserve_exact(roles.len()).map_err(|_| {
        CheckpointError::format("ALLOCATION", "unable to reserve logical role-name set")
    })?;
    for role in roles {
        if role.role.is_empty() || role.role.len() > u16::MAX as usize || seen.contains(&role.role)
        {
            return Err(CheckpointError::format(
                "LOGICAL_ROLE",
                "logical role name is empty, duplicate, or too long",
            ));
        }
        seen.push(role.role);
        hasher.update((role.role.len() as u16).to_le_bytes());
        hasher.update(role.role.as_bytes());
        hasher.update(role.logical_length.to_le_bytes());
        hasher.update(role.logical_sha256);
    }
    Ok(hasher.finalize().into())
}

/// Hash one already-bounded logical role.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Encode exactly 32 bytes as lowercase hexadecimal.
fn encode_digest(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(64);
    for byte in digest {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

/// Parse an exact lowercase SHA-256 string.
fn parse_digest(value: &str, label: &'static str) -> Result<[u8; 32], CheckpointError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "SHA256",
            format!("{label} must be 64 lowercase hexadecimal digits"),
        ));
    }
    let mut digest = [0u8; 32];
    for (index, target) in digest.iter_mut().enumerate() {
        let high = hex_nibble(value.as_bytes()[index * 2])?;
        let low = hex_nibble(value.as_bytes()[index * 2 + 1])?;
        *target = (high << 4) | low;
    }
    Ok(digest)
}

/// Decode one lowercase hexadecimal nibble after shape validation.
fn hex_nibble(byte: u8) -> Result<u8, CheckpointError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(CheckpointError::format(
            "HEX",
            "invalid lowercase hexadecimal digit",
        )),
    }
}

/// Encode a true `u64` fact for JavaScript-safe handoff.
fn encode_u64_hex(value: u64) -> String {
    format!("{value:016x}")
}

/// Parse one canonical fixed-width lowercase hexadecimal `u64`.
fn parse_u64_hex(value: &str, label: &'static str) -> Result<u64, CheckpointError> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::format(
            "U64_HEX",
            format!("{label} must be 16 lowercase hexadecimal digits"),
        ));
    }
    u64::from_str_radix(value, 16)
        .map_err(|_| CheckpointError::format("U64_HEX", format!("{label} is not a canonical u64")))
}

/// Validate and retain a canonical fixed-width lowercase hexadecimal `u64`.
fn canonical_u64_hex(value: &str, label: &'static str) -> Result<String, CheckpointError> {
    Ok(encode_u64_hex(parse_u64_hex(value, label)?))
}

/// Checked `usize` to `u64` conversion.
fn usize_to_u64(value: usize, label: &'static str) -> Result<u64, CheckpointError> {
    u64::try_from(value)
        .map_err(|_| CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit u64")))
}

/// Checked `usize` to `u32` conversion.
fn usize_to_u32(value: usize, label: &'static str) -> Result<u32, CheckpointError> {
    u32::try_from(value)
        .map_err(|_| CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit u32")))
}

/// Checked `u64` to platform-size conversion.
fn u64_to_usize(value: u64, label: &'static str) -> Result<usize, CheckpointError> {
    usize::try_from(value).map_err(|_| {
        CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit usize"))
    })
}

/// Infallible-on-supported-target `u32` to `usize` conversion expressed as a contract check.
fn u32_to_usize(value: u32, label: &'static str) -> Result<usize, CheckpointError> {
    usize::try_from(value).map_err(|_| {
        CheckpointError::format("COUNT_OVERFLOW", format!("{label} does not fit usize"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::contract::ENGINE_CONTRACT_VERSION;
    use crate::engine::rng::StatefulRng;
    use crate::engine::state::{
        normalized_config_hash, normalized_settings_schema_hash, ALLOCATOR_VERSION,
        BASELINE_ENTITY_ID_START, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START,
        GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
        RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, SENSOR_VERSION, SERIALIZER_VERSION,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Collision-free per-process temporary directory counter.
    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    /// Exact test-owned directory removed on scope exit.
    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        /// Create one unique directory below the OS temporary root.
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "slither-checkpoint-v3-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory must be unique and creatable");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let temporary_root = std::env::temp_dir().canonicalize();
            let owned = self.path.canonicalize();
            if temporary_root
                .as_ref()
                .ok()
                .zip(owned.as_ref().ok())
                .is_some_and(|(root, path)| path.starts_with(root) && path != root)
            {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    /// Reviewed checkpoint bounds used only by local codec fixtures.
    fn checkpoint_limits() -> CheckpointLimits {
        CheckpointLimits {
            max_archive_bytes: 64 * 1024 * 1024,
            max_manifest_bytes: 64 * 1024,
            max_state_bytes: 256 * 1024,
            max_graph_bytes: 512 * 1024,
            max_population_index_bytes: 1024 * 1024,
            max_population_count: 512,
            max_setting_count: 32,
            max_baseline_rng_count: 32,
            max_string_bytes: 256 * 1024,
            max_total_string_bytes: 2 * 1024 * 1024,
            max_weight_floats: 8 * 1024 * 1024,
            max_recurrent_floats: 1024 * 1024,
            max_numeric_stored_bytes: 48 * 1024 * 1024,
            max_numeric_candidate_bytes: 48 * 1024 * 1024,
            max_total_decoded_bytes: 64 * 1024 * 1024,
        }
    }

    /// Reviewed graph bounds used by the small and representative fixtures.
    fn graph_limits() -> GraphLimits {
        GraphLimits {
            max_nodes: 16,
            max_edges: 32,
            max_graph_outputs: 4,
            max_identifier_bytes: 64,
            max_total_referenced_identifier_bytes: 1024,
            max_tensor_width: 1024,
            max_mlp_hidden_layers: 8,
            max_split_output_ports: 16,
            max_parameter_floats: 1_000_000,
            max_recurrent_state_floats: 16_384,
            max_canonical_layout_bytes: 128 * 1024,
            max_architecture_key_bytes: 256 * 1024,
        }
    }

    /// Source graph with either recurrent-state coverage or representative P0-sized weights.
    fn graph_spec(representative: bool) -> GraphSpec {
        if representative {
            GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".into(),
                        kind: GraphNodeKind::Input { output_size: 3 },
                    },
                    GraphNodeSpec {
                        id: "head".into(),
                        kind: GraphNodeKind::Mlp {
                            input_size: 3,
                            hidden_sizes: vec![128, 128],
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![GraphEdge {
                    from: "input".into(),
                    to: "head".into(),
                    from_port: None,
                    to_port: None,
                }],
                outputs: vec![GraphOutputRef {
                    node_id: "head".into(),
                    port: None,
                }],
                output_size: 2,
            }
        } else {
            GraphSpec {
                nodes: vec![
                    GraphNodeSpec {
                        id: "input".into(),
                        kind: GraphNodeKind::Input { output_size: 3 },
                    },
                    GraphNodeSpec {
                        id: "memory".into(),
                        kind: GraphNodeKind::Gru {
                            input_size: 3,
                            hidden_size: 2,
                        },
                    },
                    GraphNodeSpec {
                        id: "head".into(),
                        kind: GraphNodeKind::Dense {
                            input_size: 2,
                            output_size: 2,
                        },
                    },
                ],
                edges: vec![
                    GraphEdge {
                        from: "input".into(),
                        to: "memory".into(),
                        from_port: None,
                        to_port: None,
                    },
                    GraphEdge {
                        from: "memory".into(),
                        to: "head".into(),
                        from_port: None,
                        to_port: None,
                    },
                ],
                outputs: vec![GraphOutputRef {
                    node_id: "head".into(),
                    port: None,
                }],
                output_size: 2,
            }
        }
    }

    /// Admission policy exactly matching the fixture identity and settings schema.
    fn admission_policy(settings_schema_sha256: String) -> StateAdmissionPolicy {
        StateAdmissionPolicy {
            memory_ceiling_bytes: 128 * 1024 * 1024,
            expected_source_revision: "test-revision".into(),
            expected_engine_build_id: "test-engine-build".into(),
            expected_source_sha256: "1".repeat(64),
            expected_target_triple: "x86_64-pc-windows-msvc".into(),
            expected_build_profile: "release".into(),
            expected_build_class: "test-hooks".into(),
            expected_rustc_version: "rustc-test".into(),
            expected_build_contract_sha256: format!("sha256:{}", "2".repeat(64)),
            expected_math_backend: "scalar-test".into(),
            expected_settings_schema_sha256: settings_schema_sha256,
        }
    }

    /// Construct a fully admitted exact run-start boundary.
    fn admitted_state(
        population_count: usize,
        representative: bool,
    ) -> (AuthoritativeState, Arc<GraphBundle>, StateAdmissionPolicy) {
        let graph = Arc::new(
            GraphBundle::compile(graph_spec(representative), &graph_limits())
                .expect("fixture graph must compile"),
        );
        let settings = vec![
            NormalizedSetting {
                path: "baselineBots.count".into(),
                value: NormalizedSettingValue::Integer(0),
            },
            NormalizedSetting {
                path: "brain.sensorVersion".into(),
                value: NormalizedSettingValue::Integer(i64::from(SENSOR_VERSION)),
            },
            NormalizedSetting {
                path: "simSpeed".into(),
                value: NormalizedSettingValue::Float(1.0),
            },
            NormalizedSetting {
                path: "snakeCount".into(),
                value: NormalizedSettingValue::Integer(population_count as i64),
            },
            NormalizedSetting {
                path: "worldRadius".into(),
                value: NormalizedSettingValue::Integer(1000),
            },
        ];
        let settings_schema_sha256 =
            normalized_settings_schema_hash(&settings).expect("fixture setting schema must hash");
        let mut config = NormalizedEngineConfig {
            version: NORMALIZED_CONFIG_VERSION,
            settings,
            settings_schema_sha256: settings_schema_sha256.clone(),
            graph_architecture_key: graph.architecture_key.clone(),
            fixed_step_seconds: 1.0 / 60.0,
            requested_sim_speed: 1.0,
            world_radius: 1000.0,
            population_count,
            baseline_count: 0,
            max_world_snakes: population_count,
            max_non_population_brains: 0,
            max_body_points: 100_000,
            max_pellets: 10_000,
            spatial_index_bytes: 1024,
            worker_scratch_bytes: 1024,
            checkpoint_scratch_bytes: 2 * 1024 * 1024,
            controller_input_hold_ms: 200,
            controller_disconnect_grace_ms: 5000,
        };
        let config_hash = normalized_config_hash(&config).expect("fixture config must hash");
        let identity = RunIdentity {
            run_id: "11111111-2222-4333-8444-555555555555".into(),
            seed: 7,
            config_revision: 1,
            config_hash,
            source_revision: "test-revision".into(),
            engine_build_id: "test-engine-build".into(),
            source_sha256: "1".repeat(64),
            target_triple: "x86_64-pc-windows-msvc".into(),
            build_profile: "release".into(),
            build_class: "test-hooks".into(),
            rustc_version: "rustc-test".into(),
            build_contract_sha256: format!("sha256:{}", "2".repeat(64)),
            math_backend: "scalar-test".into(),
        };
        // Keep the assignment explicit if config construction changes in a future fixture.
        config.graph_architecture_key = graph.architecture_key.clone();
        let mut population = Vec::with_capacity(population_count);
        let mut brains = Vec::with_capacity(population_count);
        for slot in 0..population_count {
            let brain = BrainHandle {
                id: slot as u64 + 1,
                epoch: 1,
            };
            let weights: Vec<f32> = (0..graph.total_parameters)
                .map(|index| {
                    let mantissa = ((slot as u32).wrapping_mul(8191)
                        ^ (index as u32).wrapping_mul(37))
                        & 0x007f_ffff;
                    f32::from_bits(0x3f00_0000 | mantissa)
                })
                .collect();
            population.push(PopulationGenome {
                slot: slot as u32,
                brain,
                lineage: GenomeLineage {
                    genome_id: slot as u64 + 1,
                    birth_generation: 1,
                    parent_a: None,
                    parent_b: None,
                },
                fitness: slot as f64 / 10.0,
                weights: weights.into_boxed_slice(),
            });
            brains.push(BrainRuntimeState {
                handle: brain,
                owner: BrainOwner::PopulationSlot(slot as u32),
                non_population_weights: None,
                recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
            });
        }
        let rng = || StatefulRng::new(7.0).export_state();
        let candidate = StateCandidate {
            versions: ContractVersions {
                state: ENGINE_STATE_VERSION,
                engine: ENGINE_CONTRACT_VERSION,
                protocol: PROTOCOL_VERSION,
                serializer: SERIALIZER_VERSION,
                sensor: SENSOR_VERSION,
                rng_bundle: RNG_BUNDLE_VERSION,
                checkpoint: CHECKPOINT_VERSION,
                graph_layout: CANONICAL_GRAPH_LAYOUT_VERSION,
            },
            identity,
            config,
            phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart),
            generation: GenerationState {
                boundary_version: GENERATION_BOUNDARY_VERSION,
                generation: 1,
                completed_step: 0,
                population_epoch: 1,
                elapsed_seconds: 0.0,
                wall_accumulator_seconds: 0.0,
                best_fitness_ever: 0.0,
            },
            fixed_step: FixedStepContinuationState::generation_boundary(),
            rng: RngStateBundle {
                version: RNG_BUNDLE_VERSION,
                world: rng(),
                evolution: rng(),
                external_controller: rng(),
                baselines: Vec::new(),
            },
            allocators: AllocatorState {
                version: ALLOCATOR_VERSION,
                next_entity_id: 1,
                next_brain_id: population_count as u64 + 1,
                next_genome_id: population_count as u64 + 1,
                next_controller_lease_id: 1,
                next_frame_v1_id: 1,
                next_external_id: EXTERNAL_ENTITY_ID_START,
                next_baseline_id: BASELINE_ENTITY_ID_START,
                next_resurrected_id: RESURRECTED_ENTITY_ID_START,
            },
            population,
            brains,
            world: WorldState::default(),
        };
        let policy = admission_policy(settings_schema_sha256);
        let admitted = AuthoritativeState::validate_and_own(candidate, Arc::clone(&graph), &policy)
            .expect("fixture state must be admissible");
        (admitted, graph, policy)
    }

    /// Publish one fixture and return its exact managed path and descriptor.
    fn publish_fixture(
        directory: &TestDirectory,
        state: &AuthoritativeState,
        policy: &StateAdmissionPolicy,
        operation: &str,
    ) -> (PathBuf, CheckpointDescriptor) {
        let descriptor = publish_checkpoint(
            &directory.path,
            CheckpointOperationId::parse(operation).unwrap(),
            9,
            state.checkpoint_boundary().unwrap(),
            &checkpoint_limits(),
            &graph_limits(),
            policy,
        )
        .expect("checkpoint publication must succeed");
        let path = directory.path.join(&descriptor.relative_filename);
        (path, descriptor)
    }

    /// Recalculate one mutated USTAR header's standard checksum.
    fn refresh_test_ustar_checksum(header: &mut [u8]) {
        header[148..156].fill(b' ');
        let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
        let encoded = format!("{sum:06o}\0 ");
        assert_eq!(encoded.len(), 8);
        header[148..156].copy_from_slice(encoded.as_bytes());
    }

    /// Replace a USTAR header's fixed short path and refresh its checksum.
    fn set_test_ustar_path(header: &mut [u8], path: &str) {
        assert!(path.len() <= 100 && path.is_ascii());
        header[..100].fill(0);
        header[..path.len()].copy_from_slice(path.as_bytes());
        refresh_test_ustar_checksum(header);
    }

    /// Write a mutated archive under its expected digest filename and require rejection.
    fn assert_mutated_archive_rejected(
        label: &str,
        original: &[u8],
        filename: &str,
        policy: &StateAdmissionPolicy,
        mutate: impl FnOnce(&mut Vec<u8>),
    ) {
        let directory = TestDirectory::new(label);
        let path = directory.path.join(filename);
        let mut bytes = original.to_vec();
        mutate(&mut bytes);
        fs::write(&path, bytes).unwrap();
        assert!(
            restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), policy).is_err(),
            "mutated archive {label} must be rejected"
        );
    }

    /// The ordered root bytes must stay cross-language compatible with retained Stage 2.
    #[test]
    fn logical_root_matches_retained_typescript_fixture() {
        let roles = [
            LogicalRoleDigest {
                role: "checkpoint",
                logical_length: 30,
                logical_sha256: parse_digest(
                    "7e5a42a9d9efa796dddccc0ac2d2cbe5ca0131d573353608cf2d29929d8b5bd7",
                    "fixture checkpoint",
                )
                .unwrap(),
            },
            LogicalRoleDigest {
                role: "population-weights",
                logical_length: 64,
                logical_sha256: parse_digest(
                    "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e",
                    "fixture weights",
                )
                .unwrap(),
            },
        ];
        assert_eq!(
            encode_digest(compute_logical_root(&roles).unwrap()),
            "f715b31f2bbd45f0b8a9cc8f48062712bcebe880507e82332eaee0cc2bf7b481"
        );
    }

    /// A small recurrent checkpoint round-trips every graph/RNG/Float32 bit and is idempotent.
    #[test]
    fn small_recurrent_checkpoint_round_trips_bit_exactly_and_idempotently() {
        let directory = TestDirectory::new("small");
        let (state, _graph, policy) = admitted_state(3, false);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000001",
        );
        assert_eq!(descriptor.managed_root, "checkpoint-v3");
        assert_eq!(descriptor.protocol_version, 1);
        assert_eq!(descriptor.role_count_hex, "0000000000000005");
        assert_eq!(descriptor.recurrent_state_count_hex, "0000000000000006");
        assert_eq!(
            descriptor.relative_filename,
            format!("{}.checkpoint-v3", descriptor.logical_root_sha256)
        );
        let restored = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy)
            .expect("published checkpoint must strictly restore");
        assert_eq!(restored.state.state(), state.state());
        assert_eq!(restored.state.graph_spec(), state.graph_spec());
        for (left, right) in restored
            .state
            .state()
            .population
            .iter()
            .zip(&state.state().population)
        {
            assert_eq!(
                left.weights
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                right
                    .weights
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>()
            );
        }
        let second = publish_checkpoint(
            &directory.path,
            CheckpointOperationId::parse("00000000000000000000000000000002").unwrap(),
            10,
            state.checkpoint_boundary().unwrap(),
            &checkpoint_limits(),
            &graph_limits(),
            &policy,
        )
        .expect("strictly validated content-addressed file must publish idempotently");
        assert_eq!(second.relative_filename, descriptor.relative_filename);
        assert_eq!(
            second.operation_id.as_str(),
            "00000000000000000000000000000002"
        );
        assert_eq!(second.transition_epoch_hex, "000000000000000a");
    }

    /// An unusable zero transition epoch is rejected before any managed artifact exists.
    #[test]
    fn zero_transition_epoch_is_rejected_before_publication() {
        let directory = TestDirectory::new("zero-transition-epoch");
        let (state, _graph, policy) = admitted_state(2, false);
        let error = publish_checkpoint(
            &directory.path,
            CheckpointOperationId::parse("00000000000000000000000000000018").unwrap(),
            0,
            state.checkpoint_boundary().unwrap(),
            &checkpoint_limits(),
            &graph_limits(),
            &policy,
        )
        .expect_err("zero transition epoch must fail before publication");
        assert!(matches!(
            error,
            CheckpointError::Format {
                code: "TRANSITION_EPOCH",
                ..
            }
        ));
        assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
    }

    /// A 55-slot multi-block population selects compression only when measured smaller.
    #[test]
    fn representative_checkpoint_round_trips_multi_block_weights_bit_exactly() {
        let directory = TestDirectory::new("representative");
        let (state, _graph, policy) = admitted_state(55, true);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000003",
        );
        assert_eq!(
            descriptor.weights_encoding,
            NumericEncoding::F32LeShuffle4ZstdV1
        );
        assert_eq!(
            descriptor.recurrent_state_encoding,
            NumericEncoding::RawF32LeV1
        );
        let restored = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy)
            .expect("representative checkpoint must restore");
        assert_eq!(restored.state.state(), state.state());
        assert_eq!(
            restored.state.graph().layout_digest_sha256,
            state.graph().layout_digest_sha256
        );
    }

    /// A small feed-forward population round-trips independently from recurrent coverage.
    #[test]
    fn small_feed_forward_checkpoint_round_trips_bit_exactly() {
        let directory = TestDirectory::new("small-feed-forward");
        let (state, _graph, policy) = admitted_state(2, true);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000011",
        );
        assert_eq!(descriptor.recurrent_state_count_hex, "0000000000000000");
        let restored = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy)
            .expect("small feed-forward checkpoint must restore");
        assert_eq!(restored.state.state(), state.state());
    }

    /// Payload corruption is diagnosed before an invalid candidate can become authoritative.
    #[test]
    fn corrupted_numeric_payload_is_rejected() {
        let directory = TestDirectory::new("corrupt");
        let (state, _graph, policy) = admitted_state(4, false);
        let (path, _descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000004",
        );
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let length = file.metadata().unwrap().len();
        let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
        let target = &entries[3];
        file.seek(SeekFrom::Start(target.data_offset)).unwrap();
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x80;
        file.seek(SeekFrom::Start(target.data_offset)).unwrap();
        file.write_all(&byte).unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy).is_err());
    }

    /// Malformed SFZ1 envelope data is rejected before invoking Zstandard decode.
    #[test]
    fn malformed_sfz1_envelope_is_rejected() {
        let directory = TestDirectory::new("sfz1-malformed");
        let (state, _graph, policy) = admitted_state(55, true);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000012",
        );
        assert_eq!(
            descriptor.weights_encoding,
            NumericEncoding::F32LeShuffle4ZstdV1
        );
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let length = file.metadata().unwrap().len();
        let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
        file.seek(SeekFrom::Start(entries[3].data_offset)).unwrap();
        file.write_all(b"BAD!").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(matches!(
            restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &policy),
            Err(CheckpointError::Format {
                code: "SHUFFLED_MAGIC",
                ..
            })
        ));
    }

    /// A declared decoder window over the one-MiB contract fails during preflight.
    #[test]
    fn oversized_zstandard_window_is_rejected_before_numeric_allocation() {
        let mut frame_header = vec![0x28, 0xb5, 0x2f, 0xfd, 0x80, 0x58];
        frame_header.extend_from_slice(&(SHUFFLED_BLOCK_BYTES as u32).to_le_bytes());
        let error = validate_zstd_frame_header(&frame_header, SHUFFLED_BLOCK_BYTES)
            .expect_err("two-MiB decoder window must fail the one-MiB preflight bound");
        assert!(matches!(
            error,
            CheckpointError::Format {
                code: "ZSTD_WINDOW",
                ..
            }
        ));
    }

    /// A truncated partial is invalid while the same complete unreferenced file remains restorable.
    #[test]
    fn partial_is_rejected_and_complete_orphan_is_restorable() {
        let directory = TestDirectory::new("partial");
        let orphan_directory = TestDirectory::new("orphan");
        let (state, _graph, policy) = admitted_state(2, false);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000005",
        );
        let partial = directory.path.join("checkpoint-v3-test.partial");
        fs::copy(&path, &partial).unwrap();
        let partial_file = OpenOptions::new().write(true).open(&partial).unwrap();
        partial_file
            .set_len(partial_file.metadata().unwrap().len() - 512)
            .unwrap();
        drop(partial_file);
        assert!(
            restore_checkpoint(&partial, &checkpoint_limits(), &graph_limits(), &policy).is_err()
        );

        let orphan = orphan_directory.path.join(&descriptor.relative_filename);
        fs::copy(&path, &orphan).unwrap();
        let restored = restore_checkpoint(&orphan, &checkpoint_limits(), &graph_limits(), &policy)
            .expect("complete metadata-orphan file remains a valid immutable checkpoint");
        assert_eq!(restored.state.state(), state.state());
    }

    /// The selected raw representation remains valid when the discarded SFZ1 candidate expands.
    #[test]
    fn raw_at_selected_entry_limit_succeeds_when_sfz1_candidate_expands() {
        let directory = TestDirectory::new("raw-limit");
        let mut word = 0x1234_5678u32;
        let mut values = Vec::new();
        for _ in 0..16 {
            word ^= word << 13;
            word ^= word >> 17;
            word ^= word << 5;
            if word & 0x7f80_0000 == 0x7f80_0000 {
                word ^= 0x0080_0000;
            }
            values.push(f32::from_bits(word));
        }
        let source = FloatSource::new(vec![values.as_slice()], values.len(), "raw-limit").unwrap();
        let mut limits = checkpoint_limits();
        limits.max_numeric_stored_bytes = source.raw_bytes().unwrap();
        limits.max_numeric_candidate_bytes = 4096;
        let mut artifacts = TemporaryArtifacts::new();
        let candidate = select_numeric_candidate(
            &directory.path,
            "00000000000000000000000000000006",
            "raw-limit",
            &source,
            &limits,
            &mut artifacts,
        )
        .expect("raw at the selected limit must not be rejected by an expanding candidate");
        assert_eq!(candidate.encoding, NumericEncoding::RawF32LeV1);
        assert_eq!(candidate.stored_bytes, limits.max_numeric_stored_bytes);
        assert!(candidate.encoded_blocks >= 1);
        assert_eq!(candidate.scratch_capacity_growths, 0);
    }

    /// Multi-block encoding reuses its three bounded scratch allocations without capacity growth.
    #[test]
    fn multi_block_codec_reuses_bounded_scratch() {
        let directory = TestDirectory::new("scratch");
        let count = SHUFFLED_BLOCK_BYTES / 4 * 3 + 17;
        let values = vec![f32::from_bits(0x3f12_3456); count];
        let source = FloatSource::new(vec![values.as_slice()], count, "scratch").unwrap();
        let mut artifacts = TemporaryArtifacts::new();
        let candidate = select_numeric_candidate(
            &directory.path,
            "00000000000000000000000000000007",
            "scratch",
            &source,
            &checkpoint_limits(),
            &mut artifacts,
        )
        .unwrap();
        assert_eq!(candidate.encoded_blocks, 4);
        assert_eq!(candidate.scratch_capacity_growths, 0);
        assert_eq!(candidate.encoding, NumericEncoding::F32LeShuffle4ZstdV1);
    }

    /// Archive length admission happens before an archive partial is created.
    #[test]
    fn undersized_archive_limit_leaves_no_partial_or_candidate() {
        let directory = TestDirectory::new("archive-limit");
        let (state, _graph, policy) = admitted_state(3, false);
        let mut limits = checkpoint_limits();
        limits.max_archive_bytes = 1024;
        let error = publish_checkpoint(
            &directory.path,
            CheckpointOperationId::parse("00000000000000000000000000000008").unwrap(),
            1,
            state.checkpoint_boundary().unwrap(),
            &limits,
            &graph_limits(),
            &policy,
        )
        .expect_err("undersized archive limit must reject before publication");
        assert!(matches!(
            error,
            CheckpointError::Format {
                code: "ARCHIVE_LIMIT",
                ..
            }
        ));
        assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
    }

    /// Shared state admission rejects an oversized candidate before touching corrupt numeric bytes.
    #[test]
    fn memory_ceiling_is_enforced_before_numeric_decode_allocation() {
        let directory = TestDirectory::new("memory-preflight");
        let (state, _graph, policy) = admitted_state(8, false);
        let (path, _) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000009",
        );
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let length = file.metadata().unwrap().len();
        let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
        file.seek(SeekFrom::Start(entries[3].data_offset)).unwrap();
        file.write_all(b"BROKEN").unwrap();
        file.sync_all().unwrap();
        drop(file);
        let mut constrained = admission_policy(policy.expected_settings_schema_sha256.clone());
        constrained.memory_ceiling_bytes = 1;
        let error = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &constrained)
            .expect_err("memory ceiling must reject before corrupt numeric decode");
        assert!(matches!(
            error,
            CheckpointError::State(StateError::MemoryCeilingExceeded { .. })
        ));
    }

    /// Manifest-declared population workspace is charged before index bytes are decoded.
    #[test]
    fn population_workspace_ceiling_is_enforced_before_index_decode() {
        let directory = TestDirectory::new("population-predecode");
        let (state, _graph, policy) = admitted_state(480, true);
        let (path, _) = publish_fixture(
            &directory,
            &state,
            &policy,
            "0000000000000000000000000000001a",
        );
        let mut file = File::open(&path).unwrap();
        let length = file.metadata().unwrap().len();
        let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[entries[2].data_offset as usize] ^= 0xff;
        fs::write(&path, bytes).unwrap();

        let minimum = checkpoint_workspace_bound(&checkpoint_limits(), 0).unwrap();
        let declared = checkpoint_workspace_bound(&checkpoint_limits(), 480).unwrap();
        assert!(declared > minimum);
        let mut constrained = admission_policy(policy.expected_settings_schema_sha256.clone());
        constrained.memory_ceiling_bytes = minimum + (declared - minimum) / 2;
        let error = restore_checkpoint(&path, &checkpoint_limits(), &graph_limits(), &constrained)
            .expect_err("population workspace must fail before the corrupt index is read");
        assert!(
            matches!(
                error,
                CheckpointError::State(StateError::MemoryCeilingExceeded { .. })
            ),
            "unexpected predecode error: {error:?}"
        );
    }

    /// Codec-derived working memory replaces an underdeclared configuration reservation.
    #[test]
    fn publication_charges_actual_checkpoint_workspace_against_memory_ceiling() {
        let directory = TestDirectory::new("workspace-accounting");
        let (state, _graph, policy) = admitted_state(8, false);
        let boundary = state.checkpoint_boundary().unwrap();
        let workspace = checkpoint_workspace_bound(&checkpoint_limits(), 8).unwrap();
        let declared = boundary.state().config.checkpoint_scratch_bytes;
        assert!(workspace > declared);
        let actual_peak = boundary
            .memory_estimate()
            .total_bytes
            .checked_sub(declared)
            .and_then(|value| value.checked_add(workspace))
            .unwrap();
        let mut constrained = admission_policy(policy.expected_settings_schema_sha256.clone());
        constrained.memory_ceiling_bytes = actual_peak - 1;
        let error = publish_checkpoint(
            &directory.path,
            CheckpointOperationId::parse("00000000000000000000000000000019").unwrap(),
            1,
            boundary,
            &checkpoint_limits(),
            &graph_limits(),
            &constrained,
        )
        .expect_err("actual checkpoint workspace must be charged before encoding");
        assert!(matches!(
            error,
            CheckpointError::State(StateError::MemoryCeilingExceeded { .. })
        ));
        assert_eq!(fs::read_dir(&directory.path).unwrap().count(), 0);
    }

    /// Manifest mirrors are not trusted merely because the logical role root is unchanged.
    #[test]
    fn manifest_mirror_tampering_is_rejected() {
        let directory = TestDirectory::new("manifest-source");
        let (state, _graph, policy) = admitted_state(3, false);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "0000000000000000000000000000000a",
        );
        let mut file = File::open(&path).unwrap();
        let length = file.metadata().unwrap().len();
        let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
        let manifest = &entries[5];
        let original = fs::read(&path).unwrap();
        assert_mutated_archive_rejected(
            "manifest-tamper",
            &original,
            &descriptor.relative_filename,
            &policy,
            |bytes| {
                let start = manifest.data_offset as usize;
                let end = start + manifest.size as usize;
                let needle = b"\"generationHex\":\"0000000000000001\"";
                let position = bytes[start..end]
                    .windows(needle.len())
                    .position(|window| window == needle)
                    .expect("manifest generation mirror must be present")
                    + start;
                bytes[position + needle.len() - 2] = b'2';
            },
        );
    }

    /// Strict USTAR parsing rejects checksum, trailer, padding, duplicate/order/path, and type faults.
    #[test]
    fn strict_ustar_fault_matrix_is_rejected() {
        let directory = TestDirectory::new("ustar-source");
        let (state, _graph, policy) = admitted_state(3, false);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "0000000000000000000000000000000b",
        );
        let mut file = File::open(&path).unwrap();
        let length = file.metadata().unwrap().len();
        let entries = scan_strict_ustar(&mut file, length, &checkpoint_limits()).unwrap();
        let original = fs::read(&path).unwrap();
        let filename = descriptor.relative_filename.as_str();

        assert_mutated_archive_rejected("ustar-checksum", &original, filename, &policy, |bytes| {
            bytes[0] ^= 1;
        });
        assert_mutated_archive_rejected("ustar-trailer", &original, filename, &policy, |bytes| {
            let last = bytes.len() - 1;
            bytes[last] = 1;
        });
        assert_mutated_archive_rejected("ustar-padding", &original, filename, &policy, |bytes| {
            let end = (entries[0].data_offset + entries[0].size) as usize;
            assert_ne!(end % 512, 0);
            bytes[end] = 1;
        });
        assert_mutated_archive_rejected("ustar-duplicate", &original, filename, &policy, |bytes| {
            let start = entries[1].header_offset as usize;
            set_test_ustar_path(&mut bytes[start..start + 512], STATE_PATH);
        });
        assert_mutated_archive_rejected("ustar-path", &original, filename, &policy, |bytes| {
            set_test_ustar_path(&mut bytes[..512], "../evil");
        });
        assert_mutated_archive_rejected("ustar-type", &original, filename, &policy, |bytes| {
            bytes[156] = b'2';
            refresh_test_ustar_checksum(&mut bytes[..512]);
        });
        assert_mutated_archive_rejected(
            "ustar-manifest-order",
            &original,
            filename,
            &policy,
            |bytes| {
                let manifest_header = entries[5].header_offset as usize;
                let recurrent_header = entries[4].header_offset as usize;
                set_test_ustar_path(
                    &mut bytes[manifest_header..manifest_header + 512],
                    RECURRENT_RAW_PATH,
                );
                set_test_ustar_path(
                    &mut bytes[recurrent_header..recurrent_header + 512],
                    MANIFEST_PATH,
                );
            },
        );
    }

    /// A corrupt same-name destination is never overwritten or trusted by filename alone.
    #[test]
    fn corrupt_existing_digest_file_is_left_untouched() {
        let directory = TestDirectory::new("collision");
        let (state, _graph, policy) = admitted_state(3, false);
        let (path, _) = publish_fixture(
            &directory,
            &state,
            &policy,
            "0000000000000000000000000000000c",
        );
        let mut corrupt = fs::read(&path).unwrap();
        corrupt[0] ^= 1;
        fs::write(&path, &corrupt).unwrap();
        let error = publish_checkpoint(
            &directory.path,
            CheckpointOperationId::parse("0000000000000000000000000000000d").unwrap(),
            2,
            state.checkpoint_boundary().unwrap(),
            &checkpoint_limits(),
            &graph_limits(),
            &policy,
        )
        .expect_err("corrupt digest collision must fail");
        assert!(matches!(
            error,
            CheckpointError::Format {
                code: "DIGEST_COLLISION",
                ..
            }
        ));
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    /// Concurrent same-root publishers cannot replace each other and both validate one immutable file.
    #[test]
    fn concurrent_same_root_publication_is_no_replace_and_idempotent() {
        let directory = TestDirectory::new("race");
        let (state, _graph, policy) = admitted_state(3, false);
        let results = std::thread::scope(|scope| {
            let left = scope.spawn(|| {
                publish_checkpoint(
                    &directory.path,
                    CheckpointOperationId::parse("0000000000000000000000000000000e").unwrap(),
                    14,
                    state.checkpoint_boundary().unwrap(),
                    &checkpoint_limits(),
                    &graph_limits(),
                    &policy,
                )
            });
            let right = scope.spawn(|| {
                publish_checkpoint(
                    &directory.path,
                    CheckpointOperationId::parse("0000000000000000000000000000000f").unwrap(),
                    15,
                    state.checkpoint_boundary().unwrap(),
                    &checkpoint_limits(),
                    &graph_limits(),
                    &policy,
                )
            });
            (left.join().unwrap(), right.join().unwrap())
        });
        let left = results.0.unwrap();
        let right = results.1.unwrap();
        assert_eq!(left.relative_filename, right.relative_filename);
        assert_eq!(left.logical_root_sha256, right.logical_root_sha256);
        assert_eq!(
            fs::read_dir(&directory.path)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "checkpoint-v3"))
                .count(),
            1
        );
    }

    /// Direct restore rejects non-regular filesystem objects before archive parsing.
    #[test]
    fn restore_rejects_non_regular_target() {
        let directory = TestDirectory::new("non-regular");
        let error = restore_checkpoint(
            &directory.path,
            &checkpoint_limits(),
            &graph_limits(),
            &admission_policy("sha256:".to_owned() + &"0".repeat(64)),
        )
        .expect_err("directory target must be rejected");
        assert!(matches!(
            error,
            CheckpointError::Format {
                code: "MANAGED_FILE_TYPE",
                ..
            }
        ));
    }

    /// Unix restores reject a direct symlink even when it points to a valid regular checkpoint.
    #[cfg(unix)]
    #[test]
    fn restore_rejects_symlink_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("symlink");
        let (state, _graph, policy) = admitted_state(2, false);
        let (path, descriptor) = publish_fixture(
            &directory,
            &state,
            &policy,
            "00000000000000000000000000000010",
        );
        let link_directory = TestDirectory::new("symlink-link");
        let link = link_directory.path.join(descriptor.relative_filename);
        symlink(path, &link).unwrap();
        assert!(matches!(
            restore_checkpoint(&link, &checkpoint_limits(), &graph_limits(), &policy),
            Err(CheckpointError::Format {
                code: "MANAGED_FILE_TYPE",
                ..
            })
        ));
    }
}
