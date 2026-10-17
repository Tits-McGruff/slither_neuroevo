import { parentPort, workerData } from 'node:worker_threads';
import { DistillationSpoolStore } from './distillationSpoolStore.ts';
import { distillationInteger, parseSpoolLimits, storagePolicyFromLimits, type DistillationSpoolLimits } from './distillationProtocol.ts';
import type { TeacherCaptureDrain } from '../../src/protocol/distillation.ts';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';
import { BrainCandidateStore, parseBrainDescriptor } from './brainCandidateStore.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';
import { validateExperimentalEngineBinding } from './experimentalNativeBridge.ts';
import type { BrainArtifactDescriptor } from '../../src/protocol/brainArtifacts.ts';

/** A candidate validator is loaded only in this filesystem/native diagnostic worker. */
const requireNative = createRequire(import.meta.url);
/** Retain a verified coarse validator; no world/engine instance is constructed. */
let nativeValidator: ((path: string, registerShadow?: boolean) => string) | undefined;
/** Transient candidate files/catalog have independent bounded retention. */
let candidates: BrainCandidateStore | undefined;

/** Require the actual current-source production addon before accepting a candidate. */
function validateCandidate(path: string, registerShadow = false): BrainArtifactDescriptor {
  if (!nativeValidator) {
    const root = resolve(import.meta.dirname, '../../native');
    const loaded: unknown = requireNative(join(root, 'index.js'));
    validateExperimentalEngineBinding(loaded, computeNativeSourceIdentity(root));
    const method = (loaded as Record<string, unknown>)['validateBrainArtifactFile'];
    if (typeof method !== 'function') throw new Error('native brain validator missing; build the native addon');
    nativeValidator = method.bind(loaded) as (path: string, registerShadow?: boolean) => string;
  }
  const result = nativeValidator(path, registerShadow);
  if (typeof result !== 'string' || Buffer.byteLength(result) > 1024 * 1024) throw new Error('native candidate metadata admission');
  return parseBrainDescriptor(JSON.parse(result));
}

/** Lazy candidate admission remains independent of ordinary teacher spool startup. */
function candidateStore(): BrainCandidateStore {
  candidates ??= new BrainCandidateStore(join(store.root, 'candidates'), validateCandidate, store.limits.diskReserveBytes);
  return candidates;
}

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
    case 'storage': return storagePolicyFromLimits(store.limits);
    case 'ingest': return store.ingest(args[0] as TeacherCaptureDrain, args[1] as number);
    case 'dataset': return store.dataset(args[0] as string);
    case 'datasets': return store.datasets(args[0] as string, args[1] as number);
    case 'list': return store.list(args[0] as string, args[1] as string, args[2] as number);
    case 'lease': return store.lease(args[0] as string, args[1] as string[], args[2] as number);
    case 'renew': return store.renewLease(args[0] as string, args[1] as number);
    case 'release': store.releaseLease(args[0] as string); return null;
    case 'download': return store.download(args[0] as string, args[1] as string, args[2] as string);
    case 'interrupt': store.interrupt(args[0] as string); return null;
    case 'candidate-ingest': return candidateStore().ingest(args[0] as string, args[1] as Uint8Array);
    case 'candidate-get': return candidateStore().get(args[0] as string);
    case 'shadow-prepare': return candidateStore().prepareShadow(args[0] as string);
    case 'shadow-record': return candidateStore().recordShadow(args[0]);
    case 'shadow-receipt': return candidateStore().shadowReceipt(args[0] as string, args[1] as string);
    case 'shadow-events': return candidateStore().shadowEvents(args[0] as string, args[1] as number);
    case 'candidate-download': return candidateStore().download(args[0] as string);
    case 'close': candidates?.close(); store.close(); return null;
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
    if ((request['operation'] === 'download' || request['operation'] === 'candidate-download')) {
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
