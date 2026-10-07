# Jarvis Voice Workflow Design

## Goal

Make Jarvis report and execute startup and voice-session work as explicit sequential workflows. A session is only ready after speaker verification, Codex app-server initialization, thread creation or resume, memory loading decision, and OpenAI Voice WebRTC connection have completed.

## Current failure

The renderer currently interleaves startup, wake callbacks, speaker verification, Codex runtime creation, memory preparation, and Voice events. Several callbacks update the UI independently, so the startup message can claim initialization before the native Codex runtime or Voice session exists. The avatar meter and voiceprint capture also use separate consumers of the same microphone stream.

## State model

The renderer owns one workflow state:

- `booting`: application code is loading.
- `waiting-wake`: workspace, model status, permissions status, and wake listener are ready; no Codex Voice session exists.
- `capturing-speaker`: a wake event started a microphone capture and voiceprint sample.
- `initializing-codex`: Codex app-server is running, initialized, and has created or resumed a thread.
- `loading-memory`: the backend has applied the verified speaker's memory policy to the thread.
- `connecting-voice`: WebRTC offer and the app-server realtime start request are in progress.
- `ready`: `thread/realtime/started` has arrived and all required workflow steps succeeded.
- `degraded`: the current workflow failed; the error identifies the failed step and wake listening can be re-armed.
- `stopped`: the user paused Jarvis.

The existing visual modes remain presentation modes. A separate workflow state prevents visual animation from being used as a readiness signal.

## Workflows

### Startup workflow

1. Resolve and persist the workspace.
2. Read local model status as optional fallback information.
3. Read permission status without requesting new permissions.
4. Arm the wake listener.
5. Emit `waiting-wake`.

Startup must never claim Codex Agent, private memory, or OpenAI Voice is connected.

### Voice session workflow

1. Transition to `capturing-speaker`.
2. Request microphone permission and acquire the stream.
3. Capture one complete voiced segment. The same processor callback supplies the microphone meter and starts the sample buffer when RMS crosses the configured threshold.
4. Verify the speaker and record `allen` or `unknown`.
5. Transition to `initializing-codex`; invoke the backend Voice command. The backend performs app-server `initialize`, `initialized`, thread resume/start, and applies the speaker memory policy.
6. If the speaker is Allen, report memory status after `prepare_wake_context`; otherwise explicitly report that private memory was withheld.
7. Transition to `connecting-voice`; wait for `thread/realtime/started`.
8. Transition to `ready` and announce the truthful result exactly once per app run.

A failed step carries its workflow step name, returns to `degraded`, cleans the peer and stream, and re-arms wake listening.

## Interfaces

Introduce a small renderer workflow module with:

- `WorkflowStep` and `WorkflowState` string unions.
- `VoiceWorkflowContext` containing speaker access, memory status, thread id, and Voice info.
- `runVoiceWorkflow(context, steps)` or equivalent sequential executor that emits step-start, step-success, and step-failure events through callbacks.

Keep native capabilities behind the existing Tauri commands. The workflow module coordinates them; it does not import Tauri internals or implement microphone, Codex, or memory logic.

## User-visible status

All user-visible workflow copy comes from `config/ui.json`. Startup uses a waiting message. Completion has separate verified and unverified messages. The verified message is only used when Voice is connected and private memory was loaded. The unverified message explicitly states that private memory was not loaded.

## Error and logging policy

Every step logs its name and outcome in the existing event stream. A failure message includes the failed step and original error. No callback may emit the completion message before the workflow executor reaches `ready`.

## Verification

- Unit tests cover valid step order, failure transition, exactly-once completion, and unverified-memory copy.
- Existing frontend and Rust tests continue to pass.
- Production build installs successfully.
- Runtime logs show startup `waiting-wake`, then ordered voice steps, and only then `ready`.
