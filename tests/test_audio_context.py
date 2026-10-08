import sys
import unittest
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src-tauri"))

from speaker_activity import summarize_audio  # noqa: E402


class AudioContextTests(unittest.TestCase):
    def test_summarizes_volume_and_speech_duration_without_raw_audio(self):
        samples = np.zeros(16000, dtype=np.float32)
        samples[4000:8000] = 0.2

        result = summarize_audio(samples, 16000)

        self.assertEqual(result["duration_ms"], 1000)
        self.assertEqual(result["volume"], "normal")
        self.assertIn("cough_count", result)
        self.assertNotIn("pcm", result)

    def test_marks_sudden_bursts_as_possible_coughs(self):
        samples = np.zeros(16000, dtype=np.float32)
        samples[:] = 0.1
        samples[4000:4400] = 0.9
        samples[6000:8000] = 0.0

        result = summarize_audio(samples, 16000)

        self.assertGreaterEqual(result["cough_count"], 1)
        self.assertGreaterEqual(result["breath_like_pause_count"], 1)
        self.assertEqual(result["breathing"], "possible")


if __name__ == "__main__":
    unittest.main()
