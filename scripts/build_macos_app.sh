#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/FastPad.app"
BIN="$ROOT/target/release/FastPad"
ICON="$ROOT/crates/fastpad_app_macos/Assets/AppIcon.icns"

cargo build --release -p fastpad_app_macos --bin FastPad

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/FastPad"
cp "$ROOT/crates/fastpad_app_macos/Info.plist" "$APP/Contents/Info.plist"
if [[ -f "$ICON" ]]; then
  cp "$ICON" "$APP/Contents/Resources/AppIcon.icns"
fi
codesign --force --deep --sign - "$APP" >/dev/null

echo "$APP"
