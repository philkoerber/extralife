//! Ricoh 2A03 CPU core: a MOS 6502 with decimal (BCD) mode disabled.
//!
//! The 2A03 is a stock 6502 minus the decimal ALU path — `ADC`/`SBC` always
//! compute in binary regardless of the D flag (the flag bit still stores/loads
//! via PHP/PLP/SED/CLD, it just has no arithmetic effect). Everything else is a
//! textbook NMOS 6502, including the documented instruction set and the handful
//! of quirks the SingleStepTests exercise (page-crossing dummy reads, the JMP
//! ($xxFF) indirect-fetch bug, BRK's B flag on the stack).
//!
//! Cycle model: the 6502 touches the bus on *every* cycle (there are no true
//! internal cycles — even "internal operation" cycles perform a dummy read).
//! `Bus::read`/`Bus::write` therefore each represent exactly one CPU cycle, and
//! the SingleStepTests validate the full ordered list of (address, value, kind)
//! bus events per instruction. This matches how the PPU/APU are clocked: three
//! PPU dots per CPU cycle, advanced from inside the bus callbacks at the system
//! level.
//!
//! Clean-room: implemented from the NESdev wiki (6502 reference, "CPU unofficial
//! opcodes", "CPU addressing modes") and the widely published MOS datasheet
//! behavior. No emulator source was translated (license-policy: ares is ISC and
//! portable, but this core is documentation-derived; Mesen2 is read-only).

/// The memory seam the CPU drives. Every `read`/`write` is exactly one CPU
/// cycle; the system implementation advances the PPU/APU/DMA inside these.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Pending maskable interrupt request line (IRQ), level-sensitive: true
    /// while any source (APU frame/DMC, mapper IRQ) holds the line low.
    fn irq_pending(&self) -> bool {
        false
    }
    /// Pending non-maskable interrupt (NMI), edge-triggered by the PPU at the
    /// start of vblank. The bus latches the edge; the CPU consumes it.
    fn nmi_pending(&mut self) -> bool {
        false
    }
}

/// Processor-status flag bit masks.
pub const FLAG_C: u8 = 0x01;
pub const FLAG_Z: u8 = 0x02;
pub const FLAG_I: u8 = 0x04;
pub const FLAG_D: u8 = 0x08;
pub const FLAG_B: u8 = 0x10;
pub const FLAG_U: u8 = 0x20; // unused, reads as 1
pub const FLAG_V: u8 = 0x40;
pub const FLAG_N: u8 = 0x80;

const STACK_BASE: u16 = 0x0100;
pub(super) const NMI_VECTOR: u16 = 0xFFFA;
const RESET_VECTOR: u16 = 0xFFFC;
pub(super) const IRQ_VECTOR: u16 = 0xFFFE;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    /// Processor status (P). Bit 5 (U) reads as 1; bit 4 (B) only exists on the
    /// value pushed by PHP/BRK, not as live state.
    pub p: u8,
}

impl Default for Cpu {
    fn default() -> Self {
        // Post-reset state. Real hardware leaves A/X/Y/SP indeterminate at
        // power-on, but the de-facto convention (nestest, most emulators) is
        // A=X=Y=0, SP=0xFD, P=0x24 (I set, U set), PC from the reset vector.
        // The system resets SP/PC properly via `reset`; these are just a sane
        // default so a Cpu built in isolation (unit tests) is deterministic.
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            sp: 0x00,
            pc: 0,
            p: FLAG_I | FLAG_U,
        }
    }
}

impl Cpu {
    /// Power-on / reset sequence: load PC from the reset vector, set I, leave
    /// A/X/Y as-is (hardware does), decrement SP by 3 (a reset does three dummy
    /// stack "pushes" without writing). We model the net register effect; the
    /// system calls this after building the bus.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        let lo = bus.read(RESET_VECTOR) as u16;
        let hi = bus.read(RESET_VECTOR + 1) as u16;
        self.pc = lo | (hi << 8);
        self.sp = self.sp.wrapping_sub(3);
        self.p |= FLAG_I | FLAG_U;
    }

    fn flag(&self, mask: u8) -> bool {
        self.p & mask != 0
    }
    fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.p |= mask;
        } else {
            self.p &= !mask;
        }
    }
    /// Set N and Z from a result byte (the common ALU flag update).
    fn set_nz(&mut self, v: u8) {
        self.set_flag(FLAG_Z, v == 0);
        self.set_flag(FLAG_N, v & 0x80 != 0);
    }

    fn read(&mut self, bus: &mut impl Bus, addr: u16) -> u8 {
        bus.read(addr)
    }
    fn write(&mut self, bus: &mut impl Bus, addr: u16, val: u8) {
        bus.write(addr, val);
    }

    fn fetch8(&mut self, bus: &mut impl Bus) -> u8 {
        let v = self.read(bus, self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    fn fetch16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch8(bus) as u16;
        let hi = self.fetch8(bus) as u16;
        lo | (hi << 8)
    }

    fn push(&mut self, bus: &mut impl Bus, val: u8) {
        self.write(bus, STACK_BASE | self.sp as u16, val);
        self.sp = self.sp.wrapping_sub(1);
    }
    fn pull(&mut self, bus: &mut impl Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.read(bus, STACK_BASE | self.sp as u16)
    }

    /// Execute one instruction (or service a pending interrupt). Bus callbacks
    /// carry timing: this returns once the instruction's final cycle is done.
    pub fn step(&mut self, bus: &mut impl Bus) {
        // NMI is edge-triggered and takes priority over IRQ. IRQ is masked by I.
        if bus.nmi_pending() {
            self.interrupt(bus, NMI_VECTOR);
            return;
        }
        if bus.irq_pending() && !self.flag(FLAG_I) {
            self.interrupt(bus, IRQ_VECTOR);
            return;
        }
        let opcode = self.fetch8(bus);
        self.execute(bus, opcode);
    }

    /// Common interrupt sequence shared by NMI and IRQ (not BRK — see `brk`).
    /// Two internal cycles (dummy reads), push PCH/PCL/status, set I, jump.
    fn interrupt(&mut self, bus: &mut impl Bus, vector: u16) {
        let _ = self.read(bus, self.pc);
        let _ = self.read(bus, self.pc);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        let status = (self.p | FLAG_U) & !FLAG_B;
        self.push(bus, status);
        self.set_flag(FLAG_I, true);
        let lo = self.read(bus, vector) as u16;
        let hi = self.read(bus, vector + 1) as u16;
        self.pc = lo | (hi << 8);
    }

    /// BRK: like an IRQ through the IRQ/BRK vector, but the opcode fetch already
    /// happened (so only one operand dummy read), PC is pushed pointing *past*
    /// the padding byte, and the pushed status has B set.
    fn brk(&mut self, bus: &mut impl Bus) {
        let _ = self.read(bus, self.pc); // dummy read of the BRK padding byte
        self.pc = self.pc.wrapping_add(1);
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        self.push(bus, self.p | FLAG_B | FLAG_U);
        self.set_flag(FLAG_I, true);
        let lo = self.read(bus, IRQ_VECTOR) as u16;
        let hi = self.read(bus, IRQ_VECTOR + 1) as u16;
        self.pc = lo | (hi << 8);
    }
}

mod exec;
