/** Browser-player action transmission independent from sensor delivery. */

/** Candidate resend rates retained for Stage 2 measurement. */
export const PLAYER_ACTION_RATE_CANDIDATES = [30, 60] as const;
/** Temporary default candidate; Stage 2 compares both on the real LAN workload. */
export const DEFAULT_PLAYER_ACTION_RATE_HZ = 60;

/** Fully constructed latest-value command ready for Protocol 2 transmission. */
export interface LatestPlayerAction {
  /** Latest observed authoritative tick plus the existing compatibility offset. */
  tick: number;
  /** Current assigned snake id. */
  snakeId: number;
  /** Recomputed turn command. */
  turn: number;
  /** Current boost state. */
  boost: number;
}

/** Timer and clock functions injectable for deterministic browser tests. */
export interface PlayerActionPumpClock {
  /** Monotonic browser clock in milliseconds. */
  nowMs: () => number;
  /** Schedule one delayed trailing send. */
  setTimeout: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout>;
  /** Cancel one delayed trailing send. */
  clearTimeout: (timer: ReturnType<typeof setTimeout>) => void;
  /** Schedule periodic latest-command resends. */
  setInterval: (callback: () => void, delayMs: number) => ReturnType<typeof setInterval>;
  /** Cancel periodic resends. */
  clearInterval: (timer: ReturnType<typeof setInterval>) => void;
}

/** Construction options for the browser-player latest-value sender. */
export interface PlayerActionPumpOptions {
  /** Candidate periodic resend rate. */
  cadenceHz: number;
  /** Whether a live browser player currently owns a snake. */
  isActive: () => boolean;
  /** Build the newest command at send time, or null when state is incomplete. */
  buildLatestAction: () => LatestPlayerAction | null;
  /** Transmit one already-built Protocol 2 action. */
  sendAction: (action: LatestPlayerAction) => void;
  /** Optional deterministic timer/clock implementation. */
  clock?: PlayerActionPumpClock;
}

/**
 * Resolve a supported 30/60 Hz candidate without turning the temporary choice
 * into a permanent protocol rule.
 * @param requested - Query/config value to inspect.
 * @returns A supported candidate, defaulting to 60 Hz.
 */
export function normalizePlayerActionRate(requested: unknown): number {
  const numeric = typeof requested === 'number' ? requested : Number(requested);
  return PLAYER_ACTION_RATE_CANDIDATES.includes(
    numeric as (typeof PLAYER_ACTION_RATE_CANDIDATES)[number]
  )
    ? numeric
    : DEFAULT_PLAYER_ACTION_RATE_HZ;
}

/**
 * Coalescing browser-player sender with immediate bounded changes and periodic
 * latest-value resend. Sensor callbacks are deliberately absent from this API.
 */
export class PlayerActionPump {
  /** Validated periodic resend rate. */
  private readonly cadenceHz: number;
  /** Minimum interval between actual sends. */
  private readonly minimumIntervalMs: number;
  /** Ownership predicate. */
  private readonly isActive: PlayerActionPumpOptions['isActive'];
  /** Send-time action builder. */
  private readonly buildLatestAction: PlayerActionPumpOptions['buildLatestAction'];
  /** Protocol transport callback. */
  private readonly sendAction: PlayerActionPumpOptions['sendAction'];
  /** Timer and monotonic clock implementation. */
  private readonly clock: PlayerActionPumpClock;
  /** Periodic resend timer. */
  private intervalTimer: ReturnType<typeof setInterval> | null = null;
  /** Single coalesced trailing-send timer. */
  private trailingTimer: ReturnType<typeof setTimeout> | null = null;
  /** Monotonic time of the latest successful transport call. */
  private lastSentAtMs = Number.NEGATIVE_INFINITY;

  /**
   * @param options - Ownership, command builder, transport, cadence, and clock.
   */
  constructor(options: PlayerActionPumpOptions) {
    this.cadenceHz = normalizePlayerActionRate(options.cadenceHz);
    this.minimumIntervalMs = 1000 / this.cadenceHz;
    this.isActive = options.isActive;
    this.buildLatestAction = options.buildLatestAction;
    this.sendAction = options.sendAction;
    this.clock = options.clock ?? {
      nowMs: () => performance.now(),
      setTimeout: (callback, delayMs) => setTimeout(callback, delayMs),
      clearTimeout: timer => clearTimeout(timer),
      setInterval: (callback, delayMs) => setInterval(callback, delayMs),
      clearInterval: timer => clearInterval(timer)
    };
  }

  /** Start periodic latest-command resends for the current ownership epoch. */
  start(): void {
    if (this.intervalTimer !== null) return;
    this.intervalTimer = this.clock.setInterval(
      () => this.requestSend(),
      this.minimumIntervalMs
    );
    this.requestSend();
  }

  /** Stop all periodic and trailing work without changing desired input state. */
  stop(): void {
    if (this.intervalTimer !== null) {
      this.clock.clearInterval(this.intervalTimer);
      this.intervalTimer = null;
    }
    if (this.trailingTimer !== null) {
      this.clock.clearTimeout(this.trailingTimer);
      this.trailingTimer = null;
    }
    this.lastSentAtMs = Number.NEGATIVE_INFINITY;
  }

  /**
   * Request an immediate bounded send after pointer/button/boost state changes.
   * Multiple changes inside one interval collapse into one trailing send.
   */
  requestImmediate(): void {
    this.requestSend();
  }

  /** Send now when allowed, otherwise maintain one newest-value trailing timer. */
  private requestSend(): void {
    if (!this.isActive()) return;
    const now = this.clock.nowMs();
    const remaining = this.minimumIntervalMs - (now - this.lastSentAtMs);
    if (remaining <= 0) {
      this.sendLatest(now);
      return;
    }
    if (this.trailingTimer !== null) return;
    this.trailingTimer = this.clock.setTimeout(() => {
      this.trailingTimer = null;
      if (!this.isActive()) return;
      this.sendLatest(this.clock.nowMs());
    }, remaining);
  }

  /**
   * Build from the newest pointer/camera/player state and invoke the transport.
   * @param now - Monotonic send time.
   */
  private sendLatest(now: number): void {
    const action = this.buildLatestAction();
    if (!action) return;
    if (this.trailingTimer !== null) {
      this.clock.clearTimeout(this.trailingTimer);
      this.trailingTimer = null;
    }
    this.sendAction(action);
    this.lastSentAtMs = now;
  }
}
