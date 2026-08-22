// Package main is a minimal Go/WebAssembly module: registers a JS-callable
// `add` function via `syscall/js`, the target for `paws ci --toolchain go`'s
// wasm detection (`crates/paws_go::is_wasm_project`) and GOOS=js/GOARCH=wasm
// build path. Not runnable via plain `go run`/`go test` — that's the whole
// point of this fixture (a wasm binary can't execute on the host), matching
// examples/rust-fixture's role for the plain (non-wasm) `--toolchain go` case.
package main

import "syscall/js"

func add(this js.Value, args []js.Value) any {
	return args[0].Int() + args[1].Int()
}

func main() {
	js.Global().Set("add", js.FuncOf(add))
	select {}
}
