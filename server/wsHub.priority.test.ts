import { describe, expect, it, vi } from 'vitest';
import { WebSocket } from 'ws';
import type { AssignMsg, ServerMessage } from './protocol.ts';
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
    mode: 'player',
    lastMessageTime: 0,
    reliableQueue: [],
    reliableQueueBytes: 0,
    pendingStats: null,
    pendingFrame: null,
    sending: false,
    replacedFrames: 0,
    reliableFailures: 0
  };
  const hub = Object.create(WsHub.prototype) as WsHub;
  const access = hub as unknown as {
    connections: Map<number, ConnectionState>;
    maxBufferedAmount: number;
  };
  access.connections = new Map([[state.id, state]]);
  access.maxBufferedAmount = 512 * 1024;
  return { hub, state, socket };
}

describe('WsHub lifecycle priority', () => {
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
});
