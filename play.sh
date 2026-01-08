#!/bin/sh
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
cd "$SCRIPT_DIR" || exit 1

echo "========================================"
echo "Slither Neuroevolution Launcher"
echo "========================================"

# --------------------------------------------------------------------
# Prerequisites
# --------------------------------------------------------------------

# Ensure Node.js is available on PATH.
if ! command -v node >/dev/null 2>&1; then
  echo "[ERROR] Node.js is not installed or not in your PATH."
  echo "Please download and install it from https://nodejs.org/"
  exit 1
fi

# --------------------------------------------------------------------
# Dependency installation
# --------------------------------------------------------------------

# Install dependencies when they are missing or incomplete.
# This protects against stale node_modules after a pull or dependency change.
need_install=0

# First-run: node_modules does not exist.
if [ ! -d "node_modules" ]; then
  need_install=1
else
  # Sanity check: verify required runtime dependency is resolvable.
  # If this fails, node_modules exists but the install is incomplete or stale.
  node -e "require.resolve('smol-toml')" >/dev/null 2>&1 || need_install=1
fi

if [ "$need_install" -eq 1 ]; then
  echo
  echo "[SETUP] Installing dependencies..."
  echo
  if [ -f "package-lock.json" ]; then
    # Reproducible install when a lockfile is present.
    if ! npm ci; then
      echo
      echo "[ERROR] Failed to install dependencies (npm ci)."
      exit 1
    fi
  else
    # Standard install when no lockfile is present.
    if ! npm install; then
      echo
      echo "[ERROR] Failed to install dependencies (npm install)."
      exit 1
    fi
  fi
  echo
  echo "[SUCCESS] Dependencies installed!"
fi

# --------------------------------------------------------------------
# Server process lifecycle
# --------------------------------------------------------------------

# Start the simulation server in the background and record its PID.
echo
echo "Starting Simulation Server..."
echo
npm run server &
SERVER_PID=$!

# Ensure the background server process is terminated on script exit/signals.
cleanup() {
  if [ -n "${SERVER_PID:-}" ]; then
    kill "$SERVER_PID" 2>/dev/null
  fi
}
trap cleanup EXIT INT TERM

# Fail fast if the server exits immediately (dependency/config/startup error).
sleep 1
if ! kill -0 "$SERVER_PID" 2>/dev/null; then
  echo
  echo "[ERROR] Simulation server exited during startup."
  echo "Check the server output above for the reason."
  exit 1
fi

# --------------------------------------------------------------------
# Frontend dev server
# --------------------------------------------------------------------

# Start Vite dev server. Auto-open a browser only when xdg-open is available.
# This avoids hard failures on headless servers/containers.
echo
echo "Starting Simulation..."
echo

OPEN_ARGS=""
if command -v xdg-open >/dev/null 2>&1; then
  OPEN_ARGS="--open"
else
  echo "xdg-open not found; not auto-opening a browser."
  echo "Open http://localhost:5173/ manually."
fi

npm run dev -- --force $OPEN_ARGS
