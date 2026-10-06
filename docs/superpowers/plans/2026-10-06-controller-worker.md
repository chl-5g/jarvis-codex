# Offline speech worker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reuse Whisper and Kokoro via an isolated JSONL speech worker managed by Tauri.

**Architecture:** controller.py dispatches worker mode before any legacy initialization. speech_worker.py implements only speech operations. offline_speech.rs serializes bounded requests and owns cancellation, timeout, process lifetime and response validation; speak_text retains its existing fallback.

**Tech Stack:** Python standard library multiprocessing, MLX Whisper/Audio, Rust Tokio, Tauri.

**Spec:** docs/superpowers/specs/2026-10-06-controller-worker-design.md

## Global Constraints

- No second Qwen, Codex, 8082 server, memory, or permission path in worker mode.
- Rust owns model routing and tools; Codex Voice is unchanged.
- Logs go to stderr; every response includes id and ok.
- Do not push GitHub or launch/restart Jarvis.
- Packaged interactive smoke test is deferred because it requires launching Jarvis.

## Review Focus

- Missing models return bounded errors and preserve fallback.
- Bad ids and malformed or oversized responses never become audio.
- Cancel during synthesis promptly kills work and suppresses fallback playback.
- EOF and application exit leave no worker or audio process.
- Payload limits apply before base64 decoding and during output generation.

### Task 1: Python protocol and isolated entrypoint

**Files:** controller.py, src-tauri/speech_worker.py, tests/test_speech_worker.py, tests/fixtures/speech_backend.py.
**Interfaces:** JSONL {id, op, text? / pcm?, sampleRate?}; replies {id, ok, result / error}. Operations status/transcribe/synthesize/cancel only. PCM mono int16 at 16000 Hz, max 30 seconds; text max 1800 characters; WAV max 8 MiB; JSONL max 12 MiB.

- [ ] Write behavior tests for malformed input, bounds, valid speech outputs, cancellation, EOF, no legacy side effects.
- [ ] Run python3 -m unittest discover -s tests -p 'test_speech_worker.py'; expect missing protocol failures.
- [ ] Implement isolated dispatch and lazy offline backends; subprocess cancellation isolates native MLX calls.
- [ ] Run same tests; expect all passing.

### Task 2: Rust process owner

**Files:** src-tauri/src/offline_speech.rs, src-tauri/Cargo.toml.
**Interfaces:** OfflineSpeech::request(script, op, fields, timeout) -> Result<Value,String>; cancel() cleans worker; cancellation identified separately from failures. Requests serialized, ids validated, reads capped; stalled worker killed and reaped.

- [ ] Add real child-process tests for reply matching, errors, oversized response, timeout, cancellation and cleanup.
- [ ] Run cargo test offline_speech; expect new tests fail before implementation.
- [ ] Implement child ownership, bounded protocol, cancellation notification, kill-on-drop.
- [ ] Run cargo test offline_speech; expect passing tests.

### Task 3: Tauri integration and packaging

**Files:** src-tauri/src/lib.rs, src-tauri/tauri.conf.json, README.zh-CN.md, package.json.
**Interfaces:** speak_text uses worker WAV and afplay; offline_transcribe accepts bounded PCM and returns transcript only. Existing local_tts.py remains fallback. stop_all/shutdown/exit terminate speech work and playback.

- [ ] Test WAV validation and packaging/entrypoint against isolated fixtures.
- [ ] Add worker lifecycle events through events::emit; maintain fallback on failures, suppress on cancellation.
- [ ] Run Python suite, npm run check, cargo test and npm run build. Verify bundled worker files without launching Jarvis.
- [ ] Review diff and commit only this feature; preserve existing uncommitted edits.
