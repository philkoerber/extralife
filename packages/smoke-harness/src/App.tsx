import { useEffect, useRef, useState } from "react";
import { DEVICES, type CoreInstance, type DeviceEntry } from "./devices.js";
import { AudioPump } from "./audio.js";

type Status =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "running"; litPixels: number }
  | { kind: "error"; message: string };

export function App() {
  const [deviceId, setDeviceId] = useState(DEVICES[0].id);
  const [romIndex, setRomIndex] = useState(0);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const audioRef = useRef<AudioPump | null>(null);

  const device = DEVICES.find((d) => d.id === deviceId)!;

  useEffect(() => setRomIndex(0), [deviceId]);

  // AudioContext starts suspended until a user gesture. Resume on any control
  // interaction so audio-producing cores play once the user touches the page.
  const resumeAudio = () => audioRef.current?.resume();

  useEffect(() => {
    let raf = 0;
    let core: CoreInstance | null = null;
    let cancelled = false;
    let audio: AudioPump | null = null;

    async function run() {
      setStatus({ kind: "loading" });
      try {
        core = await device.init();
        const rom = new Uint8Array(
          await (await fetch(device.roms[romIndex].url)).arrayBuffer(),
        );
        core.loadRom(rom);
        if (cancelled) return;

        // Opt-in audio: only cores that report a rate get a Web Audio pump. The
        // context starts suspended until a user gesture; `audioRef` lets the
        // control handlers resume it (see onChange below).
        if (core.sampleRate > 0) {
          audio = new AudioPump(core.sampleRate);
          audioRef.current = audio;
          audio.resume();
        }

        const canvas = canvasRef.current!;
        canvas.width = core.width;
        canvas.height = core.height;
        const ctx = canvas.getContext("2d")!;
        ctx.imageSmoothingEnabled = false;
        const image = ctx.createImageData(core.width, core.height);

        const tick = () => {
          if (cancelled || !core) return;
          core.stepFrame();
          if (audio) audio.push(core.audio());
          const fb = core.framebuffer();
          image.data.set(fb);
          ctx.putImageData(image, 0, 0);
          const lit = countLit(fb);
          setStatus({ kind: "running", litPixels: lit });
          raf = requestAnimationFrame(tick);
        };
        tick();
      } catch (e) {
        if (!cancelled)
          setStatus({ kind: "error", message: String((e as Error).message ?? e) });
      }
    }

    run();
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      if (audio) audio.close();
      audioRef.current = null;
    };
  }, [device, romIndex]);

  return (
    <div style={styles.page} onClick={resumeAudio}>
      <h1 style={styles.h1}>extralife · smoke harness</h1>
      <p style={styles.sub}>
        Loads a device core (Rust → WASM) and runs a test ROM live. The last
        end-to-end check before a core session is done.
      </p>

      <div style={styles.controls}>
        <Select
          label="Device"
          value={deviceId}
          onChange={(v) => {
            resumeAudio();
            setDeviceId(v);
          }}
          options={DEVICES.map((d: DeviceEntry) => ({ value: d.id, label: d.label }))}
        />
        <Select
          label="ROM"
          value={String(romIndex)}
          onChange={(v) => {
            resumeAudio();
            setRomIndex(Number(v));
          }}
          options={device.roms.map((r, i) => ({ value: String(i), label: r.label }))}
        />
      </div>

      <canvas
        ref={canvasRef}
        style={styles.canvas}
        data-testid="screen"
      />

      <StatusLine status={status} />
    </div>
  );
}

function countLit(fb: Uint8Array): number {
  let n = 0;
  for (let i = 0; i < fb.length; i += 4) if (fb[i] > 0) n++;
  return n;
}

function StatusLine({ status }: { status: Status }) {
  const map: Record<Status["kind"], { text: string; color: string }> = {
    idle: { text: "idle", color: "#888" },
    loading: { text: "loading core + ROM…", color: "#e0a020" },
    running: { text: "", color: "#40c060" },
    error: { text: "", color: "#e04040" },
  };
  const base = map[status.kind];
  const text =
    status.kind === "running"
      ? `running · ${status.litPixels} lit pixels`
      : status.kind === "error"
        ? `error: ${status.message}`
        : base.text;
  return (
    <div style={{ ...styles.status, color: base.color }} data-testid="status">
      {text}
    </div>
  );
}

function Select({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <label style={styles.label}>
      {label}
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        style={styles.select}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

const styles: Record<string, React.CSSProperties> = {
  page: {
    fontFamily: "ui-sans-serif, system-ui, sans-serif",
    background: "#0d0d10",
    color: "#e8e8ea",
    minHeight: "100vh",
    margin: 0,
    padding: "48px 24px",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
  },
  h1: { fontSize: 22, fontWeight: 700, margin: 0, letterSpacing: -0.3 },
  sub: { color: "#9a9aa2", maxWidth: 460, textAlign: "center", lineHeight: 1.5 },
  controls: { display: "flex", gap: 16, margin: "8px 0 24px" },
  label: { display: "flex", flexDirection: "column", gap: 6, fontSize: 13, color: "#b0b0b8" },
  select: {
    background: "#1a1a20",
    color: "#e8e8ea",
    border: "1px solid #33333c",
    borderRadius: 8,
    padding: "8px 10px",
    fontSize: 14,
  },
  canvas: {
    width: 512,
    imageRendering: "pixelated",
    background: "#000",
    border: "1px solid #33333c",
    borderRadius: 8,
  },
  status: { marginTop: 16, fontFamily: "ui-monospace, monospace", fontSize: 14 },
};
