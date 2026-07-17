//! Sharp SM83 CPU (the Game Boy's CPU: an 8080/Z80 hybrid).
//!
//! Cycle accuracy is at the M-cycle (machine cycle = 4 T-cycles) granularity,
//! which is what SingleStepTests/sm83 validates: every memory access is one
//! M-cycle with a recorded (address, value, kind), and internal delays are
//! extra M-cycles with no bus access. The CPU drives the rest of the system
//! by calling `Bus::tick`-style hooks on every M-cycle, so the PPU/timer stay
//! in lockstep with instruction timing.
//!
//! Implemented from Pandocs (https://gbdev.io/pandocs/) and the public SM83
//! opcode tables (https://gbdev.io/gb-opcodes/optables/). No emulator source
//! was translated — clean-room per the license policy.

/// The memory/IO seam the CPU drives. Each `read`/`write` is exactly one
/// M-cycle; `tick` is an internal M-cycle with no memory access. Implementors
/// advance any per-cycle hardware (timer, PPU, DMA) inside these calls so
/// timing is exact.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// One internal M-cycle: no memory access, but time still passes.
    fn tick(&mut self);
    /// The pending interrupt lines: `IE & IF & 0x1F`. Nonzero means an
    /// interrupt is requested (and, if IME, should be serviced).
    fn pending_interrupts(&self) -> u8;
    /// Acknowledge (clear) the IF bit for the serviced interrupt.
    fn ack_interrupt(&mut self, bit: u8);
}

/// Flag register bit positions.
const FLAG_Z: u8 = 0x80;
const FLAG_N: u8 = 0x40;
const FLAG_H: u8 = 0x20;
const FLAG_C: u8 = 0x10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    /// Interrupt Master Enable.
    pub ime: bool,
    /// `EI` enables interrupts *after the next instruction* — this defers it.
    pub ime_pending: bool,
    /// HALT low-power state, resumed by a pending interrupt.
    pub halted: bool,
    /// The infamous HALT bug: HALT with IME=0 and a pending interrupt fails to
    /// increment PC on the next fetch.
    pub halt_bug: bool,
}

impl Cpu {
    /// Append CPU state to a save-state blob.
    pub(crate) fn serialize(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[self.a, self.f, self.b, self.c, self.d, self.e, self.h, self.l]);
        out.extend_from_slice(&self.sp.to_le_bytes());
        out.extend_from_slice(&self.pc.to_le_bytes());
        out.push(
            (self.ime as u8)
                | (self.ime_pending as u8) << 1
                | (self.halted as u8) << 2
                | (self.halt_bug as u8) << 3,
        );
    }

    /// Restore CPU state; advances `p`. Returns false if the blob is too short.
    pub(crate) fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 13 {
            return false;
        }
        let b = &s[*p..*p + 13];
        [self.a, self.f, self.b, self.c, self.d, self.e, self.h, self.l] =
            [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
        self.sp = u16::from_le_bytes([b[8], b[9]]);
        self.pc = u16::from_le_bytes([b[10], b[11]]);
        let flags = b[12];
        self.ime = flags & 1 != 0;
        self.ime_pending = flags & 2 != 0;
        self.halted = flags & 4 != 0;
        self.halt_bug = flags & 8 != 0;
        *p += 13;
        true
    }
}

impl Default for Cpu {
    fn default() -> Self {
        // Post-boot DMG register values (Pandocs "Power-Up Sequence").
        Self {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
            ime: false,
            ime_pending: false,
            halted: false,
            halt_bug: false,
        }
    }
}

impl Cpu {
    fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }
    fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    fn set_bc(&mut self, v: u16) {
        [self.b, self.c] = v.to_be_bytes();
    }
    fn set_de(&mut self, v: u16) {
        [self.d, self.e] = v.to_be_bytes();
    }
    fn set_hl(&mut self, v: u16) {
        [self.h, self.l] = v.to_be_bytes();
    }
    fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = (v & 0xF0) as u8; // low nibble of F is always zero
    }

    fn flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }
    fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }
    fn set_flags(&mut self, z: bool, n: bool, h: bool, c: bool) {
        self.f = (z as u8) << 7 | (n as u8) << 6 | (h as u8) << 5 | (c as u8) << 4;
    }

    // --- primitive bus operations (each is one M-cycle) --------------------

    fn read(&mut self, bus: &mut impl Bus, addr: u16) -> u8 {
        bus.read(addr)
    }
    fn write(&mut self, bus: &mut impl Bus, addr: u16, val: u8) {
        bus.write(addr, val);
    }

    fn fetch8(&mut self, bus: &mut impl Bus) -> u8 {
        let v = self.read(bus, self.pc);
        // HALT bug: the byte after HALT is read twice (PC not incremented once).
        if self.halt_bug {
            self.halt_bug = false;
        } else {
            self.pc = self.pc.wrapping_add(1);
        }
        v
    }

    fn fetch16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch8(bus);
        let hi = self.fetch8(bus);
        u16::from_le_bytes([lo, hi])
    }

    fn push(&mut self, bus: &mut impl Bus, val: u16) {
        let [lo, hi] = val.to_le_bytes();
        self.sp = self.sp.wrapping_sub(1);
        self.write(bus, self.sp, hi);
        self.sp = self.sp.wrapping_sub(1);
        self.write(bus, self.sp, lo);
    }

    fn pop(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.read(bus, self.sp);
        self.sp = self.sp.wrapping_add(1);
        let hi = self.read(bus, self.sp);
        self.sp = self.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    /// Execute exactly one instruction (or service an interrupt / stay halted),
    /// driving `bus` one M-cycle at a time. Returns nothing; timing lives in the
    /// bus callbacks.
    pub fn step(&mut self, bus: &mut impl Bus) {
        // EI takes effect after the instruction *following* EI.
        let enable_ime_after = self.ime_pending;

        if self.handle_interrupts(bus) {
            return;
        }

        if self.halted {
            bus.tick();
            return;
        }

        let opcode = self.fetch8(bus);
        self.execute(bus, opcode);

        if enable_ime_after {
            self.ime = true;
            self.ime_pending = false;
        }
    }

    /// If an interrupt is pending and serviceable, dispatch it (5 M-cycles) and
    /// return true. Also breaks HALT even when IME is clear.
    fn handle_interrupts(&mut self, bus: &mut impl Bus) -> bool {
        let pending = bus.pending_interrupts();
        if pending == 0 {
            return false;
        }
        // A pending interrupt always wakes HALT, regardless of IME.
        if self.halted {
            self.halted = false;
        }
        if !self.ime {
            return false;
        }
        self.ime = false;
        self.ime_pending = false;

        // Interrupt dispatch: 2 internal cycles, push PC (2 cycles), then jump.
        bus.tick();
        bus.tick();
        let [lo, hi] = self.pc.to_le_bytes();
        self.sp = self.sp.wrapping_sub(1);
        self.write(bus, self.sp, hi);
        // The IE/IF are sampled here (mid-push); a lower-priority vector can be
        // chosen if IE changed, but for our system IE is stable — take highest.
        let bit = pending.trailing_zeros() as u8;
        self.sp = self.sp.wrapping_sub(1);
        self.write(bus, self.sp, lo);
        bus.ack_interrupt(bit);
        self.pc = 0x0040 + (bit as u16) * 8;
        bus.tick();
        true
    }
}

mod alu;
mod exec;
