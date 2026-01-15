#!/bin/sh

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd) || exit 1
cd "$SCRIPT_DIR" || exit 1

# --------------------------------------------------------------------
# Purpose
# --------------------------------------------------------------------
# Stop the detached processes started by play.sh, robustly.
# PID files alone are not always reliable with npm, tsx, vite, and WSL;
# sometimes the recorded PID is a wrapper and the real listener survives.
#
# This shutdown script therefore:
#   1) Tries to stop recorded PIDs (server.pid, dev.pid)
#   2) Reads server/config.toml to find uiPort and port
#   3) Finds processes actually LISTENing on those ports
#   4) Stops those listener processes, killing their process groups too
#   5) Only removes PID files when the ports are no longer being served
#
# It tries hard to only kill processes that belong to this repo directory.

# --------------------------------------------------------------------
# Small helpers
# --------------------------------------------------------------------

read_pidfile() {
  _file="$1"
  if [ -f "$_file" ]; then
    tr -d ' \t\r\n' <"$_file" 2>/dev/null || true
  else
    echo ""
  fi
}

is_pid_running() {
  _pid="$1"
  [ -n "$_pid" ] && kill -0 "$_pid" 2>/dev/null
}

# Return the process group id (PGID) for a PID, empty if unavailable.
get_pgid() {
  _pid="$1"
  ps -o pgid= -p "$_pid" 2>/dev/null | tr -d ' \t\r\n' || true
}

# Return the command line for a PID (best effort).
get_args() {
  _pid="$1"
  ps -o args= -p "$_pid" 2>/dev/null || true
}

# True if PID's command line looks like it belongs to this repo.
pid_belongs_to_repo() {
  _pid="$1"
  _args="$(get_args "$_pid")"
  echo "$_args" | grep -F "$SCRIPT_DIR" >/dev/null 2>&1
}

# Find PIDs listening on a TCP port, best effort, prefers ss then lsof then fuser.
pids_listening_on_port() {
  _port="$1"

  if command -v ss >/dev/null 2>&1; then
    # ss output typically contains: users:(("node",pid=2225,fd=20))
    ss -H -ltnp 2>/dev/null \
      | grep -E "[:.]${_port}[[:space:]]" \
      | sed -n 's/.*pid=\([0-9]\+\).*/\1/p' \
      | sort -u
    return 0
  fi

  if command -v lsof >/dev/null 2>&1; then
    lsof -tiTCP:"$_port" -sTCP:LISTEN 2>/dev/null | sort -u
    return 0
  fi

  if command -v fuser >/dev/null 2>&1; then
    # fuser prints PIDs, may include extra output, normalize to numbers only.
    fuser -n tcp "$_port" 2>/dev/null | tr ' ' '\n' | sed -n 's/^\([0-9]\+\)$/\1/p' | sort -u
    return 0
  fi

  echo ""
}

# Try to stop a PID cleanly, and also stop its whole process group.
# This is critical for vite, tsx, esbuild helpers, and other subprocess trees.
stop_pid_and_group() {
  _name="$1"
  _pid="$2"

  if ! is_pid_running "$_pid"; then
    return 0
  fi

  # Safety check: do not kill unrelated processes unless they look like this repo.
  if ! pid_belongs_to_repo "$_pid"; then
    echo "[INFO] ${_name}: PID ${_pid} does not look like it belongs to ${SCRIPT_DIR}, skipping."
    return 1
  fi

  _pgid="$(get_pgid "$_pid")"

  echo "[INFO] Stopping ${_name} PID ${_pid}..."

  # First try TERM on process group, then on PID.
  if [ -n "$_pgid" ]; then
    kill -TERM "-$_pgid" 2>/dev/null || true
  fi
  kill -TERM "$_pid" 2>/dev/null || true

  # Wait up to ~5 seconds.
  _i=0
  while [ "$_i" -lt 10 ]; do
    if ! is_pid_running "$_pid"; then
      echo "[INFO] ${_name} stopped."
      return 0
    fi
    _i=$(( _i + 1 ))
    sleep 0.5
  done

  echo "[INFO] ${_name} did not exit, sending SIGKILL..."

  if [ -n "$_pgid" ]; then
    kill -KILL "-$_pgid" 2>/dev/null || true
  fi
  kill -KILL "$_pid" 2>/dev/null || true

  # Wait a little more.
  _i=0
  while [ "$_i" -lt 6 ]; do
    if ! is_pid_running "$_pid"; then
      echo "[INFO] ${_name} stopped."
      return 0
    fi
    _i=$(( _i + 1 ))
    sleep 0.5
  done

  echo "[ERROR] ${_name} PID ${_pid} is still running."
  return 1
}

# --------------------------------------------------------------------
# Read config ports (uiPort, port) from server/config.toml
# --------------------------------------------------------------------

UI_PORT="5173"
SERVER_PORT="5174"

if [ -f "server/config.toml" ]; then
  # Use Node + smol-toml, because you already depend on it, and TOML parsing in sh is pain.
  cfg_lines="$(
    node -e "
      const fs=require('fs');
      const toml=require('smol-toml');
      let cfg={};
      try { cfg = toml.parse(fs.readFileSync('server/config.toml','utf8')); } catch { cfg = {}; }
      if (cfg.uiPort != null) console.log('UIPORT=' + cfg.uiPort);
      if (cfg.port != null) console.log('PORT=' + cfg.port);
    " 2>/dev/null
  )"

  echo "$cfg_lines" | while IFS= read -r line; do
    case "$line" in
      UIPORT=*) UI_PORT="${line#UIPORT=}" ;;
      PORT=*) SERVER_PORT="${line#PORT=}" ;;
    esac
  done
fi

# Normalize to digits only, keep defaults if garbage.
echo "$UI_PORT" | grep -E '^[0-9]+$' >/dev/null 2>&1 || UI_PORT="5173"
echo "$SERVER_PORT" | grep -E '^[0-9]+$' >/dev/null 2>&1 || SERVER_PORT="5174"

# --------------------------------------------------------------------
# Step 1, stop PIDs from pid files, best effort
# --------------------------------------------------------------------

SERVER_PID="$(read_pidfile server.pid)"
DEV_PID="$(read_pidfile dev.pid)"

if [ -n "$SERVER_PID" ]; then
  stop_pid_and_group "Simulation Server (pidfile)" "$SERVER_PID" || true
fi

if [ -n "$DEV_PID" ]; then
  stop_pid_and_group "Vite Dev Server (pidfile)" "$DEV_PID" || true
fi

# --------------------------------------------------------------------
# Step 2, if ports are still listening, stop the real listener processes
# --------------------------------------------------------------------

# Collect listener PIDs for both ports, de-duplicate.
LISTENER_PIDS="$( (pids_listening_on_port "$UI_PORT"; pids_listening_on_port "$SERVER_PORT") 2>/dev/null | sort -u )"

if [ -n "$LISTENER_PIDS" ]; then
  echo "[INFO] Detected listener processes on ports ${UI_PORT} and/or ${SERVER_PORT}, stopping them..."
  for pid in $LISTENER_PIDS; do
    # Label names based on port association is messy, just call them listeners.
    stop_pid_and_group "Listener" "$pid" || true
  done
fi

# --------------------------------------------------------------------
# Step 3, verify shutdown, only then delete pid files
# --------------------------------------------------------------------

# After kills, re-check listening PIDs that belong to this repo.
LEFT_UI="$(pids_listening_on_port "$UI_PORT" | while read -r p; do pid_belongs_to_repo "$p" && echo "$p"; done)"
LEFT_SRV="$(pids_listening_on_port "$SERVER_PORT" | while read -r p; do pid_belongs_to_repo "$p" && echo "$p"; done)"

if [ -z "$LEFT_UI" ] && [ -z "$LEFT_SRV" ]; then
  rm -f server.pid dev.pid
  echo "[OK] Shutdown complete."
else
  echo "[ERROR] Some repo processes are still listening:"
  [ -n "$LEFT_UI" ] && echo "  UI port ${UI_PORT} still has PIDs: $LEFT_UI"
  [ -n "$LEFT_SRV" ] && echo "  Server port ${SERVER_PORT} still has PIDs: $LEFT_SRV"
  echo "[INFO] Keeping pid files so you can retry shutdown without losing references."
fi
