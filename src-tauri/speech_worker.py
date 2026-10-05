"""Bounded offline speech JSONL protocol. No models, tools or HTTP routes."""
import base64
import contextlib
import io
import json
import multiprocessing as mp
import os
from pathlib import Path
import queue
import signal
import sys
import threading
import wave

MAX_LINE = 12 * 1024 * 1024
MAX_PCM = 16000 * 2 * 30
MAX_WAV = 8 * 1024 * 1024
MAX_TEXT = 1800

class SpeechBackend:
    def __init__(self):
        root = Path(os.environ.get('JARVIS_MODEL_ROOT', Path.home() / 'Jarvis-codex/models'))
        self.stt = Path(os.environ.get('JARVIS_STT_MODEL_DIR', root / 'whisper'))
        self.tts = Path(os.environ.get('JARVIS_TTS_MODEL_DIR', root / 'kokoro'))
        os.environ['HF_HUB_OFFLINE'] = '1'
        os.environ['TRANSFORMERS_OFFLINE'] = '1'
    def status(self):
        # File readiness is reported separately from dependency/model-load errors.
        return {'revision': 1, 'sttReady': (self.stt / 'revision.json').is_file(),
                'ttsReady': (self.tts / 'config.json').is_file()}
    def transcribe(self, pcm):
        if not self.stt.is_dir():
            raise RuntimeError('Whisper model not found')
        import numpy as np
        import mlx_whisper
        audio = np.frombuffer(pcm, dtype='<i2').astype(np.float32) / 32768
        return mlx_whisper.transcribe(audio, path_or_hf_repo=str(self.stt), fp16=True,
            condition_on_previous_text=False, hallucination_silence_threshold=1)['text'].strip()
    def synthesize(self, text):
        if not (self.tts / 'config.json').is_file():
            raise RuntimeError('Kokoro model not found')
        import numpy as np
        import re
        from mlx_audio.tts.utils import load_model
        clean = re.sub(r'[`*#]', '', text).strip()
        chinese = bool(re.search(r'[\u4e00-\u9fff]', clean))
        name = os.environ.get('JARVIS_TTS_ZH_VOICE', 'zm_yunxi') if chinese else os.environ.get('JARVIS_TTS_EN_VOICE', 'bm_george')
        # Voice is selected locally, never from protocol-supplied paths.
        if Path(name).name != name:
            raise ValueError('Invalid voice name')
        model = load_model(str(self.tts))
        chunks, size = [], 0
        for out in model.generate(text=clean, voice=str(self.tts / 'voices' / (name + '.safetensors')),
                                  lang_code='z' if chinese else 'b', speed=1.0):
            chunk = np.asarray(out.audio).reshape(-1)
            size += chunk.size * 2
            if size + 44 > MAX_WAV:
                raise ValueError('Audio output too large')
            chunks.append(chunk)
        if not chunks:
            raise RuntimeError('Kokoro returned no audio')
        output = io.BytesIO()
        with wave.open(output, 'wb') as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(24000)
            wav.writeframes((np.clip(np.concatenate(chunks), -1, 1) * 32767).astype('<i2').tobytes())
        return output.getvalue()

def validate(body):
    if not isinstance(body, dict):
        raise ValueError('Expected an object')
    ident = body.get('id')
    if type(ident) is not int or not 0 <= ident < 2**64:
        raise ValueError('Invalid operation id')
    op = body.get('op')
    if op in ('status', 'cancel'):
        return op, None
    if op == 'synthesize':
        text = body.get('text')
        if not isinstance(text, str) or not text.strip() or len(text) > MAX_TEXT:
            raise ValueError('Text must contain 1..1800 characters')
        return op, text
    if op == 'transcribe':
        encoded = body.get('pcm')
        if type(body.get('sampleRate')) is not int or body['sampleRate'] != 16000:
            raise ValueError('PCM must be mono int16 at 16000 Hz')
        if not isinstance(encoded, str) or len(encoded) > (MAX_PCM + 2) // 3 * 4:
            raise ValueError('PCM input too large')
        pcm = base64.b64decode(encoded, validate=True)
        if not pcm or len(pcm) % 2 or len(pcm) > MAX_PCM:
            raise ValueError('Invalid PCM size')
        return op, pcm
    raise ValueError('Unsupported speech operation')

def _execute(connection, backend_type, op, payload):
    # Native libraries may write to stdout; redirect the actual descriptor too.
    os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    try:
        backend = backend_type()
        if op == 'transcribe':
            text = backend.transcribe(payload)
            result = {'text': text[:MAX_TEXT]}
        else:
            wav = backend.synthesize(payload)
            if not 44 <= len(wav) <= MAX_WAV or wav[:4] != b'RIFF' or wav[8:12] != b'WAVE':
                raise ValueError('Invalid WAV output')
            result = {'wav': base64.b64encode(wav).decode('ascii')}
        connection.send({'ok': True, 'result': result})
    except Exception as error:
        connection.send({'ok': False, 'error': str(error)[:500]})
    finally:
        connection.close()

def serve(backend_type=SpeechBackend):
    inbox = queue.Queue(maxsize=8)
    def read_input():
        while True:
            line = sys.stdin.buffer.readline(MAX_LINE + 2)
            if not line:
                inbox.put(None)
                return
            if len(line) > MAX_LINE:
                while line and not line.endswith(b'\n'):
                    line = sys.stdin.buffer.readline(MAX_LINE + 2)
                inbox.put(ValueError('JSONL request too large'))
            else:
                inbox.put(line)
    threading.Thread(target=read_input, daemon=True).start()
    # A separate native operation process allows cancellation even during MLX calls.
    context = mp.get_context('spawn')
    active = None
    def reply(ident, result):
        sys.stdout.write(json.dumps({'id': ident, **result}, ensure_ascii=False) + '\n')
        sys.stdout.flush()
    def stop():
        nonlocal active
        if active:
            ident, process, receiver = active
            process.kill() if process.is_alive() else None
            process.join()
            receiver.close()
            active = None
            reply(ident, {'ok': False, 'error': 'Speech operation cancelled'})
    def terminated(_signal, _frame):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, terminated)
    try:
        while True:
            if active:
                ident, process, receiver = active
                if receiver.poll():
                    try:
                        result = receiver.recv()
                    except EOFError:
                        result = {'ok': False, 'error': 'Speech operation exited'}
                    process.join(timeout=1)
                    if process.is_alive():
                        process.kill()
                        process.join()
                    receiver.close()
                    active = None
                    reply(ident, result)
            try:
                line = inbox.get(timeout=.02)
            except queue.Empty:
                continue
            if line is None:
                break
            ident = None
            try:
                if isinstance(line, Exception):
                    raise line
                body = json.loads(line)
                if isinstance(body, dict) and type(body.get('id')) is int:
                    ident = body['id']
                op, payload = validate(body)
                if op == 'cancel':
                    stop()
                    reply(ident, {'ok': True, 'result': {'cancelled': True}})
                elif op == 'status':
                    with contextlib.redirect_stdout(sys.stderr):
                        result = backend_type().status()
                    reply(ident, {'ok': True, 'result': result})
                elif active:
                    raise ValueError('Speech worker busy')
                else:
                    receiver, sender = context.Pipe(duplex=False)
                    process = context.Process(target=_execute, args=(sender, backend_type, op, payload))
                    process.start()
                    sender.close()
                    active = (ident, process, receiver)
            except Exception as error:
                reply(ident, {'ok': False, 'error': str(error)[:500]})
    finally:
        stop()

if __name__ == '__main__':
    serve()
