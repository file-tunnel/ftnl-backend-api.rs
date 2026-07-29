# shellcheck shell=bash
set -euo pipefail

cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
bash scripts/formal-check.sh check
