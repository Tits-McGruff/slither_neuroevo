import { afterEach, expect, it, vi } from 'vitest';
import { BrainLibraryView, type LibraryEnvironment } from './brainLibraryView.ts';
import type { BrainLibraryClient } from './net/brainLibrary.ts';

/** Inert DOM surface proving text-only metadata rendering and explicit user actions. */
class Element {
  /** Visible content, independent of markup. */
  textContent = '';
  /** Form value. */
  value = '';
  /** Optional opaque selected File list. */
  files: FileList | null = null;
  /** Button admission. */
  disabled = false;
  /** Conditional visibility. */
  hidden = false;
  /** Browser element presentation. */
  className = '';
  /** Button type. */
  type = '';
  /** Link destination. */
  href = '';
  /** Direct-download hint. */
  download = '';
  /** Direct-download browsing context. */
  target = '';
  /** Opener isolation. */
  rel = '';
  /** Synthetic children. */
  children: Element[] = [];
  /** Ordinary DOM class toggle. */
  classList = {toggle: vi.fn()};
  /** User event handlers. */
  listeners = new Map<string, (() => void)[]>();
  /** Reject imported HTML interpretation in every test path. */
  set innerHTML(_value: string) { throw new Error('metadata must use textContent'); }
  /** Add direct children. */
  append(...children: Element[]): void { this.children.push(...children); }
  /** Replace one cold view. */
  replaceChildren(): void { this.children = []; this.textContent = ''; }
  /** Register a user action. */
  addEventListener(type: string, action: () => void): void { this.listeners.set(type, [...this.listeners.get(type) ?? [], action]); }
  /** Activate an admitted user action. */
  click(): void { if (!this.disabled) for (const action of this.listeners.get('click') ?? []) action(); }
  /** Discard a temporary direct download element. */
  remove(): void {}
}

/** Text traversal for assertions without a browser HTML parser or model execution. */
function text(element: Element): string { return element.textContent + element.children.map(text).join(' '); }
/** A bounded graph/provenance metadata placeholder, never neural weights. */
const RECORD = {brainId: 'ab'.repeat(32), name: '<img src=x onerror=bad>', artifact: {sourceKind: 'manual', inputSize: 72,
  parameterCount: 146, stateSize: 0, controlIntervalSeconds: 0.06, lifeStateRule: 'preserve', architectureKey: 'saved', sensorSha256: 'saved-sensors'},
  graphManifest: {schemaVersion: 1, graphSpec: {type: 'graph', nodes: [{id: 'input', type: 'Input', outputSize: 72},
    {id: 'motor', type: 'Dense', inputSize: 72, outputSize: 2}], edges: [{from: 'input', to: 'motor', fromPort: 0, toPort: 0}],
    outputs: [{nodeId: 'motor', port: 0}], outputSize: 2}},
  provenance: {details: {teacherContract: {configSha256: 'source-config'}, note: '<script>bad</script>'}}};

/** Explicit presentation-only transport double; real browser transport/native tests cover wire admission. */
function fixture(environment: Partial<LibraryEnvironment> = {}) {
  const elements = new Map<string, Element>();
  const get = (id: string): Element => { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id)!; };
  const client = {hasPendingSpawn: vi.fn(() => false), dismissPendingSpawn: vi.fn(),
    list: vi.fn(async () => ({page: {entries: [RECORD], next: null}})), read: vi.fn(async () => RECORD),
    spawn: vi.fn(async () => ({snakeId: 41})), isCurrent: vi.fn(async () => true), exportUrl: vi.fn(() => 'http://server/export')};
  const followed = vi.fn();
  vi.stubGlobal('document', {getElementById: get, createElement: () => new Element(), querySelector: () => null,
    querySelectorAll: () => [], body: new Element()});
  const view = new BrainLibraryView(client as unknown as BrainLibraryClient,
    () => ({graphSourceJson: 'current', sensorSha256: 'other-sensors', cadence: 0.02, configHash: 'current-config', ...environment}), followed);
  view.contextChanged(true);
  return {client, view, get, followed};
}

/** Restore the inert browser after each presentation boundary test. */
afterEach(() => vi.unstubAllGlobals());

it('loads only on an explicit action and renders untrusted names/provenance as text', async () => {
  const f = fixture(); expect(f.client.list).not.toHaveBeenCalled(); f.view.refresh();
  await vi.waitFor(() => expect(text(f.get('brainLibraryList'))).toContain(RECORD.name));
  f.get('brainLibraryList').children[0]!.children[2]!.children[0]!.click();
  await vi.waitFor(() => expect(text(f.get('brainLibraryDetails'))).toContain('<script>bad</script>'));
  expect(text(f.get('brainLibraryDetails'))).toContain('Different structural sensors');
  expect(text(f.get('brainLibraryDetails'))).toContain('Graph descriptions differ');
  expect(text(f.get('brainLibraryDetails'))).toContain('Different neural cadence');
  expect(text(f.get('brainLibraryDetails'))).toContain('Source configuration differs');
});

it('does not follow a confirmed spawn that belongs to a replaced world', async () => {
  const f = fixture(); f.client.isCurrent.mockResolvedValue(false); f.view.refresh();
  await vi.waitFor(() => expect(f.get('brainLibraryList').children).toHaveLength(1));
  f.get('brainLibraryList').children[0]!.children[2]!.children[1]!.click();
  await vi.waitFor(() => expect(text(f.get('brainLibraryStatus'))).toContain('was confirmed'));
  expect(f.followed).not.toHaveBeenCalled();
});

it('discards metadata that arrives after disconnect and performs no background catalog polling', async () => {
  const f = fixture(); let resolve!: (value: typeof RECORD) => void;
  f.client.read.mockImplementationOnce(() => new Promise(done => {resolve = done;}));
  f.view.refresh(); await vi.waitFor(() => expect(f.get('brainLibraryList').children).toHaveLength(1));
  f.get('brainLibraryList').children[0]!.children[2]!.children[0]!.click();
  await vi.waitFor(() => expect(f.client.read).toHaveBeenCalledTimes(1));
  f.view.contextChanged(false); resolve(RECORD);
  await new Promise(done => setTimeout(done, 0));
  expect(text(f.get('brainLibraryDetails'))).toBe('');
  expect(text(f.get('brainLibraryStatus'))).toContain('Connect to a server');
  expect(f.client.list).toHaveBeenCalledTimes(1);
});

it('compares applied native graph descriptions without comparing editor and native key formats', async () => {
  const f = fixture({graphSourceJson: JSON.stringify(RECORD.graphManifest.graphSpec)});
  f.view.refresh(); await vi.waitFor(() => expect(f.get('brainLibraryList').children).toHaveLength(1));
  f.get('brainLibraryList').children[0]!.children[2]!.children[0]!.click();
  await vi.waitFor(() => expect(text(f.get('brainLibraryDetails'))).toContain('Graph descriptions match'));
  expect(text(f.get('brainLibraryDetails'))).toContain('Rust checks the compiled layout');
});
