//! Joypad (0xFF00). Two 4-bit selectable rows (buttons / dpad) read as active-
//! low: 0 = pressed. A high-to-low transition on any selected line requests a
//! joypad interrupt.

use extralife_core::Button;

#[derive(Default)]
pub struct Joypad {
    /// Which row is selected: bit5=select buttons, bit4=select dpad (active low).
    select: u8,
    /// Pressed state. Direction: bit0 R,1 L,2 U,3 D. Buttons: bit0 A,1 B,2 Sel,3 Start.
    dpad: u8,
    buttons: u8,
    irq: bool,
}

impl Joypad {
    pub fn set(&mut self, button: Button, pressed: bool) {
        let (field, bit) = match button {
            Button::Right => (Field::Dpad, 0),
            Button::Left => (Field::Dpad, 1),
            Button::Up => (Field::Dpad, 2),
            Button::Down => (Field::Dpad, 3),
            Button::A => (Field::Btn, 0),
            Button::B => (Field::Btn, 1),
            Button::Select => (Field::Btn, 2),
            Button::Start => (Field::Btn, 3),
            // The Game Boy has 8 buttons; the extra abstract buttons are ignored.
            _ => return,
        };
        let target = match field {
            Field::Dpad => &mut self.dpad,
            Field::Btn => &mut self.buttons,
        };
        let before = *target;
        if pressed {
            *target |= 1 << bit;
        } else {
            *target &= !(1 << bit);
        }
        // A newly-pressed selected key drives its line low -> interrupt.
        if *target & !before != 0 {
            self.irq = true;
        }
    }

    pub fn read(&self) -> u8 {
        // Bits 6-7 always read 1. Selected row's keys read active-low.
        let mut v = 0xC0 | (self.select & 0x30);
        let mut lines = 0x0F;
        if self.select & 0x20 == 0 {
            lines &= !self.buttons;
        }
        if self.select & 0x10 == 0 {
            lines &= !self.dpad;
        }
        v |= lines & 0x0F;
        v
    }

    pub fn write(&mut self, val: u8) {
        self.select = val & 0x30;
    }

    pub fn take_irq(&mut self) -> bool {
        let v = self.irq;
        self.irq = false;
        v
    }
}

enum Field {
    Dpad,
    Btn,
}
