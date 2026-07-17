/**
 * The core registry — the one place that knows how to instantiate each device's
 * WASM core and at what frame rate to run it. `<ExtraLife device="gameboy" />`
 * looks a core up here; the caller never touches WASM loading or timing.
 *
 * `load()` mirrors what a wasm-pack "web" target expects: import the JS glue,
 * call its default export to initialize the WASM, then construct `Core`.
 *
 * ponytail: the WASM modules are referenced by the `@<device>-core` specifiers
 * that the smoke harness maps via Vite aliases to `crates/<crate>/pkg`. Ceiling:
 * this resolves inside the monorepo/harness but NOT for an external npm consumer
 * — publishing the cores as real sub-package deps is a separate packaging step.
 * Upgrade path: replace these specifiers with published `extralife-<device>`
 * package imports (or bundled sub-exports) once the cores ship to npm.
 */

import type { CoreInstance } from "./runner.js";
import type { DeviceId } from "./device.js";

interface WasmCoreModule {
  default: (input?: unknown) => Promise<unknown>;
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
      instantiate((await import("@chip8-core/extralife_chip8.js")) as WasmCoreModule),
  },
  gameboy: {
    // DMG refreshes at ~59.7275 Hz; pacing to this (not the monitor's rAF rate)
    // keeps games at true speed on 120 Hz / ProMotion displays.
    frameHz: 59.7275,
    load: async () =>
      instantiate((await import("@gameboy-core/extralife_gameboy.js")) as WasmCoreModule),
  },
  tamagotchi: {
    // The P1 core advances a fixed 32768/30 oscillator ticks per step_frame, so
    // 30 fps presents the emulated "life clock" at true speed.
    frameHz: 30,
    load: async () =>
      instantiate(
        (await import("@tamagotchi-core/extralife_tamagotchi.js")) as WasmCoreModule,
      ),
  },
};

export function getRegistration(device: DeviceId): CoreRegistration {
  const reg = REGISTRY[device];
  if (!reg) throw new Error(`extralife: no core registered for device "${device}"`);
  return reg;
}
