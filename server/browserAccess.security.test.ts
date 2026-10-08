import { once } from 'node:events';
import { createServer, type Server } from 'node:http';
import { expect, it } from 'vitest';
import { WebSocket } from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { admitBrowserRequest, createBrowserOriginPolicy } from './browserOrigins.ts';
import type { WelcomeMsg } from './protocol.ts';
import { WsHub } from './wsHub.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';

/** Lightweight real HTTP/WS transport fixture; no simulation or database is needed. */
async function transportFixture(handshakeTimeoutMs = 5000): Promise<{
  /** Bound loopback HTTP URL. */
  url: string;
  /** Bound WebSocket URL. */
  wsUrl: string;
  /** Hub used to assert rejected peers do not occupy capacity. */
  hub: WsHub;
  /** Routed HTTP mutation count. */
  mutations: () => number;
  /** Join every test-owned transport. */
  close: () => Promise<void>;
}> {
  const policy = createBrowserOriginPolicy({ ...DEFAULT_CONFIG, port: 0 });
  let mutations = 0;
  const server: Server = createServer((request, response) => {
    if (!admitBrowserRequest(request, response, policy)) return;
    if (request.method === 'POST') mutations++;
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ ok: true }));
  });
  // These tests exercise transport admission, not welcome metadata construction.
  const hub = new WsHub(server, { type: 'welcome' } as WelcomeMsg,
    { browserOrigins: policy, maxConnections: 2, handshakeTimeoutMs });
  await new Promise<void>((done, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => { server.off('error', reject); done(); });
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('transport fixture has no TCP address');
  const url = `http://127.0.0.1:${address.port}`;
  return { url, wsUrl: url.replace('http:', 'ws:'), hub, mutations: () => mutations,
    async close() {
      hub.closeAll();
      await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
    } };
}

/** Open a real peer, capturing welcome before sending hello. */
async function welcomedPeer(wsUrl: string, origin?: string): Promise<WebSocket> {
  const peer = new WebSocket(wsUrl, origin === undefined ? {} : { origin });
  try {
    await once(peer, 'open');
    const welcome = once(peer, 'message');
    peer.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
    expect(JSON.parse(String((await welcome)[0]))).toEqual({ type: 'welcome' });
    return peer;
  } catch (error) {
    peer.terminate();
    throw error;
  }
}

describeNetworkSuite('browser origin and handshake admission', () => {
  it('rejects untrusted HTTP reads, simple mutations and preflights before routing', async () => {
    const fixture = await transportFixture();
    try {
      for (const origin of ['http://evil.test', 'null', 'http://127.0.0.1:9000']) {
        for (const method of ['GET', 'POST', 'OPTIONS']) {
          const response = await fetch(`${fixture.url}/api/import/archive`, {
            method, headers: { Origin: origin, 'Content-Type': 'text/plain',
              ...(method === 'OPTIONS' ? { 'Access-Control-Request-Method': 'POST' } : {}) },
            ...(method === 'POST' ? { body: 'untrusted simple POST' } : {})
          });
          expect(response.status).toBe(403);
          expect(response.headers.has('Access-Control-Allow-Origin')).toBe(false);
          expect(await response.json()).toMatchObject({ ok: false });
        }
      }
      expect(fixture.mutations()).toBe(0);
      for (const origin of ['http://localhost:5173', fixture.url]) {
        const preflight = await fetch(fixture.url, { method: 'OPTIONS', headers: { Origin: origin } });
        expect(preflight.status).toBe(204);
        expect(preflight.headers.get('Access-Control-Allow-Origin')).toBe(origin);
        expect(preflight.headers.get('Vary')).toBe('Origin, Sec-Fetch-Site, Referer');
        const mutation = await fetch(fixture.url, { method: 'POST', headers: { Origin: origin } });
        expect(mutation.status).toBe(200);
        await mutation.text();
      }
      const direct = await fetch(fixture.url);
      expect(direct.status).toBe(200);
      expect(direct.headers.has('Access-Control-Allow-Origin')).toBe(false);
      await direct.text();
      expect(fixture.mutations()).toBe(2);
    } finally { await fixture.close(); }
  });

  it('blocks originless cross-origin subresources while preserving CLI and trusted UI downloads', async () => {
    const fixture = await transportFixture();
    try {
      for (const site of ['cross-site', 'same-site']) {
        for (const referer of [undefined, 'http://evil.test/page', 'http://localhost:9000/page']) {
          const response = await fetch(`${fixture.url}/api/export/latest`, {
            headers: { 'Sec-Fetch-Site': site, 'Sec-Fetch-Mode': 'no-cors', 'Sec-Fetch-Dest': 'image',
              ...(referer === undefined ? {} : { Referer: referer }) }
          });
          expect(response.status).toBe(403);
          expect(response.headers.has('Access-Control-Allow-Origin')).toBe(false);
          await response.text();
        }
      }
      for (const headers of [ {}, { 'Sec-Fetch-Site': 'same-origin' }, { 'Sec-Fetch-Site': 'none' },
        { 'Sec-Fetch-Site': 'cross-site', Referer: 'http://localhost:5173/' },
        { 'Sec-Fetch-Site': 'same-site', Referer: 'http://localhost:5173/' } ]) {
        const response = await fetch(`${fixture.url}/api/export/latest`, { headers });
        expect(response.status).toBe(200);
        await response.text();
      }
    } finally { await fixture.close(); }
  });

  it('rejects untrusted WebSocket upgrades before admitting a connection', async () => {
    const fixture = await transportFixture();
    try {
      for (const origin of ['http://evil.test', 'null', 'http://localhost:9000']) {
        const peer = new WebSocket(fixture.wsUrl, { origin });
        const rejection = once(peer, 'error');
        const closed = new Promise<void>(done => peer.once('close', () => done()));
        expect(String((await rejection)[0])).toContain('403');
        await closed;
        expect(fixture.hub.getClientCount()).toBe(0);
      }
      for (const origin of ['http://localhost:5173', fixture.url, undefined]) {
        const peer = await welcomedPeer(fixture.wsUrl, origin);
        const closed = once(peer, 'close');
        peer.close();
        await closed;
      }
    } finally { await fixture.close(); }
  });

  it('reclaims a full cap of peers without hello even when they send protocol pings', async () => {
    const fixture = await transportFixture(1000);
    const peers: WebSocket[] = [];
    try {
      for (let index = 0; index < 2; index++) {
        const peer = new WebSocket(fixture.wsUrl);
        peers.push(peer);
        await once(peer, 'open');
      }
      expect(fixture.hub.getClientCount()).toBe(2);
      const closed = peers.map(peer => once(peer, 'close'));
      const ping = setInterval(() => {
        const peer = peers[0]!;
        if (peer.readyState === WebSocket.OPEN) peer.send(JSON.stringify({ type: 'ping' }));
      }, 50);
      try { await Promise.all(closed); } finally { clearInterval(ping); }
      expect(fixture.hub.getClientCount()).toBe(0);
      const viewer = await welcomedPeer(fixture.wsUrl, 'http://localhost:5173');
      peers.push(viewer);
      viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      await new Promise<void>(done => setTimeout(done, 1100));
      expect(viewer.readyState).toBe(WebSocket.OPEN);
      expect(fixture.hub.getClientCount()).toBe(1);
    } finally {
      for (const peer of peers) peer.terminate();
      await fixture.close();
    }
  });
});
