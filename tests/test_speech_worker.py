import base64
import io
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]

class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.p = subprocess.Popen([sys.executable, str(ROOT / 'tests/fixtures/speech_backend.py')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.out_buffer = b''
    def tearDown(self):
        self.p.stdin.close()
        try:
            self.p.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()
        self.p.stdout.close()
        self.p.stderr.close()
    def send(self, body):
        self.p.stdin.write(json.dumps(body) + '\n')
        self.p.stdin.flush()
    def receive(self):
        while b'\n' not in self.out_buffer:
            self.assertTrue(select.select([self.p.stdout], [], [], 3)[0], 'worker reply timeout')
            chunk = os.read(self.p.stdout.fileno(), 65536)
            self.assertTrue(chunk, 'worker exited instead of responding')
            self.out_buffer += chunk
        line, self.out_buffer = self.out_buffer.split(b'\n', 1)
        return json.loads(line)
    def test_status_and_valid_audio_operations(self):
        self.send({'id': 1, 'op': 'status'})
        self.assertEqual(self.receive()['result']['revision'], 1)
        self.send({'id': 2, 'op': 'transcribe', 'pcm': base64.b64encode(b'\x00\x00').decode(), 'sampleRate': 16000})
        self.assertEqual(self.receive(), {'id': 2, 'ok': True, 'result': {'text': 'fixture transcript'}})
        self.send({'id': 3, 'op': 'synthesize', 'text': 'hello'})
        self.assertEqual(base64.b64decode(self.receive()['result']['wav'])[:4], b'RIFF')
    def test_malformed_requests_and_validation_do_not_kill_worker(self):
        for body in [[], {'id': 1, 'op': 'run_command'}, {'id': 2, 'op': 'synthesize', 'text': 'x' * 1801}, {'id': 3, 'op': 'transcribe', 'pcm': '!!!', 'sampleRate': 16000}, {'id': 4, 'op': 'transcribe', 'pcm': 'AA==', 'sampleRate': 16000}, {'id': 5, 'op': 'transcribe', 'pcm': 'AAA=', 'sampleRate': 44100}, {'id': True, 'op': 'status'}]:
            self.send(body)
            self.assertFalse(self.receive()['ok'])
        self.p.stdin.write('{bad json\n')
        self.p.stdin.flush()
        self.assertFalse(self.receive()['ok'])
        self.send({'id': 9, 'op': 'status'})
        self.assertTrue(self.receive()['ok'])
    def test_pcm_and_line_limits(self):
        self.send({'id': 1, 'op': 'transcribe', 'pcm': base64.b64encode(b'\0' * 960002).decode(), 'sampleRate': 16000})
        self.assertFalse(self.receive()['ok'])
        self.p.stdin.write('x' * (12 * 1024 * 1024 + 1) + '\n')
        self.p.stdin.flush()
        self.assertFalse(self.receive()['ok'])
    def test_cancel_interrupts_native_work_and_worker_remains_usable(self):
        self.send({'id': 1, 'op': 'synthesize', 'text': 'stall'})
        time.sleep(.2)
        self.send({'id': 2, 'op': 'cancel'})
        results = [self.receive(), self.receive()]
        self.assertEqual({r['id'] for r in results}, {1, 2})
        self.assertFalse(next(r for r in results if r['id'] == 1)['ok'])
        self.send({'id': 3, 'op': 'status'})
        self.assertTrue(self.receive()['ok'])
    def test_eof_cleans_active_operation(self):
        self.send({'id': 1, 'op': 'synthesize', 'text': 'stall'})
        time.sleep(.2)
        self.p.stdin.close()
        self.assertEqual(self.p.wait(timeout=3), 0)
    def test_controller_worker_has_no_legacy_side_effects(self):
        result = subprocess.run([sys.executable, str(ROOT / 'controller.py'), '--speech-worker'], input='{"id":1,"op":"status"}\n', text=True, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['ok'])
    def test_controller_requires_explicit_mode(self):
        result = subprocess.run([sys.executable, str(ROOT / 'controller.py')], capture_output=True, timeout=3)
        self.assertNotEqual(result.returncode, 0)

if __name__ == '__main__':
    unittest.main()
