import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("activity", Path(__file__).parents[1] / "src-tauri/speaker_activity.py")
activity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(activity)


class SpeechSamplesTest(unittest.TestCase):
    def test_no_speech_does_not_create_a_voiceprint_sample(self):
        samples = activity.SpeechSamples(16000, 600)
        self.assertFalse(samples.ready)
        self.assertEqual(samples.duration_ms, 0)

    def test_short_speech_and_silence_do_not_satisfy_minimum(self):
        samples = activity.SpeechSamples(16000, 600)
        samples.add([0.1] * 16000)
        self.assertTrue(samples.ready)
        self.assertEqual(samples.duration_ms, 1000)

    def test_separate_confirmed_speech_segments_accumulate(self):
        samples = activity.SpeechSamples(16000, 600)
        samples.add([0.1] * 32000)
        self.assertTrue(samples.ready)
        self.assertEqual(samples.duration_ms, 2000)
        self.assertEqual(len(samples.wav()), 44 + 32000 * 2)


if __name__ == "__main__":
    unittest.main()
