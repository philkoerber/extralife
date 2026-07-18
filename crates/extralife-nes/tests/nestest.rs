//! nestest CPU integration test: run Kevin Horton's nestest ROM in "automation
//! mode" (entry at $C000) and diff our per-instruction CPU trace against the
//! canonical `nestest.log`.
//!
//! The log records, before each instruction, PC / A / X / Y / P / SP and the
//! total CPU cycle count (`CYC:`). Matching it line-for-line validates the CPU
//! *and* the system's cycle accounting against a reference the whole community
//! trusts (Nintendulator's output). We compare PC, the registers, P, SP and
//! CYC; the disassembly and PPU dot columns are informational.
//!
//! Both the ROM and the log ship in the committed `nes-test-roms` submodule at
//! `tests/roms/nes-test-roms/other/`. If absent the test is skipped.

use extralife_core::Device;
use extralife_nes::Nes;
use std::path::PathBuf;

struct LogLine {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    sp: u8,
    cyc: u64,
}

fn parse_hex_u8(s: &str) -> u8 {
    u8::from_str_radix(s, 16).unwrap()
}

/// Parse the fixed-column nestest log format. Fields are located by their
/// labels (`A:`, `X:`, ... `CYC:`) so column drift in the disassembly is fine.
fn parse_line(line: &str) -> Option<LogLine> {
    if line.len() < 4 {
        return None;
    }
    let pc = u16::from_str_radix(&line[0..4], 16).ok()?;
    let field = |label: &str| -> Option<&str> {
        let idx = line.find(label)? + label.len();
        Some(line[idx..].split_whitespace().next()?)
    };
    Some(LogLine {
        pc,
        a: parse_hex_u8(field("A:")?),
        x: parse_hex_u8(field("X:")?),
        y: parse_hex_u8(field("Y:")?),
        p: parse_hex_u8(field("P:")?),
        sp: parse_hex_u8(field("SP:")?),
        cyc: field("CYC:")?.parse().ok()?,
    })
}

fn roms_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/roms/nes-test-roms/other")
}

#[test]
fn nestest_log_matches() {
    let dir = roms_dir();
    let rom = match std::fs::read(dir.join("nestest.nes")) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("skipping nestest: nes-test-roms submodule not checked out");
            return;
        }
    };
    let log = std::fs::read_to_string(dir.join("nestest.log")).expect("nestest.log");

    let mut nes = Nes::default();
    nes.load_rom(&rom).expect("load nestest");
    // Automation mode: start at $C000; the reset put PC at the vector's target.
    nes.set_pc(0xC000);

    let lines: Vec<LogLine> = log.lines().filter_map(parse_line).collect();
    assert!(!lines.is_empty(), "nestest.log parsed empty");

    for (i, expected) in lines.iter().enumerate() {
        let cpu = nes.cpu();
        let got_cyc = nes.cycles() + 7; // reset consumes 7 cycles before $C000
        let mismatch = cpu.pc != expected.pc
            || cpu.a != expected.a
            || cpu.x != expected.x
            || cpu.y != expected.y
            || cpu.p != expected.p
            || cpu.sp != expected.sp
            || got_cyc != expected.cyc;
        if mismatch {
            panic!(
                "nestest diverged at log line {} (instruction {}):\n  \
                 got  PC:{:04X} A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}\n  \
                 want PC:{:04X} A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}",
                i + 1,
                i,
                cpu.pc, cpu.a, cpu.x, cpu.y, cpu.p, cpu.sp, got_cyc,
                expected.pc, expected.a, expected.x, expected.y, expected.p, expected.sp, expected.cyc,
            );
        }
        nes.step_instruction();
    }

    // nestest writes its result codes to $02 (and $03); 0 means all passed.
    let r02 = nes.peek(0x02);
    let r03 = nes.peek(0x03);
    assert_eq!(r02, 0, "nestest official-opcode result byte $02 = {r02:#x} (nonzero = failure)");
    assert_eq!(r03, 0, "nestest illegal-opcode result byte $03 = {r03:#x}");
    eprintln!("nestest: {} instructions matched the reference log", lines.len());
}
