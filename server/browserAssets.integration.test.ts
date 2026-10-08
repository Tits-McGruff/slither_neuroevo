import { createServer } from 'node:http';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket, WebSocketServer } from 'ws';
import { afterEach, expect, it, vi } from 'vitest';
import { serveBrowserAsset } from './browserAssets.ts';
import { resolveServerUrl } from '../src/net/wsClient.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

afterEach(() => { vi.unstubAllGlobals(); });

describeNetworkSuite('production browser runtime routing', () => {
  it.each(['', 'ws://split-host.test:6174/game?key=a&mode=b'])('serves current routing without rebuilding the client: %s', async route => {
    const root = await mkdtemp(join(tmpdir(), 'slither-runtime-route-'));
    const server = createServer((request, response) => {
      void serveBrowserAsset(new URL(request.url!, 'http://localhost').pathname, response, root, route);
    });
    const sockets = new WebSocketServer({ server });
    let peer: WebSocket | undefined;
    try {
      // The fixture represents a bundle built before service environment overrides.
      await writeFile(join(root, 'index.html'), '<!doctype html><html><head><script type="module">const bakedPort = 5174;</script></head><body>game</body></html>');
      await writeFile(join(root, 'app.js'), 'export const unchanged = true;');
      await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
      const port = (server.address() as { port: number }).port;
      const base = `http://127.0.0.1:${port}`;
      for (const path of ['/', '/index.html']) {
        const response = await fetch(base + path);
        expect(response.status).toBe(200);
        expect(response.headers.get('cache-control')).toBe('no-store');
        const html = await response.text();
        expect(html).toContain('const bakedPort = 5174;');
        const tag = html.match(/<meta name="slither-server-url" content="([^"]*)" \/>/u);
        expect(tag).not.toBeNull();
        expect(html.indexOf(tag![0])).toBeLessThan(html.indexOf('<script'));
        const content = tag![1]!.replace(/&amp;/gu, '&');
        vi.stubGlobal('document', { querySelector: (selector: string) =>
          selector === 'meta[name="slither-server-url"]' ? { content } : null });
        vi.stubGlobal('window', { location: new URL(base + path) });
        vi.stubGlobal('localStorage', { getItem: () => 'ws://cached.test:5174' });
        expect(resolveServerUrl('ws://baked.test:5174')).toBe(route || `ws://127.0.0.1:${port}`);
      }
      if (!route) {
        // The resolved browser URL reaches the running service on its actual bound port.
        const accepted = new Promise<void>(done => sockets.once('connection', () => done()));
        peer = new WebSocket(resolveServerUrl());
        await new Promise<void>((done, reject) => { peer!.once('open', done); peer!.once('error', reject); });
        await accepted;
      }
      const asset = await fetch(base + '/app.js');
      expect(asset.headers.get('content-type')).toBe('text/javascript; charset=utf-8');
      expect(await asset.text()).toBe('export const unchanged = true;');
      expect((await fetch(base + '/missing')).status).toBe(404);
      expect((await fetch(base + '/%2e%2e%2foutside')).status).toBe(404);
    } finally {
      peer?.terminate();
      for (const client of sockets.clients) client.terminate();
      await new Promise<void>(done => sockets.close(() => done()));
      await new Promise<void>(done => server.close(() => done()));
      await rm(root, { recursive: true, force: true });
    }
  }, 5000);

  it('escapes configured routing as metadata without inserting markup', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-runtime-route-escape-'));
    const route = 'ws://trusted.test:6174/?x="><script>alert(1)</script>&y=\'value\'';
    const server = createServer((request, response) => {
      void serveBrowserAsset(request.url!, response, root, route);
    });
    try {
      await writeFile(join(root, 'index.html'), '<html><head></head><body>game</body></html>');
      await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
      const port = (server.address() as { port: number }).port;
      const html = await (await fetch(`http://127.0.0.1:${port}/`)).text();
      expect(html).toContain('&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;&amp;y=&#39;value&#39;');
      expect(html).not.toContain('<script>');
    } finally {
      await new Promise<void>(done => server.close(() => done()));
      await rm(root, { recursive: true, force: true });
    }
  });
});