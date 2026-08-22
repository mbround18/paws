# Rust + React fixture

An Axum backend (`src/main.rs`) that serves a built React SPA (`frontend/`, a real
`npm create vite -- --template react-ts` scaffold) as static assets under `/`, plus a JSON
`/api/health` route — the "Backend API + Static UI" output shape from `docs/ROADMAP.md`'s
Rust + React row.

Unlike Tauri, this isn't a single composite pipeline: `paws ci` takes one `--toolchain` per
invocation, so this stack is covered by two independent runs against the same repo, each
exercising its own real toolchain:

```sh
paws ci --toolchain rust   # from the repo root — builds/tests the Axum backend
paws ci --toolchain node   # from frontend/ — builds the React SPA into frontend/dist
```

Both are verified for real, end to end, through Dagger. There's no `paws`-level orchestration
between them (same as any real deploy: the backend just expects `frontend/dist` to exist), so
this fixture isn't a new capability — it's proof the existing `rust`/`node` toolchains compose
cleanly for this stack shape without extra wiring.
