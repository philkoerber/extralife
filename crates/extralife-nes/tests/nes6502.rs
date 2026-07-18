//! SingleStepTests/nes6502 runner — the 2A03 CPU's definition of done.
//!
//! Each opcode file (`nes6502/v1/<hex>.json`) holds thousands of cases. A case
//! sets the CPU + a flat 64 KiB memory to `initial`, executes exactly one
//! instruction, then asserts the `final` CPU state, memory, and the exact
//! ordered list of bus cycles `[address, value, "read"|"write"]`.
//!
//! On the NMOS 6502 every cycle is a bus access, so our CPU's `Bus::read`/
//! `Bus::write` map one-to-one onto the suite's cycle list — we assert both the
//! count and each (addr, value, kind).
//!
//! The suite is a git submodule at `tests/roms/ProcessorTests` (sparse-checked
//! to `nes6502/v1`). If absent the test is skipped (CI checks out submodules).

use extralife_nes::cpu::{Bus, Cpu};
use serde::Deserialize;
use std::path::PathBuf;

struct TestBus {
    mem: [u8; 0x10000],
    cycles: Vec<(u16, u8, &'static str)>,
}
impl TestBus {
    fn new() -> Self {
        TestBus { mem: [0; 0x10000], cycles: Vec::new() }
    }
}
impl Bus for TestBus {
    fn read(&mut self, addr: u16) -> u8 {
        let v = self.mem[addr as usize];
        self.cycles.push((addr, v, "read"));
        v
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
        self.cycles.push((addr, val, "write"));
    }
}

#[derive(Deserialize)]
struct State {
    pc: u16,
    s: u8,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    ram: Vec<(u16, u8)>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    initial: State,
    #[serde(rename = "final")]
    final_: State,
    cycles: Vec<(u16, u8, String)>,
}

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/roms/ProcessorTests/nes6502/v1")
}

fn setup(state: &State) -> (Cpu, TestBus) {
    let cpu = Cpu {
        a: state.a,
        x: state.x,
        y: state.y,
        sp: state.s,
        pc: state.pc,
        p: state.p,
    };
    let mut bus = TestBus::new();
    for &(addr, val) in &state.ram {
        bus.mem[addr as usize] = val;
    }
    (cpu, bus)
}

fn check_regs(cpu: &Cpu, f: &State) -> Option<String> {
    let mut errs = Vec::new();
    macro_rules! chk {
        ($field:ident, $exp:expr, $label:expr) => {
            if cpu.$field != $exp {
                errs.push(format!("{}: got {:#x}, want {:#x}", $label, cpu.$field, $exp));
            }
        };
    }
    chk!(a, f.a, "a");
    chk!(x, f.x, "x");
    chk!(y, f.y, "y");
    chk!(sp, f.s, "s");
    chk!(pc, f.pc, "pc");
    chk!(p, f.p, "p");
    if errs.is_empty() { None } else { Some(errs.join(", ")) }
}

fn check_cycles(bus: &TestBus, expected: &[(u16, u8, String)]) -> Option<String> {
    if bus.cycles.len() != expected.len() {
        return Some(format!(
            "cycle count: got {}, want {} (got {:?})",
            bus.cycles.len(),
            expected.len(),
            bus.cycles
        ));
    }
    for (i, ((ga, gv, gk), (wa, wv, wk))) in bus.cycles.iter().zip(expected).enumerate() {
        if ga != wa || gv != wv || gk != wk {
            return Some(format!(
                "cycle {i}: got ({ga:#x},{gv:#x},{gk}), want ({wa:#x},{wv:#x},{wk})"
            ));
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
                "skipping nes6502 SingleStepTests: submodule not checked out at {}",
                dir.display()
            );
            return;
        }
    };
    entries.sort();
    assert!(!entries.is_empty(), "no nes6502 test files found in {}", dir.display());

    let mut total = 0usize;
    let mut failures = Vec::new();
    for path in &entries {
        match run_file(path) {
            Ok(n) => total += n,
            Err(e) => failures.push(format!("{}: {e}", path.file_name().unwrap().to_string_lossy())),
        }
        if failures.len() >= 30 {
            break;
        }
    }

    assert!(
        failures.is_empty(),
        "{} nes6502 cases checked; {} failing opcode files (first errors):\n{}",
        total,
        failures.len(),
        failures.join("\n")
    );
    eprintln!("nes6502 SingleStepTests: {total} cases across {} files passed", entries.len());
}
