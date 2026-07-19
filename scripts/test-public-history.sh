#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$root/scripts/verify-public-history.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
}

init_repo() {
    local repo=$1
    git init -q "$repo"
    git -C "$repo" config user.name 'fixture'
    git -C "$repo" config user.email 'fixture@example.invalid'
    printf 'public SDK fixture\n' > "$repo/README.md"
    git -C "$repo" add README.md
    git -C "$repo" commit -qm 'initial public SDK source'
}

expect_failure() {
    local repo=$1
    local label=$2
    if ZT_EDGE_REPO_ROOT="$repo" "$guard" --require-history >/dev/null 2>&1; then
        fail "history guard accepted $label"
    fi
}

good="$tmp_root/good"
init_repo "$good"
ZT_EDGE_REPO_ROOT="$good" "$guard" --require-history >/dev/null

old_sha="$(printf '%s%s%s' '91757ea7d7012ec3d308' 'b86ff06a6a2d' '2471e5e2')"
printf 'safe change\n' > "$good/change.txt"
git -C "$good" add change.txt
git -C "$good" commit -qm "record source $old_sha"
expect_failure "$good" 'historical public revision'

second="$tmp_root/second"
init_repo "$second"
git -C "$good" branch fixture-old-root HEAD~1
git -C "$good" fetch -q "$second" HEAD:fixture-second-root
expect_failure "$good" 'second root ref'

printf 'PASS: history guard old_revision=blocked second_root=blocked one_root=accepted\n'
