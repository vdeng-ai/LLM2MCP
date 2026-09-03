#!/usr/bin/env bash
set -euo pipefail

TARGET="${TARGET:-x86_64-unknown-linux-gnu}"
VERSION="${VERSION:-$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["package"]["version"])')}"
OUT_DIR="${OUT_DIR:-release-artifacts}"
STAGE_DIR="target/release-packages/linux"

python3 scripts/release/generate_icons.py
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR" "$OUT_DIR"

cargo build --locked --release --target "$TARGET"
cargo packager --release --target "$TARGET" --formats appimage \
  --out-dir "$STAGE_DIR" \
  --binaries-dir "target/$TARGET/release"

artifact="$(find "$STAGE_DIR" -maxdepth 1 -type f -name '*.AppImage' -print -quit)"
test -n "$artifact"
normalized="$OUT_DIR/LLM2MCP_${VERSION}_x86_64.AppImage"
cp "$artifact" "$normalized"
if [[ -f "$artifact.sig" ]]; then
  cp "$artifact.sig" "$normalized.sig"
fi

printf 'Linux package: %s\n' "$normalized"
