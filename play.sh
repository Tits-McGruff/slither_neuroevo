#!/bin/sh
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
cd "$SCRIPT_DIR" || exit 1

echo "========================================"
echo "Slither Neuroevolution Launcher"
echo "========================================"

# 1. Check if Node.js is installed
if ! command -v node >/dev/null 2>&1; then
  echo "[ERROR] Node.js is not installed or not in your PATH."
  echo "Please download and install it from https://nodejs.org/"
  exit 1
fi

# 2. Check if node_modules exists, install if missing
if [ ! -d "node_modules" ]; then
  echo
  echo "[FIRST RUN] Dependencies not found. Installing now..."
  echo
  if ! npm install; then
    echo
    echo "[ERROR] Failed to install dependencies."
    exit 1
  fi
  echo
  echo "[SUCCESS] Dependencies installed!"
fi

# 3. Start the simulation server in the background
echo
echo "Starting Simulation Server..."
echo
npm run server &
SERVER_PID=$!

cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null
  fi
}
trap cleanup EXIT INT TERM

# 4. Run the development server and open browser
echo
echo "Starting Simulation..."
echo "Your browser should open automatically."
echo
npm run dev -- --open --force
