//! Golden-image regression tests for the CHIP-8 core.
//!
//! Each entry runs a test-suite ROM headless for a fixed number of frames and
//! compares the framebuffer against a committed PNG under
//! `tests/golden/chip8/`. The goldens are produced by a human-verified run
//! (the ROMs draw ✓/✗ per opcode — we eyeball "all check marks" in the browser
//! harness once, then freeze the pixels here). CI then guards against regressions.
//!
//! Regenerate after a legitimate behavior change:
//!     UPDATE_GOLDEN=1 cargo test -p extralife-chip8 --test golden

use extralife_chip8::Chip8;
use extralife_core::Device;
use std::path::PathBuf;

const W: u32 = 64;
const H: u32 = 32;

/// (rom file, frames to run, golden name). Frames chosen so each ROM has settled
/// on its result screen. Scrolling/beep/keypad are interactive or SUPER-CHIP-only
/// and excluded from golden diffing.
const CASES: &[(&str, u32, &str)] = &[
    ("1-chip8-logo.ch8", 40, "1-chip8-logo"),
    ("2-ibm-logo.ch8", 40, "2-ibm-logo"),
    ("3-corax+.ch8", 200, "3-corax+"),
    ("4-flags.ch8", 300, "4-flags"),
    ("5-quirks.ch8", 400, "5-quirks"),
];

fn roms_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/roms/chip8-test-suite/bin")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/chip8")
}

fn render(rom_file: &str, frames: u32) -> Vec<u8> {
    let rom = std::fs::read(roms_dir().join(rom_file)).expect("read ROM");
    let mut c = Chip8::default();
    c.load_rom(&rom).expect("load ROM");
    for _ in 0..frames {
        c.step_frame();
    }
    c.framebuffer().to_vec()
}

fn save_png(path: &PathBuf, rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(path, rgba, W, H, image::ExtendedColorType::Rgba8).expect("write png");
}

#[test]
fn golden_images() {
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let mut failures = Vec::new();

    for &(rom_file, frames, name) in CASES {
        let actual = render(rom_file, frames);
        let golden_path = golden_dir().join(format!("{name}.png"));

        if update {
            save_png(&golden_path, &actual);
            eprintln!("updated golden: {}", golden_path.display());
            continue;
        }

        let golden = match image::open(&golden_path) {
            Ok(img) => img.to_rgba8().into_raw(),
            Err(_) => {
                failures.push(format!("{name}: missing golden (run UPDATE_GOLDEN=1)"));
                continue;
            }
        };

        if golden != actual {
            // Dump actual + a red-highlighted diff next to the golden (gitignored).
            let actual_path = golden_dir().join("__actual__").join(format!("{name}.png"));
            save_png(&actual_path, &actual);
            let diff: Vec<u8> = actual
                .chunks(4)
                .zip(golden.chunks(4))
                .flat_map(|(a, g)| {
                    if a == g {
                        a.to_vec()
                    } else {
                        vec![0xFF, 0x00, 0x00, 0xFF]
                    }
                })
                .collect();
            let diff_path = golden_dir().join("__diff__").join(format!("{name}.png"));
            save_png(&diff_path, &diff);
            let n = actual
                .chunks(4)
                .zip(golden.chunks(4))
                .filter(|(a, g)| a != g)
                .count();
            failures.push(format!(
                "{name}: {n} pixels differ (see {})",
                diff_path.display()
            ));
        }
    }

    assert!(failures.is_empty(), "golden mismatches:\n{}", failures.join("\n"));
}
