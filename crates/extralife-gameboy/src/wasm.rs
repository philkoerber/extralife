//! wasm-bindgen surface for the Game Boy core.
//!
//! Identical shape to every other core (see `extralife-chip8/src/wasm.rs`): a
//! `Core` class with `loadRom`/`stepFrame`/`setButton`/`framebuffer`, so the JS
//! `Device` layer drives any console without knowing which one it is.
//!
//! Button index maps to `extralife_core::Button` in declaration order:
//! 0=Up 1=Down 2=Left 3=Right 4=A 5=B 6=X 7=Y 8=L 9=R 10=Start 11=Select.
//! The Game Boy uses Up/Down/Left/Right/A/B/Start/Select; X/Y/L/R are ignored.

use crate::GameBoy;
use extralife_core::{Button, Device};
use wasm_bindgen::prelude::*;

const BUTTONS: [Button; 12] = [
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
    Button::A,
    Button::B,
    Button::X,
    Button::Y,
    Button::L,
    Button::R,
    Button::Start,
    Button::Select,
];

#[wasm_bindgen]
pub struct Core {
    inner: GameBoy,
}

#[wasm_bindgen]
impl Core {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Core {
        Core {
            inner: GameBoy::default(),
        }
    }

    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.inner.screen().width
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.inner.screen().height
    }

    #[wasm_bindgen(js_name = loadRom)]
    pub fn load_rom(&mut self, rom: &[u8]) -> Result<(), JsError> {
        self.inner
            .load_rom(rom)
            .map_err(|_| JsError::new("invalid ROM for this device"))
    }

    #[wasm_bindgen(js_name = stepFrame)]
    pub fn step_frame(&mut self) {
        self.inner.step_frame();
    }

    #[wasm_bindgen(js_name = setButton)]
    pub fn set_button(&mut self, button: usize, pressed: bool) {
        if let Some(&b) = BUTTONS.get(button) {
            self.inner.set_button(b, pressed);
        }
    }

    /// RGBA8888 framebuffer, copied out to a JS `Uint8Array`.
    pub fn framebuffer(&self) -> Vec<u8> {
        self.inner.framebuffer().to_vec()
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}
