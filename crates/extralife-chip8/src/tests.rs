//! Tests for the CHIP-8 core: direct opcode/quirk checks (precise, fast) plus
//! test-suite ROM smoke checks. The golden-image comparison for the visual
//! ROMs lives in `tests/golden.rs` at the crate root.
#![allow(clippy::field_reassign_with_default)] // test setup: mutate a default machine, clearer than struct literals over private fields

use super::*;

/// Build a machine with a program loaded at 0x200 and run `cycles` instructions.
fn run(program: &[u8], cycles: u32) -> Chip8 {
    let mut c = Chip8::default();
    c.load_rom(program).unwrap();
    // step per-cycle by faking a 1-cycle frame budget is awkward; drive raw.
    for _ in 0..cycles {
        let op = c.fetch();
        c.execute(op);
    }
    c
}

#[test]
fn contract_framebuffer_and_state_roundtrip() {
    let mut c = Chip8::default();
    assert!(c.load_rom(&[0x00, 0xE0]).is_ok());
    assert_eq!(c.framebuffer().len(), SCREEN.framebuffer_len());
    assert!(c.load_rom(&[]).is_err());

    // Mutate, snapshot, mutate more, restore.
    c.v[3] = 42;
    c.i = 0x321;
    let snap = c.save_state();
    c.v[3] = 0;
    c.i = 0;
    assert!(c.load_state(&snap).is_ok());
    assert_eq!(c.v[3], 42);
    assert_eq!(c.i, 0x321);
    assert!(c.load_state(&[1, 2, 3]).is_err());
}

#[test]
fn arithmetic_carry_and_borrow_flags() {
    // 8xy4 add with carry: 0xFF + 0x01 = 0x00, vF=1.
    let mut c = Chip8::default();
    c.v[0] = 0xFF;
    c.v[1] = 0x01;
    c.execute(0x8014);
    assert_eq!(c.v[0], 0x00);
    assert_eq!(c.v[0xF], 1);

    // 8xy5 sub with borrow: 0x01 - 0x02 => vF=0 (borrow occurred).
    c.v[0] = 0x01;
    c.v[1] = 0x02;
    c.execute(0x8015);
    assert_eq!(c.v[0], 0xFF);
    assert_eq!(c.v[0xF], 0);
}

#[test]
fn vf_reset_quirk_on_logical_ops() {
    let mut c = Chip8::default();
    c.v[0] = 0b1010;
    c.v[1] = 0b0110;
    c.v[0xF] = 1; // dirty
    c.execute(0x8011); // OR
    assert_eq!(c.v[0], 0b1110);
    assert_eq!(c.v[0xF], 0, "logical ops must reset vF (Cosmac VIP quirk)");
}

#[test]
fn shift_quirk_reads_vy() {
    let mut c = Chip8::default();
    c.v[0] = 0x00;
    c.v[1] = 0b0000_0011;
    c.execute(0x8016); // 8xy6: vX = vY >> 1
    assert_eq!(c.v[0], 0b0000_0001);
    assert_eq!(c.v[0xF], 1, "shifted-out bit goes to vF");
}

#[test]
fn memory_load_store_increments_i() {
    let mut c = Chip8::default();
    c.i = 0x300;
    c.v[0] = 10;
    c.v[1] = 20;
    c.v[2] = 30;
    c.execute(0xF255); // store v0..=v2
    assert_eq!(c.i, 0x303, "Fx55 must increment I (memory quirk)");
    assert_eq!(c.mem[0x300], 10);
    assert_eq!(c.mem[0x302], 30);

    c.i = 0x300;
    c.v = [0; 16];
    c.execute(0xF265); // load v0..=v2
    assert_eq!(c.i, 0x303);
    assert_eq!(c.v[1], 20);
}

#[test]
fn bcd_splits_digits() {
    let mut c = Chip8::default();
    c.i = 0x400;
    c.v[0] = 123;
    c.execute(0xF033);
    assert_eq!([c.mem[0x400], c.mem[0x401], c.mem[0x402]], [1, 2, 3]);
}

#[test]
fn call_and_return() {
    // 0x200: CALL 0x206 ; 0x202: JP 0x202 (spin) ; 0x206: RET
    let prog = [0x22, 0x06, 0x12, 0x02, 0x00, 0x00, 0x00, 0xEE];
    let mut c = Chip8::default();
    c.load_rom(&prog).unwrap();
    let op = c.fetch();
    c.execute(op); // CALL
    assert_eq!(c.pc, 0x206);
    assert_eq!(c.stack.len(), 1);
    let op = c.fetch();
    c.execute(op); // RET
    assert_eq!(c.pc, 0x202);
    assert!(c.stack.is_empty());
}

#[test]
fn skip_instructions() {
    let mut c = run(&[0x60, 0x05, 0x30, 0x05], 2); // LD v0,5 ; SE v0,5
    assert_eq!(c.pc, 0x200 + 6, "SE with equal must skip one instruction");
    let _ = &mut c;
}

#[test]
fn keypad_skip_opcodes() {
    let mut c = Chip8::default();
    c.v[0] = 0xA;
    c.keys[0xA] = true;
    let pc = c.pc;
    c.execute(0xE09E); // SKP v0 -> key down -> skip
    assert_eq!(c.pc, pc + 2);

    c.keys[0xA] = false;
    let pc = c.pc;
    c.execute(0xE09E); // key up -> no skip
    assert_eq!(c.pc, pc);
}

/// End-to-end: the IBM logo still renders after the rewrite.
#[test]
fn ibm_logo_renders() {
    let rom = include_bytes!("../../../tests/roms/chip8-test-suite/bin/2-ibm-logo.ch8");
    let mut c = Chip8::default();
    c.load_rom(rom).unwrap();
    for _ in 0..30 {
        c.step_frame();
    }
    let lit = c.framebuffer().chunks(4).filter(|p| p[0] > 0).count();
    assert!(lit > 100, "IBM logo should light many pixels, got {lit}");
}

/// Deterministic: same ROM run twice yields identical framebuffers.
#[test]
fn deterministic_render() {
    let rom = include_bytes!("../../../tests/roms/chip8-test-suite/bin/3-corax+.ch8");
    let mut a = Chip8::default();
    let mut b = Chip8::default();
    a.load_rom(rom).unwrap();
    b.load_rom(rom).unwrap();
    for _ in 0..120 {
        a.step_frame();
        b.step_frame();
    }
    assert_eq!(a.framebuffer(), b.framebuffer());
}
