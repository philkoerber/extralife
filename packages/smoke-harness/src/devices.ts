/**
 * The device registry — the one place you touch to add a console to the
 * smoke harness. Each entry says how to instantiate that device's WASM core
 * and which test ROMs to offer in the dropdown. Everything else in the harness
 * is device-agnostic and reads only from here.
 *
 * To add a new device (next build session):
 *   1. wasm-pack build its crate to `crates/extralife-<device>/pkg`.
 *   2. Add a Vite alias `@<device>-core` in vite.config.ts.
 *   3. Add one `DeviceEntry` below with an `init()` and its test ROMs.
 */

/** A live core instance. Matches the wasm-bindgen `Core` class every crate exports. */
export interface CoreInstance {
  readonly width: number;
  readonly height: number;
  loadRom(rom: Uint8Array): void;
  stepFrame(): void;
  setButton(button: number, pressed: boolean): void;
  framebuffer(): Uint8Array;
}

export interface RomEntry {
  label: string;
  /** Vite `?url` asset URL for the ROM binary. */
  url: string;
}

export interface DeviceEntry {
  id: string;
  label: string;
  /** How many core steps per animation frame. */
  frameHz?: number;
  /** Load the WASM module and return a fresh core instance. */
  init(): Promise<CoreInstance>;
  roms: RomEntry[];
}

// --- CHIP-8 ---------------------------------------------------------------

import ibmLogo from "../../../tests/roms/chip8-test-suite/bin/2-ibm-logo.ch8?url";
import chip8Logo from "../../../tests/roms/chip8-test-suite/bin/1-chip8-logo.ch8?url";
import corax from "../../../tests/roms/chip8-test-suite/bin/3-corax+.ch8?url";
import flags from "../../../tests/roms/chip8-test-suite/bin/4-flags.ch8?url";
import quirks from "../../../tests/roms/chip8-test-suite/bin/5-quirks.ch8?url";

const chip8: DeviceEntry = {
  id: "chip8",
  label: "CHIP-8",
  async init() {
    const mod = await import("@chip8-core/extralife_chip8.js");
    await mod.default();
    return new mod.Core();
  },
  roms: [
    { label: "IBM logo", url: ibmLogo },
    { label: "CHIP-8 logo", url: chip8Logo },
    { label: "Corax+ opcode test", url: corax },
    { label: "Flags test", url: flags },
    { label: "Quirks test", url: quirks },
  ],
};

// --- Game Boy (DMG) -------------------------------------------------------

// dmg-acid2 is built into the gitignored submodule build dir (see the golden
// test); cpu_instrs ships in the committed gb-test-roms submodule so it loads
// on any fresh clone.
import dmgAcid2 from "../../../tests/roms/dmg-acid2/build/dmg-acid2.gb?url";
import cpuInstrs from "../../../tests/roms/gb-test-roms/cpu_instrs/cpu_instrs.gb?url";

const gameboy: DeviceEntry = {
  id: "gameboy",
  label: "Game Boy (DMG)",
  async init() {
    const mod = await import("@gameboy-core/extralife_gameboy.js");
    await mod.default();
    return new mod.Core();
  },
  roms: [
    { label: "dmg-acid2", url: dmgAcid2 },
    { label: "Blargg cpu_instrs", url: cpuInstrs },
  ],
};

export const DEVICES: DeviceEntry[] = [chip8, gameboy];
