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
      // Use the library's source directly so harness dev/HMR picks up edits
      // without a separate build; the WASM cores stay behind their own aliases.
      extralife: resolve(repoRoot, "packages/extralife/src/index.ts"),
      "@chip8-core": resolve(repoRoot, "crates/extralife-chip8/pkg"),
      "@gameboy-core": resolve(repoRoot, "crates/extralife-gameboy/pkg"),
      "@nes-core": resolve(repoRoot, "crates/extralife-nes/pkg"),
      "@tamagotchi-core": resolve(repoRoot, "crates/extralife-tamagotchi/pkg"),
    },
  },
  server: {
    fs: { allow: [repoRoot] },
  },
  // Treat CHIP-8, Game Boy, NES and Tamagotchi ROMs as static binary assets so
  // `?url` imports work.
  assetsInclude: ["**/*.ch8", "**/*.gb", "**/*.nes", "**/*.bin"],
});
