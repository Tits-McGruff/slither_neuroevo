#!/bin/bash
set -eu
root=/srv/opt/apps/slither_neuroevo/data/codex-reclaim-gate-4b7f6b0
test "$(pwd -P)" = "$root"
test "$(git rev-parse HEAD)" = 4b7f6b09d73f3bee97ab25bd8ec8b4892d348659
server_pid=
player_pid=
monitor_pid=
cleanup() {
  trap - EXIT INT TERM
  if [ -n "$monitor_pid" ] && kill -0 "$monitor_pid" 2>/dev/null; then kill -USR1 "$monitor_pid"; wait "$monitor_pid" || true; fi
  for owned_pid in "$player_pid" "$server_pid"; do
    if [ -n "$owned_pid" ] && kill -0 "$owned_pid" 2>/dev/null; then
      test "$(readlink /proc/$owned_pid/cwd)" = "$root"
      kill -TERM "$owned_pid"
    fi
  done
  for owned_pid in "$player_pid" "$server_pid"; do
    if [ -n "$owned_pid" ]; then
      for attempt in $(seq 1 100); do
        kill -0 "$owned_pid" 2>/dev/null || break
        sleep 0.1
      done
      if kill -0 "$owned_pid" 2>/dev/null; then kill -KILL "$owned_pid"; fi
      wait "$owned_pid" 2>/dev/null || true
    fi
  done
  printf '%s\n' 'owned processes stopped'
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
node --import tsx server/rustServer.ts --host 0.0.0.0 --port 5180 --db-path ./soak.sqlite --resume latest --rust-workers 6 > server.log 2>&1 &
server_pid=$!
printf 'owned server pid=%s\n' "$server_pid"
node --input-type=module - <<'READY'
const deadline=Date.now()+300000;
while(Date.now()<deadline){
  try { const response=await fetch('http://127.0.0.1:5180/api/health',{signal:AbortSignal.timeout(5000)}); const health=await response.json();
    if(response.ok&&health.ok&&health.telemetry.controllerActivity.trainer.freshAssignments>=2&&health.telemetry.controllerActivity.trainer.appliedActions>100){console.log('two real trainer actors active; beginning measured window');process.exit(0);}
    if(health.interfaceFault)throw new Error(`interfaceFault: ${health.interfaceFault}`);
  }catch(error){if(String(error).includes('interfaceFault'))throw error;}
  await new Promise(resolve=>setTimeout(resolve,1000));
}
throw new Error('trainer readiness deadline reached');
READY
node --input-type=module - "$server_pid" > p7-p1-handles-4b7f6b0-20261001.json <<'HANDLES' &
import { readdir } from 'node:fs/promises';
const pid=process.argv[2];
const started=performance.now();
const startedAtUtc=new Date().toISOString();
const samples=[];
let sampling=Promise.resolve();
function sample(){sampling=sampling.then(async()=>{samples.push({wallSeconds:(performance.now()-started)/1000,fileDescriptors:(await readdir(`/proc/${pid}/fd`)).length,threads:(await readdir(`/proc/${pid}/task`)).length});});return sampling;}
await sample();
const timer=setInterval(()=>{void sample();},30000);
async function finish(){clearInterval(timer);clearTimeout(deadline);await sample();console.log(JSON.stringify({scope:'Linux production server /proc file-descriptor and thread counts, sampled every thirty seconds',startedAtUtc,pid:Number(pid),samples},null,2));process.exit(0);}
const deadline=setTimeout(()=>{void finish();},2400000);
process.once('SIGUSR1',()=>{void finish();});
HANDLES
monitor_pid=$!
node --import tsx scripts/stage7/soak-player.ts ws://127.0.0.1:5180 p7-p1-player-4b7f6b0-20261001.json 1830 &
player_pid=$!
set +e
node --import tsx scripts/stage7/loaded-runtime-window.ts http://127.0.0.1:5180 p7-p1-window-4b7f6b0-20261001.json 1800 4b7f6b09d73f3bee97ab25bd8ec8b4892d348659 P1 6
sampler_status=$?
wait "$player_pid"
player_status=$?
player_pid=
set -e
curl -fsS --max-time 15 http://127.0.0.1:5180/api/health > final-health.json
printf '\nsampler_status=%s player_status=%s\n' "$sampler_status" "$player_status"
test "$sampler_status" -eq 0
test "$player_status" -eq 0
