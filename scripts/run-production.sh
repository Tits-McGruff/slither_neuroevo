#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
cd "$REPO_ROOT"

fail() {
  echo "[ERROR] $*" >&2
  exit 1
}

command -v node >/dev/null 2>&1 || fail "Node.js 24 or newer is required."
NODE_MAJOR=$(node -p "Number(process.versions.node.split('.')[0])")
[ "$NODE_MAJOR" -ge 24 ] || fail "Node.js 24 or newer is required; found $(node --version)."

[ -f node_modules/tsx/dist/cli.mjs ] || fail "Dependencies are missing. Run: npm ci"
[ -f native/index.js ] || fail "The native addon loader is missing. Run: npm run build"
[ -f dist/index.html ] || fail "The browser build is missing. Run: npm run build"

have_addon=0
for addon in native/slither-native.*.node; do
  if [ -f "$addon" ]; then
    have_addon=1
    break
  fi
done
[ "$have_addon" -eq 1 ] || fail "The release native addon is missing. Run: npm run build"

# Resolve the same TOML/env/CLI configuration the Rust server will consume.
# server/config.toml stays authoritative unless an explicit environment override is supplied.
eval "$(node ./node_modules/tsx/dist/cli.mjs scripts/resolve-launcher-config.ts)"
MANAGED_DIR="${DB_PATH}.checkpoints"

mkdir -p "$(dirname -- "$DB_PATH")"

if [ "$RESOLVED_RESUME" = "fresh" ] && [ ! -e "$DB_PATH" ]; then
  [ ! -d "$MANAGED_DIR" ] || [ -z "$(ls -A "$MANAGED_DIR" 2>/dev/null || true)" ] || \
    fail "Fresh start refused because the managed checkpoint directory is not empty: $MANAGED_DIR"
fi

echo "[START] Rust-authoritative server on ${HOST}:${PORT}"
echo "[INFO] Config: $CONFIG_PATH"
echo "[INFO] Database: $DB_PATH"
echo "[INFO] Mode: $RESOLVED_RESUME"
exec node ./node_modules/tsx/dist/cli.mjs server/rustServer.ts "$@"