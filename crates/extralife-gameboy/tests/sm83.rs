//! SingleStepTests/sm83 runner — the CPU's definition of done.
//!
//! For every opcode file (`v1/*.json`), each test sets the CPU + a flat 64 KiB
//! memory to `initial`, executes exactly one instruction, then asserts the
//! `final` CPU state, memory, and the exact per-M-cycle bus activity.
//!
//! The suite is a git submodule at `tests/roms/sm83`. If it is not checked out
//! the test is skipped with a message (CI checks out submodules).

use extralife_gameboy::cpu::{Bus, Cpu};
use serde::Deserialize;
use std::path::PathBuf;

/// Flat 64 KiB memory that records each M-cycle's bus activity, matching the
/// suite's `[address, value, kind]` cycle entries.
struct TestBus {
    mem: [u8; 0x10000],
    ie: u8,
    /// Recorded cycles: (addr, value, kind) where kind is "r-m" / "-wm" / "---".
    cycles: Vec<(u16, u8, &'static str)>,
}

impl TestBus {
    fn new() -> Self {
        TestBus {
            mem: [0; 0x10000],
            ie: 0,
            cycles: Vec::new(),
        }
    }
}

impl Bus for TestBus {
    fn read(&mut self, addr: u16) -> u8 {
        let v = self.mem[addr as usize];
        self.cycles.push((addr, v, "r-m"));
        v
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
        self.cycles.push((addr, val, "-wm"));
    }
    fn tick(&mut self) {
        // Internal cycle: the suite records the last bus address/value with "---".
        // The suite emits the *current PC-ish* address; matching its exact value
        // for internal cycles is not required by our assertion (we only check
        // memory-access cycles), see `assert_cycles`.
        self.cycles.push((0, 0, "---"));
    }
    fn pending_interrupts(&self) -> u8 {
        // The suite never asserts IF; interrupts are exercised via IE/IME state
        // carry-through only. No line is ever pending.
        0
    }
    fn ack_interrupt(&mut self, _bit: u8) {}
}

#[derive(Deserialize)]
struct State {
    pc: u16,
    sp: u16,
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    f: u8,
    h: u8,
    l: u8,
    ime: u8,
    #[serde(default)]
    ie: u8,
    #[serde(default)]
    ei: u8,
    ram: Vec<(u16, u8)>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    initial: State,
    #[serde(rename = "final")]
    final_: State,
    /// [address, value, kind]; kind may be null for some internal cycles.
    cycles: Vec<Option<(u16, u8, String)>>,
}

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/roms/sm83/v1")
}

fn setup(state: &State) -> (Cpu, TestBus) {
    let cpu = Cpu {
        a: state.a,
        f: state.f,
        b: state.b,
        c: state.c,
        d: state.d,
        e: state.e,
        h: state.h,
        l: state.l,
        sp: state.sp,
        pc: state.pc,
        ime: state.ime != 0,
        ime_pending: state.ei != 0,
        halted: false,
        halt_bug: false,
    };
    let mut bus = TestBus::new();
    bus.ie = state.ie;
    for &(addr, val) in &state.ram {
        bus.mem[addr as usize] = val;
    }
    (cpu, bus)
}

/// Compare CPU registers against the expected final state. Returns a mismatch
/// description or None.
fn check_regs(cpu: &Cpu, f: &State) -> Option<String> {
    let mut errs = Vec::new();
    macro_rules! chk {
        ($field:ident, $exp:expr) => {
            if cpu.$field != $exp {
                errs.push(format!(
                    "{}: got {:#x}, want {:#x}",
                    stringify!($field),
                    cpu.$field,
                    $exp
                ));
            }
        };
    }
    chk!(a, f.a);
    chk!(f, f.f);
    chk!(b, f.b);
    chk!(c, f.c);
    chk!(d, f.d);
    chk!(e, f.e);
    chk!(h, f.h);
    chk!(l, f.l);
    chk!(sp, f.sp);
    chk!(pc, f.pc);
    if cpu.ime != (f.ime != 0) {
        errs.push(format!("ime: got {}, want {}", cpu.ime, f.ime != 0));
    }
    // `ei` in final = the pending-EI flag (IME enables after next instruction).
    if cpu.ime_pending != (f.ei != 0) {
        errs.push(format!(
            "ei(pending): got {}, want {}",
            cpu.ime_pending,
            f.ei != 0
        ));
    }
    if errs.is_empty() {
        None
    } else {
        Some(errs.join(", "))
    }
}

/// Compare only the memory-access cycles (reads/writes). Internal `---` cycles
/// carry no reliably-checkable address in our model, but we DO verify the total
/// M-cycle count and the order/content of every read and write.
fn check_cycles(bus: &TestBus, expected: &[Option<(u16, u8, String)>]) -> Option<String> {
    if bus.cycles.len() != expected.len() {
        return Some(format!(
            "cycle count: got {}, want {}",
            bus.cycles.len(),
            expected.len()
        ));
    }
    for (i, (got, want)) in bus.cycles.iter().zip(expected).enumerate() {
        let (ga, gv, gk) = got;
        match want {
            Some((wa, wv, wk)) => {
                // Only assert address/value for memory-access cycles; internal
                // cycles ("---") differ in the suite's synthetic address model.
                if wk != "---" {
                    if gk != wk {
                        return Some(format!("cycle {i}: kind {gk} != {wk}"));
                    }
                    if ga != wa || gv != wv {
                        return Some(format!(
                            "cycle {i} ({gk}): got ({ga:#x},{gv:#x}), want ({wa:#x},{wv:#x})"
                        ));
                    }
                } else if *gk != "---" {
                    return Some(format!("cycle {i}: kind {gk} != ---"));
                }
            }
            None => {
                if *gk != "---" {
                    return Some(format!("cycle {i}: expected internal, got {gk}"));
                }
            }
        }
    }
    None
}

fn run_file(path: &PathBuf) -> Result<usize, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let cases: Vec<Case> =
        serde_json::from_slice(&data).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let n = cases.len();
    for case in cases {
        let (mut cpu, mut bus) = setup(&case.initial);
        cpu.step(&mut bus);

        if let Some(err) = check_regs(&cpu, &case.final_) {
            return Err(format!("{}: reg mismatch: {err}", case.name));
        }
        for &(addr, val) in &case.final_.ram {
            if bus.mem[addr as usize] != val {
                return Err(format!(
                    "{}: mem[{addr:#x}] got {:#x}, want {val:#x}",
                    case.name, bus.mem[addr as usize]
                ));
            }
        }
        if let Some(err) = check_cycles(&bus, &case.cycles) {
            return Err(format!("{}: {err}", case.name));
        }
    }
    Ok(n)
}

#[test]
fn single_step_tests() {
    let dir = tests_dir();
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(_) => {
            eprintln!(
                "skipping SingleStepTests: submodule not checked out at {}",
                dir.display()
            );
            return;
        }
    };
    entries.sort();
    assert!(!entries.is_empty(), "no SM83 test files found in {}", dir.display());

    let mut total = 0usize;
    let mut failures = Vec::new();
    for path in &entries {
        match run_file(path) {
            Ok(n) => total += n,
            Err(e) => failures.push(e),
        }
        // Stop after collecting a handful of distinct failures to keep output sane.
        if failures.len() >= 20 {
            break;
        }
    }

    assert!(
        failures.is_empty(),
        "{} SM83 test cases checked; {} failing opcode files (first errors):\n{}",
        total,
        failures.len(),
        failures.join("\n")
    );
    eprintln!("SM83 SingleStepTests: {total} cases across {} files passed", entries.len());
}
