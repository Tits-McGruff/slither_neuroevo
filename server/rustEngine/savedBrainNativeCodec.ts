import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { parseBrainDescriptor } from './brainCandidateStore.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';
import { validateExperimentalEngineBinding } from './experimentalNativeBridge.ts';
import type { SavedBrainNativeCodec } from './savedBrainCatalog.ts';

/** Native codec signatures are coarse, cold and metadata-only across this bridge. */
interface NativeLibraryBinding {
  /** Full portable source admission; no shadow registration for library saves. */
  validateBrainArtifactFile(path: string): string;
  /** Immutable originals and adaptive weight publication with exact library admission. */
  publishSavedBrainFile(path: string, root: string, operation: string, expected: string,
    available: string, artifactCounted: boolean, weightsCounted: boolean): string;
}

/** Construct lazy worker-local access; no DLL or world is loaded during ordinary metadata startup. */
export function createSavedBrainNativeCodec(): SavedBrainNativeCodec {
  /** Retain only the independently source/build-validated production binding. */
  let binding: NativeLibraryBinding | undefined;
  /** Resolve and validate on first explicit library save, outside the game thread. */
  const load = (): NativeLibraryBinding => {
    if (binding) return binding;
    const root = resolve(import.meta.dirname, '../../native');
    const loaded: unknown = createRequire(import.meta.url)(resolve(root, 'index.js'));
    validateExperimentalEngineBinding(loaded, computeNativeSourceIdentity(root));
    const methods = loaded as Partial<NativeLibraryBinding>;
    if (typeof methods.validateBrainArtifactFile !== 'function' || typeof methods.publishSavedBrainFile !== 'function') {
      throw new Error('native library codec missing; rebuild the native addon');
    }
    binding = methods as NativeLibraryBinding;
    return binding;
  };
  return {
    inspect: path => parseBrainDescriptor(JSON.parse(load().validateBrainArtifactFile(path))),
    publish: (path, root, operation, expected, available, artifactCounted, weightsCounted) => {
      const result = load().publishSavedBrainFile(path, root, operation, expected, available, artifactCounted, weightsCounted);
      if (typeof result !== 'string' || Buffer.byteLength(result) > 1200 * 1024) throw new Error('native library metadata admission');
      return JSON.parse(result) as unknown;
    }
  };
}
