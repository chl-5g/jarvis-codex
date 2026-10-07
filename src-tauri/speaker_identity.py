#!/usr/bin/env python3
import argparse, json, os
from pathlib import Path
import numpy as np
import soundfile as sf
import sherpa_onnx

def embedding(path, model):
    samples, rate = sf.read(path, dtype="float32", always_2d=False)
    if getattr(samples, "ndim", 1) > 1: samples = samples[:, 0]
    if rate != 16000:
        indexes = np.linspace(0, len(samples) - 1, max(1, round(len(samples) * 16000 / rate))).astype(np.int64)
        samples = samples[indexes]
        rate = 16000
    config = sherpa_onnx.SpeakerEmbeddingExtractorConfig(model=model, num_threads=2, provider=os.getenv("JARVIS_SPEAKER_PROVIDER", "coreml"))
    if not config.validate(): raise RuntimeError("invalid speaker model configuration")
    extractor = sherpa_onnx.SpeakerEmbeddingExtractor(config)
    stream = extractor.create_stream(); stream.accept_waveform(rate, samples); stream.input_finished()
    value = np.asarray(extractor.compute(stream), dtype=np.float32)
    return value / max(np.linalg.norm(value), 1e-8)

def main():
    parser = argparse.ArgumentParser(); parser.add_argument("--verify", required=True); args = parser.parse_args()
    profile_path = Path(os.environ["JARVIS_SPEAKER_PROFILE"]); profile = json.loads(profile_path.read_text())
    actual = embedding(args.verify, os.environ["JARVIS_SPEAKER_MODEL"])
    expected = np.asarray(profile["embedding"], dtype=np.float32); expected /= max(np.linalg.norm(expected), 1e-8)
    score = float(np.dot(actual, expected)); threshold = float(os.getenv("JARVIS_SPEAKER_THRESHOLD", "0.85"))
    print(json.dumps({"speaker": "allen" if score >= threshold else "unknown", "verified": score >= threshold, "score": score, "threshold": threshold}, ensure_ascii=False))

if __name__ == "__main__": main()
