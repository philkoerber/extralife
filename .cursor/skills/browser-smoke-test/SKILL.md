---
name: browser-smoke-test
description: Build and run the extralife browser smoke test — load a device core (Rust→WASM) with a test ROM in the React harness and verify pixels render live in the browser. Use as the final end-to-end check when a core session is done, when adding a new device to the harness, or when the user mentions the smoke harness, browser test, or "does the ROM load".
---

# extralife browser smoke test

The last end-to-end check of the loop: prove a core actually runs in a browser.
`Rust core → wasm-pack → React harness → canvas`. Lives in
`packages/smoke-harness`. Registry-driven, so adding a device is one entry.

## Run the existing harness

```bash
# 1. Build the WASM package for the device (outputs to the gitignored pkg/)
wasm-pack build crates/extralife-<device> --target web --out-dir pkg

# 2. Start the dev server (background it; it must stay alive for the browser)
cd packages/smoke-harness && pnpm dev   # serves http://localhost:5173
```

Then open `http://localhost:5173` in the browser, pick device + ROM, and check
the canvas. Verify success via the status line, not just the screenshot (the
first screenshot often catches the "loading…" frame before the loop starts):

```js
// browser_cdp Runtime.evaluate
document.querySelector('[data-testid=status]')?.textContent
// expect "running · N lit pixels" with N > 0; a black screen = broken CPU/draw path
```

A live core reports lit pixels and renders a recognizable image. `error: …` in
the status means the WASM failed to load or `loadRom` rejected the ROM.

## Add a new device to the harness

Each device exposes the **same** wasm-bindgen surface, so the harness stays
device-agnostic. Do all three:

1. **wasm-bindgen wrapper** — in `crates/extralife-<device>/src/wasm.rs`, expose
   a `Core` class matching `crates/extralife-chip8/src/wasm.rs` exactly:
   `constructor`, getters `width`/`height`, `loadRom(&[u8])`, `stepFrame()`,
   `setButton(usize, bool)`, `framebuffer() -> Vec<u8>` (RGBA8888).
   Set `crate-type = ["cdylib", "rlib"]` and add `wasm-bindgen` in `Cargo.toml`.
2. **Vite alias** — add `"@<device>-core": resolve(repoRoot, "crates/extralife-<device>/pkg")`
   in `packages/smoke-harness/vite.config.ts`.
3. **Registry entry** — add one `DeviceEntry` to `DEVICES` in
   `packages/smoke-harness/src/devices.ts`: an `init()` that imports the module,
   calls `await mod.default()`, returns `new mod.Core()`, plus its test ROMs as
   `?url` imports from `tests/roms/<suite>`.

Nothing else changes — dropdowns, canvas, and the run loop read from `DEVICES`.

## Notes

- ROMs are `?url` asset imports from the `tests/roms/` submodules; never copy a
  ROM into the harness. Vite `server.fs.allow` is set to the repo root.
- The harness is a dev tool, not shipped: `pkg/` and `node_modules/` are gitignored.
- Keep this as the *final* check. CPU/system tests (`cargo test`) and golden
  pixel diffs are the real definition of done; the browser just confirms it lives.
