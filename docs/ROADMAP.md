# Roadmap: stack coverage

Target application stacks `paws` should eventually understand, and an honest read of what's
actually wired today versus what's just detected/planned. This is a target list, not a promise —
see [`docs/DEVELOPMENT.md`](DEVELOPMENT.md) for how a new stack actually gets added.

## Current coverage (as of this doc)

Confirmed directly against the code, not from memory:

- **`paws ci --toolchain <x>`** (build/lint/test execution): `rust`, `node`, `python`, `go`,
  `java`, `kotlin`, `tauri`, `tauri-android`, `flatpak`, `esp32` (`crates/paws-cli-core/src/lib.rs`)
  — see "Embedded (ESP32 / no_std targets)" below for `esp32`'s full write-up. `java`
  (2026-08-22, `crates/paws-java`) is a new native implementation too — `gh-reusable` only ever
  had `setupJava` (container setup picking a JDK distribution/image, no build/test steps). Detects
  Maven (`pom.xml`) vs Gradle (`build.gradle`/`build.gradle.kts`) and **requires** the project's
  own wrapper script (`mvnw`/`gradlew`) rather than falling back to an unpinned system
  `mvn`/`gradle` this crate would have to install and version itself — a repo without one
  committed gets a clear error instead. Runs `sh mvnw -B verify` or `sh gradlew build` (invoked via
  `sh`, not `./`, so a missing execute bit on the wrapper isn't a build failure) against
  `builders/java` (JDK 21 + JDK 25 side by side, not a plain image pull — see "JVM / Go stacks"
  below for the full finding on why one JDK pin can't cover both an old Gradle <=8.10 project and
  a real `java.toolchain.languageVersion = JavaLanguageVersion.of(25)` declaration).
  `eclipse-temurin`'s base layer isn't a port of `gh-reusable`'s own `DEFAULT_TEMURIN_IMAGE`
  (`eclipse-temurin:<version>-jdk-trixie`), which doesn't actually exist on Docker Hub (confirmed
  directly: no Debian-suffixed tag is published for `eclipse-temurin` at all); `-jammy` is the real
  verified equivalent for the same JDK 21 Temurin distribution. Verified for real, end to end,
  against `examples/java-maven-fixture`, `examples/java-gradle-fixture`, and
  `examples/java-jdk25-toolchain-fixture` — real `mvnw`/`gradlew` wrappers generated from actual
  Maven 3.9.9/Gradle 8.10/Gradle 9.3.1 installs, JUnit 5 tests genuinely executing through Dagger.
  No `paws-provision` support for `java` yet (unlike `go`) — JDK version management has no single
  obviously-right "assume it's already present" tool the way `rustup`/`corepack`/`uv`/Go's own
  `golang.org/dl` mechanism do, so that's left as a deliberate gap rather than a guess.
  `go` (2026-08-22, `crates/paws-go`) is a new native implementation, not a port — unlike
  `rust`/`python`, `gh-reusable` never had a `goBuildAndTest` Dagger function, only `setupGo`
  (container setup with
  no build/test steps of its own), so there was no real logic to port for parity. Runs `go build
  ./...`, `go vet ./...`, `go test ./...` against the plain `golang:1-bookworm` image,
  fail-fast — deliberately no `gofmt` gate for this first cut (`gofmt -l` doesn't itself fail on
  unformatted code the way `cargo fmt -- --check` does, and wiring a shell one-liner through
  `dagger core`'s comma-joined `--args` risks the same CSV-parsing fragility `paws-audit` moved
  away from). Verified for real, end to end, against `examples/go-fixture` — a genuine `go1.23.4`
  toolchain was installed into this dev environment specifically to sanity-check the fixture
  locally first (it had none beforehand), then the actual `paws ci --toolchain go` run went
  through a real Dagger `golang:1-bookworm` container, independent of that local toolchain.
  `paws ci`'s auto-multi-provision detection (`detect_needed_ecosystems`) also now treats `go.mod`
  as a signal, alongside `paws-provision`'s `install_go` (see below) making `go` a real
  `paws provision` target too. Node execution is now natively multi-package-manager
  (`crates/paws-node` — npm/yarn/pnpm/bun, detected from lockfiles or `package.json`'s
  `packageManager` field, no longer the old `pnpmBuildAndTest`-only interim path) and
  framework-aware (Vite, Next.js, or plain, informational for now). The `Node` row's "Backend
  Server" output half is verified for real, end to end, against `examples/node-server-fixture` (a
  plain `node:http` server, no framework/bundler) — the "NPM Package" half stays unverified since
  it's really about publishing, which `paws publish` (see below) doesn't do yet. Verified for real against
  fixtures covering all 4 package managers plus real `create-vite`/`create-next-app` scaffolds
  (including a React+TSX one) — see `examples/README.md`. It also detects Playwright e2e projects
  (`@playwright/test` dependency or a `playwright.config.*` file) and runs a dedicated
  `npx playwright install --with-deps && npx playwright test` pipeline instead of the plain
  build+test one — verified for real, end to end, against `examples/playwright-fixture` (a real
  `create-playwright` scaffold); confirmed directly that no `xvfb` is needed for standard headless
  Playwright runs, contrary to the initial assumption. `tauri` (`crates/paws-tauri`) builds a
  Tauri app through a dedicated `builders/tauri-linux` Dockerfile via Dagger, verified for real
  against a `create-tauri-app` scaffold (`examples/tauri-fixture`) — Linux-only so far. `python`
  (`crates/paws-python`) is a native port of `gh-reusable`'s real `pythonBuildAndTest` function
  (`uv sync --all-groups [--frozen] && uv build && uv run pytest` against
  `astral/uv:python3.12-trixie-slim`) — `uv`-based projects only, matching what `gh-reusable`
  actually supports (no poetry/pipenv/pip path exists there to port). Verified for real, end to
  end, against `examples/python-fixture` (a real `uv init` scaffold). `rust` (`crates/paws-rust`)
  is a native port of `gh-reusable`'s real `rustBuildAndTest` function (`cargo fmt -- --check`,
  `cargo clippy`, `cargo build --verbose`, `cargo test --verbose`, fail-fast) against the plain
  `rust:1-bookworm` image — no `gh-reusable` dependency left for `--toolchain rust` at all.
  Verified for real, end to end, dogfooding `paws` on its own repo. `flatpak`
  (`crates/paws-flatpak`) runs `flatpak-builder --build-only` against an auto-detected manifest,
  through `builders/flatpak` (`ubuntu:26.04` + `xvfb` + flatpak + flatpak-builder + the Flathub
  freedesktop runtime/SDK/rust-stable extension baked in) — needs `--insecure-root-capabilities`
  on the `with-exec` (flatpak-builder's sandboxed build is FUSE-backed; verified directly that
  `fuse3` + a bare device flag aren't enough on their own), and is build-only, not a full bundle
  export: the base image switch from Debian fixed a real `flatpak-builder`-version-specific
  missing-binary failure, but a full bundle still hits a separate, unresolved `appstreamcli
compose` runtime difference under this pipeline's root context that a real GitHub-hosted
  runner (same versions, non-root) doesn't hit. Verified for real, end to end, against
  `mbround18/oled-wallpaper`'s actual manifest — a real, heavy wgpu/winit GUI app, not a
  synthetic fixture; a full bundle/release flow should keep using its own existing pipeline for
  now (see `builders/README.md` for the full story).
- **`paws provision`** (concurrent toolchain installers): `rust`, `node`, `python`, `go`, `esp32`
  (`paws_provision::Ecosystem`) — `esp32`'s `install_esp32` shells to `espup install` (Design
  Decision 6, `specs/007-esp32-toolchain`), same "shell to the real installer" precedent as every
  other ecosystem below. `go` (2026-08-22) is real, not detection-only: `install_go` runs
  `go install golang.org/dl/goX.Y.Z@latest` then that version's own `download` subcommand — Go's
  own official mechanism for fetching an additional pinned toolchain version, the same shape as
  `install_rust`/`install_node`/`install_python` shelling out to `rustup`/`corepack`/`uv`
  respectively rather than reimplementing an installer. Unlike those three, Go has no floating
  "stable"/"latest" alias for this specific mechanism, so the version is a real pinned release
  (`1.23.4` by default, overridable via `$PAWS_GO_VERSION`) rather than a moving target — mirrors
  `gh-reusable`'s own `DEFAULT_GO_VERSION` in spirit, a concrete version instead of a floating
  Docker tag. Verified for real: downloaded a genuine `go1.23.4` toolchain into this dev
  environment (which had no `go` at all beforehand) specifically to exercise `install_go` against
  it, confirming both a first install and a repeated (idempotent) call succeed — this also caught
  a real test-harness bug, not a production one: the crate's own `tool_on_path` test helper checks
  `<bin> --version`, which `go` doesn't support (only the `version` subcommand), so every
  Go-dependent test had been silently skipping via a captured `eprintln!` that only became visible
  under `--nocapture`. `paws ci`'s auto-multi-provision path (`detect_needed_ecosystems`) now also
  treats `go.mod` as a signal, matching `Cargo.toml`/`package.json`/`pyproject.toml`. `gh-reusable`
  (the TS system `paws` is replacing) also has `setupRuby`/`setupJava`/`setupTerraform`/
  `setupPulumi` — none of those ecosystems are wired into `paws-provision` yet, they're just
  precedent for what "add a new ecosystem" looks like next.
- **`paws audit`** (language detection for scanner selection): detects `rust`, `node`, `python`,
  `go`, `docker` signals (`paws_audit::LanguageFamily`) — detection only, not execution; still
  doesn't detect `java`/`kotlin` signals even though `paws ci --toolchain java`/`kotlin` both exist
  now (`paws-audit`'s `LanguageFamily` hasn't been extended to match — a real, if minor, gap
  between what `paws ci` can build and what `paws audit`'s scanner-selection step notices).
  As of 2026-08-19, `paws audit` no longer depends on `gh-reusable` at all — scanner execution
  (`semgrep`/`gitleaks`, the only two scanners `gh-reusable`'s `audit` function ran) is native
  `crates/paws-audit` logic running through `paws-dagger::core`, a byte-for-byte port of
  `gh-reusable`'s `runSemgrepScanner`/`runGitleaksScanner` shell scripts. `AuditOverallStatus`
  (`Pass`/`Findings`/`Degraded`/`Failed`, ported from `audit-types.ts`/`audit-logic.ts`) is
  computed locally in Rust rather than read back out of a TS pipeline's report — deliberately only
  `Failed` (a scanner itself errored) fails `paws audit`; `Findings` (scanners ran clean but found
  real issues) stays non-fatal, since there's no severity-threshold concept yet to decide which
  findings should block a build. Building this surfaced two real `dagger core --args` CSV-parsing
  bugs (embedded raw newlines get silently truncated; embedded literal double-quotes break the
  parser entirely) — fixed by writing scanner scripts via `with-new-file` instead of inlining them
  into `--args`. A third scanner, **`cargo-audit`** (2026-08-23, `specs/005-close-remaining-cli`),
  joined the catalog — `ScannerName::CargoAudit`, gated on the `LanguageFamily::Rust` signal alone
  (`ScannerFamily::Language(Rust)`, not `CrossLanguage` like Semgrep/Gitleaks), running
  `cargo install cargo-audit --locked && cargo audit --json` against `rust:1-bookworm` and mapping
  RustSec advisories (`vulnerabilities.list[]`) to `TopFinding`s the same shape every other
  scanner's parser produces. Same report-don't-fail default as Semgrep/Gitleaks (only a genuine
  scanner error, not a finding, fails the run) — confirmed by reading `run_audit`'s actual dispatch
  loop, not assumed: it's fully generic over the scanner catalog, no per-scanner special-casing.
- **`paws docker`**: any `docker-compose.yml`/`Dockerfile`-based project, regardless of source
  language — this one's already stack-agnostic, since it works from the compose file / Dockerfile
  contract rather than a language-specific build step. Registry auth
  (`--dockerhub-username`/`--ghcr-username`, or their `$DOCKERHUB_USERNAME`/`$GHCR_USERNAME`
  fallbacks, plus `$DOCKER_TOKEN`/`$GHCR_TOKEN`) — verified for real, end to end, converting
  `mbround18/ark-manager-web`'s Docker workflows onto `paws docker` (2026-08-18): before this,
  `push=true` resolved correctly but the pipeline had nothing to authenticate a publish with, so
  it built and silently published nothing. As of 2026-08-19, `paws docker` no longer depends on
  `gh-reusable` at all — build+tag+push for docker.io/ghcr.io go through `paws-docker`'s own
  native `Container.withRegistryAuth`/`Container.publish` calls, the same primitives that also
  make arbitrary registries work (`--registries myco.jfrog.io --registry-username
myco.jfrog.io=you`, token read from a derived `$MYCO_JFROG_IO_TOKEN`-style env var) —
  Artifactory, a private registry, anything `dockerRelease` (the `gh-reusable` function this
  replaced) had no way to authenticate to. With `paws audit`'s native scanner port landing the
  same day (see above), `paws` now has zero runtime dependency on `gh-reusable` for any
  subcommand.
- **`paws release`**: primarily cross-compiles **`paws` itself** (the Rust binary) for multiple
  OS/arch via its prebuilt `paws-builders` images — don't read the full target matrix here as
  stack coverage for user projects. `--local-build` (2026-08-19) extends it to other Rust repos,
  but narrowly: an embedded generic Linux-gnu builder only, `x86_64`/`aarch64-unknown-linux-gnu`
  targets only, no macOS/Windows. See `docs/DEVELOPMENT.md`'s release-pipeline section.
- **Java** was flagged 2026-08-18 as the next gap worth naming, per a real `gh api
  users/mbround18/repos` audit: after Rust/JS/TS, Java was the next-most-common language across
  `mbround18`'s own non-fork repos (3, all Gradle-based Hytale mods). `crates/paws-java`/`paws ci
  --toolchain java` landed 2026-08-22 — but re-checked for real against those same 3 repos the
  same day, none of them actually built with it: `hytale-modding-template`/
  `hytale-vex-lich-dungeon`/`hytale-demo-ui` all declare `java.toolchain.languageVersion =
  JavaLanguageVersion.of(25)` for plugin compilation, and `paws-java`'s original
  `eclipse-temurin:21-jdk-jammy` pin had no JDK 25 to resolve that against.
  **Root-caused and fixed the same day**: the earlier "JDK 25 breaks Gradle" finding was
  real but the wrong culprit was blamed — confirmed for real it's Gradle **8.10** that can't even
  *launch* on a JDK-25 host JVM at all (`Unsupported class file major version 69`, Gradle's own
  Groovy DSL parsing, unrelated to Kotlin specifically), while the actual Hytale repos pin Gradle
  **9.3.x**, which launches fine on JDK 21 *or* 25 — confirmed for real with an isolated pure-Java
  Gradle-9.3.1 project. So the real fix wasn't "pick one JDK" at all: `builders/java` (new,
  2026-08-22) installs JDK 21 *and* JDK 25 side by side under `/usr/lib/jvm/` — the location
  Gradle's own toolchain auto-detection scans on Linux, confirmed for real, no target-project
  config needed — with `JAVA_HOME` defaulting to 21 so old Gradle <=8.10 pins keep working while a
  real `languageVersion = JavaLanguageVersion.of(25)` declaration resolves the 25 install
  alongside it. `crates/paws-java`/`crates/paws-kotlin` both now build through this image instead
  of a plain pull — the same "needs multiple toolchains combined" reasoning every other
  `builders/*` image exists for (`docs/DEVELOPMENT.md`'s "How a new stack gets added"), just
  discovered for JVM version selection rather than a language combination. Verified for real, end
  to end, against `examples/java-jdk25-toolchain-fixture` (a real Gradle 9.3.1 wrapper, `.
  toolchain.languageVersion = JavaLanguageVersion.of(25)`, full build+test) *and* against
  `hytale-modding-template` itself directly — its `plugin` module's JDK-25 toolchain resolves and
  its own orchestration tasks (`:ensureDirectories`/`:assetsZip`) run fine; the repo's *root*
  `:ensureServerJar` task then fails on an unrelated, genuinely out-of-scope requirement (Docker-
  in-Docker plus an interactive OAuth login, to fetch a proprietary game server jar) — a
  repo-level CI design question for that project, not a `paws-java` gap.
- **`paws publish --target rust-crate`** (2026-08-22): closes the `game-server-management` half of
  the gap below (`--target rust-crate`, crates.io, for `libs/env-parse` — the only `publish.yaml`
  target that repo's real workflow actually uses, confirmed against its actual workflow rather than
  assumed from `publish.yaml`'s general `target: node | rust-crate | helm-chart` shape). New
  `paws-publish` crate runs `cargo check && cargo test && cargo package && cargo publish` via
  `dagger core`, gated on `$CARGO_REGISTRY_TOKEN` (or `$CARGO_REGISTRIES_<NAME>_TOKEN` for a
  non-default `--registry`), with `--dry-run` to verify packaging without publishing. Along the way,
  found and fixed a real, previously undiagnosed bug in the pipeline it replaces: `gh-reusable`'s
  `publishRustCrate` mounts only the target crate's own subdirectory, which breaks on any crate
  that's a workspace member (`edition.workspace = true` etc. needs the real workspace root
  Cargo.toml on disk) — confirmed for real via `gh run list`/`gh api .../logs` against
  `mbround18/game-server-management`'s actual tag-triggered publish runs: all 10 `libs/*` crates
  have failed on every real attempt. `paws-publish::find_workspace_root` walks up to the real
  workspace root (stopping correctly at a crate's own self-contained `[workspace]` declaration,
  e.g. `examples/rust-fixture`, rather than walking past it into an enclosing workspace) and mounts
  that instead, with `workdir` set to the crate's relative subpath. Verified for real, end to end,
  against three cases: `examples/rust-fixture` (standalone, own empty `[workspace]`), `crates/paws-
  go` (a real member of paws's own workspace), and a downloaded copy of the actual blocked crate,
  `libs/env-parse` from `mbround18/game-server-management`.
  Deliberately narrow, matching `gh-reusable`'s own actual usage rather than its full advertised
  contract: only `--target rust-crate` exists — `node`/`helm-chart` targets are unneeded (no real
  caller uses them; `paws helm --publish` already covers Helm chart publishing on a different,
  OCI-registry path — see below). The five remaining real conversion candidates from the
  2026-08-19/2026-08-22 repo audit (`valheim-docker`, `meilisearch-operator`,
  `cloudflare-discord-oidc-worker`, `helm-hub`, `backup-docker` — `vein-docker` and
  `foundryvtt-docker` turned out already converted) only call `rust-build-n-test`/`docker-release`/
  `tagger`, functions `paws ci`/`paws docker`/`paws semver` already cover — pure conversion work,
  not a `paws` gap.
- **`paws helm`** (2026-08-19): closes the lint/package half of the Helm-chart gap above, found
  converting `mbround18/helm-charts` itself. New `paws-helm` crate + `builders/helm/Dockerfile`
  (Alpine + Helm's own official install script, no maintained "official" Helm image exists to
  just pull) run `helm lint`/`helm package` for every chart under `charts/*/Chart.yaml` (or a
  root `Chart.yaml`), ported from that repo's own `tools/chart_tasks.py` — including a real gap
  that script worked around with an ad hoc recursive pre-build: charts with local `file://`
  dependencies (`mbround18/helm-charts` has real 2-level chains, e.g. `bubbles-ttrpg` ->
  `mongo` -> `gitops-tools`) need their dependencies packaged before them, which plain
  alphabetical/discovery order gets wrong. `paws-helm` does this with a proper topological sort
  over the local-dependency graph instead. Verified for real, end to end, against actual charts
  pulled from `mbround18/helm-charts` (`gitops-tools` standalone, and `mongo` -> `gitops-tools`
  for the dependency-ordering + `--package` export path) — not just unit tests.
  Deliberately still narrow, matching the scope decided for this first cut: no `chart-releaser`/
  `gh-pages` publishing (a GitHub-App-token based mechanism, unrelated to registry auth, and
  fundamentally different from the OCI `publish.yaml` `helm-chart` target in the gap above), and
  this repo's Python jobs (which don't fit `paws-python`'s fixed `uv sync && uv build && uv run
pytest` pipeline shape — no package meant for `uv build`, several separately-scoped `pytest`
  invocations with per-suite JUnit summaries, not one blanket call) are still untouched.
- **Dagger build cache** (2026-08-23, `specs/005-close-remaining-cli`): `paws docker`/`paws ci`
  now detect and use a remote build cache via `paws_dagger::CacheBackend` — auto-selected (no CLI
  flag), `DAGGER_CLOUD_TOKEN` winning if both signatures are present. **`GitHubActionsCache`**
  (`$ACTIONS_CACHE_URL` + `$ACTIONS_RUNTIME_TOKEN`, the legacy Cache Service v1 REST API) is the
  fully-implemented provider — real restore-before/save-after around the shared Dagger engine
  container's persistent state (a Docker volume at `/var/lib/dagger`, confirmed for real via
  `docker inspect` against a live engine, not assumed), and deliberately given equal-or-greater
  implementation weight than `DaggerCloud`: it needs no external paid account, so it's what most
  GitHub-Actions-only consumers actually depend on. **`DaggerCloud`** (`DAGGER_CLOUD_TOKEN`) needs
  near-zero code beyond detection — the token already reaches the `dagger` subprocess via
  inherited environment. Neither backend wraps `paws_dagger::core`/`core_streaming` themselves
  (those stay byte-for-byte unchanged) — restore/save bracket a whole `paws docker`/`paws ci`
  invocation once, via `restore_cache_backend`/`save_cache_backend`, since a single invocation can
  call `core` many times (e.g. `paws ci`'s own fmt/clippy/build/test steps) and stopping/restoring
  the shared engine around each one individually would be both wasteful and unsafe for adjacent
  calls in the same process. `$ACTIONS_RESULTS_URL`-only environments (the newer Twirp/protobuf
  results service) are a known, explicitly-out-of-scope follow-up — detected but not implemented,
  falls through to no cache rather than a nonfunctional half-implementation.
  **Update (`specs/006-paws-doesn-expose`)**: `005` shipped `GitHubActionsCache`'s detection and
  restore/save logic, but `$ACTIONS_CACHE_URL`/`$ACTIONS_RUNTIME_TOKEN` never actually reached a
  `paws-up`-provisioned job's process environment — GitHub Actions withholds both from a plain
  bash `run:` step, so `CacheBackend::detect()` always resolved `None` in practice. `paws-up`
  (`actions/paws-up/action.yml`) now runs a pinned `actions/github-script` step before `paws
  init` that exports both vars to `$GITHUB_ENV` when present, closing the gap with no new
  consumer-facing setup step required. This exposure fix is what let `GitHubActionsCache`'s
  restore/save logic run against a real Actions cache service for the first time ever
  (`005`'s own tests only ever injected the vars directly); doing so surfaced one real bug in
  the save path — `save_github_actions_cache` mounted a not-yet-existing host path into the
  helper container, which Docker creates as a directory rather than a file, breaking every save
  with `tar: can't open '/backup.tar.gz': Is a directory` (harmlessly — `005`'s "broken cache
  backend is never worse than no cache backend" guarantee held; only the save silently no-opped).
  Fixed in the same change by pre-creating the file.
- **`paws docs --provider`** (2026-08-23, `specs/005-close-remaining-cli`): `paws docs` now
  actually publishes, closing the gap where `--help` claimed GitHub Pages publishing that didn't
  exist. `--provider github-pages` (comma-delimited, multi-provider capable) builds `target/doc`
  once and publishes it in exactly one commit via the GitHub Git Trees API (`GitHubReleaseClient::
  create_blob`/`publish_tree` — blob-create every file, one tree/commit/ref-update for the whole
  set, never a per-file `put_content` loop), auto-selecting between that Git Trees path and the
  Pages deployment/artifact API from the repo's live Pages configuration (`build_type: "legacy"`
  vs `"workflow"`) — the deployment API path is gated on GitHub Actions' own runtime env vars
  (`ACTIONS_RUNTIME_TOKEN`/`ACTIONS_RESULTS_URL`) since it only works inside a real Actions job,
  and fails with those vars named explicitly rather than attempting a doomed call. Idempotent: a
  content digest stashed at `.paws-docs-manifest` on the publish branch skips a no-op republish.
  `--provider cloudflare-pages`/`--provider s3` are recognized (`PublishTarget::CloudflarePages`/
  `::S3`) but intentionally not implemented yet — they fail immediately with an actionable "not
  implemented — see docs/ROADMAP.md" error rather than a silent no-op or an "unrecognized value"
  parse error, reserving the surface for a real future implementation. Tracked here as the explicit
  follow-up this bullet exists to name.

## Status legend

- ✅ **Supported** — `paws ci`/`paws provision` actually runs this today.
- 🚧 **Partial** — some real support exists (detection, provisioning, or a related toolchain), but
  the specific stack/output isn't fully wired.
- 📋 **Planned** — not started; listed here as a target, not committed to a timeline.

## Web / desktop (JS, Rust, Tauri) stacks

| Stack Permutation           | Primary Languages      | Package Manager(s)          | Core Toolchain / Frameworks               | Output Type                           | Status       |
| --------------------------- | ---------------------- | --------------------------- | ----------------------------------------- | ------------------------------------- | ------------ |
| React                       | JavaScript, TypeScript | npm, yarn, pnpm, bun        | Node.js (build env), React, Vite/Webpack  | Static Web Assets (HTML/CSS/JS)       | ✅           |
| Node                        | JavaScript, TypeScript | npm, yarn, pnpm, bun        | Node.js                                   | NPM Package / Backend Server          | 🚧           |
| Rust                        | Rust                   | cargo                       | rustc, Cargo                              | Native Executable (.exe, ELF, Mach-O) | ✅           |
| Node + React                | JavaScript, TypeScript | npm, yarn, pnpm, bun        | Node.js, React, Next.js / Express         | SSR Web App / Full-stack              | ✅           |
| Rust + React                | Rust, JS/TS            | cargo & (npm/yarn/pnpm/bun) | Rust (Actix/Axum), React                  | Backend API + Static UI               | ✅           |
| Node + Rust                 | JS/TS, Rust            | npm/yarn/pnpm/bun & cargo   | Node.js, Rust, napi-rs or neon            | Native Node bindings (.node)          | 📋           |
| React + Rust                | JS/TS, Rust            | npm/yarn/pnpm/bun & cargo   | React, Rust, wasm-pack                    | WebAssembly (.wasm) + React UI        | 📋           |
| Tauri + Rust                | Rust, HTML/CSS/JS      | cargo                       | Tauri, Rust, OS Webview (WebKit/WebView2) | Desktop App Installer                 | ✅           |
| Tauri + Node + Rust         | Rust, JS/TS            | cargo & (npm/yarn/pnpm/bun) | Tauri, Node.js (Sidecar), Rust            | Desktop App + Node Backend Process    | 📋           |
| Tauri + React + Rust        | Rust, JS/TS            | cargo & (npm/yarn/pnpm/bun) | Tauri, React, Rust, Vite/Next.js          | Desktop App (React UI)                | ✅           |
| Tauri + React + Node + Rust | Rust, JS/TS            | cargo & (npm/yarn/pnpm/bun) | Tauri, React, Node.js, Rust               | Desktop App + Embedded Node APIs      | 📋           |
| Tauri + Android             | Rust, JS/TS            | cargo & (npm/yarn/pnpm/bun) | Tauri, JDK, Android SDK/NDK               | .apk / .aab                           | 🚧           |
| Tauri + iOS                 | Rust, JS/TS            | cargo & (npm/yarn/pnpm/bun) | Tauri, Xcode, `xcodebuild`                | .ipa                                  | 📋 (blocked) |

`paws ci --toolchain tauri` (`crates/paws-tauri`) detects a Tauri project (`src-tauri/tauri.conf.json`)
and runs `<package manager> run tauri build` against a dedicated `builders/tauri-linux` Dockerfile
(Rust + Node + the GTK/WebKit libs Tauri's Linux backend needs), through Dagger like every other
builder — Tauri's own CLI handles the frontend-then-Rust sequencing via `tauri.conf.json`'s
`beforeBuildCommand`, so `paws` never has to. Verified for real, full (non-`--no-bundle`) build —
producing `.deb`/`.rpm`/`.AppImage` — against a real `create-tauri-app` vanilla-ts scaffold
(`examples/tauri-fixture`) and, as of 2026-08-22, a real `create-tauri-app` react-ts scaffold
(`examples/tauri-react-fixture`) — same full, non-`--no-bundle` build, same three bundle outputs,
confirming the pipeline is genuinely package-manager-driven rather than framework-driven as
predicted, so this row is now ✅. Linux-only for now — no macOS/Windows Tauri
builder exists yet, and the Node-sidecar row is a distinct capability (spawning a persistent Node
process alongside the Tauri shell) this crate doesn't attempt.

**Android** gets its own `builders/tauri-android` Dockerfile (JDK 17 + Android SDK/NDK + Rust's
Android cross targets + Node) and `paws ci --toolchain tauri-android`, which runs `<package
manager> run tauri android build`. The builder image itself is build-verified — JDK, `sdkmanager`,
platform/build-tools/NDK install, and `rustup target add` for all four Android ABIs all confirmed
working through a real Dagger build. It's marked 🚧, not ✅, because it assumes the target repo has
already run `tauri android init` (`src-tauri/gen/android` committed) — `paws` doesn't scaffold
mobile projects itself — and a full `tauri android build` hasn't been run against a real generated
Android project yet.

**iOS is explicitly blocked, not just unstarted.** Unlike macOS (where `osxcross` lets Rust
cross-compile against the Darwin ABI using Apple's redistributed SDK headers — see
`builders/macos/`), there's no equivalent path for iOS: `cargo tauri ios build` shells out to real
Xcode (`xcodebuild`) to generate and build the Xcode project and produce the `.ipa`, and Apple's
license terms require Xcode to run on genuine Apple hardware/macOS. That's a legal and technical
constraint a container image can't route around. iOS support would need a real macOS build host
(a GitHub-hosted `macos-*` runner or a self-hosted Mac) wired in as a _different kind_ of backend
than the Docker-image-through-Dagger approach every other target here uses — not attempted yet.

`Rust + React` (2026-08-22) is not one composite pipeline the way Tauri is — `paws ci` takes one
`--toolchain` per invocation, so this row is two independent, unwired runs against the same repo:
`paws ci --toolchain rust` (Axum backend) and `paws ci --toolchain node` (the React SPA it serves
as static assets from `frontend/dist`). Both verified for real, end to end, against
`examples/rust-react-fixture` — no new `paws` capability needed, since the existing `rust`/`node`
toolchains already compose cleanly for this shape (see that fixture's README).

`Node + Rust`/`React + Rust` need real new capability, not just wiring: native addon builds
(`napi-rs`/`neon`) and `wasm-pack` WebAssembly builds are each a distinct toolchain `paws` doesn't
drive today, on top of whatever multi-ecosystem provisioning already gets you partway (see
`examples/multi-ecosystem-fixture`).

## JVM / Go stacks

| Stack Permutation    | Primary Languages | Package Manager(s)      | Core Toolchain / Frameworks            | Output Type                                     | Status |
| -------------------- | ----------------- | ----------------------- | -------------------------------------- | ----------------------------------------------- | ------ |
| Java                 | Java              | Maven (mvn), Gradle     | JDK, Spring Boot, Quarkus              | .jar, .war, Docker Image                        | ✅     |
| Kotlin (JVM)         | Kotlin            | Gradle, Maven           | JDK, kotlinc, Ktor, Spring Boot        | .jar, Docker Image                              | ✅     |
| Java + Kotlin        | Java, Kotlin      | Gradle                  | JDK, Mixed Kotlin/Java Compilation     | .jar, Maven/Gradle Package                      | ✅     |
| Go                   | Go                | Go Modules (go mod)     | Go Toolchain (go build), standard lib  | Native Executables (ELF, .exe, Mach-O)          | ✅     |
| Kotlin (Android)     | Kotlin            | Gradle                  | Android SDK, Jetpack Compose           | .apk, .aab (Android App Bundle)                 | 📋     |
| Kotlin Multiplatform | Kotlin            | Gradle                  | Kotlin/JVM, Kotlin/Native, Kotlin/Wasm | .jar (JVM), .framework (iOS), .js / .wasm (Web) | 📋     |
| Go + WebAssembly     | Go                | Go Modules              | Go Compiler (GOOS=js GOARCH=wasm)      | WebAssembly (.wasm) + JS wrapper                | ✅     |
| Go + C/C++ (cgo)     | Go, C/C++         | Go Modules, make/cmake  | Go Toolchain (cgo), GCC/Clang          | Native Executables (dynamically linked)         | ✅     |
| Java + React/Node    | Java, JS/TS       | Maven/Gradle & npm/yarn | JDK, Node.js, Spring Boot, React       | Backend .jar + Static Web Assets                | ✅     |
| Go + React/Node      | Go, JS/TS         | Go Modules & npm/yarn   | Go Toolchain, Node.js, React           | Go Binary + Static Web Assets                   | ✅     |

`Java` (`crates/paws-java`, see "Current coverage" above) and `Java + React/Node` both landed
2026-08-22 too — the latter is, like `Rust + React`/`Go + React/Node`, two independent `paws ci`
runs against one repo rather than one composite pipeline: `paws ci --toolchain java` (a plain
`com.sun.net.httpserver.HttpServer` backend, no framework dependency) and `paws ci --toolchain
node` (the React SPA it serves from `frontend/dist`), both verified for real against
`examples/java-react-fixture` — its `ServerTest` makes a genuine `java.net.http.HttpClient` round
trip against a real `/api/health` response, not just a compile check.

`Kotlin (JVM)` (`crates/paws-kotlin`, 2026-08-22) is Gradle-only, deliberately — real Kotlin
projects are overwhelmingly Gradle-based, and unlike Java, `kotlinc` isn't part of the JDK at all;
it's a Gradle plugin dependency Gradle fetches itself, so this needed no *new* build commands,
just reusing `paws-java`'s `sh gradlew build` pipeline shape — and, since 2026-08-22, `paws-java`'s
`builders/java` image too (duplicated rather than shared, matching how `paws-go`/`paws-rust`/
`paws-python` each independently own their pipeline builder). Detection scans for real `.kt`/`.kts` source files
under a Gradle build — a build file merely mentioning Kotlin isn't the same as having Kotlin code
— and, like `paws-java`, requires the project's own `gradlew` wrapper. Verified for real, end to
end, against `examples/kotlin-fixture` through Dagger (`:compileKotlin`/`:test`/`:build` all
genuinely executing). This also closed `Java + Kotlin` for free, the same "zero code changes"
story as `Go + C/C++ (cgo)`: a single Gradle module with both `.java` and `.kt` sources — one
calling into the other, not just sitting side by side unused — compiles and tests through this
exact same pipeline unchanged, because Gradle's `java`/`kotlin` plugins already handle mixed
compilation themselves. `examples/java-kotlin-mixed-fixture` exists purely to prove that; its
`CalculatorTest` (Kotlin) exercises `Calculator.add` (Kotlin) calling `Adder.add` (Java), real
interop, not a coincidence of file extensions.

`Go`'s `paws-provision` support (`install_go`) landed 2026-08-22, and `paws ci --toolchain go`
(`crates/paws-go`, see "Current coverage" above) followed the same day — it was the most natural
first pickup here, needing no JVM-style version-matrix design the way `Java`/`Kotlin` would. Its
three variant rows below all closed the same day too:
- **`Go + WebAssembly`** genuinely needed new pipeline logic: `paws_go::is_wasm_project` scans
  `.go` files for a `"syscall/js"` import (the same purpose-built-signal detection style
  `paws_rust::is_wasm_project` uses for `wasm-bindgen`), and when detected,
  `dagger_pipeline_args` sets `GOOS=js`/`GOARCH=wasm` on the container *before* `go vet`/`go
  build -o app.wasm` run — Go's build-constraint system uses those to decide which files even
  exist to the compiler — and skips `go test` (a `js`/`wasm` binary can't execute in this
  container, no JS engine present, same rationale as `paws-rust`'s wasm path skipping `cargo
  test`). Verified for real, end to end, against `examples/go-wasm-fixture`.
- **`Go + C/C++ (cgo)`** needed **zero code changes** — confirmed for real that
  `golang:1-bookworm` already has `CGO_ENABLED=1` as its default and already ships `gcc`, so a
  package with `import "C"` already builds/tests through the exact same plain pipeline used for
  ordinary `Go`. `examples/go-cgo-fixture` exists purely to prove that.
- **`Go + React/Node`** is, like `Rust + React`, two independent `paws ci` runs against one repo
  rather than one composite pipeline — `paws ci --toolchain go` (a plain `net/http` backend) and
  `paws ci --toolchain node` (the React SPA it serves from `frontend/dist`), both verified for
  real against `examples/go-react-fixture`, no new capability needed.

**`paws ci --toolchain go --targets <GOOS>/<GOARCH>[,...]`** (2026-08-22) is a real multi-platform
build matrix, not just the single wasm cross-compile above generalized in name only: Go needs no
cross-linker or extra toolchain component for any host/target combination (confirmed for real
against `golang:1-bookworm` — plain `GOOS`/`GOARCH` env vars are enough for every pairing tried),
so `paws_go::cross_dagger_pipeline_args` builds N targets in one pipeline (a `with-env-variable`
pair + `go vet`/`go build -o dist/<module>-<goos>-<goarch>[.exe]` group per target, `go test`
skipped for all of them — once multiple targets are being produced there's no single "the native
one" to special-case, and none of the resulting binaries can run in this build container anyway),
ending in one `directory`/`export` of the whole `dist/` folder — a different (and, for a matrix,
more efficient) pattern than `paws-release`'s one-`file`-export-per-invocation
(`crates/paws-release`), verified for real that `dagger core`'s `directory ... export` chain
exports a populated build directory correctly, not just single files. Verified for real, end to
end, against `examples/go-fixture` with `--targets linux/amd64,darwin/arm64,windows/amd64`: `file`
confirms genuine ELF/Mach-O/PE32+ binaries came out, respectively, not just three files with
plausible names. `--targets` is rejected outside `--toolchain go` with a clear error.

## Embedded (ESP32 / no_std targets)

| Stack Permutation           | Primary Languages | Package Manager(s) | Core Toolchain / Frameworks                          | Output Type                              | Status |
| ---------------------------- | ------------------ | ------------------- | ------------------------------------------------------ | ------------------------------------------ | ------ |
| ESP32 (ESP-IDF, `esp-idf-svc`) | Rust               | cargo + `embuild`   | `espup`-installed Xtensa/RISC-V toolchain, ESP-IDF, CMake/Python | Flashable firmware ELF + bootloader binary | ✅     |
| ESP32 (bare-metal `esp-hal`, `no_std`) | Rust      | cargo               | `esp-hal`, no ESP-IDF/Python/CMake                      | Flashable firmware binary                  | 📋     |

This gets its own section, not a row in "Other language ecosystems"'s table above — that table's
Output Type column doesn't fit "flashable firmware binary + bootloader", the same reasoning that
already gave Tauri/Android its own callout there rather than a table row.

`paws ci --toolchain esp32` (`crates/paws-esp32`, `specs/007-esp32-toolchain`) is new, native
capability — no `gh-reusable` ESP32/embedded function exists to port for parity, same as
`paws-go`/`paws-kotlin`. Detects a project the same marker-file way `paws-rust`'s
`is_wasm_project` does: a `Cargo.toml` dependency on `esp-idf-sys`/`esp-idf-svc`, or a
`.cargo/config.toml` `build.target` set to an `*-espidf` triple
(`xtensa-esp32*-espidf`/`riscv32im*-esp-espidf`). Runs `cargo fmt --check`, `cargo clippy -- -D
warnings`, `cargo build --release` — the real `embuild`-driven ESP-IDF cross-compile — against a
dedicated `builders/esp32` image (`rust:1-bookworm` + `espup install` + `libclang`/`clang` +
`python3`+pip + `espflash`, joining the `tauri-android`/`flatpak`/`java` "needs multiple
toolchains combined" builder-image category). Deliberately `fmt → clippy → build → conditional
test`, not `paws-rust`'s default `fmt → clippy → test → build` — the embedded target itself has no
`cargo test` story at all (a real `[[bin]] harness = false` skips even *compiling* `#[cfg(test)]`
code for that target, confirmed against this feature's own driver, `mbround18/ha-kiosk`'s
`firmware/` crate), so `build` — the one step that has to succeed for this toolchain to mean
anything — never sits behind a test step that might not even exist for a given project. When a
host-testable sibling workspace member exists (a `Cargo.toml` with no `esp-idf-sys`/`esp-idf-svc`
dependency and no `*-espidf` target override — not a hardcoded name like `firmware-core`, a
generic structural check), `cargo test` runs against *that* crate instead of the embedded target.

A new opt-in `--publish-artifacts` flag (gated to `--toolchain esp32` only, mirroring
`--coverage`'s existing `--toolchain rust`-only gating) uploads the built bootloader
(`bootloader.bin`) and firmware ELF as assets on the GitHub Release matching the current tag,
reusing `paws-release`'s existing `GitHubReleaseClient::get_or_create_release`/`upload_asset_with`
rather than a second GitHub-API client — the first `paws-release` type reused outside
`paws-release` itself. Needs the same `$GITHUB_TOKEN`/`$GH_TOKEN` + `$GITHUB_REPOSITORY` every
other GitHub-Release-touching `paws` subcommand already reads, no new env var name. Verified for
real against `examples/esp32-fixture` (a minimal `esp-idf-svc` "blink" project); the bare-metal
`esp-hal`/`no_std` row is a plausible fast-follow, not bundled into this first cut — a different
(simpler — no ESP-IDF/Python/CMake) toolchain shape.

## Other language ecosystems

| Stack Permutation     | Primary Languages | Package Manager(s)               | Core Toolchain / Frameworks        | Output Type                             | Status |
| --------------------- | ----------------- | -------------------------------- | ---------------------------------- | --------------------------------------- | ------ |
| Python                | Python            | uv                               | CPython, FastAPI, Django           | Python Package (.whl), Docker Image     | ✅     |
| C# / .NET             | C#, F#            | NuGet                            | .NET SDK, ASP.NET Core, EF Core    | Binaries (.exe, .dll), NuGet Package    | 📋     |
| C / C++               | C, C++            | conan, vcpkg, system pkg mgrs    | GCC, Clang, MSVC, CMake, Make      | Native Binaries, Libs (.so, .dll, .a)   | 📋     |
| Ruby                  | Ruby              | gem, bundler                     | Ruby (MRI), Ruby on Rails, Sinatra | Gem Package, Docker Image               | 📋     |
| PHP                   | PHP               | composer                         | PHP-FPM, Laravel, Symfony          | Source code deploy, Docker Image        | 📋     |
| Swift (iOS/macOS)     | Swift             | Swift Package Manager, CocoaPods | Xcode Command Line Tools, iOS SDK  | iOS App (.ipa), macOS App (.app)        | 📋     |
| Flutter               | Dart              | pub                              | Flutter SDK, Android/iOS SDKs      | Mobile (.apk, .aab, .ipa), Web Assets   | 📋     |
| Electron + React/Node | JS/TS             | npm, yarn, pnpm                  | Node.js, Electron, React           | Desktop App Installers                  | 📋     |
| .NET Blazor           | C#, HTML/CSS      | NuGet                            | .NET SDK, WebAssembly              | Wasm Binary + Static Web Assets         | 📋     |
| .NET MAUI             | C#, XAML          | NuGet                            | .NET SDK, Android/iOS/Mac/Win SDKs | Mobile/Desktop Apps (.apk, .ipa, .msix) | 📋     |
| Python + C/C++        | Python, C/C++     | pip, cmake                       | CPython, pybind11, Cython          | Native Python Extensions (.so, .pyd)    | 📋     |
| React Native          | JS/TS, Java/Swift | npm, gradle, CocoaPods           | Node.js, React Native, Mobile SDKs | Mobile Apps (.apk, .aab, .ipa)          | 📋     |
| Zig                   | Zig               | zon (Zig Object Notation)        | Zig Compiler                       | Native Executable (.exe, ELF, Mach-O)   | 📋     |

`Python`'s ✅ is `uv`-only, matching what `gh-reusable`'s real `pythonBuildAndTest` function
actually supports — pip/poetry/conda projects (no `pyproject.toml` + `uv.lock`) aren't detected
and fall through to `paws ci`'s "unsupported toolchain" error; see "Current coverage" above.
`Swift`/`.NET MAUI`/`Flutter` mobile+Apple targets are the hardest group: they need either a
macOS-hosted runner or a cross-compilation story `paws` doesn't have for anything beyond `paws`'s
own binary (`builders/macos`'s osxcross setup is Rust-specific, not a general Swift/Xcode
toolchain).

## How a new stack gets added

Roughly, in order:

1. **Detection** — add the language's marker files to `paws-audit`'s `LanguageFamily`/signal
   list (`crates/paws-audit/src/lib.rs`), if it isn't there already.
2. **Provisioning** — add a real installer to `paws-provision`'s `Ecosystem` enum
   (`crates/paws-provision/src/lib.rs`), following the existing `install_rust`/`install_node`/
   `install_python` pattern (shell to the real toolchain installer, don't reimplement it).
3. **CI execution** — wire a new `--toolchain <x>` value in `paws ci`
   (`crates/paws-cli-core/src/lib.rs`), either against a `gh-reusable` function that already exists
   for it (check `packages/dagger-module/src/index.ts` first — several of the ecosystems above
   already have a `setupX`/`xBuildAndTest` precedent there) or a new native crate, following
   `paws-semver`'s/`paws-audit`'s/`paws-docker`'s precedent of porting real logic rather than
   guessing at it.
4. **Fixtures** — add a real example project under `examples/` (see `examples/README.md`) so the
   new support is tested against something real, not just unit tests of the Rust logic.

Most stacks don't need a new builder image under `./builders/*` — only ones needing *multiple*
toolchains (or, per `java/`'s 2026-08-22 addition, multiple *versions* of the same toolchain)
combined that no single public image provides (Rust+Node+GTK for `tauri`, JDK+Android SDK/NDK+
Rust+Node for `tauri-android`, Ubuntu+flatpak-builder+runtime for `flatpak`, JDK 21+JDK 25 for
`java`/`kotlin` — see `builders/java/Dockerfile`'s header comment). `paws-go`/`paws-python`/
`paws-rust`, plus `paws-node`, still just pull a public image directly, since one already has
everything needed; confirmed for real per-crate (e.g. `gcc`/`CGO_ENABLED=1` already present in
`golang:1-bookworm` for cgo). A stack needing cross-compilation support for _`paws` itself_ to
target is a separate, third reason a builder image might exist — unrelated to `paws` being able to
build _other projects_ written in that language.

**Base image version policy** (2026-08-22): pin to a tag that *itself* tracks "current" wherever
the upstream image publishes one, and only hardcode a specific version where it doesn't:
- `node:lts-trixie` (`paws-node`), `golang:1-bookworm` (`paws-go`), `rust:1-bookworm`
  (`paws-rust`), `oven/bun:1-debian` (`paws-node`'s Bun path) all self-update — Node's official
  image publishes a genuine `lts` alias Docker Hub itself maintains (confirmed for real: not to be
  confused with `ltsc*`-suffixed tags there, which are Windows Server's unrelated *Long-Term
  Servicing Channel*), and Go/Rust/Bun have no LTS concept at all — "latest major" already *is*
  "current" for them. Nothing to bump here, ever; not a Renovate target.
- `astral/uv:python<X.Y>-trixie-slim` (`paws-python`, `DEFAULT_PYTHON_VERSION`) is a real, bare
  hardcoded pin (`3.13` currently) — confirmed directly against Docker Hub that `astral/uv`
  publishes no floating "latest Python" tag, unlike Node. A genuine Renovate target (see
  `renovate.json`'s `customManagers`).
- `eclipse-temurin:21-jdk-jammy` (`paws-java`/`paws-kotlin`, shared) stays on JDK 21, not the
  newer JDK 25 LTS (Sept 2025), *not* because no floating tag exists (true, but not the reason) —
  confirmed for real that a JDK-25 build JVM breaks a genuine `gradlew build` under Gradle 8.10 +
  Kotlin Gradle plugin 1.9.24 (still common), even though `gradle --version` alone succeeds on it.
  `renovate.json` marks this dependency `automerge: false` specifically so a version-bump PR gets
  a human's judgment call, not blind automation, given that real regression risk.

`renovate.json`'s default `config:recommended` docker manager only scans `Dockerfile`/compose
files — it's blind to image tags embedded in Rust string literals like all of the above. The
`customManagers` entries there (regex-based) exist to give Renovate visibility into the ones that
actually need bumping over time (`DEFAULT_PYTHON_VERSION`, the shared Temurin pin,
`paws-provision`'s `DEFAULT_GO_VERSION`); verified for real against the actual
`renovate-config-validator`/`renovate --dry-run=extract` tooling, not just JSON-schema validity,
including a real bug caught and fixed in the process — the Python regex, unanchored, also matched
an unrelated test's deliberately-non-default `"3.11"` literal as a second false dependency.

## Test coverage reporting (`paws ci --coverage`)

Target: an opt-in `--coverage` flag on `paws ci` that runs each toolchain's native coverage tool
inside the same Dagger pipeline `paws ci` already runs tests through, and prints a summary (with a
machine-readable report exportable later for upload to a service like Codecov). Tracked here as a
target list, same spirit as the stack-coverage tables above.

| Toolchain | Coverage tool | Status |
| --- | --- | --- |
| Rust (`paws-rust`) | `cargo llvm-cov` | ✅ |
| Node (`paws-node`) | `c8` / `istanbul` (package-manager-driven, like the existing build/test path) | 📋 |
| Python (`paws-python`) | `coverage.py` (via `uv run coverage`) | 📋 |
| Go (`paws-go`) | `go test -cover` (built into the Go toolchain, no extra install) | 📋 |
| Java/Kotlin (`paws-java`/`paws-kotlin`) | JaCoCo (Maven/Gradle plugin) | 📋 |

Rust (`specs/004-rust-coverage/`) was the deliberate starting point and landed first: a new
`builders/rust` image, pre-baked with `cargo-llvm-cov`/`llvm-tools-preview` (the first Rust
builder-image exception, per that spec's Clarifications — every other Rust `paws ci` run still
pulls plain `rust:1-bookworm` directly). `paws ci --toolchain rust --coverage` runs the existing
`fmt`/`clippy`/`build`/`test` sequence completely unchanged, then appends a `cargo llvm-cov
--workspace --summary-only` step — tests execute twice under `--coverage` (once for the pass/fail
gate, once for the coverage report), a deliberate tradeoff to keep the existing `cargo test` step
untouched. On a wasm project, `--coverage` is a silent no-op (the wasm pipeline already can't run
`cargo test` on the host, so there's nothing to measure). This proved out the CLI contract
(`--coverage` flag shape, `--toolchain`-only gating, report format) the remaining languages, each
with their own tool and output format, will follow next.

## MCP + llms.txt

`paws mcp setup` writes/merges an MCP client config (`.mcp.json` for Claude Code by default, or
`claude_desktop_config.json` via `--client claude-desktop`) pointing at `paws mcp serve`.
`paws mcp serve` runs an MCP server (stdio transport, `crates/paws-mcp`) exposing every `paws`
subcommand as an MCP tool — each tool calls the same `run_*` function `paws`'s own CLI dispatch
calls (`crates/paws-cli-core`), not a subprocess of `paws` itself, so an MCP client gets identical
behavior to the CLI with zero proxy overhead. `paws llms generate` renders `llms.txt` (the
<https://llmstxt.org> convention) directly from the CLI's own `clap::Command` metadata, so it can't
drift from real `--help` output; `--publish` commits it to GitHub via the Contents API
(`GitHubReleaseClient::get_content`/`put_content`, the same mechanism `paws helm --publish` uses
for `index.yaml`). CI regenerates and publishes it automatically on every push to `main`
(`.github/workflows/ci.yaml`'s `llms` job) — a PR merge is itself a push event, so this covers both
without a separate trigger.

`llms.txt` also documents paws's GitHub Actions (today just `actions/paws-up`) in a `## GitHub
Actions` section, and the `actions` MCP tool returns the same metadata as JSON — so an agent
working inside a _consumer_ repo (not paws's own checkout) can discover `paws-up`'s inputs/outputs
without leaving the MCP session. This works from any working directory because the action's YAML is
embedded into the `paws` binary at compile time (`crates/paws-cli-core/src/action_metadata.rs`,
`include_str!`) rather than read from disk at runtime — a runtime path relative to cwd would find
nothing once `paws` is running inside someone else's repo.

## Native GitHub App auth (`paws auth github-app`)

Every `paws` subcommand that needs a GitHub token (`semver --push`, `helm --publish`, `release`,
`llms generate --publish`) resolves it through `paws_environment::resolve_github_token`
(`crates/paws-environment/src/lib.rs`): if `$GH_APP_CLIENT_ID` and a private key
(`$GH_APP_PRIVATE_KEY` or `$GH_APP_PRIVATE_KEY_FILE`) are both set, it signs a JWT and mints a
fresh GitHub App installation token (`mint_github_app_installation_token`) itself — the exact
mechanism `actions/create-github-app-token` provides as a separate Action, done natively so no
extra CI step is needed. Otherwise it falls back to the plain `$GITHUB_TOKEN`/`$GH_TOKEN` env vars
every call site used to read directly before this existed. `CiContext::detect()` picks this up for
free (it's the shared token path both `semver --push` and `llms generate --publish`'s
`$GITHUB_REPOSITORY` branch already went through); `paws helm --publish` and `paws release`'s
upload step call `resolve_github_token` directly instead of reading env vars themselves. This
matters beyond convenience: this repo's `main` branch ruleset only bypasses its required-status-
check rule for direct (non-PR) commits when they're authored by this specific GitHub App, not the
default `GITHUB_TOKEN` — see `.github/workflows/ci.yaml`'s `llms` job for that in practice, now just
two env vars (`GH_APP_CLIENT_ID`, `GH_APP_PRIVATE_KEY`) on the `paws llms generate --publish` step
rather than a `create-github-app-token` step feeding a `GITHUB_TOKEN` override.

`paws auth github-app` (`--client-id`/`--private-key`/`--private-key-file`/`--repository`, same env
var fallbacks) exists for the case that wants the raw token directly rather than paws using it
internally — it prints _only_ the token to stdout (diagnostics go to stderr), so
`TOKEN=$(paws auth github-app)` works as a plain shell capture, e.g. for handing the token to
another tool.

## CI/CD onboarding for consumer repos (`paws workflow generate`)

`paws workflow generate` scaffolds a starter GitHub Actions workflow (default
`.github/workflows/paws.yml`) for a repo that wants to adopt `paws`, run from _inside that repo_ —
distinct from `paws`'s own `.github/workflows/ci.yaml`. It reuses `collect_repository_signals()`
(the same detection `paws audit` runs) to decide which `paws ci --toolchain <x>` steps to emit
(rust/node/python), plus a `paws docker` step if a Dockerfile/compose file is present and a `paws
helm` step if `paws_helm::detect_project` finds a chart. The emitted workflow always starts with
`actions/checkout@v7` + `mbround18/paws/actions/paws-up@main`. The Docker step is deliberately
build-only — it never guesses at `--push`/registry credentials, since the generator has no way to
know what registry secrets (if any) the target repo has configured; the generated file leaves a
comment telling the user how to turn on publishing themselves. If nothing is detected at all
(no recognizable Rust/Node/Python/Docker/Helm markers), it prints a message and writes nothing
rather than emitting an empty/useless workflow.

**Multi-origin readiness**: the command takes `--provider` (default `"github"`), matched explicitly
in `run_workflow_generate` — anything else fails loudly naming `paws_environment::Provider` (the
existing GitHub-only-today CI-context enum, see `crates/paws-environment/src/lib.rs`) as the
extension point. Deliberately _not_ a new trait/abstraction: `Provider` already exists for exactly
this generalization, and until a second origin (e.g. GitLab) has a real implementation to pattern
the shape after, inventing one now would just be speculative structure. Adding a second origin
later means: a `Provider::GitLab` (or similar) variant in `paws-environment`, and a second
`"gitlab" => { ... }` match arm here rendering `.gitlab-ci.yml` instead of a GitHub Actions
workflow — the `DetectedWorkflowInputs`/detection logic stays provider-agnostic and is reused as-is.
