//! Cartridge: ROM image + memory-bank controller (MBC).
//!
//! Supports the two mappers that cover the bulk of the DMG library:
//!   - No MBC (ROM only, type 0x00): 32 KiB flat ROM, optional 8 KiB RAM.
//!   - MBC1 (types 0x01–0x03): ROM/RAM banking with the 0x20/0x40/0x60 quirk.
//!
//! ponytail: MBC2/MBC3/MBC5 (and RTC) are not implemented — they cover most of
//! the remaining library and are the next mapper work. Loading a cart with an
//! unsupported type is rejected by `Cartridge::new` (LoadError::Invalid), so the
//! core never silently mis-runs a game it can't map. Upgrade path: add a variant
//! to `Mbc` and handle its control writes in `write`.

/// Bank-controller state. Header cartridge type selects the variant.
enum Mbc {
    None,
    Mbc1 {
        /// 5-bit ROM bank number (low), 2-bit upper (RAM or ROM-high).
        bank_lo: u8,
        bank_hi: u8,
        ram_enabled: bool,
        /// false = ROM banking mode (default), true = RAM banking / advanced.
        ram_mode: bool,
    },
}

pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    mbc: Mbc,
    rom_bank_mask: usize,
    has_ram: bool,
}

impl Cartridge {
    /// Parse the header and pick an MBC. Returns None for unsupported types or
    /// malformed images.
    pub fn new(rom: &[u8]) -> Option<Cartridge> {
        if rom.len() < 0x0150 {
            return None; // too small to even hold a header
        }
        let cart_type = rom[0x0147];
        let rom_banks = rom_bank_count(rom[0x0148]);
        let ram_size = ram_byte_size(rom[0x0149]);

        // ROM must be at least the header-declared size.
        if rom.len() < rom_banks * 0x4000 {
            return None;
        }

        let (mbc, has_ram, ram_bytes) = match cart_type {
            0x00 => (Mbc::None, false, 0),
            0x08 | 0x09 => (Mbc::None, true, ram_size.max(0x2000)),
            0x01 => (
                Mbc::Mbc1 {
                    bank_lo: 1,
                    bank_hi: 0,
                    ram_enabled: false,
                    ram_mode: false,
                },
                false,
                0,
            ),
            0x02 | 0x03 => (
                Mbc::Mbc1 {
                    bank_lo: 1,
                    bank_hi: 0,
                    ram_enabled: false,
                    ram_mode: false,
                },
                true,
                ram_size.max(0x2000),
            ),
            _ => return None,
        };

        Some(Cartridge {
            rom: rom.to_vec(),
            ram: vec![0; ram_bytes],
            mbc,
            rom_bank_mask: rom_banks.next_power_of_two().max(2) - 1,
            has_ram,
        })
    }

    /// Read from ROM space (0x0000–0x7FFF).
    pub fn read_rom(&self, addr: u16) -> u8 {
        let a = addr as usize;
        match &self.mbc {
            Mbc::None => self.rom.get(a).copied().unwrap_or(0xFF),
            Mbc::Mbc1 {
                bank_lo,
                bank_hi,
                ram_mode,
                ..
            } => {
                if addr < 0x4000 {
                    // Bank 0 region: in advanced (RAM) mode the high bits can
                    // remap this to bank 0x20/0x40/0x60.
                    let bank = if *ram_mode {
                        ((*bank_hi as usize) << 5) & self.rom_bank_mask
                    } else {
                        0
                    };
                    self.rom_byte(bank, a)
                } else {
                    let mut bank = (*bank_lo as usize) | ((*bank_hi as usize) << 5);
                    bank &= self.rom_bank_mask;
                    self.rom_byte(bank, a - 0x4000)
                }
            }
        }
    }

    fn rom_byte(&self, bank: usize, offset_in_bank: usize) -> u8 {
        let idx = bank * 0x4000 + offset_in_bank;
        self.rom.get(idx).copied().unwrap_or(0xFF)
    }

    /// Write to ROM space = MBC control registers.
    pub fn write_rom(&mut self, addr: u16, val: u8) {
        match &mut self.mbc {
            Mbc::None => {}
            Mbc::Mbc1 {
                bank_lo,
                bank_hi,
                ram_enabled,
                ram_mode,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = val & 0x0F == 0x0A,
                0x2000..=0x3FFF => {
                    // Low 5 bits; a written 0 becomes 1 (bank 0 is unaddressable
                    // through this register — the classic MBC1 quirk).
                    let v = val & 0x1F;
                    *bank_lo = if v == 0 { 1 } else { v };
                }
                0x4000..=0x5FFF => *bank_hi = val & 0x03,
                0x6000..=0x7FFF => *ram_mode = val & 1 != 0,
                _ => {}
            },
        }
    }

    /// Read cartridge RAM (0xA000–0xBFFF).
    pub fn read_ram(&self, addr: u16) -> u8 {
        if !self.has_ram {
            return 0xFF;
        }
        match &self.mbc {
            Mbc::None => self.ram.get((addr - 0xA000) as usize).copied().unwrap_or(0xFF),
            Mbc::Mbc1 {
                ram_enabled,
                bank_hi,
                ram_mode,
                ..
            } => {
                if !ram_enabled {
                    return 0xFF;
                }
                let bank = if *ram_mode { *bank_hi as usize } else { 0 };
                let idx = bank * 0x2000 + (addr - 0xA000) as usize;
                self.ram.get(idx).copied().unwrap_or(0xFF)
            }
        }
    }

    pub fn write_ram(&mut self, addr: u16, val: u8) {
        if !self.has_ram {
            return;
        }
        let idx = match &self.mbc {
            Mbc::None => (addr - 0xA000) as usize,
            Mbc::Mbc1 {
                ram_enabled,
                bank_hi,
                ram_mode,
                ..
            } => {
                if !ram_enabled {
                    return;
                }
                let bank = if *ram_mode { *bank_hi as usize } else { 0 };
                bank * 0x2000 + (addr - 0xA000) as usize
            }
        };
        if idx < self.ram.len() {
            self.ram[idx] = val;
        }
    }

    /// Cartridge RAM contents, for save-state serialization.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }
    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }
}

fn rom_bank_count(code: u8) -> usize {
    // 0x00 => 2 banks (32 KiB); each increment doubles, up to 0x08 = 512 banks.
    match code {
        0x00..=0x08 => 2usize << code,
        _ => 2,
    }
}

fn ram_byte_size(code: u8) -> usize {
    match code {
        0x02 => 0x2000,       // 8 KiB
        0x03 => 0x8000,       // 32 KiB (4 banks)
        0x04 => 0x20000,      // 128 KiB
        0x05 => 0x10000,      // 64 KiB
        _ => 0,
    }
}
