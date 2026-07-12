#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <os-name> <binary-path> [output-dir]" >&2
  exit 1
fi

OS_NAME="$1"
BIN_PATH="$2"
OUT_DIR="${3:-dist}"
SKILL_SRC="skills/codex-loop"
STAGING="$OUT_DIR/codex-loop"
ARCHIVE="$OUT_DIR/codex-loop-${OS_NAME}.skill"

if [[ ! -f "$BIN_PATH" ]]; then
  echo "binary not found: $BIN_PATH" >&2
  exit 1
fi

rm -rf "$STAGING"
mkdir -p "$STAGING/assets/bin"

cp "$SKILL_SRC/SKILL.md" "$STAGING/"
cp "$SKILL_SRC/examples.md" "$STAGING/"
cp -R "$SKILL_SRC/scripts" "$STAGING/"
cp -R "$SKILL_SRC/references" "$STAGING/"
cp "$SKILL_SRC/assets/mcp-config.example.yaml" "$STAGING/assets/"
cp "$BIN_PATH" "$STAGING/assets/bin/"
chmod +x "$STAGING/scripts/"*.sh 2>/dev/null || true
chmod +x "$STAGING/assets/bin/"* 2>/dev/null || true

rm -f "$ARCHIVE"
(
  cd "$OUT_DIR"
  zip -r "$(basename "$ARCHIVE")" codex-loop
)

echo "packaged $ARCHIVE"
