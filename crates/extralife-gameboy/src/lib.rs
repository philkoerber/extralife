//! Game Boy (DMG) core for extralife.
//!
//! Playbook order: CPU headless first (SingleStepTests/sm83), then MMU +
//! cartridge, timer + interrupts, PPU to a 160x144 RGBA framebuffer diffed
//! against dmg-acid2, then the APU to a fixed-rate stereo f32 stream verified
//! against Blargg's `dmg_sound` suite and a golden-audio buffer.
//!
//! Determinism: `step_frame` runs the CPU until the PPU completes one video
//! frame, ticking the APU from the same system clock (no wall-clock, no
//! time-seeded RNG), so the same ROM+inputs render and *sound* identically
//! every run (required for golden-image and golden-audio diffs).

use extralife_core::{Button, Device, LoadError, Screen};

pub mod apu;
pub mod cpu;
mod cartridge;
mod joypad;
mod mmu;
mod ppu;
mod timer;
mod wasm;

use cartridge::Cartridge;
use cpu::Cpu;
use mmu::Mmu;

const SCREEN: Screen = Screen::new(ppu::W as u32, ppu::H as u32);

#[derive(Default)]
pub struct GameBoy {
    cpu: Cpu,
    mmu: Mmu,
    /// Kept so `step_frame` before any `load_rom` is a harmless no-op that still
    /// returns a valid (blank) framebuffer.
    loaded: bool,
}

impl GameBoy {
    /// The bytes the ROM has written to the serial port so far, as UTF-8-lossy
    /// text. Blargg's test ROMs report pass/fail here, so headless tests read it.
    pub fn serial_text(&self) -> String {
        String::from_utf8_lossy(&self.mmu.serial_out).into_owned()
    }

    /// Blargg's memory-mapped test protocol: many of his suites (dmg_sound
    /// among them) write a status byte to $A000, a `$DE,$B0,$61` signature to
    /// $A001-3, and a zero-terminated result string from $A004 — instead of, or
    /// in addition to, the serial port. Returns the result string once the
    /// signature is present and the status byte is a final code (not $80
    /// "running"); `None` while still running or before the signature appears.
    pub fn blargg_mem_result(&self) -> Option<String> {
        if self.mmu.read_raw(0xA001) != 0xDE
            || self.mmu.read_raw(0xA002) != 0xB0
            || self.mmu.read_raw(0xA003) != 0x61
        {
            return None;
        }
        if self.mmu.read_raw(0xA000) == 0x80 {
            return None; // still running
        }
        let mut text = Vec::new();
        let mut addr = 0xA004u16;
        while addr < 0xBFFF {
            let b = self.mmu.read_raw(addr);
            if b == 0 {
                break;
            }
            text.push(b);
            addr += 1;
        }
        Some(String::from_utf8_lossy(&text).into_owned())
    }
}

impl Device for GameBoy {
    fn screen(&self) -> Screen {
        SCREEN
    }

    fn load_rom(&mut self, rom: &[u8]) -> Result<(), LoadError> {
        let cart = Cartridge::new(rom).ok_or(LoadError::Invalid)?;
        self.cpu = Cpu::default();
        self.mmu = Mmu::new(cart);
        self.loaded = true;
        Ok(())
    }

    fn step_frame(&mut self) {
        if !self.loaded {
            return;
        }
        // Fresh audio for this frame; the APU appends resampled stereo pairs as
        // the CPU drives it, and `audio()` returns them until the next frame.
        self.mmu.apu.clear_samples();
        // Run instructions until the PPU signals it finished a frame. The frame
        // is bounded by the PPU's own 154-line cycle, so this always terminates;
        // the cap is a safety net against a runaway (e.g. a stuck LCD-off ROM).
        self.mmu.ppu.frame_ready = false;
        let mut guard = 0u32;
        while !self.mmu.ppu.frame_ready && guard < 200_000 {
            self.cpu.step(&mut self.mmu);
            guard += 1;
        }
    }

    fn set_button(&mut self, button: Button, pressed: bool) {
        self.mmu.joypad.set(button, pressed);
    }

    fn framebuffer(&self) -> &[u8] {
        self.mmu.ppu.framebuffer()
    }

    fn audio(&self) -> &[f32] {
        self.mmu.apu.samples()
    }

    fn sample_rate(&self) -> u32 {
        apu::OUTPUT_RATE
    }

    fn save_state(&self) -> Vec<u8> {
        // Compact hand-rolled blob. Version byte guards format changes.
        let mut s = Vec::new();
        s.push(STATE_VERSION);
        self.cpu.serialize(&mut s);
        self.mmu.serialize(&mut s);
        s
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError> {
        if state.first() != Some(&STATE_VERSION) {
            return Err(LoadError::Invalid);
        }
        // Build into a scratch copy so a malformed blob leaves self untouched.
        let mut cpu = self.cpu;
        let mut mmu = std::mem::take(&mut self.mmu);
        let mut p = 1;
        let ok = cpu.deserialize(state, &mut p) && mmu.deserialize(state, &mut p);
        if !ok {
            // Restore the moved-out MMU and report failure atomically.
            self.mmu = mmu;
            return Err(LoadError::Invalid);
        }
        self.cpu = cpu;
        self.mmu = mmu;
        self.loaded = true;
        Ok(())
    }
}

const STATE_VERSION: u8 = 2;

#[cfg(test)]
mod tests;
