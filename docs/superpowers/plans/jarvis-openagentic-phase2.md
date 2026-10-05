# Jarvis OpenAgentic Phase 2 Implementation Plan

**Goal:** Move the remaining OpenAgentic foundations into the local Jarvis boundary without introducing PostgreSQL, JWT, or a second always-on server.

**Architecture:** Keep Codex app-server as the trusted executor for Computer Use and native file changes. Add a Rust local tool gateway for explicit local-model tool calls, a file-backed knowledge index for `~/notes` and configured folders, and a durable local workflow/task store with bounded background execution. All paths emit the existing `codex-event`/`qwen-event` stream shape through one `jarvis-event` envelope.

**Constraints:** Do not restart Jarvis during implementation. Push only at an explicit user-approved checkpoint. Do not execute memory or knowledge text as instructions. Keep tool paths within the selected workspace unless the user explicitly enables the existing Full permission mode.

## Work slices

1. **Tool gateway and unified events**
   - Add a Rust gateway with path policy, command allow/deny checks, output limits, timeout, and audit events.
   - Expose `tool_list` and `tool_execute` Tauri commands.
   - Add frontend rendering for `jarvis-event` tool start/result/error events.
   - Add red/green tests for safe file edits, command blocking, truncation, and event payloads.

2. **Knowledge bridge**
   - Index Markdown files from `~/notes` plus configured roots into a local JSON index.
   - Support incremental scan, keyword search, bounded excerpts, and source paths.
   - Inject search results into local Qwen and Codex text turns only when relevant.
   - Keep indexing local and optional; missing roots return an empty result.

3. **Workflow and task scheduler**
   - Add a JSON workflow definition with sequential/conditional steps and explicit approval boundaries.
   - Persist tasks under the local Jarvis state directory and run due tasks in a bounded Tokio scheduler.
   - Emit lifecycle events and support cancel/resume without touching Codex Voice.

4. **Cross-device boundary and verification**
   - Add a localhost-only authenticated event endpoint suitable for a future iPhone/Shortcuts adapter.
   - Keep it disabled by default and document the pairing boundary.
   - Run Rust/TypeScript tests, production build, and static review. Deployment/restart/push are separate user-authorized actions.
