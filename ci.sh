#!/bin/bash
# Repo CI gate — bupha runs this natively in a detached worktree.
# fmt -> clippy -D warnings -> test -> doc via the xtask crate.
set -euo pipefail
cd "$(dirname "$0")"
cargo run --manifest-path xtask/Cargo.toml -- gate
