"""Streaming local speech detection. Never computes speaker embeddings."""
import argparse
import base64
import io
import json
import os
import struct
import sys
import wave

import numpy as np


def summarize_audio(samples, rate):
    """Return bounded local audio features; never include samples or a waveform."""
    values = np.asarray(samples, dtype=np.float32)
    if values.size == 0 or rate <= 0:
        return {
            "duration_ms": 0,
            "volume": "unknown",
            "cough_count": 0,
            "breathing": "unknown",
        }
    rms = float(np.sqrt(np.mean(np.square(values))))
    volume = "quiet" if rms < 0.03 else "normal" if rms < 0.2 else "loud"
    frame_size = max(1, round(rate * 0.02))
    frame_rms = np.asarray(
        [
            np.sqrt(np.mean(np.square(values[index : index + frame_size])))
            for index in range(0, len(values), frame_size)
            if len(values[index : index + frame_size])
        ],
        dtype=np.float32,
    )
    baseline = max(float(np.median(frame_rms)), 0.01)
    burst = frame_rms > max(0.45, baseline * 3.5)
    quiet = frame_rms < max(0.015, baseline * 0.45)
    cough_count = 0
    breath_like_pause_count = 0
    in_burst = False
    burst_frames = 0
    quiet_frames = 0
    for active, quiet_frame in zip(burst, quiet):
        if active:
            in_burst = True
            burst_frames += 1
        elif in_burst:
            if 2 <= burst_frames <= 25:
                cough_count += 1
            in_burst = False
            burst_frames = 0
        if quiet_frame:
            quiet_frames += 1
        elif quiet_frames:
            if 4 <= quiet_frames <= 40:
                breath_like_pause_count += 1
            quiet_frames = 0
    if in_burst and 2 <= burst_frames <= 25:
        cough_count += 1
    if 4 <= quiet_frames <= 40:
        breath_like_pause_count += 1
    return {
        "duration_ms": round(values.size * 1000 / rate),
        "volume": volume,
        "cough_count": min(cough_count, 8),
        "breath_like_pause_count": min(breath_like_pause_count, 8),
        "breathing": "possible" if breath_like_pause_count else "not_detected",
    }


class SpeechSamples:
    """Only accepts segments confirmed by VAD, never ambient microphone frames."""
    def __init__(self, rate, minimum_ms):
        self.rate = rate
        self.minimum_ms = minimum_ms
        self.samples = []

    @property
    def duration_ms(self):
        return round(len(self.samples) * 1000 / self.rate)

    @property
    def ready(self):
        return self.duration_ms >= self.minimum_ms

    def add(self, segment):
        self.samples.extend(segment)

    def wav(self):
        output = io.BytesIO()
        pcm = [round(max(-1, min(1, float(value))) * 32767) for value in self.samples]
        with wave.open(output, "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(self.rate)
            wav.writeframes(struct.pack(f"<{len(pcm)}h", *pcm))
        return output.getvalue()


def serve(settings):
    import numpy as np
    import sherpa_onnx

    config = sherpa_onnx.VadModelConfig()
    config.silero_vad.model = os.getenv("JARVIS_SPEAKER_VAD_MODEL", settings["vad"]["model"])
    config.silero_vad.threshold = settings["vad"]["threshold"]
    config.silero_vad.min_speech_duration = settings["vad"]["minSpeechMs"] / 1000
    config.silero_vad.min_silence_duration = settings["vad"]["minSilenceMs"] / 1000
    config.silero_vad.max_speech_duration = settings["timeouts"]["speakerVerificationMaxMs"] / 1000
    config.sample_rate = 16000
    vad = sherpa_onnx.VoiceActivityDetector(config, buffer_size_in_seconds=30)
    buffer = np.empty(0, dtype=np.float32)
    saw_speech = False
    print(json.dumps({"ready": False, "speaking": False, "speechMs": 0}), flush=True)
    for line in sys.stdin:
        request = json.loads(line)
        pcm = base64.b64decode(request["pcm"], validate=True)
        if not pcm or len(pcm) % 2 or len(pcm) > 32000:
            raise ValueError("Invalid speech detection PCM frame")
        buffer = np.concatenate((buffer, np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768))
        while len(buffer) >= config.silero_vad.window_size:
            vad.accept_waveform(buffer[:config.silero_vad.window_size])
            buffer = buffer[config.silero_vad.window_size:]
            saw_speech = saw_speech or vad.is_speech_detected()
            while not vad.empty():
                segment = SpeechSamples(16000, settings["timeouts"]["speakerVerificationMinSpeechMs"])
                segment.add(vad.front.samples)
                vad.pop()
                if segment.ready:
                    audio_events = summarize_audio(segment.samples, 16000)
                    result = {"ready": True, "speaking": True, "speechMs": segment.duration_ms,
                              "segmentEnded": True, "audio": base64.b64encode(segment.wav()).decode("ascii"),
                              "audioEvents": audio_events}
                    print(json.dumps(result), flush=True)
                    return
        result = {"ready": False, "speaking": saw_speech, "speechMs": 0, "segmentEnded": False}
        print(json.dumps(result), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    serve(json.loads(parser.parse_args().config))
