"""Measure complete LAN steering bounds separately from the P1 memory window."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

ROOT = Path(__file__).resolve().parent
REPO = Path(r'C:\Users\jlow8\source\repos\slither_neuroevo')
TRAINER_ROOT = Path(r'C:\Users\jlow8\source\repos\PyRL-trainer')
REMOTE = '/srv/opt/apps/slither_neuroevo/data/codex-reclaim-gate-2f1e4b5'
SOURCE = '2f1e4b5c887535ff20b6a9c5db37353b4208c08e'
for scenario in ['P1', 'P0', 'P2']:
    trainer = None
    ssh = None
    trainer_log = None
    try:
        if scenario != 'P1':
            setup = f'''set -eu
cd {REMOTE}
test "$(git rev-parse HEAD)" = {SOURCE}
test -z "$(ss -ltnH 'sport = :5180')"
df -B1 .
node --import tsx scripts/stage7/realtime-workload.ts --scenario {scenario} --db-path ./lan-{scenario}.sqlite --measure-seconds 1 --rust-workers 6
'''
            subprocess.run(['ssh', 'oxygen', setup], check=True, timeout=240)
        ssh = subprocess.Popen(['ssh', 'oxygen', f'cd {REMOTE} && timeout 300 bash lan-server.sh {scenario}'])
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if ssh.poll() is not None:
                raise RuntimeError('LAN server exited before readiness')
            try:
                with urllib.request.urlopen('http://192.168.0.200:5180/api/health', timeout=3) as response:
                    health = json.load(response)
                if health.get('ok'):
                    if '+2f1e4b5c8875.' not in health['nativeBuildIdentifier']:
                        raise RuntimeError('wrong addon identity')
                    break
            except (OSError, TimeoutError):
                pass
            time.sleep(0.5)
        else:
            raise RuntimeError('LAN server readiness deadline')
        env = os.environ.copy()
        env.update(SLITHER_WS_URL='ws://192.168.0.200:5180', SLITHER_ACTORS='2',
            SLITHER_BOT_NAME=f'Lan2f1e4b5{scenario}', SLITHER_TRAIN_DEVICE='cpu', SLITHER_INFER_DEVICE='cpu',
            SLITHER_NET_HIDDEN='256', SLITHER_NET_LAYERS='2', SLITHER_CKPT_DIR=str(ROOT / f'trainer-lan-{scenario}'),
            SLITHER_SAVE_EVERY_UPDATES='1000000', SLITHER_LOG_EVERY='30')
        trainer_log = (ROOT / f'trainer-lan-{scenario}.log').open('w', encoding='utf-8')
        trainer = subprocess.Popen([str(TRAINER_ROOT / '.venv/Scripts/python.exe'), '-m', 'pyrl_trainer'],
            cwd=TRAINER_ROOT, env=env, stdout=trainer_log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if trainer.poll() is not None:
                raise RuntimeError('trainer exited before LAN load')
            with urllib.request.urlopen('http://192.168.0.200:5180/api/health', timeout=5) as response:
                health = json.load(response)
            activity = health['telemetry']['controllerActivity']['trainer']
            if activity['freshAssignments'] >= 2 and activity['appliedActions'] > 100:
                break
            time.sleep(0.5)
        else:
            raise RuntimeError('two-actor readiness deadline')
        print(f'{scenario}: two real trainer actors live, starting 200 trials per route', flush=True)
        subprocess.run(['node', str(REPO / 'node_modules/tsx/dist/cli.mjs'),
            str(REPO / 'scripts/stage7/lan-turn-response.ts'), 'ws://192.168.0.200:5180',
            str(ROOT / f'lan-turn-{scenario.lower()}-2f1e4b5-20261001.json'), SOURCE, scenario, '200'],
            cwd=REPO, check=True, timeout=180)
    finally:
        if trainer is not None and trainer.poll() is None:
            trainer.terminate()
            try:
                trainer.wait(timeout=15)
            except subprocess.TimeoutExpired:
                trainer.kill()
                trainer.wait(timeout=5)
        if ssh is not None and ssh.poll() is None:
            stop = f'''set -eu
cd {REMOTE}
owned_pid=$(cat lan-server.pid)
test "$(readlink /proc/$owned_pid/cwd)" = {REMOTE}
kill -TERM "$owned_pid"
'''
            subprocess.run(['ssh', 'oxygen', stop], timeout=20, check=True)
            try:
                ssh.wait(timeout=20)
            except subprocess.TimeoutExpired:
                ssh.terminate()
                ssh.wait(timeout=5)
        if trainer_log is not None:
            trainer_log.close()
        print(f'{scenario}: owned LAN processes terminal', flush=True)
