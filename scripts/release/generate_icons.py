#!/usr/bin/env python3
"""Generate deterministic PNG/ICO packaging icons without third-party Python packages."""

from __future__ import annotations

import argparse
import binascii
import struct
import zlib
from pathlib import Path

BG = (10, 29, 72, 255)
CYAN = (53, 198, 255, 255)
WHITE = (244, 248, 255, 255)


def canvas(size: int) -> bytearray:
    return bytearray(size * size * 4)


def pixel(data: bytearray, size: int, x: int, y: int, color: tuple[int, int, int, int]) -> None:
    if 0 <= x < size and 0 <= y < size:
        offset = (y * size + x) * 4
        data[offset : offset + 4] = bytes(color)


def circle(data: bytearray, size: int, cx: int, cy: int, radius: int, color: tuple[int, int, int, int]) -> None:
    radius2 = radius * radius
    for y in range(cy - radius, cy + radius + 1):
        for x in range(cx - radius, cx + radius + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= radius2:
                pixel(data, size, x, y, color)


def rounded_rect(
    data: bytearray,
    size: int,
    left: int,
    top: int,
    right: int,
    bottom: int,
    radius: int,
    color: tuple[int, int, int, int],
) -> None:
    for y in range(top, bottom):
        for x in range(left, right):
            inner_x = min(max(x, left + radius), right - radius - 1)
            inner_y = min(max(y, top + radius), bottom - radius - 1)
            if (x - inner_x) ** 2 + (y - inner_y) ** 2 <= radius * radius:
                pixel(data, size, x, y, color)


def line(
    data: bytearray,
    size: int,
    x0: int,
    y0: int,
    x1: int,
    y1: int,
    width: int,
    color: tuple[int, int, int, int],
) -> None:
    dx, dy = x1 - x0, y1 - y0
    steps = max(abs(dx), abs(dy), 1)
    radius = max(1, width // 2)
    for step in range(steps + 1):
        x = x0 + dx * step // steps
        y = y0 + dy * step // steps
        circle(data, size, x, y, radius, color)


def render(size: int) -> bytearray:
    data = canvas(size)
    s = size / 64.0
    q = lambda value: int(round(value * s))

    rounded_rect(data, size, q(3), q(3), q(61), q(61), q(14), BG)

    # Neural-side speech bubble.
    rounded_rect(data, size, q(10), q(14), q(30), q(43), q(8), CYAN)
    rounded_rect(data, size, q(13), q(17), q(27), q(40), q(5), BG)
    line(data, size, q(16), q(43), q(16), q(49), q(3), CYAN)
    line(data, size, q(16), q(49), q(22), q(43), q(3), CYAN)
    for x, y in [(16, 24), (24, 23), (21, 33)]:
        circle(data, size, q(x), q(y), q(3), WHITE)
    line(data, size, q(17), q(25), q(23), q(24), q(2), WHITE)
    line(data, size, q(23), q(25), q(21), q(31), q(2), WHITE)
    line(data, size, q(20), q(31), q(17), q(26), q(2), WHITE)

    # Bridge arrow and MCP-side connector.
    line(data, size, q(31), q(31), q(43), q(31), q(4), CYAN)
    line(data, size, q(40), q(27), q(45), q(31), q(3), CYAN)
    line(data, size, q(40), q(35), q(45), q(31), q(3), CYAN)
    rounded_rect(data, size, q(44), q(23), q(54), q(39), q(4), WHITE)
    line(data, size, q(54), q(27), q(59), q(27), q(3), WHITE)
    line(data, size, q(54), q(35), q(59), q(35), q(3), WHITE)
    line(data, size, q(43), q(27), q(40), q(27), q(3), WHITE)
    line(data, size, q(43), q(35), q(40), q(35), q(3), WHITE)
    return data


def png_chunk(kind: bytes, payload: bytes) -> bytes:
    body = kind + payload
    return struct.pack(">I", len(payload)) + body + struct.pack(">I", binascii.crc32(body) & 0xFFFFFFFF)


def encode_png(size: int, rgba: bytes) -> bytes:
    stride = size * 4
    raw = b"".join(b"\x00" + rgba[y * stride : (y + 1) * stride] for y in range(size))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", header) + png_chunk(b"IDAT", zlib.compress(raw, 9)) + png_chunk(b"IEND", b"")


def encode_ico(png: bytes) -> bytes:
    # ICO supports PNG-compressed entries. Width/height 0 encode 256px.
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png), 6 + 16)
    return header + entry + png


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", default="target/release-icons")
    args = parser.parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    png_512 = encode_png(512, render(512))
    png_256 = encode_png(256, render(256))
    (out_dir / "llm2mcp.png").write_bytes(png_512)
    (out_dir / "llm2mcp.ico").write_bytes(encode_ico(png_256))
    print(out_dir / "llm2mcp.png")
    print(out_dir / "llm2mcp.ico")


if __name__ == "__main__":
    main()
