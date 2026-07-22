import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/** Generic element surface sufficient for the application startup boundary. */
interface SmokeElement {
  /** Element identifier. */
  id: string;
  /** Form value. */
  value: string;
  /** Checkbox state. */
  checked: boolean;
  /** Text content. */
  textContent: string;
  /** HTML content. */
  innerHTML: string;
  /** Mutable inline style. */
  style: Record<string, string>;
  /** Mutable data attributes. */
  dataset: Record<string, string>;
  /** Class-list shim. */
  classList: DOMTokenList;
  /** Canvas width. */
  width: number;
  /** Canvas height. */
  height: number;
  /** Register an event listener. */
  addEventListener: () => void;
  /** Append a child node. */
  appendChild: () => void;
  /** Set one attribute. */
  setAttribute: (name: string, value: string) => void;
  /** Read one attribute. */
  getAttribute: (name: string) => string | null;
  /** Query descendants. */
  querySelectorAll: () => SmokeElement[];
  /** Resolve a matching ancestor. */
  closest: () => Element | null;
  /** Return a canvas context. */
  getContext: () => CanvasRenderingContext2D;
  /** Trigger a click. */
  click: () => void;
}

/**
 * Build a small inert DOM element used only while importing the app entry point.
 * @param id - Element identifier.
 * @returns Generic element stub.
 */
function makeElement(id: string): SmokeElement {
  const attributes = new Map<string, string>();
  const classList = {
    add() { },
    remove() { },
    toggle() { return false; },
    contains() { return false; }
  } as unknown as DOMTokenList;
  const context = {
    setTransform() { },
    clearRect() { },
    beginPath() { },
    moveTo() { },
    lineTo() { },
    arc() { },
    fill() { },
    stroke() { },
    fillText() { }
  } as unknown as CanvasRenderingContext2D;
  return {
    id,
    value: '',
    checked: false,
    textContent: '',
    innerHTML: '',
    style: {},
    dataset: {},
    classList,
    width: 800,
    height: 600,
    addEventListener() { },
    appendChild() { },
    setAttribute(name, value) { attributes.set(name, value); },
    getAttribute(name) { return attributes.get(name) ?? null; },
    querySelectorAll: () => [],
    closest: () => null,
    getContext: () => context,
    click() { }
  };
}

/** Build isolated local storage for one startup import. */
function makeStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() { return values.size; },
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: key => values.delete(key),
    clear: () => values.clear(),
    key: index => Array.from(values.keys())[index] ?? null
  };
}

describe('main.ts startup smoke', () => {
  /** URL passed to the WebSocket constructor during startup. */
  let connectedUrl = '';

  beforeEach(() => {
    vi.resetModules();
    connectedUrl = '';
    const elements = new Map<string, SmokeElement>();
    const getElement = (id: string): SmokeElement => {
      const existing = elements.get(id);
      if (existing) return existing;
      const created = makeElement(id);
      elements.set(id, created);
      return created;
    };
    const documentStub = {
      body: getElement('body'),
      getElementById: (id: string) => getElement(id),
      querySelectorAll: () => [],
      querySelector: () => null,
      createElement: () => makeElement('created'),
      createElementNS: () => makeElement('created-ns')
    } as unknown as Document;
    const windowStub = {
      devicePixelRatio: 1,
      innerWidth: 800,
      innerHeight: 600,
      location: { search: '', hostname: 'localhost', protocol: 'http:' },
      addEventListener() { }
    } as unknown as Window & typeof globalThis;

    class StubWebSocket {
      /** Construct a socket and capture its resolved URL. */
      constructor(url: string) {
        connectedUrl = url;
      }

      /** Close the inert socket. */
      close(): void { }

      /** Ignore sends in this startup-only smoke. */
      send(): void { }
    }

    vi.stubGlobal('document', documentStub);
    vi.stubGlobal('window', windowStub);
    vi.stubGlobal('localStorage', makeStorage());
    vi.stubGlobal('requestAnimationFrame', () => 0);
    vi.stubGlobal('WebSocket', StubWebSocket);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('attempts the resolved server connection when the entry module loads', async () => {
    await import('./main.ts');

    expect(connectedUrl).toBe('ws://localhost:5174');
  });
});
