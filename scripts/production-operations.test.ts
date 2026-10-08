import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
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

  it('runs the Rust entry point in the foreground without rebuilding or falling back', () => {
    expect(RUNNER).toContain('exec node ./node_modules/tsx/dist/cli.mjs server/rustServer.ts');
    expect(RUNNER).toContain('--resume "$RESUME_TARGET"');
    expect(RUNNER).toContain('--fresh');
    expect(RUNNER).not.toMatch(/^\s*npm run build(?:\s|$)/mu);
    expect(RUNNER).not.toContain('server/index.ts');
    expect(RUNNER).not.toMatch(/--(?:backend|mt)(?:[=\s]|$)/u);
    expect(RUNNER).toContain('--input-hold-ms 500');
    expect(RUNNER).toContain('--disconnect-grace-ms 30000');
    expect(RUNNER).toContain('--checkpoint-every 1');
  });

  it('uses bounded restart policy and graceful termination', () => {
    expect(SERVICE).toContain('ExecStart="@REPO_ROOT@/scripts/run-production.sh"');
    expect(SERVICE).toContain('WorkingDirectory=@REPO_ROOT@');
    expect(SERVICE).toContain('EnvironmentFile="-@REPO_ROOT@/server/systemd.env"');
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
    expect(INSTALLER).toContain('loginctl show-user "$USER" -p Linger');
    expect(INSTALLER).toContain('sudo loginctl enable-linger $USER');
  });
});
