# Examples / fixtures

Small, real (not mocked) projects that `paws`'s own tests and CI run subcommands against, so
parity claims are backed by an actual build/lint/test/detect run rather than a unit test that
only exercises Rust logic in isolation. These are the fixture projects referenced by
`specs/001-paws-core-cli/spec.md` (FR-007, FR-008, FR-012) and `tasks.md` task groups 3-6.

Each fixture is intentionally standalone (its own `Cargo.toml`/`package.json`/etc., not a
workspace member) so it behaves like a real target repository `paws` is pointed at, not like
part of `paws` itself.

- `rust-fixture/` — a minimal crate that builds and tests cleanly; the "clean" half of
  `paws ci --toolchain rust`'s acceptance scenario.
- `node-fixture/` — a minimal pnpm-style project; the target for `paws ci --toolchain node`.
- `docker-fixture/` — a plain `Dockerfile`, no compose file at all; exercises `paws docker`'s
  "no `docker-compose.yml`" fallback to `./Dockerfile` + `.` (spec.md's Edge Cases, FR-004).
- `docker-compose-fixture/` — a `docker-compose.yml` with two services, one whose `image:`
  matches the expected name (`app`) and one that doesn't (`sidecar`), exercising FR-012's
  "first matching service wins, no arbitrary fallback" rule for `paws docker`.
- `docker-buildkit-fixture/` — a `Dockerfile` using BuildKit-only syntax (`RUN --mount=type=cache`,
  heredoc `COPY`) that fails under the legacy builder (`DOCKER_BUILDKIT=0`) and only succeeds via
  `docker buildx build`. Verifies `paws docker`'s build path actually goes through BuildKit
  rather than falling back to a builder that silently can't build modern Dockerfiles.
- `python-fixture/` — a minimal Python project managed the way `uv` would manage it
  (`pyproject.toml`, a trivial `python_fixture` module, and a `unittest`-based test runnable via
  plain `python3 -m unittest`, no pytest/uv install required). Gives `paws-audit`'s
  `detect_family` python-family signals (`pyproject.toml`, `uv.lock`, `poetry.lock`,
  `requirements.txt`, `setup.py`) and `paws-provision`'s future python/uv ecosystem support a
  real target to point at.
- `multi-ecosystem-fixture/` — a single repo containing both a minimal Rust crate
  (`Cargo.toml` + `src/lib.rs`, `cargo test`-clean) and a minimal Node project (`package.json`
  + `index.js`/`index.test.js`) in the same directory, simulating "a repo with both a Rust crate
  and a pnpm workspace". Exercises spec.md's User Story 5, acceptance scenario 3: `paws ci`/
  `paws provision` must provision multiple detected ecosystems concurrently, not sequentially.
- `node-fixture-with-lint-failure/` — a Node project like `node-fixture/`, but `index.js`
  deliberately contains a `console.log(...)` call, which `lint.js` (a dependency-free regex-based
  "lint" stand-in run via `npm run lint`, no eslint/npm install needed) flags as a violation of
  its one rule, `no-console-log`, exiting non-zero and printing the exact file and line of the
  offending call. Exercises spec.md's User Story 1, acceptance scenario 1 ("Given a Node project
  fixture with a failing lint rule, When `paws ci --toolchain node` runs, Then it exits non-zero
  and reports the specific lint failure"). Its `index.test.js` still passes cleanly, so the
  fixture isolates the lint failure from test behavior. Also has no lockfile (`npm` detected via
  fallback, not a `package-lock.json`) — caught a real bug where `npm ci` (which requires an
  existing lockfile) was used even when none exists yet; see `crates/paws-node`.
- `node-fixture-npm/`, `node-fixture-yarn/` — the same minimal project as `node-fixture/`
  (pnpm), one per remaining package manager `paws-node` detects from lockfiles
  (`package-lock.json`, `yarn.lock`). Each verified for real: `paws ci --toolchain node` installs,
  builds, and tests through a real Dagger container.
- `node-fixture-bun/` — same again for Bun, but detected via `package.json`'s `packageManager`
  field rather than a lockfile (a zero-dependency `bun install` produces no lockfile at all) —
  exercises the same lockfile-optional install path as `node-fixture-with-lint-failure/`, this
  time for real with Bun's own base image (`oven/bun:1-debian`, since Bun isn't bundled with the
  official Node image the way npm/yarn/pnpm are via `corepack`).
- `vite-fixture/` — a real, unmodified `npm create vite -- --template vanilla-ts` scaffold (plus
  a `test.mjs` asserting `dist/index.html` exists post-build, since Vite's own templates don't
  include a test script). Exercises `paws-node`'s `Framework::Vite` detection and a real
  `tsc && vite build`.
- `react-vite-fixture/` — a real `npm create vite -- --template react-ts` scaffold (React + TSX),
  keeping its generated `oxlint` lint script — exercises the full install → build → test → lint
  chain for a real React/TypeScript project, not just plain JS.
- `next-fixture/` — a real `create-next-app` scaffold (TypeScript, App Router, ESLint), with a
  `test.mjs` asserting `.next/BUILD_ID` exists post-build. Exercises `paws-node`'s
  `Framework::NextJs` detection and a real `next build` (Turbopack) + `eslint` run.
