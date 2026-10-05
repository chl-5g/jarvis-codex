import json
import urllib.request
from pathlib import Path
from mcp.server.fastmcp import FastMCP

ROOT = Path(__file__).resolve().parent
mcp = FastMCP('jarvis_controls')

@mcp.tool()
def request_confirmation(action: str, detail: str) -> dict:
    """Ask the human in the Jarvis app to approve a sensitive action. Describe the exact data, destination and consequence. Never proceed unless approved is true."""
    token = (ROOT / 'runtime/token').read_text().strip()
    req = urllib.request.Request('http://127.0.0.1:8082/confirm',
                                 data=json.dumps({'action': action, 'detail': detail}).encode(),
                                 headers={'Content-Type': 'application/json', 'X-Jarvis-Token': token})
    with urllib.request.urlopen(req, timeout=190) as r:
        return json.load(r)

if __name__ == '__main__':
    mcp.run()
