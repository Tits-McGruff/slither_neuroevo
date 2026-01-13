import { describe, it, expect } from 'vitest';
import WebSocket, { type RawData } from 'ws';
import { startServer } from './index.ts';
import { DEFAULT_CONFIG } from './config.ts';

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
    logLevel: 'error'
  }).catch((err) => {
    if (isEperm(err)) return null;
    throw err;
  });

  let cleanup = () => {};
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

describe('system: server lifecycle', () => {
  it('boots, streams, and shuts down cleanly', async () => {
    const server = await startServerWithGuard();
    if (!server) return;

    const ws = new WebSocket(server.wsUrl);
    ws.binaryType = 'arraybuffer';
    let sawFrame = false;

    try {
      const result = new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('timeout')), 4000);

        ws.on('error', (err: Error) => {
          clearTimeout(timeout);
          reject(err);
        });

        ws.on('message', (_data: RawData, isBinary: boolean) => {
          if (isBinary) sawFrame = true;
          if (sawFrame) {
            clearTimeout(timeout);
            resolve();
          }
        });

        ws.on('open', () => {
          ws.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 1 }));
          ws.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
        });
      });

      await result;
    } finally {
      ws.close();
      await server.close();
    }

    expect(sawFrame).toBe(true);
  }, 20000);
});
