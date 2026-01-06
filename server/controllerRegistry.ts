import type { ControlInput } from '../src/snake.ts';
import type { ActionMsg, AssignMsg, SensorsMsg, ServerMessage } from './protocol.ts';

export type ControllerType = 'player' | 'bot';

export interface ControllerRegistryOptions {
  actionTimeoutTicks: number;
  maxActionsPerTick: number;
  maxActionsPerSecond: number;
}

export interface ControllerRegistryDeps {
  getSnakes: () => Array<{ id: number; alive: boolean }>;
  send: (connId: number, payload: ServerMessage) => void;
}

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

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export class ControllerRegistry {
  private bySnake = new Map<number, ControllerState>();
  private byConn = new Map<number, ControllerState>();
  private currentTickId = 0;
  private options: ControllerRegistryOptions;
  private getSnakes: ControllerRegistryDeps['getSnakes'];
  private send: ControllerRegistryDeps['send'];

  constructor(options: ControllerRegistryOptions, deps: ControllerRegistryDeps) {
    this.options = options;
    this.getSnakes = deps.getSnakes;
    this.send = deps.send;
  }

  setTickId(tickId: number): void {
    this.currentTickId = tickId;
  }

  isControlled(snakeId: number): boolean {
    return this.bySnake.has(snakeId);
  }

  getAssignedSnakeId(connId: number): number | null {
    return this.byConn.get(connId)?.snakeId ?? null;
  }

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

  releaseSnake(connId: number): void {
    const state = this.byConn.get(connId);
    if (!state) return;
    this.byConn.delete(connId);
    this.bySnake.delete(state.snakeId);
  }

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

  handleAction(connId: number, msg: ActionMsg): void {
    const state = this.byConn.get(connId);
    if (!state) return;
    if (msg.snakeId !== state.snakeId) return;
    if (!isFiniteNumber(msg.turn) || !isFiniteNumber(msg.boost) || !isFiniteNumber(msg.tick)) return;

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
    state.actionsThisTick += 1;
    state.actionsThisSecond += 1;
    state.lastTurn = clamp(msg.turn, -1, 1);
    state.lastBoost = clamp(msg.boost, 0, 1);
    state.lastClientTick = msg.tick;
    state.lastServerTick = this.currentTickId;
  }

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

  private pickAvailableSnake(): number | null {
    const snakes = this.getSnakes();
    for (const snake of snakes) {
      if (!snake.alive) continue;
      if (this.bySnake.has(snake.id)) continue;
      return snake.id;
    }
    return null;
  }

  private isSnakeAssignable(snakeId: number): boolean {
    if (this.bySnake.has(snakeId)) return false;
    const snakes = this.getSnakes();
    return snakes.some(snake => snake.id === snakeId && snake.alive);
  }
}
