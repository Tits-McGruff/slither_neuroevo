import type { ControlInput } from '../src/snake.ts';
import type { ActionMsg, AssignMsg, SensorsMsg, ServerMessage } from './protocol.ts';

/** Supported controller types for a snake. */
export type ControllerType = 'player' | 'bot';

/** Rate limits and timeouts for controller actions. */
export interface ControllerRegistryOptions {
  actionTimeoutTicks: number;
  maxActionsPerTick: number;
  maxActionsPerSecond: number;
}

/** Dependencies for controller registry state updates. */
export interface ControllerRegistryDeps {
  getSnakes: () => Array<{ id: number; alive: boolean }>;
  send: (connId: number, payload: ServerMessage) => void;
}

/** Internal per-controller state tracked across ticks. */
interface ControllerState {
  snakeId: number;
  connId: number;
  controllerType: ControllerType;
  lastTurn: number;
  lastBoost: number;
  lastClientTick: number;
  lastServerTick: number;
  actionTickId: number;
  actionsThisTick: number;
  actionSecondStartMs: number;
  actionsThisSecond: number;
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

/** Registry tracking which connections control which snakes. */
export class ControllerRegistry {
  /** Controller state keyed by snake id. */
  private bySnake = new Map<number, ControllerState>();
  /** Controller state keyed by connection id. */
  private byConn = new Map<number, ControllerState>();
  /** Current server tick id for timeout comparisons. */
  private currentTickId = 0;
  /** Registry options for rate limiting and timeouts. */
  private options: ControllerRegistryOptions;
  /** Supplier for the current snake list. */
  private getSnakes: ControllerRegistryDeps['getSnakes'];
  /** Sender for per-connection messages. */
  private send: ControllerRegistryDeps['send'];

  /**
   * Create a controller registry instance.
   * @param options - Rate limit and timeout configuration.
   * @param deps - Dependencies for snake access and message sending.
   */
  constructor(options: ControllerRegistryOptions, deps: ControllerRegistryDeps) {
    this.options = options;
    this.getSnakes = deps.getSnakes;
    this.send = deps.send;
  }

  /**
   * Update the current server tick id for timeout comparisons.
   * @param tickId - Current server tick.
   */
  setTickId(tickId: number): void {
    this.currentTickId = tickId;
  }

  /**
   * Check if a snake is currently controlled.
   * @param snakeId - Snake id to check.
   * @returns True when the snake has an active controller.
   */
  isControlled(snakeId: number): boolean {
    return this.bySnake.has(snakeId);
  }

  /**
   * Return the assigned snake id for a connection.
   * @param connId - Connection id to check.
   * @returns Assigned snake id or null.
   */
  getAssignedSnakeId(connId: number): number | null {
    return this.byConn.get(connId)?.snakeId ?? null;
  }

  /**
   * Assign a controller to a snake and notify the client.
   * @param connId - Connection id to assign.
   * @param controllerType - Controller type for the connection.
   * @param snakeId - Optional explicit snake id to assign.
   * @returns Assigned snake id or null when none available.
   */
  assignSnake(connId: number, controllerType: ControllerType, snakeId?: number): number | null {
    this.releaseSnake(connId);
    const assignedId = snakeId ?? this.pickAvailableSnake();
    if (assignedId == null) return null;
    if (!this.isSnakeAssignable(assignedId)) return null;
    const now = Date.now();
    const state: ControllerState = {
      snakeId: assignedId,
      connId,
      controllerType,
      lastTurn: 0,
      lastBoost: 0,
      lastClientTick: this.currentTickId,
      lastServerTick: this.currentTickId,
      actionTickId: this.currentTickId,
      actionsThisTick: 0,
      actionSecondStartMs: now,
      actionsThisSecond: 0,
      droppedActions: 0
    };
    this.byConn.set(connId, state);
    this.bySnake.set(assignedId, state);
    const assignMsg: AssignMsg = {
      type: 'assign',
      snakeId: assignedId,
      controller: controllerType
    };
    this.send(connId, assignMsg);
    return assignedId;
  }

  /**
   * Release the snake controlled by a connection.
   * @param connId - Connection id to release.
   */
  releaseSnake(connId: number): void {
    const state = this.byConn.get(connId);
    if (!state) return;
    this.byConn.delete(connId);
    this.bySnake.delete(state.snakeId);
  }

  /**
   * Reassign controllers whose snakes died.
   * @param spawn - Optional spawn callback for new snake ids.
   */
  reassignDeadSnakes(spawn?: () => number | null): void {
    const aliveIds = new Set<number>();
    for (const snake of this.getSnakes()) {
      if (snake.alive) aliveIds.add(snake.id);
    }
    for (const state of this.byConn.values()) {
      if (aliveIds.has(state.snakeId)) continue;
      this.bySnake.delete(state.snakeId);
      const nextId = spawn ? spawn() : this.pickAvailableSnake();
      if (nextId == null) continue;
      if (!this.isSnakeAssignable(nextId)) continue;
      const now = Date.now();
      state.snakeId = nextId;
      state.lastTurn = 0;
      state.lastBoost = 0;
      state.lastClientTick = this.currentTickId;
      state.lastServerTick = this.currentTickId;
      state.actionTickId = this.currentTickId;
      state.actionsThisTick = 0;
      state.actionSecondStartMs = now;
      state.actionsThisSecond = 0;
      state.droppedActions = 0;
      this.bySnake.set(nextId, state);
      const assignMsg: AssignMsg = {
        type: 'assign',
        snakeId: nextId,
        controller: state.controllerType
      };
      this.send(state.connId, assignMsg);
    }
  }

  /**
   * Apply an action message to the controller state, enforcing limits.
   * @param connId - Connection id sending the action.
   * @param msg - Action message payload.
   */
  handleAction(connId: number, msg: ActionMsg): void {
    const state = this.byConn.get(connId);
    if (!state) return;
    if (msg.snakeId !== state.snakeId) return;
    if (!isFiniteNumber(msg.turn) || !isFiniteNumber(msg.boost) || !isFiniteNumber(msg.tick)) return;

    // Reset per-tick counters on tick change.
    if (state.actionTickId !== this.currentTickId) {
      state.actionTickId = this.currentTickId;
      state.actionsThisTick = 0;
    }
    const overTickLimit = state.actionsThisTick >= this.options.maxActionsPerTick;

    const now = Date.now();
    if (now - state.actionSecondStartMs >= 1000) {
      state.actionSecondStartMs = now;
      state.actionsThisSecond = 0;
    }
    if (state.actionsThisSecond >= this.options.maxActionsPerSecond) {
      state.droppedActions += 1;
      return;
    }

    if (overTickLimit) {
      state.droppedActions += 1;
      state.actionsThisSecond += 1;
      return;
    }
    // Apply clamped actions and record the latest client tick.
    state.actionsThisTick += 1;
    state.actionsThisSecond += 1;
    state.lastTurn = clamp(msg.turn, -1, 1);
    state.lastBoost = clamp(msg.boost, 0, 1);
    state.lastClientTick = msg.tick;
    state.lastServerTick = this.currentTickId;
  }

  /**
   * Fetch the current action for a snake, handling timeouts and releases.
   * @param snakeId - Snake id to query.
   * @param tickId - Current tick id.
   * @returns Control input or null when control is released.
   */
  getAction(snakeId: number, tickId: number): ControlInput | null {
    const state = this.bySnake.get(snakeId);
    if (!state) return null;
    const delta = tickId - state.lastServerTick;
    if (!Number.isFinite(delta) || delta <= 0) {
      return { turn: state.lastTurn, boost: state.lastBoost };
    }
    if (delta <= this.options.actionTimeoutTicks) {
      return { turn: state.lastTurn, boost: state.lastBoost };
    }
    const releaseAfter = this.options.actionTimeoutTicks * 2;
    if (delta > releaseAfter) {
      this.releaseSnake(state.connId);
      return null;
    }
    return { turn: 0, boost: 0 };
  }

  /**
   * Publish sensor data to a controlling client.
   * @param snakeId - Snake id owning the sensors.
   * @param tickId - Tick id for the sensor sample.
   * @param sensors - Sensor values.
   * @param meta - Optional metadata for UI overlays.
   */
  publishSensors(
    snakeId: number,
    tickId: number,
    sensors: Float32Array,
    meta: { x: number; y: number; dir: number }
  ): void {
    const state = this.bySnake.get(snakeId);
    if (!state) return;
    const msg: SensorsMsg = {
      type: 'sensors',
      tick: tickId,
      snakeId,
      sensors: Array.from(sensors),
      meta
    };
    this.send(state.connId, msg);
  }

  /**
   * Pick an available, alive snake id that is not already controlled.
   * @returns Snake id or null when none available.
   */
  private pickAvailableSnake(): number | null {
    const snakes = this.getSnakes();
    for (const snake of snakes) {
      if (!snake.alive) continue;
      if (this.bySnake.has(snake.id)) continue;
      return snake.id;
    }
    return null;
  }

  /**
   * Check if a specific snake id can be assigned to a controller.
   * @param snakeId - Snake id to validate.
   * @returns True when the snake is alive and not already controlled.
   */
  private isSnakeAssignable(snakeId: number): boolean {
    if (this.bySnake.has(snakeId)) return false;
    const snakes = this.getSnakes();
    return snakes.some((snake) => snake.id === snakeId && snake.alive);
  }
}
