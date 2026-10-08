#!/bin/sh
# Exercise the launcher's real build selection in a caller-owned private directory.
set -eu

LAUNCHER_PATH="$1"
cd "$2"
mkdir -p dist native node_modules server
touch dist/index.html native/index.js native/slither-native.fixture.node server/config.toml
BUILD_STAMP=node_modules/.slither-rust-server-build
BUILD_SCRIPT=$(sed -n '/^need_build=0$/,/^OLD_PID=/p' "$LAUNCHER_PATH" | sed '/^OLD_PID=/,$d')
test -n "$BUILD_SCRIPT"
unset PORT PUBLIC_WS_URL SERVER_CONFIG SLITHER_SKIP_BUILD

# Stand in only for the expensive build; ensure its child environment has the resolved port.
npm() {
  test "$1 $2" = 'run build'
  env | grep -Fx "PORT=$PORT" >/dev/null
  printf 'build=%s|%s\n' "$PORT" "${PUBLIC_WS_URL:-}" >>builds.log
}

PORT=6200
eval "$BUILD_SCRIPT"
# Reuse identical source and routing inputs.
eval "$BUILD_SCRIPT"
test "$(wc -l <builds.log | tr -d ' ')" = 1

# A changed runtime port must invalidate the cached browser route.
PORT=6201
eval "$BUILD_SCRIPT"
test "$(wc -l <builds.log | tr -d ' ')" = 2

# A changed split-host route must invalidate it too.
PUBLIC_WS_URL=ws://split-host:6201
export PUBLIC_WS_URL
eval "$BUILD_SCRIPT"
test "$(wc -l <builds.log | tr -d ' ')" = 3
cat builds.log
