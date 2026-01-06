import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  createWsClient,
  resolveServerUrl,
  storeServerUrl,
  DEFAULT_SERVER_URL
} from './wsClient.ts';

/** Global window and storage overrides for websocket client tests. */
type TestGlobal = typeof globalThis & {
  window?: Window & typeof globalThis;
  localStorage?: Storage;
  WebSocket?: typeof WebSocket;
};

/** Global alias with test-specific WebSocket overrides. */
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

    globalAny.window = { location: { search: '' } } as unknown as Window & typeof globalThis;
    const store = new Map<string, string>();
    globalAny.localStorage = {
      length: 0,
      getItem: (key: string) => (store.has(key) ? store.get(key) ?? null : null),
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
      removeItem: (key: string) => {
        store.delete(key);
      },
      clear: () => {
        store.clear();
      },
      key: () => null
    } as Storage;
  });

  afterEach(() => {
    globalAny.window = originalWindow as Window & typeof globalThis;
    globalAny.localStorage = originalStorage as Storage;
    globalAny.WebSocket = originalWebSocket as typeof WebSocket;
  });

  it('resolves server url from query param', () => {
    globalAny.window.location.search = '?server=ws://example:9000';
    expect(resolveServerUrl()).toBe('ws://example:9000');
  });

  it('falls back to localStorage then default', () => {
    storeServerUrl('ws://stored:1234');
    expect(resolveServerUrl()).toBe('ws://stored:1234');

    globalAny.localStorage = {
      length: 0,
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
      clear: () => {},
      key: () => null
    } as Storage;
    expect(resolveServerUrl()).toBe(DEFAULT_SERVER_URL);
  });

  it('dispatches welcome and frame messages', () => {
    /** WebSocket stub used to simulate connection events. */
    class StubWebSocket {
      /** Ready state value for open sockets. */
      static OPEN = 1;
      /** Collected instances for inspection. */
      static instances: StubWebSocket[] = [];
      /** Current ready state value. */
      readyState = 1;
      /** Open handler hook. */
      onopen: (() => void) | null = null;
      /** Message handler hook. */
      onmessage: ((event: { data: unknown }) => void) | null = null;
      /** Error handler hook. */
      onerror: (() => void) | null = null;
      /** Close handler hook. */
      onclose: (() => void) | null = null;
      /** Binary type preference for messages. */
      binaryType = 'arraybuffer';

      /** Create a stub socket and register it in the instance list. */
      constructor() {
        StubWebSocket.instances.push(this);
      }

      /** No-op send implementation. */
      send(): void {}
      /** Close the socket and emit a close event. */
      close(): void {
        this.readyState = 3;
        this.onclose?.();
      }

      /** Emit an open event for the socket. */
      open(): void {
        this.onopen?.();
      }

      /**
       * Emit a message event with provided data.
       * @param data - Payload to emit.
       */
      emit(data: unknown): void {
        this.onmessage?.({ data });
      }
    }

    globalAny.WebSocket = StubWebSocket as unknown as typeof WebSocket;

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
