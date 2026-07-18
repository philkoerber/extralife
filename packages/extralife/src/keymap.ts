/**
 * Keyboard → console-button mapping for `<ExtraLife>`.
 *
 * The seam stays device-agnostic: we map physical keys to the shared `Button`
 * superset (see `device.ts`), and each core ignores buttons it lacks (the NES
 * pad has no X/Y/L/R, etc.). Consumers can override the default via the
 * `<ExtraLife keymap=…>` prop.
 *
 * Keys are matched on `KeyboardEvent.code` (physical key, layout-independent) so
 * the mapping is the same on QWERTY/AZERTY/etc.
 */

import type { Button } from "./device.js";

/**
 * The `Button` variants in declaration order — the index each core's wasm
 * `setButton(index, …)` expects (mirrors `extralife_core::Button`; see any
 * core's `wasm.rs`). Order is the contract; do not reorder without the Rust
 * enum.
 */
export const BUTTON_ORDER: readonly Button[] = [
  "Up",
  "Down",
  "Left",
  "Right",
  "A",
  "B",
  "X",
  "Y",
  "L",
  "R",
  "Start",
  "Select",
];

/** The wasm `setButton` index for a button name. */
export function buttonIndex(button: Button): number {
  return BUTTON_ORDER.indexOf(button);
}

/** `KeyboardEvent.code` → button. Undefined codes are ignored (not swallowed). */
export type Keymap = Record<string, Button>;

/**
 * Default layout, standard for web NES/Game Boy emulators: arrows drive the
 * d-pad, Z/X are B/A, Enter is Start, right Shift is Select.
 */
export const DEFAULT_KEYMAP: Keymap = {
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  KeyZ: "B",
  KeyX: "A",
  Enter: "Start",
  ShiftRight: "Select",
};
