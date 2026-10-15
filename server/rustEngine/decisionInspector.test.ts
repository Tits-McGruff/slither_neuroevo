import { describe, expect, it, vi } from 'vitest';
import { DecisionInspectorRouting } from './decisionInspector.ts';
import type { InspectorMsg, InspectorResultMsg } from '../../src/protocol/inspector.ts';

/** Queue/socket metadata doubles; no simulation or graph runs in JavaScript. */
function fixture() {
  const reliable: Array<{connection: number; result: InspectorResultMsg}> = [];
  const rows: Array<{connection: number; message: InspectorMsg}> = [];
  const commands: Array<{sequence: string; run: string; token?: string; frame?: number; pureProbe?: boolean}> = [];
  const scope = {sessionId: 'session', runId: 'run', available: true};
  const state = {next: 1n, capacity: true, json: null as string | null, send: true};
  const read = vi.fn(() => state.json);
  const router = new DecisionInspectorRouting({runtime: () => ({latestInspector: read,
    submitInspector(sequence, run, token, frame, pureProbe) {commands.push({sequence, run,
      ...(token === undefined ? {} : {token}), ...(frame === undefined ? {} : {frame}), ...(pureProbe === undefined ? {} : {pureProbe})});}}),
    scope: () => scope, admission: {trySubmitControl(submit) {
      if (!state.capacity) return false;
      submit((state.next++).toString(16).padStart(16, '0')); return true;
    }}, reliable(connection, result) {reliable.push({connection, result}); return true;},
    replaceable(connection, message) {rows.push({connection, message}); return state.send;}, clear: vi.fn(), schedule: vi.fn()});
  /** Current-scope request, optionally enabling one exact selected physical target. */
  const request = (connection = 1, enabled = true): void => router.request(connection, {
    type: 'inspectEntity', requestId: 'request-' + connection, expectedSessionId: 'session', expectedRunId: scope.runId,
    ...(enabled ? {selectionToken: '1'.repeat(64), snakeId: 3} : {})});
  /** One acknowledged native command. */
  const ack = (index: number, enabled: boolean): void => router.event({kind: 'inspectorChanged',
    commandSequence: commands[index]!.sequence, inspectorEnabled: enabled});
  return {router, reliable, rows, commands, scope, state, read, request, ack};
}

/** Minimal valid native retirement projection; identity filtering applies to clears too. */
function clear(sequence: number, subscription = 1): string {
  return JSON.stringify({schemaVersion: 1, sequence: sequence.toString(16).padStart(16, '0'),
    subscriptionSequence: subscription.toString(16).padStart(16, '0'), worldEpoch: '0000000000000001',
    completedStep: '0000000000000001', sample: null});
}

describe('selected inspector routing', () => {
  it('keeps closed capture idle, bounds pending/owner admission and filters old subscriptions', () => {
    const f = fixture(); f.router.service(); expect(f.read).not.toHaveBeenCalled();
    f.request(); f.request(2); expect(f.commands).toHaveLength(1);
    expect(f.reliable.at(-1)?.result.applied).toBe(false);
    f.ack(0, true); f.request(2); expect(f.commands).toHaveLength(1);
    f.state.json = clear(1, 99); f.router.service(); expect(f.rows).toHaveLength(0);
    f.state.json = clear(2); f.router.service();
    expect(f.rows).toHaveLength(1); expect(f.rows[0]?.connection).toBe(1);
    f.request(1, false); f.ack(1, false);
    f.router.service(); expect(f.read).toHaveBeenCalledTimes(2);
    f.request(2); expect(f.commands).toHaveLength(3);
  });

  it('disables a disconnected in-flight enable before admitting a new viewer, including queue backpressure', () => {
    const f = fixture(); f.request(); f.router.disconnect(1); f.ack(0, true);
    f.state.capacity = false; f.router.service(); f.request(2);
    expect(f.commands).toHaveLength(1);
    f.state.capacity = true; f.router.service();
    expect(f.commands[1]).toEqual({sequence: '0000000000000002', run: 'run'});
    f.ack(1, false); f.request(2); expect(f.commands).toHaveLength(3);
  });

  it('retains cleanup of a pending subscription across scope replacement', () => {
    const f = fixture(); f.request(); f.scope.runId = 'new-run'; f.router.reset();
    expect(f.reliable.at(-1)?.result.enabled).toBe(false);
    f.ack(0, true); f.router.service();
    expect(f.commands[1]).toMatchObject({run: 'new-run'});
    f.ack(1, false); f.request(2); expect(f.commands[2]).toMatchObject({run: 'new-run'});
  });

  it('isolates malformed projections and failed socket delivery from gameplay', () => {
    for (const invalid of [true, false]) {
      const f = fixture(); f.request(); f.ack(0, true);
      f.state.json = invalid ? 'x'.repeat(65537) : clear(1); f.state.send = false;
      expect(() => f.router.service()).not.toThrow();
      f.router.service(); expect(f.commands[1]).toEqual({sequence: '0000000000000002', run: 'run'});
    }
  });

  it('rejects stale scope and preserves an existing owner when a target change is rejected', () => {
    const f = fixture(); f.scope.available = false; f.request(); expect(f.commands).toHaveLength(0);
    f.scope.available = true; f.request(); f.ack(0, true); f.request();
    f.router.event({kind: 'commandRejected', commandSequence: f.commands[1]!.sequence, rejectionDetail: 'stale token'});
    f.state.json = clear(1); f.router.service(); expect(f.rows).toHaveLength(1);
  });

  it('routes an explicit probe through the same exact-token exclusive admission', () => {
    const f = fixture(); f.router.request(1, {type: 'inspectEntity', requestId: 'probe', expectedSessionId: 'session',
      expectedRunId: 'run', selectionToken: '1'.repeat(64), snakeId: 3, pureProbe: true});
    expect(f.commands[0]).toMatchObject({pureProbe: true, token: '1'.repeat(64), frame: 3});
    f.ack(0, true); f.request(2); expect(f.commands).toHaveLength(1);
    f.router.disconnect(1); f.router.service(); expect(f.commands[1]?.token).toBeUndefined();
  });

});
