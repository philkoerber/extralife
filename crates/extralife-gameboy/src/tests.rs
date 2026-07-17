//! Unit tests for the Game Boy core.
//!
//! The exhaustive CPU validation is the SingleStepTests runner in
//! `tests/sm83.rs`; these are fast, targeted checks (a few opcodes wired
//! against a flat memory) plus — once the system is assembled — the `Device`
//! contract round-trip. Keep them cheap so they run on every `cargo test`.
#![allow(clippy::field_reassign_with_default)] // test setup: mutate a default CPU, clearer than struct literals

use crate::cpu::{Bus, Cpu};

/// Minimal flat-memory bus for driving the CPU in isolation.
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
    fn tick(&mut self) {}
    fn pending_interrupts(&self) -> u8 {
        0
    }
    fn ack_interrupt(&mut self, _bit: u8) {}
}

fn run(program: &[u8]) -> (Cpu, FlatBus) {
    let mut bus = FlatBus { mem: [0; 0x10000] };
    bus.mem[..program.len()].copy_from_slice(program);
    let mut cpu = Cpu::default();
    cpu.pc = 0;
    (cpu, bus)
}

#[test]
fn ld_and_add_registers() {
    // LD B,0x12 ; LD A,0x34 ; ADD A,B  -> A = 0x46
    let (mut cpu, mut bus) = run(&[0x06, 0x12, 0x3E, 0x34, 0x80]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x46);
    assert_eq!(cpu.b, 0x12);
}

#[test]
fn add_half_carry_flag() {
    // A=0x0F + 0x01 sets H (carry out of bit 3).
    let (mut cpu, mut bus) = run(&[0x3E, 0x0F, 0xC6, 0x01]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x10);
    assert_eq!(cpu.f & 0x20, 0x20, "half-carry must be set");
}

#[test]
fn push_pop_roundtrip() {
    // LD BC,0xBEEF ; PUSH BC ; POP DE
    let (mut cpu, mut bus) = run(&[0x01, 0xEF, 0xBE, 0xC5, 0xD1]);
    cpu.sp = 0xFFFE;
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.d, 0xBE);
    assert_eq!(cpu.e, 0xEF);
}
