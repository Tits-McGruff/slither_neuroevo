import { describe, expect, it, vi } from 'vitest';
import { WebSocket } from 'ws';
import type { AssignMsg, ClientMessage, ServerMessage, StateReplacedMsg, WelcomeMsg } from './protocol.ts';
import { WsHub, type ConnectionState } from './wsHub.ts';

/** Fake `ws` socket with manually completed writes. */
interface FakeSocket {
  /** WebSocket ready state. */
  readyState: number;
  /** Bytes already buffered by the transport. */
  bufferedAmount: number;
  /** Payloads handed to `ws` in order. */
  sent: Array<{ payload: unknown; binary: boolean; complete: (error?: Error) => void }>;
  /** Close calls made by outbound failure handling. */
  closes: Array<{ code?: number; reason?: string }>;
  /** Capture one send without completing it. */
  send: (
    payload: unknown,
    options: { binary: boolean },
    callback: (error?: Error) => void
  ) => void;
  /** Capture one close. */
  close: (code?: number, reason?: string) => void;
}

/**
 * Build a hub instance around one fake joined UI connection without opening a port.
 * @returns Hub, connection state, and fake socket.
 */
function buildFakeHub(): { hub: WsHub; state: ConnectionState; socket: FakeSocket } {
  const socket: FakeSocket = {
    readyState: WebSocket.OPEN,
    bufferedAmount: 0,
    sent: [],
    closes: [],
    send(payload, options, callback) {
      this.sent.push({ payload, binary: options.binary, complete: callback });
    },
    close(code, reason) {
      this.closes.push({
        ...(code === undefined ? {} : { code }),
        ...(reason === undefined ? {} : { reason })
      });
      this.readyState = WebSocket.CLOSING;
    }
  };
  const state: ConnectionState = {
    id: 1,
    socket: socket as unknown as WebSocket,
    clientType: 'ui',
    joined: true,
    awaitingRejoin: false,
    mode: 'player',
    reliableQueue: [],
    reliableQueueBytes: 0,
    pendingStats: null,
    pendingFrame: null,
    pendingFrameRelease: null,
    sending: false,
    replacedFrames: 0,
    reliableFailures: 0
  };
  const hub = Object.create(WsHub.prototype) as WsHub;
  const access = hub as unknown as {
    connections: Map<number, ConnectionState>;
    maxBufferedAmount: number;
    replacedFrames: number;
    reliableFailures: number;
    highWaterReliableMessagesPerConnection: number;
    highWaterReliableBytesPerConnection: number;
    maxConnections: number;
  };
  access.connections = new Map([[state.id, state]]);
  access.maxBufferedAmount = 512 * 1024;
  access.replacedFrames = 0;
  access.reliableFailures = 0;
  access.highWaterReliableMessagesPerConnection = 0;
  access.highWaterReliableBytesPerConnection = 0;
  access.maxConnections = 4;
  return { hub, state, socket };
}

describe('WsHub lifecycle priority', () => {
  it.each(['reset', 'newRun', 'import'] as const)('discards old controls until rejoin while %s is queued behind a frame', reason => {
    const { hub, state, socket } = buildFakeHub();
    const handlers = { onJoin: vi.fn(), onAction: vi.fn(), onView: vi.fn(), onViz: vi.fn(),
      onReset: vi.fn(), onSettings: vi.fn(), onGodMode: vi.fn(), onNewRun: vi.fn() };
    hub.setHandlers(handlers);
    const routing = hub as unknown as {
      maxMessageBytes: number;
      handleMessage(state: ConnectionState, data: Buffer, binary: boolean): void;
      protocolError(state: ConnectionState, message: string): void;
    };
    routing.maxMessageBytes = 64 * 1024;
    const rejected = vi.spyOn(routing, 'protocolError');
    const send = (message: ClientMessage): void => routing.handleMessage(state, Buffer.from(JSON.stringify(message)), false);
    const action = { type: 'action', tick: 7, snakeId: 1, turn: 0.5, boost: 0 } as const;
    hub.broadcastFrame(Uint8Array.of(1));
    const replacement: StateReplacedMsg = { type: 'stateReplaced', reason,
      checkpointId: 'a'.repeat(64), welcome: { runId: 'replacement' } as WelcomeMsg };
    hub.enterAwaitingRejoin(replacement);
    hub.sendJsonToAwaitingConnection(1, { type: 'newRunResult', requestId: 'new-run',
      applied: true, runId: 'replacement', worldSeed: 1 });
    const stale: ClientMessage[] = [action, { type: 'view', viewW: 800, viewH: 600 },
      { type: 'viz', enabled: true }, { type: 'reset' },
      { type: 'settings', requestId: 'old', updates: [{ path: 'simSpeed', value: 2 }] },
      { type: 'godMode', requestId: 'old', action: 'kill', snakeId: 1 },
      { type: 'newRun', requestId: 'old' }];
    for (const message of stale) send(message);
    expect(rejected).not.toHaveBeenCalled();
    for (const handler of Object.values(handlers)) expect(handler).not.toHaveBeenCalled();
    expect(state.awaitingRejoin).toBe(true);
    expect(state.joined).toBe(false);
    expect(socket.closes).toEqual([]);
    expect(socket.sent).toHaveLength(1);
    socket.sent[0]!.complete();
    expect(JSON.parse(String(socket.sent[1]!.payload))).toEqual(replacement);
    socket.sent[1]!.complete();
    expect(JSON.parse(String(socket.sent[2]!.payload))).toMatchObject({ type: 'newRunResult', applied: true });
    socket.sent[2]!.complete();
    send({ type: 'join', mode: 'player' });
    send(action);
    send({ type: 'view', viewW: 800 });
    expect(state.awaitingRejoin).toBe(false);
    expect(handlers.onJoin).toHaveBeenCalledOnce();
    expect(handlers.onAction).toHaveBeenCalledWith(1, action);
    expect(handlers.onView).toHaveBeenCalledOnce();
  });

  it('still rejects control traffic from a client that never joined', () => {
    const { hub, state } = buildFakeHub();
    state.joined = false;
    const routing = hub as unknown as {
      maxMessageBytes: number;
      handleMessage(state: ConnectionState, data: Buffer, binary: boolean): void;
      protocolError(state: ConnectionState, message: string): void;
    };
    routing.maxMessageBytes = 64 * 1024;
    const rejected = vi.spyOn(routing, 'protocolError').mockImplementation(() => undefined);
    routing.handleMessage(state, Buffer.from(JSON.stringify({ type: 'action', tick: 0, snakeId: 1, turn: 0, boost: 0 })), false);
    expect(rejected).toHaveBeenCalledWith(state, 'join required before action');
  });
  it('retains shared frame bytes until every send completes and releases replaced frames once', () => {
    const { hub, socket } = buildFakeHub();
    const slow = buildFakeHub();
    slow.state.id = 2;
    (hub as unknown as { connections: Map<number, ConnectionState> }).connections.set(2, slow.state);
    const released: number[] = [];
    hub.broadcastFrame(Uint8Array.of(1), () => { released.push(1); });
    hub.broadcastFrame(Uint8Array.of(2), () => { released.push(2); });
    hub.broadcastFrame(Uint8Array.of(3), () => { released.push(3); });
    expect(released).toEqual([2]);
    socket.sent[0]!.complete();
    expect(released).toEqual([2]);
    expect(socket.sent[1]!.payload).toEqual(Uint8Array.of(3));
    socket.sent[1]!.complete();
    expect(released).toEqual([2]);
    slow.socket.sent[0]!.complete();
    expect(released).toEqual([2, 1]);
    slow.socket.sent[1]!.complete();
    expect(released).toEqual([2, 1, 3]);
  });

  it('releases unused and cancelled leases while retaining an in-flight shutdown send', () => {
    const { hub, state, socket } = buildFakeHub();
    const released: number[] = [];
    state.joined = false;
    hub.broadcastFrame(Uint8Array.of(0), () => { released.push(0); });
    expect(released).toEqual([0]);
    state.joined = true;
    hub.broadcastFrame(Uint8Array.of(1), () => { released.push(1); });
    hub.broadcastFrame(Uint8Array.of(2), () => { released.push(2); });
    (hub as unknown as { wss: { close(): void } }).wss = { close() {} };
    hub.closeAll();
    expect(released).toEqual([0, 2]);
    socket.sent[0]!.complete();
    expect(released).toEqual([0, 2, 1]);
  });

  it('releases a borrowed frame when the transport throws before accepting it', () => {
    const { hub, socket } = buildFakeHub();
    const release = vi.fn();
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    try {
      socket.send = () => { throw new Error('closed transport'); };
      hub.broadcastFrame(Uint8Array.of(1), release);
      expect(release).toHaveBeenCalledTimes(1);
      expect(socket.closes).toHaveLength(1);
    } finally { errorSpy.mockRestore(); }
  });

  it('drains assignment, reclaim, control, and error traffic before the newest frame', () => {
    const { hub, socket } = buildFakeHub();
    const frame1 = Uint8Array.of(1);
    const frame2 = Uint8Array.of(2);
    const frame3 = Uint8Array.of(3);
    hub.broadcastFrame(frame1);
    hub.broadcastFrame(frame2);
    hub.broadcastFrame(frame3);
    const assign: AssignMsg = {
      type: 'assign',
      snakeId: 7,
      controller: 'player',
      resumeToken: 'priority-token'
    };
    expect(hub.sendJsonTo(1, assign)).toBe(true);
    const reliable: ServerMessage[] = [
      {
        type: 'reclaimResult',
        reclaimed: true,
        reason: 'reclaimed',
        snakeId: 7
      },
      {
        type: 'sensors',
        tick: 4,
        snakeId: 7,
        sensors: [0.1, 0.2],
        meta: { x: 1, y: 2, dir: 0.5 }
      },
      {
        type: 'error',
        message: 'visible lifecycle failure'
      }
    ];
    for (const message of reliable) expect(hub.sendJsonTo(1, message)).toBe(true);

    expect(socket.sent).toHaveLength(1);
    expect(socket.sent[0]?.payload).toBe(frame1);
    socket.sent[0]!.complete();
    const expectedTypes = ['assign', 'reclaimResult', 'sensors', 'error'];
    for (let index = 0; index < expectedTypes.length; index++) {
      const sentIndex = index + 1;
      expect(JSON.parse(String(socket.sent[sentIndex]?.payload))).toMatchObject({
        type: expectedTypes[index]
      });
      socket.sent[sentIndex]!.complete();
    }
    expect(socket.sent[expectedTypes.length + 1]?.payload).toBe(frame3);
    expect(hub.getOutboundDiagnostics().replacedFrames).toBe(1);
  });

  it('makes a failed first reliable write observable and closes the affected socket', () => {
    const { hub, socket } = buildFakeHub();
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    try {
      const assign: AssignMsg = {
        type: 'assign',
        snakeId: 8,
        controller: 'bot',
        resumeToken: 'failure-token'
      };
      hub.sendJsonTo(1, assign);
      socket.sent[0]!.complete(new Error('simulated write failure'));

      expect(hub.getOutboundDiagnostics().reliableFailures).toBe(1);
      expect(socket.closes).toEqual([
        { code: 1011, reason: 'outbound send failed' }
      ]);
      expect(errorSpy).toHaveBeenCalledWith(
        '[ws.reliable_send_failed]',
        expect.objectContaining({ connId: 1, reason: 'simulated write failure' })
      );
    } finally {
      errorSpy.mockRestore();
    }
  });

  it('retains replacements and late reliable failures after the peer has been removed', () => {
    const { hub, socket, state } = buildFakeHub();
    const connections = (hub as unknown as { connections: Map<number, ConnectionState> }).connections;
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    try {
      hub.broadcastFrame(Uint8Array.of(1));
      hub.broadcastFrame(Uint8Array.of(2));
      hub.broadcastFrame(Uint8Array.of(3));
      hub.sendJsonTo(1, { type: 'error', message: 'reliable write behind frame' });
      socket.sent[0]!.complete();
      expect(JSON.parse(String(socket.sent[1]!.payload))).toMatchObject({ type: 'error' });
      connections.delete(state.id);
      expect(hub.getOutboundDiagnostics()).toMatchObject({ connections: 0, replacedFrames: 1, reliableFailures: 0 });
      socket.sent[1]!.complete(new Error('failure after disconnect'));
      expect(hub.getOutboundDiagnostics()).toEqual({ connections: 0, reliableQueuedMessages: 0,
        reliableQueuedBytes: 0, pendingFrames: 0, replacedFrames: 1, reliableFailures: 1,
        highWaterReliableMessagesPerConnection: 1,
        highWaterReliableBytesPerConnection: Buffer.byteLength(JSON.stringify({ type: 'error', message: 'reliable write behind frame' })),
        maxReliableMessagesPerConnection: 1024, maxReliableBytesPerConnection: 4 * 1024 * 1024, maxConnections: 4 });
      const replacement = buildFakeHub();
      replacement.state.id = 2;
      connections.set(2, replacement.state);
      expect(hub.getOutboundDiagnostics()).toMatchObject({ connections: 1, replacedFrames: 1, reliableFailures: 1 });
    } finally { errorSpy.mockRestore(); }
  });
});
