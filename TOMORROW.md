## 10. Session 7 (2026-08-19, same day): paws audit drops gh-reusable — the goal is done

Picked up right where session 6 left off: "audit queued" is now done.
`paws audit`'s scanner orchestration (`semgrep`/`gitleaks` — the only two
`gh-reusable`'s `audit` function ever ran) is now native `crates/paws-audit`
logic executed through `paws-dagger::core`, not a delegated TS pipeline
call. `audit-mcp` (a separate repo/tool the user owns, 95+ scanners via
`bollard`) was used only as *catalog-shape* inspiration — its raw-Docker
execution engine conflicts with ADR-0001 (route everything through
`dagger core`), so it was never reused directly; scope stayed narrow,
matching `gh-reusable`'s existing semgrep+gitleaks-only coverage rather
than porting the full 95+ scanner catalog.

**Shipped**: `scanner_script()` in `paws-audit` is a byte-for-byte port of
`gh-reusable`'s `runSemgrepScanner`/`runGitleaksScanner` shell scripts
(same `SEMGREP_CONFIG=auto`, same empty-result fallback JSON on a clean
scan). `scanner_json_pipeline_args`/`scanner_exit_code_pipeline_args` build
the two `dagger core` chains (JSON-report path and exit-code path) sharing
an identical prefix so the second replays from Dagger's own cache. All the
existing pure logic (`select_audit_scanners`, `parse_scanner_findings`,
`normalize_scanner_status`, `aggregate_audit_results`,
`derive_overall_status`, `render_audit_intelligence_section`) was already
built in an earlier session and slotted in unchanged. `crates/paws-cli`'s
`Audit` handler was rewritten to loop scanners, run both pipelines per
scanner, and aggregate — replacing the old `call_pipeline_report("audit",
...)` delegation entirely. `pipeline_report_succeeded`,
`call_pipeline_report`, and `GH_REUSABLE_DAGGER_MODULE` are now fully
deleted from `paws-cli` — **zero references to `gh-reusable` remain
anywhere in `paws`'s runtime code.**

**Two real bugs caught, not just theorized**, both in `dagger core`'s
`--args` CSV parsing (previously only ever used for short, plain
arguments — this is the first time paws needed to pass a real multi-line
shell script through it):
1. Embedded raw newlines inside one comma-segment get silently truncated
   (`sh -c 'echo line1\necho line2'` passed via `--args` only ever printed
   `line1`).
2. Embedded literal double-quotes break the CSV parser outright (`bare "
   in non-quoted-field`) — this one only surfaced once the JSON fallback
   content (`{"results":[]}`) was added to the semgrep script.

Fixed definitively the same way for both: never inline a script into
`--args` at all — `with-new-file --path=/scan.sh --contents=<script>` +
`with-exec --args=sh,/scan.sh` instead. Verified for real against
`returntocorp/semgrep:1.81.0` and `zricethezav/gitleaks:v8.24.2` with a
genuine git-repo fixture containing real findings in both scanners —
confirmed correct JSON parsing, correct `AuditOverallStatus` derivation
(`Findings`, not `Failed`), and correct process exit code 0 (findings are
non-fatal by design, only a scanner erroring makes `paws audit` fail).
Root-caused one fixture false alarm along the way: gitleaks reported
"Failed" until the fixture directory was made a real git repo (`git init`
+ commit) — `gh-reusable`'s exact gitleaks command has no `--no-git` flag,
so it requires one; not a paws bug.

8 new/total passing unit tests in `paws-audit` (JSON pipeline shape, no
`SEMGREP_CONFIG` env leaking into the gitleaks pipeline, exit-code pipeline
sharing the JSON pipeline's prefix, a regression guard against a raw
newline ever landing inside one `--args` token again). Full workspace
(`cargo build/test/clippy/fmt`) green. `docs/DEVELOPMENT.md`,
`docs/ROADMAP.md`, and the `GH_REUSABLE_DAGGER_MODULE` mention in
`docs/adr/0001-route-container-execution-through-dagger.md` all updated to
reflect that the dependency is gone, not just pinned.

**Still open**: nothing left on the "drop gh-reusable entirely" goal
itself — it's done. Everything else queued from earlier sessions (seven
ready-to-convert repos, `game-server-management`'s `paws publish` gap,
`ark-manager-web`'s `release.yml` rewrite, rechecking `ark-manager-web`'s
docker publishing post-registries-fix) is unchanged and still open.

## 9. Session 6 (2026-08-19, same day): paws docker drops gh-reusable entirely

User's real goal stated directly: drop the `gh-reusable` dependency and
build `paws` truly independently. Checked scope first — only 2 remaining
`call_pipeline_report` call sites in `crates/paws-cli/src/main.rs`:
`docker-release` (`paws docker`) and `audit` (`paws audit`). Everything
else (every `paws ci` toolchain, `paws release`, `paws semver`, `paws
helm`) was already fully native. User's call: docker now (it was already
90% there given this session's native-registry work), audit queued (a
different, bigger shape of problem — real scanner orchestration, not
build+publish).

**Shipped**: `paws docker` no longer calls `dockerRelease` (`gh-reusable`'s
Dagger Function) at all. docker.io/ghcr.io now publish through the exact
same native `Container.withRegistryAuth`/`Container.publish` path already
built for arbitrary registries — one unified per-registry loop instead of
"docker.io/ghcr.io via gh-reusable, everything else native". Two real
things that had to be gotten right, not just mechanically swapped:
1. `dockerRelease` used to always build (validate) the Dockerfile even on
   a PR/build-only run, regardless of whether it was about to push. The
   native path would have silently dropped that validation entirely on
   push=false — added `build_only_pipeline_args` (`docker-build` + `sync`,
   no registry involved) specifically to preserve it.
2. docker.io's own tags have no registry-hostname prefix at all (unlike
   every other registry) — `docker_hub_tags` finds them by elimination
   against whatever's in `--registries`, not by a `"docker.io/"` prefix
   that doesn't exist. Has a real regression test for the trap here: an
   `org/repo:tag` (namespaced Docker Hub image, has a `/` in it) must not
   get mistaken for a registry-prefixed reference.

Credential-missing behavior deliberately differs by registry: docker.io/
ghcr.io gracefully skip (matches `dockerRelease`'s old behavior, doesn't
break existing repos with partial credential setups); an explicit
`--registries` entry with no `--registry-username` still fails loudly
(same reasoning as the registries-CSV bug fixed earlier this session — a
registry silently not getting published to is the exact failure mode
worth erring loud about).

Verified for real, end to end, through the actual `paws docker` CLI
(not just the underlying `dagger` calls) against `steamcmd-bases`' real
Dockerfile: build-only now genuinely builds (confirmed non-instant, real
trace) instead of the earlier suspicious ~5s no-op; a forced `--push` run
against a live local registry correctly built, attached auth, and
attempted publish with the right tag — blocked only by the test
registry's lack of TLS, the same known limitation as the native-registry
work earlier this session, not a code issue. Released as
`v0.0.1-prerelease.21`. `docs/DEVELOPMENT.md`/`docs/ROADMAP.md` updated —
`paws audit` is now the *only* subcommand depending on `gh-reusable` at
all.

**Still open**: `paws audit`'s native port (semgrep/gitleaks scanner
orchestration through Dagger directly) — the last piece of the "drop
gh-reusable entirely" goal, queued, not started.

## 8. Session 5 (2026-08-19, same day): steamcmd-bases + a real registries bug

Converted `mbround18/steamcmd-bases`' `deployer.yaml` (the last of the 7
"ready-to-convert" gh-reusable repos actually started) from
`gh-reusable`'s `docker-release.yaml`/`tagger.yaml` to `paws docker`
(3-target matrix: base/wine/proton)/`paws semver`. PR:
https://github.com/mbround18/steamcmd-bases/pull/15 — **fully green**
(real ~1min builds per target, not a lazy no-op), ready for the user's
merge. `test-compatibility.yml` (not gh-reusable-based, wants
always-build-never-push regardless of event) deliberately left untouched.

**Real bug caught wiring up GHCR alongside Docker Hub**: `paws docker
--registries ghcr.io` was passing `--registries-csv ghcr.io` straight to
`dockerRelease`, whose `registriesCsv` param *replaces* its own
`"docker.io"` default wholesale instead of adding to it — contrary to
`--registries`' own doc string ("Additional registries to mirror tags
into"). Result: passing `--registries ghcr.io` silently published to
ghcr.io **only**, dropping Docker Hub entirely, even with
`--dockerhub-username`/`DOCKER_TOKEN` correctly configured. Fixed
(`full_registries_csv` in `crates/paws-cli/src/main.rs`, always keeps
`docker.io` + appends extras, deduplicated) and released as
`v0.0.1-prerelease.19` (live). **This was already merged and live in
`mbround18/ark-manager-web`'s `docker.yml`** from an earlier session —
unnoticed only because no real push to `main` has succeeded yet there
(still blocked on the npm immer quarantine from session 2/3). Worth
rechecking once that clears: confirm `ark-manager-web`'s images are
actually landing on both registries post-fix.

**Shipped same session**: generic registry support (Artifactory, private
registries, anything beyond docker.io/ghcr.io) — native in `paws-docker`,
per the user's call, bypassing `dockerRelease` entirely for any registry
it doesn't already hardcode. New `paws-docker` functions
(`native_registries`, `registry_token_env_var`, `tags_for_registry`,
`native_publish_pipeline_args`/`BuildSpec`/`NativeRegistryPublish`) plus a
new `paws docker --registry-username "<registry>=<username>"` flag
(token/password read from a derived env var, e.g. `myco.jfrog.io` ->
`$MYCO_JFROG_IO_TOKEN`). Only attempted on a real publish — PR/build-only
runs just print a skip line, since the Dockerfile is already validated by
the existing `dockerRelease` call regardless of how many registries it's
headed to. Verified for real against a live local registry (`registry:2`
+ htpasswd auth, real 33.5s build of `steamcmd-bases`' actual multi-stage
Rust-compiling Dockerfile) — build and registry auth both succeeded; the
only failure was the test registry lacking TLS (Dagger defaults to
HTTPS), not the feature itself. Released as `v0.0.1-prerelease.20`
(live). README documents the new flag.

**Still open**: confirm `ark-manager-web`'s `docker.yml` is actually
publishing to both registries now, once its `main` branch clears the npm
immer quarantine and a real push finally succeeds — it was silently
GHCR-only before the `full_registries_csv` fix (`v0.0.1-prerelease.19`).

## 7. Session 4 (2026-08-19, same day): paws helm --publish

User confirmed the bar: standard `index.yaml` Helm chart repo (`helm repo
add mbround18 https://mbround18.github.io/helm-charts/`), explicitly no
HTML catalog page (that's custom, `helm-hub` covers it separately). Planned
via EnterPlanMode (plan approved) since it touches real, live release infra.

**Shipped**: `paws helm --publish` — per-chart GitHub Release (tag
`<chart>-<version>`, asset skip-if-exists, never clobbered) + a real
`index.yaml` built via `helm repo index --merge` (once per chart, each its
own subdirectory so each gets its own correct download URL) pushed to a
pages branch via the Contents API (no git worktree/identity needed).
`paws-release::GitHubReleaseClient` gained `upload_asset_with`
(configurable content-type + Clobber/SkipIfExisting) and
`get_content`/`put_content`. Committed as `5196d2a`, released as
`v0.0.1-prerelease.18` (live, all 7 target zips).

**Two real bugs caught by live end-to-end verification** (created a
throwaway repo, `mbround18/paws-helm-publish-test`, left up per user's
call) — neither would have been caught by unit tests alone:
1. The running index's parent directory (`/idx`) was never created when no
   existing index was seeded (a repo's first-ever publish) — `cp` failed
   with "No such file or directory".
2. `cp` can't overwrite a Dagger bind-mounted file — the seeded existing
   index failed on a second publish run with "File exists". Fixed by
   mounting the seed at a separate `.seed` path, never written back onto
   directly.

Verified for real: two chart releases created, `.tgz` assets uploaded,
`index.yaml` published and — crucially — confirmed to actually work via a
genuine `helm repo add`/`helm search repo` against it (found both charts
correctly). A second publish run correctly skipped both already-uploaded
assets and didn't corrupt the index (new `created`/`digest` per run is
expected — `helm repo index` recomputes those from the repackaged `.tgz`
every time, harmless).

**Also shipped**: `mbround18/helm-charts#170` (same PR as session 3's
lint/package conversion) now also swaps `gh-pages.yml`'s publish job from
`tools/release_charts.py` to `paws helm --publish` — pushed, CI re-running.
**Not merged** — left for the user to review/merge, since this is real
production release infra for that repo.

**Still open** (unchanged from session 3, plus):
- The 7 ready-to-convert repos, `game-server-management`'s `paws publish`
  gap (crates.io/npm — a different thing from the Helm-chart publish just
  shipped), and `ark-manager-web`'s `release.yml` rewrite are all still
  untouched.
- `helm-charts`' Python test suite and the HTML catalog page
  (`index.html`/`charts-data.json`) are still explicitly out of scope.

# Tomorrow

Where the "convert ark-manager-web's CI/CD to paws" coverage push left off
(2026-08-19, overnight). Read top to bottom — later sections depend on
earlier ones landing. Updated right before logging off — the paws release
finished and I used the remaining time to re-verify against real CI runs,
so this supersedes the first draft of this file.

## 5. Session 2 (2026-08-19, daytime): repo audit + paws helm

After session 1: cut `v0.0.1-prerelease.15` (item 0's `--local-build` +
item 4's `paws audit` fix, both released and live). Then did a fresh
`gh api users/mbround18/repos` audit for the next conversion candidates —
findings and the full `paws helm` writeup are in `docs/ROADMAP.md`
(`paws publish` gap + Helm-chart gap sections), not repeated here.

**Shipped**: `paws-helm` (new crate) + `builders/helm/Dockerfile` + `paws
helm` CLI command — `helm lint`/`helm package` with proper topological
ordering over local `file://` chart dependencies. Verified for real against
charts pulled from `mbround18/helm-charts`. Committed + pushed (`e5d07de`).

## 6. Session 3 (2026-08-19, same day): helm-charts converted for real, plus two real paws bugs caught

Picked up right where session 2 left off: actually converted
`mbround18/helm-charts`'s CI to `paws helm`.

**Shipped**:
- `mbround18/helm-charts#170` — swaps `lint`/`lint-helm`/`build`'s actual
  `helm lint`/`helm package` invocations from `tools/chart_tasks.py` to
  `paws helm`/`paws helm --package --output ./tmp`, plus a `paws-up` step in
  the 3 CI jobs that now need `paws`/`dagger` (Helm Lint, Helm Build,
  `gh-pages`'s publish job). `deps-update`, the Python test suite,
  prettier/ruff, and chart-releaser/`gh-pages` publishing are all
  untouched, matching the scope call from session 2. **Merged/CI green**:
  `Helm Lint` + `Helm Build` both pass for real in GitHub Actions
  (confirmed after two follow-up paws fixes below — first run failed).
- **Real bug #1, caught by a full 32-chart dry run (not just unit tests)**:
  `paws-helm` had no remote Helm-repository handling — a fresh `helm`
  install has zero repos configured, so any chart with a non-`file://`
  dependency (`meilisearch` -> `istio-ingress`, `grafana`'s upstream chart)
  failed with "no repository definition ... please add the missing repos
  via 'helm repo add'". Fixed: scans every chart's `Chart.yaml`/`Chart.lock`
  for `repository:` URLs and does one `helm repo add --force-update`/`helm
  repo update` pass up front — mirrors `helm-charts`' own
  `.github/actions/setup-helm` composite action, which exists for exactly
  this reason. This fix was made *during* the session-2 dry run but never
  actually committed — caught only because the helm-charts PR's first real
  CI run failed with `error: unrecognized subcommand 'helm'` (the release
  binary `paws-up` installs was still v0.0.1-prerelease.15, predating even
  the `paws helm` command existing at all). Committed as `8fda4e7`.
- **Real bug #2, caught by that same release**: `paws release`'s
  `get_or_create_release` had a genuine race — `release.yaml`'s per-target
  matrix runs it concurrently, two legs can both see "no release yet" and
  both try to create one; GitHub accepts the first and 422s the second
  (`already_exists`/`tag_name`), which was previously fatal instead of
  recoverable. Reproduced for real on `v0.0.1-prerelease.16` (`linux-gnu` vs
  `linux-musl-x86_64` raced, musl lost, its asset never uploaded). Fixed
  (`36baf4a`): on that specific 422, re-fetch the release by tag instead of
  bailing. Verified for real: `v0.0.1-prerelease.17` published all 7 target
  zips cleanly, no race loss.
- Released `v0.0.1-prerelease.16` (helm remote-repo fix) and
  `v0.0.1-prerelease.17` (release-race fix), both live.

**Still open**:
- Seven ready-to-convert repos (`valheim-docker`, `meilisearch-operator`,
  `cloudflare-discord-oidc-worker`, `vein-docker`, `helm-hub`,
  `backup-docker`, `foundryvtt-docker`) — pure conversion work, no new paws
  capability needed, same shape as `ark-manager-web`/`helm-charts`. Not
  started.
- `game-server-management` also needs `paws publish` (crates.io/npm/OCI
  Helm), which doesn't exist yet.
- `helm-charts`' Python test jobs and `chart-releaser`/`gh-pages` publish
  flow are still explicitly out of scope — see `docs/ROADMAP.md`.
- Item 0's actual `ark-manager-web` `release.yml` rewrite (drop cargo-make/
  `auto shipit`, wire up `paws-up` + `paws semver` + `paws release
  --local-build`) — still unstarted, this is work in a different repo.
  `ark-manager-web`'s `docker.yml` was also still red as of this session
  due to the npm immer quarantine (session 2's finding) — worth a quick
  recheck, it should have cleared by now.

## 0. The actual goal: fully replace `release.yml`, not just gate it

Explicit ask: get `paws` to fully replace
[`release.yml`](https://github.com/mbround18/ark-manager-web/actions/runs/32222045387/workflow)
("Release Train WooohWoooohh") — not just gate it with `paws ci` (already
done) while leaving the cargo-make/`auto shipit` guts in place. That job
currently:

1. Sets up Rust nightly + `cargo-make`, builds `cargo make -p production
   release` (builds `agent` + `server` release binaries).
2. Zips them (`vimtor/action-zip`).
3. Runs `yarn install` + `yarn release` (`auto shipit`, driven by
   `.autorc.json`: PR labels `Version: Major`/`Version: Minor`/
   `Version: Patch` pick the bump, `Release <3` gates whether a release
   cuts at all via `onlyPublishWithReleaseLabel`, `git-tag` +
   `upload-assets` plugins tag + attach `./tmp/bundle.zip` to a GitHub
   Release).

**Why this can't just be `paws release --target ... --package server`
today**: read `crates/paws-release/src/lib.rs`'s `known_targets()` —
every target's `builder_dir` (e.g. `"builders/linux-gnu"`) is a path
resolved *relative to the calling repo's working directory*, not embedded
in the `paws` binary. That's fine for `paws`'s own release pipeline
(`./builders/*` ships in this repo), but `ark-manager-web` has no
`builders/` directory at all — `paws release` would immediately fail
trying to build a Dockerfile that doesn't exist there. Compare
`paws-tauri`/`paws-flatpak`, which embed their builder Dockerfiles via
`include_str!` and materialize them to a temp dir at runtime
(`write_builder_dockerfile()`) specifically so they work from *any*
target repo, not just `paws`'s own. `paws-release` needs the same
treatment before it's usable outside this repo.

**Concrete plan**:

1. ✅ **Done (2026-08-19).** Added a generic Rust-Linux builder to
   `paws-release` (`GENERIC_LINUX_GNU_DOCKERFILE` — literally
   `builders/linux-gnu/Dockerfile`, embedded via `include_str!` and
   materialized by `write_generic_builder_dockerfile()`, modeled directly
   on `paws-tauri`'s `write_builder_dockerfile()` pattern) plus
   `build_binary_local()`, which builds it locally via Dagger
   `docker-build` (mirrors `paws-tauri::dagger_pipeline_args`'s
   `host directory ... docker-build` chain) instead of pulling a
   prebuilt `paws-builders` image. Wired up as `paws release
   --local-build`, scoped to `paws_release::local_build_targets()`
   (`x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu` only, per the
   "start narrow" note — no macOS/Windows generic builder yet). Tests +
   clippy + `cargo test --workspace` all green; docs updated
   (`docs/DEVELOPMENT.md`'s release-pipeline section, `docs/ROADMAP.md`).
   **Not yet released** — still needs a `paws` version bump/tag before
   `ark-manager-web` can actually consume `--local-build` via
   `paws-up`.
2. ✅ **Done (2026-08-19), same change.** `--package`/`--binary-name` now
   both take a comma-separated list (paired 1:1, e.g. `--package
   agent,server --binary-name agent,server`); every listed binary gets
   built and packaged into one archive (name: joined binary names +
   version + target), matching `vimtor/action-zip`'s `files:
   target/release/agent target/release/server` step. No second
   invocation needed.
3. **Still open.** `paws semver`'s label flags
   (`--major-label`/`--minor-label`/`--patch-label`) already exist and
   already match what `.autorc.json`'s `Version: Major/Minor/Patch`
   labels need — nothing to build there. What's still undecided:
   `onlyPublishWithReleaseLabel` (`Release <3` gating whether a release
   cuts at all, not just which bump size) has no `paws` equivalent and
   probably shouldn't — likely a workflow-level `if:` in
   `ark-manager-web`'s `release.yml` checking for that label before
   calling `paws release` at all, since it's project-specific policy.
4. **Still open — the actual `ark-manager-web` wiring.** Once a new
   `paws` prerelease ships item 1, rewrite `release.yml`'s `release` job:
   drop `dtolnay/rust-toolchain`/`davidB/rust-cargo-make`/
   `vimtor/action-zip`/`yarn install`+`yarn release`, replace with
   `paws-up` + `paws semver` (bump computation) + `paws release
   --local-build --package agent,server --binary-name agent,server`
   (build+package+upload). This is unstarted — it's a change to
   `ark-manager-web`, a different repo, not this one.

This is the biggest remaining piece of "fully convert this repo's CI/CD
to paws" — everything else (`rust.yml`, `docker.yml`, `enforce-labels.yml`)
is done.

## 1. paws: fully done, both fixes released and live

- **#3** — `paws docker` never forwarded `--dockerhub-username`/
  `--ghcr-username`/`$DOCKER_TOKEN`/`$GHCR_TOKEN` to the underlying
  `dockerRelease` Dagger call at all. Fixed, released in
  `v0.0.1-prerelease.13`.
- **#4** — `paws docker` silently exited 0 on a *real* publish failure,
  because `dockerRelease`'s JSON has no `success` field (only
  `decision`/`outcome`), and `call_pipeline_report` only ever checked
  `success`. Fixed (`pipeline_report_succeeded()`), released in
  `v0.0.1-prerelease.14` — **confirmed built and published**, all 7
  target zips + 8 builder images green, and confirmed working for real
  against `ark-manager-web` (see below — it's what surfaced the real
  content-hash bug in the first place instead of hiding it).

Both documented in `docs/ROADMAP.md`'s `paws docker` coverage note. No
further paws-side action needed unless you want to chase item 2 below.

## 2. RESOLVED (2026-08-19, overnight session 2): not a Dagger bug — quarantined npm package

Root-caused. `failed to content hash dockerfile copy` is a misleading BuildKit
wrapper message, **not** a Dagger/BuildKit content-hashing bug and **not** a
`gh-reusable` `__withDirectoryDockerfileCompat` bug as originally suspected
below. Reproduced locally (`dagger core host directory --path=. docker-build
--dockerfile=./Dockerfile.client sync` against a fresh clone) and got the real
underlying error:

```
➤ YN0016: │ immer@npm:11.1.18: All versions satisfying "11.1.18" are quarantined
```

`__withDirectoryDockerfileCompat` eagerly runs `RUN yarn install` as part of
BuildKit's content-hash computation for caching — when that `yarn install`
fails, BuildKit reports the outer, generic "failed to content hash dockerfile
copy" instead of surfacing yarn's actual error. A Renovate PR
(`renovate/immer-11.x`) auto-merged and bumped `immer` to `11.1.18`; per
`registry.npmjs.org/immer`, that version published 2026-08-19T07:25 UTC and
was still inside npm's post-publish quarantine (malware-scan hold) window
when both real-push failures happened tonight (`32249474965` at 11:49 UTC,
and my re-trigger `32267534893` at 15:01 UTC to confirm reproduction — both
identical root cause).

**This is transient, not a real bug to fix** — it should clear on its own
once npm's quarantine lifts (typically within hours of publish) or immer gets
manually verified. Options, none of which I did without asking: wait it out,
temporarily pin immer back to the last known-good version, or add Kodiak/CI
guardrails against auto-merging a Renovate PR whose fresh dependency version
is still quarantine-age. The original "one Dockerfile confirmed, one
unconfirmed" framing below is superseded — there was never a `Build client`
vs `Build web` Dockerfile-specific bug; both dockerBuild calls just eagerly
`RUN yarn install`/equivalent, so whichever leg runs first hits whatever
`yarn install`/`npm install` state is broken at that moment.

<details>
<summary>Original (superseded) investigation notes</summary>

## 2 (superseded). Real, still-open bug: `failed to content hash dockerfile copy`

With #4's fix live, a `paws docker` push against `main` (triggered by an
auto-merged verify PR, #2035 — see item 3) surfaced a **real** BuildKit
failure on `Build client` (`./Dockerfile.client`, the simpler of the two —
`node:lts` → `caddy:latest`, no external `FROM <other-image>` stage):

```
failed to content hash dockerfile copy: exit code: 1
```

(full error is in the job log for run `32222045186`, job
`95974189198` — it's a genuine BuildKit/Dagger error, buried at the end of
a long serialized-container-ID dump in the `Reason` field). This is
inside `gh-reusable`'s vendored Dagger module
(`packages/dagger-module/src/index.ts`'s `dockerRelease` →
`contextDir.dockerBuild()`), not paws's own Rust code.

**This is a different error from what I chased earlier tonight**
(`failed to load container from converted ID`, seen on `Build web`
against the *main* `./Dockerfile`, which has the external
`FROM mbround18/ark-manager-client:latest` stage). That one did **not**
reproduce when re-tested in PR/build-only mode (#2035, build succeeded
cleanly). But `Build web` never actually got exercised against a real
push tonight — its leg was `cancelled` (matrix `fail-fast: true`,
`Build client` failed first) on the one run that would have told us. So:
**one Dockerfile has a confirmed-reproducing content-hash failure on
push; the other's earlier failure might have been a one-off flake, but
that's unconfirmed, not disproven.**

**Next step**: re-run `docker.yml` on `main` (`gh workflow run` needs a
trigger — easiest is `git commit --allow-empty` + push, or just wait for
the next real commit) and watch both legs run all the way through this
time. If `failed to content hash dockerfile copy` reproduces again on
`Build client`, that's a real bug worth root-causing or reporting
upstream against `gh-reusable` — it's very likely something about how
`__withDirectoryDockerfileCompat` (a custom compat shim visible in the
error dump, not stock Dagger) hashes a particular file in this repo's
build context (`.yarn/releases`, `yarn.lock`, or similar — the Dockerfile
copies several before `RUN yarn install`). Try isolating with a minimal
reproduction Dockerfile in `paws`'s own `examples/` before touching
`gh-reusable` itself.

</details>

## 3. ark-manager-web status

- **`rust.yml`, `docker.yml`, `release.yml`, `enforce-labels.yml`** — all
  converted/fixed, merged to `main` (PRs #2032, #2034).
- **PR #2035** — a no-op verification PR I opened to re-test `docker.yml`
  post-fix (build-only in PR mode; succeeded cleanly). It got
  **auto-merged into `main` by the `kodiakhq` bot** (per `.kodiak.toml`)
  before I could close it — harmless (one trailing newline in
  `README.md`), but it's why `docker.yml` then ran for real against
  `main` and surfaced item 2 above. Nothing to undo, just flagging so the
  README diff in history doesn't look mysterious.
- **`main` is currently red on `docker.yml`** (2026-08-19, see item 2) — a
  `renovate/immer-11.x` PR auto-merged (kodiak) and bumped `immer` to
  `11.1.18`, which is still inside npm's post-publish quarantine window as
  of this writing. I pushed one empty commit (`cfdf341`) to re-confirm the
  failure reproduces reliably; did **not** revert/pin immer or touch
  Kodiak config — that's a real decision (wait it out vs. pin vs. add a
  guardrail) left for the morning, not something to silently fix.
  **Decision (2026-08-19): leave it, don't touch main again** — npm
  quarantine windows are typically hours, not days, and immer published
  07:25 UTC today, so it should clear on its own. Re-check
  `docker.yml`'s next real run before assuming it's still broken.
- **`release.yml`'s `Release Train WooohWoooohh` job** — the cargo-make
  build succeeds (dead `ATiltedTree/setup-rust` action fixed); the
  `GH_TOKEN` secret that was causing `yarn release`/`auto shipit` to fail
  with `Bad credentials` has been **rotated by the user (2026-08-19)** —
  fixed, no longer blocked.
- No open PRs left on `ark-manager-web`.

## 4. DONE (2026-08-19, session 2): paws audit's success-reporting gap

Confirmed the gap was real, then closed it deliberately rather than
copy-pasting the docker fix. `gh-reusable`'s `audit` Dagger function's
top-level JSON really does have no `success`/`decision`/`outcome` (same gap
shape `dockerRelease` had), but it *does* compute a real nested status:
`report.outputs.auditSummary.overallStatus`
(`"pass"|"findings"|"degraded"|"failed"`, per `audit-types.ts`/
`audit-logic.ts` in `gh-reusable`). `pipeline_report_succeeded` in
`crates/paws-cli/src/main.rs` now reads it.

Decision (confirmed with the user): only `"failed"` (a scanner itself
errored/couldn't run) makes `paws audit` exit non-zero. `"findings"`
(scanners ran clean but found real security issues) stays non-fatal —
`paws audit` doesn't have a severity-threshold concept yet, and failing on
any finding with no way to tune it would turn every repo's first run red.
Tests added (`pipeline_report_succeeded_reads_audit_overall_status`),
`cargo test --workspace` + clippy green, `docs/ROADMAP.md` updated.
