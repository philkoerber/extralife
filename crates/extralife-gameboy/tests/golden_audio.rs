//! Golden-audio regression for the Game Boy APU.
//!
//! Analogous to the golden-image harness (`golden.rs`), but for sound: run a
//! deterministic ROM that exercises the sound hardware for a fixed number of
//! frames, collect the interleaved stereo f32 samples `audio()` produces, and
//! diff them against a committed reference buffer under `tests/golden/gameboy/`.
//!
//! The APU is clocked from the system clock (not wall-clock), so the sample
//! stream is byte-identical every run — that determinism is what makes this
//! diff meaningful. The reference is our own output, generated once with
//! `UPDATE_GOLDEN=1` and committed.
//!
//! The driver ROM is Blargg's `dmg_sound/rom_singles/01-registers.gb` (already a
//! committed submodule). It writes to the channel registers and triggers tones,
//! so the first frames contain a non-silent, structured waveform. If the ROM is
//! absent the test is skipped.
//!
//! Regenerate after a legitimate APU change:
//!     UPDATE_GOLDEN=1 cargo test -p extralife-gameboy --test golden_audio

use extralife_core::Device;
use extralife_gameboy::GameBoy;
use std::path::PathBuf;

fn rom_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/roms/gb-test-roms/dmg_sound/rom_singles/01-registers.gb")
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/gameboy/apu-01-registers.f32")
}

/// Frames to run before capturing, and how many to capture. 01-registers is
/// silent while it pokes registers, then plays a short structured burst around
/// frames 24-46. We skip the leading silence and capture a 40-frame window that
/// straddles the burst, keeping the committed reference small (~256 KiB) while
/// still non-silent and waveform-shaped.
const SKIP_FRAMES: u32 = 20;
const CAPTURE_FRAMES: u32 = 40;

fn capture(rom: &[u8]) -> Vec<f32> {
    let mut gb = GameBoy::default();
    gb.load_rom(rom).expect("load ROM");
    for _ in 0..SKIP_FRAMES {
        gb.step_frame();
    }
    let mut out = Vec::new();
    for _ in 0..CAPTURE_FRAMES {
        gb.step_frame();
        out.extend_from_slice(gb.audio());
    }
    out
}

fn to_bytes(samples: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

fn from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[test]
fn golden_audio() {
    let rp = rom_path();
    let rom = match std::fs::read(&rp) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("skipping golden_audio: ROM not present at {}", rp.display());
            return;
        }
    };

    let actual = capture(&rom);
    // Sanity: the buffer must be non-silent (the whole point of a golden that a
    // human verified once as plausible sound, not zeros).
    assert!(
        actual.iter().any(|&s| s.abs() > 0.001),
        "captured audio is silent — the driver ROM produced no sound"
    );

    let gp = golden_path();
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(gp.parent().unwrap()).unwrap();
        std::fs::write(&gp, to_bytes(&actual)).unwrap();
        eprintln!("updated golden audio: {} ({} samples)", gp.display(), actual.len());
        return;
    }

    let golden = match std::fs::read(&gp) {
        Ok(b) => from_bytes(&b),
        Err(_) => panic!("missing golden audio (run UPDATE_GOLDEN=1): {}", gp.display()),
    };

    assert_eq!(
        actual.len(),
        golden.len(),
        "sample count changed: {} vs golden {}",
        actual.len(),
        golden.len()
    );
    // Deterministic core => byte-exact match.
    let diff = actual
        .iter()
        .zip(golden.iter())
        .filter(|(a, g)| a.to_bits() != g.to_bits())
        .count();
    assert_eq!(diff, 0, "{diff} of {} samples differ from golden", actual.len());
}
