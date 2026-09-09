import { describe, expect, it } from 'vitest';
import type { ExperimentalRunningAuthorityNativeHandle } from './backgroundRuntime.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';
import { ExternalControllerRouting } from './externalRouting.ts';

/** Capacity-controlled native queue with observable accepted commands only. */
function fixture() {
  let available = true;
  const commands: Array<{ kind: string; sequence: string; value: unknown }> = [];
  const packets: unknown[] = [];
  /** Match the native rule that a rejected admission consumes no command identity. */
  const submit = (kind: string, sequence: string, value: unknown): void => {
    if (!available) throw new Error('QueueCountLimit: full');
    commands.push({ kind, sequence, value });
  };
  const native = {
    submitControllerReclaim(sequence: string, value: unknown) { submit('reclaim', sequence, value); },
    submitControllerJoin(sequence: string, value: unknown) { submit('join', sequence, value); },
    submitControllerAction(sequence: string, value: unknown) { submit('action', sequence, value); },
    submitControllerDisconnect(sequence: string, value: unknown) { submit('close', sequence, value); },
    submitControllerDeliveryReceipt() {}, submitControllerReclaimReceipt() {}, submitControllerJoinReceipt() {}, submitGenerationAssignmentReceipt() {},
    health() { return { completedStep: '0000000000000000' }; }
  } as unknown as ExperimentalRunningAuthorityNativeHandle;
  const routing = new ExternalControllerRouting({ native, admission: new BackgroundCommandAdmission(native), maxControllers: 1,
    maxActionsPerSecond: 120, maxActionsPerTick: 1, send(_connection, packet) { packets.push(packet); return true; } });
  return { routing, commands, packets, capacity(value: boolean) { available = value; } };
}

describe('bounded external socket routing', () => {
  it('retains one fresh join, newest player input, and exact close under queue pressure', () => {
    const { routing, commands, capacity } = fixture();
    capacity(false);
    routing.join(1, { type: 'join', mode: 'player', name: 'player' }, 'ui');
    expect(commands).toHaveLength(0);
    capacity(true); routing.flush();
    routing.event({ kind: 'commandRejected', commandSequence: '0000000000000001', rejectionCode: 'InvalidCommand',
      rejectionDetail: 'InvalidCommand: no reserved legacy identity match' });
    expect(commands.map(command => command.kind)).toEqual(['reclaim', 'join']);
    routing.event({ kind: 'controllerJoinAssignment', controllerJoinAssignment: {
      requestSequence: '0000000000000002', connectionId: '0000000000000001', leaseId: '0000000000000009',
      snakeId: 12, controllerKind: 'player', resumeToken: 'a'.repeat(32), completedStep: '0000000000000000'
    } });
    routing.event({ kind: 'controllerJoinResolved', controllerJoinResolution: { requestSequence: '0000000000000002', matched: true, accepted: true } });
    capacity(false);
    routing.action(1, { type: 'action', snakeId: 12, tick: 10, turn: 1, boost: 1 });
    routing.action(1, { type: 'action', snakeId: 12, tick: 11, turn: -0.5, boost: 0 });
    routing.action(1, { type: 'action', snakeId: 99, tick: 12, turn: 0, boost: 1 });
    capacity(true); routing.flush();
    expect(commands[2]).toMatchObject({ kind: 'action', sequence: '0000000000000003', value: {
      leaseId: '0000000000000009', clientTick: '000000000000000b', turn: -0.5, boost: false
    } });
    capacity(false); routing.disconnect(1); routing.flush();
    expect(commands).toHaveLength(3);
    capacity(true); routing.flush(); routing.flush();
    expect(commands).toHaveLength(4);
    expect(commands[3]).toMatchObject({ kind: 'close', sequence: '0000000000000004', value: { leaseId: '0000000000000009' } });
  });

  it('bounds unresolved routes and never converts an explicit invalid token to a fresh join', () => {
    const { routing, commands, packets } = fixture();
    routing.join(1, { type: 'join', mode: 'player', name: 'bot', resumeToken: 'a'.repeat(32) }, 'bot');
    routing.join(2, { type: 'join', mode: 'player', name: 'other' }, 'bot');
    expect(packets).toContainEqual({ type: 'error', message: 'experimental controller capacity reached' });
    routing.event({ kind: 'commandRejected', commandSequence: '0000000000000001', rejectionCode: 'InvalidCommand',
      rejectionDetail: 'InvalidCommand: no reserved legacy identity match' });
    expect(commands.map(command => command.kind)).toEqual(['reclaim']);
    expect(packets).toContainEqual({ type: 'reclaimResult', reclaimed: false, reason: 'invalid' });
  });
});
