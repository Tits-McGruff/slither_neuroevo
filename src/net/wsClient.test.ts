import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  createWsClient,
  resolveServerUrl,
  storeServerUrl,
  DEFAULT_SERVER_URL
} from './wsClient.ts';

type WebSocketCtor = new (...args: unknown[]) => {
  readyState: number;
  onopen: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onerror: (() => void) | null;
  onclose: (() => void) | null;
  binaryType: string;
  send: (...args: unknown[]) => void;
  close: () => void;
};

type TestGlobal = typeof globalThis & {
  window?: { location: { search: string } };
  localStorage?: Storage;
  WebSocket?: WebSocketCtor;
};

const globalAny = globalThis as TestGlobal;

describe('wsClient', () => {
  let originalWindow: unknown;
  let originalStorage: unknown;
  let originalWebSocket: unknown;

  beforeEach(() => {
    vi.resetModules();
    originalWindow = globalAny.window;
    originalStorage = globalAny.localStorage;
    originalWebSocket = globalAny.WebSocket;

    globalAny.window = {
      location: { search: '' }
    };
    const store = new Map<string, string>();
    globalAny.localStorage = {
      getItem: (key: string) => (store.has(key) ? store.get(key) ?? null : null),
      setItem: (key: string, value: string) => {
        store.set(key, value);
      }
    };
  });

  afterEach(() => {
    globalAny.window = originalWindow;
    globalAny.localStorage = originalStorage;
    globalAny.WebSocket = originalWebSocket;
  });

  it('resolves server url from query param', () => {
    globalAny.window.location.search = '?server=ws://example:9000';
    expect(resolveServerUrl()).toBe('ws://example:9000');
  });

  it('falls back to localStorage then default', () => {
    storeServerUrl('ws://stored:1234');
    expect(resolveServerUrl()).toBe('ws://stored:1234');

    globalAny.localStorage = {
      getItem: () => null,
      setItem: () => {}
    };
    expect(resolveServerUrl()).toBe(DEFAULT_SERVER_URL);
  });

  it('dispatches welcome and frame messages', () => {
    class StubWebSocket {
      static OPEN = 1;
      static instances: StubWebSocket[] = [];
      readyState = 1;
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: unknown }) => void) | null = null;
      onerror: (() => void) | null = null;
      onclose: (() => void) | null = null;
      binaryType = 'arraybuffer';

      constructor() {
        StubWebSocket.instances.push(this);
      }

      send(): void {}
      close(): void {
        this.readyState = 3;
        this.onclose?.();
      }

      open(): void {
        this.onopen?.();
      }

      emit(data: unknown): void {
        this.onmessage?.({ data });
      }
    }

    globalAny.WebSocket = StubWebSocket as unknown as WebSocketCtor;

    let sawWelcome = false;
    let sawFrame = false;
    const client = createWsClient({
      onConnected: () => {
        sawWelcome = true;
      },
      onDisconnected: () => {},
      onFrame: () => {
        sawFrame = true;
      },
      onStats: () => {}
    });

    client.connect('ws://localhost:9999');
    const instance = StubWebSocket.instances[0];
    expect(instance).toBeDefined();
    if (!instance) {
      throw new Error('Expected WebSocket instance');
    }
    instance.open();
    instance.emit(JSON.stringify({ type: 'welcome', tickRate: 60 }));
    instance.emit(new ArrayBuffer(8));

    expect(sawWelcome).toBe(true);
    expect(sawFrame).toBe(true);
  });
});
