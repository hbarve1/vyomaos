#!/usr/bin/env python3
"""
VyomaOS screenshot verifier.
Reads a QEMU screendump PPM (P6 binary) and checks basic UI invariants.
"""
import sys
import struct


def read_ppm(path):
    with open(path, 'rb') as f:
        header = f.readline().strip()
        if header != b'P6':
            raise ValueError(f"Not a P6 PPM: {header}")
        # Skip comments
        line = f.readline()
        while line.startswith(b'#'):
            line = f.readline()
        w, h = map(int, line.split())
        maxval = int(f.readline().strip())
        data = f.read()
    return w, h, maxval, data


def pixel(data, w, x, y):
    off = (y * w + x) * 3
    return data[off], data[off + 1], data[off + 2]


def check(path):
    w, h, maxval, data = read_ppm(path)
    print(f"Screenshot: {w}x{h}")

    errors = []

    # Check 1: menu bar region (y=0..23) is dark
    MENUBAR_H = 24
    dark_count = 0
    sample_count = 0
    for y in range(min(MENUBAR_H, h)):
        for x in range(0, w, 20):  # sample every 20px
            r, g, b = pixel(data, w, x, y)
            sample_count += 1
            if r < 80 and g < 80 and b < 80:
                dark_count += 1
    if sample_count > 0:
        dark_ratio = dark_count / sample_count
        print(f"Menu bar dark ratio: {dark_ratio:.2f} ({dark_count}/{sample_count})")
        if dark_ratio < 0.7:
            errors.append(f"Menu bar not dark enough: {dark_ratio:.2f} < 0.70")

    # Check 2: unique colors >= 30 (content was rendered)
    unique = set()
    for y in range(0, h, 5):
        for x in range(0, w, 5):
            r, g, b = pixel(data, w, x, y)
            unique.add((r >> 3, g >> 3, b >> 3))  # quantize to 5-bit buckets
    print(f"Unique color buckets: {len(unique)}")
    if len(unique) < 30:
        errors.append(f"Too few unique colors: {len(unique)} < 30 (image may be blank)")

    # Check 3: desktop region has non-black pixels
    non_black = 0
    for y in range(50, min(800, h), 10):
        for x in range(0, w, 10):
            r, g, b = pixel(data, w, x, y)
            if r > 5 or g > 5 or b > 5:
                non_black += 1
    print(f"Non-black pixels in desktop region: {non_black}")
    if non_black < 100:
        errors.append(f"Desktop region looks blank: only {non_black} non-black pixels")

    if errors:
        for e in errors:
            print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(1)
    else:
        print("SCREENSHOT: OK")


if __name__ == '__main__':
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <screenshot.ppm>", file=sys.stderr)
        sys.exit(1)
    check(sys.argv[1])
