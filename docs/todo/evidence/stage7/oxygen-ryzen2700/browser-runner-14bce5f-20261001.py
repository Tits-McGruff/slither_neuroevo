import json, os, subprocess, time, urllib.request, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parent
REMOTE='/srv/opt/apps/slither_neuroevo/data/codex-browser-gate-14bce5f'
TRAINER=Path(r'C:\Users\jlow8\source\repos\PyRL-trainer')
scenario=sys.argv[1]
assert scenario in ['P0','P1','P2','P3']
trainer=None
ssh=None
log=None
try:
    ssh=subprocess.Popen(['ssh','oxygen',f'cd {REMOTE} && timeout 3600 bash browser-server.sh {scenario}'])
    deadline=time.monotonic()+90
    while time.monotonic()<deadline:
        if ssh.poll() is not None: raise RuntimeError('server exited before readiness')
        try:
            with urllib.request.urlopen('http://192.168.0.200:5180/api/health',timeout=3) as response: health=json.load(response)
            if health.get('ok'):
                if '+14bce5f5f89a.' not in health['nativeBuildIdentifier']: raise RuntimeError('wrong build identity')
                break
        except (OSError,TimeoutError): pass
        time.sleep(.5)
    else: raise RuntimeError('server readiness deadline')
    env=os.environ.copy()
    env.update(SLITHER_WS_URL='ws://192.168.0.200:5180',SLITHER_ACTORS='2',SLITHER_BOT_NAME=f'Browser14b{scenario}',SLITHER_TRAIN_DEVICE='cpu',SLITHER_INFER_DEVICE='cpu',SLITHER_NET_HIDDEN='256',SLITHER_NET_LAYERS='2',SLITHER_CKPT_DIR=str(ROOT/f'trainer-{scenario}'),SLITHER_SAVE_EVERY_UPDATES='1000000',SLITHER_LOG_EVERY='30')
    log=(ROOT/f'trainer-{scenario}.log').open('w',encoding='utf-8')
    trainer=subprocess.Popen([str(TRAINER/'.venv/Scripts/python.exe'),'-m','pyrl_trainer'],cwd=TRAINER,env=env,stdout=log,stderr=subprocess.STDOUT)
    deadline=time.monotonic()+90
    while time.monotonic()<deadline:
        if trainer.poll() is not None: raise RuntimeError('trainer exited before readiness')
        with urllib.request.urlopen('http://192.168.0.200:5180/api/health',timeout=5) as response: health=json.load(response)
        activity=health['telemetry']['controllerActivity']['trainer']
        if activity['freshAssignments']>=2 and activity['appliedActions']>100: break
        time.sleep(.5)
    else: raise RuntimeError('two trainer actors did not become active')
    (ROOT/f'{scenario}-ready.json').write_text(json.dumps(health,indent=2),encoding='utf-8')
    print(f'{scenario} browser server and two actual trainer actors ready',flush=True)
    deadline=time.monotonic()+3300
    while time.monotonic()<deadline and not (ROOT/f'{scenario}-stop').exists():
        if trainer.poll() is not None or ssh.poll() is not None: raise RuntimeError('owned load process exited')
        time.sleep(1)
finally:
    if trainer is not None and trainer.poll() is None:
        trainer.terminate()
        try: trainer.wait(timeout=15)
        except subprocess.TimeoutExpired: trainer.kill(); trainer.wait(timeout=5)
    if ssh is not None and ssh.poll() is None:
        subprocess.run(['ssh','oxygen',f'cd {REMOTE} && owned_pid=$(cat browser-server.pid) && test "$(readlink /proc/$owned_pid/cwd)" = {REMOTE} && kill -TERM "$owned_pid"'],check=True,timeout=20)
        try: ssh.wait(timeout=20)
        except subprocess.TimeoutExpired: ssh.terminate(); ssh.wait(timeout=5)
    if log is not None: log.close()
    print(f'{scenario}: owned processes stopped',flush=True)
