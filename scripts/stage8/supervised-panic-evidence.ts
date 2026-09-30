/** Measure explicit caught-panic service recovery on a dedicated Debian user unit. */
import { execFile, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { readFile, readdir, mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { promisify } from 'node:util';
import Database from 'better-sqlite3';
import WebSocket from 'ws';
import assert from 'node:assert/strict';
import { loadPanicBinding } from '../../server/test/panicRuntime.ts';

/** Required named option without implicit owner-service defaults. */
function option(name: string): string {
  const index = process.argv.indexOf(name);
  const value = process.argv[index + 1];
  if (index < 0 || !value || value.startsWith('--')) throw new Error(`missing ${name}`);
  return value;
}
/** Task-owned disposable unit, never the owner's production unit. */
const unit = option('--unit');
/** Loopback fixture URL with an explicitly disposable port. */
const baseUrl = new URL(option('--url'));
/** Real fixture database inspected independently from its persistence worker. */
const databasePath = resolve(option('--db-path'));
/** Permanent compact report copied out before workspace cleanup. */
const reportPath = resolve(option('--report'));
assert.equal(process.platform, 'linux');
assert.match(unit, /^codex-slither-caught-panic-[a-z0-9]+\.service$/u);
assert.equal(baseUrl.hostname, '127.0.0.1');
assert(baseUrl.port && baseUrl.port !== '5174', 'use an explicit disposable port');
assert.equal(baseUrl.protocol, 'http:');
assert(databasePath.includes('/codex-supervision-'), 'database must be in the task-owned fixture checkout');
/** Independent source/target/profile/class validation before touching the service. */
const binding = loadPanicBinding();

/** Run one bounded supervisor operation with literal arguments. */
function systemctl(...args: string[]): string {
  return execFileSync('systemctl', ['--user', ...args, unit], { encoding: 'utf8', timeout: 30_000 });
}
/** Keep the observer's event loop free to complete WebSocket close handshakes. */
const execute = promisify(execFile);
/** Long lifecycle operations must not block the browser/trainer observer. */
async function serviceCommand(action: 'start' | 'restart' | 'stop'): Promise<void> {
  await execute('systemctl', ['--user', action, unit], { timeout: 30_000 });
}
/** Observe actual supervisor properties, including restart count and process identity. */
function serviceState(): Record<string, string> {
  return Object.fromEntries(systemctl('show', '--property=MainPID,NRestarts,ActiveState,SubState,ExecMainStatus')
    .trim().split('\n').map(line => {
      const separator = line.indexOf('=');
      return [line.slice(0, separator), line.slice(separator + 1)];
    }));
}
/** Bounded retry of real health, retaining the last observation as a failure diagnostic. */
async function healthUntil(predicate: (value: Record<string, unknown>) => boolean): Promise<Record<string, unknown>> {
  const deadline = performance.now() + 10_000;
  let last: unknown;
  do {
    try {
      const response = await fetch(new URL('/api/health', baseUrl), { signal: AbortSignal.timeout(2000) });
      const value = await response.json() as Record<string, unknown>;
      last = { status: response.status, value };
      if (predicate(value)) return { ...value, httpStatus: response.status };
    } catch (error) { last = String(error); }
    await new Promise<void>(done => setTimeout(done, 50));
  } while (performance.now() < deadline);
  throw new Error(`health deadline: ${JSON.stringify(last)}`);
}
/** Capture pointers, rows, file inventory and bytes without trusting the server's report. */
async function durableState(checkpointId: string): Promise<unknown> {
  const database = new Database(databasePath, { readonly: true });
  try {
    const bytes = await readFile(join(`${databasePath}.checkpoints`, `${checkpointId}.checkpoint-v3`));
    return {
      current: database.prepare('SELECT * FROM rust_checkpoint_v3_current ORDER BY run_id').all(),
      checkpoints: database.prepare('SELECT * FROM rust_checkpoint_v3_metadata ORDER BY checkpoint_id').all(),
      files: (await readdir(`${databasePath}.checkpoints`)).sort(),
      checkpointBytes: bytes.length,
      checkpointSha256: createHash('sha256').update(bytes).digest('hex')
    };
  } finally { database.close(); }
}
/** Join an actual UI WebSocket and retain its bounded protocol chronology. */
async function connect(): Promise<{ peer: WebSocket; messages: Array<Record<string, unknown>> }> {
  const peer = new WebSocket(new URL('/', baseUrl).href.replace(/^http:/u, 'ws:'));
  const messages: Array<Record<string, unknown>> = [];
  peer.on('message', (data, binary) => {
    if (!binary) {
      assert(messages.length < 128, 'short fixture must keep bounded message chronology');
      messages.push(JSON.parse(data.toString()) as Record<string, unknown>);
    }
  });
  try {
    await once(peer, 'open', { signal: AbortSignal.timeout(5000) });
    peer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
    peer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
    const joined = once(peer, 'pong', { signal: AbortSignal.timeout(5000) });
    peer.ping();
    await joined;
    return { peer, messages };
  } catch (error) { peer.terminate(); throw error; }
}

/** Live connections released before the disposable unit is stopped. */
let first: Awaited<ReturnType<typeof connect>> | undefined;
/** Reconnected client, proving a real new WebSocket handshake after restart. */
let second: Awaited<ReturnType<typeof connect>> | undefined;
/** Complete accepted evidence, written only after assertions pass and stop is verified. */
let report: Record<string, unknown> | undefined;
try {
  await serviceCommand('start');
  const failed = await healthUntil(value => value['ok'] === false);
  assert.equal(failed['httpStatus'], 503);
  assert.match(String(failed['interfaceFault']), /inference.*partition 1/u);
  assert.equal(failed['completedStep'], '0000000000000000');
  const faultService = serviceState();
  assert.equal(faultService['ActiveState'], 'active');
  assert.equal(faultService['NRestarts'], '0');
  const retained = await durableState(String(failed['startupCheckpointId']));
  first = await connect();
  first.peer.send(JSON.stringify({ type: 'settings', requestId: 'supervised-fault',
    updates: [{ path: 'simSpeed', value: 2 }] }));
  const deadline = performance.now() + 5000;
  while (!first.messages.some(message => message['requestId'] === 'supervised-fault') && performance.now() < deadline) {
    await new Promise<void>(done => setTimeout(done, 10));
  }
  const rejected = first.messages.find(message => message['requestId'] === 'supervised-fault');
  assert.equal(rejected?.['type'], 'settingsApplied');
  assert.equal(rejected?.['applied'], false);
  assert.equal(rejected?.['reason'], failed['interfaceFault']);
  const disconnected = once(first.peer, 'close', { signal: AbortSignal.timeout(10_000) });
  await serviceCommand('restart');
  await disconnected;
  const resumed = await healthUntil(value => value['ok'] === true &&
    BigInt(`0x${String(value['completedStep'])}`) >= 2n);
  for (const key of ['runId', 'startupCheckpointId', 'generation']) assert.equal(resumed[key], failed[key]);
  assert.deepEqual(await durableState(String(resumed['startupCheckpointId'])), retained);
  const restoredService = serviceState();
  assert.equal(restoredService['ActiveState'], 'active');
  assert.notEqual(restoredService['MainPID'], faultService['MainPID']);
  assert.equal(restoredService['NRestarts'], '0');
  second = await connect();
  const welcome = second.messages.find(message => message['type'] === 'welcome');
  assert(welcome, 'reconnected browser must receive an actual welcome');
  assert.equal(welcome['runId'], resumed['runId']);
  report = { measuredAt: new Date().toISOString(), sourceSha256: binding.nativeAddonSourceSha256(),
    target: binding.nativeAddonBuildTarget(), buildClass: binding.nativeAddonBuildClass(),
    buildProfile: binding.nativeAddonBuildProfile(), unit,
    recovery: 'explicit systemctl restart after caught panic; no automatic exit',
    failed, resumed, faultService, restoredService, durable: retained,
    rejectedSettings: rejected, reconnectedWelcome: welcome,
    journal: execFileSync('journalctl', ['--user', '-u', unit, '-n', '80', '--no-pager', '-o', 'cat'],
      { encoding: 'utf8', timeout: 5000 }) };
} finally {
  first?.peer.terminate();
  second?.peer.terminate();
  await serviceCommand('stop');
}
const stoppedService = serviceState();
assert.equal(stoppedService['ActiveState'], 'inactive');
assert.equal(stoppedService['MainPID'], '0');
await mkdir(dirname(reportPath), { recursive: true });
await writeFile(reportPath, `${JSON.stringify({ ...report, stoppedService }, null, 2)}\n`);
console.info(JSON.stringify({ reportPath, stoppedService, passed: true }));
