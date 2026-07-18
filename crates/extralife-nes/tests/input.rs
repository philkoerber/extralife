//! End-to-end input regression: a button press must reach the core and affect
//! output. Uses blargg's `read_joy3/test_buttons.nes`, which prompts for a
//! button and reacts to what it receives — a self-contained proof that
//! `set_button` → controller strobe/shift → `$4016` serial read is wired right.

use extralife_core::{Button, Device};
use extralife_nes::Nes;
use std::path::PathBuf;

fn rom() -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/roms/nes-test-roms/read_joy3/test_buttons.nes");
    std::fs::read(p).unwrap()
}

fn fb_hash(nes: &Nes) -> u64 {
    let mut h = 1469598103934665603u64;
    for b in nes.framebuffer() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

#[test]
fn button_press_reaches_core() {
    let mut nes = Nes::default();
    nes.load_rom(&rom()).unwrap();
    for _ in 0..180 {
        nes.step_frame();
    }
    let before = fb_hash(&nes);

    // The prompt asks for a button; feed one and let the ROM react. The exact
    // pass/fail text doesn't matter — any frame change proves input landed
    // (with input disconnected the ROM sits on a static prompt forever).
    nes.set_button(Button::A, true);
    for _ in 0..8 {
        nes.step_frame();
    }
    nes.set_button(Button::A, false);
    for _ in 0..30 {
        nes.step_frame();
    }

    assert_ne!(
        fb_hash(&nes),
        before,
        "framebuffer unchanged after a button press — input isn't reaching the core"
    );
}
