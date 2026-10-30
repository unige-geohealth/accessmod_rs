#!/bin/sh
# Full-stack development: `accessmod serve` (API) in the background and the
# client's Vite dev server (http://localhost:5173, proxying /api to it) in
# the foreground. Ctrl-C stops both. Arguments go to `accessmod serve`, e.g.
# `sh scripts/dev.sh --data _shared/dev-data`; ACCESSMOD_DEV_PORT sets the
# API port (default 5099). Rust changes need a restart; client changes
# reload live.
set -e
cd "$(dirname "$0")/.."
port=${ACCESSMOD_DEV_PORT:-5099}
api="http://127.0.0.1:$port"
if curl -s -o /dev/null "$api"; then
  echo "port $port is in use: stop that server or set ACCESSMOD_DEV_PORT" >&2
  exit 1
fi
cargo build -q -p accessmod-cli
./target/debug/accessmod serve --port "$port" "$@" &
serve=$!
trap 'kill "$serve" 2>/dev/null' EXIT INT TERM
until curl -sf -o /dev/null "$api/api/health"; do
  kill -0 "$serve" 2>/dev/null || { echo "accessmod serve did not start" >&2; exit 1; }
  sleep 0.2
done
ACCESSMOD_API="$api" npm -w @accessmod/web run dev
