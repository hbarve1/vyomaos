import re
import time
from PIL import Image

DESKTOP_BG = (28, 28, 30)
MENUBAR_BG = (20, 20, 22)
TOLERANCE  = 15


def _matches(actual: tuple, expected: tuple, tolerance: int) -> bool:
    return all(abs(a - e) <= tolerance for a, e in zip(actual, expected))


def assert_pixel(img: Image.Image, x: int, y: int, rgb: tuple, tolerance: int = TOLERANCE):
    if x < 0 or x >= img.width or y < 0 or y >= img.height:
        raise AssertionError(
            f"Pixel ({x},{y}): coordinates out of bounds for {img.width}×{img.height} image"
        )
    actual = img.getpixel((x, y))[:3]
    if not _matches(actual, rgb, tolerance):
        raise AssertionError(
            f"Pixel ({x},{y}): expected RGB{rgb} ±{tolerance}, got RGB{actual}"
        )


def assert_region_color(
    img: Image.Image,
    x: int, y: int, w: int, h: int,
    rgb: tuple,
    min_ratio: float = 0.7,
    tolerance: int = TOLERANCE,
):
    match_count = 0
    total = 0
    for py in range(y, min(y + h, img.height), 5):
        for px in range(x, min(x + w, img.width), 5):
            total += 1
            if _matches(img.getpixel((px, py))[:3], rgb, tolerance):
                match_count += 1
    if total == 0:
        raise AssertionError(f"Region ({x},{y},{w},{h}) has no pixels")
    ratio = match_count / total
    if ratio < min_ratio:
        raise AssertionError(
            f"Region ({x},{y},{w},{h}): expected ≥{min_ratio:.0%} pixels matching "
            f"RGB{rgb} ±{tolerance}, got {ratio:.0%} ({match_count}/{total})"
        )


def assert_no_ghost(img: Image.Image, x: int, y: int, w: int, h: int):
    assert_region_color(img, x, y, w, h, DESKTOP_BG, min_ratio=0.7)


def assert_log_contains(serial_log: str, pattern: str, timeout: int = 5):
    regex = re.compile(pattern)
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with open(serial_log) as f:
                for line in f:
                    if regex.search(line):
                        return
        except FileNotFoundError:
            pass
        time.sleep(0.1)
    raise AssertionError(
        f"Pattern {pattern!r} not found in {serial_log} within {timeout}s"
    )


def assert_min_unique_colors(img: Image.Image, min_count: int = 30):
    buckets: set = set()
    for py in range(0, img.height, 5):
        for px in range(0, img.width, 5):
            r, g, b = img.getpixel((px, py))[:3]
            buckets.add((r >> 3, g >> 3, b >> 3))
    if len(buckets) < min_count:
        raise AssertionError(
            f"Expected ≥{min_count} unique color buckets (5-bit), got {len(buckets)}"
        )
