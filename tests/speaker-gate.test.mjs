import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const script = await readFile(new URL("../scripts/speaker_gate.py", import.meta.url), "utf8");
const runtimeScript = await readFile(new URL("../src-tauri/speaker_identity.py", import.meta.url), "utf8");

test("local speaker gate has explicit enrollment and fail-closed verification", () => {
  assert.match(script, /def enroll\(/);
  assert.match(script, /def verify\(/);
  assert.match(script, /speakerAccess.*unknown/);
  assert.match(script, /access = "allen" if score >= threshold else "rejected"/);
  assert.match(script, /speakerAccess[\s\S]*allen/);
  assert.match(script, /at least two voice samples/);
  assert.match(script, /score >= threshold/);
  assert.match(script, /Exit codes: 0 = Allen, 1 = rejected speaker, 2 = unknown/);
  assert.match(script, /def serve\(/);
  assert.match(script, /json\.loads\(line\)/);
  assert.match(script, /flush=True/);
});

test("runtime verifier trims silence and reports capture quality", () => {
  assert.match(runtimeScript, /def trim_silence\(/);
  assert.match(runtimeScript, /durationMs/);
  assert.match(runtimeScript, /rms/);
});
