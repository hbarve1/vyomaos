#!/usr/bin/env bash
# One bounded local Codex cycle. The timer is installed/enabled separately.
set -euo pipefail
umask 077
control_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${VYOMA_REPO:?Set the absolute repository path}"
: "${VYOMA_MAX_RUNS_PER_DAY:?Set an explicit daily run limit}"
: "${VYOMA_RUN_MINUTES:?Set an explicit per-run time limit}"
[[ "$VYOMA_REPO" = /* && -d "$VYOMA_REPO/.git" ]] || { echo 'Repository must be the main checkout' >&2; exit 78; }
[[ "$VYOMA_MAX_RUNS_PER_DAY" =~ ^[1-9][0-9]*$ && "$VYOMA_MAX_RUNS_PER_DAY" -le 48 ]] || exit 78
[[ "$VYOMA_RUN_MINUTES" =~ ^[1-9][0-9]*$ && "$VYOMA_RUN_MINUTES" -le 45 ]] || exit 78
state="${VYOMA_STATE_DIR:-$VYOMA_REPO/out/codex-worker}"
codex_bin="${VYOMA_CODEX_BIN:-codex}"
mkdir -p "$state"
exec 9>"$state/worker.lock"
flock -n 9 || { echo 'Another worker owns the lock'; exit 0; }
[[ ! -e "$state/STOP" ]] || { echo 'Paused: STOP file exists'; exit 0; }
day="$(date -u +%F)"
counter="$state/runs-$day"
count=0
[[ ! -e "$counter" ]] || read -r count < "$counter"
[[ "$count" =~ ^[0-9]+$ ]] || { echo 'Invalid daily counter; refusing to reset it' >&2; exit 78; }
if (( count >= VYOMA_MAX_RUNS_PER_DAY )); then
    echo "Daily run limit reached ($count); next budget day is UTC"
    exit 0
fi
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="$state/$run_id"
mkdir "$run_dir"
# Reserve before launching: interrupted/failed runs also consume a slot.
printf '%s\n' "$((count + 1))" > "$counter.tmp"
mv "$counter.tmp" "$counter"
printf '%s\n' "$run_dir" > "$state/latest"
export VYOMA_RUN_ID="$run_id"
cleanup() {
    # Containers outlive their Docker client; remove only this run's labels.
    if command -v docker >/dev/null 2>&1; then
        local containers=()
        mapfile -t containers < <(docker ps -aq --filter "label=vyomaos.worker-run=$run_id" 2>/dev/null)
        if (( ${#containers[@]} )); then
            timeout 30s docker rm -f "${containers[@]}" >> "$run_dir/cleanup.log" 2>&1 || true
        fi
    fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
export VYOMA_RUN_DEADLINE_UTC="$(date -u -d "+$VYOMA_RUN_MINUTES minutes" +%FT%TZ)"
cp "$control_root/ops/codex/worker-prompt.md" "$run_dir/prompt.md"
printf '\nRun ID: %s\nDeadline (UTC): %s\n' "$run_id" "$VYOMA_RUN_DEADLINE_UTC" >> "$run_dir/prompt.md"
set +e
timeout --signal=TERM --kill-after=30s "${VYOMA_RUN_MINUTES}m" \
  "$codex_bin" exec --approve-for-me --json \
  --output-schema "$control_root/ops/codex/result.schema.json" \
  --output-last-message "$run_dir/result.json" \
  -C "$VYOMA_REPO" - < "$run_dir/prompt.md" \
  > "$run_dir/events.jsonl" 2> "$run_dir/stderr.log"
code=$?
set -e
printf '%s\n' "$code" > "$run_dir/exit-code"
failures=0
[[ ! -e "$state/failures" ]] || read -r failures < "$state/failures"
[[ "$failures" =~ ^[0-9]+$ ]] || { echo 'Invalid failure counter' >&2; exit 78; }
if [[ "$code" -eq 0 ]]; then
    if ! status="$(python3 - "$run_dir/result.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
status = data["status"]
assert status in ("progress", "waiting_review", "blocked")
print(status)
PY
    )"; then
        code=65
    fi
fi
if [[ "$code" -ne 0 ]]; then
    printf '%s\n' "$code" > "$run_dir/exit-code"
    failures=$((failures + 1))
    printf '%s\n' "$failures" > "$state/failures"
    if (( failures >= 3 )); then
        printf 'Three failed worker invocations; inspect %s\n' "$run_dir" > "$state/STOP"
    fi
    echo "Worker failed ($code): $run_dir" >&2
    exit "$code"
fi
printf '0\n' > "$state/failures"
if [[ "$status" != progress ]]; then
    printf '%s: inspect %s before resuming\n' "$status" "$run_dir" > "$state/STOP"
fi
echo "Worker $status: $run_dir"
