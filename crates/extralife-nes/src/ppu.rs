//! Picture Processing Unit (2C02) — background + sprites to a 256x240 RGBA
//! framebuffer.
//!
//! Timing model: a per-dot scanline state machine, 341 dots x 262 scanlines
//! (NTSC). Lines 0-239 are visible, 240 is post-render, 241-260 vblank (NMI at
//! 241 dot 1), 261 the pre-render line. The CPU is clocked at one cycle per 3
//! PPU dots by the system bus, so `tick` is called three times per CPU cycle.
//!
//! Rendering is done per-visible-scanline in one shot at the *end* of the line
//! (accurate enough for the vast majority of games and the acid-style tests;
//! see the ponytail note). The internal v/t/x/w loopy registers track scroll
//! exactly per the NESdev "PPU scrolling" page so split-scroll and status-timing
//! behave, even though pixel emission is batched.
//!
//! Clean-room from the NESdev wiki (PPU registers, rendering, scrolling, sprite
//! evaluation, OAM). No emulator source was translated.
//!
//! ponytail: no mid-scanline background fetch pipeline / pixel FIFO, no
//! sprite-overflow hardware bug emulation, and sprite-0 hit is computed from the
//! batched line render (set when the first opaque sprite-0 pixel overlaps an
//! opaque BG pixel) rather than dot-exact. Ceiling: raster-effect demos and a
//! few sprite-0-timing-sensitive games mis-split; upgrade path is a dot-accurate
//! BG/sprite fetch pipeline.

use crate::cartridge::{Cartridge, Mirroring};

pub const W: usize = 256;
pub const H: usize = 240;
const DOTS: u16 = 341;
const LINES: u16 = 262;

/// The 2C02 master palette as RGB (64 entries), a commonly used NTSC decode.
/// Values from the NESdev "PPU palettes" reference table.
#[rustfmt::skip]
const PALETTE: [[u8; 3]; 64] = [
    [ 84, 84, 84],[  0, 30,116],[  8, 16,144],[ 48,  0,136],[ 68,  0,100],[ 92,  0, 48],[ 84,  4,  0],[ 60, 24,  0],
    [ 32, 42,  0],[  8, 58,  0],[  0, 64,  0],[  0, 60,  0],[  0, 50, 60],[  0,  0,  0],[  0,  0,  0],[  0,  0,  0],
    [152,150,152],[  8, 76,196],[ 48, 50,236],[ 92, 30,228],[136, 20,176],[160, 20,100],[152, 34, 32],[120, 60,  0],
    [ 84, 90,  0],[ 40,114,  0],[  8,124,  0],[  0,118, 40],[  0,102,120],[  0,  0,  0],[  0,  0,  0],[  0,  0,  0],
    [236,238,236],[ 76,154,236],[120,124,236],[176, 98,236],[228, 84,236],[236, 88,180],[236,106,100],[212,136, 32],
    [160,170,  0],[116,196,  0],[ 76,208, 32],[ 56,204,108],[ 56,180,204],[ 60, 60, 60],[  0,  0,  0],[  0,  0,  0],
    [236,238,236],[168,204,236],[188,188,236],[212,178,236],[236,174,236],[236,174,212],[236,180,176],[228,196,144],
    [204,210,120],[180,222,120],[168,226,144],[152,226,180],[160,214,228],[160,162,160],[  0,  0,  0],[  0,  0,  0],
];

pub struct Ppu {
    // Registers.
    ctrl: u8,   // $2000
    mask: u8,   // $2001
    status: u8, // $2002
    oam_addr: u8,

    // Loopy scroll registers.
    v: u16, // current VRAM address (15 bits)
    t: u16, // temporary VRAM address / topleft
    x: u8,  // fine X scroll (3 bits)
    w: bool, // write toggle
    read_buffer: u8, // buffered $2007 read

    vram: [u8; 0x800], // 2 KiB nametable RAM
    palette: [u8; 0x20],
    pub oam: [u8; 0x100],

    dot: u16,
    scanline: u16,
    odd_frame: bool,

    pub nmi_line: bool,
    pub frame_ready: bool,
    framebuffer: Vec<u8>,

    /// Set when the A12 line rises during rendering, so the mapper (MMC3) can
    /// clock its scanline IRQ. We approximate by pulsing once per visible line.
    pub a12_rise: bool,
}

impl Default for Ppu {
    fn default() -> Self {
        Ppu {
            ctrl: 0,
            mask: 0,
            status: 0,
            oam_addr: 0,
            v: 0,
            t: 0,
            x: 0,
            w: false,
            read_buffer: 0,
            vram: [0; 0x800],
            palette: [0; 0x20],
            oam: [0; 0x100],
            dot: 0,
            scanline: 0,
            odd_frame: false,
            nmi_line: false,
            frame_ready: false,
            framebuffer: vec![0; W * H * 4],
            a12_rise: false,
        }
    }
}

impl Ppu {
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    pub fn oam_addr_get(&self) -> u8 {
        self.oam_addr
    }
    pub fn oam_addr_inc(&mut self) {
        self.oam_addr = self.oam_addr.wrapping_add(1);
    }

    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    // --- CPU-facing register access ($2000-$2007) ------------------------

    pub fn read_reg(&mut self, cart: &Cartridge, reg: u16) -> u8 {
        match reg & 7 {
            2 => {
                // $2002 PPUSTATUS: reading clears vblank + the write toggle.
                let v = self.status;
                self.status &= 0x7F;
                self.w = false;
                v
            }
            4 => self.oam[self.oam_addr as usize],
            7 => {
                // $2007 PPUDATA: buffered read except for palette.
                let addr = self.v & 0x3FFF;
                let result = if addr >= 0x3F00 {
                    // Palette reads are immediate; the buffer gets the nametable
                    // byte "under" the palette.
                    self.read_buffer = self.vram_read(cart, addr - 0x1000);
                    self.palette_read(addr)
                } else {
                    let buffered = self.read_buffer;
                    self.read_buffer = self.vram_read(cart, addr);
                    buffered
                };
                self.v = self.v.wrapping_add(self.vram_increment());
                result
            }
            _ => 0, // write-only registers read as (approximate) open bus / 0
        }
    }

    pub fn write_reg(&mut self, cart: &mut Cartridge, reg: u16, val: u8) {
        match reg & 7 {
            0 => {
                self.ctrl = val;
                // t: ....BA.. ........ = d: ......BA (nametable select)
                self.t = (self.t & 0xF3FF) | ((val as u16 & 0x03) << 10);
            }
            1 => self.mask = val,
            3 => self.oam_addr = val,
            4 => {
                self.oam[self.oam_addr as usize] = val;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => {
                if !self.w {
                    // First write: fine X + coarse X.
                    self.x = val & 0x07;
                    self.t = (self.t & 0xFFE0) | (val as u16 >> 3);
                    self.w = true;
                } else {
                    // Second write: fine Y + coarse Y.
                    self.t = (self.t & 0x8FFF) | ((val as u16 & 0x07) << 12);
                    self.t = (self.t & 0xFC1F) | ((val as u16 & 0xF8) << 2);
                    self.w = false;
                }
            }
            6 => {
                if !self.w {
                    self.t = (self.t & 0x00FF) | ((val as u16 & 0x3F) << 8);
                    self.w = true;
                } else {
                    self.t = (self.t & 0xFF00) | val as u16;
                    self.v = self.t;
                    self.w = false;
                }
            }
            7 => {
                let addr = self.v & 0x3FFF;
                self.vram_write(cart, addr, val);
                self.v = self.v.wrapping_add(self.vram_increment());
            }
            _ => {}
        }
    }

    fn vram_increment(&self) -> u16 {
        if self.ctrl & 0x04 != 0 {
            32
        } else {
            1
        }
    }

    // --- PPU memory bus --------------------------------------------------

    fn vram_read(&self, cart: &Cartridge, addr: u16) -> u8 {
        let a = addr & 0x3FFF;
        match a {
            0x0000..=0x1FFF => cart.ppu_read(a),
            0x2000..=0x3EFF => self.vram[self.nt_index(cart, a)],
            _ => self.palette_read(a),
        }
    }

    fn vram_write(&mut self, cart: &mut Cartridge, addr: u16, val: u8) {
        let a = addr & 0x3FFF;
        match a {
            0x0000..=0x1FFF => cart.ppu_write(a, val),
            0x2000..=0x3EFF => {
                let idx = self.nt_index(cart, a);
                self.vram[idx] = val;
            }
            _ => self.palette_write(a, val),
        }
    }

    /// Map a $2000-$3EFF nametable address to a 2 KiB VRAM index per mirroring.
    fn nt_index(&self, cart: &Cartridge, addr: u16) -> usize {
        let a = (addr - 0x2000) & 0x0FFF;
        let table = a / 0x400;
        let offset = (a % 0x400) as usize;
        let bank = match cart.mirroring() {
            Mirroring::Horizontal => [0, 0, 1, 1][table as usize],
            Mirroring::Vertical => [0, 1, 0, 1][table as usize],
            Mirroring::SingleLo => 0,
            Mirroring::SingleHi => 1,
            Mirroring::FourScreen => table as usize & 1, // ponytail: 4-screen extra RAM not modeled; folds to 2 banks
        };
        bank * 0x400 + offset
    }

    fn palette_read(&self, addr: u16) -> u8 {
        self.palette[palette_index(addr)]
    }
    fn palette_write(&mut self, addr: u16, val: u8) {
        self.palette[palette_index(addr)] = val;
    }
}

/// Palette mirroring: $3F10/$14/$18/$1C mirror $3F00/$04/$08/$0C.
fn palette_index(addr: u16) -> usize {
    let mut i = (addr & 0x1F) as usize;
    if i >= 0x10 && i % 4 == 0 {
        i -= 0x10;
    }
    i
}

mod render;
