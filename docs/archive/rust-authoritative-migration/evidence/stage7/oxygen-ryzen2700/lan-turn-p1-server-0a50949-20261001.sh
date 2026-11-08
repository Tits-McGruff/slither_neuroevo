#!/bin/bash
set -eu
root=/srv/opt/apps/slither_neuroevo/data/codex-rss-gate-0a50949
test "$(pwd -P)" = "$root"
node --import tsx server/rustServer.ts --host 0.0.0.0 --port 5180 --db-path ./soak.sqlite --resume latest --rust-workers 6 > lan-server.log 2>&1 &
server_pid=$!
printf '%s\n' "$server_pid" > lan-server.pid
cleanup(){
 trap - EXIT INT TERM HUP
 if kill -0 "$server_pid" 2>/dev/null; then test "$(readlink /proc/$server_pid/cwd)" = "$root"; kill -TERM "$server_pid"; fi
 wait "$server_pid" || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
wait "$server_pid"
