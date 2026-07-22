import { parentPort } from 'node:worker_threads';
import type { BrainPoolWorkerRequest } from '../brainPoolProtocol.ts';

/** Parent port used by this deliberately non-acknowledging timeout fixture. */
const port = parentPort;
if (!port) {
  throw new Error('stallBrainPoolWorker requires parentPort');
}

port.on('message', (message: BrainPoolWorkerRequest) => {
  if (message.type !== 'shutdown') return;
  port.removeAllListeners('message');
  port.close();
});
