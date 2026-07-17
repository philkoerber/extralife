import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

const repoRoot = resolve(__dirname, "../..");

// The smoke harness pulls in two things that live outside its own folder:
// the wasm-pack output (crates/*/pkg) and the test ROMs (tests/roms submodules).
// Allow Vite to serve from the repo root so both resolve.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@chip8-core": resolve(repoRoot, "crates/extralife-chip8/pkg"),
      "@gameboy-core": resolve(repoRoot, "crates/extralife-gameboy/pkg"),
    },
  },
  server: {
    fs: { allow: [repoRoot] },
  },
  // Treat CHIP-8 and Game Boy ROMs as static binary assets so `?url` imports work.
  assetsInclude: ["**/*.ch8", "**/*.gb"],
});
