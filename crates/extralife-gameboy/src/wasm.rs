//! wasm-bindgen surface for the Game Boy core.
//!
//! Identical shape to every other core (see `extralife-chip8/src/wasm.rs`): a
//! `Core` class with `loadRom`/`stepFrame`/`setButton`/`framebuffer`/`audio`/
//! `sampleRate`, so the JS `Device` layer drives any console without knowing
//! which one it is. `sampleRate == 0` means "no audio", keeping the JS side
//! device-agnostic; the Game Boy reports 48000 and fills `audio()` per frame.
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
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: &str);
}

/// Route Rust panics to `console.error` with the real `file:line: reason`.
/// Without this a panic in the core surfaces in the browser as an opaque
/// `RuntimeError: unreachable`, which is undebuggable. Installed once from the
/// `Core` constructor (a full panic still aborts the WASM instance — this only
/// makes the *cause* visible). Ships no extra crate: uses the wasm-bindgen we
/// already depend on. ponytail: not `#[cfg(target_arch = "wasm32")]`-gated
/// because `wasm.rs` is only ever compiled into the cdylib WASM build anyway.
fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| error(&info.to_string())));
    });
}

#[wasm_bindgen]
pub struct Core {
    inner: GameBoy,
}

#[wasm_bindgen]
impl Core {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Core {
        install_panic_hook();
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

    /// Interleaved stereo f32 produced during the last `stepFrame`. Copied into
    /// a JS `Float32Array`. Empty until the frame has run.
    pub fn audio(&self) -> Vec<f32> {
        self.inner.audio().to_vec()
    }

    /// Output sample rate in Hz (48000 for the Game Boy APU). A value of 0 means
    /// the core produces no audio, which lets the JS side stay device-agnostic.
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
