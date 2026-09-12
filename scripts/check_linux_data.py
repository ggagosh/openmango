#!/usr/bin/env python3
"""Verify packaged Forge and BSON tools against a disposable MongoDB container."""
import json
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time

tools = Path(sys.argv[1]).resolve()


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=60).stdout.strip()


container = run("docker", "run", "--rm", "-d", "-p", "127.0.0.1::27017", "mongo:7.0")
sidecar = None
try:
    endpoint = run("docker", "port", container, "27017/tcp").splitlines()[0]
    uri = f"mongodb://{endpoint}/?directConnection=true"
    deadline = time.monotonic() + 30
    while True:
        ready = subprocess.run(
            ["docker", "exec", container, "mongosh", "--quiet", "--eval", "db.adminCommand({ping:1})"],
            capture_output=True, timeout=5,
        )
        if ready.returncode == 0:
            break
        if time.monotonic() > deadline:
            raise TimeoutError("MongoDB fixture did not become ready")
        time.sleep(0.25)
    run("docker", "exec", container, "mongosh", "--quiet", "--eval",
        'db.getSiblingDB("package_source").fixture.insertOne({_id:1,title:"Linux package",count:Long("9007199254740993")})')
    with tempfile.TemporaryDirectory(prefix="openmango-data-") as directory:
        archive = str(Path(directory) / "BSON ფაილი.archive")
        run(str(tools / "mongodump"), f"--uri={uri}", "--db=package_source", f"--archive={archive}")
        run(str(tools / "mongorestore"), f"--uri={uri}", f"--archive={archive}",
            "--nsFrom=package_source.*", "--nsTo=package_restored.*")
        with tempfile.TemporaryFile() as errors:
            sidecar = subprocess.Popen([str(tools / "mongosh-sidecar")], stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=errors, bufsize=0)

            def rpc(request_id, method, params):
                sidecar.stdin.write((json.dumps({"id": request_id, "method": method, "params": params}) + "\n").encode())
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    if not select.select([sidecar.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                        break
                    line = sidecar.stdout.readline()
                    if not line:
                        raise RuntimeError("Packaged Forge exited before replying")
                    response = json.loads(line)
                    if response.get("id") == request_id:
                        assert response.get("ok"), response
                        return response.get("result")
                raise TimeoutError(f"Packaged Forge did not complete {method}")

            rpc(1, "create_session", {"session_id": "package", "uri": uri, "database": "package_restored"})
            result = rpc(2, "evaluate", {"session_id": "package", "code": 'db.fixture.findOne({_id:1}).count.toString()'})
            assert result["printable"] == "9007199254740993", result
            rpc(3, "dispose_session", {"session_id": "package"})
            sidecar.stdin.close()
            assert sidecar.wait(timeout=5) == 0
    print("Packaged BSON dump/restore and Forge query passed, including exact Int64 preservation")
finally:
    if sidecar and sidecar.poll() is None:
        sidecar.kill()
        sidecar.wait()
    run("docker", "rm", "-f", container)
