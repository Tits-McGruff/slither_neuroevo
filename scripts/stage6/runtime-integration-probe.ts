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

/** Validated command-line controls for one bounded probe. */
interface ProbeOptions {
  /** Experimental server WebSocket endpoint. */
  wsUrl: string;
  /** Total connected wall duration. */
  durationMs: number;
  /** Observation count before disconnect/token reclaim. */
  reconnectAfterSensors: number;
  /** Require the native generation counter to advance. */
  requireGenerationTransition: boolean;
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
}

/** Open controller plus its first assignment/observation boundary. */
interface ControllerConnection {
  /** Real network socket. */
  socket: WebSocket;
  /** Resolves after both assignment and observation arrive. */
  ready: Promise<void>;
}

/** Default live duration for a fast diagnostic rather than the complete gate. */
const DEFAULT_DURATION_SECONDS = 30;

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
    frameInterval: new FixedLatencyHistogram()
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
  for (let index = 0; index < arguments_.length; index++) {
    const option = arguments_[index]!;
    if (option === '--require-generation-transition') requireGenerationTransition = true;
    else if (option === '--ws-url') wsUrl = arguments_[++index] ?? '';
    else if (option === '--duration-seconds') durationMs = positiveNumber(arguments_[++index], option) * 1_000;
    else if (option === '--reconnect-after-sensors') {
      reconnectAfterSensors = positiveNumber(arguments_[++index], option);
      if (!Number.isSafeInteger(reconnectAfterSensors)) throw new RangeError(`${option} must be an integer`);
    } else throw new Error(`unknown option: ${option}`);
  }
  const parsedUrl = new URL(wsUrl);
  if ((parsedUrl.protocol !== 'ws:' && parsedUrl.protocol !== 'wss:') || !parsedUrl.host) {
    throw new TypeError('--ws-url must be an absolute ws:// or wss:// URL');
  }
  return { wsUrl: parsedUrl.href, durationMs, reconnectAfterSensors, requireGenerationTransition };
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
async function fetchHealth(url: string): Promise<Record<string, unknown>> {
  const response = await fetch(url);
  const body = record(await response.json(), 'health response');
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
async function run(options: ProbeOptions): Promise<Record<string, unknown>> {
  const startedAt = performance.now();
  const deadline = startedAt + options.durationMs;
  const endpoint = healthUrl(options.wsUrl);
  const initialHealth = await fetchHealth(endpoint);
  const controller: ControllerCounters = { assignments: 0, sensors: 0, actions: 0,
    successfulReclaims: 0, errors: [] };
  const viewer: ViewerCounters = { frames: 0, stats: 0, maximumFrameBytes: 0, errors: [] };
  const timings = createClientTimings();
  const outbound: OutboundMaxima = { samples: 0, reliableQueuedMessages: 0,
    reliableQueuedBytes: 0, pendingFrames: 0, replacedFrames: 0, reliableFailures: 0 };
  observeOutbound(initialHealth, outbound);
  const viewerSocket = createViewer(options, viewer, timings);
  let active = createController(options, controller, timings);
  try {
    await active.ready;
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
    while (performance.now() < deadline) {
      observeOutbound(await fetchHealth(endpoint), outbound);
      const remaining = Math.max(0, deadline - performance.now());
      await new Promise<void>(resolvePromise => setTimeout(resolvePromise, Math.min(250, remaining)));
    }
    const finalHealth = await fetchHealth(endpoint);
    observeOutbound(finalHealth, outbound);
    const initialGeneration = nativeCounter(initialHealth['generation'], 'generation');
    const finalGeneration = nativeCounter(finalHealth['generation'], 'generation');
    if (controller.errors.length > 0 || viewer.errors.length > 0) {
      throw new Error(`protocol failures: ${JSON.stringify({ controller: controller.errors, viewer: viewer.errors })}`);
    }
    if (controller.sensors === 0 || controller.actions === 0 || viewer.frames === 0 || viewer.stats === 0) {
      throw new Error('required trainer or viewer traffic was not observed');
    }
    if (options.requireGenerationTransition && finalGeneration <= initialGeneration) {
      throw new Error('native generation did not advance during the required complete-generation probe');
    }
    return {
      schema: 'slither-stage6a-runtime-integration-probe',
      version: 1,
      caveat: 'Protocol 2 wire-compatible diagnostic client; not the owner trainer or a browser on another LAN device.',
      capturedAt: new Date().toISOString(),
      wsUrl: options.wsUrl,
      measuredWallSeconds: (performance.now() - startedAt) / 1_000,
      requireGenerationTransition: options.requireGenerationTransition,
      generation: { before: initialGeneration.toString(), after: finalGeneration.toString() },
      controller,
      viewer,
      outboundMaxima: outbound,
      clientLatency: {
        controllerLifecycle: timings.controllerLifecycle.snapshot(),
        sensorInterval: timings.sensorInterval.snapshot(),
        sensorToActionDispatch: timings.sensorToActionDispatch.snapshot(),
        actionToNextSensor: timings.actionToNextSensor.snapshot(),
        frameInterval: timings.frameInterval.snapshot()
      },
      telemetry: record(finalHealth['telemetry'], 'health telemetry')
    };
  } finally {
    active.socket.terminate();
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
