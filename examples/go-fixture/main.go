// Package main is a minimal, deliberately clean module: builds and tests
// successfully, for `paws ci --toolchain go`'s "clean run" acceptance
// scenario, matching examples/rust-fixture's role for `--toolchain rust`.
package main

import "fmt"

func add(a, b int) int {
	return a + b
}

func main() {
	fmt.Println(add(2, 3))
}
