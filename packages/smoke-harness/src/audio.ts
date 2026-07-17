/**
 * Minimal Web Audio playback for cores that produce sound.
 *
 * The seam stays device-agnostic: the harness calls `AudioPump` for any core
 * whose `sampleRate > 0`, feeding it the interleaved-stereo f32 buffer that
 * `core.audio()` returns each frame. Cores with no audio (sampleRate 0) never
 * construct one.
 *
 * ponytail: queued `AudioBufferSourceNode`s (one per emulated frame) rather than
 * an AudioWorklet + ring buffer. Ceiling: a scheduling hiccup can cause a small
 * gap/pop under load, and there's no drift correction beyond a catch-up reset.
 * Upgrade path: move to an AudioWorklet fed by a SharedArrayBuffer ring when the
 * standalone Web Audio package is extracted. Fine for a live smoke check.
 *
 * Browsers gate AudioContext behind a user gesture; construction may yield a
 * "suspended" context. Call `resume()` from a click/change handler.
 */
export class AudioPump {
  private ctx: AudioContext;
  private sampleRate: number;
  /** Next start time (in ctx time) to schedule a buffer at. */
  private nextTime = 0;
  /** How far ahead of `currentTime` we let the queue run before resyncing. */
  private readonly maxLeadSeconds = 0.25;

  constructor(sampleRate: number) {
    this.sampleRate = sampleRate;
    this.ctx = new AudioContext({ sampleRate });
  }

  /** Resume the context; must be called from a user gesture on first use. */
  resume(): void {
    if (this.ctx.state === "suspended") void this.ctx.resume();
  }

  /** Enqueue one frame's interleaved-stereo f32 samples for playback. */
  push(interleaved: Float32Array): void {
    const frames = interleaved.length >> 1;
    if (frames === 0) return;

    const buf = this.ctx.createBuffer(2, frames, this.sampleRate);
    const left = buf.getChannelData(0);
    const right = buf.getChannelData(1);
    for (let i = 0; i < frames; i++) {
      left[i] = interleaved[2 * i];
      right[i] = interleaved[2 * i + 1];
    }

    const src = this.ctx.createBufferSource();
    src.buffer = buf;
    src.connect(this.ctx.destination);

    const now = this.ctx.currentTime;
    // Resync if we've fallen behind (started) or drifted too far ahead.
    if (this.nextTime < now || this.nextTime > now + this.maxLeadSeconds) {
      this.nextTime = now + 0.02;
    }
    src.start(this.nextTime);
    this.nextTime += frames / this.sampleRate;
  }

  close(): void {
    void this.ctx.close();
  }
}
