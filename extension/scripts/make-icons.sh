#!/usr/bin/env bash
# Regenerate the extension icons (toolbar + extension-management sizes).
#
# Source of truth: src-tauri/icons/ (the desktop app icon masters).
# Every size is downscaled from 128x128@2x.png (256px), the smallest master
# that is >= every target, so nothing is ever upscaled.
#
# Idempotent: safe to re-run; outputs are overwritten in place.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$ROOT/src-tauri/icons/128x128@2x.png"
OUT="$ROOT/extension/public/icons"

mkdir -p "$OUT"
for size in 16 32 48 128; do
  sips -z "$size" "$size" "$SRC" --out "$OUT/icon-$size.png" >/dev/null
done

echo "wrote $OUT/icon-{16,32,48,128}.png"
