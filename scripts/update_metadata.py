#!/usr/bin/env python3
"""Write release identity and digest before signing the exact JSON bytes."""
import argparse
import hashlib
import json
import re
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("artifact", type=Path)
parser.add_argument("--version", required=True)
parser.add_argument("--commit", required=True)
parser.add_argument("--channel", required=True, choices=["stable", "nightly"])
parser.add_argument("--arch", required=True, choices=["x86_64", "aarch64"])
parser.add_argument("--os", default="linux", choices=["linux", "windows"])
args = parser.parse_args()
if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", args.commit):
    parser.error("a full Git commit is required")
# Not hashlib.file_digest: it needs Python 3.11, and Ubuntu 22.04, where releases are built, has 3.10.
hasher = hashlib.sha256()
with args.artifact.open("rb") as source:
    for chunk in iter(lambda: source.read(1 << 20), b""):
        hasher.update(chunk)
digest = hasher.hexdigest()
metadata = {
    "schema": 1,
    "os": args.os,
    "arch": args.arch,
    "channel": args.channel,
    "version": args.version,
    "commit": args.commit,
    "filename": args.artifact.name,
    "size": args.artifact.stat().st_size,
    "sha256": digest,
}
args.artifact.with_name(args.artifact.name + ".json").write_text(
    json.dumps(metadata, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8"
)
args.artifact.with_name(args.artifact.name + ".sha256").write_text(digest + "\n")
