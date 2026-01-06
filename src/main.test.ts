import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

type WorkerStub = {
  messages: unknown[];
  onmessage: ((event: MessageEvent) => void) | null;
  postMessage: (msg: unknown) => void;
};

type TestGlobal = typeof globalThis & {
  document?: Document;
  window?: Window & typeof globalThis;
  Worker?: typeof Worker;
  WebSocket?: unknown;
  localStorage?: Storage;
  requestAnimationFrame?: (callback: FrameRequestCallback) => number;
  __workerInstance?: WorkerStub;
  currentWorld?: { fitnessHistory: unknown[] };
};

const globalAny = globalThis as TestGlobal;

function makeCtx(): CanvasRenderingContext2D {
  return {
    setTransform() {},
    clearRect() {},
    beginPath() {},
    moveTo() {},
    lineTo() {},
    arc() {},
    fill() {},
    stroke() {},
    fillText() {}
  } as unknown as CanvasRenderingContext2D;
}

type TestElement = Partial<HTMLElement> & {
  id: string;
  value: string;
  textContent: string;
  innerHTML: string;
  style: Record<string, string>;
  dataset: Record<string, string>;
  classList: {
    add: () => void;
    remove: () => void;
    toggle: () => void;
    contains: () => boolean;
  };
  addEventListener: () => void;
  appendChild: () => void;
  setAttribute: () => void;
  querySelectorAll: () => TestElement[];
  getContext: () => CanvasRenderingContext2D;
  click: () => void;
};

function makeElement(id: string, overrides: Record<string, unknown> = {}): TestElement {
  return {
    id,
    value: '',
    textContent: '',
    innerHTML: '',
    style: {},
    dataset: {},
    classList: {
      add() {},
      remove() {},
      toggle() {},
      contains() {
        return false;
      }
    },
    addEventListener() {},
    appendChild() {},
    setAttribute() {},
    querySelectorAll: () => [],
    getContext: () => makeCtx(),
    click() {},
    ...overrides
  } as TestElement;
}

describe('main.ts', () => {
  let originalDocument: unknown;
  let originalWindow: unknown;
  let originalWorker: unknown;
  let originalRaf: unknown;
  let originalLocalStorage: unknown;
  let elements: Map<string, TestElement>;

  beforeEach(() => {
    vi.resetModules();
    elements = new Map();
    const ids = [
      'c',
      'snakes',
      'simSpeed',
      'layers',
      'n1',
      'n2',
      'n3',
      'n4',
      'n5',
      'graphNodes',
      'graphEdges',
      'graphOutputs',
      'graphNodeAdd',
      'graphEdgeAdd',
      'graphOutputAdd',
      'graphApply',
      'graphReset',
      'graphPresetList',
      'graphPresetName',
      'graphPresetSave',
      'graphSpecInput',
      'graphSpecApply',
      'graphSpecCopy',
      'graphSpecExport',
      'graphSpecStatus',
      'snakesVal',
      'simSpeedVal',
      'layersVal',
      'n1Val',
      'n2Val',
      'n3Val',
      'n4Val',
      'n5Val',
      'apply',
      'defaults',
      'toggle',
      'settingsContainer',
      'connectionStatus',
      'vizCanvas',
      'statsCanvas',
      'btnExport',
      'btnImport',
      'fileInput',
      'statsTitle',
      'statsSubtitle',
      'statsInfo',
      'hofTable',
      'vizInfo',
      'godModeLog',
      'joinOverlay',
      'joinName',
      'joinPlay',
      'joinSpectate',
      'joinStatus',
      'toggleSettingsLock',
      'settingsControls',
      'settingsLockHint',
      'tab-settings',
      'tab-viz',
      'tab-stats'
    ];

    ids.forEach((id) => {
      elements.set(id, makeElement(id));
    });

    const tabBtns = [
      makeElement('tabBtnSettings', { dataset: { tab: 'tab-settings' } }),
      makeElement('tabBtnViz', { dataset: { tab: 'tab-viz' } }),
      makeElement('tabBtnStats', { dataset: { tab: 'tab-stats' } })
    ];
    const tabContents = [
      elements.get('tab-settings'),
      elements.get('tab-viz'),
      elements.get('tab-stats')
    ];

    originalDocument = globalAny.document;
    const mockDocument: Partial<Document> = {
      getElementById: (id: string) => elements.get(id) || makeElement(id),
      querySelectorAll: (selector: string) => {
        if (selector === '.tab-btn') return tabBtns;
        if (selector === '.tab-content') return tabContents;
        return [];
      },
      querySelector: () => tabBtns[1] ?? null,
      createElement: () => makeElement('created') as unknown as HTMLElement,
      createElementNS: () => makeElement('created-ns') as unknown as HTMLElement
    };
    globalAny.document = mockDocument as Document;

    originalWindow = globalAny.window;
    globalAny.window = globalAny;
    globalAny.window.devicePixelRatio = 1;
    globalAny.window.addEventListener = () => {};

    originalLocalStorage = globalAny.localStorage;
    const storage = new Map<string, string>();
    globalAny.localStorage = {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: (key: string) => {
        storage.delete(key);
      }
    };

    originalRaf = globalAny.requestAnimationFrame;
    globalAny.requestAnimationFrame = () => 0;

    originalWorker = globalAny.Worker;
    class StubWorker implements WorkerStub {
      messages: unknown[];
      onmessage: ((event: MessageEvent) => void) | null;

      constructor() {
        this.messages = [];
        this.onmessage = null;
        globalAny.__workerInstance = this;
      }
      postMessage(msg: unknown) {
        this.messages.push(msg);
      }
    }
    globalAny.Worker = StubWorker as unknown as typeof Worker;

    globalAny.WebSocket = undefined;
  });

  afterEach(() => {
    globalAny.document = originalDocument;
    globalAny.window = originalWindow;
    globalAny.Worker = originalWorker;
    globalAny.localStorage = originalLocalStorage;
    delete globalAny.WebSocket;
    globalAny.requestAnimationFrame = originalRaf;
    delete globalAny.__workerInstance;
  });

  it('initializes the worker and posts init', async () => {
    await import('./main.ts');

    const worker = globalAny.__workerInstance;
    expect(worker).toBeDefined();
    expect(worker?.messages.length ?? 0).toBeGreaterThan(0);
    const first = worker?.messages[0];
    const firstMsg =
      first && typeof first === 'object' ? (first as Record<string, unknown>) : null;
    expect(firstMsg?.type).toBe('init');
  });

  it('maps fitness history payloads into the shared history buffer', async () => {
    await import('./main.ts');
    const worker = globalAny.__workerInstance;
    if (!worker || !worker.onmessage) {
      throw new Error('Expected worker to be initialized');
    }
    const buffer = new Float32Array([1, 0, 0, 0, 0, 1]).buffer;
    worker.onmessage({
      data: {
        type: 'frame',
        buffer,
        stats: {
          gen: 1,
          alive: 0,
          fps: 60,
          fitnessHistory: [{ gen: 1, best: 4, avg: 2.5, min: 1 }]
        }
      }
    } as MessageEvent);

    expect(globalAny.currentWorld).toBeDefined();
    expect(globalAny.currentWorld.fitnessHistory.length).toBe(1);
    expect(globalAny.currentWorld.fitnessHistory[0]).toEqual({
      gen: 1,
      avgFitness: 2.5,
      maxFitness: 4,
      minFitness: 1,
      speciesCount: 0,
      topSpeciesSize: 0,
      avgWeight: 0,
      weightVariance: 0
    });
  });
});
