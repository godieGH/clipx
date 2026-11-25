let ctx: AudioContext | null = null;

/** Short two-note chime for a new notification. */
export function playChime(): void {
  try {
    const audio = (ctx ??= new AudioContext());
    if (audio.state === "suspended") void audio.resume();
    const t0 = audio.currentTime;
    [660, 880].forEach((freq, i) => {
      const osc = audio.createOscillator();
      const gain = audio.createGain();
      const start = t0 + i * 0.12;
      osc.type = "sine";
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(0.18, start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.22);
      osc.connect(gain).connect(audio.destination);
      osc.start(start);
      osc.stop(start + 0.25);
    });
  } catch (error) {
    console.warn("could not play notification sound", error);
  }
}