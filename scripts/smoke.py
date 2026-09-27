#!/usr/bin/env python3
"""Exercise actual CLI, native libraries, layered patches, HTTP and encrypted reports."""
import hashlib
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

build = pathlib.Path(sys.argv[1]).resolve()
repo = pathlib.Path(__file__).resolve().parent.parent
exe = build / ("syncer.exe" if os.name == "nt" else "syncer")
suffix = ".dll" if os.name == "nt" else ".dylib" if sys.platform == "darwin" else ".so"
prefix = "" if os.name == "nt" else "lib"

with tempfile.TemporaryDirectory(prefix="syncer-smoke-") as temporary:
    root = pathlib.Path(temporary).resolve()
    project = root / "project"
    project.mkdir()
    state = root / "state"
    def run(*args, code=0):
        print("smoke:", " ".join(map(str,args[:2])), flush=True)
        result = subprocess.run([str(exe), "--output", "json", "--state-dir", str(state), "--project", str(project), *map(str, args)], capture_output=True, text=True)
        assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
        return result.stdout
    for name in ["json", "http", "git", "structured"]:
        run("extension", "install", build / f"{prefix}syncer_extension_{name}{suffix}")
    assert len(json.loads(run("extension", "list"))) == 4
    target = project / "settings.json"
    target.write_text(json.dumps({"personal": {"theme": "dark"}, "sandbox": {"network": {"allowedDomains": ["personal.example", "retired.example.com"]}}}))
    for priority, name in enumerate(["company", "team", "personal"]):
        run("add", name, repo / "examples" / f"{name}.hcl", "--priority", priority)
    original = target.read_bytes()
    snapshot = {str(p.relative_to(state)): p.read_bytes() for p in state.rglob("*") if p.is_file()}
    plan = json.loads(run("apply", "--dry-run", "--diff"))
    assert plan["changed_files"] == 1
    assert target.read_bytes() == original
    assert snapshot == {str(p.relative_to(state)): p.read_bytes() for p in state.rglob("*") if p.is_file()}
    run("apply")
    data = json.loads(target.read_text())
    assert data["personal"] == {"theme": "dark"}
    assert set(data["sandbox"]["network"]["allowedDomains"]) == {"personal.example", "github.com", "registry.npmjs.org", "docs.rs"}
    assert data["sandbox"]["enabled"] is True
    assert json.loads(run("apply", "--check"))["changed_files"] == 0

    # Alternate-ID changes cannot turn off an inherited locked setting.
    bad = root / "bad.hcl"
    bad.write_text('schema_version=1\nname="bypass"\nrule "zzz" {\n target="settings.json"\n kind="json"\n operation="set"\n pointer="/sandbox/enabled"\n value=false\n}\n')
    run("add", "bypass", bad)
    before = target.read_bytes()
    run("apply", code=1)
    assert target.read_bytes() == before
    run("remove", "bypass")

    # Reports are encrypted before reaching even a local sink.
    identity = root / "provider.agekey"
    recipient = json.loads(run("report", "keygen", identity))["recipient"]
    sink = root / "reports"
    run("report", "enable", "company", sink, "--recipient", recipient)
    run("apply")
    reports = list(sink.glob("*.age"))
    assert reports and all(b"sandbox" not in p.read_bytes() for p in reports)
    summary = json.loads(run("report", "summarize", sink, "--identity", identity))
    assert summary["rules"] and all(r["all_observed_compliant"] for r in summary["rules"])
    assert any(r["id"] == "sandbox-domains" for r in summary["rules"])

    # Binary whole-file sync uses an explicitly enrolled asset.
    asset = root / "asset.bin"
    asset.write_bytes(bytes(range(256)))
    run("add", "blob", asset, "--asset")
    binary_policy = root / "binary.hcl"
    binary_policy.write_text('schema_version=1\nname="binary"\nrule "blob" {\n target="blob.bin"\n kind="file"\n operation="replace"\n content_from="blob"\n}\n')
    run("add", "binary", binary_policy)
    run("apply")
    assert (project / "blob.bin").read_bytes() == asset.read_bytes()
    run("apply", "--check")

    # Actual HTTP native transport, including conditional publication and no redirects.
    policy = (repo / "examples" / "local.hcl").read_bytes()
    received = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args): pass
        def do_GET(self):
            if self.path == "/redirect":
                self.send_response(302); self.send_header("Location", "/policy"); self.end_headers(); return
            self.send_response(200); self.send_header("ETag", '"v1"'); self.end_headers(); self.wfile.write(policy)
        def do_PUT(self):
            if self.headers.get("If-Match") != '"v1"':
                self.send_response(412); self.end_headers(); return
            received.append(self.rfile.read(int(self.headers["Content-Length"])))
            self.send_response(200); self.end_headers()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    try:
        uri = f"http://127.0.0.1:{server.server_port}"
        run("add", "http-test", uri + "/policy")
        run("push", "http-test", repo / "examples" / "local.hcl")
        assert received == [policy]
        run("add", "redirect-test", uri + "/redirect", code=1)
        run("remove", "http-test")
    finally:
        server.shutdown(); server.server_close(); thread.join()

    # Tampering with a library is detected before its native entry point is called.
    enrollment = json.loads((state / "config.json").read_text())
    library = pathlib.Path(enrollment["extensions"][0]["path"])
    with library.open("ab") as file: file.write(b"tampered")
    run("extension", "list", code=1)
print("native extension, layering, dry-run, asset, reporting and HTTP smoke tests passed")
subprocess.run([sys.executable, str(repo / "scripts" / "structured_smoke.py"), str(build)], check=True)
subprocess.run([sys.executable, str(repo / "scripts" / "development_smoke.py"), str(build)], check=True)
