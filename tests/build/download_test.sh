#!/usr/bin/env bash
# Verify real download/cache paths with local fixtures, including bad content.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../../base/config.sh"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
printf 'verified fixture\n' > "$test_dir/source"
digest="$(sha256sum "$test_dir/source" | cut -d ' ' -f1)"
download_file "file://$test_dir/source" "$test_dir/cache/archive" fixture "$digest"
cmp "$test_dir/source" "$test_dir/cache/archive"
# Valid cache works without the original source.
download_file "file://$test_dir/missing" "$test_dir/cache/archive" fixture "$digest"
printf 'corruption\n' > "$test_dir/cache/archive"
if download_file "file://$test_dir/source" "$test_dir/cache/archive" fixture "$digest"; then
    echo 'FAIL: corrupt cached download accepted' >&2; exit 1
fi
if download_file "file://$test_dir/source" "$test_dir/new" fixture "$(printf '0%.0s' {1..64})"; then
    echo 'FAIL: wrong download digest accepted' >&2; exit 1
fi
[[ ! -e "$test_dir/new" ]]
if download_file "file://$test_dir/missing" "$test_dir/failed" fixture "$digest"; then
    echo 'FAIL: failed transfer accepted' >&2; exit 1
fi
[[ ! -e "$test_dir/failed" ]]
[[ -z "$(find "$test_dir" -name '*.tmp.*' -print)" ]]
echo 'PASS: verified downloads, valid cache, corrupt cache, wrong digest, failed transfer, temporary cleanup'
