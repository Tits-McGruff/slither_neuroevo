/** Backend labels retained by the Protocol 2 data contract, not runtime selection. */
export type InferenceBackend = 'js' | 'native';

/** Native addon loading status exposed through Protocol 2. */
export type NativeAddonStatus = 'unavailable' | 'loading' | 'ready' | 'failed';

/** Active backend summary when all executing brains cannot be identified uniformly. */
export type ActiveInferenceBackend = InferenceBackend | 'mixed' | 'unknown';

/** Runtime record describing the inference path that was requested and attached. */
export interface InferenceModeRecord {
  /** Immutable neural math backend requested by server configuration. */
  requestedBackend: InferenceBackend;
  /** Backend attached to the currently executing native authority. */
  activeBackend: ActiveInferenceBackend;
  /** Whether multi-threaded inference was requested in server configuration. */
  requestedMt: boolean;
  /** Number of active native calculation workers; zero denotes serial execution. */
  activeWorkerCount: number;
  /** Reserved Protocol 2 pool epoch; native authority publishes null. */
  poolEpoch: number | null;
  /** Reserved Protocol 2 weight epoch; native authority publishes null. */
  weightEpoch: number | null;
  /** Stable key for the active brain graph. */
  graphKey: string;
  /** Number of Float32 parameters in one active population genome. */
  parameterCount: number;
  /** Active authoritative run seed. */
  seed: number;
  /** Current native-addon loader state without triggering a load. */
  nativeAddonStatus: NativeAddonStatus;
  /** Source-derived native-addon build identifier, or null when native is not loaded. */
  nativeAddonBuildIdentifier: string | null;
}

/** Operational collision-grid measurements that never affect simulation decisions. */
export interface SpatialHashDiagnostics {
  /** Number of entries currently stored. */
  currentEntries: number;
  /** Largest completed entry count observed since construction. */
  peakEntries: number;
  /** Number of entries available without another allocation. */
  capacity: number;
  /** Configured hard admission ceiling. */
  maxCapacity: number;
  /** Estimated bytes represented by the current capacity. */
  estimatedCapacityBytes: number;
  /** Number of complete grid rebuilds. */
  rebuilds: number;
  /** Number of successful capacity increases. */
  growths: number;
  /** Number of entries rejected because their coordinates were outside the grid. */
  outOfBoundsEntries: number;
  /** Last successful admission or growth reason. */
  admissionReason: string;
  /** Last capacity/allocation failure, or null when none has occurred. */
  faultReason: string | null;
}
