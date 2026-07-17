//! Game Boy (DMG) core for extralife.
//!
//! Playbook order: CPU headless first (SingleStepTests/sm83), then MMU +
//! cartridge, timer + interrupts, PPU to a 160x144 RGBA framebuffer diffed
//! against dmg-acid2. APU is stubbed this session.
//!
//! Determinism: `step_frame` runs a fixed T-cycle budget per frame with no
//! wall-clock and no time-seeded RNG, so the same ROM+inputs render identically
//! every run (required for golden-image diffs).

pub mod cpu;

#[cfg(test)]
mod tests;
