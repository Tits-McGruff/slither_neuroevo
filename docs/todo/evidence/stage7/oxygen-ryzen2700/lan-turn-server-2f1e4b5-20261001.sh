#!/bin/bash
set -eu
root=/srv/opt/apps/slither_neuroevo/data/codex-reclaim-gate-2f1e4b5
test "$(pwd -P)" = "$root"
test "$(git rev-parse HEAD)" = 2f1e4b5c887535ff20b6a9c5db37353b4208c08e
case "$1" in P0) db=lan-P0.sqlite;; P1) db=soak.sqlite;; P2) db=lan-P2.sqlite;; *) exit 2;; esac
test -f "$db"
node --import tsx server/rustServer.ts --host 0.0.0.0 --port 5180 --db-path "./$db" --resume latest --rust-workers 6 > "lan-$1-server.log" 2>&1 &
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
