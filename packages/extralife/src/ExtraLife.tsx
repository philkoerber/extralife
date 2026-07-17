/**
 * `<ExtraLife device="gameboy" rom={file} />` — the whole library in one prop
 * pair. It looks the core up in the registry, loads the WASM, loads the ROM,
 * and runs it at the device's native frame rate on an internal canvas. No
 * timing, loops, or WASM wiring for the caller to think about; swapping consoles
 * is just changing `device`.
 */

import { useEffect, useRef } from "react";
import type { CSSProperties } from "react";
import { getRegistration } from "./registry.js";
import { runRealtime, type RealtimeHandle } from "./runner.js";
import type { DeviceId } from "./device.js";

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
  "data-testid": testId,
}: ExtraLifeProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const handleRef = useRef<RealtimeHandle | null>(null);

  // Keep the latest callbacks in a ref so they don't re-trigger the effect.
  // (Inline `onFrame`/`onError` are new on every render; if they were effect
  // deps, an onFrame that calls setState would reload the core every frame.)
  const onFrameRef = useRef(onFrame);
  const onErrorRef = useRef(onError);
  onFrameRef.current = onFrame;
  onErrorRef.current = onError;

  useEffect(() => {
    let cancelled = false;
    let handle: RealtimeHandle | null = null;

    (async () => {
      try {
        const reg = getRegistration(device);
        const [core, bytes] = await Promise.all([reg.load(), resolveRom(rom)]);
        if (cancelled) return;

        core.loadRom(bytes);

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
    };
  }, [device, rom]);

  // Browsers gate audio behind a user gesture; resume on any pointer/key down.
  const resumeAudio = () => handleRef.current?.resumeAudio();

  return (
    <canvas
      ref={canvasRef}
      className={className}
      data-testid={testId}
      onPointerDown={resumeAudio}
      onKeyDown={resumeAudio}
      style={{ imageRendering: "pixelated", ...style }}
    />
  );
}
