# extralife — every old device gets an extra life

```jsx
<ExtraLife device="gameboy" rom={file} />
```

A multi-system emulator library for the web. Rust cores compiled to WASM,
wrapped in typed React primitives, published to npm. Consoles first —
Game Boy, NES, SNES, Genesis — then the weird stuff: Tamagotchis,
calculators, anything with a screen and a soul.

Built almost entirely by AI agents using the Bun-rewrite playbook:
machine-checkable tests as the definition of done, agents grinding
autonomously until green.

## Architecture

- **Core contract** (`extralife-core`, Rust trait): every device implements
  `load_rom`, `step_frame`, RGBA framebuffer out, audio out, button input,
  `save_state`/`load_state`. Deterministic, headless-runnable.
- **One crate per device** (`extralife-chip8`, `extralife-gameboy`, …),
  each compiled to its own lazy-loaded WASM module.
- **One React package** (`extralife` on npm): the `<ExtraLife>` component
  plus hooks — `useFramebuffer`, `useSaveStates`, `useRewind` — and a
  headless mode for CI.
- **Docs site** with live homebrew demos per core.

## Development

```bash
git clone https://github.com/philkoerber/extralife.git
cd extralife
pnpm install
pnpm setup:roms   # checks out the tests/roms submodules (see note below)
cargo test --workspace   # headless cores + CPU/ROM/golden tests
```

Test inputs live in `tests/roms/` git submodules (permissive suites only — no
commercial ROMs, ever). Most are small, but **ProcessorTests** is a monorepo of
~10 CPU test suites; a full checkout inflates to several GB on disk. We only need
`nes6502/v1`, so `pnpm setup:roms` sparse-checks-out just that path. A plain
`git submodule update --init --recursive` also works but materializes every suite
— prefer the script. (Sparse-checkout is a local setting git can't pin in
`.gitmodules`, hence the helper.)

## Why agents can build this

Emulation has the best test infrastructure in software:

1. **CPU level:** [SingleStepTests](https://github.com/SingleStepTests) —
   JSON per-instruction tests (SM83, 6502, 65816, SPC700, Z80, 68000) with
   bus activity. Red/green signal before any video exists.
2. **System level:** community test ROMs run headless in CI; golden-image
   pixel diffs (dmg-acid2 etc.) verify PPU behavior to the pixel, and
   golden-audio sample diffs verify the APU deterministically.

Every core is an independent workstream. CI is the reviewer.

## Roadmap

See `consoles.csv` for the full target table (references, licenses, tests).

| Phase | Device                                                         | Status  |
| ----- | -------------------------------------------------------------- | ------- |
| 0     | CHIP-8 (pipeline proof)                                        | done    |
| 1     | Game Boy / Game Boy Color                                      | DMG done, GBC planned |
| 2     | NES                                                            | done    |
| 3     | Master System / Game Gear                                      | planned |
| 4     | SNES                                                           | planned |
| 5     | Genesis / Mega Drive                                           | planned |
| 6     | Game Boy Advance                                               | planned |
| —     | Extra lives for the weird: Atari 2600, Tamagotchi, sound chips | Tamagotchi done, rest planned |

## npm packages

- `extralife` — React components + hooks (the only package most users need)
- `extralife-gameboy`, `extralife-nes`, … — individual WASM cores,
  installed automatically as optional deps, loaded lazily by device

## License policy (strict)

This library is MIT. Therefore:

- **Port freely:** ISC/MIT sources only — [ares](https://github.com/ares-emulator/ares) (ISC),
  [SameBoy](https://github.com/LIJI32/SameBoy) (MIT/Expat).
- **Reference only, never translate:** GPL/MPL sources (Mesen 2, mGBA,
  Stella, blastem, tamalib).
- **Do not touch:** non-commercial licenses (snes9x, Genesis Plus GX).

## Legal guardrails

- Emulators are legal. **This repo never contains or distributes ROMs or
  BIOS files.** Demos use homebrew, test ROMs, and public-domain titles only.
- No copy-protection circumvention, anywhere.
