"""Рисует nebula-client.ico тем же рисунком, что crates/app/src/icons.rs (планета с орбитой).

Запуск: python dist/windows/make_icon.py  -> dist/windows/nebula-client.ico и dist/linux/nebula-client.png
"""
import math
import os
import struct
import zlib

COLOR = (0x2E, 0xC2, 0x7E)


def rgba(size):
    s = size / 32.0  # рисунок задан для 32 px
    c = (size - 1) / 2.0
    px = bytearray(size * size * 4)
    r, g, b = COLOR
    for y in range(size):
        for x in range(size):
            dx, dy = (x - c) / s, (y - c) / s
            d = math.hypot(dx, dy)
            disk = min(max(9.5 - d, 0.0) * s, 1.0)
            sn, co = -0.44, 0.9
            u, v = dx * co - dy * sn, dx * sn + dy * co
            e = math.sqrt((u / 15.0) ** 2 + (v / 5.5) ** 2)
            ring = min(max((1.0 - abs(e - 1.0) * 9.0) * s, 0.0), 1.0)
            if v < 0.0 and d < 10.5:
                ring = 0.0
            a = max(disk, ring)
            if a > 0:
                i = (y * size + x) * 4
                light = 28 if disk > 0 and dx + dy < -4.0 else 0
                px[i:i + 4] = bytes((min(r + light, 255), min(g + light, 255), min(b + light, 255), int(a * 255)))
    return bytes(px)


def png(size):
    raw = bytearray()
    data = rgba(size)
    for y in range(size):
        raw.append(0)
        raw += data[y * size * 4:(y + 1) * size * 4]

    def chunk(t, body):
        return struct.pack(">I", len(body)) + t + body + struct.pack(">I", zlib.crc32(t + body) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def ico(sizes):
    images = [png(s) for s in sizes]
    out = struct.pack("<HHH", 0, 1, len(sizes))
    off = 6 + 16 * len(sizes)
    for s, im in zip(sizes, images):
        out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(im), off)
        off += len(im)
    return out + b"".join(images)


if __name__ == "__main__":
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "nebula-client.ico")
    with open(path, "wb") as f:
        f.write(ico([16, 24, 32, 48, 64, 128, 256]))
    print(path)
    png_path = os.path.join(os.path.dirname(path), "..", "linux", "nebula-client.png")
    with open(png_path, "wb") as f:
        f.write(png(256))
    print(os.path.normpath(png_path))
