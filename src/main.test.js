import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

function makeCtx() {
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
  };
}

function makeElement(id, overrides = {}) {
  return {
    id,
    value: '',
    textContent: '',
    style: {},
    dataset: {},
    classList: {
      add() {},
      remove() {},
      toggle() {}
    },
    addEventListener() {},
    appendChild() {},
    querySelectorAll: () => [],
    getContext: () => makeCtx(),
    click() {},
    ...overrides
  };
}

describe('main.js', () => {
  let originalDocument;
  let originalWindow;
  let originalWorker;
  let originalRaf;
  let elements;

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

    originalDocument = globalThis.document;
    globalThis.document = {
      getElementById: (id) => elements.get(id) || makeElement(id),
      querySelectorAll: (selector) => {
        if (selector === '.tab-btn') return tabBtns;
        if (selector === '.tab-content') return tabContents;
        return [];
      },
      querySelector: () => tabBtns[1],
      createElement: () => makeElement('created')
    };

    originalWindow = globalThis.window;
    globalThis.window = globalThis;
    globalThis.window.devicePixelRatio = 1;
    globalThis.window.addEventListener = () => {};

    originalRaf = globalThis.requestAnimationFrame;
    globalThis.requestAnimationFrame = () => 0;

    originalWorker = globalThis.Worker;
    globalThis.Worker = class StubWorker {
      constructor() {
        this.messages = [];
        this.onmessage = null;
        globalThis.__workerInstance = this;
      }
      postMessage(msg) {
        this.messages.push(msg);
      }
    };
  });

  afterEach(() => {
    globalThis.document = originalDocument;
    globalThis.window = originalWindow;
    globalThis.Worker = originalWorker;
    globalThis.requestAnimationFrame = originalRaf;
    delete globalThis.__workerInstance;
  });

  it('initializes the worker and posts init', async () => {
    await import('./main.js');

    const worker = globalThis.__workerInstance;
    expect(worker).toBeDefined();
    expect(worker.messages.length).toBeGreaterThan(0);
    expect(worker.messages[0].type).toBe('init');
  });

  it('maps fitness history payloads into the shared history buffer', async () => {
    await import('./main.js');
    const worker = globalThis.__workerInstance;
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
    });

    expect(globalThis.currentWorld).toBeDefined();
    expect(globalThis.currentWorld.fitnessHistory.length).toBe(1);
    expect(globalThis.currentWorld.fitnessHistory[0]).toEqual({
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
