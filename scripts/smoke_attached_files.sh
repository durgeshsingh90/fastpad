#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

FILES=(
  "/Users/durgesh/Downloads/Native_macOS_Text_Editor_Project_Objective.md"
  "/Users/durgesh/Downloads/fastpad_ai_native_complete_srs.json"
  "/Users/durgesh/Downloads/fastpad_true_engineering_blueprint_unix_style.md"
  "/Users/durgesh/Downloads/fastpad_big_text_analysis_requirements_overview.md"
  "/Users/durgesh/Downloads/fastpad_ai_native_srs_overview.md"
  "/Users/durgesh/Downloads/fastpad_big_text_analysis_requirements.json"
  "/Users/durgesh/Downloads/fastpad_true_engineering_blueprint_unix_style.json"
)

for file in "${FILES[@]}"; do
  if [[ ! -f "$file" ]]; then
    echo "Missing smoke file: $file" >&2
    exit 1
  fi
done

FASTPAD_SMOKE_FILES="$(
  IFS=:
  echo "${FILES[*]}"
)"
export FASTPAD_SMOKE_FILES

cd "$ROOT"
cargo test -p fastpad_core --test smoke_files -- --nocapture

"$ROOT/scripts/build_macos_app.sh" >/dev/null
codesign --verify --deep --strict --verbose=2 "$ROOT/FastPad.app"

osascript -e 'tell application "FastPad" to quit' 2>/dev/null || true
sleep 1

for file in "${FILES[@]}"; do
  echo "Launching FastPad with $file"
  FASTPAD_SKIP_BUILD=1 "$ROOT/start.sh" "$file" >/dev/null
  sleep 2
  if ! pgrep -fl "FastPad.app/Contents/MacOS/FastPad" >/dev/null; then
    echo "FastPad did not stay running for: $file" >&2
    exit 1
  fi
  osascript -e 'tell application "FastPad" to quit' 2>/dev/null || true
  sleep 1
done

echo "Attached-file smoke test passed for ${#FILES[@]} files."
