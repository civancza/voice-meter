// Deterministic soundtrack synth: music + muffled "talking" voice -> assets/audio/music.wav
// Run: node scripts/make-audio.mjs   (then scripts/mix.sh mixes SFX on top)
import { writeFileSync, mkdirSync } from "node:fs";

const SR = 44100;
const DUR = 30;
const N = SR * DUR;
const BEAT = 0.5; // 120 BPM
const L = new Float32Array(N);
const R = new Float32Array(N);

// Same loudness curve the HUD uses (index.html) — voice volume follows it.
const DB_KEYS = [
  [0, -52], [1.2, -50], [1.6, -34], [4.5, -31], [6.5, -22], [7.6, -13],
  [10.6, -11], [11.6, -30], [13.8, -32], [15, -50], [30, -52],
];
const baseDb = (t) => {
  for (let i = 1; i < DB_KEYS.length; i++) {
    const [t1, d1] = DB_KEYS[i];
    const [t0, d0] = DB_KEYS[i - 1];
    if (t <= t1) {
      const u = (t - t0) / (t1 - t0);
      const s = u * u * (3 - 2 * u);
      return d0 + (d1 - d0) * s;
    }
  }
  return DB_KEYS[DB_KEYS.length - 1][1];
};

let seed = 1234567;
const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff) * 2 - 1;
const mtof = (m) => 440 * Math.pow(2, (m - 69) / 12);
const env = (t, a, d) => (t < 0 ? 0 : t < a ? t / a : Math.exp(-(t - a) / d));
const add = (i, v, pan = 0) => {
  if (i < 0 || i >= N) return;
  L[i] += v * (1 - Math.max(0, pan));
  R[i] += v * (1 + Math.min(0, pan));
};

// Section gains
const sec = (t) => {
  if (t < 8) return { pad: 0.9, hat: t > 2 ? 0.5 : 0, kick: t > 4 ? 0.6 : 0, bass: t > 4 ? 0.5 : 0, pluck: 0, tension: 0 };
  if (t < 11.5) return { pad: 0.25, hat: 0.7, kick: 0.8, bass: 0, pluck: 0, tension: 1 };
  if (t < 16) return { pad: 1, hat: 0, kick: 0, bass: 0.35, pluck: 0.5, tension: 0 };
  if (t < 24) return { pad: 0.85, hat: 0.8, kick: 1, bass: 0.8, pluck: 0.8, tension: 0 };
  return { pad: 1, hat: 0.3, kick: t < 26 ? 0.6 : 0, bass: 0.4, pluck: 0.7, tension: 0 };
};
const master = (t) => (t < 0.6 ? t / 0.6 : t > 27.5 ? Math.max(0, (30 - t) / 2.5) : 1);

// Chords (A minor: Am F C G), one per bar (2s)
const CHORDS = [
  [57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62],
];
const chordAt = (t) => CHORDS[Math.floor(t / 2) % 4];

// Pad: detuned saws through a one-pole lowpass, sidechained by kick
{
  const phases = new Float64Array(12);
  let lp = 0;
  for (let i = 0; i < N; i++) {
    const t = i / SR;
    const s = sec(t);
    const ch = chordAt(t);
    let v = 0;
    let k = 0;
    for (const n of ch) {
      for (const det of [-0.08, 0.08]) {
        const f = mtof(n + det);
        phases[k] = (phases[k] + f / SR) % 1;
        v += phases[k] * 2 - 1;
        k++;
      }
    }
    const cutoff = 900 + 500 * Math.sin(t * 0.4);
    const a = 1 - Math.exp((-2 * Math.PI * cutoff) / SR);
    lp += a * (v - lp);
    const beatPos = t % BEAT;
    const duck = s.kick > 0 ? 1 - 0.55 * s.kick * Math.exp(-beatPos / 0.12) : 1;
    add(i, lp * 0.035 * s.pad * duck * master(t), Math.sin(t * 0.7) * 0.3);
  }
}

// Kick, hats, bass, plucks, tension pulse on the grid
for (let b = 0; b < DUR / (BEAT / 4); b++) {
  const t0 = b * (BEAT / 4); // 16th grid
  const s = sec(t0);
  const i0 = Math.round(t0 * SR);
  const sixteenth = b % 4;
  if (sixteenth === 0 && s.kick > 0) {
    let ph = 0;
    for (let j = 0; j < SR * 0.45; j++) {
      const tt = j / SR;
      const f = 45 + 110 * Math.exp(-tt / 0.04);
      ph += f / SR;
      add(i0 + j, Math.sin(2 * Math.PI * ph) * env(tt, 0.002, 0.16) * 0.55 * s.kick * master(t0));
    }
  }
  if (sixteenth === 2 && s.hat > 0) {
    let prev = 0;
    for (let j = 0; j < SR * 0.08; j++) {
      const n = rnd();
      const hp = n - prev;
      prev = n;
      add(i0 + j, hp * env(j / SR, 0.001, 0.025) * 0.09 * s.hat * master(t0), 0.25);
    }
  }
  if ((sixteenth === 0 || sixteenth === 2) && s.bass > 0) {
    const f = mtof(chordAt(t0)[0] - 24);
    let ph = 0;
    let lp = 0;
    for (let j = 0; j < SR * 0.24; j++) {
      ph = (ph + f / SR) % 1;
      const a = 1 - Math.exp((-2 * Math.PI * 380) / SR);
      lp += a * (ph * 2 - 1 - lp);
      add(i0 + j, lp * env(j / SR, 0.004, 0.12) * 0.3 * s.bass * master(t0));
    }
  }
  if (s.pluck > 0) {
    const ch = chordAt(t0);
    const pattern = [0, 1, 2, 1];
    const n = ch[pattern[sixteenth]] + 12 + (b % 16 >= 8 && sixteenth === 3 ? 7 : 0);
    const f = mtof(n);
    for (let j = 0; j < SR * 0.3; j++) {
      const tt = j / SR;
      const v = Math.sin(2 * Math.PI * f * tt) + 0.3 * Math.sin(4 * Math.PI * f * tt);
      add(i0 + j, v * env(tt, 0.002, 0.09) * 0.07 * s.pluck * master(t0), sixteenth % 2 ? 0.4 : -0.4);
    }
  }
  if (s.tension > 0 && sixteenth % 2 === 0) {
    const f = mtof(45);
    for (let j = 0; j < SR * 0.18; j++) {
      const tt = j / SR;
      const v = Math.sign(Math.sin(2 * Math.PI * f * tt)) * 0.5 + Math.sin(2 * Math.PI * f * 2 * tt);
      add(i0 + j, v * env(tt, 0.003, 0.06) * 0.06 * s.tension);
    }
  }
}

// Muffled "talking" voice (wah-wah): saw glottal source -> two resonant formant filters
{
  const coeffs = (f, q) => {
    const w = (2 * Math.PI * f) / SR;
    const alpha = Math.sin(w) / (2 * q);
    const a0 = 1 + alpha;
    return { b0: alpha / a0, b2: -alpha / a0, a1: (-2 * Math.cos(w)) / a0, a2: (1 - alpha) / a0 };
  };
  const mk = () => ({ c: coeffs(700, 6), x1: 0, x2: 0, y1: 0, y2: 0 });
  const run = (bq, x) => {
    const { b0, b2, a1, a2 } = bq.c;
    const y = b0 * x + b2 * bq.x2 - a1 * bq.y1 - a2 * bq.y2;
    bq.x2 = bq.x1; bq.x1 = x; bq.y2 = bq.y1; bq.y1 = y;
    return y;
  };
  const VOWELS = [[700, 1220], [400, 2000], [550, 900], [300, 870], [600, 1700]];
  const hash = (k) => {
    const x = Math.sin(k * 127.1 + 311.7) * 43758.5453;
    return x - Math.floor(x);
  };
  let ph = 0;
  const f1 = mk();
  const f2 = mk();
  let lastSyl = -1;
  for (let i = 0; i < N; i++) {
    const t = i / SR;
    if (t < 1.6 || t > 14.2) continue;
    const syl = Math.floor(t * 5.2);
    const u = t * 5.2 - syl;
    if (syl !== lastSyl) {
      const v = VOWELS[Math.floor(hash(syl) * VOWELS.length)];
      f1.c = coeffs(v[0], 6);
      f2.c = coeffs(v[1], 8);
      lastSyl = syl;
    }
    const gap = hash(syl + 99) < 0.18 ? 0 : 1; // word gaps
    const sylEnv = Math.sin(Math.PI * Math.min(1, u * 1.15)) ** 0.7 * gap;
    const db = baseDb(t);
    const loud = Math.pow(10, (db + 12) / 20); // unity at -12 dB
    const pitch = 125 + 40 * hash(syl + 7) + (db > -20 ? 45 : 0);
    ph = (ph + pitch / SR) % 1;
    const src = ph * 2 - 1;
    const v = run(f1, src) + run(f2, src) * 0.5;
    add(i, v * sylEnv * loud * 0.55, -0.1);
  }
}

// Normalize + write 16-bit stereo WAV
let peak = 0;
for (let i = 0; i < N; i++) peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
const g = 0.89 / peak;
const buf = Buffer.alloc(44 + N * 4);
buf.write("RIFF", 0); buf.writeUInt32LE(36 + N * 4, 4); buf.write("WAVE", 8);
buf.write("fmt ", 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(2, 22);
buf.writeUInt32LE(SR, 24); buf.writeUInt32LE(SR * 4, 28); buf.writeUInt16LE(4, 32); buf.writeUInt16LE(16, 34);
buf.write("data", 36); buf.writeUInt32LE(N * 4, 40);
for (let i = 0; i < N; i++) {
  buf.writeInt16LE(Math.round(Math.tanh(L[i] * g) * 32767), 44 + i * 4);
  buf.writeInt16LE(Math.round(Math.tanh(R[i] * g) * 32767), 46 + i * 4);
}
mkdirSync("assets/audio", { recursive: true });
writeFileSync("assets/audio/music.wav", buf);
console.log("wrote assets/audio/music.wav", { peak: peak.toFixed(3) });
