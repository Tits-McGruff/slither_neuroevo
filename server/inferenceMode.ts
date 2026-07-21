import type { SimdKernelStatus } from '../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../src/brains/types.ts';

/** Active backend summary when all executing brains cannot be identified uniformly. */
export type ActiveInferenceBackend = InferenceBackend | 'mixed' | 'unknown';

/** Runtime record describing the inference path that was requested and attached. */
export interface InferenceModeRecord {
  /** Requested neural math backend, or null because the baseline has no selector. */
  requestedBackend: InferenceBackend | null;
  /** Backend attached to the currently executing serial brains or ready worker pool. */
  activeBackend: ActiveInferenceBackend;
  /** Whether multi-threaded inference was requested in server configuration. */
  requestedMt: boolean;
  /** Number of worker threads in a ready active pool. */
  activeWorkerCount: number;
  /** Pool lifecycle epoch, or null because the baseline pool has no epoch. */
  poolEpoch: number | null;
  /** Population-weight epoch, or null because the baseline pool has no epoch. */
  weightEpoch: number | null;
  /** Stable key for the active brain graph. */
  graphKey: string;
  /** Number of Float32 parameters in one active population genome. */
  parameterCount: number;
  /** Seed recorded at server startup; the baseline World does not yet consume it. */
  seed: number;
  /** Current native-addon loader state without triggering a load. */
  nativeAddonStatus: SimdKernelStatus;
  /** Native-addon build identifier, or null because the baseline addon exports none. */
  nativeAddonBuildIdentifier: string | null;
}
