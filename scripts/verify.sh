#!/usr/bin/bash
set -euo pipefail
app_project=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$app_project"
export PATH="$HOME/.cargo/bin:/usr/bin"
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.local/state/omarchy-app-updates/build/development}"
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
/usr/share/omarchy/bin/omarchy plugin validate "$app_project"
tools/lint-qml
tools/test-qml
bash -n scripts/prepare-engine
git diff --check
