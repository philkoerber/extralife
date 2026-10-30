# extralife

Multi-system emulation for React: Rust/WASM cores, a canvas-based component,
and TypeScript declarations in **one ESM npm package**:
[`@philkoerber/extralife`](https://www.npmjs.com/package/@philkoerber/extralife).

## Install

In an existing React application:

```bash
npm install @philkoerber/extralife
```

**React >=18 is a required peer dependency**, including when using the
lower-level exports. Install React separately if your application does not
already provide it. The package is ESM-only, not a CommonJS `require()` entry point.

All four device cores, their WASM binaries/glue, and TypeScript declarations
ship together. The selected core loads automatically and lazily through
portable relative imports. Consumers need **no Rust toolchain, separate core
packages, import aliases, or Vite WASM plugins/configuration**.

## Quick start

```tsx
import { ExtraLife } from "@philkoerber/extralife";

export default function Emulator() {
  return (
    <ExtraLife
      device="gameboy"
      rom="/roms/homebrew.gb"
      onError={console.error}
      style={{ width: 480, height: 432 }}
    />
  );
}
```

Optional: to use imports from `"extralife"`, install a local npm alias instead:

```bash
npm install extralife@npm:@philkoerber/extralife
```

This aliases the same scoped package locally. npm rejects the unscoped name
`extralife` as too similar to the existing `extra-life` package, so bare
`npm install extralife` does **not** install this library.

Serve your own legally distributable ROM at `/roms/homebrew.gb`; that file is
an example URL, not an included asset. The component loads the core and ROM,
steps at the device's native frame rate, renders to a pixelated canvas, and
handles keyboard input and browser audio.

### Supported devices

| `device` | System |
| --- | --- |
| `chip8` | CHIP-8 |
| `gameboy` | Game Boy (DMG) |
| `nes` | NES |
| `tamagotchi` | Tamagotchi |

These are the only supported devices. **Game Boy Color is planned**, not part
of the Game Boy core's supported feature set. Other roadmap systems are not
included cores.

The Game Boy core supports ROM-only, MBC1, **MBC3 without a clock** (cartridge
types `0x11`–`0x13`, added in 0.1.1), and MBC5. Original Pokémon Red/Blue use
MBC3 + RAM + battery (`0x13`). RTC-equipped MBC3 cartridges (`0x0f`/`0x10`)
and MBC2 remain unsupported and are rejected when loaded. Cartridge RAM works
within a running session; browser-persistent battery-save import/export is
not yet exposed by the component.

### ROM inputs

`rom` accepts a **URL string**, **`Uint8Array`**, or **`ArrayBuffer`**. URL strings
are fetched in the browser; cross-origin ROM servers must allow CORS.
A browser `File` is **not** accepted directly. Convert it with
`await file.arrayBuffer()`:

```tsx
import { useState } from "react";
import { ExtraLife } from "@philkoerber/extralife";

export function LocalRom() {
  const [rom, setRom] = useState<ArrayBuffer | null>(null);

  return (
    <>
      <input
        type="file"
        accept=".gb"
        onChange={async (event) => {
          const file = event.currentTarget.files?.[0];
          if (file) setRom(await file.arrayBuffer());
        }}
      />
      {rom && (
        <ExtraLife device="gameboy" rom={rom} onError={console.error} />
      )}
    </>
  );
}
```

Changing `device` or `rom` reloads the emulator. Keep byte-buffer props stable
between unrelated renders to avoid restarting it.

## Component props

`ExtraLifeProps` is exported for TypeScript users.

| Prop | Type | Behavior |
| --- | --- | --- |
| `device` | `DeviceId` | Required; use one of the four supported IDs above. |
| `rom` | `Uint8Array \| ArrayBuffer \| string` | Required ROM bytes or URL. |
| `className` | `string` | Applied to the canvas. |
| `style` | `CSSProperties` | Canvas styling; pixelated rendering is the default. |
| `keymap` | `Keymap` | Maps `KeyboardEvent.code` to button names; `{}` disables keyboard input. |
| `onFrame` | `(litPixels: number) => void` | Called after each rendered frame with its non-black pixel count. |
| `onError` | `(error: Error) => void` | Reports core/ROM loading failures. |
| `data-testid` | `string` | Forwarded to the canvas for tests. |

### Controls and audio

Focus the canvas by clicking it or using Tab. Keyboard input is scoped to that
canvas, not the whole page. The default mapping uses physical keyboard codes:

| Keys | Buttons |
| --- | --- |
| Arrow keys | Up / Down / Left / Right |
| Z (`KeyZ`) | B |
| X (`KeyX`) | A |
| Enter | Start |
| Right Shift (`ShiftRight`) | Select |

Devices use only the buttons they support. Pass a custom `keymap` to replace
the mapping, or extend the exported `DEFAULT_KEYMAP`. `Keymap`, `Button`,
`BUTTON_ORDER`, and `buttonIndex` are also exported for custom input handling.

Browsers gate audio behind a user gesture. **After the core has loaded**, click
or tap the canvas, or press a key while it is focused, to resume audio. Autoplay
with sound cannot be guaranteed.

## Browser and bundler support

- Targets modern browsers with WebAssembly, ES modules/dynamic imports, Canvas
  2D, `fetch`, `requestAnimationFrame`, and Web Audio.
- Use a browser bundler that handles dynamic imports and URL-based WASM assets.
  **Vite needs no WASM-specific configuration or aliases.** Deploy the generated
  chunks and WASM assets together; do not copy only the top-level JS file.
- Serve the app/assets over HTTP(S), rather than opening an HTML file via `file://`.
- In SSR frameworks, put the component in a **client component** (for example,
  add `"use client"` in Next.js). Emulation, canvas rendering, and audio are
  browser-side, not server-rendered functionality.

The package contains `dist/*.js`, `dist/*.d.ts`, and for each device:

```text
wasm/<device>/extralife_<device>.js
wasm/<device>/extralife_<device>.d.ts
wasm/<device>/extralife_<device>_bg.wasm
```

The component manages loading these assets; applications do not need to import
WASM files manually. Lower-level exports include `getRegistration`, `REGISTRY`,
`runRealtime`, and `AudioPump`. Higher-level hooks such as `useFramebuffer`,
`useSaveStates`, and `useRewind` are planned, not available exports.

## License and ROM policy

[MIT](LICENSE) — Copyright (c) 2026 extralife contributors.

**Never distribute commercial ROMs or BIOS files** with this package or your
demos. No ROMs or BIOS files are included. Supply only content you are legally
entitled to use; redistribute homebrew, test ROMs, or public-domain titles only
when their licenses permit it, retaining any required notices. The library's
MIT license does not grant rights to third-party content.

For source builds, tests, and release instructions, see the
[repository README](https://github.com/philkoerber/extralife#development).
