import type { TeacherCaptureContract, TeacherChunkDescriptor, DistillationDataset as DatasetWire } from "../../contracts/types.ts";
export type { TeacherCaptureContract, TeacherChunkDescriptor, TeacherStartReason, TeacherEndReason } from "../../contracts/types.ts";
/** Current supported dataset revision. */
export type DistillationDataset = Omit<Required<DatasetWire>, "schemaVersion"> & {schemaVersion: 1};

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

/** Admitted global producer storage policy; byte ceilings use exact hexadecimal u64 words. */
export interface DistillationStoragePolicy {
  /** Additive diagnostic storage-policy schema. */
  schemaVersion: 1;
  /** Quota covers complete objects across all retained datasets. */
  scope: 'producer-global';
  /** Maximum retained encoded object bytes. */
  maxBytes: DistillationWord;
  /** Maximum retained complete objects. */
  maxChunks: number;
  /** Maximum retained dataset manifests. */
  maxDatasets: number;
  /** Maximum simultaneous storage leases. */
  maxLeases: number;
  /** Maximum distinct pinned complete objects. */
  maxPinnedChunks: number;
  /** Maximum distinct pinned encoded bytes. */
  maxPinnedBytes: DistillationWord;
  /** Maximum lease duration or renewal interval. */
  maxLeaseMs: number;
  /** Operating filesystem reserve, outside the encoded-object ceiling. */
  diskReserveBytes: DistillationWord;
}
