//! Regression guard: a real MBC5 game (Pokémon Red) must run without panicking.
//!
//! A panic in the core surfaces in the browser as an opaque
//! `RuntimeError: unreachable` (that's what the `console.error` panic hook in
//! `wasm.rs` now makes debuggable). This test is the native tripwire: it boots
//! the ROM and steps ~60 s of emulated time while mashing Start/A like a player
//! clearing the intro, so any reachable `unwrap`/index-OOB/`unreachable!` in the
//! boot+menu path fails here — with a real file:line — long before WASM.
//!
//! ROM lives under `tests/roms/pokemon-gb` (gitignored, not a submodule); the
//! test skips cleanly when it's absent so CI without the ROM stays green.

use extralife_core::{Button, Device};
use extralife_gameboy::GameBoy;
use std::path::PathBuf;

#[test]
fn pokemon_red_runs_without_panic() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/roms/pokemon-gb/Pokemon - Rote Edition (Germany) (SGB Enhanced).gb");
    if !path.exists() {
        eprintln!("skipping: Pokémon Red ROM not present");
        return;
    }
    let rom = std::fs::read(&path).expect("read ROM");
    let mut gb = GameBoy::default();
    gb.load_rom(&rom).expect("MBC5 ROM must load");

    // Two phases matching what the browser does: first let the attract/intro
    // demo run untouched for a long stretch (that's where the browser hit an
    // out-of-bounds), then mash Start/A to drive into the menus.
    for frame in 0..20_000u32 {
        if frame < 10_000 {
            // pure intro demo, no input
        } else {
            let pressed = (frame / 8) % 2 == 0;
            gb.set_button(Button::Start, pressed);
            gb.set_button(Button::A, !pressed);
        }
        gb.step_frame();
        // Exercise the exact host-facing calls the browser makes each frame.
        let _ = gb.framebuffer();
        let _ = gb.audio();
    }
}
