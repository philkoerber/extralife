/**
 * `<ExtraLife device="gameboy" rom={file} />` — the whole library in one prop
 * pair. It looks the core up in the registry, loads the WASM, loads the ROM,
 * and runs it at the device's native frame rate on an internal canvas. No
 * timing, loops, or WASM wiring for the caller to think about; swapping consoles
 * is just changing `device`.
 */

import { useEffect, useRef } from "react";
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent } from "react";
import { getRegistration } from "./registry.js";
import { runRealtime, type RealtimeHandle, type CoreInstance } from "./runner.js";
import type { DeviceId } from "./device.js";
import { DEFAULT_KEYMAP, buttonIndex, type Keymap } from "./keymap.js";

export interface ExtraLifeProps {
  /** Which console core to run. Must be registered (see `registry.ts`). */
  device: DeviceId;
  /** ROM image: raw bytes, an ArrayBuffer, or a URL to fetch. */
  rom: Uint8Array | ArrayBuffer | string;
  className?: string;
  style?: CSSProperties;
  /** Optional per-frame callback with the lit-pixel count (a cheap liveness signal). */
  onFrame?: (litPixels: number) => void;
  /** Called if the core or ROM fails to load. */
  onError?: (error: Error) => void;
  /**
   * `KeyboardEvent.code` → button map for keyboard input. Defaults to
   * `DEFAULT_KEYMAP` (arrows + Z/X + Enter/RightShift). Pass `{}` to disable
   * keyboard input entirely.
   */
  keymap?: Keymap;
  /** Optional test hook forwarded to the underlying canvas. */
  "data-testid"?: string;
}

async function resolveRom(rom: ExtraLifeProps["rom"]): Promise<Uint8Array> {
  if (typeof rom === "string") {
    const res = await fetch(rom);
    return new Uint8Array(await res.arrayBuffer());
  }
  return rom instanceof ArrayBuffer ? new Uint8Array(rom) : rom;
}

export function ExtraLife({
  device,
  rom,
  className,
  style,
  onFrame,
  onError,
  keymap = DEFAULT_KEYMAP,
  "data-testid": testId,
}: ExtraLifeProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const handleRef = useRef<RealtimeHandle | null>(null);
  const coreRef = useRef<CoreInstance | null>(null);

  // Keep the latest callbacks in a ref so they don't re-trigger the effect.
  // (Inline `onFrame`/`onError` are new on every render; if they were effect
  // deps, an onFrame that calls setState would reload the core every frame.)
  const onFrameRef = useRef(onFrame);
  const onErrorRef = useRef(onError);
  const keymapRef = useRef(keymap);
  onFrameRef.current = onFrame;
  onErrorRef.current = onError;
  keymapRef.current = keymap;

  useEffect(() => {
    let cancelled = false;
    let handle: RealtimeHandle | null = null;

    (async () => {
      try {
        const reg = getRegistration(device);
        const [core, bytes] = await Promise.all([reg.load(), resolveRom(rom)]);
        if (cancelled) return;

        core.loadRom(bytes);
        coreRef.current = core;

        const canvas = canvasRef.current;
        if (!canvas) return;
        canvas.width = core.width;
        canvas.height = core.height;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        ctx.imageSmoothingEnabled = false;

        handle = runRealtime(core, ctx, {
          frameHz: reg.frameHz,
          onFrame: (n) => onFrameRef.current?.(n),
        });
        handleRef.current = handle;
      } catch (e) {
        if (!cancelled) onErrorRef.current?.(e instanceof Error ? e : new Error(String(e)));
      }
    })();

    return () => {
      cancelled = true;
      handle?.stop();
      handleRef.current = null;
      coreRef.current = null;
    };
  }, [device, rom]);

  // Browsers gate audio behind a user gesture; resume on any pointer/key down.
  const resumeAudio = () => handleRef.current?.resumeAudio();

  // Translate a key event to a core button press. Scoped to the canvas (it's
  // focusable via tabIndex), so we don't hijack the whole page's arrow keys.
  const handleKey = (pressed: boolean) => (e: ReactKeyboardEvent<HTMLCanvasElement>) => {
    if (pressed) resumeAudio();
    const button = keymapRef.current[e.code];
    if (button === undefined) return; // unmapped key: leave it to the page
    if (e.repeat && pressed) return; // ignore auto-repeat; the button's already down
    e.preventDefault(); // stop arrows/Enter/Space from scrolling or activating
    coreRef.current?.setButton(buttonIndex(button), pressed);
  };

  return (
    <canvas
      ref={canvasRef}
      className={className}
      data-testid={testId}
      tabIndex={0}
      onPointerDown={resumeAudio}
      onKeyDown={handleKey(true)}
      onKeyUp={handleKey(false)}
      style={{ imageRendering: "pixelated", ...style }}
    />
  );
}
