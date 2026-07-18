import type { Device } from "./device.js";

export type { Button, Device, DeviceId, Screen } from "./device.js";
export { ExtraLife, type ExtraLifeProps } from "./ExtraLife.js";
export {
  runRealtime,
  type CoreInstance,
  type RealtimeHandle,
  type RealtimeOptions,
} from "./runner.js";
export { AudioPump } from "./audio.js";
export { REGISTRY, getRegistration, type CoreRegistration } from "./registry.js";
export {
  DEFAULT_KEYMAP,
  BUTTON_ORDER,
  buttonIndex,
  type Keymap,
} from "./keymap.js";

/**
 * Run a core headless for `frames` steps and return the final framebuffer.
 * This is the CI seam: golden-image tests instantiate a WASM core, call this,
 * and pixel-diff the result against `tests/golden/<device>/…`. No DOM, no React.
 *
 * ponytail: intentionally the whole "engine" for now — no audio pumping, no
 * timing. Loops are the point of an emulator; when a core needs sub-frame audio
 * draining in CI, extend here, not in the component.
 */
export function runHeadless(device: Device, frames: number): Uint8Array {
  for (let i = 0; i < frames; i++) {
    device.stepFrame();
  }
  return device.framebuffer();
}
