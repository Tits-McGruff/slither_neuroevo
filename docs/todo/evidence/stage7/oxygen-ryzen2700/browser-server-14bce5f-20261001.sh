#!/bin/bash
set -eu
root=/srv/opt/apps/slither_neuroevo/data/codex-browser-gate-14bce5f
test "$(pwd -P)" = "$root"
test "$(git rev-parse HEAD)" = 14bce5f5f89a7cb5b5ac215eabf6a879dee2b735
case "$1" in P0|P1|P2|P3) ;; *) exit 2;; esac
node --import tsx server/rustServer.ts --host 0.0.0.0 --port 5180 --db-path "./browser-$1.sqlite" --resume latest --rust-workers 6 > "browser-$1-server.log" 2>&1 &
server_pid=$!
printf '%s\n' "$server_pid" > browser-server.pid
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
