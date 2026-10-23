#!/bin/sh
# Full Rust gate before a commit: fmt, clippy, tests, layers, LUT, schemas.
# Prints a short summary; exits non-zero on the first failure.
set -e
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy -q --workspace --all-targets -- -D warnings
out=$(cargo test -q --workspace 2>&1) || { echo "$out" | grep -E "FAILED|panicked|error" | head -40; exit 1; }
echo "$out" | awk '/test result/ {p+=$4; f+=$6; i+=$8} END {print "tests: " p " passed, " f " failed, " i " ignored"}'
cargo xtask layers
cargo xtask lut --check 2>&1 | tail -1
cargo xtask schemas --check
cargo xtask i18n --check
