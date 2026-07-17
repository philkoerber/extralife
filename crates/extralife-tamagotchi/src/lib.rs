//! Tamagotchi P1 (Bandai, 1996) core for extralife.
//!
//! The P1 is a single-program device built on Epson's E0C6S46 — a 4-bit MCU
//! with the E0C6200 core (`cpu.rs`), 6144×12-bit mask ROM, 4-bit RAM, a
//! 32×16 dot LCD driven from display RAM, three buttons, and a piezo buzzer.
//! There is no cartridge: the ROM *is* the program.
//!
//! Determinism (core-contract): the "life clock" is driven purely by the
//! emulated 32.768 kHz oscillator counted in CPU clocks — never wall-clock,
//! never time-seeded. Same ROM + same inputs + same frame count ⇒ identical
//! framebuffer.
//!
//! Clean-room: the CPU and peripherals are implemented from the Epson
//! "S1C6200/6200A Core CPU Manual" and "E0C6S46 Technical Manual" only. GPL
//! emulators (TamaLib etc.) were not consulted for code or structure.

use extralife_core::{Button, Device, LoadError, Screen};

mod cpu;
mod system;
mod wasm;

pub use system::Tamagotchi;

/// P1 LCD: 32×16 dots, upscaled 4× to a comfortable window, plus an icon strip.
/// We render the raw 32×16 matrix; the harness scales it. Screen geometry is
/// the raw dot matrix so golden diffs are exact at native resolution.
pub const SCREEN: Screen = Screen::new(32, 16);

impl Device for Tamagotchi {
    fn screen(&self) -> Screen {
        SCREEN
    }

    fn load_rom(&mut self, rom: &[u8]) -> Result<(), LoadError> {
        self.load_rom_bytes(rom)
    }

    fn step_frame(&mut self) {
        self.run_frame();
    }

    fn set_button(&mut self, button: Button, pressed: bool) {
        self.set_button(button, pressed);
    }

    fn framebuffer(&self) -> &[u8] {
        self.framebuffer()
    }

    fn audio(&self) -> &[f32] {
        self.audio()
    }

    fn sample_rate(&self) -> u32 {
        32_768
    }

    fn save_state(&self) -> Vec<u8> {
        self.save_state_bytes()
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError> {
        self.load_state_bytes(state)
    }
}

#[cfg(test)]
mod tests;
