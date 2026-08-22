.PHONY: build release fixtures

PARALLEL ?= 4

build:
	cargo build --workspace

release:
	cargo build --workspace --release

# Runs the full fixture gauntlet (scripts/run-fixtures.sh) for real, through
# Dagger, against every example under examples/ -- depends on release so
# it's always exercising the binary the gauntlet's failure/success claims
# are actually about. Override concurrency with `make fixtures PARALLEL=8`.
fixtures: release
	PARALLEL=$(PARALLEL) ./scripts/run-fixtures.sh
