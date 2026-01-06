import { describe, it, expect } from 'vitest';
import { ControllerRegistry } from './controllerRegistry.ts';

describe('ControllerRegistry', () => {
  const makeRegistry = () => {
    const snakes = [
      { id: 1, alive: true },
      { id: 2, alive: true }
    ];
    const sent: Array<{ connId: number; payload: unknown }> = [];
    const registry = new ControllerRegistry(
      {
        actionTimeoutTicks: 2,
        maxActionsPerTick: 2,
        maxActionsPerSecond: 100
      },
      {
        getSnakes: () => snakes,
        send: (connId, payload) => {
          sent.push({ connId, payload });
        }
      }
    );
    return { registry, sent, snakes };
  };

  it('assigns a snake and sends assign message', () => {
    const { registry, sent } = makeRegistry();
    registry.setTickId(1);
    const snakeId = registry.assignSnake(7, 'player');

    expect(snakeId).toBe(1);
    expect(registry.isControlled(1)).toBe(true);
    expect(sent.length).toBe(1);
    expect((sent[0]?.payload as { type?: string }).type).toBe('assign');
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

  it('drops actions beyond maxActionsPerTick', () => {
    const snakes = [{ id: 1, alive: true }];
    const registry = new ControllerRegistry(
      {
        actionTimeoutTicks: 2,
        maxActionsPerTick: 1,
        maxActionsPerSecond: 100
      },
      {
        getSnakes: () => snakes,
        send: () => {}
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
    expect(action?.turn).toBe(-0.4);
    expect(action?.boost).toBe(0);
  });

  it('returns neutral action after timeout and releases on extended timeout', () => {
    const { registry } = makeRegistry();
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

    const fresh = registry.getAction(snakeId, 2);
    expect(fresh?.turn).toBe(0.2);

    const neutral = registry.getAction(snakeId, 4);
    expect(neutral).toEqual({ turn: 0, boost: 0 });
    expect(registry.isControlled(snakeId)).toBe(true);

    const released = registry.getAction(snakeId, 6);
    expect(released).toBeNull();
    expect(registry.isControlled(snakeId)).toBe(false);
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
});
