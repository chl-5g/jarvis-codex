import test from "node:test";
import assert from "node:assert/strict";
import { summarizeAudioFrames } from "../src/voice/audio-events.mjs";

test("summarizes local audio frames into cough and breath-like events", () => {
  const result = summarizeAudioFrames(
    [0.1, 0.1, 0.9, 0.1, 0.01, 0.01, 0.01, 0.1],
    16000,
    16000,
  );

  assert.equal(result.cough_count, 1);
  assert.equal(result.breathing, "possible");
  assert.equal(result.raw_audio_sent, false);
});

test("does not create events from an empty window", () => {
  const result = summarizeAudioFrames([], 16000, 16000);
  assert.equal(result.cough_count, 0);
  assert.equal(result.breathing, "unknown");
});
