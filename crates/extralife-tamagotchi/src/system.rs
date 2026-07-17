//! System integration for the Tamagotchi P1: the E0C6200 CPU plus the
//! E0C6S46's memory-mapped RAM, display RAM, I/O registers, clock-timer
//! prescaler, input ports, buzzer and interrupt logic.
//!
//! Everything is driven by the emulated 32.768 kHz oscillator (`OSC_HZ`) so the
//! "life clock" is deterministic: we advance a fixed number of oscillator ticks
//! per video frame and derive the timer/interrupt cadence from that count only.
//!
//! Addresses and register semantics are from the Epson E0C6S46 Technical Manual
//! (§2.3 memory map, §7.3 display RAM, §6 input ports, §8.1 clock timer,
//! §10 buzzer). Clean-room; no GPL emulator consulted.

use crate::cpu::{Bus, Cpu, I_FLAG};
use extralife_core::{Button, LoadError};

/// Oscillator frequency (OSC1). The whole machine's time base.
const OSC_HZ: u32 = 32_768;
/// Video frames per second we present. 32 Hz is the LCD frame frequency
/// (fOSC1/1024) but that's too slow to watch; 30 fps is a natural harness rate
/// and keeps the life clock honest since we count real oscillator ticks.
const FPS: u32 = 30;
/// Oscillator ticks advanced per video frame. Each CPU instruction consumes
/// 5/7/12 of these (one "clock" == one OSC1 tick here; the E0C6S46 runs the
/// CPU directly from OSC1 for the P1).
const TICKS_PER_FRAME: u32 = OSC_HZ / FPS;

const ROM_WORDS: usize = 6144;
/// Flat data-memory space (12-bit address). Only mapped regions are real.
const RAM_SIZE: usize = 0x1000;

const W: usize = 32;
const H: usize = 16;

// --- I/O register addresses (E0C6S46 Technical Manual) --------------------
const REG_IT_FLAGS: u16 = 0xF00; // clock-timer interrupt factor flags (R, auto-clear on read)
const REG_IK_FLAGS: u16 = 0xF04; // K00-K03 interrupt factor flag
const REG_IT_MASK: u16 = 0xF10; // clock-timer interrupt mask (EIT1/2/8/32)
const REG_IK0_MASK: u16 = 0xF14; // K00-K03 interrupt mask
const REG_TM_LO: u16 = 0xF20; // TM0-TM3 (128/64/32/16 Hz)
const REG_TM_HI: u16 = 0xF21; // TM4-TM7 (8/4/2/1 Hz)
const REG_K0: u16 = 0xF40; // input port K00-K03 (buttons), active low
const REG_K1: u16 = 0xF42; // input port K10-K13
const REG_BZ: u16 = 0xF54; // R42/R43 buzzer output port
const REG_TIMER_RST: u16 = 0xF76; // TMRST / WDRST (write)

pub struct Tamagotchi {
    cpu: Cpu,
    rom: Vec<u16>,
    mem: [u8; RAM_SIZE],
    /// Free-running 15-bit clock-timer divider off OSC1. Bit 7 = 128 Hz … it
    /// counts OSC1 ticks; TM0..TM7 are the high bits (see `tm_byte`).
    clk_div: u32,
    /// Latched button state (true = pressed). K0 = A/B/C on K00..K02.
    keys: [bool; 3],
    framebuffer: Vec<u8>,
    /// Buzzer: true while the piezo is being driven (R42 low). Exposed as a
    /// single square tone for the audio path.
    buzzer_on: bool,
    audio: Vec<f32>,
}

impl Default for Tamagotchi {
    fn default() -> Self {
        Self::new()
    }
}

/// Bus view over RAM/display/I/O. Reads/writes are 4-bit. Special-cased regs
/// (timer data, input ports, interrupt-flag auto-clear) are handled here so the
/// CPU stays a pure core.
struct SysBus<'a> {
    mem: &'a mut [u8; RAM_SIZE],
    clk_div: u32,
    keys: &'a [bool; 3],
    buzzer_on: &'a mut bool,
    /// Interrupt factor flags accumulated this step; cleared when read.
    it_flags: &'a mut u8,
    ik_flags: &'a mut u8,
}

impl Bus for SysBus<'_> {
    fn read(&mut self, addr: u16) -> u8 {
        let a = addr & 0x0FFF;
        match a {
            REG_TM_LO => tm_byte(self.clk_div) & 0xF,
            REG_TM_HI => (tm_byte(self.clk_div) >> 4) & 0xF,
            REG_K0 => {
                // K00=A, K01=B, K02=C. Input ports read HIGH (1) when the
                // button is open and LOW (0) when pressed (active-low pull-up).
                let mut v = 0xF;
                if self.keys[0] {
                    v &= !0x1;
                }
                if self.keys[1] {
                    v &= !0x2;
                }
                if self.keys[2] {
                    v &= !0x4;
                }
                v & 0xF
            }
            REG_K1 => 0xF,
            REG_IT_FLAGS => {
                let v = *self.it_flags & 0xF;
                *self.it_flags = 0; // factor flags reset on read (§8.1)
                v
            }
            REG_IK_FLAGS => {
                let v = *self.ik_flags & 0xF;
                *self.ik_flags = 0;
                v
            }
            _ => self.mem[a as usize] & 0xF,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        let a = addr & 0x0FFF;
        let v = val & 0xF;
        match a {
            REG_TIMER_RST => {
                // Writing TMRST resets the divider; handled by the caller which
                // owns clk_div. Record intent via a sentinel bit in mem.
                self.mem[a as usize] = v;
            }
            REG_BZ => {
                // R42 drives the buzzer; the P1 pulses it low to sound. Treat a
                // non-1 R42 output as "buzzer active".
                *self.buzzer_on = v & 0x4 == 0;
                self.mem[a as usize] = v;
            }
            _ => self.mem[a as usize] = v,
        }
    }
}

/// Assemble the 8-bit clock-timer value (TM0..TM7 = 128..1 Hz) from the OSC1
/// divider. TM0 (128 Hz) = OSC1 / 256, so it is divider bit 8; TM7 (1 Hz) is
/// bit 15. We expose bits [15:8] as TM0..TM7 with TM0 as the LSB of the byte.
fn tm_byte(div: u32) -> u8 {
    ((div >> 8) & 0xFF) as u8
}

impl Tamagotchi {
    pub fn new() -> Tamagotchi {
        Tamagotchi {
            cpu: Cpu::new(),
            rom: vec![0; ROM_WORDS],
            mem: [0; RAM_SIZE],
            clk_div: 0,
            keys: [false; 3],
            framebuffer: vec![0; W * H * 4],
            buzzer_on: false,
            audio: Vec::new(),
        }
    }

    /// Load a P1 ROM. Format (per TamaTool): 16-bit big-endian words, each
    /// holding one 12-bit instruction in the low bits. 6144 words = 12288 bytes.
    pub fn load_rom_bytes(&mut self, rom: &[u8]) -> Result<(), LoadError> {
        if rom.len() < 2 || !rom.len().is_multiple_of(2) {
            return Err(LoadError::Invalid);
        }
        let words = rom.len() / 2;
        if words > ROM_WORDS {
            return Err(LoadError::Invalid);
        }
        let mut r = vec![0u16; ROM_WORDS];
        for (i, chunk) in rom.chunks_exact(2).enumerate() {
            r[i] = (((chunk[0] as u16) << 8) | chunk[1] as u16) & 0x0FFF;
        }
        // Full reset on load (core-contract): two loads = two clean runs.
        self.cpu = Cpu::new();
        self.mem = [0; RAM_SIZE];
        self.clk_div = 0;
        self.keys = [false; 3];
        self.buzzer_on = false;
        self.audio.clear();
        self.rom = r;
        self.render();
        Ok(())
    }

    pub fn set_button(&mut self, button: Button, pressed: bool) {
        // Three physical buttons: A (Select), B (Execute), C (Cancel).
        // The abstract Button enum has A and B; map C to Select (harness binds
        // a third key to it).
        let idx = match button {
            Button::A => 0,
            Button::B => 1,
            Button::Select => 2,
            _ => return,
        };
        self.keys[idx] = pressed;
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// Advance one video frame: run OSC1 ticks worth of CPU + timer, poll
    /// interrupts between instructions, then render.
    pub fn run_frame(&mut self) {
        let mut it_flags = self.mem[REG_IT_FLAGS as usize] & 0xF;
        let mut ik_flags = self.mem[REG_IK_FLAGS as usize] & 0xF;
        let mut ticks_left = TICKS_PER_FRAME as i64;

        while ticks_left > 0 {
            // Snapshot timer bits before stepping so we can detect edges.
            let before = tm_byte(self.clk_div);

            let cycles = {
                let mut bus = SysBus {
                    mem: &mut self.mem,
                    clk_div: self.clk_div,
                    keys: &self.keys,
                    buzzer_on: &mut self.buzzer_on,
                    it_flags: &mut it_flags,
                    ik_flags: &mut ik_flags,
                };
                self.cpu.step(&self.rom, &mut bus)
            };

            // Handle a TMRST write (F76 bit1) by zeroing the divider.
            if self.mem[REG_TIMER_RST as usize] & 0x2 != 0 {
                self.clk_div = 0;
                self.mem[REG_TIMER_RST as usize] = 0;
            }

            self.clk_div = self.clk_div.wrapping_add(cycles);
            ticks_left -= cycles as i64;

            let after = tm_byte(self.clk_div);
            // Rising edges of the timer bits set the matching interrupt factor.
            // TM0=128Hz(bit0) TM3=16Hz(bit3) TM4=8Hz(bit4) TM7=1Hz(bit7).
            // Clock-timer interrupt sources: 1Hz(TM7), 2Hz(TM6), 8Hz(TM4),
            // 32Hz(TM2) → factor bits IT1/IT2/IT8/IT32 at F00 D0..D3.
            self.accumulate_timer_irq(before, after, &mut it_flags);

            // Poll for a pending, unmasked, enabled interrupt between ops.
            self.mem[REG_IT_FLAGS as usize] = it_flags;
            self.mem[REG_IK_FLAGS as usize] = ik_flags;
            self.service_interrupts(&mut it_flags, &mut ik_flags);
        }

        self.mem[REG_IT_FLAGS as usize] = it_flags;
        self.mem[REG_IK_FLAGS as usize] = ik_flags;

        // Buzzer → a short square-wave burst for the frame if active.
        self.fill_audio();
        self.render();
    }

    fn accumulate_timer_irq(&self, before: u8, after: u8, it_flags: &mut u8) {
        let rose = |bit: u8| (before & bit == 0) && (after & bit != 0);
        // TM2=32Hz→bit2, TM4=8Hz→bit4, TM6=2Hz→bit6, TM7=1Hz→bit7.
        if rose(1 << 2) {
            *it_flags |= 0x8; // IT32
        }
        if rose(1 << 4) {
            *it_flags |= 0x4; // IT8
        }
        if rose(1 << 6) {
            *it_flags |= 0x2; // IT2
        }
        if rose(1 << 7) {
            *it_flags |= 0x1; // IT1
        }
    }

    /// If interrupts are enabled (I flag) and a masked-in factor is pending,
    /// vector to the clock-timer interrupt routine. The E0C6S46 clock-timer
    /// vector is page 1, step 0x0C (bank of the currently-running code).
    fn service_interrupts(&mut self, it_flags: &mut u8, ik_flags: &mut u8) {
        if self.cpu.flags & I_FLAG == 0 {
            return;
        }
        let it_mask = self.mem[REG_IT_MASK as usize] & 0xF;
        let ik_mask = self.mem[REG_IK0_MASK as usize] & 0xF;

        if *it_flags & it_mask != 0 {
            self.vector_to(0x0C, it_flags, ik_flags);
        } else if *ik_flags & ik_mask != 0 {
            self.vector_to(0x02, it_flags, ik_flags); // input-port vector
        }
    }

    fn vector_to(&mut self, step: u8, _it: &mut u8, _ik: &mut u8) {
        // Interrupt: push PC, clear I (DI), jump to (current bank, page1, step).
        let mut bus = SysBus {
            mem: &mut self.mem,
            clk_div: self.clk_div,
            keys: &self.keys,
            buzzer_on: &mut self.buzzer_on,
            it_flags: &mut 0,
            ik_flags: &mut 0,
        };
        self.cpu.interrupt(&mut bus, 1, step);
    }

    fn render(&mut self) {
        // Display RAM → 32×16. Per §7.3, SEG n occupies addresses E00+2n/E01+2n
        // (COM0-3 / COM4-7) and E80+2n/E81+2n (COM8-11 / COM12-15). Each nibble
        // holds 4 COM lines in D0..D3. We map SEG→x (column), COM→y (row).
        for seg in 0..W {
            let base_lo = 0xE00 + seg * 2;
            let base_hi = 0xE80 + seg * 2;
            let coms = [
                self.mem[base_lo] & 0xF,        // COM0-3
                self.mem[base_lo + 1] & 0xF,    // COM4-7
                self.mem[base_hi] & 0xF,        // COM8-11
                self.mem[base_hi + 1] & 0xF,    // COM12-15
            ];
            for (grp, nib) in coms.iter().enumerate() {
                for bit in 0..4 {
                    let com = grp * 4 + bit;
                    if com >= H {
                        continue;
                    }
                    let on = (nib >> bit) & 1 != 0;
                    let idx = (com * W + seg) * 4;
                    let c = if on { 0x00 } else { 0xC8 }; // dark dots on grey LCD
                    self.framebuffer[idx] = c;
                    self.framebuffer[idx + 1] = c;
                    self.framebuffer[idx + 2] = c;
                    self.framebuffer[idx + 3] = 0xFF;
                }
            }
        }
    }

    fn fill_audio(&mut self) {
        self.audio.clear();
        if !self.buzzer_on {
            return;
        }
        // ~4 kHz square tone (P1 buzzer freq is ~4096 Hz) at 32 kHz for one frame.
        let samples = (OSC_HZ / FPS) as usize;
        let period = 8; // 32768/8 ≈ 4096 Hz
        for i in 0..samples {
            let s = if (i / (period / 2)) % 2 == 0 { 0.2 } else { -0.2 };
            self.audio.push(s);
            self.audio.push(s);
        }
    }

    pub fn save_state_bytes(&self) -> Vec<u8> {
        let mut s = Vec::with_capacity(RAM_SIZE + 64);
        s.extend_from_slice(&self.mem);
        s.extend_from_slice(&self.clk_div.to_le_bytes());
        s.extend_from_slice(&self.cpu.serialize());
        s
    }

    pub fn load_state_bytes(&mut self, state: &[u8]) -> Result<(), LoadError> {
        let cpu_len = Cpu::serialized_len();
        if state.len() != RAM_SIZE + 4 + cpu_len {
            return Err(LoadError::Invalid);
        }
        let mut mem = [0u8; RAM_SIZE];
        mem.copy_from_slice(&state[..RAM_SIZE]);
        let mut p = RAM_SIZE;
        let clk = u32::from_le_bytes([state[p], state[p + 1], state[p + 2], state[p + 3]]);
        p += 4;
        let cpu = Cpu::deserialize(&state[p..p + cpu_len]).ok_or(LoadError::Invalid)?;
        self.mem = mem;
        self.clk_div = clk;
        self.cpu = cpu;
        self.render();
        Ok(())
    }
}
