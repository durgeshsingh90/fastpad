#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${FASTPAD_SKIP_BUILD:-0}" == "1" ]]; then
  APP="$ROOT/FastPad.app"
  if [[ ! -x "$APP/Contents/MacOS/FastPad" ]]; then
    echo "FastPad.app is missing; run scripts/build_macos_app.sh first." >&2
    exit 1
  fi
else
  APP="$("$ROOT/scripts/build_macos_app.sh")"
fi
ARGS=()

for arg in "$@"; do
  if [[ -e "$arg" ]]; then
    dir="$(cd "$(dirname "$arg")" && pwd -P)"
    ARGS+=("$dir/$(basename "$arg")")
  else
    ARGS+=("$arg")
  fi
done

if [[ ${#ARGS[@]} -gt 0 ]]; then
  open -n "$APP" --args "${ARGS[@]}"
else
  open -n "$APP"
fi

echo "Started FastPad: $APP"
