#!/bin/zsh
set -euo pipefail

project_root=${0:A:h:h}
python_bin=${JARVIS_PYTHON:-/Users/caihaolun/Documents/Codex/2026-10-05/ni/work/jarvis-venv/bin/python}
model_path="$project_root/models/vision/face_landmarker.task"

"$python_bin" -m pip install -r "$project_root/src-tauri/vision-requirements.txt"
mkdir -p "${model_path:h}"
if [[ ! -s "$model_path" ]]; then
  curl -fL --retry 3 -o "$model_path" \
    https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task
fi
print "Vision runtime ready: $model_path"
