//! Minimal esp-idf-svc "blink" firmware — the `paws ci --toolchain esp32`
//! fixture (see `examples/README.md` and `docs/ROADMAP.md`'s Embedded
//! section). Deliberately trivial: this exists to prove
//! `paws_esp32::is_esp32_project` detects it and that
//! `cargo fmt`/`cargo clippy`/`cargo build --release` genuinely run through
//! Dagger against a real `esp-idf-svc` dependency, not to demonstrate a
//! real application.

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::peripherals::Peripherals;

fn main() -> anyhow::Result<()> {
    // esp-idf-svc's own recommended patching for its (Rust <-> C) linkage
    // step — required for any esp-idf-svc binary to link at all.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let mut led = PinDriver::output(peripherals.pins.gpio8)?;

    loop {
        led.set_high()?;
        FreeRtos::delay_ms(500);
        led.set_low()?;
        FreeRtos::delay_ms(500);
    }
}
