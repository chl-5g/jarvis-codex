# Jarvis Voice Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace scattered startup and Voice readiness callbacks with explicit sequential workflows and truthful initialization status.

**Architecture:** A small renderer workflow module owns step order and completion gating. Existing Tauri commands remain the capability boundary; `bootstrap.ts` supplies step functions and renders workflow events. Startup and Voice sessions have distinct states so visual animation cannot imply readiness.

**Tech Stack:** TypeScript, Vite, Tauri 2, Rust app-server bridge, Node test runner.

**Spec:** `docs/superpowers/specs/2026-10-08-jarvis-workflow-design.md`

## Global Constraints

- Keep native Codex and system capability access behind existing Tauri commands.
- Keep user-visible copy in `config/*.json`.
- Preserve speaker verification and private-memory gating.
- Do not request microphone or speech permissions during passive startup.
- Preserve the existing direct Codex Voice WebRTC path and `cove` voice selection.

## Review Focus

- A startup failure must never emit Voice-ready or initialization-complete copy.
- A speaker-verification failure must continue with anonymous memory policy and must not load Allen memory.
- A realtime event arriving before the workflow reaches the Voice step must not complete startup.
- Repeated realtime-started events must announce completion only once.
- Stopping or a failed step must clean up the stream and re-arm wake listening.

### Task 1: Define the sequential workflow module

**Files:**
- Create: `src/workflows/voice-workflow.ts`
- Test: `tests/voice-workflow.test.mjs`

**Interfaces:**
- `WorkflowStep = "capture-speaker" | "initialize-codex" | "load-memory" | "connect-voice"`
- `WorkflowState = "waiting-wake" | "capturing-speaker" | "initializing-codex" | "loading-memory" | "connecting-voice" | "ready" | "degraded"`
- `runWorkflow(steps, hooks): Promise<WorkflowResult>` runs steps in order, calls `onStepStart`, `onStepSuccess`, `onFailure`, and never calls completion after a failure.

- [ ] Write failing tests for order, failure short-circuit, and exactly-once completion.
- [ ] Implement the module without Tauri imports.
- [ ] Run `node --test tests/voice-workflow.test.mjs`.

### Task 2: Route bootstrap voice startup through the workflow

**Files:**
- Modify: `src/app/bootstrap.ts`
- Modify: `config/ui.json`
- Test: `tests/wav.test.mjs`

**Interfaces:**
- Pass existing microphone, speaker, memory, WebRTC, and backend calls as workflow step functions.
- Render workflow states through the existing `setMode`, `appendStreamLine`, and response elements.
- Mark initialization complete only from the workflow completion callback.

- [ ] Add tests pinning startup waiting copy and verified/unverified completion gates.
- [ ] Replace direct sequential body in `startDirectVoice` with workflow step functions.
- [ ] Remove duplicate completion flags and direct completion announcements from unrelated event handlers.
- [ ] Run `npm test` and `npm run web:build`.

### Task 3: Add runtime step diagnostics

**Files:**
- Modify: `src/app/bootstrap.ts`
- Modify: `src-tauri/src/lib.rs` only if a backend diagnostic boundary is needed.
- Test: `tests/wav.test.mjs`

- [ ] Emit step start/success/failure messages with the workflow step name.
- [ ] Ensure errors return to degraded state and re-arm the wake listener.
- [ ] Run the frontend tests and Rust tests.

### Task 4: Build, install, restart, and observe

**Files:**
- Modify: generated build artifacts only.

- [ ] Run `npm run build`.
- [ ] Run `npm run install:release`.
- [ ] Confirm the installed binary timestamp.
- [ ] Watch `logs/jarvis-runtime.jsonl` through startup and one wake attempt.
- [ ] Run `git diff --check`, `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings`.
