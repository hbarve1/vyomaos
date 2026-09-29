"""Local wrapper regression tests. Stub Codex/Docker; no model calls or services."""
import datetime
import fcntl
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/codex-worker.sh"


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vyoma-worker-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        (self.repo / ".git").mkdir(parents=True)
        self.state = self.root / "state"
        self.state.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.env = dict(os.environ, VYOMA_REPO=str(self.repo),
                        VYOMA_STATE_DIR=str(self.state),
                        VYOMA_MAX_RUNS_PER_DAY="8", VYOMA_RUN_MINUTES="1",
                        VYOMA_CODEX_BIN=str(self.bin / "codex"),
                        STUB_CALLS=str(self.root / "calls"),
                        PATH=str(self.bin) + ":" + os.environ["PATH"])
        self.executable("docker", "#!/bin/sh\nexit 0\n")
        self.executable("timeout", """#!/bin/sh
if [ "${STUB_TIMEOUT:-}" = 1 ]; then exit 124; fi
exec /usr/bin/timeout "$@"
""")
        self.executable("codex", """#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
with open(os.environ["STUB_CALLS"], "a") as f: f.write("call\\n")
assert "exec" in sys.argv and "--approve-for-me" in sys.argv
assert "--dangerously-bypass-approvals-and-sandbox" not in sys.argv
assert os.environ["VYOMA_RUN_ID"] in sys.stdin.read()
mode = os.environ.get("STUB_MODE", "progress")
if mode == "fail": sys.exit(7)
out = Path(sys.argv[sys.argv.index("--output-last-message") + 1])
if mode == "bad":
    out.write_text("broken")
else:
    out.write_text(json.dumps({"status": mode, "task": "test", "branch": "test",
        "base_sha": "base", "head_sha": "head", "parent_pr": None, "pr_url": None,
        "tests": [], "evidence": [], "blockers": [], "next_action": "continue"}))
print(json.dumps({"type": "turn.completed"}))
""")

    def executable(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o700)

    def run_worker(self, **changes):
        return subprocess.run(["bash", str(RUNNER)],
                              env=dict(self.env, **changes),
                              capture_output=True, text=True, timeout=10)

    def calls(self):
        path = self.root / "calls"
        return len(path.read_text().splitlines()) if path.exists() else 0

    def test_success_records_logs_budget_and_handoff(self):
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        run = Path((self.state / "latest").read_text().strip())
        self.assertEqual(json.loads((run / "result.json").read_text())["status"], "progress")
        self.assertTrue((run / "events.jsonl").read_text())
        self.assertEqual((run / "events.jsonl").stat().st_mode & 0o777, 0o600)
        self.assertFalse((self.state / "STOP").exists())
        self.assertEqual(self.calls(), 1)
        day = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
        self.assertEqual((self.state / ("runs-" + day)).read_text().strip(), "1")

    def test_requires_explicit_limits(self):
        self.assertNotEqual(self.run_worker(VYOMA_MAX_RUNS_PER_DAY="").returncode, 0)
        self.assertEqual(self.calls(), 0)

    def test_rejects_out_of_range_limits(self):
        for values in ({"VYOMA_MAX_RUNS_PER_DAY": "0"}, {"VYOMA_RUN_MINUTES": "46"},
                       {"VYOMA_MAX_RUNS_PER_DAY": "49"}):
            self.assertNotEqual(self.run_worker(**values).returncode, 0)
        self.assertEqual(self.calls(), 0)

    def test_daily_limit_survives_new_processes(self):
        for _ in range(3):
            self.assertEqual(self.run_worker(VYOMA_MAX_RUNS_PER_DAY="2").returncode, 0)
        self.assertEqual(self.calls(), 2)

    def test_stop_file_prevents_launch(self):
        (self.state / "STOP").write_text("operator pause")
        self.assertEqual(self.run_worker().returncode, 0)
        self.assertEqual(self.calls(), 0)

    def test_lock_prevents_overlapping_workers(self):
        with open(self.state / "worker.lock", "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.run_worker().returncode, 0)
        self.assertEqual(self.calls(), 0)

    def test_three_failures_pause_and_consume_slots(self):
        for _ in range(3):
            self.assertEqual(self.run_worker(STUB_MODE="fail").returncode, 7)
        self.assertTrue((self.state / "STOP").exists())
        self.assertEqual(self.run_worker().returncode, 0)
        self.assertEqual(self.calls(), 3)

    def test_bad_result_is_failure(self):
        self.assertEqual(self.run_worker(STUB_MODE="bad").returncode, 65)
        run = Path((self.state / "latest").read_text().strip())
        self.assertEqual((run / "exit-code").read_text().strip(), "65")

    def test_timeout_is_not_success(self):
        self.assertEqual(self.run_worker(STUB_TIMEOUT="1").returncode, 124)
        self.assertEqual((self.state / "failures").read_text().strip(), "1")

    def test_review_wait_pauses(self):
        self.assertEqual(self.run_worker(STUB_MODE="waiting_review").returncode, 0)
        self.assertIn("waiting_review", (self.state / "STOP").read_text())
        self.assertEqual(self.run_worker().returncode, 0)
        self.assertEqual(self.calls(), 1)

    def test_external_blocker_pauses(self):
        self.assertEqual(self.run_worker(STUB_MODE="blocked").returncode, 0)
        self.assertIn("blocked", (self.state / "STOP").read_text())


if __name__ == "__main__":
    unittest.main()
