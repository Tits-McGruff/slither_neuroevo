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

/** Coherent immutable dataset identity plus current durable dispositions. */
export interface DistillationDataset {
  /** Portable metadata schema. */
  schemaVersion: 1;
  /** Native run seed used by stable stream partitioning; legacy manifests may lack it. */
  runSeed: number | null;
  /** Exact Rust capture identity. */
  contract: TeacherCaptureContract;
  /** Exact Rust canonical bytes represented as UTF-8 JSON. */
  sensorContractJson: string;
  /** Interrupted epochs cannot resume or acquire newly produced tail objects. */
  state: 'active' | 'stopped' | 'interrupted';
  /** Highest durably observed assigned native sequence, including drops. */
  productionWatermark: string;
  /** Highest successfully published immutable object, not a contiguous guarantee. */
  storageWatermark: string;
  /** No retained object has a lower sequence; holes may also exist above it. */
  retentionFloor: string;
  /** All sequences through here have settled stored/unavailable dispositions. */
  resolvedThrough: string;
  /** Absolute native whole-object drop counter last durably observed. */
  nativeDroppedChunks: string;
  /** Absolute native sample drop counter. */
  nativeDroppedSamples: string;
  /** Absolute native encoded-byte drop counter. */
  nativeDroppedBytes: string;
  /** Whole transferred objects refused by spool quota/disk admission. */
  spoolDroppedChunks: string;
  /** Number of complete currently retained objects. */
  retainedChunks: number;
  /** Exact encoded bytes currently retained. */
  retainedBytes: string;
  /** Last bounded native capture diagnostic. */
  error: string | null;
}

/** Listing includes the same snapshot used to select its bounded page. */
export interface DistillationChunkPage {
  /** Coherent durable watermarks, contracts and drop counters. */
  dataset: DistillationDataset;
  /** Available immutable descriptors strictly after the cursor. */
  chunks: TeacherChunkDescriptor[];
  /** Last returned sequence, or the unchanged input cursor for an empty page. */
  nextCursor: string;
  /** Another available descriptor follows; finish pagination before inferring gaps. */
  hasMore: boolean;
}

/** Expiring immutable-object pin, separate from any gameplay controller lease. */
export interface DistillationLease {
  /** Random storage-only lease identifier. */
  id: string;
  /** Dataset whose complete objects are protected. */
  captureEpoch: string;
  /** Absolute Unix millisecond expiry. */
  expiresAt: number;
  /** Exact bounded immutable descriptors protected by this lease. */
  chunks: TeacherChunkDescriptor[];
}

/** Bounded discovery omits large sensor manifests until one dataset is selected. */
export interface DistillationDatasetSummary {
  /** Exact immutable dataset key. */
  captureEpoch: string;
  /** Associated authoritative run. */
  runId: string;
  /** Durable capture closure status. */
  state: DistillationDataset['state'];
  /** Highest durably observed production sequence. */
  productionWatermark: string;
  /** Highest historically stored object. */
  storageWatermark: string;
  /** Contiguous durable stored/unavailable disposition. */
  resolvedThrough: string;
}

/** Strict lowercase identity used exclusively for safe managed paths and keys. */
export function distillationHex(value: unknown, length: 16 | 32 | 64, field: string, nonzero = false): string {
  if (typeof value !== 'string' || value.length !== length || !/^[0-9a-f]+$/u.test(value) ||
      (nonzero && /^0+$/u.test(value))) throw new TypeError(`invalid ${field}`);
  return value;
}

/** Bound integer metadata without converting long sequence identities through Number. */
export function distillationInteger(value: unknown, min: number, max: number, field: string): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min || value > max) {
    throw new TypeError(`invalid ${field}`);
  }
  return value;
}

/** One bounded discovery page, independent of chunk downloads and gameplay traffic. */
export interface DistillationDatasetPage {
  /** Retained dataset identities in canonical order. */
  datasets: DistillationDatasetSummary[];
  /** Exact last returned identity. */
  nextCursor: string;
  /** Another retained dataset follows this page. */
  hasMore: boolean;
}
