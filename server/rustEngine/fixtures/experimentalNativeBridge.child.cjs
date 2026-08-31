'use strict';

/**
 * Isolated native-bridge safety fixture.  Keeping callback exceptions out of
 * Vitest proves the N-API wake handler cannot terminate the embedding process.
 */

const mode = process.argv[2];
const loader = process.argv[3];

if (!mode || !loader) {
  throw new Error('Usage: experimentalNativeBridge.child.cjs <mode> <native-loader>');
}

/** Small bounded limits shared with the integration test. */
const init = {
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

/** Load the generated local addon only after arguments have been validated. */
const native = require(loader);

/** Stop and join one engine, retaining errors as a nonzero child failure. */
async function stopAndJoin(engine) {
  engine.requestStop();
  await engine.join();
}

/** Exercise each intentionally isolated process case. */
async function main() {
  if (mode === 'weak-exit') {
    // The callback and engine remain reachable until process termination. A
    // strong TSFN would keep this child alive; the native adapter uses weak.
    globalThis.liveWeakEngine = new native.ExperimentalRustEngine(init, () => {});
    process.stdout.write('WEAK_EXIT\n');
    return;
  }

  if (mode === 'stop-join') {
    const engine = new native.ExperimentalRustEngine(init, () => {});
    engine.start();
    await stopAndJoin(engine);
    process.stdout.write('STOP_JOIN_EXIT\n');
    return;
  }

  if (mode === 'wake-throws') {
    let resolveWakeCallback;
    const wakeCallbackDelivered = new Promise(resolve => {
      resolveWakeCallback = resolve;
    });
    const engine = new native.ExperimentalRustEngine(init, () => {
      resolveWakeCallback();
      throw new Error('intentional child wake callback failure');
    });
    engine.start();
    let callbackTimeout;
    try {
      await Promise.race([
        wakeCallbackDelivered,
        new Promise((_resolve, reject) => {
          callbackTimeout = setTimeout(() => {
            const current = engine.health();
            reject(new Error(
              'Timed out waiting for the JavaScript wake callback; ' +
              `lifecycle=${current.lifecycle}, attempts=${current.wakeAttempts}, ` +
              `notifications=${current.wakeNotifications}, pending=${current.wakePending}.`
            ));
          }, 15_000);
        })
      ]);
    } finally {
      clearTimeout(callbackTimeout);
    }
    const deadline = Date.now() + 5_000;
    let health = engine.health();
    while (health.faultCode !== 'WakeDelivery') {
      if (Date.now() >= deadline) {
        throw new Error(`Timed out waiting for WakeDelivery, received ${health.faultCode || 'none'}.`);
      }
      await new Promise(resolve => setTimeout(resolve, 10));
      health = engine.health();
    }
    if (health.faultCode !== 'WakeDelivery') {
      throw new Error(`Expected WakeDelivery fault, received ${health.faultCode || 'none'}.`);
    }
    let rejected = false;
    try {
      engine.submitProbeBatch([{ sequence: 1n, correlationId: 1n, payload: Uint8Array.of(1) }]);
    } catch {
      rejected = true;
    }
    if (!rejected) throw new Error('Faulted native engine accepted a later probe batch.');
    process.stdout.write(`WAKE_THROW_SURVIVED ${health.faultCode}\nSUBMIT_REJECTED\n`);
    await stopAndJoin(engine);
    process.stdout.write('JOINED\n');
    return;
  }

  throw new Error(`Unknown child mode: ${mode}`);
}

void main().catch(error => {
  process.stderr.write(`${error instanceof Error ? error.stack || error.message : String(error)}\n`);
  process.exitCode = 1;
});
