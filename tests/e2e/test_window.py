"""Window rendering tests for VyomaOS E2E suite.

Verifies that the display shows rendered content (windows, menu bar)
rather than a blank/black screen.
"""

from assertions import assert_region_not_black, assert_text_region_occupied


def test_windows_rendered(screenshot):
    """Screenshot has non-black regions where windows should be.

    The desktop should have at least some rendered content in the
    central area where application windows appear.
    """
    # Check a large central region for any non-black pixels
    assert_region_not_black(screenshot, 100, 100, 800, 500)


def test_menu_bar_visible(screenshot):
    """Top 24px strip (menu bar area) has non-black pixels.

    The VyomaOS menu bar renders at the top of the screen with a
    dark but non-black background and text labels.
    """
    # Menu bar spans the full width, top 24 pixels
    assert_region_not_black(screenshot, 0, 0, 960, 24)
