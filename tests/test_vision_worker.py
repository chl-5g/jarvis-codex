import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src-tauri"))

from vision_worker import compact_face_state  # noqa: E402


class VisionWorkerTests(unittest.TestCase):
    def test_compacts_blendshapes_and_never_returns_image_data(self):
        result = compact_face_state(
            [
                {"category_name": "mouthSmileLeft", "score": 0.9},
                {"category_name": "eyeBlinkLeft", "score": 0.2},
            ],
            face_present=True,
            confidence=0.97,
        )

        self.assertEqual(result["present"], True)
        self.assertEqual(result["blendshapes"]["mouthSmileLeft"], 0.9)
        self.assertNotIn("image", result)
        self.assertNotIn("frame", result)

    def test_empty_detection_becomes_unknown_state(self):
        result = compact_face_state([], face_present=False, confidence=0.0)

        self.assertEqual(result["present"], False)
        self.assertEqual(result["expression"], "unknown")


if __name__ == "__main__":
    unittest.main()
