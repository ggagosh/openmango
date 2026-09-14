#!/usr/bin/env python3
"""Check the packaged Forge protocol without requiring a database or host Bun."""
import json
import subprocess
import sys

process = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
try:
    stdout, stderr = process.communicate(json.dumps({"id": 1, "method": "ping", "params": {}}) + "\n", timeout=20)
except subprocess.TimeoutExpired:
    process.kill()
    process.communicate()
    raise SystemExit("Bundled Forge runtime did not answer within 20 seconds")
responses = [json.loads(line) for line in stdout.splitlines() if line.startswith("{")]
if process.returncode != 0 or not any(reply.get("id") == 1 and reply.get("ok") is True for reply in responses):
    raise SystemExit(f"Bundled Forge protocol check failed (exit {process.returncode}): " + stderr[-1000:] + stdout[-1000:])
print("Bundled Forge protocol passed")
