# extralife — every old device gets an extra life

A multi-system emulator library for the web: Rust cores compiled to WASM,
wrapped in typed React primitives. Published as **one public npm package**,
[`@philkoerber/extralife`](https://www.npmjs.com/package/@philkoerber/extralife),
built from `packages/extralife`.

**Published release: [`0.1.0`](https://www.npmjs.com/package/@philkoerber/extralife/v/0.1.0).**
See the [maintainer guide below](#publishing-and-releases) for future releases.

Built almost entirely by AI agents using the Bun-rewrite playbook:
machine-checkable tests as the definition of done, agents grinding
autonomously until green.

## Install and use

In a React application with **React >=18** installed (a required peer dependency,
not optional):

```bash
npm install @philkoerber/extralife
```

```tsx
import { ExtraLife } from "@philkoerber/extralife";

export default function Emulator() {
  return (
    <ExtraLife
      device="gameboy"
      rom="/roms/homebrew.gb"
      onError={console.error}
    />
  );
}
```

Optional: to use imports from `"extralife"`, install a local npm alias instead:

```bash
npm install extralife@npm:@philkoerber/extralife
```

This aliases the same scoped package locally. The unscoped name `extralife` is
blocked by npm's similarity rule for the existing `extra-life` package, so bare
`npm install extralife` does **not** install this library.

Serve your own legally distributable ROM at that URL; no ROM is included.
The `rom` prop accepts a URL string, `Uint8Array`, or `ArrayBuffer` — **not a
`File`**. Convert a selected file with `await file.arrayBuffer()` first.

Focus the canvas to use the default keyboard controls: arrows for the D-pad,
Z/X for B/A, Enter for Start, and Right Shift for Select. After loading,
click/tap the canvas or press a key while it is focused to unlock browser audio.

The package is **ESM-only**, targeting modern browsers and bundlers with
WebAssembly, dynamic imports, Canvas 2D, and Web Audio support. Vite consumers
need no aliases, WASM plugins/configuration, Rust toolchain, or separate core
packages: the selected device's bundled WASM loads automatically and lazily via
portable relative imports. SSR frameworks must use a client component (for
example, a Next.js component with `"use client"`); emulation runs in the browser.
See the [package README](packages/extralife/README.md) for props and local-file usage.

## Supported devices

| Device ID | System |
| --- | --- |
| `chip8` | CHIP-8 |
| `gameboy` | Game Boy (DMG) |
| `nes` | NES |
| `tamagotchi` | Tamagotchi |

Only these four devices are supported. Game Boy Color (GBC) and the other
systems in the roadmap are planned, not included cores.

## Architecture

- **Core contract** (`extralife-core`, Rust trait): deterministic, headless
  emulation with ROM loading, frame stepping, RGBA framebuffer output, audio,
  and button input.
- **One crate per device** (`extralife-chip8`, `extralife-gameboy`,
  `extralife-nes`, `extralife-tamagotchi`), each compiled to a lazy-loaded WASM module.
- **One npm package** (`@philkoerber/extralife`): the `<ExtraLife>` React component,
  lower-level playback/registry/input helpers, TypeScript declarations, and all
  four WASM cores. There are no separate npm core packages to install or publish.

Higher-level hooks such as `useFramebuffer`, `useSaveStates`, and `useRewind`,
and a live documentation site, remain planned rather than shipped APIs/tools.

## Development

Repository build prerequisites (not consumer requirements):

- Node.js **>=22** and **pnpm@10.33.0**.
- Rust and the `wasm32-unknown-unknown` target.
- **wasm-pack 0.15.0**, installed with the pinned command below.
- Chromium for the package browser smoke test, installed through Playwright below.

```bash
npm install --global pnpm@10.33.0
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --version 0.15.0 --locked

git clone https://github.com/philkoerber/extralife.git
cd extralife
pnpm install

# Package-only build, including all four WASM cores and their generated types.
pnpm --filter @philkoerber/extralife build

# Source typechecks/tests require the built WASM types: build first.
pnpm typecheck
pnpm test

pnpm exec playwright install chromium
pnpm test:package
```

Build and check commands:

| Command | Purpose |
| --- | --- |
| `pnpm build:wasm` | Build all four WASM cores and declarations into `packages/extralife/wasm/`. |
| `pnpm --filter @philkoerber/extralife build` | Build the publishable package, including WASM. Recommended for package-only work. |
| `pnpm build` | Build all workspace packages, including the smoke harness. |
| `pnpm typecheck` / `pnpm test` | Run workspace source typechecks/tests after a package build. |
| `pnpm test:package` | Build the package, then install its actual tarball in an isolated Vite consumer and run browser tests. |
| `node scripts/test-package.mjs --published` | Download the exact published version from npm and run the same isolated consumer checks. |

The WASM build uses `wasm-pack --no-pack` and removes its generated `.gitignore`
files so npm includes the assets. Package ESM and declarations are emitted to
`packages/extralife/dist/`.

The JavaScript job in [CI](.github/workflows/ci.yml) builds the package before
typechecking, then runs source tests and the packaged browser smoke test.

The workspace smoke harness still uses test-ROM submodules, so initialize them
before `pnpm build`. They are not needed for the filtered package build.
For the Rust CPU/ROM/golden tests:

```bash
pnpm setup:roms
cargo test --workspace
```

Test inputs live in `tests/roms/` git submodules (no commercial ROMs or BIOS
files). Most are small, but **ProcessorTests** is a monorepo of ~10 CPU test
suites; a full checkout inflates to several GB on disk. We only need
`nes6502/v1`, so `pnpm setup:roms` sparse-checks-out just that path. A plain
`git submodule update --init --recursive` also works but materializes every suite
— prefer the script. Third-party test inputs retain their own licenses.

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

## npm package contents

Publish **only `packages/extralife`** as the public npm package
**`@philkoerber/extralife`**, not the private workspace root or individual Rust
crates. The package ships compiled ESM, TypeScript declarations, and WASM
glue/binaries together:

```text
packages/extralife/
  dist/*.js
  dist/*.d.ts
  wasm/<device>/extralife_<device>.js
  wasm/<device>/extralife_<device>.d.ts
  wasm/<device>/extralife_<device>_bg.wasm
  README.md
  LICENSE
```

Here `<device>` is `chip8`, `gameboy`, `nes`, or `tamagotchi`. All four cores
ship in the same tarball; only the selected core is loaded at runtime. ROMs,
BIOS files, and the repository smoke harness are not package contents.

## Publishing and releases

### Manual publishing

Version `0.1.0` is already published and cannot be overwritten. For a new release,
[bump the package version first](#subsequent-releases); do not republish `0.1.0`
or push a `v0.1.0` tag that would trigger another publish of that version.

Before publishing locally, [enable two-factor authentication on your npm
account](https://docs.npmjs.com/configuring-two-factor-authentication/): on
npmjs.com, open **Account → Two-Factor Authentication → Enable 2FA**, register
a security key/passkey (such as Touch ID), and save your recovery codes securely.
A normal CLI login alone does not satisfy npm's publishing requirement.

Use a current npm CLI for browser-based security-key challenges. On Node.js
22.14+ or 24, update with `npm install --global npm@11` if needed, then refresh
your login with `npm login --auth-type=web`. Complete any browser authentication
prompts during login and publishing. Keep credentials and recovery codes out of
the repository.

Run the build/checks above, then inspect the tarball contents and publish from
the repository root:

```bash
npm pack ./packages/extralife --dry-run
npm login --auth-type=web
npm publish ./packages/extralife --access public
```

If npm returns `E403` with “Two-factor authentication or granular access token
with bypass 2fa enabled is required,” complete the account setup and refresh
the login above. A rejected publish does not require a version bump; retry the
same version if it has not been published successfully.

Use an npm account authorized to publish **`@philkoerber/extralife`** in the
**`@philkoerber`** scope. Keep `--access public` for this public scoped package.
The `prepack` hook automatically builds when packing or publishing. Use this
manual flow until trusted publishing is configured; no npm token needs to be
stored in the repository.

After a successful first publish (or a later release), run:

```bash
node scripts/test-package.mjs --published
```

This downloads and installs the exact package `name@version` from the public npm
registry, then repeats the archive, types, Vite, and browser core tests instead
of using a local tarball. **Do not run before that version is published.** npm
may need a few minutes after accepting a publish before the version is available
to install.

### Configure trusted publishing

After the initial local publish of `@philkoerber/extralife`, configure both
GitHub and npm:

1. Create a GitHub environment named **`npm`** in
   **[`philkoerber/extralife`](https://github.com/philkoerber/extralife)**, with any
   desired approval/protection rules.
2. In **`@philkoerber/extralife`**'s npm trusted-publisher settings, select
   GitHub Actions and enter owner **`philkoerber`**, repository **`extralife`**,
   workflow filename **`release.yml`**, and environment **`npm`**.
3. Under **Allowed actions**, explicitly select **Allow npm publish**. New
   configurations since September 3, 2026 default to allowing only
   `npm stage publish`; this workflow uses direct `npm publish`. No
   `npm dist-tag` permission is required for this publish command.

Configure the npm trusted publisher **shortly before the first automated
release**: its first successful publish must complete within **2 days of
creation** or it expires. If it expires, delete and recreate it.

The [release workflow](.github/workflows/release.yml) runs on **`v*` tags** and
rejects tags that do not exactly match `v` plus the version in
`packages/extralife/package.json`. It uses the **`npm`** GitHub environment,
**Node.js 24 / npm 11**, and **`contents: read`** / **`id-token: write`** permissions
to publish through npm trusted publishing (**OIDC**).

npm automatically generates provenance for OIDC publishing from a **public
GitHub repository**. Leave `--provenance` unset so a private source repository
can also publish the public npm package, without provenance. Do not add
`NPM_TOKEN` or other publishing credentials to the repository or workflow secrets.
See the [official npm trusted-publishing documentation](https://docs.npmjs.com/trusted-publishers/).

### Subsequent releases

To prepare the next release of `@philkoerber/extralife`, bump its version:

```bash
npm --prefix packages/extralife version patch --no-git-tag-version
```

Review and commit the version change and any corresponding lockfile/metadata
updates, then run the checks above. If trusted publishing is configured, create
and push a matching version tag
(for example, **`v0.1.1`** only when the package version is **`0.1.1`**). The
release workflow builds and tests the package, then publishes from
`packages/extralife`. Otherwise, follow the manual publishing steps above.
Each release needs a new npm version; published versions cannot be overwritten.

## License policy (strict)

This library is [MIT-licensed](LICENSE), copyright 2026 extralife contributors.
The same license is included in the npm package. Therefore:

- **Port freely:** ISC/MIT sources only — [ares](https://github.com/ares-emulator/ares) (ISC),
  [SameBoy](https://github.com/LIJI32/SameBoy) (MIT/Expat).
- **Reference only, never translate:** GPL/MPL sources (Mesen 2, mGBA,
  Stella, blastem, tamalib).
- **Do not touch:** non-commercial licenses (snes9x, Genesis Plus GX).

## Legal guardrails

- **Never distribute commercial ROMs or BIOS files** in this repository, the
  npm package, or demos. Use only homebrew, test ROMs, or public-domain titles
  whose licenses permit the intended use and redistribution; retain their notices.
- The MIT license covers the library, not rights to third-party software or
  content. Users must supply ROMs they are legally entitled to use.
- No copy-protection circumvention, anywhere.
