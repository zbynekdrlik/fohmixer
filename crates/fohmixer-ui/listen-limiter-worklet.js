// Listen (#210 phase 2): the last stage of the page's player, after the
// volume: a linked-stereo peak limiter at -1 dBFS, instant attack, 50 ms
// release, so a raised quiet track never clips the device's output. As
// iemmixer's listen-limiter-worklet.js.
const CEILING = Math.pow(10, -1 / 20);
const RELEASE_S = 0.05;

class ListenLimiter extends AudioWorkletProcessor {
  constructor() {
    super();
    this.gain = 1;
    this.release = Math.exp(-1 / (RELEASE_S * sampleRate));
  }

  process(inputs, outputs) {
    const input = inputs[0];
    const output = outputs[0];
    if (!input || input.length === 0) {
      for (const ch of output) ch.fill(0);
      return true;
    }
    const n = input[0].length;
    for (let i = 0; i < n; i++) {
      let peak = 0;
      for (let c = 0; c < input.length; c++) {
        const v = Math.abs(input[c][i]);
        if (v > peak) peak = v;
      }
      const target = peak > CEILING ? CEILING / peak : 1;
      this.gain = target < this.gain ? target : target + (this.gain - target) * this.release;
      for (let c = 0; c < output.length; c++) {
        const src = input[c] || input[0];
        output[c][i] = src[i] * this.gain;
      }
    }
    return true;
  }
}

registerProcessor("listen-limiter", ListenLimiter);
