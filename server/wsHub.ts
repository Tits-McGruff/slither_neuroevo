import { WebSocket, WebSocketServer } from 'ws';
import type { RawData } from 'ws';
import type { Server } from 'node:http';
import { parseClientMessage } from './protocol.ts';
import type {
  ActionMsg,
  ClientType,
  JoinMode,
  JoinMsg,
  ViewMsg,
  VizMsg,
  ServerMessage,
  StatsMsg,
  WelcomeMsg
} from './protocol.ts';

/** Default max size accepted for a single client message. */
const DEFAULT_MAX_MESSAGE_BYTES = 64 * 1024;
/** Default max buffered bytes before we stop sending to a client. */
const DEFAULT_MAX_BUFFERED_BYTES = 512 * 1024;

/** Per-connection state tracked by the websocket hub. */
export interface ConnectionState {
  id: number;
  socket: WebSocket;
  clientType: 'unknown' | ClientType;
  joined: boolean;
  mode?: JoinMode;
  lastMessageTime: number;
}

/** Optional hub configuration overrides. */
export interface WsHubOptions {
  maxMessageBytes?: number;
  maxBufferedAmount?: number;
}

/** Event handlers invoked by the websocket hub. */
export interface WsHubHandlers {
  onJoin?: (connId: number, msg: JoinMsg, clientType: ClientType) => void;
  onAction?: (connId: number, msg: ActionMsg) => void;
  onView?: (connId: number, msg: ViewMsg) => void;
  onViz?: (connId: number, msg: VizMsg) => void;
  onDisconnect?: (connId: number) => void;
}

/** WebSocket hub responsible for client connections and routing messages. */
export class WsHub {
  /** Underlying WebSocket server instance. */
  private wss: WebSocketServer;
  /** Active connection state keyed by id. */
  private connections = new Map<number, ConnectionState>();
  /** Next connection id to assign. */
  private nextId = 1;
  /** Cached JSON payload for welcome messages. */
  private welcomeJson: string;
  /** Maximum accepted message size in bytes. */
  private maxMessageBytes: number;
  /** Maximum buffered outbound bytes per socket. */
  private maxBufferedAmount: number;
  /** Registered event handlers for hub callbacks. */
  private handlers: WsHubHandlers | null;

  /**
   * Create a WebSocket hub for the given HTTP server.
   * @param httpServer - HTTP server to attach the WebSocket server to.
   * @param welcome - Welcome payload to send after `hello`.
   * @param options - Optional connection limits.
   * @param handlers - Optional event handlers.
   */
  constructor(
    httpServer: Server,
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

  /**
   * Set or replace the event handlers for hub callbacks.
   * @param handlers - Callback set to register.
   */
  setHandlers(handlers: WsHubHandlers): void {
    this.handlers = handlers;
  }

  /**
   * Return the current connected client count.
   * @returns Active connection count.
   */
  getClientCount(): number {
    return this.connections.size;
  }

  /**
   * Close and clear all active connections.
   */
  closeAll(): void {
    for (const state of this.connections.values()) {
      state.socket.close();
    }
    this.connections.clear();
    this.wss.close();
  }

  /**
   * Broadcast a binary frame buffer to UI clients.
   * @param buffer - Serialized world frame buffer.
   */
  broadcastFrame(buffer: ArrayBuffer | ArrayBufferView): void {
    for (const state of this.connections.values()) {
      if (state.clientType !== 'ui' || !state.joined) continue;
      if (state.socket.readyState !== WebSocket.OPEN) continue;
      if (state.socket.bufferedAmount > this.maxBufferedAmount) continue;
      state.socket.send(buffer, { binary: true });
    }
  }

  /**
   * Broadcast a stats payload to all joined clients.
   * @param stats - Stats message payload.
   */
  broadcastStats(stats: StatsMsg): void {
    const payload = JSON.stringify(stats);
    for (const state of this.connections.values()) {
      if (!state.joined) continue;
      if (state.socket.readyState !== WebSocket.OPEN) continue;
      if (state.socket.bufferedAmount > this.maxBufferedAmount) continue;
      state.socket.send(payload);
    }
  }

  /**
   * Send a JSON payload to a specific client by connection id.
   * @param connId - Connection id to target.
   * @param payload - Server message to send.
   */
  sendJsonTo(connId: number, payload: ServerMessage): void {
    const state = this.connections.get(connId);
    if (!state || !state.joined) return;
    if (state.socket.readyState !== WebSocket.OPEN) return;
    if (state.socket.bufferedAmount > this.maxBufferedAmount) return;
    state.socket.send(JSON.stringify(payload));
  }

  /**
   * Register a new WebSocket connection.
   * @param socket - New connection socket.
   */
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

  /**
   * Handle an incoming WebSocket message.
   * @param state - Connection state sending the message.
   * @param data - Raw message payload.
   * @param isBinary - Whether the payload is binary.
   */
  private handleMessage(state: ConnectionState, data: RawData, isBinary: boolean): void {
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
      case 'view':
        if (!state.joined) {
          this.protocolError(state, 'join required before view');
          return;
        }
        this.handlers?.onView?.(state.id, msg);
        return;
      case 'viz':
        if (!state.joined) {
          this.protocolError(state, 'join required before viz');
          return;
        }
        this.handlers?.onViz?.(state.id, msg);
        return;
      case 'ping':
        return;
      default:
        this.protocolError(state, 'unknown message');
    }
  }

  /**
   * Notify a client of a protocol error and close the connection.
   * @param state - Connection state to close.
   * @param message - Error message to send.
   */
  private protocolError(state: ConnectionState, message: string): void {
    if (state.socket.readyState === WebSocket.OPEN) {
      state.socket.send(JSON.stringify({ type: 'error', message }));
    }
    state.socket.close(1008, message);
  }
}

/**
 * Compute the byte length of a raw WebSocket payload.
 * @param data - Raw data from ws.
 * @returns Byte size of the payload.
 */
function payloadSize(data: RawData): number {
  if (Buffer.isBuffer(data)) return data.byteLength;
  if (data instanceof ArrayBuffer) return data.byteLength;
  if (Array.isArray(data)) return data.reduce((sum, buf) => sum + buf.byteLength, 0);
  return 0;
}

/**
 * Convert a raw WebSocket payload to UTF-8 text.
 * @param data - Raw data from ws.
 * @returns UTF-8 string payload.
 */
function payloadToText(data: RawData): string {
  if (Buffer.isBuffer(data)) return data.toString('utf8');
  if (data instanceof ArrayBuffer) return Buffer.from(data).toString('utf8');
  if (Array.isArray(data)) {
    return Buffer.concat(data).toString('utf8');
  }
  return '';
}
