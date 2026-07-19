#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$root/scripts/verify-public-history.sh"
tmp_root="$(mktemp -d)"
daemon_pid=''
daemon_port=''
daemon_url=''
missing_tree_oid=''

cleanup() {
    local status=$?
    if [[ -n "$daemon_pid" ]] && kill -0 "$daemon_pid" 2>/dev/null; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    rm -rf "$tmp_root"
    exit "$status"
}
trap cleanup EXIT

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
}

expect_equals() {
    local actual=$1
    local expected=$2
    local label=$3
    if [[ "$actual" != "$expected" ]]; then
        fail "$label (expected $expected, got $actual)"
    fi
}

physical_object_count() {
    local repo=$1
    local loose
    local packed
    loose="$(git -C "$repo" count-objects -v | awk '$1 == "count:" { print $2 }')"
    packed="$(git -C "$repo" count-objects -v | awk '$1 == "in-pack:" { print $2 }')"
    printf '%s\n' "$(( ${loose:-0} + ${packed:-0} ))"
}

assert_promisor_clone() {
    local repo=$1
    local normal_objects=$2
    local promisor_objects

    expect_equals "$(git -C "$repo" config --get remote.origin.promisor)" true \
        'blob:none clone did not record a promisor remote'
    expect_equals "$(git -C "$repo" config --get remote.origin.partialclonefilter)" blob:none \
        'blob:none clone did not record its filter'
    if ! find "$repo/.git/objects/pack" -type f -name '*.promisor' -print -quit | grep -q .; then
        fail 'blob:none clone did not retain a promisor pack marker'
    fi
    promisor_objects="$(physical_object_count "$repo")"
    if (( promisor_objects >= normal_objects )); then
        fail "blob:none clone did not reduce physical object count (normal=$normal_objects promisor=$promisor_objects)"
    fi
}

make_source_graph() {
    local work=$1
    local served=$2

    git init -q -b main "$work"
    git -C "$work" config user.name fixture
    git -C "$work" config user.email fixture@example.invalid
    mkdir -p "$work/scripts"
    install -m 755 "$guard" "$work/scripts/verify-public-history.sh"
    printf 'public source root\n' > "$work/README.md"
    git -C "$work" add README.md scripts/verify-public-history.sh
    git -C "$work" commit -qm 'public source root'

    mkdir -p "$work/metadata"
    printf 'metadata descendant\n' > "$work/metadata/release.txt"
    git -C "$work" add metadata/release.txt
    git -C "$work" commit -qm 'metadata descendant'
    git -C "$work" tag -a v0.0.1-metadata -m 'metadata tag'

    git init -q --bare "$served"
    git -C "$served" config uploadpack.allowFilter true
    git -C "$served" symbolic-ref HEAD refs/heads/main
    git -C "$work" remote add origin "$served"
    git -C "$work" push -q origin main --tags
}

start_daemon() {
    local served_root=$1
    local attempt
    local probe_url

    for attempt in 1 2 3 4 5; do
        daemon_port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
        git daemon --reuseaddr --export-all --base-path="$served_root" --listen=127.0.0.1 \
            --port="$daemon_port" "$served_root" > "$tmp_root/git-daemon.log" 2>&1 &
        daemon_pid=$!
        probe_url="git://127.0.0.1:${daemon_port}/source.git"
        sleep 1
        if kill -0 "$daemon_pid" 2>/dev/null && git ls-remote -q "$probe_url" >/dev/null 2>&1; then
            daemon_url="$probe_url"
            return 0
        fi
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
        daemon_pid=''
    done

    fail 'unable to start a loopback git daemon'
}

clone_promisor_with_scripts() {
    local url=$1
    local destination=$2

    git clone -q --filter=blob:none --no-checkout "$url" "$destination"
    git -C "$destination" sparse-checkout init --cone
    git -C "$destination" sparse-checkout set scripts
    git -C "$destination" checkout -q main
}

make_reachable_missing_tree() {
    local repo=$1
    local blob_oid
    local tree_oid
    local commit_oid
    local parent
    local git_dir
    local tree_path

    git -C "$repo" config user.name fixture
    git -C "$repo" config user.email fixture@example.invalid
    parent="$(git -C "$repo" rev-parse HEAD)"
    blob_oid="$(printf 'fixture tree payload\n' | git -C "$repo" hash-object -w --stdin)"
    tree_oid="$(printf '100644 blob %s\tfixture.txt\n' "$blob_oid" | git -C "$repo" mktree)"
    commit_oid="$(printf 'fixture reachable missing tree\n' | git -C "$repo" commit-tree "$tree_oid" -p "$parent")"
    git -C "$repo" update-ref refs/heads/main "$commit_oid" "$parent"
    git_dir="$(git -C "$repo" rev-parse --absolute-git-dir)"
    tree_path="$git_dir/objects/${tree_oid:0:2}/${tree_oid:2}"
    if [[ ! -f "$tree_path" ]]; then
        fail 'reachable missing-tree fixture did not retain a loose tree'
    fi
    mv "$tree_path" "$tmp_root/missing-tree-${tree_oid}"
    missing_tree_oid="$tree_oid"
}

expect_guard_failure() {
    local repo=$1
    local label=$2
    local guard_failure

    if guard_failure="$(ZT_EDGE_REPO_ROOT="$repo" "$repo/scripts/verify-public-history.sh" --require-history 2>&1)"; then
        fail "history guard accepted $label"
    fi
    if [[ "$guard_failure" != 'FAIL: public SDK object database contains unreachable or invalid objects' ]]; then
        fail "$label guard failed for more than its object-database check: $guard_failure"
    fi
}

reachable_parentless_source_root_in_blob_none_promisor_clone_does_not_fail_public_history_guard_solely_for_unreachable_output() {
    local source_work="$tmp_root/source-work"
    local served_root="$tmp_root/served"
    local served="$served_root/source.git"
    local url
    local normal="$tmp_root/normal"
    local promisor="$tmp_root/promisor"
    local normal_objects
    local promisor_objects
    local normal_fsck
    local promisor_fsck
    local promisor_strict_fsck
    local guard_output
    local missing_tree_guard_output
    local unreachable_normal="$tmp_root/normal-unreachable"
    local missing_tree_promisor="$tmp_root/promisor-missing-tree"
    local normal_unreachable_fsck

    mkdir -p "$served_root"
    make_source_graph "$source_work" "$served"
    start_daemon "$served_root"
    url="$daemon_url"

    git clone -q "$url" "$normal"
    if ! normal_fsck="$(git -C "$normal" fsck --full --no-reflogs --unreachable --no-progress 2>&1)"; then
        fail "normal full-clone fsck exited nonzero: $normal_fsck"
    fi
    if [[ -n "$normal_fsck" ]]; then
        fail "normal full-clone fsck reported output: $normal_fsck"
    fi
    if ! ZT_EDGE_REPO_ROOT="$normal" "$guard" --require-history > "$tmp_root/normal-guard.out" 2>&1; then
        fail "normal full-clone guard failed: $(cat "$tmp_root/normal-guard.out")"
    fi
    if find "$normal/.git/objects/pack" -type f -name '*.promisor' -print -quit | grep -q .; then
        fail 'normal full clone unexpectedly retained a promisor pack marker'
    fi
    normal_objects="$(physical_object_count "$normal")"

    clone_promisor_with_scripts "$url" "$promisor"
    assert_promisor_clone "$promisor" "$normal_objects"
    promisor_objects="$(physical_object_count "$promisor")"
    if ! promisor_fsck="$(git -C "$promisor" fsck --full --no-reflogs --unreachable --no-progress 2>&1)"; then
        fail "valid blob:none promisor fsck exited nonzero: $promisor_fsck"
    fi
    if [[ -z "$promisor_fsck" ]]; then
        fail 'fixture did not reproduce nonempty unreachable output in a blob:none promisor clone'
    fi
    if ! promisor_strict_fsck="$(git -C "$promisor" fsck --full --strict --no-reflogs --no-progress 2>&1)"; then
        fail "valid blob:none promisor strict fsck exited nonzero: $promisor_strict_fsck"
    fi
    if [[ -n "$promisor_strict_fsck" ]]; then
        fail "valid blob:none promisor strict fsck reported output: $promisor_strict_fsck"
    fi
    if ! guard_output="$(ZT_EDGE_REPO_ROOT="$promisor" "$guard" --require-history 2>&1)"; then
        if [[ "$guard_output" != 'FAIL: public SDK object database contains unreachable or invalid objects' ]]; then
            fail "promisor guard failed for more than unreachable output: $guard_output"
        fi
        printf 'RED: normal_unreachable=<empty>\n' >&2
        printf 'RED: promisor_unreachable=%s\n' "$promisor_fsck" >&2
        printf 'RED: promisor_strict=<empty>\n' >&2
        printf 'RED: guard=%s\n' "$guard_output" >&2
        fail 'reachable_parentless_source_root_in_blob_none_promisor_clone_does_not_fail_public_history_guard_solely_for_unreachable_output'
    fi

    git clone -q "$url" "$unreachable_normal"
    printf 'deliberately unreachable object\n' | git -C "$unreachable_normal" hash-object -w --stdin >/dev/null
    if ! normal_unreachable_fsck="$(git -C "$unreachable_normal" fsck --full --no-reflogs --unreachable --no-progress 2>&1)"; then
        fail "normal unreachable-object fsck exited nonzero: $normal_unreachable_fsck"
    fi
    if [[ -z "$normal_unreachable_fsck" ]]; then
        fail 'normal unreachable-object fixture did not emit fsck diagnostics'
    fi
    expect_guard_failure "$unreachable_normal" 'normal-unreachable-object'

    clone_promisor_with_scripts "$url" "$missing_tree_promisor"
    assert_promisor_clone "$missing_tree_promisor" "$normal_objects"
    make_reachable_missing_tree "$missing_tree_promisor"
    if GIT_NO_LAZY_FETCH=1 git -C "$missing_tree_promisor" fsck --full --strict --no-reflogs --no-progress \
        > "$tmp_root/missing-tree-promisor-fsck.out" 2>&1; then
        fail 'reachable missing-tree promisor fixture unexpectedly passed strict fsck'
    fi
    if ! grep -Fq 'broken link from' "$tmp_root/missing-tree-promisor-fsck.out"; then
        fail 'reachable missing-tree fixture did not report a broken link'
    fi
    if ! grep -Fq "missing tree $missing_tree_oid" "$tmp_root/missing-tree-promisor-fsck.out"; then
        fail 'reachable missing-tree fixture did not report its missing tree'
    fi
    if missing_tree_guard_output="$(GIT_NO_LAZY_FETCH=1 ZT_EDGE_REPO_ROOT="$missing_tree_promisor" \
        "$missing_tree_promisor/scripts/verify-public-history.sh" --require-history 2>&1)"; then
        fail 'history guard accepted promisor-missing-tree'
    fi
    if [[ "$missing_tree_guard_output" != 'FAIL: public SDK object database contains unreachable or invalid objects' ]]; then
        fail "promisor missing-tree guard failed for more than strict fsck: $missing_tree_guard_output"
    fi

    printf 'PASS: promisor history normal=accepted filtered=accepted normal_unreachable=blocked promisor_missing_tree=blocked normal_objects=%s promisor_objects=%s promisor_unreachable=reported strict=clean\n' \
        "$normal_objects" "$promisor_objects"
}

reachable_parentless_source_root_in_blob_none_promisor_clone_does_not_fail_public_history_guard_solely_for_unreachable_output
