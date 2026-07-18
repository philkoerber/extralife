#!/usr/bin/env bash
# Initialize the tests/roms submodules for a fresh clone.
#
# Most suites are small and check out normally. ProcessorTests is the exception:
# it's a monorepo of ~10 CPU test suites and a full checkout inflates to several
# GB on disk. We only need nes6502/v1, so we sparse- + shallow-checkout just that
# path. Sparse-checkout is a local working-tree setting that can't be pinned in
# .gitmodules, which is why this lives in a script rather than committed config.
#
# Usage: scripts/setup-submodules.sh   (or: pnpm setup:roms)
set -euo pipefail

cd "$(dirname "$0")/.."

PT="tests/roms/ProcessorTests"

# Init every submodule *except* ProcessorTests the normal (small) way.
git submodule update --init \
  tests/roms/chip8-test-suite \
  tests/roms/sm83 \
  tests/roms/gb-test-roms \
  tests/roms/dmg-acid2 \
  tests/roms/nes-test-roms

# ProcessorTests: init the gitlink, then scope the working tree to nes6502/v1
# via sparse-checkout so the other ~9 CPU suites don't sit on disk. The 185 MiB
# object pack is fetched either way; sparse-checkout controls what's *checked out*.
git submodule update --init --depth 1 "$PT"
git -C "$PT" sparse-checkout set --cone nes6502/v1
git -C "$PT" checkout

echo "submodules ready. ProcessorTests sparse set to: $(git -C "$PT" sparse-checkout list 2>/dev/null | tr '\n' ' ')"
