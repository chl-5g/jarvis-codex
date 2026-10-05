import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./style.css";

type Mode = "booting" | "ready" | "voice-starting" | "listening" | "working" | "speaking" | "degraded" | "stopped";
type Message = { id?: number | string; method?: string; params?: any };
type Session = { threadId: string; cwd: string };
type DirectVoice = {
  codexConnected: boolean;
  voiceActive: boolean;
  phase: string;
  protocol: string;
  threadId?: string;
  realtimeSessionId?: string;
};
type WakeStatus = {
  enabled: boolean;
  ready: boolean;
  authorization: string;
};
type WakeEvent = {
  ok: boolean;
  error?: string;
  cold?: boolean;
  /** Set by the local speaker verifier when one is installed. */
  speaker?: SpeakerAccess;
  speakerAccess?: SpeakerAccess;
};
type PermissionMode = "safe" | "auto" | "full";
type SpeakerAccess = "unknown" | "allen" | "rejected";
type ModelMode = "hybrid" | "qwen" | "codex";
type QwenEvent = { delta?: string; done?: boolean; error?: string };
type JarvisEvent = {
  kind?: string;
  phase?: string;
  callId?: string;
  toolName?: string;
  runId?: string;
  workflowId?: string;
  taskId?: string;
  stepId?: string;
  message?: string;
  output?: string;
  error?: string;
};
const SPEAKER_GATE_ENABLED = false;

const state = {
  mode: "booting" as Mode,
  session: null as Session | null,
  directVoice: null as DirectVoice | null,
  wake: null as WakeStatus | null,
  level: 0,
  manualStop: false,
  agentWorking: false,
  // This is a single-user local deployment. Keep Computer Use available
  // without waiting for the optional voiceprint verifier.
  speakerAccess: "allen" as SpeakerAccess,
};

const WORKSPACE_KEY = "jarvis.workspace";
// Bump this when runtime instructions change materially. Older threads may
// contain stale workflow preferences (for example, routing file edits through
// Obsidian), so a new runtime policy must not inherit that conversation state.
const THREAD_KEY_PREFIX = "jarvis.threadId:v2:";
// Use a new key so an older session that was left in safe mode does not make
// the single-user deployment ask for approval on every task.
const PERMISSION_KEY = "jarvis.permissionMode:v2";
const MODEL_MODE_KEY = "jarvis.modelMode:v1";
const permissionLabels: Record<PermissionMode, string> = {
  safe: "安全模式 · 需要时确认",
  auto: "自动办公 · 当前目录自主执行",
  full: "完全访问 · 高风险",
};
const modelModeLabels: Record<ModelMode, string> = {
  hybrid: "混合模式 · Voice 优先，本地 Qwen 兜底",
  qwen: "本地 Qwen · 8080",
  codex: "Codex 原生 · 工作线程",
};
function storedPermissionMode(): PermissionMode {
  const value = localStorage.getItem(PERMISSION_KEY);
  return value === "safe" ? "safe" : "full";
}
let workspace = "";
let permissionMode = storedPermissionMode();
function storedModelMode(): ModelMode {
  const value = localStorage.getItem(MODEL_MODE_KEY);
  return value === "qwen" || value === "codex" ? value : "hybrid";
}
let modelMode = storedModelMode();
const savedThreadId = () => localStorage.getItem(`${THREAD_KEY_PREFIX}${workspace}`);
let peer: RTCPeerConnection | null = null;
let microphoneStream: MediaStream | null = null;
let remoteStream: MediaStream | null = null;
let audioContext: AudioContext | null = null;
let microphoneAnalyser: AnalyserNode | null = null;
let remoteAnalyser: AnalyserNode | null = null;
let userTranscriptBuffer = "";
let assistantTranscriptBuffer = "";
let agentMessageBuffer = "";
let lastCompletedAgentText = "";
let lastUserTurnText = "";
let qwenAnswerBuffer = "";
let voiceStartInFlight = false;
let recoverableColdStartError = false;
const voiceAudio = new Audio();
voiceAudio.autoplay = true;

function extractAgentText(value: unknown): string {
  if (typeof value === "string") return value;
  if (!value || typeof value !== "object") return "";
  if (Array.isArray(value)) return value.map(extractAgentText).filter(Boolean).join("\n");
  const record = value as Record<string, unknown>;
  for (const key of ["text", "transcript", "output_text", "message", "content", "parts"]) {
    const text = extractAgentText(record[key]);
    if (text) return text;
  }
  return "";
}

async function saveMemoryEpisode(title: string, summary: string, tags: string[]) {
  if (!currentWindow || !summary.trim()) return;
  await invoke("memory_save_episode", { title, summary, tags }).catch((error) => {
    appendStreamLine(`记忆保存失败：${String(error)}`, "error");
  });
}

const previewParams = new URLSearchParams(window.location.search);
const tauriInternals = (window as Window & { __TAURI_INTERNALS__?: { invoke?: unknown } }).__TAURI_INTERNALS__;
const currentWindow = typeof tauriInternals?.invoke === "function" ? getCurrentWindow() : null;
const previewModeValue = previewParams.get("preview");
const previewActionValue = previewParams.get("action");
const visualPreviewMode: Mode = ["booting", "ready", "voice-starting", "listening", "working", "speaking", "degraded", "stopped"].includes(previewModeValue ?? "")
  ? previewModeValue as Mode
  : "ready";
if (!currentWindow) {
  document.documentElement.classList.add("visual-preview");
  if (previewParams.get("grid") === "1") document.documentElement.classList.add("transparency-grid");
}
if (currentWindow) {
  void currentWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    await currentWindow.hide();
  });
}

document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
<main class="shell" data-mode="booting">
  <canvas id="particle-field" width="1440" height="900" aria-hidden="true"></canvas>
  <header class="topbar hud-panel">
    <div class="brand"><i></i><strong>JARVIS</strong><span></span><em>CODEX VOICE SYSTEM</em></div>
    <div class="status"><i></i><b id="mode-label">INITIALIZING</b></div>
    <button id="settings" class="icon-button" aria-label="设置">⌘</button>
  </header>
  <section class="stage">
    <div class="avatar-window">
      <div class="character-aura"></div>
      <div class="character-rig">
        <div class="assembly-orbits" aria-hidden="true"><i></i><i></i><i></i></div>
        <div class="armor-shards" aria-hidden="true"></div>
        <img id="jarvis-character" class="helmet-character" src="/assets/jarvis-character-v2.png" alt="Jarvis holographic helmet">
        <div class="helmet-scan"></div>
        <div class="assembly-flash" aria-hidden="true"></div>
      </div>
      <canvas id="wave" width="900" height="120"></canvas>
    </div>
    <div class="identity"><span>JARVIS CORE</span><b id="identity-state">SYSTEM BOOT</b></div>
  </section>
  <aside class="workers">
    <article class="worker active" data-role="orchestrator"><span>›_</span><div><b>Codex</b><small>Connecting</small></div><i></i></article>
    <article class="worker" data-role="developer"><span>⌬</span><div><b>Developer</b><small>Standby</small></div><i></i></article>
    <article class="worker" data-role="researcher"><span>⌕</span><div><b>Researcher</b><small>Standby</small></div><i></i></article>
    <article class="worker" data-role="reviewer"><span>✓</span><div><b>Reviewer</b><small>Standby</small></div><i></i></article>
  </aside>
  <section class="dialogue hud-panel">
    <div class="dialogue-title"><b>JARVIS STREAM</b><small id="stream-state">LIVE</small></div>
    <div id="event-stream" class="event-stream" aria-live="polite" aria-label="Jarvis 实时对话信息"></div>
    <div class="dialogue-current">
      <b>YOU</b><p id="user-transcript">“嗨，Jarvis”</p>
      <b class="jarvis">JARVIS</b><p id="assistant-transcript">正在连接 Codex 原生任务线程…</p>
    </div>
  </section>
  <footer class="controls">
    <button id="mic" class="control mic"><span aria-hidden="true"><svg viewBox="0 0 24 24"><rect x="8.25" y="3" width="7.5" height="11.5" rx="3.75"></rect><path d="M5.5 11.25v.75a6.5 6.5 0 0 0 13 0v-.75M12 18.5V22M8.75 22h6.5"></path></svg></span><b>CODEX VOICE</b><small>V3 WEBRTC · DIRECT</small></button>
    <form id="command-form" class="command"><input id="command-input" aria-label="文字指令" placeholder="Voice 不可用时，发送本地 Codex 文字任务…" autocomplete="off"><button>SEND</button></form>
    <button id="stop" class="control stop" aria-label="暂停 Jarvis"><span id="stop-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><rect class="stop-mark" x="6.5" y="6.5" width="11" height="11" rx="1.8"></rect></svg></span><b id="stop-label">PAUSE</b><small id="stop-hint">PAUSE ALL</small></button>
  </footer>
  <div id="degraded-banner" class="degraded-banner" hidden><b>JARVIS NEEDS PERMISSION</b><span id="degraded-copy">首次使用请允许麦克风和语音识别。</span></div>
  <dialog id="approval"><h2>高风险操作确认</h2><p id="approval-copy">Codex 请求执行需要确认的动作。</p><div><button id="deny">拒绝</button><button id="approve">允许一次</button></div></dialog>
  <dialog id="settings-dialog"><h2>JARVIS SYSTEM</h2><dl><dt>Wake phrase</dt><dd>嗨 Jarvis / Hey Jarvis</dd><dt>Wake listener</dt><dd id="wake-auth">检测中</dd><dt>Computer Use</dt><dd id="speaker-auth">已开放：完全访问</dd><dt>Codex thread</dt><dd id="thread-id">—</dd><dt>Workspace</dt><dd id="workspace">—</dd><dt>Permission</dt><dd id="permission-mode-label">—</dd><dt>Model route</dt><dd id="model-mode-label">—</dd><dt>Voice kernel</dt><dd id="voice-auth">检测中</dd></dl><label class="workspace-setting">工作目录<input id="workspace-setting" autocomplete="off" spellcheck="false"></label><fieldset class="permission-setting"><legend>Codex 操作权限</legend><label><input type="radio" name="permission-mode" value="safe"><span><b>安全模式</b><small>超出当前目录或高风险操作时询问</small></span></label><label class="recommended"><input type="radio" name="permission-mode" value="auto"><span><b>自动办公</b><small>当前目录内自主执行，越界操作直接阻止</small></span><em>推荐</em></label><label class="danger"><input type="radio" name="permission-mode" value="full"><span><b>完全访问</b><small>不限制目录且不询问，请谨慎使用</small></span></label></fieldset><fieldset class="permission-setting model-setting"><legend>模型路由</legend><label class="recommended"><input type="radio" name="model-mode" value="hybrid"><span><b>混合模式</b><small>Voice 在线时用 Codex，文字任务优先本地 Qwen</small></span><em>推荐</em></label><label><input type="radio" name="model-mode" value="qwen"><span><b>本地 Qwen</b><small>所有文字任务走本机 8080 推理服务</small></span></label><label><input type="radio" name="model-mode" value="codex"><span><b>Codex 原生</b><small>文字任务接入 Codex 工作线程</small></span></label></fieldset><p>权限切换会停止当前任务并重建 Codex 运行时，但会继续使用当前工作目录保存的 thread。</p><p>修改工作目录后，下次重启 Jarvis 生效。每个工作目录会续接自己的 Codex thread。</p><p>模型路由修改立即保存；混合模式在语音关闭时自动使用本地 Qwen。</p><p>“新开线程”会结束当前任务并创建一个全新的 Codex thread；原线程仍保留在 Codex 历史记录中。</p><p>唤醒词在本机识别；Jarvis 页面通过 Codex app-server V3 WebRTC 进入官方 Voice 线程。认证复用本机 Codex 登录，不读取凭据、不模拟点击，也不建立第二套 GPT-Live。</p><div class="settings-actions"><button id="new-thread" class="new-thread">＋ 新开线程</button><span></span><button id="save-settings">保存</button><button id="close-settings">关闭</button></div></dialog>
</main>`;

const $ = <T extends Element>(selector: string) => document.querySelector<T>(selector)!;
const shell = $<HTMLElement>(".shell");
const transcript = $("#user-transcript");
const response = $("#assistant-transcript");
const eventStream = $("#event-stream");
const streamState = $("#stream-state");
const banner = $("#degraded-banner") as HTMLDivElement;
const mic = $("#mic") as HTMLButtonElement;
const approval = $("#approval") as HTMLDialogElement;
const settings = $("#settings-dialog") as HTMLDialogElement;
const characterRig = $<HTMLElement>(".character-rig");
const hoverControls = $<HTMLElement>(".controls");
const settingsButton = $<HTMLButtonElement>("#settings");
const stopButton = $<HTMLButtonElement>("#stop");
const stopIcon = $("#stop-icon");
const stopLabel = $("#stop-label");
const stopHint = $("#stop-hint");
let controlsHideTimer: number | undefined;
let characterActionTimer: number | undefined;
type StreamKind = "user" | "assistant" | "task" | "tool" | "system" | "error";
type StreamLine = { kind: StreamKind; text: string; key?: string; time: string };
const streamLines: StreamLine[] = [];

function streamTime() {
  return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function renderStream() {
  const fragment = document.createDocumentFragment();
  for (const line of streamLines) {
    const row = document.createElement("div");
    row.className = `stream-line stream-${line.kind}`;
    const time = document.createElement("time");
    time.textContent = line.time;
    const body = document.createElement("span");
    body.textContent = line.text;
    row.append(time, body);
    fragment.append(row);
  }
  eventStream.replaceChildren(fragment);
  eventStream.scrollTop = eventStream.scrollHeight;
  for (const current of [transcript, response]) current.scrollTop = current.scrollHeight;
}

function findOpenStreamLine(key: string) {
  for (let index = streamLines.length - 1; index >= 0; index -= 1) {
    const line = streamLines[index];
    if (line.key === key) return line;
  }
  return undefined;
}

function appendStreamLine(text: string, kind: StreamKind = "system", key?: string) {
  const clean = text.trim();
  if (!clean) return;
  const previous = key ? findOpenStreamLine(key) : undefined;
  if (previous && previous.key === key) {
    previous.text = clean;
    previous.kind = kind;
  } else {
    streamLines.push({ kind, text: clean, key, time: streamTime() });
    if (streamLines.length > 80) streamLines.splice(0, streamLines.length - 80);
  }
  renderStream();
}

function sealStreamLine(key: string) {
  const previous = findOpenStreamLine(key);
  if (previous?.key === key) delete previous.key;
}

function describeEventItem(params: any) {
  const item = params?.item ?? params;
  const type = String(item?.type ?? "任务步骤");
  const role = roleOf(params);
  if (type === "agentMessage") return "Jarvis 回复";
  if (type.toLowerCase().includes("command")) return `${role} 执行电脑指令`;
  if (type.toLowerCase().includes("file")) return `${role} 处理文件`;
  if (type.toLowerCase().includes("mcp") || type.toLowerCase().includes("tool")) return `${role} 调用工具`;
  return `${role}：${type}`;
}

function revealControls() {
  if (controlsHideTimer !== undefined) window.clearTimeout(controlsHideTimer);
  shell.classList.add("controls-visible");
}

function scheduleControlsHide() {
  if (controlsHideTimer !== undefined) window.clearTimeout(controlsHideTimer);
  controlsHideTimer = window.setTimeout(() => shell.classList.remove("controls-visible"), 1500);
}

type CharacterAction = "acknowledge" | "approval" | "complete" | "error";

function triggerCharacterAction(action: CharacterAction, duration = 1100) {
  if (characterActionTimer !== undefined) window.clearTimeout(characterActionTimer);
  for (const name of ["acknowledge", "approval", "complete", "error"] as const) {
    shell.classList.remove(`action-${name}`);
  }
  void shell.clientWidth;
  shell.classList.add(`action-${action}`);
  characterActionTimer = window.setTimeout(() => shell.classList.remove(`action-${action}`), duration);
}

for (const area of [characterRig, hoverControls, settingsButton]) {
  area.addEventListener("pointerenter", revealControls);
  area.addEventListener("pointerleave", scheduleControlsHide);
}
shell.addEventListener("focusin", (event) => {
  if (event.target instanceof Element && (hoverControls.contains(event.target) || settingsButton.contains(event.target))) revealControls();
});
shell.addEventListener("focusout", scheduleControlsHide);

let approvalId: number | string | undefined;
const copy: Record<Mode, [string, string]> = {
  booting: ["INITIALIZING", "SYSTEM BOOT"], ready: ["READY", "CODEX VOICE STANDBY"],
  "voice-starting": ["VOICE LINKING", "OPENING CODEX VOICE"], listening: ["LISTENING", "OFFICIAL VOICE ONLINE"],
  working: ["CODEX WORKING", "TASK EXECUTION"], speaking: ["JARVIS SPEAKING", "VOICE OUTPUT"],
  degraded: ["PERMISSION NEEDED", "WAKE SYSTEM OFFLINE"], stopped: ["PAUSED", "JARVIS PAUSED"],
};

function setMode(mode: Mode) {
  state.mode = mode; shell.setAttribute("data-mode", mode);
  $("#mode-label").textContent = copy[mode][0]; $("#identity-state").textContent = copy[mode][1];
  const paused = mode === "stopped";
  stopButton.setAttribute("aria-label", paused ? "恢复 Jarvis" : "暂停 Jarvis");
  stopLabel.textContent = paused ? "RESUME" : "PAUSE";
  stopHint.textContent = paused ? "RESUME VOICE" : "PAUSE ALL";
  stopIcon.innerHTML = paused
    ? '<svg viewBox="0 0 24 24"><path class="play-mark" d="M8 5.8v12.4a1.2 1.2 0 0 0 1.85 1.02l9.2-6.2a1.23 1.23 0 0 0 0-2.04l-9.2-6.2A1.2 1.2 0 0 0 8 5.8Z"></path></svg>'
    : '<svg viewBox="0 0 24 24"><rect class="stop-mark" x="6.5" y="6.5" width="11" height="11" rx="1.8"></rect></svg>';
  if (mode === "voice-starting") {
    shell.classList.remove("is-forming");
    void shell.clientWidth;
    shell.classList.add("is-forming");
    startParticleFormation();
  }
}
function setWorker(role: string, label: string, active = true) {
  const card = document.querySelector<HTMLElement>(`.worker[data-role="${role}"]`);
  if (!card) return;
  card.classList.toggle("active", active); card.querySelector("small")!.textContent = label;
}
function updateSpeakerAccess(access: SpeakerAccess) {
  const label = !SPEAKER_GATE_ENABLED
    ? "已开放：完全访问"
    : access === "allen"
    ? "Allen：允许 Computer Use"
    : access === "rejected"
      ? "未识别：任务已拒绝"
      : "未识别：仅回答，不控制电脑";
  $("#speaker-auth").textContent = label;
}
function roleOf(params: any) {
  const text = JSON.stringify(params ?? {}).toLowerCase();
  return text.includes("research") ? "researcher" : text.includes("review") ? "reviewer" :
    text.includes("developer") || text.includes("commandexecution") || text.includes("filechange") ? "developer" : "orchestrator";
}
function drawWave() {
  const canvas = $("#wave") as HTMLCanvasElement, context = canvas.getContext("2d")!;
  context.clearRect(0, 0, canvas.width, canvas.height);
  const time = performance.now() / 370, amplitude = 5 + state.level * 42 + (state.mode === "speaking" ? 22 : 0);
  context.beginPath();
  for (let x = 0; x <= canvas.width; x += 3) {
    const y = canvas.height / 2 + (Math.sin(x * .085 + time * 2.2) + Math.sin(x * .031 - time) * .55) * amplitude * Math.sin(x / canvas.width * Math.PI) * .48;
    x ? context.lineTo(x, y) : context.moveTo(x, y);
  }
  context.strokeStyle = state.mode === "working" ? "#ff9d2e" : state.mode === "stopped" ? "#ff3d33" : "#22c7ff";
  context.shadowColor = context.strokeStyle; context.shadowBlur = 15; context.lineWidth = 2; context.stroke();
  state.level *= .9; requestAnimationFrame(drawWave);
}
drawWave();

type VisualParticle = {
  fromX: number;
  fromY: number;
  targetX: number;
  targetY: number;
  size: number;
  phase: number;
  delay: number;
  curve: number;
  amber: boolean;
};

const particleCanvas = $("#particle-field") as HTMLCanvasElement;
const particleContext = particleCanvas.getContext("2d")!;
const characterImage = $("#jarvis-character") as HTMLImageElement;
const armorShardLayer = $<HTMLElement>(".armor-shards");
const armorShardSpecs = [
  ["polygon(35% 4%,65% 4%,63% 23%,37% 23%)", 0, -390, -8, 80],
  ["polygon(17% 9%,37% 4%,38% 31%,22% 35%)", -430, -280, -24, 0],
  ["polygon(63% 4%,83% 9%,78% 35%,62% 31%)", 430, -280, 24, 20],
  ["polygon(37% 22%,63% 22%,61% 45%,39% 45%)", 30, -320, 10, 170],
  ["polygon(20% 31%,39% 27%,43% 48%,19% 49%)", -470, -120, -32, 100],
  ["polygon(61% 27%,80% 31%,81% 49%,57% 48%)", 470, -120, 32, 120],
  ["polygon(16% 45%,43% 44%,46% 57%,20% 59%)", -520, -20, -18, 220],
  ["polygon(57% 44%,84% 45%,80% 59%,54% 57%)", 520, -20, 18, 240],
  ["polygon(42% 43%,58% 43%,59% 70%,41% 70%)", 0, 390, -10, 300],
  ["polygon(19% 56%,42% 54%,41% 73%,24% 78%)", -480, 180, -28, 280],
  ["polygon(58% 54%,81% 56%,76% 78%,59% 73%)", 480, 180, 28, 300],
  ["polygon(24% 73%,42% 68%,45% 84%,31% 88%)", -340, 330, 22, 390],
  ["polygon(58% 68%,76% 73%,69% 88%,55% 84%)", 340, 330, -22, 410],
  ["polygon(41% 68%,59% 68%,56% 88%,44% 88%)", 40, 430, 14, 470],
  ["polygon(31% 85%,45% 82%,44% 96%,36% 94%)", -250, 460, -30, 500],
  ["polygon(55% 82%,69% 85%,64% 94%,56% 96%)", 250, 460, 30, 520],
  ["polygon(43% 86%,57% 86%,56% 98%,44% 98%)", 0, 520, -12, 560],
  ["polygon(12% 25%,24% 19%,22% 45%,14% 52%)", -560, -210, -38, 180],
  ["polygon(76% 19%,88% 25%,86% 52%,78% 45%)", 560, -210, 38, 200],
] as const;

for (const [clip, translateX, translateY, rotation, delay] of armorShardSpecs) {
  const shard = document.createElement("i");
  shard.className = "armor-shard";
  shard.style.setProperty("--shard-clip", clip);
  shard.style.setProperty("--shard-x", `${translateX}px`);
  shard.style.setProperty("--shard-y", `${translateY}px`);
  shard.style.setProperty("--shard-rotation", `${rotation}deg`);
  shard.style.setProperty("--shard-delay", `${delay}ms`);
  if (Math.abs(rotation) >= 28 || delay % 3 === 0) shard.classList.add("amber-edge");
  armorShardLayer.append(shard);
}

let visualParticles: VisualParticle[] = [];
let formationStartedAt = -10_000;
let particleTargetBounds = { left: 0, top: 0, width: 1, height: 1 };
const FORMATION_DURATION = 3200;

function scatterParticleFromWindowEdge(particle: VisualParticle) {
  const edge = Math.floor(Math.random() * 4);
  const inset = Math.random() * 24;
  if (edge === 0) {
    particle.fromX = inset;
    particle.fromY = Math.random() * particleCanvas.height;
  } else if (edge === 1) {
    particle.fromX = particleCanvas.width - inset;
    particle.fromY = Math.random() * particleCanvas.height;
  } else {
    particle.fromX = Math.random() * particleCanvas.width;
    particle.fromY = edge === 2 ? inset : particleCanvas.height - inset;
  }
}

function syncParticleCanvasSize() {
  const bounds = shell.getBoundingClientRect();
  const width = Math.max(1, Math.round(bounds.width));
  const height = Math.max(1, Math.round(bounds.height));
  if (particleCanvas.width !== width) particleCanvas.width = width;
  if (particleCanvas.height !== height) particleCanvas.height = height;
}

function prepareVisualParticles() {
  if (!characterImage.naturalWidth) return;
  syncParticleCanvasSize();
  const sample = document.createElement("canvas");
  sample.width = particleCanvas.width;
  sample.height = particleCanvas.height;
  const context = sample.getContext("2d", { willReadFrequently: true })!;
  const shellBounds = shell.getBoundingClientRect();
  const characterBounds = characterImage.getBoundingClientRect();
  const left = characterBounds.left - shellBounds.left;
  const top = characterBounds.top - shellBounds.top;
  particleTargetBounds = { left, top, width: characterBounds.width, height: characterBounds.height };
  context.drawImage(characterImage, left, top, characterBounds.width, characterBounds.height);
  const pixels = context.getImageData(0, 0, sample.width, sample.height).data;
  visualParticles = [];
  for (let y = 0; y < sample.height; y += 7) {
    for (let x = 0; x < sample.width; x += 7) {
      const alpha = pixels[(y * sample.width + x) * 4 + 3];
      if (alpha < 46 || Math.random() > .82) continue;
      const particle: VisualParticle = {
        fromX: 0,
        fromY: 0,
        targetX: x + (Math.random() - .5) * 5,
        targetY: y + (Math.random() - .5) * 5,
        size: Math.random() > .965 ? 2.6 + Math.random() * 1.8 : .45 + Math.random() * 1.65,
        phase: Math.random() * Math.PI * 2,
        delay: Math.random() * .3,
        curve: (Math.random() - .5) * (90 + Math.random() * 190),
        amber: Math.random() < .16,
      };
      scatterParticleFromWindowEdge(particle);
      visualParticles.push(particle);
    }
  }
}

function startParticleFormation() {
  if (!visualParticles.length) prepareVisualParticles();
  formationStartedAt = performance.now();
  for (const particle of visualParticles) scatterParticleFromWindowEdge(particle);
}

function easeFormation(value: number) {
  const clamped = Math.max(0, Math.min(1, value));
  return clamped * clamped * (3 - 2 * clamped);
}

function particlePosition(particle: VisualParticle, progress: number) {
  const eased = easeFormation(progress);
  const deltaX = particle.targetX - particle.fromX;
  const deltaY = particle.targetY - particle.fromY;
  const distance = Math.max(1, Math.hypot(deltaX, deltaY));
  const bend = Math.sin(eased * Math.PI) * particle.curve;
  return {
    x: particle.fromX + deltaX * eased - deltaY / distance * bend,
    y: particle.fromY + deltaY * eased + deltaX / distance * bend,
  };
}

function drawFormationEnergy(now: number, rawProgress: number) {
  if (rawProgress < 0 || rawProgress > 1.12) return;
  const progress = Math.max(0, Math.min(1, rawProgress));
  const centerX = particleTargetBounds.left + particleTargetBounds.width / 2;
  const centerY = particleTargetBounds.top + particleTargetBounds.height / 2;
  const intensity = Math.sin(progress * Math.PI);
  const outerRadius = Math.max(particleCanvas.width, particleCanvas.height) * .56;
  const targetRadius = Math.max(particleTargetBounds.width, particleTargetBounds.height) * .48;
  const radius = outerRadius + (targetRadius - outerRadius) * easeFormation(progress);

  particleContext.save();
  particleContext.globalCompositeOperation = "lighter";
  particleContext.translate(centerX, centerY);
  particleContext.scale(1, .72);
  for (let index = 0; index < 4; index += 1) {
    const ringRadius = radius + index * 34;
    particleContext.beginPath();
    particleContext.setLineDash([16 + index * 5, 34 + index * 7]);
    particleContext.lineDashOffset = (index % 2 ? 1 : -1) * now / (12 + index * 3);
    particleContext.arc(0, 0, ringRadius, 0, Math.PI * 2);
    particleContext.lineWidth = index === 0 ? 1.8 : .75;
    particleContext.strokeStyle = index === 2
      ? `rgba(255,155,47,${intensity * .48})`
      : `rgba(34,199,255,${intensity * (.56 - index * .08)})`;
    particleContext.stroke();
  }
  particleContext.restore();

  const shock = Math.max(0, Math.min(1, (progress - .68) / .25));
  if (shock > 0 && shock < 1) {
    particleContext.save();
    particleContext.globalCompositeOperation = "lighter";
    particleContext.beginPath();
    particleContext.ellipse(
      centerX,
      centerY,
      targetRadius * (.65 + shock * 1.35),
      targetRadius * (.48 + shock * .95),
      0,
      0,
      Math.PI * 2,
    );
    particleContext.strokeStyle = `rgba(134,229,255,${(1 - shock) * .85})`;
    particleContext.shadowColor = "#22c7ff";
    particleContext.shadowBlur = 28;
    particleContext.lineWidth = 2.4;
    particleContext.stroke();
    particleContext.restore();
  }
}

function drawParticleField(now: number) {
  particleContext.clearRect(0, 0, particleCanvas.width, particleCanvas.height);
  if (visualParticles.length) {
    const rawProgress = (now - formationStartedAt) / FORMATION_DURATION;
    const forming = rawProgress >= 0 && rawProgress < 1.08;
    const idleStrength = state.mode === "speaking" ? .18 + state.level * .48 : state.mode === "working" ? .12 : .045;
    particleContext.globalCompositeOperation = "lighter";
    for (const particle of visualParticles) {
      const localRaw = forming ? (rawProgress - particle.delay) / (1 - particle.delay) : 1;
      const progress = Math.max(0, Math.min(1, localRaw));
      const point = particlePosition(particle, progress);
      const previous = particlePosition(particle, Math.max(0, progress - (.035 + particle.size * .008)));
      const drift = forming ? 0 : Math.sin(now / 760 + particle.phase) * (1.1 + state.level * 3.2);
      const x = point.x + drift;
      const y = point.y + Math.cos(now / 830 + particle.phase) * (forming ? 0 : 1.4);
      const alpha = forming
        ? localRaw < 0
          ? .12 + Math.sin(now / 180 + particle.phase) * .08
          : .34 + Math.sin(progress * Math.PI) * .66
        : idleStrength;
      const color = state.mode === "working"
        ? "255,155,47"
        : state.mode === "stopped"
          ? "255,73,62"
          : particle.amber
            ? "255,155,47"
            : "34,199,255";
      if (forming) {
        particleContext.beginPath();
        particleContext.moveTo(previous.x, previous.y);
        particleContext.lineTo(x, y);
        particleContext.strokeStyle = `rgba(${color},${alpha * (particle.size > 2.5 ? .62 : .28)})`;
        particleContext.lineWidth = particle.size > 2.5 ? 1.5 : .65;
        particleContext.stroke();
      }
      particleContext.beginPath();
      particleContext.arc(x, y, particle.size * (forming ? 1.3 : 1), 0, Math.PI * 2);
      particleContext.fillStyle = `rgba(${color},${alpha})`;
      particleContext.fill();
    }
    if (forming) drawFormationEnergy(now, rawProgress);
    particleContext.globalCompositeOperation = "source-over";
  }
  requestAnimationFrame(drawParticleField);
}

if (characterImage.complete && characterImage.naturalWidth) prepareVisualParticles();
else characterImage.addEventListener("load", prepareVisualParticles, { once: true });
new ResizeObserver(() => prepareVisualParticles()).observe(shell);
requestAnimationFrame(drawParticleField);

function analyserLevel(analyser: AnalyserNode | null) {
  if (!analyser) return 0;
  const samples = new Uint8Array(analyser.fftSize);
  analyser.getByteTimeDomainData(samples);
  let energy = 0;
  for (const sample of samples) {
    const normalized = (sample - 128) / 128;
    energy += normalized * normalized;
  }
  return Math.min(1, Math.sqrt(energy / samples.length) * 5);
}

function updateAudioMeters() {
  if (!currentWindow) {
    const time = performance.now();
    state.level = state.mode === "speaking"
      ? .52 + Math.sin(time / 125) * .16
      : state.mode === "working"
        ? .2 + Math.sin(time / 310) * .08
        : state.mode === "listening"
          ? .08 + Math.sin(time / 430) * .035
          : 0;
    shell.style.setProperty("--voice-pulse", String(1 + state.level * .026));
    shell.style.setProperty("--voice-glow", String(.45 + state.level * .55));
    requestAnimationFrame(updateAudioMeters);
    return;
  }
  const micLevel = analyserLevel(microphoneAnalyser);
  const speakerLevel = analyserLevel(remoteAnalyser);
  state.level = Math.max(state.level, micLevel, speakerLevel);
  shell.style.setProperty("--voice-pulse", String(1 + state.level * .026));
  shell.style.setProperty("--voice-glow", String(.45 + state.level * .55));
  if (state.directVoice?.voiceActive && !state.agentWorking) {
    if (speakerLevel > 0.08 && state.mode !== "speaking") setMode("speaking");
    if (speakerLevel < 0.025 && state.mode === "speaking") setMode("listening");
  }
  requestAnimationFrame(updateAudioMeters);
}
updateAudioMeters();

function updateVoiceInfo(info: DirectVoice) {
  state.directVoice = info;
  mic.classList.toggle("active", info.voiceActive);
  mic.querySelector("span")!.innerHTML = info.voiceActive
    ? '<svg viewBox="0 0 24 24"><rect x="8.25" y="3" width="7.5" height="11.5" rx="3.75"></rect><path d="M5.5 11.25v.75a6.5 6.5 0 0 0 13 0v-.75M12 18.5V22M8.75 22h6.5"></path></svg>'
    : '<svg viewBox="0 0 24 24"><rect x="8.25" y="3" width="7.5" height="11.5" rx="3.75"></rect><path d="M5.5 11.25v.75a6.5 6.5 0 0 0 13 0v-.75M12 18.5V22M8.75 22h6.5"></path><path class="slash-mark" d="M4 4 20 20"></path></svg>';
  mic.setAttribute("aria-label", info.voiceActive ? "关闭语音" : "开启语音");
  mic.querySelector("b")!.textContent = info.voiceActive ? "CLOSE VOICE" : "CODEX VOICE";
  mic.querySelector("small")!.textContent = info.voiceActive ? "CLICK TO STOP" : "V3 WEBRTC · DIRECT";
  $("#voice-auth").textContent = info.codexConnected
    ? `${info.protocol} · ${info.voiceActive ? "connected" : info.phase}`
    : `${info.protocol} · standby`;
  if (info.threadId) {
    state.session = { threadId: info.threadId, cwd: workspace };
    $("#thread-id").textContent = info.threadId;
    localStorage.setItem(`${THREAD_KEY_PREFIX}${workspace}`, info.threadId);
  }
}

async function handle(message: Message) {
  if (message.id !== undefined && message.method) {
    if (permissionMode === "full") {
      appendStreamLine(`已自动授权：${message.method}`, "system");
      await invoke("resolve_server_request", { requestId: message.id, approved: true }).catch((error) => {
        appendStreamLine(`自动授权失败：${String(error)}`, "error");
      });
      return;
    }
    triggerCharacterAction("approval", 1800);
    approvalId = message.id; $("#approval-copy").textContent = `Codex 请求：${message.method}`; approval.showModal(); return;
  }
  const method = message.method, params = message.params;
  if (method === "thread/realtime/sdp") {
    if (!peer || !params?.sdp) return;
    try {
      await peer.setRemoteDescription({ type: "answer", sdp: params.sdp });
    } catch (error) {
      triggerCharacterAction("error");
      setMode("degraded");
      response.textContent = `Codex Voice SDP 连接失败：${String(error)}`;
    }
  } else if (method === "thread/realtime/started") {
    updateVoiceInfo({
      codexConnected: true,
      voiceActive: true,
      phase: "connected",
      protocol: "Codex app-server V3 · WebRTC",
      threadId: params?.threadId,
      realtimeSessionId: params?.realtimeSessionId,
    });
    banner.hidden = true;
    setMode("listening");
    triggerCharacterAction("acknowledge");
    setWorker("orchestrator", "Official Voice online");
    streamState.textContent = "LIVE";
    appendStreamLine("Codex Voice 已连接", "system");
    response.textContent = "Codex 官方 Voice 已上线。你现在可以直接和 Jarvis 对话。";
  } else if (method === "thread/realtime/transcript/delta") {
    const delta = typeof params?.delta === "string" ? params.delta : "";
    if (params?.role === "assistant") {
      assistantTranscriptBuffer += delta;
      response.textContent = assistantTranscriptBuffer;
      appendStreamLine(assistantTranscriptBuffer, "assistant", "voice-assistant");
      if (!state.agentWorking) setMode("speaking");
    } else {
      userTranscriptBuffer += delta;
      transcript.textContent = userTranscriptBuffer;
      appendStreamLine(userTranscriptBuffer, "user", "voice-user");
      if (!state.agentWorking) setMode("listening");
    }
  } else if (method === "thread/realtime/transcript/done") {
    const text = typeof params?.text === "string" ? params.text.trim() : "";
    if (params?.role === "assistant") {
      if (text) response.textContent = text;
      if (text) appendStreamLine(text, "assistant", "voice-assistant");
      sealStreamLine("voice-assistant");
      assistantTranscriptBuffer = "";
      if (!state.agentWorking) setMode("listening");
    } else {
      if (text) transcript.textContent = text;
      if (text) appendStreamLine(text, "user", "voice-user");
      sealStreamLine("voice-user");
      if (text) lastUserTurnText = text;
      userTranscriptBuffer = "";
      if (text) triggerCharacterAction("acknowledge");
    }
  } else if (method === "thread/realtime/itemAdded") {
    const itemType = String(params?.item?.type ?? "");
    if (itemType.includes("handoff") || itemType.includes("delegation")) {
      setMode("working");
      setWorker("orchestrator", "Delegating to Codex");
    }
    appendStreamLine(`开始：${describeEventItem(params)}`, "tool");
  } else if (method === "thread/realtime/error") {
    triggerCharacterAction("error");
    setMode("degraded");
    banner.hidden = false;
    const detail = params?.message ?? "Codex Voice realtime error";
    $("#degraded-copy").textContent = detail;
    response.textContent = detail;
    streamState.textContent = "ERROR";
    appendStreamLine(String(detail), "error");
  } else if (method === "thread/realtime/closed") {
    cleanupPeer();
    updateVoiceInfo({
      codexConnected: true,
      voiceActive: false,
      phase: "closed",
      protocol: "Codex app-server V3 · WebRTC",
      threadId: params?.threadId ?? state.session?.threadId,
    });
    if (!state.manualStop) {
      setMode("ready");
      streamState.textContent = "STANDBY";
      appendStreamLine("Codex Voice 已结束，等待下一次唤醒", "system");
      response.textContent = "Codex Voice 已结束。再次说“嗨 Jarvis”即可唤醒。";
      await armWakeListener();
    }
  } else if (method === "turn/started") {
    if (state.manualStop) return;
    agentMessageBuffer = "";
    lastCompletedAgentText = "";
    state.agentWorking = true;
    appendStreamLine("开始处理任务", "task");
    setMode("working"); setWorker("orchestrator", "Codex working");
  } else if (method === "item/agentMessage/delta") {
    const delta = typeof params?.delta === "string" ? params.delta : "";
    agentMessageBuffer += delta;
    if (agentMessageBuffer) {
      response.textContent = agentMessageBuffer;
      appendStreamLine(agentMessageBuffer, "assistant", "agent-message");
    }
  } else if (method === "turn/completed") {
    state.agentWorking = false;
    if (!state.manualStop) triggerCharacterAction("complete", 1400);
    setMode(state.manualStop ? "stopped" : state.directVoice?.voiceActive ? "listening" : "ready");
    setWorker("orchestrator", state.manualStop ? "Interrupted" : "Ready", !state.manualStop);
    for (const role of ["developer", "researcher", "reviewer"]) {
      setWorker(role, state.manualStop ? "Interrupted" : "Standby", false);
    }
    appendStreamLine(state.manualStop ? "任务已暂停" : "任务完成", state.manualStop ? "system" : "task");
    const completedText = (agentMessageBuffer.trim() || lastCompletedAgentText.trim()).trim();
    lastCompletedAgentText = "";
    if (!state.manualStop && completedText && lastUserTurnText) {
      const userText = lastUserTurnText;
      lastUserTurnText = "";
      void saveMemoryEpisode(
        "Jarvis Codex turn",
        `User: ${userText}\nJarvis: ${completedText}`,
        ["jarvis", "codex"],
      );
    }
    if (!state.manualStop && completedText && !state.directVoice?.voiceActive) {
      setMode("speaking");
      void invoke("speak_text", { text: completedText }).catch((error) => {
        response.textContent = `本机语音失败：${String(error)}`;
      }).finally(() => {
        if (!state.directVoice?.voiceActive && state.mode === "speaking") setMode("ready");
      });
    }
  } else if (method === "item/started") {
    if (!state.manualStop) setWorker(roleOf(params), "Working");
    appendStreamLine(`开始：${describeEventItem(params)}`, "tool");
  }
  else if (method === "item/completed") {
    setWorker(roleOf(params), "Complete", false);
    appendStreamLine(`完成：${describeEventItem(params)}`, "tool");
    if (params?.item?.type === "agentMessage") {
      const text = extractAgentText(params.item).trim();
      if (text) lastCompletedAgentText = text;
      if (text) {
        response.textContent = text;
        appendStreamLine(text, "assistant", "agent-message");
        sealStreamLine("agent-message");
      }
    }
  }
}

async function waitForIceGathering(connection: RTCPeerConnection) {
  if (connection.iceGatheringState === "complete") return;
  await new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(() => {
      connection.removeEventListener("icegatheringstatechange", changed);
      reject(new Error("WebRTC ICE gathering timed out"));
    }, 12_000);
    const changed = () => {
      if (connection.iceGatheringState !== "complete") return;
      window.clearTimeout(timer);
      connection.removeEventListener("icegatheringstatechange", changed);
      resolve();
    };
    connection.addEventListener("icegatheringstatechange", changed);
  });
}

function attachAnalyser(stream: MediaStream, target: "microphone" | "remote") {
  audioContext ??= new AudioContext();
  const analyser = audioContext.createAnalyser();
  analyser.fftSize = 512;
  audioContext.createMediaStreamSource(stream).connect(analyser);
  if (target === "microphone") microphoneAnalyser = analyser;
  else remoteAnalyser = analyser;
}

function cleanupPeer() {
  peer?.close();
  peer = null;
  microphoneStream?.getTracks().forEach((track) => track.stop());
  microphoneStream = null;
  remoteStream?.getTracks().forEach((track) => track.stop());
  remoteStream = null;
  voiceAudio.pause();
  voiceAudio.srcObject = null;
  microphoneAnalyser = null;
  remoteAnalyser = null;
}

const sleep = (milliseconds: number) =>
  new Promise<void>((resolve) => window.setTimeout(resolve, milliseconds));

function isNotAllowedError(error: unknown) {
  return error instanceof DOMException
    ? error.name === "NotAllowedError"
    : String(error).includes("NotAllowedError");
}

async function acquireMicrophone(coldStart: boolean) {
  const attempts = coldStart ? 6 : 1;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
    } catch (error) {
      if (!coldStart || !isNotAllowedError(error) || attempt === attempts) throw error;
      response.textContent = `正在等待系统释放麦克风… ${attempt}/${attempts - 1}`;
      await sleep(700);
    }
  }
  throw new Error("麦克风初始化失败");
}

async function startDirectVoice({ coldStart = false } = {}) {
  if (SPEAKER_GATE_ENABLED && state.speakerAccess === "rejected") {
    const message = "未识别的说话人";
    response.textContent = message;
    setMode("ready");
    void invoke("speak_text", { text: message }).catch(() => undefined);
    return;
  }
  if (!currentWindow) {
    setMode("voice-starting");
    window.setTimeout(() => setMode("listening"), FORMATION_DURATION);
    return;
  }
  if (voiceStartInFlight || peer || state.directVoice?.voiceActive) return;
  voiceStartInFlight = true;
  state.manualStop = false;
  recoverableColdStartError = false;
  setMode("voice-starting");
  banner.hidden = true;
  response.textContent = "正在建立 Codex 官方 Voice V3 WebRTC 会话…";
  try {
    const microphoneAuthorization = await invoke<string>("request_microphone_permission");
    if (microphoneAuthorization !== "authorized") {
      throw new Error("请在系统设置 → 隐私与安全性 → 麦克风中允许 Jarvis Codex。");
    }
    await invoke("disarm_wake_listener");
    if (coldStart) {
      // A newly created WKWebView can reject an otherwise-authorized
      // getUserMedia call until its first visible/focused render cycle.
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      await sleep(900);
    }
    microphoneStream = await acquireMicrophone(coldStart);
    attachAnalyser(microphoneStream, "microphone");

    const connection = new RTCPeerConnection();
    peer = connection;
    const track = microphoneStream.getAudioTracks()[0];
    if (!track) throw new Error("未找到麦克风音轨");
    connection.addTrack(track, microphoneStream);
    connection.createDataChannel("oai-events");
    connection.ontrack = (event) => {
      remoteStream = event.streams[0] ?? new MediaStream([event.track]);
      voiceAudio.srcObject = remoteStream;
      voiceAudio.muted = false;
      attachAnalyser(remoteStream, "remote");
      void audioContext?.resume();
      void voiceAudio.play().catch((error) => {
        response.textContent = `Codex Voice 音频播放失败：${String(error)}`;
      });
    };
    connection.onconnectionstatechange = () => {
      if (connection.connectionState === "failed") {
        setMode("degraded");
        response.textContent = "Codex Voice WebRTC 连接失败。";
      }
    };
    const offer = await connection.createOffer();
    await connection.setLocalDescription(offer);
    await waitForIceGathering(connection);
    const sdp = connection.localDescription?.sdp;
    if (!sdp) throw new Error("WebRTC 未生成 SDP offer");

    const info = await invoke<DirectVoice>("start_codex_voice", {
      cwd: workspace,
      threadId: savedThreadId(),
      permissionMode,
      speakerAccess: state.speakerAccess,
      sdp,
      voice: "cove",
    });
    updateVoiceInfo(info);
  } catch (error) {
    cleanupPeer();
    state.directVoice = null;
    recoverableColdStartError = coldStart && isNotAllowedError(error);
    setMode("degraded");
    banner.hidden = false;
    $("#degraded-copy").textContent = String(error);
    response.textContent = String(error);
    await armWakeListener();
  } finally {
    voiceStartInFlight = false;
  }
}

async function waitForVoiceActive(timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (!state.directVoice?.voiceActive) {
    if (state.mode === "degraded" || Date.now() >= deadline) {
      throw new Error("Codex Voice 尚未连接");
    }
    await sleep(100);
  }
}

async function stopDirectVoice() {
  try {
    const info = await invoke<DirectVoice>("stop_codex_voice");
    updateVoiceInfo(info);
  } finally {
    cleanupPeer();
  }
}

if (currentWindow) {
  await listen<JarvisEvent>("jarvis-event", ({ payload }) => {
    if (payload.kind === "workflow" || payload.kind === "task") {
      const id = payload.runId ?? payload.taskId ?? payload.workflowId ?? payload.kind;
      const phase = payload.phase ?? "event";
      const detail = payload.message ?? payload.stepId ?? "";
      const label = payload.kind === "workflow" ? "工作流" : "任务";
      const kind = phase === "error" || phase === "approval-required" ? "error" : "task";
      appendStreamLine(`${label}${phase}：${detail}`, kind, `${payload.kind}-${id}`);
      return;
    }
    if (payload.kind !== "tool") return;
    const tool = payload.toolName ?? "tool";
    if (payload.phase === "started") {
      appendStreamLine(`工具开始：${tool}`, "tool", `tool-${payload.callId ?? tool}`);
      setMode("working");
      setWorker("developer", `${tool} working`);
      return;
    }
    const key = `tool-${payload.callId ?? tool}`;
    if (payload.phase === "error") {
      appendStreamLine(`工具失败：${tool}：${payload.error ?? "unknown error"}`, "error", key);
      setWorker("developer", "Tool error", false);
      return;
    }
    appendStreamLine(`工具完成：${tool}${payload.output ? ` · ${payload.output}` : ""}`, "tool", key);
    setWorker("developer", "Tool ready", false);
  });
  await listen<QwenEvent>("qwen-event", ({ payload }) => {
    if (payload.error) {
      appendStreamLine(`本地 Qwen：${payload.error}`, "error", "qwen-assistant");
      response.textContent = payload.error;
      setMode("degraded");
      return;
    }
    // reasoning chunks are intentionally ignored; only final `delta` content
    // reaches the Jarvis stream and the voice output path.
    if (payload.delta) {
      qwenAnswerBuffer += payload.delta;
      response.textContent = qwenAnswerBuffer;
      appendStreamLine(qwenAnswerBuffer, "assistant", "qwen-assistant");
      setMode("speaking");
      setWorker("orchestrator", "Local Qwen responding");
    }
    if (payload.done) {
      if (qwenAnswerBuffer.trim()) {
        appendStreamLine(qwenAnswerBuffer, "assistant", "qwen-assistant");
        sealStreamLine("qwen-assistant");
      }
      setWorker("orchestrator", "Local Qwen ready");
    }
  });
  await listen<Message>("codex-event", ({ payload }) => void handle(payload));
  await listen<WakeStatus>("jarvis-wake-status", ({ payload }) => {
    state.wake = payload;
    $("#wake-auth").textContent = payload.ready
      ? "Local listener ready"
      : payload.authorization === "authorized"
        ? "Waiting to re-arm"
        : payload.authorization;
    if (payload.ready) {
      if (recoverableColdStartError && state.mode === "degraded") {
        recoverableColdStartError = false;
        banner.hidden = true;
        setMode("ready");
      }
      if (state.mode === "ready") {
        response.textContent = "我在。直接说“嗨 Jarvis”。";
        setWorker("orchestrator", "Wake word armed");
      }
    }
  });
  await listen<WakeEvent>("jarvis-wake", ({ payload }) => {
    transcript.textContent = "“嗨，Jarvis”";
    appendStreamLine("收到唤醒词：嗨 Jarvis", "user");
    state.manualStop = false;
    state.speakerAccess = SPEAKER_GATE_ENABLED
      ? payload.speakerAccess ?? payload.speaker ?? "unknown"
      : "allen";
    updateSpeakerAccess(state.speakerAccess);
    if (SPEAKER_GATE_ENABLED && state.speakerAccess === "rejected") {
      const message = "未识别的说话人";
      setMode("ready");
      banner.hidden = true;
      response.textContent = message;
      void invoke("speak_text", { text: message }).catch(() => undefined);
      void armWakeListener();
      return;
    }
    if (!payload.ok) {
      setMode("degraded");
      banner.hidden = false;
      $("#degraded-copy").textContent = payload.error ?? "无法打开官方 Codex Voice。";
      response.textContent = payload.error ?? "无法打开官方 Codex Voice。";
      return;
    }
    banner.hidden = true;
    void startDirectVoice({ coldStart: payload.cold === true });
  });
}
$("#command-form").addEventListener("submit", async (event) => {
  event.preventDefault(); const input = $("#command-input") as HTMLInputElement, text = input.value.trim();
  if (!text) return;
  state.manualStop = false;
  lastUserTurnText = text;
  transcript.textContent = text;
  appendStreamLine(`文字指令：${text}`, "user");
  input.value = "";
  if (!currentWindow) {
    response.textContent = "视觉预览：文字任务已切换为 Codex 工作态。";
    setMode("working");
    return;
  }
  const useLocalQwen = modelMode === "qwen"
    || (modelMode === "hybrid" && !state.directVoice?.voiceActive);
  if (state.directVoice?.voiceActive && !useLocalQwen) {
    response.textContent = "已将文字作为用户话语注入当前 Codex Voice 会话。";
    await invoke("append_codex_voice_text", { text });
    return;
  }
  if (SPEAKER_GATE_ENABLED && state.speakerAccess === "rejected") {
    const message = "未识别的说话人";
    response.textContent = message;
    setMode("ready");
    void invoke("speak_text", { text: message }).catch(() => undefined);
    return;
  }
  if (useLocalQwen) {
    qwenAnswerBuffer = "";
    setMode("working");
    appendStreamLine("本地 Qwen 开始处理", "task");
    try {
      const answer = await invoke<string>("local_qwen_chat", { text });
      const finalAnswer = answer.trim() || qwenAnswerBuffer.trim();
      if (!finalAnswer) throw new Error("本地 Qwen 没有返回最终答案");
      response.textContent = finalAnswer;
      appendStreamLine(finalAnswer, "assistant", "qwen-assistant");
      sealStreamLine("qwen-assistant");
      qwenAnswerBuffer = "";
      lastUserTurnText = "";
      setMode("speaking");
      await invoke("speak_text", { text: finalAnswer }).catch((error) => {
        appendStreamLine(`本地语音失败：${String(error)}`, "error");
      });
      if (state.mode === "speaking") setMode("ready");
      return;
    } catch (error) {
      qwenAnswerBuffer = "";
      if (modelMode === "qwen") {
        const message = `本地 Qwen 不可用：${String(error)}`;
        response.textContent = message;
        appendStreamLine(message, "error");
        setMode("degraded");
        return;
      }
      response.textContent = "本地 Qwen 尚未连接，改用 Codex 原生任务。";
      appendStreamLine("本地 Qwen 不可用，切换 Codex", "system");
    }
  }
  // Keep typed turns on the same Codex Voice path as spoken turns. If the
  // realtime session cannot be established, retain a local text-task fallback
  // and speak its reply with the bundled offline voice.
  try {
    await startDirectVoice();
    await waitForVoiceActive();
    response.textContent = "已接入 Codex 原始语音，正在处理文字指令。";
    await invoke("append_codex_voice_text", { text });
    return;
  } catch {
    response.textContent = "Codex Voice 尚未连接，改用本地模型语音播报。";
  }
  // Re-check the runtime on every text turn. This is cheap when the speaker
  // state is unchanged, and rebuilds the thread instructions if a verifier
  // changed Unknown/Allen access since the previous turn.
  state.session = await invoke<Session>("start_jarvis", {
    cwd: workspace,
    threadId: savedThreadId(),
    permissionMode,
    speakerAccess: state.speakerAccess,
  });
  localStorage.setItem(`${THREAD_KEY_PREFIX}${workspace}`, state.session.threadId);
  $("#thread-id").textContent = state.session.threadId;
  $("#workspace").textContent = state.session.cwd;
  setMode("working");
  try {
    await invoke("send_text", { text });
  } catch (error) {
    const message = String(error);
    response.textContent = message;
    setMode("ready");
  }
});
mic.addEventListener("click", () => {
  if (state.directVoice?.voiceActive || peer) {
    void stopDirectVoice().catch((error) => {
      response.textContent = `关闭语音失败：${String(error)}`;
    });
  } else {
    void startDirectVoice();
  }
});
$("#stop").addEventListener("click", async () => {
  if (state.mode === "stopped" || state.manualStop) {
    state.manualStop = false;
    state.agentWorking = false;
    setMode("ready");
    appendStreamLine("正在恢复 Jarvis", "system");
    response.textContent = "正在恢复 Jarvis Voice…";
    try {
      await startDirectVoice({ coldStart: true });
      if (!state.directVoice?.voiceActive && !peer && state.mode !== "degraded") {
        await armWakeListener();
        response.textContent = "Jarvis 已恢复，等待唤醒词。";
        appendStreamLine("Jarvis 已恢复", "system");
      }
    } catch (error) {
      setMode("degraded");
      response.textContent = `恢复失败：${String(error)}`;
    }
    return;
  }
  triggerCharacterAction("error", 700);
  appendStreamLine("暂停当前任务", "system");
  state.manualStop = true; setMode("stopped");
  state.agentWorking = false;
  for (const role of ["orchestrator", "developer", "researcher", "reviewer"]) setWorker(role, "Interrupted", false);
  if (!currentWindow) return;
  if (state.directVoice?.voiceActive || peer) {
    try { await stopDirectVoice(); } catch { /* task interruption still continues */ }
  }
  await invoke("stop_all");
  await armWakeListener();
});
function syncPermissionControls() {
  const input = document.querySelector<HTMLInputElement>(
    `input[name="permission-mode"][value="${permissionMode}"]`,
  );
  if (input) input.checked = true;
  $("#permission-mode-label").textContent = permissionLabels[permissionMode];
  const modelInput = document.querySelector<HTMLInputElement>(
    `input[name="model-mode"][value="${modelMode}"]`,
  );
  if (modelInput) modelInput.checked = true;
  $("#model-mode-label").textContent = modelModeLabels[modelMode];
}

$("#settings").addEventListener("click", () => {
  syncPermissionControls();
  settings.showModal();
});
$("#close-settings").addEventListener("click", () => settings.close());
$("#new-thread").addEventListener("click", async () => {
  const button = $("#new-thread") as HTMLButtonElement;
  button.disabled = true;
  state.manualStop = true;
  settings.close();
  response.textContent = "正在结束当前任务并创建新的 Codex thread…";
  try {
    if (state.directVoice?.voiceActive || peer) {
      try { await stopDirectVoice(); } catch { cleanupPeer(); }
    }
    try { await invoke("stop_all"); } catch { /* no active runtime */ }
    await invoke("shutdown");
    const freshSession = await invoke<Session>("start_jarvis", {
      cwd: workspace,
      threadId: null,
      permissionMode,
      speakerAccess: state.speakerAccess,
    });
    state.session = freshSession;
    state.directVoice = null;
    localStorage.setItem(`${THREAD_KEY_PREFIX}${workspace}`, freshSession.threadId);
    $("#thread-id").textContent = freshSession.threadId;
    $("#workspace").textContent = freshSession.cwd;
    userTranscriptBuffer = "";
    assistantTranscriptBuffer = "";
    agentMessageBuffer = "";
    transcript.textContent = "“新开线程”";
    response.textContent = "新的 Codex thread 已创建。下一次唤醒和文字任务都会进入这个线程。";
    setMode("ready");
    setWorker("orchestrator", "Fresh thread ready");
  } catch (error) {
    try { await invoke("shutdown"); } catch { /* already stopped */ }
    state.session = null;
    state.directVoice = null;
    setMode("degraded");
    response.textContent = `新建线程失败，原线程仍可续接：${String(error)}`;
  } finally {
    button.disabled = false;
    await armWakeListener();
  }
});
$("#save-settings").addEventListener("click", async () => {
  const nextWorkspace = ($("#workspace-setting") as HTMLInputElement).value.trim();
  if (!nextWorkspace) return;
  const selectedPermission = document.querySelector<HTMLInputElement>(
    'input[name="permission-mode"]:checked',
  )?.value as PermissionMode | undefined;
  const nextPermission = selectedPermission ?? permissionMode;
  const selectedModel = document.querySelector<HTMLInputElement>(
    'input[name="model-mode"]:checked',
  )?.value as ModelMode | undefined;
  const nextModelMode = selectedModel ?? modelMode;
  if (nextWorkspace !== workspace) {
    localStorage.setItem(WORKSPACE_KEY, nextWorkspace);
    response.textContent = "工作目录已保存，重启 Jarvis 后生效。";
  }
  if (nextPermission !== permissionMode) {
    state.manualStop = true;
    if (state.directVoice?.voiceActive || peer) {
      try { await stopDirectVoice(); } catch { cleanupPeer(); }
    }
    try { await invoke("stop_all"); } catch { /* no active runtime */ }
    await invoke("shutdown");
    permissionMode = nextPermission;
    localStorage.setItem(PERMISSION_KEY, permissionMode);
    state.session = null;
    state.directVoice = null;
    syncPermissionControls();
    setMode("ready");
    response.textContent = `权限已切换为“${permissionLabels[permissionMode]}”，下一次任务将续接当前 Codex thread。`;
    await armWakeListener();
  }
  if (nextModelMode !== modelMode) {
    modelMode = nextModelMode;
    localStorage.setItem(MODEL_MODE_KEY, modelMode);
    syncPermissionControls();
    appendStreamLine(`模型路由已切换：${modelModeLabels[modelMode]}`, "system");
  }
  settings.close();
});
for (const [selector, approved] of [["#approve", true], ["#deny", false]] as const) {
  $(selector).addEventListener("click", async () => { await invoke("resolve_server_request", { requestId: approvalId, approved }); approval.close(); });
}

if (currentWindow) {
  try {
    workspace = localStorage.getItem(WORKSPACE_KEY)
      ?? await invoke<string>("default_workspace");
    localStorage.setItem(WORKSPACE_KEY, workspace);
    $("#thread-id").textContent = "Not started";
    $("#workspace").textContent = workspace;
    ($("#workspace-setting") as HTMLInputElement).value = workspace;
    syncPermissionControls();
    updateSpeakerAccess(state.speakerAccess);
    setWorker("orchestrator", "Wake word starting");
    setMode("ready");
    const backgroundStart = await invoke<boolean>("startup_is_background");
    if (!backgroundStart) {
      const microphoneAuthorization = await invoke<string>("request_microphone_permission");
      if (microphoneAuthorization !== "authorized") {
        setMode("degraded");
        banner.hidden = false;
        $("#degraded-copy").textContent =
          "请在系统设置 → 隐私与安全性 → 麦克风中允许 Jarvis Codex。";
      }
    }
    await armWakeListener();
    updateVoiceInfo(await invoke<DirectVoice>("direct_voice_status"));
    if (await invoke<boolean>("consume_cold_wake")) {
      transcript.textContent = "“嗨，Jarvis”";
    } else if (!backgroundStart && state.mode === "ready") {
      // A normal launch should be immediately usable. The wake listener is
      // still used for later re-entry after the session is stopped.
      void startDirectVoice();
    }
  } catch (error) { setMode("stopped"); response.textContent = `启动失败：${String(error)}`; }
} else {
  workspace = "Visual preview · native systems disconnected";
  $("#thread-id").textContent = "Preview only";
  $("#workspace").textContent = workspace;
  ($("#workspace-setting") as HTMLInputElement).value = workspace;
  $("#wake-auth").textContent = "Preview · not connected";
  $("#voice-auth").textContent = "Preview · not connected";
  transcript.textContent = visualPreviewMode === "stopped" ? "“停下”" : "“嗨，Jarvis”";
  response.textContent = visualPreviewMode === "voice-starting"
    ? "正在从粒子中重构 Jarvis 核心…"
    : visualPreviewMode === "working"
      ? "Codex 正在执行任务，装甲能量切换为工作态。"
      : visualPreviewMode === "speaking"
        ? "语音输出正在驱动角色光效与声波。"
        : visualPreviewMode === "stopped"
          ? "所有任务已中断，等待下一次唤醒。"
          : "Jarvis 视觉系统预览就绪。";
  setMode(visualPreviewMode);
  if (["acknowledge", "approval", "complete", "error"].includes(previewActionValue ?? "")) {
    window.setTimeout(() => triggerCharacterAction(previewActionValue as CharacterAction, 1800), 180);
  }
  setWorker("orchestrator", visualPreviewMode === "working" ? "Codex working" : "Visual preview");
  if (visualPreviewMode === "working") {
    setWorker("developer", "Working");
  }
}

async function armWakeListener() {
  try {
    state.wake = await invoke<WakeStatus>("arm_wake_listener");
    $("#wake-auth").textContent = state.wake.ready ? "Local listener ready" : state.wake.authorization;
    if (["denied", "restricted"].includes(state.wake.authorization)) {
      setMode("degraded");
      banner.hidden = false;
      $("#degraded-copy").textContent = "请在系统设置 → 隐私与安全性中允许麦克风和语音识别。";
    }
  } catch (error) {
    $("#wake-auth").textContent = String(error);
  }
}
