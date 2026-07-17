//! SM83 instruction execution. The base opcode map and the CB-prefixed map.
//!
//! The opcode layout is regular: for many groups the low 3 bits select an
//! operand register (`B C D E H L (HL) A`) and middle bits select the op or
//! target. We lean on that regularity for the CB block and the LD r,r' block,
//! but the irregular ops are spelled out for clarity and correctness.

use super::{Bus, Cpu, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};

/// The 8 register operands addressable by the low 3 opcode bits.
/// Index 6 is `(HL)` — a memory operand, handled specially by callers.
#[derive(Clone, Copy)]
enum R8 {
    B,
    C,
    D,
    E,
    H,
    L,
    HlMem,
    A,
}

impl R8 {
    fn from(i: u8) -> R8 {
        match i & 7 {
            0 => R8::B,
            1 => R8::C,
            2 => R8::D,
            3 => R8::E,
            4 => R8::H,
            5 => R8::L,
            6 => R8::HlMem,
            _ => R8::A,
        }
    }
}

impl Cpu {
    /// Read an r8 operand; `(HL)` costs one memory M-cycle.
    fn get_r8(&mut self, bus: &mut impl Bus, r: R8) -> u8 {
        match r {
            R8::B => self.b,
            R8::C => self.c,
            R8::D => self.d,
            R8::E => self.e,
            R8::H => self.h,
            R8::L => self.l,
            R8::A => self.a,
            R8::HlMem => {
                let hl = self.hl();
                self.read(bus, hl)
            }
        }
    }

    fn set_r8(&mut self, bus: &mut impl Bus, r: R8, val: u8) {
        match r {
            R8::B => self.b = val,
            R8::C => self.c = val,
            R8::D => self.d = val,
            R8::E => self.e = val,
            R8::H => self.h = val,
            R8::L => self.l = val,
            R8::A => self.a = val,
            R8::HlMem => {
                let hl = self.hl();
                self.write(bus, hl, val);
            }
        }
    }

    fn cond(&self, i: u8) -> bool {
        match i & 3 {
            0 => !self.flag(FLAG_Z), // NZ
            1 => self.flag(FLAG_Z),  // Z
            2 => !self.flag(FLAG_C), // NC
            _ => self.flag(FLAG_C),  // C
        }
    }

    pub(super) fn execute(&mut self, bus: &mut impl Bus, op: u8) {
        match op {
            0x00 => {} // NOP
            0x10 => {
                // STOP. In the SingleStepTests model it is a fixed 3-M-cycle op
                // (fetch + 2 internal) that does NOT consume the following byte.
                // ponytail: real DMG STOP has complex div-reset / operand
                // behavior and CGB speed-switch semantics; deferred until GBC.
                bus.tick();
                bus.tick();
            }
            0x76 => self.halt(bus),

            // 8-bit loads: LD r, r'  (0x40..=0x7F except 0x76)
            0x40..=0x7F => {
                let dst = R8::from(op >> 3);
                let src = R8::from(op);
                let v = self.get_r8(bus, src);
                self.set_r8(bus, dst, v);
            }

            // LD r, d8
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let dst = R8::from(op >> 3);
                let v = self.fetch8(bus);
                self.set_r8(bus, dst, v);
            }

            // 8-bit ALU with r8: 0x80..=0xBF
            0x80..=0xBF => {
                let src = R8::from(op);
                let v = self.get_r8(bus, src);
                self.alu_op(op >> 3 & 7, v);
            }

            // 8-bit ALU with d8
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let v = self.fetch8(bus);
                self.alu_op(op >> 3 & 7, v);
            }

            // INC/DEC r8
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let r = R8::from(op >> 3);
                let v = self.get_r8(bus, r);
                let nv = self.inc8(v);
                self.set_r8(bus, r, nv);
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let r = R8::from(op >> 3);
                let v = self.get_r8(bus, r);
                let nv = self.dec8(v);
                self.set_r8(bus, r, nv);
            }

            // 16-bit loads: LD rr, d16
            0x01 => {
                let v = self.fetch16(bus);
                self.set_bc(v);
            }
            0x11 => {
                let v = self.fetch16(bus);
                self.set_de(v);
            }
            0x21 => {
                let v = self.fetch16(bus);
                self.set_hl(v);
            }
            0x31 => self.sp = self.fetch16(bus),

            // LD (rr), A / LD A, (rr)
            0x02 => {
                let addr = self.bc();
                self.write(bus, addr, self.a);
            }
            0x12 => {
                let addr = self.de();
                self.write(bus, addr, self.a);
            }
            0x22 => {
                let hl = self.hl();
                self.write(bus, hl, self.a);
                self.set_hl(hl.wrapping_add(1));
            }
            0x32 => {
                let hl = self.hl();
                self.write(bus, hl, self.a);
                self.set_hl(hl.wrapping_sub(1));
            }
            0x0A => {
                let addr = self.bc();
                self.a = self.read(bus, addr);
            }
            0x1A => {
                let addr = self.de();
                self.a = self.read(bus, addr);
            }
            0x2A => {
                let hl = self.hl();
                self.a = self.read(bus, hl);
                self.set_hl(hl.wrapping_add(1));
            }
            0x3A => {
                let hl = self.hl();
                self.a = self.read(bus, hl);
                self.set_hl(hl.wrapping_sub(1));
            }

            // LD (a16), SP
            0x08 => {
                let addr = self.fetch16(bus);
                let [lo, hi] = self.sp.to_le_bytes();
                self.write(bus, addr, lo);
                self.write(bus, addr.wrapping_add(1), hi);
            }

            // INC/DEC rr (2 M-cycles; the extra cycle is an internal tick)
            0x03 => {
                bus.tick();
                self.set_bc(self.bc().wrapping_add(1));
            }
            0x13 => {
                bus.tick();
                self.set_de(self.de().wrapping_add(1));
            }
            0x23 => {
                bus.tick();
                self.set_hl(self.hl().wrapping_add(1));
            }
            0x33 => {
                bus.tick();
                self.sp = self.sp.wrapping_add(1);
            }
            0x0B => {
                bus.tick();
                self.set_bc(self.bc().wrapping_sub(1));
            }
            0x1B => {
                bus.tick();
                self.set_de(self.de().wrapping_sub(1));
            }
            0x2B => {
                bus.tick();
                self.set_hl(self.hl().wrapping_sub(1));
            }
            0x3B => {
                bus.tick();
                self.sp = self.sp.wrapping_sub(1);
            }

            // ADD HL, rr (extra internal M-cycle)
            0x09 => {
                bus.tick();
                self.add_hl(self.bc());
            }
            0x19 => {
                bus.tick();
                self.add_hl(self.de());
            }
            0x29 => {
                bus.tick();
                self.add_hl(self.hl());
            }
            0x39 => {
                bus.tick();
                self.add_hl(self.sp);
            }

            // Accumulator rotates (Z always cleared, unlike CB variants)
            0x07 => {
                self.a = self.rlc(self.a);
                self.set_flag(FLAG_Z, false);
            }
            0x0F => {
                self.a = self.rrc(self.a);
                self.set_flag(FLAG_Z, false);
            }
            0x17 => {
                self.a = self.rl(self.a);
                self.set_flag(FLAG_Z, false);
            }
            0x1F => {
                self.a = self.rr(self.a);
                self.set_flag(FLAG_Z, false);
            }

            0x27 => self.daa(),
            0x2F => {
                // CPL
                self.a = !self.a;
                self.set_flag(FLAG_N, true);
                self.set_flag(FLAG_H, true);
            }
            0x37 => {
                // SCF
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                self.set_flag(FLAG_C, true);
            }
            0x3F => {
                // CCF
                self.set_flag(FLAG_N, false);
                self.set_flag(FLAG_H, false);
                let c = self.flag(FLAG_C);
                self.set_flag(FLAG_C, !c);
            }

            // Relative jumps
            0x18 => self.jr(bus, true),
            0x20 | 0x28 | 0x30 | 0x38 => {
                let take = self.cond(op >> 3);
                self.jr(bus, take);
            }

            // Absolute jumps
            0xC3 => {
                let addr = self.fetch16(bus);
                bus.tick();
                self.pc = addr;
            }
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let addr = self.fetch16(bus);
                if self.cond(op >> 3) {
                    bus.tick();
                    self.pc = addr;
                }
            }
            0xE9 => self.pc = self.hl(), // JP (HL): no extra cycle

            // Calls
            0xCD => {
                let addr = self.fetch16(bus);
                bus.tick();
                self.push(bus, self.pc);
                self.pc = addr;
            }
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let addr = self.fetch16(bus);
                if self.cond(op >> 3) {
                    bus.tick();
                    self.push(bus, self.pc);
                    self.pc = addr;
                }
            }

            // Returns
            0xC9 => {
                self.pc = self.pop(bus);
                bus.tick();
            }
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                bus.tick(); // conditional RET has an extra internal cycle
                if self.cond(op >> 3) {
                    self.pc = self.pop(bus);
                    bus.tick();
                }
            }
            0xD9 => {
                // RETI
                self.pc = self.pop(bus);
                bus.tick();
                self.ime = true;
            }

            // RST
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                bus.tick();
                self.push(bus, self.pc);
                self.pc = (op & 0x38) as u16;
            }

            // PUSH/POP
            0xC1 => {
                let v = self.pop(bus);
                self.set_bc(v);
            }
            0xD1 => {
                let v = self.pop(bus);
                self.set_de(v);
            }
            0xE1 => {
                let v = self.pop(bus);
                self.set_hl(v);
            }
            0xF1 => {
                let v = self.pop(bus);
                self.set_af(v);
            }
            0xC5 => {
                bus.tick();
                self.push(bus, self.bc());
            }
            0xD5 => {
                bus.tick();
                self.push(bus, self.de());
            }
            0xE5 => {
                bus.tick();
                self.push(bus, self.hl());
            }
            0xF5 => {
                bus.tick();
                self.push(bus, self.af());
            }

            // High-RAM / IO loads
            0xE0 => {
                let off = self.fetch8(bus);
                self.write(bus, 0xFF00 | off as u16, self.a);
            }
            0xF0 => {
                let off = self.fetch8(bus);
                self.a = self.read(bus, 0xFF00 | off as u16);
            }
            0xE2 => self.write(bus, 0xFF00 | self.c as u16, self.a),
            0xF2 => {
                let addr = 0xFF00 | self.c as u16;
                self.a = self.read(bus, addr);
            }
            0xEA => {
                let addr = self.fetch16(bus);
                self.write(bus, addr, self.a);
            }
            0xFA => {
                let addr = self.fetch16(bus);
                self.a = self.read(bus, addr);
            }

            // 16-bit SP arithmetic
            0xE8 => {
                // ADD SP, e8 — 4 M-cycles
                let e = self.fetch8(bus) as i8;
                let r = self.add_sp_e8(e);
                bus.tick();
                bus.tick();
                self.sp = r;
            }
            0xF8 => {
                // LD HL, SP+e8 — 3 M-cycles
                let e = self.fetch8(bus) as i8;
                let r = self.add_sp_e8(e);
                bus.tick();
                self.set_hl(r);
            }
            0xF9 => {
                // LD SP, HL — 2 M-cycles
                bus.tick();
                self.sp = self.hl();
            }

            // Interrupt control
            0xF3 => {
                self.ime = false;
                self.ime_pending = false;
            }
            0xFB => self.ime_pending = true,

            0xCB => self.execute_cb(bus),

            // Illegal opcodes: these hang real hardware. SingleStepTests omits
            // them; if a bad ROM hits one, freeze the PC (spin) rather than panic.
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                self.pc = self.pc.wrapping_sub(1);
            }
        }
    }

    fn alu_op(&mut self, kind: u8, v: u8) {
        match kind {
            0 => self.add8(v, false),
            1 => self.add8(v, self.flag(FLAG_C)),
            2 => self.sub8(v, false),
            3 => self.sub8(v, self.flag(FLAG_C)),
            4 => self.and8(v),
            5 => self.xor8(v),
            6 => self.or8(v),
            _ => self.cp8(v),
        }
    }

    fn jr(&mut self, bus: &mut impl Bus, take: bool) {
        let e = self.fetch8(bus) as i8;
        if take {
            bus.tick();
            self.pc = self.pc.wrapping_add(e as u16);
        }
    }

    fn halt(&mut self, bus: &mut impl Bus) {
        // HALT bug: if IME=0 and an interrupt is already pending, the CPU does
        // not halt and the next fetch reads the same byte twice.
        if !self.ime && bus.pending_interrupts() != 0 {
            self.halt_bug = true;
        } else {
            self.halted = true;
        }
        // SingleStepTests models HALT as a fixed 3-M-cycle op (fetch + 2). The
        // two internal cycles are harmless in the running system (hardware just
        // advances two cycles before the halt loop begins ticking).
        bus.tick();
        bus.tick();
    }

    fn execute_cb(&mut self, bus: &mut impl Bus) {
        let op = self.fetch8(bus);
        let r = R8::from(op);
        let v = self.get_r8(bus, r);
        let bit = op >> 3 & 7;
        let result = match op >> 6 {
            0 => match op >> 3 & 7 {
                0 => self.rlc(v),
                1 => self.rrc(v),
                2 => self.rl(v),
                3 => self.rr(v),
                4 => self.sla(v),
                5 => self.sra(v),
                6 => self.swap(v),
                _ => self.srl(v),
            },
            1 => {
                // BIT n, r — no writeback; (HL) form is 3 M-cycles (read only).
                self.bit(v, bit);
                return;
            }
            2 => v & !(1 << bit), // RES
            _ => v | (1 << bit),  // SET
        };
        self.set_r8(bus, r, result);
    }
}
