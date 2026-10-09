import type { SimdKernelStatus } from '../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../src/brains/types.ts';

/** Active backend summary when all executing brains cannot be identified uniformly. */
export type ActiveInferenceBackend = InferenceBackend | 'mixed' | 'unknown';

/** Runtime record describing the inference path that was requested and attached. */
export interface InferenceModeRecord {
  /** Immutable neural math backend requested by server configuration. */
  requestedBackend: InferenceBackend;
  /** Backend attached to the currently executing serial brains or ready worker pool. */
  activeBackend: ActiveInferenceBackend;
  /** Whether multi-threaded inference was requested in server configuration. */
  requestedMt: boolean;
  /** Number of worker threads in a ready active pool. */
  activeWorkerCount: number;
  /** Active pool lifecycle epoch, or null when serial/failed/uninitialized. */
  poolEpoch: number | null;
  /** Active population-weight epoch, or null when serial/failed/uninitialized. */
  weightEpoch: number | null;
  /** Stable key for the active brain graph. */
  graphKey: string;
  /** Number of Float32 parameters in one active population genome. */
  parameterCount: number;
  /** Active authoritative run seed. */
  seed: number;
  /** Current native-addon loader state without triggering a load. */
  nativeAddonStatus: SimdKernelStatus;
  /** Source-derived native-addon build identifier, or null when native is not loaded. */
  nativeAddonBuildIdentifier: string | null;
}
