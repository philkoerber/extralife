//! Per-instruction-class CPU tests derived from the Epson S1C6200/6200A Core
//! CPU Manual §3.5 worked examples, plus system-level sanity checks. These are
//! the "definition of done" for the headless CPU before any pixels exist.

use crate::cpu::{Bus, Cpu, C_FLAG, D_FLAG, I_FLAG, Z_FLAG};

/// Flat 4KB RAM bus for isolated CPU tests (no I/O special-casing).
struct RamBus {
    mem: [u8; 0x1000],
}

impl RamBus {
    fn new() -> RamBus {
        RamBus { mem: [0; 0x1000] }
    }
}

impl Bus for RamBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[(addr & 0xFFF) as usize] & 0xF
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[(addr & 0xFFF) as usize] = val & 0xF;
    }
}

/// Run a single opcode from a fresh CPU parked at bank0/page1/step0.
fn run1(cpu: &mut Cpu, bus: &mut RamBus, rom: &[u16]) -> u32 {
    // Place the program at the reset PC (bank0, page1, step0 => index 0x100).
    let mut full = vec![0u16; 0x2000];
    for (i, &w) in rom.iter().enumerate() {
        full[0x100 + i] = w;
    }
    cpu.step(&full, bus)
}

#[test]
fn ld_r_i_loads_immediate() {
    // LD A,6  => E00 | (r=A=00)<<2 | i=6  = 0xE06
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    run1(&mut cpu, &mut bus, &[0xE06]);
    assert_eq!(cpu.a, 6);
}

#[test]
fn add_sets_carry_and_zero() {
    // LD A,0xF (E0F) then ADD A,1 (C00|A<<... ) : ADD r,i = 110000 r i => C00|r<<4? 
    // Encoding: 110000 r1 r0 i3 i2 i1 i0 => base 0xC00, r in bits[5:4], i in [3:0].
    // ADD A,1: r=A=0 => 0xC01.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    cpu.a = 0xF;
    // Manually execute ADD A,1 via one-op ROM.
    run1(&mut cpu, &mut bus, &[0xC01]);
    assert_eq!(cpu.a, 0x0, "0xF + 1 wraps to 0");
    assert_ne!(cpu.flags & C_FLAG, 0, "carry set on overflow");
    assert_ne!(cpu.flags & Z_FLAG, 0, "zero set on 0 result");
}

#[test]
fn adc_uses_carry_in() {
    // ADC A,0 with C=1 => A becomes A+1.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    cpu.a = 3;
    cpu.flags |= C_FLAG;
    run1(&mut cpu, &mut bus, &[0xC40]); // ADC r,i base 0xC40, r=A=0, i=0
    assert_eq!(cpu.a, 4);
}

#[test]
fn decimal_adjust_on_add() {
    // With D flag set, ADD 5+5 = 10 => BCD-adjusts to 0 with carry.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    cpu.a = 5;
    cpu.flags |= D_FLAG;
    run1(&mut cpu, &mut bus, &[0xC05]); // ADD A,5
    assert_eq!(cpu.a, 0, "5+5 decimal = 10 -> low digit 0");
    assert_ne!(cpu.flags & C_FLAG, 0, "decimal carry out");
}

#[test]
fn sub_rq_sets_borrow() {
    // SUB A,B (r=A=0, q=B=1) base 0xAA0 => 0xAA1. A=0, B=1 => borrow, result 0xF.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    cpu.a = 0;
    cpu.b = 1;
    run1(&mut cpu, &mut bus, &[0xAA1]);
    assert_eq!(cpu.a, 0xF, "0 - 1 = 0xF (4-bit)");
    assert_ne!(cpu.flags & C_FLAG, 0, "borrow sets C");
}

#[test]
fn cp_sets_flags_without_writing() {
    // CP A,4 with A=4 => Z set, C clear, A unchanged. CP r,i base 0xDC0.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    cpu.a = 4;
    run1(&mut cpu, &mut bus, &[0xDC4]); // CP A,4
    assert_eq!(cpu.a, 4, "CP does not write");
    assert_ne!(cpu.flags & Z_FLAG, 0, "equal => zero");
    assert_eq!(cpu.flags & C_FLAG, 0, "no borrow when equal");
}

#[test]
fn pset_then_jp_crosses_page() {
    // PSET 5 (E40|p=5 => 0xE45) then JP 0x10 (0x010). Should land page5 step0x10.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    let mut full = vec![0u16; 0x2000];
    full[0x100] = 0xE45; // PSET 5
    full[0x101] = 0x010; // JP 0x10
    cpu.step(&full, &mut bus); // PSET
    cpu.step(&full, &mut bus); // JP
    assert_eq!(cpu.pcp, 5, "page from PSET");
    assert_eq!(cpu.pcs, 0x10, "step from JP");
}

#[test]
fn call_ret_round_trip() {
    // At page1/step0: PSET1 ; CALL 0x20 ; ... at page1/step0x20: RET.
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    let mut full = vec![0u16; 0x2000];
    full[0x100] = 0xE41; // PSET 1
    full[0x101] = 0x420; // CALL 0x20
    full[0x120] = 0xFDF; // RET
    cpu.step(&full, &mut bus); // PSET
    cpu.step(&full, &mut bus); // CALL -> step 0x20
    assert_eq!(cpu.pcs, 0x20, "called into 0x20");
    cpu.step(&full, &mut bus); // RET -> back to 0x02
    assert_eq!(cpu.pcs, 0x02, "returned past the CALL");
}

#[test]
fn interrupt_disables_and_vectors() {
    let mut cpu = Cpu::new();
    let mut bus = RamBus::new();
    cpu.sp = 0x80;
    cpu.flags |= I_FLAG;
    cpu.interrupt(&mut bus, 1, 0x0C);
    assert_eq!(cpu.flags & I_FLAG, 0, "I cleared on entry");
    assert_eq!(cpu.pcp, 1);
    assert_eq!(cpu.pcs, 0x0C);
}

// --- System-level: full seam CPU -> display RAM -> framebuffer ------------

/// Pack 12-bit words into the P1 ROM byte format: 16-bit big-endian per word.
/// The reset vector sits at bank0/page1/step0 == word index 0x100, so we place
/// the program there.
fn assemble(words_at_0x100: &[u16]) -> Vec<u8> {
    let mut rom = vec![0u16; 6144];
    for (i, &w) in words_at_0x100.iter().enumerate() {
        rom[0x100 + i] = w & 0x0FFF;
    }
    let mut bytes = Vec::with_capacity(rom.len() * 2);
    for w in rom {
        bytes.push((w >> 8) as u8);
        bytes.push((w & 0xFF) as u8);
    }
    bytes
}

#[test]
fn system_draws_pixel_to_framebuffer() {
    use crate::Tamagotchi;
    use extralife_core::Device;

    // Program: point IX at display-RAM SEG0/COM0 (0xE00), write bit0 -> pixel
    // (x=0,y=0) lit, then spin. Clean-room hand assembly of documented opcodes.
    //   LD X,0x00   ; XH=0, XL=0                          -> 0xB00
    //   LD A,0x0E   ; A = 0xE                             -> 0xE0E
    //   LD XP,A     ; XP = 0xE  => IX = 0xE00             -> 0xE80
    //   LDPX MX,1   ; M(0xE00) <- 1 (SEG0 COM0..3 = 0001) -> 0xE61
    //   JP  0x04    ; spin in place (step 0x04)           -> 0x004
    let rom = assemble(&[0xB00, 0xE0E, 0xE80, 0xE61, 0x004]);

    let mut tama = Tamagotchi::new();
    tama.load_rom(&rom).expect("valid rom");
    // One frame runs far more than 5 instructions, so the pixel is set.
    tama.step_frame();

    let fb = tama.framebuffer();
    // Pixel (0,0): dark (lit) => RGB 0x00. Byte index 0.
    assert_eq!(fb[0], 0x00, "pixel (0,0) should be lit (dark on LCD)");
    assert_eq!(fb[3], 0xFF, "alpha opaque");
    // A neighbouring off pixel (x=1,y=0) should be the grey background.
    let off = 4; // (row 0, col 1) * 4 bytes/px
    assert_eq!(fb[off], 0xC8, "pixel (1,0) unlit background");
}

#[test]
fn save_state_round_trips() {
    use crate::Tamagotchi;
    use extralife_core::Device;
    let rom = assemble(&[0xE0E, 0xE80, 0x001]);
    let mut a = Tamagotchi::new();
    a.load_rom(&rom).unwrap();
    a.step_frame();
    let snap = a.save_state();
    let mut b = Tamagotchi::new();
    b.load_rom(&rom).unwrap();
    b.load_state(&snap).expect("round trip");
    assert_eq!(a.framebuffer(), b.framebuffer());
    assert_eq!(a.save_state(), b.save_state());
}
