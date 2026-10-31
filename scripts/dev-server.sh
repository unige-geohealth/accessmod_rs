#!/bin/sh
# Development server: `accessmod serve` (debug build), rebuilt and restarted
# on every Rust change when watchexec is installed (`brew install watchexec`,
# or a release binary). Pair it with `npm run dev:web` in another terminal,
# or run both with `npm run dev`.
#
#   ACCESSMOD_DEV_PORT   port (default 5099; dev:web proxies to it)
#   ACCESSMOD_DATA       data directory (default _shared/dev-data, so the
#                        projects of the installed application stay apart)
# Arguments go to `accessmod serve`.
set -e
cd "$(dirname "$0")/.."
port=${ACCESSMOD_DEV_PORT:-5099}
data=${ACCESSMOD_DATA:-_shared/dev-data}
if curl -s -o /dev/null "http://127.0.0.1:$port"; then
	echo "port $port is in use: stop that server or set ACCESSMOD_DEV_PORT" >&2
	exit 1
fi
if command -v watchexec >/dev/null; then
	exec watchexec --restart --watch crates --exts rs,toml --quiet -- \
		cargo run -q -p accessmod-cli -- serve --port "$port" --data "$data" "$@"
fi
echo "dev-server: no watchexec, Rust changes need a restart (brew install watchexec)" >&2
exec cargo run -q -p accessmod-cli -- serve --port "$port" --data "$data" "$@"
