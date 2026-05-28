import time
from assertions import assert_no_ghost


def test_drag_leaves_no_ghost(vm):
    vm.move(200, 60)
    vm.click(200, 60)
    for dx in range(0, 100, 10):
        vm.move(200 + dx, 60)
        time.sleep(0.05)
    vm.move(300, 60)
    time.sleep(0.3)
    img = vm.screenshot()
    assert_no_ghost(img, 180, 45, 40, 30)
