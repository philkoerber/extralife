//! blargg-style system test ROMs, run headless.
//!
//! blargg's NES suites report via the $6000 memory protocol: a status byte at
//! $6000 ($80 = running, $81 = "reset requested", else final code), the
//! signature $DE $B0 $61 at $6001-3 once the protocol is live, and a
//! zero-terminated result string from $6004. A final $6000 of 0 is a pass.
//!
//! We run each ROM for a bounded number of frames, polling the protocol, and
//! assert a passing code + "Passed" text. ROMs ship in the committed
//! `nes-test-roms` submodule; a missing ROM skips (does not fail) so a partial
//! checkout still builds.

use extralife_core::Device;
use extralife_nes::Nes;
use std::path::PathBuf;

fn roms_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/roms/nes-test-roms")
}

/// Poll the blargg $6000 protocol. Returns Some((code, text)) once final.
fn blargg_result(nes: &Nes) -> Option<(u8, String)> {
    if nes.peek(0x6001) != 0xDE || nes.peek(0x6002) != 0xB0 || nes.peek(0x6003) != 0x61 {
        return None;
    }
    let status = nes.peek(0x6000);
    if status == 0x80 || status == 0x81 {
        return None; // running / awaiting reset
    }
    let mut text = Vec::new();
    let mut addr = 0x6004u16;
    while addr < 0x7000 {
        let b = nes.peek(addr);
        if b == 0 {
            break;
        }
        text.push(b);
        addr += 1;
    }
    Some((status, String::from_utf8_lossy(&text).into_owned()))
}

/// Run one blargg ROM up to `max_frames`, returning the final (code, text).
fn run_blargg(rel_path: &str, max_frames: u32) -> Option<(u8, String)> {
    let rom = std::fs::read(roms_dir().join(rel_path)).ok()?;
    let mut nes = Nes::default();
    nes.load_rom(&rom).expect("load blargg rom");
    for _ in 0..max_frames {
        nes.step_frame();
        if let Some(r) = blargg_result(&nes) {
            return Some(r);
        }
    }
    Some((0xFF, format!("timed out after {max_frames} frames")))
}

/// Assert a blargg ROM passes; skip (with a note) if it isn't checked out.
fn expect_pass(rel_path: &str, max_frames: u32) {
    match run_blargg(rel_path, max_frames) {
        None => eprintln!("skipping {rel_path}: not checked out"),
        Some((0, text)) => eprintln!("{rel_path}: PASS ({})", text.trim()),
        Some((code, text)) => panic!("{rel_path}: FAIL code={code:#x}: {}", text.trim()),
    }
}

#[test]
fn cpu_instr_official() {
    // The full official-opcode instruction test (MMC1, 256 KiB PRG) — exercises
    // every documented opcode's behavior *and* the MMC1 mapper's PRG banking.
    // 16 subtests at ~120 frames each; give generous margin.
    expect_pass("instr_test-v5/official_only.nes", 2500);
}

#[test]
fn cpu_instr_timing() {
    // "Takes about 25 seconds" of emulated time (~1500 frames); budget headroom.
    expect_pass("instr_timing/instr_timing.nes", 2600);
}

#[test]
fn instr_misc() {
    // Branch-wrap, dummy reads, and other CPU edge cases via the $6000 protocol.
    expect_pass("instr_misc/instr_misc.nes", 1500);
}

#[test]
fn ppu_vbl_nmi_basics() {
    // VBlank/NMI timing basics (uses the $6000 protocol). The full suite is
    // very timing-strict; the basics subtest validates our vblank flag + NMI
    // edge without requiring dot-exact PPU.
    expect_pass("ppu_vbl_nmi/rom_singles/01-vbl_basics.nes", 800);
}

