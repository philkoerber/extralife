//! Audio Processing Unit (2A03 APU): 2 pulse + triangle + noise channels, the
//! frame-sequencer, length/envelope/sweep units, and a fixed-rate stereo (mono
//! duplicated) f32 output stream for the Web Audio path.
//!
//! The APU is clocked once per CPU cycle. Channel timers, the frame counter,
//! and the length/sweep/envelope units run off that clock; a fractional
//! accumulator downsamples the ~1.79 MHz mix to `OUTPUT_RATE` and pushes stereo
//! pairs, matching the Game Boy core's per-frame `samples()` contract.
//!
//! Clean-room from the NESdev wiki (APU, APU Pulse/Triangle/Noise, APU Frame
//! Counter, APU Mixer). Nonlinear mixing uses the published lookup formulas.
//!
//! ponytail: the DMC (sample) channel is stubbed — its output is silent and its
//! IRQ never fires, and DMC's CPU-stealing DMA cycles are not modeled. Ceiling:
//! games using sampled drums/voice lose that channel and any DMC-IRQ timing
//! test fails. Upgrade path: add the DMC timer + sample buffer + memory reader.

use crate::cartridge::Cartridge;

pub const OUTPUT_RATE: u32 = 48000;
/// NTSC CPU frequency (Hz). The APU is clocked at the CPU rate.
const CPU_HZ: f64 = 1_789_773.0;

/// Length-counter load values indexed by the 5-bit length field.
#[rustfmt::skip]
const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
];

/// Duty-cycle waveforms for the pulse channels (8 steps each).
const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

/// Triangle 32-step ramp.
const TRIANGLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

/// Noise timer periods (NTSC).
const NOISE_PERIODS: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

#[derive(Default)]
struct Envelope {
    start: bool,
    loop_flag: bool,
    constant: bool,
    volume: u8,
    divider: u8,
    decay: u8,
}
impl Envelope {
    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.volume;
        } else if self.divider == 0 {
            self.divider = self.volume;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.loop_flag {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }
    fn output(&self) -> u8 {
        if self.constant {
            self.volume
        } else {
            self.decay
        }
    }
}

#[derive(Default)]
struct Pulse {
    enabled: bool,
    duty: u8,
    duty_step: u8,
    timer: u16,
    timer_period: u16,
    length: u8,
    length_halt: bool,
    env: Envelope,
    // Sweep unit.
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_reload: bool,
    sweep_divider: u8,
    /// Ones-complement (pulse 1) vs twos-complement (pulse 2) negate.
    ones_complement: bool,
}
impl Pulse {
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.duty_step = (self.duty_step + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }
    fn target_period(&self) -> u16 {
        let change = self.timer_period >> self.sweep_shift;
        if self.sweep_negate {
            if self.ones_complement {
                self.timer_period.wrapping_sub(change).wrapping_sub(1)
            } else {
                self.timer_period.wrapping_sub(change)
            }
        } else {
            self.timer_period.wrapping_add(change)
        }
    }
    fn sweep_muted(&self) -> bool {
        self.timer_period < 8 || self.target_period() > 0x7FF
    }
    fn clock_sweep(&mut self) {
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift > 0 && !self.sweep_muted() {
            self.timer_period = self.target_period();
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }
    fn output(&self) -> u8 {
        if !self.enabled
            || self.length == 0
            || self.sweep_muted()
            || DUTY[self.duty as usize][self.duty_step as usize] == 0
        {
            0
        } else {
            self.env.output()
        }
    }
}

#[derive(Default)]
struct Triangle {
    enabled: bool,
    timer: u16,
    timer_period: u16,
    step: u8,
    length: u8,
    length_halt: bool,
    linear: u8,
    linear_reload_value: u8,
    linear_reload: bool,
}
impl Triangle {
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            if self.length > 0 && self.linear > 0 {
                self.step = (self.step + 1) & 31;
            }
        } else {
            self.timer -= 1;
        }
    }
    fn output(&self) -> u8 {
        if !self.enabled || self.timer_period < 2 {
            // Ultrasonic periods produce a steady value; report the current step.
            TRIANGLE[self.step as usize]
        } else {
            TRIANGLE[self.step as usize]
        }
    }
}

#[derive(Default)]
struct Noise {
    enabled: bool,
    mode: bool,
    shift: u16,
    timer: u16,
    timer_period: u16,
    length: u8,
    length_halt: bool,
    env: Envelope,
}
impl Noise {
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            let bit = if self.mode { 6 } else { 1 };
            let feedback = (self.shift & 1) ^ ((self.shift >> bit) & 1);
            self.shift >>= 1;
            self.shift |= feedback << 14;
        } else {
            self.timer -= 1;
        }
    }
    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || self.shift & 1 != 0 {
            0
        } else {
            self.env.output()
        }
    }
}

pub struct Apu {
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,

    frame_mode_5step: bool,
    frame_irq_inhibit: bool,
    frame_irq: bool,
    frame_counter: u32,

    sample_accum: f64,
    samples: Vec<f32>,
}

impl Default for Apu {
    fn default() -> Self {
        let mut noise = Noise::default();
        noise.shift = 1;
        Apu {
            pulse1: Pulse { ones_complement: true, ..Default::default() },
            pulse2: Pulse::default(),
            triangle: Triangle::default(),
            noise,
            frame_mode_5step: false,
            frame_irq_inhibit: false,
            frame_irq: false,
            frame_counter: 0,
            sample_accum: 0.0,
            samples: Vec::new(),
        }
    }
}

impl Apu {
    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
    pub fn irq_pending(&self) -> bool {
        self.frame_irq
    }

    /// One CPU-cycle APU tick: triangle clocks every CPU cycle; pulses/noise
    /// clock every *other* CPU cycle (APU cycle). Frame sequencer + resampling.
    pub fn tick(&mut self, _cart: &Cartridge) {
        // Triangle timer runs at CPU rate.
        self.triangle.clock_timer();
        // Pulse/noise run at half rate.
        if self.frame_counter % 2 == 0 {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
            self.noise.clock_timer();
        }
        self.clock_frame_sequencer();
        self.frame_counter = self.frame_counter.wrapping_add(1);

        // Downsample to OUTPUT_RATE.
        self.sample_accum += OUTPUT_RATE as f64;
        if self.sample_accum >= CPU_HZ {
            self.sample_accum -= CPU_HZ;
            let s = self.mix();
            self.samples.push(s);
            self.samples.push(s);
        }
    }

    fn mix(&self) -> f32 {
        let p1 = self.pulse1.output() as f32;
        let p2 = self.pulse2.output() as f32;
        let t = self.triangle.output() as f32;
        let n = self.noise.output() as f32;
        // NESdev nonlinear mixer approximation.
        let pulse_out = if p1 + p2 == 0.0 {
            0.0
        } else {
            95.88 / (8128.0 / (p1 + p2) + 100.0)
        };
        let tnd = t / 8227.0 + n / 12241.0;
        let tnd_out = if tnd == 0.0 {
            0.0
        } else {
            159.79 / (1.0 / tnd + 100.0)
        };
        (pulse_out + tnd_out) * 2.0 - 1.0
    }

    /// The frame counter divides the CPU clock into 4 (or 5) steps that clock
    /// the length/sweep and envelope/linear units. We approximate its exact
    /// timing with cycle thresholds at the documented NTSC counts.
    fn clock_frame_sequencer(&mut self) {
        // Quarter-frame ~ every 3729 APU cycles; use CPU-cycle thresholds.
        const Q1: u32 = 7457;
        const Q2: u32 = 14913;
        const Q3: u32 = 22371;
        const Q4_4STEP: u32 = 29829;
        const Q4_5STEP: u32 = 37281;

        let c = self.frame_counter;
        if !self.frame_mode_5step {
            match c {
                x if x == Q1 => self.quarter_frame(),
                x if x == Q2 => {
                    self.quarter_frame();
                    self.half_frame();
                }
                x if x == Q3 => self.quarter_frame(),
                x if x == Q4_4STEP => {
                    self.quarter_frame();
                    self.half_frame();
                    if !self.frame_irq_inhibit {
                        self.frame_irq = true;
                    }
                    self.frame_counter = u32::MAX; // wraps to 0 next increment
                }
                _ => {}
            }
        } else {
            match c {
                x if x == Q1 => self.quarter_frame(),
                x if x == Q2 => {
                    self.quarter_frame();
                    self.half_frame();
                }
                x if x == Q3 => self.quarter_frame(),
                x if x == Q4_5STEP => {
                    self.quarter_frame();
                    self.half_frame();
                    self.frame_counter = u32::MAX;
                }
                _ => {}
            }
        }
    }

    fn quarter_frame(&mut self) {
        // Envelopes + triangle linear counter.
        self.pulse1.env.clock();
        self.pulse2.env.clock();
        self.noise.env.clock();
        if self.triangle.linear_reload {
            self.triangle.linear = self.triangle.linear_reload_value;
        } else if self.triangle.linear > 0 {
            self.triangle.linear -= 1;
        }
        if !self.triangle.length_halt {
            self.triangle.linear_reload = false;
        }
    }

    fn half_frame(&mut self) {
        // Length counters + sweeps.
        if !self.pulse1.length_halt && self.pulse1.length > 0 {
            self.pulse1.length -= 1;
        }
        if !self.pulse2.length_halt && self.pulse2.length > 0 {
            self.pulse2.length -= 1;
        }
        if !self.triangle.length_halt && self.triangle.length > 0 {
            self.triangle.length -= 1;
        }
        if !self.noise.length_halt && self.noise.length > 0 {
            self.noise.length -= 1;
        }
        self.pulse1.clock_sweep();
        self.pulse2.clock_sweep();
    }

    pub fn read_status(&mut self) -> u8 {
        let mut s = 0;
        if self.pulse1.length > 0 { s |= 0x01; }
        if self.pulse2.length > 0 { s |= 0x02; }
        if self.triangle.length > 0 { s |= 0x04; }
        if self.noise.length > 0 { s |= 0x08; }
        if self.frame_irq { s |= 0x40; }
        self.frame_irq = false; // reading clears the frame IRQ flag
        s
    }

    pub fn write_reg(&mut self, addr: u16, val: u8) {
        match addr {
            0x4000 => self.write_pulse_ctrl(true, val),
            0x4001 => self.write_pulse_sweep(true, val),
            0x4002 => self.pulse1.timer_period = (self.pulse1.timer_period & 0x700) | val as u16,
            0x4003 => self.write_pulse_hi(true, val),
            0x4004 => self.write_pulse_ctrl(false, val),
            0x4005 => self.write_pulse_sweep(false, val),
            0x4006 => self.pulse2.timer_period = (self.pulse2.timer_period & 0x700) | val as u16,
            0x4007 => self.write_pulse_hi(false, val),
            0x4008 => {
                self.triangle.length_halt = val & 0x80 != 0;
                self.triangle.linear_reload_value = val & 0x7F;
            }
            0x400A => self.triangle.timer_period = (self.triangle.timer_period & 0x700) | val as u16,
            0x400B => {
                self.triangle.timer_period =
                    (self.triangle.timer_period & 0xFF) | ((val as u16 & 0x07) << 8);
                if self.triangle.enabled {
                    self.triangle.length = LENGTH_TABLE[(val >> 3) as usize];
                }
                self.triangle.linear_reload = true;
            }
            0x400C => {
                self.noise.length_halt = val & 0x20 != 0;
                self.noise.env.loop_flag = val & 0x20 != 0;
                self.noise.env.constant = val & 0x10 != 0;
                self.noise.env.volume = val & 0x0F;
            }
            0x400E => {
                self.noise.mode = val & 0x80 != 0;
                self.noise.timer_period = NOISE_PERIODS[(val & 0x0F) as usize];
            }
            0x400F => {
                if self.noise.enabled {
                    self.noise.length = LENGTH_TABLE[(val >> 3) as usize];
                }
                self.noise.env.start = true;
            }
            0x4015 => {
                self.pulse1.enabled = val & 0x01 != 0;
                self.pulse2.enabled = val & 0x02 != 0;
                self.triangle.enabled = val & 0x04 != 0;
                self.noise.enabled = val & 0x08 != 0;
                if !self.pulse1.enabled { self.pulse1.length = 0; }
                if !self.pulse2.enabled { self.pulse2.length = 0; }
                if !self.triangle.enabled { self.triangle.length = 0; }
                if !self.noise.enabled { self.noise.length = 0; }
            }
            0x4017 => {
                self.frame_mode_5step = val & 0x80 != 0;
                self.frame_irq_inhibit = val & 0x40 != 0;
                if self.frame_irq_inhibit {
                    self.frame_irq = false;
                }
                self.frame_counter = 0;
                if self.frame_mode_5step {
                    // 5-step mode clocks quarter+half immediately.
                    self.quarter_frame();
                    self.half_frame();
                }
            }
            _ => {}
        }
    }

    fn write_pulse_ctrl(&mut self, first: bool, val: u8) {
        let p = if first { &mut self.pulse1 } else { &mut self.pulse2 };
        p.duty = val >> 6;
        p.length_halt = val & 0x20 != 0;
        p.env.loop_flag = val & 0x20 != 0;
        p.env.constant = val & 0x10 != 0;
        p.env.volume = val & 0x0F;
    }
    fn write_pulse_sweep(&mut self, first: bool, val: u8) {
        let p = if first { &mut self.pulse1 } else { &mut self.pulse2 };
        p.sweep_enabled = val & 0x80 != 0;
        p.sweep_period = (val >> 4) & 0x07;
        p.sweep_negate = val & 0x08 != 0;
        p.sweep_shift = val & 0x07;
        p.sweep_reload = true;
    }
    fn write_pulse_hi(&mut self, first: bool, val: u8) {
        let enabled = if first { self.pulse1.enabled } else { self.pulse2.enabled };
        let p = if first { &mut self.pulse1 } else { &mut self.pulse2 };
        p.timer_period = (p.timer_period & 0xFF) | ((val as u16 & 0x07) << 8);
        if enabled {
            p.length = LENGTH_TABLE[(val >> 3) as usize];
        }
        p.duty_step = 0;
        p.env.start = true;
    }
}
