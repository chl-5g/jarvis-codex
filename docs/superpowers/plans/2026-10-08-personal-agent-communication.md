# Personal Agent Communication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend the existing Jarvis CipherPipe adapter into a secure, structured personal-agent task channel with registration, capability ACLs, task lifecycle events, approvals, and result handling.

**Architecture:** Keep CipherPipe as the encrypted transport. Add focused Rust modules for protocol validation, Agent registry, and task lifecycle; route received envelopes through a capability-scoped executor and existing Jarvis event/logging infrastructure. Keep the first implementation single-task and explicit-peer, with no arbitrary shell execution.

**Tech Stack:** Rust/Tauri, serde/serde_json, tokio, existing CipherPipe Python bridge, TypeScript/Vite UI.

**Spec:** `docs/superpowers/specs/2026-10-08-personal-agent-communication-design.md`

## Global Constraints

- CipherPipe remains transport only; Jarvis owns authorization and execution.
- Protocol version is `1`; all messages have bounded size, timestamps, and expiry.
- Remote messages never invoke arbitrary shell commands.
- Explicit peer registration is used; no automatic discovery in phase one.
- All product code is written before the final concentrated test task, per user requirement.
- Existing uncommitted work in `config/ui.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src/app/bootstrap.ts`, and `pnpm-lock.yaml` must be preserved and integrated deliberately.

## Review Focus

- Unknown, malformed, oversized, expired, or replayed envelopes are rejected without execution.
- A known peer cannot invoke a capability outside its ACL or an unregistered tool.
- Approval-required tasks remain pending until explicit approval and cannot run early.
- Duplicate delivery does not execute a task twice.
- CipherPipe disconnects and malformed bridge output produce terminal task errors and UI events.

## File Map

- Create `src-tauri/src/agent_protocol.rs`: typed envelopes, task payloads, size/time validation, deduplication keys.
- Create `src-tauri/src/agent_registry.rs`: durable explicit peer registry and capability/trust records.
- Create `src-tauri/src/agent_tasks.rs`: task state machine, idempotency, expiry, ACL and approval state.
- Modify `src-tauri/src/cipherpipe.rs`: structured envelope send/receive over the existing bridge and bounded inbound handling.
- Modify `src-tauri/src/lib.rs`: state wiring, Tauri commands, inbound dispatch, approval/result events.
- Modify `src/app/bootstrap.ts` and relevant UI config: minimal registration/task/approval interactions.
- Create `docs/superpowers/plans/2026-10-08-personal-agent-communication.md` only if this plan is copied or superseded; this file is the canonical plan.
- Final test changes: Rust unit tests colocated with the new modules, TypeScript routing tests, and integration-style bridge/protocol fixtures.

## Implementation Tasks

### Task 1: Define the protocol types and validation boundary

**Files:**
- Create: `src-tauri/src/agent_protocol.rs`
- Modify: `src-tauri/src/lib.rs` module declarations

**Interfaces:**
- `AgentEnvelope { version, message_id, kind, task_id, from, to, created_at, expires_at, payload }`
- `AgentMessageKind`: hello, task request, task result, approval request, task cancel
- `TaskRequestPayload { capability, input, requires_approval }`
- `TaskResultPayload { status, output, error }`
- `validate_inbound(envelope, now, max_bytes) -> Result<(), String>`
- `new_task_request(...) -> AgentEnvelope`

**Steps:**
- [ ] Add serde-backed types with camelCase JSON names and explicit status/kind enums.
- [ ] Enforce version 1, required identities, bounded serialized size, valid timestamps, and expiry.
- [ ] Generate collision-resistant message/task IDs using existing standard-library/runtime facilities.
- [ ] Keep validation pure and independent of Tauri state so later modules can call it.

### Task 2: Add explicit Agent registry and capability records

**Files:**
- Create: `src-tauri/src/agent_registry.rs`
- Modify: `src-tauri/src/lib.rs` state and command registration
- Modify: project config path handling if a new durable file path is required

**Interfaces:**
- `AgentRecord { public_key, name, device, trusted, capabilities, last_seen }`
- `AgentRegistry::list() -> Vec<AgentRecord>`
- `AgentRegistry::upsert(record) -> Result<AgentRecord, String>`
- `AgentRegistry::remove(public_key) -> Result<(), String>`
- `AgentRegistry::authorize(peer, capability) -> bool`

**Steps:**
- [ ] Persist registry JSON atomically under the Jarvis data/config path.
- [ ] Validate public-key format, capability names, record bounds, and duplicate keys.
- [ ] Expose Tauri list/upsert/remove commands.
- [ ] Make registry reads tolerant of a missing or corrupt file by returning an empty registry plus an error event where appropriate.
- [ ] Define the initial local capability allowlist using existing tool names; do not expose shell.

### Task 3: Add task lifecycle and idempotent execution gate

**Files:**
- Create: `src-tauri/src/agent_tasks.rs`
- Modify: `src-tauri/src/lib.rs` AppState and event wiring

**Interfaces:**
- `RemoteTask { id, peer, capability, input, status, expires_at, requires_approval }`
- `RemoteTaskStatus`: queued, awaitingApproval, running, completed, failed, rejected, expired, cancelled
- `TaskStore::create(request) -> Result<RemoteTask, String>`
- `TaskStore::accept_once(message_id, task_id) -> Result<bool, String>`
- `TaskStore::transition(id, next) -> Result<RemoteTask, String>`
- `TaskStore::expire(now) -> Vec<RemoteTask>`

**Steps:**
- [ ] Implement bounded durable state with atomic writes and a maximum retained task count.
- [ ] Enforce legal state transitions and terminal-state immutability.
- [ ] Make message/task deduplication durable enough to prevent duplicate execution after reconnect.
- [ ] Add approval-pending state and explicit approve/reject operations.
- [ ] Emit existing Jarvis event envelopes for queued, running, approval, result, failure, expiry, and cancellation.

### Task 4: Extend CipherPipe adapter for structured envelopes

**Files:**
- Modify: `src-tauri/src/cipherpipe.rs`
- Modify: `src-tauri/cipherpipe_bridge.py`

**Interfaces:**
- `CipherPipe::send_envelope(app, envelope) -> Result<(), String>`
- inbound callback/event payload carrying the decoded `AgentEnvelope`
- bridge operation remains JSONL over stdin/stdout; transport payload is a single bounded JSON message

**Steps:**
- [ ] Add a structured send operation while retaining the existing plain-message command for compatibility.
- [ ] Mark structured messages with a stable transport prefix/type and decode only valid JSON envelopes.
- [ ] Reject oversized or malformed bridge input before dispatch.
- [ ] Preserve sender/recipient metadata from CipherPipe and surface disconnect/error events.
- [ ] Ensure no code path calls CipherPipe `cmd:<command>` execution.

### Task 5: Dispatch remote tasks through ACL, approval, and local tools

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/tools.rs` only where a capability-scoped invocation seam is needed
- Modify: `src-tauri/src/cipherpipe.rs` for result replies

**Interfaces:**
- `agent_register`, `agent_list`, `agent_remove`
- `agent_task_send(peer, capability, input, expires_at, requires_approval)`
- `agent_task_approve(task_id)`
- `agent_task_reject(task_id)`
- `agent_task_cancel(task_id)`
- `agent_task_list()`

**Steps:**
- [ ] On outbound task creation, verify target peer and capability through the registry before sending.
- [ ] On inbound request, validate envelope, verify trusted peer and capability ACL, then create exactly one task.
- [ ] Route approval-required tasks to a pending state and emit an approval event without executing.
- [ ] Invoke only the existing structured tool gateway for approved capabilities; pass JSON input without shell interpolation.
- [ ] Send a structured result envelope for completed, failed, rejected, expired, or cancelled tasks.
- [ ] Update sender-side task state when result envelopes arrive and emit UI-visible events.
- [ ] Add timeout handling around remote waits and bridge operations.

### Task 6: Add the minimum user-facing controls

**Files:**
- Modify: `src/app/bootstrap.ts`
- Modify: relevant `config/*.json` UI labels/configuration

**Steps:**
- [ ] Add explicit commands or controls for listing/registering peers and sending a structured task.
- [ ] Render queued/running/awaiting approval/completed/rejected/failed states in the existing stream.
- [ ] Add approve/reject actions for pending remote tasks.
- [ ] Surface peer, task ID, capability, and failure reason without exposing private payloads unnecessarily.
- [ ] Keep the existing “发给 CipherPipe” plain-message route working.

### Task 7: Write and run the concentrated verification suite

**Files:**
- Modify/create Rust unit test modules colocated with `agent_protocol.rs`, `agent_registry.rs`, and `agent_tasks.rs`
- Create: `tests/agent-protocol.test.mjs` if TypeScript routing behavior needs coverage
- Create: bridge/protocol fixture tests under `tests/`

**Steps:**
- [ ] Test valid envelope round trips and rejection of unknown versions, missing identities, oversized payloads, invalid timestamps, and expired tasks.
- [ ] Test registry persistence, malformed records, peer ACL allow/deny, and capability bounds.
- [ ] Test task transitions, approval gating, expiry, cancellation, durable deduplication, and duplicate delivery.
- [ ] Test structured CipherPipe bridge send/receive, malformed bridge lines, disconnects, and prohibition of shell execution.
- [ ] Test end-to-end request/result state transitions and emitted event fields.
- [ ] Run `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `npm run web:build`, and `git diff --check`.
- [ ] Fix failures, rerun the complete suite, and record the evidence in the final report.

