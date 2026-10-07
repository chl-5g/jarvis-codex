import test from "node:test";
import assert from "node:assert/strict";
import { runWorkflow } from "../src/workflows/voice-workflow.mjs";

test("runWorkflow executes voice steps in order and completes once", async () => {
  const calls = [];
  const events = [];
  const result = await runWorkflow([
    { name: "capture-speaker", run: async () => { calls.push("capture"); return { speakerAccess: "allen" }; } },
    { name: "initialize-codex", run: async () => { calls.push("codex"); return { threadId: "thread-1" }; } },
    { name: "load-memory", run: async () => { calls.push("memory"); return { privateMemoryLoaded: true }; } },
    { name: "connect-voice", run: async () => { calls.push("voice"); return { voiceActive: true }; } },
  ], {
    onStepStart: (name) => events.push(`start:${name}`),
    onStepSuccess: (name) => events.push(`success:${name}`),
    onComplete: () => events.push("complete"),
  });

  assert.deepEqual(calls, ["capture", "codex", "memory", "voice"]);
  assert.equal(result.state, "ready");
  assert.deepEqual(events, [
    "start:capture-speaker", "success:capture-speaker",
    "start:initialize-codex", "success:initialize-codex",
    "start:load-memory", "success:load-memory",
    "start:connect-voice", "success:connect-voice", "complete",
  ]);
});

test("runWorkflow stops at the failed step and does not complete", async () => {
  const calls = [];
  const events = [];
  const result = await runWorkflow([
    { name: "capture-speaker", run: async () => { calls.push("capture"); return {}; } },
    { name: "initialize-codex", run: async () => { calls.push("codex"); throw new Error("initialize failed"); } },
    { name: "load-memory", run: async () => { calls.push("memory"); return {}; } },
  ], {
    onFailure: (name, error) => events.push(`failure:${name}:${error.message}`),
    onComplete: () => events.push("complete"),
  });

  assert.deepEqual(calls, ["capture", "codex"]);
  assert.equal(result.state, "degraded");
  assert.equal(result.failedStep, "initialize-codex");
  assert.deepEqual(events, ["failure:initialize-codex:initialize failed"]);
});
