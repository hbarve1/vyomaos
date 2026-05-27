#!/bin/sh
# rootfs.sh — x86-64 desktop platform
# Delegates to base/rootfs.sh until platform migration is complete.
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
exec "$REPO_ROOT/base/rootfs.sh" "$@"
