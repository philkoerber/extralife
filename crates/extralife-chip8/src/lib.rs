//! CHIP-8 core for extralife — complete classic (Cosmac VIP) interpreter.
//!
//! Targets plain CHIP-8 semantics as validated by Timendus' chip8-test-suite:
//! vF-reset, memory-increment, display-wait and clipping quirks all ON, shifts
//! read vY, `Bnnn` uses v0. SUPER-CHIP/XO-CHIP (hires, scrolling) is out of
//! scope for this core.
//!
//! Determinism: `step_frame` runs a fixed cycle budget and ticks the 60Hz
//! timers once. No wall-clock, no time-seeded RNG (the `Cxnn` RNG is a seeded
//! xorshift so the same ROM+inputs render identically every run — required for
//! golden-image diffs).

use extralife_core::{Button, Device, LoadError, Screen};

mod wasm;

const SCREEN: Screen = Screen::new(64, 32);
const W: usize = 64;
const H: usize = 32;
const MEM: usize = 4096;
const ROM_START: usize = 0x200;
const FONT_START: usize = 0x50;
/// Cosmac VIP ran ~700–1000 instructions/sec; 15/frame @60fps ≈ 900Hz. High
/// enough that the quirks test doesn't report SLOW, low enough to stay classic.
const CYCLES_PER_FRAME: u32 = 15;

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

/// Maps the 12 abstract `Button`s onto the 16-key CHIP-8 hex pad. The test-suite
/// menus use 1/2/3 to pick and E/F/A to navigate, so those are reachable; the
/// rest fill a natural game layout.
fn key_index(b: Button) -> usize {
    match b {
        Button::Up => 0x2,
        Button::Down => 0x8,
        Button::Left => 0x4,
        Button::Right => 0x6,
        Button::A => 0x5, // action / "select" in menus is A(0xA), but 5 is the classic center
        Button::B => 0xB,
        Button::X => 0x1,
        Button::Y => 0x3,
        Button::L => 0xA,
        Button::R => 0x0,
        Button::Start => 0xE,
        Button::Select => 0xF,
    }
}

pub struct Chip8 {
    mem: [u8; MEM],
    v: [u8; 16],
    i: u16,
    pc: u16,
    stack: Vec<u16>,
    delay: u8,
    sound: u8,
    keys: [bool; 16],
    pixels: [bool; W * H],
    framebuffer: Vec<u8>,
    rng: u32,
    /// `Fx0A` halts execution until a key is pressed then released.
    waiting_key: Option<usize>,
    /// Display-wait quirk: at most one DRW takes effect per frame.
    drew_this_frame: bool,
}

impl Default for Chip8 {
    fn default() -> Self {
        let mut c = Self {
            mem: [0; MEM],
            v: [0; 16],
            i: 0,
            pc: ROM_START as u16,
            stack: Vec::with_capacity(16),
            delay: 0,
            sound: 0,
            keys: [false; 16],
            pixels: [false; W * H],
            framebuffer: vec![0; SCREEN.framebuffer_len()],
            rng: 0x1234_5678,
            waiting_key: None,
            drew_this_frame: false,
        };
        c.mem[FONT_START..FONT_START + FONT.len()].copy_from_slice(&FONT);
        c
    }
}

impl Chip8 {
    fn next_rand(&mut self) -> u8 {
        // xorshift32: deterministic, good enough for a `Cxnn` mask.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0xFF) as u8
    }

    fn fetch(&mut self) -> u16 {
        let pc = self.pc as usize;
        let op = ((self.mem[pc] as u16) << 8) | self.mem[(pc + 1) % MEM] as u16;
        self.pc = self.pc.wrapping_add(2);
        op
    }

    fn execute(&mut self, op: u16) {
        let nnn = op & 0x0FFF;
        let nn = (op & 0x00FF) as u8;
        let n = (op & 0x000F) as usize;
        let x = ((op & 0x0F00) >> 8) as usize;
        let y = ((op & 0x00F0) >> 4) as usize;

        match op & 0xF000 {
            0x0000 => match op {
                0x00E0 => self.pixels = [false; W * H],       // CLS
                0x00EE => self.pc = self.stack.pop().unwrap_or(ROM_START as u16), // RET
                _ => {}                                        // 0nnn (SYS) ignored
            },
            0x1000 => self.pc = nnn,                           // JP nnn
            0x2000 => {                                         // CALL nnn
                self.stack.push(self.pc);
                self.pc = nnn;
            }
            0x3000 => self.skip_if(self.v[x] == nn),           // SE Vx, nn
            0x4000 => self.skip_if(self.v[x] != nn),           // SNE Vx, nn
            0x5000 if n == 0 => self.skip_if(self.v[x] == self.v[y]), // SE Vx, Vy
            0x6000 => self.v[x] = nn,                          // LD Vx, nn
            0x7000 => self.v[x] = self.v[x].wrapping_add(nn),  // ADD Vx, nn
            0x8000 => self.arithmetic(x, y, n),
            0x9000 if n == 0 => self.skip_if(self.v[x] != self.v[y]), // SNE Vx, Vy
            0xA000 => self.i = nnn,                            // LD I, nnn
            0xB000 => self.pc = nnn.wrapping_add(self.v[0] as u16), // JP V0, nnn
            0xC000 => {                                        // RND Vx, nn
                let r = self.next_rand();
                self.v[x] = r & nn;
            }
            0xD000 => self.draw(x, y, n),                      // DRW Vx, Vy, n
            0xE000 => match nn {
                0x9E => self.skip_if(self.keys[self.v[x] as usize & 0xF]),  // SKP
                0xA1 => self.skip_if(!self.keys[self.v[x] as usize & 0xF]), // SKNP
                _ => {}
            },
            0xF000 => self.misc(x, nn),
            _ => {}
        }
    }

    fn skip_if(&mut self, cond: bool) {
        if cond {
            self.pc = self.pc.wrapping_add(2);
        }
    }

    fn arithmetic(&mut self, x: usize, y: usize, n: usize) {
        let vx = self.v[x];
        let vy = self.v[y];
        match n {
            0x0 => self.v[x] = vy,
            // vF-reset quirk: logical ops clear vF (Cosmac VIP behavior).
            0x1 => {
                self.v[x] = vx | vy;
                self.v[0xF] = 0;
            }
            0x2 => {
                self.v[x] = vx & vy;
                self.v[0xF] = 0;
            }
            0x3 => {
                self.v[x] = vx ^ vy;
                self.v[0xF] = 0;
            }
            0x4 => {
                let (r, carry) = vx.overflowing_add(vy);
                self.v[x] = r;
                self.v[0xF] = carry as u8;
            }
            0x5 => {
                let (r, borrow) = vx.overflowing_sub(vy);
                self.v[x] = r;
                self.v[0xF] = (!borrow) as u8;
            }
            0x6 => {
                // Shift quirk (classic): shifts vY, stores in vX.
                let bit = vy & 1;
                self.v[x] = vy >> 1;
                self.v[0xF] = bit;
            }
            0x7 => {
                let (r, borrow) = vy.overflowing_sub(vx);
                self.v[x] = r;
                self.v[0xF] = (!borrow) as u8;
            }
            0xE => {
                let bit = (vy >> 7) & 1;
                self.v[x] = vy << 1;
                self.v[0xF] = bit;
            }
            _ => {}
        }
    }

    fn misc(&mut self, x: usize, nn: u8) {
        match nn {
            0x07 => self.v[x] = self.delay,       // LD Vx, DT
            0x15 => self.delay = self.v[x],       // LD DT, Vx
            0x18 => self.sound = self.v[x],       // LD ST, Vx
            0x1E => self.i = self.i.wrapping_add(self.v[x] as u16), // ADD I, Vx
            0x0A => self.waiting_key = Some(x),   // LD Vx, K (halt handled in step)
            0x29 => self.i = (FONT_START + (self.v[x] as usize & 0xF) * 5) as u16, // LD F, Vx
            0x33 => {                             // LD B, Vx (BCD)
                let val = self.v[x];
                let i = self.i as usize;
                self.mem[i % MEM] = val / 100;
                self.mem[(i + 1) % MEM] = (val / 10) % 10;
                self.mem[(i + 2) % MEM] = val % 10;
            }
            0x55 => {                             // LD [I], Vx  (increments I)
                for r in 0..=x {
                    self.mem[(self.i as usize + r) % MEM] = self.v[r];
                }
                self.i = self.i.wrapping_add(x as u16 + 1);
            }
            0x65 => {                             // LD Vx, [I]  (increments I)
                for r in 0..=x {
                    self.v[r] = self.mem[(self.i as usize + r) % MEM];
                }
                self.i = self.i.wrapping_add(x as u16 + 1);
            }
            _ => {}
        }
    }

    /// XOR sprite draw. Origin wraps (x%64, y%32); pixels clip at the edges.
    fn draw(&mut self, x: usize, y: usize, n: usize) {
        let ox = self.v[x] as usize % W;
        let oy = self.v[y] as usize % H;
        self.v[0xF] = 0;
        for row in 0..n {
            let py = oy + row;
            if py >= H {
                break;
            }
            let sprite = self.mem[(self.i as usize + row) % MEM];
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

    /// Test-suite ROMs can be forced past their menus by writing the selection
    /// into 0x1FF. We default to CHIP-8 (1); callers may override via load_rom.
    fn autoselect_chip8(&mut self) {
        self.mem[0x1FF] = 1;
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
        let rng = self.rng;
        *self = Self::default();
        self.rng = rng; // preserve seed across loads; still deterministic per seed
        self.mem[ROM_START..ROM_START + rom.len()].copy_from_slice(rom);
        self.autoselect_chip8();
        self.render();
        Ok(())
    }

    fn step_frame(&mut self) {
        self.drew_this_frame = false;
        for _ in 0..CYCLES_PER_FRAME {
            // Fx0A: halt until a key is pressed, then released.
            if let Some(x) = self.waiting_key {
                if let Some(k) = (0..16).find(|&k| self.keys[k]) {
                    self.v[x] = k as u8;
                    // Wait for release before resuming (approximate: next frame).
                    if !self.keys[k] {
                        self.waiting_key = None;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            let op = self.fetch();
            // Display-wait quirk: a second DRW in the same frame stalls until
            // the next vblank. Rewind PC and stop cycling this frame.
            if op & 0xF000 == 0xD000 && self.drew_this_frame {
                self.pc = self.pc.wrapping_sub(2);
                break;
            }
            if op & 0xF000 == 0xD000 {
                self.drew_this_frame = true;
            }
            self.execute(op);
        }
        if self.delay > 0 {
            self.delay -= 1;
        }
        if self.sound > 0 {
            self.sound -= 1;
        }
        self.render();
    }

    fn set_button(&mut self, button: Button, pressed: bool) {
        let k = key_index(button);
        // Fx0A resumes on release of the awaited key.
        if !pressed {
            if let Some(x) = self.waiting_key {
                if self.keys[k] {
                    self.v[x] = k as u8;
                    self.waiting_key = None;
                }
            }
        }
        self.keys[k] = pressed;
    }

    fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    fn save_state(&self) -> Vec<u8> {
        let mut s = Vec::with_capacity(MEM + 64);
        s.extend_from_slice(&self.mem);
        s.extend_from_slice(&self.v);
        s.extend_from_slice(&self.i.to_le_bytes());
        s.extend_from_slice(&self.pc.to_le_bytes());
        s.push(self.delay);
        s.push(self.sound);
        s.push(self.stack.len() as u8);
        for &f in &self.stack {
            s.extend_from_slice(&f.to_le_bytes());
        }
        for &p in &self.pixels {
            s.push(p as u8);
        }
        s
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError> {
        // Minimum: mem + regs + i + pc + delay + sound + stacklen.
        if state.len() < MEM + 16 + 2 + 2 + 3 {
            return Err(LoadError::Invalid);
        }
        let mut c = Chip8::default();
        let mut p = 0;
        c.mem.copy_from_slice(&state[p..p + MEM]);
        p += MEM;
        c.v.copy_from_slice(&state[p..p + 16]);
        p += 16;
        c.i = u16::from_le_bytes([state[p], state[p + 1]]);
        p += 2;
        c.pc = u16::from_le_bytes([state[p], state[p + 1]]);
        p += 2;
        c.delay = state[p];
        c.sound = state[p + 1];
        let slen = state[p + 2] as usize;
        p += 3;
        if state.len() < p + slen * 2 + W * H {
            return Err(LoadError::Invalid);
        }
        for _ in 0..slen {
            c.stack.push(u16::from_le_bytes([state[p], state[p + 1]]));
            p += 2;
        }
        for px in c.pixels.iter_mut() {
            *px = state[p] != 0;
            p += 1;
        }
        c.rng = self.rng;
        c.render();
        *self = c;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
