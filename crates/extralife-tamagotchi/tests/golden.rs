//! Golden-image regression test for the Tamagotchi P1 core.
//!
//! The P1 program ROM is a commercial mask-ROM we do not ship (see
//! `.cursor/rules/license-policy.mdc`: this repo never contains commercial
//! ROMs). So instead of booting the pet, this golden freezes the deterministic
//! framebuffer produced by a small **hand-assembled, clean-room** E0C6200 test
//! program that drives the real display path: it points the index register at
//! display RAM and lights the top four COM rows across all 32 segments via
//! `LBPX`. If the CPU decode, the display-RAM→pixel mapping, or the framebuffer
//! packing regress, these pixels change and the diff fails.
//!
//! Regenerate after a legitimate behavior change:
//!     UPDATE_GOLDEN=1 cargo test -p extralife-tamagotchi --test golden

use extralife_core::Device;
use extralife_tamagotchi::Tamagotchi;
use std::path::PathBuf;

const W: u32 = 32;
const H: u32 = 16;

/// Assemble a P1 ROM (16-bit big-endian words) with `words` placed at the reset
/// vector (bank0/page1/step0 == word index 0x100).
fn assemble(words: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u16; 6144];
    for (i, &w) in words.iter().enumerate() {
        rom[0x100 + i] = w & 0x0FFF;
    }
    rom.iter().flat_map(|w| [(w >> 8) as u8, (w & 0xFF) as u8]).collect()
}

/// Program: IX <- 0xE00 (display RAM), then 32× `LBPX MX,0x0F` which writes
/// 0xF to M(X) (COM0-3 lit) and 0x0 to M(X+1) (COM4-7 off), advancing X by 2
/// per segment. Result: the top 4 rows are lit across all 32 columns. Then spin.
fn test_program() -> Vec<u8> {
    let mut prog = vec![
        0xB00, // LD X,0x00   (XH=0, XL=0)
        0xE0E, // LD A,0x0E
        0xE80, // LD XP,A      => IX = 0xE00
    ];
    for _ in 0..32 {
        prog.push(0x90F); // LBPX MX,0x0F
    }
    let spin = prog.len() as u16; // step index of the spin instruction
    prog.push(0x000 | spin); // JP spin  (self-loop)
    assemble(&prog)
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/tamagotchi")
}

fn render() -> Vec<u8> {
    let mut t = Tamagotchi::default();
    t.load_rom(&test_program()).expect("load ROM");
    for _ in 0..2 {
        t.step_frame();
    }
    t.framebuffer().to_vec()
}

fn save_png(path: &PathBuf, rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(path, rgba, W, H, image::ExtendedColorType::Rgba8).expect("write png");
}

#[test]
fn golden_images() {
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let actual = render();
    let golden_path = golden_dir().join("display-top-rows.png");

    if update {
        save_png(&golden_path, &actual);
        eprintln!("updated golden: {}", golden_path.display());
        return;
    }

    let golden = image::open(&golden_path)
        .unwrap_or_else(|_| panic!("missing golden (run UPDATE_GOLDEN=1)"))
        .to_rgba8()
        .into_raw();

    if golden != actual {
        let actual_path = golden_dir().join("__actual__").join("display-top-rows.png");
        save_png(&actual_path, &actual);
        let n = actual
            .chunks(4)
            .zip(golden.chunks(4))
            .filter(|(a, g)| a != g)
            .count();
        panic!("{n} pixels differ (see {})", actual_path.display());
    }
}
