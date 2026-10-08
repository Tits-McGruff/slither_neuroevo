#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

HOST="${SLITHER_HOST:-127.0.0.1}"
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

wait_for_process_exit() {
  _pid="$1"
  _tries=0
  while [ "$_tries" -lt 50 ]; do
    if ! pid_is_running "$_pid"; then
      return 0
    fi
    _tries=$(( _tries + 1 ))
    sleep 0.1
  done
  if pid_is_running "$_pid"; then
    kill -KILL "-$_pid" 2>/dev/null || kill -KILL "$_pid" 2>/dev/null || true
  fi
  _tries=0
  while [ "$_tries" -lt 20 ]; do
    if ! pid_is_running "$_pid"; then
      return 0
    fi
    _tries=$(( _tries + 1 ))
    sleep 0.1
  done
  return 1
}

# Probe explicit binds directly; wildcard binds use a reachable loopback URL.
probe_url_host() {
  case "$HOST" in
    0.0.0.0) printf '%s' '127.0.0.1' ;;
    ::) printf '%s' '[::1]' ;;
    \[*\]) printf '%s' "$HOST" ;;
    *:*) printf '[%s]' "$HOST" ;;
    *) printf '%s' "$HOST" ;;
  esac
}

HEALTH_URL="http://$(probe_url_host):${PORT}/api/health"

wait_for_health() {
  _pid="$1"
  _url="$HEALTH_URL"
  _tries=0
  while [ "$_tries" -lt 120 ]; do
    if ! pid_is_running "$_pid"; then
      return 1
    fi
    if node -e "fetch(process.argv[1]).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))" "$_url" >/dev/null 2>&1; then
      return 0
    fi
    if [ -f "$LOG_FILE" ] && grep -Fq '[rust.startup-fault]' "$LOG_FILE" 2>/dev/null; then
      return 1
    fi
    _tries=$(( _tries + 1 ))
    sleep 0.5
  done
  return 1
}

start_server_process() {
  _mode="$1"
  echo
  echo "[START] Rust-authoritative server"
  echo "[INFO] Bind: $HOST:$PORT"
  echo "[INFO] Database: $DB_PATH"
  echo "[INFO] Mode: $_mode"
  : >"$LOG_FILE"
  if [ "$_mode" = "fresh" ]; then
    nohup setsid npm run server -- --host "$HOST" --port "$PORT" --db-path "$DB_PATH" --input-hold-ms 500 --disconnect-grace-ms 30000 --checkpoint-every 1 --fresh </dev/null >"$LOG_FILE" 2>&1 &
  else
    nohup setsid npm run server -- --host "$HOST" --port "$PORT" --db-path "$DB_PATH" --input-hold-ms 500 --disconnect-grace-ms 30000 --checkpoint-every 1 --resume "$RESUME_TARGET" </dev/null >"$LOG_FILE" 2>&1 &
  fi
  SERVER_PID=$!
  echo "$SERVER_PID" >"$PID_FILE"
  echo "$PORT" >"$PORT_FILE"
}

print_start_failure() {
  echo
  echo "[ERROR] Rust server did not become healthy."
  echo "[INFO] Last server log lines:"
  tail -n 60 "$LOG_FILE" 2>/dev/null || true
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
# Build-time routing is part of the cached browser output.
BUILD_SETTINGS=$(printf 'port=%s\npublicWsUrl=%s\nconfig=%s\n' "$PORT" "${PUBLIC_WS_URL:-}" "${SERVER_CONFIG:-server/config.toml}")
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

  if [ "$need_build" -eq 0 ] && [ "$(cat "$BUILD_STAMP")" != "$BUILD_SETTINGS" ]; then
    need_build=1
  fi

  if [ "$need_build" -eq 0 ]; then
    for _path in package.json package-lock.json tsconfig.json vite.config.ts index.html styles.css server src native/Cargo.toml native/Cargo.lock native/src "${SERVER_CONFIG:-server/config.toml}"; do
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
  PORT="$PORT" npm run build
  printf '%s\n' "$BUILD_SETTINGS" >"$BUILD_STAMP"
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

# Existing stores are classified by the server; fresh appends only to managed databases.
if [ "$ACTIVE_MODE" = "fresh" ] && [ ! -e "$DB_PATH" ]; then
  if [ -d "$MANAGED_DIR" ] && [ -n "$(ls -A "$MANAGED_DIR" 2>/dev/null || true)" ]; then
    echo "[ERROR] Fresh start requested but managed checkpoint directory is not empty: $MANAGED_DIR"
    exit 1
  fi
fi

start_server_process "$ACTIVE_MODE"

if ! wait_for_health "$SERVER_PID"; then
  print_start_failure
  if grep -Fq '[rust.startup-fault]' "$LOG_FILE" 2>/dev/null; then
    echo "[ERROR] The existing database was not moved or replaced; no new run was started."
    echo "[INFO] The server remains health-only at $HEALTH_URL"
    echo "[INFO] Stop it with: sh shutdown.sh"
    exit 1
  fi
  stop_started_process "$SERVER_PID"
  wait_for_process_exit "$SERVER_PID" || true
  rm -f "$PID_FILE" "$PORT_FILE"
  exit 1
fi

echo
echo "[OK] Rust-authoritative server is healthy."
echo "[OK] PID: $SERVER_PID"
echo "[OK] Log: $LOG_FILE"
echo "[OK] Health: $HEALTH_URL"
echo

case "$HOST" in
  0.0.0.0|::)
    if command -v hostname >/dev/null 2>&1; then
      for _ip in $(hostname -I 2>/dev/null || true); do
        case "$_ip" in
          *:*) continue ;;
        esac
        echo "[LAN] Browser:   http://${_ip}:${PORT}/"
        echo "[LAN] WebSocket: ws://${_ip}:${PORT}"
      done
    fi
    ;;
  *)
    echo "[UI] Browser:   http://$(probe_url_host):${PORT}/"
    echo "[UI] WebSocket: ws://$(probe_url_host):${PORT}"
    ;;
esac

echo
echo "Stop with: sh shutdown.sh"
echo
exit 0
