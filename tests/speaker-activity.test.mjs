import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";

test("speaker activity keeps noise out of voiceprint extraction", () => {
  const result = spawnSync("python3", ["-m", "unittest", "discover", "-s", "tests", "-p", "speaker_activity_test.py"], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stdout + result.stderr);
});
