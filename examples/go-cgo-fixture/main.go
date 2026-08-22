// Package main calls into a small inline C function via cgo. Unlike
// examples/go-wasm-fixture, this needs no special handling in
// crates/paws-go's pipeline: CGO_ENABLED=1 is already the default on the
// golang:1-bookworm image, which already ships gcc, so this fixture exists
// purely to prove the plain (non-wasm) pipeline already builds/tests a real
// cgo package correctly -- not because paws-go branches on cgo at all.
package main

/*
int add(int a, int b) {
	return a + b;
}
*/
import "C"
import "fmt"

func add(a, b int) int {
	return int(C.add(C.int(a), C.int(b)))
}

func main() {
	fmt.Println(add(2, 3))
}
