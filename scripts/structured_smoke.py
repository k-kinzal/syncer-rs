#!/usr/bin/env python3
"""Verify structured adapters through the native ABI and core transaction planner."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile

build = pathlib.Path(sys.argv[1]).resolve()
exe = build / ("syncer.exe" if os.name == "nt" else "syncer")
suffix = ".dll" if os.name == "nt" else ".dylib" if sys.platform == "darwin" else ".so"
prefix = "" if os.name == "nt" else "lib"
cases = [
    ("yaml", "personal: PERSONAL\nmanaged: false\n", "/managed", True),
    ("toml", "personal = 'PERSONAL'\nmanaged = false\n", "/managed", True),
    ("hcl", 'personal = "PERSONAL"\nconfig "main" { managed = false }\n', "/config/main/managed", True),
    ("xml", '<config><personal>PERSONAL</personal><managed enabled="false"/></config>', "/config/managed/0/@enabled", "true"),
    ("jsonc", '{/* PERSONAL */ "personal":"PERSONAL", "managed":false}', "/managed", True),
    ("json5", "{personal:'PERSONAL', managed:false,}", "/managed", True),
    ("ini", "personal=PERSONAL\n[config]\nmanaged=false\n", "/config/managed", "true"),
    ("dotenv", "PERSONAL=PERSONAL\nMANAGED=false\n", "/MANAGED", "true"),
    ("properties", "personal=PERSONAL\nmanaged=false\n", "/managed", "true"),
    ("csv", "personal,managed\nPERSONAL,false\n", "/0/managed", "true"),
    ("tsv", "personal\tmanaged\nPERSONAL\tfalse\n", "/0/managed", "true"),
    ("plist", '<plist><dict><key>personal</key><string>PERSONAL</string><key>managed</key><false/></dict></plist>', "/managed", True),
]

with tempfile.TemporaryDirectory(prefix="syncer-structured-smoke-") as temporary:
    root = pathlib.Path(temporary).resolve()
    project = root / "project"
    project.mkdir()
    state = root / "state"

    def run(*args, code=0):
        result = subprocess.run([str(exe), "--project", str(project), "--state-dir", str(state), *map(str, args)], capture_output=True, text=True)
        assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
        return result.stdout

    run("extension", "install", build / f"{prefix}syncer_extension_structured{suffix}")
    rules = ['schema_version=1', 'name="formats"']
    for kind, content, pointer, value in cases:
        (project / f"config.{kind}").write_text(content, encoding="utf-8")
        rules.append(f'rule "{kind}" {{\n target="config.{kind}"\n kind="{kind}"\n pointer={json.dumps(pointer)}\n operation="set"\n value={json.dumps(value)}\n override="locked"\n}}')
    policy = root / "formats.hcl"
    policy.write_text("\n".join(rules) + "\n", encoding="utf-8")
    run("validate", policy)
    run("add", "formats", policy)
    before = {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}
    plan = json.loads(run("apply", "--dry-run"))
    assert plan["changed_files"] == len(cases)
    assert before == {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}
    run("apply")
    assert json.loads(run("apply", "--check"))["changed_files"] == 0
    for kind, *_ in cases:
        assert "PERSONAL" in (project / f"config.{kind}").read_text(encoding="utf-8")

    # A whole-file override cannot bypass a locked structured field.
    bypass = root / "bypass.hcl"
    bypass.write_text('schema_version=1\nname="bypass"\nrule "zzz" {\n target="config.yaml"\n kind="file"\n operation="replace"\n value="personal: PERSONAL\\nmanaged: false\\n"\n}\n', encoding="utf-8")
    run("add", "bypass", bypass)
    before = {p.name: p.read_bytes() for p in project.iterdir()}
    run("apply", code=1)
    assert before == {p.name: p.read_bytes() for p in project.iterdir()}
print("all 12 native structured formats, dry-run immutability, idempotence and locked-field bypass checks passed")
