#!/usr/bin/env bash
set -euo pipefail

root="${ZT_EDGE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$root"

if [[ "${1:-}" != "--require-history" || $# -ne 1 ]]; then
    printf 'ERROR: usage: %s --require-history\n' "$0" >&2
    exit 2
fi

if ! git rev-parse --verify HEAD >/dev/null 2>&1; then
    printf 'FAIL: public SDK history is required\n' >&2
    exit 1
fi

failures=0

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    failures=1
}

old_revisions=(
    "$(printf '%s%s%s' '91757ea7d7012ec3d308' 'b86ff06a6a2d' '2471e5e2')"
    "$(printf '%s%s%s' '83eebe42752279c4bb027' '49cd4e16' '23b656dd706')"
    "$(printf '%s%s%s' 'da0abd0d9103f660d9ca' '0b7920c8c2d' '2b6407409')"
    "$(printf '%s%s%s' '4a5bb9040528cb3d5374' '24' '73e0804987c29d3c71')"
    "$(printf '%s%s%s' '547dc846cb65a76a7b07' '7a4687612b9' 'da53350e0')"
)

root_count="$(git rev-list --all --max-parents=0 | wc -l | tr -d ' ')"
if [[ "$root_count" != "1" ]]; then
    fail "public SDK history must have exactly one root, found $root_count"
fi

while IFS= read -r revision; do
    if git log --all --format=%B | grep -Fqi -- "$revision"; then
        fail 'historical source revision appears in a commit message'
    fi
    if git grep --text -i -l -F -e "$revision" $(git rev-list --all) -- >/dev/null 2>&1; then
        fail 'historical source revision appears in a committed tree'
    fi
    if git show-ref --head | grep -Fqi -- "$revision"; then
        fail 'historical source revision appears in a ref'
    fi
done < <(printf '%s\n' "${old_revisions[@]}")

promisor_blob_none=0
while IFS= read -r remote; do
    if [[ "$(git config --bool "remote.${remote}.promisor" 2>/dev/null || true)" == true && \
        "$(git config --get "remote.${remote}.partialclonefilter" 2>/dev/null || true)" == blob:none ]]; then
        promisor_blob_none=1
        break
    fi
done < <(git remote)

if (( promisor_blob_none )); then
    if ! fsck_output="$(git fsck --full --strict --no-reflogs --no-progress 2>&1)"; then
        fail 'public SDK object database contains unreachable or invalid objects'
    elif [[ -n "$fsck_output" ]]; then
        fail 'public SDK object database contains unreachable or invalid objects'
    fi
else
    fsck_output="$(git fsck --full --no-reflogs --unreachable --no-progress 2>&1 || true)"
    if [[ -n "$fsck_output" ]]; then
        fail 'public SDK object database contains unreachable or invalid objects'
    fi
fi

if (( failures )); then
    exit 1
fi

printf 'PASS: public history roots=1 legacy_revisions=0 unreachable_objects=0\n'
