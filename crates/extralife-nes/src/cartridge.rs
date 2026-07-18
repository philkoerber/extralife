//! Cartridge: iNES image parsing + memory-bank mappers.
//!
//! Parses the iNES (.nes) header (also accepting NES 2.0, reading the common
//! subset) and exposes CPU-space ($4020-$FFFF) and PPU-space ($0000-$1FFF)
//! reads/writes plus the nametable mirroring the mapper selects. Supported
//! mappers, chosen to cover the bulk of the library:
//!   - 0  NROM: fixed 16/32 KiB PRG, 8 KiB CHR (ROM or RAM).
//!   - 1  MMC1: serial-loaded control/banking; PRG/CHR banking + mirroring.
//!   - 2  UNROM: 16 KiB switchable PRG bank + fixed last bank; CHR RAM.
//!   - 3  CNROM: fixed PRG, switchable 8 KiB CHR bank.
//!   - 4  MMC3: PRG/CHR banking with the scanline IRQ counter.
//!
//! Clean-room from the NESdev wiki (iNES, "Mapper 0/1/2/3/4" pages). Loading an
//! unsupported mapper is rejected by `Cartridge::new` so the core never
//! silently mis-runs a game it cannot map.
//!
//! ponytail: MMC3 IRQ uses the simple A12-rising-edge scanline counter model
//! (clocked by the PPU at a fixed dot per line), not the exact filtered A12
//! timing. Ceiling: a few raster-precise MMC3 titles will mistime split
//! effects; upgrade path is A12-edge filtering driven from PPU fetches.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    SingleLo,
    SingleHi,
    FourScreen,
}

enum Mapper {
    Nrom,
    Mmc1 {
        shift: u8,
        count: u8,
        control: u8,
        chr0: u8,
        chr1: u8,
        prg: u8,
    },
    Unrom {
        bank: u8,
    },
    Cnrom {
        chr: u8,
    },
    Mmc3 {
        bank_select: u8,
        banks: [u8; 8],
        prg_mode: bool,
        chr_mode: bool,
        irq_latch: u8,
        irq_counter: u8,
        irq_reload: bool,
        irq_enabled: bool,
        irq_pending: bool,
    },
}

pub struct Cartridge {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    mapper: Mapper,
    mirroring: Mirroring,
    prg_banks_16k: usize,
    chr_banks_8k: usize,
}

const PRG_BANK_16K: usize = 0x4000;
const CHR_BANK_8K: usize = 0x2000;

impl Cartridge {
    pub fn new(rom: &[u8]) -> Option<Cartridge> {
        if rom.len() < 16 || &rom[0..4] != b"NES\x1A" {
            return None;
        }
        let prg_16k = rom[4] as usize;
        let chr_8k = rom[5] as usize;
        if prg_16k == 0 {
            return None;
        }
        let flags6 = rom[6];
        let flags7 = rom[7];
        let has_trainer = flags6 & 0x04 != 0;
        let four_screen = flags6 & 0x08 != 0;
        let mirroring = if four_screen {
            Mirroring::FourScreen
        } else if flags6 & 0x01 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
        let mapper_num = (flags6 >> 4) | (flags7 & 0xF0);

        let mut off = 16;
        if has_trainer {
            off += 512;
        }
        let prg_len = prg_16k * PRG_BANK_16K;
        if rom.len() < off + prg_len {
            return None;
        }
        let prg = rom[off..off + prg_len].to_vec();
        off += prg_len;

        let (chr, chr_is_ram) = if chr_8k == 0 {
            // No CHR ROM => 8 KiB CHR RAM.
            (vec![0u8; CHR_BANK_8K], true)
        } else {
            let chr_len = chr_8k * CHR_BANK_8K;
            if rom.len() < off + chr_len {
                return None;
            }
            (rom[off..off + chr_len].to_vec(), false)
        };

        let mapper = match mapper_num {
            0 => Mapper::Nrom,
            1 => Mapper::Mmc1 {
                shift: 0,
                count: 0,
                control: 0x0C, // power-on: PRG mode 3 (fix last bank)
                chr0: 0,
                chr1: 0,
                prg: 0,
            },
            2 => Mapper::Unrom { bank: 0 },
            3 => Mapper::Cnrom { chr: 0 },
            4 => Mapper::Mmc3 {
                bank_select: 0,
                banks: [0; 8],
                prg_mode: false,
                chr_mode: false,
                irq_latch: 0,
                irq_counter: 0,
                irq_reload: false,
                irq_enabled: false,
                irq_pending: false,
            },
            _ => return None,
        };

        Some(Cartridge {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0u8; 0x2000],
            mapper,
            mirroring,
            prg_banks_16k: prg_16k,
            chr_banks_8k: chr_8k.max(1),
        })
    }

    pub fn mirroring(&self) -> Mirroring {
        match &self.mapper {
            Mapper::Mmc1 { control, .. } => match control & 0x03 {
                0 => Mirroring::SingleLo,
                1 => Mirroring::SingleHi,
                2 => Mirroring::Vertical,
                _ => Mirroring::Horizontal,
            },
            _ => self.mirroring,
        }
    }

    // --- CPU space ($4020..=$FFFF) ---------------------------------------

    pub fn cpu_read(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => self.prg_ram[(addr - 0x6000) as usize],
            0x8000..=0xFFFF => {
                let idx = self.prg_offset(addr);
                self.prg[idx % self.prg.len()]
            }
            _ => 0,
        }
    }

    pub fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x7FFF => self.prg_ram[(addr - 0x6000) as usize] = val,
            0x8000..=0xFFFF => self.mapper_write(addr, val),
            _ => {}
        }
    }

    /// Map a CPU $8000..=$FFFF address to a PRG-ROM byte offset per the mapper.
    fn prg_offset(&self, addr: u16) -> usize {
        let a = addr as usize;
        let last_16k = (self.prg_banks_16k - 1) * PRG_BANK_16K;
        match &self.mapper {
            Mapper::Nrom => {
                if self.prg_banks_16k == 1 {
                    (a - 0x8000) & 0x3FFF
                } else {
                    a - 0x8000
                }
            }
            Mapper::Unrom { bank } => {
                if a < 0xC000 {
                    (*bank as usize % self.prg_banks_16k) * PRG_BANK_16K + (a - 0x8000)
                } else {
                    last_16k + (a - 0xC000)
                }
            }
            Mapper::Cnrom { .. } => {
                if self.prg_banks_16k == 1 {
                    (a - 0x8000) & 0x3FFF
                } else {
                    a - 0x8000
                }
            }
            Mapper::Mmc1 { control, prg, .. } => {
                let mode = (control >> 2) & 0x03;
                let bank = (*prg & 0x0F) as usize;
                match mode {
                    0 | 1 => {
                        // 32 KiB switch (ignore low bit).
                        (bank & !1) * PRG_BANK_16K + (a - 0x8000)
                    }
                    2 => {
                        // fix first bank at $8000, switch $C000.
                        if a < 0xC000 {
                            a - 0x8000
                        } else {
                            bank * PRG_BANK_16K + (a - 0xC000)
                        }
                    }
                    _ => {
                        // fix last bank at $C000, switch $8000.
                        if a < 0xC000 {
                            bank * PRG_BANK_16K + (a - 0x8000)
                        } else {
                            last_16k + (a - 0xC000)
                        }
                    }
                }
            }
            Mapper::Mmc3 {
                banks, prg_mode, ..
            } => {
                // PRG in 8 KiB windows: $8000,$A000,$C000,$E000.
                let bank_count_8k = self.prg_banks_16k * 2;
                let r6 = banks[6] as usize % bank_count_8k;
                let r7 = banks[7] as usize % bank_count_8k;
                let second_last = bank_count_8k - 2;
                let last = bank_count_8k - 1;
                let window = (a - 0x8000) / 0x2000;
                let off = (a - 0x8000) % 0x2000;
                let bank = match (window, prg_mode) {
                    (0, false) => r6,
                    (0, true) => second_last,
                    (1, _) => r7,
                    (2, false) => second_last,
                    (2, true) => r6,
                    _ => last,
                };
                bank * 0x2000 + off
            }
        }
    }

    fn mapper_write(&mut self, addr: u16, val: u8) {
        match &mut self.mapper {
            Mapper::Nrom => {}
            Mapper::Unrom { bank } => *bank = val & 0x0F,
            Mapper::Cnrom { chr } => *chr = val & 0x03,
            Mapper::Mmc1 {
                shift,
                count,
                control,
                chr0,
                chr1,
                prg,
            } => {
                if val & 0x80 != 0 {
                    // Reset: clear shift, set PRG mode 3.
                    *shift = 0;
                    *count = 0;
                    *control |= 0x0C;
                    return;
                }
                *shift |= (val & 1) << *count;
                *count += 1;
                if *count == 5 {
                    let reg = (addr >> 13) & 0x03; // $8000/$A000/$C000/$E000
                    match reg {
                        0 => *control = *shift & 0x1F,
                        1 => *chr0 = *shift & 0x1F,
                        2 => *chr1 = *shift & 0x1F,
                        _ => *prg = *shift & 0x1F,
                    }
                    *shift = 0;
                    *count = 0;
                }
            }
            Mapper::Mmc3 {
                bank_select,
                banks,
                prg_mode,
                chr_mode,
                irq_latch,
                irq_reload,
                irq_enabled,
                irq_pending,
                irq_counter: _,
            } => {
                let even = addr & 1 == 0;
                match (addr & 0xE000, even) {
                    (0x8000, true) => {
                        *bank_select = val & 0x07;
                        *prg_mode = val & 0x40 != 0;
                        *chr_mode = val & 0x80 != 0;
                    }
                    (0x8000, false) => {
                        let idx = (*bank_select & 0x07) as usize;
                        banks[idx] = val;
                    }
                    (0xA000, true) => {
                        // Mirroring (ignored when four-screen).
                        self.mirroring = if val & 1 != 0 {
                            Mirroring::Horizontal
                        } else {
                            Mirroring::Vertical
                        };
                    }
                    (0xA000, false) => {} // PRG-RAM protect (ignored)
                    (0xC000, true) => *irq_latch = val,
                    (0xC000, false) => *irq_reload = true,
                    (0xE000, true) => {
                        *irq_enabled = false;
                        *irq_pending = false;
                    }
                    (0xE000, false) => *irq_enabled = true,
                    _ => {}
                }
            }
        }
    }

    // --- PPU space ($0000..=$1FFF pattern tables) ------------------------

    pub fn ppu_read(&self, addr: u16) -> u8 {
        let idx = self.chr_offset(addr);
        self.chr[idx % self.chr.len()]
    }

    pub fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let idx = self.chr_offset(addr);
            let len = self.chr.len();
            self.chr[idx % len] = val;
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let a = (addr & 0x1FFF) as usize;
        match &self.mapper {
            Mapper::Cnrom { chr } => (*chr as usize % self.chr_banks_8k) * CHR_BANK_8K + a,
            Mapper::Mmc1 { control, chr0, chr1, .. } => {
                if control & 0x10 != 0 {
                    // 4 KiB banks.
                    if a < 0x1000 {
                        (*chr0 as usize) * 0x1000 + a
                    } else {
                        (*chr1 as usize) * 0x1000 + (a - 0x1000)
                    }
                } else {
                    // 8 KiB bank (low bit of chr0 ignored).
                    ((*chr0 & !1) as usize) * 0x1000 + a
                }
            }
            Mapper::Mmc3 { banks, chr_mode, .. } => {
                // Two 2 KiB + four 1 KiB windows; chr_mode swaps the halves.
                let (r0, r1) = if *chr_mode { (4, 0) } else { (0, 4) };
                let window_1k = a / 0x400;
                let off = a % 0x400;
                let bank = match window_1k {
                    0 => (banks[r0] & 0xFE) as usize,
                    1 => (banks[r0] | 0x01) as usize,
                    2 => (banks[r0 + 1] & 0xFE) as usize,
                    3 => (banks[r0 + 1] | 0x01) as usize,
                    4 => banks[r1 + 2] as usize,
                    5 => banks[r1 + 3] as usize,
                    6 => banks[r1 + 4] as usize,
                    _ => banks[r1 + 5] as usize,
                };
                bank * 0x400 + off
            }
            _ => a, // NROM/UNROM: flat 8 KiB CHR
        }
    }

    // --- MMC3 scanline IRQ ------------------------------------------------

    /// Clock the MMC3 scanline counter once (called by the PPU per rendered
    /// scanline on the A12-rising edge). Sets the IRQ line when it hits zero.
    pub fn mmc3_clock_scanline(&mut self) {
        if let Mapper::Mmc3 {
            irq_counter,
            irq_latch,
            irq_reload,
            irq_enabled,
            irq_pending,
            ..
        } = &mut self.mapper
        {
            if *irq_counter == 0 || *irq_reload {
                *irq_counter = *irq_latch;
                *irq_reload = false;
            } else {
                *irq_counter -= 1;
            }
            if *irq_counter == 0 && *irq_enabled {
                *irq_pending = true;
            }
        }
    }

    pub fn irq_pending(&self) -> bool {
        matches!(self.mapper, Mapper::Mmc3 { irq_pending: true, .. })
    }
}
