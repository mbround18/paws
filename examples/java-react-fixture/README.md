# Java + React fixture

A plain JDK-only backend (`Server.java`, `com.sun.net.httpserver.HttpServer`, no framework
dependency) serving a built React SPA (`frontend/`, a real `npm create vite -- --template
react-ts` scaffold) as static assets, plus a JSON `/api/health` route — the "Backend .jar +
Static Web Assets" output shape from `docs/ROADMAP.md`'s Java + React/Node row.

Like `examples/rust-react-fixture` and `examples/go-react-fixture`, this isn't a single composite
pipeline: `paws ci` takes one `--toolchain` per invocation, so this stack is covered by two
independent runs against the same repo, each exercising its own real toolchain:

```sh
paws ci --toolchain java   # from the repo root — builds/tests the Java backend via mvnw
paws ci --toolchain node   # from frontend/ — builds the React SPA into frontend/dist
```

Both are verified for real, end to end, through Dagger — `ServerTest` makes a genuine
`java.net.http.HttpClient` round trip against the backend's `/api/health` route. There's no
`paws`-level orchestration between them (the backend just expects `frontend/dist` to exist, same
as any real deploy), so this fixture isn't new `paws` capability, it's proof the existing
`java`/`node` toolchains compose cleanly for this stack shape without extra wiring.
