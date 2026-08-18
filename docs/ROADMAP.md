# Roadmap: stack coverage

Target application stacks `paws` should eventually understand, and an honest read of what's
actually wired today versus what's just detected/planned. This is a target list, not a promise —
see [`docs/DEVELOPMENT.md`](DEVELOPMENT.md) for how a new stack actually gets added.

## Current coverage (as of this doc)

Confirmed directly against the code, not from memory:

- **`paws ci --toolchain <x>`** (build/lint/test execution): `rust`, `node`, `python`, `tauri`
  (`crates/paws-cli/src/main.rs`). Node execution is now natively multi-package-manager
  (`crates/paws-node` — npm/yarn/pnpm/bun, detected from lockfiles or `package.json`'s
  `packageManager` field, no longer the old `pnpmBuildAndTest`-only interim path) and
  framework-aware (Vite, Next.js, or plain, informational for now). Verified for real against
  fixtures covering all 4 package managers plus real `create-vite`/`create-next-app` scaffolds
  (including a React+TSX one) — see `examples/README.md`. `tauri` (`crates/paws-tauri`) builds a
  Tauri app through a dedicated `builders/tauri-linux` Dockerfile via Dagger, verified for real
  against a `create-tauri-app` scaffold (`examples/tauri-fixture`) — Linux-only so far. `python`
  (`crates/paws-python`) is a native port of `gh-reusable`'s real `pythonBuildAndTest` function
  (`uv sync --all-groups [--frozen] && uv build && uv run pytest` against
  `astral/uv:python3.12-trixie-slim`) — `uv`-based projects only, matching what `gh-reusable`
  actually supports (no poetry/pipenv/pip path exists there to port). Verified for real, end to
  end, against `examples/python-fixture` (a real `uv init` scaffold).
- **`paws provision`** (concurrent toolchain installers): `rust`, `node`, `python`
  (`paws_provision::Ecosystem`). `gh-reusable` (the TS system `paws` is replacing) already has
  `setupGo`/`setupRuby`/`setupJava`/`setupTerraform`/`setupPulumi` — none of those ecosystems are
  wired into `paws-provision` yet, they're just precedent for what "add a new ecosystem" looks
  like.
- **`paws audit`** (language detection for scanner selection): detects `rust`, `node`, `python`,
  `go`, `docker` signals (`paws_audit::LanguageFamily`) — detection only, not execution. A repo
  with a `go.mod` gets audited correctly; `paws ci --toolchain go` doesn't exist.
- **`paws docker`**: any `docker-compose.yml`/`Dockerfile`-based project, regardless of source
  language — this one's already stack-agnostic, since it works from the compose file / Dockerfile
  contract rather than a language-specific build step.
- **`paws release`**: cross-compiles **`paws` itself** (the Rust binary) for multiple OS/arch —
  this is not a general "build any project for any target" capability, it's specific to `paws`'s
  own release pipeline. Don't read the target matrix here as stack coverage for user projects.
- **Java** is the next gap worth naming specifically, per a real `gh api users/mbround18/repos`
  audit (2026-08-18): after Rust/JS/TS, Java is the next-most-common language across
  `mbround18`'s own non-fork repos (3, all Gradle-based Hytale mods) with no `paws-provision`/
  `paws-audit` precedent yet to build on — unlike Python, which had both before `paws ci
  --toolchain python` was wired.

## Status legend

- ✅ **Supported** — `paws ci`/`paws provision` actually runs this today.
- 🚧 **Partial** — some real support exists (detection, provisioning, or a related toolchain), but
  the specific stack/output isn't fully wired.
- 📋 **Planned** — not started; listed here as a target, not committed to a timeline.

## Web / desktop (JS, Rust, Tauri) stacks

| Stack Permutation | Primary Languages | Package Manager(s) | Core Toolchain / Frameworks | Output Type | Status |
| --- | --- | --- | --- | --- | --- |
| React | JavaScript, TypeScript | npm, yarn, pnpm, bun | Node.js (build env), React, Vite/Webpack | Static Web Assets (HTML/CSS/JS) | ✅ |
| Node | JavaScript, TypeScript | npm, yarn, pnpm, bun | Node.js | NPM Package / Backend Server | 🚧 |
| Rust | Rust | cargo | rustc, Cargo | Native Executable (.exe, ELF, Mach-O) | ✅ |
| Node + React | JavaScript, TypeScript | npm, yarn, pnpm, bun | Node.js, React, Next.js / Express | SSR Web App / Full-stack | ✅ |
| Rust + React | Rust, JS/TS | cargo & (npm/yarn/pnpm/bun) | Rust (Actix/Axum), React | Backend API + Static UI | 🚧 |
| Node + Rust | JS/TS, Rust | npm/yarn/pnpm/bun & cargo | Node.js, Rust, napi-rs or neon | Native Node bindings (.node) | 📋 |
| React + Rust | JS/TS, Rust | npm/yarn/pnpm/bun & cargo | React, Rust, wasm-pack | WebAssembly (.wasm) + React UI | 📋 |
| Tauri + Rust | Rust, HTML/CSS/JS | cargo | Tauri, Rust, OS Webview (WebKit/WebView2) | Desktop App Installer | ✅ |
| Tauri + Node + Rust | Rust, JS/TS | cargo & (npm/yarn/pnpm/bun) | Tauri, Node.js (Sidecar), Rust | Desktop App + Node Backend Process | 📋 |
| Tauri + React + Rust | Rust, JS/TS | cargo & (npm/yarn/pnpm/bun) | Tauri, React, Rust, Vite/Next.js | Desktop App (React UI) | 🚧 |
| Tauri + React + Node + Rust | Rust, JS/TS | cargo & (npm/yarn/pnpm/bun) | Tauri, React, Node.js, Rust | Desktop App + Embedded Node APIs | 📋 |
| Tauri + Android | Rust, JS/TS | cargo & (npm/yarn/pnpm/bun) | Tauri, JDK, Android SDK/NDK | .apk / .aab | 🚧 |
| Tauri + iOS | Rust, JS/TS | cargo & (npm/yarn/pnpm/bun) | Tauri, Xcode, `xcodebuild` | .ipa | 📋 (blocked) |

`paws ci --toolchain tauri` (`crates/paws-tauri`) detects a Tauri project (`src-tauri/tauri.conf.json`)
and runs `<package manager> run tauri build` against a dedicated `builders/tauri-linux` Dockerfile
(Rust + Node + the GTK/WebKit libs Tauri's Linux backend needs), through Dagger like every other
builder — Tauri's own CLI handles the frontend-then-Rust sequencing via `tauri.conf.json`'s
`beforeBuildCommand`, so `paws` never has to. Verified for real, full (non-`--no-bundle`) build —
producing `.deb`/`.rpm`/`.AppImage` — against a real `create-tauri-app` vanilla-ts scaffold
(`examples/tauri-fixture`); marked ✅ only for that plain-TS case. The React/Vue rows should follow
the same code path (the pipeline is package-manager-driven, not framework-driven) but aren't
independently verified yet, hence 🚧 rather than ✅. Linux-only for now — no macOS/Windows Tauri
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
(a GitHub-hosted `macos-*` runner or a self-hosted Mac) wired in as a *different kind* of backend
than the Docker-image-through-Dagger approach every other target here uses — not attempted yet.

`Node + Rust`/`React + Rust` need real new capability, not just wiring: native addon builds
(`napi-rs`/`neon`) and `wasm-pack` WebAssembly builds are each a distinct toolchain `paws` doesn't
drive today, on top of whatever multi-ecosystem provisioning already gets you partway (see
`examples/multi-ecosystem-fixture`).

## JVM / Go stacks

| Stack Permutation | Primary Languages | Package Manager(s) | Core Toolchain / Frameworks | Output Type | Status |
| --- | --- | --- | --- | --- | --- |
| Java | Java | Maven (mvn), Gradle | JDK, Spring Boot, Quarkus | .jar, .war, Docker Image | 📋 |
| Kotlin (JVM) | Kotlin | Gradle, Maven | JDK, kotlinc, Ktor, Spring Boot | .jar, Docker Image | 📋 |
| Java + Kotlin | Java, Kotlin | Gradle | JDK, Mixed Kotlin/Java Compilation | .jar, Maven/Gradle Package | 📋 |
| Go | Go | Go Modules (go mod) | Go Toolchain (go build), standard lib | Native Executables (ELF, .exe, Mach-O) | 📋 |
| Kotlin (Android) | Kotlin | Gradle | Android SDK, Jetpack Compose | .apk, .aab (Android App Bundle) | 📋 |
| Kotlin Multiplatform | Kotlin | Gradle | Kotlin/JVM, Kotlin/Native, Kotlin/Wasm | .jar (JVM), .framework (iOS), .js / .wasm (Web) | 📋 |
| Go + WebAssembly | Go | Go Modules | Go Compiler (GOOS=js GOARCH=wasm) | WebAssembly (.wasm) + JS wrapper | 📋 |
| Go + C/C++ (cgo) | Go, C/C++ | Go Modules, make/cmake | Go Toolchain (cgo), GCC/Clang | Native Executables (dynamically linked) | 📋 |
| Java + React/Node | Java, JS/TS | Maven/Gradle & npm/yarn | JDK, Node.js, Spring Boot, React | Backend .jar + Static Web Assets | 📋 |
| Go + React/Node | Go, JS/TS | Go Modules & npm/yarn | Go Toolchain, Node.js, React | Go Binary + Static Web Assets | 📋 |

`Go`'s `paws-provision` support is the most natural first pickup here — `gh-reusable` already has
a `setupGo` function to port from, and Go's toolchain is a single static binary (`go`), no JVM
version-matrix complexity to design around the way `Java`/`Kotlin` would need.

## Other language ecosystems

| Stack Permutation | Primary Languages | Package Manager(s) | Core Toolchain / Frameworks | Output Type | Status |
| --- | --- | --- | --- | --- | --- |
| Python | Python | uv | CPython, FastAPI, Django | Python Package (.whl), Docker Image | ✅ |
| C# / .NET | C#, F# | NuGet | .NET SDK, ASP.NET Core, EF Core | Binaries (.exe, .dll), NuGet Package | 📋 |
| C / C++ | C, C++ | conan, vcpkg, system pkg mgrs | GCC, Clang, MSVC, CMake, Make | Native Binaries, Libs (.so, .dll, .a) | 📋 |
| Ruby | Ruby | gem, bundler | Ruby (MRI), Ruby on Rails, Sinatra | Gem Package, Docker Image | 📋 |
| PHP | PHP | composer | PHP-FPM, Laravel, Symfony | Source code deploy, Docker Image | 📋 |
| Swift (iOS/macOS) | Swift | Swift Package Manager, CocoaPods | Xcode Command Line Tools, iOS SDK | iOS App (.ipa), macOS App (.app) | 📋 |
| Flutter | Dart | pub | Flutter SDK, Android/iOS SDKs | Mobile (.apk, .aab, .ipa), Web Assets | 📋 |
| Electron + React/Node | JS/TS | npm, yarn, pnpm | Node.js, Electron, React | Desktop App Installers | 📋 |
| .NET Blazor | C#, HTML/CSS | NuGet | .NET SDK, WebAssembly | Wasm Binary + Static Web Assets | 📋 |
| .NET MAUI | C#, XAML | NuGet | .NET SDK, Android/iOS/Mac/Win SDKs | Mobile/Desktop Apps (.apk, .ipa, .msix) | 📋 |
| Python + C/C++ | Python, C/C++ | pip, cmake | CPython, pybind11, Cython | Native Python Extensions (.so, .pyd) | 📋 |
| React Native | JS/TS, Java/Swift | npm, gradle, CocoaPods | Node.js, React Native, Mobile SDKs | Mobile Apps (.apk, .aab, .ipa) | 📋 |
| Zig | Zig | zon (Zig Object Notation) | Zig Compiler | Native Executable (.exe, ELF, Mach-O) | 📋 |

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
   (`crates/paws-cli/src/main.rs`), either against a `gh-reusable` function that already exists
   for it (check `packages/dagger-module/src/index.ts` first — several of the ecosystems above
   already have a `setupX`/`xBuildAndTest` precedent there) or a new native crate, following
   `paws-semver`'s/`paws-audit`'s/`paws-docker`'s precedent of porting real logic rather than
   guessing at it.
4. **Fixtures** — add a real example project under `examples/` (see `examples/README.md`) so the
   new support is tested against something real, not just unit tests of the Rust logic.

None of this needs a new builder image under `./builders/*` unless the stack also needs
cross-compilation support for *`paws` itself* to target — that's a separate concern from `paws`
being able to build *other projects* written in that language.
