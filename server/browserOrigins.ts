import type { IncomingMessage, ServerResponse } from 'node:http';
import { hostname, networkInterfaces } from 'node:os';
import type { ServerConfig } from './config.ts';

/** Shared admission policy for HTTP requests and WebSocket upgrades. */
export interface BrowserOriginPolicy {
  /** Accept configured browser origins or clients that omit Origin. */
  allows(origin: string | undefined, serverPort?: number): boolean;
}

/** Canonical loopback names supported by the launchers and browser client. */
const LOOPBACK_HOSTS = ['localhost', '127.0.0.1', '[::1]'];

/** Normalize one configured hostname/IP without accepting URL credentials or paths. */
function normalizeHost(host: string): string {
  const authority = host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
  const url = new URL(`http://${authority}`);
  if (url.username || url.password || url.port || url.pathname !== '/' || url.search || url.hash) {
    throw new Error('UI/server hosts must be hostnames or IP addresses without URL credentials, ports or paths');
  }
  return url.hostname;
}

/** Build exact host allowlists; wildcard binds expand to this machine's addresses only. */
function boundHosts(host: string): Set<string> {
  const hosts = new Set(LOOPBACK_HOSTS);
  if (host === '0.0.0.0' || host === '::') {
    const name = hostname();
    hosts.add(normalizeHost(name));
    hosts.add(normalizeHost(name.split('.')[0]!));
    for (const addresses of Object.values(networkInterfaces())) {
      for (const address of addresses ?? []) hosts.add(normalizeHost(address.address.split('%')[0]!));
    }
  } else {
    hosts.add(normalizeHost(host));
  }
  return hosts;
}

/**
 * Allow the built UI and configured Vite UI, including advertised LAN addresses.
 * Request Host headers and arbitrary private-network addresses never expand trust.
 * @param config - Resolved UI/server routing configuration.
 * @returns Startup policy shared by both browser transports.
 */
export function createBrowserOriginPolicy(
  config: Pick<ServerConfig, 'host' | 'port' | 'uiHost' | 'uiPort' | 'publicWsUrl'>
): BrowserOriginPolicy {
  const serverHosts = boundHosts(config.host);
  const uiHosts = boundHosts(config.uiHost);
  if (config.publicWsUrl) serverHosts.add(new URL(config.publicWsUrl).hostname);
  return {
    allows(origin, serverPort = config.port) {
      if (origin === undefined) return true;
      try {
        const url = new URL(origin);
        if (url.protocol !== 'http:' || url.origin !== origin) return false;
        const port = Number(url.port || 80);
        return (uiHosts.has(url.hostname) && port === config.uiPort) ||
          (serverHosts.has(url.hostname) && port === serverPort);
      } catch {
        return false;
      }
    }
  };
}

/**
 * Reject untrusted browser requests before routing, including simple POSTs.
 * @returns True when routing may proceed, false after a complete rejection.
 */
export function admitBrowserRequest(
  request: IncomingMessage,
  response: ServerResponse,
  policy: BrowserOriginPolicy
): boolean {
  response.setHeader('Vary', 'Origin, Sec-Fetch-Site, Referer');
  let allowed = policy.allows(request.headers.origin, request.socket.localPort);
  const site = request.headers['sec-fetch-site'];
  if (request.headers.origin === undefined && (site === 'cross-site' || site === 'same-site')) {
    // Direct downloads may omit Origin even for a configured split-host UI.
    // Browser-generated Referer identifies that UI; CLI clients omit Fetch Metadata.
    try {
      const referer = new URL(request.headers.referer ?? '');
      allowed = allowed && policy.allows(referer.origin, request.socket.localPort);
    } catch { allowed = false; }
  }
  if (!allowed) {
    response.writeHead(403, { 'Content-Type': 'application/json', 'Connection': 'close' });
    request.resume();
    response.end(JSON.stringify({ ok: false, message: 'browser origin is not allowed' }));
    return false;
  }
  if (request.headers.origin !== undefined) {
    response.setHeader('Access-Control-Allow-Origin', request.headers.origin);
    response.setHeader('Access-Control-Allow-Credentials', 'true');
    response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS, PUT, DELETE');
    response.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization, X-Requested-With, Accept');
  }
  if (request.method === 'OPTIONS') {
    response.writeHead(204);
    response.end();
    return false;
  }
  return true;
}
