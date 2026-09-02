#!/usr/bin/env bash
# Verifies a published release actually came out whole.
#
# The release workflow's own job statuses are not sufficient evidence. Two real
# failures got past them:
#
#   v0.0.1-prerelease.39 — 4 of 7 legs failed to cross-compile; the release
#     published with 3 binaries.
#   v0.0.1-prerelease.40 — all 7 legs reported success, but two GitHub releases
#     had been created for the one tag and the assets split 6/1. Every job was
#     internally consistent; the published release was missing a platform.
#
# So this checks the *artifacts*, from the outside, the way a user would:
#
#   1. exactly one release points at the tag (a create-race leaves two)
#   2. every target `paws release` knows about has an archive attached
#   3. each binary really is the architecture its filename claims
#
# The target list comes from `paws release --list-targets`, not from a copy
# kept here — the workflow matrix and the code already disagreeing is the exact
# class of drift this repo has been removing.
#
# Usage: scripts/verify-release.sh <tag> [path-to-paws-binary]
set -uo pipefail

tag="${1:-}"
paws_bin="${2:-paws}"
repo="${GITHUB_REPOSITORY:-mbround18/paws}"

if [[ -z "$tag" ]]; then
    echo "usage: $0 <tag> [path-to-paws-binary]" >&2
    exit 2
fi
if ! command -v "$paws_bin" >/dev/null 2>&1 && [[ ! -x "$paws_bin" ]]; then
    echo "error: no usable paws binary at '$paws_bin' - pass one as \$2" >&2
    exit 2
fi

fail=0
note_failure() { echo "  FAIL  $1"; fail=1; }

echo "verify-release: $tag in $repo"

# --- 1. exactly one release for the tag ------------------------------------
echo
echo "[1/3] releases pointing at $tag"
mapfile -t release_ids < <(
    gh api "repos/$repo/releases" --paginate \
        --jq ".[] | select(.tag_name==\"$tag\") | .id"
)
case "${#release_ids[@]}" in
    1) echo "  ok    exactly one release (${release_ids[0]})" ;;
    0) note_failure "no release found for tag $tag" ;;
    *) note_failure "${#release_ids[@]} releases share this tag (${release_ids[*]}) - a
        create-race split the assets; see GitHubReleaseClient::converge_on_one_release" ;;
esac

# --- 2. every known target has an archive ----------------------------------
echo
echo "[2/3] published archives"
mapfile -t targets < <("$paws_bin" release --list-targets)
if [[ "${#targets[@]}" -eq 0 ]]; then
    echo "error: 'paws release --list-targets' returned nothing" >&2
    exit 2
fi
asset_names="$(gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name' 2>/dev/null)"
for target in "${targets[@]}"; do
    if grep -qF -- "$target" <<<"$asset_names"; then
        echo "  ok    $target"
    else
        note_failure "$target has no published archive"
    fi
done

# --- 3. each binary is the architecture its name claims --------------------
# Derived from the triple rather than a lookup table, so a new target needs no
# edit here. `file` spells the same architecture differently per binary format,
# which is why Mach-O is separated out.
expected_arch() {
    case "$1" in
        x86_64-apple-*)  echo "x86_64" ;;
        aarch64-apple-*) echo "arm64" ;;
        x86_64-*)        echo "x86-64" ;;
        aarch64-*)       echo "aarch64" ;;
        *)               echo "" ;;
    esac
}

echo
echo "[3/3] binary architectures"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
for target in "${targets[@]}"; do
    want="$(expected_arch "$target")"
    if [[ -z "$want" ]]; then
        echo "  skip  $target (no architecture expectation known)"
        continue
    fi
    if ! gh release download "$tag" --repo "$repo" \
        --pattern "*${target}.zip" --dir "$workdir/$target" >/dev/null 2>&1; then
        # Already reported as missing in step 2; don't double-count it.
        echo "  skip  $target (nothing to download)"
        continue
    fi
    unzip -qo "$workdir/$target"/*.zip -d "$workdir/$target" 2>/dev/null
    binary="$(find "$workdir/$target" -type f ! -name '*.zip' | head -1)"
    if [[ -z "$binary" ]]; then
        note_failure "$target archive contains no binary"
        continue
    fi
    described="$(file -b "$binary")"
    if grep -qF -- "$want" <<<"$described"; then
        echo "  ok    $target is $want"
    else
        note_failure "$target should be $want, but is: ${described:0:60}"
    fi
done

echo
if [[ "$fail" -eq 0 ]]; then
    echo "verify-release: $tag is complete (${#targets[@]} targets)"
else
    echo "verify-release: $tag is INCOMPLETE - see the failures above"
fi
exit "$fail"
