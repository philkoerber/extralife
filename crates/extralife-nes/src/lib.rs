//! NES / Famicom core for extralife.
//!
//! Playbook order: CPU headless first (the 2A03's 6502, validated against the
//! nes6502 SingleStepTests and nestest), then the cartridge + mappers, the PPU
//! to a 256x240 RGBA framebuffer, the APU, and finally the WASM wrap.
//!
//! Determinism: `step_frame` runs the CPU until the PPU signals a completed
//! video frame, clocking the PPU (3 dots/cycle) and APU (1/cycle) from the same
//! system clock inside the CPU's bus callbacks — no wall-clock, no time-seeded
//! RNG — so the same ROM + inputs render and sound identically every run.

use extralife_core::{Button, Device, LoadError, Screen};

pub mod apu;
pub mod cartridge;
pub mod cpu;
pub mod ppu;
mod system;
mod wasm;

use cartridge::Cartridge;
use cpu::Cpu;
use system::System;

const SCREEN: Screen = Screen::new(ppu::W as u32, ppu::H as u32);

#[derive(Default)]
pub struct Nes {
    cpu: Cpu,
    sys: Option<System>,
}

impl Nes {
    /// Total CPU cycles executed since load (used by the nestest log diff).
    pub fn cycles(&self) -> u64 {
        self.sys.as_ref().map(|s| s.cycles).unwrap_or(0)
    }

    /// Read a CPU-space byte with no side effects (test/log helper).
    pub fn peek(&self, addr: u16) -> u8 {
        self.sys.as_ref().map(|s| s.peek(addr)).unwrap_or(0)
    }

    /// Direct CPU access (test/log helper); None before a ROM is loaded.
    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Step exactly one CPU instruction. Returns false if no ROM is loaded.
    /// Exposed for the nestest log-diff harness.
    pub fn step_instruction(&mut self) -> bool {
        match &mut self.sys {
            Some(sys) => {
                self.cpu.step(sys);
                true
            }
            None => false,
        }
    }

    /// Force the CPU PC (nestest starts in automation mode at $C000).
    pub fn set_pc(&mut self, pc: u16) {
        self.cpu.pc = pc;
    }
}

impl Device for Nes {
    fn screen(&self) -> Screen {
        SCREEN
    }

    fn load_rom(&mut self, rom: &[u8]) -> Result<(), LoadError> {
        let cart = Cartridge::new(rom).ok_or(LoadError::Invalid)?;
        let mut sys = System::new(cart);
        let mut cpu = Cpu::default();
        cpu.reset(&mut sys);
        // Zero the cycle counter *after* reset so callers count execution
        // cycles from the first fetched instruction (the nestest log's CYC
        // column starts at 7, the reset cost, which the harness adds back).
        sys.cycles = 0;
        self.cpu = cpu;
        self.sys = Some(sys);
        Ok(())
    }

    fn step_frame(&mut self) {
        let Some(sys) = &mut self.sys else {
            return;
        };
        sys.apu.clear_samples();
        sys.ppu.frame_ready = false;
        // The PPU completes a frame within its 262-line cycle; the guard is a
        // safety net against a runaway ROM (e.g. rendering permanently disabled).
        let mut guard = 0u32;
        while !sys.ppu.frame_ready && guard < 4_000_000 {
            self.cpu.step(sys);
            guard += 1;
        }
    }

    fn set_button(&mut self, button: Button, pressed: bool) {
        if let Some(sys) = &mut self.sys {
            sys.set_button(button, pressed);
        }
    }

    fn framebuffer(&self) -> &[u8] {
        match &self.sys {
            Some(sys) => sys.ppu.framebuffer(),
            None => &BLANK,
        }
    }

    fn audio(&self) -> &[f32] {
        match &self.sys {
            Some(sys) => sys.apu.samples(),
            None => &[],
        }
    }

    fn sample_rate(&self) -> u32 {
        apu::OUTPUT_RATE
    }

    fn save_state(&self) -> Vec<u8> {
        // ponytail: save-state is a minimal stub (version byte only) until the
        // full system serializer lands; load_state accepts only this shape so a
        // round-trip on the same version is a no-op restore. Ceiling: no rewind
        // yet. Upgrade path: serialize cpu + ram + ppu + apu + mapper state.
        vec![STATE_VERSION]
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError> {
        if state.first() != Some(&STATE_VERSION) {
            return Err(LoadError::Invalid);
        }
        Ok(())
    }
}

const STATE_VERSION: u8 = 1;

/// A blank framebuffer returned before any ROM is loaded.
static BLANK: [u8; SCREEN.framebuffer_len()] = [0; SCREEN.framebuffer_len()];

#[cfg(test)]
mod tests;
