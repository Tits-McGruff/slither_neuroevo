import { parentPort, workerData } from 'node:worker_threads';

/** Parent port used by this focused client protocol-failure fixture. */
if (!parentPort) throw new Error('checkpointPersistenceInvalidResponseWorker requires parentPort');
/** Non-null parent port after the worker-context assertion. */
const port = parentPort;

/** Test-only worker response behavior selected by the client test fixture. */
const mode = (workerData as { checkpointPersistenceTestMode?: unknown } | null)
  ?.checkpointPersistenceTestMode;

port.on('message', message => {
  if (mode === 'exit') process.exit(3);
  if (mode === 'exit-clean') process.exit(0);
  if (mode === 'mismatched') {
    const request = message as {
      descriptor?: Record<string, unknown> & {
        operationId?: unknown;
        runId?: unknown;
        logicalRootSha256?: unknown;
      };
    };
    const descriptor = {
      ...request.descriptor,
      transitionEpoch: '0000000000000002'
    };
    port.postMessage({
      type: 'managedCheckpointCommitted',
      operationId: request.descriptor?.operationId,
      transitionEpoch: '0000000000000002',
      runId: request.descriptor?.runId,
      checkpointId: request.descriptor?.logicalRootSha256,
      descriptor
    });
    return;
  }
  /** Deliberately invalid response that must fault and terminate the client-owned worker. */
  port.postMessage({ type: 'not-a-checkpoint-persistence-response' });
});
