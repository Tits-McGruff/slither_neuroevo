import { WebSocket, WebSocketServer } from 'ws';
import { parseClientMessage } from './protocol.ts';
import type {
  ActionMsg,
  ClientType,
  JoinMode,
  JoinMsg,
  ServerMessage,
  StatsMsg,
  WelcomeMsg
} from './protocol.ts';

const DEFAULT_MAX_MESSAGE_BYTES = 64 * 1024;
const DEFAULT_MAX_BUFFERED_BYTES = 512 * 1024;

export interface ConnectionState {
  id: number;
  socket: WebSocket;
  clientType: 'unknown' | ClientType;
  joined: boolean;
  mode?: JoinMode;
  lastMessageTime: number;
}

export interface WsHubOptions {
  maxMessageBytes?: number;
  maxBufferedAmount?: number;
}

export interface WsHubHandlers {
  onJoin?: (connId: number, msg: JoinMsg, clientType: ClientType) => void;
  onAction?: (connId: number, msg: ActionMsg) => void;
  onDisconnect?: (connId: number) => void;
}

export class WsHub {
  private wss: WebSocketServer;
  private connections = new Map<number, ConnectionState>();
  private nextId = 1;
  private welcomeJson: string;
  private maxMessageBytes: number;
  private maxBufferedAmount: number;
  private handlers: WsHubHandlers | null;

  constructor(
    httpServer: any,
    welcome: WelcomeMsg,
    options: WsHubOptions = {},
    handlers?: WsHubHandlers
  ) {
    this.maxMessageBytes = options.maxMessageBytes ?? DEFAULT_MAX_MESSAGE_BYTES;
    this.maxBufferedAmount = options.maxBufferedAmount ?? DEFAULT_MAX_BUFFERED_BYTES;
    this.wss = new WebSocketServer({
      server: httpServer,
      maxPayload: this.maxMessageBytes
    });
    this.welcomeJson = JSON.stringify(welcome);
    this.handlers = handlers ?? null;
    this.wss.on('connection', (socket) => this.handleConnection(socket));
  }

  setHandlers(handlers: WsHubHandlers): void {
    this.handlers = handlers;
  }

  getClientCount(): number {
    return this.connections.size;
  }

  closeAll(): void {
    for (const state of this.connections.values()) {
      state.socket.close();
    }
    this.connections.clear();
    this.wss.close();
  }

  broadcastFrame(buffer: ArrayBuffer | ArrayBufferView): void {
    for (const state of this.connections.values()) {
      if (state.clientType !== 'ui' || !state.joined) continue;
      if (state.socket.readyState !== WebSocket.OPEN) continue;
      if (state.socket.bufferedAmount > this.maxBufferedAmount) continue;
      state.socket.send(buffer, { binary: true });
    }
  }

  broadcastStats(stats: StatsMsg): void {
    const payload = JSON.stringify(stats);
    for (const state of this.connections.values()) {
      if (!state.joined) continue;
      if (state.socket.readyState !== WebSocket.OPEN) continue;
      if (state.socket.bufferedAmount > this.maxBufferedAmount) continue;
      state.socket.send(payload);
    }
  }

  sendJsonTo(connId: number, payload: ServerMessage): void {
    const state = this.connections.get(connId);
    if (!state || !state.joined) return;
    if (state.socket.readyState !== WebSocket.OPEN) return;
    if (state.socket.bufferedAmount > this.maxBufferedAmount) return;
    state.socket.send(JSON.stringify(payload));
  }

  private handleConnection(socket: WebSocket): void {
    const state: ConnectionState = {
      id: this.nextId++,
      socket,
      clientType: 'unknown',
      joined: false,
      lastMessageTime: Date.now()
    };
    this.connections.set(state.id, state);
    socket.on('message', (data, isBinary) => this.handleMessage(state, data, isBinary));
    socket.on('close', () => {
      this.connections.delete(state.id);
      this.handlers?.onDisconnect?.(state.id);
    });
  }

  private handleMessage(state: ConnectionState, data: any, isBinary: boolean): void {
    const size = payloadSize(data);
    if (size > this.maxMessageBytes) {
      this.protocolError(state, 'message too large');
      return;
    }
    if (isBinary) {
      this.protocolError(state, 'binary messages are not supported');
      return;
    }
    const text = payloadToText(data);
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      this.protocolError(state, 'invalid JSON');
      return;
    }
    const msg = parseClientMessage(parsed);
    if (!msg) {
      this.protocolError(state, 'invalid message');
      return;
    }
    state.lastMessageTime = Date.now();
    switch (msg.type) {
      case 'hello':
        if (state.clientType !== 'unknown') {
          this.protocolError(state, 'duplicate hello');
          return;
        }
        state.clientType = msg.clientType;
        state.socket.send(this.welcomeJson);
        return;
      case 'join':
        if (state.clientType === 'unknown') {
          this.protocolError(state, 'hello required before join');
          return;
        }
        state.joined = true;
        state.mode = msg.mode;
        this.handlers?.onJoin?.(state.id, msg, state.clientType);
        return;
      case 'action':
        if (!state.joined) {
          this.protocolError(state, 'join required before action');
          return;
        }
        if (state.mode !== 'player') {
          this.protocolError(state, 'action requires player mode');
          return;
        }
        this.handlers?.onAction?.(state.id, msg);
        return;
      case 'ping':
        return;
      default:
        this.protocolError(state, 'unknown message');
    }
  }

  private protocolError(state: ConnectionState, message: string): void {
    if (state.socket.readyState === WebSocket.OPEN) {
      state.socket.send(JSON.stringify({ type: 'error', message }));
    }
    state.socket.close(1008, message);
  }
}

function payloadSize(data: any): number {
  if (typeof data === 'string') return Buffer.byteLength(data, 'utf8');
  if (Buffer.isBuffer(data)) return data.byteLength;
  if (data instanceof ArrayBuffer) return data.byteLength;
  if (ArrayBuffer.isView(data)) return data.byteLength;
  return 0;
}

function payloadToText(data: any): string {
  if (typeof data === 'string') return data;
  if (Buffer.isBuffer(data)) return data.toString('utf8');
  if (data instanceof ArrayBuffer) return Buffer.from(data).toString('utf8');
  if (ArrayBuffer.isView(data)) {
    return Buffer.from(data.buffer, data.byteOffset, data.byteLength).toString('utf8');
  }
  return String(data ?? '');
}
