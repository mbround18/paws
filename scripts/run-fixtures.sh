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
    "go-fixture|examples/go-fixture|pass|ci --toolchain go --silent"
    "go-wasm-fixture|examples/go-wasm-fixture|pass|ci --toolchain go --silent"
    "go-cgo-fixture|examples/go-cgo-fixture|pass|ci --toolchain go --silent"
    "go-react-fixture (go)|examples/go-react-fixture|pass|ci --toolchain go --silent"
    "go-react-fixture (node)|examples/go-react-fixture/frontend|pass|ci --toolchain node --silent"
    "java-maven-fixture|examples/java-maven-fixture|pass|ci --toolchain java --silent"
    "java-gradle-fixture|examples/java-gradle-fixture|pass|ci --toolchain java --silent"
    "java-react-fixture (java)|examples/java-react-fixture|pass|ci --toolchain java --silent"
    "java-react-fixture (node)|examples/java-react-fixture/frontend|pass|ci --toolchain node --silent"
    "tauri-fixture|examples/tauri-fixture|pass|ci --toolchain tauri --silent"
    "tauri-react-fixture|examples/tauri-react-fixture|pass|ci --toolchain tauri --silent"
    "multi-ecosystem-fixture (rust)|examples/multi-ecosystem-fixture|pass|ci --toolchain rust --silent"
    "multi-ecosystem-fixture (node)|examples/multi-ecosystem-fixture|pass|ci --toolchain node --silent"
    "rust-react-fixture (rust)|examples/rust-react-fixture|pass|ci --toolchain rust --silent"
    "rust-react-fixture (node)|examples/rust-react-fixture/frontend|pass|ci --toolchain node --silent"
    "docker-fixture|examples/docker-fixture|pass|docker --image paws-fixtures/docker-fixture"
    "docker-compose-fixture|examples/docker-compose-fixture|pass|docker --image ghcr.io/example/app"
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

is_tty=0
[[ -t 1 ]] && is_tty=1

interrupted=0
supervisor_pids=()

# cleanup: runs on normal exit *and* on Ctrl-C/SIGTERM. On interrupt it
# first terminates every in-flight fixture -- each one's real work
# (`paws`, and whatever `dagger` subprocess it spawns) runs under `setsid`
# specifically so it's the leader of its own process group, letting us
# `kill` that whole group by PID rather than just the wrapper shell,
# leaving orphaned dagger builds running in the background.
cleanup() {
    local rc=$?
    if ((interrupted)); then
        echo
        echo "interrupted -- stopping in-flight fixtures..."
        for i in "${!fixtures[@]}"; do
            [[ -f "$results_dir/$i.pid" ]] || continue
            local pgid; pgid=$(<"$results_dir/$i.pid")
            kill -TERM -- "-$pgid" 2>/dev/null || true
        done
        for pid in "${supervisor_pids[@]}"; do
            kill -TERM "$pid" 2>/dev/null || true
        done
        # Grace period, then escalate to SIGKILL for anything that ignored it.
        local waited=0
        while ((waited < 20)) && jobs -r >/dev/null 2>&1 && [[ -n "$(jobs -r)" ]]; do
            sleep 0.1
            waited=$((waited + 1))
        done
        for i in "${!fixtures[@]}"; do
            [[ -f "$results_dir/$i.pid" ]] || continue
            local pgid; pgid=$(<"$results_dir/$i.pid")
            kill -KILL -- "-$pgid" 2>/dev/null || true
        done
        for pid in "${supervisor_pids[@]}"; do
            kill -KILL "$pid" 2>/dev/null || true
        done
        wait 2>/dev/null || true
    fi
    ((is_tty)) && printf '\033[?25h'
    exec 3>&- 2>/dev/null || true
    rm -rf "$results_dir"
    ((interrupted)) && exit 130
    exit "$rc"
}
trap cleanup EXIT
trap 'interrupted=1; exit 130' INT TERM

fmt_hms() {
    local s=$1
    printf '%02d:%02d:%02d' $((s / 3600)) $((s % 3600 / 60)) $((s % 60))
}

# run_one <index>: waits for a semaphore token (i.e. sits "queued" until a
# slot frees up), then runs one fixture for real, recording start/end times
# and pass/fail so the renderer and the final summary can read them back.
# The real work runs under `setsid --wait` so it's the leader of its own
# process group -- `cleanup` can kill that whole group by PID on interrupt
# (see above) -- while `--wait` still forwards the real exit code back to
# us. The group leader's PID is captured by having it write its own `$$` to
# a pidfile (rather than trusting bash's `$!` for the backgrounded `setsid`
# invocation), since `setsid` forks internally -- and so gets a *different*
# PID than the program it starts -- whenever the caller already happens to
# be a process group leader; confirmed for real, this really happens for a
# script backgrounded under its own session (e.g. this script re-launched
# via `setsid script.sh &` for testing).
run_one() {
    # Each run_one instance is its own forked subshell (from `run_one &`)
    # and inherits the INT/TERM trap set below on the main script. Left
    # alone, a Ctrl-C delivered to the whole process group would make every
    # worker race to run cleanup() concurrently -- confirmed for real, this
    # causes a lost-kill race where the first worker to finish removes
    # results_dir before slower workers read their fixture's pidfile,
    # leaving that fixture's paws/dagger processes orphaned. Resetting to
    # the *default* disposition (not ignoring -- an explicit `kill -TERM`
    # from the main script's own cleanup() must still work) means a
    # worker just dies immediately with no custom trap code racing;
    # cleanup() is the sole thing that ever kills a fixture's setsid group.
    trap - INT TERM
    local index="$1"
    IFS='|' read -r label dir expect args <<<"${fixtures[$index]}"
    read -r -u 3 _
    date +%s >"$results_dir/$index.start"
    cd "$repo_root/$dir" || return
    local -a arg_array
    read -r -a arg_array <<<"$args"
    setsid --wait bash -c 'pidfile=$1; shift; echo $$ >"$pidfile"; exec "$@"' \
        _ "$results_dir/$index.pid" "$paws" "${arg_array[@]}" \
        >"$results_dir/$index.log" 2>&1 &
    wait "$!"
    local rc=$?
    date +%s >"$results_dir/$index.end"
    local outcome=pass
    ((rc != 0)) && outcome=fail
    echo "$label|$outcome|$expect" >"$results_dir/$index.status"
    echo >&3
}

for i in "${!fixtures[@]}"; do
    run_one "$i" &
    supervisor_pids+=("$!")
done

# Live status board: redraws $count lines in place each tick, driven purely
# by the .start/.status files run_one writes -- no coordination with the
# background jobs beyond that, so the renderer can't get out of sync with
# what's actually running.
spinner=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏)
frame=0
first_draw=1
((is_tty)) && printf '\033[?25l'

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
