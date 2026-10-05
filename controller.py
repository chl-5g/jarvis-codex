"""Loopback-only Jarvis controller. Audio and history stay in memory."""
import base64
import concurrent.futures
import io
import json
import os
import re
import secrets
import subprocess
import threading
import time
import urllib.error
import urllib.request
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CONFIG = json.loads((ROOT / 'settings.json').read_text())
STATE_DIR = ROOT / 'runtime'
STATE_DIR.mkdir(exist_ok=True)
TOKEN_FILE = STATE_DIR / 'token'
if not TOKEN_FILE.exists():
    fd = os.open(TOKEN_FILE, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f:
        f.write(secrets.token_hex(32))
TOKEN = TOKEN_FILE.read_text().strip()
LOCK = threading.RLock()
POOL = concurrent.futures.ThreadPoolExecutor(max_workers=1)
STATE = {'phase': 'ready', 'mode': CONFIG['default_brain'], 'listening': False,
         'transcript': '', 'reply': '', 'error': '', 'revision': 0,
         'voice_backend': 'macOS 本地语音', 'wake_ready': False, 'stt_ready': False,
         'tts_ready': False, 'pending': None, 'activity': '', 'audio_id': 0}
HISTORY = {'local': [], 'codex': []}
CURRENT = None
CANCEL = threading.Event()
AUDIO = b''
SPEECH = None
WAKE = None
RECORDER = bytearray()
FRAME_BUFFER = bytearray()
RECORD_START = 0.0
LAST_VOICE = 0.0
RECORDING = False
VOICE_MUTEX = threading.RLock()
CONFIRM_EVENT = threading.Event()
CONFIRM_RESULT = False
CONTEXT = ROOT / 'agent-workspace'
CONTEXT.mkdir(exist_ok=True)

def update(**kw):
    with LOCK:
        STATE.update(kw)
        STATE['revision'] += 1

def status():
    with LOCK:
        return dict(STATE)

def check_cancel():
    if CANCEL.is_set():
        raise InterruptedError('已停止')

def codex_command():
    return [CONFIG['codex'], 'exec', '-p', 'jarvis', '--skip-git-repo-check',
            '--ephemeral', '--json', '-C', str(CONTEXT), '-']

def codex_reply(text):
    global CURRENT
    prompt = ('你是用户 Mac 上的贾维斯。用户授权你执行明确提出的电脑任务。'
              '中文回答，简洁适合朗读。普通问答不要读文件或查看屏幕。'
              '需要 GUI 操作时，先读取 Computer Use skill，使用 node_repl 中的 @oai/sky，'
              '每次操作后重新 get_app_state。系统设置、删除、对外发送、上传敏感数据、'
              '付款、安装新软件下载等动作前，调用 jarvis_controls.request_confirmation，'
              '得到明确批准再执行；若批准工具不可用，停止并说明。'
              '不要调用其他云模型。不要更改全局配置或安装新服务来绕过限制。'
              '不要声称执行了未验证的动作。不要停止在方案或再问是否开始。'
              '前文仅供上下文：\n' + json.dumps(HISTORY['codex'][-10:], ensure_ascii=False)
              + '\n用户当前明确指令：\n' + text)
    env = os.environ.copy()
    # Reuse ChatGPT login, never accidentally select usage-billed API keys.
    for k in ('OPENAI_API_KEY', 'CODEX_API_KEY', 'OPENAI_BASE_URL'):
        env.pop(k, None)
    log = (STATE_DIR / 'codex.log').open('w')
    try:
        CURRENT = subprocess.Popen(codex_command(), stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=log, text=True, env=env)
        CURRENT.stdin.write(prompt)
        CURRENT.stdin.close()
        reply = ''
        started = time.monotonic()
        for line in CURRENT.stdout:
            check_cancel()
            if time.monotonic() - started > 240:
                CURRENT.terminate()
                raise TimeoutError('Codex 响应超时，请检查网络')
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            item = event.get('item', {})
            if event.get('type') == 'item.completed' and item.get('type') == 'agent_message':
                reply = item.get('text', '')
                update(activity='Codex 已回答')
            elif event.get('type') == 'item.started':
                update(activity='Codex 正在执行：' + item.get('type', 'task'))
            elif event.get('type') == 'error':
                update(activity=event.get('message', '')[:160])
        rc = CURRENT.wait()
        if rc or not reply:
            raise RuntimeError('Codex 未返回答案，请查看应用状态或 runtime/codex.log')
        return reply
    finally:
        if CURRENT and CURRENT.poll() is None:
            CURRENT.terminate()
        CURRENT = None
        log.close()

def local_reply(text):
    body = {'model': CONFIG['model_id'], 'messages': [
        {'role': 'system', 'content': '你是贾维斯，私人助理。用用户的语言简洁回答，适合语音播报。没有调用工具时不要声称操作过电脑。'},
        *HISTORY['local'][-10:], {'role': 'user', 'content': text}],
        'max_tokens': 400, 'temperature': 0.6, 'stream': False,
        'enable_thinking': True}
    req = urllib.request.Request(CONFIG['llm_url'] + '/chat/completions',
                                 data=json.dumps(body).encode(),
                                 headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=120) as r:
        content = json.load(r)['choices'][0]['message']['content']
    return re.sub(r'<think>.*?</think>', '', content, flags=re.S).strip()

def wav_bytes(samples, sr):
    import numpy as np
    with io.BytesIO() as f:
        with wave.open(f, 'wb') as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(sr)
            w.writeframes((np.clip(np.asarray(samples), -1, 1) * 32767).astype('<i2').tobytes())
        return f.getvalue()

def speak(text):
    global AUDIO, CURRENT
    clean = re.sub(r'[`*#]', '', text)[:1800]
    if SPEECH is not None:
        import numpy as np
        chinese = bool(re.search('[\u4e00-\u9fff]', clean))
        chunks = []
        for out in SPEECH.generate(text=clean, voice='zm_yunyang' if chinese else 'bm_george',
                                   lang_code='z' if chinese else 'b', speed=1.0):
            check_cancel()
            chunks.append(np.asarray(out.audio))
        if not chunks:
            raise RuntimeError('本地语音合成未返回音频')
        AUDIO = wav_bytes(np.concatenate(chunks), 24000)
        update(phase='speaking', audio_id=STATE['audio_id'] + 1, voice_backend='Kokoro 本地模型')
    else:
        # Offline system speech while the downloaded speech models are being prepared.
        chinese = bool(re.search('[\u4e00-\u9fff]', clean))
        update(phase='speaking', voice_backend='macOS 本地语音（模型准备中）')
        CURRENT = subprocess.Popen(['/usr/bin/say', '-v', 'Tingting' if chinese else 'Daniel', clean])
        CURRENT.wait()
        CURRENT = None
        update(phase='ready')

def run_job(text, mode):
    try:
        update(phase='thinking', transcript=text, error='', activity='正在处理任务')
        check_cancel()
        reply = codex_reply(text) if mode == 'codex' else local_reply(text)
        check_cancel()
        with LOCK:
            HISTORY[mode] += [{'role': 'user', 'content': text}, {'role': 'assistant', 'content': reply}]
            HISTORY[mode] = HISTORY[mode][-12:]
        update(reply=reply, phase='synthesizing', activity='正在本地生成语音')
        speak(reply)
    except InterruptedError:
        update(phase='ready', activity='已停止')
    except Exception as e:
        update(phase='ready', error=str(e), activity='任务未完成')

def submit(text, mode):
    text = text.strip()
    if not text:
        raise ValueError('请输入或说出指令')
    if mode not in ('local', 'codex'):
        raise ValueError('未知大脑模式')
    with LOCK:
        if STATE['phase'] not in ('ready', 'recording'):
            raise ValueError('当前任务还在运行，请先停止')
        CANCEL.clear()
        update(phase='thinking', mode=mode, transcript=text, error='')
        POOL.submit(run_job, text, mode)

def transcribe(pcm):
    global RECORDING
    try:
        import numpy as np
        import mlx_whisper
        update(phase='transcribing', activity='本地识别语音')
        text = mlx_whisper.transcribe(np.frombuffer(pcm, dtype='<i2').astype(np.float32) / 32768,
                                    path_or_hf_repo=CONFIG['stt_dir'],
                                    fp16=True, condition_on_previous_text=False,
                                    hallucination_silence_threshold=1)['text'].strip()
        check_cancel()
        update(phase='ready')
        if text:
            run_job(text, STATE['mode'])
        else:
            update(activity='没有听清，请再说一次')
    except Exception as e:
        update(phase='ready', error=str(e))

def start_recording():
    global RECORDING, RECORD_START, LAST_VOICE
    if not STATE['stt_ready']:
        raise ValueError('语音识别模型正在准备；目前可输入文字')
    if STATE['phase'] != 'ready':
        raise ValueError('请先停止当前任务')
    CANCEL.clear()
    RECORDING = True
    RECORD_START = LAST_VOICE = time.monotonic()
    RECORDER.clear()
    update(phase='recording', activity='请说指令，说完后稍停一下', error='')

def feed_audio(pcm):
    global RECORDING, LAST_VOICE
    import numpy as np
    with VOICE_MUTEX:
        if STATE['phase'] not in ('ready', 'recording'):
            return
        if not RECORDING:
            if not STATE['listening'] or WAKE is None:
                return
            FRAME_BUFFER.extend(pcm)
            while len(FRAME_BUFFER) >= 2560:
                frame = np.frombuffer(bytes(FRAME_BUFFER[:2560]), dtype='<i2')
                del FRAME_BUFFER[:2560]
                if max(WAKE.predict(frame).values(), default=0) > 0.65:
                    WAKE.reset()
                    start_recording()
                    break
        else:
            RECORDER.extend(pcm)
            level = np.sqrt(np.mean((np.frombuffer(pcm, dtype='<i2').astype(np.float32) / 32768) ** 2))
            now = time.monotonic()
            if level > 0.012:
                LAST_VOICE = now
            if ((now - LAST_VOICE > 1.15 and now - RECORD_START > 1.6) or now - RECORD_START > 25):
                data = bytes(RECORDER)
                RECORDER.clear()
                RECORDING = False
                update(phase='transcribing')
                POOL.submit(transcribe, data)

def load_speech():
    global WAKE, SPEECH
    # All model files have been downloaded and verified before this offline phase.
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    for _ in range(600):
        try:
            if (Path(CONFIG['stt_dir']) / 'revision.json').exists():
                import mlx_whisper
                update(stt_ready=True)
            if SPEECH is None and (Path(CONFIG['tts_dir']) / 'revision.json').exists():
                from mlx_audio.tts.utils import load_model
                SPEECH = load_model(CONFIG['tts_dir'])
                update(tts_ready=True)
            if WAKE is None and (Path(CONFIG['wake_dir']) / 'hey_jarvis_v0.1.onnx').exists():
                from openwakeword.model import Model
                WD = Path(CONFIG['wake_dir'])
                WAKE = Model(wakeword_models=[str(WD / 'hey_jarvis_v0.1.onnx')], inference_framework='onnx',
                             melspec_model_path=str(WD / 'melspectrogram.onnx'),
                             embedding_model_path=str(WD / 'embedding_model.onnx'))
                update(wake_ready=True)
            if STATE['stt_ready'] and STATE['wake_ready'] and STATE['tts_ready']:
                break
        except ImportError:
            pass
        except Exception as e:
            update(activity='语音组件准备：' + str(e)[:160])
        time.sleep(3)

def cancel():
    global RECORDING
    CANCEL.set()
    if CURRENT and CURRENT.poll() is None:
        CURRENT.terminate()
    CONFIRM_EVENT.set()
    with VOICE_MUTEX:
        RECORDING = False
        RECORDER.clear()
        FRAME_BUFFER.clear()
    update(phase='ready', pending=None, activity='已停止')

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def respond(self, data, code=200, mime='application/json'):
        if not isinstance(data, bytes):
            data = json.dumps(data, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header('Content-Type', mime)
        self.send_header('Content-Length', str(len(data)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(data)

    def allowed(self):
        return secrets.compare_digest(self.headers.get('X-Jarvis-Token', ''), TOKEN)

    def do_GET(self):
        if not self.allowed():
            return self.respond({'error': 'Unauthorized'}, 401)
        if self.path == '/status':
            self.respond(status())
        elif self.path == '/audio':
            self.respond(AUDIO, mime='audio/wav')
        else:
            self.respond({'error': 'Not found'}, 404)

    def do_POST(self):
        global CONFIRM_RESULT
        if not self.allowed():
            return self.respond({'error': 'Unauthorized'}, 401)
        try:
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 <= size <= 1048576:
                return self.respond({'error': 'Request too large'}, 413)
            payload = self.rfile.read(size)
            if self.path == '/pcm':
                if len(payload) % 2:
                    raise ValueError('Invalid PCM')
                if STATE['stt_ready']:
                    feed_audio(payload)
                return self.respond({'ok': True})
            body = json.loads(payload or '{}')
            if self.path == '/ask':
                submit(body['text'], body.get('mode', STATE['mode']))
            elif self.path == '/mode':
                if STATE['phase'] != 'ready':
                    raise ValueError('请先停止当前任务再切换模式')
                if body['mode'] not in ('local', 'codex'):
                    raise ValueError('Invalid mode')
                update(mode=body['mode'])
            elif self.path == '/listen':
                update(listening=bool(body.get('enabled')))
            elif self.path == '/record':
                with VOICE_MUTEX:
                    start_recording()
            elif self.path == '/cancel':
                cancel()
            elif self.path == '/played':
                update(phase='ready', activity='等待下一条指令')
            elif self.path == '/confirm':
                CONFIRM_RESULT = False
                CONFIRM_EVENT.clear()
                update(pending={'id': secrets.token_hex(8), 'action': body['action'], 'detail': body['detail']})
                CONFIRM_EVENT.wait(180)
                result = CONFIRM_RESULT and not CANCEL.is_set()
                update(pending=None)
                return self.respond({'approved': result})
            elif self.path == '/approval':
                if not STATE['pending'] or body.get('id') != STATE['pending']['id']:
                    raise ValueError('确认请求已失效')
                CONFIRM_RESULT = body.get('approved') is True
                update(pending=None)
                CONFIRM_EVENT.set()
            else:
                return self.respond({'error': 'Not found'}, 404)
            self.respond({'ok': True})
        except (ValueError, KeyError) as e:
            self.respond({'error': str(e)}, 400)
        except Exception as e:
            self.respond({'error': str(e)}, 500)

if __name__ == '__main__':
    threading.Thread(target=load_speech, daemon=True).start()
    ThreadingHTTPServer(('127.0.0.1', CONFIG['controller_port']), Handler).serve_forever()
