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
  /** Anchor destination or other URL-valued field. */
  href: string;
  /** Browser download hint. */
  download: string;
  /** Navigation context for a direct download or its error response. */
  target: string;
  /** Link relationship, including opener isolation without suppressing the UI referrer. */
  rel: string;
  /** Whether the element is hidden. */
  hidden: boolean;
  /** Whether a synthetic click was requested. */
  clicked: boolean;
  /** Selected browser files for the import input. */
  files: FileList | null;
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
  /** Dispatch one synthetic event to registered listeners. */
  dispatch: (type: string, event: Event) => void;
  /** Append a child node. */
  appendChild: () => void;
  /** Append one or more direct-download elements. */
  append: (...children: SmokeElement[]) => void;
  /** Remove a temporary direct-download element. */
  remove: () => void;
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
  /** Return stable canvas bounds for pointer-coordinate tests. */
  getBoundingClientRect: () => DOMRect;
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
  const classes = new Set<string>();
  const classList = {
    add(...tokens: string[]) { for (const token of tokens) classes.add(token); },
    remove(...tokens: string[]) { for (const token of tokens) classes.delete(token); },
    toggle(token: string, force?: boolean) {
      const enabled = force ?? !classes.has(token);
      if (enabled) classes.add(token);
      else classes.delete(token);
      return enabled;
    },
    contains(token: string) { return classes.has(token); }
  } as unknown as DOMTokenList;
  const context = {
    save() { },
    restore() { },
    translate() { },
    scale() { },
    getTransform() { return { a: 1 }; },
    createPattern() { return null; },
    fillRect() { },
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
    href: '',
    download: '',
    target: '',
    rel: '',
    hidden: false,
    clicked: false,
    files: null,
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
    dispatch(type, event) {
      for (const listener of listeners.get(type) ?? []) listener(event);
    },
    appendChild() { },
    append() { },
    remove() { },
    setAttribute(name, value) { attributes.set(name, value); },
    getAttribute(name) { return attributes.get(name) ?? null; },
    querySelectorAll: () => [],
    closest: () => null,
    getContext: () => context,
    getBoundingClientRect: () => ({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 800,
      bottom: 600,
      width: 800,
      height: 600,
      toJSON: () => ({})
    } as DOMRect),
    click() {
      this.clicked = true;
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
  /** Window listeners registered by the application. */
  let windowListeners: Map<string, Array<(event: Event) => void>>;
  /** Temporary DOM elements created during the current startup import. */
  let createdElements: SmokeElement[];
  /** Latest browser animation callback, advanced explicitly by camera tests. */
  let animationFrame: FrameRequestCallback | null;

  beforeEach(() => {
    vi.resetModules();
    connectedUrl = '';
    activeSocket = null;
    elements = new Map<string, SmokeElement>();
    createdElements = [];
    animationFrame = null;
    windowListeners = new Map<string, Array<(event: Event) => void>>();
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
      createElement: () => {
        const created = makeElement('created');
        createdElements.push(created);
        return created;
      },
      createElementNS: () => makeElement('created-ns')
    } as unknown as Document;
    const windowStub = {
      devicePixelRatio: 1,
      innerWidth: 800,
      innerHeight: 600,
      location: { search: '', hostname: 'localhost', protocol: 'http:' },
      /** Delegate browser timers to Vitest's controllable timer surface. */
      setTimeout(handler: () => void, delay: number) { return setTimeout(handler, delay); },
      addEventListener(type: string, listener: EventListener) {
        const registered = windowListeners.get(type) ?? [];
        registered.push(listener);
        windowListeners.set(type, registered);
      }
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
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      animationFrame = callback;
      return 0;
    });
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
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  /** Connect a spectator to a large arena with one selectable snake and a real binary frame. */
  async function openCameraSession(): Promise<{ socket: StubSocketSurface; canvas: SmokeElement }> {
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'camera-session', tickRate: 60,
      worldSeed: 42, runId: 'camera-run', configRevision: 1, configHash: 'cfg-camera',
      settings: { core: { simSpeed: 1 }, updates: [
        { path: 'worldRadius', value: 10000 }, { path: 'observer.overviewPadding', value: 1.25 },
        { path: 'observer.overviewExtraWorldMargin', value: 0 }
      ] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 80
    }) });
    socket.onmessage?.({ data: new Float32Array([
      1, 1, 1, 10000, 0, 0, 1, 7, 9, 0, 200, 100, 0, 0, 2, 200, 100, 210, 100, 0
    ]).buffer });
    elements.get('joinSpectate')!.click();
    elements.get('fitArena')!.click();
    return { socket, canvas: elements.get('c')! };
  }

  /** Advance an actual rendering frame to catch automatic camera snap-back. */
  function renderCameraFrame(): void {
    if (!animationFrame) throw new Error('missing animation callback');
    animationFrame(performance.now());
  }

  /** Build the mouse event surface consumed by camera and God Mode listeners. */
  function cameraMouse(clientX: number, clientY: number, button = 0, shiftKey = false): Event {
    return { clientX, clientY, button, shiftKey, preventDefault() {} } as unknown as Event;
  }

  it('zooms around the cursor and preserves the manual camera across rendering frames', async () => {
    const { canvas, socket } = await openCameraSession();
    const camera = window.currentWorld;
    const originalZoom = camera.zoom;
    const anchorX = camera.cameraX + 200 / camera.zoom;
    const anchorY = camera.cameraY + 150 / camera.zoom;
    const priorPackets = socket.sent.length;
    const preventDefault = vi.fn();
    canvas.dispatch('wheel', { clientX: 600, clientY: 450, deltaY: -200, deltaMode: 0, preventDefault } as unknown as Event);
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(camera.zoom).toBeGreaterThan(originalZoom);
    expect(camera.cameraX + 200 / camera.zoom).toBeCloseTo(anchorX, 10);
    expect(camera.cameraY + 150 / camera.zoom).toBeCloseTo(anchorY, 10);
    const manual = { zoom: camera.zoom, x: camera.cameraX, y: camera.cameraY };
    for (let index = 0; index < 3; index++) renderCameraFrame();
    expect({ zoom: camera.zoom, x: camera.cameraX, y: camera.cameraY }).toEqual(manual);
    expect(socket.sent.length).toBe(priorPackets);
  });

  it('pans to an outside release point and stops dragging after release or focus loss', async () => {
    const { canvas, socket } = await openCameraSession();
    const zoom = window.currentWorld.zoom;
    const priorPackets = socket.sent.length;
    canvas.dispatch('mousedown', cameraMouse(400, 300));
    canvas.dispatch('mousemove', cameraMouse(480, 340));
    expect(window.currentWorld.cameraX).toBeCloseTo(-80 / zoom);
    expect(window.currentWorld.cameraY).toBeCloseTo(-40 / zoom);
    for (const listener of windowListeners.get('mousemove') ?? []) listener(cameraMouse(900, 650));
    for (const listener of windowListeners.get('mouseup') ?? []) listener(cameraMouse(1000, 700));
    expect(window.currentWorld.cameraX).toBeCloseTo(-600 / zoom);
    expect(window.currentWorld.cameraY).toBeCloseTo(-400 / zoom);
    canvas.dispatch('click', cameraMouse(1000, 700));
    canvas.dispatch('mousemove', cameraMouse(1100, 750));
    renderCameraFrame();
    expect(window.currentWorld.cameraX).toBeCloseTo(-600 / zoom);
    expect(window.currentWorld.cameraY).toBeCloseTo(-400 / zoom);
    expect(socket.sent.length).toBe(priorPackets);
    canvas.dispatch('mousedown', cameraMouse(400, 300));
    for (const listener of windowListeners.get('blur') ?? []) listener(new Event('blur'));
    canvas.dispatch('mousemove', cameraMouse(500, 400));
    expect(window.currentWorld.cameraX).toBeCloseTo(-600 / zoom);
    expect(canvas.style['cursor']).toBe('grab');
  });

  it('bounds zoom and returns to automatic overview or follow with Home, Fit arena, and V', async () => {
    const { canvas } = await openCameraSession();
    for (let index = 0; index < 5; index++) {
      canvas.dispatch('wheel', { clientX: 400, clientY: 300, deltaY: -1000000, deltaMode: 0, preventDefault() {} } as unknown as Event);
    }
    expect(window.currentWorld.zoom).toBe(4);
    for (let index = 0; index < 5; index++) {
      canvas.dispatch('wheel', { clientX: 400, clientY: 300, deltaY: 1000000, deltaMode: 0, preventDefault() {} } as unknown as Event);
    }
    expect(window.currentWorld.zoom).toBe(0.01);
    elements.get('toggle')!.click();
    renderCameraFrame();
    expect(window.currentWorld.cameraX).toBe(200);
    expect(window.currentWorld.cameraY).toBe(100);
    expect(window.currentWorld.viewMode).toBe('follow');
    for (const listener of windowListeners.get('keydown') ?? []) {
      listener({ code: 'Home', preventDefault() {} } as unknown as Event);
    }
    expect(window.currentWorld.cameraX).toBe(0);
    expect(window.currentWorld.cameraY).toBe(0);
    expect(window.currentWorld.zoom).toBeCloseTo(600 / 25000);
    expect(window.currentWorld.viewMode).toBe('overview');
    renderCameraFrame();
    expect(window.currentWorld.zoom).toBeCloseTo(600 / 25000);
  });

  it('pans even with a selected snake and reserves Shift-drag for God Mode movement', async () => {
    vi.useFakeTimers();
    const { canvas, socket } = await openCameraSession();
    const zoom = window.currentWorld.zoom;
    canvas.dispatch('click', cameraMouse(400 + 200 * zoom, 300 + 100 * zoom));
    canvas.dispatch('mousedown', cameraMouse(400, 300));
    canvas.dispatch('mousemove', cameraMouse(480, 340));
    canvas.dispatch('mouseup', cameraMouse(480, 340));
    canvas.dispatch('click', cameraMouse(480, 340));
    await vi.advanceTimersByTimeAsync(100);
    expect(socket.sent.map(payload => JSON.parse(payload)).filter(packet => packet.type === 'godMode')).toEqual([]);
    elements.get('fitArena')!.click();
    canvas.dispatch('mousedown', cameraMouse(400, 300, 0, true));
    canvas.dispatch('mousemove', cameraMouse(500, 360, 0, true));
    for (const listener of windowListeners.get('mouseup') ?? []) listener(cameraMouse(550, 390, 0, true));
    const moves = socket.sent.map(payload => JSON.parse(payload)).filter(packet => packet.type === 'godMode');
    expect(moves.at(-1)).toMatchObject({ action: 'move', snakeId: 7, x: 150 / zoom, y: 90 / zoom });
  });

  it('attempts the resolved server connection when the entry module loads', async () => {
    await import('./main.ts');

    expect(connectedUrl).toBe('ws://localhost:5174');
  });

  it('keeps camera sliders local and submits the full authoritative panel through Apply and reset', async () => {
    vi.useFakeTimers();
    /** Expose the generated panel inputs to its event wiring and reset collector. */
    const panelInputs = (): SmokeElement[] => createdElements.filter(element => element.dataset['path']);
    for (const id of ['settingsContainer', 'settingsControls']) {
      const panel = document.getElementById(id) as unknown as SmokeElement;
      panel.querySelectorAll = panelInputs;
    }
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'reset-session', tickRate: 60,
      worldSeed: 42, runId: 'reset-run', configRevision: 1, configHash: 'cfg-reset',
      settings: { core: { snakeCount: 12, simSpeed: 1, hiddenLayers: 1, neurons1: 8 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28
    }) });
    const { CFG } = await import('./config.ts');
    for (const [path, value] of [
      ['observer.overviewPadding', 1.5], ['observer.zoomLerpFollow', 0.15],
      ['observer.zoomLerpOverview', 0.2], ['observer.overviewExtraWorldMargin', 400]
    ] as const) {
      const slider = panelInputs().find(element => element.dataset['path'] === path)!;
      slider.value = String(value);
      slider.dispatch('input', new Event('input'));
      expect(CFG.observer[path.slice('observer.'.length) as keyof typeof CFG.observer]).toBe(value);
    }
    vi.advanceTimersByTime(100);
    expect(socket.sent.map(payload => JSON.parse(payload)).filter(packet => packet.type === 'settings')).toEqual([]);
    const radius = panelInputs().find(element => element.dataset['path'] === 'worldRadius')!;
    radius.value = '4200';
    elements.get('apply')!.click();
    const reset = socket.sent.map(payload => JSON.parse(payload)).find(packet => packet.type === 'reset');
    expect(reset).toMatchObject({ settings: { snakeCount: 12 },
      updates: expect.arrayContaining([{ path: 'worldRadius', value: 4200 }]),
      graphSpec: { type: 'graph' } });
    expect(reset.updates.length).toBeGreaterThan(60);
    expect(reset.updates.filter((update: { path: string }) => update.path.startsWith('observer.')))
      .toEqual(expect.arrayContaining([
        { path: 'observer.earlyEndMinSeconds', value: CFG.observer.earlyEndMinSeconds },
        { path: 'observer.earlyEndAliveThreshold', value: CFG.observer.earlyEndAliveThreshold }
      ]));
    expect(reset.updates.filter((update: { path: string }) => update.path.startsWith('observer.'))).toHaveLength(2);
  });

  it('restores an explicitly selected spectator session after reconnect', async () => {
    vi.useFakeTimers();
    await import('./main.ts');
    const first = activeSocket;
    if (!first) throw new Error('missing browser WebSocket');
    const welcome = {
      type: 'welcome', protocolVersion: 2, sessionId: 'spectator-session', tickRate: 60,
      worldSeed: 42, runId: 'spectator-run', configRevision: 0, configHash: 'cfg-spectator',
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native',
        requestedMt: false, activeWorkerCount: 1 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28
    };
    first.onopen?.();
    first.onmessage?.({ data: JSON.stringify(welcome) });
    expect(elements.get('joinOverlay')?.classList.contains('hidden')).toBe(false);

    elements.get('joinSpectate')?.click();
    expect(elements.get('joinOverlay')?.classList.contains('hidden')).toBe(true);
    first.onclose?.();
    expect(elements.get('joinOverlay')?.classList.contains('hidden')).toBe(false);
    await vi.advanceTimersByTimeAsync(1000);

    const second = activeSocket;
    if (!second || second === first) throw new Error('browser did not reconnect');
    second.onopen?.();
    second.onmessage?.({ data: JSON.stringify(welcome) });
    expect(second.sent.map(payload => JSON.parse(payload) as Record<string, unknown>)
      .filter(message => message['type'] === 'join'))
      .toEqual([{ type: 'join', mode: 'spectator' }]);
    expect(elements.get('joinOverlay')?.classList.contains('hidden')).toBe(true);
    expect(elements.get('joinStatus')?.textContent).toBe('Spectating');
  });

  it('downloads the Rust archive through one direct link without fetching population JSON', async () => {
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'archive-session', tickRate: 60,
      worldSeed: 42, runId: 'archive-run', configRevision: 0, configHash: 'cfg-archive',
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28,
      capabilities: { archiveExport: true, archiveImport: true }
    }) });

    elements.get('btnExport')?.click();
    const link = createdElements.find(element => element.href.endsWith('/api/export/latest'));
    expect(link).toMatchObject({
      href: 'http://localhost:5174/api/export/latest',
      download: '', hidden: true, clicked: true, target: '_blank', rel: 'noopener'
    });
    const fetchCalls = vi.mocked(fetch).mock.calls.map(([input]) => String(input));
    expect(fetchCalls.filter(url => url.includes('/api/save') || url.includes('/api/export/latest'))).toEqual([]);
  });

  it.each(['reset', 'newRun', 'import'] as const)('replaces the Hall of Fame view after %s', async reason => {
    const oldEntry = { entryId: '0000000000000001', gen: 1, fitness: 10, seed: 42, points: 2, length: 4 };
    const newEntry = { entryId: '0000000000000002', gen: 7, fitness: 70, seed: 51, points: 3, length: 5 };
    let records = [oldEntry];
    vi.mocked(fetch).mockImplementation(async input => {
      const payload = String(input).includes('/api/hof') ? { hof: [...records] } : { ok: true, presets: [] };
      return { ok: true, json: async () => payload } as Response;
    });
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    const welcome = {
      type: 'welcome', protocolVersion: 2, sessionId: 'hof-session', tickRate: 60,
      worldSeed: 42, runId: 'old-run', configRevision: 0, configHash: 'cfg-hof',
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' }, serializerVersion: 1, frameByteLength: 28
    };
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify(welcome) });
    await vi.waitFor(() => expect(elements.get('hofTable')?.innerHTML).toContain('Gen 1'));
    records = reason === 'import' ? [newEntry] : [];
    socket.onmessage?.({ data: JSON.stringify({ type: 'stateReplaced', reason, checkpointId: 'c'.repeat(64),
      welcome: { ...welcome, runId: 'new-run', worldSeed: 51 } }) });
    expect(elements.get('hofTable')?.innerHTML).toBe('');
    await vi.waitFor(() => expect(elements.get('hofTable')?.innerHTML).toContain(reason === 'import' ? 'Gen 7' : 'No records yet'));
    const { hof } = await import('./hallOfFame.ts');
    expect(await hof.getAll()).toEqual(records);
  });

  it('refreshes compact production Hall of Fame records once when a generation advances', async () => {
    const entry = { entryId: '0000000000000001', gen: 1, fitness: 10, seed: 42, points: 2, length: 4 };
    let records: typeof entry[] = [];
    vi.mocked(fetch).mockImplementation(async input => {
      const payload = String(input).includes('/api/hof') ? { hof: [...records] } : { ok: true, presets: [] };
      return { ok: true, json: async () => payload } as Response;
    });
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'hof-session', tickRate: 60,
      worldSeed: 42, runId: 'run', configRevision: 0, configHash: 'cfg-hof',
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' }, serializerVersion: 1, frameByteLength: 28,
      capabilities: { archiveExport: true }
    }) });
    await vi.waitFor(() => expect(elements.get('hofTable')?.innerHTML).toContain('No records yet'));
    records = [entry];
    const stats = { type: 'stats', tick: 480, gen: 2, generationTime: 0, generationSeconds: 8,
      alive: 12, fps: 60 };
    socket.onmessage?.({ data: JSON.stringify(stats) });
    await vi.waitFor(() => expect(elements.get('hofTable')?.innerHTML).toContain('Gen 1'));
    const loads = vi.mocked(fetch).mock.calls.filter(([input]) => String(input).includes('/api/hof')).length;
    socket.onmessage?.({ data: JSON.stringify({ ...stats, tick: 481 }) });
    await Promise.resolve();
    expect(vi.mocked(fetch).mock.calls.filter(([input]) => String(input).includes('/api/hof'))).toHaveLength(loads);
  });

  it('uploads the selected Rust archive File unchanged', async () => {
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'archive-import-session', tickRate: 60,
      worldSeed: 42, runId: 'archive-run', configRevision: 0, configHash: 'cfg-archive',
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28,
      capabilities: { archiveExport: true, archiveImport: true }
    }) });

    let uploadUrl = '';
    let sentBody: unknown;
    class StubArchiveRequest {
      /** Browser upload progress hook installed by the UI. */
      upload = { onprogress: null as (() => void) | null };
      /** Complete response callback installed by the UI. */
      onload: (() => void) | null = null;
      /** Connection error callback installed by the UI. */
      onerror: (() => void) | null = null;
      /** Cancellation callback installed by the UI. */
      onabort: (() => void) | null = null;
      /** Upload timeout selected by the UI. */
      timeout = 0;
      /** Successful small result status. */
      status = 200;
      /** Small scalar result; no population bytes. */
      responseText = JSON.stringify({ ok: true, runId: 'archive-run',
        generation: '0000000000000001', checkpointId: 'a'.repeat(64) });
      /** Capture the direct archive endpoint. */
      open(_method: string, url: string): void { uploadUrl = url; }
      /** Accept the archive media type. */
      setRequestHeader(): void { }
      /** Record exact object identity and resolve the small response. */
      send(body: unknown): void { sentBody = body; this.onload?.(); }
    }
    vi.stubGlobal('XMLHttpRequest', StubArchiveRequest);
    vi.stubGlobal('FileReader', class { constructor() { throw new Error('FileReader must not inspect Rust archives'); } });
    vi.stubGlobal('alert', vi.fn());
    const file = new File([new Uint8Array([1, 2, 3])], 'checkpoint.save',
      { type: 'application/vnd.slither-neuroevo.save' });
    const input = elements.get('fileInput');
    if (!input) throw new Error('missing archive file input');
    input.files = { length: 1, item: () => file } as unknown as FileList;
    input.dispatch('change', { target: input } as unknown as Event);
    await vi.waitFor(() => expect(sentBody).toBe(file));
    expect(uploadUrl).toBe('http://localhost:5174/api/import/archive');
    const fetchCalls = vi.mocked(fetch).mock.calls.map(([request]) => String(request));
    expect(fetchCalls.filter(url => url.includes('/api/import'))).toEqual([]);
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

  it('makes a recovered Rust lineage visible with exact provenance', async () => {
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'recovery-session', tickRate: 60,
      worldSeed: 42, runId: 'branch-run', configRevision: 0, configHash: 'cfg-recovery',
      recovery: { failedRunId: 'source-run', branchRunId: 'branch-run',
        failedCheckpointId: 'f'.repeat(64), recoveredCheckpointId: 'a'.repeat(64),
        recoveredGeneration: '0000000000000002', lostCompletedGenerations: null },
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28
    }) });
    expect(elements.get('connectionStatus')?.textContent)
      .toBe('Server · recovered · seed 42 · native single-thread');
    expect(elements.get('connectionStatus')?.getAttribute('title'))
      .toContain('at generation 2 from failed run source-run into branch branch-run');
  });

  it('warns visibly when an old SQLite save supplied only the population', async () => {
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({
      type: 'welcome', protocolVersion: 2, sessionId: 'converted-session', tickRate: 60,
      worldSeed: 42, runId: 'converted-run', configRevision: 0, configHash: 'cfg-converted',
      legacyConversion: { sourceSnapshotId: 17, sourceFormat: 'legacy-gzip',
        completeness: 'population-only', exactContinuation: false },
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28
    }) });
    expect(elements.get('connectionStatus')?.textContent)
      .toBe('Server · converted save · seed 42 · native single-thread');
    expect(elements.get('connectionStatus')?.getAttribute('title'))
      .toContain('snapshot 17. The population was converted, but this is a new run');
  });

  it('makes an exact imported branch visible after live state replacement', async () => {
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');
    socket.onopen?.();
    const welcome = {
      type: 'welcome', protocolVersion: 2, sessionId: 'import-session', tickRate: 60,
      worldSeed: 51, runId: 'branch-run', configRevision: 0, configHash: 'cfg-import',
      importBranch: { sourceRunId: 'source-run', branchRunId: 'branch-run',
        sourceGeneration: '0000000000000007', sourceCheckpointId: 'c'.repeat(64) },
      settings: { core: { simSpeed: 1 }, updates: [] },
      inferenceMode: { requestedBackend: 'native', activeBackend: 'native', requestedMt: false, activeWorkerCount: 0 },
      sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
      serializerVersion: 1, frameByteLength: 28
    };
    socket.onmessage?.({ data: JSON.stringify({
      type: 'stateReplaced', reason: 'import', checkpointId: 'c'.repeat(64), welcome
    }) });
    expect(elements.get('connectionStatus')?.textContent)
      .toBe('Server · imported branch · seed 51 · native single-thread');
    expect(elements.get('connectionStatus')?.getAttribute('title'))
      .toContain('at generation 7 from run source-run into branch branch-run');
  });

  it('sends canvas steering and boost release without another sensor or display frame', async () => {
    vi.useFakeTimers();
    await import('./main.ts');
    const socket = activeSocket;
    if (!socket) throw new Error('missing browser WebSocket');

    socket.onopen?.();
    socket.onmessage?.({
      data: JSON.stringify({
        type: 'welcome',
        protocolVersion: 2,
        sessionId: 'player-session',
        tickRate: 60,
        worldSeed: 42,
        runId: 'run-player',
        configRevision: 0,
        configHash: 'cfg-player',
        settings: { core: { simSpeed: 1 }, updates: [] },
        inferenceMode: {
          requestedBackend: 'native',
          activeBackend: 'native',
          requestedMt: false,
          activeWorkerCount: 0
        },
        sensorSpec: { sensorCount: 83, order: [], layoutVersion: 'v3' },
        serializerVersion: 1,
        frameByteLength: 28
      })
    });
    const name = elements.get('joinName');
    if (!name) throw new Error('missing join-name input');
    name.value = 'Pointer';
    elements.get('joinPlay')?.click();
    socket.onmessage?.({
      data: JSON.stringify({
        type: 'assign',
        snakeId: 100_000,
        controller: 'player',
        resumeToken: 'browser-input-token'
      })
    });
    socket.onmessage?.({
      data: JSON.stringify({
        type: 'sensors',
        tick: 9,
        snakeId: 100_000,
        sensors: [],
        meta: { x: 0, y: 0, dir: 0 }
      })
    });

    const canvas = elements.get('c');
    if (!canvas) throw new Error('missing game canvas');
    canvas.dispatch('mousemove', {
      clientX: 400,
      clientY: 0,
      button: 0
    } as unknown as Event);
    canvas.dispatch('mousedown', {
      clientX: 400,
      clientY: 0,
      button: 0
    } as unknown as Event);
    await vi.advanceTimersByTimeAsync(17);

    canvas.dispatch('mousemove', {
      clientX: 400,
      clientY: 600,
      button: 0
    } as unknown as Event);
    for (const listener of windowListeners.get('mouseup') ?? []) {
      listener({
        clientX: 400,
        clientY: 600,
        button: 0
      } as unknown as Event);
    }
    await vi.advanceTimersByTimeAsync(17);

    const actions = socket.sent
      .map(payload => JSON.parse(payload) as Record<string, unknown>)
      .filter(message => message['type'] === 'action');
    expect(actions.length).toBeGreaterThanOrEqual(3);
    expect(actions.at(-2)).toMatchObject({
      snakeId: 100_000,
      tick: 10,
      turn: -1,
      boost: 1
    });
    expect(actions.at(-1)).toMatchObject({
      snakeId: 100_000,
      tick: 10,
      turn: 1,
      boost: 0
    });

    elements.get('joinSpectate')?.click();
  });
});
