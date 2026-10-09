#!/bin/sh
# Fetch the pinned AccessMod 5 source (read-only reference) into _am5/accessmod.
# Idempotent: an existing clone is only verified.
set -eu

TAG="5.9.2-beta.6"
COMMIT="e845cc96d7ba412665f02833f5d0c75ae305fb87"
REPO="https://github.com/unige-geohealth/accessmod"

root=$(cd "$(dirname "$0")/.." && pwd)
dest="$root/_am5/accessmod"

if [ ! -d "$dest/.git" ]; then
    mkdir -p "$root/_am5"
    git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$TAG" "$REPO" "$dest"
fi

head=$(git -C "$dest" rev-parse HEAD)
if [ "$head" != "$COMMIT" ]; then
    echo "error: $dest is at $head, expected $COMMIT ($TAG)" >&2
    exit 1
fi

echo "AccessMod 5 $TAG at $dest"
