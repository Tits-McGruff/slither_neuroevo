import type { FitnessData, FitnessHistoryEntry, HallOfFameEntry, VizData } from '../protocol/messages.ts';
import type { GraphSpec } from '../brains/graph/schema.ts';
import type { CoreSettings, SettingsUpdate } from '../protocol/settings.ts';

/** Default WebSocket URL injected at build time. */
declare const __SLITHER_DEFAULT_WS_URL__: string | undefined;
/** Server port injected at build time. */
declare const __SLITHER_SERVER_PORT__: number | undefined;

/** Welcome message payload from the server. */
export interface WelcomeMsg {
  type: 'welcome';
  sessionId: string;
  tickRate: number;
  worldSeed: number;
  cfgHash: string;
  sensorSpec: { sensorCount: number; order: string[] };
  serializerVersion: number;
  frameByteLength: number;
}

/** Stats message payload from the server. */
export interface StatsMsg {
  type: 'stats';
  tick: number;
  gen: number;
  alive: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
  viz?: VizData;
  hofEntry?: HallOfFameEntry;
}

/** Action message payload sent to the server. */
export interface ActionMsg {
  type: 'action';
  tick: number;
  snakeId: number;
  turn: number;
  boost: number;
}

/** Assignment message for controlled snakes. */
export interface AssignMsg {
  type: 'assign';
  snakeId: number;
  controller: 'player' | 'bot';
}

/** Sensor message payload for controlled snakes. */
export interface SensorsMsg {
  type: 'sensors';
  tick: number;
  snakeId: number;
  sensors: number[];
  meta?: { x: number; y: number; dir: number };
}

/** Error message payload from the server. */
export interface ErrorMsg {
  type: 'error';
  message: string;
}

/** Reset request payload sent to the server. */
export interface ResetMsg {
  type: 'reset';
  settings: CoreSettings;
  updates?: SettingsUpdate[];
  graphSpec?: GraphSpec | null;
}

/** Callback handlers for the websocket client lifecycle and messages. */
export interface WsClientCallbacks {
  onConnected: (info: WelcomeMsg) => void;
  onDisconnected: () => void;
  onFrame: (buffer: ArrayBuffer) => void;
  onStats: (msg: StatsMsg) => void;
  onAssign?: (msg: AssignMsg) => void;
  onSensors?: (msg: SensorsMsg) => void;
  onError?: (msg: ErrorMsg) => void;
}

/** WebSocket client API used by the main thread. */
export interface WsClient {
  connect: (url: string) => void;
  disconnect: () => void;
  sendJoin: (mode: 'spectator' | 'player', name?: string) => void;
  sendAction: (tick: number, snakeId: number, turn: number, boost: number) => void;
  sendView: (payload: { viewW?: number; viewH?: number; mode?: 'overview' | 'follow' | 'toggle' }) => void;
  sendViz: (enabled: boolean) => void;
  sendReset: (settings: CoreSettings, updates: SettingsUpdate[], graphSpec?: GraphSpec | null) => void;
  isConnected: () => boolean;
}

/** Default server URL used when none is configured. */
/** Build-time server URL from Vite when configured. */
const INJECTED_SERVER_URL =
  typeof __SLITHER_DEFAULT_WS_URL__ === 'string' ? __SLITHER_DEFAULT_WS_URL__ : '';
/** Default server URL used when no runtime host is available. */
export const DEFAULT_SERVER_URL = 'ws://localhost:5174';

/**
 * Format a host for URL usage, adding brackets for IPv6 literals.
 * @param host - Hostname or IP literal.
 * @returns Host string safe for URL assembly.
 */
function formatHostForUrl(host: string): string {
  if (!host) return host;
  return host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
}

/**
 * Resolve the default server URL from injected config and runtime location.
 * @returns Default WebSocket URL when no explicit override is provided.
 */
export function getDefaultServerUrl(): string {
  const injected = INJECTED_SERVER_URL.trim();
  if (injected) return injected;
  const port =
    typeof __SLITHER_SERVER_PORT__ === 'number' && Number.isFinite(__SLITHER_SERVER_PORT__)
      ? __SLITHER_SERVER_PORT__
      : 5174;
  if (typeof window !== 'undefined' && window.location) {
    const host = window.location.hostname || '';
    if (host) {
      const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
      return `${protocol}://${formatHostForUrl(host)}:${port}`;
    }
  }
  return DEFAULT_SERVER_URL;
}
/** Handshake timeout in milliseconds before forcing reconnect. */
const HANDSHAKE_TIMEOUT_MS = 1500;
/** Local storage key for persisting the server URL. */
const STORAGE_KEY = 'slither_server_url';

/**
 * Resolve the server URL from query params, local storage, or runtime defaults.
 * @param defaultUrl - Fallback URL when none is provided.
 * @returns Resolved WebSocket URL.
 */
export function resolveServerUrl(defaultUrl = getDefaultServerUrl()): string {
  const search = typeof window !== 'undefined' && window.location ? window.location.search : '';
  const params = new URLSearchParams(search || '');
  const paramUrl = params.get('server');
  if (paramUrl) return paramUrl;
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(STORAGE_KEY);
  } catch {
    stored = null;
  }
  return stored || defaultUrl;
}

/**
 * Store the server URL in local storage.
 * @param url - WebSocket URL to persist.
 */
export function storeServerUrl(url: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, url);
  } catch {
    // Ignore storage failures in non-browser environments.
  }
}

/**
 * Create a WebSocket client wrapper with typed callbacks.
 * @param callbacks - Lifecycle and message callbacks.
 * @returns WebSocket client instance.
 */
export function createWsClient(callbacks: WsClientCallbacks): WsClient {
  let socket: WebSocket | null = null;
  let connected = false;
  let handshakeTimer: ReturnType<typeof setTimeout> | null = null;

  const clearHandshakeTimer = (): void => {
    if (handshakeTimer === null) return;
    clearTimeout(handshakeTimer);
    handshakeTimer = null;
  };

  const connect = (url: string): void => {
    if (socket) disconnect();
    socket = new WebSocket(url);
    socket.binaryType = 'arraybuffer';
    socket.onopen = () => {
      connected = false;
      socket?.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 1 }));
      clearHandshakeTimer();
      handshakeTimer = setTimeout(() => {
        if (connected || !socket) return;
        callbacks.onError?.({ type: 'error', message: 'Handshake timed out' });
        socket.close();
      }, HANDSHAKE_TIMEOUT_MS);
    };
    socket.onmessage = (event) => {
      handleMessage(event.data);
    };
    socket.onerror = () => {
      callbacks.onError?.({ type: 'error', message: 'WebSocket error' });
    };
    socket.onclose = () => {
      clearHandshakeTimer();
      connected = false;
      callbacks.onDisconnected();
    };
  };

  const disconnect = (): void => {
    if (!socket) return;
    socket.onopen = null;
    socket.onmessage = null;
    socket.onerror = null;
    socket.onclose = null;
    socket.close();
    socket = null;
    connected = false;
    clearHandshakeTimer();
  };

  const sendJoin = (mode: 'spectator' | 'player', name?: string): void => {
    if (!socket || socket.readyState !== WebSocket.OPEN || !connected) return;
    const payload = name ? { type: 'join', mode, name } : { type: 'join', mode };
    socket.send(JSON.stringify(payload));
  };

  const sendAction = (tick: number, snakeId: number, turn: number, boost: number): void => {
    if (!socket || socket.readyState !== WebSocket.OPEN || !connected) return;
    const payload: ActionMsg = { type: 'action', tick, snakeId, turn, boost };
    socket.send(JSON.stringify(payload));
  };

  const sendView = (payload: { viewW?: number; viewH?: number; mode?: 'overview' | 'follow' | 'toggle' }): void => {
    if (!socket || socket.readyState !== WebSocket.OPEN || !connected) return;
    socket.send(JSON.stringify({ type: 'view', ...payload }));
  };

  const sendViz = (enabled: boolean): void => {
    if (!socket || socket.readyState !== WebSocket.OPEN || !connected) return;
    socket.send(JSON.stringify({ type: 'viz', enabled }));
  };

  const sendReset = (
    settings: CoreSettings,
    updates: SettingsUpdate[],
    graphSpec?: GraphSpec | null
  ): void => {
    if (!socket || socket.readyState !== WebSocket.OPEN || !connected) return;
    const payload: ResetMsg = { type: 'reset', settings, updates };
    if (graphSpec !== undefined) payload.graphSpec = graphSpec;
    socket.send(JSON.stringify(payload));
  };

  const isConnected = (): boolean => connected;

  const handleMessage = (data: unknown): void => {
    if (data instanceof ArrayBuffer) {
      callbacks.onFrame(data);
      return;
    }
    if (typeof data === 'string') {
      handleJson(data);
      return;
    }
    if (data instanceof Blob) {
      data
        .text()
        .then(handleJson)
        .catch(() => {
          callbacks.onError?.({ type: 'error', message: 'Invalid message payload' });
        });
      return;
    }
  };

  const handleJson = (raw: string): void => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw) as unknown;
    } catch {
      callbacks.onError?.({ type: 'error', message: 'Invalid JSON message' });
      return;
    }
    if (!parsed || typeof parsed !== 'object') return;
    const msg = parsed as Record<string, unknown>;
    if (typeof msg['type'] !== 'string') return;
    switch (msg['type']) {
      case 'welcome':
        connected = true;
        clearHandshakeTimer();
        callbacks.onConnected(msg as unknown as WelcomeMsg);
        return;
      case 'stats':
        callbacks.onStats(msg as unknown as StatsMsg);
        return;
      case 'assign':
        callbacks.onAssign?.(msg as unknown as AssignMsg);
        return;
      case 'sensors':
        callbacks.onSensors?.(msg as unknown as SensorsMsg);
        return;
      case 'error':
        callbacks.onError?.(msg as unknown as ErrorMsg);
        return;
      default:
        return;
    }
  };

  return {
    connect,
    disconnect,
    sendJoin,
    sendAction,
    sendView,
    sendViz,
    sendReset,
    isConnected
  };
}
