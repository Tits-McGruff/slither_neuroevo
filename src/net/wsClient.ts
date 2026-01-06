import type { FitnessData, FitnessHistoryEntry, VizData } from '../protocol/messages.ts';

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

export interface StatsMsg {
  type: 'stats';
  tick: number;
  gen: number;
  alive: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
  viz?: VizData;
}

export interface ActionMsg {
  type: 'action';
  tick: number;
  snakeId: number;
  turn: number;
  boost: number;
}

export interface AssignMsg {
  type: 'assign';
  snakeId: number;
  controller: 'player' | 'bot';
}

export interface SensorsMsg {
  type: 'sensors';
  tick: number;
  snakeId: number;
  sensors: number[];
  meta?: { x: number; y: number; dir: number };
}

export interface ErrorMsg {
  type: 'error';
  message: string;
}

export interface WsClientCallbacks {
  onConnected: (info: WelcomeMsg) => void;
  onDisconnected: () => void;
  onFrame: (buffer: ArrayBuffer) => void;
  onStats: (msg: StatsMsg) => void;
  onAssign?: (msg: AssignMsg) => void;
  onSensors?: (msg: SensorsMsg) => void;
  onError?: (msg: ErrorMsg) => void;
}

export interface WsClient {
  connect: (url: string) => void;
  disconnect: () => void;
  sendJoin: (mode: 'spectator' | 'player', name?: string) => void;
  sendAction: (tick: number, snakeId: number, turn: number, boost: number) => void;
  sendView: (payload: { viewW?: number; viewH?: number; mode?: 'overview' | 'follow' | 'toggle' }) => void;
  sendViz: (enabled: boolean) => void;
  isConnected: () => boolean;
}

export const DEFAULT_SERVER_URL = 'ws://localhost:5174';
const HANDSHAKE_TIMEOUT_MS = 1500;
const STORAGE_KEY = 'slither_server_url';

export function resolveServerUrl(defaultUrl = DEFAULT_SERVER_URL): string {
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

export function storeServerUrl(url: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, url);
  } catch {
    // Ignore storage failures in non-browser environments.
  }
}

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
    if (typeof msg.type !== 'string') return;
    switch (msg.type) {
      case 'welcome':
        connected = true;
        clearHandshakeTimer();
        callbacks.onConnected(msg as WelcomeMsg);
        return;
      case 'stats':
        callbacks.onStats(msg as StatsMsg);
        return;
      case 'assign':
        callbacks.onAssign?.(msg as AssignMsg);
        return;
      case 'sensors':
        callbacks.onSensors?.(msg as SensorsMsg);
        return;
      case 'error':
        callbacks.onError?.(msg as ErrorMsg);
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
    isConnected
  };
}
