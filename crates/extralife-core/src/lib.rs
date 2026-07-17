//! The device-agnostic contract every extralife core implements.
//!
//! One trait, `Device`, is the entire seam between "whatever the emulator was
//! written in, rewritten to Rust" and "the React component + CI harness". If a
//! struct implements `Device`, it can be:
//!   - driven headless in CI (step frames, diff the framebuffer against a golden PNG),
//!   - compiled to WASM and wrapped by the `<ExtraLife>` component,
//!   - snapshotted for save states / rewind,
//!
//! without the caller knowing which console it is.
//!
//! Design rules (keep this file small — it is the spec):
//!   - Deterministic: same ROM + same inputs => same framebuffer, every run.
//!   - Headless: no assumption of a screen, audio device, or event loop.
//!   - Plain data at the boundary: slices and POD structs, so the WASM/JS edge
//!     is a memory read, not a serialization protocol.

/// A pressed/released button. Cores map these to their own pad; a device that
/// lacks a button simply ignores it. Kept as a superset so the React input
/// layer is one fixed enum across every console.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
}

/// Fixed, known-at-compile-time screen geometry for a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
}

impl Screen {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Bytes in one RGBA8888 framebuffer. The core must expose exactly this many.
    pub const fn framebuffer_len(&self) -> usize {
        (self.width * self.height * 4) as usize
    }
}

/// Why `load_rom` refused a ROM. Cores add nothing here — a rejected ROM is a
/// rejected ROM; detail belongs in logs, not the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The byte slice is not a valid image for this device (bad size, bad header, unsupported mapper, …).
    Invalid,
}

/// The one trait. Every `extralife-<device>` crate implements exactly this.
pub trait Device {
    /// Screen geometry. Constant for the lifetime of the device instance.
    fn screen(&self) -> Screen;

    /// Load a cartridge/program image and reset to power-on state.
    /// Must fully reset: calling twice yields a clean run each time.
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), LoadError>;

    /// Advance emulation by exactly one video frame using the current input state.
    /// Deterministic: no wall-clock, no RNG seeded from time.
    fn step_frame(&mut self);

    /// Set the held/released state of a button, effective from the next `step_frame`.
    fn set_button(&mut self, button: Button, pressed: bool);

    /// The current frame as RGBA8888, row-major, top-left origin.
    /// Length is always `screen().framebuffer_len()`.
    fn framebuffer(&self) -> &[u8];

    /// Interleaved stereo f32 samples produced during the last `step_frame`,
    /// at `sample_rate()`. Empty slice is valid (silent core / no APU yet).
    fn audio(&self) -> &[f32] {
        &[]
    }

    /// Output sample rate in Hz for `audio()`. Irrelevant if audio is empty.
    fn sample_rate(&self) -> u32 {
        0
    }

    /// Serialize full emulation state (save state / rewind ring buffer).
    /// Round-trips with `load_state`. Opaque bytes; format is the core's business.
    fn save_state(&self) -> Vec<u8>;

    /// Restore a state produced by `save_state` on a compatible core version.
    /// Returns Err if the blob is not recognizable; the core must be left unchanged on Err.
    fn load_state(&mut self, state: &[u8]) -> Result<(), LoadError>;
}
