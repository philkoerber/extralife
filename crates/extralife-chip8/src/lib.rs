//! CHIP-8 core — the pipeline proof for extralife.
//!
//! ponytail: this is a deliberately empty shell. It implements the `Device`
//! contract with a black 64x32 screen and nothing else, so the workspace
//! compiles and CI can exercise the full seam (Rust -> `Device` -> golden PNG)
//! before a single opcode exists. The actual ~35-opcode VM is the job of the
//! CHIP-8 build session; grow this file there, don't scaffold it now.

use extralife_core::{Button, Device, LoadError, Screen};

const SCREEN: Screen = Screen::new(64, 32);

pub struct Chip8 {
    framebuffer: Vec<u8>,
}

impl Default for Chip8 {
    fn default() -> Self {
        Self {
            framebuffer: vec![0; SCREEN.framebuffer_len()],
        }
    }
}

impl Device for Chip8 {
    fn screen(&self) -> Screen {
        SCREEN
    }

    fn load_rom(&mut self, _rom: &[u8]) -> Result<(), LoadError> {
        // ponytail: no memory map yet, so any ROM "loads" into a blank machine.
        self.framebuffer.iter_mut().for_each(|b| *b = 0);
        Ok(())
    }

    fn step_frame(&mut self) {
        // ponytail: no CPU yet. A real frame executes ~N cycles here.
    }

    fn set_button(&mut self, _button: Button, _pressed: bool) {}

    fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    fn save_state(&self) -> Vec<u8> {
        self.framebuffer.clone()
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError> {
        if state.len() != SCREEN.framebuffer_len() {
            return Err(LoadError::Invalid);
        }
        self.framebuffer.copy_from_slice(state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one runnable check: the core honors the `Device` contract's framebuffer
    /// invariant and save/load round-trips. Fails the moment the contract is violated.
    #[test]
    fn honors_framebuffer_and_state_contract() {
        let mut c = Chip8::default();
        assert!(c.load_rom(&[]).is_ok());
        assert_eq!(c.framebuffer().len(), SCREEN.framebuffer_len());

        c.step_frame();
        let snapshot = c.save_state();
        assert!(c.load_state(&snapshot).is_ok());
        assert!(c.load_state(&[1, 2, 3]).is_err(), "malformed state must be rejected");
    }
}
