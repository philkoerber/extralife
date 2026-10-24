import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

const repoRoot = resolve(__dirname, "../..");

// The smoke harness pulls in the library source/generated WASM and test ROMs
// (tests/roms submodules) from outside its own folder.
// Allow Vite to serve from the repo root so both resolve.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      // Use the library's source directly so harness dev/HMR picks up edits
      // without rebuilding the JS; build:wasm generates the relative core imports.
      "@philkoerber/extralife": resolve(repoRoot, "packages/extralife/src/index.ts"),
    },
  },
  server: {
    fs: { allow: [repoRoot] },
  },
  // Treat CHIP-8, Game Boy, NES and Tamagotchi ROMs as static binary assets so
  // `?url` imports work.
  assetsInclude: ["**/*.ch8", "**/*.gb", "**/*.nes", "**/*.bin"],
});
