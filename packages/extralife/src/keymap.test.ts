import { describe, expect, it } from "vitest";
import { BUTTON_ORDER, DEFAULT_KEYMAP, buttonIndex } from "./keymap.js";
import type { Button } from "./device.js";

describe("keymap", () => {
  it("buttonIndex matches BUTTON_ORDER (the wasm setButton contract)", () => {
    // This order mirrors `extralife_core::Button`; a mismatch means every core
    // gets the wrong button. Pin it explicitly.
    expect(BUTTON_ORDER).toEqual([
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
    ]);
    BUTTON_ORDER.forEach((b, i) => expect(buttonIndex(b)).toBe(i));
  });

  it("default keymap resolves every code to a real button index", () => {
    const codes = Object.keys(DEFAULT_KEYMAP);
    // Sanity: the canonical d-pad + face-button layout is present.
    expect(DEFAULT_KEYMAP.ArrowUp).toBe("Up");
    expect(DEFAULT_KEYMAP.KeyZ).toBe("B");
    expect(DEFAULT_KEYMAP.KeyX).toBe("A");
    expect(DEFAULT_KEYMAP.Enter).toBe("Start");
    expect(DEFAULT_KEYMAP.ShiftRight).toBe("Select");
    for (const code of codes) {
      const idx = buttonIndex(DEFAULT_KEYMAP[code] as Button);
      expect(idx).toBeGreaterThanOrEqual(0);
      expect(idx).toBeLessThan(BUTTON_ORDER.length);
    }
  });

  it("simulates the component's key→setButton translation", () => {
    // Mirror the handler's core logic without a DOM: look the code up in the
    // map and forward (index, pressed) to the core.
    const calls: Array<[number, boolean]> = [];
    const core = { setButton: (i: number, p: boolean) => calls.push([i, p]) };
    const press = (code: string, pressed: boolean) => {
      const button = DEFAULT_KEYMAP[code];
      if (button === undefined) return;
      core.setButton(buttonIndex(button), pressed);
    };

    press("ArrowRight", true); // Right = index 3
    press("KeyX", true); // A = index 4
    press("KeyX", false);
    press("Backquote", true); // unmapped: ignored

    expect(calls).toEqual([
      [3, true],
      [4, true],
      [4, false],
    ]);
  });
});
