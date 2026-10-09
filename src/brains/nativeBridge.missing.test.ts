import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

/** Required-native startup failure suite. */
const SUITE = 'Phase 3 required native addon failure';

describe(SUITE, () => {
  it('fails with actionable build instructions when the addon loader is missing', () => {
    const bridgeUrl = pathToFileURL(resolve('src/brains/nativeBridge.ts')).href;
    const missingAddon = resolve('native/definitely-missing-index.js');
    const script = `
      import { prepareInferenceBackend } from ${JSON.stringify(bridgeUrl)};
      try {
        await prepareInferenceBackend('native', ${JSON.stringify(missingAddon)});
        process.exit(0);
      } catch (error) {
        console.error(error instanceof Error ? error.message : String(error));
        process.exit(23);
      }
    `;
    const result = spawnSync(
      process.execPath,
      ['--import', 'tsx/esm', '--input-type=module', '--eval', script],
      { cwd: process.cwd(), encoding: 'utf8' }
    );

    expect(result.status).toBe(23);
    expect(result.stderr).toContain('Native inference backend could not start');
    expect(result.stderr).toContain('npm --prefix native run build');
    expect(result.stderr).toContain('--backend js only for explicit diagnostics');
  });
});
