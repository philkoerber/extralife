//! Blargg's Game Boy test ROMs, run headless.
//!
//! Blargg's ROMs print their progress and final result to the serial port
//! (0xFF01/0xFF02); our MMU captures those bytes in `serial_out`. A run is a
//! pass when the captured text ends with "Passed" (and never says "Failed").
//! We step whole frames with a generous cap so slow suites (cpu_instrs is ~30 s
//! of emulated time) still finish in CI.
//!
//! Submodule: `tests/roms/gb-test-roms`. Skipped with a message if absent.

use extralife_gameboy::GameBoy;
use extralife_core::Device;
use std::path::PathBuf;

fn roms_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/roms/gb-test-roms")
}

/// Run a ROM until its serial log reports a result or `max_frames` elapses.
fn run_serial(path: &PathBuf, max_frames: u32) -> Result<String, String> {
    let rom = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut gb = GameBoy::default();
    gb.load_rom(&rom).map_err(|_| "load_rom rejected ROM".to_string())?;
    for _ in 0..max_frames {
        gb.step_frame();
        let text = gb.serial_text();
        if text.contains("Passed") || text.contains("Failed") {
            return Ok(text);
        }
    }
    Ok(gb.serial_text())
}

#[test]
fn blargg_cpu_instrs() {
    let dir = roms_dir().join("cpu_instrs/individual");
    if !dir.exists() {
        eprintln!("skipping Blargg cpu_instrs: submodule not checked out");
        return;
    }
    // Each individual suite finishes within a few seconds of emulated time.
    let cases = [
        "01-special.gb",
        "02-interrupts.gb",
        "03-op sp,hl.gb",
        "04-op r,imm.gb",
        "05-op rp.gb",
        "06-ld r,r.gb",
        "07-jr,jp,call,ret,rst.gb",
        "08-misc instrs.gb",
        "09-op r,r.gb",
        "10-bit ops.gb",
        "11-op a,(hl).gb",
    ];
    let mut failures = Vec::new();
    for name in cases {
        match run_serial(&dir.join(name), 4000) {
            Ok(text) if text.contains("Passed") => {}
            Ok(text) => failures.push(format!("{name}: {}", text.trim().replace('\n', " "))),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "cpu_instrs failures:\n{}", failures.join("\n"));
}

#[test]
fn blargg_instr_timing() {
    let path = roms_dir().join("instr_timing/instr_timing.gb");
    if !path.exists() {
        eprintln!("skipping Blargg instr_timing: submodule not checked out");
        return;
    }
    let text = run_serial(&path, 2000).expect("run instr_timing");
    assert!(
        text.contains("Passed"),
        "instr_timing did not pass: {}",
        text.trim().replace('\n', " ")
    );
}
