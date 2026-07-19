#!/usr/bin/env bash
set -euo pipefail

root="${ZT_EDGE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$root"

if [[ $# -ne 0 ]]; then
    printf 'ERROR: usage: %s\n' "$0" >&2
    exit 2
fi

if ! command -v rg >/dev/null 2>&1; then
    printf 'ERROR: verify-public-boundary.sh requires rg\n' >&2
    exit 2
fi

failures=0

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    failures=1
}

expected_crates="$(printf '%s\n' \
    rshome-actor \
    rshome-codegen \
    rshome-config \
    rshome-device-link \
    rshome-entity \
    rshome-mcp \
    rshome-native-api \
    rshome-schema \
    rshome-state \
    rshome-svc \
    rshome-wasm-host \
    rshome-wf \
    rshome-wizard | sort)"
actual_crates="$(find crates -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort)"

if [[ "$actual_crates" != "$expected_crates" ]]; then
    fail 'workspace members are not the approved 13-crate rshome source closure'
fi

allowed_root_entries="$(printf '%s\n' \
    .github .gitignore Cargo.lock Cargo.toml LICENSE README.md clippy.toml crates deny.toml docs \
    rust-toolchain.toml rustfmt.toml scripts sdk | sort)"
actual_root_entries="$(find . -mindepth 1 -maxdepth 1 ! -name .git -exec basename {} \; | sort)"
if [[ "$actual_root_entries" != "$allowed_root_entries" ]]; then
    fail 'public SDK root contains an unapproved path'
fi

if find . -path './.git' -prune -o -type l -print | grep -q .; then
    fail 'public SDK tree contains a symbolic link'
fi

private_directory_prefix="$(printf '%s%s' 'mi' 'mi')"
private_transport_dir="$(printf '%s%s' 'embedded' '-tls')"
if find . -path './.git' -prune -o -type d \( \
    -name "${private_directory_prefix}-*" -o -name "$private_transport_dir" -o -name '.prd' -o -name bench -o \
    -name deploy -o -name firmware -o -name ops -o -name test-artifacts \
\) -print | grep -q .; then
    fail 'public SDK tree contains a private runtime or operations path'
fi

if [[ ! -f Cargo.lock ]]; then
    fail 'public SDK tree is missing the controlled root Cargo.lock'
fi

if find . -path './.git' -prune -o -name Cargo.lock ! -path './Cargo.lock' -print | grep -q .; then
    fail 'public SDK tree contains an unapproved nested Cargo.lock'
fi

private_terms=(
    "$(printf '%s%s' 'mi' 'mi')"
    "$(printf '%s%s%s' 'c' 'law' 'room')"
    "$(printf '%s%s' 'bas' 'tion')"
    "$(printf '%s%s' 'tier' '-m')"
    "$(printf '%s%s' 'spif' 'fe')"
    "$(printf '%s%s' 'wire' 'guard')"
    "$(printf '%s%s' 'side' 'car')"
    "$private_transport_dir"
    "$(printf '%s%s' 'web' 'authn')"
    "$(printf '%s%s%s' 'open' 'c' 'law')"
    "$(printf '%s%s%s' 'zero' 'c' 'law')"
)
private_pattern="${private_terms[0]}"
for private_term in "${private_terms[@]:1}"; do
    private_pattern+="|${private_term}"
done
if rg -i -l -e "$private_pattern" . --hidden --glob '!.git/**' >/dev/null 2>&1; then
    fail 'public SDK tree contains a private source term'
fi

ffi_prefix="${private_directory_prefix}-$(printf '%s%s' 'side' 'car')-ffi"
if rg -n -i -e '^\[ffi\.' -e "$ffi_prefix" -e "$private_transport_dir" Cargo.toml sdk \
    --glob '*.toml' >/dev/null 2>&1; then
    fail 'public SDK manifest exposes a private native closure'
fi

if (( failures )); then
    exit 1
fi

printf 'PASS: public boundary crates=13 private_paths=0 private_terms=0 ffi=0 root=approved\n'
