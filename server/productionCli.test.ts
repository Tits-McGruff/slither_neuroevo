import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { parseProductionCli, PRODUCTION_CLI_HELP } from './productionCli.ts';

/** Task-owned configuration roots, removed after each test. */
const roots: string[] = [];

/** Allocate an absent config path plus owner data that startup must not touch on invalid input. */
function fixture(): { root: string; config: string; database: string } {
  const root = mkdtempSync(join(tmpdir(), 'slither-production-cli-'));
  roots.push(root);
  const database = join(root, 'owner.db');
  writeFileSync(database, 'retained owner data');
  return { root, config: join(root, 'missing.toml'), database };
}

afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

it.each([
  ['--db-pth', '/mnt/experiment.db'],
  ['--resume', 'latest', '--db-pth', '/mnt/experiment.db'],
  ['constructor'],
  ['toString'],
  ['--db-path'],
  ['--db-path', '--fresh'],
  ['--host', ''],
  ['--port', '6174oops'],
  ['--port', '6174', '--port=6175'],
  ['--fresh=true'],
  ['--fresh', '--resume', 'latest'],
  ['--resume', '17'],
  ['--backend', 'js'],
  ['--mt'],
  ['--tick', '30']
].map(args => ({ args })))('rejects the complete invalid production vector before loading config: ', ({ args }) => {
  const { root, config, database } = fixture();
  expect(() => parseProductionCli(args, { SERVER_CONFIG: config, DB_PATH: database })).toThrow();
  expect(readdirSync(root)).toEqual(['owner.db']);
  expect(readFileSync(database, 'utf8')).toBe('retained owner data');
});

it.each([['--help'], ['-h'], ['--help', '--config', '/not-a-real-location/server.toml']].map(args => ({ args })))('handles help without config creation or database work: ', ({ args }) => {
    const { root, config, database } = fixture();
    expect(parseProductionCli(args, { SERVER_CONFIG: config, DB_PATH: database })).toBeNull();
    expect(readdirSync(root)).toEqual(['owner.db']);
    expect(PRODUCTION_CLI_HELP).toContain('--db-path PATH');
    expect(PRODUCTION_CLI_HELP).toContain('--rust-workers N');
    expect(PRODUCTION_CLI_HELP).toContain('500 ms');
  });

it('retains both value syntaxes, paths with spaces, and explicit CLI precedence', () => {
  const { config } = fixture();
  writeFileSync(config, 'port = 5174\nhost = "127.0.0.1"\n');
  const selected = parseProductionCli(['--config', config, '--port=6174', '--host', '192.168.1.25',
    '--db-path', '/owner/experiment with spaces.db', '--resume=latest', '--rust-workers', '6',
    '--input-hold-ms', '500', '--log=warn'], { PORT: '7174', DB_PATH: '/different.db' });
  expect(selected).toMatchObject({ port: 6174, host: '192.168.1.25', dbPath: '/owner/experiment with spaces.db',
    resume: 'latest', rustCalculationWorkers: 6, controllerInputHoldMs: 500, logLevel: 'warn' });
});