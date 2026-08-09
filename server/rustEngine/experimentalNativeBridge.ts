/**
 * Explicit, non-production adapter for the minimum Rust engine spine.
 *
 * This deliberately exposes only the Stage 3 probe contract.  It is not a
 * second game API and normal server startup does not import this module.
 */

import { computeNativeSourceIdentity, type NativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Native targets admitted by the first supported authoritative-engine builds. */
const SUPPORTED_NATIVE_TARGETS = new Set([
  'x86_64-pc-windows-msvc',
  'x86_64-unknown-linux-gnu'
]);

/** Versioned SHA-256 form exported by the native build contract. */
const BUILD_CONTRACT_SHA256 = /^sha256:[0-9a-f]{64}$/;
/** Raw SHA-256 form exported by the native source identity. */
const SOURCE_SHA256 = /^[0-9a-f]{64}$/;
/** Largest unsigned 64-bit value accepted by the native probe contract. */
const U64_MAX = (1n << 64n) - 1n;
/** Native engine fault-detail ceiling. */
const MAX_NATIVE_FAULT_DETAIL_UTF8_BYTES = 512;
/**
 * A UTF-16 code unit needs at most three UTF-8 bytes, including an unpaired
 * surrogate converted to U+FFFD. This conservative bound avoids an encoding
 * pass or a population-sized copy while guaranteeing the native byte ceiling.
 */
const MAX_FAULT_DETAIL_UTF16_UNITS = Math.floor(MAX_NATIVE_FAULT_DETAIL_UTF8_BYTES / 3);

/** Build instruction shared by all strict-addon handshake failures. */
const BUILD_INSTRUCTION = 'Run `npm --prefix native run build` from the repository root.';

/** Callback scheduler used to keep native wake handling deterministic in tests. */
export interface DrainScheduler {
  /** Arrange one later drain on the Node event loop. */
  schedule(callback: () => void): void;
}

/** Default Node scheduler.  A wake is never handled synchronously on the native callback stack. */
const DEFAULT_DRAIN_SCHEDULER: DrainScheduler = {
  schedule(callback) {
    setImmediate(callback);
  }
};

/** Queue limits required before the native coordinator allocates persistent state. */
export interface ExperimentalEngineInit {
  /** Version of the coarse Rust engine contract. */
  contractVersion: number;
  /** Maximum queued inbound command batches. */
  maxInboundBatches: number;
  /** Maximum commands retained across inbound batches. */
  maxInboundCommands: number;
  /** Maximum payload bytes retained across inbound batches. */
  maxInboundOwnedBytes: number;
  /** Maximum commands in one atomic inbound batch. */
  maxBatchCommands: number;
  /** Maximum payload bytes in one atomic inbound batch. */
  maxBatchOwnedBytes: number;
  /** Maximum normal reliable output events. */
  maxOutputReliable: number;
  /** Maximum payload bytes held by normal reliable output. */
  maxOutputReliableOwnedBytes: number;
  /** Maximum normal discrete output events. */
  maxOutputDiscrete: number;
  /** Maximum payload bytes held by normal discrete output. */
  maxOutputDiscreteOwnedBytes: number;
  /** Maximum bytes retained across all normal output classes. */
  maxOutputTotalOwnedBytes: number;
  /** Maximum bytes retained by one output event. */
  maxOutputEventOwnedBytes: number;
  /** Maximum connections retaining a replaceable frame. */
  maxOutputFrameConnections: number;
}

/** One exact, bounded Stage 3 probe sent across the coarse bridge. */
export interface ExperimentalProbeCommand {
  /** Node-assigned arrival sequence.  This is never narrowed to Number. */
  sequence: bigint;
  /** Caller correlation value.  This is never narrowed to Number. */
  correlationId: bigint;
  /** Bounded opaque bytes echoed by the current Rust spine. */
  payload: Uint8Array;
}

/** One all-or-nothing current-engine probe batch. */
export interface ExperimentalProbeBatch {
  /** Must match the engine's versioned contract. */
  contractVersion: number;
  /** Strictly sequence-ordered probe commands. */
  commands: readonly ExperimentalProbeCommand[];
}

/** Minimal typed output that current Rust probe support can return. */
export type ExperimentalEngineEvent =
  | { kind: 'started' }
  | { kind: 'stopped' }
  | {
      kind: 'probeResult';
      sequence: bigint;
      correlationId: bigint;
      payload: Uint8Array;
    }
  | { kind: 'discrete'; sequence: bigint; payload: Uint8Array }
  | { kind: 'stats'; sequence: bigint; payload: Uint8Array }
  | { kind: 'frame'; sequence: bigint; connectionId: bigint; payload: Uint8Array }
  | { kind: 'fault'; faultCode: string; faultDetail: string };

/** Native fault retained outside normal bounded event capacity. */
export interface ExperimentalEngineFault {
  /** Stable native fault category. */
  code: string;
  /** Bounded diagnostic text. */
  detail: string;
}

/** One bounded drain response from the native coordinator. */
export interface ExperimentalEngineDrain {
  /** Prepared output events in native priority order. */
  events: readonly ExperimentalEngineEvent[];
  /** True when a continuation must drain without waiting for another wake. */
  moreWork: boolean;
  /** Exact output generation observed when the drain re-armed. */
  generation: bigint;
}

/** Small, payload-free native health summary retaining every Stage 3 queue and wake metric. */
export interface ExperimentalEngineHealth {
  /** One-shot coordinator lifecycle state. */
  lifecycle: 'created' | 'running' | 'stopRequested' | 'faulted' | 'stopped';
  /** Current inbound batch depth. */
  inboundBatches: bigint;
  /** Current inbound command depth. */
  inboundCommands: bigint;
  /** Current inbound owned-byte depth. */
  inboundOwnedBytes: bigint;
  /** Highest observed inbound batch depth. */
  inboundHighWaterBatches: bigint;
  /** Highest observed inbound command depth. */
  inboundHighWaterCommands: bigint;
  /** Highest observed inbound owned-byte depth. */
  inboundHighWaterOwnedBytes: bigint;
  /** Exact rejected inbound submission count. */
  inboundRejections: bigint;
  /** Exact accepted inbound batches discarded by the first fault. */
  inboundFaultDiscardedBatches: bigint;
  /** Exact accepted inbound commands discarded by the first fault. */
  inboundFaultDiscardedCommands: bigint;
  /** Exact accepted inbound bytes discarded by the first fault. */
  inboundFaultDiscardedOwnedBytes: bigint;
  /** Exact last accepted command sequence, when one exists. */
  inboundLastAcceptedSequence: bigint | undefined;
  /** Whether the inbound stop request is set. */
  inboundStopRequested: boolean;
  /** Current normal reliable output depth. */
  outputReliable: bigint;
  /** Current reliable output-owned bytes. */
  outputReliableOwnedBytes: bigint;
  /** Current normal discrete output depth. */
  outputDiscrete: bigint;
  /** Current discrete output-owned bytes. */
  outputDiscreteOwnedBytes: bigint;
  /** Whether a replaceable stats payload is currently retained. */
  outputHasStats: boolean;
  /** Current replaceable display-frame count. */
  outputFrames: bigint;
  /** Current normal output owned-byte total. */
  outputOwnedBytes: bigint;
  /** Highest observed normal output count. */
  outputHighWaterCount: bigint;
  /** Highest observed normal output-owned bytes. */
  outputHighWaterOwnedBytes: bigint;
  /** Exact reliable/discrete output overflow attempts. */
  outputPriorityOverflows: bigint;
  /** Exact stats replacement count. */
  outputStatsReplacements: bigint;
  /** Exact frame replacement count. */
  outputFrameReplacements: bigint;
  /** Exact stale stats publication count. */
  outputStaleStats: bigint;
  /** Exact stale frame publication count. */
  outputStaleFrames: bigint;
  /** Exact stats rejection count. */
  outputStatsRejections: bigint;
  /** Exact frame rejection count. */
  outputFrameRejections: bigint;
  /** Exact stats eviction count. */
  outputStatsEvictions: bigint;
  /** Exact frame eviction count. */
  outputFrameEvictions: bigint;
  /** Whether Rust's reserved out-of-capacity fault slot is occupied. */
  outputHasReservedFault: boolean;
  /** Exact processed batch count. */
  processedBatches: bigint;
  /** Exact processed command count. */
  processedCommands: bigint;
  /** Exact output generation. */
  wakeGeneration: bigint;
  /** Exact attempted payload-free wake count. */
  wakeAttempts: bigint;
  /** Exact accepted payload-free wake count. */
  wakeNotifications: bigint;
  /** Exact failed payload-free wake count. */
  wakeFailures: bigint;
  /** Exact wake re-arm race count. */
  wakeRearmRaces: bigint;
  /** Whether Rust currently considers one coalesced wake outstanding. */
  wakePending: boolean;
  /** First native terminal fault code, when present. */
  faultCode: string | undefined;
  /** First native terminal fault detail, when present. */
  faultDetail: string | undefined;
}

/** One native coordinator instance created by the future narrow N-API wrapper. */
export interface ExperimentalEngineNativeHandle {
  /** Start the one coordinator thread exactly once. */
  start(): void;
  /** Submit only the currently implemented coarse probe batch. */
  submitProbeBatch(commands: readonly ExperimentalProbeCommand[]): void;
  /** Drain at most the requested number of prepared events. */
  drainOutputs(maxEvents: number, maxOwnedBytes: number): ExperimentalEngineDrain;
  /** Read small health metadata without copying authoritative game state. */
  health(): ExperimentalEngineHealth;
  /** Request shutdown without waiting on Node's event loop. */
  requestStop(): void;
  /** Resolve only after the coordinator has joined off the Node event loop. */
  join(): Promise<void>;
  /** Record a bounded bridge-side failure so Rust becomes terminal too. */
  reportBridgeFault(detail: string): void;
}

/** Addon exports required by the explicit experimental adapter. */
export interface ExperimentalEngineNativeBinding {
  /** Source SHA embedded by the Rust build script. */
  nativeAddonSourceSha256(): string;
  /** Exact Cargo target triple. */
  nativeAddonBuildTarget(): string;
  /** Cargo build profile. */
  nativeAddonBuildProfile(): string;
  /** Production versus deliberately enabled test-hook build class. */
  nativeAddonBuildClass(): string;
  /** Rust compiler provenance. */
  nativeAddonRustcVersion(): string;
  /** Versioned digest of correctness-relevant build attributes. */
  nativeAddonBuildContractSha256(): string;
  /** Exact Stage 3 engine contract version represented by the native class. */
  experimentalEngineContractVersion(): number;
  /** N-API class constructor with its payload-free coalesced wake callback. */
  ExperimentalRustEngine: new (
    init: ExperimentalEngineInit,
    onWake: () => void
  ) => ExperimentalEngineNativeHandle;
}

/** Callback hooks owned by the thin Node-side experimental adapter. */
export interface ExperimentalNativeBridgeHandlers {
  /** Route one already-prepared output event without reconstructing a world. */
  onEvent(event: ExperimentalEngineEvent): void;
  /** Surface one terminal bridge or native-engine failure. */
  onFault(error: Error): void;
}

/** Creation dependencies intentionally injectable for fake-binding tests. */
export interface CreateExperimentalNativeBridgeOptions {
  /** Loaded addon exports. */
  binding: unknown;
  /** Source identity independently calculated from this stable native checkout. */
  sourceIdentity: NativeSourceIdentity;
  /** Coordinator allocation limits and contract version. */
  init: ExperimentalEngineInit;
  /** Node-owned event and fault routing hooks. */
  handlers: ExperimentalNativeBridgeHandlers;
  /** Maximum prepared events consumed in a single Node turn. */
  maxDrainEvents?: number;
  /** Maximum prepared bytes consumed in a single Node turn. */
  maxDrainOwnedBytes?: number;
  /** Injectable continuation scheduler. */
  scheduler?: DrainScheduler;
}

/** Loader dependencies for a real addon path without coupling normal startup to this module. */
export interface LoadExperimentalNativeBridgeOptions
  extends Omit<CreateExperimentalNativeBridgeOptions, 'binding' | 'sourceIdentity'> {
  /** Directory containing `native/Cargo.toml` for independent source-SHA calculation. */
  nativeManifestDirectory: string;
  /** Explicit addon loader.  The normal server does not call this during Stage 3 foundation work. */
  loadBinding(): unknown | Promise<unknown>;
}

/** Failure that leaves the bridge terminal and prevents any further drain scheduling. */
export class ExperimentalNativeBridgeFault extends Error {
  /** Native fault category when Rust supplied one. */
  readonly nativeCode: string | null;

  /**
   * Create a bridge-terminal failure.
   * @param message - Concise safe diagnostic.
   * @param nativeCode - Optional native fault category.
   */
  constructor(message: string, nativeCode: string | null = null) {
    super(message);
    this.name = 'ExperimentalNativeBridgeFault';
    this.nativeCode = nativeCode;
  }
}

/**
 * Validate identity metadata and the deliberately small native Stage 3 surface.
 * @param candidate - Unknown addon exports supplied by the caller.
 * @param sourceIdentity - Independently calculated current-tree identity.
 * @returns Typed experimental native binding.
 */
export function validateExperimentalEngineBinding(
  candidate: unknown,
  sourceIdentity: NativeSourceIdentity
): ExperimentalEngineNativeBinding {
  if (typeof candidate !== 'object' || candidate === null) {
    throw new TypeError(`Experimental native addon did not export an object. ${BUILD_INSTRUCTION}`);
  }
  const binding = candidate as Partial<ExperimentalEngineNativeBinding>;
  const required = [
    'nativeAddonSourceSha256',
    'nativeAddonBuildTarget',
    'nativeAddonBuildProfile',
    'nativeAddonBuildClass',
    'nativeAddonRustcVersion',
    'nativeAddonBuildContractSha256',
    'experimentalEngineContractVersion',
    'ExperimentalRustEngine'
  ] as const;
  const missing = required.filter(name => typeof binding[name] !== 'function');
  if (missing.length > 0) {
    throw new TypeError(
      `Experimental native addon is missing exports: ${missing.join(', ')}. ${BUILD_INSTRUCTION}`
    );
  }
  const typed = binding as ExperimentalEngineNativeBinding;
  const sourceSha = checkedText(typed.nativeAddonSourceSha256(), 'nativeAddonSourceSha256');
  if (!SOURCE_SHA256.test(sourceSha)) {
    throw new TypeError(`Experimental native addon returned an invalid source SHA. ${BUILD_INSTRUCTION}`);
  }
  if (sourceSha !== sourceIdentity.sha256) {
    throw new Error(
      `Experimental native addon is stale: addon=${sourceSha}, tree=${sourceIdentity.sha256}. ${BUILD_INSTRUCTION}`
    );
  }
  const target = checkedText(typed.nativeAddonBuildTarget(), 'nativeAddonBuildTarget');
  if (!SUPPORTED_NATIVE_TARGETS.has(target)) {
    throw new Error(`Experimental native addon target ${target} is unsupported. ${BUILD_INSTRUCTION}`);
  }
  if (checkedText(typed.nativeAddonBuildProfile(), 'nativeAddonBuildProfile') !== 'release') {
    throw new Error(`Experimental native addon must use the release profile. ${BUILD_INSTRUCTION}`);
  }
  if (checkedText(typed.nativeAddonBuildClass(), 'nativeAddonBuildClass') !== 'production') {
    throw new Error(`Experimental native addon must use the production build class. ${BUILD_INSTRUCTION}`);
  }
  if (!checkedText(typed.nativeAddonRustcVersion(), 'nativeAddonRustcVersion').startsWith('rustc ')) {
    throw new Error(`Experimental native addon returned invalid Rust compiler provenance. ${BUILD_INSTRUCTION}`);
  }
  if (!BUILD_CONTRACT_SHA256.test(
    checkedText(typed.nativeAddonBuildContractSha256(), 'nativeAddonBuildContractSha256')
  )) {
    throw new Error(`Experimental native addon returned an invalid build-contract SHA. ${BUILD_INSTRUCTION}`);
  }
  if (!Number.isSafeInteger(typed.experimentalEngineContractVersion()) || typed.experimentalEngineContractVersion() <= 0) {
    throw new Error(`Experimental native addon returned an unsupported engine contract version. ${BUILD_INSTRUCTION}`);
  }
  return typed;
}

/**
 * Create a strict bridge from preloaded addon exports.  This has no production startup side effect.
 * @param options - Explicit addon, identity, scheduler, and routing dependencies.
 * @returns An unstarted Stage 3 probe bridge.
 */
export function createExperimentalNativeBridge(
  options: CreateExperimentalNativeBridgeOptions
): ExperimentalNativeBridge {
  const binding = validateExperimentalEngineBinding(options.binding, options.sourceIdentity);
  return new ExperimentalNativeBridge(binding, options);
}

/**
 * Calculate source identity, load an explicitly requested addon, then construct the experimental bridge.
 * @param options - Explicit loader and non-production adapter dependencies.
 * @returns An unstarted Stage 3 probe bridge.
 */
export async function loadExperimentalNativeBridge(
  options: LoadExperimentalNativeBridgeOptions
): Promise<ExperimentalNativeBridge> {
  const sourceIdentity = computeNativeSourceIdentity(options.nativeManifestDirectory);
  const binding = await options.loadBinding();
  return createExperimentalNativeBridge({ ...options, binding, sourceIdentity });
}

/**
 * One Node-owned consumer of a native coordinator's bounded output queue.
 *
 * The native payload-free wake only schedules this drain.  It never transports
 * game data, never polls, and duplicate wakes collapse to one pending Node turn.
 */
export class ExperimentalNativeBridge {
  /** Underlying native coordinator after its strict identity/surface handshake. */
  private native!: ExperimentalEngineNativeHandle;
  /** Exact contract version admitted when this native coordinator was constructed. */
  private readonly contractVersion: number;
  /** Maximum commands admitted in one all-or-nothing probe batch. */
  private readonly maxBatchCommands: number;
  /** Maximum output events consumed before yielding back to Node. */
  private readonly maxDrainEvents: number;
  /** Maximum output bytes consumed before yielding back to Node. */
  private readonly maxDrainOwnedBytes: number;
  /** Node event-loop continuation scheduler. */
  private readonly scheduler: DrainScheduler;
  /** Node routing callbacks. */
  private readonly handlers: ExperimentalNativeBridgeHandlers;
  /** Whether one continuation is already scheduled. */
  private drainScheduled = false;
  /** Whether this sole consumer is inside native drain/event routing. */
  private draining = false;
  /** A native wake observed while the sole consumer was draining. */
  private wakeDuringDrain = false;
  /** Terminal failure; no future scheduling or command submission is allowed. */
  private terminalFault: ExperimentalNativeBridgeFault | null = null;
  /** Whether start has completed. */
  private started = false;
  /** Shared asynchronous native stop/join operation. */
  private stopPromise: Promise<void> | null = null;

  /**
   * Construct one bridge and hand native only a payload-free coalesced wake callback.
   * @param binding - Already identity-validated addon exports.
   * @param options - Adapter routing and bounded-drain parameters.
   */
  constructor(
    binding: ExperimentalEngineNativeBinding,
    options: CreateExperimentalNativeBridgeOptions
  ) {
    this.maxDrainEvents = positiveSafeInteger(options.maxDrainEvents ?? 128, 'maxDrainEvents');
    this.maxDrainOwnedBytes = positiveSafeInteger(
      options.maxDrainOwnedBytes ?? options.init.maxOutputTotalOwnedBytes,
      'maxDrainOwnedBytes'
    );
    this.scheduler = options.scheduler ?? DEFAULT_DRAIN_SCHEDULER;
    this.handlers = options.handlers;
    const nativeContractVersion = binding.experimentalEngineContractVersion();
    if (nativeContractVersion !== options.init.contractVersion) {
      throw new Error(
        `Experimental engine contract mismatch: addon=${nativeContractVersion}, requested=${options.init.contractVersion}.`
      );
    }
    this.contractVersion = options.init.contractVersion;
    this.maxBatchCommands = positiveSafeInteger(
      options.init.maxBatchCommands,
      'init.maxBatchCommands'
    );
    const native = new binding.ExperimentalRustEngine(options.init, () => this.scheduleDrain());
    this.native = validateNativeHandle(native);
  }

  /** Start the native coordinator exactly once. */
  start(): void {
    this.assertUsable('start');
    if (this.started) throw new Error('Experimental native bridge has already started.');
    try {
      this.native.start();
      this.started = true;
    } catch (error) {
      throw this.transitionTerminalFault('native start failed', error);
    }
  }

  /**
   * Submit the only currently supported coarse command type without narrowing 64-bit identities.
   * @param batch - Ordered bounded probe batch.
   */
  submitProbeBatch(batch: ExperimentalProbeBatch): void {
    this.assertUsable('submit a probe batch');
    if (!this.started) throw new Error('Experimental native bridge must start before accepting probe batches.');
    validateProbeBatch(batch, this.contractVersion, this.maxBatchCommands);
    try {
      this.native.submitProbeBatch(batch.commands);
    } catch (error) {
      throw this.transitionTerminalFault('native probe submission failed', error);
    }
  }

  /** Return small health metadata without draining or copying world state. */
  health(): ExperimentalEngineHealth {
    try {
      return validateHealth(this.native.health());
    } catch (error) {
      throw this.transitionTerminalFault('native health query failed', error);
    }
  }

  /** Return the terminal fault, if native or the Node event handler has faulted the bridge. */
  get fault(): ExperimentalNativeBridgeFault | null {
    return this.terminalFault;
  }

  /**
   * Stop accepting work and asynchronously wait for the native coordinator to join.
   * @returns One shared join promise for repeated shutdown callers.
   */
  stop(): Promise<void> {
    if (this.stopPromise) return this.stopPromise;
    this.drainScheduled = false;
    this.wakeDuringDrain = false;
    this.stopPromise = Promise.resolve()
      .then(async () => {
        let shutdownFailed = false;
        try {
          this.native.requestStop();
        } catch (error) {
          shutdownFailed = true;
          this.transitionTerminalFault('native stop request failed', error);
        }
        try {
          await this.native.join();
        } catch (error) {
          shutdownFailed = true;
          this.transitionTerminalFault('native coordinator join failed', error);
        }
        if (shutdownFailed) {
          throw this.terminalFault ?? new ExperimentalNativeBridgeFault('native shutdown failed');
        }
      });
    return this.stopPromise;
  }

  /** Schedule a bounded drain only once for any number of native wake callbacks. */
  private scheduleDrain(): void {
    if (this.terminalFault || this.stopPromise) return;
    if (this.draining) {
      this.wakeDuringDrain = true;
      return;
    }
    if (this.drainScheduled) return;
    this.drainScheduled = true;
    this.scheduler.schedule(() => this.drainScheduledCallback());
  }

  /** Run one bounded native drain, then yield/re-arm only when native reports work remains. */
  private drainScheduledCallback(): void {
    this.drainScheduled = false;
    if (this.terminalFault || this.stopPromise) return;
    this.draining = true;
    this.wakeDuringDrain = false;
    try {
      const result = validateDrain(this.native.drainOutputs(this.maxDrainEvents, this.maxDrainOwnedBytes));
      for (const event of result.events) {
        if (event.kind === 'fault') {
          this.recordNativeFault({ code: event.faultCode, detail: event.faultDetail });
          return;
        }
        this.handlers.onEvent(event);
      }
      if (result.moreWork || this.wakeDuringDrain) this.scheduleContinuationAfterDrain();
    } catch (error) {
      this.transitionTerminalFault('native drain or Node event handler failed', error);
    } finally {
      this.draining = false;
    }
  }

  /** Queue one later event-loop turn after the sole consumer leaves native drain. */
  private scheduleContinuationAfterDrain(): void {
    // `scheduleDrain` records wakeDuringDrain when invoked while draining.  We
    // intentionally schedule directly here so a true moreWork result cannot be
    // lost behind that re-entrancy guard.
    if (this.terminalFault || this.stopPromise || this.drainScheduled) return;
    this.drainScheduled = true;
    this.scheduler.schedule(() => this.drainScheduledCallback());
  }

  /** Convert a native return fault into one visible terminal bridge result. */
  private recordNativeFault(fault: ExperimentalEngineFault): void {
    this.transitionTerminalFault(
      'native engine fault',
      fault.detail,
      fault.code,
      false
    );
  }

  /**
   * Fault both Node adapter and native coordinator, retaining the first failure.
   * @param context - Operation that failed.
   * @param cause - Original thrown value, when there is one.
   * @param nativeCode - Native terminal category, when supplied by a drain result.
   * @param notifyNative - Whether this began in Node and must be reported back to Rust.
   * @returns The retained first terminal failure.
   */
  private transitionTerminalFault(
    context: string,
    cause: unknown = undefined,
    nativeCode: string | null = null,
    notifyNative = true
  ): ExperimentalNativeBridgeFault {
    if (!this.terminalFault) {
      const detail = boundedFaultDetail(context, cause);
      this.terminalFault = new ExperimentalNativeBridgeFault(detail, nativeCode);
      this.drainScheduled = false;
      this.wakeDuringDrain = false;
      if (notifyNative) {
        try {
          this.native.reportBridgeFault(detail);
        } catch {
          // The original bridge failure remains the honest terminal result.
        }
      }
      try {
        this.handlers.onFault(this.terminalFault);
      } catch {
        // Fault reporting must not create an unhandled exception on Node's loop.
      }
    }
    return this.terminalFault;
  }

  /** Reject commands once the bridge is terminal or shutdown has started. */
  private assertUsable(operation: string): void {
    if (this.terminalFault) throw this.terminalFault;
    if (this.stopPromise) throw new Error(`Experimental native bridge cannot ${operation} after stop begins.`);
  }
}

/** Check a native text result without accepting objects coerced to strings. */
function checkedText(value: unknown, name: string): string {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new TypeError(`Experimental native addon returned an empty or invalid ${name}. ${BUILD_INSTRUCTION}`);
  }
  return value.trim();
}

/** Validate the returned native instance before its first coordinator call. */
function validateNativeHandle(candidate: unknown): ExperimentalEngineNativeHandle {
  if (typeof candidate !== 'object' || candidate === null) {
    throw new TypeError('Experimental native addon returned no coordinator handle.');
  }
  const handle = candidate as Partial<ExperimentalEngineNativeHandle>;
  const required = ['start', 'submitProbeBatch', 'drainOutputs', 'health', 'requestStop', 'join', 'reportBridgeFault'] as const;
  const missing = required.filter(name => typeof handle[name] !== 'function');
  if (missing.length > 0) {
    throw new TypeError(`Experimental native coordinator is missing methods: ${missing.join(', ')}.`);
  }
  return handle as ExperimentalEngineNativeHandle;
}

/** Reject invalid drain bounds before crossing into native code. */
function positiveSafeInteger(value: number, field: string): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError(`${field} must be a positive safe integer.`);
  }
  return value;
}

/** Validate only the present Stage 3 probe batch rather than inventing future game commands. */
function validateProbeBatch(
  batch: ExperimentalProbeBatch,
  expectedContractVersion: number,
  maxBatchCommands: number
): void {
  if (!Number.isSafeInteger(batch.contractVersion) || batch.contractVersion <= 0) {
    throw new TypeError('probe batch contractVersion must be a positive safe integer.');
  }
  if (batch.contractVersion !== expectedContractVersion) {
    throw new TypeError(
      `probe batch contractVersion ${batch.contractVersion} does not match engine contract ${expectedContractVersion}.`
    );
  }
  if (!Array.isArray(batch.commands) || batch.commands.length === 0) {
    throw new TypeError('probe batch must contain at least one command.');
  }
  if (batch.commands.length > maxBatchCommands) {
    throw new TypeError(
      `probe batch contains ${batch.commands.length} commands; maximum is ${maxBatchCommands}.`
    );
  }
  let previous = -1n;
  for (const command of batch.commands) {
    if (
      typeof command.sequence !== 'bigint' ||
      command.sequence <= previous ||
      command.sequence < 1n ||
      command.sequence > U64_MAX
    ) {
      throw new TypeError(
        'probe batch sequences must be strictly increasing positive unsigned-64-bit bigint values.'
      );
    }
    if (
      typeof command.correlationId !== 'bigint' ||
      command.correlationId < 0n ||
      command.correlationId > U64_MAX
    ) {
      throw new TypeError('probe correlationId must be an unsigned-64-bit bigint value.');
    }
    if (!(command.payload instanceof Uint8Array)) {
      throw new TypeError('probe payload must be a Uint8Array.');
    }
    previous = command.sequence;
  }
}

/** Validate the native drain shape before routing events across Node boundaries. */
function validateDrain(value: unknown): ExperimentalEngineDrain {
  if (typeof value !== 'object' || value === null) throw new TypeError('native drain returned no result object.');
  const result = value as Partial<ExperimentalEngineDrain>;
  if (
    !Array.isArray(result.events) ||
    typeof result.moreWork !== 'boolean' ||
    typeof result.generation !== 'bigint'
  ) {
    throw new TypeError('native drain returned invalid events, moreWork, or generation fields.');
  }
  for (const event of result.events) validateEvent(event);
  return { events: result.events, moreWork: result.moreWork, generation: result.generation };
}

/** Validate the small health shape and exact 64-bit field. */
function validateHealth(value: unknown): ExperimentalEngineHealth {
  if (typeof value !== 'object' || value === null) throw new TypeError('native health returned no result object.');
  const health = value as Partial<ExperimentalEngineHealth>;
  if (!['created', 'running', 'stopRequested', 'stopped', 'faulted'].includes(health.lifecycle ?? '')) {
    throw new TypeError('native health returned an invalid lifecycle.');
  }
  const exactCounters = [
    health.inboundBatches,
    health.inboundCommands,
    health.inboundOwnedBytes,
    health.inboundHighWaterBatches,
    health.inboundHighWaterCommands,
    health.inboundHighWaterOwnedBytes,
    health.inboundRejections,
    health.inboundFaultDiscardedBatches,
    health.inboundFaultDiscardedCommands,
    health.inboundFaultDiscardedOwnedBytes,
    health.outputReliable,
    health.outputReliableOwnedBytes,
    health.outputDiscrete,
    health.outputDiscreteOwnedBytes,
    health.outputFrames,
    health.outputOwnedBytes,
    health.outputHighWaterCount,
    health.outputHighWaterOwnedBytes,
    health.outputPriorityOverflows,
    health.outputStatsReplacements,
    health.outputFrameReplacements,
    health.outputStaleStats,
    health.outputStaleFrames,
    health.outputStatsRejections,
    health.outputFrameRejections,
    health.outputStatsEvictions,
    health.outputFrameEvictions,
    health.processedBatches,
    health.processedCommands,
    health.wakeGeneration,
    health.wakeAttempts,
    health.wakeNotifications,
    health.wakeFailures,
    health.wakeRearmRaces
  ];
  if (!exactCounters.every(value => typeof value === 'bigint')) {
    throw new TypeError('native health must expose exact bigint queue and wake counters.');
  }
  if (health.inboundLastAcceptedSequence !== undefined && typeof health.inboundLastAcceptedSequence !== 'bigint') {
    throw new TypeError('native health returned an invalid last accepted command sequence.');
  }
  if (
    typeof health.inboundStopRequested !== 'boolean' ||
    typeof health.outputHasStats !== 'boolean' ||
    typeof health.outputHasReservedFault !== 'boolean' ||
    typeof health.wakePending !== 'boolean'
  ) throw new TypeError('native health returned invalid boolean queue or wake state.');
  if (
    (health.faultCode !== undefined && typeof health.faultCode !== 'string') ||
    (health.faultDetail !== undefined && typeof health.faultDetail !== 'string')
  ) throw new TypeError('native health returned invalid fault metadata.');
  return health as ExperimentalEngineHealth;
}

/** Validate current probe output rather than treating native objects as trusted arbitrary payloads. */
function validateEvent(value: unknown): asserts value is ExperimentalEngineEvent {
  if (typeof value !== 'object' || value === null) throw new TypeError('native drain event is invalid.');
  const event = value as Partial<ExperimentalEngineEvent>;
  if (event.kind === 'started' || event.kind === 'stopped') return;
  if (
    event.kind === 'probeResult' &&
    typeof event.sequence === 'bigint' &&
    typeof event.correlationId === 'bigint' &&
    event.payload instanceof Uint8Array
  ) return;
  if (
    (event.kind === 'discrete' || event.kind === 'stats') &&
    typeof event.sequence === 'bigint' &&
    event.payload instanceof Uint8Array
  ) return;
  if (
    event.kind === 'frame' &&
    typeof event.sequence === 'bigint' &&
    typeof event.connectionId === 'bigint' &&
    event.payload instanceof Uint8Array
  ) return;
  if (
    event.kind === 'fault' &&
    typeof event.faultCode === 'string' &&
    event.faultCode.length > 0 &&
    typeof event.faultDetail === 'string'
  ) return;
  throw new TypeError('native drain event does not match the current Stage 3 probe contract.');
}

/**
 * Build one bounded fault prefix without concatenating an unbounded thrown message.
 * @param context - Bridge-owned operation label.
 * @param cause - Unknown thrown value whose useful prefix may be retained.
 * @returns Detail whose worst-case UTF-8 encoding is within Rust's 512-byte reserve.
 */
function boundedFaultDetail(context: string, cause: unknown): string {
  const boundedContext = context.slice(0, MAX_FAULT_DETAIL_UTF16_UNITS);
  if (cause === undefined || boundedContext.length === MAX_FAULT_DETAIL_UTF16_UNITS) {
    return boundedContext;
  }
  const separator = ': ';
  const remainingUnits = MAX_FAULT_DETAIL_UTF16_UNITS - boundedContext.length - separator.length;
  if (remainingUnits <= 0) return boundedContext;

  let causeText: string;
  try {
    if (cause instanceof Error) {
      causeText = typeof cause.message === 'string' ? cause.message : 'error without a message';
    } else if (typeof cause === 'string') {
      causeText = cause;
    } else if (typeof cause === 'bigint') {
      causeText = 'bigint thrown value';
    } else if (
      cause === null ||
      typeof cause === 'number' ||
      typeof cause === 'boolean' ||
      cause === undefined
    ) {
      causeText = String(cause);
    } else {
      causeText = 'non-Error thrown value';
    }
  } catch {
    causeText = 'unreadable thrown error';
  }
  return boundedContext + separator + causeText.slice(0, remainingUnits);
}
