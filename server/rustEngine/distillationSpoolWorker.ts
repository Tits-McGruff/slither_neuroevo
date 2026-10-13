import { parentPort, workerData } from 'node:worker_threads';
import { DistillationSpoolStore } from './distillationSpoolStore.ts';
import { distillationInteger, parseSpoolLimits, type DistillationSpoolLimits } from './distillationProtocol.ts';
import type { TeacherCaptureDrain } from '../../src/protocol/distillation.ts';

/** A spool worker receives controlled directory/bootstrap limits, never world state. */
interface Bootstrap {
  /** Dedicated diagnostic database/object directory. */
  root: string;
  /** Fully bounded physical and lease ceilings. */
  limits: DistillationSpoolLimits;
}

/** Require the isolated worker context. */
if (!parentPort) throw new Error('distillation spool requires a worker');
/** Validated non-null isolated parent port. */
const port = parentPort;
/** Bootstrap uses only controlled storage paths and immutable admission. */
const bootstrap = workerData as Bootstrap;
/** Single synchronous store confined to this worker. */
const store = new DistillationSpoolStore(bootstrap.root, parseSpoolLimits(bootstrap.limits));
port.postMessage({ kind: 'ready' });

/** Serialize one bounded request and return only metadata or an opaque complete object. */
function dispatch(operation: string, args: unknown[]): unknown {
  switch (operation) {
    case 'ingest': return store.ingest(args[0] as TeacherCaptureDrain);
    case 'dataset': return store.dataset(args[0] as string);
    case 'datasets': return store.datasets(args[0] as string, args[1] as number);
    case 'list': return store.list(args[0] as string, args[1] as string, args[2] as number);
    case 'lease': return store.lease(args[0] as string, args[1] as string[], args[2] as number);
    case 'renew': return store.renewLease(args[0] as string, args[1] as number);
    case 'release': store.releaseLease(args[0] as string); return null;
    case 'download': return store.download(args[0] as string, args[1] as string, args[2] as string);
    case 'interrupt': store.interrupt(args[0] as string); return null;
    case 'close': store.close(); return null;
    default: throw new TypeError('unknown distillation spool operation');
  }
}

port.on('message', (message: unknown) => {
  if (!message || typeof message !== 'object' || Array.isArray(message)) return;
  const request = message as Record<string, unknown>;
  let id: number;
  try {
    id = distillationInteger(request['id'], 1, Number.MAX_SAFE_INTEGER, 'worker request identity');
  } catch { return; }
  try {
    if (Object.keys(request).length !== 3 || typeof request['operation'] !== 'string' ||
        !Array.isArray(request['args']) || request['args'].length > 3) throw new TypeError('invalid bounded spool request');
    const result = dispatch(request['operation'], request['args']);
    if (request['operation'] === 'download') {
      const object = result as { bytes: Uint8Array };
      const bytes = new Uint8Array(object.bytes); // Exclusive transferable backing; no Buffer slab ownership.
      port.postMessage({ kind: 'result', id, ok: true, result: { ...object, bytes } }, [bytes.buffer]);
    } else {
      port.postMessage({ kind: 'result', id, ok: true, result });
    }
    if (request['operation'] === 'close') port.close();
  } catch (error) {
    port.postMessage({ kind: 'result', id, ok: false,
      error: (error instanceof Error ? error.message : String(error)).slice(0, 1024) });
  }
});
