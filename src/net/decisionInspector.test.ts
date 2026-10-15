import { afterEach, describe, expect, it, vi } from 'vitest';
import { DecisionInspectorClient } from './decisionInspector.ts';
import type { InspectEntityMsg, InspectorSnapshot } from '../protocol/inspector.ts';
import type { SelectedEntity } from '../protocol/entitySelection.ts';
import { inspectorFixture } from '../test/inspectorFixture.ts';

/** Bounded current native selection, without interpreting its token. */
const SELECTED: SelectedEntity = {token: '1'.repeat(64), snakeId: 3, populationSlot: 0, x: 0, y: 0, radius: 9, skin: 0};
/** Metadata-only transport fixture. */
function fixture() {
  const sent: InspectEntityMsg[] = []; const shown: Array<InspectorSnapshot | null> = []; const status: string[] = [];
  const client = new DecisionInspectorClient({send: value => sent.push(value), changed: value => shown.push(value), status: value => status.push(value)});
  client.context({sessionId: 'session', runId: 'run', supported: true}); client.selection(SELECTED);
  /** Acknowledge one actual client intent using a native-shaped sequence. */
  const ack = (index: number, enabled: boolean, sequence = '0000000000000001'): void => client.result({type: 'inspectorResult',
    requestId: sent[index]!.requestId, applied: true, enabled, subscriptionSequence: sequence});
  return {client, sent, shown, status, ack};
}

afterEach(() => { vi.useRealTimers(); });

describe('selected decision inspector client', () => {
  it('closes unknown native ownership when an admission result has an invalid identity', () => {
    vi.useFakeTimers(); const f = fixture(); f.client.setEnabled(true);
    f.client.result({type: 'inspectorResult', requestId: f.sent[0]!.requestId, applied: true,
      enabled: true, subscriptionSequence: 'invalid'});
    vi.advanceTimersByTime(250); expect(f.sent[1]?.selectionToken).toBeUndefined();
    expect(f.sent).toHaveLength(2); f.ack(1, false, '0000000000000002');
    vi.advanceTimersByTime(1000); expect(f.sent).toHaveLength(2); f.client.dispose();
  });

  it('stays off until explicit opt-in, accepts only acknowledged snapshots and closes immediately on opt-out', () => {
    const f = fixture(); expect(f.sent).toHaveLength(0);
    f.client.setEnabled(true); expect(f.sent[0]).toMatchObject({selectionToken: SELECTED.token, snakeId: 3});
    const row = inspectorFixture(); f.client.snapshot(row); expect(f.shown.at(-1)).toBeNull();
    f.ack(0, true); f.client.snapshot(row); expect(f.shown.at(-1)).toBe(row);
    f.client.setEnabled(false); expect(f.sent[1]?.selectionToken).toBeUndefined(); expect(f.shown.at(-1)).toBeNull();
    f.client.snapshot({...row, sequence: '0000000000000002'}); expect(f.shown.at(-1)).toBeNull();
    f.ack(1, false, '0000000000000002'); expect(f.sent).toHaveLength(2); f.client.dispose();
  });

  it('coalesces changing selection and opt-out while an enable is in flight', () => {
    const f = fixture(); f.client.setEnabled(true);
    f.client.selection({...SELECTED, token: '2'.repeat(64), snakeId: 4}); f.client.setEnabled(false);
    expect(f.sent).toHaveLength(1); f.ack(0, true);
    expect(f.sent).toHaveLength(2); expect(f.sent[1]?.selectionToken).toBeUndefined();
    f.ack(1, false); f.client.dispose();
  });

  it('filters previous subscriptions and duplicate rows after a target change', () => {
    const f = fixture(); f.client.setEnabled(true); f.ack(0, true);
    f.client.selection({...SELECTED, token: '2'.repeat(64), snakeId: 4}); f.ack(1, true, '0000000000000002');
    f.client.snapshot(inspectorFixture()); expect(f.shown.at(-1)).toBeNull();
    const row = inspectorFixture(); row.subscriptionSequence = '0000000000000002'; row.sample!.snakeId = 4;
    f.client.snapshot(row); expect(f.shown.at(-1)).toBe(row);
    const count = f.shown.length; f.client.snapshot(row); expect(f.shown).toHaveLength(count); f.client.dispose();
  });

  it('retries busy admission with one timer and cancels all old-scope intent on disconnect', () => {
    vi.useFakeTimers(); const f = fixture(); f.client.setEnabled(true);
    f.client.result({type: 'inspectorResult', requestId: f.sent[0]!.requestId, applied: false, enabled: false, reason: 'inspector is busy'});
    vi.advanceTimersByTime(249); expect(f.sent).toHaveLength(1);
    vi.advanceTimersByTime(1); expect(f.sent).toHaveLength(2);
    f.client.dispose(); f.ack(1, true); vi.advanceTimersByTime(1000); expect(f.sent).toHaveLength(2);
    expect(f.shown.at(-1)).toBeNull();
  });

  it('waits for a new physical token after rejection and never fabricates a non-due sample', () => {
    const f = fixture(); f.client.setEnabled(true);
    f.client.result({type: 'inspectorResult', requestId: f.sent[0]!.requestId, applied: false, enabled: false, reason: 'stale physical selection'});
    f.client.selection(SELECTED); expect(f.sent).toHaveLength(1);
    f.client.selection({...SELECTED, token: '2'.repeat(64)}); expect(f.sent).toHaveLength(2);
    f.ack(1, true, '0000000000000002'); const row = inspectorFixture(); row.subscriptionSequence = '0000000000000002';
    f.client.snapshot(row); expect(f.shown.at(-1)?.sample?.ageSteps).toBe('0000000000000001');
    f.client.selection(null); expect(f.sent[2]?.selectionToken).toBeUndefined(); f.client.dispose();
  });

  it('coalesces explicit one-shot probes and resumes delivered samples without accepting mixed modes', () => {
    const f = fixture(); f.client.probeOnce();
    expect(f.sent[0]).toMatchObject({pureProbe: true, snakeId: 3});
    f.client.probeOnce(); f.client.probeOnce(); expect(f.sent).toHaveLength(1);
    f.ack(0, true); expect(f.sent).toHaveLength(2); expect(f.sent[1]?.pureProbe).toBe(true);
    f.ack(1, true, '0000000000000002');
    const row = inspectorFixture(); row.subscriptionSequence = '0000000000000002';
    f.client.snapshot(row); expect(f.shown.at(-1)).toBeNull();
    row.sample!.pureProbe = true; row.sample!.neuralOutputs = null;
    f.client.snapshot(row); expect(f.shown.at(-1)).toBe(row); expect(f.status.at(-1)).toContain('explicit pure sensor probe');
    f.client.showDelivered(); expect(f.sent[2]?.pureProbe).toBeUndefined();
    f.ack(2, true, '0000000000000003'); row.subscriptionSequence = '0000000000000003';
    f.client.snapshot(row); expect(f.shown.at(-1)).toBeNull();
    row.sample!.pureProbe = false; row.sample!.neuralOutputs = [0, 0];
    f.client.snapshot(row); expect(f.shown.at(-1)).toBe(row);
    f.client.setEnabled(false); expect(f.sent[3]?.selectionToken).toBeUndefined(); f.client.dispose();
  });


  it('does not automatically probe a replacement life while following an individual', () => {
    const f = fixture(); f.client.probeOnce(); f.ack(0, true);
    f.client.selection({...SELECTED, token: '2'.repeat(64), snakeId: 4});
    expect(f.sent[1]?.selectionToken).toBeUndefined(); f.ack(1, false);
    expect(f.sent).toHaveLength(2);
    f.client.probeOnce(); expect(f.sent[2]).toMatchObject({selectionToken: '2'.repeat(64), snakeId: 4, pureProbe: true});
    f.client.dispose();
  });

});
