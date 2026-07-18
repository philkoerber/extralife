//! Undocumented ("illegal") NMOS 6502 opcodes.
//!
//! The SingleStepTests cover all 256 opcodes, so the stable illegals must run
//! with correct results *and* correct bus timing. Implemented from the NESdev
//! wiki "CPU unofficial opcodes" page (behavior described there; nothing
//! transcribed from an emulator). Grouped by kind:
//!   - Multi-byte NOPs (various addressing modes; some do a dummy read).
//!   - Combined RMW+ALU: SLO ASO, RLA, SRE LSE, RRA, DCP, ISC ISB.
//!   - Combined load: LAX; combined store: SAX.
//!   - The unstable "magic-constant" ops (ANE/LXA/TAS/SHA/SHX/SHY/LAS) are
//!     implemented to their commonly-agreed deterministic form so the tests
//!     (which use fixed operands) pass; see notes inline.

use crate::cpu::{Bus, Cpu, FLAG_C};

impl Cpu {
    pub(super) fn undocumented(&mut self, bus: &mut impl Bus, op: u8) {
        macro_rules! rmw_alu { ($f:ident, $mode:ident) => {{
            let a = self.$mode(bus);
            self.rmw(bus, a, Cpu::$f);
        }}; }
        macro_rules! lax { ($mode:ident) => {{
            let a = self.$mode(bus);
            let v = self.load_operand(bus, a);
            self.a = v; self.x = v; self.set_nz(v);
        }}; }

        match op {
            // --- NOPs (implied, single byte) ---
            0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => self.dummy_read(bus),
            // --- NOP #imm (2-byte) ---
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => { let _ = self.fetch8(bus); }
            // --- NOP zp / zp,X / abs / abs,X (dummy read of operand) ---
            0x04 | 0x44 | 0x64 => { let a = self.am_zp(bus); let _ = self.read(bus, a.addr); }
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => {
                let a = self.am_zp_x(bus); let _ = self.read(bus, a.addr);
            }
            0x0C => { let a = self.am_abs(bus); let _ = self.read(bus, a.addr); }
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => {
                let a = self.am_abs_x(bus); let _ = self.load_operand(bus, a);
            }
            // --- LAX (LDA+LDX) ---
            0xA7 => lax!(am_zp),
            0xB7 => lax!(am_zp_y),
            0xAF => lax!(am_abs),
            0xBF => lax!(am_abs_y),
            0xA3 => lax!(am_ind_x),
            0xB3 => lax!(am_ind_y),
            0xAB => { // LXA #imm (unstable): A = X = (A | CONST) & imm. The magic
                // constant reproducing the SingleStepTests fixtures is 0xEE.
                let v = self.fetch8(bus);
                let r = (self.a | 0xEE) & v;
                self.a = r; self.x = r; self.set_nz(r);
            }
            // --- SAX (store A & X) ---
            0x87 => { let a = self.am_zp(bus); self.store(bus, a, self.a & self.x); }
            0x97 => { let a = self.am_zp_y(bus); self.store(bus, a, self.a & self.x); }
            0x8F => { let a = self.am_abs(bus); self.store(bus, a, self.a & self.x); }
            0x83 => { let a = self.am_ind_x(bus); self.store(bus, a, self.a & self.x); }
            // --- SLO (ASL + ORA) ---
            0x07 => rmw_alu!(op_slo, am_zp),
            0x17 => rmw_alu!(op_slo, am_zp_x),
            0x0F => rmw_alu!(op_slo, am_abs),
            0x1F => rmw_alu!(op_slo, am_abs_x),
            0x1B => rmw_alu!(op_slo, am_abs_y),
            0x03 => rmw_alu!(op_slo, am_ind_x),
            0x13 => rmw_alu!(op_slo, am_ind_y),
            // --- RLA (ROL + AND) ---
            0x27 => rmw_alu!(op_rla, am_zp),
            0x37 => rmw_alu!(op_rla, am_zp_x),
            0x2F => rmw_alu!(op_rla, am_abs),
            0x3F => rmw_alu!(op_rla, am_abs_x),
            0x3B => rmw_alu!(op_rla, am_abs_y),
            0x23 => rmw_alu!(op_rla, am_ind_x),
            0x33 => rmw_alu!(op_rla, am_ind_y),
            // --- SRE (LSR + EOR) ---
            0x47 => rmw_alu!(op_sre, am_zp),
            0x57 => rmw_alu!(op_sre, am_zp_x),
            0x4F => rmw_alu!(op_sre, am_abs),
            0x5F => rmw_alu!(op_sre, am_abs_x),
            0x5B => rmw_alu!(op_sre, am_abs_y),
            0x43 => rmw_alu!(op_sre, am_ind_x),
            0x53 => rmw_alu!(op_sre, am_ind_y),
            // --- RRA (ROR + ADC) ---
            0x67 => rmw_alu!(op_rra, am_zp),
            0x77 => rmw_alu!(op_rra, am_zp_x),
            0x6F => rmw_alu!(op_rra, am_abs),
            0x7F => rmw_alu!(op_rra, am_abs_x),
            0x7B => rmw_alu!(op_rra, am_abs_y),
            0x63 => rmw_alu!(op_rra, am_ind_x),
            0x73 => rmw_alu!(op_rra, am_ind_y),
            // --- DCP (DEC + CMP) ---
            0xC7 => rmw_alu!(op_dcp, am_zp),
            0xD7 => rmw_alu!(op_dcp, am_zp_x),
            0xCF => rmw_alu!(op_dcp, am_abs),
            0xDF => rmw_alu!(op_dcp, am_abs_x),
            0xDB => rmw_alu!(op_dcp, am_abs_y),
            0xC3 => rmw_alu!(op_dcp, am_ind_x),
            0xD3 => rmw_alu!(op_dcp, am_ind_y),
            // --- ISC / ISB (INC + SBC) ---
            0xE7 => rmw_alu!(op_isc, am_zp),
            0xF7 => rmw_alu!(op_isc, am_zp_x),
            0xEF => rmw_alu!(op_isc, am_abs),
            0xFF => rmw_alu!(op_isc, am_abs_x),
            0xFB => rmw_alu!(op_isc, am_abs_y),
            0xE3 => rmw_alu!(op_isc, am_ind_x),
            0xF3 => rmw_alu!(op_isc, am_ind_y),
            // --- immediate AND-family ---
            0x0B | 0x2B => { // ANC: AND #imm, then C = bit 7 of result.
                let v = self.fetch8(bus);
                self.a &= v; self.set_nz(self.a);
                self.set_flag(FLAG_C, self.a & 0x80 != 0);
            }
            0x4B => { // ALR/ASR: AND #imm then LSR A.
                let v = self.fetch8(bus);
                self.a &= v;
                self.set_flag(FLAG_C, self.a & 1 != 0);
                self.a >>= 1;
                self.set_nz(self.a);
            }
            0x6B => { // ARR: AND #imm then ROR A, with the quirky C/V flags.
                let v = self.fetch8(bus);
                self.a &= v;
                let c = self.flag(FLAG_C) as u8;
                self.a = (self.a >> 1) | (c << 7);
                self.set_nz(self.a);
                self.set_flag(FLAG_C, self.a & 0x40 != 0);
                self.set_flag(crate::cpu::FLAG_V, ((self.a >> 6) ^ (self.a >> 5)) & 1 != 0);
            }
            0x8B => { // ANE/XAA (unstable): A = (A | CONST) & X & imm. CONST=0xEE
                // reproduces the SingleStepTests fixtures.
                let v = self.fetch8(bus);
                self.a = (self.a | 0xEE) & self.x & v;
                self.set_nz(self.a);
            }
            0xCB => { // SBX/AXS: X = (A & X) - imm, set C like CMP.
                let v = self.fetch8(bus);
                let t = self.a & self.x;
                self.set_flag(FLAG_C, t >= v);
                self.x = t.wrapping_sub(v);
                self.set_nz(self.x);
            }
            // --- unstable store-high ops (SHA/SHX/SHY/TAS) ---
            0x9C => self.sh_y(bus), // SHY abs,X
            0x9E => self.sh_x(bus), // SHX abs,Y
            0x9F => self.sha_abs_y(bus), // SHA abs,Y
            0x93 => self.sha_ind_y(bus), // SHA (zp),Y
            0x9B => self.tas(bus), // TAS abs,Y
            0xBB => self.las(bus), // LAS abs,Y
            // --- KIL/JAM (halt): the CPU wedges. The suite models it as the
            // opcode fetch, PC left at the opcode byte, and two dummy reads of
            // the following byte. We replicate that exact bus signature.
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2
            | 0xD2 | 0xF2 => {
                let _ = self.read(bus, self.pc);
                let _ = self.read(bus, self.pc);
                self.pc = self.pc.wrapping_sub(1);
            }
            _ => self.dummy_read(bus),
        }
    }

    // Combined RMW+ALU cores: each takes the freshly-read memory value, returns
    // the byte to write back, and applies the ALU side-effect on A/flags.
    fn op_slo(&mut self, v: u8) -> u8 { let r = self.op_asl(v); self.a |= r; self.set_nz(self.a); r }
    fn op_rla(&mut self, v: u8) -> u8 { let r = self.op_rol(v); self.a &= r; self.set_nz(self.a); r }
    fn op_sre(&mut self, v: u8) -> u8 { let r = self.op_lsr(v); self.a ^= r; self.set_nz(self.a); r }
    fn op_rra(&mut self, v: u8) -> u8 { let r = self.op_ror(v); self.adc(r); r }
    fn op_dcp(&mut self, v: u8) -> u8 { let r = v.wrapping_sub(1); self.cmp_with(self.a, r); r }
    fn op_isc(&mut self, v: u8) -> u8 { let r = v.wrapping_add(1); self.sbc(r); r }

    // Unstable "store reg & (high+1)" ops (SHA/SHX/SHY/TAS). Real hardware ANDs
    // the register with the high byte of the (pre-index) base address plus one;
    // on a page cross the target's high byte is *also* replaced by that value.
    // The SingleStepTests pin the deterministic form these settle to.
    fn sh_common(&mut self, bus: &mut impl Bus, base: u16, index: u8, reg: u8) {
        let addr = base.wrapping_add(index as u16);
        // Dummy read at the un-carried address (low fixed, high not).
        let unfixed = (addr & 0x00FF) | (base & 0xFF00);
        let _ = self.read(bus, unfixed);
        let value = reg & (((base >> 8) as u8).wrapping_add(1));
        let crossed = (base & 0xFF00) != (addr & 0xFF00);
        let target = if crossed {
            (addr & 0x00FF) | ((value as u16) << 8)
        } else {
            addr
        };
        self.write(bus, target, value);
    }
    fn sh_x(&mut self, bus: &mut impl Bus) {
        let base = self.fetch16(bus);
        self.sh_common(bus, base, self.y, self.x);
    }
    fn sh_y(&mut self, bus: &mut impl Bus) {
        let base = self.fetch16(bus);
        self.sh_common(bus, base, self.x, self.y);
    }
    fn sha_abs_y(&mut self, bus: &mut impl Bus) {
        let base = self.fetch16(bus);
        self.sh_common(bus, base, self.y, self.a & self.x);
    }
    fn sha_ind_y(&mut self, bus: &mut impl Bus) {
        let ptr = self.fetch8(bus);
        let lo = self.read(bus, ptr as u16) as u16;
        let hi = self.read(bus, ptr.wrapping_add(1) as u16) as u16;
        let base = lo | (hi << 8);
        self.sh_common(bus, base, self.y, self.a & self.x);
    }
    fn tas(&mut self, bus: &mut impl Bus) {
        let base = self.fetch16(bus);
        self.sp = self.a & self.x;
        self.sh_common(bus, base, self.y, self.sp);
    }
    fn las(&mut self, bus: &mut impl Bus) {
        let a = self.am_abs_y(bus);
        let v = self.load_operand(bus, a);
        let r = v & self.sp;
        self.a = r; self.x = r; self.sp = r;
        self.set_nz(r);
    }
}
