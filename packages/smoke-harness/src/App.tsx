import { useState } from "react";
import { ExtraLife } from "extralife";
import { DEVICES, type DeviceEntry } from "./devices.js";

type Status =
  | { kind: "idle" }
  | { kind: "running"; litPixels: number }
  | { kind: "error"; message: string };

export function App() {
  const [deviceId, setDeviceId] = useState(DEVICES[0].id);
  const [romIndex, setRomIndex] = useState(0);
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const device = DEVICES.find((d) => d.id === deviceId)!;
  const rom = device.roms[romIndex];

  return (
    <div style={styles.page}>
      <h1 style={styles.h1}>extralife · smoke harness</h1>
      <p style={styles.sub}>
        Renders the <code>&lt;ExtraLife&gt;</code> library component with a test
        ROM live. The last end-to-end check before a core session is done.
      </p>

      <div style={styles.controls}>
        <Select
          label="Device"
          value={deviceId}
          onChange={(v) => {
            setDeviceId(v as DeviceEntry["id"]);
            setRomIndex(0);
            setStatus({ kind: "idle" });
          }}
          options={DEVICES.map((d: DeviceEntry) => ({ value: d.id, label: d.label }))}
        />
        <Select
          label="ROM"
          value={String(romIndex)}
          onChange={(v) => {
            setRomIndex(Number(v));
            setStatus({ kind: "idle" });
          }}
          options={device.roms.map((r, i) => ({ value: String(i), label: r.label }))}
        />
      </div>

      <ExtraLife
        // Remount on device/ROM change so the core is freshly loaded.
        key={`${deviceId}:${romIndex}`}
        device={deviceId}
        rom={rom.url}
        data-testid="screen"
        style={styles.canvas}
        onFrame={(litPixels) => setStatus({ kind: "running", litPixels })}
        onError={(e) => setStatus({ kind: "error", message: e.message })}
      />

      <StatusLine status={status} />
    </div>
  );
}

function StatusLine({ status }: { status: Status }) {
  const map: Record<Status["kind"], { text: string; color: string }> = {
    idle: { text: "loading core + ROM…", color: "#e0a020" },
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
    background: "#000",
    border: "1px solid #33333c",
    borderRadius: 8,
  },
  status: { marginTop: 16, fontFamily: "ui-monospace, monospace", fontSize: 14 },
};
