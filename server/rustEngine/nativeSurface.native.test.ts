import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { expect, it } from 'vitest';
import { validateExperimentalFreshRunBinding } from './experimentalFreshRunSession.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Source-identified production addon; absence must fail the required-native overlay. */
const NATIVE_DIRECTORY = resolve(import.meta.dirname, '../../native');
/** CommonJS loader for napi-rs generated bindings. */
const require = createRequire(import.meta.url);

it('exposes complete native authority without standalone reference neural-kernel exports', async () => {
  const binding = require(resolve(NATIVE_DIRECTORY, 'index.js')) as Record<string, unknown>;
  const identity = await computeNativeSourceIdentity(NATIVE_DIRECTORY);
  expect(() => validateExperimentalFreshRunBinding(binding, identity)).not.toThrow();
  for (const name of ['denseForwardNative', 'mlpForwardNative', 'gruStepNative', 'lstmStepNative', 'rruStepNative']) {
    expect(Object.hasOwn(binding, name)).toBe(false);
  }
});
