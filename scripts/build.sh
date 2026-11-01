#!/usr/bin/env bash
# Cross-compiles the release exe for Windows. Needs: rustup target
# x86_64-pc-windows-msvc and `cargo install --locked cargo-xwin`.
set -euo pipefail
cd "$(dirname "$0")/.."
XWIN_ACCEPT_LICENSE=1 cargo xwin build --release --target x86_64-pc-windows-msvc -p twenty-twenty-twenty "$@"
echo "target/x86_64-pc-windows-msvc/release/202020.exe"
