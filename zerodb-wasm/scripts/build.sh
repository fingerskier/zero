#!/usr/bin/env bash
# Size-oriented wasm-pack build (M4a-a / ISSUES O4).
# LTO + opt-level=z on the release profile for this invocation only.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_PROFILE_RELEASE_OPT_LEVEL=z
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_STRIP=symbols
export CARGO_PROFILE_RELEASE_LTO=true
exec wasm-pack build --target web --release "$@"
