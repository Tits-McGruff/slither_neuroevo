#!/bin/sh
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
cd "$SCRIPT_DIR" || exit 1

echo "========================================"
echo "Slither Neuroevolution Launcher"
echo "========================================"

# --------------------------------------------------------------------
# Prerequisites
# --------------------------------------------------------------------

# Ensure Node.js is available on PATH.
if ! command -v node >/dev/null 2>&1; then
  echo "[ERROR] Node.js is not installed or not in your PATH."
  echo "Please download and install it from https://nodejs.org/"
  exit 1
fi

# Ensure npm is available on PATH.
if ! command -v npm >/dev/null 2>&1; then
  echo "[ERROR] npm is not installed or not in your PATH."
  echo "It normally ships with Node.js; reinstall from https://nodejs.org/"
  exit 1
fi

# Ensure we are in a Node project directory.
if [ ! -f "package.json" ]; then
  echo "[ERROR] package.json not found in $SCRIPT_DIR"
  echo "Make sure you are running play.sh from the project directory."
  exit 1
fi

# --------------------------------------------------------------------
# Dependency installation
# --------------------------------------------------------------------

# Install dependencies when they are missing or incomplete.
# This protects against stale node_modules after a pull or dependency change.
need_install=0

# First-run: node_modules does not exist.
if [ ! -d "node_modules" ]; then
  need_install=1
else
  # Sanity check: verify required runtime dependency is resolvable.
  # If this fails, node_modules exists but the install is incomplete or stale.
  node -e "require.resolve('smol-toml')" >/dev/null 2>&1 || need_install=1
fi

if [ "$need_install" -eq 1 ]; then
  echo
  echo "[SETUP] Installing dependencies..."
  echo
  if [ -f "package-lock.json" ]; then
    # Reproducible install when a lockfile is present.
    if ! npm ci; then
      echo
      echo "[ERROR] Failed to install dependencies (npm ci)."
      exit 1
    fi
  else
    # Standard install when no lockfile is present.
    if ! npm install; then
      echo
      echo "[ERROR] Failed to install dependencies (npm install)."
      exit 1
    fi
  fi
  echo
  echo "[SUCCESS] Dependencies installed!"
fi

# --------------------------------------------------------------------
# Detached mode defaults
# --------------------------------------------------------------------
# This version is designed to keep running after the shell ends.
# It starts BOTH the simulation server and the Vite dev server detached,
# writes PID files, and logs to server.log/dev.log.
#
# Stop later with:
#   sh shutdown.sh
#
# Logs:
#   tail -f server.log
#   tail -f dev.log

# --------------------------------------------------------------------
# Small helpers
# --------------------------------------------------------------------

read_pid() {
  if [ -f "$1" ]; then
    tr -d ' \t\r\n' <"$1" 2>/dev/null || true
  else
    echo ""
  fi
}

pid_is_running() {
  _pid="$1"
  [ -n "$_pid" ] && kill -0 "$_pid" 2>/dev/null
}

wait_startup_ok() {
  _pid="$1"
  _tries=0
  while [ "$_tries" -lt 20 ]; do
    if pid_is_running "$_pid"; then
      return 0
    fi
    _tries=$(( _tries + 1 ))
    sleep 0.5
  done
  return 1
}

# Start a command detached, preferring setsid so the PID becomes a session leader.
# That lets shutdown.sh kill the whole process group reliably via kill -PID.
start_detached() {
  _name="$1"
  _pidfile="$2"
  _logfile="$3"
  _cmd="$4"

  echo
  echo "Starting ${_name} (detached)..."
  echo

  if [ -f "$_pidfile" ]; then
    _old_pid="$(read_pid "$_pidfile")"
    if pid_is_running "$_old_pid"; then
      echo "[INFO] ${_name} already running with PID ${_old_pid}"
      return 0
    fi
    rm -f "$_pidfile"
  fi

  if command -v setsid >/dev/null 2>&1; then
    nohup setsid sh -c "exec ${_cmd}" </dev/null >"$_logfile" 2>&1 &
  else
    nohup sh -c "exec ${_cmd}" </dev/null >"$_logfile" 2>&1 &
  fi

  _pid=$!
  echo "$_pid" >"$_pidfile"

  if ! wait_startup_ok "$_pid"; then
    echo
    echo "[ERROR] ${_name} exited during startup."
    echo "Check ${_logfile} for the reason."
    exit 1
  fi
}

# --------------------------------------------------------------------
# Start services
# --------------------------------------------------------------------

start_detached "Simulation Server" "server.pid" "server.log" "npm run server"
start_detached "Vite Dev Server"   "dev.pid"    "dev.log"    "npm run dev -- --force"

echo
echo "[OK] Simulation server running   PID: $(read_pid server.pid)   Log: server.log"
echo "[OK] Vite dev server running     PID: $(read_pid dev.pid)      Log: dev.log"
echo

# --------------------------------------------------------------------
# Connection details (from ./server/config.toml, enumerate real IPs for 0.0.0.0)
# --------------------------------------------------------------------

CFG_HOST=""
CFG_PORT=""
CFG_UIHOST=""
CFG_UIPORT=""
CFG_PUBLIC_WS_URL=""
CFG_IPS=""

if [ -f "server/config.toml" ]; then
  while IFS= read -r line; do
    case "$line" in
      HOST=*) CFG_HOST="${line#HOST=}" ;;
      PORT=*) CFG_PORT="${line#PORT=}" ;;
      UIHOST=*) CFG_UIHOST="${line#UIHOST=}" ;;
      UIPORT=*) CFG_UIPORT="${line#UIPORT=}" ;;
      PUBLIC_WS_URL=*) CFG_PUBLIC_WS_URL="${line#PUBLIC_WS_URL=}" ;;
      IPS=*) CFG_IPS="${line#IPS=}" ;;
    esac
  done <<EOF
$(node -e "
const fs=require('fs');
const toml=require('smol-toml');
const os=require('os');

let cfg={};
try { cfg = toml.parse(fs.readFileSync('server/config.toml','utf8')); } catch { cfg = {}; }

const ifs=os.networkInterfaces();
const ips=[];
for (const name of Object.keys(ifs)) {
  for (const i of (ifs[name]||[])) {
    if (i && i.family==='IPv4' && !i.internal) ips.push(i.address);
  }
}

function score(ip){
  if (/^10\\./.test(ip)) return 0;
  if (/^192\\.168\\./.test(ip)) return 1;
  const m = ip.match(/^172\\.(\\d+)\\./);
  if (m) {
    const n = parseInt(m[1],10);
    if (n>=16 && n<=31) return 2;
  }
  return 9;
}
ips.sort((a,b)=>score(a)-score(b) || a.localeCompare(b));

function out(k,v){ process.stdout.write(k+'='+(v==null?'':String(v))+'\\n'); }
out('HOST', cfg.host);
out('PORT', cfg.port);
out('UIHOST', cfg.uiHost);
out('UIPORT', cfg.uiPort);
out('PUBLIC_WS_URL', cfg.publicWsUrl || '');
out('IPS', ips.join(' '));
")
EOF
fi

[ -z "$CFG_UIPORT" ] && CFG_UIPORT="5173"
[ -z "$CFG_PORT" ] && CFG_PORT="5174"

echo "Connection details:"
echo
echo "UI Local:       http://localhost:$CFG_UIPORT/"
echo "Server Local:   http://localhost:$CFG_PORT/"
echo

UI_CONNECT_HOSTS=""
SERVER_CONNECT_HOSTS=""

case "$CFG_UIHOST" in
  "0.0.0.0") UI_CONNECT_HOSTS="$CFG_IPS" ;;
  ""|"127.0.0.1"|"localhost") UI_CONNECT_HOSTS="" ;;
  *) UI_CONNECT_HOSTS="$CFG_UIHOST" ;;
esac

case "$CFG_HOST" in
  "0.0.0.0") SERVER_CONNECT_HOSTS="$CFG_IPS" ;;
  ""|"127.0.0.1"|"localhost") SERVER_CONNECT_HOSTS="" ;;
  *) SERVER_CONNECT_HOSTS="$CFG_HOST" ;;
esac

if [ -n "$UI_CONNECT_HOSTS" ]; then
  for ip in $UI_CONNECT_HOSTS; do
    echo "UI Network:     http://$ip:$CFG_UIPORT/"
  done
  echo
fi

if [ -n "$SERVER_CONNECT_HOSTS" ]; then
  for ip in $SERVER_CONNECT_HOSTS; do
    echo "Server Network: http://$ip:$CFG_PORT/"
  done
  echo
fi

if [ -n "$CFG_PUBLIC_WS_URL" ]; then
  echo "WebSocket Public: $CFG_PUBLIC_WS_URL"
  echo
else
  echo "WebSocket Local:   ws://localhost:$CFG_PORT/"
  if [ -n "$UI_CONNECT_HOSTS" ]; then
    for ip in $UI_CONNECT_HOSTS; do
      echo "WebSocket Network: ws://$ip:$CFG_PORT/"
    done
  elif [ -n "$SERVER_CONNECT_HOSTS" ]; then
    for ip in $SERVER_CONNECT_HOSTS; do
      echo "WebSocket Network: ws://$ip:$CFG_PORT/"
    done
  fi
  echo
fi

echo "Open the UI URL in your browser."
echo
exit 0
