/** Programmatic Protocol 2 player for long-run frames, actions and reconnect evidence. */
import { once } from 'node:events';
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import WebSocket from 'ws';

/** Explicit disposable server, new evidence destination and full run duration. */
const [base, destination, secondsText] = process.argv.slice(2);
/** Wall duration includes recurring disconnect/reclaim attempts. */
const seconds = Number(secondsText);
if (!base || !destination || !Number.isInteger(seconds) || seconds < 1800 || seconds > 7200) {
  throw new Error('usage: soak-player.ts WEBSOCKET_URL NEW_REPORT_PATH SECONDS>=1800');
}
/** Latest real assignment token retained through transient disconnects. */
let resumeToken: string | undefined;
/** Latest assigned public snake ID. */
let snakeId: number | undefined;
/** Most recent authoritative tick advertised by stats/sensors. */
let tick = 0;
/** Current peer, replaced only after a clean deliberate disconnect. */
let peer: WebSocket | undefined;
/** Monotonic full-run observation boundary. */
const started = performance.now();
/** Compact scalar traffic and lifecycle chronology, without frame storage. */
const report = { startedAtUtc: new Date().toISOString(), requestedSeconds: seconds,
  clientScope: 'Programmatic player; measures received frames and reconnects, not browser rendering.',
  connections: 0, frames: 0, frameBytes: 0, actions: 0, assignments: 0,
  reclaimed: 0, reclaimResults: 0, failedReclaims: 0, errors: [] as string[],
  reconnects: [] as Array<{ wallSeconds: number; reclaimed: boolean; snakeId?: number }> };

/** Connect using the real prior token and handle every actual server assignment. */
async function connect(): Promise<void> {
  const requestedToken = resumeToken;
  const requestedSnakeId = snakeId;
  let assigned: { snakeId: number; tokenChanged: boolean } | undefined;
  let reclaimResult: { reclaimed: boolean; snakeId?: number } | undefined;
  const socket = new WebSocket(base!);
  peer = socket;
  snakeId = undefined;
  socket.on('message', (data, binary) => {
    if (binary) { report.frames++; report.frameBytes += data.byteLength; return; }
    const message = JSON.parse(data.toString()) as Record<string, unknown>;
    if (typeof message['tick'] === 'number') tick = message['tick'];
    if (message['type'] === 'assign') {
      snakeId = Number(message['snakeId']);
      resumeToken = String(message['resumeToken']);
      assigned ??= { snakeId, tokenChanged: resumeToken !== requestedToken };
      report.assignments++;
      if (message['reclaimed'] === true) report.reclaimed++;
      report.reconnects.push({ wallSeconds: (performance.now() - started) / 1000,
        reclaimed: message['reclaimed'] === true, snakeId });
    } else if (message['type'] === 'reclaimResult') {
      report.reclaimResults++;
      reclaimResult = { reclaimed: message['reclaimed'] === true,
        ...(typeof message['snakeId'] === 'number' ? { snakeId: message['snakeId'] } : {}) };
      if (!reclaimResult.reclaimed) report.failedReclaims++;
    } else if (message['type'] === 'error') {
      if (report.errors.length >= 16) throw new Error('too many player protocol errors');
      report.errors.push(String(message['message']));
    }
  });
  await once(socket, 'open', { signal: AbortSignal.timeout(5000) });
  report.connections++;
  socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
  socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'Stage7SoakPlayer',
    ...(requestedToken ? { resumeToken: requestedToken } : {}) }));
  const deadline = performance.now() + 5000;
  while ((!assigned || (requestedToken && !reclaimResult)) && socket.readyState === WebSocket.OPEN && performance.now() < deadline) {
    await new Promise<void>(done => setTimeout(done, 10));
  }
  if (!assigned) throw new Error('real player assignment was not delivered');
  if (requestedToken && !reclaimResult) throw new Error('real player reclaim result was not delivered');
  if (reclaimResult?.reclaimed && (!assigned.tokenChanged || assigned.snakeId !== requestedSnakeId ||
      reclaimResult.snakeId !== requestedSnakeId)) throw new Error('reclaim did not preserve the snake and rotate its token');
}

/** Independent action clock: incoming sensor/frame traffic never starts or stops it. */
const sender = setInterval(() => {
  if (peer?.readyState !== WebSocket.OPEN || snakeId === undefined) return;
  peer.send(JSON.stringify({ type: 'action', tick, snakeId, turn: 0.5, boost: 0 }));
  report.actions++;
}, 1000 / 30);
/** Next deliberate reconnect follows the configured thirty-second disconnect grace. */
let nextReconnect = 30;
/** Report a failed observation without changing the live authority. */
let failure: string | undefined;
try {
  await connect();
  while ((performance.now() - started) / 1000 < seconds) {
    await new Promise<void>(done => setTimeout(done, 500));
    if (peer?.readyState !== WebSocket.OPEN) throw new Error('unexpected player disconnect');
    const elapsed = (performance.now() - started) / 1000;
    if (elapsed >= nextReconnect && elapsed < seconds - 5) {
      const closed = once(peer, 'close', { signal: AbortSignal.timeout(5000) });
      peer.close();
      await closed;
      await new Promise<void>(done => setTimeout(done, 100));
      await connect();
      nextReconnect += 30;
    }
  }
} catch (error) { failure = String(error); }
finally { clearInterval(sender); peer?.terminate(); }
const wallSeconds = (performance.now() - started) / 1000;
/** Lifecycle evidence must include actual frames, traffic and successful same-snake reclaim. */
const passed = !failure && wallSeconds >= seconds && report.connections >= 30 &&
  report.frames >= 1000 && report.actions > 1000 && report.reclaimed > 0 &&
  report.reclaimResults === report.connections - 1 && report.errors.length === 0;
await writeFile(resolve(destination), `${JSON.stringify({ ...report, wallSeconds, passed, failure }, null, 2)}\n`, { flag: 'wx' });
console.info(JSON.stringify({ wallSeconds, connections: report.connections, frames: report.frames,
  actions: report.actions, reclaimed: report.reclaimed, passed, failure }));
if (!passed) process.exitCode = 1;
