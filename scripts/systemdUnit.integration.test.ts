import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { expect, it } from 'vitest';

it.skipIf(process.platform !== 'linux').each(['yes', 'no'])('installs without USER in a path containing spaces (linger: %s)', linger => {
  const root = mkdtempSync(join(tmpdir(), 'slither-systemd-unit-'));
  const repository = join(root, 'repository with spaces');
  const scripts = join(repository, 'scripts');
  const bin = join(root, 'bin');
  const configuration = join(root, 'config');
  try {
    mkdirSync(scripts, { recursive: true });
    mkdirSync(bin);
    copyFileSync(resolve('scripts/install-systemd-user.sh'), join(scripts, 'install-systemd-user.sh'));
    copyFileSync(resolve('scripts/slither-neuroevo.service.in'), join(scripts, 'slither-neuroevo.service.in'));
    writeFileSync(join(scripts, 'run-production.sh'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
    // The installer writes only the private XDG tree; no real service manager is contacted.
    writeFileSync(join(bin, 'systemctl'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
    writeFileSync(join(bin, 'loginctl'), `#!/bin/sh
[ "$1" = "show-user" ] && [ "$2" = "$(id -un)" ] || exit 1
printf '${linger}\\n'
`, { mode: 0o755 });
    const installed = spawnSync('/bin/sh', [join(scripts, 'install-systemd-user.sh')], {
      encoding: 'utf8', timeout: 5000,
      env: { ...process.env, USER: undefined, XDG_CONFIG_HOME: configuration, PATH: `${bin}:${process.env['PATH'] ?? ''}` }
    });
    expect(installed.error).toBeUndefined();
    expect(installed.status, installed.stderr).toBe(0);
    if (linger === 'no') {
      const account = spawnSync('id', ['-un'], { encoding: 'utf8' });
      expect(account.status).toBe(0);
      expect(installed.stdout).toContain(`sudo loginctl enable-linger ${account.stdout.trim()}`);
    } else expect(installed.stdout).not.toContain('enable-linger');
    const unit = join(configuration, 'systemd/user/slither-neuroevo.service');
    expect(readFileSync(unit, 'utf8')).toContain(`EnvironmentFile=-${repository}/server/systemd.env`);
    const verified = spawnSync('systemd-analyze', ['verify', '--man=no', '--generators=no', unit], {
      encoding: 'utf8', timeout: 5000
    });
    expect(verified.error).toBeUndefined();
    expect(verified.status, verified.stderr).toBe(0);
    expect(verified.stderr).not.toMatch(/EnvironmentFile=.*(?:not absolute|ignoring)/u);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
