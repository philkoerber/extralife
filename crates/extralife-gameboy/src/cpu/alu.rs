//! SM83 ALU: arithmetic, logic, rotates, and the CB-prefix bit operations.
//! Flag semantics are per Pandocs / the SM83 opcode reference. Half-carry (H)
//! is carry from bit 3; carry (C) is carry from bit 7 (or bit 11/15 for 16-bit
//! ADD HL).

use super::{Cpu, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};

impl Cpu {
    pub(super) fn add8(&mut self, val: u8, carry: bool) {
        let c = carry as u16;
        let a = self.a as u16;
        let r = a + val as u16 + c;
        let h = (a & 0xF) + (val as u16 & 0xF) + c > 0xF;
        self.set_flags(r as u8 == 0, false, h, r > 0xFF);
        self.a = r as u8;
    }

    pub(super) fn sub8(&mut self, val: u8, carry: bool) {
        let c = carry as i16;
        let a = self.a as i16;
        let r = a - val as i16 - c;
        let h = (a & 0xF) - (val as i16 & 0xF) - c < 0;
        self.set_flags(r as u8 == 0, true, h, r < 0);
        self.a = r as u8;
    }

    pub(super) fn and8(&mut self, val: u8) {
        self.a &= val;
        self.set_flags(self.a == 0, false, true, false);
    }

    pub(super) fn or8(&mut self, val: u8) {
        self.a |= val;
        self.set_flags(self.a == 0, false, false, false);
    }

    pub(super) fn xor8(&mut self, val: u8) {
        self.a ^= val;
        self.set_flags(self.a == 0, false, false, false);
    }

    pub(super) fn cp8(&mut self, val: u8) {
        // Compare = subtract discarding the result.
        let a = self.a as i16;
        let r = a - val as i16;
        let h = (a & 0xF) - (val as i16 & 0xF) < 0;
        self.set_flags(r as u8 == 0, true, h, r < 0);
    }

    pub(super) fn inc8(&mut self, val: u8) -> u8 {
        let r = val.wrapping_add(1);
        self.set_flag(FLAG_Z, r == 0);
        self.set_flag(FLAG_N, false);
        self.set_flag(FLAG_H, val & 0xF == 0xF);
        r
    }

    pub(super) fn dec8(&mut self, val: u8) -> u8 {
        let r = val.wrapping_sub(1);
        self.set_flag(FLAG_Z, r == 0);
        self.set_flag(FLAG_N, true);
        self.set_flag(FLAG_H, val & 0xF == 0);
        r
    }

    pub(super) fn add_hl(&mut self, val: u16) {
        let hl = self.hl();
        let r = hl as u32 + val as u32;
        let h = (hl & 0x0FFF) + (val & 0x0FFF) > 0x0FFF;
        self.set_flag(FLAG_N, false);
        self.set_flag(FLAG_H, h);
        self.set_flag(FLAG_C, r > 0xFFFF);
        self.set_hl(r as u16);
    }

    /// `ADD SP, e8` / `LD HL, SP+e8` share this: flags come from the low-byte
    /// unsigned add (bit 3 -> H, bit 7 -> C), N and Z are cleared.
    pub(super) fn add_sp_e8(&mut self, e: i8) -> u16 {
        let sp = self.sp;
        let off = e as u16;
        let h = (sp & 0xF) + (off & 0xF) > 0xF;
        let c = (sp & 0xFF) + (off & 0xFF) > 0xFF;
        self.set_flags(false, false, h, c);
        sp.wrapping_add(off)
    }

    /// `DAA` — decimal-adjust A after an add/subtract of BCD values.
    pub(super) fn daa(&mut self) {
        let mut a = self.a;
        let mut adjust = 0u8;
        let mut carry = self.flag(FLAG_C);
        if self.flag(FLAG_H) || (!self.flag(FLAG_N) && (a & 0xF) > 9) {
            adjust |= 0x06;
        }
        if carry || (!self.flag(FLAG_N) && a > 0x99) {
            adjust |= 0x60;
            carry = true;
        }
        if self.flag(FLAG_N) {
            a = a.wrapping_sub(adjust);
        } else {
            a = a.wrapping_add(adjust);
        }
        self.a = a;
        self.set_flag(FLAG_Z, a == 0);
        self.set_flag(FLAG_H, false);
        self.set_flag(FLAG_C, carry);
    }

    // --- rotates / shifts (accumulator + CB variants) ----------------------

    pub(super) fn rlc(&mut self, val: u8) -> u8 {
        let c = val >> 7;
        let r = val.rotate_left(1);
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn rrc(&mut self, val: u8) -> u8 {
        let c = val & 1;
        let r = val.rotate_right(1);
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn rl(&mut self, val: u8) -> u8 {
        let old_c = self.flag(FLAG_C) as u8;
        let c = val >> 7;
        let r = (val << 1) | old_c;
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn rr(&mut self, val: u8) -> u8 {
        let old_c = self.flag(FLAG_C) as u8;
        let c = val & 1;
        let r = (val >> 1) | (old_c << 7);
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn sla(&mut self, val: u8) -> u8 {
        let c = val >> 7;
        let r = val << 1;
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn sra(&mut self, val: u8) -> u8 {
        let c = val & 1;
        let r = (val >> 1) | (val & 0x80); // arithmetic: keep sign bit
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn srl(&mut self, val: u8) -> u8 {
        let c = val & 1;
        let r = val >> 1;
        self.set_flags(r == 0, false, false, c != 0);
        r
    }
    pub(super) fn swap(&mut self, val: u8) -> u8 {
        let r = val.rotate_left(4);
        self.set_flags(r == 0, false, false, false);
        r
    }

    pub(super) fn bit(&mut self, val: u8, n: u8) {
        self.set_flag(FLAG_Z, val & (1 << n) == 0);
        self.set_flag(FLAG_N, false);
        self.set_flag(FLAG_H, true);
        // C unaffected.
    }
}
