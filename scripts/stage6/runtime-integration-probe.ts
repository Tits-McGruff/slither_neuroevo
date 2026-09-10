/**
 * Exercise the experimental Rust server through its real HTTP/WebSocket boundary.
 *
 * This is a Protocol 2 wire-compatible diagnostic client, not a substitute for
 * the owner's unchanged trainer or a browser on another LAN device. It prints
 * one bounded scalar report and never writes an evidence file.
 */

import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import WebSocket, { type RawData } from 'ws';
import { PlayerActionPump } from '../../src/net/playerActionPump.ts';

/** Validated command-line controls for one bounded probe. */
export interface ProbeOptions {
  /** Experimental server WebSocket endpoint. */
  wsUrl: string;
  /** Total connected wall duration. */
  durationMs: number;
  /** Observation count before disconnect/token reclaim. */
  reconnectAfterSensors: number;
  /** Require the native generation counter to advance. */
  requireGenerationTransition: boolean;
  /** Duration for which the browser-player socket stops consuming inbound traffic. */
  playerSuppressionMs: number;
  /** Require the slow browser-player socket to cause observable frame replacement. */
  requireFrameReplacement: boolean;
}

/** Small controller counters retained across the reconnect. */
interface ControllerCounters {
  /** Assignment envelopes received. */
  assignments: number;
  /** Sensor observations received. */
  sensors: number;
  /** Observation-driven actions sent. */
  actions: number;
  /** Successful reclaim results received. */
  successfulReclaims: number;
  /** Latest rotating Rust-issued token. */
  resumeToken?: string;
  /** Latest assigned public snake ID. */
  snakeId?: number;
  /** Bounded protocol/server failures. */
  errors: string[];
}

/** Small spectator counters for display delivery. */
interface ViewerCounters {
  /** Binary frame-v1 messages received. */
  frames: number;
  /** JSON stats messages received. */
  stats: number;
  /** Largest observed frame byte length. */
  maximumFrameBytes: number;
  /** Bounded protocol/server failures. */
  errors: string[];
}

/** Browser-player traffic retained while exercising independent latest-value actions. */
interface BrowserPlayerCounters {
  /** Assignment envelopes received. */
  assignments: number;
  /** Sensor observations consumed outside the deliberate inbound pause. */
  sensors: number;
  /** Binary frames consumed outside the deliberate inbound pause. */
  frames: number;
  /** Latest-value player actions sent by the independent pump. */
  actions: number;
  /** Actions sent while all inbound socket consumption was paused. */
  actionsDuringSuppression: number;
  /** Messages unexpectedly delivered after the socket was paused and before resume. */
  inboundDuringSuppression: number;
  /** Largest browser-player frame observed. */
  maximumFrameBytes: number;
  /** Latest sent turn value. */
  latestTurn: number;
  /** Latest sent boost value. */
  latestBoost: number;
  /** Bounded protocol/server failures. */
  errors: string[];
}

/** High-water outbound routing observations sampled from health. */
interface OutboundMaxima {
  /** Health samples included. */
  samples: number;
  /** Largest queued reliable-message count. */
  reliableQueuedMessages: number;
  /** Largest queued reliable-byte count. */
  reliableQueuedBytes: number;
  /** Largest simultaneous pending-frame count. */
  pendingFrames: number;
  /** Largest cumulative replacement count visible on connected sockets. */
  replacedFrames: number;
  /** Largest cumulative reliable-failure count visible on connected sockets. */
  reliableFailures: number;
}

/** Scalar client-visible latency distribution. */
interface LatencySummary {
  /** Completed observations. */
  samples: number;
  /** Arithmetic mean duration. */
  meanMs: number;
  /** Conservative fixed-bucket p95 ceiling. */
  p95Ms: number;
  /** Largest exact duration. */
  maxMs: number;
}

/** Bounded client-side timing owners for the live probe. */
interface ClientTimingOwners {
  /** Socket open to first assignment for fresh and reclaimed joins. */
  controllerLifecycle: FixedLatencyHistogram;
  /** Inter-arrival time between observations on one connection. */
  sensorInterval: FixedLatencyHistogram;
  /** Local observation callback through action send return. */
  sensorToActionDispatch: FixedLatencyHistogram;
  /** Sent action through the next observation on the same connection. */
  actionToNextSensor: FixedLatencyHistogram;
  /** Inter-arrival time between browser display frames. */
  frameInterval: FixedLatencyHistogram;
  /** Complete health request/JSON response latency through Node. */
  healthRequest: FixedLatencyHistogram;
  /** Browser-player latest-action send interval. */
  playerActionInterval: FixedLatencyHistogram;
  /** Socket resume until both a new browser sensor and display frame are consumed. */
  playerInboundRecovery: FixedLatencyHistogram;
}

/** Open controller plus its first assignment/observation boundary. */
interface ControllerConnection {
  /** Real network socket. */
  socket: WebSocket;
  /** Resolves after both assignment and observation arrive. */
  ready: Promise<void>;
}

/** UI-class player connection driven by the production latest-action pump. */
interface BrowserPlayerConnection extends ControllerConnection {
  /** Pause inbound frames, sensors, and lifecycle traffic without stopping actions. */
  beginSuppression(): void;
  /** Resume inbound socket consumption. */
  endSuppression(): void;
  /** Stop timers and restore socket consumption for cleanup. */
  stop(): void;
}

/** Default live duration for a fast diagnostic rather than the complete gate. */
const DEFAULT_DURATION_SECONDS = 30;
/** Default bounded receive pause used by the diagnostic browser player. */
const DEFAULT_PLAYER_SUPPRESSION_SECONDS = 1.5;

/** Inclusive millisecond ceilings for bounded probe-side latency histograms. */
const LATENCY_BUCKETS_MS = [0.1, 0.25, 0.5, 1, 2, 4, 8, 16, 32, 64, 125, 250,
  500, 1_000, 2_000, 4_000, 8_000, 15_000, 30_000, 60_000, Infinity] as const;

/** Fixed-memory client-side latency accumulator. */
class FixedLatencyHistogram {
  /** Counts by inclusive fixed ceiling. */
  private readonly buckets = LATENCY_BUCKETS_MS.map(() => 0);
  /** Completed finite observations. */
  private samples = 0;
  /** Sum used only for the scalar mean. */
  private totalMs = 0;
  /** Largest exact finite observation. */
  private maxMs = 0;

  /** Record one finite non-negative duration. */
  public record(durationMs: number): void {
    if (!Number.isFinite(durationMs) || durationMs < 0) return;
    this.samples++;
    this.totalMs += durationMs;
    this.maxMs = Math.max(this.maxMs, durationMs);
    const found = LATENCY_BUCKETS_MS.findIndex(upper => durationMs <= upper);
    const index = found < 0 ? this.buckets.length - 1 : found;
    this.buckets[index] = this.buckets[index]! + 1;
  }

  /** Project the retained scalars and conservative p95 ceiling. */
  public snapshot(): LatencySummary {
    if (this.samples === 0) return { samples: 0, meanMs: 0, p95Ms: 0, maxMs: 0 };
    const rank = Math.ceil(this.samples * 0.95);
    let cumulative = 0;
    let p95Ms = this.maxMs;
    for (let index = 0; index < this.buckets.length; index++) {
      cumulative += this.buckets[index]!;
      if (cumulative < rank) continue;
      const upper = LATENCY_BUCKETS_MS[index]!;
      p95Ms = Number.isFinite(upper) ? upper : this.maxMs;
      break;
    }
    return { samples: this.samples, meanMs: this.totalMs / this.samples,
      p95Ms, maxMs: this.maxMs };
  }
}

/** Construct every bounded client-side timing owner. */
function createClientTimings(): ClientTimingOwners {
  return {
    controllerLifecycle: new FixedLatencyHistogram(),
    sensorInterval: new FixedLatencyHistogram(),
    sensorToActionDispatch: new FixedLatencyHistogram(),
    actionToNextSensor: new FixedLatencyHistogram(),
    frameInterval: new FixedLatencyHistogram(),
    healthRequest: new FixedLatencyHistogram(),
    playerActionInterval: new FixedLatencyHistogram(),
    playerInboundRecovery: new FixedLatencyHistogram()
  };
}

/** Convert one unknown JSON value into a record or fail at the boundary. */
function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError(`${label} must be a JSON object`);
  }
  return value as Record<string, unknown>;
}

/** Parse one positive finite command-line number. */
function positiveNumber(value: string | undefined, option: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) throw new RangeError(`${option} must be positive`);
  return parsed;
}

/** Parse the intentionally small Stage 6A probe CLI. */
function parseOptions(arguments_: readonly string[]): ProbeOptions {
  let wsUrl = 'ws://127.0.0.1:3000';
  let durationMs = DEFAULT_DURATION_SECONDS * 1_000;
  let reconnectAfterSensors = 5;
  let requireGenerationTransition = false;
  let playerSuppressionMs = DEFAULT_PLAYER_SUPPRESSION_SECONDS * 1_000;
  let requireFrameReplacement = false;
  for (let index = 0; index < arguments_.length; index++) {
    const option = arguments_[index]!;
    if (option === '--require-generation-transition') requireGenerationTransition = true;
    else if (option === '--require-frame-replacement') requireFrameReplacement = true;
    else if (option === '--ws-url') wsUrl = arguments_[++index] ?? '';
    else if (option === '--duration-seconds') durationMs = positiveNumber(arguments_[++index], option) * 1_000;
    else if (option === '--player-suppression-seconds') {
      playerSuppressionMs = positiveNumber(arguments_[++index], option) * 1_000;
    }
    else if (option === '--reconnect-after-sensors') {
      reconnectAfterSensors = positiveNumber(arguments_[++index], option);
      if (!Number.isSafeInteger(reconnectAfterSensors)) throw new RangeError(`${option} must be an integer`);
    } else throw new Error(`unknown option: ${option}`);
  }
  const parsedUrl = new URL(wsUrl);
  if ((parsedUrl.protocol !== 'ws:' && parsedUrl.protocol !== 'wss:') || !parsedUrl.host) {
    throw new TypeError('--ws-url must be an absolute ws:// or wss:// URL');
  }
  if (durationMs < playerSuppressionMs + 1_000) {
    throw new RangeError('--duration-seconds must leave at least one second after player suppression');
  }
  return { wsUrl: parsedUrl.href, durationMs, reconnectAfterSensors,
    requireGenerationTransition, playerSuppressionMs, requireFrameReplacement };
}

/** Derive the experimental scalar health endpoint from the socket endpoint. */
function healthUrl(wsUrl: string): string {
  const url = new URL(wsUrl);
  url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
  url.pathname = '/api/health';
  url.search = '';
  url.hash = '';
  return url.href;
}

/** Fetch and validate one successful Rust-authoritative health response. */
async function fetchHealth(
  url: string,
  timing?: FixedLatencyHistogram
): Promise<Record<string, unknown>> {
  const startedAt = performance.now();
  const response = await fetch(url);
  const body = record(await response.json(), 'health response');
  timing?.record(performance.now() - startedAt);
  if (!response.ok || body['ok'] !== true || body['authority'] !== 'rust') {
    throw new Error(`experimental health failed (${String(response.status)}): ${JSON.stringify(body)}`);
  }
  return body;
}

/** Decode one exact native counter from health. */
function nativeCounter(value: unknown, field: string): bigint {
  if (typeof value !== 'string' || !/^[0-9a-f]{16}$/u.test(value)) {
    throw new TypeError(`health ${field} is not an exact native counter`);
  }
  return BigInt(`0x${value}`);
}

/** Fold one scalar outbound health snapshot into bounded maxima. */
function observeOutbound(health: Record<string, unknown>, maxima: OutboundMaxima): void {
  const outbound = record(health['outbound'], 'health outbound diagnostics');
  maxima.samples++;
  for (const field of [
    'reliableQueuedMessages', 'reliableQueuedBytes', 'pendingFrames',
    'replacedFrames', 'reliableFailures'
  ] as const) {
    const value = outbound[field];
    if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
      throw new TypeError(`health outbound ${field} must be a non-negative integer`);
    }
    maxima[field] = Math.max(maxima[field], value);
  }
}

/** Read one exact Node-side telemetry sample count from health. */
function telemetrySamples(health: Record<string, unknown>, field: string): number {
  const telemetry = record(health['telemetry'], 'health telemetry');
  const latency = record(telemetry[field], `health telemetry ${field}`);
  const samples = latency['samples'];
  if (typeof samples !== 'number' || !Number.isSafeInteger(samples) || samples < 0) {
    throw new TypeError(`health telemetry ${field} samples must be a non-negative integer`);
  }
  return samples;
}

/** Parse one non-binary Protocol 2 message. */
function parseMessage(data: RawData): Record<string, unknown> {
  const bytes = Array.isArray(data)
    ? Buffer.concat(data)
    : data instanceof ArrayBuffer
      ? Buffer.from(data)
      : data;
  return record(JSON.parse(bytes.toString('utf8')) as unknown, 'Protocol 2 message');
}

/** Return the exact byte count for every ws raw-data representation. */
function rawByteLength(data: RawData): number {
  if (Array.isArray(data)) return data.reduce((total, part) => total + part.byteLength, 0);
  return data.byteLength;
}

/** Resolve only once when assignment and a sensor observation are both available. */
function createController(
  options: ProbeOptions,
  counters: ControllerCounters,
  timings: ClientTimingOwners,
  resumeToken?: string
): ControllerConnection {
  const socket = new WebSocket(options.wsUrl);
  const openedAt = performance.now();
  const startingSensors = counters.sensors;
  const startingAssignments = counters.assignments;
  let assignmentTimed = false;
  let lastSensorAt: number | undefined;
  let lastActionAt: number | undefined;
  let settled = false;
  let resolveReady!: () => void;
  let rejectReady!: (error: Error) => void;
  const ready = new Promise<void>((resolvePromise, rejectPromise) => {
    resolveReady = resolvePromise;
    rejectReady = rejectPromise;
  });
  const maybeReady = (): void => {
    if (!settled && counters.assignments > startingAssignments && counters.sensors > startingSensors) {
      settled = true;
      resolveReady();
    }
  };
  socket.on('open', () => {
    socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'bot' }));
    socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'stage6-runtime-probe',
      ...(resumeToken ? { resumeToken } : {}) }));
  });
  socket.on('error', error => {
    if (counters.errors.length < 16) counters.errors.push(error.message);
    if (!settled) { settled = true; rejectReady(error); }
  });
  socket.on('close', () => {
    if (!settled) { settled = true; rejectReady(new Error('controller socket closed before assignment')); }
  });
  socket.on('message', (data, binary) => {
    if (binary) return;
    try {
      const message = parseMessage(data);
      if (message['type'] === 'error') {
        const error = new Error(String(message['message'] ?? 'server error'));
        if (counters.errors.length < 16) counters.errors.push(error.message);
        if (!settled) { settled = true; rejectReady(error); }
      } else if (message['type'] === 'assign') {
        if (typeof message['snakeId'] !== 'number' || typeof message['resumeToken'] !== 'string') {
          throw new TypeError('invalid assignment envelope');
        }
        counters.assignments++;
        counters.snakeId = message['snakeId'];
        counters.resumeToken = message['resumeToken'];
        if (!assignmentTimed) {
          assignmentTimed = true;
          timings.controllerLifecycle.record(performance.now() - openedAt);
        }
        maybeReady();
      } else if (message['type'] === 'reclaimResult' && message['reclaimed'] === true) {
        counters.successfulReclaims++;
      } else if (message['type'] === 'sensors') {
        if (typeof message['tick'] !== 'number' || typeof message['snakeId'] !== 'number') {
          throw new TypeError('invalid sensor envelope');
        }
        const sensorAt = performance.now();
        if (lastSensorAt !== undefined) timings.sensorInterval.record(sensorAt - lastSensorAt);
        lastSensorAt = sensorAt;
        if (lastActionAt !== undefined) {
          timings.actionToNextSensor.record(sensorAt - lastActionAt);
          lastActionAt = undefined;
        }
        counters.sensors++;
        counters.snakeId = message['snakeId'];
        const phase = counters.sensors;
        const dispatchStarted = performance.now();
        socket.send(JSON.stringify({ type: 'action', tick: message['tick'], snakeId: message['snakeId'],
          turn: Math.sin(phase * 0.17), boost: phase % 20 < 5 ? 1 : 0 }));
        lastActionAt = performance.now();
        timings.sensorToActionDispatch.record(lastActionAt - dispatchStarted);
        counters.actions++;
        maybeReady();
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (counters.errors.length < 16) counters.errors.push(message);
      if (!settled) { settled = true; rejectReady(new Error(message)); }
    }
  });
  return { socket, ready };
}

/** Open one UI-class player whose actions do not depend on inbound callbacks. */
function createBrowserPlayer(
  options: ProbeOptions,
  counters: BrowserPlayerCounters,
  timings: ClientTimingOwners
): BrowserPlayerConnection {
  const socket = new WebSocket(options.wsUrl);
  const openedAt = performance.now();
  let snakeId: number | undefined;
  let latestTick: number | undefined;
  let assigned = false;
  let connected = false;
  let suppressing = false;
  let paused = false;
  let turn = 1;
  let boost = 1;
  let lastActionAt: number | undefined;
  let settled = false;
  let resolveReady!: () => void;
  let rejectReady!: (error: Error) => void;
  const ready = new Promise<void>((resolvePromise, rejectPromise) => {
    resolveReady = resolvePromise;
    rejectReady = rejectPromise;
  });
  const maybeReady = (): void => {
    if (!settled && assigned && latestTick !== undefined && counters.frames > 0) {
      settled = true;
      resolveReady();
    }
  };
  const fail = (error: Error): void => {
    if (counters.errors.length < 16) counters.errors.push(error.message);
    if (!settled) { settled = true; rejectReady(error); }
  };
  const pump = new PlayerActionPump({
    cadenceHz: 60,
    isActive: () => connected && assigned && socket.readyState === WebSocket.OPEN,
    buildLatestAction: () => snakeId === undefined || latestTick === undefined
      ? null
      : { tick: latestTick + 1, snakeId, turn, boost },
    sendAction: action => {
      const sentAt = performance.now();
      if (lastActionAt !== undefined) timings.playerActionInterval.record(sentAt - lastActionAt);
      lastActionAt = sentAt;
      socket.send(JSON.stringify({ type: 'action', ...action }));
      counters.actions++;
      if (suppressing) counters.actionsDuringSuppression++;
      counters.latestTurn = action.turn;
      counters.latestBoost = action.boost;
    }
  });
  socket.on('open', () => {
    connected = true;
    socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
    socket.send(JSON.stringify({ type: 'join', mode: 'player', name: 'stage6-browser-probe' }));
    socket.send(JSON.stringify({ type: 'view', mode: 'follow', viewW: 1280, viewH: 720 }));
  });
  socket.on('error', error => fail(error));
  socket.on('close', () => {
    connected = false;
    pump.stop();
    if (!settled) fail(new Error('browser-player socket closed before assignment'));
  });
  socket.on('message', (data, binary) => {
    if (suppressing) counters.inboundDuringSuppression++;
    if (binary) {
      counters.frames++;
      counters.maximumFrameBytes = Math.max(counters.maximumFrameBytes, rawByteLength(data));
      maybeReady();
      return;
    }
    try {
      const message = parseMessage(data);
      if (message['type'] === 'error') {
        fail(new Error(String(message['message'] ?? 'server error')));
      } else if (message['type'] === 'assign') {
        if (typeof message['snakeId'] !== 'number') throw new TypeError('invalid browser-player assignment');
        counters.assignments++;
        snakeId = message['snakeId'];
        assigned = true;
        timings.controllerLifecycle.record(performance.now() - openedAt);
        pump.start();
        maybeReady();
      } else if (message['type'] === 'sensors') {
        if (typeof message['tick'] !== 'number' || typeof message['snakeId'] !== 'number') {
          throw new TypeError('invalid browser-player sensor envelope');
        }
        counters.sensors++;
        latestTick = message['tick'];
        snakeId = message['snakeId'];
        maybeReady();
      }
    } catch (error) {
      fail(error instanceof Error ? error : new Error(String(error)));
    }
  });
  return {
    socket,
    ready,
    beginSuppression() {
      if (paused) throw new Error('browser-player inbound delivery is already paused');
      socket.pause();
      paused = true;
      suppressing = true;
      turn = -1;
      boost = 0;
      pump.requestImmediate();
    },
    endSuppression() {
      suppressing = false;
      if (paused) socket.resume();
      paused = false;
    },
    stop() {
      suppressing = false;
      if (paused) socket.resume();
      paused = false;
      pump.stop();
    }
  };
}

/** Open one real full-frame spectator. */
function createViewer(
  options: ProbeOptions,
  counters: ViewerCounters,
  timings: ClientTimingOwners
): WebSocket {
  const socket = new WebSocket(options.wsUrl);
  let lastFrameAt: number | undefined;
  socket.on('open', () => {
    socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
    socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
  });
  socket.on('error', error => {
    if (counters.errors.length < 16) counters.errors.push(error.message);
  });
  socket.on('message', (data, binary) => {
    if (binary) {
      const frameAt = performance.now();
      if (lastFrameAt !== undefined) timings.frameInterval.record(frameAt - lastFrameAt);
      lastFrameAt = frameAt;
      counters.frames++;
      counters.maximumFrameBytes = Math.max(counters.maximumFrameBytes, rawByteLength(data));
      return;
    }
    try {
      const message = parseMessage(data);
      if (message['type'] === 'stats') counters.stats++;
      if (message['type'] === 'error' && counters.errors.length < 16) {
        counters.errors.push(String(message['message'] ?? 'server error'));
      }
    } catch (error) {
      if (counters.errors.length < 16) counters.errors.push(error instanceof Error ? error.message : String(error));
    }
  });
  return socket;
}

/** Wait for a bounded condition without creating an unbounded sample series. */
async function waitUntil(predicate: () => boolean, deadline: number, label: string): Promise<void> {
  while (!predicate() && performance.now() < deadline) {
    await new Promise<void>(resolvePromise => setTimeout(resolvePromise, 20));
  }
  if (!predicate()) throw new Error(`${label} timed out`);
}

/** Close one socket and wait briefly for its disconnect to reach the server. */
async function closeSocket(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.CLOSED) return;
  const closed = new Promise<void>(resolvePromise => socket.once('close', () => resolvePromise()));
  socket.close();
  await closed;
}

/** Run one scalar live integration probe and enforce its selected gates. */
export async function run(options: ProbeOptions): Promise<Record<string, unknown>> {
  const startedAt = performance.now();
  const deadline = startedAt + options.durationMs;
  const endpoint = healthUrl(options.wsUrl);
  const timings = createClientTimings();
  const initialHealth = await fetchHealth(endpoint, timings.healthRequest);
  const controller: ControllerCounters = { assignments: 0, sensors: 0, actions: 0,
    successfulReclaims: 0, errors: [] };
  const viewer: ViewerCounters = { frames: 0, stats: 0, maximumFrameBytes: 0, errors: [] };
  const browserPlayer: BrowserPlayerCounters = { assignments: 0, sensors: 0, frames: 0,
    actions: 0, actionsDuringSuppression: 0, inboundDuringSuppression: 0,
    maximumFrameBytes: 0, latestTurn: 0, latestBoost: 0, errors: [] };
  const outbound: OutboundMaxima = { samples: 0, reliableQueuedMessages: 0,
    reliableQueuedBytes: 0, pendingFrames: 0, replacedFrames: 0, reliableFailures: 0 };
  observeOutbound(initialHealth, outbound);
  const viewerSocket = createViewer(options, viewer, timings);
  const player = createBrowserPlayer(options, browserPlayer, timings);
  let active = createController(options, controller, timings);
  try {
    await Promise.all([active.ready, player.ready]);
    await waitUntil(() => controller.sensors >= options.reconnectAfterSensors, deadline, 'pre-reclaim observations');
    const firstToken = controller.resumeToken;
    const firstSnake = controller.snakeId;
    if (!firstToken || firstSnake === undefined) throw new Error('initial assignment omitted reclaim identity');
    await closeSocket(active.socket);
    active = createController(options, controller, timings, firstToken);
    await active.ready;
    if (controller.successfulReclaims < 1 || controller.snakeId !== firstSnake || controller.resumeToken === firstToken) {
      throw new Error('same-snake reclaim or token rotation was not observed');
    }
    const beforeSuppression = await fetchHealth(endpoint, timings.healthRequest);
    observeOutbound(beforeSuppression, outbound);
    const playerSamplesBefore = telemetrySamples(beforeSuppression, 'playerAction');
    const replacedFramesBefore = outbound.replacedFrames;
    const sensorsBeforeResume = browserPlayer.sensors;
    const framesBeforeResume = browserPlayer.frames;
    player.beginSuppression();
    const suppressionStartedAt = performance.now();
    const suppressionEndsAt = suppressionStartedAt + options.playerSuppressionMs;
    let suppressedHealth = beforeSuppression;
    while (performance.now() < suppressionEndsAt) {
      suppressedHealth = await fetchHealth(endpoint, timings.healthRequest);
      observeOutbound(suppressedHealth, outbound);
      const remaining = Math.max(0, suppressionEndsAt - performance.now());
      await new Promise<void>(resolvePromise => setTimeout(resolvePromise, Math.min(100, remaining)));
    }
    suppressedHealth = await fetchHealth(endpoint, timings.healthRequest);
    observeOutbound(suppressedHealth, outbound);
    const playerSamplesAfter = telemetrySamples(suppressedHealth, 'playerAction');
    const suppressionActualMs = performance.now() - suppressionStartedAt;
    player.endSuppression();
    const resumedAt = performance.now();
    await waitUntil(() => browserPlayer.sensors > sensorsBeforeResume && browserPlayer.frames > framesBeforeResume,
      deadline, 'browser-player inbound recovery');
    timings.playerInboundRecovery.record(performance.now() - resumedAt);
    if (browserPlayer.actionsDuringSuppression === 0 || playerSamplesAfter <= playerSamplesBefore ||
        browserPlayer.latestTurn !== -1 || browserPlayer.latestBoost !== 0) {
      throw new Error('browser-player turn change and boost release did not apply during inbound suppression');
    }
    if (browserPlayer.inboundDuringSuppression !== 0) {
      throw new Error('browser-player inbound traffic was consumed during the deliberate socket pause');
    }
    const replacedFramesDuringSuppression = outbound.replacedFrames - replacedFramesBefore;
    if (options.requireFrameReplacement && replacedFramesDuringSuppression < 1) {
      throw new Error('slow browser-player socket did not produce a server-side frame replacement');
    }
    while (performance.now() < deadline) {
      observeOutbound(await fetchHealth(endpoint, timings.healthRequest), outbound);
      const remaining = Math.max(0, deadline - performance.now());
      await new Promise<void>(resolvePromise => setTimeout(resolvePromise, Math.min(250, remaining)));
    }
    const finalHealth = await fetchHealth(endpoint, timings.healthRequest);
    observeOutbound(finalHealth, outbound);
    const initialGeneration = nativeCounter(initialHealth['generation'], 'generation');
    const finalGeneration = nativeCounter(finalHealth['generation'], 'generation');
    if (controller.errors.length > 0 || viewer.errors.length > 0 || browserPlayer.errors.length > 0) {
      throw new Error(`protocol failures: ${JSON.stringify({ controller: controller.errors,
        browserPlayer: browserPlayer.errors, viewer: viewer.errors })}`);
    }
    if (controller.sensors === 0 || controller.actions === 0 || viewer.frames === 0 || viewer.stats === 0) {
      throw new Error('required trainer or viewer traffic was not observed');
    }
    if (options.requireGenerationTransition && finalGeneration <= initialGeneration) {
      throw new Error('native generation did not advance during the required complete-generation probe');
    }
    return {
      schema: 'slither-stage6a-runtime-integration-probe',
      version: 2,
      caveat: 'Protocol 2 wire-compatible diagnostic client; not the owner trainer or a browser on another LAN device.',
      capturedAt: new Date().toISOString(),
      wsUrl: options.wsUrl,
      measuredWallSeconds: (performance.now() - startedAt) / 1_000,
      requireGenerationTransition: options.requireGenerationTransition,
      requireFrameReplacement: options.requireFrameReplacement,
      generation: { before: initialGeneration.toString(), after: finalGeneration.toString() },
      controller,
      browserPlayer,
      playerSuppression: {
        requestedMs: options.playerSuppressionMs,
        actualMs: suppressionActualMs,
        serverPlayerActionSamplesBefore: playerSamplesBefore,
        serverPlayerActionSamplesAfter: playerSamplesAfter,
        replacedFrames: replacedFramesDuringSuppression,
        recoveredSensors: browserPlayer.sensors - sensorsBeforeResume,
        recoveredFrames: browserPlayer.frames - framesBeforeResume
      },
      viewer,
      outboundMaxima: outbound,
      clientLatency: {
        controllerLifecycle: timings.controllerLifecycle.snapshot(),
        sensorInterval: timings.sensorInterval.snapshot(),
        sensorToActionDispatch: timings.sensorToActionDispatch.snapshot(),
        actionToNextSensor: timings.actionToNextSensor.snapshot(),
        frameInterval: timings.frameInterval.snapshot(),
        healthRequest: timings.healthRequest.snapshot(),
        playerActionInterval: timings.playerActionInterval.snapshot(),
        playerInboundRecovery: timings.playerInboundRecovery.snapshot()
      },
      telemetry: record(finalHealth['telemetry'], 'health telemetry')
    };
  } finally {
    player.stop();
    active.socket.terminate();
    player.socket.terminate();
    viewerSocket.terminate();
  }
}

/** Execute the standalone probe. */
async function main(): Promise<void> {
  const report = await run(parseOptions(process.argv.slice(2)));
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
}

/** Resolved CLI path used only to avoid side effects when imported. */
const invokedPath = process.argv[1];
if (invokedPath && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
  main().catch(error => {
    console.error(error);
    process.exitCode = 1;
  });
}
