#!/bin/bash
# Local gate entry — the bupha/peregrin gate is workflows/gate.yaml
# (handl-gates rust campaign); this script runs the same xtask gate.
# fmt -> clippy -D warnings -> test -> doc via the xtask crate.
set -euo pipefail
cd "$(dirname "$0")"
cargo run --manifest-path xtask/Cargo.toml -- gate
