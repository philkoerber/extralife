# Test assets

Two kinds of files live under here. They are **not** the same thing and the
distinction is a hard legal + correctness rule for the whole project.

## `roms/` — inputs, pulled in as git submodules

Programs we feed *into* a core. These are third-party and come in as
**git submodules**, never copied into this repo. Clone with:

```bash
git submodule update --init --recursive
```

Rules:

- Only submodule sources whose license we may redistribute the *reference* to
  (the suites here are permissively licensed test ROMs / homebrew).
- **Never** a commercial ROM or BIOS, ever, in any form. See the License policy
  in the root `README.md`.
- One submodule per suite, named after its upstream repo.

Current submodules:

- `roms/chip8-test-suite` — https://github.com/Timendus/chip8-test-suite (MIT)

## `golden/` — expected outputs, committed by us

The "golden images": PNG (or raw RGBA) snapshots of the **correct framebuffer**
after running a known ROM for a known number of frames. We generate these
ourselves from a trusted run, eyeball them once, then commit them. CI re-runs
the core and pixel-diffs against them — that diff is the definition of done.

Layout: `golden/<device>/<rom-name>.<frame>.png`, e.g.
`golden/chip8/1-chip8-logo.60.png`.

When a golden image legitimately changes (a real behavior fix), regenerate and
commit the new PNG in the same change, with the reason in the commit message.
On failure, CI writes the actual frame + a diff next to the golden as
`__actual__/` and `__diff__/` (both gitignored) so you can inspect locally.
