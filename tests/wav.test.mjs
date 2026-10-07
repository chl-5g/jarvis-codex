import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const frontend = await readFile(new URL("../src/main.ts", import.meta.url), "utf8");
const style = await readFile(new URL("../src/style.css", import.meta.url), "utf8");
const backend = await readFile(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
const memoryBackend = await readFile(new URL("../src-tauri/src/memory.rs", import.meta.url), "utf8");
const qwenBackend = await readFile(new URL("../src-tauri/src/on_device_model.rs", import.meta.url), "utf8");
const promptsConfig = await readFile(new URL("../config/prompts.json", import.meta.url), "utf8");
const uiConfig = await readFile(new URL("../config/ui.json", import.meta.url), "utf8");
const voiceConfig = await readFile(new URL("../config/voice.json", import.meta.url), "utf8");
const toolsConfig = await readFile(new URL("../config/tools.json", import.meta.url), "utf8");
const eventsBackend = await readFile(new URL("../src-tauri/src/events.rs", import.meta.url), "utf8");
const toolsBackend = await readFile(new URL("../src-tauri/src/tools.rs", import.meta.url), "utf8");
const pdfspineBackend = await readFile(new URL("../src-tauri/src/pdfspine.rs", import.meta.url), "utf8");
const connectorsConfig = await readFile(new URL("../config/connectors.json", import.meta.url), "utf8");
const knowledgeBackend = await readFile(new URL("../src-tauri/src/knowledge.rs", import.meta.url), "utf8");
const workflowBackend = await readFile(new URL("../src-tauri/src/workflow.rs", import.meta.url), "utf8");
const tasksBackend = await readFile(new URL("../src-tauri/src/tasks.rs", import.meta.url), "utf8");
const bridgeBackend = await readFile(new URL("../src-tauri/src/bridge.rs", import.meta.url), "utf8");
const wakeHelper = await readFile(
  new URL("../src-tauri/wake-helper/JarvisWakeListener.swift", import.meta.url),
  "utf8",
);
const wakeConfig = await readFile(new URL("../config/wake.json", import.meta.url), "utf8");
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
const infoPlist = await readFile(new URL("../src-tauri/Info.plist", import.meta.url), "utf8");
const installScript = await readFile(new URL("../scripts/install-release.sh", import.meta.url), "utf8");
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

test("Voice capability results are returned through native Codex tools", () => {
  assert.match(backend, /item\/tool\/call/);
  assert.match(backend, /dynamicTools/);
  assert.doesNotMatch(frontend, /routeVoiceCapabilityTask/);
});

test("camera intent is described as autonomous visual inspection", () => {
  assert.match(toolsConfig, /what is in front of them/);
  assert.match(backend, /inputImage/);
  assert.match(backend, /data:image\/jpeg;base64/);
});

test("bundled Codex runtime inherits the macOS proxy for realtime connectivity", () => {
  assert.match(tauriConfig, /"codex"/);
  assert.match(codexWrapper, /scutil --proxy/);
  assert.match(codexWrapper, /HTTP_PROXY/);
  assert.match(codexWrapper, /HTTPS_PROXY/);
  assert.match(codexWrapper, /model=gpt-5\.6-sol/);
  assert.match(backend, /JARVIS_MODEL: &str = "gpt-5\.6-sol"/);
  assert.match(backend, /"model": JARVIS_MODEL/);
  assert.match(promptsConfig, /directly use Codex's native file-change and command-execution tools/);
  assert.match(promptsConfig, /Do not route direct file edits through Obsidian or any other GUI/);
});

test("bundled Codex runtime prefers one stable local CLI identity", () => {
  assert.match(codexWrapper, /JARVIS_REAL_CODEX_BIN:-\$HOME\/\.local\/bin\/codex/);
  assert.doesNotMatch(codexWrapper, /ChatGPT\.app/);
  assert.match(codexWrapper, /JARVIS_REAL_CODEX_BIN/);
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

test("wake reads memory before opening Voice", () => {
  assert.match(frontend, /prepare_wake_context/);
  assert.match(frontend, /await prepareWakeMemory\(\)/);
  assert.match(backend, /wake memory loaded/);
});

test("Voice sleeps after configured inactivity and waits for wake", () => {
  assert.match(frontend, /voiceIdleSleepTimer/);
  assert.match(frontend, /voiceIdleSleepMs/);
  assert.match(frontend, /VITE_VOICE_IDLE_SLEEP_MS/);
  assert.match(frontend, /sleepVoiceAfterIdle\(\)/);
  assert.match(frontend, /await armWakeListener\(\)/);
  assert.match(frontend, /VOICE_IDLE_SLEEP_MS/);
  assert.match(voiceConfig, /"voiceIdleSleepMs": 300000/);
});

test("tool descriptions are kept outside prompt policy", () => {
  assert.match(toolsConfig, /"descriptions"/);
  assert.doesNotMatch(promptsConfig, /"toolDescriptions"/);
});

test("pdfspine OCR is a discoverable connector with visible lifecycle events", () => {
  assert.match(toolsConfig, /"ocr_image"/);
  assert.match(toolsConfig, /"ocr_pdf"/);
  assert.match(toolsConfig, /"make_searchable_pdf"/);
  assert.match(connectorsConfig, /"pdfspine"/);
  assert.match(pdfspineBackend, /JARVIS_PDFSPINE_BIN/);
  assert.match(pdfspineBackend, /events::emit/);
  assert.match(frontend, /payload\.kind === "connector"/);
});

test("Voice degradation copy is configuration-driven", () => {
  assert.match(frontend, /uiConfig\.messages\.voicePermissionTitle/);
  assert.match(frontend, /uiConfig\.messages\.voicePermissionCopy/);
  assert.match(frontend, /uiConfig\.messages\.voiceConnectionTitle/);
  assert.match(uiConfig, /"voicePermissionCopy"/);
});

test("wake listener accepts Chinese greeting and English Hi Jarvis phrases", () => {
  assert.match(wakeConfig, /你好jarvis/);
  assert.match(wakeConfig, /"你好"/);
  assert.match(wakeConfig, /你好贾维斯/);
  assert.match(wakeConfig, /hi jarvis/);
  assert.match(wakeConfig, /hijarvis/);
  assert.match(wakeHelper, /forResource: "wake"/);
});

test("STOP suppresses transcript-tail handoffs and interrupts late turns", () => {
  assert.match(backend, /"flushTranscriptTailOnSessionEnd":\s*false/);
  assert.match(backend, /for _ in 0\.\.6/);
  assert.match(backend, /"turn\/interrupt"/);
});

test("typed commands use the normal Codex task turn", () => {
  assert.doesNotMatch(frontend, /文字指令已进入原生 Codex Voice/);
  assert.match(frontend, /Realtime Voice is an[\s\S]*audio transport/);
  assert.match(frontend, /正在发送文字指令到 Codex 任务线程/);
  assert.match(backend, /"turn\/start"/);
});

test("workspace is initialized before voice or text turns", () => {
  assert.match(frontend, /async function ensureWorkspace\(\)/);
  assert.match(frontend, /import pathsConfig from "\$PROJECT_PATH\/config\/paths\.json"/);
  assert.match(frontend, /const PROJECT_WORKSPACE = `\$\{PROJECT_ROOT\}\/\$\{pathsConfig\.workspace\}`/);
  assert.match(frontend, /let workspace = PROJECT_WORKSPACE/);
  assert.match(frontend, /value !== "\/"/);
  assert.match(frontend, /await ensureWorkspace\(\);\n  voiceStartInFlight/);
  assert.match(frontend, /await ensureWorkspace\(\);[\s\S]*const useLocalQwen = shouldUseLocalQwen/);
  assert.match(frontend, /savedWorkspace !== "\/"/);
});

test("Codex Voice is primary and local speech is only the fallback", () => {
  assert.doesNotMatch(frontend, /voiceReplyRoute|containsChinese|local-zh/);
  assert.match(frontend, /voice: "cove"/);
  assert.match(frontend, /正在发送文字指令到 Codex 任务线程/);
  assert.match(frontend, /invoke\("speak_text"/);
});

test("normal launch opens Codex Voice automatically", () => {
  assert.match(frontend, /!backgroundStart && state\.mode === "ready"/);
  assert.match(frontend, /等待你启用 Codex Voice/);
});

test("full permission auto-accepts server requests", () => {
  assert.match(frontend, /PERMISSION_KEY = "jarvis\.permissionMode:v2"/);
  assert.match(frontend, /permissionMode === "full"/);
  assert.match(frontend, /resolve_server_request.*approved: true/);
});

test("macOS file access uses one installed app identity", () => {
  assert.match(infoPlist, /NSDesktopFolderUsageDescription/);
  assert.match(infoPlist, /NSDocumentsFolderUsageDescription/);
  assert.match(infoPlist, /NSDownloadsFolderUsageDescription/);
  assert.match(installScript, /\/Applications\/Jarvis Codex\.app/);
  assert.match(installScript, /ditto/);
});

test("text input button is labelled SEND", () => {
  assert.match(frontend, /<button>SEND<\/button>/);
});

test("voice button is only a microphone mute toggle", () => {
  assert.match(frontend, /mic\.addEventListener\("click"/);
  assert.match(frontend, /setVoiceMuted\(!state\.muted\)/);
  assert.match(frontend, /track\.enabled = !muted/);
  assert.match(frontend, /MUTED/);
  assert.match(frontend, /UNMUTED/);
  const micHandler = frontend.match(/mic\.addEventListener\("click"[\s\S]*?\n\}\);/)?.[0] ?? "";
  assert.doesNotMatch(micHandler, /startDirectVoice|toggleVoiceMute/);
  assert.match(frontend, /class="slash-mark"/);
  assert.match(style, /\.mic \.slash-mark/);
});

test("action and URL text bypass local Qwen in hybrid mode", () => {
  assert.match(frontend, /shouldUseLocalQwen/);
});

test("idle text input uses the Codex task thread", () => {
  assert.match(frontend, /await invoke\("send_text", \{ text \}\)/);
  assert.match(frontend, /turn\/start/);
});

test("new runtime instructions do not inherit stale Obsidian-only file workflow", () => {
  assert.match(frontend, /jarvis\.threadId:v4:/);
  assert.match(promptsConfig, /HIGHEST PRIORITY FILE RULE/);
  assert.match(promptsConfig, /paths under ~\/notes/);
  assert.match(promptsConfig, /previous conversation preference to use Obsidian is superseded/);
  assert.match(promptsConfig, /Do not route direct file edits through Obsidian/);
  assert.match(backend, /config::prompt\("codexBaseInstructions"\)/);
});

test("pause control can resume Jarvis and swaps to a play icon", () => {
  assert.match(frontend, /state\.mode === "stopped" \|\| state\.manualStop/);
  assert.match(frontend, /正在恢复 Jarvis Voice/);
  assert.match(frontend, /stopLabel\.textContent = paused \? "RESUME" : "PAUSE"/);
  assert.match(frontend, /uiConfig\.status/);
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
  assert.match(wakeHelper, /requiresOnDeviceRecognition = false/);
  assert.match(wakeHelper, /contextualStrings = phrases/);
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

test("OpenAgentic memory bridge uses the existing four-layer Markdown layout", () => {
  assert.match(backend, /mod memory;/);
  assert.match(memoryBackend, /OPENAGENTIC_MEMORY_DIR/);
  assert.match(backend, /memory_recall/);
  assert.match(backend, /memory_save_episode/);
  assert.match(memoryBackend, /fn initial_context/);
  assert.match(memoryBackend, /fn skills_context/);
  assert.match(backend, /memory_context/);
});

test("local Qwen route keeps reasoning out of Jarvis rendering", () => {
  assert.match(backend, /local_qwen_chat/);
  assert.match(qwenBackend, /chat_template_kwargs/);
  assert.match(qwenBackend, /enable_thinking/);
  assert.match(frontend, /qwen-event/);
  assert.match(frontend, /local_qwen_chat/);
  assert.match(frontend, /modelMode/);
  assert.match(frontend, /[Rr]easoning/);
});

test("local Qwen can call the audited tool gateway and keeps the selected workspace", () => {
  assert.match(qwenBackend, /openai_schemas/);
  assert.match(qwenBackend, /MAX_TOOL_ROUNDS/);
  assert.match(qwenBackend, /tools::execute/);
  assert.match(qwenBackend, /"role":"tool"/);
  assert.match(qwenBackend, /JARVIS_ON_DEVICE_TOOLS/);
  assert.match(qwenBackend, /config::prompt\("toolPolicy"\)/);
  assert.match(promptsConfig, /Agent 工具层已经接入并可用/);
  assert.match(backend, /async fn local_qwen_chat/);
  assert.match(backend, /validated_workspace/);
  assert.match(frontend, /local_qwen_chat.*workspace/);
});

test("Codex, Qwen, tools, workflows, and tasks publish one local event envelope", () => {
  assert.match(eventsBackend, /schema_version/);
  assert.match(eventsBackend, /event_id/);
  assert.match(eventsBackend, /timestamp/);
  assert.match(eventsBackend, /source/);
  assert.match(eventsBackend, /jarvis-event/);
  assert.match(backend, /crate::events::emit\(&event_app, "codex"/);
  assert.match(qwenBackend, /crate::events::emit/);
  assert.match(toolsBackend, /crate::events::emit/);
  assert.match(workflowBackend, /crate::events::emit/);
  assert.match(tasksBackend, /crate::events::emit/);
});

test("Jarvis exposes online-first, local Qwen, and Codex model routes", () => {
  assert.match(frontend, /jarvis\.modelMode/);
  assert.match(frontend, /端侧模型/);
  assert.match(frontend, /在线优先/);
  assert.match(frontend, /Codex 原生/);
});

test("local OpenAgentic tool gateway exposes bounded audited execution", () => {
  assert.match(backend, /mod tools;/);
  assert.match(backend, /tool_list/);
  assert.match(backend, /tool_execute/);
  assert.match(toolsBackend, /jarvis-event/);
  assert.match(toolsBackend, /resolve_path/);
  assert.match(toolsBackend, /dangerous/);
  assert.match(toolsBackend, /MAX_OUTPUT/);
});

test("local knowledge bridge indexes Markdown roots and returns bounded source excerpts", () => {
  assert.match(backend, /mod knowledge;/);
  assert.match(backend, /knowledge_status/);
  assert.match(backend, /knowledge_scan/);
  assert.match(backend, /knowledge_search/);
  assert.match(backend, /KnowledgeStore::default\(\)\.context/);
  assert.match(knowledgeBackend, /JARVIS_KNOWLEDGE_ROOTS/);
  assert.match(knowledgeBackend, /incrementally indexed|Incrementally scan/);
  assert.match(knowledgeBackend, /Source:/);
});

test("file-backed workflows and tasks preserve explicit approval boundaries", () => {
  assert.match(backend, /mod workflow;/);
  assert.match(backend, /workflow_list/);
  assert.match(backend, /workflow_save/);
  assert.match(backend, /workflow_run/);
  assert.match(backend, /task_schedule/);
  assert.match(backend, /task_cancel/);
  assert.match(backend, /task_resume/);
  assert.match(backend, /task_run_due/);
  assert.match(workflowBackend, /JARVIS_WORKFLOW_FILE/);
  assert.match(workflowBackend, /approval-required/);
  assert.match(workflowBackend, /requires_approval/);
  assert.match(tasksBackend, /JARVIS_TASKS_FILE/);
  assert.match(tasksBackend, /status.*completed/);
  assert.match(frontend, /payload\.kind === "workflow" \|\| payload\.kind === "task"/);
});

test("future device bridge is disabled by default and loopback-token protected", () => {
  assert.match(backend, /bridge_status/);
  assert.match(backend, /bridge_enable/);
  assert.match(bridgeBackend, /127\.0\.0\.1/);
  assert.match(bridgeBackend, /POST.*\/command|\("POST", "\/command"\)/s);
  assert.match(bridgeBackend, /MAX_COMMAND_CHARS/);
  assert.match(bridgeBackend, /validate_bind_address/);
  assert.match(bridgeBackend, /pairing token/);
  assert.match(bridgeBackend, /Authorization/);
  assert.match(bridgeBackend, /MAX_EVENTS/);
  assert.match(frontend, /id="bridge-enable"/);
  assert.match(frontend, /bridge_enable/);
  assert.match(frontend, /jarvis\.bridgeToken:v1/);
});

test("wake activates the macOS app before focusing the Jarvis window", () => {
  assert.match(backend, /fn raise_jarvis_window/);
  assert.match(backend, /activateIgnoringOtherApps\(true\)/);
  assert.match(backend, /set_always_on_top\(true\)/);
  assert.match(backend, /set_always_on_top\(false\)/);
  assert.match(backend, /raise_jarvis_window\(&app\)/);
});

test("text routing sends action and URL prompts to Codex", async () => {
  const routing = await import("../src/text-routing.mjs");
  assert.equal(routing.shouldUseLocalQwen({ modelMode: "hybrid", voiceActive: false, text: "今天天气怎么样" }), false);
  assert.equal(routing.shouldUseLocalQwen({ modelMode: "hybrid", voiceActive: false, text: "请访问 https://github.com/chl-5g/cipherpipe" }), false);
  assert.equal(routing.shouldUseLocalQwen({ modelMode: "hybrid", voiceActive: false, text: "打开这个仓库" }), false);
  assert.equal(routing.shouldUseLocalQwen({ modelMode: "qwen", voiceActive: false, text: "请访问 https://example.com" }), false);
  assert.equal(routing.shouldUseLocalQwen({ modelMode: "hybrid", voiceActive: true, text: "普通问题" }), false);
  assert.equal(routing.parseCipherPipeCommand("发给 CipherPipe：请回复"), "请回复");
  assert.equal(routing.parseCipherPipeCommand("普通问题"), null);
});
