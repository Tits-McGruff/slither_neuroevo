#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

HOST="${SLITHER_HOST:-0.0.0.0}"
PORT="${SLITHER_PORT:-5174}"
DB_PATH="${SLITHER_DB_PATH:-./data/rust-authority.db}"
START_MODE="${SLITHER_START_MODE:-auto}"
RESUME_TARGET="${SLITHER_RESUME_TARGET:-latest}"
PID_FILE="${SLITHER_PID_FILE:-server.pid}"
PORT_FILE="${SLITHER_PORT_FILE:-server.port}"
LOG_FILE="${SLITHER_LOG_FILE:-server.log}"
BUILD_STAMP="node_modules/.slither-rust-server-build"
MANAGED_DIR="${DB_PATH}.checkpoints"

echo "========================================"
echo "Slither Neuroevolution Launcher"
echo "Rust-authoritative Debian server"
echo "========================================"

require_command() {
  _name="$1"
  if ! command -v "$_name" >/dev/null 2>&1; then
    echo "[ERROR] Required command not found: $_name"
    exit 1
  fi
}

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

stop_started_process() {
  _pid="$1"
  kill -TERM "-$_pid" 2>/dev/null || kill -TERM "$_pid" 2>/dev/null || true
}

wait_for_health() {
  _pid="$1"
  _url="http://127.0.0.1:${PORT}/api/health"
  _tries=0
  while [ "$_tries" -lt 120 ]; do
    if ! pid_is_running "$_pid"; then
      return 1
    fi
    if node -e "fetch(process.argv[1]).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))" "$_url" >/dev/null 2>&1; then
      return 0
    fi
    _tries=$(( _tries + 1 ))
    sleep 0.5
  done
  return 1
}

require_command node
require_command npm
require_command setsid

NODE_MAJOR=$(node -p "Number(process.versions.node.split('.')[0])")
if [ "$NODE_MAJOR" -lt 24 ]; then
  echo "[ERROR] Node.js 24 or newer is required; found $(node --version)."
  exit 1
fi

if [ ! -f package.json ]; then
  echo "[ERROR] package.json not found in $SCRIPT_DIR"
  exit 1
fi

need_install=0
if [ ! -d node_modules ] || [ ! -f native/node_modules/@napi-rs/cli/dist/cli.js ]; then
  need_install=1
elif ! node -e "require.resolve('smol-toml')" >/dev/null 2>&1; then
  need_install=1
fi

if [ "$need_install" -eq 1 ]; then
  echo
  echo "[SETUP] Installing dependencies..."
  if [ -f package-lock.json ]; then
    npm ci
  else
    npm install
  fi
fi

need_build=0
if [ "${SLITHER_SKIP_BUILD:-0}" != "1" ]; then
  if [ ! -f dist/index.html ] || [ ! -f native/index.js ] || [ ! -f "$BUILD_STAMP" ]; then
    need_build=1
  else
    have_native=0
    for _addon in native/slither-native.*.node; do
      if [ -f "$_addon" ]; then
        have_native=1
        break
      fi
    done
    if [ "$have_native" -eq 0 ]; then
      need_build=1
    fi
  fi

  if [ "$need_build" -eq 0 ]; then
    for _path in package.json package-lock.json tsconfig.json vite.config.ts index.html styles.css server src native/Cargo.toml native/Cargo.lock native/src; do
      if [ -e "$_path" ] && find "$_path" -type f -newer "$BUILD_STAMP" -print -quit 2>/dev/null | grep -q .; then
        need_build=1
        break
      fi
    done
  fi
fi

if [ "$need_build" -eq 1 ]; then
  echo
  echo "[SETUP] Building native addon and browser client..."
  npm run build
  touch "$BUILD_STAMP"
elif [ "${SLITHER_SKIP_BUILD:-0}" = "1" ]; then
  echo "[INFO] Build skipped because SLITHER_SKIP_BUILD=1."
else
  echo "[INFO] Existing build is current."
fi

OLD_PID=$(read_pid "$PID_FILE")
if pid_is_running "$OLD_PID"; then
  echo "[INFO] Rust server already running with PID $OLD_PID."
  echo "[INFO] Log: $LOG_FILE"
  exit 0
fi
rm -f "$PID_FILE" "$PORT_FILE"

mkdir -p "$(dirname "$DB_PATH")"

case "$START_MODE" in
  auto)
    if [ -e "$DB_PATH" ]; then
      ACTIVE_MODE="resume"
    else
      ACTIVE_MODE="fresh"
    fi
    ;;
  fresh|resume)
    ACTIVE_MODE="$START_MODE"
    ;;
  *)
    echo "[ERROR] SLITHER_START_MODE must be auto, fresh, or resume."
    exit 1
    ;;
esac

if [ "$ACTIVE_MODE" = "fresh" ]; then
  if [ -e "$DB_PATH" ]; then
    echo "[ERROR] Fresh start requested but database already exists: $DB_PATH"
    echo "[INFO] Use SLITHER_START_MODE=resume or choose a new SLITHER_DB_PATH."
    exit 1
  fi
  if [ -d "$MANAGED_DIR" ] && [ -n "$(ls -A "$MANAGED_DIR" 2>/dev/null || true)" ]; then
    echo "[ERROR] Fresh start requested but managed checkpoint directory is not empty: $MANAGED_DIR"
    exit 1
  fi
fi

COMMON_ARGS="--host $HOST --port $PORT --db-path $DB_PATH --backend native --mt=false --input-hold-ms 500 --disconnect-grace-ms 30000 --checkpoint-every 1"

echo
echo "[START] Rust-authoritative server"
echo "[INFO] Bind: $HOST:$PORT"
echo "[INFO] Database: $DB_PATH"
echo "[INFO] Mode: $ACTIVE_MODE${ACTIVE_MODE:+${ACTIVE_MODE:+}}"

if [ "$ACTIVE_MODE" = "fresh" ]; then
  nohup setsid sh -c 'exec npm run server:rust -- "$@"' sh --host "$HOST" --port "$PORT" --db-path "$DB_PATH" --backend native --mt=false --input-hold-ms 500 --disconnect-grace-ms 30000 --checkpoint-every 1 --fresh </dev/null >"$LOG_FILE" 2>&1 &
else
  nohup setsid sh -c 'exec npm run server:rust -- "$@"' sh --host "$HOST" --port "$PORT" --db-path "$DB_PATH" --backend native --mt=false --input-hold-ms 500 --disconnect-grace-ms 30000 --checkpoint-every 1 --resume "$RESUME_TARGET" </dev/null >"$LOG_FILE" 2>&1 &
fi

SERVER_PID=$!
echo "$SERVER_PID" >"$PID_FILE"
echo "$PORT" >"$PORT_FILE"

if ! wait_for_health "$SERVER_PID"; then
  echo
  echo "[ERROR] Rust server did not become healthy."
  echo "[INFO] Last server log lines:"
  tail -n 60 "$LOG_FILE" 2>/dev/null || true
  stop_started_process "$SERVER_PID"
  rm -f "$PID_FILE" "$PORT_FILE"
  exit 1
fi

echo
echo "[OK] Rust-authoritative server is healthy."
echo "[OK] PID: $SERVER_PID"
echo "[OK] Log: $LOG_FILE"
echo "[OK] Local health: http://127.0.0.1:$PORT/api/health"
echo

if command -v hostname >/dev/null 2>&1; then
  for _ip in $(hostname -I 2>/dev/null || true); do
    case "$_ip" in
      *:*) continue ;;
    esac
    echo "[LAN] Browser:   http://${_ip}:${PORT}/"
    echo "[LAN] WebSocket: ws://${_ip}:${PORT}"
  done
fi

echo
echo "Stop with: sh shutdown.sh"
echo
exit 0
