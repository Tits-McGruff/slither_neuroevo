import { describe, it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';

/**
 * Parses WS text payloads into JSON objects when possible.
 * @param data - Raw websocket payload.
 * @returns Parsed JSON object or null on failure.
 */
function parseJsonMessage(data: RawData): Record<string, unknown> | null {
  const text =
    typeof data === 'string'
      ? data
      : Buffer.isBuffer(data)
        ? data.toString('utf8')
        : data instanceof ArrayBuffer
          ? Buffer.from(data).toString('utf8')
          : String(data ?? '');
  try {
    const parsed = JSON.parse(text) as unknown;
    if (!parsed || typeof parsed !== 'object') return null;
    return parsed as Record<string, unknown>;
  } catch {
    return null;
  }
}

/**
 * Waits for a protocol error response or policy-violation close after sending data.
 * @param ws - WebSocket client to monitor.
 * @param timeoutMs - Timeout in milliseconds.
 * @param sendOnOpen - Callback that sends the test payload after open.
 * @returns True when a protocol rejection is observed.
 */
function waitForProtocolRejection(
  ws: WebSocket,
  timeoutMs: number,
  sendOnOpen: () => void
): Promise<boolean> {
  return new Promise<boolean>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timeout')), timeoutMs);

    const cleanup = () => {
      ws.off('open', onOpen);
      ws.off('message', onMessage);
      ws.off('close', onClose);
      ws.off('error', onError);
    };

    const finish = (result: boolean) => {
      clearTimeout(timeout);
      cleanup();
      resolve(result);
    };

    const onOpen = () => {
      try {
        sendOnOpen();
      } catch (err) {
        cleanup();
        clearTimeout(timeout);
        reject(err);
      }
    };

    const onMessage = (data: RawData, isBinary: boolean) => {
      if (isBinary) return;
      const msg = parseJsonMessage(data);
      if (!msg) return;
      if (msg['type'] === 'error') {
        finish(true);
      }
    };

    const onClose = (code: number) => {
      finish(code === 1008);
    };

    const onError = (err: Error) => {
      cleanup();
      clearTimeout(timeout);
      reject(err);
    };

    ws.on('open', onOpen);
    ws.on('message', onMessage);
    ws.on('close', onClose);
    ws.on('error', onError);
  });
}

/**
 * Starts the server and returns null when permissions prevent binding.
 * @returns Server handle or null when the port is unavailable.
 */
async function startServerWithGuard() {
  const isEperm = (err: unknown): boolean =>
    (err as { code?: string } | null)?.code === 'EPERM';
  const startPromise = startServer({
    ...DEFAULT_CONFIG,
    port: 0,
    dbPath: ':memory:',
    resume: 'fresh',
    inferenceBackend: 'js',
    logLevel: 'error'
  }).catch((err) => {
    if (isEperm(err)) return null;
    throw err;
  });

  let cleanup = () => { };
  const guard = new Promise<null>((resolve) => {
    const handler = (err: unknown) => {
      if (isEperm(err)) {
        resolve(null);
        return;
      }
      throw err;
    };
    process.once('uncaughtException', handler);
    cleanup = () => process.off('uncaughtException', handler);
  });

  let server: Awaited<ReturnType<typeof startServer>> | null = null;
  try {
    server = await Promise.race([startPromise, guard]);
  } finally {
    cleanup();
  }

  return server;
}

describe('security: invalid WS payloads', () => {
  it('rejects malformed JSON without crashing', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    let sawError = false;

    try {
      sawError = await waitForProtocolRejection(ws, 6000, () => {
        ws.send('{ this is not json');
      });
    } finally {
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CLOSING) {
        ws.close();
      }
      await server.close();
    }

    expect(sawError).toBe(true);
  }, 20000);

  it('rejects player join without name', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    let sawError = false;

    try {
      sawError = await waitForProtocolRejection(ws, 6000, () => {
        ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
        ws.send(JSON.stringify({ type: 'join', mode: 'player', name: '' }));
      });
    } finally {
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CLOSING) {
        ws.close();
      }
      await server.close();
    }

    expect(sawError).toBe(true);
  }, 20000);
});
