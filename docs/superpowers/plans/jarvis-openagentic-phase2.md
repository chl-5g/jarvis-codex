# Jarvis OpenAgentic Phase 2 Implementation Plan

**Goal:** Move the remaining OpenAgentic foundations into the local Jarvis boundary without introducing PostgreSQL, JWT, or a second always-on server.

**Architecture:** Keep Codex app-server as the trusted executor for Computer Use and native file changes. Add a Rust local tool gateway for explicit local-model tool calls, a file-backed knowledge index for `~/notes` and configured folders, and a durable local workflow/task store with bounded background execution. All paths emit the existing `codex-event`/`qwen-event` stream shape through one `jarvis-event` envelope.

**Constraints:** Do not restart Jarvis during implementation. Push only at an explicit user-approved checkpoint. Do not execute memory or knowledge text as instructions. Keep tool paths within the selected workspace unless the user explicitly enables the existing Full permission mode.

## Work slices

1. **Tool gateway and unified events**
   - [x] Add a Rust gateway with path policy, command allow/deny checks, output limits, timeout, and audit events.
   - [x] Expose `tool_list` and `tool_execute` Tauri commands.
   - [x] Add frontend rendering for `jarvis-event` tool start/result/error events.
   - [x] Add red/green tests for safe file edits, command blocking, truncation, and event payloads.

2. **Knowledge bridge**
   - [x] Index Markdown files from `~/notes` plus configured roots into a local JSON index.
   - [x] Support incremental scan, keyword search, bounded excerpts, and source paths.
   - [x] Inject search results into local Qwen and Codex text turns only when relevant.
   - [x] Keep indexing local and optional; missing roots return an empty result.

3. **Workflow and task scheduler**
   - [x] Add a JSON workflow definition with sequential/conditional steps and explicit approval boundaries.
   - [x] Persist tasks under the local Jarvis state directory and run due tasks in a bounded Tokio scheduler.
   - [x] Emit lifecycle events and support cancel/resume without touching Codex Voice.

4. **Cross-device boundary and verification**
   - [x] Add a localhost-only authenticated event endpoint suitable for a future iPhone/Shortcuts adapter.
   - [x] Keep it disabled by default and document the pairing boundary.
   - [x] Run Rust/TypeScript tests, production web build, and static review. Deployment/restart remain separate user-authorized actions.
