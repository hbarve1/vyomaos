import time
from assertions import assert_min_unique_colors


def test_keyboard_reaches_shell(vm):
    vm.key("ret")
    time.sleep(0.5)
    img = vm.screenshot()
    assert_min_unique_colors(img, 30)
