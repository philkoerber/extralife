#!/usr/bin/env node

// Run after the package build, or pass --published after publication. Everything
// generated here, including the tarball, stays in one disposable directory;
// dist/ and wasm/ are never modified.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFileSync } from "node:fs";
import { lstat, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { basename, dirname, extname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("../", import.meta.url));
const packageDirectory = join(root, "packages/extralife");
const { name: packageName, version: packageVersion } = JSON.parse(readFileSync(join(packageDirectory, "package.json"), "utf8"));
const packageRequire = createRequire(join(packageDirectory, "package.json"));
const harnessRequire = createRequire(join(root, "packages/smoke-harness/package.json"));
const execFileAsync = promisify(execFile);
const devices = ["chip8", "gameboy", "nes", "tamagotchi"];
const deadline = Date.now() + 270_000;

function timeout(limit) {
  const remaining = deadline - Date.now();
  assert(remaining > 0, "Package smoke test exceeded its 270-second budget");
  return Math.min(limit, remaining);
}

// Playwright's evaluate() has no timeout of its own. A wedged WASM main thread
// must not make either the result read or failure diagnostics hang the caller.
async function bounded(label, limit, operation) {
  const milliseconds = timeout(limit);
  let timer;
  try {
    return await Promise.race([
      Promise.resolve().then(operation),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(label + " timed out")), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function installedPackage(requireFrom, name) {
  try {
    const filename = requireFrom.resolve(name + "/package.json");
    return { filename, manifest: JSON.parse(readFileSync(filename, "utf8")) };
  } catch (error) {
    throw new Error("Cannot resolve installed " + name + "; run pnpm install first", { cause: error });
  }
}

function toolchainVersions() {
  const versions = {
    react: installedPackage(harnessRequire, "react").manifest.version,
    "react-dom": installedPackage(harnessRequire, "react-dom").manifest.version,
    "@types/react": installedPackage(packageRequire, "@types/react").manifest.version,
    "@types/react-dom": installedPackage(harnessRequire, "@types/react-dom").manifest.version,
    typescript: installedPackage(packageRequire, "typescript").manifest.version,
    vite: installedPackage(harnessRequire, "vite").manifest.version,
  };
  for (const [name, version] of Object.entries(versions)) {
    assert(/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(version), "Expected an exact installed version for " + name);
  }
  for (const name of ["react", "react-dom"]) {
    assert(Number(versions[name].split(".")[0]) >= 18, "The installed smoke-harness " + name + " must be >=18");
  }
  return versions;
}

async function run(label, executable, args, cwd, limit, { quiet = false, env = {} } = {}) {
  console.log("[package-smoke] " + label);
  try {
    const result = await execFileAsync(executable, args, {
      cwd,
      env: { ...process.env, npm_config_update_notifier: "false", ...env },
      timeout: timeout(limit),
      killSignal: "SIGKILL",
      maxBuffer: 4 * 1024 * 1024,
      encoding: "utf8",
    });
    if (!quiet && result.stdout.trim()) console.log(result.stdout.trim());
    if (result.stderr.trim()) console.error(result.stderr.trim());
    return result.stdout;
  } catch (error) {
    throw new Error(
      label + " failed" + (error.killed ? " (subprocess timed out or exceeded its output limit)" : "") +
        "\n" + [error.stdout, error.stderr, error.message].filter(Boolean).join("\n"),
      { cause: error },
    );
  }
}

function checkPack(stdout) {
  let packs;
  try {
    packs = JSON.parse(stdout);
  } catch (error) {
    throw new Error("npm pack did not return valid JSON:\n" + stdout, { cause: error });
  }
  assert(Array.isArray(packs) && packs.length === 1, "Expected exactly one packed artifact");
  const pack = packs[0];
  assert.equal(pack.name, packageName, "Packed name must match the source package.json");
  assert.equal(pack.version, packageVersion, "Packed version must match the source package.json");
  assert(typeof pack.filename === "string" && /^[\w.-]+\.tgz$/.test(pack.filename), "Unsafe tarball filename");
  assert(Array.isArray(pack.files) && pack.files.length > 0, "npm pack omitted its file list");

  // Exact paths, rather than a broad dist/** allowlist, catch leaked tests,
  // source, fixtures, ROMs, BIOS images, build metadata, and stale output.
  const allowed = new Set(["package.json"]);
  const required = new Set(["package.json", "dist/index.js", "dist/index.d.ts"]);
  for (const module of ["index", "ExtraLife", "audio", "device", "keymap", "registry", "runner"]) {
    allowed.add("dist/" + module + ".js");
    allowed.add("dist/" + module + ".d.ts");
  }
  for (const device of devices) {
    const prefix = "wasm/" + device + "/extralife_" + device;
    for (const suffix of [".js", ".d.ts", "_bg.wasm"]) {
      allowed.add(prefix + suffix);
      required.add(prefix + suffix);
    }
    // Some wasm-bindgen versions emit declarations for the raw WASM exports.
    allowed.add(prefix + "_bg.wasm.d.ts");
  }
  const readme = /^README(?:\.md|\.txt)?$/i;
  const license = /^LICEN[CS]E(?:\.md|\.txt)?$/i;
  const names = new Set();
  for (const file of pack.files) {
    assert(typeof file.path === "string", "Invalid packed file entry");
    assert(!names.has(file.path), "Duplicate packed file: " + file.path);
    names.add(file.path);
    assert(
      allowed.has(file.path) || readme.test(file.path) || license.test(file.path),
      "Unexpected packed file (source/tests/ROMs/BIOS must not ship): " + file.path,
    );
    assert(Number.isFinite(file.size) && file.size > 0, "Empty packed file: " + file.path);
  }
  for (const name of required) assert(names.has(name), "Missing packed file: " + name);
  assert([...names].some((name) => readme.test(name)), "Packed README is missing");
  assert([...names].some((name) => license.test(name)), "Packed LICENSE is missing");
  console.log("[package-smoke] Archive allowlist passed (" + names.size + " files)");
  return pack;
}

function inside(directory, filename) {
  const path = relative(directory, filename);
  return !isAbsolute(path) && path !== ".." && !path.startsWith(".." + sep);
}

async function checkInstallation(consumerDirectory, versions) {
  const consumerRequire = createRequire(join(consumerDirectory, "package.json"));
  const nodeModules = join(consumerDirectory, "node_modules");
  const installedDirectory = join(nodeModules, ...packageName.split("/"));
  const info = await lstat(installedDirectory);
  assert(info.isDirectory() && !info.isSymbolicLink(), "extralife must be extracted from the tarball, not linked");
  assert.equal(await realpath(installedDirectory), installedDirectory);
  assert.equal(consumerRequire.resolve(packageName), join(installedDirectory, "dist/index.js"));
  const manifest = JSON.parse(await readFile(join(installedDirectory, "package.json"), "utf8"));
  const entry = (path) => typeof path === "string" ? path.replace(/^\.\//, "") : path;
  assert.equal(manifest.name, packageName, "Installed name must match the source package.json");
  assert.equal(manifest.version, packageVersion, "Installed version must match the source package.json");
  assert.equal(manifest.type, "module");
  assert.notEqual(manifest.private, true, "The packed package must be publishable");
  assert.equal(entry(manifest.main), "dist/index.js");
  assert.equal(entry(manifest.types), "dist/index.d.ts");
  assert.equal(entry(manifest.exports?.["."]?.types), "dist/index.d.ts");
  assert.equal(entry(manifest.exports?.["."]?.import ?? manifest.exports?.["."]?.default), "dist/index.js");
  assert.equal(manifest.peerDependencies?.react, ">=18", "React >=18 must be a required peer");
  assert.notEqual(manifest.peerDependenciesMeta?.react?.optional, true, "React must not be an optional peer");
  assert(Array.isArray(manifest.files) && manifest.files.includes("dist") && manifest.files.includes("wasm"));
  for (const [name, version] of Object.entries(versions)) {
    const installed = installedPackage(consumerRequire, name);
    assert(inside(nodeModules, await realpath(installed.filename)), name + " resolved outside the isolated consumer");
    assert.equal(installed.manifest.version, version, "Consumer toolchain version mismatch for " + name);
  }
  return consumerRequire;
}

const consumerSource = String.raw`
import { createRoot } from "react-dom/client";
import { ExtraLife, getRegistration, REGISTRY, type DeviceId, type CoreInstance } from ${JSON.stringify(packageName)};

type CoreCheck = {
  id: DeviceId;
  width: number;
  height: number;
  framebufferBytes: number;
  sampleRate: number;
};
type ComponentCheck = { frames: number; width: number; height: number; litPixels: number };
type SmokeResult =
  | { status: "running"; stage: string }
  | { status: "passed"; cores: CoreCheck[]; component: ComponentCheck }
  | { status: "failed"; error: string };

declare global {
  interface Window {
    __packageSmoke: SmokeResult;
  }
}

window.__packageSmoke = { status: "running", stage: "Starting installed-package consumer" };

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function fail(error: unknown): void {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  window.__packageSmoke = { status: "failed", error: message };
  console.error(message);
}

window.addEventListener("error", (event) => fail(event.error ?? event.message));
window.addEventListener("unhandledrejection", (event) => fail(event.reason));

// Entirely original programs/data, not imports from fixtures or submodules.
// CLS; V0=V1=0; I=$20c; draw one pixel; jump to the jump forever.
const chip8Rom = Uint8Array.of(
  0x00, 0xe0, 0x60, 0x00, 0x61, 0x00, 0xa2, 0x0c,
  0xd0, 0x11, 0x12, 0x0a, 0x80,
);

function gameboyRom(): Uint8Array {
  // ROM-only, 32 KiB, no RAM. The core starts post-boot: no BIOS/logo needed.
  const rom = new Uint8Array(0x8000);
  rom.set([0xf3, 0x18, 0xfe], 0x100); // DI; JR to itself.
  rom.set(new TextEncoder().encode("PACKAGE SMOKE"), 0x134);
  let checksum = 0;
  for (let i = 0x134; i <= 0x14c; i++) checksum = (checksum - rom[i] - 1) & 0xff;
  rom[0x14d] = checksum;
  return rom;
}

function nesRom(): Uint8Array {
  // iNES mapper 0: one 16 KiB PRG bank and CHR RAM.
  const rom = new Uint8Array(16 + 0x4000);
  rom.set([0x4e, 0x45, 0x53, 0x1a, 1, 0]);
  rom.set([0x78, 0x4c, 0x01, 0x80], 16); // SEI; JMP $8001.
  for (const vector of [0x3ffa, 0x3ffc, 0x3ffe]) rom.set([0x00, 0x80], 16 + vector);
  return rom;
}

function mbc3Rom(): Uint8Array {
  // Original synthetic program: report mapper success as a solid shade-1
  // frame, failure as shade-3. No commercial ROM is needed in package CI.
  const rom = new Uint8Array(128 * 0x4000);
  for (let bank = 0; bank < 128; bank++) rom[bank * 0x4000] = bank;
  rom.set([0xf3, 0xc3, 0x50, 0x01], 0x100); // DI; JP $0150
  rom[0x147] = 0x13;
  rom[0x148] = 6;
  rom[0x149] = 3;
  const program: number[] = [];
  const failures: number[] = [];
  const write = (address: number, value: number) => {
    program.push(0x3e, value, 0xea, address & 255, address >> 8);
  };
  const expect = (address: number, value: number) => {
    program.push(0xfa, address & 255, address >> 8, 0xfe, value, 0x28, 3, 0xc3);
    failures.push(program.length);
    program.push(0, 0); // Patched JP failure.
  };
  for (const bank of [0, 1, 0x20, 0x40, 0x60, 0x7f, 0x80, 0xff]) {
    write(0x2000, bank);
    expect(0x4000, Math.max(1, bank & 0x7f));
    expect(0x0000, 0);
  }
  expect(0xa000, 0xff); // RAM starts disabled.
  write(0x0000, 0x0a);
  for (let bank = 0; bank < 4; bank++) {
    write(0x4000, bank);
    write(0xa000, 0x30 + bank);
  }
  for (const bank of [8, 12, 0xff]) {
    write(0x4000, bank);
    expect(0xa000, 0xff);
    write(0xa000, 0xee); // Absent RTC must not overwrite a RAM bank.
  }
  for (let bank = 0; bank < 4; bank++) {
    write(0x4000, bank);
    expect(0xa000, 0x30 + bank);
  }
  write(0x0000, 0);
  write(0xa000, 0xee);
  expect(0xa000, 0xff);
  write(0x0000, 0x0a);
  expect(0xa000, 0x33);
  write(0xff47, 0x55); // All BG palette colors -> shade 1 (success).
  program.push(0x18, 0xfe);
  const failureAddress = 0x150 + program.length;
  write(0xff47, 0xff); // All BG palette colors -> shade 3 (failure).
  program.push(0x18, 0xfe);
  for (const position of failures) {
    program[position] = failureAddress & 255;
    program[position + 1] = failureAddress >> 8;
  }
  rom.set(program, 0x150);
  return rom;
}

function tamagotchiRom(): Uint8Array {
  // 6144 big-endian 12-bit words. $000 is JP 0, a legal self-loop in the
  // reset page ($100), with interrupts disabled by the core's reset state.
  return new Uint8Array(6144 * 2);
}

function framebuffer(core: CoreInstance, id: DeviceId): Uint8Array {
  const pixels = core.framebuffer();
  check(pixels instanceof Uint8Array, id + ": framebuffer must be Uint8Array");
  check(pixels.length === core.width * core.height * 4, id + ": wrong RGBA framebuffer size");
  return pixels;
}

function litPixels(pixels: Uint8Array | Uint8ClampedArray): number {
  let lit = 0;
  for (let i = 0; i < pixels.length; i += 4) {
    if (pixels[i] !== 0 || pixels[i + 1] !== 0 || pixels[i + 2] !== 0) lit++;
  }
  return lit;
}

async function testCores(): Promise<CoreCheck[]> {
  const cases: { id: DeviceId; width: number; height: number; rom: Uint8Array; mbc3?: boolean }[] = [
    { id: "chip8", width: 64, height: 32, rom: chip8Rom },
    { id: "gameboy", width: 160, height: 144, rom: gameboyRom() },
    { id: "gameboy", width: 160, height: 144, rom: mbc3Rom(), mbc3: true },
    { id: "nes", width: 256, height: 240, rom: nesRom() },
    { id: "tamagotchi", width: 32, height: 16, rom: tamagotchiRom() },
  ];
  const results: CoreCheck[] = [];
  for (const item of cases) {
    window.__packageSmoke = { status: "running", stage: "Loading " + item.id };
    const registration = getRegistration(item.id);
    check(REGISTRY[item.id] === registration, item.id + ": registry lookup mismatch");
    check(Number.isFinite(registration.frameHz) && registration.frameHz > 0, item.id + ": invalid frame rate");
    // No custom init argument, URL override, aliases, or direct WASM imports.
    const core: CoreInstance = await registration.load();
    try {
      check(core.width === item.width && core.height === item.height, item.id + ": wrong dimensions");
      framebuffer(core, item.id);
      core.loadRom(item.rom);
      core.setButton(4, true);
      core.setButton(4, false);
      core.stepFrame();
      core.stepFrame();
      const pixels = framebuffer(core, item.id);
      check(core.width === item.width && core.height === item.height, item.id + ": dimensions changed after load");
      check(Number.isFinite(core.sampleRate) && core.sampleRate >= 0, item.id + ": invalid sample rate");
      const audio = core.audio();
      check(audio instanceof Float32Array && audio.length % 2 === 0, item.id + ": invalid stereo audio buffer");
      check(audio.every((sample) => Number.isFinite(sample)), item.id + ": non-finite audio sample");
      if (item.id === "chip8") check(litPixels(pixels) > 0, "CHIP-8 program did not light a pixel");
      if (item.mbc3) {
        check(pixels.every((value, index) => value === [0x88, 0xc0, 0x70, 0xff][index % 4]),
          "MBC3 ROM/RAM banking program did not render its success frame");
        console.info("Installed MBC3 banking program passed");
      }
      results.push({
        id: item.id, width: core.width, height: core.height,
        framebufferBytes: pixels.length, sampleRate: core.sampleRate,
      });
      console.info("Installed core passed: " + item.id);
    } finally {
      // wasm-bindgen provides free(), although it is not part of CoreInstance.
      (core as CoreInstance & { free?: () => void }).free?.();
    }
  }
  return results;
}

function testComponent(): Promise<ComponentCheck> {
  window.__packageSmoke = { status: "running", stage: "Mounting installed ExtraLife" };
  const host = document.getElementById("app");
  check(host, "Missing component mount point");
  const root = createRoot(host);
  return new Promise((resolve, reject) => {
    let frames = 0;
    let finished = false;
    const timer = window.setTimeout(() => {
      finished = true;
      reject(new Error("ExtraLife did not paint a nonblack CHIP-8 frame within 10 seconds"));
    }, 10_000);
    root.render(
      <ExtraLife
        device="chip8"
        rom={chip8Rom}
        data-testid="package-canvas"
        onError={(error) => {
          if (finished) { fail(error); return; }
          finished = true;
          window.clearTimeout(timer);
          reject(error);
        }}
        onFrame={(reportedLitPixels) => {
          if (finished) return;
          frames++;
          if (frames < 3 || reportedLitPixels <= 0) return;
          try {
            const canvas = host.querySelector("canvas");
            check(canvas instanceof HTMLCanvasElement, "ExtraLife did not mount a canvas");
            check(canvas.width === 64 && canvas.height === 32, "ExtraLife canvas has wrong dimensions");
            const context = canvas.getContext("2d");
            check(context, "ExtraLife canvas has no 2D context");
            const lit = litPixels(context.getImageData(0, 0, canvas.width, canvas.height).data);
            check(lit > 0, "ExtraLife canvas is black despite its onFrame callback");
            finished = true;
            window.clearTimeout(timer);
            resolve({ frames, width: canvas.width, height: canvas.height, litPixels: lit });
          } catch (error) {
            finished = true;
            window.clearTimeout(timer);
            reject(error);
          }
        }}
      />,
    );
  });
}

async function main(): Promise<void> {
  const cores = await testCores();
  const component = await testComponent();
  if (window.__packageSmoke.status !== "failed") {
    window.__packageSmoke = { status: "passed", cores, component };
  }
}

void main().catch(fail);
`;

async function writeConsumer(directory, dependency, versions) {
  const files = {
    "package.json": JSON.stringify({
      name: "extralife-package-smoke-consumer",
      version: "0.0.0",
      private: true,
      type: "module",
      dependencies: { [packageName]: dependency, ...versions },
    }, null, 2) + "\n",
    "tsconfig.json": JSON.stringify({
      compilerOptions: {
        target: "ES2022",
        lib: ["ES2022", "DOM", "DOM.Iterable"],
        module: "ESNext",
        moduleResolution: "Bundler",
        jsx: "react-jsx",
        strict: true,
        skipLibCheck: false,
        noEmit: true,
        esModuleInterop: true,
        verbatimModuleSyntax: true,
        forceConsistentCasingInFileNames: true,
        types: ["react", "react-dom"],
      },
      include: ["main.tsx"],
    }, null, 2) + "\n",
    "index.html": '<!doctype html>\n<html><head><meta charset="utf-8"><title>extralife package smoke</title><link rel="icon" href="data:,"></head><body><div id="app"></div><script type="module" src="/main.tsx"></script></body></html>\n',
    "main.tsx": consumerSource,
    "vite.config.mjs": [
      'import { defineConfig } from "vite";',
      "export default defineConfig({",
      "  publicDir: false,",
      '  esbuild: { jsx: "automatic" },',
      '  build: { target: "es2022", assetsInlineLimit: 0, sourcemap: false, emptyOutDir: true },',
      "});",
      "",
    ].join("\n"),
  };
  await Promise.all(Object.entries(files).map(([name, content]) => writeFile(join(directory, name), content)));
}

const contentTypes = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".json", "application/json"],
  [".wasm", "application/wasm"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
]);

async function startServer(directory) {
  const documentRoot = await realpath(directory);
  const server = createServer((request, response) => {
    void (async () => {
      if (request.method !== "GET" && request.method !== "HEAD") {
        response.writeHead(405, { Allow: "GET, HEAD" }).end();
        return;
      }
      let pathname;
      try {
        pathname = decodeURIComponent(new URL(request.url, "http://127.0.0.1").pathname);
      } catch {
        response.writeHead(400).end();
        return;
      }
      const filename = resolve(documentRoot, "." + (pathname === "/" ? "/index.html" : pathname));
      if (pathname.includes("\0") || pathname.includes("\\") || !inside(documentRoot, filename)) {
        response.writeHead(403).end();
        return;
      }
      const actual = await realpath(filename);
      if (!inside(documentRoot, actual)) {
        response.writeHead(403).end();
        return;
      }
      const bytes = await readFile(actual);
      response.writeHead(200, {
        "Content-Type": contentTypes.get(extname(actual)) ?? "application/octet-stream",
        "Content-Length": bytes.length,
        "Cache-Control": "no-store",
        "X-Content-Type-Options": "nosniff",
      });
      response.end(request.method === "HEAD" ? undefined : bytes);
    })().catch((error) => {
      const status = ["ENOENT", "ENOTDIR", "EISDIR"].includes(error.code) ? 404 : 500;
      if (status === 500) console.error("[package-smoke] HTTP server error:", error);
      response.writeHead(status).end();
    });
  });
  server.requestTimeout = 10_000;
  server.headersTimeout = 10_000;
  await new Promise((resolveListening, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveListening);
  });
  const address = server.address();
  assert(address && typeof address !== "string", "HTTP server did not acquire a TCP port");
  return { server, url: "http://127.0.0.1:" + address.port + "/" };
}

async function testBrowser(directory) {
  let server;
  let browser;
  let page;
  const logs = [];
  const failures = [];
  const wasmRequests = new Set();
  const wasmResponses = new Map();
  try {
    const listening = await startServer(join(directory, "dist"));
    server = listening.server;
    console.log("[package-smoke] Launching Chromium (install it with pnpm exec playwright install chromium if missing)");
    browser = await chromium.launch({ headless: true, timeout: timeout(20_000) });
    page = await bounded("Creating browser page", 20_000, () => browser.newPage());
    page.setDefaultTimeout(timeout(30_000));
    page.on("console", (message) => {
      logs.push("[" + message.type() + "] " + message.text());
      if (logs.length > 80) logs.shift();
      if (message.type() === "error") failures.push("Browser console error: " + message.text());
    });
    page.on("pageerror", (error) => failures.push("Browser pageerror: " + (error.stack ?? error.message)));
    page.on("requestfailed", (request) => failures.push("Request failed: " + request.url() + " " + request.failure()?.errorText));
    page.on("request", (request) => {
      if (new URL(request.url()).pathname.endsWith(".wasm")) wasmRequests.add(request.url());
    });
    page.on("response", (response) => {
      if (response.status() >= 400) failures.push("HTTP " + response.status() + ": " + response.url());
      if (new URL(response.url()).pathname.endsWith(".wasm")) {
        wasmResponses.set(response.url(), { status: response.status(), type: response.headers()["content-type"] });
      }
    });
    await page.goto(listening.url, { waitUntil: "load", timeout: timeout(20_000) });
    await page.waitForFunction(
      () => ["passed", "failed"].includes(window.__packageSmoke?.status),
      null,
      { timeout: timeout(30_000) },
    );
    // Also give delayed module/network errors a chance to surface before closing.
    await page.waitForLoadState("networkidle", { timeout: timeout(10_000) });
    const result = await bounded("Reading consumer result", 5_000, () => page.evaluate(() => window.__packageSmoke));
    assert.equal(result?.status, "passed", "Consumer failed:\n" + JSON.stringify(result, null, 2));
    assert.deepEqual(result.cores.map((core) => core.id).sort(), [...devices, "gameboy"].sort());
    assert.equal(wasmRequests.size, 4, "Expected four distinct network WASM requests, got: " + [...wasmRequests].join(", "));
    assert.equal(wasmResponses.size, 4, "Expected four distinct WASM responses");
    for (const [url, response] of wasmResponses) {
      assert(wasmRequests.has(url), "WASM response without a matching request: " + url);
      assert.equal(new URL(url).origin, new URL(listening.url).origin, "WASM was not served by the production app");
      assert(new URL(url).pathname.startsWith("/assets/"), "WASM did not use a Vite-emitted asset URL: " + url);
      assert.equal(response.status, 200, "WASM fetch failed: " + url);
      assert.equal(response.type?.split(";")[0], "application/wasm", "Wrong WASM content type: " + url);
    }
    for (const device of devices) {
      assert(
        [...wasmResponses.keys()].some((url) => basename(new URL(url).pathname).startsWith("extralife_" + device + "_bg")),
        "No emitted WASM asset fetched for " + device,
      );
    }
    assert.equal(failures.length, 0, failures.join("\n"));
    console.log("[package-smoke] Browser passed: " + result.cores.map((core) => core.id + " " + core.width + "x" + core.height).join(", "));
    console.log("[package-smoke] ExtraLife painted " + result.component.frames + " frames; " + result.component.litPixels + " lit pixel(s)");
  } catch (error) {
    if (page && !page.isClosed()) {
      const state = await bounded("Reading failure diagnostics", 2_000, () => page.evaluate(() => window.__packageSmoke)).catch(() => undefined);
      if (state) console.error("[package-smoke] Browser state:", JSON.stringify(state, null, 2));
    }
    if (logs.length) console.error("[package-smoke] Browser console:\n" + logs.join("\n"));
    if (failures.length) console.error("[package-smoke] Browser/network failures:\n" + failures.join("\n"));
    throw error;
  } finally {
    try {
      if (browser) await browser.close();
    } finally {
      if (server) {
        await new Promise((resolveClosed, reject) => {
          server.close((error) => error ? reject(error) : resolveClosed());
          server.closeAllConnections();
        });
      }
    }
  }
}

async function main() {
  const args = process.argv.slice(2);
  assert(
    args.length === 0 || (args.length === 1 && args[0] === "--published"),
    "Unsupported arguments: " + args.join(" ") + "\nUsage: node scripts/test-package.mjs [--published]",
  );
  const published = args[0] === "--published";
  const registryArgs = published ? ["--registry=https://registry.npmjs.org/", "--prefer-online"] : [];
  const versions = toolchainVersions();
  // Canonicalize the parent so installed-path assertions also work in symlinked workspaces.
  const directory = await mkdtemp(join(await realpath(packageDirectory), ".package-smoke-"));
  try {
    const pack = checkPack(await run(published ? "Packing published extralife from the public registry" : "Packing extralife without lifecycle scripts", "npm", [
      "pack", ...(published ? [packageName + "@" + packageVersion] : []), ...registryArgs,
      "--ignore-scripts", "--json", "--pack-destination", directory,
    ], published ? directory : packageDirectory, 30_000, { quiet: true }));
    assert((await lstat(join(directory, pack.filename))).isFile(), "npm pack did not create the reported tarball");
    await writeConsumer(directory, published ? packageVersion : "file:./" + pack.filename, versions);
    await run((published ? "Installing published extralife" : "Installing tarball") + " and exact consumer toolchain " + JSON.stringify(versions), "npm", [
      "install", ...registryArgs, "--ignore-scripts", "--no-audit", "--no-fund", "--package-lock=false",
      "--workspaces=false", "--include=dev", "--fetch-retries=1", "--fetch-timeout=30000",
      "--fetch-retry-mintimeout=1000", "--fetch-retry-maxtimeout=5000",
    ], directory, 150_000);
    const consumerRequire = await checkInstallation(directory, versions);
    const binary = (name, command) => {
      const installed = installedPackage(consumerRequire, name);
      const path = typeof installed.manifest.bin === "string" ? installed.manifest.bin : installed.manifest.bin?.[command];
      assert(typeof path === "string", "Missing installed CLI: " + command);
      return resolve(dirname(installed.filename), path);
    };
    await run("Typechecking standalone consumer (strict, skipLibCheck=false)", process.execPath, [
      binary("typescript", "tsc"), "--project", "tsconfig.json",
    ], directory, 30_000);
    await run("Building standalone Vite production app (no aliases, no asset inlining)", process.execPath, [
      binary("vite", "vite"), "build", "--config", "vite.config.mjs", "--mode", "production",
    ], directory, 30_000, { env: { NODE_ENV: "production" } });
    await testBrowser(directory);
    console.log("[package-smoke] PASS: packed files, required React peer, strict declarations, installed component, and all four fetched WASM cores");
  } finally {
    await rm(directory, { recursive: true, force: true, maxRetries: 2, retryDelay: 100 });
  }
}

await main().catch((error) => {
  console.error("[package-smoke] FAIL:", error);
  process.exitCode = 1;
});
