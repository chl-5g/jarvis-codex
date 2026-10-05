#!/usr/bin/env python3
"""Local Allen voiceprint enrollment and verification.

The command deliberately has no network API. ModelScope may download the
configured model on its first run, then inference and the stored voiceprint
remain local. It prints one JSON object so the Swift wake helper can call it
without importing Python into the macOS app.

Typical use:

  speaker_gate.py enroll --profile ~/.jarvis/allen.json allen-1.wav allen-2.wav
  speaker_gate.py verify --profile ~/.jarvis/allen.json spoken-wake.wav
  speaker_gate.py serve --model iic/speech_campplus_sv_zh-cn_16k-common

Exit codes: 0 = Allen, 1 = rejected speaker, 2 = unknown/invalid input.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from pathlib import Path
from typing import Any

DEFAULT_MODEL = "iic/speech_campplus_sv_zh-cn_16k-common"
DEFAULT_THRESHOLD = 0.62
MIN_SECONDS = 0.8


def _load_pipeline(model: str):
    try:
        from modelscope.pipelines import pipeline
        from modelscope.utils.constant import Tasks
    except ImportError as error:
        raise RuntimeError(
            "speaker verifier dependencies are missing; install modelscope, "
            "datasets, soundfile, and scikit-learn in the local Jarvis venv"
        ) from error
    return pipeline(task=Tasks.speaker_verification, model=model, device="cpu")


def _embedding(pipe: Any, audio_path: Path) -> list[float]:
    if not audio_path.is_file():
        raise ValueError(f"audio file does not exist: {audio_path}")
    result = pipe([str(audio_path)], output_emb=True)
    values = result.get("embs") if isinstance(result, dict) else None
    if values is None:
        raise RuntimeError(f"speaker model returned no embedding for {audio_path}")
    if hasattr(values, "detach"):
        values = values.detach()
    if hasattr(values, "cpu"):
        values = values.cpu()
    if hasattr(values, "numpy"):
        values = values.numpy()
    if getattr(values, "ndim", 1) == 2:
        values = values[0]
    embedding = [float(value) for value in values]
    if not embedding or not all(math.isfinite(value) for value in embedding):
        raise RuntimeError(f"speaker model returned an invalid embedding for {audio_path}")
    return embedding


def _normalise(values: list[float]) -> list[float]:
    length = math.sqrt(sum(value * value for value in values))
    if length <= 1e-8:
        raise ValueError("speaker embedding is empty")
    return [value / length for value in values]


def _cosine(left: list[float], right: list[float]) -> float:
    if len(left) != len(right):
        raise ValueError("enrollment and query embeddings have different dimensions")
    return sum(a * b for a, b in zip(left, right))


def _audio_seconds(path: Path) -> float | None:
    try:
        import soundfile as sf

        info = sf.info(path)
        return float(info.frames) / float(info.samplerate)
    except Exception:
        # The model can still validate a readable input. Duration is only a
        # conservative quality gate when soundfile cannot inspect the file.
        return None


def enroll(args: argparse.Namespace) -> int:
    if len(args.audio) < 2:
        raise ValueError("enrollment needs at least two voice samples")
    pipe = _load_pipeline(args.model)
    vectors = [_embedding(pipe, Path(path)) for path in args.audio]
    dimensions = {len(vector) for vector in vectors}
    if len(dimensions) != 1:
        raise ValueError("enrollment samples produced different embedding dimensions")
    average = [sum(vector[index] for vector in vectors) / len(vectors) for index in range(len(vectors[0]))]
    profile = {
        "version": 1,
        "speaker": "Allen",
        "model": args.model,
        "dimensions": len(average),
        "embedding": _normalise(average),
        "samples": len(vectors),
        "threshold": args.threshold if args.threshold is not None else DEFAULT_THRESHOLD,
    }
    destination = Path(args.profile).expanduser()
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"speakerAccess": "allen", "profile": str(destination), "samples": len(vectors)}))
    return 0


def _verify(args: argparse.Namespace, pipe: Any | None = None) -> tuple[dict[str, Any], int]:
    profile_path = Path(args.profile).expanduser()
    if not profile_path.is_file():
        return {"speakerAccess": "unknown", "reason": "profile-not-enrolled"}, 2
    seconds = _audio_seconds(Path(args.audio))
    if seconds is not None and seconds < MIN_SECONDS:
        return {"speakerAccess": "unknown", "reason": "audio-too-short", "seconds": seconds}, 2
    profile = json.loads(profile_path.read_text())
    pipe = pipe or _load_pipeline(args.model or profile.get("model", DEFAULT_MODEL))
    query = _normalise(_embedding(pipe, Path(args.audio)))
    enrolled = _normalise([float(value) for value in profile["embedding"]])
    score = _cosine(enrolled, query)
    threshold = float(args.threshold if args.threshold is not None else profile.get("threshold", DEFAULT_THRESHOLD))
    access = "allen" if score >= threshold else "rejected"
    result = {"speakerAccess": access, "score": score, "threshold": threshold}
    return result, 0 if access == "allen" else 1


def verify(args: argparse.Namespace) -> int:
    result, code = _verify(args)
    print(json.dumps(result, ensure_ascii=False))
    return code


def serve(args: argparse.Namespace) -> int:
    """Keep the voiceprint model resident and verify JSON-lines requests."""
    pipe = _load_pipeline(args.model)
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
            request_args = argparse.Namespace(
                profile=request.get("profile", args.profile),
                audio=request["audio"],
                model=request.get("model", args.model),
                threshold=request.get("threshold"),
            )
            result, _ = _verify(request_args, pipe)
        except Exception as error:
            result = {"speakerAccess": "unknown", "error": str(error)}
        print(json.dumps(result, ensure_ascii=False), flush=True)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", default=DEFAULT_MODEL, help="ModelScope model id or local model directory")
    parser.add_argument("--profile", default="~/.jarvis/allen-speaker.json")
    parser.add_argument("--threshold", type=float, default=None)
    subparsers = parser.add_subparsers(dest="command", required=True)
    enroll_parser = subparsers.add_parser("enroll", help="create Allen's local voiceprint")
    enroll_parser.add_argument("--model", default=argparse.SUPPRESS)
    enroll_parser.add_argument("--profile", default=argparse.SUPPRESS)
    enroll_parser.add_argument("--threshold", type=float, default=argparse.SUPPRESS)
    enroll_parser.add_argument("audio", nargs="+")
    enroll_parser.set_defaults(handler=enroll)
    verify_parser = subparsers.add_parser("verify", help="verify a WAV/AIFF/FLAC sample")
    verify_parser.add_argument("--model", default=argparse.SUPPRESS)
    verify_parser.add_argument("--profile", default=argparse.SUPPRESS)
    verify_parser.add_argument("--threshold", type=float, default=argparse.SUPPRESS)
    verify_parser.add_argument("audio")
    verify_parser.set_defaults(handler=verify)
    serve_parser = subparsers.add_parser("serve", help="keep the local model loaded for repeated checks")
    serve_parser.add_argument("--model", default=argparse.SUPPRESS)
    serve_parser.add_argument("--profile", default=argparse.SUPPRESS)
    serve_parser.add_argument("--threshold", type=float, default=argparse.SUPPRESS)
    serve_parser.set_defaults(handler=serve)
    args = parser.parse_args()
    try:
        return args.handler(args)
    except Exception as error:
        print(json.dumps({"speakerAccess": "unknown", "error": str(error)}, ensure_ascii=False))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
