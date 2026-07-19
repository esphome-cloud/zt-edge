#!/usr/bin/env bash
set -euo pipefail

root="${ZT_EDGE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
template_manifest="$root/sdk/zt-edge-sdk-v0.1.0.toml"
release_manifest="$root/sdk/releases/zt-edge-sdk-v0.1.0.toml"
require_published=0

if [[ "${1:-}" == "--require-published" && $# -eq 1 ]]; then
    require_published=1
elif [[ $# -ne 0 ]]; then
    printf 'ERROR: usage: %s [--require-published]\n' "$0" >&2
    exit 2
fi

manifest="$template_manifest"
if (( require_published )); then
    manifest="$release_manifest"
fi

if [[ ! -f "$manifest" ]]; then
    printf 'FAIL: SDK manifest is missing\n' >&2
    exit 1
fi

expected_packages=(
    rshome-schema rshome-config rshome-codegen rshome-mcp rshome-actor
    rshome-entity rshome-state rshome-svc rshome-wizard
)

package_count="$(rg -c '^\[\[rust_crate\]\]$' "$manifest")"
if [[ "$package_count" != "9" ]]; then
    printf 'FAIL: SDK manifest must expose exactly nine Rust crate entries\n' >&2
    exit 1
fi

for package in "${expected_packages[@]}"; do
    matches="$(rg -Fxc "package = \"$package\"" "$manifest" || true)"
    if [[ "$matches" != "1" ]]; then
        printf 'FAIL: SDK manifest must expose %s exactly once\n' "$package" >&2
        exit 1
    fi
done

if ! rg -Fxq 'git_url = "https://github.com/esphome-cloud/zt-edge.git"' "$manifest"; then
    printf 'FAIL: SDK manifest does not declare the canonical public Git URL\n' >&2
    exit 1
fi

private_prefix="$(printf '%s%s' 'mi' 'mi')"
private_transport="$(printf '%s%s' 'embedded' '-tls')"
ffi_prefix="${private_prefix}-$(printf '%s%s' 'side' 'car')-ffi"
if rg -n -i -e '^\[ffi\.' -e "$ffi_prefix" -e "$private_transport" "$manifest" >/dev/null 2>&1; then
    printf 'FAIL: SDK manifest exposes a private native entry\n' >&2
    exit 1
fi

if rg -n 'path\s*=' "$manifest" >/dev/null 2>&1; then
    printf 'FAIL: SDK manifest contains a cross-repository path dependency\n' >&2
    exit 1
fi

revision="$(awk -F'"' '/^git_revision = / { print $2 }' "$manifest")"
revision_kind="$(awk -F'"' '/^revision_kind = / { print $2 }' "$manifest")"

if (( ! require_published )); then
    if [[ "$revision" != 'UNPUBLISHED' || "$revision_kind" != 'unpublished' ]]; then
        printf 'FAIL: SDK template must remain unpublished\n' >&2
        exit 1
    fi
    printf 'PASS: SDK contract crates=9 ffi=0 revision=UNPUBLISHED\n'
    exit 0
fi

if [[ ! "$revision" =~ ^[0-9a-f]{40}$ || "$revision_kind" != 'immutable-git-commit' ]]; then
    printf 'FAIL: published SDK record must declare an immutable Git revision\n' >&2
    exit 1
fi

resolved="$(git -C "$root" rev-parse --verify "${revision}^{commit}" 2>/dev/null || true)"
if [[ "$resolved" != "$revision" ]]; then
    printf 'FAIL: SDK release revision does not resolve locally\n' >&2
    exit 1
fi

parents="$(git -C "$root" rev-list --parents -n 1 "$revision")"
if [[ "$parents" != "$revision" ]]; then
    printf 'FAIL: SDK release revision must be the clean source-root commit\n' >&2
    exit 1
fi

if ! git -C "$root" merge-base --is-ancestor "$revision" HEAD; then
    printf 'FAIL: SDK release revision is not reachable from HEAD\n' >&2
    exit 1
fi

printf 'PASS: SDK contract crates=9 ffi=0 revision=%s source_root=1\n' "$revision"
