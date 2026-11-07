//! Strict checkpoint-v3 encoding, restore, and immutable managed-file publication.
//!
//! The container is an intentionally small USTAR subset with fixed role paths,
//! ordinary regular files only, and `manifest.json` as the final entry. Engine
//! state, graph, and population records use explicit bounded little-endian
//! binary contracts. Only the small private manifest uses Serde.

use super::graph::{
    CompiledGraph, GraphBundle, GraphEdge, GraphLimits, GraphNodeKind, GraphNodeSpec,
    GraphOutputRef, GraphSpec, CANONICAL_GRAPH_LAYOUT_VERSION,
};
use super::rng::SerializedRngState;
use super::state::{
    preflight_generation_boundary_allocation, AllocatorState, AuthoritativeState, AuthorityPhase,
    BaselineRngState, BrainHandle, BrainOwner, BrainRuntimeState, ContractVersions,
    EvolvedRespawnRngState, FixedStepContinuationState, GenerationBoundaryKind,
    GenerationBoundaryView, GenerationState, GenomeLineage, NormalizedEngineConfig,
    NormalizedSetting, NormalizedSettingValue, PopulationGenome, RngStateBundle, RunIdentity,
    StateAdmissionPolicy, StateCandidate, StateError, WorldState, CHECKPOINT_VERSION,
};
use super::work_progress::{advance as advance_work_progress, ArchivePhase, ArchivePhaseScope};
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
// Level 1 keeps adaptive shuffled encoding while shortening the generation
// boundary for large brains; raw blocks still win whenever compression grows.
const ZSTD_LEVEL: i32 = 1;
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
/// Content-addressed suffix for one independently retained Hall-of-Fame genome.
const HALL_OF_FAME_WEIGHTS_SUFFIX: &str = ".hof-weights-v1";
/// Required final manifest path.
const MANIFEST_PATH: &str = "manifest.json";
/// State-role binary magic.
const STATE_MAGIC: &[u8; 8] = b"SLCSTV3\0";
/// State-role encoding with a bounded optional legacy origin after allocator state.
const STATE_ENCODING_WITH_LEGACY_ORIGIN: u32 = 4;
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
#[cfg_attr(feature = "engine-test-hooks", derive(serde::Serialize))]
#[cfg_attr(feature = "engine-test-hooks", serde(rename_all = "camelCase"))]
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
        Ok(Self(value.into_boxed_str().into_string()))
    }

    /// Borrow the exact validated operation token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Heap bytes retained by this owned correlation token.
    #[must_use]
    pub(crate) fn owned_bytes(&self) -> usize {
        self.0.capacity()
    }
}

/// Versioned numeric-role encoding selected from measured stored bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum NumericEncoding {
    /// Packed four-byte little-endian Float32 values.
    #[serde(rename = "raw-f32le-v1")]
    RawF32LeV1,
    /// One-MiB byte-shuffled Float32 blocks in independent Zstandard frames.
    #[serde(rename = "f32le-shuffle4-zstd-v1")]
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
    pub(super) fn parse(value: &str) -> Result<Self, CheckpointError> {
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

/// Content-addressed weight object shared by winners and saved neural brains.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedNeuralWeightsDescriptor {
    /// Version of this scalar descriptor and file contract.
    pub version: u32,
    /// Encoding-independent SHA-256 of packed little-endian Float32 bits.
    pub logical_sha256: String,
    /// Digest-derived direct child of the controlled managed directory.
    pub relative_filename: String,
    /// Selected adaptive numeric encoding.
    pub encoding: NumericEncoding,
    /// Complete stored file bytes as canonical `u64` hexadecimal.
    #[serde(rename = "storedByteCount")]
    pub stored_byte_count_hex: String,
    /// Packed logical bytes as canonical `u64` hexadecimal.
    #[serde(rename = "decodedByteCount")]
    pub decoded_byte_count_hex: String,
    /// Exact Float32 parameter count as canonical `u64` hexadecimal.
    #[serde(rename = "weightCount")]
    pub weight_count_hex: String,
}

/// Compatibility name for existing HoF callers and immutable file metadata.
pub type HallOfFameWeightsDescriptor = ManagedNeuralWeightsDescriptor;

impl ManagedNeuralWeightsDescriptor {
    /// Exact string-storage bytes retained after publication compacts spare capacity.
    pub(crate) const PUBLICATION_OWNED_BYTES: usize =
        64 + 64 + HALL_OF_FAME_WEIGHTS_SUFFIX.len() + 3 * 16;

    /// Remove spare string capacity before the descriptor crosses a bounded queue.
    fn compact_storage(mut self) -> Self {
        for value in [
            &mut self.logical_sha256,
            &mut self.relative_filename,
            &mut self.stored_byte_count_hex,
            &mut self.decoded_byte_count_hex,
            &mut self.weight_count_hex,
        ] {
            *value = std::mem::take(value).into_boxed_str().into_string();
        }
        self
    }

    /// Heap bytes retained by one reliable output event.
    #[must_use]
    pub(crate) fn owned_bytes(&self) -> usize {
        self.logical_sha256
            .capacity()
            .saturating_add(self.relative_filename.capacity())
            .saturating_add(self.stored_byte_count_hex.capacity())
            .saturating_add(self.decoded_byte_count_hex.capacity())
            .saturating_add(self.weight_count_hex.capacity())
    }
}

impl CheckpointDescriptor {
    /// Exact string-storage bound for a Rust-published compact descriptor.
    /// Twelve u64 words, two digests, the managed-root/filename labels and the
    /// fixed operation token have invariant lengths. Publication removes spare
    /// string capacity so this bound also covers actual retained heap bytes.
    pub(crate) fn publication_owned_byte_bound(run_id: &str) -> Option<usize> {
        run_id.len().checked_add(
            12 * 16 + 2 * 64 + "checkpoint-v3".len() + 64 + ".checkpoint-v3".len() + 32,
        )
    }

    /// Remove spare string capacity before the descriptor crosses a bounded queue.
    fn compact_storage(mut self) -> Self {
        for value in [
            &mut self.managed_root,
            &mut self.operation_id.0,
            &mut self.transition_epoch_hex,
            &mut self.run_id,
            &mut self.generation_hex,
            &mut self.completed_step_hex,
            &mut self.checkpoint_format_version_hex,
            &mut self.state_version_hex,
            &mut self.graph_layout_version_hex,
            &mut self.logical_root_sha256,
            &mut self.relative_filename,
            &mut self.stored_byte_count_hex,
            &mut self.decoded_byte_count_hex,
            &mut self.population_count_hex,
            &mut self.role_count_hex,
            &mut self.weight_count_hex,
            &mut self.recurrent_state_count_hex,
            &mut self.graph_layout_sha256,
        ] {
            *value = std::mem::take(value).into_boxed_str().into_string();
        }
        self
    }

    /// Heap bytes retained by the bounded descriptor strings.
    ///
    /// This is queue-accounting metadata only. It deliberately excludes the
    /// fixed-size enum and integer fields stored inline with the event.
    #[must_use]
    pub(crate) fn owned_bytes(&self) -> usize {
        [
            self.managed_root.capacity(),
            self.operation_id.owned_bytes(),
            self.transition_epoch_hex.capacity(),
            self.run_id.capacity(),
            self.generation_hex.capacity(),
            self.completed_step_hex.capacity(),
            self.checkpoint_format_version_hex.capacity(),
            self.state_version_hex.capacity(),
            self.graph_layout_version_hex.capacity(),
            self.logical_root_sha256.capacity(),
            self.relative_filename.capacity(),
            self.stored_byte_count_hex.capacity(),
            self.decoded_byte_count_hex.capacity(),
            self.population_count_hex.capacity(),
            self.role_count_hex.capacity(),
            self.weight_count_hex.capacity(),
            self.recurrent_state_count_hex.capacity(),
            self.graph_layout_sha256.capacity(),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }

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

    /// Compare only immutable content, permitting an idempotent import to reuse
    /// the destination database's original publication token and epoch.
    #[must_use]
    pub fn first_content_mismatch(&self, actual: &Self) -> Option<&'static str> {
        let mut normalized = actual.clone();
        normalized.operation_id = self.operation_id.clone();
        normalized.transition_epoch_hex = self.transition_epoch_hex.clone();
        self.first_mismatch(&normalized)
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
    pub(crate) fn format(code: &'static str, detail: impl Into<String>) -> Self {
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

/// One already validated checkpoint role that can be copied into a flat save archive.
#[derive(Clone, Debug)]
pub(super) struct CheckpointArchiveRole {
    pub(super) manifest: ManifestRole,
    pub(super) data_offset: u64,
    pub(super) stored_bytes: u64,
}

/// Validated checkpoint manifest and direct role extents used by exact save export.
#[derive(Clone, Debug)]
pub(super) struct CheckpointArchiveLayout {
    pub(super) manifest: CheckpointManifest,
    pub(super) roles: Vec<CheckpointArchiveRole>,
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

/// Count a metadata-only boundary using the writer's encoders before allocating numeric roles.
/// Fixed-width population records have the same size before and after recurrent initialization.
pub(crate) fn boundary_decoded_byte_count(
    state: &StateCandidate,
    spec: &GraphSpec,
    graph: &super::graph::CompiledGraph,
    limits: &CheckpointLimits,
) -> Result<u64, CheckpointError> {
    decoded_role_byte_count(boundary_decoded_role_byte_counts(
        state, spec, graph, limits,
    )?)
}

/// Exact state/graph/index/weight/recurrent role sizes, including metadata-only boundaries.
pub(crate) fn boundary_decoded_role_byte_counts(
    state: &StateCandidate,
    spec: &GraphSpec,
    graph: &super::graph::CompiledGraph,
    limits: &CheckpointLimits,
) -> Result<[u64; LOGICAL_ROLE_COUNT], CheckpointError> {
    validate_limits(limits)?;
    let weight_bytes = usize_to_u64(state.population.len(), "population count")?
        .checked_mul(usize_to_u64(graph.total_parameters, "parameter count")?)
        .and_then(|count| count.checked_mul(size_of::<f32>() as u64))
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "population weight bytes overflow")
        })?;
    let recurrent_bytes = usize_to_u64(state.brains.len(), "brain count")?
        .checked_mul(usize_to_u64(graph.total_state_size, "recurrent count")?)
        .and_then(|count| count.checked_mul(size_of::<f32>() as u64))
        .ok_or_else(|| CheckpointError::format("COUNT_OVERFLOW", "recurrent bytes overflow"))?;
    Ok([
        encode_state(state, limits)?.len() as u64,
        encode_graph(spec, graph, limits)?.len() as u64,
        encode_population_index(
            state,
            graph.total_parameters,
            graph.total_state_size,
            limits,
        )?
        .len() as u64,
        weight_bytes,
        recurrent_bytes,
    ])
}

/// Sum exact logical role lengths with the same checked arithmetic at admission and publication.
fn decoded_role_byte_count(lengths: [u64; LOGICAL_ROLE_COUNT]) -> Result<u64, CheckpointError> {
    lengths
        .into_iter()
        .try_fold(0u64, |total, value| total.checked_add(value))
        .ok_or_else(|| {
            CheckpointError::format("COUNT_OVERFLOW", "preflight decoded-byte total overflows")
        })
}

/// Publish one exact generation-boundary checkpoint as an immutable managed file.
///
/// Ordinary publication deliberately performs no mandatory second full decode.
/// A pre-existing destination is fully stream-decoded and hashed against the
/// admitted logical root without allocating a second population.
pub fn publish_checkpoint(
    managed_directory: &Path,
    operation_id: CheckpointOperationId,
    transition_epoch: u64,
    boundary: GenerationBoundaryView<'_>,
    limits: &CheckpointLimits,
    _graph_limits: &GraphLimits,
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
    let preflight_decoded_bytes = decoded_role_byte_count([
        state_role.bytes.len() as u64,
        graph_role.bytes.len() as u64,
        population_role.bytes.len() as u64,
        weight_source.raw_bytes()?,
        recurrent_source.raw_bytes()?,
    ])?;
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
            boundary.graph(),
            &logical_root_hex,
            limits,
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
    #[cfg(test)]
    super::task_panic_fixture::hit(
        super::task_panic_fixture::PanicPoint::CheckpointWritten,
        &partial_path,
    );
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
                boundary.graph(),
                &logical_root_hex,
                limits,
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
pub(crate) fn checkpoint_workspace_bound(
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
    let decoder_workspace = [
        SHUFFLED_BLOCK_BYTES,
        SHUFFLED_BLOCK_BYTES,
        frame_bytes,
        decompressor_context,
    ]
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

/// Atomically publish without replacing an existing content-addressed file on Linux.
#[cfg(target_os = "linux")]
pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};

    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

/// Atomically rename on Windows without permitting replacement or copy across volumes.
#[cfg(target_os = "windows")]
pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let source = windows_publication_path(source)?;
    let destination = windows_publication_path(destination)?;
    // SAFETY: both owned buffers contain initialized UTF-16 with exactly one
    // trailing NUL and remain live throughout the call. MoveFileExW reads the
    // paths and retains no pointer. Zero flags forbid replacement, deferred
    // moves and cross-volume copy/delete; callers already fsync file contents.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Use a canonical parent for extended-length Windows paths without following the final name.
#[cfg(target_os = "windows")]
fn windows_publication_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication path needs a filename",
        )
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let absolute = parent.canonicalize()?.join(filename);
    let mut encoded: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication path contains NUL",
        ));
    }
    encoded.push(0);
    Ok(encoded)
}

/// Unsupported development targets use an atomic exclusive hard-link publication.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(source, destination)?;
    fs::remove_file(source)
}

/// Flush the containing directory on Unix after the same-directory rename.
#[cfg(unix)]
pub(crate) fn sync_parent_directory(directory: &Path) -> Result<(), CheckpointError> {
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// Windows has no portable directory handle `sync_all`; file fsync and rename still apply.
#[cfg(not(unix))]
pub(crate) fn sync_parent_directory(_directory: &Path) -> Result<(), CheckpointError> {
    Ok(())
}

/// Accept an existing digest-derived file only after a complete strict restore and state comparison.
#[allow(clippy::too_many_arguments)]
/// Verify every logical byte without allocating another population. The expected
/// root comes from an independently admitted boundary or fully restored import.
pub(super) fn verify_existing_checkpoint_content(
    path: &Path,
    expected_state: &StateCandidate,
    expected_graph: &CompiledGraph,
    expected_root: &str,
    limits: &CheckpointLimits,
) -> Result<CheckpointContentDescriptor, CheckpointError> {
    validate_limits(limits)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > limits.max_archive_bytes
    {
        return Err(CheckpointError::format(
            "MANAGED_FILE_TYPE",
            "existing checkpoint must be one bounded regular file",
        ));
    }
    let mut file = File::open(path)?;
    let archive_length = file.metadata()?.len();
    let entries = scan_strict_ustar(&mut file, archive_length, limits)?;
    let manifest_entry = entries
        .last()
        .ok_or_else(|| CheckpointError::format("USTAR_EMPTY", "archive has no entries"))?;
    let manifest_bytes = read_entry_bytes(&mut file, manifest_entry, limits.max_manifest_bytes)?;
    let manifest: CheckpointManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| CheckpointError::format("MANIFEST_JSON", error.to_string()))?;
    let validated = validate_manifest(&manifest, &entries, limits)?;
    if manifest.logical_root_sha256 != expected_root {
        return Err(CheckpointError::format(
            "DIGEST_COLLISION",
            "existing checkpoint has a different logical root",
        ));
    }
    crosscheck_manifest_state(&manifest, &validated, expected_state, expected_graph)?;
    for (index, maximum) in [
        limits.max_state_bytes,
        limits.max_graph_bytes,
        limits.max_population_index_bytes,
    ]
    .into_iter()
    .enumerate()
    {
        read_and_verify_small_role(&mut file, &entries[index], &validated.roles[index], maximum)?;
    }
    for (index, encoding, count) in [
        (3, validated.weights_encoding, validated.weight_float_count),
        (
            4,
            validated.recurrent_encoding,
            validated.recurrent_float_count,
        ),
    ] {
        file.seek(SeekFrom::Start(entries[index].data_offset))?;
        decode_adaptive_numeric_reader(
            &mut file,
            encoding,
            entries[index].size,
            usize_to_u64(count, "numeric float count")?,
            validated.roles[index].logical_sha256,
            &mut io::sink(),
        )?;
    }
    content_descriptor_from_manifest(
        &manifest,
        archive_length,
        path.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                CheckpointError::format("MANAGED_FILENAME", "checkpoint filename is not UTF-8")
            })?
            .to_owned(),
    )
}

/// Preserve descriptor operation identity while verifying the existing content.
fn validate_existing_idempotent(
    final_path: &Path,
    operation_id: &CheckpointOperationId,
    transition_epoch: u64,
    expected_state: &StateCandidate,
    expected_graph: &CompiledGraph,
    expected_root: &str,
    limits: &CheckpointLimits,
) -> Result<CheckpointDescriptor, CheckpointError> {
    let content = verify_existing_checkpoint_content(
        final_path,
        expected_state,
        expected_graph,
        expected_root,
        limits,
    )
    .map_err(|error| {
        CheckpointError::format(
            "DIGEST_COLLISION",
            format!("existing digest-derived file failed strict validation: {error}"),
        )
    })?;
    Ok(publication_descriptor(
        content,
        operation_id.clone(),
        transition_epoch,
    ))
}

mod manifest_validation;
pub(crate) use manifest_validation::publication_descriptor_for_restored;
use manifest_validation::*;
pub(super) use manifest_validation::{CheckpointManifest, ManifestRole};
mod numeric_sources;
use numeric_sources::*;
mod numeric_roles;
use numeric_roles::*;
// Preserve the existing engine-visible facade even in build classes without direct callers.
#[allow(unused_imports)]
pub(super) use numeric_roles::{
    decode_adaptive_numeric_reader, select_adaptive_numeric_file, AdaptiveNumericFile,
};
mod managed_weights;
pub use managed_weights::{publish_hall_of_fame_weights, publish_managed_neural_weights};
#[allow(unused_imports)]
pub(crate) use managed_weights::{
    read_validated_hall_of_fame_weights, validate_existing_hall_of_fame_weights,
};
mod managed_restore;
pub(super) use managed_restore::validated_checkpoint_archive_layout;
pub use managed_restore::{restore_checkpoint, restore_committed_checkpoint};
mod graph_population_wire;
use graph_population_wire::*;
mod state_wire;
use state_wire::*;
mod strict_ustar;
use strict_ustar::*;
#[allow(unused_imports)]
pub(super) use strict_ustar::{
    scan_strict_ustar_role_range, scan_strict_ustar_roles, validate_scanned_entry_size,
    ScannedEntry,
};

#[cfg(test)]
mod tests;
