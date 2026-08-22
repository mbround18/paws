// Package main is a plain net/http backend serving a built React SPA
// (frontend/dist) as static assets, plus a JSON /api/health route -- the
// "Go Binary + Static Web Assets" output shape from docs/ROADMAP.md's
// Go + React/Node row. Like examples/rust-react-fixture, this isn't a
// composite pipeline: paws ci --toolchain go (this package) and paws ci
// --toolchain node (frontend/) are two independent runs against the same
// repo, proving the existing go/node toolchains already compose cleanly
// for this shape with no new paws capability needed.
package main

import (
	"encoding/json"
	"net/http"
)

func newMux() *http.ServeMux {
	mux := http.NewServeMux()
	mux.HandleFunc("/api/health", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("content-type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
	})
	mux.Handle("/", http.FileServer(http.Dir("frontend/dist")))
	return mux
}

func main() {
	if err := http.ListenAndServe(":3000", newMux()); err != nil {
		panic(err)
	}
}
