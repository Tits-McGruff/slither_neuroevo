#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

PID_FILE="${SLITHER_PID_FILE:-server.pid}"
PORT_FILE="${SLITHER_PORT_FILE:-server.port}"
DEFAULT_PORT="${SLITHER_PORT:-5174}"
LEGACY_DEV_PID_FILE="dev.pid"

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

pid_belongs_to_repo() {
  _pid="$1"
  if [ -d "/proc/${_pid}" ]; then
    _cwd=$(readlink -f "/proc/${_pid}/cwd" 2>/dev/null || true)
    case "$_cwd" in
      "$SCRIPT_DIR"|"$SCRIPT_DIR"/*) return 0 ;;
    esac
  fi
  _args=$(ps -o args= -p "$_pid" 2>/dev/null || true)
  printf '%s\n' "$_args" | grep -F "$SCRIPT_DIR" >/dev/null 2>&1
}

get_pgid() {
  ps -o pgid= -p "$1" 2>/dev/null | tr -d ' \t\r\n' || true
}

current_pgid() {
  ps -o pgid= -p "$$" 2>/dev/null | tr -d ' \t\r\n' || true
}

stop_pid_and_group() {
  _name="$1"
  _pid="$2"

  if ! pid_is_running "$_pid"; then
    return 0
  fi

  if ! pid_belongs_to_repo "$_pid"; then
    echo "[WARN] ${_name}: PID ${_pid} does not belong to $SCRIPT_DIR; leaving it alone."
    return 1
  fi

  _pgid=$(get_pgid "$_pid")
  _self_pgid=$(current_pgid)
  _group_safe=0
  if [ -n "$_pgid" ] && [ "$_pgid" != "$_self_pgid" ]; then
    _group_safe=1
  fi

  echo "[STOP] ${_name} PID ${_pid}"
  if [ "$_group_safe" -eq 1 ]; then
    kill -TERM "-$_pgid" 2>/dev/null || true
  else
    kill -TERM "$_pid" 2>/dev/null || true
  fi

  _tries=0
  while pid_is_running "$_pid" && [ "$_tries" -lt 20 ]; do
    _tries=$(( _tries + 1 ))
    sleep 0.5
  done

  if pid_is_running "$_pid"; then
    echo "[WARN] ${_name} did not exit after SIGTERM; sending SIGKILL."
    if [ "$_group_safe" -eq 1 ]; then
      kill -KILL "-$_pgid" 2>/dev/null || true
    else
      kill -KILL "$_pid" 2>/dev/null || true
    fi
  fi

  _tries=0
  while pid_is_running "$_pid" && [ "$_tries" -lt 10 ]; do
    _tries=$(( _tries + 1 ))
    sleep 0.2
  done

  if pid_is_running "$_pid"; then
    echo "[ERROR] ${_name} PID ${_pid} is still running."
    return 1
  fi
  return 0
}

pids_listening_on_port() {
  _port="$1"
  if command -v ss >/dev/null 2>&1; then
    ss -H -ltnp 2>/dev/null \
      | grep -E "[:.]${_port}[[:space:]]" \
      | sed -n 's/.*pid=\([0-9][0-9]*\).*/\1/p' \
      | sort -u
    return 0
  fi
  if command -v lsof >/dev/null 2>&1; then
    lsof -tiTCP:"$_port" -sTCP:LISTEN 2>/dev/null | sort -u
    return 0
  fi
  if command -v fuser >/dev/null 2>&1; then
    fuser -n tcp "$_port" 2>/dev/null | tr ' ' '\n' | sed -n 's/^\([0-9][0-9]*\)$/\1/p' | sort -u
    return 0
  fi
  return 0
}

PORT="$DEFAULT_PORT"
if [ -f "$PORT_FILE" ]; then
  _saved_port=$(tr -d ' \t\r\n' <"$PORT_FILE" 2>/dev/null || true)
  case "$_saved_port" in
    ''|*[!0-9]*) ;;
    *) PORT="$_saved_port" ;;
  esac
fi

SERVER_PID=$(read_pid "$PID_FILE")
if [ -n "$SERVER_PID" ]; then
  stop_pid_and_group "Rust server" "$SERVER_PID" || true
fi

# One-time compatibility with the old launcher, which also started Vite.
LEGACY_DEV_PID=$(read_pid "$LEGACY_DEV_PID_FILE")
if [ -n "$LEGACY_DEV_PID" ]; then
  stop_pid_and_group "legacy Vite server" "$LEGACY_DEV_PID" || true
fi

LISTENER_PIDS=$(pids_listening_on_port "$PORT" || true)
for _pid in $LISTENER_PIDS; do
  if pid_belongs_to_repo "$_pid"; then
    stop_pid_and_group "server listener on port $PORT" "$_pid" || true
  fi
done

LEFT=""
for _pid in $(pids_listening_on_port "$PORT" || true); do
  if pid_belongs_to_repo "$_pid"; then
    LEFT="$LEFT $_pid"
  fi
done

if [ -n "$LEFT" ]; then
  echo "[ERROR] Repo-owned process still listening on port $PORT:$LEFT"
  echo "[INFO] Keeping PID metadata so shutdown can be retried."
  exit 1
fi

rm -f "$PID_FILE" "$PORT_FILE" "$LEGACY_DEV_PID_FILE"
echo "[OK] Rust-authoritative server stopped."
exit 0
