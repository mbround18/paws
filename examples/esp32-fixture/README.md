# esp32-fixture

A minimal, structurally-correct `esp-idf-svc` "blink" project — the target for
`paws ci --toolchain esp32` (`crates/paws-esp32`, `specs/007-esp32-toolchain`).

This is deliberately stripped down from a real device project (see
`mbround18/ha-kiosk`'s `firmware/`, this feature's first concrete driver) to just enough to be
detected and built:

- `Cargo.toml` — depends on `esp-idf-svc`, the signal `paws_esp32::is_esp32_project` looks for.
- `.cargo/config.toml` — sets `build.target` to `riscv32imc-esp-espidf` (an ESP32-C3 target; the
  same detection signal also fires on any other `*-espidf` triple, e.g. the Xtensa
  `xtensa-esp32*-espidf` family).
- `rust-toolchain.toml` — pins the `esp` channel `espup install` provisions.
- `sdkconfig.defaults` — the minimal ESP-IDF config override a real esp-idf-svc "hello world"
  typically needs (a larger main-task stack than ESP-IDF's own default).
- `build.rs` — the standard `embuild` glue every `esp-idf-sys`/`esp-idf-svc` project needs.

It does not compile against a real ESP-IDF toolchain in `paws`'s own dev/CI sandbox (no ESP
toolchain is installed there) — it's a structurally correct fixture for `is_esp32_project`
detection and for `paws ci --toolchain esp32` to build for real once run through
`builders/esp32` (which does have the full `espup`-installed toolchain), the same "verified for
real, end to end" bar every other fixture in this directory is held to.
