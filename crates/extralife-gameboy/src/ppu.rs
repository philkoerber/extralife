//! Picture Processing Unit (DMG) per Pandocs.
//!
//! Timing is a per-T-cycle scanline state machine: 456 dots per line, 154 lines
//! (144 visible + 10 VBlank). Modes cycle 2 (OAM scan) -> 3 (draw) -> 0 (HBlank)
//! on visible lines, then mode 1 (VBlank). We render each visible scanline in
//! one shot when entering HBlank — accurate enough for dmg-acid2 and the vast
//! majority of games, and much simpler than a dot-accurate pixel FIFO.
//!
//! ponytail: mid-scanline register changes (mid-line SCX/palette writes, the
//! FIFO's fine timing, the OAM/VRAM access-blocking quirks) are not modeled —
//! that's the pixel-FIFO upgrade needed for the handful of raster-effect demos
//! and the Mooneye PPU-timing tests. dmg-acid2 and normal games pass without it.

pub const W: usize = 160;
pub const H: usize = 144;
const DOTS_PER_LINE: u32 = 456;
const VISIBLE_LINES: u8 = 144;
const TOTAL_LINES: u8 = 154;

/// The DMG's 4 grays as RGBA (lightest -> darkest), a classic green-tinted set.
const SHADES: [[u8; 4]; 4] = [
    [0xE0, 0xF8, 0xD0, 0xFF],
    [0x88, 0xC0, 0x70, 0xFF],
    [0x34, 0x68, 0x56, 0xFF],
    [0x08, 0x18, 0x20, 0xFF],
];

pub struct Ppu {
    pub vram: [u8; 0x2000],
    pub oam: [u8; 0xA0],

    // LCD registers.
    lcdc: u8,
    stat: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    wy: u8,
    wx: u8,

    /// Dot counter within the current scanline (0..456).
    dot: u32,
    /// Window internal line counter (only advances on lines where WY<=LY and
    /// the window is enabled and visible).
    window_line: u8,
    /// BG color index (0..3) for each pixel of the current scanline; sprites
    /// use it for the behind-BG priority test. Scratch, not serialized.
    bg_index: [u8; W],

    framebuffer: Vec<u8>,

    pub vblank_irq: bool,
    pub stat_irq: bool,
    /// Set true for one frame when VBlank starts, so the host can present.
    pub frame_ready: bool,
}

impl Default for Ppu {
    fn default() -> Self {
        Ppu {
            vram: [0; 0x2000],
            oam: [0; 0xA0],
            lcdc: 0x91,
            stat: 0x85,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            dot: 0,
            window_line: 0,
            bg_index: [0; W],
            framebuffer: vec![0; W * H * 4],
            vblank_irq: false,
            stat_irq: false,
            frame_ready: false,
        }
    }
}

impl Ppu {
    fn lcd_on(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    /// Advance one T-cycle.
    pub fn tick(&mut self) {
        if !self.lcd_on() {
            // LCD off: LY held at 0, mode 0, counter reset (Pandocs).
            self.ly = 0;
            self.dot = 0;
            self.window_line = 0;
            self.stat &= !0x03;
            return;
        }

        self.dot += 1;
        if self.dot >= DOTS_PER_LINE {
            self.dot = 0;
            // End of a scanline.
            if self.ly < VISIBLE_LINES {
                self.render_scanline();
            }
            self.ly += 1;

            if self.ly == VISIBLE_LINES {
                // Enter VBlank.
                self.vblank_irq = true;
                self.frame_ready = true;
                self.window_line = 0;
            }
            if self.ly >= TOTAL_LINES {
                self.ly = 0;
            }
            self.check_lyc();
        }

        self.update_mode();
    }

    fn update_mode(&mut self) {
        let prev_mode = self.stat & 0x03;
        let mode = if self.ly >= VISIBLE_LINES {
            1 // VBlank
        } else if self.dot < 80 {
            2 // OAM scan
        } else if self.dot < 80 + 172 {
            3 // Drawing (fixed length; see ponytail note about FIFO timing)
        } else {
            0 // HBlank
        };
        self.stat = (self.stat & !0x03) | mode;

        // STAT interrupt on mode-transition into an enabled source, or LY=LYC.
        if mode != prev_mode {
            let src = match mode {
                0 => self.stat & 0x08,
                1 => self.stat & 0x10,
                2 => self.stat & 0x20,
                _ => 0,
            };
            if src != 0 {
                self.stat_irq = true;
            }
        }
    }

    fn check_lyc(&mut self) {
        if self.ly == self.lyc {
            self.stat |= 0x04;
            if self.stat & 0x40 != 0 {
                self.stat_irq = true;
            }
        } else {
            self.stat &= !0x04;
        }
    }

    fn render_scanline(&mut self) {
        let y = self.ly;
        // Background / window.
        if self.lcdc & 0x01 != 0 {
            self.render_bg_window(y);
        } else {
            // BG disabled: line is blank (shade 0).
            for x in 0..W {
                self.put_pixel(x, y as usize, SHADES[0]);
            }
        }
        // Sprites.
        if self.lcdc & 0x02 != 0 {
            self.render_sprites(y);
        }
    }

    fn render_bg_window(&mut self, y: u8) {
        let win_enabled = self.lcdc & 0x20 != 0 && self.wy <= y;
        let win_x_start = self.wx.wrapping_sub(7);
        let bg_tilemap = if self.lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
        let win_tilemap = if self.lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 };
        let signed_tiles = self.lcdc & 0x10 == 0;

        let mut window_drawn = false;
        for x in 0..W as u8 {
            let in_window = win_enabled && x >= win_x_start;
            let (map_base, tile_x, tile_y) = if in_window {
                window_drawn = true;
                let wx_off = x.wrapping_sub(win_x_start);
                (win_tilemap, wx_off as u16, self.window_line as u16)
            } else {
                let bg_x = x.wrapping_add(self.scx);
                let bg_y = y.wrapping_add(self.scy);
                (bg_tilemap, bg_x as u16, bg_y as u16)
            };

            let tile_col = (tile_x / 8) & 31;
            let tile_row = (tile_y / 8) & 31;
            let tile_idx_addr = map_base + tile_row * 32 + tile_col;
            let tile_num = self.vram[tile_idx_addr as usize];

            let tile_addr = if signed_tiles {
                (0x1000i32 + (tile_num as i8 as i32) * 16) as usize
            } else {
                (tile_num as usize) * 16
            };
            let line = (tile_y % 8) * 2;
            let lo = self.vram[tile_addr + line as usize];
            let hi = self.vram[tile_addr + line as usize + 1];
            let bit = 7 - (tile_x % 8);
            let color = (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1);
            let shade = (self.bgp >> (color * 2)) & 0x03;
            self.put_pixel(x as usize, y as usize, SHADES[shade as usize]);
            // Track BG color index 0 for sprite priority.
            self.bg_index[x as usize] = color;
        }
        if window_drawn {
            self.window_line = self.window_line.wrapping_add(1);
        }
    }

    fn render_sprites(&mut self, y: u8) {
        let tall = self.lcdc & 0x04 != 0;
        let height: u8 = if tall { 16 } else { 8 };

        // Collect up to 10 sprites on this line, in OAM order (lower OAM index =
        // higher priority on DMG when X ties).
        let mut visible: Vec<usize> = Vec::with_capacity(10);
        for i in 0..40 {
            let oy = self.oam[i * 4];
            let sy = oy as i16 - 16;
            if (y as i16) >= sy && (y as i16) < sy + height as i16 {
                visible.push(i);
                if visible.len() == 10 {
                    break;
                }
            }
        }
        // Draw lowest priority first so higher priority overwrites. DMG priority:
        // smaller X wins; on equal X, smaller OAM index wins. So sort by (x desc,
        // index desc) and draw in that order -> the winner is drawn last.
        visible.sort_by(|&a, &b| {
            let xa = self.oam[a * 4 + 1];
            let xb = self.oam[b * 4 + 1];
            xb.cmp(&xa).then(b.cmp(&a))
        });

        for &i in &visible {
            let sy = self.oam[i * 4] as i16 - 16;
            let sx = self.oam[i * 4 + 1] as i16 - 8;
            let mut tile = self.oam[i * 4 + 2];
            let attr = self.oam[i * 4 + 3];
            let flip_x = attr & 0x20 != 0;
            let flip_y = attr & 0x40 != 0;
            let behind_bg = attr & 0x80 != 0;
            let palette = if attr & 0x10 != 0 { self.obp1 } else { self.obp0 };

            let mut row = (y as i16 - sy) as u8;
            if flip_y {
                row = height - 1 - row;
            }
            if tall {
                tile &= 0xFE; // ignore low bit in 8x16 mode
            }
            let tile_addr = (tile as usize) * 16 + (row as usize) * 2;
            let lo = self.vram[tile_addr];
            let hi = self.vram[tile_addr + 1];

            for px in 0..8u8 {
                let bit = if flip_x { px } else { 7 - px };
                let color = (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1);
                if color == 0 {
                    continue; // transparent
                }
                let screen_x = sx + px as i16;
                if !(0..W as i16).contains(&screen_x) {
                    continue;
                }
                let sxu = screen_x as usize;
                if behind_bg && self.bg_index[sxu] != 0 {
                    continue; // BG (colors 1-3) is in front
                }
                let shade = (palette >> (color * 2)) & 0x03;
                self.put_pixel(sxu, y as usize, SHADES[shade as usize]);
            }
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, rgba: [u8; 4]) {
        let o = (y * W + x) * 4;
        self.framebuffer[o..o + 4].copy_from_slice(&rgba);
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    pub(crate) fn serialize(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.vram);
        out.extend_from_slice(&self.oam);
        out.extend_from_slice(&[
            self.lcdc, self.stat, self.scy, self.scx, self.ly, self.lyc, self.bgp, self.obp0,
            self.obp1, self.wy, self.wx, self.window_line,
        ]);
        out.extend_from_slice(&self.dot.to_le_bytes());
    }

    pub(crate) fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        let need = 0x2000 + 0xA0 + 12 + 4;
        if s.len() < *p + need {
            return false;
        }
        self.vram.copy_from_slice(&s[*p..*p + 0x2000]);
        *p += 0x2000;
        self.oam.copy_from_slice(&s[*p..*p + 0xA0]);
        *p += 0xA0;
        let r = &s[*p..*p + 12];
        [
            self.lcdc, self.stat, self.scy, self.scx, self.ly, self.lyc, self.bgp, self.obp0,
            self.obp1, self.wy, self.wx, self.window_line,
        ] = [
            r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8], r[9], r[10], r[11],
        ];
        *p += 12;
        self.dot = u32::from_le_bytes([s[*p], s[*p + 1], s[*p + 2], s[*p + 3]]);
        *p += 4;
        true
    }

    // --- register access ----------------------------------------------------

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => self.stat | 0x80,
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF40 => {
                let was_on = self.lcd_on();
                self.lcdc = val;
                if was_on && !self.lcd_on() {
                    // Turning the LCD off resets the line state.
                    self.ly = 0;
                    self.dot = 0;
                    self.stat &= !0x03;
                }
            }
            0xFF41 => self.stat = (self.stat & 0x87) | (val & 0x78),
            0xFF42 => self.scy = val,
            0xFF43 => self.scx = val,
            0xFF44 => {} // LY is read-only
            0xFF45 => {
                self.lyc = val;
                self.check_lyc();
            }
            0xFF47 => self.bgp = val,
            0xFF48 => self.obp0 = val,
            0xFF49 => self.obp1 = val,
            0xFF4A => self.wy = val,
            0xFF4B => self.wx = val,
            _ => {}
        }
    }
}
