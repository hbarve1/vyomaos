#!/usr/bin/env bash
# Emit the validated R1 image allowlist for Make and rootfs assembly.
set -euo pipefail
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
allowlist="$project_root/base/r1-apps.txt"
[[ -s "$allowlist" ]] || { echo "ERROR: R1 app allowlist missing or empty: $allowlist" >&2; exit 1; }
declare -A seen=()
apps=()
while IFS= read -r app || [[ -n "$app" ]]; do
    [[ -z "$app" || "$app" == \#* ]] && continue
    if [[ ! "$app" =~ ^[a-z0-9][a-z0-9-]*$ || -n "${seen[$app]:-}" ]]; then
        echo "ERROR: invalid or duplicate R1 app name: $app" >&2
        exit 1
    fi
    seen[$app]=1
    apps+=("$app")
done < "$allowlist"
((${#apps[@]})) || { echo 'ERROR: R1 app allowlist selects no apps' >&2; exit 1; }
printf '%s\n' "${apps[@]}"
