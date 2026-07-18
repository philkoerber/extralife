/**
 * The harness ROM catalog — device labels + which test ROMs to offer per device.
 * Core loading and timing now live in the `extralife` library's registry; the
 * harness only picks a `device` id and a ROM and hands both to `<ExtraLife>`.
 *
 * To add ROMs for a device: add `?url` imports and an entry below. To add a whole
 * new device, register its core in `packages/extralife/src/registry.ts` and add a
 * Vite alias in vite.config.ts, then list its ROMs here.
 */

import type { DeviceId } from "extralife";

export interface RomEntry {
  label: string;
  /** Vite `?url` asset URL for the ROM binary. */
  url: string;
}

export interface DeviceEntry {
  id: DeviceId;
  label: string;
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
// dmg_sound 01-registers pokes the APU and plays short beeps — an audible core
// check for the Web Audio path. Ships in the committed gb-test-roms submodule.
import dmgSound01 from "../../../tests/roms/gb-test-roms/dmg_sound/rom_singles/01-registers.gb?url";
// Pokémon Red (MBC5) — a real commercial game to prove mapper + PPU end to end.
// Copyrighted: lives only in the gitignored tests/roms/pokemon-gb/, never
// committed. Remove this line if the ROM isn't present locally.
import pokemonRed from "../../../tests/roms/pokemon-gb/Pokemon - Rote Edition (Germany) (SGB Enhanced).gb?url";

const gameboy: DeviceEntry = {
  id: "gameboy",
  label: "Game Boy (DMG)",
  roms: [
    { label: "dmg-acid2", url: dmgAcid2 },
    { label: "Blargg cpu_instrs", url: cpuInstrs },
    { label: "dmg_sound 01 (audio)", url: dmgSound01 },
    { label: "Pokémon Red (MBC5)", url: pokemonRed },
  ],
};

// --- NES / Famicom --------------------------------------------------------

// Permissive test ROMs from the committed nes-test-roms submodule, so they load
// on any fresh clone. colorwin renders the golden background/text screen (the
// frozen golden image); nes15 is a BSD-2-Clause homebrew 15-puzzle that proves a
// real game screen renders; full_palette shows the master palette.
import nesColorwin from "../../../tests/roms/nes-test-roms/window5/colorwin_ntsc.nes?url";
import nes15 from "../../../tests/roms/nes-test-roms/nes15-1.0.0/nes15-NTSC.nes?url";
import nesPalette from "../../../tests/roms/nes-test-roms/full_palette/full_palette.nes?url";
// blargg read_joy3 test_buttons: prompts for a button and reacts to input —
// the interactive check that keyboard → controller wiring works in the browser.
import nesButtons from "../../../tests/roms/nes-test-roms/read_joy3/test_buttons.nes?url";

const nes: DeviceEntry = {
  id: "nes",
  label: "NES / Famicom",
  roms: [
    { label: "colorwin (background test)", url: nesColorwin },
    { label: "nes15 (homebrew puzzle)", url: nes15 },
    { label: "test_buttons (input demo)", url: nesButtons },
    { label: "Full palette", url: nesPalette },
  ],
};

// --- Tamagotchi P1 --------------------------------------------------------

// The commercial P1 mask-ROM is never shipped (license-policy). This is our own
// clean-room E0C6200 self-test program (also frozen as the golden image): it
// drives the display path and lights the top four rows across all 32 columns.
import tamaSelftest from "../../../tests/roms/tamagotchi/display-selftest.bin?url";

const tamagotchi: DeviceEntry = {
  id: "tamagotchi",
  label: "Tamagotchi P1",
  roms: [{ label: "Display self-test", url: tamaSelftest }],
};

export const DEVICES: DeviceEntry[] = [chip8, gameboy, nes, tamagotchi];
