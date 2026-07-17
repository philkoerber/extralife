//! Audio Processing Unit (DMG) per Pandocs, Gekkio's docs, and the behavior
//! probed by Blargg's `dmg_sound` suite.
//!
//! Four channels: CH1 pulse+sweep, CH2 pulse, CH3 32-sample 4-bit wave, CH4
//! noise (15-bit LFSR). A 512 Hz frame sequencer clocks length counters (256
//! Hz), volume envelopes (64 Hz), and CH1's frequency sweep (128 Hz). Each
//! channel produces a 0..15 digital sample; NR51 pans them to L/R and NR50
//! scales the two master volumes. We accumulate the analog mix per T-cycle and
//! resample to a fixed output rate deterministically (a fractional counter, no
//! wall-clock), so `audio()` is byte-identical for a given ROM + inputs.
//!
//! Clean-room: implemented from Pandocs (https://gbdev.io/pandocs/#audio) and
//! the Game Boy sound docs; SameBoy (MIT) was available as a reference but no
//! code/structure was copied.
//!
//! ponytail: the frame sequencer runs off its own T-cycle counter rather than
//! being driven by DIV bit 4 falling edges. Ceiling: the "length/sweep period
//! sync after DIV write" obscure tests (dmg_sound 07/08 exercise this) can be
//! off by one step because writing DIV doesn't perturb our sequencer phase.
//! Upgrade path: clock `step_frame_sequencer` from the timer's DIV bit-4
//! falling edge and reset the phase on APU power-on.

/// CPU clock (T-cycles/sec). The frame sequencer ticks every CPU_HZ/512 cycles.
const CPU_HZ: u32 = 4_194_304;
const FRAME_SEQ_PERIOD: u32 = CPU_HZ / 512; // 8192 T-cycles per sequencer step
/// Fixed output rate for `audio()`. 48 kHz is the Web Audio default.
pub const OUTPUT_RATE: u32 = 48_000;

/// A length counter shared by all four channels: counts down at 256 Hz and
/// disables the channel when it reaches zero (only while its enable bit is set).
#[derive(Clone, Copy, Default)]
struct Length {
    counter: u16,
    enabled: bool,
    /// Max is 64 for pulse/noise/wave-volume channels, 256 for CH3 wave.
    max: u16,
}

impl Length {
    fn new(max: u16) -> Self {
        Length { counter: 0, enabled: false, max }
    }

    /// Reload the counter from a register write of the "initial length" field.
    fn set(&mut self, value: u16) {
        self.counter = self.max - value;
    }

    /// Called by the frame sequencer at 256 Hz. Returns true if it just
    /// clocked the channel to zero (caller disables the channel).
    fn tick(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            return self.counter == 0;
        }
        false
    }
}

/// Volume envelope (CH1/CH2/CH4): steps volume up or down every `period`
/// envelope clocks (64 Hz), holding at 0 or 15.
#[derive(Clone, Copy, Default)]
struct Envelope {
    /// Initial volume + direction latched from NRx2 at trigger.
    initial: u8,
    add: bool,
    period: u8,
    /// Live volume 0..15 and the down-counter until the next step.
    volume: u8,
    timer: u8,
}

impl Envelope {
    /// NRx2 write: bits 7-4 initial vol, bit 3 direction (1=increase),
    /// bits 2-0 period. A period of 0 means "no automatic envelope".
    fn write(&mut self, val: u8) {
        self.initial = val >> 4;
        self.add = val & 0x08 != 0;
        self.period = val & 0x07;
    }

    /// A channel's DAC is on iff NRx2 top 5 bits are nonzero (init vol or add).
    fn dac_on(&self) -> bool {
        self.initial != 0 || self.add
    }

    fn trigger(&mut self) {
        self.volume = self.initial;
        self.timer = if self.period == 0 { 8 } else { self.period };
    }

    /// Called by the frame sequencer at 64 Hz.
    fn tick(&mut self) {
        if self.period == 0 {
            return;
        }
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = self.period;
            if self.add && self.volume < 15 {
                self.volume += 1;
            } else if !self.add && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

/// The 8-step duty patterns (NRx1 bits 7-6). 1 = high (amplitude on).
const DUTY: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

/// A square/pulse channel. CH1 additionally has a frequency sweep (`sweep`).
#[derive(Clone, Copy)]
struct Pulse {
    enabled: bool,
    dac_on: bool,
    duty: u8,
    /// 11-bit frequency (NRx3 = low 8, NRx4 bits 2-0 = high 3).
    freq: u16,
    /// Frequency timer (T-cycles until the next duty step) and duty phase 0..7.
    timer: u16,
    phase: u8,
    length: Length,
    env: Envelope,
    sweep: Option<Sweep>,
}

/// CH1 frequency sweep (NR10): shifts `freq` toward a new value every `period`
/// sweep clocks (128 Hz), disabling the channel if it overflows 2047.
#[derive(Clone, Copy, Default)]
struct Sweep {
    period: u8,
    negate: bool,
    shift: u8,
    // Live sweep state.
    enabled: bool,
    shadow: u16,
    timer: u8,
    /// Set once a calculation used negate; clearing negate afterward disables
    /// the channel (the "overflow on trigger" obscure behavior).
    negate_used: bool,
}

impl Pulse {
    fn new(with_sweep: bool) -> Self {
        Pulse {
            enabled: false,
            dac_on: false,
            duty: 0,
            freq: 0,
            timer: 0,
            phase: 0,
            length: Length::new(64),
            env: Envelope::default(),
            sweep: if with_sweep { Some(Sweep::default()) } else { None },
        }
    }

    /// Advance the frequency timer one T-cycle, stepping the duty phase.
    fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = (2048 - self.freq) * 4;
            self.phase = (self.phase + 1) & 7;
        }
    }

    /// Current 0..15 digital output (0 when off / DAC off).
    fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_on {
            return 0;
        }
        if DUTY[self.duty as usize][self.phase as usize] == 1 {
            self.env.volume
        } else {
            0
        }
    }

    fn trigger(&mut self) {
        self.enabled = true;
        // Length reload is handled by the APU-level write path (DMG quirks).
        self.timer = (2048 - self.freq) * 4;
        self.env.trigger();
        if !self.dac_on {
            self.enabled = false;
        }
        if let Some(sw) = self.sweep.as_mut() {
            sw.shadow = self.freq;
            sw.timer = if sw.period == 0 { 8 } else { sw.period };
            sw.enabled = sw.period != 0 || sw.shift != 0;
            sw.negate_used = false;
            // An initial overflow check runs immediately when shift != 0.
            if sw.shift != 0 {
                let (_, overflow) = sw.next_freq();
                if overflow {
                    self.enabled = false;
                }
            }
        }
    }

    /// Frame-sequencer sweep step (128 Hz). Returns false if the channel must
    /// be disabled by a frequency overflow.
    fn tick_sweep(&mut self) -> bool {
        let Some(sw) = self.sweep.as_mut() else { return true };
        if sw.timer > 0 {
            sw.timer -= 1;
        }
        if sw.timer != 0 {
            return true;
        }
        sw.timer = if sw.period == 0 { 8 } else { sw.period };
        if !sw.enabled || sw.period == 0 {
            return true;
        }
        let (new_freq, overflow) = sw.next_freq();
        if overflow {
            return false;
        }
        if sw.shift != 0 && new_freq <= 2047 {
            sw.shadow = new_freq;
            self.freq = new_freq;
            // A second calculation checks overflow again (discarded result).
            if sw.next_freq().1 {
                return false;
            }
        }
        true
    }
}

impl Sweep {
    /// Compute the candidate next frequency; second field is overflow (>2047).
    fn next_freq(&mut self) -> (u16, bool) {
        let delta = self.shadow >> self.shift;
        let new = if self.negate {
            self.negate_used = true;
            self.shadow.wrapping_sub(delta)
        } else {
            self.shadow + delta
        };
        (new & 0x7FF, new > 2047)
    }
}

/// CH3: plays 32 4-bit samples from wave RAM, at a volume shifted by NR32.
#[derive(Clone)]
struct Wave {
    enabled: bool,
    dac_on: bool,
    freq: u16,
    timer: u16,
    /// Position 0..31 into the 4-bit sample stream.
    position: u8,
    /// NR32 bits 6-5: volume code (0=mute, 1=100%, 2=50%, 3=25%).
    volume_code: u8,
    length: Length,
    /// 16 bytes = 32 nibbles of wave RAM (0xFF30-0xFF3F).
    ram: [u8; 16],
    /// Last nibble read, so a sample is available immediately after trigger.
    sample_buffer: u8,
}

impl Default for Wave {
    fn default() -> Self {
        Wave {
            enabled: false,
            dac_on: false,
            freq: 0,
            timer: 0,
            position: 0,
            volume_code: 0,
            length: Length::new(256),
            ram: [0; 16],
            sample_buffer: 0,
        }
    }
}

impl Wave {
    /// CPU read of wave RAM.
    ///
    /// ponytail: the DMG "wave RAM is only CPU-accessible during the APU's
    /// single read cycle" quirk is not modeled — Blargg `09-wave read while
    /// on`, `10-wave trigger while on`, and `12-wave write while on` probe it
    /// and are skipped (see tests/sound.rs). Ceiling: our APU ticks four
    /// T-cycles as a batch inside one bus M-cycle, so we can't phase-align the
    /// CPU's exact read T-cycle with the APU's wave fetch. Upgrade path: drive
    /// the APU one T-cycle at a time interleaved with the CPU bus access and
    /// gate access on the exact fetch cycle. Normal games read/write wave RAM
    /// while CH3 is off, so this simple always-accessible model is correct for
    /// them.
    fn read_ram(&self, index: usize) -> u8 {
        self.ram[index]
    }

    fn write_ram(&mut self, index: usize, val: u8) {
        self.ram[index] = val;
    }

    /// CH3 steps through wave RAM at (2048-freq)*2 T-cycles per nibble.
    fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = (2048 - self.freq) * 2;
            self.position = (self.position + 1) & 31;
            let byte = self.ram[(self.position / 2) as usize];
            self.sample_buffer = if self.position & 1 == 0 { byte >> 4 } else { byte & 0x0F };
        }
    }

    fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_on {
            return 0;
        }
        match self.volume_code {
            0 => 0,
            1 => self.sample_buffer,
            2 => self.sample_buffer >> 1,
            _ => self.sample_buffer >> 2,
        }
    }

    fn trigger(&mut self) {
        self.enabled = true;
        self.timer = (2048 - self.freq) * 2;
        self.position = 0;
        if !self.dac_on {
            self.enabled = false;
        }
    }
}

/// CH4: pseudo-random noise from a 15-bit LFSR clocked by a divisor/shift.
#[derive(Clone, Copy, Default)]
struct Noise {
    enabled: bool,
    dac_on: bool,
    timer: u16,
    /// 15-bit linear-feedback shift register; starts all-ones on trigger.
    lfsr: u16,
    /// NR43: clock shift (bits 7-4), width mode (bit 3), divisor code (2-0).
    clock_shift: u8,
    width_7bit: bool,
    divisor_code: u8,
    length: Length,
    env: Envelope,
}

impl Noise {
    fn new() -> Self {
        Noise { length: Length::new(64), ..Noise::default() }
    }

    /// The base divisor from NR43 bits 2-0 (0 maps to 8).
    fn divisor(&self) -> u16 {
        match self.divisor_code {
            0 => 8,
            n => (n as u16) * 16,
        }
    }

    fn period(&self) -> u16 {
        self.divisor() << self.clock_shift
    }

    fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = self.period();
            // Clock shifts of 14/15 leave the LFSR unclocked (obscure behavior).
            if self.clock_shift >= 14 {
                return;
            }
            // XNOR of the low two bits feeds bit 15 (and bit 7 in short mode),
            // then the register shifts right. (Pandocs "Noise channel".)
            let xnor = ((self.lfsr & 1) ^ ((self.lfsr >> 1) & 1)) ^ 1;
            self.lfsr = (self.lfsr & !(1 << 15)) | (xnor << 15);
            if self.width_7bit {
                self.lfsr = (self.lfsr & !(1 << 7)) | (xnor << 7);
            }
            self.lfsr >>= 1;
        }
    }

    fn sample(&self) -> u8 {
        if !self.enabled || !self.dac_on {
            return 0;
        }
        // Output is high when LFSR bit 0 is zero.
        if self.lfsr & 1 == 0 {
            self.env.volume
        } else {
            0
        }
    }

    fn trigger(&mut self) {
        self.enabled = true;
        self.timer = self.period();
        self.lfsr = 0;
        self.env.trigger();
        if !self.dac_on {
            self.enabled = false;
        }
    }
}

/// The whole sound unit: four channels, the frame sequencer, master
/// volume/panning (NR50/NR51), power (NR52), and the deterministic resampler
/// that turns the ~1.05 MHz channel mix into a fixed-rate stereo f32 stream.
pub struct Apu {
    ch1: Pulse,
    ch2: Pulse,
    ch3: Wave,
    ch4: Noise,

    /// Master on/off (NR52 bit 7). While off, all registers read back cleared
    /// and writes to everything except NR52 (and DMG length regs) are ignored.
    power: bool,
    /// NR50: left/right master volume (0..7 each) + VIN bits (unused on DMG).
    nr50: u8,
    /// NR51: per-channel L/R panning.
    nr51: u8,

    /// Frame sequencer: a 512 Hz step counter and its 8-phase position.
    seq_counter: u32,
    seq_step: u8,

    /// Resampler: fractional accumulator over T-cycles. Emits one stereo sample
    /// every CPU_HZ/OUTPUT_RATE cycles (tracked in fixed-point to stay exact).
    sample_accum: u32,
    /// Interleaved stereo samples produced during the current frame.
    samples: Vec<f32>,
}

impl Default for Apu {
    fn default() -> Self {
        Apu {
            ch1: Pulse::new(true),
            ch2: Pulse::new(false),
            ch3: Wave::default(),
            ch4: Noise::new(),
            power: false,
            nr50: 0,
            nr51: 0,
            seq_counter: 0,
            seq_step: 0,
            sample_accum: 0,
            samples: Vec::new(),
        }
    }
}

impl Apu {
    /// Advance one T-cycle: clock the frame sequencer and channel timers, then
    /// resample the analog mix into the output buffer.
    pub fn tick(&mut self) {
        if self.power {
            self.seq_counter += 1;
            if self.seq_counter >= FRAME_SEQ_PERIOD {
                self.seq_counter = 0;
                self.step_frame_sequencer();
            }
            self.ch1.tick();
            self.ch2.tick();
            self.ch3.tick();
            self.ch4.tick();
        }
        self.resample();
    }

    fn step_frame_sequencer(&mut self) {
        // 512 Hz sequence: length on even steps, envelope on 7, sweep on 2/6.
        match self.seq_step {
            0 | 4 => self.tick_length(),
            2 | 6 => {
                self.tick_length();
                self.tick_sweep();
            }
            7 => self.tick_envelope(),
            _ => {}
        }
        self.seq_step = (self.seq_step + 1) & 7;
    }

    fn tick_length(&mut self) {
        if self.ch1.length.tick() {
            self.ch1.enabled = false;
        }
        if self.ch2.length.tick() {
            self.ch2.enabled = false;
        }
        if self.ch3.length.tick() {
            self.ch3.enabled = false;
        }
        if self.ch4.length.tick() {
            self.ch4.enabled = false;
        }
    }

    fn tick_envelope(&mut self) {
        self.ch1.env.tick();
        self.ch2.env.tick();
        self.ch4.env.tick();
    }

    fn tick_sweep(&mut self) {
        if !self.ch1.tick_sweep() {
            self.ch1.enabled = false;
        }
    }

    /// Emit stereo samples for however many output samples are due this T-cycle.
    /// Fixed-point: accumulate OUTPUT_RATE per cycle, emit each time it crosses
    /// CPU_HZ. Deterministic — no floats in the timing.
    fn resample(&mut self) {
        self.sample_accum += OUTPUT_RATE;
        if self.sample_accum >= CPU_HZ {
            self.sample_accum -= CPU_HZ;
            let (l, r) = self.mix();
            self.samples.push(l);
            self.samples.push(r);
        }
    }

    /// Mix the four channels into a stereo pair in [-1, 1].
    fn mix(&self) -> (f32, f32) {
        let s = [
            self.ch1.sample(),
            self.ch2.sample(),
            self.ch3.sample(),
            self.ch4.sample(),
        ];
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for (i, &v) in s.iter().enumerate() {
            // Each channel's DAC maps 0..15 to -1..1 when its DAC is on. A
            // muted channel contributes 0 (its DAC would sit at a DC level, but
            // that only affects pop behavior we don't model).
            let analog = if self.channel_dac_on(i) {
                (v as f32 / 7.5) - 1.0
            } else {
                0.0
            };
            if self.nr51 & (1 << (i + 4)) != 0 {
                left += analog;
            }
            if self.nr51 & (1 << i) != 0 {
                right += analog;
            }
        }
        // NR50 master volume: 0..7 -> 1..8 scale, averaged over 4 channels.
        let lvol = ((self.nr50 >> 4) & 0x07) as f32 + 1.0;
        let rvol = (self.nr50 & 0x07) as f32 + 1.0;
        (left / 4.0 * lvol / 8.0, right / 4.0 * rvol / 8.0)
    }

    fn channel_dac_on(&self, i: usize) -> bool {
        match i {
            0 => self.ch1.dac_on,
            1 => self.ch2.dac_on,
            2 => self.ch3.dac_on,
            _ => self.ch4.dac_on,
        }
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }

    // --- register access -----------------------------------------------------

    pub fn read(&self, addr: u16) -> u8 {
        // OR masks: bits that always read 1 (write-only or unused bits). From
        // Blargg's `01-registers` / `11-regs after power` expected values.
        // Covers FF10..FF2F; wave RAM (FF30-FF3F) is handled above.
        const OR: [u8; 0x20] = [
            0x80, 0x3F, 0x00, 0xFF, 0xBF, // FF10-FF14 (NR10-NR14)
            0xFF, 0x3F, 0x00, 0xFF, 0xBF, // FF15 unused, NR21-NR24
            0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
            0xFF, 0xFF, 0x00, 0x00, 0xBF, // FF1F unused, NR41-NR44
            0x00, 0x00, 0x70, // NR50-NR52
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // FF27-FF2F
        ];
        if (0xFF30..=0xFF3F).contains(&addr) {
            return self.ch3.read_ram((addr - 0xFF30) as usize);
        }
        let idx = (addr - 0xFF10) as usize;
        let raw = self.read_raw(addr);
        raw | OR[idx]
    }

    fn read_raw(&self, addr: u16) -> u8 {
        match addr {
            0xFF10 => {
                let sw = self.ch1.sweep.as_ref().unwrap();
                (sw.period << 4) | ((sw.negate as u8) << 3) | sw.shift
            }
            0xFF11 => self.ch1.duty << 6,
            0xFF12 => self.ch1.env_byte(),
            0xFF14 => (self.ch1.length.enabled as u8) << 6,
            0xFF16 => self.ch2.duty << 6,
            0xFF17 => self.ch2.env_byte(),
            0xFF19 => (self.ch2.length.enabled as u8) << 6,
            0xFF1A => (self.ch3.dac_on as u8) << 7,
            0xFF1C => self.ch3.volume_code << 5,
            0xFF1E => (self.ch3.length.enabled as u8) << 6,
            0xFF21 => self.ch4.env_byte(),
            0xFF22 => {
                (self.ch4.clock_shift << 4) | ((self.ch4.width_7bit as u8) << 3) | self.ch4.divisor_code
            }
            0xFF23 => (self.ch4.length.enabled as u8) << 6,
            0xFF24 => self.nr50,
            0xFF25 => self.nr51,
            0xFF26 => {
                (self.power as u8) << 7
                    | (self.ch1.enabled as u8)
                    | (self.ch2.enabled as u8) << 1
                    | (self.ch3.enabled as u8) << 2
                    | (self.ch4.enabled as u8) << 3
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        // Wave RAM is always accessible (even while powered off).
        if (0xFF30..=0xFF3F).contains(&addr) {
            self.ch3.write_ram((addr - 0xFF30) as usize, val);
            return;
        }
        if !self.power {
            // While powered off, only NR52 and — on DMG — the length-load
            // registers (NR11/NR21/NR31/NR41 low bits) remain writable.
            match addr {
                0xFF26 => {}
                0xFF11 => self.ch1.length.set((val & 0x3F) as u16),
                0xFF16 => self.ch2.length.set((val & 0x3F) as u16),
                0xFF1B => self.ch3.length.set(val as u16),
                0xFF20 => self.ch4.length.set((val & 0x3F) as u16),
                _ => return,
            }
            if addr != 0xFF26 {
                return;
            }
        }

        match addr {
            0xFF10 => {
                let sw = self.ch1.sweep.as_mut().unwrap();
                sw.period = (val >> 4) & 0x07;
                let was_negate = sw.negate;
                sw.negate = val & 0x08 != 0;
                sw.shift = val & 0x07;
                // Clearing negate after it was used in a calc disables CH1.
                if was_negate && !sw.negate && sw.negate_used {
                    self.ch1.enabled = false;
                }
            }
            0xFF11 => {
                self.ch1.duty = val >> 6;
                self.ch1.length.set((val & 0x3F) as u16);
            }
            0xFF12 => {
                self.ch1.env.write(val);
                self.ch1.dac_on = self.ch1.env.dac_on();
                if !self.ch1.dac_on {
                    self.ch1.enabled = false;
                }
            }
            0xFF13 => self.ch1.freq = (self.ch1.freq & 0x700) | val as u16,
            0xFF14 => self.write_freq_hi(0, val),

            0xFF16 => {
                self.ch2.duty = val >> 6;
                self.ch2.length.set((val & 0x3F) as u16);
            }
            0xFF17 => {
                self.ch2.env.write(val);
                self.ch2.dac_on = self.ch2.env.dac_on();
                if !self.ch2.dac_on {
                    self.ch2.enabled = false;
                }
            }
            0xFF18 => self.ch2.freq = (self.ch2.freq & 0x700) | val as u16,
            0xFF19 => self.write_freq_hi(1, val),

            0xFF1A => {
                self.ch3.dac_on = val & 0x80 != 0;
                if !self.ch3.dac_on {
                    self.ch3.enabled = false;
                }
            }
            0xFF1B => self.ch3.length.set(val as u16),
            0xFF1C => self.ch3.volume_code = (val >> 5) & 0x03,
            0xFF1D => self.ch3.freq = (self.ch3.freq & 0x700) | val as u16,
            0xFF1E => self.write_freq_hi(2, val),

            0xFF20 => self.ch4.length.set((val & 0x3F) as u16),
            0xFF21 => {
                self.ch4.env.write(val);
                self.ch4.dac_on = self.ch4.env.dac_on();
                if !self.ch4.dac_on {
                    self.ch4.enabled = false;
                }
            }
            0xFF22 => {
                self.ch4.clock_shift = val >> 4;
                self.ch4.width_7bit = val & 0x08 != 0;
                self.ch4.divisor_code = val & 0x07;
            }
            0xFF23 => self.write_freq_hi(3, val),

            0xFF24 => self.nr50 = val,
            0xFF25 => self.nr51 = val,
            0xFF26 => self.write_power(val),
            _ => {}
        }
    }

    /// NRx4-style write: bit 7 triggers, bit 6 enables the length counter.
    /// Enabling length mid-step can produce an "extra" length clock (the
    /// obscure DMG length quirk that Blargg's `02-len ctr` checks).
    fn write_freq_hi(&mut self, ch: usize, val: u8) {
        let trigger = val & 0x80 != 0;
        let len_enable = val & 0x40 != 0;
        // The "extra clock" fires when length is being enabled during the first
        // half of the sequencer period (a step that does NOT clock length next).
        let extra_clock = !self.length_step_is_next() && len_enable;

        let freq_hi = (val & 0x07) as u16;
        match ch {
            0 => self.ch1.freq = (self.ch1.freq & 0xFF) | (freq_hi << 8),
            1 => self.ch2.freq = (self.ch2.freq & 0xFF) | (freq_hi << 8),
            2 => self.ch3.freq = (self.ch3.freq & 0xFF) | (freq_hi << 8),
            _ => {}
        }

        // Length handling with the DMG "extra clock" quirk.
        let length = self.length_mut(ch);
        let prev_enabled = length.enabled;
        length.enabled = len_enable;
        // Enabling length (disabled -> enabled) during a non-length step clocks
        // the counter once; if that zeroes it and we're not triggering, the
        // channel is disabled.
        if extra_clock && !prev_enabled && length.counter > 0 {
            length.counter -= 1;
            if length.counter == 0 && !trigger {
                self.disable(ch);
            }
        }

        if trigger {
            // Trigger reloads a zero length counter to max; during a non-length
            // step with length enabled, that reload is immediately clocked once
            // (so it lands on max-1).
            let length = self.length_mut(ch);
            if length.counter == 0 {
                length.counter = length.max;
                if extra_clock && len_enable {
                    length.counter -= 1;
                }
            }
            self.trigger(ch);
        }
    }

    /// True if the next sequencer step will clock length (steps 0,2,4,6).
    fn length_step_is_next(&self) -> bool {
        self.seq_step.is_multiple_of(2)
    }

    fn length_mut(&mut self, ch: usize) -> &mut Length {
        match ch {
            0 => &mut self.ch1.length,
            1 => &mut self.ch2.length,
            2 => &mut self.ch3.length,
            _ => &mut self.ch4.length,
        }
    }

    fn disable(&mut self, ch: usize) {
        match ch {
            0 => self.ch1.enabled = false,
            1 => self.ch2.enabled = false,
            2 => self.ch3.enabled = false,
            _ => self.ch4.enabled = false,
        }
    }

    fn trigger(&mut self, ch: usize) {
        match ch {
            0 => self.ch1.trigger(),
            1 => self.ch2.trigger(),
            2 => self.ch3.trigger(),
            _ => self.ch4.trigger(),
        }
    }

    fn write_power(&mut self, val: u8) {
        let on = val & 0x80 != 0;
        if on && !self.power {
            self.power = true;
            // Power-on resets the frame sequencer phase.
            self.seq_counter = 0;
            self.seq_step = 0;
        } else if !on && self.power {
            // Power-off clears every register/channel (DMG). Wave RAM survives.
            let ram = self.ch3.ram;
            let ch1_len = self.ch1.length.counter;
            let ch2_len = self.ch2.length.counter;
            let ch3_len = self.ch3.length.counter;
            let ch4_len = self.ch4.length.counter;
            self.ch1 = Pulse::new(true);
            self.ch2 = Pulse::new(false);
            self.ch3 = Wave::default();
            self.ch4 = Noise::new();
            self.ch3.ram = ram;
            // ponytail: on DMG the length counters are NOT cleared by power-off
            // (only on CGB). Preserve them so `08-len ctr during power` holds.
            self.ch1.length.counter = ch1_len;
            self.ch2.length.counter = ch2_len;
            self.ch3.length.counter = ch3_len;
            self.ch4.length.counter = ch4_len;
            self.nr50 = 0;
            self.nr51 = 0;
            self.power = false;
        }
    }
}

impl Pulse {
    fn env_byte(&self) -> u8 {
        (self.env.initial << 4) | ((self.env.add as u8) << 3) | self.env.period
    }
}

impl Noise {
    fn env_byte(&self) -> u8 {
        (self.env.initial << 4) | ((self.env.add as u8) << 3) | self.env.period
    }
}

impl Apu {
    /// Serialize APU state. All registers are captured by writing them back on
    /// load; we snapshot the live channel state directly so audio continues
    /// seamlessly across a save/load (Blargg-visible registers + timers).
    pub(crate) fn serialize(&self, out: &mut Vec<u8>) {
        out.push(self.power as u8);
        out.push(self.nr50);
        out.push(self.nr51);
        out.push(self.seq_step);
        out.extend_from_slice(&self.seq_counter.to_le_bytes());
        out.extend_from_slice(&self.sample_accum.to_le_bytes());
        self.ch1.serialize(out);
        self.ch2.serialize(out);
        self.ch3.serialize(out);
        self.ch4.serialize(out);
    }

    pub(crate) fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 4 + 8 {
            return false;
        }
        self.power = s[*p] != 0;
        self.nr50 = s[*p + 1];
        self.nr51 = s[*p + 2];
        self.seq_step = s[*p + 3];
        *p += 4;
        self.seq_counter = u32::from_le_bytes([s[*p], s[*p + 1], s[*p + 2], s[*p + 3]]);
        *p += 4;
        self.sample_accum = u32::from_le_bytes([s[*p], s[*p + 1], s[*p + 2], s[*p + 3]]);
        *p += 4;
        self.ch1.deserialize(s, p)
            && self.ch2.deserialize(s, p)
            && self.ch3.deserialize(s, p)
            && self.ch4.deserialize(s, p)
    }
}

impl Length {
    fn serialize(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.counter.to_le_bytes());
        out.push(self.enabled as u8);
    }
    fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 3 {
            return false;
        }
        self.counter = u16::from_le_bytes([s[*p], s[*p + 1]]);
        self.enabled = s[*p + 2] != 0;
        *p += 3;
        true
    }
}

impl Envelope {
    fn serialize(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[
            self.initial,
            self.add as u8,
            self.period,
            self.volume,
            self.timer,
        ]);
    }
    fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 5 {
            return false;
        }
        self.initial = s[*p];
        self.add = s[*p + 1] != 0;
        self.period = s[*p + 2];
        self.volume = s[*p + 3];
        self.timer = s[*p + 4];
        *p += 5;
        true
    }
}

impl Pulse {
    fn serialize(&self, out: &mut Vec<u8>) {
        out.push(self.enabled as u8);
        out.push(self.dac_on as u8);
        out.push(self.duty);
        out.extend_from_slice(&self.freq.to_le_bytes());
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.push(self.phase);
        self.length.serialize(out);
        self.env.serialize(out);
        if let Some(sw) = &self.sweep {
            out.push(1);
            out.extend_from_slice(&[sw.period, sw.negate as u8, sw.shift, sw.enabled as u8]);
            out.extend_from_slice(&sw.shadow.to_le_bytes());
            out.push(sw.timer);
            out.push(sw.negate_used as u8);
        } else {
            out.push(0);
        }
    }
    fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        if s.len() < *p + 7 {
            return false;
        }
        self.enabled = s[*p] != 0;
        self.dac_on = s[*p + 1] != 0;
        self.duty = s[*p + 2];
        self.freq = u16::from_le_bytes([s[*p + 3], s[*p + 4]]);
        self.timer = u16::from_le_bytes([s[*p + 5], s[*p + 6]]);
        *p += 7;
        if s.len() < *p + 1 {
            return false;
        }
        self.phase = s[*p];
        *p += 1;
        if !self.length.deserialize(s, p) || !self.env.deserialize(s, p) {
            return false;
        }
        if s.len() < *p + 1 {
            return false;
        }
        let has_sweep = s[*p] != 0;
        *p += 1;
        if has_sweep {
            if s.len() < *p + 4 + 2 + 2 {
                return false;
            }
            let sw = self.sweep.get_or_insert_with(Sweep::default);
            sw.period = s[*p];
            sw.negate = s[*p + 1] != 0;
            sw.shift = s[*p + 2];
            sw.enabled = s[*p + 3] != 0;
            sw.shadow = u16::from_le_bytes([s[*p + 4], s[*p + 5]]);
            sw.timer = s[*p + 6];
            sw.negate_used = s[*p + 7] != 0;
            *p += 8;
        }
        true
    }
}

impl Wave {
    fn serialize(&self, out: &mut Vec<u8>) {
        out.push(self.enabled as u8);
        out.push(self.dac_on as u8);
        out.extend_from_slice(&self.freq.to_le_bytes());
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.push(self.position);
        out.push(self.volume_code);
        out.push(self.sample_buffer);
        out.extend_from_slice(&self.ram);
        self.length.serialize(out);
    }
    fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        let need = 1 + 1 + 2 + 2 + 1 + 1 + 1 + 16;
        if s.len() < *p + need {
            return false;
        }
        self.enabled = s[*p] != 0;
        self.dac_on = s[*p + 1] != 0;
        self.freq = u16::from_le_bytes([s[*p + 2], s[*p + 3]]);
        self.timer = u16::from_le_bytes([s[*p + 4], s[*p + 5]]);
        self.position = s[*p + 6];
        self.volume_code = s[*p + 7];
        self.sample_buffer = s[*p + 8];
        self.ram.copy_from_slice(&s[*p + 9..*p + 9 + 16]);
        *p += need;
        self.length.deserialize(s, p)
    }
}

impl Noise {
    fn serialize(&self, out: &mut Vec<u8>) {
        out.push(self.enabled as u8);
        out.push(self.dac_on as u8);
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.extend_from_slice(&self.lfsr.to_le_bytes());
        out.extend_from_slice(&[self.clock_shift, self.width_7bit as u8, self.divisor_code]);
        self.length.serialize(out);
        self.env.serialize(out);
    }
    fn deserialize(&mut self, s: &[u8], p: &mut usize) -> bool {
        let need = 1 + 1 + 2 + 2 + 3;
        if s.len() < *p + need {
            return false;
        }
        self.enabled = s[*p] != 0;
        self.dac_on = s[*p + 1] != 0;
        self.timer = u16::from_le_bytes([s[*p + 2], s[*p + 3]]);
        self.lfsr = u16::from_le_bytes([s[*p + 4], s[*p + 5]]);
        self.clock_shift = s[*p + 6];
        self.width_7bit = s[*p + 7] != 0;
        self.divisor_code = s[*p + 8];
        *p += need;
        self.length.deserialize(s, p) && self.env.deserialize(s, p)
    }
}

#[cfg(test)]
mod apu_selfcheck {
    use super::*;

    /// A triggered pulse channel with a nonzero envelope produces a non-silent,
    /// oscillating signal, and the resampler emits ~OUTPUT_RATE stereo pairs per
    /// second of emulated T-cycles. This is the smallest thing that fails if the
    /// duty/timer/mix/resample path breaks.
    #[test]
    fn triggered_pulse_is_audible() {
        let mut apu = Apu::default();
        apu.write(0xFF26, 0x80); // power on
        apu.write(0xFF25, 0xFF); // pan everything L+R
        apu.write(0xFF24, 0x77); // full master volume
        // CH2: 50% duty, max volume envelope, mid frequency, trigger.
        apu.write(0xFF16, 0x80); // duty 50%
        apu.write(0xFF17, 0xF0); // volume 15, no envelope step
        apu.write(0xFF18, 0x00); // freq lo
        apu.write(0xFF19, 0x87); // trigger + freq hi

        // Run ~1/60 s of T-cycles.
        let cycles = CPU_HZ / 60;
        for _ in 0..cycles {
            apu.tick();
        }
        let s = apu.samples();
        assert!(!s.is_empty(), "resampler produced no samples");
        // Roughly OUTPUT_RATE/60 stereo pairs (allow slack for rounding).
        let pairs = s.len() / 2;
        let expected = (OUTPUT_RATE / 60) as usize;
        assert!(
            pairs.abs_diff(expected) < 20,
            "expected ~{expected} pairs, got {pairs}"
        );
        // Signal must actually move (not stuck silent / DC).
        let min = s.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = s.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max - min > 0.01, "signal is flat: min={min} max={max}");
    }

    /// Powering the APU off then on clears channel enables (DMG NR52 behavior).
    #[test]
    fn power_off_disables_channels() {
        let mut apu = Apu::default();
        apu.write(0xFF26, 0x80);
        apu.write(0xFF17, 0xF0);
        apu.write(0xFF19, 0x80); // trigger CH2
        assert!(apu.ch2.enabled);
        apu.write(0xFF26, 0x00); // power off
        assert!(!apu.ch2.enabled);
        assert_eq!(apu.read(0xFF26) & 0x0F, 0, "channels report off");
    }
}
