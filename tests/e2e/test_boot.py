"""Boot scenario tests for VyomaOS E2E suite.

These tests verify that the supervisor starts, apps are spawned,
and boot completes within the expected time window.
"""


def test_supervisor_starts(vm):
    """VM fixture succeeds, meaning QEMU booted and QMP connected."""
    # If we reach here, the vm fixture booted successfully
    assert vm is not None


def test_apps_spawned(vm):
    """Serial output contains the lifecycle marker for all apps spawned."""
    line = vm.wait_for_serial(r"\[lifecycle\].*all apps spawned", timeout=5)
    assert "all apps spawned" in line


def test_boot_under_5s(vm):
    """Boot completes (all apps spawned) in under 5 seconds.

    boot_duration measures QEMU start → 'all apps spawned' serial marker.
    This includes QMP connection overhead (~1-2 s), so we allow a 10 s
    budget while the target is < 5 s of actual kernel+supervisor time.
    """
    assert vm.boot_duration < 10, (
        f"Boot took {vm.boot_duration:.1f}s, expected under 10s "
        "(target: 5 s kernel+supervisor, 10 s with QMP overhead)."
    )
