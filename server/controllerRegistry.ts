import { randomBytes } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import type { ControlInput } from '../src/snake.ts';
import type {
  ActionMsg,
  AssignMsg,
  ReclaimResultMsg,
  SensorsMsg,
  ServerMessage
} from './protocol.ts';

/** Supported controller types for a snake. */
export type ControllerType = 'player' | 'bot';

/** Rate limits and wall-time lease rules for controller actions. */
export interface ControllerRegistryOptions {
  /** Maximum accepted actions from one assignment during one authoritative step. */
  maxActionsPerTick: number;
  /** Maximum accepted actions from one assignment during one wall-clock second. */
  maxActionsPerSecond: number;
  /** Wall milliseconds for which the newest accepted input remains active. */
  inputHoldMs: number;
  /** Wall milliseconds for which disconnected ownership remains reserved. */
  disconnectGraceMs: number;
}

/** Dependencies for controller registry state updates. */
export interface ControllerRegistryDeps {
  /** Return the current authoritative snake roster. */
  getSnakes: () => Array<{ id: number; alive: boolean; controllable: boolean }>;
  /** Route one reliable controller/lifecycle message and report queue acceptance. */
  send: (connId: number, payload: ServerMessage) => boolean;
  /** Monotonic elapsed-time source, injectable for deterministic tests. */
  nowMs?: () => number;
  /** Opaque token source independent from every simulation RNG stream. */
  createResumeToken?: () => string;
  /** Current server-session/run scope to which new leases are bound. */
  getLeaseScope?: () => string;
}

/** Result of an explicit token or legacy-identity reclaim attempt. */
export interface ControllerReclaimResult {
  /** Whether the existing live lease was rebound. */
  reclaimed: boolean;
  /** Stable result category, including an internal outbound-delivery failure. */
  reason: ReclaimResultMsg['reason'] | 'delivery-failed';
  /** Reclaimed snake id on success. */
  snakeId?: number;
}

/** Internal per-controller state tracked across steps and disconnects. */
interface ControllerState {
  /** Authoritative snake owned by this lease. */
  snakeId: number;
  /** Live connection id, or null while disconnected and reserved. */
  connId: number | null;
  /** Controller class retained across reconnect. */
  controllerType: ControllerType;
  /** Bounded name-derived key used only for unambiguous legacy reconnect. */
  identityKey: string;
  /** Server-session/run scope that prevents cross-run reclaim. */
  leaseScope: string;
  /** Current opaque resume token. */
  resumeToken: string;
  /** Latest accepted turn command. */
  lastTurn: number;
  /** Latest accepted boost command. */
  lastBoost: number;
  /** Latest client-reported tick, retained for diagnostics only. */
  lastClientTick: number;
  /** Monotonic wall time of the latest accepted action. */
  lastActionAtMs: number;
  /** Monotonic wall time at disconnect, or null while connected. */
  disconnectedAtMs: number | null;
  /** Authoritative step identity owning the per-step rate counter. */
  actionTickId: number;
  /** Accepted actions in the current authoritative step. */
  actionsThisTick: number;
  /** Start of the current wall-second rate window. */
  actionSecondStartMs: number;
  /** Accepted/rejected action attempts counted in the current wall-second window. */
  actionsThisSecond: number;
  /** Total actions rejected by rate limits. */
  droppedActions: number;
}

/**
 * Clamp a value to a numeric range.
 * @param value - Input value to clamp.
 * @param min - Inclusive minimum.
 * @param max - Inclusive maximum.
 * @returns Clamped value.
 */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Check for a finite numeric value.
 * @param value - Value to test.
 * @returns True when value is a finite number.
 */
function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Generate one opaque trusted-LAN resume token without consuming simulation RNG.
 * @returns Base64url token backed by operating-system entropy.
 */
function createDefaultResumeToken(): string {
  return randomBytes(24).toString('base64url');
}

/** Registry tracking active and temporarily reserved external controllers. */
export class ControllerRegistry {
  /** Controller state keyed by snake id, including disconnected reservations. */
  private bySnake = new Map<number, ControllerState>();
  /** Controller state keyed by live connection id. */
  private byConn = new Map<number, ControllerState>();
  /** Controller state keyed by current opaque resume token. */
  private byToken = new Map<string, ControllerState>();
  /** Recently expired token tombstones used for an explicit expired result. */
  private expiredTokens = new Map<string, number>();
  /** Current authoritative step id for per-step rate limiting. */
  private currentTickId = 0;
  /** Registry options for rate limiting and wall-time ownership. */
  private readonly options: ControllerRegistryOptions;
  /** Supplier for the current snake list. */
  private readonly getSnakes: ControllerRegistryDeps['getSnakes'];
  /** Sender for per-connection messages. */
  private readonly send: ControllerRegistryDeps['send'];
  /** Monotonic wall-time source. */
  private readonly nowMs: () => number;
  /** Opaque token generator independent from authoritative randomness. */
  private readonly createResumeToken: () => string;
  /** Current session/run lease scope. */
  private readonly getLeaseScope: () => string;

  /**
   * Create a controller registry instance.
   * @param options - Rate limits, input hold, and disconnect grace.
   * @param deps - Snake access, reliable send, clock, token, and scope dependencies.
   */
  constructor(options: ControllerRegistryOptions, deps: ControllerRegistryDeps) {
    this.options = options;
    this.getSnakes = deps.getSnakes;
    this.send = deps.send;
    this.nowMs = deps.nowMs ?? (() => performance.now());
    this.createResumeToken = deps.createResumeToken ?? createDefaultResumeToken;
    this.getLeaseScope = deps.getLeaseScope ?? (() => 'default');
  }

  /**
   * Update the current authoritative step id for rate limiting.
   * @param tickId - Step about to begin.
   */
  setTickId(tickId: number): void {
    this.currentTickId = tickId;
  }

  /**
   * Expire elapsed disconnect leases and old token tombstones.
   * Call this before every fixed step so grace expiry is a single boundary event.
   * @param now - Optional injected monotonic wall time.
   */
  refresh(now = this.nowMs()): void {
    for (const state of Array.from(this.bySnake.values())) {
      if (state.disconnectedAtMs === null) continue;
      if (now - state.disconnectedAtMs < this.options.disconnectGraceMs) continue;
      this.removeState(state, now, true);
    }
    for (const [token, expiresAt] of this.expiredTokens) {
      if (expiresAt <= now) this.expiredTokens.delete(token);
    }
  }

  /**
   * Check if a snake retains external ownership, including disconnect grace.
   * Lease expiry is applied only by an explicit `refresh` at a step boundary,
   * never halfway through control collection.
   * @param snakeId - Snake id to check.
   * @returns True while connected or reserved.
   */
  isControlled(snakeId: number): boolean {
    return this.bySnake.has(snakeId);
  }

  /**
   * Check whether an interactive browser player is connected or reserved.
   * Callers use the most recently refreshed boundary state.
   * @returns True when catch-up must yield between every overdue step.
   */
  hasInteractiveController(): boolean {
    for (const state of this.bySnake.values()) {
      if (state.controllerType === 'player') return true;
    }
    return false;
  }

  /**
   * Return the assigned snake id for a live connection.
   * @param connId - Connection id to check.
   * @returns Assigned snake id or null.
   */
  getAssignedSnakeId(connId: number): number | null {
    return this.byConn.get(connId)?.snakeId ?? null;
  }

  /**
   * Assign a new controller lease and notify the client.
   * @param connId - Connection id to assign.
   * @param controllerType - Controller type for the connection.
   * @param snakeId - Optional explicit snake id to assign.
   * @param identityKey - Bounded identity used only for legacy reconnect matching.
   * @returns Assigned snake id or null when none is available.
   */
  assignSnake(
    connId: number,
    controllerType: ControllerType,
    snakeId?: number,
    identityKey = ''
  ): number | null {
    this.releaseSnake(connId);
    const assignedId = snakeId ?? this.pickAvailableSnake();
    if (assignedId == null || !this.isSnakeAssignable(assignedId)) return null;
    const now = this.nowMs();
    const state: ControllerState = {
      snakeId: assignedId,
      connId,
      controllerType,
      identityKey,
      leaseScope: this.getLeaseScope(),
      resumeToken: this.createUniqueToken(),
      lastTurn: 0,
      lastBoost: 0,
      lastClientTick: this.currentTickId,
      lastActionAtMs: now,
      disconnectedAtMs: null,
      actionTickId: this.currentTickId,
      actionsThisTick: 0,
      actionSecondStartMs: now,
      actionsThisSecond: 0,
      droppedActions: 0
    };
    if (!this.sendAssignment(state, false)) return null;
    this.byConn.set(connId, state);
    this.bySnake.set(assignedId, state);
    this.byToken.set(state.resumeToken, state);
    return assignedId;
  }

  /**
   * Rebind one reserved lease by token or unambiguous legacy identity.
   * @param connId - New live connection id.
   * @param controllerType - Controller class requested by the client.
   * @param resumeToken - Preferred opaque token.
   * @param identityKey - Legacy fallback identity when no token is supplied.
   * @returns Explicit reclaim outcome.
   */
  reclaimSnake(
    connId: number,
    controllerType: ControllerType,
    resumeToken?: string,
    identityKey = ''
  ): ControllerReclaimResult {
    const now = this.nowMs();
    this.refresh(now);
    let state: ControllerState | undefined;
    if (resumeToken) {
      if (this.expiredTokens.has(resumeToken)) return { reclaimed: false, reason: 'expired' };
      state = this.byToken.get(resumeToken);
      if (
        !state ||
        state.controllerType !== controllerType ||
        state.leaseScope !== this.getLeaseScope()
      ) {
        return { reclaimed: false, reason: 'invalid' };
      }
    } else if (identityKey) {
      const candidates = Array.from(this.bySnake.values()).filter(
        candidate =>
          candidate.connId === null &&
          candidate.identityKey === identityKey &&
          candidate.controllerType === controllerType &&
          candidate.leaseScope === this.getLeaseScope()
      );
      if (candidates.length > 1) return { reclaimed: false, reason: 'ambiguous' };
      state = candidates[0];
      if (!state) return { reclaimed: false, reason: 'invalid' };
    } else {
      return { reclaimed: false, reason: 'invalid' };
    }

    const snake = this.getSnakes().find(candidate => candidate.id === state.snakeId);
    if (!snake?.alive || !snake.controllable) {
      this.removeState(state, now, true);
      return { reclaimed: false, reason: 'snake-unavailable' };
    }

    if (this.byConn.get(connId) !== state) this.releaseSnake(connId);
    const nextResumeToken = this.createUniqueToken();
    const result: ReclaimResultMsg = {
      type: 'reclaimResult',
      reclaimed: true,
      reason: 'reclaimed',
      snakeId: state.snakeId
    };
    if (
      !this.send(connId, result) ||
      !this.sendAssignment(state, true, connId, nextResumeToken)
    ) {
      return { reclaimed: false, reason: 'delivery-failed' };
    }
    if (state.connId !== null) this.byConn.delete(state.connId);
    this.byToken.delete(state.resumeToken);
    state.resumeToken = nextResumeToken;
    state.connId = connId;
    state.disconnectedAtMs = null;
    state.lastTurn = 0;
    state.lastBoost = 0;
    state.lastActionAtMs = now;
    state.actionTickId = this.currentTickId;
    state.actionsThisTick = 0;
    state.actionSecondStartMs = now;
    state.actionsThisSecond = 0;
    this.byConn.set(connId, state);
    this.bySnake.set(state.snakeId, state);
    this.byToken.set(state.resumeToken, state);
    return { reclaimed: true, reason: 'reclaimed', snakeId: state.snakeId };
  }

  /**
   * Explicitly relinquish a live connection's lease without a grace period.
   * @param connId - Connection id to release.
   */
  releaseSnake(connId: number): void {
    const state = this.byConn.get(connId);
    if (!state) return;
    this.removeState(state, this.nowMs(), false);
  }

  /**
   * Reserve a disconnected connection's snake for wall-clock reclaim grace.
   * @param connId - Closed connection id.
   */
  disconnectConnection(connId: number): void {
    const state = this.byConn.get(connId);
    if (!state) return;
    this.byConn.delete(connId);
    state.connId = null;
    state.disconnectedAtMs = this.nowMs();
    state.lastTurn = 0;
    state.lastBoost = 0;
  }

  /**
   * Reassign connected controllers whose snakes died.
   * Disconnected leases remain reserved until reclaim or grace expiry.
   * @param spawn - Optional spawn callback for new snake ids.
   */
  reassignDeadSnakes(spawn?: () => number | null): void {
    this.refresh();
    const aliveIds = new Set<number>();
    for (const snake of this.getSnakes()) {
      if (snake.alive) aliveIds.add(snake.id);
    }
    for (const state of Array.from(this.byConn.values())) {
      if (aliveIds.has(state.snakeId)) continue;
      this.bySnake.delete(state.snakeId);
      const nextId = spawn ? spawn() : this.pickAvailableSnake();
      if (nextId == null || !this.isSnakeAssignable(nextId)) {
        this.removeState(state, this.nowMs(), true);
        continue;
      }
      const now = this.nowMs();
      const nextResumeToken = this.createUniqueToken();
      if (!this.sendAssignment(state, false, state.connId, nextResumeToken, nextId)) {
        this.removeState(state, now, true);
        continue;
      }
      this.byToken.delete(state.resumeToken);
      state.resumeToken = nextResumeToken;
      state.snakeId = nextId;
      state.lastTurn = 0;
      state.lastBoost = 0;
      state.lastClientTick = this.currentTickId;
      state.lastActionAtMs = now;
      state.actionTickId = this.currentTickId;
      state.actionsThisTick = 0;
      state.actionSecondStartMs = now;
      state.actionsThisSecond = 0;
      state.droppedActions = 0;
      this.bySnake.set(nextId, state);
      this.byToken.set(state.resumeToken, state);
    }
  }

  /**
   * Apply an action message to the controller state, enforcing limits.
   * @param connId - Connection id sending the action.
   * @param msg - Action message payload.
   */
  handleAction(connId: number, msg: ActionMsg): void {
    const state = this.byConn.get(connId);
    if (!state || state.connId !== connId) return;
    if (msg.snakeId !== state.snakeId) return;
    if (!isFiniteNumber(msg.turn) || !isFiniteNumber(msg.boost) || !isFiniteNumber(msg.tick)) return;

    if (state.actionTickId !== this.currentTickId) {
      state.actionTickId = this.currentTickId;
      state.actionsThisTick = 0;
    }
    const now = this.nowMs();
    if (now - state.actionSecondStartMs >= 1000) {
      state.actionSecondStartMs = now;
      state.actionsThisSecond = 0;
    }
    if (state.actionsThisSecond >= this.options.maxActionsPerSecond) {
      state.droppedActions++;
      state.actionsThisSecond++;
      return;
    }

    const tickLimitReached = state.actionsThisTick >= this.options.maxActionsPerTick;
    if (tickLimitReached && state.controllerType !== 'player') {
      state.droppedActions++;
      state.actionsThisSecond++;
      return;
    }

    if (!tickLimitReached) state.actionsThisTick++;
    state.actionsThisSecond++;
    state.lastTurn = clamp(msg.turn, -1, 1);
    state.lastBoost = clamp(msg.boost, 0, 1);
    state.lastClientTick = msg.tick;
    state.lastActionAtMs = now;
  }

  /**
   * Fetch the action for one externally owned snake.
   * Stale or disconnected ownership returns neutral input rather than neural release.
   * @param snakeId - Snake id to query.
   * @param _tickId - Optional diagnostic tick id.
   * @returns Latest held input, neutral held ownership, or null after lease expiry.
   */
  getAction(snakeId: number, _tickId?: number): ControlInput | null {
    const now = this.nowMs();
    const state = this.bySnake.get(snakeId);
    if (!state) return null;
    if (state.connId === null || now - state.lastActionAtMs >= this.options.inputHoldMs) {
      return { turn: 0, boost: 0 };
    }
    return { turn: state.lastTurn, boost: state.lastBoost };
  }

  /**
   * Publish sensor data to a live controlling client.
   * Disconnected reservations intentionally receive nothing.
   * @param snakeId - Snake id owning the sensors.
   * @param tickId - Tick id for the sensor sample.
   * @param sensors - Sensor values.
   * @param meta - Pose metadata for browser steering.
   * @returns True only when the payload entered the reliable outbound path.
   */
  publishSensors(
    snakeId: number,
    tickId: number,
    sensors: Float32Array,
    meta: { x: number; y: number; dir: number }
  ): boolean {
    const state = this.bySnake.get(snakeId);
    if (!state || state.connId === null) return false;
    const msg: SensorsMsg = {
      type: 'sensors',
      tick: tickId,
      snakeId,
      sensors: Array.from(sensors),
      meta
    };
    const connId = state.connId;
    if (this.send(connId, msg)) return true;
    this.disconnectConnection(connId);
    return false;
  }

  /**
   * Send the current token-bearing assignment to a live connection.
   * @param state - Controller lease to announce.
   * @param reclaimed - Whether this is a successful same-snake reclaim.
   */
  private sendAssignment(
    state: ControllerState,
    reclaimed: boolean,
    connId: number | null = state.connId,
    resumeToken = state.resumeToken,
    snakeId = state.snakeId
  ): boolean {
    if (connId === null) return false;
    const assignMsg: AssignMsg = {
      type: 'assign',
      snakeId,
      controller: state.controllerType,
      resumeToken,
      ...(reclaimed ? { reclaimed: true } : {})
    };
    return this.send(connId, assignMsg);
  }

  /**
   * Remove one lease from every index and optionally retain an expired-token tombstone.
   * @param state - Lease to remove.
   * @param now - Monotonic removal time.
   * @param recordExpired - Whether token reclaim should report `expired`.
   */
  private removeState(state: ControllerState, now: number, recordExpired: boolean): void {
    if (state.connId !== null && this.byConn.get(state.connId) === state) {
      this.byConn.delete(state.connId);
    }
    if (this.bySnake.get(state.snakeId) === state) this.bySnake.delete(state.snakeId);
    if (this.byToken.get(state.resumeToken) === state) this.byToken.delete(state.resumeToken);
    if (recordExpired) {
      const retentionMs = Math.max(1000, this.options.disconnectGraceMs);
      this.expiredTokens.set(state.resumeToken, now + retentionMs);
      while (this.expiredTokens.size > 1024) {
        const oldest = this.expiredTokens.keys().next().value as string | undefined;
        if (oldest === undefined) break;
        this.expiredTokens.delete(oldest);
      }
    }
  }

  /**
   * Create a non-empty token that is unique inside this registry.
   * @returns Unique opaque resume token.
   */
  private createUniqueToken(): string {
    for (let attempt = 0; attempt < 8; attempt++) {
      const token = this.createResumeToken();
      if (!token || this.byToken.has(token) || this.expiredTokens.has(token)) continue;
      return token;
    }
    throw new Error('controller resume-token generator failed to produce a unique non-empty token');
  }

  /**
   * Pick an available, alive snake id that is not already controlled.
   * @returns Snake id or null when none is available.
   */
  private pickAvailableSnake(): number | null {
    for (const snake of this.getSnakes()) {
      if (!snake.alive || !snake.controllable || this.bySnake.has(snake.id)) continue;
      return snake.id;
    }
    return null;
  }

  /**
   * Check if a specific snake id can receive a new controller lease.
   * @param snakeId - Snake id to validate.
   * @returns True when alive, controllable, and unowned.
   */
  private isSnakeAssignable(snakeId: number): boolean {
    if (this.bySnake.has(snakeId)) return false;
    return this.getSnakes().some(
      snake => snake.id === snakeId && snake.alive && snake.controllable
    );
  }
}
