import type { LiveSettingPath, LiveSettingsUpdate } from '../protocol/settings.ts';

/** Default quiet period used to coalesce slider input. */
const DEFAULT_SETTINGS_DEBOUNCE_MS = 80;
/** Default minimum interval between non-final drag messages. */
const DEFAULT_DRAG_THROTTLE_MS = 50;
/** Process-local fallback ordinal for request-id prefixes. */
let nextControlClientOrdinal = 1;

/** Minimal transport consumed by the extracted authoritative control logic. */
export interface AuthoritativeControlTransport {
  /** Send an atomic live-settings request. */
  sendSettings: (requestId: string, updates: LiveSettingsUpdate[]) => void;
  /** Send a God Mode kill request. */
  sendGodModeKill: (requestId: string, snakeId: number) => void;
  /** Send a God Mode move request. */
  sendGodModeMove: (requestId: string, snakeId: number, x: number, y: number) => void;
  /** Send a New Run request. */
  sendNewRun: (requestId: string) => void;
}

/** Injectable timer and clock dependencies for deterministic tests. */
export interface AuthoritativeControlClock {
  /** Return monotonic milliseconds. */
  now: () => number;
  /** Schedule a callback after a delay. */
  schedule: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout>;
  /** Cancel a scheduled callback. */
  cancel: (timer: ReturnType<typeof setTimeout>) => void;
}

/** Optional behavior overrides for one browser control client. */
export interface AuthoritativeControlOptions {
  /** Quiet period used for setting coalescing. */
  settingsDebounceMs?: number;
  /** Minimum non-final drag send interval. */
  dragThrottleMs?: number;
  /** Deterministic request-id prefix. */
  requestIdPrefix?: string;
  /** Injectable timer/clock implementation. */
  clock?: AuthoritativeControlClock;
}

/** Extracted API used by main.ts for authoritative operations. */
export interface AuthoritativeControls {
  /** Queue the latest unsent value for one setting path. */
  queueSetting: (path: LiveSettingPath, value: number) => void;
  /** Immediately send all currently queued settings as one atomic request. */
  flushSettings: () => void;
  /** Send one God Mode kill request and return its correlation id. */
  killSnake: (snakeId: number) => string;
  /** Throttle one intermediate God Mode drag position. */
  moveSnake: (snakeId: number, x: number, y: number) => void;
  /** Always send the final mouse-up position immediately. */
  finishMove: (snakeId: number, x: number, y: number) => string;
  /** Send one New Run request and return its correlation id. */
  requestNewRun: () => string;
  /** Cancel timers and discard unsent local values. */
  dispose: () => void;
}

/** Pending coalesced God Mode drag position. */
interface PendingMove {
  /** Target snake id. */
  snakeId: number;
  /** Requested head X. */
  x: number;
  /** Requested head Y. */
  y: number;
}

/** Browser-backed default timer and monotonic clock. */
const DEFAULT_CLOCK: AuthoritativeControlClock = {
  now: () => performance.now(),
  schedule: (callback, delayMs) => setTimeout(callback, delayMs),
  cancel: (timer) => clearTimeout(timer)
};

/**
 * Create a collision-resistant non-authoritative request-id prefix.
 * @returns Prefix unique enough for concurrent browser clients.
 */
function createRequestIdPrefix(): string {
  const ordinal = nextControlClientOrdinal++;
  const randomId = typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${ordinal.toString(36)}`;
  return `ui-${randomId}`;
}

/**
 * Create coalesced authoritative settings and God Mode transport behavior.
 * @param transport - Connected WebSocket send surface.
 * @param options - Optional timing and request-id overrides.
 * @returns Small browser control client used by main.ts.
 */
export function createAuthoritativeControls(
  transport: AuthoritativeControlTransport,
  options: AuthoritativeControlOptions = {}
): AuthoritativeControls {
  const clock = options.clock ?? DEFAULT_CLOCK;
  const settingsDebounceMs = Math.max(0, options.settingsDebounceMs ?? DEFAULT_SETTINGS_DEBOUNCE_MS);
  const dragThrottleMs = Math.max(0, options.dragThrottleMs ?? DEFAULT_DRAG_THROTTLE_MS);
  const requestIdPrefix = options.requestIdPrefix?.trim() || createRequestIdPrefix();
  const pendingSettings = new Map<LiveSettingPath, number>();
  let requestOrdinal = 0;
  let settingsTimer: ReturnType<typeof setTimeout> | null = null;
  let dragTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingMove: PendingMove | null = null;
  let lastMoveSentAt = Number.NEGATIVE_INFINITY;

  /** Return the next correlation id owned by this client instance. */
  const nextRequestId = (): string => `${requestIdPrefix}-${++requestOrdinal}`;

  /** Send and clear the latest per-path settings values. */
  const flushSettings = (): void => {
    if (settingsTimer !== null) {
      clock.cancel(settingsTimer);
      settingsTimer = null;
    }
    if (pendingSettings.size === 0) return;
    const updates = Array.from(pendingSettings, ([path, value]) => ({ path, value }));
    pendingSettings.clear();
    transport.sendSettings(nextRequestId(), updates);
  };

  /** Queue one path while replacing only this client's prior unsent value. */
  const queueSetting = (path: LiveSettingPath, value: number): void => {
    pendingSettings.set(path, value);
    if (settingsTimer !== null) clock.cancel(settingsTimer);
    settingsTimer = clock.schedule(() => {
      settingsTimer = null;
      flushSettings();
    }, settingsDebounceMs);
  };

  /** Send one move immediately and update throttle accounting. */
  const sendMove = (move: PendingMove): string => {
    const requestId = nextRequestId();
    transport.sendGodModeMove(requestId, move.snakeId, move.x, move.y);
    lastMoveSentAt = clock.now();
    return requestId;
  };

  /** Send the most recent queued intermediate drag position. */
  const flushMove = (): void => {
    dragTimer = null;
    const move = pendingMove;
    pendingMove = null;
    if (move) sendMove(move);
  };

  /** Throttle intermediate drag positions while retaining the newest one. */
  const moveSnake = (snakeId: number, x: number, y: number): void => {
    pendingMove = { snakeId, x, y };
    const remaining = dragThrottleMs - (clock.now() - lastMoveSentAt);
    if (remaining <= 0 && dragTimer === null) {
      flushMove();
      return;
    }
    if (dragTimer === null) {
      dragTimer = clock.schedule(flushMove, Math.max(0, remaining));
    }
  };

  /** Cancel a pending intermediate position and always send mouse-up state. */
  const finishMove = (snakeId: number, x: number, y: number): string => {
    if (dragTimer !== null) {
      clock.cancel(dragTimer);
      dragTimer = null;
    }
    pendingMove = null;
    return sendMove({ snakeId, x, y });
  };

  /** Send one kill request immediately. */
  const killSnake = (snakeId: number): string => {
    const requestId = nextRequestId();
    transport.sendGodModeKill(requestId, snakeId);
    return requestId;
  };

  /** Send one New Run request immediately. */
  const requestNewRun = (): string => {
    const requestId = nextRequestId();
    transport.sendNewRun(requestId);
    return requestId;
  };

  /** Cancel timers and discard all unsent client-local state. */
  const dispose = (): void => {
    if (settingsTimer !== null) clock.cancel(settingsTimer);
    if (dragTimer !== null) clock.cancel(dragTimer);
    settingsTimer = null;
    dragTimer = null;
    pendingSettings.clear();
    pendingMove = null;
  };

  return {
    queueSetting,
    flushSettings,
    killSnake,
    moveSnake,
    finishMove,
    requestNewRun,
    dispose
  };
}
