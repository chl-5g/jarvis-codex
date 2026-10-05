#!/bin/zsh
set -e
TASK_ROOT='/Users/caihaolun/Jarvis-codex'
export JARVIS_WORKSPACE="$TASK_ROOT/agent-workspace"
export JARVIS_CODEX_BIN="$TASK_ROOT/Jarvis Codex.app/Contents/Resources/codex"
export JARVIS_PYTHON='/Users/caihaolun/Documents/Codex/2026-10-05/ni/work/jarvis-venv/bin/python'
export JARVIS_TTS_MODEL_DIR="$TASK_ROOT/models/kokoro"
exec "$TASK_ROOT/Jarvis Codex.app/Contents/MacOS/jarvis-codex"
