//! wasm-bindgen surface for the NES core.
//!
//! Identical shape to every other core: a `Core` class with `loadRom`/
//! `stepFrame`/`setButton`/`framebuffer`/`audio`/`sampleRate`, so the JS
//! `Device` layer drives any console without knowing which one it is.
//!
//! Button index maps to `extralife_core::Button` in declaration order:
//! 0=Up 1=Down 2=Left 3=Right 4=A 5=B 6=X 7=Y 8=L 9=R 10=Start 11=Select.
//! The NES pad uses Up/Down/Left/Right/A/B/Start/Select; X/Y/L/R are ignored.

use crate::Nes;
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
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: &str);
}

/// Route Rust panics to `console.error` with the real `file:line: reason`, so a
/// core panic is debuggable instead of an opaque `RuntimeError: unreachable`.
/// Installed once from the `Core` constructor.
fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| error(&info.to_string())));
    });
}

#[wasm_bindgen]
pub struct Core {
    inner: Nes,
}

#[wasm_bindgen]
impl Core {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Core {
        install_panic_hook();
        Core { inner: Nes::default() }
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

    /// Interleaved stereo f32 produced during the last `stepFrame`.
    pub fn audio(&self) -> Vec<f32> {
        self.inner.audio().to_vec()
    }

    /// Output sample rate in Hz (48000 for the NES APU).
    #[wasm_bindgen(getter, js_name = sampleRate)]
    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}
