/**
 * The real-time engine: drives a WASM core at its native frame rate, paints the
 * framebuffer to a canvas, and pumps audio. This is the seam that turns a
 * headless, deterministic core (`stepFrame` = "compute one frame", no notion of
 * wall-clock — see `crates/extralife-core/src/lib.rs`) into something that runs
 * at real speed in a browser. The `<ExtraLife>` component wires this up; the
 * smoke harness uses it via the component too.
 *
 * Why timing lives here and not in the core: the core must stay wall-clock-free
 * so CI can step it as fast as possible for deterministic golden diffs. Pacing
 * to real time is the caller's job — and this is that caller, shared by every
 * consumer so nobody re-invents the loop.
 */

import { AudioPump } from "./audio.js";

/**
 * A live core instance, matching the wasm-bindgen `Core` class every crate
 * exports (see e.g. `crates/extralife-gameboy/src/wasm.rs`). Note this is the
 * real WASM surface (width/height getters, `setButton(index, pressed)`), which
 * differs from the aspirational full `Device` contract in `device.ts`.
 */
export interface CoreInstance {
  readonly width: number;
  readonly height: number;
  loadRom(rom: Uint8Array): void;
  stepFrame(): void;
  setButton(button: number, pressed: boolean): void;
  /** RGBA8888, row-major, top-left origin, length = width*height*4. */
  framebuffer(): Uint8Array;
  /** Interleaved stereo f32 produced by the last stepFrame; empty if silent. */
  audio(): Float32Array;
  /** Output sample rate in Hz; 0 means the core produces no audio. */
  readonly sampleRate: number;
}

export interface RealtimeOptions {
  /** Emulated frames per second. Decoupled from the display's refresh rate. */
  frameHz: number;
  /** Called after each painted frame with the count of lit (non-black) pixels. */
  onFrame?: (litPixels: number) => void;
}

export interface RealtimeHandle {
  /** Stop the loop, close audio, and release the animation-frame callback. */
  stop(): void;
  /** Resume the AudioContext from a user gesture (browsers gate audio). */
  resumeAudio(): void;
}

/**
 * Run `core` in real time, painting into `ctx`. Returns a handle to stop it.
 *
 * The loop accumulates real elapsed time and steps exactly as many emulated
 * frames as have truly elapsed, so a 120 Hz / ProMotion display doesn't run a
 * 60 Hz console at double speed. A catch-up cap prevents a backgrounded tab from
 * fast-forwarding through hundreds of frames when it regains focus.
 */
export function runRealtime(
  core: CoreInstance,
  ctx: CanvasRenderingContext2D,
  opts: RealtimeOptions,
): RealtimeHandle {
  const frameMs = 1000 / opts.frameHz;
  const image = ctx.createImageData(core.width, core.height);

  let audio: AudioPump | null = null;
  if (core.sampleRate > 0) {
    audio = new AudioPump(core.sampleRate);
    audio.resume();
  }

  let raf = 0;
  let stopped = false;
  let last = performance.now();
  let acc = 0;

  const tick = () => {
    if (stopped) return;
    const now = performance.now();
    acc += now - last;
    last = now;

    // Cap catch-up so a huge dt (backgrounded tab) doesn't fast-forward.
    if (acc > frameMs * 4) acc = frameMs * 4;

    let ran = false;
    while (acc >= frameMs) {
      core.stepFrame();
      if (audio) audio.push(core.audio());
      acc -= frameMs;
      ran = true;
    }

    if (ran) {
      const fb = core.framebuffer();
      image.data.set(fb);
      ctx.putImageData(image, 0, 0);
      if (opts.onFrame) opts.onFrame(countLit(fb));
    }
    raf = requestAnimationFrame(tick);
  };
  tick();

  return {
    stop() {
      stopped = true;
      cancelAnimationFrame(raf);
      if (audio) audio.close();
      audio = null;
    },
    resumeAudio() {
      audio?.resume();
    },
  };
}

function countLit(fb: Uint8Array): number {
  let n = 0;
  for (let i = 0; i < fb.length; i += 4) if (fb[i] > 0) n++;
  return n;
}
