/** Exact sixteen-digit lowercase hexadecimal u64 word at the coarse boundary. */
export type DistillationWord = string;

/** Cold diagnostic budgets, with no controller or reliable-output reservation. */
export interface TeacherCaptureRequest {
  /** New nonzero capture identity, encoded as 32 lowercase hexadecimal digits. */
  captureEpoch: string;
  /** Current run identity expected when the ordered arm is applied. */
  expectedRunId: string;
  /** Exact current configuration revision as a hexadecimal u64 word. */
  expectedConfigRevision: DistillationWord;
  /** Strictly increasing unique baseline slots, from zero through 119. */
  baselineSlots: readonly number[];
  /** Total admitted reusable rows and sealing-copy headroom, at most 16 MiB. */
  pendingBytes: number;
  /** Maximum complete objects retained in the separate native queue. */
  queueChunks: number;
  /** Total native queue/handoff/metadata/encoding admission, at most 16 MiB. */
  queueBytes: number;
}

/** Rust-owned identity repeated in every self-contained immutable binary object. */
export interface TeacherCaptureContract {
  /** Nonzero 128-bit capture epoch. */
  captureEpoch: string;
  /** Authoritative run identity. */
  runId: string;
  /** Canonical run configuration digest without the sha256 prefix. */
  configSha256: string;
  /** Digest of exact Rust canonical sensor metadata. */
  sensorSha256: string;
  /** Source-bound baseline teacher implementation identity. */
  teacherSha256: string;
  /** Explicit teacher formula revision; currently two. */
  teacherRevision: number;
  /** Zero deterministic or one stochastic behavior mode. */
  teacherMode: number;
  /** Declared delivered-control interval, distinct from fixed physics cadence. */
  controlIntervalSeconds: number;
  /** Exact observation width. */
  inputSize: number;
}

/** Chunk starts encode capture, life, generation and missing-data boundaries. */
export type TeacherStartReason = 'captureStart' | 'physicalLifeStart' | 'continuation' | 'gap' | 'generationStart';

/** Ordinary time/size seals preserve recurrent continuity. */
export type TeacherEndReason = 'maximumSamples' | 'simulatedSecond' | 'lifeEnded' | 'generationEnded' | 'contractChanged' | 'captureStopped' | 'interrupted';

/** Rust constructs all descriptors; Node stores/routes their opaque byte objects. */
export interface TeacherChunkDescriptor {
  /** Capture identity shared with its immutable dataset metadata. */
  captureEpoch: string;
  /** Monotonic assigned chunk sequence, including gaps from dropped objects. */
  sequence: DistillationWord;
  /** Authoritative generation for this physical stream fragment. */
  generation: DistillationWord;
  /** Stable teacher slot, distinct from physical identity. */
  baselineSlot: number;
  /** Exact physical snake whose actions/outcomes were recorded. */
  physicalSnakeId: DistillationWord;
  /** First delivered-sample sequence in this capture/baseline stream. */
  firstSample: DistillationWord;
  /** Last delivered-sample sequence in this fragment. */
  lastSample: DistillationWord;
  /** First committed physical step represented by a sample. */
  firstStep: DistillationWord;
  /** Last committed physical step represented by a sample. */
  lastStep: DistillationWord;
  /** Number of exact delivered observations. */
  count: number;
  /** Exact per-row Float32 width. */
  inputSize: number;
  /** Configuration digest bound to the observations. */
  configSha256: string;
  /** Exact sensor identity digest. */
  sensorSha256: string;
  /** Teacher implementation identity digest. */
  teacherSha256: string;
  /** Fragment start semantics. */
  startReason: TeacherStartReason;
  /** Fragment sealing semantics. */
  endReason: TeacherEndReason;
  /** Complete encoded object size, at most one MiB. */
  byteCount: number;
  /** SHA-256 of the complete immutable bytes. */
  sha256: string;
}

/** One coarse independently owned immutable transfer. */
export interface TeacherChunkTransfer {
  /** Rust-produced metadata for the opaque object. */
  descriptor: TeacherChunkDescriptor;
  /** Encoded little-endian data; Node must not parse/rebuild the sample payload. */
  bytes: Uint8Array;
}

/** Coherent native progress; the durable spool settles its own resolvedThrough. */
export interface TeacherCaptureDrain {
  /** Epoch identity, absent before any capture has been admitted. */
  contract?: TeacherCaptureContract | null;
  /** Exact Rust canonical sensor JSON, including its original key/byte ordering. */
  sensorContractJson?: string | null;
  /** Whole immutable objects within the independent drain count/byte limits. */
  chunks: TeacherChunkTransfer[];
  /** Highest assigned committed sequence, including native drops. */
  productionWatermark: DistillationWord;
  /** Native ownership settled through here; this does not prove durable storage. */
  releasedThrough: DistillationWord;
  /** Complete objects still owned by native capture or handoff. */
  queuedChunks: number;
  /** Encoded bytes still retained by native capture/handoff. */
  queuedBytes: DistillationWord;
  /** Whole newly sealed objects dropped before handoff. */
  droppedChunks: DistillationWord;
  /** Exact delivered rows belonging to those dropped objects. */
  droppedSamples: DistillationWord;
  /** Encoded byte count of whole dropped objects. */
  droppedBytes: DistillationWord;
  /** True after an ordered stop, contract change or interruption. */
  stopped: boolean;
  /** Bounded diagnostic capture failure without an authoritative game fault. */
  error?: string | null;
  /** More complete objects remain to transfer. */
  moreWork: boolean;
}
