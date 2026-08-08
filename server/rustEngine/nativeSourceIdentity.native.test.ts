import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Native crate directory resolved independently from the test working directory. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));

/** Generated napi-rs loader for the repository's freshly built native addon. */
const NATIVE_LOADER = fileURLToPath(new URL('../../native/index.js', import.meta.url));

/** Common build instruction included in missing, incompatible, or stale-addon failures. */
const BUILD_INSTRUCTION = 'Run `npm --prefix native run build` from the repository root.';

/** CommonJS loader scoped to this ESM test module. */
const require = createRequire(import.meta.url);

/** Identity exports required from a Stage 3-capable native addon. */
interface NativeIdentityExports {
  /** Return the source SHA embedded by build.rs. */
  nativeAddonSourceSha256: () => string;
  /** Return the compiled target triple. */
  nativeAddonBuildTarget: () => string;
  /** Return Cargo's build profile. */
  nativeAddonBuildProfile: () => string;
  /** Return the production or test-hook build class. */
  nativeAddonBuildClass: () => string;
  /** Return the compiler version captured during build. */
  nativeAddonRustcVersion: () => string;
  /** Return the effective compiler/codegen contract admitted by Rust state. */
  nativeAddonBuildContractSha256: () => string;
}

/**
 * Load and validate only the native identity exports used by this integration test.
 * @returns Strictly typed native identity functions.
 */
function loadNativeIdentityExports(): NativeIdentityExports {
  let loaded: unknown;
  try {
    loaded = require(NATIVE_LOADER) as unknown;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`Native addon could not be loaded. ${BUILD_INSTRUCTION} Cause: ${reason}`);
  }
  if (typeof loaded !== 'object' || loaded === null) {
    throw new TypeError(`Native addon did not export an object. ${BUILD_INSTRUCTION}`);
  }
  const exports = loaded as Record<string, unknown>;
  for (const name of [
    'nativeAddonSourceSha256',
    'nativeAddonBuildTarget',
    'nativeAddonBuildProfile',
    'nativeAddonBuildClass',
    'nativeAddonRustcVersion',
    'nativeAddonBuildContractSha256'
  ] as const) {
    if (typeof exports[name] !== 'function') {
      throw new TypeError(`Native addon is missing ${name}(). ${BUILD_INSTRUCTION}`);
    }
  }
  return exports as unknown as NativeIdentityExports;
}

describe('native source identity addon handshake', () => {
  it('matches the current tree and exposes complete build provenance', () => {
    const native = loadNativeIdentityExports();
    const embeddedSha = native.nativeAddonSourceSha256();
    const currentSha = computeNativeSourceIdentity(NATIVE_DIRECTORY).sha256;

    expect(embeddedSha).toMatch(/^[0-9a-f]{64}$/);
    expect(native.nativeAddonBuildTarget()).toMatch(
      /^x86_64-(?:pc-windows-msvc|unknown-linux-gnu)$/
    );
    expect(native.nativeAddonBuildProfile()).toBe('release');
    expect(native.nativeAddonBuildClass()).toBe('production');
    expect(native.nativeAddonRustcVersion()).toMatch(/^rustc\s+\S+/);
    expect(native.nativeAddonBuildContractSha256()).toMatch(/^sha256:[0-9a-f]{64}$/);
    if (embeddedSha !== currentSha) {
      throw new Error(
        `Native addon source SHA is stale: addon=${embeddedSha}, tree=${currentSha}. ${BUILD_INSTRUCTION}`
      );
    }
  });
});
