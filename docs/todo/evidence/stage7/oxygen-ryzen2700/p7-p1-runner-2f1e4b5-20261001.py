"""Own one real trainer and one SSH-supervised P1 measurement; stop both on exit."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

ROOT = Path(__file__).resolve().parent
TRAINER_ROOT = Path(r'C:\Users\jlow8\source\repos\PyRL-trainer')
REMOTE = '/srv/opt/apps/slither_neuroevo/data/codex-reclaim-gate-2f1e4b5'
trainer = None
ssh = None
trainer_log = None
try:
    ssh = subprocess.Popen(['ssh', 'oxygen', f'cd {REMOTE} && timeout 2400 bash supervisor.sh'])
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if ssh.poll() is not None:
            raise RuntimeError(f'supervisor exited before health: {ssh.returncode}')
        try:
            with urllib.request.urlopen('http://192.168.0.200:5180/api/health', timeout=3) as response:
                health = json.load(response)
            if health.get('ok'):
                if '+2f1e4b5c8875.' not in health['nativeBuildIdentifier']:
                    raise RuntimeError('wrong native source identity')
                break
        except (OSError, TimeoutError):
            pass
        time.sleep(0.5)
    else:
        raise RuntimeError('server readiness deadline')
    env = os.environ.copy()
    env.update(SLITHER_WS_URL='ws://192.168.0.200:5180', SLITHER_ACTORS='2',
        SLITHER_BOT_NAME='Reclaim2f1e4b5', SLITHER_TRAIN_DEVICE='cpu', SLITHER_INFER_DEVICE='cpu',
        SLITHER_NET_HIDDEN='256', SLITHER_NET_LAYERS='2', SLITHER_CKPT_DIR=str(ROOT / 'trainer-checkpoints'),
        SLITHER_SAVE_EVERY_UPDATES='1000000', SLITHER_LOG_EVERY='30')
    trainer_log = (ROOT / 'trainer.log').open('w', encoding='utf-8')
    trainer = subprocess.Popen([str(TRAINER_ROOT / '.venv/Scripts/python.exe'), '-m', 'pyrl_trainer'],
        cwd=TRAINER_ROOT, env=env, stdout=trainer_log, stderr=subprocess.STDOUT)
    print(f'owned trainer pid={trainer.pid}; fresh checkpoint directory', flush=True)
    deadline = time.monotonic() + 2450
    while ssh.poll() is None:
        if trainer.poll() is not None:
            raise RuntimeError(f'trainer exited unexpectedly: {trainer.returncode}')
        if time.monotonic() > deadline:
            raise RuntimeError('supervised measurement deadline')
        time.sleep(0.5)
    print(f'supervisor terminal status={ssh.returncode}', flush=True)
    if ssh.returncode:
        raise RuntimeError('measurement failed; retain reports before cleanup')
finally:
    for proc in (trainer, ssh):
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
    if trainer_log is not None:
        trainer_log.close()
    print('local owned processes terminal', flush=True)