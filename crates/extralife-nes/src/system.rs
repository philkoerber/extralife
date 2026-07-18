//! System bus: the CPU's memory map, PPU/APU clocking, DMA, and interrupt lines.
//!
//! CPU address space (NESdev "CPU memory map"):
//!   $0000-$07FF  2 KiB internal RAM, mirrored through $1FFF
//!   $2000-$2007  PPU registers, mirrored every 8 bytes through $3FFF
//!   $4000-$4013  APU registers
//!   $4014        OAM DMA
//!   $4015        APU status
//!   $4016/$4017  controller ports (+ APU frame counter on $4017 write)
//!   $4020-$FFFF  cartridge (mapper)
//!
//! The bus is the CPU's `cpu::Bus`: every `read`/`write` is one CPU cycle, and
//! it advances the PPU three dots and the APU one cycle inside each, so all
//! subsystems stay in lockstep with instruction timing.

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::cpu::Bus as CpuBus;
use crate::ppu::Ppu;
use extralife_core::Button;

pub struct System {
    ram: [u8; 0x800],
    pub ppu: Ppu,
    pub apu: Apu,
    pub cart: Cartridge,

    /// Controller shift registers + latch (strobe) state.
    controller_shift: [u8; 2],
    strobe: bool,
    /// Button state per controller, bit order A,B,Select,Start,Up,Down,Left,Right.
    buttons: [u8; 2],

    /// Total CPU cycles executed (for cycle-accurate logging / timing).
    pub cycles: u64,
    /// Latched NMI edge from the PPU, consumed by the CPU.
    nmi_edge: bool,
    prev_nmi: bool,
}

impl System {
    pub fn new(cart: Cartridge) -> System {
        System {
            ram: [0; 0x800],
            ppu: Ppu::default(),
            apu: Apu::default(),
            cart,
            controller_shift: [0; 2],
            strobe: false,
            buttons: [0; 2],
            cycles: 0,
            nmi_edge: false,
            prev_nmi: false,
        }
    }

    pub fn set_button(&mut self, button: Button, pressed: bool) {
        // Standard NES pad bit order (as read serially): A,B,Select,Start,
        // Up,Down,Left,Right. Player 1 only for now.
        let bit = match button {
            Button::A => 0,
            Button::B => 1,
            Button::Select => 2,
            Button::Start => 3,
            Button::Up => 4,
            Button::Down => 5,
            Button::Left => 6,
            Button::Right => 7,
            _ => return,
        };
        if pressed {
            self.buttons[0] |= 1 << bit;
        } else {
            self.buttons[0] &= !(1 << bit);
        }
    }

    /// One PPU-side clock triple + APU clock, done on every CPU cycle.
    fn clock(&mut self) {
        self.cycles += 1;
        for _ in 0..3 {
            self.ppu.tick(&mut self.cart);
            if self.ppu.nmi_line && !self.prev_nmi {
                self.nmi_edge = true;
            }
            self.prev_nmi = self.ppu.nmi_line;
        }
        self.apu.tick(&self.cart);
    }

    fn read_controller(&mut self, port: usize) -> u8 {
        if self.strobe {
            // While strobing, always return button A's current state.
            self.controller_shift[port] = self.buttons[port];
        }
        let bit = self.controller_shift[port] & 1;
        self.controller_shift[port] >>= 1;
        // Open-bus upper bits read as 0x40 on real hardware; report just the bit.
        0x40 | bit
    }

    /// Raw read with no side effects (for save-state / debugging).
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x4020..=0xFFFF => self.cart.cpu_read(addr),
            _ => 0,
        }
    }

    fn bus_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x2000..=0x3FFF => self.ppu.read_reg(&self.cart, addr & 7),
            0x4015 => self.apu.read_status(),
            0x4016 => self.read_controller(0),
            0x4017 => self.read_controller(1),
            0x4020..=0xFFFF => self.cart.cpu_read(addr),
            _ => 0,
        }
    }

    fn bus_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x2000..=0x3FFF => self.ppu.write_reg(&mut self.cart, addr & 7, val),
            0x4014 => self.oam_dma(val),
            0x4016 => {
                self.strobe = val & 1 != 0;
                if self.strobe {
                    self.controller_shift[0] = self.buttons[0];
                    self.controller_shift[1] = self.buttons[1];
                }
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => self.apu.write_reg(addr, val),
            0x4020..=0xFFFF => self.cart.cpu_write(addr, val),
            _ => {}
        }
    }

    /// OAM DMA ($4014): copy 256 bytes from $XX00 into OAM, costing 513/514 CPU
    /// cycles (one idle + one per byte, +1 alignment cycle on odd cycles).
    fn oam_dma(&mut self, page: u8) {
        // Alignment: DMA waits for a read cycle; add one dummy cycle if odd.
        self.clock(); // the DMA "get" halt cycle
        if self.cycles % 2 == 1 {
            self.clock();
        }
        let base = (page as u16) << 8;
        for i in 0..256u16 {
            let byte = self.bus_read(base + i);
            self.clock();
            // Writes go through OAMDATA at the current OAM address.
            let oam_addr = self.ppu.oam_addr_get();
            self.ppu.oam[oam_addr as usize] = byte;
            self.ppu.oam_addr_inc();
            self.clock();
        }
    }
}

impl CpuBus for System {
    fn read(&mut self, addr: u16) -> u8 {
        self.clock();
        self.bus_read(addr)
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.clock();
        self.bus_write(addr, val);
    }
    fn irq_pending(&self) -> bool {
        self.apu.irq_pending() || self.cart.irq_pending()
    }
    fn nmi_pending(&mut self) -> bool {
        let e = self.nmi_edge;
        self.nmi_edge = false;
        e
    }
}
