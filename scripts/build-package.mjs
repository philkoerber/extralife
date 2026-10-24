#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { rm, stat } from "node:fs/promises";
import { createRequire } from "node:module";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const packageDirectory = join(root, "packages/extralife");
const require = createRequire(join(packageDirectory, "package.json"));
const devices = ["chip8", "gameboy", "nes", "tamagotchi"];
const wasmOnly = process.argv.includes("--wasm-only");

function run(command, args) {
  const result = spawnSync(command, args, {
    // Cargo must pick up the workspace's WASM linker configuration.
    cwd: root,
    stdio: "inherit",
    timeout: 300_000,
  });
  if (result.error) {
    throw new Error(`Could not run ${command}: ${result.error.message}`, { cause: result.error });
  }
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.signal ?? `exit ${result.status}`})`);
  }
}

async function build() {
  if (process.argv.slice(2).some((arg) => arg !== "--wasm-only")) {
    throw new Error("Usage: node scripts/build-package.mjs [--wasm-only]");
  }

  const tool = spawnSync("wasm-pack", ["--version"], { encoding: "utf8", timeout: 10_000 });
  if (tool.error || tool.status !== 0) {
    throw new Error(
      "wasm-pack is required to build from source. Install Rust, run `rustup target add " +
      "wasm32-unknown-unknown`, then `cargo install wasm-pack --version 0.15.0 --locked`. " +
      "npm consumers do not need these tools.",
    );
  }
  console.log(tool.stdout.trim());

  // Start clean so removed devices and old generated files cannot leak into npm.
  const wasmDirectory = join(packageDirectory, "wasm");
  await rm(wasmDirectory, { recursive: true, force: true });
  for (const device of devices) {
    const name = `extralife_${device}`;
    const output = join(wasmDirectory, device);
    console.log(`\nBuilding ${device} WASM…`);
    run("wasm-pack", [
      "build", join(root, "crates", `extralife-${device}`),
      "--target", "web", "--release", "--out-dir", output,
      "--out-name", name, "--no-pack", "--", "--locked",
    ]);

    // wasm-pack's nested ignore file would otherwise hide generated assets from npm.
    await rm(join(output, ".gitignore"), { force: true });
    for (const suffix of [".js", ".d.ts", "_bg.wasm"]) {
      const artifact = join(output, name + suffix);
      const info = await stat(artifact);
      if (!info.isFile() || info.size === 0) throw new Error(`Missing or empty artifact: ${artifact}`);
    }
  }

  if (!wasmOnly) {
    await rm(join(packageDirectory, "dist"), { recursive: true, force: true });
    console.log("\nBuilding JavaScript and TypeScript declarations…");
    run(process.execPath, [
      require.resolve("typescript/bin/tsc"),
      "--project", join(packageDirectory, "tsconfig.build.json"),
    ]);
  }
  console.log(`\nextralife ${wasmOnly ? "WASM" : "package"} build complete.`);
}

await build().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
