#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
template_manifest="$root/sdk/zt-edge-sdk-v0.1.0.toml"
release_manifest="$root/sdk/releases/zt-edge-sdk-v0.1.0.toml"

if [[ "${1:-}" != '--source-revision' || $# -ne 2 ]]; then
    printf 'ERROR: usage: %s --source-revision <clean-source-root-sha>\n' "$0" >&2
    exit 2
fi

source_revision=$2
if [[ ! "$source_revision" =~ ^[0-9a-f]{40}$ ]]; then
    printf 'FAIL: source revision must be a full lowercase commit SHA\n' >&2
    exit 1
fi

resolved="$(git -C "$root" rev-parse --verify "${source_revision}^{commit}" 2>/dev/null || true)"
if [[ "$resolved" != "$source_revision" ]]; then
    printf 'FAIL: source revision does not resolve locally\n' >&2
    exit 1
fi

parents="$(git -C "$root" rev-list --parents -n 1 "$source_revision")"
if [[ "$parents" != "$source_revision" ]]; then
    printf 'FAIL: source revision must be the one-root public source commit\n' >&2
    exit 1
fi

if ! git -C "$root" merge-base --is-ancestor "$source_revision" HEAD; then
    printf 'FAIL: source revision is not reachable from HEAD\n' >&2
    exit 1
fi

if ! bash "$root/scripts/verify-public-history.sh" --require-history; then
    printf 'FAIL: source history is not clean\n' >&2
    exit 1
fi

if ! bash "$root/scripts/verify-sdk-contract.sh"; then
    printf 'FAIL: SDK template is not publishable\n' >&2
    exit 1
fi

mkdir -p "$(dirname "$release_manifest")"
temporary_manifest="$(mktemp "${release_manifest}.tmp.XXXXXX")"
trap 'rm -f "$temporary_manifest"' EXIT
awk -v revision="$source_revision" '
    /^git_revision = / { print "git_revision = \"" revision "\""; next }
    /^revision_kind = / { print "revision_kind = \"immutable-git-commit\""; next }
    { print }
' "$template_manifest" > "$temporary_manifest"
mv "$temporary_manifest" "$release_manifest"
trap - EXIT

printf 'PASS: release metadata source_root=%s metadata_commit=separate\n' "$source_revision"
