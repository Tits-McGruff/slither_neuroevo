import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/** Production foreground launcher source. */
const RUNNER = readFileSync(resolve('scripts/run-production.sh'), 'utf8');

/** User-service template source. */
const SERVICE = readFileSync(resolve('scripts/slither-neuroevo.service.in'), 'utf8');

/** User-service installer source. */
const INSTALLER = readFileSync(resolve('scripts/install-systemd-user.sh'), 'utf8');

describe('production service operations', () => {
  it('runs the Rust entry point in the foreground without rebuilding or falling back', () => {
    expect(RUNNER).toContain('exec node ./node_modules/tsx/dist/cli.mjs server/rustServer.ts');
    expect(RUNNER).toContain('--resume "$RESUME_TARGET"');
    expect(RUNNER).toContain('--fresh');
    expect(RUNNER).not.toMatch(/^\s*npm run build(?:\s|$)/mu);
    expect(RUNNER).not.toContain('server/index.ts');
    expect(RUNNER).not.toContain('--backend');
  });

  it('uses bounded restart policy and graceful termination', () => {
    expect(SERVICE).toContain('ExecStart=@REPO_ROOT@/scripts/run-production.sh');
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
  });
});
