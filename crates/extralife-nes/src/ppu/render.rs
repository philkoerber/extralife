//! PPU dot state machine + per-scanline background and sprite rendering.
//!
//! `tick` advances one dot. On visible lines we render the whole line into the
//! framebuffer at dot 256 (end of visible pixels), then perform the loopy
//! scroll increments the hardware does (coarse-X per tile is folded into the
//! batched render; the coarse-Y increment at dot 256 and the horizontal-copy at
//! dot 257 and vertical-copy over dots 280-304 on the pre-render line are done
//! so `v` tracks the real scroll origin used for the next line).

use super::{Ppu, DOTS, LINES, PALETTE, W};
use crate::cartridge::Cartridge;

impl Ppu {
    /// Advance the PPU by one dot. Called three times per CPU cycle.
    pub fn tick(&mut self, cart: &mut Cartridge) {
        let visible = self.scanline < 240;
        let pre_render = self.scanline == 261;

        // Sprite-0 / vblank flag handling at specific dots.
        if self.scanline == 241 && self.dot == 1 {
            self.status |= 0x80; // set vblank
            if self.ctrl & 0x80 != 0 {
                self.nmi_line = true;
            }
        }
        if pre_render && self.dot == 1 {
            self.status &= !0xE0; // clear vblank, sprite-0, overflow
        }

        // Render a visible scanline in one shot at dot 256.
        if visible && self.dot == 256 {
            self.render_scanline(cart);
        }

        // Loopy scroll bookkeeping (only while rendering is enabled).
        if self.rendering_enabled() {
            if (visible || pre_render) && self.dot == 256 {
                self.increment_y();
            }
            if (visible || pre_render) && self.dot == 257 {
                self.copy_horizontal();
            }
            if pre_render && self.dot >= 280 && self.dot <= 304 {
                self.copy_vertical();
            }
            // MMC3 scanline IRQ: approximate the A12 rising edge with one clock
            // per visible/pre-render line at dot 260 (when sprite pattern
            // fetches raise A12 in normal configurations).
            if (visible || pre_render) && self.dot == 260 {
                cart.mmc3_clock_scanline();
            }
        }

        // Advance dot/scanline, honoring the odd-frame skipped dot.
        self.dot += 1;
        if pre_render
            && self.dot == 340
            && self.odd_frame
            && self.rendering_enabled()
        {
            self.dot += 1; // skip the last dot on odd frames
        }
        if self.dot >= DOTS {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline >= LINES {
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
                self.frame_ready = true;
            }
        }
    }

    fn increment_y(&mut self) {
        // Fine Y increment with coarse-Y carry / nametable flip (NESdev).
        if (self.v & 0x7000) != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let mut y = (self.v & 0x03E0) >> 5;
            if y == 29 {
                y = 0;
                self.v ^= 0x0800;
            } else if y == 31 {
                y = 0;
            } else {
                y += 1;
            }
            self.v = (self.v & !0x03E0) | (y << 5);
        }
    }

    fn copy_horizontal(&mut self) {
        // Copy coarse-X and horizontal nametable bit from t to v.
        self.v = (self.v & !0x041F) | (self.t & 0x041F);
    }
    fn copy_vertical(&mut self) {
        // Copy fine-Y, coarse-Y and vertical nametable bit from t to v.
        self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
    }

    /// Render the current visible scanline (self.scanline) using the current v
    /// register as the scroll origin. Background then sprites, with priority
    /// and a sprite-0-hit test.
    fn render_scanline(&mut self, cart: &Cartridge) {
        let y = self.scanline as usize;
        let show_bg = self.mask & 0x08 != 0;
        let show_sp = self.mask & 0x10 != 0;
        let left_bg = self.mask & 0x02 != 0;
        let left_sp = self.mask & 0x04 != 0;

        // Background color index (0..3) per pixel, and the composited palette.
        let mut bg_pixels = [0u8; W];
        if show_bg {
            self.render_bg_line(cart, y, &mut bg_pixels, left_bg);
        }

        // Base backdrop.
        let backdrop = self.palette[0];
        for x in 0..W {
            let pal_entry = if bg_pixels[x] & 0x03 != 0 {
                self.palette[bg_pixels[x] as usize & 0x1F]
            } else {
                backdrop
            };
            self.put_pixel(x, y, pal_entry);
        }

        if show_sp {
            self.render_sprites_line(cart, y, &bg_pixels, left_sp);
        }
    }

    fn render_bg_line(&self, cart: &Cartridge, _y: usize, out: &mut [u8; W], left: bool) {
        // Derive the scroll from v/x. We reconstruct the per-pixel nametable
        // fetch from the loopy address rather than a fetch pipeline.
        let fine_y = (self.v >> 12) & 0x07;
        let base_nt = 0x2000 | (self.v & 0x0C00);
        let coarse_x0 = self.v & 0x1F;
        let coarse_y = (self.v >> 5) & 0x1F;
        let pattern_base: u16 = if self.ctrl & 0x10 != 0 { 0x1000 } else { 0 };

        for screen_x in 0..W {
            if !left && screen_x < 8 {
                continue;
            }
            let fine_x = (self.x as usize + screen_x) & 0x07;
            let tile_col = coarse_x0 + ((self.x as usize + screen_x) / 8) as u16;
            // Handle horizontal nametable wrap across the 32-tile boundary.
            let mut nt = base_nt;
            let mut cx = tile_col;
            if cx >= 32 {
                cx -= 32;
                nt ^= 0x0400;
            }
            let nt_addr = nt | (coarse_y << 5) | cx;
            let tile = self.vram_read_nt(cart, nt_addr);
            let attr_addr =
                (nt & 0x0C00) | 0x03C0 | ((coarse_y >> 2) << 3) | (cx >> 2);
            let attr = self.vram_read_nt(cart, 0x2000 | attr_addr);
            let shift = (((coarse_y & 0x02) << 1) | (cx & 0x02)) as u8;
            let palette_hi = (attr >> shift) & 0x03;

            let pat = pattern_base + (tile as u16) * 16 + fine_y;
            let lo = cart.ppu_read(pat);
            let hi = cart.ppu_read(pat + 8);
            let bit = 7 - fine_x;
            let color = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
            out[screen_x] = if color == 0 {
                0
            } else {
                (palette_hi << 2) | color
            };
        }
    }

    fn render_sprites_line(&mut self, cart: &Cartridge, y: usize, bg: &[u8; W], left: bool) {
        let tall = self.ctrl & 0x20 != 0;
        let height = if tall { 16 } else { 8 };
        let sp_pattern: u16 = if self.ctrl & 0x08 != 0 { 0x1000 } else { 0 };

        // Hardware scans OAM front-to-back and draws back-to-front for priority;
        // we collect the up-to-8 sprites on this line then draw in reverse.
        let mut chosen: [usize; 8] = [0; 8];
        let mut n = 0;
        for i in 0..64 {
            let sy = self.oam[i * 4] as usize;
            if y >= sy && y < sy + height {
                if n < 8 {
                    chosen[n] = i;
                    n += 1;
                } else {
                    self.status |= 0x20; // sprite overflow (approx)
                    break;
                }
            }
        }

        for &i in chosen[..n].iter().rev() {
            let sy = self.oam[i * 4] as usize;
            let tile = self.oam[i * 4 + 1];
            let attr = self.oam[i * 4 + 2];
            let sx = self.oam[i * 4 + 3] as usize;
            let flip_h = attr & 0x40 != 0;
            let flip_v = attr & 0x80 != 0;
            let behind = attr & 0x20 != 0;
            let palette_hi = 4 + (attr & 0x03);

            let mut row = y - sy;
            if flip_v {
                row = height - 1 - row;
            }
            let (pat_addr, fine_row) = if tall {
                let base = ((tile & 1) as u16) * 0x1000 + ((tile & 0xFE) as u16) * 16;
                let r = row as u16;
                (base + (r & 7) + if r >= 8 { 16 } else { 0 }, 0u16)
            } else {
                (sp_pattern + (tile as u16) * 16 + row as u16, 0)
            };
            let _ = fine_row;
            let lo = cart.ppu_read(pat_addr);
            let hi = cart.ppu_read(pat_addr + 8);

            for col in 0..8 {
                let x = sx + col;
                if x >= W {
                    continue;
                }
                if !left && x < 8 {
                    continue;
                }
                let bit = if flip_h { col } else { 7 - col };
                let color = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
                if color == 0 {
                    continue; // transparent
                }
                let bg_opaque = bg[x] & 0x03 != 0;
                // Sprite-0 hit: sprite index 0, both pixels opaque, x != 255.
                if i == 0 && bg_opaque && x != 255 {
                    self.status |= 0x40;
                }
                if behind && bg_opaque {
                    continue; // BG has priority
                }
                let entry = self.palette[(palette_hi << 2 | color) as usize & 0x1F];
                self.put_pixel(x, y, entry);
            }
        }
    }

    /// Nametable read that goes through mirroring (used by the batched renderer).
    fn vram_read_nt(&self, cart: &Cartridge, addr: u16) -> u8 {
        self.vram[self.nt_index(cart, addr)]
    }

    fn put_pixel(&mut self, x: usize, y: usize, palette_entry: u8) {
        let [r, g, b] = PALETTE[(palette_entry & 0x3F) as usize];
        let o = (y * W + x) * 4;
        self.framebuffer[o] = r;
        self.framebuffer[o + 1] = g;
        self.framebuffer[o + 2] = b;
        self.framebuffer[o + 3] = 0xFF;
    }
}
