#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$("$ROOT/scripts/build_macos_app.sh")"
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
