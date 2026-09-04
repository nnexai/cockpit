#!/usr/bin/env python3
"""Render a dependency-free PNG through the Kitty graphics protocol."""

import base64
import binascii
import struct
import sys
import zlib

WIDTH = 256
HEIGHT = 128


def png_chunk(kind: bytes, data: bytes) -> bytes:
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", binascii.crc32(body))


def make_png() -> bytes:
    rows = bytearray()
    for y in range(HEIGHT):
        rows.append(0)  # PNG filter: none
        for x in range(WIDTH):
            if ((x // 16) + (y // 16)) % 2:
                rows.extend((0x35, 0xD0, 0xBA))
            else:
                rows.extend((0xFF, 0x5C, 0x8A))
    return b"".join(
        (
            b"\x89PNG\r\n\x1a\n",
            png_chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 2, 0, 0, 0)),
            png_chunk(b"IDAT", zlib.compress(bytes(rows), level=9)),
            png_chunk(b"IEND", b""),
        )
    )


def main() -> None:
    png = make_png()
    payload = base64.b64encode(png)
    if len(payload) > 4096:
        raise RuntimeError("smoke-test image exceeds one Kitty protocol chunk")

    output = sys.stdout.buffer
    output.write(b"Kitty graphics smoke test: a pink/teal checkerboard should appear below.\n")
    output.write(b"\x1b_Ga=T,f=100,t=d,q=2,i=1,c=32,r=8;" + payload + b"\x1b\\\n")
    output.write(b"If you only see text, the terminal dropped or did not render the image command.\n")
    output.flush()


if __name__ == "__main__":
    main()
