#!/usr/bin/env python3
"""Offline Kokoro TTS bridge used by the Jarvis macOS shell.

The model and voice files are supplied by the deployment directory.  This
process replaces itself with ``afplay`` after synthesis, so terminating the
tracked child also stops playback.
"""

from __future__ import annotations

import argparse
import os
import re
import tempfile
import wave
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--text", required=True)
    args = parser.parse_args()

    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    import numpy as np
    from mlx_audio.tts.utils import load_model

    text = re.sub(r"[`*#]", "", args.text).strip()[:1800]
    if not text:
        return
    chinese = bool(re.search(r"[\u4e00-\u9fff]", text))
    voice_name = os.environ.get("JARVIS_TTS_ZH_VOICE", "zm_yunxi") if chinese else os.environ.get("JARVIS_TTS_EN_VOICE", "bm_george")
    voice = str(args.model / "voices" / f"{voice_name}.safetensors")
    lang_code = "z" if chinese else "b"
    model = load_model(str(args.model))
    chunks = [np.asarray(out.audio).reshape(-1) for out in model.generate(
        text=text,
        voice=voice,
        lang_code=lang_code,
        speed=1.0,
    )]
    if not chunks:
        raise RuntimeError("本地 Kokoro 没有生成音频")

    samples = np.clip(np.concatenate(chunks), -1, 1)
    with tempfile.NamedTemporaryFile(prefix="jarvis-tts-", suffix=".wav", delete=False) as output:
        wav_path = output.name
    try:
        with wave.open(wav_path, "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(24000)
            wav.writeframes((samples * 32767).astype("<i2").tobytes())
        os.execvp("afplay", ["afplay", wav_path])
    finally:
        # afplay owns the path after exec; a later cleanup pass removes it.
        pass


if __name__ == "__main__":
    main()
