import { execFileSync } from 'node:child_process';

/** Prepare the source-identified test addon once for Vitest's native project. */
export default function setup(): void {
  execFileSync(process.execPath, ['node_modules/tsx/dist/cli.mjs', 'scripts/prepare-panic-test-addon.ts'], {stdio: 'inherit'});
}
