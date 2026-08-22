#!/usr/bin/env bash
# Runs `paws ci`/`paws docker` for real (through Dagger) against every
# fixture under examples/ -- the full local gauntlet `make fixtures` calls
# into. CI's ci-e2e job (.github/workflows/ci.yaml) only exercises three of
# these (rust-fixture, node-fixture, python-fixture); this script covers
# every fixture docs/ROADMAP.md and examples/README.md claim are verified
# for real, so a regression in any of them is caught locally before it's
# caught in CI.
#
# Fixtures run PARALLEL-at-a-time (default 4, override with
# `PARALLEL=8 make fixtures` or `make fixtures PARALLEL=8`): each is a
# separate `paws` process talking to the one shared Dagger engine, so
# concurrent runs are safe -- Dagger itself is what serializes/parallelizes
# the actual container work. Each job's output is buffered to its own log
# and printed only once it finishes, in original list order, so concurrent
# runs don't interleave into unreadable output.
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

paws="${PAWS_BIN:-$repo_root/target/release/paws}"
if [[ ! -x "$paws" ]]; then
    echo "error: $paws not found or not executable -- run 'make release' first" >&2
    exit 1
fi

parallel="${PARALLEL:-4}"

# label|dir|expect (pass|fail)|paws args
fixtures=(
    "rust-fixture|examples/rust-fixture|pass|ci --toolchain rust --silent"
    "node-fixture|examples/node-fixture|pass|ci --toolchain node --silent"
    "node-fixture-npm|examples/node-fixture-npm|pass|ci --toolchain node --silent"
    "node-fixture-yarn|examples/node-fixture-yarn|pass|ci --toolchain node --silent"
    "node-fixture-bun|examples/node-fixture-bun|pass|ci --toolchain node --silent"
    "node-fixture-with-lint-failure|examples/node-fixture-with-lint-failure|fail|ci --toolchain node --silent"
    "node-server-fixture|examples/node-server-fixture|pass|ci --toolchain node --silent"
    "vite-fixture|examples/vite-fixture|pass|ci --toolchain node --silent"
    "react-vite-fixture|examples/react-vite-fixture|pass|ci --toolchain node --silent"
    "next-fixture|examples/next-fixture|pass|ci --toolchain node --silent"
    "playwright-fixture|examples/playwright-fixture|pass|ci --toolchain node --silent"
    "python-fixture|examples/python-fixture|pass|ci --toolchain python --silent"
    "tauri-fixture|examples/tauri-fixture|pass|ci --toolchain tauri --silent"
    "tauri-react-fixture|examples/tauri-react-fixture|pass|ci --toolchain tauri --silent"
    "multi-ecosystem-fixture (rust)|examples/multi-ecosystem-fixture|pass|ci --toolchain rust --silent"
    "multi-ecosystem-fixture (node)|examples/multi-ecosystem-fixture|pass|ci --toolchain node --silent"
    "rust-react-fixture (rust)|examples/rust-react-fixture|pass|ci --toolchain rust --silent"
    "rust-react-fixture (node)|examples/rust-react-fixture/frontend|pass|ci --toolchain node --silent"
    "docker-fixture|examples/docker-fixture|pass|docker --image paws-fixtures/docker-fixture"
    "docker-compose-fixture|examples/docker-compose-fixture|pass|docker --image paws-fixtures/docker-compose-fixture"
    "docker-buildkit-fixture|examples/docker-buildkit-fixture|pass|docker --image paws-fixtures/docker-buildkit-fixture"
)

results_dir="$(mktemp -d)"
trap 'rm -rf "$results_dir"' EXIT

# run_one <index> <spec>: executes one fixture, writes its outcome + log to
# results_dir/<index>.status / .log. Runs in a background subshell.
run_one() {
    local index="$1" spec="$2"
    IFS='|' read -r label dir expect args <<<"$spec"
    local log="$results_dir/$index.log"
    {
        echo "==> $label ($dir): paws $args"
        # shellcheck disable=SC2086 -- args is a deliberate word-split arg list
        if (cd "$repo_root/$dir" && "$paws" $args); then
            outcome=pass
        else
            outcome=fail
        fi
        if [[ "$outcome" == "$expect" ]]; then
            echo "    ok ($outcome, expected $expect)"
        else
            echo "    FAILED: got $outcome, expected $expect"
        fi
    } >"$log" 2>&1
    echo "$label|$outcome|$expect" >"$results_dir/$index.status"
}

echo "running ${#fixtures[@]} fixtures, $parallel at a time..."
echo

running=0
for i in "${!fixtures[@]}"; do
    run_one "$i" "${fixtures[$i]}" &
    running=$((running + 1))
    if ((running >= parallel)); then
        wait -n
        running=$((running - 1))
    fi
done
wait

pass=0
fail=0
failed=()
for i in "${!fixtures[@]}"; do
    cat "$results_dir/$i.log"
    IFS='|' read -r label outcome expect <"$results_dir/$i.status"
    if [[ "$outcome" == "$expect" ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failed+=("$label")
    fi
done

echo
echo "fixtures: $pass passed, $fail failed"
if ((fail > 0)); then
    printf '  - %s\n' "${failed[@]}"
    exit 1
fi
