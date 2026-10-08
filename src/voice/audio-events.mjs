function median(values) {
  if (!values.length) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)];
}

export function summarizeAudioFrames(frameRms, sampleRate, sampleCount) {
  if (!frameRms.length || !sampleRate || !sampleCount) {
    return {
      window_ms: 0,
      volume: "unknown",
      cough_count: 0,
      breath_like_pause_count: 0,
      breathing: "unknown",
      raw_audio_sent: false,
    };
  }
  const baseline = Math.max(median(frameRms), 0.01);
  const coughThreshold = Math.max(0.45, baseline * 3.5);
  const quietThreshold = Math.max(0.015, baseline * 0.45);
  let coughCount = 0;
  let pauseCount = 0;
  let highFrames = 0;
  let quietFrames = 0;
  const flush = () => {
    if (highFrames >= 1 && highFrames <= 4) coughCount += 1;
    highFrames = 0;
    if (quietFrames >= 2 && quietFrames <= 40) pauseCount += 1;
    quietFrames = 0;
  };
  for (const rms of frameRms) {
    if (rms > coughThreshold) highFrames += 1;
    else if (highFrames) flush();
    if (rms < quietThreshold) quietFrames += 1;
    else if (quietFrames) {
      if (quietFrames >= 2 && quietFrames <= 40) pauseCount += 1;
      quietFrames = 0;
    }
  }
  flush();
  const average = frameRms.reduce((sum, value) => sum + value, 0) / frameRms.length;
  return {
    window_ms: Math.round(sampleCount * 1000 / sampleRate),
    volume: average < 0.03 ? "quiet" : average < 0.2 ? "normal" : "loud",
    cough_count: Math.min(coughCount, 8),
    breath_like_pause_count: Math.min(pauseCount, 8),
    breathing: pauseCount ? "possible" : "not_detected",
    raw_audio_sent: false,
  };
}

export class AudioEventWindow {
  constructor(sampleRate, windowMs = 2000) {
    this.sampleRate = sampleRate;
    this.windowSamples = Math.max(1, Math.round(sampleRate * windowMs / 1000));
    this.samples = 0;
    this.frames = [];
  }

  push(samples) {
    let sum = 0;
    for (const sample of samples) sum += sample * sample;
    this.frames.push(Math.sqrt(sum / Math.max(1, samples.length)));
    this.samples += samples.length;
    if (this.samples < this.windowSamples) return null;
    const result = summarizeAudioFrames(this.frames, this.sampleRate, this.samples);
    this.samples = 0;
    this.frames = [];
    return result;
  }
}
