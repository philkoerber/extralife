//! Golden-image regression for the Game Boy PPU.
//!
//! dmg-acid2 (https://github.com/mattcurrie/dmg-acid2) exercises background,
//! window, and sprite rendering with a known-correct target image. We run it
//! headless for enough frames to settle, then diff the 160x144 RGBA framebuffer
//! against a committed PNG under `tests/golden/gameboy/`. The golden is our own
//! rendered output, human-verified once against the ROM's published reference,
//! then frozen so CI guards regressions.
//!
//! The ROM itself is never committed (license policy): it is built from / into
//! the gitignored `tests/roms/dmg-acid2/build/`. If it is absent the test is
//! skipped with a message.
//!
//! Regenerate after a legitimate PPU change:
//!     UPDATE_GOLDEN=1 cargo test -p extralife-gameboy --test golden

use extralife_core::Device;
use extralife_gameboy::GameBoy;
use std::path::PathBuf;

const W: u32 = 160;
const H: u32 = 144;

fn rom_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/roms/dmg-acid2/build/dmg-acid2.gb")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/gameboy")
}

fn render(rom: &[u8], frames: u32) -> Vec<u8> {
    let mut gb = GameBoy::default();
    gb.load_rom(rom).expect("load ROM");
    for _ in 0..frames {
        gb.step_frame();
    }
    gb.framebuffer().to_vec()
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
            eprintln!(
                "skipping dmg-acid2 golden: ROM not built at {} (make in the submodule, or fetch the release)",
                rp.display()
            );
            return;
        }
    };

    // dmg-acid2 draws its final image within the first couple of frames; give it
    // margin so LCD-on timing and the initial VBlank have settled.
    let actual = render(&rom, 30);
    let golden_path = golden_dir().join("dmg-acid2.png");

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
        let actual_path = golden_dir().join("__actual__").join("dmg-acid2.png");
        save_png(&actual_path, &actual);
        let n = actual
            .chunks(4)
            .zip(golden.chunks(4))
            .filter(|(a, g)| a != g)
            .count();
        panic!(
            "dmg-acid2: {n} pixels differ (actual written to {})",
            actual_path.display()
        );
    }
}
