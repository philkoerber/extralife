//! Clean-room Epson E0C6200 core CPU (as used in the E0C6S46 MCU that drives
//! the Tamagotchi P1). Implemented from the Epson "E0C6S46 Technical Manual"
//! instruction tables 2.1.1(a)–(c) and section 2 (ROM/RAM/PC) — NOT from any
//! GPL emulator source (TamaLib et al. are read-only reference material only).
//!
//! Architecture (from the manual):
//!   - 4-bit ALU. Registers A, B (4-bit each).
//!   - Index registers IX = XP.XH.XL, IY = YP.YH.YL (each 12-bit: 4+4+4).
//!   - Stack pointer SP (8-bit), addresses RAM as M(SP).
//!   - Program counter PC = PCB(1 bit bank).PCP(4 bit page).PCS(8 bit step) = 13 bits.
//!   - Branches go through NBP/NPP set by `PSET p`; a taken branch loads
//!     PCB<-NBP, PCP<-NPP, PCS<-immediate, then NBP/NPP reset to PCB/PCP.
//!   - Flags F = I (interrupt), D (decimal), Z (zero), C (carry).
//!   - All instructions are one 12-bit word; timing is 5, 7 or 12 clocks.
//!
//! The `Bus` trait is the seam to memory-mapped RAM / display / I/O so the CPU
//! stays a pure decoder+ALU and the system wiring lives in `lib.rs`.

/// Memory-mapped data bus (RAM + display + I/O), addressed by 12-bit word.
/// Each cell is a 4-bit nibble (only low 4 bits are meaningful).
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
}

/// 2-bit register/memory selector code used by `r`/`q` operands.
/// 0=A, 1=B, 2=M(X), 3=M(Y) (manual §2.1, "r q Register" table).
#[derive(Clone, Copy)]
enum Rq {
    A,
    B,
    Mx,
    My,
}

impl Rq {
    fn from_bits(b: u8) -> Rq {
        match b & 3 {
            0 => Rq::A,
            1 => Rq::B,
            2 => Rq::Mx,
            _ => Rq::My,
        }
    }
}

pub const C_FLAG: u8 = 0b0001;
pub const Z_FLAG: u8 = 0b0010;
pub const D_FLAG: u8 = 0b0100;
pub const I_FLAG: u8 = 0b1000;

pub struct Cpu {
    pub a: u8,
    pub b: u8,
    pub xp: u8,
    pub xh: u8,
    pub xl: u8,
    pub yp: u8,
    pub yh: u8,
    pub yl: u8,
    pub sp: u8,
    /// Program counter, held decomposed: bank (1 bit), page (4 bits), step (8 bits).
    pub pcb: u8,
    pub pcp: u8,
    pub pcs: u8,
    /// New bank/page pointers, loaded by PSET, consumed by the next branch.
    pub nbp: u8,
    pub npp: u8,
    pub flags: u8,
    /// HALT stops the CPU clock until an interrupt; the system clock keeps
    /// ticking timers. Modeled as "skip instruction execution while halted".
    pub halted: bool,
    /// Clock cycles consumed by the last executed instruction (5/7/12).
    pub last_cycles: u32,
    /// Set right after a PSET so the "no interrupt directly after PSET/branch
    /// prep" behavior can be honored if needed. (Kept simple: interrupts are
    /// polled between instructions in the system layer.)
    pub after_pset: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            a: 0,
            b: 0,
            xp: 0,
            xh: 0,
            xl: 0,
            yp: 0,
            yh: 0,
            yl: 0,
            sp: 0,
            // Initial reset: bank 0, page 1, step 0x00 (manual §2.2).
            pcb: 0,
            pcp: 1,
            pcs: 0,
            nbp: 0,
            npp: 1,
            flags: 0,
            halted: false,
            last_cycles: 0,
            after_pset: false,
        }
    }

    /// Full 13-bit PC as a linear ROM word index: bank<<12 | page<<8 | step.
    pub fn pc(&self) -> u16 {
        ((self.pcb as u16) << 12) | ((self.pcp as u16) << 8) | self.pcs as u16
    }

    fn ix(&self) -> u16 {
        ((self.xp as u16) << 8) | ((self.xh as u16) << 4) | self.xl as u16
    }

    fn iy(&self) -> u16 {
        ((self.yp as u16) << 8) | ((self.yh as u16) << 4) | self.yl as u16
    }

    fn set_ix(&mut self, v: u16) {
        self.xp = ((v >> 8) & 0xF) as u8;
        self.xh = ((v >> 4) & 0xF) as u8;
        self.xl = (v & 0xF) as u8;
    }

    fn set_iy(&mut self, v: u16) {
        self.yp = ((v >> 8) & 0xF) as u8;
        self.yh = ((v >> 4) & 0xF) as u8;
        self.yl = (v & 0xF) as u8;
    }

    fn inc_x(&mut self) {
        self.set_ix((self.ix() + 1) & 0xFFF);
    }

    fn inc_y(&mut self) {
        self.set_iy((self.iy() + 1) & 0xFFF);
    }

    /// Advance PCS by one step, wrapping within the current page (8-bit step).
    /// Fetching does NOT cross pages automatically except by wrap; real code
    /// keeps routines within a page and uses JP/CALL to move pages.
    fn incr_pc(&mut self) {
        self.pcs = self.pcs.wrapping_add(1);
    }

    fn zero_from(&mut self, v: u8) {
        if v & 0xF == 0 {
            self.flags |= Z_FLAG;
        } else {
            self.flags &= !Z_FLAG;
        }
    }

    fn set_c(&mut self, carry: bool) {
        if carry {
            self.flags |= C_FLAG;
        } else {
            self.flags &= !C_FLAG;
        }
    }

    // --- r/q operand access ------------------------------------------------

    fn read_rq<B: Bus>(&mut self, bus: &mut B, r: Rq) -> u8 {
        match r {
            Rq::A => self.a,
            Rq::B => self.b,
            Rq::Mx => bus.read(self.ix()) & 0xF,
            Rq::My => bus.read(self.iy()) & 0xF,
        }
    }

    fn write_rq<B: Bus>(&mut self, bus: &mut B, r: Rq, v: u8) {
        let v = v & 0xF;
        match r {
            Rq::A => self.a = v,
            Rq::B => self.b = v,
            Rq::Mx => bus.write(self.ix(), v),
            Rq::My => bus.write(self.iy(), v),
        }
    }

    // --- 4-bit ALU primitives (flags: C set/reset, Z set/reset) ------------
    //
    // Decimal-adjust (`★` in the manual): ADD/ADC/SUB/SBC and the ACPX/ACPY/
    // SCPX/SCPY memory ops adjust the result when the D flag is set, so BCD
    // arithmetic works. Adjust rule from the E0C6200 core: on add, if D set and
    // (result > 9 or carry) add 6 and force carry; on subtract, if D set and
    // (borrow or result > 9) subtract 6.

    fn op_add(&mut self, x: u8, y: u8) -> u8 {
        let mut r = (x & 0xF) + (y & 0xF);
        let mut carry = r > 0xF;
        if self.flags & D_FLAG != 0 && (r > 9) {
            r += 6;
            carry = true;
        }
        let r = r & 0xF;
        self.set_c(carry);
        self.zero_from(r);
        r
    }

    fn op_adc(&mut self, x: u8, y: u8) -> u8 {
        let cin = self.flags & C_FLAG;
        let mut r = (x & 0xF) + (y & 0xF) + cin;
        let mut carry = r > 0xF;
        if self.flags & D_FLAG != 0 && r > 9 {
            r += 6;
            carry = true;
        }
        let r = r & 0xF;
        self.set_c(carry);
        self.zero_from(r);
        r
    }

    /// ADC without decimal adjust — used for ADC XH/XL/YH/YL,i, which operate
    /// on address nibbles (manual: D "not affected", no `★`).
    fn op_adc_nodec(&mut self, x: u8, y: u8) -> u8 {
        let cin = self.flags & C_FLAG;
        let r = (x & 0xF) + (y & 0xF) + cin;
        let carry = r > 0xF;
        let r = r & 0xF;
        self.set_c(carry);
        self.zero_from(r);
        r
    }

    fn op_sub(&mut self, x: u8, y: u8) -> u8 {
        let x = x & 0xF;
        let y = y & 0xF;
        let mut borrow = x < y;
        let mut r = x.wrapping_sub(y) & 0x1F;
        if self.flags & D_FLAG != 0 && borrow {
            r = r.wrapping_sub(6);
        }
        let out = r & 0xF;
        // Recompute borrow across the decimal fixup path.
        borrow = x < y;
        self.set_c(borrow);
        self.zero_from(out);
        out
    }

    fn op_sbc(&mut self, x: u8, y: u8) -> u8 {
        let cin = self.flags & C_FLAG;
        let x = x & 0xF;
        let y = y & 0xF;
        let borrow = (x as i8 - y as i8 - cin as i8) < 0;
        let mut r = x.wrapping_sub(y).wrapping_sub(cin) & 0x1F;
        if self.flags & D_FLAG != 0 && borrow {
            r = r.wrapping_sub(6);
        }
        let out = r & 0xF;
        self.set_c(borrow);
        self.zero_from(out);
        out
    }

    /// CP r,i / CP r,q: compute r - operand, set C (borrow) and Z, discard result.
    fn op_cp(&mut self, x: u8, y: u8) {
        let x = x & 0xF;
        let y = y & 0xF;
        self.set_c(x < y);
        self.zero_from(x.wrapping_sub(y) & 0xF);
    }

    fn op_and(&mut self, x: u8, y: u8) -> u8 {
        let r = x & y & 0xF;
        self.zero_from(r);
        r
    }

    fn op_or(&mut self, x: u8, y: u8) -> u8 {
        let r = (x | y) & 0xF;
        self.zero_from(r);
        r
    }

    fn op_xor(&mut self, x: u8, y: u8) -> u8 {
        let r = (x ^ y) & 0xF;
        self.zero_from(r);
        r
    }

    /// FAN r,i / FAN r,q: AND for test only — sets Z, leaves operands, C unchanged.
    fn op_fan(&mut self, x: u8, y: u8) {
        self.zero_from(x & y & 0xF);
    }

    fn op_not(&mut self, x: u8) -> u8 {
        let r = (!x) & 0xF;
        self.zero_from(r);
        r
    }

    fn op_rlc(&mut self, x: u8) -> u8 {
        let cin = self.flags & C_FLAG;
        let r = ((x << 1) | cin) & 0xF;
        self.set_c(x & 0x8 != 0);
        // RLC affects Z per the manual (Z column marked); reflect result.
        self.zero_from(r);
        r
    }

    fn op_rrc(&mut self, x: u8) -> u8 {
        let cin = self.flags & C_FLAG;
        let r = ((x >> 1) | (cin << 3)) & 0xF;
        self.set_c(x & 0x1 != 0);
        self.zero_from(r);
        r
    }

    fn op_inc(&mut self, x: u8) -> u8 {
        let r = x.wrapping_add(1) & 0xF;
        self.zero_from(r);
        r
    }

    fn op_dec(&mut self, x: u8) -> u8 {
        let r = x.wrapping_sub(1) & 0xF;
        self.zero_from(r);
        r
    }

    // --- Stack -------------------------------------------------------------

    fn push_nibble<B: Bus>(&mut self, bus: &mut B, v: u8) {
        self.sp = self.sp.wrapping_sub(1);
        bus.write(self.sp as u16, v & 0xF);
    }

    fn pop_nibble<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = bus.read(self.sp as u16) & 0xF;
        self.sp = self.sp.wrapping_add(1);
        v
    }

    /// Push return address for CALL/CALZ: PCP, PCSH, PCSL+1 (manual §2.1).
    /// The saved step is the address of the CALL plus one (return past it).
    fn push_return<B: Bus>(&mut self, bus: &mut B) {
        let ret = self.pcs; // PC already advanced past the CALL word.
        self.push_nibble(bus, self.pcp);
        self.push_nibble(bus, (ret >> 4) & 0xF);
        self.push_nibble(bus, ret & 0xF);
    }

    fn ret_from_stack<B: Bus>(&mut self, bus: &mut B) {
        let pcsl = self.pop_nibble(bus);
        let pcsh = self.pop_nibble(bus);
        let pcp = self.pop_nibble(bus);
        self.pcs = (pcsh << 4) | pcsl;
        self.pcp = pcp;
    }

    /// Load NBP/NPP into the PC bank/page and take a branch to step `s`,
    /// then reset NBP/NPP to the (new) current bank/page for the next branch.
    fn branch_to(&mut self, s: u8) {
        self.pcb = self.nbp & 1;
        self.pcp = self.npp & 0xF;
        self.pcs = s;
        self.nbp = self.pcb;
        self.npp = self.pcp;
    }

    /// After a non-taken conditional branch (or any non-PSET instruction),
    /// NBP/NPP fall back to the current bank/page.
    fn sync_nbp_npp(&mut self) {
        self.nbp = self.pcb;
        self.npp = self.pcp;
    }

    /// Fetch and execute one instruction. `rom` is the linear 12-bit word
    /// array (index = bank<<12 | page<<8 | step). Returns clocks consumed.
    pub fn step<B: Bus>(&mut self, rom: &[u16], bus: &mut B) -> u32 {
        if self.halted {
            self.last_cycles = 5;
            return 5;
        }
        let op = rom.get(self.pc() as usize).copied().unwrap_or(0) & 0xFFF;
        self.after_pset = false;
        self.incr_pc();
        self.last_cycles = 5;
        self.execute(op, bus);
        self.last_cycles
    }

    /// Full 12-bit opcode dispatch. Encodings and semantics are taken verbatim
    /// from the Epson S1C6200/6200A Core CPU Manual §3.5 (per-instruction
    /// descriptions with exact OP-Code hex ranges). Clean-room: no emulator
    /// source consulted.
    #[allow(clippy::too_many_lines)]
    fn execute<B: Bus>(&mut self, op: u16, bus: &mut B) {
        let hi = (op >> 8) & 0xF;
        let s = (op & 0xFF) as u8;
        let i4 = (op & 0xF) as u8;
        // Two operand encodings the E0C6200 uses:
        //  - r,i forms: r is in bits [5:4], immediate i in [3:0].
        //  - r,q forms: r is in bits [3:2], q in [1:0].
        let r_ri = Rq::from_bits(((op >> 4) & 3) as u8);
        let r_rq = Rq::from_bits(((op >> 2) & 3) as u8);
        let q = Rq::from_bits((op & 3) as u8);

        match hi {
            0x0 => {
                // JP s
                self.branch_to(s);
            }
            0x1 => {
                // RETD e: return, then M(X)<-e_lo, M(X+1)<-e_hi, X+=2
                self.ret_from_stack(bus);
                let x = self.ix();
                bus.write(x, s & 0xF);
                bus.write((x + 1) & 0xFFF, (s >> 4) & 0xF);
                self.set_ix((x + 2) & 0xFFF);
                self.last_cycles = 12;
            }
            0x2 => self.cond_jp(self.flags & C_FLAG != 0, s), // JP C,s
            0x3 => self.cond_jp(self.flags & C_FLAG == 0, s), // JP NC,s
            0x4 => {
                // CALL s
                self.push_return(bus);
                self.pcp = self.npp & 0xF;
                self.pcb = self.nbp & 1;
                self.pcs = s;
                self.sync_nbp_npp();
                self.last_cycles = 7;
            }
            0x5 => {
                // CALZ s (page 0 of current bank; NPP reset to 0)
                self.push_return(bus);
                self.pcb = self.nbp & 1;
                self.pcp = 0;
                self.pcs = s;
                self.sync_nbp_npp();
                self.last_cycles = 7;
            }
            0x6 => self.cond_jp(self.flags & Z_FLAG != 0, s), // JP Z,s
            0x7 => self.cond_jp(self.flags & Z_FLAG == 0, s), // JP NZ,s
            0x8 => {
                // LD Y,e
                self.yh = (s >> 4) & 0xF;
                self.yl = s & 0xF;
            }
            0x9 => {
                // LBPX MX,e: M(X)<-e_lo, M(X+1)<-e_hi, X+=2
                let x = self.ix();
                bus.write(x, s & 0xF);
                bus.write((x + 1) & 0xFFF, (s >> 4) & 0xF);
                self.set_ix((x + 2) & 0xFFF);
            }
            0xA => self.exec_a(op, i4, r_rq, q, bus),
            0xB => {
                // LD X,e
                self.xh = (s >> 4) & 0xF;
                self.xl = s & 0xF;
            }
            0xC => {
                // r,i arithmetic/logical block (110xxx). r is in bits [5:4].
                // Decimal-adjust (D) applies to ADD/ADC.
                let v = self.read_rq(bus, r_ri);
                let out = match (op >> 6) & 3 {
                    0 => self.op_add(v, i4), // ADD r,i  C00-C3F
                    1 => self.op_adc(v, i4), // ADC r,i  C40-C7F
                    2 => self.op_and(v, i4), // AND r,i  C80-CBF
                    _ => self.op_or(v, i4),  // OR  r,i  CC0-CFF
                };
                self.write_rq(bus, r_ri, out);
                self.last_cycles = 7;
            }
            0xD => self.exec_d(op, i4, r_ri, bus),
            0xE => self.exec_e(op, i4, r_ri, r_rq, q, bus),
            0xF => self.exec_f(op, i4, r_rq, bus),
            _ => {}
        }
    }

    fn cond_jp(&mut self, take: bool, s: u8) {
        if take {
            self.branch_to(s);
        } else {
            self.sync_nbp_npp();
        }
    }

    /// 0xA block: index-register immediates (ADC/CP XH..YL,i), r,q arithmetic
    /// and logical ops, and RLC r.
    fn exec_a<B: Bus>(&mut self, op: u16, i4: u8, r: Rq, q: Rq, bus: &mut B) {
        self.last_cycles = 7;
        match (op >> 4) & 0xF {
            // A0x: ADC XH,i / XL,i / YH,i / YL,i  (A00-A3F, sub in bits[5:4])
            0x0 => {
                let sel = (op >> 4) & 3; // 0=XH 1=XL 2=YH 3=YL
                let cur = match sel {
                    0 => self.xh,
                    1 => self.xl,
                    2 => self.yh,
                    _ => self.yl,
                };
                let out = self.op_adc_nodec(cur, i4);
                match sel {
                    0 => self.xh = out,
                    1 => self.xl = out,
                    2 => self.yh = out,
                    _ => self.yl = out,
                }
            }
            // A4x: CP XH,i / XL,i / YH,i / YL,i  (A40-A7F)
            0x4 => {
                let sel = (op >> 4) & 3;
                let cur = match sel {
                    0 => self.xh,
                    1 => self.xl,
                    2 => self.yh,
                    _ => self.yl,
                };
                self.op_cp(cur, i4);
            }
            0x8 => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_add(v, qv); // ADD r,q  A80-A8F
                self.write_rq(bus, r, out);
            }
            0x9 => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_adc(v, qv); // ADC r,q  A90-A9F
                self.write_rq(bus, r, out);
            }
            0xA => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_sub(v, qv); // SUB r,q  AA0-AAF
                self.write_rq(bus, r, out);
            }
            0xB => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_sbc(v, qv); // SBC r,q  AB0-ABF
                self.write_rq(bus, r, out);
            }
            0xC => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_and(v, qv); // AND r,q  AC0-ACF
                self.write_rq(bus, r, out);
            }
            0xD => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_or(v, qv); // OR r,q  AD0-ADF
                self.write_rq(bus, r, out);
            }
            0xE => {
                let v = self.read_rq(bus, r);
                let qv = self.read_rq(bus, q);
                let out = self.op_xor(v, qv); // XOR r,q  AE0-AEF
                self.write_rq(bus, r, out);
            }
            0xF => {
                // RLC r  AF0-AFF (r duplicated in low nibbles)
                let v = self.read_rq(bus, r);
                let out = self.op_rlc(v);
                self.write_rq(bus, r, out);
            }
            _ => {}
        }
    }

    /// 0xD block: XOR/SBC/FAN/CP r,i, NOT r, and the D0F/D1F.. NOT special case.
    fn exec_d<B: Bus>(&mut self, op: u16, i4: u8, r: Rq, bus: &mut B) {
        self.last_cycles = 7;
        match (op >> 6) & 3 {
            0 => {
                // 110100 => XOR r,i (D00-D3F), except low nibble F => NOT r
                if i4 == 0xF {
                    let v = self.read_rq(bus, r);
                    let out = self.op_not(v);
                    self.write_rq(bus, r, out);
                } else {
                    let v = self.read_rq(bus, r);
                    let out = self.op_xor(v, i4);
                    self.write_rq(bus, r, out);
                }
            }
            1 => {
                // 110101 => SBC r,i (D40-D7F)
                let v = self.read_rq(bus, r);
                let out = self.op_sbc(v, i4);
                self.write_rq(bus, r, out);
            }
            2 => {
                // 110110 => FAN r,i (D80-DBF)
                let v = self.read_rq(bus, r);
                self.op_fan(v, i4);
            }
            _ => {
                // 110111 => CP r,i (DC0-DFF)
                let v = self.read_rq(bus, r);
                self.op_cp(v, i4);
            }
        }
    }

    /// 0xE block: LD r,i; PSET; LDPX/LDPY; inter-register loads; RRC.
    fn exec_e<B: Bus>(&mut self, op: u16, i4: u8, r_ri: Rq, r_rq: Rq, q: Rq, bus: &mut B) {
        match (op >> 4) & 0xF {
            // E00-E3F: LD r,i  (111000 r i) — r is bits[5:4].
            0x0..=0x3 => {
                self.write_rq(bus, r_ri, i4);
                self.last_cycles = 5;
            }
            // E40-E5F: PSET p  (1110010 p) — arm NBP/NPP for the next branch.
            0x4 | 0x5 => {
                let p = (op & 0x1F) as u8;
                self.npp = p & 0xF;
                self.nbp = (p >> 4) & 1;
                self.after_pset = true;
                self.last_cycles = 5;
            }
            // E60-E6F: LDPX MX,i
            0x6 => {
                bus.write(self.ix(), i4);
                self.inc_x();
                self.last_cycles = 5;
            }
            // E70-E7F: LDPY MY,i
            0x7 => {
                bus.write(self.iy(), i4);
                self.inc_y();
                self.last_cycles = 5;
            }
            // E80-EFF: register/register block (r in bits[3:2], q in [1:0]).
            _ => self.exec_e_regs(op, r_rq, q, bus),
        }
    }

    fn exec_e_regs<B: Bus>(&mut self, op: u16, r: Rq, q: Rq, bus: &mut B) {
        self.last_cycles = 5;
        match (op >> 4) & 0xF {
            0x8 => match (op >> 2) & 3 {
                0 => self.xp = self.read_rq(bus, r), // LD XP,r  E80-E83
                1 => self.xh = self.read_rq(bus, r), // LD XH,r  E84-E87
                2 => self.xl = self.read_rq(bus, r), // LD XL,r  E88-E8B
                _ => {
                    // RRC r  E8C-E8F
                    let v = self.read_rq(bus, r);
                    let out = self.op_rrc(v);
                    self.write_rq(bus, r, out);
                    self.last_cycles = 5;
                }
            },
            0x9 => match (op >> 2) & 3 {
                0 => self.yp = self.read_rq(bus, r), // LD YP,r  E90-E93
                1 => self.yh = self.read_rq(bus, r), // LD YH,r  E94-E97
                2 => self.yl = self.read_rq(bus, r), // LD YL,r  E98-E9B
                _ => {}
            },
            0xA => {
                let val = match (op >> 2) & 3 {
                    0 => self.xp, // LD r,XP  EA0-EA3
                    1 => self.xh, // LD r,XH  EA4-EA7
                    2 => self.xl, // LD r,XL  EA8-EAB
                    _ => return,
                };
                self.write_rq(bus, r, val);
            }
            0xB => {
                let val = match (op >> 2) & 3 {
                    0 => self.yp, // LD r,YP  EB0-EB3
                    1 => self.yh, // LD r,YH  EB4-EB7
                    2 => self.yl, // LD r,YL  EB8-EBB
                    _ => return,
                };
                self.write_rq(bus, r, val);
            }
            0xC => {
                // LD r,q  EC0-ECF
                let v = self.read_rq(bus, q);
                self.write_rq(bus, r, v);
            }
            0xE => {
                // LDPX r,q  EE0-EEF  (also encodes INC X as LDPX A,A)
                let v = self.read_rq(bus, q);
                self.write_rq(bus, r, v);
                self.inc_x();
            }
            0xF => {
                // LDPY r,q  EF0-EFF  (also encodes INC Y as LDPY A,A)
                let v = self.read_rq(bus, q);
                self.write_rq(bus, r, v);
                self.inc_y();
            }
            _ => {}
        }
    }

    /// 0xF block: CP/FAN r,q, ACPX/ACPY/SCPX/SCPY, SET/RST flags, INC/DEC Mn,
    /// LD Mn,A/B & A/B,Mn, stack PUSH/POP, SP loads, RET/RETS/RETD, JPBA,
    /// HALT, NOP5, NOP7.
    #[allow(clippy::too_many_lines)]
    fn exec_f<B: Bus>(&mut self, op: u16, i4: u8, r: Rq, bus: &mut B) {
        let lo8 = (op & 0xFF) as u8;
        match (op >> 4) & 0xF {
            0x0 => {
                // F00-F0F: CP r,q
                let v = self.read_rq(bus, r);
                let q = Rq::from_bits((op & 3) as u8);
                let qv = self.read_rq(bus, q);
                self.op_cp(v, qv);
                self.last_cycles = 7;
            }
            0x1 => {
                // F10-F1F: FAN r,q
                let v = self.read_rq(bus, r);
                let q = Rq::from_bits((op & 3) as u8);
                let qv = self.read_rq(bus, q);
                self.op_fan(v, qv);
                self.last_cycles = 7;
            }
            0x2 => {
                // F28-F2B ACPX MX,r ; F2C-F2F ACPY MY,r
                self.last_cycles = 7;
                let rr = Rq::from_bits((op & 3) as u8);
                let rv = self.read_rq(bus, rr);
                if op & 0x4 == 0 {
                    let m = bus.read(self.ix()) & 0xF;
                    let out = self.op_adc(m, rv);
                    bus.write(self.ix(), out);
                    self.inc_x();
                } else {
                    let m = bus.read(self.iy()) & 0xF;
                    let out = self.op_adc(m, rv);
                    bus.write(self.iy(), out);
                    self.inc_y();
                }
            }
            0x3 => {
                // F38-F3B SCPX MX,r ; F3C-F3F SCPY MY,r
                self.last_cycles = 7;
                let rr = Rq::from_bits((op & 3) as u8);
                let rv = self.read_rq(bus, rr);
                if op & 0x4 == 0 {
                    let m = bus.read(self.ix()) & 0xF;
                    let out = self.op_sbc(m, rv);
                    bus.write(self.ix(), out);
                    self.inc_x();
                } else {
                    let m = bus.read(self.iy()) & 0xF;
                    let out = self.op_sbc(m, rv);
                    bus.write(self.iy(), out);
                    self.inc_y();
                }
            }
            0x4 => {
                // F40-F4F: SET F,i  (OR i into flags; SCF/SDF/EI are subsets)
                self.flags |= i4;
                self.last_cycles = 7;
            }
            0x5 => {
                // F50-F5F: RST F,i  (AND ~i into flags; RCF/RDF/RZF/DI subsets)
                self.flags &= !i4;
                self.last_cycles = 7;
            }
            0x6 => {
                // F60-F6F: INC Mn  (n = low nibble; Mn = address 0x000..0x00F)
                let m = bus.read(i4 as u16) & 0xF;
                let out = self.op_inc(m);
                bus.write(i4 as u16, out);
                self.last_cycles = 7;
            }
            0x7 => {
                // F70-F7F: DEC Mn
                let m = bus.read(i4 as u16) & 0xF;
                let out = self.op_dec(m);
                bus.write(i4 as u16, out);
                self.last_cycles = 7;
            }
            0x8 => {
                // F80-F8F: LD Mn,A
                bus.write(i4 as u16, self.a);
                self.last_cycles = 5;
            }
            0x9 => {
                // F90-F9F: LD Mn,B
                bus.write(i4 as u16, self.b);
                self.last_cycles = 5;
            }
            0xA => {
                // FA0-FAF: LD A,Mn
                self.a = bus.read(i4 as u16) & 0xF;
                self.last_cycles = 5;
            }
            0xB => {
                // FB0-FBF: LD B,Mn
                self.b = bus.read(i4 as u16) & 0xF;
                self.last_cycles = 5;
            }
            0xC => self.exec_fc(op, lo8, r, bus), // PUSH / DEC SP / SPH loads
            0xD => self.exec_fd(op, lo8, r, bus), // POP / INC SP / RET(S/D)
            0xE => self.exec_fe(lo8, r, bus),     // LD SPH,r / r,SPH / JPBA / HALT
            _ => self.exec_ff(lo8, r, bus),       // LD SPL,r / r,SPL / SLP / NOP
        }
    }

    fn exec_fc<B: Bus>(&mut self, op: u16, lo8: u8, r: Rq, bus: &mut B) {
        self.last_cycles = 5;
        // FC0-FC3 PUSH r ; FC4 PUSH XP; FC5 XH; FC6 XL; FC7 YP; FC8 YH; FC9 YL;
        // FCA PUSH F ; FCB DEC SP.
        match lo8 {
            0xC0..=0xC3 => {
                let v = self.read_rq(bus, r);
                self.push_nibble(bus, v);
            }
            0xC4 => self.push_nibble(bus, self.xp),
            0xC5 => self.push_nibble(bus, self.xh),
            0xC6 => self.push_nibble(bus, self.xl),
            0xC7 => self.push_nibble(bus, self.yp),
            0xC8 => self.push_nibble(bus, self.yh),
            0xC9 => self.push_nibble(bus, self.yl),
            0xCA => self.push_nibble(bus, self.flags),
            0xCB => self.sp = self.sp.wrapping_sub(1), // DEC SP
            _ => {}
        }
        let _ = op;
    }

    fn exec_fd<B: Bus>(&mut self, op: u16, lo8: u8, r: Rq, bus: &mut B) {
        self.last_cycles = 5;
        // FD0-FD3 POP r ; FD4 XP; FD5 XH; FD6 XL; FD7 YP; FD8 YH; FD9 YL;
        // FDA POP F ; FDB INC SP ; FDE RETS ; FDF RET.
        match lo8 {
            0xD0..=0xD3 => {
                let v = self.pop_nibble(bus);
                self.write_rq(bus, r, v);
            }
            0xD4 => self.xp = self.pop_nibble(bus),
            0xD5 => self.xh = self.pop_nibble(bus),
            0xD6 => self.xl = self.pop_nibble(bus),
            0xD7 => self.yp = self.pop_nibble(bus),
            0xD8 => self.yh = self.pop_nibble(bus),
            0xD9 => self.yl = self.pop_nibble(bus),
            0xDA => self.flags = self.pop_nibble(bus) & 0xF,
            0xDB => self.sp = self.sp.wrapping_add(1), // INC SP
            0xDE => {
                // RETS: return then PC <- PC+1 (skip the word after the call)
                self.ret_from_stack(bus);
                self.incr_pc();
                self.last_cycles = 12;
            }
            0xDF => {
                // RET
                self.ret_from_stack(bus);
                self.last_cycles = 12;
            }
            _ => {}
        }
        let _ = op;
    }

    fn exec_fe<B: Bus>(&mut self, lo8: u8, r: Rq, bus: &mut B) {
        self.last_cycles = 5;
        // FE0-FE3 LD SPH,r ; FE4-FE7 LD r,SPH ; FE8 JPBA ; (E40 was PSET).
        match lo8 {
            0xE0..=0xE3 => {
                let v = self.read_rq(bus, r);
                self.sp = (self.sp & 0x0F) | (v << 4);
            }
            0xE4..=0xE7 => {
                let v = (self.sp >> 4) & 0xF;
                self.write_rq(bus, r, v);
            }
            0xE8 => {
                // JPBA: PCB<-NBP, PCP<-NPP, PCSH<-B, PCSL<-A
                self.pcb = self.nbp & 1;
                self.pcp = self.npp & 0xF;
                self.pcs = (self.b << 4) | (self.a & 0xF);
                self.sync_nbp_npp();
            }
            _ => {}
        }
    }

    fn exec_ff<B: Bus>(&mut self, lo8: u8, r: Rq, bus: &mut B) {
        self.last_cycles = 5;
        // FF0-FF3 LD SPL,r ; FF4-FF7 LD r,SPL ; FF8 HALT ; FF9 SLP ;
        // FFB NOP5 ; FFF NOP7.
        match lo8 {
            0xF0..=0xF3 => {
                let v = self.read_rq(bus, r);
                self.sp = (self.sp & 0xF0) | v;
            }
            0xF4..=0xF7 => {
                let v = self.sp & 0xF;
                self.write_rq(bus, r, v);
            }
            0xF8 => self.halted = true, // HALT (stop CPU clock until interrupt)
            0xF9 => self.halted = true, // SLP — not used on E0C6S46; treat as HALT
            0xFB => {}                  // NOP5
            0xFF => self.last_cycles = 7, // NOP7
            _ => {}
        }
    }

    /// Enter an interrupt: push the current PC (PCP, PCSH, PCSL), clear the I
    /// flag (DI), and vector to (page, step) in the current bank. Wakes the CPU
    /// from HALT. (Epson E0C6S46 Technical Manual §2.5.)
    pub fn interrupt<B: Bus>(&mut self, bus: &mut B, page: u8, step: u8) {
        let ret = self.pcs;
        self.push_nibble(bus, self.pcp);
        self.push_nibble(bus, (ret >> 4) & 0xF);
        self.push_nibble(bus, ret & 0xF);
        self.flags &= !I_FLAG;
        self.pcp = page & 0xF;
        self.pcs = step;
        self.sync_nbp_npp();
        self.halted = false;
    }

    /// Fixed-size register snapshot for save states.
    pub fn serialize(&self) -> [u8; 16] {
        [
            self.a, self.b, self.xp, self.xh, self.xl, self.yp, self.yh, self.yl, self.sp,
            self.pcb, self.pcp, self.pcs, self.nbp, self.npp, self.flags, self.halted as u8,
        ]
    }

    pub fn serialized_len() -> usize {
        16
    }

    pub fn deserialize(b: &[u8]) -> Option<Cpu> {
        if b.len() != 16 {
            return None;
        }
        Some(Cpu {
            a: b[0],
            b: b[1],
            xp: b[2],
            xh: b[3],
            xl: b[4],
            yp: b[5],
            yh: b[6],
            yl: b[7],
            sp: b[8],
            pcb: b[9],
            pcp: b[10],
            pcs: b[11],
            nbp: b[12],
            npp: b[13],
            flags: b[14],
            halted: b[15] != 0,
            last_cycles: 0,
            after_pset: false,
        })
    }
}
