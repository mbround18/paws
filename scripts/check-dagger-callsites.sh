#!/usr/bin/env bash
# SC-004: `dagger` must be spawned from exactly one place, crates/paws-dagger.
# Also enforced here per docs/adr/0001 (paws-release's build/smoke-test path
# routes through `dagger core`, not `docker`/`cross` directly): those two
# must not be spawned outside crates/paws-dagger either, with one deliberate
# exception — crates/paws-docker's e2e test suite shells to `docker` on
# purpose, to validate paws-docker's own facts-resolution logic against a
# real Docker daemon; that's testing paws, not paws executing a pipeline, so
# it's excluded rather than routed through paws-dagger.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

dagger_matches="$(grep -rn --include='*.rs' -E 'Command::new\(\s*"dagger"\s*\)' crates \
    | grep -v '^crates/paws-dagger/' || true)"

docker_cross_matches="$(grep -rn --include='*.rs' -E 'Command::new\(\s*"(docker|cross)"\s*\)' crates \
    | grep -v '^crates/paws-dagger/' \
    | grep -v '^crates/paws-docker/tests/' || true)"

if [[ -n "$dagger_matches" ]]; then
    echo "SC-004 violation: found dagger spawn call sites outside crates/paws-dagger:" >&2
    echo "$dagger_matches" >&2
    exit 1
fi

if [[ -n "$docker_cross_matches" ]]; then
    echo "ADR-0001 violation: found docker/cross spawn call sites outside crates/paws-dagger:" >&2
    echo "$docker_cross_matches" >&2
    exit 1
fi

echo "OK: no dagger/docker/cross spawn call sites outside crates/paws-dagger (except paws-docker's e2e tests)"
