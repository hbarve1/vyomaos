from assertions import MENUBAR_BG, DESKTOP_BG, assert_region_color, assert_min_unique_colors


def test_desktop_renders(vm):
    img = vm.screenshot()
    assert_min_unique_colors(img, 30)
    assert_region_color(img, 0, 0, img.width, 24, MENUBAR_BG, min_ratio=0.7)


def test_desktop_bg_present(vm):
    img = vm.screenshot()
    assert_region_color(
        img,
        100, 100, img.width - 200, img.height - 200,
        DESKTOP_BG, min_ratio=0.3,
    )
