import { createReadStream } from 'node:fs';
import { readFile, stat } from 'node:fs/promises';
import type { ServerResponse } from 'node:http';
import { extname, resolve, sep } from 'node:path';
import { RUNTIME_SERVER_URL_META } from '../src/protocol/browserRouting.ts';

/** Browser asset MIME types emitted by Vite. */
const CONTENT_TYPES: Readonly<Record<string, string>> = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' };

/** Escape runtime configuration as inert HTML attribute text. */
function attributeText(value: string): string {
  return value.replace(/&/gu, '&amp;').replace(/</gu, '&lt;').replace(/>/gu, '&gt;')
    .replace(/"/gu, '&quot;').replace(/'/gu, '&#39;');
}

/**
 * Serve built assets with current process routing inserted before browser modules execute.
 * @param pathname - Decoded by this function within the permitted asset root.
 * @param response - Actual HTTP response receiving the asset.
 * @param clientRoot - Absolute directory containing the built browser.
 * @param publicWsUrl - Runtime configured route, or empty for the serving page's origin.
 */
export async function serveBrowserAsset(
  pathname: string,
  response: ServerResponse,
  clientRoot: string,
  publicWsUrl: string
): Promise<void> {
  try {
    const path = resolve(clientRoot, `.${decodeURIComponent(pathname === '/' ? '/index.html' : pathname)}`);
    if (!path.startsWith(`${clientRoot}${sep}`) || !(await stat(path)).isFile()) {
      response.writeHead(404); response.end(); return;
    }
    if (path === resolve(clientRoot, 'index.html')) {
      const html = await readFile(path, 'utf8');
      const route = `<meta name="${RUNTIME_SERVER_URL_META}" content="${attributeText(publicWsUrl)}" />`;
      response.writeHead(200, { 'Content-Type': CONTENT_TYPES['.html']!, 'Cache-Control': 'no-store' });
      response.end(html.replace(/<head(?:\s[^>]*)?>/iu, head => `${head}\n  ${route}`));
      return;
    }
    response.writeHead(200, { 'Content-Type': CONTENT_TYPES[extname(path)] ?? 'application/octet-stream' });
    const stream = createReadStream(path);
    stream.on('error', () => response.destroy());
    response.on('close', () => stream.destroy());
    stream.pipe(response);
  } catch {
    if (!response.headersSent) response.writeHead(404);
    response.end();
  }
}