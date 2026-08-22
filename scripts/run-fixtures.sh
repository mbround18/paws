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
# `PARALLEL=8 make fixtures` or `make fixtures PARALLEL=8`) via a counting
# semaphore (a FIFO seeded with PARALLEL tokens) -- each fixture blocks
# acquiring a token before it actually starts, so "queued" is a real state,
# not just "not yet launched". A live status board redraws in place
# (queued -> spinning "running" -> done in HH:MM:SS), and full logs are
# only dumped afterward for fixtures that didn't match their expected
# pass/fail outcome.
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
count=${#fixtures[@]}

results_dir="$(mktemp -d)"
sem_fifo="$results_dir/sem"
mkfifo "$sem_fifo"
exec 3<>"$sem_fifo"
rm -f "$sem_fifo"
for ((i = 0; i < parallel; i++)); do
    echo >&3
done

cleanup() {
    exec 3>&- 2>/dev/null || true
    rm -rf "$results_dir"
}
trap cleanup EXIT

fmt_hms() {
    local s=$1
    printf '%02d:%02d:%02d' $((s / 3600)) $((s % 3600 / 60)) $((s % 60))
}

# run_one <index>: waits for a semaphore token (i.e. sits "queued" until a
# slot frees up), then runs one fixture for real, recording start/end times
# and pass/fail so the renderer and the final summary can read them back.
run_one() {
    local index="$1"
    IFS='|' read -r label dir expect args <<<"${fixtures[$index]}"
    read -r -u 3 _
    date +%s >"$results_dir/$index.start"
    # shellcheck disable=SC2086 -- args is a deliberate word-split arg list
    (cd "$repo_root/$dir" && "$paws" $args) >"$results_dir/$index.log" 2>&1
    local rc=$?
    date +%s >"$results_dir/$index.end"
    local outcome=pass
    ((rc != 0)) && outcome=fail
    echo "$label|$outcome|$expect" >"$results_dir/$index.status"
    echo >&3
}

for i in "${!fixtures[@]}"; do
    run_one "$i" &
done

# Live status board: redraws $count lines in place each tick, driven purely
# by the .start/.status files run_one writes -- no coordination with the
# background jobs beyond that, so the renderer can't get out of sync with
# what's actually running.
spinner=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏)
frame=0
first_draw=1
is_tty=0
[[ -t 1 ]] && is_tty=1

render() {
    local now; now=$(date +%s)
    if ((is_tty)) && ((!first_draw)); then
        printf '\033[%dA' "$count"
    fi
    first_draw=0
    for i in "${!fixtures[@]}"; do
        IFS='|' read -r label _ _ _ <<<"${fixtures[$i]}"
        local line
        if [[ -f "$results_dir/$i.status" ]]; then
            IFS='|' read -r _ outcome expect <"$results_dir/$i.status"
            local start end dur icon word
            start=$(<"$results_dir/$i.start")
            end=$(<"$results_dir/$i.end")
            dur=$(fmt_hms $((end - start)))
            if [[ "$outcome" == "$expect" ]]; then
                icon="✔"; word="Completed"
            else
                icon="✘"; word="FAILED"
            fi
            line="  $icon $word  $label  in $dur"
        elif [[ -f "$results_dir/$i.start" ]]; then
            local start elapsed
            start=$(<"$results_dir/$i.start")
            elapsed=$(fmt_hms $((now - start)))
            line="  ${spinner[frame % ${#spinner[@]}]} Running    $label  ($elapsed)"
        else
            line="    Queued     $label"
        fi
        if ((is_tty)); then
            printf '\033[2K%s\n' "$line"
        else
            printf '%s\n' "$line"
        fi
    done
    frame=$((frame + 1))
}

if ((is_tty)); then
    while [[ -n "$(jobs -r)" ]]; do
        render
        sleep 0.1
    done
    render
else
    # Non-interactive (e.g. CI log): just poll and print a plain snapshot
    # occasionally instead of redrawing, since there's no terminal to
    # redraw in place on.
    while [[ -n "$(jobs -r)" ]]; do
        sleep 2
    done
    render
fi
wait

pass=0
fail=0
failed=()
for i in "${!fixtures[@]}"; do
    IFS='|' read -r label outcome expect <"$results_dir/$i.status"
    if [[ "$outcome" == "$expect" ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failed+=("$i")
    fi
done

echo
echo "fixtures: $pass passed, $fail failed"
if ((fail > 0)); then
    for i in "${failed[@]}"; do
        IFS='|' read -r label _ _ <<<"${fixtures[$i]}"
        echo
        echo "----- $label -----"
        cat "$results_dir/$i.log"
    done
    exit 1
fi
