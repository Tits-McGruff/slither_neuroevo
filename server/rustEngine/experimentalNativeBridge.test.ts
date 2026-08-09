import { describe, expect, it } from 'vitest';
import { fileURLToPath } from 'node:url';
import {
  createExperimentalNativeBridge,
  loadExperimentalNativeBridge,
  type DrainScheduler,
  type ExperimentalEngineDrain,
  type ExperimentalEngineEvent,
  type ExperimentalEngineHealth,
  type ExperimentalEngineInit,
  type ExperimentalEngineNativeBinding,
  type ExperimentalEngineNativeHandle,
  type ExperimentalProbeBatch
} from './experimentalNativeBridge.ts';
import { computeNativeSourceIdentity, type NativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Test-suite label for the isolated Stage 3 adapter. */
const SUITE = 'experimental native engine bridge';

/** Native crate root used to prove the explicit loader calculates a real checkout identity. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));

/** Source identity supplied to fake bindings; the loader's real hash work is separately tested. */
const SOURCE_IDENTITY: NativeSourceIdentity = {
  sha256: 'a'.repeat(64),
  fileCount: 8,
  totalCanonicalBytes: 32,
  totalAccountedPathBytes: 64,
  manifest: []
};

/** Minimal bounded init sent to every fake coordinator. */
const INIT: ExperimentalEngineInit = {
  contractVersion: 1,
  maxInboundBatches: 8,
  maxInboundCommands: 32,
  maxInboundOwnedBytes: 4096,
  maxBatchCommands: 8,
  maxBatchOwnedBytes: 1024,
  maxOutputReliable: 64,
  maxOutputReliableOwnedBytes: 8192,
  maxOutputDiscrete: 32,
  maxOutputDiscreteOwnedBytes: 4096,
  maxOutputTotalOwnedBytes: 16384,
  maxOutputEventOwnedBytes: 1024,
  maxOutputFrameConnections: 8
};

/** Deterministic scheduler that exposes each scheduled Node turn to assertions. */
class ManualScheduler implements DrainScheduler {
  /** Continuations waiting to run. */
  readonly callbacks: Array<() => void> = [];

  /** Save one continuation instead of running it recursively. */
  schedule(callback: () => void): void {
    this.callbacks.push(callback);
  }

  /** Run exactly one scheduled Node turn. */
  runOne(): void {
    const callback = this.callbacks.shift();
    if (!callback) throw new Error('No scheduled callback is available.');
    callback();
  }
}

/** Configurable fake native handle that records each cross-boundary operation. */
class FakeHandle implements ExperimentalEngineNativeHandle {
  /** Native wake registered by the adapter factory. */
  wake: (() => void) | null = null;
  /** Pre-programmed bounded drain responses. */
  readonly drains: ExperimentalEngineDrain[] = [];
  /** Batches sent through the exact probe-only API. */
  readonly submitted: ExperimentalProbeBatch[] = [];
  /** Details reported when Node translation/routing fails. */
  readonly reportedFaults: string[] = [];
  /** Number of start calls. */
  startCalls = 0;
  /** Number of drain calls. */
  drainCalls = 0;
  /** Number of stop/join calls. */
  stopCalls = 0;
  /** Number of asynchronous coordinator join calls. */
  joinCalls = 0;
  /** Optional request-stop failure injected by a shutdown test. */
  requestStopError: Error | null = null;
  /** Last bounded requested drain size. */
  lastDrainLimit: number | null = null;
  /** Lifecycle returned by the small health snapshot. */
  healthLifecycle: ExperimentalEngineHealth['lifecycle'] = 'running';

  /** Record coordinator start. */
  start(): void {
    this.startCalls += 1;
  }

  /** Retain the exact batch so tests can prove bigint values were not narrowed. */
  submitProbeBatch(commands: readonly ExperimentalProbeBatch['commands'][number][]): void {
    this.submitted.push({ contractVersion: 1, commands });
  }

  /** Return the next prepared response without manufacturing work. */
  drainOutputs(maxEvents: number, _maxOwnedBytes: number): ExperimentalEngineDrain {
    this.drainCalls += 1;
    this.lastDrainLimit = maxEvents;
    return this.drains.shift() ?? { events: [], moreWork: false, generation: 0n };
  }

  /** Return small exact health metadata. */
  health(): ExperimentalEngineHealth {
    return {
      lifecycle: this.healthLifecycle,
      inboundBatches: 0n,
      inboundCommands: 0n,
      inboundOwnedBytes: 0n,
      inboundHighWaterBatches: 0n,
      inboundHighWaterCommands: 0n,
      inboundHighWaterOwnedBytes: 0n,
      inboundRejections: 0n,
      inboundFaultDiscardedBatches: 0n,
      inboundFaultDiscardedCommands: 0n,
      inboundFaultDiscardedOwnedBytes: 0n,
      inboundLastAcceptedSequence: undefined,
      inboundStopRequested: false,
      outputReliable: 0n,
      outputReliableOwnedBytes: 0n,
      outputDiscrete: 0n,
      outputDiscreteOwnedBytes: 0n,
      outputHasStats: false,
      outputFrames: 0n,
      outputOwnedBytes: 0n,
      outputHighWaterCount: 0n,
      outputHighWaterOwnedBytes: 0n,
      outputPriorityOverflows: 0n,
      outputStatsReplacements: 0n,
      outputFrameReplacements: 0n,
      outputStaleStats: 0n,
      outputStaleFrames: 0n,
      outputStatsRejections: 0n,
      outputFrameRejections: 0n,
      outputStatsEvictions: 0n,
      outputFrameEvictions: 0n,
      outputHasReservedFault: false,
      processedBatches: 1n,
      processedCommands: 2n ** 63n,
      wakeGeneration: 7n,
      wakeAttempts: 2n,
      wakeNotifications: 2n,
      wakeFailures: 0n,
      wakeRearmRaces: 0n,
      wakePending: false,
      faultCode: undefined,
      faultDetail: undefined
    };
  }

  /** Simulate an asynchronous coordinator join. */
  requestStop(): void {
    this.stopCalls += 1;
    if (this.requestStopError) throw this.requestStopError;
  }

  /** Simulate an asynchronous coordinator join. */
  async join(): Promise<void> {
    this.joinCalls += 1;
  }

  /** Retain the first Node-side failure Rust must treat as terminal. */
  reportBridgeFault(detail: string): void {
    this.reportedFaults.push(detail);
  }
}

/** Build a fake addon with complete production provenance and a coordinator factory. */
function createBinding(handle: FakeHandle): ExperimentalEngineNativeBinding {
  return {
    nativeAddonSourceSha256: () => SOURCE_IDENTITY.sha256,
    nativeAddonBuildTarget: () => 'x86_64-pc-windows-msvc',
    nativeAddonBuildProfile: () => 'release',
    nativeAddonBuildClass: () => 'production',
    nativeAddonRustcVersion: () => 'rustc 1.92.0',
    nativeAddonBuildContractSha256: () => `sha256:${'b'.repeat(64)}`,
    experimentalEngineContractVersion: () => 1,
    ExperimentalRustEngine: class {
      constructor(_init: ExperimentalEngineInit, onWake: () => void) {
        handle.wake = onWake;
        return handle;
      }
    } as unknown as ExperimentalEngineNativeBinding['ExperimentalRustEngine']
  };
}

/** Create a bridge and expose its observability arrays to individual tests. */
function createFixture(overrides: Partial<ExperimentalEngineNativeBinding> = {}) {
  const scheduler = new ManualScheduler();
  const handle = new FakeHandle();
  const received: ExperimentalEngineEvent[] = [];
  const faults: Error[] = [];
  const bridge = createExperimentalNativeBridge({
    binding: { ...createBinding(handle), ...overrides },
    sourceIdentity: SOURCE_IDENTITY,
    init: INIT,
    handlers: {
      onEvent: event => received.push(event),
      onFault: error => faults.push(error)
    },
    maxDrainEvents: 2,
    scheduler
  });
  return { bridge, scheduler, handle, received, faults };
}

describe(SUITE, () => {
  it('computes the current native tree identity before accepting an explicitly loaded binding', async () => {
    const handle = new FakeHandle();
    const actualIdentity = computeNativeSourceIdentity(NATIVE_DIRECTORY);
    const binding = {
      ...createBinding(handle),
      nativeAddonSourceSha256: () => actualIdentity.sha256
    };
    const bridge = await loadExperimentalNativeBridge({
      nativeManifestDirectory: NATIVE_DIRECTORY,
      loadBinding: () => binding,
      init: INIT,
      handlers: { onEvent: () => {}, onFault: () => {} }
    });

    expect(bridge.fault).toBeNull();
  });

  it('requires the strict addon surface and current independently computed source SHA', () => {
    const handle = new FakeHandle();
    const missingFactory = { ...createBinding(handle) } as Record<string, unknown>;
    delete missingFactory['ExperimentalRustEngine'];
    expect(() => createExperimentalNativeBridge({
      binding: missingFactory,
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      handlers: { onEvent: () => {}, onFault: () => {} }
    })).toThrow(/missing exports: ExperimentalRustEngine/);

    expect(() => createExperimentalNativeBridge({
      binding: { ...createBinding(handle), nativeAddonSourceSha256: () => 'c'.repeat(64) },
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      handlers: { onEvent: () => {}, onFault: () => {} }
    })).toThrow(/stale/);

    expect(() => createExperimentalNativeBridge({
      binding: { ...createBinding(handle), nativeAddonBuildClass: () => 'test-hooks' },
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      handlers: { onEvent: () => {}, onFault: () => {} }
    })).toThrow(/production build class/);

    expect(() => createExperimentalNativeBridge({
      binding: { ...createBinding(handle), nativeAddonBuildTarget: () => 'aarch64-unknown-linux-gnu' },
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      handlers: { onEvent: () => {}, onFault: () => {} }
    })).toThrow(/unsupported/);
  });

  it('preserves bigint probe and health identities exactly', () => {
    const { bridge, handle } = createFixture();
    bridge.start();
    const batch: ExperimentalProbeBatch = {
      contractVersion: 1,
      commands: [{ sequence: (2n ** 63n) + 9n, correlationId: (2n ** 63n) + 11n, payload: Uint8Array.of(7) }]
    };
    bridge.submitProbeBatch(batch);

    expect(handle.submitted[0]?.commands[0]?.sequence).toBe((2n ** 63n) + 9n);
    expect(handle.submitted[0]?.commands[0]?.correlationId).toBe((2n ** 63n) + 11n);
    expect(bridge.health().processedCommands).toBe(2n ** 63n);
  });

  it('rejects a mismatched probe contract before native without faulting the bridge', () => {
    const { bridge, handle, faults } = createFixture();
    bridge.start();

    expect(() => bridge.submitProbeBatch({
      contractVersion: 2,
      commands: [{ sequence: 1n, correlationId: 1n, payload: Uint8Array.of(1) }]
    })).toThrow(/does not match engine contract 1/);
    expect(handle.submitted).toEqual([]);
    expect(handle.reportedFaults).toEqual([]);
    expect(faults).toEqual([]);
    expect(bridge.fault).toBeNull();
  });

  it('rejects out-of-range unsigned-64-bit probe values before native without faulting', () => {
    const { bridge, handle, faults } = createFixture();
    bridge.start();
    const tooLarge = 1n << 64n;

    expect(() => bridge.submitProbeBatch({
      contractVersion: 1,
      commands: [{ sequence: tooLarge, correlationId: 1n, payload: Uint8Array.of() }]
    })).toThrow(/unsigned-64-bit/);
    expect(() => bridge.submitProbeBatch({
      contractVersion: 1,
      commands: [{ sequence: 1n, correlationId: -1n, payload: Uint8Array.of() }]
    })).toThrow(/unsigned-64-bit/);
    expect(() => bridge.submitProbeBatch({
      contractVersion: 1,
      commands: [{ sequence: 1n, correlationId: tooLarge, payload: Uint8Array.of() }]
    })).toThrow(/unsigned-64-bit/);

    expect(handle.submitted).toEqual([]);
    expect(handle.reportedFaults).toEqual([]);
    expect(faults).toEqual([]);
    expect(bridge.fault).toBeNull();
  });

  it('rejects an oversized sparse batch before reading elements or calling native', () => {
    const { bridge, handle, faults } = createFixture();
    bridge.start();
    let elementReads = 0;
    const commands = new Array<ExperimentalProbeBatch['commands'][number]>(
      INIT.maxBatchCommands + 1
    );
    Object.defineProperty(commands, 0, {
      get() {
        elementReads += 1;
        throw new Error('oversized batch element must not be read');
      }
    });

    expect(() => bridge.submitProbeBatch({ contractVersion: 1, commands })).toThrow(
      /contains 9 commands; maximum is 8/
    );
    expect(elementReads).toBe(0);
    expect(handle.submitted).toEqual([]);
    expect(handle.reportedFaults).toEqual([]);
    expect(faults).toEqual([]);
    expect(bridge.fault).toBeNull();
  });

  it('coalesces duplicate payload-free wakes, preserves event order, and does not poll when empty', () => {
    const { bridge, scheduler, handle, received } = createFixture();
    bridge.start();
    handle.drains.push({
      events: [
        { kind: 'started' },
        { kind: 'probeResult', sequence: 8n, correlationId: 9n, payload: Uint8Array.of(1) }
      ],
      moreWork: false,
      generation: 1n
    });
    handle.wake?.();
    handle.wake?.();
    handle.wake?.();

    expect(scheduler.callbacks).toHaveLength(1);
    scheduler.runOne();
    expect(handle.drainCalls).toBe(1);
    expect(handle.lastDrainLimit).toBe(2);
    expect(received.map(event => event.kind)).toEqual(['started', 'probeResult']);
    expect(scheduler.callbacks).toHaveLength(0);
  });

  it('yields one continuation for moreWork instead of a timer poll or recursive drain', () => {
    const { bridge, scheduler, handle, received } = createFixture();
    bridge.start();
    handle.drains.push(
      { events: [{ kind: 'started' }], moreWork: true, generation: 1n },
      { events: [{ kind: 'stopped' }], moreWork: false, generation: 2n }
    );
    handle.wake?.();

    scheduler.runOne();
    expect(handle.drainCalls).toBe(1);
    expect(scheduler.callbacks).toHaveLength(1);
    expect(received.map(event => event.kind)).toEqual(['started']);
    scheduler.runOne();
    expect(handle.drainCalls).toBe(2);
    expect(scheduler.callbacks).toHaveLength(0);
    expect(received.map(event => event.kind)).toEqual(['started', 'stopped']);
  });

  it('keeps one drain consumer when native wakes while an event is being routed', () => {
    const scheduler = new ManualScheduler();
    const handle = new FakeHandle();
    const bridge = createExperimentalNativeBridge({
      binding: createBinding(handle),
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      scheduler,
      handlers: {
        onEvent: () => handle.wake?.(),
        onFault: () => {}
      }
    });
    bridge.start();
    handle.drains.push(
      { events: [{ kind: 'started' }], moreWork: false, generation: 1n },
      { events: [], moreWork: false, generation: 2n }
    );
    handle.wake?.();

    scheduler.runOne();
    expect(handle.drainCalls).toBe(1);
    expect(scheduler.callbacks).toHaveLength(1);
    scheduler.runOne();
    expect(handle.drainCalls).toBe(2);
  });

  it('faults the bridge and native coordinator when Node event routing throws, then stops further draining', () => {
    const scheduler = new ManualScheduler();
    const handle = new FakeHandle();
    const faults: Error[] = [];
    const bridge = createExperimentalNativeBridge({
      binding: createBinding(handle),
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      scheduler,
      handlers: {
        onEvent: () => { throw new Error('socket routing exploded'); },
        onFault: error => faults.push(error)
      }
    });
    bridge.start();
    handle.drains.push({ events: [{ kind: 'started' }], moreWork: true, generation: 1n });
    handle.wake?.();
    scheduler.runOne();

    expect(bridge.fault?.message).toMatch(/socket routing exploded/);
    expect(handle.reportedFaults).toHaveLength(1);
    expect(faults).toHaveLength(1);
    expect(scheduler.callbacks).toHaveLength(0);
    handle.wake?.();
    expect(scheduler.callbacks).toHaveLength(0);
    expect(bridge.health().processedCommands).toBe(2n ** 63n);
    expect(() => bridge.submitProbeBatch({ contractVersion: 1, commands: [{ sequence: 1n, correlationId: 1n, payload: Uint8Array.of() }] })).toThrow(/socket routing exploded/);
  });

  it('bounds a very long thrown message before reporting the terminal fault to native', () => {
    const scheduler = new ManualScheduler();
    const handle = new FakeHandle();
    const bridge = createExperimentalNativeBridge({
      binding: createBinding(handle),
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      scheduler,
      handlers: {
        onEvent: () => {
          throw new Error(`useful-prefix-${'💥'.repeat(100_000)}`);
        },
        onFault: () => {}
      }
    });
    bridge.start();
    handle.drains.push({ events: [{ kind: 'started' }], moreWork: false, generation: 1n });
    handle.wake?.();

    expect(() => scheduler.runOne()).not.toThrow();
    const reported = handle.reportedFaults[0];
    expect(reported).toBeDefined();
    expect(reported).toMatch(/^native drain or Node event handler failed: useful-prefix-/);
    expect(Buffer.byteLength(reported ?? '', 'utf8')).toBeLessThanOrEqual(512);
    expect(bridge.fault?.message).toBe(reported);
    expect(scheduler.callbacks).toHaveLength(0);
  });

  it('reports a constant description instead of decimal-stringifying a thrown huge bigint', () => {
    const scheduler = new ManualScheduler();
    const handle = new FakeHandle();
    const bridge = createExperimentalNativeBridge({
      binding: createBinding(handle),
      sourceIdentity: SOURCE_IDENTITY,
      init: INIT,
      scheduler,
      handlers: {
        onEvent: () => {
          throw 1n << 1_000_000n;
        },
        onFault: () => {}
      }
    });
    bridge.start();
    handle.drains.push({ events: [{ kind: 'started' }], moreWork: false, generation: 1n });
    handle.wake?.();

    expect(() => scheduler.runOne()).not.toThrow();
    expect(handle.reportedFaults).toEqual([
      'native drain or Node event handler failed: bigint thrown value'
    ]);
    expect(bridge.fault?.message).toBe(handle.reportedFaults[0]);
    expect(scheduler.callbacks).toHaveLength(0);
  });

  it('surfaces a returned native terminal fault before routing later success events', () => {
    const { bridge, scheduler, handle, received, faults } = createFixture();
    bridge.start();
    handle.drains.push({
      events: [{ kind: 'fault', faultCode: 'faulted', faultDetail: 'coordinator panic caught' }],
      moreWork: true,
      generation: 1n
    });
    handle.wake?.();
    scheduler.runOne();

    expect(received).toEqual([]);
    expect(bridge.fault?.nativeCode).toBe('faulted');
    expect(handle.reportedFaults).toHaveLength(0);
    expect(faults[0]?.message).toMatch(/coordinator panic caught/);
  });

  it('stops and joins asynchronously exactly once, disabling later wake scheduling', async () => {
    const { bridge, scheduler, handle } = createFixture();
    bridge.start();
    const first = bridge.stop();
    const second = bridge.stop();
    expect(second).toBe(first);
    await first;
    expect(handle.stopCalls).toBe(1);
    expect(handle.joinCalls).toBe(1);
    handle.healthLifecycle = 'stopped';
    expect(bridge.health().lifecycle).toBe('stopped');
    handle.wake?.();
    expect(scheduler.callbacks).toHaveLength(0);
  });

  it('still joins when requestStop throws and rejects with the retained first fault', async () => {
    const { bridge, handle, faults } = createFixture();
    bridge.start();
    handle.requestStopError = new Error('request stop failed in fake native handle');

    await expect(bridge.stop()).rejects.toThrow(/request stop failed in fake native handle/);
    expect(handle.stopCalls).toBe(1);
    expect(handle.joinCalls).toBe(1);
    expect(handle.reportedFaults).toHaveLength(1);
    expect(faults).toHaveLength(1);
    expect(bridge.fault?.message).toMatch(/request stop failed in fake native handle/);
  });
});
