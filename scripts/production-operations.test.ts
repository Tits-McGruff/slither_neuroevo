import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { describe, expect, it } from 'vitest';

/** Production foreground launcher source. */
const RUNNER = readFileSync(resolve('scripts/run-production.sh'), 'utf8');

/** User-service template source. */
const SERVICE = readFileSync(resolve('scripts/slither-neuroevo.service.in'), 'utf8');

/** User-service installer source. */
const INSTALLER = readFileSync(resolve('scripts/install-systemd-user.sh'), 'utf8').replaceAll('\r\n', '\n');

/** Production launcher source; only its probe functions are executed below. */
const LAUNCHER = readFileSync(resolve('play.sh'), 'utf8').replaceAll('\r\n', '\n');

/** POSIX shell provided by Debian or Git for Windows, without invoking WSL. */
const POSIX_SHELL = process.platform === 'win32'
  ? resolve(process.env['ProgramFiles'] ?? 'C:/Program Files', 'Git/bin/bash.exe')
  : '/bin/sh';

describe('production service operations', () => {
  it('uses TOML deployment settings by default and keeps SLITHER_* as explicit overrides', () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-launcher-config-'));
    const config = join(root, 'server.toml');
    const database = join(root, 'owner database.db');
    try {
      writeFileSync(config, [
        'host = "0.0.0.0"',
        'port = 6123',
        `dbPath = ${JSON.stringify(database.replaceAll('\\', '/'))}`,
        'resume = "auto"'
      ].join('\n')); 
      const resolver = resolve('scripts/resolve-launcher-config.ts').replaceAll('\\', '/');
      const tsx = resolve('node_modules/tsx/dist/cli.mjs').replaceAll('\\', '/');
      const run = (overrides: NodeJS.ProcessEnv = {}) => spawnSync(POSIX_SHELL, ['-c', `set -eu
eval "$(node "$1" "$2")"
printf '%s\n%s\n%s\n%s\n' "$HOST" "$PORT" "$DB_PATH" "$RESOLVED_RESUME"
`, 'launcher-config', tsx, resolver], {
        encoding: 'utf8',
        timeout: 5000,
        env: {
          ...process.env,
          SERVER_CONFIG: config,
          HOST: '', PORT: '', DB_PATH: '', SERVER_RESUME: '',
          SLITHER_HOST: '', SLITHER_PORT: '', SLITHER_DB_PATH: '',
          SLITHER_START_MODE: '', SLITHER_RESUME_TARGET: '',
          ...overrides
        }
      });

      const base = run();
      expect(base.error).toBeUndefined();
      expect(base.status, base.stderr).toBe(0);
      expect(base.stdout.trim().split('\n')).toEqual(['0.0.0.0', '6123', database.replaceAll('\\', '/'), 'auto']);

      const overridden = run({
        SLITHER_HOST: '192.168.1.25',
        SLITHER_PORT: '7123',
        SLITHER_DB_PATH: join(root, 'override.db').replaceAll('\\', '/'),
        SLITHER_START_MODE: 'resume',
        SLITHER_RESUME_TARGET: 'latest'
      });
      expect(overridden.error).toBeUndefined();
      expect(overridden.status, overridden.stderr).toBe(0);
      expect(overridden.stdout.trim().split('\n')).toEqual([
        '192.168.1.25',
        '7123',
        join(root, 'override.db').replaceAll('\\', '/'),
        'latest'
      ]);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  it('leaves the copied service environment override-free by default', () => {
    const example = readFileSync(resolve('server/systemd.env.example'), 'utf8');
    expect(example).toContain('Normal deployment settings come from server/config.toml.');
    expect(example).not.toMatch(/^SLITHER_(?:HOST|PORT|DB_PATH|START_MODE|RESUME_TARGET)=/mu);
  });

  it.each([
    ['0.0.0.0', 'http://127.0.0.1:5174/api/health'],
    ['::', 'http://[::1]:5174/api/health'],
    ['127.0.0.1', 'http://127.0.0.1:5174/api/health'],
    ['192.168.1.25', 'http://192.168.1.25:5174/api/health'],
    ['slither-pc', 'http://slither-pc:5174/api/health'],
    ['::1', 'http://[::1]:5174/api/health'],
    ['fd00::25', 'http://[fd00::25]:5174/api/health']
  ])('probes the configured bind without terminating a healthy server: %s', (host, expectedUrl) => {
    const begin = LAUNCHER.indexOf('probe_url_host() {');
    const end = LAUNCHER.indexOf('\nstart_server_process() {', begin);
    expect(begin).toBeGreaterThanOrEqual(0);
    expect(end).toBeGreaterThan(begin);
    const result = spawnSync(POSIX_SHELL, ['-c', `
set -eu
HOST="$1"
EXPECTED_URL="$2"
PORT=5174
HEALTH_URL="$EXPECTED_URL"
LOG_FILE=/dev/null
${LAUNCHER.slice(begin, end)}
pid_is_running() { return 0; }
# Stand in for a healthy server reachable only at the expected bind address.
node() { [ "$3" = "$EXPECTED_URL" ]; }
# An incorrect probe must fail immediately rather than consume the retry window.
sleep() { return 1; }
wait_for_health 123
printf '%s' "$HEALTH_URL"
`, 'launcher-probe', host, expectedUrl], { encoding: 'utf8', timeout: 5000 });
    expect(result.error).toBeUndefined();
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toBe(expectedUrl);
  });

  it('delegates fresh startup of an existing store to the server in both Unix launchers', () => {
    for (const source of [RUNNER, LAUNCHER]) {
      expect(source).toContain('if [ "$RESOLVED_RESUME" = "fresh" ] && [ ! -e "$DB_PATH" ]');
      expect(source).not.toMatch(/\b(?:rm|mv)\b[^\n]*\$DB_PATH/u);
    }
  });

  it('runs the Rust entry point without hard-coded deployment overrides', () => {
    expect(RUNNER).toContain('scripts/resolve-launcher-config.ts');
    expect(RUNNER).toContain('exec node ./node_modules/tsx/dist/cli.mjs server/rustServer.ts "$@"');
    expect(RUNNER).not.toMatch(/^\s*npm run build(?:\s|$)/mu);
    expect(RUNNER).not.toContain('server/index.ts');
    expect(RUNNER).not.toContain('--host "$HOST"');
    expect(RUNNER).not.toContain('--port "$PORT"');
    expect(RUNNER).not.toContain('--db-path "$DB_PATH"');
    expect(RUNNER).not.toContain('--input-hold-ms 500');
    expect(RUNNER).not.toContain('--disconnect-grace-ms 30000');
    expect(RUNNER).not.toContain('--checkpoint-every 1');
    expect(RUNNER).not.toMatch(/--(?:backend|mt)(?:[=\s]|$)/u);
  });

  it('uses bounded restart policy and graceful termination', () => {
    expect(SERVICE).toContain('ExecStart="@REPO_ROOT@/scripts/run-production.sh"');
    expect(SERVICE).toContain('WorkingDirectory=@REPO_ROOT@');
    expect(SERVICE).toContain('EnvironmentFile=-@REPO_ROOT@/server/systemd.env');
    expect(SERVICE).toContain('Restart=on-failure');
    expect(SERVICE).toContain('RestartSec=5');
    expect(SERVICE).toContain('StartLimitIntervalSec=120');
    expect(SERVICE).toContain('StartLimitBurst=3');
    expect(SERVICE).toContain('TimeoutStopSec=30');
    expect(SERVICE).toContain('KillSignal=SIGTERM');
  });

  it('installs a path-specific user unit without starting it implicitly', () => {
    expect(INSTALLER).toContain('systemctl --user daemon-reload');
    expect(INSTALLER).toContain('systemctl --user enable slither-neuroevo.service');
    expect(INSTALLER).not.toContain('enable --now');
    expect(INSTALLER).not.toMatch(/^\s*systemctl --user start(?:\s|$)/mu);
    expect(INSTALLER).toContain('loginctl show-user "$LOGIN_NAME" -p Linger');
    expect(INSTALLER).toContain('sudo loginctl enable-linger $LOGIN_NAME');
  });
});