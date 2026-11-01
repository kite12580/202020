#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy -p capsule-core --all-targets -- -D warnings
cargo test -p capsule-core
XWIN_ACCEPT_LICENSE=1 cargo xwin clippy --target x86_64-pc-windows-msvc -p twenty-twenty-twenty -- -D warnings
scripts/build.sh
exe=target/x86_64-pc-windows-msvc/release/202020.exe
imports=$(objdump -p "$exe")
if grep -qi 'vcruntime' <<<"$imports"; then
  echo "error: exe depends on the VC++ runtime" >&2
  exit 1
fi
if grep -qiE 'DLL Name: (d2d1|dwrite|ole32|combase)\.dll' <<<"$imports"; then
  echo "error: capsule bar DLLs are not delay-loaded" >&2
  exit 1
fi
objdump -h "$exe" | grep -q '\.rsrc' || { echo "error: manifest resource missing" >&2; exit 1; }
echo "ok"
