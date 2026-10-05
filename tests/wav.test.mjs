import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const frontend = await readFile(new URL("../src/main.ts", import.meta.url), "utf8");
const style = await readFile(new URL("../src/style.css", import.meta.url), "utf8");
const backend = await readFile(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
const wakeHelper = await readFile(
  new URL("../src-tauri/wake-helper/JarvisWakeListener.swift", import.meta.url),
  "utf8",
);
const entitlements = await readFile(
  new URL("../src-tauri/Entitlements.plist", import.meta.url),
  "utf8",
);
const helperEntitlements = await readFile(
  new URL("../src-tauri/wake-helper/Entitlements.plist", import.meta.url),
  "utf8",
);
const tauriConfig = await readFile(
  new URL("../src-tauri/tauri.conf.json", import.meta.url),
  "utf8",
);
const codexWrapper = await readFile(
  new URL("../src-tauri/codex", import.meta.url),
  "utf8",
);
const localTts = await readFile(
  new URL("../src-tauri/local_tts.py", import.meta.url),
  "utf8",
);

test("text replies are spoken by the local macOS voice fallback", () => {
  assert.match(backend, /async fn speak_text/);
  assert.match(backend, /local_tts\.py/);
  assert.doesNotMatch(backend, /\/usr\/bin\/say/);
  assert.match(localTts, /mlx_audio\.tts\.utils/);
  assert.match(localTts, /zm_yunxi/);
  assert.match(frontend, /invoke\("speak_text"/);
  assert.match(frontend, /function extractAgentText/);
  assert.match(frontend, /lastCompletedAgentText/);
  assert.match(frontend, /agentMessageBuffer\.trim\(\) \|\| lastCompletedAgentText/);
});

test("Voice uses Codex app-server V3 WebRTC directly", () => {
  assert.match(backend, /"version":\s*"v3"/);
  assert.match(backend, /"transport":\s*\{"type":\s*"webrtc"/);
  assert.match(backend, /"app-server",\s*"--enable",\s*"realtime_conversation",\s*"--stdio"/);
  assert.doesNotMatch(frontend, /OPENAI_API_KEY|ChatGPT.*button|hotkey/i);
});

test("bundled Codex runtime inherits the macOS proxy for realtime connectivity", () => {
  assert.match(tauriConfig, /"codex"/);
  assert.match(codexWrapper, /scutil --proxy/);
  assert.match(codexWrapper, /HTTP_PROXY/);
  assert.match(codexWrapper, /HTTPS_PROXY/);
  assert.match(codexWrapper, /model=gpt-5\.6-sol/);
  assert.match(backend, /JARVIS_MODEL: &str = "gpt-5\.6-sol"/);
  assert.match(backend, /"model": JARVIS_MODEL/);
  assert.match(backend, /directly use Codex's native file-change and command-execution tools/);
  assert.match(backend, /Do not route direct file edits through Obsidian or any other GUI/);
});

test("wake phrase opens the same direct Voice path", () => {
  assert.match(frontend, /listen<WakeEvent>\("jarvis-wake"/);
  assert.match(frontend, /void startDirectVoice\(\{ coldStart: payload\.cold === true \}\)/);
  assert.match(frontend, /const attempts = coldStart \? 6 : 1/);
  assert.match(frontend, /requestAnimationFrame\(\(\) => requestAnimationFrame/);
  assert.match(frontend, /recoverableColdStartError/);
  assert.match(backend, /"--host-app"/);
  assert.match(wakeHelper, /NSWorkspace\.shared\.openApplication/);
  assert.match(wakeHelper, /configuration\.arguments\s*=\s*\["--jarvis-wake"\]/);
  assert.match(frontend, /consume_cold_wake/);
  assert.match(backend, /AVAudioEngine releases the input device asynchronously/);
  assert.match(backend, /matches!\(authorization, "denied" \| "restricted"\)/);
  assert.match(backend, /requestAccessForMediaType_completionHandler/);
  assert.match(frontend, /request_microphone_permission/);
  assert.match(frontend, /startup_is_background/);
  assert.match(entitlements, /com\.apple\.security\.device\.audio-input/);
  assert.match(helperEntitlements, /com\.apple\.security\.device\.audio-input/);
  assert.match(backend, /tauri_plugin_autostart/);
  assert.match(frontend, /onCloseRequested/);
  assert.match(wakeHelper, /"--test-wake"/);
});

test("STOP suppresses transcript-tail handoffs and interrupts late turns", () => {
  assert.match(backend, /"flushTranscriptTailOnSessionEnd":\s*false/);
  assert.match(backend, /for _ in 0\.\.6/);
  assert.match(backend, /"turn\/interrupt"/);
});

test("text input can join the active Voice conversation", () => {
  assert.match(frontend, /append_codex_voice_text/);
  assert.match(backend, /"thread\/realtime\/appendText"/);
});

test("Codex Voice is primary and local speech is only the fallback", () => {
  assert.doesNotMatch(frontend, /voiceReplyRoute|containsChinese|local-zh/);
  assert.match(frontend, /voice: "cove"/);
  assert.match(frontend, /Codex Voice 尚未连接，改用本地模型语音播报/);
  assert.match(frontend, /invoke\("speak_text"/);
});

test("normal launch opens Codex Voice automatically", () => {
  assert.match(frontend, /!backgroundStart && state\.mode === "ready"/);
  assert.match(frontend, /void startDirectVoice\(\)/);
});

test("full permission auto-accepts server requests", () => {
  assert.match(frontend, /PERMISSION_KEY = "jarvis\.permissionMode:v2"/);
  assert.match(frontend, /permissionMode === "full"/);
  assert.match(frontend, /resolve_server_request.*approved: true/);
});

test("text input button is labelled SEND", () => {
  assert.match(frontend, /<button>SEND<\/button>/);
});

test("idle text input starts Codex Voice so replies keep the original Codex voice", () => {
  assert.match(frontend, /await startDirectVoice\(\)/);
  assert.match(frontend, /await waitForVoiceActive\(\)/);
  assert.match(frontend, /Codex Voice 尚未连接/);
});

test("new runtime instructions do not inherit stale Obsidian-only file workflow", () => {
  assert.match(frontend, /jarvis\.threadId:v2:/);
  assert.match(backend, /HIGHEST PRIORITY FILE RULE/);
  assert.match(backend, /paths under ~\/notes/);
  assert.match(backend, /previous conversation preference to use Obsidian is superseded/);
  assert.match(backend, /Do not route direct file edits through Obsidian/);
});

test("pause control can resume Jarvis and swaps to a play icon", () => {
  assert.match(frontend, /state\.mode === "stopped" \|\| state\.manualStop/);
  assert.match(frontend, /正在恢复 Jarvis Voice/);
  assert.match(frontend, /stopLabel\.textContent = paused \? "RESUME" : "PAUSE"/);
  assert.match(frontend, /stopped: \["PAUSED", "JARVIS PAUSED"\]/);
  assert.match(frontend, /play-mark/);
});

test("Jarvis renders a chronological SSE-style conversation stream", () => {
  assert.match(frontend, /id="event-stream"/);
  assert.match(frontend, /function appendStreamLine/);
  assert.match(frontend, /thread\/realtime\/transcript\/delta/);
  assert.match(frontend, /appendStreamLine\(assistantTranscriptBuffer, "assistant", "voice-assistant"\)/);
  assert.match(frontend, /appendStreamLine\(`开始：\$\{describeEventItem\(params\)\}`/);
  assert.match(frontend, /eventStream\.scrollTop = eventStream\.scrollHeight/);
  assert.match(frontend, /streamLines\.findLast|for \(let index = streamLines\.length - 1/);
  assert.match(frontend, /dialogue-current/);
  assert.match(frontend, /\[transcript, response\][\s\S]*current\.scrollTop = current\.scrollHeight/);
  assert.match(style, /\.dialogue-current p\{max-height:48px;overflow:auto/);
  assert.match(style, /\.shell \.dialogue\{display:flex!important;flex-direction:column/);
});

test("production configuration persists workspace and resumes threads", () => {
  assert.match(frontend, /jarvis\.workspace/);
  assert.match(frontend, /jarvis\.threadId:/);
  assert.match(backend, /"thread\/resume"/);
  assert.match(backend, /validated_workspace/);
  assert.match(wakeHelper, /requiresOnDeviceRecognition = true/);
});

test("user can create a fresh Codex thread without deleting history", () => {
  assert.match(frontend, /id="new-thread"/);
  assert.match(frontend, /threadId:\s*null/);
  assert.match(frontend, /invoke<Session>\("start_jarvis"/);
  assert.match(frontend, /freshSession\.threadId/);
  assert.match(frontend, /原线程仍保留在 Codex 历史记录中/);
});

test("permission profiles are persisted and mapped by the trusted backend", () => {
  assert.match(frontend, /jarvis\.permissionMode/);
  assert.match(frontend, /type PermissionMode = "safe" \| "auto" \| "full"/);
  assert.match(frontend, /permissionMode,/);
  assert.match(backend, /enum PermissionMode/);
  assert.match(backend, /approval_policy: "on-request"/);
  assert.match(backend, /approval_policy: "never"/);
  assert.match(backend, /sandbox: "workspace-write"/);
  assert.match(backend, /sandbox: "danger-full-access"/);
  assert.match(backend, /existing\.permission_mode == permission_mode/);
});

test("speaker verification is opt-in and Computer Use is open by default", () => {
  assert.match(frontend, /type SpeakerAccess = "unknown" \| "allen" \| "rejected"/);
  assert.match(frontend, /const SPEAKER_GATE_ENABLED = false/);
  assert.match(frontend, /speakerAccess: "allen"/);
  assert.match(frontend, /speakerAccess: state\.speakerAccess/);
  assert.match(frontend, /已开放：完全访问/);
  assert.match(backend, /enum SpeakerAccess/);
  assert.match(backend, /SpeakerAccess::Unknown/);
  assert.match(backend, /SpeakerAccess::Allen/);
  assert.match(backend, /SpeakerAccess::Rejected/);
  assert.match(backend, /use Computer Use and desktop-control tools/);
  assert.match(backend, /fn speaker_gate_enabled/);
  assert.match(backend, /JARVIS_SPEAKER_GATE/);
  assert.match(backend, /effective_speaker_access/);
  assert.match(backend, /speaker_access\.instructions\(\)/);
});

test("wake activates the macOS app before focusing the Jarvis window", () => {
  assert.match(backend, /fn raise_jarvis_window/);
  assert.match(backend, /activateIgnoringOtherApps\(true\)/);
  assert.match(backend, /set_always_on_top\(true\)/);
  assert.match(backend, /set_always_on_top\(false\)/);
  assert.match(backend, /raise_jarvis_window\(&app\)/);
});
