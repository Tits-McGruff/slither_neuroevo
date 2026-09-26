#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
cd "$REPO_ROOT"

HOST="${SLITHER_HOST:-0.0.0.0}"
PORT="${SLITHER_PORT:-5174}"
DB_PATH="${SLITHER_DB_PATH:-$REPO_ROOT/data/rust-authority.db}"
START_MODE="${SLITHER_START_MODE:-auto}"
RESUME_TARGET="${SLITHER_RESUME_TARGET:-latest}"

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

mkdir -p "$(dirname -- "$DB_PATH")"

case "$START_MODE" in
  auto)
    if [ -e "$DB_PATH" ]; then
      set -- --resume "$RESUME_TARGET"
    else
      set -- --fresh
    fi
    ;;
  fresh)
    [ ! -e "$DB_PATH" ] || fail "Fresh start refused because the database already exists: $DB_PATH"
    [ ! -d "${DB_PATH}.checkpoints" ] || [ -z "$(ls -A "${DB_PATH}.checkpoints" 2>/dev/null || true)" ] || \
      fail "Fresh start refused because the managed checkpoint directory is not empty: ${DB_PATH}.checkpoints"
    set -- --fresh
    ;;
  resume)
    set -- --resume "$RESUME_TARGET"
    ;;
  *)
    fail "SLITHER_START_MODE must be auto, fresh, or resume."
    ;;
esac

echo "[START] Rust-authoritative server on ${HOST}:${PORT}"
echo "[INFO] Database: $DB_PATH"
exec node ./node_modules/tsx/dist/cli.mjs server/rustServer.ts \
  --host "$HOST" \
  --port "$PORT" \
  --db-path "$DB_PATH" \
  --backend native \
  --mt=false \
  --input-hold-ms 500 \
  --disconnect-grace-ms 30000 \
  --checkpoint-every 1 \
  "$@"
