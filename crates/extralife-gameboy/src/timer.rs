//! Timer + divider (DIV/TIMA/TMA/TAC) per Pandocs.
//!
//! Modeled on the real hardware's internal 16-bit counter: DIV is the top 8
//! bits, and TIMA increments on a falling edge of a selected counter bit ANDed
//! with the timer-enable. This edge model (rather than a naive "increment every
//! N cycles") is what the Blargg/Mooneye timer tests check.

pub struct Timer {
    /// Internal 16-bit counter; DIV (0xFF04) is its high byte.
    div: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    /// Last value of (selected bit AND enable), for falling-edge detection.
    last_edge: bool,
    /// TIMA overflow is delayed one M-cycle before reload + IRQ (hardware quirk).
    reload_pending: bool,
    /// Interrupt request latch: set when TIMA overflows and reloads.
    pub irq: bool,
}

impl Default for Timer {
    fn default() -> Self {
        Timer {
            // Post-boot DIV high byte is 0xAB on DMG; low bits arbitrary but
            // deterministic. Using 0xABCC keeps DIV=0xAB at reset.
            div: 0xABCC,
            tima: 0,
            tma: 0,
            tac: 0xF8,
            last_edge: false,
            reload_pending: false,
            irq: false,
        }
    }
}

impl Timer {
    /// Advance one T-cycle. The bus calls this 4× per M-cycle.
    pub fn tick(&mut self) {
        self.set_div(self.div.wrapping_add(1));
        // Handle the delayed TIMA reload one T-cycle after overflow was detected.
        if self.reload_pending {
            self.reload_pending = false;
            self.tima = self.tma;
            self.irq = true;
        }
    }

    fn set_div(&mut self, new: u16) {
        self.div = new;
        self.update_edge();
    }

    /// The DIV bit that clocks TIMA, selected by TAC's low 2 bits.
    fn selected_bit(&self) -> u16 {
        match self.tac & 0x03 {
            0 => 1 << 9, // 4096 Hz
            1 => 1 << 3, // 262144 Hz
            2 => 1 << 5, // 65536 Hz
            _ => 1 << 7, // 16384 Hz
        }
    }

    fn update_edge(&mut self) {
        let enabled = self.tac & 0x04 != 0;
        let edge = enabled && (self.div & self.selected_bit() != 0);
        // Falling edge of (bit AND enable) increments TIMA.
        if self.last_edge && !edge {
            let (r, overflow) = self.tima.overflowing_add(1);
            self.tima = r;
            if overflow {
                // Reload + IRQ are delayed by one T-cycle (hardware quirk).
                self.tima = 0;
                self.reload_pending = true;
            }
        }
        self.last_edge = edge;
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF04 => self.set_div(0), // any write resets the whole counter
            0xFF05 => {
                // Writing TIMA during the reload-pending window cancels reload.
                self.tima = val;
                self.reload_pending = false;
            }
            0xFF06 => self.tma = val,
            0xFF07 => {
                self.tac = val;
                self.update_edge();
            }
            _ => {}
        }
    }

    /// Take and clear the timer-interrupt request.
    pub fn take_irq(&mut self) -> bool {
        let v = self.irq;
        self.irq = false;
        v
    }

    pub(crate) fn serialize(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.div.to_le_bytes());
        out.extend_from_slice(&[
            self.tima,
            self.tma,
            self.tac,
            self.last_edge as u8,
            self.reload_pending as u8,
            self.irq as u8,
        ]);
    }

    pub(crate) fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 8 {
            return false;
        }
        self.div = u16::from_le_bytes([s[*p], s[*p + 1]]);
        self.tima = s[*p + 2];
        self.tma = s[*p + 3];
        self.tac = s[*p + 4];
        self.last_edge = s[*p + 5] != 0;
        self.reload_pending = s[*p + 6] != 0;
        self.irq = s[*p + 7] != 0;
        *p += 8;
        true
    }
}
