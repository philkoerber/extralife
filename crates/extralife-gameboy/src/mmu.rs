//! The system bus / MMU: the DMG memory map, wired to cartridge, PPU, timer,
//! joypad, and interrupt registers. Implements the CPU's `Bus` so every memory
//! access ticks the rest of the hardware by exactly one M-cycle (4 T-cycles),
//! keeping PPU/timer in lockstep with instruction timing.
//!
//! Memory map (Pandocs):
//!   0000-7FFF ROM (cartridge)      A000-BFFF cartridge RAM
//!   8000-9FFF VRAM                 C000-DFFF WRAM
//!   E000-FDFF echo of WRAM         FE00-FE9F OAM
//!   FEA0-FEFF unusable             FF00-FF7F IO registers
//!   FF80-FFFE HRAM                 FFFF      IE

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::cpu::Bus;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::timer::Timer;

pub struct Mmu {
    pub cart: Option<Cartridge>,
    pub ppu: Ppu,
    pub apu: Apu,
    pub timer: Timer,
    pub joypad: Joypad,
    wram: [u8; 0x2000],
    hram: [u8; 0x7F],
    /// Interrupt Flag (0xFF0F) and Interrupt Enable (0xFFFF), low 5 bits used.
    iflag: u8,
    ie: u8,
    /// Serial data/control stub (0xFF01/0xFF02). Kept so Blargg's serial output
    /// (used by its test ROMs to print results) can be captured for CI checks.
    serial: u8,
    pub serial_out: Vec<u8>,
    /// OAM DMA source high byte + remaining bytes; DMA copies one byte per
    /// M-cycle while active.
    dma_active: bool,
    dma_src: u16,
    dma_index: u16,
}

const IRQ_VBLANK: u8 = 0x01;
const IRQ_STAT: u8 = 0x02;
const IRQ_TIMER: u8 = 0x04;
const IRQ_JOYPAD: u8 = 0x10;

impl Default for Mmu {
    fn default() -> Self {
        Mmu {
            cart: None,
            ppu: Ppu::default(),
            apu: Apu::default(),
            timer: Timer::default(),
            joypad: Joypad::default(),
            wram: [0; 0x2000],
            hram: [0; 0x7F],
            iflag: 0xE1,
            ie: 0x00,
            serial: 0,
            serial_out: Vec::new(),
            dma_active: false,
            dma_src: 0,
            dma_index: 0,
        }
    }
}

impl Mmu {
    pub fn new(cart: Cartridge) -> Self {
        Mmu {
            cart: Some(cart),
            ..Mmu::default()
        }
    }

    /// One M-cycle of hardware: tick timer + PPU 4 T-cycles, advance OAM DMA,
    /// and latch any device interrupts into IF.
    fn advance_mcycle(&mut self) {
        for _ in 0..4 {
            self.timer.tick();
            self.ppu.tick();
            self.apu.tick();
        }
        if self.timer.take_irq() {
            self.iflag |= IRQ_TIMER;
        }
        if self.ppu.vblank_irq {
            self.ppu.vblank_irq = false;
            self.iflag |= IRQ_VBLANK;
        }
        if self.ppu.stat_irq {
            self.ppu.stat_irq = false;
            self.iflag |= IRQ_STAT;
        }
        if self.joypad.take_irq() {
            self.iflag |= IRQ_JOYPAD;
        }
        if self.dma_active {
            self.step_dma();
        }
    }

    fn step_dma(&mut self) {
        // One byte per M-cycle. ponytail: real DMA blocks CPU access to all but
        // HRAM during transfer; we don't enforce the block (no test needs it and
        // well-behaved ROMs run their copy loop from HRAM anyway).
        let byte = self.read_raw(self.dma_src + self.dma_index);
        self.ppu.oam[self.dma_index as usize] = byte;
        self.dma_index += 1;
        if self.dma_index >= 0xA0 {
            self.dma_active = false;
        }
    }

    /// Direct read with no timing side effects (used by DMA and debugging).
    pub(crate) fn read_raw(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.as_ref().map_or(0xFF, |c| c.read_rom(addr)),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.cart.as_ref().map_or(0xFF, |c| c.read_ram(addr)),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize],
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00 => self.joypad.read(),
            0xFF01 => self.serial,
            0xFF02 => 0x7E,
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.iflag | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF40..=0xFF4B => self.ppu.read(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    fn write_raw(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF => {
                if let Some(c) = self.cart.as_mut() {
                    c.write_rom(addr, val);
                }
            }
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize] = val,
            0xA000..=0xBFFF => {
                if let Some(c) = self.cart.as_mut() {
                    c.write_ram(addr, val);
                }
            }
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = val,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize] = val,
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = val,
            0xFEA0..=0xFEFF => {}
            0xFF00 => self.joypad.write(val),
            0xFF01 => self.serial = val,
            // Bit 7 = transfer start. On DMG we have no link cable, so a
            // "transfer" just echoes the serial byte to our capture buffer
            // (Blargg's test ROMs print pass/fail this way).
            0xFF02 if val & 0x80 != 0 => self.serial_out.push(self.serial),
            0xFF02 => {}
            0xFF04..=0xFF07 => self.timer.write(addr, val),
            0xFF0F => self.iflag = val & 0x1F,
            0xFF10..=0xFF3F => self.apu.write(addr, val),
            0xFF46 => {
                // OAM DMA start: source = val * 0x100.
                self.dma_src = (val as u16) << 8;
                self.dma_index = 0;
                self.dma_active = true;
            }
            0xFF40..=0xFF4B => self.ppu.write(addr, val),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = val,
            0xFFFF => self.ie = val & 0x1F,
            _ => {}
        }
    }
}

impl Mmu {
    pub(crate) fn serialize(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.wram);
        out.extend_from_slice(&self.hram);
        out.push(self.iflag);
        out.push(self.ie);
        self.ppu.serialize(out);
        self.timer.serialize(out);
        self.apu.serialize(out);
        let empty: &[u8] = &[];
        let ram = self.cart.as_ref().map_or(empty, |c| c.ram());
        out.extend_from_slice(&(ram.len() as u32).to_le_bytes());
        out.extend_from_slice(ram);
    }

    pub(crate) fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 0x2000 + 0x7F + 2 {
            return false;
        }
        self.wram.copy_from_slice(&s[*p..*p + 0x2000]);
        *p += 0x2000;
        self.hram.copy_from_slice(&s[*p..*p + 0x7F]);
        *p += 0x7F;
        self.iflag = s[*p];
        self.ie = s[*p + 1];
        *p += 2;
        if !self.ppu.deserialize(s, p) || !self.timer.deserialize(s, p) {
            return false;
        }
        if !self.apu.deserialize(s, p) {
            return false;
        }
        if s.len() < *p + 4 {
            return false;
        }
        let len = u32::from_le_bytes([s[*p], s[*p + 1], s[*p + 2], s[*p + 3]]) as usize;
        *p += 4;
        if s.len() < *p + len {
            return false;
        }
        if let Some(c) = self.cart.as_mut() {
            let ram = c.ram_mut();
            if ram.len() == len {
                ram.copy_from_slice(&s[*p..*p + len]);
            }
        }
        *p += len;
        true
    }
}

impl Bus for Mmu {
    fn read(&mut self, addr: u16) -> u8 {
        self.advance_mcycle();
        self.read_raw(addr)
    }

    fn write(&mut self, addr: u16, val: u8) {
        self.advance_mcycle();
        self.write_raw(addr, val);
    }

    fn tick(&mut self) {
        self.advance_mcycle();
    }

    fn pending_interrupts(&self) -> u8 {
        self.ie & self.iflag & 0x1F
    }

    fn ack_interrupt(&mut self, bit: u8) {
        self.iflag &= !(1 << bit);
    }
}
