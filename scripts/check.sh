#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy -p capsule-core --all-targets -- -D warnings
cargo test -p capsule-core
cargo clippy --target x86_64-pc-windows-msvc -p capsule-bar -- -D warnings
scripts/build.sh
exe=target/x86_64-pc-windows-msvc/release/capsule-bar.exe
if objdump -p "$exe" | grep -qi 'vcruntime'; then
  echo "error: exe depends on the VC++ runtime" >&2
  exit 1
fi
objdump -h "$exe" | grep -q '\.rsrc' || { echo "error: manifest resource missing" >&2; exit 1; }
echo "ok"
