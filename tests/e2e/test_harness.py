import json
import os
import sys
import pytest
from unittest.mock import MagicMock, patch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import QmpClient

GREETING = json.dumps({"QMP": {"version": {}, "capabilities": []}}).encode() + b"\n"
OK       = json.dumps({"return": {}}).encode() + b"\n"


def _make_client(tmp_path):
    return QmpClient(
        str(tmp_path / "qmp.sock"),
        str(tmp_path / "serial.log"),
        str(tmp_path / "screen.ppm"),
    )


def _connected(tmp_path, extra=()):
    """Return (client, fake_socket) with connect() already called."""
    fake = MagicMock()
    fake.recv.side_effect = [GREETING, OK] + list(extra)
    with patch("harness.socket.socket") as cls, \
         patch("harness.os.path.exists", return_value=True):
        cls.return_value = fake
        c = _make_client(tmp_path)
        c.connect()
    return c, fake


def test_connect_sends_qmp_capabilities(tmp_path):
    c, fake = _connected(tmp_path)
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert b'"qmp_capabilities"' in sent


def test_move_sends_two_abs_events(tmp_path):
    c, fake = _connected(tmp_path, extra=[OK, OK])
    c.move(720, 450)
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert sent.count(b'"input-send-event"') == 2
    assert b'"axis": "x"' in sent
    assert b'"axis": "y"' in sent


def test_click_sends_four_events(tmp_path):
    c, fake = _connected(tmp_path, extra=[OK] * 4)
    c.click(100, 200)
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert sent.count(b'"input-send-event"') == 4


def test_key_sends_two_events_with_qcode(tmp_path):
    c, fake = _connected(tmp_path, extra=[OK, OK])
    c.key("ret")
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert sent.count(b'"input-send-event"') == 2
    assert b'"ret"' in sent
    assert b'"qcode"' in sent


def test_wait_log_returns_matching_line(tmp_path):
    (tmp_path / "serial.log").write_text("[lifecycle] all apps spawned\n")
    c = _make_client(tmp_path)
    result = c.wait_log(r"\[lifecycle\].*all apps spawned", timeout=1)
    assert "all apps spawned" in result


def test_wait_log_raises_timeout(tmp_path):
    (tmp_path / "serial.log").write_text("nothing\n")
    c = _make_client(tmp_path)
    with pytest.raises(TimeoutError, match="not seen"):
        c.wait_log(r"never_matches", timeout=0.2)


def test_close_closes_socket(tmp_path):
    c, fake = _connected(tmp_path)
    c.close()
    fake.close.assert_called_once()
