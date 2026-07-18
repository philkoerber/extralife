//! Golden-image regression for the NES PPU.
//!
//! `window5/colorwin_ntsc.nes` (from the permissive nes-test-roms submodule)
//! renders a static screen — a colored background plus a "dummy text box" — with
//! no input required. It exercises background tile fetch, attribute/palette
//! selection, and text tile rendering, giving a clean deterministic target. We
//! run it headless until the image settles, then diff the 256x240 RGBA
//! framebuffer against a committed PNG under `tests/golden/nes/`.
//!
//! Per the loop playbook + license policy: the ROM is a permissive test ROM in
//! the submodule (never a commercial ROM), and the golden PNG is *our own*
//! rendered output, human-verified once then frozen so CI guards regressions.
//!
//! ponytail: this ROM also does a mid-screen color-window split (a raster
//! effect); our batched per-scanline renderer reproduces the static text box +
//! background faithfully but the golden captures whatever our renderer produces
//! for the split region, not necessarily hardware-exact. Ceiling: raster-split
//! demos won't be pixel-exact; upgrade path is a dot-accurate fetch pipeline.
//!
//! Regenerate after a legitimate PPU change:
//!     UPDATE_GOLDEN=1 cargo test -p extralife-nes --test golden

use extralife_core::Device;
use extralife_nes::Nes;
use std::path::PathBuf;

const W: u32 = 256;
const H: u32 = 240;

fn rom_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/roms/nes-test-roms/window5/colorwin_ntsc.nes")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/nes")
}

fn render(rom: &[u8], frames: u32) -> Vec<u8> {
    let mut nes = Nes::default();
    nes.load_rom(rom).expect("load ROM");
    for _ in 0..frames {
        nes.step_frame();
    }
    nes.framebuffer().to_vec()
}

fn save_png(path: &PathBuf, rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(path, rgba, W, H, image::ExtendedColorType::Rgba8).expect("write png");
}

#[test]
fn golden_images() {
    let rp = rom_path();
    let rom = match std::fs::read(&rp) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("skipping NES golden: ROM not found at {}", rp.display());
            return;
        }
    };

    // The dummy text box + background are drawn during init; a couple dozen
    // frames settles the initial vblank and the static background.
    let actual = render(&rom, 30);
    let golden_path = golden_dir().join("colorwin.png");

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        save_png(&golden_path, &actual);
        eprintln!("updated golden: {}", golden_path.display());
        return;
    }

    let golden = match image::open(&golden_path) {
        Ok(img) => img.to_rgba8().into_raw(),
        Err(_) => panic!("missing golden (run UPDATE_GOLDEN=1): {}", golden_path.display()),
    };

    if golden != actual {
        let actual_path = golden_dir().join("__actual__").join("colorwin.png");
        save_png(&actual_path, &actual);
        let n = actual
            .chunks(4)
            .zip(golden.chunks(4))
            .filter(|(a, g)| a != g)
            .count();
        panic!(
            "colorwin: {n} pixels differ (actual written to {})",
            actual_path.display()
        );
    }
}
