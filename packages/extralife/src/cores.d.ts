/**
 * Ambient module declarations for the WASM core glue that the registry imports
 * via `@<device>-core` specifiers (see `registry.ts`). These are resolved at
 * runtime by the consumer's bundler (the smoke harness maps them to
 * `crates/<crate>/pkg` via Vite aliases); this declaration just lets `tsc` type
 * the dynamic imports without the actual generated `.d.ts` on the tsc path.
 *
 * ponytail: typed loosely as the wasm-pack shape (default init + `Core` class).
 * When the cores are published as real packages, drop these and import their
 * generated types directly.
 */

declare module "@chip8-core/extralife_chip8.js" {
  const init: (input?: unknown) => Promise<unknown>;
  export default init;
  export class Core {
    readonly width: number;
    readonly height: number;
    loadRom(rom: Uint8Array): void;
    stepFrame(): void;
    setButton(button: number, pressed: boolean): void;
    framebuffer(): Uint8Array;
    audio(): Float32Array;
    readonly sampleRate: number;
  }
}

declare module "@gameboy-core/extralife_gameboy.js" {
  const init: (input?: unknown) => Promise<unknown>;
  export default init;
  export class Core {
    readonly width: number;
    readonly height: number;
    loadRom(rom: Uint8Array): void;
    stepFrame(): void;
    setButton(button: number, pressed: boolean): void;
    framebuffer(): Uint8Array;
    audio(): Float32Array;
    readonly sampleRate: number;
  }
}

declare module "@nes-core/extralife_nes.js" {
  const init: (input?: unknown) => Promise<unknown>;
  export default init;
  export class Core {
    readonly width: number;
    readonly height: number;
    loadRom(rom: Uint8Array): void;
    stepFrame(): void;
    setButton(button: number, pressed: boolean): void;
    framebuffer(): Uint8Array;
    audio(): Float32Array;
    readonly sampleRate: number;
  }
}

declare module "@tamagotchi-core/extralife_tamagotchi.js" {
  const init: (input?: unknown) => Promise<unknown>;
  export default init;
  export class Core {
    readonly width: number;
    readonly height: number;
    loadRom(rom: Uint8Array): void;
    stepFrame(): void;
    setButton(button: number, pressed: boolean): void;
    framebuffer(): Uint8Array;
    audio(): Float32Array;
    readonly sampleRate: number;
  }
}
