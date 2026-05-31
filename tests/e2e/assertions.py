"""Visual assertion helpers for E2E screenshot testing.

All functions load a PNG image from disk and perform pixel-level checks.
"""

from PIL import Image


def assert_pixel_color(image_path: str, x: int, y: int, expected_rgba: tuple, tolerance: int = 10):
    """Check that a single pixel matches the expected RGBA color.

    Args:
        image_path: Path to a PNG/PPM screenshot.
        x, y: Pixel coordinates.
        expected_rgba: Expected (R, G, B, A) or (R, G, B) tuple.
        tolerance: Max per-channel deviation allowed.

    Raises:
        AssertionError: If the pixel color is outside tolerance.
    """
    img = Image.open(image_path).convert("RGBA")
    actual = img.getpixel((x, y))
    expected = expected_rgba if len(expected_rgba) == 4 else (*expected_rgba, 255)

    for i, (a, e) in enumerate(zip(actual, expected)):
        if abs(a - e) > tolerance:
            channels = "RGBA"
            raise AssertionError(
                f"Pixel ({x},{y}) channel {channels[i]}: "
                f"expected {e} +/-{tolerance}, got {a}. "
                f"Full pixel: {actual}, expected: {expected}"
            )


def assert_region_not_black(image_path: str, x: int, y: int, w: int, h: int):
    """Verify that a region contains at least some non-black pixels.

    Samples every 4th pixel in the region for performance. A pixel is
    considered non-black if any of its RGB channels exceeds 10.

    Args:
        image_path: Path to a PNG/PPM screenshot.
        x, y: Top-left corner of the region.
        w, h: Width and height of the region.

    Raises:
        AssertionError: If the entire sampled region is black.
    """
    img = Image.open(image_path).convert("RGB")
    non_black = 0
    total = 0

    for py in range(y, min(y + h, img.height), 4):
        for px in range(x, min(x + w, img.width), 4):
            total += 1
            r, g, b = img.getpixel((px, py))
            if r > 10 or g > 10 or b > 10:
                non_black += 1

    if total == 0:
        raise AssertionError(
            f"Region ({x},{y},{w},{h}) is empty (outside image bounds)"
        )

    if non_black == 0:
        raise AssertionError(
            f"Region ({x},{y},{w},{h}) is entirely black "
            f"({total} pixels sampled, all black)"
        )


def assert_text_region_occupied(image_path: str, x: int, y: int, w: int, h: int,
                                 min_ratio: float = 0.05):
    """Verify that a region has enough non-background pixels to indicate text.

    Considers the most common color in the region as the background, then
    checks that at least min_ratio of sampled pixels differ from it.

    Args:
        image_path: Path to a PNG/PPM screenshot.
        x, y: Top-left corner of the region.
        w, h: Width and height of the region.
        min_ratio: Minimum fraction of non-background pixels required.

    Raises:
        AssertionError: If the region appears to contain no text content.
    """
    img = Image.open(image_path).convert("RGB")
    colors = {}
    total = 0

    step = 2
    for py in range(y, min(y + h, img.height), step):
        for px in range(x, min(x + w, img.width), step):
            total += 1
            # Quantize to reduce noise: group into 8-value buckets
            r, g, b = img.getpixel((px, py))
            key = (r >> 3, g >> 3, b >> 3)
            colors[key] = colors.get(key, 0) + 1

    if total == 0:
        raise AssertionError(
            f"Region ({x},{y},{w},{h}) is empty (outside image bounds)"
        )

    # Find the most common color (presumed background)
    bg_count = max(colors.values())
    non_bg = total - bg_count
    ratio = non_bg / total

    if ratio < min_ratio:
        raise AssertionError(
            f"Region ({x},{y},{w},{h}) appears empty: only {ratio:.1%} non-background "
            f"pixels ({non_bg}/{total}), need at least {min_ratio:.1%}"
        )
