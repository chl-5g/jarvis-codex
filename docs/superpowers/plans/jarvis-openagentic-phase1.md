# Jarvis OpenAgentic Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Connect Jarvis to the local OpenAgentic-compatible foundation for memory, local Qwen text turns, Skills, and one unified UI event stream without replacing Codex Voice or Computer Use.

**Architecture:** Keep Codex app-server as the native executor and realtime voice path. Add a small Rust local bridge that reads and writes the existing `~/.openagentic/memory` Markdown layout, scans OpenAgentic Skills, and calls the loopback Qwen OpenAI-compatible endpoint through the system `curl` binary. The frontend selects a hybrid route: Codex Voice remains primary when active; local Qwen handles explicit local-model text turns and fallback text turns. Both paths render into the existing Jarvis stream.

**Tech Stack:** Tauri 2 / Rust, TypeScript/Vite, serde_json, local Markdown files, MLX-LM OpenAI-compatible HTTP/SSE.

**Spec:** User request: “全做”; first-stage boundary is memory + Qwen route + Skills + unified events, followed by a verified Jarvis restart. Do not push GitHub.

## Global Constraints

- Keep the current Codex Voice/WebRTC and Computer Use paths intact.
- Local Qwen calls must target `http://127.0.0.1:8080/v1/chat/completions` only.
- Preserve backend reasoning but never render or speak Qwen `reasoning` / `delta.reasoning`.
- Read and write the existing OpenAgentic Markdown layout under `~/.openagentic/memory` (or `OPENAGENTIC_MEMORY_DIR`); do not require PostgreSQL, JWT, or a running OpenAgentic API for phase 1.
- Do not restart Jarvis until all phase-1 tests and the production build pass.
- Do not push GitHub.

## Review Focus

- Memory files with path-like or instruction-like content must stay data-only and cannot escape the memory root.
- Missing memory or Skills directories must degrade to empty context.
- Qwen SSE reasoning chunks must never enter the UI stream or TTS.
- Qwen service failure must leave the existing Codex fallback path usable.
- Existing voice, permissions, pause/resume, and thread-resume tests must remain green.

### Task 1: Local OpenAgentic Markdown memory and Skills bridge

**Files:**
- Create: `src-tauri/src/memory.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: Rust unit tests in `src-tauri/src/memory.rs`, plus `tests/wav.test.mjs`

**Interfaces:**
- `MemoryStore::from_root(root: PathBuf)`
- `MemoryStore::initial_context(max_chars: usize) -> String`
- `MemoryStore::recall(query: &str, max_chars: usize) -> String`
- `MemoryStore::save_core(key, value, category) -> Result<String, String>`
- `MemoryStore::save_episode(title, summary, tags) -> Result<String, String>`
- `MemoryStore::skills_context(max_chars: usize) -> String`
- Tauri commands `memory_status`, `memory_recall`, `memory_save_core`, `memory_save_episode`

- [x] Write failing tests for core save/recall, episode persistence, missing-root fallback, and skills discovery.
- [x] Run the focused Rust tests and observe failure because the bridge does not exist.
- [x] Implement bounded keyword recall, safe filename sanitization, Markdown frontmatter, root selection, and Skills metadata scanning.
- [x] Inject initial memory/Skills context into Codex `baseInstructions`; add per-turn memory recall to text and Voice append paths.
- [x] Run Rust tests and existing frontend contract tests.
- [x] Commit locally.

### Task 2: Local Qwen route and reasoning-hidden SSE events

**Files:**
- Create: `src-tauri/src/qwen.rs`
- Modify: `src-tauri/src/lib.rs`, `src/main.ts`, `src/style.css`, `tests/wav.test.mjs`

**Interfaces:**
- Tauri command `local_qwen_chat(text: String) -> Result<String, String>`
- Event `qwen-event` with `{ delta?: string, done?: boolean, error?: string }`
- Request uses `chat_template_kwargs: { enable_thinking: true, reasoning_effort: "medium", preserve_thinking: false }`.

- [x] Write failing contract tests for the Qwen endpoint, reasoning filtering, and local-model mode controls.
- [x] Run focused tests and observe failure.
- [x] Implement local Qwen SSE parsing through loopback `curl`, emitting only `delta.content` and saving the completed episode.
- [x] Add hybrid/local-Qwen/Codex mode persistence and route typed turns without changing active Voice behavior.
- [x] Render Qwen deltas through the existing chronological event stream and speak only final content with local TTS.
- [x] Run focused tests and build.
- [x] Commit locally.

### Task 3: Phase-1 verification and deployment

**Files:**
- Modify: `README.md`, `docs/ARCHITECTURE.md`, `tests/wav.test.mjs` as needed.

- [x] Run the full Jarvis test suite, web build, Rust format/check/test, and production Tauri build.
- [x] Verify `127.0.0.1:8080/v1/models` and a Qwen completion locally.
- [x] Deploy the built app to the existing local Jarvis output without GitHub push.
- [x] Restart Jarvis once (explicitly authorized by the user) and verify the process and local mode UI.
- [x] Record the completed phase and any deferred phase-2 capabilities.
