import os
import sys
import pytest
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from assertions import (
    DESKTOP_BG, MENUBAR_BG, TOLERANCE,
    assert_pixel, assert_region_color, assert_no_ghost,
    assert_log_contains, assert_min_unique_colors,
)


def _solid(w, h, rgb):
    return Image.new("RGB", (w, h), rgb)


def test_assert_pixel_exact_match():
    img = _solid(10, 10, (100, 150, 200))
    assert_pixel(img, 5, 5, (100, 150, 200))


def test_assert_pixel_within_tolerance():
    img = _solid(10, 10, (100, 100, 100))
    assert_pixel(img, 0, 0, (90, 90, 90), tolerance=15)


def test_assert_pixel_fails_outside_tolerance():
    img = _solid(10, 10, (100, 100, 100))
    with pytest.raises(AssertionError, match=r"Pixel \(5,5\)"):
        assert_pixel(img, 5, 5, (0, 0, 0), tolerance=15)


def test_assert_region_color_solid_match():
    img = _solid(100, 100, DESKTOP_BG)
    assert_region_color(img, 0, 0, 100, 100, DESKTOP_BG, min_ratio=0.9)


def test_assert_region_color_fails_wrong_color():
    img = _solid(100, 100, (255, 0, 0))
    with pytest.raises(AssertionError, match="Region"):
        assert_region_color(img, 0, 0, 100, 100, DESKTOP_BG, min_ratio=0.5)


def test_assert_no_ghost_passes_on_desktop_bg():
    img = _solid(100, 100, DESKTOP_BG)
    assert_no_ghost(img, 10, 10, 50, 50)


def test_assert_no_ghost_fails_on_non_bg():
    img = _solid(100, 100, (255, 128, 0))
    with pytest.raises(AssertionError):
        assert_no_ghost(img, 10, 10, 50, 50)


def test_assert_log_contains_found(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("[lifecycle] all apps spawned\n")
    assert_log_contains(str(log), r"\[lifecycle\].*all apps spawned")


def test_assert_log_contains_not_found(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("nothing here\n")
    with pytest.raises(AssertionError, match="not found"):
        assert_log_contains(str(log), r"missing_pattern", timeout=0.2)


def test_assert_log_contains_missing_file(tmp_path):
    with pytest.raises(AssertionError, match="not found"):
        assert_log_contains(str(tmp_path / "no.log"), r"pattern", timeout=0.2)


def test_assert_min_unique_colors_pass():
    img = Image.new("RGB", (200, 200))
    pixels = [((x * 13 + y * 7) % 256, (y * 11) % 256, (x + y * 3) % 256)
              for y in range(200) for x in range(200)]
    img.putdata(pixels)
    assert_min_unique_colors(img, 30)


def test_assert_min_unique_colors_fails_solid():
    img = _solid(100, 100, (50, 50, 50))
    with pytest.raises(AssertionError, match="unique color buckets"):
        assert_min_unique_colors(img, 30)
