/**
 * The TypeScript mirror of the Rust `Device` contract
 * (see `crates/extralife-core/src/lib.rs`). This is the seam on the JS side:
 * every WASM core, once instantiated, exposes exactly this shape, and the
 * `<ExtraLife>` component + hooks + the headless CI runner all speak only to
 * this interface — never to a specific console.
 *
 * Keep this in lockstep with the Rust trait. Adding a method here without the
 * matching Rust default (or vice versa) breaks the swap-any-core promise.
 */

/** Superset of console buttons; a core ignores what it lacks. Mirrors `Button`. */
export type Button =
  | "Up"
  | "Down"
  | "Left"
  | "Right"
  | "A"
  | "B"
  | "X"
  | "Y"
  | "L"
  | "R"
  | "Start"
  | "Select";

export interface Screen {
  width: number;
  height: number;
}

/** A live, instantiated core. WASM modules expose this after loading a ROM. */
export interface Device {
  readonly screen: Screen;
  loadRom(rom: Uint8Array): void;
  stepFrame(): void;
  setButton(button: Button, pressed: boolean): void;
  /** RGBA8888, row-major, top-left origin, length = width*height*4. */
  framebuffer(): Uint8Array;
  /** Interleaved stereo f32; empty if the core has no audio yet. */
  audio(): Float32Array;
  readonly sampleRate: number;
  saveState(): Uint8Array;
  loadState(state: Uint8Array): void;
}

/** Identifiers matching the `extralife-<device>` cores. Grows one per session. */
export type DeviceId =
  | "chip8"
  | "gameboy"
  | "nes"
  | "sms"
  | "snes"
  | "genesis"
  | "gba";
