import { describe, expect, it, vi } from 'vitest';
import { DEFAULT_CONFIG } from './config.ts';
import { createBrowserOriginPolicy } from './browserOrigins.ts';

vi.mock('node:os', () => ({
  hostname: () => 'slither-pc.home',
  networkInterfaces: () => ({ lan: [{ address: '192.168.1.25' }, { address: 'fe80::1%eth0' }] })
}));

describe('configured browser origins', () => {
  it('accepts loopback UI aliases, built UI and non-browser clients', () => {
    const policy = createBrowserOriginPolicy(DEFAULT_CONFIG);
    for (const origin of [undefined, 'http://localhost:5173', 'http://127.0.0.1:5173',
      'http://[::1]:5173', 'http://127.0.0.1:5174']) expect(policy.allows(origin)).toBe(true);
    expect(policy.allows('http://localhost:61234', 61234)).toBe(true);
    expect(policy.allows('http://localhost:5174', 61234)).toBe(false);
  });

  it.each(['null', '', 'garbage', 'https://localhost:5173', 'http://localhost:5173/',
    'http://user@localhost:5173', 'http://localhost:9000', 'http://localhost.evil.test:5173',
    'http://evil.test:5173', 'http://192.168.1.25:5173', 'http://172.99.1.2:5173'])(
    'rejects unconfigured or malformed origins: %s', origin => {
      expect(createBrowserOriginPolicy(DEFAULT_CONFIG).allows(origin)).toBe(false);
    });

  it('allows only this machine at the configured ports for wildcard LAN binds', () => {
    const policy = createBrowserOriginPolicy({ ...DEFAULT_CONFIG, host: '0.0.0.0', uiHost: '::' });
    for (const host of ['192.168.1.25', 'slither-pc.home', 'slither-pc', '[fe80::1]']) {
      expect(policy.allows(`http://${host}:5173`)).toBe(true);
      expect(policy.allows(`http://${host}:5174`)).toBe(true);
      expect(policy.allows(`http://${host}:8000`)).toBe(false);
    }
    expect(policy.allows('http://192.168.1.26:5173')).toBe(false);
    expect(policy.allows('http://unrelated-pc:5173')).toBe(false);
  });

  it('keeps explicit split UI/server hosts and advertised server aliases separate', () => {
    const policy = createBrowserOriginPolicy({ ...DEFAULT_CONFIG, host: '192.168.1.25',
      uiHost: 'ui-pc', uiPort: 55173, port: 55174, publicWsUrl: 'ws://sim-pc:55174' });
    expect(policy.allows('http://ui-pc:55173')).toBe(true);
    expect(policy.allows('http://192.168.1.25:55174')).toBe(true);
    expect(policy.allows('http://sim-pc:55174')).toBe(true);
    expect(policy.allows('http://sim-pc:55173')).toBe(false);
    expect(policy.allows('http://ui-pc:55174')).toBe(false);
  });
});
