import io
from pathlib import Path
import sys
import time
import wave
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'src-tauri'))

class FixtureBackend:
    def status(self):
        return {'revision': 1, 'sttReady': True, 'ttsReady': True}
    def transcribe(self, pcm):
        return 'fixture transcript'
    def synthesize(self, text):
        if text == 'stall':
            time.sleep(60)
        output = io.BytesIO()
        with wave.open(output, 'wb') as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(24000)
            wav.writeframes(b'\0\0')
        return output.getvalue()

if __name__ == '__main__':
    from speech_worker import serve
    serve(backend_type=FixtureBackend)
