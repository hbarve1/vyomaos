#!/usr/bin/env bash
# Host or builder: install pinned Python tests into a repository-local venv.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 -m venv out/test-venv
out/test-venv/bin/python -m pip install --require-hashes -r tests/e2e/requirements.txt
