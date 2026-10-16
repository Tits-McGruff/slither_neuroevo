import { afterEach, expect, it, vi } from 'vitest';
import { DistillationView } from './distillationView.ts';
import type { DistillationClient } from './net/distillation.ts';

/** Synthetic DOM inventory, replaced for each independent panel. */
let elements = new Map<string, Element>();

/** Inert text-only DOM catches accidental interpretation of diagnostic strings as HTML. */
class Element {
  /** Opaque element identity registered by both static and dynamically constructed controls. */
  private identity = '';
  /** Visible text in this element, independent of children. */
  private content = '';
  /** Cold child nodes. */
  children: Element[] = [];
  /** Form contents. */
  value = '';
  /** Conditional controls. */
  disabled = false;
  /** Conditional presentation. */
  hidden = false;
  /** Ordinary class presentation. */
  className = '';
  /** Captured button events. */
  listeners = new Map<string, (() => void)[]>();
  /** Status color presentation. */
  classList = { toggle: vi.fn() };
  /** Register constructed IDs so production DOM lookup finds the same control. */
  set id(value: string) { this.identity = value; elements.set(value, this); }
  /** Preserve exact constructed control identities. */
  get id(): string { return this.identity; }
  /** Emulate DOM text replacement, including removal of stale children. */
  set textContent(value: string) { this.content = value; this.children = []; }
  /** Return only the direct text node. */
  get textContent(): string { return this.content; }
  /** Imported metadata must never become markup. */
  set innerHTML(_value: string) { throw new Error('Use text nodes for diagnostic metadata.'); }
  /** Add bounded children. */
  append(...children: Element[]): void { this.children.push(...children); }
  /** Replace the previous bounded publication. */
  replaceChildren(): void { this.children = []; this.content = ''; }
  /** Register an explicit user action. */
  addEventListener(name: string, action: () => void): void { this.listeners.set(name, [...this.listeners.get(name) ?? [], action]); }
  /** Only admitted actions may run. */
  click(): void { if (!this.disabled) for (const action of this.listeners.get('click') ?? []) action(); }
  /** Return rendered row buttons for control disable assertions. */
  querySelectorAll(): Element[] { return this.children.flatMap(child => [child, ...child.querySelectorAll()]); }
}

/** Read text without an HTML parser or any model/sample interpretation. */
function text(element: Element): string { return element.textContent + ' ' + element.children.map(text).join(' '); }
/** Exact diagnostic capture key. */
const EPOCH = 'ab'.repeat(16);
/** Bounded display-only manifest supplied by the tested transport. */
const DATASET = { contract: { captureEpoch: EPOCH, runId: 'current', teacherRevision: 2, teacherMode: 1,
  controlIntervalSeconds: 1 / 60, inputSize: 264, sensorSha256: '<script>sensor</script>' }, state: 'active',
productionWatermark: 'fffffffffffffffe', storageWatermark: 'fffffffffffffffd', resolvedThrough: 'fffffffffffffffc',
retainedChunks: 1, retainedBytes: '0000000000000100', nativeDroppedChunks: '0000000000000000',
nativeDroppedSamples: '0000000000000000', spoolDroppedChunks: '0000000000000000', error: null };
/** Fixed exact native aggregate used only for presentation, without inference. */
const TOTALS = { samples: '0000000000000005', turnMae: 0.125, turnMse: 0.0625, boostDisagreementRate: 0.2 };
/** Native snapshot retaining a deliberately untrusted error as ordinary text. */
const SHADOW = { shadowSlot: 0, captureEpoch: EPOCH, artifactSha256: 'cd'.repeat(32), baselineSlot: 2, enabled: true,
activationSequence: '0000000000000003', lastSampleStep: '0000000000000042', error: '<img src=x onerror=bad>',
totals: TOTALS, cruising: TOTALS, preDeath: TOTALS, strategies: Array(4).fill(TOTALS),
lifeStages: Array(3).fill(TOTALS), strata: Array(6).fill(TOTALS) };

/** Bind real event handlers against a metadata-only transport double. */
function fixture(ready = true) {
  elements = new Map();
  const get = (id: string): Element => {
    if (!elements.has(id)) { const value = new Element(); value.id = id; }
    return elements.get(id)!;
  };
  const tab = new Element();
  vi.stubGlobal('document', { getElementById: get, createElement: () => new Element(), querySelector: () => tab });
  const client = { hasPendingShadow: vi.fn(() => false), hasUncertainCapture: vi.fn(() => false),
    dismissPendingShadow: vi.fn(), dismissUncertainCapture: vi.fn(),
    datasets: vi.fn(async () => ({datasets: [{captureEpoch: EPOCH, runId: 'current', state: 'active'}], hasMore: false, nextCursor: EPOCH})),
    manifest: vi.fn(async () => DATASET), shadows: vi.fn(async () => ({schemaVersion: 1, processEpoch: 'ef'.repeat(16), slots: [null, null, null, null] as (typeof SHADOW | null)[]})),
    start: vi.fn(async (_slots: number[]) => DATASET), stop: vi.fn(async (_epoch: string) => ({...DATASET, state: 'stopped'})),
    activate: vi.fn(async () => ({})), stopShadow: vi.fn(async () => ({})), retryShadow: vi.fn(async () => ({})) };
  const view = new DistillationView(client as unknown as DistillationClient);
  get('distillationTeachers').value = '0,1,2,3,4,5,6,7,8,9';
  view.contextChanged(true, ready, 'current');
  return { get, tab, client, view };
}

/** Restore the browser after every context/transport boundary. */
afterEach(() => vi.unstubAllGlobals());

it('starts off and fetches only explicit bounded metadata; v3 admission keeps capture disabled', async () => {
  const f = fixture(false);
  expect(f.client.start).not.toHaveBeenCalled(); expect(f.client.datasets).not.toHaveBeenCalled(); expect(f.client.shadows).not.toHaveBeenCalled();
  expect(f.get('distillationStart').disabled).toBe(true);
  f.get('distillationRefresh').click();
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toContain('refreshed'));
  expect(f.client.datasets).toHaveBeenCalledTimes(1); expect(f.client.shadows).toHaveBeenCalledTimes(1);
  expect(f.client.start).not.toHaveBeenCalled(); expect(f.get('distillationStart').disabled).toBe(true);
});

it('captures only explicit teacher slots and stops the selected exact epoch', async () => {
  const f = fixture();
  f.get('distillationTeachers').value = '0,2,9'; f.get('distillationStart').click();
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toBe('Teacher capture started.'));
  expect(f.client.start).toHaveBeenCalledWith([0,2,9]);
  expect(f.get('distillationDataset').textContent).toContain('<script>sensor</script>');
  expect(f.get('distillationDataset').textContent).toContain('fffffffffffffffe');
  expect(f.get('distillationStart').disabled).toBe(true);
  f.get('distillationStop').click();
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toBe('Teacher capture stopped.'));
  expect(f.client.stop).toHaveBeenCalledWith(EPOCH); expect(f.get('distillationStart').disabled).toBe(false);
});

it('discards reads and start confirmations crossing a run replacement', async () => {
  const f = fixture();
  let resolve: (value: typeof DATASET) => void = () => {};
  f.client.start.mockImplementation(() => new Promise(done => { resolve = done; }));
  f.get('distillationStart').click(); expect(f.get('distillationStart').disabled).toBe(true);
  f.view.contextChanged(true, false, 'replacement'); resolve(DATASET);
  await vi.waitFor(() => expect(f.get('distillationRefresh').disabled).toBe(false));
  expect(f.get('distillationDataset').textContent).toBe('');
  expect(f.get('distillationStart').disabled).toBe(true);
  expect(f.get('distillationStatus').textContent).toContain('admitted v4');
  f.view.contextChanged(false, false, ''); expect(f.tab.hidden).toBe(true);
});

it('shows four explicit native shadow slots and renders all error strata as text', async () => {
  const f = fixture();
  f.client.shadows.mockResolvedValue({schemaVersion: 1, processEpoch: 'ef'.repeat(16), slots: [SHADOW, null, null, null]});
  f.view.refresh();
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toContain('refreshed'));
  const status = text(f.get('distillationShadowStatus0'));
  expect(status).toContain('<img src=x onerror=bad>'); expect(status).toContain('Pre-death'); expect(status).toContain('Respawn');
  expect(status).toContain('0.1250'); expect(status).toContain('20.00%');
  f.get('distillationDatasets').children[0]!.children[1]!.click();
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toBe('Capture selected.'));
  f.get('distillationCandidate3').value = 'cd'.repeat(32); f.get('distillationTeacher3').value = '2';
  f.get('distillationActivate3').click();
  await vi.waitFor(() => expect(f.client.activate).toHaveBeenCalledTimes(1));
  expect(f.client.activate.mock.calls[0]).toEqual([3, 2, 'cd'.repeat(32), EPOCH, expect.objectContaining({processEpoch: 'ef'.repeat(16)})]);
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toBe('Shadow candidate activated.'));
  f.get('distillationStopShadow0').click();
  await vi.waitFor(() => expect(f.client.stopShadow).toHaveBeenCalledWith(0, expect.objectContaining({slots: [SHADOW, null, null, null]})));
});

it('keeps uncertain capture starts and shadow retry bodies under explicit user control', async () => {
  const f = fixture();
  f.client.hasUncertainCapture.mockReturnValue(true); f.view.contextChanged(true, true, 'current');
  expect(f.get('distillationStart').disabled).toBe(true); expect(f.get('distillationRetry').hidden).toBe(true);
  expect(f.get('distillationPending').textContent).toContain('never automatically retried');
  f.get('distillationDismiss').click(); expect(f.client.dismissUncertainCapture).toHaveBeenCalledTimes(1);
  f.client.hasUncertainCapture.mockReturnValue(false); f.client.hasPendingShadow.mockReturnValue(true);
  f.view.contextChanged(true, true, 'current'); expect(f.get('distillationRetry').hidden).toBe(false);
  f.get('distillationRetry').click(); await vi.waitFor(() => expect(f.client.retryShadow).toHaveBeenCalledTimes(1));
});

it('rejects malformed teacher input before a capture command', async () => {
  const f = fixture();
  f.get('distillationTeachers').value = '0,,2'; f.get('distillationStart').click();
  await vi.waitFor(() => expect(f.get('distillationStatus').textContent).toContain('Enter teacher slots'));
  expect(f.client.start).not.toHaveBeenCalled();
});
