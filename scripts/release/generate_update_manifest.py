#!/usr/bin/env python3
"""Build cargo-packager-updater latest.json from normalized release artifacts."""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


def signature_for(path: Path) -> str:
    sig = Path(f"{path}.sig")
    if not sig.is_file():
        raise SystemExit(f"missing updater signature: {sig}")
    value = sig.read_text(encoding="utf-8").strip()
    if not value:
        raise SystemExit(f"empty updater signature: {sig}")
    return value


def one(paths: list[Path], pattern: str) -> Path:
    matches = sorted(path for path in paths if path.match(pattern))
    if len(matches) != 1:
        names = ", ".join(path.name for path in matches) or "none"
        raise SystemExit(f"expected exactly one {pattern}; found {names}")
    return matches[0]


def platform(url_base: str, path: Path, fmt: str) -> dict[str, str]:
    return {
        "url": f"{url_base}/{path.name}",
        "signature": signature_for(path),
        "format": fmt,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts-dir", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repo", required=True, help="owner/repository")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--notes", default="See the GitHub Release for details.")
    parser.add_argument("--output", default="latest.json")
    args = parser.parse_args()

    root = Path(args.artifacts_dir)
    paths = [path for path in root.iterdir() if path.is_file()]
    url_base = f"https://github.com/{args.repo}/releases/download/{args.tag}"

    linux = one(paths, "LLM2MCP_*_x86_64.AppImage")
    windows = one(paths, "LLM2MCP_*_x86_64-setup.exe")
    mac_arm = one(paths, "LLM2MCP_*_aarch64.app.tar.gz")
    mac_x64 = one(paths, "LLM2MCP_*_x86_64.app.tar.gz")

    manifest = {
        "version": args.version,
        "notes": args.notes,
        "pub_date": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "platforms": {
            "linux-x86_64": platform(url_base, linux, "appimage"),
            "windows-x86_64": platform(url_base, windows, "nsis"),
            "macos-aarch64": platform(url_base, mac_arm, "app"),
            "macos-x86_64": platform(url_base, mac_x64, "app"),
        },
    }
    output = Path(args.output)
    output.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
