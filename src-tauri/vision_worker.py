"""Local face-state encoder for Jarvis.

The worker accepts one temporary image path and emits only a compact JSON face
state. It never emits image bytes, landmarks, or the source path.
"""
import argparse
import json
import os
from pathlib import Path


def compact_face_state(blendshapes, face_present, confidence):
    scores = {}
    for item in blendshapes:
        if isinstance(item, dict):
            name = item.get("category_name") or item.get("name")
            score = item.get("score")
        else:
            name = getattr(item, "category_name", None) or getattr(item, "display_name", None)
            score = getattr(item, "score", None)
        if name and isinstance(score, (int, float)):
            scores[str(name)] = round(max(0.0, min(1.0, float(score))), 3)
    expression = "unknown"
    if face_present:
        smile = max(scores.get("mouthSmileLeft", 0.0), scores.get("mouthSmileRight", 0.0))
        frown = max(scores.get("mouthFrownLeft", 0.0), scores.get("mouthFrownRight", 0.0))
        jaw = scores.get("jawOpen", 0.0)
        brow_down = max(scores.get("browDownLeft", 0.0), scores.get("browDownRight", 0.0))
        if smile >= 0.55:
            expression = "smile"
        elif frown >= 0.55:
            expression = "frown"
        elif jaw >= 0.65 and brow_down < 0.35:
            expression = "surprised"
        elif brow_down >= 0.65:
            expression = "tense"
        else:
            expression = "neutral"
    top_blendshapes = dict(sorted(scores.items(), key=lambda item: item[1], reverse=True)[:12])
    return {
        "present": bool(face_present),
        "confidence": round(max(0.0, min(1.0, float(confidence))), 3),
        "expression": expression,
        "blendshapes": top_blendshapes,
        "model": "mediapipe-face-landmarker",
    }


def analyze_image(path, model_path):
    try:
        import mediapipe as mp
        from mediapipe.tasks import python
        from mediapipe.tasks.python import vision
    except ImportError as error:
        raise RuntimeError("vision.dependencies_missing: install mediapipe in the Jarvis vision environment") from error

    image_path = Path(path).expanduser()
    if not image_path.is_file() or image_path.stat().st_size > 10 * 1024 * 1024:
        raise RuntimeError("vision.image_invalid")
    if not model_path:
        raise RuntimeError("vision.model_missing: set JARVIS_FACE_LANDMARKER_MODEL")
    options = vision.FaceLandmarkerOptions(
        base_options=python.BaseOptions(model_asset_path=model_path),
        running_mode=vision.RunningMode.IMAGE,
        num_faces=1,
        output_face_blendshapes=True,
        output_facial_transformation_matrixes=True,
    )
    with vision.FaceLandmarker.create_from_options(options) as landmarker:
        result = landmarker.detect(mp.Image.create_from_file(str(image_path)))
    blendshapes = result.face_blendshapes[0] if result.face_blendshapes else []
    return compact_face_state(blendshapes, bool(result.face_landmarks), 1.0 if result.face_landmarks else 0.0)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", required=True)
    parser.add_argument("--model", default=os.getenv("JARVIS_FACE_LANDMARKER_MODEL", ""))
    args = parser.parse_args()
    try:
        print(json.dumps({"ok": True, "face": analyze_image(args.image, args.model)}, ensure_ascii=False))
    except Exception as error:  # pragma: no cover - exercised by the host process
        print(json.dumps({"ok": False, "error": str(error)}, ensure_ascii=False))
        raise SystemExit(1)


if __name__ == "__main__":
    main()
