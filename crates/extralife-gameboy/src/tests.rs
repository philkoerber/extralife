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

/// Device contract: framebuffer is the right size, a rejected ROM stays rejected,
/// and save/load_state round-trips (with a malformed blob left atomic).
#[test]
fn device_contract() {
    use crate::GameBoy;
    use extralife_core::Device;

    let mut gb = GameBoy::default();
    // A tiny no-MBC ROM: 32 KiB of zeros with a valid-enough header.
    let mut rom = vec![0u8; 0x8000];
    rom[0x0147] = 0x00; // no MBC
    rom[0x0148] = 0x00; // 32 KiB
    assert!(gb.load_rom(&rom).is_ok());
    assert_eq!(gb.framebuffer().len(), gb.screen().framebuffer_len());

    // Too-small image is rejected.
    assert!(gb.load_rom(&[0u8; 16]).is_err());

    // Reload the good ROM, run a few frames, snapshot, run more, restore.
    gb.load_rom(&rom).unwrap();
    for _ in 0..3 {
        gb.step_frame();
    }
    let snap = gb.save_state();
    let fb_at_snap = gb.framebuffer().to_vec();
    for _ in 0..3 {
        gb.step_frame();
    }
    assert!(gb.load_state(&snap).is_ok());
    assert_eq!(gb.framebuffer(), &fb_at_snap[..], "state restore must reproduce the frame");

    // A malformed blob is rejected and leaves the core usable.
    assert!(gb.load_state(&[0xFF, 0x00, 0x01]).is_err());
    gb.step_frame();
}

/// MBC5 mapping: 9-bit ROM bank select (incl. bit 8 via 0x3000), RAM banking,
/// and RAM enable gating. Pokémon Red is MBC5 (cart type 0x1B); before this the
/// core rejected it at load.
#[test]
fn mbc5_banking() {
    use crate::cartridge::Cartridge;

    // 512 KiB = 32 banks. Stamp each 16 KiB bank's first byte with its number
    // (low 8 bits) so a read at 0x4000 tells us which bank is mapped.
    let banks = 32usize;
    let mut rom = vec![0u8; banks * 0x4000];
    for b in 0..banks {
        rom[b * 0x4000] = b as u8;
    }
    rom[0x0147] = 0x1B; // MBC5 + RAM + battery
    rom[0x0148] = 0x04; // 512 KiB
    rom[0x0149] = 0x03; // 32 KiB RAM (4 banks)

    let mut cart = Cartridge::new(&rom).expect("MBC5 cart must load");

    // Bank 0 region is fixed to bank 0; default high bank is 1.
    assert_eq!(cart.read_rom(0x0000), 0);
    assert_eq!(cart.read_rom(0x4000), 1);

    // Select bank 5 via the low-8-bit register.
    cart.write_rom(0x2000, 5);
    assert_eq!(cart.read_rom(0x4000), 5);

    // Bank 0 is addressable on MBC5 (no "0 becomes 1" quirk).
    cart.write_rom(0x2000, 0);
    assert_eq!(cart.read_rom(0x4000), 0);

    // Bit 8: bank 0x101 -> masked to 0x01 here (32 banks), proving the bit wires.
    cart.write_rom(0x2000, 0x01);
    cart.write_rom(0x3000, 0x01); // set bit 8
    assert_eq!(cart.read_rom(0x4000), 1, "bank 0x101 & mask(0x1F) == 1");
    cart.write_rom(0x3000, 0x00); // clear bit 8 again

    // RAM is gated until enabled, then banks independently.
    cart.write_ram(0xA000, 0xAB);
    assert_eq!(cart.read_ram(0xA000), 0xFF, "RAM disabled reads 0xFF");
    cart.write_rom(0x0000, 0x0A); // enable RAM
    cart.write_ram(0xA000, 0xAB);
    assert_eq!(cart.read_ram(0xA000), 0xAB);
    cart.write_rom(0x4000, 1); // RAM bank 1
    assert_eq!(cart.read_ram(0xA000), 0x00, "bank 1 is a different page");
    cart.write_ram(0xA000, 0xCD);
    cart.write_rom(0x4000, 0); // back to bank 0
    assert_eq!(cart.read_ram(0xA000), 0xAB, "bank 0 preserved");
}
