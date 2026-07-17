import { describe, expect, it } from "vitest";
import { runHeadless } from "./index.js";
import type { Device, Screen } from "./device.js";

/** A fake core that honors the contract, so we can test the harness without WASM. */
function fakeDevice(screen: Screen): Device {
  const buf = new Uint8Array(screen.width * screen.height * 4);
  let steps = 0;
  return {
    screen,
    loadRom() {},
    stepFrame() {
      steps++;
      buf[0] = steps & 0xff;
    },
    setButton() {},
    framebuffer: () => buf,
    audio: () => new Float32Array(0),
    sampleRate: 0,
    saveState: () => Uint8Array.from(buf),
    loadState() {},
  };
}

describe("runHeadless", () => {
  it("steps exactly `frames` times and returns a full-size framebuffer", () => {
    const screen = { width: 64, height: 32 };
    const fb = runHeadless(fakeDevice(screen), 60);
    expect(fb.length).toBe(screen.width * screen.height * 4);
    expect(fb[0]).toBe(60);
  });
});
