//! wasm-bindgen surface for the Tamagotchi P1 core. Same shape as every other
//! core (see `extralife-chip8/src/wasm.rs`) so the JS `Device` interface drives
//! it without knowing the console. Button index maps to `extralife_core::Button`
//! declaration order: 0=Up 1=Down 2=Left 3=Right 4=A 5=B 6=X 7=Y 8=L 9=R
//! 10=Start 11=Select. The P1 uses A(4), B(5) and Select(11) for its 3 buttons.

use crate::Tamagotchi;
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

fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| error(&info.to_string())));
    });
}

#[wasm_bindgen]
pub struct Core {
    inner: Tamagotchi,
}

#[wasm_bindgen]
impl Core {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Core {
        install_panic_hook();
        Core {
            inner: Tamagotchi::default(),
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

    pub fn framebuffer(&self) -> Vec<u8> {
        self.inner.framebuffer().to_vec()
    }

    pub fn audio(&self) -> Vec<f32> {
        self.inner.audio().to_vec()
    }

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
