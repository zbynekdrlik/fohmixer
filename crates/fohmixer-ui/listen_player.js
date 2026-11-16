// Listen (#210 phase 2): the page's player of the hub's /ws/audio packets.
// One Opus packet per 20 ms (48 kHz stereo) is decoded with WebCodecs'
// AudioDecoder (Safari 26 / iPadOS 26, Chromium; a secure page), scheduled
// as Web Audio buffers behind an adaptive jitter buffer (80 to 500 ms), and
// played through the volume's gain into a -1 dBFS limiter worklet
// (listen-limiter-worklet.js). Adapted from iemmixer's audio_player.js (the
// same hub encoder, the same rules); the decisions of when to listen live in
// Rust (behave/listen.rs), this file only plays.

const SAMPLE_RATE = 48000;
const MIN_BUFFER_MS = 80;
const MAX_BUFFER_MS = 500;
// A gap between packets over this is a hole: the buffer grows.
const HOLE_MS = 60;
// Ten quiet seconds before the buffer shrinks again, 1 ms a packet.
const SHRINK_AFTER_MS = 10000;
// Scheduled further ahead than this (a stall, then a burst): back to live.
const MAX_AHEAD_S = 0.5;
// While the context is not running (its start, or iOS suspended it), only
// the newest this many packets wait (100 ms): the sound stays live.
const DRAIN_NEWEST = 5;

let context = null;
let gainNode = null;
let limiter = null;
let decoder = null;
let gain = 1;
let nextStart = 0;
let packets = 0;
// Every packet handed to the player, even with no context (the E2E's
// proof that a stopped page gets none).
let arrived = 0;
let decoded = 0;
let played = 0;
let dropouts = 0;
let waiting = [];
let bufferMs = MIN_BUFFER_MS;
let lastArrival = 0;
let lastHole = 0;
let level = -150;
let lastError = null;

/** Whether this page can decode the hub's Opus (WebCodecs, a secure page). */
export function listenDecoder() {
  return typeof AudioDecoder !== "undefined" && window.isSecureContext === true;
}

function resetCounts() {
  nextStart = 0;
  packets = 0;
  decoded = 0;
  played = 0;
  dropouts = 0;
  waiting = [];
  bufferMs = MIN_BUFFER_MS;
  lastArrival = 0;
  lastHole = 0;
  level = -150;
  lastError = null;
}

/**
 * Starts the player. Called inside the key's tap: a browser lets a page's
 * audio start only in a user's gesture, so the context is made and resumed
 * here and nowhere later.
 */
export function startListenPlayer() {
  stopListenPlayer();
  resetCounts();
  const ctx = new AudioContext({ sampleRate: SAMPLE_RATE });
  context = ctx;
  gainNode = ctx.createGain();
  gainNode.gain.value = gain;
  // The gain reaches the output only through the limiter: until the
  // worklet loaded the player is silent, never unlimited.
  ctx.audioWorklet
    .addModule("/listen-limiter-worklet.js")
    .then(() => {
      if (context !== ctx || !gainNode) return;
      limiter = new AudioWorkletNode(ctx, "listen-limiter", {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [2],
      });
      gainNode.connect(limiter);
      limiter.connect(ctx.destination);
    })
    .catch((e) => {
      lastError = "limiter: " + e.message;
      console.error("[listen] the limiter did not load:", e);
    });
  if (ctx.state === "suspended") {
    ctx.resume().catch((e) => {
      lastError = "resume: " + e.message;
    });
  }
  ctx.onstatechange = () => {
    if (context === ctx && ctx.state === "running" && waiting.length > 0) {
      // Only the newest packets: the sound is live, never a burst of the
      // past (a context resumed after iOS suspended it).
      nextStart = 0;
      for (const packet of waiting.splice(0)) decode(packet);
    }
  };
}

/**
 * Resumes a player iOS suspended (the page hidden, the screen locked): the
 * glue calls it when the page is shown again and at every finger's lift
 * (a user's gesture). It does nothing while the player plays or is stopped.
 */
export function resumeListenPlayer() {
  if (context && context.state !== "running" && context.state !== "closed") {
    context.resume().catch(() => {});
  }
}

/** Stops the player: the decoder, the context and every count. */
export function stopListenPlayer() {
  if (decoder) {
    try {
      decoder.close();
    } catch (_) {
      // Already closed by its own error.
    }
    decoder = null;
  }
  gainNode = null;
  limiter = null;
  if (context) {
    const ctx = context;
    context = null;
    ctx.onstatechange = null;
    ctx.close().catch(() => {});
  }
  waiting = [];
  level = -150;
}

/** The volume's linear gain (0 is silence). */
export function setListenGain(value) {
  gain = value;
  if (gainNode && context) {
    gainNode.gain.setValueAtTime(value, context.currentTime);
  }
}

/** One Opus packet from the hub (its bytes). */
export function feedListenPacket(bytes) {
  arrived++;
  if (!context) return;
  packets++;
  const now = performance.now();
  if (lastArrival > 0 && now - lastArrival > HOLE_MS) {
    bufferMs = Math.min(bufferMs + 40, MAX_BUFFER_MS);
    lastHole = now;
  }
  lastArrival = now;
  if (lastHole > 0 && now - lastHole > SHRINK_AFTER_MS && bufferMs > MIN_BUFFER_MS) {
    bufferMs = Math.max(bufferMs - 1, MIN_BUFFER_MS);
  }
  if (context.state !== "running") {
    waiting.push(bytes);
    if (waiting.length > DRAIN_NEWEST) waiting.shift();
    return;
  }
  decode(bytes);
}

function ensureDecoder() {
  if (decoder && decoder.state !== "closed") return;
  decoder = new AudioDecoder({
    output: (data) => schedule(data),
    error: (e) => {
      lastError = e.message;
      console.error("[listen] the decoder failed:", e.message);
    },
  });
  decoder.configure({ codec: "opus", sampleRate: SAMPLE_RATE, numberOfChannels: 2 });
}

function decode(bytes) {
  try {
    ensureDecoder();
    const chunk = new EncodedAudioChunk({
      type: "key",
      timestamp: packets * 20000,
      data: bytes,
    });
    decoder.decode(chunk);
  } catch (e) {
    lastError = e.message;
  }
}

function schedule(data) {
  if (!context || !gainNode) {
    data.close();
    return;
  }
  decoded++;
  const frames = data.numberOfFrames;
  const channels = data.numberOfChannels;
  const buffer = context.createBuffer(channels, frames, data.sampleRate);
  let peak = 0;
  for (let ch = 0; ch < channels; ch++) {
    const opts = { planeIndex: ch, format: "f32-planar" };
    const samples = new Float32Array(data.allocationSize(opts) / 4);
    data.copyTo(samples, opts);
    for (let i = 0; i < samples.length; i++) {
      const abs = Math.abs(samples[i]);
      if (abs > peak) peak = abs;
    }
    buffer.copyToChannel(samples, ch);
  }
  data.close();
  level = peak > 0.0001 ? 20 * Math.log10(peak) : -150;
  const source = context.createBufferSource();
  source.buffer = buffer;
  source.connect(gainNode);
  const now = context.currentTime;
  if (nextStart - now > MAX_AHEAD_S) {
    nextStart = now + bufferMs / 1000;
  }
  if (nextStart <= now) {
    if (played > 0) dropouts++;
    nextStart = now + bufferMs / 1000;
  }
  source.start(nextStart);
  nextStart += buffer.duration;
  played++;
}

/** What the player did (the page's diagnostics and the E2E read it). */
export function listenStats() {
  return {
    running: context !== null && context.state === "running",
    limiter: limiter !== null,
    packets,
    arrived,
    decoded,
    played,
    dropouts,
    bufferMs,
    level,
    gain,
    error: lastError,
  };
}

if (typeof window !== "undefined") {
  window.__fohmixerListen = listenStats;
}
