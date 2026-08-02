import { describe, it, expect } from 'vitest';
import { ControllerRegistry } from './controllerRegistry.ts';

describe('ControllerRegistry', () => {
  /**
   * Builds a registry with a simple snake list and capture buffer.
   * @returns Registry and shared test buffers.
   */
  const makeRegistry = () => {
    const snakes = [
      { id: 1, alive: true, controllable: true },
      { id: 2, alive: true, controllable: true }
    ];
    const sent: Array<{ connId: number; payload: unknown }> = [];
    let now = 1_000;
    let token = 0;
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 2,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: (connId, payload) => {
          sent.push({ connId, payload });
          return true;
        },
        nowMs: () => now,
        createResumeToken: () => `token-${++token}`,
        getLeaseScope: () => 'session:run'
      }
    );
    return {
      registry,
      sent,
      snakes,
      advanceWallTime: (elapsedMs: number) => {
        now += elapsedMs;
      }
    };
  };

  it('assigns a snake and sends assign message', () => {
    const { registry, sent } = makeRegistry();
    registry.setTickId(1);
    const snakeId = registry.assignSnake(7, 'player');

    expect(snakeId).toBe(1);
    expect(registry.isControlled(1)).toBe(true);
    expect(sent.length).toBe(1);
    expect((sent[0]?.payload as { type?: string }).type).toBe('assign');
    expect((sent[0]?.payload as { resumeToken?: string }).resumeToken).toBe('token-1');
  });

  it('does not claim a snake when its token-bearing assignment is rejected', () => {
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 2,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => [{ id: 1, alive: true, controllable: true }],
        send: () => false,
        createResumeToken: () => 'undelivered-token'
      }
    );

    expect(registry.assignSnake(7, 'player')).toBeNull();
    expect(registry.getAssignedSnakeId(7)).toBeNull();
    expect(registry.isControlled(1)).toBe(false);
  });

  it('uses the latest action within a tick when allowed', () => {
    const { registry } = makeRegistry();
    registry.setTickId(5);
    const snakeId = registry.assignSnake(3, 'bot');
    expect(snakeId).toBe(1);
    if (!snakeId) return;
    registry.handleAction(3, {
      type: 'action',
      tick: 5,
      snakeId,
      turn: -0.6,
      boost: 0
    });
    registry.handleAction(3, {
      type: 'action',
      tick: 5,
      snakeId,
      turn: 0.7,
      boost: 1
    });
    const action = registry.getAction(snakeId, 5);
    expect(action?.turn).toBe(0.7);
    expect(action?.boost).toBe(1);
  });

  it('coalesces browser-player actions beyond maxActionsPerTick to the latest value', () => {
    const snakes = [{ id: 1, alive: true, controllable: true }];
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 1,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: () => true
      }
    );
    registry.setTickId(5);
    const snakeId = registry.assignSnake(11, 'player');
    expect(snakeId).toBe(1);
    if (!snakeId) return;
    registry.handleAction(11, {
      type: 'action',
      tick: 5,
      snakeId,
      turn: -0.4,
      boost: 0
    });
    registry.handleAction(11, {
      type: 'action',
      tick: 5,
      snakeId,
      turn: 0.9,
      boost: 1
    });
    const action = registry.getAction(snakeId, 5);
    expect(action?.turn).toBe(0.9);
    expect(action?.boost).toBe(1);
  });

  it('retains the existing per-tick action limit for observation-driven RL bots', () => {
    const snakes = [{ id: 1, alive: true, controllable: true }];
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 1,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: () => true
      }
    );
    registry.setTickId(5);
    const snakeId = registry.assignSnake(11, 'bot');
    expect(snakeId).toBe(1);
    if (!snakeId) return;
    registry.handleAction(11, {
      type: 'action',
      tick: 5,
      snakeId,
      turn: -0.4,
      boost: 0
    });
    registry.handleAction(11, {
      type: 'action',
      tick: 5,
      snakeId,
      turn: 0.9,
      boost: 1
    });

    expect(registry.getAction(snakeId, 5)).toEqual({ turn: -0.4, boost: 0 });
  });

  it('holds the last action for 500 ms then uses neutral input without releasing ownership', () => {
    const { registry, advanceWallTime } = makeRegistry();
    registry.setTickId(1);
    const snakeId = registry.assignSnake(4, 'player');
    expect(snakeId).toBe(1);
    if (!snakeId) return;
    registry.handleAction(4, {
      type: 'action',
      tick: 1,
      snakeId,
      turn: 0.2,
      boost: 1
    });

    advanceWallTime(499);
    const fresh = registry.getAction(snakeId, 2);
    expect(fresh?.turn).toBe(0.2);

    advanceWallTime(1);
    const stale = registry.getAction(snakeId, 4);
    expect(stale).toEqual({ turn: 0, boost: 0 });
    expect(registry.isControlled(snakeId)).toBe(true);

    registry.releaseSnake(4);
    const released = registry.getAction(snakeId, 6);
    expect(released).toBeNull();
    expect(registry.isControlled(snakeId)).toBe(false);
  });

  it('keeps disconnected ownership neutral for 30 seconds then releases to neural once', () => {
    const { registry, advanceWallTime, sent } = makeRegistry();
    const snakeId = registry.assignSnake(4, 'player', undefined, 'player:alice');
    expect(snakeId).toBe(1);
    if (!snakeId) return;
    registry.handleAction(4, {
      type: 'action',
      tick: 1,
      snakeId,
      turn: 0.8,
      boost: 1
    });

    registry.disconnectConnection(4);
    expect(registry.getAction(snakeId)).toEqual({ turn: 0, boost: 0 });
    expect(registry.publishSensors(
      snakeId,
      2,
      Float32Array.of(0.25),
      { x: 0, y: 0, dir: 0 }
    )).toBe(false);
    expect(sent).toHaveLength(1);
    advanceWallTime(29_999);
    expect(registry.isControlled(snakeId)).toBe(true);
    expect(registry.getAction(snakeId)).toEqual({ turn: 0, boost: 0 });

    advanceWallTime(1);
    registry.refresh();
    expect(registry.getAction(snakeId)).toBeNull();
    expect(registry.isControlled(snakeId)).toBe(false);
    expect(registry.getAction(snakeId)).toBeNull();
  });

  it('enters reclaimable neutral grace when a live sensor enqueue is rejected', () => {
    const snakes = [{ id: 1, alive: true, controllable: true }];
    const sent: Array<{ connId: number; payload: unknown }> = [];
    let acceptSend = true;
    let token = 0;
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 1,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: (connId, payload) => {
          sent.push({ connId, payload });
          return acceptSend;
        },
        createResumeToken: () => `rejected-send-${++token}`,
        getLeaseScope: () => 'session:run'
      }
    );
    const snakeId = registry.assignSnake(4, 'bot', undefined, 'bot:trainer');
    expect(snakeId).toBe(1);
    if (!snakeId) return;
    const resumeToken = (sent[0]?.payload as { resumeToken?: string }).resumeToken;
    registry.handleAction(4, {
      type: 'action',
      tick: 1,
      snakeId,
      turn: 0.8,
      boost: 1
    });
    acceptSend = false;

    expect(registry.publishSensors(
      snakeId,
      2,
      Float32Array.of(0.5),
      { x: 1, y: 2, dir: 0.25 }
    )).toBe(false);
    expect(registry.getAssignedSnakeId(4)).toBeNull();
    expect(registry.isControlled(snakeId)).toBe(true);
    expect(registry.getAction(snakeId)).toEqual({ turn: 0, boost: 0 });

    acceptSend = true;
    expect(registry.reclaimSnake(8, 'bot', resumeToken, 'bot:trainer')).toMatchObject({
      reclaimed: true,
      reason: 'reclaimed',
      snakeId
    });
  });

  it('reclaims the same live snake by token during grace and rotates the token', () => {
    const { registry, sent, advanceWallTime } = makeRegistry();
    const snakeId = registry.assignSnake(4, 'player', undefined, 'player:alice');
    const firstToken = (sent[0]?.payload as { resumeToken?: string }).resumeToken;
    expect(firstToken).toBe('token-1');
    registry.disconnectConnection(4);
    advanceWallTime(10_000);

    const result = registry.reclaimSnake(8, 'player', firstToken, 'player:alice');

    expect(result).toEqual({ reclaimed: true, reason: 'reclaimed', snakeId });
    expect(registry.getAssignedSnakeId(8)).toBe(snakeId);
    const reassignment = sent.at(-1)?.payload as { type?: string; resumeToken?: string; reclaimed?: boolean };
    expect(reassignment).toMatchObject({
      type: 'assign',
      resumeToken: 'token-2',
      reclaimed: true
    });
    expect(registry.reclaimSnake(9, 'player', firstToken, 'player:alice').reason).toBe('invalid');
  });

  it('keeps the client-known token usable when either reclaim enqueue is rejected', () => {
    const snakes = [{ id: 1, alive: true, controllable: true }];
    const sendResults: boolean[] = [];
    const sent: unknown[] = [];
    let token = 0;
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 2,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: (_connId, payload) => {
          sent.push(payload);
          return sendResults.shift() ?? true;
        },
        createResumeToken: () => `recoverable-token-${++token}`,
        getLeaseScope: () => 'session:run'
      }
    );
    const snakeId = registry.assignSnake(4, 'player', undefined, 'player:alice');
    expect(snakeId).toBe(1);
    const firstToken = (sent[0] as { resumeToken?: string }).resumeToken;
    expect(firstToken).toBe('recoverable-token-1');
    registry.disconnectConnection(4);

    sendResults.push(false);
    expect(registry.reclaimSnake(8, 'player', firstToken, 'player:alice')).toEqual({
      reclaimed: false,
      reason: 'delivery-failed'
    });
    expect(registry.getAssignedSnakeId(8)).toBeNull();
    expect(registry.isControlled(1)).toBe(true);

    sendResults.push(true, true);
    expect(registry.reclaimSnake(8, 'player', firstToken, 'player:alice')).toMatchObject({
      reclaimed: true,
      reason: 'reclaimed',
      snakeId
    });
    const secondToken = (sent.at(-1) as { resumeToken?: string }).resumeToken;
    expect(secondToken).toBe('recoverable-token-3');
    registry.disconnectConnection(8);

    sendResults.push(true, false);
    expect(registry.reclaimSnake(9, 'player', secondToken, 'player:alice')).toEqual({
      reclaimed: false,
      reason: 'delivery-failed'
    });
    expect(registry.getAssignedSnakeId(9)).toBeNull();
    expect(registry.isControlled(1)).toBe(true);

    sendResults.push(true, true);
    expect(registry.reclaimSnake(10, 'player', secondToken, 'player:alice')).toMatchObject({
      reclaimed: true,
      reason: 'reclaimed',
      snakeId
    });
  });

  it('reports an expired token instead of silently assigning a different snake', () => {
    const { registry, sent, advanceWallTime } = makeRegistry();
    registry.assignSnake(4, 'bot', undefined, 'bot:trainer');
    const token = (sent[0]?.payload as { resumeToken?: string }).resumeToken;
    registry.disconnectConnection(4);
    advanceWallTime(30_000);
    registry.refresh();

    expect(registry.reclaimSnake(8, 'bot', token, 'bot:trainer')).toEqual({
      reclaimed: false,
      reason: 'expired'
    });
  });

  it('supports one unambiguous legacy identity reclaim without a token', () => {
    const { registry, advanceWallTime } = makeRegistry();
    const snakeId = registry.assignSnake(4, 'bot', undefined, 'bot:trainer');
    registry.disconnectConnection(4);
    advanceWallTime(1_000);

    expect(registry.reclaimSnake(8, 'bot', undefined, 'bot:trainer')).toMatchObject({
      reclaimed: true,
      reason: 'reclaimed',
      snakeId
    });
  });

  it('reassigns a controller when the assigned snake dies', () => {
    const { registry, sent, snakes } = makeRegistry();
    registry.setTickId(1);
    const snakeId = registry.assignSnake(9, 'player');
    expect(snakeId).toBe(1);
    snakes[0]!.alive = false;
    registry.reassignDeadSnakes();
    expect(registry.isControlled(1)).toBe(false);
    expect(registry.isControlled(2)).toBe(true);
    const assigns = sent.filter(entry => (entry.payload as { type?: string }).type === 'assign');
    expect(assigns.length).toBe(2);
  });

  it('releases a dead-snake reassignment whose new token cannot be delivered', () => {
    const snakes = [
      { id: 1, alive: true, controllable: true },
      { id: 2, alive: true, controllable: true }
    ];
    let acceptSend = true;
    let token = 0;
    const sent: unknown[] = [];
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 2,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: (_connId, payload) => {
          sent.push(payload);
          return acceptSend;
        },
        createResumeToken: () => `dead-reassign-${++token}`
      }
    );
    expect(registry.assignSnake(9, 'player')).toBe(1);
    const firstToken = (sent[0] as { resumeToken?: string }).resumeToken;
    snakes[0]!.alive = false;
    acceptSend = false;

    registry.reassignDeadSnakes();

    expect(registry.getAssignedSnakeId(9)).toBeNull();
    expect(registry.isControlled(1)).toBe(false);
    expect(registry.isControlled(2)).toBe(false);
    expect(registry.reclaimSnake(10, 'player', firstToken)).toEqual({
      reclaimed: false,
      reason: 'expired'
    });
  });

  it('skips non-controllable snakes', () => {
    const snakes = [
      { id: 1, alive: true, controllable: false },
      { id: 2, alive: true, controllable: true }
    ];
    const registry = new ControllerRegistry(
      {
        maxActionsPerTick: 2,
        maxActionsPerSecond: 100,
        inputHoldMs: 500,
        disconnectGraceMs: 30_000
      },
      {
        getSnakes: () => snakes,
        send: () => true
      }
    );
    registry.setTickId(1);
    const snakeId = registry.assignSnake(5, 'player');
    expect(snakeId).toBe(2);
  });
});
