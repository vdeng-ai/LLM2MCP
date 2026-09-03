#!/usr/bin/env bash
set -euo pipefail

TARGET="${TARGET:-$(rustc -vV | awk '/host:/ {print $2}')}"
ARCH="${ARCH:-$(case "$TARGET" in aarch64-*) echo aarch64 ;; x86_64-*) echo x86_64 ;; *) echo unknown ;; esac)}"
VERSION="${VERSION:-$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["package"]["version"])')}"
OUT_DIR="${OUT_DIR:-release-artifacts}"
STAGE_DIR="target/release-packages/macos-$ARCH"

if [[ "$ARCH" == "unknown" ]]; then
  echo "Unsupported macOS target: $TARGET" >&2
  exit 1
fi

python3 scripts/release/generate_icons.py
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR" "$OUT_DIR"

cargo build --locked --release --target "$TARGET"
cargo packager --release --target "$TARGET" --formats app,dmg \
  --out-dir "$STAGE_DIR" \
  --binaries-dir "target/$TARGET/release"

bundle="$(find "$STAGE_DIR" -maxdepth 1 -type d -name '*.app' -print -quit)"
test -n "$bundle"
archive="$(find "$STAGE_DIR" -maxdepth 1 -type f -name '*.app.tar.gz' -print -quit)"

# cargo-packager creates the updater archive automatically when a signing key is supplied.
# Keep local unsigned packaging useful too by creating the same archive shape when needed.
if [[ -z "$archive" ]]; then
  archive="$STAGE_DIR/$(basename "$bundle").tar.gz"
  tar -C "$STAGE_DIR" -czf "$archive" "$(basename "$bundle")"
fi

normalized="$OUT_DIR/LLM2MCP_${VERSION}_${ARCH}.app.tar.gz"
cp "$archive" "$normalized"
if [[ -f "$archive.sig" ]]; then
  cp "$archive.sig" "$normalized.sig"
fi

dmg="$(find "$STAGE_DIR" -maxdepth 1 -type f -name '*.dmg' -print -quit)"
test -n "$dmg"
normalized_dmg="$OUT_DIR/LLM2MCP_${VERSION}_${ARCH}.dmg"
cp "$dmg" "$normalized_dmg"

printf 'macOS updater bundle: %s\n' "$normalized"
printf 'macOS installer image: %s\n' "$normalized_dmg"
