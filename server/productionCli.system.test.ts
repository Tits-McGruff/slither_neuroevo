import { mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { expect, it } from 'vitest';

/** Repository-owned production entry point and transpiler, independent of the child working directory. */
const ENTRY = resolve('server/rustServer.ts');
/** Local CLI runtime used without npm shell indirection. */
const TSX = resolve('node_modules/tsx/dist/cli.mjs');

it.each([
  { args: ['--help'], status: 0, text: 'Usage: npm run server' },
  { args: ['--db-pth', '/mnt/experiment.db', '--resume', 'latest'], status: 1, text: 'unknown production argument: --db-pth' }
])('exits production CLI before configuration, database or listener startup: $args', ({ args, status, text }) => {
  const root = mkdtempSync(join(tmpdir(), 'slither-production-cli-process-'));
  try {
    const child = spawnSync(process.execPath, [TSX, ENTRY, ...args], {
      cwd: root, encoding: 'utf8', timeout: 8000,
      env: { ...process.env, SERVER_CONFIG: join(root, 'server.toml'), DB_PATH: join(root, 'owner.db') }
    });
    expect(child.error).toBeUndefined();
    expect(child.status, child.stderr).toBe(status);
    expect(child.stdout + child.stderr).toContain(text);
    expect(child.stdout + child.stderr).not.toContain('Rust server:');
    expect(readdirSync(root)).toEqual([]);
  } finally { rmSync(root, { recursive: true, force: true }); }
}, 10_000);