#!/bin/sh
# Full-stack development in one terminal: the development server
# (scripts/dev-server.sh, restarted on Rust changes when watchexec is
# installed) in the background and the client's Vite dev server
# (http://localhost:5173, proxying /api to it) in the foreground. Ctrl-C
# stops both. For separate logs, run `npm run dev:server` and
# `npm run dev:web` in two terminals. Arguments go to `accessmod serve`.
set -e
cd "$(dirname "$0")/.."
port=${ACCESSMOD_DEV_PORT:-5099}
sh scripts/dev-server.sh "$@" &
server=$!
trap 'kill "$server" 2>/dev/null' EXIT INT TERM
until curl -sf -o /dev/null "http://127.0.0.1:$port/api/health"; do
	kill -0 "$server" 2>/dev/null || { echo "the development server did not start" >&2; exit 1; }
	sleep 0.5
done
ACCESSMOD_DEV_PORT=$port npm -w @accessmod/web run dev
