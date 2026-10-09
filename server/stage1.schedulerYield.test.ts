import { performance } from 'node:perf_hooks';
import { beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { SimCore } from '../src/sim/SimCore.ts';
import { DEFAULT_CONFIG } from './config.ts';
import type { ControllerRegistry } from './controllerRegistry.ts';
import type { ServerMessage, WelcomeMsg } from './protocol.ts';
import { SimServer } from './simServer.ts';
import type { WsHub } from './wsHub.ts';

/** Private server fields used only by deterministic scheduler correction tests. */
interface SchedulerServerAccess {
  /** Unified fixed-step scheduler. */
  core: SimCore;
  /** Whether startup runs asynchronous MT initialization. */
  mtEnabled: boolean;
  /** Replaceable initialization method for a startup-delay fixture. */
  initMT: () => Promise<void>;
  /** Suppressed loop launcher for isolated startup-clock inspection. */
  startLoopIteration: () => void;
  /** Previous scheduler-pump wall timestamp. */
  lastTickAt: number;
  /** Controller ownership registry. */
  controllers: ControllerRegistry;
}

/**
 * Build a fake hub that captures direct controller lifecycle messages.
 * @returns Hub and captured direct messages.
 */
function buildHub(): { hub: WsHub; direct: ServerMessage[] } {
  const direct: ServerMessage[] = [];
  const hub = {
    sendJsonTo: (_connId: number, message: ServerMessage) => {
      direct.push(message);
      return true;
    },
    broadcastJsonToUi: () => undefined,
    updateWelcome: (_patch: Partial<WelcomeMsg>) => undefined,
    updateSensorSpec: () => undefined,
    broadcastError: () => undefined,
    hasFrameRecipients: () => false,
    broadcastFrame: () => undefined,
    broadcastStats: () => undefined
  } as unknown as WsHub;
  return { hub, direct };
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
  CFG.generationSeconds = 10_000;
  CFG.observer.earlyEndAliveThreshold = -1;
});

describe('Stage 1 scheduler servicing corrections', () => {
  it('resets the scheduler clock after asynchronous startup initialization', async () => {
    const { hub } = buildHub();
    const server = new SimServer(
      { ...DEFAULT_CONFIG, inferenceBackend: 'js', mtEnabled: false },
      hub,
      undefined,
      '',
      9301,
      { snakeCount: 1 },
      'startup-clock'
    );
    const access = server as unknown as SchedulerServerAccess;
    access.mtEnabled = true;
    access.startLoopIteration = () => undefined;
    access.initMT = async () => {
      await new Promise<void>((resolve) => setTimeout(resolve, 25));
    };
    const before = performance.now();

    await server.start();

    expect(access.lastTickAt).toBeGreaterThanOrEqual(before + 20);
    await server.stop();
  });

  it('services Node between interactive catch-up steps so fresh input reaches step two', async () => {
    const { hub, direct } = buildHub();
    const server = new SimServer(
      {
        ...DEFAULT_CONFIG,
        inferenceBackend: 'js',
        mtEnabled: false,
        maxActionsPerTick: 1
      },
      hub,
      undefined,
      '',
      9302,
      { snakeCount: 1 },
      'catch-up-input'
    );
    const access = server as unknown as SchedulerServerAccess;
    server.handleJoin(1, 'player', 'ui', 'catch-up-player');
    const assignment = direct.find(message => message.type === 'assign');
    if (!assignment || assignment.type !== 'assign') throw new Error('missing assignment');
    const assignedSnake = access.core.world.snakes.find(
      candidate => candidate.id === assignment.snakeId
    );
    if (!assignedSnake) throw new Error('assigned snake is missing');
    for (const snake of access.core.world.snakes) {
      if (snake !== assignedSnake) snake.alive = false;
    }
    assignedSnake.x = 0;
    assignedSnake.y = 0;
    assignedSnake.points.forEach((point, index) => {
      point.x = -index * CFG.snakeSpacing;
      point.y = 0;
    });
    server.handleAction(1, {
      type: 'action',
      tick: 0,
      snakeId: assignment.snakeId,
      turn: -0.7,
      boost: 0
    });
    expect(access.controllers.isControlled(assignment.snakeId)).toBe(true);
    expect(access.controllers.getAction(assignment.snakeId)).toEqual({
      turn: -0.7,
      boost: 0
    });

    const update = access.core.update(access.core.fixedDt * 2, access.controllers);
    setImmediate(() => {
      server.handleAction(1, {
        type: 'action',
        tick: 2,
        snakeId: assignment.snakeId,
        turn: 0.85,
        boost: 1
      });
    });
    await update;

    expect(access.core.tickId).toBe(2);
    expect(assignedSnake.turnInput).toBeCloseTo(0.85, 6);
    expect(assignedSnake.boostInput).toBe(1);
  });
});
