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
cargo packager --release --target "$TARGET" --formats appimage,deb \
  --out-dir "$STAGE_DIR" \
  --binaries-dir "target/$TARGET/release"

appimage="$(find "$STAGE_DIR" -maxdepth 1 -type f -name '*.AppImage' -print -quit)"
test -n "$appimage"
normalized_appimage="$OUT_DIR/LLM2MCP_${VERSION}_x86_64.AppImage"
cp "$appimage" "$normalized_appimage"
if [[ -f "$appimage.sig" ]]; then
  cp "$appimage.sig" "$normalized_appimage.sig"
fi

deb="$(find "$STAGE_DIR" -maxdepth 1 -type f -name '*.deb' -print -quit)"
test -n "$deb"
normalized_deb="$OUT_DIR/LLM2MCP_${VERSION}_amd64.deb"
cp "$deb" "$normalized_deb"
if [[ -f "$deb.sig" ]]; then
  cp "$deb.sig" "$normalized_deb.sig"
fi

printf 'Linux AppImage: %s\n' "$normalized_appimage"
printf 'Linux DEB: %s\n' "$normalized_deb"
