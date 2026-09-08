# python-no-tests-fixture

A `uv` project that declares no `pytest` and has no tests — the shape a real
application repo often has when it only builds a wheel.

`paws ci --toolchain python` must build it and skip the test step. Running
`uv run pytest` here fails with ``Failed to spawn: `pytest` `` after a full
sync and build, which is what this fixture exists to keep from regressing.
