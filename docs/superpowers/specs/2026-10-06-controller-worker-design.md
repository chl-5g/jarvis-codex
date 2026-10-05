# Offline speech worker integration

## Goal

Reuse the local speech capabilities in `controller.py` inside the current Jarvis
runtime without bringing back a second Jarvis server, a second Codex thread, or
a second permission path. When Codex Voice is unavailable, the existing Rust
orchestrator should keep owning model routing, memory, Skills, tools, workflows,
tasks, and the unified event stream while a managed Python worker provides local
speech input/output.

## Current boundary

`controller.py` currently combines four concerns: local Qwen HTTP calls,
per-request Codex CLI calls, local Whisper transcription, and local Kokoro or
macOS speech synthesis. It also starts an independent loopback HTTP server on
port 8082, keeps its own in-memory history, and owns a separate cancellation
and confirmation state. The Tauri application already owns the first two model
concerns, the memory and tool layers, permissions, and event publication. The
independent server and histories must not be reintroduced.

## Design

`controller.py` gains a worker mode selected by an explicit command-line flag.
In worker mode it loads the offline speech dependencies and communicates over
stdin/stdout using newline-delimited JSON. It accepts bounded operations:

- `status`: return model readiness and the worker revision;
- `transcribe`: accept one base64-encoded, mono 16-bit PCM payload and return
  the bounded transcript;
- `synthesize`: accept bounded text and return a base64-encoded WAV payload;
- `cancel`: stop the current speech operation.

Each response carries the operation id, a success flag, and either a bounded
result or an error. Logs go to stderr so stdout remains a machine-readable
protocol. The standalone 8082 HTTP server remains available only behind an
explicit legacy flag and is not used by the packaged Jarvis app.

The Rust side adds `offline_speech.rs`, which owns one child process per Jarvis
application instance. It starts the worker lazily for offline speech, serializes
requests through one mutex, enforces payload and text limits, times out stalled
operations, and terminates the child during application shutdown. Worker
readiness, failures, and operation phases are emitted through the existing
`jarvis-event` envelope. The worker never receives model prompts, filesystem
commands, permission decisions, or tool arguments.

The existing `local_qwen_chat` route remains the only local text-generation
route. The existing Rust tool gateway remains the only local tool-execution
route. `speak_text` uses the managed worker for offline synthesis and preserves
the current fallback when the worker or model is unavailable. Codex Voice keeps
its current WebRTC path and is not routed through the worker.

## Data flow

1. The UI selects local Qwen or detects that Codex Voice is unavailable.
2. Rust sends text to the existing local Qwen route, including memory, Skills,
   knowledge, and audited tool calls.
3. For speech output, Rust sends the final bounded answer to the worker.
4. The worker returns WAV bytes; Rust plays them through the existing playback
   path and emits lifecycle events to the UI and local bridge.
5. For a future offline microphone path, Rust can send captured PCM to the same
   worker without changing model or tool routing.

## Failure handling

- Missing Python, missing model files, malformed JSON, oversized payloads, and
  worker timeouts become bounded local events and do not crash Jarvis.
- Worker failure falls back to the existing local TTS implementation where
  possible; it never silently starts the legacy 8082 server.
- Cancellation terminates the current worker operation and clears its pending
  request without affecting the Codex thread.
- No worker response is treated as a model answer unless it has a matching
  operation id and a successful result.

## Verification

- Unit tests cover the Python JSONL protocol, payload limits, cancellation, and
  malformed requests.
- Rust tests cover worker command serialization, timeout/error mapping, and
  shutdown cleanup.
- Existing JavaScript, TypeScript, Rust, and clippy checks remain required.
- A packaged smoke test verifies one visible Jarvis process, no 8082 listener,
  local Qwen text response, and offline speech fallback when the worker models
  are present.

## Scope exclusions

This change does not add a new model, change Codex Voice networking, expose the
worker to the LAN, or migrate the existing memory/tool/knowledge implementation
to Python. GitHub push and Jarvis restart remain separate, explicitly requested
operations.
