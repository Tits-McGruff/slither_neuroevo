import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { parseProductionCli, productionCliHelp, PRODUCTION_CLI_HELP } from './productionCli.ts';
import { DEFAULT_CONFIG, normalizeConfig } from './config.ts';
import { RUST_CALCULATION_WORKER_MAX, validateRustCalculationWorkers } from './rustWorkers.ts';

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
  ['--input-hold-ms', '750'],
  ['--disconnect-grace-ms', '60000'],
  ['--checkpoint-every', '2'],
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
    '--input-hold-ms', '500', '--disconnect-grace-ms=30000', '--checkpoint-every', '1', '--log=warn'], { PORT: '7174', DB_PATH: '/different.db' }, 16);
  expect(selected).toMatchObject({ port: 6174, host: '192.168.1.25', dbPath: '/owner/experiment with spaces.db',
    resume: 'latest', rustCalculationWorkers: 6, controllerInputHoldMs: 500, controllerDisconnectGraceMs: 30000, checkpointEveryGenerations: 1, logLevel: 'warn' });
});

it.each([1, 2, 8, 16, 32, 64])('uses the supplied %i-CPU manual range across TOML, environment, CLI and help', maximum => {
  const { config } = fixture();
  const warnings = vi.spyOn(console, 'warn').mockImplementation(() => {});
  try {
    for (let count = 1; count <= maximum; count++) {
      expect(validateRustCalculationWorkers(count, maximum)).toBe(count);
      expect(normalizeConfig({ rustCalculationWorkers: count }, undefined, maximum).rustCalculationWorkers).toBe(count);
    }
    for (const count of [0, -1, maximum + 1, 1.5, NaN, Infinity]) {
      expect(() => validateRustCalculationWorkers(count, maximum)).toThrow(`1 to ${maximum}`);
    }
    writeFileSync(config, `rustCalculationWorkers = ${maximum}\n`);
    expect(parseProductionCli([], { SERVER_CONFIG: config }, maximum)?.rustCalculationWorkers).toBe(maximum);
    writeFileSync(config, 'rustCalculationWorkers = 1\n');
    expect(parseProductionCli([], { SERVER_CONFIG: config, RUST_WORKERS: String(maximum) }, maximum)?.rustCalculationWorkers).toBe(maximum);
    expect(parseProductionCli(['--rust-workers', String(maximum)], { SERVER_CONFIG: config, RUST_WORKERS: '1' }, maximum)?.rustCalculationWorkers).toBe(maximum);
    writeFileSync(config, `rustCalculationWorkers = ${maximum + 1}\n`);
    expect(parseProductionCli([], { SERVER_CONFIG: config }, maximum)?.rustCalculationWorkers).toBe(maximum);
    expect(parseProductionCli(['--rust-workers', String(maximum + 1)], { SERVER_CONFIG: config }, maximum)?.rustCalculationWorkers).toBe(maximum);
    expect(warnings.mock.calls.flat().join('\n')).toContain(`1..${maximum}`);
    expect(productionCliHelp(maximum)).toContain(`Range: 1..${maximum} available logical CPUs`);
    expect(normalizeConfig({}, undefined, maximum).rustCalculationWorkers).toBe(Math.min(5, maximum));
    expect(normalizeConfig({ rustCalculationWorkers: 0 }, undefined, maximum).rustCalculationWorkers).toBe(1);
  } finally { warnings.mockRestore(); }
});

it('keeps the five-worker default and the separate Node MT settings', () => {
  expect(DEFAULT_CONFIG.rustCalculationWorkers).toBe(5);
  expect(normalizeConfig({ mtEnabled: true, mtWorkers: 128 }, undefined, 32))
    .toMatchObject({ rustCalculationWorkers: 5, mtEnabled: true, mtWorkers: 128 });
  expect(PRODUCTION_CLI_HELP).toContain(`Range: 1..${RUST_CALCULATION_WORKER_MAX} available logical CPUs`);
});

it('bounds real-server fixture defaults and comparison counts on a two-CPU process', async () => {
  vi.resetModules();
  vi.doMock('./rustWorkers.ts', async () => ({
    ...await vi.importActual<typeof import('./rustWorkers.ts')>('./rustWorkers.ts'),
    RUST_CALCULATION_WORKER_MAX: 2
  }));
  try {
    const fixtureConfig = await import('./test/rustConfig.ts');
    expect(fixtureConfig.RUST_TEST_CONFIG.rustCalculationWorkers).toBe(2);
    expect([1, 4, 5, 6].map(fixtureConfig.rustWorkersForTest)).toEqual([1, 2, 2, 2]);
    expect(DEFAULT_CONFIG.rustCalculationWorkers).toBe(5);
  } finally {
    vi.doUnmock('./rustWorkers.ts');
    vi.resetModules();
  }
});
