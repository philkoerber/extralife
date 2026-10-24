/**
 * The core registry — the one place that knows how to instantiate each device's
 * WASM core and at what frame rate to run it. `<ExtraLife device="gameboy" />`
 * looks a core up here; the caller never touches WASM loading or timing.
 *
 * `load()` mirrors what a wasm-pack "web" target expects: import the JS glue,
 * call its default export to initialize the WASM, then construct `Core`.
 *
 * The generated modules live in the package's `wasm/` directory, alongside
 * both `src/` and `dist/`, so these relative imports work in development and
 * in the published package without consumer-specific aliases.
 */

import type { CoreInstance } from "./runner.js";
import type { DeviceId } from "./device.js";

interface WasmCoreModule {
  default: () => Promise<unknown>;
  Core: new () => CoreInstance;
}

export interface CoreRegistration {
  /** Load + initialize the WASM module and return a fresh core instance. */
  load: () => Promise<CoreInstance>;
  /** Native emulated frames per second for this device. */
  frameHz: number;
}

async function instantiate(mod: WasmCoreModule): Promise<CoreInstance> {
  await mod.default();
  return new mod.Core();
}

export const REGISTRY: Partial<Record<DeviceId, CoreRegistration>> = {
  chip8: {
    frameHz: 60,
    load: async () =>
      instantiate(await import("../wasm/chip8/extralife_chip8.js")),
  },
  gameboy: {
    // DMG refreshes at ~59.7275 Hz; pacing to this (not the monitor's rAF rate)
    // keeps games at true speed on 120 Hz / ProMotion displays.
    frameHz: 59.7275,
    load: async () =>
      instantiate(await import("../wasm/gameboy/extralife_gameboy.js")),
  },
  nes: {
    // NTSC NES refreshes at ~60.0988 Hz (master clock 21.477272 MHz / 4 CPU
    // divider, 341*262 - 0.5 dots/frame). Pace to this for true game speed.
    frameHz: 60.0988,
    load: async () =>
      instantiate(await import("../wasm/nes/extralife_nes.js")),
  },
  tamagotchi: {
    // The P1 core advances a fixed 32768/30 oscillator ticks per step_frame, so
    // 30 fps presents the emulated "life clock" at true speed.
    frameHz: 30,
    load: async () =>
      instantiate(await import("../wasm/tamagotchi/extralife_tamagotchi.js")),
  },
};

export function getRegistration(device: DeviceId): CoreRegistration {
  const reg = REGISTRY[device];
  if (!reg) throw new Error(`extralife: no core registered for device "${device}"`);
  return reg;
}
