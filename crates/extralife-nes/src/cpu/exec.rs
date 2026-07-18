//! 6502 instruction execution: addressing modes + the official opcode map.
//!
//! Cycle accuracy follows the NMOS 6502's real bus behavior, which the
//! SingleStepTests validate access-by-access:
//!   - Every cycle is a bus access (there are no accessless internal cycles;
//!     "internal" cycles issue a dummy read).
//!   - Indexed addressing (abs,X / abs,Y / (ind),Y) does a dummy read at the
//!     un-carried address; for *read* instructions this extra read only happens
//!     when the index crosses a page, for *store* and read-modify-write
//!     instructions it always happens.
//!   - Read-modify-write does read, dummy write-back of the old value, then the
//!     modified write (three accesses at the effective address).
//!   - JMP (indirect) reproduces the page-wrap bug: the high byte is fetched
//!     from the same page as the low byte.
//!
//! Decimal mode is disabled (2A03): ADC/SBC ignore the D flag.

use super::{Bus, Cpu, FLAG_B, FLAG_C, FLAG_D, FLAG_I, FLAG_N, FLAG_U, FLAG_V, FLAG_Z, STACK_BASE};

/// How an instruction computes its operand address / whether it crossed a page.
#[derive(Clone, Copy)]
struct Addr {
    addr: u16,
    /// True when an index carried into a new page (the read-instruction extra
    /// cycle). Stores/RMW always take the penalty regardless.
    page_crossed: bool,
    /// True for indexed modes (abs,X / abs,Y / (ind),Y), where stores and RMW
    /// always issue one dummy read even without a page cross.
    indexed: bool,
}

impl Cpu {
    // --- address-mode resolvers ------------------------------------------
    // Each returns the effective address and consumes exactly the operand-fetch
    // cycles. Index dummy reads for the read path are applied by the caller via
    // `spurious_read_on_cross`, so stores/RMW can force the read unconditionally.

    fn am_zp(&mut self, bus: &mut impl Bus) -> Addr {
        Addr { addr: self.fetch8(bus) as u16, page_crossed: false, indexed: false }
    }
    fn am_zp_x(&mut self, bus: &mut impl Bus) -> Addr {
        let base = self.fetch8(bus);
        // Dummy read at the un-indexed zero-page address, then wrap in ZP.
        let _ = self.read(bus, base as u16);
        Addr { addr: base.wrapping_add(self.x) as u16, page_crossed: false, indexed: false }
    }
    fn am_zp_y(&mut self, bus: &mut impl Bus) -> Addr {
        let base = self.fetch8(bus);
        let _ = self.read(bus, base as u16);
        Addr { addr: base.wrapping_add(self.y) as u16, page_crossed: false, indexed: false }
    }
    fn am_abs(&mut self, bus: &mut impl Bus) -> Addr {
        Addr { addr: self.fetch16(bus), page_crossed: false, indexed: false }
    }
    fn am_abs_x(&mut self, bus: &mut impl Bus) -> Addr {
        let base = self.fetch16(bus);
        let addr = base.wrapping_add(self.x as u16);
        Addr { addr, page_crossed: (base & 0xFF00) != (addr & 0xFF00), indexed: true }
    }
    fn am_abs_y(&mut self, bus: &mut impl Bus) -> Addr {
        let base = self.fetch16(bus);
        let addr = base.wrapping_add(self.y as u16);
        Addr { addr, page_crossed: (base & 0xFF00) != (addr & 0xFF00), indexed: true }
    }
    fn am_ind_x(&mut self, bus: &mut impl Bus) -> Addr {
        // (zp,X): read pointer from zero page at (operand + X), wrapping in ZP.
        let base = self.fetch8(bus);
        let _ = self.read(bus, base as u16); // dummy read before indexing
        let ptr = base.wrapping_add(self.x);
        let lo = self.read(bus, ptr as u16) as u16;
        let hi = self.read(bus, ptr.wrapping_add(1) as u16) as u16;
        Addr { addr: lo | (hi << 8), page_crossed: false, indexed: false }
    }
    fn am_ind_y(&mut self, bus: &mut impl Bus) -> Addr {
        // (zp),Y: read a 16-bit base from zero page, add Y (may cross a page).
        let ptr = self.fetch8(bus);
        let lo = self.read(bus, ptr as u16) as u16;
        let hi = self.read(bus, ptr.wrapping_add(1) as u16) as u16;
        let base = lo | (hi << 8);
        let addr = base.wrapping_add(self.y as u16);
        Addr { addr, page_crossed: (base & 0xFF00) != (addr & 0xFF00), indexed: true }
    }

    /// The indexed-read page-cross dummy read: reads the un-carried address when
    /// a page was crossed. Call for *read* instructions using abs,X/abs,Y/(ind),Y.
    fn read_cross_penalty(&mut self, bus: &mut impl Bus, a: Addr) {
        if a.page_crossed {
            // The CPU reads the address before the high-byte fixup; modeling the
            // exact wrong address isn't needed for our system, but the tests
            // require an access at (addr - 0x100) when the low byte wrapped.
            let wrong = (a.addr & 0x00FF) | (a.addr.wrapping_sub(0x100) & 0xFF00);
            let _ = self.read(bus, wrong);
        }
    }
    /// The unconditional indexed dummy read for stores and RMW at abs,X/abs,Y/
    /// (ind),Y: always one dummy read at the (possibly un-carried) address.
    fn always_cross_penalty(&mut self, bus: &mut impl Bus, a: Addr) {
        let wrong = if a.page_crossed {
            (a.addr & 0x00FF) | (a.addr.wrapping_sub(0x100) & 0xFF00)
        } else {
            a.addr
        };
        let _ = self.read(bus, wrong);
    }

    // --- ALU helpers ------------------------------------------------------

    /// ADC with decimal mode disabled (2A03): binary add of A + operand + carry.
    fn adc(&mut self, val: u8) {
        let a = self.a as u16;
        let sum = a + val as u16 + self.flag(FLAG_C) as u16;
        let result = sum as u8;
        self.set_flag(FLAG_C, sum > 0xFF);
        // Overflow: operands same sign, result differs.
        self.set_flag(FLAG_V, (self.a ^ result) & (val ^ result) & 0x80 != 0);
        self.a = result;
        self.set_nz(result);
    }
    /// SBC = ADC of the one's complement of the operand (decimal mode disabled).
    fn sbc(&mut self, val: u8) {
        self.adc(val ^ 0xFF);
    }
    fn cmp_with(&mut self, reg: u8, val: u8) {
        let r = reg.wrapping_sub(val);
        self.set_flag(FLAG_C, reg >= val);
        self.set_nz(r);
    }

    fn branch(&mut self, bus: &mut impl Bus, cond: bool) {
        let off = self.fetch8(bus) as i8 as u16;
        if cond {
            // Taken branch: dummy read of the opcode slot, +1 cycle on page cross.
            let _ = self.read(bus, self.pc);
            let target = self.pc.wrapping_add(off);
            if (self.pc & 0xFF00) != (target & 0xFF00) {
                let fixup = (target & 0x00FF) | (self.pc & 0xFF00);
                let _ = self.read(bus, fixup);
            }
            self.pc = target;
        }
    }

    // --- operation kinds parameterized by resolved address ----------------

    /// Read the operand for a read-type instruction, taking the page-cross
    /// penalty. Non-indexed modes (zp/abs/(zp,X)) pass `page_crossed=false`.
    fn load_operand(&mut self, bus: &mut impl Bus, a: Addr) -> u8 {
        self.read_cross_penalty(bus, a);
        self.read(bus, a.addr)
    }

    /// Read-modify-write at `a`: read, write-back original (dummy), write result.
    /// `op` returns the modified byte and updates flags.
    fn rmw(&mut self, bus: &mut impl Bus, a: Addr, op: impl FnOnce(&mut Cpu, u8) -> u8) {
        // Indexed RMW always eats the dummy read regardless of page cross.
        self.always_cross_penalty_if_indexed(bus, a);
        let val = self.read(bus, a.addr);
        self.write(bus, a.addr, val); // dummy write-back of the original
        let res = op(self, val);
        self.write(bus, a.addr, res);
    }

    /// Marker set by indexed RMW/store resolvers: they call the always-penalty
    /// helper. For zp/abs the `page_crossed` flag is false and no penalty read
    /// is issued (zp/abs RMW have no indexed dummy read).
    fn always_cross_penalty_if_indexed(&mut self, bus: &mut impl Bus, a: Addr) {
        if a.indexed {
            self.always_cross_penalty(bus, a);
        }
    }

    /// Store `val` at `a`, taking the indexed always-penalty read first.
    fn store(&mut self, bus: &mut impl Bus, a: Addr, val: u8) {
        self.always_cross_penalty_if_indexed(bus, a);
        self.write(bus, a.addr, val);
    }

    // --- shift/rotate primitives (used by RMW and accumulator variants) ---
    fn op_asl(&mut self, v: u8) -> u8 {
        self.set_flag(FLAG_C, v & 0x80 != 0);
        let r = v << 1;
        self.set_nz(r);
        r
    }
    fn op_lsr(&mut self, v: u8) -> u8 {
        self.set_flag(FLAG_C, v & 1 != 0);
        let r = v >> 1;
        self.set_nz(r);
        r
    }
    fn op_rol(&mut self, v: u8) -> u8 {
        let c = self.flag(FLAG_C) as u8;
        self.set_flag(FLAG_C, v & 0x80 != 0);
        let r = (v << 1) | c;
        self.set_nz(r);
        r
    }
    fn op_ror(&mut self, v: u8) -> u8 {
        let c = self.flag(FLAG_C) as u8;
        self.set_flag(FLAG_C, v & 1 != 0);
        let r = (v >> 1) | (c << 7);
        self.set_nz(r);
        r
    }
    fn op_inc(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.set_nz(r);
        r
    }
    fn op_dec(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.set_nz(r);
        r
    }

    /// Dispatch and run one opcode. Operand-fetch and access cycles happen
    /// inside the addressing-mode and access helpers, so timing is exact.
    pub(super) fn execute(&mut self, bus: &mut impl Bus, op: u8) {
        macro_rules! ld { ($reg:ident, $mode:ident) => {{
            let a = self.$mode(bus);
            let v = self.load_operand(bus, a);
            self.$reg = v;
            self.set_nz(v);
        }}; }
        macro_rules! st { ($reg:ident, $mode:ident) => {{
            let a = self.$mode(bus);
            self.store(bus, a, self.$reg);
        }}; }
        macro_rules! alu { ($f:ident, $mode:ident) => {{
            let a = self.$mode(bus);
            let v = self.load_operand(bus, a);
            self.$f(v);
        }}; }
        macro_rules! rmw { ($f:ident, $mode:ident) => {{
            let a = self.$mode(bus);
            self.rmw(bus, a, Cpu::$f);
        }}; }

        match op {
            // --- LDA ---
            0xA9 => { let v = self.fetch8(bus); self.a = v; self.set_nz(v); }
            0xA5 => ld!(a, am_zp),
            0xB5 => ld!(a, am_zp_x),
            0xAD => ld!(a, am_abs),
            0xBD => ld!(a, am_abs_x),
            0xB9 => ld!(a, am_abs_y),
            0xA1 => ld!(a, am_ind_x),
            0xB1 => ld!(a, am_ind_y),
            // --- LDX ---
            0xA2 => { let v = self.fetch8(bus); self.x = v; self.set_nz(v); }
            0xA6 => ld!(x, am_zp),
            0xB6 => ld!(x, am_zp_y),
            0xAE => ld!(x, am_abs),
            0xBE => ld!(x, am_abs_y),
            // --- LDY ---
            0xA0 => { let v = self.fetch8(bus); self.y = v; self.set_nz(v); }
            0xA4 => ld!(y, am_zp),
            0xB4 => ld!(y, am_zp_x),
            0xAC => ld!(y, am_abs),
            0xBC => ld!(y, am_abs_x),
            // --- STA ---
            0x85 => st!(a, am_zp),
            0x95 => st!(a, am_zp_x),
            0x8D => st!(a, am_abs),
            0x9D => st!(a, am_abs_x),
            0x99 => st!(a, am_abs_y),
            0x81 => st!(a, am_ind_x),
            0x91 => st!(a, am_ind_y),
            // --- STX / STY ---
            0x86 => st!(x, am_zp),
            0x96 => st!(x, am_zp_y),
            0x8E => st!(x, am_abs),
            0x84 => st!(y, am_zp),
            0x94 => st!(y, am_zp_x),
            0x8C => st!(y, am_abs),
            // --- register transfers ---
            0xAA => { self.x = self.a; self.set_nz(self.x); self.dummy_read(bus); } // TAX
            0xA8 => { self.y = self.a; self.set_nz(self.y); self.dummy_read(bus); } // TAY
            0x8A => { self.a = self.x; self.set_nz(self.a); self.dummy_read(bus); } // TXA
            0x98 => { self.a = self.y; self.set_nz(self.a); self.dummy_read(bus); } // TYA
            0xBA => { self.x = self.sp; self.set_nz(self.x); self.dummy_read(bus); } // TSX
            0x9A => { self.sp = self.x; self.dummy_read(bus); } // TXS (no flags)
            // --- stack ---
            0x48 => { self.dummy_read(bus); self.push(bus, self.a); } // PHA
            0x08 => { self.dummy_read(bus); self.push(bus, self.p | FLAG_B | FLAG_U); } // PHP
            0x68 => { // PLA
                self.dummy_read(bus);
                let _ = self.read(bus, 0x0100 | self.sp as u16); // stack dummy
                let v = self.pull_no_predummy(bus);
                self.a = v; self.set_nz(v);
            }
            0x28 => { // PLP
                self.dummy_read(bus);
                let _ = self.read(bus, 0x0100 | self.sp as u16);
                let v = self.pull_no_predummy(bus);
                self.p = (v & !(FLAG_B)) | FLAG_U;
            }
            // --- logic ---
            0x29 => { let v = self.fetch8(bus); self.a &= v; self.set_nz(self.a); }
            0x25 => alu!(op_and_m, am_zp),
            0x35 => alu!(op_and_m, am_zp_x),
            0x2D => alu!(op_and_m, am_abs),
            0x3D => alu!(op_and_m, am_abs_x),
            0x39 => alu!(op_and_m, am_abs_y),
            0x21 => alu!(op_and_m, am_ind_x),
            0x31 => alu!(op_and_m, am_ind_y),
            0x09 => { let v = self.fetch8(bus); self.a |= v; self.set_nz(self.a); }
            0x05 => alu!(op_ora_m, am_zp),
            0x15 => alu!(op_ora_m, am_zp_x),
            0x0D => alu!(op_ora_m, am_abs),
            0x1D => alu!(op_ora_m, am_abs_x),
            0x19 => alu!(op_ora_m, am_abs_y),
            0x01 => alu!(op_ora_m, am_ind_x),
            0x11 => alu!(op_ora_m, am_ind_y),
            0x49 => { let v = self.fetch8(bus); self.a ^= v; self.set_nz(self.a); }
            0x45 => alu!(op_eor_m, am_zp),
            0x55 => alu!(op_eor_m, am_zp_x),
            0x4D => alu!(op_eor_m, am_abs),
            0x5D => alu!(op_eor_m, am_abs_x),
            0x59 => alu!(op_eor_m, am_abs_y),
            0x41 => alu!(op_eor_m, am_ind_x),
            0x51 => alu!(op_eor_m, am_ind_y),
            // --- BIT ---
            0x24 => { let a = self.am_zp(bus); let v = self.read(bus, a.addr); self.bit(v); }
            0x2C => { let a = self.am_abs(bus); let v = self.read(bus, a.addr); self.bit(v); }
            // --- ADC / SBC ---
            0x69 => { let v = self.fetch8(bus); self.adc(v); }
            0x65 => alu!(adc, am_zp),
            0x75 => alu!(adc, am_zp_x),
            0x6D => alu!(adc, am_abs),
            0x7D => alu!(adc, am_abs_x),
            0x79 => alu!(adc, am_abs_y),
            0x61 => alu!(adc, am_ind_x),
            0x71 => alu!(adc, am_ind_y),
            0xE9 | 0xEB => { let v = self.fetch8(bus); self.sbc(v); } // 0xEB = undoc USBC
            0xE5 => alu!(sbc, am_zp),
            0xF5 => alu!(sbc, am_zp_x),
            0xED => alu!(sbc, am_abs),
            0xFD => alu!(sbc, am_abs_x),
            0xF9 => alu!(sbc, am_abs_y),
            0xE1 => alu!(sbc, am_ind_x),
            0xF1 => alu!(sbc, am_ind_y),
            // --- CMP / CPX / CPY ---
            0xC9 => { let v = self.fetch8(bus); self.cmp_with(self.a, v); }
            0xC5 => alu!(cmp_a, am_zp),
            0xD5 => alu!(cmp_a, am_zp_x),
            0xCD => alu!(cmp_a, am_abs),
            0xDD => alu!(cmp_a, am_abs_x),
            0xD9 => alu!(cmp_a, am_abs_y),
            0xC1 => alu!(cmp_a, am_ind_x),
            0xD1 => alu!(cmp_a, am_ind_y),
            0xE0 => { let v = self.fetch8(bus); self.cmp_with(self.x, v); }
            0xE4 => alu!(cmp_x, am_zp),
            0xEC => alu!(cmp_x, am_abs),
            0xC0 => { let v = self.fetch8(bus); self.cmp_with(self.y, v); }
            0xC4 => alu!(cmp_y, am_zp),
            0xCC => alu!(cmp_y, am_abs),
            // --- INC / DEC (memory RMW) ---
            0xE6 => rmw!(op_inc, am_zp),
            0xF6 => rmw!(op_inc, am_zp_x),
            0xEE => rmw!(op_inc, am_abs),
            0xFE => rmw!(op_inc, am_abs_x),
            0xC6 => rmw!(op_dec, am_zp),
            0xD6 => rmw!(op_dec, am_zp_x),
            0xCE => rmw!(op_dec, am_abs),
            0xDE => rmw!(op_dec, am_abs_x),
            // --- INX/INY/DEX/DEY ---
            0xE8 => { self.x = self.x.wrapping_add(1); self.set_nz(self.x); self.dummy_read(bus); }
            0xC8 => { self.y = self.y.wrapping_add(1); self.set_nz(self.y); self.dummy_read(bus); }
            0xCA => { self.x = self.x.wrapping_sub(1); self.set_nz(self.x); self.dummy_read(bus); }
            0x88 => { self.y = self.y.wrapping_sub(1); self.set_nz(self.y); self.dummy_read(bus); }
            // --- shifts/rotates: accumulator + memory RMW ---
            0x0A => { self.dummy_read(bus); let r = self.op_asl(self.a); self.a = r; }
            0x06 => rmw!(op_asl, am_zp),
            0x16 => rmw!(op_asl, am_zp_x),
            0x0E => rmw!(op_asl, am_abs),
            0x1E => rmw!(op_asl, am_abs_x),
            0x4A => { self.dummy_read(bus); let r = self.op_lsr(self.a); self.a = r; }
            0x46 => rmw!(op_lsr, am_zp),
            0x56 => rmw!(op_lsr, am_zp_x),
            0x4E => rmw!(op_lsr, am_abs),
            0x5E => rmw!(op_lsr, am_abs_x),
            0x2A => { self.dummy_read(bus); let r = self.op_rol(self.a); self.a = r; }
            0x26 => rmw!(op_rol, am_zp),
            0x36 => rmw!(op_rol, am_zp_x),
            0x2E => rmw!(op_rol, am_abs),
            0x3E => rmw!(op_rol, am_abs_x),
            0x6A => { self.dummy_read(bus); let r = self.op_ror(self.a); self.a = r; }
            0x66 => rmw!(op_ror, am_zp),
            0x76 => rmw!(op_ror, am_zp_x),
            0x6E => rmw!(op_ror, am_abs),
            0x7E => rmw!(op_ror, am_abs_x),
            // --- flag ops ---
            0x18 => { self.set_flag(FLAG_C, false); self.dummy_read(bus); }
            0x38 => { self.set_flag(FLAG_C, true); self.dummy_read(bus); }
            0x58 => { self.set_flag(FLAG_I, false); self.dummy_read(bus); }
            0x78 => { self.set_flag(FLAG_I, true); self.dummy_read(bus); }
            0xB8 => { self.set_flag(FLAG_V, false); self.dummy_read(bus); }
            0xD8 => { self.set_flag(FLAG_D, false); self.dummy_read(bus); }
            0xF8 => { self.set_flag(FLAG_D, true); self.dummy_read(bus); }
            // --- branches ---
            0x10 => self.branch(bus, !self.flag(FLAG_N)), // BPL
            0x30 => self.branch(bus, self.flag(FLAG_N)),  // BMI
            0x50 => self.branch(bus, !self.flag(FLAG_V)), // BVC
            0x70 => self.branch(bus, self.flag(FLAG_V)),  // BVS
            0x90 => self.branch(bus, !self.flag(FLAG_C)), // BCC
            0xB0 => self.branch(bus, self.flag(FLAG_C)),  // BCS
            0xD0 => self.branch(bus, !self.flag(FLAG_Z)), // BNE
            0xF0 => self.branch(bus, self.flag(FLAG_Z)),  // BEQ
            // --- jumps / subroutines ---
            0x4C => { self.pc = self.fetch16(bus); } // JMP abs
            0x6C => { // JMP (indirect) with the page-wrap bug
                let ptr = self.fetch16(bus);
                let lo = self.read(bus, ptr) as u16;
                let hi_addr = (ptr & 0xFF00) | ((ptr + 1) & 0x00FF);
                let hi = self.read(bus, hi_addr) as u16;
                self.pc = lo | (hi << 8);
            }
            0x20 => { // JSR
                let lo = self.fetch8(bus) as u16;
                let _ = self.read(bus, 0x0100 | self.sp as u16); // internal
                self.push(bus, (self.pc >> 8) as u8);
                self.push(bus, self.pc as u8);
                let hi = self.fetch8(bus) as u16;
                self.pc = lo | (hi << 8);
            }
            0x60 => { // RTS
                self.dummy_read(bus);
                let _ = self.read(bus, 0x0100 | self.sp as u16);
                let lo = self.pull_no_predummy(bus) as u16;
                let hi = self.pull(bus) as u16;
                let target = lo | (hi << 8);
                let _ = self.read(bus, target); // dummy read of the return addr
                self.pc = target.wrapping_add(1);
            }
            0x40 => { // RTI
                self.dummy_read(bus);
                let _ = self.read(bus, 0x0100 | self.sp as u16);
                let p = self.pull_no_predummy(bus);
                self.p = (p & !FLAG_B) | FLAG_U;
                let lo = self.pull(bus) as u16;
                let hi = self.pull(bus) as u16;
                self.pc = lo | (hi << 8);
            }
            0x00 => self.brk(bus), // BRK
            0xEA => self.dummy_read(bus), // NOP
            _ => self.undocumented(bus, op),
        }
    }

    /// One internal cycle: the 6502 issues a dummy read of the next opcode byte
    /// (at PC) without advancing it. Used by all implied/accumulator ops.
    fn dummy_read(&mut self, bus: &mut impl Bus) {
        let _ = self.read(bus, self.pc);
    }

    /// PLA/PLP/RTS/RTI read the stack byte after their own dummy stack read, so
    /// they must not repeat the pre-increment dummy. This pulls without it.
    fn pull_no_predummy(&mut self, bus: &mut impl Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.read(bus, STACK_BASE | self.sp as u16)
    }

    fn op_and_m(&mut self, v: u8) { self.a &= v; self.set_nz(self.a); }
    fn op_ora_m(&mut self, v: u8) { self.a |= v; self.set_nz(self.a); }
    fn op_eor_m(&mut self, v: u8) { self.a ^= v; self.set_nz(self.a); }
    fn cmp_a(&mut self, v: u8) { self.cmp_with(self.a, v); }
    fn cmp_x(&mut self, v: u8) { self.cmp_with(self.x, v); }
    fn cmp_y(&mut self, v: u8) { self.cmp_with(self.y, v); }
    fn bit(&mut self, v: u8) {
        self.set_flag(FLAG_Z, self.a & v == 0);
        self.set_flag(FLAG_N, v & 0x80 != 0);
        self.set_flag(FLAG_V, v & 0x40 != 0);
    }
}

mod illegal;
