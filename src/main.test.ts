import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/** Generic element surface sufficient for the application startup boundary. */
interface SmokeElement {
  /** Element identifier. */
  id: string;
  /** Form value. */
  value: string;
  /** Checkbox state. */
  checked: boolean;
  /** Disabled form-control state. */
  disabled: boolean;
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
  addEventListener: (type: string, listener: (event: Event) => void) => void;
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
  const listeners = new Map<string, Array<(event: Event) => void>>();
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
    disabled: false,
    textContent: '',
    innerHTML: '',
    style: {},
    dataset: {},
    classList,
    width: 800,
    height: 600,
    addEventListener(type, listener) {
      const registered = listeners.get(type) ?? [];
      registered.push(listener);
      listeners.set(type, registered);
    },
    appendChild() { },
    setAttribute(name, value) { attributes.set(name, value); },
    getAttribute(name) { return attributes.get(name) ?? null; },
    querySelectorAll: () => [],
    closest: () => null,
    getContext: () => context,
    click() {
      for (const listener of listeners.get('click') ?? []) {
        listener(new Event('click'));
      }
    }
  };
}

/** Controllable WebSocket surface exposed to startup tests. */
interface StubSocketSurface {
  /** Browser-ready state. */
  readyState: number;
  /** Binary response mode selected by the client. */
  binaryType: BinaryType;
  /** Open callback installed by the client. */
  onopen: (() => void) | null;
  /** Message callback installed by the client. */
  onmessage: ((event: { data: unknown }) => void) | null;
  /** Error callback installed by the client. */
  onerror: (() => void) | null;
  /** Close callback installed by the client. */
  onclose: (() => void) | null;
  /** Serialized messages sent by the client. */
  sent: string[];
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
  /** DOM elements created during the current startup import. */
  let elements: Map<string, SmokeElement>;
  /** Socket instance created during the current startup import. */
  let activeSocket: StubSocketSurface | null;

  beforeEach(() => {
    vi.resetModules();
    connectedUrl = '';
    activeSocket = null;
    elements = new Map<string, SmokeElement>();
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
    /** Expose a constructed socket without aliasing `this` inside the stub. */
    const captureSocket = (socket: StubSocketSurface): void => {
      activeSocket = socket;
    };

    class StubWebSocket {
      /** Ready-state constant consumed by the transport send guards. */
      static OPEN = 1;
      /** Browser-ready state. */
      readyState = StubWebSocket.OPEN;
      /** Binary response mode selected by the client. */
      binaryType: BinaryType = 'arraybuffer';
      /** Open callback installed by the client. */
      onopen: (() => void) | null = null;
      /** Message callback installed by the client. */
      onmessage: ((event: { data: unknown }) => void) | null = null;
      /** Error callback installed by the client. */
      onerror: (() => void) | null = null;
      /** Close callback installed by the client. */
      onclose: (() => void) | null = null;
      /** Serialized messages sent by the client. */
      sent: string[] = [];

      /** Construct a socket and capture its resolved URL. */
      constructor(url: string) {
        connectedUrl = url;
        captureSocket(this);
      }

      /** Close the inert socket. */
      close(): void { }

      /** Capture one serialized client message. */
      send(payload: string): void {
        this.sent.push(payload);
      }
    }

    vi.stubGlobal('document', documentStub);
    vi.stubGlobal('window', windowStub);
    vi.stubGlobal('localStorage', makeStorage());
    vi.stubGlobal('requestAnimationFrame', () => 0);
    vi.stubGlobal('WebSocket', StubWebSocket);
    vi.stubGlobal('fetch', vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      const payload = url.includes('/api/graph-presets')
        ? { ok: true, presets: [] }
        : { ok: true, hof: [] };
      return { ok: true, json: async () => payload } as Response;
    }));
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('attempts the resolved server connection when the entry module loads', async () => {
    await import('./main.ts');

    expect(connectedUrl).toBe('ws://localhost:5174');
  });

  it('sends New Run and refreshes the visible seed after acknowledgement', async () => {
    await import('./main.ts');
    const socket = activeSocket;
    expect(socket).not.toBeNull();
    if (!socket) return;

    socket.onopen?.();
    socket.onmessage?.({
      data: JSON.stringify({
        type: 'welcome',
        protocolVersion: 2,
        sessionId: 'test-session',
        tickRate: 60,
        worldSeed: 42,
        runId: 'run-42',
        configRevision: 0,
        configHash: 'cfg-test',
        settings: { core: { simSpeed: 1 }, updates: [] },
        inferenceMode: {
          requestedBackend: 'native',
          activeBackend: 'native',
          requestedMt: true,
          activeWorkerCount: 2
        },
        sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
        serializerVersion: 1,
        frameByteLength: 28
      })
    });

    expect(elements.get('connectionStatus')?.textContent)
      .toBe('Server · seed 42 · native MT×2');

    const newRunButton = elements.get('newRun');
    newRunButton?.click();
    const request = socket.sent
      .map((payload) => JSON.parse(payload) as Record<string, unknown>)
      .find((message) => message['type'] === 'newRun');
    expect(request?.['requestId']).toEqual(expect.any(String));
    expect(newRunButton?.disabled).toBe(true);

    socket.onmessage?.({
      data: JSON.stringify({
        type: 'newRunResult',
        requestId: request?.['requestId'],
        applied: true,
        worldSeed: 99,
        runId: 'run-99'
      })
    });

    expect(newRunButton?.disabled).toBe(false);
    expect(elements.get('connectionStatus')?.textContent)
      .toBe('Server · seed 99 · native MT×2');
  });
});
