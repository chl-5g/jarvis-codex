#!/usr/bin/env python3
"""Small JSONL adapter between Jarvis and a running CipherPipe hub."""
import argparse, asyncio, json, os, sys
import websockets

MAX_TEXT = 4000

async def main():
    p = argparse.ArgumentParser()
    p.add_argument("--proxy", required=True)
    p.add_argument("--keyfile", required=True)
    p.add_argument("--peer", default=os.environ.get("JARVIS_CIPHERPIPE_PEER", ""))
    args = p.parse_args()
    sys.path.insert(0, os.path.abspath(os.environ.get("JARVIS_CIPHERPIPE_ROOT", ".")))
    from backend.core.crypto import load_or_create_key
    sk = load_or_create_key(args.keyfile)
    pubkey = sk.public_key.format().hex()
    ws = await websockets.connect(f"ws://{args.proxy}", proxy=None)
    await ws.recv()  # hub identity
    await ws.send(json.dumps({"type": "lan_hello", "pubkey": pubkey}))
    await ws.recv()
    print(json.dumps({"event": "ready", "pubkey": pubkey}, ensure_ascii=False), flush=True)
    async def receive():
        async for raw in ws:
            if isinstance(raw, bytes):
                continue
            try:
                frame = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if frame.get("type") == "msg" and frame.get("from") not in ("me", ""):
                print(json.dumps({"event": "message", "from": frame.get("from", ""),
                                  "text": frame.get("text", ""), "id": frame.get("id", "")},
                                 ensure_ascii=False), flush=True)
    async def send():
        loop = asyncio.get_running_loop()
        while True:
            line = await loop.run_in_executor(None, sys.stdin.readline)
            if not line:
                return
            try:
                req = json.loads(line)
            except json.JSONDecodeError:
                print(json.dumps({"id": None, "ok": False, "error": "invalid JSON"}), flush=True)
                continue
            req_id = req.get("id")
            text = str(req.get("text", "")).strip()
            if req.get("op") != "send" or not text:
                print(json.dumps({"id": req_id, "ok": False, "error": "send requires text"}), flush=True)
                continue
            if len(text) > MAX_TEXT:
                print(json.dumps({"id": req_id, "ok": False, "error": "text too long"}), flush=True)
                continue
            target = str(req.get("to") or args.peer).strip()
            if not target:
                print(json.dumps({"id": req_id, "ok": False, "error": "peer is not configured"}), flush=True)
                continue
            await ws.send(json.dumps({"type": "msg", "text": text, "to": target}, ensure_ascii=False))
            print(json.dumps({"id": req_id, "ok": True, "result": {"to": target}}, ensure_ascii=False), flush=True)
    await asyncio.gather(receive(), send())

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except Exception as exc:
        print(json.dumps({"event": "error", "error": str(exc)}), flush=True)
        raise
