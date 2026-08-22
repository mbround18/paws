# Go + React fixture

A plain `net/http` backend (`main.go`) that serves a built React SPA (`frontend/`, a real
`npm create vite -- --template react-ts` scaffold) as static assets under `/`, plus a JSON
`/api/health` route — the "Go Binary + Static Web Assets" output shape from `docs/ROADMAP.md`'s
Go + React/Node row.

Like `examples/rust-react-fixture`, this isn't a single composite pipeline: `paws ci` takes one
`--toolchain` per invocation, so this stack is covered by two independent runs against the same
repo, each exercising its own real toolchain:

```sh
paws ci --toolchain go     # from the repo root — builds/tests the Go backend
paws ci --toolchain node   # from frontend/ — builds the React SPA into frontend/dist
```

Both are verified for real, end to end, through Dagger. There's no `paws`-level orchestration
between them — the backend just expects `frontend/dist` to exist, same as any real deploy — so
this fixture isn't new `paws` capability, it's proof the existing `go`/`node` toolchains compose
cleanly for this stack shape without extra wiring.
