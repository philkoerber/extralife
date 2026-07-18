//! Fast, targeted unit tests for the NES core.
//!
//! Exhaustive CPU validation is the SingleStepTests runner in `tests/nes6502.rs`
//! plus the nestest log diff; these are cheap per-opcode sanity checks against a
//! flat memory, kept fast so they run on every `cargo test`.
#![allow(clippy::field_reassign_with_default)]

use crate::cpu::{Bus, Cpu};

struct FlatBus {
    mem: [u8; 0x10000],
}
impl Bus for FlatBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
}

fn run(program: &[u8]) -> (Cpu, FlatBus) {
    let mut bus = FlatBus { mem: [0; 0x10000] };
    bus.mem[..program.len()].copy_from_slice(program);
    let mut cpu = Cpu::default();
    cpu.pc = 0;
    (cpu, bus)
}

#[test]
fn lda_imm_sets_zero_flag() {
    // LDA #$00 -> A=0, Z set, N clear.
    let (mut cpu, mut bus) = run(&[0xA9, 0x00]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0);
    assert!(cpu.p & 0x02 != 0, "Z must be set");
    assert!(cpu.p & 0x80 == 0, "N must be clear");
}

#[test]
fn adc_binary_no_decimal() {
    // Decimal mode is disabled on the 2A03: SED then ADC still computes binary.
    // SED ; LDA #$09 ; ADC #$09 -> 0x12 (binary), NOT 0x18 (BCD).
    let (mut cpu, mut bus) = run(&[0xF8, 0xA9, 0x09, 0x69, 0x09]);
    cpu.step(&mut bus); // SED
    cpu.step(&mut bus); // LDA
    cpu.step(&mut bus); // ADC
    assert_eq!(cpu.a, 0x12, "2A03 ignores decimal mode");
}

#[test]
fn adc_overflow_flag() {
    // 0x50 + 0x50 = 0xA0: signed overflow (positive + positive -> negative).
    let (mut cpu, mut bus) = run(&[0xA9, 0x50, 0x69, 0x50]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xA0);
    assert!(cpu.p & 0x40 != 0, "V must be set");
}

#[test]
fn jsr_rts_roundtrip() {
    // JSR $0005 ; (pad) ; at $0005: RTS. After RTS, PC returns past the JSR.
    let mut prog = vec![0x20, 0x05, 0x00, 0xEA, 0xEA, 0x60];
    prog.resize(8, 0);
    let (mut cpu, mut bus) = run(&prog);
    cpu.step(&mut bus); // JSR
    assert_eq!(cpu.pc, 0x0005);
    cpu.step(&mut bus); // RTS
    assert_eq!(cpu.pc, 0x0003, "RTS returns to the byte after the JSR operand");
}

#[test]
fn stack_push_pull() {
    // LDA #$AB ; PHA ; LDA #$00 ; PLA -> A back to 0xAB.
    let (mut cpu, mut bus) = run(&[0xA9, 0xAB, 0x48, 0xA9, 0x00, 0x68]);
    for _ in 0..4 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.a, 0xAB);
}
