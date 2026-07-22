import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { normalizeConfig, parseConfig } from '../server/config.ts';
import { buildViteConfig, resolveUiDefaults } from '../vite.config.ts';

/** Phase 10 trusted-LAN configuration and launcher contract suite. */
const SUITE = 'Phase 10 trusted-LAN configuration';
/** Temporary roots created by this suite. */
const temporaryRoots: string[] = [];

/**
 * Create one isolated temporary directory tracked for cleanup.
 * @returns Absolute temporary directory path.
 */
function makeTemporaryRoot(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-phase10-lan-'));
  temporaryRoots.push(root);
  return root;
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  while (temporaryRoots.length > 0) {
    const root = temporaryRoots.pop();
    if (root) fs.rmSync(root, { recursive: true, force: true });
  }
});

describe(SUITE, () => {
  it('loads publicWsUrl from TOML and applies environment then CLI precedence', () => {
    const root = makeTemporaryRoot();
    const configPath = path.join(root, 'server.toml');
    fs.writeFileSync(
      configPath,
      [
        'host = "0.0.0.0"',
        'uiHost = "0.0.0.0"',
        'publicWsUrl = "ws://192.168.1.20:5174"',
        'port = 5174',
        'uiPort = 5173'
      ].join('\n'),
      'utf8'
    );

    expect(parseConfig(['--config', configPath], {})).toMatchObject({
      host: '0.0.0.0',
      uiHost: '0.0.0.0',
      publicWsUrl: 'ws://192.168.1.20:5174'
    });
    expect(parseConfig(['--config', configPath], {
      PUBLIC_WS_URL: 'ws://192.168.1.21:6174'
    }).publicWsUrl).toBe('ws://192.168.1.21:6174');
    expect(parseConfig([
      '--config',
      configPath,
      '--public-ws-url',
      'ws://192.168.1.22:7174'
    ], {
      PUBLIC_WS_URL: 'ws://192.168.1.21:6174'
    }).publicWsUrl).toBe('ws://192.168.1.22:7174');
  });

  it('writes the trusted-LAN routing field into a generated default config', () => {
    const root = makeTemporaryRoot();
    const configPath = path.join(root, 'generated.toml');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);

    expect(parseConfig(['--config', configPath], {}).publicWsUrl).toBe('');
    expect(fs.readFileSync(configPath, 'utf8')).toContain('publicWsUrl = ""');
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('created default'));
  });

  it('normalizes configured WebSocket URLs without turning the field into a security claim', () => {
    const warnings: string[] = [];
    expect(normalizeConfig({
      publicWsUrl: '  ws://192.168.1.30:5174  '
    }).publicWsUrl).toBe('ws://192.168.1.30:5174');
    expect(normalizeConfig({
      publicWsUrl: 42
    }, warning => warnings.push(warning)).publicWsUrl).toBe('');
    expect(warnings).toContain('publicWsUrl is invalid; leaving unset.');
  });

  it('keeps a connectable HMR host for explicit and all-interface LAN binds', () => {
    expect(resolveUiDefaults({
      uiHost: '0.0.0.0',
      uiPort: 55173,
      port: 55174
    }, '192.168.1.25')).toMatchObject({
      uiHost: '0.0.0.0',
      uiPort: 55173,
      serverPort: 55174,
      publicWsUrl: '',
      hmrHost: '192.168.1.25'
    });
    expect(resolveUiDefaults({
      uiHost: 'slither-pc',
      publicWsUrl: 'ws://sim-pc:6174'
    }, '192.168.1.25')).toMatchObject({
      uiHost: 'slither-pc',
      publicWsUrl: 'ws://sim-pc:6174',
      hmrHost: 'slither-pc'
    });
  });

  it('injects the configured split-host route through Vite in development and production', () => {
    const root = makeTemporaryRoot();
    const configPath = path.join(root, 'server.toml');
    fs.writeFileSync(
      configPath,
      [
        'uiHost = "0.0.0.0"',
        'uiPort = 55173',
        'port = 55174',
        'publicWsUrl = "ws://192.168.1.40:55174"'
      ].join('\n'),
      'utf8'
    );
    vi.stubEnv('SERVER_CONFIG', configPath);

    const config = buildViteConfig();
    expect(config.define).toMatchObject({
      'import.meta.env.SLITHER_DEFAULT_WS_URL': '"ws://192.168.1.40:55174"',
      'import.meta.env.SLITHER_SERVER_PORT': '55174'
    });
    expect(config.server).toMatchObject({
      host: '0.0.0.0',
      port: 55173,
      hmr: { host: '192.168.1.40' }
    });
  });

  it('retains LAN discovery, network URL output, and the mandatory native build in both launchers', () => {
    const powershell = fs.readFileSync(path.resolve('scripts/slither.ps1'), 'utf8');
    const posix = fs.readFileSync(path.resolve('play.sh'), 'utf8');

    for (const launcher of [powershell, posix]) {
      expect(launcher).toContain('publicWsUrl');
      expect(launcher).toContain('UI Network:');
      expect(launcher).toContain('WebSocket Network:');
      expect(launcher).toContain('@napi-rs');
      expect(launcher).toContain('build');
    }
    expect(powershell).toContain('Get-NonLoopbackIPv4');
    expect(posix).toContain('networkInterfaces');
  });
});
