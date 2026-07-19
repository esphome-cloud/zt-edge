#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$root/scripts/verify-public-boundary.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

expected_crates=(
    rshome-schema rshome-config rshome-codegen rshome-mcp rshome-wizard
    rshome-actor rshome-entity rshome-state rshome-svc rshome-device-link
    rshome-native-api rshome-wasm-host rshome-wf
)

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
}

make_fixture() {
    local repo=$1
    local crate
    mkdir -p "$repo/.github" "$repo/crates" "$repo/docs" "$repo/scripts" "$repo/sdk"
    : > "$repo/.gitignore"
    : > "$repo/clippy.toml"
    : > "$repo/deny.toml"
    : > "$repo/rust-toolchain.toml"
    : > "$repo/rustfmt.toml"
    printf '[workspace]\nmembers = []\n' > "$repo/Cargo.toml"
    printf '# controlled release artifact\nversion = 4\n' > "$repo/Cargo.lock"
    printf 'public SDK\n' > "$repo/README.md"
    printf 'SPDX-License-Identifier: MIT OR Apache-2.0\n' > "$repo/LICENSE"
    for crate in "${expected_crates[@]}"; do
        mkdir -p "$repo/crates/$crate"
        printf '[package]\nname = "%s"\nversion = "0.1.0"\n' "$crate" \
            > "$repo/crates/$crate/Cargo.toml"
    done
}

expect_failure() {
    local repo=$1
    local label=$2
    if ZT_EDGE_REPO_ROOT="$repo" "$guard" >/dev/null 2>&1; then
        fail "boundary guard accepted $label"
    fi
}

good="$tmp_root/good"
make_fixture "$good"
ZT_EDGE_REPO_ROOT="$good" "$guard" >/dev/null

extra="$tmp_root/extra-crate"
make_fixture "$extra"
mkdir -p "$extra/crates/rshome-extra"
expect_failure "$extra" 'fourteenth crate'

forbidden="$tmp_root/forbidden-source"
make_fixture "$forbidden"
private_term="$(printf '%s%s' 'mi' 'mi')"
printf 'pub const FORBIDDEN: &str = "%s";\n' "$private_term" \
    > "$forbidden/crates/rshome-actor/private.rs"
expect_failure "$forbidden" 'private source term'

missing_lock="$tmp_root/missing-lock"
make_fixture "$missing_lock"
rm "$missing_lock/Cargo.lock"
expect_failure "$missing_lock" 'missing controlled lockfile'

printf 'PASS: boundary guard crates=13 fourteenth=blocked private_source_term=blocked lockfile=required\n'
