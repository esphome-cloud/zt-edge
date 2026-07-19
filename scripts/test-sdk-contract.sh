#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$root/scripts/verify-sdk-contract.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

packages=(
    rshome-schema rshome-config rshome-codegen rshome-mcp rshome-actor
    rshome-entity rshome-state rshome-svc rshome-wizard
)

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
}

make_manifest() {
    local repo=$1
    local package
    mkdir -p "$repo/sdk/releases"
    {
        printf 'schema_version = 1\nrelease_line = "v0.1"\n'
        printf 'git_url = "https://github.com/esphome-cloud/zt-edge.git"\n'
        printf 'git_revision = "UNPUBLISHED"\nrevision_kind = "unpublished"\n'
        for package in "${packages[@]}"; do
            printf '\n[[rust_crate]]\npackage = "%s"\n' "$package"
        done
    } > "$repo/sdk/zt-edge-sdk-v0.1.0.toml"
}

expect_failure() {
    local repo=$1
    local label=$2
    if ZT_EDGE_REPO_ROOT="$repo" "$guard" >/dev/null 2>&1; then
        fail "SDK contract guard accepted $label"
    fi
}

good="$tmp_root/good"
make_manifest "$good"
ZT_EDGE_REPO_ROOT="$good" "$guard" >/dev/null

ffi="$tmp_root/ffi"
make_manifest "$ffi"
ffi_name="$(printf '%s%s%s%s' 'mi' 'mi-' 'side' 'car-ffi')"
printf '\n[ffi."%s"]\nstatic_library = "forbidden.a"\n' "$ffi_name" \
    >> "$ffi/sdk/zt-edge-sdk-v0.1.0.toml"
expect_failure "$ffi" 'native FFI entry'

printf 'PASS: SDK contract crates=9 FFI=blocked\n'
