#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT="${OUTPUT:-$ROOT_DIR/target/fastpad-benchmarks/latest.json}"
BYTES="${BYTES:-8M}"
ITERATIONS="${ITERATIONS:-3}"

cargo run --release -p fastpad_benchmarks -- \
  --bytes "$BYTES" \
  --iterations "$ITERATIONS" \
  --output "$OUTPUT" \
  "$@"
