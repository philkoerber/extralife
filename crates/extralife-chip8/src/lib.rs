//! CHIP-8 core — the pipeline proof for extralife.
//!
//! ponytail: this is a *minimal* CHIP-8, not a complete one. It implements just
//! the opcodes the IBM-logo test ROM needs (CLS, JP, LD Vx, ADD Vx, LD I, DRW)
//! plus a couple of freebies, enough to render a real picture end-to-end:
//! Rust -> WASM -> React canvas. The full ~35-opcode VM (all quirks, timers,
//! keypad, the whole chip8-test-suite green) is the dedicated CHIP-8 session.
//! Upgrade path: fill in the `_ =>` arm of `execute` opcode by opcode.

use extralife_core::{Button, Device, LoadError, Screen};

mod wasm;

const SCREEN: Screen = Screen::new(64, 32);
const W: usize = 64;
const H: usize = 32;
const MEM: usize = 4096;
const ROM_START: usize = 0x200;
/// CHIP-8 runs ~classic 700Hz; at 60fps that's ~11 instructions per frame.
const CYCLES_PER_FRAME: u32 = 11;

/// Standard low-res font, loaded at 0x50. Needed by ROMs that draw digits;
/// harmless for IBM logo. Kept because it's five lines and every CHIP-8 has it.
#[rustfmt::skip]
const FONT: [u8; 80] = [
    0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
    0x20, 0x60, 0x20, 0x20, 0x70, // 1
    0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
    0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
    0x90, 0x90, 0xF0, 0x10, 0x10, // 4
    0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
    0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
    0xF0, 0x10, 0x20, 0x40, 0x40, // 7
    0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
    0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
    0xF0, 0x90, 0xF0, 0x90, 0x90, // A
    0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
    0xF0, 0x80, 0x80, 0x80, 0xF0, // C
    0xE0, 0x90, 0x90, 0x90, 0xE0, // D
    0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
    0xF0, 0x80, 0xF0, 0x80, 0x80, // F
];

pub struct Chip8 {
    mem: [u8; MEM],
    v: [u8; 16],
    i: u16,
    pc: u16,
    /// 1 bit per pixel; expanded to RGBA in `framebuffer`.
    pixels: [bool; W * H],
    /// RGBA8888 view, rebuilt from `pixels` after each frame.
    framebuffer: Vec<u8>,
}

impl Default for Chip8 {
    fn default() -> Self {
        let mut c = Self {
            mem: [0; MEM],
            v: [0; 16],
            i: 0,
            pc: ROM_START as u16,
            pixels: [false; W * H],
            framebuffer: vec![0; SCREEN.framebuffer_len()],
        };
        c.mem[0x50..0x50 + FONT.len()].copy_from_slice(&FONT);
        c
    }
}

impl Chip8 {
    fn reset(&mut self) {
        let font = self.mem[0x50..0x50 + FONT.len()].to_vec();
        *self = Self::default();
        self.mem[0x50..0x50 + FONT.len()].copy_from_slice(&font);
    }

    fn execute(&mut self, op: u16) {
        let nnn = op & 0x0FFF;
        let nn = (op & 0x00FF) as u8;
        let n = (op & 0x000F) as usize;
        let x = ((op & 0x0F00) >> 8) as usize;
        let y = ((op & 0x00F0) >> 4) as usize;

        match op & 0xF000 {
            0x0000 if op == 0x00E0 => self.pixels = [false; W * H], // CLS
            0x1000 => self.pc = nnn,                                // JP nnn
            0x6000 => self.v[x] = nn,                               // LD Vx, nn
            0x7000 => self.v[x] = self.v[x].wrapping_add(nn),       // ADD Vx, nn
            0xA000 => self.i = nnn,                                 // LD I, nnn
            0xD000 => self.draw(x, y, n),                           // DRW Vx, Vy, n
            // ponytail: everything else is a no-op for now. The full VM fills
            // these in (arithmetic, control flow, timers, keypad, quirks).
            _ => {}
        }
    }

    /// XOR-sprite draw with collision flag and edge wrapping of the origin.
    fn draw(&mut self, x: usize, y: usize, n: usize) {
        let ox = self.v[x] as usize % W;
        let oy = self.v[y] as usize % H;
        self.v[0xF] = 0;
        for row in 0..n {
            let sprite = self.mem[(self.i as usize + row) % MEM];
            let py = oy + row;
            if py >= H {
                break; // sprites clip at the bottom (no vertical wrap)
            }
            for bit in 0..8 {
                if (sprite >> (7 - bit)) & 1 == 0 {
                    continue;
                }
                let px = ox + bit;
                if px >= W {
                    break;
                }
                let idx = py * W + px;
                if self.pixels[idx] {
                    self.v[0xF] = 1;
                }
                self.pixels[idx] ^= true;
            }
        }
    }

    fn render(&mut self) {
        for (i, &on) in self.pixels.iter().enumerate() {
            let c = if on { 0xFF } else { 0x00 };
            let o = i * 4;
            self.framebuffer[o] = c;
            self.framebuffer[o + 1] = c;
            self.framebuffer[o + 2] = c;
            self.framebuffer[o + 3] = 0xFF;
        }
    }
}

impl Device for Chip8 {
    fn screen(&self) -> Screen {
        SCREEN
    }

    fn load_rom(&mut self, rom: &[u8]) -> Result<(), LoadError> {
        if rom.is_empty() || ROM_START + rom.len() > MEM {
            return Err(LoadError::Invalid);
        }
        self.reset();
        self.mem[ROM_START..ROM_START + rom.len()].copy_from_slice(rom);
        self.render();
        Ok(())
    }

    fn step_frame(&mut self) {
        for _ in 0..CYCLES_PER_FRAME {
            let pc = self.pc as usize;
            let op = ((self.mem[pc] as u16) << 8) | self.mem[(pc + 1) % MEM] as u16;
            self.pc = self.pc.wrapping_add(2);
            self.execute(op);
        }
        self.render();
    }

    fn set_button(&mut self, _button: Button, _pressed: bool) {}

    fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    fn save_state(&self) -> Vec<u8> {
        // ponytail: framebuffer-only snapshot for now; the full VM serializes
        // mem/registers/pc so rewind actually resumes execution.
        self.framebuffer.clone()
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError> {
        if state.len() != SCREEN.framebuffer_len() {
            return Err(LoadError::Invalid);
        }
        self.framebuffer.copy_from_slice(state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_framebuffer_and_state_contract() {
        let mut c = Chip8::default();
        assert!(c.load_rom(&[0x00, 0xE0]).is_ok());
        assert_eq!(c.framebuffer().len(), SCREEN.framebuffer_len());
        assert!(c.load_rom(&[]).is_err(), "empty ROM must be rejected");

        let snapshot = c.save_state();
        assert!(c.load_state(&snapshot).is_ok());
        assert!(c.load_state(&[1, 2, 3]).is_err());
    }

    /// The end-to-end check: the real IBM-logo ROM must draw *something*.
    /// A blank screen after running it means the CPU/DRW path is broken.
    #[test]
    fn ibm_logo_rom_draws_pixels() {
        let rom = include_bytes!(
            "../../../tests/roms/chip8-test-suite/bin/2-ibm-logo.ch8"
        );
        let mut c = Chip8::default();
        c.load_rom(rom).unwrap();
        for _ in 0..20 {
            c.step_frame();
        }
        let lit = c.framebuffer().chunks(4).filter(|p| p[0] > 0).count();
        assert!(lit > 100, "IBM logo should light many pixels, got {lit}");
    }
}
