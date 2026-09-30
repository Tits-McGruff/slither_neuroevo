/** Prepare one isolated release test addon without replacing the production addon. */
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { computeNativeSourceIdentity } from '../server/rustEngine/nativeSourceIdentity.ts';

/** Stable ignored build path used by the full server panic tests. */
const destination = resolve('native/target/ci-panic-hooks.node');
/** Independent current-tree identity required before reusing any test binary. */
const sourceSha = computeNativeSourceIdentity(resolve('native')).sha256;
/** Inspect in a child so Windows releases the old DLL before a replacement copy. */
const checkScript = `
  try {
    const binding = require(process.argv[1]);
    const valid = binding.nativeAddonSourceSha256() === process.argv[2] &&
      binding.nativeAddonBuildClass() === 'test-hooks' &&
      binding.nativeAddonBuildProfile() === 'release' &&
      typeof binding.ExperimentalRunningAuthority.prototype.armCalculationPanicForTest === 'function';
    process.exit(valid ? 0 : 1);
  } catch { process.exit(1); }
`;
/** Whether the existing isolated addon still matches this source and required hook. */
const current = existsSync(destination) && spawnSync(process.execPath,
  ['-e', checkScript, destination, sourceSha], { stdio: 'ignore' }).status === 0;
if (!current) {
  console.info('[tests.panic-addon] building isolated release test hooks');
  const build = spawnSync('cargo', ['build', '--manifest-path', resolve('native/Cargo.toml'),
    '--release', '--features', 'engine-test-hooks'], { stdio: 'inherit' });
  if (build.error) throw build.error;
  if (build.status !== 0) process.exit(build.status ?? 1);
  const library = process.platform === 'win32' ? 'slither_native.dll' : 'libslither_native.so';
  copyFileSync(resolve('native/target/release', library), destination);
  if (spawnSync(process.execPath, ['-e', checkScript, destination, sourceSha],
    { stdio: 'ignore' }).status !== 0) throw new Error('built panic-test addon failed its source/class/hook check');
}
