import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

/** Minimal worker stub for tracking messages in main thread tests. */
type WorkerStub = {
  messages: unknown[];
  onmessage: ((event: MessageEvent) => void) | null;
  postMessage: (msg: unknown) => void;
};

/** Global object shape override used in DOM-free tests. */
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

/** Mutable global alias with test-specific fields. */
const globalAny = globalThis as TestGlobal;

/**
 * Builds a minimal 2D canvas context stub.
 * @returns Canvas 2D context shim.
 */
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

/** Minimal element stub used for DOM wiring tests. */
type TestElement = {
  id: string;
  value: string;
  textContent: string;
  innerHTML: string;
  style: Record<string, string>;
  dataset: Record<string, string>;
  classList: DOMTokenList;
  addEventListener: () => void;
  appendChild: () => void;
  setAttribute: () => void;
  querySelectorAll: () => TestElement[];
  getContext: () => CanvasRenderingContext2D;
  click: () => void;
};

/**
 * Builds a DOMTokenList stub for classList usage.
 * @returns DOMTokenList shim.
 */
function makeClassList(): DOMTokenList {
  return {
    length: 0,
    value: '',
    add() {},
    remove() {},
    toggle() {
      return false;
    },
    contains() {
      return false;
    },
    item() {
      return null;
    },
    replace() {
      return false;
    },
    supports() {
      return false;
    },
    forEach() {},
    entries() {
      return [][Symbol.iterator]();
    },
    keys() {
      return [][Symbol.iterator]();
    },
    values() {
      return [][Symbol.iterator]();
    }
  } as unknown as DOMTokenList;
}

/**
 * Creates a test element with optional property overrides.
 * @param id - Element id to assign.
 * @param overrides - Optional field overrides.
 * @returns Test element stub.
 */
function makeElement(id: string, overrides: Record<string, unknown> = {}): TestElement {
  return {
    id,
    value: '',
    textContent: '',
    innerHTML: '',
    style: {},
    dataset: {},
    classList: makeClassList(),
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
      getElementById: (id: string) =>
        (elements.get(id) || makeElement(id)) as unknown as HTMLElement,
      querySelectorAll: (selector: string) => {
        if (selector === '.tab-btn') return tabBtns as unknown as NodeListOf<HTMLElement>;
        if (selector === '.tab-content') return tabContents as unknown as NodeListOf<HTMLElement>;
        return [] as unknown as NodeListOf<HTMLElement>;
      },
      querySelector: () => tabBtns[1] ?? null,
      createElement: () => makeElement('created') as unknown as HTMLElement,
      createElementNS: ((namespaceURI: string, qualifiedName: string) => {
        void namespaceURI;
        void qualifiedName;
        return makeElement('created-ns') as unknown as Element;
      }) as Document['createElementNS']
    };
    globalAny.document = mockDocument as Document;

    originalWindow = globalAny.window;
    globalAny.window = globalAny as unknown as Window & typeof globalThis;
    globalAny.window.devicePixelRatio = 1;
    globalAny.window.addEventListener = () => {};

    originalLocalStorage = globalAny.localStorage;
    const storage = new Map<string, string>();
    globalAny.localStorage = {
      length: 0,
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: (key: string) => {
        storage.delete(key);
      },
      clear: () => {
        storage.clear();
      },
      key: () => null
    } as Storage;

    originalRaf = globalAny.requestAnimationFrame;
    globalAny.requestAnimationFrame = () => 0;

    originalWorker = globalAny.Worker;
    /** Worker stub that captures posted messages for assertions. */
    class StubWorker implements WorkerStub {
      /** Recorded messages posted by the main thread. */
      messages: unknown[];
      /** Message handler assigned by the main module. */
      onmessage: ((event: MessageEvent) => void) | null;

      /** Create a stub worker and register it on the test global. */
      constructor() {
        this.messages = [];
        this.onmessage = null;
        globalAny.__workerInstance = this;
      }
      /**
       * Record a posted message.
       * @param msg - Message payload to store.
       */
      postMessage(msg: unknown) {
        this.messages.push(msg);
      }
    }
    globalAny.Worker = StubWorker as unknown as typeof Worker;

    globalAny.WebSocket = undefined as unknown as typeof WebSocket;
  });

  afterEach(() => {
    globalAny.document = originalDocument as Document;
    globalAny.window = originalWindow as Window & typeof globalThis;
    globalAny.Worker = originalWorker as typeof Worker;
    globalAny.localStorage = originalLocalStorage as Storage;
    delete globalAny.WebSocket;
    globalAny.requestAnimationFrame = originalRaf as (callback: FrameRequestCallback) => number;
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
    expect(firstMsg?.['type']).toBe('init');
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
    const world = globalAny.currentWorld as NonNullable<TestGlobal['currentWorld']>;
    expect(world.fitnessHistory.length).toBe(1);
    expect(world.fitnessHistory[0]).toEqual({
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
