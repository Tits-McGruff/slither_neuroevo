/** Bound LAN action-to-step latency through actual client sends and Rust sensor motion. */
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import WebSocket from 'ws';
import type { AssignMsg, SensorsMsg, WelcomeMsg } from '../../server/protocol.ts';
import { observedTurn, summarizeTurnResponseBounds, TurnResponseMarker, type HeadingObservation } from './turn-response-marker.ts';

/** One command and its independently observed return-path upper bound. */
interface Trial {
  /** Current assigned snake identity at send time. */
  snakeId: number;
  /** Prior advertised observation tick carried in the action. */
  clientTick: number;
  /** Deliberate opposite steering command. */
  requestedTurn: -1 | 1;
  /** Monotonic client clock immediately before WebSocket.send. */
  sentAtMs: number;
  /** First authoritative observation showing reversed angular motion. */
  observedTick?: number;
  /** Includes outgoing LAN, Rust control/physics, observation delivery and incoming LAN. */
  latencyUpperBoundMs?: number;
  /** Missing or interrupted responses remain explicit in the report. */
  failure?: string;
}

/** Measure one actual UI or Protocol 2 bot lease without server instrumentation. */
export async function measureTurnResponses(url: string, kind: 'ui' | 'bot', count: number): Promise<{
  clientType: 'ui' | 'bot'; welcome: WelcomeMsg | undefined; trials: Trial[]; failure: string | undefined;
  latencyUpperBoundP95Ms: number | undefined; latencyUpperBoundMaxMs: number | undefined; meetsInputLatencyUpperBound: boolean;
  completedTrials: number; unknownTrials: number;
}> {
  if (!Number.isInteger(count) || count < 1 || count > 1000) throw new Error('trial count must be 1..1000');
  const socket = new WebSocket(url);
  let welcome: WelcomeMsg | undefined;
  let assignment: AssignMsg | undefined;
  let latest: HeadingObservation | undefined;
  let establishedTurn: -1 | 0 | 1 = 0;
  let heldTurn: -1 | 1 = 1;
  let lastBotSentTick: number | undefined;
  let marker: TurnResponseMarker | undefined;
  let result: ReturnType<TurnResponseMarker['observe']>;
  let interrupted = false;
  let failure: string | undefined;
  const trials: Trial[] = [];
  let keepalive: ReturnType<typeof setInterval> | undefined;
  /** Send the current held control independently of incoming sensor callbacks. */
  const send = (): void => {
    if (socket.readyState === WebSocket.OPEN && assignment && latest?.snakeId === assignment.snakeId) {
      if (kind === 'bot' && latest.tick === lastBotSentTick) return;
      socket.send(JSON.stringify({ type: 'action', snakeId: assignment.snakeId,
        tick: latest.tick, turn: heldTurn, boost: 0 }));
      if (kind === 'bot') lastBotSentTick = latest.tick;
    }
  };
  /** Wait on bounded actual socket state and surface asynchronous protocol failures. */
  const until = async (predicate: () => boolean, milliseconds: number): Promise<void> => {
    const deadline = performance.now() + milliseconds;
    while (!predicate()) {
      if (failure) throw new Error(failure);
      if (performance.now() >= deadline) throw new Error('authoritative turn response deadline reached');
      await new Promise<void>(done => setTimeout(done, 5));
    }
    if (failure) throw new Error(failure);
  };
  socket.on('error', error => { failure ??= String(error); });
  socket.on('close', () => { failure ??= 'measurement socket closed'; });
  socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: kind })));
  socket.on('message', (bytes, binary) => {
    if (binary || failure) return;
    try {
      const message = JSON.parse(bytes.toString()) as Record<string, unknown>;
      if (message['type'] === 'welcome') {
        welcome = message as unknown as WelcomeMsg;
        socket.send(JSON.stringify({ type: 'join', mode: 'player', name: `LanTurnProbe-${kind}` }));
      } else if (message['type'] === 'assign') {
        if (marker) { interrupted = true; marker = undefined; }
        assignment = message as unknown as AssignMsg;
        lastBotSentTick = undefined;
        latest = undefined;
        establishedTurn = 0;
      } else if (message['type'] === 'sensors') {
        const sensor = message as unknown as SensorsMsg;
        if (sensor.snakeId !== assignment?.snakeId) return;
        const sine = sensor.sensors[0];
        const cosine = sensor.sensors[1];
        if (!Number.isFinite(sine) || !Number.isFinite(cosine) || Math.hypot(sine!, cosine!) < 0.5) {
          throw new Error('invalid v3 heading pair');
        }
        const sample = { snakeId: sensor.snakeId, tick: sensor.tick,
          direction: Math.atan2(sine!, cosine!), receivedAtMs: performance.now() };
        if (latest && sample.tick <= latest.tick) return;
        establishedTurn = latest ? observedTurn(latest.direction, sample.direction) : 0;
        latest = sample;
        result ??= marker?.observe(sample);
      } else if (message['type'] === 'error') throw new Error(String(message['message']));
    } catch (error) { failure ??= String(error); }
  });
  try {
    await until(() => !!welcome && !!assignment && !!latest, 10_000);
    if (welcome?.sensorSpec.order[0] !== 'heading_sin' || welcome.sensorSpec.order[1] !== 'heading_cos') {
      throw new Error('welcome does not advertise the required v3 heading pair');
    }
    send();
    keepalive = setInterval(send, 100);
    for (let index = 0; index < count; index++) {
      await until(() => !!latest && establishedTurn === heldTurn && performance.now() - latest.receivedAtMs < 100 &&
        (kind === 'ui' || latest.tick !== lastBotSentTick), 2000);
      const baseline = latest!;
      const priorTurn: -1 | 1 = heldTurn;
      heldTurn = priorTurn === 1 ? -1 : 1;
      result = undefined;
      interrupted = false;
      const sentAtMs = performance.now();
      marker = new TurnResponseMarker(baseline, sentAtMs, priorTurn, heldTurn);
      const trial: Trial = { snakeId: baseline.snakeId, clientTick: baseline.tick, requestedTurn: heldTurn, sentAtMs };
      trials.push(trial);
      send();
      try { await until(() => result !== undefined || interrupted, 500); }
      catch (error) {
        trial.failure = `${String(error)}; latency unknown`;
        if (failure) throw error;
      }
      if (interrupted) trial.failure = 'assignment changed before a correlated observation; latency unknown';
      else if (result) Object.assign(trial, result);
      marker = undefined;
      await new Promise<void>(done => setTimeout(done, 100));
    }
  } catch (error) { failure ??= String(error); }
  finally {
    if (keepalive) clearInterval(keepalive);
    socket.removeAllListeners();
    socket.on('error', () => {});
    socket.terminate();
  }
  const summary = summarizeTurnResponseBounds(trials, count);
  return { clientType: kind, welcome, trials, failure, latencyUpperBoundP95Ms: summary.p95Ms,
    latencyUpperBoundMaxMs: summary.maximumKnownMs, completedTrials: summary.completedTrials, unknownTrials: summary.unknownTrials,
    meetsInputLatencyUpperBound: !failure && summary.meetsP95Gate };
}

/** Validate a source-identified production workload and preserve both real client routes. */
async function main(): Promise<void> {
  const [urlText, destination, sourceRevision, scenario, countText = '200'] = process.argv.slice(2);
  const count = Number(countText);
  if (!urlText || !destination || !/^[0-9a-f]{40}$/u.test(sourceRevision ?? '') ||
      !['P0', 'P1', 'P2'].includes(scenario ?? '') || !Number.isInteger(count) || count < 100 || count > 1000) {
    throw new Error('usage: lan-turn-response.ts WS_URL NEW_REPORT SOURCE_COMMIT P0|P1|P2 [TRIALS>=100]');
  }
  const endpoint = new URL(urlText);
  if (!['ws:', 'wss:'].includes(endpoint.protocol)) throw new Error('an explicit WebSocket route is required');
  const healthUrl = new URL('/api/health', endpoint);
  healthUrl.protocol = endpoint.protocol === 'ws:' ? 'http:' : 'https:';
  /** Read identity independently without injecting a timer into the server process. */
  const health = async (): Promise<Record<string, unknown>> => {
    const response = await fetch(healthUrl, { signal: AbortSignal.timeout(15_000) });
    const value = await response.json() as Record<string, unknown>;
    if (!response.ok || value['ok'] !== true) throw new Error(`authority fault: ${String(value['interfaceFault'])}`);
    if (!String(value['nativeBuildIdentifier']).includes(`+${sourceRevision!.slice(0, 12)}.`)) {
      throw new Error('running addon does not identify the requested source');
    }
    return value;
  };
  const initialHealth = await health();
  const startedAtUtc = new Date().toISOString();
  const clients = [];
  for (const kind of ['ui', 'bot'] as const) clients.push(await measureTurnResponses(urlText, kind, count));
  const finalHealth = await health();
  const identityStable = ['nativeBuildIdentifier', 'runId', 'configHash'].every(key => initialHealth[key] === finalHealth[key]);
  const workloadMatches = clients.every(client => {
    const welcome = client.welcome;
    /** Read an advertised authoritative setting without assuming it equals local defaults. */
    const value = (path: string): unknown => welcome?.settings.updates.find(update => update.path === path)?.value;
    return welcome?.settings.core.snakeCount === (scenario === 'P1' ? 300 : 55) &&
      welcome.settings.core.simSpeed === 1 && welcome.sensorSpec.sensorCount === (scenario === 'P2' ? 147 : 83) &&
      welcome.inferenceMode.activeBackend === 'native' &&
      welcome.inferenceMode.activeWorkerCount === initialHealth['calculationWorkers'] &&
      value('generationSeconds') === 60 && value('baselineBots.count') === 10 && value('pelletCountTarget') === 3500 &&
      value('sense.bubbleBins') === (scenario === 'P2' ? 32 : 16) &&
      (scenario !== 'P2' || (welcome.inferenceMode.parameterCount ?? 0) >= 400_000);
  });
  const meetsInputLatencyUpperBound = identityStable && workloadMatches && clients.every(client => client.meetsInputLatencyUpperBound);
  const report = { sourceRevision, scenario, route: urlText, startedAtUtc, clientPlatform: process.platform,
    scope: 'Client send through an authoritative heading reversal and sensor return. This is an upper bound on action-to-step, including both network directions and sensor delivery. Every attempted command enters p95; assignment-interrupted unknown responses rank beyond all finite bounds. Bot sends use each advertised tick at most once, respecting the one-action-per-tick allowance. It does not measure browser event/render cadence, sensor suppression, boost release, or the owner trainer policy.',
    initialHealth, finalHealth, identityStable, workloadMatches, clients, meetsInputLatencyUpperBound };
  await writeFile(resolve(destination), `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  process.stdout.write(`${JSON.stringify({ identityStable, workloadMatches, meetsInputLatencyUpperBound,
    clients: clients.map(client => ({ clientType: client.clientType, samples: client.trials.length,
      latencyUpperBoundP95Ms: client.latencyUpperBoundP95Ms, failure: client.failure })) })}\n`);
  if (!meetsInputLatencyUpperBound) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  void main().catch(error => { process.stderr.write(`${String(error)}\n`); process.exitCode = 1; });
}
