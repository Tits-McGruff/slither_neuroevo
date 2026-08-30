import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  loadExperimentalNativeBridge,
  type ExperimentalEngineInit,
  type ExperimentalEngineNativeBinding,
  type ExperimentalEngineEvent
} from './experimentalNativeBridge.ts';

/** Native crate directory, independently resolved from the Vitest working directory. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));

/** Generated loader for the freshly built local N-API addon. */
const NATIVE_LOADER = fileURLToPath(new URL('../../native/index.js', import.meta.url));

/** Child fixture used where a native callback failure must not terminate Vitest itself. */
const CHILD_FIXTURE = fileURLToPath(
  new URL('./fixtures/experimentalNativeBridge.child.cjs', import.meta.url)
);

/** CommonJS native-loader function scoped to this ESM test module. */
const require = createRequire(import.meta.url);

/** Small valid limits that keep bridge tests bounded while exercising real queue accounting. */
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

/** Load the locally built addon through the generated platform loader. */
function loadNativeBinding(): ExperimentalEngineNativeBinding {
  return require(NATIVE_LOADER) as ExperimentalEngineNativeBinding;
}

/** Yield one event-loop turn without making the production bridge poll on a timer. */
function nextTurn(): Promise<void> {
  return new Promise(resolve => setImmediate(resolve));
}

/** Wait for a bounded observable condition while preserving a hard failure deadline. */
async function waitFor(condition: () => boolean, description: string, timeoutMs = 2_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() >= deadline) throw new Error(`Timed out waiting for ${description}.`);
    await nextTurn();
  }
}

/** Run one safety fixture in a separate Node process and retain its complete diagnostic output. */
function runChild(mode: string, timeoutMs = 8_000): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [CHILD_FIXTURE, mode, NATIVE_LOADER], {
      cwd: NATIVE_DIRECTORY,
      stdio: ['ignore', 'pipe', 'pipe']
    });
    let stdout = '';
    let stderr = '';
    let settled = false;
    const timeout = setTimeout(() => {
      child.kill();
      reject(new Error(`Child ${mode} exceeded ${timeoutMs} ms. stdout=${stdout}\nstderr=${stderr}`));
    }, timeoutMs);
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', data => { stdout += data; });
    child.stderr.on('data', data => { stderr += data; });
    child.once('error', error => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      reject(error);
    });
    child.once('exit', code => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      resolve({ code, stdout, stderr });
    });
  });
}

/** Return the currently implemented coarse N-API roots for an accidental-surface assertion. */
function exportedNativeRoots(binding: ExperimentalEngineNativeBinding): string[] {
  return Object.keys(binding).sort();
}

describe('experimental native bridge real-addon integration', () => {
  it('handshakes the real addon, drains one coarse batch, and preserves exact bigint identities', async () => {
    const received: ExperimentalEngineEvent[] = [];
    const binding = loadNativeBinding();
    const bridge = await loadExperimentalNativeBridge({
      nativeManifestDirectory: NATIVE_DIRECTORY,
      loadBinding: () => binding,
      init: INIT,
      handlers: {
        onEvent: event => received.push(event),
        onFault: error => { throw error; }
      },
      maxDrainEvents: 16,
      maxDrainOwnedBytes: 4096
    });
    try {
      bridge.start();
      bridge.submitProbeBatch({
        contractVersion: 1,
        commands: [
          { sequence: 1n, correlationId: 0n, payload: Uint8Array.of(0, 1) },
          { sequence: (2n ** 53n) + 1n, correlationId: (2n ** 53n) + 1n, payload: Uint8Array.of(2) },
          { sequence: (2n ** 64n) - 1n, correlationId: (2n ** 64n) - 1n, payload: Uint8Array.of(3, 4, 5) }
        ]
      });
      await waitFor(
        () => received.filter(event => event.kind === 'probeResult').length === 3,
        'three exact probe results'
      );
      const probes = received.filter(
        (event): event is Extract<ExperimentalEngineEvent, { kind: 'probeResult' }> => event.kind === 'probeResult'
      );
      expect(probes.map(event => event.sequence)).toEqual([1n, (2n ** 53n) + 1n, (2n ** 64n) - 1n]);
      expect(probes.map(event => event.correlationId)).toEqual([0n, (2n ** 53n) + 1n, (2n ** 64n) - 1n]);
      expect(probes.map(event => [...event.payload])).toEqual([[0, 1], [2], [3, 4, 5]]);

      const health = bridge.health();
      expect(health.processedBatches).toBe(1n);
      expect(health.processedCommands).toBe(3n);
      expect(health.wakeAttempts).toBeGreaterThanOrEqual(1n);
      expect(health.wakeNotifications).toBeLessThanOrEqual(health.wakeAttempts);
      expect(health.wakeFailures).toBe(0n);
    } finally {
      await bridge.stop();
      expect(bridge.health().lifecycle).toBe('stopped');
    }
  });

  it('rejects invalid N-API limits and non-lossless bigint commands before coordinator authority starts', async () => {
    const binding = loadNativeBinding();
    const invalid = { ...INIT, maxInboundBatches: 0 };
    expect(() => new binding.ExperimentalRustEngine(invalid, () => {})).toThrow(/maxInboundBatches/i);
    expect(() => new binding.ExperimentalRustEngine({
      ...INIT,
      maxInboundCommands: 65_537
    }, () => {})).toThrow(/maxInboundCommands.*65(?:,?536)?/i);
    expect(() => new binding.ExperimentalRustEngine({
      ...INIT,
      maxOutputEventOwnedBytes: (64 * 1024 * 1024) + 1
    }, () => {})).toThrow(/maxOutputEventOwnedBytes.*67108864/i);

    const engine = new binding.ExperimentalRustEngine(INIT, () => {});
    try {
      expect(() => engine.submitProbeBatch([
        { sequence: -1n, correlationId: 0n, payload: Uint8Array.of(1) }
      ])).toThrow(/non-negative|unsigned|sequence/i);
      expect(() => engine.submitProbeBatch([
        { sequence: 1n, correlationId: 2n ** 64n, payload: Uint8Array.of(1) }
      ])).toThrow(/lossless|unsigned|correlationId/i);
      expect(() => engine.submitProbeBatch([
        { sequence: 1n, correlationId: 1n << 4096n, payload: Uint8Array.of(1) }
      ])).toThrow(/lossless|unsigned|correlationId/i);
      expect(() => engine.reportBridgeFault('x'.repeat(513))).toThrow(/513.*512|512.*limit/i);
      const health = engine.health();
      expect(health.lifecycle).toBe('created');
      expect(health.faultCode).toBeUndefined();
      expect(health.faultDetail).toBeUndefined();
    } finally {
      engine.requestStop();
      await engine.join();
    }
  });

  it('uses payload-free wake coalescing rather than timer polling and exposes its bounded observability', async () => {
    const received: ExperimentalEngineEvent[] = [];
    const source = await import('./experimentalNativeBridge.ts');
    const sourceText = await import('node:fs/promises').then(fs => fs.readFile(
      fileURLToPath(new URL('./experimentalNativeBridge.ts', import.meta.url)), 'utf8'
    ));
    expect(source.ExperimentalNativeBridge).toBeTypeOf('function');
    expect(sourceText).not.toMatch(/setInterval|setTimeout/);

    const bridge = await loadExperimentalNativeBridge({
      nativeManifestDirectory: NATIVE_DIRECTORY,
      loadBinding: loadNativeBinding,
      init: INIT,
      handlers: { onEvent: event => received.push(event), onFault: error => { throw error; } },
      maxDrainEvents: 1,
      maxDrainOwnedBytes: 1024
    });
    try {
      bridge.start();
      bridge.submitProbeBatch({
        contractVersion: 1,
        commands: [
          { sequence: 1n, correlationId: 1n, payload: Uint8Array.of(1) },
          { sequence: 2n, correlationId: 2n, payload: Uint8Array.of(2) },
          { sequence: 3n, correlationId: 3n, payload: Uint8Array.of(3) }
        ]
      });
      await waitFor(
        () => received.filter(event => event.kind === 'probeResult').length === 3,
        'coalesced bounded drain continuation'
      );
      const health = bridge.health();
      expect(health.wakeAttempts).toBeGreaterThanOrEqual(1n);
      expect(health.wakeNotifications).toBeGreaterThanOrEqual(1n);
      expect(health.wakeNotifications).toBeLessThanOrEqual(health.wakeAttempts);
      expect(health.wakePending).toBe(false);
    } finally {
      await bridge.stop();
    }
  });

  it('has only a coarse experimental surface, never per-snake, layer, or fixed-step controls', () => {
    const binding = loadNativeBinding();
    const roots = exportedNativeRoots(binding);
    expect(roots).toContain('ExperimentalRustEngine');
    expect(roots).toContain('ExperimentalStage6aFreshRunSession');
    expect(roots).toContain('experimentalEngineContractVersion');
    expect(roots.filter(name => /(?:snake|layer|fixed.?step|world.?step|neural.?step)/i.test(name))).toEqual([]);
    expect(Object.getOwnPropertyNames(binding.ExperimentalRustEngine.prototype).sort()).toEqual([
      'constructor',
      'drainOutputs',
      'health',
      'join',
      'reportBridgeFault',
      'requestStop',
      'start',
      'submitProbeBatch'
    ]);
    const freshRunConstructor = (binding as unknown as Record<string, unknown>)[
      'ExperimentalStage6aFreshRunSession'
    ];
    expect(freshRunConstructor).toBeTypeOf('function');
    expect(Object.getOwnPropertyNames(
      (freshRunConstructor as { prototype: object }).prototype
    ).sort()).toEqual([
      'acknowledgeRunStartPersistence',
      'activateRunningAuthority',
      'constructor',
      'initialize',
      'publishFirstScheduledFrameV1',
      'publishInitialFrameV1',
      'publishRunStartCheckpoint',
      'snapshot'
    ]);
  });

  it('contains a throwing JS wake callback, keeps Node alive, faults Rust, rejects later input, and joins', async () => {
    const result = await runChild('wake-throws');
    expect(result.code, result.stderr).toBe(0);
    expect(result.stdout).toContain('WAKE_THROW_SURVIVED');
    expect(result.stdout).toMatch(/WakeDelivery/);
    expect(result.stdout).toContain('SUBMIT_REJECTED');
    expect(result.stdout).toContain('JOINED');
  });

  it('uses a weak TSFN and permits explicit stop/join child processes to exit promptly', async () => {
    const weak = await runChild('weak-exit');
    expect(weak.code, weak.stderr).toBe(0);
    expect(weak.stdout).toContain('WEAK_EXIT');

    const joined = await runChild('stop-join');
    expect(joined.code, joined.stderr).toBe(0);
    expect(joined.stdout).toContain('STOP_JOIN_EXIT');
  });
});
